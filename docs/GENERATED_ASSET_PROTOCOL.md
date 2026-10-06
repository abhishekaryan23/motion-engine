# Generated asset protocol (0.5)

MotionEngine never calls an image generator. It writes **what it needs**,
any external tool makes the files, and MotionEngine **ingests, analyzes and
places** them. No vendor SDK, model or network call lives in the engine.

```
CreativeIntent + StyleProfile
   │ motion-engine plan-assets            (semantic: AssetPlan, 0.4)
   ▼
AssetPlan ── motion-engine asset-prompts ──▶ asset-prompts.json   (AssetPromptSet)
                                                   │
                        EXTERNAL GENERATOR (any vendor, MCP, script, human)
                                                   │ writes <file_stem>.png|.jpg [+ <file_stem>.json]
                                                   ▼
                                           delivery directory
   motion-engine ingest-assets asset-prompts.json <delivery dir> -o manifest.json [--cache DIR] [--replace ID]
                                                   │ decode · validate · analyze · cache · QA
                                                   ▼
                                  AssetManifest v0.2 (analysis included)
   motion-engine validate-assets manifest.json [--prompts asset-prompts.json]    (preflight QA, re-runnable)
   motion-engine compile intent.json --style s.json --asset-manifest manifest.json -o out.motion.json
```

## AssetPromptSpec (`motion_core::assets`)

Deterministic art direction derived from the plan by
`motion_core::asset_prompts::prompt_specs(&AssetPlan) -> AssetPromptSet`.
It is **not** CreativeIntent and weak models never write it.

| field | meaning |
|---|---|
| `id` | the first request id it serves (`beat_1.hero_subject`) |
| `fingerprint` | `fp1-<16 hex>`, see below |
| `continuity_key` | the depicted entity (`office_worker`) |
| `serves` | every request id this one image fulfils, plan order (includes `id`) |
| `role`, `priority` | role of the first request; `required` if any served request is |
| `subject`, `context` | the intent's words (request subject; beat statement verbatim) |
| `presentation`, `background` | from the request |
| `style` | the plan's `AssetStyleProfile` (copied into every spec so it is self-contained) |
| `composition` | `{framing, negative_space, subject_whole, head_inside_frame}` — constraints **inside** the image, never canvas placement |
| `avoid` | fixed list per presentation/framing |
| `output` | `{file_stem, alpha_required, min_short_side, aspect}` |
| `prompt` | one plain-language string assembled from the fields (for generators that take a single prompt; not fingerprinted) |

### Derivation rules (`prompt_specs`)

1. Only requests with `source: generated_image` produce specs (`svg` / `user_asset` come from the library).
2. **Grouping.** Requests are grouped by `(continuity_key, framing, presentation, background)`; each group is one spec. A request without a continuity key uses `continuity_key(subject)` (first four alphanumeric words, lowercased, `_`-joined). Groups are ordered by their first request in plan order; `serves` lists the group's request ids in plan order. The spec copies `subject`, `context`, `role`, `presentation`, `background`, `negative_space` from the **first** request of the group.
3. **Framing** by role: `hero_subject`, `foreground_occluder` → `half_figure`; `portrait` → `head_and_shoulders`; `hero_object`, `supporting_object`, `transition_object` → `object`; `evidence_image` → `artifact`; `environment` → `scene`.
4. **Negative space** is kept only for non-cutout presentations (`full_frame`, `background_plate`); `isolated_cutout` and `flat_artifact` → `none`. The engine makes negative space by *placing* a cutout; the image itself must not bake it in. (This is what lets one cutout serve several scenes.)
5. `subject_whole` = presentation is `isolated_cutout` or `flat_artifact` **and** framing ≠ `half_figure` (half figures may be cut at the bottom edge only). `head_inside_frame` = framing ∈ {`full_figure`, `half_figure`, `head_and_shoulders`}.
6. **Output.** `alpha_required` = background `transparent`. `min_short_side`: cutouts 1024, `background_plate`/`full_frame` 1080, `flat_artifact` 900. `aspect`: `half_figure` `3:4`, `full_figure` `2:3`, `head_and_shoulders` `4:5`, `object` `1:1`, `artifact` `3:4`, `scene` `3:4`. `file_stem` = `<continuity_key>-<first 10 hex of the fingerprint hash>` (e.g. `office_worker-3fa91c2e07`).
7. **Avoid** (in this order): always `embedded text`, `letters or numbers`, `logos`, `watermark`, `signature`, `border or frame`; cutouts add `background scenery`, `cast shadow on the ground`; `head_inside_frame` adds `cropped head`; people framings add `extra fingers`, `distorted hands`.
8. **Prompt** (deterministic, one line per clause, joined by a space):
   `"{Subject}."` · `"Context: {context}"` (when non-empty) · presentation clause (`"Isolated photographic cutout on a fully transparent background."` / `"Full-frame photograph."` / `"Flat document-like artifact, photographed straight on."` / `"Soft environmental background plate."`) · framing clause (`"Half figure, head to waist, head fully inside the frame with headroom."`, …) · negative-space clause when ≠ none (`"Leave empty space on the right."`) · `"Style: {medium}; {realism}; {lighting}; {contrast}; {palette_tendency}; {edge_treatment}; {camera_feel}."` · `"Avoid: {avoid joined by ', '}."`
