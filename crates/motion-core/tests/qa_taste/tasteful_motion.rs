//! (0.24 M10) Tasteful motion: nothing flickers or pulsates, the camera stays
//! within 3 %, the payoff colour field reaches every canvas edge, and the
//! per-look motion signature is data with the right character.

use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{look_defaults, ArtMode, Look};
use motion_core::compiler::signature::{self, MotionSignature};
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{CameraOp, Layer, LayerKind, MotionOp, MotionProject, SpringSpec};
use motion_core::slop_qa::{slop_report, SlopCheck, SlopRules};
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord};
use motion_core::style::StyleProfile;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

const STORIES: [(&str, &str); 3] = [
    (
        "sleep_review",
        "examples/bench/stories/sleep_review.intent.json",
    ),
    (
        "money_review",
        "examples/bench/stories/money_review.intent.json",
    ),
    ("ai_learns", "examples/topics/ai_learns.intent.json"),
];

/// Three punchy beats the hype slam grammar sets as units.
const SLAM_INTENT: &str = r#"{
  "version": "0.2", "title": "slam_field", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Cash is king",
     "primary": {"kind": "phrase", "value": "$381 billion", "meaning": "cash reserves"},
     "energy": "impact", "keyword": "cash"},
    {"purpose": "emphasize", "statement": "Robots keep selling for 36 months",
     "primary": {"kind": "number", "value": "36 months", "meaning": "selling"},
     "energy": "building", "keyword": "selling"},
    {"purpose": "emphasize", "statement": "Thirteen million dollars",
     "primary": {"kind": "number", "value": "$13M", "meaning": "a year"},
     "energy": "impact", "keyword": "million"},
    {"purpose": "emphasize", "statement": "Still waiting for the crash",
     "primary": {"kind": "phrase", "value": "still waiting"},
     "energy": "impact", "keyword": "waiting"}
  ]
}"#;

/// A voice-over that says each beat's statement, words 0.32 s apart.
fn slam_speech(intent: &CreativeIntent) -> SpeechMap {
    let (mut words, mut sentences) = (Vec::new(), Vec::new());
    let mut t = 0.3;
    for (i, beat) in intent.beats.iter().enumerate() {
        let start = t;
        for w in motion_core::speech::statement_tokens(&beat.statement) {
            words.push(SpeechWord {
                text: w,
                start: (t * 1000.0_f64).round() / 1000.0,
                end: ((t + 0.27) * 1000.0_f64).round() / 1000.0,
                confidence: 1.0,
            });
            t += 0.32;
        }
        sentences.push(SpeechSentence {
            beat: i,
            start,
            end: t,
        });
        t += 0.5;
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: t,
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(ROOT).join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn compile(story: usize, look: Look, seed: u64) -> MotionProject {
    let (name, path) = STORIES[story];
    let intent = CreativeIntent::from_json(&read(path)).expect("intent");
    let map = SpeechMap::from_json(&read(&format!(
        "golden/fixtures/variety/{name}.speech.json"
    )))
    .expect("speech fixture");
    let spoken: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    let opts = CompileOptions {
        art: Some(ArtMode::Force(look)),
        variety: Some(seed),
        speech: Some(repair(&map, &spoken).0),
        ..CompileOptions::default()
    };
    let style: StyleProfile = serde_json::from_str("{\"tone\": \"hype\"}").expect("style");
    compile_with_options(
        &intent,
        &style,
        None,
        &AssetLibrary::new(Path::new(ROOT).join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap_or_else(|e| panic!("{name} x {look:?}: {e}"))
}

fn walk<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            walk(children, out);
        }
    }
}

#[test]
fn no_look_pulses_glitches_echoes_or_shakes() {
    for look in Look::ALL {
        let fx = look_defaults(look).fx;
        assert_eq!(fx.pulse_gain, 0.0, "{look:?} pulses");
        assert!(!fx.glitch_cuts, "{look:?} glitches");
        assert!(!fx.echo_entrances, "{look:?} echoes");
        assert_eq!(fx.shake_trauma, 0.0, "{look:?} shakes");
        assert!(
            !fx.bloom || look == Look::Cinematic3d,
            "{look:?} blooms (only Cinematic3d may)"
        );
    }
}

#[test]
fn compiled_projects_have_no_flicker_ops_and_pass_slop_flicker() {
    let rules = SlopRules::bundled();
    for look in Look::ALL {
        for story in 0..STORIES.len() {
            let project = compile(story, look, 3);
            for s in &project.scenes {
                for m in &s.motions {
                    assert!(
                        !matches!(m.op, MotionOp::Pulse { .. } | MotionOp::Echo { .. }),
                        "{look:?} {}: {} on {}",
                        s.id,
                        m.op.op_name(),
                        m.target
                    );
                }
            }
            let report = slop_report(&project, &rules);
            let flicker: Vec<_> = report
                .findings
                .iter()
                .filter(|f| f.check == SlopCheck::Flicker)
                .collect();
            assert!(flicker.is_empty(), "{look:?} story {story}: {flicker:?}");
        }
    }
}

#[test]
fn slop_flicker_flags_a_pulse_an_echo_and_a_second_shake() {
    let mut project = compile(1, Look::StudioPop, 3);
    let rules = SlopRules::bundled();
    let beat = project
        .scenes
        .iter_mut()
        .find(|s| s.id.starts_with("beat_"))
        .expect("a beat");
    let target = beat.layers[0].id.clone();
    beat.motions.push(motion_core::scene::Motion {
        id: None,
        target: target.clone(),
        start: 0.0,
        duration: 0.5,
        easing: motion_core::Easing::Linear,
        spring: None,
        op: MotionOp::Echo {
            count: 2,
            spacing: 0.04,
            decay: 0.5,
        },
    });
    let report = slop_report(&project, &rules);
    assert!(report
        .findings
        .iter()
        .any(|f| f.check == SlopCheck::Flicker));
}

#[test]
fn the_camera_and_the_stage_stay_within_three_percent() {
    for look in [Look::HypeSlam, Look::Dossier, Look::StudioPop] {
        for story in 0..STORIES.len() {
            let project = compile(story, look, 5);
            for s in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
                if let Some(cam) = &s.camera {
                    for m in &cam.motions {
                        if let CameraOp::Push { from, to } = m.op {
                            assert!(
                                from.max(to) <= 1.0301,
                                "{look:?} {}: camera push {from} -> {to}",
                                s.id
                            );
                        }
                    }
                }
                let stage = format!("{}.stage", s.id.replace("beat_", "b"));
                for m in s.motions.iter().filter(|m| m.target == stage) {
                    if let MotionOp::Scale { from, to, .. } = m.op {
                        assert!(
                            from.max(to) <= 1.0301,
                            "{look:?} {}: stage scale {from} -> {to}",
                            s.id
                        );
                    }
                }
            }
        }
    }
}

