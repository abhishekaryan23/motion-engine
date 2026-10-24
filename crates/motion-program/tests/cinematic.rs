//! Cinematic components and finishes: 2.5D perspective camera, Bento grids, the closing
//! lockup (rules from a curve, sheen), the impact transition and the film
//! finish.

use motion_core::scene::{LayerKind, MotionOp, PostKind};
use motion_program::program::MotionProgram;
use motion_program::{lower, validate, Typesetter};

fn program(components: &str, extra: &str) -> MotionProgram {
    let json = format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 6}},
        "palette": {{"background": "#16171A", "ink": "#E6E3DD", "accent": "#3FD6E4", "muted": "#6E727B"}}
        {extra}, "components": [{components}]}}"##
    );
    MotionProgram::from_json(&json).expect("parses")
}

const LOCKUP: &str = r#"{"id": "close", "kind": "lockup", "start": 1.0, "headline": "Origin",
    "subline": "Every form begins at zero.",
    "rules_from": [[0.0, 1.0], [0.4, 1.0], [0.6, 0.0], [1.0, 0.0]],
    "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}, {"at": 0.7, "do": "sheen"}]}"#;

#[test]
fn lockup_rules_spring_straight_from_the_curve() {
    let p = program(
        &format!(r#"{{"id": "g", "kind": "backdrop"}}, {LOCKUP}"#),
        "",
    );
    validate(&p).expect("valid");
    let s = lower(&p, None, &Typesetter::approx());
    let sc = s
        .scenes
        .iter()
        .find(|s| s.id == "close")
        .expect("lockup scene");
    let rules: Vec<_> = sc
        .layers
        .iter()
        .filter(|l| l.id.starts_with("close.rule"))
        .collect();
    assert_eq!(
        rules.len(),
        3,
        "rule above, on the baseline, under the line"
    );
    for r in &rules {
        let LayerKind::Path { d, .. } = &r.kind else {
            panic!("rules are paths")
        };
        // Starts bent (the curve spans the frame height), ends flat.
        let start = motion_core::path::parse(d).unwrap();
        let ys: Vec<f32> = start.contours[0]
            .segs
            .iter()
            .map(|g| g.to[1])
            .chain([start.contours[0].start[1]])
            .collect();
        let span = ys.iter().cloned().fold(f32::MIN, f32::max)
            - ys.iter().cloned().fold(f32::MAX, f32::min);
        assert!(span > 500.0, "{} starts on the curve (span {span})", r.id);
        let to = sc
            .motions
            .iter()
            .find_map(|m| match (&m.op, m.target == r.id) {
                (MotionOp::Morph { to }, true) => Some(to.clone()),
                _ => None,
            })
            .expect("rule morphs");
        let end = motion_core::path::parse(&to).unwrap();
        let c = &end.contours[0];
        assert!(c.segs.iter().all(|g| (g.to[1] - c.start[1]).abs() < 1e-3));
        // A visible overshoot: the morph is a spring.
        assert!(sc
            .motions
            .iter()
            .any(|m| m.target == r.id && m.spring.is_some()));
    }
    // Rules are stacked: top above the headline, the last under the line.
    let head = sc
        .layers
        .iter()
        .find(|l| l.id == "close.headline")
        .expect("headline");
    assert!(rules[0].y < head.y && rules[2].y > head.y);
    let LayerKind::Text(t) = &head.kind else {
        panic!()
    };
    assert!(t.uppercase);
}

#[test]
fn sheen_sweeps_masked_highlights_across_the_headline_once() {
    let p = program(LOCKUP, "");
    let s = lower(&p, None, &Typesetter::approx());
    let sc = s.scenes.iter().find(|s| s.id == "close").unwrap();
    let copies: Vec<_> = sc
        .layers
        .iter()
        .filter(|l| l.id.starts_with("close.sheen"))
        .collect();
    assert_eq!(copies.len(), 2, "a core and a halo");
    for c in &copies {
        let mask = c.mask.as_ref().expect("masked");
        let from = motion_core::path::parse(&mask.d).unwrap();
        let to = sc
            .motions
            .iter()
            .find_map(|m| match (&m.op, m.target == c.id) {
                (MotionOp::MaskMorph { to }, true) => Some((m.start, to.clone())),
                _ => None,
            })
            .expect("mask travels");
        assert!((to.0 - 0.7).abs() < 1e-6, "at the sheen");
        let to = motion_core::path::parse(&to.1).unwrap();
        // Starts left of the type, ends right of it.
        assert!(from.contours[0].start[0] < 0.0);
        assert!(to.contours[0].start[0] > c.width);
    }
    // The highlight is the accent, not a white flash.
    let LayerKind::Text(t) = &copies[0].kind else {
        panic!()
    };
    assert_ne!((t.color.r, t.color.g, t.color.b), (255, 255, 255));
}

#[test]
fn lockup_validation() {
    let long = program(
        r#"{"id": "c", "kind": "lockup", "headline": "A headline that is far too long to set"}"#,
        "",
    );
    assert!(validate(&long)
        .unwrap_err()
        .to_string()
        .contains("headline"));
    let sheen_on_title = program(
        r#"{"id": "t", "kind": "title", "text": "Hi", "timeline": [{"at": 0.5, "do": "sheen"}]}"#,
        "",
    );
    assert!(validate(&sheen_on_title)
        .unwrap_err()
        .to_string()
        .contains("sheen is for lockups"));
}

#[test]
fn bento_grid_validates_and_lowers() {
    let bento_json = r#"{
        "id": "features",
        "kind": "bento",
        "start": 0.5,
        "end": 4.0,
        "tiles": [
            {"stat": "99.9%", "title": "Uptime guarantee", "accent": true},
            {"stat": "<10ms", "title": "Ultra-low latency", "detail": "GLOBAL EDGE"},
            {"stat": "ZERO", "title": "Trust architecture", "detail": "END-TO-END"}
        ],
        "timeline": [
            {"at": 0.0, "do": "enter", "style": "pop"},
            {"at": 1.0, "do": "tilt", "from": [0.0, 15.0], "to": [0.0, 0.0]}
        ]
    }"#;
    let p = program(bento_json, "");
    validate(&p).expect("bento should validate cleanly");
    let s = lower(&p, None, &Typesetter::approx());
    motion_core::validate(&s, None).expect("lowered bento scene must validate in motion_core");
    let sc = s
        .scenes
        .iter()
        .find(|sc| sc.id == "features")
        .expect("bento scene");
    // Should have 3 tile group layers
    let tiles: Vec<_> = sc
        .layers
        .iter()
        .filter(|l| l.id.starts_with("features.tile"))
        .collect();
    assert_eq!(tiles.len(), 3);
    // Should have numeric count up motion for 99.9%
    let has_count = sc
        .motions
        .iter()
        .any(|m| matches!(m.op, MotionOp::Count { .. }));
    assert!(has_count, "tile with numeric stat should have Count motion");
    // Should have camera perspective since Bento uses 2.5D tilt
    assert!(sc.camera.as_ref().and_then(|c| c.perspective).is_some());
}

