# MotionEngine Showcase Samples

This directory contains lightweight sample videos rendered directly by MotionEngine (720x1280 vertical video format, H.264 + AAC audio) demonstrating various layout genres, semantic grammars, voice alignment, and sound design.

---

### Sample Videos

| File | Genre / Grammar | Key Features |
| :--- | :--- | :--- |
| **`documentary_money_720p.mp4`** | Documentary / Dossier | Vox-style data journalism, clipping containers, redlines, stat cards, figure highlights, and synchronized speech. |
| **`countdown_tense_720p.mp4`** | Kinetic / Tense | High-energy countdown, precision audio-visual hits, dramatic typography, and fast-paced transitions. |
| **`layers_documentary_720p.mp4`** | Layers Grammar | Vertical hierarchical layer focus, depth planes, and structured conceptual breakdown. |
| **`short_state_change_720p.mp4`** | State Change Grammar | High-velocity state transitions, before/after comparisons, and directional kinetic motion. |

---

### Generating Your Own Videos

You can render any of the included examples using the `motion-engine` CLI:

```bash
# 1. Compile intent + style profile into a MotionScene graph
motion-engine compile examples/public/minimal-emphasize.intent.json \
  --style examples/public/minimal.style.json \
  -o output/scene.motion.json

# 2. Render to MP4
motion-engine render output/scene.motion.json \
  --out-dir output/render/
```
