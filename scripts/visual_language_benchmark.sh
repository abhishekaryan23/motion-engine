#!/usr/bin/env bash
# MotionEngine 0.7.1 visual-language benchmark rerun.
# Reruns the EXACT 0.7 DNA case (byte-identical CreativeIntent + style, never edited) under
#   baseline (no reference), reference A, reference B
# and writes output/visual_language_benchmark/<case>/{baseline,reference-a,reference-b}/ + result.json + report.md.
# Also compiles the 0.2 complaint-rate (derived-metric) intent with each profile as a data regression check.
# Nothing is tuned: no profile, intent, style or engine edits. A failing CLI step is recorded, not fatal.
#   usage: scripts/visual_language_benchmark.sh
#   VL_BENCH_OUT=<dir> writes the case output there instead of output/visual_language_benchmark/<case>.
#   Keep <dir> at the same depth under the repo root: compiled scenes store a relative asset_root.
set -uo pipefail
cd "$(dirname "$0")/.."
ME=target/release/motion-engine
CASE=dna-translesion-style-transfer-01
SRC=golden/fixtures/benchmarks/$CASE/intent
INTENT=$SRC/attempt-01.intent.json
STYLE=$SRC/neutral.style.json
REF_A=references/benchmark071/reference_a/reference-style.json
REF_B=references/benchmark071/reference_b/reference-style.json
OUT=${VL_BENCH_OUT:-output/visual_language_benchmark/$CASE}
DATA_INTENT=$(ls golden/fixtures/benchmarks/complaint-rate-reversal-02/attempt-*.intent.json | head -1)
DATA_STYLE=golden/fixtures/benchmarks/complaint-rate-reversal-02/style.json

cargo build --release -q -p motion-cli 2>/dev/null || { echo 'build failed' >&2; exit 1; }

# step <run-dir> <name> <stdout-file|-> cmd...   : runs, records "name<TAB>rc" and stderr, never aborts
step() {
  local run=$1 name=$2 out=$3; shift 3
  mkdir -p "$run/logs"
  local rc
  if [ "$out" = "-" ]; then "$@" >"$run/logs/$name.out" 2>"$run/logs/$name.err"; rc=$?
  else "$@" >"$out" 2>"$run/logs/$name.err"; rc=$?; fi
  printf '%s\t%s\n' "$name" "$rc" >> "$run/steps.tsv"
  return 0
}

has_generated() { # plan.json -> exit 0 when some request is a generated_image
  python3 - "$1" <<'PY'
import json, sys
try:
    plan = json.load(open(sys.argv[1]))
except Exception:
    sys.exit(1)
sys.exit(0 if any(r.get("source") == "generated_image" for r in plan.get("requests", [])) else 1)
PY
}

