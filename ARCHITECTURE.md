# Architecture

MotionEngine has two front ends and one renderer. MotionEngine 2 turns a brief or a `MotionProgram`
into a beat-synced motion-graphics video. The v1 engine turns a `CreativeIntent` into a narrated
explainer reel. Both produce a `MotionScene`, and everything after that point is shared: timeline
evaluation, rendering, encoding and the audio mix.

## Data flow

```
brief ───► motion-bridge::parse_brief ─┐
intent ──► motion-bridge::from_intent ─┴─► Content ─► pick_recipe ─► plan bars on the beat grid ─► realize
                                                                                │
                                                                                ▼
MotionProgram v2.0 (schema/motion_program.json, motion-program/src/program.rs)
   │ program assets: resolve `gen:<id>` images through the provider fallback (cached, lockfile)
   │ resolve_beats: music analysis → frame-snapped seconds, 2-frame cut lead
   │ validate: closed vocabulary, timing fit, channel overlaps, bans
   ▼
lower: one Scene per component (chart, diagram, kinetic, field, lockup, code, device, flow, sketch,
   bento, photo, …); palette roles per ground; design passes; camera; layout follows content
   ▼
MotionScene (motion-core/src/scene.rs)
   │ QA: the mistake ledger, static gates on the program and scene, rendered gates on frames
   │ repair: at most two passes; `program render` refuses to render on a remaining failure
   ▼
motion-core::evaluate_frame ─► ResolvedFrame ─► motion-render CpuRenderer (tiny-skia; optional GPU post)
   ─► frames streamed to FFmpeg ─► MP4 ─► audio: music bed, SFX cues, voice (v1), −16 LUFS
   ─► critique (motion-core/src/slop_qa.rs, scored out of 40)
```

v1 path: `CreativeIntent` + `StyleProfile` → `motion-core` compiler (grammars, lifecycle, speech
plan) → `MotionScene` → the same renderer. `voice` and `reel` add the voice-over (`motion-voice`) and
speech QA; `edit` ingests a talking-head video, times its words and adds cutaways and captions.

## Crates and boundaries

| crate | owns | must not |
|---|---|---|
| `motion-core` | intent and style schemas, the v1 compiler, the `MotionScene` schema, validation, easing and springs, the timeline, path operations, audio planning, critique | touch pixels, font files or the network |
| `motion-render` | the `Renderer` trait, `CpuRenderer`, text shaping and outlines, textures, image decode and analysis, post effects (CPU, optional wgpu), QA on frames, PNG and FFmpeg export | make semantic or layout decisions |
| `motion-program` | the closed program vocabulary (components, verbs, camera, transitions), validation, taste tables (durations × tempo, springs, sizes, slots), deterministic lowering, palette roles and design passes (`palette.rs`, `layout.rs`), layout that follows content (`flip.rs`), QA (`qa.rs`, `qa/design.rs`, `ui_gates.rs`), the mistake ledger (`ledger.rs`) and repairs (`repair.rs`) | take colours or sizes from outside the palette and taste tables; accumulate time; call a provider |
| `motion-bridge` | brief or intent → `Content` → recipe (`assets/recipes/make/*.json`) → bar plan on the music grid → program; claim headlines; typed UI flows | put direction words on screen |
| `motion-providers` | asset providers: MCP client with OAuth, OpenArt adapter, local MOSS-Image provider, policy (allow-list, caps, provider order), cache, spend log, lockfiles | spend without a quote within the caps |
| `motion-voice` | text to speech, voice cache, key handling, optional local speech recognition (whisper.cpp, CTC) | log keys |
| `motion-mcp` | the MCP server profiles (`v2`, `weak`, `creator`, `operator`) | read or log keys; make network calls itself |
| `motion-cli` | the `motion-engine` binary | contain logic beyond wiring |

Network code lives only in `motion-voice` and `motion-providers`. Compiling, lowering, QA and
rendering are offline.

## Rules that hold everywhere

