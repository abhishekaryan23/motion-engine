//! (0.18) SphereGallery — a collection of 3+ items rides a slowly turning
//! sphere that brings each item to the front, one after another. At the front
//! an item sits on the focus plane (`z = 0`): sharp, 1:1 and slightly
//! enlarged, with its label; the rest are smaller, blurred by the depth of
//! field and pass behind. The look is the cinematic one (glow discs far back,
//! bokeh in front, a glyph-cascade title), so the grammar reuses its helpers.
//!
//! Every sphere layer (items and halo dots) is a top-level stage child placed
//! AT the sphere centre (anchor 0.5 / 0.5) with `z = R`: the sphere's front
//! point is then `z = 0`. The FX director hoists them into depth wrappers and
//! the timeline's `revolve` op moves each layer to its point of the sphere:
//! the sphere is rigid, so every layer gets the same chain of angles.
//!
//! | element | spec |
//! |---|---|
//! | glow / bokeh | the cinematic background (`cinematic3d::glow_discs`, `bokeh_discs`) |
//! | environment | (0.19) the matched environment plate behind the sphere (`cinematic3d::environment_plate`) |
//! | title | the statement (≤ 3 lines, `z = -120`, glyph cascade) |
//! | item `k` | the object picture (≈ 0.40 of the short side at the front) or a text card (phrase / number), at `[lon_k, lat_k]`: `lon_k = k·360/n`, `lat_k` a seeded zig-zag within ±25° |
//! | halo | 48 dots (`dust.<j>`: decorative) on a Fibonacci sphere of 0.94 R, riding the same rotation |
//! | label | the item's meaning (or humanised name) in a pill under the front, `z = 0`, fading in while the item is in front and out before the next turn |
//!
//! Rotation (angles `[-lon_k, -lat_k]` bring item `k` to the front): the
//! intro spins the sphere in during ENTER (≈ 1.2 s, OutCubic); READ..ANTICIPATE
//! is divided evenly into one dwell per item and at the start of dwell `k` the
//! sphere turns from item `k-1` to item `k` (≤ 0.8 s, InOutCubic, the short
//! way round in yaw). An item scales 1 → 1.12 (spring) when it arrives and
//! back to 1 as it leaves. The beat has no `focal` record: the front of the
//! sphere is the focus plane by construction.

use super::cinematic3d::{self, disc, sprung};
use super::{plate, Composition};
use crate::assets::{AssetRole, ManifestEntry};
use crate::compiler::catalog::{request_words, MatchKind};
use crate::compiler::recipes::{self, Slot, B};
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mix, mo, round3, BeatPlan, Carry, CompileError, Ctx};
use crate::easing::Easing;
use crate::intent::{CollectionItem, ObjectAtom, Subject};
use crate::scene::{Color, Layer, LayerKind, Motion, MotionOp, SpringSpec, Stroke, TextAlign};
use crate::speech::RevealRole;

/// Stage z-index of every sphere layer (the FX director's depth sort orders
/// them by distance, so they all share one).
const SPHERE_Z_INDEX: i32 = 12;
const LABEL_Z_INDEX: i32 = 14;
/// Sphere radius and the front size of an item, as shares of the short side.
const RADIUS_SHARE: f32 = 0.32;
const FRONT_SHARE: f32 = 0.40;
/// An item at the front is enlarged by this much.
const FOCUS_SCALE: f32 = 1.12;
/// Halo dots: count and radius (× R; a little inside the items' sphere, so
/// the front dots sit behind the front item).
const HALO_DOTS: usize = 48;
const HALO_RADIUS: f32 = 0.94;
/// Golden angle (degrees) spacing the Fibonacci sphere's longitudes.
const GOLDEN_ANGLE: f64 = 137.507_764_050_037_85;
/// Latitude of the items zig-zags within ± this (degrees).
const LAT_MAX: f32 = 25.0;
/// The intro spins in from this much further round in yaw / lower in pitch.
const INTRO_SPIN: f64 = 140.0;
const INTRO_LIFT: f64 = 15.0;
const INTRO_SECS: f64 = 1.2;
const TURN_SECS: f64 = 0.8;

const POP: SpringSpec = SpringSpec {
    stiffness: 260.0,
    damping: 18.0,
    mass: 1.0,
};

/// One rotation of the (rigid) sphere: identical for every layer on it.
#[derive(Debug, Clone, Copy)]
struct Turn {
    start: f64,
    duration: f64,
    easing: Easing,
    from: [f32; 2],
    to: [f32; 2],
}

