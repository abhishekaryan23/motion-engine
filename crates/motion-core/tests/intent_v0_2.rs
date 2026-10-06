//! CreativeIntent v0.2: parsing, version dispatch, legacy conversion, semantic
//! validation and the v0.2 public schema.
//!
//! Structured subjects (collection / state_change / derived_metric) are tested for
//! parse + validate + schema only; their composers are tested elsewhere.

use std::path::PathBuf;

use jsonschema::Validator;
use motion_core::intent::{
    v0_1, Atom, Beat, Collection, CollectionItem, Continuity, CreativeIntent, DerivedMetric,
    Energy, Format, MetricFormat, NumericTerm, ObjectAtom, Operation, Purpose, Relationship,
    StateChange, Subject,
};
use motion_core::{compile, ApproxMeasure, AssetLibrary, CompileError, StyleProfile};
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn load(path: PathBuf) -> Value {
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn validator(name: &str) -> Validator {
    let schema = load(repo().join("schema").join(name));
    jsonschema::validator_for(&schema).unwrap_or_else(|e| panic!("compile schema {name}: {e}"))
}

fn schema_v02(v: &Value) -> bool {
    validator("creative-intent-v0.2.schema.json").is_valid(v)
}

fn schema_v01(v: &Value) -> bool {
    validator("creative-intent-v0.1.schema.json").is_valid(v)
}

/// Parse + semantic validation, as `compile` sees a document.
fn rust_ok(v: &Value) -> bool {
    match serde_json::from_value::<CreativeIntent>(v.clone()) {
        Ok(i) => i.validate().is_ok(),
        Err(_) => false,
    }
}

fn parse_err(v: &Value) -> String {
    serde_json::from_value::<CreativeIntent>(v.clone())
        .expect_err("should not parse")
        .to_string()
}

fn compile_ok(v: &Value) -> bool {
    let Ok(intent) = serde_json::from_value::<CreativeIntent>(v.clone()) else {
        return false;
    };
    compile(
        &intent,
        &StyleProfile::default(),
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .is_ok()
}

/// A v0.2 document with the given primary subject and optional extras on the beat.
fn doc(primary: Value) -> Value {
    json!({
        "version": "0.2",
        "title": "t",
        "beats": [{
            "purpose": "emphasize",
            "statement": "A statement here",
            "primary": primary
        }]
    })
}

fn doc_with(f: impl FnOnce(&mut Value)) -> Value {
    let mut d = doc(json!({ "kind": "phrase", "value": "hi" }));
    f(&mut d);
    d
}

fn collection(items: Vec<Value>) -> Value {
    json!({ "kind": "collection", "items": items, "meaning": "drinks" })
}

fn items(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| json!({ "kind": "phrase", "value": format!("item {i}") }))
        .collect()
}

fn state_change() -> Value {
    json!({ "kind": "state_change", "entity": "delivery time", "from": "three days", "to": "same day" })
}

fn ratio(numerator: Value, denominator: Value) -> Value {
    json!({
        "kind": "derived_metric",
        "numerator": { "value": numerator, "meaning": "sign-ups" },
        "denominator": { "value": denominator, "meaning": "visitors" }
    })
}

/// Assert schema and Rust agree that the document is accepted / rejected.
fn assert_verdict(name: &str, d: &Value, expected: bool) {
    assert_eq!(schema_v02(d), expected, "{name}: schema verdict\n{d:#}");
    assert_eq!(rust_ok(d), expected, "{name}: Rust verdict\n{d:#}");
}

// ---------------------------------------------------------------------------
// Structured subjects
// ---------------------------------------------------------------------------

#[test]
fn valid_collection() {
    assert_verdict("collection_3", &doc(collection(items(3))), true);
    assert_verdict("collection_2", &doc(collection(items(2))), true);
    assert_verdict("collection_6", &doc(collection(items(6))), true);
    let mixed = collection(vec![
        json!({ "kind": "phrase", "value": "Coffee" }),
        json!({ "kind": "number", "value": "₹120", "meaning": "coffee" }),
        json!({ "kind": "object", "asset": "shopping_basket" }),
    ]);
    assert_verdict("collection_mixed", &doc(mixed), true);
    // Optional `meaning` may be omitted.
    assert_verdict(
        "collection_no_meaning",
        &doc(json!({ "kind": "collection", "items": items(2) })),
        true,
    );
}

#[test]
fn collection_size_out_of_range_is_rejected() {
    for n in [0usize, 1, 7, 12] {
        assert_verdict(
            &format!("collection_{n}"),
            &doc(collection(items(n))),
            false,
        );
    }
    // The count is a semantic rule for Rust (serde alone parses it): validate names it.
    let d = doc(collection(items(1)));
    let intent: CreativeIntent = serde_json::from_value(d).expect("parses");
    let errs = intent.validate().expect_err("1 item is invalid");
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].contains("beat 1") && errs[0].contains("2 to 6"),
        "{errs:?}"
    );
}

