//! Timeline evaluator: `(MotionProject, frame) -> ResolvedFrame`.
//!
//! The frame number is authoritative: `time = frame / fps`, never accumulated.
//! Evaluation is a pure function of its inputs, so the same frame always
//! resolves to an identical state. No pixels are produced here.

use serde::Serialize;
use std::collections::HashMap;

mod perspective;

use self::perspective::PerspState;
use crate::easing::{self, Easing};
use crate::noise;
use crate::scene::{
    Asset, AssetKind, Axis, Camera, CameraMotion, CameraOp, Canvas, ClipInset, Color, Direction,
    Envelope, GlyphOrder, GlyphPose, KeyState, Layer, LayerKind, LayoutBinding, Motion, MotionOp,
    MotionProject, PostEffect, RevealMode, Scene, SharedElement, SpringSpec,
};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum TimelineError {
    #[error("canvas fps must be > 0")]
    ZeroFps,
    #[error("shared element '{element}' references unknown scene '{scene}'")]
    UnknownScene { element: String, scene: String },
}

/// 2D affine transform. Maps `(x, y)` to `(a*x + c*y + e, b*x + d*y + f)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn translate(x: f32, y: f32) -> Self {
        Affine {
            e: x,
            f: y,
            ..Self::IDENTITY
        }
    }
    pub fn scale(sx: f32, sy: f32) -> Self {
        Affine {
            a: sx,
            d: sy,
            ..Self::IDENTITY
        }
    }
    pub fn rotate_degrees(deg: f32) -> Self {
        let (s, c) = deg.to_radians().sin_cos();
        Affine {
            a: c,
            b: s,
            c: -s,
            d: c,
            e: 0.0,
            f: 0.0,
        }
    }
    /// `self · other` (apply `other` first, then `self`).
    pub fn then_apply(self, other: Affine) -> Affine {
        Affine {
            a: self.a * other.a + self.c * other.b,
            b: self.b * other.a + self.d * other.b,
            c: self.a * other.c + self.c * other.d,
            d: self.b * other.c + self.d * other.d,
            e: self.a * other.e + self.c * other.f + self.e,
            f: self.b * other.e + self.d * other.f + self.f,
        }
    }
    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedFrame<'a> {
    pub frame: u32,
    pub time_seconds: f64,
    pub width: u32,
    pub height: u32,
    pub background: Color,
    pub active_scenes: Vec<&'a str>,
    /// Active shared element ids.
    pub active_shared: Vec<&'a str>,
    /// Top-level layers in draw order (back to front).
    pub layers: Vec<ResolvedLayer<'a>>,
    /// (0.15) Post effects of the active scenes with non-zero strength, in
    /// scene order then list order; applied to the finished frame.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub post: Vec<ResolvedPost<'a>>,
}

/// (0.15) A post effect at this frame.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedPost<'a> {
    pub kind: &'a crate::scene::PostKind,
    /// 0..=1 (already eased).
    pub strength: f32,
    /// Project time (seconds), for per-frame noise.
    pub time: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedLayer<'a> {
    pub id: &'a str,
    /// Owning scene; `None` for shared elements.
    pub scene: Option<&'a str>,
    pub z_index: i32,
    /// Box size (after geometry animation).
    pub width: f32,
    pub height: f32,
    /// Local box space -> canvas.
    pub transform: Affine,
    /// Content space -> canvas (box transform plus ClipReveal content offset).
    pub content_transform: Affine,
    /// The layer's own opacity (0..=1); parents are composited separately.
    pub opacity: f32,
    /// Clip in box space, if any.
    pub clip: Option<ClipInset>,
    pub kind: &'a LayerKind,
    /// Text replacing the layer's own text this frame (Count), if any.
    pub text: Option<String>,
    /// Visible fraction of a polyline's length (Trim), if any.
    pub trim: Option<f32>,
    /// Colour tint (Tint channel, or inherited from a group); `None` when the
    /// layer has none or the amount is negligible.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tint: Option<ResolvedTint>,
    /// (0.11) Sprite frame (0-based) of an image layer whose asset is a
    /// sprite sequence. Time is scene-local for scene layers and
    /// `t - first track key time` for shared elements.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sprite_frame: Option<u32>,
    /// (0.11) Sprite frame of the screen insert when its asset is a sprite
    /// sequence (same clock as `sprite_frame`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_frame: Option<u32>,
    /// (0.14) Per-glyph poses of a text layer under `glyph_cascade`, one per
    /// non-whitespace char of the displayed text in reading order; `None` =
    /// all glyphs at rest.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyphs: Option<Vec<crate::scene::GlyphPose>>,
    /// (0.14) Polyline points replacing the layer's own (`path_morph`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f32; 2]>>,
    /// (0.16) Gaussian blur radius in px for this layer (depth of field).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blur: Option<f32>,
    /// (0.16) Row-major 3×3 homography from the layer's local box space to
    /// canvas px; when present it replaces `transform` for drawing
    /// (perspective tilt). `content_transform` is then box-relative content
    /// offset only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projective: Option<[f32; 9]>,
    /// Resolved children of a group, in draw order.
    pub children: Vec<ResolvedLayer<'a>>,
}

/// A resolved colour tint: rendered RGB moves toward `color` by `amount`
/// (0..=1, alpha untouched).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ResolvedTint {
    pub color: Color,
    pub amount: f32,
}

/// Smallest tint amount that is still applied.
const MIN_TINT: f32 = 1e-4;

fn tint_value(color: Color, from: f32, to: f32, p: f64) -> Option<ResolvedTint> {
    let amount = lerp(from, to, p).clamp(0.0, 1.0);
    (amount > MIN_TINT).then_some(ResolvedTint { color, amount })
}

/// Opacity below which a layer is dropped from the resolved frame.
const MIN_OPACITY: f32 = 1.0 / 512.0;

pub fn frame_time(fps: u32, frame: u32) -> f64 {
    frame as f64 / fps as f64
}

pub fn evaluate_frame(
    project: &MotionProject,
    frame: u32,
) -> Result<ResolvedFrame<'_>, TimelineError> {
    let fps = project.canvas.fps;
    if fps == 0 {
        return Err(TimelineError::ZeroFps);
    }
    let t = frame_time(fps, frame);
    let assets: AssetMap<'_> = project.assets.iter().map(|a| (a.id.as_str(), a)).collect();

    // (sort key, layer) for stable global ordering by (z, scene index, depth
    // rank, layer index). The depth rank is 0 unless the scene's perspective
    // camera has `depth_sort` (0.18).
    let mut top: Vec<((i32, usize, i64, usize), ResolvedLayer<'_>)> = Vec::new();
    let mut active_scenes = Vec::new();
    let mut post: Vec<ResolvedPost<'_>> = Vec::new();
    // Scene index -> world boxes of every resolved layer of that scene.
    let mut tables: HashMap<usize, WorldTable<'_>> = HashMap::new();

    for (si, scene) in project.scenes.iter().enumerate() {
        if !(t >= scene.start_seconds && t < scene.end_seconds()) {
            continue;
        }
        active_scenes.push(scene.id.as_str());
        let local_t = t - scene.start_seconds;
        let index = MotionIndex::new(&scene.motions);
        let camera = camera_state(scene, &project.canvas, local_t);
        let ctx = EvalCtx {
            t: local_t,
            sprite_t: local_t,
            abs_t: t,
            assets: &assets,
            motions: &index,
            envelopes: &project.envelopes,
            scene: Some(scene.id.as_str()),
            camera: camera.as_ref(),
            echo: true,
        };
        let depth_sort = camera
            .as_ref()
            .and_then(|c| c.persp.as_ref())
            .is_some_and(|p| p.depth_sort);
        let mut world = WorldTable::new();
        let mut siblings = Siblings::new();
        for (li, layer) in scene.layers.iter().enumerate() {
            // Echo ghosts come first (same key, stable sort): behind the layer.
            let drawn = resolve_layer(layer, ROOT, &ctx, &mut siblings, &mut world);
            let rank = if depth_sort { depth_rank(drawn.dz) } else { 0 };
            for r in drawn.layers {
                top.push(((layer.z_index, si, rank, li), r));
            }
        }
        tables.insert(si, world);
        for effect in &scene.post {
            let strength = post_strength(effect, local_t);
            if strength > MIN_POST_STRENGTH {
                post.push(ResolvedPost {
                    kind: &effect.kind,
                    strength,
                    time: t,
                });
            }
        }
    }

    let empty = MotionIndex::default();
    let mut active_shared = Vec::new();
    for (ei, element) in project.shared.iter().enumerate() {
        let Some(mut placement) = shared_placement(project, element, t, &mut tables)? else {
            continue;
        };
        placement.motion = shared_motion(project, &element.layer, t);
        active_shared.push(element.id.as_str());
        let ctx = EvalCtx {
            t: 0.0,
            sprite_t: t - placement.start,
            abs_t: t,
            assets: &assets,
            motions: &empty,
            envelopes: &project.envelopes,
            scene: None,
            camera: None,
            echo: false,
        };
        let mut world = WorldTable::new();
        let mut siblings = Siblings::new();
        if let Some(r) = resolve_layer_with(
            &element.layer,
            ROOT,
            &ctx,
            &mut siblings,
            &mut world,
            Some(placement),
        ) {
            top.push(((element.layer.z_index, project.scenes.len(), 0, ei), r.0));
        }
    }

    top.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(ResolvedFrame {
        post,
        frame,
        time_seconds: t,
        width: project.canvas.width,
        height: project.canvas.height,
        background: project.canvas.background,
        active_scenes,
        active_shared,
        layers: top.into_iter().map(|(_, l)| l).collect(),
    })
}

