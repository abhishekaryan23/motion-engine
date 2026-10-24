#!/usr/bin/env python3
"""Stem-level measurements of a voice-relative mix (sprint 0.23, W5a).

    stem_levels.py --stems DIR --speech SPEECH.json [--offset SECONDS]
                   [--compare QA.json [--tolerance DB]]

DIR holds `voice.wav`, `bed.wav` and optionally `sfx.wav` (what
`motion-engine mix --stems DIR` writes: post-gain, post-envelope, pre-makeup,
48 kHz stereo float). SPEECH.json is the SpeechMap; `--offset` is the voice's
start on the project timeline (default 0). Prints one JSON object (dB):

  voice_over_music  voice loudness - bed loudness over the voiced segments.
                    Both are K-weighted (BS.1770) energy means over the same
                    samples, without gating.
  bed_over_voice    max bed momentary loudness inside the narration gaps
                    (>= 0.3 s, between the first and the last word) - the
                    voice's integrated loudness (ffmpeg ebur128, gated).
  speech_band       bed 300-4000 Hz level - voice 300-4000 Hz level over the
                    voiced segments (plain energy, no K-weighting).
  sfx_over_voice    max SFX momentary loudness inside the voiced segments -
                    the voice's integrated loudness (null without an sfx stem).

With `--compare QA.json` (the output of `motion-engine qa --speech ... --json`,
whose stem checks are measured in Rust by motion_render::mix_qa on stems it
rebuilds itself) the object also gets a `compare` entry: the Rust values
(`voice_over_music_db`, `bed_over_voice_db`, `speech_band_masking_db`), the
Rust - Python difference per measure, `max_abs_diff` and `within_tolerance`
(every difference <= `--tolerance`, default 0.5 dB, and no value missing on
one side only). The exit status is not affected.

Voiced segments: word spans merged across gaps shorter than 0.3 s. Momentary
loudness: 400 ms windows on a 100 ms hop; a window is "inside" a segment when
its centre is. Needs ffmpeg and numpy only; no file is written.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

import numpy as np

SR = 48000
MERGE_GAP = 0.3  # s: word gaps shorter than this stay inside one voiced segment
MOMENTARY = 0.4  # s
HOP = 0.1  # s

# BS.1770-4 K-weighting at 48 kHz: high-shelf pre-filter, then the RLB high-pass.
K_WEIGHTING = (
    "biquad=b0=1.53512485958697:b1=-2.69169618940638:b2=1.19839281085285:"
    "a0=1:a1=-1.69065929318241:a2=0.73248077421585,"
    "biquad=b0=1:b1=-2:b2=1:a0=1:a1=-1.99004745483398:a2=0.99007225036621"
)
SPEECH_BAND = "highpass=f=300:poles=2,lowpass=f=4000:poles=2"


def decode(path, chain):
    """The file as float32 (samples, 2) at 48 kHz, through the filter `chain`."""
    af = f"aformat=sample_fmts=fltp:sample_rates={SR}:channel_layouts=stereo,{chain}"
    out = subprocess.run(
        ["ffmpeg", "-v", "error", "-nostdin", "-i", str(path), "-af", af,
         "-f", "f32le", "-ac", "2", "-ar", str(SR), "-"],
        capture_output=True,
    )
    if out.returncode != 0:
        sys.exit(f"ffmpeg failed on {path}: {out.stderr.decode(errors='replace').strip()}")
    return np.frombuffer(out.stdout, dtype="<f4").reshape(-1, 2).astype(np.float64)


def integrated_lufs(path):
    """Gated integrated loudness of the file (ffmpeg ebur128)."""
    out = subprocess.run(
        ["ffmpeg", "-nostats", "-hide_banner", "-nostdin", "-i", str(path),
         "-af", "ebur128", "-f", "null", "-"],
        capture_output=True, text=True,
    )
    summary = out.stderr[out.stderr.rfind("Summary:"):]
    m = re.search(r"I:\s+(-?[\d.]+|-inf)\s+LUFS", summary)
    if not m or m.group(1) == "-inf":
        return None
    return float(m.group(1))


def voiced_segments(words, offset):
    """Word spans merged across gaps shorter than MERGE_GAP, on the project timeline."""
    spans = sorted((w["start"] + offset, w["end"] + offset) for w in words)
    merged = []
    for a, b in spans:
        if merged and a - merged[-1][1] < MERGE_GAP:
            merged[-1][1] = max(merged[-1][1], b)
        else:
            merged.append([a, b])
    return [(a, b) for a, b in merged]


def gaps_between(segments):
    """The narration gaps (>= MERGE_GAP) between consecutive voiced segments."""
    return [(segments[i][1], segments[i + 1][0]) for i in range(len(segments) - 1)]


def energy_db(x, segments, offset_db=0.0):
    """10 log10 of the mean per-sample power (summed over channels) inside segments."""
    parts = [x[int(a * SR):int(b * SR)] for a, b in segments]
    parts = [p for p in parts if len(p)]
    if not parts:
        return None
    sq = np.concatenate([p for p in parts])
    power = float(np.mean(np.sum(sq * sq, axis=1)))
    if power <= 0.0:
        return None
    return offset_db + 10.0 * np.log10(power)


def momentary(x, segments):
    """Max momentary loudness (LUFS, K-weighted x) over the windows whose centre is in a segment.

    Windows are ffmpeg ebur128's: 400 ms ending on the 100 ms grid (centres
    0.2, 0.3, 0.4 ... s), full windows only.
    """
    power = np.sum(x * x, axis=1)
    cs = np.concatenate([[0.0], np.cumsum(power)])
    n = len(power)
    half = int(round(MOMENTARY / 2 * SR))
    values = []
    k = 0
    while True:
        c = MOMENTARY / 2 + k * HOP
        i, j = int(round(c * SR)) - half, int(round(c * SR)) + half
        if j > n:
            break
        if any(a - 1e-6 <= c <= b + 1e-6 for a, b in segments):
            values.append(-0.691 + 10.0 * np.log10(max((cs[j] - cs[i]) / (j - i), 1e-12)))
        k += 1
    return max(values) if values else None


# The report fields of `qa --speech --json` that measure the same three things.
RUST_FIELDS = {
    "voice_over_music": "voice_over_music_db",
    "bed_over_voice": "bed_over_voice_db",
    "speech_band": "speech_band_masking_db",
}


def compare(result, rust, tolerance):
    """The Rust values of a `qa --speech --json` report against `result`."""
    out = {"rust": {}, "diff": {}}
    unpaired = []
    for key, field in RUST_FIELDS.items():
        r, p = rust.get(field), result.get(key)
        out["rust"][key] = None if r is None else round(r, 2)
        out["diff"][key] = None if r is None or p is None else round(r - p, 2)
        if (r is None) != (p is None):
            unpaired.append(key)
    diffs = [abs(d) for d in out["diff"].values() if d is not None]
    out["max_abs_diff"] = max(diffs) if diffs else None
    out["tolerance"] = tolerance
    out["unpaired"] = unpaired
    out["within_tolerance"] = not unpaired and all(d <= tolerance for d in diffs)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--stems", required=True, help="directory with voice.wav, bed.wav, [sfx.wav]")
    ap.add_argument("--speech", required=True, help="SpeechMap JSON")
    ap.add_argument("--offset", type=float, default=0.0, help="voice start on the project timeline (s)")
    ap.add_argument("--compare", help="qa --speech --json output: add the Rust values and the differences")
    ap.add_argument("--tolerance", type=float, default=0.5, help="largest accepted |Rust - Python| (dB, default 0.5)")
    args = ap.parse_args()

    stems = Path(args.stems)
    speech = json.loads(Path(args.speech).read_text())
    words = speech.get("words", [])
    if not words:
        sys.exit("speech has no words")
    segments = voiced_segments(words, args.offset)
    gaps = gaps_between(segments)

    voice_k = decode(stems / "voice.wav", K_WEIGHTING)
    result = {
        "voiced_segments": len(segments),
        "voiced_seconds": round(sum(b - a for a, b in segments), 3),
        "gaps": len(gaps),
    }
    voice_i = integrated_lufs(stems / "voice.wav")
    result["voice_integrated_lufs"] = None if voice_i is None else round(voice_i, 2)
    v_loud = energy_db(voice_k, segments, -0.691)
    result["voice_voiced_lufs"] = None if v_loud is None else round(v_loud, 2)

    bed = stems / "bed.wav"
    if bed.is_file():
        bed_k = decode(bed, K_WEIGHTING)
        b_loud = energy_db(bed_k, segments, -0.691)
        result["bed_voiced_lufs"] = None if b_loud is None else round(b_loud, 2)
        result["voice_over_music"] = (
            None if v_loud is None or b_loud is None else round(v_loud - b_loud, 2)
        )
        m = momentary(bed_k, gaps) if gaps else None
        result["bed_gap_momentary_max_lufs"] = None if m is None else round(m, 2)
        result["bed_over_voice"] = (
            None if m is None or voice_i is None else round(m - voice_i, 2)
        )
        vb = energy_db(decode(stems / "voice.wav", SPEECH_BAND), segments)
        bb = energy_db(decode(bed, SPEECH_BAND), segments)
        result["speech_band"] = None if vb is None or bb is None else round(bb - vb, 2)
    else:
        result.update(voice_over_music=None, bed_over_voice=None, speech_band=None)

    sfx = stems / "sfx.wav"
    if sfx.is_file():
        m = momentary(decode(sfx, K_WEIGHTING), segments)
        result["sfx_voiced_momentary_max_lufs"] = None if m is None else round(m, 2)
        result["sfx_over_voice"] = None if m is None or voice_i is None else round(m - voice_i, 2)
    else:
        result["sfx_over_voice"] = None
    if args.compare:
        result["compare"] = compare(result, json.loads(Path(args.compare).read_text()), args.tolerance)
    print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
