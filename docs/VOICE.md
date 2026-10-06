# Voice-led reels (0.10)

A voice-over is an **operator** input beside the intent, like music or SFX.
Weak models still write only CreativeIntent + the five taste fields; the
engine speaks each beat's `narration` (else its `statement`), picks the voice
from the emotion the taste evokes, and times beats, captions and choreography
to the speech.

```
motion-engine voice  story.intent.json --style s.json [--tts-model auto|fish|say|gemini|mai|deepgram] [--voice ID] [--align auto|local|onset|asr] -o out/
motion-engine compile story.intent.json --style s.json --speech out/story.speech.json -o out/story.motion.json
motion-engine render  out/story.motion.json --speech out/story.speech.json [--music bed.music.json] [--sfx-library DIR]
motion-engine qa      out/story.motion.json --speech out/story.speech.json
```

## Provider (B0 verification, 2026-10-01)
OpenRouter exposes TTS at `POST https://openrouter.ai/api/v1/audio/speech`
(OpenAI-compatible): body `{model, input, voice?, response_format: "mp3"|"pcm", speed?}`,
`Authorization: Bearer <key>`. The response is a raw audio byte stream
(`audio/mpeg` for mp3), errors are JSON `{"error":{"message","code"}}`.
**No word timestamps are returned.**

| `--tts-model` | OpenRouter id | voices |
|---|---|---|
| `deepgram` | `deepgram/flux-tts:free` | 36 English `flux-*-en` voices (models API `supported_voices`) |
| `fish` | `fish-audio/s2.1-pro-free:free` | no OpenRouter catalog; `voice` is a Fish voice-library model id (0.18: always pinned, see below) |
| `say` | — (macOS `say`, offline) | system voices |

Voice ids were read from `GET /api/v1/models?output_modalities=speech`.
B0 live calls ("Every number tells a story.", mp3): Deepgram Flux
`flux-hannah-en` → HTTP 200, `audio/mpeg`, 24 kHz mono, 1.82 s; Fish S2.1 Pro
Free (no voice) → HTTP 200, `audio/mpeg`, 44.1 kHz mono, 2.06 s. Only
`X-Generation-Id` comes back; no timestamps in body or headers.

## Keys
Resolution: env `OPENROUTER_API_KEY` → `~/.config/motionengine/providers.toml`
(`[openrouter] api_key = "..."`, or a top-level `OPENROUTER_API_KEY = "..."`;
directory 0700, file 0600 — `providers show` warns when the mode is wider). The key never
enters the repo, scenes, speech files, logs or fixtures.

```
printf %s "$OPENROUTER_API_KEY" | motion-engine providers set-key openrouter   # also: rotate an expired key
motion-engine providers show        # provider, masked key (sk-or-…abcd), source, config path
motion-engine providers test openrouter
```

## Voice choice (engine-owned, owner rules 2026-10-02)
Owner rules: **free OpenRouter TTS only, one voice, one take, a continuous
script.** A reel whose beats were read in separate calls sounded like a
different speaker per beat; Hinglish narration is dropped.

`--tts-model auto` (default) is **Fish S2.1 Pro free**
(`fish-audio/s2.1-pro-free:free`), the only free TTS model OpenRouter lists.
OpenRouter lists no voices for it, but passes `voice` to Fish as a
voice-library model id. **(0.18, owner rule: male narrators only.)** Without
an id Fish picks a different speaker on every request — our renders measured
82–221 Hz median pitch, some of them female — so the engine always pins one of
three public, generic male narrators (no celebrity or character clones),
checked with a pitch test on the free model: **Ethan**
`536d3a5e000945adb7038665781a4aca` (a curious explainer, ≈107 Hz), **Slax**
`c5f56a6cc2ec4fa8920cb4c5889a3fb7` (clear, precise, ≈107 Hz) and **calm
storyteller male** `e686ae649ee44f219a108aacba206c1a` (deep documentary,
≈92 Hz). Every other model's table is male too. `say`
(macOS, offline) and the paid models `gemini`, `mai` and `deepgram` are
explicit opt-ins only; a failed paid opt-in falls back to Fish for the
**whole take**. `--voice` overrides the provider voice id.

