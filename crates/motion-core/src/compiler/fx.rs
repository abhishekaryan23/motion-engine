//! (0.14) FX director: a genre look's finishing effects, applied to the built
//! beat scenes after layout. The 0.12 looks have an all-off
//! [`LookFx`], so they render exactly as before.
//!
//! Inputs: the beat scenes (one per beat, same order as `plans`), the beat
//! plans (start/duration/lifecycle), the beats (energy, keyword), the speech
//! map (word times; `None` without a voice-over), the look's [`LookFx`] and
//! the id of a music envelope in the project (`Some("music")` when the
//! operator measured one).
//!
//! Rules (all deterministic, seeds from the beat index):
//! - `grain` / `vignette` > 0: a `grain` / `vignette` [`PostEffect`] over each
//!   beat scene's whole duration at that amount.
//! - `chroma_hits`: a 0.25 s chromatic-aberration flash (shift 6 px at
//!   strength 1, strength 1 → 0) on every beat's ENTER and on the spoken
//!   keyword (or the first impact-energy word) when there is speech.
//! - `glitch_cuts`: a 0.18 s glitch (8 bands, 60 px) straddling each beat
//!   start except the first.
//! - `shake_trauma` > 0: a camera `shake` (trauma = value × 1.0 for
//!   `impact` energy, × 0.5 for `building`, none for `calm`), 0.45 s from the
//!   ENTER, decay 6, 18 Hz; plus a 0.3 s shake on the spoken keyword.
//! - `echo_entrances`: an `echo` (count 3, spacing 0.04, decay 0.55) on the
//!   largest image/text layer of the beat over its entrance motion window.
//! - `pulse_gain` > 0 and a music envelope: a `pulse` on the same hero layer
//!   from SETTLE to the scene end.
//! - `bloom`: a soft bloom (threshold 0.75, radius 24) at strength 0.6 over
//!   the whole beat.
//! - `perspective` (0.16): a perspective camera (fov 40, aperture 6) with a
//!   slow dolly (0 → 60 px) over the beat and z offsets on background
//!   layers (plates +600, ghost words +300).
//! - (0.23 W4) On the product path (`--variety`) the choreographed camera's
//!   fly-in lasts 0.45 s and lands in time for the dead-air limit
//!   (`checks::DEAD_AIR_*`: it is shortened, 0.4 s at the least, and the
//!   opening one starts on the first frames) and the zoom-burst blooms of a
//!   handoff last 0.4 s at most.
//!   Without `--variety` the 0.75 s fly-in and the 0.55 s burst are as ever.

use crate::compiler::art_direction::LookFx;
use crate::easing::Easing;
use crate::intent::{Beat, Energy, SubjectKind};
use crate::scene::{
    Camera, CameraMotion, CameraOp, Layer, LayerKind, Motion, MotionOp, Perspective, PostEffect,
    PostKind, Scene,
};
use crate::speech::SpeechMap;

use super::speech_plan::{anchor_time, first_term};
use super::typeset::{text_layer, Block, Voice};
use super::{direction, BeatPlan, Ctx};
use crate::scene::{SpringSpec, TextAlign};

/// Seconds a chromatic flash lasts.
const CHROMA_FLASH: f64 = 0.25;
/// Seconds of the glitch burst at a cut.
const GLITCH_CUT: f64 = 0.18;
/// Entrance shake / keyword shake lengths.
const SHAKE_ENTER: f64 = 0.45;
const SHAKE_WORD: f64 = 0.3;
/// Share of the style's 2D drift camera kept under a genre look.
const DRIFT_DAMP: f32 = 0.4;

