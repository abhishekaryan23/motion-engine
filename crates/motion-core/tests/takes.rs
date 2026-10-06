//! (0.23 B2a) Takes: a story gets another version on request, and nothing
//! changes without a variety seed.
//!
//! Under a direction seed (`CompileOptions.variety` + `take`) each beat picks
//! among curated template alternates (`grammar::candidates`), the position
//! rotations (cinematic arrival side, FX camera move, hype layout cycle,
//! subject-first image layout) become seeded sequences in which neighbours
//! never repeat, and what was chosen is written to `ProjectMeta.direction`.
//!
//! The eight bench stories (the variety bench's matrix with its committed
//! offline speech fixtures) are compiled the way the product path compiles
//! them (`--art auto --variety auto --speech`). The synthetic sweep at the end
//! vets every curated alternate: it must show every value the semantic choice
//! shows, for every take.

use std::collections::BTreeSet;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::direction::BeatDirection;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_with_options, compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions,
    CompileWarning, WARN_VALUE_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_report;
use motion_core::scene::MotionProject;
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord};
use motion_core::style::StyleProfile;

/// The variety bench's stories (`scripts/variety_bench.sh --list-stories`).
const STORIES: [(&str, &str); 8] = [
    (
        "sleep_review",
        "docs/plans/sprint_0_23/stories/sleep_review.intent.json",
    ),
    (
        "money_review",
        "docs/plans/sprint_0_23/stories/money_review.intent.json",
    ),
    (
        "collection-accumulate",
        "examples/public/collection-accumulate.intent.json",
    ),
    ("state-change", "examples/public/state-change.intent.json"),
    (
        "derived-metric",
        "examples/public/derived-metric.intent.json",
    ),
    ("layers", "examples/public/layers.intent.json"),
    ("ai_learns", "examples/topics/ai_learns.intent.json"),
    ("space", "examples/cinematic/space.intent.json"),
];

const TONES: [&str; 9] = [
    "auto",
    "editorial",
    "technical",
    "playful",
    "street",
    "documentary",
    "hype",
    "studio",
    "cinematic",
];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The CLI's `--variety auto` seed (FNV-1a over the title and statements).
fn story_seed(intent: &CreativeIntent) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    eat(&intent.title);
    for b in &intent.beats {
        eat(&b.statement);
    }
    h
}

/// A tone name, or a style object (`{"tone": ..., "polarity": ...}`).
fn style(tone: &str) -> StyleProfile {
    if tone.starts_with('{') {
        return serde_json::from_str(tone).expect("style");
    }
    serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style")
}

fn intent_of(story: usize) -> CreativeIntent {
    CreativeIntent::from_json(&read(STORIES[story].1)).expect("intent")
}

/// The committed offline voice-over of a bench story, repaired against its
/// spoken lines as `compile --speech` does.
fn fixture_speech(story: usize, intent: &CreativeIntent) -> SpeechMap {
    let map = SpeechMap::from_json(&read(&format!(
        "golden/fixtures/variety/{}.speech.json",
        STORIES[story].0
    )))
    .expect("speech fixture");
    let spoken: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    repair(&map, &spoken).0
}

