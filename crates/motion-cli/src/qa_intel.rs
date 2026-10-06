//! (0.23 W5a) `mix_intelligibility`: can a listener still make out the words
//! on the final mix? The mix's audio (the MP4 given to `qa --speech --mixed`)
//! goes through the same local recogniser `voice --align local` uses (whisper
//! on-device, cached by sha256 of the audio like `asr_local`), and the share
//! of the take's script words recognised as written is set against the same
//! share for the clean voice (`SpeechMap.recognised`, else the voice file
//! recognised the same way). "As written" is the whole-take alignment
//! `spoken_mismatch` runs ([`motion_render::speech_qa::script_words_heard`]).
//!
//! The drop, in percentage points of the script's words, is judged by the
//! frozen thresholds: at most [`MIX_INTELLIGIBILITY_PASS_POINTS`] PASS, more
//! than [`MIX_INTELLIGIBILITY_FAIL_POINTS`] FAIL, WARN between. Builds without
//! `asr-local`, or without the model installed, SKIP with the reason (the
//! recogniser is local: nothing is fetched or called over the network).
//!
//! The mix's audio is read through a pipe and held in memory: no extracted wav
//! is written to disk. The only thing cached is the recogniser's word list.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use motion_core::checks::{
    MIX_INTELLIGIBILITY, MIX_INTELLIGIBILITY_FAIL_POINTS, MIX_INTELLIGIBILITY_PASS_POINTS,
};
use motion_core::speech::{RecognisedWord, SpeechMap};
use motion_render::speech_qa::{script_words_heard, CheckStatus, SpeechQaCheck};
use motion_voice::asr_local::{self, LocalParams, ModelSpec};
use motion_voice::{wav, VoiceCache, VoiceError, SAMPLE_RATE};

/// How many words lost on the mix a detail names.
const LOST_SHOWN: usize = 6;

