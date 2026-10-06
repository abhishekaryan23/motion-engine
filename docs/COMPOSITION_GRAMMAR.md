# Composition grammars (0.4, subject-aware since 0.5)

A **composition grammar** is a reusable spatial strategy: what dominates the
frame, how elements relate, which depth planes they occupy. It is chosen from a
beat's **semantic structure** (subject kinds, purpose, relationship, resolved
motion language, delivered assets) — never from its topic. There is no
"finance template" and no public knob: CreativeIntent and StyleProfile are
unchanged.

```
beat structure ─→ grammar::select ─→ Composition {grammar, variant, shape, reason}
                                          │
motion language (how it moves) ───────────┤
lifecycle (when each piece arrives) ──────┴→ builder ─→ layers + motions
```

## Selection (`compiler::grammar::select`, first match wins)

| beat structure | grammar | shape |
|---|---|---|
| collection of ≥ 3 countable numbers, relationship ≠ accumulate | DataStory | NumberSeries |
| collection (other) | SequentialStack | Structured(Collection) |
| state_change (single or dual) | SplitContrast | Structured(StateChange) |
| derived_metric | DataStory | Structured(DerivedMetric) |
| layers (primary) | LayerStack | Structured(Layers) |
| reveal / explain with an object subject | EvidenceStack | Atomic |
| object primary | HeroObject | Atomic |
| contrast / compare, both subjects countable numbers | DataStory | NumberPair |
| contrast / compare with compress · grow · accumulate (or contrast with no relationship) | SpatialCauseEffect | Atomic |
| contrast / compare otherwise (separate · replace · carry · compare default) | SplitContrast | Atomic |
| explain | SequentialStack | Atomic |
| reveal of a countable number | DataStory | Atomic |
| reveal (other) | KineticPoster | Atomic |
| emphasize with a delivered subject image (AssetManifest) | TypeImageInterlock | Atomic |
| emphasize, kinetic language | KineticPoster | Atomic |
| emphasize, parallax language | CinematicMultiplane | Atomic |
| emphasize (other) | EditorialCollage | Atomic |

## Variants (`variant_for`, deterministic)

Format constrains, the beat seed (`mix(style.seed, beat)`) chooses:
- SplitContrast: landscape → LeftRight; square → LeftRight / CenterOpposition; vertical → TopBottom / CenterOpposition.
- EditorialCollage, HeroObject, CinematicMultiplane, KineticPoster, TypeImageInterlock, EvidenceStack: Standard / Mirror (asymmetry flipped left↔right).
- Others: Standard.

## Builders

`type Builder = fn(&mut Ctx, &mut B, &mut Vec<Carry>, Composition) -> Result<(), CompileError>`,
one file per grammar under `compiler/grammar/`. Existing recipes are the
builders of the grammars they already embody:

| grammar | builder |
|---|---|
| EditorialCollage | `recipes::emphasize` (collage card, halftone, ghost, serif) |
| SpatialCauseEffect | `recipes::contrast` (panel + zone; compress / grow / accumulate transform the space) |
| SequentialStack | `collection::compose` (structured), `recipes::explain` (atomic) |
| SplitContrast | `state_change::compose` (structured), `grammar/split_contrast.rs` (atomic) |
| DataStory | `derived::compose` (structured), `grammar/data_story.rs` (NumberPair, NumberSeries, number reveal) |
| KineticPoster | `grammar/kinetic_poster.rs` |
| HeroObject | `grammar/hero_object.rs` |
| CinematicMultiplane | `grammar/multiplane.rs` |
| EvidenceStack | `grammar/evidence_stack.rs` |
| TypeImageInterlock | `grammar/type_image.rs` (+ `grammar/plate.rs`) |

