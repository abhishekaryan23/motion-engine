//! (0.23 W8d) The diagram look (`grammar/diagram.rs`, entities on a process
//! track) draws the relationship between two subjects.
//!
//! Every relationship a compare / contrast beat of two phrases plays gets its
//! connector: an arrow for grow / replace / compress, a plus for accumulate, a
//! VS badge on the divide for separate, a link for carry. The connector ids
//! are the ones `story_warnings::has_connector` recognises, so the
//! `relationship_dropped` warning is the judge here. The diagram builder is
//! reached through a reference whose visual language is diagrammatic (the
//! `--reference-style` path); the beat's `entity_primary` / `entity_secondary`
//! layers say it was used.
//!
//! The layout check compares the beat with and without its connector layers:
//! a connector must add no layout finding (`text_overlap` included).

use std::collections::BTreeSet;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::taste::ReferencePrinciples;
use motion_core::compiler::visual::{Explanation, Medium, VisualLanguage};
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning,
    WARN_RELATIONSHIP_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_report;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::validate;
use serde_json::{json, Value};

/// (relationship, purpose, the connector word its layer name carries).
const RELATIONS: [(&str, &str, &str); 6] = [
    ("grow", "compare", "arrow"),
    ("replace", "contrast", "arrow"),
    ("compress", "contrast", "arrow"),
    ("accumulate", "compare", "plus"),
    ("separate", "compare", "vs"),
    ("carry", "contrast", "link"),
];

const FORMATS: [&str; 3] = ["vertical", "square", "landscape"];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn diagram_reference() -> ReferencePrinciples {
    ReferencePrinciples {
        visual: VisualLanguage {
            medium: Medium::Diagrammatic,
            explanation: Some(Explanation::Diagrammatic),
            ..VisualLanguage::default()
        },
        ..ReferencePrinciples::default()
    }
}

fn phrase(v: &str) -> Value {
    json!({ "kind": "phrase", "value": v })
}

fn beat(purpose: &str, rel: Option<&str>, a: &str, b: &str) -> Value {
    let mut v = json!({
        "purpose": purpose,
        "statement": format!("{a} and {b}"),
        "narration": format!("{a} meets {b} on the long road ahead."),
        "primary": phrase(a),
        "secondary": phrase(b),
        "energy": "building",
    });
    if let Some(r) = rel {
        v["relationship"] = json!(r);
    }
    v
}

fn solo(statement: &str) -> Value {
    json!({
        "purpose": "emphasize",
        "statement": statement,
        "narration": format!("{statement} is the point of this beat."),
        "primary": phrase(statement),
        "energy": "calm",
    })
}

fn compile(
    beats: Vec<Value>,
    format: &str,
    style: Value,
    opts: &CompileOptions,
) -> (MotionProject, Vec<CompileWarning>) {
    let intent: CreativeIntent = serde_json::from_value(json!({
        "version": "0.2", "title": "diagram_connectors", "format": format, "beats": beats
    }))
    .expect("intent");
    let style: StyleProfile = serde_json::from_value(style).expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    compile_with_report(
        &intent,
        &style,
        Some(&diagram_reference()),
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        opts,
    )
    .expect("compile")
}

/// Phrase pairs of different shapes and lengths (the bodies and labels differ).
const PAIRS: [(&str, &str); 6] = [
    ("Small team", "Big team"),
    ("Old process", "Automation"),
    ("Fixed budget", "Rising costs"),
    ("Year one", "Year ten"),
    ("Sales", "Support"),
    ("A", "B"),
];

/// A three-beat story with the relationship beat in the middle (`beat_2`).
fn story_of(rel: &str, purpose: &str, pair: (&str, &str)) -> Vec<Value> {
    vec![
        solo("Where it starts"),
        beat(purpose, Some(rel), pair.0, pair.1),
        solo("Where it ends"),
    ]
}

fn story(rel: &str, purpose: &str) -> Vec<Value> {
    story_of(rel, purpose, PAIRS[0])
}

/// The words of a layer's name (the rule `story_warnings::has_connector`
/// reads): the first segment of the id after the `b<N>.` scene prefix, split
/// on `_`.
fn words(id: &str) -> Vec<String> {
    let prefixed = id.split('.').next().is_some_and(|p| {
        p.starts_with('b') && p.len() > 1 && p[1..].chars().all(|c| c.is_ascii_digit())
    });
    let rest = if prefixed {
        id.split_once('.').map_or("", |(_, r)| r)
    } else {
        id
    };
    rest.split('.')
        .next()
        .unwrap_or("")
        .split('_')
        .map(str::to_string)
        .collect()
}

