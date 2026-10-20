//! (0.19) The cinematic look's pictures arrive from different directions, do
//! not throb to the music, and the supporting picture is readable.
//!
//! Compiled from the in-repo `examples/cinematic/space` story (four beats, each
//! a hero object with a supporting object) with a music envelope, so "no pulse"
//! cannot pass merely because there was no music.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Envelope, Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn space() -> MotionProject {
    let read = |ext: &str| {
        std::fs::read_to_string(repo().join(format!("examples/cinematic/space.{ext}.json")))
            .unwrap_or_else(|e| panic!("space.{ext}.json: {e}"))
    };
    let intent = CreativeIntent::from_json(&read("intent")).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read("style")).expect("style");
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        music_envelope: Some(Envelope {
            id: "music".to_string(),
            fps: 30.0,
            values: (0..2400).map(|i| (i % 7) as f32 / 6.0).collect(),
        }),
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn beats(p: &MotionProject) -> Vec<&Scene> {
    p.scenes
        .iter()
        .filter(|s| s.id.starts_with("beat_"))
        .collect()
}

fn on<'a>(s: &'a Scene, target: &str) -> Vec<&'a Motion> {
    s.motions.iter().filter(|m| m.target == target).collect()
}

fn find<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    layers.iter().find_map(|l| {
        if l.id == id {
            return Some(l);
        }
        match &l.kind {
            LayerKind::Group { children } => find(children, id),
            _ => None,
        }
    })
}

/// The beats whose hero is a picture (a number or phrase hero is big type,
/// which has no card to swing in): `(beat number, scene)`.
fn picture_beats(p: &MotionProject) -> Vec<(usize, &Scene)> {
    beats(p)
        .into_iter()
        .enumerate()
        .filter(|(i, s)| {
            on(s, &format!("b{}.hero.depth", i + 1))
                .iter()
                .any(|m| matches!(m.op, MotionOp::Tilt { .. }))
        })
        .map(|(i, s)| (i + 1, s))
        .collect()
}

/// `(slide offset, tilt)` at the start of the arrival of the plane `id`.
fn arrival(s: &Scene, id: &str) -> ([f32; 2], [f32; 2]) {
    let ms = on(s, id);
    let slide = ms
        .iter()
        .find_map(|m| match m.op {
            MotionOp::Move { from, .. } => Some(from),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{id}: no arrival slide"));
    let turn = ms
        .iter()
        .find_map(|m| match m.op {
            MotionOp::Tilt { from, .. } => Some(from),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{id}: no arrival tilt"));
    (slide, turn)
}

#[test]
fn the_cinematic_look_compiles_valid_scenes_with_arrivals() {
    let p = space();
    validate(&p, None).expect("valid");
    assert_eq!(beats(&p).len(), 4);
    assert!(
        !p.envelopes.is_empty(),
        "the music envelope is in the project"
    );
}

#[test]
fn heroes_do_not_pulse_to_the_music() {
    let p = space();
    let pulses: Vec<String> = p
        .scenes
        .iter()
        .flat_map(|s| s.motions.iter())
        .filter(|m| matches!(m.op, MotionOp::Pulse { .. }))
        .map(|m| m.target.clone())
        .collect();
    assert!(pulses.is_empty(), "cinematic pulses: {pulses:?}");
}

#[test]
fn every_picture_arrives_from_a_different_side_than_the_one_before() {
    let p = space();
    let pics = picture_beats(&p);
    assert!(pics.len() >= 3, "picture beats: {}", pics.len());
    let mut seen = Vec::new();
    for (n, s) in &pics {
        let (slide, turn) = arrival(s, &format!("b{n}.hero.depth"));
        // The slide and the turn agree on the side, and the leading edge turns away.
        if slide[0].abs() > slide[1].abs() {
            assert!(
                turn[1] * slide[0] < 0.0 && turn[0] == 0.0,
                "beat {n}: slide {slide:?} turn {turn:?}"
            );
        } else {
            assert!(
                turn[0] * slide[1] < 0.0 && turn[1] == 0.0,
                "beat {n}: slide {slide:?} turn {turn:?}"
            );
        }
        seen.push(turn);
    }
    // No two pictures in the story arrive alike.
    for (i, a) in seen.iter().enumerate() {
        for b in &seen[i + 1..] {
            assert_ne!(a, b, "{seen:?}");
        }
    }
}

#[test]
fn the_slide_and_the_turn_move_together_and_come_to_rest() {
    let p = space();
    for (n, s) in picture_beats(&p) {
        for plane in ["hero", "prop"] {
            let id = format!("b{n}.{plane}.depth");
            let ms = on(s, &id);
            if ms.is_empty() && plane == "prop" {
                continue; // a beat without a supporting picture
            }
            let slide = ms
                .iter()
                .find(|m| matches!(m.op, MotionOp::Move { .. }))
                .expect("slide");
            let turn = ms
                .iter()
                .find(|m| matches!(m.op, MotionOp::Tilt { .. }))
                .expect("turn");
            assert_eq!(slide.start, turn.start, "{id}: same start");
            assert_eq!(slide.duration, turn.duration, "{id}: same duration");
            match (&slide.op, &turn.op) {
                (MotionOp::Move { to: a, .. }, MotionOp::Tilt { to: b, .. }) => {
                    assert_eq!(*a, [0.0, 0.0], "{id}: slide ends at rest");
                    assert_eq!(*b, [0.0, 0.0], "{id}: turn ends flat");
                }
                _ => unreachable!(),
            }
            assert!(
                slide.start + slide.duration < s.duration_seconds - 1.0,
                "{id}: settles well before the beat ends"
            );
        }
    }
}

#[test]
fn the_supporting_picture_is_inside_the_frame_and_named() {
    let p = space();
    let (w, h) = (p.canvas.width as f32, p.canvas.height as f32);
    let mut checked = 0;
    for (n, s) in picture_beats(&p) {
        let Some(prop) = find(&s.layers, &format!("b{n}.prop.depth")) else {
            continue; // a beat without a supporting picture
        };
        checked += 1;
        let (pw, ph) = (
            prop.width * prop.scale_x.abs(),
            prop.height * prop.scale_y.abs(),
        );
        let (x, y) = (prop.x - prop.anchor_x * pw, prop.y - prop.anchor_y * ph);
        let slack = 0.02 * w;
        assert!(
            x >= -slack && y >= -slack && x + pw <= w + slack && y + ph <= h + slack,
            "beat {n}: supporting picture ({x:.0},{y:.0},{pw:.0},{ph:.0}) leaves the {w}x{h} frame"
        );
        // It sits on the focus side of the hero plane, not far behind it.
        assert!(prop.z.is_some_and(|z| z <= 0.0), "beat {n}: z {:?}", prop.z);
        // Its meaning is written under it.
        let label = find(&s.layers, &format!("b{n}.prop_label.depth"))
            .unwrap_or_else(|| panic!("beat {n}: no label"));
        let LayerKind::Group { children } = &label.kind else {
            panic!("beat {n}: label wrapper is not a group");
        };
        assert!(
            children
                .iter()
                .any(|c| matches!(&c.kind, LayerKind::Text(t) if !t.text.trim().is_empty())),
            "beat {n}: empty label"
        );
    }
    assert!(checked >= 2, "supporting pictures checked: {checked}");
}