/// The rotation schedule of the beat.
struct Schedule {
    /// The intro spin, then one turn per item after the first.
    turns: Vec<Turn>,
    /// When item `k` has arrived at the front.
    arrive: Vec<f64>,
    /// When item `k` starts to leave (`None` for the last item).
    leave: Vec<Option<f64>>,
    /// Duration of one turn between items.
    turn_secs: f64,
    /// Scene-local time the last item holds until.
    end: f64,
}

fn r3(v: f64) -> f32 {
    ((v * 1000.0).round() / 1000.0) as f32
}

/// `d` normalised to (-180, 180].
fn wrap180(d: f64) -> f64 {
    let mut d = d % 360.0;
    if d > 180.0 {
        d -= 360.0;
    } else if d <= -180.0 {
        d += 360.0;
    }
    d
}

/// Longitude / latitude (degrees) of item `k` of `n`: evenly spaced round the
/// sphere with a seeded zig-zag in latitude.
fn item_points(seed: u64, n: usize) -> Vec<[f32; 2]> {
    let flip = mix(seed, 0x5F10) & 1 == 1;
    (0..n)
        .map(|k| {
            let lon = k as f64 * 360.0 / n as f64;
            let r = (mix(seed, 0x5F20 + k as u64) % 10_000) as f32 / 10_000.0;
            let mag = LAT_MAX * (0.4 + 0.6 * r);
            let up = (k % 2 == 0) != flip;
            [r3(lon), r3(f64::from(if up { mag } else { -mag }))]
        })
        .collect()
}

/// The rotation schedule: angles `[-lon, -lat]` bring an item to the front.
fn schedule(plan: &BeatPlan, enter: f64, points: &[[f32; 2]]) -> Schedule {
    let n = points.len().max(1);
    let life = plan.life;
    let front = |p: [f32; 2]| [-f64::from(p[0]), -f64::from(p[1])];
    let dwell = (life.anticipate - life.read).max(0.0) / n as f64;
    let start = |k: usize| round3(life.read + dwell * k as f64);
    let turn_secs = round3(TURN_SECS.min(0.5 * dwell));
    let first = points.first().copied().unwrap_or([0.0, 0.0]);
    let mut cur = front(first);
    let mut turns = Vec::new();

    // Intro: spin in during ENTER, landing on item 0.
    let next_start = if n > 1 { start(1) } else { life.anticipate };
    let intro_end = (enter + INTRO_SECS).min(next_start - 0.05);
    let arrive0 = if intro_end > enter + 0.2 {
        turns.push(Turn {
            start: round3(enter),
            duration: round3(intro_end - enter),
            easing: Easing::OutCubic,
            from: [r3(cur[0] - INTRO_SPIN), r3(cur[1] + INTRO_LIFT)],
            to: [r3(cur[0]), r3(cur[1])],
        });
        round3(intro_end)
    } else {
        round3(enter)
    };

    let mut arrive = vec![arrive0];
    let mut leave = Vec::new();
    for (k, point) in points.iter().enumerate().skip(1) {
        let target = front(*point);
        // Short way round in yaw; the pitch simply follows the item's latitude.
        let to = [cur[0] + wrap180(target[0] - cur[0]), target[1]];
        turns.push(Turn {
            start: start(k),
            duration: turn_secs,
            easing: Easing::InOutCubic,
            from: [r3(cur[0]), r3(cur[1])],
            to: [r3(to[0]), r3(to[1])],
        });
        cur = to;
        arrive.push(round3(start(k) + turn_secs));
        leave.push(Some(start(k)));
    }
    leave.push(None);
    Schedule {
        turns,
        arrive,
        leave,
        turn_secs,
        end: round3(life.anticipate),
    }
}

/// Where the sphere sits on the canvas and how big it is.
#[derive(Debug, Clone, Copy)]
struct Sphere {
    cx: f32,
    cy: f32,
    r: f32,
    /// Side of an item at the front (before the focus enlargement).
    front: f32,
}

