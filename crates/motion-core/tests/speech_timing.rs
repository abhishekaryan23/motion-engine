//! Speech-led compile timing (0.10): beats follow the spoken sentences, EVOLVE
//! snaps to word starts, wrong sentence counts error, `speech: None` is the
//! old output. See docs/VOICE.md.

use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::audio::{MusicPlan, MUSIC_PLAN_VERSION};
use motion_core::compiler::{
    compile_full, compile_with_music, compile_with_options, ApproxMeasure, AssetLibrary,
    CompileError, CompileOptions,
};
use motion_core::scene::MotionProject;
use motion_core::speech::{
    repair, SpeechMap, SpeechSentence, SpeechWord, EVOLVE_SNAP_WINDOW, SPEECH_VERSION,
};
use motion_core::validate::validate;
use motion_core::{CreativeIntent, StyleProfile};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const INTENT: &str = include_str!("../../../examples/editorial_demo.intent.json");
const STYLE: &str = include_str!("../../../examples/editorial_demo.style.json");

fn load() -> (CreativeIntent, StyleProfile) {
    (
        serde_json::from_str(INTENT).expect("intent"),
        serde_json::from_str(STYLE).expect("style"),
    )
}

/// Sentence starts with irregular spacing; every sentence is voiced for 2 s
/// and its statement words spread evenly across it.
fn synthetic_speech(intent: &CreativeIntent, starts: &[f64]) -> SpeechMap {
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    for (beat, (b, &start)) in intent.beats.iter().zip(starts).enumerate() {
        let end = start + 2.0;
        sentences.push(SpeechSentence { beat, start, end });
        let toks = motion_core::speech::statement_tokens(&b.statement);
        let each = 2.0 / toks.len().max(1) as f64;
        for (i, t) in toks.iter().enumerate() {
            words.push(SpeechWord {
                text: t.clone(),
                start: start + i as f64 * each,
                end: start + (i as f64 + 0.8) * each,
                confidence: 0.9,
            });
        }
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: starts[starts.len() - 1] + 2.0 + 0.9,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
    }
}

fn compile(
    intent: &CreativeIntent,
    style: &StyleProfile,
    opts: &CompileOptions,
    music: Option<&MusicPlan>,
) -> Result<MotionProject, CompileError> {
    compile_with_options(
        intent,
        style,
        None,
        &AssetLibrary::new(Path::new(ASSETS)),
        &ApproxMeasure,
        &AssetManifest::empty(),
        music,
        opts,
    )
}

fn statements(intent: &CreativeIntent) -> Vec<String> {
    intent.beats.iter().map(|b| b.statement.clone()).collect()
}

fn beat_scenes(p: &MotionProject) -> Vec<&motion_core::scene::Scene> {
    p.scenes
        .iter()
        .filter(|s| s.id.starts_with("beat_"))
        .collect()
}

#[test]
fn beat_starts_follow_sentences_minus_lead() {
    let (intent, style) = load();
    assert_eq!(intent.beats.len(), 3);
    let speech = synthetic_speech(&intent, &[0.35, 4.1, 9.35]);
    let (speech, _) = repair(&speech, &statements(&intent));
    let opts = CompileOptions {
        speech: Some(speech.clone()),
        ..CompileOptions::default()
    };
    let project = compile(&intent, &style, &opts, None).expect("compile");
    let lead = speech.sentences[0].start;
    let scenes = beat_scenes(&project);
    assert_eq!(scenes.len(), 3);
    for (scene, sentence) in scenes.iter().zip(&speech.sentences) {
        assert!(
            (scene.start_seconds - (sentence.start - lead)).abs() <= 0.001,
            "{}: starts {} expected {}",
            scene.id,
            scene.start_seconds,
            sentence.start - lead
        );
    }
    // The last beat holds past its sentence's end by at least the tail.
    let last = scenes[2];
    assert!(last.start_seconds + last.duration_seconds >= speech.sentences[2].end + 0.9 - 1e-6);
    // The compiled project is valid.
    validate(&project, Some(Path::new(ASSETS))).expect("valid");
    assert!(project.project.speech.is_some());
}

