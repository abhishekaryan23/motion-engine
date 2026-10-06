//! KineticPoster and atomic SplitContrast composition tests: compile small
//! intents through the public API and assert structure (ids, ordering,
//! lifecycle scheduling, validity), never exact design values.

use std::path::PathBuf;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;
use serde_json::{json, Value};

const EPS: f64 = 1e-6;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn style(language: &str, seed: u64) -> StyleProfile {
    let text = std::fs::read_to_string(repo().join("examples/public/minimal.style.json"))
        .expect("public style");
    let mut v: Value = serde_json::from_str(&text).expect("style json");
    v["motion_language"] = json!(language);
    v["seed"] = json!(seed);
    serde_json::from_value(v).expect("style parses")
}

fn intent(format: &str, beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2",
        "title": "grammar_typographic",
        "format": format,
        "beats": beats,
    }))
    .expect("intent parses")
}

fn build(i: &CreativeIntent, s: &StyleProfile) -> MotionProject {
    let p = compile(
        i,
        s,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .expect("compiles");
    if let Err(e) = validate(&p, Some(&repo().join("assets"))) {
        panic!("scene fails validation: {e:?}");
    }
    p
}

fn flatten<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            flatten(children, out);
        }
    }
}

fn layers(s: &Scene) -> Vec<&Layer> {
    let mut out = Vec::new();
    flatten(&s.layers, &mut out);
    out
}

fn beat(p: &MotionProject, n: usize) -> &Scene {
    let id = format!("beat_{n}");
    p.scenes
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("{id} missing"))
}

fn layer<'a>(s: &'a Scene, id: &str) -> &'a Layer {
    layers(s)
        .into_iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("layer '{id}' missing in {}", s.id))
}

fn ids_with_prefix(s: &Scene, prefix: &str) -> Vec<String> {
    layers(s)
        .into_iter()
        .filter(|l| l.id.starts_with(prefix))
        .map(|l| l.id.clone())
        .collect()
}

fn motions_on<'a>(s: &'a Scene, target: &'a str) -> impl Iterator<Item = &'a Motion> {
    s.motions.iter().filter(move |m| m.target == target)
}

fn has_op(s: &Scene, target: &str, op: &str) -> usize {
    motions_on(s, target)
        .filter(|m| m.op.op_name() == op)
        .count()
}

fn text_of(l: &Layer) -> &str {
    match &l.kind {
        LayerKind::Text(t) => &t.text,
        _ => panic!("{} is not text", l.id),
    }
}

/// Nothing but anticipation motions starts at/after ANTICIPATE.
fn assert_lifecycle_schedule(p: &MotionProject) {
    for s in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let life = s.lifecycle.expect("compiled scenes carry a lifecycle");
        for m in &s.motions {
            if m.start >= life.anticipate - EPS {
                assert!(
                    // Stage wrapper and parallax ghost fade are anticipation motions.
                    m.target.ends_with(".stage") || m.target.ends_with(".ghost"),
                    "{}: '{}' {} starts at {} >= anticipate {}",
                    s.id,
                    m.target,
                    m.op.op_name(),
                    m.start,
                    life.anticipate
                );
            }
        }
    }
}

/// Subject-plane, non-ghost, non-bound layers stay on the canvas (start rect; panel moves
/// applied at their end state).
fn assert_inside_canvas(p: &MotionProject) {
    let (cw, ch) = (p.canvas.width as f32, p.canvas.height as f32);
    for s in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        for l in layers(s) {
            if l.layout.is_some()
                || matches!(l.kind, LayerKind::Group { .. })
                || l.id.ends_with(".ghost")
                // Background/foreground plane layers may bleed (multiplane).
                || l.depth.is_some()
            {
                continue;
            }
            let mut left = l.x - l.anchor_x * l.width;
            let mut top = l.y - l.anchor_y * l.height;
            for m in motions_on(s, &l.id) {
                if let MotionOp::Move { to, .. } = &m.op {
                    // Bounds of the start and the end state must both fit.
                    for (dx, dy) in [(0.0, 0.0), (to[0], to[1])] {
                        let (lx, ty) = (left + dx, top + dy);
                        assert!(
                            lx >= -1.0 && lx + l.width <= cw + 1.0,
                            "{}: '{}' leaves the canvas horizontally",
                            s.id,
                            l.id
                        );
                        if !l.id.contains(".serif.") && !l.id.contains(".head.") {
                            assert!(
                                ty >= -1.0 && ty + l.height <= ch + 1.0,
                                "{}: '{}' leaves the canvas vertically",
                                s.id,
                                l.id
                            );
                        }
                    }
                }
            }
            left = left.max(-1.0);
            top = top.max(-1.0);
            assert!(
                left + l.width <= cw + 1.0 && top + l.height <= ch + 1.0,
                "{}: '{}' leaves the canvas",
                s.id,
                l.id
            );
        }
    }
}

