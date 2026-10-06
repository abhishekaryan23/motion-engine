//! HeroObject: one semantic object dominates and persists.
//!
//! The object fills about 55 % of the short side on a soft plate (card disc,
//! offset halftone, contact shadow on the midground plane), the headline sits
//! smaller above it. The object arrives in ENTER and drifts very slowly across
//! READ + EVOLVE; a stamped figure and the secondary arrive at EVOLVE events.
//! A delivered `hero_object` image replaces the library object (0.5), sized by
//! its analyzed subject bounds. (0.22) A secondary object's value is stamped
//! too, beside the secondary (`support_stamp`, a Value anchor).

use super::placement::{self, Rect, SubjectFacts};
use super::plate::{self, events};
use super::{Composition, Variant};
use crate::assets::AssetRole;
use crate::compiler::direction::{self, AccentMark, EntranceFamily, Placed, Reveal};
use crate::compiler::recipes::{self, plane, Entrance, Slot, B};
use crate::compiler::taste_rules::FRAME_PAD_MAX;
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{
    base_layer, mix, mo, subject_rules, texture_layer, Carry, CompileError, Ctx, Which,
};
use crate::easing::Easing;
use crate::intent::{Purpose, SubjectKind};
use crate::scene::{Direction, LayerKind, Material, Stroke, TextAlign, TextureSpec};

/// Pad (fraction of the image size, per side) of the card that hugs a
/// delivered opaque hero image.
const HERO_CARD_PAD: f32 = 0.05;

