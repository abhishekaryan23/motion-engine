//! TypeImageInterlock: a subject image and the headline interlock (behind / in
//! front / crossing / beside), chosen per image by subject-aware placement
//! (0.5, `placement.rs`).
//!
//! The builder generates internal layout candidates from the image's analyzed
//! geometry (alpha bounds, occupancy, head region) and the measured headline,
//! scores them (face collision, occlusion, clipping, balance, lower third,
//! hierarchy, side preference) and builds the best one. Headline lines are
//! layered per line: a line crossing the head is always *behind* the subject;
//! a front line crossing the body sits on a paper strip. The image arrives in
//! ENTER, pushes very slightly through READ/EVOLVE on its own depth plane,
//! and the primary subject and a serif secondary arrive at EVOLVE events in
//! the free space the chosen layout leaves.

use super::placement::{
    self, Candidate, Frame, Interlock, Layering, LineBox, Rect, Side, SubjectFacts,
};
use super::plate::{self, events};
use super::{Composition, Grammar, ImageLayout, Variant, IMAGE_LAYOUT_ROTATION};
use crate::assets::{AssetRole, NormBox};
use crate::compiler::layout_frame::LayoutFrame;
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::taste_rules::{
    BODY_MAX_LINES, HEADLINE_MAX_H_WITH_SUBJECT, HEADLINE_MAX_LINES_TALL, HEADLINE_MAX_LINES_WIDE,
};
use crate::compiler::typeset::{Block, HeadlineBudget, Typesetter, Voice};
use crate::compiler::{
    base_layer, direction, mix, mo, subject_rules, temporal, texture_layer, BeatPlan, Carry,
    CompileError, Ctx, Which,
};
use crate::easing::Easing;
use crate::intent::{Beat, Format, Subject, SubjectKind};
use crate::scene::{Direction, LayerKind, Material, Stroke, TextAlign, TextureSpec};

const Z_BACK: i32 = 20;
const Z_IMAGE: i32 = 22;
const Z_STRIP: i32 = 23;
const Z_FRONT: i32 = 24;
/// Subject plane: slightly in front of the type plane (subtle parallax).
const SUBJECT_DEPTH_SHARE: f32 = 0.35;
/// READ/EVOLVE push on the subject (almost static).
const READ_PUSH: f32 = 1.025;

/// Headline set on at least two lines (when it has two words), so a layout can
/// put a line behind the subject and one elsewhere.
fn headline_block(ctx: &Ctx, text: &str, max_w: f32, max_h: f32, max_size: f32) -> Block {
    let block = ctx
        .ts
        .fit_block(Voice::HEADLINE, text, max_w, max_h, max_size, 4);
    if block.lines.len() >= 2 {
        return block;
    }
    let prepared = Typesetter::prepare(&Voice::HEADLINE, text);
    let words: Vec<&str> = prepared.split(' ').collect();
    if words.len() < 2 {
        return block;
    }
    let cut = words.len() / 2;
    let lines = vec![words[..cut].join(" "), words[cut..].join(" ")];
    let widest = lines
        .iter()
        .map(|l| ctx.ts.width(&Voice::HEADLINE, l, 100.0))
        .fold(1.0, f32::max);
    let size = (max_w / widest * 100.0)
        .min(max_h / (2.0 * Voice::HEADLINE.line_height))
        .min(max_size)
        .max(1.0);
    let line_widths = lines
        .iter()
        .map(|l| ctx.ts.width(&Voice::HEADLINE, l, size))
        .collect();
    Block {
        lines,
        size,
        line_widths,
        voice: Voice::HEADLINE,
    }
}

/// Stand-in geometry for the procedural plate (no delivered image).
fn placeholder_facts() -> SubjectFacts {
    SubjectFacts {
        aspect: 2.0 / 3.0,
        subject: NormBox {
            x: 0.1,
            y: 0.08,
            width: 0.8,
            height: 0.84,
        },
        head: NormBox {
            x: 0.33,
            y: 0.08,
            width: 0.34,
            height: 0.24,
        },
        grid: None,
        bottom_cut: false,
    }
}

/// Ink band of each line (cap-height band) for a block set at `top`.
fn line_boxes(ctx: &Ctx, block: &Block, top: f32, align: TextAlign, col: Rect) -> Vec<Rect> {
    let adv = block.line_advance();
    block
        .line_widths
        .iter()
        .enumerate()
        .map(|(i, &lw)| {
            let x = match align {
                TextAlign::Left => col.x,
                TextAlign::Center => col.cx() - lw / 2.0,
                TextAlign::Right => col.right() - lw,
            };
            let _ = ctx;
            Rect::new(
                x,
                top + i as f32 * adv + 0.14 * block.size,
                lw,
                0.74 * block.size,
            )
        })
        .collect()
}

/// A candidate's lines with their layering: a line that touches the head is
/// always behind the subject; `front` lines cross the body on paper strips.
fn layered(
    facts: &SubjectFacts,
    image: &Rect,
    rects: Vec<Rect>,
    front: impl Fn(usize) -> bool,
) -> Vec<LineBox> {
    let head = placement::to_canvas(image, facts.head);
    rects
        .into_iter()
        .enumerate()
        .map(|(i, r)| {
            let on_head = r.intersect(&head).is_some();
            let layering = if front(i) && !on_head {
                Layering::Front
            } else {
                Layering::Behind
            };
            let strip = layering == Layering::Front && facts.coverage(image, &r) > 0.04;
            LineBox {
                rect: r,
                layering,
                strip,
            }
        })
        .collect()
}

/// Everything the builder decides before emitting layers.
pub(crate) struct Layout {
    pub candidate: Candidate,
    /// Why this candidate won (tests / diagnostics).
    #[cfg_attr(not(test), allow(dead_code))]
    pub score: placement::Score,
    pub block: Block,
    pub align: TextAlign,
    pub role: AssetRole,
    pub facts: SubjectFacts,
}

