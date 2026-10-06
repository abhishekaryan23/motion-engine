#!/usr/bin/env bash
# Sprint 0.23 variety bench (W0): compile the benchmark matrix the way the product
# path compiles (speech-led, `--art auto --variety auto`), then write
# output/sprint23/bench/variety_report.json: structural variety (take distance,
# cross-story distance, consecutive repeats, tone distinctness), determinism,
# compile warnings and hard QA findings: `qa --layout` (incl. text_local_contrast) and
# `qa --speech <fixture>` (the four 0.23 checks text_local_contrast, dead_air,
# count_unsettled, repeat_template; FAIL / WARN lists in `qa_checks`, `hard_qa`,
# `warn_qa`). Offline: speech maps are committed
# fixtures (golden/fixtures/variety/*.speech.json), no provider is ever called.
#
# Usage:  scripts/variety_bench.sh
# Env:    TAKES=0            takes to compile (space or comma separated, default 0);
#                            `--take N` is passed only for N > 0 and only when
#                            `compile --help` lists `--take`, else the report says
#                            "takes": "unavailable" and runs take 0 only
#         SHEETS=1           also write contact sheets (3 stories x 4 tones, one row
#                            per take) to output/sprint23/bench/sheets/
#         STRICT=1           exit 1 on any compile failure, hard layout-QA finding,
#                            non-deterministic compile or failed sheet (default: the
#                            report is written and the exit code is 0)
#         SKIP_BUILD=1       do not run `cargo build --release -p motion-cli`
#         ENGINE=PATH        engine binary (default target/release/motion-engine)
#         OUT=DIR            output directory (default output/sprint23/bench, or
#                            output/sprint23/bench_tight with SPEECH=tight)
#         SPEECH=tight       compile with the continuous-take fixtures
#                            (golden/fixtures/variety/tight/*.speech.json: every gap
#                            between sentences 0.30 s) instead of the `say` ones
#         REGEN_TIGHT=1      rebuild the tight fixtures from the default ones and exit
#                            (docs/plans/sprint_0_23/tools/tighten_speech.py --all)
#         REGEN_FIXTURES=1   re-synthesise the speech fixtures and exit:
#                              motion-engine voice <intent> --tts-model say --align onset -o <tmp>
#                            for each story (macOS only); only the .speech.json is
#                            kept, the wav lives in a temp dir that is deleted.
#
# Metric definitions live in docs/plans/sprint_0_23/tools/variety_report.py.
# Rendered frames are never kept: sheet.py deletes its own, and this script sweeps
# the output directory for any stray frame_*.png on exit.
set -euo pipefail
cd "$(dirname "$0")/.."

TOOLS=docs/plans/sprint_0_23/tools
ENGINE=${ENGINE:-target/release/motion-engine}
SPEECH=${SPEECH:-default}
case "$SPEECH" in
  default) OUT=${OUT:-output/sprint23/bench} ;;
  tight) OUT=${OUT:-output/sprint23/bench_tight} ;;
  *) echo "variety_bench: SPEECH must be default or tight, not '$SPEECH'" >&2; exit 2 ;;
esac
TAKES=${TAKES:-0}
SHEETS=${SHEETS:-0}
FIXTURES=golden/fixtures/variety

if [[ "${REGEN_TIGHT:-0}" == 1 ]]; then
  python3 "$TOOLS/tighten_speech.py" --all
  exit 0
fi

if [[ "${SKIP_BUILD:-0}" != 1 ]]; then
  cargo build --release -p motion-cli -q
fi
if [[ ! -x "$ENGINE" ]]; then
  echo "variety_bench: no engine at $ENGINE (cargo build --release -p motion-cli)" >&2
  exit 2
fi

if [[ "${REGEN_FIXTURES:-0}" == 1 ]]; then
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/variety_voice.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  mkdir -p "$FIXTURES"
  while IFS=$'\t' read -r name path; do
    mkdir -p "$tmp/$name"
    "$ENGINE" voice "$path" --tts-model say --align onset -o "$tmp/$name" >/dev/null
    cp "$tmp/$name"/*.speech.json "$FIXTURES/$name.speech.json"
    echo "fixture $FIXTURES/$name.speech.json"
  done < <(python3 "$TOOLS/variety_report.py" --list-stories)
  exit 0
fi

mkdir -p "$OUT"
sweep_frames() {
  # Owner rule: no rendered frames are kept (only ever under the bench output).
  find "$OUT" -name 'frame_*.png' -delete 2>/dev/null || true
}
trap sweep_frames EXIT

args=(--engine "$ENGINE" --out "$OUT" --takes "$TAKES" --speech "$SPEECH")
if [[ "$SHEETS" == 1 ]]; then
  args+=(--sheets)
fi
if [[ "${STRICT:-0}" == 1 ]]; then
  args+=(--strict)
fi
exec_status=0
python3 "$TOOLS/variety_report.py" "${args[@]}" || exec_status=$?
exit "$exec_status"
