#!/usr/bin/env bash
# (0.20 A4) Ground-truth speech fixtures for the word-timing benchmark
# (cargo test -p motion-voice --test timing_benchmark -- --ignored --nocapture).
#
# Compiles scripts/tts_groundtruth.swift into target/tools/ (skipped while the
# binary is newer than the source), then writes into output/groundtruth/ (or
# $MOTION_GROUNDTRUTH_DIR):
#   fixture_us_045 / fixture_us_055   male en-US voice, rate 0.45 / 0.55, scripts 1 / 2
#   fixture_gb_045 / fixture_gb_055   male en-GB voice, rate 0.45 / 0.55, scripts 3 / 4
#   fixture_us_045_music              fixture_us_045 + a music bed at -18 dB (EBU R128
#                                     integrated loudness) relative to the voice
# each as <stem>.wav (48 kHz mono s16) + .words.json + .script.txt + .meta.json.
#
# Voices: the best installed male voice per locale (premium > enhanced >
# default; declared-male com.apple.voice.* voices first, then the Eloquence
# male voices, which report no gender, then any declared-male voice). A voice
# that yields no word markers is skipped. Override with GT_VOICE_US /
# GT_VOICE_GB; the bed with GT_MUSIC_BED (catalog id) and GT_MUSIC_OFFSET (s).
# macOS 13+ only.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/scripts/tts_groundtruth.swift"
BIN="${CARGO_TARGET_DIR:-$ROOT/target}/tools/tts_groundtruth"
OUT="${MOTION_GROUNDTRUTH_DIR:-$ROOT/output/groundtruth}"
CATALOG="$ROOT/assets/music/catalog.json"
BED_ID="${GT_MUSIC_BED:-driving_pulse}"
BED_OFFSET="${GT_MUSIC_OFFSET:-30}"
BED_DB_BELOW_VOICE=18

die() { echo "build_groundtruth: $*" >&2; exit 1; }

[[ "$(uname -s)" == Darwin ]] || die "needs macOS 13+ (AVSpeechSynthesizer)"
command -v swiftc >/dev/null || die "swiftc not found (install the Xcode command line tools)"
command -v ffmpeg >/dev/null || die "ffmpeg not found"
command -v jq >/dev/null || die "jq not found"

# 1. Helper (rebuilt only when the source is newer than the binary).
if [[ -x "$BIN" && "$BIN" -nt "$SRC" ]]; then
  echo "helper: $BIN (up to date)"
else
  mkdir -p "$(dirname "$BIN")"
  echo "helper: compiling $SRC"
  swiftc -O "$SRC" -o "$BIN"
fi
mkdir -p "$OUT"

# 2. Voices.
VOICES="$("$BIN" --list-voices)"

# Candidate identifiers for a locale, best first.
candidates() {
  printf '%s\n' "$VOICES" | awk -F'\t' -v lang="$1" '
    NR == 1 || $3 != lang { next }
    {
      q = ($5 == "premium") ? 3 : ($5 == "enhanced") ? 2 : 1
      male = ($4 == "male")
      # Eloquence male voices report no gender.
      if (!male && $1 ~ /^com\.apple\.eloquence\./ && $2 ~ /^(Reed|Eddy|Rocko)$/) male = 2
      if (!male) next
      fam = ($1 ~ /^com\.apple\.voice\./) ? 3 : ($1 ~ /^com\.apple\.eloquence\./) ? 2 : 1
      pref = ($2 == "Reed") ? 3 : ($2 == "Eddy") ? 2 : 1
      printf "%d\t%s\n", q * 1000 + fam * 100 + pref, $1
    }' | sort -t$'\t' -k1,1nr -k2,2 | cut -f2
}

# gen_pair <label> <override> <lang> <stem1> <rate1> <script1> <stem2> <rate2> <script2>
# Generates the first fixture with the first candidate that yields word
# markers, then the second with the same voice.
gen_pair() {
  local label="$1" override="$2" lang="$3"
  local list
  if [[ -n "$override" ]]; then list="$override"; else list="$(candidates "$lang")"; fi
  [[ -n "$list" ]] || die "no male $lang voice installed (see: $BIN --list-voices)"
  local voice=""
  for v in $list; do
    if "$BIN" "$OUT" --name "$4" --voice "$v" --rate "$5" --script-id "$6"; then
      voice="$v"
      break
    fi
    echo "voice $v failed (no audio or no word markers); trying the next one" >&2
  done
  [[ -n "$voice" ]] || die "no $lang voice produced word markers"
  "$BIN" "$OUT" --name "$7" --voice "$voice" --rate "$8" --script-id "$9"
  echo "$label voice: $voice"
}

gen_pair "en-US" "${GT_VOICE_US:-}" en-US fixture_us_045 0.45 1 fixture_us_055 0.55 2
gen_pair "en-GB" "${GT_VOICE_GB:-}" en-GB fixture_gb_045 0.45 3 fixture_gb_055 0.55 4

