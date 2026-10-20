//! End-to-end check of the public CLI surface with the real binary:
//! compile -> validate -> render (single frame) -> inspect, plus a negative
//! case. Runs from the workspace root because `--assets` defaults to `assets`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn workspace_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

/// Per-test scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_public_{}", std::process::id()))
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
        // Remove the per-process parent once its last scratch dir is gone.
        if let Some(parent) = self.0.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn motion(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(workspace_root())
        .args(args)
        .output()
        .expect("run motion-engine")
}

fn p(s: &str) -> &Path {
    Path::new(s)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_success(what: &str, out: &Output) {
    assert!(
        out.status.success(),
        "{what} failed ({}):\nstdout: {}\nstderr: {}",
        out.status,
        text(&out.stdout),
        text(&out.stderr)
    );
}

fn public_flow(name: &str) {
    let root = workspace_root();
    let scratch = Scratch::new(name);
    let dir = &scratch.0;
    let intent = root.join(format!("examples/public/{name}.intent.json"));
    let style = root.join("examples/public/minimal.style.json");
    let scene = dir.join("x.motion.json");
    let render_dir = dir.join("render");

    let out = motion(&[
        p("compile"),
        &intent,
        p("--style"),
        &style,
        p("--output"),
        &scene,
    ]);
    assert_success("compile", &out);
    assert!(scene.is_file(), "compiled scene written");

    let out = motion(&[p("validate"), &scene]);
    assert_success("validate", &out);
    let stdout = text(&out.stdout);
    assert!(stdout.starts_with("ok:"), "validate stdout: {stdout}");

    let out = motion(&[
        p("render"),
        &scene,
        p("--out-dir"),
        &render_dir,
        p("--frame"),
        p("60"),
    ]);
    assert_success("render", &out);
    assert!(
        render_dir.join("frame_000060.png").is_file(),
        "frame_000060.png missing in {}",
        render_dir.display()
    );

    let out = motion(&[p("inspect"), &scene, p("--frame"), p("60")]);
    assert_success("inspect", &out);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("inspect prints JSON");
    assert!(v["layers"].is_array(), "inspect JSON has a `layers` array");
}

#[test]
fn minimal_emphasize_compile_validate_render_inspect() {
    public_flow("minimal-emphasize");
}

#[test]
fn minimal_contrast_compile_validate_render_inspect() {
    public_flow("minimal-contrast");
}

#[test]
fn v0_2_collection_compile_validate_render_inspect() {
    public_flow("collection-accumulate");
}

#[test]
fn v0_2_state_change_compile_validate_render_inspect() {
    public_flow("state-change");
}

#[test]
fn v0_2_derived_metric_compile_validate_render_inspect() {
    public_flow("derived-metric");
}

#[test]
fn compile_rejects_invalid_enum_value_with_message() {
    let scratch = Scratch::new("invalid_enum");
    let dir = &scratch.0;
    let root = workspace_root();
    let mut intent: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("examples/public/minimal-emphasize.intent.json"))
            .expect("read example"),
    )
    .expect("parse example");
    intent["beats"][0]["purpose"] = serde_json::json!("summarize");
    let bad = dir.join("bad.intent.json");
    std::fs::write(&bad, serde_json::to_string_pretty(&intent).unwrap()).unwrap();
    let scene = dir.join("bad.motion.json");

    let out = motion(&[p("compile"), &bad, p("--output"), &scene]);
    assert!(!out.status.success(), "compile of invalid intent must fail");
    let stderr = text(&out.stderr);
    assert!(
        stderr.contains("summarize"),
        "error should mention the bad value, got: {stderr}"
    );
    assert!(!scene.exists(), "no scene written on failure");
}

/// (0.22) A full render deletes its PNG frames once the MP4 is encoded;
/// `--keep-frames` keeps them. A 64x64, six-frame scene keeps it fast.
#[test]
fn full_render_drops_frames_after_encoding_unless_kept() {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("skipped: no ffmpeg");
        return;
    }
    let scratch = Scratch::new("drop_frames");
    let dir = &scratch.0;
    let scene = dir.join("tiny.motion.json");
    std::fs::write(
        &scene,
        r##"{"version":"0.2","project":{"name":"tiny","duration_seconds":0.2},
            "canvas":{"width":64,"height":64,"fps":30,"background":"#E9E6E1"},
            "scenes":[{"id":"backdrop","start_seconds":0.0,"duration_seconds":0.2}]}"##,
    )
    .unwrap();
    let pngs = |d: &Path| {
        std::fs::read_dir(d)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().is_some_and(|x| x == "png"))
                    .count()
            })
            .unwrap_or(0)
    };

    let dropped = dir.join("dropped");
    let out = motion(&[p("render"), &scene, p("--out-dir"), &dropped]);
    assert_success("render", &out);
    assert!(dropped.join("tiny.mp4").is_file(), "mp4 written");
    assert_eq!(pngs(&dropped), 0, "frames deleted after encoding");

    let kept = dir.join("kept");
    let out = motion(&[
        p("render"),
        &scene,
        p("--out-dir"),
        &kept,
        p("--keep-frames"),
    ]);
    assert_success("render --keep-frames", &out);
    assert!(kept.join("tiny.mp4").is_file(), "mp4 written");
    assert_eq!(pngs(&kept), 6, "--keep-frames keeps all six frames");
}