// ---------------------------------------------------------------------------
// Scene layers
// ---------------------------------------------------------------------------

#[derive(Default)]
struct MotionIndex<'m> {
    /// target id -> motions sorted by start.
    by_target: HashMap<&'m str, Vec<&'m Motion>>,
}

impl<'m> MotionIndex<'m> {
    fn new(motions: &'m [Motion]) -> Self {
        let mut by_target: HashMap<&str, Vec<&Motion>> = HashMap::new();
        for m in motions {
            by_target.entry(m.target.as_str()).or_default().push(m);
        }
        for list in by_target.values_mut() {
            // Stable sort keeps authoring order for equal starts.
            list.sort_by(|a, b| a.start.total_cmp(&b.start));
        }
        MotionIndex { by_target }
    }

    fn for_target(&self, id: &str) -> &[&'m Motion] {
        self.by_target.get(id).map(Vec::as_slice).unwrap_or(&[])
    }
}

#[derive(Clone, Copy)]
struct EvalCtx<'c, 'm> {
    /// Scene-local seconds.
    t: f64,
    /// Seconds driving sprite playback: scene-local for scene layers,
    /// `t - shared element start` for shared elements.
    sprite_t: f64,
    /// Project time (seconds) matching `t`; envelopes are sampled on it.
    abs_t: f64,
    /// Asset id -> asset (kinds for sprite lookup).
    assets: &'c AssetMap<'m>,
    motions: &'c MotionIndex<'m>,
    /// Project envelopes read by `pulse`.
    envelopes: &'c [Envelope],
    scene: Option<&'m str>,
    /// Camera of the owning scene at `t`; `None` = identity for every depth.
    camera: Option<&'c CameraState>,
    /// Emit `echo` ghosts for layers with an active echo motion. Off for the
    /// ghosts themselves (no recursion) and for layout-only passes.
    echo: bool,
}

/// Scene-space state carried down the layer tree.
#[derive(Clone, Copy)]
struct Inherit {
    /// Container content space -> scene space (no camera).
    container: Affine,
    /// Depth inherited from the nearest ancestor that sets one.
    depth: f32,
    /// Tint inherited from the nearest ancestor that has one.
    tint: Option<ResolvedTint>,
    /// Top-level layer (scene layer or shared element), not a group child.
    top: bool,
    /// (0.16) Perspective plane of the top-level ancestor, handed down to
    /// descendants (they are not projected individually).
    plane: Option<perspective::PlanePose>,
}

const ROOT: Inherit = Inherit {
    container: Affine::IDENTITY,
    depth: 1.0,
    tint: None,
    top: true,
    plane: None,
};

/// An already-resolved sibling: box space -> container space, and box size.
#[derive(Clone, Copy)]
struct Sibling {
    local: Affine,
    width: f32,
    height: f32,
}

/// Asset id -> asset, built once per evaluated frame.
type AssetMap<'a> = HashMap<&'a str, &'a Asset>;

/// Siblings of one container (group children / scene top level), by id.
type Siblings<'a> = HashMap<&'a str, Sibling>;

/// Scene-wide table: layer id -> (box space -> canvas incl. camera, width, height).
type WorldTable<'a> = HashMap<&'a str, (Affine, f32, f32)>;

/// Placement of a shared element for this frame, in canvas space.
#[derive(Clone)]
struct SharedPlacement {
    /// Absolute time of the element's first track key (its start).
    start: f64,
    /// Canvas position of the layer's anchor point.
    x: f32,
    y: f32,
    rotation: f32,
    opacity: f32,
    /// Uniform multiplier applied on top of the layer's own scale.
    scale: f32,
    /// Canvas pixels per scene pixel at the element's placement (camera zoom
    /// at its depth, or the bound parent's scale): scales motion offsets so a
    /// Move reads the same size as it would on a scene layer.
    unit: f32,
    /// Scene motions targeting the element this frame.
    motion: SharedMotion,
}

/// Motion channels that active scenes apply to a shared element (0.4). The
/// same channel semantics as scene layers, composed on top of the track state:
/// move offset (scene pixels), scale and fade multipliers, added rotation,
/// count text.
#[derive(Clone, Debug, PartialEq)]
struct SharedMotion {
    offset: (f32, f32),
    scale: (f32, f32),
    rotation: f32,
    opacity: f32,
    text: Option<String>,
    tint: Option<ResolvedTint>,
}

impl Default for SharedMotion {
    fn default() -> Self {
        SharedMotion {
            offset: (0.0, 0.0),
            scale: (1.0, 1.0),
            rotation: 0.0,
            opacity: 1.0,
            text: None,
            tint: None,
        }
    }
}

