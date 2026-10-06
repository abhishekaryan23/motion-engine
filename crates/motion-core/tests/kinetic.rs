//! Kinetic typography behaviors: structure, timing rules, stagger conformance,
//! validation and evaluated start/end states. Runs are built by hand (fixed
//! fake metrics), so no fonts are needed. Also writes and checks the
//! `kinetic_typography` / `stagger` golden scenes.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use motion_core::easing::{Easing, MotionPreset, PresetParams};
use motion_core::motion::kinetic::{
    keyword_punch, line_reveal, text_mask_reveal, tracking_reveal, type_replace,
    type_scale_emphasis, word_cascade, KineticParams, TextRun, UnitBox,
};
use motion_core::motion::stagger::{self, StaggerOrder, StaggerPreset, StaggerSpec};
use motion_core::motion::Expansion;
use motion_core::scene::{
    Channel, Color, Direction, FontRole, InkBounds, LayerKind, Motion, MotionOp, MotionProject,
    RevealMode, TextAlign, TextStyle,
};
use motion_core::timeline::{ResolvedFrame, ResolvedLayer};
use motion_core::{evaluate_frame, validate};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

const CHAR_W: f32 = 30.0;
const GAP: f32 = 24.0;
const LINE_ADVANCE: f32 = 100.0;
const ORIGIN: [f32; 2] = [80.0, 200.0];
const ACCENT: Color = Color::rgb(0xD6, 0x3A, 0x2B);

fn style() -> TextStyle {
    TextStyle {
        text: String::new(),
        font_role: FontRole::Display,
        font_size: 100.0,
        font_weight: 900,
        italic: false,
        color: Color::rgb(0x16, 0x13, 0x0F),
        align: TextAlign::Left,
        line_height: 1.0,
        letter_spacing: -0.02,
        max_width: None,
        uppercase: false,
        ink: None,
    }
}

/// A run with fake metrics: every char is `CHAR_W` wide, words are `GAP` apart.
fn run(id: &str, lines: &[&[&str]], ink: Option<InkBounds>) -> TextRun {
    let lines = lines
        .iter()
        .map(|words| {
            let mut x = 0.0;
            words
                .iter()
                .map(|w| {
                    let width = w.chars().count() as f32 * CHAR_W;
                    let unit = UnitBox {
                        text: (*w).to_string(),
                        x,
                        width,
                    };
                    x += width + GAP;
                    unit
                })
                .collect()
        })
        .collect();
    TextRun {
        id: id.to_string(),
        style: style(),
        origin: ORIGIN,
        line_advance: LINE_ADVANCE,
        lines,
        ink,
    }
}

fn ink() -> Option<InkBounds> {
    Some(InkBounds {
        top: 18.0,
        bottom: 88.0,
    })
}

fn headline() -> TextRun {
    run("hd", &[&["ONE", "IDEA,"], &["ONE", "MOVE"]], ink())
}

fn params_with(preset: PresetParams, stagger: StaggerSpec) -> KineticParams {
    KineticParams {
        preset,
        stagger,
        u: 1.0,
    }
}

fn params() -> KineticParams {
    params_with(
        MotionPreset::Editorial.params(),
        StaggerSpec {
            preset: StaggerPreset::Editorial,
            order: StaggerOrder::Forward,
        },
    )
}

fn layer_ids(exp: &Expansion) -> Vec<&str> {
    exp.layers.iter().map(|l| l.id.as_str()).collect()
}

fn motions_of<'a>(exp: &'a Expansion, id: &str) -> Vec<&'a Motion> {
    exp.motions.iter().filter(|m| m.target == id).collect()
}

fn on_channel<'a>(exp: &'a Expansion, id: &str, ch: Channel) -> Vec<&'a Motion> {
    let mut v: Vec<&Motion> = motions_of(exp, id)
        .into_iter()
        .filter(|m| m.op.channel() == ch)
        .collect();
    v.sort_by(|a, b| a.start.total_cmp(&b.start));
    v
}

fn first_start(exp: &Expansion, id: &str) -> f64 {
    motions_of(exp, id)
        .iter()
        .map(|m| m.start)
        .fold(f64::INFINITY, f64::min)
}

