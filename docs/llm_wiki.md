# MotionEngine LLM Wiki

## 1. System Persona & Purpose
MotionEngine is a deterministic Rust-based editorial motion graphics engine. It empowers AI authors and developers to generate broadcast-quality video stories (1080x1920 vertical reels, 1:1, or 16:9) from small, semantic `CreativeIntent` JSON specifications.

## 2. Repository Map
- `crates/motion-core`: Intent/style schemas, compiler, timeline evaluation, life-cycle manager, typography registry, asset catalogs.
- `crates/motion-render`: Frame rendering (tiny-skia), text engine (cosmic-text), texture generation, FFmpeg export, and audio mixing engine.
- `crates/motion-cli`: The unified `motion-engine` CLI (`compile`, `voice`, `plan-assets`, `validate`, `render`, `qa`, etc.).
- `assets/fonts`: OFL curated fonts across editorial, technical, and display genres.
- `assets/library`: Named cutout and vector asset collections (`people_everyday`, `editorial_cutout`, `classical_greyscale`, `halftone_retro_objects`, etc.).
- `assets/sfx`: Foley, interface, paper, and cinematic SFX packs.
- `docs/`: Specification, developer guidelines, and architecture documentation.

## 3. Key Domain Entities
- **`CreativeIntent v0.2`**: Defines story beats, purposes (`emphasize`, `compare`, `contrast`, `reveal`, `explain`), statements, and subjects (`phrase`, `number`, `object`, `collection`, `state_change`, `derived_metric`).
- **`StyleProfile`**: Expresses visual taste via 5 high-level dimensions: `tone`, `polarity`, `temperature`, `temperament`, `density`, plus `accent_role`.
- **`MotionScene`**: The compiled, fully deterministic layout and animation graph consumed by the renderer.
- **`AudioPlan`**: Scheduled sound design cues (transients, handoffs, impacts) aligned off speech word-onsets.

## 4. Active Guardrails & Principles
- **No Direct Geometry/Timing in Intent**: AI authors specify meaning and purpose, never pixel coordinates, durations, or easing.
- **Deterministic Compilation**: Identical inputs yield identical frames.
- **Broadcast Audio Standards**: Master video mixes automatically conform to -16.0 LUFS with controlled peak levels.
