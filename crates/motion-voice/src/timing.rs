//! Word timing without provider timestamps (`WordTimer`).
//!
//! 1. A syllable-weighted estimate lays the statement's words across the
//!    voiced span of the clip.
//! 2. Each word start after the first snaps to the nearest onset of the 10 ms
//!    RMS envelope within ±35 % of the word's estimated duration.
//! 3. Ends come from the envelope: a word ends where its voiced run ends
//!    before the next word starts (or at the next start when speech runs on).
//!
//! The 10 ms RMS envelope and dB conventions mirror `motion-render/src/sfx.rs`
//! (no import: that crate must not depend on this one). Pure and
//! deterministic: no randomness, no clocks.

use motion_core::speech::{SpeechWord, MIN_WORD_SECONDS};

/// Envelope hop = window (seconds).
pub const HOP_SECONDS: f64 = 0.010;
/// An onset is a rise of at least this many dB within [`RISE_SECONDS`] from a local minimum.
pub const RISE_DB: f64 = 6.0;
pub const RISE_SECONDS: f64 = 0.030;
/// A word start may snap this far (fraction of the word's estimated duration).
pub const SNAP_FRACTION: f64 = 0.35;
/// Snaps closer than this keep confidence [`CONF_SNAPPED`].
pub const CLOSE_SNAP_SECONDS: f64 = 0.040;
pub const CONF_SNAPPED: f32 = 0.9;
pub const CONF_FAR_SNAP: f32 = 0.7;
pub const CONF_ESTIMATE: f32 = 0.5;
/// Frames this far below the loudest frame (dB) count as silence.
const VOICED_BELOW_PEAK_DB: f64 = 40.0;
/// Every word costs this many syllables of extra time (consonants, breath).
const WORD_OVERHEAD_SYLLABLES: f64 = 0.6;
const FLOOR_DB: f64 = -100.0;

fn to_db(x: f64) -> f64 {
    if x <= 1e-5 {
        FLOOR_DB
    } else {
        (20.0 * x.log10()).max(FLOOR_DB)
    }
}

/// 10 ms RMS envelope in dBFS (non-overlapping frames; a short tail frame is kept).
pub fn envelope_db(samples: &[i16], sample_rate: u32) -> Vec<f64> {
    let hop = ((sample_rate as f64 * HOP_SECONDS).round() as usize).max(1);
    samples
        .chunks(hop)
        .map(|c| {
            let e: f64 = c.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
            to_db((e / c.len() as f64).sqrt() / 32768.0)
        })
        .collect()
}

/// Onset times (seconds): the end of a local-minimum frame from which the
/// envelope rises by [`RISE_DB`] within [`RISE_SECONDS`]. A flat valley
/// reports its LAST frame, so a silent plateau yields one onset, at the point
/// speech begins.
pub fn onsets(env: &[f64]) -> Vec<f64> {
    let max_gap = (RISE_SECONDS / HOP_SECONDS).round() as usize;
    let mut out = Vec::new();
    if env.is_empty() {
        return out;
    }
    let mut valley = 0usize;
    for k in 1..env.len() {
        if env[k] <= env[valley] {
            valley = k;
        } else if env[k] - env[valley] >= RISE_DB && k - valley <= max_gap {
            out.push((valley + 1) as f64 * HOP_SECONDS);
            // The next onset needs a fresh dip below the level just reached.
            valley = k;
        } else if k - valley > max_gap && env[k] - env[valley] < RISE_DB {
            // A slow drift upward is not an onset; follow it.
            valley = k;
        }
    }
    out
}

/// (0.20) Syllable estimation lives in `motion_core::speech` (speech QA
/// measures the speech rate with it); the voice crate keeps this path.
pub use motion_core::speech::estimate_syllables;

