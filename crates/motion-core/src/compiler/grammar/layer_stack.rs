//! (0.19) LayerStack: the beat's scene over the persistent layer column.
//!
//! The column itself (full-width strata with names and notes, the lit layer,
//! the glide from beat to beat) lives in the backdrop scene
//! (`compiler/layer_stack.rs`). This builder adds what the beat is about on
//! top of it: the headline, and the secondary subject pinned inside the lit
//! layer with its meaning as a caption. The picture swings in from this beat's
//! side once the column has settled, like the cinematic pictures do.
//!
//! The scene's own camera never moves (see `fx.rs`): the column is a 2D
//! backdrop, so a moving camera would pull the pinned picture off its layer.

use super::cinematic3d::{arrival_motions, place_picture, set_depth, stage_rect, Arrival};
use super::placement::Rect;
use super::Composition;
use crate::assets::AssetRole;
use crate::compiler::layer_stack as stack;
use crate::compiler::recipes::{self, B};
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mo, rect_layer, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::Subject;
use crate::scene::{LayerKind, Stroke, TextAlign};

/// (0.23) Put the pinned text layer `id` on a card like the column's label
/// cards (light type on the deep colour): the layer becomes a group (the
/// original id) of the card (`<id>.card`) and the text (`<id>.text`), so its
/// motions, focal record and reveal anchors apply to both. A pinned picture
/// is left as it is.
fn back_pinned_text(ctx: &Ctx, b: &mut B, id: &str) {
    let Some(pos) = b.stage.iter().position(|l| l.id == id) else {
        return;
    };
    if !matches!(b.stage[pos].kind, LayerKind::Text(_)) {
        return;
    }
    let (light, deep) = stack::card_colors(ctx);
    let mut text = b.stage.remove(pos);
    let (w, h) = (
        text.width * text.scale_x.abs(),
        text.height * text.scale_y.abs(),
    );
    let (x, y) = (text.x - text.anchor_x * w, text.y - text.anchor_y * h);
    let pad = 0.02 * ctx.w.min(ctx.h);
    let z_index = text.z_index;
    if let LayerKind::Text(style) = &mut text.kind {
        style.color = light;
    }
    text.id = format!("{id}.text");
    text.x = pad + text.anchor_x * w;
    text.y = pad + text.anchor_y * h;
    text.z_index = 1;
    let card = base_layer(
        format!("{id}.card"),
        (0.0, 0.0, w + 2.0 * pad, h + 2.0 * pad),
        LayerKind::RoundedRectangle {
            fill: deep.with_alpha(0xDC),
            radius: 12.0 * ctx.u,
            stroke: Some(Stroke {
                color: ctx.palette.accent.with_alpha(0xB0),
                width: (2.5 * ctx.u).max(1.5),
            }),
        },
        0,
    );
    let group = base_layer(
        id.to_string(),
        (x - pad, y - pad, w + 2.0 * pad, h + 2.0 * pad),
        LayerKind::Group {
            children: vec![card, text],
        },
        z_index,
    );
    b.stage.insert(pos, group);
}

