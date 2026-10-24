//! Derived-metric arithmetic, formatting and composition.

use std::path::Path;

use motion_core::compiler::{
    compile, compute_metric, format_metric, ApproxMeasure, AssetLibrary, FormattedMetric,
};
use motion_core::intent::{CreativeIntent, DerivedMetric, MetricFormat, NumericTerm, Operation};
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::{MotionLanguage, StyleProfile};
use motion_core::timeline::format_count;
use motion_core::validate::validate;
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");

fn metric(n: f64, d: f64, format: MetricFormat) -> DerivedMetric {
    DerivedMetric {
        operation: Operation::Ratio,
        numerator: NumericTerm {
            value: n,
            meaning: "sales".into(),
        },
        denominator: NumericTerm {
            value: d,
            meaning: "visits".into(),
        },
        format,
        meaning: None,
    }
}

fn fmt(n: f64, d: f64, format: MetricFormat) -> FormattedMetric {
    let f = format_metric(compute_metric(&metric(n, d, format)), format);
    // A count from 0 to `shown` must land exactly on `text`.
    let landed = format_count(f.shown, f.decimals, f.grouping(), &f.prefix, &f.suffix);
    assert_eq!(landed, f.text, "count of {n}/{d} must land on its text");
    f
}

fn text(n: f64, d: f64, format: MetricFormat) -> String {
    fmt(n, d, format).text
}

#[test]
fn compute_is_plain_division() {
    let m = metric(80.0, 1000.0, MetricFormat::Percent);
    assert_eq!(compute_metric(&m), 0.08);
    assert_eq!(text(80.0, 1000.0, MetricFormat::Percent), "8%");
    let f = fmt(80.0, 1000.0, MetricFormat::Percent);
    assert_eq!((f.shown, f.decimals, f.suffix.as_str()), (8.0, 0, "%"));
}

#[test]
fn rounding_rules() {
    use MetricFormat::*;
    assert_eq!(text(800.0, 60000.0, Percent), "1.33%");
    assert_eq!(text(800.0, 60000.0, PerThousand), "13.3 per 1,000");
    assert_eq!(text(800.0, 60000.0, Decimal), "0.013");
    assert_eq!(text(1000.0, 100000.0, Percent), "1%");
    assert_eq!(text(2.0, 3.0, Percent), "66.67%");
    assert_eq!(text(1.0, 3.0, Decimal), "0.333");
    assert_eq!(text(1.0, 200000.0, Percent), "0.0005%");
    assert_eq!(text(5.0, 2.0, Percent), "250%");
    assert_eq!(text(5.0, 2.0, Decimal), "2.5");
    assert_eq!(text(123456.0, 10.0, Decimal), "12,345.6");
    assert_eq!(text(-1.0, 4.0, Percent), "-25%");
    assert_eq!(text(1.0, 8.0, Decimal), "0.125");
    // Half away from zero, symmetric for negatives.
    assert_eq!(text(1.0, 8.0, Percent), "12.5%");
    assert_eq!(text(5.0, 8000.0, Percent), "0.06%");
    assert_eq!(text(-5.0, 8000.0, Percent), "-0.06%");
    // Grouping applies to the displayed number.
    assert_eq!(text(12345.0, 10.0, Percent), "123,450%");
    // Zero is never "-0".
    assert_eq!(text(0.0, 5.0, Percent), "0%");
    assert_eq!(text(-0.0, 5.0, Percent), "0%");
    assert_eq!(text(0.0, -5.0, Percent), "0%");
    // Very small values still show two significant digits, then trim.
    assert_eq!(text(1.0, 3_000_000.0, Decimal), "0");
    assert_eq!(text(-1.0, 10_000.0, Decimal), "-0.0001");
    assert_eq!(text(4567.0, 1e7, Percent), "0.05%");
    assert_eq!(text(4567.0, 1e8, Percent), "0.0046%");
}