#[test]
fn nested_collection_item_is_rejected() {
    let nested = collection(vec![
        json!({ "kind": "phrase", "value": "a" }),
        collection(items(2)),
    ]);
    let d = doc(nested);
    assert!(!schema_v02(&d));
    assert!(!rust_ok(&d));
    for kind in ["state_change", "derived_metric"] {
        let d = doc(collection(vec![
            json!({ "kind": "phrase", "value": "a" }),
            json!({ "kind": kind }),
        ]));
        assert!(!schema_v02(&d) && !rust_ok(&d), "{kind} item");
    }
}

#[test]
fn valid_state_change() {
    assert_verdict("state_change", &doc(state_change()), true);
    let mut with_meaning = state_change();
    with_meaning["meaning"] = json!("faster");
    assert_verdict("state_change_meaning", &doc(with_meaning), true);
    // As secondary too (two changes at once).
    let d = doc_with(|d| {
        d["beats"][0]["purpose"] = json!("compare");
        d["beats"][0]["primary"] = state_change();
        d["beats"][0]["secondary"] = state_change();
    });
    assert_verdict("state_change_pair", &d, true);
}

#[test]
fn state_change_missing_or_empty_fields_are_rejected() {
    for field in ["entity", "from", "to"] {
        let mut sc = state_change();
        sc.as_object_mut().expect("object").remove(field);
        assert!(!schema_v02(&doc(sc.clone())), "missing {field}: schema");
        assert!(
            serde_json::from_value::<CreativeIntent>(doc(sc)).is_err(),
            "missing {field}: serde"
        );

        for blank in ["", "   ", "\t\n"] {
            let mut sc = state_change();
            sc[field] = json!(blank);
            assert_verdict(&format!("{field}={blank:?}"), &doc(sc), false);
        }
    }
}

#[test]
fn valid_ratio() {
    assert_verdict("ratio", &doc(ratio(json!(80), json!(1000))), true);
    assert_verdict("ratio_float", &doc(ratio(json!(0.5), json!(2.5))), true);
    assert_verdict(
        "ratio_zero_numerator",
        &doc(ratio(json!(0), json!(10))),
        true,
    );
    assert_verdict("ratio_negative", &doc(ratio(json!(-3), json!(10))), true);
    let mut full = ratio(json!(800), json!(60000));
    full["format"] = json!("per_thousand");
    full["operation"] = json!("ratio");
    full["meaning"] = json!("complaint rate");
    assert_verdict("ratio_full", &doc(full), true);
    // Two metrics compared directly.
    let d = doc_with(|d| {
        d["beats"][0]["purpose"] = json!("compare");
        d["beats"][0]["primary"] = ratio(json!(80), json!(1000));
        d["beats"][0]["secondary"] = ratio(json!(120), json!(1000));
    });
    assert_verdict("ratio_pair", &d, true);
}

#[test]
fn zero_denominator_is_rejected_by_schema_and_rust() {
    for zero in [json!(0), json!(0.0), json!(-0.0)] {
        let d = doc(ratio(json!(80), zero.clone()));
        assert!(!schema_v02(&d), "schema accepts denominator {zero}");
        let intent: CreativeIntent = serde_json::from_value(d).expect("parses");
        let errs = intent.validate().expect_err("zero denominator");
        assert_eq!(
            errs,
            vec!["beat 1: primary.denominator.value must not be 0".to_string()]
        );
    }
    // Secondary slot and later beats are numbered and named correctly.
    let d = doc_with(|d| {
        let second = json!({
            "purpose": "compare",
            "statement": "Second",
            "primary": { "kind": "phrase", "value": "x" },
            "secondary": ratio(json!(1), json!(0))
        });
        d["beats"].as_array_mut().expect("array").push(second);
    });
    let intent: CreativeIntent = serde_json::from_value(d).expect("parses");
    assert_eq!(
        intent.validate().expect_err("invalid"),
        vec!["beat 2: secondary.denominator.value must not be 0".to_string()]
    );
}

