//! (0.7.1) Entity depiction: phrase subjects as procedural entity tokens on a
//! process track; the beat's relationship plays as a visual operation.
//! Spec: docs/VISUAL_LANGUAGE.md §5.
//!
//! What the viewer sees instead of a text card:
//! - one **token** per phrase subject (shape family from a hash of the
//!   normalized phrase, so the same phrase is the same shape in every beat),
//! - a **process track** (two rails + rungs, identical geometry in every
//!   Entities beat) with an accent progress line,
//! - the beat's relationship as an **operation** on the track, scheduled on the
//!   lifecycle (primary in ENTER, secondary / operation at EVOLVE events,
//!   nothing new at or after ANTICIPATE).
//!
//! Built from existing primitives only: rounded rectangles, polylines + trim,
//! text, groups and the existing motion ops. No topic logic: shape and layout
//! depend on the beat's structure (grammar, relationship, energy, purpose) and
//! a hash of the phrase, never on its meaning.
//!
//! (0.23 W8d) Every relationship between two tokens draws its connector, in
//! the track's own stroke language, after the second token has landed and
//! before ANTICIPATE (the ids are the ones `story_warnings::has_connector`
//! recognises):
//!
//! | op (relationship) | connector | ids |
//! |---|---|---|
//! | grow | an arrow along the track from the primary to where the secondary ends up | `arrow`, `arrow_head` |
//! | replace | an arrow from the lifted primary down to the secondary (elbow when stacked, diagonal when side by side) | `arrow`, `arrow_head` |
//! | compress | an arrow from the secondary's zone onto the primary | `arrow`, `arrow_head` |
//! | accumulate | a line between the tokens with a plus badge on it | `connector`, `plus` |
//! | separate | the divide plus a VS badge between the parted tokens | `divide`, `vs` |
//! | carry | a link stem between the carrier and its rider (it travels with them) | `link` |
//! | none | the arrow-headed line (unchanged since 0.7.1) | `connector` |
//!
//! Connectors sit below the tokens (`CONNECTOR_Z`) and end in the gap between
//! the token bodies, so they never cover a token's text.

use super::placement::{self, Rect, SubjectFacts};
use super::plate::{self, events};
use super::{Composition, Grammar, Variant};
use crate::assets::AssetRole;
use crate::compiler::explore::contrast_ratio;
use crate::compiler::recipes::{self, plane, B};
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mo, rect_layer, round3, Carry, CompileError, Ctx};
use crate::easing::Easing;
use crate::intent::{Energy, Purpose, Relationship};
use crate::reference::evidence::Fnv64;
use crate::scene::{Color, Direction, Layer, LayerKind, Motion, MotionOp, Stroke, TextAlign};

/// Label size relative to the nominal `label_size` of a token.
const LABEL_K: f32 = 1.4;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Shape family of an entity token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Family {
    Disc,
    Capsule,
    Hexagon,
    Diamond,
    RoundedSquare,
}

/// Normalized phrase identity: lowercase alphanumeric words joined by one space.
pub(super) fn entity_key(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Shape family of a key: FNV-1a(key) % 5.
pub(super) fn family_of(key: &str) -> Family {
    let mut h = Fnv64::default();
    h.write(key.as_bytes());
    match h.finish() % 5 {
        0 => Family::Disc,
        1 => Family::Capsule,
        2 => Family::Hexagon,
        3 => Family::Diamond,
        _ => Family::RoundedSquare,
    }
}

// ---------------------------------------------------------------------------
// Process track (identical in every Entities beat)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct Track {
    x0: f32,
    len: f32,
    /// Center line between the two rails.
    y: f32,
    gap: f32,
}

fn track_of(ctx: &Ctx) -> Track {
    let m = ctx.margin();
    let frac = 0.6;
    Track {
        x0: m,
        len: ctx.w - 2.0 * m,
        y: frac * ctx.h,
        gap: 24.0 * ctx.u,
    }
}

fn trim(target: &str, start: f64, dur: f64, from: f32, to: f32, easing: Easing) -> Motion {
    Motion {
        spring: None,
        id: None,
        target: target.to_string(),
        start: round3(start),
        duration: round3(dur),
        easing,
        op: MotionOp::Trim { from, to },
    }
}

/// Duration that lets a motion starting at `start` end before `limit` (and
/// inside the scene).
fn fit(b: &B, start: f64, dur: f64, limit: f64) -> f64 {
    let cap = (limit - start - 0.02).min(b.plan.duration - start - 0.02);
    dur.min(cap).max(0.1)
}

fn build_track(ctx: &Ctx, b: &mut B, t0: f64) {
    let (u, tr) = (ctx.u, track_of(ctx));
    let (plan, life) = (b.plan, b.plan.life);
    let mid = plane(plan.lang.depth.midground);
    let ink = ctx.palette.ink;
    let draw = (plan.lang.preset.duration * 1.6)
        .min(life.read - t0 - 0.05)
        .max(0.5);
    let ease = plan.lang.preset.easing.landing();
    let ya = tr.y - tr.gap / 2.0;
    let yb = tr.y + tr.gap / 2.0;

    for (name, y, delay) in [("track_a", ya, 0.0), ("track_b", yb, 0.08)] {
        let id = b.id(name);
        let mut rail = base_layer(
            id.clone(),
            (tr.x0, y, tr.len, 5.0 * u),
            LayerKind::Polyline {
                points: vec![[0.0, 0.0], [tr.len, 0.0]],
                stroke: Stroke {
                    color: ink.with_alpha(0x8C),
                    width: 5.0 * u,
                },
                closed: false,
                fill: None,
            },
            8,
        );
        rail.depth = mid;
        b.motions.push(trim(&id, t0 + delay, draw, 0.0, 1.0, ease));
        b.push(rail);
    }

    // Rungs: short perpendicular ticks between the rails, one group.
    let step = 44.0 * u;
    let tick_w = 3.0 * u;
    let over = 6.0 * u;
    let count = (((tr.len - tick_w) / step).floor() as usize) + 1;
    let rungs_id = b.id("rungs");
    let children: Vec<Layer> = (0..count)
        .map(|i| {
            rect_layer(
                format!("{rungs_id}.{i}"),
                (i as f32 * step, 0.0, tick_w, tr.gap + 2.0 * over),
                ink.with_alpha(0x66),
                0,
            )
        })
        .collect();
    let mut rungs = base_layer(
        rungs_id.clone(),
        (tr.x0, ya - over, tr.len, tr.gap + 2.0 * over),
        LayerKind::Group { children },
        8,
    );
    rungs.depth = mid;
    b.motions.push(mo::mask(
        &rungs_id,
        t0 + 0.1,
        draw,
        Direction::Right,
        Easing::OutCubic,
    ));
    b.push(rungs);

    // Progress: an accent line over the upper rail. Its end moves from this
    // beat's share of the track to the next one's during READ -> EVOLVE.
    let n = ctx.beat_count.max(1) as f64;
    let start = (plan.index as f64 / n).min(1.0);
    let end = if plan.is_last {
        1.0
    } else {
        ((plan.index + 1) as f64 / n).min(1.0)
    };
    let pid = b.id("progress");
    let mut progress = base_layer(
        pid.clone(),
        (tr.x0, ya, tr.len, 7.0 * u),
        LayerKind::Polyline {
            points: vec![[0.0, 0.0], [tr.len, 0.0]],
            stroke: Stroke {
                color: ctx.palette.accent,
                width: 7.0 * u,
            },
            closed: false,
            fill: None,
        },
        9,
    );
    progress.depth = mid;
    let (s, e) = (start as f32, end as f32);
    if s > 0.0 {
        let dur = (draw * 0.8).min(life.read - t0 - 0.05).max(0.3);
        b.motions.push(trim(&pid, t0, dur, 0.0, s, ease));
    }
    let span = life.anticipate - life.read;
    if span > 0.3 {
        b.motions.push(trim(
            &pid,
            life.read,
            fit(b, life.read, span, life.anticipate + 0.02),
            s,
            e,
            Easing::InOutCubic,
        ));
    }
    b.push(progress);
}

/// Process stage indicator below the track: one mark per beat, identical layout
/// in every Entities beat. Past stages are ink, the current one is accent and
/// grows in during ENTER, future ones are outlined. Says where the process is,
/// nothing topic-specific.
fn build_stage(ctx: &Ctx, b: &mut B, t0: f64) {
    let u = ctx.u;
    let plan = b.plan;
    let n = ctx.beat_count.max(1);
    let mid = plane(plan.lang.depth.midground);
    let ink = ctx.palette.ink;
    let y = ctx.frame.blend(0.84, 0.945, 0.945) * ctx.h;
    let pitch = (84.0 * u).min((ctx.w - 2.0 * ctx.margin()) / n as f32);
    let x_of = |i: usize| ctx.w / 2.0 + (i as f32 - (n - 1) as f32 / 2.0) * pitch;

    if n > 1 {
        let id = b.id("stage_line");
        let (x0, x1) = (x_of(0), x_of(n - 1));
        let mut line = rect_layer(
            id.clone(),
            (x0, y - 1.0 * u, x1 - x0, 2.0 * u),
            ink.with_alpha(0x40),
            8,
        );
        line.depth = mid;
        b.motions
            .push(mo::fade(&id, t0 + 0.1, 0.5, 0.0, 1.0, Easing::OutCubic));
        b.push(line);
    }
    for i in 0..n {
        let current = i == plan.index;
        let past = i < plan.index;
        let d = if current { 42.0 * u } else { 26.0 * u };
        let (fill, stroke) = if current {
            (ctx.palette.accent, ink)
        } else if past {
            (ink, ink)
        } else {
            (ink.with_alpha(0), ink.with_alpha(0x80))
        };
        let id = b.id(&format!("stage_{i}"));
        let mut mark = base_layer(
            id.clone(),
            (x_of(i), y, d, d),
            LayerKind::RoundedRectangle {
                fill,
                radius: d / 2.0,
                stroke: Some(Stroke {
                    color: stroke,
                    width: 2.5 * u,
                }),
            },
            9,
        );
        mark.anchor_x = 0.5;
        mark.anchor_y = 0.5;
        mark.depth = mid;
        b.motions
            .push(mo::fade(&id, t0 + 0.1, 0.5, 0.0, 1.0, Easing::OutCubic));
        if current {
            b.motions.push(mo::scale(
                &id,
                t0 + 0.3,
                (plan.lang.preset.duration * 1.1).max(0.5),
                0.2,
                1.0,
                plan.lang.settle,
            ));
        }
        b.push(mark);
    }
}

// ---------------------------------------------------------------------------
// Entity token
// ---------------------------------------------------------------------------

struct TokenParams<'a> {
    /// `primary` or `secondary` (layer ids `entity_<name>`, `label_<name>`).
    name: &'a str,
    text: &'a str,
    primary: bool,
    /// Nominal body size (the long dimension of a disc / square).
    size: f32,
    cx: f32,
    cy: f32,
    z: i32,
    /// Label above the body instead of below.
    label_above: bool,
    /// The body is a delivered image: no procedural body or mark is drawn
    /// and the box is `size` x `size`.
    image: bool,
    label_size: f32,
}

struct Token {
    layer: Layer,
    id: String,
    /// Body center in canvas space (after clamping into the frame).
    cx: f32,
    /// Group box (body + label).
    gw: f32,
    bw: f32,
    bh: f32,
    /// Body box origin inside the group (for a delivered image).
    bx: f32,
    by: f32,
}

