#!/usr/bin/env python3
"""Remove magenta chroma-key leftovers from RGBA cutouts and sprite frames.

Library assets are generated on a pure magenta (#FF00FF) ground and keyed.
Their prompts forbid pink, purple and magenta in the subject, so any magenta
cast left in the cutout is key spill:

* residue: near-pure ground colour still visible (gaps, slivers, compression
  smears) becomes fully transparent;
* spill: every visible pixel loses its magenta cast (min(R, B) - G is taken
  out of R and B), which neutralises tinted edges without touching the
  allowed palette (yellow, teal, coral, cream, greys);
* fully transparent pixels get neutral RGB so later resizing or filtering
  can never bleed the ground colour back into edges.

    scripts/despill_magenta.py <png or dir> [...]   # clean in place, keeps palette PNGs as palette
    scripts/despill_magenta.py --check <png or dir> [...]   # exit 1 if anything would change

`clean(rgba)` (float32 HxWx4, straight alpha, 0..1) is imported by
key_cutout.py and extract_sprite.py.
"""
import os
import sys

import numpy as np
from PIL import Image

# Visible pixels this close to the ground colour are residue.
RESIDUE_CAST = 0.35
RESIDUE_BALANCE = 0.35
# A cast below this (0..1) is left alone (sensor/JPEG noise on greys).
SPILL_MIN = 0.02


def clean(rgba):
    out = rgba.astype(np.float32).copy()
    r, g, b, a = out[..., 0], out[..., 1], out[..., 2], out[..., 3]
    cast = np.minimum(r, b) - g
    residue = (a > 0) & (cast > RESIDUE_CAST) & (np.abs(r - b) < RESIDUE_BALANCE)
    a[residue] = 0.0
    spill = np.where(cast > SPILL_MIN, cast, 0.0)
    r -= spill
    b -= spill
    clear = a <= 0.0
    for c in (r, g, b):
        c[clear] = 0.0
    return np.clip(out, 0.0, 1.0)


def residue_stats(rgba):
    """(residue pixels, spilled visible pixels) on a uint8/float RGBA array."""
    x = rgba.astype(np.float32)
    if x.max() > 1.0:
        x = x / 255.0
    r, g, b, a = x[..., 0], x[..., 1], x[..., 2], x[..., 3]
    vis = a > 0.03
    cast = np.minimum(r, b) - g
    residue = vis & (cast > RESIDUE_CAST) & (np.abs(r - b) < RESIDUE_BALANCE)
    spill = vis & (cast > 0.18)
    return int(residue.sum()), int(spill.sum())


def _files(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, names in os.walk(p):
                for n in sorted(names):
                    if n.lower().endswith(".png"):
                        yield os.path.join(root, n)
        elif p.lower().endswith(".png"):
            yield p


def main():
    args = sys.argv[1:]
    check = "--check" in args
    paths = [a for a in args if a != "--check"]
    if not paths:
        sys.exit(__doc__)
    changed = 0
    total = 0
    for path in _files(paths):
        im = Image.open(path)
        if im.mode not in ("RGBA", "P", "LA") and "transparency" not in im.info:
            continue
        palette = im.mode == "P"
        rgba = np.asarray(im.convert("RGBA")).astype(np.float32) / 255.0
        res, spill = residue_stats(rgba)
        total += 1
        if res == 0 and spill == 0:
            continue
        changed += 1
        if check:
            print(f"{path}: {res} residue px, {spill} spill px")
            continue
        out = Image.fromarray((clean(rgba) * 255 + 0.5).astype(np.uint8), "RGBA")
        if palette:
            out = out.quantize(256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)
        out.save(path, optimize=True)
    verb = "need cleaning" if check else "cleaned"
    print(f"{changed} of {total} image(s) {verb}")
    if check and changed:
        sys.exit(1)


if __name__ == "__main__":
    main()
