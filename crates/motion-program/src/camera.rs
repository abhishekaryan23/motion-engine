//! Program camera and transitions.
//!
//! The program has one camera (zoom about the frame centre plus a pan) whose
//! state is a piecewise curve over program time: each move eases from the
//! current state to its target. Every scene gets the slice of that curve
//! that falls inside it, as linear camera segments sampled from the eased
//! curve, so one continuous move reads across cuts. Ground layers sit at
//! depth 0 and stay put. Transitions: `whip` adds a sideways camera throw
//! out of the old shot and into the new one with directional blur at the
//! cut; `colour_flip` sweeps an accent panel across and off over the cut.

use motion_core::easing::Easing;
use motion_core::scene::{
    Camera, CameraMotion, CameraOp, LayerKind, MotionOp, Perspective, PostEffect, PostKind, Scene,
};

use crate::lower::{r6, Ctx, Track};
use crate::program::{CameraVerb, MotionProgram, TransitionKind};
use crate::taste::{self, Shape};

/// Camera state: zoom, centre, orbit, dolly, roll, focus_z, aperture.
#[derive(Debug, Clone, Copy, PartialEq)]
struct State {
    zoom: f32,
    centre: [f32; 2],
    orbit: [f32; 2],
    dolly: f32,
    roll: f32,
    focus_z: f32,
    aperture: f32,
}

/// One move of the camera curve: from `a` at `t0` to `b` at `t1`.
#[derive(Debug, Clone, Copy)]
struct Seg {
    t0: f64,
    t1: f64,
    a: State,
    b: State,
    ease: Easing,
}

const MOVE_SECONDS: f64 = 0.7;
const WHIP_OUT: f64 = 0.12;
const WHIP_IN: f64 = 0.22;
/// Seconds an impact shake lasts.
const IMPACT_SHAKE: f64 = 0.36;

fn lerp(a: f32, b: f32, p: f32) -> f32 {
    a + (b - a) * p
}

