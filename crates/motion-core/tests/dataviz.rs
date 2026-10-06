//! Data-viz primitives: geometry, ids, timing hygiene, determinism and
//! validity when embedded in a MotionProject.

use std::collections::BTreeMap;
use std::path::Path;

use motion_core::easing::MotionPreset;
use motion_core::motion::dataviz::{
    bar_chart, comparison_bar, counter, path_draw, progress_bar, CountFormat, DataStyle, NumberSlot,
};
use motion_core::motion::stagger::{StaggerOrder, StaggerPreset, StaggerSpec};
use motion_core::motion::Expansion;
use motion_core::scene::{
    BoxRect, Color, FontRole, LayerKind, Motion, MotionOp, MotionProject, TextAlign, TextStyle,
};
use motion_core::timeline::format_count;
use motion_core::validate::validate;
use serde_json::json;

const INK: Color = Color::rgb(22, 19, 15);
const ACCENT: Color = Color::rgb(214, 58, 43);
const MUTED: Color = Color::rgb(140, 132, 120);
const TRACK: Color = Color::rgb(220, 210, 190);

fn style() -> DataStyle {
    DataStyle {
        ink: INK,
        accent: ACCENT,
        muted: MUTED,
        track: TRACK,
        u: 1.0,
    }
}

fn rect() -> BoxRect {
    BoxRect {
        x: 100.0,
        y: 200.0,
        width: 800.0,
        height: 400.0,
    }
}

fn params() -> motion_core::easing::PresetParams {
    MotionPreset::Editorial.params()
}

fn stagger() -> StaggerSpec {
    StaggerSpec {
        preset: StaggerPreset::Editorial,
        order: StaggerOrder::Forward,
    }
}

fn slot() -> NumberSlot {
    NumberSlot {
        style: TextStyle {
            text: "0".into(),
            font_role: FontRole::Number,
            font_size: 240.0,
            font_weight: 400,
            italic: false,
            color: INK,
            align: TextAlign::Center,
            line_height: 1.0,
            letter_spacing: 0.0,
            max_width: None,
            uppercase: false,
            ink: None,
        },
        rect: BoxRect {
            x: 90.0,
            y: 120.0,
            width: 900.0,
            height: 260.0,
        },
    }
}

fn fmt() -> CountFormat {
    CountFormat {
        decimals: 1,
        grouping: true,
        prefix: "$".into(),
        suffix: "M".into(),
    }
}

fn all_expansions() -> Vec<(&'static str, Expansion)> {
    let p = params();
    let s = style();
    vec![
        (
            "counter",
            counter("hero", &slot(), 0.0, 1234.5, &fmt(), 0.2, 1.4, &p),
        ),
        ("progress", progress_bar("prog", rect(), 0.72, &s, 0.3, &p)),
        (
            "bars",
            bar_chart(
                "bars",
                rect(),
                &[3.0, 5.0, 9.0, 4.0, 6.0],
                Some(2),
                &s,
                0.3,
                &p,
                stagger(),
            ),
        ),
        (
            "cmp",
            comparison_bar("cmp", rect(), 40.0, 100.0, &s, 0.3, &p),
        ),
        (
            "path",
            path_draw("path", rect(), &[1.0, 3.0, 2.0, 5.0, 4.0], &s, 0.3, 1.6, &p),
        ),
    ]
}

fn layer_by_id<'a>(e: &'a Expansion, id: &str) -> &'a motion_core::scene::Layer {
    e.layers
        .iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("layer {id} missing in {:?}", ids(e)))
}

fn ids(e: &Expansion) -> Vec<&str> {
    e.layers.iter().map(|l| l.id.as_str()).collect()
}

fn motions_for<'a>(e: &'a Expansion, target: &str) -> Vec<&'a Motion> {
    e.motions.iter().filter(|m| m.target == target).collect()
}

