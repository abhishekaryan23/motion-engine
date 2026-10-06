//! (0.17) Cinematic3d — the beat staged in depth for a moving 3D camera.
//!
//! The builder lays the beat out in screen space (as it should look at the
//! main camera position) and tags every layer with its depth `z`; the FX
//! director (`compiler::fx`) hoists the depth layers out of the stage,
//! compensates their scale/position for the perspective so the composition
//! holds at rest, and choreographs the camera (push-in, truck, crane, orbit),
//! the rack focus and the fly-through transitions. Planes, far to near:
//!
//! | z | plane |
//! |---|---|
//! | 2200 | two soft glow discs in the palette's field colours |
//! | 600 | (0.19) the environment plate: a scene that fits the story (matched in the look's asset families by the beat's words), dimmed and out of focus |
//! | 1300 | the keyword as a giant ghost word |
//! | 300 – 1600 | dust motes (also some in front, −300 – 0) |
//! | 0 | the hero: the primary picture, or a big number / phrase |
//! | −60 | the supporting picture and its label, in focus (0.19) |
//! | −120 | the display title |
//! | −700 | three big foreground bokeh discs |

use super::placement::{self, Rect, SubjectFacts};
use super::{plate, relation_stage, Composition};
use crate::assets::AssetRole;
use crate::compiler::catalog::{request_words, MatchKind};
use crate::compiler::recipes::{self, subject_words, title_key_words, Entrance, Slot, B};
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, direction, mix, mo, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::{Subject, SubjectKind};
use crate::scene::{
    Color, GlyphOrder, GlyphPose, ImageTreatment, Layer, LayerKind, Motion, MotionOp, SpringSpec,
    TextAlign,
};
use crate::speech::RevealRole;

/// (0.19) Marks the slide of a picture's arrival: the FX director moves it
/// (with the beat's `tilt`) from the stage child to its depth wrapper, so the
/// whole plane slides and turns together.
pub(crate) const ARRIVAL_ID: &str = "arrival";
/// The supporting picture sits just in front of the focus plane, so it is in
/// focus and passes in front of the hero when the camera moves (0.19).
pub(crate) const Z_SUPPORT: f32 = -60.0;

pub(crate) const Z_GLOW: f32 = 2200.0;
pub(crate) const Z_GHOST: f32 = 1300.0;
/// (0.19) The environment plate sits behind the prop and in front of the ghost word.
pub(crate) const Z_ENV: f32 = 600.0;
pub(crate) const Z_HERO: f32 = 0.0;
pub(crate) const Z_TITLE: f32 = -120.0;
pub(crate) const Z_BOKEH: f32 = -700.0;

/// (0.19) Where a beat's picture comes in from. Chosen from the beat index
/// alone (deterministic, and neighbours always differ), so a story's pictures
/// swing in from the right, the left, below and above in turn instead of all
/// growing out of the centre. (0.23) Under a direction seed the side comes from
/// a seeded sequence instead ([`Arrival::rotated`]); neighbours still differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arrival {
    Right,
    Left,
    Below,
    Above,
}

impl Arrival {
    /// The four sides, in the order of the position rule and of the seeded
    /// sequence's option indices.
    const SIDES: [Arrival; 4] = [
        Arrival::Right,
        Arrival::Left,
        Arrival::Below,
        Arrival::Above,
    ];

    pub(crate) fn for_beat(index: usize) -> Arrival {
        Self::SIDES[index % 4]
    }

    /// (0.23) The side beat `index` comes in from. Without a direction seed
    /// the position rule ([`Arrival::for_beat`]); under one the beat's entry of
    /// `seeded_sequence(seed, Dim::Arrival, beats, 4)` (neighbouring beats never
    /// share a side), recorded in the direction record.
    pub(crate) fn rotated(ctx: &mut Ctx, index: usize) -> Arrival {
        let Some(seed) = ctx.direction_seed else {
            return Arrival::for_beat(index);
        };
        let beats = ctx.beat_count.max(index + 1);
        let side = direction::seeded_sequence(seed, direction::Dim::Arrival, beats, 4)
            .get(index)
            .map_or(index % 4, |&k| k % 4);
        let arrival = Self::SIDES[side];
        super::note_rotation(ctx, index, "arrival", arrival.name());
        arrival
    }

    /// (0.23) [`Arrival::rotated`] for a picture pinned inside a horizontal
    /// band (the layers grammar): it slides along its band, from the right or
    /// from the left (a seeded sequence of two, so neighbouring beats come in
    /// from opposite sides). A vertical entrance would cross the other layers'
    /// bands, and the type pinned on it would read over their colours at READ.
    pub(crate) fn rotated_along_band(ctx: &mut Ctx, index: usize) -> Arrival {
        let Some(seed) = ctx.direction_seed else {
            return Arrival::for_beat(index);
        };
        let beats = ctx.beat_count.max(index + 1);
        let side = direction::seeded_sequence(seed, direction::Dim::Arrival, beats, 2)
            .get(index)
            .map_or(index % 2, |&k| k % 2);
        let arrival = Self::SIDES[side];
        super::note_rotation(ctx, index, "arrival", arrival.name());
        arrival
    }

    /// The option name recorded in `DirectionRecord`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Arrival::Right => "right",
            Arrival::Left => "left",
            Arrival::Below => "below",
            Arrival::Above => "above",
        }
    }

    /// The side a supporting picture comes from: opposite its hero's.
    pub(crate) fn opposite(self) -> Arrival {
        match self {
            Arrival::Right => Arrival::Left,
            Arrival::Left => Arrival::Right,
            Arrival::Below => Arrival::Above,
            Arrival::Above => Arrival::Below,
        }
    }

    /// Start offset (canvas px) and `[pitch, yaw]` tilt (degrees) of the
    /// arrival; both settle to rest. The leading edge turns away from the
    /// viewer and swings round to face it, like a card carried in on an arc.
    fn pose(self, w: f32, h: f32, scale: f32) -> ([f32; 2], [f32; 2]) {
        match self {
            Arrival::Right => ([0.55 * w * scale, 0.02 * h], [0.0, -48.0 * scale]),
            Arrival::Left => ([-0.55 * w * scale, 0.02 * h], [0.0, 48.0 * scale]),
            Arrival::Below => ([0.0, 0.34 * h * scale], [-40.0 * scale, 0.0]),
            Arrival::Above => ([0.0, -0.34 * h * scale], [40.0 * scale, 0.0]),
        }
    }
}

