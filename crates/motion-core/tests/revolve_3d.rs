//! (0.18) `revolve`, perspective `billboard` / `depth_sort` and camera
//! `velocity` (Hermite) in the Timeline (docs/MOTION_OPS_2.md, "(0.18)").
//! Canvas 1080x1920 at 30 fps, pivot (540, 960), fov 40 unless stated, so
//! `f = 960 / tan(20 deg)`.
//!
//! Expected values come from small independent statements of the spec
//! (`reference`) and from hand-worked cardinal cases.

use motion_core::easing::spring_progress;
use motion_core::scene::{MotionProject, SpringSpec};
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
use motion_core::validate::validate;
use serde_json::{json, Value};

const FPS: u32 = 30;
const PIVOT: [f64; 2] = [540.0, 960.0];

fn focal(fov_deg: f64) -> f64 {
    960.0 / (fov_deg.to_radians() / 2.0).tan()
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse(v: &Value) -> MotionProject {
    MotionProject::from_json(&v.to_string()).expect("test project parses")
}

fn project_json(scenes: Vec<Value>) -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "revolve_3d" },
        "canvas": { "width": 1080, "height": 1920, "fps": FPS, "background": "#000000" },
        "scenes": scenes,
        "shared": [],
    })
}

/// A 4 s scene `s` without a camera.
fn plain(layers: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    parse(&project_json(vec![json!({
        "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
        "layers": layers, "motions": motions,
    })]))
}

fn persp(billboard: bool, depth_sort: bool, aperture: f64) -> Value {
    json!({ "fov_deg": 40.0, "focus_z": 0.0, "aperture": aperture,
            "billboard": billboard, "depth_sort": depth_sort })
}

/// A 4 s scene `s` with a perspective camera.
fn scene_with(
    layers: Vec<Value>,
    camera_motions: Vec<Value>,
    motions: Vec<Value>,
    perspective: Value,
) -> MotionProject {
    parse(&project_json(vec![json!({
        "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
        "layers": layers, "motions": motions,
        "camera": { "perspective": perspective, "motions": camera_motions },
    })]))
}

/// Perspective scene without DoF, billboard or depth sorting.
fn scene(layers: Vec<Value>, camera_motions: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    scene_with(layers, camera_motions, motions, persp(false, false, 0.0))
}

/// A `w x h` card centred at `(x, y)`.
fn card(id: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "id": id, "type": "rectangle", "x": x, "y": y, "width": w, "height": h,
            "anchor_x": 0.5, "anchor_y": 0.5, "fill": "#FFFFFF" })
}

fn with(mut layer: Value, key: &str, value: Value) -> Value {
    layer[key] = value;
    layer
}

fn merged(mut m: Value, op: Value) -> Value {
    for (k, v) in op.as_object().expect("op is an object") {
        m[k] = v.clone();
    }
    m
}

fn revolve(
    target: &str,
    start: f64,
    dur: f64,
    radius: f64,
    at: [f64; 2],
    from: [f64; 2],
    to: [f64; 2],
) -> Value {
    merged(
        json!({ "target": target, "start": start, "duration": dur, "easing": "linear" }),
        json!({ "op": "revolve", "radius": radius, "at": at, "from": from, "to": to }),
    )
}

fn move_op(target: &str, start: f64, dur: f64, from: [f64; 2], to: [f64; 2]) -> Value {
    merged(
        json!({ "target": target, "start": start, "duration": dur, "easing": "linear" }),
        json!({ "op": "move", "from": from, "to": to }),
    )
}

fn echo_op(target: &str, start: f64, dur: f64, count: u32) -> Value {
    merged(
        json!({ "target": target, "start": start, "duration": dur, "easing": "linear" }),
        json!({ "op": "echo", "count": count, "spacing": 0.1, "decay": 0.5 }),
    )
}

fn cam_motion(start: f64, dur: f64, op: Value) -> Value {
    merged(
        json!({ "start": start, "duration": dur, "easing": "linear" }),
        op,
    )
}

fn dolly_v(from: f64, to: f64, velocity: Value) -> Value {
    let mut m = cam_motion(0.0, 2.0, json!({ "op": "dolly", "from": from, "to": to }));
    m["velocity"] = velocity;
    m
}

fn orbit_const(yaw: f64, pitch: f64) -> Value {
    cam_motion(
        0.0,
        2.0,
        json!({ "op": "orbit", "from": [yaw, pitch], "to": [yaw, pitch] }),
    )
}

fn frame(p: &MotionProject, f: u32) -> ResolvedFrame<'_> {
    evaluate_frame(p, f).expect("frame evaluates")
}

fn find_in<'a, 'b>(layers: &'b [ResolvedLayer<'a>], id: &str) -> Option<&'b ResolvedLayer<'a>> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let Some(c) = find_in(&l.children, id) {
            return Some(c);
        }
    }
    None
}

