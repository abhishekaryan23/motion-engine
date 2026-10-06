//! (0.9) LayoutFrame: everything a builder needs to know about the canvas,
//! derived from (W, H) only. Operator input (`compile --canvas WxH` /
//! `--aspect R`), never authored by weak models; the intent `format` stays the
//! default canvas.
//!
//! Contract (frozen 0.9 Phase 3):
//! * `u` = short side / 1080 (unchanged unit), `aspect` = W / H, allowed range
//!   0.4..=2.5.
//! * `class` = the nearest legacy layout family by log-aspect: aspect < 0.75 →
//!   Vertical, 0.75..=1.3334 → Square, > 1.3334 → Landscape. Builders branch on
//!   `class` where they used `w > h` / `w == h` / `Format`, so the three legacy
//!   canvases (1080×1920, 1080×1080, 1920×1080) reproduce today's numbers exactly.
//! * `weights` are continuous (tall, square, wide) memberships, piecewise
//!   linear in ln(aspect) between the anchors 9:16, 1:1 and 16:9 (each anchor
//!   has weight 1 for its own class). Constants that differ per class are
//!   written `frame.blend(tall, square, wide)`: exact at the legacy anchors,
//!   smooth in between.
//! * `safe` is the canvas inset by the margin (84u, today's `Ctx::margin`).
//!   `regions` are QA zones (`qa --layout`), not placement rules.

use serde::{Deserialize, Serialize};

use crate::intent::Format;

/// Allowed canvas aspect range (W / H).
pub const MIN_ASPECT: f32 = 0.4;
pub const MAX_ASPECT: f32 = 2.5;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, o: &Rect) -> bool {
        o.x >= self.x - 0.5
            && o.y >= self.y - 0.5
            && o.x + o.w <= self.x + self.w + 0.5
            && o.y + o.h <= self.y + self.h + 0.5
    }
}

/// Continuous aspect-class memberships (sum = 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AspectWeights {
    pub tall: f32,
    pub square: f32,
    pub wide: f32,
}

