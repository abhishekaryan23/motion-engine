//! Image plates: a delivered image (AssetManifest) or a procedural placeholder
//! in the same rectangle. The TypeImageInterlock foundation.

use crate::assets::{AssetRole, ManifestEntry};
use crate::compiler::recipes::{self, B};
use crate::compiler::typeset::{text_layer, Voice};
use std::collections::BTreeSet;

use crate::compiler::{
    base_layer, mix, mo, slug, subject_rules, texture_layer, track_key, treatment, wants_carry,
    BeatPlan, Carry, Ctx, Which,
};
use crate::easing::Easing;
use crate::intent::{Beat, Subject, SubjectKind};
use crate::scene::{
    Asset, AssetKind, Direction, Fit, KeyState, Layer, LayerKind, Material, Motion, MotionOp,
    SharedElement, Stroke, TextAlign, TextureSpec,
};

/// Request id the asset planner uses for `role` in beat `index` (0-based).
pub(crate) fn request_id(index: usize, role: AssetRole) -> String {
    let role = serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    format!("beat_{}.{role}", index + 1)
}

/// The delivered image for `role` in beat `index`, if the manifest has one.
pub(crate) fn image<'a>(ctx: &Ctx<'a>, index: usize, role: AssetRole) -> Option<&'a ManifestEntry> {
    ctx.manifest.get(&request_id(index, role))
}

/// A delivered hero-subject or portrait image for beat `index`.
pub(crate) fn subject_image<'a>(ctx: &Ctx<'a>, index: usize) -> Option<&'a ManifestEntry> {
    image(ctx, index, AssetRole::HeroSubject).or_else(|| image(ctx, index, AssetRole::Portrait))
}

/// An image region: the delivered image, or a procedural stand-in in the same box.
pub(crate) struct Plate {
    /// The main layer's id (the image, or the placeholder card).
    pub id: String,
    /// A manifest image was used (false = placeholder).
    pub delivered: bool,
    /// Layers in draw order; the caller pushes them (and may tag depth / tilt).
    pub layers: Vec<Layer>,
}

/// Place the image for `role` in `rect` (`x, y, w, h`, canvas space, top-left).
///
/// Delivered: an `image` layer (Fit::Contain, box sized to the image aspect and
/// centered in `rect`, anchored at the entry's `subject_anchor`) registered as
/// project asset `asset.<request id>`. Missing: a rounded card with a halftone
/// patch and a small mono label naming the role. Both reveal at `start`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn image_plate(
    ctx: &mut Ctx,
    b: &mut B,
    name: &str,
    role: AssetRole,
    rect: (f32, f32, f32, f32),
    z: i32,
    start: f64,
) -> Plate {
    let id = b.id(name);
    if let Some(entry) = image(ctx, b.plan.index, role) {
        let layer = image_layer(ctx, b, id.clone(), entry, role, rect, z);
        b.motions
            .push(mo::mask(&id, start, 0.85, Direction::Up, Easing::OutQuint));
        return Plate {
            id,
            delivered: true,
            layers: vec![layer],
        };
    }
    placeholder(ctx, b, name, role, rect, z, start)
}

