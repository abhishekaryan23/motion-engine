//! (0.19) The `layers` subject compiled: one persistent column in the
//! backdrop, the lit layer gliding to the same place on screen beat after beat,
//! every layer named, and the beat's secondary subject pinned inside the lit
//! layer. Built from in-repo data only (the committed `clay_concepts_3d`
//! family), with the cinematic look (perspective camera, FX director) and
//! without any look.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{CameraOp, LayerKind, MotionProject};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use motion_core::validate::validate;
use serde_json::{json, Value};

const H: f64 = 1920.0;
/// Where the lit layer's centre sits: 60 % of the content canvas.
const TARGET_Y: f64 = 0.60 * H;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn stack(focus: Option<&str>) -> Value {
    let mut s = json!({
        "kind": "layers",
        "meaning": "water column",
        "layers": [
            { "name": "Sunlit", "note": "bright, warm" },
            { "name": "Divide", "note": "the boundary", "boundary": true },
            { "name": "Dark", "note": "cold, still" }
        ]
    });
    if let Some(f) = focus {
        s["focus"] = json!(f);
    }
    s
}

fn beat(statement: &str, primary: Value, secondary: Option<(&str, &str)>) -> Value {
    let mut b = json!({
        "purpose": "emphasize", "statement": statement, "primary": primary,
        "narration": "", "energy": "building"
    });
    if let Some((asset, meaning)) = secondary {
        b["secondary"] = json!({ "kind": "object", "asset": asset, "meaning": meaning });
    }
    b
}

/// Three beats about one stack (top, bottom, boundary) and a closing phrase.
fn story() -> Value {
    json!({
        "version": "0.2", "title": "layers_story", "format": "vertical",
        "beats": [
            beat("The top", stack(Some("Sunlit")), Some(("atom", "particles"))),
            beat("The bottom", stack(Some("Dark")), Some(("brain", "memory"))),
            beat("The divide", stack(Some("Divide")), Some(("compass", "direction"))),
            {
                "purpose": "emphasize", "statement": "All of it",
                "primary": { "kind": "phrase", "value": "one column" },
                "narration": "", "energy": "calm"
            }
        ]
    })
}

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(
        r#"{{"tone":"{tone}","polarity":"dark","temperature":"cool","temperament":"balanced",
        "density":"balanced"}}"#
    ))
    .expect("style")
}

fn compile_story(v: &Value, tone: &str, art: bool) -> MotionProject {
    let intent: CreativeIntent = serde_json::from_value(v.clone()).expect("intent");
    let library = AssetLibrary::new(repo().join("assets"))
        .with_families(vec!["clay_concepts_3d".to_string()]);
    let opts = CompileOptions {
        art: art.then_some(ArtMode::Auto),
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &style(tone),
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn find<'a, 'b>(layers: &'b [ResolvedLayer<'a>], id: &str) -> Option<&'b ResolvedLayer<'a>> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let Some(c) = find(&l.children, id) {
            return Some(c);
        }
    }
    None
}

/// Canvas point of a box-space point of a resolved layer.
fn at(l: &ResolvedLayer<'_>, x: f64, y: f64) -> [f64; 2] {
    let t = l.transform;
    [
        f64::from(t.a) * x + f64::from(t.c) * y + f64::from(t.e),
        f64::from(t.b) * x + f64::from(t.d) * y + f64::from(t.f),
    ]
}

/// A frame well inside beat `n` (1-based), after the column has glided and the
/// beat's subject has arrived.
fn read_frame(p: &MotionProject, n: usize) -> u32 {
    let s = p
        .scenes
        .iter()
        .find(|s| s.id == format!("beat_{n}"))
        .expect("beat scene");
    ((s.start_seconds + 0.62 * s.duration_seconds) * f64::from(p.canvas.fps)) as u32
}

fn resolved<'a>(p: &'a MotionProject, n: usize) -> motion_core::timeline::ResolvedFrame<'a> {
    evaluate_frame(p, read_frame(p, n)).expect("frame")
}

#[test]
fn the_column_is_one_group_in_the_backdrop_for_the_whole_run() {
    for (tone, art) in [("cinematic", true), ("editorial", false)] {
        let p = compile_story(&story(), tone, art);
        validate(&p, None).unwrap_or_else(|e| panic!("{tone}: {e:?}"));
        let backdrop = p
            .scenes
            .iter()
            .find(|s| s.id == "backdrop")
            .expect("backdrop");
        let columns: Vec<&str> = backdrop
            .layers
            .iter()
            .filter(|l| l.id.starts_with("backdrop.stack"))
            .map(|l| l.id.as_str())
            .collect();
        assert_eq!(
            columns,
            ["backdrop.stack0.column"],
            "{tone}: one run, one column"
        );
        // Every layer is named: a card at the top edge of each, a second at
        // the bottom edge of each ordinary one.
        let LayerKind::Group { children } = &backdrop
            .layers
            .iter()
            .find(|l| l.id == "backdrop.stack0.column")
            .expect("column")
            .kind
        else {
            panic!("column is not a group");
        };
        let ids: Vec<&str> = children.iter().map(|c| c.id.as_str()).collect();
        for want in [
            "backdrop.stack0.card0",
            "backdrop.stack0.card1",
            "backdrop.stack0.card2",
            "backdrop.stack0.bcard0",
        ] {
            assert!(ids.contains(&want), "{tone}: missing {want}");
        }
        // A thin boundary has one card, not two.
        assert!(
            !ids.contains(&"backdrop.stack0.bcard1"),
            "{tone}: boundary has no bottom card"
        );
    }
}

