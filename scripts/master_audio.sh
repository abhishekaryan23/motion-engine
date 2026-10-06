#!/usr/bin/env bash
# Re-master a rendered reel's audio so `qa --speech --mixed` passes the peak check.
#   scripts/master_audio.sh in.mp4 out.mp4
# `render` limits at -1 dBTP before the AAC encoder, and AAC then overshoots by
# ~0.8 dB (fresh reels read -0.3 .. -0.5 dBTP; the check wants <= -0.7). This
# two-pass loudnorm (-16 LUFS, -2.2 dBTP ceiling before AAC) lands at about
# -16.1 LUFS / -1.6 dBTP. The video stream is copied, not re-encoded.
set -euo pipefail
in=$1 out=$2
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
ffmpeg -y -v error -i "$in" -vn -c:a pcm_f32le "$tmp/a.wav"
m=$(ffmpeg -hide_banner -i "$tmp/a.wav" -af loudnorm=I=-16:TP=-2.2:LRA=11:print_format=json -f null - 2>&1 | sed -n '/{/,/}/p')
get() { printf '%s' "$m" | python3 -c "import json,sys;print(json.load(sys.stdin)['$1'])"; }
ffmpeg -y -v error -i "$tmp/a.wav" -af "loudnorm=I=-16:TP=-2.2:LRA=11:measured_I=$(get input_i):measured_TP=$(get input_tp):measured_LRA=$(get input_lra):measured_thresh=$(get input_thresh):offset=$(get target_offset):linear=true" -ar 48000 -c:a pcm_f32le "$tmp/n.wav"
ffmpeg -y -v error -i "$in" -i "$tmp/n.wav" -map 0:v -map 1:a -c:v copy -c:a aac -b:a 192k -movflags +faststart "$out"
echo "mastered: $out"
