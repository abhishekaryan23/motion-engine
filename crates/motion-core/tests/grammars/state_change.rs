//! State-change composition: A -> B reads as a transformation (the old state is
//! readable, then replaced, then stays visible in reduced form), two changes
//! play in parallel and stay distinct, everything fits and ends before exit,
//! under every motion language. Compiles with the font-free `ApproxMeasure`.

use std::path::Path;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::timeline::ResolvedLayer;
use motion_core::{evaluate_frame, validate};
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const FPS: f64 = 30.0;
const LANGUAGES: [&str; 6] = [
    "auto",
    "minimal",
    "kinetic",
    "parallax",
    "sequential",
    "data",
];

fn style(language: &str) -> StyleProfile {
    StyleProfile::from_json(
        &json!({
            "material": "paper",
            "typography_style": "grotesk_serif",
            "depth": "layered",
            "camera_style": "slow_push",
            "motion_language": language,
            "texture_style": "subtle_print",
            "accent_role": "cobalt",
            "seed": 1
        })
        .to_string(),
    )
    .expect("style parses")
}

fn intent(beats: Vec<Value>, format: &str) -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2",
        "title": "state_change_test",
        "format": format,
        "beats": beats,
    }))
    .expect("intent parses")
}

fn change(entity: &str, from: &str, to: &str, meaning: Option<&str>) -> Value {
    let mut v = json!({ "kind": "state_change", "entity": entity, "from": from, "to": to });
    if let Some(m) = meaning {
        v["meaning"] = json!(m);
    }
    v
}

fn single_beat() -> Value {
    json!({
        "purpose": "emphasize",
        "statement": "Delivery got faster.",
        "primary": change("delivery", "three days", "same day", Some("faster")),
    })
}

fn dual_beat(meanings: bool) -> Value {
    json!({
        "purpose": "contrast",
        "statement": "Better and cheaper at once.",
        "primary": change("delivery", "slow", "fast", meanings.then_some("quicker")),
        "secondary": change("cost", "high", "low", meanings.then_some("cheaper")),
    })
}

/// Follow-up beat so the state-change beat has an outgoing transition.
fn follow_up() -> Value {
    json!({
        "purpose": "emphasize",
        "statement": "Customers noticed.",
        "primary": { "kind": "phrase", "value": "Noticed" },
    })
}

fn build(i: &CreativeIntent, language: &str) -> MotionProject {
    let p = compile(
        i,
        &style(language),
        &AssetLibrary::new(ASSETS),
        &ApproxMeasure,
    )
    .expect("compiles");
    validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("{language}: invalid: {e:?}"));
    p
}

fn compile_single() -> MotionProject {
    build(&intent(vec![single_beat()], "vertical"), "auto")
}

fn compile_dual() -> MotionProject {
    build(&intent(vec![dual_beat(false)], "vertical"), "auto")
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene '{id}' missing"))
}

fn find_layer<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            if let Some(f) = find_layer(children, id) {
                return Some(f);
            }
        }
    }
    None
}

fn layer<'a>(p: &'a MotionProject, id: &str) -> &'a Layer {
    find_layer(&scene(p, "beat_1").layers, id).unwrap_or_else(|| panic!("layer '{id}' missing"))
}

fn has(p: &MotionProject, id: &str) -> bool {
    find_layer(&scene(p, "beat_1").layers, id).is_some()
}

fn collect_ids(layers: &[Layer], out: &mut Vec<String>) {
    for l in layers {
        out.push(l.id.clone());
        if let LayerKind::Group { children } = &l.kind {
            collect_ids(children, out);
        }
    }
}

/// Unit layer ids of a run (`b1.sc0.from` -> `b1.sc0.from.{line}.{unit}`).
fn units(p: &MotionProject, run: &str) -> Vec<String> {
    let mut ids = Vec::new();
    collect_ids(&scene(p, "beat_1").layers, &mut ids);
    let prefix = format!("{run}.");
    ids.into_iter()
        .filter(|id| id.starts_with(&prefix))
        .collect()
}

fn motions<'a>(p: &'a MotionProject, target: &'a str) -> impl Iterator<Item = &'a Motion> {
    scene(p, "beat_1")
        .motions
        .iter()
        .filter(move |m| m.target == target)
}

