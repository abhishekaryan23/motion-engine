# Asset analysis & preflight QA (0.5)

Measured **once at ingestion** (`motion_render::analysis::analyze`), stored
normalized in the manifest (`ManifestEntry.analysis`), read by the compiler.
No pixel scanning at render time, no ML runtime, no face detector. External
analyzers may supply `face_bounds` / `head_bounds` through the sidecar; the
engine prefers them over its own estimate (`ManifestEntry::head_region`).

All coordinates are fractions of image width/height, origin top-left.
"Subject pixel" = alpha ≥ `alpha_threshold` (default 16).

## AssetAnalysis

| field | definition |
|---|---|
| `subject_bounds` | tight box of subject pixels: `x = min_x / W`, `width = (max_x + 1 - min_x) / W` (same for y). Image with no transparent pixel ⇒ `{0,0,1,1}`. Image with no subject pixel ⇒ `{0,0,0,0}` is not representable: use `{0,0,1,1}` and `coverage = 0`. |
| `edges` | `top` = some subject pixel in rows `[0, m)`, `bottom` rows `[H-m, H)`, `left`/`right` columns likewise, `m = max(1, round(0.01 · side))`. Opaque images: all true. |
| `coverage` | subject pixels / all pixels. Opaque ⇒ 1. |
| `occupancy` | `grid × grid` (16) cells, cell `(c, r)` covers pixel columns `[floor(c·W/g), floor((c+1)·W/g))` (rows likewise; empty ranges clamp to ≥ 1 px). Value = `round(15 · subject pixels / cell pixels)` as one lowercase hex digit. `rows[0]` is the top row. |
| `safe_regions` | see below. Opaque images ⇒ empty. |
| `head_estimate` | people only (`person = true`) with transparency, see below. |
| `monochrome` | (0.9) flat single-ink artwork; the compiler recolours such cutouts to the palette ink. |
| `mean_color` | (0.10 Q) alpha-weighted mean sRGB of the subject pixels, `[r, g, b]`, rounded: `round(255 · Σ premultiplied / Σ alpha)` over pixels with alpha ≥ threshold (every pixel when opaque). Absent in manifests written before 0.10. The compiler and layout QA judge subject/ground contrast from it. |

### Safe regions

Eight named zones (image fractions `x, y, w, h`):
`left_upper (0, 0, .5, .5)`, `right_upper (.5, 0, .5, .5)`, `left_middle (0, .25, .5, .5)`,
`right_middle (.5, .25, .5, .5)`, `lower_left (0, .5, .5, .5)`, `lower_right (.5, .5, .5, .5)`,
`top (0, 0, 1, 1/3)`, `bottom (0, 2/3, 1, 1/3)`.

For each zone, over the occupancy cells whose centers lie inside the zone,
find the **largest-area axis-aligned rectangle of cells whose values are all
≤ 2** (maximal rectangle in a binary grid; ties → the top-most, then
left-most). Its normalized rect is `rect`, `occupancy` = mean cell value / 15
inside it, `score = area(rect) · (1 − occupancy)`. Keep regions with
`area ≥ 0.04`; sort by score descending, then by name order above. Deterministic.

### Head estimate (alpha silhouette)

For a person cutout: let `y0, y1` = subject rows, `Hs = y1 − y0`. For every row
`y ∈ [y0, y0 + 0.5·Hs]` compute the row span width `w(y)` (max − min subject
column). Smooth with a 3-row moving average.
- **Neck** = the row `yn ∈ [y0 + 0.08·Hs, y0 + 0.45·Hs]` with the minimal smoothed width, accepted only if `w(yn) ≤ 0.8 · max w(y)` over `[y0, yn]` **and** some row below `yn` (within `0.5·Hs`) is ≥ `1.3 · w(yn)` (shoulders).
- Head box = rows `[y0, yn]`, columns = union of row spans in those rows.
- No accepted neck ⇒ head = rows `[y0, y0 + 0.22·Hs]` with their column union (a head-and-shoulders crop or a figure seen from behind still gets a conservative top region).

## Preflight QA (`motion_render::asset_qa`, `motion-engine validate-assets`)

Each asset gets named checks with PASS / WARNING / FAIL. Any FAIL on a
required asset stops the pipeline before rendering.

| check | FAIL | WARNING | PASS message example |
|---|---|---|---|
| `file` | file missing/unreadable | — | |
| `decode` | not PNG/JPEG, corrupt, zero size | — | `png 1024×1365` |
| `manifest` | entry width/height ≠ decoded | id not in the prompt set; file content changed since ingestion (`content_hash` mismatch, `validate-assets`) | |
| `duplicate` | duplicate id or a request served twice (`AssetManifest::validate`) | — | |
| `fingerprint` | entry/sidecar fingerprint ≠ spec fingerprint; cache content conflict without `--replace` | — | |
| `resolution` | short side < 512 | short side < `output.min_short_side` (default 1024) | |
| `aspect` | long/short > 5 | long/short > 3 | |
| `alpha` | `alpha_required` and no transparency, or coverage > 0.98 | alpha present but not required (opaque request) | |
| `edges` | — | cutout touches top when `head_inside_frame`; touches left/right when `subject_whole`; touches bottom when `subject_whole` (half figures may touch bottom) | `subject clear of edges` |
| `head_headroom` | — | head region top `y < 0.03` | |
| `occupancy` | — | cutout subject width > 0.9 or height > 0.97 of the image, or coverage > 0.75 (message: `subject occupies 78% width`) | |
| `safe_region` | — | opaque full-frame image with no safe region of score ≥ 0.08 | cutouts: `type placed outside the silhouette by the engine; best in-image region: left_upper`; background plates: type stays on engine planes above it |
| `face_metadata` | — | supplied face/head box not inside `subject_bounds` grown by 0.02 | |
| `delivery` | required spec not delivered; several files for one stem | optional spec not delivered (`falls back to the image-free composition`) | |

Report text format (one block per asset):
```
beat_1.hero_subject  WARNING
  PASS     resolution: 1024×1365 (min 1024)
  PASS     alpha: transparent cutout (coverage 41%)
  WARNING  occupancy: subject occupies 78% width
  PASS     safe_region: type placed outside the silhouette by the engine; best in-image region: left_upper
  WARNING  head_headroom: head is close to the upper edge
assets: 1 (0 fail, 1 warning)
```
