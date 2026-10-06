//! motion-voice (0.10): voice-over synthesis OUTSIDE the deterministic core.
//!
//! Providers turn one sentence into audio; the content-addressed cache makes
//! every rerun offline and byte-identical; `build_speech` turns an intent's
//! beat statements into one voice-over WAV plus a [`SpeechMap`]. The engine
//! (motion-core) only ever reads the resulting `<name>.speech.json`.
//! API keys come from env `OPENROUTER_API_KEY` or
//! `~/.config/motionengine/providers.toml` (0600) — never from the repo, a
//! scene, a log line or a test fixture. See docs/VOICE.md.
//!
//! FROZEN: `TtsModel`, `VoiceProfile`, `voice_for`, `VoiceProvider`,
//! `Synthesis`, `VoiceError`, the cache key.

pub mod asr;
pub mod asr_ctc;
pub mod asr_local;
pub mod audio;
pub mod cache;
pub mod config;
pub mod fixture;
pub mod openrouter;
pub mod say;
pub mod speech_build;
pub mod timing;
pub mod wav;

pub use cache::VoiceCache;
pub use fixture::FixtureProvider;
pub use motion_core::compiler::typography::Emotion;
pub use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
pub use openrouter::{HttpPost, HttpResponse, OpenRouterTts, UreqTransport};
pub use say::MacSay;
pub use speech_build::{build_speech, pauses_for, BuildStats, Pauses, WordTimer};

/// Every provider's audio is normalised to this before caching (mono s16 WAV).
pub const SAMPLE_RATE: u32 = 48_000;

/// The TTS model an operator picks with `--tts-model`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TtsModel {
    /// OpenRouter `deepgram/flux-tts:free` (36 English voices; no longer
    /// listed as free by OpenRouter, explicit opt-in only).
    Deepgram,
    /// OpenRouter `fish-audio/s2.1-pro-free:free` (free). The `auto` model.
    /// OpenRouter lists no voices for it, but `voice` is passed to Fish as a
    /// voice-library model id; without one Fish picks a different speaker on
    /// every request (measured 82-221 Hz across our renders), so the engine
    /// always pins one (see [`voice_for`]).
    Fish,
    /// macOS `say` (offline fallback).
    Say,
    /// (0.10 Q) OpenRouter `google/gemini-3.8-flash-tts` (paid, raw PCM
    /// 24 kHz). Explicit opt-in only.
    Gemini,
    /// (0.10 Q) OpenRouter `microsoft/mai-voice-2.1` (paid). Explicit opt-in only.
    Mai,
}

impl TtsModel {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "deepgram" | "flux" | "deepgram/flux-tts:free" => Some(Self::Deepgram),
            "fish" | "fish-audio/s2.1-pro-free:free" => Some(Self::Fish),
            "say" | "macos" | "macos/say" => Some(Self::Say),
            "gemini" | "google/gemini-3.8-flash-tts" => Some(Self::Gemini),
            "mai" | "microsoft/mai-voice-2.1" => Some(Self::Mai),
            _ => None,
        }
    }
    /// Stable model id (cache key + SpeechMap.model).
    pub fn model_id(self) -> &'static str {
        match self {
            Self::Deepgram => "deepgram/flux-tts:free",
            Self::Fish => "fish-audio/s2.1-pro-free:free",
            Self::Say => "macos/say",
            Self::Gemini => "google/gemini-3.8-flash-tts",
            Self::Mai => "microsoft/mai-voice-2.1",
        }
    }
    pub fn provider(self) -> &'static str {
        match self {
            Self::Deepgram | Self::Fish | Self::Gemini | Self::Mai => "openrouter",
            Self::Say => "macos_say",
        }
    }
    /// OpenRouter `response_format` (Gemini only returns raw PCM).
    pub fn response_format(self) -> &'static str {
        match self {
            Self::Gemini => "pcm",
            _ => "mp3",
        }
    }
    /// Sample rate of raw PCM responses (`response_format == "pcm"`).
    pub fn pcm_rate(self) -> u32 {
        24_000
    }
    /// Paid OpenRouter models (fall back to free ones on payment errors).
    pub fn is_paid(self) -> bool {
        matches!(self, Self::Gemini | Self::Mai)
    }
}

/// Rule: `--tts-model auto` is the free OpenRouter voice (owner rule: free
/// TTS only, one voice, one take).
pub fn auto_model() -> TtsModel {
    TtsModel::Fish
}

/// Whole-take fallback after an explicitly chosen paid model fails.
pub const FREE_FALLBACKS: [TtsModel; 1] = [TtsModel::Fish];

/// The three narrator characters the engine chooses between (weak models
/// never pick voices; operators override with `--voice`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceProfile {
    /// trust · calm · warmth · handmade — an even, unhurried narrator.
    CalmNarrator,
    /// joy · energy · urgency · playful_retro — bright and quick.
    Bright,
    /// drama · luxury — low and slow.
    LowSlow,
    /// precision — clear and neutral.
    Clear,
}

/// Curated table: emotion → narrator character.
pub fn profile_for(emotion: Emotion) -> VoiceProfile {
    use Emotion as E;
    match emotion {
        E::Trust | E::Calm | E::Warmth | E::Handmade => VoiceProfile::CalmNarrator,
        E::Joy | E::Energy | E::Urgency | E::PlayfulRetro => VoiceProfile::Bright,
        E::Drama | E::Luxury => VoiceProfile::LowSlow,
        E::Precision => VoiceProfile::Clear,
    }
}