| emotions | narrator | fish | say | gemini (opt-in) |
|---|---|---|---|---|
| trust · calm · warmth · handmade | calm narrator | Ethan | Reed | Achird |
| joy · energy · urgency · playful_retro | bright | Ethan | Eddy | Puck |
| drama · luxury | low & slow | calm storyteller male | Daniel | Orus |
| precision | clear | Slax | Reed | Charon |

## Synthesis and cache
**Always one take.** Every beat's line (its `narration`, else its
`statement`) joins one paragraph that is synthesised in ONE call, so
intonation flows across beats and the voice never changes. The take is split
back into one sentence per beat at the narrator's own pauses (runs ≥ 120 ms
below peak −35 dB; a DP picks the N−1 pauses closest to the syllable-weighted
expectation, preferring longer pauses). When the take has too few clear
pauses, each boundary is cut at the quietest 80 ms within ±35 % of an average
line of its expected time (`quiet_cuts`). The take is **never re-synthesised
per sentence**. Sentences are re-laid with their natural pause (0.22–0.65 s),
extended by at most 1.0 s to keep sentence starts ≥ 0.9× the planned visual
beat spacing.

Audio is normalised to 48 kHz mono s16 WAV and cached at
`<cache>/<sha256(model ‖ voice ‖ rate ‖ text)>.wav` (default
`~/.cache/motionengine/voice/`, `$MOTION_VOICE_CACHE`, `--cache`). Reruns make
zero network calls and produce byte-identical files; `--offline` turns a miss
into an error.

**Word timing** (`--align auto`, default since 0.20: `local` when the binary
has the recogniser and the model is installed, else `onset` with one line
saying why). `--align onset` (free): words are placed inside their
sentence by a syllable-weighted estimate snapped to onsets of the 10 ms RMS
envelope (≈ 130 ms mean error). `--align local` (free, on-device, 0.20): see
"Local word timing" below. `--align asr` (paid opt-in) transcribes the
final WAV once with Deepgram Nova-3 through OpenRouter
(`/audio/transcriptions`, cached at `<cache>/asr/<sha256(model ‖ language ‖
audio)>.asr.json`); recognised words replace the estimates where they align.
With `local` or `asr` the take is also cut between beats at word level: each
cut is the centre of the quietest 40 ms between a line's last recognised word
and the next line's first (`snap_cuts_to_silence`), so a cut never clips a
syllable; the narrator's pauses and `quiet_cuts` remain the fallback.

## Local word timing (0.20)
whisper.cpp through `whisper-rs` (MIT), on-device and free: Metal on Apple
Silicon, CPU elsewhere. It needs a binary built with the cargo feature
`asr-local` (whisper.cpp needs cmake and a C++ toolchain) and a model fetched
once; nothing downloads silently.

```
cargo build --release -p motion-cli --features asr-local
motion-engine models list                      # known models, sizes, installed?
motion-engine models fetch whisper-base.en     # pinned URL + SHA256, atomic write
motion-engine models path whisper-base.en
motion-engine voice story.intent.json --style s.json --align local [--asr-model whisper-small.en] [--asr-beam 5] -o out/
motion-engine reel  story.intent.json --style s.json            # --align auto: local when available
```
Models live in `$MOTION_MODELS_DIR` or `~/.cache/motionengine/models/`
(ggml weights from the whisper.cpp repository on Hugging Face, MIT):

| `--asr-model` | size | sha256 | licence | note |
|---|---:|---|---|---|
| `whisper-tiny.en` | 74 MiB | `921e4cf8…` | MIT | fastest; one fixture decoded to 5 words (degenerate), not recommended |
| `whisper-base.en` | 141 MiB | `a03779c8…` | MIT | default |
| `whisper-small.en` | 465 MiB | `c6138d6d…` | MIT | most accurate, about 2.5× slower |

`auto` falls back to onset (with the `models fetch` command) when the binary
lacks the feature or the model is missing; `--align local` then fails instead
(`Unsupported` / `ModelMissing`, naming the command). `--offline` blocks
provider calls only (TTS and the paid `asr`): whisper and the CTC aligner run
on-device, so a local cache miss under `--offline` just runs them. The paid
`asr` stays an explicit opt-in.

