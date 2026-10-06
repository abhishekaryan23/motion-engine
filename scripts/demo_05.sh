#!/usr/bin/env bash
# MotionEngine 0.5 image-led demo: the full vendor-neutral generated-asset handoff.
#   CreativeIntent → plan-assets → asset-prompts → EXTERNAL generator (here the
#   procedural fixture producer, a stand-in for any image model) → ingest-assets
#   (decode · validate · analyze · cache) → validate-assets → compile → render → qa
set -euo pipefail
cd "$(dirname "$0")/.."
ME=target/release/motion-engine
EX=examples/demo_05
OUT=output/demo_05
cargo build --release -q -p motion-cli
mkdir -p "$OUT"
$ME plan-assets "$EX/late_night_worker.intent.json" --style "$EX/late_night_worker.style.json" -o "$OUT/asset-plan.json"
$ME asset-prompts "$OUT/asset-plan.json" -o "$OUT/asset-prompts.json"
# External producer (outside the engine). Swap for any generator honouring docs/GENERATED_ASSET_PROTOCOL.md.
python3 scripts/external_generator/fixture_generator.py "$OUT/asset-prompts.json" "$OUT/delivery"
$ME ingest-assets "$OUT/asset-prompts.json" "$OUT/delivery" -o "$OUT/manifest.json" --cache "$OUT/cache"
# Second ingest with an EMPTY delivery dir: every image comes from the cache (no regeneration).
mkdir -p "$OUT/empty_delivery"
$ME ingest-assets "$OUT/asset-prompts.json" "$OUT/empty_delivery" -o "$OUT/manifest.cached.json" --cache "$OUT/cache" | tail -1
$ME validate-assets "$OUT/manifest.json" --prompts "$OUT/asset-prompts.json" | tail -1
$ME compile "$EX/late_night_worker.intent.json" --style "$EX/late_night_worker.style.json" \
  --asset-manifest "$OUT/manifest.json" -o "$OUT/late_night_worker.motion.json"
$ME render "$OUT/late_night_worker.motion.json" --out-dir "$OUT/render"
$ME qa "$OUT/late_night_worker.motion.json" --step 2 | tail -3
ffprobe -v error -select_streams v:0 -show_entries stream=codec_name,width,height,r_frame_rate,nb_frames \
  -show_entries format=duration -of compact "$OUT/render/late_night_worker.mp4"
