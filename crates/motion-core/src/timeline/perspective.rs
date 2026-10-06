//! (0.16) True-3D perspective camera: projection of a top-level layer's plane.
//!
//! Only used when the scene camera has `perspective`. Everything is a pure
//! function of the resolved camera state and the layer; the math runs in f64
//! and is narrowed to f32 only for the resolved output.
//!
//! Coordinates: canvas px, x right, y down, z into the screen (0 = the focus
//! plane at rest, positive = farther). The pinhole starts at `z = -f` looking
//! along +z, `f = (canvas_height / 2) / tan(fov / 2)`, so the `z = 0` plane maps
//! 1:1 at rest. A world point `(x, y, z)` is processed as
//!
//! 1. layer `tilt` about the layer's anchor (top-level plane only),
//! 2. translate by `-(pivot + track pan)`,
//! 3. camera orbit (yaw about y, then pitch about x) about the pivot,
//! 4. projection `p' = pivot + zoom * f / (z' - cam_z) * (x', y')`,
//! 5. camera roll/shake (screen space, after projection).
//!
//! (0.18) With `Perspective.billboard` a layer without tilt skips step 1 and
//! the orbit's effect on its plane: only its box centre travels through steps 2-5
//! and the plane itself stays a flat, uniformly scaled sprite.

use super::{Affine, CameraState, Inherit};
use crate::scene::{Canvas, Layer, LayerKind, Perspective};

/// Points at or nearer than this many px in front of the camera are behind it.
const NEAR: f64 = 1.0;
/// Depth-of-field blur ceiling (px) and the smallest blur that is emitted.
const MAX_BLUR: f64 = 24.0;
const MIN_BLUR: f64 = 0.5;
/// Projected quads with less area (px^2) than this are degenerate (edge-on).
const MIN_AREA: f64 = 1e-3;

/// Resolved perspective camera at one instant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PerspState {
    /// Focal length in px.
    f: f64,
    /// Camera z (`-f + dolly`).
    cam_z: f64,
    /// `[yaw, pitch]` degrees.
    orbit: [f64; 2],
    focus_z: f64,
    aperture: f64,
    /// (0.18) `Perspective.billboard`.
    billboard: bool,
    /// (0.18) `Perspective.depth_sort`.
    pub(super) depth_sort: bool,
}

impl PerspState {
    /// `None` when the field of view is not usable (validation rejects it).
    pub(super) fn new(
        p: &Perspective,
        canvas: &Canvas,
        dolly: f32,
        orbit: [f32; 2],
        focus: Option<f32>,
    ) -> Option<Self> {
        let fov = f64::from(p.fov_deg);
        if !(fov.is_finite() && fov > 0.0 && fov < 180.0) {
            return None;
        }
        let f = f64::from(canvas.height) * 0.5 / (fov.to_radians() * 0.5).tan();
        if !(f.is_finite() && f > 0.0) {
            return None;
        }
        Some(PerspState {
            f,
            cam_z: -f + f64::from(dolly),
            orbit: [f64::from(orbit[0]), f64::from(orbit[1])],
            focus_z: f64::from(focus.unwrap_or(p.focus_z)),
            aperture: f64::from(p.aperture),
            billboard: p.billboard,
            depth_sort: p.depth_sort,
        })
    }

    /// Depth-of-field blur radius for a plane whose view distance (world z
    /// minus camera z) is `dz`.
    fn blur(&self, dz: f64) -> Option<f32> {
        let z_view = dz - self.f;
        let b = (self.aperture * (z_view - self.focus_z).abs() / 100.0).min(MAX_BLUR);
        (b >= MIN_BLUR).then_some(b as f32)
    }
}

/// The plane a top-level layer lives in, shared with its descendants: they
/// are not projected individually but move rigidly with it.
#[derive(Debug, Clone, Copy)]
pub(super) struct PlanePose {
    /// The flat projection (no tilt, no orbit): roll/shake after a uniform
    /// scale about the pivot. Drawn transforms of the layer tree fold this in.
    flat: Affine,
    /// Tilt anchor in rest canvas px.
    anchor: [f64; 2],
    z: f64,
    /// Tilt `[x, y]` degrees.
    tilt: [f64; 2],
    /// Tilt or orbit make the projection non-affine.
    warped: bool,
    /// (0.18) View distance of the box centre of a billboarded plane (blur, depth
    /// order, behind-camera test); `None` for every other plane.
    view_dz: Option<f64>,
    /// (0.19) A billboarded plane that is also tilted: it turns about its own
    /// centre with a local perspective, independent of the orbit.
    sprite: Option<SpriteFrame>,
}