/// The sphere's centre sits below the title, a little below the middle of
/// the free band; everything stays above the bottom limit of the safe area
/// (which already excludes the caption lane). In a band that is too short the
/// whole sphere shrinks.
fn layout(ctx: &Ctx, title_bottom: f32, reserve: f32) -> Sphere {
    let (w, h) = (ctx.w, ctx.h);
    let short = w.min(h);
    let safe = ctx.frame.safe;
    let top = title_bottom + 0.03 * h;
    // (0.23) `reserve`: the band the total's card holds at the bottom.
    let bottom = safe.y + safe.h + 0.04 * short - reserve;
    // Vertical reach of the sphere (items on its upper / lower half) at scale 1.
    let reach = 0.77 * RADIUS_SHARE + 0.35 * FRONT_SHARE;
    let fit = ((bottom - top) / (2.0 * reach * short)).clamp(0.6, 1.0);
    let (r, front) = (RADIUS_SHARE * short * fit, FRONT_SHARE * short * fit);
    let up = reach * short * fit;
    // Below the front item: its enlarged half height, the gap and the label.
    let label = 0.5 * FOCUS_SCALE * front + 0.085 * short;
    let down = up.max(label);
    let (lo, hi) = (top + up, bottom - down);
    let cy = if hi >= lo {
        lo + 0.3 * (hi - lo)
    } else {
        0.5 * (lo + hi)
    };
    Sphere {
        cx: w / 2.0,
        cy,
        r,
        front,
    }
}

/// "brain_circuit" -> "Brain circuit".
pub(super) fn humanise(name: &str) -> String {
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

/// The picture of an object item that is not a root-library file: the best
/// catalog match among the look's asset families, delivered like the planner's
/// library deliveries (the request id is the item's).
fn catalog_entry(ctx: &Ctx, o: &ObjectAtom, beat: usize, k: usize) -> Option<ManifestEntry> {
    let words = request_words(&[
        o.asset.as_str(),
        o.value.as_deref().unwrap_or_default(),
        o.meaning.as_deref().unwrap_or_default(),
    ]);
    let m = ctx.library.catalog_match(&words, MatchKind::Object)?;
    ctx.library
        .catalog_delivery(&format!("{}/{}", m.family, m.id), &item_request(beat, k))
}

/// Request id of item `k` of beat `beat` (0-based): `beat_N.item_k`.
fn item_request(beat: usize, k: usize) -> String {
    format!("beat_{}.item_{k}", beat + 1)
}

/// (0.22) The picture of object item `k` of a collection, centred in `slot`
/// (anchor 0.5 / 0.5, at most `slot.w × slot.h`): a delivered
/// `beat_N.item_k` image, else the root-library file, else the best catalog
/// match among the look's families. `None` when it has no picture anywhere
/// (the caller shows the item as text; no error, so the item keeps its value).
/// (0.22) True when item `k`'s picture comes from the root library (no
/// delivered image): it has no analysis, so its colour is unknown.
pub(super) fn library_picture(ctx: &Ctx, beat: usize, k: usize, o: &ObjectAtom) -> bool {
    ctx.library.find_object(&o.asset).is_some()
        && ctx.manifest.get(&item_request(beat, k)).is_none()
}

pub(super) fn item_picture(
    ctx: &mut Ctx,
    b: &B,
    id: String,
    o: &ObjectAtom,
    k: usize,
    slot: Slot,
    z: i32,
) -> Result<Option<Layer>, CompileError> {
    let delivered = ctx.manifest.get(&item_request(b.plan.index, k)).cloned();
    let mut layer = if library_picture(ctx, b.plan.index, k, o) {
        let ink = ctx.palette.ink;
        recipes::subject_layer(ctx, b, id, &Subject::Object(o.clone()), slot, ink)?
    } else if let Some(entry) = delivered.or_else(|| catalog_entry(ctx, o, b.plan.index, k)) {
        let rect = (
            slot.cx - slot.w / 2.0,
            slot.cy - slot.h / 2.0,
            slot.w,
            slot.h,
        );
        plate::image_layer(ctx, b, id, &entry, AssetRole::HeroObject, rect, z)
    } else {
        return Ok(None);
    };
    layer.x = slot.cx;
    layer.y = slot.cy;
    layer.anchor_x = 0.5;
    layer.anchor_y = 0.5;
    layer.z_index = z;
    Ok(Some(layer))
}

/// (0.22) An object's figure as a chip on the lower edge of its picture
/// (`item.{k}.value`): the figure in the number voice on the card stock with
/// an accent keyline, like the number cards. Coordinates are the item
/// group's (`side × side`).
fn value_chip(ctx: &Ctx, id: &str, side: f32, value: &str) -> Layer {
    let u = ctx.u;
    let block = ctx
        .ts
        .fit_line(Voice::HERO_NUMBER, value, 0.62 * side, 0.15 * side);
    let pad = (0.035 * side).max(6.0 * u);
    let (tw, th) = (block.width() * 1.02 + 2.0, block.height());
    let (cw, ch) = (tw + 2.0 * pad, th + pad);
    let mut t = recipes::with_ink(
        ctx,
        text_layer(
            format!("{id}.value.text"),
            &block,
            ctx.palette.ink,
            TextAlign::Center,
        ),
        &block,
    );
    t.x = pad;
    t.y = (ch - th) / 2.0;
    t.z_index = 1;
    let chip = base_layer(
        format!("{id}.value.chip"),
        (0.0, 0.0, cw, ch),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: ch / 2.0,
            stroke: Some(Stroke {
                color: ctx.palette.accent,
                width: 3.0 * u,
            }),
        },
        0,
    );
    let mut group = base_layer(
        format!("{id}.value"),
        (side / 2.0, side - ch / 2.0, cw, ch),
        LayerKind::Group {
            children: vec![chip, t],
        },
        2,
    );
    group.anchor_x = 0.5;
    group.anchor_y = 0.5;
    group
}

