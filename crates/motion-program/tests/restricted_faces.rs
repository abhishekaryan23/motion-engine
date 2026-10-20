//! Restricted faces (licence forbids redistribution, assets/fonts/RESTRICTED.md):
//! every `Face` and the UI sans load either a bundled open file or the open
//! fallback of a restricted file, so programs render in the public tree.

use std::path::PathBuf;

use motion_core::compiler::typography::{resolve_font_path, restricted_fallback};
use motion_program::program::Face;

fn assets() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"))
}

fn all_faces() -> Vec<Face> {
    let schema = serde_json::to_value(schemars::schema_for!(Face)).unwrap();
    let names: Vec<String> = schema
        .get("enum")
        .or_else(|| schema.pointer("/schema/enum"))
        .and_then(|e| e.as_array())
        .expect("Face schema lists its variants")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(names.len() >= 19, "{names:?}");
    names
        .iter()
        .map(|n| serde_json::from_value(serde_json::Value::String(n.clone())).unwrap())
        .collect()
}

#[test]
fn every_face_resolves_to_a_bundled_open_file() {
    let mut restricted = 0;
    for face in all_faces() {
        let (_, file) = face.asset();
        match restricted_fallback(file) {
            Some(fallback) => {
                restricted += 1;
                assert!(
                    assets().join(&fallback).is_file(),
                    "{face:?}: fallback {fallback} is bundled"
                );
                // Whatever the checkout holds, the loader finds a file.
                assert!(
                    resolve_font_path(&assets().join(file)).is_file(),
                    "{face:?}"
                );
            }
            None => assert!(
                assets().join(file).is_file(),
                "{face:?}: {file} is an open bundled face"
            ),
        }
    }
    // Cabinet Grotesk, Clash Display, Panchang, Satoshi, Zodiak x2.
    assert_eq!(restricted, 6);
}
