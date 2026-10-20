//! (0.10) A weak model naming an object asset that exists nowhere degrades
//! that subject to a phrase instead of failing the compile.
use motion_core::compiler::{compile_with_options, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::{ApproxMeasure, StyleProfile};

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

#[test]
fn unknown_object_asset_becomes_a_phrase() {
    let intent = CreativeIntent::from_json(
        r#"{"version":"0.2","title":"t","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"A thing nobody drew","primary":{"kind":"object","asset":"zz_not_a_real_asset","meaning":"mystery box"}},
        {"purpose":"reveal","statement":"Still works","primary":{"kind":"number","value":"42%"}}]}"#,
    )
    .expect("intent");
    let style = StyleProfile::from_json(r#"{"tone":"editorial"}"#).unwrap();
    let lib = AssetLibrary::new(root());
    let p = compile_with_options(
        &intent,
        &style,
        None,
        &lib,
        &ApproxMeasure,
        &motion_core::assets::AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .expect("compiles with the fallback");
    let json = serde_json::to_string(&p).unwrap();
    assert!(json.to_lowercase().contains("mystery box"));
    assert!(!json.contains("zz_not_a_real_asset"));
}
