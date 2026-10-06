# Golden files

Small hand-written MotionScenes (30 fps, `asset_root: ../assets`; the original five are 540x960, the motion-language ones 1080x1920) plus intents. Compiled goldens (the five v0.2 ones and all 0.4 ones) are compiled from the matching `golden/*.intent.json` with `examples/public/minimal.style.json` (`compile <intent> --style examples/public/minimal.style.json --output golden/<name>.motion.json`); `type_image` adds `--asset-manifest assets/test_manifest.json`. Recompile them after intentional compiler changes (0.4 recompiled the v0.2 five).
Each must pass `motion-engine validate golden/<file>`.

| File | Covers |
|---|---|
| `typography.motion.json` | every font role, left/center/right align, `\n` multi-line, letter spacing (positive and negative), `max_width` wrapping, uppercase, italic serif |
| `movement.motion.json` | `move`, `scale` (both and `x` axis), `rotate`, `fade`, several easings (incl. springs), a group with a moved child |
| `masking.motion.json` | `mask_reveal` in all four directions, one `conceal`, `clip_reveal` up/left on groups and text |
| `texture.motion.json` | full-frame paper, grain overlay, halftone patch, flat |
| `overlap.motion.json` | two overlapping scenes, a shared element whose track spans both, an `accent_expand` rectangle |
| `layout_dynamic_center.motion.json` | 1080x1920. Layout-bound text on cards whose geometry animates (`accent_expand` shrink and grow, `move` + `scale`), center/center with ink bounds, left/top with padding, right/bottom with padding + offset, and a bound child inside a rotating/scaling group |
| `minimal_motion.motion.json` | 1080x1920. Restrained editorial language: `mask_reveal` headline lines with a short OutQuint rise, a revealed rule, a fading caption, one small linear camera push; no springs |
| `kinetic_typography.motion.json` | 1080x1920. Kinetic language: word cascade, a keyword punch (per-word scale/fade/move) and a type-replace, expanded into primitive text layers and motions |
| `stagger.motion.json` | 1080x1920. Sequential language: irregular stagger of columns and rows (`fade` + `move` with staggered starts) |
| `parallax.motion.json` | 1080x1920. Scene camera (`push` + `track`, InOutCubic) over four depth planes (0.35 halftone + ghost word, 0.7 tilted cards, 1.0 headline, 1.45 accent shapes); planes move at different rates with no per-plane motions |
| `data_viz.motion.json` | 1080x1920. Data language: `count` hero number, growing bar chart, comparison progress bars (`accent_expand`), a `trim` path draw |
| `collection_accumulate.motion.json` | 1080x1920. Compiled from `collection_accumulate.intent.json` (v0.2 `collection` + `accumulate`, 4 phrase items): items placed one by one into a growing arrangement |
| `state_change.motion.json` | 1080x1920. Compiled from `state_change.intent.json` (v0.2 `state_change`, before/after states with a `meaning` label) |
| `dual_state_change.motion.json` | 1080x1920. Compiled from `dual_state_change.intent.json` (primary + secondary `state_change`, contrast) |
| `derived_metric.motion.json` | 1080x1920. Compiled from `derived_metric.intent.json` (v0.2 `derived_metric`, 80 / 1000 as a percent) |
| `derived_metric_compare.motion.json` | 1080x1920. Compiled from `derived_metric_compare.intent.json` (primary + secondary `derived_metric`, compared) |
| `scene_lifecycle.motion.json` | 1080x1920, 0.4. Two beats (EditorialCollage → SplitContrast) with `Scene.lifecycle`: primary in ENTER, secondary at EVOLVE, stage anticipation into the bridge |
| `read_phase_evolution.motion.json` | 0.4. Atomic explain (SequentialStack): body, support card and accent rule arrive one per evolve event; earlier lines dim |
| `shared_motion_target.motion.json` | 0.4. A revealed number carried into the next beat: `count` and an emphasis `scale` pulse targeting the shared element, which moves/scales through the track and camera |
| `editorial_collage.motion.json` | 0.4 EditorialCollage: tilted card + halftone, serif secondary and underline at EVOLVE |
| `split_contrast.motion.json` | 0.4 SplitContrast (atomic): two panels, divider, separate / replace (type_replace) with a connective word |
| `sequential_stack.motion.json` | 0.4 SequentialStack: four phrase items, one per evolve event |
| `data_story.motion.json` | 0.4 DataStory: counted number pair + comparison bar + direction line; bar chart series + path draw + peak pulse |
| `kinetic_poster.motion.json` | 0.4 KineticPoster: oversized headline, keyword emphasis at EVOLVE, tracking reveal, type replace |
| `multiplane.motion.json` | 0.4 CinematicMultiplane: background/midground/foreground planes under the camera |
| `hero_object.motion.json` | 0.4 HeroObject: dominant library object, stamp + serif at EVOLVE |
| `evidence_stack.motion.json` | 0.4 EvidenceStack: tilted evidence card, source label, highlight sweep, note card |
| `type_image.motion.json` | 0.4 TypeImageInterlock with a delivered test image (`assets/test_manifest.json`): back lines behind the cutout, front line over it, caption card |
| `semantic_compile.intent.json` | a 3-beat CreativeIntent (explain + object, compare + separate, emphasize + phrase), compiled and rendered by the render tests |

## Regression hashes

`hashes.json` holds an FNV-1a 64 hash of the middle frame's RGBA pixels, keyed
`"{file}@{os}-{arch}"`. Tests compare only when an entry exists for the current
platform (rasterization can differ across platforms). After an intentional
rendering change, look at the frames, then regenerate:

```
MOTION_UPDATE_GOLDEN=1 cargo test -p motion-render --test golden
```

This adds/updates entries for the current platform only.
