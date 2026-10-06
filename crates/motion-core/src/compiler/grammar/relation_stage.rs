//! (0.19) RelationStage — two pictures and how they relate.
//!
//! A `compare` / `contrast` beat whose primary and secondary are both pictures
//! (objects) used to lose its relationship in the cinematic look: the second
//! picture was only a dim, blurred prop. The stage shows both pictures on the
//! focus plane at a size that fits their shapes, each with its label (the
//! subject's `meaning`) and stamped figure (its `value`), and draws the beat's
//! `relationship` between them. No new intent vocabulary: purpose, relationship,
//! meaning and value are the existing fields.
//!
//! | relationship | what the viewer sees |
//! |---|---|
//! | `separate` (compare default) | a divider with a VS disc between the pictures |
//! | `replace` | an arrow from the first to the second; the first steps back and dims as the second arrives |
//! | `grow` | an arrow from the first to the second; the second grows |
//! | `compress` (contrast default) | an arrow from the second onto the first; the first shrinks under it |
//! | `carry` | a link line between them; both stay |
//! | `accumulate` | a plus disc between them |
//!
//! Layout. The free band under the title is split either side by side or
//! stacked, whichever lets the two pictures be larger for their actual shapes
//! (equal visual area, weighted by the relationship); a connector zone sits
//! between them and each picture has a label pill below it (and a stamp above
//! it when it has a `value`).
//!
//! Timing. The first picture enters with ENTER from one side; the second enters
//! from the opposite side when the narrator names it (else at EVOLVE); the
//! connector draws once the second has landed. Everything sits on the focus
//! plane (`z = 0`). A small node at the middle of the stage pops in with the
//! first picture and is the beat's focal layer, so the camera pivots on the
//! middle of the relation and the focal layer is drawn while it is read.
//!
//! (0.22) The same stage also serves every flat look ([`Stage::Flat`], via
//! the `StatPair` grammar): no depth, and each picture arrives with a fade and
//! a short slide from its side instead of the cinematic arrival. Two pictures
//! with values are then a co-equal stat pair in every look, as the intent
//! contract promises ("a short figure stamped next to the object").

use super::cinematic3d;
use super::placement::{self, Rect, SubjectFacts};
use super::plate;
use crate::assets::AssetRole;
use crate::compiler::explore::contrast_ratio;
use crate::compiler::recipes::B;
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mo, round3, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::{Beat, Purpose, Relationship, Subject, SubjectKind};
use crate::scene::{
    Color, GlyphOrder, GlyphPose, Layer, LayerKind, Motion, MotionOp, SpringSpec, Stroke, TextAlign,
};

const PIC_Z_INDEX: i32 = 12;
const DEVICE_Z_INDEX: i32 = 13;
const LABEL_Z_INDEX: i32 = 14;

const POP: SpringSpec = SpringSpec {
    stiffness: 260.0,
    damping: 18.0,
    mass: 1.0,
};

/// Largest side of a picture, as a share of the short side.
const MAX_SIDE_SHARE: f32 = 0.62;
/// How far a label pill reaches up over the picture's lower edge (share of the region width).
const LABEL_OVERLAP: f32 = 0.018;
/// A beat shorter than this (seconds) brings its second picture in early.
const SHORT_BEAT: f64 = 5.0;
/// Space kept free of the canvas edges (the camera's push and orbit reach it).
const EDGE_SHARE: f32 = 0.05;

/// Which look the stage is built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stage {
    /// Staged in depth for the cinematic camera (the 0.19 behaviour).
    Cinematic,
    /// (0.22) A flat look: no depth, flat entrances.
    Flat,
}

impl Stage {
    /// The depth its layers carry (`None` = a flat layer).
    fn depth(self) -> Option<f32> {
        match self {
            Stage::Cinematic => Some(cinematic3d::Z_HERO),
            Stage::Flat => None,
        }
    }
}

/// How the two pictures are arranged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// Side by side: the connector runs left-right.
    Across,
    /// Stacked: the connector runs top-bottom.
    Down,
}

/// What is drawn between the pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Device {
    /// Divider line and a VS disc.
    Versus,
    /// Arrow from the first picture to the second.
    ArrowForward,
    /// Arrow from the second picture onto the first.
    ArrowBack,
    /// A plain link line.
    Link,
    /// A plus disc.
    Plus,
    /// (0.23) Nothing: two pictures that each carry a figure in a beat that
    /// is neither a comparison nor has a relationship of its own (an emphasize
    /// beat of two valued pictures): they are shown side by side, not versus.
    None,
}

/// What happens to the pictures once both are on stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Play {
    None,
    /// The first steps back and dims; the second comes forward.
    Replace,
    /// The second grows.
    Grow,
    /// The first shrinks under the second.
    Compress,
}

fn relation(beat: &crate::intent::Beat) -> (Device, Play, [f32; 2]) {
    // (0.23) A "VS" belongs to a comparison: any other purpose without a
    // relationship of its own draws no connector.
    if beat.relationship.is_none() && !matches!(beat.purpose, Purpose::Compare | Purpose::Contrast)
    {
        return (Device::None, Play::None, [1.0, 1.0]);
    }
    let rel = beat.relationship.unwrap_or(match beat.purpose {
        Purpose::Contrast => Relationship::Compress,
        _ => Relationship::Separate,
    });
    match rel {
        Relationship::Separate => (Device::Versus, Play::None, [1.0, 1.0]),
        Relationship::Replace => (Device::ArrowForward, Play::Replace, [0.8, 1.2]),
        Relationship::Grow => (Device::ArrowForward, Play::Grow, [0.8, 1.35]),
        Relationship::Compress => (Device::ArrowBack, Play::Compress, [0.85, 1.25]),
        Relationship::Carry => (Device::Link, Play::None, [1.0, 1.0]),
        Relationship::Accumulate => (Device::Plus, Play::None, [1.0, 1.0]),
    }
}

/// A picture of the pair.
struct Pic {
    which: Which,
    subject: Subject,
    roles: &'static [AssetRole],
    /// Width / height of the picture's alpha bounds.
    aspect: f32,
}