/// A phrase / number / missing-picture item: big type on a rounded card.
fn card(ctx: &Ctx, id: String, sphere: &Sphere, text: &str, number: bool) -> Layer {
    let u = ctx.u;
    let (cw, ch) = (sphere.front, 0.72 * sphere.front);
    // The text stays well inside the card: a card on the sphere's flank sits
    // near the edge of the canvas (its text must stay in the safe area).
    let inner = 0.64 * cw;
    let block = if number {
        ctx.ts.fit_line(Voice::HERO_NUMBER, text, inner, 0.5 * ch)
    } else {
        ctx.ts
            .fit_block(Voice::HEADLINE, text, inner, 0.62 * ch, 0.22 * cw, 3)
    };
    let mut label = text_layer(
        format!("{id}.text"),
        &block,
        ctx.palette.ink,
        TextAlign::Center,
    );
    label.x = (cw - label.width) / 2.0;
    label.y = (ch - label.height) / 2.0;
    label.z_index = 1;
    let label = recipes::with_ink(ctx, label, &block);
    let bg = base_layer(
        format!("{id}.card"),
        (0.0, 0.0, cw, ch),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: 0.09 * cw,
            stroke: Some(Stroke {
                color: ctx.palette.accent,
                width: 3.0 * u,
            }),
        },
        0,
    );
    let mut group = base_layer(
        id,
        (sphere.cx, sphere.cy, cw, ch),
        LayerKind::Group {
            children: vec![bg, label],
        },
        SPHERE_Z_INDEX,
    );
    group.anchor_x = 0.5;
    group.anchor_y = 0.5;
    group
}

/// One item of the collection as a layer at the sphere centre, with the text
/// of its label (`None` when the layer already shows its text).
fn item_layer(
    ctx: &mut Ctx,
    b: &B,
    k: usize,
    item: &CollectionItem,
    sphere: &Sphere,
) -> Result<(Layer, Option<String>), CompileError> {
    let id = b.id(&format!("item.{k}"));
    let side = sphere.front;
    match item {
        CollectionItem::Object(o) => {
            let label = o
                .meaning
                .clone()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| humanise(&o.asset));
            // (0.22) The figure the object carries, shown like a number card's.
            let value = o.value.as_deref().map(str::trim).filter(|v| !v.is_empty());
            let slot = Slot {
                cx: sphere.cx,
                cy: sphere.cy,
                w: side,
                h: side,
            };
            let Some(picture) = item_picture(ctx, b, id.clone(), o, k, slot, SPHERE_Z_INDEX)?
            else {
                // No picture anywhere: the object is shown as text (its
                // figure on the card and its name as the label, like a
                // number item).
                return Ok(match value {
                    Some(v) => (card(ctx, id, sphere, v, true), Some(label)),
                    None => (card(ctx, id, sphere, &label, false), None),
                });
            };
            let Some(value) = value else {
                return Ok((picture, Some(label)));
            };
            // Picture and figure ride the sphere as one group.
            let mut picture = picture;
            picture.id = format!("{id}.picture");
            picture.x = side / 2.0;
            picture.y = side / 2.0;
            picture.z_index = 0;
            let chip = value_chip(ctx, &id, side, value);
            let mut group = base_layer(
                id,
                (sphere.cx, sphere.cy, side, side),
                LayerKind::Group {
                    children: vec![picture, chip],
                },
                SPHERE_Z_INDEX,
            );
            group.anchor_x = 0.5;
            group.anchor_y = 0.5;
            Ok((group, Some(label)))
        }
        CollectionItem::Phrase(a) | CollectionItem::Number(a) => {
            let number = matches!(item, CollectionItem::Number(_));
            let text = a
                .value
                .as_deref()
                .or(a.meaning.as_deref())
                .unwrap_or_default();
            // The card shows the value; the meaning (if any) becomes the label.
            let label = a
                .value
                .as_ref()
                .and_then(|_| a.meaning.clone())
                .filter(|m| !m.trim().is_empty());
            Ok((card(ctx, id, sphere, text, number), label))
        }
    }
}