#[test]
fn tiny_values_extend_decimals() {
    use MetricFormat::*;
    assert_eq!(text(1.0, 20000.0, Percent), "0.01%");
    assert_eq!(text(1.0, 40000.0, Percent), "0.0025%");
    assert_eq!(text(3.0, 1_000_000.0, Percent), "0.0003%");
    assert_eq!(text(1.0, 1e9, Percent), "0%");
    let f = fmt(1.0, 200000.0, Percent);
    assert_eq!((f.shown, f.decimals), (0.0005, 4));
}

#[test]
fn formatting_is_deterministic() {
    for _ in 0..3 {
        assert_eq!(
            format_metric(2.0 / 3.0, MetricFormat::Percent),
            format_metric(2.0 / 3.0, MetricFormat::Percent)
        );
    }
    assert_eq!(
        format_metric(f64::NAN, MetricFormat::Decimal).text,
        "0",
        "non-finite input never panics"
    );
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

fn term_json(v: f64, meaning: &str) -> Value {
    json!({ "value": v, "meaning": meaning })
}

fn metric_json(n: (f64, &str), d: (f64, &str), format: &str, meaning: Option<&str>) -> Value {
    let mut m = json!({
        "kind": "derived_metric",
        "numerator": term_json(n.0, n.1),
        "denominator": term_json(d.0, d.1),
        "format": format,
    });
    if let Some(mean) = meaning {
        m["meaning"] = json!(mean);
    }
    m
}

fn intent(beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({ "version": "0.2", "title": "derived", "beats": beats }))
        .expect("intent parses")
}

fn single_beat() -> Value {
    json!({
        "purpose": "reveal",
        "statement": "Most visitors never sign up.",
        "primary": metric_json((80.0, "sign-ups"), (1000.0, "visitors"), "percent", Some("sign-up rate")),
    })
}

fn compare_beat(a: Value, b: Value) -> Value {
    json!({
        "purpose": "compare",
        "statement": "Same shop, two months.",
        "primary": a,
        "secondary": b,
    })
}