**Decode.** English, no translation, temperature 0 without fallback, greedy
(`--asr-beam 1`, default) or beam search, no context carried between
whisper's 30 s windows (long takes are decoded whole; times are absolute),
DTW token timestamps with the model's alignment-heads preset, `max_len = 1`
with `split_on_word` (one segment per word). Sub-word pieces merge into whole
words ("$381", "sun-lit"); a `%` token becomes the word "percent".
whisper.cpp's logging is silenced.

**Cache.** `<cache>/asr/<sha256(model sha256 ‖ 0 ‖ params ‖ 0 ‖ wav bytes)>.local.json`
with params `lang=en;beam=<n>;dtw=1;words=<version>` (threads excluded:
they do not change the result; the words version changes whenever word
building does). Same audio + model + params → the same JSON; reruns are
offline and byte-identical.

**What was said.** The recogniser's words are kept in
`SpeechMap.recognised` and the method in `SpeechMap.alignment`
(`onset` | `asr` | `local:<model>:beam<n>`). After matching, `voice` prints
one line: how many script words took measured times and what was heard that
the script lacks (`beat N: ~word`). The take is never re-synthesised (one-take
rule); the operator decides. Matching treats a one-word number written as
digits or as a word as equal ("8" / "eight").

**Timestamp source (measured).** A token's DTW mark lies inside its own audio,
about where the next token begins. Against the ground-truth fixtures (base.en,
beam 1, 274 words; |Δstart| ms):

| source | mean | p95 | max | bias |
|---|---:|---:|---:|---:|
| whisper's segment times (heuristic token timestamps) | 207 | 574 | 1097 | +45 |
| the word's first token DTW mark | 206 | 426 | 1470 | +204 |
| the previous token's DTW mark | 129 | 336 | 1096 | −113 |
| **previous token's mark, moved past a pause (shipped)** | **120** | **336** | **976** | −108 |

A word starts at the previous text token's mark (moved to the end of a
≥ 80 ms silence that lies before the word's own mark: a word never starts in a
pause) and ends at the mark of its last alphanumeric token. A mark more than
1.5 s outside its segment (a degenerate decode) falls back to the segment
times. No offset is applied; the early bias is the benign direction.

**Measured (2026-10-03, Apple Silicon, Metal; `cargo test --release -p
motion-voice --features asr-local --test timing_benchmark -- --ignored
--nocapture`; 5 fixtures, 118.8 s; `_cached` = the same call served from the
cache).**

| arm | matched % | mean abs Δstart ms | p95 ms | max ms | wall ms per 30 s audio |
|---|---:|---:|---:|---:|---:|
| onset (whole clip as one sentence) | 100.0 | 562.1 | 1813.2 | 2507.2 | 1 |
| local_base_beam1 | 96.0 | 120.0 | 335.7 | 975.8 | 614–706 |
| local_base_beam1_cached | 96.0 | 120.0 | 335.7 | 975.8 | 6–9 |
| local_base_beam5 | 96.7 | 119.8 | 335.7 | 975.8 | 779–1110 |
| local_small_beam1 | 97.1 | 74.8 | 207.4 | 815.8 | 1311–1757 |
| local_tiny_beam1 | 78.8 | 133.0 | 397.7 | 817.7 | 430–836 |

Targets (mean ≤ 40, p95 ≤ 90, max ≤ 150 ms, matched ≥ 97 %): every arm
fails the timing targets; per-word jitter (sd ≈ 115–137 ms) dominates, a
constant offset would bring base.en to mean 86 / p95 258 ms only. Matched %
counts the benchmark's own spelling match: whisper writes "8", "3" and "$381"
where the script says "eight", "three" and "three hundred and eighty one
dollars" (the engine's matcher accepts the first two). The first decode after
a new build pays about 6 s once for Metal shader compilation. On a real Fish
take (five_wait_what, 102.4 s) base.en matched 248/250 script words in 1.85 s
(cached: 0.02 s) and showed the old pause-based split had put beat boundaries
up to 6 s off; word-level cuts fixed that.

### CTC refinement (0.20 A3)
whisper's DTW marks miss the timing targets, so a second, on-device pass
re-times whisper's words on a 20 ms grid: whisper still supplies WHAT was
said, a wav2vec2 CTC acoustic model WHEN (`motion_voice::asr_ctc`). It needs a
build with the cargo feature `asr-ctc` (the `ort` crate, ONNX Runtime) and
the model, fetched once; the release CLI is built with both features.

