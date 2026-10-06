#!/usr/bin/env python3
"""MotionEngine fixture image producer -- a procedural STAND-IN for an AI image generator.

This lives outside the Rust engine on purpose: it only reads an
`asset-prompts.json` (the vendor-neutral request) and writes files into a
delivery directory (the vendor-neutral response, see
docs/GENERATED_ASSET_PROTOCOL.md, "Delivery contract").  It is NOT an AI image
generator; the sidecar says so.

    python3 fixture_generator.py <asset-prompts.json> <delivery-dir>
        [--defect none|head_cropped|too_small|no_alpha] [--only SPEC_ID]
        [--with-head-metadata]

Deterministic: the numpy RNG is seeded from the spec fingerprint, nothing
depends on time, and PNG/JPEG encoder parameters are set explicitly.
"""
import argparse
import json
import math
import os
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

SS = 2  # supersample factor; everything is drawn at 2x and downsampled (LANCZOS)

# framing -> (width, height, kind)
FRAMINGS = {
    "half_figure": (1024, 1365, "person"),
    "head_and_shoulders": (1024, 1280, "person"),
    "full_figure": (1024, 1536, "person"),
    "object": (1024, 1024, "object"),
    "scene": (1080, 1440, "scene"),
    "artifact": (900, 1200, "artifact"),
}


# ---------------------------------------------------------------------------
# small geometry / raster helpers
# ---------------------------------------------------------------------------
def cr_closed(pts, n=10):
    """Closed Catmull-Rom spline through `pts`."""
    p = np.array(pts, float)
    m = len(p)
    out = []
    for i in range(m):
        p0, p1, p2, p3 = p[(i - 1) % m], p[i], p[(i + 1) % m], p[(i + 2) % m]
        for t in np.linspace(0, 1, n, endpoint=False):
            out.append(
                0.5
                * (
                    2 * p1
                    + (-p0 + p2) * t
                    + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t
                    + (-p0 + 3 * p1 - 3 * p2 + p3) * t ** 3
                )
            )
    return [tuple(q) for q in out]


def cr_open(pts, n=10):
    """Open Catmull-Rom spline through `pts` (end points included)."""
    p = [np.array(q, float) for q in pts]
    p = [p[0]] + p + [p[-1]]
    out = []
    for i in range(1, len(p) - 2):
        p0, p1, p2, p3 = p[i - 1], p[i], p[i + 1], p[i + 2]
        for t in np.linspace(0, 1, n, endpoint=False):
            out.append(
                0.5
                * (
                    2 * p1
                    + (-p0 + p2) * t
                    + (2 * p0 - 5 * p1 + 4 * p2 - p3) * t * t
                    + (-p0 + 3 * p1 - 3 * p2 + p3) * t ** 3
                )
            )
    out.append(p[-2])
    return [tuple(q) for q in out]


def mirror(pts):
    """Mirror a left-half point list (x<=0) into a full closed outline."""
    return list(pts) + [(-x, y) for x, y in reversed(pts)]


def shift(a, dx, dy):
    """Shift a 2D array by (dx,dy) pixels with edge replication."""
    dx, dy = int(round(dx)), int(round(dy))
    h, w = a.shape
    pad = max(abs(dx), abs(dy), 1)
    p = np.pad(a, pad, mode="edge")
    return p[pad - dy : pad - dy + h, pad - dx : pad - dx + w]


def blur(a, r):
    if r <= 0.3:
        return a
    im = Image.fromarray(np.clip(a * 255 + 0.5, 0, 255).astype(np.uint8), "L")
    return np.asarray(im.filter(ImageFilter.GaussianBlur(r)), np.float32) / 255.0


class Canvas:
    """Float RGB + alpha working surface at supersampled resolution."""

    def __init__(self, w, h):
        self.w, self.h = w * SS, h * SS
        self.rgb = np.zeros((self.h, self.w, 3), np.float32)
        self.a = np.zeros((self.h, self.w), np.float32)
        self.X = np.arange(self.w, dtype=np.float32)[None, :]
        self.Y = np.arange(self.h, dtype=np.float32)[:, None]

    # masks --------------------------------------------------------------
    def poly(self, pts, b=0.0):
        im = Image.new("L", (self.w, self.h), 0)
        ImageDraw.Draw(im).polygon([(float(x), float(y)) for x, y in pts], fill=255)
        return blur(np.asarray(im, np.float32) / 255.0, b)

    def ellipse(self, cx, cy, rx, ry, ang=0.0, b=0.0):
        t = np.linspace(0, 2 * math.pi, 72, endpoint=False)
        ca, sa = math.cos(ang), math.sin(ang)
        pts = [
            (cx + rx * math.cos(k) * ca - ry * math.sin(k) * sa, cy + rx * math.cos(k) * sa + ry * math.sin(k) * ca)
            for k in t
        ]
        return self.poly(pts, b)

    def stroke(self, pts, width, b=0.0):
        im = Image.new("L", (self.w, self.h), 0)
        d = ImageDraw.Draw(im)
        pts = [(float(x), float(y)) for x, y in pts]
        d.line(pts, fill=255, width=max(1, int(round(width))), joint="curve")
        r = width / 2.0
        for x, y in (pts[0], pts[-1]):
            d.ellipse([x - r, y - r, x + r, y + r], fill=255)
        return blur(np.asarray(im, np.float32) / 255.0, b)

    # colour ---------------------------------------------------------------
    def grad(self, p0, p1, c0, c1):
        d = (p1[0] - p0[0], p1[1] - p0[1])
        l2 = d[0] ** 2 + d[1] ** 2
        t = ((self.X - p0[0]) * d[0] + (self.Y - p0[1]) * d[1]) / l2
        t = np.clip(t, 0, 1)[..., None]
        c0 = np.array(c0, np.float32) / 255.0
        c1 = np.array(c1, np.float32) / 255.0
        return c0 + (c1 - c0) * t

    def paint(self, mask, color, op=1.0, solid=True):
        m = (mask * op).astype(np.float32)
        c = np.array(color, np.float32) / 255.0 if isinstance(color, (tuple, list)) else color
        self.rgb += (c - self.rgb) * m[..., None]
        if solid:
            self.a = 1.0 - (1.0 - self.a) * (1.0 - m)

    def edge_dark(self, mask, dx, dy, b):
        """Inner edge band on the side opposite to the (dx,dy) shift direction."""
        return np.clip(mask * (1.0 - blur(shift(mask, dx, dy), b)), 0, 1)


