//! Subject-aware placement (0.5): internal layout candidates for "one analyzed
//! image + one headline", scored deterministically against the image's
//! measured geometry (alpha bounds, occupancy, head region). The lowest cost
//! wins; ties keep generation order. Nothing here is public and no candidate
//! parameter reaches CreativeIntent. See docs/COMPOSITION_GRAMMAR.md
//! §Subject-aware composition for the cost terms and weights.
//!
//! Coordinates are canvas pixels unless a name says `norm` (fractions of the
//! image).

use crate::assets::{ManifestEntry, NormBox, OccupancyGrid};
use crate::compiler::layout_frame::LayoutFrame;

/// Type column beside the subject (instead of stacked above it). Landscape
/// class, plus wider-than-square canvases of the square class (6:5, 4:3),
/// where a stacked layout leaves no height for the supporting items. The
/// square legacy canvas (aspect 1.0) keeps its stacked layout.
pub(crate) fn side_by_side(frame: &LayoutFrame) -> bool {
    frame.is_landscape() || (frame.is_square() && frame.aspect > 1.05)
}

/// One of today's three canvases. Pre-0.9 layouts that fall short of the
/// responsive floors (e.g. type below `min_type_px`) are frozen there.
pub(crate) fn is_legacy_canvas(frame: &LayoutFrame) -> bool {
    LayoutFrame::legacy_size(frame.class) == (frame.w as u32, frame.h as u32)
}

/// Compression factor (<= 1) for a vertical stack whose `need` (px) exceeds the
/// `avail` (px) left above the safe bottom edge: spacing and card heights
/// scale by it. Exactly 1 on the three legacy canvases (frozen, byte-identical)
/// and whenever the stack already fits; never below 0.55 (past that the
/// builder's own text fitting has to give).
pub(crate) fn stack_fit(frame: &LayoutFrame, avail: f32, need: f32) -> f32 {
    if is_legacy_canvas(frame) || need <= avail || need <= 0.0 {
        1.0
    } else {
        (avail / need).clamp(0.55, 1.0)
    }
}

/// Axis-aligned rectangle, canvas pixels, top-left origin.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn area(&self) -> f32 {
        self.w.max(0.0) * self.h.max(0.0)
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let (x0, y0) = (self.x.max(o.x), self.y.max(o.y));
        let (x1, y1) = (self.right().min(o.right()), self.bottom().min(o.bottom()));
        (x1 > x0 && y1 > y0).then(|| Rect::new(x0, y0, x1 - x0, y1 - y0))
    }
    /// Fraction of `self` inside `o`.
    pub fn frac_inside(&self, o: &Rect) -> f32 {
        if self.area() <= 0.0 {
            return 1.0;
        }
        self.intersect(o).map_or(0.0, |i| i.area()) / self.area()
    }
}

/// Geometry of the delivered image the scorer needs (normalized to the image).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SubjectFacts {
    /// Image width / height.
    pub aspect: f32,
    /// Alpha bounds (whole image when unknown/opaque).
    pub subject: NormBox,
    /// Head region (supplied, else estimated, else top of the subject).
    pub head: NormBox,
    pub grid: Option<OccupancyGrid>,
    /// The subject is cut by the image's bottom edge (half figure): it must
    /// bleed off the canvas bottom, never show its cut edge mid-frame.
    pub bottom_cut: bool,
}

impl SubjectFacts {
    pub fn from_entry(e: &ManifestEntry) -> Self {
        let aspect = if e.width > 0 && e.height > 0 {
            e.width as f32 / e.height as f32
        } else {
            0.75
        };
        let analysis = e.analysis.as_ref();
        let subject = analysis
            .map(|a| a.subject_bounds)
            .or(e.safe_bounds)
            .unwrap_or(NormBox {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            });
        let head = e.head_region().unwrap_or_else(|| {
            // 0.4 manifests: a box around the face anchor, else the top of the subject.
            match e.face_anchor {
                Some(p) => NormBox {
                    x: (p.x - 0.14).max(0.0),
                    y: (p.y - 0.12).max(0.0),
                    width: 0.28,
                    height: 0.22,
                },
                None => NormBox {
                    x: subject.x + 0.25 * subject.width,
                    y: subject.y,
                    width: 0.5 * subject.width,
                    height: 0.22 * subject.height,
                },
            }
        });
        let bottom_cut = analysis.map_or(e.alpha, |a| a.edges.bottom && e.alpha);
        SubjectFacts {
            aspect,
            subject,
            head,
            grid: analysis.map(|a| a.occupancy.clone()),
            bottom_cut,
        }
    }

