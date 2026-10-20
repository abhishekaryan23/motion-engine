//! Public contract guard: the checked-in JSON Schemas (what AI consumers see)
//! and the Rust types/compiler (what actually runs) must agree.
//!
//! Key guarantee: the schema never claims valid what Rust rejects
//! (`schema_ok => compile_ok` for intents, `schema_ok => serde_ok` for styles).
//! For deserialization-level rules the two are exactly equal. For compile-level
//! rules (version, non-empty beats, object assets) the schema is intentionally
//! stricter than serde, but matches `compile`.

use std::path::PathBuf;

use jsonschema::Validator;
use motion_core::{compile, ApproxMeasure, AssetLibrary, CreativeIntent, StyleProfile};
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

/// The legacy v0.1 schema (the `base_intent` documents below are v0.1).
fn intent_validator() -> Validator {
    validator("creative-intent-v0.1.schema.json")
}

fn intent_v02_validator() -> Validator {
    validator("creative-intent-v0.2.schema.json")
}

fn style_validator() -> Validator {
    validator("style-profile-v0.1.schema.json")
}

fn intent_schema_ok(v: &Value) -> bool {
    intent_validator().is_valid(v)
}

fn intent_serde_ok(v: &Value) -> bool {
    serde_json::from_value::<CreativeIntent>(v.clone()).is_ok()
}