fn resolve(ctx: &Ctx, b: &B, which: Which) -> Option<Pic> {
    let (subject, roles): (Subject, &'static [AssetRole]) = match which {
        Which::Primary => (
            b.beat.primary.clone(),
            &[
                AssetRole::HeroSubject,
                AssetRole::Portrait,
                AssetRole::HeroObject,
            ],
        ),
        Which::Secondary => (b.beat.secondary.clone()?, &[AssetRole::SupportingObject]),
    };
    let Subject::Object(o) = &subject else {
        return None;
    };
    let aspect = if let Some((entry, _)) = cinematic3d::delivered(ctx, b.plan.index, roles) {
        let f = SubjectFacts::from_entry(&entry);
        (f.subject.width * f.aspect / f.subject.height.max(0.05)).clamp(0.2, 5.0)
    } else if ctx.library.find_object(&o.asset).is_some() {
        1.0
    } else {
        return None;
    };
    Some(Pic {
        which,
        subject,
        roles,
        aspect,
    })
}

/// "brain_circuit" -> "Brain circuit".
fn humanise(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|c| if matches!(c, '_' | '-') { ' ' } else { c })
        .collect();
    let mut chars = spaced.trim().chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn label_of(s: &Subject) -> String {
    let Subject::Object(o) = s else {
        return String::new();
    };
    o.meaning
        .clone()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| humanise(&o.asset))
}

fn value_of(s: &Subject) -> Option<String> {
    let Subject::Object(o) = s else {
        return None;
    };
    o.value.clone().filter(|v| !v.trim().is_empty())
}

/// The sizes and places of the two pictures and of the connector zone.
#[derive(Debug, Clone, Copy)]
struct Arrangement {
    axis: Axis,
    /// Alpha-bounds rectangles of the first and second picture.
    pics: [Rect; 2],
    /// The connector zone between them.
    zone: Rect,
    /// Where the label pill of each picture is centred horizontally / its top.
    label_top: [f32; 2],
}

/// Pick the arrangement that lets the pair be larger. `weights` bias the
/// pictures' areas (the grown picture is larger); `band` is the height a label
/// (and a stamp) needs around a picture.
#[allow(clippy::too_many_arguments)]
fn arrange(
    region: Rect,
    aspects: [f32; 2],
    weights: [f32; 2],
    label_band: f32,
    stamp_band: f32,
    gaps: [f32; 2],
    cap_side: f32,
    stagger: f32,
) -> Arrangement {
    // The connector zone is wider when the pictures are stacked: the disc and
    // the arrow then sit between one picture's label and the next one's stamp.
    let [gap_across, gap_down] = gaps;
    let a = aspects;
    let k = weights;
    let sq = |i: usize| (k[i] * a[i]).sqrt();
    let sq_inv = |i: usize| (k[i] / a[i]).sqrt();
    // Side by side: widths add up, the tallest picture and one label band.
    let across_w = region.w - gap_across;
    let across_h = region.h - label_band - stamp_band;
    let s_across = (across_w / (sq(0) + sq(1))).min(across_h / sq_inv(0).max(sq_inv(1)));
    // Stacked: heights add up with two label bands, the widest picture.
    let down_h = region.h - gap_down - 2.0 * (label_band + stamp_band);
    let s_down = (down_h / (sq_inv(0) + sq_inv(1))).min(region.w / sq(0).max(sq(1)));
    let axis = if s_across >= s_down {
        Axis::Across
    } else {
        Axis::Down
    };
    let mut s = s_across.max(s_down).max(1.0);
    let sizes =
        |s: f32| -> [(f32, f32); 2] { [(s * sq(0), s * sq_inv(0)), (s * sq(1), s * sq_inv(1))] };
    let biggest = sizes(s)
        .iter()
        .map(|(w, h)| w.max(*h))
        .fold(0.0f32, f32::max);
    if biggest > cap_side {
        s *= cap_side / biggest;
    }
    let [(w0, h0), (w1, h1)] = sizes(s);
    match axis {
        Axis::Across => {
            let gap = gap_across;
            let total = w0 + gap + w1;
            let x0 = region.x + (region.w - total) / 2.0;
            let band = h0.max(h1);
            let block = stamp_band + band + label_band;
            let cy = region.y + (region.h - block) / 2.0 + stamp_band + band / 2.0;
            let r0 = Rect::new(x0, cy - h0 / 2.0 - stagger, w0, h0);
            let r1 = Rect::new(x0 + w0 + gap, cy - h1 / 2.0 + stagger, w1, h1);
            Arrangement {
                axis,
                pics: [r0, r1],
                zone: Rect::new(x0 + w0, cy - band / 2.0, gap, band),
                label_top: [
                    r0.y + r0.h - LABEL_OVERLAP * region.w,
                    r1.y + r1.h - LABEL_OVERLAP * region.w,
                ],
            }
        }
        Axis::Down => {
            let gap = gap_down;
            let total = h0 + h1 + gap + 2.0 * (label_band + stamp_band);
            let y0 = region.y + (region.h - total) / 2.0 + stamp_band;
            let cx = region.x + region.w / 2.0;
            let shift = 0.05 * region.w;
            let r0 = Rect::new(cx - shift - w0 / 2.0, y0, w0, h0);
            let zone_top = y0 + h0 + label_band;
            let r1 = Rect::new(cx + shift - w1 / 2.0, zone_top + gap + stamp_band, w1, h1);
            Arrangement {
                axis,
                pics: [r0, r1],
                zone: Rect::new(region.x, zone_top, region.w, gap),
                label_top: [
                    r0.y + r0.h - LABEL_OVERLAP * region.w,
                    r1.y + r1.h - LABEL_OVERLAP * region.w,
                ],
            }
        }
    }
}

fn motion(
    target: &str,
    start: f64,
    duration: f64,
    easing: Easing,
    op: MotionOp,
    spring: Option<SpringSpec>,
) -> Motion {
    Motion {
        id: None,
        target: target.to_string(),
        start: round3(start),
        duration: round3(duration),
        easing,
        spring,
        op,
    }
}

/// An open or closed polyline through canvas points.
fn polyline(
    id: String,
    pts: &[(f32, f32)],
    stroke: Stroke,
    closed: bool,
    fill: Option<Color>,
    depth: Option<f32>,
) -> Layer {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        x0 = x0.min(p.0);
        y0 = y0.min(p.1);
        x1 = x1.max(p.0);
        y1 = y1.max(p.1);
    }
    let local: Vec<[f32; 2]> = pts.iter().map(|p| [p.0 - x0, p.1 - y0]).collect();
    let mut l = base_layer(
        id,
        (x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)),
        LayerKind::Polyline {
            points: local,
            stroke,
            closed,
            fill,
        },
        DEVICE_Z_INDEX,
    );
    l.z = depth;
    l
}