/// A label pill under the sphere's front (z = 0).
fn label_layer(ctx: &Ctx, id: String, sphere: &Sphere, text: &str) -> Layer {
    let short = ctx.w.min(ctx.h);
    let pad = 0.03 * short;
    let block = ctx
        .ts
        .fit_line(Voice::LABEL, text, 0.78 * short - 2.0 * pad, 0.04 * short);
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
    let pill = base_layer(
        format!("{id}.pill"),
        (0.0, 0.0, pw, ph),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: ph / 2.0,
            stroke: None,
        },
        0,
    );
    let y = sphere.cy + 0.5 * FOCUS_SCALE * sphere.front + 0.025 * short + ph / 2.0;
    let mut group = base_layer(
        id,
        (sphere.cx, y, pw, ph),
        LayerKind::Group {
            children: vec![pill, t],
        },
        LABEL_Z_INDEX,
    );
    group.anchor_x = 0.5;
    group.anchor_y = 0.5;
    group.z = Some(0.0);
    group
}

fn revolve(target: &str, radius: f32, at: [f32; 2], turn: &Turn) -> Motion {
    Motion {
        id: None,
        target: target.to_string(),
        start: turn.start,
        duration: turn.duration,
        easing: turn.easing,
        spring: None,
        op: MotionOp::Revolve {
            radius,
            at,
            from: turn.from,
            to: turn.to,
        },
    }
}

/// Fibonacci sphere: the `[lon, lat]` (degrees) of dot `j` of `n`.
fn halo_point(j: usize, n: usize) -> [f32; 2] {
    let y = 1.0 - 2.0 * (j as f64 + 0.5) / n as f64;
    let lat = y.asin().to_degrees();
    let lon = wrap180(j as f64 * GOLDEN_ANGLE);
    [r3(lon), r3(lat)]
}

