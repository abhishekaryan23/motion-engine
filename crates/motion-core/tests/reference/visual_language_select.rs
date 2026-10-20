//! Neutral visual language is the identity: compiling with default
//! `ReferencePrinciples` (neutral `visual`) must give byte-identical output to
//! compiling without a reference.
use motion_core::assets::AssetManifest;
use motion_core::compiler::{self, taste::ReferencePrinciples, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::style::StyleProfile;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

fn compile_json(intent: &CreativeIntent, r: Option<&ReferencePrinciples>) -> String {
    let library = AssetLibrary::new(format!("{ROOT}/assets"));
    let project = compiler::compile_full(
        intent,
        &StyleProfile::default(),
        r,
        &library,
        &ApproxMeasure,
        &AssetManifest::default(),
    )
    .unwrap();
    serde_json::to_string(&project).unwrap()
}

#[test]
fn neutral_reference_principles_are_identity() {
    let mut paths = vec![format!("{ROOT}/examples/editorial_demo.intent.json")];
    let mut public: Vec<_> = std::fs::read_dir(format!("{ROOT}/examples/public"))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.to_string_lossy().ends_with(".intent.json"))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    public.sort();
    paths.extend(public);
    assert!(paths.len() >= 2);
    let neutral = ReferencePrinciples::default();
    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let intent = CreativeIntent::from_json(&text).unwrap();
        assert_eq!(
            compile_json(&intent, Some(&neutral)),
            compile_json(&intent, None),
            "{path}"
        );
    }
}
