//! (0.23 W5a) The audio checks never skip silently, through the real binary:
//! a plan written under a symlinked directory points at its bed (`plan-audio`
//! resolves symlinks before taking the relative path), so `qa --speech` runs
//! all four bed checks; a plan whose bed file is missing WARNs with the path;
//! `mix_intelligibility` SKIPs with its reason when there is no mix or no
//! recogniser; a passing check line never holds the upper-case word `reel`
//! echoes. No network, no TTS; ffmpeg builds the fixture voice (tests skip
//! without it).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
use motion_core::MotionProject;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

struct Temp(PathBuf);
impl Temp {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!("motion-cli-intel-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("tempdir");
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn motion_in(dir: &Path, envs: &[(&str, &Path)], args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_motion-engine"));
    cmd.current_dir(dir).args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().expect("run motion-engine")
}

fn motion(args: &[&str]) -> Output {
    motion_in(&root(), &[], args)
}

fn ok(o: &Output) -> String {
    assert!(
        o.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8")
}

fn have_ffmpeg() -> bool {
    let ok = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping: ffmpeg not available");
    }
    ok
}

const INTENT: &str = "examples/voice_10/city_water.intent.json";
const STYLE: &str = "examples/taste/warm_editorial.style.json";
/// A committed bed (and its `music-index` plan) from the repository.
const BED_PLAN: &str = "assets/music/calm_ambient.music.json";
const BED_TRACK: &str = "assets/music/calm_ambient.mp3";

/// Two sentences with a 2 s gap between them.
fn speech_map(duration: f64) -> SpeechMap {
    let spoken = [
        ("Sleep", 0.5),
        ("takes", 1.0),
        ("about", 1.5),
        ("eight", 2.0),
        ("hours", 2.5),
        ("for", 3.0),
        ("most", 5.5),
        ("adults", 6.0),
    ];
    SpeechMap {
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration,
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words: spoken
            .iter()
            .map(|&(text, start)| SpeechWord {
                text: text.to_string(),
                start,
                end: start + 0.4,
                confidence: 0.5,
            })
            .collect(),
        sentences: vec![
            SpeechSentence {
                beat: 0,
                start: 0.5,
                end: 3.4,
            },
            SpeechSentence {
                beat: 1,
                start: 5.5,
                end: 6.4,
            },
        ],
        recognised: Vec::new(),
        alignment: None,
    }
}

/// A compiled scene, a speech map with its voice wav, an empty SFX library and
/// a plan directory reached through a symlink to a directory at another depth
/// (the shape of `/tmp/x` on macOS).
struct Fx {
    _tmp: Temp,
    scene: PathBuf,
    speech: PathBuf,
    voice: PathBuf,
    library: PathBuf,
    /// The symlink, and the real directory behind it.
    link: PathBuf,
    real: PathBuf,
}

#[cfg(unix)]
fn fixture(tag: &str) -> Fx {
    let tmp = Temp::new(tag);
    let dir = tmp.0.clone();
    let scene = dir.join("fx.motion.json");
    ok(&motion(&[
        "compile",
        INTENT,
        "--style",
        STYLE,
        "-o",
        s(&scene),
    ]));
    let project =
        MotionProject::from_json(&std::fs::read_to_string(&scene).expect("scene")).expect("parses");
    let duration = project.duration_seconds();

    let speech_dir = dir.join("speech");
    std::fs::create_dir_all(&speech_dir).expect("speech dir");
    let speech = speech_dir.join("fx.speech.json");
    std::fs::write(
        &speech,
        serde_json::to_string_pretty(&speech_map(duration)).expect("json"),
    )
    .expect("write speech");
    let voice = speech_dir.join("voice.wav");
    let status = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-f", "lavfi", "-i"])
        .arg(format!(
            "sine=frequency=600:duration={}:sample_rate=48000",
            duration + 1.0
        ))
        .args(["-ac", "1"])
        .arg(&voice)
        .status()
        .expect("ffmpeg");
    assert!(status.success());

    let library = dir.join("sfx");
    std::fs::create_dir_all(&library).expect("sfx dir");
    std::fs::write(
        library.join("sfx-library.json"),
        r#"{"version":"0.1","sounds":[]}"#,
    )
    .expect("library");

    // The plan directory is a symlink to a directory three levels deeper, so a
    // lexical `..` from the link lands somewhere else than from its target.
    let real = dir.join("real").join("a").join("b");
    std::fs::create_dir_all(&real).expect("real dir");
    let link = dir.join("link");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    Fx {
        _tmp: tmp,
        scene,
        speech,
        voice,
        library,
        link,
        real,
    }
}

/// `plan-audio` of the fixture, written through the symlinked directory.
#[cfg(unix)]
fn plan_through_symlink(fx: &Fx) -> PathBuf {
    let plan = fx.link.join("fx.audio.json");
    ok(&motion(&[
        "plan-audio",
        s(&fx.scene),
        "--intent",
        INTENT,
        "--style",
        STYLE,
        "--sfx-library",
        s(&fx.library),
        "--speech",
        s(&fx.speech),
        "--music",
        BED_PLAN,
        "-o",
        s(&plan),
    ]));
    plan
}

fn qa_speech(fx: &Fx, plan: &Path, extra: &[&str], envs: &[(&str, &Path)]) -> Output {
    let mut args = vec![
        "qa",
        s(&fx.scene),
        "--speech",
        s(&fx.speech),
        "--audio-plan",
        s(plan),
    ];
    args.extend_from_slice(extra);
    motion_in(&root(), envs, &args)
}

fn qa_json(fx: &Fx, plan: &Path, extra: &[&str], envs: &[(&str, &Path)]) -> serde_json::Value {
    let mut more = vec!["--json"];
    more.extend_from_slice(extra);
    let out = ok(&qa_speech(fx, plan, &more, envs));
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json: {e}\n{out}"))
}

fn check<'a>(report: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no check {name}: {report}"))
}

fn status<'a>(report: &'a serde_json::Value, name: &str) -> &'a str {
    check(report, name)["status"].as_str().expect("status")
}

