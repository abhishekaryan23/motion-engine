//! (0.20) When does an anchored layer group become readable on screen?
//!
//! Speech QA (`reveal_before_speech`, `reveal_late`) compares that moment
//! with the time the narrator says the group's anchor word. Everything here
//! runs on the timeline's resolved frames (`evaluate_frame`, frame
//! resolution); no pixels are drawn.
//!
//! A LEAF layer (anything but a group) is readable at a frame when
//! - its opacity, compounded with its ancestors' (groups are composited
//!   with their own opacity, as the renderer does) and, for text under a
//!   `glyph_cascade`, with the least visible glyph's resolved opacity (the
//!   cascade has landed), is at least [`READABLE_OPACITY`];
//! - its blur is at most [`READABLE_BLUR_PX`] scaled by `short side / 1080`,
//!   or [`TEXT_BLUR_SHARE`] of its font size (text) / [`PICTURE_BLUR_SHARE`]
//!   of its short side (pictures) when that is more.
//!   The blur is the depth-of-field radius of the layer and of its ancestors
//!   (a blurred group blurs its children; nested Gaussian radii add in
//!   quadrature) plus the frame's `directional_blur` post effect (a box of
//!   `length x strength` px, counted as the Gaussian radius of the same
//!   spread, `length / sqrt(3)`);
//! - its box, minus its own clip inset, mapped to canvas px (through the
//!   perspective homography when the layer has one) intersects the canvas.
//!   A polyline uses its points' bounds (plus half the stroke) and needs a
//!   non-zero trim.
//!
//! Groups are judged by their leaves. A group's readable time is the first
//! frame at which any member leaf is readable; a layer is a member when its
//! own id or an ancestor's id, minus the scene prefix, equals the group or
//! starts with the group plus `"."`.

use motion_core::scene::{LayerKind, MotionProject, PostKind, Scene};
use motion_core::speech::{READABLE_BLUR_PX, READABLE_OPACITY};
use motion_core::timeline::{evaluate_frame, frame_time, ResolvedFrame, ResolvedLayer};
use motion_core::TimelineError;

/// The short side the blur threshold is specified at (px).
const REFERENCE_SHORT_SIDE: f32 = 1080.0;

/// One anchored layer group to watch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupRef {
    /// Scene id (`beat_2`).
    pub scene: String,
    /// The layer-id prefix of that scene, with its dot (`b2.`).
    pub prefix: String,
    /// The anchor's group path after the prefix (`hero`, `card.0`).
    pub group: String,
}

impl GroupRef {
    /// Whether layer `id` belongs to this group (its id minus the prefix is
    /// the group or starts with the group plus `"."`).
    pub fn matches_id(&self, id: &str) -> bool {
        id.strip_prefix(self.prefix.as_str())
            .and_then(|rest| rest.strip_prefix(self.group.as_str()))
            .is_some_and(|tail| tail.is_empty() || tail.starts_with('.'))
    }
}

/// The layer-id prefix the compiler strips from a scene's layer ids: the
/// first layer's id up to its first dot, plus the dot (`b2.`). Falls back to
/// `b<N>.` for a `beat_<N>` scene without layers.
pub fn scene_prefix(scene: &Scene) -> Option<String> {
    let from_layer = scene
        .layers
        .first()
        .and_then(|l| l.id.split_once('.'))
        .map(|(head, _)| format!("{head}."));
    from_layer.or_else(|| {
        scene
            .id
            .strip_prefix("beat_")
            .filter(|n| n.parse::<usize>().is_ok())
            .map(|n| format!("b{n}."))
    })
}

/// Readability limits for one canvas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub width: f32,
    pub height: f32,
    /// Minimum compounded opacity.
    pub opacity: f32,
    /// Maximum blur radius (px on this canvas).
    pub blur_px: f32,
}

/// Blur a text leaf may carry and still read, as a share of its font size.
pub const TEXT_BLUR_SHARE: f32 = 0.12;
/// Blur a picture may carry and still read, as a share of its short side.
pub const PICTURE_BLUR_SHARE: f32 = 0.03;

impl Limits {
    /// The 0.20 limits ([`READABLE_OPACITY`], [`READABLE_BLUR_PX`] at a
    /// 1080 px short side) for a `width x height` canvas.
    pub fn for_canvas(width: u32, height: u32) -> Limits {
        let short = width.min(height) as f32;
        Limits {
            width: width as f32,
            height: height as f32,
            opacity: READABLE_OPACITY,
            blur_px: READABLE_BLUR_PX * short / REFERENCE_SHORT_SIDE,
        }
    }
}

/// A readable leaf at one frame: its scene and the ids from the top-level
/// ancestor down to the leaf itself.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadableLeaf<'a> {
    pub scene: Option<&'a str>,
    pub chain: Vec<&'a str>,
}