/// Apply a genre look's finishing effects to the beat scenes.
///
/// Returns the camera move each beat's seeded rotation chose, as (beat index,
/// move name), for the direction record: empty without a direction seed and
/// for beats whose camera the look fixes.
pub(crate) fn apply_look_fx(
    ctx: &Ctx,
    scenes: &mut [Scene],
    plans: &[BeatPlan],
    beats: &[Beat],
    speech: Option<&SpeechMap>,
    fx: &LookFx,
    music_envelope: Option<&str>,
) -> Vec<(usize, &'static str)> {
    let mut camera_picks: Vec<(usize, &'static str)> = Vec::new();
    if *fx == LookFx::default() {
        return camera_picks;
    }
    // (0.23) Under a direction seed the cinematic camera moves rotate by a
    // seeded sequence instead of by beat position, planned over the whole piece
    // so that no two neighbouring beats share a move (a turning sphere, which
    // the camera can only push in on, counts as a push-in).
    let camera_plan: Vec<Option<usize>> = match ctx.direction_seed {
        Some(seed)
            if fx.perspective
                && fx.camera_moves
                && fx.grammar == Some(crate::compiler::art_direction::LookGrammar::Cinematic3d) =>
        {
            let seq = direction::seeded_sequence(seed, direction::Dim::Camera, plans.len(), 4);
            let safe = (ctx.frame.safe.x, ctx.frame.safe.x + ctx.frame.safe.w);
            let slots: Vec<Option<CameraSlot>> = scenes
                .iter()
                .zip(beats)
                .enumerate()
                .map(|(index, (scene, beat))| {
                    // A layers beat keeps a still camera: no move, and it ends a run.
                    if matches!(beat.primary, crate::intent::Subject::Layers(_)) {
                        return None;
                    }
                    // The scene as `choreograph` meets it.
                    let mut probe = scene.clone();
                    hoist_depth_layers(&mut probe);
                    drop_exit_moves(&mut probe);
                    let (room, span) = (side_room(&probe, ctx.w), text_span(&probe));
                    Some(CameraSlot {
                        fits: [0, 1, 2, 3].map(|m| move_fits(m, index, room, span, safe)),
                        forced: probe
                            .motions
                            .iter()
                            .any(|m| matches!(m.op, MotionOp::Revolve { .. })),
                    })
                })
                .collect();
            plan_camera(&seq, &slots)
        }
        _ => Vec::new(),
    };
    let safe = (ctx.frame.safe.x, ctx.frame.safe.x + ctx.frame.safe.w);
    let mut prev_end: Option<f64> = None;
    for ((scene, plan), beat) in scenes.iter_mut().zip(plans).zip(beats) {
        let this_prev_end = prev_end.replace(scene.start_seconds + scene.duration_seconds);
        let prev_end = this_prev_end;
        let seed = (plan.index as u32 + 1).wrapping_mul(7919);
        let (enter, settle) = scene
            .lifecycle
            .map(|l| (l.enter, l.settle.max(l.enter + 0.3)))
            .unwrap_or((0.0, 0.6));
        let duration = scene.duration_seconds;
        let hit = keyword_time(scene, plan, beat, speech).filter(|t| *t < duration - 0.1);
        // (0.19) A layers beat sits on a 2D backdrop column: its camera stays
        // still (no shake, push, orbit or fly-through) so what it pins stays
        // on its layer.
        let stack_beat = matches!(beat.primary, crate::intent::Subject::Layers(_));
        // Genre looks frame deliberately: damp the style's 2D drift camera to
        // 40 % so heroes, stencils and furniture stay inside the canvas.
        if let Some(cam) = scene.camera.as_mut() {
            for m in cam.motions.iter_mut() {
                match &mut m.op {
                    CameraOp::Push { from, to } => {
                        *from = 1.0 + (*from - 1.0) * DRIFT_DAMP;
                        *to = 1.0 + (*to - 1.0) * DRIFT_DAMP;
                    }
                    CameraOp::Track { from, to } => {
                        for v in from.iter_mut().chain(to.iter_mut()) {
                            *v *= DRIFT_DAMP;
                        }
                    }
                    _ => {}
                }
            }
        }

        // Persistent finish.
        if fx.grain > 0.0 {
            scene.post.push(persistent(PostKind::Grain {
                amount: fx.grain,
                seed,
            }));
        }
        if fx.vignette > 0.0 {
            scene.post.push(persistent(PostKind::Vignette {
                amount: fx.vignette,
            }));
        }
        if fx.bloom {
            scene.post.push(PostEffect {
                from: 0.6,
                to: 0.6,
                ..persistent(PostKind::Bloom {
                    threshold: 0.75,
                    radius: 24.0,
                })
            });
        }
        // Hits.
        if fx.chroma_hits {
            for t in std::iter::once(enter).chain(hit) {
                scene.post.push(flash(
                    t,
                    CHROMA_FLASH,
                    PostKind::ChromaticAberration {
                        shift: 6.0,
                        angle_deg: 0.0,
                    },
                ));
            }
        }
        if fx.glitch_cuts && plan.index > 0 {
            scene.post.push(flash(
                0.0,
                GLITCH_CUT,
                PostKind::Glitch {
                    bands: 8,
                    max_shift: 60.0,
                    seed,
                },
            ));
        }
        let trauma = fx.shake_trauma
            * match beat.energy {
                Energy::Impact => 1.0,
                Energy::Building => 0.5,
                Energy::Calm => 0.0,
            };
        if trauma > 0.0 && !stack_beat {
            let cam = scene.camera.get_or_insert_with(Camera::default);
            cam.motions.push(shake(enter, SHAKE_ENTER, trauma, seed));
            if let Some(t) = hit.filter(|t| *t >= enter + SHAKE_ENTER) {
                cam.motions
                    .push(shake(t, SHAKE_WORD, (trauma * 0.6).min(1.0), seed ^ 0x51));
            }
        }
        // Studio: the spoken words, large, behind the hero. (0.23 A5a) Under a
        // direction seed (the product path) not on a layers beat: its headline
        // is the beat's type, set on the strata scrim, and the words (round that
        // headline's head) were drawn over it; below the scrim they would be ink
        // on the dark strata.
        if fx.kinetic_words && !(stack_beat && ctx.direction_seed.is_some()) {
            if let Some(speech) = speech {
                kinetic_words(ctx, scene, plan, beat, speech);
            }
        }
        // Hero layer: entrance echo and audio pulse.
        if let Some(hero) = hero_layer(&scene.layers) {
            if fx.echo_entrances {
                scene.motions.push(motion(
                    &hero,
                    enter,
                    (settle - enter).max(0.3),
                    MotionOp::Echo {
                        count: 3,
                        spacing: 0.04,
                        decay: 0.55,
                    },
                ));
            }
            if let Some(env) = music_envelope.filter(|_| fx.pulse_gain > 0.0 && !stack_beat) {
                scene.motions.push(motion(
                    &hero,
                    settle,
                    (duration - settle).max(0.0),
                    MotionOp::Pulse {
                        envelope: env.to_string(),
                        gain: fx.pulse_gain,
                    },
                ));
            }
        }
        // (0.16) Perspective: slow dolly, background type and plates pushed back.
        if stack_beat && fx.perspective {
            // A still perspective camera: z = 0 maps 1:1, so the pinned
            // subject can turn into view (`tilt`) and still sit on its layer.
            hoist_depth_layers(scene);
            let cam = scene.camera.get_or_insert_with(Camera::default);
            cam.motions
                .retain(|m| matches!(m.op, CameraOp::Roll { .. }));
            cam.perspective = Some(Perspective {
                fov_deg: CINE_FOV,
                focus_z: 0.0,
                aperture: 0.0,
                billboard: false,
                depth_sort: false,
            });
        } else if fx.perspective && fx.camera_moves {
            hoist_depth_layers(scene);
            let gentle =
                fx.grammar != Some(crate::compiler::art_direction::LookGrammar::Cinematic3d);
            let last = plan.index + 1 == plans.len();
            // Where the previous beat (and its fly-through) ends.
            let arrive = prev_end.map_or(0.0, |e| (e - scene.start_seconds).max(0.0));
            drop_exit_moves(scene);
            let focal = ctx.focal.get(&plan.index).map(String::as_str);
            let z_f = focal_z(scene, focal);
            // The camera orbits round what the beat is about.
            let pivot = focal_centre(scene, focal).unwrap_or((ctx.w / 2.0, ctx.h / 2.0));
            let rack_from = if gentle {
                z_f + 300.0
            } else {
                crate::compiler::grammar::cinematic_ghost_z()
            };
            let (picked, rotated) = choreograph(
                scene,
                plan.index,
                last,
                gentle,
                z_f,
                rack_from,
                (ctx.w, ctx.h),
                pivot,
                arrive,
                super::speech_lifecycle::dead_air_rules(ctx),
                camera_plan
                    .get(plan.index)
                    .copied()
                    .flatten()
                    .map(|planned| CameraRotation { planned, safe }),
            );
            // Only the moves the seeded rotation chose are "rotations that
            // applied"; a move the look or a sphere forces is not recorded.
            if rotated {
                camera_picks.push((plan.index, camera_move_name(picked)));
            }
        } else if fx.perspective {
            hoist_depth_layers(scene);
            let cam = scene.camera.get_or_insert_with(Camera::default);
            cam.perspective = Some(Perspective {
                fov_deg: 40.0,
                focus_z: 0.0,
                aperture: 6.0,
                billboard: false,
                depth_sort: false,
            });
            cam.motions.push(CameraMotion {
                start: 0.0,
                duration,
                easing: Easing::InOutCubic,
                velocity: None,
                op: CameraOp::Dolly {
                    from: 0.0,
                    to: 60.0,
                },
            });
            for l in scene.layers.iter_mut() {
                if l.z.is_some() {
                    continue;
                }
                if l.id.contains("ghost") {
                    l.z = Some(300.0);
                } else if l.id.contains("env") || l.id.contains("plate") {
                    l.z = Some(600.0);
                }
            }
        }
    }
    camera_picks
}

/// (0.23) The seeded camera move planned for a beat ([`plan_camera`]) and the
/// safe area's left and right canvas x.
#[derive(Clone, Copy)]
struct CameraRotation {
    planned: usize,
    safe: (f32, f32),
}

/// (0.23) What the camera plan needs to know about a choreographed beat.
#[derive(Clone, Copy)]
struct CameraSlot {
    /// Which moves (0 push-in, 1 truck, 2 crane, 3 orbit) fit the beat.
    fits: [bool; 4],
    /// The camera is forced onto the push-in (a turning sphere): no rotation.
    forced: bool,
}

/// (0.23) Whether main move `pick` fits a beat. The truck needs sideways room.
/// The position rule never gave the crane to a beat of wide type or to the
/// opening, and the variety bench (every story, the crane forced onto every
/// beat) shows why: it pushes type that already reaches the safe edge
/// (`safe`: left and right canvas x) out of it (`text_outside_safe`), and as the
/// opening it reads after the 0.5 s dead-air limit. The push-in, the orbit and
/// the truck (with its room check) passed everywhere.
fn move_fits(
    pick: usize,
    index: usize,
    room: f32,
    span: Option<(f32, f32)>,
    safe: (f32, f32),
) -> bool {
    match pick {
        1 => room >= TRUCK_ROOM_MIN,
        2 => index > 0 && span.is_none_or(|(x0, x1)| x0 >= safe.0 - 1.0 && x1 <= safe.1 + 1.0),
        _ => true,
    }
}

/// (0.23) The main move of every choreographed beat under a direction seed:
/// per run of consecutive choreographed beats the cheapest sequence in which
/// no two neighbours share a move, a forced beat (a turning sphere) counting
/// as a push-in. A beat costs nothing for its seeded move (`seq`), one for the
/// orbit (the safe fallback) and two for another move that fits; a repeated
/// neighbour costs far more than any of that, so the seeded moves are kept
/// wherever they fit and differ, and a beat whose seeded move does not fit, or
/// would repeat its neighbour's (a forced push-in before or after it
/// included), is replaced as little as possible. Only when no sequence avoids
/// a repeat (two spheres side by side) does one remain. Beats that are not
/// choreographed (`None`) end a run and get no entry.
fn plan_camera(seq: &[usize], slots: &[Option<CameraSlot>]) -> Vec<Option<usize>> {
    const REPEAT: u32 = 1000;
    let mut plan = vec![None; slots.len()];
    let mut start = 0;
    while start < slots.len() {
        if slots[start].is_none() {
            start += 1;
            continue;
        }
        let run: Vec<CameraSlot> = slots[start..].iter().map_while(|s| *s).collect();
        let n = run.len();
        // Cheapest cost of a run ending on move `p` at beat k, and its predecessor.
        let mut cost = vec![[u32::MAX; 4]; n];
        let mut from = vec![[0usize; 4]; n];
        for (k, slot) in run.iter().enumerate() {
            let base = seq.get(start + k).map_or(3, |&b| b % 4);
            for p in 0..4 {
                let own = match (slot.forced, p) {
                    (true, 0) => 0,
                    (true, _) => continue,
                    _ if !slot.fits[p] => continue,
                    _ if p == base => 0,
                    _ if p == 3 => 1,
                    _ => 2,
                };
                if k == 0 {
                    cost[0][p] = own;
                    continue;
                }
                for q in 0..4 {
                    if cost[k - 1][q] == u32::MAX {
                        continue;
                    }
                    let total = cost[k - 1][q] + own + if q == p { REPEAT } else { 0 };
                    if total < cost[k][p] {
                        cost[k][p] = total;
                        from[k][p] = q;
                    }
                }
            }
        }
        let mut p = (0..4).min_by_key(|&p| cost[n - 1][p]).unwrap_or(3);
        for k in (0..n).rev() {
            plan[start + k] = Some(p);
            p = from[k][p];
        }
        start += n;
    }
    plan
}

/// The name of a camera pick in the direction record.
fn camera_move_name(pick: usize) -> &'static str {
    match pick {
        0 => "push_in",
        1 => "truck",
        2 => "crane",
        _ => "orbit",
    }
}

/// Vertical field of view of the choreographed camera (degrees).
const CINE_FOV: f32 = 40.0;

/// (0.23 W4) The fly-in is this long (seconds, at most a quarter of the beat)
/// unless the beat's dead-air limit asks for less.
const FLY_IN_S: f64 = 0.75;
/// (0.23 W4) The fly-in on the product path: the focus has racked onto the
/// stage, and what waits there (the kicker, a held hero's placeholder) reads,
/// half a second after ENTER.
const FLY_IN_PRODUCT_S: f64 = 0.45;
/// (0.23 W4) The shortest fly-in the product path allows.
const FLY_IN_MIN_S: f64 = 0.4;
/// (0.23 W4) The focus has racked onto the stage this long before the
/// dead-air limit, so the beat's text reads before it.
const FLY_IN_SLACK_S: f64 = 0.1;
/// (0.23 W4) The longest a handoff bloom (the zoom-burst rays leaving one beat
/// and arriving in the next) is on screen.
const BLOOM_MAX_S: f64 = 0.4;