pub(crate) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    _carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let Subject::Collection(collection) = &b.beat.primary else {
        return Err(CompileError::Beat {
            beat: b.plan.index + 1,
            message: "a sphere gallery needs a collection".into(),
        });
    };
    let items: Vec<CollectionItem> = collection.items.clone();
    let n = items.len();
    let plan = b.plan;
    let (t, seed, u) = (plan.enter_at(), plan.seed, ctx.u);
    let fields = cinematic3d::field_colors(ctx);

    cinematic3d::glow_discs(ctx, b, &fields);
    cinematic3d::environment_plate(ctx, b);
    // The camera pushes in over the beat (the title, nearer than the focus
    // plane, grows ~15 %): keep it clear of the canvas edges.
    let title_bottom = cinematic3d::title(ctx, b, t, 0.8 * ctx.w);
    // (0.23) The consequence the items add up to (the secondary: a number or
    // a pictured thing with a value) is a card under the sphere, which gives
    // it the band it needs.
    let total = b.beat.secondary.clone().filter(|s| s.value().is_some());
    let total_card = total.as_ref().and_then(|s| {
        cinematic3d::figure_card(ctx, b.id("total"), s, 0.7 * ctx.w, 0.09 * ctx.w.min(ctx.h))
    });
    let reserve = total_card
        .as_ref()
        .map_or(0.0, |c| c.height + 0.045 * ctx.h);
    let sphere = layout(ctx, title_bottom, reserve);
    let points = item_points(seed, n);
    let sched = schedule(plan, t, &points);

    // Items, labels and their scale / fade chains.
    let mut riders: Vec<(String, f32, [f32; 2])> = Vec::new();
    let mut labels: Vec<(usize, String)> = Vec::new();
    for (k, item) in items.iter().enumerate() {
        let (mut layer, label) = item_layer(ctx, b, k, item, &sphere)?;
        layer.z = Some(sphere.r);
        let id = layer.id.clone();
        b.motions.push(mo::fade(
            &id,
            t + 0.04 * k as f64,
            0.5,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        // Arrival: enlarge while in front; leaving: back to 1.
        let hold_end = sched.leave[k].unwrap_or(sched.end);
        let hold = hold_end - sched.arrive[k];
        if hold > 0.1 {
            let up = 0.5f64.min(0.9 * hold);
            b.motions.push(sprung(
                mo::scale(&id, sched.arrive[k], up, 1.0, FOCUS_SCALE, Easing::OutCubic),
                POP,
            ));
            if let Some(leave) = sched.leave[k] {
                let down = 0.4f64.min(0.8 * sched.turn_secs);
                b.motions.push(mo::scale(
                    &id,
                    leave,
                    down,
                    FOCUS_SCALE,
                    1.0,
                    Easing::InOutCubic,
                ));
            }
        }
        riders.push((id, sphere.r, points[k]));
        if let Some(label) = label {
            labels.push((k, label));
        }
        b.stage.push(layer);
    }

    // Halo: dots on a Fibonacci sphere riding the same rotation.
    let rnd = |j: u64| (mix(seed, 0x5F90 + j) % 10_000) as f32 / 10_000.0;
    for j in 0..HALO_DOTS {
        let id = b.id(&format!("dust.{j}"));
        let d = (12.0 + 22.0 * rnd(j as u64)) * u;
        let color: Color = if j % 3 == 0 {
            ctx.palette.accent.with_alpha(0xE0)
        } else {
            ctx.palette.ink.with_alpha(0xC8)
        };
        let dot = disc(
            id.clone(),
            (sphere.cx, sphere.cy),
            d,
            color,
            SPHERE_Z_INDEX,
            sphere.r,
        );
        b.motions.push(mo::fade(
            &id,
            t + 0.012 * j as f64,
            0.6,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        riders.push((id, HALO_RADIUS * sphere.r, halo_point(j, HALO_DOTS)));
        b.stage.push(dot);
    }

    // Labels: in while their item is in front, out before the next turn.
    for (k, text) in labels {
        let id = b.id(&format!("sphere_label.{k}"));
        let hold_end = sched.leave[k].unwrap_or(sched.end);
        let hold = hold_end - sched.arrive[k];
        if hold <= 0.1 {
            continue;
        }
        let fade_in = 0.3f64.min(0.35 * hold);
        b.motions.push(mo::fade(
            &id,
            sched.arrive[k] + 0.03,
            fade_in,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        if let (Some(leave), true) = (sched.leave[k], hold >= 0.3) {
            let fade_out = 0.25f64.min(0.3 * hold);
            b.motions.push(mo::fade(
                &id,
                leave - fade_out - 0.02,
                fade_out,
                1.0,
                0.0,
                Easing::InCubic,
            ));
        }
        b.stage.push(label_layer(ctx, id, &sphere, &text));
    }

    // (0.23) The total comes up as the last item turns to the front (or when
    // the narrator says it), centred under the sphere and above the caption lane.
    if let (Some(mut card), Some(subject)) = (total_card, total.as_ref()) {
        let lane = ctx.frame.safe.y + ctx.frame.safe.h - 0.015 * ctx.h;
        card.x = ctx.w / 2.0;
        card.y = lane - card.height / 2.0;
        let id = card.id.clone();
        let last_turn = sched
            .leave
            .get(n.saturating_sub(2))
            .copied()
            .flatten()
            .unwrap_or(plan.life.read);
        let at = round3(last_turn.min(plan.life.anticipate - 1.2).max(t + 0.6));
        b.reveal("total", recipes::subject_words(subject), RevealRole::Value);
        b.motions
            .push(mo::fade(&id, at, 0.4, 0.0, 1.0, Easing::OutCubic));
        b.motions.push(sprung(
            mo::scale(&id, at, 0.5, 0.85, 1.0, Easing::OutCubic),
            POP,
        ));
        b.stage.push(card);
    }

    // The rigid sphere: every rider gets the same chain of turns.
    for (id, radius, at) in &riders {
        for turn in &sched.turns {
            b.motions.push(revolve(id, *radius, *at, turn));
        }
    }

    cinematic3d::bokeh_discs(ctx, b, t, &fields);
    Ok(())
}
