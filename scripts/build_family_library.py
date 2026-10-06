#!/usr/bin/env python3
"""Package one spec-driven OpenArt cutout family into an engine asset library.

    scripts/build_family_library.py <spec.json> <work_dir> <library_root>

<spec.json> is an assets/cutout_families/<family>.json file. <work_dir> holds
raw/<id>.png (downloaded generations) and jobs.tsv (`family\\tid\\thistoryId\\turl`,
url optional). Objects/figures are keyed with scripts/key_cutout.py into
<work_dir>/keyed/<id>.png (+ .key.json); textures are copied as opaque RGB PNG.
Output goes to <library_root>/library/<family>/: <id>.png, <id>.json
(GeneratedSidecar), prompts.json (AssetPromptSet), manifest.json (from
`motion-engine ingest-assets`), catalog.json and LICENSE.md.

Assets with `screen_box: true` get the bounding box (normalized to the keyed
cutout PNG) of the largest enclosed transparent hole (not connected to the
border, area >= 2% of the cutout) recorded as catalog `screen_box`.

Optional family-level `"greyscale": true` converts each keyed cutout's RGB to
Rec.709 luma grey (alpha untouched) before packaging.

Optional family-level `"neutralize_violet": true` runs scripts/neutralize_tint.py
on each keyed cutout (violet/pink/magenta tints and any leftover red+blue cast
become cool neutral), for families whose palette has no violet or pink.

Missing raw files are skipped and reported (qa MISSING in the catalog, absent
from the manifest). Deterministic for the same inputs.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)

ENGINE = os.environ.get("MOTION_ENGINE", "target/release/motion-engine")
MODELS = {"flare": "gpt-image-2-5-flare (low, 1k)", "sunburst": "gpt-image-2-5-sunburst (low, 1k)"}
ROLE_MAP = {"object": "hero_object", "figure": "hero_subject", "texture": "environment"}
FRAMING = {"object": "object", "figure": "full_figure", "texture": "scene"}
AVOID = ["embedded text", "letters or numbers", "logos", "watermark", "brand marks"]
MIN_HOLE_FRAC = 0.02
KEY_NOTE = "scripts/key_cutout.py (magenta, border+pocket key, tight crop)"


def read_jobs(work):
    """id -> (history_id, url) for rows of jobs.tsv."""
    jobs = {}
    path = os.path.join(work, "jobs.tsv")
    if not os.path.exists(path):
        return jobs
    for line in open(path):
        line = line.rstrip("\n")
        if not line.strip():
            continue
        cols = line.split("\t")
        cols += [""] * (4 - len(cols))
        jobs[(cols[0], cols[1])] = (cols[2], cols[3])
    return jobs


def to_greyscale(png_path):
    """Replace RGB of a keyed RGBA cutout with Rec.709 luma grey; alpha untouched."""
    import numpy as np
    from PIL import Image

    px = np.asarray(Image.open(png_path).convert("RGBA")).astype(np.float64)
    luma = px[..., 0] * 0.2126 + px[..., 1] * 0.7152 + px[..., 2] * 0.0722
    g = np.clip(np.floor(luma + 0.5), 0, 255).astype(np.uint8)
    out = np.dstack([g, g, g, px[..., 3].astype(np.uint8)])
    Image.fromarray(out, "RGBA").save(png_path, optimize=True)


def find_screen_box(png_path, cutout_bbox):
    """Largest enclosed transparent hole (not touching the border) as a
    normalized {x,y,width,height}, or None."""
    import numpy as np
    from PIL import Image
    from scipy import ndimage

    alpha = np.asarray(Image.open(png_path).convert("RGBA"))[..., 3]
    h, w = alpha.shape
    lab, n = ndimage.label(alpha < 128)
    if n == 0:
        return None
    border = np.unique(np.concatenate([lab[0], lab[-1], lab[:, 0], lab[:, -1]]))
    cut_area = max(1.0, (cutout_bbox[2] - cutout_bbox[0]) * w * (cutout_bbox[3] - cutout_bbox[1]) * h)
    best = None
    for i in range(1, n + 1):
        if i in border:
            continue
        ys, xs = np.nonzero(lab == i)
        area = len(xs)
        if area < MIN_HOLE_FRAC * cut_area:
            continue
        if best is None or area > best[0]:
            best = (area, int(xs.min()), int(ys.min()), int(xs.max()) + 1, int(ys.max()) + 1)
    if best is None:
        return None
    _, x0, y0, x1, y1 = best
    return {"x": round(x0 / w, 4), "y": round(y0 / h, 4),
            "width": round((x1 - x0) / w, 4), "height": round((y1 - y0) / h, 4)}


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    spec_path, work, root = sys.argv[1], sys.argv[2], sys.argv[3]
    spec = json.load(open(spec_path))
    family, project = spec["family"], spec["project"]
    greyscale = bool(spec.get("greyscale", False))
    no_violet = bool(spec.get("neutralize_violet", False))
    lib = os.path.join(root, "library", family)
    os.makedirs(lib, exist_ok=True)
    os.makedirs(os.path.join(work, "keyed"), exist_ok=True)
    jobs = read_jobs(work)
    style = {
        "medium": spec.get("medium", ""),
        "realism": "as generated",
        "lighting": "as generated",
        "contrast": "as generated",
        "palette_tendency": "as generated",
        "edge_treatment": "crisp clean cut edge",
        "shadow_treatment": "no baked shadow (engine adds its own)",
        "camera_feel": "subject centered in its own frame",
        "background_behavior": "isolated subject on transparency (keyed from magenta)",
    }

    catalog, specs, notes = [], [], {}  # notes: id -> (status, [reasons])
    for a in spec["assets"]:
        id_, role = a["id"], a["role"]
        is_tex = role == "texture"
        entry = {"id": id_, "file": id_ + ".png", "role": role, "subject": a["subject"],
                 "tags": sorted(set(a.get("tags", []))), "alpha": not is_tex}
        raw = os.path.join(work, "raw", id_ + ".png")
        if not os.path.exists(raw):
            entry["qa"] = "MISSING"
            notes[id_] = ("MISSING", ["raw file not found"])
            if a.get("screen_box"):
                entry["screen_box"] = None
            catalog.append(entry)
            continue
        dst = os.path.join(lib, id_ + ".png")
        key = None
        if is_tex:
            from PIL import Image
            Image.open(raw).convert("RGB").save(dst, optimize=True)
        else:
            import key_cutout
            stem = os.path.join(work, "keyed", id_)
            try:
                key = key_cutout.key(raw, stem)
            except SystemExit as e:
                entry["qa"] = "FAIL"
                notes[id_] = ("FAIL", [f"keying failed: {e}"])
                if a.get("screen_box"):
                    entry["screen_box"] = None
                catalog.append(entry)
                continue
            if greyscale:
                to_greyscale(stem + ".png")
            if no_violet:
                import neutralize_tint
                neutralize_tint.fix_file(stem + ".png")
            shutil.copyfile(stem + ".png", dst)

        hid, _url = jobs.get((family, id_), ("", ""))
        gen = {"vendor": "openart", "project": project, "model": MODELS[a["model"]], "history_id": hid,
               "keyed": None if is_tex else KEY_NOTE}
        side = {"generator": gen}
        if key:
            b = key["cutout_bbox_output_norm"]
            side["subject_anchor"] = {"x": round((b[0] + b[2]) / 2, 4), "y": round((b[1] + b[3]) / 2, 4)}
        with open(os.path.join(lib, id_ + ".json"), "w") as f:
            json.dump(side, f, indent=1)

        if key:
            entry["cutout_bbox"] = key["cutout_bbox_output_norm"]
            entry["open_edge"] = bool(key["touches_frame"])
            if not key["ok"]:
                notes[id_] = ("WARN", ["key_cutout reports not ok (coverage/residue/touches frame)"])
        else:
            entry["cutout_bbox"] = None
            entry["open_edge"] = False
        if a.get("screen_box"):
            sb = find_screen_box(dst, key["cutout_bbox_output_norm"]) if key else None
            entry["screen_box"] = sb
            if sb is None:
                print(f"warning: {family}/{id_}: no enclosed screen hole found (screen_box: null)")
                prev = notes.get(id_, ("PASS", []))
                notes[id_] = ("WARN", prev[1] + ["no screen_box hole found"])
        entry["source"] = gen
        catalog.append(entry)

        fr = FRAMING[role]
        art = json.dumps([id_, a["subject"], fr, is_tex, style], sort_keys=True)
        fp = "fp1-" + hashlib.sha256(art.encode()).hexdigest()[:16]
        specs.append({
            "id": f"library.{id_}", "fingerprint": fp, "continuity_key": id_, "serves": [f"library.{id_}"],
            "role": ROLE_MAP[role], "priority": "optional", "subject": a["subject"],
            "context": f"library asset: {a['subject']}",
            "presentation": "background_plate" if is_tex else "isolated_cutout",
            "background": "opaque" if is_tex else "transparent", "style": style,
            "composition": {"framing": fr, "negative_space": "none", "subject_whole": True,
                            "head_inside_frame": role == "figure"},
            "avoid": AVOID,
            "output": {"file_stem": id_, "alpha_required": not is_tex, "min_short_side": 512,
                       "aspect": a.get("aspect", "1:1")},
            "prompt": a["prompt"],
        })

    with open(os.path.join(lib, "prompts.json"), "w") as f:
        json.dump({"version": "0.1", "specs": specs}, f, indent=1)

    status = {}
    if specs:
        r = subprocess.run([ENGINE, "ingest-assets", os.path.join(lib, "prompts.json"), lib,
                            "-o", os.path.join(lib, "manifest.json"), "--json"],
                           capture_output=True, text=True)
        try:
            report = json.loads(r.stdout)
        except json.JSONDecodeError:
            sys.exit(f"ingest-assets failed (exit {r.returncode}): {r.stderr.strip() or r.stdout.strip()}")
        for a in report["assets"]:
            worst, issues = "PASS", []
            for c in a.get("checks", []):
                lvl = str(c.get("level", c.get("status", ""))).upper()
                if lvl in ("PASS", "OK"):
                    continue
                issues.append(c.get("message", c.get("detail", lvl)))
                if lvl == "FAIL":
                    worst = "FAIL"
                elif lvl in ("WARN", "WARNING") and worst == "PASS":
                    worst = "WARN"
            status[a.get("id", "").replace("library.", "")] = (worst, issues)
        manifest = json.load(open(os.path.join(lib, "manifest.json")))
    else:
        manifest = {"assets": []}
    by_id = {e["id"].replace("library.", ""): e for e in manifest["assets"]}

    for c in catalog:
        if c.get("qa") in ("MISSING", "FAIL"):
            continue
        worst, issues = status.get(c["id"], ("MISSING", ["not in ingest report"]))
        extra = notes.get(c["id"])
        if extra and extra[0] == "WARN" and worst == "PASS":
            worst = "WARN"
        if extra:
            issues = extra[1] + issues
        c["qa"] = worst
        notes[c["id"]] = (worst, issues)
        e = by_id.get(c["id"])
        if e and e.get("analysis"):
            an = e["analysis"]
            c["analysis"] = {"subject_bounds": an.get("subject_bounds"), "coverage": an.get("coverage"),
                             "edges": an.get("edges")}
            c["width"], c["height"] = e["width"], e["height"]

    with open(os.path.join(lib, "catalog.json"), "w") as f:
        json.dump({"version": "0.1", "family": family, "emotions": spec.get("emotions", []),
                   "medium": spec.get("medium", ""), "assets": catalog}, f, indent=1)
    with open(os.path.join(lib, "LICENSE.md"), "w") as f:
        f.write(f"# {family} asset family\n\n"
                "Generated with OpenArt (GPT Image 2.5 Flare/Sunburst, low quality, 1k) in the project "
                f"'{project}' for MotionEngine 0.9. Cutouts keyed from a magenta ground by "
                "scripts/key_cutout.py.\n")

    counts = {k: 0 for k in ("PASS", "WARN", "FAIL", "MISSING")}
    for c in catalog:
        counts[c["qa"]] = counts.get(c["qa"], 0) + 1
    print(f"{family}: " + " ".join(f"{k}={v}" for k, v in counts.items()) + f" (of {len(catalog)})")
    for c in catalog:
        if c["qa"] != "PASS":
            print(f"  {c['id']} {c['qa']}: {'; '.join(str(x) for x in notes.get(c['id'], ('', []))[1])}")


if __name__ == "__main__":
    main()