/// The recogniser the clean voice was heard with (`SpeechMap.alignment`, e.g.
/// `local:whisper-base.en:beam1+ctc`): its model and beam width. Anything else
/// (onset, the paid service, no label) falls back to the default local model.
fn recogniser_of(alignment: Option<&str>) -> (&'static ModelSpec, u32) {
    let default =
        || asr_local::model_spec(asr_local::DEFAULT_MODEL).unwrap_or(&asr_local::MODELS[0]);
    let Some(rest) = alignment.and_then(|a| a.strip_prefix("local:")) else {
        return (default(), 1);
    };
    let mut parts = rest.split(':');
    let spec = parts.next().and_then(asr_local::model_spec);
    let beam = parts
        .next()
        .and_then(|b| b.strip_prefix("beam"))
        .map(|b| {
            b.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|b| b.parse::<u32>().ok())
        .filter(|b| *b >= 1);
    (spec.unwrap_or_else(default), beam.unwrap_or(1))
}

/// The local recogniser for one run.
struct Recogniser {
    spec: &'static ModelSpec,
    params: LocalParams,
    models: PathBuf,
    cache: PathBuf,
}

impl Recogniser {
    /// The recogniser `speech` was timed with, or why this build / machine
    /// cannot run it (the SKIP reason).
    fn open(speech: &SpeechMap) -> Result<Recogniser, String> {
        if !asr_local::AVAILABLE {
            return Err(
                "this build has no local recogniser (build with `--features asr-local` and run \
                 `motion-engine models fetch whisper-base.en`)"
                    .to_string(),
            );
        }
        let (spec, beam) = recogniser_of(speech.alignment.as_deref());
        let models = asr_local::models_root();
        if asr_local::installed(&models, spec).is_none() {
            return Err(format!(
                "{} is not installed (run `motion-engine models fetch {}`)",
                spec.name, spec.name
            ));
        }
        Ok(Recogniser {
            spec,
            // The words do not depend on the CTC refinement (it moves times,
            // never words), so the mix is recognised without it.
            params: LocalParams {
                beam_size: beam,
                ..LocalParams::default()
            },
            models,
            // The cache `voice --align local` fills.
            cache: VoiceCache::with_default_root().root().join("asr"),
        })
    }

    /// Recognised words of a 16-bit mono WAV (cached by its bytes).
    fn words(&self, wav_bytes: &[u8]) -> Result<Vec<RecognisedWord>, VoiceError> {
        let (words, _cached) = asr_local::transcribe(
            wav_bytes,
            self.spec,
            &self.models,
            &self.params,
            &self.cache,
        )?;
        Ok(words
            .into_iter()
            .map(|w| RecognisedWord {
                word: w.word,
                start: w.start,
                end: w.end,
            })
            .collect())
    }
}

/// The first audio stream of `mixed` as a mono 16-bit WAV at the voice-over's
/// own rate, in memory.
fn mix_wav(mixed: &Path) -> Result<Vec<u8>, String> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(mixed)
        .args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar"])
        .arg(SAMPLE_RATE.to_string())
        .args(["-f", "s16le", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("could not run ffmpeg (is it installed?): {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ffmpeg could not read the audio of {}: {}",
            mixed.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let samples: Vec<i16> = out
        .stdout
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    if samples.is_empty() {
        return Err(format!("{} has no audio", mixed.display()));
    }
    Ok(wav::encode(&samples))
}

/// The words of the clean voice: `SpeechMap.recognised` when the voice was
/// heard by the local recogniser (`alignment` `local:...`), else the voice
/// file recognised the same way as the mix.
fn clean_voice_words(
    rec: &Recogniser,
    speech: &SpeechMap,
    speech_dir: &Path,
) -> Result<Vec<RecognisedWord>, String> {
    let local = speech
        .alignment
        .as_deref()
        .is_some_and(|a| a.starts_with("local:"));
    if local && !speech.recognised.is_empty() {
        return Ok(speech.recognised.clone());
    }
    let file = speech_dir.join(&speech.audio);
    let bytes = std::fs::read(&file)
        .map_err(|e| format!("voice file {} not readable: {e}", file.display()))?;
    rec.words(&bytes)
        .map_err(|e| format!("local recogniser on the clean voice: {e}"))
}

/// The share of the script heard on the mix against the clean voice.
#[derive(Debug, Clone, PartialEq)]
pub struct Intelligibility {
    /// Words in the script.
    pub script: usize,
    /// Of those, heard as written on the mix.
    pub mix_heard: usize,
    /// Of those, heard as written on the clean voice.
    pub voice_heard: usize,
    /// Script words the clean voice had and the mix lost, in script order.
    pub lost: Vec<String>,
}

impl Intelligibility {
    /// Percentage of the script heard on the mix.
    pub fn mix_pct(&self) -> f64 {
        pct(self.mix_heard, self.script)
    }

    /// Percentage of the script heard on the clean voice.
    pub fn voice_pct(&self) -> f64 {
        pct(self.voice_heard, self.script)
    }

    /// How many percentage points fewer script words the mix has than the
    /// clean voice (negative: the mix was recognised better).
    pub fn drop_points(&self) -> f64 {
        self.voice_pct() - self.mix_pct()
    }
}

fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        100.0 * part as f64 / whole as f64
    }
}

/// The shares of `speech`'s script heard in `voice` and in `mix`, by the
/// alignment `spoken_mismatch` uses.
pub fn compare(
    speech: &SpeechMap,
    voice: &[RecognisedWord],
    mix: &[RecognisedWord],
) -> Intelligibility {
    let on_voice = script_words_heard(speech, voice);
    let on_mix = script_words_heard(speech, mix);
    let mut lost = Vec::new();
    for ((word, v), (_, m)) in on_voice.iter().zip(&on_mix) {
        if *v && !*m {
            lost.push(word.clone());
        }
    }
    Intelligibility {
        script: on_voice.len(),
        mix_heard: on_mix.iter().filter(|(_, h)| *h).count(),
        voice_heard: on_voice.iter().filter(|(_, h)| *h).count(),
        lost,
    }
}

/// PASS within [`MIX_INTELLIGIBILITY_PASS_POINTS`] points of drop, FAIL beyond
/// [`MIX_INTELLIGIBILITY_FAIL_POINTS`], WARN between.
pub fn drop_status(points: f64) -> CheckStatus {
    // A whole word of a short script is a fraction of a point: tolerate float
    // noise at the thresholds themselves.
    const EPS: f64 = 1e-9;
    if points <= MIX_INTELLIGIBILITY_PASS_POINTS + EPS {
        CheckStatus::Pass
    } else if points <= MIX_INTELLIGIBILITY_FAIL_POINTS + EPS {
        CheckStatus::Warn
    } else {
        CheckStatus::Fail
    }
}

fn check(status: CheckStatus, detail: impl Into<String>) -> SpeechQaCheck {
    SpeechQaCheck {
        name: MIX_INTELLIGIBILITY.to_string(),
        status,
        detail: detail.into(),
    }
}

/// The check of one comparison. (The text says "fail" in lower case: `reel`
/// echoes every QA line holding the upper-case word, and a passing line is
/// not one.)
fn judge(i: &Intelligibility, model: &str) -> SpeechQaCheck {
    if i.script == 0 {
        return check(CheckStatus::Skip, "the speech file has no script words");
    }
    if i.voice_heard == 0 {
        return check(
            CheckStatus::Warn,
            format!(
                "the recogniser ({model}) heard none of the clean voice's {} script word(s): nothing to compare the mix with",
                i.script
            ),
        );
    }
    // `+ 0.0` turns a -0.0 into 0.0 for printing.
    let drop = i.drop_points() + 0.0;
    let mut detail = format!(
        "{}/{} script words heard as written on the mix ({:.1}%), {}/{} on the clean voice ({:.1}%): drop {drop:.1} points (pass within {MIX_INTELLIGIBILITY_PASS_POINTS:.0}, fail beyond {MIX_INTELLIGIBILITY_FAIL_POINTS:.0}); local recogniser {model}",
        i.mix_heard,
        i.script,
        i.mix_pct(),
        i.voice_heard,
        i.script,
        i.voice_pct(),
    );
    if !i.lost.is_empty() {
        let shown: Vec<&str> = i.lost.iter().take(LOST_SHOWN).map(String::as_str).collect();
        detail.push_str(&format!("; lost on the mix: {}", shown.join(", ")));
        if i.lost.len() > LOST_SHOWN {
            detail.push_str(&format!(" (+{} more)", i.lost.len() - LOST_SHOWN));
        }
    }
    check(drop_status(drop), detail)
}

/// `mix_intelligibility` for `qa --speech`: `mixed` is the delivered MP4 (or
/// audio file). SKIP without it, without the `asr-local` feature or without
/// the model; WARN when the recogniser or ffmpeg fails (never a silent skip).
pub fn mix_intelligibility(
    speech: &SpeechMap,
    speech_dir: &Path,
    mixed: Option<&Path>,
) -> SpeechQaCheck {
    let Some(mixed) = mixed else {
        return check(CheckStatus::Skip, "no --mixed file");
    };
    if speech.words.is_empty() {
        return check(CheckStatus::Skip, "no spoken words");
    }
    let rec = match Recogniser::open(speech) {
        Ok(r) => r,
        Err(why) => return check(CheckStatus::Skip, why),
    };
    let wav_bytes = match mix_wav(mixed) {
        Ok(w) => w,
        Err(why) => return check(CheckStatus::Warn, why),
    };
    let on_mix = match rec.words(&wav_bytes) {
        Ok(w) => w,
        Err(e) => {
            return check(
                CheckStatus::Warn,
                format!("local recogniser on the mix: {e}"),
            )
        }
    };
    let on_voice = match clean_voice_words(&rec, speech, speech_dir) {
        Ok(w) => w,
        Err(why) => return check(CheckStatus::Warn, why),
    };
    judge(&compare(speech, &on_voice, &on_mix), rec.spec.name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::speech::{SpeechSentence, SpeechWord, SPEECH_VERSION};

    fn map(text: &str) -> SpeechMap {
        let words: Vec<SpeechWord> = text
            .split_whitespace()
            .enumerate()
            .map(|(i, w)| SpeechWord {
                text: w.to_string(),
                start: i as f64 * 0.4,
                end: i as f64 * 0.4 + 0.3,
                confidence: 1.0,
            })
            .collect();
        let end = words.len() as f64 * 0.4;
        SpeechMap {
            version: SPEECH_VERSION.to_string(),
            audio: "voice.wav".into(),
            sample_rate: 48_000,
            duration: end,
            provider: "fixture".into(),
            model: "fixture".into(),
            voice: "fx".into(),
            words,
            sentences: vec![SpeechSentence {
                beat: 0,
                start: 0.0,
                end,
            }],
            recognised: vec![],
            alignment: None,
        }
    }

    fn heard(text: &str) -> Vec<RecognisedWord> {
        text.split_whitespace()
            .enumerate()
            .map(|(i, w)| RecognisedWord {
                word: w.to_string(),
                start: i as f64 * 0.4,
                end: i as f64 * 0.4 + 0.3,
            })
            .collect()
    }

    #[test]
    fn thresholds_are_the_frozen_ones() {
        assert_eq!(drop_status(-3.0), CheckStatus::Pass);
        assert_eq!(drop_status(0.0), CheckStatus::Pass);
        assert_eq!(drop_status(2.0), CheckStatus::Pass);
        assert_eq!(drop_status(2.01), CheckStatus::Warn);
        assert_eq!(drop_status(5.0), CheckStatus::Warn);
        assert_eq!(drop_status(5.01), CheckStatus::Fail);
        assert_eq!(drop_status(40.0), CheckStatus::Fail);
        // One word of fifty is exactly two points.
        let one_of_fifty = Intelligibility {
            script: 50,
            mix_heard: 49,
            voice_heard: 50,
            lost: vec!["x".into()],
        };
        assert_eq!(drop_status(one_of_fifty.drop_points()), CheckStatus::Pass);
    }

    #[test]
    fn the_recogniser_follows_the_speech_alignment_label() {
        let (spec, beam) = recogniser_of(Some("local:whisper-small.en:beam5+ctc"));
        assert_eq!((spec.name, beam), ("whisper-small.en", 5));
        let (spec, beam) = recogniser_of(Some("local:whisper-base.en:beam1"));
        assert_eq!((spec.name, beam), ("whisper-base.en", 1));
        for other in [None, Some("onset"), Some("asr"), Some("local:nope:beam0")] {
            let (spec, beam) = recogniser_of(other);
            assert_eq!(
                (spec.name, beam),
                (asr_local::DEFAULT_MODEL, 1),
                "{other:?}"
            );
        }
    }

    #[test]
    fn shares_use_the_spoken_mismatch_alignment() {
        let speech = map("Sleep takes eight hours for most adults");
        // The voice has every word (a number said as a numeral counts); the mix
        // lost "hours" and "adults".
        let voice = heard("sleep takes 8 hours for most adults");
        let mix = heard("sleep takes 8 for most");
        let i = compare(&speech, &voice, &mix);
        assert_eq!((i.script, i.voice_heard, i.mix_heard), (7, 7, 5));
        assert_eq!(i.lost, vec!["hours", "adults"]);
        assert!((i.drop_points() - 100.0 * 2.0 / 7.0).abs() < 1e-9);
        let c = judge(&i, "whisper-base.en");
        assert_eq!(c.status, CheckStatus::Fail);
        assert_eq!(c.name, MIX_INTELLIGIBILITY);
        assert!(c.detail.contains("5/7"), "{}", c.detail);
        assert!(c.detail.contains("7/7"), "{}", c.detail);
        assert!(
            c.detail.contains("lost on the mix: hours, adults"),
            "{}",
            c.detail
        );
    }

    #[test]
    fn a_clean_mix_passes_and_the_text_has_no_upper_case_fail() {
        let speech = map("the quick brown fox jumps over the lazy dog");
        let same = heard("the quick brown fox jumps over the lazy dog");
        let c = judge(&compare(&speech, &same, &same), "whisper-base.en");
        assert_eq!(c.status, CheckStatus::Pass);
        assert!(c.detail.contains("drop 0.0 points"), "{}", c.detail);
        assert!(
            !c.detail.contains("FAIL"),
            "reel echoes lines with FAIL: {}",
            c.detail
        );
        // A mix recognised better than the voice is a negative drop: PASS.
        let weak = heard("the quick brown fox");
        let c = judge(&compare(&speech, &weak, &same), "whisper-base.en");
        assert_eq!(c.status, CheckStatus::Pass, "{}", c.detail);
    }

    #[test]
    fn nothing_to_compare_is_a_warning_not_a_pass() {
        let speech = map("the quick brown fox");
        let silent: Vec<RecognisedWord> = Vec::new();
        let c = judge(&compare(&speech, &silent, &silent), "whisper-base.en");
        assert_eq!(c.status, CheckStatus::Warn, "{}", c.detail);
        let empty = SpeechMap {
            words: vec![],
            sentences: vec![],
            ..map("x")
        };
        let c = judge(&compare(&empty, &silent, &silent), "whisper-base.en");
        assert_eq!(c.status, CheckStatus::Skip);
    }

    #[test]
    fn skips_say_why() {
        let speech = map("the quick brown fox");
        let dir = Path::new(".");
        let c = mix_intelligibility(&speech, dir, None);
        assert_eq!(c.status, CheckStatus::Skip);
        assert!(c.detail.contains("--mixed"), "{}", c.detail);
        let c = mix_intelligibility(&speech, dir, Some(Path::new("missing.mp4")));
        if asr_local::AVAILABLE
            && asr_local::installed(&asr_local::models_root(), recogniser_of(None).0).is_some()
        {
            // The recogniser could run: a mix it cannot read is a WARN.
            assert_eq!(c.status, CheckStatus::Warn, "{}", c.detail);
        } else {
            assert_eq!(c.status, CheckStatus::Skip);
            assert!(
                c.detail.contains("asr-local") || c.detail.contains("models fetch"),
                "{}",
                c.detail
            );
        }
    }
}
