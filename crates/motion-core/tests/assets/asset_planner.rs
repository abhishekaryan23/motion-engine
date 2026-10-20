//! AssetPlanner rules (docs/ASSET_PIPELINE.md), manifest validation and the
//! serde contract of the asset boundary types. Intents are synthetic.

use std::path::PathBuf;

use motion_core::assets::{
    AssetManifest, AssetPlan, AssetRequest, AssetRole, AssetSource, Background, NegativeSpace,
    Presentation, Priority,
};
use motion_core::compiler::plan_assets;
use motion_core::intent::CreativeIntent;
use motion_core::style::StyleProfile;
use motion_core::{AssetLibrary, CompileError};
use serde_json::{json, Value};

fn repo_assets() -> AssetLibrary {
    AssetLibrary::new(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets"
    )))
}

/// A library root under target/ holding `library/receipt.png` and nothing else.
fn png_only_library() -> AssetLibrary {
    let root =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/asset_planner_test_library");
    let lib = root.join("library");
    std::fs::create_dir_all(&lib).expect("create test library");
    std::fs::write(lib.join("receipt.png"), b"png").expect("write png");
    AssetLibrary::new(root)
}

fn style_from(v: Value) -> StyleProfile {
    StyleProfile::from_json(&v.to_string()).expect("style parses")
}

fn intent(beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2",
        "title": "asset_planner_test",
        "beats": beats,
    }))
    .expect("intent parses")
}

fn plan_with(beats: Vec<Value>, style: &StyleProfile, lib: &AssetLibrary) -> AssetPlan {
    plan_assets(&intent(beats), style, lib).expect("plan")
}

fn plan(beats: Vec<Value>) -> AssetPlan {
    plan_with(beats, &StyleProfile::default(), &repo_assets())
}

fn phrase(value: &str) -> Value {
    json!({ "kind": "phrase", "value": value })
}

fn emphasize(primary: Value) -> Value {
    json!({ "purpose": "emphasize", "statement": "A short statement here", "primary": primary })
}

fn request<'a>(plan: &'a AssetPlan, id: &str) -> &'a AssetRequest {
    plan.requests
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no request {id} in {:?}", plan.requests))
}

#[test]
fn human_phrase_in_emphasize_beat_requests_a_hero_subject() {
    let p = plan(vec![emphasize(phrase(
        "office worker concentrating at a laptop",
    ))]);
    assert_eq!(p.requests.len(), 1);
    let r = request(&p, "beat_1.hero_subject");
    assert_eq!(r.beat, 1);
    assert_eq!(r.role, AssetRole::HeroSubject);
    assert_eq!(r.source, AssetSource::GeneratedImage);
    assert_eq!(r.presentation, Presentation::IsolatedCutout);
    assert_eq!(r.background, Background::Transparent);
    assert_eq!(r.priority, Priority::Optional);
    assert_eq!(r.composition, "type_image_interlock");
    assert_eq!(r.subject, "office worker concentrating at a laptop");
    assert!(matches!(
        r.negative_space,
        NegativeSpace::Left | NegativeSpace::Right
    ));
    assert!(r.library_asset.is_none());
    assert_eq!(p.beats[0].source, AssetSource::GeneratedImage);
    assert_eq!(p.beats[0].requests, vec!["beat_1.hero_subject".to_string()]);
}

#[test]
fn negative_space_follows_the_layout_variant() {
    // The variant is seed-derived; across many titles-free styles both sides occur
    // and always agree with the (deterministic) plan for the same seed.
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..64u64 {
        let style = style_from(json!({ "seed": seed }));
        let p = plan_with(
            vec![emphasize(phrase("a customer waiting"))],
            &style,
            &repo_assets(),
        );
        let side = p.requests[0].negative_space;
        assert!(matches!(side, NegativeSpace::Left | NegativeSpace::Right));
        seen.insert(format!("{side:?}"));
    }
    assert_eq!(seen.len(), 2, "both variants should be reachable: {seen:?}");
}

#[test]
fn face_phrase_becomes_a_portrait() {
    let p = plan(vec![emphasize(phrase("a tired face"))]);
    let r = request(&p, "beat_1.portrait");
    assert_eq!(r.role, AssetRole::Portrait);
    assert_eq!(r.source, AssetSource::GeneratedImage);
    assert_eq!(r.priority, Priority::Optional);
}