/// The payoff colour field is full-bleed: it spans the whole canvas, past the
/// caption lane the beat content is laid out above (the strip under the $13M
/// field).
#[test]
fn the_slam_colour_field_reaches_every_canvas_edge() {
    let intent = CreativeIntent::from_json(SLAM_INTENT).expect("intent");
    let speech = slam_speech(&intent);
    let style: StyleProfile = serde_json::from_str("{\"tone\": \"hype\"}").expect("style");
    let mut seen = 0;
    for seed in 0..12u64 {
        let opts = CompileOptions {
            art: Some(ArtMode::Force(Look::HypeSlam)),
            variety: Some(seed),
            speech: Some(speech.clone()),
            ..CompileOptions::default()
        };
        let project = compile_with_options(
            &intent,
            &style,
            None,
            &AssetLibrary::new(Path::new(ROOT).join("assets")),
            &ApproxMeasure,
            &AssetManifest::empty(),
            None,
            &opts,
        )
        .expect("compiles");
        let (w, h) = (project.canvas.width as f32, project.canvas.height as f32);
        for s in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
            let mut all = Vec::new();
            walk(&s.layers, &mut all);
            for l in all.iter().filter(|l| l.id.ends_with(".field")) {
                seen += 1;
                // Groups sit at the origin: the layer's own box is canvas space.
                assert!(
                    l.x <= 0.5
                        && l.y <= 0.5
                        && l.x + l.width >= w - 0.5
                        && l.y + l.height >= h - 0.5,
                    "seed {seed} {} {}: field ({}, {}, {} x {}) on {w} x {h}",
                    s.id,
                    l.id,
                    l.x,
                    l.y,
                    l.width,
                    l.height
                );
            }
        }
    }
    assert!(seen > 0, "no slam field was compiled; the test is vacuous");
}

fn overshoot(s: SpringSpec) -> f32 {
    let zeta = s.damping / (2.0 * (s.stiffness * s.mass).sqrt());
    if zeta >= 1.0 {
        0.0
    } else {
        (-std::f32::consts::PI * zeta / (1.0 - zeta * zeta).sqrt()).exp()
    }
}

#[test]
fn signatures_differ_in_character_not_in_effects() {
    let hype = signature::for_look(Some(Look::HypeSlam));
    let doc = signature::for_look(Some(Look::Dossier));
    let cine = signature::for_look(Some(Look::Cinematic3d));
    assert_eq!(signature::for_look(None), MotionSignature::BASE);
    // Hype: bigger slides, snappier; documentary: slower, smaller; cinematic: longest.
    assert!(hype.slide_px > doc.slide_px);
    assert!(hype.hero_secs < doc.hero_secs && doc.hero_secs < cine.hero_secs);
    assert!(hype.slide_secs < doc.slide_secs && doc.slide_secs < cine.slide_secs);
    for look in Look::ALL {
        let s = signature::for_look(Some(look));
        assert!(overshoot(s.spring) <= 0.02, "{look:?} overshoots");
        assert!((0.9..1.0).contains(&s.hero_from), "{look:?}");
        assert!((24.0..=40.0).contains(&s.slide_px), "{look:?}");
        assert!(s.camera_push_max <= 0.03, "{look:?}");
        assert_eq!(s.exit_drift_px, 12.0);
        assert_eq!(s.exit_secs.min(0.3), s.exit_secs.min(0.3));
        assert!(s.support_lag >= 0.06 && s.support_lag <= 0.12);
    }
    // Pictures arrive from alternating sides.
    assert_ne!(signature::slide_side(0), signature::slide_side(1));
}
