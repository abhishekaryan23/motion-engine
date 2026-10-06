#!/usr/bin/env bash
# A stand-in for `motion-engine` in motion-mcp's tests: no voice, no real render.
#
#   reel <intent.json> [-o OUT] [flags…]  prints the reel's stage lines, writes stub
#       scene / speech / audio JSON and a 1 s black 1080x1920 mp4 at
#       OUT/<title>/<title>.mp4, then the `video: <path>` line.
#   qa … --json                            prints a speech-QA report in the real shape.
#
# Environment:
#   FAKE_ENGINE_COUNTER  file: one line per reel run ("reel <args>")
#   FAKE_ENGINE_PIDFILE  file: the reel's pid (written when it starts)
#   FAKE_ENGINE_FAIL     non-empty: the reel fails in the voice stage
#   FAKE_ENGINE_SLEEP    seconds to sleep in the render stage
#   FAKE_ENGINE_QA       PASS (default) or FAIL
#   FAKE_ENGINE_TRACE    file: "start <title>" / "end <title>" around each reel
set -u

cmd="${1:-}"
[ $# -gt 0 ] && shift

case "$cmd" in
reel)
  [ -n "${FAKE_ENGINE_COUNTER:-}" ] && echo "reel $*" >> "$FAKE_ENGINE_COUNTER"
  [ -n "${FAKE_ENGINE_PIDFILE:-}" ] && echo "$$" > "$FAKE_ENGINE_PIDFILE"
  intent="${1:-}"
  [ $# -gt 0 ] && shift
  out="output/reel"
  while [ $# -gt 0 ]; do
    case "$1" in
      -o|--output) out="$2"; shift 2 ;;
      *) shift ;;
    esac
  done
  title=$(grep -m1 '"title"' "$intent" | sed -E 's/.*"title"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')
  [ -z "$title" ] && title=reel
  stem=$(printf '%s' "$title" | LC_ALL=C sed 's/[^A-Za-z0-9_-]/_/g')
  [ -n "${FAKE_ENGINE_TRACE:-}" ] && echo "start $title" >> "$FAKE_ENGINE_TRACE"

  echo "[1/5] voice (auto · emotion energy)"
  echo "  voice: model fake (single take, offline=true)"
  if [ -n "${FAKE_ENGINE_FAIL:-}" ]; then
    echo "Error: voice failed:" >&2
    echo "  voice: model fake" >&2
    echo "Error: provider returned 401 Unauthorized for key sk-or-v1-0123456789abcdef" >&2
    exit 1
  fi
  mkdir -p "$out/$title"
  echo '{"words": []}' > "$out/$stem.speech.json"
  echo "[3/5] music: none"
  echo "[2/5] compile (art auto, variety auto)"
  echo '{"version": "0.2", "scenes": []}' > "$out/$stem.motion.json"
  echo "  compiled 3 beat(s) -> $out/$stem.motion.json"
  echo "[4/5] sound design + render"
  echo '{"cues": []}' > "$out/$stem.audio.json"
  if [ -n "${FAKE_ENGINE_SLEEP:-}" ]; then
    sleep "$FAKE_ENGINE_SLEEP"
  fi
  video="$out/$title/$title.mp4"
  ffmpeg -nostdin -v error -y \
    -f lavfi -i color=c=black:s=1080x1920:d=1 \
    -f lavfi -i anullsrc=r=48000:cl=stereo \
    -shortest -c:v libx264 -preset ultrafast -pix_fmt yuv420p -c:a aac "$video" || exit 1
  echo "[5/5] speech QA"
  echo "  speech qa: ${FAKE_ENGINE_QA:-PASS}"
  [ -n "${FAKE_ENGINE_TRACE:-}" ] && echo "end $title" >> "$FAKE_ENGINE_TRACE"
  echo "video: $video"
  ;;
qa)
  verdict="${FAKE_ENGINE_QA:-PASS}"
  if [ "$verdict" = "PASS" ]; then peak=pass; else peak=fail; fi
  cat <<EOF
{
  "verdict": "$verdict",
  "checks": [
    {"name": "caption_timing", "status": "pass", "detail": "20 word(s), max delta 0.5 ms (limit 33.3 ms), 0 finding(s)"},
    {"name": "caption_safe_area", "status": "pass", "detail": "0 finding(s)"},
    {"name": "caption_lines", "status": "pass", "detail": "max 2 line(s), max 32 char(s)/line, 0 finding(s)"},
    {"name": "layout", "status": "pass", "detail": "0 finding(s)"},
    {"name": "sfx_vs_words", "status": "pass", "detail": "3 cue(s), 0 within 80 ms of a word onset"},
    {"name": "loudness", "status": "pass", "detail": "-16.1 LUFS (target -16 +-1)"},
    {"name": "peak", "status": "$peak", "detail": "true -0.7 dBTP, sample -0.7 dBFS (limit -1.0, codec margin 0.3)"},
    {"name": "bed_duck", "status": "skip", "detail": "no music bed"}
  ],
  "max_caption_delta_ms": 0.5,
  "caption_limit_ms": 33.333333333333336,
  "words": 20,
  "sfx_conflicts": [],
  "integrated_lufs": -16.1,
  "true_peak_db": -0.7,
  "sample_peak_db": -0.68,
  "bed_duck_db": null
}
EOF
  ;;
*)
  echo "fake motion-engine: unknown command '$cmd'" >&2
  exit 2
  ;;
esac
