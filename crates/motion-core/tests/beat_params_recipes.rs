//! (0.23 C2b) The recipes builders (EditorialCollage / `emphasize`, `contrast`,
//! `explain`), the collection composition (SequentialStack) and HeroObject
//! honour a beat's planned `BeatParams`.
//!
//! * Identity: the default params (what every compile without a direction
//!   seed hands a builder) build the exact old beat.
//! * Steering: each of the eight presets (the ones C2a / C2c reuse) changes
//!   the motions the builder owns, in the planned way: reveal kind and side,
//!   easing, travel, stagger, accent present or absent.
//! * Safety: under every preset, in vertical and square, the scene validates,
//!   every text is still there, the reveal anchors are the same, no motion
//!   ends after the scene, no count settles later than at the identity, and
//!   the layout report gains no finding.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::checks::{COUNT_HOLD_S, COUNT_SETTLE_S};
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::direction::{
    AccentMark, BeatParams, EasingFamily, EntranceFamily, Reveal, StaggerOrder, StaggerPreset,
};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile, compile_with_beat_params, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning,
    WARN_VALUE_DROPPED,
};
use motion_core::easing::Easing;
use motion_core::intent::{CreativeIntent, Format};
use motion_core::layout_qa::layout_report_with;
use motion_core::scene::{
    Direction, GlyphOrder, Layer, LayerKind, Motion, MotionOp, MotionProject, Scene,
};
use motion_core::speech::{repair, SpeechMap};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, format_count, frame_time, ResolvedLayer};
use motion_core::validate::validate;
use motion_core::ImageIndex;
use serde_json::{json, Value};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");

// ---------------------------------------------------------------------------
// The eight presets
// ---------------------------------------------------------------------------

fn presets() -> Vec<(&'static str, BeatParams)> {
    let id = BeatParams::default();
    vec![
        (
            "P1",
            BeatParams {
                entrance: EntranceFamily::WipeLeft,
                easing: EasingFamily::Snappy,
                ..id
            },
        ),
        (
            "P2",
            BeatParams {
                entrance: EntranceFamily::SlideRight,
                travel: 1.5,
                stagger: StaggerPreset::Loose,
                ..id
            },
        ),
        (
            "P3",
            BeatParams {
                entrance: EntranceFamily::Rise,
                stagger: StaggerPreset::Tight,
                stagger_order: StaggerOrder::Reverse,
                ..id
            },
        ),
        (
            "P4",
            BeatParams {
                entrance: EntranceFamily::ScalePop,
                accent: AccentMark::None,
                amplitude: 1.3,
                ..id
            },
        ),
        (
            "P5",
            BeatParams {
                entrance: EntranceFamily::GlyphCascade,
                accent: AccentMark::Underline,
                stagger_order: StaggerOrder::CenterOut,
                ..id
            },
        ),
        (
            "P6",
            BeatParams {
                entrance: EntranceFamily::FocusPull,
                easing: EasingFamily::Smooth,
                amplitude: 0.7,
                ..id
            },
        ),
        (
            "P7",
            BeatParams {
                entrance: EntranceFamily::WipeUp,
                accent: AccentMark::Slab,
                travel: 0.7,
                ..id
            },
        ),
        (
            "P8",
            BeatParams {
                entrance: EntranceFamily::SlideLeft,
                accent: AccentMark::Stamp,
                easing: EasingFamily::Spring,
                ..id
            },
        ),
    ]
}

/// What an edge entrance (clip / mask reveal) becomes under a preset, written
/// out by hand (reveal kind, side, easing; `None` = the builder's own).
fn planned_edge(name: &str) -> (Option<Reveal>, Option<Direction>, Option<Easing>) {
    match name {
        "P1" => (
            Some(Reveal::Mask),
            Some(Direction::Left),
            Some(Easing::OutQuint),
        ),
        "P2" => (Some(Reveal::Clip), Some(Direction::Right), None),
        "P3" => (Some(Reveal::Clip), Some(Direction::Up), None),
        // ScalePop, GlyphCascade and FocusPull keep the builder's wipe.
        "P4" | "P5" => (None, None, None),
        "P6" => (None, None, Some(Easing::OutCubic)),
        "P7" => (Some(Reveal::Mask), Some(Direction::Up), None),
        "P8" => (
            Some(Reveal::Clip),
            Some(Direction::Left),
            Some(Easing::EditorialSpring),
        ),
        _ => unreachable!("{name}"),
    }
}

/// The easing a preset plans for a landing (`None` = the builder's own).
fn planned_easing(name: &str) -> Option<Easing> {
    planned_edge(name).2
}

