# Visual language (0.7.1)

A reference teaches two things:

- **Style** (0.7): how it looks and how motion feels — palette, typography, material, temperament, transitions, rhythm.
- **Visual construction language** (0.7.1): *what fills the frame* — how much imagery or objects carry the story, what kind, which composition families dominate, how ideas are explained visually.

CreativeIntent still decides **what the story means**. The visual language only biases **which kind of scene** the engine builds for a beat whose meaning can be shown in more than one way. Source: `compiler/visual.rs` (semantics), `grammar::select` (bias), `grammar/diagram.rs` (entity depiction), `compiler/asset_plan.rs` (planner), `motion-render/src/visual_qa.rs` (QA).

## 1. Contract

`ReferenceStyleProfile` **v0.2** = v0.1 + optional `visual_language` (`schema/reference-style-profile-v0.2.schema.json`). v0.1 documents are still accepted unchanged (`schema/reference-style-profile-v0.1.schema.json` is frozen); a v0.1 document may not carry `visual_language`.

| field | values |
|---|---|
| `medium` | type_only, type_led, image_led, object_led, diagrammatic, collage, interface_led, mixed |
| `asset_usage` | none, sparse, balanced, dense |
| `type_image_balance` | type_dominant, balanced, visual_dominant |
| `asset_character` | photographic, cutout, illustrative, diagrammatic, object_centric, collage, interface, procedural, mixed |
| `asset_roles` | ≤ 4 × `{role, confidence}`; roles = the engine's AssetRole names |
| `composition_language` | `{preferred ≤ 3, secondary ≤ 3, avoid ≤ 3, confidence}` over the 10 engine grammars, disjoint |
| `explanation_mode` | literal, diagrammatic, symbolic, metaphorical, evidence_based, mixed |

Traits are `{value | null, confidence, evidence?}` like every other profile dimension; confidence < 0.5 is recorded, not applied. No free text, coordinates, layouts, source text, objects, characters, brands or images.

Normalization is 1:1 into the internal `compiler::visual::VisualLanguage` (carried on `ReferencePrinciples.visual` → `ResolvedStyleProfile.visual`). No StyleProfile field exists for it; without a reference it is **neutral** and every compiler path is the pre-0.7.1 one (byte-identical goldens).

## 2. Derived policy (`VisualLanguage`)

- `weight()`: **Type** (medium type_only/type_led; or usage none / balance type_dominant), **Visual** (medium image_led/object_led/diagrammatic/collage/interface_led; or balance visual_dominant / usage dense; or mixed + usage balanced), else **Neutral**.
- `prefers_procedural()`: Visual and (medium object_led/diagrammatic, or character diagrammatic/procedural/object_centric, or explanation diagrammatic). Visual explanation should be built from engine-made shapes, not requested images.
- `wants_images()`: Visual, not procedural, medium image_led/collage/interface_led/mixed, usage ≠ none, character photographic/cutout/illustrative/collage/interface/mixed (or unknown).
- `preference(g)`: preferred +2, secondary +1, avoid −3.

## 3. Composition bias (`grammar::select` → `visual_bias`)

Meaning first. The semantic table (docs/COMPOSITION_GRAMMAR.md) picks `s`. The bias returns `s` unchanged when:
the weight is Neutral · `s.shape` is not Atomic (collections, state changes, derived metrics, number pairs/series) · any subject is a number or an object · `s` is TypeImageInterlock.

Otherwise (atomic **phrase** beats) candidates and semantic fit:

| purpose | candidates (fit) |
|---|---|
| emphasize | `s` 3 · EditorialCollage 2 · KineticPoster 2 · CinematicMultiplane 2 · HeroObject 2 |
| reveal | `s` 3 · HeroObject 2 · CinematicMultiplane 1 |
| contrast / compare | `s` 3 · the other of {SpatialCauseEffect, SplitContrast} 2 |
| explain | `s` 3 · SpatialCauseEffect 2 (only with a secondary subject) |

(a candidate equal to `s` is listed once, with fit 3)

score(g) = fit + `preference(g)` + medium term + explanation term:
- **Visual**: entity-capable grammars E = {HeroObject, SpatialCauseEffect} get +2 when the beat has a secondary subject, or `prefers_procedural()`, or a delivered `hero_object` image exists; CinematicMultiplane +1 (so a single strong phrase still ties with its semantic KineticPoster/collage and keeps it).
- **Type**: KineticPoster, EditorialCollage, SplitContrast +1.
- **explanation**: diagrammatic → SpatialCauseEffect +1, SequentialStack +1; literal → HeroObject +1; symbolic / metaphorical → CinematicMultiplane +1, EditorialCollage +1.

