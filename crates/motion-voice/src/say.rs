//! macOS `say` provider (offline fallback): `say` → AIFF → ffmpeg → 48 kHz mono WAV.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::audio::{self, AudioInput};
use crate::{Synthesis, TtsModel, VoiceChoice, VoiceError, VoiceProvider};

#[derive(Debug, Default, Clone, Copy)]
pub struct MacSay;

impl MacSay {
    /// Whether the `say` binary exists (it only does on macOS).
    pub fn available() -> bool {
        Command::new("say")
            .arg("-v")
            .arg("?")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn temp_aiff() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "motion-say-{}-{}.aiff",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ))
    }
}

impl VoiceProvider for MacSay {
    fn id(&self) -> &'static str {
        "macos_say"
    }

    fn synthesize(
        &self,
        text: &str,
        voice: &VoiceChoice,
        _model: TtsModel,
    ) -> Result<Synthesis, VoiceError> {
        let tmp = Self::temp_aiff();
        let mut cmd = Command::new("say");
        if !voice.voice.is_empty() {
            cmd.arg("-v").arg(&voice.voice);
        }
        if let Some(rate) = voice.say_rate {
            cmd.arg("-r").arg(rate.to_string());
        }
        cmd.arg("-o").arg(&tmp).arg("--").arg(text);
        let out = cmd
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| VoiceError::Audio(format!("cannot run say: {e}")))?;
        if !out.status.success() {
            let _ = std::fs::remove_file(&tmp);
            let msg = String::from_utf8_lossy(&out.stderr);
            return Err(VoiceError::Audio(format!("say failed: {}", msg.trim())));
        }
        let result = audio::to_wav_48k_mono(AudioInput::Path(&tmp));
        let _ = std::fs::remove_file(&tmp);
        Ok(Synthesis {
            wav: result?,
            words: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav;

    #[test]
    #[cfg(target_os = "macos")]
    fn say_produces_48k_mono_wav() {
        if !MacSay::available() || !audio::ffmpeg_available() {
            eprintln!("skipping: say/ffmpeg unavailable");
            return;
        }
        let voice = VoiceChoice {
            voice: String::new(),
            say_rate: Some(180),
        };
        let s = MacSay
            .synthesize("Hello world.", &voice, TtsModel::Say)
            .unwrap();
        let w = wav::decode(&s.wav).unwrap();
        assert_eq!(w.sample_rate, 48_000);
        assert!(w.duration() > 0.3, "{}", w.duration());
        assert!(w.samples.iter().any(|s| s.abs() > 500));
    }
}