/// The `image` layer for a delivered entry in `rect` (box sized to the image
/// aspect, centred in `rect`, anchored at the subject anchor), with the
/// style's treatment; registers the project asset `asset.<entry id>`.
pub(super) fn image_layer(
    ctx: &mut Ctx,
    b: &B,
    id: String,
    entry: &ManifestEntry,
    role: AssetRole,
    rect: (f32, f32, f32, f32),
    z: i32,
) -> Layer {
    let (x, y, w, h) = rect;
    let asset_id = format!("asset.{}", entry.id);
    ctx.assets.insert(
        asset_id.clone(),
        Asset {
            id: asset_id.clone(),
            kind: AssetKind::Image,
            path: entry.path.clone(),
            sprite: None,
        },
    );
    let (bw, bh) = if entry.width > 0 && entry.height > 0 {
        let s = (w / entry.width as f32).min(h / entry.height as f32);
        (entry.width as f32 * s, entry.height as f32 * s)
    } else {
        (w, h)
    };
    let (bx, by) = (x + (w - bw) / 2.0, y + (h - bh) / 2.0);
    let (ax, ay) = entry
        .subject_anchor
        .map(|p| (p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0)))
        .unwrap_or((0.5, 0.5));
    let mut tr = treatment::treatment_for(
        treatment::preset_for_taste(&ctx.taste, ctx.style, role),
        &ctx.palette,
        mix(b.plan.seed, 0x7EA7),
        ctx.u,
        entry.alpha,
    );
    // (0.9) Monochrome line-art cutouts are inked in the palette ink so they
    // stay legible on any ground.
    // People photos (hero subjects, portraits) are never line art, even in
    // black-and-white clothes.
    let person = matches!(role, AssetRole::HeroSubject | AssetRole::Portrait);
    if entry.alpha && !person && entry.analysis.as_ref().is_some_and(|a| a.monochrome) {
        tr = treatment::monochrome_ink(tr, &ctx.palette);
    }
    // (0.10 Q) A subject that would vanish into its ground (mean colour under
    // ASSET_GROUND_MIN_CONTRAST against the page, card stock or colour fields)
    // gets a sticker outline (cutouts) or a keyline (opaque images).
    tr = subject_rules::guard_contrast(
        tr,
        entry.analysis.as_ref().and_then(|a| a.mean_color),
        entry.alpha,
        &ctx.palette,
        ctx.u,
    );
    let mut layer = base_layer(
        id,
        (bx + ax * bw, by + ay * bh, bw, bh),
        LayerKind::Image {
            asset: asset_id,
            fit: Fit::Contain,
            treatment: Some(tr),
            playback: None,
            insert: None,
        },
        z,
    );
    layer.anchor_x = ax;
    layer.anchor_y = ay;
    layer
}

/// Continuity key of a delivered image carried across beats.
fn asset_carry_key(entry_id: &str) -> String {
    format!("asset:{entry_id}")
}

/// (0.5) Which beats hand their delivered subject image to the next beat:
/// both beats are TypeImageInterlock and are served by the SAME manifest
/// entry (one generated image, one continuity key). Pure function of the
/// intent, plans and manifest.
pub(crate) fn image_carries(ctx: &Ctx, plans: &[BeatPlan], beats: &[Beat]) -> BTreeSet<usize> {
    let interlock = |i: usize| -> Option<&str> {
        // Accent-wipe beats restack the stage above everything: no carry.
        if plans[i].accent_in || plans[i].accent_out {
            return None;
        }
        let entry = subject_image(ctx, i)?;
        let comp = super::select(super::SelectInput {
            beat: &beats[i],
            language: plans[i].lang.language,
            format: ctx.format,
            side_by_side: super::placement::side_by_side(&ctx.frame),
            seed: plans[i].seed,
            has_subject_image: true,
            has_object_image: image(ctx, i, AssetRole::HeroObject).is_some(),
            visual: &ctx.taste.visual,
            look_grammar: ctx.art.as_ref().and_then(|(a, _)| a.fx.grammar),
        });
        (comp.grammar == super::Grammar::TypeImageInterlock).then_some(entry.id.as_str())
    };
    (0..plans.len().saturating_sub(1))
        .filter(|&i| matches!((interlock(i), interlock(i + 1)), (Some(a), Some(b)) if a == b))
        .collect()
}

/// A subject image placed by [`place_subject_image`].
pub(crate) struct SubjectImage {
    /// Id to target with motions (a scene layer, or the shared element's layer).
    pub id: String,
    /// Scene layers to push (empty when the image lives in a SharedElement).
    pub layers: Vec<Layer>,
    /// The image is a SharedElement (carried in or carried forward): only
    /// shared-safe motions (move/scale/rotate/fade/count/tint), ending neutral.
    pub shared: bool,
}