/// Channels of `layer` (a shared element's layer) driven by motions of the
/// scenes active at absolute time `t`. A motion applies only while its scene
/// is active. Per channel, the most recently started motion wins (holding its
/// end value); before any has started, the earliest upcoming one holds its
/// `from` value — the same hold semantics as scene layers, across scenes.
fn shared_motion(project: &MotionProject, layer: &Layer, t: f64) -> SharedMotion {
    // (absolute start, motion, owning scene start)
    let mut list: Vec<(f64, &Motion, f64)> = Vec::new();
    for scene in &project.scenes {
        if !(t >= scene.start_seconds && t < scene.end_seconds()) {
            continue;
        }
        for m in scene.motions.iter().filter(|m| m.target == layer.id) {
            list.push((scene.start_seconds + m.start, m, scene.start_seconds));
        }
    }
    let mut out = SharedMotion::default();
    if list.is_empty() {
        return out;
    }
    list.sort_by(|a, b| a.0.total_cmp(&b.0));
    let pick = |f: &dyn Fn(&MotionOp) -> bool| -> Option<(&Motion, f64)> {
        let matching = list.iter().filter(|(_, m, _)| f(&m.op));
        let current = matching.clone().filter(|(a, _, _)| *a <= t).last();
        match current.or_else(|| matching.clone().next()) {
            Some((a, m, _)) if *a <= t => Some((m, progress_at(m, t - a))),
            Some((_, m, _)) => Some((m, 0.0)),
            None => None,
        }
    };
    if let Some((m, p)) = pick(&|op| matches!(op, MotionOp::Move { .. })) {
        if let MotionOp::Move { from, to } = &m.op {
            out.offset = (lerp(from[0], to[0], p), lerp(from[1], to[1], p));
        }
    }
    if let Some((m, p)) = pick(&|op| matches!(op, MotionOp::Scale { .. })) {
        if let MotionOp::Scale { from, to, axis } = &m.op {
            let v = lerp(*from, *to, p);
            out.scale = match axis {
                Axis::Both => (v, v),
                Axis::X => (v, 1.0),
                Axis::Y => (1.0, v),
            };
        }
    }
    if let Some((m, p)) = pick(&|op| matches!(op, MotionOp::Rotate { .. })) {
        if let MotionOp::Rotate { from, to } = &m.op {
            out.rotation = lerp(*from, *to, p);
        }
    }
    if let Some((m, p)) = pick(&|op| matches!(op, MotionOp::Fade { .. })) {
        if let MotionOp::Fade { from, to } = &m.op {
            out.opacity = lerp(*from, *to, p);
        }
    }
    if let Some((m, p)) = pick(&|op| matches!(op, MotionOp::Tint { .. })) {
        if let MotionOp::Tint { color, from, to } = &m.op {
            out.tint = tint_value(*color, *from, *to, p);
        }
    }
    if matches!(layer.kind, LayerKind::Text(_)) {
        if let Some((m, p)) = pick(&|op| matches!(op, MotionOp::Count { .. })) {
            if let MotionOp::Count {
                from,
                to,
                decimals,
                grouping,
                prefix,
                suffix,
            } = &m.op
            {
                out.text = Some(format_count(
                    from + (to - from) * p,
                    *decimals,
                    *grouping,
                    prefix,
                    suffix,
                ));
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

/// Camera state of a scene at a scene-local time.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CameraState {
    zoom: f32,
    pan: [f32; 2],
    pivot: [f32; 2],
    /// (0.14) Total roll in degrees about the pivot (roll channel + shake).
    roll: f32,
    /// (0.14) Camera shake displacement in canvas px.
    shake: [f32; 2],
    /// (0.16) Perspective camera at this instant; `None` = 2D multi-plane.
    persp: Option<PerspState>,
}

impl CameraState {
    fn zoom_at(&self, depth: f32) -> f32 {
        self.zoom.powf(depth.max(0.0))
    }

    /// Canvas mapping for a plane at `depth`:
    /// `p' = pivot + zoom^k * (p - pan*k - pivot)`.
    fn affine(&self, depth: f32) -> Affine {
        let k = depth.max(0.0);
        let z = self.zoom_at(k);
        let (px, py) = (self.pan[0] * k, self.pan[1] * k);
        let parallax = Affine::translate(self.pivot[0], self.pivot[1])
            .then_apply(Affine::scale(z, z))
            .then_apply(Affine::translate(-self.pivot[0] - px, -self.pivot[1] - py));
        match self.view() {
            Some(view) => view.then_apply(parallax),
            None => parallax,
        }
    }

    /// Roll and shake, applied equally to every depth after the parallax;
    /// `None` when both are zero (keeps the plain camera byte-identical).
    fn view(&self) -> Option<Affine> {
        if self.roll == 0.0 && self.shake == [0.0, 0.0] {
            return None;
        }
        Some(
            Affine::translate(self.shake[0], self.shake[1])
                .then_apply(Affine::translate(self.pivot[0], self.pivot[1]))
                .then_apply(Affine::rotate_degrees(self.roll))
                .then_apply(Affine::translate(-self.pivot[0], -self.pivot[1])),
        )
    }
}

/// Progress of the camera motion active on a channel (same hold semantics as
/// layer motions).
fn camera_channel<F>(camera: &Camera, t: f64, pick: F) -> Option<(&CameraMotion, f64)>
where
    F: Fn(&CameraOp) -> bool,
{
    let mut list: Vec<&CameraMotion> = camera.motions.iter().filter(|m| pick(&m.op)).collect();
    list.sort_by(|a, b| a.start.total_cmp(&b.start));
    let mut current = None;
    for m in &list {
        if m.start <= t {
            current = Some(*m);
        }
    }
    match current {
        Some(m) => {
            let raw = if m.duration <= 0.0 {
                1.0
            } else {
                (t - m.start) / m.duration
            };
            let p = match m.velocity {
                Some(v) => hermite(raw, v),
                None => m.easing.apply(raw),
            };
            Some((m, p))
        }
        None => list.first().map(|m| (*m, 0.0)),
    }
}

/// (0.18) Cubic Hermite from 0 to 1 with normalised end slopes `v`
/// (`CameraMotion.velocity`); `u` is clamped to 0..=1.
fn hermite(u: f64, v: [f32; 2]) -> f64 {
    let u = if u.is_nan() { 0.0 } else { u.clamp(0.0, 1.0) };
    let (v0, v1) = (f64::from(v[0]), f64::from(v[1]));
    let (u2, u3) = (u * u, u * u * u);
    (u3 - 2.0 * u2 + u) * v0 + (3.0 * u2 - 2.0 * u3) + (u3 - u2) * v1
}

/// The scene's camera at scene-local time `t`, or `None` when it has none.
fn camera_state(scene: &Scene, canvas: &Canvas, t: f64) -> Option<CameraState> {
    let cam = scene.camera.as_ref()?;
    let mut zoom = 1.0;
    if let Some((m, p)) = camera_channel(cam, t, |op| matches!(op, CameraOp::Push { .. })) {
        if let CameraOp::Push { from, to } = &m.op {
            zoom = lerp(*from, *to, p);
        }
    }
    let mut pan = [0.0, 0.0];
    if let Some((m, p)) = camera_channel(cam, t, |op| matches!(op, CameraOp::Track { .. })) {
        if let CameraOp::Track { from, to } = &m.op {
            pan = [lerp(from[0], to[0], p), lerp(from[1], to[1], p)];
        }
    }
    let pivot = cam
        .pivot
        .unwrap_or([canvas.width as f32 * 0.5, canvas.height as f32 * 0.5]);
    let mut roll = 0.0f32;
    if let Some((m, p)) = camera_channel(cam, t, |op| matches!(op, CameraOp::Roll { .. })) {
        if let CameraOp::Roll { from, to } = &m.op {
            roll = lerp(*from, *to, p);
        }
    }
    let mut shake = [0.0f32, 0.0];
    for m in &cam.motions {
        let CameraOp::Shake {
            trauma,
            frequency,
            decay,
            seed,
        } = &m.op
        else {
            continue;
        };
        let elapsed = t - m.start;
        if !(elapsed >= 0.0 && elapsed <= m.duration) {
            continue;
        }
        let trauma = (*trauma as f64).clamp(0.0, 1.0);
        let gain = trauma * trauma * (-(*decay as f64) * elapsed).exp();
        let x = *frequency as f64 * elapsed;
        let reach = 0.025 * canvas.width as f64;
        shake[0] += (gain * reach * noise::smooth(*seed, x)) as f32;
        shake[1] += (gain * reach * noise::smooth(*seed ^ SHAKE_Y, x)) as f32;
        roll += (gain * CAMERA_SHAKE_ROLL_DEG * noise::smooth(*seed ^ SHAKE_ROT, x)) as f32;
    }
    let persp = cam.perspective.as_ref().and_then(|p| {
        let mut dolly = 0.0f32;
        if let Some((m, pr)) = camera_channel(cam, t, |op| matches!(op, CameraOp::Dolly { .. })) {
            if let CameraOp::Dolly { from, to } = &m.op {
                dolly = lerp(*from, *to, pr);
            }
        }
        let mut orbit = [0.0f32, 0.0];
        if let Some((m, pr)) = camera_channel(cam, t, |op| matches!(op, CameraOp::Orbit { .. })) {
            if let CameraOp::Orbit { from, to } = &m.op {
                orbit = [lerp(from[0], to[0], pr), lerp(from[1], to[1], pr)];
            }
        }
        let mut focus = None;
        if let Some((m, pr)) = camera_channel(cam, t, |op| matches!(op, CameraOp::Focus { .. })) {
            if let CameraOp::Focus { from, to } = &m.op {
                focus = Some(lerp(*from, *to, pr));
            }
        }
        PerspState::new(p, canvas, dolly, orbit, focus)
    });
    Some(CameraState {
        zoom,
        pan,
        pivot,
        roll,
        shake,
        persp,
    })
}

/// Seed salts decorrelating the y and rotation noise of a shake from x.
const SHAKE_Y: u32 = 0x9E37_79B9;
const SHAKE_ROT: u32 = 0x85EB_CA6B;
/// Camera roll at trauma 1 (degrees).
const CAMERA_SHAKE_ROLL_DEG: f64 = 3.0;

/// Eased (or spring) progress of `m` at `elapsed` seconds after its start.
fn progress_at(m: &Motion, elapsed: f64) -> f64 {
    shaped_progress(
        m.easing,
        m.spring,
        matches!(m.op, MotionOp::Count { .. }),
        elapsed,
        m.duration,
    )
}

/// Progress over `duration` with `easing`, or the spring when one is set.
/// `landing` (count motions) swaps a spring for OutQuint so numbers never
/// overshoot.
fn shaped_progress(
    easing: Easing,
    spring: Option<SpringSpec>,
    landing: bool,
    elapsed: f64,
    duration: f64,
) -> f64 {
    let raw = if duration <= 0.0 {
        1.0
    } else {
        elapsed / duration
    };
    match spring {
        None => easing.apply(raw),
        Some(_) if landing => Easing::OutQuint.apply(raw),
        Some(_) if duration <= 0.0 => 1.0,
        Some(s) => easing::spring_progress(elapsed, duration, s),
    }
}

/// Smallest post strength that is still emitted.
const MIN_POST_STRENGTH: f32 = 1e-4;

/// Strength of a post effect at scene-local time `t`: zero before `start`;
/// eased `from` → `to` inside the window; zero after it. `duration <= 0`
/// means "from `start` to the scene end at strength `to`" (persistent
/// effects such as grain).
fn post_strength(e: &PostEffect, t: f64) -> f32 {
    if t < e.start {
        return 0.0;
    }
    if e.duration <= 0.0 {
        return e.to.clamp(0.0, 1.0);
    }
    if t > e.start + e.duration {
        return 0.0;
    }
    lerp(e.from, e.to, e.easing.apply((t - e.start) / e.duration)).clamp(0.0, 1.0)
}

/// Progress of the motion active on a channel at time `t`: picks the latest
/// motion that has started, or the first one (at progress 0) if none has.
fn channel_progress<'m, F>(motions: &[&'m Motion], t: f64, pick: F) -> Option<(&'m Motion, f64)>
where
    F: Fn(&MotionOp) -> bool,
{
    let mut first = None;
    let mut current = None;
    for m in motions.iter().filter(|m| pick(&m.op)) {
        if first.is_none() {
            first = Some(*m);
        }
        if m.start <= t {
            current = Some(*m);
        }
    }
    match current {
        Some(m) => Some((m, progress_at(m, t - m.start))),
        None => first.map(|m| (m, 0.0)),
    }
}

fn lerp(a: f32, b: f32, p: f64) -> f32 {
    a + (b - a) * p as f32
}

/// A resolved layer tree plus the `echo` ghosts drawn behind it.
struct Drawn<'a> {
    /// Ghosts (farthest first) followed by the layer itself. Empty when the
    /// layer is culled and has no ghosts.
    layers: Vec<ResolvedLayer<'a>>,
    /// (0.18) View distance of the layer's plane under a perspective camera
    /// (what `depth_sort` orders by); `None` without one or when not drawn.
    /// Ghosts share their layer's distance so they stay right behind it.
    dz: Option<f64>,
}

/// Depth-sort rank of a plane at view distance `dz`: farther sorts first.
/// A layer without a distance (not drawn, no perspective) ranks 0.
fn depth_rank(dz: Option<f64>) -> i64 {
    dz.map_or(0, |dz| ((dz * 1000.0).round() as i64).saturating_neg())
}

/// Resolve a scene layer into draw-ordered output: its `echo` ghosts (farthest
/// first, so they sit behind it) followed by the layer itself. Empty when the
/// layer is culled and has no ghosts.
fn resolve_layer<'a>(
    layer: &'a Layer,
    inherit: Inherit,
    ctx: &EvalCtx<'_, 'a>,
    siblings: &mut Siblings<'a>,
    world: &mut WorldTable<'a>,
) -> Drawn<'a> {
    let main = resolve_layer_with(layer, inherit, ctx, siblings, world, None);
    let (mut layers, ghost_dz) = if ctx.echo {
        echo_ghosts(layer, inherit, ctx, siblings)
    } else {
        (Vec::new(), None)
    };
    let mut dz = ghost_dz;
    if let Some((r, main_dz)) = main {
        layers.push(r);
        dz = main_dz.or(dz);
    }
    Drawn { layers, dz }
}