/// (0.23 W4) When the fly-in starts and how long it lasts: from ENTER for
/// `FLY_IN_S`, as ever. On the product path it lasts `FLY_IN_PRODUCT_S`, and a
/// fly-in that would still land after the beat's dead-air limit
/// (`checks::DEAD_AIR_*`: the first beat's text reads within 0.5 s of the video
/// start, every later beat's within 1.2 s of its start; the rack keeps the
/// stage's text soft until it lands) is shortened to land in time, to
/// `FLY_IN_MIN_S` at the least, and starts ahead of ENTER when even that would
/// be late. The first beat's camera therefore starts moving on the first
/// frames instead of after ENTER.
fn fly_in(product: bool, index: usize, enter: f64, dur: f64) -> (f64, f64) {
    let start = enter.clamp(0.0, 0.3 * dur);
    let len = FLY_IN_S.min(0.25 * dur);
    if !product {
        return (start, len);
    }
    let len = len.min(FLY_IN_PRODUCT_S);
    let limit = if index == 0 {
        crate::checks::DEAD_AIR_FIRST_READABLE_S
    } else {
        crate::checks::DEAD_AIR_MAX_HOLD_S
    };
    let land_by = limit - FLY_IN_SLACK_S;
    if start + len <= land_by + 1e-9 {
        return (start, len);
    }
    let len = len.min((land_by - start).max(FLY_IN_MIN_S));
    (start.min((land_by - len).max(0.0)), len)
}

/// (0.17) A three-act camera per beat: fly-in (from far back, decelerating,
/// zoom-blur fading), a main move from a vocabulary (push-in with a slight
/// orbit, lateral truck, crane down, orbit round the hero; `gentle` keeps to
/// the small push-in), and a fly-through exit (accelerating forward with
/// zoom-blur) into the next beat. Depth layers are compensated so the
/// laid-out composition holds at the main move's midpoint while their
/// parallax stays real.
///
/// (0.18) One continuous move: the dolly's three acts hand their speed over
/// (`CameraMotion.velocity`), so the camera never stops between them; the
/// main move (orbit / truck / crane) drifts at constant speed across the
/// whole beat. Focus racks from the background onto the focal layer (`z_f`)
/// during the fly-in and then tracks it exactly, so what the beat is about is
/// always sharp. Cinematic sprites face the camera (`billboard`).
///
/// (0.23) Returns the main move the beat really has (0 push-in, 1 truck, 2
/// crane, 3 orbit; a forced push-in included) and whether the seeded rotation
/// chose it (so it is a recorded rotation).
#[allow(clippy::too_many_arguments)]
fn choreograph(
    scene: &mut Scene,
    index: usize,
    last: bool,
    gentle: bool,
    z_f: f32,
    rack_from: f32,
    canvas: (f32, f32),
    pivot: (f32, f32),
    arrive: f64,
    product: bool,
    rotation: Option<CameraRotation>,
) -> (usize, bool) {
    use crate::scene::PostKind;
    let dur = scene.duration_seconds;
    // (0.18) The fly-in starts when the content enters (ENTER), not at the
    // scene start, which overlaps the previous beat's fly-through.
    // (0.23 W4) On the product path it lands in time for the dead-air limit.
    let enter = scene.lifecycle.map_or(0.0, |l| l.enter);
    let (t_in, fly_len) = fly_in(product, index, enter, dur);
    let entry = t_in + fly_len;
    let exit = if last {
        dur
    } else {
        (dur - 0.55).max(entry + 0.6)
    };
    let amp: f32 = if gentle { 0.45 } else { 1.0 };
    // (0.18) A turning sphere (`revolve` layers, SphereGallery): the sphere is
    // the move, so the camera only pushes in calmly, and the layers ride the
    // sphere through depth: draw them far to near.
    let rides: std::collections::BTreeSet<&str> = scene
        .motions
        .iter()
        .filter(|m| matches!(m.op, MotionOp::Revolve { .. }))
        .map(|m| m.target.as_str())
        .collect();
    let revolving = !rides.is_empty();
    // Gentle (documentary): only the slow push-in, so the evidence stays
    // inside the frame; the cinematic look rotates the whole vocabulary.
    // (0.18) The cinematic vocabulary rotates beat by beat (the seed-based
    // pick was always the wide orbit: seed = (index + 1) * 7919 ≡ 3 mod 4):
    // wide orbit opener, push-in, truck, crane, ...
    // (0.23) Under a direction seed the move is the one `plan_camera` planned
    // for the beat (the seeded sequence, kept off neighbours' moves and off
    // moves that do not fit); the gentle / revolving override applies after it
    // as ever.
    let rotation = rotation.filter(|_| !(gentle || revolving));
    let pick = if gentle || revolving {
        0
    } else {
        rotation.map_or((index + 3) % 4, |r| r.planned % 4)
    };
    // A truck needs sideways room: full-width type would leave the canvas.
    // Without it the beat orbits instead (the push-in is its neighbour's move).
    let room = side_room(scene, canvas.0);
    let pick = match rotation {
        None if pick == 1 && room < TRUCK_ROOM_MIN => 3,
        None => pick,
        // The plan was made on this scene's own state: the orbit is a safety net.
        Some(r) if move_fits(pick, index, room, text_span(scene), r.safe) => pick,
        Some(_) => 3,
    };
    let truck = (0.8 * room).min(280.0);
    let (d0, d1) = match pick {
        0 => (0.0, 520.0),
        1 => (150.0, 320.0),
        2 => (-100.0, 280.0),
        _ => (120.0, 320.0),
    };
    let (d0, d1) = (d0 * amp, d1 * amp);
    let f = canvas.1 * 0.5 / (CINE_FOV.to_radians() * 0.5).tan();

    // Depth compensation at the main move's midpoint.
    let d_ref = 0.5 * (d0 + d1);
    // (0.18) About the pivot: the orbit, the projection and the zoom bursts
    // all centre on the focal layer.
    let c = pivot;
    for l in scene.layers.iter_mut() {
        // A layer on a sphere is measured at its front position (z = 0).
        let z = if rides.contains(l.id.as_str()) {
            0.0
        } else {
            l.z.unwrap_or(0.0)
        };
        // At dolly d a plane at z is magnified f / (z + f - d): scaling by
        // (z + f - d_ref) / f makes every plane, the hero's included, land at
        // its laid-out size at d_ref.
        let k = (z + f - d_ref) / f;
        if !(k.is_finite() && k > 0.05) {
            continue;
        }
        l.x = c.0 + (l.x - c.0) * k;
        l.y = c.1 + (l.y - c.1) * k;
        l.scale_x *= k;
        l.scale_y *= k;
    }

    let cam = scene.camera.get_or_insert_with(Camera::default);
    cam.pivot = Some([c.0, c.1]);
    // The choreography replaces the style's 2D drift; shakes and rolls stay.
    cam.motions
        .retain(|m| matches!(m.op, CameraOp::Shake { .. } | CameraOp::Roll { .. }));
    cam.perspective = Some(Perspective {
        fov_deg: CINE_FOV,
        focus_z: z_f - d0,
        aperture: if gentle { 3.5 } else { 9.0 },
        billboard: !gentle,
        depth_sort: revolving,
    });
    let mv = |start: f64, duration: f64, velocity: [f32; 2], op: CameraOp| CameraMotion {
        start,
        duration: duration.max(0.01),
        easing: Easing::Linear,
        velocity: Some(velocity.map(|v| v.clamp(0.0, 3.0))),
        op,
    };
    let back = 900.0 * amp;
    let fly = 1500.0 * amp.max(0.6);
    // Px/s of the main move, handed to the fly-in's end and the exit's start.
    let main_len = (exit - entry).max(0.01) as f32;
    let speed = (d1 - d0) / main_len;
    // Act 1: fly in, decelerating into the main move's speed; focus racks
    // from the background onto the focal plane.
    let entry_len = (entry - t_in).max(0.01) as f32;
    cam.motions.push(mv(
        t_in,
        entry - t_in,
        [2.6, speed * entry_len / back],
        CameraOp::Dolly {
            from: d0 - back,
            to: d0,
        },
    ));
    cam.motions.push(mv(
        t_in,
        entry - t_in,
        [0.0, 1.0],
        CameraOp::Focus {
            from: rack_from - (d0 - back),
            to: z_f - d0,
        },
    ));
    // Act 2: constant-speed push; focus tracks the focal plane exactly.
    if exit > entry + 0.05 {
        cam.motions.push(mv(
            entry,
            exit - entry,
            [1.0, 1.0],
            CameraOp::Dolly { from: d0, to: d1 },
        ));
        cam.motions.push(mv(
            entry,
            exit - entry,
            [1.0, 1.0],
            CameraOp::Focus {
                from: z_f - d0,
                to: z_f - d1,
            },
        ));
    }
    // The main move drifts at constant speed through the whole beat.
    let lin = [1.0, 1.0];
    match pick {
        0 => cam.motions.push(mv(
            0.0,
            dur,
            lin,
            CameraOp::Orbit {
                from: [-5.0 * amp, 1.5 * amp],
                to: [5.0 * amp, -amp],
            },
        )),
        // A pure truck: an orbit on top would swing the focal plane (off the
        // rotation centre once the camera has panned) out of focus.
        1 => cam.motions.push(mv(
            0.0,
            dur,
            lin,
            CameraOp::Track {
                from: [-truck, 0.0],
                to: [truck, 0.0],
            },
        )),
        2 => cam.motions.push(mv(
            0.0,
            dur,
            lin,
            CameraOp::Orbit {
                from: [2.0, 8.0],
                to: [-2.0, -2.0],
            },
        )),
        _ => cam.motions.push(mv(
            0.0,
            dur,
            lin,
            CameraOp::Orbit {
                from: [-20.0, 3.0],
                to: [20.0, -2.0],
            },
        )),
    }
    // Act 3: fly through into the next beat, picking up the push's speed.
    if !last && dur > exit + 0.05 {
        let exit_len = (dur - exit) as f32;
        let v0 = speed * exit_len / fly;
        cam.motions.push(mv(
            exit,
            dur - exit,
            [v0, 2.8],
            CameraOp::Dolly {
                from: d1,
                to: d1 + fly,
            },
        ));
        cam.motions.push(mv(
            exit,
            dur - exit,
            [v0, 2.8],
            CameraOp::Focus {
                from: z_f - d1,
                to: z_f - d1 - fly,
            },
        ));
        // (0.23 W4) The handoff bloom is on screen for BLOOM_MAX_S at most.
        let (bloom_at, bloom) = if product && dur - exit > BLOOM_MAX_S {
            (dur - BLOOM_MAX_S, BLOOM_MAX_S)
        } else {
            (exit, dur - exit)
        };
        scene.post.push(PostEffect {
            start: bloom_at,
            duration: bloom,
            easing: Easing::InCubic,
            from: 0.0,
            to: 1.0,
            kind: PostKind::Rays {
                center: [c.0, c.1],
                length: 0.55,
            },
        });
    }
    // The burst fades from where the previous beat's fly-through left it.
    if index > 0 && entry > arrive {
        let bloom = if product {
            (entry - arrive).min(BLOOM_MAX_S)
        } else {
            entry - arrive
        };
        scene.post.push(PostEffect {
            start: arrive,
            duration: bloom,
            easing: Easing::OutCubic,
            from: 1.0,
            to: 0.0,
            kind: PostKind::Rays {
                center: [c.0, c.1],
                length: 0.45,
            },
        });
    }
    (pick, rotation.is_some())
}

