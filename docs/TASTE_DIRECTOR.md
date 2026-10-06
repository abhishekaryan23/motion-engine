# TasteDirector (0.6)

Taste is an **engine capability**, not something the model has to write out.
A weak model names a handful of *tendencies* (`tone`, `polarity`, `temperature`,
`temperament`, `density`); the engine's TasteDirector resolves them, together,
into a complete and internally consistent design system: palette, background,
typography, material, image treatment, **and** how motion feels (motion
temperament, transition character, composition rhythm, visual density, scale
contrast, layer activity).

The model never picks a hex color, a font, a duration or an easing curve. It says
"technical, dark, balanced"; the engine knows what that means.

Source of truth: `crates/motion-core/src/compiler/taste.rs` (semantic resolution),
`compiler/temporal.rs` (the only place tendencies become numbers), and the
consumers `backdrop.rs`, `transition.rs`, `furniture.rs`, `recipes.rs`
(`wrap_stage`, ghost word), `mod.rs` (`plan_timing_with`). Design rationale:
`DECISIONS.md` entries 44-50.

## 1. Pipeline

```
StyleProfile (+ optional ReferenceStyleProfile, + story key)
   │
   ▼  compiler/taste.rs   "stage 0" of the compiler, runs once per compile
TasteDirector — small deterministic resolvers, driven by one tone table
   ColorDirector            palette family + curated colors (accent from a curated list)
   BackgroundDirector       PaperCollage | PaperField | TechnicalGrid | PrintFields | CleanFlat
   TypographyDirector       GroteskSerif | SerifSans | CondensedMono | PosterBold
   MaterialDirector         UncoatedPaper | CleanFlat | Screen | CoatedPrint
   ImageTreatmentDirector   Classic | PaperCutout | Monochrome | PrintCutout
   MotionTemperamentDirector  Restrained | Editorial | Precise | Energetic  (+ trait table)
   TransitionCharacterDirector Subtle | Editorial | Geometric | Kinetic | Hard  (+ wipe/exit/overlap)
   CompositionRhythmDirector  SlowBreathing | MeasuredEditorial | Progressive | Active | HighFrequency
   VisualDensityDirector    density level + annotation style + collage patches + ghost word level
   ScaleContrast / LayerActivity   Subtle..Dramatic; foreground presence + mid/background activity
   VariationDirector        curated alternatives hashed from (style seed, story key)
   │
   ▼
ResolvedStyleProfile        internal, semantic (enums + curated colors). No durations, frames, easing constants.
   │      also carries `effective`: the StyleProfile the older compiler paths read
   ▼
compiler/temporal.rs        tendencies → multipliers, curves, counts (the single mapping table)
   │
   ▼
Lifecycle (motion/lifecycle.rs) + Timeline scenes/motions  →  MotionScene
```

**Boundary.** TasteDirector output is *semantic* (`Precise`, `Geometric`,
`Progressive`, `Large`). `temporal.rs` is the only module that turns those words
into numbers. The renderer and Timeline never see taste. The resolved profile is
internal: weak models never author it (only the five fields on `StyleProfile`
are public).

Determinism: no clocks, no accumulated time. The only variation input is
`(style.seed, story_key)`, where `story_key` is FNV-1a over the title and beat
statements (`taste::story_key`). Same inputs, same output.

## 2. Public fields and precedence

`StyleProfile` gains five optional enums (all default `auto`):

| field | values |
|---|---|
| `tone` | `auto`, `editorial`, `technical`, `playful` |
| `polarity` | `auto`, `light`, `dark` |
| `temperature` | `auto`, `warm`, `cool` |
| `temperament` | `auto`, `restrained`, `balanced`, `energetic` |
| `density` | `auto`, `sparse`, `balanced`, `dense` |

Resolution order for every dimension: **explicit taste field > reference
principle (if a ReferenceStyleProfile was given) > tone default.**

- Tone itself: an explicit `tone` wins; else the reference's tone; else, if *any
  other* taste field is set, tone becomes `editorial`; else the profile is
  **Classic**.
- Legacy fields (`material`, `texture_style`, `camera_style`, `typography_style`,
  `depth`, `accent_role`, `motion_language`, `seed`, `family`) stay supported. A
  legacy field at its default never overrides taste; a **non-default** legacy
  field is an explicit override and wins over the tone's preset (implemented in
  `effective_style`). Examples: `typography_style: condensed_mono` forces the
  condensed/mono pairing under any tone; `accent_role: cobalt` or `acid` replaces
  the palette's curated accent; `texture_style: heavy_print` stays heavy under
  `technical`; a non-`auto` `motion_language` disables the temperament's language
  lean.