/// Generate and score the internal candidates for this beat; return the best.
pub(crate) fn plan_layout(ctx: &Ctx, b: &B, c: Composition) -> Layout {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let idx = b.plan.index;
    let role = if plate::image(ctx, idx, AssetRole::HeroSubject).is_none()
        && plate::image(ctx, idx, AssetRole::Portrait).is_some()
    {
        AssetRole::Portrait
    } else {
        AssetRole::HeroSubject
    };
    let entry = plate::image(ctx, idx, role);
    let facts = entry.map_or_else(placeholder_facts, SubjectFacts::from_entry);
    let land = placement::side_by_side(&ctx.frame);
    let preferred = if c.variant == Variant::Mirror {
        Side::Left
    } else {
        Side::Right
    };
    let frame = Frame {
        w,
        h,
        margin: m,
        max_size: 170.0 * u,
        preferred,
        previous: None,
    };
    let statement = &b.beat.statement;
    let text_top = 0.17 * h;
    // A bottom-cut (half) figure bleeds off the frame; a whole one stands above the bottom margin.
    let bottom = if facts.bottom_cut {
        h + 0.01 * h
    } else {
        h - 0.07 * h
    };
    let (min_h, max_h) = (0.35 * h, 1.2 * h);
    let full_col = if land {
        Rect::new(m, 0.0, 0.58 * w, h)
    } else {
        Rect::new(m, 0.0, w - 2.0 * m, h)
    };
    let full_block = headline_block(
        ctx,
        statement,
        full_col.w,
        if land { 0.5 * h } else { 0.27 * h },
        170.0 * u,
    );

    let mut cands = Vec::new();
    for side in [preferred, opposite(preferred)] {
        let right = side == Side::Right;
        let align = if right {
            TextAlign::Left
        } else {
            TextAlign::Right
        };
        let cx = if right { 0.66 * w } else { 0.34 * w };
        let col = if land && !right {
            Rect::new(w - m - full_col.w, 0.0, full_col.w, h)
        } else {
            full_col
        };

        // Split: the head rises into the last line (lines behind the subject),
        // at the layout's subject column, or aimed past the end of the line
        // so the head only overlaps the line's tail.
        let n = full_block.lines.len();
        let adv = full_block.line_advance();
        let last_top = text_top + (n.saturating_sub(1)) as f32 * adv;
        let head_top = last_top + 0.22 * full_block.size;
        let rects = line_boxes(ctx, &full_block, text_top, align, col);
        let base = placement::fit_between(&facts, Some(head_top), bottom, None, cx, min_h, max_h);
        cands.push(Candidate {
            name: "split",
            mode: Interlock::Split,
            side,
            image: base,
            text_top,
            text_col: col,
            lines: layered(&facts, &base, rects, |_| false),
            size: full_block.size,
            support: None,
        });
        // split_end: a narrower column leaves room for the head at the tail
        // of the last line.
        let ncol = if right {
            Rect::new(col.x, col.y, 0.72 * col.w, col.h)
        } else {
            Rect::new(col.right() - 0.72 * col.w, col.y, 0.72 * col.w, col.h)
        };
        let nblock = headline_block(ctx, statement, ncol.w, 0.3 * h, 170.0 * u);
        let nrects = line_boxes(ctx, &nblock, text_top, align, ncol);
        let n_last_top =
            text_top + (nblock.lines.len().saturating_sub(1)) as f32 * nblock.line_advance();
        let probe = placement::fit_between(
            &facts,
            Some(n_last_top + 0.22 * nblock.size),
            bottom,
            None,
            cx,
            min_h,
            max_h,
        );
        let head = placement::to_canvas(&probe, facts.head);
        let last = nrects[nrects.len() - 1];
        let at_end = if right {
            last.right() + 0.15 * head.w
        } else {
            last.x - 0.15 * head.w
        };
        let aimed = Rect::new(probe.x + at_end - head.cx(), probe.y, probe.w, probe.h);
        cands.push(Candidate {
            name: "split_end",
            mode: Interlock::Split,
            side,
            image: aimed,
            text_top,
            text_col: ncol,
            lines: layered(&facts, &aimed, nrects, |_| false),
            size: nblock.size,
            support: None,
        });

        // Behind: the subject's upper body stands in front of the headline.
        let head_top = text_top + 0.35 * full_block.height();
        let image = placement::fit_between(&facts, Some(head_top), bottom, None, cx, min_h, max_h);
        let rects = line_boxes(ctx, &full_block, text_top, align, col);
        cands.push(Candidate {
            name: "behind",
            mode: Interlock::Behind,
            side,
            image,
            text_top,
            text_col: col,
            lines: layered(&facts, &image, rects, |_| false),
            size: full_block.size,
            support: None,
        });

        // Front: a large subject; the headline crosses its torso below the head.
        for share in [0.62, 0.72] {
            let head_top = bottom - share * h;
            let image =
                placement::fit_between(&facts, Some(head_top), bottom, None, cx, min_h, max_h);
            let head = placement::to_canvas(&image, facts.head);
            let top = head.bottom() + 0.035 * h;
            let block = headline_block(
                ctx,
                statement,
                col.w,
                (0.88 * h - top).clamp(0.08 * h, 0.27 * h),
                150.0 * u,
            );
            let rects = line_boxes(ctx, &block, top, align, col);
            cands.push(Candidate {
                name: if share < 0.7 { "front" } else { "front_large" },
                mode: Interlock::Front,
                side,
                image,
                text_top: top,
                text_col: col,
                lines: layered(&facts, &image, rects, |_| true),
                size: block.size,
                support: None,
            });
        }

        // Beside: the headline owns the negative space next to the subject.
        let col_w = if land { 0.46 * w } else { 0.54 * w - m };
        let bcol = if right {
            Rect::new(m, 0.0, col_w, h)
        } else {
            Rect::new(w - m - col_w, 0.0, col_w, h)
        };
        let bblock = headline_block(
            ctx,
            statement,
            bcol.w,
            if land { 0.6 * h } else { 0.42 * h },
            150.0 * u,
        );
        let bcx = if right { 0.72 * w } else { 0.28 * w };
        for share in [0.6, 0.72] {
            let image = placement::fit_between(
                &facts,
                Some(bottom - share * h),
                bottom,
                None,
                bcx,
                min_h,
                max_h,
            );
            let top = 0.2 * h;
            let rects = line_boxes(ctx, &bblock, top, align, bcol);
            cands.push(Candidate {
                name: if share < 0.7 {
                    "beside"
                } else {
                    "beside_large"
                },
                mode: Interlock::Beside,
                side,
                image,
                text_top: top,
                text_col: bcol,
                lines: layered(&facts, &image, rects, |_| false),
                size: bblock.size,
                support: None,
            });
        }
    }

    // Every candidate reserves room for the supporting items on its type side.
    let need = support_need(b, h, land);
    for c in &mut cands {
        // The column stops short of the subject silhouette where it can.
        let subject = c.subject(&facts);
        let (min_w, max_w) = if land {
            (0.24 * w, 0.34 * w)
        } else {
            (0.3 * w, 0.52 * w)
        };
        let col = if c.side == Side::Right {
            let col_w = (subject.x - 0.02 * w - m).clamp(min_w, max_w);
            Rect::new(m, 0.0, col_w, h)
        } else {
            let col_w = (w - m - subject.right() - 0.02 * w).clamp(min_w, max_w);
            Rect::new(w - m - col_w, 0.0, col_w, h)
        };
        c.support = placement::support_zone(&c.lines, col, need, 0.04 * h, 0.14 * h, 0.93 * h);
    }
    let (candidate, score) = placement::choose(cands, &facts, &frame).expect("candidates");
    let right = candidate.side == Side::Right;
    let align = if right {
        TextAlign::Left
    } else {
        TextAlign::Right
    };
    // Re-measure the chosen candidate's block (same inputs → same block).
    let block = match candidate.mode {
        Interlock::Split if candidate.name == "split_end" => {
            headline_block(ctx, statement, candidate.text_col.w, 0.3 * h, 170.0 * u)
        }
        Interlock::Split | Interlock::Behind => full_block,
        Interlock::Front => headline_block(
            ctx,
            statement,
            candidate.text_col.w,
            (0.88 * h - candidate.text_top).clamp(0.08 * h, 0.27 * h),
            150.0 * u,
        ),
        Interlock::Beside => headline_block(
            ctx,
            statement,
            candidate.text_col.w,
            if land { 0.6 * h } else { 0.42 * h },
            150.0 * u,
        ),
    };
    Layout {
        candidate,
        score,
        block,
        align,
        role,
        facts,
    }
}

/// Height the supporting items need: the primary label, plus a serif note
/// when there is a secondary.
fn support_need(b: &B, h: f32, land: bool) -> f32 {
    let label = if land { 0.16 * h } else { 0.075 * h };
    if b.beat.secondary.is_some() {
        label + 0.025 * h + 0.085 * h
    } else {
        label
    }
}

