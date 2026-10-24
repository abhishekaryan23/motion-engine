# MotionEngine 2.0 (v2) Authoring & Editing Guide

## 1. Two Ways to Create Videos (`make_video`)
1. **Brief + Brand Overrides (Fastest):**
   Pass `brief`, `recipe` (`top5_countdown`, `showreel`, `kinetic_poster`, `chart_reveal`, `ledger_ink`, `dot_story`), `aspect` (`9:16`, `16:9`, `1:1`), `seconds`, plus optional brand overrides:
   - `palette`: `"#0B0F17,#F8FAFC,#10B981,#64748B"` (`#BG,#INK,#ACCENT,#MUTED`) or JSON object
   - `fonts`: `"panchang_extrabold,instrument_serif_regular,space_mono_bold"` (`display,serif,mono`)
   - `ending`: `"spline_frame"` | `"monolith_split"` | `"orbital_seal"` | `"terminal_prompt"` | `"editorial_badge"`
2. **Full `MotionProgram` 2.0 (`program`):**
   Pass a complete `program` JSON object (schema at `motionengine://schema/v2`) for 100% control over beats, components, split-screen slots, cutouts, camera moves, and transitions.

## 2. Visual Inspection & Iterative Editing
1. **Inspect Beat Sheet & Component IDs:**
   Call `preview({"path": "<video_or_program_path>", "sheet": true})`. Returns the per-beat contact sheet image AND the JSON list of `components` (`id`, `kind`, `preset`, `start`, `duration`), `palette`, and `fonts`.
2. **Revise / Patch Any Component or Brand Setting:**
   Call `make_video` with `"edit": "<program_or_video_path>"` plus `"palette"`, `"fonts"`, `"ending"`, or `"patch"`:
   ```json
   {
     "edit": "output/make/kestrel_ledger/kestrel_ledger.program.json",
     "palette": "#0B0F17,#F8FAFC,#10B981,#64748B",
     "ending": "monolith_split",
     "patch": {
       "components": [
         {"id": "outro", "headline": "KESTREL LEDGER", "subline": "Settlement, simplified", "specs": ["T+0 // INSTANT", "FLOAT // -68%"]}
       ]
     }
   }
   ```
   Components in `patch.components` match existing components by `id` (deep-merged), append if `id` is new, or remove if `"delete": true`.

## 3. Closed Vocabulary Reference
- **19 Fonts (`display`, `serif`, `mono`):**
  - Display/Sans: `archivo_black`, `poppins_black`, `anton`, `cabinet_grotesk_extrabold`, `clash_display_bold`, `panchang_extrabold`, `bebas_neue`, `barlow_condensed_bold`, `satoshi_bold`
  - Serif/Slab: `instrument_serif_italic`, `instrument_serif_regular`, `dm_serif_italic`, `dm_serif_regular`, `zodiak_italic`, `zodiak_bold`, `zilla_slab_bold`
  - Mono: `ibm_plex_mono`, `space_mono`, `space_mono_bold`
- **7 Music Tracks (`music.track`):**
  `driving_pulse`, `tech_pulse`, `cinematic_strings`, `bright_upbeat`, `retro_groove`, `warm_piano`, `calm_ambient`
- **Spatial Slots (`slot`):**
  `center`, `left`, `right`, `top`, `bottom`, `lead`, `trail` (`left` + `right` compose side-by-side without overlap).
