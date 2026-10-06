# MotionEngine Architecture & System Design

This document details the High-Level Design (HLD) and Low-Level Design (LLD) of the MotionEngine system.

---

## 1. High-Level Design (HLD)

MotionEngine translates high-level semantic intent into fully composed, broadcast-grade editorial motion graphics videos with synchronized narration and sound design.

```mermaid
graph TD
    User[User / AI Author] -->|CreativeIntent v0.2 + StyleProfile| CLI[MotionEngine CLI]
    CLI --> Voice[Voice Synthesis Engine / TTS]
    Voice -->|WAV + SpeechMap| Compiler[Motion Compiler]
    CLI --> Compiler
    Compiler -->|Asset Library & Catalogs| ArtDir[Art Direction & Grammar Engine]
    ArtDir --> Compiler
    Compiler -->|Validate & LifeCycle| Scene[MotionScene v0.2 Graph]
    Scene --> AudioPlan[Sound Design Planner]
    AudioPlan -->|AudioPlan Cues| Renderer[Render & Mix Engine]
    Scene --> Renderer
    Renderer -->|Tiny-Skia CPU Frames| FrameBuffer[PNG Frames]
    FrameBuffer --> FFmpeg[FFmpeg Encoder & Audio Bus]
    Renderer --> FFmpeg
    FFmpeg --> MP4[Final Master MP4 1080x1920 30fps]
```

---

## 2. Low-Level Design (LLD)

```mermaid
classDiagram
    class CreativeIntent {
        +String version
        +String title
        +Format format
        +Vec~Beat~ beats
    }

    class Beat {
        +Purpose purpose
        +String statement
        +Subject primary
        +Option~Subject~ secondary
        +Option~Relationship~ relationship
        +Energy energy
        +Continuity continuity
        +Option~String~ keyword
    }

    class StyleProfile {
        +Tone tone
        +Polarity polarity
        +Temperature temperature
        +Temperament temperament
        +Density density
        +AccentRole accent_role
    }

    class SpeechMap {
        +String version
        +String audio
        +f64 duration
        +Vec~WordTiming~ words
        +Vec~SentenceTiming~ sentences
    }

    class MotionScene {
        +String version
        +Format format
        +f64 duration
        +Vec~Scene~ scenes
        +Vec~SharedElement~ shared
    }

    class AudioPlan {
        +String version
        +Vec~Cue~ cues
        +Option~MusicBed~ music
    }

    CreativeIntent *-- Beat
    MotionScene <.. CreativeIntent : compiled from
    MotionScene <.. StyleProfile : styled by
    MotionScene <.. SpeechMap : aligned with
    AudioPlan <.. MotionScene : planned from
```

---

## 3. Pipeline Stages

1. **Semantic Intent (`CreativeIntent v0.2`)**: The author specifies *what* to communicate, *not* how to move or style.
2. **Taste Director (`StyleProfile`)**: Selects the palette, typography pairing, motion temperament, material, and density.
3. **Voice & Speech Synchronization (`motion-voice`)**: Speaks the statements, extracts word-level timings into `SpeechMap`, and anchors scene life-cycles.
4. **Art Direction & Asset Planner (`motion-core`)**: Identifies human and object entities, matches them against curated photographic and classical libraries, and produces an `AssetPlan`.
5. **Timeline & Frame Evaluation (`motion-render`)**: Resolves frame-by-frame primitives deterministically via `tiny-skia` and `cosmic-text`.
6. **Sound Design & Master Encoding (`motion-render::audio_mix`)**: Mixes foley sound cues and narration at -16.0 LUFS broadcast target with true peak control.