```
cargo build --release -p motion-cli --features asr-local,asr-ctc
motion-engine models fetch wav2vec2-base-960h  # pinned URLs + SHA256, atomic writes
motion-engine voice story.intent.json --style s.json [--asr-ctc auto|on|off] -o out/
motion-engine reel  story.intent.json --style s.json [--asr-ctc auto|on|off]
```
`ort`'s default `download-binaries` feature fetches a prebuilt ONNX Runtime
(1.28, static) from pyke's CDN **at build time**: a build dependency like any
crate. At runtime nothing downloads except `models fetch`, and ONNX Runtime's
logging is off. `--asr-ctc auto` (default) is on when the binary has the
feature and the model is installed, else whisper's times with one line naming
`models fetch wav2vec2-base-960h`; `on` fails instead (`Unsupported` /
`ModelMissing`); `off` keeps whisper's times. It only applies to local word
timing; `models list` shows it with kind `ctc` (whisper models: `whisper`),
`models path` prints its directory `<models root>/wav2vec2-base-960h/`.

| model | size | sha256 | licence | note |
|---|---:|---|---|---|
| `wav2vec2-base-960h` | 91 MiB | `model_quantized.onnx` `cd5040c1…`, `vocab.json` `4178db26…` | Apache-2.0 | facebook/wav2vec2-base-960h, int8 ONNX export by Xenova |

**Alignment.** Each word whisper heard is spelled as the model's letters
(`A`–`Z`, `'`, no digits): upper-cased; digits spelled out in English
("$381" → THREE HUNDRED EIGHTY ONE, "3.5" → THREE POINT FIVE, a four-digit
number without separators read as a year, "1969" → NINETEEN SIXTY NINE,
"15th" → FIFTEENTH, "1990s" → NINETEEN NINETIES, `%` → PERCENT, `&` → AND);
currency symbols and other characters dropped; hyphens, dashes and slashes
split a word into pieces that map back to the one word. The pieces joined by
the word delimiter `|` are force-aligned to the window's log-softmax logits
with a standard CTC Viterbi (tokens interleaved with blanks, f64). A word
starts at the boundary before its first letter's first frame and ends at the
boundary after its last letter's last frame. A word with nothing spellable
keeps whisper's times. Long takes are cut into windows of at most 20 s at the
widest gaps between whisper's words (the quietest 10 ms of the gap, never
inside a word); each window is normalised (`(x − mean) / sqrt(var + 1e-7)`)
and aligned separately. A window with no path (non-finite score), no speech
(below −60 dBFS) or nothing spellable keeps whisper's times and is listed in
`asr_ctc::refine_detailed`'s report.

**Frame times (derived from the model).** The feature encoder is seven
unpadded convolutions, kernels 10,3,3,3,3,2,2 and strides 5,2,2,2,2,2,2:
hop 320 samples (20 ms), receptive field 400 samples (25 ms), so frame `t`
sees samples `[320t, 320t + 400)` and is centred at `20t + 12.5` ms. A letter
first emitted at frame `t` was not emitted at `t − 1`, so its onset lies
between the two centres: the boundary is taken at their midpoint,
`20t + 2.5` ms (offset (400 − 320) / 2 = 40 samples). Checked on the model:
`T = ⌊(N − 400) / 320⌋ + 1` output frames for N input samples, exactly.

**Measured bias: +57 ms (not calibrated).** On the ground-truth fixtures the
refined starts are late by +57 ms on average (sd 31 ms; after pauses +47 ms,
flowing speech +58 ms), while the fixtures' markers sit within 10 ms of the
acoustic onsets. The grid is right; the late start is the model's peaky CTC
emission (on average the first letter's posterior is still below 1 % 54 ms
after the onset), and it grows with slower speech (mean |Δstart| 37 and 49 ms
on the rate-0.55 voices, 60 and 72 ms on the rate-0.45 ones). Dropping the `|` delimiters (+54 ms) or rescoring with label
priors (α = 0.3–1.0: +55–56 ms) does not remove it. No constant is applied:
the model's geometry justifies only the 2.5 ms above, and a constant fitted to
these fixtures (−57 ms would give mean 25 / p95 62 / max 89 ms) would be a
calibration on the test set.

