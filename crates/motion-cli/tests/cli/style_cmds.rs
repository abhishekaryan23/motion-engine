//! `compare-styles` and `style-preview` through the real binary.

use std::path::PathBuf;
use std::process::{Command, Output};

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

/// Scratch directory unique to one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_style_cmds_{}", std::process::id()))
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
    fn file(&self, name: &str, content: &str) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, content).expect("write scratch file");
        path.to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn differing(o: &Output) -> u64 {
    let v: Value = serde_json::from_str(&stdout(o)).expect("json output");
    v["differing"].as_u64().expect("differing count")
}

#[test]
fn compare_warm_editorial_and_dark_technical_differ_widely() {
    let a = "examples/taste/warm_editorial.style.json";
    let b = "examples/taste/dark_technical.style.json";
    let human = motion(&["compare-styles", a, b]);
    assert!(human.status.success());
    let text = stdout(&human);
    assert!(text.contains("Palette"), "{text}");
    assert!(text.contains("DIFFERENT"), "{text}");
    assert!(text.contains("dimensions differ"), "{text}");
    let json = motion(&["compare-styles", a, b, "--json"]);
    assert!(json.status.success());
    assert!(differing(&json) >= 8, "{}", stdout(&json));
}

#[test]
fn accent_change_alone_is_not_a_different_style() {
    let s = Scratch::new("accent");
    let a = s.file("a.style.json", "{}");
    let b = s.file("b.style.json", r#"{"accent_role":"cobalt"}"#);
    let human = motion(&["compare-styles", &a, &b]);
    assert!(human.status.success());
    assert!(stdout(&human).contains("0/11 dimensions differ"));
    let json = motion(&["compare-styles", &a, &b, "--json"]);
    assert!(json.status.success());
    assert_eq!(differing(&json), 0);
}

#[test]
fn style_preview_writes_png_and_describes_temporal_character() {
    let s = Scratch::new("preview");
    let out = s.0.join("preview.png");
    let out_str = out.to_string_lossy().into_owned();
    let o = motion(&[
        "style-preview",
        "examples/taste/dark_technical.style.json",
        "--output",
        &out_str,
    ]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let text = stdout(&o);
    for key in [
        "Motion temperament:",
        "Transition character:",
        "Density:",
        "Composition rhythm:",
        "Scale contrast:",
    ] {
        assert!(text.contains(key), "missing {key}\n{text}");
    }
    let png = std::fs::read(&out).expect("png written");
    assert_eq!(&png[1..4], b"PNG");

    let j = motion(&[
        "style-preview",
        "examples/taste/dark_technical.style.json",
        "--output",
        &out_str,
        "--json",
    ]);
    assert!(j.status.success());
    let v: Value = serde_json::from_str(&stdout(&j)).expect("profile json");
    assert_eq!(v["tone"], "technical");
}
