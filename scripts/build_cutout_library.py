#!/usr/bin/env python3
"""Package keyed OpenArt cutouts + textures into an engine asset library family.

    scripts/build_cutout_library.py <work_dir> <library_dir>

<work_dir> holds urls.tsv (id, kind, model, history id, url), raw/<id>.png and
keyed/<id>.png + keyed/<id>.key.json (scripts/key_cutout.py). Writes into
<library_dir>: <id>.png, <id>.json (GeneratedSidecar), prompts.json
(AssetPromptSet authored for the library), then runs `motion-engine
ingest-assets` (decode, analysis, QA -> manifest.json) and writes catalog.json
(id, role, subject, tags, file, alpha, cutout box, analysis summary) and
LICENSE.md. Deterministic for the same inputs.
"""
import hashlib, json, os, shutil, subprocess, sys

WORK, LIB = sys.argv[1], sys.argv[2]
ENGINE = os.environ.get("MOTION_ENGINE", "target/release/motion-engine")
PROJECT = "MotionEngine Asset Library (OpenArt)"
MODELS = {"flare": "gpt-image-2-5-flare (low, 1k)", "sunburst": "gpt-image-2-5-sunburst (low, 1k)"}

# subject words + tags per asset (exact words the planner may match)
META = {
    "phone": ("smartphone", ["phone", "smartphone", "mobile", "device", "screen", "app"]),
    "plant": ("potted plant", ["plant", "growth", "green", "office"]),
    "hand_pointing": ("hand pointing right", ["hand", "pointing", "gesture", "direction"]),
    "laptop": ("laptop computer", ["laptop", "computer", "work", "device", "screen"]),
    "document": ("printed document page", ["document", "page", "report", "paper", "paperwork", "contract"]),
    "envelope": ("kraft envelope", ["envelope", "mail", "letter", "message", "post"]),
    "coin_stack": ("stack of coins", ["coins", "coin", "money", "cash", "savings", "price", "cost"]),
    "receipt": ("till receipt", ["receipt", "purchase", "bill", "payment", "invoice", "price"]),
    "clock": ("wall clock", ["clock", "time", "hour", "deadline", "late"]),
    "calendar_page": ("calendar page", ["calendar", "date", "schedule", "month", "deadline"]),
    "chart_printout": ("bar chart printout", ["chart", "graph", "growth", "data", "report", "results"]),
    "magnifier": ("magnifying glass", ["magnifier", "magnifying", "search", "inspect", "detail", "research"]),
    "key": ("brass key", ["key", "access", "unlock", "secret", "security"]),
    "lightbulb": ("light bulb", ["lightbulb", "bulb", "idea", "insight", "energy"]),
    "hand_holding_phone": ("hand holding a phone", ["hand", "phone", "smartphone", "mobile", "user", "app"]),
    "shopping_bag": ("paper shopping bag", ["bag", "shopping", "shop", "purchase", "retail", "store"]),
    "parcel": ("cardboard parcel", ["parcel", "package", "box", "delivery", "shipping", "order"]),
    "coffee_cup": ("coffee cup", ["coffee", "cup", "break", "morning", "cafe"]),
    "notebook": ("closed notebook", ["notebook", "notes", "journal", "plan"]),
    "pen": ("fountain pen", ["pen", "writing", "signature", "sign"]),
    "globe": ("desk globe", ["globe", "world", "global", "earth", "international"]),
    "building": ("building facade", ["building", "office", "company", "city", "bank", "headquarters"]),
    "door": ("closed door", ["door", "entrance", "exit", "opportunity", "access"]),
    "chair": ("office chair", ["chair", "seat", "office", "desk"]),
    "torn_paper_strip": ("torn paper strip", ["strip", "torn", "paper", "label", "banner"]),
    "figure_worker_half": ("office worker, half figure", ["worker", "employee", "person", "office"]),
    "figure_walking": ("person walking", ["person", "walking", "commuter", "pedestrian"]),
    "figure_seated_laptop": ("person working on a laptop", ["person", "laptop", "worker", "freelancer", "remote"]),
    "figure_shopper": ("shopper with a bag", ["shopper", "customer", "person", "shopping"]),
    "figure_two_talking": ("two people talking", ["people", "conversation", "team", "meeting", "talking"]),
    "tex_paper_grain": ("warm paper grain", ["paper", "grain", "ground"]),
    "tex_halftone_field": ("halftone dot field", ["halftone", "dots", "print", "ground"]),
    "tex_soft_gradient": ("soft teal-grey gradient", ["gradient", "sky", "ground"]),
    "tex_vignette_ground": ("vignette studio ground", ["vignette", "studio", "ground"]),
    "tex_ink_wash": ("ink wash", ["ink", "wash", "watercolour", "ground"]),
    "tex_kraft_paper": ("kraft paper", ["kraft", "paper", "ground"]),
    "tex_charcoal_paper": ("charcoal paper", ["charcoal", "dark", "paper", "ground"]),
}
STYLE = {
    "medium": "editorial photo cut-out, printed on paper",
    "realism": "photographic, documentary",
    "lighting": "soft natural light from the upper left",
    "contrast": "medium contrast",
    "palette_tendency": "muted warm greys, faded ochre, soft teal",
    "edge_treatment": "crisp clean cut edge",
    "shadow_treatment": "no baked shadow (engine adds its own)",
    "camera_feel": "35-50mm, subject centered in its own frame",
    "background_behavior": "isolated subject on transparency (keyed from magenta)",
}
AVOID = ["embedded text", "letters or numbers", "logos", "watermark", "brand marks"]