/// Place the delivered subject image for `role` in `rect` (0.5 asset carry):
/// - the same image was carried in from the previous beat → the shared
///   element travels to `rect` (arrives with the entrance), no new layer;
/// - it continues into the next beat → it becomes a SharedElement here;
/// - otherwise → an ordinary plate ([`image_plate`], placeholder when missing).
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_subject_image(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    name: &str,
    role: AssetRole,
    rect: (f32, f32, f32, f32),
    z: i32,
    depth: Option<f32>,
    start: f64,
) -> SubjectImage {
    let plan = b.plan;
    let entry = image(ctx, plan.index, role);
    let carry_on = !plan.is_last && ctx.image_carry.contains(&plan.index);
    if let Some(entry) = entry {
        let key = asset_carry_key(&entry.id);
        let probe = image_layer(ctx, b, String::new(), entry, role, rect, z);
        // Carried in from the previous beat.
        if let Some(ci) = carries
            .iter()
            .position(|c| c.open && c.key == key && c.last_beat + 1 == plan.index)
        {
            let c = &mut carries[ci];
            let s = probe.width / c.base_w.max(1.0);
            let settle = plan.lang.preset;
            c.element.track.push(track_key(
                &plan.id,
                start + settle.duration * 0.6,
                "subject",
                settle.settle,
                KeyState {
                    x: Some(probe.x),
                    y: Some(probe.y),
                    scale: Some(s),
                    opacity: Some(1.0),
                    ..Default::default()
                },
            ));
            c.state = (probe.x, probe.y, s);
            c.last_beat = plan.index;
            c.open = carry_on;
            b.placed.push(ci);
            // Re-read at EVOLVE: a brief accent tint on the carried image (the
            // image analogue of the carried-subject pulse); ends neutral.
            let (life, target) = (plan.life, c.element.layer.id.clone());
            if life.anticipate - life.evolve > 1.1 {
                let accent = ctx.palette.accent;
                for (at, dur, from, to, easing) in [
                    (life.evolve, 0.35, 0.0, 0.18, Easing::OutCubic),
                    (life.evolve + 0.35, 0.6, 0.18, 0.0, Easing::InOutCubic),
                ] {
                    b.motions.push(Motion {
                        spring: None,
                        id: None,
                        target: target.clone(),
                        start: at,
                        duration: dur,
                        easing,
                        op: MotionOp::Tint {
                            color: accent,
                            from,
                            to,
                        },
                    });
                }
            }
            return SubjectImage {
                id: c.element.layer.id.clone(),
                layers: Vec::new(),
                shared: true,
            };
        }
        // Continues into the next beat: a new shared element.
        if carry_on {
            let id = format!("shared.asset.{}", slug(&entry.id));
            let mut layer = image_layer(ctx, b, id.clone(), entry, role, rect, z);
            layer.depth = depth;
            let (x, y) = (layer.x, layer.y);
            let track = vec![
                track_key(
                    &plan.id,
                    start,
                    "subject",
                    Easing::Linear,
                    KeyState {
                        x: Some(x),
                        y: Some(y + 40.0 * ctx.u),
                        opacity: Some(0.0),
                        ..Default::default()
                    },
                ),
                track_key(
                    &plan.id,
                    start + 0.7,
                    "subject",
                    Easing::OutCubic,
                    KeyState {
                        y: Some(y),
                        opacity: Some(1.0),
                        ..Default::default()
                    },
                ),
            ];
            let (bw, bh) = (layer.width, layer.height);
            carries.push(Carry {
                element: SharedElement {
                    id: id.clone(),
                    layer,
                    track,
                },
                key,
                base_w: bw,
                base_h: bh,
                open: true,
                last_beat: plan.index,
                state: (x, y, 1.0),
            });
            b.placed.push(carries.len() - 1);
            return SubjectImage {
                id,
                layers: Vec::new(),
                shared: true,
            };
        }
    }
    let mut plate = image_plate(ctx, b, name, role, rect, z, start);
    for l in &mut plate.layers {
        l.depth = depth;
    }
    SubjectImage {
        id: plate.id,
        layers: plate.layers,
        shared: false,
    }
}

// ---------------------------------------------------------------------------
// (0.23 W8a) Picture continuity: a delivered or library picture carried on
// ---------------------------------------------------------------------------