fn opposite(s: Side) -> Side {
    match s {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

/// The legacy interlock's primary-label slot: `(centre x, centre y, width,
/// height)`. `right` = the subject stands on the right (type on the left).
fn legacy_label_slot(ctx: &Ctx, cand: &Candidate, land: bool, right: bool) -> (f32, f32, f32, f32) {
    let (w, h, m) = (ctx.w, ctx.h, ctx.margin());
    let sh = if land { 0.16 * h } else { 0.075 * h };
    let sw = cand
        .support
        .map_or(if land { 0.28 * w } else { 0.5 * w }, |z| z.w);
    let zone_top = match cand.support {
        Some(z) => z.y,
        None => (cand.text_bottom() + 0.04 * h).min(0.8 * h),
    };
    let scy = zone_top + sh / 2.0;
    let sx = if right {
        m + sw / 2.0
    } else {
        w - m - sw / 2.0
    };
    (sx, scy, sw, sh)
}

// ---------------------------------------------------------------------------
// (0.10 Q) Subject-first layouts
//
// The owner's review of weak-model reels: assets too small or pushed into the
// background, headline type printed over people, every image beat laid out the
// same. A subject-first layout puts the TYPE and the SUBJECT in disjoint
// regions (so nothing is ever printed over the picture and a head is never
// covered), makes the subject as large as the remaining room allows (never
// under `SUBJECT_MIN_AREA`), and differs from the previous image beat's.
//
//   TextTopSubjectBelow    title block on top, the subject stands below it
//   SubjectRightTextLeft   title column left, subject bottom-right
//   SubjectLeftTextRight   mirror
//   SubjectCenterTextBand  subject centred above a title band at the bottom
//
// With a voice-over the builders already lay out on a canvas shortened by the
// caption lane (`LayoutFrame::with_bottom_reserve`), so "the bottom" of every
// layout is above the captions.
//
// `plan_image_layouts` decides one layout per image beat before any beat is
// built; `solve` is the pure geometry both the planner and the builder use.
// ---------------------------------------------------------------------------

/// Gap between the type region and the subject: a fraction of the short side
/// (beside) / of the height (stacked).
const GAP_X: f32 = 0.03;
const GAP_Y: f32 = 0.022;
/// The READ push grows the subject about 2.5 %: keep room for it.
const PUSH_ROOM: f32 = 1.02;
/// Side margin of the subject's bounds, in margins (`m` = 84 u): a 1.12 zoom
/// about the centre carries the edge of a 1080 px canvas out by 65 px.
const LIVE_MARGIN: f32 = 0.8;
/// A layout fits when the subject reaches this multiple of `SUBJECT_MIN_AREA`
/// (headroom for measuring differences and rounding)...
const AREA_HEADROOM: f32 = 1.04;
/// ...and the headline only claims room the subject can spare once the subject
/// is this far above the minimum.
const AREA_COMFORT: f32 = 1.35;
/// Smallest headline a subject-first layout accepts, in `u` (the floor the
/// headline budget shrinks to before it would cut the text).
const MIN_TITLE_U: f32 = 46.0;
/// Title column widths tried by the side layouts (fractions of the canvas
/// width): stacked canvases / side-by-side canvases.
const SIDE_COLUMNS: [f32; 3] = [0.50, 0.56, 0.62];
const SIDE_COLUMNS_WIDE: [f32; 3] = [0.44, 0.50, 0.56];
/// Headline block heights tried, largest first (fractions of the canvas
/// height; all within `taste_rules::HEADLINE_MAX_H_WITH_SUBJECT`).
const TITLE_BUDGETS: [f32; 5] = [0.28, 0.23, 0.19, 0.15, 0.12];
/// The size (u) the headline budget shrinks a headline to before it would cut
/// its text (`recipes::HEADLINE_FLOOR_U`).
const BUDGET_FLOOR_U: f32 = 46.0;
/// Body copy ("deck") under a derived title: gap, size and floor, in `u`
/// (the values `recipes::deck` uses).
const DECK_GAP_U: f32 = 22.0;
const DECK_SIZE_U: f32 = 40.0;
const DECK_MIN_U: f32 = 30.0;

/// Under the kicker row (the kicker and its rule end at `0.105 h + 57 u`).
fn content_top(ctx: &Ctx) -> f32 {
    0.105 * ctx.h + 57.0 * ctx.u + 0.018 * ctx.h
}

/// The role the beat's subject image is delivered under.
fn hero_role(ctx: &Ctx, idx: usize) -> AssetRole {
    if plate::image(ctx, idx, AssetRole::HeroSubject).is_none()
        && plate::image(ctx, idx, AssetRole::Portrait).is_some()
    {
        AssetRole::Portrait
    } else {
        AssetRole::HeroSubject
    }
}

/// The smallest subject alpha-bounds area (canvas pixels²) the beat must
/// reach: planned against the real canvas (`BeatPlan::image_min_px`), else
/// `SUBJECT_MIN_AREA` of the layout frame.
fn min_px(ctx: &Ctx, plan: &BeatPlan) -> f32 {
    if plan.image_min_px > 0.0 {
        plan.image_min_px
    } else {
        subject_rules::subject_min_area(&ctx.frame) * ctx.frame.w * ctx.frame.h
    }
}

/// Everything a subject-first layout decides.
#[derive(Debug, Clone)]
pub(crate) struct SubjectFirst {
    #[cfg_attr(not(test), allow(dead_code))]
    pub layout: ImageLayout,
    /// The display headline.
    pub block: Block,
    pub align: TextAlign,
    /// Canvas y of the headline block's top.
    pub title_top: f32,
    /// Body copy under the headline (a derived title without a voice-over).
    pub deck: Option<Block>,
    /// The type region: headline, deck, label slot and note room.
    pub type_rect: Rect,
    /// Slot of the primary label (inside `type_rect`, under the headline).
    pub label: Rect,
    /// The whole image (aspect kept), canvas pixels.
    pub image: Rect,
    /// The subject's alpha bounds, canvas pixels.
    pub subject: Rect,
    pub role: AssetRole,
    #[cfg_attr(not(test), allow(dead_code))]
    pub facts: SubjectFacts,
    /// Visible subject area over the required minimum (1.0 = exactly the minimum).
    pub reach: f32,
    /// `reach` clears the minimum with headroom.
    pub fits: bool,
}

/// Which side of its region a standing subject keeps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Left,
    Centre,
    Right,
}

/// Largest subject (aspect kept) whose alpha bounds fit `region`, standing on
/// its bottom edge and kept to `anchor` horizontally: `(image height, alpha
/// bounds)`. With `image_floor` the region's bottom is where the IMAGE ends
/// (its transparent margin below the feet included), so the picture never
/// leaves the layout canvas; otherwise it is where the subject's feet stand.
/// The feet never stand lower than `feet_limit` (the camera carries them
/// outward; `f32::MAX` = no limit).
fn stand_in(
    facts: &SubjectFacts,
    region: Rect,
    anchor: Anchor,
    image_floor: bool,
    feet_limit: f32,
) -> Option<(f32, Rect)> {
    let uw = facts.subject.width * facts.aspect;
    let uh = facts.subject.height;
    if !(region.w > 1.0 && region.h > 1.0 && uw > 0.0 && uh > 0.0) {
        return None;
    }
    // Margin under the feet, as a fraction of the image height.
    let below = if image_floor {
        (1.0 - facts.subject.y - facts.subject.height).max(0.0)
    } else {
        0.0
    };
    let by_feet = (feet_limit.min(1.0e6) - region.y) / uh;
    if by_feet <= 0.0 {
        return None;
    }
    let ih = (region.w / uw).min(region.h / (uh + below)).min(by_feet) / PUSH_ROOM;
    let (sw, sh) = (uw * ih, uh * ih);
    let x = match anchor {
        Anchor::Left => region.x,
        Anchor::Centre => region.cx() - sw / 2.0,
        Anchor::Right => region.right() - sw,
    };
    let feet = (region.bottom() - below * ih).min(feet_limit);
    Some((ih, Rect::new(x, feet - sh, sw, sh)))
}

/// The bigger of two candidate subjects (the first wins a tie).
fn larger(a: Option<(f32, Rect)>, b: Option<(f32, Rect)>) -> Option<(f32, Rect)> {
    match (a, b) {
        (Some(x), Some(y)) => Some(if y.0 > x.0 { y } else { x }),
        (x, None) => x,
        (None, y) => y,
    }
}

/// One attempt at `layout` with the headline held to `budget` (a fraction of
/// the canvas height); side layouts set it in a column of `col_frac` of the
/// canvas width.
#[allow(clippy::too_many_arguments)]
fn attempt(
    ctx: &Ctx,
    b: &B,
    c: Composition,
    layout: ImageLayout,
    facts: &SubjectFacts,
    role: AssetRole,
    budget: f32,
    col_frac: f32,
) -> Option<SubjectFirst> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let frame = &ctx.frame;
    let land = placement::side_by_side(frame);
    let tall = frame.class == Format::Vertical;
    let mirror = c.variant == Variant::Mirror;

    let top = content_top(ctx);
    // The image ends at the bottom margin; a half figure bleeds off the canvas.
    let floor = if facts.bottom_cut {
        h + 0.01 * h
    } else {
        h - 0.5 * m
    };
    // The camera push and track, and the stage reframes of EVOLVE, zoom about
    // the canvas centre and carry what stands at the edge outward: x lands at
    // `c + z (x - pan - c)`, so the subject's bounds stay `pan + c (1 - 1/z)`
    // off the edge (the drift `place_subject` also keeps clear of).
    let cam = b.plan.lang.camera;
    let (shifts, step) = temporal::hierarchy_shifts(ctx.taste.rhythm);
    let zoom = cam.push.max(1.0) * (1.0 + step * shifts as f32);
    let drift = cam.track[0].abs() * u + w / 2.0 * (1.0 - 1.0 / zoom);
    let side = (LIVE_MARGIN * m).max(drift);
    let (live_l, live_r) = (side, w - side);
    // The same zoom (and a pan that moves the scene down) carries the feet
    // toward the bottom edge: a whole figure stands at least that far above it
    // (never lower than the 7 % the interlock keeps). A half figure bleeds off.
    let lift = cam.track[1].abs() * u * 1.05 + h / 2.0 * (1.0 - 1.0 / zoom);
    let feet_limit = if facts.bottom_cut {
        f32::MAX
    } else {
        h - lift.max(0.07 * h)
    };
    let gap_x = GAP_X * w.min(h);
    let gap_y = GAP_Y * h;

    // Headline.
    let side_layout = matches!(
        layout,
        ImageLayout::SubjectRightTextLeft | ImageLayout::SubjectLeftTextRight
    );
    let col_w = if side_layout {
        col_frac * w - m
    } else {
        w - 2.0 * m
    };
    let max_lines = if tall {
        HEADLINE_MAX_LINES_TALL
    } else {
        HEADLINE_MAX_LINES_WIDE
    };
    // The text layer box is 2 % + 2 px wider than the set text: fit the text
    // so that the box, not just the ink, stays inside the column.
    let block = ctx.ts.fit_block(
        Voice::HEADLINE,
        &b.plan.display.title,
        (col_w - 2.0) / 1.02,
        budget * h,
        150.0 * u,
        max_lines,
    );
    if block.size < MIN_TITLE_U * u {
        return None;
    }
    let align = match layout {
        ImageLayout::TextTopSubjectBelow if mirror => TextAlign::Right,
        ImageLayout::TextTopSubjectBelow | ImageLayout::SubjectRightTextLeft => TextAlign::Left,
        ImageLayout::SubjectLeftTextRight => TextAlign::Right,
        _ => TextAlign::Center,
    };
    let title_w = block.width() * 1.02 + 2.0;
    // Body copy under a derived title (no voice-over), in the same column.
    let deck = b.plan.display.body.as_deref().map(|text| {
        ctx.ts.fit_body(
            Voice::BODY,
            text,
            DECK_SIZE_U * u,
            DECK_MIN_U * u,
            col_w,
            BODY_MAX_LINES,
        )
    });
    let deck_h = deck.as_ref().map_or(0.0, |d| DECK_GAP_U * u + d.height());
    let deck_w = deck.as_ref().map_or(0.0, |d| d.width() * 1.02 + 2.0);
    let type_w = title_w.max(deck_w).max(0.6 * col_w).min(col_w);
    let type_x = match align {
        TextAlign::Left => m,
        TextAlign::Right => w - m - type_w,
        TextAlign::Center => (w - type_w) / 2.0,
    };

    // The label (primary phrase) and an optional note sit under the headline.
    let lab_h = if land { 0.12 * h } else { 0.06 * h };
    let note_h = match b.beat.secondary.as_ref() {
        None => 0.0,
        Some(s) if s.kind() == SubjectKind::Object => 0.125 * h,
        Some(_) => 0.11 * h,
    };
    let gap_t = 0.025 * h;
    let type_h = block.height() + deck_h + gap_t + lab_h + note_h;
    // The band layout stands its type at the bottom of the canvas, above the
    // safe edge by the camera's lift: the push and EVOLVE reframes carry content
    // at the margin outward, and a title must not leave the frame doing it.
    let type_top = match layout {
        ImageLayout::SubjectCenterTextBand => h - lift.max(1.5 * m) - type_h,
        _ => top,
    };
    let type_rect = Rect::new(type_x, type_top, type_w, type_h);
    // A phrase label is set as wide as it needs (not as the whole column) so it
    // lines up with the headline's alignment.
    let label_w = match &b.beat.primary {
        Subject::Phrase(_) => {
            let text = b.beat.primary.display_text().unwrap_or("");
            let lb = ctx
                .ts
                .fit_block(Voice::HEADLINE, text, type_w, lab_h, 200.0 * u, 3);
            (lb.width() + 1.0).min(type_w)
        }
        _ => type_w,
    };
    let label_x = match align {
        TextAlign::Left => type_x,
        TextAlign::Right => type_x + type_w - label_w,
        TextAlign::Center => type_x + (type_w - label_w) / 2.0,
    };
    let label = Rect::new(
        label_x,
        type_top + block.height() + deck_h + gap_t,
        label_w,
        lab_h,
    );

    // The subject takes the largest room that is disjoint from the type.
    let live_w = live_r - live_l;
    let below = |from: f32| Rect::new(live_l, from, live_w, floor - from);
    let stand = match layout {
        ImageLayout::TextTopSubjectBelow => stand_in(
            facts,
            below(type_rect.bottom() + gap_y),
            Anchor::Centre,
            true,
            feet_limit,
        ),
        ImageLayout::SubjectCenterTextBand => {
            let bottom = type_rect.y - gap_y;
            stand_in(
                facts,
                Rect::new(live_l, top, live_w, bottom - top),
                Anchor::Centre,
                false,
                f32::MAX,
            )
        }
        ImageLayout::SubjectRightTextLeft => {
            let x0 = type_rect.right() + gap_x;
            larger(
                stand_in(
                    facts,
                    Rect::new(x0, top, live_r - x0, floor - top),
                    Anchor::Right,
                    true,
                    feet_limit,
                ),
                stand_in(
                    facts,
                    below(type_rect.bottom() + gap_y),
                    Anchor::Right,
                    true,
                    feet_limit,
                ),
            )
        }
        ImageLayout::SubjectLeftTextRight => {
            let x1 = type_rect.x - gap_x;
            larger(
                stand_in(
                    facts,
                    Rect::new(live_l, top, x1 - live_l, floor - top),
                    Anchor::Left,
                    true,
                    feet_limit,
                ),
                stand_in(
                    facts,
                    below(type_rect.bottom() + gap_y),
                    Anchor::Left,
                    true,
                    feet_limit,
                ),
            )
        }
        ImageLayout::Legacy => None,
    };
    let (ih, subject) = stand?;
    let iw = ih * facts.aspect;
    let image = Rect::new(
        subject.x - facts.subject.x * iw,
        subject.y - facts.subject.y * ih,
        iw,
        ih,
    );
    let visible = subject.intersect(&Rect::new(0.0, 0.0, w, h));
    let reach = visible.map_or(0.0, |r| r.area()) / min_px(ctx, b.plan).max(1.0);
    Some(SubjectFirst {
        layout,
        block,
        align,
        title_top: type_top,
        deck,
        type_rect,
        label,
        image,
        subject,
        role,
        facts: facts.clone(),
        reach,
        fits: false,
    })
}

