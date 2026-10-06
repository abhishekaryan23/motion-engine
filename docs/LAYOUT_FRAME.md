# LayoutFrame — responsive canvas (0.9)

The canvas is an **operator** input. CreativeIntent `format` (vertical / square /
landscape) stays the default; `compile` and `explore` accept

```
--canvas 1080x1350          # any W×H: even sides, short side ≥ 320, 0.4 ≤ W/H ≤ 2.5
--aspect portrait           # story 9:16 · portrait 4:5 · square 1:1 · landscape 16:9
                            # cinema 21:9 · tall / fold_cover 9:21 · tablet 3:4 · fold_inner 6:5 · any a:b
```
(presets: short side 1080, long side rounded to even). No flag → byte-identical to 0.8.

## The frame (`compiler/layout_frame.rs`)
Derived from (W, H) only and stored on the compile context as `ctx.frame`:

| field | meaning |
|---|---|
| `u` | short side / 1080 (the unit every builder already uses) |
| `aspect` | W / H |
| `class` | nearest legacy layout family: < 0.75 vertical · 0.75–1.333 square · > 1.333 landscape (log-aspect midpoints between 9:16, 1:1, 16:9) |
| `weights` | continuous (tall, square, wide) memberships, piecewise linear in ln(aspect) between the anchors 9:16, 1:1, 16:9 |
| `blend(t, s, w)` | per-class constant, exact at the anchors, smooth in between |
| `safe` | canvas inset by 84u (`Ctx::margin`) |
| `regions` | QA zones: headline, subject, caption, furniture lanes |
| `min_type_px` | 20u — the legibility floor used by guardrails and layout QA |

At 1080×1920, 1080×1080 and 1920×1080 every value equals the 0.8 constants, so
legacy goldens are unchanged by construction.

## Builder rules
- Discrete layout choices branch on `class` (`is_landscape()`, `is_square()`),
  never on `w > h` / `w == h`. Stacked vs side-by-side uses
  `side_by_side = landscape || (square && aspect > 1.05)` so 6:5 and 4:3 lay
  out like landscape (a stacked secondary note fell off 1296×1080).
- Sizes and fractions are expressions in `w`, `h`, `u` and `frame.safe`; per-class
  numbers become `frame.blend(...)`.
- Tall canvases (h > 1920u) lay out on an effective height and offset the content
  block so the composition is not top-heavy; furniture lanes stay on the real edges.

## Layout QA (`motion_core::layout_qa`, `qa <scene> --layout`)
At each beat's READ time and READ/EVOLVE midpoint: text inside the safe area,
no text clipped by the canvas, text ≥ `min_type_px`, delivered subject images at
least 60 % on canvas, no non-decorative layer fully off-canvas. Backdrop scene,
ghost words, colour fields, grain and edge furniture are exempt by design.

### Subject checks (0.10 Q, `motion_core::subject_qa`)
`layout_report_with(project, frame, &ImageIndex)` adds four checks on delivered
images; the index gives what the analysis measured per asset path (alpha bounds,
head region, mean colour): `motion_render::image_index(project, base_dir)` builds
it from the files (`qa --layout` does), `ImageIndex::from_manifest` from a
manifest. Without an index (`layout_report`) they are skipped.

| check | rule |
|---|---|
| `text_over_subject` | text drawn over a subject covers ≤ 4 % of its alpha bounds, never any of its head region (even behind it). Text on its own opaque card or strip is a label: exempt from the 4 % rule, not from the head rule |
| `subject_too_small` | a beat's hero subject (`<beat>.subject`, carried `shared.asset.*`) covers ≥ 16 / 14 / 12 % of the canvas (tall / square / wide) |
| `frame_too_loose` | a card behind a subject exceeds its alpha bounds by ≤ 12 % of the subject size per side |
| `asset_low_contrast` | the treated mean colour has ≥ 2:1 against what touches it (ground, card, paper edge), or it carries a sticker / keyline that does |

Evaluated at READ, the READ/EVOLVE midpoint and the end of EVOLVE (slow drifts —
the READ push, a parallax glide — are not "in flight"). Known limit: SUBJECT_MIN_AREA
is geometrically out of reach for a very narrow cutout (the 0.31-aspect student)
on wide canvases, and on square ones once a voice-over reserves the caption lane;
the layouts then keep the biggest subject they can and QA reports it.

## Aspect sweep
`crates/motion-core/tests/aspect_sweep.rs`: aspects {0.43, 0.5625, 0.75, 0.8,
1.0, 1.2, 1.333, 1.778, 2.37} × demo/golden intents × three tastes → compile,
validate, layout QA PASS, no motion outlives its scene.
`crates/motion-core/tests/canvas_builders_a.rs`: nine canvases × tastes for the
image/hero/evidence/type-image/kinetic/data grammars, legacy `Some(legacy)` ≡ `None`.

## Status (0.10 Phase A)
Builder set B is canvas-agnostic; `placement::stack_fit` and `side_by_side` feed
SplitContrast variants (6:5 and 4:3 split left/right); tall canvases use
`frame.content_h()` / `content_y0()` (h_eff = 1920u, block offset (h − h_eff)×0.45).
Aspect sweep green (8 tests, 9 aspects × goldens × 3 tastes). `note.tab` is
exempt as decorative (a stage-reframe zoom pushes the 10u tab off-canvas).
Known legacy finding kept for byte-identity: editorial_demo beat 2's rotated
stamp crosses the safe edge by ≈ 60 px at 9:16 (and 9:21), still on canvas.
Sheet: `output/sheets/gateA_editorial_demo_aspects.png`.
