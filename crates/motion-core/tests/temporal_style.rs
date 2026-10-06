//! Temporal style (0.6): one CreativeIntent, three TasteDirector styles.
//!
//! Proves that warm-editorial (A), dark-technical (B) and playful-print (C)
//! differ not just in their resolved profile but in how the compiled project
//! plays: duration, READ span, wipes, hierarchy reframes, easing, backdrop
//! motion, furniture and camera. Assertions are ordinal / structural, never
//! exact design values. A compact metrics golden (`golden/taste_abc.metrics.json`)
//! guards the numbers; regenerate with `MOTION_UPDATE_GOLDEN=1`.

use std::collections::BTreeSet;
use std::path::Path;

use motion_core::compiler::taste::{
    AnnotationStyle, CompositionRhythm, DensityLevel, ScaleContrast, TemperamentKind,
    TransitionFamily,
};
use motion_core::compiler::{compile, resolve_taste, ApproxMeasure, AssetLibrary};
use motion_core::easing::Easing;
use motion_core::intent::CreativeIntent;
use motion_core::scene::{CameraOp, Layer, LayerKind, MotionProject, Scene};
use motion_core::style::StyleProfile;
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../golden/taste_abc.metrics.json"
);

const DEMO_INTENT: &str = include_str!("../../../examples/editorial_demo.intent.json");
const POSTER_INTENT: &str = include_str!("../../../golden/kinetic_poster.intent.json");

const STYLE_A: &str = include_str!("../../../examples/taste/warm_editorial.style.json");
const STYLE_B: &str = include_str!("../../../examples/taste/dark_technical.style.json");
const STYLE_C: &str = include_str!("../../../examples/taste/playful_print.style.json");

const STYLES: [(&str, &str); 3] = [
    ("A_warm_editorial", STYLE_A),
    ("B_dark_technical", STYLE_B),
    ("C_playful_print", STYLE_C),
];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn library() -> AssetLibrary {
    AssetLibrary::new(ASSETS)
}

fn style(json: &str) -> StyleProfile {
    StyleProfile::from_json(json).expect("style parses")
}

fn intent(json: &str) -> CreativeIntent {
    CreativeIntent::from_json(json).expect("intent parses")
}

fn compile_with(intent_json: &str, style_json: &str) -> MotionProject {
    compile(
        &intent(intent_json),
        &style(style_json),
        &library(),
        &ApproxMeasure,
    )
    .expect("compiles")
}

// ---------------------------------------------------------------------------
// Metric extraction
// ---------------------------------------------------------------------------

/// Beat scenes are the ones the compiler gave a lifecycle (the backdrop has none).
fn beat_scenes(p: &MotionProject) -> Vec<&Scene> {
    p.scenes.iter().filter(|s| s.lifecycle.is_some()).collect()
}

fn backdrop(p: &MotionProject) -> &Scene {
    p.scenes
        .iter()
        .find(|s| s.id == "backdrop")
        .expect("backdrop scene")
}

fn walk<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            walk(children, out);
        }
    }
}

fn all_layers(s: &Scene) -> Vec<&Layer> {
    let mut out = Vec::new();
    walk(&s.layers, &mut out);
    out
}

