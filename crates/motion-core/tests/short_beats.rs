//! (0.23 A4) Short speech-led beats compile. A voice-over sentence of 1-1.5 s
//! leaves a beat only 2-3 s long; every builder must fit its motions inside it
//! (a state change used to run to 4 s, "motion ends after scene"), and the look
//! director must never put two overlapping motions on one channel of one layer
//! (the pinned layers picture under a depth look). Everything compiles with the
//! font-free `ApproxMeasure`, the way the other compiler tests do, and is then
//! validated.
//!
//! (0.23 A4-short) Section (d) is the continuous take: one voice-over take
//! leaves 0.25-0.6 s between sentences, so every structure (state change,
//! compare numbers / phrases, two pictures, collection, ranked numbers, derived
//! metric, layers, number reveal, phrase emphasize, picture) is told with
//! sentences of 1.0-2.5 s and pauses of 0.25-1.0 s on three canvases in nine
//! tones, and a state change must keep its old and new state readable.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{MotionOp, MotionProject, Scene};
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;
use serde_json::{json, Value};

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

const STRUCTURES: [&str; 6] = [
    "state_change",
    "compare",
    "collection",
    "derived_metric",
    "layers",
    "number_reveal",
];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn intent_file(path: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// The variety seed `compile --variety auto` derives from the story text
/// (FNV-1a over the title and the statements, as `motion-engine` does).
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

/// How `compile --speech` loads a speech map: repaired against the spoken lines.
fn repaired(intent: &CreativeIntent, speech: &SpeechMap) -> SpeechMap {
    let lines: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    repair(speech, &lines).0
}

fn fixture(name: &str) -> SpeechMap {
    SpeechMap::from_json(&read(&format!(
        "golden/fixtures/variety/{name}.speech.json"
    )))
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// One compile of the matrix: its name, the project and the validator's verdict.
struct Job {
    name: String,
    project: MotionProject,
    verdict: Result<(), String>,
}

/// Compile the way the product does: `--speech`, and with `product` also
/// `--art auto --variety auto` (the path the variety bench compiles), then
/// validate.
fn compile(intent: &CreativeIntent, tone: &str, speech: &SpeechMap, product: bool) -> Job {
    let project = compile_project(intent, tone, speech, product)
        .unwrap_or_else(|e| panic!("{tone}: compile: {e}"));
    let verdict = validate(&project, Some(&repo().join("assets"))).map_err(|e| e.to_string());
    Job {
        name: format!(
            "{} {tone} {}",
            intent.title,
            if product { "art+variety" } else { "speech" }
        ),
        project,
        verdict,
    }
}

/// The compiler's result for one story, tone and path (no validation).
fn compile_project(
    intent: &CreativeIntent,
    tone: &str,
    speech: &SpeechMap,
    product: bool,
) -> Result<MotionProject, String> {
    let style = StyleProfile::from_json(&json!({ "tone": tone }).to_string())
        .map_err(|e| format!("style: {e}"))?;
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        speech: Some(repaired(intent, speech)),
        art: product.then_some(ArtMode::Auto),
        variety: product.then(|| story_seed(intent)),
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .map_err(|e| e.to_string())
}

/// The compiles that do not validate, one line each.
fn failures(jobs: &[Job]) -> Vec<String> {
    jobs.iter()
        .filter_map(|j| {
            j.verdict.as_ref().err().map(|e| {
                format!(
                    "{}: {}",
                    j.name,
                    e.lines().take(3).collect::<Vec<_>>().join(" | ")
                )
            })
        })
        .collect()
}

fn assert_all_valid(jobs: &[Job]) {
    let failed = failures(jobs);
    assert!(
        failed.is_empty(),
        "{} of {} compiles failed validation:\n{}",
        failed.len(),
        jobs.len(),
        failed.join("\n")
    );
}

// ---------------------------------------------------------------------------
// (a) the committed state-change fixture
// ---------------------------------------------------------------------------

#[test]
fn state_change_with_its_speech_fixture_compiles_in_every_tone() {
    let intent = intent_file("examples/public/state-change.intent.json");
    let speech = fixture("state-change");
    let jobs: Vec<Job> = TONES
        .iter()
        .flat_map(|tone| [false, true].map(|product| compile(&intent, tone, &speech, product)))
        .collect();
    assert_eq!(jobs.len(), 18);
    assert_all_valid(&jobs);
}

// ---------------------------------------------------------------------------
// (b) synthetic short stories, every structure
// ---------------------------------------------------------------------------

/// Beat `i` of a story told with `structure`: three variants crossed with the
/// three energies (a story of nine beats has every pair once).
fn beat_json(structure: &str, i: usize) -> Value {
    let energy = ["building", "impact", "calm"][(i / 3) % 3];
    match structure {
        "state_change" => {
            let (statement, primary, secondary) = match i % 3 {
                0 => (
                    "The new line cut the trip.",
                    json!({"kind":"state_change","entity":"trip time","from":"4 hours","to":"90 minutes","meaning":"faster"}),
                    None,
                ),
                1 => (
                    "Faster and cheaper at once.",
                    json!({"kind":"state_change","entity":"speed","from":"slow","to":"fast"}),
                    Some(
                        json!({"kind":"state_change","entity":"ticket price","from":"high","to":"low"}),
                    ),
                ),
                _ => (
                    "Then the queue vanished.",
                    json!({"kind":"state_change","entity":"queue","from":"long","to":"gone","meaning":"no wait"}),
                    None,
                ),
            };
            let mut b = json!({"purpose":"emphasize","statement":statement,"primary":primary,"energy":energy});
            if let Some(secondary) = secondary {
                b["secondary"] = secondary;
                b["purpose"] = json!("contrast");
            }
            b
        }
        "compare" => match i % 3 {
            0 => {
                json!({"purpose":"contrast","statement":"The queue grew fourfold.","primary":{"kind":"number","value":"5","meaning":"people at opening"},"secondary":{"kind":"number","value":"20","meaning":"people at noon"},"relationship":"grow","energy":energy})
            }
            1 => {
                json!({"purpose":"compare","statement":"Old way against the new way.","primary":{"kind":"phrase","value":"old way"},"secondary":{"kind":"phrase","value":"new way"},"energy":energy})
            }
            _ => {
                json!({"purpose":"compare","statement":"Cheaper than the bus.","primary":{"kind":"number","value":"$4","meaning":"by train"},"secondary":{"kind":"number","value":"$9","meaning":"by bus"},"energy":energy})
            }
        },
        "collection" => match i % 3 {
            0 => {
                json!({"purpose":"emphasize","statement":"Small things fill the bag.","primary":{"kind":"collection","meaning":"things packed","items":[{"kind":"phrase","value":"Water bottle"},{"kind":"phrase","value":"Jacket"},{"kind":"phrase","value":"Laptop"},{"kind":"phrase","value":"Books"}]},"secondary":{"kind":"number","value":"9 kg","meaning":"on your back"},"relationship":"accumulate","energy":energy,"keyword":"weight"})
            }
            1 => {
                json!({"purpose":"emphasize","statement":"Three tools do it all.","primary":{"kind":"collection","meaning":"tools","items":[{"kind":"phrase","value":"Hammer"},{"kind":"phrase","value":"Saw"},{"kind":"phrase","value":"Drill"}]},"energy":energy})
            }
            _ => {
                json!({"purpose":"explain","statement":"Five habits stack up.","primary":{"kind":"collection","meaning":"habits","items":[{"kind":"phrase","value":"Read"},{"kind":"phrase","value":"Walk"},{"kind":"phrase","value":"Write"},{"kind":"phrase","value":"Sleep"},{"kind":"phrase","value":"Save"}]},"energy":energy})
            }
        },
        "derived_metric" => match i % 3 {
            0 => {
                json!({"purpose":"compare","statement":"The small class did better.","primary":{"kind":"derived_metric","meaning":"pass rate","numerator":{"value":540,"meaning":"passed"},"denominator":{"value":600,"meaning":"students"}},"secondary":{"kind":"derived_metric","meaning":"pass rate","numerator":{"value":190,"meaning":"passed"},"denominator":{"value":200,"meaning":"students"}},"energy":energy,"keyword":"rate"})
            }
            1 => {
                json!({"purpose":"explain","statement":"Most of them passed.","primary":{"kind":"derived_metric","meaning":"pass rate","numerator":{"value":540,"meaning":"passed"},"denominator":{"value":600,"meaning":"students"}},"energy":energy})
            }
            _ => {
                json!({"purpose":"explain","statement":"Nearly all of them did.","primary":{"kind":"derived_metric","meaning":"pass rate","numerator":{"value":190,"meaning":"passed"},"denominator":{"value":200,"meaning":"students"}},"energy":energy})
            }
        },
        "layers" => {
            let focus = ["Troposphere", "Tropopause", "Stratosphere"][i % 3];
            let statement = [
                "Weather lives low",
                "A lid on the weather",
                "Calm air above",
            ][i % 3];
            let secondary = match i % 3 {
                0 => json!({"kind":"number","value":"12 km","meaning":"deep"}),
                1 => json!({"kind":"phrase","value":"temperature stops falling"}),
                _ => json!({"kind":"number","value":"50 km","meaning":"top of the layer"}),
            };
            json!({"purpose":"emphasize","statement":statement,"primary":{"kind":"layers","meaning":"the atmosphere","layers":[{"name":"Troposphere","note":"weather, clouds"},{"name":"Tropopause","note":"the lid","boundary":true},{"name":"Stratosphere","note":"ozone, calm air"}],"focus":focus},"secondary":secondary,"energy":energy})
        }
        _ => {
            let (statement, value, meaning) = [
                ("Almost seven times faster.", "6.7x", "faster crossing"),
                ("Half of them quit.", "50%", "quit"),
                ("Three million downloads.", "3 million", "downloads"),
            ][i % 3];
            json!({"purpose":"reveal","statement":statement,"primary":{"kind":"number","value":value,"meaning":meaning},"energy":energy})
        }
    }
}

fn story(structure: &str, beats: usize) -> CreativeIntent {
    let list: Vec<Value> = (0..beats).map(|i| beat_json(structure, i)).collect();
    let v = json!({"version":"0.2","title":format!("short_{structure}"),"format":"vertical","beats":list});
    CreativeIntent::from_json(&v.to_string()).expect("intent")
}

/// One sentence per beat, `lens` seconds long (cycled), the first at 0.35 s;
/// the spoken words are spread evenly over the sentence and `gaps` (cycled)
/// are the silences after each sentence.
fn synthetic_speech(intent: &CreativeIntent, lens: &[f64], gaps: &[f64]) -> SpeechMap {
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let mut t = 0.35;
    for (beat, b) in intent.beats.iter().enumerate() {
        let line = display_text(b, true).spoken;
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let len = lens[beat % lens.len()];
        let per = len / tokens.len().max(1) as f64;
        for (k, token) in tokens.iter().enumerate() {
            words.push(SpeechWord {
                text: token.to_string(),
                start: t + per * k as f64,
                end: t + per * (k + 1) as f64,
                confidence: 1.0,
            });
        }
        sentences.push(SpeechSentence {
            beat,
            start: t,
            end: t + len,
        });
        t += len + gaps[beat % gaps.len()];
    }
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: t + 1.0,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
        recognised: Vec::new(),
        alignment: None,
    }
}

/// Sentences of 1.0, 1.25 and 1.5 s, each followed by the pause the committed
/// speech fixtures show after a sentence that short (`state-change` 1.32 s,
/// `layers` 1.24 s and 1.32 s): beats of 2.4-3.4 s with 0.4-0.7 s of overlap.
const SENTENCES: [f64; 3] = [1.0, 1.25, 1.5];
const PAUSES: [f64; 2] = [1.24, 1.32];

#[test]
fn short_sentences_compile_in_every_tone_for_every_structure() {
    let mut jobs = Vec::new();
    for structure in STRUCTURES {
        let intent = story(structure, 9);
        let speech = synthetic_speech(&intent, &SENTENCES, &PAUSES);
        for tone in TONES {
            for product in [false, true] {
                jobs.push(compile(&intent, tone, &speech, product));
            }
        }
    }
    assert_eq!(jobs.len(), 6 * 9 * 2);
    assert_all_valid(&jobs);
}

// ---------------------------------------------------------------------------
// (c) layers under the depth looks
// ---------------------------------------------------------------------------

/// Motions that overlap another on the same layer and channel (the validator's
/// rule, restated so the test does not only rely on it).
fn overlapping_motions(p: &MotionProject) -> Vec<String> {
    let mut out = Vec::new();
    for scene in &p.scenes {
        let mut by_key: BTreeMap<(String, String), Vec<(f64, f64)>> = BTreeMap::new();
        for m in &scene.motions {
            by_key
                .entry((m.target.clone(), format!("{:?}", m.op.channel())))
                .or_default()
                .push((m.start, m.start + m.duration));
        }
        for ((layer, channel), mut spans) in by_key {
            spans.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            for pair in spans.windows(2) {
                if pair[1].0 < pair[0].1 - 1e-9 && pair[1].1 - pair[1].0 > 1e-9 {
                    out.push(format!(
                        "{}: {layer} {channel} {:.3}..{:.3} overlaps {:.3}..{:.3}",
                        scene.id, pair[0].0, pair[0].1, pair[1].0, pair[1].1
                    ));
                }
            }
        }
    }
    out
}

#[test]
fn layers_with_its_speech_fixture_compile_under_every_look_without_stacked_motions() {
    let intent = intent_file("examples/public/layers.intent.json");
    let speech = fixture("layers");
    let jobs: Vec<Job> = TONES
        .iter()
        .map(|tone| compile(&intent, tone, &speech, true))
        .collect();
    assert_all_valid(&jobs);
    for tone in ["documentary", "cinematic"] {
        assert!(jobs.iter().any(|j| j.name.contains(tone)), "{tone}");
    }
    for job in &jobs {
        let stacked = overlapping_motions(&job.project);
        assert!(stacked.is_empty(), "{}: {}", job.name, stacked.join("\n"));
    }
}

#[test]
fn layers_beats_of_2_to_3_seconds_stack_nothing_under_any_look() {
    // Nine layers beats: the pinned picture's 0.9 s arrival runs into the
    // stage's ANTICIPATE lift on several of them (the cinematic look moves the
    // arrival onto the picture's depth wrapper, which carries that lift).
    let intent = story("layers", 9);
    let speech = synthetic_speech(&intent, &SENTENCES, &PAUSES);
    for tone in TONES {
        let job = compile(&intent, tone, &speech, true);
        assert_eq!(job.verdict, Ok(()), "{}", job.name);
        let stacked = overlapping_motions(&job.project);
        assert!(stacked.is_empty(), "{}: {}", job.name, stacked.join("\n"));
    }
}

// ---------------------------------------------------------------------------
// the fixed state-change beat still reads
// ---------------------------------------------------------------------------

/// How long the first row's old state and new state stay readable, from their
/// motions in the compiled scene: `(old: landed to replaced, new: landed to the
/// exit, new: landed to ANTICIPATE)`, each the worst unit of the state. The
/// replace begins with a dip (a move from rest to a positive y); the old state
/// has landed when its last entrance motion has ended, the new one when the
/// last motion on its units has.
fn readable(scene: &Scene, prefix: &str) -> (f64, f64, f64) {
    let life = scene.lifecycle.expect("lifecycle");
    let motions_of = |id: &str| -> Vec<(f64, f64, &MotionOp)> {
        scene
            .motions
            .iter()
            .filter(|m| m.target == id)
            .map(|m| (m.start, m.start + m.duration, &m.op))
            .collect()
    };
    let units = |tag: &str| -> Vec<String> {
        let mut ids: Vec<String> = scene
            .motions
            .iter()
            .filter(|m| m.target.starts_with(&format!("{prefix}.{tag}.")))
            .map(|m| m.target.clone())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let (mut old, mut new_exit, mut new_ant) = (f64::MAX, f64::MAX, f64::MAX);
    for id in units("from") {
        let motions = motions_of(&id);
        let replaced = motions
            .iter()
            .filter_map(|(s, _, op)| match op {
                MotionOp::Move { from, to } if *from == [0.0, 0.0] && to[1] > 0.0 => Some(*s),
                _ => None,
            })
            .fold(f64::MAX, f64::min);
        // An echo trail (street, hype) only repeats the word behind itself; the
        // word is readable once its own entrance has ended.
        let landed = motions
            .iter()
            .filter(|(s, _, op)| *s < replaced && !matches!(op, MotionOp::Echo { .. }))
            .map(|(_, e, _)| *e)
            .fold(0.0, f64::max);
        old = old.min(replaced - landed);
    }
    for id in units("to") {
        let landed = motions_of(&id)
            .iter()
            .map(|(_, e, _)| *e)
            .fold(0.0, f64::max);
        new_exit = new_exit.min(life.bridge - landed);
        new_ant = new_ant.min(life.anticipate - landed);
    }
    (old, new_exit, new_ant)
}

/// The tones whose relaxed layout of the fixture's first beat ended after the
/// beat before 0.23 A4 (the compile failed validation); they now use the
/// compact layout. In the other three tones (editorial, technical, documentary)
/// the compile always passed - `clamp_to_scene` shortens what overruns - so
/// without `--variety` its output must stay as it was (sprint 0.23 fix policy)
/// and is not asserted; on the product path (0.23 A4-short) they use the
/// compact layout too, where the relaxed one would show the new state late.
const COMPACT_TONES: [&str; 6] = ["auto", "playful", "street", "hype", "studio", "cinematic"];

#[test]
fn the_fixed_state_change_beat_keeps_from_and_to_readable() {
    let intent = intent_file("examples/public/state-change.intent.json");
    let speech = fixture("state-change");
    for tone in TONES {
        for product in [false, true] {
            // Without the direction seed the other three tones keep what they
            // built (fix policy); on the product path every tone is compact
            // where the relaxed layout would clamp the new state.
            if !product && !COMPACT_TONES.contains(&tone) {
                continue;
            }
            let job = compile(&intent, tone, &speech, product);
            assert_eq!(job.verdict, Ok(()), "{}", job.name);
            let scene = job
                .project
                .scenes
                .iter()
                .find(|s| s.id == "beat_1")
                .expect("beat_1");
            let (old, new_exit, new_ant) = readable(scene, "b1.sc0");
            eprintln!(
                "{}: old state readable {old:.3} s, new state {new_exit:.3} s to the exit, {new_ant:.3} s to ANTICIPATE",
                job.name
            );
            assert!(old >= 0.6, "{}: old state readable {old:.3} s", job.name);
            assert!(
                new_exit >= 0.6,
                "{}: new state readable {new_exit:.3} s before the exit",
                job.name
            );
            // (review note on A4-compile) Nothing of the row begins once the
            // beat anticipates its exit.
            let anticipate = scene.lifecycle.expect("lifecycle").anticipate;
            let late: Vec<String> = scene
                .motions
                .iter()
                .filter(|m| m.target.starts_with("b1.sc0") && m.start >= anticipate - 1e-9)
                .map(|m| format!("{} at {:.3}", m.target, m.start))
                .collect();
            assert!(
                late.is_empty(),
                "{}: b1.sc0 motions start at or after ANTICIPATE ({anticipate:.3}): {}",
                job.name,
                late.join(", ")
            );
        }
    }
}

/// The rows (`sc0`, and `sc1` of a dual beat) of a state-change beat scene.
fn rows_of(scene: &Scene, beat: usize) -> Vec<String> {
    (0..2)
        .map(|i| format!("b{beat}.sc{i}"))
        .filter(|p| {
            scene
                .motions
                .iter()
                .any(|m| m.target.starts_with(&format!("{p}.")))
        })
        .collect()
}

#[test]
fn a_state_change_on_a_continuous_take_keeps_from_and_to_readable_in_every_tone() {
    // Sentences of 1.5 s with 0.25 s between them, the way a narration is read
    // in one take: each beat has about 1.3-1.5 s between its ENTER and its exit.
    let intent = story("state_change", 3);
    let speech = synthetic_speech(&intent, &[1.5], &[0.25]);
    let (mut table, mut short) = (Vec::new(), Vec::new());
    for tone in TONES {
        let job = compile(&intent, tone, &speech, true);
        assert_eq!(job.verdict, Ok(()), "{}", job.name);
        let mut least = (f64::MAX, f64::MAX, f64::MAX);
        for (i, scene) in job
            .project
            .scenes
            .iter()
            .filter(|s| s.id.starts_with("beat_"))
            .enumerate()
        {
            let rows = rows_of(scene, i + 1);
            assert!(!rows.is_empty(), "{}: beat {} has no row", job.name, i + 1);
            for row in rows {
                let (old, new_exit, new_ant) = readable(scene, &row);
                least = (
                    least.0.min(old),
                    least.1.min(new_exit),
                    least.2.min(new_ant),
                );
                if old < 0.4 || new_exit < 0.4 {
                    short.push(format!(
                        "{tone} {row}: old state {old:.3} s, new state {new_exit:.3} s \
                         before the exit ({new_ant:.3} s before ANTICIPATE)"
                    ));
                }
            }
        }
        table.push(format!(
            "{tone}: from {:.3} s, to {:.3} s to the exit ({:.3} s to ANTICIPATE)",
            least.0, least.1, least.2
        ));
    }
    eprintln!(
        "state change, sentences 1.5 s, pauses 0.25 s, least over the rows:\n{}",
        table.join("\n")
    );
    assert!(short.is_empty(), "under 0.4 s:\n{}", short.join("\n"));
}

// ---------------------------------------------------------------------------
// (d) the continuous take: every structure, short sentences, short pauses
// ---------------------------------------------------------------------------
//
// The owner's voice rule is one continuous take: 0.25-0.6 s between sentences
// (the bench's `say` fixtures leave 1.2-1.3 s). Each structure is told with
// sentences of 1.0 / 1.5 / 2.0 / 2.5 s and pauses of 0.25 / 0.4 / 0.6 / 1.0 s
// on three canvases in nine tones, the way the product compiles
// (`--art auto --variety auto`), and every compile must validate.

const MATRIX: [&str; 11] = [
    "state_change",
    "compare_numbers",
    "compare_phrases",
    "two_pictures",
    "collection",
    "ranked_numbers",
    "derived_metric",
    "layers",
    "number_reveal",
    "phrase_emphasize",
    "picture",
];
const MATRIX_SENTENCES: [f64; 4] = [1.0, 1.5, 2.0, 2.5];
const MATRIX_PAUSES: [f64; 4] = [0.25, 0.4, 0.6, 1.0];
const CANVASES: [&str; 3] = ["vertical", "square", "landscape"];

/// Variant `v` (0-2) of a structure at energy `e` (0-2), as `beat_json`'s
/// virtual index `v + 3 e`. The six original structures are `beat_json`'s.
fn matrix_beat(structure: &str, v: usize, e: usize) -> Value {
    let energy = ["building", "impact", "calm"][e % 3];
    match structure {
        "compare_numbers" => match v % 3 {
            0 => {
                json!({"purpose":"contrast","statement":"The queue grew fourfold.","primary":{"kind":"number","value":"5","meaning":"people at opening"},"secondary":{"kind":"number","value":"20","meaning":"people at noon"},"relationship":"grow","energy":energy})
            }
            1 => {
                json!({"purpose":"compare","statement":"Cheaper than the bus.","primary":{"kind":"number","value":"$4","meaning":"by train"},"secondary":{"kind":"number","value":"$9","meaning":"by bus"},"energy":energy})
            }
            _ => {
                json!({"purpose":"contrast","statement":"Fewer people quit.","primary":{"kind":"number","value":"12%","meaning":"quit last year"},"secondary":{"kind":"number","value":"7%","meaning":"quit this year"},"relationship":"compress","energy":energy})
            }
        },
        "compare_phrases" => match v % 3 {
            0 => {
                json!({"purpose":"compare","statement":"Old way against the new way.","primary":{"kind":"phrase","value":"old way"},"secondary":{"kind":"phrase","value":"new way"},"energy":energy})
            }
            1 => {
                json!({"purpose":"contrast","statement":"Talk is cheap, proof is not.","primary":{"kind":"phrase","value":"Talk","meaning":"what people say"},"secondary":{"kind":"phrase","value":"Proof","meaning":"what people show"},"energy":energy})
            }
            _ => {
                json!({"purpose":"compare","statement":"Slow and steady beats fast.","primary":{"kind":"phrase","value":"slow and steady"},"secondary":{"kind":"phrase","value":"fast and loose"},"keyword":"steady","energy":energy})
            }
        },
        "two_pictures" => match v % 3 {
            0 => {
                json!({"purpose":"compare","statement":"A model against a mind.","primary":{"kind":"object","asset":"robot","meaning":"a model"},"secondary":{"kind":"object","asset":"brain_circuit","meaning":"a mind"},"energy":energy})
            }
            1 => {
                json!({"purpose":"contrast","statement":"Who guesses better.","primary":{"kind":"object","asset":"robot","value":"4.1%","meaning":"errors"},"secondary":{"kind":"object","asset":"brain_circuit","value":"3.4%","meaning":"errors"},"energy":energy})
            }
            _ => {
                json!({"purpose":"compare","statement":"Chat beats the network.","primary":{"kind":"object","asset":"chat_bubbles","meaning":"chat"},"secondary":{"kind":"number","value":"86","meaning":"billion links"},"energy":energy})
            }
        },
        "ranked_numbers" => match v % 3 {
            0 => {
                json!({"purpose":"explain","statement":"Three models, ranked.","primary":{"kind":"collection","meaning":"error rate","items":[{"kind":"number","value":"4.1%","meaning":"robots"},{"kind":"number","value":"3.4%","meaning":"brains"},{"kind":"number","value":"3.1%","meaning":"atoms"}]},"energy":energy})
            }
            1 => {
                json!({"purpose":"explain","statement":"Who scores highest.","primary":{"kind":"collection","meaning":"score","items":[{"kind":"object","asset":"robot","value":"92"},{"kind":"object","asset":"brain_circuit","value":"71"},{"kind":"object","asset":"neural_network","value":"64"}]},"energy":energy})
            }
            _ => {
                json!({"purpose":"explain","statement":"Four cities by size.","primary":{"kind":"collection","meaning":"people","items":[{"kind":"number","value":"9 million","meaning":"Alpha"},{"kind":"number","value":"7 million","meaning":"Beta"},{"kind":"number","value":"4 million","meaning":"Gamma"},{"kind":"number","value":"2 million","meaning":"Delta"}]},"energy":energy})
            }
        },
        "phrase_emphasize" => match v % 3 {
            0 => {
                json!({"purpose":"emphasize","statement":"It reads. A lot.","primary":{"kind":"phrase","value":"Billions of sentences","meaning":"training data"},"energy":energy})
            }
            1 => {
                json!({"purpose":"emphasize","statement":"Patterns, not facts.","primary":{"kind":"phrase","value":"Patterns"},"keyword":"patterns","energy":energy})
            }
            _ => {
                json!({"purpose":"explain","statement":"Guess the next word.","primary":{"kind":"phrase","value":"Guess the next word","meaning":"its only job"},"secondary":{"kind":"phrase","value":"again and again"},"energy":energy})
            }
        },
        "picture" => match v % 3 {
            0 => {
                json!({"purpose":"emphasize","statement":"How AI actually learns.","primary":{"kind":"object","asset":"robot","meaning":"a blank model"},"keyword":"learns","energy":energy})
            }
            1 => {
                json!({"purpose":"reveal","statement":"Patterns, not facts.","primary":{"kind":"object","asset":"neural_network","meaning":"patterns"},"energy":energy})
            }
            _ => {
                json!({"purpose":"reveal","statement":"Why it sounds human.","primary":{"kind":"object","asset":"chat_bubbles","value":"86 billion","meaning":"links"},"energy":energy})
            }
        },
        other => beat_json(other, v + 3 * e),
    }
}

/// A story of 16 beats: beat `i` has variant `(i + shift) % 3`, so three shifts
/// cover every (variant, sentence, pause) once.
fn matrix_story(structure: &str, canvas: &str, shift: usize) -> CreativeIntent {
    let list: Vec<Value> = (0..16)
        .map(|i| matrix_beat(structure, (i + shift) % 3, (i / 3) % 3))
        .collect();
    let v = json!({
        "version": "0.2",
        "title": format!("cont_{structure}_{canvas}_{shift}"),
        "format": canvas,
        "beats": list
    });
    CreativeIntent::from_json(&v.to_string()).expect("intent")
}

/// Beat `i` is spoken for `MATRIX_SENTENCES[i % 4]` and followed by a pause of
/// `MATRIX_PAUSES[i / 4]`: the 16 beats of a story cover every pair once.
fn matrix_speech(intent: &CreativeIntent) -> SpeechMap {
    let gaps: Vec<f64> = (0..16).map(|i| MATRIX_PAUSES[i / 4]).collect();
    synthetic_speech(intent, &MATRIX_SENTENCES, &gaps)
}

/// Compile one structure on `canvases`, in every tone and with each of the
/// three variant shifts, the product way (`--art auto --variety auto`) or with
/// the speech alone, and report every compile that does not validate.
fn continuous_take_on(structure: &str, canvases: &[&str], product: bool) {
    let (mut failed, mut total) = (Vec::new(), 0);
    for canvas in canvases {
        for shift in 0..3 {
            let intent = matrix_story(structure, canvas, shift);
            let speech = matrix_speech(&intent);
            for tone in TONES {
                total += 1;
                let verdict = compile_project(&intent, tone, &speech, product).and_then(|p| {
                    validate(&p, Some(&repo().join("assets"))).map_err(|e| e.to_string())
                });
                if let Err(e) = verdict {
                    failed.push(format!(
                        "{} {tone}: {}",
                        intent.title,
                        e.lines().take(3).collect::<Vec<_>>().join(" | ")
                    ));
                }
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{structure}: {} of {total} compiles failed validation:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// The product path on every canvas (acceptance matrix: 11 structures x 4
/// sentences x 4 pauses x 9 tones x 3 canvases, 16 beats a story).
fn continuous_take(structure: &str) {
    continuous_take_on(structure, &CANVASES, true);
}

#[test]
fn continuous_take_state_change() {
    continuous_take("state_change");
}

#[test]
fn continuous_take_compare_numbers() {
    continuous_take("compare_numbers");
}

#[test]
fn continuous_take_compare_phrases() {
    continuous_take("compare_phrases");
}

#[test]
fn continuous_take_two_pictures() {
    continuous_take("two_pictures");
}

#[test]
fn continuous_take_collection() {
    continuous_take("collection");
}

#[test]
fn continuous_take_ranked_numbers() {
    continuous_take("ranked_numbers");
}

#[test]
fn continuous_take_derived_metric() {
    continuous_take("derived_metric");
}

#[test]
fn continuous_take_layers() {
    continuous_take("layers");
}

#[test]
fn continuous_take_number_reveal() {
    continuous_take("number_reveal");
}

#[test]
fn continuous_take_phrase_emphasize() {
    continuous_take("phrase_emphasize");
}

#[test]
fn continuous_take_picture() {
    continuous_take("picture");
}

#[test]
fn continuous_take_without_variety_compiles_every_structure() {
    // Without `--variety` the same stories must validate too (the fixes that
    // reach them only change a compile that would not validate).
    for structure in MATRIX {
        continuous_take_on(structure, &["vertical"], false);
    }
}

#[test]
fn the_matrix_names_every_structure() {
    assert_eq!(MATRIX.len(), 11);
    for structure in MATRIX {
        let intent = matrix_story(structure, "vertical", 0);
        assert_eq!(intent.beats.len(), 16, "{structure}");
    }
}