fn get<'a>(p: &'a MotionProject, f: u32, id: &str) -> ResolvedLayer<'a> {
    find_in(&frame(p, f).layers, id)
        .cloned()
        .unwrap_or_else(|| panic!("layer '{id}' missing at frame {f}"))
}

fn has(p: &MotionProject, f: u32, id: &str) -> bool {
    find_in(&frame(p, f).layers, id).is_some()
}

/// Top-level draw order (back to front) as ids (ghosts repeat their id).
fn order(p: &MotionProject, f: u32) -> Vec<String> {
    frame(p, f)
        .layers
        .iter()
        .map(|l| l.id.to_string())
        .collect()
}

fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: got {a}, expected {b}");
}

fn through_affine(l: &ResolvedLayer<'_>, x: f64, y: f64) -> [f64; 2] {
    let t = l.transform;
    [
        f64::from(t.a) * x + f64::from(t.c) * y + f64::from(t.e),
        f64::from(t.b) * x + f64::from(t.d) * y + f64::from(t.f),
    ]
}

fn assert_pt(got: [f64; 2], want: [f64; 2], tol: f64, what: &str) {
    assert!(
        (got[0] - want[0]).abs() <= tol && (got[1] - want[1]).abs() <= tol,
        "{what}: got {got:?}, expected {want:?}"
    );
}

fn origin(l: &ResolvedLayer<'_>) -> [f64; 2] {
    [f64::from(l.transform.e), f64::from(l.transform.f)]
}

/// Independent statement of the spec's sphere offset: `radius * Rx(pitch) *
/// Ry(yaw) * dir`.
mod reference {
    fn ry(a: f64, v: [f64; 3]) -> [f64; 3] {
        let (s, c) = a.to_radians().sin_cos();
        [v[0] * c - v[2] * s, v[1], v[0] * s + v[2] * c]
    }

    fn rx(b: f64, v: [f64; 3]) -> [f64; 3] {
        let (s, c) = b.to_radians().sin_cos();
        [v[0], v[1] * c + v[2] * s, -v[1] * s + v[2] * c]
    }

    pub fn offset(radius: f64, at: [f64; 2], yaw: f64, pitch: f64) -> [f64; 3] {
        let (lon, lat) = (at[0].to_radians(), at[1].to_radians());
        let dir = [lon.sin() * lat.cos(), -lat.sin(), -lon.cos() * lat.cos()];
        let v = rx(pitch, ry(yaw, dir));
        [radius * v[0], radius * v[1], radius * v[2]]
    }

    /// Yaw about y, then pitch about x (the camera's orbit).
    pub fn rot(v: [f64; 3], pitch: f64, yaw: f64) -> [f64; 3] {
        let (yaw, pitch) = (yaw.to_radians(), pitch.to_radians());
        let x1 = v[0] * yaw.cos() + v[2] * yaw.sin();
        let z1 = -v[0] * yaw.sin() + v[2] * yaw.cos();
        [
            x1,
            v[1] * pitch.cos() - z1 * pitch.sin(),
            v[1] * pitch.sin() + z1 * pitch.cos(),
        ]
    }

    /// World point -> (canvas point, view distance) for a camera at rest
    /// distance `f` with `orbit` `[yaw, pitch]` and `roll` degrees.
    pub fn project(f: f64, orbit: [f64; 2], roll: f64, w: [f64; 3]) -> ([f64; 2], f64) {
        let v = [w[0] - super::PIVOT[0], w[1] - super::PIVOT[1], w[2]];
        let v = rot(v, orbit[1], orbit[0]);
        let dz = v[2] + f;
        let s = f / dz;
        let (dx, dy) = (s * v[0], s * v[1]);
        let (sr, cr) = roll.to_radians().sin_cos();
        (
            [
                super::PIVOT[0] + cr * dx - sr * dy,
                super::PIVOT[1] + sr * dx + cr * dy,
            ],
            dz,
        )
    }
}

// ---------------------------------------------------------------------------
// revolve: formula
// ---------------------------------------------------------------------------

#[test]
fn revolve_front_point_is_radius_nearer_than_the_layer() {
    let f = focal(40.0);
    let r = 400.0;
    // Base z = 0: the front point sits at z = -r, so the layer is enlarged.
    let p = scene(
        vec![card("c", 300.0, 700.0, 200.0, 120.0)],
        vec![],
        vec![revolve(
            "c",
            0.0,
            2.0,
            r,
            [0.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0],
        )],
    );
    let l = get(&p, 0, "c");
    close(
        f64::from(l.transform.a),
        f / (f - r),
        1e-5,
        "front is nearer",
    );
    assert!(f64::from(l.transform.a) > 1.0);
}

