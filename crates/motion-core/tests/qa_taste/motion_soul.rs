//! (0.24 Wave 1) Motion soul: one motion grammar for every layout (role
//! offsets, masked rises, no only-fade text) and the `≠` substitution.

use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions,
    WARN_DISPLAY_SYMBOL_SUBSTITUTED,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{LayerKind, MotionProject, Scene};
use motion_core::slop_qa::{slop_report, soul_metrics, SlopCheck, SlopRules};
use motion_core::style::StyleProfile;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn compile(json: &str) -> (MotionProject, Vec<String>) {
    let intent = CreativeIntent::from_json(json).expect("intent");
    let (p, w) = compile_with_report(
        &intent,
        &StyleProfile::default(),
        None,
        &AssetLibrary::new(Path::new(ROOT).join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .expect("compiles");
    (p, w.into_iter().map(|w| w.code).collect())
}

fn texts(layers: &[motion_core::scene::Layer], out: &mut Vec<String>) {
    for l in layers {
        match &l.kind {
            LayerKind::Text(t) => out.push(t.text.clone()),
            LayerKind::Group { children } => texts(children, out),
            _ => {}
        }
    }
}

#[test]
fn examples_have_no_text_that_only_fades_and_overlap_roles() {
    let mut total = 0.0;
    let mut n = 0;
    for name in [
        "examples/editorial_demo",
        "examples/public/minimal-emphasize",
        "examples/public/minimal-contrast",
        "examples/public/three-beat-story",
        "examples/public/state-change",
        "examples/voice_10/small_savings",
        "examples/topics/vaccines",
    ] {
        let json = std::fs::read_to_string(Path::new(ROOT).join(format!("{name}.intent.json")))
            .expect("example");
        let (p, _) = compile(&json);
        let m = soul_metrics(&p);
        assert_eq!(m.things_appear, 0, "{name}");
        let slop = slop_report(&p, &SlopRules::bundled());
        assert!(
            !slop
                .findings
                .iter()
                .any(|f| matches!(f.check, SlopCheck::ThingsAppear | SlopCheck::Flicker)),
            "{name}"
        );
        total += m.overlap_ratio;
        n += 1;
    }
    assert!(total / n as f32 >= 0.6, "mean overlap {}", total / n as f32);
}

#[test]
fn not_equal_is_said_in_words_on_its_own_line_in_display_text() {
    let json = r#"{
      "version": "0.2", "title": "ne", "format": "vertical",
      "beats": [{"purpose": "emphasize", "statement": "Income ≠ wealth",
        "primary": {"kind": "phrase", "value": "income is not wealth"},
        "energy": "impact"}]
    }"#;
    let (p, codes) = compile(json);
    assert!(codes.iter().any(|c| c == WARN_DISPLAY_SYMBOL_SUBSTITUTED));
    let mut all = Vec::new();
    for s in p.scenes.iter() {
        let s: &Scene = s;
        texts(&s.layers, &mut all);
    }
    assert!(all.iter().all(|t| !t.contains('≠')), "{all:?}");
    assert!(
        all.iter()
            .any(|t| t.lines().any(|l| l.trim() == "IS NOT") && t.lines().count() == 3),
        "{all:?}"
    );
}