- **All fields `auto` = Classic = the identity.** Classic is the pre-0.6 warm
  editorial collage; every temporal mapping is the identity and palette/fonts are
  the pre-0.6 ones, so old styles compile byte-identically (goldens unchanged).
- `polarity` / `temperature` `auto`: Technical resolves dark + cool; every other
  tone light + warm. Palette family from (tone, polarity, temperature): Playful
  gets `print_bright` / `print_dark`; otherwise `warm_paper` (light warm),
  `cool_paper` (light cool), `dark_warm`, `dark_cool`.
- `temperament: balanced` resolves to `precise` under Technical, otherwise to
  `editorial` motion.
- **Accent hue is not style.** `accent_role` only chooses the accent color; it
  does not change the palette family, typography, motion or anything else that
  makes a design system distinct.

## 3. Tone table (defaults when a dimension is `auto`)

Derived from `resolve_with` and its resolvers in `taste.rs`.

| dimension | Classic (all auto) | Editorial | Technical | Playful |
|---|---|---|---|---|
| polarity / temperature | light / warm | light / warm | dark / cool | light / warm |
| palette family | warm_paper (pre-0.6 palette) | warm_paper | dark_cool | print_bright |
| palette trajectory | steady | steady | steady | field_cycle (field colors rotate per beat) |
| background | paper_collage | paper_field | technical_grid | print_fields |
| typography | grotesk_serif | serif_sans | condensed_mono | poster_bold |
| material | uncoated_paper (legacy `material` decides: `flat` gives clean_flat) | uncoated_paper | screen | coated_print |
| image treatment | classic | paper_cutout | monochrome | print_cutout |
| motion temperament | editorial | restrained | precise | energetic |
| transition character | editorial | editorial | geometric | kinetic |
| visual density | balanced | balanced | balanced | dense |
| annotation | none | editorial | structured | graphic |
| collage patches | yes if `depth` layered | no | no | yes |
| ghost word level | medium | low | medium | high |
| composition rhythm | measured_editorial | measured_editorial | progressive | progressive |
| scale contrast | moderate | large | moderate | dramatic |
| layers (fg / mid / bg) | balanced / quiet / quiet | balanced / quiet / still | balanced / structured / structured | dominant / active / active |

Temperament-dependent rules (an explicit `temperament` shifts these):

- Motion trait table (`motion_temperament`):

| trait | restrained | editorial | precise | energetic |
|---|---|---|---|---|
| amplitude | low | medium | medium | high |
| settle | soft | standard | crisp | springy |
| overshoot | low | medium | low | high |
| stagger | editorial | editorial | structured | cascading |
| camera | controlled | standard | controlled | active |
| anticipation | low | medium | medium | high |
| secondary motion | low | medium | medium | high |
| READ activity | low | medium | medium | high |
| language lean | calm | classic | structured | kinetic |

- Transition (tone, motion kind): Classic always `editorial`; `restrained`
  motion under any tone gives `editorial`; otherwise Editorial tone `editorial`,
  Technical `geometric`, Playful `kinetic`.
- Rhythm (tone, motion kind): Classic `measured_editorial`; `restrained` gives
  `measured_editorial` (all tones); Editorial + energetic `progressive`
  (otherwise `measured_editorial`); Technical/Playful + energetic `active`,
  otherwise `progressive`.
- Density `auto`: Playful `dense`, else `balanced`. Density level then drives
  annotation (`sparse` none; Technical structured; Playful graphic; otherwise
  editorial), collage patches (`sparse` off, `dense` on, `balanced` on only for
  Classic and Playful) and ghost word (`sparse` low; Editorial low; `dense`
  high; else medium). Classic + balanced is fixed at none / patches by `depth` /
  medium. Every density keeps one clear primary focus and readable text.
- Transition families carry wipe / exit / overlap: `subtle` never wipes, fade
  exit, long overlap; `editorial` wipes on impact only, lift exit, standard
  overlap; `geometric` frequent wipes, slide exit, standard overlap; `kinetic`
  frequent wipes, punch exit, short overlap; `hard` never wipes, fade exit,
  short overlap.

