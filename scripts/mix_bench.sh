#!/usr/bin/env bash
# (0.23 W5a) Mix bench: does the narrator stay on top of every bed?
#
#   scripts/mix_bench.sh
#
# Offline (macOS `say`, no provider call). Synthesises a test voice from
# docs/plans/sprint_0_23/stories/sleep_review.intent.json, compiles it, plans
# the audio against every bed of assets/music/catalog.json and renders the
# video once. Voice copies are normalised to -14 / -20 / -26 LUFS. For every
# bed x voice level x {SFX on, SFX off} it runs `motion-engine mix --stems`
# (the voice-relative mix) and `mix --legacy --stems` (the pre-0.23 mix, the
# recorded baseline), measures the stems (docs/plans/sprint_0_23/tools/
# stem_levels.py) and the final MP4 (ffmpeg ebur128), and writes
# output/sprint23/mix_bench/report.json and report.md. Every stem and temp wav
# is deleted. Exit status 1 when a voice-relative run misses a target:
#   voice_over_music >= 15 dB, bed_over_voice <= -6 dB, final -16 +- 1 LUFS and
#   <= -1 dBTP, sfx_over_voice <= -6 dB (SFX runs).
# Each run is also measured by `motion-engine qa --speech --json` (the Rust
# stem checks `voice_over_music`, `bed_over_voice`, `speech_band_masking`,
# measured on stems QA rebuilds from the plan; the legacy mix is QA'd from a
# copy of the plan without `levels`). The report lists the Rust values beside
# the Python ones and states the worst |Rust - Python| difference; exit status
# 1 when a voice-relative run differs by more than 0.5 dB. The legacy runs
# with SFX on differ by design (QA does not rebuild the SFX duck of the legacy
# bed), so they are listed apart. Every run also checks that the stems, summed
# and raised by the makeup gain the mix logged, reproduce the final mix's
# integrated loudness within 0.5 LU.
# A second scenario (a narration with 1.2 s pauses between sentences, SFX on)
# exercises the bed's rise in long gaps.
# A third scenario (A5c, SFX on) is a peaky narration: the same voice with a
# plosive-like burst (150 Hz, 2 ms wide) at every third word onset, scaled so
# the voice's peak-to-loudness ratio is 20 dB (MIX_BENCH_PEAKY_PLR; the real
# Fish take that failed -16 LUFS was 19.3 dB). Made deterministically with
# ffmpeg from the `say` voice, never committed. Its runs go through the voice
# limiter; every voice-relative run must also keep its voice stem at -18 +- 1
# LUFS (`voice_integrated_lufs`). The report counts 63 runs of the first two
# scenarios (unchanged) plus the 21 peaky ones.
#
# Env: MOTION_SFX_LIBRARY (default assets/sfx/library, else the main checkout's),
#      MIX_BENCH_STYLE (a StyleProfile json: the look picks the MixLevels table;
#      default none = the standard table), MIX_BENCH_JOBS (parallel mixes,
#      default 4), MIX_BENCH_TMP (default /tmp), MIX_BENCH_PAUSE (the pause
#      length of the second scenario in seconds, default 1.2), MIX_BENCH_PEAKY_PLR
#      (the peak-to-loudness ratio of the third scenario in dB, default 20).
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
ROOT=$PWD
for tool in ffmpeg ffprobe python3 say; do
  command -v "$tool" >/dev/null || { echo "mix_bench: $tool not found" >&2; exit 1; }
done

SECONDS=0
cargo build --release -p motion-cli >/dev/null
E=$ROOT/target/release/motion-engine
TOOLS=$ROOT/docs/plans/sprint_0_23/tools
STORY=$ROOT/docs/plans/sprint_0_23/stories/sleep_review.intent.json
MUSIC=$ROOT/assets/music
OUT=$ROOT/output/sprint23/mix_bench
JOBS=${MIX_BENCH_JOBS:-4}
STYLE=()
[ -n "${MIX_BENCH_STYLE:-}" ] && STYLE=(--style "$MIX_BENCH_STYLE")

