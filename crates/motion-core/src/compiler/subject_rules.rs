//! (0.10 Q) Subject rules shared by the builders and layout QA: the
//! `taste_rules` budgets turned into functions, so a builder that places a
//! subject and the QA check that judges it use the same numbers.
//!
//! * [`subject_min_area`] — the canvas-class share a hero subject must fill.
//! * [`treated_color`] — a subject's mean colour after its image treatment's
//!   colour operations (the renderer's formulas, applied to one colour).
//! * [`neighbours`] / [`needs_contrast_treatment`] / [`guard_contrast`] — the
//!   contrast rule: a subject whose mean colour is under
//!   `ASSET_GROUND_MIN_CONTRAST` against what touches its silhouette gets a
//!   sticker outline (cutouts) or a keyline (opaque images).
//! * [`shrink_wrap`] — the frame rule: a card drawn behind a subject exceeds
//!   it by at most `FRAME_PAD_MAX` per side.

use super::explore::contrast_ratio;
use super::layout_frame::LayoutFrame;
use super::taste_rules::{
    ASSET_GROUND_MIN_CONTRAST, FRAME_PAD_MAX, NARROW_SUBJECT_ASPECT, SUBJECT_MIN_AREA,
};
use super::theme::Palette;
use crate::intent::Format;
use crate::scene::{Color, ImageTreatment, PaperEdge, Sticker};

/// Sticker outline width, in `u`.
pub const STICKER_WIDTH_U: f32 = 8.0;
/// Keyline width around a low-contrast opaque image, in `u` (2–4u).
pub const KEYLINE_WIDTH_U: f32 = 3.0;
/// An existing paper edge at least this wide (in `u`) already separates a
/// cutout from its ground: its colour, not the ground, touches the subject.
pub const EDGE_SEPARATES_U: f32 = 2.0;

/// Minimum subject alpha-bounds area as a fraction of the canvas area for the
/// frame's layout class (`SUBJECT_MIN_AREA`: tall, square, wide).
pub fn subject_min_area(frame: &LayoutFrame) -> f32 {
    match frame.class {
        Format::Vertical => SUBJECT_MIN_AREA.0,
        Format::Square => SUBJECT_MIN_AREA.1,
        Format::Landscape => SUBJECT_MIN_AREA.2,
    }
}

/// (0.10 Q) The minimum for a subject of alpha-bounds `aspect` (w/h): narrow
/// subjects (standing figures) scale it by `aspect / NARROW_SUBJECT_ASPECT`,
/// clamped to [0.5, 1] of the base; `None` = the base.
pub fn subject_min_area_for(frame: &LayoutFrame, aspect: Option<f32>) -> f32 {
    let base = subject_min_area(frame);
    match aspect.filter(|a| a.is_finite() && *a > 0.0) {
        Some(a) => base * (a / NARROW_SUBJECT_ASPECT).clamp(0.5, 1.0),
        None => base,
    }
}