fn clamp_cx(ctx: &Ctx, cx: f32, gw: f32) -> f32 {
    let half = ctx.margin() * 0.5 + gw / 2.0;
    cx.max(half).min(ctx.w - half)
}

/// Body (width, height) for a family at a nominal size.
fn body_size(fam: Family, s: f32) -> (f32, f32) {
    match fam {
        Family::Disc | Family::Diamond => (s, s),
        Family::Capsule => (1.7 * 0.62 * s, 0.62 * s),
        Family::Hexagon => (s, 0.88 * s),
        Family::RoundedSquare => (0.9 * s, 0.9 * s),
    }
}

fn token(ctx: &Ctx, b: &B, p: &TokenParams) -> Token {
    let u = ctx.u;
    let key = entity_key(p.text);
    let fam = family_of(&key);
    let (bw, bh) = if p.image {
        (p.size, p.size)
    } else {
        body_size(fam, p.size)
    };
    let ink = ctx.palette.ink;
    let fill = if p.primary {
        ctx.palette.accent
    } else {
        ctx.palette.card
    };
    let outline = Stroke {
        color: ink,
        width: 3.5 * u,
    };
    let name = p.name;

    // Label (inside the group so every motion moves token + label as one).
    let short = ctx.w.min(ctx.h);
    let label_w = (bw * 1.3).max(0.32 * short).min(0.46 * short);
    let label = (!p.text.trim().is_empty()).then(|| {
        ctx.ts.fit_block(
            Voice::LABEL,
            p.text,
            label_w,
            p.label_size * LABEL_K * 2.6,
            p.label_size * LABEL_K,
            2,
        )
    });
    let (lw, lh) = label
        .as_ref()
        .map_or((0.0, 0.0), |l| (l.width() * 1.02 + 2.0, l.height()));
    let gap_l = if label.is_some() { 16.0 * u } else { 0.0 };
    let gw = bw.max(lw);
    let gh = bh + gap_l + lh;
    let (by, ly) = if p.label_above {
        (lh + gap_l, 0.0)
    } else {
        (0.0, bh + gap_l)
    };
    let bx = (gw - bw) / 2.0;
    let (mx, my) = (bx + bw / 2.0, by + bh / 2.0);

    let mut children: Vec<Layer> = Vec::new();
    let body_id = format!("{}.body", b.id(&format!("entity_{name}")));
    match fam {
        _ if p.image => {}
        Family::Disc | Family::Capsule | Family::RoundedSquare => {
            let radius = match fam {
                Family::Disc => bw / 2.0,
                Family::Capsule => bh / 2.0,
                _ => 0.2 * bw,
            };
            children.push(base_layer(
                body_id,
                (bx, by, bw, bh),
                LayerKind::RoundedRectangle {
                    fill,
                    radius,
                    stroke: Some(outline),
                },
                1,
            ));
        }
        Family::Hexagon => {
            let pts = vec![
                [0.25 * bw, 0.0],
                [0.75 * bw, 0.0],
                [bw, 0.5 * bh],
                [0.75 * bw, bh],
                [0.25 * bw, bh],
                [0.0, 0.5 * bh],
            ];
            children.push(base_layer(
                body_id,
                (bx, by, bw, bh),
                LayerKind::Polyline {
                    points: pts,
                    stroke: outline,
                    closed: true,
                    fill: Some(fill),
                },
                1,
            ));
        }
        Family::Diamond => {
            let side = 0.7 * p.size;
            let mut l = base_layer(
                body_id,
                (mx, my, side, side),
                LayerKind::RoundedRectangle {
                    fill,
                    radius: 0.14 * side,
                    stroke: Some(outline),
                },
                1,
            );
            l.anchor_x = 0.5;
            l.anchor_y = 0.5;
            l.rotation_degrees = 45.0;
            children.push(l);
        }
    }
    // Inner mark: secondary = accent dot on a card body; primary = a small
    // card dot on the accent body.
    let md = 0.28 * bw.min(bh);
    let mut mark = base_layer(
        format!("{}.mark", b.id(&format!("entity_{name}"))),
        (mx, my, md, md),
        LayerKind::RoundedRectangle {
            fill: if p.primary {
                ctx.palette.card
            } else {
                ctx.palette.accent
            },
            radius: md / 2.0,
            stroke: p.primary.then_some(Stroke {
                color: ink,
                width: 2.5 * u,
            }),
        },
        2,
    );
    mark.anchor_x = 0.5;
    mark.anchor_y = 0.5;
    if !p.image {
        children.push(mark);
    }

    if let Some(l) = &label {
        let mut layer = text_layer(b.id(&format!("label_{name}")), l, ink, TextAlign::Center);
        layer.x = (gw - layer.width) / 2.0;
        layer.y = ly;
        layer.z_index = 3;
        children.push(layer);
    }

    let cx = clamp_cx(ctx, p.cx, gw);
    let id = b.id(&format!("entity_{name}"));
    let mut group = base_layer(
        id.clone(),
        (cx, p.cy, gw, gh),
        LayerKind::Group { children },
        p.z,
    );
    group.anchor_x = 0.5;
    group.anchor_y = my / gh;
    group.depth = plane(b.plan.lang.depth.foreground);
    Token {
        layer: group,
        id,
        cx,
        gw,
        bw,
        bh,
        bx,
        by,
    }
}

impl Token {
    fn set_cx(&mut self, ctx: &Ctx, cx: f32) {
        self.cx = clamp_cx(ctx, cx, self.gw);
        self.layer.x = self.cx;
    }
    fn set_scale(&mut self, s: f32) {
        self.layer.scale_x = s;
        self.layer.scale_y = s;
    }
    /// Pull in so the label stays inside the safe area at the largest scale
    /// the token reaches (a no-op when it already does).
    fn keep_inside(&mut self, ctx: &Ctx, scale: f32) {
        let (m, half) = (ctx.margin(), scale * self.gw / 2.0);
        if 2.0 * (m + half) <= ctx.w {
            self.set_cx(ctx, self.cx.max(m + half).min(ctx.w - m - half));
        }
    }
    /// Move up to `extra` along `dir` (+1 / -1), so a connector between two
    /// tokens has room: never past the safe area the label must stay inside
    /// (`scale` is the largest scale the token reaches) and never back toward
    /// where the token stood.
    fn nudge(&mut self, ctx: &Ctx, dir: f32, extra: f32, scale: f32) {
        let (m, half) = (ctx.margin(), scale * self.gw / 2.0);
        let want = self.cx + dir * extra;
        let to = if dir > 0.0 {
            want.min(ctx.w - m - half).max(self.cx)
        } else {
            want.max(m + half).min(self.cx)
        };
        self.set_cx(ctx, to);
    }
}

// ---------------------------------------------------------------------------
// Connectors (0.23 W8d)
// ---------------------------------------------------------------------------

/// Layer order of a connector: over the track (8-9) and the zone (10), under
/// every token (22 and up), so a connector never covers a token's text.
const CONNECTOR_Z: i32 = 12;
/// Layer order of a badge (plus, VS): above the connector line it sits on.
const BADGE_Z: i32 = CONNECTOR_Z + 1;
/// Smallest contrast an accent connector needs against the paper; under it
/// the ink draws the connector.
const CONNECTOR_MIN_CONTRAST: f32 = 3.0;
/// Shaft width, head length and the gap kept to a token body (all in `u`).
const ARROW_WIDTH: f32 = 11.0;
const ARROW_HEAD: f32 = 38.0;
const CONNECTOR_PAD: f32 = 10.0;

/// The colour connectors are drawn in: the palette accent when it reads
/// against the paper, else the ink.
fn connector_color(ctx: &Ctx) -> Color {
    let p = &ctx.palette;
    if contrast_ratio(p.accent, p.paper) >= CONNECTOR_MIN_CONTRAST {
        p.accent
    } else {
        p.ink
    }
}

/// A polyline through canvas points, as a layer at the points' bounding box.
fn poly_layer(
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
        CONNECTOR_Z,
    );
    l.depth = depth;
    l
}

/// Ends `(from, to)` of a connector that runs in direction `dir` (+1 / -1)
/// along x: the gap between two bodies when it is at least `min_len`, else a
/// `min_len` span centred on it (the connector sits under the tokens, so it is
/// still drawn when a very wide label squeezes the gap).
fn span(from: f32, to: f32, dir: f32, min_len: f32) -> (f32, f32) {
    if dir * (to - from) >= min_len {
        return (from, to);
    }
    let mid = (from + to) / 2.0;
    (mid - dir * min_len / 2.0, mid + dir * min_len / 2.0)
}

/// An arrow along `pts` (tail first, two points or more). The shaft draws
/// itself (trim) over `[start, start + dur]`; the head, a filled triangle,
/// fades in as the trim reaches it. Layer ids `arrow` and `arrow_head`.
fn draw_arrow(ctx: &Ctx, b: &mut B, pts: &[(f32, f32)], start: f64, dur: f64, ease: Easing) {
    let [.., prev, tip] = pts else {
        return;
    };
    let (dx, dy) = (tip.0 - prev.0, tip.1 - prev.1);
    let last = (dx * dx + dy * dy).sqrt();
    if last < 1.0 {
        return;
    }
    let (ux, uy) = (dx / last, dy / last);
    let total: f32 = pts
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum();
    let u = ctx.u;
    let color = connector_color(ctx);
    let mid = plane(b.plan.lang.depth.midground);
    let head = (ARROW_HEAD * u).min(0.9 * last).min(0.5 * total);
    let width = (ARROW_WIDTH * u).min(0.4 * head);
    let base = (tip.0 - ux * head, tip.1 - uy * head);

    let mut shaft: Vec<(f32, f32)> = pts[..pts.len() - 1].to_vec();
    shaft.push(base);
    let id = b.id("arrow");
    b.motions.push(trim(&id, start, dur, 0.0, 1.0, ease));
    b.push(poly_layer(
        id,
        &shaft,
        Stroke { color, width },
        false,
        None,
        mid,
    ));

    // Head: a filled triangle on the end, in as the shaft arrives.
    let (nx, ny) = (-uy, ux);
    let half = 0.55 * head;
    let tri = [
        *tip,
        (base.0 + nx * half, base.1 + ny * half),
        (base.0 - nx * half, base.1 - ny * half),
    ];
    let head_id = b.id("arrow_head");
    b.motions.push(mo::fade(
        &head_id,
        start + 0.75 * dur,
        0.25 * dur,
        0.0,
        1.0,
        Easing::OutCubic,
    ));
    b.push(poly_layer(
        head_id,
        &tri,
        Stroke { color, width: 1.0 },
        true,
        Some(color),
        mid,
    ));
}