def to_image(cv, extra_blur=1.1):
    """Downsample the working surface to a final RGBA image (premultiplied resize)."""
    a = blur(cv.a, extra_blur)
    a = np.clip(a, 0, 1)
    pm = np.clip(cv.rgb, 0, 1) * a[..., None]
    w, h = cv.w // SS, cv.h // SS
    chans = []
    for k in range(3):
        im = Image.fromarray(pm[..., k].astype(np.float32), "F")
        chans.append(np.asarray(im.resize((w, h), Image.LANCZOS), np.float32))
    aim = Image.fromarray(a.astype(np.float32), "F").resize((w, h), Image.LANCZOS)
    a2 = np.clip(np.asarray(aim, np.float32), 0, 1)
    pm2 = np.stack(chans, -1)
    rgb = np.where(a2[..., None] > 1e-4, pm2 / np.maximum(a2[..., None], 1e-4), 0.0)
    rgb = np.clip(rgb, 0, 1)
    out = np.dstack([rgb, a2])
    return Image.fromarray((out * 255 + 0.5).astype(np.uint8), "RGBA")


def opaque_to_image(cv):
    w, h = cv.w // SS, cv.h // SS
    chans = []
    for k in range(3):
        im = Image.fromarray(np.clip(cv.rgb[..., k], 0, 1).astype(np.float32), "F")
        chans.append(np.asarray(im.resize((w, h), Image.LANCZOS), np.float32))
    rgb = np.clip(np.stack(chans, -1), 0, 1)
    return Image.fromarray((rgb * 255 + 0.5).astype(np.uint8), "RGB")


# ---------------------------------------------------------------------------
# the person
# ---------------------------------------------------------------------------
JACKET = (40, 44, 53)
JACKET_LIGHT = (66, 71, 82)
SHIRT = (236, 236, 232)
SHIRT_SHADE = (176, 182, 192)
SKIN = (222, 186, 160)
SKIN_SHADE = (158, 116, 98)
HAIR = (30, 26, 27)
TIE = (30, 37, 54)
TROUSER = (46, 48, 57)