#[test]
fn human_subject_text_joins_value_and_meaning_and_collapses_whitespace() {
    let p = plan(vec![emphasize(
        json!({ "kind": "phrase", "value": "  a   tired\n commuter ", "meaning": " rush  hour " }),
    )]);
    assert_eq!(p.requests[0].subject, "a tired commuter, rush hour");
}

#[test]
fn human_detection_uses_whole_words() {
    // "manual" contains "man", "teammate" contains "team": neither is a person.
    let p = plan(vec![emphasize(phrase("manual override of teammate"))]);
    assert!(p
        .requests
        .iter()
        .all(|r| !matches!(r.role, AssetRole::HeroSubject | AssetRole::Portrait)));
}

#[test]
fn derived_metric_beat_needs_no_image() {
    let p = plan(vec![json!({
        "purpose": "reveal",
        "statement": "Most visitors never sign up",
        "primary": {
            "kind": "derived_metric",
            "numerator": { "value": 80, "meaning": "sign-ups" },
            "denominator": { "value": 1000, "meaning": "visitors" }
        }
    })]);
    assert!(p.requests.is_empty());
    assert_eq!(p.beats[0].source, AssetSource::None);
    assert!(p.beats[0].requests.is_empty());
    assert_eq!(p.beats[0].composition, "data_story");
}

#[test]
fn number_comparison_needs_no_image() {
    let p = plan(vec![json!({
        "purpose": "compare",
        "statement": "Two numbers side by side",
        "primary": { "kind": "number", "value": "120", "meaning": "before" },
        "secondary": { "kind": "number", "value": "480", "meaning": "after" }
    })]);
    assert!(p.requests.is_empty());
    assert_eq!(p.beats[0].source, AssetSource::None);
}

#[test]
fn collection_of_phrases_is_procedural_without_requests() {
    let p = plan(vec![json!({
        "purpose": "explain",
        "statement": "Small things add up over time",
        "primary": {
            "kind": "collection",
            "items": [
                { "kind": "phrase", "value": "Coffee" },
                { "kind": "phrase", "value": "Snacks" },
                { "kind": "phrase", "value": "Rides" }
            ]
        }
    })]);
    assert!(p.requests.is_empty());
    assert_eq!(p.beats[0].source, AssetSource::Procedural);
    assert_eq!(p.beats[0].composition, "sequential_stack");
}

#[test]
fn reveal_with_library_object_is_an_svg_evidence_image() {
    let p = plan(vec![json!({
        "purpose": "reveal",
        "statement": "The basket tells the story",
        "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "the basket" }
    })]);
    let r = request(&p, "beat_1.evidence_image");
    assert_eq!(r.source, AssetSource::Svg);
    assert_eq!(r.library_asset.as_deref(), Some("shopping_basket"));
    assert_eq!(r.role, AssetRole::EvidenceImage);
    assert_eq!(r.presentation, Presentation::FlatArtifact);
    assert_eq!(r.background, Background::Opaque);
    assert_eq!(r.priority, Priority::Required);
    assert_eq!(r.subject, "the basket");
    assert_eq!(p.beats[0].composition, "evidence_stack");
    assert_eq!(p.beats[0].source, AssetSource::Svg);
}

#[test]
fn emphasized_library_object_is_a_hero_object() {
    let p = plan(vec![emphasize(
        json!({ "kind": "object", "asset": "shopping_basket" }),
    )]);
    let r = request(&p, "beat_1.hero_object");
    assert_eq!(r.source, AssetSource::Svg);
    assert_eq!(r.role, AssetRole::HeroObject);
    assert_eq!(r.presentation, Presentation::IsolatedCutout);
    assert_eq!(r.background, Background::Transparent);
    assert_eq!(r.subject, "shopping_basket");
    assert_eq!(r.composition, "hero_object");
}

#[test]
fn second_object_gets_a_distinct_role_and_unique_ids() {
    let p = plan(vec![json!({
        "purpose": "reveal",
        "statement": "Two objects appear",
        "primary": { "kind": "object", "asset": "shopping_basket" },
        "secondary": { "kind": "object", "asset": "shopping_basket" }
    })]);
    assert_eq!(p.requests.len(), 2);
    let ids: std::collections::BTreeSet<_> = p.requests.iter().map(|r| &r.id).collect();
    assert_eq!(ids.len(), 2);
}

