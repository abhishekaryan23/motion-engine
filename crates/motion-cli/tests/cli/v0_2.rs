//! CreativeIntent v0.2 end-to-end through the real binary: determinism for every motion
//! language, mixed subject structures, collection sizes across formats, metric formats,
//! and error paths. Structure only (compile + validate succeed, output identical, exit codes);
//! no pixel or design values.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// Per-test scratch directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_v0_2_{}", std::process::id()))
            .join(tag);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // Only our own dir: removing the shared parent races with tests
        // that are creating their scratch dirs in parallel.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn motion(args: &[&Path]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .args(args)
        .output()
        .expect("run motion-engine")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn p(s: &str) -> &Path {
    Path::new(s)
}

fn compile(intent: &Path, style: &Path, out: &Path) -> Output {
    motion(&[
        p("compile"),
        intent,
        p("--style"),
        style,
        p("--output"),
        out,
    ])
}

fn compile_ok(intent: &Path, style: &Path, out: &Path) {
    let r = compile(intent, style, out);
    assert!(
        r.status.success(),
        "compile {} failed: {}",
        intent.display(),
        stderr(&r)
    );
}

fn validate_ok(scene: &Path) {
    let r = motion(&[p("validate"), scene]);
    assert!(
        r.status.success(),
        "validate {} failed: {}",
        scene.display(),
        stderr(&r)
    );
}

fn write_json(dir: &Path, name: &str, v: &Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_string_pretty(v).unwrap()).unwrap();
    path
}

fn style_file(dir: &Path, language: &str) -> PathBuf {
    let mut v: Value = serde_json::from_str(&read("examples/public/minimal.style.json")).unwrap();
    v["motion_language"] = json!(language);
    write_json(dir, &format!("{language}.style.json"), &v)
}

const LANGUAGES: &[&str] = &[
    "auto",
    "minimal",
    "kinetic",
    "parallax",
    "sequential",
    "data",
];

const V0_2_EXAMPLES: &[&str] = &[
    "examples/public/collection-accumulate.intent.json",
    "examples/public/state-change.intent.json",
    "examples/public/derived-metric.intent.json",
];

#[test]
fn v0_2_public_examples_compile_byte_identically_for_every_language() {
    let s = Scratch::new("determinism");
    for language in LANGUAGES {
        let style = style_file(&s.0, language);
        for rel in V0_2_EXAMPLES {
            let stem = Path::new(rel).file_stem().unwrap().to_string_lossy();
            let intent = root().join(rel);
            let (a, b) = (
                s.0.join(format!("{language}.{stem}.a.json")),
                s.0.join(format!("{language}.{stem}.b.json")),
            );
            compile_ok(&intent, &style, &a);
            compile_ok(&intent, &style, &b);
            assert_eq!(
                std::fs::read(&a).unwrap(),
                std::fs::read(&b).unwrap(),
                "{language} x {rel}: two compiles differ"
            );
            validate_ok(&a);
        }
    }
}

fn phrase(v: &str) -> Value {
    json!({ "kind": "phrase", "value": v })
}

fn collection(n: usize) -> Value {
    let names = ["Coffee", "Tea", "Juice", "Water", "Milk", "Soda"];
    json!({
        "kind": "collection",
        "meaning": "drinks",
        "items": names[..n].iter().map(|s| phrase(s)).collect::<Vec<_>>(),
    })
}

fn state_change(entity: &str, from: &str, to: &str) -> Value {
    json!({ "kind": "state_change", "entity": entity, "from": from, "to": to })
}

fn metric(num: f64, den: f64, format: Option<&str>) -> Value {
    let mut v = json!({
        "kind": "derived_metric",
        "numerator": { "value": num, "meaning": "hits" },
        "denominator": { "value": den, "meaning": "tries" },
    });
    if let Some(f) = format {
        v["format"] = json!(f);
    }
    v
}

fn intent(format: &str, beats: Vec<Value>) -> Value {
    json!({ "version": "0.2", "title": "edge_case", "format": format, "beats": beats })
}

fn beat(purpose: &str, primary: Value, secondary: Option<Value>, rel: Option<&str>) -> Value {
    let mut b = json!({
        "purpose": purpose,
        "statement": "A statement about the idea.",
        "energy": "building",
        "primary": primary,
    });
    if let Some(s) = secondary {
        b["secondary"] = s;
    }
    if let Some(r) = rel {
        b["relationship"] = json!(r);
    }
    b
}

/// Compile with every motion language and validate the result.
fn compile_validate_all_languages(tag: &str, doc: &Value) {
    let s = Scratch::new(tag);
    let intent = write_json(&s.0, "in.intent.json", doc);
    for language in LANGUAGES {
        let style = style_file(&s.0, language);
        let out = s.0.join(format!("{language}.motion.json"));
        let r = compile(&intent, &style, &out);
        assert!(
            r.status.success(),
            "{tag} x {language}: compile failed: {}",
            stderr(&r)
        );
        validate_ok(&out);
    }
}

#[test]
fn mixed_structures_compile_and_validate() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "collection_primary_metric_secondary",
            beat(
                "reveal",
                collection(4),
                Some(metric(30.0, 120.0, None)),
                Some("accumulate"),
            ),
        ),
        (
            "state_change_primary_number_secondary",
            beat(
                "emphasize",
                state_change("delivery", "three days", "same day"),
                Some(json!({ "kind": "number", "value": "72 hours", "meaning": "saved" })),
                None,
            ),
        ),
        (
            "atomic_primary_collection_secondary",
            beat(
                "explain",
                json!({ "kind": "phrase", "value": "Small things" }),
                Some(collection(3)),
                None,
            ),
        ),
        (
            "metric_primary_state_change_secondary",
            beat(
                "contrast",
                metric(9.0, 10.0, None),
                Some(state_change("rate", "low", "high")),
                None,
            ),
        ),
    ];
    for (tag, b) in cases {
        compile_validate_all_languages(tag, &intent("vertical", vec![b]));
    }
}

