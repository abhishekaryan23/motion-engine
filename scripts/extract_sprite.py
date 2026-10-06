#!/usr/bin/env python3
"""Extract a video into a MotionEngine sprite-sequence directory (0.11),
outside the engine.

    scripts/extract_sprite.py in.mp4 out_dir [--fps 12] [--mode loop|once]
        [--key] [--max-seconds 4] [--short 512] [--crop x,y,w,h] [--palette]

Writes out_dir/frame_0001.png … and out_dir/sprite.json:
  {frame_count, fps, mode, pattern, width, height, ssim_first_last, source}

--key   chroma-keys a flat magenta ground (same thresholds as key_cutout.py),
        then crops EVERY frame to the union alpha box (+6 % pad) so the
        subject never jumps between frames.
--mode loop drops a final frame that duplicates the first (start = end
        frame generations) and reports the first/last SSIM of the kept loop
        (the seam: last frame → first frame). A seamless loop needs ≥ 0.9.
Deterministic for a given input file and ffmpeg build.
"""
import argparse, glob, json, os, shutil, subprocess, sys, tempfile

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import despill_magenta  # noqa: E402
from PIL import Image
from scipy import ndimage

LO, HI = 0.18, 0.38
CORE = 0.08


def alpha_for(rgb):
    border = np.concatenate([rgb[0], rgb[-1], rgb[:, 0], rgb[:, -1]])
    bg = np.median(border, axis=0)
    dist = np.sqrt(((rgb - bg) ** 2).sum(-1)) / np.sqrt(3.0)
    near = dist < HI
    lab, _ = ndimage.label(near)
    edge = np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]]))
    edge = edge[edge > 0]
    bgmask = np.isin(lab, edge)
    # Enclosed pockets that contain near-exact ground colour.
    for l in np.unique(lab[near & ~bgmask]):
        if l == 0:
            continue
        reg = lab == l
        if (dist[reg] < CORE).mean() > 0.3:
            bgmask |= reg
    a = np.clip((dist - LO) / (HI - LO), 0.0, 1.0)
    a[bgmask & (dist < LO)] = 0.0
    a = np.where(bgmask, a, 1.0)
    return a, bg


def despill(rgb, a, bg):
    # Pull magenta spill out of semi-transparent edges.
    spill = np.clip(np.minimum(rgb[..., 0], rgb[..., 2]) - rgb[..., 1], 0, 1)
    k = (1.0 - a)[..., None] * 0.9
    out = rgb.copy()
    out[..., 0] -= spill * k[..., 0]
    out[..., 2] -= spill * k[..., 0]
    return np.clip(out, 0, 1)


def ssim(a, b):
    a = a.astype(np.float64); b = b.astype(np.float64)
    c1, c2 = (0.01 * 255) ** 2, (0.03 * 255) ** 2
    g = lambda x: ndimage.gaussian_filter(x, 1.5)
    ma, mb = g(a), g(b)
    va, vb, cov = g(a * a) - ma * ma, g(b * b) - mb * mb, g(a * b) - ma * mb
    m = ((2 * ma * mb + c1) * (2 * cov + c2)) / ((ma * ma + mb * mb + c1) * (va + vb + c2))
    return float(m.mean())


