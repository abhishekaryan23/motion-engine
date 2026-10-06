//! (0.23 A5b) The hard-QA findings outside studio / hype that Gate A left on
//! the benchmark (layers, documentary, the evidence stack's note card, the
//! cinematic hero word) and the root fixes behind them (the collection's
//! arrival chain, the stat pair's seeded arrival, the split's second panel),
//! plus the picture-heavy repro cases of `habit_math` (stamps inside the safe
//! area, documentary cards that hug what is drawn, a cinematic `grow` that
//! draws its arrow).
//!
//! Everything compiles with the font-free `ApproxMeasure` the way the other
//! compiler tests do; the layout QA is `layout_report(_with)` (structural, with
//! an `ImageIndex` built from the library catalogs' own analysis and the mean
//! colours the pixel QA measured). The rendered `text_local_contrast` check is
//! the bench's (motion-render); here its causes are asserted structurally.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::assets::{AssetManifest, NormBox};
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::direction::{seeded_sequence, Dim};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning,
    WARN_VALUE_DROPPED,
};
use motion_core::easing::Easing;
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::{layout_report, layout_report_with, LayoutCheck, LayoutFinding};
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::style::StyleProfile;
use motion_core::subject_qa::{ImageFacts, ImageIndex};
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

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn intent_file(path: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn intent_json(v: &Value) -> CreativeIntent {
    CreativeIntent::from_json(&v.to_string()).expect("intent")
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

fn fixture(name: &str, tight: bool) -> SpeechMap {
    let dir = if tight {
        "golden/fixtures/variety/tight"
    } else {
        "golden/fixtures/variety"
    };
    SpeechMap::from_json(&read(&format!("{dir}/{name}.speech.json")))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// One sentence per beat (its spoken line), `lens` seconds long (cycled), the
/// first at 0.35 s; the words are spread evenly over the sentence and `gaps`
/// (cycled) are the silences after each.
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

/// What a compile is asked for.
#[derive(Clone)]
struct Ask {
    tone: &'static str,
    speech: Option<SpeechMap>,
    /// `--art auto`.
    art: bool,
    /// `--variety auto` (the product path): the story's seed.
    seeded: bool,
    families: Vec<&'static str>,
}

impl Ask {
    fn new(tone: &'static str) -> Self {
        Ask {
            tone,
            speech: None,
            art: false,
            seeded: false,
            families: Vec::new(),
        }
    }
    fn product(tone: &'static str, speech: Option<SpeechMap>) -> Self {
        Ask {
            tone,
            speech,
            art: true,
            seeded: true,
            families: Vec::new(),
        }
    }
    fn families(mut self, f: &[&'static str]) -> Self {
        self.families = f.to_vec();
        self
    }
}

fn compile(intent: &CreativeIntent, ask: &Ask) -> (MotionProject, Vec<CompileWarning>) {
    let style: StyleProfile =
        serde_json::from_str(&json!({ "tone": ask.tone }).to_string()).expect("style");
    let library = AssetLibrary::new(repo().join("assets"))
        .with_families(ask.families.iter().map(|f| f.to_string()).collect());
    let opts = CompileOptions {
        speech: ask.speech.as_ref().map(|s| repaired(intent, s)),
        art: ask.art.then_some(ArtMode::Auto),
        variety: ask.seeded.then(|| story_seed(intent)),
        ..CompileOptions::default()
    };
    let (project, warnings) = compile_with_report(
        intent,
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap_or_else(|e| panic!("{} ({}): compile: {e}", intent.title, ask.tone));
    (project, warnings)
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene {id}"))
}

fn find<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    layers.iter().find_map(|l| {
        if l.id == id {
            return Some(l);
        }
        match &l.kind {
            LayerKind::Group { children } => find(children, id),
            _ => None,
        }
    })
}

/// Every layer of `layers` (any depth) whose id contains `part`.
fn find_all<'a>(layers: &'a [Layer], part: &str, out: &mut Vec<&'a Layer>) {
    for l in layers {
        if l.id.contains(part) {
            out.push(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            find_all(children, part, out);
        }
    }
}

fn on<'a>(s: &'a Scene, target: &str) -> Vec<&'a Motion> {
    s.motions.iter().filter(|m| m.target == target).collect()
}

/// When the entrance fade `m` has its layer at half opacity.
fn half_opacity_at(m: &Motion) -> Option<f64> {
    let MotionOp::Fade { from, to } = m.op else {
        return None;
    };
    if from >= 0.5 || to < 0.5 {
        return None;
    }
    let want = f64::from((0.5 - from) / (to - from));
    let ease = |u: f64| {
        if m.spring.is_some() {
            Easing::OutQuint.apply(u)
        } else {
            m.easing.apply(u)
        }
    };
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if ease(mid) >= want {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    Some(m.start + hi * m.duration)
}

/// The layout QA on a compiled project.
fn layout(p: &MotionProject, index: &ImageIndex) -> Vec<LayoutFinding> {
    let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
    layout_report_with(p, &frame, index).findings
}

fn findings_of(found: &[LayoutFinding], check: LayoutCheck) -> Vec<String> {
    found
        .iter()
        .filter(|f| f.check == check)
        .map(|f| format!("{}: {} ({})", f.scene, f.layer, f.detail))
        .collect()
}

// ---------------------------------------------------------------------------
// An image index from what the catalogs and the pixel QA know
// ---------------------------------------------------------------------------

/// Mean colours (alpha-weighted, as the layout QA's analysis measures them) of
/// the library pictures these stories show.
const MEANS: [(&str, [u8; 3]); 9] = [
    ("server_rack", [0x32, 0x35, 0x33]),
    ("speech_bubbles", [0x1D, 0x1B, 0x19]),
    ("satellite_dish", [0xCB, 0xCD, 0xC8]),
    ("telescope", [0x99, 0x9B, 0x94]),
    ("brain_model", [0xA4, 0x93, 0x77]),
    ("atom_model", [0x89, 0x80, 0x62]),
    ("globe", [0x84, 0x83, 0x5E]),
    ("piggy_bank", [0x89, 0x9E, 0x8E]),
    ("shopping_bag", [0xD8, 0xAB, 0x32]),
];

/// Facts for every still of the families' catalogs (alpha bounds from the
/// catalog's analysis, the mean colour where `MEANS` knows it), keyed by the
/// path a compiled project draws it by, plus the first frame of the sprite
/// loops that replace a still (`library/<family>/loops/<name>`).
fn catalog_index(families: &[&str], sprites: &[(&str, &str, u32, u32, [f32; 4])]) -> ImageIndex {
    let mut index = ImageIndex::new();
    for family in families {
        let path = repo().join(format!("assets/library/{family}/catalog.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let catalog: Value = serde_json::from_str(&text).expect("catalog");
        for a in catalog["assets"].as_array().into_iter().flatten() {
            let (Some(id), Some(file)) = (a["id"].as_str(), a["file"].as_str()) else {
                continue;
            };
            let bounds = &a["analysis"]["subject_bounds"];
            let f = |k: &str| bounds[k].as_f64().unwrap_or(0.0) as f32;
            let subject = if bounds.is_object() {
                NormBox {
                    x: f("x"),
                    y: f("y"),
                    width: f("width"),
                    height: f("height"),
                }
            } else {
                NormBox {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 1.0,
                }
            };
            index.insert(
                format!("library/{family}/{file}"),
                ImageFacts {
                    width: a["width"].as_u64().unwrap_or(0) as u32,
                    height: a["height"].as_u64().unwrap_or(0) as u32,
                    alpha: a["alpha"].as_bool().unwrap_or(true),
                    subject,
                    head: None,
                    mean_color: MEANS.iter().find(|(n, _)| *n == id).map(|(_, c)| *c),
                },
            );
        }
    }
    for (family, name, w, h, b) in sprites {
        index.insert(
            format!("library/{family}/loops/{name}"),
            ImageFacts {
                width: *w,
                height: *h,
                alpha: true,
                subject: NormBox {
                    x: b[0],
                    y: b[1],
                    width: b[2],
                    height: b[3],
                },
                head: None,
                mean_color: MEANS.iter().find(|(n, _)| n == name).map(|(_, c)| *c),
            },
        );
    }
    index
}

// ---------------------------------------------------------------------------
// 1. Collection: the aggregate lands with room before ANTICIPATE
// ---------------------------------------------------------------------------

/// Seconds the aggregate must be at half opacity before ANTICIPATE.
const AGGREGATE_ROOM: f64 = 0.6;

fn dropped(w: &[CompileWarning]) -> Vec<String> {
    w.iter()
        .filter(|w| w.code == WARN_VALUE_DROPPED)
        .map(|w| format!("beat {}: {}", w.beat.map_or(0, |b| b + 1), w.message))
        .collect()
}

#[test]
fn the_collection_aggregate_lands_with_room_before_anticipate_on_its_own() {
    let intent = intent_file("examples/public/collection-accumulate.intent.json");
    for tight in [false, true] {
        let speech = fixture("collection-accumulate", tight);
        for tone in TONES {
            let (p, w) = compile(&intent, &Ask::product(tone, Some(speech.clone())));
            let s = scene(&p, "beat_1");
            let Some(life) = s.lifecycle else {
                panic!("no lifecycle")
            };
            // A look that builds the picture instead of the slab has no aggregate.
            if find(&s.layers, "b1.aggregate").is_none() {
                continue;
            }
            let entrance = on(s, "b1.aggregate")
                .into_iter()
                .filter_map(half_opacity_at)
                .fold(f64::MAX, f64::min);
            assert!(
                life.anticipate - entrance >= AGGREGATE_ROOM - 1e-6,
                "{tone} (tight {tight}): the aggregate is readable {:.3} s before ANTICIPATE \
                 (at {entrance:.3}, ANTICIPATE {:.3})",
                life.anticipate - entrance,
                life.anticipate
            );
            // So the late-text pass of the product path has nothing to move
            // (it only moves a text readable less than 0.18 s before).
            assert!(entrance <= life.anticipate - 0.18);
            assert!(dropped(&w).is_empty(), "{tone}: {:?}", dropped(&w));
            // The items still arrive in order and the aggregate after the last.
            let starts: Vec<f64> = (0..4)
                .map(|i| {
                    on(s, &format!("b1.items.{i}.card"))
                        .iter()
                        .map(|m| m.start)
                        .fold(f64::MAX, f64::min)
                })
                .collect();
            assert!(starts.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
            let slab = on(s, "b1.aggregate.slab")
                .iter()
                .map(|m| m.start)
                .fold(f64::MAX, f64::min);
            assert!(slab > starts[3], "{slab} after {starts:?}");
            // And the compile is valid.
            validate(&p, Some(&repo().join("assets"))).unwrap_or_else(|e| panic!("{tone}: {e}"));
        }
    }
}

#[test]
fn without_a_seed_only_a_chain_that_drops_its_value_changes() {
    // The 3 s beat of the fixture drops "9 kg" (value_dropped) in the old
    // chain: the new one shows it.
    let intent = intent_file("examples/public/collection-accumulate.intent.json");
    let (p, w) = compile(
        &intent,
        &Ask::new("auto").clone_with(Some(fixture("collection-accumulate", false))),
    );
    assert!(dropped(&w).is_empty(), "{:?}", dropped(&w));
    let s = scene(&p, "beat_1");
    let life = s.lifecycle.expect("lifecycle");
    let entrance = on(s, "b1.aggregate")
        .into_iter()
        .filter_map(half_opacity_at)
        .fold(f64::MAX, f64::min);
    assert!(entrance <= life.anticipate - 0.18, "{entrance}");

    // A beat with room keeps the usual chain: every item after the first
    // arrives in EVOLVE, as before (no speech: the beat is long).
    let (long, _) = compile(&intent, &Ask::new("auto"));
    let s = scene(&long, "beat_1");
    let life = s.lifecycle.expect("lifecycle");
    let second = on(s, "b1.items.1.card")
        .iter()
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    assert!(
        second >= life.evolve - 1e-6,
        "item 1 at {second}, EVOLVE at {}",
        life.evolve
    );
}

impl Ask {
    fn clone_with(&self, speech: Option<SpeechMap>) -> Ask {
        Ask {
            speech,
            ..self.clone()
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Stat pair / relation stage: the seeded arrival, recorded
// ---------------------------------------------------------------------------

/// Four beats, each two pictures with a figure each (a stat pair in a flat
/// look, the relation stage in the cinematic one).
fn pairs() -> CreativeIntent {
    let beat = |a: &str, av: &str, b: &str, bv: &str, rel: &str| {
        json!({
            "purpose": "compare",
            "statement": format!("{a} against {b}"),
            "primary": {"kind": "object", "asset": a, "meaning": a, "value": av},
            "secondary": {"kind": "object", "asset": b, "meaning": b, "value": bv},
            "relationship": rel,
            "energy": "building"
        })
    };
    intent_json(&json!({
        "version": "0.2", "title": "pairs", "format": "vertical",
        "beats": [
            beat("shopping_bag", "$5", "calendar", "$1,825", "grow"),
            beat("credit_card", "$40", "piggy_bank", "$90", "replace"),
            beat("piggy_bank", "$90", "coin_stack", "$250", "accumulate"),
            beat("house", "$75", "rocket", "$185", "separate"),
        ]
    }))
}

fn arrival_sides(p: &MotionProject) -> Vec<Option<String>> {
    let record = p.project.direction.as_ref().expect("direction record");
    record
        .beats
        .iter()
        .map(|b| b.rotations.get("arrival").cloned())
        .collect()
}

fn expected_sides(intent: &CreativeIntent) -> Vec<String> {
    const SIDES: [&str; 4] = ["right", "left", "below", "above"];
    seeded_sequence(story_seed(intent), Dim::Arrival, intent.beats.len(), 4)
        .into_iter()
        .map(|k| SIDES[k].to_string())
        .collect()
}

#[test]
fn stat_pair_arrivals_follow_the_seeded_sequence_and_are_recorded() {
    let intent = pairs();
    let want = expected_sides(&intent);
    for tone in ["playful", "street", "editorial", "cinematic"] {
        let ask = Ask::product(tone, None).families(&["clay_props_3d"]);
        let (p, _) = compile(&intent, &ask);
        let record = p.project.direction.as_ref().expect("direction record");
        let mut staged = 0;
        for b in &record.beats {
            if !matches!(b.template.as_str(), "stat_pair" | "cinematic_3d") {
                continue;
            }
            // A cinematic beat rotates its own arrival; a stat pair and the
            // relation stage record the beat's entry of the sequence.
            let side = b.rotations.get("arrival");
            assert_eq!(
                side,
                Some(&want[b.beat]),
                "{tone} beat {}: {side:?}",
                b.beat
            );
            staged += 1;
        }
        assert!(staged >= 2, "{tone}: {staged} staged beats in {record:?}");
        // No two neighbouring beats share a side.
        let sides = arrival_sides(&p);
        for w in sides.windows(2) {
            if let [Some(a), Some(b)] = w {
                assert_ne!(a, b, "{tone}: {sides:?}");
            }
        }
    }
}

#[test]
fn a_flat_stat_pair_slides_in_from_its_seeded_side_and_the_second_from_the_opposite() {
    let intent = pairs();
    let (p, _) = compile(
        &intent,
        &Ask::product("playful", None).families(&["clay_props_3d"]),
    );
    let record = p.project.direction.as_ref().expect("direction record");
    let mut checked = 0;
    for b in record.beats.iter().filter(|b| b.template == "stat_pair") {
        let Some(side) = b.rotations.get("arrival") else {
            panic!("beat {} records no arrival", b.beat)
        };
        let s = scene(&p, &format!("beat_{}", b.beat + 1));
        let prefix = format!("b{}", b.beat + 1);
        let slide = |name: &str| -> [f32; 2] {
            let m = on(s, &format!("{prefix}.{name}"))
                .into_iter()
                .find_map(|m| match m.op {
                    MotionOp::Move { from, .. } => Some(from),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{prefix}.{name}: no slide"));
            m
        };
        let (first, second) = (slide("hero"), slide("prop"));
        let sign = |v: [f32; 2]| match side.as_str() {
            "right" => v[0] > 0.0,
            "left" => v[0] < 0.0,
            "below" => v[1] > 0.0,
            _ => v[1] < 0.0,
        };
        assert!(
            sign(first),
            "beat {}: {side} but the first slides from {first:?}",
            b.beat
        );
        assert!(
            !sign(second) && (second[0] != 0.0 || second[1] != 0.0),
            "beat {}: the second slides from {second:?}, not the opposite of {side}",
            b.beat
        );
        checked += 1;
    }
    assert!(checked >= 2, "{checked} stat pairs");
}

#[test]
fn without_a_seed_the_stat_pair_slides_left_and_right_and_records_nothing() {
    let intent = pairs();
    let (p, _) = compile(&intent, &Ask::new("playful").families(&["clay_props_3d"]));
    assert!(p.project.direction.is_none());
    for beat in 1..=4 {
        let s = scene(&p, &format!("beat_{beat}"));
        let slide = |name: &str| {
            on(s, &format!("b{beat}.{name}"))
                .into_iter()
                .find_map(|m| match m.op {
                    MotionOp::Move { from, .. } if from[0] != 0.0 => Some(from),
                    _ => None,
                })
        };
        if let (Some(a), Some(b)) = (slide("hero"), slide("prop")) {
            assert!(a[0] < 0.0 && b[0] > 0.0, "beat {beat}: {a:?} {b:?}");
            assert_eq!((a[1], b[1]), (0.0, 0.0));
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Split contrast: the second panel comes in with the first of its words
// ---------------------------------------------------------------------------

/// Two numbers compared; the second one's label ("start at 35") is said
/// before its value.
fn retirement() -> CreativeIntent {
    intent_json(&json!({
        "version": "0.2", "title": "retire", "format": "vertical",
        "beats": [{
            "purpose": "compare",
            "statement": "Start early",
            "narration": "Start at thirty five and you retire with two hundred and forty thousand dollars, \
                          but starting later leaves one hundred and twenty thousand dollars.",
            "primary": {"kind": "number", "value": "$240,000", "meaning": "start at 35"},
            "secondary": {"kind": "number", "value": "$120,000", "meaning": "start later"},
            "energy": "building"
        }]
    }))
}

#[test]
fn the_number_pair_alternate_is_offered_under_a_voice_over_and_its_panel_comes_first() {
    let intent = retirement();
    let speech = synthetic_speech(&intent, &[6.0], &[0.5]);
    let mut split = 0;
    // Seeds that pick the alternate (the story's own seed is one of many).
    for variety in 0u64..48 {
        let style: StyleProfile = serde_json::from_str(r#"{"tone":"playful"}"#).expect("style");
        let opts = CompileOptions {
            speech: Some(repaired(&intent, &speech)),
            art: Some(ArtMode::Auto),
            variety: Some(variety),
            ..CompileOptions::default()
        };
        let (p, _) = compile_with_report(
            &intent,
            &style,
            None,
            &AssetLibrary::new(repo().join("assets")),
            &ApproxMeasure,
            &AssetManifest::empty(),
            None,
            &opts,
        )
        .expect("compile");
        let record = p.project.direction.as_ref().expect("record");
        let beat = &record.beats[0];
        assert_eq!(
            beat.alternates, 2,
            "the split is offered under a voice-over"
        );
        if beat.template != "split_contrast" {
            continue;
        }
        split += 1;
        let s = scene(&p, "beat_1");
        let panel = on(s, "b1.panel_b")
            .iter()
            .find_map(|m| matches!(m.op, MotionOp::MaskReveal { .. }).then_some(m.start))
            .expect("the second panel's mask");
        // Every text drawn on the second panel (the figure and its label) is
        // there after the panel has begun to come in.
        let mut texts = Vec::new();
        find_all(&s.layers, "b1.", &mut texts);
        let bound: Vec<&Layer> = texts
            .into_iter()
            .filter(|l| {
                matches!(l.kind, LayerKind::Text(_))
                    && l.layout.as_ref().is_some_and(|b| b.parent == "b1.panel_b")
            })
            .collect();
        assert!(bound.len() >= 2, "figure and label: {}", bound.len());
        for l in bound {
            let first = on(s, &l.id)
                .into_iter()
                .filter_map(|m| matches!(m.op, MotionOp::Fade { .. }).then_some(m.start))
                .fold(f64::MAX, f64::min);
            assert!(
                panel <= first + 1e-6,
                "variety {variety}: {} fades in at {first:.3} s, its panel at {panel:.3} s",
                l.id
            );
        }
    }
    assert!(
        split >= 3,
        "the alternate was chosen {split} times in 48 seeds"
    );
}

// ---------------------------------------------------------------------------
// 4. Layers: the pinned subject is drawn when the beat is read; the headline
//    stands on paper; a text pinned in a seam stands on a card
// ---------------------------------------------------------------------------

fn layers_intent() -> CreativeIntent {
    intent_file("examples/public/layers.intent.json")
}

#[test]
fn the_pinned_subject_of_a_layers_beat_is_drawn_when_it_is_read() {
    let intent = layers_intent();
    let speeches = [
        (
            "spoken",
            synthetic_speech(&intent, &[2.4, 2.0, 2.6], &[0.5]),
        ),
        ("tight", synthetic_speech(&intent, &[1.5, 1.2, 1.6], &[0.3])),
    ];
    for tone in TONES {
        for (name, speech) in &speeches {
            let (p, _) = compile(&intent, &Ask::product(tone, Some(speech.clone())));
            validate(&p, Some(&repo().join("assets")))
                .unwrap_or_else(|e| panic!("{tone} {name}: {e}"));
            let found = layout(&p, &ImageIndex::new());
            let focal = findings_of(&found, LayoutCheck::FocalOutOfFocus);
            assert!(focal.is_empty(), "{tone} ({name}): {focal:?}");
            // Its entrance has begun before READ, with time to be seen.
            for beat in 1..=3 {
                let s = scene(&p, &format!("beat_{beat}"));
                let life = s.lifecycle.expect("lifecycle");
                let pinned = s
                    .motions
                    .iter()
                    .filter(|m| m.target.starts_with(&format!("b{beat}.pinned.")))
                    .filter(|m| matches!(m.op, MotionOp::Fade { from, .. } if from < 0.5))
                    .map(|m| m.start)
                    .fold(f64::MAX, f64::min);
                assert!(
                    pinned <= life.read - 0.05 + 1e-6,
                    "{tone} ({name}) beat {beat}: pinned fades in at {pinned:.3} s, READ at {:.3}",
                    life.read
                );
            }
        }
    }
}

#[test]
fn without_a_seed_only_a_pinned_subject_that_starts_at_read_moves() {
    // Without a direction seed and art direction a layers compile is what it
    // was: the pinned subject keeps its time, whatever the headline does.
    let intent = layers_intent();
    let speech = synthetic_speech(&intent, &[2.4, 2.0, 2.6], &[0.5]);
    let (plain, _) = compile(
        &intent,
        &Ask::new("playful").clone_with(Some(speech.clone())),
    );
    let (art, _) = compile(
        &intent,
        &Ask {
            art: true,
            ..Ask::new("playful").clone_with(Some(speech))
        },
    );
    let pinned_start = |p: &MotionProject, beat: usize| {
        on(scene(p, &format!("beat_{beat}")), &{
            let s = scene(p, &format!("beat_{beat}"));
            let mut found = Vec::new();
            find_all(&s.layers, &format!("b{beat}.pinned."), &mut found);
            found.first().map(|l| l.id.clone()).unwrap_or_default()
        })
        .iter()
        .filter(|m| matches!(m.op, MotionOp::Fade { from, .. } if from < 0.5))
        .map(|m| m.start)
        .fold(f64::MAX, f64::min)
    };
    for beat in 1..=3 {
        let life = scene(&art, &format!("beat_{beat}"))
            .lifecycle
            .expect("lifecycle");
        // With art direction the focal layer is checked: it starts before READ.
        assert!(pinned_start(&art, beat) < life.read, "beat {beat}");
        // Without it nothing moved the pinned subject earlier than its plan:
        // never earlier than with art direction, which only ever moves it up.
        assert!(pinned_start(&plain, beat) >= pinned_start(&art, beat) - 1e-9);
    }
}

#[test]
fn the_title_paper_holds_to_the_foot_of_the_headline_on_the_product_path() {
    let intent = layers_intent();
    let speech = synthetic_speech(&intent, &[2.4, 2.0, 2.6], &[0.5]);
    let mut seen = 0;
    for tone in ["documentary", "hype", "editorial", "playful", "street"] {
        let (p, _) = compile(&intent, &Ask::product(tone, Some(speech.clone())));
        for beat in 1..=3 {
            let s = scene(&p, &format!("beat_{beat}"));
            let Some(scrim) = find(&s.layers, &format!("b{beat}.scrim")) else {
                continue; // a light-type palette needs no paper
            };
            let LayerKind::Group { children } = &scrim.kind else {
                panic!("scrim is a group")
            };
            // The lowest edge of the headline's lines.
            let mut heads = Vec::new();
            find_all(&s.layers, &format!("b{beat}.head."), &mut heads);
            let foot = heads
                .iter()
                .filter(|l| matches!(l.kind, LayerKind::Text(_)))
                .map(|l| l.y + l.height)
                .fold(0.0_f32, f32::max);
            assert!(foot > 0.0, "{tone} beat {beat}: no headline lines");
            for strip in children {
                let LayerKind::Rectangle { fill, .. } = &strip.kind else {
                    continue;
                };
                if strip.y + strip.height <= foot {
                    assert!(
                        fill.a >= 0xE0,
                        "{tone} beat {beat}: the paper is {:#04x} opaque at y {:.0}, \
                         the headline ends at {foot:.0}",
                        fill.a,
                        strip.y
                    );
                }
            }
            seen += 1;
        }
    }
    assert!(seen >= 6, "{seen} scrims judged");
}

#[test]
fn a_text_pinned_in_a_seam_stands_on_a_card_and_a_picture_does_not() {
    let intent = layers_intent();
    let speech = synthetic_speech(&intent, &[2.4, 2.0, 2.6], &[0.5]);
    // Beat 2 pins the phrase "temperature stops falling" in the boundary layer.
    let (product, _) = compile(&intent, &Ask::product("hype", Some(speech.clone())));
    let s = scene(&product, "beat_2");
    let mut pinned = Vec::new();
    find_all(&s.layers, "b2.pinned.", &mut pinned);
    let group = pinned
        .iter()
        .find(|l| matches!(l.kind, LayerKind::Group { .. }))
        .expect("the pinned text is a group of card and text");
    let LayerKind::Group { children } = &group.kind else {
        unreachable!()
    };
    assert!(children.iter().any(|c| c.id.ends_with(".card")));
    assert!(children
        .iter()
        .any(|c| matches!(c.kind, LayerKind::Text(_))));
    // Without a seed it is the text alone, as before.
    let (plain, _) = compile(&intent, &Ask::new("hype").clone_with(Some(speech)));
    let s = scene(&plain, "beat_2");
    let mut pinned = Vec::new();
    find_all(&s.layers, "b2.pinned.", &mut pinned);
    assert!(pinned
        .iter()
        .all(|l| !matches!(l.kind, LayerKind::Group { .. })));
}

// ---------------------------------------------------------------------------
// 5. Documentary: photos that hold their border, stamps that clear the subject
// ---------------------------------------------------------------------------

/// The luminance-based WCAG contrast of two colours (as the QA computes it).
fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    fn lum(c: [u8; 3]) -> f64 {
        let f = |v: u8| {
            let v = f64::from(v) / 255.0;
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    }
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn rgb(c: &motion_core::scene::Color) -> [u8; 3] {
    [c.r, c.g, c.b]
}

#[test]
fn a_documentary_library_photo_keeps_2_to_1_from_its_border_whatever_its_colour() {
    let intent = intent_file("examples/topics/ai_learns.intent.json");
    let (p, _) = compile(
        &intent,
        &Ask::product("documentary", Some(fixture("ai_learns", false))),
    );
    let mut judged = 0;
    for s in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let mut pics = Vec::new();
        find_all(&s.layers, ".photo.pic.", &mut pics);
        for l in pics {
            let LayerKind::Image { treatment, .. } = &l.kind else {
                continue;
            };
            let t = treatment.as_ref().expect("a treatment");
            let sticker = t.sticker.expect("an ink border");
            let tint = t.tint.expect("a faded print");
            assert!(tint.amount > 0.0 && tint.amount < 0.6, "{}: {tint:?}", l.id);
            // A black and a white picture end up 2:1 from the border.
            for extreme in [[0u8, 0, 0], [255, 255, 255]] {
                let mix = |c: u8, to: u8| {
                    (f32::from(c) + (f32::from(to) - f32::from(c)) * tint.amount).round() as u8
                };
                let treated = [
                    mix(extreme[0], tint.color.r),
                    mix(extreme[1], tint.color.g),
                    mix(extreme[2], tint.color.b),
                ];
                assert!(
                    contrast(treated, rgb(&sticker.color)) >= 2.0,
                    "{}: {treated:?} against {:?}",
                    l.id,
                    sticker.color
                );
            }
            judged += 1;
        }
    }
    assert!(judged >= 4, "{judged} documentary photos judged");
    // Without a seed the picture is exactly what it was: border, no fade.
    let (plain, _) = compile(
        &intent,
        &Ask::new("documentary").clone_with(Some(fixture("ai_learns", false))),
    );
    for s in plain.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let mut pics = Vec::new();
        find_all(&s.layers, ".photo.pic.", &mut pics);
        for l in pics {
            if let LayerKind::Image { treatment, .. } = &l.kind {
                assert!(
                    treatment.as_ref().is_none_or(|t| t.tint.is_none()),
                    "{}",
                    l.id
                );
            }
        }
    }
}

#[test]
fn the_documentary_layout_qa_is_clean_with_the_measured_pictures() {
    let sprites = [(
        "clay_props_3d",
        "piggy_bank",
        384,
        497,
        [0.096, 0.177, 0.823, 0.767],
    )];
    for (story, intent, speech, families) in [
        (
            "ai_learns",
            intent_file("examples/topics/ai_learns.intent.json"),
            fixture("ai_learns", false),
            vec!["editorial_concepts", "clay_concepts_3d", "clay_props_3d"],
        ),
        (
            "space",
            intent_file("examples/cinematic/space.intent.json"),
            fixture("space", false),
            vec!["editorial_concepts", "clay_concepts_3d"],
        ),
    ] {
        let ask = Ask::product("documentary", Some(speech)).families(&families);
        let (p, _) = compile(&intent, &ask);
        let index = catalog_index(&families, &sprites);
        let found = layout(&p, &index);
        for check in [
            LayoutCheck::AssetLowContrast,
            LayoutCheck::TextOverSubject,
            LayoutCheck::FrameTooLoose,
        ] {
            let list = findings_of(&found, check);
            assert!(list.is_empty(), "{story}: {list:?}");
        }
    }
}

// ---------------------------------------------------------------------------
// 6. The evidence stack's note card hugs its picture
// ---------------------------------------------------------------------------

#[test]
fn the_note_card_hugs_the_picture_it_frames() {
    let intent = intent_file("examples/cinematic/space.intent.json");
    let families = ["editorial_concepts", "clay_concepts_3d"];
    let index = catalog_index(&families, &[]);
    let mut judged = 0;
    for tone in ["auto", "editorial", "technical", "playful"] {
        let ask = Ask::product(tone, Some(fixture("space", false))).families(&families);
        let (p, _) = compile(&intent, &ask);
        let found = layout(&p, &index);
        let loose = findings_of(&found, LayoutCheck::FrameTooLoose);
        assert!(loose.is_empty(), "{tone}: {loose:?}");
        let s = scene(&p, "beat_4");
        if let (Some(card), Some(tab)) = (
            find(&s.layers, "b4.note.card"),
            find(&s.layers, "b4.note.tab"),
        ) {
            // The tab rides the card's left edge and has its height.
            assert!((tab.x - card.x).abs() < 1e-3 && (tab.height - card.height).abs() < 1e-3);
            // A picture's card is a frame, not a plate: a few times the picture at most.
            let mut pics = Vec::new();
            find_all(&s.layers, "b4.note.", &mut pics);
            let pic = pics
                .iter()
                .find(|l| matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. }))
                .expect("the note picture");
            assert!(
                card.width <= 1.4 * pic.width,
                "{tone}: {} against {}",
                card.width,
                pic.width
            );
            judged += 1;
        }
    }
    assert!(judged >= 3, "{judged} note cards judged");
}

// ---------------------------------------------------------------------------
// 7. The cinematic hero word fits the safe width and keeps its fly-in
// ---------------------------------------------------------------------------

#[test]
fn a_cinematic_hero_word_fits_the_safe_width_and_keeps_its_fly_in() {
    let intent = intent_file("golden/fixtures/music_mood/volcano_eruption.intent.json");
    let speech = synthetic_speech(&intent, &[3.0, 2.8, 3.2], &[0.5]);
    let frame = LayoutFrame::new(1080, 1920).expect("frame");
    let mut judged = 0;
    for tone in ["cinematic", "auto"] {
        let (p, _) = compile(&intent, &Ask::product(tone, Some(speech.clone())));
        for s in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
            let mut words = Vec::new();
            find_all(&s.layers, ".hero_word", &mut words);
            for l in words {
                if !matches!(l.kind, LayerKind::Text(_)) {
                    continue;
                }
                assert!(
                    l.width <= frame.safe.w + 1e-3,
                    "{tone} {}: {:.0} px wide, the safe width is {:.0}",
                    l.id,
                    l.width,
                    frame.safe.w
                );
                assert!(
                    on(s, &l.id)
                        .iter()
                        .any(|m| matches!(m.op, MotionOp::GlyphCascade { .. })),
                    "{}: lost its fly-in",
                    l.id
                );
                judged += 1;
            }
        }
    }
    assert!(judged >= 4, "{judged} hero words judged");
}

// ---------------------------------------------------------------------------
// 8. habit_math: pictures with figures in the flat, documentary and cinematic looks
// ---------------------------------------------------------------------------

fn habit_math() -> CreativeIntent {
    let obj = |asset: &str, meaning: &str, value: &str| json!({"kind": "object", "asset": asset, "meaning": meaning, "value": value});
    intent_json(&json!({
        "version": "0.2", "title": "habit_math", "format": "vertical",
        "beats": [
            {"purpose": "emphasize", "statement": "A five dollar treat",
             "narration": "A five dollar treat every day feels like nothing.",
             "primary": obj("shopping_bag", "treat", "$5"),
             "energy": "building", "continuity": "carry_primary", "keyword": "treat"},
            {"purpose": "compare", "statement": "A day versus a year",
             "narration": "But five dollars a day grows to one thousand eight hundred and twenty five dollars a year.",
             "primary": obj("shopping_bag", "a day", "$5"),
             "secondary": obj("calendar", "a year", "$1,825"),
             "relationship": "grow", "energy": "building", "keyword": "year"},
            {"purpose": "contrast", "statement": "Spend it or save it",
             "narration": "Swap the card for the piggy bank, and the same money starts working for you.",
             "primary": obj("credit_card", "spent", "$1,825"),
             "secondary": obj("piggy_bank", "saved", "$1,825"),
             "relationship": "replace", "energy": "building",
             "continuity": "carry_secondary", "keyword": "save"},
            {"purpose": "compare", "statement": "Ten years of saving",
             "narration": "Saved and invested for ten years, the piggy bank adds up to about twenty five thousand dollars.",
             "primary": obj("piggy_bank", "saved each year", "$1,825"),
             "secondary": obj("coin_stack", "ten years", "$25,000"),
             "relationship": "accumulate", "energy": "building", "keyword": "ten years"}
        ]
    }))
}

fn habit_ask(tone: &'static str) -> Ask {
    let intent = habit_math();
    // Four sentences of 3.5 s: the figures are said late in them.
    Ask::product(
        tone,
        Some(synthetic_speech(&intent, &[3.6, 4.2, 3.8, 4.4], &[0.4])),
    )
    .families(&["clay_props_3d"])
}

#[test]
fn the_stamps_of_a_flat_stat_pair_stay_inside_the_safe_area() {
    let intent = habit_math();
    for tone in ["playful", "street", "hype", "editorial"] {
        for product in [true, false] {
            let mut ask = habit_ask(tone);
            if !product {
                ask.seeded = false;
                ask.art = false;
            }
            let (p, _) = compile(&intent, &ask);
            let found = layout(&p, &ImageIndex::new());
            let out: Vec<String> = findings_of(&found, LayoutCheck::TextOutsideSafe)
                .into_iter()
                .filter(|f| f.contains("pair_stamp"))
                .collect();
            assert!(out.is_empty(), "{tone} (product {product}): {out:?}");
        }
    }
}

#[test]
fn documentary_paper_cards_hug_what_is_drawn_and_carried_pictures_wear_a_border() {
    let intent = habit_math();
    let sprites = [(
        "clay_props_3d",
        "piggy_bank",
        384,
        497,
        [0.096, 0.177, 0.823, 0.767],
    )];
    let index = catalog_index(&["clay_props_3d"], &sprites);
    let (p, _) = compile(&intent, &habit_ask("documentary"));
    let found = layout(&p, &index);
    for check in [LayoutCheck::FrameTooLoose, LayoutCheck::AssetLowContrast] {
        let list = findings_of(&found, check);
        assert!(list.is_empty(), "{list:?}");
    }
    // The shared (carried) picture has the ink border.
    for shared in &p.shared {
        if let LayerKind::Image { treatment, .. } = &shared.layer.kind {
            let t = treatment
                .as_ref()
                .expect("a treatment on the carried picture");
            assert!(t.sticker.is_some(), "{}: no border", shared.layer.id);
        }
    }
    // Without a seed the carried picture is what it was (no border added).
    let mut plain = habit_ask("documentary");
    plain.seeded = false;
    plain.art = false;
    let (q, _) = compile(&intent, &plain);
    for shared in &q.shared {
        if let LayerKind::Image { treatment, .. } = &shared.layer.kind {
            assert!(treatment.as_ref().is_none_or(|t| t.sticker.is_none()));
        }
    }
}

#[test]
fn a_cinematic_grow_draws_its_arrow_while_the_pair_is_read() {
    let intent = habit_math();
    let (p, _) = compile(&intent, &habit_ask("cinematic"));
    let s = scene(&p, "beat_2");
    let life = s.lifecycle.expect("lifecycle");
    let shaft = on(s, "b2.pair_arrow")
        .into_iter()
        .find(|m| matches!(m.op, MotionOp::Trim { .. }))
        .expect("the arrow's shaft draws");
    let head = on(s, "b2.pair_arrowhead")
        .into_iter()
        .find(|m| matches!(m.op, MotionOp::Fade { .. }))
        .expect("the arrowhead");
    // Drawn with room to be seen before the beat leaves, not at its end.
    assert!(
        head.start + head.duration <= life.anticipate - 0.4,
        "the arrow is complete at {:.2} s, ANTICIPATE at {:.2} s",
        head.start + head.duration,
        life.anticipate
    );
    // And the target grows once it is there.
    let grow = on(s, "b2.prop")
        .into_iter()
        .filter(|m| matches!(m.op, MotionOp::Scale { to, .. } if to > 1.05))
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    assert!(
        grow >= shaft.start,
        "the grow {grow} follows the arrow at {}",
        shaft.start
    );
}

#[test]
fn a_cinematic_stamp_stands_on_a_chip_it_reads_on() {
    let intent = habit_math();
    let (p, _) = compile(&intent, &habit_ask("cinematic"));
    let mut judged = 0;
    for s in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let mut stamps = Vec::new();
        find_all(&s.layers, ".pair_stamp.", &mut stamps);
        for g in stamps {
            let LayerKind::Group { children } = &g.kind else {
                continue;
            };
            let Some(chip) = children.iter().find(|c| c.id.ends_with(".chip")) else {
                continue;
            };
            let LayerKind::RoundedRectangle { fill, .. } = &chip.kind else {
                panic!("the chip is a rounded rectangle")
            };
            let text = children
                .iter()
                .find_map(|c| match &c.kind {
                    LayerKind::Text(t) => Some(t),
                    _ => None,
                })
                .expect("the figure");
            assert!(
                contrast(rgb(fill), rgb(&text.color)) >= 4.5,
                "{}: {:?} on {:?}",
                g.id,
                text.color,
                fill
            );
            judged += 1;
        }
    }
    assert!(judged >= 2, "{judged} stamps judged");
    // Without a seed the stamps are the bare text they were.
    let mut plain = habit_ask("cinematic");
    plain.seeded = false;
    plain.art = false;
    let (q, _) = compile(&intent, &plain);
    for s in q.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let mut stamps = Vec::new();
        find_all(&s.layers, ".pair_stamp.", &mut stamps);
        assert!(stamps.iter().all(|g| matches!(g.kind, LayerKind::Text(_))));
    }
}

/// The fun-facts family is a gitignored library (the owner's checkout has it,
/// a fresh worktree does not): its reel is only judged where it exists.
#[test]
fn the_five_facts_pair_stamps_read_on_their_ground() {
    if !repo()
        .join("assets/library/fun_facts/catalog.json")
        .exists()
    {
        return;
    }
    let intent = intent_file("examples/facts/five_wait_what.intent.json");
    let style: StyleProfile =
        serde_json::from_str(&read("examples/facts/five_wait_what.style.json")).expect("style");
    let library =
        AssetLibrary::new(repo().join("assets")).with_families(vec!["fun_facts".to_string()]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: Some(story_seed(&intent)),
        ..CompileOptions::default()
    };
    let (p, _) = compile_with_report(
        &intent,
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile");
    let mut by_scene: BTreeMap<String, usize> = BTreeMap::new();
    for s in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let mut stamps = Vec::new();
        find_all(&s.layers, ".pair_stamp.", &mut stamps);
        // The stamp itself (`bN.pair_stamp.K`), not its depth wrapper or parts.
        for g in stamps.into_iter().filter(|g| {
            g.id.rsplit('.')
                .next()
                .is_some_and(|k| k.parse::<u8>().is_ok())
        }) {
            let LayerKind::Group { children } = &g.kind else {
                panic!("{}: a cinematic stamp stands on a chip", g.id)
            };
            assert!(children.iter().any(|c| c.id.ends_with(".chip")));
            *by_scene.entry(s.id.clone()).or_default() += 1;
        }
    }
    assert!(!by_scene.is_empty(), "no stamps in the five facts");
    assert!(
        layout_report(&p, &LayoutFrame::new(1080, 1920).expect("frame"))
            .findings
            .iter()
            .all(|f| f.check != LayoutCheck::TextOutsideSafe || !f.layer.contains("pair_stamp"))
    );
}
