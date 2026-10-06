//! Collection composition: items stay identifiable, arrive in order, and
//! (with `accumulate`) build visibly into one aggregate.

use std::path::Path;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::{MotionLanguage, StyleProfile};
use motion_core::timeline::format_count;
use motion_core::validate::validate;
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");

fn phrases(names: &[&str]) -> Vec<Value> {
    names
        .iter()
        .map(|n| json!({ "kind": "phrase", "value": n }))
        .collect()
}

fn drinks(n: usize) -> Value {
    let all = ["Coffee", "Tea", "Juice", "Water", "Soda", "Milk"];
    json!({ "kind": "collection", "meaning": "drinks", "items": phrases(&all[..n]) })
}

fn intent(primary: Value, secondary: Option<Value>, rel: Option<&str>, purpose: &str) -> Value {
    let mut beat = json!({
        "purpose": purpose,
        "statement": "Every drink adds up.",
        "primary": primary,
    });
    if let Some(s) = secondary {
        beat["secondary"] = s;
    }
    if let Some(r) = rel {
        beat["relationship"] = json!(r);
    }
    json!({ "version": "0.2", "title": "collection_test", "beats": [beat] })
}

fn build(v: Value, style: &StyleProfile) -> MotionProject {
    let intent: CreativeIntent = serde_json::from_value(v).expect("intent parses");
    compile(&intent, style, &AssetLibrary::new(ASSETS), &ApproxMeasure).expect("compiles")
}

fn build_default(v: Value) -> MotionProject {
    build(v, &StyleProfile::default())
}

fn beat_scene(p: &MotionProject) -> &Scene {
    p.scenes.iter().find(|s| s.id == "beat_1").expect("beat_1")
}

fn flatten<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            flatten(children, out);
        }
    }
}

fn all_layers(s: &Scene) -> Vec<&Layer> {
    let mut v = Vec::new();
    flatten(&s.layers, &mut v);
    v
}

fn layer<'a>(s: &'a Scene, id: &str) -> Option<&'a Layer> {
    all_layers(s).into_iter().find(|l| l.id == id)
}

fn text_of(l: &Layer) -> &str {
    match &l.kind {
        LayerKind::Text(t) => &t.text,
        _ => "",
    }
}

fn motions_of<'a>(s: &'a Scene, target: &str, op: &str) -> Vec<&'a Motion> {
    let mut v: Vec<&Motion> = s
        .motions
        .iter()
        .filter(|m| m.target == target && m.op.op_name() == op)
        .collect();
    v.sort_by(|a, b| a.start.total_cmp(&b.start));
    v
}

/// `(from, to)` of each count motion on `target`, in time order.
fn counts(s: &Scene, target: &str) -> Vec<(f64, f64)> {
    motions_of(s, target, "count")
        .iter()
        .map(|m| match &m.op {
            MotionOp::Count { from, to, .. } => (*from, *to),
            _ => unreachable!(),
        })
        .collect()
}

fn card_start(s: &Scene, i: usize) -> f64 {
    let id = format!("b1.items.{i}.card");
    motions_of(s, &id, "mask_reveal")[0].start
}

fn accumulate_intent(items: Value, secondary: Option<Value>) -> Value {
    intent(items, secondary, Some("accumulate"), "emphasize")
}

#[test]
fn one_card_per_item_with_its_text() {
    let p = build_default(accumulate_intent(drinks(4), None));
    let s = beat_scene(&p);
    for (i, name) in ["COFFEE", "TEA", "JUICE", "WATER"].iter().enumerate() {
        let card = layer(s, &format!("b1.items.{i}.card")).expect("card");
        assert!(matches!(card.kind, LayerKind::RoundedRectangle { .. }));
        let content = layer(s, &format!("b1.items.{i}.content")).expect("content");
        assert_eq!(text_of(content).to_uppercase(), *name);
        let bind = content.layout.as_ref().expect("content is layout-bound");
        assert_eq!(bind.parent, format!("b1.items.{i}.card"));
    }
    assert!(layer(s, "b1.items.4.card").is_none());
}

#[test]
fn items_arrive_in_order_at_least_0_3s_apart() {
    for n in [2, 4, 6] {
        let p = build_default(accumulate_intent(drinks(n), None));
        let s = beat_scene(&p);
        let starts: Vec<f64> = (0..n).map(|i| card_start(s, i)).collect();
        for w in starts.windows(2) {
            assert!(w[1] - w[0] >= 0.3 - 1e-9, "n={n}: gap {:?}", w);
        }
    }
}