- **Component Kinds & Presets:**
  - `backdrop`: `style` (`solid`, `spotlight`, `grid`, `dots`, `diagonal`), `image` (path), `image_opacity` (`0.05..0.55`)
  - `kinetic_text`: `preset` (`poster_cuts`, `word_pair`, `whisper_scream`, `type_on`, `counter`, `split_screen`)
  - `bento`: `preset` (`grid_2x2`, `hero_left`, `pillars`, `golden_spiral` — 61.8% / 38.2% Fibonacci subdivision with $\varphi$ modular typography), `tiles` (`[{stat, title, detail, accent}]`)
  - `sketch` (`p5.brush` natural media + Golden Ratio engine):
    - `preset`: `golden_spiral` (Fibonacci Golden Rectangle + logarithmic spiral + watercolor wash + cross-hatching), `flow_field` (multi-streamline vector field tapestry), `architectural_hatch` (drafting study with hatched geometry + watercolor washes), `freeform` (custom `elements`)
    - `field`: `hand`, `curved`, `zigzag`, `waves`, `seabed`, `spiral`, `columns`
    - `brush` (11 `p5.brush` presets): `hb`, `2b`, `2h`, `cpencil`, `pen`, `rotring`, `spray`, `marker`, `marker2`, `charcoal`, `hatch_brush`
    - `elements`: `shape` (`line`, `spline`, `flow_line`, `rect`, `circle`, `arc`, `polygon`, `path`), `brush`, `stroke_swatch`, `weight`, `pressure`, `wiggle`, `hatch` (`{dist, angle_deg, cross, rand, continuous, gradient, brush}`), `watercolor` (`{swatch, opacity, bleed, texture, border, angle_deg, inward}`), `wash`
  - `chart`: `chart` (`bar`, `line`, `area`, `dot_plot`, `radial`), `series`, `callout`
  - `diagram`: `preset` (`outline`, `cards`, `rail`), `slot`, `nodes`, `edges`
  - `code_block`: `slot`, `title`, `lines` (`[{text, highlight}]`)
  - `photo_stack`: `preset` (`hero`, `word_morph`, `grid`, `vitruvian`, `constellation`, `zoetrope`, `blueprint`), `cutouts` (`marble_bust`, `statue_eye`, `hand_holding`, `hands_touching`, `horse_head`, `laurel`, `scales`, `hourglass`, `ancient_coin`, `armillary_sphere`, `marble_brain`, `winged_victory`, `atlas_globe`, `lion_statue`, or `"assets/generated/<file>.png"`)
  - `lockup`: `preset` (`spline_frame`, `monolith_split` — 61.8%/38.2% Golden Split, `orbital_seal`, `terminal_prompt`, `editorial_badge`), `headline`, `subline`, `badge`, `specs`, `headline_font`, `subline_font`, `headline_case`, `subline_case`

## 4. Creative Taste, Anti-Cloning & 9:16 Composition Rules
1. **Never Clone Example Skeletons:**
   - Every brand or topic must have its own distinct **aspect ratio** (`9:16`, `16:9`, or `1:1`), **music track**, **font trio**, **colour world** (dark obsidian, saturated brand ground, warm washi/alabaster), **component sequence**, and **ending `lockup` preset**.
   - Never copy the beat grid, camera keyframes, or 5-act sequence of another `.program.json` file.
2. **Multi-Ground Pacing (`grounds` + `colour_flip`):**
   - Do not stay on a single flat background colour for an entire video. Define 2 contrasting `grounds` in `palette` and switch `backdrop.colour` (`"background"` $\leftrightarrow$ `"ground1"` / `"ground2"`) on a downbeat with a `"colour_flip"` transition to create visual contrast and chapter separation.
3. **Dual-Zone Split-Screen Storytelling (`slot`):**
   - In **`9:16` Portrait (`1080×1920`)**: Pair `slot: "top"` (38.2% Golden Minor — e.g. `kinetic_text` or `title`) with `slot: "bottom"` (61.8% Golden Major — e.g. `sketch`, `chart`, or `bento`) so both zones compose simultaneously with zero overlap.
   - In **`16:9` Landscape (`1920×1080`)**: Pair `slot: "left"` with `slot: "right"` (e.g. `kinetic_text` / `code_block` on the left + `sketch` / `chart` / `diagram` on the right).
4. **Mix Component Families:**
   - Combine `kinetic_text` (`poster_cuts`, `word_pair`, `whisper_scream`, `counter`), quantitative `chart` (`radial`, `area`, `dot_plot`, `bars`) or `bento` (`golden_spiral`, `pillars`, `hero_left`), `sketch` (`golden_spiral`, `flow_field`, `architectural_hatch`, `freeform`), `photo_stack`, and a bespoke `lockup` (`terminal_prompt`, `monolith_split`, `orbital_seal`, `editorial_badge`, `spline_frame`).