/// A voice-over that says each beat's narration (else statement), words
/// 0.32 s apart and 0.5 s between beats.
fn synthetic_speech(intent: &CreativeIntent) -> SpeechMap {
    let (mut words, mut sentences) = (Vec::new(), Vec::new());
    let mut t = 0.3;
    for (i, beat) in intent.beats.iter().enumerate() {
        let line = beat
            .narration
            .clone()
            .unwrap_or_else(|| beat.statement.clone());
        let start = t;
        for w in motion_core::speech::statement_tokens(&line) {
            words.push(SpeechWord {
                text: w,
                start: (t * 1000.0_f64).round() / 1000.0,
                end: ((t + 0.27) * 1000.0_f64).round() / 1000.0,
                confidence: 1.0,
            });
            t += 0.32;
        }
        sentences.push(SpeechSentence {
            beat: i,
            start,
            end: t,
        });
        t += 0.5;
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: t,
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

/// How a compile is asked for.
#[derive(Clone, Copy)]
struct Ask<'a> {
    tone: &'a str,
    /// `Some(take)` = the product path's variety seed with this take.
    take: Option<u64>,
}

fn compile_intent(
    intent: &CreativeIntent,
    speech: Option<&SpeechMap>,
    ask: Ask,
) -> (MotionProject, Vec<CompileWarning>) {
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: ask.take.map(|_| story_seed(intent)),
        take: ask.take.unwrap_or(0),
        speech: speech.cloned(),
        ..CompileOptions::default()
    };
    compile_with_report(
        intent,
        &style(ask.tone),
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap_or_else(|e| panic!("{} take {:?}: {e}", ask.tone, ask.take))
}

fn compile_story(story: usize, ask: Ask) -> (MotionProject, Vec<CompileWarning>) {
    let intent = intent_of(story);
    let speech = fixture_speech(story, &intent);
    compile_intent(&intent, Some(&speech), ask)
}

fn json(p: &MotionProject) -> String {
    p.to_json_pretty()
}

/// The beat scenes' content, without the project record (the direction record
/// always differs between takes: it carries the take and its seed).
fn scenes_json(p: &MotionProject) -> String {
    serde_json::to_string(&p.scenes).expect("scenes")
}

fn beats_of(p: &MotionProject) -> &[BeatDirection] {
    &p.project
        .direction
        .as_ref()
        .expect("a variety compile writes ProjectMeta.direction")
        .beats
}

// ---------------------------------------------------------------------------
// Without a variety seed nothing changes
// ---------------------------------------------------------------------------

#[test]
fn without_a_variety_seed_there_is_no_record_and_a_take_changes_nothing() {
    for story in [0, 1, 6] {
        let intent = intent_of(story);
        let speech = fixture_speech(story, &intent);
        for tone in ["editorial", "cinematic", "hype"] {
            let plain = compile_intent(&intent, Some(&speech), Ask { tone, take: None }).0;
            assert!(plain.project.direction.is_none(), "{tone}");
            // `take` is read only under `variety`.
            let opts = CompileOptions {
                art: Some(ArtMode::Auto),
                take: 3,
                speech: Some(speech.clone()),
                ..CompileOptions::default()
            };
            let taken = compile_with_options(
                &intent,
                &style(tone),
                None,
                &AssetLibrary::new(repo().join("assets")),
                &ApproxMeasure,
                &AssetManifest::empty(),
                None,
                &opts,
            )
            .expect("compile");
            assert_eq!(json(&plain), json(&taken), "{} x {tone}", STORIES[story].0);
        }
    }
}

// ---------------------------------------------------------------------------
// Takes
// ---------------------------------------------------------------------------

#[test]
fn the_same_story_tone_and_take_is_byte_identical() {
    for (story, tone, take) in [
        (0, "editorial", 1),
        (1, "playful", 2),
        (6, "cinematic", 3),
        (7, "hype", 1),
    ] {
        let first = json(
            &compile_story(
                story,
                Ask {
                    tone,
                    take: Some(take),
                },
            )
            .0,
        );
        for _ in 0..2 {
            let again = json(
                &compile_story(
                    story,
                    Ask {
                        tone,
                        take: Some(take),
                    },
                )
                .0,
            );
            assert_eq!(first, again, "{} x {tone} take {take}", STORIES[story].0);
        }
    }
}

#[test]
fn take_zero_and_take_one_differ_for_every_bench_story() {
    // Stories made only of structured subjects (a collection, a state change)
    // keep their one template per beat in every look; the direction record
    // still differs, and their cinematic camera moves only where the beat has
    // room for more than the orbit and the push-in.
    const STRUCTURAL: [&str; 2] = ["collection-accumulate", "state-change"];
    for (story, &(name, _)) in STORIES.iter().enumerate() {
        let mut content_differs = false;
        for tone in TONES {
            let a = compile_story(
                story,
                Ask {
                    tone,
                    take: Some(0),
                },
            )
            .0;
            for take in 1..4 {
                let b = compile_story(
                    story,
                    Ask {
                        tone,
                        take: Some(take),
                    },
                )
                .0;
                let (da, db) = (
                    a.project.direction.as_ref().expect("record"),
                    b.project.direction.as_ref().expect("record"),
                );
                assert_ne!(da, db, "{name} x {tone}: same direction record");
                assert_eq!((da.take, db.take), (0, take));
                assert_ne!(da.seed, db.seed);
                assert_ne!(json(&a), json(&b), "{name} x {tone}: same scene");
                content_differs |= scenes_json(&a) != scenes_json(&b);
            }
        }
        // Every other bench story has a tone in which a take moves a beat (a
        // template alternate or a seeded rotation), not only the record.
        assert!(
            content_differs || STRUCTURAL.contains(&name),
            "{name}: no tone changes with the take"
        );
    }
}

#[test]
fn takes_differ_in_what_was_chosen() {
    // A story with phrase beats and genre-free tones: the template alternates.
    let story = 0;
    for tone in ["editorial", "playful"] {
        let chosen: BTreeSet<Vec<(String, u8)>> = (0..6)
            .map(|take| {
                let p = compile_story(
                    story,
                    Ask {
                        tone,
                        take: Some(take),
                    },
                )
                .0;
                beats_of(&p)
                    .iter()
                    .map(|d| (d.template.clone(), d.alternate))
                    .collect()
            })
            .collect();
        assert!(
            chosen.len() >= 2,
            "{tone}: six takes chose the same templates"
        );
    }
    // The cinematic look: arrival side and camera move rotate with the seed. A
    // story whose beats all fit the crane and the truck (ai_learns: pictures
    // with short labels) has the whole vocabulary to rotate through; a story of
    // wide type (sleep_review) has only the push-in and the orbit, so its
    // neighbours must alternate and only the arrival side varies.
    let sets: BTreeSet<Vec<std::collections::BTreeMap<String, String>>> = (0..6)
        .map(|take| {
            let p = compile_story(
                6,
                Ask {
                    tone: "cinematic",
                    take: Some(take),
                },
            )
            .0;
            beats_of(&p).iter().map(|d| d.rotations.clone()).collect()
        })
        .collect();
    assert!(
        sets.len() >= 4,
        "cinematic rotations barely differ: {}",
        sets.len()
    );
}

#[test]
fn the_cinematic_camera_never_opens_on_the_crane_and_never_repeats() {
    // The crane enters smaller than its layout (a backward dolly), which the
    // opening's 0.5 s dead-air limit cannot take, and it zooms type that
    // already touches the safe edge out of the frame (bench: sleep_review and
    // money_review `text_outside_safe`): the seeded rotation keeps it off such
    // beats. Neighbouring beats never share a move: a turning sphere
    // (`sphere_gallery`) is forced onto the push-in and counts as one, so the
    // beats around it are not push-ins (unless no other move fits them).
    let mut moves = BTreeSet::new();
    for (story, &(name, _)) in STORIES.iter().enumerate() {
        for take in 0..16 {
            let p = compile_story(
                story,
                Ask {
                    tone: "cinematic",
                    take: Some(take),
                },
            )
            .0;
            let picks: Vec<Option<&str>> = beats_of(&p).iter().map(camera_of).collect();
            assert_ne!(picks[0], Some("crane"), "{} take {take}", name);
            for w in picks.windows(2) {
                if let (Some(a), Some(b)) = (w[0], w[1]) {
                    assert_ne!(a, b, "{} take {take}", name);
                }
            }
            moves.extend(picks.into_iter().flatten().map(str::to_string));
        }
    }
    // The vocabulary is still used.
    assert!(moves.len() >= 3, "{moves:?}");
}

#[test]
fn the_record_names_every_choice_and_its_candidates() {
    let p = compile_story(
        0,
        Ask {
            tone: "cinematic",
            take: Some(2),
        },
    )
    .0;
    let d = p.project.direction.as_ref().expect("record");
    assert_eq!(d.take, 2);
    assert_eq!(d.chosen, 0);
    assert!(d.candidates.is_empty() && d.shipped_with_failure.is_none());
    assert_eq!(d.beats.len(), 5);
    assert!(
        d.beats.iter().any(|b| !b.params.is_identity()),
        "the planner gives the beats their direction"
    );
    for (i, b) in d.beats.iter().enumerate() {
        assert_eq!(b.beat, i);
        assert!(!b.template.is_empty() && !b.reason.is_empty(), "{b:?}");
        assert!(b.alternates >= 1 && b.alternate < b.alternates, "{b:?}");
        // (0.23 W2d) The planner's params and the beat's role are recorded.
        assert!(!b.role.is_empty(), "{b:?}");
        if i == 0 {
            assert_eq!(b.role, "hook");
        }
        // Cinematic beats are staged for the moving camera: every one has a
        // seeded camera move, and the beats with a hero picture an arrival.
        let camera = b.rotations.get("camera").map(String::as_str);
        assert!(
            matches!(camera, Some("push_in" | "truck" | "crane" | "orbit"))
                || b.template == "sphere_gallery",
            "{b:?}"
        );
        for (dim, option) in &b.rotations {
            assert!(!option.is_empty(), "{dim}");
            assert!(
                ["arrival", "camera", "slam_layout", "image_layout"].contains(&dim.as_str()),
                "{dim}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Neighbours never repeat
// ---------------------------------------------------------------------------

/// The main camera move of a beat: its seeded one, or the push-in a turning
/// sphere (`sphere_gallery`) is forced onto.
fn camera_of(d: &BeatDirection) -> Option<&str> {
    if d.template == "sphere_gallery" {
        Some("push_in")
    } else {
        d.rotations.get("camera").map(String::as_str)
    }
}

#[test]
fn neighbouring_beats_never_repeat_a_template_a_rotated_arrival_or_a_camera_move() {
    for (story, &(name, _)) in STORIES.iter().enumerate() {
        for tone in TONES {
            for take in 0..4 {
                let p = compile_story(
                    story,
                    Ask {
                        tone,
                        take: Some(take),
                    },
                )
                .0;
                for w in beats_of(&p).windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    let ctx = format!("{} x {tone} take {take} beats {}-{}", name, a.beat, b.beat);
                    if b.alternates > 1 {
                        assert_ne!(a.template, b.template, "{ctx}: template repeats");
                    }
                    if let (Some(x), Some(y)) =
                        (a.rotations.get("arrival"), b.rotations.get("arrival"))
                    {
                        assert_ne!(x, y, "{ctx}: arrival repeats");
                    }
                    if let (Some(x), Some(y)) = (camera_of(a), camera_of(b)) {
                        assert_ne!(x, y, "{ctx}: camera move repeats");
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Alternates show what the semantic choice shows
// ---------------------------------------------------------------------------

/// One beat of each shape of the tie table, plus neighbours that exercise the
/// no-repeat rule. `{pos}` beats are the ones whose alternates are vetted.
const SWEEP: &str = r#"{
  "version": "0.2", "title": "alternates", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Sleep is a skill",
     "narration": "Sleep is a skill you can train, like any other.",
     "primary": {"kind": "phrase", "value": "Rest", "meaning": "deep sleep"},
     "energy": "impact", "keyword": "skill"},
    {"purpose": "reveal", "statement": "The hidden cost of a late night",
     "narration": "The hidden cost of a late night shows up the next afternoon.",
     "primary": {"kind": "phrase", "value": "Brain fog", "meaning": "the next afternoon"},
     "secondary": {"kind": "phrase", "value": "Slower decisions", "meaning": "all day"},
     "energy": "building", "keyword": "cost"},
    {"purpose": "compare", "statement": "Early start beats a late start",
     "narration": "Start at twenty five and you end with 240000 dollars, start at thirty five and you end with 120000.",
     "primary": {"kind": "number", "value": "$240,000", "meaning": "start at 25"},
     "secondary": {"kind": "number", "value": "$120,000", "meaning": "start at 35"},
     "relationship": "separate", "energy": "impact", "keyword": "start"},
    {"purpose": "explain", "statement": "How a habit forms",
     "narration": "A cue triggers a routine, and the routine earns a reward.",
     "primary": {"kind": "phrase", "value": "Cue", "meaning": "the trigger"},
     "secondary": {"kind": "phrase", "value": "Reward", "meaning": "the payoff"},
     "relationship": "replace", "energy": "building", "keyword": "habit"},
    {"purpose": "contrast", "statement": "Tired against rested",
     "narration": "Tired runs at 40 percent, rested at 90 percent.",
     "primary": {"kind": "number", "value": "40%", "meaning": "tired"},
     "secondary": {"kind": "number", "value": "90%", "meaning": "rested"},
     "relationship": "grow", "energy": "calm"},
    {"purpose": "emphasize", "statement": "Protect the first hour",
     "narration": "Protect the first hour of the morning.",
     "primary": {"kind": "phrase", "value": "First hour", "meaning": "the anchor of the day"},
     "secondary": {"kind": "phrase", "value": "Mornings", "meaning": "set the tone"},
     "energy": "building", "keyword": "hour"}
  ]
}"#;

fn sweep_intent() -> CreativeIntent {
    CreativeIntent::from_json(SWEEP).expect("sweep intent")
}

fn value_dropped(warnings: &[CompileWarning]) -> Vec<String> {
    warnings
        .iter()
        .filter(|w| w.code == WARN_VALUE_DROPPED)
        .map(|w| format!("beat {:?}: {}", w.beat.map(|b| b + 1), w.message))
        .collect()
}

fn hard_findings(p: &MotionProject) -> usize {
    let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
    layout_report(p, &frame).findings.len()
}

#[test]
fn every_alternate_shows_every_value_the_semantic_choice_shows() {
    let intent = sweep_intent();
    let speech = synthetic_speech(&intent);
    let mut seen_alternates: BTreeSet<(usize, String)> = BTreeSet::new();
    for tone in [
        "auto",
        "editorial",
        "technical",
        "playful",
        r#"{"tone": "editorial", "temperament": "restrained", "density": "sparse", "polarity": "light", "temperature": "warm"}"#,
        r#"{"tone": "playful", "polarity": "dark"}"#,
    ] {
        for voice in [None, Some(&speech)] {
            // The semantic choice (no variety) is the reference.
            let (plain, plain_w) = compile_intent(&intent, voice, Ask { tone, take: None });
            let reference = value_dropped(&plain_w);
            let reference_findings = hard_findings(&plain);
            for take in 0..24 {
                let (p, w) = compile_intent(
                    &intent,
                    voice,
                    Ask {
                        tone,
                        take: Some(take),
                    },
                );
                let dropped = value_dropped(&w);
                assert!(
                    dropped.iter().all(|d| reference.contains(d)),
                    "{tone} voice {} take {take}: {dropped:?} (semantic: {reference:?})",
                    voice.is_some()
                );
                for d in beats_of(&p) {
                    seen_alternates.insert((d.beat, d.template.clone()));
                }
                let found = hard_findings(&p);
                assert!(
                    found <= reference_findings,
                    "{tone} voice {} take {take}: {found} layout findings vs {reference_findings}",
                    voice.is_some()
                );
            }
        }
    }
    // The sweep reaches the alternates of every row of the tie table.
    for (beat, template) in [
        (0, "kinetic_poster"),
        (0, "editorial_collage"),
        (0, "cinematic_multiplane"),
        (2, "data_story"),
        (2, "split_contrast"),
        (3, "sequential_stack"),
        (3, "spatial_cause_effect"),
    ] {
        assert!(
            seen_alternates.contains(&(beat, template.to_string())),
            "beat {beat} never used {template}: {seen_alternates:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// The alternates of every row show the same words as the semantic template
// ---------------------------------------------------------------------------

/// Upper-cased alphanumeric words.
fn words_of(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_uppercase)
        .collect()
}

/// The words of every text layer of a beat scene and of the shared elements
/// that run through it (a carried subject is on screen in the beat), plus the
/// letters that a run of single-glyph layers spells (a tracking reveal sets
/// `S`, `l`, `o`, `w`, ... as layers of their own): the last entry of the set
/// is all the layers' letters in a row.
fn shown_words(p: &MotionProject, scene: &motion_core::scene::Scene) -> BTreeSet<String> {
    use motion_core::scene::{Layer, LayerKind};
    fn walk(layers: &[Layer], out: &mut BTreeSet<String>, letters: &mut String) {
        for l in layers {
            match &l.kind {
                LayerKind::Text(t) => {
                    out.extend(words_of(&t.text));
                    letters.extend(
                        t.text
                            .chars()
                            .filter(|c| c.is_alphanumeric())
                            .flat_map(char::to_uppercase),
                    );
                }
                LayerKind::Group { children } => walk(children, out, letters),
                _ => {}
            }
        }
    }
    let (mut out, mut letters) = (BTreeSet::new(), String::new());
    walk(&scene.layers, &mut out, &mut letters);
    for shared in p
        .shared
        .iter()
        .filter(|e| e.track.iter().any(|k| k.scene == scene.id))
    {
        walk(std::slice::from_ref(&shared.layer), &mut out, &mut letters);
    }
    out.insert(letters);
    out
}

/// Whether `word` is shown by `shown` (a whole word of a layer, or, from four
/// letters, spelled in a row by the layers' letters).
fn is_shown(shown: &BTreeSet<String>, word: &str) -> bool {
    shown.contains(word) || (word.chars().count() >= 4 && shown.iter().any(|s| s.contains(word)))
}

/// The words the intent gives a beat to show: its statement, keyword and the
/// value and meaning of its subjects.
fn intent_words(beat: &motion_core::intent::Beat) -> BTreeSet<String> {
    let mut out = words_of(&beat.statement);
    out.extend(beat.keyword.iter().flat_map(|k| words_of(k)));
    for s in std::iter::once(&beat.primary).chain(beat.secondary.as_ref()) {
        out.extend(s.value().into_iter().flat_map(words_of));
        out.extend(s.meaning().into_iter().flat_map(words_of));
    }
    out
}

/// Every phrase pair a tie-table row applies to: (purpose, relationship) of a
/// beat with a primary phrase and a secondary phrase, and the same with a lone
/// primary; plus two numbers compared.
fn generated_sweep() -> CreativeIntent {
    use serde_json::json;
    let mut beats = Vec::new();
    for purpose in ["emphasize", "reveal", "explain"] {
        for rel in [None, Some("grow"), Some("replace"), Some("carry")] {
            for pair in [false, true] {
                let mut b = json!({
                    "purpose": purpose,
                    "statement": format!("{purpose} {} {}", rel.unwrap_or("plain"), if pair { "pair" } else { "single" }),
                    "narration": format!("A line that says {purpose} {} for the {}.", rel.unwrap_or("plain"), if pair { "pair" } else { "single" }),
                    "primary": {"kind": "phrase", "value": "Slow mornings", "meaning": "the first hour"},
                    "energy": "building",
                    "keyword": "mornings"
                });
                if pair {
                    b["secondary"] = json!({"kind": "phrase", "value": "Quick evenings", "meaning": "the last hour"});
                }
                if let Some(r) = rel {
                    b["relationship"] = json!(r);
                }
                beats.push(b);
            }
        }
    }
    // A beat that carries a subject on, then a beat that takes it over.
    for (carry_purpose, carry_pair) in [("emphasize", true), ("explain", true), ("reveal", false)] {
        for follow in ["explain", "emphasize"] {
            let mut first = json!({
                "purpose": carry_purpose, "statement": format!("{carry_purpose} then carry"),
                "narration": "A beat that carries its subject on.",
                "primary": {"kind": "phrase", "value": "Carried phrase", "meaning": "carried on"},
                "relationship": "carry", "energy": "building"
            });
            if carry_pair {
                first["secondary"] =
                    json!({"kind": "phrase", "value": "Second phrase", "meaning": "stays behind"});
            }
            beats.push(first);
            beats.push(json!({
                "purpose": follow, "statement": format!("{follow} after carry"),
                "narration": "A beat that comes after a carry.",
                "primary": {"kind": "phrase", "value": "Fresh phrase", "meaning": "new subject"},
                "secondary": {"kind": "phrase", "value": "Partner phrase", "meaning": "its partner"},
                "continuity": "carry_secondary", "energy": "building"
            }));
        }
    }
    for follow in ["explain", "emphasize"] {
        beats.push(json!({
            "purpose": "emphasize", "statement": "carries the phrase on",
            "narration": "A beat that carries its primary on.",
            "primary": {"kind": "phrase", "value": "Shared phrase", "meaning": "persists"},
            "relationship": "carry", "energy": "building"
        }));
        beats.push(json!({
            "purpose": follow, "statement": format!("{follow} takes the phrase over"),
            "narration": "A beat that starts from the carried phrase.",
            "primary": {"kind": "phrase", "value": "Shared phrase", "meaning": "persists"},
            "secondary": {"kind": "phrase", "value": "Added phrase", "meaning": "joins it"},
            "energy": "building"
        }));
    }
    for purpose in ["compare", "contrast"] {
        for rel in [None, Some("grow"), Some("separate"), Some("replace")] {
            let mut b = json!({
                "purpose": purpose,
                "statement": format!("{purpose} two figures {}", rel.unwrap_or("plain")),
                "narration": "The first figure is 1200 dollars and the second is 300 dollars.",
                "primary": {"kind": "number", "value": "$1,200", "meaning": "first figure"},
                "secondary": {"kind": "number", "value": "$300", "meaning": "second figure"},
                "energy": "impact", "keyword": "figure"
            });
            if let Some(r) = rel {
                b["relationship"] = json!(r);
            }
            beats.push(b);
        }
    }
    CreativeIntent::from_json(
        &json!({"version": "0.2", "title": "generated alternates", "format": "vertical", "beats": beats}).to_string(),
    )
    .expect("generated sweep")
}

#[test]
fn every_alternate_shows_the_words_the_semantic_template_shows() {
    let mut failures: BTreeSet<String> = BTreeSet::new();
    for intent in [sweep_intent(), generated_sweep()] {
        let speech = synthetic_speech(&intent);
        for tone in ["auto", "playful"] {
            for voice in [None, Some(&speech)] {
                let (plain, _) = compile_intent(&intent, voice, Ask { tone, take: None });
                let plain_beats: Vec<_> = plain
                    .scenes
                    .iter()
                    .filter(|s| s.id.starts_with("beat_"))
                    .collect();
                for take in 0..8 {
                    let (p, _) = compile_intent(
                        &intent,
                        voice,
                        Ask {
                            tone,
                            take: Some(take),
                        },
                    );
                    let beats: Vec<_> = p
                        .scenes
                        .iter()
                        .filter(|s| s.id.starts_with("beat_"))
                        .collect();
                    assert_eq!(beats.len(), intent.beats.len());
                    for (i, scene) in beats.iter().enumerate() {
                        let wanted = intent_words(&intent.beats[i]);
                        let (shown_plain, shown_alt) =
                            (shown_words(&plain, plain_beats[i]), shown_words(&p, scene));
                        let lost: Vec<&String> = wanted
                            .iter()
                            .filter(|w| is_shown(&shown_plain, w) && !is_shown(&shown_alt, w))
                            .collect();
                        if !lost.is_empty() {
                            let b = &intent.beats[i];
                            failures.insert(format!(
                                "{:?} {:?} secondary {} -> {} loses {lost:?}",
                                b.purpose,
                                b.relationship,
                                b.secondary.is_some(),
                                beats_of(&p)[i].template,
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

// ---------------------------------------------------------------------------
// Image layouts and the explore record
// ---------------------------------------------------------------------------

/// Four emphasis beats, each with a delivered subject image (the in-repo test
/// figure): TypeImageInterlock beats, which take a subject-first layout.
fn figure_story() -> (CreativeIntent, AssetManifest) {
    use serde_json::json;
    let beats: Vec<_> = (1..=4)
        .map(|n| {
            json!({
                "purpose": "emphasize", "statement": format!("A person at point {n}"),
                "narration": format!("A person stands at point {n} of the story."),
                "primary": {"kind": "phrase", "value": "Person", "meaning": "the subject"},
                "energy": "building", "keyword": "person"
            })
        })
        .collect();
    let intent = CreativeIntent::from_json(
        &json!({"version": "0.2", "title": "figures", "format": "vertical", "beats": beats})
            .to_string(),
    )
    .expect("figure story");
    let assets: Vec<_> = (1..=4)
        .map(|n| {
            json!({
                "id": format!("beat_{n}.hero_subject"), "path": "test_images/figure.png",
                "width": 600, "height": 900, "alpha": true,
                "subject_anchor": {"x": 0.5, "y": 0.5}, "face_anchor": {"x": 0.5, "y": 0.21}
            })
        })
        .collect();
    let manifest: AssetManifest =
        serde_json::from_value(json!({"version": "0.1", "assets": assets})).expect("manifest");
    (intent, manifest)
}

#[test]
fn image_layouts_rotate_by_a_seeded_sequence_and_neighbours_differ() {
    let (intent, manifest) = figure_story();
    let speech = synthetic_speech(&intent);
    let mut starts: BTreeSet<Vec<String>> = BTreeSet::new();
    for voice in [None, Some(&speech)] {
        for take in 0..8 {
            let opts = CompileOptions {
                art: Some(ArtMode::Auto),
                variety: Some(story_seed(&intent)),
                take,
                speech: voice.cloned(),
                ..CompileOptions::default()
            };
            let (p, _) = compile_with_report(
                &intent,
                &style("editorial"),
                None,
                &AssetLibrary::new(repo().join("assets")),
                &ApproxMeasure,
                &manifest,
                None,
                &opts,
            )
            .expect("compile");
            motion_core::validate::validate(&p, Some(&repo().join("assets"))).expect("valid");
            let layouts: Vec<String> = beats_of(&p)
                .iter()
                .map(|d| {
                    d.rotations
                        .get("image_layout")
                        .unwrap_or_else(|| panic!("take {take}: no image layout in {d:?}"))
                        .clone()
                })
                .collect();
            assert!(
                layouts.windows(2).all(|w| w[0] != w[1]),
                "take {take}: neighbouring image beats share a layout: {layouts:?}"
            );
            assert!(layouts.iter().all(|l| l != "legacy"), "{layouts:?}");
            starts.insert(layouts);
        }
    }
    assert!(
        starts.len() >= 3,
        "eight takes gave {} layout sequences",
        starts.len()
    );
}

#[test]
fn exploration_reports_the_template_candidates_instead_of_unavailable() {
    let intent = sweep_intent();
    let speech = synthetic_speech(&intent);
    let explore = |variety: Option<u64>, take: u64| {
        let opts = CompileOptions {
            art: Some(ArtMode::Auto),
            explore: 2,
            variety,
            take,
            speech: Some(speech.clone()),
            ..CompileOptions::default()
        };
        let p = compile_with_options(
            &intent,
            &style("editorial"),
            None,
            &AssetLibrary::new(repo().join("assets")),
            &ApproxMeasure,
            &AssetManifest::empty(),
            None,
            &opts,
        )
        .expect("compile");
        let record = p.project.exploration.clone().expect("exploration record");
        let choice = record
            .choices
            .iter()
            .find(|c| {
                c.dimension == motion_core::compiler::explore::ExploreDimension::GrammarAlternate
            })
            .expect("grammar_alternate choice")
            .clone();
        (p, choice)
    };
    // Without variety exploration keeps every beat's semantic template
    // (alternate 0) and says how many candidates each beat has.
    let (p, choice) = explore(None, 0);
    assert_ne!(choice.value, "unavailable");
    let pairs: Vec<&str> = choice.value.split(',').collect();
    assert_eq!(pairs.len(), intent.beats.len(), "{}", choice.value);
    assert!(
        pairs.iter().all(|x| x.starts_with("0/")),
        "{}",
        choice.value
    );
    // The phrase beats (1, 2, 6 of the sweep) have three candidates.
    assert!(
        pairs[0].ends_with("/3") && pairs[1].ends_with("/3"),
        "{}",
        choice.value
    );
    assert_eq!(choice.value, choice.canonical);
    assert!(p.project.direction.is_none());
    // Under a variety seed it reports the alternate the take chose.
    let (p, choice) = explore(Some(story_seed(&intent)), 2);
    let chosen: Vec<u8> = beats_of(&p).iter().map(|d| d.alternate).collect();
    let reported: Vec<u8> = choice
        .value
        .split(',')
        .map(|x| {
            x.split('/')
                .next()
                .unwrap_or_default()
                .parse()
                .expect("index")
        })
        .collect();
    assert_eq!(chosen, reported, "{}", choice.value);
}
