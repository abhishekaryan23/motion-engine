#!/usr/bin/env bash
# 0.11: package OpenArt image->video loops into assets/library/<family>/loops/.
# Raw MP4s live in output/openart/loops/raw/ (downloaded from the OpenArt
# project "MotionEngine Asset Library"; PixVerse V6 image2video, 540p, 4 s,
# start frame = end frame). Each loop: <family>/loops/<id>/frame_%04d.png +
# sprite.json; <family>/loops/catalog.json lists them with the host asset.
set -euo pipefail
cd "$(dirname "$0")/.."
RAW=output/openart/loops/raw
LIB=assets/library

# Families whose palette has no violet/pink: loop frames also go through
# scripts/neutralize_tint.py (same rule as the family spec's neutralize_violet).
NO_VIOLET="ocean_layers"

# id | family | host asset (or "-" for a screen insert) | keyed | history id
LOOPS="
hourglass|classical_greyscale|hourglass|key|fmdNRXAXUmA9GfqoEjNv
piggy_bank|clay_props_3d|piggy_bank|key|voYnMVCqd746hLiJpxJh
clock|clay_props_3d|clock|key|6gM65sOty6mL7J7gEeZP
rocket|clay_props_3d|rocket|key|AwDfkQINi9cQlaM6OqZp
photic_zone|ocean_layers|photic_zone|key|P2zQ8vuMorshHvHq50LG
fish_skeleton|ocean_layers|fish_skeleton|key|XaI1LoPDpe3fiiGDQyp2
ocean_layers|ocean_layers|ocean_layers|key|ycvoma8u0d0ZqwrTzV13
density_column|ocean_layers|density_column|key|E4Ftzcwg6eS8ZjgX3HRI
oxygen_molecule|ocean_layers|oxygen_molecule|key|sAeFLjjDkG2J7UC86maE
tv_toaster_ad|halftone_retro_objects|-|insert|BjFe4MTtEFBgdRJWWR3Q
"

for line in $LOOPS; do
  IFS='|' read -r id fam host mode hist <<<"$line"
  # optional filter: scripts/build_loops.sh <id>... builds only those loops
  if [ $# -gt 0 ] && [[ " $* " != *" $id "* ]]; then continue; fi
  src="$RAW/$id.mp4"
  [ -f "$src" ] || { echo "skip $id (no $src)"; continue; }
  out="$LIB/$fam/loops/$id"
  if [ "$mode" = key ]; then
    python3 scripts/extract_sprite.py "$src" "$out" --key --fps 12 --short 384 --palette >/dev/null
  else
    python3 scripts/extract_sprite.py "$src" "$out" --fps 12 --short 360 --palette >/dev/null
  fi
  case " $NO_VIOLET " in *" $fam "*) python3 scripts/neutralize_tint.py "$out" >/dev/null ;; esac
  python3 - "$LIB/$fam/loops/catalog.json" "$id" "$host" "$mode" "$hist" "$out/sprite.json" <<'EOF'
import json, os, sys
cat_path, lid, host, mode, hist, meta_path = sys.argv[1:]
meta = json.load(open(meta_path))
cat = json.load(open(cat_path)) if os.path.exists(cat_path) else {"version": "0.1", "loops": []}
entry = {
    "id": lid,
    "host": None if host == "-" else host,
    "role": "screen_insert" if mode == "insert" else "hero_loop",
    "path": f"loops/{lid}",
    "frame_count": meta["frame_count"], "fps": meta["fps"], "mode": "loop",
    "pattern": meta["pattern"], "width": meta["width"], "height": meta["height"],
    "ssim_first_last": meta["ssim_first_last"],
    "source": {"vendor": "openart", "project": "MotionEngine Asset Library",
               "model": "pixverseV6 image2video 540p 4s (start = end frame)", "history_id": hist},
}
cat["loops"] = sorted([l for l in cat["loops"] if l["id"] != lid] + [entry], key=lambda l: l["id"])
json.dump(cat, open(cat_path, "w"), indent=2)
print(f"{lid}: {meta['frame_count']} frames, seam SSIM {meta['ssim_first_last']}")
EOF
done