fn build(i: &CreativeIntent, style: &StyleProfile) -> MotionProject {
    let p = compile(i, style, &AssetLibrary::new(ASSETS), &ApproxMeasure).expect("compiles");
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

fn text_of(l: &Layer) -> Option<&str> {
    match &l.kind {
        LayerKind::Text(t) => Some(t.text.as_str()),
        _ => None,
    }
}

fn texts(s: &Scene) -> Vec<String> {
    layers_of(s)
        .into_iter()
        .filter_map(|l| text_of(l).map(|t| t.to_lowercase()))
        .collect()
}

fn ids(s: &Scene, part: &str) -> Vec<String> {
    layers_of(s)
        .into_iter()
        .filter(|l| l.id.contains(part))
        .map(|l| l.id.clone())
        .collect()
}

/// Count motions as `(target, motion)` with their `to`.
fn counts(s: &Scene) -> Vec<(&str, &Motion, f64)> {
    s.motions
        .iter()
        .filter_map(|m| match &m.op {
            MotionOp::Count { to, .. } => Some((m.target.as_str(), m, *to)),
            _ => None,
        })
        .collect()
}

fn beat_scene(p: &MotionProject) -> &Scene {
    &p.scenes[1]
}

fn assert_inside_beat(p: &MotionProject) {
    for (i, s) in p.scenes.iter().enumerate().skip(1) {
        let next = p.scenes.get(i + 1).map(|n| n.start_seconds);
        let exit = next.map_or(s.duration_seconds, |n| n - s.start_seconds);
        for m in &s.motions {
            // The shared skeleton (ghost drift, stage exit) is not ours.
            if m.target.contains(".stage") || m.target.contains(".ghost") {
                continue;
            }
            assert!(
                m.start + m.duration <= exit + 1e-6,
                "{} ends {} after exit {exit}",
                m.target,
                m.start + m.duration
            );
        }
    }
}

#[test]
fn single_metric_shows_fraction_and_result() {
    let p = build(&intent(vec![single_beat()]), &StyleProfile::default());
    let s = beat_scene(&p);
    let all = texts(s);
    for want in ["sign-ups", "visitors", "="] {
        assert!(
            all.iter().any(|t| t.contains(want)),
            "missing {want}: {all:?}"
        );
    }
    let cs = counts(s);
    let by = |suffix: &str| {
        cs.iter()
            .find(|(t, _, _)| t.ends_with(suffix))
            .unwrap_or_else(|| panic!("no count on {suffix}: {cs:?}"))
    };
    assert_eq!(by("frac.num").2, 80.0);
    assert_eq!(by("frac.den").2, 1000.0);
    let result = by("result");
    assert_eq!(result.2, 8.0);
    match &result.1.op {
        MotionOp::Count {
            from,
            decimals,
            suffix,
            ..
        } => assert_eq!((*from, *decimals, suffix.as_str()), (0.0, 0, "%")),
        _ => unreachable!(),
    }
    // Result lands last: it starts after both terms started and ends last.
    let end = |m: &Motion| m.start + m.duration;
    for suffix in ["frac.num", "frac.den"] {
        let term = by(suffix).1;
        assert!(result.1.start > term.start);
        assert!(end(result.1) >= end(term));
    }
    assert_inside_beat(&p);
}

#[test]
fn single_percent_has_progress_bar_others_do_not() {
    let p = build(&intent(vec![single_beat()]), &StyleProfile::default());
    let s = beat_scene(&p);
    assert_eq!(ids(s, "progress.track").len(), 1);
    assert_eq!(ids(s, "progress.fill").len(), 1);

    for (n, d, format) in [
        (80.0, 1000.0, "decimal"),
        (80.0, 1000.0, "per_thousand"),
        (2000.0, 1000.0, "percent"),
    ] {
        let beat = json!({
            "purpose": "emphasize",
            "statement": "A ratio.",
            "primary": metric_json((n, "a"), (d, "b"), format, None),
        });
        let p = build(&intent(vec![beat]), &StyleProfile::default());
        assert!(
            ids(beat_scene(&p), "progress").is_empty(),
            "{format} {n}/{d}"
        );
    }
}

#[test]
fn atomic_subject_becomes_caption() {
    let beat = json!({
        "purpose": "explain",
        "statement": "Most visitors never sign up.",
        "primary": metric_json((80.0, "sign-ups"), (1000.0, "visitors"), "percent", None),
        "secondary": { "kind": "phrase", "value": "Measured over January" },
    });
    let p = build(&intent(vec![beat]), &StyleProfile::default());
    assert!(texts(beat_scene(&p))
        .iter()
        .any(|t| t.contains("measured over january")));
    assert_inside_beat(&p);
}

fn conclusion_of(s: &Scene) -> String {
    layers_of(s)
        .into_iter()
        .find(|l| l.id.ends_with(".conclusion"))
        .and_then(text_of)
        .expect("conclusion text")
        .to_lowercase()
        .replace('\n', " ")
}

#[test]
fn comparison_structure() {
    // 1% -> 1.33%: rate higher, numerator fewer.
    let a = metric_json(
        (1000.0, "sales"),
        (100000.0, "visits"),
        "percent",
        Some("conversion"),
    );
    let b = metric_json((800.0, "sales"), (60000.0, "visits"), "percent", None);
    let p = build(&intent(vec![compare_beat(a, b)]), &StyleProfile::default());
    let s = beat_scene(&p);
    let cs = counts(s);
    assert_eq!(cs.len(), 2, "two result counters: {cs:?}");
    assert!(cs
        .iter()
        .any(|c| c.0.ends_with("row.0.result") && c.2 == 1.0));
    assert!(cs
        .iter()
        .any(|c| c.0.ends_with("row.1.result") && c.2 == 1.33));
    for part in ["a.track", "a.fill", "b.track", "b.fill"] {
        assert_eq!(ids(s, &format!("bars.{part}")).len(), 1, "{part}");
    }
    let c = conclusion_of(s);
    assert_eq!(c, "fewer sales · higher conversion");
    // Conclusion lands last.
    let concl_start = s
        .motions
        .iter()
        .filter(|m| m.target.ends_with(".conclusion"))
        .map(|m| m.start)
        .fold(f64::MAX, f64::min);
    for m in &s.motions {
        if m.target.contains("row.") || m.target.contains("bars.") {
            assert!(
                m.start < concl_start,
                "{} starts after the conclusion",
                m.target
            );
        }
    }
    assert_inside_beat(&p);
}

#[test]
fn conclusion_words_come_from_the_numbers() {
    let conclude = |a: (f64, f64), b: (f64, f64)| {
        let a = metric_json((a.0, "sales"), (a.1, "visits"), "percent", None);
        let b = metric_json((b.0, "sales"), (b.1, "visits"), "percent", None);
        let p = build(&intent(vec![compare_beat(a, b)]), &StyleProfile::default());
        conclusion_of(beat_scene(&p))
    };
    // rate down, numerator up: "more ... · lower".
    assert_eq!(
        conclude((800.0, 60000.0), (1000.0, 100000.0)),
        "more sales · lower rate"
    );
    // rate up, numerator up: no prefix.
    assert_eq!(conclude((10.0, 1000.0), (30.0, 1000.0)), "higher rate");
    assert_eq!(conclude((30.0, 1000.0), (10.0, 1000.0)), "lower rate");
    // same rate, different numerators.
    assert_eq!(
        conclude((10.0, 1000.0), (20.0, 2000.0)),
        "more sales · same rate"
    );
    // identical.
    assert_eq!(conclude((10.0, 1000.0), (10.0, 1000.0)), "same rate");
    for words in ["worse", "better"] {
        assert!(!conclude((10.0, 1000.0), (30.0, 1000.0)).contains(words));
    }
}

#[test]
fn comparison_uses_primary_format_for_both() {
    let a = metric_json((800.0, "sales"), (60000.0, "visits"), "per_thousand", None);
    let b = metric_json((1000.0, "sales"), (100000.0, "visits"), "percent", None);
    let p = build(&intent(vec![compare_beat(a, b)]), &StyleProfile::default());
    let s = beat_scene(&p);
    for (target, m, to) in counts(s) {
        match &m.op {
            MotionOp::Count {
                suffix, decimals, ..
            } => {
                assert_eq!(suffix, " per 1,000", "{target}");
                assert!(*decimals <= 1);
                assert!(to == 13.3 || to == 10.0, "{target} {to}");
            }
            _ => unreachable!(),
        }
    }
}

#[test]
fn every_language_and_energy_compiles_and_validates() {
    let langs = [
        MotionLanguage::Auto,
        MotionLanguage::Minimal,
        MotionLanguage::Kinetic,
        MotionLanguage::Parallax,
        MotionLanguage::Sequential,
        MotionLanguage::Data,
    ];
    let a = metric_json(
        (120.0, "sales"),
        (4000.0, "visits"),
        "percent",
        Some("conversion"),
    );
    let b = metric_json((90.0, "sales"), (2000.0, "visits"), "percent", None);
    for lang in langs {
        for energy in ["calm", "building", "impact"] {
            let style = StyleProfile {
                motion_language: lang,
                ..StyleProfile::default()
            };
            let mut single = single_beat();
            single["energy"] = json!(energy);
            let mut two = compare_beat(a.clone(), b.clone());
            two["energy"] = json!(energy);
            // Two beats so the second has an incoming transition and the first an exit.
            let p = build(&intent(vec![single, two, single_beat()]), &style);
            assert_inside_beat(&p);
        }
    }
}

#[test]
fn all_formats_compile_in_every_canvas() {
    for canvas in ["vertical", "square", "landscape"] {
        for format in ["percent", "decimal", "per_thousand"] {
            let a = metric_json((800.0, "sales"), (60000.0, "visits"), format, Some("rate"));
            let b = metric_json((1000.0, "sales"), (100000.0, "visits"), format, None);
            let i: CreativeIntent = serde_json::from_value(json!({
                "version": "0.2", "title": "canvas", "format": canvas,
                "beats": [
                    { "purpose": "emphasize", "statement": "One ratio.", "primary": a.clone() },
                    compare_beat(a, b),
                ],
            }))
            .expect("parses");
            let p = build(&i, &StyleProfile::default());
            assert_inside_beat(&p);
        }
    }
}

#[test]
fn zero_denominator_is_a_compile_error() {
    let beat = json!({
        "purpose": "emphasize",
        "statement": "Nope.",
        "primary": metric_json((1.0, "a"), (0.0, "b"), "percent", None),
    });
    let r = compile(
        &intent(vec![beat]),
        &StyleProfile::default(),
        &AssetLibrary::new(ASSETS),
        &ApproxMeasure,
    );
    assert!(r.is_err());
}

#[test]
fn compilation_is_deterministic() {
    let a = metric_json(
        (120.0, "sales"),
        (4000.0, "visits"),
        "percent",
        Some("conversion"),
    );
    let b = metric_json((90.0, "sales"), (2000.0, "visits"), "percent", None);
    let i = intent(vec![single_beat(), compare_beat(a, b)]);
    let style = StyleProfile::default();
    let first = serde_json::to_string(&build(&i, &style)).unwrap();
    let second = serde_json::to_string(&build(&i, &style)).unwrap();
    assert_eq!(first, second);
}

/// Earliest motion start on a layer whose id ends with `suffix`.
fn first_start(s: &Scene, suffix: &str) -> f64 {
    s.motions
        .iter()
        .filter(|m| m.target.ends_with(suffix))
        .map(|m| m.start)
        .fold(f64::MAX, f64::min)
}

#[test]
fn single_metric_unfolds_numerator_denominator_result_bar() {
    let p = build(&intent(vec![single_beat()]), &StyleProfile::default());
    let s = beat_scene(&p);
    let life = s.lifecycle.expect("lifecycle");
    let num = first_start(s, "frac.num");
    let den = first_start(s, "frac.den");
    let res = first_start(s, "result");
    let bar = first_start(s, "progress.fill");
    assert!(
        num < den && den < res && res < bar,
        "{num} {den} {res} {bar}"
    );
    assert!(num < life.evolve, "numerator arrives before EVOLVE");
    assert!(den >= life.evolve - 1e-9, "denominator in EVOLVE");
    assert!(bar < life.anticipate, "last step starts before ANTICIPATE");
}

#[test]
fn comparison_unfolds_row_two_bars_conclusion() {
    let a = metric_json(
        (1000.0, "sales"),
        (100000.0, "visits"),
        "percent",
        Some("conversion"),
    );
    let b = metric_json((800.0, "sales"), (60000.0, "visits"), "percent", None);
    let p = build(&intent(vec![compare_beat(a, b)]), &StyleProfile::default());
    let s = beat_scene(&p);
    let life = s.lifecycle.expect("lifecycle");
    let r0 = first_start(s, "row.0.result");
    let r1 = first_start(s, "row.1.result");
    let bars = first_start(s, "bars.a.fill");
    let concl = first_start(s, "conclusion");
    assert!(
        r0 < r1 && r1 < bars && bars < concl,
        "{r0} {r1} {bars} {concl}"
    );
    assert!(r0 < life.evolve, "row 1 arrives before EVOLVE");
    assert!(r1 >= life.evolve - 1e-9, "row 2 in EVOLVE");
    assert!(concl < life.anticipate);
}
