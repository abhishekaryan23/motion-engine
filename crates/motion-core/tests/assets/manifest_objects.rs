//! (0.9) Manifest-first object resolution: an object that only exists as a
//! library-catalog asset (e.g. `sketch_icons/calendar`, no `library/calendar.*`)
//! compiles through the manifest entry the planner's request is served by.

use std::path::PathBuf;

use motion_core::assets::AssetPlan;
use motion_core::compiler::{compile, plan_assets, ApproxMeasure};
use motion_core::intent::CreativeIntent;
use motion_core::scene::MotionProject;
use motion_core::style::StyleProfile;
use motion_core::AssetLibrary;
use serde_json::{json, Value};

fn assets_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn library(families: &[&str]) -> AssetLibrary {
    AssetLibrary::new(assets_root()).with_families(families.iter().map(|s| s.to_string()).collect())
}

fn intent(beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({ "version": "0.2", "title": "manifest_objects", "beats": beats }))
        .expect("intent parses")
}

fn object(asset: &str, meaning: &str) -> Value {
    json!({ "kind": "object", "asset": asset, "meaning": meaning })
}

/// Names that exist only in the sketch_icons catalog.
fn catalog_only(name: &str) -> bool {
    let lib = assets_root().join("library");
    !lib.join(format!("{name}.svg")).is_file() && !lib.join(format!("{name}.png")).is_file()
}

fn plan(beats: &[Value], lib: &AssetLibrary) -> AssetPlan {
    plan_assets(&intent(beats.to_vec()), &StyleProfile::default(), lib).expect("plan")
}

fn build(beats: &[Value], lib: &AssetLibrary) -> Result<MotionProject, String> {
    compile(
        &intent(beats.to_vec()),
        &StyleProfile::default(),
        lib,
        &ApproxMeasure,
    )
    .map_err(|e| e.to_string())
}

fn sketch_paths(p: &MotionProject) -> Vec<String> {
    p.assets
        .iter()
        .filter(|a| a.path.starts_with("library/sketch_icons/"))
        .map(|a| a.path.clone())
        .collect()
}

#[test]
fn supporting_object_resolves_through_the_catalog_manifest() {
    assert!(catalog_only("calendar") && catalog_only("padlock"));
    // A primary phrase with a secondary object: a supporting_object request.
    let beats = vec![
        json!({
            "purpose": "emphasize",
            "statement": "Deadlines decide everything here",
            "primary": { "kind": "phrase", "value": "Every deadline counts" },
            "secondary": object("calendar", "calendar")
        }),
        json!({
            "purpose": "reveal",
            "statement": "Your data stays locked down",
            "primary": { "kind": "number", "value": "256", "meaning": "bit encryption" },
            "secondary": object("padlock", "padlock")
        }),
    ];
    let with = library(&["sketch_icons"]);
    let p = plan(&beats, &with);
    assert!(
        p.requests.iter().any(|r| r
            .library_asset
            .as_deref()
            .is_some_and(|l| l.starts_with("sketch_icons/"))),
        "planner matched no catalog asset"
    );
    let project = build(&beats, &with).expect("compiles with the family");
    let paths = sketch_paths(&project);
    assert!(
        paths.iter().any(|p| p.contains("calendar")) && paths.iter().any(|p| p.contains("padlock")),
        "scene references: {paths:?}"
    );
    // Deterministic.
    assert_eq!(
        serde_json::to_string(&project).unwrap(),
        serde_json::to_string(&build(&beats, &with).unwrap()).unwrap()
    );
    // Without the family nothing delivers these objects: (0.10) they degrade
    // to phrases instead of failing the compile.
    build(&beats, &library(&[])).expect("missing objects degrade to phrases");
}

#[test]
fn evidence_object_resolves_through_the_catalog_manifest() {
    assert!(catalog_only("newspaper"));
    let beats = vec![json!({
        "purpose": "reveal",
        "statement": "The press covered it",
        "primary": object("newspaper", "newspaper"),
        "secondary": { "kind": "phrase", "value": "Front page news" }
    })];
    let with = library(&["sketch_icons"]);
    let project = build(&beats, &with).expect("compiles with the family");
    assert!(
        sketch_paths(&project)
            .iter()
            .any(|p| p.contains("newspaper")),
        "scene references: {:?}",
        sketch_paths(&project)
    );
}

#[test]
fn library_named_objects_are_unchanged_by_families() {
    // `shopping_basket` exists in the library: families must not alter output.
    let beats = vec![json!({
        "purpose": "emphasize",
        "statement": "Carts get abandoned all the time",
        "primary": { "kind": "phrase", "value": "Abandoned carts" },
        "secondary": object("shopping_basket", "basket")
    })];
    let a = build(&beats, &library(&[])).expect("plain library");
    let b = build(&beats, &library(&["sketch_icons"])).expect("with family");
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}