const BED_CHECKS: [&str; 4] = [
    "voice_over_music",
    "bed_over_voice",
    "speech_band_masking",
    "bed_duck",
];

#[cfg(unix)]
#[test]
fn a_plan_under_a_symlinked_directory_points_at_its_bed() {
    if !have_ffmpeg() {
        return;
    }
    let fx = fixture("symlink");
    let plan = plan_through_symlink(&fx);
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).expect("plan")).expect("plan json");
    let track = json["music"]["track"].as_str().expect("track");
    // Read the way every consumer does: the plan's directory joined with the
    // written track, followed physically through the symlink.
    let resolved = fx.link.join(track).canonicalize().unwrap_or_else(|e| {
        panic!("the plan's track '{track}' does not resolve from the symlinked directory: {e}")
    });
    assert_eq!(
        resolved,
        root().join(BED_TRACK).canonicalize().expect("bed exists")
    );
    // The same written path through the real directory is the same file.
    assert_eq!(
        fx.real
            .join(track)
            .canonicalize()
            .expect("via the real dir"),
        resolved
    );
}

#[cfg(unix)]
#[test]
fn qa_speech_runs_every_bed_check_for_a_plan_under_a_symlinked_directory() {
    if !have_ffmpeg() {
        return;
    }
    let fx = fixture("bedchecks");
    let plan = plan_through_symlink(&fx);
    // The voice file doubles as the mixed file: bed_duck needs one.
    let report = qa_json(&fx, &plan, &["--mixed", s(&fx.voice)], &[]);
    for name in BED_CHECKS {
        let st = status(&report, name);
        assert_ne!(st, "skip", "{name}: {}", check(&report, name));
        assert!(
            !check(&report, name)["detail"]
                .as_str()
                .expect("detail")
                .contains("not found"),
            "{name}: {}",
            check(&report, name)
        );
    }
    assert!(report["voice_over_music_db"].is_number(), "{report}");

    // `reel` echoes every QA line holding the upper-case word FAIL: a passing
    // check states its thresholds without it.
    let text = String::from_utf8_lossy(&qa_speech(&fx, &plan, &[], &[]).stdout).into_owned();
    for name in ["voice_over_music", "bed_over_voice", "speech_band_masking"] {
        let line = text
            .lines()
            .find(|l| l.contains(&format!(" {name}:")))
            .unwrap_or_else(|| panic!("{name}: {text}"));
        if name == "voice_over_music" {
            // The voice-relative bed sits 18 dB under the voice: a clear pass.
            assert!(line.trim_start().starts_with("PASS"), "{line}");
        }
        if !line.trim_start().starts_with("FAIL") {
            assert!(!line.contains("FAIL"), "{line}");
        }
    }
}

#[cfg(unix)]
#[test]
fn a_plan_naming_a_missing_bed_warns_with_the_path() {
    if !have_ffmpeg() {
        return;
    }
    let fx = fixture("missing");
    let plan = plan_through_symlink(&fx);
    let mut json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&plan).expect("plan")).expect("plan json");
    json["music"]["track"] = serde_json::json!("nowhere/gone.mp3");
    std::fs::write(&plan, json.to_string()).expect("rewrite plan");
    let report = qa_json(&fx, &plan, &["--mixed", s(&fx.voice)], &[]);
    for name in BED_CHECKS {
        let c = check(&report, name);
        assert_eq!(c["status"], "warn", "{name}: {c}");
        let detail = c["detail"].as_str().expect("detail");
        assert!(detail.contains("nowhere/gone.mp3"), "{name}: {detail}");
        assert!(
            detail.contains("fx.audio.json") || detail.contains("link"),
            "{detail}"
        );
    }
    // A WARN does not fail the verdict by itself.
    assert!(
        report["checks"]
            .as_array()
            .expect("checks")
            .iter()
            .filter(|c| BED_CHECKS.contains(&c["name"].as_str().unwrap_or("")))
            .all(|c| c["status"] != "fail"),
        "{report}"
    );
}

#[cfg(unix)]
#[test]
fn mix_intelligibility_skips_with_its_reason() {
    if !have_ffmpeg() {
        return;
    }
    let fx = fixture("skips");
    let plan = plan_through_symlink(&fx);
    // No mix to listen to.
    let report = qa_json(&fx, &plan, &[], &[]);
    let c = check(&report, "mix_intelligibility");
    assert_eq!(c["status"], "skip", "{c}");
    assert!(c["detail"].as_str().expect("detail").contains("--mixed"));

    // A mix, but no recogniser model (an empty models directory; a build
    // without `asr-local` skips for that reason first).
    let empty = fx._tmp.0.join("no_models");
    std::fs::create_dir_all(&empty).expect("models dir");
    let report = qa_json(
        &fx,
        &plan,
        &["--mixed", s(&fx.voice)],
        &[("MOTION_MODELS_DIR", &empty)],
    );
    let c = check(&report, "mix_intelligibility");
    assert_eq!(c["status"], "skip", "{c}");
    let detail = c["detail"].as_str().expect("detail");
    assert!(
        detail.contains("asr-local") || detail.contains("models fetch"),
        "{detail}"
    );
}
