//! (0.18) Focal-first beats and the continuous camera, from in-repo data only.
//!
//! * Focal QA: the compiler records what each beat is about
//!   (`ProjectMeta.art.focal`: scene id -> layer id); layout QA checks that the
//!   layer is sharp (`focal_out_of_focus`) and that the stamp touches it
//!   (`emphasis_off_focal`). The dossier (documentary) and the cinematic look
//!   are both compiled and resolved through the timeline.
//! * Continuous camera (`compiler/fx.rs` `choreograph`): the fly-in starts at
//!   ENTER, the dolly's three acts hand their speed over
//!   (`CameraMotion.velocity`, Hermite), focus tracks the focal plane after the
//!   fly-in, the orbit pivots on the focal layer and, like track, runs linear
//!   across the beat, the camera (not the lifecycle) carries the beat out, and
//!   only the cinematic look billboards its sprites.
//!
//! Act boundaries are always read from the scene's camera motions. The camera
//! is sampled by a small re-statement of the spec in this file
//! (most-recent-start-wins hold, cubic Hermite progress) which is itself
//! checked against the resolved scale of a probe plane, so no test trusts a
//! helper that the timeline does not agree with.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::{LayoutCheck, FOCAL_BLUR_MAX};
use motion_core::scene::{
    Camera, CameraMotion, CameraOp, Layer, LayerKind, MotionOp, MotionProject, Scene,
};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use motion_core::validate::validate;
use motion_core::{layout_report, Easing, LayoutReport};
use serde_json::json;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(
        r#"{{"tone":"{tone}","polarity":"dark","temperature":"warm","temperament":"balanced",
        "density":"dense","accent_role":"signal_red","texture_style":"heavy_print",
        "depth":"layered","camera_style":"drift","motion_language":"parallax"}}"#
    ))
    .expect("style")
}

/// The documentary fixture of `genre_grammars.rs`: a figure with a library
/// object, a phrase with a number and an object, and a phrase-only beat.
const DOCUMENTARY: &str = r#"{
  "version": "0.2", "title": "genre_grammars", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Cash is king",
     "primary": {"kind": "phrase", "value": "$381 billion", "meaning": "cash reserves"},
     "secondary": {"kind": "object", "asset": "money_bag", "meaning": "money"},
     "energy": "impact", "keyword": "cash"},
    {"purpose": "emphasize", "statement": "Robots keep selling for 36 months",
     "primary": {"kind": "number", "value": "36 months", "meaning": "selling"},
     "secondary": {"kind": "object", "asset": "robot", "meaning": "robot"},
     "energy": "building", "keyword": "selling"},
    {"purpose": "emphasize", "statement": "Still waiting for the crash",
     "primary": {"kind": "phrase", "value": "still waiting"},
     "energy": "impact", "keyword": "waiting"}
  ]
}"#;

/// The other two focal cards of the dossier: a picture without a figure (the
/// photo is what the beat is about) and a long phrase (only the clipping).
const DOCUMENTARY_PICTURE_AND_CLIPPING: &str = r#"{
  "version": "0.2", "title": "focal_variants", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "The robot arrives",
     "primary": {"kind": "object", "asset": "robot", "meaning": "robot"},
     "energy": "impact", "keyword": "robot"},
    {"purpose": "emphasize", "statement": "Nothing about this is simple",
     "primary": {"kind": "phrase", "value": "this is a long phrase without digits", "meaning": "x"},
     "energy": "impact", "keyword": "simple"},
    {"purpose": "emphasize", "statement": "Last words",
     "primary": {"kind": "phrase", "value": "this is a long phrase without digits"},
     "energy": "calm", "keyword": "words"}
  ]
}"#;

fn compile_intent(
    intent: &CreativeIntent,
    style: &StyleProfile,
    canvas: Option<(u32, u32)>,
) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        canvas,
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        intent,
        style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn documentary(json: &str, canvas: Option<(u32, u32)>) -> MotionProject {
    let intent = CreativeIntent::from_json(json).expect("intent");
    compile_intent(&intent, &style("documentary"), canvas)
}

/// An in-repo cinematic example (`examples/cinematic/<name>.{intent,style}.json`).
fn cinematic(name: &str) -> MotionProject {
    let read = |ext: &str| {
        std::fs::read_to_string(repo().join(format!("examples/cinematic/{name}.{ext}.json")))
            .unwrap_or_else(|e| panic!("{name}.{ext}.json: {e}"))
    };
    let intent = CreativeIntent::from_json(&read("intent")).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read("style")).expect("style");
    compile_intent(&intent, &style, None)
}

/// A compile to test: `cinematic` tells which look it must have.
struct Case {
    name: &'static str,
    cinematic: bool,
    project: MotionProject,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "documentary",
            cinematic: false,
            project: documentary(DOCUMENTARY, None),
        },
        Case {
            name: "documentary picture and clipping",
            cinematic: false,
            project: documentary(DOCUMENTARY_PICTURE_AND_CLIPPING, None),
        },
        Case {
            name: "cinematic ai_age",
            cinematic: true,
            project: cinematic("ai_age"),
        },
        Case {
            name: "cinematic space",
            cinematic: true,
            project: cinematic("space"),
        },
    ]
}

// ---------------------------------------------------------------------------
// Layer helpers
// ---------------------------------------------------------------------------

fn beat_scenes(p: &MotionProject) -> Vec<&Scene> {
    p.scenes.iter().filter(|s| s.lifecycle.is_some()).collect()
}

fn prefix(s: &Scene) -> String {
    s.id.replace("beat_", "b")
}

fn holds(l: &Layer, id: &str) -> bool {
    l.id == id
        || matches!(&l.kind, LayerKind::Group { children } if children.iter().any(|c| holds(c, id)))
}

fn find_layer<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            if let Some(found) = find_layer(children, id) {
                return Some(found);
            }
        }
    }
    None
}

fn find_layer_mut<'a>(layers: &'a mut [Layer], id: &str) -> Option<&'a mut Layer> {
    for l in layers.iter_mut() {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &mut l.kind {
            if let Some(found) = find_layer_mut(children, id) {
                return Some(found);
            }
        }
    }
    None
}

/// The top-level layer of `s` that holds `id` (the perspective camera places
/// top-level planes only).
fn top_of<'a>(s: &'a Scene, id: &str) -> Option<&'a Layer> {
    s.layers.iter().find(|l| holds(l, id))
}

fn top_of_mut<'a>(s: &'a mut Scene, id: &str) -> Option<&'a mut Layer> {
    s.layers.iter_mut().find(|l| holds(l, id))
}

fn focal_of<'a>(p: &'a MotionProject, s: &Scene) -> &'a str {
    p.project
        .art
        .as_ref()
        .and_then(|a| a.focal.get(&s.id))
        .unwrap_or_else(|| panic!("{}: no focal layer recorded", s.id))
}