#[test]
fn accumulate_steps_running_count_in_order() {
    let p = build_default(accumulate_intent(drinks(4), None));
    let s = beat_scene(&p);
    let ms = motions_of(s, "b1.total", "count");
    let steps = counts(s, "b1.total");
    assert_eq!(steps, vec![(0.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)]);
    for w in ms.windows(2) {
        assert!(w[0].start + w[0].duration <= w[1].start + 1e-9, "overlap");
    }
    // Each step follows its item landing.
    for (i, m) in ms.iter().enumerate() {
        assert!(m.start >= card_start(s, i));
    }
    // The bar grows in chained steps to the full track.
    let bars = motions_of(s, "b1.bar.fill", "accent_expand");
    assert_eq!(bars.len(), 4);
    let track = layer(s, "b1.bar.track").expect("track");
    if let MotionOp::AccentExpand { to } = &bars[3].op {
        assert!((to.width - track.width).abs() < 0.01);
    }
    // Aggregate text.
    let agg = layer(s, "b1.aggregate").expect("aggregate");
    assert_eq!(text_of(agg), "4 drinks");
    assert!(layer(s, "b1.aggregate.slab").is_some());
}

#[test]
fn numbers_of_one_unit_sum_exactly() {
    let items = json!({
        "kind": "collection",
        "items": [
            { "kind": "number", "value": "₹1,200", "meaning": "rent" },
            { "kind": "number", "value": "₹350", "meaning": "food" },
            { "kind": "number", "value": "₹99.50", "meaning": "phone" }
        ]
    });
    let p = build_default(accumulate_intent(items, None));
    let s = beat_scene(&p);
    let expected = format_count(1649.5, 2, true, "₹", "");
    assert_eq!(expected, "₹1,649.50");
    assert_eq!(text_of(layer(s, "b1.aggregate").unwrap()), expected);
    let steps = counts(s, "b1.total");
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[0], (0.0, 1200.0));
    assert!((steps[2].1 - 1649.5).abs() < 1e-9);
    assert_eq!(steps[1].0, steps[0].1);
    // Number item + meaning label.
    let label = layer(s, "b1.items.0.label").expect("label");
    assert_eq!(text_of(label).to_uppercase(), "RENT");
}

#[test]
fn mixed_units_fall_back_to_count() {
    let items = json!({
        "kind": "collection",
        "meaning": "costs",
        "items": [
            { "kind": "number", "value": "₹100" },
            { "kind": "number", "value": "5%" }
        ]
    });
    let p = build_default(accumulate_intent(items, None));
    let s = beat_scene(&p);
    assert_eq!(text_of(layer(s, "b1.aggregate").unwrap()), "2 costs");
}

#[test]
fn consequence_secondary_wins_over_computed_total() {
    let sec = json!({ "kind": "number", "value": "₹2,400", "meaning": "a year" });
    let p = build_default(accumulate_intent(drinks(4), Some(sec)));
    let s = beat_scene(&p);
    let agg = layer(s, "b1.aggregate").expect("aggregate");
    let c = counts(s, "b1.aggregate");
    assert_eq!(c, vec![(0.0, 2400.0)]);
    assert!(matches!(&agg.kind, LayerKind::Text(_)));
    // The running total still counts items.
    assert_eq!(counts(s, "b1.total").last().copied(), Some((3.0, 4.0)));
    let mut texts = String::new();
    for l in all_layers(s) {
        texts.push_str(text_of(l));
        texts.push('|');
    }
    assert!(!texts.contains("4 drinks"));
}

#[test]
fn collection_secondary_with_atomic_primary_is_context_not_total() {
    // An atomic primary beside a collection secondary is context (caption);
    // the accumulated total is the engine's own ("3 drinks"), never the primary.
    let primary = json!({ "kind": "phrase", "value": "Small habits" });
    let v = intent(primary, Some(drinks(3)), Some("accumulate"), "reveal");
    let p = build_default(v);
    let s = beat_scene(&p);
    assert_eq!(counts(s, "b1.total").len(), 3);
    let agg = text_of(layer(s, "b1.aggregate").unwrap())
        .to_lowercase()
        .replace('\n', " ");
    assert_eq!(agg, "3 drinks");
    let caption = text_of(layer(s, "b1.caption").unwrap()).replace('\n', " ");
    assert_eq!(caption, "Small habits");
    assert!(layer(s, "b1.items.2.card").is_some());
}

#[test]
fn non_accumulate_has_no_invented_total() {
    let p = build_default(intent(drinks(3), None, None, "emphasize"));
    let s = beat_scene(&p);
    assert!(layer(s, "b1.total").is_none());
    assert!(layer(s, "b1.aggregate").is_none());
    assert!(layer(s, "b1.bar.fill").is_none());
    assert!(layer(s, "b1.items.2.card").is_some());

    let sec = json!({ "kind": "phrase", "value": "Enough" });
    let p = build_default(intent(drinks(3), Some(sec), None, "emphasize"));
    let s = beat_scene(&p);
    assert!(layer(s, "b1.total").is_none());
    assert!(layer(s, "b1.aggregate").is_some());
}