/// A round disc with a short word or sign in it, centred at `c`.
fn badge(ctx: &Ctx, id: String, c: (f32, f32), d: f32, text: &str, depth: Option<f32>) -> Layer {
    let block = ctx.ts.fit_line(Voice::HEADLINE, text, 0.72 * d, 0.42 * d);
    let mut t = text_layer(
        format!("{id}.text"),
        &block,
        ctx.palette.ink,
        TextAlign::Center,
    );
    t.x = (d - t.width) / 2.0;
    t.y = (d - t.height) / 2.0;
    t.z_index = 1;
    let disc = base_layer(
        format!("{id}.disc"),
        (0.0, 0.0, d, d),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: d / 2.0,
            stroke: Some(Stroke {
                color: ctx.palette.accent,
                width: 0.07 * d,
            }),
        },
        0,
    );
    let mut g = base_layer(
        id,
        (c.0, c.1, d, d),
        LayerKind::Group {
            children: vec![disc, t],
        },
        DEVICE_Z_INDEX,
    );
    g.anchor_x = 0.5;
    g.anchor_y = 0.5;
    g.z = depth;
    g
}

/// A label pill centred on `cx` with its top edge at `top`.
#[allow(clippy::too_many_arguments)]
fn pill(
    ctx: &Ctx,
    id: String,
    cx: f32,
    top: f32,
    text: &str,
    max_w: f32,
    bounds: (f32, f32),
    depth: Option<f32>,
) -> Layer {
    let short = ctx.w.min(ctx.h);
    let pad = 0.03 * short;
    let block = ctx
        .ts
        .fit_line(Voice::LABEL, text, max_w - 2.0 * pad, 0.042 * short);
    let (tw, th) = (block.width() * 1.02 + 2.0, block.height());
    let (pw, ph) = (tw + 2.0 * pad, th + pad);
    let mut t = text_layer(
        format!("{id}.text"),
        &block,
        ctx.palette.ink,
        TextAlign::Center,
    );
    t.x = pad;
    t.y = (ph - th) / 2.0;
    t.z_index = 1;
    let bg = base_layer(
        format!("{id}.pill"),
        (0.0, 0.0, pw, ph),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: ph / 2.0,
            stroke: None,
        },
        0,
    );
    let cx = cx.clamp(
        bounds.0 + pw / 2.0,
        (bounds.1 - pw / 2.0).max(bounds.0 + pw / 2.0),
    );
    let mut g = base_layer(
        id,
        (cx, top + ph / 2.0, pw, ph),
        LayerKind::Group {
            children: vec![bg, t],
        },
        LABEL_Z_INDEX,
    );
    g.anchor_x = 0.5;
    g.anchor_y = 0.5;
    g.z = depth;
    g
}

/// A stamped figure centred on `cx` (kept inside `[lo, hi]` and the safe
/// area), its bottom edge at `bottom`. Set in the figure font: "1400s" stays
/// "1400s".
///
/// (0.23) `backed`: the figure sits on a chip of the card colour, like the
/// label pills. A picture sliding in (or a ground plate) can be under the
/// stamp when the beat is read, and the accent colour on a mid-tone picture
/// is not readable ("much later" #2DD4BF on #81A49F, 1.46:1); on the chip the
/// ground is known. The chip's height is [`STAMP_CHIP_H`] of the short side.
#[allow(clippy::too_many_arguments)]
fn stamp(
    ctx: &Ctx,
    id: String,
    cx: f32,
    bottom: f32,
    text: &str,
    bounds: (f32, f32),
    depth: Option<f32>,
    backed: bool,
) -> Layer {
    let short = ctx.w.min(ctx.h);
    // (0.23) Never past the safe area: the stamps of a wide figure ("$25,000")
    // used to reach the edge of the region, outside the side margins.
    let safe = ctx.frame.safe;
    let lo = bounds.0.max(safe.x);
    let hi = bounds.1.min(safe.x + safe.w).max(lo);
    if backed {
        let (pad_x, pad_y) = (0.03 * short, 0.02 * short);
        let block = ctx.ts.fit_line(
            Voice::HERO_NUMBER,
            text,
            ((hi - lo).min(0.46 * ctx.w) - 2.0 * pad_x).max(0.1 * ctx.w),
            0.085 * short,
        );
        // The accent when it reads on the chip, else the ink.
        let color = if contrast_ratio(ctx.palette.accent, ctx.palette.card) >= STAMP_CHIP_CONTRAST {
            ctx.palette.accent
        } else {
            ctx.palette.ink
        };
        let mut t = text_layer(format!("{id}.text"), &block, color, TextAlign::Center);
        t.x = pad_x;
        t.y = pad_y;
        t.z_index = 1;
        let (cw, ch) = (t.width + 2.0 * pad_x, t.height + 2.0 * pad_y);
        let chip = base_layer(
            format!("{id}.chip"),
            (0.0, 0.0, cw, ch),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.card,
                radius: 0.3 * ch,
                stroke: Some(Stroke {
                    color: ctx.palette.accent,
                    width: (0.004 * short).max(1.5),
                }),
            },
            0,
        );
        let x = (cx - cw / 2.0).clamp(lo, (hi - cw).max(lo));
        let mut g = base_layer(
            id,
            (x, bottom - ch, cw, ch),
            LayerKind::Group {
                children: vec![chip, t],
            },
            LABEL_Z_INDEX,
        );
        g.z = depth;
        return g;
    }
    let block = ctx.ts.fit_line(
        Voice::HERO_NUMBER,
        text,
        (bounds.1 - bounds.0).min(0.46 * ctx.w),
        0.085 * short,
    );
    let mut l = text_layer(id, &block, ctx.palette.accent, TextAlign::Center);
    // Where the stamp stands inside the safe area (with the padding layout QA
    // grants a text box) it stays where the region put it; only one that
    // would stand outside it is moved in.
    let x = (cx - l.width / 2.0).clamp(bounds.0, (bounds.1 - l.width).max(bounds.0));
    let tol = (0.02 * l.width + 2.0) / 1.02;
    let inside = x >= safe.x - tol && x + l.width <= safe.x + safe.w + tol;
    l.x = if inside {
        x
    } else {
        (cx - l.width / 2.0).clamp(lo, (hi - l.width).max(lo))
    };
    l.y = bottom - l.height;
    l.z_index = LABEL_Z_INDEX;
    l.z = depth;
    l
}

/// (0.23) Height of a stamp's chip as a share of the short side, and the
/// contrast the accent colour needs on the card colour to be the figure's
/// colour (else the ink).
const STAMP_CHIP_H: f32 = 0.135;
const STAMP_CHIP_CONTRAST: f32 = 4.5;

