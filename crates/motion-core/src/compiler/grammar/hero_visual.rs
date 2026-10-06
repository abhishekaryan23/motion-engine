//! (0.14) HeroVisual — street / music / sports: the picture owns the frame.
//!
//! A wheat-pasted poster: the delivered environment photo full-bleed with a
//! slow push, a giant stencil punchword BEHIND the picture (decorative
//! `ghost_punch`, glyphs cascading in on a spring), the hero picture at about
//! three quarters of the layout height leaning a few degrees and springing up
//! into place, two tape strips across its corners, and the keyword on a paper
//! sticker that pops at EVOLVE. Spec: docs/GENRE_GRAMMARS.md. Finishing
//! effects (grain, shake, flashes, echo, pulse) come from the FX director.

use super::placement::{self, Rect, SubjectFacts};
use super::{plate, Composition};
use crate::assets::{AssetRole, ManifestEntry};
use crate::compiler::art_direction::HeroStyle;
use crate::compiler::recipes::{self, Entrance, Slot, B};
use crate::compiler::subject_rules::STICKER_WIDTH_U;
use crate::compiler::taste_rules::{FRAME_PAD_MAX, TEXT_OVER_SUBJECT_MAX};
use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mix, mo, treatment, Carry, CompileError, Ctx, Which};
use crate::easing::Easing;
use crate::intent::{Subject, SubjectKind};
use crate::scene::{
    Color, Direction, GlyphOrder, GlyphPose, LayerKind, Motion, MotionOp, SpringSpec, Sticker,
    TextAlign,
};

/// Picture height as a share of the layout height.
const HERO_HEIGHT: f32 = 0.74;
/// Picture width cap as a share of the layout width.
const HERO_WIDTH: f32 = 0.88;
/// Studio (disc) picture height: the subject may overlap the words.
const STUDIO_HERO_HEIGHT: f32 = 0.78;
/// Stencil punchword: width share, height budget share.
const PUNCH_WIDTH: f32 = 0.8;
const PUNCH_HEIGHT: f32 = 0.34;

const POP: SpringSpec = SpringSpec {
    stiffness: 300.0,
    damping: 18.0,
    mass: 1.0,
};
const SLAM: SpringSpec = SpringSpec {
    stiffness: 420.0,
    damping: 22.0,
    mass: 1.0,
};

fn lightness(c: Color) -> u32 {
    u32::from(c.r) + u32::from(c.g) + u32::from(c.b)
}

fn sprung(mut m: Motion, s: SpringSpec) -> Motion {
    m.spring = Some(s);
    m
}

/// The words the poster shouts: the primary phrase, else (0.22) the figure of
/// the pictured object (picture + figure = a stat card), else the keyword,
/// else the first two words of the statement.
fn punchword(b: &B) -> String {
    let beat = b.beat;
    let phrase = match &beat.primary {
        s if s.kind() != SubjectKind::Object => s.value().map(str::to_string),
        _ => None,
    };
    let figure = || {
        object_subject(b)
            .and_then(|(_, s)| s.value().map(str::trim).map(str::to_string))
            .filter(|v| !v.is_empty())
    };
    phrase
        .or_else(figure)
        .or_else(|| beat.keyword.clone())
        .unwrap_or_else(|| {
            beat.statement
                .split_whitespace()
                .take(2)
                .collect::<Vec<_>>()
                .join(" ")
        })
}

/// The delivered picture for the hero, with the manifest role it is filed under.
fn delivered_hero<'a>(ctx: &Ctx<'a>, index: usize) -> Option<(&'a ManifestEntry, AssetRole)> {
    [
        AssetRole::HeroSubject,
        AssetRole::Portrait,
        AssetRole::HeroObject,
    ]
    .into_iter()
    .find_map(|role| plate::image(ctx, index, role).map(|e| (e, role)))
}

/// The object subject the hero falls back to (library picture or text).
fn object_subject(b: &B) -> Option<(Which, Subject)> {
    let beat = b.beat;
    if beat.primary.kind() == SubjectKind::Object {
        return Some((Which::Primary, beat.primary.clone()));
    }
    beat.secondary
        .as_ref()
        .filter(|s| s.kind() == SubjectKind::Object)
        .map(|s| (Which::Secondary, s.clone()))
}

/// The box (canvas) of what a delivered or library picture depicts: its
/// alpha bounds, drawn inside the layer box `layer` (`Fit::Contain`).
fn depicted(entry: &ManifestEntry, layer: &Rect) -> Rect {
    let (iw, ih) = (entry.width.max(1) as f32, entry.height.max(1) as f32);
    let s = (layer.w / iw).min(layer.h / ih);
    let (dw, dh) = (iw * s, ih * s);
    let drawn = Rect::new(
        layer.x + (layer.w - dw) / 2.0,
        layer.y + (layer.h - dh) / 2.0,
        dw,
        dh,
    );
    placement::to_canvas(&drawn, SubjectFacts::from_entry(entry).subject)
}

