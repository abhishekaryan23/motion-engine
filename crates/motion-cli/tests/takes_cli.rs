//! (0.23 B1) `compile --take N` and `reel --take N` through the real binary.
//!
//! The take is meaningful only with `--variety`: without it the flag is
//! accepted and the scene is byte-identical. With it the take reaches
//! `CompileOptions.take`; `compile` confirms that in one stdout line
//! (`take N`, or `take N ignored (no --variety)`), and once the direction
//! layer writes `project.direction` (B2a) the record must carry the take.

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

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Scratch directory unique to one test, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir()
            .join(format!("motion_takes_cli_{}", std::process::id()))
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
const STYLE: &str = "examples/editorial_demo.style.json";

/// Whether compile printed its take line (`take N ...`); a default compile
/// prints none.
fn has_take_line(out: &str) -> bool {
    out.lines().any(|l| l.starts_with("take "))
}

/// Compile the demo into `<scratch>/<name>.motion.json` with `extra` flags;
/// returns the compile's stdout and the scene bytes.
fn compile(s: &Scratch, name: &str, extra: &[&str]) -> (String, Vec<u8>) {
    let out = s.path(&format!("{name}.motion.json"));
    let mut args = vec![
        "compile",
        INTENT,
        "--style",
        STYLE,
        "--output",
        out.as_str(),
    ];
    args.extend_from_slice(extra);
    let o = motion(&args);
    assert!(
        o.status.success(),
        "compile {extra:?} failed:\n{}{}",
        stdout(&o),
        stderr(&o)
    );
    (stdout(&o), std::fs::read(&out).expect("scene written"))
}

#[test]
fn take_without_variety_is_accepted_and_byte_identical() {
    let s = Scratch::new("no_variety");
    let (plain_out, plain) = compile(&s, "plain", &[]);
    let (zero_out, zero) = compile(&s, "zero", &["--take", "0"]);
    let (one_out, one) = compile(&s, "one", &["--take", "1"]);
    let (_, seven) = compile(&s, "seven", &["--take", "7"]);
    assert_eq!(plain, zero, "--take 0 changes nothing");
    assert_eq!(plain, one, "--take 1 without --variety changes nothing");
    assert_eq!(plain, seven, "--take 7 without --variety changes nothing");
    // The default compile prints no take line; a take says it was ignored.
    assert!(!has_take_line(&plain_out), "{plain_out}");
    assert!(!has_take_line(&zero_out), "{zero_out}");
    assert!(
        one_out
            .lines()
            .any(|l| l == "take 1 ignored (no --variety)"),
        "{one_out}"
    );
}

#[test]
fn take_zero_with_variety_is_todays_output() {
    let s = Scratch::new("variety_zero");
    let (auto_out, auto) = compile(&s, "auto", &["--variety", "auto"]);
    let (_, zero) = compile(&s, "zero", &["--variety", "auto", "--take", "0"]);
    assert_eq!(auto, zero, "take 0 is the story-keyed variety seed");
    assert!(!has_take_line(&auto_out), "{auto_out}");
}

#[test]
fn take_reaches_the_compile_under_variety() {
    let s = Scratch::new("variety_take");
    for take in ["1", "2", "99"] {
        let (out, scene) = compile(&s, take, &["--variety", "auto", "--take", take]);
        // `CompileOptions.take` was set: the compile says so.
        let line = format!("take {take}");
        assert!(out.lines().any(|l| l == line), "{out}");
        assert!(
            !out.lines().any(|l| l.ends_with("ignored (no --variety)")),
            "{out}"
        );
        // Once B2a records the direction, it carries the take.
        let scene: Value = serde_json::from_slice(&scene).expect("scene json");
        if let Some(direction) = scene["project"]["direction"].as_object() {
            assert_eq!(direction["take"], take.parse::<u64>().unwrap());
        }
    }
    // A numeric variety seed takes a take too.
    let (out, _) = compile(&s, "seeded", &["--variety", "42", "--take", "3"]);
    assert!(out.lines().any(|l| l == "take 3"), "{out}");
}

#[test]
fn take_must_be_a_whole_number() {
    let s = Scratch::new("bad");
    let out = s.path("bad.motion.json");
    for bad in ["x", "-1", "1.5", ""] {
        let o = motion(&[
            "compile", INTENT, "--style", STYLE, "--output", &out, "--take", bad,
        ]);
        assert!(!o.status.success(), "--take '{bad}' must fail");
        assert!(!stderr(&o).is_empty());
    }
    assert!(!std::path::Path::new(&out).exists());
}

#[test]
fn compile_and_reel_list_the_take_flag() {
    for cmd in ["compile", "reel"] {
        let o = motion(&[cmd, "--help"]);
        assert!(o.status.success());
        let help = stdout(&o);
        assert!(help.contains("--take <TAKE>"), "{cmd}: {help}");
        assert!(help.contains("another version"), "{cmd}: {help}");
    }
    // `reel` forwards it to compile; a bad value is refused up front.
    let o = motion(&["reel", INTENT, "--take", "x"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("--take"), "{}", stderr(&o));
}
