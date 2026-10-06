# Motion QA (structural pacing diagnostic)

`motion-engine qa <file.motion.json> [--step N]` renders a low-resolution luma
profile of a scene and reports how much the composition changes between
consecutive sampled frames. It exists to catch the pacing
"static composition -> giant transition spike -> static composition". It is not
computer vision: no objects, no semantics.

## How it works
1. Each sampled frame (every `N`th, default 1) is resolved and rendered with `CpuRenderer`.
2. Animated grain textures (`texture` layers with `animated: true`) are removed first; grain is not scene motion.
3. Luma is box-downsampled about 16x, blurred 3x3, and consecutive samples are diffed (mean absolute difference, 0..1).

## Output
- `median / p95 / max` of the per-transition diffs.
- `spikes`: frames whose diff exceeds `max(4 x median, 0.02)`, grouped into consecutive ranges (`spike_clusters`). Each cluster reports its peak frame and peak as a multiple of `max(median, 0.005)`.
- `static runs`: at least `fps/2` frames with diff < 0.002.
- `verdict`: `ok`, or e.g. `static 2.0s then spike 127\u2013133 peak x83.2 at 4.3s` when a spike cluster starts within 4 sampled transitions (a short ramp) after a static run of at least 1.0 s.
- A sparkline (`▁▂▃▄▅▆▇█`), one character per sampled transition, scaled to `max(max diff, 0.02)`.

## Library
`motion_render::structural_profile(&project, &renderer, step) -> MotionProfile`.
`motion_render::qa::analyze(fps, step, diffs)` builds a profile from raw diffs.

## Lifecycle section (0.4)
For every scene with a compiler-written `lifecycle` (see `docs/SCENE_LIFECYCLE.md`; backdrops and hand-authored scenes without one are skipped), `qa` prints a `lifecycle` section after the profile, then `lifecycle: N scenes, W warnings`. `--json` prints `{ "profile": {...}, "scenes": [...] }` instead.

```
beat_3
  ENTER activity: PASS
  SETTLE: PASS
  READ structural activity: LOW (mean 0.0031)
  EVOLVE events: 2 (activity PASS)
  ANTICIPATE: PASS
  transition spike ratio: HIGH (x11.4)
  longest static hold: 0.4s
  thirds (top/middle/bottom): 0.31 / 0.28 / 0.03
  WARNING: Static read phase before abrupt transition
  WARNING: Empty lower third (top-heavy composition): content top 0.31, middle 0.28, bottom 0.03
```

Uses the same grain-free, downsampled luma diffs as above; no new rendering.

- **Frame mapping**: phase bounds are scene-local seconds; absolute frame = `round((scene.start + t) x fps)`. Diff `i` is the transition into frame `(i + 1) x step` and belongs to the phase containing that frame. PRE_ENTER of a scene overlaps the previous scene's BRIDGE, so the same transitions can count for both.
- **Phase status** (`PhaseActivity`): `EMPTY` no sampled transition in the phase; `STATIC` every diff < 0.002; `LOW` mean < 0.004; else `PASS`. A `STATIC`/`LOW` phase whose mean 1 s accumulated change is >= 0.001 is `DRIFT` instead (see below). `DRIFT` is alive: it never triggers the quiet-phase warnings.
- **evolve events**: distinct motion start times in `[read, anticipate)` (starts within 0.12 s of the previous cluster member are one event), ignoring targets ending in `.stage` or `.ghost`. Read from the motion file, not from pixels.
- **spike ratio**: max diff over `[bridge - 0.2 s, scene end]` divided by `max(median diff over [enter, bridge), 0.005)`; `HIGH` above 8. The last scene (`bridge == duration`) reports x0.0 / OK.
- **longest static hold**: longest run of consecutive diffs < 0.002 inside `[settle, bridge)`, in seconds (run length x step / fps). A sample whose 1 s accumulated change is >= 0.001 (a drift span) does not count as static.
- **Warnings**: `Static read phase before abrupt transition` (READ static/low, EVOLVE static/low/empty, spike HIGH); `Dead hold X.Xs in READ/EVOLVE` (hold >= 1.5 s; shorter static reads after new information are deliberate reading time); `No evolve events` (none, and EVOLVE >= 0.6 s); `ENTER shows no activity` (ENTER static).

- **thirds**: on the middle frame of READ (nearest sampled frame), the fraction of downsampled cells whose grain-free luma differs from that frame's median luma by more than 0.04, per horizontal third: `[top, middle, bottom]`. Warning `Empty lower third (top-heavy composition)` when bottom < 0.04 while top + middle > 0.25. Informational only. All zero (no warning) if the frame was not measured.

Library: `motion_render::lifecycle_report(&project, &profile) -> Vec<SceneQa>`; `SceneQa::report()` formats one scene.

## Slow drift (0.5)
Very slow continuous motion (ghost drift, a 1.0 -> 1.02 push over 3 s) produces per-frame diffs below the noise floor and used to read `LOW`/`STATIC`. Every sampled frame is now also diffed against the sample about 1 s earlier (window = `round(fps / step)` samples, never reaching back past the start of the scene containing it) using the same downsampled grain-free luma. `MotionProfile.accum[i]` is that value for the transition `diffs[i]`; `PhaseActivity.accum` is its mean over a phase.

- A phase that is `STATIC`/`LOW` per frame but has mean accumulated change >= 0.001 is `DRIFT`. A truly still scene accumulates exactly 0 (deterministic render, grain stripped), so it stays `STATIC`; a 1.0 -> 1.02 push on one large typographic layer accumulates about 0.0016 and reads `DRIFT`.
- The 1 s window is clamped at the scene start, so drift reads static for the first fraction of a second (the report's static hold can include that start-up lag).
- Text: `READ structural activity: DRIFT (mean 0.0007, 1s change 0.0337)`.
- JSON (all additive): `profile.accum`, `scenes[].phases[].accum`, `scenes[].thirds` (`[top, middle, bottom]`), and status `"DRIFT"`.

## Limits
- Lifecycle status is a heuristic: a phase that is `LOW` may still be fine on a large canvas (diffs are global means), and with a large `--step` short phases can be `EMPTY`.
- Evolve events are counted from motion start times, so a motion that starts but is visually imperceptible still counts.
- A ramp-in longer than 4 sampled transitions between the hold and the spike hides the pattern; holds shorter than 1.0 s are never flagged.
- Diffs are global means, so small local motion on a large canvas can read as static.
