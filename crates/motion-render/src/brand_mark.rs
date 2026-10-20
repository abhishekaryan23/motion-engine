//! (0.24 M10) Operator brand mark: a small logo drawn by the renderer as the
//! topmost layer of every frame, after the post effects.
//!
//! It is an operator option (`reel --brand-mark <png>`), not part of the
//! CreativeIntent or the scene schema, so a project file never carries it.
//!
//! - Size: [`MARK_SHARE`] of the short side, inset [`INSET_SHARE`] from the
//!   chosen corner.
//! - Contrast guard: the mean colour under the mark is compared with the
//!   mark's own mean colour; below 2:1 a backing disc (the project ground at
//!   85 %, or white/black when the ground is no better) is drawn behind it. The
//!   disc's opacity follows the measured contrast continuously
//!   ([`disc_strength`]), so it eases in over a changing picture instead of
//!   switching; across a hard cut it follows the cut.
//! - Corners: planned once per beat from the static text boxes of the beats
//!   and of the caption track. A beat whose corner collides with its text
//!   moves the mark to another corner (the previous beat's first); the move is
//!   a [`CROSSFADE_SECONDS`] crossfade at the beat start, at most one per beat.

use std::path::Path;

use crate::tiny_skia::{
    FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Transform,
};
use motion_core::audio::beat_scenes;
use motion_core::layout_qa::is_decorative;
use motion_core::scene::{Color, Layer, LayerKind, MotionProject};

use crate::RenderError;

/// Mark size as a share of the canvas's short side.
pub const MARK_SHARE: f32 = 0.07;
/// Distance from the canvas edge, as a share of the short side.
pub const INSET_SHARE: f32 = 0.05;
/// A mark wider than this aspect (w/h) is a wordmark: sized by height.
pub const WIDE_ASPECT: f32 = 1.5;
/// Wordmark height as a share of the canvas height.
pub const WORDMARK_HEIGHT_SHARE: f32 = 0.04;
/// Backing disc diameter, in mark sizes.
const DISC_SCALE: f32 = 1.3;
/// Opacity of the backing disc at full strength.
const DISC_ALPHA: f32 = 0.85;
/// Below this contrast ratio the disc is fully on, above `GUARD_FULL` off.
const GUARD_ON: f32 = 1.8;
const GUARD_OFF: f32 = 2.2;
/// Corner crossfade at a beat start.
pub const CROSSFADE_SECONDS: f64 = 0.2;
/// Clear space kept between the mark and text, in mark sizes.
const TEXT_CLEARANCE: f32 = 0.6;

/// Canvas corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    Tl,
    Tr,
    Bl,
    Br,
}

impl Corner {
    pub fn parse(s: &str) -> Option<Corner> {
        match s {
            "tl" => Some(Corner::Tl),
            "tr" => Some(Corner::Tr),
            "bl" => Some(Corner::Bl),
            "br" => Some(Corner::Br),
            _ => None,
        }
    }
}

/// WCAG contrast ratio of two relative luminances.
pub fn ratio(a: f32, b: f32) -> f32 {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
}