/// Where a billboarded plane's centre lands, for the tilt turn.
#[derive(Debug, Clone, Copy)]
struct SpriteFrame {
    /// Box centre in rest canvas px.
    centre: [f64; 2],
    /// Projected centre (before roll/shake).
    point: [f64; 2],
    /// Camera zoom x focal length: a point at view distance `d` scales by
    /// `focal / d`.
    focal: f64,
    /// View distance of the centre.
    dz: f64,
}

/// The animated parts of a top-level layer's plane (from its motions).
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PlaneAnim {
    /// (0.18, `revolve`) Added to the plane depth.
    pub z_offset: f64,
    /// (0.19, `tilt`) Replaces the layer's static tilt while set.
    pub tilt: Option<[f32; 2]>,
}

/// Where a layer ends up under the perspective camera.
pub(super) struct Placed {
    /// Box space -> canvas. With `projective` it is the flat fallback.
    pub transform: Affine,
    pub projective: Option<[f32; 9]>,
    pub blur: Option<f32>,
    /// (0.18) View distance of a top-level plane (what the blur is computed
    /// from); `None` for non-top-level layers.
    pub dz: Option<f64>,
    /// Plane handed down to the children.
    pub pose: PlanePose,
}

/// Place `layer` (box `w x h`, rest transform `scene_space`) under the
/// perspective camera. `None` = not drawn (behind the camera or degenerate).
///
/// A top-level layer defines the plane (`z`, `tilt`); a non-top-level layer
/// inherits its top-level ancestor's plane. Without tilt/orbit everything
/// folds into the affine `transform`. Otherwise every LEAF layer gets a
/// homography fitted to its own four box corners (groups are not warped; the
/// renderer draws their leaves), and the top-level layer carries the blur.
///
/// `size` is the box `(w, h)`; `z_offset` (0.18, `revolve`) adds to a
/// top-level layer's plane depth and is ignored for descendants.
pub(super) fn place(
    cam: &CameraState,
    persp: &PerspState,
    layer: &Layer,
    inherit: &Inherit,
    scene_space: Affine,
    size: (f32, f32),
    anim: PlaneAnim,
) -> Option<Placed> {
    let (w, h) = size;
    let top = inherit.top;
    let pose = if top {
        PlanePose::of_layer(cam, persp, layer, scene_space, size, anim)?
    } else {
        inherit.plane?
    };
    let is_group = matches!(layer.kind, LayerKind::Group { .. });
    let mut blur = None;
    let mut projective = None;
    let mut view_dz = None;
    if !pose.warped {
        if top {
            let dz = pose.view_dz.unwrap_or(pose.z - persp.cam_z);
            if dz <= NEAR {
                return None;
            }
            blur = persp.blur(dz);
            view_dz = Some(dz);
        }
    } else if top || !is_group {
        let quad = project_box(cam, persp, &pose, scene_space, w, h)?;
        if top {
            blur = persp.blur(quad.mean_dz);
            view_dz = Some(quad.mean_dz);
        }
        if !is_group {
            projective = Some(quad.homography()?);
        }
    }
    Some(Placed {
        transform: pose.flat.then_apply(scene_space),
        projective,
        blur,
        dz: view_dz,
        pose,
    })
}