fn accent_to(e: &Expansion, target: &str) -> BoxRect {
    motions_for(e, target)
        .iter()
        .find_map(|m| match m.op {
            MotionOp::AccentExpand { to } => Some(to),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no accent_expand for {target}"))
}

fn fill_of(l: &motion_core::scene::Layer) -> Color {
    match &l.kind {
        LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => *fill,
        other => panic!("not a rect: {other:?}"),
    }
}

fn approx(a: f32, b: f32) {
    assert!((a - b).abs() < 1e-3, "{a} != {b}");
}

// ---------------------------------------------------------------------------

#[test]
fn every_primitive_has_channel_disjoint_motions() {
    for (name, e) in all_expansions() {
        assert!(!e.layers.is_empty(), "{name}: no layers");
        assert!(!e.motions.is_empty(), "{name}: no motions");
        // Every motion targets a layer of the expansion, with sane timing.
        for m in &e.motions {
            assert!(e.layers.iter().any(|l| l.id == m.target), "{name}: {m:?}");
            assert!(m.start >= 0.0 && m.duration >= 0.0, "{name}: {m:?}");
        }
        // No same-channel overlapping windows per layer.
        for a in 0..e.motions.len() {
            for b in (a + 1)..e.motions.len() {
                let (x, y) = (&e.motions[a], &e.motions[b]);
                if x.target == y.target && x.op.channel() == y.op.channel() {
                    let disjoint = x.start + x.duration <= y.start + 1e-9
                        || y.start + y.duration <= x.start + 1e-9;
                    assert!(disjoint, "{name}: overlap {x:?} vs {y:?}");
                }
            }
        }
        // Unique ids.
        let mut seen = std::collections::HashSet::new();
        for l in &e.layers {
            assert!(seen.insert(l.id.clone()), "{name}: duplicate id {}", l.id);
        }
    }
}

#[test]
fn primitives_are_deterministic() {
    let a = all_expansions();
    let b = all_expansions();
    for ((n, x), (_, y)) in a.iter().zip(b.iter()) {
        assert_eq!(x, y, "{n} not deterministic");
    }
}

#[test]
fn counter_initial_text_matches_timeline_formatting() {
    let f = fmt();
    for from in [0.0, 12.34, 1234.5, -7.0] {
        let e = counter("c", &slot(), from, 99.0, &f, 0.0, 1.0, &params());
        let LayerKind::Text(t) = &layer_by_id(&e, "c").kind else {
            panic!("not text");
        };
        assert_eq!(
            t.text,
            format_count(from, f.decimals, f.grouping, &f.prefix, &f.suffix)
        );
    }
}

#[test]
fn counter_layer_and_count_motion() {
    let e = counter("hero", &slot(), 0.0, 1234.5, &fmt(), 0.2, 1.4, &params());
    assert_eq!(ids(&e), vec!["hero"]);
    let l = layer_by_id(&e, "hero");
    assert_eq!((l.x, l.y, l.width, l.height), (90.0, 120.0, 900.0, 260.0));
    match &l.kind {
        LayerKind::Text(t) => {
            assert_eq!(t.text, "$0.0M");
            assert_eq!(t.align, TextAlign::Center);
            assert_eq!(t.font_size, 240.0);
        }
        other => panic!("{other:?}"),
    }
    let count = e
        .motions
        .iter()
        .find(|m| matches!(m.op, MotionOp::Count { .. }))
        .expect("count motion");
    assert_eq!(count.start, 0.2);
    assert_eq!(count.duration, 1.4);
    match &count.op {
        MotionOp::Count {
            from,
            to,
            decimals,
            grouping,
            prefix,
            suffix,
        } => {
            assert_eq!((*from, *to, *decimals, *grouping), (0.0, 1234.5, 1, true));
            assert_eq!((prefix.as_str(), suffix.as_str()), ("$", "M"));
        }
        _ => unreachable!(),
    }
    // Count never overshoots its target.
    assert!(matches!(
        count.easing,
        motion_core::Easing::OutQuint | motion_core::Easing::OutCubic
    ));
    // Rise-in: fade and move start together with the count.
    assert!(e
        .motions
        .iter()
        .any(|m| matches!(m.op, MotionOp::Fade { .. }) && m.start == 0.2));
    assert!(e
        .motions
        .iter()
        .any(|m| matches!(m.op, MotionOp::Move { .. }) && m.start == 0.2));
}

#[test]
fn progress_bar_fill_reaches_fraction() {
    let e = progress_bar("prog", rect(), 0.72, &style(), 0.3, &params());
    assert_eq!(ids(&e), vec!["prog.track", "prog.fill"]);
    let track = layer_by_id(&e, "prog.track");
    let fill = layer_by_id(&e, "prog.fill");
    assert_eq!(fill_of(track), TRACK);
    assert_eq!(fill_of(fill), ACCENT);
    assert_eq!(
        (track.x, track.y, track.width, track.height),
        (100.0, 200.0, 800.0, 400.0)
    );
    assert_eq!(fill.width, 0.0);
    assert_eq!((fill.x, fill.y, fill.height), (100.0, 200.0, 400.0));
    let to = accent_to(&e, "prog.fill");
    approx(to.width, 0.72 * 800.0);
    assert_eq!((to.x, to.y, to.height), (100.0, 200.0, 400.0));
    // Out-of-range / NaN fractions are clamped.
    let over = progress_bar("p", rect(), 3.0, &style(), 0.0, &params());
    approx(accent_to(&over, "p.fill").width, 800.0);
    let nan = progress_bar("p", rect(), f32::NAN, &style(), 0.0, &params());
    approx(accent_to(&nan, "p.fill").width, 0.0);
}

#[test]
fn bar_chart_geometry() {
    let values = [3.0, 5.0, 9.0, 4.0, 6.0];
    let e = bar_chart(
        "bars",
        rect(),
        &values,
        Some(2),
        &style(),
        0.3,
        &params(),
        stagger(),
    );
    assert_eq!(e.layers.len(), 1 + values.len());
    assert_eq!(e.layers[0].id, "bars.base");
    let baseline = 200.0 + 400.0;
    let slot = 800.0 / 5.0;
    let mut starts = Vec::new();
    let mut durs = Vec::new();
    for (i, v) in values.iter().enumerate() {
        let id = format!("bars.bar.{i}");
        let l = layer_by_id(&e, &id);
        // Starts flat on the baseline.
        assert_eq!(l.height, 0.0);
        approx(l.y, baseline);
        approx(l.width, slot * 0.62);
        // Even gaps: bar centered in its slot.
        approx(l.x + l.width / 2.0, 100.0 + slot * (i as f32 + 0.5));
        let to = accent_to(&e, &id);
        approx(to.height, 400.0 * (*v as f32 / 9.0));
        approx(to.y + to.height, baseline);
        approx(to.width, l.width);
        approx(to.x, l.x);
        assert_eq!(fill_of(l), if i == 2 { ACCENT } else { INK });
        let m = motions_for(&e, &id)[0];
        starts.push(m.start);
        durs.push(m.duration);
    }
    // Tallest bar reaches the full height.
    approx(accent_to(&e, "bars.bar.2").height, 400.0);
    // Not robotic: distinct start times and distinct durations.
    let distinct = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(f64::total_cmp);
        s.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        s.len()
    };
    assert!(distinct(&starts) > 1, "{starts:?}");
    assert!(distinct(&durs) > 1, "{durs:?}");
    // Baseline rule sits under the bars, muted.
    let base = layer_by_id(&e, "bars.base");
    assert_eq!(fill_of(base), MUTED);
    approx(base.y, baseline);
    approx(base.width, 800.0);
}

#[test]
fn bar_chart_edge_cases() {
    let zeros = bar_chart(
        "z",
        rect(),
        &[0.0, 0.0, 0.0],
        None,
        &style(),
        0.0,
        &params(),
        stagger(),
    );
    for i in 0..3 {
        let to = accent_to(&zeros, &format!("z.bar.{i}"));
        assert_eq!(to.height, 0.0);
        approx(to.y, 600.0);
    }
    let odd = bar_chart(
        "o",
        rect(),
        &[f64::NAN, -4.0, 2.0, f64::INFINITY],
        Some(9),
        &style(),
        0.0,
        &params(),
        stagger(),
    );
    for i in 0..4 {
        let to = accent_to(&odd, &format!("o.bar.{i}"));
        assert!(to.height.is_finite() && to.height >= 0.0 && to.height <= 400.0);
        assert!(to.y.is_finite());
    }
    // Out-of-range highlight: nothing is accented.
    for i in 0..4 {
        assert_eq!(fill_of(layer_by_id(&odd, &format!("o.bar.{i}"))), INK);
    }
    let empty = bar_chart("e", rect(), &[], None, &style(), 0.0, &params(), stagger());
    assert_eq!(ids(&empty), vec!["e.base"]);
}

#[test]
fn comparison_bar_sequences_and_scales() {
    let p = params();
    let e = comparison_bar("cmp", rect(), 40.0, 100.0, &style(), 0.3, &p);
    assert_eq!(
        ids(&e),
        vec!["cmp.a.track", "cmp.a.fill", "cmp.b.track", "cmp.b.fill"]
    );
    // Rows stacked: a above b.
    assert!(layer_by_id(&e, "cmp.a.track").y < layer_by_id(&e, "cmp.b.track").y);
    let (a, b) = (accent_to(&e, "cmp.a.fill"), accent_to(&e, "cmp.b.fill"));
    approx(a.width, 800.0 * 0.4);
    approx(b.width, 800.0);
    // Larger gets accent, other ink.
    assert_eq!(fill_of(layer_by_id(&e, "cmp.a.fill")), INK);
    assert_eq!(fill_of(layer_by_id(&e, "cmp.b.fill")), ACCENT);
    // b starts ~0.35 of a's duration after a.
    let ma = motions_for(&e, "cmp.a.fill")[0];
    let mb = motions_for(&e, "cmp.b.fill")[0];
    approx((mb.start - ma.start) as f32, (0.35 * ma.duration) as f32);
    // Flipped values flip the accent.
    let f = comparison_bar("cmp", rect(), 100.0, 40.0, &style(), 0.0, &p);
    assert_eq!(fill_of(layer_by_id(&f, "cmp.a.fill")), ACCENT);
    approx(accent_to(&f, "cmp.a.fill").width, 800.0);
    // Both zero: valid zero-width fills.
    let z = comparison_bar("z", rect(), 0.0, 0.0, &style(), 0.0, &p);
    assert_eq!(accent_to(&z, "z.a.fill").width, 0.0);
    assert_eq!(accent_to(&z, "z.b.fill").width, 0.0);
}

#[test]
fn path_draw_points_trim_and_dot() {
    let vals = [1.0, 3.0, 2.0, 5.0, 4.0];
    let e = path_draw("path", rect(), &vals, &style(), 0.3, 1.6, &params());
    assert_eq!(ids(&e), vec!["path.line", "path.dot"]);
    let line = layer_by_id(&e, "path.line");
    assert_eq!(
        (line.x, line.y, line.width, line.height),
        (100.0, 200.0, 800.0, 400.0)
    );
    let LayerKind::Polyline {
        points,
        stroke,
        closed,
        fill,
    } = &line.kind
    else {
        panic!("not a polyline");
    };
    assert_eq!(points.len(), 5);
    assert!(!closed && fill.is_none());
    assert_eq!(stroke.color, ACCENT);
    approx(stroke.width, 8.0);
    // Evenly spaced across the width, min at the bottom, max at the top.
    for (i, p) in points.iter().enumerate() {
        approx(p[0], 800.0 * i as f32 / 4.0);
    }
    approx(points[0][1], 400.0); // value 1 = min
    approx(points[3][1], 0.0); // value 5 = max
    approx(points[1][1], 400.0 * (1.0 - 0.5));
    // Trim 0 -> 1 across the whole duration.
    let trim = motions_for(&e, "path.line")[0];
    assert_eq!((trim.start, trim.duration), (0.3, 1.6));
    assert!(matches!(trim.op, MotionOp::Trim { from, to } if from == 0.0 && to == 1.0));
    // Dot sits on the last point (layer space) and fades in near the end.
    let dot = layer_by_id(&e, "path.dot");
    approx(dot.x, 100.0 + points[4][0]);
    approx(dot.y, 200.0 + points[4][1]);
    let fade = motions_for(&e, "path.dot")
        .into_iter()
        .find(|m| matches!(m.op, MotionOp::Fade { .. }))
        .expect("dot fade");
    assert!(fade.start >= 0.3 + 1.6 * 0.8);
    // Flat series is drawn on the middle line; empty is empty.
    let flat = path_draw("f", rect(), &[2.0, 2.0, 2.0], &style(), 0.0, 1.0, &params());
    if let LayerKind::Polyline { points, .. } = &layer_by_id(&flat, "f.line").kind {
        assert!(points.iter().all(|p| (p[1] - 200.0).abs() < 1e-3));
    }
    assert!(path_draw("x", rect(), &[], &style(), 0.0, 1.0, &params())
        .layers
        .is_empty());
}

fn project_with(e: &Expansion) -> MotionProject {
    let v = json!({
        "version": "0.2",
        "project": { "name": "dataviz_fixture" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#ECE3D2" },
        "theme": { "fonts": { "number": "font.number" } },
        "assets": [ { "id": "font.number", "type": "font", "path": "fonts/Anton-Regular.ttf" } ],
        "scenes": [ {
            "id": "s",
            "start_seconds": 0.0,
            "duration_seconds": 6.0,
            "layers": e.layers,
            "motions": e.motions,
        } ]
    });
    MotionProject::from_json(&v.to_string()).expect("fixture parses")
}

#[test]
fn expansions_embed_in_a_valid_project() {
    let mut all = Expansion::default();
    for (_, e) in all_expansions() {
        all.extend(e);
    }
    let project = project_with(&all);
    if let Err(e) = validate(&project, None) {
        panic!("{e:#?}");
    }
    // And each expands into a project that evaluates at every frame.
    for f in [0, 15, 45, 90, 150, 179] {
        motion_core::evaluate_frame(&project, f).expect("evaluate");
    }
}

#[test]
fn golden_data_viz_parses_and_validates() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/data_viz.motion.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let project = MotionProject::from_json(&text).expect("golden parses");
    assert_eq!(
        (
            project.canvas.width,
            project.canvas.height,
            project.canvas.fps
        ),
        (1080, 1920, 30)
    );
    let base = path.parent().map(Path::to_path_buf);
    if let Err(e) = validate(&project, base.as_deref()) {
        panic!("{e:#?}");
    }
    let kinds: BTreeMap<&str, usize> =
        project
            .scenes
            .iter()
            .flat_map(|s| s.layers.iter())
            .fold(BTreeMap::new(), |mut m, l| {
                *m.entry(l.kind.type_name()).or_insert(0) += 1;
                m
            });
    assert!(kinds.contains_key("polyline"), "{kinds:?}");
    let ops: Vec<&str> = project
        .scenes
        .iter()
        .flat_map(|s| s.motions.iter())
        .map(|m| m.op.op_name())
        .collect();
    assert!(ops.contains(&"count") && ops.contains(&"trim") && ops.contains(&"accent_expand"));
}