fn phrase(value: &str) -> Value {
    json!({ "kind": "phrase", "value": value })
}

// ---------------------------------------------------------------------------
// KineticPoster
// ---------------------------------------------------------------------------

fn poster_emphasize() -> Value {
    json!({
        "purpose": "emphasize",
        "statement": "Attention is the scarcest thing",
        "primary": phrase("every alert costs focus"),
        "keyword": "scarcest",
        "energy": "building",
    })
}

#[test]
fn poster_keyword_manipulation_happens_at_evolve() {
    let p = build(
        &intent("vertical", vec![poster_emphasize()]),
        &style("kinetic", 1),
    );
    let s = beat(&p, 1);
    let life = s.lifecycle.expect("lifecycle");
    // Headline words are individual layers.
    assert!(!ids_with_prefix(s, "b1.head.").is_empty(), "head units");
    assert!(layer(s, "b1.serif.0.0").id == "b1.serif.0.0");
    layer(s, "b1.serif_rule");
    layer(s, "b1.ghost");

    let emphasis: Vec<&Motion> = s
        .motions
        .iter()
        .filter(|m| {
            m.target.starts_with("b1.head.")
                && (matches!(m.op, MotionOp::Scale { .. }) || m.target.ends_with(".punch"))
        })
        .collect();
    assert!(!emphasis.is_empty(), "the keyword is manipulated");
    for m in &emphasis {
        assert!(
            m.start >= life.evolve - EPS,
            "{} {} starts {} before EVOLVE {}",
            m.target,
            m.op.op_name(),
            m.start,
            life.evolve
        );
    }
    // The entrance itself happens in ENTER/SETTLE.
    assert!(s
        .motions
        .iter()
        .any(|m| m.target.starts_with("b1.head.") && m.start < life.read));
    assert_lifecycle_schedule(&p);
    assert_inside_canvas(&p);
}

#[test]
fn poster_reveal_phrase_arrives_as_tracking_reveal() {
    let reveal = json!({
        "purpose": "reveal",
        "statement": "Quiet is a feature",
        "primary": { "kind": "phrase", "value": "silence ships", "meaning": "result" },
    });
    for language in ["minimal", "kinetic", "parallax"] {
        let p = build(
            &intent("vertical", vec![reveal.clone()]),
            &style(language, 3),
        );
        let s = beat(&p, 1);
        let life = s.lifecycle.expect("lifecycle");
        let glyphs = ids_with_prefix(s, "b1.serif.0.");
        assert!(
            glyphs.len() >= 8,
            "{language}: glyph clusters, got {}",
            glyphs.len()
        );
        for id in &glyphs {
            for m in motions_on(s, id) {
                assert!(m.start >= life.enter - EPS, "{language}: {id} too early");
                assert!(
                    m.start < life.read,
                    "{language}: the primary lands before READ"
                );
            }
        }
        assert_lifecycle_schedule(&p);
        assert_inside_canvas(&p);
    }
}

#[test]
fn poster_replace_type_replaces_the_primary_phrase() {
    let mut beat_json = poster_emphasize();
    beat_json["secondary"] = phrase("protect it deliberately");
    beat_json["relationship"] = json!("replace");
    let p = build(&intent("vertical", vec![beat_json]), &style("kinetic", 1));
    let s = beat(&p, 1);
    let life = s.lifecycle.expect("lifecycle");
    let old = ids_with_prefix(s, "b1.serif.");
    let new = ids_with_prefix(s, "b1.serif_next.");
    assert!(!old.is_empty() && !new.is_empty(), "old and new runs");
    // Old units: entrance fade + leaving fade; new units arrive in EVOLVE.
    for id in &old {
        assert!(has_op(s, id, "fade") >= 2, "{id} enters and leaves");
    }
    for id in &new {
        for m in motions_on(s, id) {
            assert!(m.start >= life.evolve - EPS, "{id} arrives before EVOLVE");
        }
    }
    assert_lifecycle_schedule(&p);
    assert_inside_canvas(&p);
}

