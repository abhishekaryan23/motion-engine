# Genre grammars (0.14)

Weak models pick a genre with one taste field: `tone: "street" | "documentary" | "hype" | "studio" | "cinematic"`.
Under `--art auto` (the `reel` default) the tone selects an art-direction look
(`street_collage`, `dossier`, `hype_slam`, `studio_pop`, `cinematic_3d`) and that look's grammar takes over atomic
beats (`grammar::genre_bias`). Structured beats (series, collections, state changes,
derived metrics) keep their semantic grammars. The finishing effects (post, shakes,
echoes, pulses) come from the FX director (`compiler::fx`), not from the builders.

Canvas units: `u` = 1.0 at a 1080 px short side. All builders respect the caption lane
(`ctx.frame` already excludes it when captions are on), the safe area, and the layout QA
budgets. Layer ids follow `b<N>.<name>`. Background typography uses an id containing
`ghost` so layout QA treats it as decorative. Every random choice is seeded from
`b.plan.seed`.

## HeroVisual (`grammar/hero_visual.rs`) — street, music, sports

The picture owns the frame, type plays behind it like a wheat-pasted poster.

| element | spec |
|---|---|
| picture | delivered subject image > delivered object image > library object (`ctx.object_asset`). Scaled to 72–80 % of the layout height (≤ 88 % width), centred, bottom on the layout's content bottom, rotated −3°..3°. Enters at ENTER: scale 1.25 → 1 and +140u → 0 rise with spring (stiffness 300, damping 18), 0.6 s. |
| stencil punchword | primary phrase value (keyword if the primary is an object), uppercased, display role, fitted to 92 % width in 1–2 lines, block ≤ 34 % of the layout height, centre 30–38 % from the top, **behind** the picture (`b<N>.ghost_punch`). Colour: palette accent. Enters via `glyph_cascade` (stagger 0.035, from dy −120, scale 1.6, rotation 8, opacity 0, order random, spring stiffness 420 damping 22). |
| bouquet | the secondary picture (if it is an object/picture) at 28–34 % height, opposite side to the hero's lean, rotated ±8°, pops 0.25 s after the hero (scale 0 → 1, spring). |
| tape | two strips 260×54u in a palette field colour (accent when none) at 0.92 opacity, rotated 18–28° (opposite signs), crossing the picture's top-left and bottom-right corners; `clip_reveal` along their length at SETTLE. |
| badge | the keyword in a paper sticker (rounded rect, ink text, −6°) beside the picture's upper third; pops at EVOLVE (word cues move it to the spoken keyword). |
| statement | with speech: not drawn (captions carry it). Without speech: the display title as a 2-line strap at the top in the body face. |

## KineticSlam (`grammar/kinetic_slam.rs`) — hype openers

The beat becomes a run of hard cuts; each cut slams one or two words (or the picture) into frame.

| element | spec |
|---|---|
| units | with speech: the beat's spoken content words (skip stop words), merged into 1–2-word units so every unit lasts ≥ 0.4 s; the primary value and keyword are always their own units; numbers stay whole. Without speech: the display title split into 1–2-word units of 0.6–1.0 s (syllable-weighted) filling the beat. |
| cut layouts | rotate by unit index: A giant word centred (display face, ≤ 90 % width, ink on paper); B inverse field: full-bleed accent rectangle + word in `on_accent`; C word + the beat's picture at 55 % height behind it. |
| timing | each unit is a group visible from its cut time until the next cut (hard cut: opacity steps, no crossfade). Word: scale 1.8 → 1 with spring (stiffness 520, damping 26) over 0.22 s; words ≤ 8 chars also get `glyph_cascade` (stagger 0.02, from scale 1.4, opacity 0). |
| last cut | holds to the scene end and carries the beat's handoff. |

## DocumentaryDossier (`grammar/dossier.rs`) — Vox-style explainers

Evidence lands on a desk: documents, a photo, a stamp, a highlighter.