/// The travel multiplier of a preset.
fn planned_travel(name: &str) -> f32 {
    match name {
        "P2" => 1.5,
        "P7" => 0.7,
        _ => 1.0,
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Builder {
    Emphasize,
    Contrast,
    Explain,
    Collection,
    Hero,
}

struct Fixture {
    name: &'static str,
    builder: Builder,
    /// A layer id (after the `b1.` prefix) the selected builder always draws.
    marker: &'static str,
    beat: Value,
    style: Value,
    art: bool,
}

fn phrase(v: &str) -> Value {
    json!({"kind": "phrase", "value": v})
}

fn labelled(v: &str, meaning: &str) -> Value {
    json!({"kind": "phrase", "value": v, "meaning": meaning})
}

fn object(meaning: &str, value: Option<&str>) -> Value {
    let mut o = json!({"kind": "object", "asset": "shopping_basket", "meaning": meaning});
    if let Some(v) = value {
        o["value"] = json!(v);
    }
    o
}

fn items() -> Value {
    json!({"kind": "collection", "meaning": "things packed",
        "items": [phrase("Water bottle"), phrase("Jacket"), phrase("Laptop"), phrase("Books")]})
}

fn beat(purpose: &str, statement: &str, primary: Value, secondary: Option<Value>) -> Value {
    let mut b = json!({"purpose": purpose, "statement": statement, "primary": primary,
        "energy": "building"});
    if let Some(s) = secondary {
        b["secondary"] = s;
    }
    b
}

fn with_relationship(mut b: Value, relationship: &str) -> Value {
    b["relationship"] = json!(relationship);
    b
}

fn fixtures() -> Vec<Fixture> {
    // Layered (halftone patches) without a moving camera or a forced language
    // that would hand the beat to another grammar.
    let layered = json!({"depth": "layered", "camera_style": "static",
        "motion_language": "sequential"});
    let minimal = json!({"depth": "layered", "camera_style": "static",
        "motion_language": "minimal"});
    let data = json!({"depth": "layered", "camera_style": "static",
        "motion_language": "data"});
    let fx = |name, builder, marker, beat, style: &Value, art| Fixture {
        name,
        builder,
        marker,
        beat,
        style: style.clone(),
        art,
    };
    vec![
        fx(
            "emphasize with a serif note",
            Builder::Emphasize,
            "hero.card",
            beat(
                "emphasize",
                "One idea carries the story",
                labelled("one idea", "focus"),
                Some(phrase("the rest is detail")),
            ),
            &layered,
            false,
        ),
        fx(
            "emphasize alone",
            Builder::Emphasize,
            "hero.card",
            beat(
                "emphasize",
                "One idea carries the story",
                labelled("one idea", "focus"),
                None,
            ),
            &layered,
            false,
        ),
        fx(
            "emphasize with a picture",
            Builder::Emphasize,
            "hero.card",
            beat(
                "emphasize",
                "One idea carries the story",
                labelled("one idea", "focus"),
                Some(object("a bag", None)),
            ),
            &layered,
            false,
        ),
        fx(
            "emphasize, minimal language",
            Builder::Emphasize,
            "hero_rule",
            beat(
                "emphasize",
                "One idea carries the story",
                labelled("one idea", "focus"),
                Some(phrase("the rest is detail")),
            ),
            &minimal,
            false,
        ),
        fx(
            "contrast with a stamped picture",
            Builder::Contrast,
            "zone_b.card",
            with_relationship(
                beat(
                    "contrast",
                    "Small habits, big shifts",
                    phrase("daily coffee"),
                    Some(object("a bag", Some("$5"))),
                ),
                "compress",
            ),
            &layered,
            false,
        ),
        fx(
            "explain",
            Builder::Explain,
            "body_rule",
            beat(
                "explain",
                "Why compounding works quietly",
                phrase("Compounding"),
                Some(phrase("interest earns interest")),
            ),
            &layered,
            false,
        ),
        fx(
            "collection, running total",
            Builder::Collection,
            "aggregate.slab",
            with_relationship(
                beat(
                    "emphasize",
                    "Small things fill the bag.",
                    items(),
                    Some(json!({"kind": "number", "value": "9 kg", "meaning": "on your back"})),
                ),
                "accumulate",
            ),
            &layered,
            false,
        ),
        fx(
            "collection, plain list with a consequence",
            Builder::Collection,
            "aggregate.slab",
            beat(
                "explain",
                "Four things in the bag",
                items(),
                Some(json!({"kind": "number", "value": "9 kg", "meaning": "on your back"})),
            ),
            &layered,
            false,
        ),
        fx(
            "collection, plain list",
            Builder::Collection,
            "items.0.card",
            beat("explain", "Four things in the bag", items(), None),
            &layered,
            false,
        ),
        // Numbers count up in the data language: the counts keep their timing.
        fx(
            "collection, numeric running total in the data language",
            Builder::Collection,
            "aggregate.slab",
            with_relationship(
                beat(
                    "emphasize",
                    "Small things fill the bag.",
                    json!({"kind": "collection", "meaning": "kilos packed", "items": [
                        {"kind": "number", "value": "1 kg", "meaning": "water"},
                        {"kind": "number", "value": "2 kg", "meaning": "jacket"},
                        {"kind": "number", "value": "3 kg", "meaning": "laptop"},
                        {"kind": "number", "value": "3 kg", "meaning": "books"}]}),
                    None,
                ),
                "accumulate",
            ),
            &data,
            false,
        ),
        fx(
            "hero object",
            Builder::Hero,
            "plate.halftone",
            beat(
                "emphasize",
                "A small treat adds up",
                object("treat", Some("$5")),
                None,
            ),
            &layered,
            false,
        ),
        fx(
            "hero object with a second picture",
            Builder::Hero,
            "plate.halftone",
            beat(
                "emphasize",
                "A small treat adds up",
                object("treat", Some("$5")),
                Some(object("a year", Some("$1,825"))),
            ),
            &layered,
            false,
        ),
        fx(
            "hero object with a note",
            Builder::Hero,
            "plate.halftone",
            beat(
                "emphasize",
                "A small treat adds up",
                object("treat", Some("$5")),
                Some(phrase("every single day")),
            ),
            &layered,
            false,
        ),
        // The look decides the grammar here (HeroObject through `--art`).
        fx(
            "hero object under art direction",
            Builder::Hero,
            "plate.halftone",
            beat(
                "emphasize",
                "A small treat adds up",
                object("treat", Some("$5")),
                Some(object("a year", Some("$1,825"))),
            ),
            &json!({"tone": "editorial"}),
            true,
        ),
    ]
}

fn tail() -> Value {
    json!({"purpose": "emphasize", "statement": "The end of the story",
        "primary": {"kind": "phrase", "value": "end"}, "energy": "calm"})
}

fn intent_of(f: &Fixture, format: &str) -> CreativeIntent {
    let v = json!({"version": "0.2", "title": "c2b", "format": format,
        "beats": [f.beat.clone(), tail()]});
    serde_json::from_value(v).expect("intent parses")
}

fn style_of(f: &Fixture) -> StyleProfile {
    serde_json::from_value(f.style.clone()).expect("style parses")
}

fn options(f: &Fixture) -> CompileOptions {
    CompileOptions {
        art: f.art.then_some(ArtMode::Auto),
        ..CompileOptions::default()
    }
}

/// Compile beat 1 of `f` with `params` (`None` = the plain compile, no hook).
fn build(f: &Fixture, format: &str, params: Option<BeatParams>) -> MotionProject {
    let intent = intent_of(f, format);
    let style = style_of(f);
    let library = AssetLibrary::new(ASSETS);
    match params {
        None if !f.art => compile(&intent, &style, &library, &ApproxMeasure).expect("compiles"),
        None => {
            compile_with_beat_params(
                &intent,
                &style,
                &library,
                &ApproxMeasure,
                &AssetManifest::empty(),
                &options(f),
                &BTreeMap::new(),
            )
            .expect("compiles")
            .0
        }
        Some(p) => {
            compile_with_beat_params(
                &intent,
                &style,
                &library,
                &ApproxMeasure,
                &AssetManifest::empty(),
                &options(f),
                &BTreeMap::from([(0usize, p)]),
            )
            .expect("compiles")
            .0
        }
    }
}

fn beat_scene(p: &MotionProject) -> &Scene {
    p.scenes.iter().find(|s| s.id == "beat_1").expect("beat_1")
}

// ---------------------------------------------------------------------------
// Reading a scene
// ---------------------------------------------------------------------------

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

/// (id, text) of every text layer.
fn texts(s: &Scene) -> BTreeSet<(String, String)> {
    all_layers(s)
        .into_iter()
        .filter_map(|l| match &l.kind {
            LayerKind::Text(t) => Some((l.id.clone(), t.text.clone())),
            _ => None,
        })
        .collect()
}

fn motions_of<'a>(s: &'a Scene, target: &str) -> Vec<&'a Motion> {
    s.motions.iter().filter(|m| m.target == target).collect()
}