**Cache and determinism.** The local cache key gains `;ctc=1;ctcalign=<aligner
version>` (whisper-only keys are unchanged); `SpeechMap.alignment` reads
`local:<model>:beam<n>+ctc`. Refined times are rounded to whole microseconds
so the JSON cache reads back bit-identical values; the ONNX session runs with
deterministic compute, so the same audio, models and params give the same
JSON and cached reruns are offline.

**Measured (2026-10-03, Apple M4; `cargo test --release -p motion-voice
--features asr-local,asr-ctc --test timing_benchmark -- --ignored
--nocapture`; 5 fixtures, 118.8 s, |Δstart| ms; bias = mean signed Δstart;
`_cached` = served from the cache).**

| arm | matched % | mean | p95 | max | bias | wall ms per 30 s audio |
|---|---:|---:|---:|---:|---:|---:|
| local_base_beam1 (whisper only) | 96.0 | 120.0 | 335.7 | 975.8 | −105.3 | 524–714 (warm) |
| local_small_beam1 (whisper only) | 97.1 | 74.8 | 207.4 | 815.8 | −43.2 | 1325 |
| **local_base_ctc** | 96.0 | **57.5** | **108.4** | **146.3** | +56.9 | 1842 (1659–2099) |
| local_base_ctc_cached | 96.0 | 57.5 | 108.4 | 146.3 | +56.9 | 6 |
| local_small_ctc | 97.1 | 57.4 | 108.5 | 146.3 | +56.8 | 2500 (2231–2829) |
| local_small_ctc_cached | 97.1 | 57.4 | 108.5 | 146.3 | +56.8 | 6 |

Targets (mean ≤ 40, p95 ≤ 90, max ≤ 150 ms, matched ≥ 97 %): CTC meets max
(146 ms, from 976) and the speed budget (≤ 3 s per 30 s) and halves the mean
(120 → 57.5 ms), but misses mean and p95 by the emission bias above; matched
% is whisper's (CTC never changes the words). The ONNX pass costs about
1.1 s per 30 s of audio on the CPU (int8; a 20 s window ≈ 0.65–0.8 s).
small.en + CTC is no more precise than base.en + CTC: once CTC sets the
times, the recogniser only decides the words. On the real Fish take
five_wait_what (102.4 s, 8 windows, all aligned) base.en + CTC matched
248/250 script words in 5.98 s (cached 0.02 s; whisper alone 2.04 s) and moved
starts by +139 ms on average (max 807 ms); ocean_layers (23.2 s, 2 windows)
matched 57/57 in 1.53 s.

## Writing narration for one take
The narration is read as **one continuous voice-over**, so write it as one
script split across beats, not as separate captions:
- each line continues the previous one (*"But here's the twist…"*, *"So…"*,
  *"That's why…"*); never restart the topic or re-introduce the subject;
- keep one point of view and tense from the first beat to the last;
- end lines at natural breath points; a sentence may run across two beats
  (end the first part with a comma and start the next in lowercase);
- keep `statement` a short on-screen title (≤ 6 words); the long line belongs
  in `narration`.

## SpeechMap v0.1 (`<name>.speech.json`)
`{version, audio, sample_rate, duration, provider, model, voice,
words:[{text,start,end,confidence}], sentences:[{beat,start,end}]}` —
`motion_core::speech`. Sentence boundaries come from the single-take split.
Without provider timestamps, words are placed deterministically inside their
sentence: a syllable-weighted estimate snapped to onsets of the 10 ms RMS
envelope; confidence records how well the snap held. One repair pass makes
times monotonic, enforces 60 ms words, records intra-sentence gaps over 0.6 s
(word starts are acoustic truth, so nothing is shifted) and
fuzzy-aligns spoken words to the beat statement (unmatched words are reported,
never invented).

## Mix, SFX placement and speech QA (V3)
**SFX stay off word onsets.** `plan-audio … --speech FILE` (`plan_audio_with_speech`)
moves any cue whose PEAK lies within 80 ms of a word onset to the nearest time
at least 80 ms from every onset (earlier on ties), inside its scene and its own
lifecycle window (information cues: `[S+evolve, S+anticipate)`; open/impact:
`[S, S+anticipate)`; handoffs: the scene) and `min_spacing` clear of the other
transient cues. A cue with nowhere to go is dropped. Every move or drop is
recorded in the plan's `speech_adjustments` (omitted when empty); without
`--speech` the plan is byte-identical.

