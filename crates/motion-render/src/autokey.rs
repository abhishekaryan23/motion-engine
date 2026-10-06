//! Auto-key on delivery (0.10 Q): an opaque image whose border is one flat
//! colour is a cutout that a generator shot on a studio ground. Ingestion keys
//! it into an alpha cutout cropped to the subject, so the compiler can place it
//! like any other cutout (no backing card, subject-bounds sizing, treatments
//! that follow the silhouette). Photographs with busy borders stay opaque.
//!
//! The algorithm is `scripts/key_cutout.py` made generic (no colour-specific
//! despill), outside the compiler and deterministic:
//!
//! 1. **Flat border.** The ground is the per-channel median of the border
//!    pixels; the image qualifies when at least [`FLAT_BORDER_FRACTION`] of the
//!    border pixels lie within [`FLAT_BORDER_DELTA`] of it (colour distance =
//!    Euclidean RGB distance / √3, channels in 0..1).
//! 2. **Background** = near-ground pixels (distance < [`KEY_HI`]) that are
//!    4-connected to the border, plus enclosed pockets that hold near-exact
//!    ground colour (distance < [`KEY_CORE`]): the gaps between a figure's arms
//!    and body, inside handles.
//! 3. **Soft band.** Background alpha = smoothstep over `[KEY_LO, KEY_HI]` of
//!    the distance, so edges anti-alias; everything else is opaque.
//! 4. Foreground islands under [`MIN_ISLAND`] of the frame are dropped; the
//!    ground colour is pulled out of the semi-transparent edge pixels (two
//!    pixels around them) by un-mixing.
//! 5. **Crop** to the alpha bounds plus [`CROP_PAD`] of the longer side; thin
//!    subjects are padded with transparency up to [`MIN_SHORT_SIDE`] (never
//!    upscaled).
//!
//! The result is `None` (the image stays opaque) when the image already has
//! transparency, the border is not flat, or keying would leave almost nothing
//! or almost everything.

use resvg::tiny_skia::{Pixmap, PremultipliedColorU8};

/// Share of the border pixels that must be near the median ground.
pub const FLAT_BORDER_FRACTION: f64 = 0.90;
/// Colour distance (RGB in 0..1, Euclidean / √3) within which a border pixel
/// counts as ground.
pub const FLAT_BORDER_DELTA: f32 = 0.08;
/// Colour distance at or below which a connected pixel is fully background.
pub const KEY_LO: f32 = 0.18;
/// Colour distance at or above which a pixel is fully foreground.
pub const KEY_HI: f32 = 0.38;
/// An enclosed region holding pixels this close to the ground is ground too.
pub const KEY_CORE: f32 = 0.08;
/// Foreground islands smaller than this fraction of the frame are dropped.
pub const MIN_ISLAND: f32 = 0.0015;
/// Crop padding as a fraction of the subject's longer side.
pub const CROP_PAD: f32 = 0.06;
/// Ingest's hard minimum short side: thin crops are padded up to it.
pub const MIN_SHORT_SIDE: u32 = 512;
/// Alpha above which a pixel counts towards the crop bounds.
const CROP_ALPHA: f32 = 0.04;
/// Keying that leaves less than / more than this share of the frame as
/// foreground did not find a subject on a ground.
const MIN_FOREGROUND: f32 = 0.01;
const MAX_FOREGROUND: f32 = 0.97;

/// A keyed cutout.
#[derive(Debug, Clone)]
pub struct Keyed {
    /// The cutout (premultiplied RGBA), cropped around the subject.
    pub pixmap: Pixmap,
    /// The flat ground colour that was removed (median of the border), sRGB.
    pub ground: [u8; 3],
    /// Subject bounding box in the SOURCE image, pixels `[x0, y0, x1, y1)`.
    pub subject_px: [u32; 4],
    /// Source pixel of the cutout's top-left corner (negative where the crop
    /// was padded with transparency past the source edge).
    pub origin: (i64, i64),
    /// Source image size in pixels.
    pub source: (u32, u32),
}

