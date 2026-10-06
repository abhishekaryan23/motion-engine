#!/usr/bin/env bash
# MotionEngine 0.7 reference benchmark: ONE byte-identical CreativeIntent styled by
# each reference's validated ReferenceStyleProfile (written by an EXTERNAL multimodal
# interpreter from the bundle made by `reference-evidence`).
#   usage: scripts/reference_benchmark.sh references/benchmark/reference_a [more dirs...]
# Each dir must hold evidence.json + reference-style.json. Nothing is tuned per reference.
set -euo pipefail
cd "$(dirname "$0")/.."
ME=target/release/motion-engine
INTENT=examples/demo_05/late_night_worker.intent.json
STYLE=examples/demo_05/late_night_worker.style.json   # all-auto: the reference fills taste
cargo build --release -q -p motion-cli
python3 scripts/reference_calibration.py
if [ "$#" -eq 0 ]; then
  echo 'usage: scripts/reference_benchmark.sh <bundle-dir> [bundle-dir...]' >&2
  exit 2
fi
for DIR in "$@"; do
  NAME=$(basename "$DIR")
  if [ "$NAME" = current ]; then NAME=$(basename "$(dirname "$DIR")"); fi
  W="$DIR/work"; mkdir -p "$W"
  echo "== $NAME"
  $ME validate-reference-style "$DIR/reference-style.json" --bundle "$DIR" --cache-dir output/reference-cache > "$DIR/validation.txt"
  $ME resolve-style "$INTENT" --style "$STYLE" --reference-style "$DIR/reference-style.json" > "$DIR/coverage.txt"
  $ME resolve-style "$INTENT" --style "$STYLE" --reference-style "$DIR/reference-style.json" --json > "$DIR/coverage.json"
  cp "$INTENT" "$W/intent.json"
  $ME plan-assets "$INTENT" --style "$STYLE" --reference-style "$DIR/reference-style.json" -o "$W/asset-plan.json"
  $ME asset-prompts "$W/asset-plan.json" -o "$W/asset-prompts.json"
  python3 scripts/external_generator/fixture_generator.py "$W/asset-prompts.json" "$W/delivery"
  $ME ingest-assets "$W/asset-prompts.json" "$W/delivery" -o "$W/manifest.json" --cache "$W/cache" | tail -1
  $ME compile "$INTENT" --style "$STYLE" --reference-style "$DIR/reference-style.json" \
    --asset-manifest "$W/manifest.json" -o "$W/$NAME.motion.json"
  $ME render "$W/$NAME.motion.json" --out-dir "$W/render" --keep-frames | tail -1
  $ME qa "$W/$NAME.motion.json" --step 2 --json > "$DIR/qa.json"
  ffmpeg -v error -y -pattern_type glob -i "$W/render/frame_*.png" \
    -vf "select='not(mod(n\,30))',scale=216:384,tile=6x3" -frames:v 1 "$DIR/render-sheet.png"
  cp "$W/render/"*.mp4 "$DIR/$NAME-styled.mp4"
  rm -f "$W"/render/frame_*.png
  shasum -a 256 "$INTENT" | cut -d' ' -f1 > "$DIR/intent.sha256"
done
