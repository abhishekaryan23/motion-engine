//! (0.24 M10) The slam colour field is full-bleed: on every frame where it is
//! on screen, all four canvas corners show the field colour (the $13M field
//! used to stop above the caption lane and show the ground under it).

use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{compile_with_options, AssetLibrary, CompileOptions, FontSet};
use motion_core::scene::{Color, Layer, LayerKind, MotionOp};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
use motion_core::{evaluate_frame, CreativeIntent, StyleProfile};
use motion_render::{CpuRenderer, FontMeasure, Renderer};

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

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn walk<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            walk(children, out);
        }
    }
}

fn near(a: [u8; 4], c: Color) -> bool {
    let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).abs();
    d(a[0], c.r) < 48 && d(a[1], c.g) < 48 && d(a[2], c.b) < 48
}

#[test]
fn the_field_covers_all_four_corners_while_it_is_on_screen() {
    let assets = root().join("assets");
    let mut checked = 0;
    let name = "slam_field";
    {
        let intent = CreativeIntent::from_json(SLAM_INTENT).expect("intent");
        let map = slam_speech(&intent);
        let style: StyleProfile = serde_json::from_str("{\"tone\": \"hype\"}").expect("style");
        let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
            .faces
            .iter()
            .map(|f| assets.join(f.path))
            .collect();
        let measure =
            FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("fonts");
        for seed in 0..8u64 {
            let opts = CompileOptions {
                art: Some(ArtMode::Force(Look::HypeSlam)),
                variety: Some(seed),
                speech: Some(map.clone()),
                ..CompileOptions::default()
            };
            let project = compile_with_options(
                &intent,
                &style,
                None,
                &AssetLibrary::new(&assets),
                &measure,
                &AssetManifest::empty(),
                None,
                &opts,
            )
            .expect("compiles");
            let renderer = CpuRenderer::new(&project, &assets).expect("renderer");
            let fps = f64::from(project.canvas.fps);
            let (w, h) = (project.canvas.width, project.canvas.height);
            for scene in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
                let mut all = Vec::new();
                walk(&scene.layers, &mut all);
                for field in all.iter().filter(|l| l.id.ends_with(".field")) {
                    let LayerKind::Rectangle { fill, .. } = field.kind else {
                        continue;
                    };
                    // The unit group is `<field id minus .field>`: visible from
                    // its 0 -> 1 step to its 1 -> 0 step (or the beat's end).
                    let group = field.id.trim_end_matches(".field");
                    let (mut on, mut off) = (None, scene.duration_seconds);
                    for m in scene.motions.iter().filter(|m| m.target == group) {
                        if let MotionOp::Fade { from, to } = m.op {
                            if from < to {
                                on = Some(m.start);
                            } else {
                                off = m.start;
                            }
                        }
                    }
                    let Some(on) = on else { continue };
                    if off - on < 0.6 {
                        continue;
                    }
                    // Late enough for the word to have landed, early enough to
                    // be before the stage's pull-back.
                    let t = scene.start_seconds + on + 0.4;
                    let frame = (t * fps).round() as u32;
                    let resolved = evaluate_frame(&project, frame).expect("frame");
                    let pm = renderer.render(&resolved).expect("render");
                    let px = |x: u32, y: u32| {
                        let p = pm.pixel(x, y).expect("pixel");
                        [p.red(), p.green(), p.blue(), p.alpha()]
                    };
                    for (x, y) in [(2, 2), (w - 3, 2), (2, h - 3), (w - 3, h - 3)] {
                        assert!(
                            near(px(x, y), fill),
                            "{name} seed {seed} {} frame {frame}: corner ({x}, {y}) is {:?}, field {fill:?}",
                            field.id,
                            px(x, y)
                        );
                    }
                    checked += 1;
                }
            }
        }
    }
    assert!(
        checked > 0,
        "no slam field was rendered; the test is vacuous"
    );
}