#[test]
fn non_numeric_and_out_of_range_values_are_rejected() {
    let d = doc(ratio(json!("1,000"), json!(10)));
    assert!(!schema_v02(&d) && !rust_ok(&d), "string numerator");
    let d = doc(ratio(json!(10), json!("1,000")));
    assert!(!schema_v02(&d) && !rust_ok(&d), "string denominator");
    let d = doc(ratio(Value::Null, json!(10)));
    assert!(!schema_v02(&d) && !rust_ok(&d), "null numerator");

    // 1e400 does not fit an f64: serde_json refuses it in text (or yields inf, which
    // validate rejects) - either way it is rejected.
    let text = r#"{"version":"0.2","title":"t","beats":[{"purpose":"emphasize","statement":"s",
        "primary":{"kind":"derived_metric",
        "numerator":{"value":1e400,"meaning":"a"},"denominator":{"value":10,"meaning":"b"}}}]}"#;
    match CreativeIntent::from_json(text) {
        Err(_) => {}
        Ok(i) => assert!(i.validate().is_err(), "1e400 must be rejected"),
    }
    match serde_json::from_str::<CreativeIntent>(text) {
        Err(_) => {}
        Ok(i) => assert!(i.validate().is_err(), "1e400 must be rejected (serde)"),
    }

    // Directly constructed non-finite values are rejected by validate.
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut i = intent_with_metric(bad, 10.0);
        assert!(i.validate().is_err(), "numerator {bad}");
        i = intent_with_metric(10.0, bad);
        assert!(i.validate().is_err(), "denominator {bad}");
    }
    // A finite ratio that overflows is rejected too.
    assert!(intent_with_metric(1e308, 1e-10).validate().is_err());
}

fn intent_with_metric(numerator: f64, denominator: f64) -> CreativeIntent {
    let mut i = base_intent();
    i.beats[0].primary = Subject::DerivedMetric(DerivedMetric {
        operation: Operation::Ratio,
        numerator: NumericTerm {
            value: numerator,
            meaning: "a".into(),
        },
        denominator: NumericTerm {
            value: denominator,
            meaning: "b".into(),
        },
        format: MetricFormat::Percent,
        meaning: None,
    });
    i
}

#[test]
fn numeric_term_meaning_must_not_be_empty() {
    for blank in ["", "  "] {
        let mut d = doc(ratio(json!(80), json!(1000)));
        d["beats"][0]["primary"]["numerator"]["meaning"] = json!(blank);
        assert_verdict(&format!("numerator meaning {blank:?}"), &d, false);
    }
}

#[test]
fn unknown_fields_inside_structured_subjects_are_rejected() {
    let subjects = [
        ("phrase", json!({ "kind": "phrase", "value": "x" })),
        ("number", json!({ "kind": "number", "value": "1" })),
        (
            "object",
            json!({ "kind": "object", "asset": "shopping_basket" }),
        ),
        ("collection", collection(items(2))),
        ("state_change", state_change()),
        ("derived_metric", ratio(json!(1), json!(2))),
    ];
    for (name, subject) in subjects {
        assert_verdict(name, &doc(subject.clone()), true);
        let mut with_extra = subject.clone();
        with_extra["x"] = json!(10);
        assert_verdict(&format!("{name}+x"), &doc(with_extra), false);
    }
    // Nested: inside a collection item, and inside a numeric term.
    let mut c = collection(items(2));
    c["items"][0]["font_size"] = json!(40);
    assert_verdict("collection item extra", &doc(c), false);
    let mut m = ratio(json!(1), json!(2));
    m["numerator"]["unit"] = json!("kg");
    assert_verdict("numeric term extra", &doc(m), false);
    // An `asset` on a phrase is no longer tolerated in v0.2 (it was ignored in v0.1).
    assert_verdict(
        "phrase with asset",
        &doc(json!({ "kind": "phrase", "value": "x", "asset": "shopping_basket" })),
        false,
    );
}

