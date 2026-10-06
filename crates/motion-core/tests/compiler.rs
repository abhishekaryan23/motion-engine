//! Compiler integration tests: semantic intent -> MotionScene structure.
//!
//! Assertions target structure (ids, ops, ordering, validity), never exact
//! pixel or design values.

use std::path::Path;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary, CompileError};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Color, Layer, LayerKind, Motion, MotionProject, Scene};
use motion_core::style::{AccentRole, StyleProfile};
use motion_core::validate::validate;
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const DEMO_INTENT: &str = include_str!("../../../examples/editorial_demo.intent.json");
const DEMO_STYLE: &str = include_str!("../../../examples/editorial_demo.style.json");

fn library() -> AssetLibrary {
    AssetLibrary::new(ASSETS)
}

fn demo_intent() -> CreativeIntent {
    CreativeIntent::from_json(DEMO_INTENT).expect("demo intent parses")
}

fn demo_style() -> StyleProfile {
    StyleProfile::from_json(DEMO_STYLE).expect("demo style parses")
}

fn compile_demo_with(style: &StyleProfile) -> MotionProject {
    compile(&demo_intent(), style, &library(), &ApproxMeasure).expect("demo compiles")
}

fn compile_demo() -> MotionProject {
    compile_demo_with(&demo_style())
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene '{id}' missing"))
}

/// Depth-first search for a layer by id.
fn find_layer<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            if let Some(found) = find_layer(children, id) {
                return Some(found);
            }
        }
    }
    None
}

fn layer_in_scene<'a>(s: &'a Scene, id: &str) -> &'a Layer {
    find_layer(&s.layers, id).unwrap_or_else(|| panic!("layer '{id}' missing in {}", s.id))
}

fn motions_on<'a>(s: &'a Scene, target: &'a str) -> impl Iterator<Item = &'a Motion> {
    s.motions.iter().filter(move |m| m.target == target)
}

fn count_op(s: &Scene, target: &str, op: &str) -> usize {
    motions_on(s, target)
        .filter(|m| m.op.op_name() == op)
        .count()
}

fn collect_ids(layers: &[Layer], out: &mut Vec<String>) {
    for l in layers {
        out.push(l.id.clone());
        if let LayerKind::Group { children } = &l.kind {
            collect_ids(children, out);
        }
    }
}

/// Sorted scene ids + layer ids + shared ids: the id structure of a project.
fn id_structure(p: &MotionProject) -> Vec<String> {
    let mut ids = Vec::new();
    for s in &p.scenes {
        ids.push(format!("scene:{}", s.id));
        collect_ids(&s.layers, &mut ids);
    }
    for e in &p.shared {
        ids.push(format!("shared:{}", e.id));
        ids.push(e.layer.id.clone());
    }
    ids.sort();
    ids
}

