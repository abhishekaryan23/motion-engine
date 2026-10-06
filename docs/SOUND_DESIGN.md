# Sound design layer (0.8)

The engine plans one-shot sound effects from the compiled scene and mixes
them under the rendered video. Weak models never pick sounds, files, gains or
times; CreativeIntent gains no field. Without `--sfx-library` every output is
byte-identical to 0.7.1.

```
CreativeIntent + StyleProfile ─→ compile ─→ MotionProject ─┐
CreativeIntent (beat energy) ───────────────────────────────┤
ResolvedStyleProfile (taste) ───────────────────────────────┼─→ plan_audio ─→ AudioPlan (<name>.audio.json)
SfxLibrary (sfx-library.json) ──────────────────────────────┤                     │
MusicPlan? (Phase 3) ───────────────────────────────────────┘                     ↓
                                  render → silent MP4 ─→ mix_audio (ffmpeg) ─→ MP4 + AAC
```

Taste = HOW it sounds (family, level). Lifecycle = WHEN (handoff, ENTER of
impact beats, EVOLVE arrivals). The MotionScene schema and the Timeline are
untouched; the AudioPlan is a separate artifact derived one-way from the scene.

## 1. Timebase
Seconds on the project timeline (`t = frame / fps`), ms-rounded. A cue's
`time` is where the sound's **measured PEAK** lands, never the file start
(pack files have long lead-in silences).

## 2. SFX library (`sfx-library.json` v0.1, `motion_core::audio::SfxLibrary`)
A library root is a directory holding `sfx-library.json` and the sounds;
`path` is relative to that root (never absolute). Supplied at run time with
`--sfx-library DIR` or `$MOTION_SFX_LIBRARY`. Built from a pack by
`motion-engine sfx-index <PACK> --curation curation.json -o <DIR>`, which copies
each curated file to `sounds/<family>/<id>.<ext>`, measures it and writes the
manifest (sorted by id). Families are curation + measurement; folder names lie.

Measurement (`motion_render::sfx::measure_sound`): ffmpeg decodes to mono
48 kHz f32. RMS envelope over 10 ms windows (480 samples), hop 1 ms; window
`k` covers `[k·1 ms, k·1 ms + 10 ms)`, its time is its centre. Let `M` be the
loudest window (first on ties):
- `peak` = centre of `M`; `peak_db` = 20·log10(max |sample|);
- `onset` = centre of the first window with RMS ≥ `M` − 30 dB;
- `audible_end` = end of the last window with RMS ≥ `M` − 40 dB;
- `lufs` = ffmpeg `ebur128` integrated, `None` when gated out (≤ −70);
- times ms-rounded; measurements cached by sha256.
Normalisation is by per-family PEAK target (LUFS fails on very short sounds).

`sfx-pack <DIR> -o motionengine-sfx-addon.zip`: deterministic STORE zip
(sorted entries, fixed timestamp) of the manifest, every referenced sound and
`LICENSE.md`/`README.md`. Packs and zips are gitignored add-ons.

