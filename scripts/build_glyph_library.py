#!/usr/bin/env python3
"""Turn a picture font (dingbats) into an engine asset-library family.

    scripts/build_glyph_library.py <spec.json> <font_dir> <library_root>

<spec.json> (assets/glyph_families/<family>.json):
  {"family", "font" (file name inside <font_dir>), "licence", "credit",
   "ink": "#1d1b19",
   "glyphs": [{"char", "id", "subject", "tags": [...]}]}

Each glyph is rendered as ink on transparency at 1024 px, cropped to its real
ink bounds (+6 % pad, transparent pad to a 512 px short side) so placement uses
the drawn shape, then packaged like scripts/build_cutout_library.py: sidecar,
prompts.json, `motion-engine ingest-assets` (analysis + QA -> manifest.json)
and catalog.json (role object, exact-word tags) under
<library_root>/library/<family>/. Deterministic.
"""
import hashlib, json, os, subprocess, sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont

SPEC, FONT_DIR, ROOT = sys.argv[1], sys.argv[2], sys.argv[3]
ENGINE = os.environ.get("MOTION_ENGINE", "target/release/motion-engine")
spec = json.load(open(SPEC))
fam = spec["family"]
out = os.path.join(ROOT, "library", fam)
os.makedirs(out, exist_ok=True)
font = ImageFont.truetype(os.path.join(FONT_DIR, spec["font"]), 900)
ink = tuple(int(spec.get("ink", "#1d1b19")[i:i + 2], 16) for i in (1, 3, 5))
STYLE = {
    "medium": f"hand-drawn glyph from the {spec['font']} picture font",
    "realism": "illustration", "lighting": "none (flat ink)", "contrast": "high contrast",
    "palette_tendency": "single ink colour", "edge_treatment": "drawn line edge",
    "shadow_treatment": "none", "camera_feel": "flat", "background_behavior": "isolated ink on transparency",
}

specs, catalog, seen = [], [], set()
for g in spec["glyphs"]:
    canvas = Image.new("L", (1400, 1400), 0)
    ImageDraw.Draw(canvas).text((250, 120), g["char"], fill=255, font=font)
    a = np.asarray(canvas)
    ys, xs = np.nonzero(a > 8)
    if len(xs) == 0:
        print("skip empty", g["id"]); continue
    digest = hashlib.sha256(a.tobytes()).hexdigest()
    if digest in seen:
        print("skip duplicate", g["id"]); continue
    seen.add(digest)
    x0, x1, y0, y1 = xs.min(), xs.max() + 1, ys.min(), ys.max() + 1
    crop = a[y0:y1, x0:x1].astype(np.float32) / 255.0
    h, w = crop.shape
    s = 1024.0 / max(h, w)  # longest side 1024
    nw, nh = max(1, round(w * s)), max(1, round(h * s))
    alpha = np.asarray(Image.fromarray((crop * 255).astype(np.uint8)).resize((nw, nh), Image.LANCZOS)).astype(np.float32) / 255.0
    pad = round(0.06 * max(nw, nh))
    W, H = nw + 2 * pad, nh + 2 * pad
    W, H = max(W, 512), max(H, 512)
    rgba = np.zeros((H, W, 4), np.uint8)
    ox, oy = (W - nw) // 2, (H - nh) // 2
    rgba[oy:oy + nh, ox:ox + nw, :3] = ink
    rgba[oy:oy + nh, ox:ox + nw, 3] = (alpha * 255 + 0.5).astype(np.uint8)
    Image.fromarray(rgba, "RGBA").save(os.path.join(out, g["id"] + ".png"), optimize=True)
    bbox = [round(ox / W, 4), round(oy / H, 4), round((ox + nw) / W, 4), round((oy + nh) / H, 4)]
    gen = {"vendor": "font", "font": spec["font"], "licence": spec["licence"], "credit": spec.get("credit", ""),
           "codepoint": f"U+{ord(g['char']):04X}"}
    json.dump({"generator": gen, "subject_anchor": {"x": round((bbox[0] + bbox[2]) / 2, 4), "y": round((bbox[1] + bbox[3]) / 2, 4)}},
              open(os.path.join(out, g["id"] + ".json"), "w"), indent=1)
    fp = "fp1-" + hashlib.sha256(json.dumps([fam, g["id"], g["char"]]).encode()).hexdigest()[:16]
    specs.append({
        "id": f"library.{g['id']}", "fingerprint": fp, "continuity_key": g["id"], "serves": [f"library.{g['id']}"],
        "role": "hero_object", "priority": "optional", "subject": g["subject"], "context": f"library glyph: {g['subject']}",
        "presentation": "isolated_cutout", "background": "transparent", "style": STYLE,
        "composition": {"framing": "object", "negative_space": "none", "subject_whole": True, "head_inside_frame": False},
        "avoid": ["embedded text"], "output": {"file_stem": g["id"], "alpha_required": True, "min_short_side": 512, "aspect": "1:1"},
        "prompt": f"{g['subject']} ({spec['font']} {gen['codepoint']})",
    })
    catalog.append({"id": g["id"], "file": g["id"] + ".png", "role": "object", "subject": g["subject"],
                    "tags": sorted(set(t.lower() for t in g["tags"])), "alpha": True, "cutout_bbox": bbox,
                    "open_edge": False, "source": gen})
json.dump({"version": "0.1", "specs": specs}, open(os.path.join(out, "prompts.json"), "w"), indent=1)
r = subprocess.run([ENGINE, "ingest-assets", os.path.join(out, "prompts.json"), out, "-o", os.path.join(out, "manifest.json"), "--json"],
                   capture_output=True, text=True)
report = json.loads(r.stdout)
level = {}
for a in report["assets"]:
    lv = [c["level"] for c in a["checks"]]
    level[a["id"].replace("library.", "")] = "FAIL" if "fail" in lv else ("WARN" if "warning" in lv else "PASS")
manifest = json.load(open(os.path.join(out, "manifest.json")))
by_id = {e["id"].replace("library.", ""): e for e in manifest["assets"]}
for c in catalog:
    c["qa"] = level.get(c["id"], "MISSING")
    e = by_id.get(c["id"])
    if e and e.get("analysis"):
        c["analysis"] = {k: e["analysis"].get(k) for k in ("subject_bounds", "coverage", "edges")}
        c["width"], c["height"] = e["width"], e["height"]
json.dump({"version": "0.1", "family": fam, "assets": catalog}, open(os.path.join(out, "catalog.json"), "w"), indent=1)
with open(os.path.join(out, "LICENSE.md"), "w") as f:
    f.write(f"# {fam}\n\nRendered from the picture font `{spec['font']}`.\nLicence: {spec['licence']}\n"
            f"Credit: {spec.get('credit', '-')}\n")
counts = {k: sum(1 for v in level.values() if v == k) for k in ("PASS", "WARN", "FAIL")}
print(f"{fam}: {len(catalog)} glyph assets, QA {counts}")