fn is_fade(m: &Motion, from: f32, to: f32) -> bool {
    matches!(m.op, MotionOp::Fade { from: a, to: b } if (a - from).abs() < 1e-6 && (b - to).abs() < 1e-6)
}

fn end(m: &Motion) -> f64 {
    m.start + m.duration
}

fn find_in<'a, 'b>(layers: &'b [ResolvedLayer<'a>], id: &str, acc: f32) -> Option<f32> {
    for l in layers {
        let o = acc * l.opacity;
        if l.id == id {
            return Some(o);
        }
        if let Some(f) = find_in(&l.children, id, o) {
            return Some(f);
        }
    }
    None
}

/// Effective opacity of a layer at scene-local time `t` (0 when culled).
fn opacity(p: &MotionProject, id: &str, t: f64) -> f32 {
    let start = scene(p, "beat_1").start_seconds;
    let frame = ((start + t) * FPS).round() as u32;
    let f = evaluate_frame(p, frame).expect("frame evaluates");
    find_in(&f.layers, id, 1.0).unwrap_or(0.0)
}

fn all_opacity(p: &MotionProject, ids: &[String], t: f64) -> Vec<f32> {
    ids.iter().map(|id| opacity(p, id, t)).collect()
}

fn duration(p: &MotionProject) -> f64 {
    scene(p, "beat_1").duration_seconds
}

/// Row timing read back from the compiled motions: when the old state has
/// fully landed, when the replace starts, and when the new state finished.
struct Timing {
    landed: f64,
    replace_at: f64,
    replace_end: f64,
}

fn timing(p: &MotionProject, row: &str) -> Timing {
    let from = units(p, &format!("b1.{row}.from"));
    let to = units(p, &format!("b1.{row}.to"));
    assert!(!from.is_empty() && !to.is_empty(), "{row}: units missing");
    let mut landed: f64 = 0.0;
    let mut replace_at = f64::MAX;
    for id in &from {
        for m in motions(p, id) {
            if is_fade(m, 0.0, 1.0) {
                landed = landed.max(end(m));
            }
            if is_fade(m, 1.0, 0.0) {
                replace_at = replace_at.min(m.start);
            }
        }
    }
    let replace_end = to
        .iter()
        .flat_map(|id| motions(p, id).map(end).collect::<Vec<_>>())
        .fold(0.0, f64::max);
    Timing {
        landed,
        replace_at,
        replace_end,
    }
}

// ---------------------------------------------------------------------------
// Single change
// ---------------------------------------------------------------------------

#[test]
fn single_layers_exist() {
    let p = compile_single();
    for id in [
        "b1.sc0.tag",
        "b1.sc0.panel",
        "b1.sc0.was",
        "b1.sc0.strike",
        "b1.sc0.meaning",
        "b1.sc0.meaning_text",
    ] {
        assert!(has(&p, id), "{id} missing");
    }
    assert_eq!(units(&p, "b1.sc0.from").len(), 2, "three | days");
    assert_eq!(units(&p, "b1.sc0.to").len(), 2, "same | day");
    let LayerKind::Text(tag) = &layer(&p, "b1.sc0.tag").kind else {
        panic!("tag is text")
    };
    assert_eq!(
        tag.font_role,
        motion_core::scene::FontRole::Mono,
        "LABEL voice"
    );
    assert!(!has(&p, "b1.sc1.tag"), "single change has one row");
}

#[test]
fn single_from_is_read_then_replaced_by_to() {
    let p = compile_single();
    let from = units(&p, "b1.sc0.from");
    let to = units(&p, "b1.sc0.to");
    let t = timing(&p, "sc0");
    assert!(
        t.replace_at - t.landed >= 0.68,
        "old state holds {:.2}s after landing",
        t.replace_at - t.landed
    );
    // Old state fully visible right before the replace, new one absent.
    for o in all_opacity(&p, &from, t.replace_at - 0.03) {
        assert!(o > 0.95, "from visible before replace, got {o}");
    }
    for o in all_opacity(&p, &to, t.replace_at - 0.03) {
        assert!(o < 0.02, "to hidden before replace, got {o}");
    }
    // After the replace: new visible, old gone (its reduced copy remains).
    let last = t.replace_end + 0.05;
    for o in all_opacity(&p, &to, last) {
        assert!(o > 0.95, "to visible after replace, got {o}");
    }
    for o in all_opacity(&p, &from, last) {
        assert!(o < 0.02, "from gone after replace, got {o}");
    }
}