- **Weak models write intent, not pixels.** Small models write a brief or a `CreativeIntent`; they
  never write programs, pixel positions or timings. Intent and style schemas carry no pixel or
  timing fields.
- **Determinism.** Compilation and lowering are pure functions of their inputs. Seeds are explicit;
  variation is hashed from the seed and stable keys. Time is `frame / fps`, never accumulated. Only
  bundled fonts are loaded.
- **The renderer is dumb.** It consumes `ResolvedFrame` and draws; all meaning is decided earlier.
- **The beat clock.** Every start and end snaps to a frame, minus a two-frame cut lead; the final
  hit lands on a downbeat.
- **Colours come from roles.** Components ask for a role (ink, structure, accent, texture) on their
  own ground; the palette resolver guarantees the contrast. Textures are never cut around elements.
- **Layout follows content.** Each step is solved from what is visible (`flip::follow`); nothing
  reserves room for an element that has not arrived.
- **Generated assets are pinned.** Providers run only in `program assets`; everything downstream
  reads the lockfile and stays byte-identical.

## The mistake ledger

`assets/qa/mistakes.json` holds one entry per known design defect: `id`, `description`, a review
note, `detector`, `fix_policy`, optional `repair`, `checks` area and `fixture`.

- Static detectors run on the program and the lowered scene (`motion-program/src/ledger.rs`,
  `ui_gates.rs`); rendered detectors run on frames (`motion-program/src/qa/design.rs` and the
  visible-change, near-blank, fill and beat-clock gates in `motion-cli/src/program_cmd.rs`).
- Repairs implement `trait Repair` and are listed in `repair::repairs()`. The QA loop runs
  QA → repair → QA at most twice.
- Every entry names a fixture in `assets/qa/fixtures/` that must fail before the fix and pass after
  it (`ledger_fixtures_fail_then_pass`, `rendered_fixtures_fail_then_pass`).
- `motion-engine program learn` appends an entry and a fixture stub.

The rules the gates enforce are summarised in [docs/DESIGN_RULES.md](docs/DESIGN_RULES.md).

## Generated assets

`MotionProgram.assets` declares image plates and cutouts by id, prompt, kind, aspect and seed. Image
fields refer to them as `gen:<id>`. `program assets` walks the policy's provider order (OpenArt over
MCP, then other configured providers, then a local MOSS-Image server), shows a quote before any
spend, caches results by content and writes `<stem>.assets.lock.json`. See
[docs/PROVIDERS.md](docs/PROVIDERS.md).

## MCP

`motion-mcp` runs the CLI as a subprocess and returns terse JSON. The `v2` profile exposes
`make_video`, `edit_video`, `preview` and `critique`; the v1 profiles expose narrated-reel tools in
three sizes. The server reads no keys and makes no network calls itself. See [docs/MCP.md](docs/MCP.md).

## v1 engine (CreativeIntent → compiler)

The sections below describe the v1 compiler, used by `compile`, `voice`, `reel`, `edit` and the v1
MCP profiles, and by MotionEngine 2 for everything after `MotionScene`.
### v1 compiler detail (v0.7.1 Visual Language, on v0.7 Reference Style and v0.6 TasteDirector)

```
CreativeIntent ─┐
StyleProfile  ──┴→ MotionCompiler ─→ MotionScene ─→ validate ─→ evaluate_frame(project, n) ─→ ResolvedFrame ─→ Renderer ─→ PNG ─→ ffmpeg ─→ MP4
   (intent.rs,        (compiler/)       (scene.rs)   (validate.rs)       (timeline.rs)                     (motion-render)
    style.rs)            │    ↑
                         │    AssetManifest v0.2 (analyzed images, vendor-neutral)
                         │    ↑ ingest-assets: decode PNG/JPEG · validate · analyze · cache · QA (motion-render)
                         │    ↑ files from ANY external generator (outside the engine)
                         └→ plan_assets → AssetPlan → prompt_specs → AssetPromptSet (asset_prompts.rs)
```
Generated-asset protocol: docs/GENERATED_ASSET_PROTOCOL.md. The engine has no
vendor SDK, model or network dependency; images are raw material, the engine
is the art director (placement, treatment, time, reuse).

