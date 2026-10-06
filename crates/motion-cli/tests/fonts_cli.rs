//! (0.9) `fonts` subcommands and the typography flags through the real binary.

use std::path::PathBuf;
use std::process::{Command, Output};

use motion_core::compiler::typography::{option_fontset, option_fontset_in};
use motion_core::scene::{Color, FontRole, TextAlign, TextStyle};
use motion_render::text::TextEngine;
use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

fn motion(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .args(args)
        .output()
        .expect("run motion-engine")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_fonts_cli_{}", std::process::id()))
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fonts_list_json_parses() {
    let o = motion(&["fonts", "list", "--json"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).expect("json");
    assert_eq!(v["faces"].as_array().unwrap().len(), 55);
    assert_eq!(v["options"].as_array().unwrap().len(), 33);
    assert!(v["errors"].as_array().unwrap().is_empty());
    let face = &v["faces"][0];
    for key in [
        "id",
        "file",
        "family",
        "weight",
        "italic",
        "file_exists",
        "coverage",
    ] {
        assert!(face.get(key).is_some(), "face key {key}");
    }
    let trust1 = v["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "trust.1")
        .unwrap();
    assert_eq!(trust1["available"], true);
    let trust2 = v["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["id"] == "luxury.1")
        .unwrap();
    assert_eq!(trust2["available"], false);
    assert!(!trust2["problems"].as_array().unwrap().is_empty());
}

#[test]
fn specimen_writes_a_png_with_one_block_per_option() {
    let s = Scratch::new("specimen");
    let out = s.path("s.png");
    let o = motion(&["fonts", "specimen", "--emotion", "drama", "-o", &out]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("3 option block(s)"), "{}", stdout(&o));
    let png = std::fs::read(&out).expect("png");
    assert_eq!(&png[1..4], b"PNG");
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    assert!(width > 0 && height >= 3 * 400, "{width}x{height}");

    let bad = motion(&["fonts", "specimen", "--emotion", "nope", "-o", &out]);
    assert!(!bad.status.success());
    let neither = motion(&["fonts", "specimen", "-o", &out]);
    assert!(!neither.status.success());
}

fn rupee_intent(s: &Scratch) -> String {
    let text =
        std::fs::read_to_string(root().join("examples/motion_language_demo.intent.json")).unwrap();
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["beats"][0]["statement"] = Value::String("\u{20B9}50,000 saved every year.".into());
    let path = s.path("rupee.intent.json");
    std::fs::write(&path, v.to_string()).unwrap();
    path
}

#[test]
fn compile_emotion_records_typography_and_renders() {
    let s = Scratch::new("compile");
    let intent = rupee_intent(&s);
    let scene = s.path("rupee.motion.json");
    let o = motion(&["compile", &intent, "--emotion", "energy", "-o", &scene]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&scene).unwrap()).unwrap();
    let t = &v["theme"]["typography"];
    assert_eq!(t["emotion"], "energy");
    let option = t["option"].as_str().expect("option recorded");
    let set = option_fontset_in(option, &root().join("assets")).expect("option font set");
    for (role, face) in &set.roles {
        let key = serde_json::to_value(role).unwrap();
        assert_eq!(v["theme"]["fonts"][key.as_str().unwrap()], face.asset_id);
    }
    for face in &set.faces {
        assert!(
            v["assets"]
                .as_array()
                .unwrap()
                .iter()
                .any(|a| a["id"] == face.asset_id && a["path"] == face.path),
            "{} missing from scene assets",
            face.asset_id
        );
    }

    // The scene renders a frame.
    let frames = s.path("frames");
    let r = motion(&[
        "render",
        &scene,
        "--out-dir",
        &frames,
        "--frame",
        "20",
        "--no-video",
    ]);
    assert!(r.status.success(), "{}", stderr(&r));
    assert!(PathBuf::from(&frames).join("frame_000020.png").is_file());

    // Flags absent: no typography record, same bytes as before.
    let plain = s.path("plain.motion.json");
    let a = motion(&["compile", &intent, "-o", &plain]);
    assert!(a.status.success());
    let pv: Value = serde_json::from_str(&std::fs::read_to_string(&plain).unwrap()).unwrap();
    assert!(pv["theme"].get("typography").is_none());
    let explicit0 = s.path("explicit0.motion.json");
    let b = motion(&[
        "compile",
        &intent,
        "--explore",
        "0",
        "--seed",
        "9",
        "-o",
        &explicit0,
    ]);
    assert!(b.status.success());
    assert_eq!(
        std::fs::read(&plain).unwrap(),
        std::fs::read(&explicit0).unwrap()
    );
}

fn style(text: &str, role: FontRole, size: f32) -> TextStyle {
    TextStyle {
        text: text.to_string(),
        font_role: role,
        font_size: size,
        font_weight: 400,
        italic: false,
        color: Color::rgb(0, 0, 0),
        align: TextAlign::Left,
        line_height: 1.2,
        letter_spacing: 0.0,
        max_width: None,
        uppercase: false,
        ink: None,
    }
}

/// The energy option's display face (Archivo Black) has no rupee sign; the
/// option's FontSet carries the fallback face so the glyph is still inked.
#[test]
fn rupee_sign_renders_through_the_fallback_face() {
    let set = option_fontset("energy.3").expect("energy.3");
    assert_eq!(set.role(FontRole::Display).asset_id, "font.archivo_black");
    let mut engine = TextEngine::new();
    let mut display_family = String::new();
    for face in &set.faces {
        let family = engine.load(&root().join("assets").join(face.path)).unwrap();
        if face.asset_id == "font.archivo_black" {
            display_family = family;
        }
    }
    let rupee = engine
        .outline(
            &display_family,
            &style("\u{20B9}", FontRole::Display, 120.0),
            2000.0,
        )
        .expect("rupee outline");
    let b = rupee.bounds();
    assert!(b.width() > 20.0 && b.height() > 20.0, "inked: {b:?}");
    let with = engine
        .outline(
            &display_family,
            &style("\u{20B9}50,000", FontRole::Display, 120.0),
            2000.0,
        )
        .unwrap();
    let without = engine
        .outline(
            &display_family,
            &style("50,000", FontRole::Display, 120.0),
            2000.0,
        )
        .unwrap();
    assert!(with.bounds().width() > without.bounds().width() + 20.0);
}

#[test]
fn resolve_style_reports_typography() {
    let o = motion(&[
        "resolve-style",
        "examples/editorial_demo.intent.json",
        "--style",
        "examples/taste/warm_editorial.style.json",
        "--json",
        "--explore",
        "1",
        "--seed",
        "3",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).unwrap();
    let t = &v["typography"];
    assert!(t["emotion"].is_string());
    assert!(t["option"].is_string(), "{t}");
    assert_eq!(t["faces"].as_object().unwrap().len(), 6);

    let text = motion(&[
        "resolve-style",
        "examples/editorial_demo.intent.json",
        "--emotion",
        "calm",
    ]);
    assert!(text.status.success(), "{}", stderr(&text));
    assert!(
        stdout(&text).contains("Typography choice"),
        "{}",
        stdout(&text)
    );

    let level0 = motion(&[
        "resolve-style",
        "examples/editorial_demo.intent.json",
        "--json",
    ]);
    let v: Value = serde_json::from_str(&stdout(&level0)).unwrap();
    assert!(v["typography"]["option"].is_null());
    assert!(v["typography"]["legacy"].is_string());
}