/// The `echo` ghosts of `layer` at scene-local time `ctx.t`, farthest in the
/// past first. Each ghost is the layer (and its children) resolved `k *
/// spacing` seconds earlier with the current camera (the trail is locked to
/// the world), opacity × `decay^k`. Ghosts resolve against scratch tables and
/// never echo themselves. Also returns the view distance of the most recent
/// ghost (the layer's own is preferred by the caller).
fn echo_ghosts<'a>(
    layer: &'a Layer,
    inherit: Inherit,
    ctx: &EvalCtx<'_, 'a>,
    siblings: &Siblings<'a>,
) -> (Vec<ResolvedLayer<'a>>, Option<f64>) {
    let active = ctx
        .motions
        .for_target(&layer.id)
        .iter()
        .find_map(|m| match &m.op {
            MotionOp::Echo {
                count,
                spacing,
                decay,
            } if in_window(m, ctx.t) => Some((*count, *spacing, *decay)),
            _ => None,
        });
    let Some((count, spacing, decay)) = active else {
        return (Vec::new(), None);
    };
    let mut ghosts = Vec::new();
    let mut ghost_dz = None;
    for k in (1..=count).rev() {
        let dt = k as f64 * spacing;
        // A ghost from before the owning scene's start does not exist (the
        // tolerance keeps `t - k * spacing == 0` stable under rounding).
        if ctx.t - dt < -TIME_EPS {
            continue;
        }
        let ghost_ctx = EvalCtx {
            t: (ctx.t - dt).max(0.0),
            sprite_t: ctx.sprite_t - dt,
            abs_t: ctx.abs_t - dt,
            echo: false,
            ..*ctx
        };
        let mut scratch_siblings = siblings.clone();
        let mut scratch_world = WorldTable::new();
        if let Some((mut g, dz)) = resolve_layer_with(
            layer,
            inherit,
            &ghost_ctx,
            &mut scratch_siblings,
            &mut scratch_world,
            None,
        ) {
            g.opacity *= decay.powi(k as i32);
            if g.opacity >= MIN_OPACITY {
                ghosts.push(g);
                ghost_dz = dz;
            }
        }
    }
    (ghosts, ghost_dz)
}

/// Tolerance (seconds) for time comparisons that sit exactly on a boundary.
const TIME_EPS: f64 = 1e-9;

/// Whether scene-local `t` lies in `[start, start + duration]` of `m`.
fn in_window(m: &Motion, t: f64) -> bool {
    t >= m.start && t <= m.start + m.duration
}

/// Alignment point inside the child's own `w x h` box.
fn align_point(layer: &Layer, b: &LayoutBinding, w: f32, h: f32) -> (f32, f32) {
    let hf = b.horizontal.fraction();
    let vf = b.vertical.fraction();
    let ay = match &layer.kind {
        LayerKind::Text(t) => match t.ink {
            Some(ink) => match b.vertical {
                crate::scene::VAlign::Top => ink.top,
                crate::scene::VAlign::Center => (ink.top + ink.bottom) * 0.5,
                crate::scene::VAlign::Bottom => ink.bottom,
            },
            None => vf * h,
        },
        _ => vf * h,
    };
    (hf * w, ay)
}

/// Matching point of the parent's content box (parent size `pw x ph`), plus the
/// binding offset, in the parent's box space.
fn parent_point(b: &LayoutBinding, pw: f32, ph: f32) -> (f32, f32) {
    let pad = b.padding;
    (
        pad.left + b.horizontal.fraction() * (pw - pad.left - pad.right) + b.offset[0],
        pad.top + b.vertical.fraction() * (ph - pad.top - pad.bottom) + b.offset[1],
    )
}

