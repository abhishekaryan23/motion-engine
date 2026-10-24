//! Image plates and the HeroObject / CinematicMultiplane / EvidenceStack /
//! TypeImageInterlock builders, through the public compile API. Assertions
//! target structure (ids, z bands, depth planes, assets, lifecycle), never
//! exact pixel values.

use std::collections::BTreeSet;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::{compile, compile_with_assets, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, Material, MotionProject, Scene};
use motion_core::style::StyleProfile;
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

fn manifest() -> AssetManifest {
    serde_json::from_str(&read("assets/test_manifest.json")).expect("manifest parses")
}

fn style(seed: u64, language: &str) -> StyleProfile {
    let mut v: Value = serde_json::from_str(&read("examples/public/minimal.style.json")).unwrap();
    v["seed"] = json!(seed);
    v["motion_language"] = json!(language);
    serde_json::from_value(v).expect("style parses")
}

fn golden(name: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(&format!("golden/{name}.intent.json")))
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn intent(v: Value) -> CreativeIntent {
    serde_json::from_value(v).expect("intent parses")
}

fn build(i: &CreativeIntent, s: &StyleProfile, m: Option<&AssetManifest>) -> MotionProject {
    match m {
        Some(m) => compile_with_assets(i, s, &library(), &ApproxMeasure, m),
        None => compile(i, s, &library(), &ApproxMeasure),
    }
    .expect("compiles")
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

fn beat_scene(p: &MotionProject, n: usize) -> &Scene {
    let id = format!("beat_{n}");
    p.scenes
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene {id}"))
}

/// Nothing new starts at/after ANTICIPATE except anticipation motions.
fn assert_nothing_new_after_anticipate(p: &MotionProject, what: &str) {
    for (n, s) in p.scenes.iter().filter(|s| s.id != "backdrop").enumerate() {
        let life = s.lifecycle.expect("compiled scenes carry a lifecycle");
        let stage = format!("b{}.stage", n + 1);
        for m in &s.motions {
            // The stage wrapper and the parallax ghost fade are anticipation motions.
            if m.target == stage
                || m.target == format!("{stage}_front")
                || m.target.ends_with(".ghost")
            {
                continue;
            }
            assert!(
                m.start < life.anticipate - 1e-6,
                "{what}: {} '{}' starts at {} >= anticipate {}",
                s.id,
                m.target,
                m.start,
                life.anticipate
            );
        }
    }
}

fn image_layer<'a>(s: &'a Scene, asset: &str) -> Option<&'a Layer> {
    all_layers(s)
        .into_iter()
        .find(|l| matches!(&l.kind, LayerKind::Image { asset: a, .. } if a == asset))
}

/// Top edge of a layer (its `y` is the anchor point).
fn top_of(l: &Layer) -> f32 {
    l.y - l.anchor_y * l.height
}

const LANGUAGES: &[&str] = &[
    "auto",
    "minimal",
    "kinetic",
    "parallax",
    "sequential",
    "data",
];

fn two_beat_intent() -> CreativeIntent {
    intent(json!({
        "version": "0.2",
        "title": "image_story",
        "format": "vertical",
        "beats": [
            {
                "purpose": "emphasize",
                "statement": "Nobody warned the new hires about the queue",
                "primary": { "kind": "phrase", "value": "new hires", "meaning": "a worker" },
                "secondary": { "kind": "phrase", "value": "on their first morning." },
                "energy": "building"
            },
            {
                "purpose": "reveal",
                "statement": "The receipt shows what changed",
                "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "grocery receipt" },
                "secondary": { "kind": "number", "value": "6900", "meaning": "this week" },
                "energy": "building",
                "keyword": "changed"
            }
        ]
    }))
}

// ---------------------------------------------------------------------------
// Compile + validate
// ---------------------------------------------------------------------------

