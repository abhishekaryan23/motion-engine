#!/usr/bin/env bash
# Runs every offline CLI example documented in README.md and docs/CLI_CONSUMER_GUIDE.md and prints one
# PASS/FAIL line per command. No network, no API keys, no paid calls.
# Leaves nothing behind: work goes to a temp dir that is removed at the end
# (renders delete their own frames; single-frame PNGs are deleted here).
#
#   scripts/cli_smoke.sh [path/to/motion-engine]   # default target/release/motion-engine
#
# Commands that need the network, a key, Apple Vision, a whisper model or the
# a licensed SFX pack are listed as SKIP with the reason.
set -u
ME="${1:-target/release/motion-engine}"
[ -x "$ME" ] || { echo "no engine at $ME: cargo build --release -p motion-cli"; exit 2; }
T="$(mktemp -d "${TMPDIR:-/tmp}/motion-smoke.XXXXXX")"
trap 'rm -rf "$T"' EXIT
pass=0; fail=0
run() { # name, command...
  local name="$1"; shift
  if "$@" >"$T/log" 2>&1; then echo "PASS  $name"; pass=$((pass+1));
  else echo "FAIL  $name: $(tail -1 "$T/log" | cut -c1-160)"; fail=$((fail+1)); fi
}
skip() { echo "SKIP  $1: $2"; }

I=examples/editorial_demo.intent.json
S=examples/editorial_demo.style.json
P=examples/v2/flow_board.program.json

# v1: CreativeIntent -> MotionScene -> MP4
run compile            "$ME" -q compile "$I" --style "$S" --output "$T/demo.motion.json"
run validate           "$ME" -q validate "$T/demo.motion.json"
run inspect            "$ME" -q inspect "$T/demo.motion.json" --frame 60
run "render --frame"   "$ME" -q render "$T/demo.motion.json" --frame 60 --out-dir "$T/one"
rm -rf "$T/one"
run qa                 "$ME" -q qa "$T/demo.motion.json"
run critique           "$ME" -q critique "$T/demo.motion.json" --json
SFX="${MOTION_SFX_LIBRARY:-assets/sfx/library}"
if [ -f "$SFX/sfx-library.json" ]; then
  run plan-audio       "$ME" -q plan-audio "$T/demo.motion.json" --intent "$I" --sfx-library "$SFX"
else
  skip plan-audio      "no SFX library at $SFX (gitignored; build it with sfx-index from a licensed SFX pack, or set MOTION_SFX_LIBRARY)"
fi
run plan-assets        "$ME" -q plan-assets "$I" --output "$T/plan.json"
run asset-prompts      "$ME" -q asset-prompts "$T/plan.json" --output "$T/prompts.json"
run validate-assets    "$ME" -q validate-assets assets/library/editorial_cutout/manifest.json
run resolve-style      "$ME" -q resolve-style "$I" --style "$S"
run style-preview      "$ME" -q style-preview "$S" --output "$T/preview.png"
run compare-styles     "$ME" -q compare-styles "$S" examples/styles/documentary_dark.style.json
R=references/benchmark071/reference_a/reference-style.json
if [ -f "$R" ]; then run validate-reference-style "$ME" -q validate-reference-style "$R"
else skip validate-reference-style "no reference bundle in this checkout"; fi
run music-index        "$ME" -q music-index assets/music/calm_ambient.mp3 --output "$T/calm.music.json"
run "fonts list"       "$ME" -q fonts list
run "models list"      "$ME" -q models list
run "providers show"   "$ME" -q providers show
if [ "$(uname -s)" = Darwin ]; then
  run "voice (say, offline)" "$ME" -q voice "$I" --style "$S" --tts-model say --align onset --output "$T/voice"
else skip "voice (say, offline)" "macOS say only"; fi
run "render (full)"    "$ME" -q render "$T/demo.motion.json"

# v2: MotionProgram
run "program validate" "$ME" -q program validate "$P"
run "program compile"  "$ME" -q program compile "$P" --output "$T/flow.motion.json"
run "program qa"       "$ME" -q program qa "$P"
run "make (no render)" "$ME" -q make --brief "Make a dynamic 15 s showreel about motion design. Go all out." --no-render --out-dir "$T/make"

skip "program render"  "writes output/<name>/ (run it by hand: ~2-25 s per sample)"
skip "program learn"   "edits assets/qa/mistakes.json (run by hand when you record a new mistake)"
skip "reel"            "needs the free OpenRouter voice key (providers set-key openrouter) or --tts-model say + minutes of render"
skip "edit"            "needs a talking-head video and the local whisper model (models fetch whisper-base.en, asr-local build)"
skip "explore"         "renders K variants (minutes); same pipeline as compile + render"
skip "mix"             "needs a rendered MP4 + its audio plan + speech map (from reel/voice)"
skip "reference-evidence" "needs a reference video"
skip "ingest-assets"   "needs generated images for a prompt set"
skip "matte"           "Apple Vision (macOS 14+) on your photos"
skip "sfx-index / sfx-pack" "needs a licensed SFX pack of your own"
skip "providers set-key/test/connect/tools/call/quote/generate" "network + your provider accounts; generate spends credits"
skip "models fetch"    "downloads ~148 MB (pinned SHA256)"

echo "smoke: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