### Builder contract
- **Subjects go through `recipes::place_subject`** (atomic primary/secondary), so continuity (`carry_*`) keeps working: carried-in subjects get track keys, carried-forward ones become SharedElements. Push the returned layer only when it is `Some`.
- Set `b.anchor` (where carried-in subjects this beat doesn't mention park).
- Reuse the skeleton helpers: `recipes::{ghost, headline_lines, absorb, underline, collage_card, subject_layer, with_ink, centered_in, plane, find_word, shared_pulse}`; the kicker, carry closing and stage wrapper/exit are added by `build_beat`.
- Follow the lifecycle scheduling contract (docs/SCENE_LIFECYCLE.md): primary from `b.plan.life.enter`, secondary at `b.plan.life.evolve_events(n)`, nothing new at/after `life.anticipate`.
- Depth planes: `plane(b.plan.lang.depth.{background, midground, foreground})`; z bands: 0 ghost/background · 6–9 plates/halftone · 10–14 cards/panels · 20–26 type · 30 kicker (added) · 45 shared.
- Existing operators only (`motion::kinetic::*`, `motion::dataviz::*`, `mo::*`); no new MotionOps.
- Ids via `b.id("name")`; deterministic; no HashMap iteration in output.
- Text measured through `ctx.ts` (`fit_block`, `fit_line`, `text_run`), sized in `ctx.u` units; margins `ctx.margin()`.

## The grammars

**EditorialCollage** — oversized type, intentional overlap, asymmetric placement, paper/halftone layers, negative space; optional image region (placeholder plate when no image).

**TypeImageInterlock** — the subject image and the headline behave as one composition. Since 0.5 the layout is chosen by subject-aware placement (below) from the image's analyzed geometry: the head rises into the last line (Split), the upper body stands in front of the headline (Behind), the headline crosses the torso in front on paper strips (Front), or the headline owns the negative space beside the subject (Beside). A headline line that touches the head is always *behind* the subject; supporting items go in a reserved free zone. Uses a delivered image when the AssetManifest has `beat_<n>.hero_subject` / `portrait`; selected only when one exists (EditorialCollage / KineticPoster / CinematicMultiplane are the image-free fallbacks). The collage's "image variant" *is* this grammar.

**HeroObject** — one object dominates (≈ 55 % of the short side), persists, receives the lifecycle: arrives, settles, slow scale drift in READ, secondary label/serif at EVOLVE. Works with SharedElements (carry).

**SplitContrast** — two states share the frame (LeftRight / TopBottom / CenterOpposition): two panels of contrasting tone, a divider rule that draws; primary side arrives first (ENTER), the second side at EVOLVE; `replace` → the first side's phrase TypeReplaces into the second's; `separate` → the halves drift apart; `carry` → the primary crosses the divider.

**EvidenceStack** — a stack of evidence: object/document card (tilted), a source label (kicker-like mono line), a highlight bar that sweeps over the key phrase, the number if any. Items stack progressively at EVOLVE events.

**DataStory** — numbers reveal their reasoning. NumberPair → both counters then a ComparisonBar at EVOLVE and a direction line; NumberSeries → BarChart grows (items as bars), then PathDraw traces the trend through bar tops, then the last/peak value is emphasized; number reveal → Counter on an accent slab, then the meaning line at EVOLVE.

**SequentialStack** — ordered information that evolves: items one per EVOLVE event, earlier items dim slightly as new ones arrive, the aggregate/consequence last.

**SpatialCauseEffect** — one element physically influences another: compress (panel shrinks, the other grows toward it), grow, separate, replace, accumulate — the transformation plays in EVOLVE.

**CinematicMultiplane** — foreground / midground / background planes (depth 1.45 / 0.7 / 0.35) under a pushing, drifting camera; the headline on the subject plane, a large ghost/halftone field behind, a foreground occluder bar or plate crossing the frame; midground reveals the secondary at EVOLVE.

**LayerStack** (0.19) — a story about layers (zones, levels, stages). The layers are drawn once by the engine as full-width strata in the BACKDROP scene (`compiler/layer_stack.rs`), shared by every beat of a *run* (consecutive beats whose `layers` list is the same). Each layer is a gradient band from the palette (a thin `boundary` is a bright seam) with a dark label card (name over note); the beat's `focus` layer is lit and sits at the same canvas y (60 % of the content canvas) in every beat, so between beats the column glides vertically (`move`, 0.9–1.4 s) while veils, seams and label cards fade to the new emphasis. A layer above the lit one is named at its bottom edge, the others at their top, and a card that would fall under the headline is hidden for that beat. The column never zooms (labels stay at the margin). The beat scene (`grammar/layer_stack.rs`) adds the headline and pins the `secondary` subject inside the lit layer on the right half, with its meaning as a caption; it arrives from this beat's side with a 3D turn (the `tilt` op) once the column has settled. A layers scene has no camera motion (no shake, push, orbit or fly-through; a still perspective camera under looks that have one, no depth of field), so what it pins stays on its layer. `relationship: separate` lights all the seams. The column fades in with the first beat of a run and out after its last beat unless that is the end of the piece.

**KineticPoster** — typography is the dominant visual: oversized display text with serif contrast, WordCascade/TrackingReveal entrance, keyword manipulation at EVOLVE (TypeScaleEmphasis / KeywordPunch), TypeReplace when the beat replaces one phrase by another, a secondary phrase after.

## Visual-language bias (0.7.1)

When a `ReferenceStyleProfile` v0.2 carries a `visual_language`, `select` may re-score the semantic choice for atomic phrase beats only (structured, number and object beats never change; ties keep the semantic choice). The result also carries a `Depiction`: `Typographic` (as before) or `Entities`, where phrase subjects are drawn as identity-stable procedural tokens on a process track with the relationship shown as an operation (`grammar/diagram.rs`, HeroObject and SpatialCauseEffect). Scoring and depiction rules: [VISUAL_LANGUAGE.md](VISUAL_LANGUAGE.md) section 3 and section 5. Without a reference nothing here applies.

## Subject-aware composition (0.5)

Generated images are not rectangles to fill. A delivered image carries its
analysis (`ManifestEntry.analysis`: alpha bounds, 16×16 occupancy, head
region — docs/ASSET_ANALYSIS.md), and `grammar/placement.rs` turns it into
**internal layout candidates** that are scored deterministically; the lowest
cost wins, ties keep generation order. None of this is public: CreativeIntent
has no layout field and never will.

```
analysis (normalized) ─→ SubjectFacts {aspect, subject, head, occupancy, bottom_cut}
measured headline    ─→ candidates {mode, side, image rect, lines + layering, support zone}
                            │ score (cost terms below)
                            ▼
                     best candidate ─→ builder emits layers + motions
```

Candidates (TypeImageInterlock, vertical): for each side (the variant's
preferred side first) — `split`, `behind`, `front` (two subject scales),
`beside` (two scales). A half figure (subject touches the image bottom)
always bleeds off the canvas bottom; a whole figure stands above the margin.
`split` is also built as a `split_end` variant (a narrower type column that
leaves room for the head at the tail of the last line); both are `split` mode
for scoring.
Every candidate reserves a support zone (primary label + secondary note) on
its type side, below the headline if it fits, else above it.

Cost terms (`placement::w`, lower is better):

| term | weight | meaning |
|---|---|---|
| face_collision | 600 × head area covered | a *front* line over the head (never intended) |
| back_occlusion | 60 × occluded, or 180 + … above 30 % | a line behind the subject must stay readable |
| subject_collision | 10 × body coverage | front lines over the body (paper strips keep them legible) |
| text_clip | 400 × fraction outside the live area | |
| head_clip | 600 × fraction outside the canvas | |
| subject_side_clip | 120 × fraction cut left/right | |
| floating_cut | 150 | a half figure whose cut edge is visible mid-frame |
| feet_clip | 100 × fraction | a whole figure cut by the canvas bottom |
| balance | 300 × distance outside [0.03, 0.18] | mass centroid off-centre, but not extreme (controlled asymmetry) |
| lower_third | 200 × (0.2 − share) | top-heavy layouts / empty lower third |
| dead_band | 250 × (gap − 0.2 h) | the largest empty horizontal band |
| hierarchy | 140 × (1 − size / max) | the headline stays large |
| text_over_subject | 40 × column overlap | beside mode only |
| support_room | 90 none · 25 × subject coverage | supporting items need clean room |
| support_face | 600 × head area covered | supporting type never covers the head |
| side_preference | 12 | variant's side (asymmetry from the beat seed) |
| continuity | 10 | same subject on the other side than before (hook) |
| mode_prior | split 0 · beside 4 · front 6 · behind 8 | documented tie-breaker |

Weights are tuned on rendered frames, not per benchmark. `MOTION_DEBUG_LAYOUT=1`
prints every candidate's terms to stderr (output is unaffected).

Treatment, depth and time: the image gets the style's treatment
(docs/IMAGE_TREATMENTS.md), sits on a slightly nearer plane than the type
(`1 + 0.35·(foreground − 1)`, so camera push yields subtle parallax), a
halftone field on the background plane behind the far shoulder or the type
side, and an almost-static push (1 → 1.025) through READ/EVOLVE — never a
constant Ken Burns.

**Asset carry.** When consecutive beats are both TypeImageInterlock and are
served by the *same* manifest entry (one generated image, one continuity
key), the image becomes one SharedElement keyed in both scenes (it travels to
the next layout with the entrance). The stage is then split into a back and a
front group with identical anticipation/exit motions so the shared image still
sits between back and front lines. Accent-wipe beats don't carry.

## Subject-first image layouts (0.10 Q)

Weak-model reels put type over people, shrink the picture and repeat one layout
(owner review, DECISIONS 85–86). TypeImageInterlock beats can therefore take a
**subject-first** layout (`grammar/type_image.rs`, `ImageLayout`): type and
subject live in **disjoint regions** (nothing printed over the picture, a head
never covered), the subject alpha bounds are as large as the remaining room
allows and never under `SUBJECT_MIN_AREA` (16 / 14 / 12 % of the canvas, tall /
square / wide), and the layout differs from the previous image beat's.

| layout | title | subject |
|---|---|---|
| `TextTopSubjectBelow` | block at the top (left, or right when mirrored) | stands below it, centred |
| `SubjectRightTextLeft` | column top-left | bottom-right (beside the column, or below it when that is larger) |
| `SubjectLeftTextRight` | mirror | bottom-left |
| `SubjectCenterTextBand` | centred band at the bottom, above the camera's lift | centred above it |

`solve` is the geometry (pure; shared by the planner and the builder): for each
headline budget (28 → 12 % of the height) and, for the side layouts, each title
column (50 / 56 / 62 % of the width; 44 / 50 / 56 % on side-by-side canvases) it
sets the headline, reserves the type region (headline, optional body copy of a
derived title, label slot, note room) and stands the subject in the largest
room that does not touch it. The first attempt whose subject reaches 1.35× the
minimum wins, else the first reaching 1.04×, else the biggest subject (the layout
is then reported as not fitting). Headlines are never set under 46 u (the
budget's floor) and are fitted so the text *box* stays inside the column. The
subject keeps off the side edges by the camera's drift and the EVOLVE reframe
(`pan + c·(1 − 1/zoom)`), and a whole figure's feet stay above the bottom edge
by the same lift (never lower than 7 % of the height), so the push never cuts
a limb; a half figure bleeds off the bottom.

`grammar::plan_image_layouts` decides one layout per image beat before any beat
is built (`BeatPlan.image_layout`), on the canvas the builders will see (with a
voice-over: shortened by the caption lane; `BeatPlan.image_min_px` keeps the
minimum area that of the real canvas):

* **`CompileOptions.variety = Some(seed)`** (`compile --variety`): image beats rotate
  `IMAGE_LAYOUT_ROTATION` from `seed % 4`, one step per image beat, skipping the
  previous image beat's layout and any layout that does not fit; side-by-side
  canvases (wide, 6:5, 4:3) try left/right first. A carried image (same entry in
  consecutive beats) simply moves between the layouts' rects.
* **`variety = None`**: the 0.9 interlock stays byte-identical, except that a beat
  whose headline lines (or label slot) would touch the subject's head region, or
  whose subject is under the minimum area, falls back to `TextTopSubjectBelow`
  (the first fitting layout if that does not fit). Front lines on paper strips
  over the body stay (the strip is the label's own backing, see layout QA).

### Frames hug their subject (0.10 Q)

A card behind a subject exceeds its bounds by at most `FRAME_PAD_MAX` (12 %) per
side. HeroObject keeps the round card for near-square objects and uses the image
bounds plus 5 % behind a delivered opaque image the disc would exceed; a delivered
alpha cutout still has no backing (DECISION 69). EvidenceStack's paper sheets are
the evidence's alpha bounds (the whole image when opaque), peeking out by their
offset and tilt only.
