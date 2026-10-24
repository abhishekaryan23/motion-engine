//! (0.24 M4) 16:9-native layouts: emphasize, contrast and the number reveal
//! put their content side by side on a landscape canvas instead of scaling the
//! 9:16 stack. Structural assertions only; no pixels.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::{CreativeIntent, Format};
use motion_core::layout_report;
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::style::StyleProfile;

const GUTTER: f32 = 24.0;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn compile(canvas: (u32, u32), format: Format, measure: &dyn TextMeasure) -> MotionProject {
    let mut intent =
        CreativeIntent::from_json(&read("examples/editorial_demo.intent.json")).expect("intent");
    intent.format = format;
    let style: StyleProfile =
        serde_json::from_str(&read("examples/editorial_demo.style.json")).expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        canvas: Some(canvas),
        ..CompileOptions::default()
    };
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

/// `(id, x, y, w, h)` in canvas pixels, groups resolved (scale and rotation
/// ignored: the resting boxes).
type Box4 = (String, f32, f32, f32, f32);

fn flatten(layers: &[Layer], ox: f32, oy: f32, out: &mut Vec<Box4>) {
    for l in layers {
        let (x, y) = (
            ox + l.x - l.anchor_x * l.width,
            oy + l.y - l.anchor_y * l.height,
        );
        out.push((l.id.clone(), x, y, l.width, l.height));
        if let LayerKind::Group { children } = &l.kind {
            flatten(children, x, y, out);
        }
    }
}

fn boxes(p: &MotionProject, scene: usize) -> Vec<Box4> {
    let mut out = Vec::new();
    flatten(&p.scenes[scene].layers, 0.0, 0.0, &mut out);
    out
}

/// Union of the boxes whose id contains `needle`.
fn union(b: &[Box4], needle: &str) -> (f32, f32, f32, f32) {
    let hit: Vec<&Box4> = b.iter().filter(|r| r.0.contains(needle)).collect();
    assert!(!hit.is_empty(), "no layer matching {needle}");
    let x0 = hit.iter().map(|r| r.1).fold(f32::MAX, f32::min);
    let y0 = hit.iter().map(|r| r.2).fold(f32::MAX, f32::min);
    let x1 = hit.iter().map(|r| r.1 + r.3).fold(f32::MIN, f32::max);
    let y1 = hit.iter().map(|r| r.2 + r.4).fold(f32::MIN, f32::max);
    (x0, y0, x1 - x0, y1 - y0)
}

/// `a` is left of `b` with a gutter and they share vertical extent.
fn side_by_side(a: (f32, f32, f32, f32), b: (f32, f32, f32, f32), what: &str) {
    assert!(
        b.0 - (a.0 + a.2) >= GUTTER,
        "{what}: gutter {} < {GUTTER} ({a:?} | {b:?})",
        b.0 - (a.0 + a.2)
    );
    let overlap = (a.1 + a.3).min(b.1 + b.3) - a.1.max(b.1);
    assert!(
        overlap > 0.0,
        "{what}: not vertically overlapping ({a:?} | {b:?})"
    );
}

#[test]
fn landscape_beats_are_side_by_side() {
    let p = compile((1920, 1080), Format::Landscape, &ApproxMeasure);
    // beat 1: emphasize, headline left of the hero card.
    let b = boxes(&p, 1);
    side_by_side(union(&b, ".head."), union(&b, "hero.card"), "emphasize");
    // beat 2: contrast, zone A left of zone B.
    let b = boxes(&p, 2);
    side_by_side(union(&b, "panel"), union(&b, "zone_b.card"), "contrast");
    // beat 3: number reveal, the number left of its label.
    let b = boxes(&p, 3);
    side_by_side(union(&b, "hero."), union(&b, "label."), "number_reveal");
}