def draw_person(cv, cx, y0, u, rng, mode, head_dy=0.0):
    """Draw the office worker. cx/y0/u in supersampled px (y0 = top of hair, u = head width).

    mode: 'half' | 'hs' | 'full'.  Returns head box (x0,y0,x1,y1) in canvas px.
    """
    full = mode == "full"

    BX = 0.86  # body is narrower than the raw drawing units (head-relative proportions)

    def by(y):
        return y if y < 1.0 else 1.0 + (y - 1.0) * 0.9

    def P(x, y):
        return (cx + x * u * BX, y0 + by(y) * u)

    def P0(x, y):  # head space: unscaled
        return (cx + x * u, y0 + y * u)

    def PL(pts):
        return [P(x, y) for x, y in pts]

    ang = math.radians(-3.5)  # head tilt (bowed, a touch to the side)
    hc = (0.02, 0.70)

    def PH(x, y):
        dx, dy = x - hc[0], y - hc[1]
        xr = dx * math.cos(ang) - dy * math.sin(ang) + hc[0]
        yr = dx * math.sin(ang) + dy * math.cos(ang) + hc[1] + head_dy
        return P0(xr, yr)

    def PHL(pts):
        return [PH(x, y) for x, y in pts]

    bottom = 14.0  # far below the image for cropped modes

    # ------------------------------------------------------------ trousers/shoes (full)
    if full:
        legs = np.zeros((cv.h, cv.w), np.float32)
        for s in (-1, 1):
            leg = [
                (s * 0.98, 5.0),
                (s * 0.90, 6.6),
                (s * 0.80, 7.6),
                (s * 0.70, 9.35),
                (s * 0.24, 9.35),
                (s * 0.14, 7.6),
                (s * 0.06, 6.0),
                (s * 0.0, 5.0),
            ]
            m = cv.poly(PL(cr_closed(leg, 6)))
            legs = np.maximum(legs, m)
            cv.paint(m, cv.grad(P(-1, 5), P(1, 9), (58, 60, 70), (34, 36, 44)))
            # inner-leg shadow and crease
            cv.paint(m * cv.edge_dark(m, -0.10 * u, 0, 0.05 * u), (16, 17, 22), 0.55, solid=False)
            crease = cv.stroke(PL([(s * 0.52, 5.6), (s * 0.50, 7.0), (s * 0.44, 9.2)]), 0.03 * u, 0.02 * u)
            cv.paint(crease * m, (20, 20, 26), 0.45, solid=False)
            cv.paint(cv.stroke(PL([(s * 0.86, 6.0), (s * 0.78, 7.6), (s * 0.70, 9.2)]), 0.03 * u, 0.02 * u) * m, (90, 96, 110), 0.25, solid=False)
            # shoe
            shoe = [
                (s * 0.72, 9.22),
                (s * 0.24, 9.22),
                (s * 0.20, 9.62),
                (s * 0.42, 9.74),
                (s * 0.86, 9.70),
                (s * 0.88, 9.50),
            ]
            sm = cv.poly(PL(cr_closed(shoe, 6)))
            cv.paint(sm, cv.grad(P(-1, 9.2), P(1, 9.8), (30, 28, 30), (12, 12, 14)))
            cv.paint(cv.stroke(PL([(s * 0.50, 9.32), (s * 0.66, 9.34)]), 0.05 * u, 0.03 * u) * sm, (120, 120, 128), 0.35, solid=False)

    # ------------------------------------------------------------ jacket silhouette
    sh = [(-0.30, 1.38), (-0.68, 1.48), (-1.00, 1.64), (-1.24, 1.90), (-1.36, 2.30), (-1.37, 3.2), (-1.30, 4.2)]
    sh = cr_open(sh, 8)
    if full:
        tail = [(-1.24, 5.0), (-1.06, 5.03), (-0.98, 5.32)]
    else:
        tail = [(-1.26, 6.0), (-1.20, bottom)]
    left = sh + tail
    jm = cv.poly(PL(mirror(left)))
    cv.paint(jm, cv.grad(P(-1.5, 1.6), P(1.5, 4.5), JACKET_LIGHT, (30, 33, 41)))
    # vertical falloff to the bottom
    cv.paint(jm, (20, 22, 28), 0.35, solid=False)
    cv.a = np.maximum(cv.a, jm)

    # soft light on the left shoulder / arm, darker right arm
    cv.paint(jm * cv.ellipse(*P(-0.95, 2.05), 0.75 * u, 0.32 * u, math.radians(18), 0.16 * u), (112, 118, 132), 0.30, solid=False)
    cv.paint(jm * cv.edge_dark(jm, -0.09 * u, -0.05 * u, 0.09 * u), (10, 11, 15), 0.55, solid=False)

    # ------------------------------------------------------------ shirt V
    vpts = [(-0.30, 1.40), (0.30, 1.40), (0.30, 1.6), (0.03, 4.55), (-0.03, 4.55), (-0.30, 1.6)]
    sm_ = cv.poly(PL(vpts))
    cv.paint(sm_, cv.grad(P(-0.3, 1.5), P(0.35, 3.5), SHIRT, SHIRT_SHADE))
    # shirt soft folds
    for k, (x, y0_, y1_) in enumerate([(-0.10, 2.4, 4.2), (0.13, 2.6, 4.3)]):
        f = cv.stroke(PL([(x, y0_), (x * 1.3, (y0_ + y1_) / 2), (x * 0.5, y1_)]), 0.05 * u, 0.04 * u)
        cv.paint(f * sm_, (150, 156, 168), 0.35, solid=False)

    # ------------------------------------------------------------ neck
    neck = [(-0.215, 0.90), (0.215, 0.90), (0.245, 1.35), (0.25, 1.52), (0.0, 1.90), (-0.25, 1.52), (-0.245, 1.35)]
    nm = cv.poly(PL(cr_closed(neck, 6)))
    cv.paint(nm, cv.grad(P(-0.2, 1.2), P(0.25, 1.5), (204, 166, 142), (150, 108, 92)))
    # chin cast-shadow (bowed head => big shadow under jaw)
    sh_g = cv.grad(P(0, 1.02), P(0, 1.52), (70, 42, 38), SKIN_SHADE)
    chin_t = np.clip((cv.Y - P(0, 1.0)[1]) / (0.5 * u), 0, 1)
    cv.paint(nm * (1 - chin_t) * 0.85, (66, 40, 36), 1.0, solid=False)
    del sh_g

    # ------------------------------------------------------------ tie (loosened)
    knot = [(-0.11, 1.74), (0.11, 1.74), (0.145, 1.93), (0.10, 2.14), (-0.10, 2.14), (-0.145, 1.93)]
    blade = [(-0.10, 2.10), (0.10, 2.10), (0.15, 2.9), (0.26, 3.9), (0.09, 4.25), (-0.12, 3.9), (-0.10, 2.9)]
    bm = cv.poly(PL(cr_closed(blade, 5)))
    cv.paint(bm, cv.grad(P(-0.2, 2.1), P(0.3, 4.0), (44, 53, 76), (22, 27, 40)))
    diag = cv.stroke(PL([(-0.02, 2.3), (0.02, 2.9), (0.12, 3.9)]), 0.05 * u, 0.04 * u)
    cv.paint(diag * bm, (110, 124, 156), 0.30, solid=False)
    cv.paint(bm * cv.edge_dark(bm, -0.05 * u, 0, 0.03 * u), (6, 8, 14), 0.6, solid=False)
    km = cv.poly(PL(cr_closed(knot, 6)))
    cv.paint(km, cv.grad(P(-0.15, 1.75), P(0.15, 2.15), (56, 66, 92), (24, 29, 44)))
    cv.paint(km * cv.edge_dark(km, -0.03 * u, -0.03 * u, 0.02 * u), (6, 8, 14), 0.6, solid=False)
    # dimple under the knot
    cv.paint(cv.ellipse(*P(0, 2.2), 0.09 * u, 0.04 * u, 0, 0.02 * u) * bm, (8, 10, 18), 0.6, solid=False)
    # tie shadow on shirt
    cv.paint(cv.poly(PL(cr_closed(blade, 5)), 0.05 * u) * sm_ * (1 - bm), (60, 66, 82), 0.35, solid=False)

    # ------------------------------------------------------------ lapels
    lap_m = np.zeros((cv.h, cv.w), np.float32)
    for s in (-1, 1):
        lap = [(s * 0.30, 1.42), (s * 0.56, 1.56), (s * 0.90, 2.55), (s * 0.66, 3.05), (s * 0.03, 4.55), (s * 0.03, 4.50)]
        # inner edge follows the V of the shirt: use the same line
        lap = [(s * 0.30, 1.40), (s * 0.58, 1.55), (s * 0.92, 2.58), (s * 0.72, 3.0), (s * 0.05, 4.62), (s * 0.03, 4.55), (s * 0.30, 1.62)]
        lm = cv.poly(PL(lap))
        lap_m = np.maximum(lap_m, lm)
        # shirt receives a soft shadow from the lapel
        cv.paint(blur(shift(lm, 0.02 * u * s * -1, 0.04 * u), 0.03 * u) * sm_ * (1 - lm), (60, 64, 78), 0.55, solid=False)
        cv.paint(lm, cv.grad(P(-0.9, 1.6), P(0.9, 4.5), (62, 67, 79), (38, 42, 51)))
        # lapel sheen and edge line
        cv.paint(cv.stroke(PL([(s * 0.58, 1.6), (s * 0.90, 2.55), (s * 0.70, 3.0), (s * 0.06, 4.5)]), 0.02 * u, 0.012 * u) * lm, (130, 138, 156), 0.35, solid=False)
        cv.paint(lm * cv.edge_dark(lm, s * 0.04 * u, -0.02 * u, 0.03 * u), (12, 13, 18), 0.35, solid=False)
        # notch
        cv.paint(cv.stroke(PL([(s * 0.52, 1.54), (s * 0.60, 1.78)]), 0.012 * u, 0.008 * u), (12, 13, 18), 0.6, solid=False)

    # ------------------------------------------------------------ collar wings (loosened, splayed)
    for s in (-1, 1):
        wing = [(s * 0.19, 1.24), (s * 0.34, 1.30), (s * 0.54, 1.98), (s * 0.34, 1.90), (s * 0.15, 1.70), (s * 0.13, 1.45)]
        wm = cv.poly(PL(cr_closed(wing, 4)))
        base = cv.grad(P(-0.5, 1.3), P(0.5, 2.0), (240, 240, 236), (196, 204, 216))
        cv.paint(wm, base)
        cv.paint(wm * cv.edge_dark(wm, -0.03 * u * s, 0, 0.02 * u), (120, 126, 140), 0.55, solid=False)
        cv.paint(cv.poly(PL(wing), 0.03 * u) * 0 + wm * cv.stroke(PL([(s * 0.20, 1.40), (s * 0.16, 1.72)]), 0.03 * u, 0.02 * u), (110, 90, 84), 0.25, solid=False)
        # collar drop shadow onto jacket/lapel and shirt
        cv.paint(blur(shift(wm, 0.01 * u, 0.04 * u), 0.03 * u) * (1 - wm) * (jm), (10, 10, 14), 0.4, solid=False)

    # ------------------------------------------------------------ sleeve seams, shoulder seams, folds
    for s in (-1, 1):
        seam = cr_open(PL([(s * 1.20, 1.84), (s * 1.12, 2.4), (s * 1.02, 3.0), (s * 1.00, 4.0), (s * 0.98, 5.0)]), 8)
        st = cv.stroke(seam, 0.028 * u, 0.014 * u)
        cv.paint(st * jm, (8, 9, 12), 0.75, solid=False)
        hl = cv.stroke([(x + s * 0.03 * u, y) for x, y in seam], 0.014 * u, 0.01 * u)
        cv.paint(hl * jm, (120, 128, 146), 0.14 if s < 0 else 0.07, solid=False)
        # shoulder seam
        ss_ = cv.stroke(PL([(s * 0.62, 1.58), (s * 1.10, 1.80)]), 0.014 * u, 0.008 * u)
        cv.paint(ss_ * jm, (8, 9, 12), 0.5, solid=False)
        # elbow folds
        for yy in (3.3, 3.55, 3.8):
            fo = cv.stroke(PL([(s * 1.46, yy), (s * 1.28, yy + 0.10), (s * 1.10, yy + 0.05)]), 0.03 * u, 0.03 * u)
            cv.paint(fo * jm, (8, 9, 12), 0.35, solid=False)
            fh = cv.stroke(PL([(s * 1.46, yy + 0.04), (s * 1.28, yy + 0.14), (s * 1.10, yy + 0.09)]), 0.02 * u, 0.02 * u)
            cv.paint(fh * jm, (120, 128, 146), 0.10, solid=False)
        # chest fold near armpit
        fo = cv.stroke(PL([(s * 0.95, 2.5), (s * 0.80, 3.1), (s * 0.72, 3.6)]), 0.04 * u, 0.04 * u)
        cv.paint(fo * jm * (1 - lap_m), (8, 9, 12), 0.35, solid=False)
    # button + jacket closing edge at the bottom
    cv.paint(cv.ellipse(*P(0.0, 4.55), 0.05 * u, 0.05 * u, 0, 0.005 * u), (20, 22, 26), 1.0, solid=False)
    cv.paint(cv.ellipse(*P(-0.012, 4.54), 0.02 * u, 0.02 * u, 0, 0.01 * u), (150, 156, 170), 0.5, solid=False)
    below = cv.stroke(PL([(0.0, 4.62), (0.0, bottom if not full else 5.3)]), 0.022 * u, 0.015 * u)
    cv.paint(below * jm, (6, 7, 10), 0.6, solid=False)

    # ------------------------------------------------------------ hands (full)
    if full:
        for s in (-1, 1):
            hand = [(s * 1.30, 4.98), (s * 1.06, 4.98), (s * 1.02, 5.35), (s * 1.06, 5.62), (s * 1.20, 5.72), (s * 1.32, 5.5), (s * 1.34, 5.2)]
            hm = cv.poly(PL(cr_closed(hand, 6)))
            cv.paint(hm, cv.grad(P(-1.3, 5), P(1.3, 5.7), (208, 170, 146), (150, 108, 92)))
            cv.paint(hm * cv.edge_dark(hm, -0.03 * u, -0.02 * u, 0.02 * u), (100, 64, 56), 0.5, solid=False)
            # shirt cuff
            cf = cv.poly(PL([(s * 1.38, 4.90), (s * 1.10, 4.90), (s * 1.10, 5.02), (s * 1.36, 5.04)]))
            cv.paint(cf, (226, 226, 222))
            cv.paint(cv.stroke(PL([(s * 1.24, 5.2), (s * 1.24, 5.6)]), 0.012 * u, 0.008 * u) * hm, (90, 56, 48), 0.4, solid=False)

    # ------------------------------------------------------------ ears
    for s in (-1, 1):
        em = cv.ellipse(*PH(s * 0.485, 0.72), 0.075 * u, 0.155 * u, s * math.radians(-6) + ang, 0.004 * u)
        cv.paint(em, cv.grad(PH(-0.55, 0.6), PH(0.55, 0.9), (206, 168, 144), (138, 98, 84)))
        inner = cv.ellipse(*PH(s * 0.485 + s * 0.006, 0.73), 0.035 * u, 0.09 * u, 0, 0.012 * u)
        cv.paint(inner * em, (120, 78, 68), 0.45, solid=False)

    # ------------------------------------------------------------ face
    face = [(0, 0.10), (0.30, 0.15), (0.45, 0.42), (0.47, 0.70), (0.42, 0.98), (0.28, 1.20), (0.11, 1.30), (0.0, 1.32)]
    fm = cv.poly(PHL(cr_closed(mirror_face(face), 7)))
    cv.paint(fm, cv.grad(PH(-0.5, 0.3), PH(0.5, 1.1), (228, 194, 170), (170, 128, 108)))
    # forehead highlight, blush, cheeks
    cv.paint(fm * cv.ellipse(*PH(-0.16, 0.50), 0.20 * u, 0.13 * u, 0, 0.08 * u), (244, 220, 200), 0.45, solid=False)
    cv.paint(fm * cv.ellipse(*PH(-0.22, 0.92), 0.14 * u, 0.09 * u, 0, 0.06 * u), (206, 132, 118), 0.22, solid=False)
    cv.paint(fm * cv.ellipse(*PH(0.24, 0.92), 0.12 * u, 0.09 * u, 0, 0.06 * u), (150, 96, 84), 0.25, solid=False)
    cv.paint(fm * cv.edge_dark(fm, -0.07 * u, -0.03 * u, 0.06 * u), (100, 62, 54), 0.55, solid=False)
    cv.paint(fm * cv.edge_dark(fm, 0, -0.08 * u, 0.06 * u), (100, 62, 54), 0.35, solid=False)
    # eyes in shadow (head bowed -> eye sockets dark)
    for s in (-1, 1):
        sock = cv.ellipse(*PH(s * 0.18, 0.70), 0.13 * u, 0.055 * u, s * math.radians(4), 0.03 * u)
        cv.paint(sock * fm, (84, 50, 46), 0.55, solid=False)
        lid = cv.stroke(PHL([(s * 0.10, 0.715), (s * 0.18, 0.725), (s * 0.26, 0.712)]), 0.016 * u, 0.006 * u)
        cv.paint(lid * fm, (36, 22, 20), 0.85, solid=False)
        brow = cv.stroke(PHL(cr_open([(s * 0.07, 0.625), (s * 0.17, 0.598), (s * 0.29, 0.62)], 6)), 0.036 * u, 0.008 * u)
        cv.paint(brow * fm, (40, 28, 26), 0.9, solid=False)
    # cheekbones, temples, jaw
    cv.paint(fm * cv.ellipse(*PH(-0.25, 0.80), 0.12 * u, 0.06 * u, math.radians(-15), 0.05 * u), (246, 224, 205), 0.40, solid=False)
    cv.paint(fm * cv.ellipse(*PH(0.27, 1.00), 0.13 * u, 0.09 * u, math.radians(25), 0.06 * u), (96, 58, 50), 0.32, solid=False)
    cv.paint(fm * cv.ellipse(*PH(-0.36, 0.55), 0.05 * u, 0.12 * u, 0, 0.04 * u), (120, 80, 70), 0.25, solid=False)
    cv.paint(fm * cv.ellipse(*PH(0.38, 0.55), 0.05 * u, 0.12 * u, 0, 0.04 * u), (100, 62, 54), 0.35, solid=False)
    cv.paint(fm * cv.stroke(PHL([(-0.07, 0.70), (-0.05, 0.86)]), 0.03 * u, 0.02 * u), (246, 224, 205), 0.40, solid=False)
    # nose: shadow side + tip highlight + nostril shadow
    nose = cv.stroke(PHL([(0.055, 0.72), (0.075, 0.86)]), 0.04 * u, 0.025 * u)
    cv.paint(nose * fm, (104, 62, 54), 0.60, solid=False)
    cv.paint(cv.ellipse(*PH(-0.005, 0.90), 0.04 * u, 0.03 * u, 0, 0.012 * u) * fm, (240, 214, 194), 0.5, solid=False)
    cv.paint(cv.ellipse(*PH(0.02, 0.965), 0.075 * u, 0.02 * u, 0, 0.01 * u) * fm, (92, 56, 50), 0.55, solid=False)
    # mouth
    cv.paint(cv.stroke(PHL(cr_open([(-0.11, 1.075), (0.0, 1.09), (0.11, 1.07)], 6)), 0.014 * u, 0.007 * u) * fm, (110, 62, 60), 0.75, solid=False)
    cv.paint(cv.ellipse(*PH(0.0, 1.14), 0.08 * u, 0.02 * u, 0, 0.012 * u) * fm, (110, 70, 62), 0.25, solid=False)
    # a hint of five o'clock shadow on jaw
    stub = rng.random((cv.h, cv.w), dtype=np.float32) if False else None
    del stub

    # ------------------------------------------------------------ hair
    outer = [(0.0, -0.06), (0.32, -0.02), (0.53, 0.12), (0.61, 0.38), (0.56, 0.68), (0.47, 0.74)]
    inner_r = [(0.44, 0.56), (0.41, 0.38), (0.26, 0.31), (0.06, 0.34)]
    # asymmetric side-swept fringe on the left, short sides
    hair_pts = (
        outer
        + inner_r
        + [(-0.10, 0.44), (-0.28, 0.40), (-0.42, 0.44), (-0.44, 0.60), (-0.47, 0.74)]
        + [(-0.56, 0.68), (-0.61, 0.40), (-0.53, 0.12), (-0.32, -0.03)]
    )
    hm = cv.poly(PHL(cr_closed(hair_pts, 8)), 0.8 * SS)
    cv.paint(hm, cv.grad(PH(-0.6, 0.0), PH(0.6, 0.6), (56, 49, 49), (22, 19, 20)))
    # sheen on the upper left, deeper dark on the right
    sheen = cv.stroke(PHL(cr_open([(-0.40, 0.30), (-0.22, 0.10), (0.05, 0.03), (0.28, 0.10)], 8)), 0.11 * u, 0.06 * u)
    cv.paint(sheen * hm, (128, 116, 110), 0.55, solid=False)
    # strands
    for i in range(46):
        t = i / 45.0
        x0_ = -0.5 + t * 1.0 + rng.uniform(-0.03, 0.03)
        sx, sy = 0.0 + rng.uniform(-0.08, 0.08), 0.02 + rng.uniform(0, 0.05)
        ex = x0_ * 1.05
        ey = 0.34 + rng.uniform(-0.05, 0.16) + 0.06 * abs(x0_)
        mx = (sx + ex) / 2 + rng.uniform(-0.05, 0.05) - 0.04
        my = (sy + ey) / 2 - 0.05
        line = PHL(cr_open([(sx, sy), (mx, my), (ex, ey)], 6))
        light = rng.random() < 0.55
        cv.paint(cv.stroke(line, 0.010 * u, 0.004 * u) * hm, (150, 138, 130) if light else (8, 6, 7), 0.22 if light else 0.5, solid=False)
    # fringe cast shadow on forehead (also darkens where hair curves away)
    fringe_sh = blur(shift(hm, 0, 0.055 * u), 0.04 * u)
    cv.paint(fringe_sh * fm * (1 - hm), (66, 40, 36), 0.65, solid=False)
    # flyaway strands (cutout: solid alpha)
    for i in range(5):
        a0 = rng.uniform(-0.45, 0.45)
        sx, sy = a0, 0.02 + 0.10 * (abs(a0) * 1.3)
        dx, dy = rng.uniform(-0.06, 0.06) + a0 * 0.15, -rng.uniform(0.03, 0.07)
        line = PHL(cr_open([(sx, sy), (sx + dx * 0.5, sy + dy * 1.2), (sx + dx, sy + dy)], 6))
        cv.paint(cv.stroke(line, 0.006 * u, 0.0), (26, 22, 23), 0.7)
    # right-side hair rim / darker back
    cv.paint(hm * cv.edge_dark(hm, -0.04 * u, -0.02 * u, 0.04 * u), (6, 5, 6), 0.5, solid=False)

    # head box from the hair + face masks (canvas px)
    both = np.maximum(hm, fm) > 0.5
    ys, xs = np.where(both)
    hb = (xs.min(), ys.min(), xs.max() + 1, ys.max() + 1)
    return hb