impl PlanePose {
    /// `None` only for a billboarded plane whose centre is behind the camera.
    fn of_layer(
        cam: &CameraState,
        persp: &PerspState,
        layer: &Layer,
        scene_space: Affine,
        size: (f32, f32),
        anim: PlaneAnim,
    ) -> Option<PlanePose> {
        let PlaneAnim {
            z_offset,
            tilt: tilt_override,
        } = anim;
        let (w, h) = size;
        let z = layer
            .z
            .map(f64::from)
            .filter(|z| z.is_finite())
            .unwrap_or(0.0)
            + z_offset;
        let tilt = tilt_override
            .or(layer.tilt)
            .map(|t| [f64::from(t[0]), f64::from(t[1])])
            .filter(|t| t[0].is_finite() && t[1].is_finite())
            .unwrap_or([0.0, 0.0]);
        let anchor = apply(
            &scene_space,
            f64::from(layer.anchor_x) * f64::from(w),
            f64::from(layer.anchor_y) * f64::from(h),
        );
        if persp.billboard && persp.orbit != [0.0, 0.0] {
            // A sprite: the orbit moves the box centre through 3D but the
            // plane stays flat, scaled by the centre's own distance (the
            // centre, not the anchor: a text box anchored at its corner must
            // not swing out of focus). Without orbit this is the plain flat
            // projection below. (0.19) A tilt turns the sprite about that
            // centre with a local perspective (see `SpriteFrame`); at tilt 0
            // the two are the same picture, so a settling turn has no pop.
            let centre = apply(&scene_space, 0.5 * f64::from(w), 0.5 * f64::from(h));
            let (point, dz) = cam.project_unviewed(persp, [centre[0], centre[1], z])?;
            let s = f64::from(cam.zoom) * persp.f / dz;
            // q -> point + s * (q - centre), composed in f64.
            let sprite = Affine {
                a: s as f32,
                b: 0.0,
                c: 0.0,
                d: s as f32,
                e: (point[0] - s * centre[0]) as f32,
                f: (point[1] - s * centre[1]) as f32,
            };
            let flat = match cam.view() {
                Some(view) => view.then_apply(sprite),
                None => sprite,
            };
            let turned = tilt != [0.0, 0.0];
            return Some(PlanePose {
                flat,
                anchor,
                z,
                tilt,
                warped: turned,
                view_dz: Some(dz),
                sprite: turned.then_some(SpriteFrame {
                    centre,
                    point,
                    focal: f64::from(cam.zoom) * persp.f,
                    dz,
                }),
            });
        }
        let k = f64::from(cam.zoom) * persp.f / (z - persp.cam_z).max(NEAR);
        let (px, py) = (cam.pivot[0], cam.pivot[1]);
        let projection = Affine::translate(px, py)
            .then_apply(Affine::scale(k as f32, k as f32))
            .then_apply(Affine::translate(-px - cam.pan[0], -py - cam.pan[1]));
        let flat = match cam.view() {
            Some(view) => view.then_apply(projection),
            None => projection,
        };
        Some(PlanePose {
            flat,
            anchor,
            z,
            tilt,
            warped: tilt != [0.0, 0.0] || persp.orbit != [0.0, 0.0],
            view_dz: None,
            sprite: None,
        })
    }

    /// A rest-canvas point of the plane as a world point (tilted about the
    /// anchor, at the plane's depth).
    fn world_point(&self, rest: [f64; 2]) -> [f64; 3] {
        let mut v = [rest[0] - self.anchor[0], rest[1] - self.anchor[1], 0.0];
        if self.tilt != [0.0, 0.0] {
            v = rotate(v, self.tilt[0], self.tilt[1]);
        }
        [self.anchor[0] + v[0], self.anchor[1] + v[1], self.z + v[2]]
    }
}

impl SpriteFrame {
    /// A rest-canvas point of the plane, turned by `tilt` about the box centre
    /// and projected with a local perspective at the centre's view distance.
    /// Returns the canvas point (roll/shake applied) and its view distance;
    /// `None` when the turned point falls behind the camera.
    fn project(
        &self,
        cam: &CameraState,
        tilt: [f64; 2],
        rest: [f64; 2],
    ) -> Option<([f64; 2], f64)> {
        let v = rotate(
            [rest[0] - self.centre[0], rest[1] - self.centre[1], 0.0],
            tilt[0],
            tilt[1],
        );
        let dz = self.dz + v[2];
        if dz <= NEAR {
            return None;
        }
        let k = self.focal / dz;
        let point = [self.point[0] + k * v[0], self.point[1] + k * v[1]];
        let out = match cam.view() {
            Some(view) => apply(&view, point[0], point[1]),
            None => point,
        };
        Some((out, dz))
    }
}

impl CameraState {
    /// Project a world point: subtract the track pan, orbit about the pivot,
    /// project, then apply roll/shake. Returns the canvas point and its view
    /// distance `z' - cam_z`; `None` when it is behind the camera.
    fn project(&self, p: &PerspState, world: [f64; 3]) -> Option<([f64; 2], f64)> {
        let (point, dz) = self.project_unviewed(p, world)?;
        let out = match self.view() {
            Some(view) => apply(&view, point[0], point[1]),
            None => point,
        };
        Some((out, dz))
    }

    /// `project` before the roll/shake `view` is applied.
    fn project_unviewed(&self, p: &PerspState, world: [f64; 3]) -> Option<([f64; 2], f64)> {
        let pivot = [f64::from(self.pivot[0]), f64::from(self.pivot[1])];
        let mut v = [
            world[0] - pivot[0] - f64::from(self.pan[0]),
            world[1] - pivot[1] - f64::from(self.pan[1]),
            world[2],
        ];
        if p.orbit != [0.0, 0.0] {
            v = rotate(v, p.orbit[1], p.orbit[0]);
        }
        let dz = v[2] - p.cam_z;
        if dz <= NEAR {
            return None;
        }
        let s = f64::from(self.zoom) * p.f / dz;
        Some(([pivot[0] + s * v[0], pivot[1] + s * v[1]], dz))
    }

