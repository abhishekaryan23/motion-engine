#!/usr/bin/env bash
# 0.10: one voice-led reel end to end (operator script).
#   scripts/voice_reel.sh <intent> <style> <tts-model> <out_dir> [compile flags...]
# voice (cached) → compile --speech → plan-audio --speech → render --speech
# (VO + SFX, ducking, makeup auto) → qa --speech --mixed.
set -euo pipefail
intent=$1 style=$2 model=$3 out=$4; shift 4
E=${MOTION_ENGINE:-./target/release/motion-engine}
SFX=${MOTION_SFX_LIBRARY:-assets/sfx/library}
name=$(python3 -c "import json,sys;print(json.load(open(sys.argv[1]))['title'])" "$intent")
mkdir -p "$out"
$E voice "$intent" --style "$style" --tts-model "$model" -o "$out" | grep -E "provider calls"
$E compile "$intent" --style "$style" --speech "$out/$name.speech.json" "$@" --output "$out/$name.motion.json" | tail -1
$E plan-audio "$out/$name.motion.json" --intent "$intent" --style "$style" --sfx-library "$SFX" --speech "$out/$name.speech.json" -o "$out/$name.audio.json" >/dev/null
$E render "$out/$name.motion.json" --speech "$out/$name.speech.json" --sfx-library "$SFX" --audio-plan "$out/$name.audio.json" --makeup-gain auto >/dev/null
mp4=$(ls -t "$out"/"$name"/*.mp4 2>/dev/null | head -1 || true)
[ -n "$mp4" ] || mp4=$(ls -t "$out"/*.mp4 | head -1)
$E qa "$out/$name.motion.json" --speech "$out/$name.speech.json" --audio-plan "$out/$name.audio.json" --mixed "$mp4"
rm -f "$out/$name"/frame_*.png
echo "video: $mp4"