Inside the compiler, per beat (0.4): `plan_timing` → **Lifecycle** (phase boundaries) →
`grammar::select` → **Composition** {grammar, variant, shape} → builder (layers + motions scheduled
into the lifecycle) → kicker / continuity / stage wrapper (anticipation + exit).

### v1 crate roles
| crate | owns | must not |
|---|---|---|
| `motion-core` | intent/style schemas, compiler, MotionScene schema, validation, easing/presets, timeline, asset plan/prompt/manifest/cache types (`assets.rs`, `asset_prompts.rs`) | touch pixels or font files |
| `motion-render` | `Renderer` trait, `CpuRenderer` (tiny-skia), text engine (cosmic-text → outline paths), textures, PNG/FFmpeg export, `FontMeasure`; 0.5: `decode` (PNG + JPEG via zune-jpeg), `analysis` (alpha bounds, occupancy, safe regions, head estimate), `ingest`, `asset_qa`, `treatment` | make semantic/layout decisions |
| `motion-cli` | `motion-engine compile [--asset-manifest] [--reference-style] | plan-assets [--reference-style] | reference-evidence | validate-reference-style | resolve-style | asset-prompts | ingest-assets | validate-assets | validate | render | inspect | qa [--reference-style] | style-preview | compare-styles` | contain logic beyond wiring |

## CreativeIntent v0.2 (`intent/`)
`{version, title, format, beats[]}`; beat = `{purpose, statement, primary, secondary?, relationship?, energy, continuity, keyword?}`.
Subject is a `kind`-tagged union: `phrase | number | object` (atomic, as in v0.1) and, since v0.2,
`collection {items[2..6] of atomic, meaning?}`, `state_change {entity, from, to, meaning?}`,
`derived_metric {operation=ratio, numerator{value,meaning}, denominator{value≠0,meaning}, format=percent|decimal|per_thousand, meaning?}`.
Relationship adds `accumulate`. `deny_unknown_fields` everywhere.
- **v0.1 is frozen** in `intent/v0_1.rs` (generates `schema/creative-intent-v0.1.schema.json`, unchanged). `from_json`/`Deserialize` dispatch on `version`: "0.1" → strict v0.1 parse → lossless `TryFrom` (keeps version "0.1"); "0.2" → v0.2 parse. v0.1 documents using v0.2 features are rejected. `CreativeIntent::validate()` adds the rules JSON can't carry (finite numbers, non-blank strings) and runs first in `compile()`. A frozen-hash test pins v0.1 compile output.

## StyleProfile v0.1 (`style.rs`)
Small enums: family, material, typography_style, depth, camera_style, **motion_language** (0.3, optional, default `auto`), texture_style, accent_role, seed; and (0.6) the taste fields tone, polarity, temperature, temperament, density (all default `auto`; see "Taste & temporal style"). Source-agnostic.

## Motion languages (`motion/`, 0.3)
Behavior systems, not templates (see `docs/MOTION_LANGUAGE.md`).
- `language.rs` — `MotionLanguageProfile {preset, stagger, depth planes, camera, typography, settle, data_numbers}` per (language, energy, camera_style). Minimal / Kinetic / Parallax / Sequential / Data.
- `stagger.rs` — closed-form irregular offsets (Calm/Editorial/Impact patterns + cycle), orders forward/reverse/center-out/edges-in.
- `kinetic.rs` — WordCascade, LineReveal, TextMaskReveal, TypeReplace, TypeScaleEmphasis, KeywordPunch, TrackingReveal over a measured `TextRun`.
- `dataviz.rs` — counter, progress_bar, bar_chart, hbar_chart (0.22 rankings: picture, name, bar, value per row), comparison_bar, path_draw.
All of them **expand into primitive layers + motions** (`Expansion`); Timeline/Renderer never see words, charts or meaning.
The compiler picks the language per beat (`compiler::language_for`: explicit style value, else from beat structure — countable number → data, explain → sequential, emphasized keyword on building/impact → kinetic, calm → minimal, layered+moving camera → parallax, else minimal).