#[test]
fn perspective_camera_and_25d_transforms_lower() {
    let comp = r#"{
        "id": "headline",
        "kind": "title",
        "text": "Deep Reality",
        "z": -150.0,
        "depth": 1.5,
        "tilt": [10.0, -5.0],
        "timeline": [
            {"at": 0.0, "do": "enter", "style": "fade"},
            {"at": 0.5, "do": "tilt", "from": [10.0, -5.0], "to": [0.0, 0.0]},
            {"at": 1.0, "do": "echo", "count": 3}
        ]
    }"#;
    let camera_events = r#", "camera": [
        {"at": 0.0, "do": "orbit", "yaw": 12.0, "pitch": -8.0},
        {"at": 1.0, "do": "dolly", "to": 200.0},
        {"at": 1.5, "do": "roll", "deg": 4.0},
        {"at": 2.0, "do": "focus", "z": -150.0, "aperture": 0.5}
    ]"#;
    let p = program(comp, camera_events);
    validate(&p).expect("2.5D program validates");
    let s = lower(&p, None, &Typesetter::approx());
    motion_core::validate(&s, None).expect("lowered 2.5D project validates in motion-core");
    let sc = s
        .scenes
        .iter()
        .find(|sc| sc.id == "headline")
        .expect("headline scene");
    let cam = sc.camera.as_ref().expect("scene camera must be present");
    assert!(cam.perspective.is_some(), "perspective camera attached");
    assert!(cam
        .motions
        .iter()
        .any(|m| matches!(m.op, motion_core::scene::CameraOp::Orbit { .. })));
    assert!(cam
        .motions
        .iter()
        .any(|m| matches!(m.op, motion_core::scene::CameraOp::Dolly { .. })));
    assert!(cam
        .motions
        .iter()
        .any(|m| matches!(m.op, motion_core::scene::CameraOp::Roll { .. })));
    assert!(cam
        .motions
        .iter()
        .any(|m| matches!(m.op, motion_core::scene::CameraOp::Focus { .. })));
    // Layer should have depth, z, and tilt lowered
    let text_layer = sc
        .layers
        .iter()
        .find(|l| l.id == "headline")
        .expect("headline layer");
    assert_eq!(text_layer.depth, Some(1.5));
    assert_eq!(text_layer.z, Some(-150.0));
    assert_eq!(text_layer.tilt, Some([10.0, -5.0]));
    // Motions should contain Tilt and Echo
    assert!(sc
        .motions
        .iter()
        .any(|m| matches!(m.op, MotionOp::Tilt { .. })));
    assert!(sc
        .motions
        .iter()
        .any(|m| matches!(m.op, MotionOp::Echo { count: 3, .. })));
}

