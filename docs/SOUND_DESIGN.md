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

(0.24 M5) Cue taste. The palette table is `assets/sfx/palettes.json` (not code):
a palette is picked by the art-direction look (`ProjectMeta.art.look`), else by
temperament, and lists the families for each role: editorial (paper, tick,
hit_soft, whoosh_soft, subdrop), pop (pop, swipe, hit_soft), tech (data, glitch,
click, notification), street, cinematic (whoosh_hard, hit_hard, riser, subdrop).
`click` is only in the tech palette.

Slots (scene `i`, start `S`, lifecycle `L`; at most 1 hero + 2 small per beat):
| role | when (peak lands) | families |
|---|---|---|
| hero | the beat's lead reveal at its settle: end of the first motion on the focal layer (`art.focal`), else a `shared.*` number, `.hero*`, `.zone_*`, `.stamp`, `.head.*`; fallback `S + L.settle`. On a MusicPlan an impact beat's hero moves to the nearest downbeat in `[handoff anchor, S + settle]` | `palette.hero` |
| hero bed | the video's biggest beat (highest energy, last on ties): beds at the hero's instant, a riser peaking 20 ms before it where the palette has one | `hero_big_with`, `riser` |
| close | the last beat's hero, when it is not the biggest | `palette.close` (beds only) |
| open | first visible arrival of beat 0 (soft, transition loudness) | `palette.open` |
| transition | overlap midpoint of cuts that move (overlap > 50 ms or not a hard cut); only every second moving cut | `palette.transition` |
| small | the speech accent word (`ACCENT_LEAD` before it) then EVOLVE arrivals (`arrival_times`), within the beat's two small slots and `information_cap`; none for restrained + sparse | `palette.small` |

Rules applied in order: collisions (priority impact/open 3 > handoff 2 > information 1, hero first, then earlier; transient cues closer than `min_spacing` lose, beds exempt); breath (a non-bed cue within 250 ms before a hero is dropped); families in preference order, heroes first, a family skipped once it would exceed 40 % of the cues (with 5 or more); non-hero cues of a family still over 40 % are dropped, latest small first; sound = FNV pick among the family's sounds not used in the previous 4 cues (and, when possible, ones whose role loudness is reachable under the -2 dBFS peak ceiling).
Loudness: `gain_db = role_lufs - lufs + jitter`, `role_lufs` hero -20, transition -24, small -28 (palette file), `lufs` = measured, or `peak_db - 18` when the sound is too short to measure, jitter in +-0.9 dB (FNV of seed, scene, kind, ordinal), then capped so the sample peak stays at or below -2 dBFS and clamped to [-40, +18]. No cue is louder than its beat's hero. `reason` = `"<kind>: <role> <what> (<palette>) -> <family>"`, accents `"accent: '<word>' (small, <palette>) -> <family>"`.
**Count-up tick train (0.24 M7).** Every `Count` motion of 0.5 s or more gets a train of `count tick i/n` cues (kind impact, family `palette.count`: tick for editorial/street/cinematic, data for tech, pop for pop). A tick lands each time the motion's eased progress (spring counters: OutQuint, as the timeline) crosses the next 1/N step, N = round(16 ticks/s * duration / fastest progress rate), at least 4, so ticks speed up and slow down with the number. Rate cap 18/s (pop 8/s, 10/s under speech), at least 70 ms apart; the progress crossing at 1.0 (the settle) and everything within 250 ms of it get no tick; a tick within 250 ms before any hero, or 60 ms of another transient cue, is dropped; under speech a tick is nudged (+-120 ms, 5 ms steps) off the 80 ms word-onset zones, else dropped. Fewer than 3 survivors: no train. Gain follows the rate (up to 6 dB under the fastest tick at the slow ends, the last three ticks a further -2/-4/-6 dB; `role_lufs.count` -28 + `palette.count_offset_db`, cinematic -4). Sounds go round-robin through the family so a sample never repeats back to back. The settle gets the beat's hero when one lands within 150 ms, else a soft `count lock` (`palette.count_lock`: hit_soft, tech click, pop pop, street hit_hard). Trains are outside the variety, family-share and information-cap rules, `min_spacing` (QA included) and the speech mover; each tick is an ordinary `AudioCue` (`is_count_tick`), so the mixer needs no change.

**Under a voice-over (0.24 M7).** With a SpeechMap the role targets come from `palettes.json` `voice` instead of `role_lufs`: while a word is spoken hero -30, transition -32, small -34, count -36 (+ -3 for ticks), every one at least 12 LU under `VOICE_LUFS` (-18); in gaps (silence of 0.25 s or more between words) +4 dB (still 8 LU under or more); a cue on a word's nucleus (its middle half) a further -3 dB; jitter only lowers. The 80 ms onset rule is unchanged. Without a SpeechMap the M5 targets are untouched. Mixer safety nets: the SFX sidechain duck is 8 dB (`SFX_DUCK_DB`) and the voice-relative SFX bus ceiling is 12 LU under the voice (`MixLevels::sfx_under_speech_db`).

Older tables (`handoff_cue`, `information_cue`, `impact_family`, `open_family`) stay as public reference data but no longer drive the plan.

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

`qa --speech S --audio-plan P --sfx-library DIR` (0.24 M7) adds `sfx_over_voice`: the SFX stem is rebuilt as the mix makes it and metered against the voice stem per 400 ms window inside the spoken segments; FAIL when any window has the SFX within 10 LU of the voice; one line names the worst window.

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