/// What a badge shows.
enum Mark {
    /// A plus drawn from two bars (no text layer).
    Plus,
    /// A short word in the headline voice.
    Word(&'static str),
}

/// A round badge centred at `centre`, `d` across: a card disc with a
/// connector-coloured ring and its mark in the ink, one group (layer id
/// `name`) that pops in (fade + scale) over `[start, start + dur]`.
#[allow(clippy::too_many_arguments)]
fn badge(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    centre: (f32, f32),
    d: f32,
    mark: Mark,
    start: f64,
    dur: f64,
) {
    let id = b.id(name);
    let ink = ctx.palette.ink;
    let mut children = vec![base_layer(
        format!("{id}.disc"),
        (0.0, 0.0, d, d),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: d / 2.0,
            stroke: Some(Stroke {
                color: connector_color(ctx),
                width: 0.07 * d,
            }),
        },
        0,
    )];
    match mark {
        Mark::Plus => {
            let (len, thick) = (0.46 * d, 0.1 * d);
            let o = (d - len) / 2.0;
            let c = (d - thick) / 2.0;
            children.push(rect_layer(
                format!("{id}.bar_h"),
                (o, c, len, thick),
                ink,
                1,
            ));
            children.push(rect_layer(
                format!("{id}.bar_v"),
                (c, o, thick, len),
                ink,
                1,
            ));
        }
        Mark::Word(word) => {
            let block = ctx.ts.fit_line(Voice::HEADLINE, word, 0.72 * d, 0.42 * d);
            let mut t = text_layer(format!("{id}.text"), &block, ink, TextAlign::Center);
            t.x = (d - t.width) / 2.0;
            t.y = (d - t.height) / 2.0;
            t.z_index = 1;
            children.push(t);
        }
    }
    let mut group = base_layer(
        id.clone(),
        (centre.0, centre.1, d, d),
        LayerKind::Group { children },
        BADGE_Z,
    );
    group.anchor_x = 0.5;
    group.anchor_y = 0.5;
    group.depth = plane(b.plan.lang.depth.midground);
    b.motions
        .push(mo::fade(&id, start, dur * 0.6, 0.0, 1.0, Easing::OutCubic));
    b.motions
        .push(mo::scale(&id, start, dur, 0.4, 1.0, b.plan.lang.settle));
    b.push(group);
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Solo,
    HeroPair,
    Replace,
    Compress,
    Grow,
    Separate,
    Carry,
    Connect,
}