/// All fill / texture colors in the project (hex), sorted.
fn colors(p: &MotionProject) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<Color>) {
        for l in layers {
            match &l.kind {
                LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => {
                    out.push(*fill)
                }
                LayerKind::Text(t) => out.push(t.color),
                LayerKind::Texture(t) => out.push(t.color),
                LayerKind::Group { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut cs = Vec::new();
    for s in &p.scenes {
        walk(&s.layers, &mut cs);
    }
    let mut hex: Vec<String> = cs.into_iter().map(Color::to_hex).collect();
    hex.sort();
    hex
}

fn compile_json(intent: Value) -> Result<MotionProject, CompileError> {
    let intent: CreativeIntent = serde_json::from_value(intent).expect("intent json parses");
    compile(
        &intent,
        &StyleProfile::default(),
        &library(),
        &ApproxMeasure,
    )
}

fn one_beat(beat: Value) -> Value {
    json!({ "version": "0.1", "title": "one_beat", "beats": [beat] })
}

fn assert_compiles_and_validates(intent: Value) {
    let p = compile_json(intent).expect("compiles");
    validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("validation failed:\n{e}"));
    assert_eq!(p.scenes.len(), 2, "backdrop + one beat");
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

#[test]
fn demo_files_parse() {
    let intent = demo_intent();
    assert_eq!(intent.beats.len(), 3);
    let style = demo_style();
    assert_eq!(style.seed, 7);
}

#[test]
fn default_style_compiles() {
    let p = compile(
        &demo_intent(),
        &StyleProfile::default(),
        &library(),
        &ApproxMeasure,
    )
    .expect("compiles with default style");
    validate(&p, Some(Path::new(ASSETS))).expect("valid");
}

#[test]
fn unknown_intent_field_is_rejected() {
    let bad = json!({ "version": "0.1", "title": "x", "beats": [], "pixels": 12 });
    assert!(serde_json::from_value::<CreativeIntent>(bad).is_err());

    let bad_beat = one_beat(json!({
        "purpose": "emphasize",
        "statement": "hi",
        "primary": { "kind": "phrase", "value": "hi" },
        "x": 100
    }));
    assert!(serde_json::from_value::<CreativeIntent>(bad_beat).is_err());
}

#[test]
fn unknown_style_field_is_rejected() {
    assert!(StyleProfile::from_json(r#"{ "family": "minimal", "bogus": 1 }"#).is_err());
}

// ---------------------------------------------------------------------------
// Determinism and validity
// ---------------------------------------------------------------------------

#[test]
fn compile_is_deterministic() {
    let a = compile_demo().to_json_pretty();
    let b = compile_demo().to_json_pretty();
    assert_eq!(a, b);
}

#[test]
fn compiled_demo_validates_against_assets_root() {
    let p = compile_demo();
    assert!(p.asset_root.is_none());
    validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("validation failed:\n{e}"));
}

// ---------------------------------------------------------------------------
// Scene structure
// ---------------------------------------------------------------------------

#[test]
fn scene_structure_and_overlap() {
    let p = compile_demo();
    let ids: Vec<&str> = p.scenes.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["backdrop", "beat_1", "beat_2", "beat_3"]);

    let (b1, b2, b3) = (
        scene(&p, "beat_1"),
        scene(&p, "beat_2"),
        scene(&p, "beat_3"),
    );
    assert!(
        b2.start_seconds < b1.end_seconds(),
        "beat_2 overlaps beat_1"
    );
    assert!(
        b3.start_seconds < b2.end_seconds(),
        "beat_3 overlaps beat_2"
    );

    let total = p.duration_seconds();
    assert!((10.0..=15.0).contains(&total), "duration {total}");

    assert_eq!(
        (p.canvas.width, p.canvas.height, p.canvas.fps),
        (1080, 1920, 30)
    );
}

#[test]
fn contrast_compress_beat() {
    let p = compile_demo();
    let b2 = scene(&p, "beat_2");

    layer_in_scene(b2, "b2.panel");
    assert!(
        count_op(b2, "b2.panel", "accent_expand") >= 1,
        "panel has accent_expand"
    );

    let zone = layer_in_scene(b2, "b2.zone_b");
    let LayerKind::Group { children } = &zone.kind else {
        panic!("b2.zone_b should be a group");
    };
    fn has_svg(layers: &[Layer]) -> bool {
        layers.iter().any(|l| match &l.kind {
            LayerKind::Svg { .. } => true,
            LayerKind::Group { children } => has_svg(children),
            _ => false,
        })
    }
    assert!(has_svg(children), "zone_b contains an svg layer");

    assert!(count_op(b2, "b2.zone_b", "scale") >= 1);
    assert!(count_op(b2, "b2.zone_b", "move") >= 1);
}

#[test]
fn carry_creates_single_shared_element_spanning_beats() {
    let p = compile_demo();
    assert_eq!(p.shared.len(), 1, "one shared element");
    let track = &p.shared[0].track;

    for beat in ["beat_1", "beat_2", "beat_3"] {
        assert!(
            track.iter().any(|k| k.scene == beat),
            "track has a key in {beat}"
        );
    }

    let times: Vec<f64> = track
        .iter()
        .map(|k| p.scene_start(&k.scene).expect("known scene") + k.at)
        .collect();
    assert!(
        times.windows(2).all(|w| w[0] <= w[1]),
        "track key times non-decreasing: {times:?}"
    );

    assert!(
        track
            .iter()
            .any(|k| k.scene == "beat_2" && k.state.scale.is_some_and(|s| s < 1.0)),
        "beat_2 applies pressure (scale < 1)"
    );
}

#[test]
fn impact_reveal_after_building_beat() {
    let p = compile_demo();
    let b3 = scene(&p, "beat_3");
    let stage = layer_in_scene(b3, "b3.stage");
    assert_eq!(stage.z_index, 40);
    assert_eq!(count_op(b3, "b3.accent_slab", "accent_expand"), 2);
}

#[test]
fn accent_role_changes_colors_not_structure() {
    let base = demo_style();
    let mut cobalt = base.clone();
    cobalt.accent_role = AccentRole::Cobalt;
    assert_ne!(base.accent_role, cobalt.accent_role);

    let a = compile_demo_with(&base);
    let b = compile_demo_with(&cobalt);
    assert_eq!(id_structure(&a), id_structure(&b));
    assert_ne!(colors(&a), colors(&b), "accent colors differ");
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn wrong_version_is_rejected() {
    let mut intent = demo_intent();
    intent.version = "9.9".into();
    let err = compile(&intent, &demo_style(), &library(), &ApproxMeasure).unwrap_err();
    assert!(matches!(err, CompileError::Version(v) if v == "9.9"));
}

#[test]
fn empty_beats_is_rejected() {
    let mut intent = demo_intent();
    intent.beats.clear();
    let err = compile(&intent, &demo_style(), &library(), &ApproxMeasure).unwrap_err();
    assert!(matches!(err, CompileError::NoBeats));
}

#[test]
fn unknown_asset_degrades_to_a_phrase() {
    // (0.10) Weak-model safety: a missing asset no longer fails the compile.
    let p = compile_json(one_beat(json!({
        "purpose": "emphasize",
        "statement": "Look here",
        "primary": { "kind": "object", "asset": "no_such_asset", "value": "thing" }
    })))
    .expect("compiles");
    let json = serde_json::to_string(&p).unwrap();
    assert!(!json.contains("asset.no_such_asset"));
    assert!(json.to_lowercase().contains("no such asset"));
}

// ---------------------------------------------------------------------------
// One minimal beat per purpose
// ---------------------------------------------------------------------------

#[test]
fn purpose_emphasize() {
    assert_compiles_and_validates(one_beat(json!({
        "purpose": "emphasize",
        "statement": "Your salary stayed flat",
        "primary": { "kind": "number", "value": "50,000", "meaning": "salary" }
    })));
}

#[test]
fn purpose_compare() {
    assert_compiles_and_validates(one_beat(json!({
        "purpose": "compare",
        "statement": "Then versus now",
        "primary": { "kind": "number", "value": "20", "meaning": "then" },
        "secondary": { "kind": "number", "value": "45", "meaning": "now" },
        "relationship": "grow"
    })));
}

#[test]
fn purpose_contrast() {
    assert_compiles_and_validates(one_beat(json!({
        "purpose": "contrast",
        "statement": "Income stood still while costs rose",
        "primary": { "kind": "number", "value": "50,000", "meaning": "income" },
        "secondary": { "kind": "phrase", "value": "costs", "meaning": "prices" },
        "relationship": "separate"
    })));
}

#[test]
fn purpose_reveal() {
    assert_compiles_and_validates(one_beat(json!({
        "purpose": "reveal",
        "statement": "The real number",
        "primary": { "kind": "number", "value": "-27%", "meaning": "buying power" },
        "energy": "impact"
    })));
}

#[test]
fn purpose_explain() {
    assert_compiles_and_validates(one_beat(json!({
        "purpose": "explain",
        "statement": "Groceries take a bigger share",
        "primary": { "kind": "phrase", "value": "Groceries" },
        "secondary": { "kind": "object", "asset": "shopping_basket", "value": "+38%" }
    })));
}
