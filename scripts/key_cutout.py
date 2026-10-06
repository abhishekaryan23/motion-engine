#!/usr/bin/env python3
"""Chroma-key a generated cutout shot on a flat high-contrast ground (magenta by
default) into a tight RGBA PNG, outside the engine.

Writes <out>.png cropped to the actual alpha bounds (+ small pad) and
<out>.key.json with the cutout coordinates in the SOURCE image and in the
cropped output, so placement can use the real subject extent, not the photo.

    scripts/key_cutout.py in.png out_stem [--pad 0.02]

Background = near-ground pixels connected to the border, plus enclosed pockets
that contain near-exact ground colour (the family palette has no magenta).
Deterministic.
"""
import argparse, json, os, sys

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import despill_magenta  # noqa: E402
from PIL import Image
from scipy import ndimage

LO, HI = 0.18, 0.38   # colour distance: <= LO background, >= HI foreground (soft band between)
CORE = 0.08           # an enclosed region holding pixels this close to the ground is ground too
MIN_ISLAND = 0.0015   # foreground islands smaller than this fraction of the frame are dropped


def key(path, out_stem, pad_frac=0.06, min_short=512):
    src = Image.open(path).convert("RGB")
    rgb = np.asarray(src).astype(np.float32) / 255.0
    h, w, _ = rgb.shape
    border = np.concatenate([rgb[0], rgb[-1], rgb[:, 0], rgb[:, -1]])
    bg = np.median(border, axis=0)
    dist = np.sqrt(((rgb - bg) ** 2).sum(-1)) / np.sqrt(3.0)

    # Background = near-bg pixels connected to the border.
    near = dist < HI
    lab, _ = ndimage.label(near)
    edge_labels = np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]]))
    edge_labels = edge_labels[edge_labels > 0]
    # Enclosed pockets (between stems, inside handles) count when they hold
    # near-exact ground colour; the family palette never contains it.
    core_labels = np.unique(lab[dist < CORE])
    core_labels = core_labels[core_labels > 0]
    bg_region = np.isin(lab, np.union1d(edge_labels, core_labels))
    alpha = np.ones((h, w), np.float32)
    t = np.clip((dist - LO) / (HI - LO), 0.0, 1.0)
    alpha[bg_region] = (t * t * (3 - 2 * t))[bg_region]

    # Drop tiny foreground islands (noise, stray specks).
    fg = alpha > 0.5
    flab, n = ndimage.label(fg)
    if n:
        sizes = ndimage.sum(fg, flab, range(1, n + 1))
        small = np.isin(flab, 1 + np.nonzero(sizes < MIN_ISLAND * h * w)[0])
        alpha[small] = 0.0

    # Despill: pull the ground colour out of semi-transparent edge pixels.
    edge = (alpha < 0.999) & (alpha > 0.0)
    edge = ndimage.binary_dilation(edge, iterations=2) & (alpha > 0.0)
    out = rgb.copy()
    a = alpha[..., None]
    unmixed = np.clip((rgb - (1 - a) * bg) / np.maximum(a, 1e-3), 0.0, 1.0)
    out[edge] = unmixed[edge]
    # residual magenta cast: limit R and B to G + small margin on edge pixels
    if bg[0] > 0.6 and bg[2] > 0.6 and bg[1] < 0.4:
        m = np.maximum(out[..., 1] + 0.06, 0)
        for c in (0, 2):
            ch = out[..., c]
            ch[edge] = np.minimum(ch[edge], np.maximum(m[edge], out[..., 1][edge]))

    # Global magenta suppression (the palette has none): strongly magenta pixels
    # are ground seen through gaps; mildly magenta ones are spill.
    if bg[0] > 0.6 and bg[2] > 0.6 and bg[1] < 0.4:
        mag = np.minimum(out[..., 0], out[..., 2]) - out[..., 1]
        alpha[mag > 0.42] = 0.0
        cast = (mag > 0.08) & (alpha > 0.0)
        lim = out[..., 1] + 0.08
        for c in (0, 2):
            ch = out[..., c]
            ch[cast] = np.minimum(ch[cast], lim[cast])

    # Tight crop to alpha bounds + pad.
    ys, xs = np.nonzero(alpha > 0.04)
    if len(xs) == 0:
        sys.exit(f"{path}: nothing left after keying")
    x0, x1, y0, y1 = int(xs.min()), int(xs.max()) + 1, int(ys.min()), int(ys.max()) + 1
    pad = int(round(pad_frac * max(x1 - x0, y1 - y0)))
    cx0, cy0, cx1, cy1 = max(0, x0 - pad), max(0, y0 - pad), min(w, x1 + pad), min(h, y1 + pad)
    rgba = np.dstack([out, alpha])[cy0:cy1, cx0:cx1]
    # Thin subjects: pad with transparency up to the ingest minimum short side
    # (never upscale); the recorded bbox still covers only the subject.
    ph, pw = rgba.shape[:2]
    ex = max(0, min_short - pw) if min(pw, ph) == pw and pw < min_short else 0
    ey = max(0, min_short - ph) if min(pw, ph) == ph and ph < min_short else 0
    if ex or ey:
        canvas = np.zeros((ph + ey, pw + ex, 4), np.float32)
        canvas[ey // 2: ey // 2 + ph, ex // 2: ex // 2 + pw] = rgba
        rgba = canvas
        cx0 -= ex // 2
        cy0 -= ey // 2
    if bg[0] > 0.6 and bg[2] > 0.6 and bg[1] < 0.4:
        # Final guard shared with the sprite extractor: no key residue or cast.
        rgba = despill_magenta.clean(rgba)
    Image.fromarray((rgba * 255 + 0.5).astype(np.uint8), "RGBA").save(out_stem + ".png", optimize=True)

    ch, cw = rgba.shape[:2]
    cx1, cy1 = cx0 + cw, cy0 + ch
    coverage = float((alpha > 0.5).sum()) / float(cw * ch)
    solid = alpha > 0.98
    spill = float(((out[..., 0] > out[..., 1] + 0.25) & (out[..., 2] > out[..., 1] + 0.25) & solid).sum()) / max(1, solid.sum())
    touches = bool(x0 == 0 or y0 == 0 or x1 == w or y1 == h)
    info = {
        "source": {"width": w, "height": h},
        "background_rgb": [round(float(v) * 255) for v in bg],
        "cutout_bbox_source_px": [x0, y0, x1, y1],
        "cutout_bbox_source_norm": [round(x0 / w, 4), round(y0 / h, 4), round(x1 / w, 4), round(y1 / h, 4)],
        "crop_source_px": [cx0, cy0, cx1, cy1],  # may extend past the source when padded
        "output": {"width": cw, "height": ch},
        "cutout_bbox_output_norm": [round((x0 - cx0) / cw, 4), round((y0 - cy0) / ch, 4),
                                    round((x1 - cx0) / cw, 4), round((y1 - cy0) / ch, 4)],
        "coverage": round(coverage, 4),
        "magenta_residue": round(spill, 5),
        "touches_frame": touches,
        "ok": bool((not touches) and spill < 0.002 and coverage > 0.15),
    }
    json.dump(info, open(out_stem + ".key.json", "w"), indent=1)
    return info


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("out_stem")
    ap.add_argument("--pad", type=float, default=0.06)
    a = ap.parse_args()
    info = key(a.src, a.out_stem, a.pad)
    print(json.dumps({k: info[k] for k in ("output", "coverage", "magenta_residue", "touches_frame", "ok")}))
