//! (0.16) True-3D perspective camera in the Timeline (docs/MOTION_OPS_2.md,
//! "Perspective camera"). Canvas 1080x1920 at 30 fps, pivot (540, 960),
//! fov 40 unless stated, so `f = 960 / tan(20 deg)`.
//!
//! The expected values come from `reference`, an independent f64 statement of
//! the spec's pipeline (tilt about the anchor, pan, orbit, projection, roll).

use motion_core::scene::MotionProject;
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
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

fn project_json(scenes: Vec<Value>, shared: Value) -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "perspective" },
        "canvas": { "width": 1080, "height": 1920, "fps": FPS, "background": "#000000" },
        "scenes": scenes,
        "shared": shared,
    })
}

fn persp(fov: f64, focus_z: f64, aperture: f64) -> Value {
    json!({ "fov_deg": fov, "focus_z": focus_z, "aperture": aperture })
}

/// One 4 s scene `s` with a perspective camera (fov 40, no DoF) and `motions`.
fn scene(layers: Vec<Value>, camera_motions: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    scene_with(layers, camera_motions, motions, persp(40.0, 0.0, 0.0))
}

fn scene_with(
    layers: Vec<Value>,
    camera_motions: Vec<Value>,
    motions: Vec<Value>,
    perspective: Value,
) -> MotionProject {
    parse(&project_json(
        vec![json!({
            "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
            "layers": layers, "motions": motions,
            "camera": { "perspective": perspective, "motions": camera_motions },
        })],
        json!([]),
    ))
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

fn cam_motion(start: f64, dur: f64, op: Value) -> Value {
    let mut m = json!({ "start": start, "duration": dur, "easing": "linear" });
    for (k, v) in op.as_object().expect("op is an object") {
        m[k] = v.clone();
    }
    m
}

fn dolly(from: f64, to: f64) -> Value {
    cam_motion(0.0, 2.0, json!({ "op": "dolly", "from": from, "to": to }))
}

fn orbit(from: [f64; 2], to: [f64; 2]) -> Value {
    cam_motion(0.0, 2.0, json!({ "op": "orbit", "from": from, "to": to }))
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

fn close(a: f64, b: f64, tol: f64, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: got {a}, expected {b}");
}

/// Canvas point of a box-space point through the layer's affine `transform`.
fn through_affine(l: &ResolvedLayer<'_>, x: f64, y: f64) -> [f64; 2] {
    let t = l.transform;
    [
        f64::from(t.a) * x + f64::from(t.c) * y + f64::from(t.e),
        f64::from(t.b) * x + f64::from(t.d) * y + f64::from(t.f),
    ]
}

/// Canvas point of a box-space point through a row-major homography.
fn through_h(h: &[f32; 9], x: f64, y: f64) -> [f64; 2] {
    let h: Vec<f64> = h.iter().map(|v| f64::from(*v)).collect();
    let w = h[6] * x + h[7] * y + h[8];
    [
        (h[0] * x + h[1] * y + h[2]) / w,
        (h[3] * x + h[4] * y + h[5]) / w,
    ]
}

fn assert_pt(got: [f64; 2], want: [f64; 2], tol: f64, what: &str) {
    assert!(
        (got[0] - want[0]).abs() <= tol && (got[1] - want[1]).abs() <= tol,
        "{what}: got {got:?}, expected {want:?}"
    );
}

/// Independent statement of the projection pipeline.
mod reference {
    #[derive(Clone, Copy)]
    pub struct Cam {
        pub f: f64,
        pub dolly: f64,
        /// `[yaw, pitch]` degrees.
        pub orbit: [f64; 2],
        pub pan: [f64; 2],
        pub zoom: f64,
        pub roll: f64,
        /// Screen-space camera shake (px), applied after the roll.
        pub shake: [f64; 2],
    }

    impl Cam {
        pub fn rest(f: f64) -> Cam {
            Cam {
                f,
                dolly: 0.0,
                orbit: [0.0, 0.0],
                pan: [0.0, 0.0],
                zoom: 1.0,
                roll: 0.0,
                shake: [0.0, 0.0],
            }
        }
    }

    /// Yaw about y, then pitch about x.
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

    /// World point -> canvas, `None` when behind the camera.
    pub fn project(c: &Cam, w: [f64; 3]) -> Option<[f64; 2]> {
        let v = [
            w[0] - super::PIVOT[0] - c.pan[0],
            w[1] - super::PIVOT[1] - c.pan[1],
            w[2],
        ];
        let v = rot(v, c.orbit[1], c.orbit[0]);
        let dz = v[2] - (-c.f + c.dolly);
        if dz <= 1.0 {
            return None;
        }
        let s = c.zoom * c.f / dz;
        let (x, y) = (super::PIVOT[0] + s * v[0], super::PIVOT[1] + s * v[1]);
        let (sr, cr) = c.roll.to_radians().sin_cos();
        let (dx, dy) = (x - super::PIVOT[0], y - super::PIVOT[1]);
        Some([
            super::PIVOT[0] + cr * dx - sr * dy + c.shake[0],
            super::PIVOT[1] + sr * dx + cr * dy + c.shake[1],
        ])
    }
}

use reference::Cam;

// ---------------------------------------------------------------------------
// Regression: scenes without a perspective camera
// ---------------------------------------------------------------------------

#[test]
fn z_and_tilt_are_ignored_without_a_perspective_camera() {
    let plain = card("c", 540.0, 960.0, 300.0, 200.0);
    let fancy = with(
        with(plain.clone(), "z", json!(900)),
        "tilt",
        json!([20, 30]),
    );
    let build = |layer: Value| {
        parse(&project_json(
            vec![json!({
                "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
                "layers": [layer], "motions": [],
                "camera": { "motions": [
                    cam_motion(0.0, 2.0, json!({ "op": "push", "from": 1.0, "to": 1.4 })),
                    cam_motion(0.0, 2.0, json!({ "op": "track", "from": [0, 0], "to": [60, -40] })),
                ] },
            })],
            json!([]),
        ))
    };
    let (a, b) = (build(plain), build(fancy));
    for f in [0, 20, 45, 90] {
        let (fa, fb) = (frame(&a, f), frame(&b, f));
        assert_eq!(
            serde_json::to_string(&fa).unwrap(),
            serde_json::to_string(&fb).unwrap(),
            "frame {f}"
        );
        let l = &fa.layers[0];
        assert!(l.projective.is_none() && l.blur.is_none());
    }
    // And the plain 2D camera still maps by zoom^depth about the pivot.
    let l = get(&a, 30, "c");
    close(f64::from(l.transform.a), 1.2, 1e-6, "zoom at t = 1");
}

#[test]
fn perspective_camera_without_layers_fields_resolves_all_layers_flat_at_rest() {
    let p = scene(vec![card("c", 300.0, 700.0, 200.0, 120.0)], vec![], vec![]);
    let l = get(&p, 0, "c");
    assert!(l.projective.is_none() && l.blur.is_none());
    // z = 0 at rest maps 1:1: the box origin is (300 - 100, 700 - 60).
    let o = through_affine(&l, 0.0, 0.0);
    assert_pt(o, [200.0, 640.0], 1e-3, "origin");
    close(f64::from(l.transform.a), 1.0, 1e-6, "scale x");
    close(f64::from(l.transform.d), 1.0, 1e-6, "scale y");
    close(f64::from(l.transform.b), 0.0, 1e-6, "skew");
}

// ---------------------------------------------------------------------------
// Projection
// ---------------------------------------------------------------------------

#[test]
fn z_zero_at_rest_matches_the_plain_scene_exactly() {
    let layers = vec![
        card("a", 540.0, 960.0, 300.0, 200.0),
        with(
            card("b", 200.0, 300.0, 120.0, 80.0),
            "rotation_degrees",
            json!(15),
        ),
    ];
    let plain = parse(&project_json(
        vec![
            json!({ "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
                     "layers": layers.clone(), "motions": [] }),
        ],
        json!([]),
    ));
    let p = scene(layers, vec![], vec![]);
    for id in ["a", "b"] {
        let (x, y) = (get(&plain, 10, id), get(&p, 10, id));
        for (u, v) in [(0.0, 0.0), (100.0, 40.0)] {
            assert_pt(through_affine(&y, u, v), through_affine(&x, u, v), 1e-3, id);
        }
        assert!(y.projective.is_none());
    }
}

#[test]
fn positive_z_shrinks_toward_the_pivot_by_f_over_z_plus_f() {
    let f = focal(40.0);
    let z = 700.0;
    let p = scene(
        vec![with(card("far", 140.0, 360.0, 200.0, 100.0), "z", json!(z))],
        vec![],
        vec![],
    );
    let l = get(&p, 0, "far");
    let k = f / (z + f);
    close(f64::from(l.transform.a), k, 1e-5, "scale");
    close(f64::from(l.transform.d), k, 1e-5, "scale y");
    // Box centre (100, 50) -> canvas (140, 360) -> pivot + k * (centre - pivot).
    let c = through_affine(&l, 100.0, 50.0);
    let want = [
        PIVOT[0] + k * (140.0 - PIVOT[0]),
        PIVOT[1] + k * (360.0 - PIVOT[1]),
    ];
    assert_pt(c, want, 1e-3, "centre");
    assert!(l.projective.is_none(), "uniform scale folds into transform");
    // A negative z (nearer than the focus plane) enlarges.
    let near_p = scene(
        vec![with(
            card("n", 140.0, 360.0, 200.0, 100.0),
            "z",
            json!(-500),
        )],
        vec![],
        vec![],
    );
    close(
        f64::from(get(&near_p, 0, "n").transform.a),
        f / (f - 500.0),
        1e-5,
        "near scale",
    );
}

#[test]
fn dolly_enlarges_eases_and_holds() {
    let f = focal(40.0);
    let p = scene(
        vec![
            card("on", 140.0, 360.0, 200.0, 100.0),
            with(card("back", 140.0, 360.0, 200.0, 100.0), "z", json!(600)),
        ],
        vec![dolly(0.0, 500.0)],
        vec![],
    );
    for (frame_no, d) in [
        (0u32, 0.0),
        (15, 125.0),
        (30, 250.0),
        (60, 500.0),
        (100, 500.0),
    ] {
        for (id, z) in [("on", 0.0), ("back", 600.0)] {
            let l = get(&p, frame_no, id);
            let k = f / (z + f - d);
            close(
                f64::from(l.transform.a),
                k,
                1e-5,
                &format!("{id} scale @{frame_no}"),
            );
            let c = through_affine(&l, 100.0, 50.0);
            let want = [
                PIVOT[0] + k * (140.0 - PIVOT[0]),
                PIVOT[1] + k * (360.0 - PIVOT[1]),
            ];
            assert_pt(c, want, 1e-3, &format!("{id} centre @{frame_no}"));
        }
    }
    // Dolly enlarges: monotone in time, and the nearer layer grows faster.
    let s = |fr: u32, id: &str| get(&p, fr, id).transform.a;
    assert!(s(0, "on") < s(30, "on") && s(30, "on") < s(60, "on"));
    assert!(s(60, "on") / s(0, "on") > s(60, "back") / s(0, "back"));
}

#[test]
fn push_multiplies_the_projected_scale_and_track_pans_the_camera() {
    let f = focal(40.0);
    let z = 500.0;
    let p = scene(
        vec![
            card("on", 700.0, 1200.0, 100.0, 100.0),
            with(card("back", 700.0, 1200.0, 100.0, 100.0), "z", json!(z)),
        ],
        vec![
            cam_motion(0.0, 2.0, json!({ "op": "push", "from": 1.0, "to": 1.5 })),
            cam_motion(
                0.0,
                2.0,
                json!({ "op": "track", "from": [0, 0], "to": [80, -40] }),
            ),
        ],
        vec![],
    );
    // t = 1: zoom 1.25, pan (40, -20).
    let mut cam = Cam::rest(f);
    cam.zoom = 1.25;
    cam.pan = [40.0, -20.0];
    for (id, z) in [("on", 0.0), ("back", z)] {
        let l = get(&p, 30, id);
        let want = reference::project(&cam, [700.0, 1200.0, z]).unwrap();
        assert_pt(through_affine(&l, 50.0, 50.0), want, 1e-3, id);
        close(
            f64::from(l.transform.a),
            1.25 * f / (z + f),
            1e-5,
            &format!("{id} scale"),
        );
    }
}

#[test]
fn track_displaces_near_layers_more_than_far_ones() {
    let p = scene(
        vec![
            card("near", 540.0, 960.0, 100.0, 100.0),
            with(card("far", 540.0, 960.0, 100.0, 100.0), "z", json!(1500)),
        ],
        vec![cam_motion(
            0.0,
            2.0,
            json!({ "op": "track", "from": [0, 0], "to": [100, 0] }),
        )],
        vec![],
    );
    let dx = |id: &str| {
        let a = through_affine(&get(&p, 0, id), 50.0, 50.0)[0];
        let b = through_affine(&get(&p, 60, id), 50.0, 50.0)[0];
        a - b
    };
    let f = focal(40.0);
    close(dx("near"), 100.0, 1e-3, "z = 0 layer moves by the pan");
    close(dx("far"), 100.0 * f / (1500.0 + f), 1e-3, "far layer");
    assert!(dx("near") > dx("far"));
}

#[test]
fn depth_parallax_factor_is_ignored_under_perspective() {
    let base = card("c", 700.0, 1200.0, 100.0, 100.0);
    let a = scene(
        vec![base.clone()],
        vec![
            cam_motion(0.0, 2.0, json!({ "op": "push", "from": 1.0, "to": 1.5 })),
            cam_motion(
                0.0,
                2.0,
                json!({ "op": "track", "from": [0, 0], "to": [80, -40] }),
            ),
        ],
        vec![],
    );
    let b = scene(
        vec![with(base, "depth", json!(0.2))],
        vec![
            cam_motion(0.0, 2.0, json!({ "op": "push", "from": 1.0, "to": 1.5 })),
            cam_motion(
                0.0,
                2.0,
                json!({ "op": "track", "from": [0, 0], "to": [80, -40] }),
            ),
        ],
        vec![],
    );
    for f in [0, 30, 60] {
        assert_eq!(
            serde_json::to_string(&frame(&a, f)).unwrap(),
            serde_json::to_string(&frame(&b, f)).unwrap(),
            "frame {f}"
        );
    }
}

// ---------------------------------------------------------------------------
// Tilt (projective)
// ---------------------------------------------------------------------------

#[test]
fn tilt_yields_a_projective_matching_a_hand_computed_projection() {
    let f = focal(40.0);
    // Card centred at (700, 1100), 600 x 400, anchor at its centre.
    let p = scene(
        vec![with(
            card("t", 700.0, 1100.0, 600.0, 400.0),
            "tilt",
            json!([0, 30]),
        )],
        vec![],
        vec![],
    );
    let l = get(&p, 0, "t");
    let h = l.projective.expect("tilt gives a projective");
    // Hand computation: corner offsets from the anchor (+-300, +-200, 0) turn
    // about y by 30 deg: x' = x cos, z' = -x sin; then s = f / (f + z').
    let (c30, s30) = (30f64.to_radians().cos(), 30f64.to_radians().sin());
    let corner = |ox: f64, oy: f64| {
        let (x, y, z) = (ox * c30, oy, -ox * s30);
        let s = f / (f + z);
        [
            PIVOT[0] + s * (700.0 + x - PIVOT[0]),
            PIVOT[1] + s * (1100.0 + y - PIVOT[1]),
        ]
    };
    for (local, off) in [
        ([0.0, 0.0], [-300.0, -200.0]),
        ([600.0, 0.0], [300.0, -200.0]),
        ([600.0, 400.0], [300.0, 200.0]),
        ([0.0, 400.0], [-300.0, 200.0]),
    ] {
        assert_pt(
            through_h(&h, local[0], local[1]),
            corner(off[0], off[1]),
            0.01,
            &format!("corner {local:?}"),
        );
    }
    // The homography is exact for the whole plane: an interior point too.
    assert_pt(
        through_h(&h, 150.0, 300.0),
        corner(-150.0, 100.0),
        0.01,
        "interior point",
    );
    // Right edge (positive yaw) is nearer, so it is taller than the left.
    let left = through_h(&h, 0.0, 400.0)[1] - through_h(&h, 0.0, 0.0)[1];
    let right = through_h(&h, 600.0, 400.0)[1] - through_h(&h, 600.0, 0.0)[1];
    assert!(right > left, "right edge {right} vs left edge {left}");
    // The fallback transform is the flat projection; no content offset.
    close(f64::from(l.transform.a), 1.0, 1e-6, "flat scale");
    assert_eq!(
        (
            l.content_transform.a,
            l.content_transform.d,
            l.content_transform.e,
            l.content_transform.f
        ),
        (1.0, 1.0, 0.0, 0.0)
    );
}

#[test]
fn tilt_about_x_and_a_layer_z_compose() {
    let f = focal(40.0);
    let z = 300.0;
    let p = scene(
        vec![with(
            with(
                card("t", 400.0, 800.0, 500.0, 300.0),
                "tilt",
                json!([25, 0]),
            ),
            "z",
            json!(z),
        )],
        vec![],
        vec![],
    );
    let h = get(&p, 0, "t").projective.expect("projective");
    let (cx, sx) = (25f64.to_radians().cos(), 25f64.to_radians().sin());
    for (local, off) in [
        ([0.0, 0.0], [-250.0, -150.0]),
        ([500.0, 300.0], [250.0, 150.0]),
        ([500.0, 0.0], [250.0, -150.0]),
    ] {
        // Pitch about x through the anchor: y' = y cos, z' = y sin.
        let (x, y, zz) = (off[0], off[1] * cx, z + off[1] * sx);
        let s = f / (f + zz);
        let want = [
            PIVOT[0] + s * (400.0 + x - PIVOT[0]),
            PIVOT[1] + s * (800.0 + y - PIVOT[1]),
        ];
        assert_pt(
            through_h(&h, local[0], local[1]),
            want,
            0.01,
            &format!("{local:?}"),
        );
    }
}

#[test]
fn tilt_uses_the_layer_anchor_and_rest_transform() {
    // Anchor at the top-left corner, layer rotated 20 deg and scaled 1.5.
    let f = focal(40.0);
    let layer = json!({ "id": "t", "type": "rectangle", "x": 300, "y": 500,
                        "width": 400, "height": 200, "anchor_x": 0.0, "anchor_y": 0.0,
                        "rotation_degrees": 20, "scale_x": 1.5, "scale_y": 1.5,
                        "tilt": [10, -35], "fill": "#FFFFFF" });
    let p = scene(vec![layer], vec![], vec![]);
    let h = get(&p, 0, "t").projective.expect("projective");
    let (sr, cr) = 20f64.to_radians().sin_cos();
    let rest = |u: f64, v: f64| {
        [
            300.0 + 1.5 * (cr * u - sr * v),
            500.0 + 1.5 * (sr * u + cr * v),
        ]
    };
    let anchor = rest(0.0, 0.0);
    for (u, v) in [
        (0.0, 0.0),
        (400.0, 0.0),
        (400.0, 200.0),
        (0.0, 200.0),
        (123.0, 77.0),
    ] {
        let r = rest(u, v);
        let o = reference::rot([r[0] - anchor[0], r[1] - anchor[1], 0.0], 10.0, -35.0);
        let world = [anchor[0] + o[0], anchor[1] + o[1], o[2]];
        let want = reference::project(&Cam::rest(f), world).unwrap();
        assert_pt(through_h(&h, u, v), want, 0.01, &format!("({u}, {v})"));
    }
}

#[test]
fn a_vanishing_tilt_agrees_with_the_affine_fold() {
    let f = focal(40.0);
    let layer = with(card("t", 700.0, 1100.0, 600.0, 400.0), "z", json!(400));
    let flat = scene(vec![layer.clone()], vec![dolly(0.0, 200.0)], vec![]);
    let tilted = scene(
        vec![with(layer, "tilt", json!([1e-6, 0]))],
        vec![dolly(0.0, 200.0)],
        vec![],
    );
    let a = get(&flat, 30, "t");
    let b = get(&tilted, 30, "t");
    assert!(a.projective.is_none() && b.projective.is_some());
    let h = b.projective.unwrap();
    let k = f / (400.0 + f - 100.0);
    close(f64::from(a.transform.a), k, 1e-5, "fold scale");
    for (u, v) in [
        (0.0, 0.0),
        (600.0, 0.0),
        (600.0, 400.0),
        (0.0, 400.0),
        (200.0, 90.0),
    ] {
        assert_pt(
            through_h(&h, u, v),
            through_affine(&a, u, v),
            0.01,
            &format!("({u}, {v})"),
        );
    }
}

#[test]
fn clip_reveal_content_offset_is_box_relative_under_a_projective() {
    let layer = with(
        card("t", 540.0, 960.0, 600.0, 400.0),
        "tilt",
        json!([0, 20]),
    );
    let p = scene(
        vec![layer],
        vec![],
        vec![
            json!({ "target": "t", "start": 0.0, "duration": 2.0, "easing": "linear",
                     "op": "clip_reveal", "direction": "right" }),
        ],
    );
    let l = get(&p, 0, "t");
    assert!(l.projective.is_some());
    // Reveal starts displaced one width opposite to travel (box units).
    close(
        f64::from(l.content_transform.e),
        -600.0,
        1e-3,
        "content offset x",
    );
    close(
        f64::from(l.content_transform.f),
        0.0,
        1e-3,
        "content offset y",
    );
    close(
        f64::from(l.content_transform.a),
        1.0,
        1e-6,
        "no scale in the content offset",
    );
}

// ---------------------------------------------------------------------------
// Orbit
// ---------------------------------------------------------------------------

#[test]
fn orbit_rotates_about_the_pivot_and_matches_the_reference() {
    let f = focal(40.0);
    let p = scene(
        vec![
            card("mid", 540.0, 960.0, 200.0, 200.0),
            with(card("off", 800.0, 600.0, 300.0, 200.0), "z", json!(250)),
        ],
        vec![orbit([0.0, 0.0], [24.0, -10.0])],
        vec![],
    );
    // t = 1: halfway: yaw 12, pitch -5.
    let mut cam = Cam::rest(f);
    cam.orbit = [12.0, -5.0];
    let l = get(&p, 30, "off");
    let h = l.projective.expect("orbit gives a projective");
    for (u, v) in [(0.0, 0.0), (300.0, 0.0), (300.0, 200.0), (0.0, 200.0)] {
        let world = [800.0 - 150.0 + u, 600.0 - 100.0 + v, 250.0];
        let want = reference::project(&cam, world).unwrap();
        assert_pt(through_h(&h, u, v), want, 0.01, &format!("({u}, {v})"));
    }
    // A layer centred on the rotation axis keeps its centre under yaw/pitch.
    let mid = get(&p, 30, "mid");
    assert_pt(
        through_h(&mid.projective.unwrap(), 100.0, 100.0),
        PIVOT,
        0.01,
        "centre on the pivot at z = 0",
    );
    // At rest (frame 0 = orbit from [0, 0]) there is nothing to warp.
    assert!(get(&p, 0, "mid").projective.is_none());
    // The orbit holds its end after the motion.
    let late = get(&p, 100, "off").projective.expect("held orbit");
    let held = reference::project(
        &Cam {
            orbit: [24.0, -10.0],
            ..Cam::rest(f)
        },
        [650.0, 500.0, 250.0],
    )
    .unwrap();
    assert_pt(through_h(&late, 0.0, 0.0), held, 0.01, "held orbit");
}

#[test]
fn orbit_moves_near_layers_more_than_far_ones() {
    // Equal and opposite depths about the pivot plane: the nearer layer swings
    // farther on screen (perspective magnifies it).
    let layers = vec![
        with(card("near", 540.0, 960.0, 100.0, 100.0), "z", json!(-500)),
        with(card("far", 540.0, 960.0, 100.0, 100.0), "z", json!(500)),
        card("plane", 540.0, 960.0, 100.0, 100.0),
    ];
    let p = scene(layers, vec![orbit([0.0, 0.0], [20.0, 0.0])], vec![]);
    let centre = |fr: u32, id: &str| {
        let l = get(&p, fr, id);
        match l.projective {
            Some(h) => through_h(&h, 50.0, 50.0),
            None => through_affine(&l, 50.0, 50.0),
        }
    };
    let moved = |id: &str| {
        let (a, b) = (centre(0, id), centre(60, id));
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    };
    assert!(moved("plane") < 1e-2, "the pivot plane does not move");
    assert!(
        moved("near") > moved("far") * 1.3,
        "near {} should swing more than far {}",
        moved("near"),
        moved("far")
    );
    assert!(moved("far") > 50.0);
}

#[test]
fn camera_shake_applies_in_screen_space_after_projection() {
    use motion_core::noise::smooth;
    let f = focal(40.0);
    let (trauma, freq, decay, seed) = (0.6f64, 9.0f64, 2.0f64, 4u32);
    let p = scene(
        vec![
            with(card("near", 700.0, 1000.0, 100.0, 100.0), "z", json!(-300)),
            with(card("far", 700.0, 1000.0, 100.0, 100.0), "z", json!(900)),
        ],
        vec![
            dolly(0.0, 150.0),
            cam_motion(
                0.5,
                1.5,
                json!({ "op": "shake", "trauma": trauma, "frequency": freq,
                        "decay": decay, "seed": seed }),
            ),
        ],
        vec![],
    );
    let t = 1.0; // frame 30: inside the shake window, 0.5 s in
    let e = t - 0.5;
    let g = trauma * trauma * (-decay * e).exp();
    let mut cam = Cam::rest(f);
    cam.dolly = 75.0;
    cam.shake = [
        g * 0.025 * 1080.0 * smooth(seed, freq * e),
        g * 0.025 * 1080.0 * smooth(seed ^ 0x9E37_79B9, freq * e),
    ];
    cam.roll = g * 3.0 * smooth(seed ^ 0x85EB_CA6B, freq * e);
    assert!(cam.shake[0].abs() > 0.01, "the shake is not trivially zero");
    for (id, z) in [("near", -300.0), ("far", 900.0)] {
        let l = get(&p, 30, id);
        let want = reference::project(&cam, [700.0, 1000.0, z]).unwrap();
        assert_pt(through_affine(&l, 50.0, 50.0), want, 1e-3, id);
    }
}

#[test]
fn roll_applies_after_the_orbit_projection() {
    let f = focal(40.0);
    let p = scene(
        vec![with(card("c", 800.0, 600.0, 100.0, 100.0), "z", json!(200))],
        vec![
            cam_motion(0.0, 2.0, json!({ "op": "roll", "from": 0, "to": 40 })),
            orbit([0.0, 0.0], [10.0, 0.0]),
        ],
        vec![],
    );
    // t = 1: roll 20 deg, yaw 5.
    let mut cam = Cam::rest(f);
    cam.roll = 20.0;
    cam.orbit = [5.0, 0.0];
    let h = get(&p, 30, "c").projective.expect("projective");
    for (u, v) in [(0.0, 0.0), (100.0, 100.0)] {
        let want = reference::project(&cam, [750.0 + u, 550.0 + v, 200.0]).unwrap();
        assert_pt(through_h(&h, u, v), want, 0.01, &format!("({u}, {v})"));
    }
}

// ---------------------------------------------------------------------------
// Depth of field
// ---------------------------------------------------------------------------

#[test]
fn depth_of_field_blur_grows_with_distance_from_the_focus_plane() {
    let layers = vec![
        card("focus", 540.0, 960.0, 100.0, 100.0),
        with(card("near", 540.0, 960.0, 100.0, 100.0), "z", json!(-250)),
        with(card("far", 540.0, 960.0, 100.0, 100.0), "z", json!(500)),
        with(card("slight", 540.0, 960.0, 100.0, 100.0), "z", json!(20)),
        with(card("huge", 540.0, 960.0, 100.0, 100.0), "z", json!(5000)),
    ];
    let p = scene_with(layers.clone(), vec![], vec![], persp(40.0, 0.0, 2.0));
    assert_eq!(get(&p, 0, "focus").blur, None);
    // z_view = (z + f) - f = z at rest: blur = aperture * |z - focus| / 100.
    close(
        f64::from(get(&p, 0, "near").blur.unwrap()),
        5.0,
        1e-4,
        "near",
    );
    close(
        f64::from(get(&p, 0, "far").blur.unwrap()),
        10.0,
        1e-4,
        "far",
    );
    // Under the 0.5 px threshold: unset.
    assert_eq!(get(&p, 0, "slight").blur, None);
    // Capped at 24 px.
    close(
        f64::from(get(&p, 0, "huge").blur.unwrap()),
        24.0,
        1e-4,
        "cap",
    );

    // focus_z moves the sharp plane.
    let q = scene_with(layers.clone(), vec![], vec![], persp(40.0, 500.0, 2.0));
    assert_eq!(get(&q, 0, "far").blur, None);
    close(
        f64::from(get(&q, 0, "focus").blur.unwrap()),
        10.0,
        1e-4,
        "focus plane blurs",
    );

    // Dolly changes the view distance: z_view = z - d.
    let r = scene_with(
        layers,
        vec![dolly(0.0, 100.0)],
        vec![],
        persp(40.0, 0.0, 2.0),
    );
    close(
        f64::from(get(&r, 60, "far").blur.unwrap()),
        2.0 * (500.0 - 100.0) / 100.0,
        1e-4,
        "far with dolly",
    );
    close(
        f64::from(get(&r, 60, "focus").blur.unwrap()),
        2.0 * 100.0 / 100.0,
        1e-4,
        "focus layer behind the dolly",
    );
}

#[test]
fn no_blur_without_an_aperture() {
    let p = scene(
        vec![with(
            card("far", 540.0, 960.0, 100.0, 100.0),
            "z",
            json!(3000),
        )],
        vec![],
        vec![],
    );
    assert_eq!(get(&p, 0, "far").blur, None);
}

// ---------------------------------------------------------------------------
// Behind the camera
// ---------------------------------------------------------------------------

#[test]
fn layers_behind_the_camera_are_not_drawn() {
    let f = focal(40.0);
    let p = scene(
        vec![
            card("here", 540.0, 960.0, 100.0, 100.0),
            with(
                card("behind", 540.0, 960.0, 100.0, 100.0),
                "z",
                json!(-f - 10.0),
            ),
            with(
                card("edge", 540.0, 960.0, 100.0, 100.0),
                "z",
                json!(-f + 1.0),
            ),
            with(
                card("just_in_front", 540.0, 960.0, 100.0, 100.0),
                "z",
                json!(-f + 2.0),
            ),
        ],
        vec![],
        vec![],
    );
    assert!(has(&p, 0, "here"));
    assert!(!has(&p, 0, "behind"));
    assert!(!has(&p, 0, "edge"), "z_world - cam_z = 1 is already behind");
    assert!(has(&p, 0, "just_in_front"));
    assert_eq!(frame(&p, 0).layers.len(), 2);

    // Dolly through the focus plane: the z = 0 layer disappears once the
    // camera reaches it.
    let q = scene(
        vec![card("c", 540.0, 960.0, 100.0, 100.0)],
        vec![dolly(0.0, f + 100.0)],
        vec![],
    );
    assert!(has(&q, 0, "c"));
    assert!(has(&q, 30, "c"));
    assert!(!has(&q, 60, "c"), "camera is past the layer");
    assert!(!has(&q, 100, "c"));
}

#[test]
fn a_card_swung_behind_the_camera_by_an_orbit_is_skipped() {
    let f = focal(40.0);
    // A wide card at z = -f + 100 turned 80 deg about y: one end is behind.
    let p = scene(
        vec![with(
            with(
                card("c", 540.0, 960.0, 800.0, 200.0),
                "z",
                json!(-f + 100.0),
            ),
            "tilt",
            json!([0, 80]),
        )],
        vec![],
        vec![],
    );
    assert!(!has(&p, 0, "c"));
    let q = scene(
        vec![with(
            with(card("c", 540.0, 960.0, 800.0, 200.0), "z", json!(0)),
            "tilt",
            json!([0, 80]),
        )],
        vec![],
        vec![],
    );
    assert!(has(&q, 0, "c"));
}

#[test]
fn an_edge_on_card_is_skipped_instead_of_producing_nonsense() {
    let p = scene(
        vec![with(
            card("c", 540.0, 960.0, 600.0, 400.0),
            "tilt",
            json!([0, 90]),
        )],
        vec![],
        vec![],
    );
    assert!(!has(&p, 0, "c"), "a zero-area projection is not drawn");
    // A hair off edge-on still draws, with finite coefficients.
    let q = scene(
        vec![with(
            card("c", 540.0, 960.0, 600.0, 400.0),
            "tilt",
            json!([0, 89]),
        )],
        vec![],
        vec![],
    );
    let h = get(&q, 0, "c").projective.expect("projective");
    assert!(h.iter().all(|v| v.is_finite()));
}

// ---------------------------------------------------------------------------
// Groups: children are not projected individually
// ---------------------------------------------------------------------------

fn group_with_children(extra: Value) -> Value {
    let mut g = json!({
        "id": "g", "type": "group", "x": 100, "y": 200, "width": 800, "height": 600,
        "anchor_x": 0.5, "anchor_y": 0.5,
        "children": [
            { "id": "a", "type": "rectangle", "x": 50, "y": 60, "width": 300, "height": 200,
              "fill": "#FFFFFF" },
            { "id": "b", "type": "rectangle", "x": 400, "y": 300, "width": 250, "height": 150,
              "fill": "#FF0000" },
        ],
    });
    for (k, v) in extra.as_object().expect("object") {
        g[k] = v.clone();
    }
    g
}

#[test]
fn a_tilted_group_warps_its_leaves_and_blurs_as_a_whole() {
    let f = focal(40.0);
    let z = 200.0;
    let g = group_with_children(json!({ "z": z, "tilt": [0, 25] }));
    let p = scene_with(vec![g], vec![], vec![], persp(40.0, 0.0, 2.0));
    let fr = frame(&p, 0);
    let group = &fr.layers[0];
    assert!(
        group.projective.is_none(),
        "groups are never warped themselves"
    );
    close(
        f64::from(group.blur.expect("group blur")),
        2.0 * z / 100.0,
        1e-4,
        "group blur",
    );
    assert_eq!(group.children.len(), 2);
    // Rest geometry of the group: box origin (100 - 400, 200 - 300) = (-300, -100);
    // anchor (box centre) at canvas (100, 200).
    let anchor = [100.0, 200.0];
    let origin = [-300.0, -100.0];
    for (id, w, h, at) in [
        ("a", 300.0, 200.0, [50.0, 60.0]),
        ("b", 250.0, 150.0, [400.0, 300.0]),
    ] {
        let leaf = &group.children[if id == "a" { 0 } else { 1 }];
        assert_eq!(leaf.id, id);
        assert!(leaf.blur.is_none(), "blur lives on the top-level layer");
        let hm = leaf.projective.expect("leaf homography");
        for (u, v) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h), (w / 3.0, h / 2.0)] {
            let rest = [origin[0] + at[0] + u, origin[1] + at[1] + v];
            let o = reference::rot([rest[0] - anchor[0], rest[1] - anchor[1], 0.0], 0.0, 25.0);
            let world = [anchor[0] + o[0], anchor[1] + o[1], z + o[2]];
            let want = reference::project(&Cam::rest(f), world).unwrap();
            assert_pt(
                through_h(&hm, u, v),
                want,
                0.01,
                &format!("{id} ({u}, {v})"),
            );
        }
    }
}