#[test]
fn speech_compile_is_deterministic() {
    let (intent, style) = load();
    let speech = synthetic_speech(&intent, &[0.35, 4.1, 9.35]);
    let opts = CompileOptions {
        speech: Some(speech),
        ..CompileOptions::default()
    };
    let a = serde_json::to_string(&compile(&intent, &style, &opts, None).expect("a")).expect("j");
    let b = serde_json::to_string(&compile(&intent, &style, &opts, None).expect("b")).expect("j");
    assert_eq!(a, b);
}

#[test]
fn wrong_sentence_count_is_a_clear_error() {
    let (intent, style) = load();
    let mut speech = synthetic_speech(&intent, &[0.35, 4.1, 9.35]);
    speech.sentences.pop();
    let opts = CompileOptions {
        speech: Some(speech),
        ..CompileOptions::default()
    };
    let err = compile(&intent, &style, &opts, None).expect_err("must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("2 sentences") && msg.contains("3 beats"),
        "{msg}"
    );
}

#[test]
fn evolve_snaps_to_a_word_start_and_lifecycle_stays_ordered() {
    let (intent, style) = load();
    let speech = synthetic_speech(&intent, &[0.35, 4.1, 9.35]);
    let (speech, _) = repair(&speech, &statements(&intent));
    let opts = CompileOptions {
        speech: Some(speech.clone()),
        ..CompileOptions::default()
    };
    let project = compile(&intent, &style, &opts, None).expect("compile");
    let record = project.project.speech.as_ref().expect("record");
    assert_eq!(record.words, speech.words.len());
    assert_eq!(record.model, "fixture/model");
    for snap in &record.evolve_snaps {
        assert!(
            (snap.to - snap.from).abs() <= EVOLVE_SNAP_WINDOW + 1e-6,
            "{snap:?}"
        );
        assert!(
            speech
                .words
                .iter()
                .any(|w| (w.start - snap.to).abs() < 1e-3),
            "{snap:?} is not a word start"
        );
        let scene = beat_scenes(&project)[snap.beat];
        let life = scene.lifecycle.expect("lifecycle");
        assert!((scene.start_seconds + life.evolve - snap.to).abs() < 1e-3);
    }
    for scene in beat_scenes(&project) {
        let l = scene.lifecycle.expect("lifecycle");
        assert!(l.read < l.evolve && l.evolve < l.anticipate, "{l:?}");
        assert!(l.anticipate <= l.bridge && l.bridge <= scene.duration_seconds);
    }
}

#[test]
fn speech_overrides_music_snapping() {
    let (intent, style) = load();
    let speech = synthetic_speech(&intent, &[0.35, 4.1, 9.35]);
    let music = MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: "bed.wav".into(),
        duration: 30.0,
        bpm: 120.0,
        beat_times: Vec::new(),
        downbeat_times: (0..16).map(|i| i as f64 * 2.0 + 0.123).collect(),
        sections: Vec::new(),
        gain_db: -18.0,
        sha256: String::new(),
        lufs: None,
        lra: None,
    };
    let with_music = |m: Option<&MusicPlan>| {
        let opts = CompileOptions {
            speech: Some(speech.clone()),
            ..CompileOptions::default()
        };
        compile(&intent, &style, &opts, m).expect("compile")
    };
    let a = with_music(None);
    let b = with_music(Some(&music));
    let starts = |p: &MotionProject| -> Vec<f64> {
        beat_scenes(p).iter().map(|s| s.start_seconds).collect()
    };
    assert_eq!(starts(&a), starts(&b));
    assert!(!a.project.speech.as_ref().expect("a").music_ignored);
    assert!(b.project.speech.as_ref().expect("b").music_ignored);
}

