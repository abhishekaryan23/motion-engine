//! (0.10 Q) Layout QA for delivered subject images: `text_over_subject`,
//! `subject_too_small`, `frame_too_loose`, `asset_low_contrast`. Projects are
//! hand-built so each check is exercised on its own; the image facts are
//! supplied through an `ImageIndex` (the analysis the compiler and the CLI
//! normally provide).

use motion_core::assets::NormBox;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::layout_qa::LayoutCheck;
use motion_core::scene::MotionProject;
use motion_core::{layout_report, layout_report_with, ImageFacts, ImageIndex};
use serde_json::{json, Value};

const W: u32 = 1080;
const H: u32 = 1920;

fn base(id: &str, x: f32, y: f32, w: f32, h: f32, z: i32) -> Value {
    json!({
        "id": id, "x": x, "y": y, "width": w, "height": h,
        "scale_x": 1.0, "scale_y": 1.0, "rotation_degrees": 0.0,
        "anchor_x": 0.0, "anchor_y": 0.0, "opacity": 1.0,
        "z_index": z, "visible": true,
    })
}

fn with(mut layer: Value, extra: Value) -> Value {
    let (l, e) = (layer.as_object_mut().unwrap(), extra.as_object().unwrap());
    for (k, v) in e {
        l.insert(k.clone(), v.clone());
    }
    layer
}

fn image(id: &str, rect: (f32, f32, f32, f32), z: i32, treatment: Option<Value>) -> Value {
    let mut l = with(
        base(id, rect.0, rect.1, rect.2, rect.3, z),
        json!({"type": "image", "asset": "asset.fig", "fit": "contain"}),
    );
    if let Some(t) = treatment {
        l["treatment"] = t;
    }
    l
}

fn text(id: &str, rect: (f32, f32, f32, f32), z: i32) -> Value {
    with(
        base(id, rect.0, rect.1, rect.2, rect.3, z),
        json!({
            "type": "text", "text": "LABEL", "font_role": "display", "font_size": 60.0,
            "font_weight": 900, "italic": false, "color": "#171513", "align": "left",
            "line_height": 1.0, "letter_spacing": 0.0, "uppercase": false,
        }),
    )
}

fn rect(id: &str, r: (f32, f32, f32, f32), z: i32, fill: &str) -> Value {
    with(
        base(id, r.0, r.1, r.2, r.3, z),
        json!({"type": "rectangle", "fill": fill}),
    )
}

fn project(background: &str, layers: Vec<Value>) -> MotionProject {
    let fonts = json!({
        "display": "font.archivo_black", "display_condensed": "font.anton",
        "serif_emotional": "font.dm_serif_italic", "body": "font.fira_sans",
        "mono": "font.space_mono", "number": "font.anton",
    });
    let doc = json!({
        "version": "0.2",
        "project": {"name": "subject_qa", "duration_seconds": 3.0},
        "canvas": {"width": W, "height": H, "fps": 30, "background": background},
        "theme": {"fonts": fonts, "palette": {}},
        "assets": [{"id": "asset.fig", "type": "image", "path": "fig.png"}],
        "scenes": [{
            "id": "beat_1", "start_seconds": 0.0, "duration_seconds": 3.0,
            "layers": layers, "motions": [],
            "lifecycle": {"enter": 0.2, "settle": 0.5, "read": 1.0, "evolve": 1.5,
                          "anticipate": 2.5, "bridge": 3.0},
        }],
    });
    serde_json::from_value(doc).expect("project parses")
}

fn facts(
    alpha: bool,
    subject: NormBox,
    head: Option<NormBox>,
    mean: Option<[u8; 3]>,
) -> ImageIndex {
    let mut index = ImageIndex::new();
    index.insert(
        "fig.png",
        ImageFacts {
            width: 600,
            height: 900,
            alpha,
            subject,
            head,
            mean_color: mean,
        },
    );
    index
}

fn full() -> NormBox {
    NormBox {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    }
}

/// A 600x900 person: bounds fill the image, head in its top 20 %.
fn person(mean: Option<[u8; 3]>) -> ImageIndex {
    facts(
        true,
        full(),
        Some(NormBox {
            x: 0.3,
            y: 0.0,
            width: 0.4,
            height: 0.2,
        }),
        mean,
    )
}

fn checks(p: &MotionProject, index: &ImageIndex) -> Vec<(LayoutCheck, String)> {
    let frame = LayoutFrame::new(W, H).unwrap();
    layout_report_with(p, &frame, index)
        .findings
        .into_iter()
        .map(|f| (f.check, f.layer))
        .collect()
}

fn has(found: &[(LayoutCheck, String)], check: LayoutCheck, layer: &str) -> bool {
    found.iter().any(|(c, l)| *c == check && l == layer)
}

const PAPER: &str = "#ECE3D2";