def mirror_face(right_half):
    """Right half points (x>=0, top->bottom) -> full closed outline."""
    left = [(-x, y) for x, y in reversed(right_half[1:-1])]
    return list(right_half) + left


def finish_person(cv, rng, u, cx):
    """Global lighting: key from upper-left, cool rim on the right, fabric noise."""
    a = np.clip(cv.a, 0, 1)
    # key light falloff
    fall = cv.grad((cx - 1.6 * u, 0), (cx + 1.6 * u, 0), (255, 255, 255), (206, 210, 222))[..., 0]
    cv.rgb *= (0.64 + 0.36 * fall)[..., None] * 1.08
    # fabric noise (fine) + soft mottling
    n = rng.standard_normal((cv.h, cv.w)).astype(np.float32) * 0.030
    mott = blur(np.clip(rng.random((cv.h, cv.w), dtype=np.float32), 0, 1), 6.0) - 0.5
    cv.rgb += ((n + mott * 0.12) * (0.4 + 0.6 * (1 - cv.rgb.mean(-1))))[..., None]
    # right rim light (cool screen / night), thin
    sh1 = blur(shift(a, -0.028 * u, 0), 0.006 * u)
    rim = np.clip(a * (1 - sh1), 0, 1)
    rim = blur(rim, 0.006 * u) * 1.6
    cv.rgb += (rim[..., None] * np.array([0.32, 0.50, 0.72], np.float32)) * 0.85
    sh2 = blur(shift(a, -0.012 * u, 0.004 * u), 0.004 * u)
    rim2 = np.clip(a * (1 - sh2), 0, 1)
    cv.rgb += (blur(rim2, 0.004 * u)[..., None] * np.array([0.35, 0.55, 0.80], np.float32)) * 0.55
    # warm faint edge light on the upper-left
    sh3 = blur(shift(a, 0.02 * u, 0.02 * u), 0.006 * u)
    warm = np.clip(a * (1 - sh3), 0, 1)
    cv.rgb += (warm[..., None] * np.array([0.30, 0.24, 0.16], np.float32)) * 0.30
    cv.rgb = np.clip(cv.rgb, 0, 1)


