//! (0.20) Speech-aware lifecycle: with a voice-over, a beat's phases are placed
//! from its own spoken words (ENTER before the first content word, READ at the
//! end of the primary's clause, EVOLVE when B is named, ANTICIPATE after the
//! last word), clamped so no phase starves. See docs/SCENE_LIFECYCLE.md.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::speech_plan::WORD_CUE_LEAD;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Lifecycle, MotionProject, Scene};
use motion_core::speech::{
    repair, statement_tokens, BeatPhases, PhaseSource, SpeechMap, SpeechSentence, SpeechWord,
    SPEECH_VERSION,
};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn editorial_style() -> StyleProfile {
    serde_json::from_str(&read("examples/editorial_demo.style.json")).expect("style")
}

fn compile(
    intent: &CreativeIntent,
    style: &StyleProfile,
    speech: Option<SpeechMap>,
    art: Option<ArtMode>,
) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        speech,
        art,
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        intent,
        style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn beats(p: &MotionProject) -> Vec<&Scene> {
    p.scenes
        .iter()
        .filter(|s| s.id.starts_with("beat_"))
        .collect()
}

fn life(p: &MotionProject, beat: usize) -> Lifecycle {
    beats(p)[beat].lifecycle.expect("lifecycle")
}

fn phases(p: &MotionProject) -> Vec<BeatPhases> {
    p.project
        .speech
        .as_ref()
        .expect("speech record")
        .phases
        .clone()
}

/// A two-beat intent (contrast, then emphasize) whose narration lines are
/// given; `primary` / `secondary` are phrase values.
fn two_beats(lines: [&str; 2], primary: [&str; 2], secondary: Option<&str>) -> CreativeIntent {
    let mut first = serde_json::json!({
        "purpose": "contrast",
        "statement": "Two drinks, two moods",
        "narration": lines[0],
        "primary": { "kind": "phrase", "value": primary[0] },
        "energy": "building"
    });
    if let Some(b) = secondary {
        first["secondary"] = serde_json::json!({ "kind": "phrase", "value": b });
    }
    let intent = serde_json::json!({
        "version": "0.2",
        "title": "speech_lifecycle",
        "format": "vertical",
        "beats": [
            first,
            {
                "purpose": "emphasize",
                "statement": "Pick your cup",
                "narration": lines[1],
                "primary": { "kind": "phrase", "value": primary[1] },
                "energy": "calm"
            }
        ]
    });
    CreativeIntent::from_json(&intent.to_string()).expect("intent")
}

type Spoken<'a> = [(&'a str, f64, f64)];

/// A SpeechMap from hand-placed words, one sentence per beat (sentence =
/// first word start .. last word end).
fn speech_map(sentences: &[&Spoken]) -> SpeechMap {
    let mut words = Vec::new();
    let mut spans = Vec::new();
    for (beat, list) in sentences.iter().enumerate() {
        let start = list.first().map(|w| w.1).unwrap_or(0.0);
        let end = list.last().map(|w| w.2).unwrap_or(start);
        spans.push(SpeechSentence { beat, start, end });
        words.extend(list.iter().map(|(t, s, e)| SpeechWord {
            text: t.to_string(),
            start: *s,
            end: *e,
            confidence: 1.0,
        }));
    }
    let duration = spans.last().map(|s| s.end + 1.0).unwrap_or(1.0);
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences: spans,
        recognised: Vec::new(),
        alignment: None,
    }
}

/// The same map with one beat's words removed: the durations (from the
/// sentences) are unchanged and that beat keeps its planned fractions.
fn without_words_of(speech: &SpeechMap, beat: usize) -> SpeechMap {
    let mut out = speech.clone();
    let s = &speech.sentences[beat];
    out.words
        .retain(|w| !(w.start >= s.start - 1e-9 && w.start < s.end + 1e-9));
    out
}

fn ordered(l: &Lifecycle, duration: f64) -> bool {
    0.0 <= l.enter
        && l.enter <= l.settle
        && l.settle <= l.read
        && l.read <= l.evolve
        && l.evolve <= l.anticipate
        && l.anticipate <= l.bridge
        && l.bridge <= duration + 1e-9
}

const TEA_COFFEE: &Spoken = &[
    ("Tea", 0.60, 1.00),
    ("is", 1.10, 1.25),
    ("calm", 1.40, 2.20),
    ("coffee", 3.40, 3.80),
    ("is", 3.90, 4.05),
    ("loud", 4.15, 4.60),
];
const PICK: &Spoken = &[
    ("Pick", 7.60, 7.90),
    ("your", 7.95, 8.10),
    ("cup", 8.15, 8.50),
    ("wisely", 8.60, 9.20),
];