#[test]
fn structured_secondary_becomes_a_caption() {
    let sec = json!({
        "kind": "state_change", "entity": "habit", "from": "small", "to": "big"
    });
    let p = build_default(accumulate_intent(drinks(3), Some(sec)));
    let s = beat_scene(&p);
    assert!(layer(s, "b1.caption").is_some());
    assert_eq!(text_of(layer(s, "b1.aggregate").unwrap()), "3 drinks");
}

#[test]
fn layout_stays_inside_the_canvas_and_motions_end_before_exit() {
    for format in ["vertical", "square", "landscape"] {
        for n in [2, 4, 6] {
            let mut v = accumulate_intent(drinks(n), None);
            v["format"] = json!(format);
            let p = build_default(v);
            let s = beat_scene(&p);
            let (cw, ch) = (p.canvas.width as f32, p.canvas.height as f32);
            for l in all_layers(s) {
                if l.layout.is_some() || !l.id.starts_with("b1.") {
                    continue;
                }
                if !(l.id.contains("items.") || l.id.contains("bar.") || l.id.contains("slab")) {
                    continue;
                }
                assert!(l.x >= -0.5 && l.y >= -0.5, "{format} n={n}: {}", l.id);
                assert!(l.x + l.width <= cw + 0.5, "{format} n={n}: {}", l.id);
                assert!(l.y + l.height <= ch + 0.5, "{format} n={n}: {}", l.id);
            }
            validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("{format} n={n}: {e}"));
        }
    }
}

#[test]
fn every_motion_language_compiles_and_validates() {
    use MotionLanguage::*;
    for lang in [Auto, Minimal, Kinetic, Parallax, Sequential, Data] {
        let style = StyleProfile {
            motion_language: lang,
            ..StyleProfile::default()
        };
        for v in [
            accumulate_intent(drinks(4), None),
            accumulate_intent(drinks(6), None),
            intent(drinks(2), None, None, "emphasize"),
            intent(drinks(5), None, Some("accumulate"), "reveal"),
        ] {
            let p = build(v, &style);
            validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("{lang:?}: {e}"));
            let s = beat_scene(&p);
            let exit = s.duration_seconds;
            for m in &s.motions {
                if m.target.starts_with("b1.items.")
                    || m.target.starts_with("b1.aggregate")
                    || m.target.starts_with("b1.total")
                    || m.target.starts_with("b1.bar")
                {
                    assert!(
                        m.start + m.duration <= exit + 1e-6,
                        "{lang:?}: {}",
                        m.target
                    );
                }
            }
        }
    }
}

#[test]
fn object_items_carry_their_asset() {
    let items = json!({
        "kind": "collection",
        "meaning": "baskets",
        "items": [
            { "kind": "object", "asset": "shopping_basket", "meaning": "weekly" },
            { "kind": "object", "asset": "shopping_basket", "meaning": "monthly", "value": "12" }
        ]
    });
    let p = build_default(accumulate_intent(items, None));
    validate(&p, Some(Path::new(ASSETS))).expect("valid");
    let s = beat_scene(&p);
    assert!(matches!(
        layer(s, "b1.items.0.content").unwrap().kind,
        LayerKind::Svg { .. }
    ));
    assert_eq!(text_of(layer(s, "b1.aggregate").unwrap()), "2 baskets");
}

#[test]
fn compiles_are_deterministic() {
    let v = accumulate_intent(drinks(5), None);
    assert_eq!(build_default(v.clone()), build_default(v));
}

#[test]
fn items_unfold_through_the_evolve_phase() {
    for n in [3, 4, 6] {
        let p = build_default(accumulate_intent(drinks(n), None));
        let s = beat_scene(&p);
        let life = s.lifecycle.expect("compiled scenes carry a lifecycle");
        let starts: Vec<f64> = (0..n).map(|i| card_start(s, i)).collect();
        // The first item arrives at the end of ENTER; later ones strictly later.
        assert!(starts[0] >= life.settle - 1e-9, "n={n}: first {starts:?}");
        for w in starts.windows(2) {
            assert!(w[1] > w[0], "n={n}: item order {starts:?}");
        }
        // At least two items arrive inside EVOLVE, none at or after ANTICIPATE.
        let in_evolve = starts
            .iter()
            .filter(|t| **t >= life.evolve - 1e-9 && **t < life.anticipate)
            .count();
        assert!(in_evolve >= 2, "n={n}: {in_evolve} in evolve, {starts:?}");
        assert!(starts.iter().all(|t| *t < life.anticipate), "n={n}");
        // The aggregate slab lands after the last item, still before ANTICIPATE.
        let slab = motions_of(s, "b1.aggregate.slab", "mask_reveal")[0].start;
        assert!(slab > starts[n - 1] && slab < life.anticipate, "n={n}");
        // The running total appears with the first item, never as a bare 0.
        let total_fade = motions_of(s, "b1.total", "fade")[0].start;
        assert!(total_fade >= starts[0] && total_fade < starts[1], "n={n}");
        let first_step = motions_of(s, "b1.total", "count")[0].start;
        assert!(first_step < starts[1], "n={n}");
    }
}
