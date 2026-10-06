//! (0.19) `tilt` motion op: a layer's plane turns in 3D over time, and under a
//! `billboard` camera it turns about its own centre without shearing with the
//! orbit (docs/MOTION_OPS_2.md, "(0.19)"). Canvas 1080x1920 at 30 fps, pivot
//! (540, 960), fov 40, so `f = 960 / tan(20 deg)`.

use motion_core::scene::MotionProject;
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
use motion_core::validate::validate;
use serde_json::{json, Value};

const FPS: u32 = 30;

fn focal(fov_deg: f64) -> f64 {
    960.0 / (fov_deg.to_radians() / 2.0).tan()
}

fn parse(v: &Value) -> MotionProject {
    MotionProject::from_json(&v.to_string()).expect("test project parses")
}

/// A 4 s perspective scene (fov 40, no depth of field) with one layer `c`, the
/// camera motions and the layer motions.
fn scene(billboard: bool, camera_motions: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    let layer = json!({ "id": "c", "type": "rectangle", "x": 540.0, "y": 960.0, "width": 400.0,
        "height": 300.0, "anchor_x": 0.5, "anchor_y": 0.5, "fill": "#FFFFFF" });
    parse(&json!({
        "version": "0.2",
        "project": { "name": "tilt_motion" },
        "canvas": { "width": 1080, "height": 1920, "fps": FPS, "background": "#000000" },
        "scenes": [{
            "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
            "layers": [layer], "motions": motions,
            "camera": {
                "perspective": { "fov_deg": 40.0, "focus_z": 0.0, "aperture": 0.0,
                                 "billboard": billboard },
                "motions": camera_motions,
            },
        }],
        "shared": [],
    }))
}

fn tilt(start: f64, dur: f64, from: [f64; 2], to: [f64; 2]) -> Value {
    json!({ "target": "c", "start": start, "duration": dur, "easing": "linear",
            "op": "tilt", "from": from, "to": to })
}

fn orbit(yaw: f64, pitch: f64) -> Value {
    json!({ "start": 0.0, "duration": 2.0, "easing": "linear",
            "op": "orbit", "from": [yaw, pitch], "to": [yaw, pitch] })
}

fn frame(p: &MotionProject, f: u32) -> ResolvedFrame<'_> {
    evaluate_frame(p, f).expect("frame evaluates")
}

fn get<'a>(p: &'a MotionProject, f: u32, id: &str) -> ResolvedLayer<'a> {
    frame(p, f)
        .layers
        .iter()
        .find(|l| l.id == id)
        .cloned()
        .unwrap_or_else(|| panic!("layer '{id}' missing at frame {f}"))
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

fn through_h(h: &[f32; 9], x: f64, y: f64) -> [f64; 2] {
    let h: Vec<f64> = h.iter().map(|v| f64::from(*v)).collect();
    let w = h[6] * x + h[7] * y + h[8];
    [
        (h[0] * x + h[1] * y + h[2]) / w,
        (h[3] * x + h[4] * y + h[5]) / w,
    ]
}

/// Canvas point of a box-space point, whether the layer is affine or warped.
fn at(l: &ResolvedLayer<'_>, x: f64, y: f64) -> [f64; 2] {
    match &l.projective {
        Some(h) => through_h(h, x, y),
        None => through_affine(l, x, y),
    }
}

fn assert_pt(got: [f64; 2], want: [f64; 2], tol: f64, what: &str) {
    assert!(
        (got[0] - want[0]).abs() <= tol && (got[1] - want[1]).abs() <= tol,
        "{what}: got {got:?}, expected {want:?}"
    );
}

/// Height of the layer's left / right edge on the canvas.
fn edge_heights(l: &ResolvedLayer<'_>) -> (f64, f64) {
    let left = at(l, 0.0, 300.0)[1] - at(l, 0.0, 0.0)[1];
    let right = at(l, 400.0, 300.0)[1] - at(l, 400.0, 0.0)[1];
    (left.abs(), right.abs())
}

// ---------------------------------------------------------------------------
// validation
// ---------------------------------------------------------------------------

#[test]
fn tilt_angles_must_be_finite_and_within_89_degrees() {
    let ok = scene(false, vec![], vec![tilt(0.0, 1.0, [0.0, 40.0], [0.0, 0.0])]);
    assert!(validate(&ok, None).is_ok(), "{:?}", validate(&ok, None));
    let bad = scene(
        false,
        vec![],
        vec![tilt(0.0, 1.0, [0.0, 120.0], [0.0, 0.0])],
    );
    let err = validate(&bad, None).expect_err("120 degrees is out of range");
    assert!(
        err.to_string().contains("tilt angles"),
        "unexpected error: {err}"
    );
}

// ---------------------------------------------------------------------------
// the plain perspective path
// ---------------------------------------------------------------------------