/// The clip / mask reveal of `target`: (kind, side, easing).
fn edge(s: &Scene, target: &str) -> Option<(Reveal, Direction, Easing)> {
    motions_of(s, target).into_iter().find_map(|m| match m.op {
        MotionOp::ClipReveal { direction, .. } => Some((Reveal::Clip, direction, m.easing)),
        MotionOp::MaskReveal { direction, .. } => Some((Reveal::Mask, direction, m.easing)),
        _ => None,
    })
}

/// The entrance move of `target` (an offset easing to rest).
fn rise<'a>(s: &'a Scene, target: &str) -> Option<&'a Motion> {
    motions_of(s, target).into_iter().find(
        |m| matches!(m.op, MotionOp::Move { from, to } if to == [0.0, 0.0] && from != [0.0, 0.0]),
    )
}

fn pop<'a>(s: &'a Scene, target: &str) -> Option<&'a Motion> {
    motions_of(s, target)
        .into_iter()
        .find(|m| matches!(m.op, MotionOp::Scale { from, to, .. } if to == 1.0 && from != 1.0))
}

fn cascade<'a>(s: &'a Scene, target: &str) -> Option<&'a Motion> {
    motions_of(s, target)
        .into_iter()
        .find(|m| matches!(m.op, MotionOp::GlyphCascade { .. }))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// The motions the builder under test owns: everything but the shared