    /// Fraction of canvas rect `r` covered by subject pixels when the image
    /// occupies canvas rect `img`.
    pub fn coverage(&self, img: &Rect, r: &Rect) -> f32 {
        let Some(i) = r.intersect(img) else {
            return 0.0;
        };
        let norm = NormBox {
            x: (i.x - img.x) / img.w,
            y: (i.y - img.y) / img.h,
            width: i.w / img.w,
            height: i.h / img.h,
        };
        let inside = i.area() / r.area().max(1e-6);
        let density = match &self.grid {
            Some(g) => g.coverage_in(norm),
            // No occupancy: the bbox overlap with a typical 60 % fill.
            None => {
                let s = to_canvas(img, self.subject);
                i.intersect(&s).map_or(0.0, |o| 0.6 * o.area() / i.area())
            }
        };
        inside * density
    }
}

/// A normalized image box placed in canvas rect `img`.
pub(crate) fn to_canvas(img: &Rect, b: NormBox) -> Rect {
    Rect::new(
        img.x + b.x * img.w,
        img.y + b.y * img.h,
        b.width * img.w,
        b.height * img.h,
    )
}

/// Which side of the frame the subject stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    Left,
    Right,
}

/// How the headline and the subject share space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Interlock {
    /// The head rises into the last headline line: the lines sit behind the
    /// subject, which occludes part of the last one.
    Split,
    /// The headline sits behind the subject's upper body (text behind subject).
    Behind,
    /// The headline crosses the subject's torso in front, on paper strips
    /// (text in front of subject; never across the head).
    Front,
    /// The headline occupies the negative space beside the subject; no overlap.
    Beside,
}

/// Where a headline line sits relative to the subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Layering {
    Behind,
    Front,
}

/// One headline line placed on the canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LineBox {
    /// Ink box of the line (cap height band), canvas pixels.
    pub rect: Rect,
    pub layering: Layering,
    /// Front line crossing the subject: drawn on a paper strip.
    pub strip: bool,
}

/// One internal layout.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Candidate {
    pub name: &'static str,
    pub mode: Interlock,
    pub side: Side,
    /// The whole image, canvas pixels (aspect preserved).
    pub image: Rect,
    /// Headline block top and its text column.
    pub text_top: f32,
    pub text_col: Rect,
    pub lines: Vec<LineBox>,
    pub size: f32,
    /// Free zone for the supporting items (primary label, secondary note);
    /// `None` = no clean room (they would collide with the headline).
    pub support: Option<Rect>,
}

impl Candidate {
    pub fn subject(&self, f: &SubjectFacts) -> Rect {
        to_canvas(&self.image, f.subject)
    }
    pub fn head(&self, f: &SubjectFacts) -> Rect {
        to_canvas(&self.image, f.head)
    }
    pub fn text_bottom(&self) -> f32 {
        self.lines
            .iter()
            .map(|l| l.rect.bottom())
            .fold(self.text_top, f32::max)
    }
}

/// Canvas and preferences the scorer weighs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frame {
    pub w: f32,
    pub h: f32,
    pub margin: f32,
    /// Largest headline size the grammar allows (hierarchy reference).
    pub max_size: f32,
    /// Side the variant prefers for the subject (controlled asymmetry).
    pub preferred: Side,
    /// Side the same subject stood on in the previous scene, if carried.
    pub previous: Option<Side>,
}