/// The slide and the turn of an arrival, targeted at the stage layer `id`
/// (the FX director re-targets both at its depth wrapper).
#[allow(clippy::too_many_arguments)]
pub(super) fn arrival_motions(
    b: &mut B,
    id: &str,
    from: Arrival,
    w: f32,
    h: f32,
    scale: f32,
    start: f64,
    duration: f64,
) {
    let (offset, tilt) = from.pose(w, h, scale);
    // A plain ease-out, no spring: a spring (or a steep ease) finishes most of
    // the glide while the picture is still fading in and out of focus, and the
    // arrival is never seen.
    let mut slide = mo::shift(id, start, duration, offset, [0.0, 0.0], Easing::OutCubic);
    slide.id = Some(ARRIVAL_ID.to_string());
    b.motions.push(slide);
    b.motions.push(Motion {
        id: None,
        target: id.to_string(),
        start,
        duration,
        easing: Easing::OutCubic,
        spring: None,
        op: MotionOp::Tilt {
            from: tilt,
            to: [0.0, 0.0],
        },
    });
}

/// When a picture's arrival must have landed: the fly-in ends at ENTER +
/// min(0.75 s, a quarter of the beat) (`fx::choreograph`), and a card still
/// sliding after the focus has racked onto it goes soft.
pub(super) fn landing_for(duration: f64) -> f64 {
    (0.25 * duration + 0.1).min(0.85)
}

