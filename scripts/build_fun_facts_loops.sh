#!/usr/bin/env bash
# Package the fun_facts OpenArt image->video loops into
# assets/library/fun_facts/loops/<id>/ + loops/catalog.json.
# Raw MP4s live in output/openart/fun_facts/loops_raw/<id>.mp4 (PixVerse V6
# image2video, 720p, 4 s, start frame = end frame, from the magenta-ground
# still of the same asset; project "MotionEngine Asset Library").
# Same conventions as build_loops.sh, but frames are kept at 512 px short side
# and NOT palette-quantised (engraving / voxel / vector detail would band).
# Rejected on review: sun (seam SSIM 0.835), thermometer (liquid detaches).
set -euo pipefail
cd "$(dirname "$0")/.."
RAW=output/openart/fun_facts/loops_raw
FAM=fun_facts
LIB=assets/library/$FAM

# id | host asset | history id
LOOPS="
shark|shark|z5B8UL6Kkxt8qBCRP5JO
wombat|wombat|BhTFL4SEjUJ5csRw75KN
world_globe|world_globe|5PO07UcFaAOkKa7OddSX
tree|tree|bfXBcIEYnTcOrIz6bUyQ
"

for line in $LOOPS; do
  IFS='|' read -r id host hist <<<"$line"
  src="$RAW/$id.mp4"
  [ -f "$src" ] || { echo "skip $id (no $src)"; continue; }
  out="$LIB/loops/$id"
  rm -rf "$out"
  python3 scripts/extract_sprite.py "$src" "$out" --key --fps 12 --short 512 >/dev/null
  python3 - "$LIB/loops/catalog.json" "$id" "$host" "$hist" "$out/sprite.json" <<'EOF'
import json, os, sys
cat_path, lid, host, hist, meta_path = sys.argv[1:]
meta = json.load(open(meta_path))
cat = json.load(open(cat_path)) if os.path.exists(cat_path) else {"version": "0.1", "loops": []}
entry = {
    "id": lid, "host": host, "role": "hero_loop", "path": f"loops/{lid}",
    "frame_count": meta["frame_count"], "fps": meta["fps"], "mode": "loop",
    "pattern": meta["pattern"], "width": meta["width"], "height": meta["height"],
    "ssim_first_last": meta["ssim_first_last"],
    "source": {"vendor": "openart", "project": "MotionEngine Asset Library",
               "model": "pixverseV6 image2video 720p 4s (start = end frame)", "history_id": hist},
}
cat["loops"] = sorted([l for l in cat["loops"] if l["id"] != lid] + [entry], key=lambda l: l["id"])
os.makedirs(os.path.dirname(cat_path), exist_ok=True)
json.dump(cat, open(cat_path, "w"), indent=2)
print(f"{lid}: {meta['frame_count']} frames, seam SSIM {meta['ssim_first_last']}")
EOF
done