9. `fingerprint = spec.compute_fingerprint()` computed **after** all other fields except `output.file_stem` (the stem derives from it).

### Fingerprint (FROZEN v1)

`fp1-` + FNV-1a-64 (hex) of `AssetPromptSpec::canonical_art_direction()`:
normalized subject, presentation, background, framing, negative space,
subject_whole, head_inside_frame, all nine style fields, alpha_required,
min_short_side, aspect, avoid. **Excluded:** id, serves, role, priority,
continuity key, context, prompt, file stem. Same entity + same art direction
⇒ same fingerprint ⇒ reuse, across scenes and across runs. Editing a
statement does not regenerate the image; changing the style does.

## Delivery contract (what a generator writes)

In the delivery directory, per spec:
- `<file_stem>.png` — required when `output.alpha_required` (real alpha, transparent background); otherwise `.png`, `.jpg` or `.jpeg`.
- optional sidecar `<file_stem>.json` = `GeneratedSidecar {fingerprint?, face_bounds?, head_bounds?, face_anchor?, subject_anchor?, generator{}}` (normalized 0..1 image coordinates). A sidecar fingerprint that differs from the spec is a FAIL.
- A spec may be left undelivered (optional images simply fall back).

## Ingestion (`motion_render::ingest::ingest`)

For each spec in order:
1. **Find** `<delivery>/<file_stem>.{png,jpg,jpeg}` (lowercase extensions; more than one match → FAIL `delivery`).
2. **Not delivered:** if the cache has the fingerprint → reuse the cached entry (no regeneration). Else → `missing[] {id, priority, reason: "not delivered"}`.
3. **Delivered:** decode (PNG/JPEG, magic-byte sniffed), read the sidecar, **auto-key** (0.10 Q: an opaque delivery for a transparent-cutout request whose border is one flat colour becomes an alpha cutout cropped to the subject, see docs/IMAGE_TREATMENTS.md; the manifest entry then points at `<stem>.keyed.png` next to the manifest — or the cache file — with `keyed: true`), analyze (`analysis::analyze`, `person` = framing is a people framing), run `asset_qa::check_entry`. Any FAIL → not in `assets`, recorded in `missing` with the failing check as reason.
4. **Cache** (when `--cache DIR`): copy the file to `DIR/<fingerprint-without-fp1->.<ext>` and insert a `CacheEntry`. Same fingerprint with *different* content is refused (`CacheConflict::ContentChanged` → FAIL `fingerprint`) unless the spec id or fingerprint is listed in `--replace` (explicit regeneration, reported as `Replaced`). The index is written as `DIR/index.json` (entries sorted by fingerprint). Nothing is ever silently replaced.
5. **Manifest entry:** `id` = spec id, `serves`, `fingerprint`, `continuity_key`, `content_hash` (`fnv1a64:<hex>` of the bytes), `width`/`height` (decoded), `alpha` (decoded has transparency), `analysis`, face/head bounds + anchors from the sidecar, `subject_anchor` = sidecar or subject-bounds center, `face_anchor` = sidecar or the head estimate's center, `generator` = sidecar `generator`. `path` = relative to the manifest directory (the cached copy when caching, else the delivered file).

The manifest version is `"0.2"`. A 0.4 hand-written manifest (`"0.1"`) still loads.

## Required vs optional

- **optional** missing/invalid → listed in `missing` (WARNING); the compiler builds the image-free composition (e.g. EditorialCollage instead of TypeImageInterlock). The video never fails for an optional image.
- **required** missing/invalid → listed in `missing` (FAIL); `ingest-assets` exits non-zero and `compile_with_assets` returns `CompileError::MissingRequiredAsset` **before** rendering.
- Compiling **without** a manifest means "no generation attempted": every beat compiles with procedural plates (0.4 behavior).

## CLI

```
motion-engine asset-prompts <asset-plan.json> [-o asset-prompts.json]
motion-engine ingest-assets <asset-prompts.json> <delivery-dir> -o <manifest.json> [--cache DIR] [--replace ID|FINGERPRINT]... [--json]
motion-engine validate-assets <manifest.json> [--prompts asset-prompts.json] [--json]
```
`ingest-assets` and `validate-assets` print the preflight report and exit 1 when any FAIL is present.