/// Every readable leaf of a resolved frame (draw order).
pub fn readable_leaves<'a>(frame: &ResolvedFrame<'a>, limits: &Limits) -> Vec<ReadableLeaf<'a>> {
    let post_blur = frame
        .post
        .iter()
        .map(|p| match p.kind {
            PostKind::DirectionalBlur { length, .. } => {
                let spread = (length * p.strength).max(0.0);
                spread / 3f32.sqrt()
            }
            _ => 0.0,
        })
        .map(|r| r * r)
        .sum::<f32>();
    let mut out = Vec::new();
    let mut chain = Vec::new();
    for layer in &frame.layers {
        walk(layer, 1.0, post_blur, &mut chain, limits, &mut out);
    }
    out
}

/// Depth-first walk carrying the compounded opacity and the summed squared
/// blur radii of the ancestors.
fn walk<'a>(
    layer: &ResolvedLayer<'a>,
    opacity: f32,
    blur_sq: f32,
    chain: &mut Vec<&'a str>,
    limits: &Limits,
    out: &mut Vec<ReadableLeaf<'a>>,
) {
    let opacity = opacity * layer.opacity.clamp(0.0, 1.0);
    let own_blur = layer.blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0);
    let blur_sq = blur_sq + own_blur * own_blur;
    chain.push(layer.id);
    if let LayerKind::Group { .. } = layer.kind {
        for child in &layer.children {
            walk(child, opacity, blur_sq, chain, limits, out);
        }
    } else if leaf_readable(layer, opacity, blur_sq, limits) {
        out.push(ReadableLeaf {
            scene: layer.scene,
            chain: chain.clone(),
        });
    }
    chain.pop();
}

/// Whether a leaf is readable given the compounded opacity (ancestors and
/// own) and the summed squared blur radii (post, ancestors and own).
fn leaf_readable(layer: &ResolvedLayer<'_>, opacity: f32, blur_sq: f32, limits: &Limits) -> bool {
    let glyph = layer
        .glyphs
        .as_ref()
        .and_then(|g| g.iter().map(|p| p.opacity.clamp(0.0, 1.0)).reduce(f32::min))
        .unwrap_or(1.0);
    if opacity * glyph < limits.opacity {
        return false;
    }
    // A big headline or a picture stays legible under more blur than small
    // type: the limit grows with the leaf's size (the cinematic title plane
    // sits a few pixels soft by design).
    let limit = match layer.kind {
        LayerKind::Text(style) => limits.blur_px.max(TEXT_BLUR_SHARE * style.font_size),
        LayerKind::Image { .. } | LayerKind::Svg { .. } => limits
            .blur_px
            .max(PICTURE_BLUR_SHARE * layer.width.min(layer.height)),
        _ => limits.blur_px,
    };
    if blur_sq.sqrt() > limit + 1e-4 {
        return false;
    }
    match local_bounds(layer) {
        Some(rect) => on_canvas(layer, rect, limits),
        None => false,
    }
}

/// The drawn part of a leaf in its local box space: `[x0, y0, x1, y1]`.
/// `None` when nothing is drawn (empty text, zero trim, no points).
fn local_bounds(layer: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    match layer.kind {
        LayerKind::Polyline { points, stroke, .. } => {
            if layer.trim.is_some_and(|t| t <= 1e-3) {
                return None;
            }
            let pts = layer.points.as_deref().unwrap_or(points);
            let half = (stroke.width * 0.5).max(0.0);
            let mut b = [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ];
            for p in pts.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
                b = [
                    b[0].min(p[0] - half),
                    b[1].min(p[1] - half),
                    b[2].max(p[0] + half),
                    b[3].max(p[1] + half),
                ];
            }
            (b[0] < b[2] && b[1] < b[3]).then_some(b)
        }
        LayerKind::Text(style) => {
            let text = layer.text.as_deref().unwrap_or(&style.text);
            if text.trim().is_empty() {
                return None;
            }
            clipped_box(layer)
        }
        _ => clipped_box(layer),
    }
}

/// The layer box minus its clip inset (fractions of the box).
fn clipped_box(layer: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    let (w, h) = (layer.width, layer.height);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let c = layer.clip.unwrap_or_default();
    let b = [
        c.left.max(0.0) * w,
        c.top.max(0.0) * h,
        (1.0 - c.right.max(0.0)) * w,
        (1.0 - c.bottom.max(0.0)) * h,
    ];
    (b[0] < b[2] && b[1] < b[3]).then_some(b)
}

