//! OpenRouter TTS (`POST /api/v1/audio/speech`). The only network code in the
//! engine. The key is never logged, printed or `Debug`-formatted in full.

use std::time::Duration;

use crate::audio::{self, AudioInput};
use crate::config::{mask, OPENROUTER};
use crate::{Synthesis, TtsModel, VoiceChoice, VoiceError, VoiceProvider};

pub const SPEECH_URL: &str = "https://openrouter.ai/api/v1/audio/speech";

/// A completed HTTP response (any status).
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Minimal POST transport; tests substitute a mock. Implementations must not
/// log header values.
pub trait HttpPost {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<HttpResponse, VoiceError>;
}

/// Real transport over `ureq` (rustls).
#[derive(Debug, Default, Clone, Copy)]
pub struct UreqTransport;

impl HttpPost for UreqTransport {
    fn post(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Result<HttpResponse, VoiceError> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(20))
            .timeout(Duration::from_secs(180))
            .build();
        let mut req = agent.post(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match req.send_bytes(body) {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => {
                // Transport errors can embed the URL but never header values.
                return Err(VoiceError::Network(t.to_string()));
            }
        };
        let status = resp.status();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)?;
        Ok(HttpResponse { status, body: buf })
    }
}

pub struct OpenRouterTts {
    key: Option<String>,
    transport: Box<dyn HttpPost>,
}

impl std::fmt::Debug for OpenRouterTts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterTts")
            .field("key", &self.key.as_deref().map(mask))
            .finish_non_exhaustive()
    }
}

impl OpenRouterTts {
    /// `key = None` makes `synthesize` return [`VoiceError::MissingKey`].
    pub fn new(key: Option<String>, transport: Box<dyn HttpPost>) -> Self {
        Self { key, transport }
    }

    /// Real transport.
    pub fn with_key(key: Option<String>) -> Self {
        Self::new(key, Box::new(UreqTransport))
    }

    /// The JSON request body (voice omitted when empty).
    pub fn request_body(model: TtsModel, text: &str, voice: &VoiceChoice) -> Vec<u8> {
        let mut v = serde_json::json!({
            "model": model.model_id(),
            "input": text,
            "response_format": model.response_format(),
        });
        if !voice.voice.is_empty() {
            v["voice"] = serde_json::Value::String(voice.voice.clone());
        }
        v.to_string().into_bytes()
    }
}

/// Provider messages may embed account/key management links; never echo
/// URLs into logs (0.10 Q).
pub fn scrub_urls(message: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for word in message.split_whitespace() {
        if word.starts_with("http://") || word.starts_with("https://") {
            out.push("[link removed]");
        } else {
            out.push(word);
        }
    }
    out.join(" ")
}

/// Parse `{"error":{"message":...}}`, else a short body excerpt.
fn error_message(body: &[u8]) -> String {
    scrub_urls(&error_message_raw(body))
}

fn error_message_raw(body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(m) = v.pointer("/error/message").and_then(|m| m.as_str()) {
            return m.to_string();
        }
    }
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        "(empty response body)".to_string()
    } else {
        text.chars().take(200).collect()
    }
}