#[test]
fn object_lookup_svg_png_or_missing() {
    let lib = png_only_library();
    let beats = |asset: &str| vec![emphasize(json!({ "kind": "object", "asset": asset }))];
    let s = StyleProfile::default();

    let png = plan_with(beats("receipt"), &s, &lib);
    let r = request(&png, "beat_1.hero_object");
    assert_eq!(r.source, AssetSource::UserAsset);
    assert_eq!(r.library_asset.as_deref(), Some("receipt"));

    let missing = plan_with(beats("unknown_gadget"), &s, &lib);
    let r = request(&missing, "beat_1.hero_object");
    assert_eq!(r.source, AssetSource::GeneratedImage);
    assert!(r.library_asset.is_none());
    assert_eq!(r.priority, Priority::Required);

    // The svg in the repo library is not visible from the png-only library.
    let basket = plan_with(beats("shopping_basket"), &s, &lib);
    assert_eq!(
        request(&basket, "beat_1.hero_object").source,
        AssetSource::GeneratedImage
    );
}

#[test]
fn parallax_emphasize_without_human_gets_an_environment_plate() {
    let style = style_from(json!({ "motion_language": "parallax" }));
    let p = plan_with(
        vec![json!({
            "purpose": "emphasize",
            "statement": "The city never sleeps",
            "primary": { "kind": "phrase", "value": "empty streets at dawn" },
            "keyword": "dawn"
        })],
        &style,
        &repo_assets(),
    );
    assert_eq!(p.beats[0].composition, "cinematic_multiplane");
    let r = request(&p, "beat_1.environment");
    assert_eq!(r.role, AssetRole::Environment);
    assert_eq!(r.presentation, Presentation::BackgroundPlate);
    assert_eq!(r.background, Background::Opaque);
    assert_eq!(r.negative_space, NegativeSpace::None);
    assert_eq!(r.priority, Priority::Optional);
    assert_eq!(r.source, AssetSource::GeneratedImage);
    assert_eq!(r.subject, "dawn \u{2014} environment");
    assert_eq!(p.requests.len(), 1);
}

#[test]
fn parallax_environment_falls_back_to_the_phrase_without_keyword() {
    let style = style_from(json!({ "motion_language": "parallax" }));
    let p = plan_with(
        vec![emphasize(phrase("empty streets"))],
        &style,
        &repo_assets(),
    );
    assert_eq!(
        request(&p, "beat_1.environment").subject,
        "empty streets \u{2014} environment"
    );
}

#[test]
fn parallax_with_human_requests_the_subject_not_an_environment() {
    let style = style_from(json!({ "motion_language": "parallax" }));
    let p = plan_with(
        vec![emphasize(phrase("a lone commuter"))],
        &style,
        &repo_assets(),
    );
    assert_eq!(p.requests.len(), 1);
    assert_eq!(p.requests[0].role, AssetRole::HeroSubject);
}

#[test]
fn ids_are_beat_numbered_and_requests_ordered_by_beat() {
    let p = plan(vec![
        emphasize(phrase("a friendly doctor")),
        json!({
            "purpose": "reveal",
            "statement": "Numbers land",
            "primary": { "kind": "number", "value": "42" }
        }),
        emphasize(json!({ "kind": "object", "asset": "shopping_basket" })),
    ]);
    let ids: Vec<_> = p.requests.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, vec!["beat_1.hero_subject", "beat_3.hero_object"]);
    assert_eq!(p.beats.len(), 3);
    assert_eq!(
        p.beats.iter().map(|b| b.beat).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(p.version, "0.1");
}

#[test]
fn plan_is_deterministic() {
    let beats = || {
        vec![
            emphasize(phrase("office worker concentrating at a laptop")),
            emphasize(json!({ "kind": "object", "asset": "shopping_basket" })),
        ]
    };
    let a = plan(beats()).to_json_pretty();
    let b = plan(beats()).to_json_pretty();
    assert_eq!(a, b);
    assert!(!a.is_empty());
}