#[test]
fn impact_shocks_blurs_and_shakes_for_a_moment() {
    let p = program(
        LOCKUP,
        r#", "transitions": [{"at": 1.0, "kind": "impact"}]"#,
    );
    validate(&p).expect("valid");
    let s = lower(&p, None, &Typesetter::approx());
    let fx = s
        .scenes
        .iter()
        .find(|s| s.id.ends_with(".impact"))
        .expect("impact scene");
    let ring = fx
        .post
        .iter()
        .find(|e| matches!(e.kind, PostKind::Shockwave { .. }))
        .expect("shockwave");
    assert!(ring.duration <= 2.0 / 30.0 + 1e-6, "two frames");
    assert!(fx
        .post
        .iter()
        .any(|e| matches!(e.kind, PostKind::DirectionalBlur { .. }) && e.duration <= 0.1 + 1e-6));
    // The lockup's camera shakes after the impact and comes back to rest.
    let sc = s.scenes.iter().find(|s| s.id == "close").unwrap();
    let cam = sc.camera.as_ref().expect("shake on the camera");
    let tracks: Vec<[f32; 2]> = cam
        .motions
        .iter()
        .filter_map(|m| match m.op {
            motion_core::scene::CameraOp::Track { to, .. } => Some(to),
            _ => None,
        })
        .collect();
    assert!(tracks.iter().any(|t| t[0].abs() + t[1].abs() > 1.0));
    assert_eq!(*tracks.last().unwrap(), [0.0, 0.0]);
    motion_core::validate(&s, None).expect("valid scene");
}

#[test]
fn film_finish_runs_last_over_the_whole_program() {
    let p = program(
        r#"{"id": "g", "kind": "backdrop"}"#,
        r#", "film": {"grain": 0.035, "fringe": 0.8}"#,
    );
    let s = lower(&p, None, &Typesetter::approx());
    let last = s.scenes.last().unwrap();
    assert_eq!(last.id, "film");
    assert!(last.layers.is_empty());
    assert!((last.duration_seconds - 6.0).abs() < 1e-9);
    assert!(last
        .post
        .iter()
        .any(|e| matches!(e.kind, PostKind::Grain { .. })));
    assert!(last
        .post
        .iter()
        .any(|e| matches!(e.kind, PostKind::ChromaticAberration { .. })));
}