#[test]
fn a_is_verdict_b_is_verdict_places_every_phase_from_speech() {
    let intent = two_beats(
        ["Tea is calm, coffee is loud.", "Pick your cup wisely."],
        ["Tea", "Cup"],
        Some("Coffee"),
    );
    let speech = speech_map(&[TEA_COFFEE, PICK]);
    let p = compile(&intent, &editorial_style(), Some(speech.clone()), None);
    validate(&p, Some(&repo().join("assets"))).expect("valid");
    let l = life(&p, 0);
    let scene = beats(&p)[0];
    assert_eq!(scene.start_seconds, 0.0);
    // ENTER: WORD_CUE_LEAD before "Tea".
    assert!((l.enter - (0.60 - WORD_CUE_LEAD)).abs() < 1e-3, "{l:?}");
    // READ: at the comma (the end of "calm").
    assert!((l.read - 2.20).abs() < 1e-3, "{l:?}");
    // EVOLVE: when B ("coffee") is named; the word snap leaves it there.
    assert!((l.evolve - 3.40).abs() < 1e-3, "{l:?}");
    assert!(p
        .project
        .speech
        .as_ref()
        .expect("r")
        .evolve_snaps
        .iter()
        .all(|s| s.beat != 0));
    // ANTICIPATE: after the last word ("loud"), by a 0.25–0.6 s tail.
    let tail = 0.25 + 0.35 * (l.bridge - 4.60) / (l.bridge - 0.25);
    assert!((l.anticipate - (4.60 + tail)).abs() < 1e-3, "{l:?}");
    assert!(l.anticipate - 4.60 >= 0.25 - 1e-9 && l.anticipate - 4.60 <= 0.6 + 1e-9);
    assert!(ordered(&l, scene.duration_seconds), "{l:?}");

    let ph = phases(&p);
    assert_eq!(ph.len(), 2);
    assert_eq!(
        ph[0],
        BeatPhases {
            beat: 0,
            enter: PhaseSource::Speech,
            read: PhaseSource::Speech,
            evolve: PhaseSource::Speech,
            anticipate: PhaseSource::Speech,
        }
    );
    assert_eq!(ph[1].beat, 1);

    // The arrival keeps its planned length: SETTLE moves with ENTER.
    let planned = life(
        &compile(
            &intent,
            &editorial_style(),
            Some(without_words_of(&speech, 0)),
            None,
        ),
        0,
    );
    assert!(((l.settle - l.enter) - (planned.settle - planned.enter)).abs() < 2e-3);
    assert_eq!(l.bridge, planned.bridge);
}

#[test]
fn a_sentence_that_names_nothing_keeps_the_fractions() {
    // Rain is never said, no B, no keyword, one clause; the first word is
    // spoken as the beat opens and the last runs up to the next sentence.
    let intent = two_beats(
        ["Storms happen every single day", "Pick your cup wisely."],
        ["Rain", "Cup"],
        None,
    );
    let storms: &Spoken = &[
        ("Storms", 0.35, 0.90),
        ("happen", 1.00, 1.60),
        ("every", 1.80, 2.40),
        ("single", 2.60, 3.20),
        ("day", 3.40, 4.40),
    ];
    let pick: &Spoken = &[
        ("Pick", 4.50, 4.80),
        ("your", 4.85, 5.00),
        ("cup", 5.05, 5.40),
        ("wisely", 5.50, 6.10),
    ];
    let speech = speech_map(&[storms, pick]);
    let p = compile(&intent, &editorial_style(), Some(speech.clone()), None);
    let ph = phases(&p);
    assert_eq!(
        ph[0],
        BeatPhases {
            beat: 0,
            enter: PhaseSource::Fraction,
            read: PhaseSource::Fraction,
            evolve: PhaseSource::Fraction,
            anticipate: PhaseSource::Fraction,
        }
    );
    // The same boundaries as the beat with no words at all (EVOLVE may still
    // snap onto a nearby word start afterwards).
    let base = compile(
        &intent,
        &editorial_style(),
        Some(without_words_of(&speech, 0)),
        None,
    );
    let (l, b) = (life(&p, 0), life(&base, 0));
    assert_eq!(
        (l.enter, l.settle, l.read, l.anticipate, l.bridge),
        (b.enter, b.settle, b.read, b.anticipate, b.bridge)
    );
    let snap = p
        .project
        .speech
        .as_ref()
        .expect("r")
        .evolve_snaps
        .iter()
        .find(|s| s.beat == 0)
        .cloned();
    match snap {
        Some(s) => assert!((s.from - b.evolve).abs() < 1e-3, "{s:?} vs {b:?}"),
        None => assert_eq!(l.evolve, b.evolve),
    }
}

