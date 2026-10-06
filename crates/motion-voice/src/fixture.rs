//! Deterministic synthetic "speech" for tests: a voiced burst per word.
//!
//! Layout of one clip: 120 ms silence, then per word a 180 Hz burst (with
//! harmonics and a raised-cosine envelope) lasting `0.09 s × syllables + 0.05 s`,
//! with 70 ms of silence between words. No trailing silence.

use crate::SAMPLE_RATE;
use crate::{wav, SpeechWord, Synthesis, TtsModel, VoiceChoice, VoiceError, VoiceProvider};

pub const LEAD_SILENCE: f64 = 0.120;
pub const WORD_GAP: f64 = 0.070;
/// (0.10 Q) Silence after a word that ends a sentence (single-take scripts).
pub const SENTENCE_GAP: f64 = 0.38;
pub const F0: f64 = 180.0;

#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureProvider {
    /// Return provider word timestamps (`Some`) instead of `None`.
    pub with_words: bool,
}

impl FixtureProvider {
    pub fn new(with_words: bool) -> Self {
        Self { with_words }
    }
}

/// Vowel-group count, minimum one.
pub fn syllables(word: &str) -> usize {
    let mut n = 0;
    let mut prev = false;
    for c in word.chars() {
        let v = matches!(
            c.to_ascii_lowercase(),
            'a' | 'e' | 'i' | 'o' | 'u' | 'y' | '0'..='9'
        );
        if v && !prev {
            n += 1;
        }
        prev = v;
    }
    n.max(1)
}

/// Duration of one word's burst in seconds.
pub fn word_seconds(word: &str) -> f64 {
    0.09 * syllables(word) as f64 + 0.05
}

/// A word as written, with punctuation stripped from both ends.
pub fn strip_punct(word: &str) -> &str {
    word.trim_matches(|c: char| {
        !(c.is_alphanumeric() || matches!(c, '%' | '₹' | '$' | '€' | '£' | '+'))
    })
}

fn samples_of(seconds: f64) -> usize {
    (seconds * SAMPLE_RATE as f64).round() as usize
}

fn burst(n: usize, out: &mut Vec<i16>) {
    let sr = SAMPLE_RATE as f64;
    let amps = [1.0, 0.5, 0.33, 0.25];
    let norm: f64 = amps.iter().sum();
    for i in 0..n {
        let t = i as f64 / sr;
        let env = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * (i as f64 + 0.5) / n as f64).cos();
        let mut s = 0.0;
        for (h, a) in amps.iter().enumerate() {
            s += a * (2.0 * std::f64::consts::PI * F0 * (h + 1) as f64 * t).sin();
        }
        out.push((s / norm * env * 0.6 * 32767.0).round() as i16);
    }
}

impl FixtureProvider {
    /// Samples plus per-word (text, start, end) in seconds.
    pub fn render(&self, text: &str) -> (Vec<i16>, Vec<SpeechWord>) {
        let mut pcm = vec![0i16; samples_of(LEAD_SILENCE)];
        let mut words = Vec::new();
        let mut prev_end_of_sentence = false;
        for (i, raw) in text.split_whitespace().enumerate() {
            if i > 0 {
                // (0.10 Q) A sentence end gets a breath, like a real narrator.
                let gap = if prev_end_of_sentence {
                    SENTENCE_GAP
                } else {
                    WORD_GAP
                };
                pcm.resize(pcm.len() + samples_of(gap), 0);
            }
            prev_end_of_sentence = raw.ends_with(['.', '?', '!']);
            let start = pcm.len();
            burst(samples_of(word_seconds(raw)), &mut pcm);
            let clean = strip_punct(raw);
            if !clean.is_empty() {
                words.push(SpeechWord {
                    text: clean.to_string(),
                    start: start as f64 / SAMPLE_RATE as f64,
                    end: pcm.len() as f64 / SAMPLE_RATE as f64,
                    confidence: 1.0,
                });
            }
        }
        (pcm, words)
    }
}

impl VoiceProvider for FixtureProvider {
    fn id(&self) -> &'static str {
        "fixture"
    }

    fn synthesize(
        &self,
        text: &str,
        _voice: &VoiceChoice,
        _model: TtsModel,
    ) -> Result<Synthesis, VoiceError> {
        let (pcm, words) = self.render(text);
        Ok(Synthesis {
            wav: wav::encode(&pcm),
            words: self.with_words.then_some(words),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice() -> VoiceChoice {
        VoiceChoice {
            voice: String::new(),
            say_rate: None,
        }
    }

    #[test]
    fn deterministic_and_words_follow_layout() {
        let p = FixtureProvider::new(true);
        let a = p
            .synthesize("Hello big world.", &voice(), TtsModel::Say)
            .unwrap();
        let b = p
            .synthesize("Hello big world.", &voice(), TtsModel::Say)
            .unwrap();
        assert_eq!(a, b);
        let words = a.words.unwrap();
        assert_eq!(words.len(), 3);
        assert_eq!(words[2].text, "world");
        assert!((words[0].start - LEAD_SILENCE).abs() < 1e-3);
        // "Hello" = 2 syllables → 0.23 s.
        assert!((words[0].end - words[0].start - 0.23).abs() < 1e-3);
        assert!((words[1].start - words[0].end - WORD_GAP).abs() < 1e-3);
    }

    #[test]
    fn none_mode_has_no_words() {
        let s = FixtureProvider::new(false)
            .synthesize("a b", &voice(), TtsModel::Say)
            .unwrap();
        assert!(s.words.is_none());
        assert!(wav::decode(&s.wav).unwrap().samples.iter().any(|v| *v != 0));
    }
}