// ---------------------------------------------------------------------------
// subject_too_small
// ---------------------------------------------------------------------------

#[test]
fn a_small_hero_subject_is_flagged_and_a_large_one_passes() {
    // 400x600 = 240k px of a 2.07M canvas: 11.6 % < 16 %.
    let small = project(
        PAPER,
        vec![image("b1.subject", (340.0, 700.0, 400.0, 600.0), 22, None)],
    );
    let found = checks(&small, &person(None));
    assert!(
        has(&found, LayoutCheck::SubjectTooSmall, "b1.subject"),
        "{found:?}"
    );

    // 600x900 = 540k: 26 %.
    let large = project(
        PAPER,
        vec![image("b1.subject", (240.0, 600.0, 600.0, 900.0), 22, None)],
    );
    let found = checks(&large, &person(None));
    assert!(
        !has(&found, LayoutCheck::SubjectTooSmall, "b1.subject"),
        "{found:?}"
    );
}

#[test]
fn the_area_is_the_alpha_bounds_not_the_image_box() {
    // The image box is big (720x1080 = 37 %), but the figure only fills the
    // middle 40 % x 50 % of it: 7.5 % of the canvas.
    let layers = vec![image("b1.subject", (180.0, 500.0, 720.0, 1080.0), 22, None)];
    let p = project(PAPER, layers);
    let index = facts(
        true,
        NormBox {
            x: 0.3,
            y: 0.25,
            width: 0.4,
            height: 0.5,
        },
        None,
        None,
    );
    assert!(has(
        &checks(&p, &index),
        LayoutCheck::SubjectTooSmall,
        "b1.subject"
    ));
}

#[test]
fn only_hero_subjects_have_a_minimum_size() {
    // A small delivered object (`hero`) or document (`doc`) is not a hero subject.
    let p = project(
        PAPER,
        vec![
            image("b1.hero", (100.0, 200.0, 200.0, 300.0), 22, None),
            image("b1.doc", (500.0, 200.0, 200.0, 300.0), 22, None),
        ],
    );
    let found = checks(&p, &person(None));
    assert!(
        !found
            .iter()
            .any(|(c, _)| *c == LayoutCheck::SubjectTooSmall),
        "{found:?}"
    );
}

#[test]
fn background_plates_and_unknown_images_are_not_subjects() {
    // A full-bleed environment plate is not judged.
    let plate = project(
        PAPER,
        vec![image("b1.env", (0.0, 0.0, 1080.0, 1920.0), 1, None)],
    );
    assert!(checks(&plate, &person(None)).is_empty());
    // An image the index does not know is skipped, not failed.
    let p = project(
        PAPER,
        vec![image("b1.subject", (340.0, 700.0, 400.0, 600.0), 22, None)],
    );
    assert!(checks(&p, &ImageIndex::new()).is_empty());
}

// ---------------------------------------------------------------------------
// text_over_subject
// ---------------------------------------------------------------------------

/// Subject at (240, 600, 600, 900): head box (360..600, 600..780).
fn subject_layers() -> Vec<Value> {
    vec![image("b1.subject", (240.0, 600.0, 600.0, 900.0), 22, None)]
}

#[test]
fn text_beside_the_subject_is_fine_and_text_across_it_is_not() {
    let mut beside = subject_layers();
    beside.push(text("b1.head.0", (40.0, 300.0, 180.0, 80.0), 24));
    assert!(!has(
        &checks(&project(PAPER, beside), &person(None)),
        LayoutCheck::TextOverSubject,
        "b1.subject"
    ));

    // A 480x80 line across the torso (y 1000): 38400 / 540000 = 7 % > 4 %.
    let mut across = subject_layers();
    across.push(text("b1.head.0", (300.0, 1000.0, 480.0, 80.0), 24));
    let found = checks(&project(PAPER, across), &person(None));
    assert!(
        has(&found, LayoutCheck::TextOverSubject, "b1.subject"),
        "{found:?}"
    );
}

#[test]
fn a_sliver_of_text_under_the_budget_passes() {
    // 3 % of the subject (540000 * 0.03 = 16200 px): 300x54.
    let mut layers = subject_layers();
    layers.push(text("b1.head.0", (240.0, 1446.0, 300.0, 54.0), 24));
    let found = checks(&project(PAPER, layers), &person(None));
    assert!(
        !has(&found, LayoutCheck::TextOverSubject, "b1.subject"),
        "{found:?}"
    );
}