#[test]
fn a_short_sentence_in_a_long_beat_keeps_every_minimum_span() {
    let intent = two_beats(
        ["Tea is calm.", "Pick your cup wisely."],
        ["Tea", "Cup"],
        Some("Coffee"),
    );
    let short: &Spoken = &[
        ("Tea", 0.60, 0.80),
        ("is", 0.85, 0.95),
        ("calm", 1.00, 1.30),
    ];
    let pick: &Spoken = &[
        ("Pick", 10.60, 10.90),
        ("your", 10.95, 11.10),
        ("cup", 11.15, 11.50),
        ("wisely", 11.60, 12.20),
    ];
    let speech = speech_map(&[short, pick]);
    let p = compile(&intent, &editorial_style(), Some(speech.clone()), None);
    validate(&p, Some(&repo().join("assets"))).expect("valid");
    let base = compile(
        &intent,
        &editorial_style(),
        Some(without_words_of(&speech, 0)),
        None,
    );
    let (l, b) = (life(&p, 0), life(&base, 0));
    let scene = beats(&p)[0];
    assert!(ordered(&l, scene.duration_seconds), "{l:?}");
    let tol = 2e-3;
    // ENTER never before the overlap ends; the arrival and SETTLE keep their
    // planned lengths.
    assert!(l.enter >= b.enter - tol);
    assert!(
        ((l.settle - l.enter) - (b.settle - b.enter)).abs() < tol,
        "{l:?} {b:?}"
    );
    assert!(
        l.read - l.settle >= (b.read - b.settle) - tol,
        "{l:?} {b:?}"
    );
    // READ keeps max(0.5 s, 40 % of its planned span), EVOLVE max(0.4 s,
    // 40 % of its planned span); ANTICIPATE is never shorter than planned.
    let read_min = 0.5f64.max(0.4 * (b.evolve - b.read));
    assert!(l.evolve - l.read >= read_min - tol, "{l:?} {b:?}");
    let evolve_min = 0.4f64.max(0.4 * (b.anticipate - b.evolve));
    assert!(l.anticipate - l.evolve >= evolve_min - tol, "{l:?} {b:?}");
    assert!(
        l.bridge - l.anticipate >= (b.bridge - b.anticipate) - tol,
        "{l:?} {b:?}"
    );
    assert_eq!(l.bridge, b.bridge);
    // The read was asked for too early: it was pushed, still from speech.
    let ph = phases(&p);
    assert_eq!(ph[0].enter, PhaseSource::Speech);
    assert_eq!(ph[0].anticipate, PhaseSource::Speech);
}

#[test]
fn a_long_sentence_in_a_short_beat_never_passes_the_bridge() {
    let line = "Tea is calm and slow and warm and kind, coffee is loud and quick and \
                sharp and bright and keeps you up all night.";
    let intent = two_beats(
        [line, "Pick your cup wisely."],
        ["Tea", "Cup"],
        Some("Coffee"),
    );
    let tokens = statement_tokens(line);
    let each = 4.6 / tokens.len() as f64;
    let long: Vec<(&str, f64, f64)> = tokens
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let s = 0.35 + i as f64 * each;
            (t.as_str(), s, s + 0.85 * each)
        })
        .collect();
    let last_end = long.last().map(|w| w.2).expect("words");
    let pick: &Spoken = &[
        ("Pick", 5.05, 5.30),
        ("your", 5.35, 5.50),
        ("cup", 5.55, 5.90),
        ("wisely", 6.00, 6.60),
    ];
    let speech = speech_map(&[&long, pick]);
    let p = compile(&intent, &editorial_style(), Some(speech.clone()), None);
    validate(&p, Some(&repo().join("assets"))).expect("valid");
    let base = compile(
        &intent,
        &editorial_style(),
        Some(without_words_of(&speech, 0)),
        None,
    );
    let (l, b) = (life(&p, 0), life(&base, 0));
    let scene = beats(&p)[0];
    assert!(ordered(&l, scene.duration_seconds), "{l:?}");
    // The narration runs past the bridge: ANTICIPATE stays on its plan.
    assert!(last_end > l.bridge, "fixture: {last_end} vs {l:?}");
    assert!(l.anticipate <= l.bridge);
    assert!(l.anticipate <= b.anticipate + 1e-9, "{l:?} {b:?}");
    assert_eq!(phases(&p)[0].anticipate, PhaseSource::Fraction);
    let evolve_min = 0.4f64.max(0.4 * (b.anticipate - b.evolve));
    assert!(l.anticipate - l.evolve >= evolve_min.min(b.anticipate - b.evolve) - 2e-3);
}