fn beat_layers(p: &MotionProject) -> Vec<&Layer> {
    beat_scenes(p).into_iter().flat_map(all_layers).collect()
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Mean READ span (lifecycle `evolve - read`) over beat scenes, seconds.
fn mean_read_span(p: &MotionProject) -> f64 {
    let beats = beat_scenes(p);
    let sum: f64 = beats
        .iter()
        .map(|s| {
            let l = s.lifecycle.unwrap();
            l.evolve - l.read
        })
        .sum();
    sum / beats.len() as f64
}

/// Layers (any depth) in beat scenes that belong to a scene-handoff wipe.
fn wipe_layers(p: &MotionProject) -> usize {
    beat_layers(p)
        .iter()
        .filter(|l| l.id.contains(".wipe"))
        .count()
}

/// Stage-level scale motions that start inside `[evolve, anticipate)`.
fn evolve_stage_scales(p: &MotionProject) -> usize {
    beat_scenes(p)
        .iter()
        .map(|s| {
            let life = s.lifecycle.unwrap();
            s.motions
                .iter()
                .filter(|m| {
                    m.target.ends_with(".stage")
                        && m.op.op_name() == "scale"
                        && m.start >= life.evolve - 1e-6
                        && m.start < life.anticipate - 1e-6
                })
                .count()
        })
        .sum()
}

fn is_spring(e: Easing) -> bool {
    matches!(e, Easing::EditorialSpring | Easing::ImpactSpring)
}

/// Spring-eased motions that start before READ (entrances and settles).
fn spring_entrances(p: &MotionProject) -> usize {
    beat_scenes(p)
        .iter()
        .map(|s| {
            let life = s.lifecycle.unwrap();
            s.motions
                .iter()
                .filter(|m| is_spring(m.easing) && m.start < life.read)
                .count()
        })
        .sum()
}

/// Spring-eased motions anywhere in beat scenes.
fn spring_total(p: &MotionProject) -> usize {
    beat_scenes(p)
        .iter()
        .flat_map(|s| s.motions.iter())
        .filter(|m| is_spring(m.easing))
        .count()
}

/// Top-level group layers of the backdrop that form the measurement grid.
fn backdrop_grid_groups(p: &MotionProject) -> usize {
    backdrop(p)
        .layers
        .iter()
        .filter(|l| matches!(l.kind, LayerKind::Group { .. }) && l.id.starts_with("backdrop.grid"))
        .count()
}

/// Backdrop color-field layers (`backdrop.field{k}` / `backdrop.field{k}.c{j}`).
fn backdrop_field_layers(p: &MotionProject) -> usize {
    all_layers(backdrop(p))
        .iter()
        .filter(|l| l.id.starts_with("backdrop.field"))
        .count()
}

fn backdrop_motions(p: &MotionProject) -> usize {
    backdrop(p).motions.len()
}

/// Distinct (10 ms rounded) start times of backdrop motions: how many separate
/// moments the background changes.
fn backdrop_motion_times(p: &MotionProject) -> usize {
    backdrop(p)
        .motions
        .iter()
        .map(|m| (m.start * 100.0).round() as i64)
        .collect::<BTreeSet<_>>()
        .len()
}

/// Furniture (annotation) kind, from the layer-id suffix after the beat prefix
/// (see `compiler/furniture.rs`).
fn furniture_name(id: &str) -> Option<&str> {
    const KINDS: [&str; 10] = [
        "folio",
        "folio_rule",
        "folio_tick",
        "ticks",
        "index",
        "data",
        "numeral",
        "sticker",
        "reg_h",
        "reg_v",
    ];
    let (_, name) = id.split_once('.')?;
    KINDS.contains(&name).then_some(name)
}

fn furniture_kinds(p: &MotionProject) -> BTreeSet<String> {
    beat_layers(p)
        .iter()
        .filter_map(|l| furniture_name(&l.id).map(str::to_string))
        .collect()
}

fn furniture_layers(p: &MotionProject) -> usize {
    beat_layers(p)
        .iter()
        .filter(|l| furniture_name(&l.id).is_some())
        .count()
}

fn camera_ops(p: &MotionProject) -> impl Iterator<Item = &CameraOp> {
    p.scenes
        .iter()
        .filter_map(|s| s.camera.as_ref())
        .flat_map(|c| c.motions.iter().map(|m| &m.op))
}

/// Largest camera push excursion `|to - 1|` over all scenes.
fn max_push(p: &MotionProject) -> f64 {
    camera_ops(p)
        .filter_map(|op| match op {
            CameraOp::Push { to, .. } => Some((*to as f64 - 1.0).abs()),
            _ => None,
        })
        .fold(0.0, f64::max)
}

/// Largest camera track travel (canvas pixels) over all scenes.
fn max_track(p: &MotionProject) -> f64 {
    camera_ops(p)
        .filter_map(|op| match op {
            CameraOp::Track { from, to } => {
                Some(((to[0] - from[0]) as f64).hypot((to[1] - from[1]) as f64))
            }
            _ => None,
        })
        .fold(0.0, f64::max)
}

fn metrics(p: &MotionProject) -> Value {
    json!({
        "duration_seconds": round3(p.duration_seconds()),
        "frame_count": p.frame_count(),
        "beat_scenes": beat_scenes(p).len(),
        "mean_read_span": round3(mean_read_span(p)),
        "wipe_layers": wipe_layers(p),
        "evolve_stage_scales": evolve_stage_scales(p),
        "spring_entrances": spring_entrances(p),
        "spring_total": spring_total(p),
        "backdrop_grid_groups": backdrop_grid_groups(p),
        "backdrop_field_layers": backdrop_field_layers(p),
        "backdrop_motions": backdrop_motions(p),
        "backdrop_motion_times": backdrop_motion_times(p),
        "furniture_layers": furniture_layers(p),
        "furniture_kinds": furniture_kinds(p),
        "camera_max_push": round3(max_push(p)),
        "camera_max_track": round3(max_track(p)),
    })
}

fn fingerprint_value(intent_json: &str, style_json: &str) -> Value {
    let r = resolve_taste(&intent(intent_json), &style(style_json), None);
    serde_json::to_value(r.fingerprint()).expect("fingerprint serializes")
}

/// The intents every behavioral test runs over: the demo and a second
/// (kinetic, replace-relationship) story.
const INTENTS: [(&str, &str); 2] = [
    ("editorial_demo", DEMO_INTENT),
    ("kinetic_poster", POSTER_INTENT),
];

/// Compile `intent_json` with A, B and C.
fn abc(intent_json: &str) -> [MotionProject; 3] {
    [STYLE_A, STYLE_B, STYLE_C].map(|s| compile_with(intent_json, s))
}

// ---------------------------------------------------------------------------
// 0. One intent, three styles
// ---------------------------------------------------------------------------

#[test]
fn one_intent_is_reused_byte_identically_across_styles() {
    for (name, json) in INTENTS {
        let parsed = intent(json);
        let before = serde_json::to_string(&parsed).expect("intent serializes");
        // Reparsing the same source bytes gives the same intent.
        assert_eq!(before, serde_json::to_string(&intent(json)).unwrap());
        for (style_name, style_json) in STYLES {
            compile(&parsed, &style(style_json), &library(), &ApproxMeasure)
                .unwrap_or_else(|e| panic!("{name} x {style_name}: {e}"));
            assert_eq!(
                before,
                serde_json::to_string(&parsed).unwrap(),
                "{name}: compiling with {style_name} must not touch the intent"
            );
        }
    }
    // The style files carry tendencies only: no story content, no timing.
    for (n, s) in STYLES {
        let v: Value = serde_json::from_str(s).unwrap();
        for key in v.as_object().unwrap().keys() {
            assert!(
                ["tone", "polarity", "temperature", "temperament", "density"]
                    .contains(&key.as_str()),
                "{n}: unexpected style key {key}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 1. Resolution level
// ---------------------------------------------------------------------------

#[test]
fn resolved_profiles_are_pairwise_different_with_expected_values() {
    for (name, json) in INTENTS {
        let it = intent(json);
        let [a, b, c] = [STYLE_A, STYLE_B, STYLE_C].map(|s| resolve_taste(&it, &style(s), None));

        assert_eq!(a.motion.kind, TemperamentKind::Restrained, "{name} A");
        assert_eq!(a.transition.family, TransitionFamily::Editorial, "{name} A");
        assert_eq!(a.density.level, DensityLevel::Balanced, "{name} A");
        assert_eq!(a.density.annotation, AnnotationStyle::Editorial, "{name} A");
        assert_eq!(a.rhythm, CompositionRhythm::MeasuredEditorial, "{name} A");
        assert_eq!(a.scale, ScaleContrast::Large, "{name} A");

        assert_eq!(b.motion.kind, TemperamentKind::Precise, "{name} B");
        assert_eq!(b.transition.family, TransitionFamily::Geometric, "{name} B");
        assert_eq!(b.density.level, DensityLevel::Balanced, "{name} B");
        assert_eq!(
            b.density.annotation,
            AnnotationStyle::Structured,
            "{name} B"
        );
        assert_eq!(b.rhythm, CompositionRhythm::Progressive, "{name} B");
        assert_eq!(b.scale, ScaleContrast::Moderate, "{name} B");

        assert_eq!(c.motion.kind, TemperamentKind::Energetic, "{name} C");
        assert_eq!(c.transition.family, TransitionFamily::Kinetic, "{name} C");
        assert_eq!(c.density.level, DensityLevel::Dense, "{name} C");
        assert_eq!(c.density.annotation, AnnotationStyle::Graphic, "{name} C");
        assert_eq!(c.rhythm, CompositionRhythm::Active, "{name} C");
        assert_eq!(c.scale, ScaleContrast::Dramatic, "{name} C");

        for (l, x, y) in [("A/B", &a, &b), ("A/C", &a, &c), ("B/C", &b, &c)] {
            assert_ne!(x.motion.kind, y.motion.kind, "{name} {l} motion.kind");
            assert_ne!(
                x.transition.family, y.transition.family,
                "{name} {l} transition.family"
            );
            assert_ne!(x.density, y.density, "{name} {l} density");
            assert_ne!(x.rhythm, y.rhythm, "{name} {l} rhythm");
            assert_ne!(x.scale, y.scale, "{name} {l} scale");
            let d = x.fingerprint().distance(&y.fingerprint());
            assert!(d >= 8, "{name} {l}: fingerprint distance {d} < 8");
        }
    }
}

#[test]
fn accent_role_alone_is_not_a_different_style() {
    let it = intent(DEMO_INTENT);
    let plain = resolve_taste(&it, &style("{}"), None).fingerprint();
    let cobalt = resolve_taste(&it, &style(r#"{"accent_role":"cobalt"}"#), None).fingerprint();
    assert_eq!(plain.distance(&cobalt), 0);
}

// ---------------------------------------------------------------------------
// 2. Compiled behavior: the styles differ when played
// ---------------------------------------------------------------------------

#[test]
fn total_duration_orders_a_over_b_over_c() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        let (da, db, dc) = (
            a.duration_seconds(),
            b.duration_seconds(),
            c.duration_seconds(),
        );
        assert!(
            da > db && db > dc,
            "{name}: expected A > B > C, got {da:.3} / {db:.3} / {dc:.3}"
        );
    }
}

#[test]
fn read_span_shrinks_as_rhythm_gets_more_active() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        let (ra, rb, rc) = (mean_read_span(&a), mean_read_span(&b), mean_read_span(&c));
        assert!(
            ra >= rb && rb >= rc,
            "{name}: expected mean READ span A >= B >= C, got {ra:.3} / {rb:.3} / {rc:.3}"
        );
        assert!(ra > rc, "{name}: A and C READ spans must actually differ");
    }
}

#[test]
fn geometric_and_kinetic_styles_have_panel_wipes_editorial_does_not() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        assert_eq!(wipe_layers(&a), 0, "{name}: editorial has no panel wipe");
        assert!(wipe_layers(&b) > 0, "{name}: geometric wipe layers");
        assert!(wipe_layers(&c) > 0, "{name}: kinetic wipe layers");
        // The wipe is a scene-level overlay that is animated, not a static layer.
        for (label, p) in [("B", &b), ("C", &c)] {
            let animated = beat_scenes(p).into_iter().any(|s| {
                all_layers(s)
                    .into_iter()
                    .any(|l| l.id.contains(".wipe") && s.motions.iter().any(|m| m.target == l.id))
            });
            assert!(animated, "{name} {label}: wipes must move");
        }
    }
}

#[test]
fn hierarchy_reframes_during_evolve_only_in_active_rhythms() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        assert_eq!(evolve_stage_scales(&a), 0, "{name}: measured rhythm holds");
        assert!(evolve_stage_scales(&b) > 0, "{name}: progressive reframes");
        assert!(evolve_stage_scales(&c) > 0, "{name}: active reframes");
    }
}

