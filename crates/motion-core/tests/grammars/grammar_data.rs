//! DataStory grammar: number pair, number series and number reveal.

use std::path::Path;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");

fn build(beats: Vec<Value>, style: &StyleProfile) -> MotionProject {
    let intent: CreativeIntent = serde_json::from_value(json!({
        "version": "0.2",
        "title": "data_story",
        "beats": beats,
    }))
    .expect("intent parses");
    let p = compile(&intent, style, &AssetLibrary::new(ASSETS), &ApproxMeasure).expect("compiles");
    validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("validation failed:\n{e}"));
    p
}

fn flat<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            flat(children, out);
        }
    }
}

fn layers_of(s: &Scene) -> Vec<&Layer> {
    let mut out = Vec::new();
    flat(&s.layers, &mut out);
    out
}

/// The scene that holds beat `n` (1-based) layers.
fn beat_scene(p: &MotionProject, n: usize) -> &Scene {
    let prefix = format!("b{n}.");
    p.scenes
        .iter()
        .find(|s| layers_of(s).iter().any(|l| l.id.starts_with(&prefix)))
        .unwrap_or_else(|| panic!("no scene for beat {n}"))
}

fn layer<'a>(s: &'a Scene, part: &str) -> Option<&'a Layer> {
    layers_of(s).into_iter().find(|l| l.id.contains(part))
}

fn motions_on<'a>(s: &'a Scene, id: &str) -> Vec<&'a Motion> {
    s.motions.iter().filter(|m| m.target == id).collect()
}

fn text_of(l: &Layer) -> Option<&str> {
    match &l.kind {
        LayerKind::Text(t) => Some(t.text.as_str()),
        _ => None,
    }
}

fn texts(s: &Scene) -> Vec<String> {
    layers_of(s)
        .into_iter()
        .filter_map(|l| text_of(l).map(str::to_string))
        .collect()
}

fn number(v: &str, meaning: &str) -> Value {
    json!({ "kind": "number", "value": v, "meaning": meaning })
}

fn pair(a: (&str, &str), b: (&str, &str)) -> Value {
    json!({
        "purpose": "compare",
        "statement": "Same shop, two months.",
        "primary": number(a.0, a.1),
        "secondary": number(b.0, b.1),
    })
}

fn series(values: &[(&str, &str)]) -> Value {
    let items: Vec<Value> = values.iter().map(|(v, m)| number(v, m)).collect();
    json!({
        "purpose": "emphasize",
        "statement": "Signups climbed every quarter.",
        "primary": { "kind": "collection", "meaning": "signups", "items": items },
    })
}

fn reveal(value: &str) -> Value {
    json!({
        "purpose": "reveal",
        "statement": "Most people finish the course.",
        "primary": number(value, "completion rate"),
    })
}

const QUARTERS: [(&str, &str); 4] = [("120", "Q1"), ("180", "Q2"), ("260", "Q3"), ("410", "Q4")];

/// Nothing new starts at/after anticipate except the stage wrapper (exit).
fn assert_nothing_new_after_anticipate(s: &Scene) {
    let a = s.lifecycle.expect("lifecycle").anticipate;
    for m in &s.motions {
        if m.target.contains(".stage") {
            continue;
        }
        assert!(
            m.start < a + 1e-6,
            "{} ({:?}) starts at {} >= anticipate {a}",
            m.target,
            m.op,
            m.start
        );
    }
}

fn assert_ends_by_duration(s: &Scene) {
    for m in &s.motions {
        assert!(
            m.start + m.duration <= s.duration_seconds + 1e-6,
            "{} ends {} after duration {}",
            m.target,
            m.start + m.duration,
            s.duration_seconds
        );
    }
}

