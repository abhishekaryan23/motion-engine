//! Asset-aware variants of HeroObject, CinematicMultiplane and EvidenceStack
//! (0.5 Task D): a delivered `hero_object` / `environment` / `evidence_image`
//! changes the composition; without one the output is unchanged.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::{compile, compile_with_assets, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject, Scene};
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

fn manifest(entries: Vec<Value>) -> AssetManifest {
    serde_json::from_value(json!({ "version": "0.2", "assets": entries })).expect("manifest")
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

fn layer_named<'a>(s: &'a Scene, id: &str) -> &'a Layer {
    all_layers(s)
        .into_iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("layer {id}"))
}

fn image_layers(s: &Scene) -> Vec<&Layer> {
    all_layers(s)
        .into_iter()
        .filter(|l| matches!(l.kind, LayerKind::Image { .. }))
        .collect()
}

/// Top-left box of a layer (its x/y is the anchor point).
fn bbox(l: &Layer) -> (f32, f32, f32, f32) {
    (
        l.x - l.anchor_x * l.width,
        l.y - l.anchor_y * l.height,
        l.width,
        l.height,
    )
}

/// Nothing new starts at/after ANTICIPATE except anticipation motions.
fn assert_nothing_new_after_anticipate(p: &MotionProject, what: &str) {
    for (n, s) in p.scenes.iter().filter(|s| s.id != "backdrop").enumerate() {
        let life = s.lifecycle.expect("compiled scenes carry a lifecycle");
        let stage = format!("b{}.stage", n + 1);
        for m in &s.motions {
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

// ---------------------------------------------------------------------------
// HeroObject
// ---------------------------------------------------------------------------

/// A hero-object cutout whose analyzed subject box is x .2 y .2 w .6 h .6.
fn hero_entry() -> Value {
    json!({
        "id": "beat_1.hero_object", "path": "test_images/object.png",
        "width": 800, "height": 800, "alpha": true,
        "analysis": {
            "subject_bounds": { "x": 0.2, "y": 0.2, "width": 0.6, "height": 0.6 },
            "edges": { "top": false, "bottom": false, "left": false, "right": false },
            "coverage": 0.3,
            "occupancy": { "cols": 4, "rows": ["0000", "0ff0", "0ff0", "0000"] },
            "safe_regions": []
        }
    })
}

#[test]
fn hero_object_image_is_sized_by_its_subject_bounds() {
    let i = golden("hero_object");
    for seed in 1..=4 {
        let p = build(
            &i,
            &style(seed, "auto"),
            Some(&manifest(vec![hero_entry()])),
        );
        assert_valid(&p, "hero image");
        assert_nothing_new_after_anticipate(&p, "hero image");
        let s = beat_scene(&p, 1);
        let life = s.lifecycle.unwrap();
        let imgs = image_layers(s);
        assert_eq!(imgs.len(), 1, "one image layer (the hero)");
        let hero = imgs[0];
        assert!(
            matches!(&hero.kind, LayerKind::Image { asset, .. } if asset == "asset.beat_1.hero_object")
        );
        assert!(p.assets.iter().any(|a| a.id == "asset.beat_1.hero_object"));
        // The library object is gone.
        assert!(!all_layers(s).iter().any(|l| matches!(&l.kind,
            LayerKind::Svg { asset, .. } | LayerKind::Image { asset, .. } if asset == "asset.shopping_basket")));

        // Subject box (not the transparent margins) ≈ 55 % of the short side.
        let short = p.canvas.width.min(p.canvas.height) as f32;
        let (x, y, w, h) = bbox(hero);
        let (sw, sh) = (0.6 * w, 0.6 * h);
        let target = 0.55 * short;
        assert!(
            (sw - target).abs() < 0.02 * target && (sh - target).abs() < 0.02 * target,
            "seed {seed}: subject {sw}x{sh}, target {target}"
        );
        // ... and it sits inside the canvas.
        let (scx, scy) = (x + 0.5 * w, y + 0.5 * h);
        assert!(scx > 0.0 && scx < p.canvas.width as f32);
        assert!(scy > 0.0 && scy < p.canvas.height as f32);

        // Lifecycle: arrives in ENTER (mask), slow scale drift in READ.
        assert!(s.motions.iter().any(|m| m.target == hero.id
            && matches!(m.op, MotionOp::MaskReveal { .. })
            && m.start < life.settle + 1.0));
        let drift = s
            .motions
            .iter()
            .find(|m| m.target == hero.id && matches!(m.op, MotionOp::Scale { .. }))
            .expect("READ drift");
        assert!((drift.start - life.read).abs() < 1e-3);
        assert!(drift.start + drift.duration <= life.anticipate + 1e-3);
        // EVOLVE secondary still arrives.
        assert!(all_layers(s).iter().any(|l| l.id.starts_with("b1.serif")));
    }
}

#[test]
fn hero_object_without_the_image_is_unchanged() {
    let i = golden("hero_object");
    let s = style(1, "auto");
    let plain = serde_json::to_string(&build(&i, &s, None)).unwrap();
    let empty = serde_json::to_string(&build(&i, &s, Some(&AssetManifest::empty()))).unwrap();
    let unrelated = manifest(vec![json!({
        "id": "beat_7.hero_object", "path": "test_images/object.png",
        "width": 800, "height": 800, "alpha": true
    })]);
    let other = serde_json::to_string(&build(&i, &s, Some(&unrelated))).unwrap();
    assert_eq!(plain, empty);
    assert_eq!(plain, other);
}

// ---------------------------------------------------------------------------
// CinematicMultiplane
// ---------------------------------------------------------------------------

fn env_entry(w: u32, h: u32) -> Value {
    json!({
        "id": "beat_1.environment", "path": "test_images/environment.jpg",
        "width": w, "height": h, "alpha": false
    })
}

#[test]
fn multiplane_environment_is_a_full_bleed_background_plane() {
    let i = golden("multiplane");
    // A landscape plate and a portrait one both cover the vertical canvas.
    for (iw, ih) in [(1200, 800), (800, 1200)] {
        for seed in 1..=4 {
            let p = build(
                &i,
                &style(seed, "parallax"),
                Some(&manifest(vec![env_entry(iw, ih)])),
            );
            let what = format!("multiplane env {iw}x{ih} seed {seed}");
            assert_valid(&p, &what);
            assert_nothing_new_after_anticipate(&p, &what);
            let s = beat_scene(&p, 1);
            let life = s.lifecycle.unwrap();
            let imgs = image_layers(s);
            assert_eq!(imgs.len(), 1, "{what}: only the environment image");
            let env = imgs[0];
            assert!(
                matches!(&env.kind, LayerKind::Image { asset, .. } if asset == "asset.beat_1.environment")
            );
            assert!(env.z_index <= 2, "{what}: z {}", env.z_index);

            // Covers the canvas with a bleed (all four corners inside).
            let (cw, ch) = (p.canvas.width as f32, p.canvas.height as f32);
            let (x, y, w, h) = bbox(env);
            assert!(
                x <= -0.05 * cw && y <= -0.05 * ch && x + w >= 1.05 * cw && y + h >= 1.05 * ch,
                "{what}: box {x},{y} {w}x{h} on {cw}x{ch}"
            );
            // Aspect preserved (Cover semantics, never stretched).
            assert!((w / h - iw as f32 / ih as f32).abs() < 0.01 * (iw as f32 / ih as f32));

            // Background depth plane, below the halftone field, and not the midground.
            let ghost = layer_named(s, "b1.ghost");
            assert_eq!(env.depth, ghost.depth, "{what}: background plane");
            assert_eq!(env.depth, Some(0.35));
            let field = layer_named(s, "b1.field");
            assert!(env.z_index < field.z_index);
            assert!(image_layers(s).iter().all(|l| l.depth != Some(0.7)));

            // The veil sits on the same plane, above the plate, below the field.
            let veil = layer_named(s, "b1.env.veil");
            assert_eq!(veil.depth, env.depth);
            assert!(veil.z_index > env.z_index && veil.z_index < field.z_index);
            assert!(matches!(&veil.kind, LayerKind::Rectangle { fill, .. } if fill.a == 0x99));

            // Slow fade 0 -> 1 from the scene enter.
            let fade = s
                .motions
                .iter()
                .find(|m| m.target == env.id && matches!(m.op, MotionOp::Fade { .. }))
                .expect("fade in");
            assert!(
                fade.start >= life.enter - 1e-6,
                "{what}: fade {}",
                fade.start
            );
            assert!(matches!(fade.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0));
            assert!(fade.duration > 0.9 && fade.duration < 1.5);
            // No wipe reveal on the plane.
            assert!(!s
                .motions
                .iter()
                .any(|m| m.target == env.id && matches!(m.op, MotionOp::MaskReveal { .. })));

            // Gentle linear drift only in READ..ANTICIPATE.
            let drift = s
                .motions
                .iter()
                .find(|m| m.target == env.id && matches!(m.op, MotionOp::Move { .. }))
                .expect("drift");
            assert!((drift.start - life.read).abs() < 1e-3);
            assert!(drift.start + drift.duration <= life.anticipate + 1e-3);
            assert_eq!(drift.easing, motion_core::easing::Easing::Linear);
            let MotionOp::Move { from, to } = drift.op else {
                unreachable!()
            };
            let dist = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
            assert!(dist > 0.0 && dist <= 30.0, "{what}: drift {dist}");
            // The drift stays inside the bleed.
            assert!(dist < 0.05 * cw);
            assert!(p.assets.iter().any(|a| a.id == "asset.beat_1.environment"));
        }
    }
}

#[test]
fn multiplane_without_an_environment_is_unchanged() {
    let i = golden("multiplane");
    let s = style(1, "parallax");
    let plain = serde_json::to_string(&build(&i, &s, None)).unwrap();
    let empty = serde_json::to_string(&build(&i, &s, Some(&AssetManifest::empty()))).unwrap();
    let unrelated = manifest(vec![json!({
        "id": "beat_3.environment", "path": "test_images/environment.jpg",
        "width": 1200, "height": 800, "alpha": false
    })]);
    let other = serde_json::to_string(&build(&i, &s, Some(&unrelated))).unwrap();
    assert_eq!(plain, empty);
    assert_eq!(plain, other);
    let p: MotionProject = serde_json::from_str(&plain).unwrap();
    let scene = beat_scene(&p, 1);
    assert!(image_layers(scene).is_empty());
}

// ---------------------------------------------------------------------------
// EvidenceStack
// ---------------------------------------------------------------------------

fn evidence_entry(w: u32, h: u32) -> Value {
    json!({
        "id": "beat_1.evidence_image", "path": "test_images/document.png",
        "width": w, "height": h, "alpha": true
    })
}

#[test]
fn evidence_card_follows_the_image_aspect() {
    let i = golden("evidence_stack");
    for (iw, ih) in [(1200u32, 800u32), (800, 1200), (1000, 1000)] {
        for seed in 1..=4 {
            let p = build(
                &i,
                &style(seed, "auto"),
                Some(&manifest(vec![evidence_entry(iw, ih)])),
            );
            let what = format!("evidence {iw}x{ih} seed {seed}");
            assert_valid(&p, &what);
            assert_nothing_new_after_anticipate(&p, &what);
            let s = beat_scene(&p, 1);
            let want = iw as f32 / ih as f32;
            let doc = layer_named(s, "b1.doc");
            assert!(matches!(&doc.kind, LayerKind::Image { .. }));
            let card = layer_named(s, "b1.sheet.1");
            for l in [doc, card] {
                let got = l.width / l.height;
                assert!(
                    (got - want).abs() <= 0.04 * want,
                    "{what}: {} aspect {got} vs {want}",
                    l.id
                );
            }
            // Everything stays on the canvas (the tilted card included).
            let (cw, ch) = (p.canvas.width as f32, p.canvas.height as f32);
            let (x, y, w, h) = bbox(doc);
            assert!(x >= 0.0 && y >= 0.0 && x + w <= cw && y + h <= ch, "{what}");
            // Highlight bar and the source label still exist, below the card.
            let label = layer_named(s, "b1.source");
            assert!(label.y >= y + h - 1.0, "{what}: source under the card");
            assert!(all_layers(s).iter().any(|l| l.id == "b1.highlight"));
        }
    }
}

#[test]
fn evidence_without_the_image_is_unchanged() {
    let i = golden("evidence_stack");
    let s = style(1, "auto");
    let plain = serde_json::to_string(&build(&i, &s, None)).unwrap();
    let unrelated = manifest(vec![json!({
        "id": "beat_5.evidence_image", "path": "test_images/document.png",
        "width": 1200, "height": 800, "alpha": true
    })]);
    let other = serde_json::to_string(&build(&i, &s, Some(&unrelated))).unwrap();
    assert_eq!(plain, other);
}
