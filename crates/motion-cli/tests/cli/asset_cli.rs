//! `asset-prompts`, `ingest-assets`, `validate-assets` through the real binary.
//! Prompt sets are written by hand (independent of prompt derivation).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use motion_core::assets::*;
use serde_json::Value;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
}

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

fn fixture(name: &str) -> PathBuf {
    root()
        .join("crates/motion-render/tests/fixtures")
        .join(name)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("asset_cli")
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("delivery")).expect("mkdir");
        Scratch(dir)
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

fn spec(id: &str, stem: &str, priority: Priority, alpha: bool) -> AssetPromptSpec {
    let mut s = AssetPromptSpec {
        id: id.into(),
        fingerprint: String::new(),
        continuity_key: "office_worker".into(),
        serves: vec![id.into()],
        role: AssetRole::HeroSubject,
        priority,
        subject: format!("subject of {id}"),
        context: String::new(),
        presentation: if alpha {
            Presentation::IsolatedCutout
        } else {
            Presentation::FullFrame
        },
        background: if alpha {
            Background::Transparent
        } else {
            Background::Opaque
        },
        style: AssetStyleProfile {
            medium: "photograph".into(),
            realism: "natural".into(),
            lighting: "soft".into(),
            contrast: "medium".into(),
            palette_tendency: "neutral".into(),
            edge_treatment: "clean".into(),
            shadow_treatment: "none".into(),
            camera_feel: "documentary".into(),
            background_behavior: "plain".into(),
        },
        composition: PromptComposition {
            framing: Framing::HalfFigure,
            negative_space: NegativeSpace::None,
            subject_whole: false,
            head_inside_frame: true,
        },
        avoid: vec!["watermark".into()],
        output: PromptOutput {
            file_stem: stem.into(),
            alpha_required: alpha,
            min_short_side: 512,
            aspect: "3:4".into(),
        },
        prompt: String::new(),
    };
    s.fingerprint = s.compute_fingerprint();
    s
}

fn write_prompts(s: &Scratch, specs: Vec<AssetPromptSpec>) -> PathBuf {
    let set = AssetPromptSet {
        version: ASSET_PROMPTS_VERSION.into(),
        specs,
    };
    let path = s.path("prompts.json");
    std::fs::write(&path, set.to_json_pretty()).expect("write prompts");
    path
}

fn ingest_cmd(s: &Scratch, prompts: &Path, extra: &[&str]) -> Output {
    bin()
        .arg("ingest-assets")
        .arg(prompts)
        .arg(s.path("delivery"))
        .arg("-o")
        .arg(s.path("out/manifest.json"))
        .args(extra)
        .output()
        .expect("run")
}

fn text(o: &[u8]) -> String {
    String::from_utf8_lossy(o).into_owned()
}

#[test]
fn ingest_exits_zero_with_only_warnings() {
    let s = Scratch::new("warnings");
    std::fs::copy(
        fixture("cutout_person_768x1024.png"),
        s.path("delivery/worker-aaaa.png"),
    )
    .expect("copy");
    let prompts = write_prompts(
        &s,
        vec![
            spec(
                "beat_1.hero_subject",
                "worker-aaaa",
                Priority::Required,
                true,
            ),
            spec(
                "beat_2.environment",
                "plate-bbbb",
                Priority::Optional,
                false,
            ),
        ],
    );
    let out = ingest_cmd(&s, &prompts, &[]);
    let stdout = text(&out.stdout);
    assert!(out.status.success(), "{stdout}\n{}", text(&out.stderr));
    assert!(stdout.contains("beat_2.environment  WARNING"), "{stdout}");
    assert!(stdout.contains("assets: 2 (0 fail, "), "{stdout}");

    let manifest = AssetManifest::from_json(
        &std::fs::read_to_string(s.path("out/manifest.json")).expect("manifest"),
    )
    .expect("parse manifest");
    assert_eq!(manifest.version, ASSET_MANIFEST_VERSION);
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(manifest.assets[0].path, "../delivery/worker-aaaa.png");
    assert_eq!(manifest.missing.len(), 1);
    assert_eq!(manifest.missing[0].priority, Priority::Optional);
}