#[test]
fn poster_secondary_phrase_and_number_arrive_after_the_primary() {
    for secondary in [
        phrase("then we ship"),
        json!({ "kind": "number", "value": "40%", "meaning": "fewer alerts" }),
    ] {
        let mut b = poster_emphasize();
        b["secondary"] = secondary;
        let p = build(&intent("vertical", vec![b]), &style("kinetic", 2));
        let s = beat(&p, 1);
        let life = s.lifecycle.expect("lifecycle");
        let support: Vec<String> = ids_with_prefix(s, "b1.support");
        assert!(!support.is_empty(), "secondary layers");
        for id in &support {
            for m in motions_on(s, id) {
                assert!(m.start >= life.evolve - EPS, "{id} before EVOLVE");
            }
        }
        assert_lifecycle_schedule(&p);
        assert_inside_canvas(&p);
    }
}

#[test]
fn poster_variant_mirror_flips_the_serif_side() {
    let reveal = json!({
        "purpose": "reveal",
        "statement": "Quiet is a feature",
        "primary": phrase("silence ships"),
    });
    let mut xs: Vec<i64> = Vec::new();
    for seed in 0..40u64 {
        let p = build(
            &intent("vertical", vec![reveal.clone()]),
            &style("minimal", seed),
        );
        let s = beat(&p, 1);
        let x = layer(s, "b1.serif.0.0").x.round() as i64;
        if !xs.contains(&x) {
            xs.push(x);
        }
    }
    assert_eq!(
        xs.len(),
        2,
        "Standard and Mirror place the serif line differently: {xs:?}"
    );
}

#[test]
fn poster_carried_phrase_becomes_a_shared_element() {
    let mut first = poster_emphasize();
    first["continuity"] = json!("carry_primary");
    let second = json!({
        "purpose": "emphasize",
        "statement": "Focus is what you keep",
        "primary": phrase("every alert costs focus"),
        "keyword": "keep",
    });
    let p = build(
        &intent("vertical", vec![first, second]),
        &style("kinetic", 1),
    );
    assert_eq!(p.shared.len(), 1, "one shared element");
    assert!(
        ids_with_prefix(beat(&p, 1), "b1.serif.").is_empty(),
        "the carried phrase is not a local run"
    );
    let track = &p.shared[0].track;
    for scene in ["beat_1", "beat_2"] {
        assert!(track.iter().any(|k| k.scene == scene), "key in {scene}");
    }
    assert_lifecycle_schedule(&p);
}

#[test]
fn poster_respects_lifecycle_for_every_language_and_energy() {
    for language in [
        "auto",
        "minimal",
        "kinetic",
        "parallax",
        "sequential",
        "data",
    ] {
        for energy in ["calm", "building", "impact"] {
            let mut a = poster_emphasize();
            a["energy"] = json!(energy);
            a["secondary"] = phrase("protect it deliberately");
            a["relationship"] = json!("replace");
            let b = json!({
                "purpose": "reveal",
                "statement": "Quiet is a feature",
                "primary": phrase("silence ships"),
                "secondary": phrase("by design"),
                "energy": energy,
            });
            let p = build(&intent("vertical", vec![a, b]), &style(language, 5));
            assert_lifecycle_schedule(&p);
            assert_inside_canvas(&p);
        }
    }
}

// ---------------------------------------------------------------------------
// SplitContrast
// ---------------------------------------------------------------------------

fn split_beat(relationship: Option<&str>) -> Value {
    let mut b = json!({
        "purpose": "compare",
        "statement": "Two ways to spend an hour",
        "primary": { "kind": "phrase", "value": "scrolling", "meaning": "the habit" },
        "secondary": { "kind": "phrase", "value": "building", "meaning": "the craft" },
        "energy": "building",
    });
    if let Some(r) = relationship {
        b["relationship"] = json!(r);
    }
    b
}

#[derive(Debug, PartialEq, Eq)]
enum Layout {
    LeftRight,
    TopBottom,
    CenterOpposition,
}

