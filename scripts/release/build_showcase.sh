#!/usr/bin/env bash
# Build the showcase/ previews (GIF) and 720p MP4s from locally rendered videos.
#
# usage: scripts/release/build_showcase.sh [SRC_DIR] [OUT_DIR]
#   SRC_DIR  directory that holds the rendered sources (default: output)
#            showcase/capability_reel.mp4     45 chapters, 3.5 s each (2.5 s for the last 5)
#            replicate/typeplay/original.mp4  type styles demo
#            replicate/showreel/before_after.mp4  v2.1 (left) vs v2.2 (right), layering
#            replicate/studio/linen_vs_moss.mp4   bundled library (left) vs generated plates (right)
#   OUT_DIR  where the files go (default: showcase)
#
# Sources are only read. Needs ffmpeg with libx264 and palettegen (no drawtext needed).
set -euo pipefail

SRC=${1:-output}
OUT=${2:-showcase}
mkdir -p "$OUT"

REEL="$SRC/showcase/capability_reel.mp4"
TYPE="$SRC/replicate/typeplay/original.mp4"
LAYER="$SRC/replicate/showreel/before_after.mp4"
GEN="$SRC/replicate/studio/linen_vs_moss.mp4"
for f in "$REEL" "$TYPE" "$LAYER" "$GEN"; do
  [ -f "$f" ] || { echo "missing source: $f" >&2; exit 1; }
done

FF=(ffmpeg -hide_banner -loglevel error -y)
GIF_FPS=${GIF_FPS:-10}

# Chapter k (1-based) of the capability reel: start second.
chapter_start() {
  local k=$1
  if [ "$k" -le 40 ]; then
    awk -v k="$k" 'BEGIN { printf "%.3f", (k - 1) * 3.5 }'
  else
    awk -v k="$k" 'BEGIN { printf "%.3f", 140 + (k - 41) * 2.5 }'
  fi
}
chapter_len() { if [ "$1" -le 40 ]; then echo 3.5; else echo 2.5; fi; }

# Build a filter graph that cuts "start:dur" segments from input 0 and concatenates them.
# $1 = with audio (1/0), rest = segments.
concat_graph() {
  local audio=$1; shift
  local graph="" labels="" i=0 seg s d
  for seg in "$@"; do
    s=${seg%%:*}; d=${seg##*:}
    graph+="[0:v]trim=start=${s}:duration=${d},setpts=PTS-STARTPTS[v$i];"
    if [ "$audio" = 1 ]; then
      graph+="[0:a]atrim=start=${s}:duration=${d},asetpts=PTS-STARTPTS,afade=t=in:d=0.03,afade=t=out:st=$(awk -v d="$d" 'BEGIN{printf "%.3f", d-0.05}'):d=0.05[a$i];"
      labels+="[v$i][a$i]"
    else
      labels+="[v$i]"
    fi
    i=$((i + 1))
  done
  if [ "$audio" = 1 ]; then
    graph+="${labels}concat=n=${i}:v=1:a=1[cv][ca]"
  else
    graph+="${labels}concat=n=${i}:v=1:a=0[cv]"
  fi
  printf '%s' "$graph"
}

# GIF from a filtered video label: $1 input, $2 graph ending in [cv], $3 width, $4 out.
make_gif() {
  local in=$1 graph=$2 width=$3 out=$4
  "${FF[@]}" -i "$in" -filter_complex \
    "${graph};[cv]fps=${GIF_FPS},scale=${width}:-2:flags=lanczos,split[g1][g2];[g1]palettegen=max_colors=128:stats_mode=diff[p];[g2][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle[out]" \
    -map "[out]" -loop 0 "$out"
}

# MP4 at 720p (height or width capped), H.264 + AAC, web-friendly.
mp4_args=(-c:v libx264 -preset slow -crf 24 -pix_fmt yuv420p -c:a aac -b:a 128k -movflags +faststart)

segs_for() {
  local k out=()
  for k in "$@"; do out+=("$(chapter_start "$k"):$(chapter_len "$k")"); done
  printf '%s\n' "${out[@]}"
}
part_segs() {
  # $1 = seconds kept from each chapter start (skips the first 0.4 s), rest = chapters
  local keep=$1; shift
  local k
  for k in "$@"; do
    printf '%s:%s\n' "$(awk -v s="$(chapter_start "$k")" 'BEGIN{printf "%.3f", s+0.4}')" "$keep"
  done
}

# 1. Capability highlights: one chapter per technique (type, charts, diagrams, UI, code, photo
#    stacks, sketch, layering, shapes, lockups).
HL=($(segs_for 3 6 7 9 12 13 15 16 19 20 24 27 31 32 33 35 41 42 45))
"${FF[@]}" -i "$REEL" -filter_complex "$(concat_graph 1 "${HL[@]}");[cv]scale=1280:720:flags=lanczos[sv]" \
  -map "[sv]" -map "[ca]" "${mp4_args[@]}" "$OUT/capability_highlights_720p.mp4"

HL=($(part_segs 2.0 6 9 15 27 31 41))
make_gif "$REEL" "$(concat_graph 0 "${HL[@]}")" 1280 "$OUT/capability_highlights_preview.gif"

HL=($(part_segs 2.4 9 10 12 13))
make_gif "$REEL" "$(concat_graph 0 "${HL[@]}")" 1280 "$OUT/charts_diagrams_preview.gif"

HL=($(part_segs 3.0 15 16 17))
make_gif "$REEL" "$(concat_graph 0 "${HL[@]}")" 1280 "$OUT/product_ui_preview.gif"

# 2. Type styles (gradient, image and outline fills, case, doodles drawn with the type).
"${FF[@]}" -i "$TYPE" -vf "scale=1280:720:flags=lanczos" "${mp4_args[@]}" "$OUT/type_styles_720p.mp4"
make_gif "$TYPE" "[0:v]trim=start=0:duration=9,setpts=PTS-STARTPTS[cv]" 1280 "$OUT/type_styles_preview.gif"

# 3. Layering: before/after, v2.1 on the left, v2.2 (`over` + plate-less sketch) on the right.
#    The source ends on a lockup that is not part of the comparison; stop before it.
"${FF[@]}" -i "$LAYER" -t 11.6 -vf "scale=1920:-2:flags=lanczos" "${mp4_args[@]}" "$OUT/layering_before_after_720p.mp4"
make_gif "$LAYER" "[0:v]trim=start=0:duration=11.6,setpts=PTS-STARTPTS[cv]" 1280 "$OUT/layering_before_after_preview.gif"

# 4. Generated assets: bundled library (left) vs locally generated plates (right).
"${FF[@]}" -i "$GEN" -vf "scale=1920:-2:flags=lanczos" "${mp4_args[@]}" "$OUT/generated_assets_720p.mp4"
make_gif "$GEN" "[0:v]trim=start=0:duration=10,setpts=PTS-STARTPTS[cv]" 1280 "$OUT/generated_assets_preview.gif"

ls -l "$OUT"