/// Seconds of the exit fade at the very end of a choreographed beat.
const EXIT_FADE: f64 = 0.22;

/// (0.18) The camera carries the beat out (fly-through): the lifecycle's own
/// exit moves and shrinks on the top-level layers (from ANTICIPATE) would pull
/// the content away from it, so they go, and the exit fades wait for the last
/// `EXIT_FADE` seconds so the content is seen rushing past the camera.
fn drop_exit_moves(scene: &mut Scene) {
    let Some(life) = scene.lifecycle else {
        return;
    };
    let dur = scene.duration_seconds;
    let tops: std::collections::BTreeSet<String> =
        scene.layers.iter().map(|l| l.id.clone()).collect();
    let exiting = |m: &Motion| tops.contains(&m.target) && m.start >= life.anticipate - 1e-3;
    scene.motions.retain(|m| {
        !(exiting(m) && matches!(m.op, MotionOp::Move { .. } | MotionOp::Scale { .. }))
    });
    for m in scene.motions.iter_mut() {
        if exiting(m) && matches!(m.op, MotionOp::Fade { .. }) {
            let d = EXIT_FADE.min(dur - life.anticipate).max(0.01);
            m.start = dur - d;
            m.duration = d;
        }
    }
}

/// Least sideways room (px) a beat must leave for the truck move.
const TRUCK_ROOM_MIN: f32 = 220.0;

/// (0.18) The least horizontal room between the canvas edges and the beat's
/// text and pictures at rest (decorative layers excluded; ancestors'
/// translations and scales, rotations ignored).
fn side_room(scene: &Scene, w: f32) -> f32 {
    fn walk(ls: &[Layer], o: (f32, f32), k: f32, w: f32, room: &mut f32) {
        for l in ls {
            if crate::layout_qa::is_decorative(&l.id, &l.kind) || !l.visible {
                continue;
            }
            let (lw, lh) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
            let x0 = o.0 + k * (l.x - l.anchor_x * lw);
            let y0 = o.1 + k * (l.y - l.anchor_y * lh);
            match &l.kind {
                LayerKind::Group { children } => {
                    walk(children, (x0, y0), k * l.scale_x.abs(), w, room)
                }
                LayerKind::Text(_) | LayerKind::Image { .. } | LayerKind::Svg { .. } => {
                    *room = room.min(x0).min(w - (x0 + k * lw));
                }
                _ => {}
            }
        }
    }
    let mut room = w;
    walk(&scene.layers, (0.0, 0.0), 1.0, w, &mut room);
    room.max(0.0)
}

/// (0.23) The leftmost and rightmost canvas x of the beat's text at rest (the
/// same walk as [`side_room`], text only), or `None` without text.
fn text_span(scene: &Scene) -> Option<(f32, f32)> {
    fn walk(ls: &[Layer], o: (f32, f32), k: f32, span: &mut Option<(f32, f32)>) {
        for l in ls {
            if crate::layout_qa::is_decorative(&l.id, &l.kind) || !l.visible {
                continue;
            }
            let (lw, lh) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
            let x0 = o.0 + k * (l.x - l.anchor_x * lw);
            let y0 = o.1 + k * (l.y - l.anchor_y * lh);
            match &l.kind {
                LayerKind::Group { children } => {
                    walk(children, (x0, y0), k * l.scale_x.abs(), span)
                }
                LayerKind::Text(_) => {
                    let (a, b) = (x0, x0 + k * lw);
                    *span = Some(span.map_or((a, b), |(s, e)| (s.min(a), e.max(b))));
                }
                _ => {}
            }
        }
    }
    let mut span = None;
    walk(&scene.layers, (0.0, 0.0), 1.0, &mut span);
    span
}

/// (0.18) Rest canvas centre of the `focal` layer's box (translations and
/// scales of its ancestors; rotations ignored).
fn focal_centre(scene: &Scene, focal: Option<&str>) -> Option<(f32, f32)> {
    fn walk(ls: &[Layer], o: (f32, f32), k: f32, id: &str) -> Option<(f32, f32)> {
        for l in ls {
            let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
            let x0 = o.0 + k * (l.x - l.anchor_x * w);
            let y0 = o.1 + k * (l.y - l.anchor_y * h);
            if l.id == id {
                return Some((x0 + 0.5 * k * w, y0 + 0.5 * k * h));
            }
            if let LayerKind::Group { children } = &l.kind {
                if let Some(c) = walk(children, (x0, y0), k * l.scale_x.abs(), id) {
                    return Some(c);
                }
            }
        }
        None
    }
    walk(&scene.layers, (0.0, 0.0), 1.0, focal?).filter(|c| c.0.is_finite() && c.1.is_finite())
}

/// (0.18) Depth of the top-level layer that holds `focal` (0 = the rest plane
/// when there is none).
fn focal_z(scene: &Scene, focal: Option<&str>) -> f32 {
    fn holds(l: &Layer, id: &str) -> bool {
        l.id == id
            || matches!(&l.kind, LayerKind::Group { children } if children.iter().any(|c| holds(c, id)))
    }
    focal
        .and_then(|id| scene.layers.iter().find(|l| holds(l, id)))
        .and_then(|l| l.z)
        .filter(|z| z.is_finite())
        .unwrap_or(0.0)
}