#[test]
fn text_on_its_own_card_is_a_label_but_the_head_stays_clear_of_it() {
    // A label card across the torso carries its text: exempt from the area rule.
    let mut layers = subject_layers();
    layers.push(rect(
        "b1.strip.0",
        (280.0, 990.0, 520.0, 100.0),
        23,
        "#F6F1E6",
    ));
    layers.push(text("b1.head.0", (300.0, 1000.0, 480.0, 80.0), 24));
    let found = checks(&project(PAPER, layers), &person(None));
    assert!(
        !has(&found, LayoutCheck::TextOverSubject, "b1.subject"),
        "{found:?}"
    );

    // The same label over the head is still a finding.
    let mut layers = subject_layers();
    layers.push(rect(
        "b1.strip.0",
        (340.0, 620.0, 300.0, 100.0),
        23,
        "#F6F1E6",
    ));
    layers.push(text("b1.head.0", (350.0, 630.0, 280.0, 80.0), 24));
    let found = checks(&project(PAPER, layers), &person(None));
    assert!(
        found
            .iter()
            .any(|(c, l)| *c == LayoutCheck::TextOverSubject && l == "b1.head.0"),
        "{found:?}"
    );
}

#[test]
fn text_behind_the_subject_may_cross_the_body_but_not_the_head() {
    // Behind the figure, across its body: the subject covers the text, which
    // is the (scored) interlock look, not text over the person.
    let mut body = subject_layers();
    body.push(text("b1.head.0", (300.0, 1000.0, 480.0, 80.0), 20));
    let found = checks(&project(PAPER, body), &person(None));
    assert!(
        !has(&found, LayoutCheck::TextOverSubject, "b1.subject"),
        "{found:?}"
    );

    // Behind the head: flagged (heads are never covered or crossed).
    let mut head = subject_layers();
    head.push(text("b1.head.0", (300.0, 640.0, 480.0, 80.0), 20));
    let found = checks(&project(PAPER, head), &person(None));
    assert!(
        found
            .iter()
            .any(|(c, l)| *c == LayoutCheck::TextOverSubject && l == "b1.head.0"),
        "{found:?}"
    );
}

#[test]
fn ghost_words_and_furniture_are_exempt() {
    let mut layers = subject_layers();
    layers.push(text("b1.ghost", (0.0, 900.0, 1080.0, 300.0), 0));
    layers.push(text("b1.folio", (300.0, 1000.0, 480.0, 80.0), 40));
    let found = checks(&project(PAPER, layers), &person(None));
    assert!(
        !found
            .iter()
            .any(|(c, _)| *c == LayoutCheck::TextOverSubject),
        "{found:?}"
    );
}

// ---------------------------------------------------------------------------
// frame_too_loose
// ---------------------------------------------------------------------------

#[test]
fn a_card_far_bigger_than_its_subject_is_too_loose() {
    // Subject (240, 600, 600, 900); a card 30 % bigger per side behind it.
    let mut loose = vec![rect("b1.plate", (60.0, 330.0, 960.0, 1440.0), 9, "#F6F1E6")];
    loose.extend(subject_layers());
    let found = checks(&project(PAPER, loose), &person(None));
    assert!(
        has(&found, LayoutCheck::FrameTooLoose, "b1.plate"),
        "{found:?}"
    );
}

#[test]
fn a_shrink_wrapped_card_passes() {
    // 6 % pad on each side.
    let (x, y, w, h) = (240.0 - 36.0, 600.0 - 54.0, 600.0 + 72.0, 900.0 + 108.0);
    let mut tight = vec![rect("b1.plate", (x, y, w, h), 9, "#F6F1E6")];
    tight.extend(subject_layers());
    let found = checks(&project(PAPER, tight), &person(None));
    assert!(
        !has(&found, LayoutCheck::FrameTooLoose, "b1.plate"),
        "{found:?}"
    );
}

#[test]
fn full_bleed_grounds_and_cards_in_front_are_not_frames() {
    let mut layers = vec![rect(
        "backdrop.field0",
        (0.0, 0.0, 1080.0, 1920.0),
        1,
        "#CC5533",
    )];
    layers.extend(subject_layers());
    // A card in FRONT of the subject (a caption card) is not a frame behind it.
    layers.push(rect(
        "b1.caption_card",
        (0.0, 400.0, 1080.0, 1400.0),
        30,
        "#F6F1E6",
    ));
    let found = checks(&project(PAPER, layers), &person(None));
    assert!(
        !found.iter().any(|(c, _)| *c == LayoutCheck::FrameTooLoose),
        "{found:?}"
    );
}

// ---------------------------------------------------------------------------
// asset_low_contrast
// ---------------------------------------------------------------------------

const DARK_PAPER: &str = "#16140F";