/// (0.23 A5a) Give the hero picture `id` (a layer of this beat, or the shared
/// element a carry holds) the contrast guard against `ground`.
fn guard_picture(
    ctx: &Ctx,
    b: &mut B,
    carries: &mut [Carry],
    id: &str,
    entry: &ManifestEntry,
    ground: Color,
) {
    let layer = match b.stage.iter_mut().find(|l| l.id == id) {
        Some(l) => Some(l),
        None => carries
            .iter_mut()
            .map(|c| &mut c.element.layer)
            .find(|l| l.id == id),
    };
    if let Some(l) = layer {
        treatment::guard_on_ground(l, entry, ground, &ctx.palette, ctx.u);
    }
}

/// A card holds a subject for the frame rule (`subject_qa`) when it covers at
/// least this share of the subject's bounds.
const FRAME_HOLDS: f32 = 0.6;
/// How far the studio disc may exceed the subject it holds, as a share of the
/// subject per side: under `FRAME_PAD_MAX`, so the rule has room for rotation.
const DISC_HUG: f32 = 0.10;

/// (0.23 A5a) The studio disc's box behind `subject`: the circle itself, unless
/// the circle holds the subject (covers 60 % of it) and exceeds it by more than
/// the frame rule allows (`frame_too_loose`). Then the circle is cut to the
/// subject's box grown by [`DISC_HUG`] on each side, and drawn as a stadium (a
/// rounded box with radius half its short side): the disc hugs a narrow rack
/// or a wide ringed planet instead of floating 20-40 % beyond it.
fn hugged_disc(circle: Rect, subject: Rect) -> Rect {
    let overlap = circle.intersect(&subject).map_or(0.0, |i| i.area());
    if subject.area() <= 0.0 || overlap < FRAME_HOLDS * subject.area() {
        return circle;
    }
    let pad = [
        (subject.x - circle.x) / subject.w,
        (circle.right() - subject.right()) / subject.w,
        (subject.y - circle.y) / subject.h,
        (circle.bottom() - subject.bottom()) / subject.h,
    ]
    .into_iter()
    .fold(f32::MIN, f32::max);
    if pad <= FRAME_PAD_MAX + 0.01 {
        return circle;
    }
    let grown = Rect::new(
        subject.x - DISC_HUG * subject.w,
        subject.y - DISC_HUG * subject.h,
        subject.w * (1.0 + 2.0 * DISC_HUG),
        subject.h * (1.0 + 2.0 * DISC_HUG),
    );
    circle.intersect(&grown).unwrap_or(circle)
}

/// Whether `(px, py)` lies in the rounded box `r` of corner radius `radius`
/// (what `subject_qa` asks of the card under a subject to find its ground).
fn rounded_holds(r: &Rect, radius: f32, px: f32, py: f32) -> bool {
    if px < r.x || px > r.right() || py < r.y || py > r.bottom() {
        return false;
    }
    let rad = radius.clamp(0.0, 0.5 * r.w.min(r.h));
    let ordered = |v: f32, a: f32, b: f32| v.clamp(a.min(b), a.max(b));
    let cx = ordered(px, r.x + rad, r.right() - rad);
    let cy = ordered(py, r.y + rad, r.bottom() - rad);
    (px - cx).hypot(py - cy) <= rad + 0.5
}

