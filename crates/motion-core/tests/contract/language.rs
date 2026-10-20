//! Motion-language and layout-binding compile tests: every `motion_language`
//! value compiles every checked-in intent into a valid scene, and each language
//! leaves its structural signature. Assertions target structure (ids, ops,
//! bindings, validity), never exact pixel or design values.

use std::path::PathBuf;

use motion_core::compiler::{compile, ApproxMeasure, AssetLibrary};
use motion_core::easing::Easing;
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, Motion, MotionOp, MotionProject, Scene};
use motion_core::style::{MotionLanguage, StyleProfile};
use motion_core::validate::validate;
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn library() -> AssetLibrary {
    AssetLibrary::new(repo().join("assets"))
}

/// Every checked-in intent (public examples, demos, benchmarks) that must keep
/// compiling under any motion language.
const INTENTS: &[&str] = &[
    "examples/public/minimal-emphasize.intent.json",
    "examples/public/minimal-contrast.intent.json",
    "examples/public/three-beat-story.intent.json",
    "examples/editorial_demo.intent.json",
    "examples/motion_language_demo.intent.json",
    "golden/fixtures/benchmark/notification-fragmentation-01/attempt-01.intent.json",
    "golden/fixtures/benchmark/salary-grocery-pressure-01/attempt-01.intent.json",
];

const LANGUAGES: &[&str] = &[
    "auto",
    "minimal",
    "kinetic",
    "parallax",
    "sequential",
    "data",
];

