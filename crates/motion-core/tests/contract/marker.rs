//! (0.24) Beat markers replace the eyebrow kicker: none by default, a running
//! index on a counted story, a source/unit tag only when the beat has one; and
//! a number the beat says is unchanged does not count up.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject};
use motion_core::slop_qa::SlopCheck;
use motion_core::{slop_report, SlopRules, StyleProfile};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn compile_json(intent_json: &str, style_json: &str) -> MotionProject {
    let intent = CreativeIntent::from_json(intent_json).expect("intent");
    let style: StyleProfile = serde_json::from_str(style_json).expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    compile_with_options(
        &intent,
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .expect("compile")
}

fn demo(format: &str) -> MotionProject {
    let intent = std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json"))
        .expect("intent")
        .replace("\"vertical\"", &format!("\"{format}\""));
    let style =
        std::fs::read_to_string(repo().join("examples/editorial_demo.style.json")).expect("style");
    compile_json(&intent, &style)
}

fn texts(project: &MotionProject) -> Vec<(String, String)> {
    fn walk(layers: &[Layer], out: &mut Vec<(String, String)>) {
        for l in layers {
            match &l.kind {
                LayerKind::Text(t) => out.push((l.id.clone(), t.text.clone())),
                LayerKind::Group { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &project.scenes {
        walk(&s.layers, &mut out);
    }
    out
}

fn ids(project: &MotionProject) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>) {
        for l in layers {
            out.push(l.id.clone());
            if let LayerKind::Group { children } = &l.kind {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    for s in &project.scenes {
        walk(&s.layers, &mut out);
    }
    out
}

fn markers(project: &MotionProject) -> Vec<(String, String)> {
    texts(project)
        .into_iter()
        .filter(|(id, _)| id.ends_with(".marker"))
        .collect()
}

fn counts(project: &MotionProject, scene: &str) -> usize {
    project
        .scenes
        .iter()
        .filter(|s| s.id == scene)
        .flat_map(|s| &s.motions)
        .filter(|m| matches!(m.op, MotionOp::Count { .. }))
        .count()
}

#[test]
fn the_editorial_demo_has_no_kicker_and_no_eyebrow() {
    for format in ["vertical", "landscape"] {
        let p = demo(format);
        assert!(
            ids(&p).iter().all(|id| !id.contains("kicker")),
            "{format}: {:?}",
            ids(&p)
        );
        assert!(markers(&p).is_empty(), "{format}: {:?}", markers(&p));
        let r = slop_report(&p, &SlopRules::bundled());
        assert!(
            r.findings
                .iter()
                .all(|f| f.check != SlopCheck::EyebrowLabel),
            "{format}: {:?}",
            r.findings
        );
    }
}

fn countdown5() -> String {
    let beat = |title: &str, say: &str, asset: &str| {
        format!(
            r#"{{"purpose":"emphasize","statement":"{title}","narration":"{say}","primary":{{"kind":"object","asset":"{asset}","meaning":"{title}"}}}}"#
        )
    };
    let beats = [
        beat(
            "Older than trees",
            "Here are five animals older than trees.",
            "shark",
        )
        .replace(r#","meaning":"Older than trees""#, ""),
        beat(
            "Sharks",
            "Number five: sharks, swimming for 450 million years.",
            "shark",
        ),
        beat(
            "Jellyfish",
            "Number four, the jellyfish, drifting for 500 million years.",
            "jellyfish",
        ),
        beat(
            "Sponges",
            "Number three: sponges, nearly 600 million years old.",
            "sponge",
        ),
        beat(
            "Comb jellies",
            "Number two: comb jellies, over 700 million years.",
            "jellyfish",
        ),
        beat(
            "Placozoans",
            "And number one: the placozoans, the oldest of them all.",
            "sponge",
        ),
    ];
    format!(
        r#"{{"version":"0.2","title":"five","format":"vertical","beats":[{}]}}"#,
        beats.join(",")
    )
}

#[test]
fn a_five_item_countdown_shows_the_running_index() {
    let p = compile_json(&countdown5(), "{}");
    let m = markers(&p);
    let nums: Vec<&str> = m.iter().map(|(_, t)| t.get(..2).unwrap_or("")).collect();
    assert_eq!(nums, ["05", "04", "03", "02", "01"], "{m:?}");
    assert!(ids(&p).iter().all(|id| !id.contains("kicker")));
}

#[test]
fn a_story_that_does_not_count_has_no_index() {
    let p = demo("vertical");
    assert!(markers(&p).is_empty());
}

#[test]
fn a_unit_the_headline_lacks_becomes_a_tag_and_nothing_else_does() {
    let make = |meaning: &str| {
        format!(
            r#"{{"version":"0.2","title":"t","format":"vertical","beats":[{{"purpose":"emphasize","statement":"Rents rose again","primary":{{"kind":"number","value":"₹18,400","meaning":"{meaning}"}}}}]}}"#
        )
    };
    let tagged = compile_json(&make("per month"), "{}");
    let m = markers(&tagged);
    assert_eq!(m.len(), 1, "{m:?}");
    assert_eq!(m[0].1, "per month");
    // A meaning that is neither a unit nor a source, or that the headline
    // already says, is never printed.
    assert!(markers(&compile_json(&make("rent"), "{}")).is_empty());
    assert!(markers(&compile_json(&make("rents"), "{}")).is_empty());
}

#[test]
fn a_number_the_beat_says_is_unchanged_lands_static() {
    let make = |statement: &str| {
        format!(
            r#"{{"version":"0.2","title":"t","format":"vertical","beats":[{{"purpose":"emphasize","statement":"{statement}","primary":{{"kind":"number","value":"₹50,000","meaning":"salary"}},"energy":"calm"}}]}}"#
        )
    };
    let style = r#"{"tone":"technical"}"#;
    let flat = compile_json(&make("Your salary stayed flat"), style);
    let grew = compile_json(&make("Your salary doubled"), style);
    assert_eq!(counts(&flat, "beat_1"), 0, "flat must not count");
    assert!(counts(&grew, "beat_1") > 0, "a growing number still counts");
}