| element | spec |
|---|---|
| desk | the look plate (+ delivered environment image dimmed to 35 %). |
| clipping | a paper card (palette surface, soft shadow, rotated −2..2°) with the display title in the serif headline (≤ 4 lines) and 3–5 seeded redaction bars (ink rectangles) as body copy. |
| photo | the beat's picture on a second card, taped (one tape piece), rotated 3–6°, offset right/down. |
| figure | if the primary value contains a digit (or a currency sign): a third card with the value in the serif display at large size, its meaning as a mono label, and a red polyline underline that draws (`trim`) at EVOLVE. |
| motion | cards slide in from below/right with spring (stiffness 260, damping 24), staggered 0.18 s; at EVOLVE a stamp — the keyword uppercased in an accent-stroked rectangle (stroke 6u, rotated −8°) — slams from scale 2.4 to 1 in 0.18 s. A highlighter (accent at 0.55 opacity, or a field colour) grows left → right behind the keyword's words in the clipping headline (word cues land it on the spoken keyword). |
| 3D (0.16) | cards carry `z` 0 / 80 / 160 and `tilt` [6, −4]; the FX director adds the perspective camera. |

## FX director (`compiler/fx.rs`)

See the module documentation: grain, vignette, chromatic flashes on hits, glitch on cuts,
camera shake by energy, echo trails on hero entrances, audio pulse on hero layers, bloom,
and (0.16) the perspective camera. Effects are scene data (`Scene.post`, camera ops,
motions), so every renderer draws them the same way.

## HeroVisual variants

| variant | when | staging |
|---|---|---|
| street | street look, dark palette | environment photo full bleed, stencil punchword behind the picture, tape strips, keyword tag, sticker border |
| poster | street look, light palette | paper ground with a radial accent web drawing on, the punchword on a red tape banner above everything, a keyword starburst; with an object and a person the object drops from the top and the person stands at the bottom |
| disc | studio look | clean studio ground, a bold disc (the palette's first field) behind the picture's upper body, floor shadow, picture at 78 % height overlapping the words |

## Kinetic spoken words (studio)

The FX director places the beat's spoken words as big type around the hero's head, between the
disc (z 8) and the picture (z 20): phrases of up to three words (breaking at punctuation and pauses),
content words large and function words small, each popping in on a spring at its spoken time; the
phrase clears when the next one starts and before the outgoing transition. The look turns the caption
track off.

## Cinematic3d (0.17)

`tone: "cinematic"` → look `cinematic_3d` (four deep palettes, one light). Each atomic beat is a
stack of depth planes, every one a top-level layer with its own `z` (px, positive = farther):

| plane | z | content |
|---|---|---|
| glow | 2200 | three huge soft discs in the palette's field colours |
| ghost word | 1300 | the beat's keyword, giant, 14 % opacity |
| dust | 300–1600 (a few at −300–0) | 16 seeded motes |
| prop | 500 | the `secondary` object (role SupportingObject), 34 % width, left or right |
| hero | 0 | the `primary` object or delivered cutout (HeroSubject / Portrait / HeroObject) in 86 × 54 % of the canvas, spring scale 0.82 → 1; without a picture a glyph-cascade hero word |
| title | −120 | the statement, glyph cascade, ≤ 3 lines |
| bokeh | −700 | three soft foreground discs |

The FX director's `choreograph()` then writes the camera (fov 40, aperture 9):

1. **Fly-in** (first 25 % of the beat, ≤ 0.75 s): dolly from 900 px behind the start mark,
   OutQuint, while focus racks from the ghost word to the hero plane.
2. **Main move** (seeded per beat): push-in with a gentle orbit, a truck with a small yaw, a
   crane (pitch 11° → −2°), or a wide 18° orbit. Dolly and focus move together, so the hero stays
   sharp and every other plane gets its own parallax and blur.
3. **Fly-through** (last 0.55 s, not on the last beat): dolly +1700 px InCubic with a light-rays
   zoom burst; the next beat opens with the burst fading out.

Depth compensation: every layer's position and scale are pre-multiplied by
`k = (z + f − d_ref) / f` about the canvas centre (`d_ref` = mid dolly of the main move), so the
layout seen during the main move is the one the builder laid out. The documentary look uses the
same choreography in a gentle form (amplitude 0.45, push-in only, aperture 5).

## SphereGallery (0.18)

`grammar/sphere_gallery.rs`, name `sphere_gallery`. Chosen in `genre_bias` when the look is
`cinematic_3d` and the beat's `primary` is a `collection` of **3–6 items** (whatever shape it would
otherwise take, a numeric series included); two items and every other look keep today's grammar.
The items ride a slowly turning sphere; the sphere turns to bring each item to the front, one after
another, where it is sharp (depth of field), 1:1 and slightly enlarged with its label, while the rest
are smaller, blurred and pass behind. Example: `examples/cinematic/ai_toolkit.intent.json`.