/// Canvas y of the top and bottom edge of layer `i` (its seams are never
/// culled: they stay visible whatever is lit).
fn band_span(f: &motion_core::timeline::ResolvedFrame<'_>, i: usize) -> (f64, f64) {
    let y = |k: usize| {
        let seam = find(&f.layers, &format!("backdrop.stack0.seam{k}"))
            .unwrap_or_else(|| panic!("seam{k} missing"));
        at(
            seam,
            f64::from(seam.width) / 2.0,
            f64::from(seam.height) / 2.0,
        )[1]
    };
    (y(i), y(i + 1))
}

#[test]
fn the_lit_layer_is_at_the_same_place_on_screen_in_every_beat() {
    let p = compile_story(&story(), "cinematic", true);
    // (beat, lit layer index)
    for (n, lit) in [(1usize, 0usize), (2, 2), (3, 1)] {
        let f = resolved(&p, n);
        let (top, bottom) = band_span(&f, lit);
        let centre = 0.5 * (top + bottom);
        assert!(
            (centre - TARGET_Y).abs() < 2.0,
            "beat {n}: layer {lit} centre {centre:.1}, wanted {TARGET_Y}"
        );
        // Lit: no veil (a fully transparent layer is not drawn). Every other
        // layer is dimmed.
        for other in 0..3 {
            let v = find(&f.layers, &format!("backdrop.stack0.veil{other}"));
            if other == lit {
                assert!(
                    v.is_none_or(|v| v.opacity < 0.01),
                    "beat {n}: lit layer {other} is veiled"
                );
            } else {
                let v = v.unwrap_or_else(|| panic!("beat {n}: layer {other} has no veil"));
                assert!(
                    (v.opacity - 0.58).abs() < 0.02,
                    "beat {n}: layer {other} veil {}",
                    v.opacity
                );
            }
        }
    }
}

#[test]
fn the_secondary_subject_is_pinned_inside_the_lit_layer() {
    let p = compile_story(&story(), "cinematic", true);
    for (n, lit) in [(1usize, 0usize), (2, 2), (3, 1)] {
        let f = resolved(&p, n);
        let pinned = find(&f.layers, &format!("b{n}.pinned.depth"))
            .unwrap_or_else(|| panic!("beat {n}: nothing pinned"));
        let c = at(
            pinned,
            f64::from(pinned.width) / 2.0,
            f64::from(pinned.height) / 2.0,
        );
        let (top, bottom) = band_span(&f, lit);
        // A thin boundary is straddled by its subject; an ordinary layer holds it.
        let slack = if lit == 1 { 0.10 * H } else { 0.0 };
        assert!(
            c[1] >= top - slack && c[1] <= bottom + slack,
            "beat {n}: pinned centre {:.0} outside layer {lit} [{top:.0}, {bottom:.0}]",
            c[1]
        );
        assert!(
            c[0] > 0.5 * 1080.0,
            "beat {n}: pinned subject is on the right half"
        );
        // The picture (its box includes transparent padding) stays on the
        // canvas; its caption stays inside the safe margins.
        let left = at(pinned, 0.0, 0.0)[0];
        let right = at(pinned, f64::from(pinned.width), 0.0)[0];
        assert!(
            left >= 0.0 && right <= 1080.0,
            "beat {n}: pinned picture x {left:.0}..{right:.0} leaves the canvas"
        );
        let label = find(&f.layers, &format!("b{n}.pinned_label.depth"))
            .unwrap_or_else(|| panic!("beat {n}: no caption"));
        let (l0, l1) = (
            at(label, 0.0, 0.0)[0],
            at(label, f64::from(label.width), 0.0)[0],
        );
        assert!(
            l0 >= 84.0 - 1.0 && l1 <= 1080.0 - 84.0 + 1.0,
            "beat {n}: caption x {l0:.0}..{l1:.0} leaves the safe margins"
        );
    }
}