## MotionCompiler (`compiler/`)
- `theme.rs` — palette per accent/material; font role → bundled OFL face per typography style.
- `measure.rs` — `TextMeasure` trait. The CLI passes `FontMeasure` (render crate, same shaping as rendering) so layout uses real metrics; `ApproxMeasure` for font-free tests.
- `typeset.rs` — voices (HEADLINE, HERO_NUMBER, SERIF, KICKER, LABEL, BODY, GHOST), size fitting, balanced line breaking (DP minimizing widest line).
- `mod.rs` — timing (duration = energy base + reading time + relationship time, clamped 3.2–6 s; overlap from incoming energy), backdrop (paper + animated grain), continuity bookkeeping.
- **Structured subjects (v0.2)** route before purpose recipes (`recipes::structure_of`: primary's structure, else secondary's). Shared skeleton `structured_head` (ghost, statement headline, anchor) → `Region` → composer:
  - `collection.rs` — one layout-bound card per item, item stagger; `accumulate` → running count (chained `count` motions), stepping progress bar (chained accent_expand), aggregate slab (consequence secondary > exact sum of same-unit numbers > "N meaning").
  - `state_change.rs` — entity tag + panel; `from` holds, `type_replace` → `to`; old state stays reduced with a strike; two state changes = identical rows replacing in parallel.
  - `derived.rs` — `compute` (ratio) + `format_metric` (documented rounding); single: numerator ÷ denominator → counted result (+ progress for percent); two metrics: rows + comparison_bar + computed direction line ("fewer X · higher rate", never value judgements).
  Structured beats get extra read time (`structure_time`) and `auto` language by structure (collection → sequential/data, state_change → kinetic, derived → data). They don't carry across beats.
- **Lifecycle (0.4, `motion/lifecycle.rs`)** — `plan(LifecycleInput) -> scene::Lifecycle`: closed-form phase boundaries from duration, energy, language and density, planned in `plan_timing` (`BeatPlan.life`) before composition. Builders put the primary in ENTER, secondary information at `life.evolve_events(n)`, nothing new at/after ANTICIPATE; `wrap_stage` adds the generic anticipation (stage scale 1 → 0.975, 14u lead into the exit). See docs/SCENE_LIFECYCLE.md.
- **Composition grammars (0.4, `grammar/`)** — `select(SelectInput) -> Composition` is a first-match table over structure (subject kinds, purpose, relationship, resolved language, delivered image); `variant_for` picks layout variants from format + beat seed; `builder(c)` dispatches. EditorialCollage = `recipes::emphasize`, SpatialCauseEffect = `recipes::contrast`, SequentialStack = `collection::compose` / `recipes::explain`, SplitContrast = `state_change::compose` / `split_contrast.rs`, DataStory = `derived::compose` / `data_story.rs`; KineticPoster, HeroObject, CinematicMultiplane, EvidenceStack, TypeImageInterlock have their own files; `plate.rs` = delivered image or procedural plate. See docs/COMPOSITION_GRAMMAR.md.
- **Asset planning (0.4, `asset_plan.rs` + `assets.rs`)** — `plan_assets(intent, style, library) -> AssetPlan` reuses grammar selection and emits semantic `AssetRequest`s (or an explicit `none` per beat); `compile_with_assets(..., &AssetManifest)` places delivered images (paths relative to the library root; the CLI rewrites manifest-relative paths). See docs/ASSET_PIPELINE.md.
- **Generated assets (0.5)** — `asset_prompts::prompt_specs` (one spec per depicted entity, `fp1` fingerprint, file stem), `ingest` + cache (fingerprint → file → analysis; never silently replaced), `compile_with_assets` fails early on a *required* missing image (`CompileError::MissingRequiredAsset`) and falls back to the image-free grammar for optional ones.
- **Subject-aware composition (0.5, `grammar/placement.rs`)** — internal layout candidates scored against the analyzed image (face/back-line occlusion, clipping, balance, lower third, dead band, hierarchy, support room); TypeImageInterlock builds the winner with per-line layering (a line touching the head is always behind the subject), paper strips under front lines, a nearer subject plane and an almost-static READ push. HeroObject sizes delivered cutouts by their subject bounds; CinematicMultiplane puts a delivered environment on a full-bleed background plane; EvidenceStack sizes the card from the artifact's aspect. Treatments: `compiler::treatment` (preset from style + role → explicit `ImageTreatment`).
- **Asset carry (0.5)** — consecutive interlock beats served by the same manifest entry share ONE image as a SharedElement (`plate::place_subject_image`, decided up front in `Ctx.image_carry`); `wrap_stage` then splits the stage into back/front groups with identical motions so the shared image keeps its interlock layering. The carried image gets an accent tint emphasis at EVOLVE (tint channel).
- `recipes.rs` — per purpose: `emphasize`, `contrast/compare` (relationship → semantic transformation), `reveal` (accent slab, accent wipe after impact), `explain`. Shared skeleton: camera stage group (slow push), kicker + accent rule, ghost word with parallax drift, overlap-aware exit.