/// A concrete voice for one model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceChoice {
    /// Provider voice id; empty = the provider's default voice.
    pub voice: String,
    /// Words per minute for `say` (`-r`); ignored by OpenRouter models.
    pub say_rate: Option<u32>,
}

/// Fish voice-library ids of the pinned male narrators (see [`voice_for`]).
pub const FISH_ETHAN: &str = "536d3a5e000945adb7038665781a4aca";
pub const FISH_SLAX: &str = "c5f56a6cc2ec4fa8920cb4c5889a3fb7";
pub const FISH_CALM_STORYTELLER: &str = "e686ae649ee44f219a108aacba206c1a";

/// Curated table: (profile, model) → voice. Deepgram ids are from the OpenRouter
/// models API `supported_voices`; character choices await an owner listening pass.
pub fn voice_for(profile: VoiceProfile, model: TtsModel) -> VoiceChoice {
    use VoiceProfile as P;
    let (voice, rate) = match (model, profile) {
        // (0.18) Male narrators on every model (owner rule).
        (TtsModel::Deepgram, P::CalmNarrator) => ("flux-miles-en", None),
        (TtsModel::Deepgram, P::Bright) => ("flux-jack-en", None),
        (TtsModel::Deepgram, P::LowSlow) => ("flux-marcus-en", None),
        (TtsModel::Deepgram, P::Clear) => ("flux-colin-en", None),
        // (0.18, owner rule: male narrators only, one per take.) Public Fish
        // voice-library models, generic narrators (no celebrity or character
        // clones), chosen by tags, use and a pitch check on the free model:
        // "Ethan" (a curious explainer, ~107 Hz), "Slax" (clear, precise,
        // ~107 Hz), "calm storyteller male" (deep documentary, ~92 Hz).
        (TtsModel::Fish, P::CalmNarrator | P::Bright) => (FISH_ETHAN, None),
        (TtsModel::Fish, P::Clear) => (FISH_SLAX, None),
        (TtsModel::Fish, P::LowSlow) => (FISH_CALM_STORYTELLER, None),
        (TtsModel::Say, P::CalmNarrator) => ("Reed (English (US))", Some(168)),
        (TtsModel::Say, P::Bright) => ("Eddy (English (US))", Some(192)),
        (TtsModel::Say, P::LowSlow) => ("Daniel", Some(152)),
        (TtsModel::Say, P::Clear) => ("Reed (English (US))", Some(176)),
        // Gemini voice characters (Google's catalogue descriptions).
        (TtsModel::Gemini, P::CalmNarrator) => ("Achird", None), // friendly
        (TtsModel::Gemini, P::Bright) => ("Puck", None),         // upbeat
        (TtsModel::Gemini, P::LowSlow) => ("Orus", None),        // firm
        (TtsModel::Gemini, P::Clear) => ("Charon", None),        // informative
        // MAI-Voice-2.1 en-US.
        (TtsModel::Mai, P::CalmNarrator) => ("en-US-Grant:MAI-Voice-2.1", None),
        (TtsModel::Mai, P::Bright) => ("en-US-Ethan:MAI-Voice-2.1", None),
        (TtsModel::Mai, P::LowSlow) => ("en-US-Grant:MAI-Voice-2.1", None),
        (TtsModel::Mai, P::Clear) => ("en-US-Ethan:MAI-Voice-2.1", None),
    };
    VoiceChoice {
        voice: voice.to_string(),
        say_rate: rate,
    }
}

/// One synthesized sentence, normalised to [`SAMPLE_RATE`] mono s16 WAV.
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesis {
    pub wav: Vec<u8>,
    /// Provider word timestamps relative to the clip start, when the provider
    /// returns them (OpenRouter's speech endpoint does not).
    pub words: Option<Vec<SpeechWord>>,
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error(
        "no API key for {0}: set OPENROUTER_API_KEY or run `motion-engine providers set-key {0}`"
    )]
    MissingKey(&'static str),
    #[error("provider {provider} returned HTTP {status}: {message}")]
    Http {
        provider: &'static str,
        status: u16,
        message: String,
    },
    #[error("network: {0}")]
    Network(String),
    #[error("audio conversion: {0}")]
    Audio(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("cache miss in offline mode for {0}")]
    OfflineMiss(String),
    /// (0.20) A local recogniser model is not installed.
    #[error("model '{0}' is not installed: run `motion-engine models fetch {0}`")]
    ModelMissing(String),
    /// (0.20) A downloaded or installed model does not match its pinned SHA256.
    #[error("checksum mismatch for {file}: expected {expected}, got {actual}")]
    Checksum {
        file: String,
        expected: String,
        actual: String,
    },
    /// (0.20) The binary was built without the feature this needs.
    #[error("{0} needs a build with `--features asr-local,asr-ctc`")]
    Unsupported(&'static str),
}

/// A text-to-speech backend. Implementations never log the key.
pub trait VoiceProvider {
    /// `openrouter`, `macos_say`, `fixture`.
    fn id(&self) -> &'static str;
    fn synthesize(
        &self,
        text: &str,
        voice: &VoiceChoice,
        model: TtsModel,
    ) -> Result<Synthesis, VoiceError>;
}

/// Content-addressed cache key: sha256(model_id ‖ 0x00 ‖ voice ‖ 0x00 ‖ say_rate ‖ 0x00 ‖ text), hex.
pub fn cache_key(model: TtsModel, voice: &VoiceChoice, text: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(model.model_id().as_bytes());
    h.update([0]);
    h.update(voice.voice.as_bytes());
    h.update([0]);
    h.update(voice.say_rate.unwrap_or(0).to_string().as_bytes());
    h.update([0]);
    h.update(text.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}