#[test]
fn layer_beats_have_a_still_camera() {
    let p = compile_story(&story(), "cinematic", true);
    for n in 1..=3 {
        let s = p
            .scenes
            .iter()
            .find(|s| s.id == format!("beat_{n}"))
            .expect("scene");
        let cam = s.camera.as_ref().expect("perspective camera for the tilt");
        assert!(cam.perspective.is_some(), "beat {n}: no perspective");
        assert!(
            cam.motions.iter().all(|m| !matches!(
                m.op,
                CameraOp::Dolly { .. }
                    | CameraOp::Orbit { .. }
                    | CameraOp::Track { .. }
                    | CameraOp::Push { .. }
                    | CameraOp::Shake { .. }
            )),
            "beat {n}: the camera moves: {:?}",
            cam.motions
        );
        assert_eq!(
            cam.perspective.as_ref().map(|p| p.aperture),
            Some(0.0),
            "beat {n}: no depth of field on a diagram"
        );
    }
    // Without a look the layer beats have no camera at all.
    let plain = compile_story(&story(), "editorial", false);
    for n in 1..=3 {
        let s = plain
            .scenes
            .iter()
            .find(|s| s.id == format!("beat_{n}"))
            .expect("scene");
        assert!(s.camera.is_none(), "beat {n}: camera without a look");
    }
}

#[test]
fn the_column_stays_through_the_run_and_leaves_after_it() {
    let p = compile_story(&story(), "cinematic", true);
    let col = |n: usize| {
        find(&resolved(&p, n).layers, "backdrop.stack0.column")
            .map(|l| l.opacity)
            .unwrap_or(0.0)
    };
    for n in 1..=3 {
        assert!(col(n) > 0.99, "beat {n}: column opacity {}", col(n));
    }
    assert!(col(4) < 0.01, "the phrase beat has no column: {}", col(4));
}

#[test]
fn a_different_stack_is_a_new_run_with_its_own_column() {
    let mut v = story();
    let beats = v["beats"].as_array_mut().expect("beats");
    let other = json!({
        "kind": "layers",
        "layers": [{ "name": "Before" }, { "name": "After" }],
        "focus": "After"
    });
    beats.push(beat("Another view", other, None));
    let p = compile_story(&v, "cinematic", true);
    let backdrop = p
        .scenes
        .iter()
        .find(|s| s.id == "backdrop")
        .expect("backdrop");
    let ids: Vec<&str> = backdrop
        .layers
        .iter()
        .filter(|l| l.id.starts_with("backdrop.stack"))
        .map(|l| l.id.as_str())
        .collect();
    assert_eq!(ids, ["backdrop.stack0.column", "backdrop.stack1.column"]);
}

#[test]
fn stories_without_layers_are_untouched() {
    let v = json!({
        "version": "0.2", "title": "plain", "format": "vertical",
        "beats": [{
            "purpose": "emphasize", "statement": "Just words",
            "primary": { "kind": "phrase", "value": "hello" }
        }]
    });
    let p = compile_story(&v, "cinematic", true);
    let backdrop = p
        .scenes
        .iter()
        .find(|s| s.id == "backdrop")
        .expect("backdrop");
    assert!(backdrop
        .layers
        .iter()
        .all(|l| !l.id.starts_with("backdrop.stack")));
}

#[test]
fn compiling_twice_gives_the_same_scene() {
    let a = serde_json::to_string(&compile_story(&story(), "cinematic", true)).expect("json");
    let b = serde_json::to_string(&compile_story(&story(), "cinematic", true)).expect("json");
    assert_eq!(a, b);
}

#[test]
fn layers_shown_apart_light_the_seams_between_layers_only() {
    let mut v = story();
    // Beat 2 (focus: the bottom layer) shown apart.
    v["beats"][1]["relationship"] = json!("separate");
    let p = compile_story(&v, "cinematic", true);
    let f = resolved(&p, 2);
    let seam = |k: usize| {
        find(&f.layers, &format!("backdrop.stack0.seam{k}"))
            .unwrap_or_else(|| panic!("seam{k} missing"))
            .opacity
    };
    // Between the layers: lit. The top edge is neither between layers nor next
    // to the lit one; the bottom edge borders the lit layer, so it is lit too.
    assert!(
        seam(1) > 0.9 && seam(2) > 0.9,
        "inner seams: {} {}",
        seam(1),
        seam(2)
    );
    assert!((seam(0) - 0.22).abs() < 0.02, "top edge {}", seam(0));
    assert!(seam(3) > 0.9, "the lit layer's own bottom edge {}", seam(3));
    // Apart with the TOP layer lit: the bottom edge is not lit.
    let mut v = story();
    v["beats"][0]["relationship"] = json!("separate");
    let p = compile_story(&v, "cinematic", true);
    let f = resolved(&p, 1);
    let bottom = find(&f.layers, "backdrop.stack0.seam3")
        .expect("seam3")
        .opacity;
    assert!(
        (bottom - 0.22).abs() < 0.02,
        "bottom edge lit though nothing borders it: {bottom}"
    );
}