## Taste, temporal style, and reference style (0.6–0.7)
Full description: docs/TASTE_DIRECTOR.md. Stage 0 of the compiler: `StyleProfile` (five public taste fields `tone`, `polarity`, `temperature`, `temperament`, `density`) plus optional public `ReferenceStyleProfile` v0.1 → **TasteDirector** → internal `ResolvedStyleProfile` → the rest of the compiler. The profile is produced outside the engine from the `reference-evidence` file bundle; CLI validation and resolution remain deterministic.
```
StyleProfile ─→ compiler/taste.rs ─→ ResolvedStyleProfile ─┬→ temporal.rs   tendencies → multipliers, curves, counts
                (semantic resolvers,   (enums + curated     ├→ backdrop.rs   piece-wide background grammar
                 one tone table)        colors, `effective` ├→ transition.rs panel wipes (geometric / kinetic)
                                        StyleProfile)       ├→ furniture.rs  density annotations
                                                            └→ preview.rs    style-preview frame + description
```
- **Boundary: taste = semantic, temporal = numbers.** `taste.rs` outputs only small enums and curated colors (`Precise`, `Geometric`, `Progressive`, `Large`); it never holds durations, frame counts, easing constants or spring parameters. `temporal.rs` is the single place those words become numbers: per-beat motion-profile adaptation (`apply_temperament`, `apply_layers`, `lean_language`), beat pace and READ share (`duration_scale`, `read_bias` → `plan_timing_with` → Lifecycle), overlap and handoff (`overlap_scale`, `handoff`, `exit_gesture`), EVOLVE hierarchy reframes and background handoff (`hierarchy_shifts`, `background_handoff`, used by `recipes::wrap_stage` and the ghost word), typesetting scale gains. The Timeline, Renderer and MotionScene schema gained nothing.
- **Classic is the identity.** All-`auto` taste resolves to `ResolvedTone::Classic`; every temporal mapping is neutral, so pre-0.6 styles compile byte-identically. Non-default legacy StyleProfile fields win over the tone (`ResolvedStyleProfile.effective`, which the pre-0.6 paths still read).
- **Modules.** `taste.rs` (resolvers, internal `ReferencePrinciples`, `ResolvedStyleProfile`, `StyleFingerprint`), `reference/` (profile, evidence, normalization, coverage, file protocol), `temporal.rs`, `backdrop.rs`, `transition.rs`, `furniture.rs`, `preview.rs`; CLI wiring in `motion-cli/src/reference_cmd.rs` and `style_cmd.rs`.
- **Determinism.** Variation is hashed from `(style.seed, story_key)`; no randomness or clocks.
- **Future audio.** A later ChoreographyEngine may decide WHEN events land (speech, music) using lifecycle event slots and taste's rhythm. Current reference analysis records only whether an audio stream exists; no audio interpretation is implemented.