/// Resolve a layer. `shared` (shared elements) supplies the final canvas-space
/// transform, opacity and scale multiplier instead of the layer's own placement.
/// Also returns the layer's perspective view distance (see `Drawn::dz`).
fn resolve_layer_with<'a>(
    layer: &'a Layer,
    inherit: Inherit,
    ctx: &EvalCtx<'_, 'a>,
    siblings: &mut Siblings<'a>,
    world: &mut WorldTable<'a>,
    shared: Option<SharedPlacement>,
) -> Option<(ResolvedLayer<'a>, Option<f64>)> {
    if !layer.visible {
        return None;
    }
    let motions = ctx.motions.for_target(&layer.id);
    let t = ctx.t;

    let mut base = TrackState::from_layer(layer);
    if let Some(sp) = &shared {
        base.opacity = sp.opacity * sp.motion.opacity;
        base.scale = sp.scale;
        base.rotation = sp.rotation + sp.motion.rotation;
    }
    let (mut x, mut y, mut w, mut h) = (base.x, base.y, layer.width, layer.height);
    if let Some((m, p)) =
        channel_progress(motions, t, |op| matches!(op, MotionOp::AccentExpand { .. }))
    {
        if let MotionOp::AccentExpand { to } = &m.op {
            // Chained expansions start from the previous expansion's end state.
            let from = previous_geometry(motions, m).unwrap_or((
                base.x,
                base.y,
                layer.width,
                layer.height,
            ));
            x = lerp(from.0, to.x, p);
            y = lerp(from.1, to.y, p);
            w = lerp(from.2, to.width, p);
            h = lerp(from.3, to.height, p);
        }
    }
    let mut move_offset = (0.0f32, 0.0f32);
    let mut has_move = false;
    if let Some((m, p)) = channel_progress(motions, t, |op| matches!(op, MotionOp::Move { .. })) {
        if let MotionOp::Move { from, to } = &m.op {
            move_offset = (lerp(from[0], to[0], p), lerp(from[1], to[1], p));
            has_move = true;
        }
    }
    // (0.18) Revolve adds the sphere offset to the offset channel (after
    // `move`); its depth component moves the plane (perspective, top level).
    let mut z_offset = 0.0f64;
    if shared.is_none() {
        if let Some((m, p)) =
            channel_progress(motions, t, |op| matches!(op, MotionOp::Revolve { .. }))
        {
            if let MotionOp::Revolve {
                radius,
                at,
                from,
                to,
            } = &m.op
            {
                let o = revolve_offset(*radius, *at, *from, *to, p);
                move_offset.0 += o[0] as f32;
                move_offset.1 += o[1] as f32;
                has_move = true;
                z_offset = o[2];
            }
        }
    }
    // (0.19) Tilt turns the plane in 3D (perspective, top level).
    let mut tilt_anim: Option<[f32; 2]> = None;
    if shared.is_none() {
        if let Some((m, p)) = channel_progress(motions, t, |op| matches!(op, MotionOp::Tilt { .. }))
        {
            if let MotionOp::Tilt { from, to } = &m.op {
                tilt_anim = Some([lerp(from[0], to[0], p), lerp(from[1], to[1], p)]);
            }
        }
    }
    // (0.14) Shakes add to the offset and rotation channels.
    let shake = layer_shake(motions, t);
    if let Some(s) = shake {
        move_offset.0 += s.dx;
        move_offset.1 += s.dy;
        has_move = true;
    }
    let (mut sx, mut sy) = (layer.scale_x * base.scale, layer.scale_y * base.scale);
    if let Some(sp) = &shared {
        sx *= sp.motion.scale.0;
        sy *= sp.motion.scale.1;
    }
    if let Some((m, p)) = channel_progress(motions, t, |op| matches!(op, MotionOp::Scale { .. })) {
        if let MotionOp::Scale { from, to, axis } = &m.op {
            let s = lerp(*from, *to, p);
            match axis {
                Axis::Both => {
                    sx *= s;
                    sy *= s;
                }
                Axis::X => sx *= s,
                Axis::Y => sy *= s,
            }
        }
    }
    // (0.14) Audio-reactive pulse multiplies the scale channel.
    for m in motions {
        if let MotionOp::Pulse { envelope, gain } = &m.op {
            if !in_window(m, t) {
                continue;
            }
            if let Some(e) = ctx.envelopes.iter().find(|e| e.id == *envelope) {
                let k = 1.0 + gain * e.at(ctx.abs_t);
                sx *= k;
                sy *= k;
            }
        }
    }
    let mut rot = base.rotation;
    if let Some((m, p)) = channel_progress(motions, t, |op| matches!(op, MotionOp::Rotate { .. })) {
        if let MotionOp::Rotate { from, to } = &m.op {
            rot += lerp(*from, *to, p);
        }
    }
    if let Some(s) = shake {
        rot += s.rotation;
    }
    let mut opacity = base.opacity;
    if let Some((m, p)) = channel_progress(motions, t, |op| matches!(op, MotionOp::Fade { .. })) {
        if let MotionOp::Fade { from, to } = &m.op {
            opacity *= lerp(*from, *to, p);
        }
    }
    let opacity = opacity.clamp(0.0, 1.0);
    // Own tint (a tint motion overrides, even at amount 0) else inherited.
    let own_tint: Option<Option<ResolvedTint>> = match &shared {
        Some(sp) => Some(sp.motion.tint),
        None => channel_progress(motions, t, |op| matches!(op, MotionOp::Tint { .. })).and_then(
            |(m, p)| match &m.op {
                MotionOp::Tint { color, from, to } => Some(tint_value(*color, *from, *to, p)),
                _ => None,
            },
        ),
    };
    let tint = own_tint.unwrap_or(inherit.tint);
    let mut culled = opacity < MIN_OPACITY;

    let mut clip = layer.clip;
    if let Some((m, p)) =
        channel_progress(motions, t, |op| matches!(op, MotionOp::MaskReveal { .. }))
    {
        if let MotionOp::MaskReveal { direction, mode } = &m.op {
            let reveal = mask_inset(*direction, *mode, p as f32);
            clip = Some(combine_insets(clip.unwrap_or_default(), reveal));
        }
    }
    let mut content_offset = (0.0f32, 0.0f32);
    if let Some((m, p)) =
        channel_progress(motions, t, |op| matches!(op, MotionOp::ClipReveal { .. }))
    {
        if let MotionOp::ClipReveal { direction, mode } = &m.op {
            content_offset = clip_reveal_offset(*direction, *mode, p as f32, w, h);
            clip = Some(clip.unwrap_or_default());
        }
    }
    if let Some(c) = clip {
        if c.left + c.right >= 1.0 || c.top + c.bottom >= 1.0 {
            culled = true; // fully clipped
        }
    }
    if sx.abs() < 1e-6 || sy.abs() < 1e-6 {
        culled = true;
    }

    // Placement inside the container (box space -> container space).
    let bound = match (&shared, layer.layout.as_ref()) {
        (None, Some(b)) => siblings.get(b.parent.as_str()).map(|s| (b, *s)),
        _ => None,
    };
    let local = if let Some(sp) = &shared {
        let o = sp.motion.offset;
        Affine::translate(sp.x + o.0 * sp.unit, sp.y + o.1 * sp.unit)
            .then_apply(Affine::rotate_degrees(rot))
            .then_apply(Affine::scale(sx, sy))
            .then_apply(Affine::translate(-layer.anchor_x * w, -layer.anchor_y * h))
    } else if let Some((b, parent)) = bound {
        let (px, py) = parent_point(b, parent.width, parent.height);
        let (ax, ay) = align_point(layer, b, w, h);
        parent
            .local
            .then_apply(Affine::translate(px + move_offset.0, py + move_offset.1))
            .then_apply(Affine::rotate_degrees(rot))
            .then_apply(Affine::scale(sx, sy))
            .then_apply(Affine::translate(-ax, -ay))
    } else {
        if has_move {
            x += move_offset.0;
            y += move_offset.1;
        }
        Affine::translate(x, y)
            .then_apply(Affine::rotate_degrees(rot))
            .then_apply(Affine::scale(sx, sy))
            .then_apply(Affine::translate(-layer.anchor_x * w, -layer.anchor_y * h))
    };
    let depth = layer.depth.unwrap_or(inherit.depth).max(0.0);
    let scene_space = inherit.container.then_apply(local);
    let mut projective = None;
    let mut blur = None;
    let mut plane = None;
    let mut view_dz = None;
    let transform = match ctx.camera {
        Some(c) => match &c.persp {
            Some(p) => {
                match perspective::place(
                    c,
                    p,
                    layer,
                    &inherit,
                    scene_space,
                    (w, h),
                    perspective::PlaneAnim {
                        z_offset,
                        tilt: tilt_anim,
                    },
                ) {
                    Some(placed) => {
                        projective = placed.projective;
                        blur = placed.blur;
                        view_dz = placed.dz;
                        plane = Some(placed.pose);
                        placed.transform
                    }
                    // Behind the camera (or an edge-on plane): not drawn.
                    None => {
                        culled = true;
                        scene_space
                    }
                }
            }
            None => c.affine(depth).then_apply(scene_space),
        },
        None => scene_space,
    };
    // A homography replaces `transform`; the content offset is then relative
    // to the layer's box.
    let content_transform = if projective.is_some() {
        Affine::translate(content_offset.0, content_offset.1)
    } else {
        transform.then_apply(Affine::translate(content_offset.0, content_offset.1))
    };

    // Recorded before culling so later siblings / shared elements can bind to
    // a layer that is currently invisible on screen.
    siblings.insert(
        &layer.id,
        Sibling {
            local,
            width: w,
            height: h,
        },
    );
    world.insert(&layer.id, (transform, w, h));

    let children: Vec<ResolvedLayer<'a>> = match &layer.kind {
        LayerKind::Group { children } => {
            let child_inherit = Inherit {
                container: scene_space
                    .then_apply(Affine::translate(content_offset.0, content_offset.1)),
                depth,
                tint,
                top: false,
                plane,
            };
            let mut child_siblings = Siblings::new();
            let mut resolved: Vec<((i32, usize), ResolvedLayer<'a>)> = Vec::new();
            for (i, c) in children.iter().enumerate() {
                for r in resolve_layer(c, child_inherit, ctx, &mut child_siblings, world).layers {
                    resolved.push(((c.z_index, i), r));
                }
            }
            resolved.sort_by(|a, b| a.0.cmp(&b.0));
            resolved.into_iter().map(|(_, r)| r).collect()
        }
        _ => Vec::new(),
    };
    if culled {
        return None;
    }

    let text = match &layer.kind {
        LayerKind::Text(_) if shared.is_some() => shared.and_then(|sp| sp.motion.text),
        LayerKind::Text(_) => {
            channel_progress(motions, t, |op| matches!(op, MotionOp::Count { .. })).and_then(
                |(m, p)| match &m.op {
                    MotionOp::Count {
                        from,
                        to,
                        decimals,
                        grouping,
                        prefix,
                        suffix,
                    } => Some(format_count(
                        from + (to - from) * p,
                        *decimals,
                        *grouping,
                        prefix,
                        suffix,
                    )),
                    _ => None,
                },
            )
        }
        _ => None,
    };
    let trim = match &layer.kind {
        LayerKind::Polyline { .. } => {
            channel_progress(motions, t, |op| matches!(op, MotionOp::Trim { .. })).and_then(
                |(m, p)| match &m.op {
                    MotionOp::Trim { from, to } => Some(lerp(*from, *to, p).clamp(0.0, 1.0)),
                    _ => None,
                },
            )
        }
        _ => None,
    };

    let glyphs = match &layer.kind {
        LayerKind::Text(style) => glyph_poses(motions, t, text.as_deref().unwrap_or(&style.text)),
        _ => None,
    };
    let points = match &layer.kind {
        LayerKind::Polyline { points, closed, .. } => morph_points(motions, t, points, *closed),
        _ => None,
    };

    let (sprite_frame, insert_frame) = match &layer.kind {
        LayerKind::Image {
            asset,
            playback,
            insert,
            ..
        } => (
            sprite_spec(ctx.assets, asset)
                .map(|spec| crate::scene::sprite_frame(spec, playback.as_ref(), ctx.sprite_t)),
            insert.as_ref().and_then(|ins| {
                sprite_spec(ctx.assets, &ins.asset).map(|spec| {
                    crate::scene::sprite_frame(spec, ins.playback.as_ref(), ctx.sprite_t)
                })
            }),
        ),
        _ => (None, None),
    };

    let resolved = ResolvedLayer {
        glyphs,
        points,
        blur,
        projective,
        id: &layer.id,
        scene: ctx.scene,
        z_index: layer.z_index,
        width: w,
        height: h,
        transform,
        content_transform,
        opacity,
        clip,
        kind: &layer.kind,
        text,
        trim,
        tint,
        sprite_frame,
        insert_frame,
        children,
    };
    Some((resolved, view_dz))
}