fn named<'a>(layers: &'a [Layer], word: &str, out: &mut Vec<&'a Layer>) {
    for l in layers {
        if words(&l.id).iter().any(|w| w == word) {
            out.push(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            named(children, word, out);
        }
    }
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes.iter().find(|s| s.id == id).expect("scene")
}

fn uses_diagram(s: &Scene) -> bool {
    let mut v = Vec::new();
    named(&s.layers, "entity", &mut v);
    v.iter().any(|l| l.id.ends_with("entity_primary"))
        && v.iter().any(|l| l.id.ends_with("entity_secondary"))
}

/// Remove every connector layer (and the motions that target one) from `scene`.
fn strip_connectors(scene: &mut Scene) {
    const CONNECTOR_WORDS: [&str; 6] = ["arrow", "plus", "vs", "link", "connector", "divide"];
    fn walk(layers: &mut Vec<Layer>, removed: &mut BTreeSet<String>) {
        layers.retain(|l| {
            let gone = words(&l.id)
                .iter()
                .any(|w| CONNECTOR_WORDS.contains(&w.as_str()));
            if gone {
                removed.insert(l.id.clone());
            }
            !gone
        });
        for l in layers {
            if let LayerKind::Group { children } = &mut l.kind {
                walk(children, removed);
            }
        }
    }
    let mut removed = BTreeSet::new();
    walk(&mut scene.layers, &mut removed);
    scene.motions.retain(|m| !removed.contains(&m.target));
}

fn count(layers: &[Layer]) -> usize {
    layers
        .iter()
        .map(|l| match &l.kind {
            LayerKind::Group { children } => 1 + count(children),
            _ => 1,
        })
        .sum()
}

fn findings(p: &MotionProject) -> BTreeSet<(String, String, String)> {
    let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
    layout_report(p, &frame)
        .findings
        .into_iter()
        .map(|f| (f.scene, f.layer, f.check.name().to_string()))
        .collect()
}

fn dropped(warnings: &[CompileWarning]) -> Vec<&CompileWarning> {
    warnings
        .iter()
        .filter(|w| w.code == WARN_RELATIONSHIP_DROPPED)
        .collect()
}

#[test]
fn every_relationship_draws_its_connector_in_every_format() {
    for format in FORMATS {
        for (rel, purpose, word) in RELATIONS {
            let (project, warnings) = compile(
                story(rel, purpose),
                format,
                json!({}),
                &CompileOptions::default(),
            );
            let what = format!("{rel} {format}");
            let s = scene(&project, "beat_2");
            assert!(uses_diagram(s), "{what}: not drawn by the diagram builder");
            let mut found = Vec::new();
            named(&s.layers, word, &mut found);
            assert!(!found.is_empty(), "{what}: no {word} layer");
            assert!(
                dropped(&warnings).is_empty(),
                "{what}: relationship_dropped: {warnings:?}"
            );
            // The connector draws before ANTICIPATE.
            let life = s.lifecycle.expect("lifecycle");
            for l in &found {
                for m in s.motions.iter().filter(|m| m.target == l.id) {
                    if !matches!(m.op, MotionOp::Move { .. }) {
                        assert!(m.start < life.anticipate, "{what}: {} starts late", l.id);
                    }
                }
            }
            validate::validate(&project, Some(&repo().join("assets")))
                .unwrap_or_else(|e| panic!("{what}: invalid: {e:?}"));
        }
    }
}

