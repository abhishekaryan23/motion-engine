# MotionEngine v2.1 — LLM Context Wiki (`docs/llm_wiki.md`)

## 1. System Persona & Purpose
MotionEngine is an **AI-native, deterministic Rust Video & Image Editor and 2D / 2.5D / Natural Media Motion Graphics Engine**.
Its goal is to allow any AI agent harness, local small model, or human creator to produce studio-grade videos, static editorial posters/images, `p5.brush` natural media & watercolor animations, Golden Ratio ($\varphi = 1.618$) layouts, product UI animations, and data visualizations with **minimal token effort** (a 1-sentence brief or a ~50-line `MotionProgram` v2.1 JSON) and **zero animation math**, plus an optional **Minimal Studio Inspector & Brand Kit UI** (`serve` / `revise` / live preview) for manual human-in-the-loop tweaking of font styles, color palettes, branding, and copy.

---

## 2. Repository Map

| Path | Responsibility |
|---|---|
| `README.md` | Setup, quick start, CLI and MCP overview, project layout. |
| `ARCHITECTURE.md` | Data flow, crate boundaries, determinism rules. |
| `docs/DESIGN_RULES.md` | Taste gates, colour roles, UI kit and sound rules enforced by QA. |
| `CONTRIBUTING.md` | The gate, the mistake-ledger workflow, how to add a component. |
| `crates/motion-core/` | Pure foundation: `MotionProject` scene graph, `p5.brush` natural media & multi-chamber Golden Spiral geometry (`brush.rs`), `evaluate_frame` timeline, springs/easings, audio planning, `slop_qa` (40-pt critique rubric), and v1 compiler. |
| `crates/motion-program/` | v2.1 compiler: `MotionProgram` schema (`program.rs`), components (`kinetic.rs`, `sketch.rs`, `bento.rs`, `photo.rs`, `chart.rs`, `diagram.rs`, `device.rs`, `flow.rs`, `code.rs`, `lockup.rs`, `field.rs`), Golden Ratio ($\varphi = 1.618034$) design tokens (`taste.rs`), OKLCH palette resolver (`palette.rs`), camera (`camera.rs`), Mistake Ledger (`ledger.rs`), rendered pixel QA (`qa/design.rs`), and auto-repair (`repair.rs`). |
| `crates/motion-bridge/` | Brief/Intent → `MotionProgram`: `parse_brief` (`lib.rs`), bar planner on music beat grid (`plan.rs`), and typed UI flow extraction (`flow.rs`). |
| `crates/motion-render/` | Pixel engine: `CpuRenderer` (`tiny-skia` + `cosmic-text` + `p5.brush` stroke & watercolor rasterization), optional `wgpu` post-FX, image treatments, foreground matting (`matte.rs`), and FFmpeg video/audio export. |
| `crates/motion-voice/` | Voice synthesis (OpenRouter Fish / macOS `say`) and local Whisper + CTC word-level forced alignment. |
| `crates/motion-providers/` | External asset MCP/REST client with strict spend policy (`providers.policy.json`), caching, and audit logs. |
| `crates/motion-mcp/` | Stdio MCP server (`--profile v2|weak|creator|operator`). |
| `crates/motion-cli/` | `motion-engine` CLI binary (`make`, `program`, `serve`, `compile`, `render`, `critique`, `edit`, `reel`, etc.). |
| `assets/qa/mistakes.json` | The Mistake Ledger: every visual/structural defect and its fixture path under `assets/qa/fixtures/`. |
| `examples/v2/*.program.json` | Canonical v2.1 sample programs (`16:9` and `9:16` kinetic, natural media `sketch`, golden-ratio `bento`, `photo_stack`, charts, UI flows, and code). |

---

## 3. Key Domain Entities

1. **`MotionProgram` (`crates/motion-program/src/program.rs`)**
   - Top-level v2.1 authoring contract (`schema/motion_program.json`).
   - Contains `meta` (`name`, `duration`, `fps`, `aspect`), `recipe` or `palette`/`fonts`, `music` (`track`, `start`), `components` (`Vec<Component>`), `camera` (`Vec<CameraEvent>`), `transitions`, and `sound`.
2. **`ComponentBody` (`crates/motion-program/src/program.rs`)**
   - Closed component union: `Backdrop`, `Field`, `Title`, `Caption`, `KineticText`, `Callout`, `Shape`, `Badge`, `Chart`, `Diagram`, `Lockup` (`SplineFrame`, `MonolithSplit`, `OrbitalSeal`, `TerminalPrompt`), `CodeBlock`, `DeviceFrame`, `PhotoStack`, `Bento` (`Grid2x2`, `HeroLeft`, `Pillars`, `GoldenSpiral`), `Sketch` (`Freeform`, `GoldenSpiral`, `FlowField`, `ArchitecturalHatch`), `Hud`.
3. **`UiFlow` & `DeviceFrame` (`crates/motion-program/src/flow.rs`, `device.rs`)**
   - Typed product UI flows (`Payment`, `Signup`, `Upload`, `Share`, `ToggleSetting`) laid out on a real iOS/desktop point grid (`393×852 pt`) with continuous-corner squircle morphs, circular tap reveals, hold rings, and island toasts.
4. **`ReelPlan` & `Bar` (`crates/motion-bridge/src/plan.rs`)**
   - Music-locked bar plan (`Hook`, `Words`, `Morph`, `Chart`, `Diagram`, `Flow`, `Peak`) aligned to analyzed beat and downbeat timestamps (`Clock`).
5. **`MotionProject` & `ResolvedFrame` (`crates/motion-core/src/scene.rs`, `timeline.rs`)**
   - Low-level deterministic scene graph and per-frame evaluated transform/opacity/layer tree consumed by `motion-render`.
6. **`GateFail` & `Critique` (`crates/motion-program/src/ledger.rs`, `crates/motion-core/src/slop_qa.rs`)**
   - Structured QA defects from the Mistake Ledger and 8-axis / 40-point anti-slop critique scores.

---

## 4. Active Guardrails

- **Determinism First:** Same JSON + seed must produce byte-identical `MotionProject` output and deterministic frames. Never introduce wall-clock timestamps or unseeded RNG into compilation or rendering.
- **Fix the Engine, Never Hide Bugs in Samples:** Every visual defect is governed by `assets/qa/mistakes.json` (`ledger_fixtures_fail_then_pass` and `rendered_fixtures_fail_then_pass`). Improve lowering (`motion-program`), planning (`motion-bridge`), or auto-repair (`repair.rs`) rather than hacking individual sample files to mask engine issues.
- **Disk Hygiene:** Never leave rendered `frame_*.png` or temporary contact sheets (`sheet.png`, `*.beats.png`) on disk after inspection. Delete temporary PNGs immediately after viewing.
- **Brand Neutrality:** Samples use invented brands; never present a real company as the subject of an ad.
- **Quality Gate:** All changes must pass `cargo fmt --check`, `cargo clippy --all-targets`, `cargo nextest run --workspace` (or `cargo test --all`), and `scripts/cli_smoke.sh`.