/// (0.18) Offset `[x, y, z]` of a point on a sphere of `radius` px turned by
/// `[yaw, pitch]` = lerp(`from`, `to`, `p`) degrees (`MotionOp::Revolve`):
/// `radius * Rx(pitch) * Ry(yaw) * dir` with
/// `dir = (sin lon cos lat, -sin lat, -cos lon cos lat)` for `at = [lon, lat]`.
fn revolve_offset(radius: f32, at: [f32; 2], from: [f32; 2], to: [f32; 2], p: f64) -> [f64; 3] {
    let (lon, lat) = (f64::from(at[0]).to_radians(), f64::from(at[1]).to_radians());
    let dir = [lon.sin() * lat.cos(), -lat.sin(), -lon.cos() * lat.cos()];
    let turn = |i: usize| f64::from(from[i]) + (f64::from(to[i]) - f64::from(from[i])) * p;
    let (sy, cy) = turn(0).to_radians().sin_cos();
    let (sp, cp) = turn(1).to_radians().sin_cos();
    // Ry(yaw), then Rx(pitch).
    let y = [dir[0] * cy - dir[2] * sy, dir[1], dir[0] * sy + dir[2] * cy];
    let x = [y[0], y[1] * cp + y[2] * sp, -y[1] * sp + y[2] * cp];
    let r = f64::from(radius);
    [r * x[0], r * x[1], r * x[2]]
}

/// The sprite spec of `asset` when it is a sprite-sequence asset.
fn sprite_spec<'a>(assets: &AssetMap<'a>, asset: &str) -> Option<&'a crate::scene::SpriteSpec> {
    let a = assets.get(asset)?;
    if a.kind == AssetKind::SpriteSequence {
        a.sprite.as_ref()
    } else {
        None
    }
}

/// Format a counting number: rounds half away from zero to `decimals`,
/// optionally groups integer digits by thousands with `,`, and wraps the result
/// in `prefix`/`suffix`. Negative values render as `-` followed by the prefix
/// (`-$1,234.5%`); a value that rounds to zero never gets a sign.
pub fn format_count(
    value: f64,
    decimals: u8,
    grouping: bool,
    prefix: &str,
    suffix: &str,
) -> String {
    let decimals = decimals.min(12) as usize;
    let value = if value.is_finite() { value } else { 0.0 };
    let scaled = (value.abs() * 10f64.powi(decimals as i32)).round();
    let digits_all = format!("{scaled:.0}");
    let negative = value < 0.0 && scaled != 0.0;

    let padded = if digits_all.len() <= decimals {
        format!(
            "{}{}",
            "0".repeat(decimals + 1 - digits_all.len()),
            digits_all
        )
    } else {
        digits_all
    };
    let (int_part, frac_part) = padded.split_at(padded.len() - decimals);

    let mut int_out = String::with_capacity(int_part.len() + int_part.len() / 3);
    for (i, ch) in int_part.chars().enumerate() {
        if grouping && i > 0 && (int_part.len() - i) % 3 == 0 {
            int_out.push(',');
        }
        int_out.push(ch);
    }

    let mut out = String::new();
    if negative {
        out.push('-');
    }
    out.push_str(prefix);
    out.push_str(&int_out);
    if decimals > 0 {
        out.push('.');
        out.push_str(frac_part);
    }
    out.push_str(suffix);
    out
}

fn previous_geometry(motions: &[&Motion], current: &Motion) -> Option<(f32, f32, f32, f32)> {
    let mut prev = None;
    for m in motions {
        if std::ptr::eq(*m, current) {
            break;
        }
        if let MotionOp::AccentExpand { to } = &m.op {
            prev = Some((to.x, to.y, to.width, to.height));
        }
    }
    prev
}

// ---------------------------------------------------------------------------
// MotionOp 2.0 (0.14): shake, glyph cascade, path morph
// ---------------------------------------------------------------------------

/// Summed shake displacement of a layer: px offsets and degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LayerShake {
    dx: f32,
    dy: f32,
    rotation: f32,
}

/// Sum of the shakes of `motions` active at scene-local `t`, or `None` when no
/// shake window contains `t`. Per shake, with `e = t - start`:
/// `amplitude * smooth(seed, frequency * e) * exp(-decay * e)` (independent
/// noise for x, y and rotation).
fn layer_shake(motions: &[&Motion], t: f64) -> Option<LayerShake> {
    let mut total: Option<LayerShake> = None;
    for m in motions {
        let MotionOp::Shake {
            amplitude,
            rotation,
            frequency,
            decay,
            seed,
        } = &m.op
        else {
            continue;
        };
        if !in_window(m, t) {
            continue;
        }
        let e = t - m.start;
        let x = *frequency as f64 * e;
        let fall = (-(*decay as f64) * e).exp();
        let s = total.get_or_insert(LayerShake {
            dx: 0.0,
            dy: 0.0,
            rotation: 0.0,
        });
        s.dx += (amplitude[0] as f64 * noise::smooth(*seed, x) * fall) as f32;
        s.dy += (amplitude[1] as f64 * noise::smooth(*seed ^ SHAKE_Y, x) * fall) as f32;
        s.rotation += (*rotation as f64 * noise::smooth(*seed ^ SHAKE_ROT, x) * fall) as f32;
    }
    total
}

/// Start rank of every non-whitespace glyph (`ranks[k]` for glyph `k` in
/// reading order) under `order`. A permutation of `0..n`.
pub(crate) fn glyph_ranks(n: usize, order: GlyphOrder, seed: u32) -> Vec<usize> {
    // Glyph indices sorted by start time; the position is the rank.
    let ranked = |key: &dyn Fn(usize) -> (u64, usize)| {
        let mut sorted: Vec<usize> = (0..n).collect();
        sorted.sort_by_key(|k| key(*k));
        let mut ranks = vec![0usize; n];
        for (rank, k) in sorted.into_iter().enumerate() {
            ranks[k] = rank;
        }
        ranks
    };
    match order {
        GlyphOrder::Forward => (0..n).collect(),
        GlyphOrder::Backward => (0..n).map(|k| n - 1 - k).collect(),
        // |k - (n-1)/2| scaled by 2 stays integral; ties: left first.
        GlyphOrder::Center => ranked(&|k| ((2 * k as i64 - (n as i64 - 1)).unsigned_abs(), k)),
        GlyphOrder::Random => ranked(&|k| (noise::hash(seed, k as u32) as u64, k)),
    }
}

fn lerp_pose(a: GlyphPose, b: GlyphPose, p: f64) -> GlyphPose {
    if p <= 0.0 {
        return a;
    }
    if p >= 1.0 {
        return b;
    }
    GlyphPose {
        dx: lerp(a.dx, b.dx, p),
        dy: lerp(a.dy, b.dy, p),
        scale: lerp(a.scale, b.scale, p),
        rotation: lerp(a.rotation, b.rotation, p),
        opacity: lerp(a.opacity, b.opacity, p),
    }
}

/// Per-glyph poses of a text layer under its `glyph_cascade` at scene-local
/// `t`, for the `text` shown this frame; `None` while every glyph is at rest.
/// Glyph with start rank `r` starts `r * stagger` after the motion start and
/// runs `max(duration - (n - 1) * stagger, 1e-3)` s; before its start it holds
/// `from`, after its end it is at rest.
fn glyph_poses(motions: &[&Motion], t: f64, text: &str) -> Option<Vec<GlyphPose>> {
    let (m, _) = channel_progress(motions, t, |op| matches!(op, MotionOp::GlyphCascade { .. }))?;
    let MotionOp::GlyphCascade {
        stagger,
        from,
        order,
        seed,
    } = &m.op
    else {
        return None;
    };
    let n = text.chars().filter(|c| !c.is_whitespace()).count();
    if n == 0 {
        return None;
    }
    let ranks = glyph_ranks(n, *order, *seed);
    let run = (m.duration - (n as f64 - 1.0) * stagger).max(1e-3);
    let poses: Vec<GlyphPose> = ranks
        .iter()
        .map(|r| {
            let elapsed = t - (m.start + *r as f64 * stagger);
            // Rounding noise at the very end must not leave a glyph a hair
            // off rest for one frame.
            let p = if elapsed + TIME_EPS >= run {
                1.0
            } else {
                shaped_progress(m.easing, m.spring, false, elapsed, run)
            };
            lerp_pose(*from, GlyphPose::REST, p)
        })
        .collect();
    poses.iter().any(|p| *p != GlyphPose::REST).then_some(poses)
}

