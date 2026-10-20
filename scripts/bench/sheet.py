#!/usr/bin/env python3
"""Contact sheet: one row per compiled scene, one cell per beat at the READ/EVOLVE midpoint.

Usage: sheet.py DIR ENGINE OUT.png NAME...
  DIR     folder holding NAME.motion.json files
  ENGINE  path to the motion-engine binary (release build)

Each cell is rendered with `render --frame N`; the single-frame PNGs are deleted as soon
as the sheet is built (rule: no rendered frames are kept). For full videos, build
sheets from the MP4 with ffmpeg instead.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

CELL_W, CELL_H, LABEL_W = 216, 384, 170


def read_frames(path):
    p = json.load(open(path))
    fps = p['canvas']['fps']
    out = []
    for s in p['scenes']:
        if not s['id'].startswith('beat_'):
            continue
        lc = s.get('lifecycle') or {}
        t = s['start_seconds'] + (lc.get('read', 0) + lc.get('evolve', s['duration_seconds'] * 0.6)) / 2
        out.append(int(t * fps))
    return out


def main():
    d, engine, out, names = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
    rows = []
    tmp = tempfile.mkdtemp(prefix='sheet_frames_')
    try:
        for n in names:
            scene = f'{d}/{n}.motion.json'
            cells = []
            for f in read_frames(scene):
                fd = os.path.join(tmp, n)
                os.makedirs(fd, exist_ok=True)
                subprocess.run([engine, 'render', scene, '--frame', str(f), '--out-dir', fd],
                               check=True, capture_output=True)
                png = next(x for x in os.listdir(fd) if x.endswith(f'{f:06d}.png'))
                im = Image.open(os.path.join(fd, png)).convert('RGB')
                im.thumbnail((CELL_W, CELL_H))
                cells.append(im)
                os.remove(os.path.join(fd, png))
            rows.append((n, cells))
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    sheet = Image.new('RGB', (LABEL_W + CELL_W * max(len(c) for _, c in rows), CELL_H * len(rows)), 'white')
    draw = ImageDraw.Draw(sheet)
    for r, (n, cells) in enumerate(rows):
        draw.text((6, r * CELL_H + CELL_H // 2), n, fill='black')
        for c, im in enumerate(cells):
            sheet.paste(im, (LABEL_W + c * CELL_W, r * CELL_H))
    sheet.save(out)
    print('wrote', out, sheet.size)


if __name__ == '__main__':
    main()
