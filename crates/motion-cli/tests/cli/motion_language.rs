//! Motion languages with the real font measurer (the same one the CLI uses),
//! plus a byte-identical determinism check through the real binary.
//! Structure only: validity, bindings, determinism.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::compiler::{compile, AssetLibrary, FontSet};
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject};
use motion_core::validate::validate;
use motion_core::{CreativeIntent, StyleProfile};
use motion_render::FontMeasure;
use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const INTENTS: &[&str] = &[
    "examples/public/minimal-emphasize.intent.json",
    "examples/public/minimal-contrast.intent.json",
    "examples/public/three-beat-story.intent.json",
    "examples/editorial_demo.intent.json",
    "examples/motion_language_demo.intent.json",
    "golden/fixtures/benchmark/notification-fragmentation-01/attempt-01.intent.json",
    "golden/fixtures/benchmark/salary-grocery-pressure-01/attempt-01.intent.json",
];

const LANGUAGES: &[&str] = &[
    "auto",
    "minimal",
    "kinetic",
    "parallax",
    "sequential",
    "data",
];

fn style_with(language: &str) -> StyleProfile {
    let mut v: Value = serde_json::from_str(&read("examples/public/minimal.style.json")).unwrap();
    v["motion_language"] = json!(language);
    serde_json::from_value(v).expect("style parses")
}

fn compile_real(intent: &CreativeIntent, style: &StyleProfile) -> MotionProject {
    let assets = root().join("assets");
    let paths: Vec<PathBuf> = FontSet::for_style(style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(paths.iter().map(PathBuf::as_path)).expect("fonts load");
    compile(intent, style, &AssetLibrary::new(&assets), &measure).expect("compiles")
}

fn layers<'a>(ls: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in ls {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            layers(children, out);
        }
    }
}

#[test]
fn every_language_compiles_and_validates_with_real_fonts() {
    for lang in LANGUAGES {
        let style = style_with(lang);
        for rel in INTENTS {
            let intent = CreativeIntent::from_json(&read(rel)).unwrap();
            let p = compile_real(&intent, &style);
            validate(&p, Some(&root().join("assets")))
                .unwrap_or_else(|e| panic!("{lang} x {rel}: {e}"));
            for s in &p.scenes {
                for m in &s.motions {
                    assert!(
                        m.start + m.duration <= s.duration_seconds + 1e-6,
                        "{lang} x {rel}: '{}' outlives scene '{}'",
                        m.target,
                        s.id
                    );
                }
            }
        }
    }
}

#[test]
fn notification_carried_primary_binds_to_the_compressing_panel() {
    let rel = "golden/fixtures/benchmark/notification-fragmentation-01";
    let intent =
        CreativeIntent::from_json(&read(&format!("{rel}/attempt-01.intent.json"))).unwrap();
    let style = StyleProfile::from_json(&read(&format!("{rel}/style.json"))).unwrap();
    let a = serde_json::to_string(&compile_real(&intent, &style)).unwrap();
    let b = serde_json::to_string(&compile_real(&intent, &style)).unwrap();
    assert_eq!(a, b, "two compiles must be byte-identical");

    let p = compile_real(&intent, &style);
    let one_task = p
        .shared
        .iter()
        .find(|s| s.id == "one_task")
        .expect("shared");
    let key = one_task
        .track
        .iter()
        .find(|k| k.layout.is_some())
        .expect("a binding key");
    let parent = &key.layout.as_ref().unwrap().parent;
    let scene = p.scenes.iter().find(|s| s.id == key.scene).unwrap();
    let mut all = Vec::new();
    layers(&scene.layers, &mut all);
    let panel = all
        .iter()
        .find(|l| &l.id == parent)
        .expect("panel exists in the key's scene");
    assert!(scene
        .motions
        .iter()
        .any(|m| m.target == panel.id && matches!(m.op, MotionOp::AccentExpand { .. })));
    assert!(
        one_task
            .track
            .iter()
            .filter(|k| k.scene == key.scene)
            .all(|k| k.state.y.is_none() && k.state.x.is_none()),
        "no manual position keys in the bound scene"
    );
}

fn motion(args: &[&Path]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .args(args)
        .output()
        .expect("run motion-engine")
}

#[test]
fn cli_compile_is_byte_identical_and_accepts_every_language() {
    let dir = std::env::temp_dir().join(format!("motion_language_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rel = "golden/fixtures/benchmark/notification-fragmentation-01";
    let intent = root().join(format!("{rel}/attempt-01.intent.json"));

    for lang in LANGUAGES {
        let mut v: Value = serde_json::from_str(&read(&format!("{rel}/style.json"))).unwrap();
        v["motion_language"] = json!(lang);
        let style = dir.join(format!("{lang}.style.json"));
        std::fs::write(&style, serde_json::to_string_pretty(&v).unwrap()).unwrap();
        let (o1, o2) = (
            dir.join(format!("{lang}.1.json")),
            dir.join(format!("{lang}.2.json")),
        );
        for out in [&o1, &o2] {
            let r = motion(&[
                Path::new("compile"),
                &intent,
                Path::new("--style"),
                &style,
                Path::new("--output"),
                out,
            ]);
            assert!(
                r.status.success(),
                "{lang}: compile failed: {}",
                String::from_utf8_lossy(&r.stderr)
            );
        }
        assert_eq!(
            std::fs::read(&o1).unwrap(),
            std::fs::read(&o2).unwrap(),
            "{lang}: compiled files differ between runs"
        );
        let v = motion(&[Path::new("validate"), &o1]);
        assert!(v.status.success(), "{lang}: validate failed");
    }

    // An unknown language is rejected by the CLI with a non-zero exit.
    let mut v: Value = serde_json::from_str(&read(&format!("{rel}/style.json"))).unwrap();
    v["motion_language"] = json!("wobbly");
    let bad = dir.join("bad.style.json");
    std::fs::write(&bad, serde_json::to_string(&v).unwrap()).unwrap();
    let r = motion(&[
        Path::new("compile"),
        &intent,
        Path::new("--style"),
        &bad,
        Path::new("--output"),
        &dir.join("bad.json"),
    ]);
    assert!(!r.status.success(), "unknown motion_language must fail");
    let _ = std::fs::remove_dir_all(&dir);
}
