//! (0.19) The `layers` subject of CreativeIntent v0.2: parsing, semantic
//! validation, the v0.2-only gate and the public schema.
//!
//! The JSON schema (`schema/creative-intent-v0.2.schema.json`) states the
//! shape; `CreativeIntent::validate` the rules the shape cannot express
//! (unique names, boundary placement, the focus naming a layer).

use std::path::PathBuf;

use jsonschema::Validator;
use motion_core::intent::{CreativeIntent, Subject};
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn validator() -> Validator {
    let text = std::fs::read_to_string(repo().join("schema/creative-intent-v0.2.schema.json"))
        .expect("schema");
    let schema: Value = serde_json::from_str(&text).expect("schema json");
    jsonschema::validator_for(&schema).expect("schema compiles")
}

fn ocean() -> Value {
    json!({
        "kind": "layers",
        "meaning": "ocean water column",
        "layers": [
            { "name": "Photic zone", "note": "sunlit, photosynthesis" },
            { "name": "Thermocline", "note": "density boundary", "boundary": true },
            { "name": "Aphotic zone", "note": "dark, decomposition" }
        ],
        "focus": "Thermocline"
    })
}

fn doc(primary: Value) -> Value {
    json!({
        "version": "0.2", "title": "layers_test", "format": "vertical",
        "beats": [{ "purpose": "emphasize", "statement": "Where it happens", "primary": primary }]
    })
}

fn rust_errors(v: &Value) -> Result<Vec<String>, String> {
    match serde_json::from_value::<CreativeIntent>(v.clone()) {
        Ok(i) => Ok(i.validate().err().unwrap_or_default()),
        Err(e) => Err(e.to_string()),
    }
}

fn valid(v: &Value) -> bool {
    matches!(rust_errors(v), Ok(e) if e.is_empty())
}

#[test]
fn a_stack_parses_validates_and_matches_the_schema() {
    let d = doc(ocean());
    assert!(validator().is_valid(&d), "schema rejects a valid stack");
    assert!(valid(&d), "{:?}", rust_errors(&d));
    let intent: CreativeIntent = serde_json::from_value(d).expect("parses");
    let Subject::Layers(l) = &intent.beats[0].primary else {
        panic!("not a layers subject");
    };
    assert_eq!(l.layers.len(), 3);
    assert!(l.layers[1].boundary && !l.layers[0].boundary);
    assert_eq!(l.focus_index(), Some(1));
    assert_eq!(intent.beats[0].primary.kind_name(), "layers");
    assert_eq!(
        intent.beats[0].primary.meaning(),
        Some("ocean water column")
    );
}

#[test]
fn focus_matches_names_without_regard_to_case_and_spaces() {
    let mut s = ocean();
    s["focus"] = json!("  aphotic ZONE ");
    let intent: CreativeIntent = serde_json::from_value(doc(s)).expect("parses");
    let Subject::Layers(l) = &intent.beats[0].primary else {
        panic!()
    };
    assert_eq!(l.focus_index(), Some(2));
}

#[test]
fn same_stack_ignores_the_focus_but_not_the_layers() {
    let a: CreativeIntent = serde_json::from_value(doc(ocean())).expect("parses");
    let mut other = ocean();
    other["focus"] = json!("Photic zone");
    let b: CreativeIntent = serde_json::from_value(doc(other)).expect("parses");
    let mut renamed = ocean();
    renamed["layers"][0]["name"] = json!("Sunlit zone");
    let c: CreativeIntent = serde_json::from_value(doc(renamed)).expect("parses");
    let get = |i: &CreativeIntent| match &i.beats[0].primary {
        Subject::Layers(l) => l.clone(),
        _ => panic!(),
    };
    assert!(get(&a).same_stack(&get(&b)));
    assert!(!get(&a).same_stack(&get(&c)));
}

#[test]
fn focus_and_meaning_are_optional() {
    let d = doc(json!({
        "kind": "layers",
        "layers": [{ "name": "Top" }, { "name": "Bottom" }]
    }));
    assert!(validator().is_valid(&d));
    assert!(valid(&d), "{:?}", rust_errors(&d));
}

