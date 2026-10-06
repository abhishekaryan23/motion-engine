//! (0.19) RelationStage and environment plates in the cinematic look.
//!
//! A `compare` / `contrast` beat of two pictures shows both on the focus plane
//! with the beat's relationship drawn between them, instead of a hero and a
//! dim prop; the matched environment plate is placed far behind the picture.
//! Structural assertions on the compiled scene; no pixels. A throwaway family
//! (two objects and one environment plate) is written under `target/`.

use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::LayoutFinding;
use motion_core::layout_report;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject, Scene};
use motion_core::style::StyleProfile;
use serde_json::{json, Value};

/// A 1x1 transparent PNG (the engine never decodes it at compile time).
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xFF, 0xFF, 0x3F,
    0x00, 0x05, 0xFE, 0x02, 0xFE, 0xA7, 0x35, 0x81, 0x84, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E,
    0x44, 0xAE, 0x42, 0x60, 0x82,
];

const FAMILY: &str = "rel_fixture";

/// `library/rel_fixture/`: objects `apple` and `pear` (square cutouts) and the
/// environment plate `env_orchard` (a tall opaque picture, role `texture`).
#[allow(clippy::type_complexity)]
fn fixture_root(test: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/relation_stage_tests")
        .join(test);
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("library").join(FAMILY);
    std::fs::create_dir_all(&dir).expect("family dir");
    let assets: [(&str, &str, &[&str], u32, u32, bool); 3] = [
        ("apple", "object", &["apple", "fruit"], 512, 512, true),
        ("pear", "object", &["pear", "fruit"], 512, 512, true),
        (
            "env_orchard",
            "texture",
            &["apple", "pear", "orchard"],
            576,
            1024,
            false,
        ),
    ];
    let mut catalog = Vec::new();
    let mut manifest = Vec::new();
    for (id, role, tags, w, h, alpha) in assets {
        let file = format!("{id}.png");
        std::fs::write(dir.join(&file), PNG).expect("png");
        catalog.push(json!({
            "id": id, "file": file, "role": role, "subject": id, "tags": tags,
            "alpha": alpha, "cutout_bbox": null, "open_edge": false,
            "source": { "vendor": "test" }, "qa": "PASS"
        }));
        manifest.push(json!({
            "id": format!("library.{id}"), "path": file,
            "width": w, "height": h, "alpha": alpha,
            "serves": [format!("library.{id}")]
        }));
    }
    std::fs::write(
        dir.join("catalog.json"),
        json!({ "version": "0.1", "family": FAMILY, "assets": catalog }).to_string(),
    )
    .expect("catalog.json");
    std::fs::write(
        dir.join("manifest.json"),
        json!({ "version": "0.2", "assets": manifest }).to_string(),
    )
    .expect("manifest.json");
    root
}

fn compile_beats(test: &str, beats: Vec<Value>, canvas: Option<(u32, u32)>) -> MotionProject {
    let root = fixture_root(test);
    let intent: CreativeIntent = serde_json::from_value(json!({
        "version": "0.2", "title": "relation_stage", "beats": beats
    }))
    .expect("intent");
    let style: StyleProfile =
        serde_json::from_value(json!({ "tone": "cinematic", "polarity": "dark" })).expect("style");
    let library = AssetLibrary::new(&root).with_families(vec![FAMILY.to_string()]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        canvas,
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn object(asset: &str, meaning: &str, value: Option<&str>) -> Value {
    let mut v = json!({ "kind": "object", "asset": asset, "meaning": meaning });
    if let Some(value) = value {
        v["value"] = json!(value);
    }
    v
}

/// An apple / pear beat with the given purpose and relationship.
fn pair(purpose: &str, relationship: Option<&str>) -> Value {
    let mut beat = json!({
        "purpose": purpose,
        "statement": "Apple against pear",
        "primary": object("apple", "sweet", Some("1 kg")),
        "secondary": object("pear", "juicy", None),
        "keyword": "fruit"
    });
    if let Some(r) = relationship {
        beat["relationship"] = json!(r);
    }
    beat
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes.iter().find(|s| s.id == id).expect("scene")
}

fn find<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            if let Some(c) = find(children, id) {
                return Some(c);
            }
        }
    }
    None
}

fn has(s: &Scene, id: &str) -> bool {
    find(&s.layers, id).is_some()
}

