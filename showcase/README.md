# Showcase

Videos rendered by MotionEngine. Each GIF is a short preview; the MP4 next to it is the full clip
(H.264 + AAC, 720p class). Every frame was produced by the engine from a program, a brief or an
intent in this repository; nothing was edited by hand.

## MotionEngine 2 (programs and briefs)

| file | what it shows |
|---|---|
| `capability_highlights_720p.mp4` (64 s) | One clip per technique: kinetic type and type styles, charts, rail and card diagrams, product-UI flows, code, photo stacks, sketches, layering, shapes, lockups. Each clip is labelled with its source program. |
| `capability_highlights_preview.gif` | Six moments from the highlights: type, a chart, a payment flow, a sketch, layering, a lockup. |
| `charts_diagrams_preview.gif` | A line chart, an area chart, a rail diagram and a card diagram. |
| `product_ui_preview.gif` | Typed payment, promo and sign-up flows on device frames. |
| `type_styles_720p.mp4`, `type_styles_preview.gif` | Gradient, image and outline fills, case control, and doodles drawn with the type (`examples/v2/type_styles` shows the same features). |
| `layering_before_after_720p.mp4`, `layering_before_after_preview.gif` | One program rendered before (left) and after (right) intentional layering (`over`) and plate-less sketches. |
| `generated_assets_720p.mp4`, `generated_assets_preview.gif` | A bundled texture (left) against a generated image plate used as a type fill (right); see `docs/PROVIDERS.md`. |

## v1 narrated reels (CreativeIntent)

720×1280 vertical reels with a voice-over, word-synced captions and a music bed.

| file | genre | what it shows |
|---|---|---|
| `documentary_money_720p.mp4` | documentary | Paper clippings, figure cards and stamps timed to the narration. |
| `layers_documentary_720p.mp4` | layers | Persistent labelled strata, with the lit layer in the same place every beat. |
| `short_state_change_720p.mp4` | state change | Before/after values replacing in place, with a running comparison. |

## Making your own

```bash
cargo build --release -p motion-cli
./target/release/motion-engine program render examples/v2/type_styles.program.json
./target/release/motion-engine make --brief "A 15 s vertical about how cold brew orders grew from 12k to 31k."
./target/release/motion-engine reel examples/voice_10/why_we_nap.intent.json --tts-model say   # offline macOS voice
```

The previews in this folder were cut from local renders with `scripts/release/build_showcase.sh`
(ffmpeg only).