/// A picture that lives in a SharedElement (see [`place_carried`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Carried {
    /// The shared element's layer id: the target of the beat's shared-safe
    /// motions (move, scale, rotate, fade, tint; each ending neutral).
    pub id: String,
    /// Its index in the compile's carries.
    pub carry: usize,
    /// It was already on screen from the previous beat (no entrance here);
    /// `false` = it starts in this beat and goes on into the next.
    pub carried_in: bool,
}

/// How a picture that starts in this beat and is carried on enters: from
/// `offset` (canvas px, relative to its slot) with a fade, over `duration`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Arrive {
    pub offset: (f32, f32),
    pub duration: f64,
}

/// The latest a carried-in picture starts moving to its slot, after ENTER
/// (seconds): a picture already on screen glides with the handoff, it does
/// not wait for the word that names it in this beat.
const CARRY_IN_LATEST: f64 = 0.25;

/// Where a carried picture is drawn when the look's handoffs are panel wipes:
/// above them (`transition.rs` puts the wipe panels at z 60 and 61), so the
/// picture the story keeps on screen is never hidden behind a wipe.
const ABOVE_WIPES_Z: i32 = 62;

/// Whether the look hands off with panel wipes between ordinary beats
/// (`temporal::handoff`: a wipe family that wipes frequently).
fn wipes_between_beats(ctx: &Ctx) -> bool {
    use crate::compiler::taste::{TransitionFamily, WipeTendency};
    let t = ctx.taste.transition;
    t.wipe == WipeTendency::Frequent
        && matches!(
            t.family,
            TransitionFamily::Geometric | TransitionFamily::Kinetic
        )
}

/// (0.23 W8a) Whether beat `plan` may carry a picture on into the next beat:
/// it has a next beat, and neither it nor the next enters with an accent
/// flood (such a beat stacks its stage above every shared picture, which
/// would then be hidden behind its own plate).
pub(crate) fn picture_carries_on(plan: &BeatPlan) -> bool {
    !plan.is_last && !plan.accent_in && !plan.accent_out
}

/// Whether beat `index` has a delivered / catalog picture for its primary or
/// secondary subject: an image for one of the roles `object_request_id` files
/// that subject under.
pub(crate) fn has_delivered_picture(ctx: &Ctx, index: usize, primary: bool) -> bool {
    let roles: &[AssetRole] = if primary {
        &[
            AssetRole::HeroSubject,
            AssetRole::Portrait,
            AssetRole::HeroObject,
            AssetRole::EvidenceImage,
        ]
    } else {
        &[AssetRole::SupportingObject]
    };
    roles.iter().any(|r| image(ctx, index, *r).is_some())
}

/// Continuity key of a picture carried across beats: the image file, so one
/// library asset is one element in every beat (each beat's request has its own
/// manifest entry id).
fn picture_carry_key(entry: &ManifestEntry) -> String {
    format!("picture:{}", entry.path)
}

/// The shared element id of a carried picture: `shared.pic.<file stem>`, with
/// the beat number appended when the same file is carried again later in the
/// story (ids are unique). Not `shared.asset.*`: layout QA reads that prefix
/// as a person photo carried between interlock beats (`subject_too_small`,
/// head regions), which an object picture of a pair is not.
fn shared_picture_id(carries: &[Carry], entry: &ManifestEntry, beat: usize) -> String {
    let stem = std::path::Path::new(&entry.path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&entry.id);
    let mut name = slug(stem);
    // A layer named `subject` is a beat's hero subject for layout QA.
    if name == "subject" {
        name.push_str("_picture");
    }
    let id = format!("shared.pic.{name}");
    if carries.iter().any(|c| c.element.id == id) {
        format!("{id}_{}", beat + 1)
    } else {
        id
    }
}

/// The image file a carried element draws (`None` for text and SVG).
fn carried_path<'c>(ctx: &'c Ctx, carry: &Carry) -> Option<&'c str> {
    match &carry.element.layer.kind {
        LayerKind::Image { asset, .. } => ctx.assets.get(asset).map(|a| a.path.as_str()),
        _ => None,
    }
}