    /// Canvas position and local scale of the `z = 0` plane at `(x, y)` for
    /// shared elements (they are not projected as planes). `None` without a
    /// perspective camera or behind it.
    pub(super) fn shared_point(&self, x: f32, y: f32) -> Option<(f32, f32, f32)> {
        let p = self.persp.as_ref()?;
        let (out, dz) = self.project(p, [f64::from(x), f64::from(y), 0.0])?;
        let scale = f64::from(self.zoom) * p.f / dz;
        Some((out[0] as f32, out[1] as f32, scale as f32))
    }
}

/// A layer's fit rectangle and where its corners land.
struct Quad {
    /// Fit rectangle size in box space (the box, at least 1 px per side).
    size: [f64; 2],
    /// Projected corners of `(0,0) (w,0) (w,h) (0,h)`.
    corners: [[f64; 2]; 4],
    /// Mean view distance of the corners (the box centre's).
    mean_dz: f64,
}

/// Project the corners of the box `w x h` (rest transform `scene_space`)
/// through the plane's tilt, the orbit and the projection. `None` when any
/// corner is behind the camera.
fn project_box(
    cam: &CameraState,
    persp: &PerspState,
    pose: &PlanePose,
    scene_space: Affine,
    w: f32,
    h: f32,
) -> Option<Quad> {
    // A degenerate box (an empty group) still defines the plane; fit a
    // non-degenerate rectangle of it.
    let size = [f64::from(w).max(1.0), f64::from(h).max(1.0)];
    let local = [
        [0.0, 0.0],
        [size[0], 0.0],
        [size[0], size[1]],
        [0.0, size[1]],
    ];
    let mut corners = [[0.0; 2]; 4];
    let mut sum = 0.0;
    for (out, c) in corners.iter_mut().zip(local) {
        let rest = apply(&scene_space, c[0], c[1]);
        let (p, dz) = match &pose.sprite {
            Some(sf) => sf.project(cam, pose.tilt, rest)?,
            None => cam.project(persp, pose.world_point(rest))?,
        };
        *out = p;
        sum += dz;
    }
    Some(Quad {
        size,
        corners,
        mean_dz: sum * 0.25,
    })
}