SFX=${MOTION_SFX_LIBRARY:-$ROOT/assets/sfx/library}
if [ ! -f "$SFX/sfx-library.json" ]; then
  common=$(cd "$(git rev-parse --git-common-dir)/.." && pwd)
  [ -f "$common/assets/sfx/library/sfx-library.json" ] && SFX=$common/assets/sfx/library
fi
[ -f "$SFX/sfx-library.json" ] || { echo "mix_bench: no SFX library (set MOTION_SFX_LIBRARY)" >&2; exit 1; }

# The physical path: plan-audio writes each bed's track relative to the plan's
# directory by path arithmetic, which a symlinked /tmp (macOS) would break for
# `qa --speech --audio-plan`.
TMPBASE=$(cd "${MIX_BENCH_TMP:-/tmp}" && pwd -P)
TMP=$(mktemp -d "$TMPBASE/mix_bench.XXXXXX")
MARK=$(mktemp "$TMPBASE/mix_bench.start.XXXXXX")
trap 'rm -rf "$TMP" "$MARK"' EXIT
mkdir -p "$OUT" "$TMP/runs"
rm -f "$OUT/report.json" "$OUT/report.md"

BEDS=$(python3 - "$MUSIC/catalog.json" <<'PY'
import json, sys
for b in json.load(open(sys.argv[1]))["beds"]:
    print(b["id"], b["plan"])
PY
)

# --- the test voice (offline) ------------------------------------------------
echo "== voice (say, onset timing)"
"$E" voice "$STORY" --tts-model say --align onset --cache "$TMP/cache" -o "$TMP/base" | grep -E "sentences|wrote .*speech"
BASE_SPEECH=$TMP/base/sleep_review.speech.json
BASE_VOICE=$TMP/base/sleep_review.voice.wav

# Voice copies at -14 / -20 / -26 LUFS (float wav: a linear gain, no clipping),
# each beside a copy of the speech file that points at it.
loudness() { # file -> integrated LUFS
  ffmpeg -nostats -hide_banner -i "$1" -af ebur128 -f null - 2>&1 | awk '/Summary:/{s=1} s && /^ +I:/{i=$2} END{print i}'
}
make_levels() { # scenario_dir voice_wav speech_json
  local dir=$1 voice=$2 speech=$3 i
  i=$(loudness "$voice")
  for lv in 14 20 26; do
    mkdir -p "$dir/lv$lv"
    local g
    g=$(python3 -c "print(round(-$lv - ($i), 3))")
    ffmpeg -y -v error -i "$voice" -af "volume=${g}dB" -c:a pcm_f32le "$dir/lv$lv/voice.wav"
    python3 - "$speech" "$dir/lv$lv/speech.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["audio"] = "voice.wav"
json.dump(m, open(sys.argv[2], "w"))
PY
  done
}
make_levels "$TMP/base" "$BASE_VOICE" "$BASE_SPEECH"

# --- a narration with long pauses (second scenario) --------------------------
mkdir -p "$TMP/gap"
python3 - "$BASE_SPEECH" "$TMP/gap/speech.json" "$TMP/gap/cuts.txt" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
sents = m["sentences"]
import os
PAUSE = float(os.environ.get("MIX_BENCH_PAUSE", "1.2"))
# cut at the middle of each inter-sentence gap; insert PAUSE seconds there.
cuts = [0.0] + [(sents[i]["end"] + sents[i + 1]["start"]) / 2 for i in range(len(sents) - 1)] + [m["duration"]]
shift = lambda t: t + PAUSE * sum(1 for c in cuts[1:-1] if c <= t)
for w in m["words"]:
    w["start"], w["end"] = round(shift(w["start"]), 4), round(shift(w["end"]), 4)
for s in m["sentences"]:
    s["start"], s["end"] = round(shift(s["start"]), 4), round(shift(s["end"]), 4)