/// The beat with its connector has no layout finding the beat without it does
/// not already have (`text_overlap` included), over phrases of different
/// shapes, in a light and a dark style.
#[test]
fn a_connector_adds_no_layout_finding() {
    for format in FORMATS {
        for style in [json!({}), json!({ "tone": "technical" })] {
            for (rel, purpose, _) in RELATIONS {
                for pair in PAIRS {
                    let (project, _) = compile(
                        story_of(rel, purpose, pair),
                        format,
                        style.clone(),
                        &CompileOptions::default(),
                    );
                    let what = format!("{rel} {format} {pair:?} {style}");
                    let mut bare = project.clone();
                    for s in bare.scenes.iter_mut().filter(|s| s.id == "beat_2") {
                        let before = count(&s.layers);
                        strip_connectors(s);
                        assert!(count(&s.layers) < before, "{what}: nothing to strip");
                    }
                    let (with, without) = (findings(&project), findings(&bare));
                    let added: Vec<_> = with.difference(&without).collect();
                    assert!(
                        added.is_empty(),
                        "{what}: the connector adds layout findings {added:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn without_a_relationship_the_line_is_unchanged() {
    for format in FORMATS {
        let (project, warnings) = compile(
            vec![
                solo("Where it starts"),
                beat("contrast", None, "Cause", "Effect"),
                solo("Where it ends"),
            ],
            format,
            json!({}),
            &CompileOptions::default(),
        );
        let s = scene(&project, "beat_2");
        assert!(uses_diagram(s));
        let mut line = Vec::new();
        named(&s.layers, "connector", &mut line);
        assert_eq!(line.len(), 1, "{format}");
        for word in ["arrow", "plus", "vs", "link"] {
            let mut none = Vec::new();
            named(&s.layers, word, &mut none);
            assert!(none.is_empty(), "{format}: {word}");
        }
        assert!(dropped(&warnings).is_empty());
    }
}

/// The lite story of `docs/MCP.md` (compound interest): beat 3 compares two
/// phrases with `grow`. Under the diagram look it draws its arrow in every tone,
/// with and without the product path's variety seed and art direction.
#[test]
fn compound_interest_beat_three_draws_the_grow_arrow() {
    let beats = || {
        vec![
            json!({
                "purpose": "emphasize", "statement": "Leave it alone", "energy": "building",
                "narration": "Put one thousand dollars away today, and then leave it alone for thirty years.",
                "primary": phrase("Leave it alone"),
            }),
            json!({
                "purpose": "emphasize", "statement": "At seven percent a year", "energy": "building",
                "narration": "At seven percent a year, the interest starts earning interest of its own.",
                "primary": { "kind": "number", "value": "7%", "meaning": "every year" },
            }),
            json!({
                "purpose": "compare", "statement": "Time does the heavy lifting", "energy": "building",
                "narration": "Time does the heavy lifting: the last ten years add more than the first twenty.",
                "primary": phrase("first 20 years"),
                "secondary": phrase("last 10 years"),
                "relationship": "grow",
            }),
            json!({
                "purpose": "reveal", "statement": "Patience pays", "energy": "building",
                "narration": "So that one thousand dollars quietly grows into about seven thousand six hundred.",
                "primary": { "kind": "number", "value": "$7,600", "meaning": "after 30 years" },
            }),
        ]
    };
    let mut diagrams = 0;
    for tone in ["auto", "editorial", "technical", "playful"] {
        for product in [false, true] {
            let opts = if product {
                CompileOptions {
                    variety: Some(0x00C0_FFEE),
                    art: Some(ArtMode::Auto),
                    ..CompileOptions::default()
                }
            } else {
                CompileOptions::default()
            };
            let (project, warnings) = compile(beats(), "vertical", json!({ "tone": tone }), &opts);
            let what = format!("{tone} product={product}");
            assert!(dropped(&warnings).is_empty(), "{what}: {warnings:?}");
            let s = scene(&project, "beat_3");
            if uses_diagram(s) {
                diagrams += 1;
                let mut arrow = Vec::new();
                named(&s.layers, "arrow", &mut arrow);
                assert!(!arrow.is_empty(), "{what}: no arrow");
                // The grown second card keeps its label inside the safe area.
                let own: Vec<_> = findings(&project)
                    .into_iter()
                    .filter(|f| f.0 == "beat_3")
                    .collect();
                assert!(own.is_empty(), "{what}: layout findings {own:?}");
            }
        }
    }
    assert!(diagrams > 0, "the diagram builder was never reached");
}

/// Under a variety seed (every take) a relationship beat the diagram draws keeps
/// its connector.
#[test]
fn takes_keep_the_connector() {
    let mut diagrams = 0;
    for take in 0..4u64 {
        for (rel, purpose, word) in RELATIONS {
            let opts = CompileOptions {
                variety: Some(0xD1A6_0001),
                take,
                ..CompileOptions::default()
            };
            let (project, warnings) = compile(story(rel, purpose), "vertical", json!({}), &opts);
            let what = format!("{rel} take {take}");
            assert!(dropped(&warnings).is_empty(), "{what}: {warnings:?}");
            let s = scene(&project, "beat_2");
            if uses_diagram(s) {
                diagrams += 1;
                let mut found = Vec::new();
                named(&s.layers, word, &mut found);
                assert!(!found.is_empty(), "{what}: no {word}");
            }
        }
    }
    assert!(diagrams > 0);
}
