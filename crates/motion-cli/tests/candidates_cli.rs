//! (0.23 B3) `compile --candidates N` and `reel --candidates N` through the
//! real binary.
//!
//! Best-of-N compiles N seeded directions of the take in parallel, drops the
//! ones with a hard QA failure and ships the best by soft score. It needs the
//! variety seed (the candidates are directions of it); the default, 1, is a
//! plain compile and must stay byte-identical.

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
            .join(format!("motion_candidates_cli_{}", std::process::id()))
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

const INTENT: &str = "docs/plans/sprint_0_23/stories/sleep_review.intent.json";
const STYLE: &str = "docs/plans/sprint_0_23/styles/playful.style.json";
const SPEECH: &str = "golden/fixtures/variety/sleep_review.speech.json";

/// Compile the review story into `<scratch>/<name>.motion.json` with `extra`
/// flags; returns the output and the scene bytes (when it compiled).
fn compile(s: &Scratch, name: &str, extra: &[&str]) -> (Output, Vec<u8>) {
    let out = s.path(&format!("{name}.motion.json"));
    let mut args = vec![
        "compile",
        INTENT,
        "--style",
        STYLE,
        "--art",
        "auto",
        "--output",
        out.as_str(),
    ];
    args.extend_from_slice(extra);
    let o = motion(&args);
    let bytes = std::fs::read(&out).unwrap_or_default();
    (o, bytes)
}

fn compiled(s: &Scratch, name: &str, extra: &[&str]) -> (String, Vec<u8>) {
    let (o, bytes) = compile(s, name, extra);
    assert!(
        o.status.success(),
        "compile {extra:?} failed:\n{}{}",
        stdout(&o),
        stderr(&o)
    );
    (stdout(&o), bytes)
}

/// The `candidates: N, chose K (soft S; M hard failure(s) dropped)` line.
fn candidates_line(out: &str) -> Option<&str> {
    out.lines().find(|l| l.starts_with("candidates: "))
}

fn direction(bytes: &[u8]) -> Value {
    let v: Value = serde_json::from_slice(bytes).expect("scene json");
    v["project"]["direction"].clone()
}

#[test]
fn one_candidate_is_the_plain_compile_byte_for_byte() {
    let s = Scratch::new("one");
    for extra in [vec!["--variety", "auto"], vec![]] {
        let mut with_flag = extra.clone();
        with_flag.extend(["--candidates", "1"]);
        let (plain_out, plain) = compiled(&s, "plain", &extra);
        let (one_out, one) = compiled(&s, "one", &with_flag);
        assert_eq!(plain, one, "--candidates 1 changes nothing ({extra:?})");
        assert!(candidates_line(&plain_out).is_none(), "{plain_out}");
        assert!(candidates_line(&one_out).is_none(), "{one_out}");
    }
}

#[test]
fn more_than_one_candidate_needs_the_variety_seed() {
    let s = Scratch::new("needs_variety");
    let (o, bytes) = compile(&s, "x", &["--candidates", "4"]);
    assert!(!o.status.success(), "{}", stdout(&o));
    assert!(bytes.is_empty(), "nothing is written on a refused request");
    let err = stderr(&o);
    assert!(
        err.contains("--candidates 4") && err.contains("--variety"),
        "{err}"
    );
}

#[test]
fn candidates_outside_one_to_eight_are_refused() {
    let s = Scratch::new("range");
    for n in ["0", "9", "200"] {
        let (o, bytes) = compile(&s, "x", &["--variety", "auto", "--candidates", n]);
        assert!(!o.status.success(), "--candidates {n} was accepted");
        assert!(bytes.is_empty());
        assert!(stderr(&o).contains("expected 1 to 8"), "{}", stderr(&o));
    }
    let (o, _) = compile(&s, "x", &["--variety", "auto", "--candidates", "8"]);
    assert!(o.status.success(), "{}{}", stdout(&o), stderr(&o));
}

#[test]
fn four_candidates_print_one_line_and_record_every_score() {
    let s = Scratch::new("four");
    let (out, bytes) = compiled(
        &s,
        "four",
        &["--variety", "auto", "--speech", SPEECH, "--candidates", "4"],
    );
    let line = candidates_line(&out).unwrap_or_else(|| panic!("no candidates line:\n{out}"));
    // candidates: 4, chose 2 (soft 3.1; 1 hard failure dropped)
    let rest = line
        .strip_prefix("candidates: 4, chose ")
        .unwrap_or_else(|| panic!("{line}"));
    let (k, rest) = rest.split_once(" (soft ").expect("soft");
    let k: u64 = k.parse().expect("chosen k");
    assert!(k < 4, "{line}");
    let (soft, rest) = rest.split_once("; ").expect("soft value");
    soft.parse::<f64>().expect("soft is a number");
    assert!(
        rest.ends_with("hard failures dropped)") || rest.ends_with("hard failure dropped)"),
        "{line}"
    );
    assert_eq!(
        out.lines()
            .filter(|l| l.starts_with("candidates: "))
            .count(),
        1
    );
    assert!(
        !out.contains("shipped_with_failure"),
        "a clean take must not warn:\n{out}"
    );

    let d = direction(&bytes);
    assert_eq!(d["chosen"].as_u64(), Some(k), "{d}");
    let list = d["candidates"].as_array().expect("candidates");
    assert_eq!(list.len(), 4);
    for (i, c) in list.iter().enumerate() {
        assert_eq!(c["k"].as_u64(), Some(i as u64));
        assert!(c["seed"].is_u64() && c["soft"].is_f64(), "{c}");
        let parts = c["parts"].as_object().expect("parts");
        for name in ["variety", "focal", "density", "balance", "dead_air"] {
            assert!(parts.contains_key(name), "{name} missing in {c}");
        }
    }
    // The chosen candidate is clean and has the best soft score of the clean.
    let chosen = &list[k as usize];
    assert!(chosen.get("hard").is_none(), "{chosen}");
    let soft = |c: &Value| c["soft"].as_f64().expect("soft");
    for c in list.iter().filter(|c| c.get("hard").is_none()) {
        assert!(soft(c) <= soft(chosen) + 1e-9, "{c} beats {chosen}");
    }
    assert!(d.get("shipped_with_failure").is_none(), "{d}");
}

