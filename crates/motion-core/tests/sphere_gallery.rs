//! (0.18) SphereGallery: a cinematic collection of 3+ items rides a turning
//! sphere; each item comes to the front (sharp, enlarged, labelled) in turn.
//! Structural assertions on the compiled scene plus resolved frames; no pixels.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::LayoutFinding;
use motion_core::scene::{MotionOp, MotionProject, Scene};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::style::StyleProfile;
use motion_core::timeline::evaluate_frame;
use motion_core::validate::validate;
use motion_core::{layout_report, Easing};

const TOOLKIT: &str = include_str!("../../../examples/cinematic/ai_toolkit.intent.json");
const STYLE: &str = include_str!("../../../examples/cinematic/ai_toolkit.style.json");
const CANVASES: [(u32, u32); 4] = [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)];
const ITEMS: usize = 5;
const DOTS: usize = 48;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn compile_with(
    intent_json: &str,
    canvas: Option<(u32, u32)>,
    art: bool,
    speech: Option<SpeechMap>,
) -> MotionProject {
    let intent = CreativeIntent::from_json(intent_json).expect("intent");
    let style: StyleProfile = serde_json::from_str(STYLE).expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        art: art.then_some(ArtMode::Auto),
        canvas,
        speech,
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn toolkit(canvas: Option<(u32, u32)>) -> MotionProject {
    compile_with(TOOLKIT, canvas, true, None)
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes.iter().find(|s| s.id == id).expect("scene")
}

/// One revolve motion, comparable across layers (everything but `at`/`radius`).
#[derive(Debug, Clone, PartialEq)]
struct Turn {
    start: f64,
    duration: f64,
    easing: Easing,
    from: [f32; 2],
    to: [f32; 2],
}

/// Revolve chains by target id (in motion order): `(radius, at, turns)`.
#[allow(clippy::type_complexity)]
fn chains(s: &Scene) -> BTreeMap<String, (f32, [f32; 2], Vec<Turn>)> {
    let mut out: BTreeMap<String, (f32, [f32; 2], Vec<Turn>)> = BTreeMap::new();
    for m in &s.motions {
        if let MotionOp::Revolve {
            radius,
            at,
            from,
            to,
        } = m.op
        {
            let e = out
                .entry(m.target.clone())
                .or_insert((radius, at, Vec::new()));
            assert_eq!((e.0, e.1), (radius, at), "{}: radius/at change", m.target);
            e.2.push(Turn {
                start: m.start,
                duration: m.duration,
                easing: m.easing,
                from,
                to,
            });
        }
    }
    out
}

fn item_id(k: usize) -> String {
    format!("b2.item.{k}.depth")
}

fn dot_id(j: usize) -> String {
    format!("b2.dust.{j}.depth")
}

fn wrap180(d: f32) -> f32 {
    let d = d.rem_euclid(360.0);
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

#[test]
fn cinematic_collection_beat_becomes_a_sphere() {
    let p = toolkit(None);
    validate(&p, Some(&repo().join("assets"))).expect("valid");
    let beat2 = scene(&p, "beat_2");
    let c = chains(beat2);
    assert_eq!(c.len(), ITEMS + DOTS, "5 items + the halo ride the sphere");
    for k in 0..ITEMS {
        assert!(c.contains_key(&item_id(k)), "item {k}");
    }
    // The other beats are ordinary cinematic beats.
    for id in ["beat_1", "beat_3"] {
        assert!(chains(scene(&p, id)).is_empty(), "{id}");
    }
    // The sphere beat does not record a focal layer: the front is the focus plane.
    let art = p.project.art.as_ref().expect("art record");
    assert!(!art.focal.contains_key("beat_2"));
    assert!(art.focal.contains_key("beat_1"));
    // Depth sort draws the sphere far to near.
    let persp = beat2
        .camera
        .as_ref()
        .and_then(|c| c.perspective.as_ref())
        .expect("perspective");
    assert!(persp.depth_sort);
}

#[test]
fn every_item_and_dot_has_the_same_chain_of_turns() {
    let p = toolkit(None);
    let c = chains(scene(&p, "beat_2"));
    let reference = &c[&item_id(0)].2;
    // Intro + one turn per further item.
    assert_eq!(reference.len(), ITEMS);
    for (id, (_, _, turns)) in &c {
        assert_eq!(turns, reference, "{id} turns differently");
    }
    // Items sit at lon_k = k * 72, latitudes zig-zag within +-25 degrees.
    let radius = c[&item_id(0)].0;
    for k in 0..ITEMS {
        let (r, at, _) = &c[&item_id(k)];
        assert_eq!(*r, radius, "items share one radius");
        assert!((at[0] - k as f32 * 72.0).abs() < 1e-3, "lon of item {k}");
        assert!(at[1].abs() <= 25.0 + 1e-3, "lat of item {k}");
        if k > 0 {
            assert!(at[1] * c[&item_id(k - 1)].1[1] < 0.0, "lat zig-zags");
        }
    }
    for j in 0..DOTS {
        assert!(c[&dot_id(j)].0 < radius, "dots ride a sphere of their own");
    }
    // Intro spins in from [-lon0 - 140, -lat0 + 15] to item 0's front.
    let lat0 = c[&item_id(0)].1[1];
    let intro = &reference[0];
    assert!((intro.from[0] + 140.0).abs() < 1e-2 && (intro.from[1] - (15.0 - lat0)).abs() < 1e-2);
    assert!(intro.to[0].abs() < 1e-2 && (intro.to[1] + lat0).abs() < 1e-2);
    assert_eq!(intro.easing, Easing::OutCubic);
    assert!(intro.duration > 0.9 && intro.duration <= 1.2 + 1e-9);
    // Every later turn brings the next item to the front the short way round.
    for k in 1..ITEMS {
        let t = &reference[k];
        let lat = c[&item_id(k)].1[1];
        let lon = c[&item_id(k)].1[0];
        assert!(wrap180(t.to[0] + lon).abs() < 1e-2, "yaw of turn {k}");
        assert!((t.to[1] + lat).abs() < 1e-2, "pitch of turn {k}");
        assert!((t.to[0] - t.from[0]).abs() <= 180.0 + 1e-3, "short way");
        assert!(t.duration <= 0.8 + 1e-9);
        assert_eq!(t.easing, Easing::InOutCubic);
        // Continuous: each turn starts where the last ended.
        assert_eq!(t.from, reference[k - 1].to);
    }
}

fn nearest_item(p: &MotionProject, scene_id: &str, local: f64) -> (usize, Vec<Option<f32>>) {
    let s = scene(p, scene_id);
    let fps = f64::from(p.canvas.fps);
    let frame = ((s.start_seconds + local) * fps).round() as u32;
    let resolved = evaluate_frame(p, frame).expect("frame");
    let mut blur: Vec<Option<f32>> = vec![None; ITEMS];
    let mut order: Vec<Option<usize>> = vec![None; ITEMS];
    for (pos, l) in resolved.layers.iter().enumerate() {
        for k in 0..ITEMS {
            if l.id == item_id(k) {
                blur[k] = l.blur;
                order[k] = Some(pos);
            }
        }
    }
    let k = (0..ITEMS)
        .min_by(|a, b| {
            let (x, y) = (blur[*a].unwrap_or(0.0), blur[*b].unwrap_or(0.0));
            x.total_cmp(&y)
        })
        .expect("items");
    // The nearest item is drawn after every other item.
    for o in (0..ITEMS).filter(|o| *o != k) {
        assert!(
            order[o].expect("drawn") < order[k].expect("drawn"),
            "item {k} must be drawn in front of item {o}"
        );
    }
    (k, blur)
}

#[test]
fn item_k_is_the_sharp_front_item_during_dwell_k() {
    let p = toolkit(None);
    let s = scene(&p, "beat_2");
    let life = s.lifecycle.expect("lifecycle");
    let dwell = (life.anticipate - life.read) / ITEMS as f64;
    for k in 0..ITEMS {
        // Just before the next turn starts (the end of the hold).
        let local = life.read + dwell * (k + 1) as f64 - 0.05;
        let (front, blur) = nearest_item(&p, "beat_2", local);
        assert_eq!(front, k, "dwell {k}: item {front} is nearest");
        assert!(
            blur[k].unwrap_or(0.0) < 1.5,
            "dwell {k}: item {k} blurred {:?}",
            blur[k]
        );
        for (o, b) in blur.iter().enumerate().filter(|(o, _)| *o != k) {
            assert!(
                b.unwrap_or(0.0) > blur[k].unwrap_or(0.0) + 1.0,
                "dwell {k}: item {o} ({b:?}) is not blurrier than item {k} ({:?})",
                blur[k]
            );
        }
    }
}

fn findings(p: &MotionProject, w: u32, h: u32) -> Vec<LayoutFinding> {
    let frame = LayoutFrame::new(w, h).expect("frame");
    layout_report(p, &frame).findings
}

#[test]
fn layout_qa_passes_on_the_four_canvases() {
    for (w, h) in CANVASES {
        let p = toolkit(Some((w, h)));
        let sphere: Vec<_> = findings(&p, w, h)
            .into_iter()
            .filter(|f| f.scene == "beat_2")
            .collect();
        assert!(sphere.is_empty(), "{w}x{h}: {sphere:?}");
    }
}

/// Text cards (phrases, numbers, a missing picture) on every canvas. (0.22)
/// A number series stays a data story (a chart compares its values; the
/// sphere shows one at a time).
const CARDS: &str = r#"{
  "version": "0.2", "title": "cards", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Three kinds of cards",
     "primary": {"kind": "collection", "items": [
       {"kind": "number", "value": "₹120", "meaning": "coffee"},
       {"kind": "phrase", "value": "Cold brew"},
       {"kind": "object", "asset": "zzz_no_such_object", "meaning": "mystery"},
       {"kind": "phrase", "meaning": "Free refill"}]},
     "energy": "calm", "keyword": "cards"},
    {"purpose": "emphasize", "statement": "Numbers on a sphere",
     "primary": {"kind": "collection", "items": [
       {"kind": "number", "value": "12"}, {"kind": "number", "value": "48"},
       {"kind": "number", "value": "96"}]},
     "energy": "calm", "keyword": "numbers"}
  ]
}"#;

