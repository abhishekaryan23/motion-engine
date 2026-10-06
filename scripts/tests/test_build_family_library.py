#!/usr/bin/env python3
"""Synthetic-fixture test for scripts/build_family_library.py.

    python3 scripts/tests/test_build_family_library.py
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

import numpy as np
from PIL import Image

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SCRIPT = os.path.join(ROOT, "scripts", "build_family_library.py")
ENGINE = os.path.join(ROOT, "target", "release", "motion-engine")

# object canvas 1024x1024: grey square 256..768, magenta hole 400..620 x 360..560
W = H = 1024
SQ = (256, 256, 768, 768)
HOLE = (400, 360, 620, 560)


def make_fixture(tmp):
    work = os.path.join(tmp, "work")
    os.makedirs(os.path.join(work, "raw"))
    img = np.zeros((H, W, 3), np.uint8)
    img[:] = (255, 0, 255)
    x0, y0, x1, y1 = SQ
    img[y0:y1, x0:x1] = (170, 120, 80)
    hx0, hy0, hx1, hy1 = HOLE
    img[hy0:hy1, hx0:hx1] = (255, 0, 255)
    Image.fromarray(img).save(os.path.join(work, "raw", "box.png"))
    Image.new("RGB", (576, 1024), (40, 90, 160)).save(os.path.join(work, "raw", "flat.png"))
    with open(os.path.join(work, "jobs.tsv"), "w") as f:
        f.write("testfam\tbox\thist-1\thttps://example.invalid/box.png\n")
        f.write("testfam\tflat\thist-2\n")
    spec = {
        "family": "testfam", "project": "Test Project", "project_id": "x", "emotions": ["calm"],
        "medium": "synthetic", "assets": [
            {"id": "box", "role": "object", "subject": "grey box", "tags": ["box", "grey"],
             "model": "flare", "aspect": "1:1", "prompt": "a grey box", "screen_box": True},
            {"id": "flat", "role": "texture", "subject": "flat blue", "tags": ["blue"],
             "model": "sunburst", "aspect": "9:16", "prompt": "flat blue"},
            {"id": "ghost", "role": "object", "subject": "never generated", "tags": [],
             "model": "flare", "aspect": "1:1", "prompt": "ghost"},
        ]}
    spec_path = os.path.join(tmp, "testfam.json")
    with open(spec_path, "w") as f:
        json.dump(spec, f)
    return spec_path, work


def run(spec, work, root):
    env = dict(os.environ, MOTION_ENGINE=ENGINE)
    r = subprocess.run([sys.executable, SCRIPT, spec, work, root], capture_output=True, text=True,
                       env=env, cwd=ROOT)
    assert r.returncode == 0, r.stdout + r.stderr
    return r.stdout


def main():
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "motion-cli"], check=True, cwd=ROOT)
    tmp = tempfile.mkdtemp()
    try:
        spec, work = make_fixture(tmp)
        root1, root2 = os.path.join(tmp, "out1"), os.path.join(tmp, "out2")
        print(run(spec, work, root1))
        lib = os.path.join(root1, "library", "testfam")
        assert os.path.exists(os.path.join(lib, "manifest.json"))
        cat = json.load(open(os.path.join(lib, "catalog.json")))
        assert cat["version"] == "0.1" and cat["family"] == "testfam" and cat["emotions"] == ["calm"]
        by = {a["id"]: a for a in cat["assets"]}
        assert by["box"]["qa"] in ("PASS", "WARN"), by["box"]
        assert by["ghost"]["qa"] == "MISSING"
        manifest = json.load(open(os.path.join(lib, "manifest.json")))
        ids = [e["id"] for e in manifest["assets"]]
        assert "library.ghost" not in ids and "library.box" in ids, ids

        # screen_box roughly matches the hole, normalized to the keyed PNG
        sb = by["box"]["screen_box"]
        assert sb is not None, "screen_box not found"
        w, h = by["box"]["width"], by["box"]["height"]
        key = json.load(open(os.path.join(work, "keyed", "box.key.json")))
        cx0, cy0 = key["crop_source_px"][:2]
        exp = ((HOLE[0] - cx0) / w, (HOLE[1] - cy0) / h,
               (HOLE[2] - HOLE[0]) / w, (HOLE[3] - HOLE[1]) / h)
        got = (sb["x"], sb["y"], sb["width"], sb["height"])
        for g, e in zip(got, exp):
            assert abs(g - e) <= 0.05, (got, exp)

        # texture copied opaque
        tex = Image.open(os.path.join(lib, "flat.png"))
        assert tex.mode == "RGB" and tex.size == (576, 1024), (tex.mode, tex.size)
        assert by["flat"]["alpha"] is False and by["flat"]["role"] == "texture"

        # no redistribution wording
        for name in ("catalog.json", "LICENSE.md", "prompts.json"):
            assert "redistribut" not in open(os.path.join(lib, name)).read().lower()

        # determinism
        run(spec, work, root2)
        a = open(os.path.join(lib, "catalog.json"), "rb").read()
        b = open(os.path.join(root2, "library", "testfam", "catalog.json"), "rb").read()
        assert a == b, "catalog.json differs between runs"
        # greyscale family flag: RGB -> luma grey, alpha untouched, default leaves colour
        plain = np.asarray(Image.open(os.path.join(lib, "box.png")).convert("RGBA"))
        assert (plain[..., 0] != plain[..., 2]).any(), "fixture object should be coloured"
        grey_spec = json.load(open(spec))
        grey_spec["greyscale"] = True
        grey_spec_path = os.path.join(tmp, "testfam_grey.json")
        with open(grey_spec_path, "w") as f:
            json.dump(grey_spec, f)
        root3 = os.path.join(tmp, "out3")
        run(grey_spec_path, work, root3)
        glib = os.path.join(root3, "library", "testfam")
        grey = np.asarray(Image.open(os.path.join(glib, "box.png")).convert("RGBA"))
        assert (grey[..., 0] == grey[..., 1]).all() and (grey[..., 1] == grey[..., 2]).all()
        assert (grey[..., 3] == plain[..., 3]).all(), "alpha must be untouched"
        assert Image.open(os.path.join(glib, "flat.png")).getpixel((0, 0)) == (40, 90, 160)
        print("OK")
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
