#!/usr/bin/env python3
"""Remove violet / pink / magenta tints from RGBA cutouts and sprite frames.

`despill_magenta.py` strips the chroma-key cast (min(R, B) - G). Glass, bubbles
and bones can keep a softer lilac or pink cast that is not key spill but still
reads as pink on a cool ocean palette. For families whose palette has no violet
or pink at all (spec flag `neutralize_violet`), this rotates every pixel whose
hue lies in the violet..pink band to a cool cyan-blue and caps its saturation,
keeping value and alpha, so highlights stay white-blue. It then removes any
remaining pink/purple cast outright (min(R, B) - G, with no noise floor), so a
near-grey lilac like (134, 128, 144) becomes a true neutral. Blues and cyans
have no such cast and are untouched.

    scripts/neutralize_tint.py <png or dir> [...]          # fix in place
    scripts/neutralize_tint.py --check <png or dir> [...]  # exit 1 if any tint is left

Palette (mode P) PNGs are fixed by editing their palette entries directly, so
the 256-colour structure is kept and no re-quantisation can bring a cast back.
`neutralize(rgb)` (float32 ...x3, 0..1) is importable. Deterministic.
"""
import os
import sys

import numpy as np
from PIL import Image

# Hue band (degrees) that counts as violet / pink / magenta: full effect from
# FULL_LO up to PINK_HI, with a smooth ramp on the blue side.
RAMP_LO = 252.0
FULL_LO = 270.0
# Rose-red hues above PINK_HI are left to the cast removal below (it pulls them
# to plain red/neutral); rotating them would swing a red across the whole band.
PINK_HI = 340.0
PALE_LO = 228.0
PALE_FULL = 244.0
PALE_SAT_MAX = 0.55  # periwinkle rotation fades out between 0.35 and 0.55
TARGET_HUE = 205.0  # cyan-blue
SAT_CAP = 0.35
# Pixels below this saturation / value are neutral enough to leave alone.
SAT_MIN = 0.04
# --check: clean = no visible pixel in the tint band and no cast above ~3/255;
# a few uint8 rounding stragglers are tolerated.
CHECK_CAST = 3.0 / 255.0
CHECK_VAL = 0.15
CHECK_TOLERANCE = 10


def _hsv(rgb):
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    mx = rgb.max(-1)
    mn = rgb.min(-1)
    d = mx - mn
    safe = np.where(d > 0, d, 1.0)
    h = np.where(
        mx == r,
        ((g - b) / safe) % 6.0,
        np.where(mx == g, (b - r) / safe + 2.0, (r - g) / safe + 4.0),
    )
    h = np.where(d > 0, h * 60.0, 0.0)
    s = np.where(mx > 0, d / np.where(mx > 0, mx, 1.0), 0.0)
    return h, s, mx


def _hsv_to_rgb(h, s, v):
    c = v * s
    hp = (h % 360.0) / 60.0
    x = c * (1.0 - np.abs(hp % 2.0 - 1.0))
    z = np.zeros_like(c)
    sector = np.floor(hp).astype(np.int32) % 6
    r = np.choose(sector, [c, x, z, z, x, c])
    g = np.choose(sector, [x, c, c, x, z, z])
    b = np.choose(sector, [z, z, x, c, c, x])
    m = v - c
    return np.stack([r + m, g + m, b + m], axis=-1)


def _weight(h, s):
    up = np.clip((h - RAMP_LO) / (FULL_LO - RAMP_LO), 0.0, 1.0)
    pink = np.where(h <= PINK_HI, up, 0.0)
    # Pale blue-violet (periwinkle glass highlights): only low-saturation
    # pixels, so saturated navy and deep blue water keep their hue.
    pup = np.clip((h - PALE_LO) / (PALE_FULL - PALE_LO), 0.0, 1.0)
    gate = np.clip((PALE_SAT_MAX - s) / 0.2, 0.0, 1.0)
    pale = np.where(h <= PINK_HI, pup, 0.0) * gate
    return np.maximum(pink, pale)


def neutralize(rgb):
    """Rotate violet..pink hues to cyan-blue; float32 ...x3 in 0..1."""
    h, s, v = _hsv(rgb)
    w = _weight(h, s) * (s > SAT_MIN)
    new_h = h * (1.0 - w) + TARGET_HUE * w
    new_s = np.where(w > 0, s * (1.0 - w) + np.minimum(s, SAT_CAP) * w, s)
    out = np.where((w > 0)[..., None], _hsv_to_rgb(new_h, new_s, v), rgb)
    cast = np.maximum(np.minimum(out[..., 0], out[..., 2]) - out[..., 1], 0.0)
    out[..., 0] -= cast
    out[..., 2] -= cast
    return out.clip(0.0, 1.0).astype(np.float32)


def tinted(rgba):
    """Count of visible, non-black pixels still in the tint band or carrying a cast."""
    rgb = rgba[..., :3]
    h, sat, v = _hsv(rgb)
    in_band = (_weight(h, sat) >= 0.999) & (sat > SAT_MIN)
    cast = np.minimum(rgb[..., 0], rgb[..., 2]) - rgb[..., 1]
    vis = (rgba[..., 3] > 0.1) & (v > CHECK_VAL)
    return int((vis & (in_band | (cast > CHECK_CAST))).sum())


def _files(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, names in os.walk(p):
                for n in sorted(names):
                    if n.lower().endswith(".png"):
                        yield os.path.join(root, n)
        elif p.lower().endswith(".png"):
            yield p


def fix_file(path, check=False):
    """Returns the tinted-pixel count before fixing (0 = nothing to do)."""
    im = Image.open(path)
    if im.mode == "P":
        pal = np.asarray(im.getpalette()[: 3 * 256], dtype=np.float32).reshape(-1, 3) / 255.0
        before = tinted(np.dstack([pal[np.asarray(im)], np.ones(im.size[::-1], np.float32)]))
        if check or before == 0:
            return before
        fixed = (neutralize(pal) * 255 + 0.5).astype(np.uint8)
        im.putpalette(fixed.reshape(-1).tolist())
        im.save(path, optimize=True, transparency=im.info.get("transparency"))
        return before
    rgba = np.asarray(im.convert("RGBA")).astype(np.float32) / 255.0
    before = tinted(rgba)
    if check or before == 0:
        return before
    out = np.dstack([neutralize(rgba[..., :3]), rgba[..., 3]])
    Image.fromarray((out * 255 + 0.5).astype(np.uint8), "RGBA").save(path, optimize=True)
    return before


def main():
    args = sys.argv[1:]
    check = "--check" in args
    paths = [a for a in args if a != "--check"]
    if not paths:
        sys.exit(__doc__)
    total = changed = 0
    for path in _files(paths):
        n = fix_file(path, check)
        total += 1
        if n > (CHECK_TOLERANCE if check else 0):
            changed += 1
            if check:
                print(f"{path}: {n} tinted px")
    print(f"{changed} of {total} image(s) {'have a tint' if check else 'fixed'}")
    if check and changed:
        sys.exit(1)


if __name__ == "__main__":
    main()
