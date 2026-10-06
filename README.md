<div align="center">

# 🎬 MotionEngine

**Autonomous Editorial Motion Graphics & Video Generation Engine in Rust**

[![License](https://img.shields.io/badge/license-Limited%20(Evaluation%20Only)-red.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey.svg)]()

*Transform declarative semantic intent into broadcast-grade editorial motion graphics with synchronized narration, procedural typography, and reactive sound design.*

<br/>

<p align="center">
  <img src="showcase/documentary_money_preview.gif" width="23%" alt="Documentary Money Preview" />
  <img src="showcase/countdown_tense_preview.gif" width="23%" alt="Countdown Tense Preview" />
  <img src="showcase/layers_documentary_preview.gif" width="23%" alt="Layers Breakdown Preview" />
  <img src="showcase/short_state_change_preview.gif" width="23%" alt="State Change Preview" />
</p>
<p align="center">
  <i>Live MotionEngine renders: Documentary Dossier · Kinetic Countdown · Layers Hierarchy · State Change</i>
</p>

</div>

---

## 🎥 Showcase Video Previews

MotionEngine compiles declarative intent directly into broadcast-grade 1080×1920 (or 720×1280) H.264 + AAC MP4 videos. Below are rendered showcase cuts included directly in the repository:

### 1. Vox-Style Financial Documentary (`documentary_money_720p.mp4`)
* **Genre / Style**: Documentary / Dossier Look
* **Key Features**: Evidence desk styling, paper clippings, stamps, figure cards, and word-synchronized voiceover.
* **Full Master MP4**: [▶️ Download / Watch `documentary_money_720p.mp4`](showcase/documentary_money_720p.mp4)

<p align="center">
  <a href="showcase/documentary_money_720p.mp4">
    <img src="showcase/documentary_money_preview.gif" width="300px" alt="Financial Documentary Preview" />
  </a>
</p>

---

### 2. Kinetic Rocket Countdown (`countdown_tense_720p.mp4`)
* **Genre / Style**: Kinetic Slam / Tense Mood
* **Key Features**: High-velocity countdown beats, audio-reactive hits, and kinetic display typography.
* **Full Master MP4**: [▶️ Download / Watch `countdown_tense_720p.mp4`](showcase/countdown_tense_720p.mp4)

<p align="center">
  <a href="showcase/countdown_tense_720p.mp4">
    <img src="showcase/countdown_tense_preview.gif" width="300px" alt="Rocket Countdown Preview" />
  </a>
</p>

---

### 3. Layers Hierarchy Breakdown (`layers_documentary_720p.mp4`)
* **Genre / Style**: Layers Grammar / Documentary
* **Key Features**: Multi-tier vertical conceptual breakdown with persistent depth focus and level boundaries.
* **Full Master MP4**: [▶️ Download / Watch `layers_documentary_720p.mp4`](showcase/layers_documentary_720p.mp4)

<p align="center">
  <a href="showcase/layers_documentary_720p.mp4">
    <img src="showcase/layers_documentary_preview.gif" width="300px" alt="Layers Breakdown Preview" />
  </a>
</p>

---

### 4. High-Speed State Change (`short_state_change_720p.mp4`)
* **Genre / Style**: State Change Grammar
* **Key Features**: Directional velocity transitions and before/after comparative entity states.
* **Full Master MP4**: [▶️ Download / Watch `short_state_change_720p.mp4`](showcase/short_state_change_720p.mp4)

<p align="center">
  <a href="showcase/short_state_change_720p.mp4">
    <img src="showcase/short_state_change_preview.gif" width="300px" alt="State Change Preview" />
  </a>
</p>

---

## 🏛️ System Architecture (from Archify)

MotionEngine translates high-level semantic intent into fully composed, broadcast-grade editorial motion graphics videos with synchronized narration and sound design.

### Archify System Architecture Diagram

```mermaid
graph TD
    Author["Author / LLM Agent<br/><i>(CreativeIntent v0.2 + StyleProfile)</i>"] -->|intent JSON| Reel["motion-engine reel<br/><i>(motion-cli orchestrator)</i>"]
    
    subgraph Voice Pipeline
        Reel -->|1. voice pass| Voice["motion-voice<br/><i>(TTS · ASR · script)</i>"]
        Voice -->|HTTPS| Provider["OpenRouter / Neural TTS<br/><i>(Fish Audio S2.1 Pro / macOS say)</i>"]
        Voice -->|read/write| Cache[("Voice Cache<br/><i>sha256-addressed WAV</i>")]
    end
    
    subgraph Semantic Compilation
        Reel -->|2. compile pass| Compiler["Compiler<br/><i>(motion-core)</i>"]
        Voice -->|SpeechMap & word timings| Compiler
        Assets[("Asset Library<br/><i>fonts · cutouts · loops · music</i>")] -->|catalog resolution| Compiler
        Compiler -->|MotionScene graph| Timeline["Timeline<br/><i>(evaluate_frame)</i>"]
    end
    
    subgraph Rasterization & Audio Mixing
        Timeline -->|ResolvedFrame| Renderer["CPU Renderer<br/><i>(tiny-skia · cosmic-text)</i>"]
        Assets -->|music bed · SFX cues| Mixer["Audio Mixer<br/><i>(FFmpeg · ducking · -16 LUFS)</i>"]
        Renderer -->|encode frames| Video["MP4 Reel<br/><i>(H.264 + AAC 30fps)</i>"]
        Mixer -->|audio stream| Video
    end
    
    Video --> QA["Speech + Layout QA<br/><i>(qa --speech)</i>"]
```

### Interactive Archify Offline Diagrams

The repository includes standalone, interactive HTML architecture diagrams generated with [Archify](https://github.com/tt-a1i/archify) in [`docs/architecture/`](docs/architecture/):

* 📊 **[System Architecture Diagram](docs/architecture/system-architecture.html)** — Full crate layout, network boundary, and inter-stage data contracts.
* 🔄 **[Reel Workflow Pipeline](docs/architecture/reel-workflow.html)** — Step-by-step pipeline execution, take rotations, and fallback chains.
* 🎙️ **[Voice-Over Sequence](docs/architecture/voice-sequence.html)** — Script pass, single take synthesis, and Whisper CTC word-level timing alignment.

---

## ⚡ Workspace Crates

* **`motion-core`**: Core domain logic, schema definitions (`CreativeIntent`, `StyleProfile`, `MotionScene`), semantic layout grammars, typography, and animation easing.
* **`motion-render`**: High-performance CPU rasterizer built on Tiny-Skia, audio bus mixer, and FFmpeg video pipeline.
* **`motion-voice`**: Voice synthesis pipeline, OpenRouter / local TTS integration, and local Whisper ASR word-timing alignment.
* **`motion-cli`**: Command-line interface (`compile`, `render`, `voice`, `reel`, `validate`).
* **`motion-mcp`**: Model Context Protocol (MCP) server enabling AI coding assistants (Claude Desktop, Antigravity, Cursor) to author and render videos autonomously.

---

## 🛠️ Prerequisites

* **Rust**: `1.80` or later ([rustup.rs](https://rustup.rs/))
* **FFmpeg**: `6.0` or later with `libx264` and `aac` enabled:
  ```bash
  # macOS (Homebrew)
  brew install ffmpeg

  # Ubuntu / Debian
  sudo apt-get install ffmpeg
  ```
* **TTS Provider (Optional)**:
  * For neural speech synthesis: an [OpenRouter](https://openrouter.ai/) API key (Fish Audio S2.1 Pro free tier supported).
  * For offline testing on macOS: the system `say` synthesizer is supported without any API key.

---

## 🚀 Installation & Setup

### 1. Build from Source

```bash
git clone https://github.com/abhishekaryan23/motion-engine.git
cd motion-engine

# Build release binaries with local ASR features enabled
cargo build --release --features asr-local,asr-ctc
```

The resulting binary will be at `target/release/motion-engine`. Add it to your `PATH` or create an alias:

```bash
export PATH="$PWD/target/release:$PATH"
```

### 2. Configure Voice Providers (Optional)

If using OpenRouter for neural voice synthesis:

```bash
# Set your API key securely
motion-engine providers set-key openrouter
# Paste your key when prompted
```

Alternatively, set the environment variable:
```bash
export OPENROUTER_API_KEY="sk-or-v1-..."
```

---

## 💻 Quick Start & CLI Usage

### Option A: The All-in-One `reel` Command

The fastest way to generate a complete video from intent to master MP4:

```bash
motion-engine reel examples/public/minimal-emphasize.intent.json \
  --style examples/public/minimal.style.json \
  -o output/minimal.mp4
```

### Option B: The Granular 3-Stage Pipeline

#### Step 1: Synthesize & Align Narration
```bash
motion-engine voice examples/topics/fuel.intent.json \
  -o output/voice/ \
  --tts-model fish \
  --align local \
  --asr-ctc on
```

#### Step 2: Compile Semantic Motion Scene
```bash
motion-engine compile examples/topics/fuel.intent.json \
  --style examples/taste/editorial_warm.style.json \
  --speech output/voice/fuel.speech.json \
  --art dossier \
  -o output/fuel.motion.json
```

#### Step 3: Render Master Video
```bash
motion-engine render output/fuel.motion.json \
  --speech output/voice/fuel.speech.json \
  --out-dir output/render/
```

---

## 📝 Authoring Video Intents (`CreativeIntent` v0.2)

MotionEngine uses declarative JSON specifications. You state **what** the video communicates; the engine decides camera paths, typography sizes, and layout compositions.

### Minimal Example

```json
{
  "version": "0.2",
  "title": "coffee_facts",
  "format": "vertical",
  "beats": [
    {
      "purpose": "emphasize",
      "statement": "Morning ritual",
      "narration": "Over two billion cups of coffee are poured every single morning.",
      "primary": { "kind": "number", "value": "2.25B", "meaning": "cups per day" },
      "secondary": { "kind": "phrase", "value": "Worldwide", "meaning": "scale" },
      "energy": "impact",
      "keyword": "coffee"
    },
    {
      "purpose": "compare",
      "statement": "Nordic consumption",
      "narration": "Finland leads the world at twelve kilograms per person, while the US sits at four.",
      "primary":   { "kind": "object", "asset": "flag_finland", "value": "12 kg", "meaning": "Finland" },
      "secondary": { "kind": "object", "asset": "flag_us",      "value": "4.5 kg", "meaning": "United States" },
      "relationship": "separate",
      "energy": "building",
      "keyword": "highest"
    },
    {
      "purpose": "reveal",
      "statement": "The daily impact",
      "narration": "That cup is not just caffeine — it is a global trade network linking sixty countries.",
      "primary": { "kind": "number", "value": "60+", "meaning": "exporting nations" },
      "energy": "impact",
      "keyword": "global"
    }
  ]
}
```

### Supported Beat Purposes

* **`emphasize`**: Bold hook or core thesis; headline + hero figure card.
* **`compare`**: Direct head-to-head comparison between two stat cards with auto-drawn dividers.
* **`contrast`**: Tension / gap comparison with relationship connectors.
* **`reveal`**: Climactic takeaway; large display figure card with energetic entrance.
* **`explain`**: Definition or calm contextual breakdown.

### Structured Subject Types

* **`number`**: Numeric figures (`"value": "4.1%"`, `"$120"`, `"-15%"`). Renders in massive serif or grotesque display type.
* **`phrase`**: Text claims or labels.
* **`object`**: Pictured subject looked up in the asset library (`assets/library/`). Combined with `value`, becomes a **Stat Card**.
* **`collection`**: Multi-item lists. When carrying numeric values, automatically compiles into an **Animated Ranking Bar Chart**.

---

## 🎨 Art Direction & Style Profiles

Control visual tone via `StyleProfile`:

```json
{
  "tone": "documentary",
  "polarity": "dark",
  "temperature": "warm",
  "temperament": "balanced",
  "density": "balanced",
  "seed": 1
}
```

* **`tone`**: `documentary` (Vox/dossier evidence desk), `cinematic` (3D multiplane depth & bokeh), `studio` (bold studio cutouts & discs), `editorial` (warm magazine typography), `street` (urban poster collage), `playful` (bouncy energetic motion).
* **`polarity`**: `dark` or `light`.
* **`temperature`**: `warm` or `cool`.
* **`temperament`**: `restrained`, `balanced`, or `energetic`.

---

## 🤖 Model Context Protocol (MCP)

MotionEngine includes a first-class MCP server (`motion-mcp`) allowing AI coding agents (Claude Desktop, Antigravity, Cursor) to compose, check, and render videos autonomously.

### Claude Desktop Configuration

Add to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "motion-engine": {
      "command": "/path/to/motion-engine/target/release/motion-mcp",
      "args": ["--root", "/path/to/motion-engine"]
    }
  }
}
```

---

## 🧪 Verification & Smoke Tests

Run the built-in public interface smoke test:

```bash
bash scripts/public_smoke.sh
```

---

## 📄 License

MotionEngine is licensed under the **MotionEngine Limited License**.

* **Permitted**: Personal inspection, non-commercial evaluation, local testing, and academic study.
* **Prohibited**: Commercial production, SaaS deployment, hosted video generation services, or commercial redistribution without an explicit written license.

For full license terms and commercial licensing inquiries, please see [LICENSE](LICENSE).

---

## 🛡️ Security

Please report any security vulnerabilities via [GitHub Private Vulnerability Reporting](https://github.com/abhishekaryan23/motion-engine/security/advisories/new). See [SECURITY.md](SECURITY.md) for details.
