# Asset pipeline (0.4 foundation, 0.5 generation boundary)

The engine **plans** images and **ingests** them; it never generates them.
Generation lives entirely outside the engine (any vendor, MCP, script or
person) behind a file protocol: docs/GENERATED_ASSET_PROTOCOL.md. Analysis
and preflight QA: docs/ASSET_ANALYSIS.md. Treatments: docs/IMAGE_TREATMENTS.md.
Placement: docs/COMPOSITION_GRAMMAR.md §Subject-aware composition.

0.5 at a glance:
```
plan-assets → AssetPlan (+ continuity_key, context per request)
asset-prompts → AssetPromptSet (one spec per depicted entity, fp1 fingerprint, file stem)
[external generator writes <stem>.png|.jpg (+ sidecar)]
ingest-assets → decode (PNG/JPEG) · validate · analyze · cache (fingerprint) · QA → AssetManifest v0.2
validate-assets → PASS / WARNING / FAIL per asset
compile --asset-manifest → subject-aware composition, treatments, asset carry
```

```
CreativeIntent + StyleProfile
      ↓
MotionCompiler ── grammar::select per beat
      ↓
AssetPlanner (compiler::plan_assets) ─→ AssetPlan { style, beats[], requests[] }
      ↓
EXTERNAL GENERATOR (0.5, any vendor) ─→ AssetManifest { assets[] }
      ↓
MotionCompiler (compile_with_assets) ── composition builders place images / placeholders
      ↓
MotionScene (ordinary `image` layers + assets) → Timeline → Renderer
```

The Renderer never learns how an image was made. Requests contain no x/y,
crops, transforms or timing. No generator vendor is referenced anywhere.

## Types (`motion_core::assets`)

- `AssetSource`: `none | procedural | svg | generated_image | user_asset`
- `AssetRole`: `hero_subject | hero_object | supporting_object | environment | evidence_image | portrait | transition_object | foreground_occluder`
- `AssetRequest {id, beat, source, role, subject, presentation, negative_space, background, priority, composition, reason, library_asset?}`
  - `id` = `beat_<n>.<role>` (n 1-based), unique.
  - `presentation`: `isolated_cutout | full_frame | flat_artifact | background_plate`
  - `negative_space`: `none | left | right | top | bottom` — where the composition needs room.
  - `background`: `transparent | opaque`; `priority`: `required | optional`.
- `BeatAssetDecision {beat, composition, source, reason, requests[]}` — one per beat, including "no image needed" (`source: none`).
- `AssetStyleProfile {medium, realism, lighting, contrast, palette_tendency, edge_treatment, shadow_treatment, camera_feel, background_behavior}` — `AssetStyleProfile::from_style(&StyleProfile)`, never from CreativeIntent.
- `AssetPlan {version "0.1", style, beats[], requests[]}`
- `AssetManifest {version "0.2" ("0.1" still read), assets: [ManifestEntry], missing: [MissingAsset]}`; `ManifestEntry {id, path (.png/.jpg/.jpeg), width, height, alpha, safe_bounds?, subject_anchor?, face_anchor?, face_bounds?, head_bounds?, serves[], fingerprint?, continuity_key?, content_hash?, analysis?, generator{}}` — normalized (0..1) boxes/points; `generator` is free-form metadata the engine never reads. `get(request id)` also matches `serves`. A `required` entry in `missing` makes compilation fail before rendering; `optional` ones fall back to the image-free composition.
- `AssetRequest` (0.5) adds `context` (the beat statement, verbatim) and `continuity_key` (the depicted entity, e.g. `office_worker`).

## Planner rules (`compiler::plan_assets`)

Per beat, with the composition `grammar::select` picks when no image has been
delivered (`has_subject_image = false`):