/// The subject-first geometry of `layout` for this beat: the largest headline
/// (of [`TITLE_BUDGETS`]) that leaves the subject comfortably above
/// `SUBJECT_MIN_AREA`, else the largest that reaches it, else the layout with
/// the biggest subject (`fits = false`). `None` without a delivered image or a
/// headline that cannot be set at a readable size.
pub(crate) fn solve(ctx: &Ctx, b: &B, c: Composition, layout: ImageLayout) -> Option<SubjectFirst> {
    let role = hero_role(ctx, b.plan.index);
    let entry = plate::image(ctx, b.plan.index, role)?;
    let facts = SubjectFacts::from_entry(entry);
    let side_layout = matches!(
        layout,
        ImageLayout::SubjectRightTextLeft | ImageLayout::SubjectLeftTextRight
    );
    // Side layouts try a narrow title column first and widen it when the
    // headline cannot be set at a readable size (long titles, wide faces).
    let columns: &[f32] = match (side_layout, placement::side_by_side(&ctx.frame)) {
        (false, _) => &[0.0],
        (true, true) => &SIDE_COLUMNS_WIDE,
        (true, false) => &SIDE_COLUMNS,
    };
    let mut fitting: Option<SubjectFirst> = None;
    let mut best: Option<SubjectFirst> = None;
    for &col in columns {
        for budget in TITLE_BUDGETS {
            let Some(sf) = attempt(ctx, b, c, layout, &facts, role, budget, col) else {
                continue;
            };
            if sf.reach >= AREA_COMFORT {
                return Some(SubjectFirst { fits: true, ..sf });
            }
            if fitting.is_none() && sf.reach >= AREA_HEADROOM {
                fitting = Some(sf.clone());
            }
            if best.as_ref().is_none_or(|x| sf.reach > x.reach) {
                best = Some(sf);
            }
        }
    }
    match fitting {
        Some(sf) => Some(SubjectFirst { fits: true, ..sf }),
        None => best,
    }
}

/// The layouts to try for the `nth` image beat of a piece: the rotation from
/// `seed % 4` advanced one step per image beat. Side-by-side canvases (wide,
/// 6:5, 4:3) prefer the left/right layouts: those first, the stacked ones as
/// last resorts.
#[cfg(test)]
pub(crate) fn rotation_order(frame: &LayoutFrame, seed: u64, nth: usize) -> Vec<ImageLayout> {
    rotation_order_from(frame, (seed % 4) as usize + nth)
}

/// [`rotation_order`] starting at entry `begin` of [`IMAGE_LAYOUT_ROTATION`]
/// (0.23: under a direction seed `begin` is the image beat's entry of the
/// seeded sequence).
fn rotation_order_from(frame: &LayoutFrame, begin: usize) -> Vec<ImageLayout> {
    let order: Vec<ImageLayout> = (0..IMAGE_LAYOUT_ROTATION.len())
        .map(|k| IMAGE_LAYOUT_ROTATION[(begin + k) % IMAGE_LAYOUT_ROTATION.len()])
        .collect();
    if !placement::side_by_side(frame) {
        return order;
    }
    let (sides, stacked): (Vec<_>, Vec<_>) = order.into_iter().partition(|l| {
        matches!(
            l,
            ImageLayout::SubjectRightTextLeft | ImageLayout::SubjectLeftTextRight
        )
    });
    sides.into_iter().chain(stacked).collect()
}

/// Among the solved layouts (in preference order) the first that fits and is
/// `allowed`; else the allowed one with the biggest subject.
fn first_fitting(
    solved: &[(ImageLayout, Option<SubjectFirst>)],
    allowed: impl Fn(ImageLayout) -> bool,
) -> Option<ImageLayout> {
    if let Some((l, _)) = solved
        .iter()
        .find(|(l, s)| allowed(*l) && s.as_ref().is_some_and(|s| s.fits))
    {
        return Some(*l);
    }
    let mut best: Option<(ImageLayout, f32)> = None;
    for (l, s) in solved {
        if let Some(s) = s.as_ref().filter(|_| allowed(*l)) {
            if best.is_none_or(|(_, r)| s.reach > r) {
                best = Some((*l, s.reach));
            }
        }
    }
    best.map(|(l, _)| l)
}

/// Pick the layout for a beat under `variety`: the first of the rotation that
/// differs from the previous image beat's and fits; else the biggest subject
/// among those that differ.
fn choose_rotated(
    ctx: &Ctx,
    b: &B,
    c: Composition,
    begin: usize,
    previous: Option<ImageLayout>,
) -> ImageLayout {
    let order = rotation_order_from(&ctx.frame, begin);
    let solved: Vec<(ImageLayout, Option<SubjectFirst>)> =
        order.iter().map(|&l| (l, solve(ctx, b, c, l))).collect();
    first_fitting(&solved, |l| Some(l) != previous).unwrap_or(order[0])
}

/// Whether the legacy interlock would break a taste rule for this beat: a
/// headline line (or the label slot) touching the subject's head region, or a
/// subject smaller than `SUBJECT_MIN_AREA`. The interlock stays as it is
/// otherwise (byte-identical goldens).
fn legacy_violates(ctx: &Ctx, b: &B, c: Composition) -> bool {
    let lay = plan_layout(ctx, b, c);
    let cand = &lay.candidate;
    let head = cand.head(&lay.facts);
    let land = placement::side_by_side(&ctx.frame);
    let (sx, scy, sw, sh) = legacy_label_slot(ctx, cand, land, cand.side == Side::Right);
    let label = Rect::new(sx - sw / 2.0, scy - sh / 2.0, sw, sh);
    // A line's text box is a little wider and taller than its ink band.
    let line_hits_head = |l: &LineBox| {
        let (px, py) = (0.03 * l.rect.w + 4.0 * ctx.u, 0.04 * l.rect.h);
        Rect::new(
            l.rect.x - px,
            l.rect.y - py,
            l.rect.w + 2.0 * px,
            l.rect.h + 2.0 * py,
        )
        .intersect(&head)
        .is_some()
    };
    let head_hit = cand.lines.iter().any(line_hits_head) || label.intersect(&head).is_some();
    let visible = cand
        .subject(&lay.facts)
        .intersect(&Rect::new(0.0, 0.0, ctx.w, ctx.h))
        .map_or(0.0, |r| r.area());
    head_hit || visible < min_px(ctx, b.plan)
}

/// Pick the layout for a beat without `variety`: the legacy interlock unless it
/// would break a taste rule, then `TextTopSubjectBelow` (or, when the subject
/// does not fit under a title, the first layout whose subject fits).
fn choose_default(ctx: &Ctx, b: &B, c: Composition) -> ImageLayout {
    if !legacy_violates(ctx, b, c) {
        return ImageLayout::Legacy;
    }
    const FALLBACKS: [ImageLayout; 4] = [
        ImageLayout::TextTopSubjectBelow,
        ImageLayout::SubjectRightTextLeft,
        ImageLayout::SubjectLeftTextRight,
        ImageLayout::SubjectCenterTextBand,
    ];
    let solved: Vec<(ImageLayout, Option<SubjectFirst>)> = FALLBACKS
        .iter()
        .map(|&l| (l, solve(ctx, b, c, l)))
        .collect();
    first_fitting(&solved, |_| true).unwrap_or(ImageLayout::Legacy)
}

