//! (0.10 Q) Under art direction, library assets with loops come alive: hero
//! loops replace their stills, screen hosts play the family's insert loop.
use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{compile_with_options, AssetLibrary, CompileOptions};
use motion_core::scene::{AssetKind, LayerKind, MotionProject};
use motion_core::{ApproxMeasure, CreativeIntent, StyleProfile};

fn root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile(intent: &str, style: &str, art: Option<ArtMode>) -> MotionProject {
    let intent =
        CreativeIntent::from_json(&std::fs::read_to_string(root().join(intent)).unwrap()).unwrap();
    let style =
        StyleProfile::from_json(&std::fs::read_to_string(root().join(style)).unwrap()).unwrap();
    let opts = CompileOptions {
        art,
        ..CompileOptions::default()
    };
    compile_with_options(
        &intent,
        &style,
        None,
        &AssetLibrary::new(root().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap()
}

fn inserts(p: &MotionProject) -> Vec<[f32; 4]> {
    fn walk(ls: &[motion_core::scene::Layer], out: &mut Vec<[f32; 4]>) {
        for l in ls {
            if let LayerKind::Image {
                insert: Some(i), ..
            } = &l.kind
            {
                out.push(i.screen_box);
            }
            if let LayerKind::Group { children } = &l.kind {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    for s in &p.scenes {
        walk(&s.layers, &mut out);
    }
    out
}

#[test]
fn retro_tv_plays_its_insert_loop() {
    let p = compile(
        "examples/library_09/retro.intent.json",
        "examples/taste/playful_print.style.json",
        Some(ArtMode::Force(Look::HalftoneCutout)),
    );
    assert!(p
        .assets
        .iter()
        .any(|a| a.kind == AssetKind::SpriteSequence && a.id == "asset.loop.tv_toaster_ad"));
    let boxes = inserts(&p);
    assert!(!boxes.is_empty());
    for b in boxes {
        assert!(b.iter().all(|v| (0.0..=1.0).contains(v)) && b[2] > 0.0 && b[3] > 0.0);
    }
    assert!(motion_core::validate(&p, Some(&root().join("assets"))).is_ok());
}

#[test]
fn hero_loops_replace_stills_and_nothing_changes_without_art() {
    let with = compile(
        "examples/voice_10/small_savings.intent.json",
        "examples/taste/warm_editorial.style.json",
        Some(ArtMode::Force(Look::ClayPop)),
    );
    assert!(with
        .assets
        .iter()
        .any(|a| a.kind == AssetKind::SpriteSequence && a.path.ends_with("loops/piggy_bank")));
    let without = compile(
        "examples/library_09/retro.intent.json",
        "examples/taste/playful_print.style.json",
        None,
    );
    assert!(without
        .assets
        .iter()
        .all(|a| a.kind != AssetKind::SpriteSequence));
    assert!(inserts(&without).is_empty());
}