/// Weights (documented in docs/COMPOSITION_GRAMMAR.md).
pub(crate) mod w {
    pub const FACE_FRONT: f32 = 600.0;
    pub const BACK_OCCLUDED_OK: f32 = 60.0;
    pub const BACK_UNREADABLE: f32 = 180.0;
    pub const BACK_UNREADABLE_LIMIT: f32 = 0.30;
    /// A back line hidden in its interior (both ends visible) is unreadable.
    pub const BACK_INTERIOR_MIN: f32 = 0.04;
    pub const LINE_END_ZONE: f32 = 0.12;
    /// Most of a back line's width the subject may hide at its ends.
    pub const BACK_HIDDEN_MAX: f32 = 0.12;
    pub const BODY_FRONT: f32 = 10.0;
    pub const TEXT_CLIP: f32 = 400.0;
    pub const HEAD_CLIP: f32 = 600.0;
    pub const SUBJECT_SIDE_CLIP: f32 = 120.0;
    pub const FLOATING_CUT: f32 = 150.0;
    pub const FEET_CLIP: f32 = 100.0;
    pub const BALANCE: f32 = 300.0;
    pub const LOWER_THIRD: f32 = 200.0;
    pub const LOWER_THIRD_TARGET: f32 = 0.2;
    pub const HIERARCHY: f32 = 140.0;
    pub const SIDE_PREFERENCE: f32 = 12.0;
    pub const CONTINUITY: f32 = 10.0;
    pub const TEXT_OVER_SUBJECT_COLUMN: f32 = 40.0;
    pub const NO_SUPPORT_ROOM: f32 = 90.0;
    pub const SUPPORT_ON_SUBJECT: f32 = 25.0;
    pub const DEAD_BAND: f32 = 250.0;
    pub const DEAD_BAND_LIMIT: f32 = 0.2;
}

/// Cost terms of one candidate (lower is better).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Score {
    pub total: f32,
    pub terms: Vec<(&'static str, f32)>,
}

impl Score {
    #[cfg(test)]
    pub fn term(&self, name: &str) -> f32 {
        self.terms
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .sum()
    }
}

/// Mode prior: a tiny, documented tie-breaker (the interlock is the grammar's idea).
fn prior(m: Interlock) -> f32 {
    match m {
        Interlock::Split => 0.0,
        Interlock::Beside => 4.0,
        Interlock::Front => 6.0,
        Interlock::Behind => 8.0,
    }
}