impl Keyed {
    /// A normalized box of the SOURCE image (a sidecar's face or head box)
    /// expressed in the cutout, clamped to it; `None` when it falls outside.
    pub fn remap_box(
        &self,
        b: motion_core::assets::NormBox,
    ) -> Option<motion_core::assets::NormBox> {
        let (sw, sh) = (f64::from(self.source.0), f64::from(self.source.1));
        let (ow, oh) = (
            f64::from(self.pixmap.width()),
            f64::from(self.pixmap.height()),
        );
        let (ox, oy) = (self.origin.0 as f64, self.origin.1 as f64);
        let x0 = ((f64::from(b.x) * sw - ox) / ow).clamp(0.0, 1.0);
        let y0 = ((f64::from(b.y) * sh - oy) / oh).clamp(0.0, 1.0);
        let x1 = (((f64::from(b.x) + f64::from(b.width)) * sw - ox) / ow).clamp(0.0, 1.0);
        let y1 = (((f64::from(b.y) + f64::from(b.height)) * sh - oy) / oh).clamp(0.0, 1.0);
        (x1 > x0 && y1 > y0).then_some(motion_core::assets::NormBox {
            x: x0 as f32,
            y: y0 as f32,
            width: (x1 - x0) as f32,
            height: (y1 - y0) as f32,
        })
    }

    /// A normalized point of the SOURCE image expressed in the cutout.
    pub fn remap_point(&self, p: motion_core::assets::NormPoint) -> motion_core::assets::NormPoint {
        let (sw, sh) = (f64::from(self.source.0), f64::from(self.source.1));
        let (ow, oh) = (
            f64::from(self.pixmap.width()),
            f64::from(self.pixmap.height()),
        );
        motion_core::assets::NormPoint {
            x: (((f64::from(p.x) * sw) - self.origin.0 as f64) / ow).clamp(0.0, 1.0) as f32,
            y: (((f64::from(p.y) * sh) - self.origin.1 as f64) / oh).clamp(0.0, 1.0) as f32,
        }
    }
}

type Rgb = [f32; 3];

fn distance(a: Rgb, b: Rgb) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / 3f32.sqrt()
}

/// Median of a non-empty slice (mean of the two middle values when even).
fn median(values: &mut [f32]) -> f32 {
    values.sort_by(f32::total_cmp);
    let n = values.len();
    if n % 2 == 1 {
        values[n / 2]
    } else {
        (values[n / 2 - 1] + values[n / 2]) / 2.0
    }
}

/// Straight sRGB (0..1) of every pixel of an opaque pixmap.
fn rgb_pixels(img: &Pixmap) -> Vec<Rgb> {
    img.pixels()
        .iter()
        .map(|p| {
            [
                f32::from(p.red()) / 255.0,
                f32::from(p.green()) / 255.0,
                f32::from(p.blue()) / 255.0,
            ]
        })
        .collect()
}

/// Border pixel indices in the order top row, bottom row, left column, right
/// column (corners appear twice, as in `key_cutout.py`).
fn border_indices(w: usize, h: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(2 * (w + h));
    out.extend(0..w);
    out.extend((h - 1) * w..h * w);
    out.extend((0..h).map(|y| y * w));
    out.extend((0..h).map(|y| y * w + w - 1));
    out
}

/// The ground colour (per-channel median of the border) when the border is
/// flat: at least [`FLAT_BORDER_FRACTION`] of its pixels within
/// [`FLAT_BORDER_DELTA`] of the median. `None` for images with transparency.
pub fn flat_ground(img: &Pixmap) -> Option<[f32; 3]> {
    if img.pixels().iter().any(|p| p.alpha() < 255) {
        return None;
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let rgb = rgb_pixels(img);
    let border = border_indices(w, h);
    let mut ground = [0.0f32; 3];
    for (c, g) in ground.iter_mut().enumerate() {
        let mut ch: Vec<f32> = border.iter().map(|&i| rgb[i][c]).collect();
        *g = median(&mut ch);
    }
    let near = border
        .iter()
        .filter(|&&i| distance(rgb[i], ground) <= FLAT_BORDER_DELTA)
        .count();
    (near as f64 >= FLAT_BORDER_FRACTION * border.len() as f64).then_some(ground)
}

/// 4-connected components of `mask`: per-pixel label (0 = not in the mask,
/// 1.. in scan order) and the number of labels.
fn label4(mask: &[bool], w: usize, h: usize) -> (Vec<u32>, usize) {
    let mut labels = vec![0u32; mask.len()];
    let mut count = 0usize;
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..mask.len() {
        if !mask[start] || labels[start] != 0 {
            continue;
        }
        count += 1;
        labels[start] = count as u32;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if mask[j] && labels[j] == 0 {
                    labels[j] = count as u32;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
    }
    (labels, count)
}

/// One 4-neighbour binary dilation.
fn dilate4(mask: &[bool], w: usize, h: usize) -> Vec<bool> {
    let mut out = mask.to_vec();
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if mask[i] {
                continue;
            }
            out[i] = (x > 0 && mask[i - 1])
                || (x + 1 < w && mask[i + 1])
                || (y > 0 && mask[i - w])
                || (y + 1 < h && mask[i + w]);
        }
    }
    out
}

fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Key an opaque image on a flat ground into a cropped alpha cutout. `None`
/// when the image should stay opaque (see the module docs).
pub fn key_flat_ground(img: &Pixmap) -> Option<Keyed> {
    let ground = flat_ground(img)?;
    let (w, h) = (img.width() as usize, img.height() as usize);
    let rgb = rgb_pixels(img);
    let dist: Vec<f32> = rgb.iter().map(|&c| distance(c, ground)).collect();

    // Background: near-ground pixels connected to the border, plus enclosed
    // pockets holding near-exact ground colour.
    let near: Vec<bool> = dist.iter().map(|&d| d < KEY_HI).collect();
    let (labels, count) = label4(&near, w, h);
    let mut is_bg = vec![false; count + 1];
    for &i in &border_indices(w, h) {
        is_bg[labels[i] as usize] = true;
    }
    for (i, &d) in dist.iter().enumerate() {
        if d < KEY_CORE {
            is_bg[labels[i] as usize] = true;
        }
    }
    is_bg[0] = false;
    let mut alpha: Vec<f32> = labels
        .iter()
        .zip(&dist)
        .map(|(&l, &d)| {
            if is_bg[l as usize] {
                smoothstep(((d - KEY_LO) / (KEY_HI - KEY_LO)).clamp(0.0, 1.0))
            } else {
                1.0
            }
        })
        .collect();

    // Drop tiny foreground islands (specks).
    let solid: Vec<bool> = alpha.iter().map(|&a| a > 0.5).collect();
    let (flabels, fcount) = label4(&solid, w, h);
    if fcount > 0 {
        let mut sizes = vec![0usize; fcount + 1];
        for &l in &flabels {
            sizes[l as usize] += 1;
        }
        let min_size = MIN_ISLAND * (w * h) as f32;
        for (a, &l) in alpha.iter_mut().zip(&flabels) {
            if l > 0 && (sizes[l as usize] as f32) < min_size {
                *a = 0.0;
            }
        }
    }

    let foreground = solid_share(&alpha);
    if !(MIN_FOREGROUND..=MAX_FOREGROUND).contains(&foreground) {
        return None;
    }

    // Despill: un-mix the ground from the semi-transparent edge pixels (and
    // two pixels around them that still carry a ground fringe).
    let partial: Vec<bool> = alpha.iter().map(|&a| a < 0.999 && a > 0.0).collect();
    let near_edge = dilate4(&dilate4(&partial, w, h), w, h);
    let mut out = rgb.clone();
    for i in 0..out.len() {
        if near_edge[i] && alpha[i] > 0.0 {
            let a = alpha[i].max(1e-3);
            for c in 0..3 {
                out[i][c] = ((rgb[i][c] - (1.0 - alpha[i]) * ground[c]) / a).clamp(0.0, 1.0);
            }
        }
    }

    // Tight crop to the alpha bounds plus padding.
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for y in 0..h {
        for x in 0..w {
            if alpha[y * w + x] > CROP_ALPHA {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let pad = (CROP_PAD * (x1 - x0).max(y1 - y0) as f32).round() as usize;
    let (cx0, cy0) = (x0.saturating_sub(pad), y0.saturating_sub(pad));
    let (cx1, cy1) = ((x1 + pad).min(w), (y1 + pad).min(h));
    let (pw, ph) = (cx1 - cx0, cy1 - cy0);

    // Thin crops: transparent padding up to the minimum short side.
    let min_short = MIN_SHORT_SIDE as usize;
    let ex = if pw <= ph && pw < min_short {
        min_short - pw
    } else {
        0
    };
    let ey = if ph <= pw && ph < min_short {
        min_short - ph
    } else {
        0
    };
    let mut pixmap = Pixmap::new((pw + ex) as u32, (ph + ey) as u32)?;
    let stride = pw + ex;
    let pixels = pixmap.pixels_mut();
    for y in 0..ph {
        for x in 0..pw {
            let i = (cy0 + y) * w + cx0 + x;
            let a = to_u8(alpha[i]);
            let premul = |c: f32| ((u32::from(to_u8(c)) * u32::from(a) + 127) / 255) as u8;
            let px = PremultipliedColorU8::from_rgba(
                premul(out[i][0]),
                premul(out[i][1]),
                premul(out[i][2]),
                a,
            )?;
            pixels[(ey / 2 + y) * stride + ex / 2 + x] = px;
        }
    }

    Some(Keyed {
        pixmap,
        ground: [to_u8(ground[0]), to_u8(ground[1]), to_u8(ground[2])],
        subject_px: [x0 as u32, y0 as u32, x1 as u32, y1 as u32],
        origin: (cx0 as i64 - (ex / 2) as i64, cy0 as i64 - (ey / 2) as i64),
        source: (w as u32, h as u32),
    })
}

/// Share of pixels that are solid foreground (alpha above 0.5).
fn solid_share(alpha: &[f32]) -> f32 {
    if alpha.is_empty() {
        return 0.0;
    }
    alpha.iter().filter(|&&a| a > 0.5).count() as f32 / alpha.len() as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opaque image of `ground`, with `paint(x, y)` overriding pixels.
    fn image(
        w: u32,
        h: u32,
        ground: [u8; 3],
        paint: impl Fn(u32, u32) -> Option<[u8; 3]>,
    ) -> Pixmap {
        let mut pm = Pixmap::new(w, h).expect("pixmap");
        let data = pm.data_mut();
        for y in 0..h {
            for x in 0..w {
                let c = paint(x, y).unwrap_or(ground);
                let i = ((y * w + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        pm
    }

    fn alpha_at(pm: &Pixmap, x: u32, y: u32) -> u8 {
        pm.pixel(x, y).expect("in range").alpha()
    }

    const GROUND: [u8; 3] = [236, 230, 218];
    const RED: [u8; 3] = [150, 30, 30];

    /// A 1200x1600 shot with a red block subject [300, 900) x [400, 1300) on
    /// the flat ground, and a ground-coloured pocket inside it.
    fn studio_shot() -> Pixmap {
        image(1200, 1600, GROUND, |x, y| {
            let in_block = (300..900).contains(&x) && (400..1300).contains(&y);
            let in_pocket = (500..650).contains(&x) && (700..850).contains(&y);
            (in_block && !in_pocket).then_some(RED)
        })
    }

    #[test]
    fn flat_ground_shot_becomes_an_alpha_cutout_cropped_to_the_subject() {
        let keyed = key_flat_ground(&studio_shot()).expect("flat ground keys");
        assert_eq!(keyed.ground, GROUND);
        assert_eq!(keyed.subject_px, [300, 400, 900, 1300]);
        // Crop = subject bounds + 6 % of the longer side (900) on every side.
        let pad = (0.06f32 * 900.0).round() as u32;
        let (bw, bh) = (600 + 2 * pad, 900 + 2 * pad);
        assert_eq!(
            (keyed.pixmap.width(), keyed.pixmap.height()),
            (bw, bh),
            "bounds + 6 % pad"
        );
        let pm = &keyed.pixmap;
        // The ground is gone: corners are transparent; the subject is opaque.
        assert_eq!(alpha_at(pm, 0, 0), 0);
        assert_eq!(alpha_at(pm, bw - 1, bh - 1), 0);
        assert_eq!(alpha_at(pm, pad + 20, pad + 20), 255);
        let p = pm.pixel(pad + 20, pad + 20).expect("px");
        assert_eq!((p.red(), p.green(), p.blue()), (RED[0], RED[1], RED[2]));
        // The enclosed pocket (ground showing through) is keyed out too.
        assert_eq!(alpha_at(pm, pad + 200 + 70, pad + 300 + 70), 0, "pocket");
        // Tight: the subject touches the pad, not the frame edge.
        let a = crate::analysis::analyze(pm, &crate::analysis::AnalyzeOptions::default());
        assert!(!a.edges.left && !a.edges.top && !a.edges.right && !a.edges.bottom);
        assert!(
            a.subject_bounds.width > 0.8,
            "subject fills the crop: {:?}",
            a.subject_bounds
        );
    }

    #[test]
    fn sidecar_geometry_is_remapped_into_the_cutout() {
        use motion_core::assets::{NormBox, NormPoint};
        let keyed = key_flat_ground(&studio_shot()).expect("keys");
        // Crop = [246, 346) .. +708 x 1008 of the 1200x1600 source.
        assert_eq!(keyed.origin, (246, 346));
        assert_eq!(keyed.source, (1200, 1600));
        // The subject box (300,400,600,900) fills the cutout inside the 54 px pad.
        let b = keyed
            .remap_box(NormBox {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5625,
            })
            .expect("inside");
        let (ow, oh) = (keyed.pixmap.width() as f32, keyed.pixmap.height() as f32);
        assert!((b.x - 54.0 / ow).abs() < 1e-4 && (b.width - 600.0 / ow).abs() < 1e-4);
        assert!((b.y - 54.0 / oh).abs() < 1e-4 && (b.height - 900.0 / oh).abs() < 1e-4);
        // A box outside the crop is dropped; a point is clamped into it.
        assert!(keyed
            .remap_box(NormBox {
                x: 0.0,
                y: 0.0,
                width: 0.1,
                height: 0.1
            })
            .is_none());
        let p = keyed.remap_point(NormPoint { x: 0.0, y: 1.0 });
        assert_eq!((p.x, p.y), (0.0, 1.0));
    }

    #[test]
    fn thin_subjects_are_padded_with_transparency_to_the_minimum_short_side() {
        // A small object: 300x300 subject -> crop ~336 -> padded to 512.
        let shot = image(1000, 1000, GROUND, |x, y| {
            ((350..650).contains(&x) && (350..650).contains(&y)).then_some(RED)
        });
        let keyed = key_flat_ground(&shot).expect("keys");
        let pm = &keyed.pixmap;
        assert_eq!((pm.width(), pm.height()), (MIN_SHORT_SIDE, MIN_SHORT_SIDE));
        // Centred: the middle pixel is subject, the frame corners are empty.
        assert_eq!(alpha_at(pm, 256, 256), 255);
        assert_eq!(alpha_at(pm, 0, 0), 0);
        assert_eq!(alpha_at(pm, 511, 511), 0);
    }

    #[test]
    fn busy_borders_stay_opaque() {
        // A photograph-like image: the border carries a strong gradient and
        // texture, so no flat ground exists.
        let photo = image(640, 800, GROUND, |x, y| {
            Some([
                (x * 255 / 639) as u8,
                (y * 255 / 799) as u8,
                ((x * 7 + y * 13) % 256) as u8,
            ])
        });
        assert!(flat_ground(&photo).is_none());
        assert!(key_flat_ground(&photo).is_none());
    }

    #[test]
    fn a_nearly_flat_border_below_ninety_percent_stays_opaque() {
        // 15 % of the border pixels are a different colour: not flat.
        let (w, h) = (400u32, 400u32);
        let shot = image(w, h, GROUND, |x, y| {
            let border = x == 0 || y == 0 || x == w - 1 || y == h - 1;
            if border && (x + y) % 20 < 3 {
                return Some([20, 20, 200]);
            }
            ((150..250).contains(&x) && (150..250).contains(&y)).then_some(RED)
        });
        assert!(flat_ground(&shot).is_none());
        assert!(key_flat_ground(&shot).is_none());
    }

    #[test]
    fn images_with_transparency_are_left_alone() {
        let mut pm = studio_shot();
        pm.pixels_mut()[0] = PremultipliedColorU8::TRANSPARENT;
        assert!(flat_ground(&pm).is_none());
        assert!(key_flat_ground(&pm).is_none());
    }

    #[test]
    fn an_empty_ground_has_no_subject_to_key() {
        let blank = image(600, 600, GROUND, |_, _| None);
        assert!(key_flat_ground(&blank).is_none());
    }

    #[test]
    fn keying_is_deterministic() {
        let a = key_flat_ground(&studio_shot()).expect("keys");
        let b = key_flat_ground(&studio_shot()).expect("keys");
        assert_eq!(a.pixmap.data(), b.pixmap.data());
        assert_eq!(a.subject_px, b.subject_px);
    }

    #[test]
    fn soft_edges_get_a_partial_alpha_band() {
        // A subject whose edge fades from red to ground over 20 px.
        let shot = image(1000, 1000, GROUND, |x, y| {
            if !(300..700).contains(&y) {
                return None;
            }
            let t = ((x as f32 - 300.0) / 20.0).clamp(0.0, 1.0);
            let t = if x > 500 {
                ((700.0 - x as f32) / 20.0).clamp(0.0, 1.0)
            } else {
                t
            };
            if t <= 0.0 {
                return None;
            }
            let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
            Some([
                mix(GROUND[0], RED[0]),
                mix(GROUND[1], RED[1]),
                mix(GROUND[2], RED[2]),
            ])
        });
        let keyed = key_flat_ground(&shot).expect("keys");
        let partial = keyed
            .pixmap
            .pixels()
            .iter()
            .filter(|p| p.alpha() > 8 && p.alpha() < 247)
            .count();
        assert!(partial > 100, "a soft band exists, got {partial} px");
        assert!(keyed.pixmap.pixels().iter().any(|p| p.alpha() == 255));
    }
}
