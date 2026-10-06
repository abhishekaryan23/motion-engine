//! Word cues v2 (0.20 C2) through the compiler: with a voice-over, groups
//! enter when the narrator says what they show. No builder declares reveal
//! anchors yet, so these compiles exercise the name-matching fallback
//! (stemming, spoken numbers, delays) and the shape of
//! `SpeechRecord.word_cues`. Anchor roles are covered by the unit tests in
//! `compiler/speech_plan.rs`.

use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::compiler::speech_plan::WORD_CUE_LEAD;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::scene::{MotionProject, Scene};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, WordCue, SPEECH_VERSION};
use motion_core::validate::validate;
use motion_core::{CreativeIntent, StyleProfile};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const INTENT: &str = include_str!("../../../examples/editorial_demo.intent.json");
const STYLE: &str = include_str!("../../../examples/editorial_demo.style.json");

/// What the narrator says per beat of the editorial demo: the statement plus
/// the words that name its pictures and numbers.
const LINES: [&str; 3] = [
    "Your salary stayed flat for five straight years",
    "Prices kept climbing while the basket rose thirty eight percent",
    "You are earning twenty seven percent less real buying power than you think",
];
const STARTS: [f64; 3] = [0.35, 4.6, 9.2];
const STEP: f64 = 0.3;

fn load() -> (CreativeIntent, StyleProfile) {
    (
        serde_json::from_str(INTENT).expect("intent"),
        serde_json::from_str(STYLE).expect("style"),
    )
}

/// One sentence per beat, its words `STEP` seconds apart.
fn speech() -> SpeechMap {
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    for (beat, (line, &start)) in LINES.iter().zip(&STARTS).enumerate() {
        let toks: Vec<&str> = line.split_whitespace().collect();
        for (i, t) in toks.iter().enumerate() {
            let at = start + i as f64 * STEP;
            words.push(SpeechWord {
                text: t.to_string(),
                start: (at * 1000.0).round() / 1000.0,
                end: ((at + 0.8 * STEP) * 1000.0).round() / 1000.0,
                confidence: 0.9,
            });
        }
        let end = start + toks.len() as f64 * STEP;
        sentences.push(SpeechSentence { beat, start, end });
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: sentences.last().map_or(0.0, |s| s.end) + 0.9,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
    }
}

fn compile(speech: Option<SpeechMap>) -> MotionProject {
    let (intent, style) = load();
    compile_with_options(
        &intent,
        &style,
        None,
        &AssetLibrary::new(Path::new(ASSETS)),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions {
            speech,
            ..CompileOptions::default()
        },
    )
    .expect("compile")
}

fn beat_scene(p: &MotionProject, beat: usize) -> &Scene {
    p.scenes
        .iter()
        .find(|s| s.id == format!("beat_{}", beat + 1))
        .expect("beat scene")
}

/// Earliest start of a group's motions before ANTICIPATE.
fn group_start(scene: &Scene, group: &str) -> f64 {
    let prefix = scene.id.replace("beat_", "b");
    let life = scene.lifecycle.expect("lifecycle");
    let id = format!("{prefix}.{group}");
    let nested = format!("{id}.");
    scene
        .motions
        .iter()
        .filter(|m| (m.target == id || m.target.starts_with(&nested)) && m.start < life.anticipate)
        .map(|m| m.start)
        .fold(f64::MAX, f64::min)
}

/// Scene-local start of the first word of `phrase` in `beat`'s sentence.
fn spoken_at(p: &MotionProject, beat: usize, phrase: &str) -> f64 {
    let first = phrase.split_whitespace().next().expect("phrase");
    let toks: Vec<&str> = LINES[beat].split_whitespace().collect();
    let k = toks.iter().position(|t| *t == first).expect("word in line");
    STARTS[beat] + k as f64 * STEP - beat_scene(p, beat).start_seconds
}

fn cues(p: &MotionProject) -> Vec<WordCue> {
    p.project
        .speech
        .as_ref()
        .expect("speech record")
        .word_cues
        .clone()
}

#[test]
fn fallback_cues_move_named_groups_onto_their_words() {
    let p = compile(Some(speech()));
    validate(&p, Some(Path::new(ASSETS))).expect("valid");
    let cues = cues(&p);
    let found: Vec<(usize, &str, &str)> = cues
        .iter()
        .map(|c| (c.beat, c.group.as_str(), c.word.as_str()))
        .collect();
    // The basket picture by its asset name, the "−27%" figure by the spoken
    // number, the "REAL BUYING POWER" label by its first word; per beat in
    // group-name order.
    assert_eq!(
        found,
        vec![
            (1, "pressure", "basket"),
            (2, "hero", "twenty seven percent"),
            (2, "label", "real"),
        ]
    );
    for c in &cues {
        let scene = beat_scene(&p, c.beat);
        // No builder declares anchors yet: every cue is the fallback, which
        // uses the content lead and is never limited here.
        assert_eq!(c.role, None, "{c:?}");
        assert!(!c.clamped, "{c:?}");
        let expect = spoken_at(&p, c.beat, &c.word) - WORD_CUE_LEAD;
        assert!((c.to - expect).abs() < 2e-3, "{c:?}: expected {expect}");
        assert!((c.from - c.to).abs() >= 0.15 - 1e-9, "{c:?}");
        // The group's entrance now starts on the recorded time.
        let start = group_start(scene, &c.group);
        assert!((start - c.to).abs() < 2e-3, "{c:?}: group starts {start}");
    }
    // The fallback may delay as well as advance (0.20).
    let hero = &cues[1];
    assert!(hero.to > hero.from, "{hero:?}");
    assert!(cues[0].to < cues[0].from && cues[2].to < cues[2].from);
    // Titles, statement copy, kicker and furniture never move by name
    // matching although "salary", "flat", "prices" and "less" are spoken.
    for fixed in ["head", "kicker", "ghost", "statement", "serif"] {
        assert!(cues.iter().all(|c| c.group != fixed), "{fixed} moved");
    }
}

#[test]
fn word_cue_records_keep_the_v1_shape_for_fallback_moves() {
    let p = compile(Some(speech()));
    let cues = cues(&p);
    let json = serde_json::to_value(&cues[0]).expect("json");
    let mut keys: Vec<&str> = json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    // `role` (None) and `clamped` (false) are omitted, so a fallback cue
    // serialises exactly as before 0.20.
    assert_eq!(keys, vec!["beat", "from", "group", "to", "word"]);
    // Round trip.
    let back: WordCue = serde_json::from_value(json).expect("parse");
    assert_eq!(back, cues[0]);
}

#[test]
fn word_cues_are_deterministic_and_absent_without_speech() {
    let a = serde_json::to_string(&compile(Some(speech()))).expect("a");
    let b = serde_json::to_string(&compile(Some(speech()))).expect("b");
    assert_eq!(a, b);
    let plain = compile(None);
    assert!(plain.project.speech.is_none());
}