run_dna() { # <name> [reference-style.json]
  local name=$1 ref=${2:-}
  local run=$OUT/$name
  rm -rf "$run"; mkdir -p "$run"
  local refargs=()
  if [ -n "$ref" ]; then
    refargs=(--reference-style "$ref")
    step "$run" validate-reference-style "$run/validation.txt" "$ME" validate-reference-style "$ref" --bundle "$(dirname "$ref")"
  fi
  step "$run" resolve-style "$run/coverage.txt" "$ME" resolve-style "$INTENT" --style "$STYLE" ${refargs[@]+"${refargs[@]}"}
  step "$run" resolve-style-json "$run/coverage.json" "$ME" resolve-style "$INTENT" --style "$STYLE" ${refargs[@]+"${refargs[@]}"} --json
  step "$run" plan-assets - "$ME" plan-assets "$INTENT" --style "$STYLE" ${refargs[@]+"${refargs[@]}"} -o "$run/asset-plan.json"
  step "$run" asset-prompts - "$ME" asset-prompts "$run/asset-plan.json" -o "$run/asset-prompts.json"
  local manifest=()
  # Optional image requests are recorded in the plan but NOT fulfilled by default:
  # no image model exists in this environment, and the procedural fixture producer
  # draws generic stand-in objects that would misrepresent the story. The engine
  # then keeps its procedural entity tokens. Set FIXTURE_IMAGES=1 to deliver them.
  if [ "${FIXTURE_IMAGES:-0}" = "1" ] && has_generated "$run/asset-plan.json"; then
    step "$run" fixture-generator - python3 scripts/external_generator/fixture_generator.py "$run/asset-prompts.json" "$run/delivery"
    step "$run" ingest-assets - "$ME" ingest-assets "$run/asset-prompts.json" "$run/delivery" -o "$run/manifest.json" --cache "$run/cache"
    [ -f "$run/manifest.json" ] && manifest=(--asset-manifest "$run/manifest.json")
  fi
  step "$run" compile - "$ME" compile "$INTENT" --style "$STYLE" ${refargs[@]+"${refargs[@]}"} ${manifest[@]+"${manifest[@]}"} -o "$run/dna.motion.json"
  step "$run" render - "$ME" render "$run/dna.motion.json" --out-dir "$run/render" --keep-frames
  step "$run" qa-json "$run/qa.json" "$ME" qa "$run/dna.motion.json" --step 2 ${refargs[@]+"${refargs[@]}"} --json
  step "$run" qa-text "$run/qa.txt" "$ME" qa "$run/dna.motion.json" --step 2 ${refargs[@]+"${refargs[@]}"}
  # contact sheet: every 24th frame at 180x320
  step "$run" contact-sheet - ffmpeg -v error -y -pattern_type glob -i "$run/render/frame_*.png" \
    -vf "select='not(mod(n\,24))',scale=180:320,tile=7x4" -frames:v 1 -update 1 "$run/sheet.png"
  # one mid-EVOLVE frame per beat: scene start + (evolve+anticipate)/2
  python3 - "$run" <<'PY'
import json, os, shutil, sys
run = sys.argv[1]
try:
    m = json.load(open(f"{run}/dna.motion.json"))
    fps = m["canvas"]["fps"]
    n = 0
    for s in m["scenes"]:
        lc = s.get("lifecycle")
        if not lc or not s["id"].startswith("beat_"):
            continue
        n += 1
        t = s["start_seconds"] + (lc["evolve"] + lc["anticipate"]) / 2
        src = f"{run}/render/frame_{int(round(t * fps)):06d}.png"
        if os.path.exists(src):
            shutil.copy(src, f"{run}/beat{n}.png")
except Exception as e:
    print("beat frames failed:", e, file=sys.stderr)
PY
  # The sheet and beat stills are made; the MP4 holds the rest.
  rm -f "$run"/render/frame_*.png
  local mp4
  mp4=$(ls "$run"/render/*.mp4 2>/dev/null | head -1)
  if [ -n "$mp4" ]; then
    step "$run" ffprobe "$run/ffprobe.json" ffprobe -v error -print_format json -show_streams -show_format "$mp4"
  fi
}

run_data() { # <name> [reference-style.json] : plan + compile only (data-story regression)
  local name=$1 ref=${2:-}
  local run=$OUT/data-regression/$name
  rm -rf "$run"; mkdir -p "$run"
  local refargs=()
  [ -n "$ref" ] && refargs=(--reference-style "$ref")
  step "$run" plan-assets - "$ME" plan-assets "$DATA_INTENT" --style "$DATA_STYLE" ${refargs[@]+"${refargs[@]}"} -o "$run/asset-plan.json"
  step "$run" compile - "$ME" compile "$DATA_INTENT" --style "$DATA_STYLE" ${refargs[@]+"${refargs[@]}"} -o "$run/data.motion.json"
}

mkdir -p "$OUT"
echo "== baseline";    run_dna baseline
echo "== reference-a"; run_dna reference-a "$REF_A"
echo "== reference-b"; run_dna reference-b "$REF_B"
echo "== data regression ($DATA_INTENT)"
run_data baseline; run_data reference-a "$REF_A"; run_data reference-b "$REF_B"

python3 scripts/visual_language_report.py "$OUT" --intent "$INTENT" --data-intent "$DATA_INTENT" \
  --ref-a "$REF_A" --ref-b "$REF_B"