def render_person(framing, W, H, rng, defect):
    cv = Canvas(W, H)
    if framing == "half_figure":
        mode, u, top = "half", 0.27 * W * SS, 0.075 * H * SS
    elif framing == "head_and_shoulders":
        mode, u, top = "hs", 0.31 * W * SS, 0.075 * H * SS
    else:
        mode, u, top = "full", 0.152 * W * SS, 0.045 * H * SS
    cx = 0.5 * W * SS
    if defect == "head_cropped":
        top = -0.30 * u
    hb = draw_person(cv, cx, top, u, rng, mode)
    finish_person(cv, rng, u, cx)
    img = to_image(cv)
    x0, y0_, x1, y1 = hb
    sw, sh = cv.w, cv.h
    nx0, ny0 = max(0.0, x0 / sw), max(0.0, y0_ / sh)
    nx1, ny1 = min(1.0, x1 / sw), min(1.0, y1 / sh)
    box = {"x": round(nx0, 4), "y": round(ny0, 4), "width": round(nx1 - nx0, 4), "height": round(ny1 - ny0, 4)}
    return img, box


# ---------------------------------------------------------------------------
# object: a coffee mug
# ---------------------------------------------------------------------------
def render_object(W, H, rng, defect):
    cv = Canvas(W, H)
    cx, cy = 0.5 * W * SS, 0.52 * H * SS
    r = 0.24 * W * SS  # body half-width
    hgt = 0.42 * H * SS
    top_y = cy - hgt / 2
    bot_y = cy + hgt / 2
    ery = r * 0.26  # perspective ellipse half-height

    # handle (ring at right)
    hx, hy = cx + r * 0.98, cy - 0.02 * hgt
    ring_o = cv.ellipse(hx + r * 0.16, hy, r * 0.44, hgt * 0.27, 0, 0)
    ring_i = cv.ellipse(hx + r * 0.16, hy, r * 0.25, hgt * 0.17, 0, 0)
    handle = np.clip(ring_o - ring_i, 0, 1)
    cv.paint(handle, cv.grad((hx - r * 0.3, 0), (hx + r * 0.7, 0), (226, 224, 218), (120, 128, 142)))
    cv.paint(handle * cv.edge_dark(handle, -0.03 * r, -0.03 * r, 0.03 * r), (70, 72, 82), 0.6, solid=False)

    # body: rounded cylinder
    body = [(cx - r, top_y), (cx + r, top_y), (cx + r * 0.94, bot_y - ery * 0.2), (cx + r * 0.5, bot_y + ery * 0.85), (cx - r * 0.5, bot_y + ery * 0.85), (cx - r * 0.94, bot_y - ery * 0.2)]
    bm = np.maximum(cv.poly(cr_closed(body, 10) if False else body), cv.ellipse(cx, bot_y - ery * 0.1, r * 0.94, ery * 0.95, 0, 0))
    bm = np.maximum(bm, cv.ellipse(cx, top_y, r, ery, 0, 0))
    # cylindrical shading: bright left-of-centre, dark right, rim on far right
    xs = (cv.X - cx) / r
    xs = np.clip(xs, -1, 1)
    shade = 0.62 + 0.38 * np.cos((xs + 0.30) * 1.25)
    shade = np.clip(shade, 0.2, 1.0)
    base = np.array([228, 226, 220], np.float32) / 255.0
    col = base * shade[..., None]
    col[..., 2] += 0.03 * (1 - shade)  # cool shadows
    cv.paint(bm, col)
    # specular streak
    cv.paint(bm * cv.stroke([(cx - r * 0.52, top_y + ery * 1.4), (cx - r * 0.48, bot_y - ery * 0.6)], r * 0.10, r * 0.05), (255, 255, 255), 0.55, solid=False)
    # red band (the signal-red accent)
    band_t = top_y + hgt * 0.42
    band = bm * np.clip(1 - np.abs(cv.Y - (band_t + hgt * 0.07)) / (hgt * 0.07), 0, 1) ** 0.15 * (np.abs(cv.Y - (band_t + hgt * 0.07)) < hgt * 0.07)
    bandcol = np.array([176, 44, 42], np.float32) / 255.0 * shade[..., None]
    cv.paint(band, bandcol, 0.92, solid=False)
    # right rim light
    a = np.clip(cv.a, 0, 1)
    rim = np.clip(bm * (1 - blur(shift(bm, -0.05 * r, 0), 0.02 * r)), 0, 1)
    cv.rgb += blur(rim, 0.02 * r)[..., None] * np.array([0.30, 0.48, 0.70], np.float32) * 0.9
    del a

    # opening: rim ellipse, interior, coffee
    rim_o = cv.ellipse(cx, top_y, r, ery, 0, 0.5)
    cv.paint(rim_o, cv.grad((cx - r, 0), (cx + r, 0), (240, 238, 232), (170, 176, 188)))
    inner = cv.ellipse(cx, top_y + ery * 0.03, r * 0.92, ery * 0.86, 0, 0.5)
    cv.paint(inner, cv.grad((cx - r, 0), (cx + r, 0), (172, 168, 160), (110, 108, 108)), solid=False)
    coffee = cv.ellipse(cx, top_y + ery * 0.22, r * 0.84, ery * 0.68, 0, 0.5)
    cv.paint(coffee, cv.grad((cx - r, top_y - ery), (cx + r, top_y + ery), (70, 42, 28), (28, 16, 12)), solid=False)
    cv.paint(coffee * cv.ellipse(cx - r * 0.35, top_y + ery * 0.05, r * 0.35, ery * 0.2, 0, r * 0.03), (150, 100, 70), 0.5, solid=False)
    # ground contact darkening on the underside
    cv.paint(bm * cv.edge_dark(bm, 0, -0.05 * r, 0.06 * r), (40, 40, 50), 0.4, solid=False)
    # noise
    cv.rgb += (rng.standard_normal((cv.h, cv.w)).astype(np.float32) * 0.012)[..., None]
    cv.rgb = np.clip(cv.rgb, 0, 1)
    return to_image(cv), None