#[test]
fn small_and_large_collections_fit_every_format() {
    for format in ["vertical", "square", "landscape"] {
        for n in [2usize, 6] {
            for rel in [Some("accumulate"), None] {
                let doc = intent(format, vec![beat("emphasize", collection(n), None, rel)]);
                compile_validate_all_languages(
                    &format!("coll_{format}_{n}_{}", rel.is_some()),
                    &doc,
                );
            }
        }
    }
}

#[test]
fn state_change_and_metric_compile_in_square_and_landscape() {
    for format in ["square", "landscape"] {
        let doc = intent(
            format,
            vec![
                beat(
                    "contrast",
                    state_change("delivery", "slow", "fast"),
                    Some(state_change("cost", "high", "low")),
                    None,
                ),
                beat(
                    "compare",
                    metric(120.0, 4000.0, None),
                    Some(metric(90.0, 2000.0, None)),
                    None,
                ),
            ],
        );
        compile_validate_all_languages(&format!("mixed_{format}"), &doc);
    }
}

#[test]
fn every_metric_format_compiles_and_validates() {
    for format in [None, Some("percent"), Some("decimal"), Some("per_thousand")] {
        for (num, den) in [(80.0, 1000.0), (1.0, 8.0), (800.0, 60000.0), (0.0, 5.0)] {
            let doc = intent(
                "vertical",
                vec![beat("reveal", metric(num, den, format), None, None)],
            );
            compile_validate_all_languages(
                &format!("metric_{}_{num}_{den}", format.unwrap_or("default")),
                &doc,
            );
        }
    }
}

#[test]
fn zero_denominator_fails_via_cli_with_a_clear_message() {
    let s = Scratch::new("zero_den");
    let style = style_file(&s.0, "auto");
    let doc = intent(
        "vertical",
        vec![beat("reveal", metric(1.0, 0.0, None), None, None)],
    );
    let path = write_json(&s.0, "zero.intent.json", &doc);
    let out = s.0.join("zero.motion.json");
    let r = compile(&path, &style, &out);
    assert!(!r.status.success(), "zero denominator must fail");
    let msg = stderr(&r);
    assert!(
        msg.contains("denominator") && msg.contains("must not be 0"),
        "unclear message: {msg}"
    );
    assert!(!out.exists(), "no scene should be written on failure");
}

#[test]
fn v0_1_document_using_v0_2_features_fails_via_cli_mentioning_0_2() {
    let s = Scratch::new("v01_reject");
    let style = style_file(&s.0, "auto");
    let subjects = [
        collection(3),
        state_change("delivery", "slow", "fast"),
        metric(1.0, 2.0, None),
    ];
    for (i, subject) in subjects.into_iter().enumerate() {
        let mut doc = intent("vertical", vec![beat("emphasize", subject, None, None)]);
        doc["version"] = json!("0.1");
        let path = write_json(&s.0, &format!("v01_{i}.intent.json"), &doc);
        let r = compile(&path, &style, &s.0.join(format!("v01_{i}.motion.json")));
        assert!(!r.status.success(), "case {i}: must fail");
        assert!(stderr(&r).contains("0.2"), "case {i}: {}", stderr(&r));
    }
    let mut doc = intent(
        "vertical",
        vec![beat(
            "emphasize",
            json!({ "kind": "phrase", "value": "Hello" }),
            None,
            Some("accumulate"),
        )],
    );
    doc["version"] = json!("0.1");
    let path = write_json(&s.0, "v01_acc.intent.json", &doc);
    let r = compile(&path, &style, &s.0.join("v01_acc.motion.json"));
    assert!(!r.status.success());
    assert!(stderr(&r).contains("0.2"), "{}", stderr(&r));
}

#[test]
fn v0_1_files_still_compile_through_the_cli() {
    let s = Scratch::new("v01_compile");
    let style = root().join("examples/public/minimal.style.json");
    for rel in [
        "examples/public/minimal-emphasize.intent.json",
        "examples/public/minimal-contrast.intent.json",
        "examples/public/three-beat-story.intent.json",
        "examples/editorial_demo.intent.json",
    ] {
        let stem = Path::new(rel).file_stem().unwrap().to_string_lossy();
        let out = s.0.join(format!("{stem}.motion.json"));
        compile_ok(&root().join(rel), &style, &out);
        validate_ok(&out);
    }
}

#[test]
fn out_of_range_collection_sizes_fail_via_cli() {
    let s = Scratch::new("coll_range");
    let style = style_file(&s.0, "auto");
    for n in [1usize, 7] {
        let items: Vec<Value> = (0..n).map(|i| phrase(&format!("Item {i}"))).collect();
        let doc = intent(
            "vertical",
            vec![beat(
                "emphasize",
                json!({ "kind": "collection", "items": items }),
                None,
                Some("accumulate"),
            )],
        );
        let path = write_json(&s.0, &format!("n{n}.intent.json"), &doc);
        let r = compile(&path, &style, &s.0.join(format!("n{n}.motion.json")));
        assert!(!r.status.success(), "{n} items must fail");
    }
}
