//! (0.23 W5a) `DuckEnvelope::from_speech`: the bed's gain automation built from
//! the SpeechMap word times (spans, ramps, edge levels, empty speech,
//! determinism).

use motion_core::audio::{DuckEnvelope, MixLevels};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};

fn speech(words: &[(f64, f64)]) -> SpeechMap {
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: words.last().map_or(0.0, |w| w.1) + 0.5,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words: words
            .iter()
            .enumerate()
            .map(|(i, &(start, end))| SpeechWord {
                text: format!("w{i}"),
                start,
                end,
                confidence: 1.0,
            })
            .collect(),
        sentences: vec![SpeechSentence {
            beat: 0,
            start: words.first().map_or(0.0, |w| w.0),
            end: words.last().map_or(0.0, |w| w.1),
        }],
        recognised: Vec::new(),
        alignment: None,
    }
}

/// Three sentences of two words each: a 0.5 s gap between the first two
/// (inside the 0.8 s threshold) and a 1.2 s gap before the third.
fn three_sentences() -> SpeechMap {
    speech(&[
        (1.0, 1.4),
        (1.5, 2.0),
        // 0.5 s gap
        (2.5, 3.0),
        (3.1, 3.6),
        // 1.2 s gap
        (4.8, 5.2),
        (5.3, 5.8),
    ])
}

fn close(a: &[(f64, f64)], b: &[(f64, f64)]) {
    assert_eq!(a.len(), b.len(), "{a:?} vs {b:?}");
    for (x, y) in a.iter().zip(b) {
        assert!(
            (x.0 - y.0).abs() < 1e-9 && (x.1 - y.1).abs() < 1e-9,
            "{a:?} vs {b:?}"
        );
    }
}

#[test]
fn exact_points_for_three_sentences_with_a_short_and_a_long_gap() {
    let l = MixLevels::STANDARD;
    let env = DuckEnvelope::from_speech(&three_sentences(), &l);
    // Two spans: [1.0, 3.6] (the 0.5 s gap holds the bed down) and [4.8, 5.8].
    close(
        &env.points,
        &[
            (0.0, -4.0),
            // 150 ms before the first word the bed starts down.
            (0.85, -4.0),
            (1.0, -18.0),
            // Held through the 0.5 s gap to the end of the span.
            (3.6, -18.0),
            // Up over 500 ms from the last word end into the 1.2 s gap.
            (4.1, -10.0),
            // Down again 150 ms ahead of the next span.
            (4.65, -10.0),
            (4.8, -18.0),
            (5.8, -18.0),
            // After the last word: up over 500 ms to the edge level, held.
            (6.3, -4.0),
        ],
    );
}

#[test]
fn levels_follow_the_points_and_hold_at_the_ends() {
    let l = MixLevels::STANDARD;
    let env = DuckEnvelope::from_speech(&three_sentences(), &l);
    // Edge level before the first ramp, and held after the last.
    assert_eq!(env.level_at(0.0), -4.0);
    assert_eq!(env.level_at(0.85), -4.0);
    assert_eq!(env.level_at(100.0), -4.0);
    // Halfway down the 150 ms ramp.
    assert!((env.level_at(0.925) - -11.0).abs() < 1e-9);
    // Under speech, including the short 0.5 s gap inside the span.
    for t in [1.0, 1.7, 2.25, 3.6] {
        assert_eq!(env.level_at(t), -18.0, "t={t}");
    }
    // The long gap: ramps up, reaches long_gap_db, holds, ramps back down.
    assert!((env.level_at(3.85) - -14.0).abs() < 1e-9);
    assert_eq!(env.level_at(4.4), -10.0);
    assert!((env.level_at(4.725) - -14.0).abs() < 1e-9);
    assert_eq!(env.level_at(5.0), -18.0);
    // The bed never rises above long_gap_db inside the narration.
    let mut t = 1.0;
    while t < 5.8 {
        assert!(env.level_at(t) <= -10.0 + 1e-9, "t={t}");
        t += 0.01;
    }
}