/// Polyline points of a layer under its `path_morph` motions at scene-local
/// `t`: `None` before the first morph starts; from the start on, the layer's
/// own points (or the previous morph's target when chained) blended to `to`
/// after resampling both to the same arc-length point count, holding `to`
/// after the end.
fn morph_points(
    motions: &[&Motion],
    t: f64,
    own: &[[f32; 2]],
    closed: bool,
) -> Option<Vec<[f32; 2]>> {
    let morphs: Vec<&&Motion> = motions
        .iter()
        .filter(|m| matches!(m.op, MotionOp::PathMorph { .. }))
        .collect();
    let at = morphs.iter().rposition(|m| m.start <= t)?;
    let m = morphs[at];
    let MotionOp::PathMorph { to } = &m.op else {
        return None;
    };
    let from: &[[f32; 2]] = match at.checked_sub(1).map(|i| &morphs[i].op) {
        Some(MotionOp::PathMorph { to: prev }) => prev,
        _ => own,
    };
    let p = progress_at(m, t - m.start);
    if from.is_empty() || to.is_empty() {
        return Some(if p >= 1.0 { to.clone() } else { from.to_vec() });
    }
    let n = from.len().max(to.len());
    let a = resample(from, n, closed);
    let b = resample(to, n, closed);
    if p <= 0.0 {
        return Some(a);
    }
    if p >= 1.0 {
        return Some(b);
    }
    Some(
        a.iter()
            .zip(&b)
            .map(|(u, v)| [lerp(u[0], v[0], p), lerp(u[1], v[1], p)])
            .collect(),
    )
}

/// `n` points spread evenly by arc length along `pts` (an open path, or a
/// closed loop when `closed`, where the closing segment counts and the first
/// point is not repeated). Degenerate paths repeat their first point.
fn resample(pts: &[[f32; 2]], n: usize, closed: bool) -> Vec<[f32; 2]> {
    let Some(first) = pts.first().copied() else {
        return Vec::new();
    };
    if n == 0 {
        return Vec::new();
    }
    let mut path: Vec<[f32; 2]> = pts.to_vec();
    if closed && pts.len() > 1 {
        path.push(first);
    }
    let seg = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]) as f64).hypot((b[1] - a[1]) as f64);
    let mut cum = vec![0.0f64];
    for w in path.windows(2) {
        cum.push(cum[cum.len() - 1] + seg(w[0], w[1]));
    }
    let total = cum[cum.len() - 1];
    if path.len() < 2 || total <= 0.0 {
        return vec![first; n];
    }
    let steps = if closed {
        n
    } else {
        n.saturating_sub(1).max(1)
    };
    let mut out = Vec::with_capacity(n);
    let mut j = 0usize;
    for i in 0..n {
        if !closed && i + 1 == n && n > 1 {
            out.push(path[path.len() - 1]);
            continue;
        }
        let s = total * i as f64 / steps as f64;
        while j + 2 < cum.len() && cum[j + 1] < s {
            j += 1;
        }
        let span = cum[j + 1] - cum[j];
        let u = if span > 0.0 { (s - cum[j]) / span } else { 0.0 };
        let (a, b) = (path[j], path[j + 1]);
        out.push([lerp(a[0], b[0], u), lerp(a[1], b[1], u)]);
    }
    out
}

/// Clip insets for a MaskReveal at progress `p`. `direction` is the direction the
/// moving edge travels.
pub fn mask_inset(direction: Direction, mode: RevealMode, p: f32) -> ClipInset {
    let p = p.clamp(0.0, 1.0);
    let hidden = 1.0 - p;
    let mut c = ClipInset::default();
    match (mode, direction) {
        (RevealMode::Reveal, Direction::Right) => c.right = hidden,
        (RevealMode::Reveal, Direction::Left) => c.left = hidden,
        (RevealMode::Reveal, Direction::Down) => c.bottom = hidden,
        (RevealMode::Reveal, Direction::Up) => c.top = hidden,
        (RevealMode::Conceal, Direction::Right) => c.left = p,
        (RevealMode::Conceal, Direction::Left) => c.right = p,
        (RevealMode::Conceal, Direction::Down) => c.top = p,
        (RevealMode::Conceal, Direction::Up) => c.bottom = p,
    }
    c
}

fn combine_insets(a: ClipInset, b: ClipInset) -> ClipInset {
    ClipInset {
        left: a.left.max(b.left),
        top: a.top.max(b.top),
        right: a.right.max(b.right),
        bottom: a.bottom.max(b.bottom),
    }
}

/// Content offset for a ClipReveal at progress `p`: content travels in `direction`
/// from fully outside the box (p = 0) to its rest position (p = 1), or the reverse
/// for `Conceal`.
pub fn clip_reveal_offset(
    direction: Direction,
    mode: RevealMode,
    p: f32,
    w: f32,
    h: f32,
) -> (f32, f32) {
    let p = p.clamp(0.0, 1.0);
    let k = match mode {
        RevealMode::Reveal => -(1.0 - p), // start displaced opposite to travel
        RevealMode::Conceal => p,         // leave along travel
    };
    match direction {
        Direction::Right => (k * w, 0.0),
        Direction::Left => (-k * w, 0.0),
        Direction::Down => (0.0, k * h),
        Direction::Up => (0.0, -k * h),
    }
}

// ---------------------------------------------------------------------------
// Shared elements (SharedTrack)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
struct TrackState {
    x: f32,
    y: f32,
    scale: f32,
    rotation: f32,
    opacity: f32,
}

impl TrackState {
    fn from_layer(l: &Layer) -> Self {
        TrackState {
            x: l.x,
            y: l.y,
            scale: 1.0,
            rotation: l.rotation_degrees,
            opacity: l.opacity,
        }
    }
    fn overlay(mut self, k: &KeyState) -> Self {
        if let Some(v) = k.x {
            self.x = v;
        }
        if let Some(v) = k.y {
            self.y = v;
        }
        if let Some(v) = k.scale {
            self.scale = v;
        }
        if let Some(v) = k.rotation_degrees {
            self.rotation = v;
        }
        if let Some(v) = k.opacity {
            self.opacity = v;
        }
        self
    }
}

/// Where a track key's position comes from.
#[derive(Clone, Copy)]
enum Source<'p> {
    /// Absolute canvas position (mapped through the key scene's camera).
    Free { scene: usize },
    /// Placement inside a layer of `scene`.
    Bound {
        binding: &'p LayoutBinding,
        scene: usize,
    },
}

/// A track key resolved for evaluation.
struct KeyPoint<'p> {
    time: f64,
    easing: Easing,
    state: TrackState,
    source: Source<'p>,
}

/// A key's state in canvas space.
#[derive(Debug, Clone, Copy)]
struct WorldState {
    x: f32,
    y: f32,
    scale: f32,
    rotation: f32,
    opacity: f32,
    /// Canvas pixels per scene pixel (camera zoom at depth / parent scale).
    unit: f32,
}

impl WorldState {
    fn lerp(a: Self, b: Self, p: f64) -> Self {
        WorldState {
            x: lerp(a.x, b.x, p),
            y: lerp(a.y, b.y, p),
            scale: lerp(a.scale, b.scale, p),
            rotation: lerp(a.rotation, b.rotation, p),
            opacity: lerp(a.opacity, b.opacity, p),
            unit: lerp(a.unit, b.unit, p),
        }
    }
}

/// Canvas-space placement of a shared element at absolute time `t`, or `None`
/// when the element is outside its `[first key, last key]` lifetime.
fn shared_placement<'a>(
    project: &'a MotionProject,
    e: &'a SharedElement,
    t: f64,
    tables: &mut HashMap<usize, WorldTable<'a>>,
) -> Result<Option<SharedPlacement>, TimelineError> {
    let mut keys: Vec<KeyPoint<'a>> = Vec::with_capacity(e.track.len());
    let mut state = TrackState::from_layer(&e.layer);
    let mut source: Option<Source<'a>> = None;
    for k in &e.track {
        let scene_idx = project
            .scenes
            .iter()
            .position(|s| s.id == k.scene)
            .ok_or_else(|| TimelineError::UnknownScene {
                element: e.id.clone(),
                scene: k.scene.clone(),
            })?;
        let start = project.scenes[scene_idx].start_seconds;
        state = state.overlay(&k.state);
        let src = if let Some(binding) = &k.layout {
            Source::Bound {
                binding,
                scene: scene_idx,
            }
        } else if k.state.x.is_some() || k.state.y.is_some() {
            Source::Free { scene: scene_idx }
        } else {
            // Inherit the previous source; the first key defaults to Free.
            source.unwrap_or(Source::Free { scene: scene_idx })
        };
        source = Some(src);
        keys.push(KeyPoint {
            time: start + k.at,
            easing: k.easing,
            state,
            source: src,
        });
    }
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else {
        return Ok(None);
    };
    if t < first.time || t > last.time {
        return Ok(None);
    }
    let mut world = None;
    for pair in keys.windows(2) {
        let (k0, k1) = (&pair[0], &pair[1]);
        if t >= k0.time && t <= k1.time {
            let raw = if k1.time > k0.time {
                (t - k0.time) / (k1.time - k0.time)
            } else {
                1.0
            };
            let a = world_state(project, &e.layer, k0, t, tables);
            let b = world_state(project, &e.layer, k1, t, tables);
            world = Some(WorldState::lerp(a, b, k1.easing.apply(raw)));
            break;
        }
    }
    let ws = match world {
        Some(w) => w,
        None => world_state(project, &e.layer, first, t, tables),
    };
    Ok(Some(SharedPlacement {
        start: first.time,
        x: ws.x,
        y: ws.y,
        rotation: ws.rotation,
        opacity: ws.opacity,
        scale: ws.scale,
        unit: ws.unit,
        motion: SharedMotion::default(),
    }))
}