#[test]
fn revolve_front_at_base_z_radius_renders_one_to_one_in_place() {
    let r = 400.0;
    let p = scene(
        vec![with(card("c", 300.0, 700.0, 200.0, 120.0), "z", json!(r))],
        vec![],
        vec![revolve(
            "c",
            0.0,
            2.0,
            r,
            [0.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0],
        )],
    );
    let l = get(&p, 0, "c");
    close(f64::from(l.transform.a), 1.0, 1e-6, "scale x");
    close(f64::from(l.transform.d), 1.0, 1e-6, "scale y");
    assert_pt(origin(&l), [200.0, 640.0], 1e-3, "laid-out place");
    assert!(l.projective.is_none() && l.blur.is_none());
}

#[test]
fn a_point_at_lon_90_with_yaw_minus_90_comes_to_the_front() {
    let r = 500.0;
    let p = scene(
        vec![with(card("c", 300.0, 700.0, 200.0, 120.0), "z", json!(r))],
        vec![],
        vec![revolve(
            "c",
            0.0,
            2.0,
            r,
            [90.0, 0.0],
            [-90.0, 0.0],
            [-90.0, 0.0],
        )],
    );
    let l = get(&p, 0, "c");
    close(f64::from(l.transform.a), 1.0, 1e-5, "front plane scale");
    assert_pt(
        origin(&l),
        [200.0, 640.0],
        1e-2,
        "no x/y offset at the front",
    );

    // Unrotated, the same point sits at (+r, 0, 0): offset to the right, same depth.
    let p = scene(
        vec![with(card("c", 300.0, 700.0, 200.0, 120.0), "z", json!(r))],
        vec![],
        vec![revolve(
            "c",
            0.0,
            2.0,
            r,
            [90.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0],
        )],
    );
    let l = get(&p, 0, "c");
    let f = focal(40.0);
    let k = f / (f + r);
    // The layer's centre (300 + r, 700) at z = r projects toward the pivot.
    let c = through_affine(&l, 100.0, 60.0);
    assert_pt(
        c,
        [
            PIVOT[0] + k * (300.0 + r - PIVOT[0]),
            PIVOT[1] + k * (700.0 - PIVOT[1]),
        ],
        1e-2,
        "lon 90 rests at +x",
    );
}

#[test]
fn a_point_at_lat_with_pitch_minus_lat_comes_to_the_front() {
    let r = 500.0;
    let p = scene(
        vec![with(card("c", 300.0, 700.0, 200.0, 120.0), "z", json!(r))],
        vec![],
        vec![revolve(
            "c",
            0.0,
            2.0,
            r,
            [-30.0, 45.0],
            [30.0, -45.0],
            [30.0, -45.0],
        )],
    );
    let l = get(&p, 0, "c");
    close(f64::from(l.transform.a), 1.0, 1e-5, "front plane scale");
    assert_pt(
        origin(&l),
        [200.0, 640.0],
        1e-2,
        "no x/y offset at the front",
    );
}

#[test]
fn midway_the_layer_follows_the_sphere_through_the_projection() {
    let f = focal(40.0);
    let r = 500.0;
    let (cx, cy, base_z) = (300.0, 700.0, 200.0);
    let at = [90.0, 0.0];
    let p = scene(
        vec![with(card("c", cx, cy, 200.0, 120.0), "z", json!(base_z))],
        vec![],
        vec![revolve("c", 0.0, 2.0, r, at, [0.0, 0.0], [-90.0, 0.0])],
    );
    // Frame 30 = t 1.0 = halfway of a linear 2 s motion: yaw -45.
    let l = get(&p, 30, "c");
    let o = reference::offset(r, at, -45.0, 0.0);
    assert!((o[0] - r * 0.5f64.sqrt()).abs() < 1e-9 && (o[2] + r * 0.5f64.sqrt()).abs() < 1e-9);
    let (want, dz) = reference::project(f, [0.0, 0.0], 0.0, [cx + o[0], cy + o[1], base_z + o[2]]);
    let s = f / dz;
    close(f64::from(l.transform.a), s, 1e-5, "scale");
    assert_pt(through_affine(&l, 100.0, 60.0), want, 1e-2, "centre");
}

// ---------------------------------------------------------------------------
// revolve: x/y offset channel (any layer, any scene)
// ---------------------------------------------------------------------------

#[test]
fn cardinal_directions_offset_by_the_radius() {
    let r = 200.0;
    let rect = |id: &str| {
        json!({ "id": id, "type": "rectangle", "x": 100.0, "y": 300.0,
                "width": 50, "height": 50, "fill": "#FFFFFF" })
    };
    let cases = [
        ("right", [90.0, 0.0], [r, 0.0]),
        ("left", [-90.0, 0.0], [-r, 0.0]),
        ("up", [0.0, 90.0], [0.0, -r]),
        ("down", [0.0, -90.0], [0.0, r]),
        ("front", [0.0, 0.0], [0.0, 0.0]),
        ("back", [180.0, 0.0], [0.0, 0.0]),
    ];
    for (name, at, want) in cases {
        let p = plain(
            vec![rect("c")],
            vec![revolve("c", 0.0, 1.0, r, at, [0.0, 0.0], [0.0, 0.0])],
        );
        let l = get(&p, 0, "c");
        assert_pt(origin(&l), [100.0 + want[0], 300.0 + want[1]], 1e-3, name);
    }
}