#[test]
fn a_gap_just_under_the_threshold_does_not_open_the_bed() {
    let l = MixLevels::STANDARD;
    // 0.79 s gap: one span.
    let env = DuckEnvelope::from_speech(&speech(&[(1.0, 1.5), (2.29, 2.8)]), &l);
    close(
        &env.points,
        &[
            (0.0, -4.0),
            (0.85, -4.0),
            (1.0, -18.0),
            (2.8, -18.0),
            (3.3, -4.0),
        ],
    );
    // Exactly 0.8 s opens it: up over 0.5 s, down over 0.15 s (0.15 s hold).
    let env = DuckEnvelope::from_speech(&speech(&[(1.0, 1.5), (2.3, 2.8)]), &l);
    close(
        &env.points,
        &[
            (0.0, -4.0),
            (0.85, -4.0),
            (1.0, -18.0),
            (1.5, -18.0),
            (2.0, -10.0),
            (2.15, -10.0),
            (2.3, -18.0),
            (2.8, -18.0),
            (3.3, -4.0),
        ],
    );
}

#[test]
fn never_below_zero_seconds_and_times_increase() {
    let l = MixLevels::STANDARD;
    // The first word starts 50 ms in: the down ramp is clipped at 0 s.
    let env = DuckEnvelope::from_speech(&speech(&[(0.05, 0.4), (0.5, 1.0)]), &l);
    close(
        &env.points,
        &[(0.0, -4.0), (0.05, -18.0), (1.0, -18.0), (1.5, -4.0)],
    );
    // A word at 0 s: the bed is under from the start.
    let env = DuckEnvelope::from_speech(&speech(&[(0.0, 0.4)]), &l);
    close(&env.points, &[(0.0, -18.0), (0.4, -18.0), (0.9, -4.0)]);
    for e in [
        DuckEnvelope::from_speech(&three_sentences(), &l),
        DuckEnvelope::from_speech(&speech(&[(0.0, 0.2), (1.0, 1.1), (1.9, 2.0)]), &l),
    ] {
        assert!(e.points.iter().all(|p| p.0 >= 0.0));
        assert!(
            e.points.windows(2).all(|w| w[0].0 < w[1].0),
            "{:?}",
            e.points
        );
    }
}

#[test]
fn empty_speech_is_the_edge_level() {
    let l = MixLevels::STANDARD;
    let env = DuckEnvelope::from_speech(&speech(&[]), &l);
    assert_eq!(env.points, vec![(0.0, -4.0)]);
    assert_eq!(env.level_at(3.0), -4.0);
}

#[test]
fn the_look_table_moves_only_the_speech_level() {
    let hype = MixLevels {
        under_speech_db: -15.0,
        ..MixLevels::STANDARD
    };
    let env = DuckEnvelope::from_speech(&three_sentences(), &hype);
    assert_eq!(env.level_at(2.0), -15.0);
    assert_eq!(env.level_at(4.4), -10.0);
    assert_eq!(env.level_at(0.0), -4.0);
}

#[test]
fn unordered_words_and_overlaps_are_handled() {
    let l = MixLevels::STANDARD;
    let a = DuckEnvelope::from_speech(&three_sentences(), &l);
    let mut m = three_sentences();
    m.words.reverse();
    assert_eq!(DuckEnvelope::from_speech(&m, &l), a);
    // A long word swallowing the next ones keeps one span.
    let env = DuckEnvelope::from_speech(&speech(&[(1.0, 3.0), (1.2, 1.4), (3.5, 4.0)]), &l);
    close(
        &env.points,
        &[
            (0.0, -4.0),
            (0.85, -4.0),
            (1.0, -18.0),
            (4.0, -18.0),
            (4.5, -4.0),
        ],
    );
}

#[test]
fn deterministic() {
    let l = MixLevels::STANDARD;
    let a = DuckEnvelope::from_speech(&three_sentences(), &l);
    let b = DuckEnvelope::from_speech(&three_sentences(), &l);
    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_string(&a).expect("json"),
        serde_json::to_string(&b).expect("json")
    );
    // Points are whole milliseconds.
    for p in &a.points {
        assert!(
            ((p.0 * 1000.0).round() - p.0 * 1000.0).abs() < 1e-9,
            "{p:?}"
        );
    }
}
