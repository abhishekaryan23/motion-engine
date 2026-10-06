//! CinematicMultiplane: foreground / midground / background planes under a moving camera.
//!
//! Background (depth 0.35): a delivered environment plate (full bleed, veiled in
//! paper), the ghost word and a dark halftone field. Midground
//! (0.7): a tilted collage card, the secondary revealed at an EVOLVE event. Subject plane (1.0): headline and
//! primary subject. Foreground (1.45): an accent bar and a dark plate crossing
//! the frame edge. The per-scene camera (push + track) separates the planes.

use super::plate::{self, events};
use super::{Composition, Variant};
use crate::assets::AssetRole;
use crate::compiler::recipes::{self, plane, Entrance, Slot, B};
use crate::compiler::typeset::Voice;
use crate::compiler::{
    base_layer, mix, mo, rect_layer, texture_layer, Carry, CompileError, Ctx, Which,
};
use crate::easing::Easing;
use crate::intent::SubjectKind;
use crate::scene::MotionOp;
use crate::scene::{Direction, LayerKind, Material, Stroke, TextAlign, TextureSpec};

/// Bleed beyond the canvas on every side (camera push, drift and the stage's
/// anticipation pull-back never reveal an edge).
const ENV_BLEED: f32 = 0.06;
/// Background plate drift over READ..ANTICIPATE (× u); a gentle slide, not a Ken Burns.
const ENV_DRIFT: f32 = 24.0;
/// Paper veil alpha over the plate.
const ENV_VEIL_ALPHA: u8 = 0x99;