/// The `WordTimer` implementation: `(clip samples after trimming, sample rate,
/// the statement's words)` → words with times in seconds from the clip start.
pub fn time_words(samples: &[i16], sample_rate: u32, words: &[String]) -> Vec<SpeechWord> {
    if words.is_empty() {
        return Vec::new();
    }
    let sr = sample_rate.max(1) as f64;
    let clip_len = samples.len() as f64 / sr;
    let env = envelope_db(samples, sample_rate);
    let peak = env.iter().copied().fold(FLOOR_DB, f64::max);
    let voiced_thr = peak - VOICED_BELOW_PEAK_DB;
    let voiced = |k: usize| env.get(k).is_some_and(|&d| d > FLOOR_DB && d >= voiced_thr);

    // Voiced span of the clip.
    let first = (0..env.len()).find(|&k| voiced(k));
    let last = (0..env.len()).rfind(|&k| voiced(k));
    let (span_start, span_end) = match (first, last) {
        (Some(a), Some(b)) => (
            a as f64 * HOP_SECONDS,
            ((b + 1) as f64 * HOP_SECONDS).min(clip_len),
        ),
        _ => (0.0, clip_len),
    };
    let span = (span_end - span_start).max(0.0);

    // Syllable-weighted estimate across the span.
    let weights: Vec<f64> = words
        .iter()
        .map(|w| estimate_syllables(w) as f64 + WORD_OVERHEAD_SYLLABLES)
        .collect();
    let total_w: f64 = weights.iter().sum();
    let mut est_start = Vec::with_capacity(words.len());
    let mut est_len = Vec::with_capacity(words.len());
    let mut acc = 0.0;
    for w in &weights {
        est_start.push(span_start + span * acc / total_w);
        est_len.push(span * w / total_w);
        acc += w;
    }

    // Snap starts (after the first) to onsets, in order.
    let onset_times = onsets(&env);
    let n = words.len();
    let mut starts = Vec::with_capacity(n);
    let mut conf = Vec::with_capacity(n);
    for i in 0..n {
        if i == 0 {
            starts.push(span_start);
            conf.push(CONF_SNAPPED);
            continue;
        }
        let lower = starts[i - 1] + MIN_WORD_SECONDS;
        let window = SNAP_FRACTION * est_len[i];
        let mut best: Option<f64> = None;
        for &t in &onset_times {
            if t < lower || (t - est_start[i]).abs() > window {
                continue;
            }
            // Strict `<` keeps the earlier onset on ties.
            if best.is_none_or(|b| (t - est_start[i]).abs() < (b - est_start[i]).abs() - 1e-12) {
                best = Some(t);
            }
        }
        match best {
            Some(t) => {
                starts.push(t);
                conf.push(if (t - est_start[i]).abs() <= CLOSE_SNAP_SECONDS + 1e-9 {
                    CONF_SNAPPED
                } else {
                    CONF_FAR_SNAP
                });
            }
            None => {
                starts.push(est_start[i].max(lower));
                conf.push(CONF_ESTIMATE);
            }
        }
    }
    // Keep every start inside the clip with room for the words behind it.
    for i in (0..n).rev() {
        let room = if i + 1 < n {
            starts[i + 1] - MIN_WORD_SECONDS
        } else {
            clip_len
        };
        starts[i] = starts[i].min(room).max(0.0);
    }

    // Ends: where the voiced run before the next start finishes.
    (0..n)
        .map(|i| {
            let limit = if i + 1 < n {
                starts[i + 1]
            } else {
                span_end.max(starts[i])
            };
            let a = (starts[i] / HOP_SECONDS).floor() as usize;
            let b = (limit / HOP_SECONDS).ceil() as usize;
            let run_end = (a..b.min(env.len()))
                .rev()
                .find(|&k| voiced(k))
                .map(|k| (k + 1) as f64 * HOP_SECONDS);
            let end = match run_end {
                Some(e) if i + 1 < n => e.min(limit),
                _ => limit,
            };
            let end = end.max((starts[i] + MIN_WORD_SECONDS).min(limit.max(starts[i])));
            SpeechWord {
                text: words[i].clone(),
                start: starts[i],
                end: end.min(clip_len.max(starts[i])),
                confidence: conf[i],
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::FixtureProvider;
    use crate::speech_build::{audible_range, statement_words};
    use crate::SAMPLE_RATE;
    use motion_core::speech::number_syllables;

    #[test]
    fn syllable_estimates() {
        assert_eq!(estimate_syllables("₹5,200"), 6);
        assert_eq!(estimate_syllables("40%"), 4);
        assert_eq!(estimate_syllables("cat"), 1);
        assert_eq!(estimate_syllables("orders"), 2);
        assert_eq!(estimate_syllables("take"), 1);
        assert_eq!(estimate_syllables("table"), 2);
        assert_eq!(estimate_syllables("customer"), 3);
        assert_eq!(estimate_syllables("TV"), 2);
        assert_eq!(estimate_syllables("3.5"), 3);
        assert_eq!(estimate_syllables("..."), 1);
        assert_eq!(number_syllables(1_250_000), 10);
        assert_eq!(number_syllables(0), 2);
    }

    const SENTENCES: [&str; 5] = [
        "Orders ship in three days.",
        "Now they ship today!",
        "That changes everything for every customer we serve.",
        "Small steps compound into remarkable results over time.",
        "Our best quarter yet, thanks to you.",
    ];

    /// `(trimmed clip, truth word starts relative to the trimmed clip, words)`.
    fn fixture(text: &str) -> (Vec<i16>, Vec<f64>, Vec<String>) {
        let (pcm, truth) = FixtureProvider::new(true).render(text);
        let (lo, hi) = audible_range(&pcm).expect("audible");
        let offset = lo as f64 / SAMPLE_RATE as f64;
        (
            pcm[lo..hi].to_vec(),
            truth.iter().map(|w| w.start - offset).collect(),
            statement_words(text),
        )
    }

    #[test]
    fn word_starts_land_within_60ms_of_fixture_truth() {
        let (mut ok, mut total) = (0, 0);
        for text in SENTENCES {
            let (clip, truth, words) = fixture(text);
            let timed = time_words(&clip, SAMPLE_RATE, &words);
            assert_eq!(timed.len(), words.len());
            assert_eq!(truth.len(), words.len());
            for (w, t) in timed.iter().zip(&truth) {
                total += 1;
                if (w.start - t).abs() <= 0.060 {
                    ok += 1;
                }
            }
        }
        eprintln!("fixture accuracy: {ok}/{total} within 60 ms");
        assert!(ok * 100 >= total * 80, "{ok}/{total} within 60 ms");
    }

    #[test]
    fn output_is_monotonic_deterministic_and_confidence_graded() {
        for text in SENTENCES {
            let (clip, _, words) = fixture(text);
            let a = time_words(&clip, SAMPLE_RATE, &words);
            assert_eq!(a, time_words(&clip, SAMPLE_RATE, &words));
            let len = clip.len() as f64 / SAMPLE_RATE as f64;
            for w in &a {
                assert!(w.end >= w.start && w.end <= len + 1e-9, "{w:?}");
                assert!([0.9, 0.7, 0.5].contains(&w.confidence), "{w:?}");
            }
            for p in a.windows(2) {
                assert!(p[1].start >= p[0].start + MIN_WORD_SECONDS - 1e-9, "{p:?}");
                assert!(p[0].end <= p[1].start + 1e-9, "{p:?}");
            }
            assert_eq!(a[0].confidence, 0.9);
        }
    }

    #[test]
    fn silence_without_onsets_keeps_the_estimate_at_half_confidence() {
        // A steady tone has no onsets after the first: estimates are kept.
        let tone: Vec<i16> = (0..48_000)
            .map(|i| ((i as f64 * 0.05).sin() * 8000.0) as i16)
            .collect();
        let words: Vec<String> = ["alpha", "beta", "gamma"].map(String::from).to_vec();
        let timed = time_words(&tone, SAMPLE_RATE, &words);
        assert_eq!(timed.len(), 3);
        assert_eq!(timed[1].confidence, 0.5);
        assert_eq!(timed[2].confidence, 0.5);
        assert!(timed[1].start > timed[0].start);
        // Fully silent clip and empty word lists do not panic.
        assert_eq!(time_words(&[0; 4800], SAMPLE_RATE, &words).len(), 3);
        assert!(time_words(&tone, SAMPLE_RATE, &[]).is_empty());
        assert!(time_words(&[], SAMPLE_RATE, &words).len() == 3);
    }

    #[test]
    fn onset_detector_finds_one_onset_per_burst() {
        let (clip, truth, _) = fixture("Hello big world");
        let env = envelope_db(&clip, SAMPLE_RATE);
        let found = onsets(&env);
        for t in &truth[1..] {
            assert!(
                found.iter().any(|f| (f - t).abs() < 0.02),
                "no onset near {t}: {found:?}"
            );
        }
    }

    #[test]
    fn build_speech_with_the_timer_then_repair_is_clean() {
        use crate::config::testutil::TempDir;
        use crate::speech_build::{build_speech, pauses_for};
        use crate::{TtsModel, VoiceCache, VoiceChoice};
        use motion_core::compiler::taste::TemperamentKind;

        let dir = TempDir::new("timer_e2e");
        let statements: Vec<String> = SENTENCES.iter().map(|s| s.to_string()).collect();
        let (map, _) = build_speech(
            &statements,
            TtsModel::Say,
            &VoiceChoice {
                voice: "fx".into(),
                say_rate: None,
            },
            &FixtureProvider::new(false),
            &VoiceCache::new(dir.path().join("cache")),
            &pauses_for(TemperamentKind::Editorial),
            &dir.path().join("v.wav"),
            false,
            Some(time_words),
        )
        .expect("build");
        let spoken: usize = statements.iter().map(|s| statement_words(s).len()).sum();
        assert_eq!(map.words.len(), spoken);
        let (repaired, report) = motion_core::speech::repair(&map, &statements);
        assert!(report.unmatched_spoken_words.is_empty(), "{report:?}");
        assert!(report.unmatched_statement_words.is_empty(), "{report:?}");
        assert_eq!(report.stretched, 0, "{report:?}");
        assert_eq!(repaired.words.len(), spoken);
        for s in &repaired.sentences {
            for w in repaired.words_in(s) {
                assert!(w.start >= s.start - 1e-9 && w.end <= s.end + 1e-9);
            }
        }
    }
}