#[test]
fn the_number_of_layers_is_two_to_six() {
    let layer = |n: usize| json!({ "name": format!("L{n}") });
    for (count, ok) in [(1usize, false), (2, true), (6, true), (7, false)] {
        let d =
            doc(json!({ "kind": "layers", "layers": (0..count).map(layer).collect::<Vec<_>>() }));
        assert_eq!(
            validator().is_valid(&d),
            ok,
            "{count} layers: schema verdict"
        );
        assert_eq!(valid(&d), ok, "{count} layers: rust verdict");
    }
}

#[test]
fn names_must_be_present_and_distinct() {
    let mut s = ocean();
    s["layers"][2]["name"] = json!("photic ZONE");
    let errs = rust_errors(&doc(s)).expect("parses");
    assert!(
        errs.iter().any(|e| e.contains("names must differ")),
        "{errs:?}"
    );
    let mut s = ocean();
    s["layers"][0]["name"] = json!("   ");
    let errs = rust_errors(&doc(s)).expect("parses");
    assert!(
        errs.iter().any(|e| e.contains("must not be empty")),
        "{errs:?}"
    );
}

#[test]
fn a_boundary_sits_between_two_ordinary_layers() {
    // First / last cannot be boundaries.
    for idx in [0usize, 2] {
        let mut s = ocean();
        s["layers"][idx]["boundary"] = json!(true);
        let errs = rust_errors(&doc(s)).expect("parses");
        assert!(
            errs.iter()
                .any(|e| e.contains("first and the last layer cannot be boundaries")),
            "{idx}: {errs:?}"
        );
    }
    // Two boundaries in a row.
    let d = doc(json!({
        "kind": "layers",
        "layers": [
            { "name": "A" },
            { "name": "B", "boundary": true },
            { "name": "C", "boundary": true },
            { "name": "D" }
        ]
    }));
    let errs = rust_errors(&d).expect("parses");
    assert!(
        errs.iter()
            .any(|e| e.contains("between two ordinary layers")),
        "{errs:?}"
    );
    // A boundary between ordinary layers is fine.
    assert!(valid(&doc(ocean())));
}

#[test]
fn the_focus_must_name_a_layer() {
    let mut s = ocean();
    s["focus"] = json!("Abyss");
    let errs = rust_errors(&doc(s)).expect("parses");
    assert!(
        errs.iter()
            .any(|e| e.contains("focus 'Abyss' is not the name of any layer")),
        "{errs:?}"
    );
}

#[test]
fn layers_are_the_primary_never_the_secondary() {
    let d = json!({
        "version": "0.2", "title": "t", "format": "vertical",
        "beats": [{
            "purpose": "compare", "statement": "x",
            "primary": { "kind": "phrase", "value": "a" },
            "secondary": ocean()
        }]
    });
    let errs = rust_errors(&d).expect("parses");
    assert!(
        errs.iter().any(|e| e.contains("must be the primary")),
        "{errs:?}"
    );
}

#[test]
fn unknown_fields_are_rejected() {
    let mut s = ocean();
    s["height"] = json!(300);
    assert!(rust_errors(&doc(s.clone())).is_err(), "stack: rust");
    assert!(!validator().is_valid(&doc(s)), "stack: schema");
    let mut s = ocean();
    s["layers"][0]["thickness"] = json!(2);
    assert!(rust_errors(&doc(s.clone())).is_err(), "layer: rust");
    assert!(!validator().is_valid(&doc(s)), "layer: schema");
}

#[test]
fn layers_need_version_0_2() {
    let mut d = doc(ocean());
    d["version"] = json!("0.1");
    let err =
        CreativeIntent::from_json(&d.to_string()).expect_err("a 0.1 document cannot use layers");
    assert!(
        err.to_string().contains("requires \"version\": \"0.2\""),
        "{err}"
    );
}