pub(crate) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    _c: Composition,
) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let life = plan.life;
    let t = plan.enter_at();
    let lean = if mix(plan.seed, 0x51EE).is_multiple_of(2) {
        1.0
    } else {
        -1.0
    };
    let land = placement::side_by_side(&ctx.frame);
    let disc = ctx
        .art
        .as_ref()
        .is_some_and(|(a, _)| a.fx.hero_style == HeroStyle::Disc);
    let lean = if disc { 0.0 } else { lean };
    // Street on a light palette: a paper poster (radial web, red banner
    // headline, two pictures when the beat has an object and a person).
    let poster = !disc && lightness(ctx.palette.paper) > 600;
    let person = if poster {
        [AssetRole::HeroSubject, AssetRole::Portrait]
            .into_iter()
            .find_map(|r| plate::image(ctx, plan.index, r).map(|e| (e, r)))
    } else {
        None
    };
    let duo = poster && person.is_some() && object_subject(b).is_some();
    // (0.20) The hero picture arrives when its subject is named.
    if let Some((_, subject)) = object_subject(b) {
        b.reveal(
            "hero",
            recipes::subject_words(&subject),
            crate::speech::RevealRole::Content,
        );
    }
    if poster {
        web_pattern(ctx, b, (w / 2.0, if duo { 0.3 * h } else { 0.42 * h }), t);
    }

    // 1. Environment photo, full bleed, slow push (street only: the studio
    //    look keeps its clean ground).
    if let Some(env) =
        plate::image(ctx, plan.index, AssetRole::Environment).filter(|_| !disc && !poster)
    {
        let (iw, ih) = (env.width.max(1) as f32, env.height.max(1) as f32);
        let s = (w * 1.04 / iw).max(h * 1.04 / ih);
        let (bw, bh) = (iw * s, ih * s);
        let rect = ((w - bw) / 2.0, (h - bh) / 2.0, bw, bh);
        let env_plate = plate::image_plate(ctx, b, "env", AssetRole::Environment, rect, 1, t);
        let id = env_plate.id.clone();
        b.motions
            .retain(|mo| !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. })));
        b.motions
            .push(mo::fade(&id, t, 0.6, 0.0, 1.0, Easing::OutCubic));
        b.motions.push(mo::scale(
            &id,
            t,
            plan.duration.max(0.5),
            1.0,
            1.08,
            Easing::Linear,
        ));
        for mut l in env_plate.layers {
            l.opacity = 0.5;
            l.depth = Some(0.2);
            b.push(l);
        }
    }

    // 2. Stencil punchword behind the picture.
    let punch = punchword(b);
    // Fit, then shrink until even the longest unbreakable word fits (the
    // camera push and the read scale magnify it a little more).
    let mut size = 420.0 * u;
    let punch_height = if disc { 0.22 } else { PUNCH_HEIGHT };
    let mut block = ctx.ts.fit_block(
        Voice::HEADLINE,
        &punch,
        PUNCH_WIDTH * w,
        punch_height * h,
        size,
        2,
    );
    for _ in 0..6 {
        let bw = block.width();
        if bw <= PUNCH_WIDTH * w || size < 40.0 * u {
            break;
        }
        size *= (PUNCH_WIDTH * w / bw) * 0.97;
        block = ctx.ts.fit_block(
            Voice::HEADLINE,
            &punch,
            PUNCH_WIDTH * w,
            punch_height * h,
            size,
            2,
        );
    }
    let punch_id = b.id("ghost_punch");
    let mut punch_layer = text_layer(
        punch_id.clone(),
        &block,
        ctx.palette.accent,
        TextAlign::Center,
    );
    punch_layer.x = (w - punch_layer.width) / 2.0;
    punch_layer.y = if disc {
        // Around the subject's head: the picture overlaps the words.
        (h - 0.6 * m) - STUDIO_HERO_HEIGHT * h - 0.01 * h
    } else if land {
        0.12 * h
    } else {
        0.17 * h
    };
    // Street: behind everything; studio: between the disc and the picture;
    // poster: on a red tape banner above everything.
    punch_layer.z_index = if disc {
        15
    } else if poster {
        25
    } else {
        5
    };
    // Background plane: the drift camera moves it less than the picture.
    punch_layer.depth = Some(if poster { 1.0 } else { 0.35 });
    if poster {
        if duo {
            punch_layer.y = 0.44 * h;
        }
        if let LayerKind::Text(ts) = &mut punch_layer.kind {
            ts.color = ctx.palette.paper;
        }
        // Rotate the headline with its banner about their shared centre.
        let (bw, bh) = (punch_layer.width, punch_layer.height);
        punch_layer.anchor_x = 0.5;
        punch_layer.anchor_y = 0.5;
        punch_layer.x += bw / 2.0;
        punch_layer.y += bh / 2.0;
        punch_layer.rotation_degrees = -3.0;
        let pad = 0.06 * bh + 18.0 * u;
        let banner_id = b.id("banner");
        let mut banner = base_layer(
            banner_id.clone(),
            (punch_layer.x, punch_layer.y, bw + 2.0 * pad, bh + pad),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.accent,
                radius: 4.0 * u,
                stroke: None,
            },
            24,
        );
        banner.anchor_x = 0.5;
        banner.anchor_y = 0.5;
        banner.rotation_degrees = -3.0;
        b.motions.push(sprung(
            mo::scale(&banner_id, t, 0.5, 1.6, 1.0, Easing::OutQuint),
            SLAM,
        ));
        b.motions
            .push(mo::fade(&banner_id, t, 0.12, 0.0, 1.0, Easing::OutCubic));
        b.push(banner);
    }
    let punch_bottom =
        punch_layer.y - punch_layer.anchor_y * punch_layer.height + punch_layer.height;
    let punch_h = punch_layer.height;
    b.push(punch_layer);
    b.motions.push(sprung(
        Motion {
            id: None,
            target: punch_id.clone(),
            start: t,
            duration: 0.9,
            easing: Easing::OutQuint,
            spring: None,
            op: MotionOp::GlyphCascade {
                stagger: 0.035,
                from: GlyphPose {
                    dx: 0.0,
                    dy: -120.0 * u,
                    scale: 1.6,
                    rotation: 8.0,
                    opacity: 0.0,
                },
                order: GlyphOrder::Random,
                seed: plan.seed as u32,
            },
        },
        SLAM,
    ));
    let span = (life.anticipate - life.read).max(0.0);
    if span > 0.2 {
        b.motions.push(mo::scale(
            &punch_id,
            life.read,
            span,
            1.0,
            1.04,
            Easing::InOutCubic,
        ));
    }

    // 3a. Poster with two pictures: the object comes down from the top, the
    //     person stands at the bottom.
    let enter = t + 0.15;
    let bottom = h - 0.6 * m;
    if duo {
        if let (Some((which, subject)), Some((pe, prole))) = (object_subject(b), person) {
            let top_box = Rect::new(0.08 * w, 0.06 * h, 0.84 * w, 0.4 * h);
            let drop = [0.0, -0.45 * h];
            if let Some((entry, role)) = plate::image(ctx, plan.index, AssetRole::HeroObject)
                .map(|e| (e, AssetRole::HeroObject))
            {
                let img = placement::subject_fit(&SubjectFacts::from_entry(entry), top_box);
                let top = plate::image_plate(
                    ctx,
                    b,
                    "hero_top",
                    role,
                    (img.x, img.y, img.w, img.h),
                    20,
                    enter,
                );
                let id = top.id.clone();
                b.motions.retain(|mo| {
                    !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. }))
                });
                b.motions.push(sprung(
                    mo::shift(&id, enter, 0.9, drop, [0.0, 0.0], Easing::OutQuint),
                    POP,
                ));
                for l in top.layers {
                    b.push(l);
                }
            } else {
                let slot = Slot {
                    cx: top_box.x + top_box.w / 2.0,
                    cy: top_box.y + top_box.h / 2.0,
                    w: top_box.w,
                    h: top_box.h,
                };
                if let Some(mut l) = recipes::place_subject(
                    ctx,
                    b,
                    carries,
                    which,
                    &subject,
                    slot,
                    (0.0, 0.0),
                    ctx.palette.ink,
                    enter,
                    Entrance::Pop,
                    None,
                    None,
                    "hero_top",
                )? {
                    l.z_index = 20;
                    let lid = l.id.clone();
                    b.motions.retain(|mo| {
                        !(mo.target == lid
                            && matches!(
                                mo.op,
                                MotionOp::Scale { .. } | MotionOp::ClipReveal { .. }
                            ))
                    });
                    b.motions.push(sprung(
                        mo::shift(&lid, enter, 0.9, drop, [0.0, 0.0], Easing::OutQuint),
                        POP,
                    ));
                    b.push(l);
                }
            }
            let ph = 0.34 * h;
            let low_box = Rect::new(0.2 * w, bottom - ph, 0.6 * w, ph);
            let facts = SubjectFacts::from_entry(pe);
            let img = placement::subject_fit(&facts, low_box);
            let sub = placement::to_canvas(&img, facts.subject);
            let dy = bottom - (sub.y + sub.h);
            let low = plate::image_plate(
                ctx,
                b,
                "hero",
                prole,
                (img.x, img.y + dy, img.w, img.h),
                21,
                enter + 0.3,
            );
            let id = low.id.clone();
            b.motions
                .retain(|mo| !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. })));
            b.motions
                .push(mo::fade(&id, enter + 0.3, 0.2, 0.0, 1.0, Easing::OutCubic));
            b.motions.push(sprung(
                mo::shift(
                    &id,
                    enter + 0.3,
                    0.7,
                    [0.0, 120.0 * u],
                    [0.0, 0.0],
                    Easing::OutQuint,
                ),
                POP,
            ));
            for l in low.layers {
                b.push(l);
            }
            starburst(
                ctx,
                b,
                (
                    w - 1.8 * m - 135.0 * u,
                    (0.62 * h).max(punch_bottom + 160.0 * u),
                ),
                life.evolve.max(enter + 0.5),
                None,
            );
        }
        return Ok(());
    }

    // 3. The hero picture: ~3/4 of the layout height, bottom on the content
    //    bottom, leaning a few degrees.
    let (box_w, box_h) = if land {
        (0.5 * w, 0.86 * h)
    } else if disc {
        (HERO_WIDTH * w, STUDIO_HERO_HEIGHT * h)
    } else {
        // The picture may overlap the stencil by at most 35 % of its height,
        // so the word still reads (captions shorten the layout canvas).
        let room = bottom - (punch_bottom - 0.35 * punch_h);
        (HERO_WIDTH * w, (HERO_HEIGHT * h).min(room.max(0.4 * h)))
    };
    let cx = if land {
        w / 2.0 + lean * 0.2 * w
    } else {
        w / 2.0 + lean * 0.03 * w
    };
    let target = Rect::new(cx - box_w / 2.0, bottom - box_h, box_w, box_h);
    let mut hero_box = (target.x, target.y, target.w, target.h);
    let rotation = lean * (1.5 + (mix(plan.seed, 0x2A) % 15) as f32 / 10.0);
    // (0.23 A5a) What the manifest knows about the picture: its entry, the box
    // of what it depicts on the canvas, and (a library object placed with no
    // treatment of its own) the id of the layer that needs a contrast guard.
    let mut depicts: Option<(&ManifestEntry, Rect)> = None;
    let mut unguarded: Option<String> = None;
    if let Some((entry, role)) = delivered_hero(ctx, plan.index) {
        let facts = SubjectFacts::from_entry(entry);
        let img = placement::subject_fit(&facts, target);
        // Bottom-align what the picture depicts with the content bottom.
        let sub = placement::to_canvas(&img, facts.subject);
        let dy = bottom - (sub.y + sub.h);
        let rect = (img.x, img.y + dy, img.w, img.h);
        hero_box = (sub.x, sub.y + dy, sub.w, sub.h);
        depicts = Some((entry, Rect::new(sub.x, sub.y + dy, sub.w, sub.h)));
        let hero = plate::image_plate(ctx, b, "hero", role, rect, 20, enter);
        let id = hero.id.clone();
        b.motions
            .retain(|mo| !(mo.target == id && matches!(mo.op, MotionOp::MaskReveal { .. })));
        b.motions
            .push(mo::fade(&id, enter, 0.18, 0.0, 1.0, Easing::OutCubic));
        b.motions.push(sprung(
            mo::scale(&id, enter, 0.7, 1.25, 1.0, Easing::OutQuint),
            POP,
        ));
        b.motions.push(sprung(
            mo::shift(
                &id,
                enter,
                0.7,
                [0.0, 140.0 * u],
                [0.0, 0.0],
                Easing::OutQuint,
            ),
            POP,
        ));
        // Street/zine sticker border on cutouts (also the contrast guard
        // when a manifest carries no colour analysis); the studio look keeps
        // clean edges.
        let light = if lightness(ctx.palette.paper) >= lightness(ctx.palette.ink) {
            ctx.palette.paper
        } else {
            ctx.palette.ink
        };
        for mut l in hero.layers {
            l.rotation_degrees += rotation;
            if entry.alpha && !disc {
                if let LayerKind::Image {
                    treatment: Some(tr),
                    ..
                } = &mut l.kind
                {
                    tr.sticker = Some(Sticker {
                        color: light,
                        width_px: STICKER_WIDTH_U * u,
                    });
                    tr.edge = None;
                    tr.shadow = None;
                }
            }
            b.push(l);
        }
    } else if let Some((which, subject)) = object_subject(b) {
        let slot = Slot {
            cx,
            cy: bottom - box_h / 2.0,
            w: box_w,
            h: box_h,
        };
        let placed = recipes::place_subject(
            ctx,
            b,
            carries,
            which,
            &subject,
            slot,
            (0.0, 0.0),
            ctx.palette.ink,
            enter,
            Entrance::Pop,
            None,
            None,
            "hero",
        )?;
        // (0.23 A5a) A picture the story carries from beat to beat is a shared
        // element, not a layer of this beat: its box is the carry's state.
        if placed.is_none() {
            let shared = b.placed.last().and_then(|&ci| carries.get(ci));
            let entry = recipes::object_request_id(ctx, b, &subject)
                .and_then(|request| ctx.manifest.get(&request));
            if let (Some(c), Some(entry)) = (shared, entry) {
                let (cx, cy, k) = c.state;
                let (bw, bh) = (c.base_w * k, c.base_h * k);
                let layer_box = Rect::new(cx - bw / 2.0, cy - bh / 2.0, bw, bh);
                depicts = Some((entry, depicted(entry, &layer_box)));
                unguarded = Some(c.element.layer.id.clone());
            }
        }
        if let Some(mut l) = placed {
            l.z_index = 20;
            l.rotation_degrees += rotation;
            // The Pop entrance's own scale / reveal would conflict with the
            // spring below: HeroVisual owns this entrance.
            let lid = l.id.clone();
            b.motions.retain(|mo| {
                !(mo.target == lid
                    && matches!(mo.op, MotionOp::Scale { .. } | MotionOp::ClipReveal { .. }))
            });
            hero_box = (
                l.x - l.anchor_x * l.width,
                l.y - l.anchor_y * l.height,
                l.width,
                l.height,
            );
            // The library picture is drawn without a treatment, so nothing
            // guards its contrast against what it sits on (done below).
            if let Some(entry) = recipes::object_request_id(ctx, b, &subject)
                .and_then(|request| ctx.manifest.get(&request))
            {
                let layer_box = Rect::new(hero_box.0, hero_box.1, hero_box.2, hero_box.3);
                depicts = Some((entry, depicted(entry, &layer_box)));
                unguarded = Some(l.id.clone());
            }
            b.motions.push(sprung(
                mo::scale(&l.id, enter, 0.7, 1.25, 1.0, Easing::OutQuint),
                POP,
            ));
            b.push(l);
        }
    }

    let (hx, hy, hw, hh) = hero_box;
    let settle = life.settle.max(enter + 0.3);
    // (0.23 A5a) Off the studio disc a library picture sits on the page.
    if let (false, Some(id), Some((entry, _))) = (disc, &unguarded, depicts) {
        guard_picture(ctx, b, carries, id, entry, ctx.palette.paper);
    }
    if disc {
        // Studio: a bold disc behind the picture's upper body and a soft
        // floor shadow; no tape, no sticker (the words are the FX director's).
        let d = (0.78 * w).min(0.62 * hh).max(0.3 * w);
        let (dcx, dcy) = (hx + hw / 2.0, hy + 0.42 * hh);
        let fill = ctx
            .palette
            .fields
            .first()
            .copied()
            .unwrap_or(ctx.palette.accent);
        // (0.23 A5a) The disc hugs a picture it would float far beyond.
        let circle_box = Rect::new(dcx - d / 2.0, dcy - d / 2.0, d, d);
        let card = depicts.map_or(circle_box, |(_, subject)| hugged_disc(circle_box, subject));
        let radius = card.w.min(card.h) / 2.0;
        // (0.23 A5a) A library picture has no contrast guard of its own: the
        // ground it sits on is the disc where the disc holds its middle (the
        // rule `asset_low_contrast` applies), else the page.
        if let (Some(id), Some((entry, subject))) = (&unguarded, depicts) {
            let ground = if rounded_holds(&card, radius, subject.cx(), subject.y + subject.h / 2.0)
            {
                fill
            } else {
                ctx.palette.paper
            };
            guard_picture(ctx, b, carries, id, entry, ground);
        }
        let disc_id = b.id("disc");
        let mut circle = base_layer(
            disc_id.clone(),
            (card.cx(), card.y + card.h / 2.0, card.w, card.h),
            LayerKind::RoundedRectangle {
                fill,
                radius,
                stroke: None,
            },
            8,
        );
        circle.anchor_x = 0.5;
        circle.anchor_y = 0.5;
        b.motions.push(sprung(
            mo::scale(&disc_id, t, 0.6, 0.0, 1.0, Easing::OutQuint),
            SpringSpec {
                stiffness: 260.0,
                damping: 20.0,
                mass: 1.0,
            },
        ));
        b.push(circle);
        let shadow_id = b.id("floor_shadow");
        let (sw, sh) = (0.62 * hw.max(0.3 * w), 0.032 * h);
        let mut shadow = base_layer(
            shadow_id.clone(),
            (dcx, hy + hh - 0.2 * sh, sw, sh),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.ink.with_alpha(0x33),
                radius: sh / 2.0,
                stroke: None,
            },
            18,
        );
        shadow.anchor_x = 0.5;
        shadow.anchor_y = 0.5;
        b.motions
            .push(mo::fade(&shadow_id, enter, 0.4, 0.0, 1.0, Easing::OutCubic));
        b.push(shadow);
        return Ok(());
    }

    if poster {
        starburst(
            ctx,
            b,
            (
                w - 1.8 * m - 135.0 * u,
                (hy + 0.3 * hh).max(punch_bottom + 110.0 * u),
            ),
            life.evolve.max(settle + 0.2),
            depicts.map(|(_, subject)| subject),
        );
        return Ok(());
    }

    // 4. Tape strips across the picture's top and bottom corners.
    let tape = ctx
        .palette
        .fields
        .first()
        .copied()
        .unwrap_or(ctx.palette.accent);
    let (tw, th) = (260.0 * u, 54.0 * u);
    for (i, (px, py, rot, dir)) in [
        (
            hx + 0.12 * hw,
            hy + 0.06 * hh,
            -24.0 * lean,
            Direction::Right,
        ),
        (hx + 0.88 * hw, hy + 0.84 * hh, 20.0 * lean, Direction::Left),
    ]
    .into_iter()
    .enumerate()
    {
        let id = b.id(&format!("tape.{i}"));
        let mut strip = base_layer(
            id.clone(),
            (px.clamp(m, w - m), py.clamp(m, h - m), tw, th),
            LayerKind::RoundedRectangle {
                fill: tape,
                radius: 2.0 * u,
                stroke: None,
            },
            24,
        );
        strip.anchor_x = 0.5;
        strip.anchor_y = 0.5;
        strip.rotation_degrees = rot;
        strip.opacity = 0.92;
        b.motions.push(mo::mask(
            &id,
            settle + 0.12 * i as f64,
            0.35,
            dir,
            Easing::OutCubic,
        ));
        b.push(strip);
    }

    // 5. Keyword sticker beside the picture's upper third, popping at EVOLVE.
    //    (0.22) With a voice-over only a keyword the narrator says.
    if let Some(kw) = b
        .beat
        .keyword
        .clone()
        .filter(|k| !k.trim().is_empty() && super::dossier::stamp_allowed(ctx, k))
    {
        let label = ctx.ts.fit_line(Voice::LABEL, &kw, 0.4 * w, 46.0 * u);
        let pad = 18.0 * u;
        let (sw, sh) = (label.width() + 2.0 * pad, label.height() + 1.4 * pad);
        let sx = if lean > 0.0 {
            1.8 * m + sw / 2.0
        } else {
            w - 1.8 * m - sw / 2.0
        };
        let sy = (hy + 0.18 * hh).max(punch_bottom + sh);
        let pop_at = life.evolve.max(settle + 0.2);
        let sticker_id = b.id("keytag");
        // (0.20) The keyword sticker pops when the keyword is said.
        b.reveal("keytag", [kw.clone()], crate::speech::RevealRole::Label);
        let mut sticker = base_layer(
            sticker_id.clone(),
            (sx, sy, sw, sh),
            LayerKind::RoundedRectangle {
                fill: ctx.palette.paper,
                radius: 6.0 * u,
                stroke: None,
            },
            26,
        );
        sticker.anchor_x = 0.5;
        sticker.anchor_y = 0.5;
        sticker.rotation_degrees = -6.0 * lean;
        let text_id = b.id("keytag.text");
        let mut text = text_layer(text_id.clone(), &label, ctx.palette.ink, TextAlign::Center);
        text.anchor_x = 0.5;
        text.anchor_y = 0.5;
        text.x = sx;
        text.y = sy;
        text.z_index = 27;
        text.rotation_degrees = -6.0 * lean;
        for id in [&sticker_id, &text_id] {
            b.motions.push(sprung(
                mo::scale(id, pop_at, 0.5, 0.0, 1.0, Easing::OutQuint),
                POP,
            ));
        }
        b.push(sticker);
        b.push(text);
    }
    Ok(())
}