fn intent(rel: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn intent_from(v: Value) -> CreativeIntent {
    serde_json::from_value(v).expect("intent parses")
}

/// The public minimal style with `motion_language` set (or omitted for `None`).
fn style_with(language: Option<&str>) -> StyleProfile {
    let mut v: Value = serde_json::from_str(&read("examples/public/minimal.style.json")).unwrap();
    if let Some(l) = language {
        v["motion_language"] = json!(l);
    }
    serde_json::from_value(v).expect("style parses")
}

fn build(i: &CreativeIntent, s: &StyleProfile) -> MotionProject {
    compile(i, s, &library(), &ApproxMeasure).expect("compiles")
}

fn assert_valid(p: &MotionProject, what: &str) {
    if let Err(e) = validate(p, Some(&repo().join("assets"))) {
        panic!("{what}: compiled scene fails validation: {e:?}");
    }
}

fn for_each_layer<'a>(layers: &'a [Layer], f: &mut dyn FnMut(&'a Layer)) {
    for l in layers {
        f(l);
        if let LayerKind::Group { children } = &l.kind {
            for_each_layer(children, f);
        }
    }
}

fn all_layers(s: &Scene) -> Vec<&Layer> {
    let mut out = Vec::new();
    for_each_layer(&s.layers, &mut |l| out.push(l));
    out
}

fn all_motions(p: &MotionProject) -> impl Iterator<Item = (&Scene, &Motion)> {
    p.scenes
        .iter()
        .flat_map(|s| s.motions.iter().map(move |m| (s, m)))
}

/// Scenes that carry beat content (everything but the shared backdrop).
fn beat_scenes(p: &MotionProject) -> impl Iterator<Item = &Scene> {
    p.scenes.iter().filter(|s| s.id != "backdrop")
}

fn is_spring(e: Easing) -> bool {
    matches!(e, Easing::EditorialSpring | Easing::ImpactSpring)
}

// ---------------------------------------------------------------------------
// Sweep: every language x every checked-in intent
// ---------------------------------------------------------------------------

#[test]
fn every_language_compiles_and_validates_every_intent() {
    for lang in LANGUAGES {
        let style = style_with(Some(lang));
        for rel in INTENTS {
            let p = build(&intent(rel), &style);
            assert_valid(&p, &format!("{lang} x {rel}"));
            assert!(!p.scenes.is_empty());
        }
    }
    // A style with no motion_language at all behaves as auto (same output).
    for rel in INTENTS {
        let i = intent(rel);
        assert_eq!(
            build(&i, &style_with(None)),
            build(&i, &style_with(Some("auto"))),
            "{rel}: omitted motion_language must equal auto"
        );
    }
}

#[test]
fn no_motion_outlives_its_scene_in_any_language() {
    // Validation enforces this; the sweep makes sure no combination slips by.
    const EPS: f64 = 1e-6;
    for lang in LANGUAGES {
        let style = style_with(Some(lang));
        for rel in INTENTS {
            let p = build(&intent(rel), &style);
            for (s, m) in all_motions(&p) {
                assert!(
                    m.start >= -EPS && m.start + m.duration <= s.duration_seconds + EPS,
                    "{lang} x {rel}: motion on '{}' in scene '{}' spans {}..{} (scene {}s)",
                    m.target,
                    s.id,
                    m.start,
                    m.start + m.duration,
                    s.duration_seconds
                );
            }
            for s in &p.scenes {
                for cm in s.camera.iter().flat_map(|c| &c.motions) {
                    assert!(
                        cm.start + cm.duration <= s.duration_seconds + EPS,
                        "{lang} x {rel}: camera motion outlives scene '{}'",
                        s.id
                    );
                }
            }
            for sh in &p.shared {
                for k in &sh.track {
                    let sc = p
                        .scenes
                        .iter()
                        .find(|s| s.id == k.scene)
                        .unwrap_or_else(|| panic!("track key scene '{}' missing", k.scene));
                    assert!(
                        k.at <= sc.duration_seconds + EPS,
                        "{lang} x {rel}: shared key at {} outlives scene '{}'",
                        k.at,
                        sc.id
                    );
                }
            }
        }
    }
}

#[test]
fn compiles_are_deterministic_in_every_language() {
    for lang in LANGUAGES {
        let style = style_with(Some(lang));
        for rel in INTENTS {
            let i = intent(rel);
            let a = serde_json::to_string(&build(&i, &style)).unwrap();
            let b = serde_json::to_string(&build(&i, &style)).unwrap();
            assert_eq!(a, b, "{lang} x {rel}: two compiles differ");
        }
    }
}

// ---------------------------------------------------------------------------
// Language signatures
// ---------------------------------------------------------------------------

/// The contrast recipe's object "stamp" once hardcoded `ImpactSpring`; it now
/// follows the language. Kept as its own regression test.
fn is_known_stamp_spring(m: &Motion) -> bool {
    m.target.ends_with(".stamp")
}

#[test]
fn minimal_uses_no_spring_easings_on_the_contrast_stamp() {
    let style = style_with(Some("minimal"));
    let p = build(&intent("examples/editorial_demo.intent.json"), &style);
    for (s, m) in all_motions(&p).filter(|(_, m)| is_known_stamp_spring(m)) {
        assert!(
            !is_spring(m.easing),
            "scene '{}' stamp motion uses {:?} under minimal",
            s.id,
            m.easing
        );
    }
}

#[test]
fn minimal_uses_no_spring_easings() {
    let style = style_with(Some("minimal"));
    for rel in INTENTS {
        let p = build(&intent(rel), &style);
        for (s, m) in all_motions(&p) {
            assert!(
                !is_spring(m.easing),
                "{rel}: scene '{}' motion on '{}' uses {:?} under minimal",
                s.id,
                m.target,
                m.easing
            );
        }
        for s in &p.scenes {
            for cm in s.camera.iter().flat_map(|c| &c.motions) {
                assert!(
                    !is_spring(cm.easing),
                    "{rel}: camera in '{}' springs under minimal",
                    s.id
                );
            }
        }
    }
}

#[test]
fn minimal_shared_tracks_use_no_spring_easings() {
    let style = style_with(Some("minimal"));
    for rel in INTENTS {
        let p = build(&intent(rel), &style);
        for sh in &p.shared {
            for k in &sh.track {
                assert!(
                    !is_spring(k.easing),
                    "{rel}: shared '{}' key in '{}' at {} uses {:?} under minimal",
                    sh.id,
                    k.scene,
                    k.at,
                    k.easing
                );
            }
        }
    }
}

#[test]
fn parallax_gives_every_beat_scene_a_camera_and_a_far_plane() {
    let style = style_with(Some("parallax"));
    for rel in INTENTS {
        let p = build(&intent(rel), &style);
        let mut n = 0;
        for s in beat_scenes(&p) {
            n += 1;
            let cam = s
                .camera
                .as_ref()
                .unwrap_or_else(|| panic!("{rel}: scene '{}' has no camera", s.id));
            assert!(!cam.motions.is_empty(), "{rel}: '{}' camera is idle", s.id);
            let far = all_layers(s)
                .iter()
                .any(|l| l.depth.is_some_and(|d| d < 1.0));
            assert!(far, "{rel}: scene '{}' has no layer with depth < 1", s.id);
        }
        assert!(n > 0, "{rel}: no beat scenes");
    }
}

fn keyword_emphasize_intent() -> CreativeIntent {
    intent_from(json!({
        "version": "0.1", "title": "kin", "format": "vertical",
        "beats": [{
            "purpose": "emphasize",
            "statement": "Focus gets broken, not lost.",
            "primary": { "kind": "phrase", "value": "Rebuild it", "meaning": "recovery" },
            "energy": "building",
            "keyword": "broken"
        }]
    }))
}

fn word_layers(s: &Scene) -> Vec<&Layer> {
    // Word layers are `<prefix>.head.<line>.<word>`; line layers are
    // `<prefix>.head.<line>`.
    all_layers(s)
        .into_iter()
        .filter(|l| {
            l.id.split_once(".head.")
                .is_some_and(|(_, rest)| rest.split('.').count() == 2)
        })
        .collect()
}

#[test]
fn kinetic_emphasize_with_keyword_builds_per_word_layers() {
    let p = build(&keyword_emphasize_intent(), &style_with(Some("kinetic")));
    assert_valid(&p, "kinetic keyword");
    let scene = beat_scenes(&p).next().expect("beat scene");
    let words = word_layers(scene);
    // "Focus gets broken, not lost." -> 5 words, each its own layer.
    assert!(
        words.len() >= 5,
        "expected per-word layers, got {:?}",
        all_layers(scene).iter().map(|l| &l.id).collect::<Vec<_>>()
    );
    let broken = words
        .iter()
        .find(|l| match &l.kind {
            LayerKind::Text(t) => t.text.to_lowercase().starts_with("broken"),
            _ => false,
        })
        .expect("keyword word layer");
    assert!(
        scene.motions.iter().any(|m| m.target == broken.id),
        "keyword layer must be animated"
    );
    // Every word arrives at its own time (a cascade, not one block).
    let mut starts: Vec<f64> = words
        .iter()
        .filter_map(|w| {
            scene
                .motions
                .iter()
                .filter(|m| m.target == w.id)
                .map(|m| m.start)
                .reduce(f64::min)
        })
        .collect();
    starts.sort_by(f64::total_cmp);
    starts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert!(starts.len() > 1, "words should be staggered: {starts:?}");
}

#[test]
fn minimal_keeps_a_headline_as_whole_lines() {
    // Contrast for the kinetic test above: same beat, restrained language.
    let p = build(&keyword_emphasize_intent(), &style_with(Some("minimal")));
    let scene = beat_scenes(&p).next().expect("beat scene");
    assert!(
        word_layers(scene).is_empty(),
        "minimal must not split headlines into per-word layers"
    );
}

#[test]
fn data_with_numeric_subject_creates_a_count_motion() {
    let i = intent_from(json!({
        "version": "0.1", "title": "dat", "format": "vertical",
        "beats": [{
            "purpose": "reveal",
            "statement": "More deep work.",
            "primary": { "kind": "number", "value": "40%", "meaning": "more deep work" },
            "energy": "impact"
        }]
    }));
    for lang in ["data", "auto"] {
        let p = build(&i, &style_with(Some(lang)));
        assert_valid(&p, lang);
        let count = all_motions(&p).find_map(|(_, m)| match &m.op {
            MotionOp::Count { to, .. } => Some(*to),
            _ => None,
        });
        assert_eq!(count, Some(40.0), "{lang}: expected a count motion to 40");
    }
}

#[test]
fn data_language_counts_the_salary_benchmark_numbers() {
    let p = build(
        &intent("golden/fixtures/benchmark/salary-grocery-pressure-01/attempt-01.intent.json"),
        &style_with(Some("data")),
    );
    assert!(
        all_motions(&p).any(|(_, m)| matches!(m.op, MotionOp::Count { .. })),
        "numeric beats should count up under data"
    );
}

// ---------------------------------------------------------------------------
// Layout bindings from the compiler
// ---------------------------------------------------------------------------

fn contrast_compress(continuity: Option<&str>) -> CreativeIntent {
    let mut beat = json!({
        "purpose": "contrast",
        "statement": "None of these interruptions takes very long.",
        "primary": { "kind": "phrase", "value": "One task", "meaning": "deep focus" },
        "secondary": { "kind": "phrase", "value": "Constant alerts", "meaning": "interruptions" },
        "relationship": "compress",
        "energy": "building"
    });
    if let Some(c) = continuity {
        beat["continuity"] = json!(c);
    }
    intent_from(json!({
        "version": "0.1", "title": "cc", "format": "vertical", "beats": [beat]
    }))
}

fn accent_expand_window(s: &Scene, target: &str) -> (f64, f64) {
    s.motions
        .iter()
        .find(|m| m.target == target && matches!(m.op, MotionOp::AccentExpand { .. }))
        .map(|m| (m.start, m.start + m.duration))
        .unwrap_or_else(|| panic!("no accent_expand on '{target}'"))
}

#[test]
fn compress_binds_a_local_primary_to_the_panel_without_manual_offsets() {
    for lang in LANGUAGES {
        let p = build(&contrast_compress(None), &style_with(Some(lang)));
        assert_valid(&p, lang);
        assert!(p.shared.is_empty(), "{lang}: nothing is carried");
        let scene = beat_scenes(&p).next().unwrap();
        let bound: Vec<&Layer> = all_layers(scene)
            .into_iter()
            .filter(|l| l.layout.is_some())
            .collect();
        assert_eq!(bound.len(), 1, "{lang}: exactly the primary is bound");
        let primary = bound[0];
        let parent_id = &primary.layout.as_ref().unwrap().parent;
        let panel = all_layers(scene)
            .into_iter()
            .find(|l| &l.id == parent_id)
            .unwrap_or_else(|| panic!("{lang}: binding parent '{parent_id}' missing"));
        assert!(
            scene
                .motions
                .iter()
                .any(|m| m.target == panel.id && matches!(m.op, MotionOp::AccentExpand { .. })),
            "{lang}: the parent must be the compressing panel"
        );

        // No hand-made y compensation: the bound primary carries no move that
        // ends off-rest or runs while the panel is resizing.
        let (a, b) = accent_expand_window(scene, &panel.id);
        for m in scene.motions.iter().filter(|m| m.target == primary.id) {
            if let MotionOp::Move { to, .. } = &m.op {
                assert_eq!(*to, [0.0, 0.0], "{lang}: move ends off rest: {m:?}");
                assert!(
                    m.start + m.duration <= a + 1e-6 || m.start >= b - 1e-6,
                    "{lang}: move overlaps the panel compression: {m:?}"
                );
            }
        }
    }
}

#[test]
fn compress_binds_a_carried_primary_via_the_shared_track() {
    let p = build(
        &intent("golden/fixtures/benchmark/notification-fragmentation-01/attempt-01.intent.json"),
        &style_with(Some("auto")),
    );
    assert_valid(&p, "notification");
    let one_task = p
        .shared
        .iter()
        .find(|s| s.id == "one_task")
        .expect("carried primary");
    let bound: Vec<_> = one_task
        .track
        .iter()
        .filter(|k| k.layout.is_some())
        .collect();
    assert_eq!(bound.len(), 1, "one binding key, carried by later keys");
    let key = bound[0];
    let binding = key.layout.as_ref().unwrap();
    let scene = p.scenes.iter().find(|s| s.id == key.scene).unwrap();
    let parent = all_layers(scene)
        .into_iter()
        .find(|l| l.id == binding.parent)
        .unwrap_or_else(|| panic!("parent '{}' not in '{}'", binding.parent, scene.id));
    assert!(
        scene
            .motions
            .iter()
            .any(|m| m.target == parent.id && matches!(m.op, MotionOp::AccentExpand { .. })),
        "parent must be the compressing panel"
    );

    // No key in the binding scene sets x/y (no hand-made offset).
    for k in one_task.track.iter().filter(|k| k.scene == key.scene) {
        assert!(
            k.state.x.is_none() && k.state.y.is_none(),
            "key at {} sets a manual position in the bound scene",
            k.at
        );
    }
    // The primary has no local copy in the scene that binds it.
    assert!(
        all_layers(scene).iter().all(|l| !l.id.contains("one_task")),
        "carried primary must not also exist locally in '{}'",
        scene.id
    );
}

#[test]
fn notification_benchmark_compiles_byte_identically() {
    let i =
        intent("golden/fixtures/benchmark/notification-fragmentation-01/attempt-01.intent.json");
    let style = StyleProfile::from_json(&read(
        "golden/fixtures/benchmark/notification-fragmentation-01/style.json",
    ))
    .expect("benchmark style");
    let a = serde_json::to_string(&build(&i, &style)).unwrap();
    let b = serde_json::to_string(&build(&i, &style)).unwrap();
    assert_eq!(a, b);
    assert_valid(&build(&i, &style), "notification benchmark");
}

#[test]
fn benchmark_styles_compile_their_intents() {
    for case in [
        "notification-fragmentation-01",
        "salary-grocery-pressure-01",
    ] {
        let i = intent(&format!(
            "golden/fixtures/benchmark/{case}/attempt-01.intent.json"
        ));
        let style = StyleProfile::from_json(&read(&format!(
            "golden/fixtures/benchmark/{case}/style.json"
        )))
        .expect("benchmark style");
        assert_valid(&build(&i, &style), case);
    }
}

// ---------------------------------------------------------------------------
// Backwards compatibility and rejection
// ---------------------------------------------------------------------------

#[test]
fn style_without_motion_language_defaults_to_auto() {
    let s = style_with(None);
    assert_eq!(s.motion_language, MotionLanguage::Auto);
    let empty: StyleProfile = serde_json::from_str("{}").expect("empty style is valid");
    assert_eq!(empty.motion_language, MotionLanguage::Auto);
}

#[test]
fn every_language_name_round_trips() {
    for lang in LANGUAGES {
        let s = style_with(Some(lang));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["motion_language"], json!(lang));
        let back: StyleProfile = serde_json::from_value(v).unwrap();
        assert_eq!(back.motion_language, s.motion_language);
    }
}

#[test]
fn unknown_motion_language_values_are_rejected() {
    for bad in ["wobbly", "Kinetic", "KINETIC", "", "cinematic", "auto "] {
        let mut v: Value =
            serde_json::from_str(&read("examples/public/minimal.style.json")).unwrap();
        v["motion_language"] = json!(bad);
        assert!(
            serde_json::from_value::<StyleProfile>(v).is_err(),
            "'{bad}' must be rejected"
        );
    }
    let mut v: Value = serde_json::from_str(&read("examples/public/minimal.style.json")).unwrap();
    v["motion_language"] = json!(3);
    assert!(serde_json::from_value::<StyleProfile>(v).is_err());
}

#[test]
fn intents_still_parse_without_any_language_field() {
    // Intents never mention motion language (it is a style choice); every
    // checked-in intent parses as-is.
    for rel in INTENTS {
        let _ = intent(rel);
    }
    // ...and the field is not smuggled into the intent.
    let mut v: Value =
        serde_json::from_str(&read("examples/public/minimal-emphasize.intent.json")).unwrap();
    v["motion_language"] = json!("kinetic");
    assert!(serde_json::from_value::<CreativeIntent>(v).is_err());
}
