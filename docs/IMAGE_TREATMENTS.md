# Image treatments (0.5)

Generated images must look like they belong to the piece, not pasted in.
A treatment is a small set of **explicit, deterministic pixel operations**
(`scene::ImageTreatment`) on an `image` layer. The compiler chooses the preset
(`compiler::treatment::preset_for`, from StyleProfile + asset role) and writes
its explicit parameters (`treatment_for`, from the palette); the renderer
executes them once when it prepares the layer (`motion_render::treatment::apply`)
— no semantic logic in the renderer, no per-frame cost.

## Operation order

Colour ops touch RGB only (un-premultiplied), alpha is preserved:
1. `desaturate` d: `c = mix(c, luma, d)`, luma = 0.2126 R + 0.7152 G + 0.0722 B.
2. `brightness` b, `contrast` k: `c = (c − 0.5) · k + 0.5 + b`, clamped.
3. `duotone {shadow, highlight, amount}`: `c = mix(c, mix(shadow, highlight, luma), amount)`.
4. `tint {color, amount}`: `c = mix(c, color, amount)`.
5. `grain` g: `c += g · (n − 0.5) · 0.5`, n = deterministic hash noise of `(seed, x, y)` in 0..1, same n for R, G, B.
Silhouette ops (cutouts; grow the drawn area):
6. `edge {color, width}`: dilate alpha by `width · px_per_unit` pixels (disc), draw the dilated silhouette in `color` *under* the image.
7. `shadow {color, offset, blur}`: the (edge-dilated) silhouette, offset by `offset · px_per_unit`, box-blurred 3× with radius `blur · px_per_unit / 2`, multiplied by `color` alpha, drawn under everything.
The output pixmap is padded by `pad = ceil(max(edge width, |offset| + blur) · px_per_unit) + 2` on every side (0 when neither op is set); the renderer draws it shifted by `−pad` image pixels.

## Presets (`preset_for`)

| role | style | preset |
|---|---|---|
| environment | texture `heavy_print` | EditorialMonochrome |
| environment | otherwise | EditorialDuotone (ink → paper) |
| evidence image | any | MutedDocumentary |
| people / objects | texture `heavy_print` | EditorialMonochrome |
| people / objects | family `editorial_collage` + material `paper` | PaperCutout |
| people / objects | texture `none` | Natural |
| people / objects | otherwise | MutedDocumentary |

People are never duotoned (StyleProfile has no explicit duotone request).

## Preset parameters (`treatment_for(preset, palette, seed, u, alpha)`)

| preset | desaturate | brightness | contrast | duotone | tint | grain | edge (cutouts) | shadow (cutouts) |
|---|---|---|---|---|---|---|---|---|
| Natural | 0 | 0 | 1 | — | — | 0 | — | — |
| EditorialMonochrome | 1 | 0.02 | 1.18 | — | ink @ 0.06 | 0.06 | — | ink α 0x30, (0, 8u), blur 12u |
| EditorialDuotone | 0 | 0 | 1.08 | ink → paper, 0.9 | — | 0.05 | — | — |
| PaperCutout | 0.18 | 0.01 | 1.06 | — | paper @ 0.05 | 0.04 | card colour, 7u | ink α 0x38, (5u, 9u), blur 14u |
| MutedDocumentary | 0.35 | 0 | 0.94 | — | paper @ 0.08 | 0.035 | — | ink α 0x22, (0, 6u), blur 16u |

Opaque images never get edge or shadow. The engine's animated film-grain
overlay still sits above everything; the static grain here only breaks the
"too clean" generator look inside the image.

## Sticker and keyline (0.10 Q)

A subject that vanishes into its ground — a dark object on a dark page, white
clothing on cream paper — gets a contrast treatment. `ImageTreatment.sticker`
(`{color, width_px}`, serde default, skipped when absent: old scenes are
unchanged) replaces the paper edge and the shadow:

8. `sticker {color, width_px}`: dilate alpha by `width_px · px_per_unit` (disc),
   fill it with `color` under the image, over a soft shadow (10 % black, offset
   `0.5 · width` down, blur `1.5 · width`). Deterministic; same operation as the
   paper edge, so a cutout's silhouette decides the shape.

**The rule** (`compiler::subject_rules::guard_contrast`, applied to every delivered
image in `plate::image_layer`; the same functions drive layout QA): the image's
`AssetAnalysis.mean_color` (alpha-weighted mean sRGB measured at ingestion)
is taken through the treatment's colour operations (`treated_color`), and its WCAG
contrast is compared with what touches the silhouette — the paper edge's colour
for a cutout that has one of at least 2 u, otherwise the page, the card stock and
the palette's colour fields. Below `ASSET_GROUND_MIN_CONTRAST` (2:1):

* cutout → `sticker` of 8 u in paper or ink, whichever contrasts more with the
  subject (the paper edge and shadow are dropped);
* opaque image → a 3 u keyline (`edge`) in the same colour.

Manifests written before 0.10 carry no `mean_color`; nothing is decided for them
(re-run `ingest-assets` to add it, the library families already have it).

## Auto-key (0.10 Q)

`motion_render::autokey`, run by `ingest-assets`: an opaque delivery for a request
that asked for a transparent cutout whose border is one flat colour (≥ 90 % of
border pixels within 0.08 of the median, colour distance = RGB Euclidean / √3)
is keyed into an alpha cutout (the algorithm of `scripts/key_cutout.py`,
without its magenta-specific despill): background = near-ground pixels
(distance < 0.38) 4-connected to the border plus enclosed pockets of near-exact
ground (< 0.08), soft band 0.18–0.38, specks under 0.15 % of the frame dropped,
ground un-mixed from edge pixels, cropped to the alpha bounds + 6 % of the
longer side and padded with transparency to the 512 px minimum short side. The
file is written next to the manifest (`<stem>.keyed.png`, or into the cache), the
entry points at it with `keyed: true`, analysis describes the cutout, and sidecar
face/head boxes are re-expressed in it. Photographs with busy borders and requests
for opaque images are untouched.