impl VoiceProvider for OpenRouterTts {
    fn id(&self) -> &'static str {
        OPENROUTER
    }

    fn synthesize(
        &self,
        text: &str,
        voice: &VoiceChoice,
        model: TtsModel,
    ) -> Result<Synthesis, VoiceError> {
        let key = self
            .key
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or(VoiceError::MissingKey(OPENROUTER))?;
        let headers = vec![
            ("Authorization".to_string(), format!("Bearer {key}")),
            ("Content-Type".to_string(), "application/json".to_string()),
            ("Accept".to_string(), "audio/*".to_string()),
        ];
        let body = Self::request_body(model, text, voice);
        let resp = self.transport.post(SPEECH_URL, &headers, &body)?;
        if !(200..300).contains(&resp.status) {
            return Err(VoiceError::Http {
                provider: OPENROUTER,
                status: resp.status,
                message: error_message(&resp.body),
            });
        }
        if resp.body.is_empty() {
            return Err(VoiceError::Audio("empty audio response".into()));
        }
        let wav = if model.response_format() == "pcm" {
            audio::to_wav_48k_mono(AudioInput::RawPcm {
                bytes: &resp.body,
                rate: model.pcm_rate(),
            })?
        } else {
            audio::to_wav_48k_mono(AudioInput::Bytes(&resp.body))?
        };
        Ok(Synthesis { wav, words: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wav;
    use std::cell::RefCell;

    type Seen = (String, Vec<(String, String)>, Vec<u8>);

    struct Mock {
        status: u16,
        body: Vec<u8>,
        seen: RefCell<Vec<Seen>>,
    }

    impl HttpPost for Mock {
        fn post(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
        ) -> Result<HttpResponse, VoiceError> {
            self.seen
                .borrow_mut()
                .push((url.to_string(), headers.to_vec(), body.to_vec()));
            Ok(HttpResponse {
                status: self.status,
                body: self.body.clone(),
            })
        }
    }

    /// Share the mock between the provider and the assertions.
    struct Shared(std::rc::Rc<Mock>);
    impl HttpPost for Shared {
        fn post(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
        ) -> Result<HttpResponse, VoiceError> {
            self.0.post(url, headers, body)
        }
    }

    fn mp3_fixture() -> Option<Vec<u8>> {
        if !audio::ffmpeg_available() {
            return None;
        }
        let out = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=220:duration=0.5",
                "-f",
                "mp3",
                "pipe:1",
            ])
            .output()
            .ok()?;
        out.status.success().then_some(out.stdout)
    }

    fn voice() -> VoiceChoice {
        VoiceChoice {
            voice: "flux-hannah-en".into(),
            say_rate: None,
        }
    }

    #[test]
    fn success_posts_expected_request_and_returns_48k_wav() {
        let Some(mp3) = mp3_fixture() else {
            eprintln!("skipping: ffmpeg unavailable");
            return;
        };
        let mock = std::rc::Rc::new(Mock {
            status: 200,
            body: mp3,
            seen: RefCell::new(Vec::new()),
        });
        let p = OpenRouterTts::new(
            Some("sk-or-v1-testtesttesttest1234".into()),
            Box::new(Shared(mock.clone())),
        );
        let s = p
            .synthesize("Hello there.", &voice(), TtsModel::Deepgram)
            .unwrap();
        assert!(s.words.is_none());
        let w = wav::decode(&s.wav).unwrap();
        assert_eq!(w.sample_rate, 48_000);
        assert!(w.duration() > 0.3 && w.duration() < 0.8, "{}", w.duration());

        let seen = mock.seen.borrow();
        assert_eq!(seen.len(), 1);
        let (url, headers, body) = &seen[0];
        assert_eq!(url, SPEECH_URL);
        assert!(headers
            .iter()
            .any(|(k, v)| k == "Authorization" && v == "Bearer sk-or-v1-testtesttesttest1234"));
        let j: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert_eq!(j["model"], "deepgram/flux-tts:free");
        assert_eq!(j["input"], "Hello there.");
        assert_eq!(j["voice"], "flux-hannah-en");
        assert_eq!(j["response_format"], "mp3");
    }

    #[test]
    fn fish_gets_a_pinned_male_voice() {
        for profile in [
            crate::VoiceProfile::CalmNarrator,
            crate::VoiceProfile::Bright,
            crate::VoiceProfile::LowSlow,
            crate::VoiceProfile::Clear,
        ] {
            let choice = crate::voice_for(profile, TtsModel::Fish);
            assert_eq!(choice.voice.len(), 32, "{profile:?}");
            let b = OpenRouterTts::request_body(TtsModel::Fish, "Hi.", &choice);
            let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
            assert_eq!(j["voice"], choice.voice.as_str());
        }
    }

    #[test]
    fn empty_voice_is_omitted() {
        let b = OpenRouterTts::request_body(
            TtsModel::Fish,
            "Hi.",
            &VoiceChoice {
                voice: String::new(),
                say_rate: None,
            },
        );
        let j: serde_json::Value = serde_json::from_slice(&b).unwrap();
        assert!(j.get("voice").is_none());
        assert_eq!(j["model"], "fish-audio/s2.1-pro-free:free");
    }

    #[test]
    fn error_json_becomes_http_error_without_key() {
        let mock = Mock {
            status: 402,
            body: br#"{"error":{"message":"Insufficient credits","code":402}}"#.to_vec(),
            seen: RefCell::new(Vec::new()),
        };
        let key = "sk-or-v1-secretsecretsecret9999";
        let p = OpenRouterTts::new(Some(key.into()), Box::new(mock));
        let err = p
            .synthesize("Hello.", &voice(), TtsModel::Deepgram)
            .unwrap_err();
        match &err {
            VoiceError::Http {
                provider,
                status,
                message,
            } => {
                assert_eq!(*provider, "openrouter");
                assert_eq!(*status, 402);
                assert_eq!(message, "Insufficient credits");
            }
            other => panic!("unexpected {other:?}"),
        }
        assert!(!err.to_string().contains(key));
    }

    #[test]
    fn non_json_error_body_is_excerpted() {
        let mock = Mock {
            status: 502,
            body: b"Bad gateway".to_vec(),
            seen: RefCell::new(Vec::new()),
        };
        let p = OpenRouterTts::new(Some("sk-or-v1-aaaaaaaaaaaa".into()), Box::new(mock));
        match p.synthesize("x", &voice(), TtsModel::Deepgram) {
            Err(VoiceError::Http {
                status, message, ..
            }) => {
                assert_eq!(status, 502);
                assert_eq!(message, "Bad gateway");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn missing_key_makes_no_request() {
        let mock = std::rc::Rc::new(Mock {
            status: 200,
            body: vec![1],
            seen: RefCell::new(Vec::new()),
        });
        let p = OpenRouterTts::new(None, Box::new(Shared(mock.clone())));
        assert!(matches!(
            p.synthesize("x", &voice(), TtsModel::Deepgram),
            Err(VoiceError::MissingKey("openrouter"))
        ));
        assert!(mock.seen.borrow().is_empty());
    }

    #[test]
    fn debug_masks_key() {
        let key = "sk-or-v1-abcdefghijklmnop1234";
        let p = OpenRouterTts::with_key(Some(key.into()));
        let d = format!("{p:?}");
        assert!(d.contains("sk-or-…1234"), "{d}");
        assert!(!d.contains("abcdefgh"), "{d}");
    }
}