# ---------------------------------------------------------------------------
# scene: night office window
# ---------------------------------------------------------------------------
def render_scene(W, H, rng, defect):
    cv = Canvas(W, H)
    cv.a[:] = 1.0
    cv.rgb = cv.grad((0, 0), (0, cv.h), (8, 13, 28), (34, 52, 84)).astype(np.float32)
    # distant skyline silhouettes with lit windows
    horizon = cv.h * 0.60
    sk = np.zeros((cv.h, cv.w), np.float32)
    x = 0.0
    while x < cv.w:
        bw = rng.uniform(0.05, 0.13) * cv.w
        bh = rng.uniform(0.05, 0.20) * cv.h
        sk = np.maximum(sk, cv.poly([(x, horizon - bh), (x + bw, horizon - bh), (x + bw, cv.h), (x, cv.h)]))
        # lit windows
        for _ in range(int(bw * bh / 9000)):
            wx = x + rng.uniform(0.08, 0.9) * bw
            wy = horizon - bh + rng.uniform(0.05, 1.0) * bh
            if wy < horizon:
                wm = cv.poly([(wx, wy), (wx + 7 * SS, wy), (wx + 7 * SS, wy + 9 * SS), (wx, wy + 9 * SS)])
                cv.paint(wm, (250, 220, 150) if rng.random() < 0.6 else (170, 210, 250), 0.35, solid=False)
        x += bw * rng.uniform(0.85, 1.0)
    sk_b = blur(sk, 3 * SS)
    cv.paint(sk_b, (16, 24, 42), 0.55, solid=False)
    # bokeh city lights
    bok = np.zeros((cv.h, cv.w, 3), np.float32)
    for _ in range(70):
        bx = rng.uniform(0, cv.w)
        by = rng.uniform(horizon - 0.14 * cv.h, cv.h * 0.86)
        br = rng.uniform(10, 34) * SS
        colr = [(255, 214, 140), (150, 200, 255), (255, 170, 120), (190, 230, 255)][int(rng.integers(0, 4))]
        m = cv.ellipse(bx, by, br, br, 0, 1.5 * SS)
        cv.paint(m, colr, float(rng.uniform(0.10, 0.28)), solid=False)
    del bok
    # window mullion grid
    grid = np.zeros((cv.h, cv.w), np.float32)
    for gx in (0.0, 0.34, 0.68, 1.0):
        gxp = gx * cv.w
        grid = np.maximum(grid, cv.poly([(gxp - 12 * SS, 0), (gxp + 12 * SS, 0), (gxp + 12 * SS, cv.h), (gxp - 12 * SS, cv.h)]))
    for gy in (0.0, 0.45, 0.76):
        gyp = gy * cv.h
        grid = np.maximum(grid, cv.poly([(0, gyp - 10 * SS), (cv.w, gyp - 10 * SS), (cv.w, gyp + 10 * SS), (0, gyp + 10 * SS)]))
    cv.paint(blur(grid, 1.2 * SS), (10, 14, 24), 0.85, solid=False)
    hl = np.zeros_like(grid)
    for gx in (0.34, 0.68):
        gxp = gx * cv.w - 12 * SS
        hl = np.maximum(hl, cv.poly([(gxp - 2 * SS, 0), (gxp, 0), (gxp, cv.h), (gxp - 2 * SS, cv.h)]))
    cv.paint(blur(hl, SS), (110, 140, 190), 0.18, solid=False)
    # desk edge silhouette in the foreground
    desk = cv.poly([(0, cv.h * 0.90), (cv.w * 0.35, cv.h * 0.885), (cv.w, cv.h * 0.90), (cv.w, cv.h), (0, cv.h)])
    cv.paint(blur(desk, 1.0 * SS), (9, 11, 18), 0.96)
    edge = cv.stroke([(0, cv.h * 0.90), (cv.w * 0.35, cv.h * 0.885), (cv.w, cv.h * 0.90)], 3 * SS, 1.5 * SS)
    cv.paint(edge, (70, 92, 130), 0.30, solid=False)
    # monitor glow spill (cool) low on the right
    glow = cv.ellipse(cv.w * 0.86, cv.h * 0.93, cv.w * 0.35, cv.h * 0.12, 0, 60 * SS)
    cv.paint(glow, (90, 140, 220), 0.16, solid=False)
    # vignette + grain (low contrast overall)
    vig = 1.0 - 0.28 * (((cv.X / cv.w - 0.5) ** 2 + (cv.Y / cv.h - 0.5) ** 2) * 2.0)
    cv.rgb *= vig[..., None]
    cv.rgb += (rng.standard_normal((cv.h, cv.w)).astype(np.float32) * 0.010)[..., None]
    cv.rgb = np.clip(cv.rgb, 0, 1)
    return opaque_to_image(cv), None