#[test]
fn subject_shape_errors_are_rejected() {
    for (name, subject) in [
        ("missing kind", json!({ "value": "x" })),
        ("unknown kind", json!({ "kind": "image" })),
        ("string subject", json!("text")),
        ("object without asset", json!({ "kind": "object" })),
        ("collection without items", json!({ "kind": "collection" })),
        (
            "derived without denominator",
            json!({ "kind": "derived_metric", "numerator": { "value": 1, "meaning": "a" } }),
        ),
        ("derived bad format", {
            let mut m = ratio(json!(1), json!(2));
            m["format"] = json!("bps");
            m
        }),
        ("derived bad operation", {
            let mut m = ratio(json!(1), json!(2));
            m["operation"] = json!("sum");
            m
        }),
    ] {
        assert!(!schema_v02(&doc(subject.clone())), "{name}: schema accepts");
        assert!(!rust_ok(&doc(subject)), "{name}: Rust accepts");
    }
}

#[test]
fn object_assets_are_open_snake_case_names() {
    // (0.10) Any snake_case noun is valid; compile finds a picture or shows text.
    let d = doc(json!({ "kind": "object", "asset": "rocket" }));
    assert!(schema_v02(&d));
    assert!(compile_ok(&d));
    let unknown = doc(json!({ "kind": "object", "asset": "zz_unknown_thing" }));
    assert!(schema_v02(&unknown));
    assert!(compile_ok(&unknown));
    // Not a name: spaces/uppercase are still rejected by the schema.
    assert!(!schema_v02(&doc(
        json!({ "kind": "object", "asset": "Piggy Bank" })
    )));
}

#[test]
fn accumulate_relationship_is_accepted_in_v0_2() {
    let d = doc_with(|d| {
        let b = &mut d["beats"][0];
        b["purpose"] = json!("contrast");
        b["primary"] = collection(items(3));
        b["secondary"] = json!({ "kind": "number", "value": "₹9,000", "meaning": "a month" });
        b["relationship"] = json!("accumulate");
    });
    assert_verdict("accumulate", &d, true);
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

fn v01_doc() -> Value {
    json!({
        "version": "0.1",
        "title": "t",
        "beats": [{
            "purpose": "emphasize",
            "statement": "A statement here",
            "primary": { "kind": "number", "value": "120", "meaning": "passengers" }
        }]
    })
}

fn v01_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(repo().join("examples/public"))
        .expect("examples/public")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.to_string_lossy().ends_with(".intent.json"))
        .collect();
    files.sort();
    let mut bench: Vec<PathBuf> = std::fs::read_dir(repo().join("golden/fixtures/benchmark"))
        .expect("benchmark")
        .map(|e| e.expect("entry").path().join("attempt-01.intent.json"))
        .filter(|p| p.is_file())
        .collect();
    bench.sort();
    files.extend(bench);
    files.push(repo().join("examples/editorial_demo.intent.json"));
    // Public examples written for v0.2 are covered by the public_contract tests.
    files.retain(|p| load(p.clone())["version"] == "0.1");
    files
}

#[test]
fn v0_1_public_examples_and_benchmarks_are_accepted_and_compile() {
    let files = v01_files();
    assert!(files.len() >= 8, "expected public examples + benchmarks");
    for path in files {
        let name = path.display().to_string();
        let v = load(path.clone());
        assert_eq!(v["version"], "0.1", "{name}");
        assert!(schema_v01(&v), "{name}: v0.1 schema rejects");
        assert!(
            !schema_v02(&v),
            "{name}: v0.2 schema must reject version 0.1"
        );
        let text = std::fs::read_to_string(&path).expect("read");
        let intent = CreativeIntent::from_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            intent.version, "0.1",
            "{name}: conversion keeps the legacy version"
        );
        intent
            .validate()
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(compile_ok(&v), "{name}: compile fails");
    }
}

#[test]
fn v0_1_conversion_is_lossless() {
    for path in v01_files() {
        let name = path.display().to_string();
        let text = std::fs::read_to_string(&path).expect("read");
        let old: v0_1::CreativeIntent = serde_json::from_str(&text).expect("v0.1 parses");
        let converted = CreativeIntent::try_from(old.clone()).expect("converts");
        // Same wire form (both skip absent optionals and print defaults).
        assert_eq!(
            serde_json::to_value(&converted).expect("ser"),
            serde_json::to_value(&old).expect("ser"),
            "{name}: conversion changed the document"
        );
    }
}