#[test]
fn without_speech_the_output_is_unchanged() {
    let (intent, style) = load();
    let lib = AssetLibrary::new(Path::new(ASSETS));
    let full = compile_full(
        &intent,
        &style,
        None,
        &lib,
        &ApproxMeasure,
        &AssetManifest::empty(),
    )
    .expect("full");
    let opts = compile(&intent, &style, &CompileOptions::default(), None).expect("opts");
    let music_none = compile_with_music(
        &intent,
        &style,
        None,
        &lib,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
    )
    .expect("music");
    let a = serde_json::to_string(&full).expect("a");
    assert_eq!(a, serde_json::to_string(&opts).expect("b"));
    assert_eq!(a, serde_json::to_string(&music_none).expect("c"));
    assert!(!a.contains("\"speech\""));
}

#[test]
fn evolve_moves_onto_a_nearby_word_and_ignores_far_ones() {
    let (intent, style) = load();
    let mut speech = synthetic_speech(&intent, &[0.35, 4.1, 9.35]);
    speech.words.clear();
    let opts = |s: &SpeechMap| CompileOptions {
        speech: Some(s.clone()),
        ..CompileOptions::default()
    };
    let base = compile(&intent, &style, &opts(&speech), None).expect("base");
    assert!(base
        .project
        .speech
        .as_ref()
        .expect("r")
        .evolve_snaps
        .is_empty());
    let scene = beat_scenes(&base)[0];
    let evolve = scene.start_seconds + scene.lifecycle.expect("l").evolve;

    let word = |start: f64| SpeechWord {
        text: "x".into(),
        start,
        end: start + 0.2,
        confidence: 0.9,
    };
    // 60 ms away: snaps. 300 ms away: does not.
    speech.words = vec![word(evolve + 0.06)];
    let near = compile(&intent, &style, &opts(&speech), None).expect("near");
    let snaps = &near.project.speech.as_ref().expect("r").evolve_snaps;
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].beat, 0);
    assert!((snaps[0].to - (evolve + 0.06)).abs() < 2e-3, "{snaps:?}");
    let s0 = beat_scenes(&near)[0];
    assert!((s0.start_seconds + s0.lifecycle.expect("l").evolve - snaps[0].to).abs() < 1e-3);

    speech.words = vec![word(evolve + 0.3)];
    let far = compile(&intent, &style, &opts(&speech), None).expect("far");
    assert!(far
        .project
        .speech
        .as_ref()
        .expect("r")
        .evolve_snaps
        .is_empty());
}

#[test]
fn content_enters_when_its_word_is_spoken() {
    // (0.10 Q) city_water beat 4: "Sensors find a leak within hours." The
    // LEAK FOUND support card must enter on "leak", not late in EVOLVE.
    let intent: CreativeIntent = serde_json::from_str(
        &std::fs::read_to_string(
            Path::new(ASSETS).join("../examples/voice_10/city_water.intent.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let style: StyleProfile = serde_json::from_str(r#"{"tone":"technical"}"#).unwrap();
    let starts: Vec<f64> = (0..intent.beats.len())
        .map(|i| 0.35 + 4.0 * i as f64)
        .collect();
    let speech = synthetic_speech(&intent, &starts);
    let opts = CompileOptions {
        speech: Some(speech.clone()),
        ..CompileOptions::default()
    };
    let p = compile(&intent, &style, &opts, None).unwrap();
    let rec = p.project.speech.as_ref().unwrap();
    let cue = rec
        .word_cues
        .iter()
        .find(|c| c.beat == 3 && c.group == "support")
        .expect("support group cued");
    assert_eq!(cue.word, "leak");
    let scene = beat_scenes(&p)[3];
    let leak = speech.words.iter().find(|w| w.text == "leak").unwrap();
    let first = scene
        .motions
        .iter()
        .filter(|m| m.target.starts_with("b4.support."))
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    let expect =
        leak.start - scene.start_seconds - motion_core::compiler::speech_plan::WORD_CUE_LEAD;
    assert!(
        (first - expect).abs() < 0.002,
        "first {first} expect {expect}"
    );
    assert!(validate(&p, None).is_ok());
    // Deterministic.
    let again = compile(&intent, &style, &opts, None).unwrap();
    assert_eq!(
        serde_json::to_string(&p).unwrap(),
        serde_json::to_string(&again).unwrap()
    );
}
