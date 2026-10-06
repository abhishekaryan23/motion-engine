//! `explore`, `plan-assets --explore` and `plan-audio --explore` through the
//! real binary.

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

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Scratch directory unique to one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_explore_cli_{}", std::process::id()))
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

const INTENT: &str = "examples/editorial_demo.intent.json";
const STYLE: &str = "examples/taste/dark_technical.style.json";

fn json_file(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read")).expect("json")
}

#[test]
fn explore_writes_variants_sheet_and_summary() {
    let s = Scratch::new("explore");
    let out = s.path("out");
    let o = motion(&[
        "explore",
        INTENT,
        "--style",
        STYLE,
        "--explore",
        "2",
        "--variants",
        "3",
        "--seed",
        "10",
        "-o",
        &out,
    ]);
    assert!(o.status.success(), "explore failed: {}", stderr(&o));
    for i in 0..3 {
        let scene = format!("{out}/variant_{i}.motion.json");
        let project = json_file(&scene);
        assert_eq!(project["version"], "0.2");
        assert!(project["scenes"].as_array().is_some_and(|s| s.len() > 1));
        assert!(
            project["project"]["exploration"]["level"] == 2,
            "variant {i} has no exploration record"
        );
        // The scene file is a valid, renderable project.
        let v = motion(&["validate", &scene]);
        assert!(v.status.success(), "validate {i}: {}", stderr(&v));
    }
    // Contact sheet: a valid PNG with the expected signature and size > 0.
    let sheet = std::fs::read(format!("{out}/sheet.png")).expect("sheet.png");
    assert_eq!(
        &sheet[..8],
        &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
    );
    let width = u32::from_be_bytes([sheet[16], sheet[17], sheet[18], sheet[19]]);
    let height = u32::from_be_bytes([sheet[20], sheet[21], sheet[22], sheet[23]]);
    assert!(
        width >= 270 && height >= 3 * 100,
        "sheet is {width}x{height}"
    );

    let summary = json_file(&format!("{out}/explore.json"));
    let rows = summary.as_array().expect("array");
    assert_eq!(rows.len(), 3);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row["variant"], i as u64);
        assert_eq!(row["seed"], 10 + i as u64);
        assert_eq!(row["level"], 2);
        assert!(row["record"]["choices"].is_array());
        assert!(row["typography"].is_string());
    }
}

#[test]
fn explore_is_deterministic_and_rejects_bad_input() {
    let s = Scratch::new("determinism");
    let (a, b) = (s.path("a"), s.path("b"));
    for out in [&a, &b] {
        let o = motion(&[
            "explore",
            INTENT,
            "--style",
            STYLE,
            "--explore",
            "3",
            "--variants",
            "2",
            "--seed",
            "4",
            "-o",
            out,
        ]);
        assert!(o.status.success(), "{}", stderr(&o));
    }
    for name in [
        "variant_0.motion.json",
        "variant_1.motion.json",
        "sheet.png",
        "explore.json",
    ] {
        let x = std::fs::read(format!("{a}/{name}")).expect("a");
        let y = std::fs::read(format!("{b}/{name}")).expect("b");
        assert!(x == y, "{name} differs between identical runs");
    }
    let o = motion(&["explore", INTENT, "--explore", "9", "-o", &s.path("c")]);
    assert!(!o.status.success(), "explore 9 must be rejected");
    let o = motion(&[
        "explore",
        INTENT,
        "--emotion",
        "nonsense",
        "-o",
        &s.path("d"),
    ]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("unknown emotion"), "{}", stderr(&o));
}

#[test]
fn plan_assets_accepts_exploration_flags() {
    let s = Scratch::new("plan_assets");
    let canonical = motion(&["plan-assets", INTENT, "--style", STYLE]);
    assert!(canonical.status.success(), "{}", stderr(&canonical));
    let explored = motion(&[
        "plan-assets",
        INTENT,
        "--style",
        STYLE,
        "--explore",
        "2",
        "--seed",
        "5",
        "-o",
        &s.path("plan.json"),
    ]);
    assert!(explored.status.success(), "{}", stderr(&explored));
    // Without catalog families there are no ties: the plan is unchanged.
    let plan = std::fs::read_to_string(s.path("plan.json")).expect("plan");
    assert_eq!(
        serde_json::from_str::<Value>(&plan).expect("json"),
        serde_json::from_slice::<Value>(&canonical.stdout).expect("json")
    );
}
