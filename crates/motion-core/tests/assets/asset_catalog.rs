//! (0.8) Library catalogs: the AssetPlanner prefers an exact-word match from an
//! opt-in asset family to a generated image, and compile delivers it exactly
//! like an externally delivered image. Fixture families live under target/.

use std::path::PathBuf;

use motion_core::assets::{AssetManifest, AssetPlan, AssetRequest, AssetSource};
use motion_core::compiler::catalog::{request_words, MatchKind};
use motion_core::compiler::{compile, compile_with_assets, plan_assets, ApproxMeasure};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::style::StyleProfile;
use motion_core::AssetLibrary;
use serde_json::{json, Value};

/// A 1x1 transparent PNG (the engine never decodes it at compile time).
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F,
    0x00, 0x05, 0xFE, 0x02, 0xFE, 0xA7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

/// One fixture catalog asset.
struct Fx {
    id: &'static str,
    role: &'static str,
    tags: &'static [&'static str],
    qa: &'static str,
    in_manifest: bool,
}

fn fx(id: &'static str, role: &'static str, tags: &'static [&'static str]) -> Fx {
    Fx {
        id,
        role,
        tags,
        qa: "PASS",
        in_manifest: true,
    }
}

/// A fresh library root under target/ for one test.
fn root(test: &str) -> PathBuf {
    let r = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/asset_catalog_tests")
        .join(test);
    let _ = std::fs::remove_dir_all(&r);
    std::fs::create_dir_all(r.join("library")).expect("create root");
    r
}

/// Write `library/<family>/` (catalog.json, manifest.json, PNGs) under `root`.
fn write_family(root: &std::path::Path, family: &str, assets: &[Fx]) {
    let dir = root.join("library").join(family);
    std::fs::create_dir_all(&dir).expect("family dir");
    let mut catalog = Vec::new();
    let mut manifest = Vec::new();
    for a in assets {
        let file = format!("{}.png", a.id);
        std::fs::write(dir.join(&file), PNG).expect("png");
        catalog.push(json!({
            "id": a.id, "file": file, "role": a.role, "subject": a.id,
            "tags": a.tags, "alpha": true, "cutout_bbox": null, "open_edge": false,
            "source": { "vendor": "test" }, "qa": a.qa,
            "analysis": { "coverage": 0.4 }, "width": 64, "height": 64,
            "future_field": [1, 2, 3]
        }));
        if a.in_manifest {
            manifest.push(json!({
                "id": format!("library.{}", a.id), "path": file,
                "width": 64, "height": 64, "alpha": true,
                "serves": [format!("library.{}", a.id)]
            }));
        }
    }
    std::fs::write(
        dir.join("catalog.json"),
        json!({ "version": "0.1", "family": family, "assets": catalog }).to_string(),
    )
    .expect("catalog.json");
    std::fs::write(
        dir.join("manifest.json"),
        json!({ "version": "0.2", "assets": manifest }).to_string(),
    )
    .expect("manifest.json");
}

fn lib(root: &std::path::Path, families: &[&str]) -> AssetLibrary {
    AssetLibrary::new(root).with_families(families.iter().map(|s| s.to_string()).collect())
}

fn intent(beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({ "version": "0.2", "title": "asset_catalog", "beats": beats }))
        .expect("intent parses")
}

fn emphasize(primary: Value) -> Value {
    json!({ "purpose": "emphasize", "statement": "A short statement here", "primary": primary })
}

fn object(asset: &str, meaning: Option<&str>) -> Value {
    let mut v = json!({ "kind": "object", "asset": asset });
    if let Some(m) = meaning {
        v["meaning"] = json!(m);
    }
    v
}

fn plan(beats: Vec<Value>, lib: &AssetLibrary) -> AssetPlan {
    plan_assets(&intent(beats), &StyleProfile::default(), lib).expect("plan")
}

fn request<'a>(plan: &'a AssetPlan, id: &str) -> &'a AssetRequest {
    plan.requests
        .iter()
        .find(|r| r.id == id)
        .unwrap_or_else(|| panic!("no request {id} in {:?}", plan.requests))
}