/// The composition of beat `plan.index` when it is a TypeImageInterlock beat
/// with a delivered subject image (the beats that take an [`ImageLayout`]).
fn image_composition(ctx: &Ctx, plan: &BeatPlan, beat: &Beat) -> Option<Composition> {
    plate::subject_image(ctx, plan.index)?;
    let comp = super::select(super::SelectInput {
        beat,
        language: plan.lang.language,
        format: ctx.format,
        side_by_side: placement::side_by_side(&ctx.frame),
        seed: plan.seed,
        has_subject_image: true,
        has_object_image: plate::image(ctx, plan.index, AssetRole::HeroObject).is_some(),
        visual: &ctx.taste.visual,
        look_grammar: ctx.art.as_ref().and_then(|(a, _)| a.fx.grammar),
    });
    (comp.grammar == Grammar::TypeImageInterlock).then_some(comp)
}

/// Decide the [`ImageLayout`] of every TypeImageInterlock beat (stored in
/// `BeatPlan::image_layout`; every other beat keeps `Legacy`).
///
/// * `variety = Some(seed)`: image beats rotate through
///   [`IMAGE_LAYOUT_ROTATION`] starting at `seed % 4`, skipping the previous
///   image beat's layout (see [`rotation_order`]).
/// * `variety = None`: the legacy interlock, byte-identical, unless it would
///   cover the subject's head or leave it under `SUBJECT_MIN_AREA`: those beats
///   fall back to `TextTopSubjectBelow`.
///
/// Layouts are judged on the canvas the builders will see: shortened by
/// `caption_reserve` (the caption lane of a voice-over) and, on tall canvases,
/// laid out on the 1920u budget (see `recipes::compose`). The minimum subject
/// area is that of the REAL canvas and is recorded in `BeatPlan::image_min_px`.
pub(crate) fn plan_image_layouts(
    ctx: &mut Ctx,
    plans: &mut [BeatPlan],
    beats: &[Beat],
    variety: Option<u64>,
    caption_reserve: Option<f32>,
) {
    let (real_frame, h_real) = (ctx.frame, ctx.h);
    let real_min_px = subject_rules::subject_min_area(&real_frame) * real_frame.w * real_frame.h;
    if let Some(px) = caption_reserve {
        ctx.frame = real_frame.with_bottom_reserve(px);
    }
    ctx.h = ctx.frame.content_h();
    // (0.23) Under a direction seed the image layouts start from a seeded
    // sequence in which neighbouring image beats never share a start.
    let seeded: Option<Vec<usize>> = ctx
        .direction_seed
        .filter(|_| variety.is_some())
        .map(|seed| direction::seeded_sequence(seed, direction::Dim::ImageLayout, plans.len(), 4));
    let mut previous: Option<ImageLayout> = None;
    let mut nth = 0usize;
    for i in 0..plans.len().min(beats.len()) {
        // Builders see the beat with its display title as the statement.
        let titled;
        let beat: &Beat = if plans[i].display.derived {
            let mut t = beats[i].clone();
            t.statement = plans[i].display.title.clone();
            titled = t;
            &titled
        } else {
            &beats[i]
        };
        let Some(c) = image_composition(ctx, &plans[i], beat) else {
            continue;
        };
        // Narrow figures get a proportionally smaller minimum (taste_rules).
        let aspect =
            plate::subject_image(ctx, plans[i].index).and_then(subject_rules::entry_subject_aspect);
        plans[i].image_min_px = if aspect.is_some() {
            subject_rules::subject_min_area_for(&real_frame, aspect) * real_frame.w * real_frame.h
        } else {
            real_min_px
        };
        // The headline budget the builder will set (an image beat shows a
        // subject): the layouts are judged on the headline they will really get.
        ctx.ts.set_headline_budget(Some(HeadlineBudget {
            text: Typesetter::prepare(&Voice::HEADLINE, &beat.statement),
            max_h: HEADLINE_MAX_H_WITH_SUBJECT * ctx.h,
            max_lines: if real_frame.class == Format::Vertical {
                HEADLINE_MAX_LINES_TALL
            } else {
                HEADLINE_MAX_LINES_WIDE
            },
            min_size: (BUDGET_FLOOR_U * ctx.u).max(real_frame.min_type_px),
        }));
        let layout = {
            let b = B {
                reveals: Vec::new(),
                plan: &plans[i],
                beat,
                stage: Vec::new(),
                motions: Vec::new(),
                anchor: None,
                placed: Vec::new(),
                split_above: None,
                focal: None,
            };
            match variety {
                Some(seed) => {
                    // (0.23) The rotation starts at the image beat's entry of
                    // the seeded sequence (neighbouring image beats differ), the
                    // fit checks and the skip of the previous layout as ever.
                    let begin = match &seeded {
                        Some(seq) => seq.get(nth).copied().unwrap_or(nth),
                        None => (seed % 4) as usize + nth,
                    };
                    choose_rotated(ctx, &b, c, begin, previous)
                }
                None => choose_default(ctx, &b, c),
            }
        };
        ctx.ts.set_headline_budget(None);
        plans[i].image_layout = layout;
        if seeded.is_some() {
            super::note_rotation(ctx, plans[i].index, "image_layout", layout.name());
        }
        if variety.is_some() {
            previous = Some(layout);
            nth += 1;
        }
    }
    ctx.frame = real_frame;
    ctx.h = h_real;
}