#[test]
fn single_reduced_old_state_and_meaning_land_after_the_replace() {
    let p = compile_single();
    let t = timing(&p, "sc0");
    let was_start = motions(&p, "b1.sc0.was")
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    assert!(
        was_start >= t.replace_at,
        "reduced copy appears with the replace"
    );
    assert!(opacity(&p, "b1.sc0.was", t.replace_at - 0.05) < 0.02);
    let strike_start = motions(&p, "b1.sc0.strike")
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    assert!(strike_start > was_start, "strike follows the reduced copy");
    // Meaning lands last: after every new-state unit began arriving.
    let to_first = units(&p, "b1.sc0.to")
        .iter()
        .flat_map(|id| motions(&p, id).map(|m| m.start).collect::<Vec<_>>())
        .fold(f64::MAX, f64::min);
    let meaning_start = motions(&p, "b1.sc0.meaning")
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    assert!(
        meaning_start > to_first,
        "meaning after the new state starts"
    );
    // Everything is on screen at the last frame before exit.
    let at = duration(&p) - 1.0 / FPS;
    for id in ["b1.sc0.was", "b1.sc0.meaning_text"] {
        assert!(opacity(&p, id, at) > 0.95, "{id} visible at the end");
    }
    for id in units(&p, "b1.sc0.to") {
        assert!(opacity(&p, &id, at) > 0.95, "{id} visible at the end");
    }
    assert!(!motions(&p, "b1.sc0.strike").any(|m| end(m) > at));
}

#[test]
fn meaning_is_optional() {
    let beat = json!({
        "purpose": "emphasize",
        "statement": "Delivery got faster.",
        "primary": change("delivery", "three days", "same day", None),
    });
    let p = build(&intent(vec![beat], "vertical"), "auto");
    assert!(!has(&p, "b1.sc0.meaning"));
    assert!(has(&p, "b1.sc0.was") && has(&p, "b1.sc0.panel"));
}

#[test]
fn other_subject_becomes_a_caption() {
    let beat = json!({
        "purpose": "emphasize",
        "statement": "Delivery got faster.",
        "primary": { "kind": "phrase", "value": "Every order" },
        "secondary": change("delivery", "three days", "same day", None),
    });
    let p = build(&intent(vec![beat], "vertical"), "auto");
    let LayerKind::Text(t) = &layer(&p, "b1.sc0.caption").kind else {
        panic!("caption is text")
    };
    assert_eq!(t.text, "Every order");
    // Caption sits under the panel.
    let (panel, cap) = (layer(&p, "b1.sc0.panel"), layer(&p, "b1.sc0.caption"));
    assert!(cap.y >= panel.y + panel.height);
}

// ---------------------------------------------------------------------------
// Dual change
// ---------------------------------------------------------------------------

#[test]
fn dual_has_two_identical_rows() {
    let p = compile_dual();
    let names = ["tag", "panel", "was", "strike"];
    for row in ["sc0", "sc1"] {
        for n in names {
            assert!(has(&p, &format!("b1.{row}.{n}")), "{row}.{n} missing");
        }
    }
    assert_eq!(units(&p, "b1.sc0.from").len(), 1);
    assert_eq!(units(&p, "b1.sc1.from").len(), 1);
    assert_eq!(units(&p, "b1.sc0.to").len(), 1);
    assert_eq!(units(&p, "b1.sc1.to").len(), 1);
    let (p0, p1) = (layer(&p, "b1.sc0.panel"), layer(&p, "b1.sc1.panel"));
    assert_eq!(
        (p0.width, p0.height, p0.x),
        (p1.width, p1.height, p1.x),
        "same panel size"
    );
    let text = |id: &str| match &layer(&p, id).kind {
        LayerKind::Text(t) => t.text.clone(),
        _ => panic!("{id} not text"),
    };
    assert_eq!(text("b1.sc0.tag"), "DELIVERY");
    assert_eq!(text("b1.sc1.tag"), "COST");
    assert!(has(&p, "b1.sc.connective"));
}