m["duration"] = round(m["duration"] + PAUSE * (len(cuts) - 2), 4)
m["audio"] = "voice.wav"
json.dump(m, open(sys.argv[2], "w"))
open(sys.argv[3], "w").write(f"{PAUSE}\n" + "\n".join(f"{cuts[i]} {cuts[i+1]}" for i in range(len(cuts) - 1)) + "\n")
PY
PARTS=()
n=0
PAUSE=$(head -1 "$TMP/gap/cuts.txt")
while read -r a b; do
  ffmpeg -y -v error -i "$BASE_VOICE" -af "atrim=$a:$b,asetpts=PTS-STARTPTS,aformat=sample_rates=48000:channel_layouts=mono" -c:a pcm_f32le "$TMP/gap/p$n.wav"
  ffmpeg -y -v error -f lavfi -i "anullsrc=r=48000:cl=mono" -t "$PAUSE" -c:a pcm_f32le "$TMP/gap/s$n.wav"
  PARTS+=("$n"); n=$((n + 1))
done < <(tail -n +2 "$TMP/gap/cuts.txt")
INPUTS=(); FILTER=""; k=0
for p in "${PARTS[@]}"; do
  INPUTS+=(-i "$TMP/gap/p$p.wav"); FILTER+="[$k:a]"; k=$((k + 1))
  if [ "$p" -lt $((n - 1)) ]; then INPUTS+=(-i "$TMP/gap/s$p.wav"); FILTER+="[$k:a]"; k=$((k + 1)); fi
done
ffmpeg -y -v error "${INPUTS[@]}" -filter_complex "${FILTER}concat=n=$k:v=0:a=1[o]" -map "[o]" -c:a pcm_f32le "$TMP/gap/voice.wav"
rm -f "$TMP"/gap/p*.wav "$TMP"/gap/s*.wav
make_levels "$TMP/gap" "$TMP/gap/voice.wav" "$TMP/gap/speech.json"

# --- a peaky narration (third scenario) --------------------------------------
# The base voice plus a plosive-like burst at every third word onset. The burst
# amplitude is set (three corrections) so peak - integrated loudness through the
# mix's own format chain is PEAKY_PLR dB; then the voice is levelled like the
# others. The words, so the video and the audio plans, are the base ones.
mkdir -p "$TMP/peaky"
PEAKY_PLR=${MIX_BENCH_PEAKY_PLR:-20}
python3 - "$BASE_SPEECH" "$TMP/peaky/speech.json" "$TMP/peaky/times.txt" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["audio"] = "voice.wav"
json.dump(m, open(sys.argv[2], "w"))
open(sys.argv[3], "w").write(" ".join(str(w["start"]) for w in m["words"][::3]) + "\n")
PY
peaky_voice() { # burst_amplitude -> $TMP/peaky/voice.wav (float, mono, 48 kHz)
  local amp=$1 expr=0 t
  for t in $(cat "$TMP/peaky/times.txt"); do
    expr+="+$amp*cos(2*PI*150*(t-$t))*exp(-pow((t-$t)/0.002,2))"
  done
  ffmpeg -y -v error -i "$BASE_VOICE" -f lavfi -i "aevalsrc='$expr':s=48000" \
    -filter_complex "[0:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=mono[a];[1:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=mono[b];[a][b]amix=inputs=2:normalize=0:duration=first" \
    -c:a pcm_f32le "$TMP/peaky/voice.wav"
}
plr() { # file -> true peak - integrated loudness (dB) through the mix's format chain
  ffmpeg -nostats -hide_banner -i "$1" -af "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,ebur128=peak=true" -f null - 2>&1 \
    | awk '/Summary:/{s=1} s && /^ +I:/{i=$2} s && /Peak:/{p=$2} END{printf "%.3f\n", p - i}'
}
amp=0.5
for _ in 1 2 3; do
  peaky_voice "$amp"
  amp=$(python3 -c "print(round($amp * 10 ** (($PEAKY_PLR - $(plr "$TMP/peaky/voice.wav")) / 20), 5))")
done
peaky_voice "$amp"
echo "== peaky voice: burst amplitude $amp, peak-to-loudness $(plr "$TMP/peaky/voice.wav") dB (target $PEAKY_PLR)"
make_levels "$TMP/peaky" "$TMP/peaky/voice.wav" "$TMP/peaky/speech.json"