Highest score wins; ties keep `s`, then the table order. **Depiction** = `Entities` iff weight Visual and the winner ∈ E (phrase primary); otherwise `Typographic`. Reason strings name the visual language.

Consequences: a type-led reference keeps typography (ties go to the semantic choice); a single strong phrase in an image-led reference stays KineticPoster/collage unless a hero image is delivered or the reference lists HeroObject among its preferred families (a diagrammatic/object-led reference shows it as an entity); a two-entity relationship in any visual language is shown as entities; data stays data.

## 4. Asset planning (`plan_assets_with_reference`)

Same selection as compile (same resolved timing, language and visual language).

1. Data, structure and object rules are unchanged (0.4/0.5 rules 1–5).
2. **Entities** beats: decision `procedural`, reason "visual language: engine-drawn entity tokens", no external request — **except** when `wants_images()` and the grammar is HeroObject: one `optional` `generated_image` request for the primary (`hero_subject` if the phrase is human by the 0.5 word list, else `hero_object`), `isolated_cutout`, transparent. If it is delivered, the hero token is that image; if not, the procedural token stays.
3. **Typographic** emphasize beats with `wants_images()` and a phrase primary: one `optional` `generated_image` request (`hero_subject` for human phrases as before; `hero_object` otherwise), `isolated_cutout`, transparent. A delivered `hero_object` makes `select` choose HeroObject + Entities with the image as hero.
4. Type weight or Neutral: no new requests.

Source priority stays `none < procedural < svg < user_asset < generated_image` (strongest request names the decision). Procedural is preferred whenever `prefers_procedural()`: no generated image is requested for any beat in that case (the engine draws).

## 5. Entity depiction (`grammar/diagram.rs`)

What the viewer **sees** instead of a text card. Built from existing primitives only (rounded rectangles, polylines + trim, text, groups, existing motions); no new MotionOp, no renderer change.

- **Entity token**: one per phrase subject. Identity from the normalized phrase (lowercase words): shape family by hash (disc, capsule, hexagon, diamond, rounded square); the same phrase gets the same shape and colors in every beat. Primary = accent body + ink outline; secondary = card body + ink outline + accent inner mark. Label = the phrase value in a small label voice under the token.
- **Process track**: two parallel rails across the frame at a fixed height (same geometry in every Entities beat, so it reads as one persistent process), drawn with trim in ENTER, plus an accent progress line advancing from `index/n` to `(index+1)/n` of the piece during READ/EVOLVE.
- **Relationship → operation** (EVOLVE events; nothing new at/after ANTICIPATE):
  - HeroObject: primary hero token on the track; with a secondary, a smaller token/marker appears ahead on the track, the hero approaches it — calm energy halts short with a small recoil, building/impact passes over it. Reveal beats uncover the hero (mask reveal).
  - SpatialCauseEffect: `replace` — the primary lifts off the slot (dims, shrinks), the secondary drops into the same slot, then advances along the track; `compress` — the secondary is a pressure zone that grows toward the primary, which shrinks and yields; `grow` — the secondary grows; `separate` — tokens drift apart; `carry` — the primary travels along the track carrying the secondary; otherwise both tokens sit on the track and a connector draws between them.
- The statement stays as a smaller headline (supporting type); the motion language and temperament set travel, easing and settle like every other builder.

## 6. Typography-dominance QA (`motion-engine qa --reference-style`)

Structural, from the MotionScene (no OCR, no metadata). Per beat scene: text area (text layers, excluding faint ghost words with base opacity ≤ 0.25) versus figure area (image and svg layers; polylines; rectangles at object scale — ≤ 15 % of the canvas — that are not text containers and not thin rules; larger plain fields are layout), measured on the timeline-resolved frame at mid READ→ANTICIPATE (what is visible: fades, scales and shared elements included). A beat is **type-dominant** when it has no image/svg layer and its figure share `figure / (figure + text)` is below 0.25. Reported: per-beat share and counts, `pure_type_ratio`.

Against the reference visual language: weight Visual and `pure_type_ratio > 0.6` → WARNING "visual-language transfer likely failed"; weight Type and `pure_type_ratio < 0.4` → WARNING "type-led reference but the output is visual-heavy". Neutral → no verdict. Type-dominant output is never bad by itself.

## 7. Limits

No new CreativeIntent field: entities are the intent's own phrase subjects and relationships. A process whose steps are not expressed as subjects/relationships cannot be visualized beyond them. Tokens are abstract (symbolic) shapes, not illustrations; image-led references get images only through the external generator protocol.