fn layout_of(s: &Scene) -> Layout {
    let (a, b) = (layer(s, "b1.panel_a"), layer(s, "b1.panel_b"));
    if (a.y - b.y).abs() < 0.5 {
        Layout::LeftRight
    } else if (a.x - b.x).abs() < 0.5 {
        Layout::TopBottom
    } else {
        Layout::CenterOpposition
    }
}

#[test]
fn split_variants_follow_format_and_seed() {
    let landscape = build(
        &intent("landscape", vec![split_beat(Some("separate"))]),
        &style("minimal", 1),
    );
    assert_eq!(layout_of(beat(&landscape, 1)), Layout::LeftRight);

    let mut seen = Vec::new();
    for seed in 0..64u64 {
        let p = build(
            &intent("vertical", vec![split_beat(Some("separate"))]),
            &style("minimal", seed),
        );
        let l = layout_of(beat(&p, 1));
        assert_ne!(
            l,
            Layout::LeftRight,
            "vertical never uses left/right halves"
        );
        if !seen.contains(&l) {
            seen.push(l);
        }
    }
    assert_eq!(seen.len(), 2, "both vertical layouts occur: {seen:?}");
}

/// A style seed per (format, layout) so every layout is exercised.
fn seed_for(format: &str, want: &Layout) -> u64 {
    for seed in 0..256u64 {
        let p = build(
            &intent(format, vec![split_beat(Some("separate"))]),
            &style("minimal", seed),
        );
        if &layout_of(beat(&p, 1)) == want {
            return seed;
        }
    }
    panic!("no seed produces {want:?} for {format}");
}

#[test]
fn split_second_state_arrives_in_evolve_for_every_layout_and_relationship() {
    let cases = [
        ("landscape", Layout::LeftRight),
        ("vertical", Layout::TopBottom),
        ("vertical", Layout::CenterOpposition),
        ("square", Layout::CenterOpposition),
    ];
    for (format, layout) in cases {
        let seed = seed_for(format, &layout);
        for rel in [None, Some("separate"), Some("replace"), Some("carry")] {
            let what = format!("{format}/{layout:?}/{rel:?}");
            let p = build(
                &intent(format, vec![split_beat(rel)]),
                &style("minimal", seed),
            );
            let s = beat(&p, 1);
            let life = s.lifecycle.expect("lifecycle");
            assert_eq!(layout_of(s), layout, "{what}");
            layer(s, "b1.divider");
            layer(s, "b1.connective.0");

            // First side enters before EVOLVE ...
            assert!(
                motions_on(s, "b1.panel_a")
                    .all(|m| m.start < life.evolve || m.op.op_name() != "mask_reveal"),
                "{what}: panel A masks in during ENTER"
            );
            // ... the second side, the divider and the connective in EVOLVE.
            let second: Vec<&Motion> = s
                .motions
                .iter()
                .filter(|m| {
                    [
                        "b1.panel_b",
                        "b1.divider",
                        "b1.side_b",
                        "b1.replace_new",
                        "b1.label_b",
                        "b1.connective",
                    ]
                    .iter()
                    .any(|prefix| m.target.starts_with(prefix))
                })
                .collect();
            assert!(!second.is_empty(), "{what}");
            for m in second {
                assert!(
                    m.start >= life.evolve - EPS,
                    "{what}: {} starts {} before EVOLVE {}",
                    m.target,
                    m.start,
                    life.evolve
                );
            }
            assert_lifecycle_schedule(&p);
            assert_inside_canvas(&p);
        }
    }
}

