//! (0.21) Brand colours: `StyleProfile.brand` replaces the look's colours in
//! every look, keeps text readable, drops the look's paper plate under a brand
//! background, and leaves pieces without a brand byte-identical.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::brand::{contrast, parse_color, TEXT_CONTRAST};
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, WARN_BRAND_CONTRAST,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Color, Layer, LayerKind, MotionProject};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn story() -> CreativeIntent {
    CreativeIntent::from_json(
        &std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json")).unwrap(),
    )
    .unwrap()
}

fn compile(style: &str, art: Option<ArtMode>) -> (MotionProject, Vec<String>) {
    let style: StyleProfile = serde_json::from_str(style).expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        art,
        ..CompileOptions::default()
    };
    let (project, report) = compile_with_report(
        &story(),
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile");
    let warnings = report
        .iter()
        .filter(|w| w.code == WARN_BRAND_CONTRAST)
        .map(|w| w.message.clone())
        .collect();
    (project, warnings)
}

/// Every text colour in the piece.
fn text_colors(project: &MotionProject) -> Vec<Color> {
    fn walk(layers: &[Layer], out: &mut Vec<Color>) {
        for l in layers {
            match &l.kind {
                LayerKind::Text(t) => out.push(t.color),
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

const BRAND: &str = r##"{"brand": {"primary": "#0A84FF", "secondary": "#FFD60A", "background": "#101828", "text": "#1D2939"}}"##;

#[test]
fn the_brand_replaces_the_looks_colours_in_every_look() {
    let primary = parse_color("#0A84FF").unwrap();
    let ground = parse_color("#101828").unwrap();
    for art in [
        None,
        Some(ArtMode::Auto),
        Some(ArtMode::Force(Look::Cinematic3d)),
        Some(ArtMode::Force(Look::Dossier)),
        Some(ArtMode::Force(Look::ClayPop)),
    ] {
        let (project, warnings) = compile(BRAND, art);
        let palette = &project.theme.palette;
        let json = serde_json::to_string(&project).unwrap();
        assert!(json.contains(&primary.to_hex()), "{art:?}: primary unused");
        assert!(
            json.contains(&ground.to_hex()),
            "{art:?}: background unused"
        );
        assert!(
            !project
                .assets
                .iter()
                .any(|a| a.id.starts_with("asset.plate.")),
            "{art:?}: the look's paper plate stays under a brand background"
        );
        // The dark brand text colour would vanish on the dark ground: replaced and said so.
        assert!(
            !text_colors(&project).contains(&parse_color("#1D2939").unwrap()),
            "{art:?}"
        );
        assert!(
            warnings.iter().any(|w| w.starts_with("brand text #1D2939")),
            "{art:?}: {warnings:?}"
        );
        if let (Some(ink), Some(paper)) = (palette.get("ink"), palette.get("paper")) {
            assert!(contrast(*ink, *paper) >= TEXT_CONTRAST, "{art:?}");
        }
    }
}

#[test]
fn no_brand_changes_nothing() {
    let plain = compile(r#"{"tone":"editorial"}"#, Some(ArtMode::Auto)).0;
    let empty = compile(r#"{"tone":"editorial","brand":{}}"#, Some(ArtMode::Auto)).0;
    assert_eq!(
        serde_json::to_string(&plain).unwrap(),
        serde_json::to_string(&empty).unwrap()
    );
}

#[test]
fn a_bad_colour_is_a_compile_error_naming_the_field() {
    let style: StyleProfile = serde_json::from_str(r##"{"brand":{"primary":"#12345"}}"##).unwrap();
    let library = AssetLibrary::new(repo().join("assets"));
    let err = compile_with_report(
        &story(),
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("style.brand.primary '#12345'"), "{err}");
}

#[test]
fn the_documentary_underline_wears_the_brand() {
    // The look's stock red (#D62B1E) gives way to the brand's mark colour.
    let (project, _) = compile(BRAND, Some(ArtMode::Force(Look::Dossier)));
    let json = serde_json::to_string(&project).unwrap();
    assert!(!json.contains("#D62B1E"), "stock red under a brand");
    let (plain, _) = compile("{}", Some(ArtMode::Force(Look::Dossier)));
    assert!(
        serde_json::to_string(&plain).unwrap().contains("#D62B1E"),
        "the story should have an underlined figure (else this test proves nothing)"
    );
}
