#!/usr/bin/env python3
"""Build SFX audition reels: output/sfx_audition/<family>.mp4 — each sound normalised
to its family peak target, trimmed to its audible end, 0.5 s gaps, burned-in id card.
Usage: scripts/sfx_audition.py <library_dir> [out_dir]"""
import json, os, subprocess, sys, tempfile
from PIL import Image, ImageDraw, ImageFont

TARGET = {"whoosh_soft": -14, "whoosh_hard": -14, "swipe": -15, "hit_hard": -8, "hit_soft": -11,
          "subdrop": -12, "riser": -14, "click": -17, "tick": -18, "pop": -16, "paper": -18,
          "data": -20, "notification": -18, "glitch": -16}
GAP, TAIL, MAXLEN = 0.5, 0.15, 4.0
lib_dir = sys.argv[1]
out_dir = sys.argv[2] if len(sys.argv) > 2 else "output/sfx_audition"
os.makedirs(out_dir, exist_ok=True)
lib = json.load(open(os.path.join(lib_dir, "sfx-library.json")))
font = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 44)
small = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 26)
fams = sorted({s["family"] for s in lib["sounds"]})
for fam in fams:
    sounds = sorted([s for s in lib["sounds"] if s["family"] == fam], key=lambda s: s["id"])
    with tempfile.TemporaryDirectory() as tmp:
        alist, vlist = [], []
        for i, s in enumerate(sounds):
            seg = min(s["audible_end"] + TAIL, s["duration"], MAXLEN) + GAP
            gain = max(-40, min(18, TARGET[fam] - s["peak_db"]))
            wav = os.path.join(tmp, f"a{i}.wav")
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", os.path.join(lib_dir, s["path"]),
                            "-af", f"aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,"
                                   f"atrim=0:{seg - GAP:.3f},volume={gain:.1f}dB,apad=whole_dur={seg:.3f}",
                            "-ar", "48000", "-ac", "2", wav], check=True)
            png = os.path.join(tmp, f"v{i}.png")
            im = Image.new("RGB", (960, 540), (24, 24, 28)); d = ImageDraw.Draw(im)
            d.text((60, 190), s["id"], fill="white", font=font)
            d.text((60, 260), f"peak {s['peak']:.3f}s  onset {s['onset']:.3f}s  end {s['audible_end']:.3f}s",
                   fill=(180, 180, 180), font=small)
            d.text((60, 300), f"peak {s['peak_db']:.1f} dBFS → gain {gain:+.1f} dB (target {TARGET[fam]} dBFS)",
                   fill=(180, 180, 180), font=small)
            d.text((60, 340), s["path"], fill=(120, 120, 120), font=small)
            im.save(png)
            alist.append(wav); vlist.append((png, seg))
        with open(os.path.join(tmp, "a.txt"), "w") as f:
            f.writelines(f"file '{a}'\n" for a in alist)
        with open(os.path.join(tmp, "v.txt"), "w") as f:
            for png, seg in vlist:
                f.write(f"file '{png}'\nduration {seg:.3f}\n")
            f.write(f"file '{vlist[-1][0]}'\n")
        out = os.path.join(out_dir, f"{fam}.mp4")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", os.path.join(tmp, "v.txt"),
                        "-f", "concat", "-safe", "0", "-i", os.path.join(tmp, "a.txt"),
                        "-c:v", "libx264", "-pix_fmt", "yuv420p", "-r", "30", "-c:a", "aac", "-b:a", "192k",
                        "-shortest", out], check=True)
        print(f"{out}: {len(sounds)} sound(s)")