fn scene_mut<'a>(p: &'a mut MotionProject, id: &str) -> &'a mut Scene {
    p.scenes
        .iter_mut()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene {id}"))
}

fn camera(s: &Scene) -> &Camera {
    s.camera
        .as_ref()
        .unwrap_or_else(|| panic!("{}: no camera", s.id))
}

// ---------------------------------------------------------------------------
// Resolved-frame helpers
// ---------------------------------------------------------------------------

fn find_resolved<'a, 'b>(l: &'b ResolvedLayer<'a>, id: &str) -> Option<&'b ResolvedLayer<'a>> {
    if l.id == id {
        return Some(l);
    }
    l.children.iter().find_map(|c| find_resolved(c, id))
}

/// `(top-level layer, node)` of the resolved layer `id`.
fn resolved_with_top<'a, 'b>(
    layers: &'b [ResolvedLayer<'a>],
    id: &str,
) -> Option<(&'b ResolvedLayer<'a>, &'b ResolvedLayer<'a>)> {
    layers
        .iter()
        .find_map(|t| find_resolved(t, id).map(|n| (t, n)))
}

/// Axis-aligned box `[x0, y0, x1, y1]` of a resolved layer's box transform.
fn bbox(l: &ResolvedLayer) -> [f32; 4] {
    let t = l.transform;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (cx, cy) in [
        (0.0, 0.0),
        (l.width, 0.0),
        (0.0, l.height),
        (l.width, l.height),
    ] {
        let x = t.a * cx + t.c * cy + t.e;
        let y = t.b * cx + t.d * cy + t.f;
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    [x0, y0, x1, y1]
}

/// Overlap area of two `[x0, y0, x1, y1]` boxes (0 when they are apart).
fn overlap(a: [f32; 4], b: [f32; 4]) -> f32 {
    let w = a[2].min(b[2]) - a[0].max(b[0]);
    let h = a[3].min(b[3]) - a[1].max(b[1]);
    if w > 0.0 && h > 0.0 {
        w * h
    } else {
        0.0
    }
}

fn area(b: [f32; 4]) -> f32 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

fn frame_number(p: &MotionProject, s: &Scene, local: f64) -> u32 {
    let last = p.frame_count().saturating_sub(1);
    (((s.start_seconds + local) * p.canvas.fps as f64).round() as u32).min(last)
}

/// The times layout QA samples a beat at (scene-local): READ, the READ/EVOLVE
/// midpoint and the end of EVOLVE, when its arrivals have settled.
fn qa_times(s: &Scene) -> Vec<f64> {
    let life = s.lifecycle.expect("lifecycle");
    let mut times = vec![life.read];
    for t in [
        (life.read + life.evolve) / 2.0,
        (life.anticipate - 0.05).max(life.read),
    ] {
        if times.iter().all(|x| (x - t).abs() > 1e-9) {
            times.push(t);
        }
    }
    times
}

fn report(p: &MotionProject) -> LayoutReport {
    let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
    layout_report(p, &frame)
}

/// Layers (by scene and id) that the report flags with `check`.
fn flagged(r: &LayoutReport, check: LayoutCheck) -> Vec<(&str, &str)> {
    r.findings
        .iter()
        .filter(|f| f.check == check)
        .map(|f| (f.scene.as_str(), f.layer.as_str()))
        .collect()
}

fn focal_findings(r: &LayoutReport) -> Vec<(&str, &str, &'static str)> {
    r.findings
        .iter()
        .filter(|f| {
            matches!(
                f.check,
                LayoutCheck::FocalOutOfFocus | LayoutCheck::EmphasisOffFocal
            )
        })
        .map(|f| (f.scene.as_str(), f.layer.as_str(), f.check.name()))
        .collect()
}

// ---------------------------------------------------------------------------
// 1. The focal record
// ---------------------------------------------------------------------------

#[test]
fn every_beat_records_a_focal_layer_that_exists() {
    for case in cases() {
        let p = &case.project;
        validate(p, None).unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let art = p.project.art.as_ref().expect("art record");
        let beats = beat_scenes(p);
        assert!(!beats.is_empty(), "{}", case.name);
        for s in &beats {
            let focal = art
                .focal
                .get(&s.id)
                .unwrap_or_else(|| panic!("{} {}: no focal entry", case.name, s.id));
            assert!(
                find_layer(&s.layers, focal).is_some(),
                "{} {}: focal layer {focal} does not exist",
                case.name,
                s.id
            );
            assert!(
                focal.starts_with(&format!("{}.", prefix(s))),
                "{} {}: focal {focal} belongs to another beat",
                case.name,
                s.id
            );
            // The camera and the QA look a plane up by its top-level holder.
            assert!(top_of(s, focal).is_some(), "{} {}", case.name, s.id);
        }
        // No stray records: one per beat scene and nothing else.
        assert_eq!(
            art.focal.len(),
            beats.len(),
            "{}: {:?}",
            case.name,
            art.focal
        );
        for scene_id in art.focal.keys() {
            assert!(
                beats.iter().any(|s| &s.id == scene_id),
                "{}: focal recorded for non-beat scene {scene_id}",
                case.name
            );
        }
    }
}

#[test]
fn the_focal_layer_is_the_figure_card_in_the_dossier_and_the_hero_in_cinematic() {
    let focal_names = |p: &MotionProject| -> Vec<String> {
        beat_scenes(p)
            .iter()
            .map(|s| {
                let f = focal_of(p, s);
                f.strip_prefix(&format!("{}.", prefix(s)))
                    .unwrap_or(f)
                    .to_string()
            })
            .collect()
    };
    // A figure wins; else the picture; else the clipping (dossier.rs).
    assert_eq!(
        focal_names(&documentary(DOCUMENTARY, None)),
        ["figure", "figure", "figure"]
    );
    assert_eq!(
        focal_names(&documentary(DOCUMENTARY_PICTURE_AND_CLIPPING, None)),
        ["photo", "clip", "clip"]
    );
    for name in ["ai_age", "space"] {
        for f in focal_names(&cinematic(name)) {
            assert!(f == "hero" || f == "hero_word", "{name}: focal {f}");
        }
    }
    // The art record says which look was compiled.
    let look = |p: &MotionProject| p.project.art.as_ref().map(|a| a.look.clone());
    assert_eq!(
        look(&documentary(DOCUMENTARY, None)).as_deref(),
        Some("dossier")
    );
    assert_eq!(look(&cinematic("ai_age")).as_deref(), Some("cinematic_3d"));
}

// ---------------------------------------------------------------------------
// 2. The focal layer is sharp
// ---------------------------------------------------------------------------

/// The focal layer of `s` is drawn and its top-level plane is within
/// `FOCAL_BLUR_MAX` at each of the times layout QA samples.
fn assert_focal_is_sharp(case: &Case, s: &Scene) {
    let p = &case.project;
    let focal = focal_of(p, s);
    for local in qa_times(s) {
        let f = evaluate_frame(p, frame_number(p, s, local)).expect("frame");
        let (top, node) = resolved_with_top(&f.layers, focal).unwrap_or_else(|| {
            panic!(
                "{} {}: focal {focal} is not drawn at {local:.2}s",
                case.name, s.id
            )
        });
        let blur = top.blur.unwrap_or(0.0);
        assert!(
            blur <= FOCAL_BLUR_MAX,
            "{} {} at {local:.2}s: focal {focal} blurred {blur} px (max {FOCAL_BLUR_MAX})",
            case.name,
            s.id
        );
        // Drawn, not just present: a focal layer that never shows cannot be
        // read.
        assert!(
            node.width > 0.0 && node.height > 0.0,
            "{} {} at {local:.2}s",
            case.name,
            s.id
        );
    }
}

#[test]
fn the_focal_layer_stays_sharp_at_read_mid_evolve_and_settled() {
    let mut checked = 0;
    for case in cases() {
        for s in beat_scenes(&case.project) {
            assert_focal_is_sharp(&case, s);
            checked += 1;
        }
    }
    // Three + three documentary beats, four + four cinematic ones.
    assert_eq!(checked, 14);
}

/// Every frame from the end of the fly-in to the end of the beat, not only
/// the QA samples: the orbit (which turns about the camera pivot) must not
/// swing the focal plane through the depth of field, whatever its anchor is
/// (the cinematic text hero is anchored at its top-left corner, the pictures
/// at their centre).
#[test]
fn the_focal_layer_is_sharp_on_every_frame_after_the_fly_in() {
    for case in cases() {
        let p = &case.project;
        let fps = f64::from(p.canvas.fps);
        for s in beat_scenes(p) {
            let focal = focal_of(p, s);
            let acts = channel(camera(s), is_dolly);
            let entry = acts[0].start + acts[0].duration;
            let from = ((s.start_seconds + entry) * fps).ceil() as u32;
            let to = ((s.start_seconds + s.duration_seconds) * fps).floor() as u32;
            for frame in from..to {
                let f = evaluate_frame(p, frame).expect("frame");
                // The beat's own layers; the next beat's cut-in overlaps the
                // fly-through and has its own focal layer.
                let (top, _) = resolved_with_top(&f.layers, focal).unwrap_or_else(|| {
                    panic!("{} {}: focal not drawn at {frame}", case.name, s.id)
                });
                let blur = top.blur.unwrap_or(0.0);
                assert!(
                    blur <= FOCAL_BLUR_MAX,
                    "{} {} frame {frame}: focal {focal} blurred {blur} px",
                    case.name,
                    s.id
                );
            }
        }
    }
}

/// Depth of field is on: the planes the camera is not about are blurred well
/// past the focal limit, so the sharpness above is the focus at work and not
/// a flat projection.
#[test]
fn depth_of_field_blurs_what_the_beat_is_not_about() {
    for case in cases().into_iter().filter(|c| c.cinematic) {
        let p = &case.project;
        for s in beat_scenes(p) {
            let focal = focal_of(p, s);
            let local = s.lifecycle.expect("lifecycle").read;
            let f = evaluate_frame(p, frame_number(p, s, local)).expect("frame");
            let (focal_top, _) = resolved_with_top(&f.layers, focal).expect("focal");
            let most = f
                .layers
                .iter()
                .filter(|l| l.scene == Some(s.id.as_str()) && l.id != focal_top.id)
                .filter_map(|l| l.blur)
                .fold(0.0f32, f32::max);
            assert!(
                most > FOCAL_BLUR_MAX,
                "{} {}: no background plane is blurred (max {most})",
                case.name,
                s.id
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. The stamp lands on the focal card
// ---------------------------------------------------------------------------

const CANVASES: [(u32, u32); 4] = [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)];

/// Scene-local time at which the stamp's own motions (and its children's)
/// have all finished.
fn stamp_landed(s: &Scene, stamp_id: &str) -> f64 {
    let child = format!("{stamp_id}.");
    s.motions
        .iter()
        .filter(|m| m.target == stamp_id || m.target.starts_with(&child))
        .map(|m| m.start + m.duration)
        .fold(0.0, f64::max)
}

#[test]
fn the_stamp_lands_on_the_focal_card_on_every_canvas() {
    let mut stamps_checked = 0;
    for canvas in CANVASES {
        for intent in [DOCUMENTARY, DOCUMENTARY_PICTURE_AND_CLIPPING] {
            let p = documentary(intent, Some(canvas));
            assert_eq!((p.canvas.width, p.canvas.height), canvas);
            validate(&p, None).expect("validate");
            for s in beat_scenes(&p) {
                // Every beat of both fixtures has a keyword, so a stamp.
                let stamp_id = format!("{}.stamp", prefix(s));
                assert!(
                    find_layer(&s.layers, &stamp_id).is_some(),
                    "{canvas:?} {}: no stamp",
                    s.id
                );
                let focal = focal_of(&p, s);
                let life = s.lifecycle.expect("lifecycle");
                let landed = stamp_landed(s, &stamp_id);
                // Layout QA's last sample is at ANTICIPATE - 0.05 s; a stamp
                // on a short EVOLVE span is still mid-slam (and oversized)
                // there, so the landed state is sampled on its own.
                let settled = life.anticipate - 0.05;
                let after = landed + 0.05;
                assert!(
                    after < s.duration_seconds,
                    "{canvas:?} {}: the stamp lands at {landed:.2}s of {:.2}s",
                    s.id,
                    s.duration_seconds
                );
                for (local, is_landed) in [(settled, false), (after, true)] {
                    let f = evaluate_frame(&p, frame_number(&p, s, local)).expect("frame");
                    let (_, stamp) = resolved_with_top(&f.layers, &stamp_id)
                        .unwrap_or_else(|| panic!("{canvas:?} {}: stamp not drawn", s.id));
                    let (_, card) = resolved_with_top(&f.layers, focal)
                        .unwrap_or_else(|| panic!("{canvas:?} {}: focal not drawn", s.id));
                    assert!(
                        stamp.opacity >= 0.5,
                        "{canvas:?} {} at {local:.2}s: the stamp is not showing (opacity {})",
                        s.id,
                        stamp.opacity
                    );
                    let (sb, cb) = (bbox(stamp), bbox(card));
                    let shared = overlap(sb, cb);
                    assert!(
                        shared > 0.0,
                        "{canvas:?} {} at {local:.2}s: stamp {sb:?} misses the focal card {focal} {cb:?}",
                        s.id
                    );
                    // Once landed it is on the card, not merely grazing its edge.
                    if is_landed {
                        assert!(
                            shared >= 0.1 * area(sb),
                            "{canvas:?} {} at {local:.2}s: only {:.0}% of the stamp is on the card",
                            s.id,
                            100.0 * shared / area(sb)
                        );
                    }
                }
                stamps_checked += 1;
            }
            // The QA agrees on the whole compile: neither focal check fires.
            let r = report(&p);
            assert!(focal_findings(&r).is_empty(), "{canvas:?}\n{}", r.to_text());
        }
    }
    // Three beats in each of the two fixtures, on four canvases.
    assert_eq!(stamps_checked, CANVASES.len() * 6);
}

// ---------------------------------------------------------------------------
// 4. QA negative cases on hand-mutated projects
// ---------------------------------------------------------------------------

/// Push the focal layer's top-level plane far behind the focus plane.
fn push_focal_back(p: &mut MotionProject, scene_id: &str, by: f32) {
    let focal = p
        .project
        .art
        .as_ref()
        .and_then(|a| a.focal.get(scene_id))
        .expect("focal entry")
        .clone();
    let top = top_of_mut(scene_mut(p, scene_id), &focal).expect("focal's top-level layer");
    top.z = Some(top.z.unwrap_or(0.0) + by);
}

/// Offset the stamp of a beat by `(dx, dy)` canvas px.
fn move_stamp(p: &mut MotionProject, scene_id: &str, dx: f32, dy: f32) {
    let s = scene_mut(p, scene_id);
    let id = format!("{}.stamp", prefix(s));
    let stamp = find_layer_mut(&mut s.layers, &id).expect("stamp");
    assert!(
        stamp.layout.is_none(),
        "the stamp is placed by a layout binding: x/y would be ignored"
    );
    stamp.x += dx;
    stamp.y += dy;
}

#[test]
fn a_focal_layer_pushed_out_of_the_focus_plane_is_flagged() {
    let mut mutated = 0;
    for case in cases() {
        let base = &case.project;
        let r = report(base);
        assert!(
            flagged(&r, LayoutCheck::FocalOutOfFocus).is_empty(),
            "{}: baseline\n{}",
            case.name,
            r.to_text()
        );
        for s in beat_scenes(base) {
            let focal = focal_of(base, s).to_string();
            let mut p = base.clone();
            push_focal_back(&mut p, &s.id, 2000.0);
            let r = report(&p);
            assert_eq!(
                flagged(&r, LayoutCheck::FocalOutOfFocus),
                [(s.id.as_str(), focal.as_str())],
                "{} {}: expected exactly this beat's focal layer\n{}",
                case.name,
                s.id,
                r.to_text()
            );
            // The finding carries the measured blur.
            let f = r
                .findings
                .iter()
                .find(|f| f.check == LayoutCheck::FocalOutOfFocus)
                .expect("finding");
            assert!(f.detail.contains("blurred"), "{}", f.detail);
            mutated += 1;
        }
    }
    assert_eq!(mutated, 14);
}

#[test]
fn a_focal_layer_a_few_px_off_the_focus_plane_is_not_flagged() {
    // The check has a tolerance (FOCAL_BLUR_MAX px of depth-of-field blur): a
    // few px of depth is well inside it.
    for case in cases() {
        let base = &case.project;
        for s in beat_scenes(base) {
            let mut p = base.clone();
            push_focal_back(&mut p, &s.id, 5.0);
            let r = report(&p);
            assert!(
                flagged(&r, LayoutCheck::FocalOutOfFocus).is_empty(),
                "{} {}\n{}",
                case.name,
                s.id,
                r.to_text()
            );
        }
    }
}

#[test]
fn a_stamp_moved_off_the_focal_card_is_flagged() {
    let mut moved = 0;
    for intent in [DOCUMENTARY, DOCUMENTARY_PICTURE_AND_CLIPPING] {
        let base = documentary(intent, None);
        let r = report(&base);
        assert!(
            flagged(&r, LayoutCheck::EmphasisOffFocal).is_empty(),
            "baseline\n{}",
            r.to_text()
        );
        let beats: Vec<String> = beat_scenes(&base).iter().map(|s| s.id.clone()).collect();
        assert_eq!(beats.len(), 3);
        for id in &beats {
            let stamp = format!("{}.stamp", id.replace("beat_", "b"));
            for (dx, dy) in [(1500.0, 1500.0), (-1500.0, 0.0), (0.0, 1500.0)] {
                let mut p = base.clone();
                move_stamp(&mut p, id, dx, dy);
                let r = report(&p);
                assert_eq!(
                    flagged(&r, LayoutCheck::EmphasisOffFocal),
                    [(id.as_str(), stamp.as_str())],
                    "{id} moved by ({dx}, {dy})\n{}",
                    r.to_text()
                );
                // Only the stamp moved: the focal layer is still sharp.
                assert!(flagged(&r, LayoutCheck::FocalOutOfFocus).is_empty());
                moved += 1;
            }
            // A nudge keeps it on the card.
            let mut p = base.clone();
            move_stamp(&mut p, id, 6.0, 6.0);
            let r = report(&p);
            assert!(
                flagged(&r, LayoutCheck::EmphasisOffFocal).is_empty(),
                "{id} nudged\n{}",
                r.to_text()
            );
        }
    }
    assert_eq!(moved, 18);
}

#[test]
fn without_a_focal_record_neither_focal_check_fires() {
    let mut p = documentary(DOCUMENTARY, None);
    let beats: Vec<String> = beat_scenes(&p).iter().map(|s| s.id.clone()).collect();
    // Break every beat both ways at once.
    for id in &beats {
        push_focal_back(&mut p, id, 2000.0);
        move_stamp(&mut p, id, 1500.0, 1500.0);
    }
    let r = report(&p);
    let hits = focal_findings(&r);
    assert_eq!(
        hits.iter().filter(|h| h.2 == "focal_out_of_focus").count(),
        beats.len(),
        "control: every beat is out of focus\n{}",
        r.to_text()
    );
    assert_eq!(
        hits.iter().filter(|h| h.2 == "emphasis_off_focal").count(),
        beats.len(),
        "control: every stamp is off its card\n{}",
        r.to_text()
    );

    // No focal entries recorded.
    let mut no_entries = p.clone();
    no_entries
        .project
        .art
        .as_mut()
        .expect("art record")
        .focal
        .clear();
    let r = report(&no_entries);
    assert!(focal_findings(&r).is_empty(), "{}", r.to_text());

    // No art record at all.
    let mut no_art = p.clone();
    no_art.project.art = None;
    let r = report(&no_art);
    assert!(focal_findings(&r).is_empty(), "{}", r.to_text());

    // A beat without an entry is not judged while the others still are.
    let mut one_missing = p.clone();
    one_missing
        .project
        .art
        .as_mut()
        .expect("art record")
        .focal
        .remove(&beats[0]);
    let r = report(&one_missing);
    let hits = focal_findings(&r);
    assert!(hits.iter().all(|h| h.0 != beats[0]), "{hits:?}");
    for id in &beats[1..] {
        assert!(
            hits.iter()
                .any(|h| h.0 == id && h.2 == "focal_out_of_focus"),
            "{id}: {hits:?}"
        );
    }
}

#[test]
fn a_cinematic_project_without_a_focal_record_is_not_judged() {
    let mut p = cinematic("ai_age");
    let beats: Vec<String> = beat_scenes(&p).iter().map(|s| s.id.clone()).collect();
    for id in &beats {
        push_focal_back(&mut p, id, 2000.0);
    }
    let r = report(&p);
    assert_eq!(
        flagged(&r, LayoutCheck::FocalOutOfFocus).len(),
        beats.len(),
        "control\n{}",
        r.to_text()
    );
    // A cinematic beat has no stamp: the emphasis check has nothing to judge.
    assert!(flagged(&r, LayoutCheck::EmphasisOffFocal).is_empty());

    let mut no_art = p.clone();
    no_art.project.art = None;
    let r = report(&no_art);
    assert!(focal_findings(&r).is_empty(), "{}", r.to_text());

    let mut no_entries = p.clone();
    no_entries
        .project
        .art
        .as_mut()
        .expect("art record")
        .focal
        .clear();
    let r = report(&no_entries);
    assert!(focal_findings(&r).is_empty(), "{}", r.to_text());
}

#[test]
fn an_invisible_stamp_is_not_judged() {
    // A stamp that is not showing (it has not slammed yet, as at READ) cannot
    // be off its card: displace it, keep it at zero opacity throughout and
    // the finding goes away.
    let mut p = documentary(DOCUMENTARY, None);
    let id = beat_scenes(&p)[0].id.clone();
    let stamp_id = format!("{}.stamp", id.replace("beat_", "b"));
    move_stamp(&mut p, &id, 1500.0, 1500.0);
    let shown = report(&p);
    assert_eq!(
        flagged(&shown, LayoutCheck::EmphasisOffFocal).len(),
        1,
        "control\n{}",
        shown.to_text()
    );
    let s = scene_mut(&mut p, &id);
    find_layer_mut(&mut s.layers, &stamp_id)
        .expect("stamp")
        .opacity = 0.0;
    s.motions
        .retain(|m| m.target != stamp_id || !matches!(m.op, MotionOp::Fade { .. }));
    let r = report(&p);
    assert!(
        flagged(&r, LayoutCheck::EmphasisOffFocal).is_empty(),
        "{}",
        r.to_text()
    );
}

// ---------------------------------------------------------------------------
// 5. Camera: sampling helpers and continuity
// ---------------------------------------------------------------------------

/// Cubic Hermite from 0 to 1 with normalised end slopes (the spec of
/// `CameraMotion.velocity`).
fn hermite(u: f64, v: [f32; 2]) -> f64 {
    let u = u.clamp(0.0, 1.0);
    let (v0, v1) = (f64::from(v[0]), f64::from(v[1]));
    (u.powi(3) - 2.0 * u.powi(2) + u) * v0
        + (3.0 * u.powi(2) - 2.0 * u.powi(3))
        + (u.powi(3) - u.powi(2)) * v1
}

fn progress(m: &CameraMotion, t: f64) -> f64 {
    let raw = (t - m.start) / m.duration;
    match m.velocity {
        Some(v) => hermite(raw, v),
        None => m.easing.apply(raw),
    }
}

/// The motions of one channel in start order.
fn channel(cam: &Camera, pick: impl Fn(&CameraOp) -> bool) -> Vec<&CameraMotion> {
    let mut list: Vec<&CameraMotion> = cam.motions.iter().filter(|m| pick(&m.op)).collect();
    list.sort_by(|a, b| a.start.total_cmp(&b.start));
    list
}

fn is_dolly(op: &CameraOp) -> bool {
    matches!(op, CameraOp::Dolly { .. })
}

fn is_focus(op: &CameraOp) -> bool {
    matches!(op, CameraOp::Focus { .. })
}

/// Value of a `from -> to` channel at scene-local `t`: the latest motion that
/// has started holds (its end value after it ends); before the first starts,
/// the first motion's `from`.
fn sample(list: &[&CameraMotion], t: f64, ends: impl Fn(&CameraOp) -> (f32, f32)) -> Option<f64> {
    let current = list.iter().rev().find(|m| m.start <= t).or(list.first())?;
    let (from, to) = ends(&current.op);
    let p = if current.start <= t {
        progress(current, t)
    } else {
        0.0
    };
    Some(f64::from(from) + f64::from(to - from) * p)
}

fn dolly_ends(op: &CameraOp) -> (f32, f32) {
    match op {
        CameraOp::Dolly { from, to } => (*from, *to),
        _ => unreachable!("not a dolly"),
    }
}

fn focus_ends(op: &CameraOp) -> (f32, f32) {
    match op {
        CameraOp::Focus { from, to } => (*from, *to),
        _ => unreachable!("not a focus"),
    }
}

fn dolly_at(cam: &Camera, t: f64) -> f64 {
    sample(&channel(cam, is_dolly), t, dolly_ends).expect("dolly")
}

fn focus_at(cam: &Camera, t: f64) -> f64 {
    sample(&channel(cam, is_focus), t, focus_ends).expect("focus")
}

#[test]
fn the_hermite_helper_matches_its_definition() {
    for v in [[0.0f32, 1.0], [1.0, 1.0], [2.6, 0.06], [0.3, 2.8]] {
        assert!(hermite(0.0, v).abs() < 1e-12);
        assert!((hermite(1.0, v) - 1.0).abs() < 1e-12);
        // End slopes are the requested ones.
        let h = 1e-6;
        let d0 = (hermite(h, v) - hermite(0.0, v)) / h;
        let d1 = (hermite(1.0, v) - hermite(1.0 - h, v)) / h;
        assert!((d0 - f64::from(v[0])).abs() < 1e-3, "{v:?}: {d0}");
        assert!((d1 - f64::from(v[1])).abs() < 1e-3, "{v:?}: {d1}");
    }
    // Slopes [1, 1] are linear speed.
    for i in 0..=10 {
        let u = f64::from(i) / 10.0;
        assert!((hermite(u, [1.0, 1.0]) - u).abs() < 1e-12);
    }
}

/// A plane at depth `z`, centred on the pivot of the scene's camera (the
/// canvas centre when it has none) and without any motion, added to a copy of
/// the project. The camera's orbit turns about that centre, so the plane only
/// sees the dolly (magnified `f / (z + f - d)`) and the focus (blur). Returns
/// the copy and the probe's layer id.
fn with_probe(p: &MotionProject, scene_id: &str, z: f32) -> (MotionProject, String) {
    let mut p = p.clone();
    let [cx, cy] = scene_mut(&mut p, scene_id)
        .camera
        .as_ref()
        .and_then(|c| c.pivot)
        .unwrap_or([p.canvas.width as f32 / 2.0, p.canvas.height as f32 / 2.0]);
    let id = format!("probe.{scene_id}");
    let probe: Layer = serde_json::from_value(json!({
        "id": id, "type": "rectangle", "x": cx, "y": cy, "width": 100.0, "height": 100.0,
        "anchor_x": 0.5, "anchor_y": 0.5, "fill": "#FFFFFF", "z": z, "z_index": 1000,
    }))
    .expect("probe layer");
    scene_mut(&mut p, scene_id).layers.push(probe);
    (p, id)
}

/// The dolly the timeline applies, read back from the probe's resolved scale.
fn dolly_from_probe(p: &MotionProject, scene: &Scene, probe: &str, frame: u32) -> f64 {
    let fov = camera(scene).perspective.expect("perspective").fov_deg;
    let f = f64::from(p.canvas.height) * 0.5 / (f64::from(fov).to_radians() * 0.5).tan();
    let fr = evaluate_frame(p, frame).expect("frame");
    let l = fr
        .layers
        .iter()
        .find(|l| l.id == probe)
        .unwrap_or_else(|| panic!("{}: probe not drawn at frame {frame}", scene.id));
    let t = l.transform;
    let scale = f64::from(t.a).hypot(f64::from(t.b));
    f - f / scale
}

#[test]
fn the_camera_helpers_agree_with_the_timeline() {
    for case in cases() {
        for s in beat_scenes(&case.project) {
            let (p, probe) = with_probe(&case.project, &s.id, 0.0);
            let cam = camera(s);
            let first = (s.start_seconds * p.canvas.fps as f64).ceil() as u32;
            let end = ((s.start_seconds + s.duration_seconds) * p.canvas.fps as f64).floor() as u32;
            let mut checked = 0;
            for frame in (first..end).step_by(3) {
                let t = frame as f64 / p.canvas.fps as f64 - s.start_seconds;
                let want = dolly_at(cam, t);
                let got = dolly_from_probe(&p, s, &probe, frame);
                assert!(
                    (got - want).abs() < 0.5,
                    "{} {} at {t:.3}s: timeline dolly {got:.2}, helper {want:.2}",
                    case.name,
                    s.id
                );
                checked += 1;
            }
            assert!(checked >= 8, "{} {}: {checked} samples", case.name, s.id);
        }
    }
}

/// Number of dolly acts of a beat: the fly-in, the main push and (except on
/// the last beat) the fly-through.
fn expected_acts(p: &MotionProject, s: &Scene) -> usize {
    let beats = beat_scenes(p);
    if beats.last().map(|l| &l.id) == Some(&s.id) {
        2
    } else {
        3
    }
}

/// One hand-over between consecutive dolly acts: the boundary time and the
/// dolly speed (px/s) just before and just after it, by one-sided numerical
/// derivative of the sampled dolly.
struct Handover {
    at: f64,
    before: f64,
    after: f64,
}

impl Handover {
    /// The camera neither stops nor jumps in speed: the two speeds differ by
    /// less than 15 % of the larger, which is moving.
    fn is_continuous(&self) -> bool {
        let faster = self.before.max(self.after);
        faster > 1.0 && (self.before - self.after).abs() < 0.15 * faster
    }
}

fn dolly_handovers(cam: &Camera) -> Vec<Handover> {
    let h = 1e-4;
    let d = |t: f64| dolly_at(cam, t);
    channel(cam, is_dolly)
        .windows(2)
        .map(|pair| {
            let at = pair[1].start;
            Handover {
                at,
                before: (d(at) - d(at - h)) / h,
                after: (d(at + h) - d(at)) / h,
            }
        })
        .collect()
}

#[test]
fn the_dolly_hands_its_speed_over_at_the_act_boundaries() {
    let mut boundaries = 0;
    for case in cases() {
        let p = &case.project;
        for s in beat_scenes(p) {
            let cam = camera(s);
            let acts = channel(cam, is_dolly);
            assert_eq!(
                acts.len(),
                expected_acts(p, s),
                "{} {}: dolly acts",
                case.name,
                s.id
            );
            for pair in acts.windows(2) {
                // Chained: one act ends where the next begins, at the same
                // dolly.
                assert!(
                    (pair[0].start + pair[0].duration - pair[1].start).abs() < 1e-6,
                    "{} {}: acts are not chained",
                    case.name,
                    s.id
                );
                let (_, a_to) = dolly_ends(&pair[0].op);
                let (b_from, _) = dolly_ends(&pair[1].op);
                assert!((a_to - b_from).abs() < 1e-3, "{} {}", case.name, s.id);
            }
            let handovers = dolly_handovers(cam);
            assert_eq!(handovers.len(), acts.len() - 1);
            for hv in handovers {
                assert!(
                    hv.is_continuous(),
                    "{} {}: dolly speed at {:.2}s goes {:.1} -> {:.1} px/s",
                    case.name,
                    s.id,
                    hv.at,
                    hv.before,
                    hv.after
                );
                boundaries += 1;
            }
        }
    }
    // Per look: entry + exit on every beat but the last, entry on the last.
    assert!(boundaries >= 20, "only {boundaries} boundaries checked");
}

/// The continuity test is sensitive: the same camera with the 0.17 easing
/// (ease-in-out: stops at each end) or plain linear acts (jumps from the
/// fly-in's speed to the push's) fails it.
#[test]
fn the_continuity_check_catches_a_stopping_or_jumping_camera() {
    let case = cases()
        .into_iter()
        .find(|c| c.name == "cinematic ai_age")
        .expect("case");
    let s = beat_scenes(&case.project)[0];
    let cam = camera(s);
    assert!(dolly_handovers(cam).iter().all(Handover::is_continuous));
    for easing in [Easing::InOutCubic, Easing::Linear] {
        let mut cam = cam.clone();
        for m in cam.motions.iter_mut().filter(|m| is_dolly(&m.op)) {
            m.velocity = None;
            m.easing = easing;
        }
        let handovers = dolly_handovers(&cam);
        assert_eq!(handovers.len(), 2);
        assert!(
            handovers.iter().all(|hv| !hv.is_continuous()),
            "{easing:?} was judged continuous"
        );
    }
}

#[test]
fn the_dolly_never_moves_backwards_after_the_fly_in() {
    for case in cases() {
        let p = &case.project;
        for s in beat_scenes(p) {
            let cam = camera(s);
            let acts = channel(cam, is_dolly);
            let entry = acts[0].start + acts[0].duration;
            let steps = (((s.duration_seconds - entry) * 240.0).floor() as usize).max(1);
            let mut prev = dolly_at(cam, entry);
            for i in 1..=steps {
                let t = entry + (s.duration_seconds - entry) * i as f64 / steps as f64;
                let now = dolly_at(cam, t);
                assert!(
                    now >= prev - 1e-9,
                    "{} {}: dolly goes back at {t:.3}s ({prev:.3} -> {now:.3})",
                    case.name,
                    s.id
                );
                prev = now;
            }
            // And it really advances: toward the scene by the end.
            assert!(
                dolly_at(cam, s.duration_seconds) > dolly_at(cam, entry) + 50.0,
                "{} {}",
                case.name,
                s.id
            );
        }
    }
}

#[test]
fn the_fly_in_travels_forward_without_overshoot() {
    for case in cases() {
        for s in beat_scenes(&case.project) {
            let cam = camera(s);
            let acts = channel(cam, is_dolly);
            let entry = acts[0].start + acts[0].duration;
            let (from, to) = dolly_ends(&acts[0].op);
            assert!(
                from < to,
                "{} {}: the fly-in comes from far back",
                case.name,
                s.id
            );
            let mut prev = f64::from(from);
            for i in 1..=100 {
                // From where the fly-in starts (ENTER) to where it ends.
                let t = acts[0].start + (entry - acts[0].start) * f64::from(i) / 100.0;
                let now = dolly_at(cam, t);
                assert!(
                    now >= prev - 1e-9,
                    "{} {}: fly-in reverses",
                    case.name,
                    s.id
                );
                assert!(
                    now <= f64::from(to) + 1e-6,
                    "{} {}: overshoots",
                    case.name,
                    s.id
                );
                prev = now;
            }
        }
    }
}

/// The fly-in starts when the content enters (the lifecycle's ENTER, at most
/// 30 % into the beat), not at the scene start, which overlaps the previous
/// beat's fly-through; until then the camera holds its start.
#[test]
fn the_fly_in_starts_at_enter_and_the_camera_waits_for_it() {
    for case in cases() {
        for s in beat_scenes(&case.project) {
            let cam = camera(s);
            let life = s.lifecycle.expect("lifecycle");
            let dur = s.duration_seconds;
            let t_in = life.enter.clamp(0.0, 0.3 * dur);
            let acts = channel(cam, is_dolly);
            let racks = channel(cam, is_focus);
            assert!(t_in > 0.0, "{} {}: ENTER is at {t_in}", case.name, s.id);
            // The first act of each channel is the fly-in / the rack.
            for m in [acts[0], racks[0]] {
                assert!(
                    (m.start - t_in).abs() < 1e-6
                        && (m.duration - 0.75f64.min(0.25 * dur)).abs() < 1e-6,
                    "{} {}: fly-in {}..{} for ENTER {t_in}",
                    case.name,
                    s.id,
                    m.start,
                    m.start + m.duration
                );
            }
            // Waiting: dolly and focus hold their start values until then.
            let (from, _) = dolly_ends(&acts[0].op);
            let (focus_from, _) = focus_ends(&racks[0].op);
            for t in [0.0, 0.5 * t_in, t_in] {
                assert!((dolly_at(cam, t) - f64::from(from)).abs() < 1e-9);
                assert!((focus_at(cam, t) - f64::from(focus_from)).abs() < 1e-9);
            }
        }
    }
}

/// The camera carries a choreographed beat out (the fly-through): the
/// lifecycle's own exit moves and shrinks on the top-level layers are gone and
/// their exit fades wait for the last 0.22 s.
#[test]
fn the_camera_owns_the_exit_of_a_beat() {
    for case in cases() {
        for s in beat_scenes(&case.project) {
            let mut fades = 0;
            let life = s.lifecycle.expect("lifecycle");
            let tops: Vec<&str> = s.layers.iter().map(|l| l.id.as_str()).collect();
            for m in s
                .motions
                .iter()
                .filter(|m| tops.contains(&m.target.as_str()) && m.start >= life.anticipate - 1e-3)
            {
                match m.op {
                    MotionOp::Move { .. } | MotionOp::Scale { .. } => panic!(
                        "{} {}: exit {:?} on {} at {:.2}s",
                        case.name, s.id, m.op, m.target, m.start
                    ),
                    MotionOp::Fade { .. } => {
                        assert!(
                            (m.start + m.duration - s.duration_seconds).abs() < 1e-6
                                && m.duration <= 0.22 + 1e-9,
                            "{} {}: exit fade of {} runs {:.2}..{:.2} of {:.2}",
                            case.name,
                            s.id,
                            m.target,
                            m.start,
                            m.start + m.duration,
                            s.duration_seconds
                        );
                        fades += 1;
                    }
                    _ => {}
                }
            }
            // Something still fades out with the last frames of every beat
            // but the last, which has nothing to hand over to.
            let last = beat_scenes(&case.project).last().map(|l| &l.id) == Some(&s.id);
            assert!(last || fades > 0, "{} {}: no exit fade", case.name, s.id);
        }
    }
}

/// The orbit turns about what the beat is about: the camera pivot is the
/// focal layer's centre (rest layout, no camera applied).
#[test]
fn the_camera_pivots_on_the_focal_layer() {
    for case in cases() {
        let p = &case.project;
        for s in beat_scenes(p) {
            let pivot = camera(s)
                .pivot
                .unwrap_or_else(|| panic!("{} {}: no pivot", case.name, s.id));
            let focal = focal_of(p, s);
            let mut flat = p.clone();
            scene_mut(&mut flat, &s.id).camera = None;
            let life = s.lifecycle.expect("lifecycle");
            let f = evaluate_frame(&flat, frame_number(&flat, s, life.read)).expect("frame");
            let (_, node) = resolved_with_top(&f.layers, focal).expect("focal");
            let b = bbox(node);
            let centre = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
            // Within a few percent of the canvas: the pivot is on the layer,
            // not at the canvas centre.
            let tol = 0.03 * p.canvas.width.min(p.canvas.height) as f32;
            assert!(
                (pivot[0] - centre[0]).abs() < tol && (pivot[1] - centre[1]).abs() < tol,
                "{} {}: pivot {pivot:?}, focal centre {centre:?}",
                case.name,
                s.id
            );
        }
    }
}

#[test]
fn orbit_and_track_run_linearly_across_the_whole_beat() {
    let mut seen = 0;
    for case in cases() {
        for s in beat_scenes(&case.project) {
            let cam = camera(s);
            for m in cam
                .motions
                .iter()
                .filter(|m| matches!(m.op, CameraOp::Orbit { .. } | CameraOp::Track { .. }))
            {
                assert!(
                    m.start.abs() < 1e-9 && (m.duration - s.duration_seconds).abs() < 1e-6,
                    "{} {}: {:?} spans {}..{} of {}",
                    case.name,
                    s.id,
                    m.op,
                    m.start,
                    m.start + m.duration,
                    s.duration_seconds
                );
                // Linear speed: either no velocity under a linear easing, or
                // unit Hermite slopes.
                match m.velocity {
                    Some(v) => assert_eq!(v, [1.0, 1.0], "{} {}", case.name, s.id),
                    None => assert_eq!(m.easing, Easing::Linear),
                }
                for i in 0..=10 {
                    let u = f64::from(i) / 10.0;
                    assert!((progress(m, m.start + u * m.duration) - u).abs() < 1e-9);
                }
                seen += 1;
            }
        }
    }
    assert!(seen >= 8, "only {seen} orbit/track moves found");
}

// ---------------------------------------------------------------------------
// 6. Focus tracks the focal plane
// ---------------------------------------------------------------------------

/// Depth of the focal plane (the top-level layer holding the focal layer).
fn focal_plane_z(p: &MotionProject, s: &Scene) -> f64 {
    f64::from(
        top_of(s, focal_of(p, s))
            .and_then(|l| l.z)
            .expect("the focal plane has a depth"),
    )
}

/// Worst `|focus + dolly - z_f|` (px) at ten times from the end of the fly-in
/// to the end of the beat (both ends included). A plane is sharp when
/// `z - dolly == focus`, so this is how far the focal plane is from the
/// focal distance.
fn focus_error(cam: &Camera, z_f: f64, duration: f64) -> f64 {
    let acts = channel(cam, is_dolly);
    let entry = acts[0].start + acts[0].duration;
    (0..10)
        .map(|i| {
            let t = entry + (duration - entry) * f64::from(i) / 9.0;
            (focus_at(cam, t) + dolly_at(cam, t) - z_f).abs()
        })
        .fold(0.0, f64::max)
}

#[test]
fn focus_plus_dolly_equals_the_focal_plane_after_the_fly_in() {
    for case in cases() {
        let p = &case.project;
        for s in beat_scenes(p) {
            let cam = camera(s);
            let z_f = focal_plane_z(p, s);
            let acts = channel(cam, is_dolly);
            let entry = acts[0].start + acts[0].duration;
            // The rack focus is as long as the fly-in.
            let racks = channel(cam, is_focus);
            assert!(racks.len() >= 2, "{} {}", case.name, s.id);
            assert!(
                (racks[0].start + racks[0].duration - entry).abs() < 1e-6,
                "{} {}: the rack ends with the fly-in",
                case.name,
                s.id
            );
            let worst = focus_error(cam, z_f, s.duration_seconds);
            assert!(
                worst <= 1.0,
                "{} {}: focus + dolly is {worst:.2} px off the focal plane {z_f}",
                case.name,
                s.id
            );
            // The measure is sensitive: a focal plane 50 px elsewhere is off.
            assert!(focus_error(cam, z_f + 50.0, s.duration_seconds) > 40.0);
        }
    }
}

/// The same statement through the timeline: a probe plane on the focal plane
/// is sharp at every frame after the fly-in, and blurred at the first frame
/// (the rack starts on the background).
#[test]
fn a_plane_on_the_focal_depth_is_sharp_through_the_timeline_after_the_fly_in() {
    for case in cases() {
        let p = &case.project;
        for s in beat_scenes(p) {
            let z_f = focal_plane_z(p, s) as f32;
            let (probed, probe) = with_probe(p, &s.id, z_f);
            let acts = channel(camera(s), is_dolly);
            let entry = acts[0].start + acts[0].duration;
            let fps = f64::from(p.canvas.fps);
            let blur_at = |frame: u32| -> f32 {
                let f = evaluate_frame(&probed, frame).expect("frame");
                let l = f.layers.iter().find(|l| l.id == probe).unwrap_or_else(|| {
                    panic!("{} {}: probe not drawn at {frame}", case.name, s.id)
                });
                l.blur.unwrap_or(0.0)
            };
            let first = (s.start_seconds * fps).ceil() as u32;
            assert!(
                blur_at(first) > FOCAL_BLUR_MAX,
                "{} {}: the rack should start out of focus",
                case.name,
                s.id
            );
            // Frames from the end of the fly-in to the end of the beat (the
            // next beat's own camera takes over at the cut, not before).
            let from = ((s.start_seconds + entry) * fps).ceil() as u32;
            let to = ((s.start_seconds + s.duration_seconds) * fps).floor() as u32;
            let mut seen = 0;
            for frame in from..to {
                let blur = blur_at(frame);
                assert!(
                    blur <= FOCAL_BLUR_MAX,
                    "{} {} frame {frame}: a plane on the focal depth is blurred {blur} px",
                    case.name,
                    s.id
                );
                seen += 1;
            }
            assert!(seen >= 20, "{} {}: {seen} frames", case.name, s.id);
        }
    }
}

#[test]
fn the_rack_focus_starts_on_the_background_and_ends_on_the_focal_plane() {
    for case in cases() {
        let p = &case.project;
        for s in beat_scenes(p) {
            let cam = camera(s);
            let z_f = focal_plane_z(p, s);
            let acts = channel(cam, is_dolly);
            let entry = acts[0].start + acts[0].duration;
            let at = |t: f64| focus_at(cam, t) + dolly_at(cam, t);
            // The focal plane in view-space terms: sharp exactly at the end of
            // the fly-in, behind it at the start.
            assert!((at(entry) - z_f).abs() <= 1.0, "{} {}", case.name, s.id);
            assert!(
                at(0.0) - z_f > 100.0,
                "{} {}: the rack starts at {:.0} (focal {z_f})",
                case.name,
                s.id,
                at(0.0)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Billboard
// ---------------------------------------------------------------------------

#[test]
fn only_the_cinematic_look_billboards_its_sprites() {
    for case in cases() {
        for s in beat_scenes(&case.project) {
            let persp = camera(s)
                .perspective
                .unwrap_or_else(|| panic!("{} {}: no perspective camera", case.name, s.id));
            assert_eq!(
                persp.billboard, case.cinematic,
                "{} {}: billboard",
                case.name, s.id
            );
            // Choreographed beats sort by z_index, not by depth.
            assert!(!persp.depth_sort, "{} {}", case.name, s.id);
        }
    }
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn the_focal_record_and_the_camera_are_deterministic() {
    let once = |c: &Case| -> String {
        let cams: Vec<_> = beat_scenes(&c.project)
            .iter()
            .map(|s| serde_json::to_string(&s.camera).expect("json"))
            .collect();
        format!(
            "{:?}\n{}",
            c.project.project.art.as_ref().map(|a| &a.focal),
            cams.join("\n")
        )
    };
    let (a, b) = (cases(), cases());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(once(x), once(y), "{}", x.name);
    }
}