# ---------------------------------------------------------------------------
# artifact: an off-white paper document
# ---------------------------------------------------------------------------
def render_artifact(W, H, rng, defect):
    cv = Canvas(W, H)
    cv.a[:] = 1.0
    cv.rgb = cv.grad((0, 0), (cv.w, cv.h), (244, 240, 230), (232, 227, 214)).astype(np.float32)
    # paper fibre / grain
    grain = blur(rng.random((cv.h, cv.w), dtype=np.float32), 1.5) - 0.5
    cv.rgb += (grain * 0.10)[..., None]
    cv.rgb += (rng.standard_normal((cv.h, cv.w)).astype(np.float32) * 0.008)[..., None]
    mx, my = 0.10 * cv.w, 0.09 * cv.h

    def bar(x, y, w, h, col, op):
        m = cv.poly([(x, y), (x + w, y), (x + w, y + h), (x, y + h)], 0.6 * SS)
        cv.paint(m, col, op, solid=False)

    # letterhead block + title
    bar(mx, my, 0.22 * cv.w, 0.028 * cv.h, (96, 98, 104), 0.85)
    bar(cv.w - mx - 0.16 * cv.w, my, 0.16 * cv.w, 0.010 * cv.h, (150, 152, 158), 0.7)
    bar(cv.w - mx - 0.12 * cv.w, my + 0.02 * cv.h, 0.12 * cv.w, 0.010 * cv.h, (150, 152, 158), 0.7)
    bar(mx, my + 0.075 * cv.h, cv.w - 2 * mx, 2 * SS, (120, 122, 128), 0.6)
    bar(mx, my + 0.11 * cv.h, 0.55 * cv.w, 0.030 * cv.h, (70, 72, 80), 0.88)
    # paragraphs of text-like bars
    y = my + 0.18 * cv.h
    for para in range(3):
        for line in range(int(rng.integers(5, 8))):
            w = (cv.w - 2 * mx) * (1.0 if line < 4 else rng.uniform(0.35, 0.8))
            w *= rng.uniform(0.94, 1.0) if w > 0.9 * (cv.w - 2 * mx) else 1.0
            bar(mx, y, w, 0.0125 * cv.h, (128, 130, 136), 0.62)
            y += 0.0225 * cv.h
        y += 0.03 * cv.h
        if para == 1:
            # table block
            ty = y
            for r_ in range(4):
                for c_ in range(3):
                    cw = (cv.w - 2 * mx) / 3
                    bar(mx + c_ * cw + 6 * SS, ty + r_ * 0.035 * cv.h, cw * rng.uniform(0.35, 0.7), 0.011 * cv.h, (112, 114, 120), 0.6)
                bar(mx, ty + (r_ + 1) * 0.035 * cv.h - 0.008 * cv.h, cv.w - 2 * mx, 1.5 * SS, (170, 170, 174), 0.6)
            y = ty + 4.4 * 0.035 * cv.h
    # signature line + stamp-like red block (an abstract mark, no letters)
    bar(mx, cv.h * 0.90, 0.30 * cv.w, 1.5 * SS, (90, 92, 98), 0.7)
    stamp = cv.ellipse(cv.w * 0.76, cv.h * 0.87, 0.10 * cv.w, 0.10 * cv.w, 0, 0.8 * SS)
    ring = np.clip(stamp - cv.ellipse(cv.w * 0.76, cv.h * 0.87, 0.087 * cv.w, 0.087 * cv.w, 0, 0.8 * SS), 0, 1)
    cv.paint(ring * (0.75 + 0.25 * rng.random((cv.h, cv.w), dtype=np.float32)), (176, 60, 56), 0.55, solid=False)
    # a soft fold shadow and edge falloff
    fold = cv.stroke([(0, cv.h * 0.50), (cv.w, cv.h * 0.505)], 26 * SS, 14 * SS)
    cv.paint(fold, (120, 112, 96), 0.10, solid=False)
    vig = 1.0 - 0.10 * (((cv.X / cv.w - 0.5) ** 2 + (cv.Y / cv.h - 0.5) ** 2) * 2.0)
    cv.rgb *= vig[..., None]
    cv.rgb = np.clip(cv.rgb, 0, 1)
    return opaque_to_image(cv), None


