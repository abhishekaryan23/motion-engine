//! (0.10 Q) Subject guardrails in the compiler: the contrast guard (sticker /
//! keyline) and frame shrink-wrap, through the public compile API with
//! hand-built manifest entries (no pixels), judged by the same layout QA the
//! CLI runs.

use std::path::PathBuf;

use motion_core::assets::{
    AssetAnalysis, AssetManifest, EdgeContact, ManifestEntry, NormBox, NormPoint, OccupancyGrid,
};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::layout_qa::LayoutCheck;
use motion_core::scene::{Color, Layer, LayerKind, MotionProject, Scene};
use motion_core::{layout_report_with, CreativeIntent, ImageIndex, StyleProfile};
use serde_json::json;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn style(rel: &str) -> StyleProfile {
    serde_json::from_str(&read(rel)).expect("style")
}

fn intent(purpose: &str, primary: serde_json::Value) -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2", "title": "t", "format": "vertical",
        "beats": [{
            "purpose": purpose,
            "statement": "A short headline here",
            "primary": primary,
            "energy": "calm",
        }],
    }))
    .expect("intent")
}

fn analysis(subject: NormBox, mean: Option<[u8; 3]>, bottom_edge: bool) -> AssetAnalysis {
    AssetAnalysis {
        subject_bounds: subject,
        edges: EdgeContact {
            bottom: bottom_edge,
            ..EdgeContact::default()
        },
        coverage: 0.5,
        occupancy: OccupancyGrid {
            cols: 16,
            rows: (0..16).map(|_| "f".repeat(16)).collect(),
        },
        safe_regions: Vec::new(),
        head_estimate: None,
        monochrome: false,
        mean_color: mean,
    }
}

fn full() -> NormBox {
    NormBox {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    }
}

/// A delivered entry for request `id`.
fn entry(
    id: &str,
    path: &str,
    (width, height): (u32, u32),
    alpha: bool,
    subject: NormBox,
    mean: Option<[u8; 3]>,
) -> ManifestEntry {
    ManifestEntry {
        id: id.into(),
        path: path.into(),
        width,
        height,
        alpha,
        subject_anchor: Some(NormPoint { x: 0.5, y: 0.5 }),
        face_anchor: id
            .ends_with("hero_subject")
            .then_some(NormPoint { x: 0.5, y: 0.2 }),
        analysis: Some(analysis(subject, mean, false)),
        ..ManifestEntry::default()
    }
}

fn manifest(entries: Vec<ManifestEntry>) -> AssetManifest {
    AssetManifest {
        assets: entries,
        ..AssetManifest::empty()
    }
}

fn compile(
    i: &CreativeIntent,
    s: &StyleProfile,
    m: &AssetManifest,
    variety: Option<u64>,
) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        variety,
        ..CompileOptions::default()
    };
    compile_with_options(i, s, None, &library, &ApproxMeasure, m, None, &opts).expect("compiles")
}

fn all_layers(scene: &Scene) -> Vec<&Layer> {
    fn walk<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
        for l in layers {
            out.push(l);
            if let LayerKind::Group { children } = &l.kind {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&scene.layers, &mut out);
    out
}

fn layer<'a>(p: &'a MotionProject, id: &str) -> &'a Layer {
    p.scenes
        .iter()
        .flat_map(all_layers)
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("layer {id}"))
}

fn treatment(l: &Layer) -> &motion_core::scene::ImageTreatment {
    match &l.kind {
        LayerKind::Image { treatment, .. } => treatment.as_ref().expect("treated image"),
        other => panic!("{} is not an image: {other:?}", l.id),
    }
}

fn person_beat() -> CreativeIntent {
    intent(
        "emphasize",
        json!({"kind": "phrase", "value": "night worker", "meaning": "a worker"}),
    )
}

fn person_manifest(mean: Option<[u8; 3]>) -> AssetManifest {
    manifest(vec![entry(
        "beat_1.hero_subject",
        "test_images/figure.png",
        (600, 900),
        true,
        NormBox {
            x: 0.1,
            y: 0.05,
            width: 0.8,
            height: 0.9,
        },
        mean,
    )])
}

