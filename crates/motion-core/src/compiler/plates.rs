//! (0.24 M6) Video plates: a footage-plan clip (`role: plate`) becomes a
//! full-bleed muted backdrop at the bottom of its beat scene (z 1-2: above
//! the persistent paper at z 0, below every stage group), under every other
//! layer. The video layer carries the look's treatment (desaturate +
//! ground tint), a ground-coloured scrim above it keeps type readable, and a
//! slow 1.0 → 1.06 push plays over the beat. Operator input only: the intent
//! never mentions footage. Without a plan nothing here runs.

use super::{base_layer, mo, CompileWarning, Ctx};
use crate::easing::Easing;
use crate::footage::{FootageClip, FootagePlan, FootageRole};
use crate::scene::{
    Asset, AssetKind, Fit, ImageTreatment, LayerKind, Scene, SpriteMode, SpritePlayback,
    SpriteSpec, TintSpec,
};

/// Warning code: a plate clip could not be placed (bad beat, unprobed clip).
pub const WARN_PLATE_SKIPPED: &str = "plate_skipped";

/// Scrim opacity when frames cannot be probed at plan time.
pub const SCRIM_OPACITY: f32 = 0.55;
/// Scrim opacity over a plate on a dark ground (light type).
pub const SCRIM_OPACITY_DARK: f32 = 0.28;
/// Frame clock of plate assets (the canvas fps).
const PLATE_FPS: f64 = 30.0;
/// Ken-burns end scale.
const PUSH: f32 = 1.03;

/// Add the plate layers to `scenes`. `clip.beat` is 1-based like warnings and
/// layer ids (`beat: 2` = `scenes[1]`, the second beat); 0 is skipped.
pub(crate) fn apply(ctx: &mut Ctx, scenes: &mut [Scene], plan: &FootagePlan) {
    for clip in plan.clips.iter().filter(|c| c.role == FootageRole::Plate) {
        let Some(scene) = clip.beat.checked_sub(1).and_then(|i| scenes.get_mut(i)) else {
            skip(ctx, clip, "the beat does not exist");
            continue;
        };
        let native = if clip.fps > 0.0 {
            f64::from(clip.frame_count) / clip.fps
        } else {
            0.0
        };
        if !(native.is_finite() && native > 0.0) {
            skip(
                ctx,
                clip,
                "frame_count/fps are missing (the CLI probes them with ffprobe)",
            );
            continue;
        }
        let in_s = clip.in_s.unwrap_or(0.0).clamp(0.0, native);
        let out_s = clip.out_s.unwrap_or(native).clamp(0.0, native);
        let total = (native * PLATE_FPS).floor() as u32;
        let in_f = ((in_s * PLATE_FPS).round() as u32).min(total.saturating_sub(1));
        let out_f = ((out_s * PLATE_FPS).floor() as u32).saturating_sub(1);
        if total == 0 || out_f < in_f {
            skip(ctx, clip, "in_s/out_s leave no frames");
            continue;
        }
        let asset_id = format!("asset.plate.{}", clip.beat);
        ctx.assets.insert(
            asset_id.clone(),
            Asset {
                id: asset_id.clone(),
                kind: AssetKind::Video,
                path: clip.path.clone(),
                sprite: Some(SpriteSpec {
                    frame_count: total,
                    fps: PLATE_FPS,
                    mode: SpriteMode::Once,
                    pattern: "frame_%04d.png".into(),
                }),
            },
        );
        let (w, h) = (ctx.w, ctx.h);
        let ground = ctx.palette.paper;
        // Light type on a dark ground already reads over dark footage, so a
        // dark ground keeps the clip's own colour and a thin scrim; a light
        // ground needs the clip washed towards paper to hold dark type.
        let dark = super::brand::luminance(ground) < 0.2;
        let (desat, tint, scrim_opacity) = if dark {
            (0.1, 0.08, SCRIM_OPACITY_DARK)
        } else {
            (0.6, 0.35, SCRIM_OPACITY)
        };
        let mut treatment = ImageTreatment::natural();
        treatment.desaturate = desat;
        treatment.tint = Some(TintSpec {
            color: ground,
            amount: tint,
        });
        let id = format!("plate.{}", clip.beat);
        let mut video = base_layer(
            id.clone(),
            (w / 2.0, h / 2.0, w, h),
            LayerKind::Image {
                asset: asset_id,
                fit: Fit::Cover,
                treatment: Some(treatment),
                playback: Some(SpritePlayback {
                    start: 0.0,
                    in_frame: in_f,
                    out_frame: Some(out_f),
                }),
                insert: None,
            },
            1,
        );
        video.anchor_x = 0.5;
        video.anchor_y = 0.5;
        let scrim_id = format!("{id}.scrim");
        let mut scrim = base_layer(
            scrim_id,
            (0.0, 0.0, w, h),
            LayerKind::Rectangle {
                fill: ground,
                stroke: None,
            },
            2,
        );
        scrim.opacity = scrim_opacity;
        let d = scene.duration_seconds;
        let fade = (d * 0.2).clamp(0.2, 0.6);
        scene
            .motions
            .insert(0, mo::scale(&id, 0.0, d, 1.0, PUSH, Easing::Linear));
        scene
            .motions
            .insert(0, mo::fade(&id, 0.0, fade, 0.0, 1.0, Easing::OutCubic));
        scene.motions.insert(
            0,
            mo::fade(
                &id,
                (d - fade).max(fade),
                fade.min(d),
                1.0,
                0.0,
                Easing::InCubic,
            ),
        );
        scene.layers.insert(0, scrim);
        scene.layers.insert(0, video);
    }
}

fn skip(ctx: &mut Ctx, clip: &FootageClip, why: &str) {
    ctx.warnings.push(CompileWarning {
        code: WARN_PLATE_SKIPPED.into(),
        beat: Some(clip.beat),
        message: format!("plate '{}' not placed: {why}", clip.path),
    });
}