#[test]
fn the_choice_is_deterministic_across_runs() {
    let s = Scratch::new("determinism");
    let args = ["--variety", "auto", "--speech", SPEECH, "--candidates", "4"];
    let (out0, first) = compiled(&s, "run0", &args);
    for i in 1..3 {
        let (out, again) = compiled(&s, &format!("run{i}"), &args);
        assert_eq!(first, again, "run {i} wrote another scene");
        assert_eq!(
            candidates_line(&out0),
            candidates_line(&out),
            "run {i} chose differently"
        );
    }
}

/// The review story's speech map with `lead` seconds of silence in front of
/// the narration: every beat then waits for its words and the video opens on
/// nothing, whatever direction the take has (a real defect of a voice take
/// with a long lead-in, which no candidate can repair).
fn late_speech(s: &Scratch, lead: f64) -> String {
    let text = std::fs::read_to_string(root().join(SPEECH)).expect("speech fixture");
    let mut v: Value = serde_json::from_str(&text).expect("speech json");
    fn bump(v: &mut Value, lead: f64) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    if (k == "start" || k == "end") && x.is_number() {
                        *x = Value::from(x.as_f64().unwrap_or(0.0) + lead);
                    } else {
                        bump(x, lead);
                    }
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| bump(x, lead)),
            _ => {}
        }
    }
    for key in ["words", "sentences"] {
        if let Some(x) = v.get_mut(key) {
            bump(x, lead);
        }
    }
    let total = v["duration"].as_f64().unwrap_or(0.0) + lead;
    v["duration"] = Value::from(total);
    let path = s.path("late.speech.json");
    std::fs::write(&path, serde_json::to_string(&v).expect("speech out")).expect("write speech");
    path
}

#[test]
fn when_every_candidate_fails_the_best_ships_and_compile_says_so() {
    let s = Scratch::new("all_fail");
    let speech = late_speech(&s, 4.0);
    let (out, bytes) = compiled(
        &s,
        "late",
        &[
            "--variety",
            "auto",
            "--speech",
            &speech,
            "--candidates",
            "4",
        ],
    );
    let notice = out
        .lines()
        .find(|l| l.starts_with("warning[shipped_with_failure]"))
        .unwrap_or_else(|| panic!("never silent, but nothing was printed:\n{out}"));
    assert!(
        notice.contains("every candidate failed: dead_air"),
        "{notice}"
    );
    let line = candidates_line(&out).expect("candidates line");
    assert!(line.starts_with("candidates: 4, chose "), "{line}");
    let d = direction(&bytes);
    assert_eq!(
        d["shipped_with_failure"].as_str(),
        Some("every candidate failed: dead_air"),
        "{d}"
    );
    let list = d["candidates"].as_array().expect("candidates");
    assert_eq!(list.len(), 4);
    assert!(
        list.iter().all(|c| c["hard"]
            .as_array()
            .is_some_and(|h| h.iter().any(|n| n == "dead_air"))),
        "{list:?}"
    );
    // The scene is still written, and the chosen one is the best of the lot:
    // the highest soft score among those with the fewest failures.
    let chosen = d["chosen"].as_u64().expect("chosen") as usize;
    let fewest = list
        .iter()
        .map(|c| c["hard"].as_array().map_or(0, Vec::len))
        .min()
        .expect("some candidate");
    assert_eq!(
        list[chosen]["hard"].as_array().map_or(0, Vec::len),
        fewest,
        "{list:?}"
    );
}

#[test]
fn reel_takes_candidates_with_a_default_of_four() {
    let o = motion(&["reel", "--help"]);
    assert!(o.status.success());
    let help = stdout(&o);
    let at = help.find("--candidates").expect("reel lists --candidates");
    let entry = &help[at..];
    let entry = &entry[..entry.find("\n  --").unwrap_or(entry.len())];
    assert!(entry.contains("[default: 4]"), "{entry}");

    // A bad value fails before any voice is made (nothing is written).
    let s = Scratch::new("reel");
    for n in ["0", "9"] {
        let out = s.path("reel_out");
        let o = motion(&[
            "reel",
            INTENT,
            "--tts-model",
            "say",
            "--candidates",
            n,
            "--output",
            out.as_str(),
        ]);
        assert!(!o.status.success(), "--candidates {n} was accepted");
        assert!(stderr(&o).contains("expected 1 to 8"), "{}", stderr(&o));
        assert!(
            !PathBuf::from(&out).exists(),
            "reel made {out} before refusing --candidates {n}"
        );
    }
}