/// (Poster) A radial web drawn in the accent: 12 spokes and 5 slightly
/// irregular rings around `center`, drawing on from ENTER.
fn web_pattern(ctx: &Ctx, b: &mut B, center: (f32, f32), t: f64) {
    let (w, h, u) = (ctx.w, ctx.h, ctx.u);
    let r_max = 0.95 * w.max(0.6 * h);
    let color = ctx.palette.accent.with_alpha(0x8C);
    let stroke = crate::scene::Stroke {
        color,
        width: 3.0 * u,
    };
    let spokes = 12;
    let seed = b.plan.seed;
    let jitter = |k: u64| 1.0 + ((mix(seed, 0x3EB0 + k) % 1000) as f32 / 1000.0 - 0.5) * 0.12;
    for k in 0..spokes {
        let a = std::f32::consts::TAU * k as f32 / spokes as f32 + 0.13;
        let id = b.id(&format!("web.spoke{k}"));
        let pts = vec![
            [center.0, center.1],
            [center.0 + r_max * a.cos(), center.1 + r_max * a.sin()],
        ];
        let l = base_layer(
            id.clone(),
            (0.0, 0.0, w, h),
            LayerKind::Polyline {
                points: pts,
                stroke: stroke.clone(),
                closed: false,
                fill: None,
            },
            2,
        );
        b.motions.push(mo_trim(&id, t + 0.03 * k as f64, 0.7));
        b.push(l);
    }
    for ring in 1..=5 {
        let r = r_max * ring as f32 / 6.0;
        let id = b.id(&format!("web.ring{ring}"));
        let pts: Vec<[f32; 2]> = (0..spokes)
            .map(|k| {
                let a = std::f32::consts::TAU * k as f32 / spokes as f32 + 0.13;
                let rr = r * jitter((ring * 31 + k) as u64);
                [center.0 + rr * a.cos(), center.1 + rr * a.sin()]
            })
            .collect();
        let l = base_layer(
            id.clone(),
            (0.0, 0.0, w, h),
            LayerKind::Polyline {
                points: pts,
                stroke: stroke.clone(),
                closed: true,
                fill: None,
            },
            2,
        );
        b.motions
            .push(mo_trim(&id, t + 0.25 + 0.08 * ring as f64, 0.8));
        b.push(l);
    }
}

