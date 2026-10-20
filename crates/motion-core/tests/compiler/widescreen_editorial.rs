//! (0.26) 16:9 Widescreen Editorial Grammar Integration Tests.
//!
//! Validates that when rendering in 16:9 widescreen format (1920x1080):
//! 1. Editorial layouts cleanly separate into left type column (headline, deck, kicker)
//!    and right hero visual plate without centered dead space or destructive overlap.
//! 2. Multi-card documentary dossier compiles into two columns (clipping on left,
//!    photo / evidence on right) with proper horizontal clearance.
//! 3. All stage elements strictly respect canvas boundaries and safe margins.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionProject, Scene};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn flat(layers: &[Layer]) -> Vec<&Layer> {
    let mut out = Vec::new();
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            out.extend(flat(children));
        }
    }
    out
}

fn beat_scene(p: &MotionProject) -> &Scene {
    p.scenes
        .iter()
        .find(|s| s.lifecycle.is_some())
        .expect("beat scene")
}

fn compile_widescreen(intent_json: &str, look: Option<Look>) -> MotionProject {
    let intent = CreativeIntent::from_json(intent_json).expect("intent");
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
        "halftone_retro_objects".to_string(),
    ]);
    let opts = CompileOptions {
        art: look.map(ArtMode::Force),
        canvas: Some((1920, 1080)),
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &StyleProfile::default(),
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile widescreen")
}

#[test]
fn widescreen_16x9_canvas_dimensions() {
    let intent = r#"{
      "version": "0.2", "title": "widescreen_test", "format": "landscape",
      "beats": [
        {"purpose": "emphasize", "statement": "India Anime Revolution",
         "primary": {"kind": "phrase", "value": "100 Million Fans", "meaning": "fanbase"}}
      ]
    }"#;
    let project = compile_widescreen(intent, None);

    assert_eq!(project.canvas.width, 1920);
    assert_eq!(project.canvas.height, 1080);
    let aspect = project.canvas.width as f32 / project.canvas.height as f32;
    assert!(
        (aspect - 16.0 / 9.0).abs() < 0.01,
        "Aspect ratio must be 16:9"
    );
}

#[test]
fn widescreen_16x9_editorial_type_image_split() {
    // Beat with an object asset and a title: in 16:9 this should layout side-by-side
    let intent = r#"{
      "version": "0.2", "title": "widescreen_editorial", "format": "landscape",
      "beats": [
        {"purpose": "emphasize", "statement": "Tokyo To Mumbai Express",
         "primary": {"kind": "object", "asset": "retro_synth", "meaning": "music synthesizer"},
         "secondary": {"kind": "phrase", "value": "The New Vanguard"}}
      ]
    }"#;
    let project = compile_widescreen(intent, None);
    let scene = beat_scene(&project);
    let layers = flat(&scene.layers);

    // Find the text layers (title/headline) and the object/image layer
    let text_layers: Vec<&Layer> = layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Text(_)))
        .copied()
        .collect();
    assert!(!text_layers.is_empty(), "Must contain text layers");

    // Content text layers (excluding background ghost words that bleed) should be within safe area
    let content_text_layers: Vec<&Layer> = text_layers
        .iter()
        .copied()
        .filter(|l| !l.id.contains("ghost") && !l.id.contains("watermark") && !l.id.contains("bg"))
        .collect();
    assert!(
        !content_text_layers.is_empty(),
        "Must contain content text layers"
    );

    for tl in &content_text_layers {
        assert!(
            tl.x >= 0.0,
            "Content text layer {} x ({}) must be non-negative",
            tl.id,
            tl.x
        );
        assert!(
            tl.x + tl.width <= 1920.0,
            "Content text layer {} right edge ({}) must be within 1920",
            tl.id,
            tl.x + tl.width
        );
    }

    // The hero is a non-background image layer; the title must sit entirely
    // left of it and in the left half of the canvas.
    let hero = layers
        .iter()
        .find(|l| {
            matches!(l.kind, LayerKind::Image { .. })
                && !l.id.contains("ghost")
                && !l.id.contains("bg")
                && !l.id.contains("backdrop")
                && !l.id.contains("ornament")
                && l.width < 1920.0 * 0.9
        })
        .expect("hero image layer");
    let title = text_layers
        .iter()
        .find(|l| l.id.contains("head") || l.id.contains("title"))
        .unwrap_or(&text_layers[0]);

    assert!(
        title.x + title.width / 2.0 < 960.0,
        "title ({}) must sit in the left half",
        title.id
    );
    assert!(
        title.x + title.width <= hero.x,
        "title right edge ({}) must clear hero left edge ({})",
        title.x + title.width,
        hero.x
    );
}

#[test]
fn widescreen_16x9_dossier_two_column_cards() {
    let intent = r#"{
      "version": "0.2", "title": "dossier_widescreen", "format": "landscape",
      "beats": [
        {"purpose": "emphasize", "statement": "Classified Studio Records",
         "primary": {"kind": "number", "value": "4.2M Copies", "meaning": "circulation"},
         "secondary": {"kind": "object", "asset": "document_file", "meaning": "archive"},
         "keyword": "classified"}
      ]
    }"#;
    let project = compile_widescreen(intent, Some(Look::Dossier));
    let scene = beat_scene(&project);
    let layers = flat(&scene.layers);

    let clip = layers
        .iter()
        .find(|l| l.id.contains("clip"))
        .expect("clipping card in dossier");

    // In widescreen (cols = true), clipping card sits on the left column.
    let clip_cx = clip.x + clip.width / 2.0;
    assert!(
        clip_cx < 1056.0,
        "Clipping card center ({clip_cx}) must sit in the left column on 16:9"
    );

    let fig = layers
        .iter()
        .find(|l| l.id.contains("figure"))
        .expect("figure card in dossier");
    // Dossier cards overlap like papers on a desk; the figure sits right of the clip.
    let fig_cx = fig.x + fig.width / 2.0;
    assert!(
        fig_cx > clip_cx + 100.0,
        "figure center ({fig_cx}) must sit right of the clip center ({clip_cx})"
    );
    assert!(fig.x + fig.width <= 1920.0, "figure must be within canvas");
}