#[test]
fn every_grammar_compiles_and_validates_in_every_language_and_variant() {
    let m = manifest();
    for name in ["multiplane", "hero_object", "evidence_stack", "type_image"] {
        let i = golden(name);
        for lang in LANGUAGES {
            // Different seeds exercise both variants (Standard / Mirror).
            for seed in 1..=6 {
                let s = style(seed, lang);
                for with in [None, Some(&m)] {
                    let p = build(&i, &s, with);
                    let what = format!("{name} {lang} seed {seed} manifest {}", with.is_some());
                    assert_valid(&p, &what);
                    assert_nothing_new_after_anticipate(&p, &what);
                }
            }
        }
    }
}

#[test]
fn energies_and_formats_keep_the_lifecycle_contract() {
    let m = manifest();
    for format in ["vertical", "square", "landscape"] {
        for energy in ["calm", "building", "impact"] {
            let doc = json!({
                "version": "0.2", "title": "t", "format": format,
                "beats": [
                    {
                        "purpose": "emphasize",
                        "statement": "Nobody warned the new hires",
                        "primary": { "kind": "phrase", "value": "new hires", "meaning": "worker" },
                        "secondary": { "kind": "phrase", "value": "on day one." },
                        "energy": energy
                    },
                    {
                        "purpose": "emphasize",
                        "statement": "One basket carries the week",
                        "primary": { "kind": "object", "asset": "shopping_basket", "value": "6900", "meaning": "basket" },
                        "secondary": { "kind": "phrase", "value": "week after week." },
                        "energy": energy
                    },
                    {
                        "purpose": "explain",
                        "statement": "Receipts prove it",
                        "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "receipt" },
                        "secondary": { "kind": "phrase", "value": "every line item counts." },
                        "energy": energy
                    }
                ]
            });
            let i = intent(doc);
            for lang in ["auto", "parallax", "minimal"] {
                for seed in [1, 2, 3, 4] {
                    let s = style(seed, lang);
                    for with in [None, Some(&m)] {
                        let p = build(&i, &s, with);
                        let what = format!("{format} {energy} {lang} seed {seed}");
                        assert_valid(&p, &what);
                        assert_nothing_new_after_anticipate(&p, &what);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TypeImageInterlock + EvidenceStack with a manifest
// ---------------------------------------------------------------------------

#[test]
fn manifest_selects_type_image_interlock_and_uses_the_evidence_image() {
    let m = manifest();
    let p = build(&two_beat_intent(), &style(1, "auto"), Some(&m));
    assert_valid(&p, "manifest story");

    // Beat 1 (0.5 subject-aware): the headline and the cutout interlock, and
    // no headline layer in front of the image covers the head.
    let s1 = beat_scene(&p, 1);
    let layers = all_layers(s1);
    let img = image_layer(s1, "asset.beat_1.hero_subject").expect("subject image layer");
    let head: Vec<&&Layer> = layers
        .iter()
        .filter(|l| l.id.starts_with("b1.head."))
        .collect();
    assert!(!head.is_empty(), "headline present");
    // Head box from the manifest's face anchor (0.1 manifest: no analysis).
    let (ix, iy) = (img.x - img.anchor_x * img.width, top_of(img));
    let (hx0, hy0) = (
        ix + (0.5 - 0.14) * img.width,
        iy + (0.21 - 0.12) * img.height,
    );
    let (hx1, hy1) = (hx0 + 0.28 * img.width, hy0 + 0.22 * img.height);
    for l in &head {
        let (lx0, ly0) = (l.x - l.anchor_x * l.width, top_of(l));
        let crosses = lx0 < hx1 && lx0 + l.width > hx0 && ly0 < hy1 && ly0 + l.height > hy0;
        if crosses {
            assert!(
                l.z_index < img.z_index,
                "{} crosses the head and must sit behind the image",
                l.id
            );
        }
    }
    let text_top = head.iter().map(|l| top_of(l)).fold(f32::MAX, f32::min);
    let text_bottom = head
        .iter()
        .map(|l| top_of(l) + l.height)
        .fold(f32::MIN, f32::max);
    assert!(
        top_of(img) < text_bottom && top_of(img) + img.height > text_top,
        "the image crosses the headline band"
    );
    let asset = p
        .assets
        .iter()
        .find(|a| a.id == "asset.beat_1.hero_subject")
        .expect("asset");
    assert_eq!(asset.path, "test_images/figure.png");

    // Beat 2: the delivered evidence image is placed instead of the object.
    let s2 = beat_scene(&p, 2);
    assert!(image_layer(s2, "asset.beat_2.evidence_image").is_some());
    let asset = p
        .assets
        .iter()
        .find(|a| a.id == "asset.beat_2.evidence_image")
        .expect("asset");
    assert_eq!(asset.path, "test_images/document.png");
    assert!(
        all_layers(s2).iter().any(|l| l.id == "b2.highlight"),
        "highlight bar"
    );
    assert!(
        all_layers(s2).iter().any(|l| l.id == "b2.source"),
        "source label"
    );
    assert_nothing_new_after_anticipate(&p, "manifest story");
}

#[test]
fn type_image_mirror_flips_the_image_side() {
    let m = manifest();
    let i = two_beat_intent();
    let mut sides = BTreeSet::new();
    for seed in 1..=24 {
        let p = build(&i, &style(seed, "auto"), Some(&m));
        let s1 = beat_scene(&p, 1);
        let img = image_layer(s1, "asset.beat_1.hero_subject").expect("image");
        let cx = img.x - img.anchor_x * img.width + img.width / 2.0;
        sides.insert(cx < p.canvas.width as f32 / 2.0);
    }
    assert_eq!(sides.len(), 2, "both image sides occur across seeds");
}

// ---------------------------------------------------------------------------
// Without a manifest
// ---------------------------------------------------------------------------

#[test]
fn without_manifest_plates_are_procedural_and_no_image_asset_is_added() {
    let p = build(&two_beat_intent(), &style(1, "auto"), None);
    assert_valid(&p, "no manifest");
    assert!(
        !p.assets.iter().any(|a| a.id.starts_with("asset.beat_")),
        "no delivered image assets: {:?}",
        p.assets.iter().map(|a| &a.id).collect::<Vec<_>>()
    );
    // Beat 1: the collage/plate halftone texture patch is there.
    let s1 = beat_scene(&p, 1);
    assert!(all_layers(s1)
        .iter()
        .any(|l| matches!(&l.kind, LayerKind::Texture(t) if t.material == Material::Halftone)));
    // Beat 2: the object itself is the evidence (registered library asset).
    let s2 = beat_scene(&p, 2);
    assert!(image_layer(s2, "asset.beat_2.evidence_image").is_none());
    assert!(all_layers(s2).iter().any(|l| matches!(&l.kind,
        LayerKind::Svg { asset, .. } | LayerKind::Image { asset, .. } if asset == "asset.shopping_basket")));
    assert!(all_layers(s2).iter().any(|l| l.id == "b2.highlight"));
}

// ---------------------------------------------------------------------------
// HeroObject
// ---------------------------------------------------------------------------

#[test]
fn hero_object_carried_across_beats_becomes_a_shared_element() {
    let i = intent(json!({
        "version": "0.2", "title": "carry", "format": "vertical",
        "beats": [
            {
                "purpose": "emphasize",
                "statement": "The basket is the whole story",
                "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "the basket" },
                "energy": "building",
                "continuity": "carry_primary"
            },
            {
                "purpose": "emphasize",
                "statement": "And it keeps growing",
                "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "the basket" },
                "secondary": { "kind": "phrase", "value": "every week." },
                "energy": "building"
            }
        ]
    }));
    for seed in 1..=4 {
        let p = build(&i, &style(seed, "auto"), None);
        assert_valid(&p, "carry");
        assert_nothing_new_after_anticipate(&p, "carry");
        assert_eq!(p.shared.len(), 1, "one shared element");
        let scenes: BTreeSet<&str> = p.shared[0].track.iter().map(|k| k.scene.as_str()).collect();
        assert!(
            scenes.contains("beat_1") && scenes.contains("beat_2"),
            "{scenes:?}"
        );
    }
}

#[test]
fn hero_object_dominates_and_drifts_slowly_when_local() {
    let p = build(&golden("hero_object"), &style(1, "auto"), None);
    assert_valid(&p, "hero");
    let s = beat_scene(&p, 1);
    let life = s.lifecycle.unwrap();
    let hero = all_layers(s)
        .into_iter()
        .find(|l| l.id.starts_with("b1.hero."))
        .expect("local hero layer");
    let short = p.canvas.width.min(p.canvas.height) as f32;
    assert!(
        hero.width > 0.5 * short && hero.width < 0.6 * short,
        "≈55% of short side"
    );
    let drift = s
        .motions
        .iter()
        .find(|m| m.target == hero.id && matches!(m.op, motion_core::scene::MotionOp::Scale { .. }))
        .expect("READ drift");
    assert!((drift.start - life.read).abs() < 1e-3);
    assert!(drift.start + drift.duration <= life.anticipate + 1e-3);
}

// ---------------------------------------------------------------------------
// CinematicMultiplane
// ---------------------------------------------------------------------------

#[test]
fn multiplane_layers_span_at_least_three_depths() {
    for seed in 1..=4 {
        let p = build(&golden("multiplane"), &style(seed, "parallax"), None);
        assert_valid(&p, "multiplane");
        let s = beat_scene(&p, 1);
        let depths: BTreeSet<i64> = all_layers(s)
            .iter()
            .filter_map(|l| l.depth)
            .map(|d| (d * 100.0).round() as i64)
            .collect();
        assert!(depths.len() >= 3, "seed {seed}: depths {depths:?}");
        assert!(depths.contains(&35) && depths.contains(&70) && depths.contains(&145));
        assert!(s.camera.is_some(), "the scene camera moves the planes");
    }
}

#[test]
fn multiplane_uses_a_delivered_environment_plate() {
    let mut m = AssetManifest::empty();
    let mut entry = manifest().assets[1].clone();
    entry.id = "beat_1.environment".into();
    m.assets.push(entry);
    let p = build(&golden("multiplane"), &style(1, "parallax"), Some(&m));
    assert_valid(&p, "multiplane env");
    let s = beat_scene(&p, 1);
    let env = image_layer(s, "asset.beat_1.environment").expect("environment plate");
    // 0.5 Task D: the environment is the full-bleed background plane.
    assert_eq!(env.depth, Some(0.35));
    assert!(p.assets.iter().any(|a| a.id == "asset.beat_1.environment"));
}

// ---------------------------------------------------------------------------
// Throwaway: write compiled scenes for visual checks
// (`cargo test -p motion-core --test grammar_image -- --ignored write_visual_scenes`)
// ---------------------------------------------------------------------------

#[test]
#[ignore]
fn write_visual_scenes() {
    let out = repo().join("output/c3");
    std::fs::create_dir_all(&out).unwrap();
    let m = manifest();
    let mut story = build(&two_beat_intent(), &style(1, "auto"), Some(&m));
    story.asset_root = Some("../../assets".into());
    std::fs::write(
        out.join("story_img.motion.json"),
        serde_json::to_string_pretty(&story).unwrap(),
    )
    .unwrap();
    for name in ["multiplane", "hero_object", "evidence_stack", "type_image"] {
        for (tag, with) in [("", None), ("_img", Some(&m))] {
            for seed in [1u64, 2] {
                let mut p = build(&golden(name), &style(seed, "auto"), with);
                p.asset_root = Some("../../assets".into());
                let path = out.join(format!("{name}{tag}_s{seed}.motion.json"));
                std::fs::write(path, serde_json::to_string_pretty(&p).unwrap()).unwrap();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 0.5 asset carry: one generated image persists across consecutive beats
// ---------------------------------------------------------------------------

fn worker_story() -> CreativeIntent {
    intent(json!({
        "version": "0.2", "title": "worker", "format": "vertical",
        "beats": [
            { "purpose": "emphasize", "statement": "An office worker sits alone at 11:47 PM.",
              "primary": { "kind": "phrase", "value": "Office worker", "meaning": "worker" },
              "energy": "calm", "keyword": "night" },
            { "purpose": "emphasize", "statement": "The office worker is still there at dawn.",
              "primary": { "kind": "phrase", "value": "Office worker", "meaning": "worker" },
              "energy": "building", "keyword": "dawn" }
        ]
    }))
}

fn worker_manifest(shared: bool) -> AssetManifest {
    let entry = |id: &str, serves: Vec<&str>| {
        json!({ "id": id, "path": "test_images/figure.png", "width": 600, "height": 900,
                "alpha": true, "face_anchor": { "x": 0.5, "y": 0.21 },
                "serves": serves })
    };
    let assets = if shared {
        vec![entry(
            "beat_1.hero_subject",
            vec!["beat_1.hero_subject", "beat_2.hero_subject"],
        )]
    } else {
        vec![
            entry("beat_1.hero_subject", vec![]),
            entry("beat_2.hero_subject", vec![]),
        ]
    };
    serde_json::from_value(json!({ "version": "0.2", "assets": assets })).expect("manifest")
}

#[test]
fn one_generated_image_persists_across_consecutive_interlock_beats() {
    let p = build(
        &worker_story(),
        &style(1, "auto"),
        Some(&worker_manifest(true)),
    );
    assert_valid(&p, "asset carry");
    let shared: Vec<_> = p
        .shared
        .iter()
        .filter(|e| matches!(&e.layer.kind, LayerKind::Image { .. }))
        .collect();
    assert_eq!(shared.len(), 1, "one shared image element");
    let e = shared[0];
    let scenes: std::collections::BTreeSet<&str> =
        e.track.iter().map(|k| k.scene.as_str()).collect();
    assert!(
        scenes.contains("beat_1") && scenes.contains("beat_2"),
        "{scenes:?}"
    );
    // The carried image gets an accent tint emphasis in beat 2 that ends neutral.
    let tints: Vec<_> = beat_scene(&p, 2)
        .motions
        .iter()
        .filter(|m| m.target == e.layer.id && m.op.op_name() == "tint")
        .collect();
    assert_eq!(tints.len(), 2, "tint up and back");
    assert!(matches!(tints[1].op, motion_core::scene::MotionOp::Tint { to, .. } if to == 0.0));
    // No scene-local copy of the image in either beat.
    for n in [1, 2] {
        assert!(image_layer(beat_scene(&p, n), "asset.beat_1.hero_subject").is_none());
    }
    // One asset registered (never regenerated per scene).
    assert_eq!(
        p.assets
            .iter()
            .filter(|a| a.path == "test_images/figure.png")
            .count(),
        1
    );
    assert_nothing_new_after_anticipate(&p, "asset carry");
}

#[test]
fn different_images_do_not_carry() {
    let p = build(
        &worker_story(),
        &style(1, "auto"),
        Some(&worker_manifest(false)),
    );
    assert_valid(&p, "no carry");
    assert!(p
        .shared
        .iter()
        .all(|e| !matches!(&e.layer.kind, LayerKind::Image { .. })));
    assert!(image_layer(beat_scene(&p, 1), "asset.beat_1.hero_subject").is_some());
    assert!(image_layer(beat_scene(&p, 2), "asset.beat_2.hero_subject").is_some());
}