#[test]
fn tilt_turns_the_plane_then_holds_its_end_value() {
    // Positive yaw brings the right (+x) side nearer: a taller right edge.
    let p = scene(false, vec![], vec![tilt(0.0, 1.0, [0.0, 40.0], [0.0, 0.0])]);
    let start = get(&p, 0, "c");
    assert!(start.projective.is_some(), "tilted at the start");
    let (l, r) = edge_heights(&start);
    assert!(r > l * 1.05, "right edge nearer: {l} vs {r}");

    let mid = get(&p, 15, "c");
    let (l2, r2) = edge_heights(&mid);
    assert!(
        (r2 / l2) > 1.0 && (r2 / l2) < (r / l),
        "half way: {}",
        r2 / l2
    );

    // After the motion the end value (flat) holds, exactly like an untilted layer.
    let plain = scene(false, vec![], vec![]);
    let end = get(&p, 60, "c");
    assert!(end.projective.is_none(), "flat after the turn");
    assert_pt(
        [f64::from(end.transform.e), f64::from(end.transform.f)],
        [
            f64::from(get(&plain, 60, "c").transform.e),
            f64::from(get(&plain, 60, "c").transform.f),
        ],
        1e-4,
        "same placement as a never-tilted layer",
    );
}

#[test]
fn tilt_about_x_raises_the_top_edge() {
    let p = scene(
        false,
        vec![],
        vec![tilt(0.0, 1.0, [35.0, 0.0], [35.0, 0.0])],
    );
    let l = get(&p, 0, "c");
    let top = at(&l, 400.0, 0.0)[0] - at(&l, 0.0, 0.0)[0];
    let bottom = at(&l, 400.0, 300.0)[0] - at(&l, 0.0, 300.0)[0];
    // Positive pitch brings the top (-y) nearer: a wider top edge.
    assert!(top > bottom * 1.05, "top {top} vs bottom {bottom}");
}

// ---------------------------------------------------------------------------
// billboard
// ---------------------------------------------------------------------------

#[test]
fn billboard_tilt_turns_about_the_centre_and_ignores_the_orbit() {
    let f = focal(40.0);
    let turn = vec![tilt(0.0, 1.0, [0.0, 40.0], [0.0, 40.0])];
    let a = scene(true, vec![orbit(20.0, 0.0)], turn.clone());
    let b = scene(true, vec![orbit(-20.0, 5.0)], turn);
    let la = get(&a, 0, "c");
    let lb = get(&b, 0, "c");
    assert!(la.projective.is_some() && lb.projective.is_some());

    // The box centre does not move under the turn: it is where the flat
    // billboard sprite would put it.
    let flat = scene(true, vec![orbit(20.0, 0.0)], vec![]);
    let sprite = get(&flat, 0, "c");
    assert!(sprite.projective.is_none());
    assert_pt(
        at(&la, 200.0, 150.0),
        through_affine(&sprite, 200.0, 150.0),
        1e-2,
        "centre stays on the sprite track",
    );

    // Same view distance and tilt => the same turned card whatever the orbit:
    // compare the edge-height ratio against a local-perspective hand value.
    let dz = f / f64::from(sprite.transform.a);
    let (hl, hr) = edge_heights(&la);
    let yaw = 40f64.to_radians();
    let near = f / (dz - 200.0 * yaw.sin());
    let far = f / (dz + 200.0 * yaw.sin());
    close(
        hr / hl,
        near / far,
        5e-3,
        "edge ratio from the local perspective",
    );
    let (kl, kr) = edge_heights(&lb);
    close(
        kr / kl,
        hr / hl,
        5e-3,
        "the orbit does not change the shape",
    );
}

#[test]
fn billboard_tilt_settles_without_a_pop() {
    let orbit = vec![orbit(20.0, 0.0)];
    let p = scene(
        true,
        orbit.clone(),
        vec![tilt(0.0, 1.0, [0.0, 1e-3], [0.0, 1e-3])],
    );
    let flat = scene(true, orbit, vec![]);
    let (a, b) = (get(&p, 0, "c"), get(&flat, 0, "c"));
    assert!(a.projective.is_some() && b.projective.is_none());
    for (x, y) in [(0.0, 0.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)] {
        assert_pt(
            at(&a, x, y),
            through_affine(&b, x, y),
            0.05,
            "a vanishing tilt lands on the sprite's corner",
        );
    }
}

#[test]
fn a_tilt_motion_replaces_the_layers_static_tilt() {
    let layer = json!({ "id": "c", "type": "rectangle", "x": 540.0, "y": 960.0, "width": 400.0,
        "height": 300.0, "anchor_x": 0.5, "anchor_y": 0.5, "fill": "#FFFFFF", "tilt": [0, -30] });
    let mut doc = serde_json::to_value(scene(false, vec![], vec![])).expect("serializes");
    doc["scenes"][0]["layers"] = json!([layer]);
    doc["scenes"][0]["motions"] = json!([tilt(0.0, 1.0, [0.0, 30.0], [0.0, 30.0])]);
    let p = parse(&doc);
    let (l, r) = edge_heights(&get(&p, 0, "c"));
    assert!(
        r > l,
        "the motion's +30 wins over the static -30: {l} vs {r}"
    );
}