1. **Data and structure carry themselves → no image.** Number, derived_metric, state_change and collection subjects add no request. DataStory / SplitContrast / SequentialStack / SpatialCauseEffect beats with no object subject decide `none` ("data is typographic/procedural") — or `procedural` when the grammar draws cards/panels (SequentialStack, SplitContrast, SpatialCauseEffect).
2. **Object subjects** (primary, then secondary) → one request each:
   - source: library has an `.svg` → `svg`; a `.png` → `user_asset`; not found → `generated_image`. `library_asset` = the name when found.
   - role: EvidenceStack → `evidence_image`; HeroObject primary → `hero_object`; otherwise `supporting_object`.
   - presentation: evidence → `flat_artifact` (opaque); else `isolated_cutout` (transparent). Priority `required`. Subject = the object's `meaning`, else `value`, else the asset name.
3. **Human phrase subjects** in an emphasize beat whose grammar is EditorialCollage, KineticPoster or CinematicMultiplane → `generated_image`, role `portrait` when the phrase contains `face` or `portrait`, else `hero_subject`; `isolated_cutout`, transparent, priority `optional` (the grammar is complete without it; with it, compile selects TypeImageInterlock). Negative space opposite the headline side: Standard variant → `right`, Mirror → `left`. Subject = the phrase value (+ `, ` + meaning when present). "Human" = the phrase value/meaning contains one of a closed word list (whole words, case-insensitive): person, people, worker(s), employee(s), woman, women, man, men, customer(s), user(s), parent(s), child, children, kid(s), student(s), doctor(s), nurse(s), driver(s), family, families, team, face, portrait, crowd, commuter(s), shopper(s), founder(s), manager(s), patient(s), reader(s), viewer(s). This classifies what a subject *is*, never what a story is about.
4. **CinematicMultiplane without a human subject** → one `optional` `environment` request: `background_plate`, opaque, negative space `none`, subject = the beat keyword, else the primary phrase, + " — environment".
5. The decision's `source` is the strongest of its requests (`none < procedural < svg < user_asset < generated_image`); `reason` names the rule. Requests are ordered by beat, then role.

The plan is deterministic (same intent + style + library → byte-identical JSON).

### 0.7.1 visual language (`plan_assets_with_reference`)

A reference's visual language only biases planning; without a reference, or under Type/Neutral weight, rules 1-5 apply exactly as above. Details: docs/VISUAL_LANGUAGE.md section 4.

1. Rules 1-2 (data, structure, object subjects) never change.
2. An `Entities` beat (HeroObject / SpatialCauseEffect drawn as entity tokens) decides `procedural` ("visual language: engine-drawn entity tokens") and adds no external request.
3. Exception: `wants_images()` and HeroObject, one `optional` `generated_image` for the primary phrase: `hero_subject` if human (0.5 word list) else `hero_object`; `isolated_cutout`, transparent, negative space `none`.
4. A typographic emphasize beat with a phrase primary and `wants_images()` gets the same optional request (when rule 3 has not already requested a human hero_subject).
5. `prefers_procedural()` suppresses the style-driven requests of rules 3 and 4 (human cutout, multiplane environment plate); object requests stay.
6. A delivered `hero_object` for a phrase beat makes `select` choose HeroObject with the image as the hero; undelivered optional requests fall back to the procedural token or typography.

## Using a manifest

`compile_with_assets(intent, style, library, measure, &manifest)` — manifest
paths relative to `library.root` (the CLI rewrites paths given relative to the
manifest file). Builders look images up by request id
(`grammar::plate::image(ctx, beat_index, role)`); a delivered image becomes an
ordinary `image` asset + layer; a missing one becomes a procedural plate in the
same box. Delivered hero_subject / portrait images upgrade an emphasize beat to
TypeImageInterlock.

## CLI

```
motion-engine plan-assets <intent.json> [--style style.json] [--output plan.json]
motion-engine compile <intent.json> --style style.json --asset-manifest manifest.json --output out.motion.json
```

## 0.8 library catalogs (opt-in asset families)