fn intent_compile_ok(v: &Value) -> bool {
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

fn style_schema_ok(v: &Value) -> bool {
    style_validator().is_valid(v)
}

fn style_serde_ok(v: &Value) -> bool {
    serde_json::from_value::<StyleProfile>(v.clone()).is_ok()
}

// ---------------------------------------------------------------------------
// Base documents and mutation helpers
// ---------------------------------------------------------------------------

fn base_intent() -> Value {
    json!({
        "version": "0.1",
        "title": "contract_test",
        "format": "vertical",
        "beats": [{
            "purpose": "emphasize",
            "statement": "One train carried 120 passengers",
            "primary": { "kind": "number", "value": "120", "meaning": "passengers" },
            "secondary": { "kind": "phrase", "value": "on its very first run." },
            "energy": "calm"
        }]
    })
}

fn base_style() -> Value {
    json!({
        "material": "paper",
        "typography_style": "grotesk_serif",
        "depth": "layered",
        "camera_style": "slow_push",
        "texture_style": "subtle_print",
        "accent_role": "cobalt",
        "seed": 1
    })
}

/// Apply a mutation to a copy of the base intent.
fn intent(f: impl FnOnce(&mut Value)) -> Value {
    let mut v = base_intent();
    f(&mut v);
    v
}

fn style(f: impl FnOnce(&mut Value)) -> Value {
    let mut v = base_style();
    f(&mut v);
    v
}

fn beat(v: &mut Value) -> &mut Value {
    &mut v["beats"][0]
}

fn remove(v: &mut Value, key: &str) {
    v.as_object_mut().expect("object").remove(key);
}

/// Cases where schema and serde must agree exactly (deserialization-level).
/// `(name, document, expected verdict)`.
fn intent_cases() -> Vec<(&'static str, Value, bool)> {
    let mut c: Vec<(&'static str, Value, bool)> = Vec::new();
    let mut add = |name: &'static str, v: Value, ok: bool| c.push((name, v, ok));

    add("base", base_intent(), true);

    // 2. invalid enum values
    add(
        "purpose_summarize",
        intent(|v| beat(v)["purpose"] = json!("summarize")),
        false,
    );
    add(
        "energy_fast",
        intent(|v| beat(v)["energy"] = json!("fast")),
        false,
    );
    add(
        "continuity_carry_both",
        intent(|v| beat(v)["continuity"] = json!("carry_both")),
        false,
    );
    add(
        "relationship_shrink",
        intent(|v| beat(v)["relationship"] = json!("shrink")),
        false,
    );
    add(
        "format_portrait",
        intent(|v| v["format"] = json!("portrait")),
        false,
    );
    add(
        "subject_kind_image",
        intent(|v| beat(v)["primary"]["kind"] = json!("image")),
        false,
    );
    // Enum values are case-sensitive snake_case.
    add(
        "purpose_wrong_case",
        intent(|v| beat(v)["purpose"] = json!("Emphasize")),
        false,
    );
    add(
        "continuity_kebab_case",
        intent(|v| beat(v)["continuity"] = json!("carry-primary")),
        false,
    );

    // 3. missing required
    add("missing_version", intent(|v| remove(v, "version")), false);
    add("missing_title", intent(|v| remove(v, "title")), false);
    add("missing_beats", intent(|v| remove(v, "beats")), false);
    add(
        "missing_beat_purpose",
        intent(|v| remove(beat(v), "purpose")),
        false,
    );
    add(
        "missing_beat_statement",
        intent(|v| remove(beat(v), "statement")),
        false,
    );
    add(
        "missing_beat_primary",
        intent(|v| remove(beat(v), "primary")),
        false,
    );
    add(
        "missing_subject_kind",
        intent(|v| remove(&mut beat(v)["primary"], "kind")),
        false,
    );

    // 4. unknown fields
    add(
        "unknown_top_level_duration",
        intent(|v| v["duration"] = json!(5)),
        false,
    );
    add(
        "unknown_beat_font_size",
        intent(|v| beat(v)["font_size"] = json!(48)),
        false,
    );
    add(
        "unknown_subject_x",
        intent(|v| beat(v)["primary"]["x"] = json!(10)),
        false,
    );
    add(
        "unknown_secondary_subject_field",
        intent(|v| beat(v)["secondary"]["color"] = json!("red")),
        false,
    );

    // 5. accepted optional forms
    add(
        "secondary_null",
        intent(|v| beat(v)["secondary"] = Value::Null),
        true,
    );
    add(
        "relationship_null",
        intent(|v| beat(v)["relationship"] = Value::Null),
        true,
    );
    add(
        "keyword_null",
        intent(|v| beat(v)["keyword"] = Value::Null),
        true,
    );
    add(
        "subject_value_null",
        intent(|v| beat(v)["primary"]["value"] = Value::Null),
        true,
    );
    add(
        "subject_meaning_null",
        intent(|v| beat(v)["primary"]["meaning"] = Value::Null),
        true,
    );
    add("format_omitted", intent(|v| remove(v, "format")), true);
    add(
        "energy_omitted",
        intent(|v| remove(beat(v), "energy")),
        true,
    );
    add(
        "continuity_omitted",
        intent(|v| remove(beat(v), "continuity")),
        true,
    );
    add(
        "secondary_omitted",
        intent(|v| remove(beat(v), "secondary")),
        true,
    );
    add(
        "all_optionals_present",
        intent(|v| {
            v["format"] = json!("square");
            let b = beat(v);
            b["purpose"] = json!("contrast");
            b["relationship"] = json!("compress");
            b["energy"] = json!("impact");
            b["continuity"] = json!("carry_secondary");
            b["keyword"] = json!("queue");
            b["secondary"] = json!({ "kind": "number", "value": "20", "meaning": "noon" });
        }),
        true,
    );
    add(
        "every_purpose_reveal",
        intent(|v| beat(v)["purpose"] = json!("reveal")),
        true,
    );
    add(
        "every_purpose_explain",
        intent(|v| beat(v)["purpose"] = json!("explain")),
        true,
    );
    add(
        "every_purpose_compare",
        intent(|v| beat(v)["purpose"] = json!("compare")),
        true,
    );
    add(
        "format_landscape",
        intent(|v| v["format"] = json!("landscape")),
        true,
    );
    add(
        "relationship_carry",
        intent(|v| beat(v)["relationship"] = json!("carry")),
        true,
    );
    add(
        "subject_asset_ignored_for_phrase",
        intent(|v| {
            beat(v)["primary"] = json!({ "kind": "phrase", "value": "hi", "asset": "rocket" })
        }),
        true,
    );

    // 6. wrong types
    add(
        "statement_number",
        intent(|v| beat(v)["statement"] = json!(5)),
        false,
    );
    add("beats_object", intent(|v| v["beats"] = json!({})), false);
    add(
        "primary_string",
        intent(|v| beat(v)["primary"] = json!("text")),
        false,
    );
    add("title_number", intent(|v| v["title"] = json!(7)), false);
    add(
        "version_number",
        intent(|v| v["version"] = json!(0.1)),
        false,
    );
    add(
        "keyword_number",
        intent(|v| beat(v)["keyword"] = json!(3)),
        false,
    );
    add("root_array", json!([]), false);
    add("root_null", Value::Null, false);
    add("beat_null", intent(|v| v["beats"] = json!([null])), false);

    c
}

/// Cases where the schema is intentionally stricter than serde because
/// `compile` enforces the rule. `(name, document)`; the schema and `compile`
/// must both reject.
fn compile_level_rejects() -> Vec<(&'static str, Value)> {
    let obj = |asset: Option<Value>| {
        intent(|v| {
            let mut p = json!({ "kind": "object" });
            if let Some(a) = asset {
                p["asset"] = a;
            }
            beat(v)["primary"] = p;
        })
    };
    vec![
        ("version_0_1_0", intent(|v| v["version"] = json!("0.1.0"))),
        ("version_empty", intent(|v| v["version"] = json!(""))),
        ("beats_empty", intent(|v| v["beats"] = json!([]))),
        ("object_without_asset", obj(None)),
        ("object_asset_null", obj(Some(Value::Null))),
        ("object_asset_empty", obj(Some(json!("")))),
        (
            "object_asset_path_traversal",
            obj(Some(json!("../fonts/x"))),
        ),
        (
            "secondary_object_without_asset",
            intent(|v| beat(v)["secondary"] = json!({ "kind": "object" })),
        ),
        (
            "second_beat_object_without_asset",
            intent(|v| {
                let second = json!({
                    "purpose": "emphasize",
                    "statement": "Second",
                    "primary": { "kind": "object" }
                });
                v["beats"].as_array_mut().unwrap().push(second);
            }),
        ),
    ]
}

fn style_cases() -> Vec<(&'static str, Value, bool)> {
    let mut c: Vec<(&'static str, Value, bool)> = Vec::new();
    let mut add = |name: &'static str, v: Value, ok: bool| c.push((name, v, ok));

    add("base", base_style(), true);
    add("empty_object", json!({}), true);

    // invalid enums
    add(
        "accent_green",
        style(|v| v["accent_role"] = json!("green")),
        false,
    );
    add("depth_deep", style(|v| v["depth"] = json!("deep")), false);
    add(
        "camera_zoom",
        style(|v| v["camera_style"] = json!("zoom")),
        false,
    );
    add(
        "family_unknown",
        style(|v| v["family"] = json!("brutalist")),
        false,
    );
    add(
        "material_glass",
        style(|v| v["material"] = json!("glass")),
        false,
    );
    add(
        "typography_serif",
        style(|v| v["typography_style"] = json!("serif")),
        false,
    );
    add(
        "texture_medium",
        style(|v| v["texture_style"] = json!("medium")),
        false,
    );
    add(
        "accent_wrong_case",
        style(|v| v["accent_role"] = json!("Cobalt")),
        false,
    );

    // unknown fields
    add("unknown_font", style(|v| v["font"] = json!("Arial")), false);
    add(
        "unknown_duration",
        style(|v| v["duration"] = json!(5)),
        false,
    );

    // wrong types
    add("seed_negative", style(|v| v["seed"] = json!(-1)), false);
    add("seed_string", style(|v| v["seed"] = json!("7")), false);
    add("seed_bool", style(|v| v["seed"] = json!(true)), false);
    add("seed_null", style(|v| v["seed"] = Value::Null), false);
    add("depth_number", style(|v| v["depth"] = json!(1)), false);
    add("root_null", Value::Null, false);

    // accepted forms
    add("seed_zero", style(|v| v["seed"] = json!(0)), true);
    add(
        "seed_u64_max",
        style(|v| v["seed"] = json!(18446744073709551615u64)),
        true,
    );
    add("seed_omitted", style(|v| remove(v, "seed")), true);
    add("accent_omitted", style(|v| remove(v, "accent_role")), true);
    add(
        "family_minimal",
        style(|v| v["family"] = json!("minimal")),
        true,
    );
    add(
        "all_alternate_values",
        json!({
            "family": "minimal",
            "material": "flat",
            "typography_style": "condensed_mono",
            "depth": "flat",
            "camera_style": "drift",
            "texture_style": "heavy_print",
            "accent_role": "acid",
            "seed": 42
        }),
        true,
    );
    c
}

// ---------------------------------------------------------------------------
// 1. Public examples
// ---------------------------------------------------------------------------

fn public_files(suffix: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(repo().join("examples/public"))
        .expect("examples/public exists")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().ends_with(suffix))
        .collect();
    files.sort();
    files
}

#[test]
fn public_intent_examples_are_valid_everywhere() {
    let files = public_files(".intent.json");
    assert!(files.len() >= 3, "expected the public intent examples");
    for path in files {
        let v = load(path.clone());
        let name = path.display();
        // Each example is checked against the schema of the version it declares.
        let schema_ok = if v["version"] == "0.2" {
            intent_v02_validator().is_valid(&v)
        } else {
            intent_schema_ok(&v)
        };
        assert!(schema_ok, "{name}: schema rejects");
        assert!(intent_serde_ok(&v), "{name}: serde rejects");
        assert!(intent_compile_ok(&v), "{name}: compile fails");
    }
}

#[test]
fn public_style_examples_are_valid_everywhere() {
    let files = public_files(".style.json");
    assert!(!files.is_empty(), "expected a public style example");
    for path in files {
        let v = load(path.clone());
        let name = path.display();
        assert!(style_schema_ok(&v), "{name}: schema rejects");
        assert!(style_serde_ok(&v), "{name}: serde rejects");
    }
    let empty = json!({});
    assert!(style_schema_ok(&empty) && style_serde_ok(&empty));
}

// ---------------------------------------------------------------------------
// 2-6. Deserialization-level agreement
// ---------------------------------------------------------------------------

#[test]
fn intent_cases_have_expected_verdict_in_schema_and_serde() {
    for (name, v, expected) in intent_cases() {
        assert_eq!(
            intent_schema_ok(&v),
            expected,
            "intent case '{name}': schema verdict\n{v:#}"
        );
        assert_eq!(
            intent_serde_ok(&v),
            expected,
            "intent case '{name}': serde verdict\n{v:#}"
        );
    }
}

#[test]
fn style_cases_have_expected_verdict_in_schema_and_serde() {
    for (name, v, expected) in style_cases() {
        assert_eq!(
            style_schema_ok(&v),
            expected,
            "style case '{name}': schema verdict\n{v:#}"
        );
        assert_eq!(
            style_serde_ok(&v),
            expected,
            "style case '{name}': serde verdict\n{v:#}"
        );
    }
}

#[test]
fn accepted_intent_cases_compile() {
    for (name, v, expected) in intent_cases() {
        if expected {
            assert!(
                intent_compile_ok(&v),
                "intent case '{name}' fails compile\n{v:#}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Compile-level rules: schema stricter than serde, matching `compile`
// ---------------------------------------------------------------------------

#[test]
fn compile_level_rules_are_enforced_by_schema_and_compile() {
    for (name, v) in compile_level_rejects() {
        assert!(
            !intent_schema_ok(&v),
            "case '{name}': schema accepts\n{v:#}"
        );
        assert!(
            !intent_compile_ok(&v),
            "case '{name}': compile accepts\n{v:#}"
        );
    }
}

#[test]
fn compile_level_rules_that_serde_alone_accepts() {
    // Pin the intentional gap: serde is looser than the schema/compile here.
    // (Unsupported versions and object subjects without an asset are rejected at parse
    // time since the version-dispatched parser, so they are no longer in this gap.)
    let name = "beats_empty";
    let (_, v) = compile_level_rejects()
        .into_iter()
        .find(|(n, _)| *n == name)
        .expect("case exists");
    assert!(intent_serde_ok(&v), "'{name}' should deserialize");
}

#[test]
fn bundled_object_asset_is_accepted_and_compiles() {
    let v =
        intent(|v| beat(v)["primary"] = json!({ "kind": "object", "asset": "shopping_basket" }));
    assert!(intent_schema_ok(&v));
    assert!(intent_serde_ok(&v));
    assert!(intent_compile_ok(&v));

    // Also as the counter-subject of a contrast beat, with its stamped figure.
    let v = intent(|v| {
        let b = beat(v);
        b["purpose"] = json!("contrast");
        b["primary"] = json!({ "kind": "number", "value": "5", "meaning": "before" });
        b["secondary"] = json!({
            "kind": "object", "asset": "shopping_basket", "value": "20"
        });
        b["relationship"] = json!("grow");
    });
    assert!(intent_schema_ok(&v));
    assert!(intent_compile_ok(&v));
}

// ---------------------------------------------------------------------------
// 8. The key invariant
// ---------------------------------------------------------------------------

#[test]
fn schema_never_claims_valid_what_rust_rejects() {
    let mut docs: Vec<(&str, Value)> = intent_cases().into_iter().map(|(n, v, _)| (n, v)).collect();
    docs.extend(compile_level_rejects());
    for (name, v) in &docs {
        if intent_schema_ok(v) {
            assert!(
                intent_compile_ok(v),
                "INVARIANT VIOLATED for intent '{name}': schema accepts, Rust rejects\n{v:#}"
            );
        }
    }
    for (name, v, _) in style_cases() {
        if style_schema_ok(&v) {
            assert!(
                style_serde_ok(&v),
                "INVARIANT VIOLATED for style '{name}': schema accepts, serde rejects\n{v:#}"
            );
        }
    }
}

#[test]
fn schema_and_serde_agree_on_every_deserialization_level_case() {
    for (name, v, _) in intent_cases() {
        assert_eq!(
            intent_schema_ok(&v),
            intent_serde_ok(&v),
            "intent '{name}': schema and serde disagree\n{v:#}"
        );
    }
    for (name, v, _) in style_cases() {
        assert_eq!(
            style_schema_ok(&v),
            style_serde_ok(&v),
            "style '{name}': schema and serde disagree\n{v:#}"
        );
    }
}

// ---------------------------------------------------------------------------
// 9. Edge cases
//
// Each test pins CURRENT behaviour so a change (in the schema, serde or the
// validator crate) is noticed. Where the invariant `schema_ok => rust_ok` is
// violated, the test is marked `// KNOWN LIMITATION:`.
// ---------------------------------------------------------------------------

/// Sanity: the ordinary integer seed is accepted by both.
#[test]
fn edge_seed_plain_integer_agrees() {
    let v: Value = serde_json::from_str(r#"{"seed": 7}"#).unwrap();
    assert!(style_schema_ok(&v) && style_serde_ok(&v));
}

// KNOWN LIMITATION: a JSON number with a zero fractional part or an exponent
// (`7.0`, `1e3`) is a valid JSON Schema "integer", so the schema accepts it,
// but serde's u64 refuses floats, so `motion-engine compile` rejects it.
// Consumers should emit plain integer literals for `seed`.
#[test]
fn edge_seed_float_with_integral_value() {
    for doc in [r#"{"seed": 7.0}"#, r#"{"seed": 1e3}"#] {
        let v: Value = serde_json::from_str(doc).unwrap();
        assert!(
            style_schema_ok(&v),
            "{doc}: schema now rejects (limitation fixed?)"
        );
        assert!(!style_serde_ok(&v), "{doc}: serde now accepts");
        assert!(serde_json::from_str::<StyleProfile>(doc).is_err(), "{doc}");
    }
}

// Seeds above u64::MAX: the schema carries an explicit `maximum`, so schema
// and Rust both reject them.
#[test]
fn edge_seed_beyond_u64() {
    let doc = r#"{"seed": 18446744073709551616}"#;
    let v: Value = serde_json::from_str(doc).unwrap();
    assert!(
        !style_schema_ok(&v),
        "schema must reject seeds above u64::MAX"
    );
    assert!(!style_serde_ok(&v));
    assert!(serde_json::from_str::<StyleProfile>(doc).is_err());
    let max: Value = serde_json::from_str(r#"{"seed": 18446744073709551615}"#).unwrap();
    assert!(
        style_schema_ok(&max) && style_serde_ok(&max),
        "u64::MAX itself is valid"
    );
}

// KNOWN LIMITATION: duplicate object keys. serde's derived deserializer
// rejects them when it reads the text (as the CLI does), but a schema
// validator only sees the parsed value, in which the last key wins. JSON
// Schema cannot express "no duplicate keys".
#[test]
fn edge_duplicate_json_keys() {
    let doc = r#"{"version":"0.2","version":"0.1","title":"t","beats":[
        {"purpose":"emphasize","statement":"s","primary":{"kind":"phrase","value":"v"}}]}"#;
    let parsed: Value = serde_json::from_str(doc).unwrap();
    assert!(
        intent_schema_ok(&parsed),
        "schema now rejects (limitation fixed?)"
    );
    assert!(serde_json::from_str::<CreativeIntent>(doc).is_err());
}

// Intentional gap in the SAFE direction: serde's derived struct deserializer
// also accepts a JSON array (positional fields), and for StyleProfile every
// field defaults, so `[]` deserializes. The schema requires an object, which
// is the documented form. (Not a violation: schema is stricter.)
#[test]
fn edge_style_array_form_is_rejected_by_schema_only() {
    let v = json!([]);
    assert!(!style_schema_ok(&v));
    assert!(style_serde_ok(&v));
}

// Object asset names are resolved on the filesystem. On case-insensitive
// filesystems (macOS default) `Shopping_Basket` resolves; on Linux it does
// not. The schema enum is case-sensitive, i.e. stricter and portable.
#[test]
fn edge_object_asset_case_is_rejected_by_schema() {
    let v =
        intent(|v| beat(v)["primary"] = json!({ "kind": "object", "asset": "Shopping_Basket" }));
    assert!(!intent_schema_ok(&v));
}

// ---------------------------------------------------------------------------
// 10. Contract v0.2
//
// The v0.1 documents above keep their v0.1 verdicts. The same documents restated as
// v0.2 (version "0.2") must satisfy the same invariants against the v0.2 schema.
// Structured-subject cases live in tests/intent_v0_2.rs.
// ---------------------------------------------------------------------------

/// The same document declared as version "0.2".
fn as_v02(v: &Value) -> Value {
    let mut v = v.clone();
    if v.get("version") == Some(&json!("0.1")) {
        v["version"] = json!("0.2");
    }
    v
}

fn v02_schema_ok(v: &Value) -> bool {
    intent_v02_validator().is_valid(v)
}

#[test]
fn v0_1_documents_on_disk_stay_valid_against_v0_1_schema_only() {
    let v = base_intent();
    assert!(intent_schema_ok(&v) && intent_serde_ok(&v) && intent_compile_ok(&v));
    // The legacy schema rejects "0.2", the current one rejects "0.1".
    assert!(!v02_schema_ok(&v), "v0.2 schema must reject version 0.1");
    let w = as_v02(&v);
    assert!(v02_schema_ok(&w) && intent_serde_ok(&w) && intent_compile_ok(&w));
    assert!(!intent_schema_ok(&w), "v0.1 schema must reject version 0.2");
}

#[test]
fn v0_2_schema_and_serde_agree_on_every_deserialization_level_case() {
    for (name, v, _) in intent_cases() {
        let w = as_v02(&v);
        assert_eq!(
            v02_schema_ok(&w),
            intent_serde_ok(&w),
            "v0.2 restatement of '{name}': schema and serde disagree\n{w:#}"
        );
    }
}

#[test]
fn v0_2_schema_never_claims_valid_what_rust_rejects() {
    let mut docs: Vec<(&str, Value)> = intent_cases().into_iter().map(|(n, v, _)| (n, v)).collect();
    docs.extend(compile_level_rejects());
    for (name, v) in &docs {
        let w = as_v02(v);
        if v02_schema_ok(&w) {
            assert!(
                intent_compile_ok(&w),
                "INVARIANT VIOLATED for v0.2 restatement of '{name}': schema accepts, Rust rejects\n{w:#}"
            );
        }
    }
}

#[test]
fn v0_2_schema_accepts_and_compiles_the_atomic_restatement_of_accepted_cases() {
    for (name, v, expected) in intent_cases() {
        // v0.1 phrase subjects tolerated a stray `asset`; v0.2 (strict) does not.
        if !expected || name == "subject_asset_ignored_for_phrase" {
            continue;
        }
        let w = as_v02(&v);
        assert!(v02_schema_ok(&w), "'{name}': v0.2 schema rejects\n{w:#}");
        assert!(intent_compile_ok(&w), "'{name}': v0.2 compile fails\n{w:#}");
    }
}