#[test]
fn text_cards_fit_every_canvas_and_unknown_objects_become_cards() {
    for (w, h) in CANVASES {
        let p = compile_with(CARDS, Some((w, h)), true, None);
        validate(&p, Some(&repo().join("assets"))).expect("valid");
        assert!(
            !chains(scene(&p, "beat_1")).is_empty(),
            "beat_1 is a sphere"
        );
        let series = scene(&p, "beat_2");
        assert!(chains(series).is_empty(), "beat_2 is a chart");
        assert!(series.motions.iter().any(|m| m.target == "b2.chart.bar.2"));
        let bad: Vec<_> = findings(&p, w, h);
        assert!(bad.is_empty(), "{w}x{h}: {bad:?}");
    }
}

#[test]
fn compile_is_deterministic() {
    let a = serde_json::to_string(&toolkit(None)).expect("json");
    let b = serde_json::to_string(&toolkit(None)).expect("json");
    assert_eq!(a, b);
}

#[test]
fn two_items_and_no_art_keep_their_grammar() {
    // Two items: the structured collection composer, no sphere.
    let two = TOOLKIT.replace(
        r#"          { "kind": "object", "asset": "brain_circuit", "meaning": "learning" },
          { "kind": "object", "asset": "chat_bubbles", "meaning": "conversation" },
          { "kind": "object", "asset": "ai_sparkles", "meaning": "creativity" }
"#,
        "",
    );
    let two = two.replace(
        r#"{ "kind": "object", "asset": "microchip", "meaning": "compute" },"#,
        r#"{ "kind": "object", "asset": "microchip", "meaning": "compute" }"#,
    );
    assert_ne!(two, TOOLKIT);
    let p = compile_with(&two, None, true, None);
    let beat2 = scene(&p, "beat_2");
    assert!(
        chains(beat2).is_empty(),
        "2 items must not turn on a sphere"
    );
    assert!(!beat2.layers.iter().any(|l| l.id.contains(".item.")));
    // Without art there is no look, hence no sphere (default compiles unchanged).
    let plain = compile_with(TOOLKIT, None, false, None);
    for s in &plain.scenes {
        assert!(chains(s).is_empty(), "{}", s.id);
        assert!(s
            .camera
            .as_ref()
            .is_none_or(|c| c.perspective.as_ref().is_none_or(|p| !p.depth_sort)));
    }
}

