//! (0.24 V1) Under Look::OrnamentEditorial no ornament frames the page: no
//! corner flourishes and no filigree divider (the boxy video structure).

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::{CreativeIntent, Format};
use motion_core::scene::{Layer, LayerKind, MotionProject, Scene};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Minimal CreativeIntent v0.2 story for layout testing.
fn minimal_intent(format: Format) -> CreativeIntent {
    let fmt_str = match format {
        Format::Vertical => "vertical",
        Format::Square => "square",
        Format::Landscape => "landscape",
    };
    CreativeIntent::from_json(&format!(
        r#"{{
            "version": "0.2",
            "title": "ornament_editorial_test",
            "format": "{fmt_str}",
            "beats": [
                {{
                    "purpose": "emphasize",
                    "statement": "The Craft of Letterpress",
                    "primary": {{
                        "kind": "phrase",
                        "value": "timeless typography"
                    }},
                    "energy": "building"
                }}
            ]
        }}"#
    ))
    .expect("valid minimal CreativeIntent v0.2")
}

/// Compile a scene with the requested art look.
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
    .expect("compile succeeded")
}

/// Recursively collect all flattened layers from a scene.
fn flat_layers(layers: &[Layer]) -> Vec<&Layer> {
    let mut out = Vec::new();
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            out.extend(flat_layers(children));
        }
    }
    out
}

fn beat_scene(project: &MotionProject) -> &Scene {
    project
        .scenes
        .iter()
        .find(|s| s.id.starts_with("beat_"))
        .expect("beat scene exists")
}

#[test]
fn ornament_editorial_frames_nothing_on_any_format() {
    for format in [Format::Vertical, Format::Square, Format::Landscape] {
        let project = compile(&minimal_intent(format), Look::OrnamentEditorial);
        let scene = beat_scene(&project);
        let ids: Vec<&str> = flat_layers(&scene.layers)
            .iter()
            .map(|l| l.id.as_str())
            .collect();
        assert!(
            !ids.iter().any(|id| id.contains("ornament")),
            "{format:?}: {ids:?}"
        );
        assert!(
            !ids.iter().any(|id| id.ends_with("folio_rule")),
            "{format:?}: no page hairline"
        );
    }
}
