# Art direction & variety (0.10 Q / 0.12)

Owner review of weak-model reels: "the colour theme always stays the same, the
flow always stays the same, the videos feel repetitive". Each tone had exactly
one palette and each purpose exactly one grammar, so five similar beats gave
five identical screens and every story in the same tone looked alike.

Art direction is an **operator** knob (never authored by weak models). The
engine chooses everything from the emotion the five taste fields evoke; the
story text only seeds *which* curated option is used, so the same story always
renders the same and different stories differ.

```
motion-engine compile story.intent.json --style s.json --art auto --variety auto -o out/story.motion.json
motion-engine compile story.intent.json --style s.json --art clay_pop ...    # force a look
```
Without `--art` / `--variety` every output is byte-identical to before.

## Looks (`compiler/art_direction.rs`)

| look | emotions (auto) | families (operator families win) | plate dark / light | treatment | SFX palette |
|---|---|---|---|---|---|
| classical_neon | drama · luxury | classical_greyscale, people_everyday, editorial_cutout, sketch_icons | ink_wash_dark / concrete | greyscale | cinematic |
| halftone_cutout | energy · urgency · playful_retro | halftone_retro_objects, people_everyday, retro_cars_a/b, editorial_cutout, clay_props_3d | halftone_dark / warm_paper | halftone | punchy |
| clay_pop | joy | clay_props_3d, people_everyday, sketch_icons, editorial_cutout | deep_teal / blush | — | bouncy |
| ornament_editorial | trust · warmth · handmade | people_everyday, editorial_cutout, sketch_icons (handmade: woodcut_kitchen first) | ink_wash_dark / warm_paper (trust: deep_teal / linen; handmade: kraft) | — | soft |
| journey | calm · precision | people_everyday, sketch_icons, editorial_cutout, clay_props_3d | black_grain / grey_studio (calm: deep_teal / sky_gradient) | — | crisp |

Every look includes `people_everyday`: human phrases ("student", "elderly
person") need figures, otherwise they all fall back to one generic walker.

**Palettes.** Each look owns 4–5 curated palettes (21 total), tagged dark or
light. `palette_for(look, dark, seed)` picks among those matching the resolved
polarity with the variety seed. Contrast is unit-tested: ink/paper and ink/card
≥ 7:1, on-accent/accent ≥ 4.5:1, accent/paper ≥ 3:1, muted/paper ≥ 3:1.
ClassicalNeon palettes are near-black grounds with one neon accent.

**Plates.** A `grounds` image plate (chosen by emotion, never by story words)
sits just above the paper at 42 % opacity — texture and mood without moving the
ground colour far from the palette's paper, so text contrast holds.

**Record.** `ProjectMeta.art = {look, palette, plate, families, sfx}`.

## Variety (`--variety auto|N`)

Seed = FNV-1a over the intent title and statements (`story_seed`).
- palette choice within the look (above);
- typographic phrase beats (atomic, no delivered image) that would repeat the
  previous beat's grammar rotate `KineticPoster → EditorialCollage →
  CinematicMultiplane` (all render a phrase as the visual; none drops content);
- image beats (TypeImageInterlock) rotate four subject-first layouts —
  TextTopSubjectBelow, SubjectRightTextLeft, SubjectCenterTextBand,
  SubjectLeftTextRight — starting at `seed % 4` and never repeating the
  previous image beat's layout; type and subject stay in disjoint regions.

## Taste rules that apply with or without art direction (DECISION 85)

`compiler/taste_rules.rs` budgets, enforced by layout QA:
headline ≤ 4 lines and ≤ 30 % of canvas height beside a subject (46 % when type
is the whole picture); statements over 12 words become a short derived title
(primary phrase → keyword → first clause ≤ 6 words) with the sentence spoken or
set as body copy; subjects ≥ 16 / 14 / 12 % of canvas area (tall / square /
wide); text over a subject ≤ 4 % and never over its head; frames ≤ 12 % pad
per side; subject/ground contrast ≥ 2:1 or a sticker outline; kickers ≤ 3 words.
QA checks: `headline_too_big`, `text_over_subject`, `subject_too_small`,
`frame_too_loose`, `asset_low_contrast`.

## Not yet
Exploration L2/L3 switching to `neighbours(look)`; per-look treatment presets
applied to cutouts (greyscale / halftone); prop pop-ins, ornaments as
furniture, the journey ribbon and end cards (Phase D2); look-specific type.