fn collect_keys(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            for (k, v) in m {
                out.push(k.clone());
                collect_keys(v, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|v| collect_keys(v, out)),
        _ => {}
    }
}

#[test]
fn plan_contains_no_pixel_or_timing_keys() {
    let p = plan(vec![
        emphasize(phrase("a tired face")),
        emphasize(json!({ "kind": "object", "asset": "shopping_basket" })),
    ]);
    let value: Value = serde_json::from_str(&p.to_json_pretty()).expect("json");
    let mut keys = Vec::new();
    collect_keys(&value, &mut keys);
    for banned in ["x", "y", "crop", "transform", "timing", "start", "duration"] {
        assert!(!keys.iter().any(|k| k == banned), "banned key {banned}");
    }
}

#[test]
fn asset_style_varies_with_the_style_profile() {
    let beats = || vec![emphasize(phrase("a tired face"))];
    let collage = style_from(json!({
        "family": "editorial_collage", "texture_style": "heavy_print", "accent_role": "cobalt"
    }));
    let minimal = style_from(json!({
        "family": "minimal", "texture_style": "none", "accent_role": "acid"
    }));
    let a = plan_with(beats(), &collage, &repo_assets()).style;
    let b = plan_with(beats(), &minimal, &repo_assets()).style;
    assert_ne!(a, b);
    assert_ne!(a.medium, b.medium);
    assert_ne!(a.contrast, b.contrast);
    assert_ne!(a.palette_tendency, b.palette_tendency);
    assert_eq!(a.contrast, "high contrast, crushed blacks");
    assert_eq!(b.contrast, "low-medium contrast");
}

#[test]
fn planner_rejects_bad_intents_like_compile() {
    let s = StyleProfile::default();
    let lib = repo_assets();
    let empty = intent(vec![]);
    assert!(matches!(
        plan_assets(&empty, &s, &lib),
        Err(CompileError::NoBeats)
    ));
    let mut bad = intent(vec![emphasize(phrase("x"))]);
    bad.version = "9.9".into();
    assert!(matches!(
        plan_assets(&bad, &s, &lib),
        Err(CompileError::Version(_))
    ));
    let invalid = intent(vec![json!({
        "purpose": "explain",
        "statement": "s",
        "primary": { "kind": "state_change", "entity": " ", "from": "a", "to": "b" }
    })]);
    assert!(matches!(
        plan_assets(&invalid, &s, &lib),
        Err(CompileError::Invalid(_))
    ));
}

// ---- manifest ------------------------------------------------------------

fn good_manifest() -> Value {
    json!({
        "version": "0.1",
        "assets": [
            {
                "id": "beat_1.hero_subject", "path": "images/worker.PNG",
                "width": 1024, "height": 1536, "alpha": true,
                "safe_bounds": { "x": 0.1, "y": 0.1, "width": 0.7, "height": 0.8 },
                "subject_anchor": { "x": 0.5, "y": 0.5 },
                "face_anchor": { "x": 0.5, "y": 0.25 },
                "generator": { "vendor": "anything", "seed": 5 }
            },
            {
                "id": "beat_2.environment", "path": "images/street.png",
                "width": 1920, "height": 1080, "alpha": false
            }
        ]
    })
}

fn manifest_errors(v: Value) -> Vec<String> {
    AssetManifest::from_json(&v.to_string())
        .expect("parses")
        .validate()
        .expect_err("should fail validation")
}

#[test]
fn good_manifest_validates() {
    let m = AssetManifest::from_json(&good_manifest().to_string()).expect("parse");
    assert_eq!(m.validate(), Ok(()));
    assert!(m.get("beat_2.environment").is_some());
    assert_eq!(AssetManifest::empty().validate(), Ok(()));
}

#[test]
fn manifest_rejects_duplicate_ids() {
    let mut v = good_manifest();
    v["assets"][1]["id"] = json!("beat_1.hero_subject");
    let errs = manifest_errors(v);
    assert!(errs.iter().any(|e| e.contains("duplicate id")), "{errs:?}");
}