#[test]
fn offset_matches_the_formula_and_adds_after_move_in_a_plain_scene() {
    let (r, at) = (260.0, [30.0, 10.0]);
    // z is ignored without a perspective camera.
    let layer = json!({ "id": "c", "type": "rectangle", "x": 100.0, "y": 300.0,
                        "width": 50, "height": 50, "fill": "#FFFFFF", "z": 900 });
    let p = plain(
        vec![layer],
        vec![
            revolve("c", 0.0, 2.0, r, at, [0.0, 0.0], [40.0, 20.0]),
            move_op("c", 0.0, 2.0, [0.0, 0.0], [100.0, -50.0]),
        ],
    );
    // t = 1.0: halfway. Revolve [yaw, pitch] = [20, 10]; move = [50, -25].
    let l = get(&p, 30, "c");
    let o = reference::offset(r, at, 20.0, 10.0);
    assert_pt(
        origin(&l),
        [100.0 + 50.0 + o[0], 300.0 - 25.0 + o[1]],
        1e-3,
        "move + revolve",
    );
    close(f64::from(l.transform.a), 1.0, 1e-6, "z must not scale");

    // The same layer without revolve differs by exactly the revolve offset.
    let base = plain(
        vec![
            json!({ "id": "c", "type": "rectangle", "x": 100.0, "y": 300.0,
                     "width": 50, "height": 50, "fill": "#FFFFFF", "z": 900 }),
        ],
        vec![move_op("c", 0.0, 2.0, [0.0, 0.0], [100.0, -50.0])],
    );
    let b = get(&base, 30, "c");
    assert_pt(
        [origin(&l)[0] - origin(&b)[0], origin(&l)[1] - origin(&b)[1]],
        [o[0], o[1]],
        1e-3,
        "delta",
    );
}

#[test]
fn bound_layers_take_the_offset_too() {
    let (r, at) = (150.0, [60.0, -20.0]);
    let parent = card("parent", 540.0, 960.0, 400.0, 400.0);
    let child = json!({ "id": "child", "type": "rectangle", "x": 0, "y": 0,
        "width": 100, "height": 100, "fill": "#FFFFFF",
        "layout": { "parent": "parent", "horizontal": "center", "vertical": "center" } });
    let still = plain(vec![parent.clone(), child.clone()], vec![]);
    let moving = plain(
        vec![parent, child],
        vec![revolve("child", 0.0, 2.0, r, at, [0.0, 0.0], [90.0, 30.0])],
    );
    let (a, b) = (get(&still, 60, "child"), get(&moving, 60, "child"));
    let o = reference::offset(r, at, 90.0, 30.0);
    assert_pt(
        [origin(&b)[0] - origin(&a)[0], origin(&b)[1] - origin(&a)[1]],
        [o[0], o[1]],
        1e-3,
        "bound offset",
    );
}

#[test]
fn z_applies_only_to_top_level_layers_of_perspective_scenes() {
    let r = 300.0;
    let group = json!({ "id": "g", "type": "group", "x": 0, "y": 0, "width": 1080, "height": 1920,
        "children": [
            { "id": "kid", "type": "rectangle", "x": 100.0, "y": 300.0,
              "width": 50, "height": 50, "fill": "#FFFFFF" }
        ] });
    // The child revolves toward the front: x/y offset, but no depth change.
    let p = scene(
        vec![group],
        vec![],
        vec![revolve(
            "kid",
            0.0,
            2.0,
            r,
            [90.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0],
        )],
    );
    let kid = get(&p, 0, "kid");
    assert_pt(origin(&kid), [100.0 + r, 300.0], 1e-3, "offset applies");
    close(f64::from(kid.transform.a), 1.0, 1e-6, "no z for children");

    // A front-facing revolve would pull a top-level layer nearer instead.
    let top = scene(
        vec![
            json!({ "id": "kid", "type": "rectangle", "x": 100.0, "y": 300.0,
                      "width": 50, "height": 50, "fill": "#FFFFFF" }),
        ],
        vec![],
        vec![revolve(
            "kid",
            0.0,
            2.0,
            r,
            [0.0, 0.0],
            [0.0, 0.0],
            [0.0, 0.0],
        )],
    );
    assert!(f64::from(get(&top, 0, "kid").transform.a) > 1.0);
}

// ---------------------------------------------------------------------------
// revolve: hold semantics, spring
// ---------------------------------------------------------------------------