/// Height the stamp and the secondary need below the object in the stacked
/// layout (mirrors the placement further down).
fn below_object_height(ctx: &Ctx, b: &B, h: f32, u: f32, note_w: f32) -> f32 {
    let beat = b.beat;
    let mut need = 0.0;
    if let Some(text) = beat.primary.value() {
        need += ctx
            .ts
            .fit_line(Voice::LABEL, text, note_w, 44.0 * u)
            .height()
            + 36.0 * u;
    }
    if let Some(sec) = &beat.secondary {
        need += if sec.kind() == SubjectKind::Object {
            0.16 * h
        } else {
            let text = sec.display_text().unwrap_or("");
            ctx.ts
                .fit_block(Voice::SERIF, text, note_w, 0.09 * h, 84.0 * u, 2)
                .height()
                + 23.0 * u
        };
    }
    need
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
    let life = plan.life;
    let t = plan.enter_at();
    // (0.23 C2b) The beat's planned direction; the identity is the old beat.
    let bp = ctx.params_for(plan.index);
    // Layers whose entrance is steered at the end: the ones `place_subject` and
    // `image_plate` placed.
    let mut placed: Vec<Placed> = Vec::new();
    // How a stamped figure lands: the preset's curve, a hard strike under `Stamp`.
    let strike = if bp.accent == AccentMark::Stamp {
        Easing::ImpactSpring
    } else {
        plan.lang.preset.easing
    };
    let mirror = c.variant == Variant::Mirror;
    let land = placement::side_by_side(&ctx.frame);
    let sign = if mirror { -1.0 } else { 1.0 };
    let mid = plane(plan.lang.depth.midground);

    recipes::ghost(ctx, b, 0.6 * h);

    // Smaller headline above the object.
    let align = if mirror {
        TextAlign::Right
    } else {
        TextAlign::Left
    };
    let (text_w, head_top, head_h_max) = if land {
        (0.5 * w - m, 0.2 * h, 0.4 * h)
    } else {
        (w - 2.0 * m, 0.165 * h, 0.15 * h)
    };
    let head = ctx.ts.fit_block(
        Voice::HEADLINE,
        &beat.statement,
        text_w,
        head_h_max,
        104.0 * u,
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

    // The object: ~55 % of the short side.
    let mut side = 0.55 * w.min(h);
    let (cx, mut cy) = if land {
        (if mirror { 0.27 * w } else { 0.73 * w }, 0.55 * h)
    } else {
        (
            w / 2.0 + sign * 0.07 * w,
            h * ctx.frame.blend(0.57, 0.6, 0.6),
        )
    };
    if !land {
        // Stacked layout: the stamp and the secondary below the object must
        // still end above the safe bottom edge. Shrink the object (down to
        // 70 %) and, if still short, lift it. (0.22) On the legacy canvases
        // too: a square beat with a figure and a secondary picture ran off
        // the canvas; beats that fit are unchanged. The lift never reaches
        // into the headline: the object shrinks further instead.
        let below = below_object_height(ctx, b, h, u, w - 2.0 * m);
        let object_bottom = (h - m) - below - 0.05 * h;
        let top = cy - side / 2.0;
        if cy + side / 2.0 > object_bottom {
            let ceiling = (head_top + head.height() + 0.02 * h).min(top);
            side = (object_bottom - top)
                .max(0.7 * side)
                .min(object_bottom - ceiling)
                .max(0.2 * w.min(h));
            cy = object_bottom - side / 2.0;
        }
    }

    // Plate behind (midground): card disc, offset halftone, contact shadow.
    // A delivered transparent cutout IS the object: it gets the halftone
    // accent but no card disc and no generic shadow pills (those back
    // procedural pictograms; under a photo they read as a white circle and a
    // floating bar).
    let hero_entry = plate::image(ctx, plan.index, AssetRole::HeroObject);
    let cutout = hero_entry.is_some_and(|e| e.alpha);
    // A delivered object image is sized by what it depicts (its analyzed
    // subject bounds fill the object's slot, not its transparent margins).
    let hero_fit = hero_entry.map(|e| {
        let facts = SubjectFacts::from_entry(e);
        let target = Rect::new(cx - side / 2.0, cy - side / 2.0, side, side);
        let img = placement::subject_fit(&facts, target);
        (e, facts, img)
    });
    let disc = side * 1.12;
    // (0.10 Q) The disc is a circle: it hugs a near-square object only. Behind
    // a delivered opaque image that it would exceed by more than
    // FRAME_PAD_MAX per side, the card is the image's own bounds plus a small
    // pad (taste_rules; subject_rules::shrink_wrap).
    let wrapped = hero_fit.as_ref().and_then(|(e, facts, img)| {
        if e.alpha {
            return None;
        }
        let sub = placement::to_canvas(img, facts.subject);
        let pad = ((disc - sub.w) / (2.0 * sub.w)).max((disc - sub.h) / (2.0 * sub.h));
        (pad > FRAME_PAD_MAX)
            .then(|| subject_rules::shrink_wrap((sub.x, sub.y, sub.w, sub.h), HERO_CARD_PAD))
    });
    let disc_id = b.id("plate");
    let (card_box, card_radius) = match wrapped {
        Some((wx, wy, ww, wh)) => ((wx + ww / 2.0, wy + wh / 2.0, ww, wh), 0.06 * ww.min(wh)),
        None => ((cx, cy, disc, disc), disc / 2.0),
    };
    let mut disc_layer = base_layer(
        disc_id.clone(),
        card_box,
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: card_radius,
            stroke: Some(Stroke {
                color: ctx.palette.ink.with_alpha(0x22),
                width: 2.0 * u,
            }),
        },
        9,
    );
    disc_layer.anchor_x = 0.5;
    disc_layer.anchor_y = 0.5;
    disc_layer.depth = mid;
    if !cutout {
        b.motions.push(mo::fade(
            &disc_id,
            t + 0.05,
            0.6,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        // The plate pops in; `amplitude` scales the pop, ScalePop makes it a
        // bigger one.
        let from = if bp.entrance == EntranceFamily::ScalePop {
            0.74
        } else {
            0.86
        };
        b.motions.push(mo::scale(
            &disc_id,
            t + 0.05,
            1.1,
            direction::scale_amplitude(&bp, from),
            1.0,
            direction::entrance_easing(&bp, Easing::OutQuint),
        ));
    }

    let ht_id = b.id("plate.halftone");
    let mut ht = texture_layer(
        &ht_id,
        (
            cx - side * 0.52 - sign * 0.2 * side,
            cy - side * 0.45 + 0.17 * side,
            side * 1.05,
            side * 0.9,
        ),
        TextureSpec {
            material: Material::Halftone,
            seed: mix(plan.seed, 0x4E90),
            color: ctx.palette.accent,
            intensity: 0.95,
            scale: 15.0 * u,
            animated: false,
        },
        8,
    );
    ht.depth = mid;
    // The halftone patch is a decoration: `None` leaves it out, any other
    // accent keeps it; it wipes in from the planned side.
    if bp.accent != AccentMark::None {
        b.motions.push(recipes::edge_entrance(
            &bp,
            &ht_id,
            t + 0.1,
            0.9,
            Reveal::Mask,
            Direction::Down,
            Easing::OutCubic,
        ));
        b.push(ht);
    }
    if !cutout {
        b.push(disc_layer);
    }

    let pills: &[(f32, u8)] = if cutout {
        &[]
    } else {
        &[(0.82, 0x14), (0.56, 0x26)]
    };
    // The contact shadow sits under the object (under the card's bottom edge
    // when the card hugs a delivered image).
    let (pill_w, pill_bottom) = match wrapped {
        Some((_, wy, ww, wh)) => (ww.min(side * 1.2), wy + wh),
        None => (side, cy + side * 0.5),
    };
    for (i, &(frac, alpha)) in pills.iter().enumerate() {
        let sw = pill_w * frac;
        let sh = side * 0.07;
        let sid = b.id(&format!("shadow.{i}"));
        let mut shadow = base_layer(
            sid.clone(),
            (cx - sw / 2.0, pill_bottom - sh / 2.0, sw, sh),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.ink.with_alpha(alpha),
                radius: sh / 2.0,
                stroke: None,
            },
            10,
        );
        shadow.depth = mid;
        b.motions
            .push(mo::fade(&sid, t + 0.25, 0.6, 0.0, 1.0, Easing::OutCubic));
        b.push(shadow);
    }

    let span = life.anticipate - life.read;
    // (0.23 W8a) A picture the story carries across beats is a shared element:
    // carried in from the previous beat it glides into the slot, carried on it
    // starts here and stays on screen into the next beat.
    let carried = hero_fit.as_ref().and_then(|(entry, _, img)| {
        plate::place_carried(
            ctx,
            b,
            carries,
            Which::Primary,
            entry,
            AssetRole::HeroObject,
            (img.x, img.y, img.w, img.h),
            22,
            None,
            0.0,
            t + 0.25,
            plate::Arrive {
                offset: (0.0, 40.0 * u),
                duration: 0.7,
            },
        )
    });
    if let Some(c) = carried {
        // Re-read at EVOLVE, like a carried subject (it ends neutral).
        if c.carried_in && matches!(beat.purpose, Purpose::Emphasize | Purpose::Reveal) {
            recipes::shared_pulse(ctx, b, &c.id, life.evolve);
        }
    } else if let Some((_, _, img)) = hero_fit {
        let hero = plate::image_plate(
            ctx,
            b,
            "hero",
            AssetRole::HeroObject,
            (img.x, img.y, img.w, img.h),
            22,
            t + 0.25,
        );
        // Same READ drift as a library object.
        if span > 0.2 {
            b.motions.push(mo::scale(
                &hero.id,
                life.read,
                span,
                1.0,
                direction::scale_amplitude(&bp, 1.03),
                Easing::InOutCubic,
            ));
        }
        placed.push(Placed {
            id: hero.id.clone(),
            glyphs: None,
        });
        for l in hero.layers {
            b.push(l);
        }
    } else {
        let slot = Slot {
            cx,
            cy,
            w: side,
            h: side,
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
            t + 0.25,
            Entrance::Rise,
            None,
            None,
            "hero",
        )? {
            l.z_index = 22;
            // A local hero drifts very slowly through READ + EVOLVE (a carried
            // one is a shared element: its track owns the transform).
            if span > 0.2 {
                b.motions.push(mo::scale(
                    &l.id,
                    life.read,
                    span,
                    1.0,
                    direction::scale_amplitude(&bp, 1.03),
                    Easing::InOutCubic,
                ));
            }
            placed.push(Placed::of(&l));
            b.push(l);
        }
    }

    // EVOLVE: the stamped figure, then the secondary.
    let sec = beat.secondary.as_ref();
    let stamp = beat.primary.value();
    let tail = (plan.lang.preset.duration * 0.9 + 0.05).max(sec.map_or(0.0, |s| {
        if s.kind() == SubjectKind::Object {
            plate::subject_tail(b, s)
        } else {
            plate::serif_tail(b)
        }
    }));
    let ev = events(
        b,
        usize::from(stamp.is_some()) + usize::from(sec.is_some()),
        tail,
    );
    let mut ev = ev.into_iter();
    let (note_w, mut cursor) = if land {
        (0.5 * w - m, 0.64 * h)
    } else {
        (w - 2.0 * m, cy + side / 2.0 + 0.05 * h)
    };
    if let (Some(text), Some(at)) = (stamp, ev.next()) {
        let block = ctx.ts.fit_line(Voice::LABEL, text, note_w, 44.0 * u);
        let id = b.id("stamp");
        let mut layer = text_layer(id.clone(), &block, ctx.palette.accent, align);
        layer.x = match align {
            TextAlign::Right => w - m - layer.width,
            _ => m,
        };
        layer.y = cursor;
        layer.z_index = 24;
        // The stamped figure keeps its text and its word-cued time; it lands
        // from the planned side (`Stamp`: with a strike), pops or cascades.
        b.motions.extend(direction::stamp_entrance(
            &bp,
            &id,
            at,
            plan.lang.preset.duration * 0.9,
            strike,
            text.chars().filter(|c| !c.is_whitespace()).count(),
            plan.lang.preset.travel * 0.5 * u,
        ));
        cursor += block.height() + 36.0 * u;
        b.push(layer);
    }
    if let (Some(sec), Some(at)) = (sec, ev.next()) {
        if sec.kind() == SubjectKind::Object {
            let s = 0.16 * h;
            let slot = Slot {
                cx: if mirror { m + s / 2.0 } else { w - m - s / 2.0 },
                cy: cursor + s / 2.0,
                w: s,
                h: s,
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
                l.z_index = 24;
                placed.push(Placed::of(&l));
                b.push(l);
            }
            // (0.22) The secondary's value is stamped like the primary's: on
            // the secondary's row, beside it (stacked), or in the text column
            // (side by side, where the hero fills the other half).
            if let Some(text) = sec.value().map(str::trim).filter(|v| !v.is_empty()) {
                let gap = 0.03 * w;
                let (max_w, sec_align) = if land {
                    (note_w, align)
                } else if mirror {
                    (w - m - (slot.cx + s / 2.0 + gap), TextAlign::Left)
                } else {
                    (slot.cx - s / 2.0 - gap - m, TextAlign::Right)
                };
                let block = ctx
                    .ts
                    .fit_line(Voice::LABEL, text, max_w.max(0.2 * w), 44.0 * u);
                let id = b.id("support_stamp");
                let mut layer = text_layer(id.clone(), &block, ctx.palette.accent, sec_align);
                layer.x = match (land, sec_align) {
                    (true, TextAlign::Right) => w - m - layer.width,
                    (true, _) => m,
                    (false, TextAlign::Right) => slot.cx - s / 2.0 - gap - layer.width,
                    (false, _) => slot.cx + s / 2.0 + gap,
                };
                layer.y = slot.cy - layer.height / 2.0;
                layer.z_index = 24;
                b.motions.extend(direction::stamp_entrance(
                    &bp,
                    &id,
                    at + 0.1,
                    plan.lang.preset.duration * 0.9,
                    strike,
                    text.chars().filter(|c| !c.is_whitespace()).count(),
                    plan.lang.preset.travel * 0.5 * u,
                ));
                b.reveal(
                    "support_stamp",
                    [text.to_string()],
                    crate::speech::RevealRole::Value,
                );
                b.push(layer);
            }
        } else {
            let text = sec.display_text().unwrap_or("");
            plate::serif_note(
                ctx,
                b,
                "serif",
                text,
                cursor,
                align,
                note_w,
                0.09 * h,
                84.0 * u,
                at,
            );
        }
    }

    direction::steer_entrances(&bp, &mut b.motions, &placed);
    b.anchor = Some(Slot {
        cx: w - m - 130.0 * u,
        cy: 0.115 * h,
        w: 260.0 * u,
        h: 64.0 * u,
    });
    Ok(())
}