def grey(img):
    return np.asarray(img.convert("RGBA").convert("L"), dtype=np.float64)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src"); ap.add_argument("out_dir")
    ap.add_argument("--fps", type=float, default=12.0)
    ap.add_argument("--mode", choices=["loop", "once"], default="loop")
    ap.add_argument("--key", action="store_true")
    ap.add_argument("--max-seconds", type=float, default=4.0)
    ap.add_argument("--short", type=int, default=512)
    ap.add_argument("--crop", default=None, help="x,y,w,h in source pixels (non-keyed inserts)")
    ap.add_argument("--palette", action="store_true", help="256-colour PNGs (fast octree, deterministic)")
    args = ap.parse_args()

    tmp = tempfile.mkdtemp()
    try:
        subprocess.run(["ffmpeg", "-v", "error", "-i", args.src, "-t", str(args.max_seconds),
                        "-vf", f"fps={args.fps}", "-pix_fmt", "rgb24", os.path.join(tmp, "f_%04d.png")],
                       check=True)
        files = sorted(glob.glob(os.path.join(tmp, "f_*.png")))
        if not files:
            sys.exit("no frames extracted")
        frames = [np.asarray(Image.open(f).convert("RGB")).astype(np.float32) / 255.0 for f in files]
        if args.mode == "loop" and len(frames) > 2:
            # Start = end generations repeat the first frame at the end: drop it.
            if ssim(frames[0].mean(-1) * 255, frames[-1].mean(-1) * 255) > 0.97:
                frames = frames[:-1]
        out = []
        if args.key:
            keyed = []
            for f in frames:
                a, bg = alpha_for(f)
                keyed.append((despill(f, a, bg), a))
            union = np.zeros(keyed[0][1].shape, bool)
            for _, a in keyed:
                union |= a > 0.05
            ys, xs = np.nonzero(union)
            if len(xs) == 0:
                sys.exit("keyed frames are empty")
            h, w = union.shape
            pad = int(0.06 * max(xs.max() - xs.min(), ys.max() - ys.min()))
            x0, x1 = max(0, xs.min() - pad), min(w, xs.max() + 1 + pad)
            y0, y1 = max(0, ys.min() - pad), min(h, ys.max() + 1 + pad)
            for rgb, a in keyed:
                rgba = despill_magenta.clean(np.dstack([rgb, a])[y0:y1, x0:x1])
                out.append(Image.fromarray((rgba * 255 + 0.5).astype(np.uint8), "RGBA"))
        else:
            for f in frames:
                im = Image.fromarray((f * 255 + 0.5).astype(np.uint8), "RGB")
                if args.crop:
                    x, y, cw, ch = [int(v) for v in args.crop.split(",")]
                    im = im.crop((x, y, x + cw, y + ch))
                out.append(im)
        # Normalise the short side.
        w0, h0 = out[0].size
        s = args.short / min(w0, h0)
        if s < 1.0:
            size = (max(1, round(w0 * s)), max(1, round(h0 * s)))
            # Premultiplied resize: transparent pixels never bleed into edges.
            out = [
                im.convert("RGBa").resize(size, Image.LANCZOS).convert("RGBA")
                if im.mode == "RGBA"
                else im.resize(size, Image.LANCZOS)
                for im in out
            ]
        if args.key:
            out = [
                Image.fromarray(
                    (despill_magenta.clean(np.asarray(im).astype(np.float32) / 255.0) * 255 + 0.5).astype(np.uint8),
                    "RGBA",
                )
                for im in out
            ]
        if os.path.isdir(args.out_dir):
            shutil.rmtree(args.out_dir)
        os.makedirs(args.out_dir)
        seam = ssim(grey(out[-1]), grey(out[0])) if len(out) > 1 else 1.0
        if args.palette:
            out = [im.quantize(256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE) for im in out]
        for i, im in enumerate(out, 1):
            im.save(os.path.join(args.out_dir, f"frame_{i:04d}.png"), optimize=True)
        stored = ssim(grey(out[-1]), grey(out[0])) if len(out) > 1 else 1.0
        meta = {
            "frame_count": len(out), "fps": args.fps, "mode": args.mode,
            "pattern": "frame_%04d.png", "width": out[0].size[0], "height": out[0].size[1],
            "ssim_first_last": round(seam, 4), "ssim_first_last_stored": round(stored, 4), "keyed": bool(args.key),
            "source": os.path.basename(args.src),
        }
        with open(os.path.join(args.out_dir, "sprite.json"), "w") as fh:
            json.dump(meta, fh, indent=2)
        print(json.dumps(meta))
        if args.mode == "loop" and seam < 0.9:
            print(f"WARN: loop seam SSIM {seam:.3f} < 0.9", file=sys.stderr)
    finally:
        shutil.rmtree(tmp)


if __name__ == "__main__":
    main()