#[test]
fn v0_1_phrase_asset_is_dropped_and_object_needs_asset() {
    let mut v = v01_doc();
    v["beats"][0]["primary"] = json!({ "kind": "phrase", "value": "hi", "asset": "rocket" });
    let i: CreativeIntent = serde_json::from_value(v).expect("parses");
    assert_eq!(
        i.beats[0].primary,
        Subject::Phrase(Atom {
            value: Some("hi".into()),
            meaning: None
        })
    );

    let mut v = v01_doc();
    let second = json!({
        "purpose": "emphasize", "statement": "s", "primary": { "kind": "object" }
    });
    v["beats"].as_array_mut().expect("array").push(second);
    assert_eq!(parse_err(&v), "beat 2: object subject needs an 'asset'");
    assert_eq!(
        CreativeIntent::from_json(&v.to_string())
            .expect_err("no asset")
            .to_string(),
        "beat 2: object subject needs an 'asset'"
    );
}

#[test]
fn v0_1_documents_using_v0_2_features_are_rejected() {
    let cases: Vec<(&str, Value, &str)> = vec![
        ("collection", collection(items(3)), "kind 'collection'"),
        ("state_change", state_change(), "kind 'state_change'"),
        (
            "derived_metric",
            ratio(json!(1), json!(2)),
            "kind 'derived_metric'",
        ),
    ];
    for (name, subject, needle) in cases {
        for slot in ["primary", "secondary"] {
            let mut v = v01_doc();
            v["version"] = json!("0.1");
            v["beats"][0][slot] = subject.clone();
            let msg = parse_err(&v);
            assert!(
                msg.contains("beat 1") && msg.contains(slot) && msg.contains(needle),
                "{name}/{slot}: {msg}"
            );
            assert!(
                msg.contains("requires \"version\": \"0.2\""),
                "{name}: {msg}"
            );
            let text_msg = CreativeIntent::from_json(&v.to_string())
                .expect_err("rejected")
                .to_string();
            assert_eq!(text_msg, msg, "{name}/{slot}: from_json and serde differ");
            assert!(!schema_v01(&v), "{name}/{slot}: v0.1 schema accepts");
        }
    }
    let mut v = v01_doc();
    v["beats"][0]["relationship"] = json!("accumulate");
    let msg = parse_err(&v);
    assert!(
        msg.contains("relationship 'accumulate'") && msg.contains("requires \"version\": \"0.2\""),
        "{msg}"
    );
    assert!(!schema_v01(&v));
    assert_eq!(
        CreativeIntent::from_json(&v.to_string())
            .expect_err("rejected")
            .to_string(),
        msg
    );
}

