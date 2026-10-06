//! READ / EVOLVE scheduling of the atomic recipes: secondary information
//! arrives inside `[life.evolve, life.anticipate)`, nothing new starts once
//! ANTICIPATE begins, and every language still validates.

use std::path::PathBuf;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

const LANGUAGES: &[&str] = &["minimal", "kinetic", "parallax", "sequential", "data"];

fn style(language: &str) -> StyleProfile {
    let path = repo().join("examples/public/minimal.style.json");
    let text = std::fs::read_to_string(&path).expect("style file");
    let mut v: Value = serde_json::from_str(&text).expect("style json");
    v["motion_language"] = json!(language);
    serde_json::from_value(v).expect("style parses")
}

/// Emphasize, contrast and explain beats with a secondary subject and a
/// keyword, plus a reveal with a keyword in its statement.
fn intent() -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.1",
        "title": "read_phase",
        "format": "vertical",
        "beats": [
            {
                "purpose": "emphasize",
                "statement": "Your salary stayed flat",
                "primary": { "kind": "number", "value": "₹50,000", "meaning": "salary" },
                "secondary": { "kind": "phrase", "value": "for five straight years." },
                "energy": "calm",
                "keyword": "flat"
            },
            {
                "purpose": "contrast",
                "statement": "Prices kept climbing",
                "primary": { "kind": "number", "value": "₹50,000", "meaning": "salary" },
                "secondary": { "kind": "object", "asset": "shopping_basket", "value": "+38%", "meaning": "groceries" },
                "relationship": "compress",
                "energy": "building",
                "keyword": "prices"
            },
            {
                "purpose": "explain",
                "statement": "Every basket costs more than last year.",
                "primary": { "kind": "phrase", "value": "Why it feels tighter" },
                "secondary": { "kind": "object", "asset": "shopping_basket" },
                "energy": "building",
                "keyword": "costs"
            },
            {
                "purpose": "reveal",
                "statement": "You are earning less than you think.",
                "primary": { "kind": "number", "value": "−27%", "meaning": "real buying power" },
                "energy": "impact",
                "keyword": "less"
            }
        ]
    }))
    .expect("intent parses")
}

fn build(language: &str) -> MotionProject {
    let lib = AssetLibrary::new(repo().join("assets"));
    compile(&intent(), &style(language), &lib, &ApproxMeasure).expect("compiles")
}

fn beat_scenes(p: &MotionProject) -> impl Iterator<Item = &Scene> {
    p.scenes.iter().filter(|s| s.id != "backdrop")
}

#[test]
fn every_language_validates() {
    for lang in LANGUAGES {
        let p = build(lang);
        validate(&p, Some(&repo().join("assets"))).unwrap_or_else(|e| panic!("{lang}: {e:?}"));
    }
}

#[test]
fn secondary_information_arrives_in_evolve() {
    for lang in LANGUAGES {
        let p = build(lang);
        for scene in beat_scenes(&p) {
            let life = scene.lifecycle.expect("compiled scenes carry a lifecycle");
            let inside = scene
                .motions
                .iter()
                .filter(|m| m.start >= life.evolve - 1e-6 && m.start < life.anticipate)
                .count();
            assert!(
                inside > 0,
                "{lang}: scene '{}' starts nothing in [evolve {}, anticipate {})",
                scene.id,
                life.evolve,
                life.anticipate
            );
        }
    }
}

#[test]
fn nothing_new_starts_once_anticipate_begins() {
    for lang in LANGUAGES {
        let p = build(lang);
        for scene in beat_scenes(&p) {
            let life = scene.lifecycle.expect("lifecycle");
            let stage = format!("{}.stage", scene.id.replace("beat_", "b"));
            for m in &scene.motions {
                // The stage wrapper owns anticipation + exit; the parallax
                // ghost is the background that fades back during ANTICIPATE.
                if m.target == stage || m.target.ends_with(".ghost") {
                    continue;
                }
                assert!(
                    m.start < life.anticipate,
                    "{lang}: '{}' in '{}' starts at {} (anticipate {})",
                    m.target,
                    scene.id,
                    m.start,
                    life.anticipate
                );
            }
        }
    }
}

#[test]
fn kinetic_keyword_emphasis_waits_for_evolve() {
    let p = build("kinetic");
    let scene = beat_scenes(&p).next().expect("beat 1");
    let life = scene.lifecycle.expect("lifecycle");
    let emphasis: Vec<_> = scene
        .motions
        .iter()
        .filter(|m| m.target.starts_with("b1.head.") && matches!(m.op, MotionOp::Scale { .. }))
        .collect();
    assert!(!emphasis.is_empty(), "keyword emphasis expected");
    for m in emphasis {
        assert!(
            m.start >= life.evolve - 1e-6 && m.start < life.anticipate,
            "emphasis at {} outside [{}, {})",
            m.start,
            life.evolve,
            life.anticipate
        );
    }
}

#[test]
fn ghost_always_drifts() {
    for lang in LANGUAGES {
        let p = build(lang);
        for scene in beat_scenes(&p) {
            let has_ghost = scene.motions.iter().any(|m| m.target.ends_with(".ghost"));
            let drifts = scene.motions.iter().any(|m| {
                m.target.ends_with(".ghost")
                    && matches!(m.op, motion_core::scene::MotionOp::Move { .. })
            });
            assert!(
                !has_ghost || drifts,
                "{lang}: ghost of '{}' is static",
                scene.id
            );
        }
    }
}
