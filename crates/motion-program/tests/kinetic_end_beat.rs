//! `end_beat` on the program's last kinetic_text: the final word keeps the
//! remaining time to the program end instead of a zero-length window (the
//! replication study's near-blank close).

use std::path::PathBuf;

use motion_program::program::MotionProgram;
use motion_program::{lower, Typesetter};

fn beats(track: &str) -> Vec<f64> {
    let root = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    let text = std::fs::read_to_string(root.join(format!("assets/music/{track}.music.json")))
        .expect("music analysis");
    let plan: motion_core::audio::MusicPlan = serde_json::from_str(&text).unwrap();
    plan.beat_times
}

fn closing(end_beat: bool) -> MotionProgram {
    let end = if end_beat { r#", "end_beat": 8"# } else { "" };
    let mut p = MotionProgram::from_json(&format!(
        r##"{{"version": "2.0", "meta": {{"name": "close", "duration": 6.0, "end_beat": 8}},
        "palette": {{"background": "#FFFFFF", "ink": "#111111", "accent": "#20A946", "muted": "#6B6B6B"}},
        "music": {{"track": "bright_upbeat"}},
        "components": [
          {{"id": "bg", "kind": "backdrop"}},
          {{"id": "open", "kind": "title", "preset": "stacked", "text": "Pledge", "start_beat": 0, "end_beat": 4}},
          {{"id": "sign", "kind": "kinetic_text", "preset": "poster_cuts",
            "words": ["SIMPLE", "AS", "THAT", "ZERO", "2030"], "beat_cuts": 1,
            "start_beat": 4{end}}}
        ]}}"##
    ))
    .expect("program");
    p.resolve_beats(&beats("bright_upbeat"));
    p
}

/// Seconds the word scenes of `sign` cover in the last 0.3 s.
fn last_word_on_screen(p: &MotionProgram) -> f64 {
    let scene = lower(p, None, &Typesetter::approx());
    let end = p.meta.duration;
    scene
        .scenes
        .iter()
        .filter(|s| s.id.starts_with("sign"))
        .map(|s| s.start_seconds + s.duration_seconds)
        .fold(0.0, f64::max)
        .min(end)
        - (end - 0.3)
}

#[test]
fn end_beat_on_the_last_kinetic_text_keeps_its_final_word() {
    let with = closing(true);
    let without = closing(false);
    // The final word stays on to the program's end, as without end_beat.
    assert!(
        last_word_on_screen(&with) > 0.29,
        "final word ends {:.3}s before the 0.3 s tail ends",
        0.3 - last_word_on_screen(&with)
    );
    assert!((last_word_on_screen(&with) - last_word_on_screen(&without)).abs() < 1e-3);
    // It gets the time from the last cut to the end (most of a beat plus
    // the cut lead), not zero.
    let last_cut = with
        .components
        .iter()
        .find(|c| c.id == "sign")
        .unwrap()
        .timeline
        .iter()
        .filter_map(|e| e.at)
        .fold(0.0, f64::max)
        + with
            .components
            .iter()
            .find(|c| c.id == "sign")
            .unwrap()
            .start;
    let beat = 60.0 / 112.5;
    assert!(
        with.meta.duration - last_cut > 0.8 * beat,
        "final word window {:.3}s",
        with.meta.duration - last_cut
    );
}
