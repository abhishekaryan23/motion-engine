# Scene lifecycle (0.4)

Scenes used to go ENTER → SETTLE → static hold → transition. Since 0.4 every
compiled beat has an internal lifecycle, so it keeps communicating after its
entrance:

```
PRE_ENTER → ENTER → SETTLE → READ → EVOLVE → ANTICIPATE → BRIDGE
```

The lifecycle is **internal**. CreativeIntent has no lifecycle, phase or
timing fields and never will; weak models describe meaning, the engine owns
time.

## Representation

`scene::Lifecycle` — ordered boundaries in scene-local seconds:

```
0 ─PRE_ENTER─ enter ─ENTER─ settle ─SETTLE─ read ─READ─ evolve ─EVOLVE─ anticipate ─ANTICIPATE─ bridge ─BRIDGE─ duration
```

Invariant `0 <= enter <= settle <= read <= evolve <= anticipate <= bridge <= duration`.
Boundaries instead of ranges make overlapping phases unrepresentable. The
compiler writes it to `Scene.lifecycle` (optional, informational): the
Timeline and Renderer ignore it, `validate` checks the invariant, `qa` reads it.
Hand-authored scenes may omit it.

Helpers: `Lifecycle::range(phase, duration)`, `phase_at(t)`, `evolve_events(n)`
(n event times in `[evolve, anticipate)`, first at `evolve`, evenly spaced),
`Lifecycle::spread(from, to, n)`.

## Planning (`motion::lifecycle::plan`)

Planned per beat in `compiler::plan_timing`, **before** any composition runs,
from: duration, `enter_at`, `overlap_out`, energy, motion language, density
(`units` = statement words + subject units, `secondary_units` = units that
arrive after the primary is read). Closed-form, deterministic:

| phase | length |
|---|---|
| PRE_ENTER | `[0, enter_at)` — overlaps the previous scene's BRIDGE |
| ENTER | language base (minimal 0.75 · kinetic 0.9 · parallax 0.85 · sequential/data 0.8 s) + 0.03 s/unit (≤ 12), × energy (calm 1.15 · building 1 · impact 0.82), ≤ 40 % of the live span |
| SETTLE | language base (0.25–0.35 s) × energy, ≤ 12 % of the live span |
| READ / EVOLVE | the remaining hold split by a language read share (minimal/parallax 0.5, kinetic 0.42, sequential/data 0.34, −0.04 per secondary unit, ≥ 0.22); READ ≥ 0.5 s when it fits |
| ANTICIPATE | 14 % of the live span × energy, clamped 0.25–0.6 s; last scene 0.2–0.4 s |
| BRIDGE | `[duration − overlap_out, duration)` — exactly the outgoing transition; empty for the last scene |

Live span = `bridge − enter_at`. Beat durations are unchanged from 0.3.5:
the lifecycle redistributes the time that used to be a dead hold.

## With a voice-over (0.20)

With `--speech`, `compiler::speech_lifecycle` re-places each planned beat's
boundaries from its own spoken words, before EVOLVE snaps to a word start:
ENTER = `WORD_CUE_LEAD` before the first content word (not a function word);
READ = the end of the first clause (a word followed by `, ; : . ! ?` or a dash in
the spoken line) at or after the primary's first mention, else the primary's
last word + 0.25 s; EVOLVE = the first word naming the secondary, else the
keyword, after READ, else the start of the next clause; ANTICIPATE = the last
word's end + a 0.25–0.6 s tail (longer when more of the beat is left).
`lifecycle::retarget` clamps these targets into ranges derived from the
planned spans, so no phase starves: ENTER is never before the planned ENTER
(the overlap) and leaves room for everything after it; the ENTER and SETTLE
spans keep their planned lengths; READ ≥ max(0.5 s, 40 % of its planned span)
and EVOLVE ≥ max(0.4 s, 40 % of its planned span) (never more than planned, so
a dense beat's EVOLVE events and a sphere's dwells keep room); ANTICIPATE may
move earlier, never later than planned (so it never runs into the bridge);
BRIDGE never moves. A speech-placed EVOLVE that the clamp moved off its word
goes onto the nearest legal word start within the snap window. A boundary without a target
keeps its fraction. `SpeechRecord.phases` records per beat whether each of
ENTER / READ / EVOLVE / ANTICIPATE came from `speech` or the `fraction` (also
when the clamp put it back on the planned value). A beat with no spoken words,
and every compile without `--speech`, keeps exactly the closed-form plan.

## Scheduling contract (every composition follows it)

| phase | what may happen |
|---|---|
| ENTER | the primary visual arrives (headline, hero subject, first item) starting at `life.enter` |
| SETTLE | springs land; no new information |
| READ | **no new information**; only slow continuous motion (camera push, ghost drift, plane drift) |
| EVOLVE | secondary information arrives at `life.evolve_events(n)` — one semantic step per event (supporting item, state change, derived step, comparison, keyword emphasis, consequence) |
| ANTICIPATE | no new information; the stage wrapper eases the whole stage back (scale 1 → 0.975, lift 14u) so the exit grows out of motion; compositions may dim secondary layers or let background planes begin to leave |
| BRIDGE | owned by the stage exit + the incoming scene's PRE_ENTER |

Rules: nothing new *starts* at or after `life.anticipate` except anticipation
and exit motions; every motion ends by the scene duration (validation);
motions on one channel never overlap (validation). Transform motions on
**shared elements** must return to neutral inside their scene (they only apply
while the scene is active).

## Per-language read/evolve behavior

| language | READ | EVOLVE |
|---|---|---|
| minimal | very slow ghost drift (flat styles too), camera push | secondary label / serif line + underline completion; kicker rule extends; no floating, no bounce |
| sequential | slow drift | items arrive one per event: primary → supporting 1 → supporting 2 → consequence; collections never assemble at once |
| kinetic | headline holds | keyword emphasis (TypeScaleEmphasis / KeywordPunch) at the first event, secondary phrase next |
| parallax | planes keep drifting at depth-scaled rates | midground reveals secondary information; background starts fading during ANTICIPATE |
| data | the first value resolves | numerator → ÷ denominator → derived result → comparison/annotation → conclusion emphasis |

## Where it lives

| concern | module |
|---|---|
| type + helpers | `motion_core::scene::{Lifecycle, Phase}` |
| planner | `motion_core::motion::lifecycle::plan` |
| per-beat plan | `compiler::BeatPlan.life` |
| anticipation | `compiler::recipes::wrap_stage` |
| diagnostics | `motion-engine qa` (lifecycle section) |