#[test]
fn an_orbited_group_warps_every_leaf_and_nested_groups_are_not_warped() {
    let g = json!({
        "id": "g", "type": "group", "width": 1080, "height": 1920,
        "children": [
            { "id": "inner", "type": "group", "x": 100, "y": 100, "width": 500, "height": 500,
              "children": [
                { "id": "deep", "type": "rectangle", "x": 20, "y": 30, "width": 100,
                  "height": 80, "fill": "#FFFFFF" } ] },
        ],
    });
    let p = scene(vec![g], vec![orbit([0.0, 0.0], [30.0, 0.0])], vec![]);
    let fr = frame(&p, 60);
    let group = &fr.layers[0];
    assert!(group.projective.is_none());
    let inner = &group.children[0];
    assert!(inner.projective.is_none());
    let deep = &inner.children[0];
    let h = deep.projective.expect("leaf warped");
    let mut cam = Cam::rest(focal(40.0));
    cam.orbit = [30.0, 0.0];
    for (u, v) in [(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (0.0, 80.0)] {
        let want = reference::project(&cam, [120.0 + u, 130.0 + v, 0.0]).unwrap();
        assert_pt(through_h(&h, u, v), want, 0.01, &format!("({u}, {v})"));
    }
}

#[test]
fn an_untilted_group_folds_into_its_children_transforms() {
    let f = focal(40.0);
    let z = 500.0;
    let g = group_with_children(json!({ "z": z }));
    let p = scene(vec![g], vec![dolly(0.0, 100.0)], vec![]);
    let fr = frame(&p, 60);
    let group = &fr.layers[0];
    assert!(group.projective.is_none());
    let k = f / (z + f - 100.0);
    close(f64::from(group.transform.a), k, 1e-5, "group scale");
    for child in &group.children {
        assert!(child.projective.is_none(), "{}", child.id);
        close(
            f64::from(child.transform.a),
            k,
            1e-5,
            &format!("{} scale", child.id),
        );
        // Child box origin in rest canvas px: group box origin + child (x, y).
        let at = if child.id == "a" {
            [50.0, 60.0]
        } else {
            [400.0, 300.0]
        };
        let rest = [-300.0 + at[0], -100.0 + at[1]];
        let want = [
            PIVOT[0] + k * (rest[0] - PIVOT[0]),
            PIVOT[1] + k * (rest[1] - PIVOT[1]),
        ];
        assert_pt(through_affine(child, 0.0, 0.0), want, 1e-3, child.id);
    }
}

/// Everything the resolved tree says about placement.
fn placement(layers: &[ResolvedLayer<'_>]) -> Vec<String> {
    let mut out = Vec::new();
    for l in layers {
        out.push(format!(
            "{} {:?} {:?} {:?} {:?}",
            l.id, l.transform, l.content_transform, l.projective, l.blur
        ));
        out.extend(placement(&l.children));
    }
    out
}

#[test]
fn z_and_tilt_on_group_children_are_ignored() {
    let mut g = group_with_children(json!({}));
    g["children"][0]["z"] = json!(900);
    g["children"][0]["tilt"] = json!([30, 30]);
    let p = scene(vec![g], vec![], vec![]);
    let q = scene(vec![group_with_children(json!({}))], vec![], vec![]);
    let (fp, fq) = (frame(&p, 0), frame(&q, 0));
    let placed = placement(&fp.layers);
    assert_eq!(placed.len(), 3);
    assert_eq!(placed, placement(&fq.layers));
    assert!(fp.layers[0].children.iter().all(|c| c.projective.is_none()));
}

// ---------------------------------------------------------------------------
// Echo, shared elements, determinism
// ---------------------------------------------------------------------------

#[test]
fn echo_ghosts_project_like_their_layer_at_the_ghost_time() {
    let f = focal(40.0);
    let layer = with(
        with(card("ball", 200.0, 500.0, 100.0, 100.0), "z", json!(400)),
        "tilt",
        json!([0, 20]),
    );
    let p = scene_with(
        vec![layer],
        vec![dolly(0.0, 200.0)],
        vec![
            json!({ "target": "ball", "start": 0.0, "duration": 2.0, "easing": "linear",
                    "op": "move", "from": [0, 0], "to": [400, 0] }),
            json!({ "target": "ball", "start": 0.5, "duration": 1.0, "easing": "linear",
                    "op": "echo", "count": 2, "spacing": 0.1, "decay": 0.5 }),
        ],
        persp(40.0, 0.0, 2.0),
    );
    let fr = frame(&p, 30); // t = 1.0
    let balls: Vec<&ResolvedLayer> = fr.layers.iter().filter(|l| l.id == "ball").collect();
    assert_eq!(balls.len(), 3);
    // Ghost k sits at the ghost time's position; the camera is the current one
    // (dolly 100 at t = 1).
    let mut cam = Cam::rest(f);
    cam.dolly = 100.0;
    for (i, k) in [2u32, 1, 0].iter().enumerate() {
        let tk = 1.0 - f64::from(*k) * 0.1;
        let cx = 200.0 + 200.0 * tk;
        let h = balls[i].projective.expect("ghost projective");
        let (c20, s20) = (20f64.to_radians().cos(), 20f64.to_radians().sin());
        for (u, v, ox, oy) in [(0.0, 0.0, -50.0, -50.0), (100.0, 100.0, 50.0, 50.0)] {
            let (x, y, z) = (ox * c20, oy, 400.0 - ox * s20);
            let want = reference::project(&cam, [cx + x, 500.0 + y, z]).unwrap();
            assert_pt(
                through_h(&h, u, v),
                want,
                0.01,
                &format!("ghost k={k} ({u}, {v})"),
            );
        }
        assert!(balls[i].blur.is_some(), "ghosts blur like their layer");
    }
}

#[test]
fn free_shared_elements_follow_the_z0_plane_of_the_perspective_camera() {
    let f = focal(40.0);
    let track = json!([
        { "scene": "s", "at": 0.0, "state": { "x": 800, "y": 1200 } },
        { "scene": "s", "at": 2.0, "state": { "x": 800, "y": 1200 } }
    ]);
    let shared = json!([{ "id": "el", "layer": {
        "id": "sh", "type": "rectangle", "width": 20, "height": 20, "fill": "#CC2200" },
        "track": track }]);
    let p = parse(&project_json(
        vec![json!({
            "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0, "layers": [],
            "motions": [],
            "camera": { "perspective": persp(40.0, 0.0, 0.0), "motions": [dolly(0.0, 300.0)] },
        })],
        shared,
    ));
    let fr = frame(&p, 30); // dolly 150
    let sh = find_in(&fr.layers, "sh").expect("shared layer");
    let k = f / (f - 150.0);
    close(f64::from(sh.transform.a), k, 1e-5, "scale");
    let want = [
        PIVOT[0] + k * (800.0 - PIVOT[0]),
        PIVOT[1] + k * (1200.0 - PIVOT[1]),
    ];
    // The element's anchor (top-left, anchor 0) sits at the projected point.
    assert_pt(through_affine(sh, 0.0, 0.0), want, 1e-3, "position");
}

#[test]
fn evaluation_is_deterministic_and_order_independent() {
    let layers = vec![
        with(
            with(
                card("a", 400.0, 800.0, 300.0, 200.0),
                "tilt",
                json!([12, -20]),
            ),
            "z",
            json!(150),
        ),
        with(card("b", 700.0, 1300.0, 200.0, 200.0), "z", json!(-200)),
        group_with_children(json!({ "z": 80, "tilt": [0, 10] })),
    ];
    let cam = vec![
        dolly(0.0, 250.0),
        orbit([-10.0, 4.0], [14.0, -6.0]),
        cam_motion(
            0.5,
            1.0,
            json!({ "op": "shake", "trauma": 0.5, "frequency": 9, "decay": 2, "seed": 4 }),
        ),
    ];
    let p = scene_with(layers.clone(), cam.clone(), vec![], persp(40.0, 30.0, 3.0));
    let q = scene_with(layers, cam, vec![], persp(40.0, 30.0, 3.0));
    let json_at = |p: &MotionProject, f: u32| serde_json::to_string(&frame(p, f)).unwrap();
    let forward: Vec<String> = (0..90).step_by(7).map(|f| json_at(&p, f)).collect();
    let backward: Vec<String> = (0..90).step_by(7).rev().map(|f| json_at(&q, f)).collect();
    let mut backward = backward;
    backward.reverse();
    assert_eq!(forward, backward);
    // Something actually moved between the first and the last sampled frame.
    assert_ne!(forward[0], forward[forward.len() - 1]);
}