fn mo_trim(target: &str, start: f64, duration: f64) -> Motion {
    Motion {
        id: None,
        target: target.to_string(),
        start,
        duration,
        easing: Easing::OutCubic,
        spring: None,
        op: MotionOp::Trim { from: 0.0, to: 1.0 },
    }
}

/// (Poster) The keyword in a comic starburst (16 points, field colour, ink
/// keyline) that pops at `at`. (0.22) With a voice-over only a keyword the
/// narrator says in the beat. (0.23 A5a) `subject` is the box of what the
/// beat's picture depicts: a keyword that would cover more of it than
/// `TEXT_OVER_SUBJECT_MAX` (the `text_over_subject` rule) is set smaller until
/// it covers 3 % at most, down to 36 u; the burst is not a card, so its text
/// counts as type over the picture.
fn starburst(ctx: &mut Ctx, b: &mut B, center: (f32, f32), at: f64, subject: Option<Rect>) {
    let Some(kw) = b
        .beat
        .keyword
        .clone()
        .filter(|k| !k.trim().is_empty() && super::dossier::stamp_allowed(ctx, k))
    else {
        return;
    };
    let u = ctx.u;
    let mut size = 58.0 * u;
    let mut label = ctx.ts.fit_line(Voice::HEADLINE, &kw, 230.0 * u, size);
    if let Some(subject) = subject.filter(|s| s.area() > 0.0) {
        // The text box as the rule measures it: the layer width, the ink rows,
        // turned -8 degrees about its middle.
        let covers = |label: &crate::compiler::typeset::Block| {
            let (lw, lh) = (
                label.width() * 1.02 + 2.0,
                ctx.ts
                    .ink(label)
                    .map_or(label.height(), |ink| (ink.bottom - ink.top).max(0.0)),
            );
            let (sin, cos) = 8.0_f32.to_radians().sin_cos();
            let (aw, ah) = (lw * cos + lh * sin, lw * sin + lh * cos);
            let text = Rect::new(center.0 - aw / 2.0, center.1 - ah / 2.0, aw, ah);
            text.intersect(&subject).map_or(0.0, |i| i.area()) / subject.area()
        };
        if covers(&label) > TEXT_OVER_SUBJECT_MAX {
            while covers(&label) > 0.75 * TEXT_OVER_SUBJECT_MAX && size > 36.0 * u {
                size = (size * 0.92).max(36.0 * u);
                label = ctx.ts.fit_line(Voice::HEADLINE, &kw, 230.0 * u, size);
            }
        }
    }
    // (0.20) The starburst slams on the keyword.
    b.reveal("burst", [kw.clone()], crate::speech::RevealRole::Stamp);
    let r = (label.width() * 0.72).max(135.0 * u);
    let pts: Vec<[f32; 2]> = (0..32)
        .map(|k| {
            let a = std::f32::consts::TAU * k as f32 / 32.0;
            let rr = if k % 2 == 0 { r } else { 0.74 * r };
            [r + rr * a.cos(), r + rr * a.sin()]
        })
        .collect();
    let fill = ctx
        .palette
        .fields
        .iter()
        .copied()
        .find(|c| lightness(*c) > 400)
        .unwrap_or(Color {
            r: 0xFF,
            g: 0xD4,
            b: 0x00,
            a: 0xFF,
        });
    let id = b.id("burst");
    let mut burst = base_layer(
        id.clone(),
        (center.0, center.1, 2.0 * r, 2.0 * r),
        LayerKind::Polyline {
            points: pts,
            stroke: crate::scene::Stroke {
                color: ctx.palette.ink,
                width: 3.0 * u,
            },
            closed: true,
            fill: Some(fill),
        },
        27,
    );
    burst.anchor_x = 0.5;
    burst.anchor_y = 0.5;
    burst.rotation_degrees = -8.0;
    let text_id = b.id("burst.text");
    let mut text = text_layer(text_id.clone(), &label, ctx.palette.ink, TextAlign::Center);
    text.anchor_x = 0.5;
    text.anchor_y = 0.5;
    text.x = center.0;
    text.y = center.1;
    text.z_index = 28;
    text.rotation_degrees = -8.0;
    for id in [&id, &text_id] {
        b.motions.push(sprung(
            mo::scale(id, at, 0.5, 0.0, 1.0, Easing::OutQuint),
            POP,
        ));
    }
    b.push(burst);
    b.push(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(card: &Rect, s: &Rect) -> f32 {
        [
            (s.x - card.x) / s.w,
            (card.right() - s.right()) / s.w,
            (s.y - card.y) / s.h,
            (card.bottom() - s.bottom()) / s.h,
        ]
        .into_iter()
        .fold(f32::MIN, f32::max)
    }

    #[test]
    fn the_disc_hugs_a_narrow_subject_it_would_float_beyond() {
        // A 330 px circle behind a 180 x 420 rack: 38 % beyond it on a side.
        let circle = Rect::new(375.0, 400.0, 330.0, 330.0);
        let rack = Rect::new(450.0, 360.0, 180.0, 420.0);
        assert!(pad(&circle, &rack) > FRAME_PAD_MAX + 0.01);
        let card = hugged_disc(circle, rack);
        assert!(pad(&card, &rack) <= FRAME_PAD_MAX, "{card:?}");
        // It stays inside the circle it was cut from and keeps most of it.
        assert!(card.w <= circle.w && card.h <= circle.h);
        assert!(card.w >= 0.9 * rack.w && card.h >= 0.7 * circle.h);
    }

    #[test]
    fn a_disc_that_fits_or_does_not_hold_the_subject_stays_a_circle() {
        let circle = Rect::new(300.0, 300.0, 400.0, 400.0);
        // The subject fills the circle: the pad is small.
        assert_eq!(
            hugged_disc(circle, Rect::new(290.0, 295.0, 420.0, 410.0)),
            circle
        );
        // A subject far larger than the disc: the disc holds under 60 % of it.
        assert_eq!(
            hugged_disc(circle, Rect::new(50.0, 50.0, 900.0, 900.0)),
            circle
        );
    }

    #[test]
    fn a_round_card_holds_the_middle_but_not_its_corners() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert!(rounded_holds(&r, 50.0, 50.0, 50.0));
        assert!(!rounded_holds(&r, 50.0, 2.0, 2.0));
        assert!(rounded_holds(&r, 0.0, 2.0, 2.0));
        assert!(!rounded_holds(&r, 50.0, 150.0, 50.0));
    }
}