/// The environment image as a full-bleed background plane: covers the canvas
/// plus [`ENV_BLEED`] (Fit::Cover semantics via a box of the image aspect),
/// z 1-2 (under the halftone field), background depth, fades in from the
/// scene enter, drifts linearly through READ..ANTICIPATE only.
fn background_plate(ctx: &mut Ctx, b: &mut B, img_w: u32, img_h: u32, sign: f32, t: f64) {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let life = b.plan.life;
    let bg = plane(b.plan.lang.depth.background);
    let (cw, ch) = (w * (1.0 + 2.0 * ENV_BLEED), h * (1.0 + 2.0 * ENV_BLEED));
    let (bw, bh) = if img_w > 0 && img_h > 0 {
        let s = (cw / img_w as f32).max(ch / img_h as f32);
        (img_w as f32 * s, img_h as f32 * s)
    } else {
        (cw, ch)
    };
    let rect = ((w - bw) / 2.0, (h - bh) / 2.0, bw, bh);
    let fade_dur = 1.2_f64.min((b.plan.duration - t - 0.05).max(0.1));

    let mut plate = plate::image_plate(ctx, b, "env", AssetRole::Environment, rect, 1, t);
    if !plate.delivered {
        // A placeholder card is never a background plane.
        return;
    }
    let plate_id = plate.id.clone();
    // A full-bleed plane fades in; it is not revealed by a wipe.
    b.motions
        .retain(|m| !(m.target == plate_id && matches!(m.op, MotionOp::MaskReveal { .. })));
    b.motions.push(mo::fade(
        &plate_id,
        t,
        fade_dur,
        0.0,
        1.0,
        Easing::InOutCubic,
    ));
    for l in &mut plate.layers {
        l.depth = bg;
    }

    let veil_id = b.id("env.veil");
    let mut veil = rect_layer(
        veil_id.clone(),
        (-w * ENV_BLEED, -h * ENV_BLEED, cw, ch),
        ctx.palette.paper.with_alpha(ENV_VEIL_ALPHA),
        2,
    );
    veil.depth = bg;
    b.motions.push(mo::fade(
        &veil_id,
        t,
        fade_dur,
        0.0,
        1.0,
        Easing::InOutCubic,
    ));

    let span = life.anticipate - life.read;
    if span > 0.2 {
        let d = ENV_DRIFT * u * sign;
        for id in [&plate_id, &veil_id] {
            b.motions.push(mo::shift(
                id,
                life.read,
                span,
                [0.0, 0.0],
                [-d, 0.0],
                Easing::Linear,
            ));
        }
    }
    for l in plate.layers {
        b.push(l);
    }
    b.push(veil);
}

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let beat = b.beat;
    let t = plan.enter_at();
    let depth = plan.lang.depth;
    let (bg, mid, fg) = (
        plane(depth.background),
        plane(depth.midground),
        plane(depth.foreground),
    );
    let mirror = c.variant == Variant::Mirror;
    let sign = if mirror { -1.0 } else { 1.0 };

    // Background plane: ghost word (z 0) ...
    recipes::ghost(ctx, b, 0.60 * h);

    // Subject plane: headline.
    let top = 0.2 * h;
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        w - 2.0 * m,
        0.27 * h,
        176.0 * u,
        4,
    );
    let (head_h, lines_done) = recipes::headline_lines(
        ctx,
        b,
        "head",
        &head,
        top,
        TextAlign::Left,
        ctx.palette.ink,
        t + 0.1,
        true,
    );
    let head_h = head_h + recipes::deck(ctx, b, top + head_h, lines_done);

    let hero_top = top + head_h + 110.0 * u;
    let hero_h = 0.2 * h;
    let slot = Slot {
        cx: w / 2.0 + if mirror { 0.0 } else { 0.02 * w },
        cy: hero_top + hero_h / 2.0,
        w: w - 2.0 * m - 40.0 * u,
        h: hero_h,
    };
    let card_size = (w - 2.0 * m + 30.0 * u, hero_h + 70.0 * u);

    // A delivered environment plate is the full-bleed BACKGROUND plane (below
    // the halftone field), softly veiled in paper so the type stays readable.
    if let Some(entry) = plate::image(ctx, plan.index, AssetRole::Environment) {
        background_plate(ctx, b, entry.width, entry.height, sign, t);
    }

    // ... and a dark halftone field behind the card.
    let field_id = b.id("field");
    let mut field = texture_layer(
        &field_id,
        (-0.12 * w, slot.cy - hero_h * 0.95, 1.24 * w, hero_h * 1.9),
        TextureSpec {
            material: Material::Halftone,
            seed: mix(plan.seed, 0xF1E1D),
            color: ctx.palette.ink,
            intensity: 0.22,
            scale: 22.0 * u,
            animated: false,
        },
        6,
    );
    field.depth = bg;
    b.motions.push(mo::mask(
        &field_id,
        t,
        1.1,
        Direction::Right,
        Easing::OutCubic,
    ));
    b.push(field);

    // Midground plane: tilted collage card, or a delivered environment plate on it.
    let mut layers = Vec::new();
    recipes::collage_card(
        ctx,
        b,
        "hero",
        (slot.cx, slot.cy),
        card_size,
        -2.2 * sign,
        (-sign, -1.0),
        lines_done - 0.2,
        &mut layers,
    );
    for mut l in layers {
        l.depth = mid;
        b.push(l);
    }

    // Primary subject on the card (subject plane).
    if let Some(mut l) = recipes::place_subject(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        slot,
        (0.0, 0.0),
        ctx.palette.ink,
        lines_done,
        Entrance::Rise,
        None,
        None,
        "hero",
    )? {
        l.z_index = 22;
        b.push(l);
    }

    // EVOLVE: midground reveals the secondary.
    if let Some(sec) = &beat.secondary {
        let tail = if sec.kind() == SubjectKind::Object {
            plate::subject_tail(b, sec)
        } else {
            plate::serif_tail(b)
        };
        let at = events(b, 1, tail)[0];
        let cursor = hero_top + hero_h + 110.0 * u;
        let n0 = b.stage.len();
        if sec.kind() == SubjectKind::Object {
            let side = (h - cursor - 0.1 * h).min(0.3 * w);
            let s = Slot {
                cx: if mirror {
                    m + side / 2.0
                } else {
                    w - m - side / 2.0
                },
                cy: cursor + side / 2.0,
                w: side,
                h: side,
            };
            if let Some(mut l) = recipes::place_subject(
                ctx,
                b,
                carries,
                Which::Secondary,
                sec,
                s,
                (0.0, 0.0),
                ctx.palette.ink,
                at,
                Entrance::Rise,
                None,
                None,
                "support",
            )? {
                l.z_index = 24;
                b.push(l);
            }
        } else {
            let text = sec.display_text().unwrap_or("");
            plate::serif_note(
                ctx,
                b,
                "serif",
                text,
                cursor,
                if mirror {
                    TextAlign::Right
                } else {
                    TextAlign::Left
                },
                w - 2.0 * m,
                0.1 * h,
                84.0 * u,
                at,
            );
        }
        for l in &mut b.stage[n0..] {
            l.depth = mid;
        }
    }

    // Foreground plane: an accent bar and a dark plate crossing the frame edge.
    let bar_id = b.id("fg_bar");
    let bar_w = 0.5 * w;
    let mut bar = rect_layer(
        bar_id.clone(),
        (
            if mirror { w - 0.44 * w } else { -0.06 * w },
            0.865 * h,
            bar_w,
            22.0 * u,
        ),
        ctx.palette.accent,
        26,
    );
    bar.depth = fg;
    b.motions.push(mo::mask(
        &bar_id,
        t + 0.5,
        0.9,
        if mirror {
            Direction::Left
        } else {
            Direction::Right
        },
        Easing::OutQuint,
    ));
    b.push(bar);

    let occ_id = b.id("fg_plate");
    let occ_w = 0.3 * w;
    let occ_h = 0.07 * h;
    let mut occ = base_layer(
        occ_id.clone(),
        (
            if mirror { -0.02 * w } else { w - 0.14 * w },
            0.8 * h,
            occ_w,
            occ_h,
        ),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.ink,
            radius: 12.0 * u,
            stroke: Some(Stroke {
                color: ctx.palette.card.with_alpha(0x40),
                width: 2.0 * u,
            }),
        },
        25,
    );
    occ.rotation_degrees = 4.0 * sign;
    occ.depth = fg;
    b.motions.push(mo::mask(
        &occ_id,
        t + 0.65,
        0.8,
        Direction::Up,
        Easing::OutQuint,
    ));
    b.push(occ);

    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}