impl Quad {
    /// Row-major homography (`h33 = 1`) from the fit rectangle's box space to
    /// the projected corners (4-point DLT). `None` when degenerate.
    fn homography(&self) -> Option<[f32; 9]> {
        let c = &self.corners;
        let area2: f64 = (0..4)
            .map(|i| {
                let (a, b) = (c[i], c[(i + 1) % 4]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum();
        if area2.abs() * 0.5 < MIN_AREA {
            return None;
        }
        // Solve in the unit square (well conditioned), then undo the scale.
        let unit = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let mut m = [[0.0; 9]; 8];
        for (i, (s, d)) in unit.iter().zip(c).enumerate() {
            m[2 * i] = [
                s[0],
                s[1],
                1.0,
                0.0,
                0.0,
                0.0,
                -d[0] * s[0],
                -d[0] * s[1],
                d[0],
            ];
            m[2 * i + 1] = [
                0.0,
                0.0,
                0.0,
                s[0],
                s[1],
                1.0,
                -d[1] * s[0],
                -d[1] * s[1],
                d[1],
            ];
        }
        let h = solve8(m)?;
        let (fw, fh) = (self.size[0], self.size[1]);
        let out = [
            h[0] / fw,
            h[1] / fh,
            h[2],
            h[3] / fw,
            h[4] / fh,
            h[5],
            h[6] / fw,
            h[7] / fh,
            1.0,
        ];
        out.iter()
            .all(|v| v.is_finite())
            .then(|| out.map(|v| v as f32))
    }
}

/// Gauss-Jordan elimination with partial pivoting on an 8x8 augmented system.
fn solve8(mut m: [[f64; 9]; 8]) -> Option<[f64; 8]> {
    for col in 0..8 {
        let mut pivot = col;
        for r in col + 1..8 {
            if m[r][col].abs() > m[pivot][col].abs() {
                pivot = r;
            }
        }
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        let d = m[col][col];
        for v in &mut m[col][col..] {
            *v /= d;
        }
        let row = m[col];
        for (r, target) in m.iter_mut().enumerate() {
            if r == col {
                continue;
            }
            let factor = target[col];
            if factor != 0.0 {
                for (v, p) in target[col..].iter_mut().zip(&row[col..]) {
                    *v -= factor * p;
                }
            }
        }
    }
    Some(std::array::from_fn(|i| m[i][8]))
}

/// Rotate by `yaw` degrees about y, then `pitch` degrees about x. Positive
/// yaw brings the right (+x) side nearer, positive pitch brings the top
/// (-y) nearer.
fn rotate(v: [f64; 3], pitch_deg: f64, yaw_deg: f64) -> [f64; 3] {
    let (sy, cy) = yaw_deg.to_radians().sin_cos();
    let (sp, cp) = pitch_deg.to_radians().sin_cos();
    let x1 = v[0] * cy + v[2] * sy;
    let z1 = -v[0] * sy + v[2] * cy;
    let y2 = v[1] * cp - z1 * sp;
    let z2 = v[1] * sp + z1 * cp;
    [x1, y2, z2]
}

fn apply(a: &Affine, x: f64, y: f64) -> [f64; 2] {
    [
        f64::from(a.a) * x + f64::from(a.c) * y + f64::from(a.e),
        f64::from(a.b) * x + f64::from(a.d) * y + f64::from(a.f),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(size: [f64; 2], corners: [[f64; 2]; 4]) -> Quad {
        Quad {
            size,
            corners,
            mean_dz: 0.0,
        }
    }

    fn apply_h(h: &[f32; 9], x: f64, y: f64) -> [f64; 2] {
        let h: Vec<f64> = h.iter().map(|v| f64::from(*v)).collect();
        let w = h[6] * x + h[7] * y + h[8];
        [
            (h[0] * x + h[1] * y + h[2]) / w,
            (h[3] * x + h[4] * y + h[5]) / w,
        ]
    }

    #[test]
    fn homography_of_a_translated_rectangle_is_affine() {
        let q = quad(
            [200.0, 100.0],
            [[10.0, 20.0], [210.0, 20.0], [210.0, 120.0], [10.0, 120.0]],
        );
        let h = q.homography().expect("non-degenerate");
        let want = [1.0, 0.0, 10.0, 0.0, 1.0, 20.0, 0.0, 0.0, 1.0];
        for (g, w) in h.iter().zip(want) {
            assert!((f64::from(*g) - w).abs() < 1e-6, "{h:?}");
        }
    }

    #[test]
    fn homography_maps_the_fit_rectangle_corners_and_interior_points() {
        // A trapezoid: the right edge is taller than the left.
        let corners = [
            [100.0, 200.0],
            [500.0, 150.0],
            [500.0, 450.0],
            [100.0, 400.0],
        ];
        let q = quad([300.0, 200.0], corners);
        let h = q.homography().expect("non-degenerate");
        let local = [[0.0, 0.0], [300.0, 0.0], [300.0, 200.0], [0.0, 200.0]];
        for (l, want) in local.iter().zip(corners) {
            let got = apply_h(&h, l[0], l[1]);
            assert!(
                (got[0] - want[0]).abs() < 1e-3 && (got[1] - want[1]).abs() < 1e-3,
                "{l:?}: {got:?} vs {want:?}"
            );
        }
        // The image of the rectangle's centre is the diagonals' intersection.
        // Diagonals (100,200)-(500,450) and (500,150)-(100,400) meet at (260, 300).
        let c = apply_h(&h, 150.0, 100.0);
        assert!(
            (c[0] - 260.0).abs() < 1e-3 && (c[1] - 300.0).abs() < 1e-3,
            "{c:?}"
        );
    }

    #[test]
    fn degenerate_quads_have_no_homography() {
        let line = quad(
            [100.0, 100.0],
            [[0.0, 0.0], [100.0, 0.0], [100.0, 0.0], [0.0, 0.0]],
        );
        assert!(line.homography().is_none());
        let point = quad([1.0, 1.0], [[5.0, 5.0]; 4]);
        assert!(point.homography().is_none());
    }

    #[test]
    fn rotation_is_yaw_then_pitch_with_documented_signs() {
        // Positive yaw brings +x nearer (-z), positive pitch brings -y nearer.
        let v = rotate([100.0, 0.0, 0.0], 0.0, 90.0);
        assert!(v[0].abs() < 1e-9 && (v[2] + 100.0).abs() < 1e-9, "{v:?}");
        let v = rotate([0.0, -100.0, 0.0], 90.0, 0.0);
        assert!(v[1].abs() < 1e-9 && (v[2] + 100.0).abs() < 1e-9, "{v:?}");
        assert_eq!(rotate([3.0, -4.0, 12.0], 0.0, 0.0), [3.0, -4.0, 12.0]);
        let r = rotate([3.0, -4.0, 12.0], 17.0, -33.0);
        let len = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sqrt();
        assert!((len - 13.0).abs() < 1e-9);
    }
}