/// Builders put everything in the beat's stage group, but the perspective
/// camera projects top-level layers only. The stage is split, in its own
/// draw order, into top-level layers: runs of flat children stay together in
/// copies of the stage (same box, so they render exactly as before and layout
/// bindings keep their siblings), and each child carrying `z` or `tilt` gets
/// its own wrapper that pivots on the child's anchor and takes the depth and
/// tilt. Every piece gets the stage's motions. A stage whose layout bindings
/// would cross a depth child is left whole.
fn hoist_depth_layers(scene: &mut Scene) {
    let mut i = 0;
    while i < scene.layers.len() {
        let is_stage = {
            let id = &scene.layers[i].id;
            id.ends_with(".stage") || id.ends_with(".stage_front")
        };
        let pieces = if is_stage {
            split_stage(&scene.layers[i])
        } else {
            None
        };
        let Some(pieces) = pieces else {
            i += 1;
            continue;
        };
        let stage_id = scene.layers[i].id.clone();
        let copies: Vec<Motion> = pieces
            .iter()
            .filter(|p| p.id != stage_id)
            .flat_map(|p| {
                scene
                    .motions
                    .iter()
                    .filter(|m| m.target == stage_id)
                    .map(|m| Motion {
                        id: None,
                        target: p.id.clone(),
                        ..m.clone()
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        scene.motions.extend(copies);
        if !pieces.iter().any(|p| p.id == stage_id) {
            // No piece kept the stage id: its motions now live on the copies.
            scene.motions.retain(|m| m.target != stage_id);
        }
        // (0.18) `revolve` moves the plane in depth, which only top-level layers
        // take: a revolving stage child hands its rotation to its depth wrapper.
        for p in &pieces {
            let Some(child) = p.id.strip_suffix(".depth") else {
                continue;
            };
            // (0.19) A picture's arrival (its slide and its 3D turn) acts on
            // the plane too, so the whole card swings in together.
            let arrival = |m: &Motion| {
                matches!(m.op, MotionOp::Tilt { .. })
                    || (matches!(m.op, MotionOp::Move { .. })
                        && m.id.as_deref() == Some(crate::compiler::grammar::ARRIVAL_ID))
            };
            // (0.23) The wrapper already carries the stage's own motions (its
            // anticipation lift, its exit). A motion that would run over one of
            // them on the same channel stays on the child, which plays it inside
            // the wrapper as it does without depth: no layer ever holds two
            // overlapping motions on one channel.
            for i in 0..scene.motions.len() {
                let m = &scene.motions[i];
                if m.target != child || !(matches!(m.op, MotionOp::Revolve { .. }) || arrival(m)) {
                    continue;
                }
                let (channel, span) = (m.op.channel(), (m.start, m.start + m.duration));
                let clash = scene.motions.iter().enumerate().any(|(j, o)| {
                    j != i
                        && o.target == p.id
                        && o.op.channel() == channel
                        && spans_overlap(span, (o.start, o.start + o.duration))
                });
                if !clash {
                    scene.motions[i].target = p.id.clone();
                }
            }
        }
        let n = pieces.len();
        scene.layers.splice(i..=i, pieces);
        i += n;
    }
}

/// Overlap of two half-open motion spans, as the validator reads it (a
/// zero-length span only overlaps when it lies strictly inside the other).
fn spans_overlap(a: (f64, f64), b: (f64, f64)) -> bool {
    const EPS: f64 = 1e-9;
    let (a_zero, b_zero) = (a.1 - a.0 <= EPS, b.1 - b.0 <= EPS);
    match (a_zero, b_zero) {
        (true, true) => false,
        (true, false) => a.0 > b.0 + EPS && a.0 < b.1 - EPS,
        (false, true) => b.0 > a.0 + EPS && b.0 < a.1 - EPS,
        (false, false) => a.0 < b.1 - EPS && b.0 < a.1 - EPS,
    }
}

fn split_stage(stage: &Layer) -> Option<Vec<Layer>> {
    let LayerKind::Group { children } = &stage.kind else {
        return None;
    };
    let deep = |l: &Layer| (l.z.is_some() || l.tilt.is_some()) && l.layout.is_none();
    if !children.iter().any(deep) {
        return None;
    }
    // Draw order inside the stage (stable by z_index).
    let mut order: Vec<&Layer> = children.iter().collect();
    order.sort_by_key(|l| l.z_index);
    let parents: std::collections::BTreeSet<&str> = children
        .iter()
        .filter_map(|l| l.layout.as_ref().map(|b| b.parent.as_str()))
        .collect();
    if order
        .iter()
        .any(|l| deep(l) && parents.contains(l.id.as_str()))
    {
        return None;
    }
    let (sx0, sy0) = (
        stage.x - stage.anchor_x * stage.width,
        stage.y - stage.anchor_y * stage.height,
    );
    let mut pieces: Vec<Layer> = Vec::new();
    let mut run: Vec<Layer> = Vec::new();
    let mut runs = 0usize;
    let flush = |run: &mut Vec<Layer>, pieces: &mut Vec<Layer>, runs: &mut usize| {
        if run.is_empty() {
            return;
        }
        let mut copy = stage.clone();
        if *runs > 0 {
            copy.id = format!("{}.run{}", stage.id, runs);
        }
        copy.kind = LayerKind::Group {
            children: std::mem::take(run),
        };
        *runs += 1;
        pieces.push(copy);
    };
    for c in order {
        if !deep(c) {
            run.push(c.clone());
            continue;
        }
        flush(&mut run, &mut pieces, &mut runs);
        let mut c = c.clone();
        let (cw, ch) = (c.width * c.scale_x.abs(), c.height * c.scale_y.abs());
        let mut wrap = stage.clone();
        wrap.id = format!("{}.depth", c.id);
        wrap.x = sx0 + c.x;
        wrap.y = sy0 + c.y;
        wrap.width = cw.max(1.0);
        wrap.height = ch.max(1.0);
        wrap.anchor_x = c.anchor_x;
        wrap.anchor_y = c.anchor_y;
        wrap.z = c.z.take();
        wrap.tilt = c.tilt.take();
        c.x = c.anchor_x * cw;
        c.y = c.anchor_y * ch;
        wrap.kind = LayerKind::Group { children: vec![c] };
        pieces.push(wrap);
    }
    flush(&mut run, &mut pieces, &mut runs);
    // Layout bindings must find their parent inside the same piece.
    for p in &pieces {
        if let LayerKind::Group { children } = &p.kind {
            let ids: std::collections::BTreeSet<&str> =
                children.iter().map(|c| c.id.as_str()).collect();
            if children
                .iter()
                .filter_map(|c| c.layout.as_ref())
                .any(|b| !ids.contains(b.parent.as_str()))
            {
                return None;
            }
        }
    }
    Some(pieces)
}

fn persistent(kind: PostKind) -> PostEffect {
    PostEffect {
        start: 0.0,
        duration: 0.0,
        easing: Easing::Linear,
        from: 1.0,
        to: 1.0,
        kind,
    }
}

fn flash(start: f64, duration: f64, kind: PostKind) -> PostEffect {
    PostEffect {
        start,
        duration,
        easing: Easing::OutCubic,
        from: 1.0,
        to: 0.0,
        kind,
    }
}

fn shake(start: f64, duration: f64, trauma: f32, seed: u32) -> CameraMotion {
    CameraMotion {
        start,
        duration,
        easing: Easing::Linear,
        velocity: None,
        op: CameraOp::Shake {
            trauma: trauma.clamp(0.0, 1.0),
            frequency: 18.0,
            decay: 6.0,
            seed,
        },
    }
}

fn motion(target: &str, start: f64, duration: f64, op: MotionOp) -> Motion {
    Motion {
        id: None,
        target: target.to_string(),
        start,
        duration,
        easing: Easing::Linear,
        spring: None,
        op,
    }
}

/// Scene-local time of the spoken keyword (else of the primary value's first
/// term: its leading number phrase or first word) in this beat's sentence;
/// `None` without speech or a match. Matched through the shared anchor
/// lookup ([`anchor_time`]), so a plural or inflection finds its stem and a
/// number finds its spoken value ("$381 BILLION" ↔ "three hundred and eighty
/// one billion").
fn keyword_time(
    scene: &Scene,
    plan: &BeatPlan,
    beat: &Beat,
    speech: Option<&SpeechMap>,
) -> Option<f64> {
    let speech = speech?;
    let sentence = speech.sentences.iter().find(|s| s.beat == plan.index)?;
    let spoken: Vec<(String, f64)> = speech
        .words_in(sentence)
        .iter()
        .map(|w| (w.text.clone(), w.start - scene.start_seconds))
        .collect();
    let keyword = beat
        .keyword
        .as_deref()
        .and_then(|k| anchor_time(&spoken, &[k]));
    keyword.or_else(|| {
        let term = first_term(beat.primary.value()?)?;
        anchor_time(&spoken, &[term.as_str()])
    })
}

const HERO_SKIP: [&str; 11] = [
    "ghost",
    "backdrop",
    "kicker",
    "furniture",
    "rule",
    "caption",
    "plate",
    "env",
    "disc",
    "shadow",
    "kw.",
];

/// The beat's dominant picture: the largest image / svg layer anywhere in
/// the scene (builders nest them in the stage), else the largest top-level
/// text; never backdrop, ghost, furniture, discs or shadows.
fn hero_layer(layers: &[Layer]) -> Option<String> {
    hero_box(layers).map(|(id, _)| id)
}

/// The hero picture's id and canvas box `(x, y, w, h)` (stage children are
/// in canvas coordinates: the stage group spans the canvas from its origin).
fn hero_box(layers: &[Layer]) -> Option<(String, (f32, f32, f32, f32))> {
    fn walk<'a>(ls: &'a [Layer], ox: f32, oy: f32, out: &mut Vec<(&'a Layer, f32, f32)>) {
        for l in ls {
            out.push((l, ox, oy));
            if let LayerKind::Group { children } = &l.kind {
                let (x0, y0) = (
                    l.x - l.anchor_x * l.width * l.scale_x.abs(),
                    l.y - l.anchor_y * l.height * l.scale_y.abs(),
                );
                walk(children, ox + x0, oy + y0, out);
            }
        }
    }
    let skip = |id: &str| HERO_SKIP.iter().any(|s| id.contains(s));
    let mut all = Vec::new();
    walk(layers, 0.0, 0.0, &mut all);
    let area = |l: &Layer| l.width * l.height * l.scale_x.abs() * l.scale_y.abs();
    let pick = |pred: &dyn Fn(&LayerKind) -> bool| {
        all.iter()
            .filter(|(l, _, _)| l.visible && !skip(&l.id) && pred(&l.kind))
            .max_by(|a, b| area(a.0).total_cmp(&area(b.0)))
            .map(|(l, ox, oy)| {
                let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
                (
                    l.id.clone(),
                    (ox + l.x - l.anchor_x * w, oy + l.y - l.anchor_y * h, w, h),
                )
            })
    };
    pick(&|k| matches!(k, LayerKind::Image { .. } | LayerKind::Svg { .. }))
        .or_else(|| pick(&|k| matches!(k, LayerKind::Text(_))))
}

/// Short function words set small beside the big content words.
const SMALL_WORDS: [&str; 24] = [
    "a", "an", "the", "is", "are", "was", "of", "to", "in", "on", "at", "and", "or", "but", "it",
    "its", "for", "by", "as", "be", "so", "if", "we", "you",
];

/// A canvas box `(x0, y0, x1, y1)`.
type Span = (f32, f32, f32, f32);

/// Whether two boxes overlap, `margin` apart or less.
fn overlaps(a: Span, b: Span, margin: f32) -> bool {
    a.0 < b.2 + margin && b.0 < a.2 + margin && a.1 < b.3 + margin && b.1 < a.3 + margin
}

/// z of the kinetic words in the stage.
const KINETIC_Z: i32 = 15;

/// (0.23 A5a) Canvas boxes of the beat's content: the ink rows of its type, and
/// the boxes of its pictures, bars, cards and rules; what the spoken words must
/// not be set over. `under`: only the pictures drawn below that z (the words
/// are then drawn over them, which `text_over_subject` rules out); else all.
///
/// Each box is swept over the layer's own entrance: its `move` start and end,
/// and its `scale` when that is above 1 (a picture that pops in at 1.25 x), so
/// the words stay clear while it arrives. Left out: the look's own ghost word,
/// kinetic words, disc and floor shadow, every decorative layer
/// (`layout_qa::is_decorative`), and a layer that spans most of the canvas (a
/// ground, a plate). Stage children are in canvas coordinates.
fn content_boxes(
    layers: &[Layer],
    motions: &[Motion],
    canvas: (f32, f32),
    under: Option<i32>,
) -> Vec<Span> {
    /// What the walk reads and writes besides the layers.
    struct Pass<'a> {
        motions: &'a [Motion],
        canvas: (f32, f32),
        under: Option<i32>,
        out: Vec<Span>,
    }
    fn walk(ls: &[Layer], ox: f32, oy: f32, z: Option<i32>, pass: &mut Pass<'_>) {
        let (motions, canvas, under) = (pass.motions, pass.canvas, pass.under);
        for l in ls.iter().filter(|l| l.visible) {
            let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
            let (x0, y0) = (ox + l.x - l.anchor_x * w, oy + l.y - l.anchor_y * h);
            // The z the layer is drawn at among the stage's: its own, or its
            // ancestor's under the stage.
            let z_here = z.unwrap_or(l.z_index);
            if let LayerKind::Group { children } = &l.kind {
                let inner = if l.id.ends_with(".stage") {
                    None
                } else {
                    Some(z_here)
                };
                walk(children, x0, y0, inner, pass);
                continue;
            }
            if let Some(limit) = under {
                let picture = matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. });
                if !picture || z_here >= limit {
                    continue;
                }
            }
            let own = l.id.contains(".kw.")
                || l.id.contains("disc")
                || l.id.contains("shadow")
                || l.id.contains("ghost");
            if own || w <= 0.0 || h <= 0.0 || crate::layout_qa::is_decorative(&l.id, &l.kind) {
                continue;
            }
            let (top, bottom) = match &l.kind {
                LayerKind::Text(style) if style.text.trim().is_empty() => continue,
                LayerKind::Text(style) => style.ink.map_or((0.0, h), |ink| (ink.top, ink.bottom)),
                _ if w * h > 0.5 * canvas.0 * canvas.1 => continue,
                _ => (0.0, h),
            };
            let mut span = (x0, y0 + top, x0 + w, y0 + bottom);
            // The anchor the layer scales about, and the offsets it moves by.
            let (ax, ay) = (x0 + l.anchor_x * w, y0 + l.anchor_y * h);
            let mut grow = 1.0f32;
            let mut offsets: Vec<[f32; 2]> = vec![[0.0, 0.0]];
            for m in motions.iter().filter(|m| m.target == l.id) {
                match m.op {
                    MotionOp::Move { from, to } => offsets.extend([from, to]),
                    MotionOp::Scale { from, to, .. } => grow = grow.max(from).max(to),
                    _ => {}
                }
            }
            let grown = (
                ax + (span.0 - ax) * grow,
                ay + (span.1 - ay) * grow,
                ax + (span.2 - ax) * grow,
                ay + (span.3 - ay) * grow,
            );
            let base = span;
            for o in offsets {
                span.0 = span.0.min(base.0 + o[0]).min(grown.0 + o[0]);
                span.1 = span.1.min(base.1 + o[1]).min(grown.1 + o[1]);
                span.2 = span.2.max(base.2 + o[0]).max(grown.2 + o[0]);
                span.3 = span.3.max(base.3 + o[1]).max(grown.3 + o[1]);
            }
            pass.out.push(span);
        }
    }
    let mut pass = Pass {
        motions,
        canvas,
        under,
        out: Vec::new(),
    };
    walk(layers, 0.0, 0.0, None, &mut pass);
    pass.out
}

/// The layer `id` anywhere in `layers`.
fn find_layer<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    layers.iter().find_map(|l| {
        if l.id == id {
            return Some(l);
        }
        match &l.kind {
            LayerKind::Group { children } => find_layer(children, id),
            _ => None,
        }
    })
}