/// The `z` of the top-level depth wrapper that holds layer `id`.
fn depth_of(s: &Scene, id: &str) -> Option<f32> {
    s.layers
        .iter()
        .find(|l| l.id == format!("{id}.depth") || l.id == id)
        .and_then(|l| l.z)
}

fn first_start(s: &Scene, target: &str) -> f64 {
    s.motions
        .iter()
        .filter(|m| m.target == target)
        .map(|m| m.start)
        .fold(f64::MAX, f64::min)
}

fn focal(p: &MotionProject, beat: &str) -> Option<String> {
    serde_json::to_value(p).ok()?["project"]["art"]["focal"][beat]
        .as_str()
        .map(str::to_string)
}

#[test]
fn separate_is_a_versus_stage_with_both_pictures_on_the_focus_plane() {
    let p = compile_beats("versus", vec![pair("compare", Some("separate"))], None);
    let s = scene(&p, "beat_1");
    assert!(has(s, "b1.pair_badge"), "the VS disc");
    assert!(has(s, "b1.pair_divider"), "the divider line");
    assert!(!has(s, "b1.pair_arrow"));
    // Both pictures are on the focus plane (the old prop sat far behind at 500).
    assert_eq!(depth_of(s, "b1.hero"), Some(0.0));
    assert_eq!(depth_of(s, "b1.prop"), Some(0.0));
    // Each has its label; the first has its stamped figure.
    assert!(has(s, "b1.pair_label.0") && has(s, "b1.pair_label.1"));
    assert!(has(s, "b1.pair_stamp.0") && !has(s, "b1.pair_stamp.1"));
    // The relation is what the beat is about.
    assert_eq!(focal(&p, "beat_1").as_deref(), Some("b1.pair_node"));
    assert!(has(s, "b1.pair_node"));
}

#[test]
fn each_relationship_draws_its_own_connector() {
    // (relationship, layers that must exist, layers that must not)
    let table: [(&str, &[&str], &[&str]); 6] = [
        ("separate", &["pair_badge", "pair_divider"], &["pair_arrow"]),
        (
            "replace",
            &["pair_arrow", "pair_arrowhead"],
            &["pair_badge"],
        ),
        ("grow", &["pair_arrow", "pair_arrowhead"], &["pair_badge"]),
        (
            "compress",
            &["pair_arrow", "pair_arrowhead"],
            &["pair_badge"],
        ),
        ("carry", &["pair_link"], &["pair_arrow", "pair_badge"]),
        (
            "accumulate",
            &["pair_badge"],
            &["pair_divider", "pair_arrow"],
        ),
    ];
    for (rel, must, must_not) in table {
        let p = compile_with_options_for(rel);
        let s = scene(&p, "beat_1");
        for id in must {
            assert!(has(s, &format!("b1.{id}")), "{rel}: missing {id}");
        }
        for id in must_not {
            assert!(!has(s, &format!("b1.{id}")), "{rel}: unexpected {id}");
        }
    }
    // The defaults: compare separates, contrast compresses.
    let compare = compile_beats("default_compare", vec![pair("compare", None)], None);
    assert!(has(scene(&compare, "beat_1"), "b1.pair_divider"));
    let contrast = compile_beats("default_contrast", vec![pair("contrast", None)], None);
    assert!(has(scene(&contrast, "beat_1"), "b1.pair_arrow"));
}

fn compile_with_options_for(rel: &str) -> MotionProject {
    compile_beats(
        &format!("rel_{rel}"),
        vec![pair("compare", Some(rel))],
        None,
    )
}

#[test]
fn the_second_picture_arrives_after_the_first_from_the_other_side() {
    let p = compile_beats("arrival", vec![pair("compare", Some("grow"))], None);
    let s = scene(&p, "beat_1");
    assert!(
        first_start(s, "b1.prop") > first_start(s, "b1.hero"),
        "the second picture enters later"
    );
    // The arrival slide (0.19 `Arrival`) is re-targeted by the FX director
    // from the stage child to its depth wrapper, so the whole plane swings in.
    let from = |target: &str| {
        s.motions
            .iter()
            .find_map(|m| match (&m.op, m.target == target, m.id.as_deref()) {
                (MotionOp::Move { from, .. }, true, Some("arrival")) => Some(*from),
                _ => None,
            })
            .expect("an entrance move")
    };
    let (a, b) = (from("b1.hero.depth"), from("b1.prop.depth"));
    assert!(
        a[0] * b[0] < 0.0 || a[1] * b[1] < 0.0,
        "opposite sides: {a:?} vs {b:?}"
    );
    // The connector draws after the second picture has started to arrive.
    assert!(first_start(s, "b1.pair_arrow") > first_start(s, "b1.prop"));
}