/// QA zones (normalised to the safe area by construction).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Regions {
    pub headline: Rect,
    pub subject: Rect,
    pub caption: Rect,
    /// Top / bottom furniture lanes (labels, page numbers).
    pub furniture_top: Rect,
    pub furniture_bottom: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutFrame {
    pub w: f32,
    pub h: f32,
    pub u: f32,
    pub aspect: f32,
    pub class: Format,
    pub weights: AspectWeights,
    pub safe: Rect,
    pub regions: Regions,
    /// Smallest legible text size on this canvas (px).
    pub min_type_px: f32,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum CanvasError {
    #[error("canvas {0}x{1}: aspect {2:.3} outside {MIN_ASPECT}..={MAX_ASPECT}")]
    Aspect(u32, u32, f32),
    #[error("canvas {0}x{1}: sides must be even and the short side >= 320")]
    Size(u32, u32),
}

/// Tallest canvas the composition budget is designed for (units of `u`).
pub const DESIGN_HEIGHT_U: f32 = 1920.0;
/// Share of the extra height of a taller canvas that goes above the content
/// block (the rest below): the composition centres slightly high.
pub const TALL_TOP_SHARE: f32 = 0.45;

const A_TALL: f32 = 0.5625;
const A_WIDE: f32 = 16.0 / 9.0;

impl LayoutFrame {
    /// Today's canvas for an intent format.
    pub fn legacy_size(format: Format) -> (u32, u32) {
        match format {
            Format::Vertical => (1080, 1920),
            Format::Square => (1080, 1080),
            Format::Landscape => (1920, 1080),
        }
    }

    pub fn for_format(format: Format) -> Self {
        let (w, h) = Self::legacy_size(format);
        Self::new(w, h).expect("legacy canvas is valid")
    }

    pub fn new(w: u32, h: u32) -> Result<Self, CanvasError> {
        if !w.is_multiple_of(2) || !h.is_multiple_of(2) || w.min(h) < 320 {
            return Err(CanvasError::Size(w, h));
        }
        let aspect = w as f32 / h as f32;
        if !(MIN_ASPECT..=MAX_ASPECT).contains(&aspect) {
            return Err(CanvasError::Aspect(w, h, aspect));
        }
        let (wf, hf) = (w as f32, h as f32);
        let u = wf.min(hf) / 1080.0;
        Ok(Self::derive(wf, hf, u, 20.0 * u))
    }

    /// Everything derived from the canvas size `(w, h)` (aspect, class,
    /// weights, safe area, QA regions) for a given unit and type floor. The
    /// aspect is not range-checked here (the weights saturate at the anchors).
    fn derive(wf: f32, hf: f32, u: f32, min_type_px: f32) -> Self {
        let aspect = wf / hf;
        let class = if aspect < 0.75 {
            Format::Vertical
        } else if aspect <= 1.3334 {
            Format::Square
        } else {
            Format::Landscape
        };
        let la = aspect.ln();
        let weights = if la <= A_TALL.ln() {
            AspectWeights {
                tall: 1.0,
                square: 0.0,
                wide: 0.0,
            }
        } else if la < 0.0 {
            let t = (la - A_TALL.ln()) / (0.0 - A_TALL.ln());
            AspectWeights {
                tall: 1.0 - t,
                square: t,
                wide: 0.0,
            }
        } else if la < A_WIDE.ln() {
            let t = la / A_WIDE.ln();
            AspectWeights {
                tall: 0.0,
                square: 1.0 - t,
                wide: t,
            }
        } else {
            AspectWeights {
                tall: 0.0,
                square: 0.0,
                wide: 1.0,
            }
        };
        let m = 84.0 * u;
        let safe = Rect {
            x: m,
            y: m,
            w: wf - 2.0 * m,
            h: hf - 2.0 * m,
        };
        let band = |y0: f32, y1: f32| Rect {
            x: safe.x,
            y: safe.y + y0 * safe.h,
            w: safe.w,
            h: (y1 - y0) * safe.h,
        };
        let regions = Regions {
            headline: band(0.0, 0.5),
            subject: band(0.2, 0.95),
            caption: band(0.6, 1.0),
            furniture_top: band(0.0, 0.08),
            furniture_bottom: band(0.92, 1.0),
        };
        LayoutFrame {
            w: wf,
            h: hf,
            u,
            aspect,
            class,
            weights,
            safe,
            regions,
            min_type_px,
        }
    }

    /// (0.10 Q) The canvas with its bottom `px` pixels handed to something
    /// else (the caption lane): the frame builders lay beat content out on. The
    /// width, the unit `u` and the type floor stay those of the real canvas
    /// (type keeps its size); the height, aspect, class, weights, safe area and
    /// regions are those of the remaining `w x (h - px)` area, so a builder's
    /// "bottom" (`h`, `safe`) is the top of the reserved band. Furniture and
    /// captions keep using the real frame. `px <= 0` returns the frame as is;
    /// the usable height never drops below a third of the canvas.
    pub fn with_bottom_reserve(&self, px: f32) -> Self {
        if px <= 0.0 {
            return *self;
        }
        let h = (self.h - px).max(self.h / 3.0);
        Self::derive(self.w, h, self.u, self.min_type_px)
    }

    pub fn is_landscape(&self) -> bool {
        self.class == Format::Landscape
    }

    pub fn is_square(&self) -> bool {
        self.class == Format::Square
    }

    /// Height builders lay content out on: the real height up to 1920u, else
    /// 1920u (a taller canvas gets the same vertical budget as 9:16, not a
    /// stretched one). Equal to `h` on every canvas with `h <= 1920u`.
    pub fn content_h(&self) -> f32 {
        self.h.min(DESIGN_HEIGHT_U * self.u)
    }

    /// Y offset of the content block on the real canvas: [`TALL_TOP_SHARE`] of
    /// the height beyond [`content_h`](Self::content_h) goes above it. Zero
    /// whenever `h <= 1920u`. Furniture and edge lanes ignore it (they stay on
    /// the real canvas edges).
    pub fn content_y0(&self) -> f32 {
        (self.h - self.content_h()) * TALL_TOP_SHARE
    }

    /// Per-class constant, exact at the legacy anchors, blended in between.
    pub fn blend(&self, tall: f32, square: f32, wide: f32) -> f32 {
        self.weights.tall * tall + self.weights.square * square + self.weights.wide * wide
    }
}

/// Named aspect presets for `--aspect` (short side 1080, long side rounded to even).
pub fn preset_size(name: &str) -> Option<(u32, u32)> {
    let ratio = match name {
        "story" | "9:16" => (9.0, 16.0),
        "portrait" | "4:5" => (4.0, 5.0),
        "square" | "1:1" => (1.0, 1.0),
        "landscape" | "16:9" => (16.0, 9.0),
        "cinema" | "21:9" => (21.0, 9.0),
        "tall" | "fold_cover" | "9:21" => (9.0, 21.0),
        "tablet" | "3:4" => (3.0, 4.0),
        "fold_inner" | "6:5" => (6.0, 5.0),
        other => {
            let (a, b) = other.split_once(':')?;
            (a.trim().parse::<f32>().ok()?, b.trim().parse::<f32>().ok()?)
        }
    };
    let (rw, rh): (f32, f32) = ratio;
    if rw <= 0.0 || rh <= 0.0 {
        return None;
    }
    let even = |v: f32| ((v / 2.0).round() as u32) * 2;
    Some(if rw <= rh {
        (1080, even(1080.0 * rh / rw))
    } else {
        (even(1080.0 * rw / rh), 1080)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_anchors_are_pure_classes() {
        let v = LayoutFrame::for_format(Format::Vertical);
        assert_eq!((v.class, v.weights.tall, v.u), (Format::Vertical, 1.0, 1.0));
        let s = LayoutFrame::for_format(Format::Square);
        assert_eq!((s.class, s.weights.square), (Format::Square, 1.0));
        let l = LayoutFrame::for_format(Format::Landscape);
        assert_eq!((l.class, l.weights.wide), (Format::Landscape, 1.0));
        assert_eq!(v.blend(1.0, 2.0, 3.0), 1.0);
        assert_eq!(s.blend(1.0, 2.0, 3.0), 2.0);
        assert_eq!(l.blend(1.0, 2.0, 3.0), 3.0);
    }

    #[test]
    fn presets_and_classes() {
        assert_eq!(preset_size("story"), Some((1080, 1920)));
        assert_eq!(preset_size("portrait"), Some((1080, 1350)));
        assert_eq!(preset_size("cinema"), Some((2520, 1080)));
        assert_eq!(preset_size("tall"), Some((1080, 2520)));
        assert_eq!(preset_size("fold_inner"), Some((1296, 1080)));
        let c = |n| {
            let (w, h) = preset_size(n).unwrap();
            LayoutFrame::new(w, h).unwrap().class
        };
        assert_eq!(c("portrait"), Format::Square);
        assert_eq!(c("tablet"), Format::Square);
        assert_eq!(c("tall"), Format::Vertical);
        assert_eq!(c("cinema"), Format::Landscape);
        assert!(LayoutFrame::new(1080, 3000).is_err());
        assert!(LayoutFrame::new(1081, 1080).is_err());
    }

    #[test]
    fn tall_canvas_budget_is_identity_up_to_1920u() {
        for (w, h) in [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)] {
            let f = LayoutFrame::new(w, h).unwrap();
            assert_eq!(f.content_h(), f.h, "{w}x{h}");
            assert_eq!(f.content_y0(), 0.0, "{w}x{h}");
        }
        // Tall: 9:21 lays out on 1920u and sits 45 % of the extra height down.
        let t = LayoutFrame::new(1080, 2520).unwrap();
        assert_eq!(t.content_h(), 1920.0);
        assert!((t.content_y0() - 600.0 * 0.45).abs() < 1e-3);
        // The unit scales the design height with the canvas.
        let big = LayoutFrame::new(1620, 3780).unwrap();
        assert!((big.content_h() - 1920.0 * 1.5).abs() < 1e-3);
    }

    #[test]
    fn weights_are_continuous() {
        for w in (1080..=2600).step_by(20) {
            let f = LayoutFrame::new(w, 1080).unwrap().weights;
            assert!((f.tall + f.square + f.wide - 1.0).abs() < 1e-5);
        }
    }
}