#[test]
fn directly_constructed_v0_1_intent_with_v0_2_features_fails_validation_and_compile() {
    let mut i = base_intent();
    i.version = "0.1".into();
    i.beats[0].primary = Subject::StateChange(StateChange {
        entity: "e".into(),
        from: "a".into(),
        to: "b".into(),
        meaning: None,
    });
    i.beats[0].relationship = Some(Relationship::Accumulate);
    let errs = i.validate().expect_err("features require 0.2");
    assert_eq!(errs.len(), 2, "{errs:?}");
    assert!(errs
        .iter()
        .all(|e| e.contains("requires \"version\": \"0.2\"")));

    let err = compile(
        &i,
        &StyleProfile::default(),
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .expect_err("compile rejects");
    assert!(matches!(err, CompileError::Invalid(_)), "{err}");
    assert!(
        err.to_string().starts_with("invalid intent: beat 1:"),
        "{err}"
    );

    // The same intent as 0.2 is fine as far as validation goes.
    i.version = "0.2".into();
    assert!(i.validate().is_ok());
}

#[test]
fn compile_reports_invalid_structured_intent_before_composing() {
    let mut i = intent_with_metric(1.0, 0.0);
    i.version = "0.2".into();
    let err = compile(
        &i,
        &StyleProfile::default(),
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .expect_err("zero denominator");
    assert_eq!(
        err.to_string(),
        "invalid intent: beat 1: primary.denominator.value must not be 0"
    );
}

#[test]
fn v0_2_document_is_accepted_and_compiles_when_atomic() {
    let d = doc(json!({ "kind": "number", "value": "120", "meaning": "passengers" }));
    assert!(schema_v02(&d) && rust_ok(&d) && compile_ok(&d));
    let i = CreativeIntent::from_json(&d.to_string()).expect("parses");
    assert_eq!(i.version, "0.2");
    assert_eq!(i.format, Format::Vertical);
    assert_eq!(i.beats[0].energy, Energy::Building);
    assert_eq!(i.beats[0].continuity, Continuity::None);
}

#[test]
fn unsupported_or_missing_versions_are_rejected() {
    for (name, version) in [
        ("0.3", Some(json!("0.3"))),
        ("0.1.0", Some(json!("0.1.0"))),
        ("empty", Some(json!(""))),
        ("number", Some(json!(0.2))),
        ("null", Some(Value::Null)),
        ("missing", None),
    ] {
        let mut d = doc(json!({ "kind": "phrase", "value": "x" }));
        match version {
            Some(v) => d["version"] = v,
            None => {
                d.as_object_mut().expect("object").remove("version");
            }
        }
        assert!(!schema_v02(&d), "{name}: v0.2 schema accepts");
        assert!(!schema_v01(&d), "{name}: v0.1 schema accepts");
        let msg = parse_err(&d);
        assert!(
            msg.starts_with("unsupported CreativeIntent version '")
                && msg.ends_with("(expected \"0.2\", or legacy \"0.1\")"),
            "{name}: {msg}"
        );
        let text_msg = CreativeIntent::from_json(&d.to_string())
            .expect_err("rejected")
            .to_string();
        assert_eq!(text_msg, msg, "{name}: from_json and serde differ");
    }
    let msg = parse_err(&doc_with(|d| d["version"] = json!("0.3")));
    assert!(msg.contains("'0.3'"), "{msg}");

    // A directly constructed intent with a bad version fails validate and compile.
    let mut i = base_intent();
    i.version = "0.3".into();
    assert!(i.validate().is_err());
    let err = compile(
        &i,
        &StyleProfile::default(),
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .expect_err("version");
    assert!(matches!(err, CompileError::Version(_)));
}

#[test]
fn syntax_errors_keep_line_and_column() {
    let err = CreativeIntent::from_json("{\n  \"version\": \"0.2\",\n  oops\n}").expect_err("bad");
    assert!(err.line() >= 3, "line {}", err.line());
    // A type error in a v0.1 / v0.2 document also carries a position (strict text parse).
    for version in ["0.1", "0.2"] {
        let text = format!("{{\n\"version\": \"{version}\",\n\"title\": 5,\n\"beats\": []\n}}");
        let err = CreativeIntent::from_json(&text).expect_err("bad");
        assert!(err.line() > 0, "{version}: no position in {err}");
    }
}

// ---------------------------------------------------------------------------
// Entry points agree
// ---------------------------------------------------------------------------

fn fixtures() -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    for path in v01_files() {
        out.push((path.display().to_string(), load(path)));
    }
    let structured = [
        ("collection", collection(items(3))),
        ("collection_1", collection(items(1))),
        ("state_change", state_change()),
        ("ratio", ratio(json!(1), json!(2))),
        ("ratio_zero", ratio(json!(1), json!(0))),
        ("ratio_string", ratio(json!("1,000"), json!(2))),
        (
            "nested",
            collection(vec![collection(items(2)), collection(items(2))]),
        ),
        ("phrase_extra", json!({ "kind": "phrase", "x": 1 })),
        ("phrase", json!({ "kind": "phrase", "value": "hi" })),
        (
            "object",
            json!({ "kind": "object", "asset": "shopping_basket" }),
        ),
    ];
    for (name, subject) in structured {
        out.push((format!("v0.2 {name}"), doc(subject.clone())));
        let mut old = doc(subject);
        old["version"] = json!("0.1");
        out.push((format!("v0.1 {name}"), old));
    }
    for v in ["0.3", "", "0.2.0"] {
        out.push((
            format!("version {v:?}"),
            doc_with(|d| d["version"] = json!(v)),
        ));
    }
    out.push(("root array".into(), json!([])));
    out.push(("root null".into(), Value::Null));
    out.push((
        "extra top-level".into(),
        doc_with(|d| d["duration"] = json!(5)),
    ));
    out.push(("v0.1 extra top-level".into(), {
        let mut v = v01_doc();
        v["duration"] = json!(5);
        v
    }));
    out
}

#[test]
fn from_json_and_serde_agree_on_every_fixture() {
    let fixtures = fixtures();
    assert!(fixtures.len() > 30);
    for (name, v) in fixtures {
        let via_json = CreativeIntent::from_json(&v.to_string());
        let via_value = serde_json::from_value::<CreativeIntent>(v.clone());
        let via_str = serde_json::from_str::<CreativeIntent>(&v.to_string());
        match (&via_json, &via_value, &via_str) {
            (Ok(a), Ok(b), Ok(c)) => {
                assert_eq!(a, b, "{name}");
                assert_eq!(a, c, "{name}");
            }
            (Err(_), Err(_), Err(_)) => {}
            _ => panic!(
                "{name}: entry points disagree: json={:?} value={:?} str={:?}",
                via_json.map(|_| ()),
                via_value.map(|_| ()),
                via_str.map(|_| ())
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Round trip
// ---------------------------------------------------------------------------

fn base_intent() -> CreativeIntent {
    CreativeIntent {
        version: "0.2".into(),
        title: "t".into(),
        format: Format::Vertical,
        beats: vec![beat(Subject::Phrase(Atom {
            value: Some("hi".into()),
            meaning: None,
        }))],
    }
}

fn beat(primary: Subject) -> Beat {
    Beat {
        purpose: Purpose::Emphasize,
        statement: "A statement here".into(),
        primary,
        secondary: None,
        relationship: None,
        energy: Energy::Building,
        continuity: Continuity::None,
        keyword: None,
        narration: None,
    }
}

fn atom(v: &str, m: &str) -> Atom {
    Atom {
        value: Some(v.into()),
        meaning: Some(m.into()),
    }
}

#[test]
fn v0_2_round_trips_through_serialize_and_parse() {
    let mut accumulate = beat(Subject::Collection(Collection {
        items: vec![
            CollectionItem::Phrase(atom("Coffee", "drink")),
            CollectionItem::Number(atom("₹120", "coffee")),
            CollectionItem::Object(ObjectAtom {
                asset: "shopping_basket".into(),
                value: Some("₹6,900".into()),
                meaning: None,
            }),
        ],
        meaning: Some("drinks".into()),
    }));
    accumulate.purpose = Purpose::Contrast;
    accumulate.relationship = Some(Relationship::Accumulate);
    accumulate.secondary = Some(Subject::Number(atom("₹9,000", "a month")));
    accumulate.energy = Energy::Impact;
    accumulate.continuity = Continuity::CarrySecondary;
    accumulate.keyword = Some("total".into());

    let mut compare = beat(Subject::StateChange(StateChange {
        entity: "delivery time".into(),
        from: "three days".into(),
        to: "same day".into(),
        meaning: Some("faster".into()),
    }));
    compare.purpose = Purpose::Compare;
    compare.secondary = Some(Subject::DerivedMetric(DerivedMetric {
        operation: Operation::Ratio,
        numerator: NumericTerm {
            value: 80.0,
            meaning: "sign-ups".into(),
        },
        denominator: NumericTerm {
            value: 1000.0,
            meaning: "visitors".into(),
        },
        format: MetricFormat::PerThousand,
        meaning: Some("sign-up rate".into()),
    }));

    let intent = CreativeIntent {
        version: "0.2".into(),
        title: "round_trip".into(),
        format: Format::Landscape,
        beats: vec![
            accumulate,
            compare,
            beat(Subject::Object(ObjectAtom {
                asset: "shopping_basket".into(),
                value: None,
                meaning: None,
            })),
        ],
    };
    intent.validate().expect("valid");

    let value = serde_json::to_value(&intent).expect("serialize");
    assert!(
        schema_v02(&value),
        "serialized form must satisfy the v0.2 schema\n{value:#}"
    );
    let text = serde_json::to_string_pretty(&intent).expect("serialize");
    let again = CreativeIntent::from_json(&text).expect("re-parse");
    assert_eq!(again, intent);
    assert_eq!(
        serde_json::from_value::<CreativeIntent>(value).expect("value"),
        intent
    );
    assert_eq!(
        serde_json::to_string_pretty(&again).expect("serialize"),
        text
    );
}

#[test]
fn converted_v0_1_documents_round_trip_as_v0_1() {
    for path in v01_files() {
        let name = path.display().to_string();
        let intent =
            CreativeIntent::from_json(&std::fs::read_to_string(&path).expect("read")).expect(&name);
        let text = serde_json::to_string(&intent).expect("ser");
        let again = CreativeIntent::from_json(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(again, intent, "{name}");
    }
}

// ---------------------------------------------------------------------------
// Schema => compiles (atomic v0.2 documents; structured composers are tested elsewhere)
// ---------------------------------------------------------------------------

#[test]
fn schema_valid_atomic_documents_compile() {
    let atomic = [
        json!({ "kind": "phrase", "value": "hi" }),
        json!({ "kind": "phrase", "meaning": "only meaning" }),
        json!({ "kind": "phrase" }),
        json!({ "kind": "number", "value": "120", "meaning": "passengers" }),
        json!({ "kind": "object", "asset": "shopping_basket" }),
        json!({ "kind": "object", "asset": "shopping_basket", "value": "₹9" }),
    ];
    let mut checked = 0;
    for purpose in ["emphasize", "compare", "contrast", "reveal", "explain"] {
        for relationship in [None, Some("grow"), Some("compress"), Some("replace")] {
            for primary in &atomic {
                let mut d = doc(primary.clone());
                d["beats"][0]["purpose"] = json!(purpose);
                d["beats"][0]["secondary"] = json!({ "kind": "number", "value": "20" });
                if let Some(r) = relationship {
                    d["beats"][0]["relationship"] = json!(r);
                }
                assert!(schema_v02(&d), "fixture must be schema-valid\n{d:#}");
                assert!(compile_ok(&d), "schema-valid but compile fails\n{d:#}");
                checked += 1;
            }
        }
    }
    assert!(checked >= 100);
}

#[test]
fn schema_rejects_what_validate_rejects_for_structured_subjects() {
    // Every rule validate() enforces on structured subjects is mirrored in the schema.
    let bad = [
        doc(collection(items(0))),
        doc(collection(items(1))),
        doc(collection(items(7))),
        doc({
            let mut s = state_change();
            s["entity"] = json!(" ");
            s
        }),
        doc(ratio(json!(1), json!(0))),
        doc({
            let mut m = ratio(json!(1), json!(2));
            m["denominator"]["meaning"] = json!("");
            m
        }),
    ];
    for d in bad {
        assert!(!schema_v02(&d), "schema accepts\n{d:#}");
        assert!(!rust_ok(&d), "Rust accepts\n{d:#}");
    }
}

// ---------------------------------------------------------------------------
// v0.1 ≡ v0.2: a v0.1 document compiles exactly like the same document
// declared as v0.2. (Until 0.4 this pinned byte-identical v0.1 output; the
// 0.4 scene lifecycle deliberately improves every compiled beat, so the
// invariant is now parser equivalence — see DECISIONS.md #29.)
// ---------------------------------------------------------------------------

#[test]
fn v0_1_documents_compile_like_their_v0_2_equivalent() {
    let files: Vec<PathBuf> = v01_files();
    assert!(
        files.len() >= 9,
        "expected the v0.1 examples and benchmarks"
    );
    for path in files {
        let text = std::fs::read_to_string(&path).expect("read");
        let v01 = CreativeIntent::from_json(&text).expect("parses as v0.1");
        let mut doc: serde_json::Value = serde_json::from_str(&text).expect("json");
        doc["version"] = serde_json::json!("0.2");
        let v02 = CreativeIntent::from_json(&doc.to_string()).expect("parses as v0.2");
        let build = |i: &CreativeIntent| {
            let project = compile(
                i,
                &StyleProfile::default(),
                &AssetLibrary::new(repo().join("assets")),
                &ApproxMeasure,
            )
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            serde_json::to_string(&project).expect("serialize")
        };
        assert_eq!(
            build(&v01),
            build(&v02),
            "{}: v0.1 and v0.2 compile differently",
            path.display()
        );
    }
}