## 3. Cue planning (`plan_audio`, pure, deterministic)
Inputs: compiled project, resolved taste, beat energy per beat (from the
intent), library, optional MusicPlan. Beats are the scenes carrying a
lifecycle (`beat_scenes`; the compiler's whole-piece backdrop has none); a
hand-authored project without lifecycles treats every scene as a beat. The
plan records the style's `min_spacing` and `information_cap` for QA.

Candidates (scene `i`, start `S`, lifecycle `L`; scenes without lifecycle use
`L.enter = 0` and give no arrivals):
| kind | when (peak lands) | family (tables in `audio.rs`) |
|---|---|---|
| open | first scene `S + L.enter` | `open_family(temperament)`: energetic → subdrop, precise → riser (tail) |
| handoff | each scene `i ≥ 1`: midpoint of `[S_i, end_{i−1}]` (= `S_i` when no overlap) | `handoff_cue(transition)`: subtle → whoosh_soft at −24 dBFS, editorial → whoosh_soft, geometric → swipe, kinetic → whoosh_hard, hard → click |
| impact | each impact-energy scene: `S + L.enter` | `impact_family`: hit_hard (energetic temperament or kinetic transition) else hit_soft |
| impact (bed) | first impact scene: `S + L.enter`, subdrop, unless a subdrop cue already exists within 1.0 s | subdrop |
| information | `S + a` for the first `information_cap(density)` of `arrival_times(scene)` | `information_cue(temperament, density)`: restrained → tick at −24 dBFS (none when sparse), editorial → click, precise → tick, energetic → pop. Arrivals are short transients only: a page-turn rustle (`paper`) reads as an unrelated page change, so no taste uses it |

Target peak = the table's level for handoffs, else `family.peak_target_db()`.
`gain_db = round1(clamp(target − sound.peak_db, −40, +18))`.

Sound choice: candidates `library.family(f)` (sorted by id); none → no cue.
Index = `Fnv64(seed LE bytes, scene id, 0xff, kind, 0xff, ordinal LE) % n`,
`seed = taste.effective.seed`, `ordinal` = position among that scene's cues
of that kind. No RNG, no clocks.

Collisions: transient cues (non-bed) closer than `min_spacing(rhythm)`
(slow_breathing 0.6 · measured_editorial 0.45 · progressive 0.35 · active 0.25 ·
high_frequency 0.15 s) keep the higher priority (impact = open 3 > handoff 2 >
information 1); ties keep the earlier, then the smaller sound id. Accept in
order (priority desc, time asc, sound id asc). Beds are exempt.
Cues outside `[0, project duration]` are dropped. Output sorted by
(time, priority desc, sound_id). `reason` = `"<kind>: <taste value> → <family>"`.

Music (Phase 3): with a MusicPlan, `music = MusicBed { track, gain_db,
fade_in 0.5, fade_out 1.0, duck: true }`; cues are unchanged.

## 4. Mix (`motion_render::audio_mix::mix_audio`)
`render <scene> --sfx-library DIR [--audio-plan P]` renders frames and the
silent MP4 exactly as today, then mixes. Default plan: `<scene stem>.audio.json`
beside the scene (stem = file name minus `.motion.json`/`.json`); missing →
error telling the user to run `plan-audio`. `$MOTION_SFX_LIBRARY` alone never
changes a render. Per cue: input file → `aformat=fltp:48000:stereo` →
`atrim=start=max(0, peak − time)` → `volume=<gain>dB` →
`adelay=max(0, time − peak)·1000 ms (all channels)`; all cues → `amix
normalize=0` → `alimiter limit=0.841` (−1.5 dBFS sample peak, so the true peak after AAC stays under −1 dBTP; 0.891 before 0.22) → `apad`, `atrim` to the
project duration → AAC 192 kb/s; video stream copied. No cues and no music →
a silent track of the same duration. Music bed: `volume`, `afade` in/out,
`sidechaincompress` keyed by the SFX bus, mixed under the SFX before the limiter.

## 5. QA (`qa <scene> --audio-plan P [--sfx-library DIR] [--mixed MP4]`)
Text section + `audio` key in `--json`. Checks: cues per scene; minimum gap
between transient cues vs `min_spacing`; every cue inside its scene's
`[start, end]` and the project; nothing but handoffs at or after the scene's
`S + L.anticipate`; information cues per scene ≤ cap. With `--mixed`: decode
mono 48 kHz, envelope as §2; for every **isolated** cue (no other cue within
±0.25 s) the measured peak is the loudest window centre within ±0.1 s of
`time`; error ≤ 1 frame (1/fps) = PASS, else WARN. Integrated LUFS and
sample peak of the mix are reported. Verdict: FAIL on placement violations,
WARN on alignment misses, else PASS.

## 6. CLI summary
```
motion-engine sfx-index <PACK> --curation assets/sfx/curation.json -o <LIB_DIR>
motion-engine sfx-pack <LIB_DIR> -o output/motionengine-sfx-addon.zip
motion-engine plan-audio out.motion.json --intent i.json --style s.json --sfx-library <LIB_DIR> -o out.audio.json
motion-engine render out.motion.json --sfx-library <LIB_DIR>
motion-engine qa out.motion.json --audio-plan out.audio.json --sfx-library <LIB_DIR> --mixed out/name.mp4
```

## 7. Music bed and choreography v0 (Phase 3)
Input: `--music <track>` analysed offline by `music-index <track>` into a
MusicPlan (`<stem>.music.json` beside the track; `track` relative to it). The
owner supplies the track; none is bundled unless licensed.

`music-index` (deterministic, no ML): ffmpeg decode to mono 22.05 kHz f32;
onset envelope = positive spectral-flux-like difference of 23 ms RMS frames
(hop 512 samples ≈ 23.2 ms), half-wave rectified, mean-removed; tempo = lag of
the autocorrelation maximum within 60–180 BPM weighted by a log-Gaussian tempo
prior (centre 120 BPM, σ 1 octave), refined by a ±4 % comb search on a 32-sample
hop envelope (parabolic interpolation, then
octave check: prefer the double tempo when its autocorrelation ≥ 0.8× and
the result stays ≤ 180), `bpm` rounded to 0.1; beat phase = offset maximizing
the summed envelope on the grid; `beat_times` = phase + k·60/bpm over the
track; `downbeat_times` = every 4th beat starting at the beat index (0..3)
whose beats carry the most envelope energy; `gain_db` = −18 − integrated
LUFS of the track (bed sits ≈ −18 LUFS), clamped to [−30, 0]; times ms-rounded;
cached by sha256.

Choreography (`compiler::choreography::snap_handoffs`, before lifecycle
planning in `plan_timing`): each handoff anchor (overlap midpoint) snaps to the
nearest downbeat within `snap_tolerance(rhythm)` (slow_breathing ±0.35 …
high_frequency ±0.12 s) by changing the previous beat's duration within
[0.85×, 1.20×] and ≥ 2 s. Impact cues (hit + first-impact subdrop, in `plan_audio`) move to
the nearest downbeat inside `[handoff anchor, S + L.settle]` (anchor = the
overlap midpoint into the scene; `S + L.enter` for the first beat), so a hit
lands with a snapped handoff.
Taste tables are never edited; without `--music` output is unchanged.

Mix: the bed (`volume`, `afade` in 0.5 s / out 1.0 s) is sidechain-ducked
(`sidechaincompress`, keyed by the SFX bus) and mixed under the SFX before
the limiter. `MusicBed.track` in the AudioPlan is relative to the AudioPlan
file's directory.
