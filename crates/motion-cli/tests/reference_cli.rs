//! Reference commands (`validate-reference-style`, `resolve-style`,
//! `--reference-style`) through the real binary. `reference-evidence` needs
//! ffmpeg and is covered elsewhere.

use std::path::PathBuf;
use std::process::{Command, Output};

use motion_core::compiler::FontSet;
use motion_core::{compile_with_assets, AssetLibrary, CreativeIntent, StyleProfile};
use motion_render::FontMeasure;
use serde_json::Value;

const INTENT: &str = "examples/public/minimal-contrast.intent.json";
const DARK: &str = "crates/motion-cli/tests/fixtures/reference_dark_technical.json";
const INVALID: &str = "crates/motion-cli/tests/fixtures/reference_invalid.json";

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

/// Scratch directory unique to one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_reference_cli_{}", std::process::id()))
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
    fn path(&self, name: &str) -> String {
        self.0.join(name).to_string_lossy().into_owned()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn json_of(o: &Output) -> Value {
    serde_json::from_str(&stdout(o)).expect("json output")
}

fn read_json(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read file")).expect("json file")
}

fn coverage_status(v: &Value, dimension: &str) -> String {
    v["coverage"]["rows"]
        .as_array()
        .expect("coverage rows")
        .iter()
        .find(|r| r["dimension"] == dimension)
        .and_then(|r| r["status"].as_str())
        .unwrap_or_default()
        .to_string()
}

#[test]
fn validate_valid_profile_prints_fidelity_rows() {
    let o = motion(&["validate-reference-style", DARK]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("valid ReferenceStyleProfile v0.1"), "{out}");
    assert!(out.contains("fidelity"), "{out}");
    let polarity = out
        .lines()
        .find(|l| l.starts_with("polarity"))
        .expect("polarity row");
    assert!(
        polarity.contains("dark") && polarity.contains("exact"),
        "{polarity}"
    );
    assert!(out.contains("unsupported traits:"), "{out}");
}

#[test]
fn validate_reports_accepted_aliases() {
    let s = Scratch::new("aliases");
    let profile = s.file(
        "alias.json",
        r#"{ "version": "0.1", "polarity": { "value": "very dark", "confidence": 0.8 } }"#,
    );
    let o = motion(&["validate-reference-style", &profile]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("aliases:"), "{out}");
    let alias_line = out
        .lines()
        .find(|l| l.contains("very dark"))
        .expect("alias line");
    assert!(
        alias_line.contains("polarity") && alias_line.contains("dark"),
        "{alias_line}"
    );

    let j = motion(&["validate-reference-style", &profile, "--json"]);
    let v = json_of(&j);
    assert_eq!(v["valid"], true);
    assert_eq!(v["aliases"][0]["from"], "very dark");
    assert_eq!(v["aliases"][0]["to"], "dark");
    assert_eq!(v["normalized"]["principles"]["polarity"], "dark");
}

#[test]
fn validate_invalid_profile_fails_and_writes_repair_request() {
    let s = Scratch::new("repair");
    let repair = s.path("repair.json");
    let o = motion(&[
        "validate-reference-style",
        INVALID,
        "--repair-request",
        &repair,
    ]);
    assert!(!o.status.success());
    let out = stdout(&o);
    assert!(out.contains("neon_bloom"), "{out}");

    let req = read_json(&repair);
    assert_eq!(req["repair_attempt"], 1);
    assert_eq!(req["reference_fingerprint"], "rf1-0123456789abcdef");
    let issues = req["issues"].as_array().expect("issues");
    assert!(!issues.is_empty());
    assert!(issues
        .iter()
        .any(|i| i.as_str().is_some_and(|t| t.contains("neon_bloom"))));
    assert!(req["previous_response"]
        .as_str()
        .is_some_and(|t| t.contains("neon_bloom")));
}

#[test]
fn validate_rule_violations_are_listed() {
    let s = Scratch::new("confidence");
    let profile = s.file(
        "conf.json",
        r#"{ "version": "0.1", "polarity": { "value": "dark", "confidence": 1.4 } }"#,
    );
    let o = motion(&["validate-reference-style", &profile, "--json"]);
    assert!(!o.status.success());
    let v = json_of(&o);
    assert_eq!(v["valid"], false);
    assert_eq!(v["issues"][0]["path"], "polarity.confidence");
}

#[test]
fn resolve_style_with_reference_prints_coverage_matches() {
    let o = motion(&["resolve-style", INTENT, "--reference-style", DARK]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    assert!(out.contains("palette_family"), "{out}");
    assert!(out.contains("MATCH"), "{out}");
    let polarity = out
        .lines()
        .find(|l| l.starts_with("polarity") && l.contains("MATCH"))
        .expect("polarity MATCH row");
    assert!(polarity.contains("dark"), "{polarity}");

    let plain = motion(&["resolve-style", INTENT]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    assert!(!stdout(&plain).contains("MATCH"));
}

#[test]
fn explicit_light_polarity_overrides_a_dark_reference() {
    let s = Scratch::new("override");
    let style = s.file("light.style.json", r#"{ "polarity": "light" }"#);
    let o = motion(&[
        "resolve-style",
        INTENT,
        "--style",
        &style,
        "--reference-style",
        DARK,
        "--json",
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v = json_of(&o);
    assert_eq!(v["resolved"]["palette"]["polarity"], "light");
    assert_eq!(coverage_status(&v, "polarity"), "overridden");
    assert!(v["normalized"]["principles"]["polarity"] == "dark");
}

#[test]
fn resolve_style_rejects_an_invalid_reference() {
    let o = motion(&["resolve-style", INTENT, "--reference-style", INVALID]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("neon_bloom"), "{}", stderr(&o));
}

#[test]
fn compile_with_reference_style_changes_the_scene_only_when_asked() {
    let s = Scratch::new("compile");
    let (a1, a2, b) = (s.path("a1.json"), s.path("a2.json"), s.path("b.json"));
    for out in [&a1, &a2] {
        let o = motion(&["compile", INTENT, "--output", out]);
        assert!(o.status.success(), "{}", stderr(&o));
    }
    let o = motion(&["compile", INTENT, "--output", &b, "--reference-style", DARK]);
    assert!(o.status.success(), "{}", stderr(&o));

    let text = |p: &str| std::fs::read_to_string(p).expect("scene");
    assert_eq!(text(&a1), text(&a2), "no-flag compile is deterministic");
    assert_ne!(text(&a1), text(&b));
    let (plain, referenced) = (read_json(&a1), read_json(&b));
    assert_ne!(
        plain["canvas"]["background"],
        referenced["canvas"]["background"]
    );
    assert_eq!(referenced["canvas"]["background"], "#0D1117");
}

#[test]
fn compile_without_the_flag_equals_the_library_compile() {
    let s = Scratch::new("identical");
    let out = s.path("scene.json");
    let o = motion(&["compile", INTENT, "--output", &out]);
    assert!(o.status.success(), "{}", stderr(&o));

    let root = root();
    let intent =
        CreativeIntent::from_json(&std::fs::read_to_string(root.join(INTENT)).expect("intent"))
            .expect("parse intent");
    let style = StyleProfile::default();
    let assets = root.join("assets");
    let library = AssetLibrary::new(&assets);
    let fonts: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(fonts.iter().map(PathBuf::as_path)).expect("font measure");
    let expected = compile_with_assets(
        &intent,
        &style,
        &library,
        &measure,
        &motion_core::assets::AssetManifest::empty(),
    )
    .expect("library compile");

    let mut cli = read_json(&out);
    cli.as_object_mut()
        .expect("scene object")
        .remove("asset_root");
    // Through the same text form the CLI writes (f32 values print shortest).
    let mut lib: Value = serde_json::from_str(&expected.to_json_pretty()).expect("scene json");
    lib.as_object_mut()
        .expect("scene object")
        .remove("asset_root");
    assert_eq!(cli, lib);
}

#[test]
fn plan_assets_accepts_a_reference_style() {
    let plain = motion(&["plan-assets", INTENT]);
    assert!(plain.status.success(), "{}", stderr(&plain));
    let flagged = motion(&["plan-assets", INTENT, "--reference-style", DARK]);
    assert!(flagged.status.success(), "{}", stderr(&flagged));
    let (a, b) = (json_of(&plain), json_of(&flagged));
    assert!(a.is_object() && b.is_object());
    let again = motion(&["plan-assets", INTENT]);
    assert_eq!(stdout(&plain), stdout(&again));

    let bad = motion(&["plan-assets", INTENT, "--reference-style", INVALID]);
    assert!(!bad.status.success());
}

#[test]
fn cache_dir_without_bundle_only_notes_it() {
    // --cache-dir without --bundle only notes the missing --bundle.
    let s = Scratch::new("cache_note");
    let o = motion(&[
        "validate-reference-style",
        DARK,
        "--cache-dir",
        &s.path("cache"),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stderr(&o).contains("--bundle"), "{}", stderr(&o));
}

#[test]
fn resolve_style_rejects_semantically_invalid_intent() {
    let s = Scratch::new("invalid_intent");
    let mut intent = read_json(
        root()
            .join("examples/public/derived-metric.intent.json")
            .to_str()
            .unwrap(),
    );
    intent["beats"][0]["primary"]["denominator"]["value"] = serde_json::json!(0);
    let path = s.file("intent.json", &intent.to_string());
    let o = motion(&["resolve-style", &path, "--reference-style", DARK]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("invalid intent"), "{}", stderr(&o));
}

#[test]
fn reference_evidence_cli_reuses_bundles_and_validates_provenance() {
    let s = Scratch::new("evidence_pipeline");
    let video = s.path("source.mp4");
    let made = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "color=c=0x102030:s=96x160:r=12:d=1",
            "-an",
            "-c:v",
            "libx264",
            &video,
        ])
        .output()
        .expect("ffmpeg required for reference integration");
    assert!(
        made.status.success(),
        "{}",
        String::from_utf8_lossy(&made.stderr)
    );
    let bundle = s.path("bundle");
    let cache = s.path("cache");
    let args = [
        "reference-evidence",
        &video,
        "--out-dir",
        &bundle,
        "--cache-dir",
        &cache,
    ];
    let first = motion(&args);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(stdout(&first).contains("fresh analysis"));
    let ev_path = format!("{bundle}/evidence.json");
    let original = std::fs::read(&ev_path).unwrap();
    let ev = read_json(&ev_path);
    assert_eq!(ev["metadata"]["audio_present"], false);
    assert!(ev["samples"].as_array().unwrap().len() >= 2);
    let again = motion(&args);
    assert!(again.status.success(), "{}", stderr(&again));
    assert!(stdout(&again).contains("out-dir (existing bundle)"));
    assert_eq!(original, std::fs::read(&ev_path).unwrap());
    let restored = s.path("restored");
    let o = motion(&[
        "reference-evidence",
        &video,
        "--out-dir",
        &restored,
        "--cache-dir",
        &cache,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(stdout(&o).contains("reuse:        cache"));
    assert_eq!(
        original,
        std::fs::read(format!("{restored}/evidence.json")).unwrap()
    );
    let profile = s.file(
        "observed.json",
        &serde_json::json!({
            "version": "0.1", "reference_fingerprint": ev["reference_fingerprint"],
            "polarity": {"value": "dark", "confidence": 0.9,
                "evidence": {"samples": [ev["samples"][0]["id"]], "metrics": ["color.polarity"]}}
        })
        .to_string(),
    );
    let valid = motion(&[
        "validate-reference-style",
        &profile,
        "--bundle",
        &bundle,
        "--cache-dir",
        &cache,
        "--json",
    ]);
    assert!(valid.status.success(), "{}", stderr(&valid));
    assert_eq!(json_of(&valid)["valid"], true);
    let mut wrong = read_json(&profile);
    wrong["reference_fingerprint"] = Value::String("rf1-0000000000000000".into());
    let wrong = s.file("wrong.json", &wrong.to_string());
    let invalid = motion(&["validate-reference-style", &wrong, "--bundle", &bundle]);
    assert!(!invalid.status.success());
    assert!(stdout(&invalid).contains("fingerprint"));
}

/// Benchmark scripts read the engine's own visual-language policy instead of
/// re-implementing it (0.7.1 closure).
#[test]
fn json_outputs_carry_the_engine_visual_policy() {
    let profile = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/visual_diagrammatic.json"
    );
    let intent = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/editorial_demo.intent.json"
    );
    let bin = env!("CARGO_BIN_EXE_motion-engine");
    let out = std::process::Command::new(bin)
        .args([
            "resolve-style",
            intent,
            "--reference-style",
            profile,
            "--json",
        ])
        .output()
        .expect("run resolve-style");
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["visual_policy"]["weight"], "visual");
    assert_eq!(v["visual_policy"]["prefers_procedural"], true);
    let out = std::process::Command::new(bin)
        .args(["validate-reference-style", profile, "--json"])
        .output()
        .expect("run validate");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["visual_policy"]["weight"], "visual");
    let out = std::process::Command::new(bin)
        .args(["resolve-style", intent, "--json"])
        .output()
        .expect("run resolve-style without reference");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["visual_policy"]["weight"], "neutral");
}