/// Speech that names every item and label early in the beat: word cues must
/// not retime the rotation chain, the item pops or the labels.
fn speech_for(intent: &CreativeIntent) -> SpeechMap {
    let starts = [0.35, 4.6, 12.0];
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    for (beat, (b, &start)) in intent.beats.iter().zip(&starts).enumerate() {
        let end = start + 3.0;
        sentences.push(SpeechSentence { beat, start, end });
        let spoken: Vec<String> = if beat == 1 {
            "assistant compute learning conversation creativity robot microchip brain \
             circuit chat bubbles ai sparkles tools"
                .split_whitespace()
                .map(str::to_string)
                .collect()
        } else {
            motion_core::speech::statement_tokens(&b.statement)
        };
        let each = 3.0 / spoken.len() as f64;
        for (i, t) in spoken.into_iter().enumerate() {
            words.push(SpeechWord {
                text: t,
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
        duration: 12.0 + 3.0 + 0.9,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
    }
}

#[test]
fn word_cues_do_not_retime_the_sphere() {
    let intent = CreativeIntent::from_json(TOOLKIT).expect("intent");
    let p = compile_with(TOOLKIT, None, true, Some(speech_for(&intent)));
    validate(&p, Some(&repo().join("assets"))).expect("valid");
    let s = scene(&p, "beat_2");
    let life = s.lifecycle.expect("lifecycle");
    let c = chains(s);
    let turns = &c[&item_id(0)].2;
    assert_eq!(turns.len(), ITEMS);
    for (id, (_, _, t)) in &c {
        assert_eq!(t, turns, "{id}");
    }
    // The schedule is the lifecycle's: dwell k starts at read + k/5 of the span.
    let dwell = (life.anticipate - life.read) / ITEMS as f64;
    for (k, turn) in turns.iter().enumerate().skip(1) {
        let expect = life.read + dwell * k as f64;
        assert!(
            (turn.start - expect).abs() < 2e-3,
            "turn {k} starts {} expected {expect}",
            turn.start
        );
    }
    // Item pops and labels hang on the arrivals (end of the turn onto the item).
    let arrive = |k: usize| turns[k].start + turns[k].duration;
    for k in 0..ITEMS {
        let at = |target: String| {
            s.motions
                .iter()
                .filter(|m| m.target == target)
                .find(|m| {
                    matches!(m.op, MotionOp::Scale { .. })
                        || matches!(m.op, MotionOp::Fade { to, .. } if to > 0.5)
                            && m.start > life.enter + 1.0
                })
                .map(|m| m.start)
        };
        let pop = at(format!("b2.item.{k}"));
        assert!(
            pop.is_some_and(|t| (t - arrive(k)).abs() < 2e-3),
            "item {k} pop {pop:?} vs arrival {}",
            arrive(k)
        );
        let label = at(format!("b2.sphere_label.{k}"));
        assert!(
            label.is_some_and(|t| (t - (arrive(k) + 0.03)).abs() < 2e-3),
            "label {k} {label:?} vs arrival {}",
            arrive(k)
        );
    }
}
