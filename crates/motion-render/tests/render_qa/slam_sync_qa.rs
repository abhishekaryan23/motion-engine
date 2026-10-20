//! (0.24) Speech QA on hand-built slam beats: `reveal_before_speech` judges
//! every text leaf whose words are spoken (no builder anchors needed, lead
//! limit 150 ms), `reveal_unspoken` lists large text nobody says, and
//! `slam_density` allows at most two big text units per beat and no repeats.

use std::path::Path;

use motion_core::scene::MotionProject;
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_render::speech_qa::{speech_report, CheckStatus, SpeechQaReport};
use serde_json::{json, Value};

fn speech(words: &[(&str, f64)]) -> SpeechMap {
    let words: Vec<SpeechWord> = words
        .iter()
        .map(|(t, s)| SpeechWord {
            text: (*t).to_string(),
            start: *s,
            end: *s + 0.25,
            confidence: 0.9,
        })
        .collect();
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "voice.wav".into(),
        sample_rate: 48_000,
        duration: 6.0,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fixture".into(),
        sentences: vec![SpeechSentence {
            beat: 0,
            start: words.first().map_or(0.0, |w| w.start),
            end: words.last().map_or(0.0, |w| w.end),
        }],
        words,
    }
}

/// A word cut: a group `b1.cut.<k>` holding one text leaf `b1.cut.<k>.word`
/// that fades in at `at`.
fn cut(k: usize, text: &str, size: f64, at: f64) -> (Value, Value) {
    let word = format!("b1.cut.{k}.word");
    let group = json!({ "id": format!("b1.cut.{k}"), "type": "group", "x": 0, "y": 0,
        "width": 1080, "height": 1920, "children": [
        { "id": word, "type": "text", "text": text, "font_role": "display", "font_size": size,
          "color": "#FFFFFF", "x": 100, "y": 600, "width": 800, "height": 300 } ] });
    let fade = json!({ "target": word, "start": at, "duration": 0.01, "easing": "linear",
        "op": "fade", "from": 0.0, "to": 1.0 });
    (group, fade)
}

fn project(cuts: &[(Value, Value)]) -> MotionProject {
    let layers: Vec<Value> = cuts.iter().map(|c| c.0.clone()).collect();
    let motions: Vec<Value> = cuts.iter().map(|c| c.1.clone()).collect();
    let doc = json!({
        "version": "0.2",
        "project": { "name": "slam" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#000000" },
        "theme": { "fonts": { "display": "font_display" } },
        "assets": [ { "id": "font_display", "type": "font", "path": "assets/fonts/display.ttf" } ],
        "scenes": [ { "id": "beat_1", "start_seconds": 0.0, "duration_seconds": 5.0,
            "layers": layers, "motions": motions } ],
    });
    MotionProject::from_json(&doc.to_string()).expect("project parses")
}

fn report(p: &MotionProject, s: &SpeechMap) -> SpeechQaReport {
    speech_report(p, s, Path::new("."), None, None, None, None).expect("speech qa")
}

fn status(r: &SpeechQaReport, name: &str) -> CheckStatus {
    r.checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"))
        .status
}

#[test]
fn a_slam_word_shown_before_it_is_said_fails_without_any_builder_anchor() {
    let voice = speech(&[("This", 0.3), ("is", 0.6), ("the", 0.8), ("top", 2.0)]);
    // TOP is on screen from 0.5 s, 1.5 s before the narrator says it.
    let early = project(&[cut(0, "TOP", 200.0, 0.5)]);
    let r = report(&early, &voice);
    assert_eq!(status(&r, "reveal_before_speech"), CheckStatus::Fail);
    assert!(
        r.reveal_before_speech_findings
            .iter()
            .any(|f| f.contains("cut.0.word")),
        "{:?}",
        r.reveal_before_speech_findings
    );
    // On its word (a few ms before it at most) it passes.
    let on_time = project(&[cut(0, "TOP", 200.0, 1.9)]);
    let r = report(&on_time, &voice);
    assert_eq!(status(&r, "reveal_before_speech"), CheckStatus::Pass);
    assert!(r.reveal_timings.iter().any(|t| t.group == "cut.0.word"));
}

#[test]
fn the_lead_limit_is_150_ms() {
    let voice = speech(&[("top", 2.0)]);
    // 100 ms early is fine, 300 ms early is not.
    let near = project(&[cut(0, "TOP", 200.0, 1.9)]);
    assert_eq!(
        status(&report(&near, &voice), "reveal_before_speech"),
        CheckStatus::Pass
    );
    let far = project(&[cut(0, "TOP", 200.0, 1.7)]);
    assert_eq!(
        status(&report(&far, &voice), "reveal_before_speech"),
        CheckStatus::Fail
    );
}

#[test]
fn large_text_nobody_says_is_listed_and_small_type_is_not() {
    let voice = speech(&[("top", 2.0)]);
    let p = project(&[
        cut(0, "CLOSER", 220.0, 0.5),
        cut(1, "OPENING LINE", 70.0, 0.4),
    ]);
    let r = report(&p, &voice);
    assert_eq!(status(&r, "reveal_unspoken"), CheckStatus::Warn);
    assert_eq!(
        r.reveal_unspoken_findings.len(),
        1,
        "{:?}",
        r.reveal_unspoken_findings
    );
    assert!(r.reveal_unspoken_findings[0].contains("CLOSER"));
}

#[test]
fn slam_density_allows_two_big_units_and_no_repeats() {
    let voice = speech(&[("taxed", 1.0), ("tax", 2.0), ("flag", 3.0)]);
    let ok = project(&[cut(0, "TAXED", 200.0, 0.9), cut(1, "FLAG", 200.0, 2.9)]);
    assert_eq!(
        status(&report(&ok, &voice), "slam_density"),
        CheckStatus::Pass
    );
    // Three units.
    let three = project(&[
        cut(0, "TAXED", 200.0, 0.9),
        cut(1, "FLAG", 200.0, 2.9),
        cut(2, "BIG", 200.0, 3.5),
    ]);
    let r = report(&three, &voice);
    assert_eq!(status(&r, "slam_density"), CheckStatus::Fail);
    assert!(r.slam_density_findings[0].contains("3 big text units"));
    // A keyword unit that repeats the title's word (TAXED / TAX).
    let dup = project(&[cut(0, "TAXED", 200.0, 0.9), cut(1, "TAX", 200.0, 1.9)]);
    let r = report(&dup, &voice);
    assert_eq!(status(&r, "slam_density"), CheckStatus::Fail);
    assert!(r.slam_density_findings[0].contains("repeats"));
}
