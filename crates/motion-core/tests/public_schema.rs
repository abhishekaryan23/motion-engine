//! Public schema drift guard.
//!
//! The JSON Schemas in `schema/` are generated from the Rust types
//! (`CreativeIntent`, `StyleProfile`) via schemars. This test fails if the
//! checked-in files differ from what the current types generate.
//!
//! Regenerate after an intentional contract change:
//!   MOTION_UPDATE_SCHEMA=1 cargo test -p motion-core --test public_schema

use std::path::PathBuf;

use motion_core::intent::v0_1;
use motion_core::{CreativeIntent, StyleProfile};
use serde_json::Value;

fn schema_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../schema"))
}

pub fn generated(name: &str) -> Value {
    let schema = match name {
        "creative-intent-v0.1.schema.json" => schemars::schema_for!(v0_1::CreativeIntent),
        "creative-intent-v0.2.schema.json" => schemars::schema_for!(CreativeIntent),
        "style-profile-v0.1.schema.json" => schemars::schema_for!(StyleProfile),
        "reference-style-profile-v0.1.schema.json" => {
            schemars::schema_for!(motion_core::reference::profile_v0_1::ReferenceStyleProfile)
        }
        "reference-style-profile-v0.2.schema.json" => {
            schemars::schema_for!(motion_core::reference::ReferenceStyleProfile)
        }
        other => panic!("unknown schema {other}"),
    };
    serde_json::to_value(schema).expect("schema serializes")
}

fn check(name: &str) {
    let path = schema_dir().join(name);
    let fresh = generated(name);
    if std::env::var("MOTION_UPDATE_SCHEMA").as_deref() == Ok("1") {
        std::fs::create_dir_all(schema_dir()).expect("create schema dir");
        let text = serde_json::to_string_pretty(&fresh).expect("pretty") + "\n";
        std::fs::write(&path, text).expect("write schema");
        return;
    }
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .expect("checked-in schema is JSON");
    assert_eq!(
        on_disk, fresh,
        "{name} is out of date with the Rust types; regenerate with \
         MOTION_UPDATE_SCHEMA=1 cargo test -p motion-core --test public_schema"
    );
}

#[test]
fn creative_intent_v0_1_schema_matches_rust_types() {
    check("creative-intent-v0.1.schema.json");
}

#[test]
fn creative_intent_v0_2_schema_matches_rust_types() {
    check("creative-intent-v0.2.schema.json");
}

#[test]
fn style_profile_schema_matches_rust_types() {
    check("style-profile-v0.1.schema.json");
}

/// The object-asset enum in the intent schema must list exactly the bundled library.
#[test]
fn object_asset_enum_matches_bundled_library() {
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/library"));
    let mut on_disk: Vec<String> = std::fs::read_dir(&lib)
        .expect("assets/library exists")
        .filter_map(|e| {
            let p = e.ok()?.path();
            let ext = p.extension()?.to_str()?;
            (ext == "svg" || ext == "png").then(|| p.file_stem()?.to_str().map(str::to_string))?
        })
        .collect();
    on_disk.sort();
    on_disk.dedup();
    let sorted = |v: &Value| -> Vec<String> {
        let mut out: Vec<String> = v
            .as_array()
            .expect("asset enum present")
            .iter()
            .map(|v| v.as_str().expect("string").to_string())
            .collect();
        out.sort();
        out
    };

    // v0.1: one flat Subject with an if/then rule.
    let v1 = generated("creative-intent-v0.1.schema.json");
    let listed = sorted(&v1["$defs"]["Subject"]["allOf"][0]["then"]["properties"]["asset"]["enum"]);
    assert_eq!(
        listed, on_disk,
        "update the asset enum in crates/motion-core/src/intent/v0_1.rs and the AI authoring guide"
    );

    // v0.2 (0.10): object assets are open snake_case names (the compiler
    // finds a picture in the library/families or shows the meaning as text).
    let v2 = generated("creative-intent-v0.2.schema.json");
    let mut seen = 0;
    for def in ["Subject", "CollectionItem"] {
        for variant in v2["$defs"][def]["oneOf"].as_array().expect("oneOf") {
            if variant["properties"]["kind"]["const"] == "object" {
                seen += 1;
                let asset = &variant["properties"]["asset"];
                assert!(asset.get("enum").is_none(), "v0.2 asset is open");
                assert_eq!(asset["pattern"], "^[a-z0-9][a-z0-9_]{0,40}$");
            }
        }
    }
    assert_eq!(seen, 2, "object variants in Subject and CollectionItem");
}

#[test]
fn reference_style_profile_v0_1_schema_matches_rust_types() {
    check("reference-style-profile-v0.1.schema.json");
}

#[test]
fn reference_style_profile_v0_2_schema_matches_rust_types() {
    check("reference-style-profile-v0.2.schema.json");
}