## Visual language (0.7.1)
Full contract: docs/VISUAL_LANGUAGE.md. A reference teaches what fills the frame, not only how it looks; the story's meaning still comes from CreativeIntent.
- **VisualLanguage** (`compiler/visual.rs`): internal policy normalized 1:1 from the optional `visual_language` of `ReferenceStyleProfile` v0.2; carried on `ReferencePrinciples.visual` -> `ResolvedStyleProfile.visual`. Not a StyleProfile field, not part of `StyleFingerprint`. Neutral without a reference (pre-0.7.1 paths, byte-identical goldens).
- **select -> visual_bias** (`grammar::select`): the semantic table picks first; the bias rescoring applies only to atomic phrase beats, ties keep the semantic choice. It returns a `Composition` whose `Depiction` is `Typographic` or `Entities`.
- **Depiction::Entities** (`grammar/diagram.rs`): procedural entity tokens, a fixed-geometry process track with progress and stage indicator, relationship -> operation scheduled into EVOLVE. Existing primitives and motions only; a delivered `hero_object` image can be the hero token.
- **Planner rule** (`asset_plan.rs`, `plan_assets_with_reference`): same selection as compile; Entities beats are `procedural`; optional `generated_image` requests only for image-led single-entity heroes.
- **visual_qa** (`motion-render/src/visual_qa.rs`): structural text-versus-figure measure on the timeline-resolved mid-beat frame; `qa --reference-style` compares `pure_type_ratio` with the reference's weight.