/// (0.22) True when the beat is a relation of two pictures that both resolve
/// (a delivered image or a library picture each): what [`build`] needs.
pub(super) fn pair_ready(ctx: &Ctx, b: &B) -> bool {
    pair(ctx, b, Stage::Flat).is_some()
}

fn pair(ctx: &Ctx, b: &B, stage: Stage) -> Option<(Pic, Pic)> {
    let beat = b.beat;
    if beat.primary.kind() != SubjectKind::Object
        || beat.secondary.as_ref().map(Subject::kind) != Some(SubjectKind::Object)
    {
        return None;
    }
    // (0.23) In the flat looks two pictures that each carry a figure are a
    // stat pair whatever the beat's purpose (the street and studio looks give
    // an emphasize beat of two valued pictures to this stage: their one
    // punchword shows one figure). The cinematic hero + supporting picture
    // keeps its emphasize beats.
    let valued = stage == Stage::Flat
        && value_of(&beat.primary).is_some()
        && beat.secondary.as_ref().and_then(value_of).is_some();
    if !matches!(beat.purpose, Purpose::Compare | Purpose::Contrast) && !valued {
        return None;
    }
    Some((
        resolve(ctx, b, Which::Primary)?,
        resolve(ctx, b, Which::Secondary)?,
    ))
}