fn end_of(exp: &Expansion) -> f64 {
    exp.motions
        .iter()
        .map(|m| m.start + m.duration)
        .fold(0.0, f64::max)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Structural rules every expansion must satisfy.
fn check_structure(exp: &Expansion) {
    let mut seen: HashMap<&str, ()> = HashMap::new();
    for l in &exp.layers {
        assert!(seen.insert(l.id.as_str(), ()).is_none(), "dup id {}", l.id);
    }
    for m in &exp.motions {
        assert!(
            seen.contains_key(m.target.as_str()),
            "motion targets missing layer {}",
            m.target
        );
        assert!(m.start.is_finite() && m.start >= 0.0, "bad start {m:?}");
        assert!(m.duration.is_finite() && m.duration > 0.0, "bad dur {m:?}");
    }
    for l in &exp.layers {
        let mut by_ch: HashMap<Channel, Vec<(f64, f64)>> = HashMap::new();
        for m in motions_of(exp, &l.id) {
            by_ch
                .entry(m.op.channel())
                .or_default()
                .push((m.start, m.start + m.duration));
        }
        for (ch, mut spans) in by_ch {
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            for w in spans.windows(2) {
                assert!(
                    w[0].1 <= w[1].0 + 1e-9,
                    "layer {} channel {ch:?} overlaps: {w:?}",
                    l.id
                );
            }
        }
    }
}

fn project_for(exp: &Expansion) -> MotionProject {
    let duration = end_of(exp) + 0.5;
    let v = json!({
        "version": "0.2",
        "project": { "name": "kinetic_test" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#ECE3D2" },
        "theme": {
            "fonts": {
                "display": "font.a", "display_condensed": "font.a", "serif_emotional": "font.a",
                "body": "font.a", "mono": "font.a", "number": "font.a"
            }
        },
        "assets": [{ "id": "font.a", "type": "font", "path": "fonts/ArchivoBlack-Regular.ttf" }],
        "scenes": [{ "id": "s", "start_seconds": 0.0, "duration_seconds": duration }],
    });
    let mut p = MotionProject::from_json(&v.to_string()).expect("project parses");
    p.scenes[0].layers = exp.layers.clone();
    p.scenes[0].motions = exp.motions.clone();
    p
}

fn assert_valid(exp: &Expansion) {
    let p = project_for(exp);
    // No base dir: skips font file existence checks.
    if let Err(e) = validate(&p, None) {
        panic!("expansion must validate:\n{e}");
    }
}

fn frame_at(t: f64) -> u32 {
    (t * 30.0).round() as u32
}

fn layer_in<'a, 'f>(f: &'f ResolvedFrame<'a>, id: &str) -> Option<&'f ResolvedLayer<'a>> {
    f.layers.iter().find(|l| l.id == id)
}

/// Box top-left the layer should end at, from the unit measurements.
fn measured(run: &TextRun, line: usize, unit: usize) -> (f32, f32) {
    (
        run.origin[0] + run.lines[line][unit].x,
        run.origin[1] + line as f32 * run.line_advance,
    )
}

fn assert_at_rest(p: &MotionProject, t: f64, id: &str, run: &TextRun, line: usize, unit: usize) {
    let f = evaluate_frame(p, frame_at(t)).expect("evaluates");
    let l = layer_in(&f, id).unwrap_or_else(|| panic!("{id} missing at t={t}"));
    let (x, y) = measured(run, line, unit);
    assert!(
        (l.transform.e - x).abs() < 0.5,
        "{id} x {} vs {x}",
        l.transform.e
    );
    assert!(
        (l.transform.f - y).abs() < 0.5,
        "{id} y {} vs {y}",
        l.transform.f
    );
    assert!((l.opacity - 1.0).abs() < 1e-4, "{id} opacity {}", l.opacity);
    assert!(
        (l.transform.a - 1.0).abs() < 1e-4,
        "{id} scale {}",
        l.transform.a
    );
}

// ---------------------------------------------------------------------------
// word_cascade
// ---------------------------------------------------------------------------

#[test]
fn word_cascade_layers_and_ids() {
    let r = headline();
    let exp = word_cascade(&r, 0.4, &params());
    assert_eq!(
        layer_ids(&exp),
        vec!["hd.0.0", "hd.0.1", "hd.1.0", "hd.1.1"]
    );
    check_structure(&exp);
    for l in &exp.layers {
        let LayerKind::Text(t) = &l.kind else {
            panic!("unit layers are text")
        };
        assert_eq!(t.align, TextAlign::Left);
        assert_eq!(t.ink, r.ink);
        assert!(!t.text.is_empty() && !t.text.contains(' '));
        assert_eq!(l.anchor_x, 0.5);
        // Anchor follows the ink center: (18 + 88) / 2 / 100.
        assert!((l.anchor_y - 0.53).abs() < 1e-5);
        assert_eq!(l.height, 100.0);
    }
    // Every unit has exactly one fade in and one rise, both starting together.
    for l in &exp.layers {
        let fades = on_channel(&exp, &l.id, Channel::Opacity);
        let moves = on_channel(&exp, &l.id, Channel::Offset);
        assert_eq!((fades.len(), moves.len()), (1, 1));
        assert!(near(fades[0].start, moves[0].start));
        assert!(fades[0].duration < moves[0].duration);
        assert_eq!(moves[0].easing, params().preset.easing);
        assert!(matches!(fades[0].op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0));
        assert!(matches!(
            moves[0].op,
            MotionOp::Move { from, to } if from == [0.0, 90.0 * 0.45] && to == [0.0, 0.0]
        ));
    }
}

#[test]
fn word_cascade_starts_follow_stagger() {
    let r = headline();
    let p = params();
    let exp = word_cascade(&r, 0.4, &p);
    let offs = stagger::offsets(p.stagger, 4);
    for (k, id) in ["hd.0.0", "hd.0.1", "hd.1.0", "hd.1.1"].iter().enumerate() {
        assert!(near(first_start(&exp, id), 0.4 + offs[k]), "{id}");
    }
    // Reverse order flips who goes first.
    let rev = StaggerSpec {
        order: StaggerOrder::Reverse,
        ..p.stagger
    };
    let exp = word_cascade(&r, 0.0, &params_with(p.preset, rev));
    assert!(near(first_start(&exp, "hd.1.1"), 0.0));
    assert!(first_start(&exp, "hd.0.0") > first_start(&exp, "hd.1.0"));
}

#[test]
fn word_cascade_is_not_robotic() {
    let r = run("w", &[&["A", "B", "C", "D", "E", "F"]], None);
    let exp = word_cascade(&r, 0.0, &params());
    let mut durs: Vec<f64> = exp
        .layers
        .iter()
        .map(|l| on_channel(&exp, &l.id, Channel::Offset)[0].duration)
        .collect();
    let base = params().preset.duration;
    for d in &durs {
        assert!(
            (d / base - 1.0).abs() <= 0.0601,
            "duration {d} strays too far"
        );
    }
    durs.sort_by(f64::total_cmp);
    durs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert!(durs.len() >= 4, "durations should vary: {durs:?}");
    // Nothing but the rank-0 unit starts at exactly the cascade start.
    let starts: Vec<f64> = exp
        .layers
        .iter()
        .map(|l| first_start(&exp, &l.id))
        .collect();
    assert_eq!(starts.iter().filter(|s| near(**s, 0.0)).count(), 1);
    // No ink: anchor falls back to the box center.
    assert!((exp.layers[0].anchor_y - 0.5).abs() < 1e-6);
}

#[test]
fn word_cascade_positions_and_evaluation() {
    let r = headline();
    let exp = word_cascade(&r, 0.4, &params());
    let p = project_for(&exp);
    // Before the cascade: nothing visible (opacity 0 drops the layer).
    let before = evaluate_frame(&p, 0).expect("evaluates");
    assert!(before.layers.is_empty());
    // Mid-way the first word is partly in, the last not yet started.
    let mid = evaluate_frame(&p, frame_at(0.4 + 0.12)).expect("evaluates");
    assert!(layer_in(&mid, "hd.0.0").is_some());
    assert!(layer_in(&mid, "hd.1.1").is_none());
    // After: everything at its measured place, fully opaque.
    let t_end = end_of(&exp) + 0.1;
    for (l, u, id) in [
        (0, 0, "hd.0.0"),
        (0, 1, "hd.0.1"),
        (1, 0, "hd.1.0"),
        (1, 1, "hd.1.1"),
    ] {
        assert_at_rest(&p, t_end, id, &r, l, u);
    }
}

#[test]
fn word_cascade_compresses_long_runs() {
    let words: Vec<&str> = vec!["W"; 40];
    let r = run("long", &[&words], None);
    let exp = word_cascade(&r, 0.0, &params());
    let last = first_start(&exp, "long.0.39");
    assert!(last <= 2.0 + 1e-9, "spread should be capped, got {last}");
    check_structure(&exp);
}

#[test]
fn word_cascade_valid_and_deterministic() {
    let r = headline();
    let a = word_cascade(&r, 0.4, &params());
    let b = word_cascade(&r, 0.4, &params());
    assert_eq!(a, b);
    assert_valid(&a);
    for preset in [
        MotionPreset::Calm,
        MotionPreset::Editorial,
        MotionPreset::Impact,
    ] {
        let mut p = params();
        p.preset = preset.params();
        let exp = word_cascade(&r, 0.0, &p);
        check_structure(&exp);
        assert_valid(&exp);
    }
}

// ---------------------------------------------------------------------------
// line_reveal / text_mask_reveal
// ---------------------------------------------------------------------------

#[test]
fn line_reveal_per_line_clip_reveal_up() {
    let r = run(
        "ln",
        &[&["ONE", "TWO"], &["THREE"], &["FOUR", "FIVE", "SIX"]],
        ink(),
    );
    let p = params();
    let exp = line_reveal(&r, 0.2, &p);
    assert_eq!(exp.layers.len(), 6);
    assert_eq!(exp.motions.len(), 6, "one motion per unit");
    check_structure(&exp);
    let offs = stagger::offsets(p.stagger, 3);
    for (line, units) in r.lines.iter().enumerate() {
        let mut spans = Vec::new();
        for unit in 0..units.len() {
            let id = format!("ln.{line}.{unit}");
            let ms = motions_of(&exp, &id);
            assert_eq!(ms.len(), 1);
            assert!(matches!(
                ms[0].op,
                MotionOp::ClipReveal {
                    direction: Direction::Up,
                    mode: RevealMode::Reveal
                }
            ));
            assert_ne!(ms[0].easing, Easing::ImpactSpring);
            spans.push((ms[0].start, ms[0].duration));
        }
        // Words of a line share timing; the line follows the stagger.
        assert!(
            spans.windows(2).all(|w| w[0] == w[1]),
            "line {line}: {spans:?}"
        );
        assert!(near(spans[0].0, 0.2 + offs[line]));
    }
    assert_valid(&exp);
    assert_eq!(exp, line_reveal(&r, 0.2, &p));
}

#[test]
fn line_reveal_evaluates_to_rest_position() {
    let r = run("ln", &[&["ONE", "TWO"], &["THREE"]], ink());
    let exp = line_reveal(&r, 0.0, &params());
    let p = project_for(&exp);
    let t_end = end_of(&exp) + 0.1;
    assert_at_rest(&p, t_end, "ln.0.1", &r, 0, 1);
    assert_at_rest(&p, t_end, "ln.1.0", &r, 1, 0);
    // At the very start the content sits fully below its box (hidden by the clip).
    let f = evaluate_frame(&p, 0).expect("evaluates");
    let l = layer_in(&f, "ln.0.0").expect("present, content clipped");
    assert!(l.content_transform.f > l.transform.f + 90.0);
    assert!(l.clip.is_some());
}

#[test]
fn text_mask_reveal_uses_direction_per_line() {
    let r = run("mk", &[&["ONE", "TWO"], &["THREE", "FOUR"]], ink());
    let p = params();
    for dir in [
        Direction::Right,
        Direction::Left,
        Direction::Down,
        Direction::Up,
    ] {
        let exp = text_mask_reveal(&r, 0.3, dir, &p);
        assert_eq!(exp.layers.len(), 4);
        check_structure(&exp);
        assert_valid(&exp);
        let offs = stagger::offsets(p.stagger, 2);
        for (line, units) in r.lines.iter().enumerate() {
            let a = &motions_of(&exp, &format!("mk.{line}.0"))[0];
            let b = &motions_of(&exp, &format!("mk.{line}.1"))[0];
            assert_eq!((a.start, a.duration), (b.start, b.duration));
            assert!(near(a.start, 0.3 + offs[line]));
            assert!(matches!(
                a.op,
                MotionOp::MaskReveal { direction, mode: RevealMode::Reveal } if direction == dir
            ));
            let _ = units;
        }
        assert_eq!(exp, text_mask_reveal(&r, 0.3, dir, &p));
    }
    let exp = text_mask_reveal(&r, 0.0, Direction::Right, &p);
    let proj = project_for(&exp);
    assert_at_rest(&proj, end_of(&exp) + 0.1, "mk.1.1", &r, 1, 1);
    // Fully masked at t = 0: the first line has no visible area yet.
    let f = evaluate_frame(&proj, 0).expect("evaluates");
    assert!(layer_in(&f, "mk.0.0").is_none());
}

// ---------------------------------------------------------------------------
// type_replace
// ---------------------------------------------------------------------------

fn replace_runs() -> (TextRun, TextRun) {
    (
        run("old", &[&["STILL", "LIFE"]], ink()),
        run("new", &[&["LIVE", "MOTION"]], ink()),
    )
}

#[test]
fn type_replace_structure_and_timing() {
    let (old, new) = replace_runs();
    let p = params();
    let at = 1.5;
    let exp = type_replace(&old, &new, at, &p);
    assert_eq!(
        layer_ids(&exp),
        vec!["old.0.0", "old.0.1", "new.0.0", "new.0.1"]
    );
    check_structure(&exp);
    let offs = stagger::offsets(p.stagger, 2);
    for (k, id) in ["old.0.0", "old.0.1"].iter().enumerate() {
        // Visible before `at`: nothing acts on it earlier, and it never enters.
        assert!(near(first_start(&exp, id), at + offs[k]));
        let fades = on_channel(&exp, id, Channel::Opacity);
        assert_eq!(fades.len(), 1);
        assert!(matches!(fades[0].op, MotionOp::Fade { from, to } if from == 1.0 && to == 0.0));
        // Anticipation dip then a lift, sequential on the offset channel.
        let moves = on_channel(&exp, id, Channel::Offset);
        assert_eq!(moves.len(), 2);
        let up = |m: &Motion| matches!(m.op, MotionOp::Move { to, .. } if to[1] < 0.0);
        assert!(!up(moves[0]) && up(moves[1]));
    }
    let arrive = at + p.preset.duration * 0.35;
    for (k, id) in ["new.0.0", "new.0.1"].iter().enumerate() {
        assert!(near(first_start(&exp, id), arrive + offs[k]));
        let fade = &on_channel(&exp, id, Channel::Opacity)[0];
        assert!(matches!(fade.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0));
    }
    assert_valid(&exp);
    assert_eq!(exp, type_replace(&old, &new, at, &p));
}

#[test]
fn type_replace_states() {
    let (old, new) = replace_runs();
    let exp = type_replace(&old, &new, 1.5, &params());
    let p = project_for(&exp);
    // Before: old at rest, new absent.
    assert_at_rest(&p, 1.2, "old.0.0", &old, 0, 0);
    assert_at_rest(&p, 1.49, "old.0.1", &old, 0, 1);
    let f = evaluate_frame(&p, frame_at(1.2)).expect("evaluates");
    assert!(layer_in(&f, "new.0.0").is_none());
    // After: old gone, new at rest.
    let t_end = end_of(&exp) + 0.1;
    let f = evaluate_frame(&p, frame_at(t_end)).expect("evaluates");
    assert!(layer_in(&f, "old.0.0").is_none() && layer_in(&f, "old.0.1").is_none());
    assert_at_rest(&p, t_end, "new.0.0", &new, 0, 0);
    assert_at_rest(&p, t_end, "new.0.1", &new, 0, 1);
    // Mid-swap both texts exist on screen at once.
    let mid = evaluate_frame(&p, frame_at(1.5 + 0.35)).expect("evaluates");
    assert!(layer_in(&mid, "new.0.0").is_some());
}

// ---------------------------------------------------------------------------
// keyword behaviors
// ---------------------------------------------------------------------------

fn sentence() -> TextRun {
    run("sn", &[&["MADE", "TO", "FEEL", "IT"]], ink())
}

#[test]
fn scale_emphasis_hits_keyword_only() {
    let r = sentence();
    let p = params();
    let exp = type_scale_emphasis(&r, (0, 2), 0.2, 1.8, &p);
    assert_eq!(exp.layers.len(), 4);
    check_structure(&exp);
    let scales: Vec<&str> = exp
        .layers
        .iter()
        .filter(|l| !on_channel(&exp, &l.id, Channel::Scale).is_empty())
        .map(|l| l.id.as_str())
        .collect();
    assert_eq!(scales, vec!["sn.0.2"]);
    let sc = on_channel(&exp, "sn.0.2", Channel::Scale);
    assert_eq!(sc.len(), 2, "anticipation dip then growth");
    assert!(matches!(sc[0].op, MotionOp::Scale { from, to, .. } if from == 1.0 && to < 1.0));
    assert!(matches!(sc[1].op, MotionOp::Scale { to, .. } if (to - 1.16).abs() < 1e-6));
    assert_eq!(sc[1].easing, p.preset.settle);
    assert!(sc[0].start >= 1.8 - 1e-9);
    // The keyword is not dimmed; every other unit is, after its entrance fade.
    for l in &exp.layers {
        let fades = on_channel(&exp, &l.id, Channel::Opacity);
        if l.id == "sn.0.2" {
            assert_eq!(fades.len(), 1);
        } else {
            assert_eq!(fades.len(), 2, "{}", l.id);
            assert!(
                matches!(fades[1].op, MotionOp::Fade { from, to } if from == 1.0 && to > 0.3 && to < 0.6)
            );
            assert!(fades[1].start >= fades[0].start + fades[0].duration - 1e-9);
            assert!(fades[1].start >= 1.8 - 1e-9);
        }
    }
    assert_valid(&exp);
    assert_eq!(exp, type_scale_emphasis(&r, (0, 2), 0.2, 1.8, &p));
}

#[test]
fn scale_emphasis_waits_for_late_entrance() {
    // Emphasis requested while the cascade is still entering: dimming must
    // never overlap the entrance fade of the same unit.
    let r = sentence();
    let exp = type_scale_emphasis(&r, (0, 0), 0.0, 0.0, &params());
    check_structure(&exp);
    assert_valid(&exp);
}

#[test]
fn scale_emphasis_evaluated_states() {
    let r = sentence();
    let exp = type_scale_emphasis(&r, (0, 2), 0.2, 1.8, &params());
    let p = project_for(&exp);
    let t_before = 1.7;
    for (u, id) in ["sn.0.0", "sn.0.1", "sn.0.2", "sn.0.3"].iter().enumerate() {
        assert_at_rest(&p, t_before, id, &r, 0, u);
    }
    let t_end = end_of(&exp) + 0.1;
    let f = evaluate_frame(&p, frame_at(t_end)).expect("evaluates");
    let kw = layer_in(&f, "sn.0.2").expect("keyword");
    assert!((kw.transform.a - 1.16).abs() < 1e-4);
    assert!((kw.opacity - 1.0).abs() < 1e-4);
    // Scale pivots on the unit center: the box center stays put.
    let (x, y) = measured(&r, 0, 2);
    let w = r.lines[0][2].width * 1.02 + 2.0;
    let cx = kw.transform.e + kw.transform.a * w * 0.5;
    assert!((cx - (x + w * 0.5)).abs() < 0.5);
    let cy = kw.transform.f + kw.transform.d * 100.0 * 0.53;
    assert!((cy - (y + 53.0)).abs() < 0.5);
    for id in ["sn.0.0", "sn.0.1", "sn.0.3"] {
        let l = layer_in(&f, id).expect("dimmed, not gone");
        assert!((l.opacity - 0.45).abs() < 1e-4, "{id} {}", l.opacity);
    }
}

#[test]
fn keyword_punch_adds_accent_copy_above() {
    let r = sentence();
    let p = params();
    let exp = keyword_punch(&r, (0, 2), 0.2, 1.8, ACCENT, &p);
    assert_eq!(
        layer_ids(&exp),
        vec!["sn.0.0", "sn.0.1", "sn.0.2", "sn.0.3", "sn.punch"]
    );
    check_structure(&exp);
    let punch = exp
        .layers
        .last()
        .expect("punch layer is last (drawn above)");
    assert_eq!(punch.id, "sn.punch");
    let LayerKind::Text(t) = &punch.kind else {
        panic!("text")
    };
    assert_eq!(t.color, ACCENT);
    assert_eq!(t.text, "FEEL");
    let kw = &exp.layers[2];
    assert_eq!((punch.x, punch.y, punch.width), (kw.x, kw.y, kw.width));
    // Punch timing: fade in quickly, overshooting scale from 1.35.
    let fade = &on_channel(&exp, "sn.punch", Channel::Opacity)[0];
    let sc = &on_channel(&exp, "sn.punch", Channel::Scale)[0];
    assert!(fade.start >= 1.8 - 1e-9 && fade.duration < 0.15);
    assert!(near(fade.start, sc.start));
    assert_eq!(sc.easing, Easing::ImpactSpring);
    assert!(matches!(sc.op, MotionOp::Scale { from, to, .. } if from == 1.35 && to == 1.0));
    // Only the keyword's ink fades out under the punch.
    for l in &exp.layers[..4] {
        let fades = on_channel(&exp, &l.id, Channel::Opacity);
        if l.id == "sn.0.2" {
            assert_eq!(fades.len(), 2);
            assert!(matches!(fades[1].op, MotionOp::Fade { from, to } if from == 1.0 && to == 0.0));
            assert!(fades[1].start >= fade.start);
        } else {
            assert_eq!(fades.len(), 1);
        }
    }
    assert_valid(&exp);
    assert_eq!(exp, keyword_punch(&r, (0, 2), 0.2, 1.8, ACCENT, &p));
}

#[test]
fn keyword_punch_evaluated_states() {
    let r = sentence();
    let exp = keyword_punch(&r, (0, 1), 0.2, 1.8, ACCENT, &params());
    let p = project_for(&exp);
    // Before the strike the punch copy is invisible and the ink word is there.
    let f = evaluate_frame(&p, frame_at(1.7)).expect("evaluates");
    assert!(layer_in(&f, "sn.punch").is_none());
    assert!(layer_in(&f, "sn.0.1").is_some());
    // After: accent copy at rest over the measured spot, ink word gone.
    let t_end = end_of(&exp) + 0.1;
    assert_at_rest(&p, t_end, "sn.punch", &r, 0, 1);
    let f = evaluate_frame(&p, frame_at(t_end)).expect("evaluates");
    assert!(layer_in(&f, "sn.0.1").is_none());
    // The copy starts larger than the word it lands on.
    let hit = on_channel(&exp, "sn.punch", Channel::Scale)[0].start;
    let f = evaluate_frame(&p, frame_at(hit + 0.05)).expect("evaluates");
    assert!(layer_in(&f, "sn.punch").expect("punch").transform.a > 1.1);
}

#[test]
fn keyword_behaviors_with_missing_keyword_fall_back_to_cascade() {
    let r = sentence();
    let base = word_cascade(&r, 0.0, &params());
    assert_eq!(type_scale_emphasis(&r, (3, 9), 0.0, 1.0, &params()), base);
    assert_eq!(keyword_punch(&r, (0, 7), 0.0, 1.0, ACCENT, &params()), base);
}

// ---------------------------------------------------------------------------
// tracking_reveal
// ---------------------------------------------------------------------------

fn glyph_run() -> TextRun {
    run("tr", &[&["T", "R", "A", "C", "K", "S"]], ink())
}

#[test]
fn tracking_reveal_converges_from_center() {
    let r = glyph_run();
    let p = params();
    let exp = tracking_reveal(&r, 0.3, &p);
    assert_eq!(exp.layers.len(), 6);
    check_structure(&exp);
    let line_center = (r.lines[0][0].x + r.lines[0][5].x + r.lines[0][5].width) * 0.5;
    for (i, u) in r.lines[0].iter().enumerate() {
        let id = format!("tr.0.{i}");
        let m = &on_channel(&exp, &id, Channel::Offset)[0];
        let MotionOp::Move { from, to } = m.op else {
            panic!("move")
        };
        let expect = (u.x + u.width * 0.5 - line_center) * 0.6;
        assert!(
            (from[0] - expect).abs() < 1e-4,
            "{id}: {} vs {expect}",
            from[0]
        );
        assert_eq!((from[1], to), (0.0, [0.0, 0.0]));
        assert_eq!(m.easing, p.preset.easing);
        // Displaced away from the center, so left clusters start left of home.
        assert_eq!(from[0] < 0.0, i < 3);
        let fade = &on_channel(&exp, &id, Channel::Opacity)[0];
        assert!(near(fade.start, m.start));
    }
    // CenterOut: middle clusters go before the ends.
    let s = |i: usize| first_start(&exp, &format!("tr.0.{i}"));
    assert!(s(2) < s(1) && s(3) > s(2) && s(0) > s(1) && s(5) > s(4));
    let offs = stagger::offsets(
        StaggerSpec {
            preset: StaggerPreset::Editorial,
            order: StaggerOrder::CenterOut,
        },
        6,
    );
    // Glyph stagger is compressed to a 0.5 s spread.
    let max = offs.iter().copied().fold(0.0, f64::max);
    let k = if max > 0.5 { 0.5 / max } else { 1.0 };
    for (i, o) in offs.iter().enumerate() {
        assert!(near(s(i), 0.3 + o * k), "cluster {i}");
    }
    assert_valid(&exp);
    assert_eq!(exp, tracking_reveal(&r, 0.3, &p));
}

#[test]
fn tracking_reveal_glyph_stagger_is_small() {
    let letters: Vec<&str> = vec!["X"; 30];
    let r = run("tg", &[&letters], None);
    let exp = tracking_reveal(&r, 0.0, &params());
    let last = (0..30)
        .map(|i| first_start(&exp, &format!("tg.0.{i}")))
        .fold(0.0, f64::max);
    assert!(last <= 0.5 + 1e-9, "glyph stagger spread {last}");
}

#[test]
fn tracking_reveal_evaluated_states() {
    let r = glyph_run();
    let exp = tracking_reveal(&r, 0.0, &params());
    let p = project_for(&exp);
    let t_end = end_of(&exp) + 0.1;
    for i in 0..6 {
        assert_at_rest(&p, t_end, &format!("tr.0.{i}"), &r, 0, i);
    }
    // At t=0 the first cluster is displaced; sample a moment in.
    let t = first_start(&exp, "tr.0.0") + 0.15;
    let f = evaluate_frame(&p, frame_at(t)).expect("evaluates");
    if let Some(l) = layer_in(&f, "tr.0.0") {
        assert!(l.transform.e < measured(&r, 0, 0).0);
    }
}

// ---------------------------------------------------------------------------
// Generic properties over every behavior
// ---------------------------------------------------------------------------

#[test]
fn every_behavior_is_valid_under_every_preset_and_order() {
    let r = headline();
    let (old, new) = replace_runs();
    let s = sentence();
    let g = glyph_run();
    for preset in [
        MotionPreset::Calm,
        MotionPreset::Editorial,
        MotionPreset::Impact,
    ] {
        for order in [
            StaggerOrder::Forward,
            StaggerOrder::Reverse,
            StaggerOrder::CenterOut,
            StaggerOrder::EdgesIn,
        ] {
            let p = params_with(
                preset.params(),
                StaggerSpec {
                    preset: StaggerPreset::Calm,
                    order,
                },
            );
            let all = [
                word_cascade(&r, 0.1, &p),
                line_reveal(&r, 0.1, &p),
                text_mask_reveal(&r, 0.1, Direction::Right, &p),
                type_replace(&old, &new, 1.0, &p),
                type_scale_emphasis(&s, (0, 2), 0.1, 1.0, &p),
                keyword_punch(&s, (0, 2), 0.1, 1.0, ACCENT, &p),
                tracking_reveal(&g, 0.1, &p),
            ];
            for exp in &all {
                check_structure(exp);
                assert_valid(exp);
            }
        }
    }
}

#[test]
fn canvas_unit_scales_travel() {
    let r = headline();
    let mut p = params();
    p.u = 2.0;
    let exp = word_cascade(&r, 0.0, &p);
    let m = &on_channel(&exp, "hd.0.0", Channel::Offset)[0];
    assert!(matches!(m.op, MotionOp::Move { from, .. } if from[1] == 90.0 * 2.0 * 0.45));
}

// ---------------------------------------------------------------------------
// Golden scenes (generated by `write_golden_scenes`, checked by the other test)
// ---------------------------------------------------------------------------

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

/// Round to milliseconds, keeping sequential motions exactly abutting.
fn tidy(motions: &mut [Motion]) {
    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    for m in motions {
        let (s, e) = (r(m.start), r(m.start + m.duration));
        m.start = s;
        m.duration = r(e - s);
    }
}

/// Approximate advance width of `text` in the display face (Archivo Black-ish).
fn approx_width(text: &str, size: f32, em: f32, spacing_em: f32) -> f32 {
    let n = text.chars().count() as f32;
    n * em * size + (n - 1.0).max(0.0) * spacing_em * size
}

/// Hand-measured run for the goldens: like the compiler's `text_run`, unit x is
/// the width up to and including the unit minus the unit's width.
fn golden_run(id: &str, lines: &[&str], role: FontRole, size: f32, origin: [f32; 2]) -> TextRun {
    let (em, ls, lh, italic, weight) = match role {
        FontRole::SerifEmotional => (0.47, -0.01, 1.08, true, 400),
        _ => (0.78, -0.02, 0.98, false, 900),
    };
    let units = lines
        .iter()
        .map(|line| {
            let mut out = Vec::new();
            let mut end = 0;
            for word in line.split(' ') {
                let start = line[end..].find(word).map(|i| i + end).unwrap_or(end);
                end = start + word.len();
                let width = approx_width(word, size, em, ls);
                let upto = approx_width(&line[..end], size, em, ls);
                out.push(UnitBox {
                    text: word.to_string(),
                    x: (upto - width).max(0.0),
                    width,
                });
            }
            out
        })
        .collect();
    let mut st = style();
    st.font_role = role;
    st.font_size = size;
    st.font_weight = weight;
    st.italic = italic;
    st.letter_spacing = ls;
    st.line_height = lh;
    let h = size * lh;
    TextRun {
        id: id.to_string(),
        style: st,
        origin,
        line_advance: h,
        lines: units,
        ink: Some(InkBounds {
            top: h * 0.16,
            bottom: h * 0.90,
        }),
    }
}

fn golden_template(name: &str, duration: f64) -> Value {
    json!({
        "version": "0.2",
        "project": { "name": name, "duration_seconds": duration },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#ECE3D2" },
        "theme": {
            "fonts": {
                "display": "font.archivo_black",
                "display_condensed": "font.anton",
                "serif_emotional": "font.dm_serif_italic",
                "body": "font.fira_sans",
                "mono": "font.space_mono",
                "number": "font.anton"
            },
            "palette": {
                "paper": "#ECE3D2", "ink": "#16130F", "accent": "#D63A2B", "cobalt": "#1F3FBF"
            }
        },
        "assets": [
            { "id": "font.archivo_black", "type": "font", "path": "fonts/ArchivoBlack-Regular.ttf" },
            { "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" },
            { "id": "font.dm_serif_italic", "type": "font", "path": "fonts/DMSerifDisplay-Italic.ttf" },
            { "id": "font.fira_sans", "type": "font", "path": "fonts/FiraSans-Regular.ttf" },
            { "id": "font.space_mono", "type": "font", "path": "fonts/SpaceMono-Regular.ttf" }
        ],
        "asset_root": "../assets",
        "scenes": []
    })
}

fn solid(id: &str, rect: (f32, f32, f32, f32), color: Color) -> motion_core::scene::Layer {
    motion_core::scene::Layer {
        tilt: None,
        z: None,
        id: id.to_string(),
        x: rect.0,
        y: rect.1,
        width: rect.2,
        height: rect.3,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.0,
        anchor_y: 0.0,
        opacity: 1.0,
        z_index: 0,
        visible: true,
        clip: None,
        depth: None,
        layout: None,
        kind: LayerKind::Rectangle {
            fill: color,
            stroke: None,
        },
    }
}

fn build_kinetic_golden() -> MotionProject {
    let ink_c = Color::rgb(0x16, 0x13, 0x0F);
    let mut preset = MotionPreset::Editorial.params();
    preset.duration = 0.7;
    let p = params_with(
        preset,
        StaggerSpec {
            preset: StaggerPreset::Editorial,
            order: StaggerOrder::Forward,
        },
    );

    let headline = golden_run(
        "kt.head",
        &["ONE IDEA,", "ONE MOVE."],
        FontRole::Display,
        128.0,
        [90.0, 260.0],
    );
    let punch_run = golden_run(
        "kt.punch",
        &["MADE TO FEEL"],
        FontRole::Display,
        88.0,
        [90.0, 980.0],
    );
    let old = golden_run(
        "kt.old",
        &["STILL LIFE"],
        FontRole::Display,
        88.0,
        [90.0, 1420.0],
    );
    let new = golden_run(
        "kt.new",
        &["LIVE MOTION"],
        FontRole::Display,
        88.0,
        [90.0, 1420.0],
    );

    let mut exp = Expansion::default();
    exp.layers
        .push(solid("kt.rule.top", (90.0, 190.0, 900.0, 6.0), ink_c));
    exp.layers.push(solid(
        "kt.rule.mid",
        (90.0, 900.0, 300.0, 6.0),
        Color::rgb(0xD6, 0x3A, 0x2B),
    ));
    exp.extend(word_cascade(&headline, 0.2, &p));
    exp.extend(keyword_punch(&punch_run, (0, 2), 0.9, 1.5, ACCENT, &p));
    exp.extend(type_replace(&old, &new, 1.9, &p));
    tidy(&mut exp.motions);
    let duration = ((end_of(&exp) + 0.3) * 10.0).ceil() / 10.0;

    let mut proj = MotionProject::from_json(
        &golden_template("golden_kinetic_typography", duration).to_string(),
    )
    .expect("template parses");
    proj.scenes.push(motion_core::scene::Scene {
        post: Vec::new(),
        id: "kinetic".to_string(),
        start_seconds: 0.0,
        duration_seconds: duration,
        layers: exp.layers,
        motions: exp.motions,
        camera: None,
        lifecycle: None,
    });
    proj
}

fn build_stagger_golden() -> MotionProject {
    let ed = |order| StaggerSpec {
        preset: StaggerPreset::Editorial,
        order,
    };
    let paper = Color::rgb(0xEC, 0xE3, 0xD2);
    let fills = [
        Color::rgb(0x16, 0x13, 0x0F),
        Color::rgb(0xD6, 0x3A, 0x2B),
        Color::rgb(0x1F, 0x3F, 0xBF),
    ];
    let mut layers = Vec::new();
    let mut motions = Vec::new();

    let card = |id: &str, rect: (f32, f32, f32, f32), fill: Color, label: &str, size: f32| {
        let mut group = solid(id, rect, fill);
        group.kind = LayerKind::Group {
            children: vec![
                {
                    let mut bg = solid(&format!("{id}.bg"), (0.0, 0.0, rect.2, rect.3), fill);
                    bg.kind = LayerKind::RoundedRectangle {
                        fill,
                        radius: 18.0,
                        stroke: None,
                    };
                    bg
                },
                {
                    let mut t = solid(
                        &format!("{id}.label"),
                        (36.0, rect.3 * 0.5 - size * 0.7, rect.2 - 72.0, size * 1.4),
                        fill,
                    );
                    t.kind = LayerKind::Text(TextStyle {
                        text: label.to_string(),
                        font_role: FontRole::Mono,
                        font_size: size,
                        font_weight: 700,
                        italic: false,
                        color: paper,
                        align: TextAlign::Left,
                        line_height: 1.2,
                        letter_spacing: 0.08,
                        max_width: None,
                        uppercase: false,
                        ink: None,
                    });
                    t
                },
            ],
        };
        group
    };

    // Column: six cards entering from below, middle first (Editorial CenterOut).
    let col = stagger::offsets(ed(StaggerOrder::CenterOut), 6);
    let col_ranks = stagger::ranks(StaggerOrder::CenterOut, 6);
    for i in 0..6 {
        let id = format!("col.{i}");
        let y = 150.0 + i as f32 * 196.0;
        layers.push(card(
            &id,
            (100.0, y, 880.0, 168.0),
            fills[i % 3],
            &format!("ITEM {:02}", i + 1),
            44.0,
        ));
        let t0 = 0.15 + col[i];
        let dur = 0.72 * (1.0 + 0.05 * (col_ranks[i] as f64 % 3.0 - 1.0));
        motions.push(Motion {
            spring: None,
            id: None,
            target: id.clone(),
            start: t0,
            duration: dur * 0.6,
            easing: Easing::OutCubic,
            op: MotionOp::Fade { from: 0.0, to: 1.0 },
        });
        motions.push(Motion {
            spring: None,
            id: None,
            target: id,
            start: t0,
            duration: dur,
            easing: Easing::OutQuint,
            op: MotionOp::Move {
                from: [0.0, 70.0],
                to: [0.0, 0.0],
            },
        });
    }

    // Row: four tiles sliding in from the right, last one first (Reverse).
    let row = stagger::offsets(ed(StaggerOrder::Reverse), 4);
    for i in 0..4 {
        let id = format!("row.{i}");
        let x = 100.0 + i as f32 * 226.0;
        layers.push(card(
            &id,
            (x, 1420.0, 206.0, 260.0),
            fills[(i + 1) % 3],
            &format!("{:02}", i + 1),
            40.0,
        ));
        let t0 = 1.0 + row[i];
        motions.push(Motion {
            spring: None,
            id: None,
            target: id.clone(),
            start: t0,
            duration: 0.42,
            easing: Easing::OutCubic,
            op: MotionOp::Fade { from: 0.0, to: 1.0 },
        });
        motions.push(Motion {
            spring: None,
            id: None,
            target: id,
            start: t0,
            duration: 0.7,
            easing: Easing::OutQuint,
            op: MotionOp::Move {
                from: [90.0, 0.0],
                to: [0.0, 0.0],
            },
        });
    }
    tidy(&mut motions);
    let end = motions
        .iter()
        .map(|m| m.start + m.duration)
        .fold(0.0, f64::max);
    let duration = ((end + 0.3) * 10.0).ceil() / 10.0;

    let mut proj =
        MotionProject::from_json(&golden_template("golden_stagger", duration).to_string())
            .expect("template parses");
    proj.scenes.push(motion_core::scene::Scene {
        post: Vec::new(),
        id: "stagger".to_string(),
        start_seconds: 0.0,
        duration_seconds: duration,
        layers,
        motions,
        camera: None,
        lifecycle: None,
    });
    proj
}

/// Regenerate the golden files:
/// `cargo test -p motion-core --test kinetic write_golden_scenes -- --ignored`
#[test]
#[ignore = "writes golden/*.motion.json"]
fn write_golden_scenes() {
    for (file, proj) in [
        ("kinetic_typography.motion.json", build_kinetic_golden()),
        ("stagger.motion.json", build_stagger_golden()),
    ] {
        let path = golden_dir().join(file);
        std::fs::write(&path, proj.to_json_pretty() + "\n").expect("write golden");
    }
}

#[test]
fn golden_scenes_parse_and_validate() {
    for file in ["kinetic_typography.motion.json", "stagger.motion.json"] {
        let path = golden_dir().join(file);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: {e}"));
        let project = MotionProject::from_json(&text).unwrap_or_else(|e| panic!("{file}: {e}"));
        assert_eq!(
            (
                project.canvas.width,
                project.canvas.height,
                project.canvas.fps
            ),
            (1080, 1920, 30)
        );
        validate(&project, Some(&golden_dir())).unwrap_or_else(|e| panic!("{file}:\n{e}"));
        // Something is on screen mid-way.
        let mid = project.frame_count() / 2;
        let f = evaluate_frame(&project, mid).expect("evaluates");
        assert!(!f.layers.is_empty(), "{file}: empty mid frame");
    }
}

#[test]
fn golden_builders_match_checked_in_files() {
    // The files are the builders' output: catches hand edits and drift.
    for (file, proj) in [
        ("kinetic_typography.motion.json", build_kinetic_golden()),
        ("stagger.motion.json", build_stagger_golden()),
    ] {
        let text = std::fs::read_to_string(golden_dir().join(file)).expect("golden exists");
        let on_disk = MotionProject::from_json(&text).expect("parses");
        assert_eq!(on_disk, proj, "{file} is stale; rerun write_golden_scenes");
    }
}