// ---------------------------------------------------------------------------
// Examples with a synthetic (seeded) voice-over
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    /// splitmix64 in `[0, 1)`.
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Every beat's spoken line (narration, else statement) voiced word by word
/// with seeded durations, gaps and clause pauses; sentences at least 3.6 s
/// apart so every beat keeps room for its visuals. Repaired like the CLI does.
fn synthetic_speech(intent: &CreativeIntent, seed: u64) -> SpeechMap {
    let mut rng = Rng(seed);
    let lines: Vec<String> = intent
        .beats
        .iter()
        .map(|b| b.narration.clone().unwrap_or_else(|| b.statement.clone()))
        .collect();
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let mut start = 0.3 + 0.2 * rng.next();
    for (beat, line) in lines.iter().enumerate() {
        let mut t = start;
        for raw in line.split_whitespace() {
            let Some(text) = statement_tokens(raw).into_iter().next() else {
                continue;
            };
            let d = 0.12 + 0.045 * text.chars().count().min(10) as f64 + 0.08 * rng.next();
            words.push(SpeechWord {
                text,
                start: t,
                end: t + d,
                confidence: 0.9,
            });
            t += d + 0.03 + 0.08 * rng.next();
            if raw.ends_with([',', ';', ':', '.', '!', '?']) {
                t += 0.15 + 0.2 * rng.next();
            }
        }
        let end = words.last().map(|w: &SpeechWord| w.end).unwrap_or(t);
        sentences.push(SpeechSentence { beat, start, end });
        start = (end + 0.4 + 0.6 * rng.next()).max(start + 3.6);
    }
    let map = SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: sentences.last().map(|s| s.end + 1.0).unwrap_or(1.0),
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
        recognised: Vec::new(),
        alignment: None,
    };
    repair(&map, &lines).0
}

fn check_examples(intent_path: &str, style_path: &str) {
    let intent = CreativeIntent::from_json(&read(intent_path)).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read(style_path)).expect("style");
    let mut speech_phases = 0;
    for seed in [7u64, 20, 2024] {
        let speech = synthetic_speech(&intent, seed);
        for art in [None, Some(ArtMode::Auto)] {
            let p = compile(&intent, &style, Some(speech.clone()), art);
            validate(&p, None).unwrap_or_else(|e| panic!("{intent_path} {seed} {art:?}: {e}"));
            let scenes = beats(&p);
            assert_eq!(scenes.len(), intent.beats.len());
            for s in &scenes {
                let l = s.lifecycle.expect("lifecycle");
                assert!(
                    ordered(&l, s.duration_seconds),
                    "{intent_path} seed {seed} {art:?} {}: {l:?}",
                    s.id
                );
                assert!(
                    l.read < l.evolve && l.evolve < l.anticipate,
                    "{}: {l:?}",
                    s.id
                );
            }
            let ph = phases(&p);
            assert_eq!(ph.len(), intent.beats.len());
            for (i, b) in ph.iter().enumerate() {
                assert_eq!(b.beat, i);
                speech_phases += [b.enter, b.read, b.evolve, b.anticipate]
                    .iter()
                    .filter(|s| **s == PhaseSource::Speech)
                    .count();
            }
            // Deterministic.
            let again = compile(&intent, &style, Some(speech.clone()), art);
            assert_eq!(
                serde_json::to_string(&p).expect("json"),
                serde_json::to_string(&again).expect("json")
            );
        }
    }
    assert!(
        speech_phases > 0,
        "{intent_path}: no phase was placed from speech"
    );
}

#[test]
fn the_phase_order_holds_on_the_editorial_demo_with_speech() {
    check_examples(
        "examples/editorial_demo.intent.json",
        "examples/editorial_demo.style.json",
    );
}

#[test]
fn the_phase_order_holds_on_the_space_example_with_speech() {
    check_examples(
        "examples/cinematic/space.intent.json",
        "examples/cinematic/space.style.json",
    );
}

#[test]
fn without_speech_nothing_is_recorded() {
    let intent =
        CreativeIntent::from_json(&read("examples/cinematic/space.intent.json")).expect("intent");
    let style: StyleProfile =
        serde_json::from_str(&read("examples/cinematic/space.style.json")).expect("style");
    let p = compile(&intent, &style, None, Some(ArtMode::Auto));
    assert!(p.project.speech.is_none());
    assert!(!serde_json::to_string(&p)
        .expect("json")
        .contains("\"phases\""));
}