/// One word of a kinetic phrase, set and measured.
struct Word {
    block: Block,
    small: bool,
    /// Ink rows of the block (pixels from its top).
    ink: (f32, f32),
}

impl Word {
    /// The layer box's width (`typeset::text_layer`: measured x 1.02 + 2 px).
    fn box_w(&self) -> f32 {
        self.block.width() * 1.02 + 2.0
    }
}

/// A phrase set in rows within one width band.
struct Rows {
    words: Vec<Word>,
    /// Word indices per row, in order.
    rows: Vec<Vec<usize>>,
    row_h: Vec<f32>,
    total_h: f32,
}

/// Where a phrase's words go: `(x, y)` of each word's box, by word index.
type Spots = Vec<(f32, f32)>;

/// Set `phrase` in rows no wider than `max_row`, content words at `k` x 168 u
/// and function words at `k` x 66 u. `boxed`: the rows are measured by the
/// words' layer boxes (so a row fits a band exactly); else by the text widths,
/// as the studio look always set them.
fn set_rows(
    ctx: &Ctx,
    phrase: &[(String, f64)],
    k: f32,
    max_row: f32,
    boxed: bool,
    gap: f32,
) -> Rows {
    let u = ctx.u;
    let words: Vec<Word> = phrase
        .iter()
        .map(|(text, _)| {
            let clean: String = text
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, '%' | '$' | '₹' | '\'' | '-'))
                .collect();
            let small = SMALL_WORDS.contains(&clean.to_lowercase().as_str());
            let size = if small { 66.0 } else { 168.0 } * u * k;
            let fit = if boxed {
                (max_row - 2.0) / 1.02
            } else {
                max_row
            };
            let block = ctx.ts.fit_line(Voice::HEADLINE, &clean, fit, size);
            let ink = ctx
                .ts
                .ink(&block)
                .map_or((0.0, block.height()), |i| (i.top, i.bottom));
            Word { block, small, ink }
        })
        .collect();
    let width = |w: &Word| if boxed { w.box_w() } else { w.block.width() };
    let mut rows: Vec<Vec<usize>> = vec![Vec::new()];
    let mut row_w = 0.0;
    for (i, wd) in words.iter().enumerate() {
        let bw = width(wd);
        if !rows.last().is_some_and(Vec::is_empty) && row_w + gap + bw > max_row {
            rows.push(Vec::new());
            row_w = 0.0;
        }
        row_w += if row_w > 0.0 { gap } else { 0.0 } + bw;
        if let Some(r) = rows.last_mut() {
            r.push(i);
        }
    }
    let row_h: Vec<f32> = rows
        .iter()
        .map(|r| {
            r.iter()
                .map(|&i| words[i].block.height())
                .fold(0.0, f32::max)
        })
        .collect();
    let total_h = row_h.iter().sum();
    Rows {
        words,
        rows,
        row_h,
        total_h,
    }
}

impl Rows {
    /// Positions with the block's top at `top`, each row centred on `cx` and
    /// kept inside `band` (x of the left edge, x of the right edge); a row as
    /// wide as the band is centred in it. Returns the spots, the boxes the
    /// words' ink occupies and the boxes of their layers.
    fn place(
        &self,
        top: f32,
        cx: f32,
        band: (f32, f32),
        gap: f32,
        boxed: bool,
    ) -> (Spots, Vec<Span>, Vec<Span>) {
        let width = |w: &Word| if boxed { w.box_w() } else { w.block.width() };
        let mut spots: Spots = vec![(0.0, 0.0); self.words.len()];
        let mut spans = Vec::new();
        let mut boxes = Vec::new();
        let mut top = top;
        for (ri, r) in self.rows.iter().enumerate() {
            let rw: f32 = r.iter().map(|&i| width(&self.words[i])).sum::<f32>()
                + gap * (r.len().saturating_sub(1)) as f32;
            let (lo, hi) = (band.0, band.1 - rw);
            let mut x = if hi > lo {
                (cx - rw / 2.0).clamp(lo, hi)
            } else {
                band.0 + (band.1 - band.0 - rw) / 2.0
            };
            for &i in r {
                let wd = &self.words[i];
                let y = top + self.row_h[ri] - wd.block.height();
                spots[i] = (x, y);
                spans.push((x, y + wd.ink.0, x + wd.box_w(), y + wd.ink.1));
                boxes.push((x, y, x + wd.box_w(), y + wd.block.height()));
                x += width(wd) + gap;
            }
            top += self.row_h[ri];
        }
        (spots, spans, boxes)
    }
}

/// How the kinetic words of a beat are anchored.
struct Anchor {
    /// Centre of the words, horizontally.
    cx: f32,
    /// Centre of a phrase's block, vertically.
    cy: f32,
    /// A phrase's block never starts above this.
    floor: f32,
}