/// (0.23 W8a) Place a picture the story carries across beats, or return
/// `None` when this picture is an ordinary local one of the beat (the caller
/// draws it as it always did):
///
/// - **carried in**: the previous beat carried the same image file (an open
///   carry, matched by path, so a picture placed by `place_subject` and one
///   placed here are the same picture): no second copy, a track key moves and
///   scales the shared element into `rect` from ENTER to SETTLE;
/// - **carried out**: `which` is the subject the beat carries on
///   (`continuity: carry_primary` / `carry_secondary`) and a next beat exists
///   ([`picture_carries_on`]): the picture becomes the SharedElement
///   `shared.pic.<stem>`, drawn in `rect` at `z`, entering with `arrive`.
///
/// A picture the next beat does not show settles into the anchor chip and
/// leaves with that beat (`recipes::close_carries`, as text carries do).
///
/// Product path only: without a direction seed (`--variety`) nothing here
/// changes the compile (`None`), so the default compile stays what it was (a
/// carried picture there keeps no treatment: the contrast guard's sticker
/// would be new on it).
///
/// The element lives outside the stage, so the stage layers above `z` are put
/// in a front group (`B::split_above`): labels and figures stay above the
/// picture. Under a look whose handoffs are panel wipes the picture is drawn
/// above the wipes instead (z 62) and the stage stays below it. `tilt` is the
/// picture's rotation in this beat (degrees). A shared picture takes only
/// shared-safe motions that end neutral; steps that change its scale or
/// opacity go through [`shared_step`]. It has no layer in the scene, so the
/// caller declares no reveal anchor for it and adds no entrance motion.
#[allow(clippy::too_many_arguments)]
pub(crate) fn place_carried(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    which: Which,
    entry: &ManifestEntry,
    role: AssetRole,
    rect: (f32, f32, f32, f32),
    z: i32,
    depth: Option<f32>,
    tilt: f32,
    start: f64,
    arrive: Arrive,
) -> Option<Carried> {
    // No direction seed: the default compile, which carries nothing here.
    ctx.direction_seed?;
    let plan = b.plan;
    let open = carries.iter().position(|c| {
        c.open && c.last_beat + 1 == plan.index && carried_path(ctx, c) == Some(entry.path.as_str())
    });
    let carry_on = wants_carry(b.beat, which) && picture_carries_on(plan);
    if open.is_none() && !carry_on {
        return None;
    }
    // Under panel wipes the picture is drawn above them, so nothing of the
    // stage can be put above it; otherwise it sits at `z` and the stage layers
    // above `z` go to a front group.
    let wipes = wipes_between_beats(ctx);
    let z = if wipes { ABOVE_WIPES_Z } else { z };
    let mut probe = image_layer(ctx, b, String::new(), entry, role, rect, z);
    probe.depth = depth;
    probe.rotation_degrees = tilt;
    if !wipes {
        b.split_above.get_or_insert(z);
    }

    // Carried in from the previous beat: glide into this beat's slot.
    if let Some(ci) = open {
        // The beat's own request for the picture is never drawn.
        ctx.assets.remove(&format!("asset.{}", entry.id));
        let c = &mut carries[ci];
        let s = probe.width / c.base_w.max(1.0);
        let preset = plan.lang.preset;
        let at = start.min(plan.enter_at() + CARRY_IN_LATEST) + preset.duration * 0.6;
        c.element.track.push(track_key(
            &plan.id,
            at,
            "picture",
            preset.settle,
            KeyState {
                x: Some(probe.x),
                y: Some(probe.y),
                scale: Some(s),
                rotation_degrees: Some(tilt),
                opacity: Some(1.0),
            },
        ));
        c.state = (probe.x, probe.y, s);
        c.last_beat = plan.index;
        c.open = carry_on;
        b.placed.push(ci);
        return Some(Carried {
            id: c.element.layer.id.clone(),
            carry: ci,
            carried_in: true,
        });
    }

    // Starts here and goes on: a new shared element. The story keeps it on
    // screen from READ to the next beat's READ, so it has entered by READ (a
    // second picture that would wait for the narrator or for EVOLVE comes in
    // earlier).
    let start = start.min((plan.life.read - 0.5 * arrive.duration).max(plan.enter_at()));
    let id = shared_picture_id(carries, entry, plan.index);
    let mut layer = probe;
    layer.id = id.clone();
    let (x, y) = (layer.x, layer.y);
    let track = vec![
        track_key(
            &plan.id,
            start,
            "picture",
            Easing::Linear,
            KeyState {
                x: Some(x + arrive.offset.0),
                y: Some(y + arrive.offset.1),
                opacity: Some(0.0),
                ..Default::default()
            },
        ),
        track_key(
            &plan.id,
            start + arrive.duration,
            "picture",
            Easing::OutCubic,
            KeyState {
                x: Some(x),
                y: Some(y),
                opacity: Some(1.0),
                ..Default::default()
            },
        ),
    ];
    let (bw, bh) = (layer.width, layer.height);
    carries.push(Carry {
        element: SharedElement {
            id: id.clone(),
            layer,
            track,
        },
        key: picture_carry_key(entry),
        base_w: bw.max(1.0),
        base_h: bh.max(1.0),
        open: true,
        last_beat: plan.index,
        state: (x, y, 1.0),
    });
    let ci = carries.len() - 1;
    b.placed.push(ci);
    Some(Carried {
        id,
        carry: ci,
        carried_in: false,
    })
}