impl Seg {
    fn at(&self, t: f64) -> State {
        let u = if self.t1 > self.t0 {
            ((t - self.t0) / (self.t1 - self.t0)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let p = self.ease.apply(u) as f32;
        State {
            zoom: lerp(self.a.zoom, self.b.zoom, p),
            centre: [
                lerp(self.a.centre[0], self.b.centre[0], p),
                lerp(self.a.centre[1], self.b.centre[1], p),
            ],
            orbit: [
                lerp(self.a.orbit[0], self.b.orbit[0], p),
                lerp(self.a.orbit[1], self.b.orbit[1], p),
            ],
            dolly: lerp(self.a.dolly, self.b.dolly, p),
            roll: lerp(self.a.roll, self.b.roll, p),
            focus_z: lerp(self.a.focus_z, self.b.focus_z, p),
            aperture: lerp(self.a.aperture, self.b.aperture, p),
        }
    }
}

/// The camera curve of a program, with each component's measured bounds
/// available for `frame`.
fn curve(p: &MotionProgram, cx: &Ctx, bounds: &dyn Fn(&str) -> Option<[f32; 4]>) -> Vec<Seg> {
    let rest = State {
        zoom: 1.0,
        centre: [cx.w / 2.0, cx.h / 2.0],
        orbit: [0.0, 0.0],
        dolly: 0.0,
        roll: 0.0,
        focus_z: 0.0,
        aperture: 0.0,
    };
    let mut evs: Vec<(f64, Option<&CameraVerb>, Option<TransitionKind>)> = Vec::new();
    for e in &p.camera {
        if let Some(t) = e.at {
            evs.push((t, Some(&e.op), None));
        }
    }
    for e in &p.transitions {
        if let (Some(t), TransitionKind::Whip | TransitionKind::Impact) = (e.at, e.kind) {
            evs.push((t, None, Some(e.kind)));
        }
    }
    evs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let tempo = cx.tempo.factor();
    let mut segs = Vec::new();
    let mut cur = rest;
    // Default camera (none authored): a gentle push on each chart's peak
    // (its pop_label) and each diagram's final node, cut back at the end
    // of the component.
    if p.camera.is_empty() {
        for c in &p.components {
            let peak = match &c.body {
                crate::program::ComponentBody::Chart { .. } => c
                    .timeline
                    .iter()
                    .find(|e| matches!(e.verb, crate::program::Verb::PopLabel))
                    .and_then(|e| e.at),
                crate::program::ComponentBody::Diagram { .. } => c
                    .timeline
                    .iter()
                    .filter(|e| matches!(e.verb, crate::program::Verb::Advance))
                    .filter_map(|e| e.at)
                    .fold(None, |m: Option<f64>, t| Some(m.map_or(t, |x| x.max(t)))),
                _ => None,
            };
            let (Some(t), Some(b)) = (peak, bounds(&c.id)) else {
                continue;
            };
            let band = crate::lower::free_band(p, c, cx);
            let (bw, bh) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
            let zoom = 1.15f32
                .min(cx.w * 0.88 / bw)
                .min((band[1] - band[0]) / bh)
                .max(1.0);
            if zoom <= 1.01 {
                continue;
            }
            let at = c.start + t;
            let end = p.end_of(c);
            // The pushed rect lands centred in the free band.
            let mid = (band[0] + band[1]) / 2.0;
            let target = State {
                zoom,
                centre: [
                    (b[0] + b[2]) / 2.0,
                    (b[1] + b[3]) / 2.0 - (mid - cx.h / 2.0) / zoom,
                ],
                ..rest
            };
            let d = (MOVE_SECONDS * tempo).min(end - at);
            segs.push(Seg {
                t0: at,
                t1: at + d,
                a: rest,
                b: target,
                ease: Easing::OutQuint,
            });
            segs.push(Seg {
                t0: end,
                t1: end,
                a: rest,
                b: rest,
                ease: Easing::Linear,
            });
        }
        segs.sort_by(|a, b| a.t0.total_cmp(&b.t0));
    }
    for (t, op, tr) in evs {
        if tr == Some(TransitionKind::Impact) {
            // A decaying shake around the current framing, one piece per
            // frame (a deterministic, non-repeating wobble).
            let fps = cx.fps;
            let n = (IMPACT_SHAKE * fps).round() as usize;
            let amp = cx.short * 0.012 / cur.zoom;
            let mut prev = cur;
            for k in 1..=n {
                let u = k as f32 / n as f32;
                let decay = (1.0 - u) * (1.0 - u);
                let tt = (t + k as f64 / fps) * 1.0;
                let jx = ((tt * 61.0).sin() * 0.6 + (tt * 37.0 + 1.3).sin() * 0.4) as f32;
                let jy = ((tt * 53.0 + 0.7).sin() * 0.6 + (tt * 29.0 + 2.1).sin() * 0.4) as f32;
                let next = if k == n {
                    cur
                } else {
                    State {
                        centre: [
                            cur.centre[0] + amp * decay * jx,
                            cur.centre[1] + amp * decay * jy,
                        ],
                        ..cur
                    }
                };
                segs.push(Seg {
                    t0: t + (k - 1) as f64 / fps,
                    t1: t + k as f64 / fps,
                    a: prev,
                    b: next,
                    ease: Easing::Linear,
                });
                prev = next;
            }
            continue;
        }
        if tr.is_some() {
            // Throw out to the right, cut, arrive from the left.
            let off = cx.w * 0.45 / cur.zoom;
            let out = State {
                centre: [cur.centre[0] + off, cur.centre[1]],
                ..cur
            };
            let inn = State {
                centre: [cur.centre[0] - off, cur.centre[1]],
                ..cur
            };
            segs.push(Seg {
                t0: t - WHIP_OUT,
                t1: t,
                a: cur,
                b: out,
                ease: Easing::InCubic,
            });
            segs.push(Seg {
                t0: t,
                t1: t + WHIP_IN,
                a: inn,
                b: cur,
                ease: Easing::OutQuint,
            });
            continue;
        }
        let target = match op.expect("camera op") {
            CameraVerb::Push { to } => {
                // Leans in on what is on screen, centred on it, and never
                // past the 5 % safe inset.
                let want = to.clamp(1.0, 1.8);
                match content_bounds(p, bounds, t) {
                    Some(b)
                        if want > 1.0
                            && safe_fit(cx, b) >= 1.085
                            && safe_fit(cx, b) - 1.0 >= 0.5 * (want - 1.0) =>
                    {
                        let zoom = want.min(safe_fit(cx, b)).max(1.0);
                        // Move toward the content only as far as the zoom
                        // needs: at 1.0 the frame stays put.
                        let k = if want > 1.0 {
                            (zoom - 1.0) / (want - 1.0)
                        } else {
                            0.0
                        };
                        let c = [(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0];
                        State {
                            zoom,
                            centre: [lerp(rest.centre[0], c[0], k), lerp(rest.centre[1], c[1], k)],
                            ..cur
                        }
                    }
                    // Content already near the safe edge: the authored push
                    // about the frame centre (layer boxes overstate type).
                    _ => State { zoom: want, ..cur },
                }
            }
            CameraVerb::Pan { to } => State {
                centre: taste::slot_center(*to, cx.aspect),
                ..cur
            },
            CameraVerb::Frame { component } => match bounds(component) {
                Some(b) => {
                    let (bw, bh) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
                    // Fill ~70 % of the frame, but stay inside the safe
                    // width and the band left by pinned top/bottom type.
                    let band = p
                        .components
                        .iter()
                        .find(|c| &c.id == component)
                        .map_or([cx.h * 0.08, cx.h * 0.92], |c| {
                            crate::lower::free_band(p, c, cx)
                        });
                    let zoom = (0.7 * cx.w / bw)
                        .min(0.7 * cx.h / bh)
                        .min(cx.w * 0.88 / bw)
                        .min((band[1] - band[0]) / bh)
                        .clamp(1.0, 1.8);
                    // The framed rect lands centred in the band.
                    let mid = (band[0] + band[1]) / 2.0;
                    let cy = (b[1] + b[3]) / 2.0 - (mid - cx.h / 2.0) / zoom;
                    State {
                        zoom,
                        centre: [(b[0] + b[2]) / 2.0, cy],
                        ..cur
                    }
                }
                None => cur,
            },
            CameraVerb::Orbit { yaw, pitch } => State {
                orbit: [*yaw, *pitch],
                ..cur
            },
            CameraVerb::Dolly { to } => State { dolly: *to, ..cur },
            CameraVerb::Roll { deg } => State { roll: *deg, ..cur },
            CameraVerb::Focus { z, aperture } => State {
                focus_z: *z,
                aperture: aperture.unwrap_or(cur.aperture),
                ..cur
            },
            CameraVerb::Reset => rest,
        };
        let d = MOVE_SECONDS * tempo;
        segs.push(Seg {
            t0: t,
            t1: t + d,
            a: cur,
            b: target,
            ease: Easing::OutQuint,
        });
        cur = target;
    }
    segs
}

/// Adds the program camera and the transitions to the lowered scenes.
pub(crate) fn apply(
    p: &MotionProgram,
    cx: &Ctx,
    scenes: &mut Vec<Scene>,
    bounds: &dyn Fn(&str) -> Option<[f32; 4]>,
) {
    let uses_25d = p.camera.iter().any(|e| {
        matches!(
            e.op,
            CameraVerb::Orbit { .. }
                | CameraVerb::Dolly { .. }
                | CameraVerb::Roll { .. }
                | CameraVerb::Focus { .. }
        )
    }) || p.components.iter().any(|c| {
        c.depth.is_some()
            || c.z.is_some()
            || c.tilt.is_some()
            || c.timeline
                .iter()
                .any(|m| matches!(m.verb, crate::program::Verb::Tilt { .. }))
            || matches!(c.body, crate::program::ComponentBody::Bento(_))
    }) || scenes
        .iter()
        .any(|s| s.layers.iter().any(|l| l.z.is_some() || l.tilt.is_some()));
    let has_roll = p
        .camera
        .iter()
        .any(|e| matches!(e.op, CameraVerb::Roll { .. }));

    let segs = curve(p, cx, bounds);
    let centre = [cx.w / 2.0, cx.h / 2.0];
    let rest = State {
        zoom: 1.0,
        centre,
        orbit: [0.0, 0.0],
        dolly: 0.0,
        roll: 0.0,
        focus_z: 0.0,
        aperture: 0.0,
    };
    if !segs.is_empty() {
        for s in scenes.iter_mut().filter(|s| s.camera.is_none()) {
            let (s0, s1) = (s.start_seconds, s.end_seconds());
            let mut motions = Vec::new();
            // State at the scene start (held from earlier moves).
            let state_at = |t: f64| {
                let mut st = rest;
                for g in &segs {
                    if g.t0 <= t {
                        st = g.at(t);
                    }
                }
                st
            };
            let first = state_at(s0);
            // Sample every move inside the scene into short linear pieces.
            let mut prev_t = s0;
            let mut prev = first;
            let mut times: Vec<f64> = Vec::new();
            for g in &segs {
                if g.t1 <= s0 || g.t0 >= s1 {
                    continue;
                }
                for k in 0..=8 {
                    let t = g.t0 + (g.t1 - g.t0) * k as f64 / 8.0;
                    if t > s0 && t < s1 {
                        times.push(t);
                    }
                }
            }
            times.sort_by(f64::total_cmp);
            times.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
            // A static start state is set by a zero-length move at 0.
            let push = |m: &mut Vec<CameraMotion>, t0: f64, t1: f64, a: State, b: State| {
                // End = the next piece's rounded start, so rounding never
                // makes consecutive pieces overlap.
                let dur = r6((r6(t1 - s0) - r6(t0 - s0)).max(0.0));
                m.push(CameraMotion {
                    start: r6(t0 - s0),
                    duration: dur,
                    easing: Easing::Linear,
                    velocity: None,
                    op: CameraOp::Push {
                        from: a.zoom,
                        to: b.zoom,
                    },
                });
                m.push(CameraMotion {
                    start: r6(t0 - s0),
                    duration: dur,
                    easing: Easing::Linear,
                    velocity: None,
                    op: CameraOp::Track {
                        from: [a.centre[0] - centre[0], a.centre[1] - centre[1]],
                        to: [b.centre[0] - centre[0], b.centre[1] - centre[1]],
                    },
                });
                if uses_25d {
                    m.push(CameraMotion {
                        start: r6(t0 - s0),
                        duration: dur,
                        easing: Easing::Linear,
                        velocity: None,
                        op: CameraOp::Orbit {
                            from: a.orbit,
                            to: b.orbit,
                        },
                    });
                    m.push(CameraMotion {
                        start: r6(t0 - s0),
                        duration: dur,
                        easing: Easing::Linear,
                        velocity: None,
                        op: CameraOp::Dolly {
                            from: a.dolly,
                            to: b.dolly,
                        },
                    });
                    m.push(CameraMotion {
                        start: r6(t0 - s0),
                        duration: dur,
                        easing: Easing::Linear,
                        velocity: None,
                        op: CameraOp::Focus {
                            from: a.focus_z,
                            to: b.focus_z,
                        },
                    });
                }
                if has_roll || a.roll != 0.0 || b.roll != 0.0 {
                    m.push(CameraMotion {
                        start: r6(t0 - s0),
                        duration: dur,
                        easing: Easing::Linear,
                        velocity: None,
                        op: CameraOp::Roll {
                            from: a.roll,
                            to: b.roll,
                        },
                    });
                }
            };
            if first != rest || !times.is_empty() {
                push(&mut motions, s0, s0, first, first);
            }
            for t in times {
                let st = state_at(t);
                push(&mut motions, prev_t, t, prev, st);
                prev_t = t;
                prev = st;
            }
            if motions.is_empty() {
                continue;
            }
            // Camera motions on one channel must not overlap: keep them
            // strictly sequential (zero-length start piece first).
            let perspective = if uses_25d {
                Some(Perspective {
                    fov_deg: 42.0,
                    focus_z: first.focus_z,
                    aperture: first.aperture,
                    billboard: false,
                    depth_sort: true,
                })
            } else {
                None
            };
            s.camera = Some(Camera {
                pivot: Some(centre),
                motions,
                perspective,
            });
        }
    }

    // Transitions.
    for (k, e) in p.transitions.iter().enumerate() {
        let Some(t) = e.at else { continue };
        match e.kind {
            TransitionKind::Whip => {
                for s in scenes.iter_mut() {
                    let (s0, s1) = (s.start_seconds, s.end_seconds());
                    for (a, b, from, to) in
                        [(t - WHIP_OUT, t, 0.0, 1.0), (t, t + WHIP_IN, 1.0, 0.0)]
                    {
                        if a >= s0 && b <= s1 {
                            s.post.push(PostEffect {
                                start: r6(a - s0),
                                duration: r6(b - a),
                                easing: Easing::Linear,
                                from,
                                to,
                                kind: PostKind::DirectionalBlur {
                                    angle_deg: 0.0,
                                    length: cx.w * 0.05,
                                },
                            });
                        }
                    }
                }
            }
            TransitionKind::Impact => {
                // Two-frame shockwave from the frame centre and a short
                // vertical motion blur, in a scene of their own.
                let f = 1.0 / cx.fps;
                let mut s = crate::lower::scene(
                    &format!("transition{k}.impact"),
                    t,
                    0.3,
                    Vec::new(),
                    Vec::new(),
                );
                s.post.push(PostEffect {
                    start: 0.0,
                    duration: r6(2.0 * f),
                    easing: Easing::OutCubic,
                    from: 0.18,
                    to: 1.0,
                    kind: PostKind::Shockwave {
                        center: [cx.w / 2.0, cx.h / 2.0],
                        radius: cx.w * 0.62,
                        width: cx.short * 0.07,
                        amplitude: cx.short * 0.035,
                    },
                });
                s.post.push(PostEffect {
                    start: 0.0,
                    duration: r6(3.0 * f),
                    easing: Easing::OutCubic,
                    from: 1.0,
                    to: 0.0,
                    kind: PostKind::DirectionalBlur {
                        angle_deg: 90.0,
                        length: cx.short * 0.03,
                    },
                });
                scenes.push(s);
            }
            TransitionKind::ColourFlip => {
                let half = 0.16 * cx.tempo.factor();
                for (side, t0, from, to, ax) in [
                    ("in", t - half, 0.0f32, 1.0f32, 0.0f32),
                    ("out", t, 1.0, 0.0, 1.0),
                ] {
                    let id = format!("transition{k}.{side}");
                    let mut l = crate::lower::layer(
                        &id,
                        LayerKind::Rectangle {
                            fill: cx.col.accent,
                            stroke: None,
                        },
                        [cx.w * ax, cx.h / 2.0],
                        [cx.w, cx.h],
                        100_000,
                    );
                    l.anchor_x = ax;
                    l.depth = Some(0.0);
                    let mut tr = Track::default();
                    tr.push(
                        &id,
                        0.0,
                        r6(half),
                        Shape::Ease(if side == "in" {
                            Easing::InCubic
                        } else {
                            Easing::OutCubic
                        }),
                        MotionOp::Scale {
                            from,
                            to,
                            axis: motion_core::scene::Axis::X,
                        },
                    );
                    scenes.push(crate::lower::scene(
                        &id,
                        t0.max(0.0),
                        half,
                        vec![l],
                        tr.motions,
                    ));
                }
            }
        }
    }

    if uses_25d {
        for s in scenes.iter_mut() {
            if s.id.starts_with("transition") {
                continue;
            }
            if let Some(cam) = s.camera.as_mut() {
                if cam.perspective.is_none() {
                    cam.perspective = Some(Perspective {
                        fov_deg: 42.0,
                        focus_z: 0.0,
                        aperture: 0.0,
                        billboard: false,
                        depth_sort: true,
                    });
                }
            } else {
                s.camera = Some(Camera {
                    pivot: Some(centre),
                    motions: Vec::new(),
                    perspective: Some(Perspective {
                        fov_deg: 42.0,
                        focus_z: 0.0,
                        aperture: 0.0,
                        billboard: false,
                        depth_sort: true,
                    }),
                });
            }
        }
    }
}

/// Grounds ignore the camera (depth 0): pans never reveal their edges.
pub(crate) fn pin_grounds(scenes: &mut [Scene], frame_area: f32, overlays: &[&str]) {
    for s in scenes {
        let overlay = overlays
            .iter()
            .any(|o| s.id == *o || s.id.starts_with(&format!("{o}.")));
        for l in &mut s.layers {
            // An explicit depth is the author's choice (a device group
            // that fills the canvas but must ride the camera).
            if l.depth.is_some() && !overlay {
                continue;
            }
            if overlay || l.width * l.height >= frame_area * 0.9 {
                l.depth = Some(0.0);
            }
        }
    }
}

/// The zoom at which bounds `b`, centred, reach the 5 % safe inset.
fn safe_fit(cx: &Ctx, b: [f32; 4]) -> f32 {
    let m = cx.short * 0.05 + 2.0;
    let (bw, bh) = ((b[2] - b[0]).max(1.0), (b[3] - b[1]).max(1.0));
    ((cx.w - 2.0 * m) / bw).min((cx.h - 2.0 * m) / bh)
}

/// The union of the content components on screen at `t` (grounds aside).
fn content_bounds(
    p: &MotionProgram,
    bounds: &dyn Fn(&str) -> Option<[f32; 4]>,
    t: f64,
) -> Option<[f32; 4]> {
    use crate::program::ComponentBody;
    p.components
        .iter()
        .filter(|c| {
            !matches!(
                c.body,
                ComponentBody::Backdrop { .. }
                    | ComponentBody::Field { .. }
                    | ComponentBody::Hud { .. }
            ) && c.start <= t
                && t < p.end_of(c)
        })
        .filter_map(|c| bounds(&c.id))
        .reduce(|q, b| {
            [
                q[0].min(b[0]),
                q[1].min(b[1]),
                q[2].max(b[2]),
                q[3].max(b[3]),
            ]
        })
}