/// Opacity factor (0..=1) of the backing disc for a measured contrast ratio:
/// 1 at or below 1.8:1, 0 at or above 2.2:1, smooth between.
pub fn disc_strength(contrast: f32) -> f32 {
    let t = ((GUARD_OFF - contrast) / (GUARD_OFF - GUARD_ON)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lin(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(rgb: [f32; 3]) -> f32 {
    let f = |v: f32| lin(v.round().clamp(0.0, 255.0) as u8);
    0.2126 * f(rgb[0]) + 0.7152 * f(rgb[1]) + 0.0722 * f(rgb[2])
}

/// A planned corner from `start` seconds on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerStop {
    pub start: f64,
    pub corner: Corner,
}

/// The mark and its per-beat corner plan.
pub struct BrandMark {
    mark: Pixmap,
    /// Alpha-weighted mean colour of the mark (0..255 per channel).
    mean: [f32; 3],
    size: f32,
    /// Drawn width (equals `size` for square marks).
    width: f32,
    inset: f32,
    canvas: (f32, f32),
    fps: f64,
    ground: Color,
    plan: Vec<CornerStop>,
}

impl BrandMark {
    /// Load `png` for a `w x h` canvas whose ground colour is `ground`; the mark
    /// sits in `corner` for the whole piece until [`BrandMark::plan_for`].
    pub fn from_png(
        png: &Path,
        corner: Corner,
        canvas: (u32, u32),
        fps: u32,
        ground: Color,
    ) -> Result<Self, RenderError> {
        let mark = Pixmap::load_png(png)
            .map_err(|e| RenderError::Asset(format!("brand mark {}: {e}", png.display())))?;
        let (mut sum, mut wsum) = ([0.0f64; 3], 0.0f64);
        for p in mark.pixels() {
            let a = p.alpha() as f64;
            if a < 1.0 {
                continue;
            }
            // Premultiplied: colour / alpha is the straight colour, weight alpha.
            let c = p.demultiply();
            sum[0] += c.red() as f64 * a;
            sum[1] += c.green() as f64 * a;
            sum[2] += c.blue() as f64 * a;
            wsum += a;
        }
        let mean = if wsum > 0.0 {
            [
                (sum[0] / wsum) as f32,
                (sum[1] / wsum) as f32,
                (sum[2] / wsum) as f32,
            ]
        } else {
            [255.0; 3]
        };
        let short = canvas.0.min(canvas.1) as f32;
        let aspect = mark.width() as f32 / mark.height().max(1) as f32;
        let (size, width) = if aspect > WIDE_ASPECT {
            let h = WORDMARK_HEIGHT_SHARE * canvas.1 as f32;
            (h, h * aspect)
        } else {
            (MARK_SHARE * short, MARK_SHARE * short)
        };
        Ok(BrandMark {
            mark,
            mean,
            size,
            width,
            inset: INSET_SHARE * short,
            canvas: (canvas.0 as f32, canvas.1 as f32),
            fps: fps.max(1) as f64,
            ground,
            plan: vec![CornerStop { start: 0.0, corner }],
        })
    }

    /// Plan one corner per beat: the previous beat's corner, else `default`,
    /// else the first corner whose box clears every text box of the beat and of
    /// the caption track.
    pub fn plan_for(&mut self, project: &MotionProject, default: Corner) {
        let mut caption_boxes: Vec<[f32; 4]> = Vec::new();
        for s in project.scenes.iter().filter(|s| s.id == "captions") {
            text_boxes(&s.layers, (0.0, 0.0), &mut caption_boxes);
        }
        let mut plan: Vec<CornerStop> = Vec::new();
        let mut current = default;
        for (i, scene) in beat_scenes(project).iter().enumerate() {
            let mut boxes = caption_boxes.clone();
            text_boxes(&scene.layers, (0.0, 0.0), &mut boxes);
            let clear = |c: Corner| {
                let r = self.rect(c);
                let pad = TEXT_CLEARANCE * self.size;
                !boxes.iter().any(|b| {
                    r[0] - pad < b[0] + b[2]
                        && r[0] + r[2] + pad > b[0]
                        && r[1] - pad < b[1] + b[3]
                        && r[1] + r[3] + pad > b[1]
                })
            };
            let order = [
                current,
                default,
                Corner::Tr,
                Corner::Tl,
                Corner::Br,
                Corner::Bl,
            ];
            let pick = order.iter().copied().find(|&c| clear(c)).unwrap_or(default);
            if i == 0 || pick != current {
                plan.push(CornerStop {
                    start: if i == 0 { 0.0 } else { scene.start_seconds },
                    corner: pick,
                });
            }
            current = pick;
        }
        if plan.is_empty() {
            plan.push(CornerStop {
                start: 0.0,
                corner: default,
            });
        }
        self.plan = plan;
    }

    /// The corner stops, in time order.
    pub fn plan(&self) -> &[CornerStop] {
        &self.plan
    }

    /// Mark box (x, y, w, h) in `corner`.
    pub fn rect(&self, corner: Corner) -> [f32; 4] {
        let (w, h) = self.canvas;
        let s = self.size;
        let x = match corner {
            Corner::Tl | Corner::Bl => self.inset,
            Corner::Tr | Corner::Br => w - self.inset - self.width,
        };
        let y = match corner {
            Corner::Tl | Corner::Tr => self.inset,
            Corner::Bl | Corner::Br => h - self.inset - s,
        };
        [x, y, self.width, s]
    }

    /// (corner, opacity) pairs drawn at time `t`: one, or two while crossfading.
    pub fn corners_at(&self, t: f64) -> Vec<(Corner, f32)> {
        let mut idx = 0;
        for (i, s) in self.plan.iter().enumerate() {
            if s.start <= t + 1e-9 {
                idx = i;
            }
        }
        let now = self.plan[idx];
        if idx > 0 {
            let x = ((t - now.start) / CROSSFADE_SECONDS).clamp(0.0, 1.0) as f32;
            if x < 1.0 {
                let prev = self.plan[idx - 1].corner;
                let ease = x * x * (3.0 - 2.0 * x);
                return vec![(prev, 1.0 - ease), (now.corner, ease)];
            }
        }
        vec![(now.corner, 1.0)]
    }

    /// Contrast of the mark against the mean colour of `pm` under `corner`.
    pub fn contrast_at(&self, pm: &Pixmap, corner: Corner) -> f32 {
        let r = self.rect(corner);
        let under = mean_rgb(pm, r);
        ratio(luminance(under), luminance(self.mean))
    }

    /// Draw the mark (and its backing disc where the guard asks) at time `t`.
    pub fn draw(&self, pm: &mut Pixmap, t: f64) {
        for (corner, alpha) in self.corners_at(t) {
            if alpha <= 0.0 {
                continue;
            }
            let strength = disc_strength(self.contrast_at(pm, corner));
            let r = self.rect(corner);
            if strength > 0.0 {
                self.draw_disc(pm, r, strength * alpha);
            }
            let sx = r[2] / self.mark.width() as f32;
            let sy = r[3] / self.mark.height() as f32;
            let paint = PixmapPaint {
                opacity: alpha,
                quality: FilterQuality::Bicubic,
                ..PixmapPaint::default()
            };
            pm.draw_pixmap(
                0,
                0,
                self.mark.as_ref(),
                &paint,
                Transform::from_row(sx, 0.0, 0.0, sy, r[0], r[1]),
                None,
            );
        }
    }

    /// Colour of the backing disc: the ground when it contrasts with the mark
    /// by 2:1 or more, else white or black, whichever contrasts more.
    pub fn disc_color(&self) -> Color {
        let ml = luminance(self.mean);
        let g = self.ground;
        let gl = luminance([g.r as f32, g.g as f32, g.b as f32]);
        if ratio(gl, ml) >= 2.0 {
            return Color {
                r: g.r,
                g: g.g,
                b: g.b,
                a: 255,
            };
        }
        let white = ratio(1.0, ml);
        let black = ratio(0.0, ml);
        if white >= black {
            Color {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            }
        } else {
            Color {
                r: 0,
                g: 0,
                b: 0,
                a: 255,
            }
        }
    }

    fn draw_disc(&self, pm: &mut Pixmap, r: [f32; 4], strength: f32) {
        let c = self.disc_color();
        let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let mut pb = PathBuilder::new();
        if r[2] > r[3] * 1.01 {
            // Wordmark: a pill (rounded rectangle) behind it.
            let pad = r[3] * 0.35;
            let (x0, y0, x1, y1) = (r[0] - pad, r[1] - pad, r[0] + r[2] + pad, r[1] + r[3] + pad);
            let k = (y1 - y0) / 2.0;
            pb.move_to(x0 + k, y0);
            pb.line_to(x1 - k, y0);
            pb.quad_to(x1, y0, x1, y0 + k);
            pb.quad_to(x1, y1, x1 - k, y1);
            pb.line_to(x0 + k, y1);
            pb.quad_to(x0, y1, x0, y0 + k);
            pb.quad_to(x0, y0, x0 + k, y0);
            pb.close();
        } else {
            pb.push_circle(cx, cy, DISC_SCALE * r[2] / 2.0);
        }
        let Some(path) = pb.finish() else {
            return;
        };
        let mut paint = Paint::default();
        paint.set_color_rgba8(
            c.r,
            c.g,
            c.b,
            (DISC_ALPHA * strength.clamp(0.0, 1.0) * 255.0) as u8,
        );
        paint.anti_alias = true;
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Mean RGB (0..255) of `pm` inside `r` (clamped to the canvas), sampled on a
/// coarse grid.
fn mean_rgb(pm: &Pixmap, r: [f32; 4]) -> [f32; 3] {
    let (w, h) = (pm.width() as i64, pm.height() as i64);
    let x0 = (r[0].floor() as i64).clamp(0, w - 1);
    let y0 = (r[1].floor() as i64).clamp(0, h - 1);
    let x1 = ((r[0] + r[2]).ceil() as i64).clamp(x0 + 1, w);
    let y1 = ((r[1] + r[3]).ceil() as i64).clamp(y0 + 1, h);
    let step = (((x1 - x0).max(y1 - y0)) / 24).max(1);
    let (mut sum, mut n) = ([0.0f64; 3], 0.0f64);
    let px = pm.pixels();
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let p = px[(y * w + x) as usize].demultiply();
            sum[0] += p.red() as f64;
            sum[1] += p.green() as f64;
            sum[2] += p.blue() as f64;
            n += 1.0;
            x += step;
        }
        y += step;
    }
    if n == 0.0 {
        return [0.0; 3];
    }
    [
        (sum[0] / n) as f32,
        (sum[1] / n) as f32,
        (sum[2] / n) as f32,
    ]
}

/// Static boxes (x, y, w, h) of the non-decorative text layers.
fn text_boxes(layers: &[Layer], origin: (f32, f32), out: &mut Vec<[f32; 4]>) {
    for l in layers {
        if !l.visible {
            continue;
        }
        let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
        let (x0, y0) = (
            origin.0 + l.x - l.anchor_x * w,
            origin.1 + l.y - l.anchor_y * h,
        );
        match &l.kind {
            LayerKind::Group { children } => text_boxes(children, (x0, y0), out),
            LayerKind::Text(_) if !is_decorative(&l.id, &l.kind) => out.push([x0, y0, w, h]),
            _ => {}
        }
    }
}

impl BrandMark {
    /// Frame number to time.
    pub fn time_of(&self, frame: u32) -> f64 {
        frame as f64 / self.fps
    }
}