#[test]
fn holds_from_before_start_and_to_after_end() {
    let (r, at) = (200.0, [90.0, 0.0]);
    let p = plain(
        vec![
            json!({ "id": "c", "type": "rectangle", "x": 100.0, "y": 300.0,
                     "width": 50, "height": 50, "fill": "#FFFFFF" }),
        ],
        vec![revolve("c", 1.0, 1.0, r, at, [0.0, 0.0], [90.0, 0.0])],
    );
    // Before the start: `from` (yaw 0 -> offset (+r, 0)), the same as at start.
    let from = reference::offset(r, at, 0.0, 0.0);
    for f in [0, 15, 30] {
        assert_pt(
            origin(&get(&p, f, "c")),
            [100.0 + from[0], 300.0 + from[1]],
            1e-3,
            &format!("frame {f}"),
        );
    }
    // After the end (t >= 2.0): `to` (yaw 90).
    let to = reference::offset(r, at, 90.0, 0.0);
    for f in [60, 90, 119] {
        assert_pt(
            origin(&get(&p, f, "c")),
            [100.0 + to[0], 300.0 + to[1]],
            1e-3,
            &format!("frame {f}"),
        );
    }
}

#[test]
fn the_most_recently_started_revolve_wins_and_holds_its_end() {
    let (r, at) = (200.0, [90.0, 0.0]);
    let p = plain(
        vec![
            json!({ "id": "c", "type": "rectangle", "x": 100.0, "y": 300.0,
                     "width": 50, "height": 50, "fill": "#FFFFFF" }),
        ],
        vec![
            revolve("c", 0.0, 1.0, r, at, [0.0, 0.0], [90.0, 0.0]),
            revolve("c", 2.5, 1.0, r, at, [0.0, 0.0], [180.0, 0.0]),
        ],
    );
    // Between the two (t = 1.8): the first one's end.
    let first_end = reference::offset(r, at, 90.0, 0.0);
    assert_pt(
        origin(&get(&p, 54, "c")),
        [100.0 + first_end[0], 300.0 + first_end[1]],
        1e-3,
        "first end held",
    );
    // After the second (t = 3.67): its end.
    let second_end = reference::offset(r, at, 180.0, 0.0);
    assert_pt(
        origin(&get(&p, 110, "c")),
        [100.0 + second_end[0], 300.0 + second_end[1]],
        1e-3,
        "second end held",
    );
}

#[test]
fn spring_progress_drives_revolve_and_lands_on_to() {
    let (r, at) = (200.0, [90.0, 0.0]);
    let spring = SpringSpec {
        stiffness: 300.0,
        damping: 10.0,
        mass: 1.0,
    };
    let mut m = revolve("c", 0.0, 1.0, r, at, [0.0, 0.0], [-90.0, 0.0]);
    m["spring"] = json!({ "stiffness": 300.0, "damping": 10.0, "mass": 1.0 });
    let p = plain(
        vec![
            json!({ "id": "c", "type": "rectangle", "x": 100.0, "y": 300.0,
                     "width": 50, "height": 50, "fill": "#FFFFFF" }),
        ],
        vec![m],
    );
    // t = 0.4 s (frame 12): the spring's own progress, not the linear one.
    let sp = spring_progress(0.4, 1.0, spring);
    let o = reference::offset(r, at, -90.0 * sp, 0.0);
    assert_pt(
        origin(&get(&p, 12, "c")),
        [100.0 + o[0], 300.0 + o[1]],
        1e-2,
        "spring mid-flight",
    );
    let end = reference::offset(r, at, -90.0, 0.0);
    assert_pt(
        origin(&get(&p, 40, "c")),
        [100.0 + end[0], 300.0 + end[1]],
        1e-3,
        "spring lands on `to`",
    );
}

// ---------------------------------------------------------------------------
// billboard
// ---------------------------------------------------------------------------

fn billboard_layer() -> Value {
    with(card("c", 800.0, 400.0, 300.0, 200.0), "z", json!(300))
}

#[test]
fn billboard_keeps_the_plane_flat_under_an_orbit() {
    let f = focal(40.0);
    let p = scene_with(
        vec![billboard_layer()],
        vec![orbit_const(20.0, 0.0)],
        vec![],
        persp(true, false, 0.0),
    );
    let l = get(&p, 0, "c");
    assert!(l.projective.is_none(), "billboarded layers are not warped");
    let (a, b, c, d) = (
        f64::from(l.transform.a),
        f64::from(l.transform.b),
        f64::from(l.transform.c),
        f64::from(l.transform.d),
    );
    close(a, d, 1e-6, "equal x/y scale (keeps its aspect ratio)");
    close(b, 0.0, 1e-6, "no skew");
    close(c, 0.0, 1e-6, "no skew");
    let (want, dz) = reference::project(f, [20.0, 0.0], 0.0, [800.0, 400.0, 300.0]);
    close(a, f / dz, 1e-6, "scale from the anchor's own distance");
    assert_pt(through_affine(&l, 150.0, 100.0), want, 1e-2, "anchor");
    // The box keeps its shape: its corners are exactly s * (w, h) apart.
    let (o, e) = (
        through_affine(&l, 0.0, 0.0),
        through_affine(&l, 300.0, 200.0),
    );
    assert_pt(
        [e[0] - o[0], e[1] - o[1]],
        [300.0 * f / dz, 200.0 * f / dz],
        1e-2,
        "size",
    );
}