#[test]
fn portrait_lockup_qa_and_scaling() {
    let json = format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 4, "aspect": "9:16"}},
        "palette": {{"background": "#16171A", "ink": "#E6E3DD", "accent": "#3FD6E4", "muted": "#6E727B"}},
        "components": [{LOCKUP}]}}"##
    );
    let p = MotionProgram::from_json(&json).expect("parses");
    validate(&p).expect("valid");
    let s = lower(&p, None, &Typesetter::approx());
    motion_core::validate(&s, None).expect("motion-core valid");
    let (share, bad) = motion_program::qa::fill(&p, &s, &[1.5, 2.5]);
    assert_eq!(share, 1.0, "portrait lockup fill must pass: {bad:?}");
    assert!(bad.is_empty());
}

#[test]
fn all_four_lockup_presets_validate_and_lower() {
    for preset in [
        "spline_frame",
        "monolith_split",
        "orbital_seal",
        "terminal_prompt",
    ] {
        let lockup_json = format!(
            r#"{{"id": "close", "kind": "lockup", "preset": "{preset}", "start": 0.0, "headline": "NEXUS",
            "subline": "The intelligent compute fabric.",
            "specs": ["LATENCY: 0.4MS", "THROUGHPUT: 100GBPS"],
            "timeline": [{{"at": 0.0, "do": "enter", "style": "pop"}}, {{"at": 0.5, "do": "sheen"}}]}}"#
        );
        let p = program(&lockup_json, "");
        validate(&p).unwrap_or_else(|e| panic!("preset {preset} must validate: {e}"));
        let s = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&s, None)
            .unwrap_or_else(|e| panic!("preset {preset} must lower to valid scene: {e}"));
        let sc = s
            .scenes
            .iter()
            .find(|s| s.id == "close")
            .expect("close scene");
        assert!(!sc.layers.is_empty(), "preset {preset} must produce layers");
    }
}

#[test]
fn all_three_bento_presets_validate_and_lower() {
    for preset in ["grid2x2", "hero_left", "pillars"] {
        let bento_json = format!(
            r#"{{"id": "bento", "kind": "bento", "preset": "{preset}", "start": 0.0,
            "tiles": [
                {{"stat": "10X", "title": "Speedup", "accent": true}},
                {{"stat": "0.1MS", "title": "Latency", "detail": "GLOBAL EDGE"}},
                {{"stat": "99.9%", "title": "Uptime", "detail": "ENTERPRISE SLA"}}
            ],
            "timeline": [{{"at": 0.0, "do": "enter", "style": "pop"}}]}}"#
        );
        let p = program(&bento_json, "");
        validate(&p).unwrap_or_else(|e| panic!("bento preset {preset} must validate: {e}"));
        let s = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&s, None)
            .unwrap_or_else(|e| panic!("bento preset {preset} must lower to valid scene: {e}"));
    }
}

#[test]
fn split_screen_side_by_side_passes_overlap_qa() {
    let split_json = r#"
        {"id": "code", "kind": "code_block", "slot": "left", "start": 0.0, "lines": ["fn main() {", "    println!(\"hi\");", "}"]},
        {"id": "metrics", "kind": "bento", "preset": "hero_left", "slot": "right", "start": 0.0,
         "tiles": [
             {"stat": "100%", "title": "Coverage", "accent": true},
             {"stat": "0 SEC", "title": "Downtime"}
         ]}
    "#;
    let p = program(split_json, "");
    validate(&p).expect("split screen program must validate");
    let s = lower(&p, None, &Typesetter::approx());
    motion_core::validate(&s, None).expect("split screen project must validate in motion-core");
    let overlaps = motion_program::qa::component_overlap(&p, &s);
    assert!(
        overlaps.is_empty(),
        "complementary slots left & right must have zero overlap: {overlaps:?}"
    );
}