#[test]
fn manifest_rejects_unsupported_paths_and_alpha_jpeg() {
    // 0.5: JPEG is accepted (opaque only).
    let mut v = good_manifest();
    v["assets"][1]["path"] = json!("images/street.JPG");
    let m = AssetManifest::from_json(&v.to_string()).expect("parse");
    assert_eq!(m.validate(), Ok(()));
    let mut v = good_manifest();
    v["assets"][0]["path"] = json!("images/worker.jpeg");
    let errs = manifest_errors(v);
    assert!(
        errs.iter().any(|e| e.contains("JPEG cannot carry alpha")),
        "{errs:?}"
    );
    let mut v = good_manifest();
    v["assets"][1]["path"] = json!("images/street.webp");
    let errs = manifest_errors(v);
    assert!(errs.iter().any(|e| e.contains(".png")), "{errs:?}");
    let mut v = good_manifest();
    v["assets"][0]["path"] = json!("");
    assert!(!manifest_errors(v).is_empty());
}

#[test]
fn manifest_rejects_out_of_range_geometry() {
    let mut v = good_manifest();
    v["assets"][0]["safe_bounds"] = json!({ "x": 0.5, "y": 0.1, "width": 0.7, "height": 0.5 });
    assert!(manifest_errors(v).iter().any(|e| e.contains("safe_bounds")));

    let mut v = good_manifest();
    v["assets"][0]["safe_bounds"] = json!({ "x": 0.1, "y": 0.1, "width": 0.0, "height": 0.5 });
    assert!(manifest_errors(v).iter().any(|e| e.contains("safe_bounds")));

    let mut v = good_manifest();
    v["assets"][0]["face_anchor"] = json!({ "x": 1.2, "y": 0.5 });
    assert!(manifest_errors(v).iter().any(|e| e.contains("face_anchor")));

    let mut v = good_manifest();
    v["assets"][0]["subject_anchor"] = json!({ "x": -0.1, "y": 0.5 });
    assert!(manifest_errors(v)
        .iter()
        .any(|e| e.contains("subject_anchor")));
}

#[test]
fn manifest_rejects_zero_size_and_wrong_version() {
    let mut v = good_manifest();
    v["assets"][0]["width"] = json!(0);
    v["version"] = json!("0.3");
    let errs = manifest_errors(v);
    assert!(errs.iter().any(|e| e.contains("width and height")));
    assert!(errs.iter().any(|e| e.contains("version")));
}

#[test]
fn manifest_and_plan_round_trip() {
    let m = AssetManifest::from_json(&good_manifest().to_string()).expect("parse");
    let back: AssetManifest =
        serde_json::from_str(&serde_json::to_string(&m).expect("ser")).expect("de");
    assert_eq!(m, back);

    let p = plan(vec![
        emphasize(phrase("a tired face")),
        emphasize(json!({ "kind": "object", "asset": "shopping_basket" })),
    ]);
    let back: AssetPlan = serde_json::from_str(&p.to_json_pretty()).expect("de");
    assert_eq!(p, back);
}

#[test]
fn unknown_fields_are_rejected() {
    let mut v = good_manifest();
    v["assets"][0]["prompt"] = json!("x");
    assert!(AssetManifest::from_json(&v.to_string()).is_err());
    let mut v = good_manifest();
    v["extra"] = json!(1);
    assert!(AssetManifest::from_json(&v.to_string()).is_err());
    assert!(AssetManifest::from_json("not json").is_err());

    let p = plan(vec![emphasize(phrase("a tired face"))]);
    let mut v: Value = serde_json::from_str(&p.to_json_pretty()).expect("json");
    v["requests"][0]["x"] = json!(10);
    assert!(serde_json::from_value::<AssetPlan>(v).is_err());
    let mut v: Value = serde_json::from_str(&p.to_json_pretty()).expect("json");
    v["style"]["extra"] = json!("nope");
    assert!(serde_json::from_value::<AssetPlan>(v).is_err());
}

// ---------------------------------------------------------------------------
// 0.5: planner false positives — data / typography beats never ask for images
// ---------------------------------------------------------------------------

fn generated(plan: &AssetPlan) -> usize {
    plan.requests
        .iter()
        .filter(|r| r.source == AssetSource::GeneratedImage)
        .count()
}