#[test]
fn without_billboard_the_orbit_warps_the_plane() {
    let p = scene(
        vec![billboard_layer()],
        vec![orbit_const(20.0, 0.0)],
        vec![],
    );
    assert!(get(&p, 0, "c").projective.is_some());
}

#[test]
fn billboard_view_rotation_comes_along_with_roll() {
    let f = focal(40.0);
    let roll = cam_motion(0.0, 2.0, json!({ "op": "roll", "from": 10.0, "to": 10.0 }));
    let p = scene_with(
        vec![billboard_layer()],
        vec![orbit_const(20.0, -8.0), roll],
        vec![],
        persp(true, false, 0.0),
    );
    let l = get(&p, 0, "c");
    assert!(l.projective.is_none());
    let (want, dz) = reference::project(f, [20.0, -8.0], 10.0, [800.0, 400.0, 300.0]);
    let s = f / dz;
    let (sr, cr) = 10f64.to_radians().sin_cos();
    close(f64::from(l.transform.a), s * cr, 1e-5, "a");
    close(f64::from(l.transform.b), s * sr, 1e-5, "b");
    assert_pt(through_affine(&l, 150.0, 100.0), want, 1e-2, "anchor");
}

#[test]
fn billboard_tilt_still_turns_the_plane() {
    let layer = with(billboard_layer(), "tilt", json!([15, 0]));
    let p = scene_with(
        vec![layer],
        vec![orbit_const(20.0, 0.0)],
        vec![],
        persp(true, false, 0.0),
    );
    assert!(get(&p, 0, "c").projective.is_some());
}

#[test]
fn billboard_without_orbit_changes_nothing() {
    let layers = vec![
        billboard_layer(),
        with(card("d", 200.0, 300.0, 120.0, 80.0), "z", json!(-200)),
    ];
    let off = scene_with(layers.clone(), vec![], vec![], persp(false, false, 1.0));
    let on = scene_with(layers, vec![], vec![], persp(true, false, 1.0));
    for f in [0, 30, 90] {
        assert_eq!(
            serde_json::to_string(&frame(&off, f)).expect("serializes"),
            serde_json::to_string(&frame(&on, f)).expect("serializes"),
            "frame {f}"
        );
    }
}

#[test]
fn billboard_blur_and_culling_use_the_anchors_view_distance() {
    let f = focal(40.0);
    let aperture = 2.0;
    let p = scene_with(
        vec![billboard_layer()],
        vec![orbit_const(20.0, 0.0)],
        vec![],
        persp(true, false, aperture),
    );
    let l = get(&p, 0, "c");
    let (_, dz) = reference::project(f, [20.0, 0.0], 0.0, [800.0, 400.0, 300.0]);
    let want = aperture * (dz - f).abs() / 100.0;
    close(
        f64::from(l.blur.expect("blurred")),
        want,
        1e-4,
        "blur from the anchor's depth",
    );

    // An anchor behind the camera is not drawn.
    let behind = scene_with(
        vec![with(
            card("c", 800.0, 400.0, 300.0, 200.0),
            "z",
            json!(-3000),
        )],
        vec![orbit_const(20.0, 0.0)],
        vec![],
        persp(true, false, 0.0),
    );
    assert!(!has(&behind, 0, "c"));
}

// ---------------------------------------------------------------------------
// depth_sort
// ---------------------------------------------------------------------------

fn pair() -> Vec<Value> {
    vec![
        card("a", 300.0, 500.0, 200.0, 200.0),
        card("b", 700.0, 500.0, 200.0, 200.0),
    ]
}

/// `b` swings from the front (z = -500) to the back (z = +500) over 2 s.
fn swing() -> Vec<Value> {
    vec![revolve(
        "b",
        0.0,
        2.0,
        500.0,
        [0.0, 0.0],
        [0.0, 0.0],
        [180.0, 0.0],
    )]
}

#[test]
fn depth_sort_swaps_draw_order_as_a_revolve_passes_through() {
    let p = scene_with(pair(), vec![], swing(), persp(false, true, 0.0));
    // t = 0: b at the front -> drawn last. Ties would keep layer order too.
    assert_eq!(order(&p, 0), ["a", "b"]);
    // t = 2.0+: b at the back -> drawn first.
    assert_eq!(order(&p, 70), ["b", "a"]);
    assert_eq!(order(&p, 110), ["b", "a"]);
    // Early in the swing b is still nearer than a.
    assert_eq!(order(&p, 6), ["a", "b"]);
    // Late in the swing b is farther than a.
    assert_eq!(order(&p, 54), ["b", "a"]);
}

