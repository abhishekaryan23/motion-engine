//! (0.10 Q) Word timing from speech recognition (OpenRouter
//! `POST /api/v1/audio/transcriptions`, Deepgram Nova-3, ≈ $0.004/min).
//!
//! Onset-estimated word starts were off by 131 ms on average (max 567 ms)
//! against recognised timestamps — visible in karaoke captions. The final
//! voice-over WAV is transcribed once (content-addressed cache keyed by the
//! audio bytes, so reruns make no call and stay byte-identical), recognised
//! words are aligned to the spoken lines in order, and matched words take the
//! recognised times. Unmatched words keep their estimates (never invented).

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::config::OPENROUTER;
use crate::openrouter::HttpPost;
use crate::{SpeechWord, VoiceError};

pub const TRANSCRIBE_URL: &str = "https://openrouter.ai/api/v1/audio/transcriptions";
pub const ASR_MODEL: &str = "deepgram/nova-3";

/// One recognised word.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AsrWord {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

fn multipart(fields: &[(&str, &str)], wav: &[u8], boundary: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(wav.len() + 512);
    for (k, v) in fields {
        out.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    out.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    out.extend_from_slice(wav);
    out.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Cache path for a transcription of `wav` in `language`.
pub fn cache_path(root: &Path, wav: &[u8], language: &str) -> PathBuf {
    let mut h = Sha256::new();
    h.update(ASR_MODEL.as_bytes());
    h.update([0]);
    h.update(language.as_bytes());
    h.update([0]);
    h.update(wav);
    root.join(format!("{}.asr.json", hex(&h.finalize())))
}

/// Recognised words for `wav` (cached). `offline` turns a miss into an error.
pub fn transcribe(
    wav: &[u8],
    language: &str,
    key: Option<&str>,
    transport: &dyn HttpPost,
    cache_root: &Path,
    offline: bool,
) -> Result<(Vec<AsrWord>, bool), VoiceError> {
    let path = cache_path(cache_root, wav, language);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(words) = serde_json::from_str::<Vec<AsrWord>>(&text) {
            return Ok((words, true));
        }
    }
    if offline {
        return Err(VoiceError::OfflineMiss(path.display().to_string()));
    }
    let key = key
        .filter(|k| !k.is_empty())
        .ok_or(VoiceError::MissingKey(OPENROUTER))?;
    let boundary = format!("motionengine{}", &hex(&Sha256::digest(wav))[..16]);
    let body = multipart(
        &[
            ("model", ASR_MODEL),
            ("response_format", "verbose_json"),
            ("timestamp_granularities[]", "word"),
            ("language", language),
        ],
        wav,
        &boundary,
    );
    let headers = vec![
        ("Authorization".to_string(), format!("Bearer {key}")),
        (
            "Content-Type".to_string(),
            format!("multipart/form-data; boundary={boundary}"),
        ),
    ];
    let resp = transport.post(TRANSCRIBE_URL, &headers, &body)?;
    if !(200..300).contains(&resp.status) {
        let message = serde_json::from_slice::<serde_json::Value>(&resp.body)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(|m| m.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| {
                String::from_utf8_lossy(&resp.body)
                    .chars()
                    .take(200)
                    .collect()
            });
        return Err(VoiceError::Http {
            provider: OPENROUTER,
            status: resp.status,
            message: crate::openrouter::scrub_urls(&message),
        });
    }
    let v: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| VoiceError::Audio(format!("transcription response: {e}")))?;
    let words: Vec<AsrWord> = v
        .get("words")
        .and_then(|w| w.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|w| {
                    Some(AsrWord {
                        word: w.get("word")?.as_str()?.to_string(),
                        start: w.get("start")?.as_f64()?,
                        end: w.get("end")?.as_f64()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    std::fs::create_dir_all(cache_root)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(&words).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)?;
    Ok((words, false))
}

fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Value of a one-word number written either way ("8", "eight", "twenty").
fn number_value(w: &str) -> Option<u64> {
    if !w.is_empty() && w.len() <= 18 && w.chars().all(|c| c.is_ascii_digit()) {
        return w.parse().ok();
    }
    const WORDS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    const TENS: [&str; 7] = [
        "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    if let Some(i) = WORDS.iter().position(|&n| n == w) {
        return Some(i as u64);
    }
    TENS.iter()
        .position(|&n| n == w)
        .map(|i| 30 + 10 * i as u64)
}

/// Similarity in [0, 1]: 1 for equal normalised forms or the same one-word
/// number written as digits and as a word ("8" / "eight": recognisers write
/// small numbers either way), else a Levenshtein ratio.
fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if let (Some(x), Some(y)) = (number_value(a), number_value(b)) {
        if x == y {
            return 1.0;
        }
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / a.len().max(b.len()) as f64
}

/// (0.10 Q) Cut times between consecutive spoken lines from recognised words:
/// the midpoint between line k's last word and line k+1's first word. `None`
/// unless every boundary word on both sides was recognised.
pub fn line_cuts(lines: &[Vec<String>], asr: &[AsrWord]) -> Option<Vec<f64>> {
    line_gaps(lines, asr).map(|gaps| gaps.iter().map(|&(a, b)| (a + b) / 2.0).collect())
}

/// (0.20 A5) The gap between consecutive spoken lines from recognised words:
/// `(line k's last word end, line k+1's first word start)` per boundary, with
/// the alignment of [`line_cuts`]. `None` unless every boundary word on both
/// sides was recognised and the two words do not overlap by more than 50 ms
/// (a slight overlap is returned as is: the gap is then shorter than any
/// silence window and the cut falls at its midpoint).
pub fn line_gaps(lines: &[Vec<String>], asr: &[AsrWord]) -> Option<Vec<(f64, f64)>> {
    let flat: Vec<String> = lines.iter().flatten().cloned().collect();
    let pairs = align_pairs(&flat, asr);
    let time_of = |i: usize| pairs.iter().find(|p| p.0 == i).map(|p| &asr[p.1]);
    let mut gaps = Vec::new();
    let mut end = 0usize;
    for k in 0..lines.len().saturating_sub(1) {
        end += lines[k].len();
        if lines[k].is_empty() || lines[k + 1].is_empty() {
            return None;
        }
        let last = time_of(end - 1)?;
        let first = time_of(end)?;
        if first.start < last.end - 0.05 {
            return None;
        }
        gaps.push((last.end, first.start));
    }
    Some(gaps)
}

/// (0.20 A2) Indices of recognised words that align with none of `texts`
/// (the script's words, in order), with the alignment [`apply`] uses: what
/// the narrator said that the script does not contain (inserted or changed
/// words).
pub fn unmatched_recognised(texts: &[String], asr: &[AsrWord]) -> Vec<usize> {
    let pairs = align_pairs(texts, asr);
    let mut used = vec![false; asr.len()];
    for &(_, j) in &pairs {
        used[j] = true;
    }
    (0..asr.len()).filter(|&j| !used[j]).collect()
}

/// Monotone alignment of engine words to recognised words (similarity ≥ 0.6).
fn align_pairs(texts: &[String], asr: &[AsrWord]) -> Vec<(usize, usize)> {
    let (n, m) = (texts.len(), asr.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let a: Vec<String> = texts.iter().map(|w| norm(w)).collect();
    let b: Vec<String> = asr.iter().map(|w| norm(&w.word)).collect();
    let mut score = vec![vec![0.0f64; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            let s = similarity(&a[i - 1], &b[j - 1]);
            let diag = if s >= 0.6 {
                score[i - 1][j - 1] + s
            } else {
                f64::MIN
            };
            score[i][j] = diag.max(score[i - 1][j]).max(score[i][j - 1]);
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let s = similarity(&a[i - 1], &b[j - 1]);
        if s >= 0.6 && (score[i][j] - (score[i - 1][j - 1] + s)).abs() < 1e-9 {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if (score[i][j] - score[i - 1][j]).abs() < 1e-9 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

/// Align recognised words to the engine's words (same order) with a monotone
/// DP; matched pairs (similarity ≥ 0.6) take the recognised times. Returns how
/// many words were retimed. Times stay monotonic; unmatched words keep their
/// estimates, clamped between retimed neighbours.
pub fn apply(words: &mut [SpeechWord], asr: &[AsrWord]) -> usize {
    if words.is_empty() || asr.is_empty() {
        return 0;
    }
    let texts: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
    let pairs = align_pairs(&texts, asr);
    for &(wi, aj) in &pairs {
        words[wi].start = asr[aj].start;
        words[wi].end = asr[aj].end.max(asr[aj].start + 0.06);
        words[wi].confidence = 0.95;
    }
    // Keep unmatched words between their retimed neighbours.
    let mut lo = 0.0f64;
    for w in words.iter_mut() {
        if w.start < lo {
            let d = w.end - w.start;
            w.start = lo;
            w.end = lo + d.max(0.06);
        }
        lo = w.start;
    }
    pairs.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openrouter::HttpResponse;
    use std::cell::RefCell;

    fn w(t: &str, s: f64) -> SpeechWord {
        SpeechWord {
            text: t.into(),
            start: s,
            end: s + 0.2,
            confidence: 0.7,
        }
    }

    #[test]
    fn line_cuts_split_between_lines_even_without_pauses() {
        let lines = vec![
            vec!["It's".to_string(), "midnight".to_string()],
            vec!["you're".to_string(), "staring".to_string()],
        ];
        let asr = vec![
            AsrWord {
                word: "It's".into(),
                start: 0.1,
                end: 0.3,
            },
            AsrWord {
                word: "midnight,".into(),
                start: 0.3,
                end: 0.8,
            },
            AsrWord {
                word: "you're".into(),
                start: 0.82,
                end: 1.0,
            },
            AsrWord {
                word: "staring".into(),
                start: 1.0,
                end: 1.5,
            },
        ];
        let cuts = line_cuts(&lines, &asr).unwrap();
        assert_eq!(cuts.len(), 1);
        assert!((cuts[0] - 0.81).abs() < 1e-9);
        // The gap behind the cut: last word end, first word start.
        assert_eq!(line_gaps(&lines, &asr), Some(vec![(0.8, 0.82)]));
        // A boundary word that was not recognised → unknown.
        let partial = &asr[..2];
        assert!(line_cuts(&lines, partial).is_none());
        assert!(line_gaps(&lines, partial).is_none());
        // Overlapping boundary words (> 50 ms) → unknown; a slight overlap is kept.
        let mut overlap = asr.clone();
        overlap[2].start = 0.7;
        assert!(line_gaps(&lines, &overlap).is_none());
        overlap[2].start = 0.78;
        assert_eq!(line_gaps(&lines, &overlap), Some(vec![(0.8, 0.78)]));
    }

    fn aw(word: &str, start: f64) -> AsrWord {
        AsrWord {
            word: word.into(),
            start,
            end: start + 0.2,
        }
    }

    #[test]
    fn small_numbers_match_as_digits_or_words() {
        let mut words = vec![w("for", 0.1), w("eight", 0.4), w("minutes", 0.8)];
        let asr = vec![aw("for", 0.12), aw("8", 0.35), aw("minutes,", 0.7)];
        assert_eq!(apply(&mut words, &asr), 3);
        assert_eq!(words[1].start, 0.35);
        assert_eq!(similarity("3", "three"), 1.0);
        assert_eq!(similarity("40", "forty"), 1.0);
        assert!(similarity("3", "thirty") < 0.6);
        // Multi-word numbers are not guessed: "1969" stays its own form.
        assert!(similarity("1969", "nineteen") < 0.6);
    }

    #[test]
    fn unmatched_recognised_words_are_what_the_script_lacks() {
        let texts: Vec<String> = ["Only", "23", "percent", "did"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let asr = vec![
            aw("Only", 0.0),
            aw("about", 0.3),
            aw("23", 0.5),
            aw("percent", 0.8),
            aw("did.", 1.1),
            aw("Thanks.", 2.0),
        ];
        assert_eq!(unmatched_recognised(&texts, &asr), vec![1, 5]);
        assert_eq!(unmatched_recognised(&[], &asr).len(), asr.len());
        assert!(unmatched_recognised(&texts, &[]).is_empty());
    }

    #[test]
    fn alignment_retimes_matches_and_keeps_order() {
        let mut words = vec![
            w("We", 0.30),
            w("begin", 0.70),
            w("₹5,200", 1.2),
            w("today", 1.9),
        ];
        let asr = vec![
            AsrWord {
                word: "We".into(),
                start: 0.16,
                end: 0.6,
            },
            AsrWord {
                word: "begin.".into(),
                start: 0.64,
                end: 1.0,
            },
            AsrWord {
                word: "five".into(),
                start: 1.05,
                end: 1.3,
            },
            AsrWord {
                word: "thousand".into(),
                start: 1.3,
                end: 1.6,
            },
            AsrWord {
                word: "Today".into(),
                start: 1.75,
                end: 2.1,
            },
        ];
        assert_eq!(apply(&mut words, &asr), 3);
        assert_eq!(words[0].start, 0.16);
        assert_eq!(words[1].start, 0.64);
        assert_eq!(words[3].start, 1.75);
        assert_eq!(words[2].start, 1.2); // unmatched: estimate kept
        assert!(words.windows(2).all(|p| p[0].start <= p[1].start));
    }

    struct Mock(RefCell<usize>, Vec<u8>);
    impl HttpPost for Mock {
        fn post(
            &self,
            url: &str,
            headers: &[(String, String)],
            body: &[u8],
        ) -> Result<HttpResponse, VoiceError> {
            *self.0.borrow_mut() += 1;
            assert_eq!(url, TRANSCRIBE_URL);
            assert!(headers
                .iter()
                .any(|(k, v)| k == "Content-Type" && v.starts_with("multipart/form-data")));
            assert!(body
                .windows(9)
                .any(|x| x == b"nova-3\r\n-" || x == b"deepgram/"));
            Ok(HttpResponse {
                status: 200,
                body: self.1.clone(),
            })
        }
    }

    #[test]
    fn transcription_is_cached_by_audio() {
        let dir = std::env::temp_dir().join(format!("me_asr_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let json = br#"{"text":"hi","words":[{"word":"hi","start":0.1,"end":0.3}]}"#.to_vec();
        let mock = Mock(RefCell::new(0), json);
        let (w1, hit1) = transcribe(b"RIFFfake", "en", Some("k"), &mock, &dir, false).unwrap();
        let (w2, hit2) = transcribe(b"RIFFfake", "en", Some("k"), &mock, &dir, true).unwrap();
        assert_eq!(*mock.0.borrow(), 1);
        assert!(!hit1 && hit2);
        assert_eq!(w1, w2);
        assert_eq!(w1[0].start, 0.1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