def framing(id_, kind):
    if kind == "texture":
        return "scene"
    if id_ == "figure_worker_half":
        return "half_figure"
    if id_.startswith("figure_"):
        return "full_figure"
    if id_ in ("document", "receipt", "calendar_page", "chart_printout", "envelope", "torn_paper_strip"):
        return "artifact"
    return "object"


def role(id_, kind):
    if kind == "texture":
        return "environment"
    return "hero_subject" if id_.startswith("figure_") else "hero_object"


rows = [l.rstrip("\n").split("\t") for l in open(os.path.join(WORK, "urls.tsv")) if l.strip()]
os.makedirs(LIB, exist_ok=True)
specs, catalog = [], []
for id_, kind, model, hid, url in rows:
    is_tex = kind == "texture"
    src = os.path.join(WORK, "raw" if is_tex else "keyed", id_ + ".png")
    shutil.copyfile(src, os.path.join(LIB, id_ + ".png"))
    key = None if is_tex else json.load(open(os.path.join(WORK, "keyed", id_ + ".key.json")))
    subject, tags = META[id_]
    fr = framing(id_, kind)
    gen = {"vendor": "openart", "project": PROJECT, "model": MODELS[model], "history_id": hid,
           "keyed": None if is_tex else "scripts/key_cutout.py (magenta, border+pocket key, tight crop)"}
    side = {"generator": gen}
    if key:
        b = key["cutout_bbox_output_norm"]
        side["subject_anchor"] = {"x": round((b[0] + b[2]) / 2, 4), "y": round((b[1] + b[3]) / 2, 4)}
    json.dump(side, open(os.path.join(LIB, id_ + ".json"), "w"), indent=1)
    art = json.dumps([id_, subject, fr, is_tex, STYLE], sort_keys=True)
    fp = "fp1-" + hashlib.sha256(art.encode()).hexdigest()[:16]
    spec = {
        "id": f"library.{id_}", "fingerprint": fp, "continuity_key": id_, "serves": [f"library.{id_}"],
        "role": role(id_, kind), "priority": "optional", "subject": subject, "context": f"library asset: {subject}",
        "presentation": "background_plate" if is_tex else "isolated_cutout",
        "background": "opaque" if is_tex else "transparent", "style": STYLE,
        "composition": {"framing": fr, "negative_space": "none", "subject_whole": fr != "half_figure",
                        "head_inside_frame": id_.startswith("figure_")},
        "avoid": AVOID,
        "output": {"file_stem": id_, "alpha_required": not is_tex, "min_short_side": 512,
                   "aspect": {"scene": "9:16", "half_figure": "3:4", "full_figure": "2:3"}.get(fr, "1:1")},
        "prompt": f"{subject} (OpenArt history {hid})",
    }
    specs.append(spec)
    catalog.append({"id": id_, "file": id_ + ".png", "role": "texture" if is_tex else ("figure" if id_.startswith("figure_") else "object"),
                    "subject": subject, "tags": sorted(set(tags)), "alpha": not is_tex,
                    "cutout_bbox": None if is_tex else key["cutout_bbox_output_norm"],
                    "open_edge": bool(key and key["touches_frame"]),
                    "source": gen})
json.dump({"version": "0.1", "specs": specs}, open(os.path.join(LIB, "prompts.json"), "w"), indent=1)

r = subprocess.run([ENGINE, "ingest-assets", os.path.join(LIB, "prompts.json"), LIB,
                    "-o", os.path.join(LIB, "manifest.json"), "--json"], capture_output=True, text=True)
report = json.loads(r.stdout)
status = {}
for a in report["assets"]:
    worst = "PASS"
    for c in a.get("checks", []):
        if c.get("level", c.get("status")) in ("FAIL", "fail"):
            worst = "FAIL"
        elif c.get("level", c.get("status")) in ("WARN", "warn", "WARNING", "warning") and worst == "PASS":
            worst = "WARN"
    status[a.get("id", "").replace("library.", "")] = (worst, [c for c in a.get("checks", []) if c.get("level", c.get("status")) not in ("PASS", "pass")])
manifest = json.load(open(os.path.join(LIB, "manifest.json")))
by_id = {e["id"].replace("library.", ""): e for e in manifest["assets"]}
for c in catalog:
    e = by_id.get(c["id"])
    c["qa"] = status.get(c["id"], ("MISSING", []))[0]
    if e and e.get("analysis"):
        an = e["analysis"]
        c["analysis"] = {"subject_bounds": an.get("subject_bounds"), "coverage": an.get("coverage"),
                         "edges": an.get("edges")}
        c["width"], c["height"] = e["width"], e["height"]
json.dump({"version": "0.1", "family": "editorial_cutout", "assets": catalog},
          open(os.path.join(LIB, "catalog.json"), "w"), indent=1)
with open(os.path.join(LIB, "LICENSE.md"), "w") as f:
    f.write("# editorial_cutout asset family\n\n"
            "Generated with OpenArt (GPT Image 2.5 Flare / Sunburst, low quality, 1k) in the project "
            f"'{PROJECT}' for MotionEngine 0.8 on 2026-10-01. Cutouts keyed from a magenta ground by "
            "scripts/key_cutout.py.\n")
print(json.dumps({k: v[0] for k, v in sorted(status.items())}))
for k, (s, issues) in sorted(status.items()):
    if s != "PASS":
        print(k, s, [i.get("message", i.get("detail")) for i in issues])