/// (0.23 W8a) A step of a carried picture inside this beat, in its track (a
/// shared element cannot hold a scale or fade motion past its scene): from
/// `at` to `at + duration` its scale is multiplied by `scale` and, when
/// given, its opacity becomes `opacity` (the next beat's key restores it).
#[allow(clippy::too_many_arguments)]
pub(crate) fn shared_step(
    b: &B,
    carries: &mut [Carry],
    carry: usize,
    at: f64,
    duration: f64,
    easing: Easing,
    scale: f32,
    opacity: Option<f32>,
) {
    let Some(c) = carries.get_mut(carry) else {
        return;
    };
    c.element.track.push(track_key(
        &b.plan.id,
        at,
        "step",
        Easing::Linear,
        KeyState::default(),
    ));
    c.state.2 *= scale;
    c.element.track.push(track_key(
        &b.plan.id,
        at + duration,
        "step",
        easing,
        KeyState {
            scale: Some(c.state.2),
            opacity,
            ..Default::default()
        },
    ));
}

fn placeholder(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    role: AssetRole,
    rect: (f32, f32, f32, f32),
    z: i32,
    start: f64,
) -> Plate {
    let u = ctx.u;
    let (x, y, w, h) = rect;
    let id = b.id(name);
    let mut layers = Vec::new();

    let tex_id = format!("{id}.tex");
    layers.push(texture_layer(
        &tex_id,
        (x - 0.05 * w, y + 0.06 * h, w * 0.9, h * 0.9),
        TextureSpec {
            material: Material::Halftone,
            seed: mix(b.plan.seed, 0x91A7),
            color: ctx.palette.accent,
            intensity: 0.9,
            scale: 15.0 * u,
            animated: false,
        },
        z - 1,
    ));
    b.motions.push(mo::mask(
        &tex_id,
        start,
        0.9,
        Direction::Down,
        Easing::OutCubic,
    ));

    layers.push(base_layer(
        id.clone(),
        (x, y, w, h),
        LayerKind::RoundedRectangle {
            fill: ctx.palette.card,
            radius: 10.0 * u,
            stroke: Some(Stroke {
                color: ctx.palette.ink.with_alpha(0x22),
                width: 2.0 * u,
            }),
        },
        z,
    ));
    b.motions.push(mo::mask(
        &id,
        start + 0.08,
        0.75,
        Direction::Up,
        Easing::OutQuint,
    ));

    // A faint bust shape for cutout roles, so the plate reads as "a person goes here".
    if matches!(role, AssetRole::HeroSubject | AssetRole::Portrait) {
        let tone = ctx.palette.ink.with_alpha(0x14);
        let head = 0.34 * w.min(h * 0.6);
        let head_id = format!("{id}.figure_head");
        layers.push(base_layer(
            head_id.clone(),
            (x + w / 2.0 - head / 2.0, y + 0.16 * h, head, head),
            LayerKind::RoundedRectangle {
                fill: tone,
                radius: head / 2.0,
                stroke: None,
            },
            z,
        ));
        let body_id = format!("{id}.figure_body");
        layers.push(base_layer(
            body_id.clone(),
            (x + 0.14 * w, y + 0.16 * h + head * 1.12, 0.72 * w, 0.62 * h),
            LayerKind::RoundedRectangle {
                fill: tone,
                radius: 0.3 * w,
                stroke: None,
            },
            z,
        ));
        for fid in [&head_id, &body_id] {
            b.motions.push(mo::mask(
                fid,
                start + 0.16,
                0.7,
                Direction::Up,
                Easing::OutQuint,
            ));
        }
    }

    let role_name = serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.replace('_', " ")))
        .unwrap_or_default();
    let block = ctx.ts.fit_line(
        Voice::LABEL,
        &format!("IMAGE · {role_name}"),
        w * 0.84,
        26.0 * u,
    );
    let label_id = format!("{id}.label");
    let mut label = text_layer(
        label_id.clone(),
        &block,
        ctx.palette.ink.with_alpha(0x66),
        TextAlign::Left,
    );
    label.x = x + (w - label.width) / 2.0;
    label.y = y + h - block.height() - 0.05 * h;
    label.z_index = z + 1;
    b.motions.push(mo::fade(
        &label_id,
        start + 0.4,
        0.5,
        0.0,
        1.0,
        Easing::OutCubic,
    ));
    layers.push(label);

    Plate {
        id,
        delivered: false,
        layers,
    }
}