/// skeleton's (kicker, ghost, stage, headline and body / serif / connective
/// lines).
fn owned(s: &Scene) -> Vec<&Motion> {
    s.motions
        .iter()
        .filter(|m| {
            let t = m.target.as_str();
            !(t.ends_with(".kicker")
                || t.ends_with(".kicker_rule")
                || t.ends_with(".stage")
                || t.ends_with(".stage_front")
                || t.ends_with(".ghost")
                || t.contains(".head.")
                || t.ends_with(".head")
                || t.contains(".serif.")
                || t.contains(".body.")
                || t.contains(".connective."))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fixtures select their builder; identity builds the old beat
// ---------------------------------------------------------------------------

#[test]
fn every_fixture_selects_its_builder() {
    for f in fixtures() {
        let p = build(&f, "vertical", None);
        let s = beat_scene(&p);
        assert!(
            layer(s, &format!("b1.{}", f.marker)).is_some(),
            "{}: no {} ({:?})",
            f.name,
            f.marker,
            all_layers(s).iter().map(|l| &l.id).collect::<Vec<_>>()
        );
        // The builders' signature layers.
        let has = |id: &str| layer(s, &format!("b1.{id}")).is_some();
        match f.builder {
            Builder::Emphasize => assert!(has("hero.card"), "{}", f.name),
            Builder::Contrast => assert!(has("panel") && has("zone_b"), "{}", f.name),
            Builder::Explain => assert!(has("body_rule") && has("support.card"), "{}", f.name),
            Builder::Collection => assert!(has("items.0.card"), "{}", f.name),
            Builder::Hero => assert!(has("plate.halftone"), "{}", f.name),
        }
    }
}

#[test]
fn identity_params_build_the_old_beat() {
    for f in fixtures() {
        for format in ["vertical", "square"] {
            let plain = serde_json::to_string(&build(&f, format, None)).expect("json");
            let given = serde_json::to_string(&build(&f, format, Some(BeatParams::default())))
                .expect("json");
            assert_eq!(plain, given, "{} ({format})", f.name);
        }
    }
}

#[test]
fn a_preset_that_names_only_the_identity_fields_changes_nothing() {
    // Even and Forward are the identity of the stagger; Builder easing too.
    let near = BeatParams {
        stagger: StaggerPreset::Even,
        stagger_order: StaggerOrder::Forward,
        ..BeatParams::default()
    };
    for f in fixtures() {
        let a = serde_json::to_string(&build(&f, "vertical", None)).expect("json");
        let b = serde_json::to_string(&build(&f, "vertical", Some(near))).expect("json");
        assert_eq!(a, b, "{}", f.name);
    }
}

// ---------------------------------------------------------------------------
// Steering
// ---------------------------------------------------------------------------

/// The edge entrances of a fixture the preset plans: the layer ids (after the
/// prefix) whose clip / mask reveal belongs to the builder.
fn edge_targets(f: &Fixture) -> Vec<&'static str> {
    match f.builder {
        Builder::Emphasize => {
            let mut v = vec!["hero.card", "hero_rule", "serif_rule"];
            if f.style["depth"] == "layered" {
                v.push("hero.halftone");
            }
            v
        }
        Builder::Contrast => vec!["panel", "zone_b.card", "zone_b.halftone", "stamp"],
        Builder::Explain => vec!["support.card", "support.halftone", "body_rule"],
        Builder::Collection => vec!["items.0.card", "items.1.card", "aggregate.slab"],
        Builder::Hero => vec!["plate.halftone"],
    }
}

/// The edge entrances named by `edge_targets` that exist in `base`.
fn present(f: &Fixture, base: &Scene) -> Vec<String> {
    edge_targets(f)
        .into_iter()
        .map(|t| format!("b1.{t}"))
        .filter(|t| edge(base, t).is_some())
        .collect()
}

#[test]
fn every_preset_steers_the_edge_entrances_of_every_builder() {
    for f in fixtures() {
        let base_project = build(&f, "vertical", None);
        let base = beat_scene(&base_project);
        let targets = present(&f, base);
        assert!(!targets.is_empty(), "{}: nothing to steer", f.name);
        for (name, bp) in presets() {
            let p = build(&f, "vertical", Some(bp));
            let s = beat_scene(&p);
            let (reveal, side, easing) = planned_edge(name);
            for t in &targets {
                let (b_reveal, b_side, b_easing) = edge(base, t).expect("base edge");
                // A halftone patch is a decoration: `None` (P4) leaves it out.
                if (t.ends_with(".halftone") || t.ends_with("_rule"))
                    && bp.accent == AccentMark::None
                {
                    assert!(
                        layer(s, t).is_none() && edge(s, t).is_none(),
                        "{} {name}: {t}",
                        f.name
                    );
                    continue;
                }
                // A stamp (a value text) cascades glyph by glyph under
                // GlyphCascade, rising behind a fade, instead of wiping.
                if t.ends_with(".stamp") && bp.entrance == EntranceFamily::GlyphCascade {
                    assert!(
                        cascade(s, t).is_some() && edge(s, t).is_none(),
                        "{} {name}: {t}",
                        f.name
                    );
                    assert!(motions_of(s, t).iter().any(
                        |m| matches!(m.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0)
                    ));
                    continue;
                }
                let got =
                    edge(s, t).unwrap_or_else(|| panic!("{} {name}: {t} lost its wipe", f.name));
                // The stamp's strike is the builder's; `Stamp` makes it a hard one.
                let own = if t.ends_with(".stamp") && bp.accent == AccentMark::Stamp {
                    Easing::ImpactSpring
                } else {
                    b_easing
                };
                let want = (
                    reveal.unwrap_or(b_reveal),
                    side.unwrap_or(b_side),
                    easing.unwrap_or(own),
                );
                assert_eq!(got, want, "{} {name}: {t}", f.name);
            }
        }
    }
}

/// The entrance moves (a layer rising in) of `base`, as target and rise.
fn rises(base: &Scene) -> Vec<(String, f32)> {
    base.motions
        .iter()
        .filter_map(|m| match m.op {
            MotionOp::Move { from, to } if to == [0.0, 0.0] && from != [0.0, 0.0] => {
                Some((m.target.clone(), from[1]))
            }
            _ => None,
        })
        .filter(|(t, _)| owned(base).iter().any(|m| m.target == *t))
        .collect()
}

#[test]
fn every_preset_steers_the_layers_the_shared_placement_enters() {
    let mut checked = 0;
    for f in fixtures() {
        let base_project = build(&f, "vertical", None);
        let base = beat_scene(&base_project);
        // The placed layers: entrance moves of owned motions that are not a
        // headline line or a skeleton layer (`owned` already drops those).
        let layers: Vec<(String, f32)> = rises(base);
        for (name, bp) in presets() {
            let p = build(&f, "vertical", Some(bp));
            let s = beat_scene(&p);
            for (target, y) in &layers {
                checked += 1;
                let base_move = rise(base, target).expect("base rise");
                let mine = rise(s, target);
                let glyph = layer(s, target).and_then(|l| match &l.kind {
                    LayerKind::Text(t) => {
                        Some(t.text.chars().filter(|c| !c.is_whitespace()).count())
                    }
                    _ => None,
                });
                let counts = motions_of(s, target)
                    .iter()
                    .any(|m| matches!(m.op, MotionOp::Count { .. }));
                match name {
                    "P4" if pop(s, target).is_some() && mine.is_none() => {
                        // A scale pop in place of the rise; the amplitude scales it.
                        let m = pop(s, target).expect("pop");
                        let MotionOp::Scale { from, to, .. } = m.op else {
                            unreachable!()
                        };
                        assert!(close(to, 1.0));
                        assert!(close(from, 1.0 - 0.12 * 1.3), "{} {target}: {from}", f.name);
                        assert!(close_f64(m.start, base_move.start), "{} {target}", f.name);
                    }
                    "P5" if cascade(s, target).is_some() && mine.is_none() => {
                        let m = cascade(s, target).expect("cascade");
                        let MotionOp::GlyphCascade { order, from, .. } = m.op else {
                            unreachable!()
                        };
                        assert_eq!(order, GlyphOrder::Center, "{} {target}", f.name);
                        assert!(close(from.dy, *y), "{} {target}", f.name);
                        assert!(
                            glyph.is_some_and(|n| n >= 2) && !counts,
                            "{} {target}",
                            f.name
                        );
                    }
                    _ => {
                        let mine = mine
                            .unwrap_or_else(|| panic!("{} {name}: {target} lost its rise", f.name));
                        let MotionOp::Move { from, .. } = mine.op else {
                            unreachable!()
                        };
                        assert!(
                            close(from[1], y * planned_travel(name)),
                            "{} {name} {target}: {} vs {}",
                            f.name,
                            from[1],
                            y * planned_travel(name)
                        );
                        let own = planned_easing(name).unwrap_or(base_move.easing);
                        assert_eq!(mine.easing, own, "{} {name} {target}", f.name);
                        // Falling back is only allowed where the preset cannot
                        // draw its gesture here.
                        if name == "P4" {
                            assert!(
                                pop(s, target).is_none() || mine.target != *target,
                                "{} {target}",
                                f.name
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 40, "only {checked} placed entrances were checked");
}

fn close_f64(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn every_non_identity_preset_changes_what_each_builder_owns() {
    for f in fixtures() {
        let base_project = build(&f, "vertical", None);
        let base = beat_scene(&base_project);
        let base_owned = owned(base);
        for (name, bp) in presets() {
            let p = build(&f, "vertical", Some(bp));
            let s = beat_scene(&p);
            assert_ne!(
                base_owned,
                owned(s),
                "{} {name}: the builder's own motions did not change",
                f.name
            );
        }
    }
}

#[test]
fn accent_none_removes_decorations_and_never_text() {
    let none = BeatParams {
        accent: AccentMark::None,
        ..BeatParams::default()
    };
    // (fixture index, decorations that go)
    let gone: &[(&str, &[&str])] = &[
        (
            "emphasize with a serif note",
            &["hero.halftone", "serif_rule"],
        ),
        ("emphasize, minimal language", &["hero_rule", "serif_rule"]),
        ("contrast with a stamped picture", &["zone_b.halftone"]),
        ("explain", &["body_rule", "support.halftone"]),
        ("hero object", &["plate.halftone"]),
    ];
    for f in fixtures() {
        let Some((_, ids)) = gone.iter().find(|(n, _)| *n == f.name) else {
            continue;
        };
        let base_project = build(&f, "vertical", None);
        let p = build(&f, "vertical", Some(none));
        let (base, s) = (beat_scene(&base_project), beat_scene(&p));
        for id in *ids {
            let id = format!("b1.{id}");
            assert!(
                layer(base, &id).is_some(),
                "{}: {id} missing at identity",
                f.name
            );
            assert!(layer(s, &id).is_none(), "{}: {id} stayed", f.name);
            assert!(
                motions_of(s, &id).is_empty(),
                "{}: {id} still moves",
                f.name
            );
        }
        assert_eq!(texts(base), texts(s), "{}", f.name);
    }
    // Collections draw no decoration (the slab holds the aggregate, the bar
    // is the count): the accent falls back, nothing the builder owns changes
    // (only the skeleton's kicker rule goes).
    for f in fixtures()
        .iter()
        .filter(|f| f.builder == Builder::Collection)
    {
        let a = build(f, "vertical", None);
        let b = build(f, "vertical", Some(none));
        let (a, b) = (beat_scene(&a), beat_scene(&b));
        assert_eq!(owned(a), owned(b), "{}", f.name);
        let ids = |s: &Scene| -> BTreeSet<String> {
            all_layers(s)
                .into_iter()
                .map(|l| l.id.clone())
                .filter(|id| !id.ends_with(".kicker_rule"))
                .collect()
        };
        assert_eq!(ids(a), ids(b), "{}", f.name);
    }
}

#[test]
fn rule_and_underline_redraw_the_decoration_in_that_shape() {
    let shaped = |accent| BeatParams {
        accent,
        ..BeatParams::default()
    };
    let by = |name: &str| {
        fixtures()
            .into_iter()
            .find(|f| f.name == name)
            .expect("fixture")
    };
    // The hero rule is a short bar; Underline makes it as wide as the card.
    let f = by("emphasize, minimal language");
    let base = build(&f, "vertical", None);
    let rule = layer(beat_scene(&base), "b1.hero_rule").expect("rule");
    let under = build(&f, "vertical", Some(shaped(AccentMark::Underline)));
    let u = layer(beat_scene(&under), "b1.hero_rule").expect("underline");
    assert!(u.width > rule.width * 3.0 && u.height < rule.height);
    assert!(close(u.x, rule.x) && close(u.y, rule.y));
    // The serif underline is already as long as the note; Rule shortens it.
    let ruled = build(&f, "vertical", Some(shaped(AccentMark::Rule)));
    let s_base = layer(beat_scene(&base), "b1.serif_rule").expect("serif rule");
    let s_rule = layer(beat_scene(&ruled), "b1.serif_rule").expect("serif bar");
    assert!(s_rule.width < s_base.width && close(s_rule.height, s_base.height));
    let s_under = build(&f, "vertical", Some(shaped(AccentMark::Underline)));
    let same = layer(beat_scene(&s_under), "b1.serif_rule").expect("serif rule");
    assert!(close(same.width, s_base.width) && close(same.height, s_base.height));
    // Explain: the body rule is a short bar; Underline runs along the copy.
    let f = by("explain");
    let base = build(&f, "vertical", None);
    let bar = layer(beat_scene(&base), "b1.body_rule").expect("body rule");
    let line = build(&f, "vertical", Some(shaped(AccentMark::Underline)));
    let l = layer(beat_scene(&line), "b1.body_rule").expect("body line");
    assert!(l.height < bar.height && close(l.y, bar.y) && close(l.x, bar.x));
    assert!(!close(l.width, bar.width));
    // Slab and Stamp are marks these builders do not draw as decoration: the
    // decoration stays the builder's own.
    for accent in [AccentMark::Slab, AccentMark::Stamp] {
        let p = build(&f, "vertical", Some(shaped(accent)));
        let r = layer(beat_scene(&p), "b1.body_rule").expect("body rule");
        assert!(close(r.width, bar.width) && close(r.height, bar.height));
    }
}

#[test]
fn stamp_makes_the_value_stamps_land_with_a_strike() {
    let stamp = BeatParams {
        accent: AccentMark::Stamp,
        ..BeatParams::default()
    };
    for name in [
        "contrast with a stamped picture",
        "hero object",
        "hero object with a second picture",
    ] {
        let f = fixtures()
            .into_iter()
            .find(|f| f.name == name)
            .expect("fixture");
        let p = build(&f, "vertical", Some(stamp));
        let s = beat_scene(&p);
        let strikes = s
            .motions
            .iter()
            .filter(|m| m.target.ends_with(".stamp") || m.target.ends_with(".support_stamp"))
            .filter(|m| matches!(m.op, MotionOp::ClipReveal { .. }))
            .collect::<Vec<_>>();
        assert!(!strikes.is_empty(), "{name}");
        for m in strikes {
            assert_eq!(m.easing, Easing::ImpactSpring, "{name} {}", m.target);
        }
    }
}

#[test]
fn amplitude_scales_the_emphasis_never_the_layout() {
    let amp = |a: f32| BeatParams {
        amplitude: a,
        ..BeatParams::default()
    };
    let by = |name: &str| {
        fixtures()
            .into_iter()
            .find(|f| f.name == name)
            .expect("fixture")
    };
    // The hero's slow crop (1.04) and the card's settling tilt.
    let f = by("emphasize alone");
    let base = build(&f, "vertical", None);
    let big = build(&f, "vertical", Some(amp(1.3)));
    let (base, big) = (beat_scene(&base), beat_scene(&big));
    let crop = |s: &Scene| {
        s.motions
            .iter()
            .find_map(|m| match m.op {
                MotionOp::Scale { from, to, .. }
                    if from == 1.0 && m.target.contains(".hero.") && to > 1.0 =>
                {
                    Some(to)
                }
                _ => None,
            })
            .expect("crop")
    };
    assert!(crop(big) > crop(base));
    assert!(close(crop(big) - 1.0, (crop(base) - 1.0) * 1.3));
    let tilt = |s: &Scene| {
        s.motions
            .iter()
            .find_map(|m| match m.op {
                MotionOp::Rotate { from, .. } if m.target == "b1.hero.card" => Some(from),
                _ => None,
            })
            .expect("tilt")
    };
    assert!(close(tilt(big), tilt(base) * 1.3));
    // Positions and sizes stay put.
    for id in ["b1.hero.card", "b1.hero.halftone"] {
        let (a, b) = (
            layer(base, id).expect("base layer"),
            layer(big, id).expect("layer"),
        );
        assert!(close(a.x, b.x) && close(a.y, b.y) && close(a.width, b.width));
    }
    // The aggregate's landing pop (collection) and the plate's pop (hero).
    let f = by("collection, running total");
    let base = build(&f, "vertical", None);
    let big = build(&f, "vertical", Some(amp(1.3)));
    let landing = |s: &Scene| match pop(s, "b1.aggregate").expect("landing").op {
        MotionOp::Scale { from, .. } => from,
        _ => unreachable!(),
    };
    assert!(close(
        landing(beat_scene(&big)) - 1.0,
        (landing(beat_scene(&base)) - 1.0) * 1.3
    ));
    let f = by("hero object");
    let base = build(&f, "vertical", None);
    let big = build(&f, "vertical", Some(amp(1.3)));
    let plate = |s: &Scene| match pop(s, "b1.plate") {
        Some(m) => match m.op {
            MotionOp::Scale { from, .. } => from,
            _ => unreachable!(),
        },
        None => panic!("no plate pop"),
    };
    assert!(close(
        plate(beat_scene(&big)) - 1.0,
        (plate(beat_scene(&base)) - 1.0) * 1.3
    ));
}

#[test]
fn collections_stagger_their_arrivals_without_breaking_the_chain() {
    let by = |name: &str| {
        fixtures()
            .into_iter()
            .find(|f| f.name == name)
            .expect("fixture")
    };
    let starts = |s: &Scene, n: usize| -> Vec<f64> {
        (0..n)
            .map(|i| {
                motions_of(s, &format!("b1.items.{i}.card"))
                    .into_iter()
                    .find(|m| {
                        matches!(
                            m.op,
                            MotionOp::MaskReveal { .. } | MotionOp::ClipReveal { .. }
                        )
                    })
                    .expect("card wipe")
                    .start
            })
            .collect()
    };
    let with = |pace, order| BeatParams {
        stagger: pace,
        stagger_order: order,
        ..BeatParams::default()
    };
    for name in ["collection, running total", "collection, plain list"] {
        let f = by(name);
        let base_project = build(&f, "vertical", None);
        let base = starts(beat_scene(&base_project), 4);
        for pace in [StaggerPreset::Tight, StaggerPreset::Loose] {
            let p = build(&f, "vertical", Some(with(pace, StaggerOrder::Forward)));
            let got = starts(beat_scene(&p), 4);
            // The first card stays, none arrives after the last one did, and
            // the arrivals stay in order, at least 0.3 s apart (the chain's
            // own rule).
            assert!(close_f64(got[0], base[0]), "{name} {pace:?}");
            assert!(
                got[3] <= base[3] + 1e-9,
                "{name} {pace:?}: {got:?} vs {base:?}"
            );
            assert!(
                got.windows(2).all(|w| w[1] - w[0] >= 0.3 - 1e-9),
                "{name} {pace:?}: {got:?}"
            );
            match pace {
                // Tight pulls the cards toward the first.
                StaggerPreset::Tight => assert!(
                    got[1] < base[1] && got[3] < base[3],
                    "{name}: {got:?} vs {base:?}"
                ),
                // Loose pushes the middle cards later, toward the last.
                _ => {
                    assert!(close_f64(got[3], base[3]), "{name}: {got:?} vs {base:?}");
                    assert!(got[1] > base[1], "{name}: {got:?} vs {base:?}");
                }
            }
        }
        for order in [StaggerOrder::Reverse, StaggerOrder::CenterOut] {
            let p = build(&f, "vertical", Some(with(StaggerPreset::Builder, order)));
            let got = starts(beat_scene(&p), 4);
            let mut a = base.clone();
            let mut b = got.clone();
            a.sort_by(|x, y| x.total_cmp(y));
            b.sort_by(|x, y| x.total_cmp(y));
            assert_eq!(a, b, "{name} {order:?}: the same arrival times");
            if name.contains("running total") {
                // The total counts its cards in order.
                assert_eq!(got, base, "{name} {order:?}");
            } else {
                assert_ne!(got, base, "{name} {order:?}");
            }
        }
    }
    // The running total keeps its steps and the aggregate its time.
    let f = by("collection, running total");
    let base = build(&f, "vertical", None);
    let p = build(
        &f,
        "vertical",
        Some(with(StaggerPreset::Tight, StaggerOrder::Reverse)),
    );
    let steps = |s: &Scene| {
        motions_of(s, "b1.total")
            .into_iter()
            .filter(|m| matches!(m.op, MotionOp::Count { .. }))
            .count()
    };
    assert_eq!(steps(beat_scene(&base)), 4);
    assert_eq!(steps(beat_scene(&p)), 4);
    let slab = |s: &Scene| {
        edge(s, "b1.aggregate.slab").map(|_| motions_of(s, "b1.aggregate.slab")[0].start)
    };
    assert_eq!(slab(beat_scene(&base)), slab(beat_scene(&p)));
}

#[test]
fn scale_pop_and_glyph_cascade_land_where_the_builder_has_a_place_for_them() {
    let by = |name: &str| {
        fixtures()
            .into_iter()
            .find(|f| f.name == name)
            .expect("fixture")
    };
    let entrance = |e| BeatParams {
        entrance: e,
        ..BeatParams::default()
    };
    // Collection: every card's content pops, or its text cascades.
    let f = by("collection, plain list");
    let popped = build(&f, "vertical", Some(entrance(EntranceFamily::ScalePop)));
    let cascaded = build(&f, "vertical", Some(entrance(EntranceFamily::GlyphCascade)));
    for i in 0..4 {
        let id = format!("b1.items.{i}.content");
        assert!(pop(beat_scene(&popped), &id).is_some(), "{id}");
        assert!(cascade(beat_scene(&cascaded), &id).is_some(), "{id}");
        // The fade stays: the content is never visible before its time.
        for p in [&popped, &cascaded] {
            assert!(motions_of(beat_scene(p), &id)
                .iter()
                .any(|m| matches!(m.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0)));
        }
    }
    // Hero object: the stamped figure pops / cascades, keeping its reveal.
    let f = by("hero object");
    let popped = build(&f, "vertical", Some(entrance(EntranceFamily::ScalePop)));
    let cascaded = build(&f, "vertical", Some(entrance(EntranceFamily::GlyphCascade)));
    assert!(pop(beat_scene(&popped), "b1.stamp").is_some());
    let c = cascade(beat_scene(&cascaded), "b1.stamp").expect("stamp cascade");
    assert!(motions_of(beat_scene(&cascaded), "b1.stamp")
        .iter()
        .any(|m| matches!(m.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0)));
    assert!(c.duration > 0.0);
    // EditorialCollage: the hero text pops or cascades in place of its rise,
    // and the hero card pops.
    for name in ["emphasize with a serif note", "emphasize alone"] {
        let f = by(name);
        let popped = build(&f, "vertical", Some(entrance(EntranceFamily::ScalePop)));
        let cascaded = build(&f, "vertical", Some(entrance(EntranceFamily::GlyphCascade)));
        let (popped, cascaded) = (beat_scene(&popped), beat_scene(&cascaded));
        let hero = "b1.hero.one_idea";
        assert!(
            pop(popped, hero).is_some() && rise(popped, hero).is_none(),
            "{name}"
        );
        // The hero's slow crop (no secondary) starts after the pop ends.
        let pop_end = pop(popped, hero)
            .map(|m| m.start + m.duration)
            .expect("pop");
        assert!(popped
            .motions
            .iter()
            .filter(|m| m.target == hero && matches!(m.op, MotionOp::Scale { .. }))
            .all(|m| m.start + m.duration <= pop_end + 1e-9 || m.start >= pop_end - 1e-9));
        assert!(pop(popped, "b1.hero.card").is_some(), "{name}");
        assert!(
            cascade(cascaded, hero).is_some() && rise(cascaded, hero).is_none(),
            "{name}"
        );
        assert!(
            cascade(popped, hero).is_none() && pop(cascaded, hero).is_none(),
            "{name}"
        );
    }
    // FocusPull blurs nothing here: the beat is the identity.
    for f in fixtures() {
        let a = serde_json::to_string(&build(&f, "vertical", None)).expect("json");
        let b = serde_json::to_string(&build(
            &f,
            "vertical",
            Some(entrance(EntranceFamily::FocusPull)),
        ))
        .expect("json");
        assert_eq!(a, b, "{}", f.name);
    }
}

// ---------------------------------------------------------------------------
// Safety under every preset
// ---------------------------------------------------------------------------

/// Layer ids whose count would be flagged (`checks::COUNT_UNSETTLED` without a
/// voice-over): the final text shows more than `COUNT_SETTLE_S` after READ (or
/// after the last card a running total counts), or is held less than
/// `COUNT_HOLD_S` before the exit although the layer arrives well before it.
fn unsettled(project: &MotionProject) -> BTreeSet<String> {
    let fps = project.canvas.fps;
    let mut out = BTreeSet::new();
    for scene in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let Some(life) = scene.lifecycle else {
            continue;
        };
        let mut last: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, m) in scene.motions.iter().enumerate() {
            if matches!(m.op, MotionOp::Count { .. }) {
                last.insert(&m.target, i);
            }
        }
        for (id, i) in last {
            let m = &scene.motions[i];
            let MotionOp::Count {
                to,
                decimals,
                grouping,
                prefix,
                suffix,
                ..
            } = &m.op
            else {
                continue;
            };
            let final_text = format_count(*to, *decimals, *grouping, prefix, suffix);
            let first = ((scene.start_seconds + m.start) * f64::from(fps)).floor() as u32;
            let end =
                ((scene.start_seconds + m.start + m.duration) * f64::from(fps)).ceil() as u32 + 1;
            let mut settle = f64::INFINITY;
            for n in first..=end {
                let frame = evaluate_frame(project, n).expect("evaluate");
                if text_in(&frame.layers, id).as_deref() == Some(final_text.as_str()) {
                    settle = frame_time(fps, n) - scene.start_seconds;
                    break;
                }
            }
            let steps = scene
                .motions
                .iter()
                .filter(|o| o.target == id && matches!(o.op, MotionOp::Count { .. }))
                .count();
            let anchor = match id.strip_suffix(".total") {
                Some(pre) => {
                    let item = format!("{pre}.items.{}", steps.saturating_sub(1));
                    let nested = format!("{item}.");
                    scene
                        .motions
                        .iter()
                        .filter(|o| o.target == item || o.target.starts_with(&nested))
                        .map(|o| o.start)
                        .reduce(f64::min)
                        .unwrap_or(life.read)
                }
                None => life.read.max(m.start),
            };
            let exit = scene
                .motions
                .iter()
                .filter(|o| o.target == id && o.start >= m.start)
                .filter_map(|o| match o.op {
                    MotionOp::Fade { from, to } if to < from => Some(o.start),
                    _ => None,
                })
                .fold(life.anticipate, f64::min);
            let limited = exit - m.start < COUNT_HOLD_S + 0.4;
            if !settle.is_finite()
                || settle - anchor > COUNT_SETTLE_S + 1e-9
                || (exit - settle < COUNT_HOLD_S - 1e-9 && !limited)
            {
                out.insert(format!("{}:{id}", scene.id));
            }
        }
    }
    out
}

fn text_in(layers: &[ResolvedLayer<'_>], id: &str) -> Option<String> {
    for l in layers {
        if l.id == id {
            return match l.kind {
                LayerKind::Text(style) => {
                    Some(l.text.clone().unwrap_or_else(|| style.text.clone()))
                }
                _ => None,
            };
        }
        if let Some(t) = text_in(&l.children, id) {
            return Some(t);
        }
    }
    None
}

/// The findings of the structural layout report, as (scene, layer, check).
fn findings(project: &MotionProject, format: Format) -> BTreeSet<(String, String, String)> {
    layout_report_with(
        project,
        &LayoutFrame::for_format(format),
        &ImageIndex::default(),
    )
    .findings
    .into_iter()
    .map(|f| (f.scene, f.layer, f.check.name().to_string()))
    .collect()
}

fn reveals(project: &MotionProject) -> String {
    let map = project
        .project
        .art
        .as_ref()
        .map(|a| a.reveals.clone())
        .unwrap_or_default();
    serde_json::to_string(&map).expect("json")
}

#[test]
fn every_builder_stays_safe_under_every_preset() {
    let mut runs = 0;
    for f in fixtures() {
        for (format, fmt) in [("vertical", Format::Vertical), ("square", Format::Square)] {
            let base_project = build(&f, format, None);
            let base = beat_scene(&base_project);
            let base_texts = texts(base);
            let base_findings = findings(&base_project, fmt);
            let base_counts = unsettled(&base_project);
            let base_reveals = reveals(&base_project);
            for (name, bp) in presets() {
                runs += 1;
                let tag = format!("{} / {format} / {name}", f.name);
                let p = build(&f, format, Some(bp));
                validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("{tag}: {e:?}"));
                let s = beat_scene(&p);
                // Every value, label and item is still there.
                assert_eq!(base_texts, texts(s), "{tag}: texts changed");
                // The anchors are the same ids and roles.
                assert_eq!(base_reveals, reveals(&p), "{tag}: reveal anchors changed");
                // No motion ends after the scene.
                for sc in p.scenes.iter().filter(|x| x.id.starts_with("beat_")) {
                    for m in &sc.motions {
                        assert!(
                            m.start + m.duration <= sc.duration_seconds + 1e-6,
                            "{tag}: {} {:?} ends at {}",
                            m.target,
                            m.op.op_name(),
                            m.start + m.duration
                        );
                    }
                }
                // No count settles later than at the identity.
                let counts = unsettled(&p);
                assert!(
                    counts.is_subset(&base_counts),
                    "{tag}: unsettled counts {counts:?} (identity {base_counts:?})"
                );
                // The layout report gains no finding (text_overlap included).
                let now = findings(&p, fmt);
                let new: Vec<_> = now.difference(&base_findings).collect();
                assert!(new.is_empty(), "{tag}: new layout findings {new:?}");
            }
        }
    }
    assert_eq!(runs, fixtures().len() * 2 * 8);
}

#[test]
fn the_reveal_anchors_are_kept_under_art_direction() {
    // `support_stamp` is the one anchor these builders declare.
    let f = fixtures().into_iter().find(|f| f.art).expect("art fixture");
    let base = build(&f, "vertical", None);
    let anchors = reveals(&base);
    assert!(anchors.contains("support_stamp"), "{anchors}");
    for (name, bp) in presets() {
        let p = build(&f, "vertical", Some(bp));
        assert_eq!(anchors, reveals(&p), "{name}");
    }
}

// ---------------------------------------------------------------------------
// Whole stories on the product path, every preset on every beat
// ---------------------------------------------------------------------------

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

/// Every bench story compiled the way the product path compiles it (art
/// direction, a variety seed, the committed speech fixture) with one preset on
/// every beat: it compiles, validates, drops no value the identity shows, and
/// the layout report gains no finding.
#[test]
fn every_preset_on_every_beat_keeps_the_product_path_safe() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = AssetLibrary::new(repo.join("assets"));
    let mut runs = 0;
    for (name, path) in STORIES {
        let read =
            |p: &str| std::fs::read_to_string(repo.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
        let intent = CreativeIntent::from_json(&read(path)).expect("intent");
        let map = SpeechMap::from_json(&read(&format!(
            "golden/fixtures/variety/{name}.speech.json"
        )))
        .expect("speech fixture");
        let spoken: Vec<String> = intent
            .beats
            .iter()
            .map(|b| display_text(b, true).spoken)
            .collect();
        let speech = repair(&map, &spoken).0;
        let format = intent.format;
        for tone in ["auto", "editorial", "technical", "playful"] {
            let style: StyleProfile =
                serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style");
            let opts = CompileOptions {
                art: Some(ArtMode::Auto),
                variety: Some(story_seed(&intent)),
                speech: Some(speech.clone()),
                ..CompileOptions::default()
            };
            let go = |params: &BTreeMap<usize, BeatParams>| {
                compile_with_beat_params(
                    &intent,
                    &style,
                    &library,
                    &ApproxMeasure,
                    &AssetManifest::empty(),
                    &opts,
                    params,
                )
                .unwrap_or_else(|e| panic!("{name} x {tone}: {e}"))
            };
            let dropped = |w: &[CompileWarning]| -> BTreeSet<Option<usize>> {
                w.iter()
                    .filter(|w| w.code == WARN_VALUE_DROPPED)
                    .map(|w| w.beat)
                    .collect()
            };
            let (base, base_warnings) = go(&BTreeMap::new());
            let (base_dropped, base_findings) = (dropped(&base_warnings), findings(&base, format));
            for (preset, bp) in presets() {
                runs += 1;
                let tag = format!("{name} x {tone} x {preset}");
                let params: BTreeMap<usize, BeatParams> =
                    (0..intent.beats.len()).map(|i| (i, bp)).collect();
                let (p, w) = go(&params);
                validate(&p, Some(Path::new(ASSETS))).unwrap_or_else(|e| panic!("{tag}: {e:?}"));
                assert!(
                    dropped(&w).is_subset(&base_dropped),
                    "{tag}: a value was dropped"
                );
                let new: Vec<_> = findings(&p, format)
                    .difference(&base_findings)
                    .cloned()
                    .collect();
                assert!(new.is_empty(), "{tag}: new layout findings {new:?}");
            }
        }
    }
    assert_eq!(runs, STORIES.len() * 4 * 8);
}

// ---------------------------------------------------------------------------
// The visual check (not a behaviour test)
// ---------------------------------------------------------------------------

/// With `C2B_VISUAL_DIR` set, write the scenes the visual check renders:
/// an EditorialCollage beat, a collection (running total) beat and a
/// hero_object beat (the first beat of habit_math under art direction), each at
/// the identity and under three presets, as `<builder>__<preset>.motion.json`.
/// Without the variable this does nothing. Render them with `motion-engine
/// render` and build the sheets from the MP4s.
#[test]
fn writes_the_visual_check_scenes_when_asked() {
    let Some(dir) = std::env::var_os("C2B_VISUAL_DIR") else {
        return;
    };
    let dir = Path::new(&dir);
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let library = AssetLibrary::new(repo.join("assets"));
    let read =
        |p: &str| std::fs::read_to_string(repo.join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let story = |p: &str, keep: usize| {
        let mut v: Value = serde_json::from_str(&read(p)).expect("json");
        if let Some(beats) = v["beats"].as_array_mut() {
            beats.truncate(keep);
        }
        serde_json::from_value::<CreativeIntent>(v).expect("intent")
    };
    let editorial: StyleProfile = serde_json::from_str(r#"{"tone": "editorial"}"#).expect("style");
    let fixture = fixtures()
        .into_iter()
        .find(|f| f.name == "emphasize with a serif note")
        .expect("fixture");
    let scenes: Vec<(&str, CreativeIntent, StyleProfile, bool)> = vec![
        (
            "emphasize",
            intent_of(&fixture, "vertical"),
            style_of(&fixture),
            false,
        ),
        (
            "collection",
            story("examples/public/collection-accumulate.intent.json", 1),
            editorial.clone(),
            false,
        ),
        (
            "hero",
            story("docs/plans/sprint_0_23/stories/habit_math.intent.json", 2),
            editorial,
            true,
        ),
    ];
    let wanted = ["P4", "P5", "P7"];
    for (name, intent, style, art) in scenes {
        let opts = CompileOptions {
            art: art.then_some(ArtMode::Auto),
            ..CompileOptions::default()
        };
        let mut runs: Vec<(String, BeatParams)> = vec![("identity".into(), BeatParams::default())];
        runs.extend(
            presets()
                .into_iter()
                .filter(|(n, _)| wanted.contains(n))
                .map(|(n, p)| (n.to_string(), p)),
        );
        for (preset, bp) in runs {
            let (project, _) = compile_with_beat_params(
                &intent,
                &style,
                &library,
                &ApproxMeasure,
                &AssetManifest::empty(),
                &opts,
                &BTreeMap::from([(0usize, bp)]),
            )
            .expect("compiles");
            let path = dir.join(format!("{name}__{preset}.motion.json"));
            std::fs::write(&path, serde_json::to_string(&project).expect("json")).expect("write");
        }
    }
}