/// Build the relation stage. `title_bottom` is the canvas y of the title's
/// lower edge. Returns `false` (nothing placed) when the beat is not a relation
/// of two pictures, so the caller builds the usual hero and prop.
pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    title_bottom: f32,
    stage: Stage,
) -> Result<bool, CompileError> {
    let beat = b.beat;
    let Some((first, second)) = pair(ctx, b, stage) else {
        return Ok(false);
    };
    let depth = stage.depth();

    let (w, h) = (ctx.w, ctx.h);
    let short = w.min(h);
    let plan = b.plan;
    let life = plan.life;
    let t = plan.enter_at();
    let (device, play, weights) = relation(beat);

    // Free band under the title and above the caption lane.
    let safe = ctx.frame.safe;
    let edge = EDGE_SHARE * w;
    let top = title_bottom + 0.035 * h;
    let bottom = safe.y + safe.h;
    let region = Rect::new(edge, top, w - 2.0 * edge, (bottom - top).max(0.3 * h));

    let stamps = [value_of(&first.subject), value_of(&second.subject)];
    let any_stamp = stamps.iter().any(Option::is_some);
    let pill_h = 0.042 * short * 1.5;
    // The pill overlaps the picture's lower edge by a third of its height.
    let label_band = 0.65 * pill_h + 0.012 * short;
    // (0.23) On the product path the cinematic stamps sit on a chip (`stamp`),
    // which needs a taller band.
    let backed = stage == Stage::Cinematic && ctx.direction_seed.is_some();
    let stamp_band = match (any_stamp, backed) {
        (false, _) => 0.0,
        (true, false) => 0.09 * short,
        (true, true) => STAMP_CHIP_H * short,
    };
    // The versus / plus disc (0.15 of the short side) may overlap the pictures'
    // inner edges side by side, but needs its own zone when they are stacked;
    // an arrow needs room for its shaft and head either way.
    let gaps = match device {
        Device::None => [0.1 * short, 0.12 * short],
        Device::Versus | Device::Plus => [0.14 * short, 0.18 * short],
        _ => [0.18 * short, 0.19 * short],
    };
    let gap = gaps[0];
    // The cinematic pair is staggered for depth; a flat pair sits level, so
    // its labels keep inside the safe area the arrangement measured.
    let stagger = match stage {
        Stage::Cinematic => 0.012 * h,
        Stage::Flat => 0.0,
    };
    let arr = arrange(
        region,
        [first.aspect, second.aspect],
        weights,
        label_band,
        stamp_band,
        gaps,
        MAX_SIDE_SHARE * short,
        stagger,
    );

    // The pictures: the first enters with ENTER, the second at EVOLVE from the
    // opposite side. (0.23) Under a direction seed the side is the beat's entry
    // of the seeded arrival sequence (recorded in the direction record); without
    // one it is the position rule, as before.
    let entry = cinematic3d::Arrival::rotated(ctx, plan.index);
    // The second picture arrives at EVOLVE (a spoken word). A short beat would
    // leave the finished relation on screen for under a second before the
    // camera flies out, so there it arrives right after the first has landed.
    // With a voice-over it arrives as the second picture is named.
    // (0.23) On the product path the picture is named by every word its reveal
    // anchor holds (asset, value and meaning: `subject_words`), the way the
    // word cue moves it. Named by its asset and meaning alone, a picture
    // that the narrator names by its value ("one thousand eight hundred and
    // twenty five dollars") was moved onto that word while the connector kept
    // the time of the beat's last evolve event, and the relationship (the
    // arrow of `grow`) drew after ANTICIPATE.
    let named = match &second.subject {
        Subject::Object(_) if ctx.direction_seed.is_some() => {
            let words = crate::compiler::recipes::subject_words(&second.subject);
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            crate::compiler::speech_plan::name_time(&ctx.spoken, &words)
        }
        Subject::Object(o) => crate::compiler::speech_plan::name_time(
            &ctx.spoken,
            &[o.asset.as_str(), o.meaning.as_deref().unwrap_or_default()],
        ),
        _ => None,
    };
    let mut t_b = named.map_or(life.evolve - 0.1, |at| at - 0.3);
    t_b = t_b.min(life.anticipate - 0.8);
    if plan.duration < SHORT_BEAT {
        t_b = t_b.min(t + 1.0);
    }
    let t_b = t_b.max(t + 0.7).min((plan.duration - 1.2).max(t + 0.5));
    let mut ids: Vec<String> = Vec::new();
    // (0.23 W8a) The carry of each picture that lives in a shared element.
    let mut shared: [Option<usize>; 2] = [None, None];
    for (i, pic) in [&first, &second].into_iter().enumerate() {
        let name = if i == 0 { "hero" } else { "prop" };
        let object = Some((pic.which, pic.subject.clone()));
        let (start, entry) = if i == 0 {
            (t + 0.15, entry)
        } else {
            (t_b, entry.opposite())
        };
        // (0.22) Flat: a fade and a short slide in from its own side.
        // (0.23) Under a direction seed the first picture slides in from
        // the beat's seeded side and the second from the opposite one.
        let slide = if ctx.direction_seed.is_some() {
            match entry {
                cinematic3d::Arrival::Right => (0.06 * ctx.w, 0.0),
                cinematic3d::Arrival::Left => (-0.06 * ctx.w, 0.0),
                cinematic3d::Arrival::Below => (0.0, 0.04 * ctx.h),
                cinematic3d::Arrival::Above => (0.0, -0.04 * ctx.h),
            }
        } else {
            (if i == 0 { -0.06 } else { 0.06 } * ctx.w, 0.0)
        };
        // (0.23 W8a) A flat picture the story carries across beats is a shared
        // element: it enters with its track (no scene layer, no reveal anchor,
        // no entrance motion) and glides into its slot when carried in.
        let carried = if stage == Stage::Flat {
            cinematic3d::delivered(ctx, plan.index, pic.roles).and_then(|(e, role)| {
                let img = placement::subject_fit(&SubjectFacts::from_entry(&e), arr.pics[i]);
                plate::place_carried(
                    ctx,
                    b,
                    carries,
                    pic.which,
                    &e,
                    role,
                    (img.x, img.y, img.w, img.h),
                    PIC_Z_INDEX,
                    None,
                    0.0,
                    start,
                    plate::Arrive {
                        offset: slide,
                        duration: 0.6,
                    },
                )
            })
        } else {
            None
        };
        if let Some(c) = carried {
            shared[i] = Some(c.carry);
            ids.push(c.id);
            continue;
        }
        let id = cinematic3d::place_picture(
            ctx,
            b,
            carries,
            name,
            pic.roles,
            object,
            arr.pics[i],
            PIC_Z_INDEX,
            start,
            ctx.palette.ink,
        )?
        .ok_or_else(|| CompileError::Beat {
            beat: plan.index + 1,
            message: "a relation picture could not be placed".into(),
        })?;
        if let Some(z) = depth {
            cinematic3d::set_depth(b, &id, z);
        }
        // (0.20) Each picture arrives when the narrator names its subject.
        b.reveal(
            name,
            crate::compiler::recipes::subject_words(&pic.subject),
            crate::speech::RevealRole::Content,
        );
        match stage {
            // The entrance the cinematic hero uses: fade, settle and the arrival
            // slide + turn from this beat's side (the second from the opposite one).
            Stage::Cinematic => cinematic3d::arrive(
                b,
                &id,
                entry,
                ctx.w,
                ctx.h,
                1.0,
                start - 0.13,
                cinematic3d::landing_for(plan.duration),
            ),
            Stage::Flat => {
                b.motions
                    .push(mo::fade(&id, start, 0.35, 0.0, 1.0, Easing::OutCubic));
                b.motions.push(mo::shift(
                    &id,
                    start,
                    0.6,
                    [slide.0, slide.1],
                    [0.0, 0.0],
                    Easing::OutQuint,
                ));
            }
        }
        ids.push(id);
    }

    // Labels and stamps follow their picture.
    let max_w = match arr.axis {
        Axis::Across => 0.5 * region.w - 0.5 * gap,
        Axis::Down => region.w,
    };
    for (i, pic) in [&first, &second].into_iter().enumerate() {
        let start = if i == 0 { t + 0.55 } else { t_b + 0.45 };
        let cx = arr.pics[i].cx();
        let label_id = b.id(&format!("pair_label.{i}"));
        b.reveal(
            &format!("pair_label.{i}"),
            crate::compiler::recipes::subject_words(&pic.subject),
            crate::speech::RevealRole::Label,
        );
        b.stage.push(pill(
            ctx,
            label_id.clone(),
            cx,
            arr.label_top[i],
            &label_of(&pic.subject),
            max_w.max(0.3 * w),
            (region.x, region.x + region.w),
            depth,
        ));
        b.motions
            .push(mo::fade(&label_id, start, 0.35, 0.0, 1.0, Easing::OutCubic));
        if let Some(text) = &stamps[i] {
            let stamp_id = b.id(&format!("pair_stamp.{i}"));
            b.reveal(
                &format!("pair_stamp.{i}"),
                [text.clone()],
                crate::speech::RevealRole::Stamp,
            );
            b.stage.push(stamp(
                ctx,
                stamp_id.clone(),
                cx,
                arr.pics[i].y - 0.012 * short,
                text,
                (region.x, region.x + region.w),
                depth,
                backed,
            ));
            b.motions.push(motion(
                &stamp_id,
                start + 0.05,
                0.5,
                Easing::OutCubic,
                MotionOp::Fade { from: 0.0, to: 1.0 },
                None,
            ));
        }
    }

    // The node: a connection point at the middle of the stage that pops in with
    // the first picture, so the beat's focal layer is drawn (and the camera has
    // its pivot) while the first picture is read. It stays: under the VS disc,
    // or as the bead on the arrow, once the second picture has arrived.
    let t_dev = t_b + 0.45;
    let node = b.id("pair_node");
    let centre = (arr.zone.x + arr.zone.w / 2.0, arr.zone.y + arr.zone.h / 2.0);
    let mut node_disc = cinematic3d::disc(
        node.clone(),
        centre,
        0.035 * short,
        ctx.palette.accent,
        DEVICE_Z_INDEX,
        cinematic3d::Z_HERO,
    );
    node_disc.z = depth;
    b.stage.push(node_disc);
    b.motions
        .push(mo::fade(&node, t + 0.25, 0.3, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(cinematic3d::sprung(
        mo::scale(&node, t + 0.25, 0.5, 0.3, 1.0, Easing::OutCubic),
        POP,
    ));
    b.focal = Some(node);
    connector(ctx, b, &arr, device, t_dev, depth, "pair");

    // What the relationship does to the pictures once both are on stage.
    // (After the second picture's entrance scale has ended: one animation per
    // channel at a time.)
    let t_play = t_dev + 0.55;
    let room = t_play + 0.9 < plan.duration - 0.5;
    match if room { play } else { Play::None } {
        Play::None => {}
        Play::Replace => {
            Step::new(&ids[0], shared[0], t_play, 0.7, Easing::InOutCubic).scale(
                b,
                carries,
                (1.0, 0.84),
                Some((1.0, 0.6)),
            );
            Step::new(&ids[1], shared[1], t_play, 0.7, Easing::OutCubic).scale(
                b,
                carries,
                (1.0, 1.1),
                None,
            );
        }
        Play::Grow => {
            Step::new(&ids[1], shared[1], t_play, 0.9, Easing::OutCubic).scale(
                b,
                carries,
                (1.0, 1.16),
                None,
            );
        }
        Play::Compress => {
            Step::new(&ids[0], shared[0], t_play, 0.7, Easing::InOutCubic).scale(
                b,
                carries,
                (1.0, 0.8),
                None,
            );
            Step::new(&ids[1], shared[1], t_play, 0.7, Easing::OutCubic).scale(
                b,
                carries,
                (1.0, 1.1),
                None,
            );
        }
    }
    Ok(true)
}

/// What the relationship does to one picture once both are on stage: a sprung
/// scale (and fade) on a scene layer, or, for a picture that lives in a shared
/// element (one the story carries, 0.23 W8a), the same step in its track, which
/// ends where the next beat's key takes over.
struct Step<'a> {
    id: &'a str,
    carry: Option<usize>,
    at: f64,
    duration: f64,
    easing: Easing,
}

impl<'a> Step<'a> {
    fn new(id: &'a str, carry: Option<usize>, at: f64, duration: f64, easing: Easing) -> Self {
        Step {
            id,
            carry,
            at,
            duration,
            easing,
        }
    }

    /// Scale `(from, to)`; `fade` also dims `(from, to)` over the same time.
    fn scale(&self, b: &mut B, carries: &mut [Carry], scale: (f32, f32), fade: Option<(f32, f32)>) {
        match self.carry {
            Some(ci) => plate::shared_step(
                b,
                carries,
                ci,
                self.at,
                self.duration,
                self.easing,
                scale.1 / scale.0,
                fade.map(|(_, to)| to),
            ),
            None => {
                b.motions.push(cinematic3d::sprung(
                    mo::scale(
                        self.id,
                        self.at,
                        self.duration,
                        scale.0,
                        scale.1,
                        self.easing,
                    ),
                    POP,
                ));
                if let Some((from, to)) = fade {
                    b.motions.push(mo::fade(
                        self.id,
                        self.at,
                        self.duration,
                        from,
                        to,
                        self.easing,
                    ));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// (0.23) Figures: a comparison of two values, and the card a value sits on
// ---------------------------------------------------------------------------

/// One side of a comparison of figures: a number or a phrase.
struct Figure {
    /// The figure as written: the value, else the phrase / meaning.
    text: String,
    /// What it stands for, shown as a label (only when `text` is the value).
    label: Option<String>,
}

fn clean(s: Option<&String>) -> Option<String> {
    s.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn figure_of(s: &Subject) -> Option<Figure> {
    let (Subject::Number(a) | Subject::Phrase(a)) = s else {
        return None;
    };
    let (value, meaning) = (clean(a.value.as_ref()), clean(a.meaning.as_ref()));
    let text = value.clone().or_else(|| meaning.clone())?;
    Some(Figure {
        text,
        label: value.and(meaning),
    })
}

/// (0.23) The two figures of a `compare` / `contrast` beat whose sides are
/// both numbers or phrases (no pictures): `None` for any other beat.
fn figure_pair(beat: &Beat) -> Option<[Figure; 2]> {
    if !matches!(beat.purpose, Purpose::Compare | Purpose::Contrast) {
        return None;
    }
    Some([
        figure_of(&beat.primary)?,
        figure_of(beat.secondary.as_ref()?)?,
    ])
}

/// The part of a figure a narrator names it by: its first token with a digit
/// ("5" for "5 hours"), else the text itself.
fn spoken_key(text: &str) -> String {
    text.split_whitespace()
        .find(|t| t.chars().any(|c| c.is_ascii_digit()))
        .unwrap_or(text)
        .to_string()
}

/// A card for a value: the figure over what it stands for, on the card stock
/// with an accent keyline, centred on `cx` (kept inside `bounds`) with its top
/// edge at `top`. `None` when there is nothing to show. The value is set in the
/// figure voice at most `value_size` px, the meaning small in the label voice.
#[allow(clippy::too_many_arguments)]
pub(super) fn stat_card(
    ctx: &Ctx,
    id: String,
    cx: f32,
    top: f32,
    value: Option<&str>,
    meaning: Option<&str>,
    max_w: f32,
    value_size: f32,
    bounds: (f32, f32),
    depth: Option<f32>,
) -> Option<Layer> {
    let short = ctx.w.min(ctx.h);
    let pad = 0.03 * short;
    let inner = (max_w - 2.0 * pad).max(0.2 * short);
    fn present(t: Option<&str>) -> Option<&str> {
        t.map(str::trim).filter(|t| !t.is_empty())
    }
    let v = present(value).map(|t| ctx.ts.fit_line(Voice::HERO_NUMBER, t, inner, value_size));
    let m = present(meaning).map(|t| ctx.ts.fit_line(Voice::LABEL, t, inner, 0.036 * short));
    if v.is_none() && m.is_none() {
        return None;
    }
    let gap = if v.is_some() && m.is_some() {
        0.008 * short
    } else {
        0.0
    };
    let box_w = |b: &crate::compiler::typeset::Block| b.width() * 1.02 + 2.0;
    let content_w = [v.as_ref(), m.as_ref()]
        .into_iter()
        .flatten()
        .map(box_w)
        .fold(0.0, f32::max);
    let content_h =
        v.as_ref().map_or(0.0, |b| b.height()) + gap + m.as_ref().map_or(0.0, |b| b.height());
    let (pw, ph) = (content_w + 2.0 * pad, content_h + pad);
    let mut children = vec![base_layer(
        format!("{id}.card"),
        (0.0, 0.0, pw, ph),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: (0.3 * ph).min(0.045 * short),
            stroke: Some(Stroke {
                color: ctx.palette.accent,
                width: 0.006 * short,
            }),
        },
        0,
    )];
    let mut y = pad / 2.0;
    for (name, block) in [("value", v.as_ref()), ("label", m.as_ref())] {
        let Some(block) = block else { continue };
        let mut t = text_layer(
            format!("{id}.{name}"),
            block,
            ctx.palette.ink,
            TextAlign::Center,
        );
        t.x = (pw - t.width) / 2.0;
        t.y = y;
        t.z_index = 1;
        y += block.height() + gap;
        children.push(t);
    }
    let cx = cx.clamp(
        bounds.0 + pw / 2.0,
        (bounds.1 - pw / 2.0).max(bounds.0 + pw / 2.0),
    );
    let mut g = base_layer(
        id,
        (cx, top + ph / 2.0, pw, ph),
        LayerKind::Group { children },
        LABEL_Z_INDEX,
    );
    g.anchor_x = 0.5;
    g.anchor_y = 0.5;
    g.z = depth;
    Some(g)
}

/// Both figures set at the same size: the larger fit of the pair is brought
/// down to the smaller, so two co-equal values read as equals.
fn equal_blocks(
    ctx: &Ctx,
    texts: [&str; 2],
    max_w: f32,
    max_h: f32,
    max_size: f32,
) -> [crate::compiler::typeset::Block; 2] {
    let voice = Voice::HEADLINE;
    let fit = |t: &str, cap: f32| ctx.ts.fit_block(voice, t, max_w, max_h, cap, 2);
    let blocks = [fit(texts[0], max_size), fit(texts[1], max_size)];
    let (small, big) = if blocks[0].size <= blocks[1].size {
        (0, 1)
    } else {
        (1, 0)
    };
    if blocks[big].size - blocks[small].size < 0.5 {
        return blocks;
    }
    // The scale-contrast gain of the display voice (the fit caps the size at
    // `max_size * gain`): probed, since the typesetter keeps it private.
    let gain = (ctx.ts.fit_line(voice, "0", 1.0e6, 100.0).size / 100.0).max(0.01);
    let mut out = blocks.clone();
    out[big] = fit(texts[big], blocks[small].size / gain);
    out
}

/// (0.23) A `compare` / `contrast` beat of two figures (numbers or phrases)
/// staged on the focus plane: each value large, with what it stands for in a
/// label pill under it, and the beat's relationship drawn between them (the
/// same connector, node and timing as the pair of pictures). Stacked on a
/// tall canvas, side by side on a wide one. Returns `false` (nothing placed)
/// when the beat is not a comparison of two figures.
pub(super) fn build_values(
    ctx: &mut Ctx,
    b: &mut B,
    title_bottom: f32,
    stage: Stage,
) -> Result<bool, CompileError> {
    let Some(figs) = figure_pair(b.beat) else {
        return Ok(false);
    };
    let depth = stage.depth();
    let (w, h) = (ctx.w, ctx.h);
    let short = w.min(h);
    let plan = b.plan;
    let life = plan.life;
    let t = plan.enter_at();
    let (device, _, _) = relation(b.beat);

    let safe = ctx.frame.safe;
    let edge = EDGE_SHARE * w;
    let top = title_bottom + 0.035 * h;
    let bottom = safe.y + safe.h;
    let region = Rect::new(edge, top, w - 2.0 * edge, (bottom - top).max(0.3 * h));
    let across = super::placement::side_by_side(&ctx.frame);
    let gaps = match device {
        Device::Versus | Device::Plus => [0.14 * short, 0.18 * short],
        _ => [0.18 * short, 0.19 * short],
    };
    let pill_h = 0.042 * short * 1.5;
    let label_gap = 0.012 * short;
    let any_label = figs.iter().any(|f| f.label.is_some());
    let label_band = if any_label { pill_h + label_gap } else { 0.0 };

    // The figures, set at one size.
    let (max_w, max_h) = if across {
        (0.5 * (region.w - gaps[0]), 0.5 * (region.h - label_band))
    } else {
        (
            region.w,
            0.5 * (region.h - gaps[1] - 2.0 * label_band).max(0.1 * h),
        )
    };
    let cap = 0.24 * short;
    let blocks = equal_blocks(
        ctx,
        [figs[0].text.as_str(), figs[1].text.as_str()],
        max_w,
        max_h,
        cap,
    );
    let bw = [
        blocks[0].width() * 1.02 + 2.0,
        blocks[1].width() * 1.02 + 2.0,
    ];
    let bh = [blocks[0].height(), blocks[1].height()];

    // Boxes of the two figures and the connector zone between them.
    let (axis, rects, zone) = if across {
        let band = bh[0].max(bh[1]);
        let block = band + label_band;
        let cy = region.y + (region.h - block) / 2.0 + band / 2.0;
        let cell = 0.5 * (region.w - gaps[0]);
        let cx = [region.x + cell / 2.0, region.x + region.w - cell / 2.0];
        (
            Axis::Across,
            [
                Rect::new(cx[0] - bw[0] / 2.0, cy - bh[0] / 2.0, bw[0], bh[0]),
                Rect::new(cx[1] - bw[1] / 2.0, cy - bh[1] / 2.0, bw[1], bh[1]),
            ],
            Rect::new(region.x + cell, cy - band / 2.0, gaps[0], band),
        )
    } else {
        let total = bh[0] + bh[1] + 2.0 * label_band + gaps[1];
        let y0 = region.y + ((region.h - total) / 2.0).max(0.0);
        let cx = region.x + region.w / 2.0;
        let y1 = y0 + bh[0] + label_band + gaps[1];
        (
            Axis::Down,
            [
                Rect::new(cx - bw[0] / 2.0, y0, bw[0], bh[0]),
                Rect::new(cx - bw[1] / 2.0, y1, bw[1], bh[1]),
            ],
            Rect::new(region.x, y0 + bh[0] + label_band, region.w, gaps[1]),
        )
    };

    // Timing: the first figure with ENTER; the second when the narrator says
    // it, else so that it has landed (with the connector) by READ: the two
    // values are read together, and layout QA judges the whole comparison at
    // READ.
    let named =
        crate::compiler::speech_plan::name_time(&ctx.spoken, &[spoken_key(&figs[1].text).as_str()]);
    let (planned, floor) = match named {
        Some(at) => (at - 0.3, t + 0.7),
        None => (life.read - 1.2, t + 0.45),
    };
    let mut t_b = planned.min(life.anticipate - 0.8);
    if plan.duration < SHORT_BEAT {
        t_b = t_b.min(t + 1.0);
    }
    let t_b = t_b.max(floor).min((plan.duration - 1.2).max(t + 0.5));

    let seed = plan.seed;
    let u = ctx.u;
    for (i, fig) in figs.iter().enumerate() {
        let start = if i == 0 { t + 0.15 } else { t_b };
        let name = format!("fig.{i}");
        let id = b.id(&name);
        let mut l = text_layer(
            id.clone(),
            &blocks[i],
            ctx.palette.accent,
            TextAlign::Center,
        );
        l.x = rects[i].x;
        l.y = rects[i].y;
        l.z_index = PIC_Z_INDEX;
        l.z = depth;
        b.stage.push(l);
        // The figure arrives when the narrator says it, as written ("seven
        // hours" for "7 hours"); a figure the narration never says keeps its
        // planned time.
        b.reveal(&name, [fig.text.clone()], crate::speech::RevealRole::Value);
        b.motions.push(cinematic3d::sprung(
            motion(
                &id,
                start,
                0.9,
                Easing::OutQuint,
                MotionOp::GlyphCascade {
                    stagger: 0.04,
                    from: GlyphPose {
                        dx: 0.0,
                        dy: 60.0 * u,
                        scale: 1.5,
                        rotation: 0.0,
                        opacity: 0.0,
                    },
                    order: GlyphOrder::Center,
                    seed: seed as u32,
                },
                None,
            ),
            cinematic3d::SETTLE,
        ));
        // The label follows its figure.
        if let Some(text) = &fig.label {
            let label_id = b.id(&format!("fig_label.{i}"));
            // The label comes up with its figure.
            b.reveal(
                &format!("fig_label.{i}"),
                [fig.text.clone()],
                crate::speech::RevealRole::Label,
            );
            let (lx, ly) = (rects[i].cx(), rects[i].bottom() + label_gap);
            b.stage.push(pill(
                ctx,
                label_id.clone(),
                lx,
                ly,
                text,
                max_w.max(0.3 * w),
                (region.x, region.x + region.w),
                depth,
            ));
            b.motions.push(mo::fade(
                &label_id,
                start + 0.45,
                0.35,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
        }
    }

    // The node at the middle of the relation pops in with the first figure
    // and is the beat's focal layer; the connector draws once the second
    // figure has landed.
    let arr = Arrangement {
        axis,
        pics: rects,
        zone,
        label_top: [rects[0].bottom() + label_gap, rects[1].bottom() + label_gap],
    };
    let node = b.id("fig_node");
    let centre = (zone.x + zone.w / 2.0, zone.y + zone.h / 2.0);
    let mut node_disc = cinematic3d::disc(
        node.clone(),
        centre,
        0.035 * short,
        ctx.palette.accent,
        DEVICE_Z_INDEX,
        cinematic3d::Z_HERO,
    );
    node_disc.z = depth;
    b.stage.push(node_disc);
    b.motions
        .push(mo::fade(&node, t + 0.25, 0.3, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(cinematic3d::sprung(
        mo::scale(&node, t + 0.25, 0.5, 0.3, 1.0, Easing::OutCubic),
        POP,
    ));
    b.focal = Some(node);
    connector(ctx, b, &arr, device, t_b + 0.45, depth, "fig");
    Ok(true)
}

/// Draw the connector for `device` in the arrangement's zone.
fn connector(
    ctx: &Ctx,
    b: &mut B,
    arr: &Arrangement,
    device: Device,
    t: f64,
    depth: Option<f32>,
    prefix: &str,
) {
    let short = ctx.w.min(ctx.h);
    let sw = 0.016 * short;
    let accent = ctx.palette.accent;
    let ink = ctx.palette.ink;
    let zone = arr.zone;
    let (zx, zy) = (zone.x + zone.w / 2.0, zone.y + zone.h / 2.0);
    let pad = 0.02 * short;
    let head = 0.06 * short;
    let across = arr.axis == Axis::Across;

    // Endpoints of the line the connector runs along (first -> second).
    let (p0, p1) = if across {
        ((zone.x + pad, zy), (zone.x + zone.w - pad, zy))
    } else {
        ((zx, zone.y + pad), (zx, zone.y + zone.h - pad))
    };
    let draw = |b: &mut B, id: &str, start: f64, dur: f64| {
        b.motions.push(motion(
            id,
            start,
            dur,
            Easing::OutCubic,
            MotionOp::Trim { from: 0.0, to: 1.0 },
            None,
        ));
    };

    match device {
        Device::None => {}
        Device::Versus | Device::Plus => {
            let d = 0.15 * short;
            if device == Device::Versus {
                // The divider spans the zone across the line of the pictures.
                let (a, c) = if across {
                    ((zx, zone.y), (zx, zone.y + zone.h))
                } else {
                    (
                        (zone.x + 0.08 * ctx.w, zy),
                        (zone.x + zone.w - 0.08 * ctx.w, zy),
                    )
                };
                let id = b.id(&format!("{prefix}_divider"));
                let mut l = polyline(
                    id.clone(),
                    &[a, c],
                    Stroke {
                        color: ink.with_alpha(0xB0),
                        width: 0.45 * sw,
                    },
                    false,
                    None,
                    depth,
                );
                l.z_index = DEVICE_Z_INDEX - 1;
                b.stage.push(l);
                draw(b, &id, t, 0.6);
            }
            let id = b.id(&format!("{prefix}_badge"));
            b.stage.push(badge(
                ctx,
                id.clone(),
                (zx, zy),
                d,
                if device == Device::Versus { "VS" } else { "+" },
                depth,
            ));
            b.motions
                .push(mo::fade(&id, t + 0.1, 0.25, 0.0, 1.0, Easing::OutCubic));
            b.motions.push(cinematic3d::sprung(
                mo::scale(&id, t + 0.1, 0.55, 0.4, 1.0, Easing::OutCubic),
                POP,
            ));
        }
        Device::Link => {
            let id = b.id(&format!("{prefix}_link"));
            b.stage.push(polyline(
                id.clone(),
                &[p0, p1],
                Stroke {
                    color: accent,
                    width: sw,
                },
                false,
                None,
                depth,
            ));
            draw(b, &id, t, 0.6);
        }
        Device::ArrowForward | Device::ArrowBack => {
            let (from, to) = if device == Device::ArrowForward {
                (p0, p1)
            } else {
                (p1, p0)
            };
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let len = (dx * dx + dy * dy).sqrt().max(1.0);
            let (ux, uy) = (dx / len, dy / len);
            let base = (to.0 - ux * head, to.1 - uy * head);
            let shaft = b.id(&format!("{prefix}_arrow"));
            b.stage.push(polyline(
                shaft.clone(),
                &[from, base],
                Stroke {
                    color: accent,
                    width: sw,
                },
                false,
                None,
                depth,
            ));
            draw(b, &shaft, t, 0.55);
            // Arrowhead: a filled triangle on the end, in after the shaft.
            let (nx, ny) = (-uy, ux);
            let half = 0.55 * head;
            let tri = [
                to,
                (base.0 + nx * half, base.1 + ny * half),
                (base.0 - nx * half, base.1 - ny * half),
            ];
            let head_id = b.id(&format!("{prefix}_arrowhead"));
            b.stage.push(polyline(
                head_id.clone(),
                &tri,
                Stroke {
                    color: accent,
                    width: 1.0,
                },
                true,
                Some(accent),
                depth,
            ));
            b.motions.push(mo::fade(
                &head_id,
                t + 0.45,
                0.2,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
        }
    }
}