# --- compile, plan every bed, render the video once per scenario -------------
prepare() { # scenario_dir speech_json
  local dir=$1 speech=$2
  "$E" compile "$STORY" ${STYLE[@]+"${STYLE[@]}"} --speech "$speech" --canvas 360x640 --output "$dir/scene.motion.json" >"$dir/compile.log" 2>&1 \
    || { cat "$dir/compile.log" >&2; exit 1; }
  "$E" render "$dir/scene.motion.json" --out-dir "$dir/render" >/dev/null
  while read -r id plan; do
    "$E" plan-audio "$dir/scene.motion.json" --intent "$STORY" ${STYLE[@]+"${STYLE[@]}"} --sfx-library "$SFX" \
      --speech "$speech" --music "$MUSIC/$plan" -o "$dir/$id.audio.json" >/dev/null
    # The same plan without `levels`: what `qa --speech` rebuilds the legacy mix from.
    python3 - "$dir/$id.audio.json" "$dir/$id.legacy.audio.json" <<'PY'
import json, sys
plan = json.load(open(sys.argv[1]))
plan.pop("levels", None)
json.dump(plan, open(sys.argv[2], "w"))
PY
  done <<< "$BEDS"
}
echo "== compile, plan-audio, render (base)"
prepare "$TMP/base" "$BASE_SPEECH"
echo "== compile, plan-audio, render (long pauses)"
prepare "$TMP/gap" "$TMP/gap/speech.json"
# The peaky narration has the base words: the same scene, video and audio plans.
cp "$TMP"/base/scene.motion.json "$TMP"/base/*.audio.json "$TMP/peaky/"
ln -s "$TMP/base/render" "$TMP/peaky/render"

# --- one case: bed x voice level x sfx, new and legacy -----------------------
run_case() { # scenario bed plan level sfx(on|off)
  local sc=$1 id=$2 plan=$3 lv=$4 sfx=$5
  local dir=$TMP/$sc key=$sc.$id.$lv.$sfx
  local video=$dir/render/sleep_review.mp4
  local speech=$dir/lv$lv/speech.json
  local sfxarg=()
  [ "$sfx" = on ] && sfxarg=(--sfx-library "$SFX")
  for mode in new legacy; do
    local legacy=() qaplan=$dir/$id.audio.json
    if [ "$mode" = legacy ]; then legacy=(--legacy); qaplan=$dir/$id.legacy.audio.json; fi
    local stems=$TMP/stems.$key.$mode mp4=$TMP/mix.$key.$mode.mp4 err=$TMP/runs/$key.$mode.err
    "$E" mix "$video" --scene "$dir/scene.motion.json" --audio-plan "$dir/$id.audio.json" --speech "$speech" --music "$MUSIC/$plan" \
      ${sfxarg[@]+"${sfxarg[@]}"} ${legacy[@]+"${legacy[@]}"} --stems "$stems" -o "$mp4" >/dev/null 2>"$err"
    # The mix log's long-gap cap line (voice-relative mixes with a long narration gap).
    grep '^bed gap cap:' "$err" > "$TMP/runs/$key.$mode.cap.txt" || true
    # The voice limiter's line (a peaky voice).
    grep '^voice limiter:' "$err" > "$TMP/runs/$key.$mode.lim.txt" || true
    # The Rust checks: QA rebuilds its own stems (in a temp dir it removes) from the plan.
    "$E" qa "$dir/scene.motion.json" --speech "$speech" --audio-plan "$qaplan" --json > "$TMP/runs/$key.$mode.qa.json"
    python3 "$TOOLS/stem_levels.py" --stems "$stems" --speech "$speech" --compare "$TMP/runs/$key.$mode.qa.json" \
      > "$TMP/runs/$key.$mode.stems.json"
    ffmpeg -nostats -hide_banner -i "$mp4" -af ebur128=peak=true -f null - 2>&1 \
      | awk '/Summary:/{s=1} s && /^ +I:/{i=$2} s && /Peak:/{p=$2} END{print i, p}' > "$TMP/runs/$key.$mode.final.txt"
    # The stems summed and raised by the makeup gain the mix logged: the shipped mix.
    local gain n=2 ins=(-i "$stems/voice.wav" -i "$stems/bed.wav") labels='[0:a][1:a]'
    gain=$(awk '/^makeup gain:/{g=$3; sub(/^\+/, "", g); print g}' "$err")
    if [ -f "$stems/sfx.wav" ]; then n=3; ins+=(-i "$stems/sfx.wav"); labels='[0:a][1:a][2:a]'; fi
    ffmpeg -nostats -hide_banner "${ins[@]}" -filter_complex "${labels}amix=inputs=$n:normalize=0:duration=longest,volume=${gain:-0}dB,ebur128=peak=true" -f null - 2>&1 \
      | awk '/Summary:/{s=1} s && /^ +I:/{i=$2} END{print i}' > "$TMP/runs/$key.$mode.sum.txt"
    rm -rf "$stems" "$mp4"
  done
}
export -f run_case
export E TMP TOOLS MUSIC SFX

echo "== mixing ($JOBS parallel)"
{
  while read -r id plan; do
    for lv in 14 20 26; do
      for sfx in on off; do echo "base $id $plan $lv $sfx"; done
      echo "gap $id $plan $lv on"
      echo "peaky $id $plan $lv on"
    done
  done <<< "$BEDS"
} | xargs -P "$JOBS" -L 1 bash -c 'set -e; run_case "$@"' _

# --- report -------------------------------------------------------------------
status=0
python3 - "$TMP/runs" "$OUT" 84 <<'PY' || status=$?
import glob, json, os, re, sys
runs_dir, out, expected = sys.argv[1], sys.argv[2], int(sys.argv[3])
PAUSE = os.environ.get("MIX_BENCH_PAUSE", "1.2")
THRESH = dict(voice_over_music=15.0, bed_over_voice=-6.0, sfx_over_voice=-6.0, lufs=(-17.0, -15.0), tp=-1.0, voice_lufs=-18.0)
rows = {}
for f in sorted(glob.glob(os.path.join(runs_dir, "*.stems.json"))):
    key, mode = os.path.basename(f)[:-len(".stems.json")].rsplit(".", 1)
    sc, bed, lv, sfx = key.split(".", 3)
    stems = json.load(open(f))
    final = open(os.path.join(runs_dir, f"{key}.{mode}.final.txt")).read().split()
    stems["final_lufs"] = float(final[0])
    stems["final_true_peak"] = float(final[1])
    stems["sum_lufs"] = float(open(os.path.join(runs_dir, f"{key}.{mode}.sum.txt")).read().split()[0])
    cap = re.search(r"bed gap cap: ([+-][\d.]+) dB \(worst bed momentary in long gaps ([-\d.]+) -> ([-\d.]+) dB",
                    open(os.path.join(runs_dir, f"{key}.{mode}.cap.txt")).read())
    stems["gap_cap"] = None if not cap else dict(lowered_db=float(cap.group(1)), before_db=float(cap.group(2)), after_db=float(cap.group(3)))
    lim = re.search(r"voice limiter: ([+-][\d.]+) dB static gain, .* up to ([\d.]+) dB peak reduction \(voice I ([-\d.]+) -> ([-\d.]+) LUFS",
                    open(os.path.join(runs_dir, f"{key}.{mode}.lim.txt")).read())
    stems["limiter"] = None if not lim else dict(gain_db=float(lim.group(1)), reduction_db=float(lim.group(2)), before_lufs=float(lim.group(3)), after_lufs=float(lim.group(4)))
    rows.setdefault((sc, bed, int(lv), sfx), {})[mode] = stems

def failures(m, sfx):
    bad = []
    if m["voice_over_music"] is None or m["voice_over_music"] < THRESH["voice_over_music"]: bad.append("voice_over_music")
    if m["bed_over_voice"] is not None and m["bed_over_voice"] > THRESH["bed_over_voice"]: bad.append("bed_over_voice")
    if not (THRESH["lufs"][0] <= m["final_lufs"] <= THRESH["lufs"][1]): bad.append("final_lufs")
    if m["final_true_peak"] > THRESH["tp"]: bad.append("true_peak")
    if m["voice_integrated_lufs"] is None or abs(m["voice_integrated_lufs"] - THRESH["voice_lufs"]) > 1.0: bad.append("voice_lufs")
    if sfx == "on" and (m["sfx_over_voice"] is None or m["sfx_over_voice"] > THRESH["sfx_over_voice"]): bad.append("sfx_over_voice")
    return bad

keep = ["voice_over_music", "bed_over_voice", "speech_band", "sfx_over_voice", "final_lufs", "final_true_peak",
        "voice_integrated_lufs", "bed_voiced_lufs", "bed_gap_momentary_max_lufs", "gaps", "sum_lufs"]
records = []
for (sc, bed, lv, sfx), modes in sorted(rows.items(), key=lambda kv: (kv[0][0], kv[0][1], -kv[0][2], kv[0][3])):
    rec = dict(scenario=sc, bed=bed, voice_lufs=-lv, sfx=sfx == "on")
    for mode in ("new", "legacy"):
        rec[mode] = {k: modes[mode].get(k) for k in keep}
        cmp = modes[mode].get("compare") or {}
        rec[mode]["rust"] = cmp.get("rust", {})
        rec[mode]["rust_minus_python"] = cmp.get("diff", {})
        rec[mode]["unpaired"] = cmp.get("unpaired", [])
        rec[mode]["gap_cap"] = modes[mode].get("gap_cap")
        rec[mode]["limiter"] = modes[mode].get("limiter")
        # What the stems summed with the logged makeup gain add up to, against the shipped mix.
        rec[mode]["sum_minus_final"] = round(modes[mode]["sum_lufs"] - modes[mode]["final_lufs"], 2)
    rec["new_failures"] = failures(modes["new"], sfx)
    records.append(rec)

def worst(runs, mode, field="rust_minus_python"):
    # The largest |Rust - Python| per measure over `runs`: (value, the run), plus unpaired runs.
    res = {}
    for k in ("voice_over_music", "bed_over_voice", "speech_band"):
        best = None
        for r in runs:
            d = r[mode][field].get(k)
            if d is not None and (best is None or abs(d) > abs(best[0])):
                best = (d, f"{r['scenario']} {r['bed']} {r['voice_lufs']} sfx {'on' if r['sfx'] else 'off'}")
        res[k] = best
    return res

AGREE = 0.5
agree_new = worst(records, "new")
legacy_off = [r for r in records if not r["sfx"]]
legacy_on = [r for r in records if r["sfx"]]
agree_off = worst(legacy_off, "legacy")
agree_on = worst(legacy_on, "legacy")
unpaired_new = [r for r in records if r["new"]["unpaired"]]
worst_new = max([abs(v[0]) for v in agree_new.values() if v] or [0.0])
sum_new = max(records, key=lambda r: abs(r["new"]["sum_minus_final"]))
sum_legacy = max(records, key=lambda r: abs(r["legacy"]["sum_minus_final"]))
json.dump(dict(thresholds=THRESH, rust_python_agreement=dict(tolerance=AGREE, worst_new=worst_new), runs=records),
          open(os.path.join(out, "report.json"), "w"), indent=1)

fmt = lambda v, p=1: "n/a" if v is None else f"{v:.{p}f}"
L = ["# Mix bench (0.23 W5a)", "",
     "voice_over_music >= 15 dB, bed_over_voice <= -6 dB, final -16 +- 1 LUFS and <= -1 dBTP, "
     "sfx_over_voice <= -6 dB, voice stem -18 +- 1 LUFS. VoM = voice over music while speaking; BoV = bed over voice in narration gaps; "
     "SFX = SFX over voice while speaking (dB). Scenario base = the synthesised narration, gap = the same narration with " + PAUSE + " s pauses inserted between the sentences, "
     "peaky = the same narration with plosive-like peaks (peak-to-loudness 20 dB), which goes through the voice limiter.", ""]
for sc, title in (("base", "Base narration"), ("gap", "Narration with " + PAUSE + " s pauses (SFX on)"), ("peaky", "Peaky narration (SFX on)")):
    L += [f"## {title}", "",
          "| bed | voice | sfx | VoM new | VoM legacy | BoV new | BoV legacy | SFX new | SFX legacy | band new | LUFS new | dBTP new | voice LUFS new | limiter dB | LUFS legacy | Rust VoM | Rust BoV | Rust band | result |",
          "|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in (r for r in records if r["scenario"] == sc):
        n, o = r["new"], r["legacy"]
        rust = n["rust"]
        L.append(f"| {r['bed']} | {r['voice_lufs']} | {'on' if r['sfx'] else 'off'} | {fmt(n['voice_over_music'])} | {fmt(o['voice_over_music'])} "
                 f"| {fmt(n['bed_over_voice'])} | {fmt(o['bed_over_voice'])} | {fmt(n['sfx_over_voice'])} | {fmt(o['sfx_over_voice'])} "
                 f"| {fmt(n['speech_band'])} | {fmt(n['final_lufs'])} | {fmt(n['final_true_peak'])} | {fmt(n['voice_integrated_lufs'])} "
                 f"| {fmt(n['limiter']['reduction_db']) if n['limiter'] else '-'} | {fmt(o['final_lufs'])} "
                 f"| {fmt(rust.get('voice_over_music'))} | {fmt(rust.get('bed_over_voice'))} | {fmt(rust.get('speech_band'))} "
                 f"| {'PASS' if not r['new_failures'] else 'FAIL ' + ','.join(r['new_failures'])} |")
    L.append("")
L += ["## Per bed: legacy baseline range vs voice-relative range (all runs of both scenarios)", "",
      "| bed | VoM legacy | VoM new | BoV legacy | BoV new | SFX legacy | SFX new |", "|---|---|---|---|---|---|---|"]
def rng(vals):
    vals = [v for v in vals if v is not None]
    return "n/a" if not vals else f"{min(vals):.1f} .. {max(vals):.1f}"
for bed in sorted({r["bed"] for r in records}):
    rs = [r for r in records if r["bed"] == bed]
    col = lambda mode, k: rng([r[mode][k] for r in rs])
    L.append(f"| {bed} | {col('legacy', 'voice_over_music')} | {col('new', 'voice_over_music')} | {col('legacy', 'bed_over_voice')} "
             f"| {col('new', 'bed_over_voice')} | {col('legacy', 'sfx_over_voice')} | {col('new', 'sfx_over_voice')} |")
L += ["", "## Worst case per voice level (voice-relative mix)", "",
      "| voice | min VoM | max BoV | max SFX | final LUFS | max dBTP | failing runs |", "|---|---|---|---|---|---|---|"]
for lv in (-14, -20, -26):
    rs = [r for r in records if r["voice_lufs"] == lv]
    g = lambda k, f: f([r["new"][k] for r in rs if r["new"][k] is not None])
    L.append(f"| {lv} | {fmt(g('voice_over_music', min))} | {fmt(g('bed_over_voice', max))} | {fmt(g('sfx_over_voice', max))} "
             f"| {fmt(g('final_lufs', min))} .. {fmt(g('final_lufs', max))} | {fmt(g('final_true_peak', max))} | {sum(1 for r in rs if r['new_failures'])} / {len(rs)} |")
bad = [r for r in records if r["new_failures"]]
fmt_worst = lambda w: "n/a" if not w else f"{w[0]:+.2f} ({w[1]})"
capped = [r for r in records if r["new"]["gap_cap"] and r["new"]["gap_cap"]["lowered_db"] != 0.0]
metered = [r for r in records if r["new"]["gap_cap"]]
L += ["", "## Long-gap cap (voice-relative mix; the bed's worst 400 ms level in narration gaps >= 0.8 s, dB over the voice)", "",
      f"Metered in {len(metered)} of {len(records)} runs, lowered the long-gap level in {len(capped)}. "
      "Ceiling -6.5 dB (limit -6 minus 0.5 margin); the long-gap level is -10 dB before the cap.", ""]
if capped:
    L += ["| scenario | bed | voice | sfx | lowered | worst before | worst after |", "|---|---|---|---|---|---|---|"]
    for r in capped:
        c = r["new"]["gap_cap"]
        L.append(f"| {r['scenario']} | {r['bed']} | {r['voice_lufs']} | {'on' if r['sfx'] else 'off'} | {c['lowered_db']:+.2f} dB | {c['before_db']:.1f} | {c['after_db']:.1f} |")
    L.append("")
limited = [r for r in records if r["new"]["limiter"]]
L += ["", "## Voice limiter (a peaky voice: a transparent limiter on the voice stem, after the static gain)", ""]
if limited:
    lim = [r["new"] for r in limited]
    L.append(f"{len(limited)} of {len(records)} runs went through it ({', '.join(sorted({r['scenario'] for r in limited}))}): "
             f"peak reduction up to {max(m['limiter']['reduction_db'] for m in lim):.1f} dB, voice stems "
             f"{min(m['voice_integrated_lufs'] for m in lim):.1f} .. {max(m['voice_integrated_lufs'] for m in lim):.1f} LUFS, "
             f"final {min(m['final_lufs'] for m in lim):.1f} .. {max(m['final_lufs'] for m in lim):.1f} LUFS, "
             f"{max(m['final_true_peak'] for m in lim):.1f} dBTP at most.")
else:
    L.append("No run needed the voice limiter.")
L += ["", "## Rust (`qa --speech`) against Python (`stem_levels.py`)", "",
      "Worst Rust - Python difference per measure (dB); the Rust checks measure stems QA rebuilds itself.", "",
      "| runs | voice_over_music | bed_over_voice | speech_band |", "|---|---|---|---|"]
for title, w in (("voice-relative, all " + str(len(records)), agree_new),
                 ("legacy, SFX off", agree_off),
                 ("legacy, SFX on (QA does not rebuild the SFX duck of the legacy bed)", agree_on)):
    L.append(f"| {title} | {fmt_worst(w['voice_over_music'])} | {fmt_worst(w['bed_over_voice'])} | {fmt_worst(w['speech_band'])} |")
L += ["", f"Worst |Rust - Python| over the voice-relative runs: {worst_new:.2f} dB (limit {AGREE} dB); "
      f"runs where only one side had a value: {len(unpaired_new)}.", "",
      "## Stems summed with the logged makeup gain against the shipped mix (LU)", "",
      f"Voice-relative: worst sum - final {sum_new['new']['sum_minus_final']:+.2f} LU "
      f"({sum_new['scenario']} {sum_new['bed']} {sum_new['voice_lufs']} sfx {'on' if sum_new['sfx'] else 'off'}); "
      f"legacy: worst {sum_legacy['legacy']['sum_minus_final']:+.2f} LU "
      f"({sum_legacy['scenario']} {sum_legacy['bed']} {sum_legacy['voice_lufs']} sfx {'on' if sum_legacy['sfx'] else 'off'}). Limit 0.5 LU (voice-relative).", ""]
L += ["", f"{len(records)} runs, {len(bad)} failing (voice-relative mix)."]
if worst_new > AGREE or unpaired_new:
    L.append(f"RUST/PYTHON DISAGREE: worst {worst_new:.2f} dB, {len(unpaired_new)} unpaired run(s)")
    bad = bad or [None]
if abs(sum_new["new"]["sum_minus_final"]) > 0.5:
    L.append(f"STEMS DO NOT REPRODUCE THE MIX: {sum_new['new']['sum_minus_final']:+.2f} LU")
    bad = bad or [None]
if len(records) != expected:
    L.append(f"INCOMPLETE: expected {expected} runs")
    bad = bad or [None]
open(os.path.join(out, "report.md"), "w").write("\n".join(L) + "\n")
print("\n".join(L))
sys.exit(1 if bad else 0)
PY

# --- no stem or temp wav may survive ------------------------------------------
rm -rf "$TMP"
left=$(find "$OUT" "$TMPBASE" -path '*mix_bench*' -name '*.wav' -newer "$MARK" 2>/dev/null | wc -l | tr -d ' ')
echo "leftover wav files from this bench: $left"
echo "mix_bench runtime: ${SECONDS}s"
printf '\nBench runtime: %ss. Leftover wav files: %s.\n' "$SECONDS" "$left" >> "$OUT/report.md"
[ "$left" = 0 ] || exit 1
exit "$status"