pub(crate) fn score(c: &Candidate, f: &SubjectFacts, fr: &Frame) -> Score {
    let mut t: Vec<(&'static str, f32)> = Vec::new();
    let canvas = Rect::new(0.0, 0.0, fr.w, fr.h);
    let head = c.head(f);
    let subject = c.subject(f);

    // Face / head: never under front text; back lines stay readable.
    let mut face = 0.0;
    let mut back = 0.0;
    let mut body = 0.0;
    for l in &c.lines {
        match l.layering {
            Layering::Front => {
                let hit = l.rect.intersect(&head).map_or(0.0, |i| i.area());
                face += w::FACE_FRONT * hit / head.area().max(1.0);
                body += w::BODY_FRONT * f.coverage(&c.image, &l.rect);
            }
            Layering::Behind => {
                let occ = f.coverage(&c.image, &l.rect);
                // Where the subject cuts the line matters: an overlap that
                // leaves both ends of the line visible breaks words mid-line.
                const N: usize = 24;
                let cw = l.rect.w / N as f32;
                let hidden: Vec<usize> = (0..N)
                    .filter(|&k| {
                        let col = Rect::new(l.rect.x + k as f32 * cw, l.rect.y, cw, l.rect.h);
                        f.coverage(&c.image, &col) > 0.3
                    })
                    .collect();
                let end = (w::LINE_END_ZONE * N as f32).ceil() as usize;
                let interior = occ > w::BACK_INTERIOR_MIN
                    && match (hidden.first(), hidden.last()) {
                        (Some(&a), Some(&b)) => a >= end && b + end < N,
                        _ => false,
                    };
                // Tail/head overlap is editorial, but only a sliver of a line.
                let too_much = hidden.len() as f32 / N as f32 > w::BACK_HIDDEN_MAX;
                back += if interior || too_much {
                    w::BACK_UNREADABLE
                } else if occ > w::BACK_UNREADABLE_LIMIT {
                    w::BACK_UNREADABLE + w::BACK_UNREADABLE * (occ - w::BACK_UNREADABLE_LIMIT)
                } else {
                    w::BACK_OCCLUDED_OK * occ
                };
            }
        }
    }
    t.push(("face_collision", face));
    t.push(("back_occlusion", back));
    t.push(("subject_collision", body));

    // Clipping: text inside the live area, head inside the canvas, the
    // subject cut only where the image itself cuts it.
    let live = Rect::new(0.5 * fr.margin, 0.09 * fr.h, fr.w - fr.margin, 0.87 * fr.h);
    let text_clip: f32 = c
        .lines
        .iter()
        .map(|l| (1.0 - l.rect.frac_inside(&live)).max(0.0))
        .sum();
    t.push(("text_clip", w::TEXT_CLIP * text_clip));
    let head_live = Rect::new(0.0, 0.02 * fr.h, fr.w, 0.98 * fr.h);
    t.push((
        "head_clip",
        w::HEAD_CLIP * (1.0 - head.frac_inside(&head_live)).max(0.0),
    ));
    let side_out = (0.0 - subject.x).max(0.0) + (subject.right() - fr.w).max(0.0);
    t.push((
        "subject_side_clip",
        w::SUBJECT_SIDE_CLIP * side_out / subject.w.max(1.0),
    ));
    if f.bottom_cut {
        let floating = subject.bottom() < fr.h - 2.0;
        t.push(("floating_cut", if floating { w::FLOATING_CUT } else { 0.0 }));
    } else {
        let below = (subject.bottom() - fr.h).max(0.0);
        t.push(("feet_clip", w::FEET_CLIP * below / subject.h.max(1.0)));
    }

    // Balance: visual mass (text boxes + visible subject) off-centre, but not extreme.
    let visible = subject.intersect(&canvas);
    let (mut mass, mut mx, mut low) = (0.0f32, 0.0f32, 0.0f32);
    for l in &c.lines {
        mass += l.rect.area();
        mx += l.rect.area() * l.rect.cx();
        low += l
            .rect
            .intersect(&Rect::new(0.0, 2.0 * fr.h / 3.0, fr.w, fr.h / 3.0))
            .map_or(0.0, |i| i.area());
    }
    if let Some(v) = visible {
        let sm = 0.6 * v.area();
        mass += sm;
        mx += sm * v.cx();
        low += 0.6
            * v.intersect(&Rect::new(0.0, 2.0 * fr.h / 3.0, fr.w, fr.h / 3.0))
                .map_or(0.0, |i| i.area());
    }
    let d = if mass > 0.0 {
        (mx / mass - fr.w / 2.0).abs() / fr.w
    } else {
        0.0
    };
    let balance = if d < 0.03 {
        (0.03 - d) * w::BALANCE
    } else if d > 0.18 {
        (d - 0.18) * w::BALANCE
    } else {
        0.0
    };
    t.push(("balance", balance));
    let low_frac = if mass > 0.0 { low / mass } else { 0.0 };
    t.push((
        "lower_third",
        w::LOWER_THIRD * (w::LOWER_THIRD_TARGET - low_frac).max(0.0),
    ));

    // Hierarchy: the headline stays large.
    t.push((
        "hierarchy",
        w::HIERARCHY * (1.0 - c.size / fr.max_size).clamp(0.0, 1.0),
    ));
    // Text column beside the subject should not sit over the subject column.
    if c.mode == Interlock::Beside {
        let over = c.text_col.intersect(&subject).map_or(0.0, |i| i.w) / c.text_col.w.max(1.0);
        t.push(("text_over_subject", w::TEXT_OVER_SUBJECT_COLUMN * over));
    }
    t.push((
        "side_preference",
        if c.side == fr.preferred {
            0.0
        } else {
            w::SIDE_PREFERENCE
        },
    ));
    t.push((
        "continuity",
        match fr.previous {
            Some(p) if p != c.side => w::CONTINUITY,
            _ => 0.0,
        },
    ));
    // Supporting items need clean room; a zone over the subject needs a card.
    match c.support {
        None => t.push(("support_room", w::NO_SUPPORT_ROOM)),
        Some(z) => {
            t.push((
                "support_room",
                w::SUPPORT_ON_SUBJECT * (f.coverage(&c.image, &z) / 0.2).min(1.0),
            ));
            let hit = z.intersect(&head).map_or(0.0, |i| i.area());
            t.push(("support_face", w::FACE_FRONT * hit / head.area().max(1.0)));
        }
    }
    // Vertical dead space: the largest empty band between the kicker line
    // and the bottom (text, support zone and visible subject occupy space).
    let mut spans: Vec<(f32, f32)> = c
        .lines
        .iter()
        .map(|l| (l.rect.y, l.rect.bottom()))
        .collect();
    if let Some(z) = c.support {
        spans.push((z.y, z.bottom()));
    }
    if let Some(v) = visible {
        spans.push((v.y, v.bottom()));
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut cursor, mut gap) = (0.13 * fr.h, 0.0f32);
    for (a, b) in spans {
        gap = gap.max(a - cursor);
        cursor = cursor.max(b);
    }
    gap = gap.max(fr.h - cursor);
    t.push((
        "dead_band",
        w::DEAD_BAND * (gap / fr.h - w::DEAD_BAND_LIMIT).max(0.0),
    ));
    t.push(("mode_prior", prior(c.mode)));

    Score {
        total: t.iter().map(|(_, v)| v).sum(),
        terms: t,
    }
}

/// The best candidate (lowest total; the earliest wins ties). `None` when empty.
pub(crate) fn choose(
    cands: Vec<Candidate>,
    f: &SubjectFacts,
    fr: &Frame,
) -> Option<(Candidate, Score)> {
    let mut best: Option<(Candidate, Score)> = None;
    for c in cands {
        let s = score(&c, f, fr);
        if std::env::var_os("MOTION_DEBUG_LAYOUT").is_some() {
            let terms: Vec<String> = s
                .terms
                .iter()
                .filter(|(_, v)| *v > 0.05)
                .map(|(n, v)| format!("{n}={v:.1}"))
                .collect();
            eprintln!(
                "{:>13} {:?} total={:.1} {}",
                c.name,
                c.side,
                s.total,
                terms.join(" ")
            );
        }
        let better = match &best {
            Some((_, b)) => s.total < b.total - 1e-3,
            None => true,
        };
        if better {
            best = Some((c, s));
        }
    }
    best
}

/// A free band of height `need` in column `col` for the supporting items:
/// below the headline if it fits above `floor`, else above it (below
/// `ceiling`). Never overlaps a headline line.
pub(crate) fn support_zone(
    lines: &[LineBox],
    col: Rect,
    need: f32,
    gap: f32,
    ceiling: f32,
    floor: f32,
) -> Option<Rect> {
    let top = lines.iter().map(|l| l.rect.y).fold(f32::MAX, f32::min);
    let bottom = lines
        .iter()
        .map(|l| l.rect.bottom())
        .fold(f32::MIN, f32::max);
    let below = bottom + gap;
    if below + need <= floor {
        return Some(Rect::new(col.x, below, col.w, need));
    }
    let above = top - gap - need;
    (above >= ceiling).then(|| Rect::new(col.x, above, col.w, need))
}

/// Image rect (aspect preserved) such that the image's *subject bounds* —
/// not its transparent margins — fill `target` (contain) and are centred in
/// it. Use it to size a delivered cutout by what it depicts.
pub(crate) fn subject_fit(f: &SubjectFacts, target: Rect) -> Rect {
    let s = f.subject;
    let (sw, sh) = (s.width.max(0.05), s.height.max(0.05));
    // Image height H: subject box = (sw·H·aspect) × (sh·H).
    let ih = (target.w / (sw * f.aspect)).min(target.h / sh);
    let iw = ih * f.aspect;
    let x = target.cx() - iw * (s.x + sw / 2.0);
    let y = target.y + target.h / 2.0 - ih * (s.y + sh / 2.0);
    Rect::new(x, y, iw, ih)
}

/// Image rect for a subject whose head top lands at `head_top` and whose
/// subject bottom lands at `bottom` (canvas y), centred horizontally on
/// `cx`. Height clamped to `[min_h, max_h]` (then bottom-anchored).
#[allow(clippy::too_many_arguments)]
pub(crate) fn fit_between(
    f: &SubjectFacts,
    head_top: Option<f32>,
    bottom: f32,
    fixed_h: Option<f32>,
    cx: f32,
    min_h: f32,
    max_h: f32,
) -> Rect {
    let s = f.subject;
    let top_frac = f.head.y.min(s.y);
    let span = (s.y + s.height - top_frac).max(0.05);
    let ih = match (fixed_h, head_top) {
        (Some(h), _) => h,
        (None, Some(ht)) => (bottom - ht) / span,
        (None, None) => min_h,
    }
    .clamp(min_h, max_h);
    let iw = ih * f.aspect;
    let y = bottom - ih * (s.y + s.height);
    let x = cx - iw * (s.x + s.width / 2.0);
    Rect::new(x, y, iw, ih)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::{AssetAnalysis, EdgeContact};

    fn facts() -> SubjectFacts {
        // A centred half figure: head in the top quarter, body below, cut at the bottom.
        let mut rows = Vec::new();
        for r in 0..16 {
            let row: String = (0..16)
                .map(|c| {
                    let head = r < 4 && (6..10).contains(&c);
                    let body = r >= 5 && (2..14).contains(&c);
                    if head || body {
                        'f'
                    } else {
                        '0'
                    }
                })
                .collect();
            rows.push(row);
        }
        SubjectFacts {
            aspect: 0.75,
            subject: NormBox {
                x: 0.125,
                y: 0.0,
                width: 0.75,
                height: 1.0,
            },
            head: NormBox {
                x: 0.375,
                y: 0.0,
                width: 0.25,
                height: 0.25,
            },
            grid: Some(OccupancyGrid { cols: 16, rows }),
            bottom_cut: true,
        }
    }

    fn frame() -> Frame {
        Frame {
            w: 1080.0,
            h: 1920.0,
            margin: 84.0,
            max_size: 170.0,
            preferred: Side::Right,
            previous: None,
        }
    }

    fn cand(mode: Interlock, image: Rect, line: Rect, layering: Layering) -> Candidate {
        Candidate {
            name: "t",
            mode,
            side: Side::Right,
            image,
            text_top: line.y,
            text_col: line,
            lines: vec![LineBox {
                rect: line,
                layering,
                strip: false,
            }],
            size: 150.0,
            support: Some(Rect::new(84.0, 1500.0, 500.0, 200.0)),
        }
    }

    #[test]
    fn front_text_over_the_face_costs_more_than_text_below_it() {
        let f = facts();
        let img = Rect::new(400.0, 800.0, 600.0, 1120.0);
        let head = to_canvas(&img, f.head);
        let over_face = cand(
            Interlock::Front,
            img,
            Rect::new(100.0, head.y + 20.0, 900.0, 120.0),
            Layering::Front,
        );
        let below = cand(
            Interlock::Front,
            img,
            Rect::new(100.0, head.bottom() + 150.0, 900.0, 120.0),
            Layering::Front,
        );
        let (a, b) = (score(&over_face, &f, &frame()), score(&below, &f, &frame()));
        assert!(a.term("face_collision") > 100.0, "{a:?}");
        assert_eq!(b.term("face_collision"), 0.0);
        assert!(a.total > b.total);
    }

    #[test]
    fn a_visible_bottom_cut_is_penalized_and_bleeding_is_free() {
        let f = facts();
        let line = Rect::new(84.0, 330.0, 600.0, 120.0);
        let floating = cand(
            Interlock::Beside,
            Rect::new(450.0, 600.0, 600.0, 800.0),
            line,
            Layering::Behind,
        );
        let bleeding = cand(
            Interlock::Beside,
            Rect::new(450.0, 1000.0, 600.0, 930.0),
            line,
            Layering::Behind,
        );
        assert_eq!(score(&floating, &f, &frame()).term("floating_cut"), 150.0);
        assert_eq!(score(&bleeding, &f, &frame()).term("floating_cut"), 0.0);
    }

    #[test]
    fn unreadable_back_lines_are_penalized() {
        let f = facts();
        let img = Rect::new(240.0, 700.0, 600.0, 1220.0);
        let body = to_canvas(&img, f.subject);
        let hidden = cand(
            Interlock::Behind,
            img,
            Rect::new(body.x + 60.0, body.y + 0.5 * body.h, 360.0, 100.0),
            Layering::Behind,
        );
        let s = score(&hidden, &f, &frame());
        assert!(s.term("back_occlusion") >= w::BACK_UNREADABLE, "{s:?}");
    }

    #[test]
    fn choose_is_deterministic_and_keeps_the_first_on_ties() {
        let f = facts();
        let img = Rect::new(450.0, 1000.0, 600.0, 930.0);
        let line = Rect::new(84.0, 330.0, 600.0, 120.0);
        let mut a = cand(Interlock::Split, img, line, Layering::Behind);
        a.name = "a";
        let mut b = a.clone();
        b.name = "b";
        let (best, _) = choose(vec![a, b], &f, &frame()).expect("some");
        assert_eq!(best.name, "a");
    }

    #[test]
    fn subject_fit_fills_the_target_with_the_subject_not_the_margins() {
        let mut f = facts();
        f.subject = NormBox {
            x: 0.25,
            y: 0.25,
            width: 0.5,
            height: 0.5,
        };
        f.aspect = 1.0;
        let target = Rect::new(100.0, 100.0, 400.0, 400.0);
        let r = subject_fit(&f, target);
        let s = to_canvas(&r, f.subject);
        assert!((s.x - 100.0).abs() < 0.01 && (s.w - 400.0).abs() < 0.01);
        assert!((s.y - 100.0).abs() < 0.01 && (s.h - 400.0).abs() < 0.01);
        assert!((r.w - 800.0).abs() < 0.01);
    }

    #[test]
    fn fit_between_puts_head_and_bottom_where_asked() {
        let f = facts();
        let r = fit_between(&f, Some(600.0), 1930.0, None, 700.0, 500.0, 2400.0);
        let head = to_canvas(&r, f.head);
        let subject = to_canvas(&r, f.subject);
        assert!((head.y - 600.0).abs() < 0.5);
        assert!((subject.bottom() - 1930.0).abs() < 0.5);
        assert!((subject.cx() - 700.0).abs() < 0.5);
        assert!((r.w / r.h - 0.75).abs() < 1e-4);
    }

    #[test]
    fn facts_from_an_analyzed_entry_prefer_supplied_head_bounds() {
        let e = ManifestEntry {
            id: "beat_1.hero_subject".into(),
            path: "x.png".into(),
            width: 900,
            height: 1200,
            alpha: true,
            head_bounds: Some(NormBox {
                x: 0.4,
                y: 0.05,
                width: 0.2,
                height: 0.2,
            }),
            analysis: Some(AssetAnalysis {
                subject_bounds: NormBox {
                    x: 0.1,
                    y: 0.04,
                    width: 0.8,
                    height: 0.96,
                },
                edges: EdgeContact {
                    bottom: true,
                    ..Default::default()
                },
                coverage: 0.5,
                occupancy: facts().grid.expect("grid"),
                safe_regions: Vec::new(),
                head_estimate: Some(NormBox {
                    x: 0.0,
                    y: 0.0,
                    width: 1.0,
                    height: 0.5,
                }),
                monochrome: false,
                mean_color: None,
            }),
            ..Default::default()
        };
        let f = SubjectFacts::from_entry(&e);
        assert_eq!(f.head.x, 0.4);
        assert!(f.bottom_cut);
        assert!((f.aspect - 0.75).abs() < 1e-6);
    }
}