#[test]
fn landscape_layers_inside_canvas_and_qa_clean() {
    let p = compile((1920, 1080), Format::Landscape, &ApproxMeasure);
    for (i, s) in p.scenes.iter().enumerate() {
        for (id, x, y, w, h) in boxes(&p, i) {
            // The ghost word and the backdrop bleed on purpose.
            if id.contains(".ghost") || id.contains("backdrop") || id.ends_with(".stage") {
                continue;
            }
            assert!(
                x >= -0.5 && y >= -0.5 && x + w <= 1920.5 && y + h <= 1080.5,
                "{}: {id} outside the canvas ({x},{y},{w},{h})",
                s.id
            );
        }
    }
    let frame = LayoutFrame::new(1920, 1080).expect("frame");
    let bad: Vec<String> = layout_report(&p, &frame)
        .findings
        .into_iter()
        .filter(|f| matches!(f.check.name(), "text_outside_safe" | "headline_too_big"))
        .map(|f| format!("{} {} {}: {}", f.scene, f.layer, f.check.name(), f.detail))
        .collect();
    assert!(bad.is_empty(), "layout QA: {bad:#?}");
}

#[test]
fn vertical_keeps_the_stacked_layout() {
    let p = compile((1080, 1920), Format::Vertical, &ApproxMeasure);
    let b = boxes(&p, 1);
    let head = union(&b, ".head.");
    let card = union(&b, "hero.card");
    // Stacked: the hero card is below the headline and centred.
    assert!(card.1 >= head.1 + head.3 - 1.0, "{head:?} {card:?}");
    assert!((card.0 + card.2 / 2.0 - 540.0).abs() < 1.0, "{card:?}");
}

/// Canvas-space bounding box of a resolved layer (corners through its transform).
fn rbox(l: &motion_core::timeline::ResolvedLayer<'_>) -> (f32, f32, f32, f32) {
    let c = [
        (0.0, 0.0),
        (l.width, 0.0),
        (0.0, l.height),
        (l.width, l.height),
    ]
    .map(|(x, y)| l.transform.apply(x, y));
    let x0 = c.iter().map(|p| p.0).fold(f32::MAX, f32::min);
    let y0 = c.iter().map(|p| p.1).fold(f32::MAX, f32::min);
    let x1 = c.iter().map(|p| p.0).fold(f32::MIN, f32::max);
    let y1 = c.iter().map(|p| p.1).fold(f32::MIN, f32::max);
    (x0, y0, x1, y1)
}

fn find<'a, 'b>(
    ls: &'b [motion_core::timeline::ResolvedLayer<'a>],
    id: &str,
) -> Option<&'b motion_core::timeline::ResolvedLayer<'a>> {
    for l in ls {
        if l.id == id {
            return Some(l);
        }
        if let Some(h) = find(&l.children, id) {
            return Some(h);
        }
    }
    None
}

/// The beat-1 hero number (shared, counts up) stays inside its card, clear of
/// the headline, on every frame of the beat.
#[test]
fn landscape_hero_number_fits_its_card_every_frame() {
    let p = compile((1920, 1080), Format::Landscape, &ApproxMeasure);
    let s = &p.scenes[1];
    let fps = p.canvas.fps as f64;
    let (f0, f1) = (
        (s.start_seconds * fps).ceil() as u32,
        ((s.start_seconds + s.duration_seconds) * fps).floor() as u32,
    );
    let bx = boxes(&p, 1);
    let card = union(&bx, "hero.card");
    let mut seen = 0;
    for f in f0..f1 {
        let fr = motion_core::timeline::evaluate_frame(&p, f).expect("frame");
        let Some(n) = find(&fr.layers, "shared.50_000") else {
            continue;
        };
        if n.opacity < 0.5 {
            continue;
        }
        let nb = rbox(n);
        // The card's resting box (it fades out before the beat ends).
        let cb = (card.0, card.1, card.0 + card.2, card.1 + card.3);
        const PAD: f32 = 16.0;
        assert!(
            nb.0 >= cb.0 + PAD && nb.2 <= cb.2 - PAD && nb.1 >= cb.1 + PAD && nb.3 <= cb.3 - PAD,
            "frame {f}: number {nb:?} not inside card {cb:?} with {PAD}px padding"
        );
        for h in [".head.", ".serif."] {
            let r = union(&bx, h);
            let hb = (r.0, r.1, r.0 + r.2, r.1 + r.3);
            let hit = nb.0 < hb.2 && hb.0 < nb.2 && nb.1 < hb.3 && hb.1 < nb.3;
            assert!(!hit, "frame {f}: number {nb:?} overlaps {h} {hb:?}");
        }
        seen += 1;
    }
    assert!(seen > 60, "number seen on only {seen} frames");
}
