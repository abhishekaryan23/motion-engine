//! (0.22) Every value a story gives is shown. A compare / contrast beat of two
//! pictures with values — "{object, value, meaning}" twice, the consumer's
//! country-statistics shape — shows both pictures and both figures in every
//! look (the intent contract: an object's `value` is "a short figure stamped
//! next to the object in compare/contrast beats").

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Two catalog pictures every look's families include (`editorial_cutout`).
fn pair_story(purpose: &str, relationship: &str) -> CreativeIntent {
    CreativeIntent::from_json(&format!(
        r#"{{"version":"0.2","title":"pair","format":"vertical","beats":[{{
            "purpose":"{purpose}","statement":"Savings against spending",
            "primary":{{"kind":"object","asset":"coin_stack","value":"4.1%","meaning":"Savings"}},
            "secondary":{{"kind":"object","asset":"laptop","value":"3.1%","meaning":"Spending"}},
            "relationship":"{relationship}"}}]}}"#
    ))
    .expect("intent")
}

fn compile(intent: &CreativeIntent, look: Look) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        art: Some(ArtMode::Force(look)),
        ..CompileOptions::default()
    };
    compile_with_options(
        intent,
        &StyleProfile::default(),
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn texts(project: &MotionProject) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>) {
        for l in layers {
            match &l.kind {
                LayerKind::Text(t) => out.push(t.text.clone()),
                LayerKind::Group { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &project.scenes {
        walk(&s.layers, &mut out);
    }
    out
}

/// Image layers of the beats (not the backdrop), as their asset ids.
fn pictures(project: &MotionProject) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>) {
        for l in layers {
            match &l.kind {
                LayerKind::Image { asset, .. } => out.push(asset.clone()),
                LayerKind::Group { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        walk(&s.layers, &mut out);
    }
    out.retain(|a| !a.contains("plate") && !a.contains("ground"));
    out.sort();
    out.dedup();
    out
}

#[test]
fn both_pictures_and_both_values_in_every_look() {
    for (purpose, relationship) in [("compare", "separate"), ("contrast", "compress")] {
        let story = pair_story(purpose, relationship);
        for look in Look::ALL {
            let project = compile(&story, look);
            let t = texts(&project);
            for value in ["4.1%", "3.1%"] {
                assert!(
                    t.iter().any(|x| x.contains(value)),
                    "{look:?} {purpose}: value {value} missing from {t:?}"
                );
            }
            let p = pictures(&project);
            assert!(
                p.len() >= 2,
                "{look:?} {purpose}: expected two pictures, got {p:?}"
            );
        }
    }
}