#[test]
fn only_energetic_style_springs_on_entrances() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        assert_eq!(spring_total(&a), 0, "{name}: restrained has no springs");
        assert_eq!(spring_total(&b), 0, "{name}: precise is crisp, no springs");
        assert!(
            spring_entrances(&c) > 0,
            "{name}: energetic springs entrances"
        );
    }
}

#[test]
fn backdrops_move_differently_per_style() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);

        // A: paper field, quiet: nothing animates in the backdrop.
        assert_eq!(backdrop_motions(&a), 0, "{name} A backdrop is still");
        assert_eq!(backdrop_grid_groups(&a), 0, "{name} A has no grid");
        assert_eq!(backdrop_field_layers(&a), 0, "{name} A has no fields");

        // B: a drifting measurement grid (one group) that moves.
        assert_eq!(backdrop_grid_groups(&b), 1, "{name} B grid group");
        assert!(backdrop_motions(&b) > 0, "{name} B grid drifts");
        assert_eq!(backdrop_field_layers(&b), 0, "{name} B has no color fields");

        // C: several color fields that recompose at beat boundaries.
        assert!(backdrop_field_layers(&c) > 1, "{name} C field layers");
        assert_eq!(backdrop_grid_groups(&c), 0, "{name} C has no grid");
        assert!(backdrop_motions(&c) > 0, "{name} C fields move");
        let boundaries: Vec<f64> = beat_scenes(&c)
            .iter()
            .skip(1)
            .map(|s| s.start_seconds)
            .collect();
        for m in &backdrop(&c).motions {
            // t ~ 0 entries only park a hidden color variant before its first cross-fade.
            if m.start < 0.01 {
                continue;
            }
            assert!(
                boundaries.iter().any(|t| (m.start - t).abs() <= 1.0),
                "{name} C: backdrop motion at {:.2}s is not near a beat boundary {boundaries:?}",
                m.start
            );
        }
        assert!(
            backdrop_motion_times(&c) > 0,
            "{name} C: at least one recomposition"
        );
    }
}