/// (0.23) Seconds before READ at which the pinned subject starts to enter at
/// the latest (its fade takes 0.25 s): it is the beat's focal layer and must be
/// drawn when the beat is read.
const FOCAL_LEAD: f64 = 0.3;

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let Subject::Layers(layers) = &b.beat.primary else {
        return Err(CompileError::Beat {
            beat: b.plan.index + 1,
            message: "the layers grammar needs a layers primary".into(),
        });
    };
    let layers = layers.clone();
    let seeded = ctx.direction_seed.is_some();
    // A dark headline (a light palette) over the dark strata would not read:
    // give it paper to stand on, fading out into the column.
    let dark_ink = stack::lum(ctx.palette.ink) < 0.5;
    if dark_ink && !seeded {
        title_scrim(ctx, b, None);
    }
    let head = recipes::structured_head(ctx, b);
    // (0.23) On the product path the paper holds to the foot of the headline
    // (`head.top` is its lower edge plus the gap the region leaves): the
    // second line of a headline used to stand on strata the fading paper had
    // barely covered (`text_local_contrast`, 2.3-2.9:1 at READ).
    if dark_ink && seeded {
        title_scrim(ctx, b, Some(head.top - 64.0 * ctx.u + 24.0 * ctx.u));
    }
    let Some(plan) = ctx.stack.clone() else {
        return Ok(());
    };
    let geo = plan.geo;
    let bands = stack::bands(&geo, &layers);
    let focus = layers.focus_index();
    let pose = stack::pose(&geo, &bands, focus);
    let continues = plan.continues(b.plan.index);
    let life = b.plan.life;
    let mut ready = stack::settled_at(b.plan, continues).max(head.start);
    // (0.23) The pinned subject is the beat's focal layer, and layout QA
    // judges the beat from READ on: it must be drawn then. A headline that
    // reads in slowly (or a short beat whose READ comes early) used to leave
    // it starting at or after READ ("focal_out_of_focus: not drawn"). It then
    // starts [`FOCAL_LEAD`] before READ instead (never before ENTER), while
    // the column is still settling. Without a direction seed only a pinned
    // subject that would start at READ or later moves, and only where the
    // focal record is checked (art direction).
    let late = if seeded { FOCAL_LEAD } else { 0.02 };
    if (seeded || ctx.art.is_some()) && ready > life.read - late {
        ready = (life.read - FOCAL_LEAD).max(life.enter + 0.1);
    }

    let Some(secondary) = b.beat.secondary.clone() else {
        return Ok(());
    };
    let (w, u) = (ctx.w, ctx.u);
    // The lit layer's middle (the whole stack's when nothing is lit).
    let cy = match focus {
        Some(i) => {
            let (top, bottom) = stack::band_span(&bands, &pose, i);
            0.5 * (top + bottom)
        }
        None => geo.target_y(),
    };
    // The right half of the lit layer; the names and notes sit on the left.
    let (pw, ph) = (0.38 * w, 0.32 * geo.content_h);
    let bottom = geo.content_h - 0.06 * geo.content_h;
    let y = (cy - ph / 2.0).min(bottom - ph);
    let target = Rect::new(w - ctx.margin() - pw, y, pw, ph);

    // Text pinned in a layer is coloured for the layer behind it.
    let ink = stack::on_color(
        ctx,
        stack::band_color(ctx, &bands, focus.unwrap_or(bands.items.len() / 2)),
    );
    let Some(id) = place_picture(
        ctx,
        b,
        carries,
        "pinned",
        &[AssetRole::SupportingObject],
        Some((Which::Secondary, secondary.clone())),
        target,
        14,
        ready,
        ink,
    )?
    else {
        return Ok(());
    };
    // (0.23) A text pinned in a seam (a boundary layer: a thin band that
    // brightens to its middle) stands on a card: the strip behind it runs from
    // dark to bright across its height, a mid tone for most of it, and no type
    // colour reads on that in a bright palette (2.7:1 either way).
    if seeded && focus.is_some_and(|i| bands.items[i].boundary) {
        back_pinned_text(ctx, b, &id);
    }
    set_depth(b, &id, 0.0);
    b.focal = Some(id.clone());
    // (0.20) The pinned subject arrives when it is named; the headline waits
    // for its key word under a holding look.
    b.reveal(
        "pinned",
        recipes::subject_words(&secondary),
        crate::speech::RevealRole::Content,
    );
    b.reveal(
        "head",
        recipes::title_key_words(b.beat, &b.beat.statement),
        crate::speech::RevealRole::Title,
    );
    b.motions
        .push(mo::fade(&id, ready, 0.25, 0.0, 1.0, Easing::OutCubic));
    let arrival = Arrival::rotated_along_band(ctx, b.plan.index);
    // (0.23) From the left the picture would cross the lit layer's label
    // card on its way (the name and note sit at the left margin) and its
    // type read over the card at READ; it travels only as far as the card's
    // right edge allows. From the right nothing is in the way.
    let mut travel: f32 = 0.8;
    if seeded && arrival == Arrival::Left {
        if let Some(i) = focus {
            let right = stack::card_right(ctx, &geo, &layers, i);
            let room = target.x - right - 0.03 * w;
            travel = travel.min((room / (0.55 * w)).max(0.12));
        }
    }
    arrival_motions(b, &id, arrival, w, geo.content_h, travel, ready, 0.9);

    // What it is, under it.
    // A caption only says something new: a phrase or number already shows its
    // value, so only its meaning is written.
    let text = secondary.meaning().map(str::to_uppercase);
    if let (Some(text), Some(pic)) = (text, stage_rect(b, &id)) {
        let line = ctx
            .ts
            .fit_line(Voice::LABEL, &text, pic.w.max(0.3 * w), 44.0 * u);
        let label_id = b.id("pinned_label");
        b.reveal(
            "pinned_label",
            recipes::subject_words(&secondary),
            crate::speech::RevealRole::Label,
        );
        let mut l = text_layer(label_id.clone(), &line, ink, TextAlign::Center);
        // Centred under the picture, inside the side margins.
        let (lo, hi) = (ctx.margin(), (w - ctx.margin() - l.width).max(ctx.margin()));
        l.x = (pic.x + (pic.w - l.width) / 2.0).clamp(lo, hi);
        l.y = (pic.bottom() + 0.008 * geo.content_h)
            .min(geo.content_h - l.height - 0.01 * geo.content_h);
        l.z_index = 15;
        l.z = Some(0.0);
        b.motions.push(mo::fade(
            &label_id,
            ready + 0.6,
            0.4,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        b.stage.push(l);
    }
    Ok(())
}

/// A soft paper-coloured gradient over the headline zone, so dark type reads
/// whatever band is behind it. One group, one fade. `solid_to` (canvas y):
/// the paper holds to there and fades out below it; `None`: the legacy
/// profile, fading from the top of the canvas.
fn title_scrim(ctx: &Ctx, b: &mut B, solid_to: Option<f32>) {
    let (w, h) = (ctx.w, ctx.h);
    let legacy = stack::TITLE_ZONE * h;
    let zone = match solid_to {
        Some(y) => (y + 0.1 * h).max(legacy),
        None => legacy,
    };
    let strips = 40usize;
    let sh = (zone * 1.1) / strips as f32;
    let kids: Vec<_> = (0..strips)
        .map(|k| {
            let y = (k as f32 + 0.5) * sh;
            // Opaque at the top, gone by the bottom of the zone.
            let fade = match solid_to {
                Some(solid) if y <= solid => 0.0,
                Some(solid) => ((y - solid) / (zone - solid).max(1.0)).clamp(0.0, 1.0),
                None => ((k as f32 + 0.5) / strips as f32).clamp(0.0, 1.0),
            };
            let a = (255.0 * 0.96 * (1.0 - fade * fade * (3.0 - 2.0 * fade))).round() as u8;
            rect_layer(
                format!("{}.s{k}", b.id("scrim")),
                (0.0, k as f32 * sh, w, sh + 1.0),
                ctx.palette.paper.with_alpha(a),
                0,
            )
        })
        .collect();
    let id = b.id("scrim");
    let group = base_layer(
        id.clone(),
        (0.0, 0.0, w, zone * 1.1),
        crate::scene::LayerKind::Group { children: kids },
        1,
    );
    b.motions.push(mo::fade(
        &id,
        b.plan.enter_at() * 0.5,
        0.5,
        0.0,
        1.0,
        Easing::OutCubic,
    ));
    b.push(group);
}
