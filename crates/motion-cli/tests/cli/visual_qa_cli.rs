//! `qa --reference-style`: structural typography-dominance report through the real binary.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

const INTENT: &str = "examples/editorial_demo.intent.json";
const STYLE: &str = "examples/editorial_demo.style.json";
const DIAGRAMMATIC: &str = "crates/motion-cli/tests/fixtures/visual_diagrammatic.json";
const TYPE_LED: &str = "crates/motion-cli/tests/fixtures/visual_type_led.json";

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

/// Per-test scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_visual_qa_{}", std::process::id()))
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compile_demo(out: &Path) {
    let o = motion(&[
        "compile",
        INTENT,
        "--style",
        STYLE,
        "--output",
        out.to_str().expect("utf8 path"),
    ]);
    assert!(o.status.success(), "compile failed: {}", stderr(&o));
}

#[test]
fn diagrammatic_reference_warns_on_typographic_output() {
    let s = Scratch::new("warn");
    let scene = s.path("demo.motion.json");
    compile_demo(&scene);
    let o = motion(&[
        "qa",
        scene.to_str().expect("utf8 path"),
        "--step",
        "60",
        "--reference-style",
        DIAGRAMMATIC,
    ]);
    assert!(o.status.success(), "qa failed: {}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("\nlifecycle\n"), "existing output stays first");
    assert!(out.contains("pure-type ratio"), "{out}");
    assert!(
        out.contains("WARNING: reference visual language is visual/object-led, but "),
        "{out}"
    );
    assert!(
        out.contains("Visual-language transfer likely failed."),
        "{out}"
    );
}

#[test]
fn type_led_reference_does_not_warn() {
    let s = Scratch::new("typeled");
    let scene = s.path("demo.motion.json");
    compile_demo(&scene);
    let o = motion(&[
        "qa",
        scene.to_str().expect("utf8 path"),
        "--step",
        "60",
        "--reference-style",
        TYPE_LED,
    ]);
    assert!(o.status.success(), "qa failed: {}", stderr(&o));
    assert!(!stdout(&o).contains("WARNING: reference visual language"));
}

#[test]
fn without_reference_reports_but_has_no_verdict() {
    let s = Scratch::new("noref");
    let scene = s.path("demo.motion.json");
    compile_demo(&scene);
    let o = motion(&[
        "qa",
        scene.to_str().expect("utf8 path"),
        "--step",
        "60",
        "--json",
    ]);
    assert!(o.status.success(), "qa failed: {}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).expect("json");
    assert!(v["profile"].is_object() && v["scenes"].is_array());
    assert!(v["visual"]["beats"]
        .as_array()
        .is_some_and(|b| !b.is_empty()));
    assert!(v["visual"]["verdict"].is_null());
}

#[test]
fn json_carries_verdict_with_reference() {
    let s = Scratch::new("json");
    let scene = s.path("demo.motion.json");
    compile_demo(&scene);
    let o = motion(&[
        "qa",
        scene.to_str().expect("utf8 path"),
        "--step",
        "60",
        "--json",
        "--reference-style",
        DIAGRAMMATIC,
    ]);
    assert!(o.status.success(), "qa failed: {}", stderr(&o));
    let v: Value = serde_json::from_str(&stdout(&o)).expect("json");
    assert_eq!(v["visual"]["verdict"], "visual_transfer_likely_failed");
}