#[test]
fn without_depth_sort_layer_order_never_changes() {
    let p = scene_with(pair(), vec![], swing(), persp(false, false, 0.0));
    for f in [0, 6, 54, 70, 110] {
        assert_eq!(order(&p, f), ["a", "b"], "frame {f}");
    }
}

#[test]
fn depth_sort_orders_far_to_near_and_is_stable_for_ties() {
    // Listed near, far, tie-with-near: far first, then the two ties in
    // layer order.
    let layers = vec![
        with(card("near", 300.0, 500.0, 100.0, 100.0), "z", json!(-100)),
        with(card("far", 500.0, 500.0, 100.0, 100.0), "z", json!(400)),
        with(card("tie", 700.0, 500.0, 100.0, 100.0), "z", json!(-100)),
    ];
    let p = scene_with(layers, vec![], vec![], persp(false, true, 0.0));
    assert_eq!(order(&p, 0), ["far", "near", "tie"]);
}

#[test]
fn depth_sort_never_crosses_z_index() {
    let layers = vec![
        with(card("hi", 300.0, 500.0, 100.0, 100.0), "z", json!(900)),
        with(
            with(card("top", 500.0, 500.0, 100.0, 100.0), "z", json!(-300)),
            "z_index",
            json!(5),
        ),
        with(card("lo", 700.0, 500.0, 100.0, 100.0), "z", json!(-300)),
    ];
    let p = scene_with(layers, vec![], vec![], persp(false, true, 0.0));
    // z_index 0: hi (far) then lo; z_index 5 (nearer in depth anyway) last.
    assert_eq!(order(&p, 0), ["hi", "lo", "top"]);
    // z_index wins over depth: the far one at a higher z_index stays on top.
    let layers = vec![
        with(card("near", 300.0, 500.0, 100.0, 100.0), "z", json!(-300)),
        with(
            with(card("far", 500.0, 500.0, 100.0, 100.0), "z", json!(900)),
            "z_index",
            json!(5),
        ),
    ];
    let p = scene_with(layers, vec![], vec![], persp(false, true, 0.0));
    assert_eq!(order(&p, 0), ["near", "far"]);
}

#[test]
fn echo_ghosts_stay_right_behind_their_layer_under_depth_sort() {
    let layers = vec![
        card("a", 300.0, 500.0, 100.0, 100.0),
        with(card("b", 500.0, 500.0, 100.0, 100.0), "z", json!(-100)),
        with(card("c", 700.0, 500.0, 100.0, 100.0), "z", json!(300)),
    ];
    let motions = vec![echo_op("b", 0.0, 3.0, 2)];
    let sorted = scene_with(
        layers.clone(),
        vec![],
        motions.clone(),
        persp(false, true, 0.0),
    );
    // c (far), a, then b's two ghosts, then b (the nearest).
    assert_eq!(order(&sorted, 15), ["c", "a", "b", "b", "b"]);
    let plain_order = scene_with(layers, vec![], motions, persp(false, false, 0.0));
    assert_eq!(order(&plain_order, 15), ["a", "b", "b", "b", "c"]);
}

#[test]
fn blur_follows_the_revolve_depth() {
    let aperture = 2.0;
    let r = 500.0;
    let p = scene_with(
        vec![card("c", 540.0, 960.0, 200.0, 200.0)],
        vec![],
        vec![revolve(
            "c",
            0.0,
            2.0,
            r,
            [0.0, 0.0],
            [0.0, 0.0],
            [90.0, 0.0],
        )],
        persp(false, false, aperture),
    );
    // t = 0: z = -r -> |z - focus| = r.
    close(
        f64::from(get(&p, 0, "c").blur.expect("blurred at the front")),
        aperture * r / 100.0,
        1e-4,
        "front blur",
    );
    // t = 1: yaw 45 -> z = -r cos 45.
    close(
        f64::from(get(&p, 30, "c").blur.expect("blurred midway")),
        aperture * r * 45f64.to_radians().cos() / 100.0,
        1e-3,
        "mid blur",
    );
    // t >= 2: yaw 90 -> z = 0, in focus.
    assert!(get(&p, 70, "c").blur.is_none());
}

// ---------------------------------------------------------------------------
// camera velocity (Hermite)
// ---------------------------------------------------------------------------

/// Layer scale at frame `f` of a dolly with `velocity`, as the dolly distance
/// it implies: `scale = f / (f - dolly)`.
fn dolly_at(velocity: Value, from: f64, to: f64, frame_no: u32) -> f64 {
    let f = focal(40.0);
    let p = scene(
        vec![card("c", 540.0, 960.0, 200.0, 200.0)],
        vec![dolly_v(from, to, velocity)],
        vec![],
    );
    let scale = f64::from(get(&p, frame_no, "c").transform.a);
    f - f / scale
}