/// (0.19) A picture's whole entrance on the focus plane: it fades in at
/// `start`, settles from a slight shrink and swings in from `from`
/// ([`arrival_motions`]; `strength` scales the travel), landing by
/// `start + landing`. The same sequence the hero of a cinematic beat uses.
#[allow(clippy::too_many_arguments)]
pub(super) fn arrive(
    b: &mut B,
    id: &str,
    from: Arrival,
    w: f32,
    h: f32,
    strength: f32,
    start: f64,
    landing: f64,
) {
    b.motions
        .push(mo::fade(id, start, 0.2, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(sprung(
        mo::scale(id, start + 0.13, 0.9, 0.82, 1.0, Easing::OutQuint),
        SETTLE,
    ));
    arrival_motions(b, id, from, w, h, strength, start, landing);
}

/// The stage layer `id` as a canvas rectangle (its anchor and scale applied).
pub(super) fn stage_rect(b: &B, id: &str) -> Option<Rect> {
    b.stage.iter().find(|l| l.id == id).map(|l| {
        let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
        Rect::new(l.x - l.anchor_x * w, l.y - l.anchor_y * h, w, h)
    })
}

pub(super) const SETTLE: SpringSpec = SpringSpec {
    stiffness: 220.0,
    damping: 22.0,
    mass: 1.0,
};

pub(super) fn sprung(mut m: Motion, s: SpringSpec) -> Motion {
    m.spring = Some(s);
    m
}

pub(super) fn disc(
    id: String,
    center: (f32, f32),
    d: f32,
    fill: Color,
    z_index: i32,
    z: f32,
) -> Layer {
    let mut l = base_layer(
        id,
        (center.0, center.1, d, d),
        LayerKind::RoundedRectangle {
            fill,
            radius: d / 2.0,
            stroke: None,
        },
        z_index,
    );
    l.anchor_x = 0.5;
    l.anchor_y = 0.5;
    l.z = Some(z);
    l
}

/// The delivered picture for `role`s, with the role it is filed under.
pub(super) fn delivered(
    ctx: &Ctx,
    index: usize,
    roles: &[AssetRole],
) -> Option<(crate::assets::ManifestEntry, AssetRole)> {
    roles
        .iter()
        .find_map(|r| plate::image(ctx, index, *r).map(|e| (e.clone(), *r)))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn place_picture(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    name: &str,
    roles: &[AssetRole],
    object: Option<(Which, Subject)>,
    target: Rect,
    z_index: i32,
    start: f64,
    ink: Color,
) -> Result<Option<String>, CompileError> {
    if let Some((entry, role)) = delivered(ctx, b.plan.index, roles) {
        let img = placement::subject_fit(&SubjectFacts::from_entry(&entry), target);
        let p = plate::image_plate(
            ctx,
            b,
            name,
            role,
            (img.x, img.y, img.w, img.h),
            z_index,
            start,
        );
        let id = p.id.clone();
        b.motions
            .retain(|m| !(m.target == id && matches!(m.op, MotionOp::MaskReveal { .. })));
        for l in p.layers {
            b.push(l);
        }
        return Ok(Some(id));
    }
    let Some((which, subject)) = object else {
        return Ok(None);
    };
    let slot = Slot {
        cx: target.x + target.w / 2.0,
        cy: target.y + target.h / 2.0,
        w: target.w,
        h: target.h,
    };
    let Some(mut l) = recipes::place_subject(
        ctx,
        b,
        carries,
        which,
        &subject,
        slot,
        (0.0, 0.0),
        ink,
        start,
        Entrance::Pop,
        None,
        None,
        name,
    )?
    else {
        return Ok(None);
    };
    l.z_index = z_index;
    let id = l.id.clone();
    b.motions.retain(|m| {
        !(m.target == id && matches!(m.op, MotionOp::Scale { .. } | MotionOp::ClipReveal { .. }))
    });
    b.push(l);
    Ok(Some(id))
}

/// Tag the stage child `id` (and nothing else) with a depth.
pub(super) fn set_depth(b: &mut B, id: &str, z: f32) {
    for l in b.stage.iter_mut() {
        if l.id == id {
            l.z = Some(z);
        }
    }
}

/// (0.23) Where `support` may put the supporting picture and what it carries.
#[derive(Debug, Clone, Copy)]
struct Room {
    /// What the hero already carries under it (its value card): the picture
    /// starts below that.
    below_extra: f32,
    /// Put the picture beside the hero (on its roomier side), centred on it,
    /// instead of under it: for a hero that leaves no free band below.
    beside: bool,
    /// The lowest canvas y the picture's label or card may reach (the bottom
    /// of the safe area), when it must not run past it.
    floor: Option<f32>,
}

impl Room {
    /// The 0.19 placement: under the hero.
    const BELOW: Room = Room {
        below_extra: 0.0,
        beside: false,
        floor: None,
    };
}

/// (0.19) The supporting picture: fully inside the frame, on the focus plane,
/// in the free band under the hero (or beside a tall hero), swinging in from
/// the opposite side, with the secondary subject's meaning as its label. It
/// used to be a blurred prop half behind the hero, so what it was and how it
/// related to the hero was lost.
#[allow(clippy::too_many_arguments)]
fn support(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    hero_id: &str,
    object: Option<(Which, Subject)>,
    left: bool,
    from: Arrival,
    start: f64,
    room: Room,
) -> Result<(), CompileError> {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let Some(mut hero) = stage_rect(b, hero_id) else {
        return Ok(());
    };
    // (0.23) What the hero already carries under it (its value card): the
    // supporting picture starts below that.
    hero.h += room.below_extra;
    // (0.23) An object with a value and a meaning: the label becomes a card
    // with the figure over the meaning (a value alone stays the label).
    let card_value = object
        .as_ref()
        .and_then(|(_, s)| s.value())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .filter(|_| {
            object
                .as_ref()
                .and_then(|(_, s)| s.meaning())
                .is_some_and(|m| !m.trim().is_empty())
        });
    let meaning = object
        .as_ref()
        .and_then(|(_, s)| s.meaning().or_else(|| s.value()))
        .map(str::to_uppercase);
    // (0.20) The supporting picture and its label arrive when the narrator
    // names the secondary.
    let named: Vec<String> = object
        .as_ref()
        .map(|(_, s)| subject_words(s))
        .unwrap_or_default();
    let label_h = match (&meaning, &card_value) {
        (_, Some(_)) => 0.16 * w.min(h),
        (Some(_), None) => 0.05 * h,
        (None, None) => 0.0,
    };
    let margin = 0.05 * w;
    // Free band under the hero, above the lane the caller left for captions.
    let floor_y = if card_value.is_some() {
        0.95 * h
    } else {
        0.97 * h
    };
    // (0.23) Under a figure hero the picture and its label or card must end
    // inside the safe area: the picture gives way, not the label.
    let floor_y = floor_y.min(room.floor.unwrap_or(f32::MAX));
    let below = floor_y - hero.bottom() - label_h;
    let side_room = hero.x.max(w - hero.right()) - margin;
    let target = if below >= 0.14 * h && !room.beside {
        let (pw, ph) = (0.38 * w, below.min(0.26 * h));
        let x = if left { margin } else { w - margin - pw };
        Rect::new(x, hero.bottom() + 0.012 * h, pw, ph)
    } else if side_room >= 0.2 * w {
        // Beside a tall hero, on its roomier side.
        let on_left = hero.x >= w - hero.right();
        let pw = side_room.min(0.34 * w);
        let x = if on_left { margin } else { w - margin - pw };
        if room.beside {
            // Level with the hero: its label or card hangs below, inside the
            // safe area.
            let ph = 0.3 * h;
            // Kept off the edge: the camera's push carries it outward.
            let x = if on_left {
                x.max(0.09 * w)
            } else {
                x.min(w - 0.09 * w - pw)
            };
            Rect::new(x, hero.y + hero.h / 2.0 - ph / 2.0, pw, ph)
        } else {
            let ph = 0.22 * h;
            Rect::new(x, hero.y + hero.h * 0.62 - ph / 2.0, pw, ph)
        }
    } else {
        // No free room: a sticker over the hero's lower corner.
        let (pw, ph) = (0.3 * w, 0.18 * h);
        let x = if left { margin } else { w - margin - pw };
        Rect::new(x, hero.bottom() - ph * 0.8, pw, ph)
    };
    let Some(id) = place_picture(
        ctx,
        b,
        carries,
        "prop",
        &[AssetRole::SupportingObject],
        object,
        target,
        14,
        start,
        ctx.palette.ink,
    )?
    else {
        return Ok(());
    };
    set_depth(b, &id, Z_SUPPORT);
    b.reveal("prop", named.iter().cloned(), RevealRole::Content);
    b.motions
        .push(mo::fade(&id, start, 0.4, 0.0, 1.0, Easing::OutCubic));
    arrival_motions(b, &id, from, w, h, 0.6, start, 1.0);
    if let (Some(value), Some(pic)) = (card_value.as_deref(), stage_rect(b, &id)) {
        // (0.23) The figure over the meaning on a card under the picture.
        let label_id = b.id("prop_label");
        b.reveal("prop_label", named.iter().cloned(), RevealRole::Label);
        let short = w.min(h);
        let card = relation_stage::stat_card(
            ctx,
            label_id.clone(),
            pic.cx(),
            pic.bottom() + 0.008 * h,
            Some(value),
            meaning.as_deref(),
            pic.w.max(0.4 * w),
            0.07 * short,
            if room.floor.is_some() {
                (ctx.frame.safe.x, ctx.frame.safe.x + ctx.frame.safe.w)
            } else {
                (margin, w - margin)
            },
            Some(Z_SUPPORT),
        );
        if let Some(mut card) = card {
            let bottom = room.floor.map_or(0.95 * h, |f| f.min(0.95 * h));
            card.y = card.y.min(bottom - card.height / 2.0);
            b.motions.push(mo::fade(
                &label_id,
                start + 0.55,
                0.4,
                0.0,
                1.0,
                Easing::OutCubic,
            ));
            b.stage.push(card);
        }
    } else if let (Some(text), Some(pic)) = (meaning, stage_rect(b, &id)) {
        let line = ctx
            .ts
            .fit_line(Voice::LABEL, &text, pic.w.max(0.3 * w), 44.0 * u);
        let label_id = b.id("prop_label");
        b.reveal("prop_label", named.iter().cloned(), RevealRole::Label);
        let mut l = text_layer(label_id.clone(), &line, ctx.palette.ink, TextAlign::Center);
        l.x = pic.x + (pic.w - l.width) / 2.0;
        if room.floor.is_some() {
            // (0.23) A label wider than its picture stays inside the safe area.
            let safe = ctx.frame.safe;
            l.x = l.x.min(safe.x + safe.w - l.width).max(safe.x);
        }
        let bottom = room.floor.map_or(h - 0.01 * h, |f| f.min(h - 0.01 * h));
        l.y = (pic.bottom() + 0.008 * h).min(bottom - l.height);
        l.z_index = 15;
        l.z = Some(Z_SUPPORT);
        b.motions.push(mo::fade(
            &label_id,
            start + 0.55,
            0.4,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        b.stage.push(l);
    }
    Ok(())
}

/// (0.23) Height of the card a supporting picture's value and meaning sit on
/// (`support`), when it has both.
fn support_card_height(ctx: &Ctx, s: &Subject) -> Option<f32> {
    let value = s.value().map(str::trim).filter(|v| !v.is_empty())?;
    let meaning = s.meaning().map(str::trim).filter(|m| !m.is_empty())?;
    let margin = 0.05 * ctx.w;
    relation_stage::stat_card(
        ctx,
        "probe".to_string(),
        ctx.w / 2.0,
        0.0,
        Some(value),
        Some(&meaning.to_uppercase()),
        0.4 * ctx.w,
        0.07 * ctx.w.min(ctx.h),
        (margin, ctx.w - margin),
        None,
    )
    .map(|c| c.height)
}

/// (0.23) Wrap the figure's text layer `id` in its frame: a group spanning the
/// width of the composition (`b.id("hero_word.frame")`, the focal layer of a
/// figure beside a picture) at the figure's height, so the camera pivots on
/// the middle of the composition while the focal record holds the figure.
fn frame_figure(b: &mut B, id: &str, w: f32) {
    let Some(pos) = b.stage.iter().position(|l| l.id == id) else {
        return;
    };
    let mut text = b.stage.remove(pos);
    let (gx, gw) = (0.05 * w, 0.9 * w);
    let (ty, th, z_index) = (text.y, text.height, text.z_index);
    text.x -= gx;
    text.y = 0.0;
    text.z = None;
    text.z_index = 0;
    let mut frame = base_layer(
        b.id("hero_word.frame"),
        (gx, ty, gw, th),
        LayerKind::Group {
            children: vec![text],
        },
        z_index,
    );
    frame.z = Some(Z_HERO);
    b.stage.insert(pos, frame);
}

/// (0.23) The written figure of a number or phrase subject (its value).
fn figure_value(s: &Subject) -> Option<String> {
    match s {
        Subject::Number(a) | Subject::Phrase(a) => a
            .value
            .as_deref()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty()),
        _ => None,
    }
}

/// (0.23) The value card under a hero or beside it: the figure over what it
/// stands for. `None` when the subject has no value to show.
pub(super) fn figure_card(
    ctx: &Ctx,
    id: String,
    subject: &Subject,
    max_w: f32,
    value_size: f32,
) -> Option<crate::scene::Layer> {
    let value = match subject {
        Subject::Object(o) => o
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        other => figure_value(other),
    }?;
    let margin = 0.05 * ctx.w;
    relation_stage::stat_card(
        ctx,
        id,
        ctx.w / 2.0,
        0.0,
        Some(&value),
        subject.meaning().map(str::to_uppercase).as_deref(),
        max_w,
        value_size,
        (margin, ctx.w - margin),
        Some(Z_SUPPORT),
    )
}

/// (0.23) The hero's own value, as a card under the hero picture or figure,
/// and a secondary figure (a number next to a picture) the same way: the
/// card sits centred under `hero_id` (below what `below_extra` already holds),
/// fades in at `start` and arrives when the narrator says its words.
#[allow(clippy::too_many_arguments)]
fn place_card(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    mut card: crate::scene::Layer,
    hero_id: &str,
    below_extra: f32,
    subject: &Subject,
    start: f64,
) -> f32 {
    let h = ctx.h;
    let Some(hero) = stage_rect(b, hero_id) else {
        return 0.0;
    };
    card.x = hero.cx();
    card.y = (hero.bottom() + below_extra + 0.012 * h + card.height / 2.0)
        .min(0.95 * h - card.height / 2.0);
    let used = card.height + 0.012 * h;
    push_card(b, name, card, subject, start);
    used
}

/// A card already placed: its reveal anchors, entrance and the push.
fn push_card(b: &mut B, name: &str, card: crate::scene::Layer, subject: &Subject, start: f64) {
    let id = card.id.clone();
    b.reveal(name, subject_words(subject), RevealRole::Value);
    b.motions
        .push(mo::fade(&id, start, 0.4, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(sprung(
        mo::scale(&id, start, 0.5, 0.85, 1.0, Easing::OutCubic),
        SETTLE,
    ));
    b.stage.push(card);
}

/// The words of the beat that say where it happens: keyword, title and the
/// names and meanings of its subjects (a collection by its meaning alone).
fn scene_words(beat: &crate::intent::Beat) -> std::collections::BTreeSet<String> {
    let mut parts: Vec<&str> = vec![beat.statement.as_str()];
    if let Some(k) = beat.keyword.as_deref() {
        parts.push(k);
    }
    for s in std::iter::once(&beat.primary).chain(beat.secondary.as_ref()) {
        match s {
            Subject::Object(o) => {
                parts.push(o.asset.as_str());
                parts.extend(o.meaning.as_deref());
                parts.extend(o.value.as_deref());
            }
            other => {
                parts.extend(other.meaning());
                parts.extend(other.value());
            }
        }
    }
    request_words(&parts)
}

/// (0.19) The environment plate of the beat: a delivered `environment` image,
/// else the best match among the look's families (a place that fits the words
/// of the story). Placed far behind the picture, oversized so that the fly-in
/// (the camera starts far back) and the orbit never show its edges, dimmed so
/// the picture and the type stay in front. Returns whether a plate was placed.
pub(super) fn environment_plate(ctx: &mut Ctx, b: &mut B) -> bool {
    let index = b.plan.index;
    let entry = plate::image(ctx, index, AssetRole::Environment)
        .cloned()
        .or_else(|| {
            let m = ctx
                .library
                .catalog_match(&scene_words(b.beat), MatchKind::Environment)?;
            ctx.library.catalog_delivery(
                &format!("{}/{}", m.family, m.id),
                &format!("beat_{}.environment", index + 1),
            )
        });
    let Some(entry) = entry else {
        return false;
    };
    let (w, h) = (ctx.w, ctx.h);
    let (iw, ih) = (entry.width.max(1) as f32, entry.height.max(1) as f32);
    // Cover the canvas with room for the fly-in zoom-out and the orbit.
    let k = 1.9;
    let s = (k * w / iw).max(k * h / ih);
    let (pw, ph) = (iw * s, ih * s);
    let id = b.id("env");
    let mut layer = plate::image_layer(
        ctx,
        b,
        id.clone(),
        &entry,
        AssetRole::Environment,
        ((w - pw) / 2.0, (h - ph) / 2.0, pw, ph),
        0,
    );
    // The picture keeps its own colours (no palette duotone); it is dimmed.
    if let LayerKind::Image { treatment, .. } = &mut layer.kind {
        *treatment = Some(ImageTreatment {
            brightness: -0.16,
            contrast: 0.92,
            desaturate: 0.08,
            ..ImageTreatment::natural()
        });
    }
    layer.z = Some(Z_ENV);
    layer.z_index = 0;
    b.motions
        .push(mo::fade(&id, 0.0, 0.9, 0.0, 1.0, Easing::OutCubic));
    // A slow drift over the whole beat keeps the place alive.
    b.motions.push(mo::scale(
        &id,
        0.0,
        b.plan.duration.max(0.5),
        1.0,
        1.06,
        Easing::Linear,
    ));
    b.stage.push(layer);
    true
}

/// The canvas y of the title's lower edge ([`title`] without placing it).
pub(super) fn title_bottom(ctx: &Ctx, b: &B, max_w: f32) -> f32 {
    let (h, u) = (ctx.h, ctx.u);
    let block = ctx.ts.fit_block(
        Voice::HEADLINE,
        &b.beat.statement,
        max_w,
        0.17 * h,
        120.0 * u,
        3,
    );
    let tl = text_layer(
        "title".to_string(),
        &block,
        ctx.palette.ink,
        TextAlign::Center,
    );
    0.13 * h + tl.height
}

/// (0.23 W4) The placeholder of a hero that waits for its spoken word: its
/// label, the primary's `meaning`, on a line just under the hero, on the focus
/// plane, from ENTER. The value (or the picture) still waits for its word; the
/// label never is the value (`recipes::placeholder_label`). `words` name the
/// hero, `hero_bottom` is the canvas y of the hero's lower edge. Nothing is
/// added when the hero does not wait that long, the beat has no label, or the
/// line would leave the safe area.
fn held_hero_label(ctx: &Ctx, b: &mut B, words: &[String], hero_bottom: f32, t: f64) {
    let Some(text) = recipes::placeholder_label(b.beat) else {
        return;
    };
    if !crate::compiler::speech_lifecycle::hero_waits(ctx, words) {
        return;
    }
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let line = ctx.ts.fit_line(Voice::LABEL, &text, 0.8 * w, 44.0 * u);
    let id = b.id("hero_label");
    let mut l = text_layer(id.clone(), &line, ctx.palette.ink, TextAlign::Center);
    l.x = (w - l.width) / 2.0;
    l.y = hero_bottom + 0.012 * h;
    let safe = ctx.frame.safe;
    if l.y + l.height > safe.y + safe.h || l.x < safe.x || l.x + l.width > safe.x + safe.w {
        return;
    }
    l.z_index = 14;
    l.z = Some(Z_HERO);
    b.motions
        .push(mo::fade(&id, t, 0.3, 0.0, 1.0, Easing::OutCubic));
    b.stage.push(l);
}

pub(crate) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let plan = b.plan;
    let t = plan.enter_at();
    let seed = plan.seed;
    let beat = b.beat;
    let fields = field_colors(ctx);
    let rnd = |k: u64| (mix(seed, 0xC1E0 + k) % 10_000) as f32 / 10_000.0;

    glow_discs(ctx, b, &fields);
    environment_plate(ctx, b);

    // Ghost word deep in the scene.
    let ghost_text = beat
        .keyword
        .clone()
        .or_else(|| beat.primary.value().map(str::to_string))
        .unwrap_or_else(|| beat.statement.clone());
    let ghost = ctx
        .ts
        .fit_line(Voice::GHOST, &ghost_text.to_uppercase(), 1.6 * w, 420.0 * u);
    let ghost_id = b.id("ghost_word");
    let mut gl = text_layer(ghost_id.clone(), &ghost, ctx.palette.ink, TextAlign::Center);
    gl.x = (w - gl.width) / 2.0;
    gl.y = 0.5 * h - gl.height / 2.0;
    gl.opacity = 0.14;
    gl.z_index = 2;
    gl.z = Some(Z_GHOST);
    b.motions
        .push(mo::fade(&ghost_id, t, 1.0, 0.0, 1.0, Easing::OutCubic));
    b.stage.push(gl);

    // Dust motes through the depth (a few in front of the hero).
    for k in 0..16u64 {
        let id = b.id(&format!("dust.{k}"));
        let (x, y) = (rnd(k * 3) * w, rnd(k * 3 + 1) * h);
        let near = k % 5 == 0;
        let z = if near {
            -300.0 + 280.0 * rnd(k * 3 + 2)
        } else {
            300.0 + 1300.0 * rnd(k * 3 + 2)
        };
        let d = (6.0 + 14.0 * rnd(k * 7 + 5)) * u;
        let mut dot = disc(
            id.clone(),
            (x, y),
            d,
            ctx.palette.ink.with_alpha(0x99),
            3,
            z,
        );
        dot.opacity = 1.0;
        b.motions.push(mo::fade(
            &id,
            t + 0.05 * k as f64,
            0.8,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        b.stage.push(dot);
    }

    // (0.19) A compare / contrast beat of two pictures shows how they relate;
    // every other beat is a hero on the focus plane with a supporting picture.
    let title_max_w = w - 2.0 * ctx.margin();
    let title_bottom = title_bottom(ctx, b, title_max_w);
    let related = relation_stage::build(
        ctx,
        b,
        carries,
        title_bottom,
        relation_stage::Stage::Cinematic,
    )?;
    // (0.23) A compare / contrast beat of two figures (numbers or phrases)
    // shows both on the focus plane, each with its meaning as its label.
    let valued = !related
        && relation_stage::build_values(ctx, b, title_bottom, relation_stage::Stage::Cinematic)?;
    if !related && !valued {
        let secondary_obj = beat
            .secondary
            .as_ref()
            .filter(|s| s.kind() == SubjectKind::Object)
            .map(|s| (Which::Secondary, s.clone()));
        let primary_obj = (beat.primary.kind() == SubjectKind::Object)
            .then(|| (Which::Primary, beat.primary.clone()));
        let left = mix(seed, 0x51DE).is_multiple_of(2);
        let delivered_hero = delivered(
            ctx,
            plan.index,
            &[
                AssetRole::HeroSubject,
                AssetRole::Portrait,
                AssetRole::HeroObject,
            ],
        )
        .is_some();
        let has_hero = primary_obj.is_some() || delivered_hero;
        // (0.23) A figure that is the beat's primary stays the hero when a
        // picture goes with it: the picture supports the number, instead of
        // taking the hero's place and dropping the figure.
        let figure_hero = primary_obj.is_none()
            && !delivered_hero
            && secondary_obj.is_some()
            && beat.primary.kind() == SubjectKind::Number
            && figure_value(&beat.primary).is_some();
        // (0.23) The figures the hero and its companions carry: the pictured
        // hero's own value, and a number beside a picture.
        let hero_card = primary_obj
            .as_ref()
            .and_then(|(_, s)| figure_card(ctx, b.id("hero_value"), s, 0.7 * w, 0.075 * w.min(h)));
        let second_figure = beat
            .secondary
            .as_ref()
            .filter(|s| s.kind() == SubjectKind::Number)
            .and_then(|s| {
                figure_card(ctx, b.id("prop_figure"), s, 0.7 * w, 0.09 * w.min(h)).map(|c| (s, c))
            });
        // The cards sit under the hero. Without any card the hero keeps its
        // 0.54 h; with one it gives them the room: what the supporting picture
        // (and its own card) and the hero's cards need below the hero.
        let support_card = secondary_obj
            .as_ref()
            .filter(|_| has_hero)
            .and_then(|(_, s)| support_card_height(ctx, s));
        let any_card = hero_card.is_some() || second_figure.is_some() || support_card.is_some();
        let hero_h = if any_card {
            let own: f32 = hero_card.iter().map(|c| c.height + 0.012 * h).sum::<f32>()
                + second_figure
                    .iter()
                    .map(|(_, c)| c.height + 0.012 * h)
                    .sum::<f32>();
            let support_need = match (has_hero && secondary_obj.is_some(), support_card) {
                (false, _) => 0.0,
                (true, Some(card)) => 0.16 * h + card,
                (true, None) => 0.14 * h + 0.05 * h,
            };
            (0.95 * h - 0.29 * h - own - support_need).clamp(0.3 * h, 0.54 * h)
        } else {
            0.54 * h
        };
        // Hero on the focus plane.
        let hero_target = Rect::new(0.07 * w, 0.29 * h, 0.86 * w, hero_h);
        let hero_subject = if figure_hero {
            None
        } else {
            primary_obj.or_else(|| secondary_obj.clone())
        };
        // (0.20) The hero arrives when the narrator names it (a picture of
        // the primary) or when the beat's primary is said (a delivered image).
        let hero_named: Vec<String> = hero_subject
            .as_ref()
            .map(|(_, s)| subject_words(s))
            .unwrap_or_else(|| subject_words(&beat.primary));
        let hero_words = hero_named.clone();
        let hero_id = place_picture(
            ctx,
            b,
            carries,
            "hero",
            &[
                AssetRole::HeroSubject,
                AssetRole::Portrait,
                AssetRole::HeroObject,
            ],
            hero_subject,
            hero_target,
            12,
            t + 0.15,
            ctx.palette.ink,
        )?;
        match hero_id {
            Some(id) => {
                // (0.23 W4) A lone hero picture that waits for its word has a
                // label under it meanwhile.
                let alone =
                    hero_card.is_none() && secondary_obj.is_none() && second_figure.is_none();
                set_depth(b, &id, Z_HERO);
                b.focal = Some(id.clone());
                b.reveal("hero", hero_named, RevealRole::Content);
                b.motions
                    .push(mo::fade(&id, t + 0.02, 0.2, 0.0, 1.0, Easing::OutCubic));
                b.motions.push(sprung(
                    mo::scale(&id, t + 0.15, 0.9, 0.82, 1.0, Easing::OutQuint),
                    SETTLE,
                ));
                // (0.19) The picture swings in from this beat's side. It must have
                // landed when the fly-in ends (ENTER + min(0.75 s, a quarter of the
                // beat), `fx::choreograph`): the camera's depth of field is thin
                // and the orbit carries an off-centre card through depth, so a card
                // still sliding after the focus has racked onto it goes soft.
                let from = Arrival::rotated(ctx, plan.index);
                arrival_motions(
                    b,
                    &id,
                    from,
                    w,
                    h,
                    1.0,
                    t + 0.02,
                    landing_for(plan.duration),
                );
                // (0.23) The hero's own value, on a card under it.
                let used = hero_card.map_or(0.0, |card| {
                    place_card(ctx, b, "hero_value", card, &id, 0.0, &beat.primary, t + 0.6)
                });
                if has_hero && secondary_obj.is_some() {
                    support(
                        ctx,
                        b,
                        carries,
                        &id,
                        secondary_obj.clone(),
                        left,
                        from.opposite(),
                        t + 0.35,
                        Room {
                            below_extra: used,
                            ..Room::BELOW
                        },
                    )?;
                }
                if let Some((s, card)) = second_figure {
                    place_card(ctx, b, "prop_figure", card, &id, used, s, t + 0.9);
                }
                if alone {
                    if let Some(r) = stage_rect(b, &id) {
                        held_hero_label(ctx, b, &hero_words, r.bottom(), t);
                    }
                }
            }
            None => {
                // A number or phrase is the hero: big type on the focus plane.
                let text = beat
                    .primary
                    .value()
                    .map(str::to_string)
                    .unwrap_or_else(|| beat.statement.clone());
                // (0.23) On the product path the figure's box (the block plus
                // the 2 % + 2 px the text layer adds) stays inside the safe
                // area: a long word is set smaller or wrapped, never past the
                // side margins (the box used to span 0.9 of the width, wider
                // than the 0.84 the safe area leaves, `text_outside_safe`).
                let safe = ctx.frame.safe;
                let seeded = ctx.direction_seed.is_some();
                let fit_w = if seeded {
                    (0.9 * w).min((safe.w - 2.0) / 1.02)
                } else {
                    0.9 * w
                };
                let mut blk =
                    ctx.ts
                        .fit_block(Voice::HEADLINE, &text, fit_w, 0.34 * h, 380.0 * u, 2);
                // (0.23) What goes with the figure (a picture and its label or
                // card, or a figure's card) needs room under it inside the safe
                // area (above the caption lane). Where the figure leaves too
                // little (square and wide canvases), it takes the left of the
                // frame and the picture or card the right, level with it.
                let safe_bottom = ctx.frame.safe.y + ctx.frame.safe.h;
                let below_need = if figure_hero {
                    let label = secondary_obj
                        .as_ref()
                        .and_then(|(_, s)| support_card_height(ctx, s))
                        .unwrap_or(0.05 * h);
                    0.14 * h + label + 0.03 * h
                } else {
                    second_figure
                        .as_ref()
                        .map_or(0.0, |(_, c)| c.height + 0.03 * h)
                };
                let beside = (figure_hero || second_figure.is_some())
                    && safe_bottom - (0.57 * h + blk.height() / 2.0) < below_need;
                // The left of the frame the figure takes beside a picture.
                let (side_left, side_w) = if seeded {
                    let left = (0.05 * w).max(safe.x);
                    (left, 0.57 * w - left)
                } else {
                    (0.05 * w, 0.52 * w)
                };
                if beside {
                    let fit_w = if seeded {
                        (0.5 * w).min((side_w - 2.0) / 1.02)
                    } else {
                        0.5 * w
                    };
                    blk = ctx
                        .ts
                        .fit_block(Voice::HEADLINE, &text, fit_w, 0.34 * h, 380.0 * u, 2);
                }
                // (0.23 W4) A lone figure or phrase that waits for its word has
                // its label under it meanwhile.
                let alone =
                    !figure_hero && !beside && second_figure.is_none() && secondary_obj.is_none();
                let id = b.id("hero_word");
                b.focal = Some(id.clone());
                if beside {
                    // The camera orbits and zooms about the focal layer: with
                    // the figure on the left that would carry the picture and
                    // its label out of the right edge. The focal layer is the
                    // figure's frame, a group that holds the visible figure and
                    // spans the whole composition (`frame_figure`), so the
                    // camera pivots on the middle of it and the focal checks
                    // (layout QA, the arrival-by-READ floor) still guard the
                    // figure.
                    b.focal = Some(b.id("hero_word.frame"));
                }
                b.reveal("hero_word", [text.clone()], RevealRole::Value);
                let mut l = text_layer(id.clone(), &blk, ctx.palette.accent, TextAlign::Center);
                l.x = if beside {
                    side_left + (side_w - l.width) / 2.0
                } else {
                    (w - l.width) / 2.0
                };
                l.y = 0.57 * h - l.height / 2.0;
                l.z_index = 12;
                l.z = Some(Z_HERO);
                let hero_right = l.x + l.width;
                let hero_bottom = l.y + l.height;
                b.motions.push(sprung(
                    Motion {
                        id: None,
                        target: id.clone(),
                        start: t + 0.15,
                        duration: 0.9,
                        easing: Easing::OutQuint,
                        spring: None,
                        op: MotionOp::GlyphCascade {
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
                    },
                    SETTLE,
                ));
                b.stage.push(l);
                if alone {
                    held_hero_label(ctx, b, std::slice::from_ref(&text), hero_bottom, t);
                }
                // (0.23) A picture goes with a figure that is the hero (the
                // picture supports it), and a number beside the hero gets a
                // card under it.
                if figure_hero {
                    let from = Arrival::rotated(ctx, plan.index).opposite();
                    support(
                        ctx,
                        b,
                        carries,
                        &id,
                        secondary_obj.clone(),
                        left,
                        from,
                        t + 0.35,
                        Room {
                            below_extra: 0.0,
                            beside,
                            floor: Some(safe_bottom - 0.02 * h),
                        },
                    )?;
                }
                if let Some((s, card)) = second_figure {
                    if beside {
                        // To the right of the figure, level with it.
                        let (x0, x1) = (hero_right + 0.03 * w, 0.95 * w);
                        let short = w.min(h);
                        if let Some(mut c) =
                            figure_card(ctx, b.id("prop_figure"), s, x1 - x0, 0.07 * short)
                        {
                            c.x = ((x0 + x1) / 2.0)
                                .clamp(0.05 * w + c.width / 2.0, x1 - c.width / 2.0);
                            c.y = 0.57 * h;
                            push_card(b, "prop_figure", c, s, t + 0.9);
                        }
                    } else {
                        place_card(ctx, b, "prop_figure", card, &id, 0.0, s, t + 0.9);
                    }
                }
                if beside {
                    frame_figure(b, &id, w);
                }
            }
        }
    }

    title(ctx, b, t, title_max_w);

    bokeh_discs(ctx, b, t, &fields);
    Ok(())
}

/// The look's field colours (the accent and muted colours when the palette has none).
pub(super) fn field_colors(ctx: &Ctx) -> Vec<Color> {
    if ctx.palette.fields.is_empty() {
        vec![ctx.palette.accent, ctx.palette.muted]
    } else {
        ctx.palette.fields.clone()
    }
}

/// Three huge soft glow discs far behind everything ([`Z_GLOW`]).
pub(super) fn glow_discs(ctx: &Ctx, b: &mut B, fields: &[Color]) {
    let (w, h) = (ctx.w, ctx.h);
    for (i, (cx, cy)) in [
        (0.1 * w, 0.22 * h),
        (0.95 * w, 0.78 * h),
        (0.5 * w, 1.15 * h),
    ]
    .into_iter()
    .enumerate()
    {
        let id = b.id(&format!("glow.{i}"));
        let mut g = disc(
            id.clone(),
            (cx, cy),
            1.5 * w,
            fields[i % fields.len()].with_alpha(0x2C),
            1,
            Z_GLOW,
        );
        g.opacity = 1.0;
        b.motions
            .push(mo::fade(&id, 0.0, 1.2, 0.0, 1.0, Easing::OutCubic));
        b.stage.push(g);
    }
}

/// The beat statement as the display title, just in front of the focus plane
/// ([`Z_TITLE`]): at most three lines of at most `max_w` px near the top, a
/// glyph cascade entering at `t`. Returns the canvas y of the title's bottom
/// edge.
pub(super) fn title(ctx: &Ctx, b: &mut B, t: f64, max_w: f32) -> f32 {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let seed = b.plan.seed;
    let title = ctx.ts.fit_block(
        Voice::HEADLINE,
        &b.beat.statement,
        max_w,
        0.17 * h,
        120.0 * u,
        3,
    );
    let title_id = b.id("title");
    // (0.20) A title that states the number or keyword waits for it under
    // `TitleReveal::Hold`; a topic title has no key words and enters now.
    b.reveal(
        "title",
        title_key_words(b.beat, &b.beat.statement),
        RevealRole::Title,
    );
    let mut tl = text_layer(title_id.clone(), &title, ctx.palette.ink, TextAlign::Center);
    tl.x = (w - tl.width) / 2.0;
    tl.y = 0.13 * h;
    tl.z_index = 20;
    tl.z = Some(Z_TITLE);
    let bottom = tl.y + tl.height;
    b.motions.push(sprung(
        Motion {
            id: None,
            target: title_id.clone(),
            start: t + 0.05,
            duration: 0.8,
            easing: Easing::OutQuint,
            spring: None,
            op: MotionOp::GlyphCascade {
                stagger: 0.025,
                from: GlyphPose {
                    dx: 0.0,
                    dy: -40.0 * u,
                    scale: 1.0,
                    rotation: 0.0,
                    opacity: 0.0,
                },
                order: GlyphOrder::Forward,
                seed: seed as u32,
            },
        },
        SETTLE,
    ));
    b.stage.push(tl);
    bottom
}

/// Three big, close, blurred foreground discs ([`Z_BOKEH`]).
pub(super) fn bokeh_discs(ctx: &Ctx, b: &mut B, t: f64, fields: &[Color]) {
    let (w, h) = (ctx.w, ctx.h);
    for (i, (fx, fy, fd)) in [
        (-0.02 * w, 0.18 * h, 0.42 * w),
        (1.04 * w, 0.84 * h, 0.5 * w),
        (0.18 * w, 1.0 * h, 0.36 * w),
    ]
    .into_iter()
    .enumerate()
    {
        let id = b.id(&format!("bokeh.{i}"));
        let c = if i == 1 {
            ctx.palette.accent
        } else {
            fields[(i + 1) % fields.len()]
        };
        let l = disc(id.clone(), (fx, fy), fd, c.with_alpha(0x34), 30, Z_BOKEH);
        b.motions
            .push(mo::fade(&id, t + 0.2, 0.9, 0.0, 1.0, Easing::OutCubic));
        b.stage.push(l);
    }
}