# ---------------------------------------------------------------------------
# driver
# ---------------------------------------------------------------------------
def fingerprint_seed(fp):
    hexpart = fp.split("-", 1)[-1]
    try:
        return int(hexpart, 16)
    except ValueError:
        return int.from_bytes(fp.encode("utf-8")[:8].ljust(8, b"\0"), "big")


def generate_spec(spec, out_dir, defect, with_head):
    framing = spec["composition"]["framing"]
    if framing not in FRAMINGS:
        raise SystemExit(f"unsupported framing {framing!r} in spec {spec['id']}")
    W, H, kind = FRAMINGS[framing]
    seed = fingerprint_seed(spec["fingerprint"])
    rng = np.random.default_rng(seed)
    out = spec["output"]
    alpha_required = bool(out.get("alpha_required"))
    stem = out["file_stem"]

    box = None
    if kind == "person":
        img, box = render_person(framing, W, H, rng, defect)
    elif kind == "object":
        img, box = render_object(W, H, rng, defect)
    elif kind == "scene":
        img, box = render_scene(W, H, rng, defect)
    else:
        img, box = render_artifact(W, H, rng, defect)

    if not alpha_required and img.mode == "RGBA":
        bg = Image.new("RGBA", img.size, (238, 236, 230, 255))
        img = Image.alpha_composite(bg, img).convert("RGB")
    if alpha_required and defect == "no_alpha":
        bg = Image.new("RGBA", img.size, (255, 255, 255, 255))
        img = Image.alpha_composite(bg, img.convert("RGBA")).convert("RGB")
    if defect == "too_small":
        w, h = img.size
        s = 400.0 / min(w, h)
        img = img.resize((max(1, round(w * s)), max(1, round(h * s))), Image.LANCZOS)

    if alpha_required:
        path = os.path.join(out_dir, stem + ".png")
        img.save(path, format="PNG", compress_level=6, optimize=False)
    else:
        path = os.path.join(out_dir, stem + ".jpg")
        img.convert("RGB").save(path, format="JPEG", quality=90, subsampling=2, optimize=False, progressive=False)

    sidecar = {
        "fingerprint": spec["fingerprint"],
        "generator": {
            "tool": "motionengine-fixture-generator",
            "kind": "procedural stand-in, not an AI image",
            "spec_id": spec["id"],
            "framing": framing,
            "seed": seed,
        },
    }
    if defect != "none":
        sidecar["generator"]["defect"] = defect
    if with_head and kind == "person" and box is not None:
        sidecar["head_bounds"] = box
    with open(os.path.join(out_dir, stem + ".json"), "w", encoding="utf-8") as f:
        json.dump(sidecar, f, indent=2, sort_keys=True)
        f.write("\n")
    return path


def main(argv=None):
    ap = argparse.ArgumentParser(description="Procedural stand-in for an AI image generator (fixture producer).")
    ap.add_argument("prompts", help="asset-prompts.json")
    ap.add_argument("delivery", help="delivery directory to write into")
    ap.add_argument("--defect", choices=["none", "head_cropped", "too_small", "no_alpha"], default="none")
    ap.add_argument("--only", metavar="SPEC_ID", help="only generate this spec id")
    ap.add_argument("--with-head-metadata", action="store_true", help="also write head_bounds into the sidecar")
    args = ap.parse_args(argv)

    with open(args.prompts, "r", encoding="utf-8") as f:
        doc = json.load(f)
    specs = doc.get("specs", [])
    if args.only:
        specs = [s for s in specs if s["id"] == args.only]
        if not specs:
            print(f"no spec with id {args.only!r}", file=sys.stderr)
            return 1
    os.makedirs(args.delivery, exist_ok=True)
    for spec in specs:
        path = generate_spec(spec, args.delivery, args.defect, args.with_head_metadata)
        print(f"{spec['id']}: wrote {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