/// Whether the local rectangle, mapped to canvas px, overlaps the canvas.
fn on_canvas(layer: &ResolvedLayer<'_>, rect: [f32; 4], limits: &Limits) -> bool {
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[2], rect[3]),
        (rect[0], rect[3]),
    ];
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let mut any = false;
    for (x, y) in corners {
        let mapped = match &layer.projective {
            Some(m) => {
                let d = m[6] * x + m[7] * y + m[8];
                if !(d.is_finite() && d > 1e-6) {
                    continue;
                }
                (
                    (m[0] * x + m[1] * y + m[2]) / d,
                    (m[3] * x + m[4] * y + m[5]) / d,
                )
            }
            None => layer.transform.apply(x, y),
        };
        if !(mapped.0.is_finite() && mapped.1.is_finite()) {
            continue;
        }
        any = true;
        b = [
            b[0].min(mapped.0),
            b[1].min(mapped.1),
            b[2].max(mapped.0),
            b[3].max(mapped.1),
        ];
    }
    any && b[0] < limits.width && b[2] > 0.0 && b[1] < limits.height && b[3] > 0.0
}

/// Whether a resolved leaf of `scene` (or a shared element) at this frame
/// belongs to `group`.
fn leaf_in_group(leaf: &ReadableLeaf<'_>, group: &GroupRef) -> bool {
    leaf.scene.is_none_or(|s| s == group.scene) && leaf.chain.iter().any(|id| group.matches_id(id))
}

/// The first frame whose time (`frame / fps`, as the timeline computes it)
/// is at least `t`.
fn first_frame_at(t: f64, fps: u32) -> u32 {
    // Far beyond any real project (about 92 hours at 30 fps).
    const LIMIT: u32 = 10_000_000;
    if t.is_nan() || t <= 0.0 {
        return 0;
    }
    let mut k = (t * f64::from(fps)).floor().min(f64::from(LIMIT)) as u32;
    while k < LIMIT && frame_time(fps, k) < t {
        k += 1;
    }
    while k > 0 && frame_time(fps, k - 1) >= t {
        k -= 1;
    }
    k
}

/// Frames `[first, end)` during which `scene` is active (`t >= start` and
/// `t < end`, `t = frame / fps`), the timeline's own test.
fn active_frames(scene: &Scene, fps: u32) -> (u32, u32) {
    let first = first_frame_at(scene.start_seconds, fps);
    let end = first_frame_at(scene.end_seconds(), fps);
    (first, end.max(first))
}

/// The first frame at which each group becomes readable while its scene is
/// active; `None` when it never does (or its scene does not exist). One
/// `evaluate_frame` per frame that some still-unresolved group needs.
pub fn first_readable_frames(
    project: &MotionProject,
    groups: &[GroupRef],
) -> Result<Vec<Option<u32>>, TimelineError> {
    let fps = project.canvas.fps;
    if fps == 0 {
        return Err(TimelineError::ZeroFps);
    }
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let ranges: Vec<Option<(u32, u32)>> = groups
        .iter()
        .map(|g| {
            project
                .scenes
                .iter()
                .find(|s| s.id == g.scene)
                .map(|s| active_frames(s, fps))
        })
        .collect();
    let mut found: Vec<Option<u32>> = vec![None; groups.len()];
    let (Some(lo), Some(hi)) = (
        ranges.iter().flatten().map(|r| r.0).min(),
        ranges.iter().flatten().map(|r| r.1).max(),
    ) else {
        return Ok(found);
    };
    for frame in lo..hi {
        let pending: Vec<usize> = (0..groups.len())
            .filter(|&i| found[i].is_none())
            .filter(|&i| ranges[i].is_some_and(|(a, b)| frame >= a && frame < b))
            .collect();
        if pending.is_empty() {
            continue;
        }
        let resolved = evaluate_frame(project, frame)?;
        let leaves = readable_leaves(&resolved, &limits);
        for i in pending {
            if leaves.iter().any(|leaf| leaf_in_group(leaf, &groups[i])) {
                found[i] = Some(frame);
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(prefix: &str, group: &str) -> GroupRef {
        GroupRef {
            scene: "beat_1".into(),
            prefix: prefix.into(),
            group: group.into(),
        }
    }

    #[test]
    fn group_membership_follows_the_id_path() {
        let g = group("b1.", "card");
        assert!(g.matches_id("b1.card"));
        assert!(g.matches_id("b1.card.0"));
        assert!(!g.matches_id("b1.cards"));
        assert!(!g.matches_id("b1.card_label"));
        assert!(!g.matches_id("b2.card"));
        assert!(!g.matches_id("card"));
    }

    #[test]
    fn blur_limit_scales_with_the_short_side() {
        assert_eq!(Limits::for_canvas(1080, 1920).blur_px, READABLE_BLUR_PX);
        assert_eq!(Limits::for_canvas(1920, 1080).blur_px, READABLE_BLUR_PX);
        assert_eq!(Limits::for_canvas(540, 960).blur_px, READABLE_BLUR_PX * 0.5);
    }
}