#[test]
fn ingest_exits_one_when_a_required_asset_fails() {
    let s = Scratch::new("required_fail");
    let prompts = write_prompts(
        &s,
        vec![spec(
            "beat_1.hero_subject",
            "worker-cccc",
            Priority::Required,
            true,
        )],
    );
    let out = ingest_cmd(&s, &prompts, &[]);
    assert_eq!(out.status.code(), Some(1));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("FAIL     delivery"), "{stdout}");
    // The manifest is still written and records the gap.
    let manifest = AssetManifest::from_json(
        &std::fs::read_to_string(s.path("out/manifest.json")).expect("manifest"),
    )
    .expect("parse manifest");
    assert_eq!(manifest.missing_required().len(), 1);
}

#[test]
fn ingest_json_output_parses() {
    let s = Scratch::new("ingest_json");
    std::fs::copy(
        fixture("cutout_person_768x1024.png"),
        s.path("delivery/worker-dddd.png"),
    )
    .expect("copy");
    let prompts = write_prompts(
        &s,
        vec![spec(
            "beat_1.hero_subject",
            "worker-dddd",
            Priority::Required,
            true,
        )],
    );
    let out = ingest_cmd(&s, &prompts, &["--json"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["ok"], Value::Bool(true));
    assert_eq!(v["assets"][0]["id"], "beat_1.hero_subject");
}

#[test]
fn validate_assets_json_parses_and_exit_code_follows_fail() {
    let s = Scratch::new("validate");
    std::fs::copy(
        fixture("cutout_person_768x1024.png"),
        s.path("delivery/worker-eeee.png"),
    )
    .expect("copy");
    let prompts = write_prompts(
        &s,
        vec![spec(
            "beat_1.hero_subject",
            "worker-eeee",
            Priority::Required,
            true,
        )],
    );
    assert!(ingest_cmd(&s, &prompts, &[]).status.success());

    let out = bin()
        .arg("validate-assets")
        .arg(s.path("out/manifest.json"))
        .arg("--prompts")
        .arg(&prompts)
        .arg("--json")
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["ok"], Value::Bool(true));
    assert!(v["assets"].as_array().is_some_and(|a| !a.is_empty()));

    // Text mode, then break the file: exit 1 with a `decode` FAIL.
    let out = bin()
        .arg("validate-assets")
        .arg(s.path("out/manifest.json"))
        .output()
        .expect("run");
    assert!(out.status.success());
    assert!(text(&out.stdout).contains("assets: 1 (0 fail"));
    std::fs::write(s.path("delivery/worker-eeee.png"), b"garbage").expect("write");
    let out = bin()
        .arg("validate-assets")
        .arg(s.path("out/manifest.json"))
        .arg("--json")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["ok"], Value::Bool(false));
}

#[test]
fn ingest_rejects_missing_inputs() {
    let s = Scratch::new("bad_inputs");
    let out = ingest_cmd(&s, &s.path("nope.json"), &[]);
    assert!(!out.status.success());
    let out = bin()
        .arg("asset-prompts")
        .arg(s.path("nope.json"))
        .output()
        .expect("run");
    assert!(!out.status.success());
}

#[test]
fn asset_prompts_from_a_plan() {
    let s = Scratch::new("asset_prompts");
    let plan = bin()
        .arg("plan-assets")
        .arg(root().join("examples/editorial_demo.intent.json"))
        .arg("--style")
        .arg(root().join("examples/editorial_demo.style.json"))
        .arg("--assets")
        .arg(root().join("assets"))
        .arg("-o")
        .arg(s.path("plan.json"))
        .output()
        .expect("run");
    assert!(plan.status.success(), "{}", text(&plan.stderr));
    let out = bin()
        .arg("asset-prompts")
        .arg(s.path("plan.json"))
        .arg("-o")
        .arg(s.path("prompts.json"))
        .output()
        .expect("run");
    assert!(out.status.success(), "{}", text(&out.stderr));
    let set = AssetPromptSet::from_json(
        &std::fs::read_to_string(s.path("prompts.json")).expect("prompts"),
    )
    .expect("parse");
    assert_eq!(set.version, ASSET_PROMPTS_VERSION);
}