#[test]
fn data_and_typography_beats_request_no_generated_image() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "ratio contrast",
            json!({ "purpose": "contrast", "statement": "Rent now takes a third of pay",
                    "primary": { "kind": "number", "value": "₹14,000", "meaning": "rent" },
                    "secondary": { "kind": "number", "value": "₹42,000", "meaning": "salary" },
                    "relationship": "compress" }),
        ),
        (
            "derived ratio",
            json!({ "purpose": "reveal", "statement": "Only a few convert",
                    "primary": { "kind": "derived_metric", "operation": "ratio",
                                 "numerator": { "value": 80, "meaning": "buyers" },
                                 "denominator": { "value": 1000, "meaning": "visitors" },
                                 "format": "percent", "meaning": "conversion" } }),
        ),
        (
            "strong number",
            json!({ "purpose": "reveal", "statement": "The real number",
                    "primary": { "kind": "number", "value": "3.5x", "meaning": "cost" },
                    "energy": "impact" }),
        ),
        (
            "kinetic typography",
            json!({ "purpose": "emphasize", "statement": "Speed is the new normal",
                    "primary": { "kind": "phrase", "value": "speed", "meaning": "pace" },
                    "energy": "impact", "keyword": "speed" }),
        ),
        (
            "number collection",
            json!({ "purpose": "explain", "statement": "Monthly costs keep rising",
                    "primary": { "kind": "collection", "items": [
                        { "kind": "number", "value": "120" }, { "kind": "number", "value": "140" },
                        { "kind": "number", "value": "175" } ], "meaning": "costs" } }),
        ),
        (
            "phrase collection accumulate",
            json!({ "purpose": "explain", "statement": "Small things add up",
                    "primary": { "kind": "collection", "items": [
                        { "kind": "phrase", "value": "coffee" }, { "kind": "phrase", "value": "taxi" },
                        { "kind": "phrase", "value": "snacks" } ] },
                    "relationship": "accumulate" }),
        ),
        (
            "state change",
            json!({ "purpose": "contrast", "statement": "The plan changed",
                    "primary": { "kind": "state_change", "entity": "Plan", "from": "Free", "to": "Paid" } }),
        ),
        (
            "human words in a data beat",
            json!({ "purpose": "compare", "statement": "Workers versus managers",
                    "primary": { "kind": "number", "value": "120", "meaning": "workers" },
                    "secondary": { "kind": "number", "value": "12", "meaning": "managers" } }),
        ),
    ];
    for (name, beat) in cases {
        let v = json!({ "version": "0.2", "title": "t", "format": "vertical", "beats": [beat] });
        let i: CreativeIntent = serde_json::from_value(v).unwrap_or_else(|e| panic!("{name}: {e}"));
        let p = plan_assets(&i, &StyleProfile::default(), &repo_assets())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(generated(&p), 0, "{name}: {:?}", p.requests);
    }
}

#[test]
fn human_request_carries_statement_context_and_a_continuity_key() {
    let p = plan(vec![
        emphasize(json!({ "kind": "phrase", "value": "Office worker", "meaning": "worker" })),
        emphasize(json!({ "kind": "phrase", "value": "office  WORKER" })),
    ]);
    assert_eq!(generated(&p), 2);
    for r in &p.requests {
        assert_eq!(r.context.as_deref(), Some("A short statement here"));
        assert_eq!(r.continuity_key.as_deref(), Some("office_worker"));
    }
}

#[test]
fn blind_benchmarks_only_ask_for_images_where_a_person_is_the_subject() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut total = 0;
    for dir in ["golden/fixtures/benchmarks"] {
        for case in std::fs::read_dir(root.join(dir)).expect("benchmark dir") {
            let case = case.expect("entry").path();
            let intent = case.join("attempt-01.intent.json");
            if !intent.exists() {
                continue;
            }
            let i = CreativeIntent::from_json(&std::fs::read_to_string(&intent).unwrap())
                .expect("benchmark intent parses");
            let p = plan_assets(&i, &StyleProfile::default(), &repo_assets()).expect("plans");
            let n = generated(&p);
            let name = case.file_name().unwrap().to_string_lossy().to_string();
            if name.starts_with("late-night-worker") {
                assert_eq!(n, 1, "{name}");
            } else {
                assert_eq!(n, 0, "{name}: {:?}", p.requests);
            }
            total += n;
        }
    }
    assert_eq!(total, 1);
}