#[test]
fn a_dark_cutout_on_a_dark_ground_is_flagged_until_it_gets_a_sticker() {
    let dark = Some([28u8, 26, 24]);
    let bare = project(DARK_PAPER, subject_layers());
    let found = checks(&bare, &person(dark));
    assert!(
        has(&found, LayoutCheck::AssetLowContrast, "b1.subject"),
        "{found:?}"
    );

    // A light sticker outline fixes it.
    let sticker = json!({"sticker": {"color": "#ECE3D2", "width_px": 8.0}});
    let mut layers = subject_layers();
    layers[0] = image(
        "b1.subject",
        (240.0, 600.0, 600.0, 900.0),
        22,
        Some(sticker),
    );
    let found = checks(&project(DARK_PAPER, layers), &person(dark));
    assert!(
        !has(&found, LayoutCheck::AssetLowContrast, "b1.subject"),
        "{found:?}"
    );

    // A sticker in the subject's own colour does not.
    let useless = json!({"sticker": {"color": "#1C1A18", "width_px": 8.0}});
    let mut layers = subject_layers();
    layers[0] = image(
        "b1.subject",
        (240.0, 600.0, 600.0, 900.0),
        22,
        Some(useless),
    );
    let found = checks(&project(DARK_PAPER, layers), &person(dark));
    assert!(
        has(&found, LayoutCheck::AssetLowContrast, "b1.subject"),
        "{found:?}"
    );
}

#[test]
fn a_subject_that_reads_on_the_page_is_fine() {
    let p = project(PAPER, subject_layers());
    let found = checks(&p, &person(Some([60, 50, 45])));
    assert!(
        !found
            .iter()
            .any(|(c, _)| *c == LayoutCheck::AssetLowContrast),
        "{found:?}"
    );
    // Unknown colour: no judgement.
    let found = checks(&p, &person(None));
    assert!(
        !found
            .iter()
            .any(|(c, _)| *c == LayoutCheck::AssetLowContrast),
        "{found:?}"
    );
}

#[test]
fn the_ground_is_the_card_the_subject_sits_on() {
    // A mid-dark subject reads on cream paper but not on a same-tone card.
    let mean = Some([0x80u8, 0x60, 0x40]);
    let on_paper = project(PAPER, subject_layers());
    assert!(!has(
        &checks(&on_paper, &person(mean)),
        LayoutCheck::AssetLowContrast,
        "b1.subject"
    ));
    let mut layers = vec![rect("b1.card", (180.0, 540.0, 720.0, 1020.0), 9, "#7A5A3C")];
    layers.extend(subject_layers());
    let found = checks(&project(PAPER, layers), &person(mean));
    assert!(
        has(&found, LayoutCheck::AssetLowContrast, "b1.subject"),
        "{found:?}"
    );
}

#[test]
fn an_opaque_low_contrast_image_needs_a_keyline() {
    let light = Some([232u8, 226, 214]);
    let index = facts(false, full(), None, light);
    let bare = project(PAPER, subject_layers());
    assert!(has(
        &checks(&bare, &index),
        LayoutCheck::AssetLowContrast,
        "b1.subject"
    ));
    let keyline = json!({"edge": {"color": "#171513", "width": 3.0}});
    let mut layers = subject_layers();
    layers[0] = image(
        "b1.subject",
        (240.0, 600.0, 600.0, 900.0),
        22,
        Some(keyline),
    );
    assert!(!has(
        &checks(&project(PAPER, layers), &index),
        LayoutCheck::AssetLowContrast,
        "b1.subject"
    ));
}

// ---------------------------------------------------------------------------
// plumbing
// ---------------------------------------------------------------------------

#[test]
fn without_an_index_nothing_new_is_reported() {
    // `layout_report` has no image facts: it behaves as it did before 0.10 Q.
    let frame = LayoutFrame::new(W, H).unwrap();
    let mut layers = vec![rect("b1.plate", (60.0, 330.0, 960.0, 1440.0), 9, "#F6F1E6")];
    layers.push(image("b1.subject", (340.0, 700.0, 400.0, 600.0), 22, None));
    layers.push(text("b1.head.0", (300.0, 1000.0, 480.0, 80.0), 24));
    let p = project(DARK_PAPER, layers);
    let plain = layout_report(&p, &frame);
    assert!(plain.passed(), "{:?}", plain.findings);
    let with_empty = layout_report_with(&p, &frame, &ImageIndex::new());
    assert_eq!(plain, with_empty);
}

#[test]
fn findings_are_reported_once_and_deterministically() {
    let mut layers = subject_layers();
    layers.push(text("b1.head.0", (300.0, 1000.0, 480.0, 80.0), 24));
    let p = project(PAPER, layers);
    let frame = LayoutFrame::new(W, H).unwrap();
    let index = person(None);
    let a = layout_report_with(&p, &frame, &index);
    let b = layout_report_with(&p, &frame, &index);
    assert_eq!(a, b);
    let n = a
        .findings
        .iter()
        .filter(|f| f.check == LayoutCheck::TextOverSubject && f.layer == "b1.subject")
        .count();
    assert_eq!(n, 1, "one finding across the three samples");
}