/// (0.23 A5a) Place one phrase. The studio look has always set it round the
/// hero's head (`cy`) in a band of 5 %..95 % of the width; that stays whenever
/// every word then lies inside the safe area (as the layout QA measures it) and
/// clear of `blocked` (the words' layer boxes 6 u apart from it). Otherwise the
/// phrase is set to fit the safe band and moved, as little as it takes, up or
/// down to the nearest place clear of `blocked`, smaller (down to 0.46 x) when
/// the free band is narrow.
/// With no such place the phrase is not shown (`skip_ok`), else it is set in the
/// safe band at the preferred height.
fn place_phrase(
    ctx: &Ctx,
    phrase: &[(String, f64)],
    at: &Anchor,
    blocked: &[Span],
    skip_ok: bool,
) -> Option<(Rows, Spots)> {
    let (w, u) = (ctx.w, ctx.u);
    let safe = ctx.frame.safe;
    let gap = 18.0 * u;
    let clear = |spans: &[Span]| {
        !spans
            .iter()
            .any(|s| blocked.iter().any(|b| overlaps(*s, *b, 6.0 * u)))
    };
    // What the layout QA allows beyond the safe edge: 2 u plus the box padding
    // (`layout_qa::box_padding`); `tol` false is the safe area itself.
    let inside = |spans: &[Span], tol: bool| {
        spans.iter().all(|s| {
            let t = if tol {
                2.0 * u + (0.02 * (s.2 - s.0) + 2.0) / 1.02
            } else {
                0.0
            };
            s.0 >= safe.x - t
                && s.2 <= safe.x + safe.w + t
                && s.1 >= safe.y - t
                && s.3 <= safe.y + safe.h + t
        })
    };
    // The studio's own band.
    let legacy = set_rows(ctx, phrase, 1.0, 0.9 * w, false, gap);
    let top = (at.cy - legacy.total_h / 2.0).max(at.floor);
    let (spots, spans, boxes) = legacy.place(top, at.cx, (0.05 * w, 0.95 * w), gap, false);
    if inside(&spans, true) && clear(&boxes) {
        return Some((legacy, spots));
    }
    // The safe band.
    let band = (safe.x, safe.x + safe.w);
    let step = 6.0 * u;
    let mut last: Option<(Rows, f32, f32)> = None;
    for k in [1.0f32, 0.8, 0.62, 0.46] {
        let rows = set_rows(ctx, phrase, k, safe.w, true, gap * k);
        let (lo, hi) = (safe.y, (safe.y + safe.h - rows.total_h).max(safe.y));
        let pref = (at.cy - rows.total_h / 2.0).max(at.floor).clamp(lo, hi);
        // Nearest first: pref, pref + step, pref - step, pref + 2 step, ...
        let tries = ((hi - lo) / step).ceil() as usize + 1;
        let mut found: Option<Spots> = None;
        'search: for n in 0..=tries {
            for sign in [1.0f32, -1.0] {
                if n == 0 && sign < 0.0 {
                    continue;
                }
                let y = pref + sign * n as f32 * step;
                if y < lo || y > hi {
                    continue;
                }
                let (spots, spans, boxes) = rows.place(y, at.cx, band, gap * k, true);
                if inside(&spans, false) && clear(&boxes) {
                    found = Some(spots);
                    break 'search;
                }
            }
        }
        if let Some(spots) = found {
            return Some((rows, spots));
        }
        last = Some((rows, pref, gap * k));
    }
    if skip_ok {
        return None;
    }
    // No clear band: the safe band at the preferred height (the words may
    // cross other type, but never the safe edge).
    match last {
        Some((rows, pref, g)) => {
            let (spots, _, _) = rows.place(pref, at.cx, band, g, true);
            Some((rows, spots))
        }
        None => Some((legacy, spots)),
    }
}

/// (0.23 A5a) The figure the look's ghost word shows ("$58" over a piggy bank):
/// its text, when the beat's picture or number carries it as a value. The
/// kinetic words replace the ghost word, so the figure is shown only where they
/// say it.
fn punch_figure(scene: &Scene, plan: &BeatPlan, beat: &Beat) -> Option<String> {
    let ghost = find_layer(&scene.layers, &format!("{}.ghost_punch", plan.prefix))?;
    let LayerKind::Text(style) = &ghost.kind else {
        return None;
    };
    let text = style.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let carries = std::iter::once(&beat.primary)
        .chain(beat.secondary.as_ref())
        .any(|s| {
            matches!(s.kind(), SubjectKind::Number | SubjectKind::Object)
                && s.value()
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case(&text))
        });
    (carries && text.chars().any(|c| c.is_ascii_digit())).then_some(text)
}

/// (Studio) The beat's spoken words as big kinetic type, phrases of up to three
/// words (breaking at punctuation and pauses), each word popping in at its
/// spoken time, the phrase clearing when the next one starts. Replaces the
/// builder's static punchword. They sit round the hero's head, between the disc
/// (z 8) and the picture (z 20), and (0.23 A5a) inside the safe area. Under a
/// direction seed (the product path) they never sit on the beat's content: a
/// picture, a figure, a bar, a card, the headline; they take the nearest free
/// band, smaller when it is narrow, and a phrase with no free band is not shown
/// (the narration's captions carry the words). A figure the words do not show
/// (the narrator never says it, or its words had no room) is set as a label
/// from ENTER, in the first free place under the kicker.
fn kinetic_words(ctx: &Ctx, scene: &mut Scene, plan: &BeatPlan, beat: &Beat, speech: &SpeechMap) {
    let Some(sentence) = speech.sentences.iter().find(|s| s.beat == plan.index) else {
        return;
    };
    let words: Vec<(String, f64)> = speech
        .words_in(sentence)
        .iter()
        .map(|w| (w.text.clone(), w.start - scene.start_seconds))
        .filter(|(t, _)| !t.trim().is_empty())
        .collect();
    if words.is_empty() {
        return;
    }
    let Some((_, (hx, hy, hw, hh))) = hero_box(&scene.layers) else {
        return;
    };
    let (w, u) = (ctx.w, ctx.u);
    let safe = ctx.frame.safe;
    // Phrases.
    let mut phrases: Vec<Vec<(String, f64)>> = Vec::new();
    for (i, (text, t)) in words.iter().enumerate() {
        let gap = i > 0 && *t - words[i - 1].1 > 0.9;
        let full = phrases.last().is_some_and(|p| p.len() >= 3);
        let punct = i > 0 && words[i - 1].0.ends_with(['.', ',', '!', '?', ';', ':']);
        if phrases.is_empty() || gap || full || punct {
            phrases.push(Vec::new());
        }
        if let Some(p) = phrases.last_mut() {
            p.push((text.clone(), *t));
        }
    }
    // Around the face / shoulders: the head occludes part of the words.
    let mut at = Anchor {
        cx: (hx + hw / 2.0).clamp(0.3 * w, 0.7 * w),
        cy: hy + 0.11 * hh,
        floor: safe.y + 0.06 * ctx.h,
    };
    // Under a direction seed (the product path) the words also keep clear of the
    // beat's content; without one only the safe area and the figure label below
    // move them, so a compile that passes its checks is not touched.
    let canvas = (ctx.w, ctx.h);
    let content = content_boxes(&scene.layers, &scene.motions, canvas, None);
    let seeded = ctx.direction_seed.is_some();
    // Without one, the pictures the words would be drawn over (a library prop
    // below them): the words on a picture are the defect `text_over_subject`.
    let mut blocked: Vec<Span> = if seeded {
        content.clone()
    } else {
        content_boxes(&scene.layers, &scene.motions, canvas, Some(KINETIC_Z))
    };
    let place_all = |at: &Anchor, blocked: &[Span]| -> Vec<Option<(Rows, Spots)>> {
        phrases
            .iter()
            .map(|p| place_phrase(ctx, p, at, blocked, seeded))
            .collect()
    };
    let mut placed = place_all(&at, &blocked);
    // The figure the words may not show.
    let mut layers: Vec<Layer> = Vec::new();
    let mut motions: Vec<Motion> = Vec::new();
    if let Some(text) = punch_figure(scene, plan, beat) {
        // Where it is said: the phrase holding its first word, if that is shown.
        let spoken_at = first_term(&text).and_then(|term| anchor_time(&words, &[term.as_str()]));
        let said = spoken_at
            .and_then(|t| phrases.iter().rposition(|p| p[0].1 <= t + 1e-6))
            .is_some_and(|pi| placed[pi].is_some());
        if !said {
            let block = ctx
                .ts
                .fit_line(Voice::HEADLINE, &text, 0.5 * safe.w / 1.02, 130.0 * u);
            let (bw, bh) = (block.width() * 1.02 + 2.0, block.height());
            // The first place under the top of the safe area clear of the content.
            let step = 6.0 * u;
            let y0 = safe.y + 0.04 * ctx.h;
            let mut y = y0;
            while y + bh <= safe.y + safe.h
                && content
                    .iter()
                    .any(|c| overlaps((safe.x, y, safe.x + bw, y + bh), *c, 14.0 * u))
            {
                y += step;
            }
            if y + bh > safe.y + safe.h {
                y = y0;
            }
            let id = format!("{}.figure", plan.prefix);
            let mut l = text_layer(id.clone(), &block, ctx.palette.accent, TextAlign::Left);
            l.x = safe.x;
            l.y = y;
            l.z_index = KINETIC_Z;
            l.anchor_x = 0.0;
            l.anchor_y = 0.0;
            blocked.push((l.x, l.y, l.x + l.width, l.y + l.height));
            at.floor = at.floor.max(l.y + l.height + 10.0 * u);
            // From ENTER when the narrator never says it; else when it is said,
            // like the words: a figure is never shown before its spoken word.
            let enter = plan.enter_at();
            let appear = spoken_at.map_or(enter, |t| (t - 0.03).max(enter));
            motions.push(motion(
                &id,
                appear,
                0.01,
                MotionOp::Fade { from: 0.0, to: 1.0 },
            ));
            motions.push(pop_in(&id, appear, 0.6));
            layers.push(l);
            placed = place_all(&at, &blocked);
        }
    }
    // The last phrase leaves before the outgoing transition.
    let end = (scene.duration_seconds - plan.overlap_out).max(0.1);
    let mut k = 0usize;
    for (pi, phrase) in phrases.iter().enumerate() {
        let Some((set, spots)) = placed[pi].take() else {
            continue;
        };
        let show = (phrase[0].1 - 0.04).max(0.0);
        let hide = phrases
            .get(pi + 1)
            .map(|n| (n[0].1 - 0.04).max(show + 0.1))
            .unwrap_or(end);
        for (i, wd) in set.words.iter().enumerate() {
            let id = format!("{}.kw.{k}", plan.prefix);
            k += 1;
            let mut l = text_layer(id.clone(), &wd.block, ctx.palette.ink, TextAlign::Left);
            l.x = spots[i].0;
            l.y = spots[i].1;
            l.z_index = KINETIC_Z;
            l.anchor_x = 0.0;
            l.anchor_y = 0.0;
            let at_t = (phrase[i].1 - 0.03).max(show);
            motions.push(motion(
                &id,
                at_t,
                0.01,
                MotionOp::Fade { from: 0.0, to: 1.0 },
            ));
            motions.push(pop_in(&id, at_t, if wd.small { 0.8 } else { 0.6 }));
            if hide < scene.duration_seconds - 0.05 {
                motions.push(motion(
                    &id,
                    hide,
                    0.01,
                    MotionOp::Fade { from: 1.0, to: 0.0 },
                ));
            }
            layers.push(l);
        }
    }
    // Into the stage (between the disc and the picture), replacing the
    // builder's static punchword.
    let punch = format!("{}.ghost_punch", plan.prefix);
    if let Some(stage) = scene
        .layers
        .iter_mut()
        .find(|l| l.id == format!("{}.stage", plan.prefix))
    {
        if let LayerKind::Group { children } = &mut stage.kind {
            children.retain(|c| c.id != punch);
            children.extend(layers);
        }
    }
    scene.motions.retain(|m| m.target != punch);
    scene.motions.extend(motions);
}

