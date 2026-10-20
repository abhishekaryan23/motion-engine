# Motion languages

A **motion language** is a coherent way of moving: timing tendencies, stagger rhythm,
camera and depth behavior, typography behavior, settle character and energy response.
It is **not** a template and **not** a layout. Recipes still decide composition from
what a beat means (emphasize, contrast, reveal, explain); the language decides how
that composition moves. The same beat can be expressed in any language.

```
CreativeIntent (meaning)  ─┐
StyleProfile.motion_language ─┴→ MotionCompiler ─→ per-beat MotionLanguageProfile ─→ recipes
                                                        (timing, stagger, depth, camera,
                                                         typography, settle, energy)
```

## Selecting a language

`StyleProfile.motion_language` is the only public knob: `auto` (default), `minimal`,
`kinetic`, `parallax`, `sequential`, `data`. CreativeIntent never selects motion style.

With `auto`, the compiler chooses per beat from the beat's *structure* (never its topic):

| beat shape | language |
|---|---|
| a number subject the engine can count (`₹42,000`, `40%`, `3.5x`) | data |
| purpose `explain` | sequential |
| `emphasize`, energy building/impact, keyword appears in the statement | kinetic |
| otherwise, layered depth with a moving camera | parallax |
| otherwise | minimal |

## The five languages

**Minimal** — clean, readable, restrained. Short travel (≈45 % of the energy preset),
decisive curves (OutCubic / OutQuint), never an overshoot, half-strength camera,
headlines uncovered by a mask edge. Presets are the three energies:
MinimalCalm, MinimalEditorial, MinimalImpact. Motion clarifies state changes; hierarchy first.

**Sequential** — things arrive one after another in readable, *irregular* rhythms
(stagger presets in frames @30 fps: Calm `0 6 11 19`, Editorial `0 4 7 13 18`,
Impact `0 3 5 10`, extended closed-form by a cycle — no accumulated drift).
Orders: forward, reverse, center-out, edges-in. Units: word, line, item, layer.

**Kinetic** — typography is the motion. Behaviors: WordCascade, LineReveal,
TextMaskReveal, TypeReplace, TypeScaleEmphasis, KeywordPunch, TrackingReveal.
The beat's `keyword` (already in CreativeIntent) is what gets emphasized:
building energy → the keyword grows while the line recedes; impact → an accent
copy strikes over it.

**Parallax** — 2D multi-plane depth. Each layer has a depth factor (background 0.35,
midground 0.7, subject 1.0, foreground 1.45); the scene camera's push and track
move each plane by `zoom^depth` and `pan·depth`. Authors never animate planes
individually. Shared elements resolve into the same camera model.

**Data** — numbers count, quantities grow. Counters (Count op), progress bars,
bar charts, comparison bars, path draws. Numbers stay typographically strong;
motion reads grow → reach value → settle.

## Where things live

| concern | module |
|---|---|
| language profiles | `motion_core::motion::language` |
| stagger timing | `motion_core::motion::stagger` |
| kinetic typography | `motion_core::motion::kinetic` |
| data primitives | `motion_core::motion::dataviz` |
| semantic → language choice | `compiler::language_for` (MotionCompiler only) |
| camera/depth/layout evaluation | `timeline` |

All behaviors expand into primitive MotionScene layers and motions, so the
Timeline and Renderer never learn about words, cascades, charts or meaning.

## Read and evolve (0.4)

Every compiled beat has an internal lifecycle (docs/SCENE_LIFECYCLE.md):
PRE_ENTER → ENTER → SETTLE → READ → EVOLVE → ANTICIPATE → BRIDGE. The language
decides how the beat stays alive after its entrance:

| language | READ (no new information) | EVOLVE (secondary information, one step per event) |
|---|---|---|
| minimal | slow ghost drift, camera push | serif line + underline completion, accent rule extends, slow hero crop when nothing else evolves |
| sequential | slow drift | supporting items one by one, then the consequence/aggregate |
| kinetic | headline holds | keyword emphasis (scale emphasis / punch), then the secondary phrase or a type replace |
| parallax | planes keep drifting at depth-scaled rates | midground reveals the secondary; background fades during ANTICIPATE |
| data | the first value resolves | numerator → ÷ denominator → result → comparison → conclusion |

Every non-final stage eases back during ANTICIPATE, so transitions grow out of motion.
The *composition* (which elements exist and where) comes from the beat's composition
grammar (docs/COMPOSITION_GRAMMAR.md); the language only decides how it moves.

## Motion quality rules

Every behavior follows anticipation → action → settle → read → secondary evolution
→ bridge where appropriate. Avoid: motion that only starts on boundaries, identical
durations, constant bounce, continuous unnecessary motion, all layers moving together.

## Motion signature (0.24 M10)

Speech-lock (`speech_lifecycle`) decides WHEN text appears; the per-look
**motion signature** (`compiler::signature`, data) decides HOW it moves. Every look
uses one small vocabulary and differs in character, not in stacked effects.
Nothing pulses, glitches, echoes or shakes (`slop_flicker` fails a project that does),
and bloom is Cinematic3d only and never on text.

| element | motion | where it is applied |
|---|---|---|
| headline lines | rise 0.35 line heights from a baseline mask, 0.42 s OutQuint, 70 ms stagger | headline builders (language preset; table values) |
| hero figure / word | spring scale 0.94 → 1 plus opacity 0 → 1, 0.45 s, overshoot ≤ 2 % | kinetic slam (`KineticSlam`) |
| pictures, flags, cards | slide 24–40 px from the side the beat opens toward + fade, 0.5 s OutCubic; side alternates by beat | kinetic slam pictures |
| support text | fades in 0.3 s, 60–120 ms after its lead | table values |
| exits | 0.25 s fade + 12 px drift opposite the entrance; no zoom-burst exit | stage exit (`punch` is a drift) |
| camera | one slow push per beat, at most 3 % (documentary 2 %) | beat camera, stage reframes, footage plates |
| payoff colour field | full canvas (x 0..w, y 0..h), past the caption lane | `recipes::stretch_bleeds` |

Looks: Hype is snappier with bigger slides (spring 380/34, 40 px, 0.40 s hero);
Documentary is slower and smaller (24 px, 0.55 s hero, 2 % camera); Cinematic has the
longest eases (HEAVY spring, 0.7 s hero, 0.75 s slide); every other look takes the base.