#[test]
fn furniture_differs_per_style_and_classic_has_none() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        let (ka, kb, kc) = (
            furniture_kinds(&a),
            furniture_kinds(&b),
            furniture_kinds(&c),
        );
        for (label, k) in [("A", &ka), ("B", &kb), ("C", &kc)] {
            assert!(!k.is_empty(), "{name} {label}: furniture present");
        }
        for (label, x, y) in [("A/B", &ka, &kb), ("A/C", &ka, &kc), ("B/C", &kb, &kc)] {
            assert!(
                x.is_disjoint(y),
                "{name} {label}: furniture kinds should differ, got {x:?} vs {y:?}"
            );
        }
        // Every beat scene carries its own furniture.
        for (label, p) in [("A", &a), ("B", &b), ("C", &c)] {
            for s in beat_scenes(p) {
                assert!(
                    all_layers(s)
                        .iter()
                        .any(|l| furniture_name(&l.id).is_some()),
                    "{name} {label}: scene {} lacks furniture",
                    s.id
                );
            }
        }
        let classic = compile_with(json, "{}");
        assert_eq!(furniture_layers(&classic), 0, "{name}: classic has none");
    }
}

#[test]
fn camera_is_controlled_for_a_and_b_active_for_c() {
    for (name, json) in INTENTS {
        let [a, b, c] = abc(json);
        assert!(
            max_push(&c) > max_push(&a),
            "{name}: C push {:.3} > A push {:.3}",
            max_push(&c),
            max_push(&a)
        );
        assert!(
            max_push(&c) > max_push(&b),
            "{name}: C push {:.3} > B push {:.3}",
            max_push(&c),
            max_push(&b)
        );
        assert!(
            max_track(&c) > max_track(&b),
            "{name}: C track {:.1} > B track {:.1}",
            max_track(&c),
            max_track(&b)
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Determinism
// ---------------------------------------------------------------------------

#[test]
fn compiling_each_style_twice_is_identical() {
    for (name, json) in INTENTS {
        for (style_name, style_json) in STYLES {
            let x = compile_with(json, style_json);
            let y = compile_with(json, style_json);
            assert_eq!(
                x.to_json_pretty(),
                y.to_json_pretty(),
                "{name} x {style_name}: compile must be deterministic"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Golden metrics
// ---------------------------------------------------------------------------

fn golden_document() -> Value {
    let mut intents = serde_json::Map::new();
    for (intent_name, intent_json) in INTENTS {
        let mut styles = serde_json::Map::new();
        for (style_name, style_json) in STYLES {
            let p = compile_with(intent_json, style_json);
            styles.insert(
                style_name.to_string(),
                json!({
                    "fingerprint": fingerprint_value(intent_json, style_json),
                    "metrics": metrics(&p),
                }),
            );
        }
        intents.insert(intent_name.to_string(), Value::Object(styles));
    }
    json!({
        "about": "Resolved fingerprint + measured temporal metrics for the three taste styles (examples/taste/*.style.json) over the same intents. Regenerate: MOTION_UPDATE_GOLDEN=1 cargo test -p motion-core --test temporal_style",
        "intents": intents,
    })
}

#[test]
fn taste_abc_metrics_match_golden() {
    let actual = golden_document();
    let update = std::env::var("MOTION_UPDATE_GOLDEN").is_ok_and(|v| v == "1");
    if update {
        let mut text = serde_json::to_string_pretty(&actual).unwrap();
        text.push('\n');
        std::fs::write(Path::new(GOLDEN), text).expect("write golden");
        return;
    }
    let text = std::fs::read_to_string(GOLDEN)
        .unwrap_or_else(|e| panic!("{GOLDEN} missing ({e}); run with MOTION_UPDATE_GOLDEN=1"));
    let expected: Value = serde_json::from_str(&text).expect("golden parses");
    assert_eq!(
        actual,
        expected,
        "temporal metrics changed. If intentional, rerun with MOTION_UPDATE_GOLDEN=1 and review.\nactual:\n{}",
        serde_json::to_string_pretty(&actual).unwrap()
    );
}