fn build(beats: Vec<Value>, lib: &AssetLibrary) -> MotionProject {
    compile(
        &intent(beats),
        &StyleProfile::default(),
        lib,
        &ApproxMeasure,
    )
    .expect("compiles")
}

fn image_paths(p: &MotionProject) -> Vec<String> {
    fn walk<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
        for l in layers {
            out.push(l);
            if let LayerKind::Group { children } = &l.kind {
                walk(children, out);
            }
        }
    }
    let mut layers = Vec::new();
    for s in &p.scenes {
        walk(&s.layers, &mut layers);
    }
    layers
        .into_iter()
        .filter_map(|l| match &l.kind {
            LayerKind::Image { asset, .. } => p
                .assets
                .iter()
                .find(|a| &a.id == asset)
                .map(|a| a.path.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn empty_families_are_byte_identical_to_no_catalog() {
    let with_dir = root("empty_with_dir");
    write_family(
        &with_dir,
        "fixture",
        &[fx("receipt", "object", &["receipt"])],
    );
    let without_dir = root("empty_without_dir");
    let a = lib(&with_dir, &[]);
    let b = lib(&without_dir, &[]);
    // Plans (object + human beats) and scenes (beats that need no library file).
    let planned = vec![
        emphasize(object("receipt", None)),
        emphasize(json!({ "kind": "phrase", "value": "a tired office worker" })),
    ];
    assert_eq!(
        plan(planned.clone(), &a).to_json_pretty(),
        plan(planned, &b).to_json_pretty()
    );
    let compiled = vec![
        emphasize(json!({ "kind": "phrase", "value": "a tired office worker" })),
        json!({
            "purpose": "reveal",
            "statement": "Most visitors never sign up",
            "primary": { "kind": "number", "value": "120", "meaning": "visitors" }
        }),
    ];
    assert_eq!(
        serde_json::to_string(&build(compiled.clone(), &a)).unwrap(),
        serde_json::to_string(&build(compiled, &b)).unwrap()
    );
}

#[test]
fn object_matches_catalog_and_compiles_as_a_delivered_image() {
    let r = root("object_match");
    write_family(
        &r,
        "fixture",
        &[
            fx("receipt", "object", &["receipt", "paper"]),
            fx("plant", "object", &["plant"]),
        ],
    );
    let l = lib(&r, &["fixture"]);
    let beats = vec![emphasize(object("receipt", None))];
    let p = plan(beats.clone(), &l);
    let req = request(&p, "beat_1.hero_object");
    assert_eq!(req.source, AssetSource::UserAsset);
    assert_eq!(req.library_asset.as_deref(), Some("fixture/receipt"));
    assert_eq!(
        req.reason,
        "Library catalog: 'fixture/receipt' matches receipt."
    );
    assert!(req.context.is_none());
    // Deterministic.
    assert_eq!(p.to_json_pretty(), plan(beats.clone(), &l).to_json_pretty());

    let scene = build(beats.clone(), &l);
    let paths = image_paths(&scene);
    assert!(
        paths
            .iter()
            .any(|p| p.ends_with("library/fixture/receipt.png")),
        "image paths: {paths:?}"
    );
    assert_eq!(
        serde_json::to_string(&scene).unwrap(),
        serde_json::to_string(&build(beats.clone(), &l)).unwrap()
    );
    // Without the family nothing delivers the object: the library has no
    // 'receipt'. (0.10) The subject degrades to a phrase instead of failing.
    let fallback = compile(
        &intent(beats),
        &StyleProfile::default(),
        &lib(&r, &[]),
        &ApproxMeasure,
    )
    .expect("missing objects degrade to phrases");
    assert!(image_paths(&fallback)
        .iter()
        .all(|p| !p.ends_with("receipt.png")));
}

#[test]
fn matching_is_exact_word_only() {
    let r = root("exact_words");
    write_family(
        &r,
        "fixture",
        &[
            fx("receipt", "object", &["receipt"]),
            fx("stopper", "object", &["the", "a", "of"]),
        ],
    );
    let l = lib(&r, &["fixture"]);
    let plural = plan(vec![emphasize(object("receipts", None))], &l);
    let req = request(&plural, "beat_1.hero_object");
    assert_eq!(req.source, AssetSource::GeneratedImage);
    assert!(req.library_asset.is_none());

    // A stop word alone never matches, even against a catalog tag of the same word.
    let stop = plan(vec![emphasize(object("the", Some("of")))], &l);
    assert_eq!(
        request(&stop, "beat_1.hero_object").source,
        AssetSource::GeneratedImage
    );
    assert!(request_words(&["The of AND a"]).is_empty());
    assert!(l
        .catalog_match(&request_words(&["the"]), MatchKind::Object)
        .is_none());

    // Tokens are lowercase ASCII alphanumerics; extra words do not hurt.
    let ok = request_words(&["Receipt, for-the Taxman!"]);
    assert_eq!(
        ok.iter().map(String::as_str).collect::<Vec<_>>(),
        ["receipt", "taxman"]
    );
    assert!(l.catalog_match(&ok, MatchKind::Object).is_some());
}

#[test]
fn tie_break_is_score_then_family_order_then_id() {
    let r = root("tie_break");
    write_family(
        &r,
        "alpha",
        &[
            fx("b_item", "object", &["receipt"]),
            fx("a_item", "object", &["receipt"]),
            fx("c_item", "object", &["receipt", "paper"]),
        ],
    );
    write_family(&r, "beta", &[fx("a_item", "object", &["receipt"])]);
    let words = request_words(&["receipt"]);

    // Same score: family order wins, then id ascending.
    let m = lib(&r, &["beta", "alpha"])
        .catalog_match(&words, MatchKind::Object)
        .expect("match");
    assert_eq!((m.family.as_str(), m.id.as_str()), ("beta", "a_item"));
    let m = lib(&r, &["alpha", "beta"])
        .catalog_match(&words, MatchKind::Object)
        .expect("match");
    assert_eq!((m.family.as_str(), m.id.as_str()), ("alpha", "a_item"));
    assert_eq!(m.rel_path, "library/alpha/a_item.png");

    // Higher score beats family order and id.
    let two = request_words(&["receipt paper"]);
    let m = lib(&r, &["beta", "alpha"])
        .catalog_match(&two, MatchKind::Object)
        .expect("match");
    assert_eq!((m.family.as_str(), m.id.as_str()), ("alpha", "c_item"));
    assert_eq!(m.matched, vec!["paper".to_string(), "receipt".to_string()]);

    // Through the planner.
    let p = plan(
        vec![emphasize(object("receipt", Some("paper")))],
        &lib(&r, &["beta", "alpha"]),
    );
    assert_eq!(
        request(&p, "beat_1.hero_object").library_asset.as_deref(),
        Some("alpha/c_item")
    );
}

#[test]
fn human_phrase_matches_figures_only() {
    let r = root("human");
    write_family(
        &r,
        "fixture",
        &[
            fx("clerk", "figure", &["worker", "office"]),
            fx("desk_worker", "object", &["worker", "office", "tired"]),
            fx("worker_tex", "texture", &["worker", "office", "tired"]),
        ],
    );
    let l = lib(&r, &["fixture"]);
    let beats = vec![emphasize(
        json!({ "kind": "phrase", "value": "a tired office worker" }),
    )];
    let p = plan(beats.clone(), &l);
    let req = request(&p, "beat_1.hero_subject");
    assert_eq!(req.source, AssetSource::UserAsset);
    assert_eq!(req.library_asset.as_deref(), Some("fixture/clerk"));
    assert_eq!(req.priority, motion_core::assets::Priority::Optional);
    let scene = build(beats, &l);
    assert!(
        image_paths(&scene)
            .iter()
            .any(|p| p.ends_with("library/fixture/clerk.png")),
        "{:?}",
        image_paths(&scene)
    );

    // Objects and textures never serve a human request; figures never serve objects.
    let only_obj = root("human_only_object");
    write_family(
        &only_obj,
        "fixture",
        &[
            fx("desk_worker", "object", &["worker"]),
            fx("worker_tex", "texture", &["worker"]),
        ],
    );
    let p = plan(
        vec![emphasize(
            json!({ "kind": "phrase", "value": "a tired office worker" }),
        )],
        &lib(&only_obj, &["fixture"]),
    );
    assert_eq!(
        request(&p, "beat_1.hero_subject").source,
        AssetSource::GeneratedImage
    );
    let p = plan(vec![emphasize(object("clerk", None))], &l);
    assert_eq!(
        request(&p, "beat_1.hero_object").source,
        AssetSource::GeneratedImage
    );
}

#[test]
fn data_beats_still_produce_no_request() {
    let r = root("data");
    write_family(
        &r,
        "fixture",
        &[fx(
            "receipt",
            "object",
            &["receipt", "sign", "ups", "visitors", "120"],
        )],
    );
    let l = lib(&r, &["fixture"]);
    let p = plan(
        vec![
            json!({
                "purpose": "reveal",
                "statement": "Most visitors never sign up",
                "primary": {
                    "kind": "derived_metric",
                    "numerator": { "value": 80, "meaning": "sign-ups" },
                    "denominator": { "value": 1000, "meaning": "visitors" }
                }
            }),
            json!({
                "purpose": "compare",
                "statement": "Two numbers side by side",
                "primary": { "kind": "number", "value": "120", "meaning": "receipt" },
                "secondary": { "kind": "number", "value": "480", "meaning": "after" }
            }),
        ],
        &l,
    );
    assert!(p.requests.is_empty(), "{:?}", p.requests);
    assert!(p.beats.iter().all(|b| b.source == AssetSource::None));
}

#[test]
fn ineligible_assets_and_missing_families_are_skipped() {
    let r = root("eligibility");
    let mut failed = fx("failed", "object", &["receipt"]);
    failed.qa = "FAIL";
    let mut missing = fx("missing", "object", &["receipt"]);
    missing.qa = "MISSING";
    let mut unlisted = fx("unlisted", "object", &["receipt"]);
    unlisted.in_manifest = false;
    let mut warn = fx("warned", "object", &["receipt"]);
    warn.qa = "WARN";
    write_family(&r, "fixture", &[failed, missing, unlisted]);
    let words = request_words(&["receipt"]);

    // "ghost" has no directory, "broken" has an invalid catalog: both skipped silently.
    std::fs::create_dir_all(r.join("library/broken")).unwrap();
    std::fs::write(r.join("library/broken/catalog.json"), "{ not json").unwrap();
    let l = lib(&r, &["ghost", "broken", "fixture"]);
    assert!(l.catalog_match(&words, MatchKind::Object).is_none());
    let p = plan(vec![emphasize(object("receipt", None))], &l);
    assert_eq!(
        request(&p, "beat_1.hero_object").source,
        AssetSource::GeneratedImage
    );

    // WARN is eligible, and the skipped families do not stop later ones.
    write_family(&r, "fixture", &[warn]);
    let m = l
        .catalog_match(&words, MatchKind::Object)
        .expect("warn matches");
    assert_eq!(m.id, "warned");
}

#[test]
fn caller_manifest_wins_over_the_catalog() {
    let r = root("caller_wins");
    write_family(&r, "fixture", &[fx("receipt", "object", &["receipt"])]);
    let l = lib(&r, &["fixture"]);
    let beats = vec![emphasize(object("receipt", None))];
    let caller: AssetManifest = serde_json::from_value(json!({
        "version": "0.2",
        "assets": [{
            "id": "beat_1.hero_object", "path": "library/caller/own.png",
            "width": 64, "height": 64, "alpha": true
        }]
    }))
    .expect("manifest");
    let intent = intent(beats);
    let p = compile_with_assets(
        &intent,
        &StyleProfile::default(),
        &l,
        &ApproxMeasure,
        &caller,
    )
    .expect("compiles");
    let paths = image_paths(&p);
    assert!(
        paths.iter().any(|p| p.ends_with("library/caller/own.png")),
        "{paths:?}"
    );
    assert!(!paths.iter().any(|p| p.contains("library/fixture")));
}