The tone table only reaches part of the vocabulary: `slow_breathing` and
`high_frequency` rhythm and `subtle` / `hard` transitions cannot be produced by
the five public fields; in 0.6 they are reachable only through a
ReferenceStyleProfile (see section 7).

## 4. What each dimension changes in the compiled output

All numbers below come from `compiler/temporal.rs` unless noted. Classic is the
identity for every row.

**Motion temperament** (`apply_temperament`, per beat, applied to the motion
language profile after the language is chosen):
- amplitude scales the preset travel: low 0.65x, medium 1.0x, high 1.4x.
- settle scales preset duration (soft 1.1x, standard 1.0x, crisp 0.8x, springy
  0.92x) and swaps easings (soft: out-cubic / in-out-cubic; crisp: out-quint;
  springy: editorial spring, impact spring on impact beats, never for the data
  language so numbers land exactly).
- overshoot `low` replaces any spring with out-quint.
- stagger: structured uses the tight impact stagger preset; cascading uses the
  wider calm preset (editorial preset on impact beats).
- camera scales push and track: still 0, controlled 0.5x, standard 1x, active
  1.8x / 1.6x (an active camera that was moving but had no pan gets a small diagonal
  track).
- anticipation: stage scale at the end of ANTICIPATE 0.985 / 0.975 / 0.95 and
  a lift/lead multiplier of 0.5 / 1.0 / 2.2 (low / medium / high), in
  `wrap_stage`.
- secondary motion multiplies the excess of emphasis pulses and hero crop
  evolution over neutral: 0.5 / 1.0 / 1.7.
- READ activity multiplies ghost-word drift: 0.7 / 1.0 / 1.35.
- language lean, only when `motion_language` is `auto` (meaning still decides
  first): calm turns kinetic into minimal unless the beat is impact; structured
  turns parallax into minimal; kinetic turns minimal/parallax into kinetic when
  the beat has a keyword or is not calm.

**Transition character** (`plan_timing_with`, `handoff`, `wrap_stage`,
`transition.rs`):
- overlap between beats is scaled: long 1.25x, standard 1.0x, short 0.72x.
- the wipe tendency decides, per incoming beat, an accent flood and/or a panel
  wipe: never = none; impact_only = accent flood on impact beats; frequent = accent flood on
  impact beats, a panel wipe on the others. The first beat is never entered
  behind a wipe.
- `geometric` panel wipe: a straight field-colored panel with thin accent
  edges slides across (direction alternates with beat index) at z 60.
  `kinetic`: two full-height bars, accent first and the field color trailing
  0.06 s, sweeping vertically. The wipe covers during the overlap and is clear
  shortly after the incoming beat starts entering.
- exit gesture of the outgoing stage: `lift` (pre-0.6: lead up, fade, rise), `fade`
  (fade in place), `slide` (lead then slide 30 % of the width sideways), `punch`
  (scale up 1.14x and fade). Under an accent flood or panel wipe the outgoing
  stage just clears out underneath the cover.

**Composition rhythm** (a bias in event density, never timing):
- beat duration factor: slow_breathing 1.12, measured_editorial 1.0,
  progressive 0.95, active 0.88, high_frequency 0.82 (with a 3 s floor once the
  factor is not 1.0; reading-time rules still set the base duration).
- READ share shift for the lifecycle: +0.10 / 0 / -0.06 / -0.10 / -0.14, so
  busier rhythms leave more of the beat to EVOLVE (secondary information
  arrives sooner).
- **hierarchy reframes** in EVOLVE (`wrap_stage`): stage scale steps at evenly
  spaced points inside `[evolve, anticipate)`: slow_breathing and
  measured_editorial 0; progressive 1 x 3 %; active 1 x 6 %;
  high_frequency 2 x 5 %. Skipped when the EVOLVE span is 0.8 s or less.
- **background handoff** (ghost word, `recipes.rs`): active and high_frequency
  step the ghost word forward at EVOLVE (scale 1.0 to 1.08 / 1.12) when the EVOLVE
  span exceeds 0.6 s.

**Scale contrast**: typesetting gains `(display, support)`: subtle
(0.88, 1.06), moderate (1.0, 1.0), large (1.12, 0.94), dramatic (1.26, 0.86).

**Layer activity**: background drift multiplier of the ghost word and backdrop
grid (still 0.4, quiet/structured 1.0, active 1.8); mid-ground active spreads
camera planes (parallax x1.25, or introduces background/mid-ground depth
0.75 / 0.9 otherwise); still halves plane separation; a dominant foreground rides
slightly ahead of the subject (depth 1.12).