#[test]
fn the_grown_picture_grows_and_the_replaced_one_steps_back() {
    let scale_to = |s: &Scene, target: &str| {
        s.motions
            .iter()
            .filter(|m| m.target == target)
            .filter_map(|m| match m.op {
                MotionOp::Scale { to, .. } => Some(to),
                _ => None,
            })
            .next_back()
    };
    let grow = compile_beats("play_grow", vec![pair("compare", Some("grow"))], None);
    let s = scene(&grow, "beat_1");
    assert!(scale_to(s, "b1.prop").is_some_and(|v| v > 1.1));
    assert_eq!(
        scale_to(s, "b1.hero"),
        Some(1.0),
        "the first stays as it arrived"
    );
    let replace = compile_beats("play_replace", vec![pair("compare", Some("replace"))], None);
    let s = scene(&replace, "beat_1");
    assert!(scale_to(s, "b1.hero").is_some_and(|v| v < 0.9));
}

#[test]
fn other_beats_keep_the_hero_and_the_prop() {
    let mut emphasize = pair("emphasize", None);
    emphasize.as_object_mut().unwrap().remove("relationship");
    let phrases = json!({
        "purpose": "compare", "statement": "Two words",
        "primary": { "kind": "phrase", "value": "fast" },
        "secondary": { "kind": "phrase", "value": "slow" }
    });
    for (name, beat) in [("emphasize", emphasize), ("phrases", phrases)] {
        let p = compile_beats(name, vec![beat], None);
        let s = scene(&p, "beat_1");
        assert!(
            !s.layers.iter().any(|l| l.id.contains("pair_")),
            "{name}: no relation stage"
        );
    }
    // An emphasized pair is still a hero with the secondary behind it.
    let mut e = pair("emphasize", None);
    e.as_object_mut().unwrap().remove("relationship");
    let p = compile_beats("emphasize_prop", vec![e], None);
    let s = scene(&p, "beat_1");
    assert_eq!(depth_of(s, "b1.hero"), Some(0.0));
    // (0.19) The supporting picture sits just in front of the focus plane
    // (`cinematic3d::Z_SUPPORT`), in focus, not as a blurred midground prop.
    assert_eq!(depth_of(s, "b1.prop"), Some(-60.0));
}

#[test]
fn an_environment_plate_needs_two_matching_words() {
    // "apple" + "pear" (both subjects) match the plate's tags twice.
    let p = compile_beats("env_two", vec![pair("compare", Some("separate"))], None);
    let s = scene(&p, "beat_1");
    assert!(has(s, "b1.env"), "the matched plate");
    assert_eq!(depth_of(s, "b1.env"), Some(600.0));
    assert!(
        p.assets.iter().any(|a| a.path.ends_with("env_orchard.png")),
        "the plate is a project asset"
    );
    // A beat that shares a single word with the plate gets none.
    let one = json!({
        "purpose": "emphasize", "statement": "Just one",
        "primary": object("apple", "round", None)
    });
    let p = compile_beats("env_one", vec![one], None);
    assert!(
        !has(scene(&p, "beat_1"), "b1.env"),
        "one word is not enough"
    );
}

fn findings(p: &MotionProject, w: u32, h: u32) -> Vec<LayoutFinding> {
    let frame = LayoutFrame::new(w, h).expect("frame");
    layout_report(p, &frame).findings
}

#[test]
fn a_relation_stage_passes_layout_qa_on_every_canvas() {
    for (w, h) in [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)] {
        for rel in ["separate", "replace", "grow", "compress"] {
            let p = compile_beats(
                &format!("qa_{rel}_{w}x{h}"),
                vec![pair("compare", Some(rel))],
                Some((w, h)),
            );
            let f = findings(&p, w, h);
            assert!(f.is_empty(), "{rel} {w}x{h}: {f:?}");
        }
    }
}

#[test]
fn fixture_root_is_under_target() {
    let r = fixture_root("path_check");
    assert!(Path::new(&r).join("library").join(FAMILY).is_dir());
}