/// Scene-local time used to evaluate a key's scene at absolute time `t`.
fn scene_local(scene: &Scene, t: f64) -> f64 {
    (t - scene.start_seconds)
        .max(0.0)
        .min(scene.duration_seconds)
}

/// Layout table of `project.scenes[si]` at absolute time `t` (memoized).
fn scene_table<'a, 't>(
    project: &'a MotionProject,
    si: usize,
    t: f64,
    tables: &'t mut HashMap<usize, WorldTable<'a>>,
) -> &'t WorldTable<'a> {
    tables.entry(si).or_insert_with(|| {
        let scene = &project.scenes[si];
        let lt = scene_local(scene, t);
        let index = MotionIndex::new(&scene.motions);
        let camera = camera_state(scene, &project.canvas, lt);
        let assets = AssetMap::new();
        let ctx = EvalCtx {
            t: lt,
            sprite_t: lt,
            abs_t: scene.start_seconds + lt,
            assets: &assets,
            motions: &index,
            envelopes: &project.envelopes,
            scene: Some(scene.id.as_str()),
            camera: camera.as_ref(),
            echo: false,
        };
        let mut world = WorldTable::new();
        let mut siblings = Siblings::new();
        for layer in &scene.layers {
            let _ = resolve_layer(layer, ROOT, &ctx, &mut siblings, &mut world);
        }
        world
    })
}

/// Resolve a key's state into canvas space at absolute time `t`.
fn world_state<'a>(
    project: &'a MotionProject,
    layer: &Layer,
    key: &KeyPoint<'a>,
    t: f64,
    tables: &mut HashMap<usize, WorldTable<'a>>,
) -> WorldState {
    let st = key.state;
    let depth = layer.depth.unwrap_or(1.0).max(0.0);

    let free = |scene_idx: usize| {
        let scene = &project.scenes[scene_idx];
        let camera = camera_state(scene, &project.canvas, scene_local(scene, t));
        match camera {
            Some(c) => {
                let (x, y, unit) = match c.shared_point(st.x, st.y) {
                    Some(p) => p,
                    None => {
                        let (x, y) = c.affine(depth).apply(st.x, st.y);
                        (x, y, c.zoom_at(depth))
                    }
                };
                WorldState {
                    x,
                    y,
                    scale: st.scale * unit,
                    rotation: if c.roll == 0.0 {
                        st.rotation
                    } else {
                        st.rotation + c.roll
                    },
                    opacity: st.opacity,
                    unit,
                }
            }
            None => WorldState {
                x: st.x,
                y: st.y,
                scale: st.scale,
                rotation: st.rotation,
                opacity: st.opacity,
                unit: 1.0,
            },
        }
    };

    match key.source {
        Source::Free { scene } => free(scene),
        Source::Bound { binding, scene } => {
            let parent = scene_table(project, scene, t, tables)
                .get(binding.parent.as_str())
                .copied();
            let Some((wbox, pw, ph)) = parent else {
                return free(scene);
            };
            let s_p = (wbox.a * wbox.d - wbox.b * wbox.c).abs().sqrt();
            let r_p = wbox.b.atan2(wbox.a).to_degrees();
            let s = st.scale * s_p;
            let r = st.rotation + r_p;
            let (px, py) = parent_point(binding, pw, ph);
            let (pwx, pwy) = wbox.apply(px, py);
            let (ax, ay) = align_point(layer, binding, layer.width, layer.height);
            let (nx, ny) = (layer.anchor_x * layer.width, layer.anchor_y * layer.height);
            let vx = s * layer.scale_x * (nx - ax);
            let vy = s * layer.scale_y * (ny - ay);
            let (sn, cs) = r.to_radians().sin_cos();
            WorldState {
                x: pwx + cs * vx - sn * vy,
                y: pwy + sn * vx + cs * vy,
                scale: s,
                rotation: r,
                opacity: st.opacity,
                unit: s_p,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_ranks_follow_the_documented_orders() {
        assert_eq!(glyph_ranks(0, GlyphOrder::Forward, 0), Vec::<usize>::new());
        assert_eq!(glyph_ranks(4, GlyphOrder::Forward, 0), [0, 1, 2, 3]);
        assert_eq!(glyph_ranks(4, GlyphOrder::Backward, 0), [3, 2, 1, 0]);
        // Odd count: the middle glyph first, then alternating left/right.
        assert_eq!(glyph_ranks(5, GlyphOrder::Center, 0), [3, 1, 0, 2, 4]);
        // Even count: |k - 1.5| ties (k=1,2) and (k=0,3) resolve left first.
        assert_eq!(glyph_ranks(4, GlyphOrder::Center, 0), [2, 0, 1, 3]);
        assert_eq!(glyph_ranks(1, GlyphOrder::Center, 0), [0]);
    }

    #[test]
    fn random_glyph_ranks_are_a_seeded_permutation() {
        let a = glyph_ranks(12, GlyphOrder::Random, 5);
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..12).collect::<Vec<_>>());
        assert_eq!(a, glyph_ranks(12, GlyphOrder::Random, 5));
        assert_ne!(a, glyph_ranks(12, GlyphOrder::Random, 6));
        // Rank order is the order of noise::hash(seed, k).
        let by_rank: Vec<u32> = {
            let mut idx: Vec<usize> = (0..12).collect();
            idx.sort_by_key(|k| a[*k]);
            idx.iter().map(|k| noise::hash(5, *k as u32)).collect()
        };
        assert!(by_rank.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn resample_spreads_points_by_arc_length() {
        let pts = [[0.0, 0.0], [10.0, 0.0], [10.0, 30.0]];
        // Total length 40: five points every 10 px along the path.
        let r = resample(&pts, 5, false);
        let want = [
            [0.0, 0.0],
            [10.0, 0.0],
            [10.0, 10.0],
            [10.0, 20.0],
            [10.0, 30.0],
        ];
        for (g, w) in r.iter().zip(want) {
            assert!(
                (g[0] - w[0]).abs() < 1e-4 && (g[1] - w[1]).abs() < 1e-4,
                "{g:?}"
            );
        }
        // The last point is the exact end of the path.
        assert_eq!(r[4], [10.0, 30.0]);
        // Closed loop: the closing segment counts and the start is not repeated.
        let sq = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let r = resample(&sq, 8, true);
        assert_eq!(r.len(), 8);
        assert_eq!(r[0], [0.0, 0.0]);
        assert!((r[1][0] - 5.0).abs() < 1e-4 && r[1][1].abs() < 1e-4);
        assert!(r[7][0].abs() < 1e-4 && (r[7][1] - 5.0).abs() < 1e-4);
    }

    #[test]
    fn resample_handles_degenerate_paths() {
        assert!(resample(&[], 4, false).is_empty());
        assert_eq!(resample(&[[3.0, 4.0]], 3, false), vec![[3.0, 4.0]; 3]);
        assert_eq!(
            resample(&[[1.0, 1.0], [1.0, 1.0]], 3, false),
            vec![[1.0, 1.0]; 3]
        );
    }

    #[test]
    fn shaped_progress_uses_the_spring_but_lands_counts() {
        let spring = SpringSpec {
            stiffness: 300.0,
            damping: 10.0,
            mass: 1.0,
        };
        let eased = |e: f64, d: f64| shaped_progress(Easing::OutCubic, None, false, e, d);
        assert_eq!(eased(0.5, 1.0), Easing::OutCubic.apply(0.5));
        assert_eq!(eased(0.0, 0.0), 1.0, "zero duration is complete");
        let sp = |landing: bool, e: f64| {
            shaped_progress(Easing::OutCubic, Some(spring), landing, e, 1.0)
        };
        assert_eq!(sp(false, 0.4), easing::spring_progress(0.4, 1.0, spring));
        assert_eq!(sp(true, 0.4), Easing::OutQuint.apply(0.4));
        assert_eq!(sp(false, 1.0), 1.0);
        assert_eq!(sp(false, -1.0), 0.0);
    }

    #[test]
    fn post_strength_is_windowed() {
        use crate::scene::PostKind;
        let e = PostEffect {
            start: 1.0,
            duration: 2.0,
            easing: Easing::Linear,
            from: 0.2,
            to: 1.0,
            kind: PostKind::Vignette { amount: 0.5 },
        };
        assert_eq!(post_strength(&e, 0.0), 0.0);
        assert!((post_strength(&e, 1.0) - 0.2).abs() < 1e-6);
        assert!((post_strength(&e, 2.0) - 0.6).abs() < 1e-6);
        assert_eq!(post_strength(&e, 9.0), 0.0);
        let persistent = PostEffect { duration: 0.0, ..e };
        assert_eq!(post_strength(&persistent, 0.5), 0.0);
        assert_eq!(post_strength(&persistent, 9.0), 1.0);
    }
}
