//! (0.10) `qa --speech` and `plan-audio --speech` through the real binary on a
//! compiled fixture project: a hand-built SpeechMap (words evenly spaced inside
//! each beat), no network, no TTS, no ffmpeg.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
use motion_core::{CreativeIntent, MotionProject};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .canonicalize()
        .expect("workspace root")
}

struct Temp(PathBuf);
impl Temp {
    fn new(tag: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("motion-cli-speech-{}-{tag}", std::process::id()));
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

fn motion(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_motion-engine"))
        .current_dir(root())
        .args(args)
        .output()
        .expect("run motion-engine")
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

const INTENT: &str = "examples/voice_10/city_water.intent.json";
const STYLE: &str = "examples/taste/warm_editorial.style.json";

fn strip(word: &str) -> String {
    word.trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '%' | '₹' | '$' | '€' | '£')))
        .to_string()
}

/// One sentence inside each beat of `base`, words evenly spaced, ms-rounded.
fn speech_for(intent: &CreativeIntent, base: &MotionProject) -> SpeechMap {
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let beats: Vec<_> = base
        .scenes
        .iter()
        .filter(|sc| sc.lifecycle.is_some())
        .collect();
    for (bi, (scene, beat)) in beats.iter().zip(&intent.beats).enumerate() {
        let tokens: Vec<String> = beat
            .statement
            .split_whitespace()
            .map(strip)
            .filter(|w| !w.is_empty())
            .collect();
        let start = scene.start_seconds + 0.40;
        // 0.35 s per word: a natural pace (3.7-4.9 syllables per second here),
        // inside the 0.20 `speech_rate` range.
        let end = start + 0.35 * tokens.len() as f64;
        let step = (end - start) / tokens.len() as f64;
        for (i, t) in tokens.iter().enumerate() {
            let a = start + step * i as f64;
            words.push(SpeechWord {
                text: t.clone(),
                start: (a * 1000.0).round() / 1000.0,
                end: ((a + step * 0.85) * 1000.0).round() / 1000.0,
                confidence: 0.5,
            });
        }
        sentences.push(SpeechSentence {
            beat: bi,
            start,
            end,
        });
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: base.scenes.last().map(|sc| sc.end_seconds()).unwrap_or(0.0),
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

struct Fx {
    _tmp: Temp,
    dir: PathBuf,
    scene: PathBuf,
    speech: SpeechMap,
}

fn fixture(tag: &str) -> Fx {
    let tmp = Temp::new(tag);
    let dir = tmp.0.clone();
    let base_path = dir.join("base.motion.json");
    ok(&motion(&[
        "compile",
        INTENT,
        "--style",
        STYLE,
        "-o",
        s(&base_path),
    ]));
    let base = MotionProject::from_json(&std::fs::read_to_string(&base_path).expect("base"))
        .expect("base parses");
    let intent =
        CreativeIntent::from_json(&std::fs::read_to_string(root().join(INTENT)).expect("intent"))
            .expect("intent parses");
    let speech = speech_for(&intent, &base);
    let speech_path = dir.join("fx.speech.json");
    std::fs::write(
        &speech_path,
        serde_json::to_string_pretty(&speech).expect("json"),
    )
    .expect("write");
    let scene = dir.join("fx.motion.json");
    ok(&motion(&[
        "compile",
        INTENT,
        "--style",
        STYLE,
        "--speech",
        s(&speech_path),
        "-o",
        s(&scene),
    ]));
    Fx {
        _tmp: tmp,
        dir,
        scene,
        speech,
    }
}

fn qa(fx: &Fx, speech: &SpeechMap, extra: &[&str]) -> serde_json::Value {
    let path = fx.dir.join("qa.speech.json");
    std::fs::write(&path, serde_json::to_string_pretty(speech).expect("json")).expect("write");
    let mut args = vec!["qa", s(&fx.scene), "--speech", s(&path), "--json"];
    args.extend_from_slice(extra);
    // Exit 0 like the other qa modes, even on FAIL.
    let out = ok(&motion(&args));
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("json: {e}\n{out}"))
}

fn status(report: &serde_json::Value, name: &str) -> String {
    report["checks"]
        .as_array()
        .expect("checks")
        .iter()
        .find(|c| c["name"] == name)
        .unwrap_or_else(|| panic!("no check {name}"))["status"]
        .as_str()
        .expect("status")
        .to_string()
}

fn plan_json(times: &[f64]) -> String {
    let cues: Vec<serde_json::Value> = times
        .iter()
        .map(|t| {
            serde_json::json!({
                "time": t, "scene": "beat_1", "kind": "information", "family": "click",
                "sound_id": "a", "gain_db": 0.0, "priority": 1, "reason": "test"
            })
        })
        .collect();
    serde_json::json!({
        "version": "0.1", "cues": cues, "loudness_target": -16.0, "true_peak_limit": -1.0,
        "min_spacing": 0.15, "information_cap": 3
    })
    .to_string()
}

#[test]
fn speech_qa_passes_on_a_compiled_fixture() {
    let fx = fixture("pass");
    let r = qa(&fx, &fx.speech, &[]);
    for name in [
        "caption_timing",
        "caption_safe_area",
        "caption_lines",
        "layout",
    ] {
        assert_eq!(status(&r, name), "pass", "{name}: {r}");
    }
    assert_eq!(r["verdict"], "PASS", "{r}");
    assert!(r["max_caption_delta_ms"].as_f64().expect("delta") <= 1000.0 / 30.0 + 0.5);
    assert!(r["words"].as_u64().expect("words") > 0);
    // Without a plan or mixed file those checks are skipped, not passed.
    assert_eq!(status(&r, "sfx_vs_words"), "skip");
    assert_eq!(status(&r, "loudness"), "skip");

    // Text mode prints the block.
    let path = fx.dir.join("fx.speech.json");
    let text = ok(&motion(&["qa", s(&fx.scene), "--speech", s(&path)]));
    assert!(text.contains("SPEECH QA"), "{text}");
    assert!(text.contains("PASS caption_timing"), "{text}");
    assert!(text.contains("max caption delta"), "{text}");
}

#[test]
fn speech_qa_fails_when_captions_are_shifted_against_the_speech() {
    let fx = fixture("shifted");
    let mut shifted = fx.speech.clone();
    for w in &mut shifted.words {
        w.start += 0.25;
        w.end += 0.25;
    }
    let r = qa(&fx, &shifted, &[]);
    assert_eq!(status(&r, "caption_timing"), "fail", "{r}");
    assert_eq!(r["verdict"], "FAIL");
    assert!(
        r["max_caption_delta_ms"].as_f64().expect("delta") > 200.0,
        "{r}"
    );
}

#[test]
fn speech_qa_checks_planned_sfx_peaks_against_word_onsets() {
    let fx = fixture("sfx");
    let onset = fx.speech.words[3].start;
    let bad = fx.dir.join("bad.audio.json");
    std::fs::write(&bad, plan_json(&[onset + 0.05])).expect("plan");
    let r = qa(&fx, &fx.speech, &["--audio-plan", s(&bad)]);
    assert_eq!(status(&r, "sfx_vs_words"), "fail", "{r}");
    assert_eq!(r["sfx_conflicts"].as_array().expect("conflicts").len(), 1);

    let good = fx.dir.join("good.audio.json");
    // Exactly 80 ms clear of the onset and of every other word.
    let mut t = onset + 0.08;
    while fx
        .speech
        .words
        .iter()
        .any(|w| (w.start - t).abs() < 0.08 - 1e-9)
    {
        t += 0.01;
    }
    std::fs::write(&good, plan_json(&[t])).expect("plan");
    let r = qa(&fx, &fx.speech, &["--audio-plan", s(&good)]);
    assert_eq!(status(&r, "sfx_vs_words"), "pass", "{r}");
}
