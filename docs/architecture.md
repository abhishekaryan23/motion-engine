# MotionEngine v2.1 — Visual Architecture (HLD & LLD)

MotionEngine is an AI-native, deterministic Rust engine unifying **Video Editing, Image/Poster Editing, 2D Vector Motion Graphics, `p5.brush` Natural Media & Watercolor, Golden Ratio ($\varphi = 1.618$) Layouts, 2.5D Tactile Composition, and Native Product UI Flows**.

---

## 1. High-Level Design (HLD)

```mermaid
graph TD
    Client["AI Harness / CLI / Local LLM"] -->|MCP or CLI Call| Surface["Interface Layer (motion-cli / motion-mcp)"]
    StudioUI["Minimal Studio Inspector & Brand Kit UI"] -->|Font / Palette / Brand / Copy Tweaks| Surface

    subgraph Bridge & Authoring Layer
        Surface -->|1-Sentence Brief / CreativeIntent| Bridge["motion-bridge (Bar Planner, Music Grid, Recipes)"]
        Surface -->|Direct JSON / Revise / Brand Overrides| Program["motion-program (MotionProgram v2.1 DSL)"]
        Bridge --> Program
    end

    subgraph Self-Correcting QA & Compiler Layer
        Program --> Validate["Schema & Copy Validator (validate.rs / flow.rs)"]
        Validate --> Lower["Deterministic Lowering (lower.rs / sketch.rs / bento.rs / lockup.rs / layout.rs)"]
        Lower --> Ledger["Mistake Ledger & Rendered QA (ledger.rs / qa/design.rs)"]
        Ledger -->|Auto-Repair <= 2 passes| Repair["Program Repair Engine (repair.rs)"]
        Repair --> Lower
    end

    subgraph Core Scene & Natural Media Layer
        Lower --> Brush["p5.brush & Golden Spiral Geometry Engine (motion-core::brush)"]
        Brush --> Scene["MotionProject Scene Graph (motion-core)"]
        Scene --> Critique["Anti-Slop Critique 40-pt Rubric (slop_qa.rs)"]
    end

    subgraph Rendering & Audio Finishing Layer
        Scene --> Timeline["Frame Evaluator (motion-core::timeline)"]
        Timeline --> Render["2D/2.5D & Natural Media Renderer (motion-render: Tiny-Skia + Cosmic-Text + WGPU Post)"]
        Voice["Voice & Local Whisper/CTC ASR (motion-voice)"] --> AudioMix["EBU R128 -16 LUFS 3-Band Ducking Mixer"]
        Render --> Export["FFmpeg MP4 / High-DPI PNG Image Export"]
        AudioMix --> Export
    end
```

---

## 2. Low-Level Design (LLD) — `qa_loop` & Lowering Pipeline

```mermaid
sequenceDiagram
    participant User as AI Agent / User
    participant Bridge as motion-bridge
    participant Prog as motion-program
    participant Core as motion-core
    participant Render as motion-render

    User->>Bridge: make --brief "..." (or program render)
    Bridge->>Bridge: parse_brief() -> Content + FlowSpec + Stat Claims
    Bridge->>Bridge: plan() bars onto MusicPlan beat/downbeat grid
    Bridge->>Prog: realize() -> MotionProgram v2.1 JSON
    loop qa_loop (up to 2 repair passes)
        Prog->>Prog: resolve_beats() + validate()
        Prog->>Core: lower_with(design_passes=true, follow_content=true)
        Prog->>Prog: static_gates() (mistake-ledger checks)
        Prog->>Render: sample frames at >= 2 fps (composite, behind-type, bare)
        Render-->>Prog: Pixels RGB buffers
        Prog->>Prog: qa::design::run() + beat_frames() + fill()
        alt Gate Failures Found
            Prog->>Prog: repair::repair(&mut program, &fails)
        else All Gates Pass
            Prog-->>Render: Stream full 30/60fps frames to FFmpeg + Audio Mix
        end
    end
    Render->>Core: slop_qa::critique() (/40 score + SoulMetrics)
    Core-->>User: Single JSON summary (video, program, qa: "pass", critique >= 36/40)
```

---

## 3. Strict Layering & Dependency Graph

Dependencies flow strictly top-down with zero circular imports:

1. **Interface Layer (`motion-cli`, `motion-mcp`)** → orchestrates CLI commands (`serve`, `make`, `program`), stdio MCP tools, and the `qa_loop`.
2. **Bridge Layer (`motion-bridge`)** → translates natural-language briefs and v1 `CreativeIntent` into beat-locked `MotionProgram` v2 specifications.
3. **Program Compiler & QA Layer (`motion-program`)** → owns the v2.1 component vocabulary (`kinetic_text`, `sketch`, `bento`, `photo_stack`, `chart`, `diagram`, `shape`, `field`, `lockup`, `code_block`, `device_frame`), Golden Ratio ($\varphi = 1.618034$) modular scale (`taste.rs`), OKLCH color resolver (`palette.rs`), layout passes (`layout.rs`), continuous camera (`camera.rs`), Mistake Ledger (`ledger.rs`), and deterministic `repair.rs`.
4. **Voice & Provider Subsystem Layer (`motion-voice`, `motion-providers`)** → isolated domain crates for TTS/ASR word timing and external asset generation with strict spend caps.
5. **Rendering Layer (`motion-render`)** → stateless frame rasterizer (`CpuRenderer` via `tiny-skia`, `p5.brush` ribbon & opacity-bucketed stamp batching, wet watercolor bleed layers, `cosmic-text` glyph outlines, optional `wgpu` post-processing, Apple Vision `matte`, and FFmpeg streaming).
6. **Foundation Layer (`motion-core`)** → pure data contracts (`MotionProject`, `Scene`, `Layer`, `MotionOp`, `CameraOp`), `p5.brush` natural media & multi-chamber Golden Spiral geometry (`brush.rs`), spring/bezier math, frame evaluation (`evaluate_frame`), audio planning, and `slop_qa` critique. Contains zero pixel rendering or I/O network logic.