/// Alpha-bounds aspect (w/h) of a manifest entry's subject.
pub fn entry_subject_aspect(entry: &crate::assets::ManifestEntry) -> Option<f32> {
    let sb = entry.analysis.as_ref()?.subject_bounds;
    let w = sb.width * entry.width as f32;
    let h = sb.height * entry.height as f32;
    (w > 0.0 && h > 0.0).then(|| w / h)
}

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn rgb01(c: Color) -> [f32; 3] {
    [
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
    ]
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// The mean subject colour after `t`'s colour operations (desaturate →
/// brightness/contrast → duotone → tint, as `docs/IMAGE_TREATMENTS.md` and the
/// renderer define them; grain averages out). An estimate: the operations are
/// applied to the mean colour, not to every pixel.
pub fn treated_color(mean: [u8; 3], t: &ImageTreatment) -> Color {
    let mut c = [
        f32::from(mean[0]) / 255.0,
        f32::from(mean[1]) / 255.0,
        f32::from(mean[2]) / 255.0,
    ];
    if t.desaturate != 0.0 {
        let l = luma(c);
        c = mix3(c, [l, l, l], t.desaturate);
    }
    if t.brightness != 0.0 || t.contrast != 1.0 {
        for v in &mut c {
            *v = ((*v - 0.5) * t.contrast + 0.5 + t.brightness).clamp(0.0, 1.0);
        }
    }
    if let Some(d) = t.duotone.filter(|d| d.amount != 0.0) {
        let target = mix3(rgb01(d.shadow), rgb01(d.highlight), luma(c).clamp(0.0, 1.0));
        c = mix3(c, target, d.amount);
    }
    if let Some(k) = t.tint.filter(|k| k.amount != 0.0) {
        c = mix3(c, rgb01(k.color), k.amount);
    }
    Color::rgb(to_u8(c[0]), to_u8(c[1]), to_u8(c[2]))
}

/// The grounds a subject may sit on: the page, the card stock, and the
/// palette's graphic colour fields (print backgrounds). A superset of what any
/// builder puts under a subject, so a subject that clears every one of them
/// never reads as vanishing wherever it lands.
pub fn ground_colors(palette: &Palette) -> Vec<Color> {
    let mut grounds = vec![palette.paper, palette.card];
    for f in &palette.fields {
        if !grounds.contains(f) {
            grounds.push(*f);
        }
    }
    grounds
}

/// What touches a subject's silhouette: for a cutout with a paper edge of at
/// least [`EDGE_SEPARATES_U`] it is the edge colour, otherwise the grounds.
pub fn neighbours(t: &ImageTreatment, u: f32, alpha: bool, grounds: &[Color]) -> Vec<Color> {
    match t.edge {
        Some(e) if alpha && e.width >= EDGE_SEPARATES_U * u => vec![e.color],
        _ => grounds.to_vec(),
    }
}

/// True when `subject` (a treated mean colour) is below
/// `ASSET_GROUND_MIN_CONTRAST` against any of `neighbours`.
pub fn needs_contrast_treatment(subject: Color, neighbours: &[Color]) -> bool {
    neighbours
        .iter()
        .any(|&n| contrast_ratio(subject, n) < ASSET_GROUND_MIN_CONTRAST)
}

/// Paper or ink, whichever contrasts more with `subject` (paper on a tie).
pub fn outline_color(subject: Color, paper: Color, ink: Color) -> Color {
    if contrast_ratio(subject, paper) >= contrast_ratio(subject, ink) {
        paper
    } else {
        ink
    }
}

/// Apply the contrast rule to a delivered image's treatment. `mean` is the
/// image's `AssetAnalysis::mean_color` (absent in pre-0.10 manifests: nothing
/// is decided then). A low-contrast cutout gets a [`Sticker`] in paper or ink
/// ([`STICKER_WIDTH_U`], replacing its paper edge and shadow); a low-contrast
/// opaque image gets a [`KEYLINE_WIDTH_U`] keyline. Identity otherwise.
pub fn guard_contrast(
    mut t: ImageTreatment,
    mean: Option<[u8; 3]>,
    alpha: bool,
    palette: &Palette,
    u: f32,
) -> ImageTreatment {
    let Some(mean) = mean else {
        return t;
    };
    let subject = treated_color(mean, &t);
    let near = neighbours(&t, u, alpha, &ground_colors(palette));
    if !needs_contrast_treatment(subject, &near) {
        return t;
    }
    let color = outline_color(subject, palette.paper, palette.ink);
    if alpha {
        t.sticker = Some(Sticker {
            color,
            width_px: STICKER_WIDTH_U * u,
        });
        t.edge = None;
        t.shadow = None;
    } else {
        t.edge = Some(PaperEdge {
            color,
            width: KEYLINE_WIDTH_U * u,
        });
    }
    t
}

/// A card for a subject occupying `(x, y, w, h)`: the subject box grown by
/// `pad` of its size on each side (clamped to [`FRAME_PAD_MAX`]), as
/// `(x, y, w, h)`.
pub fn shrink_wrap(subject: (f32, f32, f32, f32), pad: f32) -> (f32, f32, f32, f32) {
    let pad = pad.clamp(0.0, FRAME_PAD_MAX);
    let (x, y, w, h) = subject;
    (
        x - pad * w,
        y - pad * h,
        w * (1.0 + 2.0 * pad),
        h * (1.0 + 2.0 * pad),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Duotone, TintSpec};
    use crate::style::StyleProfile;

    fn palette() -> Palette {
        Palette::for_style(&StyleProfile::default())
    }

    #[test]
    fn narrow_subjects_get_a_proportional_minimum() {
        let f = LayoutFrame::new(1080, 1920).unwrap();
        let base = subject_min_area(&f);
        assert_eq!(subject_min_area_for(&f, None), base);
        assert_eq!(subject_min_area_for(&f, Some(0.8)), base);
        assert!((subject_min_area_for(&f, Some(0.31)) - base * 0.62).abs() < 1e-6);
        assert_eq!(subject_min_area_for(&f, Some(0.1)), base * 0.5);
    }

    #[test]
    fn min_area_follows_the_layout_class() {
        let area = |w, h| subject_min_area(&LayoutFrame::new(w, h).unwrap());
        assert_eq!(area(1080, 1920), SUBJECT_MIN_AREA.0);
        assert_eq!(area(1080, 1080), SUBJECT_MIN_AREA.1);
        assert_eq!(area(1920, 1080), SUBJECT_MIN_AREA.2);
    }

    #[test]
    fn natural_treatment_keeps_the_mean_colour() {
        let c = treated_color([120, 80, 40], &ImageTreatment::natural());
        assert_eq!((c.r, c.g, c.b), (120, 80, 40));
    }

    #[test]
    fn colour_operations_apply_in_the_renderers_order() {
        // Full desaturation: every channel becomes the Rec.709 luma.
        let mut t = ImageTreatment::natural();
        t.desaturate = 1.0;
        let c = treated_color([200, 100, 50], &t);
        assert_eq!((c.r, c.g), (c.g, c.b));
        // A full tint replaces the colour.
        let mut t = ImageTreatment::natural();
        t.tint = Some(TintSpec {
            color: Color::rgb(10, 20, 30),
            amount: 1.0,
        });
        let c = treated_color([200, 100, 50], &t);
        assert_eq!((c.r, c.g, c.b), (10, 20, 30));
        // Duotone maps black to the shadow colour.
        let mut t = ImageTreatment::natural();
        t.duotone = Some(Duotone {
            shadow: Color::rgb(40, 0, 0),
            highlight: Color::rgb(255, 255, 255),
            amount: 1.0,
        });
        let c = treated_color([0, 0, 0], &t);
        assert_eq!((c.r, c.g, c.b), (40, 0, 0));
    }

    #[test]
    fn a_dark_cutout_on_a_dark_ground_gets_a_light_sticker() {
        let mut dark = palette();
        dark.paper = Color::rgb(0x16, 0x14, 0x12);
        dark.card = Color::rgb(0x22, 0x20, 0x1D);
        dark.ink = Color::rgb(0xEE, 0xE8, 0xDC);
        let t = guard_contrast(
            ImageTreatment::natural(),
            Some([28, 26, 24]),
            true,
            &dark,
            1.0,
        );
        let s = t
            .sticker
            .expect("a dark subject on a dark page needs a sticker");
        // Paper and ink swap roles on a dark ground: the sticker uses
        // whichever contrasts more with the subject (here the light ink).
        assert_eq!(s.color, dark.ink);
        assert_eq!(s.width_px, STICKER_WIDTH_U);
        assert!(t.edge.is_none() && t.shadow.is_none());
        // Scales with the canvas unit.
        let big = guard_contrast(
            ImageTreatment::natural(),
            Some([28, 26, 24]),
            true,
            &dark,
            2.0,
        );
        assert_eq!(big.sticker.map(|s| s.width_px), Some(2.0 * STICKER_WIDTH_U));
    }

    #[test]
    fn a_light_cutout_on_light_paper_gets_an_ink_sticker() {
        let p = palette();
        let t = guard_contrast(
            ImageTreatment::natural(),
            Some([236, 232, 222]),
            true,
            &p,
            1.0,
        );
        assert_eq!(t.sticker.map(|s| s.color), Some(p.ink));
    }

    #[test]
    fn a_subject_that_reads_on_the_ground_is_untouched() {
        let p = palette();
        let t = ImageTreatment::natural();
        // Mid-dark clothing on cream paper: contrast well above 2:1.
        assert_eq!(
            guard_contrast(t.clone(), Some([60, 50, 45]), true, &p, 1.0),
            t
        );
        // Unknown colour (pre-0.10 manifest): no decision.
        assert_eq!(guard_contrast(t.clone(), None, true, &p, 1.0), t);
    }

    #[test]
    fn graphic_fields_are_grounds_too() {
        let mut p = palette();
        // A mid-tone subject reads on paper but not on a same-tone field.
        p.fields = vec![Color::rgb(0xB8, 0x6A, 0x2E)];
        let mean = [0xA8, 0x62, 0x2A];
        let t = guard_contrast(ImageTreatment::natural(), Some(mean), true, &p, 1.0);
        assert!(t.sticker.is_some());
        p.fields.clear();
        let t = guard_contrast(ImageTreatment::natural(), Some(mean), true, &p, 1.0);
        assert!(t.sticker.is_none());
    }

    #[test]
    fn an_existing_paper_edge_separates_a_dark_cutout() {
        let mut dark = palette();
        dark.paper = Color::rgb(0x16, 0x14, 0x12);
        dark.card = Color::rgb(0x22, 0x20, 0x1D);
        let mut t = ImageTreatment::natural();
        // PaperCutout's light 7u edge already outlines a dark subject.
        t.edge = Some(PaperEdge {
            color: Color::rgb(0xF6, 0xF1, 0xE6),
            width: 7.0,
        });
        assert_eq!(
            guard_contrast(t.clone(), Some([28, 26, 24]), true, &dark, 1.0),
            t
        );
        // ...but not a light subject on that light edge.
        let g = guard_contrast(t, Some([240, 236, 226]), true, &dark, 1.0);
        assert!(g.sticker.is_some());
    }

    #[test]
    fn opaque_low_contrast_images_get_a_keyline_not_a_sticker() {
        let p = palette();
        let t = guard_contrast(
            ImageTreatment::natural(),
            Some([232, 226, 214]),
            false,
            &p,
            1.0,
        );
        assert!(t.sticker.is_none());
        let e = t.edge.expect("keyline");
        assert_eq!(e.width, KEYLINE_WIDTH_U);
        assert_eq!(e.color, p.ink);
    }

    #[test]
    fn shrink_wrap_pads_by_a_fraction_of_the_subject_up_to_the_cap() {
        let (x, y, w, h) = shrink_wrap((100.0, 200.0, 400.0, 300.0), 0.05);
        assert_eq!((x, y, w, h), (80.0, 185.0, 440.0, 330.0));
        // Never beyond FRAME_PAD_MAX per side.
        let (_, _, w, h) = shrink_wrap((0.0, 0.0, 100.0, 100.0), 0.9);
        assert!((w - 100.0 * (1.0 + 2.0 * FRAME_PAD_MAX)).abs() < 1e-3);
        assert!((h - 100.0 * (1.0 + 2.0 * FRAME_PAD_MAX)).abs() < 1e-3);
    }
}