#[test]
fn dual_replaces_run_in_parallel() {
    let p = compile_dual();
    let (a, b) = (timing(&p, "sc0"), timing(&p, "sc1"));
    let delay = b.replace_at - a.replace_at;
    assert!(
        (0.1..=0.2).contains(&delay),
        "secondary trails by {delay:.3}s"
    );
    assert!(
        b.replace_at < a.replace_end && a.replace_at < b.replace_end,
        "replace windows overlap: {:.2}-{:.2} vs {:.2}-{:.2}",
        a.replace_at,
        a.replace_end,
        b.replace_at,
        b.replace_end
    );
    assert!(b.landed - a.landed <= 0.2, "old states land together");
    // Mid-replace both rows are in flux at once.
    let mid = b.replace_at + 0.12;
    let from_a = units(&p, "b1.sc0.from");
    let from_b = units(&p, "b1.sc1.from");
    assert!(all_opacity(&p, &from_a, mid).iter().all(|o| *o < 0.98));
    assert!(all_opacity(&p, &from_b, mid).iter().all(|o| *o > 0.0));
}

#[test]
fn dual_rows_are_spatially_distinct_and_both_final_states_show() {
    for meanings in [false, true] {
        let p = build(&intent(vec![dual_beat(meanings)], "vertical"), "auto");
        let tag1 = layer(&p, "b1.sc1.tag");
        let mut bottom0 = {
            let panel = layer(&p, "b1.sc0.panel");
            panel.y + panel.height
        };
        if meanings {
            let pill = layer(&p, "b1.sc0.meaning");
            bottom0 = bottom0.max(pill.y + pill.height);
        }
        assert!(
            bottom0 < tag1.y,
            "row 0 ends ({bottom0}) above row 1 ({})",
            tag1.y
        );
        let (canvas_h, panel1) = (p.canvas.height as f32, layer(&p, "b1.sc1.panel"));
        assert!(panel1.y + panel1.height <= canvas_h);

        let at = duration(&p) - 1.0 / FPS;
        for row in ["sc0", "sc1"] {
            for id in units(&p, &format!("b1.{row}.to")) {
                assert!(opacity(&p, &id, at) > 0.95, "{id} visible at end");
            }
            for id in units(&p, &format!("b1.{row}.from")) {
                assert!(opacity(&p, &id, at) < 0.02, "{id} replaced");
            }
            for n in ["tag", "was"] {
                assert!(
                    opacity(&p, &format!("b1.{row}.{n}"), at) > 0.95,
                    "{row}.{n}"
                );
            }
            if meanings {
                assert!(opacity(&p, &format!("b1.{row}.meaning_text"), at) > 0.95);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// All languages, timing, fit, determinism
// ---------------------------------------------------------------------------

#[test]
fn every_language_compiles_validates_and_ends_before_exit() {
    for language in LANGUAGES {
        for beat in [single_beat(), dual_beat(true)] {
            let i = intent(vec![beat, follow_up()], "vertical");
            let p = build(&i, language);
            let s = scene(&p, "beat_1");
            let exit = s.duration_seconds - 0.55; // next beat is building
            for m in s.motions.iter().filter(|m| m.target.starts_with("b1.sc")) {
                assert!(
                    end(m) <= exit + 1e-6,
                    "{language}: {} ends {:.3} > exit {exit:.3}",
                    m.target,
                    end(m)
                );
            }
            // Old state holds >= ~0.7 s once landed, in every language.
            let t = timing(&p, "sc0");
            assert!(t.replace_at - t.landed >= 0.6, "{language}: hold");
        }
    }
}

#[test]
fn minimal_language_never_springs() {
    for beat in [single_beat(), dual_beat(true)] {
        let p = build(&intent(vec![beat], "vertical"), "minimal");
        for m in &scene(&p, "beat_1").motions {
            let e = format!("{:?}", m.easing).to_lowercase();
            assert!(!e.contains("spring"), "{}: {e}", m.target);
        }
    }
}

#[test]
fn long_states_fit_the_panel_in_every_format() {
    let long = change(
        "international delivery time",
        "eleven working days internationally",
        "supercalifragilisticexpialidocious morning",
        Some("dramatically quicker"),
    );
    let long2 = change(
        "operating cost per parcel",
        "extraordinarily expensive indeed",
        "practically negligible",
        None,
    );
    for format in ["vertical", "square", "landscape"] {
        for beats in [
            vec![
                json!({ "purpose": "emphasize", "statement": "A very long statement about how delivery changed for everybody", "primary": long.clone() }),
            ],
            vec![
                json!({ "purpose": "contrast", "statement": "Better and cheaper at once.", "primary": long.clone(), "secondary": long2.clone() }),
            ],
        ] {
            let p = build(&intent(beats, format), "auto");
            let rows = if has(&p, "b1.sc1.panel") { 2 } else { 1 };
            for r in 0..rows {
                let panel = layer(&p, &format!("b1.sc{r}.panel"));
                for run in ["from", "to"] {
                    for id in units(&p, &format!("b1.sc{r}.{run}")) {
                        let u = layer(&p, &id);
                        let (l, rt) = (u.x - u.width / 2.0, u.x + u.width / 2.0);
                        assert!(
                            l >= panel.x - 0.5 && rt <= panel.x + panel.width + 0.5,
                            "{format} {id}: x {l:.0}..{rt:.0} outside panel {:.0}..{:.0}",
                            panel.x,
                            panel.x + panel.width
                        );
                        let (t, bt) = (u.y - u.height / 2.0, u.y + u.height / 2.0);
                        assert!(
                            t >= panel.y - 0.5 && bt <= panel.y + panel.height + 0.5,
                            "{format} {id}: y {t:.0}..{bt:.0} outside panel"
                        );
                    }
                }
                let was = layer(&p, &format!("b1.sc{r}.was"));
                let tag = layer(&p, &format!("b1.sc{r}.tag"));
                assert!(
                    tag.x + tag.width <= was.x,
                    "{format}: tag and reduced state overlap"
                );
                assert!(was.x + was.width <= panel.x + panel.width + 0.5);
                if let Some(pill) =
                    find_layer(&scene(&p, "beat_1").layers, &format!("b1.sc{r}.meaning"))
                {
                    assert!(pill.x + pill.width <= panel.x + panel.width);
                }
            }
            let last = layer(&p, &format!("b1.sc{}.panel", rows - 1));
            assert!(
                last.y + last.height <= p.canvas.height as f32,
                "{format}: below canvas"
            );
        }
    }
}

#[test]
fn compile_is_deterministic() {
    for beat in [single_beat(), dual_beat(true)] {
        let i = intent(vec![beat, follow_up()], "vertical");
        let a = serde_json::to_string(&build(&i, "auto")).expect("serializes");
        let b = serde_json::to_string(&build(&i, "auto")).expect("serializes");
        assert_eq!(a, b);
    }
}

fn lifecycle(p: &MotionProject) -> motion_core::scene::Lifecycle {
    scene(p, "beat_1")
        .lifecycle
        .expect("compiled scenes carry a lifecycle")
}

#[test]
fn from_is_readable_before_the_replace_fires_in_evolve() {
    let p = build(
        &intent(vec![single_beat(), follow_up()], "vertical"),
        "auto",
    );
    let life = lifecycle(&p);
    let t = timing(&p, "sc0");
    assert!(
        t.replace_at >= life.read + 0.5,
        "replace at {} vs read {}",
        t.replace_at,
        life.read
    );
    assert!(t.replace_at >= life.evolve - 1e-9);
    assert!(t.replace_at - t.landed >= 0.5, "from readable >= 0.5s");
    // The reduced old state and the meaning follow at later events, all
    // starting before ANTICIPATE.
    let start = |id: &str| motions(&p, id).map(|m| m.start).fold(f64::MAX, f64::min);
    let (was, meaning) = (start("b1.sc0.was"), start("b1.sc0.meaning"));
    assert!(was > t.replace_at && meaning > was);
    assert!(meaning < life.anticipate);
}

#[test]
fn dual_rows_both_hold_from_until_evolve() {
    let p = build(
        &intent(vec![dual_beat(true), follow_up()], "vertical"),
        "auto",
    );
    let life = lifecycle(&p);
    for row in ["sc0", "sc1"] {
        let t = timing(&p, row);
        assert!(t.replace_at >= life.evolve - 1e-9, "{row}");
        assert!(t.replace_at - t.landed >= 0.5, "{row}: from readable");
        assert!(t.replace_at < life.anticipate, "{row}");
    }
    let (a, b) = (timing(&p, "sc0"), timing(&p, "sc1"));
    assert!(b.replace_at > a.replace_at, "second row trails the first");
}
