#!/usr/bin/env bash
# Public-interface smoke test: compile -> validate -> full render (MP4) for the
# minimal public examples, then check the MP4 with ffprobe.
# Outputs go to output/public_smoke/<name>/ (gitignored).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

command -v ffprobe >/dev/null || { echo "ffprobe not found" >&2; exit 1; }

cargo build --release -p motion-cli
BIN=target/release/motion-engine
STYLE=examples/public/minimal.style.json

for name in minimal-emphasize minimal-contrast collection-accumulate state-change derived-metric; do
  echo "== $name"
  out="output/public_smoke/$name"
  rm -rf "$out"
  mkdir -p "$out"

  "$BIN" compile "examples/public/$name.intent.json" --style "$STYLE" \
    --output "$out/scene.motion.json"
  "$BIN" validate "$out/scene.motion.json"
  "$BIN" render "$out/scene.motion.json" --out-dir "$out/render"

  mp4=$(ls "$out"/render/*.mp4 | head -n 1)
  [ -f "$mp4" ] || { echo "FAIL: no MP4 in $out/render" >&2; exit 1; }

  info=$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=codec_name,width,height \
    -of csv=p=0 "$mp4")
  echo "   $mp4: $info"
  [ "$info" = "h264,1080,1920" ] || {
    echo "FAIL: expected h264,1080,1920 but got '$info'" >&2
    exit 1
  }
done

echo "public smoke: OK"