# 3. Music fixture: fixture_us_045 + bed at -18 dB (integrated loudness).
BED_FILE="$(jq -r --arg id "$BED_ID" '[.beds[] | select(.id == $id)][0].file // empty' "$CATALOG")"
if [[ -z "$BED_FILE" ]]; then
  BED_ID="$(jq -r '.beds[0].id' "$CATALOG")"
  BED_FILE="$(jq -r '.beds[0].file' "$CATALOG")"
fi
BED="$ROOT/assets/music/$BED_FILE"
[[ -f "$BED" ]] || die "music bed $BED missing"
VOICE_WAV="$OUT/fixture_us_045.wav"
DUR="$(jq -r '.duration' "$OUT/fixture_us_045.meta.json")"

# Integrated loudness (LUFS) of an input after an optional filter chain.
lufs() { # <filters> <ffmpeg input args...>
  local filters="$1"
  shift
  ffmpeg -nostats -hide_banner "$@" -af "${filters}ebur128" -f null - 2>&1 |
    awk '/^ *I:/ { v = $2 } END { print v }'
}
BED_CHAIN="aresample=48000,aformat=channel_layouts=mono,"
VOICE_LUFS="$(lufs "" -i "$VOICE_WAV")"
BED_LUFS="$(lufs "$BED_CHAIN" -ss "$BED_OFFSET" -t "$DUR" -i "$BED")"
[[ -n "$VOICE_LUFS" && -n "$BED_LUFS" ]] || die "loudness measurement failed"
GAIN="$(awk -v v="$VOICE_LUFS" -v b="$BED_LUFS" -v d="$BED_DB_BELOW_VOICE" 'BEGIN { printf "%.2f", (v - d) - b }')"

MIX="$OUT/fixture_us_045_music.wav"
ffmpeg -nostdin -y -hide_banner -loglevel error \
  -i "$VOICE_WAV" -ss "$BED_OFFSET" -t "$DUR" -i "$BED" \
  -filter_complex "[1:a]${BED_CHAIN}volume=${GAIN}dB,apad[bed];[0:a][bed]amix=inputs=2:duration=first:normalize=0[mix]" \
  -map "[mix]" -ar 48000 -ac 1 -c:a pcm_s16le -fflags +bitexact -flags:a +bitexact "$MIX"
cp "$OUT/fixture_us_045.words.json" "$OUT/fixture_us_045_music.words.json"
cp "$OUT/fixture_us_045.script.txt" "$OUT/fixture_us_045_music.script.txt"

MIX_LUFS="$(lufs "" -i "$MIX")"
MIX_PEAK="$(ffmpeg -nostats -hide_banner -i "$MIX" -af volumedetect -f null - 2>&1 |
  awk '/max_volume/ { print $5 }')"
VOICE_BYTES="$(wc -c <"$VOICE_WAV" | tr -d ' ')"
MIX_BYTES="$(wc -c <"$MIX" | tr -d ' ')"
jq -n --arg bed "$BED_ID" --arg file "$BED_FILE" --argjson offset "$BED_OFFSET" \
  --argjson voice_lufs "$VOICE_LUFS" --argjson bed_lufs "$BED_LUFS" --argjson gain_db "$GAIN" \
  --argjson below "$BED_DB_BELOW_VOICE" --argjson mix_lufs "$MIX_LUFS" --arg peak "$MIX_PEAK" \
  '{source: "fixture_us_045", bed_id: $bed, bed_file: $file, bed_offset_s: $offset,
    bed_db_below_voice: $below, voice_lufs: $voice_lufs, bed_lufs_before_gain: $bed_lufs,
    bed_gain_db: $gain_db, mix_lufs: $mix_lufs, mix_peak_db: $peak}' \
  >"$OUT/fixture_us_045_music.meta.json"
echo "music: bed $BED_ID from ${BED_OFFSET}s, voice ${VOICE_LUFS} LUFS, bed ${BED_LUFS} LUFS -> gain ${GAIN} dB, mix peak ${MIX_PEAK} dB"
[[ "$VOICE_BYTES" == "$MIX_BYTES" ]] ||
  echo "warning: mix size $MIX_BYTES bytes differs from the voice ($VOICE_BYTES bytes)" >&2

echo "fixtures in $OUT:"
for f in "$OUT"/*.meta.json; do
  jq -r --arg f "$(basename "$f" .meta.json)" \
    'if .voice_identifier then "  \($f): \(.voice_identifier) rate \(.rate) script \(.script_id) \(.duration)s, \(.word_markers) word markers / \(.script_tokens) script tokens, \(.gt_words) timed (\(.marker_source))" else "  \($f): \(.source) + \(.bed_id) at -\(.bed_db_below_voice) dB" end' "$f"
done