`AssetLibrary.families` (CLI: repeatable `--asset-family <name>` on `compile` and `plan-assets`) enables families; empty (the default) leaves planning and compiling byte-identical to before.

A family is `<root>/library/<family>/` holding `catalog.json` (`{version, family, assets:[{id, file, role: object|figure|texture, tags, qa: PASS|WARN|FAIL|MISSING, ...}]}`; unknown fields ignored) and `manifest.json` (an `AssetManifest` from `ingest-assets`; entry ids `library.<id>`, paths relative to the family dir). A missing or invalid catalog/manifest skips that family silently.

Matching (`AssetLibrary::catalog_match`, compiler/catalog.rs) is EXACT word overlap, no stemming or synonyms:

1. Request words = lowercase ASCII-alphanumeric tokens of the object subject's `asset` + `value` + `meaning` (object requests) or the phrase `value` + `meaning` (human / `hero_object` phrase requests), minus the stop words a an the of and or to in on for with at by from is are.
2. Eligible: qa `PASS`/`WARN`, present in the family manifest. Object requests match role `object`, human requests (`hero_subject`/`portrait`) role `figure`; textures never match; environment plates never match; data and structure beats never get requests.
3. Asset words = tokens of its `id` plus its `tags`. Score = distinct request words found; needs >= 1. Best = highest score, then family order, then asset id ascending.
4. Precedence: exact-name `library/<name>.svg|png` first; else a catalog match (`source: user_asset`, `library_asset: "<family>/<id>"`, reason "Library catalog: ... matches <words>."); else `generated_image`. Priority is unchanged.

Delivery: `compile_full` (so `compile` and `compile_with_assets`) plans the assets and adds a manifest entry per catalog match (id = serves = the request id, path `library/<family>/<entry path>`) unless the caller's manifest already serves that request id (caller entries always win). Grammar selection therefore sees a library image exactly as an externally delivered one.

## 0.9 library families (spec-driven, committed)

All families are bundled under `assets/library/<family>/` (catalog.json + manifest.json + PNGs + LICENSE.md) and rebuilt by `scripts/build_all_libraries.sh [family]`.

- Glyph families (`assets/glyph_families/*.json` + picture fonts in `assets/fonts/`): sketch_icons, woodcut_kitchen, retro_cars_a/b, aikakirja_ornaments (551 assets). Ingest flags them `monochrome` (flat single ink), so the compiler recolours them to the palette ink on any ground.
- OpenArt families (`assets/cutout_families/*.json` = prompt, subject, exact-word tags, role, model, aspect per asset; optional `greyscale`, per-asset `screen_box`), generated with GPT Image 2.5 Flare low 1k on a #FF00FF ground in the OpenArt project "MotionEngine Asset Library", downloaded to `output/openart/<family>/raw/`, packaged by `scripts/build_family_library.py`:

| family | emotions | assets | notes |
|---|---|---|---|
| classical_greyscale | drama, luxury | 20 | marble hands/busts (face-away/generic), column, laurel, hourglass, scales, coin; greyscale |
| halftone_retro_objects | energy, playful_retro | 15 | B&W photographic; `retro_tv` keyed screen hole recorded as `screen_box` for future inserts |
| people_everyday | warmth, trust | 15 | figures (role figure), faces turned away, Indian-context clothing |
| clay_props_3d | joy, energy | 20 | soft clay props, yellow/teal/coral/cream palette (no magenta) |
| ocean_layers | wonder, curiosity, awe | 10 (+5 hero loops) | cinematic glossy 3D ocean-science objects; spec flag `neutralize_violet` removes every violet/pink tint (scripts/neutralize_tint.py) |
| grounds | all | 12 | full-frame 9:16 texture plates (role texture; the planner never matches textures, so they are library material for a later background-plate feature) |

QA: 78 PASS, 4 benign WARN (arms/hand entering from the frame edge, a 3.3:1 column, a full-bleed card). 410 credits. Demos: `examples/library_09/*.intent.json` (compile with `--asset-family <family>`).
