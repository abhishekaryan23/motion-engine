//! Settled holds (`qa::settled_holds` + `qa::excuse_settled_holds`): a
//! read-hold of at most two beats (1 s without music) after a component's
//! last build step is not a frozen frame; an opening freeze, a component
//! that never animated and a longer freeze still are.

use motion_program::qa::{
    beat_coverage, continuous_build_end, excuse_settled_holds, settled_holds,
};
use motion_program::MotionProgram;

fn program(components: &str) -> MotionProgram {
    MotionProgram::from_json(&format!(
        r##"{{"version": "2.0", "meta": {{"name": "hold", "duration": 20.0}},
        "palette": {{"background": "#F2EEE6", "ink": "#111111", "accent": "#E04A12", "muted": "#6B6B6B"}},
        "components": [{components}]}}"##
    ))
    .expect("program")
}

/// Beat-spaced samples (0.5 s beats) across the 20 s program.
fn samples() -> Vec<f64> {
    (0..40).map(|k| k as f64 * 0.5).collect()
}

const ENTERS_AT_2: &str =
    r#"{"id": "t", "kind": "title", "preset": "stacked", "text": "Motion", "start": 2.0}"#;

#[test]
fn two_beat_hold_after_the_last_step_is_allowed_once() {
    let p = program(ENTERS_AT_2);
    let holds = settled_holds(&p, Some(0.5));
    assert_eq!(holds.len(), 1);
    // Still on the two beats after the title lands.
    assert_eq!(
        excuse_settled_holds(&holds, &[2.5, 3.0], &samples()),
        Vec::<f64>::new()
    );
    // A second still run on the same component is not excused.
    assert_eq!(
        excuse_settled_holds(&holds, &[2.5, 3.0, 4.5], &samples()),
        vec![4.5]
    );
    // Without music the hold is 1.0 s (two 0.5 s samples).
    let holds = settled_holds(&p, None);
    assert_eq!(
        excuse_settled_holds(&holds, &[2.5, 3.0], &samples()),
        Vec::<f64>::new()
    );
}

#[test]
fn three_beat_freeze_still_fails() {
    let p = program(ENTERS_AT_2);
    let holds = settled_holds(&p, Some(0.5));
    assert_eq!(
        excuse_settled_holds(&holds, &[2.5, 3.0, 3.5], &samples()),
        vec![2.5, 3.0, 3.5]
    );
}

#[test]
fn no_hold_at_the_program_start() {
    let p = program(
        r#"{"id": "t", "kind": "title", "preset": "stacked", "text": "Motion", "start": 0.25}"#,
    );
    let holds = settled_holds(&p, Some(0.5));
    assert_eq!(holds.len(), 1);
    assert_eq!(
        excuse_settled_holds(&holds, &[0.5, 1.0], &samples()),
        vec![0.5, 1.0]
    );
}

#[test]
fn no_hold_for_a_component_that_never_animated() {
    // On screen from 0 s with no timeline: it never builds.
    let p = program(
        r#"{"id": "bg", "kind": "backdrop"},
        {"id": "t", "kind": "title", "preset": "stacked", "text": "Motion"}"#,
    );
    let holds = settled_holds(&p, Some(0.5));
    assert!(holds.is_empty(), "{holds:?}");
    assert_eq!(
        excuse_settled_holds(&holds, &[2.5, 3.0], &samples()),
        vec![2.5, 3.0]
    );
}

#[test]
fn hold_follows_the_last_timeline_step() {
    // Enters at 1.0 s, pops at 3.0 s: stillness at 1.5–2.5 is mid-build
    // (before the last step), so it is not a settled hold.
    let p = program(
        r#"{"id": "t", "kind": "title", "preset": "stacked", "text": "Motion", "start": 1.0,
            "timeline": [{"at": 2.0, "do": "sheen"}]}"#,
    );
    let holds = settled_holds(&p, Some(0.5));
    assert_eq!(
        excuse_settled_holds(&holds, &[1.5, 2.0], &samples()),
        vec![1.5, 2.0]
    );
    assert_eq!(
        excuse_settled_holds(&holds, &[3.5, 4.0], &samples()),
        Vec::<f64>::new()
    );
}

fn short(duration: f64, components: &str) -> MotionProgram {
    MotionProgram::from_json(&format!(
        r##"{{"version": "2.0", "meta": {{"name": "hold", "duration": {duration}}},
        "palette": {{"background": "#F2EEE6", "ink": "#111111", "accent": "#E04A12", "muted": "#6B6B6B"}},
        "components": [{components}]}}"##
    ))
    .expect("program")
}

const TILES: &str =
    r#""tiles": [{"stat": "38", "title": "IDENTITIES"}, {"stat": "14", "title": "LAUNCHES"}]"#;

#[test]
fn a_chain_of_holds_hits_the_program_cap() {
    // No music: 0.5 s samples over 7 s; every component holds 1 s after it
    // enters. Only the first hold fits the 15 % cap.
    let p = short(
        7.0,
        &format!(
            r#"{{"id": "b1", "kind": "bento", {TILES}, "start": 1.0, "end": 2.5}},
            {{"id": "l1", "kind": "lockup", "headline": "Fernway", "start": 2.5, "end": 4.0}},
            {{"id": "b2", "kind": "bento", {TILES}, "start": 4.0, "end": 5.5}},
            {{"id": "l2", "kind": "lockup", "headline": "Fernway", "start": 5.5}}"#
        ),
    );
    let samples: Vec<f64> = (0..14).map(|k| k as f64 * 0.5).collect();
    let still = [1.5, 2.0, 3.0, 3.5, 4.5, 5.0, 6.0, 6.5];
    let left = excuse_settled_holds(&settled_holds(&p, None), &still, &samples);
    assert_eq!(left, vec![3.0, 3.5, 4.5, 5.0, 6.0, 6.5]);
    // A single chart hold still passes.
    let p = short(
        7.0,
        r#"{"id": "c", "kind": "chart", "chart": "bar", "values": [1, 2, 3], "start": 1.0}"#,
    );
    assert_eq!(
        excuse_settled_holds(&settled_holds(&p, None), &[1.5, 2.0], &samples),
        Vec::<f64>::new()
    );
}

const TWO_STROKES: &str = r#"{"id": "s", "kind": "sketch", "preset": "freeform",
    "elements": [{"shape": "line", "from": [0.2, 0.5], "to": [0.8, 0.5]},
                 {"shape": "line", "from": [0.5, 0.2], "to": [0.5, 0.8]}]}"#;

#[test]
fn a_drawing_sketch_from_zero_gets_a_hold_after_its_strokes() {
    let p = short(20.0, TWO_STROKES);
    let holds = settled_holds(&p, None);
    assert_eq!(holds.len(), 1, "{holds:?}");
    assert!(holds[0].from > 0.0, "{holds:?}");
}

#[test]
fn freeform_strokes_credit_beats_only_while_they_draw() {
    // Two strokes over five 0.5 s beats: the last lands near 1.05 s, so the
    // beats at 1.5 and 2.0 s are not credited as drawing.
    let p = short(2.5, TWO_STROKES);
    let c = &p.components[0];
    let end = continuous_build_end(&p, c).expect("drawing");
    assert!(end < 1.15, "draws until {end:.3}");
    let beats: Vec<f64> = (0..6).map(|k| k as f64 * 0.5).collect();
    let cov = beat_coverage(&p, &beats, &[0.0, 2.0]);
    assert!(cov.coverage < 0.61, "{}", cov.report());
}