#[test]
fn zero_slopes_are_smoothstep() {
    // Frame 15 = t 0.5 of a 2 s motion: u = 0.25 -> 3u^2 - 2u^3 = 0.15625.
    let d = dolly_at(json!([0, 0]), 0.0, 1000.0, 15);
    close(d, 1000.0 * 0.15625, 1e-2, "smoothstep at u = 0.25");
    // Symmetric about the middle (frame 30 = u 0.5).
    close(
        dolly_at(json!([0, 0]), 0.0, 1000.0, 30),
        500.0,
        1e-2,
        "u = 0.5",
    );
}

#[test]
fn unit_slopes_are_linear() {
    let d = dolly_at(json!([1, 1]), 0.0, 1000.0, 15);
    close(d, 250.0, 1e-2, "linear at u = 0.25");
}

#[test]
fn general_slopes_follow_the_hermite_formula() {
    let (v0, v1) = (2.0, 0.5);
    let u: f64 = 0.25;
    let h = (u.powi(3) - 2.0 * u * u + u) * v0
        + (3.0 * u * u - 2.0 * u.powi(3))
        + (u.powi(3) - u * u) * v1;
    let d = dolly_at(json!([v0, v1]), 0.0, 1000.0, 15);
    close(d, 1000.0 * h, 1e-2, "hermite");
    // Ends are exact and the value holds after the motion.
    close(
        dolly_at(json!([v0, v1]), 0.0, 1000.0, 0),
        0.0,
        1e-2,
        "start",
    );
    close(
        dolly_at(json!([v0, v1]), 0.0, 1000.0, 60),
        1000.0,
        1e-2,
        "end",
    );
    close(
        dolly_at(json!([v0, v1]), 0.0, 1000.0, 100),
        1000.0,
        1e-2,
        "hold",
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn frames_are_a_pure_function_of_project_and_frame() {
    let p = scene_with(
        pair(),
        vec![orbit_const(15.0, 5.0)],
        swing(),
        persp(true, true, 1.5),
    );
    for f in [0, 17, 45, 90] {
        let a = serde_json::to_string(&frame(&p, f)).expect("serializes");
        let b = serde_json::to_string(&frame(&p, f)).expect("serializes");
        assert_eq!(a, b, "frame {f}");
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn velocity_project(velocity: Value) -> MotionProject {
    scene(
        vec![card("c", 540.0, 960.0, 200.0, 200.0)],
        vec![dolly_v(0.0, 100.0, velocity)],
        vec![],
    )
}

fn error_text(p: &MotionProject) -> String {
    match validate(p, None) {
        Ok(()) => String::new(),
        Err(e) => e.to_string(),
    }
}

#[test]
fn velocity_slopes_in_zero_to_three_validate() {
    for v in [json!([0, 0]), json!([1, 1]), json!([3, 0.5]), json!([0, 3])] {
        let p = velocity_project(v.clone());
        assert_eq!(error_text(&p), "", "{v}");
    }
}

#[test]
fn out_of_range_velocity_is_an_error() {
    for v in [
        json!([-0.1, 1]),
        json!([1, 3.5]),
        json!([4, 4]),
        json!([1, -1]),
    ] {
        let p = velocity_project(v.clone());
        let e = error_text(&p);
        assert!(e.contains("velocity"), "{v}: {e}");
    }
}

#[test]
fn non_finite_velocity_is_an_error() {
    let mut p = velocity_project(json!([1, 1]));
    let cam = p.scenes[0].camera.as_mut().expect("camera");
    cam.motions[0].velocity = Some([f32::NAN, 1.0]);
    assert!(error_text(&p).contains("velocity"));
    let cam = p.scenes[0].camera.as_mut().expect("camera");
    cam.motions[0].velocity = Some([1.0, f32::INFINITY]);
    assert!(error_text(&p).contains("velocity"));
}

#[test]
fn revolve_radius_must_be_positive() {
    let ok = plain(
        vec![card("c", 100.0, 100.0, 50.0, 50.0)],
        vec![revolve(
            "c",
            0.0,
            1.0,
            100.0,
            [0.0, 0.0],
            [0.0, 0.0],
            [90.0, 0.0],
        )],
    );
    assert_eq!(error_text(&ok), "");
    let bad = plain(
        vec![card("c", 100.0, 100.0, 50.0, 50.0)],
        vec![revolve(
            "c",
            0.0,
            1.0,
            0.0,
            [0.0, 0.0],
            [0.0, 0.0],
            [90.0, 0.0],
        )],
    );
    assert!(error_text(&bad).contains("revolve radius"));
}