**Background grammar** (`backdrop.rs`, one piece-wide scene under every beat,
z 1..5, grain z 90): `paper_collage`/`paper_field` the paper ground; `clean_flat`
plain ground; `technical_grid` flat ground plus a measurement grid (90 px cells,
major line every fourth) drifting linearly, with accent pulses at beat
boundaries, both scaled by background activity; `print_fields` large graphic
color fields that recompose at each beat boundary (0.6 s out-quint when the
background is active, else 0.9 s out-cubic; not at all when it is still) and
cross-fade colors per beat under `field_cycle`.

**Visual density / furniture** (`furniture.rs`): annotation layers live in the
margins, outside safe text areas, at low z, and arrive after the headline
settles: `editorial` a two-digit folio and a hairline rule (dense adds an accent
tick); `structured` coordinate ticks, a hairline and a mono data label
(`SEC 02 - T+4.6`); `graphic` big index numeral and a sticker (dense adds more);
`none` adds nothing.

**Typography / material / image treatment**: the typography pairing selects the
font set (`FontSet::for_pairing`). Ground, grain and collage patches follow the
*effective* style (a defaulted `material` / `texture_style` / `depth` takes the
tone's preset: Technical is flat with no grain, Playful flat with print grain,
dark polarity forces a flat ground). The image treatment preset
(`compiler/treatment.rs`) reads the effective tone: Technical gives monochrome /
muted documentary, Playful paper cutouts (duotone environments), otherwise the
legacy material/texture mapping. In 0.6 the resolved `material` and
`image_treatment` enums are part of the fingerprint and the preview, but the
compiler does not read them directly.

## 5. StyleFingerprint and `compare-styles`

Two styles are "genuinely different" only if their resolved design systems
differ. `ResolvedStyleProfile::fingerprint()` reduces a profile to eleven
dimensions:

palette family, polarity, background, typography, material, image treatment,
motion character (temperament kind), transition style (family), density (level
+ annotation + patches + ghost level), composition rhythm, scale contrast.

Accent hue is deliberately absent: red to blue is not a new style. `compare`
returns a per-dimension `different` flag; `distance` is the count (0..=11). There
is no aesthetic score. `motion-engine compare-styles A.style.json B.style.json`
prints it (see `docs/CLI_CONSUMER_GUIDE.md`). Use it to check that styles you
generate for different stories are not near-duplicates.

## 6. `style-preview`

`motion-engine style-preview STYLE.json [-o out.png] [--json]` renders **one
frame** (frame 0 of a one-second synthetic project): the style's backdrop,
palette swatches, a typography specimen and material, plus a text summary of
the temporal character (temperament, transition, density, rhythm, scale
contrast, layer activity). Temporal traits are described, not animated. `--json`
prints the ResolvedStyleProfile instead of the summary. It is a design-review
tool, not part of the public contract.

## 7. ReferenceStyleProfile (0.7)

The public `ReferenceStyleProfile` v0.1 contract is interpreted externally from
the evidence bundle described in `REFERENCE_EVIDENCE.md`. Its optional,
closed-vocabulary traits are normalized into internal `ReferencePrinciples`;
the CLI accepts a profile with `--reference-style` on `compile` and
`plan-assets`, and `resolve-style` previews resolution and coverage. Traits
below confidence 0.5 are not applied. Explicit StyleProfile choices win over
reference principles, which fill open dimensions before tone defaults.

A reference transfers **principles** (dark technical grid, geometric wipes,
progressive rhythm, large scale contrast), never **frames**: no layouts, images,
copy, footage or per-shot geometry are ever copied. Material and image-treatment
principles propagate through compilation; exact source RGB values are not
copied, and approximate palette evidence guides nearest curated accents. Audio
analysis is limited to the presence of an audio stream. See
`REFERENCE_STYLE_PROFILE.md` for schema and coverage states.

**Visual language (0.7.1).** A v0.2 profile may also carry a `visual_language` (what fills the frame). It rides on the resolved profile (`ResolvedStyleProfile.visual`) and is read by grammar selection and asset planning, but it is not part of `StyleFingerprint`: style is how a piece looks and moves, construction is what it is made of, so two outputs with the same fingerprint can use different compositions. TasteDirector resolvers do not read it. See `VISUAL_LANGUAGE.md`.

## 8. Example: three tastes, one story

`examples/taste/*.style.json`, all valid `StyleProfile` files:

```json
// A: warm_editorial.style.json
{ "tone": "editorial", "polarity": "light", "temperature": "warm", "temperament": "restrained", "density": "balanced" }
// B: dark_technical.style.json
{ "tone": "technical", "polarity": "dark", "temperature": "cool", "temperament": "balanced", "density": "balanced" }
// C: playful_print.style.json
{ "tone": "playful", "polarity": "light", "temperature": "warm", "temperament": "energetic", "density": "dense" }
```

| | A editorial | B technical | C playful |
|---|---|---|---|
| palette / background | warm_paper / paper_field | dark_cool / technical_grid | print_bright / print_fields |
| typography | serif_sans | condensed_mono | poster_bold |
| motion | restrained: small, soft, no bounce | precise: crisp settles, structured stagger | energetic: big, springy, cascading |
| transitions | editorial: lift exit, wipes on impact only | geometric: sliding panel wipes | kinetic: two-bar sweeps, punch exits |
| density / furniture | balanced / folio + rule | balanced / ticks + data label | dense / numerals + stickers |
| rhythm / scale | measured_editorial / large | progressive / moderate | active / dramatic |

`compare-styles` on any two of them reports 10-11 of 11 dimensions different (A
and C share polarity). Changing only `accent_role` on one of them reports 0.

## 9. Future audio choreography contract

> **Nothing in this section is implemented.** There is no SpeechMap,
> MusicMap, SoundDesignMap, ChoreographyEngine, audio input, or CLI flag for any
> of it. This is the contract 0.6 leaves room for, so audio can arrive above the
> compiler without changing the taste tables.

**Separation of concerns**

- TasteDirector (0.6) = **HOW motion feels**: character of each event, family of
  handoff, how many events a beat tends to carry.
- Choreography (future) = **WHEN events happen**: which moment an event lands on,
  given speech and music.
- A style never contains absolute times. Audio never chooses palette,
  typography, background or transition *family*.

**Inputs the future engine would consume**

| input | content |
|---|---|
| SpeechMap | word and phrase timings, emphasis |
| MusicMap | beats, downbeats, sections, energy |
| SoundDesignMap | hits, whooshes and other one-shot cues |
| SceneLifecycle | candidate event slots: EVOLVE events (`evolve_events(n)`) and the BRIDGE between beats, already produced by `motion::lifecycle` |
| taste | CompositionRhythm, MotionTemperament, TransitionCharacter (semantic enums, read-only) |

**How the taste dimensions would combine**

- **CompositionRhythm = event-density bias.** It says how many placeable events
  a beat carries and how tight the tolerance window around an audio anchor is
  (a `high_frequency` rhythm accepts many small, close events; `slow_breathing`
  few, generous ones). Today it already sets the reframe count and READ share
  (section 4); choreography would derive the *window* from the same value.
- **MotionTemperament = character of each event.** Whatever moment an event is
  placed at, temperament decides how it moves: amplitude, settle, overshoot,
  anticipation. Choreography moves the event; it never re-shapes it.
- **TransitionCharacter = family of handoff.** A geometric style bridges with a
  panel wipe, a kinetic one with a bar sweep and punch, whichever downbeat the
  bridge is snapped to.

**Mechanism (design, not code).** A ChoreographyEngine takes the candidate
event slots that SceneLifecycle already schedules (EVOLVE events and bridges)
and snaps or places each near an audio anchor, within a tolerance window
derived from the rhythm. Slots keep their lifecycle order and phase rules
(nothing new at or after ANTICIPATE); anchors only shift *when* inside the
permitted range. The output is retimed events, not new semantics. Audio would
enter as inputs *above* CreativeIntent/StyleProfile, as the North Star already
promises; nothing below the compiler needs to change, and the tables in
`temporal.rs` are never edited by choreography.

**Example (hypothetical numbers).** Style: `high_frequency` rhythm. MusicMap: a
downbeat at 4.22 s. SceneLifecycle: beat 2 has an EVOLVE slot for a hierarchy
reframe at 3.9 s (and a bridge to beat 3 whose exit starts at 4.5 s). The
tolerance window derived from `high_frequency` covers the 0.32 s gap, so the
ChoreographyEngine places the hierarchy reframe (or, if the bridge slot is
closer to the anchor, the transition) at 4.22 s. The reframe still uses the style's temperament (say springy, high
overshoot) and the transition still uses its family (say kinetic punch): only the
time moved.