**Mix.** `render <scene> --speech FILE [--sfx-library DIR] [--audio-plan P]
[--music MUSIC.json] [--makeup-gain auto|off]`: the voice-over (path relative to
the speech file's directory) is placed at t = 0, never ducked, and is the KEY of
the ducking: the music bed ducks 10 dB and the SFX bus 4 dB
(`sidechaincompress`, attack 15 ms, release 250 ms). The key is leveled first
(`volume=20dB,alimiter=limit=0.1`) so the depth does not depend on the voice's
loudness or dynamics; thresholds are fixed (bed ratio 20, SFX ratio 4). Measured on
speech: 9.4 dB (bed), 4.0 dB (SFX) +-0.5; on a stationary tone about 10.9 / 4.6.
With `--speech` the makeup gain defaults to auto and may also cut: integrated
loudness -16 LUFS, the -1 dBTP limiter stays last. A voice-over only render needs
no SFX library and no music. Without `--speech` the graph is the 0.9 graph.

**QA.** `qa <scene> --speech FILE [--audio-plan P] [--mixed MP4] [--json]` prints
a SPEECH QA block (PASS/FAIL/SKIP per check; exit 0 like the other qa modes):
caption timing (<= 1 frame), caption safe area, <= 2 lines / 32 chars, layout QA,
no planned SFX peak within 80 ms of a word onset (with `--audio-plan`), and with
`--mixed` integrated LUFS (-16 +-1), true and sample peak <= -1 dBTP (+0.3 dB
AAC decode margin) and the measured bed duck (10 +-3 dB, with a music bed in the
plan). The speech file is read as written by `voice` (already repaired).

| check | rule | status |
|---|---|---|
| `caption_timing` | every caption word fades in within 1 frame of its spoken start | FAIL |
| `caption_safe_area` | every caption text box inside the safe area | FAIL |
| `caption_lines` | at most 2 lines per page, 32 characters per line | FAIL |
| `speech_rate` (0.20) | per sentence, syllables ÷ (last word end − first word start); sentences under 2 words or 0.5 s are skipped | FAIL outside 3.0–9.0 syll/s, WARN outside 3.5–7.5 |
| `reveal_before_speech` (0.20) | an anchored group (`ArtRecord.reveals`; the kicker is exempt) becomes readable at most 0.35 s before its anchor word starts; groups whose words are not spoken are counted as "unspoken"; SKIP without `--art` anchors | FAIL |
| `reveal_late` (0.20) | the same groups are readable at most 0.8 s after the word ends; a group never readable while its scene is on is a finding too | WARN |
| `spoken_mismatch` (0.20) | the recognised words (`--align local\|asr`) against the script: a missing or changed **content word** (a number, written or said, or a word of the beat's anchors) | FAIL; other words WARN; SKIP without recognised words |
| `layout` | layout QA passes | FAIL |
| `sfx_vs_words` | no planned SFX peak within 80 ms of a word onset (`--audio-plan`) | FAIL |
| `loudness` / `peak` | −16 ±1 LUFS; true and sample peak ≤ −1 dBTP (+0.3 dB AAC margin) (`--mixed`) | FAIL |
| `bed_duck` | the music bed ducks 10 ±3 dB under the voice | FAIL |

**Readable** (0.20, frame resolution through `evaluate_frame`): a layer is
readable when its opacity times its ancestors' (and, under a glyph cascade,
the least visible glyph's) is ≥ 0.5, its depth-of-field blur (its own and its
ancestors', plus a `directional_blur` post effect) is ≤ 2 px at a 1080 px
short side, and its box (minus its clip) overlaps the canvas. A group is
readable at the first frame any of its layers is. WARN findings are printed
but the verdict stays PASS. `--json` carries every finding
(`speech_rate_findings`, `reveal_before_speech_findings`,
`reveal_late_findings`, `spoken_mismatch_findings`) and each group's timing
(`reveal_timings`: word, word start/end, readable frame and time).