/// A word's spring pop from `from` x its size to full size.
fn pop_in(id: &str, at: f64, from: f32) -> Motion {
    let mut pop = motion(
        id,
        at,
        0.3,
        MotionOp::Scale {
            from,
            to: 1.0,
            axis: Default::default(),
        },
    );
    pop.spring = Some(SpringSpec {
        stiffness: 420.0,
        damping: 24.0,
        mass: 1.0,
    });
    pop
}

#[cfg(test)]
mod kinetic_tests {
    use super::*;

    fn layer(id: &str, kind: LayerKind, rect: (f32, f32, f32, f32)) -> Layer {
        let mut l = crate::compiler::base_layer(id.to_string(), rect, kind, 10);
        l.anchor_x = 0.0;
        l.anchor_y = 0.0;
        l
    }

    fn text(id: &str, rect: (f32, f32, f32, f32)) -> Layer {
        layer(
            id,
            LayerKind::Text(crate::scene::TextStyle {
                text: "WORD".to_string(),
                font_role: crate::scene::FontRole::Display,
                font_size: 100.0,
                font_weight: 900,
                italic: false,
                color: crate::scene::Color::rgb(0, 0, 0),
                align: TextAlign::Left,
                line_height: 1.0,
                letter_spacing: 0.0,
                ink: None,
                max_width: None,
                uppercase: false,
            }),
            rect,
        )
    }

    fn picture(id: &str, rect: (f32, f32, f32, f32)) -> Layer {
        layer(
            id,
            LayerKind::Image {
                asset: "asset.x".to_string(),
                fit: crate::scene::Fit::Contain,
                treatment: None,
                playback: None,
                insert: None,
            },
            rect,
        )
    }

    #[test]
    fn content_is_the_beats_type_and_pictures_not_the_looks_own_pieces() {
        let stage = layer(
            "b1.stage",
            LayerKind::Group {
                children: vec![
                    text("b1.head.0", (10.0, 20.0, 300.0, 100.0)),
                    picture("b1.hero.x", (0.0, 400.0, 500.0, 500.0)),
                    text("b1.ghost", (0.0, 0.0, 900.0, 900.0)),
                    text("b1.kw.0", (0.0, 500.0, 100.0, 100.0)),
                    picture("b1.disc", (0.0, 0.0, 400.0, 400.0)),
                    picture("backdrop.plate", (0.0, 0.0, 400.0, 400.0)),
                ],
            },
            (100.0, 200.0, 800.0, 800.0),
        );
        // Group children sit at the group's own corner: (100, 200) + their own.
        assert_eq!(
            content_boxes(&[stage], &[], (1080.0, 1920.0), None),
            vec![
                (110.0, 220.0, 410.0, 320.0),
                (100.0, 600.0, 600.0, 1100.0)
            ],
            "the headline and the picture: not the ghost word, the kinetic words, the disc or a plate"
        );
    }

    #[test]
    fn a_box_is_swept_over_its_entrance() {
        let pic = picture("b1.hero.x", (400.0, 600.0, 200.0, 200.0));
        let mut pic = pic;
        pic.anchor_x = 0.5;
        pic.anchor_y = 0.5;
        let motions = vec![
            motion(
                "b1.hero.x",
                0.0,
                0.7,
                MotionOp::Scale {
                    from: 1.25,
                    to: 1.0,
                    axis: Default::default(),
                },
            ),
            motion(
                "b1.hero.x",
                0.0,
                0.7,
                MotionOp::Move {
                    from: [0.0, 140.0],
                    to: [0.0, 0.0],
                },
            ),
        ];
        // Final box (300, 500)..(500, 700); popped in at 1.25 x about its centre
        // it reaches 25 px further on each side; it starts 140 px lower.
        assert_eq!(
            content_boxes(&[pic], &motions, (1080.0, 1920.0), None),
            vec![(275.0, 475.0, 525.0, 865.0)]
        );
    }

    #[test]
    fn boxes_that_touch_do_not_overlap_but_a_margin_makes_them() {
        let a = (0.0, 0.0, 100.0, 100.0);
        assert!(!overlaps(a, (100.0, 0.0, 200.0, 100.0), 0.0));
        assert!(overlaps(a, (100.0, 0.0, 200.0, 100.0), 1.0));
        assert!(overlaps(a, (50.0, 50.0, 150.0, 150.0), 0.0));
    }
}

#[cfg(test)]
mod dead_air_tests {
    use super::*;

    #[test]
    fn without_the_product_path_the_fly_in_is_what_it_always_was() {
        // From ENTER, 0.75 s (a quarter of a short beat), whatever the limit.
        assert_eq!(fly_in(false, 0, 0.25, 6.0), (0.25, 0.75));
        assert_eq!(fly_in(false, 3, 0.9, 6.0), (0.9, 0.75));
        assert_eq!(fly_in(false, 2, 0.4, 2.0), (0.4, 0.5));
        // ENTER is held to 30 % of the beat.
        assert_eq!(fly_in(false, 1, 5.0, 4.0), (1.2, 0.75));
    }

    #[test]
    fn a_fly_in_that_lands_in_time_lasts_the_product_length() {
        // Beat 2 entering at 0.3 or 0.65: lands at 0.75 / 1.1, inside 1.2 - 0.1.
        assert_eq!(fly_in(true, 1, 0.3, 6.0), (0.3, FLY_IN_PRODUCT_S));
        assert_eq!(fly_in(true, 1, 0.65, 6.0), (0.65, FLY_IN_PRODUCT_S));
    }

    #[test]
    fn a_late_fly_in_is_shortened_to_land_by_the_limit() {
        // Beat 5 entering at 0.9: 0.45 s would land at 1.35; the shortest
        // fly-in (0.4 s) still lands late there, so it starts ahead of ENTER.
        let (start, len) = fly_in(true, 4, 0.9, 4.3);
        assert_eq!(len, FLY_IN_MIN_S);
        assert!((start + len - 1.1).abs() < 1e-9 && start < 0.9);
        // ENTER at 0.7 lands at 1.15 with 0.45 s: 0.4 s, from ENTER, fits.
        let (start, len) = fly_in(true, 2, 0.7, 6.0);
        assert!(
            (start + len - 1.1).abs() < 1e-9 && (FLY_IN_MIN_S..FLY_IN_PRODUCT_S).contains(&len)
        );
    }

    #[test]
    fn the_opening_camera_moves_from_the_first_frames() {
        // The first beat enters at 0.25: the fly-in would land at 1.0; the
        // opening must read by 0.5, so it starts at once and lasts 0.4 s.
        let (start, len) = fly_in(true, 0, 0.25, 5.0);
        assert_eq!((start, len), (0.0, FLY_IN_MIN_S));
        // Never later than the limit, never shorter than the minimum, never
        // negative, for any ENTER.
        for i in 0..2 {
            for k in 0..30 {
                let enter = f64::from(k) * 0.05;
                let (start, len) = fly_in(true, i, enter, 6.0);
                let limit = if i == 0 { 0.5 } else { 1.2 };
                assert!(start >= 0.0 && (FLY_IN_MIN_S - 1e-12..=FLY_IN_S + 1e-12).contains(&len));
                assert!(start + len <= limit - FLY_IN_SLACK_S + 1e-9, "{i} {enter}");
            }
        }
    }

    #[test]
    fn a_short_beat_keeps_its_own_short_fly_in() {
        // A quarter of a 1.2 s beat is 0.3 s: already under the minimum.
        assert_eq!(fly_in(true, 1, 0.2, 1.2).1, 0.3);
    }
}