fn count_target(m: &Motion) -> Option<f64> {
    match &m.op {
        MotionOp::Count { to, .. } => Some(*to),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// NumberPair
// ---------------------------------------------------------------------------

#[test]
fn number_pair_counts_both_and_compares_in_evolve() {
    let p = build(
        vec![pair(("₹42,000", "March"), ("₹18,500", "February"))],
        &StyleProfile::default(),
    );
    let s = beat_scene(&p, 1);
    let life = s.lifecycle.expect("lifecycle");

    let counts: Vec<(&Motion, f64)> = s
        .motions
        .iter()
        .filter_map(|m| count_target(m).map(|to| (m, to)))
        .collect();
    let primary = counts
        .iter()
        .find(|(_, to)| *to == 42000.0)
        .expect("primary counts");
    let secondary = counts
        .iter()
        .find(|(_, to)| *to == 18500.0)
        .expect("secondary counts");
    assert!(primary.0.start < life.settle + 0.5, "primary in ENTER");
    assert!(
        secondary.0.start >= life.evolve - 1e-6,
        "secondary counts from EVOLVE"
    );

    // Comparison bars: accent_expand on both fills, from EVOLVE on.
    for row in ["a", "b"] {
        let id = format!("b1.compare.{row}.fill");
        let ms = motions_on(s, &id);
        assert!(
            ms.iter()
                .any(|m| matches!(m.op, MotionOp::AccentExpand { .. })),
            "{id} grows"
        );
        assert!(
            ms.iter().all(|m| m.start >= life.evolve - 1e-6),
            "{id} starts in EVOLVE"
        );
    }
    // Direction line: engine-computed ratio 42000 / 18500 = 2.27.
    let all = texts(s);
    assert!(
        all.iter().any(|t| t.contains("2.3× larger")),
        "direction line missing: {all:?}"
    );
    let dir = layer(s, "direction").expect("direction layer");
    assert!(
        motions_on(s, &dir.id)
            .iter()
            .all(|m| m.start >= life.evolve - 1e-6),
        "direction line arrives in EVOLVE"
    );
    assert_nothing_new_after_anticipate(s);
    assert_ends_by_duration(s);
}

#[test]
fn percent_pair_states_the_point_difference() {
    let p = build(
        vec![pair(("40%", "before"), ("65%", "after"))],
        &StyleProfile::default(),
    );
    let s = beat_scene(&p, 1);
    let all = texts(s);
    assert!(
        all.iter().any(|t| t.contains("25 points higher")),
        "difference missing: {all:?}"
    );
    assert!(layer(s, "compare.a.fill").is_some());
    assert_nothing_new_after_anticipate(s);
}

#[test]
fn pair_with_mismatched_units_still_compiles_without_a_direction_line() {
    let p = build(
        vec![pair(("₹500", "price"), ("40%", "discount"))],
        &StyleProfile::default(),
    );
    let s = beat_scene(&p, 1);
    assert!(layer(s, "compare.a.fill").is_some(), "bars still draw");
    assert!(layer(s, "direction").is_none(), "no invented comparison");
    assert_nothing_new_after_anticipate(s);
    assert_ends_by_duration(s);
}

// ---------------------------------------------------------------------------
// NumberSeries
// ---------------------------------------------------------------------------

#[test]
fn number_series_grows_bars_then_draws_the_trend() {
    let p = build(vec![series(&QUARTERS)], &StyleProfile::default());
    let s = beat_scene(&p, 1);
    let life = s.lifecycle.expect("lifecycle");

    // One bar per item, each growing with accent_expand; bars start in order.
    let mut starts = Vec::new();
    for i in 0..QUARTERS.len() {
        let id = format!("b1.chart.bar.{i}");
        assert!(layer(s, &id).is_some(), "{id} exists");
        let grow = motions_on(s, &id)
            .into_iter()
            .find(|m| matches!(m.op, MotionOp::AccentExpand { .. }))
            .unwrap_or_else(|| panic!("{id} has no accent_expand"));
        starts.push(grow.start);
    }
    assert!(
        starts.windows(2).all(|w| w[0] < w[1]),
        "bars grow one after another: {starts:?}"
    );
    assert!(
        starts[1..].iter().all(|t| *t >= life.evolve - 1e-6),
        "later bars grow from EVOLVE: {starts:?}"
    );
    assert!(starts[0] >= life.settle - 1e-6, "first bar from SETTLE");

    // The trend line: a polyline traced with a trim, on the last evolve event.
    let line = layer(s, "trend.line").expect("polyline layer");
    assert!(matches!(line.kind, LayerKind::Polyline { .. }));
    let trim = motions_on(s, &line.id)
        .into_iter()
        .find(|m| matches!(m.op, MotionOp::Trim { .. }))
        .expect("trim motion");
    let last_event = *life
        .evolve_events(QUARTERS.len())
        .last()
        .expect("events exist");
    assert!(
        (trim.start - last_event).abs() < 0.01,
        "line at last event {last_event}, got {}",
        trim.start
    );
    assert!(trim.start > *starts.last().expect("bars") - 1e-6);

    // Labels carry the values.
    let all = texts(s);
    for (v, _) in QUARTERS {
        assert!(all.iter().any(|t| t == v), "value label {v}: {all:?}");
    }
    // Peak (last) value is emphasized with a scale pulse before anticipate.
    let peak = layer(s, "value.3").expect("peak label");
    let pulse = motions_on(s, &peak.id)
        .into_iter()
        .filter(|m| matches!(m.op, MotionOp::Scale { .. }))
        .collect::<Vec<_>>();
    assert!(!pulse.is_empty(), "peak label pulses");
    assert!(pulse.iter().all(|m| m.start < life.anticipate));
    assert_nothing_new_after_anticipate(s);
    assert_ends_by_duration(s);
}

#[test]
fn series_line_traces_through_the_bar_tops() {
    let p = build(vec![series(&QUARTERS)], &StyleProfile::default());
    let s = beat_scene(&p, 1);
    let line = layer(s, "trend.line").expect("polyline");
    let LayerKind::Polyline { points, .. } = &line.kind else {
        panic!("polyline expected");
    };
    assert_eq!(points.len(), QUARTERS.len());
    for (i, pt) in points.iter().enumerate() {
        let bar = layer(s, &format!("b1.chart.bar.{i}")).expect("bar");
        let top = motions_on(s, &bar.id)
            .into_iter()
            .find_map(|m| match &m.op {
                MotionOp::AccentExpand { to } => Some(*to),
                _ => None,
            })
            .expect("bar target");
        let (cx, cy) = (line.x + pt[0], line.y + pt[1]);
        assert!(
            (cx - (top.x + top.width / 2.0)).abs() < 1.5,
            "bar {i}: line x {cx} vs bar center {}",
            top.x + top.width / 2.0
        );
        assert!(
            (cy - top.y).abs() < 1.5,
            "bar {i}: line y {cy} vs bar top {}",
            top.y
        );
    }
}

#[test]
fn six_bar_series_and_flat_series_compile() {
    let six = [
        ("10", "Jan"),
        ("40", "Feb"),
        ("25", "Mar"),
        ("90", "Apr"),
        ("60", "May"),
        ("75", "Jun"),
    ];
    // (0.22) A series over time (meanings that are times); named things are
    // ranked bars (tests/ranked_bars.rs).
    let flat = [("5", "2021"), ("5", "2022"), ("5", "2023")];
    for values in [&six[..], &flat[..]] {
        for lang in [MotionLanguageChoice::Auto, MotionLanguageChoice::Minimal] {
            let p = build(vec![series(values)], &lang.style());
            let s = beat_scene(&p, 1);
            assert!(layer(s, "trend.line").is_some());
            assert_nothing_new_after_anticipate(s);
            assert_ends_by_duration(s);
        }
    }
}

enum MotionLanguageChoice {
    Auto,
    Minimal,
}

impl MotionLanguageChoice {
    fn style(&self) -> StyleProfile {
        use motion_core::style::MotionLanguage;
        StyleProfile {
            motion_language: match self {
                MotionLanguageChoice::Auto => MotionLanguage::Auto,
                MotionLanguageChoice::Minimal => MotionLanguage::Minimal,
            },
            ..StyleProfile::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Atomic number reveal
// ---------------------------------------------------------------------------

#[test]
fn percent_reveal_counts_then_fills_a_progress_bar() {
    let p = build(vec![reveal("78%")], &StyleProfile::default());
    let s = beat_scene(&p, 1);
    let life = s.lifecycle.expect("lifecycle");
    let count = s
        .motions
        .iter()
        .find(|m| count_target(m) == Some(78.0))
        .expect("hero counts to 78");
    assert!(count.start < life.evolve, "the number counts in ENTER");
    // Meaning line and progress bar arrive in EVOLVE.
    let label = layer(s, "label").expect("meaning label");
    assert!(motions_on(s, &label.id)
        .iter()
        .all(|m| m.start >= life.evolve - 1e-6));
    let fill = motions_on(s, "b1.progress.fill");
    assert!(fill
        .iter()
        .any(|m| matches!(m.op, MotionOp::AccentExpand { .. }) && m.start >= life.evolve - 1e-6));
    assert_nothing_new_after_anticipate(s);
    assert_ends_by_duration(s);
}

#[test]
fn plain_number_reveal_has_no_progress_bar() {
    let p = build(vec![reveal("1,200")], &StyleProfile::default());
    let s = beat_scene(&p, 1);
    assert!(layer(s, "progress").is_none());
    assert!(layer(s, "accent_slab").is_some());
    assert!(s.motions.iter().any(|m| count_target(m) == Some(1200.0)));
    assert_nothing_new_after_anticipate(s);
    let over = build(vec![reveal("140%")], &StyleProfile::default());
    assert!(layer(beat_scene(&over, 1), "progress").is_none());
}

#[test]
fn counters_count_in_every_language() {
    use motion_core::style::MotionLanguage;
    let style = StyleProfile {
        motion_language: MotionLanguage::Minimal,
        ..StyleProfile::default()
    };
    let p = build(
        vec![
            pair(("₹42,000", "March"), ("₹18,500", "February")),
            reveal("78%"),
        ],
        &style,
    );
    let pair_scene = beat_scene(&p, 1);
    let counted: Vec<f64> = pair_scene.motions.iter().filter_map(count_target).collect();
    assert!(counted.contains(&42000.0) && counted.contains(&18500.0));
    let reveal_scene = beat_scene(&p, 2);
    assert!(reveal_scene
        .motions
        .iter()
        .any(|m| count_target(m) == Some(78.0)));
}

#[test]
fn data_story_beats_are_deterministic() {
    let beats = || {
        vec![
            pair(("₹42,000", "March"), ("₹18,500", "February")),
            series(&QUARTERS),
            reveal("78%"),
        ]
    };
    let a = build(beats(), &StyleProfile::default());
    let b = build(beats(), &StyleProfile::default());
    assert_eq!(
        serde_json::to_string(&a).expect("serializes"),
        serde_json::to_string(&b).expect("serializes")
    );
}

/// A revealed number carried into the next beat becomes a SharedElement whose
/// colour persists across backgrounds, so it is set in ink (never the light
/// on-accent colour) and counts on the shared layer itself.
#[test]
fn carried_reveal_number_is_ink_and_counts_as_shared_element() {
    let mut first = reveal("₹4,200");
    first["continuity"] = json!("carry_primary");
    let second = json!({
        "purpose": "emphasize",
        "statement": "Most of it was the cooler.",
        "primary": {"kind": "number", "value": "₹4,200", "meaning": "bill"},
        "energy": "building"
    });
    let p = build(vec![first, second], &StyleProfile::default());
    assert_eq!(p.shared.len(), 1, "one shared element");
    let el = &p.shared[0];
    let LayerKind::Text(style) = &el.layer.kind else {
        panic!("shared number is text")
    };
    assert_eq!(
        style.color.to_hex(),
        p.theme.palette["ink"].to_hex(),
        "carried number uses ink"
    );
    let counts = p
        .scenes
        .iter()
        .flat_map(|s| &s.motions)
        .any(|m| m.target == el.layer.id && matches!(m.op, MotionOp::Count { .. }));
    assert!(counts, "the shared number counts");
}