/// Evolve-event times for `n` items, kept clear of the ANTICIPATE boundary and
/// early enough that an item needing `tail` seconds after its event (entrance,
/// counter, rule) still ends inside the scene.
pub(super) fn events(b: &B, n: usize, tail: f64) -> Vec<f64> {
    let (life, duration) = (b.plan.life, b.plan.duration);
    let latest = (life.anticipate - 0.5)
        .min(duration - tail - 0.02)
        .max(life.enter);
    life.evolve_events(n)
        .into_iter()
        .map(|t| t.min(latest))
        .collect()
}

/// Seconds a subject placed by `place_subject` needs after its event
/// (entrance, or the count-up of a data-language number).
pub(super) fn subject_tail(b: &B, s: &Subject) -> f64 {
    let d = b.plan.lang.preset.duration;
    if b.plan.lang.data_numbers && s.kind() == SubjectKind::Number {
        0.15 * d + (2.0 * d).max(1.1)
    } else {
        d
    }
}

/// Seconds a [`serif_note`] needs after its event (line entrance, line
/// stagger, underline).
pub(super) fn serif_tail(b: &B) -> f64 {
    (b.plan.lang.preset.duration + 0.5).max(1.1)
}

/// A serif note (secondary phrase) with an accent underline, top-aligned at
/// `top`, arriving at `start`. Returns the y of the bottom of the underline.
#[allow(clippy::too_many_arguments)]
pub(super) fn serif_note(
    ctx: &Ctx,
    b: &mut B,
    name: &str,
    text: &str,
    top: f32,
    align: TextAlign,
    max_w: f32,
    max_h: f32,
    max_size: f32,
    start: f64,
) -> f32 {
    let u = ctx.u;
    let block = ctx
        .ts
        .fit_block(Voice::SERIF, text, max_w, max_h, max_size, 2);
    let (bh, _) = recipes::headline_lines(
        ctx,
        b,
        name,
        &block,
        top,
        align,
        ctx.palette.ink,
        start,
        false,
    );
    let uw = block.width().min(max_w);
    let ux = match align {
        TextAlign::Left => ctx.margin(),
        TextAlign::Center => (ctx.w - uw) / 2.0,
        TextAlign::Right => ctx.w - ctx.margin() - uw,
    };
    let rule_y = top + bh + 14.0 * u;
    recipes::underline(
        ctx,
        b,
        &format!("{name}_rule"),
        (ux, rule_y, uw, 9.0 * u),
        ctx.palette.accent,
        start + 0.35,
    );
    rule_y + 9.0 * u
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::assets::{AssetManifest, NormPoint};
    use crate::compiler::recipes::B;
    use crate::compiler::{plan_timing, ApproxMeasure, AssetLibrary, FontSet, Palette, Typesetter};
    use crate::intent::{CreativeIntent, Format};
    use crate::style::StyleProfile;

    const STYLE: &str = r#"{"material":"paper","typography_style":"grotesk_serif","depth":"layered","camera_style":"slow_push","motion_language":"auto","texture_style":"subtle_print","accent_role":"cobalt","seed":1}"#;
    const INTENT: &str = r#"{"version":"0.2","title":"t","format":"vertical","beats":[{"purpose":"emphasize","statement":"Hello there","primary":{"kind":"phrase","value":"hello"}}]}"#;

    fn plate_for(manifest: &AssetManifest) -> (Plate, BTreeMap<String, crate::scene::Asset>) {
        let style: StyleProfile = serde_json::from_str(STYLE).expect("style");
        let intent: CreativeIntent = serde_json::from_str(INTENT).expect("intent");
        let plans = plan_timing(&intent, &style);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_style(&style);
        let ts = Typesetter::new(&ApproxMeasure, &library.root, &fonts);
        let mut ctx = Ctx {
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
            ts,
            library: &library,
            assets: BTreeMap::new(),
            format: Format::Vertical,
            frame: crate::compiler::layout_frame::LayoutFrame::for_format(Format::Vertical),
            manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: 1,
            sequence: Default::default(),
        };
        let mut b = B {
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
        let plate = image_plate(
            &mut ctx,
            &mut b,
            "subject",
            AssetRole::HeroSubject,
            (100.0, 200.0, 600.0, 900.0),
            22,
            0.3,
        );
        (plate, ctx.assets)
    }

    #[test]
    fn missing_image_is_a_procedural_plate_with_halftone_and_label() {
        let (plate, assets) = plate_for(&AssetManifest::empty());
        assert!(!plate.delivered);
        assert!(assets.is_empty(), "no image asset registered");
        assert!(plate.layers.iter().any(|l| matches!(&l.kind,
            LayerKind::Texture(t) if t.material == Material::Halftone)));
        assert!(plate.layers.iter().any(|l| matches!(&l.kind,
            LayerKind::Text(t) if t.text.contains("HERO SUBJECT"))));
        let card = plate
            .layers
            .iter()
            .find(|l| l.id == plate.id)
            .expect("card");
        assert_eq!(
            (card.x, card.y, card.width, card.height),
            (100.0, 200.0, 600.0, 900.0)
        );
    }

    #[test]
    fn delivered_image_keeps_its_aspect_inside_the_box_and_anchors_on_the_subject() {
        let manifest = AssetManifest {
            version: "0.1".into(),
            assets: vec![crate::assets::ManifestEntry {
                id: "beat_1.hero_subject".into(),
                path: "test_images/figure.png".into(),
                width: 600,
                height: 300,
                alpha: true,
                safe_bounds: None,
                subject_anchor: Some(NormPoint { x: 0.25, y: 0.5 }),
                face_anchor: None,
                ..Default::default()
            }],
            missing: Vec::new(),
        };
        let (plate, assets) = plate_for(&manifest);
        assert!(plate.delivered);
        let asset = assets.get("asset.beat_1.hero_subject").expect("registered");
        assert_eq!(asset.path, "test_images/figure.png");
        let l = &plate.layers[0];
        // 2:1 image in a 600x900 rect: full width, centered vertically.
        assert_eq!((l.width, l.height), (600.0, 300.0));
        assert_eq!((l.anchor_x, l.anchor_y), (0.25, 0.5));
        assert_eq!(l.x - l.anchor_x * l.width, 100.0);
        assert_eq!(l.y - l.anchor_y * l.height, 200.0 + 300.0);
    }
}