// ---------------------------------------------------------------------------
// Contrast guard
// ---------------------------------------------------------------------------

#[test]
fn a_dark_cutout_on_a_dark_palette_gets_a_light_sticker() {
    let dark = style("examples/taste/dark_technical.style.json");
    let p = compile(
        &person_beat(),
        &dark,
        &person_manifest(Some([26, 24, 22])),
        None,
    );
    let paper = p.canvas.background;
    let t = treatment(layer(&p, "b1.subject"));
    let sticker = t
        .sticker
        .expect("a dark subject on a dark page needs a sticker");
    // Light outline: it contrasts with the dark subject (and the dark page).
    assert!(
        u32::from(sticker.color.r) + u32::from(sticker.color.g) + u32::from(sticker.color.b) > 450,
        "{sticker:?} on {paper:?}"
    );
    assert_eq!(sticker.width_px, 8.0, "8u at u = 1");
    assert!(
        t.edge.is_none() && t.shadow.is_none(),
        "the sticker replaces the edge"
    );
    // Layout QA agrees: nothing left to flag.
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let index = ImageIndex::from_manifest(&person_manifest(Some([26, 24, 22])));
    let found: Vec<_> = layout_report_with(&p, &frame, &index)
        .findings
        .into_iter()
        .filter(|f| f.check == LayoutCheck::AssetLowContrast)
        .collect();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn the_same_cutout_on_a_light_paper_needs_no_sticker() {
    let light = style("examples/taste/warm_editorial.style.json");
    let p = compile(
        &person_beat(),
        &light,
        &person_manifest(Some([26, 24, 22])),
        None,
    );
    assert!(treatment(layer(&p, "b1.subject")).sticker.is_none());
}

#[test]
fn a_light_cutout_on_light_paper_gets_an_ink_sticker() {
    // White clothing on cream paper vanishes; the sticker is the ink colour.
    let light = style("examples/taste/warm_editorial.style.json");
    let p = compile(
        &person_beat(),
        &light,
        &person_manifest(Some([236, 232, 224])),
        None,
    );
    let t = treatment(layer(&p, "b1.subject"));
    let sticker = t.sticker.expect("sticker");
    let ink: Color = Color::parse_hex(&p.theme.palette["ink"].to_hex()).unwrap();
    assert_eq!(sticker.color, ink);
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let index = ImageIndex::from_manifest(&person_manifest(Some([236, 232, 224])));
    assert!(layout_report_with(&p, &frame, &index)
        .findings
        .iter()
        .all(|f| f.check != LayoutCheck::AssetLowContrast));
}

#[test]
fn manifests_without_a_mean_colour_are_never_second_guessed() {
    // Pre-0.10 manifests carry no mean colour: the compiler decides nothing.
    let dark = style("examples/taste/dark_technical.style.json");
    let p = compile(&person_beat(), &dark, &person_manifest(None), None);
    assert!(treatment(layer(&p, "b1.subject")).sticker.is_none());
}

#[test]
fn the_sticker_survives_the_variety_layouts() {
    let dark = style("examples/taste/dark_technical.style.json");
    for seed in 1..=4 {
        let p = compile(
            &person_beat(),
            &dark,
            &person_manifest(Some([26, 24, 22])),
            Some(seed),
        );
        assert!(
            treatment(layer(&p, "b1.subject")).sticker.is_some(),
            "variety {seed}"
        );
    }
}

#[test]
fn a_carried_subject_travels_between_the_rotating_layouts() {
    // One image serves two consecutive beats (asset carry: a SharedElement).
    // Under variety each beat takes a different layout, so the image moves.
    let light = style("examples/taste/warm_editorial.style.json");
    let two = serde_json::from_value(json!({
        "version": "0.2", "title": "t", "format": "vertical",
        "beats": [
            {"purpose": "emphasize", "statement": "A short headline here",
             "primary": {"kind": "phrase", "value": "night worker", "meaning": "a worker"},
             "energy": "calm"},
            {"purpose": "emphasize", "statement": "Another short one",
             "primary": {"kind": "phrase", "value": "same worker", "meaning": "a worker"},
             "energy": "calm"},
        ],
    }))
    .expect("intent");
    let mut e = entry(
        "beat_1.hero_subject",
        "test_images/figure.png",
        (600, 900),
        true,
        NormBox {
            x: 0.1,
            y: 0.05,
            width: 0.8,
            height: 0.9,
        },
        Some([90, 80, 70]),
    );
    e.serves = vec!["beat_1.hero_subject".into(), "beat_2.hero_subject".into()];
    let m = manifest(vec![e]);
    let p = compile(&two, &light, &m, Some(1));
    let shared: Vec<_> = p
        .shared
        .iter()
        .filter(|s| matches!(s.layer.kind, LayerKind::Image { .. }))
        .collect();
    assert_eq!(shared.len(), 1, "one carried image");
    let at = |scene: &str| {
        let key = shared[0]
            .track
            .iter()
            .rev()
            .find(|k| k.scene == scene && k.state.x.is_some())
            .unwrap_or_else(|| panic!("a key in {scene}"));
        (key.state.x.unwrap_or(0.0), key.state.y.unwrap_or(0.0))
    };
    let (a, b) = (at("beat_1"), at("beat_2"));
    assert!(
        (a.0 - b.0).abs() > 20.0 || (a.1 - b.1).abs() > 20.0,
        "the layouts differ, so the carried image moves: {a:?} -> {b:?}"
    );
    // Both layouts keep type and subject apart: layout QA is clean.
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let index = ImageIndex::from_manifest(&m);
    let found: Vec<_> = layout_report_with(&p, &frame, &index)
        .findings
        .into_iter()
        .filter(|f| {
            matches!(
                f.check,
                LayoutCheck::TextOverSubject
                    | LayoutCheck::SubjectTooSmall
                    | LayoutCheck::FrameTooLoose
                    | LayoutCheck::AssetLowContrast
            )
        })
        .collect();
    assert!(found.is_empty(), "{found:?}");
}

// ---------------------------------------------------------------------------
// Frame shrink-wrap
// ---------------------------------------------------------------------------

/// Largest pad of `card` around `inner` as a fraction of `inner`'s size, both
/// given as top-left boxes `(x, y, w, h)`.
fn pad(card: (f32, f32, f32, f32), inner: (f32, f32, f32, f32)) -> f32 {
    [
        (inner.0 - card.0) / inner.2,
        ((card.0 + card.2) - (inner.0 + inner.2)) / inner.2,
        (inner.1 - card.1) / inner.3,
        ((card.1 + card.3) - (inner.1 + inner.3)) / inner.3,
    ]
    .into_iter()
    .fold(f32::MIN, f32::max)
}

fn bbox(l: &Layer) -> (f32, f32, f32, f32) {
    (
        l.x - l.anchor_x * l.width,
        l.y - l.anchor_y * l.height,
        l.width,
        l.height,
    )
}

fn hero_intent() -> CreativeIntent {
    intent(
        "emphasize",
        json!({"kind": "object", "asset": "photo_object"}),
    )
}

fn hero_manifest(size: (u32, u32)) -> AssetManifest {
    manifest(vec![entry(
        "beat_1.hero_object",
        "test_images/object.png",
        size,
        false,
        full(),
        None,
    )])
}

#[test]
fn a_card_behind_a_wide_opaque_hero_hugs_the_image() {
    let light = style("examples/taste/warm_editorial.style.json");
    // A 2:1 opaque photo: the legacy 1.12x disc would be 2.2x taller than it.
    let p = compile(&hero_intent(), &light, &hero_manifest((800, 400)), None);
    let card = bbox(layer(&p, "b1.plate"));
    let image = bbox(layer(&p, "b1.hero"));
    let loose = pad(card, image);
    assert!(
        loose <= 0.12 + 1e-3,
        "card {card:?} around image {image:?}: pad {loose:.3}"
    );
    // ... and QA passes it.
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let index = ImageIndex::from_manifest(&hero_manifest((800, 400)));
    let found: Vec<_> = layout_report_with(&p, &frame, &index)
        .findings
        .into_iter()
        .filter(|f| f.check == LayoutCheck::FrameTooLoose)
        .collect();
    assert!(found.is_empty(), "{found:?}");
}

#[test]
fn a_square_opaque_hero_keeps_the_round_card() {
    // The disc is within the pad budget around a square image: unchanged.
    let light = style("examples/taste/warm_editorial.style.json");
    let p = compile(&hero_intent(), &light, &hero_manifest((800, 800)), None);
    let card = layer(&p, "b1.plate");
    match &card.kind {
        LayerKind::RoundedRectangle { radius, .. } => {
            assert!(
                (radius - card.width / 2.0).abs() < 0.5,
                "a disc: r {radius}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(pad(bbox(card), bbox(layer(&p, "b1.hero"))) <= 0.12 + 1e-3);
}

#[test]
fn a_transparent_hero_cutout_still_has_no_backing() {
    // DECISION 69: a delivered alpha cutout is the object itself.
    let light = style("examples/taste/warm_editorial.style.json");
    let m = manifest(vec![entry(
        "beat_1.hero_object",
        "test_images/object.png",
        (800, 400),
        true,
        NormBox {
            x: 0.1,
            y: 0.1,
            width: 0.8,
            height: 0.8,
        },
        None,
    )]);
    let p = compile(&hero_intent(), &light, &m, None);
    let ids: Vec<&str> = p
        .scenes
        .iter()
        .flat_map(all_layers)
        .map(|l| l.id.as_str())
        .collect();
    assert!(
        !ids.contains(&"b1.plate"),
        "no card behind a cutout: {ids:?}"
    );
}

#[test]
fn evidence_sheets_are_the_documents_own_bounds() {
    let light = style("examples/taste/warm_editorial.style.json");
    let ev = intent(
        "reveal",
        json!({"kind": "object", "asset": "scanned_receipt", "meaning": "receipt"}),
    );
    // A page-like cutout whose content covers the middle 80 % of its box.
    let content = NormBox {
        x: 0.1,
        y: 0.1,
        width: 0.8,
        height: 0.8,
    };
    let mut e = entry(
        "beat_1.evidence_image",
        "test_images/document.png",
        (700, 500),
        true,
        content,
        None,
    );
    e.analysis.as_mut().unwrap().coverage = 0.8;
    let m = manifest(vec![e]);
    let p = compile(&ev, &light, &m, None);
    let doc = bbox(layer(&p, "b1.doc"));
    let sheet = bbox(layer(&p, "b1.sheet.1"));
    // The sheet covers the content (80 % of the image box), not the whole box.
    assert!(
        sheet.2 < doc.2 * 0.9 && sheet.3 < doc.3 * 0.9,
        "sheet {sheet:?} vs doc box {doc:?}"
    );
    let content_box = (
        doc.0 + 0.1 * doc.2,
        doc.1 + 0.1 * doc.3,
        0.8 * doc.2,
        0.8 * doc.3,
    );
    assert!(
        pad(sheet, content_box) <= 0.12,
        "{:.3}",
        pad(sheet, content_box)
    );
    let frame = LayoutFrame::new(1080, 1920).unwrap();
    let index = ImageIndex::from_manifest(&m);
    assert!(layout_report_with(&p, &frame, &index)
        .findings
        .iter()
        .all(|f| f.check != LayoutCheck::FrameTooLoose));
}

#[test]
fn opaque_evidence_keeps_its_sheets_the_size_of_the_image() {
    // Fully opaque documents: sheets are the image box (byte-compatible with 0.9).
    let light = style("examples/taste/warm_editorial.style.json");
    let ev = intent(
        "reveal",
        json!({"kind": "object", "asset": "scanned_receipt", "meaning": "receipt"}),
    );
    let m = manifest(vec![entry(
        "beat_1.evidence_image",
        "test_images/document.png",
        (700, 500),
        false,
        full(),
        None,
    )]);
    let p = compile(&ev, &light, &m, None);
    let doc = bbox(layer(&p, "b1.doc"));
    let sheet = bbox(layer(&p, "b1.sheet.1"));
    assert!((sheet.2 - doc.2).abs() < 0.5 && (sheet.3 - doc.3).abs() < 0.5);
}