## MotionScene v0.2 (`scene.rs`)
project, canvas, theme (role → font asset), assets (image|svg|font, paths relative to motion file dir + `asset_root`), scenes (absolute start, may overlap, optional `lifecycle` metadata), layers, motions, shared.
- **Scene.lifecycle (0.4)** `{enter, settle, read, evolve, anticipate, bridge}` scene-local boundaries (PRE_ENTER = `[0, enter)`, BRIDGE ends at the duration). Informational: validated (`0 ≤ … ≤ bridge ≤ duration`), read by QA, ignored by Timeline/Renderer.
- Layers: rectangle, rounded_rectangle, text, image (0.5: optional `treatment` — explicit desaturate/brightness/contrast/duotone/tint/grain/paper edge/contact shadow, applied once at prepare time), svg, group, texture, polyline (0.3). Common transform/opacity/z/visible/clip, optional `depth` (camera plane, inherited) and `layout` (LayoutBinding).
- **LayoutBinding** `{parent (earlier sibling), horizontal left|center|right, vertical top|center|bottom, offset, padding}`: the child lives in the parent's box space and its alignment point lands on the parent's *resolved* content box. Text aligns vertically by measured `TextStyle.ink` bounds (compiler writes them from glyph outlines via `TextMeasure::ink_bounds`).
- **Camera** (per scene): `push` (zoom) and `track` (pan) channels; pivot defaults to canvas center.
- Transform: `translate(x,y) · rotate · scale · translate(-anchor·size)`.
- Motions (scene-local time) each write one **channel**: move→offset, scale, rotate, fade→opacity, mask_reveal→clip, clip_reveal→content offset, accent_expand→geometry, count→text, trim→trim, tint→tint (0.5: `{color, from, to}` amount mixes the layer's rendered colours toward `color`; groups pass it to untinted children). Overlap on the same layer+channel is a validation error. Before the first motion on a channel the `from` value holds.
- Draw order: global sort by `(z_index, scene index, layer index)`; groups sort children locally.
- **Shared elements (SharedTrack):** `shared[] = {id, layer, track[]}`; keys are `(scene id, scene-local at, state{x,y,scale,rotation,opacity}, easing, layout?)`. Lives from first to last key; omitted fields carry over. Scene-anchored so retiming scenes keeps continuity correct. A key's position source is Free (x/y, through that scene's camera) or Bound (`layout`, through the parent's world box); it carries over until a key sets x/y or a new binding.
- **Shared elements as motion targets (0.4):** a scene motion may target a shared element's *layer id* (one id namespace with scene layers) with move / scale / rotate / fade / count / tint (0.5), only in scenes where the element has a track key. It applies while its scene is active, on top of the track state with scene-layer channel semantics; move offsets are multiplied by the element's `unit` (camera zoom at its depth, or the bound parent's scale — interpolated between keys like every other state field). Across active scenes the most recently started motion on a channel wins; same-channel overlaps between scenes are a validation error. Compilers end transform channels at neutral inside the scene.

## Transform & layout order (0.3)
Per layer, evaluated by the Timeline (never the renderer):
```
base geometry (x, y, w, h)
→ geometry channel (accent_expand)            = resolved container bounds
→ layout binding: place in parent's resolved box (parent L_p · P + offset)
→ local motions: move offset, rotate, scale (pivot = alignment point when bound)
→ container (group) transform                 = scene space
→ camera at the layer's depth k: p' = pivot + zoom^k·(p − pan·k − pivot)   = canvas
→ opacity / clip → ResolvedLayer
```
Every layer's world box is recorded **before** culling so bindings and shared keys can reference faded parents. Shared elements resolve each surrounding key into canvas space with the same rules (Free: key scene's camera at the element depth; Bound: parent's world box, inheriting its uniform scale + rotation), then interpolate — one world/camera model, no parallel coordinate system.

## Timeline (`timeline.rs`)
Pure `evaluate_frame(&project, frame) -> ResolvedFrame`; `t = frame / fps`. Scenes active on `[start, end)`. Output is a tree of `ResolvedLayer {transform, content_transform, opacity, clip, kind, text (count), trim, children}` in draw order. Layers with ~0 opacity or fully clipped are dropped.

## Renderer (`motion-render`)
`trait Renderer { type Frame; fn render(&self, &ResolvedFrame) -> Result<Frame> }`.
`CpuRenderer::new` prepares everything once (fonts → glyph outline paths per text layer, PNG decode, SVG rasterized at 2× per layer, textures generated per layer), then is immutable/`Sync`; `export::render_frames` renders in parallel with rayon. Groups with opacity < 1 or a clip are composited through an offscreen layer. Only project fonts are loaded (no system fonts) for cross-machine determinism. Count overrides are shaped lazily through a mutex-guarded cache on the same TextEngine (same fonts/fallback); polylines are trimmed exactly by arc length.

## QA
`motion-engine qa <file> [--json]` — structural-motion profile (grain stripped, downsampled luma diffs) that flags "static hold → transition spike" pacing, plus (0.4) a per-scene lifecycle report: activity per phase (PASS/DRIFT/LOW/STATIC/EMPTY; DRIFT = slow change visible over a 1 s window, 0.5), evolve-event count (from motion starts in [read, anticipate)), transition spike ratio, longest static hold, warnings (`docs/MOTION_QA.md`).

## Reference analysis boundary (0.7)

`motion-render::reference` owns media probing, small RGB analysis streams, deterministic sampling/contact sheets, evidence extraction and file cache. `motion-core::reference` owns pixel-free evidence/profile contracts, the external interpreter request, strict validation, conservative normalization and coverage. The CLI only wires these stages.

```
Video → evidence + timestamped images → external multimodal interpreter
      → ReferenceStyleProfile v0.1/v0.2 → validated ReferencePrinciples
      + StyleProfile → TasteDirector → ResolvedStyleProfile → compiler
```

The bundle has no target CreativeIntent. Only the closed profile reaches the compiler; source frames never become generated assets. `StyleProfile` is the existing semantic style input; there is no separate StyleIntent or BrandProfile interface in 0.7. Non-default legacy values and explicit taste fields retain their existing precedence. Material/image treatment reach image layers and AssetStyleProfile; asset planning uses the same reference-informed resolution as compilation. Unknown profiles preserve Classic identity. Numeric evidence/sampling changes invalidate caches through analyzer version 0.7.3.