/// Build a TypeImageInterlock beat in a subject-first layout.
fn build_subject_first(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    sf: SubjectFirst,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let beat = b.beat;
    let life = plan.life;
    let t = plan.enter_at();
    // The type reads from the left unless the title is right-aligned.
    let type_left = sf.align != TextAlign::Right;

    recipes::ghost(ctx, b, 0.6 * h);

    // Halftone field behind the subject, kept out of the type region: toward
    // the type like the interlock, else behind the far shoulder, else centred.
    let visible = sf
        .subject
        .intersect(&Rect::new(0.0, 0.0, w, h))
        .unwrap_or(sf.subject);
    let field_w = 0.8 * visible.w;
    let candidates = [
        Rect::new(
            if type_left {
                visible.x - 0.18 * visible.w
            } else {
                visible.right() + 0.18 * visible.w - field_w
            },
            visible.y + 0.12 * visible.h,
            field_w,
            0.62 * visible.h,
        ),
        Rect::new(
            if type_left {
                visible.right() - 0.55 * field_w
            } else {
                visible.x - 0.45 * field_w
            },
            visible.y + 0.04 * visible.h,
            field_w,
            0.5 * visible.h,
        ),
        Rect::new(
            visible.cx() - field_w / 2.0,
            visible.y + 0.1 * visible.h,
            field_w,
            0.6 * visible.h,
        ),
    ];
    let field = candidates
        .iter()
        .find(|f| f.intersect(&sf.type_rect).is_none())
        .copied()
        .unwrap_or(candidates[2]);
    let tex_id = b.id("subject_field");
    let mut tex = texture_layer(
        &tex_id,
        (field.x, field.y, field.w, field.h),
        TextureSpec {
            material: Material::Halftone,
            seed: mix(plan.seed, 0x51E1),
            color: ctx.palette.accent,
            intensity: 0.75,
            scale: 14.0 * u,
            animated: false,
        },
        8,
    );
    tex.depth = recipes::plane(plan.lang.depth.background);
    b.motions.push(mo::mask(
        &tex_id,
        t + 0.15,
        0.9,
        if type_left {
            Direction::Right
        } else {
            Direction::Left
        },
        Easing::OutCubic,
    ));
    b.push(tex);

    // The headline: one pass (language stagger), in its own region; the body
    // copy of a derived title reads in under it.
    let (_, head_done) = recipes::headline_lines(
        ctx,
        b,
        "head",
        &sf.block,
        sf.title_top,
        sf.align,
        ctx.palette.ink,
        t + 0.1,
        true,
    );
    if let Some(deck) = &sf.deck {
        recipes::headline_lines(
            ctx,
            b,
            "deck",
            deck,
            sf.title_top + sf.block.height() + DECK_GAP_U * u,
            sf.align,
            ctx.palette.ink,
            head_done,
            false,
        );
    }

    // The subject on its own (slightly nearer) plane, almost static through
    // READ/EVOLVE.
    let img = sf.image;
    let subject_plane = 1.0 + SUBJECT_DEPTH_SHARE * (plan.lang.depth.foreground - 1.0);
    let image = plate::place_subject_image(
        ctx,
        b,
        carries,
        "subject",
        sf.role,
        (img.x, img.y, img.w, img.h),
        Z_IMAGE,
        recipes::plane(subject_plane),
        t + 0.25,
    );
    let image_id = image.id.clone();
    for l in image.layers {
        b.push(l);
    }
    if image.shared {
        b.split_above = Some(Z_IMAGE);
    }
    if !image.shared && life.anticipate > life.read + 0.3 {
        b.motions.push(mo::scale(
            &image_id,
            life.read,
            life.anticipate - life.read,
            1.0,
            READ_PUSH,
            Easing::InOutCubic,
        ));
    }

    // Primary label, then a serif note (or a second object), at EVOLVE
    // events, in the room the type region reserved under the headline.
    let sec = beat.secondary.as_ref();
    let tail = plate::subject_tail(b, &beat.primary).max(sec.map_or(0.0, |s| {
        if s.kind() == SubjectKind::Object {
            plate::subject_tail(b, s)
        } else {
            plate::serif_tail(b)
        }
    }));
    let ev = events(b, 1 + usize::from(sec.is_some()), tail);
    let slot = Slot {
        cx: sf.label.cx(),
        cy: sf.label.y + sf.label.h / 2.0,
        w: sf.label.w,
        h: sf.label.h,
    };
    if let Some(mut l) = recipes::place_subject(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        slot,
        (0.0, 0.0),
        ctx.palette.ink,
        ev[0],
        Entrance::Rise,
        None,
        None,
        "hero",
    )? {
        l.z_index = Z_FRONT + 1;
        b.push(l);
    }
    if let (Some(sec), Some(&at)) = (sec, ev.get(1)) {
        let note_top = sf.label.bottom() + 0.025 * h;
        if sec.kind() == SubjectKind::Object {
            let side = 0.1 * h;
            let cx = match sf.align {
                TextAlign::Left => m + side / 2.0,
                TextAlign::Right => w - m - side / 2.0,
                TextAlign::Center => w / 2.0,
            };
            let slot = Slot {
                cx,
                cy: note_top + side / 2.0,
                w: side,
                h: side,
            };
            if let Some(mut l) = recipes::place_subject(
                ctx,
                b,
                carries,
                Which::Secondary,
                sec,
                slot,
                (0.0, 0.0),
                ctx.palette.ink,
                at,
                Entrance::Rise,
                None,
                None,
                "support",
            )? {
                l.z_index = Z_FRONT + 1;
                b.push(l);
            }
        } else {
            let text = sec.display_text().unwrap_or("");
            let n1 = b.stage.len();
            plate::serif_note(
                ctx,
                b,
                "serif",
                text,
                note_top,
                sf.align,
                sf.type_rect.w,
                0.07 * h,
                64.0 * u,
                at,
            );
            for l in &mut b.stage[n1..] {
                l.z_index = l.z_index.max(Z_FRONT + 1);
            }
        }
    }

    b.anchor = Some(Slot {
        cx: if type_left {
            m + 130.0 * u
        } else {
            w - m - 130.0 * u
        },
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    // (0.10 Q) A subject-first layout (rotation under `variety`, or the
    // fallback when the interlock would cover a head or shrink the subject).
    let layout = b.plan.image_layout;
    if layout != ImageLayout::Legacy {
        if let Some(sf) = solve(ctx, b, c, layout) {
            return build_subject_first(ctx, b, carries, sf);
        }
    }
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let beat = b.beat;
    let life = plan.life;
    let t = plan.enter_at();
    let land = placement::side_by_side(&ctx.frame);

    let lay = plan_layout(ctx, b, c);
    let cand = &lay.candidate;
    let right = cand.side == Side::Right;

    recipes::ghost(ctx, b, 0.6 * h);

    // Material behind the subject: a halftone field on the background plane,
    // offset toward the type so image and page read as one print.
    let subject = cand.subject(&lay.facts);
    let visible = subject
        .intersect(&Rect::new(0.0, 0.0, w, h))
        .unwrap_or(subject);
    let field_w = 0.8 * visible.w;
    let mut field = Rect::new(
        if right {
            visible.x - 0.18 * visible.w
        } else {
            visible.right() + 0.18 * visible.w - field_w
        },
        visible.y + 0.12 * visible.h,
        field_w,
        0.62 * visible.h,
    );
    // Keep the print field out from under the supporting type.
    if let Some(z) = cand.support {
        if field.intersect(&z).is_some() {
            // No room on the type side: the field goes behind the far shoulder.
            field = Rect::new(
                if right {
                    visible.right() - 0.55 * field_w
                } else {
                    visible.x - 0.45 * field_w
                },
                visible.y + 0.04 * visible.h,
                field_w,
                0.5 * visible.h,
            );
        }
    }
    let tex_id = b.id("subject_field");
    let mut tex = texture_layer(
        &tex_id,
        (field.x, field.y, field.w, field.h),
        TextureSpec {
            material: Material::Halftone,
            seed: mix(plan.seed, 0x51E1),
            color: ctx.palette.accent,
            intensity: 0.75,
            scale: 14.0 * u,
            animated: false,
        },
        8,
    );
    tex.depth = recipes::plane(plan.lang.depth.background);
    b.motions.push(mo::mask(
        &tex_id,
        t + 0.15,
        0.9,
        if right {
            Direction::Right
        } else {
            Direction::Left
        },
        Easing::OutCubic,
    ));
    b.push(tex);

    // Headline, one pass (language stagger), then per-line layering.
    let n0 = b.stage.len();
    recipes::headline_lines(
        ctx,
        b,
        "head",
        &lay.block,
        cand.text_top,
        lay.align,
        ctx.palette.ink,
        t + 0.1,
        true,
    );
    let adv = lay.block.line_advance().max(1.0);
    let line_of = |y: f32| -> usize {
        (((y - cand.text_top) / adv).round().max(0.0) as usize).min(cand.lines.len() - 1)
    };
    let mut line_ids: Vec<(usize, String)> = Vec::new();
    for l in &mut b.stage[n0..] {
        let i = line_of(l.y);
        line_ids.push((i, l.id.clone()));
        let base = match cand.lines[i].layering {
            Layering::Behind => Z_BACK,
            Layering::Front => Z_FRONT,
        };
        l.z_index = base + (l.z_index - Z_BACK).max(0);
    }
    // Paper strips under front lines that cross the subject.
    for (i, line) in cand.lines.iter().enumerate().filter(|(_, l)| l.strip) {
        let pad = (0.16 * lay.block.size, 0.12 * lay.block.size);
        let id = b.id(&format!("strip.{i}"));
        let mut strip = base_layer(
            id.clone(),
            (
                line.rect.x - pad.0,
                line.rect.y - pad.1,
                line.rect.w + 2.0 * pad.0,
                line.rect.h + 2.0 * pad.1,
            ),
            LayerKind::Rectangle {
                fill: ctx.palette.card,
                stroke: None,
            },
            Z_STRIP,
        );
        strip.rotation_degrees = if i % 2 == 0 { -0.6 } else { 0.5 };
        // The strip lands just before its line's first word.
        let first = b
            .motions
            .iter()
            .filter(|mo| {
                line_ids
                    .iter()
                    .any(|(li, lid)| *li == i && *lid == mo.target)
            })
            .map(|mo| mo.start)
            .fold(f64::MAX, f64::min);
        let at = if first == f64::MAX { t + 0.1 } else { first };
        b.motions.push(mo::mask(
            &id,
            (at - 0.12).max(0.0),
            0.4,
            if right {
                Direction::Right
            } else {
                Direction::Left
            },
            Easing::OutQuint,
        ));
        b.push(strip);
    }

    // The subject image on its own (slightly nearer) plane, with an almost
    // static push through READ/EVOLVE.
    let img = cand.image;
    let subject_plane = 1.0 + SUBJECT_DEPTH_SHARE * (plan.lang.depth.foreground - 1.0);
    let image = plate::place_subject_image(
        ctx,
        b,
        carries,
        "subject",
        lay.role,
        (img.x, img.y, img.w, img.h),
        Z_IMAGE,
        recipes::plane(subject_plane),
        t + 0.25,
    );
    let image_id = image.id.clone();
    for l in image.layers {
        b.push(l);
    }
    if image.shared {
        b.split_above = Some(Z_IMAGE);
    }
    // A shared image keeps its transform neutral per scene; the push is for
    // scene-local images only.
    if !image.shared && life.anticipate > life.read + 0.3 {
        b.motions.push(mo::scale(
            &image_id,
            life.read,
            life.anticipate - life.read,
            1.0,
            READ_PUSH,
            Easing::InOutCubic,
        ));
    }

    // Primary subject then a serif secondary, at evolve events, in the free
    // zone on the type side, below the headline.
    let sec = beat.secondary.as_ref();
    let tail = plate::subject_tail(b, &beat.primary).max(sec.map_or(0.0, |s| {
        if s.kind() == SubjectKind::Object {
            plate::subject_tail(b, s)
        } else {
            plate::serif_tail(b)
        }
    }));
    let ev = events(b, 1 + usize::from(sec.is_some()), tail);
    let (sx, scy, sw, sh) = legacy_label_slot(ctx, cand, land, right);
    let slot = Slot {
        cx: sx,
        cy: scy,
        w: sw,
        h: sh,
    };
    let slot_rect = Rect::new(sx - sw / 2.0, scy - sh / 2.0, sw, sh);
    // When the free zone still crosses the subject, the label sits on a paper
    // caption card so ink never lands on the photograph.
    if lay.facts.coverage(&img, &slot_rect) > 0.03 {
        let pad = 22.0 * u;
        let id = b.id("caption_card");
        let mut card = base_layer(
            id.clone(),
            (
                slot_rect.x - pad,
                slot_rect.y - pad,
                sw + 2.0 * pad,
                sh + 2.0 * pad,
            ),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.card,
                radius: 8.0 * u,
                stroke: Some(Stroke {
                    color: ctx.palette.ink.with_alpha(0x22),
                    width: 2.0 * u,
                }),
            },
            Z_FRONT,
        );
        card.rotation_degrees = if right { -1.2 } else { 1.2 };
        b.motions.push(mo::mask(
            &id,
            (ev[0] - 0.08).max(0.0),
            0.5,
            Direction::Up,
            Easing::OutQuint,
        ));
        b.push(card);
    }
    if let Some(mut l) = recipes::place_subject(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        slot,
        (0.0, 0.0),
        ctx.palette.ink,
        ev[0],
        Entrance::Rise,
        None,
        None,
        "hero",
    )? {
        l.z_index = Z_FRONT + 1;
        b.push(l);
    }
    if let (Some(sec), Some(&at)) = (sec, ev.get(1)) {
        let note_top = scy + sh / 2.0 + 0.025 * h;
        let note_w = cand
            .support
            .map_or(if land { 0.5 * w - m } else { 0.62 * w }, |z| z.w);
        let align = if right {
            TextAlign::Left
        } else {
            TextAlign::Right
        };
        if sec.kind() == SubjectKind::Object {
            let side = 0.1 * h;
            let slot = Slot {
                cx: if right {
                    m + side / 2.0
                } else {
                    w - m - side / 2.0
                },
                cy: note_top + side / 2.0,
                w: side,
                h: side,
            };
            if let Some(mut l) = recipes::place_subject(
                ctx,
                b,
                carries,
                Which::Secondary,
                sec,
                slot,
                (0.0, 0.0),
                ctx.palette.ink,
                at,
                Entrance::Rise,
                None,
                None,
                "support",
            )? {
                l.z_index = Z_FRONT + 1;
                b.push(l);
            }
        } else {
            let text = sec.display_text().unwrap_or("");
            let n1 = b.stage.len();
            plate::serif_note(
                ctx,
                b,
                "serif",
                text,
                note_top,
                align,
                note_w,
                0.07 * h,
                64.0 * u,
                at,
            );
            for l in &mut b.stage[n1..] {
                l.z_index = l.z_index.max(Z_FRONT + 1);
            }
        }
    }

    b.anchor = Some(Slot {
        cx: if right {
            m + 130.0 * u
        } else {
            w - m - 130.0 * u
        },
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::assets::{AssetManifest, ManifestEntry, NormPoint};
    use crate::compiler::grammar::{Grammar, Shape};
    use crate::compiler::{plan_timing, ApproxMeasure, AssetLibrary, FontSet, Palette};
    use crate::intent::{CreativeIntent, Format};
    use crate::style::StyleProfile;

    const INTENT: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[{"purpose":"emphasize","statement":"An office worker sits alone at 11:47 PM.","primary":{"kind":"phrase","value":"Office worker","meaning":"worker"},"secondary":{"kind":"phrase","value":"Unsure who to call"},"energy":"calm","keyword":"night"}]}"#;

    fn layout_for(entry: ManifestEntry, variant: Variant) -> Layout {
        let style = StyleProfile::default();
        let intent: CreativeIntent = serde_json::from_str(INTENT).expect("intent");
        let plans = plan_timing(&intent, &style);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_style(&style);
        let manifest = AssetManifest {
            assets: vec![entry],
            ..AssetManifest::empty()
        };
        let ctx = Ctx {
            reveals: Default::default(),
            warnings: Vec::new(),
            direction_seed: None,
            beat_params: BTreeMap::new(),
            direction_take: None,
            direction_beats: BTreeMap::new(),
            emotion: None,
            art: None,
            w: 1080.0,
            h: 1920.0,
            u: 1.0,
            style: &style,
            taste: crate::compiler::taste::resolve(&style),
            palette: Palette::for_style(&style),
            ts: Typesetter::new(&ApproxMeasure, &library.root, &fonts),
            library: &library,
            assets: BTreeMap::new(),
            format: Format::Vertical,
            frame: crate::compiler::layout_frame::LayoutFrame::for_format(Format::Vertical),
            manifest: &manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: 1,
            sequence: Default::default(),
        };
        let b = B {
            reveals: Vec::new(),
            plan: &plans[0],
            beat: &intent.beats[0],
            stage: Vec::new(),
            motions: Vec::new(),
            anchor: None,
            placed: Vec::new(),
            split_above: None,
            focal: None,
        };
        plan_layout(
            &ctx,
            &b,
            Composition {
                grammar: Grammar::TypeImageInterlock,
                variant,
                shape: Shape::Atomic,
                depiction: super::super::Depiction::Typographic,
                reason: "test",
            },
        )
    }

    fn worker() -> ManifestEntry {
        ManifestEntry {
            id: "beat_1.hero_subject".into(),
            path: "test_images/figure.png".into(),
            width: 600,
            height: 900,
            alpha: true,
            face_anchor: Some(NormPoint { x: 0.5, y: 0.21 }),
            ..Default::default()
        }
    }

    #[test]
    fn chosen_layout_never_puts_front_text_on_the_head() {
        for variant in [Variant::Standard, Variant::Mirror] {
            let lay = layout_for(worker(), variant);
            assert_eq!(lay.score.term("face_collision"), 0.0, "{:?}", lay.score);
            let head = lay.candidate.head(&lay.facts);
            for l in &lay.candidate.lines {
                if l.layering == Layering::Front {
                    assert!(l.rect.intersect(&head).is_none());
                }
            }
            assert!(lay.candidate.support.is_some(), "supports have room");
            assert!(lay.score.term("text_clip") < 0.01, "{:?}", lay.score);
            assert!(lay.score.term("head_clip") < 0.01, "{:?}", lay.score);
        }
    }

    #[test]
    fn mirror_prefers_the_subject_on_the_left() {
        assert_eq!(
            layout_for(worker(), Variant::Standard).candidate.side,
            Side::Right
        );
        assert_eq!(
            layout_for(worker(), Variant::Mirror).candidate.side,
            Side::Left
        );
    }

    #[test]
    fn layout_is_deterministic() {
        let a = layout_for(worker(), Variant::Standard);
        let b = layout_for(worker(), Variant::Standard);
        assert_eq!(a.candidate, b.candidate);
        assert_eq!(a.score, b.score);
    }

    // --- (0.10 Q) subject-first layouts ------------------------------------

    use crate::assets::{AssetAnalysis, EdgeContact, OccupancyGrid};

    /// A delivered person: `width x height` image whose alpha bounds fill
    /// `subject` (so its aspect and fill are what the layouts size by).
    fn person(width: u32, height: u32, subject: NormBox) -> ManifestEntry {
        ManifestEntry {
            id: "beat_1.hero_subject".into(),
            path: "test_images/figure.png".into(),
            width,
            height,
            alpha: true,
            face_anchor: Some(NormPoint { x: 0.5, y: 0.15 }),
            analysis: Some(AssetAnalysis {
                subject_bounds: subject,
                edges: EdgeContact::default(),
                coverage: 0.4,
                occupancy: OccupancyGrid {
                    cols: 16,
                    rows: (0..16).map(|_| "f".repeat(16)).collect(),
                },
                safe_regions: Vec::new(),
                head_estimate: Some(NormBox {
                    x: subject.x + 0.3 * subject.width,
                    y: subject.y,
                    width: 0.4 * subject.width,
                    height: 0.2 * subject.height,
                }),
                monochrome: false,
                mean_color: None,
            }),
            ..Default::default()
        }
    }

    /// A 600x900 whole figure with 5 % margins.
    fn standing() -> ManifestEntry {
        person(
            600,
            900,
            NormBox {
                x: 0.1,
                y: 0.05,
                width: 0.8,
                height: 0.9,
            },
        )
    }

    /// A shorter headline (the approximate test measure sets text wide).
    const SHORT_INTENT: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[{"purpose":"emphasize","statement":"Alone at the desk","primary":{"kind":"phrase","value":"Office worker","meaning":"worker"},"energy":"calm"}]}"#;

    /// Run `f` on the first beat of INTENT laid out on a `w x h` canvas.
    fn on_canvas<R>(
        canvas: (u32, u32),
        entry: ManifestEntry,
        variant: Variant,
        f: impl FnOnce(&Ctx, &B, Composition) -> R,
    ) -> R {
        on_canvas_with(INTENT, canvas, entry, variant, f)
    }

    fn on_canvas_with<R>(
        intent_json: &str,
        (w, h): (u32, u32),
        entry: ManifestEntry,
        variant: Variant,
        f: impl FnOnce(&Ctx, &B, Composition) -> R,
    ) -> R {
        let style = StyleProfile::default();
        let intent: CreativeIntent = serde_json::from_str(intent_json).expect("intent");
        let plans = plan_timing(&intent, &style);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_style(&style);
        let manifest = AssetManifest {
            assets: vec![entry],
            ..AssetManifest::empty()
        };
        let frame = crate::compiler::layout_frame::LayoutFrame::new(w, h).expect("frame");
        let ctx = Ctx {
            reveals: Default::default(),
            warnings: Vec::new(),
            direction_seed: None,
            beat_params: BTreeMap::new(),
            direction_take: None,
            direction_beats: BTreeMap::new(),
            emotion: None,
            art: None,
            w: w as f32,
            h: frame.content_h(),
            u: frame.u,
            style: &style,
            taste: crate::compiler::taste::resolve(&style),
            palette: Palette::for_style(&style),
            ts: Typesetter::new(&ApproxMeasure, &library.root, &fonts),
            library: &library,
            assets: BTreeMap::new(),
            format: frame.class,
            frame,
            manifest: &manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: 1,
            sequence: Default::default(),
        };
        let b = B {
            reveals: Vec::new(),
            plan: &plans[0],
            beat: &intent.beats[0],
            stage: Vec::new(),
            motions: Vec::new(),
            anchor: None,
            placed: Vec::new(),
            split_above: None,
            focal: None,
        };
        let c = Composition {
            grammar: Grammar::TypeImageInterlock,
            variant,
            shape: Shape::Atomic,
            depiction: super::super::Depiction::Typographic,
            reason: "test",
        };
        f(&ctx, &b, c)
    }

    const ALL_LAYOUTS: [ImageLayout; 4] = IMAGE_LAYOUT_ROTATION;
    const CANVASES: [(u32, u32); 4] = [(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)];

    #[test]
    fn type_and_subject_are_disjoint_and_the_head_stays_clear() {
        for canvas in CANVASES {
            for layout in ALL_LAYOUTS {
                let sf = on_canvas(canvas, standing(), Variant::Standard, |ctx, b, c| {
                    solve(ctx, b, c, layout)
                });
                let Some(sf) = sf else { continue };
                let (w, h) = (canvas.0 as f32, canvas.1 as f32);
                let what = format!("{layout:?} on {canvas:?}");
                assert_eq!(sf.layout, layout, "{what}");
                // Disjoint regions: not a pixel of the type region touches the
                // subject's alpha bounds (so nothing is printed over it).
                assert!(
                    sf.type_rect.intersect(&sf.subject).is_none(),
                    "{what}: type {:?} vs subject {:?}",
                    sf.type_rect,
                    sf.subject
                );
                // The label slot and the headline block are inside the region.
                let slot = sf.label;
                assert!(slot.intersect(&sf.subject).is_none(), "{what}: label");
                // The head is part of the subject: clear of the type too.
                let head = placement::to_canvas(&sf.image, sf.facts.head);
                assert!(
                    head.intersect(&sf.type_rect).is_none(),
                    "{what}: the head is covered"
                );
                // Inside the canvas (left/right) and under the kicker row.
                assert!(
                    sf.subject.x >= 0.0 && sf.subject.right() <= w + 0.5,
                    "{what}"
                );
                assert!(sf.subject.y >= 0.1 * h, "{what}: under the kicker");
                // The picture itself stays on the canvas floor.
                assert!(sf.image.bottom() <= h + 0.011 * h + 0.5, "{what}");
            }
        }
    }

    #[test]
    fn a_standing_person_fits_some_layout_on_every_canvas() {
        for canvas in CANVASES {
            let fitting: Vec<ImageLayout> = ALL_LAYOUTS
                .into_iter()
                .filter(|&layout| {
                    on_canvas_with(
                        SHORT_INTENT,
                        canvas,
                        standing(),
                        Variant::Standard,
                        |ctx, b, c| solve(ctx, b, c, layout),
                    )
                    .is_some_and(|sf| sf.fits)
                })
                .collect();
            assert!(!fitting.is_empty(), "{canvas:?}: no layout fits");
        }
    }

    #[test]
    fn a_fitting_layout_gives_the_subject_its_minimum_area_and_more() {
        for canvas in CANVASES {
            for layout in ALL_LAYOUTS {
                let sf = on_canvas(canvas, standing(), Variant::Standard, |ctx, b, c| {
                    solve(ctx, b, c, layout)
                });
                let Some(sf) = sf.filter(|s| s.fits) else {
                    continue;
                };
                let frame = crate::compiler::layout_frame::LayoutFrame::new(canvas.0, canvas.1)
                    .expect("frame");
                let min = subject_rules::subject_min_area(&frame);
                let visible = sf
                    .subject
                    .intersect(&Rect::new(0.0, 0.0, frame.w, frame.h))
                    .map_or(0.0, |r| r.area());
                assert!(
                    visible / (frame.w * frame.h) >= min * AREA_HEADROOM - 1e-4,
                    "{layout:?} on {canvas:?}: {:.3} < {min}",
                    visible / (frame.w * frame.h)
                );
                assert!(sf.reach >= AREA_HEADROOM, "{layout:?} {canvas:?}");
            }
        }
    }

    #[test]
    fn the_solver_is_deterministic() {
        for layout in ALL_LAYOUTS {
            let a = on_canvas((1080, 1920), standing(), Variant::Standard, |ctx, b, c| {
                solve(ctx, b, c, layout)
                    .map(|s| (s.image, s.subject, s.type_rect, s.label, s.reach))
            });
            let b = on_canvas((1080, 1920), standing(), Variant::Standard, |ctx, b, c| {
                solve(ctx, b, c, layout)
                    .map(|s| (s.image, s.subject, s.type_rect, s.label, s.reach))
            });
            assert_eq!(a, b, "{layout:?}");
        }
    }

    #[test]
    fn side_layouts_stand_the_subject_on_their_side_and_mirror_the_title() {
        let canvas = (1080, 1920);
        let right = on_canvas(canvas, standing(), Variant::Standard, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::SubjectRightTextLeft)
        })
        .expect("right");
        let left = on_canvas(canvas, standing(), Variant::Standard, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::SubjectLeftTextRight)
        })
        .expect("left");
        assert_eq!(right.align, TextAlign::Left);
        assert_eq!(left.align, TextAlign::Right);
        // Subject bottom-right / bottom-left, flush to the live area.
        assert!(right.subject.cx() > 540.0 && left.subject.cx() < 540.0);
        // The two are mirror images of each other.
        assert!((right.subject.cx() + left.subject.cx() - 1080.0).abs() < 1.0);
        assert!((right.subject.w - left.subject.w).abs() < 0.5);
        // The title block sits on the opposite side.
        assert!(right.type_rect.right() < right.subject.right());
        assert!(left.type_rect.x > left.subject.x);
    }

    #[test]
    fn the_band_layout_puts_the_title_below_the_subject() {
        let sf = on_canvas((1080, 1920), standing(), Variant::Standard, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::SubjectCenterTextBand)
        })
        .expect("center band");
        assert_eq!(sf.align, TextAlign::Center);
        assert!(
            sf.type_rect.y > sf.subject.bottom(),
            "title below the subject"
        );
        // Centred.
        assert!((sf.subject.cx() - 540.0).abs() < 1.0);
        // The band ends above the bottom margin.
        assert!(sf.type_rect.bottom() <= 1920.0 - 84.0 + 0.5);
    }

    #[test]
    fn text_top_stacks_the_subject_under_the_headline() {
        let sf = on_canvas((1080, 1920), standing(), Variant::Standard, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::TextTopSubjectBelow)
        })
        .expect("text top");
        assert!(
            sf.subject.y >= sf.type_rect.bottom(),
            "subject below the type"
        );
        assert!((sf.subject.cx() - 540.0).abs() < 1.0);
        // The mirror variant right-aligns the title block.
        let mirrored = on_canvas((1080, 1920), standing(), Variant::Mirror, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::TextTopSubjectBelow)
        })
        .expect("mirrored");
        assert_eq!(mirrored.align, TextAlign::Right);
    }

    #[test]
    fn a_title_that_cannot_be_set_readably_rejects_the_layout() {
        // A narrow title column cannot take a long headline at 46u: the solver
        // widens the column (side layouts) rather than set it tiny.
        let sf = on_canvas((1080, 1080), standing(), Variant::Standard, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::SubjectRightTextLeft)
        })
        .expect("right");
        assert!(sf.block.size >= MIN_TITLE_U - 0.01, "{}", sf.block.size);
    }

    #[test]
    fn rotation_starts_at_the_seed_and_advances_one_layout_per_image_beat() {
        let tall = crate::compiler::layout_frame::LayoutFrame::new(1080, 1920).unwrap();
        for seed in [0u64, 1, 2, 3, 4, 5, 1_000_003] {
            for nth in 0..6usize {
                let order = rotation_order(&tall, seed, nth);
                assert_eq!(order.len(), 4);
                let begin = (seed % 4) as usize + nth;
                assert_eq!(order[0], IMAGE_LAYOUT_ROTATION[begin % 4]);
                // Every layout appears once.
                for l in IMAGE_LAYOUT_ROTATION {
                    assert_eq!(order.iter().filter(|&&o| o == l).count(), 1);
                }
                // Consecutive image beats start on different layouts.
                let next = rotation_order(&tall, seed, nth + 1);
                assert_ne!(order[0], next[0], "seed {seed} beat {nth}");
            }
        }
    }

    #[test]
    fn wide_canvases_prefer_left_and_right() {
        let wide = crate::compiler::layout_frame::LayoutFrame::new(1920, 1080).unwrap();
        let is_side = |l: &ImageLayout| {
            matches!(
                l,
                ImageLayout::SubjectRightTextLeft | ImageLayout::SubjectLeftTextRight
            )
        };
        for seed in 0..8u64 {
            for nth in 0..4usize {
                let order = rotation_order(&wide, seed, nth);
                assert_eq!(order.len(), 4);
                assert!(is_side(&order[0]) && is_side(&order[1]), "{order:?}");
                assert!(!is_side(&order[2]) && !is_side(&order[3]), "{order:?}");
            }
        }
        // 4:3 and 6:5 are side-by-side canvases too.
        let fold = crate::compiler::layout_frame::LayoutFrame::new(1296, 1080).unwrap();
        assert!(is_side(&rotation_order(&fold, 0, 0)[0]));
        // 4:5 and 1:1 keep the full rotation.
        let square = crate::compiler::layout_frame::LayoutFrame::new(1080, 1080).unwrap();
        assert_eq!(
            rotation_order(&square, 0, 0)[0],
            ImageLayout::TextTopSubjectBelow
        );
    }

    #[test]
    fn a_subject_without_room_keeps_the_biggest_attempt_and_says_it_does_not_fit() {
        // A tiny subject region: an extremely wide image on a tall canvas.
        let flat = person(
            1200,
            200,
            NormBox {
                x: 0.02,
                y: 0.1,
                width: 0.96,
                height: 0.8,
            },
        );
        let sf = on_canvas((1080, 1920), flat, Variant::Standard, |ctx, b, c| {
            solve(ctx, b, c, ImageLayout::SubjectRightTextLeft)
        })
        .expect("an attempt");
        assert!(!sf.fits && sf.reach < AREA_HEADROOM, "reach {}", sf.reach);
    }
}