| layer | id (`b<N>.…`) | content |
|---|---|---|
| glow, bokeh | `glow.i`, `bokeh.i` | the cinematic background (`cinematic3d::glow_discs`, `bokeh_discs`) |
| title | `title` | the statement, at most 3 lines in ≤ 80 % of the width, `z = −120`, glyph cascade |
| item `k` | `item.k` | an object picture (front size 0.40 × the short side; library file, else the best catalog match of the look's asset families) or a rounded text card (phrase / number, ink on the palette card, at 0.64 of the card width; an object with no picture anywhere is a card of its meaning) |
| halo | `dust.j` | 48 dots (12–34u, accent / ink) on a Fibonacci sphere of 0.94 R, riding the same rotation; the id makes layout QA treat them as decorative |
| label | `sphere_label.k` | the item's meaning (objects; a number / phrase card with a meaning) in a pill under the front, `z = 0`; fades in while the item is in front and out before the next turn |

**Sphere.** Radius `R = 0.32 × short side`, centre `(w/2, cy)` below the title and a little below the
middle of the free band (`ctx.frame.safe`, which already excludes the caption lane); in a band that is
too short everything shrinks (down to 0.6). Every item and dot is a top-level stage child placed at
the centre (anchor 0.5 / 0.5) with `z = R`, all with the same `z_index`, so the front point is `z = 0`
(the focus plane, 1:1) and the FX director's `depth_sort` draws the sphere far to near. The beat has
**no `focal` record**: the front of the sphere is the focus plane by construction.

**Rotation (`revolve`).** Item `k` sits at `at = [lon_k, lat_k]`, `lon_k = k · 360 / n`, `lat_k` a
seeded zig-zag within ±25° (`plan.seed`). Angles `[−lon_k, −lat_k]` bring item `k` to the front. The
sphere is rigid, so every layer carries the same chain, each with its own `at` and radius:

1. *Intro* (ENTER): from `[−lon_0 − 140, −lat_0 + 15]` to item 0's front, ≈ 1.2 s, OutCubic.
2. READ..ANTICIPATE is divided evenly into `n` dwells. At the start of dwell `k ≥ 1` the sphere turns
   from item `k−1`'s angles to item `k`'s, `min(0.8 s, half the dwell)`, InOutCubic, the short way
   round in yaw (the delta is normalised to (−180°, 180°]).
3. An item scales 1 → 1.12 (spring) when it arrives and back to 1 as it leaves; the label fades in
   0.03 s after the arrival and out before the next turn.

`speech_plan::apply_word_cues` never retimes this: a group that carries a `revolve` motion is skipped
and `sphere_label` is a fixed group, so the dwells stay on the lifecycle.

**FX director.** In `choreograph`, a scene with any `revolve` motion gets `perspective.depth_sort`,
the calm push (pick 0) instead of the seeded vocabulary, and layers that carry a revolve are
compensated as if `z = 0` (their front position; they sit on the canvas centre column, so only the
scale factor matters). `hoist_depth_layers` hands a revolving stage child's motions to its `.depth`
wrapper (the timeline applies `revolve`'s depth to top-level layers only).

**Layout QA.** The sphere stays inside the canvas and the text of a card inside the safe area; the
halo is decorative. No exemption is needed: back-of-sphere items are small and blurred by design but
pass the checks (cards keep their text at 0.64 of the card width so a card on the flank stays inside
the safe area).