#[test]
fn split_relationships_leave_their_structural_signature() {
    let seed = seed_for("vertical", &Layout::TopBottom);
    let compile_rel = |rel: Option<&str>, b: Value| {
        let mut b = b;
        match rel {
            Some(r) => b["relationship"] = json!(r),
            None => {
                b.as_object_mut().map(|o| o.remove("relationship"));
            }
        }
        build(&intent("vertical", vec![b]), &style("minimal", seed))
    };

    // Connective word per relationship.
    for (rel, word) in [
        (None, "versus"),
        (Some("replace"), "becomes"),
        (Some("carry"), "still"),
        (Some("separate"), "apart"),
    ] {
        let p = compile_rel(rel, split_beat(None));
        assert_eq!(text_of(layer(beat(&p, 1), "b1.connective.0")), word);
    }

    // separate: both panels drift apart (a move each).
    let p = compile_rel(Some("separate"), split_beat(None));
    let s = beat(&p, 1);
    assert_eq!(has_op(s, "b1.panel_a", "move"), 1);
    assert_eq!(has_op(s, "b1.panel_b", "move"), 1);

    // carry: only the primary panel crosses the divider.
    let p = compile_rel(Some("carry"), split_beat(None));
    let s = beat(&p, 1);
    assert_eq!(has_op(s, "b1.panel_a", "move"), 1);
    assert_eq!(has_op(s, "b1.panel_b", "move"), 0);

    // replace, two phrases: the primary text type-replaces into the secondary's.
    let p = compile_rel(Some("replace"), split_beat(None));
    let s = beat(&p, 1);
    assert!(!ids_with_prefix(s, "b1.replace_old.").is_empty());
    assert!(!ids_with_prefix(s, "b1.replace_new.").is_empty());
    assert!(
        ids_with_prefix(s, "b1.side_a").is_empty(),
        "no static side A"
    );

    // replace, number primary: fade-swap instead.
    let mut b = split_beat(None);
    b["primary"] = json!({ "kind": "number", "value": "40%", "meaning": "before" });
    let p = compile_rel(Some("replace"), b);
    let s = beat(&p, 1);
    let side_a = ids_with_prefix(s, "b1.side_a");
    assert_eq!(side_a.len(), 1);
    assert!(has_op(s, &side_a[0], "fade") >= 2, "enters, then fades out");
    assert!(ids_with_prefix(s, "b1.replace_old.").is_empty());
    let side_b = ids_with_prefix(s, "b1.side_b");
    assert_eq!(side_b.len(), 1);
    assert_lifecycle_schedule(&p);
}

#[test]
fn split_numbers_count_in_the_data_language() {
    let b = json!({
        "purpose": "compare",
        "statement": "Same hour, different pull",
        "primary": { "kind": "number", "value": "40%", "meaning": "before" },
        "secondary": phrase("after the change"),
        "relationship": "separate",
    });
    let p = build(&intent("vertical", vec![b]), &style("data", 1));
    let s = beat(&p, 1);
    let side_a = ids_with_prefix(s, "b1.side_a");
    assert_eq!(side_a.len(), 1);
    assert_eq!(has_op(s, &side_a[0], "count"), 1);
}

#[test]
fn split_carried_phrase_becomes_a_shared_element() {
    let first = split_beat(Some("carry"));
    let second = json!({
        "purpose": "emphasize",
        "statement": "Habits shape the hour",
        "primary": { "kind": "phrase", "value": "scrolling", "meaning": "the habit" },
    });
    let p = build(
        &intent("vertical", vec![first, second]),
        &style("kinetic", 4),
    );
    assert_eq!(p.shared.len(), 1, "one shared element");
    let s1 = beat(&p, 1);
    assert!(
        ids_with_prefix(s1, "b1.side_a").is_empty(),
        "carried, not local"
    );
    layer(s1, "b1.panel_a");
    let track = &p.shared[0].track;
    for scene in ["beat_1", "beat_2"] {
        assert!(track.iter().any(|k| k.scene == scene), "key in {scene}");
    }
    // The first beat's key is bound to its panel so the subject follows it.
    assert!(track.iter().any(
        |k| k.scene == "beat_1" && k.layout.as_ref().is_some_and(|l| l.parent == "b1.panel_a")
    ));
    assert_lifecycle_schedule(&p);
}

#[test]
fn split_respects_lifecycle_for_every_language_and_energy() {
    for language in [
        "auto",
        "minimal",
        "kinetic",
        "parallax",
        "sequential",
        "data",
    ] {
        for energy in ["calm", "building", "impact"] {
            for rel in ["separate", "replace", "carry"] {
                let mut a = split_beat(Some(rel));
                a["energy"] = json!(energy);
                let mut b = split_beat(Some(rel));
                b["purpose"] = json!("contrast");
                b["energy"] = json!(energy);
                let p = build(&intent("vertical", vec![a, b]), &style(language, 7));
                assert_lifecycle_schedule(&p);
                assert_inside_canvas(&p);
            }
        }
    }
}