/// Entrance of a token in ENTER: fade (or mask reveal) + settle scale.
fn enter(b: &mut B, id: &str, start: f64, dur: f64, reveal: bool) {
    let lang = b.plan.lang;
    if reveal {
        b.motions.push(mo::mask(
            id,
            start,
            dur * 1.1,
            Direction::Right,
            Easing::OutCubic,
        ));
    } else {
        b.motions
            .push(mo::fade(id, start, dur * 0.8, 0.0, 1.0, Easing::OutCubic));
    }
    b.motions
        .push(mo::scale(id, start, dur, 0.85, 1.0, lang.settle));
}

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    _carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let short = w.min(h);
    let f = &ctx.frame;
    let (plan, beat) = (b.plan, b.beat);
    let life = plan.life;
    let d = plan.lang.preset.duration;
    let t = plan.enter_at();
    let mirror = c.variant == Variant::Mirror;
    let sign: f32 = if mirror { -1.0 } else { 1.0 };
    let tr = track_of(ctx);
    let reveal = beat.purpose == Purpose::Reveal;
    // Landscape and square have less height around the track: scale tokens.
    let kf = f.blend(1.0, 0.85, 0.85);
    let sz = |f: f32| f * short * kf;
    let mk_sz = sz(0.22);
    let lift = f.blend(0.22, 0.17, 0.17) * h;
    // Smooth tall -> square/wide weight of the vertical-only type sizes.
    let tallness = |tall: f32, other: f32| f.blend(tall, other, other);

    recipes::ghost(ctx, b, 0.5 * h);

    // Smaller headline: supporting type, not the hero.
    let align = if mirror {
        TextAlign::Right
    } else {
        TextAlign::Left
    };
    let head_top = f.blend(0.14, 0.14, 0.17) * h;
    let head_h_max = f.blend(0.12, 0.12, 0.13) * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        w - 2.0 * m,
        head_h_max,
        72.0 * u,
        3,
    );
    recipes::headline_lines(
        ctx,
        b,
        "head",
        &head,
        head_top,
        align,
        ctx.palette.ink,
        t + 0.05,
        true,
    );

    build_track(ctx, b, t);
    build_stage(ctx, b, t);

    // Which operation this beat plays.
    let p_text = beat.primary.display_text().unwrap_or("");
    let s_text = beat
        .secondary
        .as_ref()
        .and_then(|s| s.display_text())
        .filter(|s| !s.trim().is_empty());
    let op = match (c.grammar, s_text) {
        (_, None) => Op::Solo,
        (Grammar::SpatialCauseEffect, Some(_)) => match beat.relationship {
            Some(Relationship::Replace) => Op::Replace,
            Some(Relationship::Compress) => Op::Compress,
            Some(Relationship::Grow) => Op::Grow,
            Some(Relationship::Separate) => Op::Separate,
            Some(Relationship::Carry) => Op::Carry,
            _ => Op::Connect,
        },
        (_, Some(_)) => Op::HeroPair,
    };

    let hero_sz = if op == Op::HeroPair {
        sz(0.36)
    } else {
        sz(0.40)
    };

    // EVOLVE events (kept clear of ANTICIPATE and the scene end).
    let n_events = match op {
        Op::Solo => 0,
        Op::HeroPair | Op::Replace => 2,
        _ => 1,
    };
    let mut ev = events(b, n_events, d * 1.6 + 0.3);
    if n_events == 2 && ev[1] - ev[0] < 0.55 {
        ev[0] = (ev[1] - 0.55).max(life.settle).min(ev[1] - 0.15);
    }
    let ev0 = ev.first().copied().unwrap_or(life.anticipate);
    let ant = life.anticipate;

    // Token entrances finish before the first event.
    let t_in = t + 0.25;
    let ed = (d * 1.1).min(ev0 - t_in - 0.05).max(0.3);
    let t_in2 = t_in + d * 0.45;
    let ed2 = (d * 1.1).min(ev0 - t_in2 - 0.05).max(0.3);

    let ty = tr.y;
    let ease_move = plan.lang.preset.easing;
    let settle = plan.lang.settle;
    // Connectors: the gap kept to a token body and the shortest arrow drawn.
    let pad = CONNECTOR_PAD * u;
    let min_len = 0.06 * short;

    match op {
        // ---------------------------------------------------------------
        // HeroObject: a large token; with a secondary, a marker ahead on the
        // track that the hero approaches.
        // ---------------------------------------------------------------
        Op::Solo | Op::HeroPair if c.grammar != Grammar::SpatialCauseEffect => {
            let (pf, mf) = if op == Op::HeroPair {
                (0.28, 0.80)
            } else {
                (0.32, 0.74)
            };
            let pcx = if mirror { (1.0 - pf) * w } else { pf * w };
            let entry = plate::image(ctx, plan.index, AssetRole::HeroObject);
            let mut prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size: hero_sz,
                    cx: pcx,
                    cy: ty,
                    z: 23,
                    label_above: false,
                    image: entry.is_some(),
                    label_size: 34.0 * u,
                },
            );
            if let Some(entry) = entry {
                // A delivered hero image replaces the procedural body.
                let facts = SubjectFacts::from_entry(entry);
                let target = Rect::new(prim.bx, prim.by, prim.bw, prim.bh);
                let img = placement::subject_fit(&facts, target);
                let hero = plate::image_plate(
                    ctx,
                    b,
                    "hero",
                    AssetRole::HeroObject,
                    (img.x, img.y, img.w, img.h),
                    1,
                    t_in,
                );
                if let LayerKind::Group { children } = &mut prim.layer.kind {
                    children.extend(hero.layers);
                }
            }
            enter(b, &prim.id, t_in, ed, reveal);
            if let Some(st) = s_text.filter(|_| op == Op::HeroPair) {
                let mcx = if mirror { (1.0 - mf) * w } else { mf * w };
                let mark = token(
                    ctx,
                    b,
                    &TokenParams {
                        name: "secondary",
                        text: st,
                        primary: false,
                        size: mk_sz,
                        cx: mcx,
                        cy: ty,
                        z: 21,
                        label_above: false,
                        image: false,
                        label_size: 26.0 * u,
                    },
                );
                let (e0, e1) = (ev[0], ev[1]);
                let fade_dur = fit(b, e0, d * 0.7, e1);
                b.motions
                    .push(mo::fade(&mark.id, e0, fade_dur, 0.0, 1.0, Easing::OutCubic));
                b.motions.push(mo::scale(
                    &mark.id,
                    e0,
                    fit(b, e0, d * 0.9, e1),
                    0.6,
                    1.0,
                    settle,
                ));
                let calm = beat.energy == Energy::Calm;
                let target = if calm {
                    mark.cx - sign * (mark.bw / 2.0 + 0.6 * mk_sz + prim.bw / 2.0)
                } else {
                    mark.cx + sign * 0.1 * w
                };
                let target = clamp_cx(ctx, target, prim.gw);
                let dx = target - prim.cx;
                if calm {
                    let mv = fit(b, e1, d * 1.1, ant - 0.1);
                    b.motions.push(mo::shift(
                        &prim.id,
                        e1,
                        mv,
                        [0.0, 0.0],
                        [dx, 0.0],
                        ease_move.landing(),
                    ));
                    // Small recoil: it came up short of the marker.
                    let rs = e1 + mv;
                    if rs < ant - 0.02 {
                        b.motions.push(mo::shift(
                            &prim.id,
                            rs,
                            fit(b, rs, d * 0.5, f64::MAX),
                            [dx, 0.0],
                            [dx - sign * 12.0 * u, 0.0],
                            settle,
                        ));
                    }
                } else {
                    let mv = fit(b, e1, d * 1.0, ant - 0.1);
                    b.motions.push(mo::shift(
                        &prim.id,
                        e1,
                        mv,
                        [0.0, 0.0],
                        [dx, 0.0],
                        ease_move,
                    ));
                    // The marker dims as it is passed over.
                    let ds = (e1 + mv * 0.4).max(e0 + fade_dur + 0.02);
                    if ds < ant - 0.02 {
                        let dd = fit(b, ds, mv * 0.5, f64::MAX);
                        b.motions
                            .push(mo::fade(&mark.id, ds, dd, 1.0, 0.35, Easing::InOutCubic));
                        // The label goes with it: no sliver left behind.
                        let label_id = b.id("label_secondary");
                        b.motions
                            .push(mo::fade(&label_id, ds, dd, 1.0, 0.0, Easing::InOutCubic));
                    }
                }
                b.push(mark.layer);
            }
            prim.layer.z_index = 23;
            b.push(prim.layer);
        }

        // ---------------------------------------------------------------
        // SpatialCauseEffect with one token only: centered on the track.
        // ---------------------------------------------------------------
        Op::Solo | Op::HeroPair => {
            let prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size: hero_sz,
                    cx: 0.5 * w,
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 34.0 * u,
                },
            );
            enter(b, &prim.id, t_in, ed, reveal);
            b.push(prim.layer);
        }

        // ---------------------------------------------------------------
        // replace: the primary lifts off the slot, the secondary drops in
        // and then advances along the track.
        // ---------------------------------------------------------------
        Op::Replace => {
            let size = sz(tallness(0.38, 0.30));
            let slot = 0.5 * w;
            let prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size,
                    cx: slot,
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            let sec = token(
                ctx,
                b,
                &TokenParams {
                    name: "secondary",
                    text: s_text.unwrap_or(""),
                    primary: false,
                    size,
                    cx: slot,
                    cy: ty,
                    z: 23,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            enter(b, &prim.id, t_in, ed, reveal);
            let (e0, e1) = (ev[0], ev[1]);
            let lift_dur = fit(b, e0, d, e1);
            b.motions.push(mo::shift(
                &prim.id,
                e0,
                lift_dur,
                [0.0, 0.0],
                [0.0, -lift],
                settle,
            ));
            b.motions
                .push(mo::scale(&prim.id, e0, lift_dur, 1.0, 0.8, settle));
            b.motions.push(mo::fade(
                &prim.id,
                e0,
                lift_dur,
                1.0,
                0.3,
                Easing::InOutCubic,
            ));
            let ds = e0 + 0.25_f64.min(0.4 * (e1 - e0));
            let drop = fit(b, ds, d, e1);
            b.motions.push(mo::shift(
                &sec.id,
                ds,
                drop,
                [0.0, 0.2 * h],
                [0.0, 0.0],
                ease_move.landing(),
            ));
            b.motions.push(mo::fade(
                &sec.id,
                ds,
                drop * 0.8,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
            b.motions
                .push(mo::scale(&sec.id, ds, drop, 0.9, 1.0, settle));
            // The relationship is an arrow from the lifted primary to the
            // secondary where it settles. Stacked (room to drop): out of the
            // primary's side, then down onto the secondary's top, clear of the
            // primary's label. Side by side: a diagonal onto the secondary's
            // near edge, so the secondary advances far enough to leave the
            // arrow room between the two bodies.
            let head_len = ARROW_HEAD * u;
            let p_cy = ty - lift;
            let sec_top = ty - sec.bh / 2.0;
            let stacked = sec_top - pad - p_cy >= 2.0 * head_len;
            let mut target = clamp_cx(ctx, slot + sign * 0.18 * w, sec.gw);
            if !stacked {
                // Advance further when the arrow needs room between the
                // bodies, as far as the label stays inside the safe area.
                let room = 0.8 * prim.bw / 2.0 + sec.bw / 2.0 + 2.4 * head_len;
                let far = clamp_cx(ctx, slot + sign * room, sec.gw);
                let safe = if sign > 0.0 {
                    far.min(w - m - sec.gw / 2.0)
                } else {
                    far.max(m + sec.gw / 2.0)
                };
                target = if sign > 0.0 {
                    target.max(safe)
                } else {
                    target.min(safe)
                };
            }
            b.motions.push(mo::shift(
                &sec.id,
                e1,
                fit(b, e1, d * 1.1, ant - 0.1),
                [0.0, 0.0],
                [target - sec.cx, 0.0],
                settle,
            ));
            let tail = (prim.cx + sign * (0.8 * prim.bw / 2.0 + pad), p_cy);
            let route = if stacked {
                let label_edge = prim.cx + sign * (0.4 * prim.gw + pad);
                let past = tail.0 + sign * 1.6 * head_len;
                let near = target + sign * 0.35 * sec.bw;
                let x_v = if sign > 0.0 {
                    target.max(label_edge).max(past).min(near)
                } else {
                    target.min(label_edge).min(past).max(near)
                };
                vec![tail, (x_v, p_cy), (x_v, sec_top - pad)]
            } else {
                let near_edge = target - sign * (sec.bw / 2.0 + pad);
                if sign * (near_edge - tail.0) >= 2.0 * head_len {
                    vec![tail, (near_edge, ty - 0.15 * sec.bh)]
                } else {
                    // A wide label keeps the secondary close to the primary:
                    // the arrow lands on the secondary's top edge instead.
                    let (x0, x1) = span(tail.0, target - sign * 0.2 * sec.bw, sign, min_len);
                    vec![
                        (x0, p_cy),
                        (
                            x1,
                            (sec_top - pad - 0.55 * head_len).max(p_cy + 0.5 * head_len),
                        ),
                    ]
                }
            };
            draw_arrow(
                ctx,
                b,
                &route,
                e1,
                fit(b, e1, d * 1.1, ant - 0.05),
                ease_move.landing(),
            );
            b.push(prim.layer);
            b.push(sec.layer);
        }

        // ---------------------------------------------------------------
        // compress: the secondary is a pressure zone that grows toward the
        // primary, which shrinks and yields.
        // ---------------------------------------------------------------
        Op::Compress => {
            let prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size: sz(0.34),
                    cx: if mirror { 0.67 * w } else { 0.33 * w },
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            let sec = token(
                ctx,
                b,
                &TokenParams {
                    name: "secondary",
                    text: s_text.unwrap_or(""),
                    primary: false,
                    size: sz(0.26),
                    cx: if mirror { 0.26 * w } else { 0.74 * w },
                    cy: ty,
                    z: 24,
                    label_above: false,
                    image: false,
                    label_size: 30.0 * u,
                },
            );
            enter(b, &prim.id, t_in, ed, reveal);
            enter(b, &sec.id, t_in2, ed2, false);

            // Zone behind the secondary: far edge fixed, near edge grows
            // toward the primary.
            let zh = sz(0.26) * 2.0;
            let zy = ty - zh / 2.0;
            let far = (sec.cx + sign * (sec.gw * 0.62))
                .max(m * 0.4)
                .min(w - m * 0.4);
            let near0 = far - sign * (sec.gw * 1.45);
            let prim_end = prim.cx - sign * 0.06 * w;
            let near1 = prim_end + sign * (prim.bw * 0.82 / 2.0 + 10.0 * u);
            let rect = |a: f32, b2: f32| (a.min(b2), zy, (a - b2).abs(), zh);
            let (zx, _, zw, _) = rect(far, near0);
            let zone_id = b.id("zone");
            let mut zone = base_layer(
                zone_id.clone(),
                (zx, zy, zw, zh),
                LayerKind::RoundedRectangle {
                    fill: ctx.palette.accent.with_alpha(0x33),
                    radius: 0.12 * zh,
                    stroke: None,
                },
                10,
            );
            zone.depth = plane(plan.lang.depth.midground);
            b.motions.push(mo::fade(
                &zone_id,
                t_in2,
                ed2 * 0.8,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
            let grow = fit(b, ev0, d * 1.1, ant - 0.1);
            b.motions
                .push(mo::expand(&zone_id, ev0, grow, rect(far, near1), settle));
            b.motions
                .push(mo::scale(&prim.id, ev0, grow, 1.0, 0.82, settle));
            b.motions.push(mo::shift(
                &prim.id,
                ev0,
                grow,
                [0.0, 0.0],
                [-sign * 0.06 * w, 0.0],
                settle,
            ));
            b.motions
                .push(mo::scale(&sec.id, ev0, grow, 1.0, 1.1, settle));
            // The relationship: an arrow from the secondary's pressure zone
            // onto the primary (its yielded edge), drawn as the zone grows.
            let (from, to) = span(
                sec.cx - sign * (1.1 * sec.bw / 2.0 + pad),
                prim_end + sign * (0.82 * prim.bw / 2.0 + pad),
                -sign,
                min_len,
            );
            draw_arrow(
                ctx,
                b,
                &[(from, ty), (to, ty)],
                ev0,
                fit(b, ev0, d * 1.1, ant - 0.05),
                ease_move.landing(),
            );
            b.push(zone);
            b.push(prim.layer);
            b.push(sec.layer);
        }

        // ---------------------------------------------------------------
        // grow: the secondary token grows 0.6 -> 1.25.
        // ---------------------------------------------------------------
        Op::Grow => {
            let mut prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size: sz(0.34),
                    cx: if mirror { 0.73 * w } else { 0.27 * w },
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            let mut sec = token(
                ctx,
                b,
                &TokenParams {
                    name: "secondary",
                    text: s_text.unwrap_or(""),
                    primary: false,
                    size: sz(0.24),
                    cx: if mirror { 0.27 * w } else { 0.73 * w },
                    cy: ty,
                    z: 23,
                    label_above: false,
                    image: false,
                    label_size: 30.0 * u,
                },
            );
            // The grown secondary's label stays inside the safe area; then
            // room for the arrow between the two bodies (the grown secondary
            // included): nudged apart only when the gap is short of it.
            prim.keep_inside(ctx, 1.0);
            sec.keep_inside(ctx, 1.25);
            let gap =
                sign * ((sec.cx - sign * 1.25 * sec.bw / 2.0) - (prim.cx + sign * prim.bw / 2.0));
            let short_by = (5.0 * ARROW_HEAD * u - gap).max(0.0);
            prim.nudge(ctx, -sign, short_by / 2.0, 1.0);
            sec.nudge(ctx, sign, short_by / 2.0, 1.25);
            sec.set_scale(0.6);
            enter(b, &prim.id, t_in, ed, reveal);
            enter(b, &sec.id, t_in2, ed2, false);
            b.motions.push(mo::scale(
                &sec.id,
                ev0,
                fit(b, ev0, d * 1.2, ant - 0.1),
                1.0,
                1.25 / 0.6,
                settle,
            ));
            // The relationship: an arrow along the track from the primary to
            // where the secondary ends up (its grown edge), drawn as it grows.
            let (from, to) = span(
                prim.cx + sign * (prim.bw / 2.0 + pad),
                sec.cx - sign * (1.25 * sec.bw / 2.0 + pad),
                sign,
                min_len,
            );
            draw_arrow(
                ctx,
                b,
                &[(from, ty), (to, ty)],
                ev0,
                fit(b, ev0, d * 1.1, ant - 0.05),
                ease_move.landing(),
            );
            b.push(prim.layer);
            b.push(sec.layer);
        }

        // ---------------------------------------------------------------
        // separate: adjacent at the center, drifting apart with a divide.
        // ---------------------------------------------------------------
        Op::Separate => {
            let size = sz(0.32);
            let mut prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size,
                    cx: 0.5 * w,
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 30.0 * u,
                },
            );
            let mut sec = token(
                ctx,
                b,
                &TokenParams {
                    name: "secondary",
                    text: s_text.unwrap_or(""),
                    primary: false,
                    size,
                    cx: 0.5 * w,
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 30.0 * u,
                },
            );
            let half = (prim.gw + sec.gw) / 4.0 + 6.0 * u;
            prim.set_cx(ctx, 0.5 * w - sign * half);
            sec.set_cx(ctx, 0.5 * w + sign * half);
            let (p_from, s_from) = (prim.cx, sec.cx);
            enter(b, &prim.id, t_in, ed, reveal);
            enter(b, &sec.id, t_in2, ed2, false);
            let drift = fit(b, ev0, d * 1.3, ant - 0.1);
            let p_to = clamp_cx(ctx, 0.5 * w - sign * 0.25 * w, prim.gw);
            let s_to = clamp_cx(ctx, 0.5 * w + sign * 0.25 * w, sec.gw);
            b.motions.push(mo::shift(
                &prim.id,
                ev0,
                drift,
                [0.0, 0.0],
                [p_to - p_from, 0.0],
                settle,
            ));
            b.motions.push(mo::shift(
                &sec.id,
                ev0,
                drift,
                [0.0, 0.0],
                [s_to - s_from, 0.0],
                settle,
            ));
            // The divide appears between them as they part.
            let divide_id = b.id("divide");
            let dh = 0.3 * short;
            let mut divide = rect_layer(
                divide_id.clone(),
                (0.5 * w - 1.5 * u, ty - dh / 2.0, 3.0 * u, dh),
                ctx.palette.ink.with_alpha(0x59),
                11,
            );
            divide.depth = plane(plan.lang.depth.midground);
            b.motions.push(mo::mask(
                &divide_id,
                ev0 + 0.15_f64.min(drift * 0.3),
                fit(b, ev0, d * 0.9, ant + 0.2),
                Direction::Down,
                Easing::OutCubic,
            ));
            // The relationship: a VS badge on the divide, once the tokens
            // have parted (never over a body: sized to the room between them).
            let room_p = sign * (0.5 * w - (p_to + sign * prim.bw / 2.0));
            let room_s = sign * ((s_to - sign * sec.bw / 2.0) - 0.5 * w);
            let room = room_p.min(room_s);
            let vs_d = (0.15 * short).min(2.0 * (room - pad)).max(0.08 * short);
            let vs_at = ev0 + 0.55 * drift;
            badge(
                ctx,
                b,
                "vs",
                (0.5 * w, ty),
                vs_d,
                Mark::Word("VS"),
                vs_at,
                fit(b, vs_at, 0.45, ant - 0.02),
            );
            b.push(divide);
            b.push(prim.layer);
            b.push(sec.layer);
        }

        // ---------------------------------------------------------------
        // carry: the primary travels along the track with the secondary
        // riding on top of it.
        // ---------------------------------------------------------------
        Op::Carry => {
            // Landscape has less height above the track: smaller tokens keep
            // the rider's label clear of the headline.

            let size = sz(tallness(0.34, 0.30));
            let from = if mirror { 0.75 * w } else { 0.25 * w };
            let to = if mirror { 0.25 * w } else { 0.75 * w };
            let prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size,
                    cx: from,
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            let rider_size = sz(tallness(0.15, 0.12));
            let (_, rbh) = body_size(family_of(&entity_key(s_text.unwrap_or(""))), rider_size);
            let mut rider = token(
                ctx,
                b,
                &TokenParams {
                    name: "secondary",
                    text: s_text.unwrap_or(""),
                    primary: false,
                    size: rider_size,
                    cx: prim.cx,
                    cy: ty - prim.bh / 2.0 - rbh / 2.0 - 12.0 * u,
                    z: 24,
                    label_above: true,
                    image: false,
                    label_size: 26.0 * u,
                },
            );
            // Room for the link: the rider rises by half of the free height
            // between its label and the headline (at most 44u), so the link
            // between the two bodies is more than a nub.
            let rider_top = rider.layer.y - (rider.by + rider.bh / 2.0);
            let free = rider_top - (head_top + head.height()) - 14.0 * u;
            rider.layer.y -= (0.5 * free).clamp(0.0, 44.0 * u);
            enter(b, &prim.id, t_in, ed, reveal);
            enter(b, &rider.id, t_in2, ed2, false);
            // The relationship: a link stem from the carrier up to its rider,
            // drawn once the rider has landed and travelling with them.
            let link_x = 0.5 * (prim.cx + rider.cx);
            let link_id = b.id("link");
            let link_at = t_in2 + ed2;
            b.motions.push(trim(
                &link_id,
                link_at,
                fit(b, link_at, 0.4, ant - 0.05),
                0.0,
                1.0,
                ease_move.landing(),
            ));
            b.push(poly_layer(
                link_id.clone(),
                &[
                    (link_x, ty - prim.bh / 2.0),
                    (link_x, rider.layer.y + rider.bh / 2.0),
                ],
                Stroke {
                    color: connector_color(ctx),
                    width: ARROW_WIDTH * u,
                },
                false,
                None,
                plane(plan.lang.depth.midground),
            ));
            let travel = fit(b, ev0, d * 1.5, ant - 0.1);
            let dx = clamp_cx(ctx, to, prim.gw) - prim.cx;
            for id in [&prim.id, &rider.id, &link_id] {
                b.motions
                    .push(mo::shift(id, ev0, travel, [0.0, 0.0], [dx, 0.0], settle));
            }
            b.push(prim.layer);
            b.push(rider.layer);
        }

        // ---------------------------------------------------------------
        // accumulate / none / other: both tokens on the track, a connector
        // draws between them.
        // ---------------------------------------------------------------
        Op::Connect => {
            // (0.23) Accumulate: a plus badge on the line between the tokens,
            // which are nudged apart when the gap is short of the badge.
            let accumulate = beat.relationship == Some(Relationship::Accumulate);
            let size = sz(0.34);
            let mut prim = token(
                ctx,
                b,
                &TokenParams {
                    name: "primary",
                    text: p_text,
                    primary: true,
                    size,
                    cx: if mirror { 0.73 * w } else { 0.27 * w },
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            let mut sec = token(
                ctx,
                b,
                &TokenParams {
                    name: "secondary",
                    text: s_text.unwrap_or(""),
                    primary: false,
                    size,
                    cx: if mirror { 0.27 * w } else { 0.73 * w },
                    cy: ty,
                    z: 22,
                    label_above: false,
                    image: false,
                    label_size: 32.0 * u,
                },
            );
            if accumulate {
                let gap =
                    sign * ((sec.cx - sign * sec.bw / 2.0) - (prim.cx + sign * prim.bw / 2.0));
                let short_by = (0.15 * short + 2.0 * pad - gap).max(0.0);
                prim.nudge(ctx, -sign, short_by / 2.0, 1.0);
                sec.nudge(ctx, sign, short_by / 2.0, 1.0);
            }
            enter(b, &prim.id, t_in, ed, reveal);
            enter(b, &sec.id, t_in2, ed2, false);
            let a = prim.cx + sign * (prim.bw / 2.0 + 6.0 * u);
            let z = sec.cx - sign * (sec.bw / 2.0 + 6.0 * u);
            let (x0, len) = (a.min(z), (a - z).abs());
            // A line with an arrowhead at the secondary's end, drawn as one
            // stroke so the head appears as the trim reaches it (accumulate:
            // a plain line, the plus badge sits on it).
            let head = (16.0 * u).min(len * 0.3);
            let pts = if accumulate {
                vec![[0.0, 0.0], [len, 0.0]]
            } else if sign > 0.0 {
                vec![
                    [0.0, 0.0],
                    [len, 0.0],
                    [len - head, -head],
                    [len, 0.0],
                    [len - head, head],
                ]
            } else {
                vec![
                    [len, 0.0],
                    [0.0, 0.0],
                    [head, -head],
                    [0.0, 0.0],
                    [head, head],
                ]
            };
            let cid = b.id("connector");
            let mut conn = base_layer(
                cid.clone(),
                (x0, ty, len, 6.0 * u),
                LayerKind::Polyline {
                    points: pts,
                    stroke: Stroke {
                        color: if accumulate {
                            connector_color(ctx)
                        } else {
                            ctx.palette.accent
                        },
                        width: 7.0 * u,
                    },
                    closed: false,
                    fill: None,
                },
                12,
            );
            conn.depth = plane(plan.lang.depth.midground);
            let line_dur = fit(
                b,
                ev0,
                d * 1.1,
                if accumulate { ant - 0.05 } else { ant + 0.2 },
            );
            b.motions
                .push(trim(&cid, ev0, line_dur, 0.0, 1.0, ease_move.landing()));
            if accumulate {
                let at = ev0 + 0.45 * line_dur;
                let plus_d = (0.15 * short).min(len - 2.0 * pad).max(0.08 * short);
                badge(
                    ctx,
                    b,
                    "plus",
                    (x0 + len / 2.0, ty),
                    plus_d,
                    Mark::Plus,
                    at,
                    fit(b, at, 0.45, ant - 0.02),
                );
            }
            b.push(conn);
            b.push(prim.layer);
            b.push(sec.layer);
        }
    }

    b.anchor = Some(recipes::Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;
    use crate::assets::AssetManifest;
    use crate::compiler::{plan_timing, ApproxMeasure, AssetLibrary, FontSet, Palette, Typesetter};
    use crate::intent::{CreativeIntent, Format};
    use crate::scene::{
        Asset, AssetKind, Canvas, Lifecycle, MotionProject, ProjectMeta, Scene, Theme,
        SCENE_VERSION,
    };
    use crate::style::StyleProfile;

    fn beat_json(
        purpose: &str,
        rel: Option<&str>,
        energy: &str,
        primary: &str,
        secondary: Option<&str>,
    ) -> serde_json::Value {
        let mut v = serde_json::json!({
            "purpose": purpose,
            "statement": format!("{primary} meets the long road ahead."),
            "primary": {"kind": "phrase", "value": primary},
            "energy": energy,
        });
        if let Some(s) = secondary {
            v["secondary"] = serde_json::json!({"kind": "phrase", "value": s});
        }
        if let Some(r) = rel {
            v["relationship"] = serde_json::json!(r);
        }
        v
    }

    fn format_name(f: Format) -> &'static str {
        match f {
            Format::Vertical => "vertical",
            Format::Square => "square",
            Format::Landscape => "landscape",
        }
    }

    /// Everything a test needs from one built beat.
    struct Built {
        stage: Vec<Layer>,
        motions: Vec<Motion>,
        life: Lifecycle,
        duration: f64,
        start: f64,
        w: f32,
        h: f32,
        assets: BTreeMap<String, Asset>,
        project: MotionProject,
    }

    /// Build beat `idx` of a story with the Entities builder.
    fn build_case(
        format: Format,
        beats: &[serde_json::Value],
        idx: usize,
        grammar: Grammar,
        variant: Variant,
    ) -> Built {
        build_with(
            format,
            beats,
            idx,
            grammar,
            variant,
            &AssetManifest::empty(),
        )
    }

    fn build_with(
        format: Format,
        beats: &[serde_json::Value],
        idx: usize,
        grammar: Grammar,
        variant: Variant,
        manifest: &AssetManifest,
    ) -> Built {
        let intent: CreativeIntent = serde_json::from_value(serde_json::json!({
            "version": "0.2",
            "title": "t",
            "format": format_name(format),
            "beats": beats,
        }))
        .expect("intent");
        let style = StyleProfile::default();
        let plans = plan_timing(&intent, &style);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_style(&style);
        let (w, h) = match format {
            Format::Vertical => (1080.0, 1920.0),
            Format::Square => (1080.0, 1080.0),
            Format::Landscape => (1920.0, 1080.0),
        };
        let mut ctx = Ctx {
            reveals: Default::default(),
            warnings: Vec::new(),
            direction_seed: None,
            beat_params: BTreeMap::new(),
            direction_take: None,
            direction_beats: BTreeMap::new(),
            emotion: None,
            art: None,
            w,
            h,
            u: 1.0,
            style: &style,
            taste: crate::compiler::taste::resolve(&style),
            palette: Palette::for_style(&style),
            ts: Typesetter::new(&ApproxMeasure, &library.root, &fonts),
            library: &library,
            assets: BTreeMap::new(),
            format,
            frame: crate::compiler::layout_frame::LayoutFrame::for_format(format),
            manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: beats.len(),
            sequence: Default::default(),
        };
        let mut b = B {
            reveals: Vec::new(),
            plan: &plans[idx],
            beat: &intent.beats[idx],
            stage: Vec::new(),
            motions: Vec::new(),
            anchor: None,
            placed: Vec::new(),
            split_above: None,
            focal: None,
        };
        let comp = Composition {
            grammar,
            variant,
            shape: super::super::Shape::Atomic,
            depiction: super::super::Depiction::Entities,
            reason: "test",
        };
        build(&mut ctx, &mut b, &mut Vec::new(), comp).expect("build");
        let plan = &plans[idx];
        let scene = Scene {
            post: Vec::new(),
            id: plan.id.clone(),
            start_seconds: 0.0,
            duration_seconds: plan.duration,
            layers: b.stage.clone(),
            motions: b.motions.clone(),
            camera: None,
            lifecycle: Some(plan.life),
        };
        let project = MotionProject {
            envelopes: Vec::new(),
            version: SCENE_VERSION.to_string(),
            project: ProjectMeta {
                name: "t".into(),
                duration_seconds: Some(plan.duration),
                exploration: None,
                speech: None,
                art: None,
                direction: None,
            },
            canvas: Canvas {
                width: w as u32,
                height: h as u32,
                fps: 30,
                background: ctx.palette.paper,
            },
            theme: Theme {
                fonts: fonts
                    .roles
                    .iter()
                    .map(|(r, f)| (*r, f.asset_id.to_string()))
                    .collect(),
                palette: ctx.palette.named(),
                typography: None,
            },
            assets: fonts
                .faces
                .iter()
                .map(|f| Asset {
                    id: f.asset_id.to_string(),
                    kind: AssetKind::Font,
                    path: f.path.to_string(),
                    sprite: None,
                })
                .chain(ctx.assets.values().cloned())
                .collect(),
            asset_root: None,
            scenes: vec![scene],
            shared: Vec::new(),
        };
        Built {
            stage: b.stage,
            motions: b.motions,
            life: plan.life,
            duration: plan.duration,
            start: plan.start,
            w,
            h,
            assets: ctx.assets.clone(),
            project,
        }
    }

    fn all_ids(layers: &[Layer], out: &mut BTreeSet<String>) {
        for l in layers {
            out.insert(l.id.clone());
            if let LayerKind::Group { children } = &l.kind {
                all_ids(children, out);
            }
        }
    }

    fn ids(b: &Built) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        all_ids(&b.stage, &mut out);
        out
    }

    const OWNED: [&str; 8] = [
        "stage_",
        "entity_",
        "zone",
        "connector",
        "divide",
        "progress",
        "track_",
        "rungs",
    ];

    /// Ids owned by the diagram builder (not the shared ghost / headline).
    fn is_diagram_target(id: &str) -> bool {
        OWNED
            .iter()
            .any(|p| id.split('.').nth(1).is_some_and(|s| s.starts_with(p)))
    }

    struct Case {
        name: &'static str,
        grammar: Grammar,
        beat: serde_json::Value,
        mirror: bool,
        expect: &'static [&'static str],
    }

    const BASE: &[&str] = &["track_a", "track_b", "rungs", "progress", "entity_primary"];
    const BOTH: &[&str] = &[
        "track_a",
        "track_b",
        "rungs",
        "progress",
        "entity_primary",
        "entity_secondary",
        "label_primary",
        "label_secondary",
    ];

    fn case(
        name: &'static str,
        grammar: Grammar,
        beat: serde_json::Value,
        mirror: bool,
        expect: &'static [&'static str],
    ) -> Case {
        Case {
            name,
            grammar,
            beat,
            mirror,
            expect,
        }
    }

    fn cases() -> Vec<Case> {
        let sec = Some("Rising costs");
        let hero = Grammar::HeroObject;
        let sce = Grammar::SpatialCauseEffect;
        vec![
            case(
                "hero_solo",
                hero,
                beat_json("emphasize", None, "calm", "Fixed budget", None),
                false,
                BASE,
            ),
            case(
                "hero_calm",
                hero,
                beat_json("emphasize", None, "calm", "Fixed budget", sec),
                false,
                BOTH,
            ),
            case(
                "hero_impact",
                hero,
                beat_json("emphasize", None, "impact", "Fixed budget", sec),
                false,
                BOTH,
            ),
            case(
                "hero_impact_mirror",
                hero,
                beat_json("emphasize", None, "impact", "Fixed budget", sec),
                true,
                BOTH,
            ),
            case(
                "hero_reveal",
                hero,
                beat_json("reveal", None, "building", "Fixed budget", sec),
                false,
                BOTH,
            ),
            case(
                "replace",
                sce,
                beat_json("contrast", Some("replace"), "building", "Old process", sec),
                false,
                BOTH,
            ),
            case(
                "compress",
                sce,
                beat_json(
                    "contrast",
                    Some("compress"),
                    "building",
                    "Fixed budget",
                    sec,
                ),
                false,
                &[
                    "track_a",
                    "entity_primary",
                    "entity_secondary",
                    "zone",
                    "label_secondary",
                ],
            ),
            case(
                "grow",
                sce,
                beat_json("compare", Some("grow"), "building", "Small team", sec),
                false,
                BOTH,
            ),
            case(
                "separate",
                sce,
                beat_json("compare", Some("separate"), "calm", "Sales", sec),
                false,
                &["entity_primary", "entity_secondary", "divide"],
            ),
            case(
                "carry",
                sce,
                beat_json("contrast", Some("carry"), "building", "Main route", sec),
                false,
                BOTH,
            ),
            case(
                "none",
                sce,
                beat_json("contrast", None, "impact", "Cause", sec),
                false,
                &["entity_primary", "entity_secondary", "connector"],
            ),
            case(
                "accumulate",
                sce,
                beat_json("compare", Some("accumulate"), "building", "Cause", sec),
                false,
                &["connector"],
            ),
            case(
                "sce_solo",
                sce,
                beat_json("contrast", None, "building", "Cause", None),
                false,
                BASE,
            ),
        ]
    }

    fn variant(mirror: bool) -> Variant {
        if mirror {
            Variant::Mirror
        } else {
            Variant::Standard
        }
    }

    const FORMATS: [Format; 3] = [Format::Vertical, Format::Square, Format::Landscape];

    fn build_one(format: Format, c: &Case) -> Built {
        build_case(
            format,
            std::slice::from_ref(&c.beat),
            0,
            c.grammar,
            variant(c.mirror),
        )
    }

    #[test]
    fn every_case_builds_with_expected_ids_timing_and_validates() {
        for format in FORMATS {
            for c in cases() {
                let built = build_one(format, &c);
                let have = ids(&built);
                for want in c.expect {
                    assert!(
                        have.iter().any(|id| id.ends_with(&format!(".{want}"))),
                        "{} {format:?}: missing {want} in {have:?}",
                        c.name
                    );
                }
                for m in built
                    .motions
                    .iter()
                    .filter(|m| is_diagram_target(&m.target))
                {
                    assert!(
                        m.start < built.life.anticipate,
                        "{} {format:?}: {} starts at {} >= anticipate {}",
                        c.name,
                        m.target,
                        m.start,
                        built.life.anticipate
                    );
                    assert!(
                        m.start + m.duration <= built.duration + 1e-6,
                        "{} {format:?}: {} ends {} > scene {}",
                        c.name,
                        m.target,
                        m.start + m.duration,
                        built.duration
                    );
                }
                if let Err(e) = crate::validate::validate(&built.project, None) {
                    // Shared headline / ghost motions are clamped later by the
                    // compiler (`clamp_to_scene`); only diagram problems count.
                    let ours: Vec<String> =
                        e.0.iter()
                            .map(|x| x.to_string())
                            .filter(|s| OWNED.iter().any(|p| s.contains(p)))
                            .collect();
                    assert!(ours.is_empty(), "{} {format:?}: {ours:?}", c.name);
                }
            }
        }
    }

    #[test]
    fn same_phrase_same_family_and_colors_across_beats() {
        assert_eq!(
            family_of(&entity_key("Fixed  budget!")),
            family_of(&entity_key("fixed budget"))
        );
        assert_eq!(entity_key("  Fixed-Budget, 2x "), "fixed budget 2x");
        let distinct: BTreeSet<String> = (0..40)
            .map(|i| format!("{:?}", family_of(&entity_key(&format!("phrase {i}")))))
            .collect();
        assert!(distinct.len() >= 3, "{distinct:?}");

        // The same primary phrase in two different beats (different
        // secondary, grammar and energy) draws the same body.
        let story = [
            beat_json("emphasize", None, "calm", "Fixed budget", None),
            beat_json(
                "contrast",
                Some("compress"),
                "impact",
                "Fixed budget",
                Some("Rising costs"),
            ),
        ];
        let body = |idx: usize, g: Grammar| {
            let built = build_case(Format::Vertical, &story, idx, g, Variant::Standard);
            let group = built
                .stage
                .iter()
                .find(|l| l.id.ends_with(".entity_primary"))
                .cloned()
                .expect("primary token");
            let LayerKind::Group { children } = group.kind else {
                panic!("token is a group");
            };
            children
                .into_iter()
                .find(|c| c.id.ends_with(".body"))
                .map(|c| c.kind)
                .expect("body")
        };
        match (
            body(0, Grammar::HeroObject),
            body(1, Grammar::SpatialCauseEffect),
        ) {
            (
                LayerKind::RoundedRectangle {
                    fill: f1,
                    stroke: s1,
                    radius: r1,
                },
                LayerKind::RoundedRectangle {
                    fill: f2,
                    stroke: s2,
                    radius: r2,
                },
            ) => {
                assert_eq!((f1, s1), (f2, s2));
                assert!(r1 > 0.0 && r2 > 0.0);
            }
            (
                LayerKind::Polyline {
                    fill: f1,
                    stroke: s1,
                    points: p1,
                    ..
                },
                LayerKind::Polyline {
                    fill: f2,
                    stroke: s2,
                    points: p2,
                    ..
                },
            ) => {
                assert_eq!((f1, s1), (f2, s2));
                assert_eq!(p1.len(), p2.len());
            }
            (a, b) => panic!("different families: {a:?} vs {b:?}"),
        }
    }

    #[test]
    fn build_is_deterministic() {
        for c in cases() {
            let (a, b) = (
                build_one(Format::Vertical, &c),
                build_one(Format::Vertical, &c),
            );
            assert_eq!(a.stage, b.stage, "{}", c.name);
            assert_eq!(a.motions, b.motions, "{}", c.name);
        }
    }

    #[test]
    fn track_geometry_is_identical_across_beats() {
        for format in FORMATS {
            let story = [
                beat_json("emphasize", None, "calm", "Fixed budget", None),
                beat_json(
                    "contrast",
                    Some("compress"),
                    "impact",
                    "Other thing",
                    Some("Rising costs"),
                ),
            ];
            let a = build_case(format, &story, 0, Grammar::HeroObject, Variant::Standard);
            let b = build_case(
                format,
                &story,
                1,
                Grammar::SpatialCauseEffect,
                Variant::Standard,
            );
            for name in ["track_a", "track_b", "rungs"] {
                let pick = |built: &Built| {
                    let mut l = built
                        .stage
                        .iter()
                        .find(|l| l.id.ends_with(&format!(".{name}")))
                        .cloned()
                        .expect(name);
                    l.id.clear();
                    // Depth is the beat language's plane, not geometry.
                    l.depth = None;
                    if let LayerKind::Group { children } = &mut l.kind {
                        for c in children {
                            c.id.clear();
                        }
                    }
                    l
                };
                assert_eq!(pick(&a), pick(&b), "{name} {format:?}");
            }
        }
    }

    /// Final canvas-space box of a token group (base + last move + last scale).
    fn final_rect(built: &Built, group: &Layer) -> (f32, f32, f32, f32) {
        let last = |pick: fn(&MotionOp) -> bool| {
            built
                .motions
                .iter()
                .filter(|m| m.target == group.id && pick(&m.op))
                .max_by(|a, b| a.start.total_cmp(&b.start))
        };
        let (mut dx, mut dy) = (0.0, 0.0);
        if let Some(Motion {
            op: MotionOp::Move { to, .. },
            ..
        }) = last(|o| matches!(o, MotionOp::Move { .. }))
        {
            (dx, dy) = (to[0], to[1]);
        }
        let mut s = group.scale_x;
        if let Some(Motion {
            op: MotionOp::Scale { to, .. },
            ..
        }) = last(|o| matches!(o, MotionOp::Scale { .. }))
        {
            s = group.scale_x * to;
        }
        let left = group.x + dx - group.anchor_x * group.width * s;
        let top = group.y + dy - group.anchor_y * group.height * s;
        (left, top, group.width * s, group.height * s)
    }

    #[test]
    fn tokens_and_labels_stay_inside_every_format() {
        for format in FORMATS {
            for c in cases() {
                let built = build_one(format, &c);
                for l in built.stage.iter().filter(|l| l.id.contains(".entity_")) {
                    let base = (
                        l.x - l.anchor_x * l.width * l.scale_x,
                        l.y - l.anchor_y * l.height * l.scale_y,
                        l.width * l.scale_x,
                        l.height * l.scale_y,
                    );
                    for (label, r) in [("final", final_rect(&built, l)), ("base", base)] {
                        assert!(
                            r.0 >= -1.0
                                && r.1 >= -1.0
                                && r.0 + r.2 <= built.w + 1.0
                                && r.1 + r.3 <= built.h + 1.0,
                            "{} {format:?} {} {label}: {r:?} outside {}x{}",
                            c.name,
                            l.id,
                            built.w,
                            built.h
                        );
                    }
                }
            }
        }
    }

    fn story4() -> Vec<serde_json::Value> {
        vec![
            beat_json("emphasize", None, "calm", "Alpha", Some("Beta")),
            beat_json(
                "contrast",
                Some("compress"),
                "building",
                "Gamma",
                Some("Delta"),
            ),
            beat_json(
                "contrast",
                Some("replace"),
                "building",
                "Epsilon",
                Some("Zeta"),
            ),
            beat_json("emphasize", None, "impact", "Eta", None),
        ]
    }

    fn progress_trims(built: &Built) -> Vec<(f64, f32, f32)> {
        let mut v: Vec<(f64, f32, f32)> = built
            .motions
            .iter()
            .filter(|m| m.target.ends_with(".progress"))
            .filter_map(|m| match m.op {
                MotionOp::Trim { from, to } => Some((m.start, from, to)),
                _ => None,
            })
            .collect();
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
        v
    }

    #[test]
    fn progress_runs_index_over_n_to_next_and_last_ends_at_one() {
        let story = story4();
        for (idx, g) in [
            (0, Grammar::HeroObject),
            (1, Grammar::SpatialCauseEffect),
            (2, Grammar::SpatialCauseEffect),
            (3, Grammar::HeroObject),
        ] {
            let built = build_case(Format::Vertical, &story, idx, g, Variant::Standard);
            let trims = progress_trims(&built);
            let last = trims.last().expect("read-phase trim");
            assert!((last.1 - idx as f32 / 4.0).abs() < 1e-6, "{trims:?}");
            assert!((last.2 - (idx + 1) as f32 / 4.0).abs() < 1e-6, "{trims:?}");
            assert!(last.0 < built.life.anticipate);
            if idx == 3 {
                assert_eq!(last.2, 1.0);
            }
        }
    }

    #[test]
    fn stage_indicator_marks_each_beat_with_identical_layout() {
        let story = story4();
        let a = build_case(
            Format::Vertical,
            &story,
            0,
            Grammar::HeroObject,
            Variant::Standard,
        );
        let c = build_case(
            Format::Vertical,
            &story,
            2,
            Grammar::SpatialCauseEffect,
            Variant::Standard,
        );
        let stage = |built: &Built, i: usize| {
            built
                .stage
                .iter()
                .find(|l| l.id.ends_with(&format!(".stage_{i}")))
                .cloned()
                .unwrap_or_else(|| panic!("stage_{i}"))
        };
        for i in 0..4 {
            let (la, lc) = (stage(&a, i), stage(&c, i));
            // Same slot in every beat (the current one is larger, same center).
            assert_eq!((la.x, la.y), (lc.x, lc.y), "stage_{i}");
            assert_eq!(la.y, 0.84 * 1920.0);
        }
        let fill = |l: &Layer| match &l.kind {
            LayerKind::RoundedRectangle { fill, .. } => *fill,
            _ => panic!("mark is a rounded rectangle"),
        };
        let pal = Palette::for_style(&StyleProfile::default());
        // Beat 2: past = ink, current = accent, future = transparent.
        assert_eq!(fill(&stage(&c, 0)), pal.ink);
        assert_eq!(fill(&stage(&c, 1)), pal.ink);
        assert_eq!(fill(&stage(&c, 2)), pal.accent);
        assert_eq!(fill(&stage(&c, 3)).a, 0);
        // The current mark grows in during ENTER; no stage_4.
        let cur = stage(&c, 2).id;
        assert!(
            c.motions
                .iter()
                .any(|m| m.target == cur
                    && matches!(m.op, MotionOp::Scale { from, .. } if from < 0.5))
        );
        assert!(!ids(&c).iter().any(|id| id.ends_with(".stage_4")));
    }

    #[test]
    fn delivered_hero_object_image_replaces_the_procedural_hero() {
        let manifest = AssetManifest {
            version: "0.1".into(),
            assets: vec![crate::assets::ManifestEntry {
                id: "beat_1.hero_object".into(),
                path: "test_images/figure.png".into(),
                width: 600,
                height: 400,
                alpha: true,
                ..Default::default()
            }],
            missing: Vec::new(),
        };
        let story = [beat_json("emphasize", None, "calm", "Alpha", Some("Beta"))];
        let built = build_with(
            Format::Vertical,
            &story,
            0,
            Grammar::HeroObject,
            Variant::Standard,
            &manifest,
        );
        let group = built
            .stage
            .iter()
            .find(|l| l.id.ends_with(".entity_primary"))
            .expect("primary token");
        let LayerKind::Group { children } = &group.kind else {
            panic!("token is a group");
        };
        assert!(
            children
                .iter()
                .any(|c| matches!(c.kind, LayerKind::Image { .. })),
            "image hero"
        );
        assert!(!children.iter().any(|c| c.id.ends_with(".body")));
        assert!(!children.iter().any(|c| c.id.ends_with(".mark")));
        assert!(children.iter().any(|c| c.id.ends_with("label_primary")));
        assert!(built.assets.contains_key("asset.beat_1.hero_object"));
        // The marker stays procedural.
        assert!(ids(&built)
            .iter()
            .any(|id| id.ends_with("entity_secondary.body")));
        // Without the entry the token is procedural.
        let plain = build_one(Format::Vertical, &cases()[1]);
        let g = plain
            .stage
            .iter()
            .find(|l| l.id.ends_with(".entity_primary"))
            .expect("token");
        let LayerKind::Group { children } = &g.kind else {
            panic!("group");
        };
        assert!(children.iter().any(|c| c.id.ends_with(".body")));
        // SpatialCauseEffect never uses the hero image.
        let sce = build_with(
            Format::Vertical,
            &story,
            0,
            Grammar::SpatialCauseEffect,
            Variant::Standard,
            &manifest,
        );
        assert!(!sce.assets.contains_key("asset.beat_1.hero_object"));
    }

    #[test]
    fn marker_label_fades_with_the_marker_when_passed_over() {
        let built = build_one(
            Format::Vertical,
            &cases()
                .into_iter()
                .find(|c| c.name == "hero_impact")
                .expect("case"),
        );
        assert!(built
            .motions
            .iter()
            .any(|m| m.target.ends_with("label_secondary")
                && matches!(m.op, MotionOp::Fade { to, .. } if to == 0.0)));
    }

    // -----------------------------------------------------------------------
    // (0.23 W8d) Relationship connectors
    // -----------------------------------------------------------------------

    type Box4 = (f32, f32, f32, f32);

    fn boxes_hit(a: Box4, b: Box4) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    /// First word of a beat layer's name (`b2.arrow_head` -> `arrow`).
    fn name_head(id: &str) -> &str {
        id.split('.')
            .nth(1)
            .and_then(|n| n.split('_').next())
            .unwrap_or("")
    }

    /// Top-level connector layers of the beat.
    fn connector_layers(built: &Built) -> Vec<&Layer> {
        built
            .stage
            .iter()
            .filter(|l| matches!(name_head(&l.id), "arrow" | "plus" | "vs" | "link"))
            .collect()
    }

    /// Canvas-space box of a child of a token group at the group's final pose.
    fn child_box(built: &Built, group: &Layer, child: &Layer) -> Box4 {
        let (left, top, gw, _) = final_rect(built, group);
        let s = gw / group.width;
        let (cx, cy, cw, ch) = (child.x, child.y, child.width, child.height);
        if child.anchor_x > 0.0 {
            // A rotated, centred body: its bounding square.
            let diag = cw.max(ch) * std::f32::consts::SQRT_2;
            return (
                left + (cx - diag / 2.0) * s,
                top + (cy - diag / 2.0) * s,
                diag * s,
                diag * s,
            );
        }
        (left + cx * s, top + cy * s, cw * s, ch * s)
    }

    /// (body box, label box) of the token `entity_<name>` at its final pose.
    fn token_boxes(built: &Built, name: &str) -> (Box4, Box4) {
        let group = built
            .stage
            .iter()
            .find(|l| l.id.ends_with(&format!(".entity_{name}")))
            .expect("token");
        let LayerKind::Group { children } = &group.kind else {
            panic!("token is a group");
        };
        let body = children
            .iter()
            .find(|c| c.id.ends_with(".body"))
            .expect("body");
        let label = children
            .iter()
            .find(|c| c.id.contains("label_"))
            .expect("label");
        (
            child_box(built, group, body),
            child_box(built, group, label),
        )
    }

    /// The strokes of a polyline layer as thin boxes (stroke width included).
    fn stroke_boxes(l: &Layer) -> Vec<Box4> {
        let LayerKind::Polyline { points, stroke, .. } = &l.kind else {
            return Vec::new();
        };
        let h = stroke.width / 2.0;
        points
            .windows(2)
            .map(|w| {
                let (a, b) = (
                    (l.x + w[0][0], l.y + w[0][1]),
                    (l.x + w[1][0], l.y + w[1][1]),
                );
                (
                    a.0.min(b.0) - h,
                    a.1.min(b.1) - h,
                    (a.0 - b.0).abs() + 2.0 * h,
                    (a.1 - b.1).abs() + 2.0 * h,
                )
            })
            .collect()
    }

    /// Whether the strokes of a polyline layer (width included, closing edge
    /// too) pass through the box `b`.
    fn strokes_hit(l: &Layer, b: Box4) -> bool {
        let LayerKind::Polyline {
            points,
            stroke,
            closed,
            ..
        } = &l.kind
        else {
            return false;
        };
        let h = stroke.width / 2.0;
        let inside = |p: (f32, f32)| {
            p.0 >= b.0 - h && p.0 <= b.0 + b.2 + h && p.1 >= b.1 - h && p.1 <= b.1 + b.3 + h
        };
        let pts: Vec<(f32, f32)> = points.iter().map(|p| (l.x + p[0], l.y + p[1])).collect();
        let n = pts.len();
        let edges = if *closed { n } else { n.saturating_sub(1) };
        (0..edges).any(|i| {
            let (a, c) = (pts[i], pts[(i + 1) % n]);
            let len = ((c.0 - a.0).powi(2) + (c.1 - a.1).powi(2)).sqrt();
            let steps = (len / 2.0).ceil().max(1.0) as usize;
            (0..=steps).any(|k| {
                let t = k as f32 / steps as f32;
                inside((a.0 + (c.0 - a.0) * t, a.1 + (c.1 - a.1) * t))
            })
        })
    }

    fn connector_case(name: &str) -> Case {
        cases().into_iter().find(|c| c.name == name).expect("case")
    }

    /// The connector each relationship case draws (word of the layer name).
    const RELATION_CONNECTORS: [(&str, &str); 6] = [
        ("grow", "arrow"),
        ("replace", "arrow"),
        ("compress", "arrow"),
        ("accumulate", "plus"),
        ("separate", "vs"),
        ("carry", "link"),
    ];

    #[test]
    fn every_relationship_draws_its_connector() {
        for format in FORMATS {
            for (name, word) in RELATION_CONNECTORS {
                for mirror in [false, true] {
                    let mut c = connector_case(name);
                    c.mirror = mirror;
                    let built = build_one(format, &c);
                    let drawn: Vec<&str> = connector_layers(&built)
                        .iter()
                        .map(|l| name_head(&l.id))
                        .collect();
                    assert!(
                        drawn.contains(&word),
                        "{name} {format:?} mirror={mirror}: want {word}, drew {drawn:?}"
                    );
                }
            }
            // Without a relationship the 0.7.1 line stays, and draws no new layer.
            let built = build_one(format, &connector_case("none"));
            assert!(connector_layers(&built).is_empty());
            assert!(ids(&built).iter().any(|id| id.ends_with(".connector")));
        }
    }

    #[test]
    fn connectors_draw_after_the_second_token_lands_and_before_anticipate() {
        for format in FORMATS {
            for (name, _) in RELATION_CONNECTORS {
                for mirror in [false, true] {
                    let mut c = connector_case(name);
                    c.mirror = mirror;
                    let built = build_one(format, &c);
                    let sec = built
                        .stage
                        .iter()
                        .find(|l| l.id.ends_with(".entity_secondary"))
                        .expect("secondary")
                        .id
                        .clone();
                    // The second token has landed when its first fade-in ends.
                    let lands = built
                        .motions
                        .iter()
                        .filter(|m| m.target == sec && matches!(m.op, MotionOp::Fade { .. }))
                        .min_by(|a, b| a.start.total_cmp(&b.start))
                        .map(|m| m.start + m.duration)
                        .expect("secondary fades in");
                    let layers = connector_layers(&built);
                    assert!(!layers.is_empty(), "{name}");
                    for l in layers {
                        let ms: Vec<&Motion> =
                            built.motions.iter().filter(|m| m.target == l.id).collect();
                        assert!(!ms.is_empty(), "{name} {}: no motion", l.id);
                        for m in &ms {
                            if matches!(m.op, MotionOp::Move { .. }) {
                                continue; // a carried link travels with its tokens
                            }
                            assert!(
                                m.start >= lands - 1e-6,
                                "{name} {format:?} {}: starts {} before the second token lands {}",
                                l.id,
                                m.start,
                                lands
                            );
                            assert!(
                                m.start + m.duration <= built.life.anticipate + 1e-6,
                                "{name} {format:?} {}: ends {} after ANTICIPATE {}",
                                l.id,
                                m.start + m.duration,
                                built.life.anticipate
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn connectors_stay_under_the_tokens_inside_the_canvas_and_clear_of_their_text() {
        for format in FORMATS {
            for (name, _) in RELATION_CONNECTORS {
                for mirror in [false, true] {
                    let mut c = connector_case(name);
                    c.mirror = mirror;
                    let built = build_one(format, &c);
                    let top_token = built
                        .stage
                        .iter()
                        .filter(|l| l.id.contains(".entity_"))
                        .map(|l| l.z_index)
                        .min()
                        .expect("tokens");
                    let labels = [
                        token_boxes(&built, "primary").1,
                        token_boxes(&built, "secondary").1,
                    ];
                    for l in connector_layers(&built) {
                        let what = format!("{name} {format:?} mirror={mirror} {}", l.id);
                        if matches!(l.kind, LayerKind::Polyline { .. }) {
                            assert!(l.z_index < top_token, "{what}: over a token");
                        }
                        for s in stroke_boxes(l) {
                            assert!(
                                s.0 >= -0.5
                                    && s.1 >= -0.5
                                    && s.0 + s.2 <= built.w + 0.5
                                    && s.1 + s.3 <= built.h + 0.5,
                                "{what}: {s:?} outside the canvas"
                            );
                        }
                        for lab in labels {
                            assert!(!strokes_hit(l, lab), "{what}: covers label {lab:?}");
                        }
                        if matches!(l.kind, LayerKind::Group { .. }) {
                            // A badge sits between the bodies: never over a label.
                            let b = (l.x - l.width / 2.0, l.y - l.height / 2.0, l.width, l.height);
                            for lab in labels {
                                assert!(!boxes_hit(b, lab), "{what}: {b:?} covers label {lab:?}");
                            }
                        }
                    }
                }
            }
        }
    }

    /// The arrowheads and badges sit in the gap between the token bodies, for
    /// phrases of different shapes and lengths.
    #[test]
    fn connectors_fit_between_the_bodies_for_any_phrase_shape() {
        let pairs = [
            ("Fixed budget", "Rising costs"),
            ("Small team", "Big team"),
            ("Sales", "Support"),
            ("First 20 years", "Last 10 years"),
            ("Old process", "Automation"),
            ("Quarterly revenue", "Churn"),
            ("A", "B"),
            ("Main route", "Extra freight"),
        ];
        for format in FORMATS {
            for (a, b2) in pairs {
                for (name, _) in RELATION_CONNECTORS {
                    let rel = connector_case(name).beat["relationship"]
                        .as_str()
                        .map(str::to_string)
                        .expect("relationship");
                    let purpose = if matches!(name, "grow" | "accumulate" | "separate") {
                        "compare"
                    } else {
                        "contrast"
                    };
                    let beat = beat_json(purpose, Some(rel.as_str()), "building", a, Some(b2));
                    let built = build_case(
                        format,
                        std::slice::from_ref(&beat),
                        0,
                        Grammar::SpatialCauseEffect,
                        Variant::Standard,
                    );
                    let bodies = [
                        token_boxes(&built, "primary").0,
                        token_boxes(&built, "secondary").0,
                    ];
                    for l in connector_layers(&built) {
                        for body in bodies {
                            let over = match &l.kind {
                                LayerKind::Group { .. } => boxes_hit(
                                    (l.x - l.width / 2.0, l.y - l.height / 2.0, l.width, l.height),
                                    body,
                                ),
                                _ if l.id.ends_with("arrow_head") => strokes_hit(l, body),
                                _ => false,
                            };
                            assert!(
                                !over,
                                "{name} {format:?} {a}/{b2}: {} over body {body:?}",
                                l.id
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn connector_colour_reads_against_the_paper() {
        use crate::compiler::explore::contrast_ratio;
        let pal = Palette::for_style(&StyleProfile::default());
        let built = build_one(Format::Vertical, &connector_case("grow"));
        let arrow = built
            .stage
            .iter()
            .find(|l| l.id.ends_with(".arrow"))
            .expect("arrow");
        let LayerKind::Polyline { stroke, .. } = &arrow.kind else {
            panic!("polyline");
        };
        assert!(stroke.color == pal.accent || stroke.color == pal.ink);
        assert!(contrast_ratio(stroke.color, pal.paper) >= CONNECTOR_MIN_CONTRAST);
    }

    /// The nudges that make room for a connector (grow, accumulate) and the
    /// replace advance never leave a label further outside the safe area than
    /// it stood before: its outward edge is inside the margin, or no further
    /// out than the label's old position.
    #[test]
    fn making_room_for_a_connector_keeps_labels_inside_the_safe_area() {
        let pairs = [
            ("Fixed budget", "Rising costs"),
            ("Quarterly recurring revenue", "Customer churn rate"),
            ("Manual spreadsheet reconciliation", "Automated ledger sync"),
            ("A", "B"),
        ];
        for format in FORMATS {
            for (a, b2) in pairs {
                for (name, old_primary, old_secondary) in [
                    ("grow", 0.27, 0.73),
                    ("accumulate", 0.27, 0.73),
                    ("replace", 0.5, 0.5 + 0.18),
                ] {
                    let rel = connector_case(name).beat["relationship"]
                        .as_str()
                        .map(str::to_string)
                        .expect("relationship");
                    for mirror in [false, true] {
                        let beat =
                            beat_json("compare", Some(rel.as_str()), "building", a, Some(b2));
                        let built = build_case(
                            format,
                            std::slice::from_ref(&beat),
                            0,
                            Grammar::SpatialCauseEffect,
                            variant(mirror),
                        );
                        let m = 84.0 * built.w.min(built.h) / 1080.0;
                        for (token, old) in [("primary", old_primary), ("secondary", old_secondary)]
                        {
                            if name == "replace" && token == "primary" {
                                continue; // lifted, not moved sideways
                            }
                            let old_cx = if mirror { 1.0 - old } else { old } * built.w;
                            let (_, label) = token_boxes(&built, token);
                            let half = label.2 / 2.0;
                            let (left, right) = (label.0, label.0 + label.2);
                            let (old_left, old_right) = (old_cx - half, old_cx + half);
                            assert!(
                                right <= (built.w - m).max(old_right) + 0.5
                                    && left >= m.min(old_left) - 0.5,
                                "{name} {format:?} mirror={mirror} {a}/{b2} {token}: label \
                                 {left:.0}..{right:.0} (was {old_left:.0}..{old_right:.0}, safe \
                                 {m:.0}..{:.0})",
                                built.w - m
                            );
                        }
                    }
                }
            }
        }
    }

    /// Writes one project per case and format to `$MOTION_DIAGRAM_OUT` for
    /// visual review (`motion-engine render <file> --frame N`).
    #[test]
    #[ignore = "writes preview projects; run with MOTION_DIAGRAM_OUT=<dir>"]
    fn dump_preview_projects() {
        let Ok(dir) = std::env::var("MOTION_DIAGRAM_OUT") else {
            return;
        };
        std::fs::create_dir_all(&dir).expect("out dir");
        for format in FORMATS {
            for c in cases() {
                // Beat 3 of 4 so progress and the stage indicator show state.
                let mut built = build_case(
                    format,
                    &vec![c.beat.clone(); 4],
                    2,
                    c.grammar,
                    variant(c.mirror),
                );
                built.project.asset_root =
                    Some(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets").to_string());
                let json = serde_json::to_string_pretty(&built.project).expect("json");
                let path = format!("{dir}/{}_{}.motion.json", c.name, format_name(format));
                std::fs::write(&path, json).expect("write");
                eprintln!(
                    "{path} duration={:.2} life={:?} start={:.2}",
                    built.duration, built.life, built.start
                );
            }
        }
    }
}
