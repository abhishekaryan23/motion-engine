//! 2D multi-plane camera: push (zoom) and track (pan) resolved per layer depth.
//!
//! Canvas is 1080x1920 (default pivot = (540, 960)), 30 fps.

use motion_core::scene::MotionProject;
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
use serde_json::{json, Value};

const TOL: f32 = 0.01;
const PIVOT: (f32, f32) = (540.0, 960.0);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn project_with(scenes: Vec<Value>, shared: Value) -> MotionProject {
    let v = json!({
        "version": "0.2",
        "project": { "name": "camera" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#000000" },
        "scenes": scenes,
        "shared": shared,
    });
    MotionProject::from_json(&v.to_string()).expect("test project parses")
}

fn scene_json(id: &str, start: f64, dur: f64, layers: Vec<Value>, camera: Option<Value>) -> Value {
    let mut s = json!({ "id": id, "start_seconds": start, "duration_seconds": dur,
                        "layers": layers, "motions": [] });
    if let Some(c) = camera {
        s["camera"] = c;
    }
    s
}

fn project(layers: Vec<Value>, camera: Option<Value>) -> MotionProject {
    project_with(vec![scene_json("s", 0.0, 4.0, layers, camera)], json!([]))
}

fn dot(id: &str, x: f64, y: f64, depth: Option<f64>) -> Value {
    let mut l = json!({ "id": id, "type": "rectangle", "x": x, "y": y, "width": 10, "height": 10,
                        "fill": "#FFFFFF" });
    if let Some(d) = depth {
        l["depth"] = json!(d);
    }
    l
}

fn cam_motion(start: f64, dur: f64, op: Value) -> Value {
    let mut m = json!({ "start": start, "duration": dur, "easing": "linear" });
    for (k, v) in op.as_object().expect("op object") {
        m[k] = v.clone();
    }
    m
}

fn push(from: f64, to: f64) -> Value {
    cam_motion(0.0, 2.0, json!({ "op": "push", "from": from, "to": to }))
}

fn track(to: [f64; 2]) -> Value {
    cam_motion(0.0, 2.0, json!({ "op": "track", "from": [0, 0], "to": to }))
}

fn camera(motions: Vec<Value>) -> Value {
    json!({ "motions": motions })
}

fn find_in<'a, 'b>(layers: &'b [ResolvedLayer<'a>], id: &str) -> Option<&'b ResolvedLayer<'a>> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let Some(f) = find_in(&l.children, id) {
            return Some(f);
        }
    }
    None
}

fn get<'a>(f: &ResolvedFrame<'a>, id: &str) -> ResolvedLayer<'a> {
    find_in(&f.layers, id)
        .cloned()
        .unwrap_or_else(|| panic!("layer '{id}' missing at frame {}", f.frame))
}

/// Canvas position of a layer's box origin.
fn origin(p: &MotionProject, frame: u32, id: &str) -> (f32, f32) {
    let f = evaluate_frame(p, frame).unwrap();
    let l = get(&f, id);
    (l.transform.e, l.transform.f)
}

fn near(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < TOL, "{what}: got {a}, expected {b}");
}

/// Expected canvas point for a plane at `depth` with camera (`zoom`, `pan`).
fn expect(p: (f32, f32), zoom: f32, pan: (f32, f32), depth: f32) -> (f32, f32) {
    let z = zoom.powf(depth);
    (
        PIVOT.0 + z * (p.0 - pan.0 * depth - PIVOT.0),
        PIVOT.1 + z * (p.1 - pan.1 * depth - PIVOT.1),
    )
}

// ---------------------------------------------------------------------------
// Push and depth
// ---------------------------------------------------------------------------

#[test]
fn push_scales_with_depth_and_depth_zero_is_static() {
    let depths: [f32; 4] = [0.0, 0.5, 1.0, 1.5];
    let layers: Vec<Value> = depths
        .iter()
        .enumerate()
        .map(|(i, d)| dot(&format!("d{i}"), 800.0, 1200.0, Some(f64::from(*d))))
        .collect();
    let p = project(layers, Some(camera(vec![push(1.0, 2.0)])));

    for frame in [0, 15, 30, 45, 60] {
        let zoom = 1.0 + frame as f32 / 60.0;
        let mut prev_disp = -1.0f32;
        for (i, d) in depths.iter().enumerate() {
            let got = origin(&p, frame, &format!("d{i}"));
            let want = expect((800.0, 1200.0), zoom, (0.0, 0.0), *d);
            near(got.0, want.0, &format!("frame {frame} depth {d} x"));
            near(got.1, want.1, &format!("frame {frame} depth {d} y"));
            let disp = ((got.0 - 800.0).powi(2) + (got.1 - 1200.0).powi(2)).sqrt();
            if *d == 0.0 {
                assert!(disp < 1e-3, "depth 0 is static, moved {disp}");
            }
            assert!(disp >= prev_disp - 1e-4, "displacement grows with depth");
            prev_disp = disp;
        }
    }
    // Depth 1 at full push: p' = pivot + 2 * (p - pivot).
    let got = origin(&p, 60, "d2");
    near(got.0, 540.0 + 2.0 * 260.0, "d2 x");
    near(got.1, 960.0 + 2.0 * 240.0, "d2 y");
}

#[test]
fn push_also_scales_the_layer_itself() {
    let p = project(
        vec![dot("a", 800.0, 1200.0, None)],
        Some(camera(vec![push(1.0, 2.0)])),
    );
    let f = evaluate_frame(&p, 60).unwrap();
    near(get(&f, "a").transform.a, 2.0, "zoom on transform");
}

#[test]
fn custom_pivot_is_respected() {
    let cam = json!({ "pivot": [100.0, 200.0], "motions": [push(1.0, 3.0)] });
    let p = project(vec![dot("a", 300.0, 400.0, None)], Some(cam));
    let got = origin(&p, 60, "a");
    near(got.0, 100.0 + 3.0 * 200.0, "x");
    near(got.1, 200.0 + 3.0 * 200.0, "y");
}

#[test]
fn track_amplitude_scales_with_depth() {
    let p = project(
        vec![
            dot("far", 400.0, 500.0, Some(0.5)),
            dot("mid", 400.0, 500.0, Some(1.0)),
            dot("near", 400.0, 500.0, Some(2.0)),
        ],
        Some(camera(vec![track([100.0, -40.0])])),
    );
    for frame in [0, 30, 60] {
        let k = frame as f32 / 60.0;
        for (id, d) in [("far", 0.5), ("mid", 1.0), ("near", 2.0)] {
            let got = origin(&p, frame, id);
            near(got.0, 400.0 - 100.0 * k * d, &format!("{id} x @{frame}"));
            near(got.1, 500.0 + 40.0 * k * d, &format!("{id} y @{frame}"));
        }
    }
}

#[test]
fn push_and_track_combine() {
    let p = project(
        vec![dot("a", 700.0, 900.0, Some(1.5))],
        Some(camera(vec![push(1.0, 1.5), track([80.0, 20.0])])),
    );
    let got = origin(&p, 60, "a");
    let want = expect((700.0, 900.0), 1.5, (80.0, 20.0), 1.5);
    near(got.0, want.0, "x");
    near(got.1, want.1, "y");
}

#[test]
fn camera_holds_its_from_value_before_the_first_motion() {
    let late = cam_motion(1.0, 1.0, json!({ "op": "push", "from": 1.5, "to": 2.0 }));
    let p = project(
        vec![dot("a", 800.0, 1200.0, None)],
        Some(camera(vec![late])),
    );
    let want = expect((800.0, 1200.0), 1.5, (0.0, 0.0), 1.0);
    let got = origin(&p, 0, "a");
    near(got.0, want.0, "held from before start");
    near(got.1, want.1, "held from before start");
    // After the end the `to` value holds.
    let want = expect((800.0, 1200.0), 2.0, (0.0, 0.0), 1.0);
    let got = origin(&p, 100, "a");
    near(got.0, want.0, "held to after end");
}

// ---------------------------------------------------------------------------
// Depth inheritance
// ---------------------------------------------------------------------------

#[test]
fn depth_is_inherited_by_children_and_can_be_overridden() {
    let group = json!({
        "id": "g", "type": "group", "depth": 0.5, "width": 1080, "height": 1920,
        "children": [
            dot("inherits", 800.0, 1200.0, None),
            {
                "id": "sub", "type": "group", "depth": 1.5, "width": 100, "height": 100,
                "children": [ dot("deep", 800.0, 1200.0, None) ]
            },
            dot("explicit", 800.0, 1200.0, Some(0.0)),
        ]
    });
    let p = project(vec![group], Some(camera(vec![push(1.0, 2.0)])));
    for (id, depth) in [("inherits", 0.5), ("deep", 1.5), ("explicit", 0.0)] {
        let got = origin(&p, 60, id);
        let want = expect((800.0, 1200.0), 2.0, (0.0, 0.0), depth);
        near(got.0, want.0, &format!("{id} x"));
        near(got.1, want.1, &format!("{id} y"));
    }
}

#[test]
fn group_transform_is_applied_in_scene_space_before_the_camera() {
    // Group at (100, 100), child at (700, 800) inside it: scene point (800, 900).
    let group = json!({
        "id": "g", "type": "group", "x": 100, "y": 100, "width": 500, "height": 500,
        "children": [ dot("c", 700.0, 800.0, None) ]
    });
    let p = project(vec![group], Some(camera(vec![push(1.0, 2.0)])));
    let got = origin(&p, 60, "c");
    let want = expect((800.0, 900.0), 2.0, (0.0, 0.0), 1.0);
    near(got.0, want.0, "x");
    near(got.1, want.1, "y");
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[test]
fn no_camera_equals_identity_camera() {
    let layers = vec![
        dot("a", 100.0, 200.0, None),
        dot("b", 300.0, 400.0, Some(0.3)),
        json!({ "id": "g", "type": "group", "x": 10, "y": 20, "rotation_degrees": 5,
                "width": 100, "height": 100, "children": [ dot("c", 5.0, 6.0, Some(2.0)) ] }),
    ];
    let none = project(layers.clone(), None);
    let empty = project(layers, Some(camera(vec![])));
    for frame in [0, 20, 90] {
        let (a, b) = (
            evaluate_frame(&none, frame).unwrap(),
            evaluate_frame(&empty, frame).unwrap(),
        );
        for id in ["a", "b", "g", "c"] {
            let (la, lb) = (get(&a, id), get(&b, id));
            for (x, y) in [
                (la.transform.a, lb.transform.a),
                (la.transform.b, lb.transform.b),
                (la.transform.c, lb.transform.c),
                (la.transform.d, lb.transform.d),
                (la.transform.e, lb.transform.e),
                (la.transform.f, lb.transform.f),
            ] {
                near(x, y, &format!("{id} @{frame}"));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared elements and multi-scene continuity
// ---------------------------------------------------------------------------

fn shared_dot(track: Value, depth: Option<f64>) -> Value {
    let mut layer = json!({ "id": "sh", "type": "rectangle", "width": 20, "height": 20,
                            "fill": "#CC2200" });
    if let Some(d) = depth {
        layer["depth"] = json!(d);
    }
    json!([{ "id": "el", "layer": layer, "track": track }])
}

#[test]
fn free_shared_key_follows_the_key_scene_camera() {
    let track = json!([
        { "scene": "s", "at": 0.0, "state": { "x": 800, "y": 1200 } },
        { "scene": "s", "at": 2.0, "state": { "x": 800, "y": 1200 } }
    ]);
    let p = project_with(
        vec![scene_json(
            "s",
            0.0,
            4.0,
            vec![],
            Some(camera(vec![push(1.0, 2.0)])),
        )],
        shared_dot(track, None),
    );
    for frame in [0, 15, 30, 45, 60] {
        let zoom = 1.0 + frame as f32 / 60.0;
        let f = evaluate_frame(&p, frame).unwrap();
        let sh = get(&f, "sh");
        let want = expect((800.0, 1200.0), zoom, (0.0, 0.0), 1.0);
        near(sh.transform.e, want.0, &format!("x @{frame}"));
        near(sh.transform.f, want.1, &format!("y @{frame}"));
        near(sh.transform.a, zoom, &format!("scale @{frame}"));
    }
}

#[test]
fn free_shared_key_respects_layer_depth() {
    let track = json!([
        { "scene": "s", "at": 0.0, "state": { "x": 800, "y": 1200 } },
        { "scene": "s", "at": 1.0, "state": { "x": 800, "y": 1200 } }
    ]);
    let p = project_with(
        vec![scene_json(
            "s",
            0.0,
            4.0,
            vec![],
            Some(camera(vec![push(1.0, 2.0)])),
        )],
        shared_dot(track, Some(0.0)),
    );
    let f = evaluate_frame(&p, 30).unwrap();
    let sh = get(&f, "sh");
    near(sh.transform.e, 800.0, "depth 0 static x");
    near(sh.transform.a, 1.0, "depth 0 no zoom");
}

#[test]
fn shared_element_without_camera_matches_plain_track() {
    let track = json!([
        { "scene": "s", "at": 0.0, "state": { "x": 100, "y": 200 } },
        { "scene": "s", "at": 2.0, "state": { "x": 300, "y": 600, "scale": 2.0 } }
    ]);
    let p = project_with(
        vec![scene_json("s", 0.0, 4.0, vec![], None)],
        shared_dot(track, None),
    );
    let f = evaluate_frame(&p, 30).unwrap();
    let sh = get(&f, "sh");
    near(sh.transform.e, 200.0, "x");
    near(sh.transform.f, 400.0, "y");
    near(sh.transform.a, 1.5, "scale");
}

#[test]
fn camera_interpolation_between_scenes_is_continuous() {
    // Scene A pushes 1 -> 1.2; scene B starts at zoom 1.0 and pushes to 1.1.
    // The shared element has Free keys in both scenes, so its blended
    // position moves smoothly even though the cameras disagree at the seam.
    let a = scene_json("a", 0.0, 2.0, vec![], Some(camera(vec![push(1.0, 1.2)])));
    let b = scene_json(
        "b",
        2.0,
        2.0,
        vec![],
        Some(camera(vec![cam_motion(
            0.0,
            2.0,
            json!({ "op": "push", "from": 1.0, "to": 1.1 }),
        )])),
    );
    let track = json!([
        { "scene": "a", "at": 0.0, "state": { "x": 740, "y": 1160 } },
        { "scene": "b", "at": 1.0, "easing": "linear", "state": { "x": 720, "y": 1140 } }
    ]);
    let p = project_with(vec![a, b], shared_dot(track, None));
    let mut prev: Option<(f32, f32)> = None;
    for frame in 0..=90 {
        let f = evaluate_frame(&p, frame).unwrap();
        let sh = get(&f, "sh");
        let pos = (sh.transform.e, sh.transform.f);
        if let Some(q) = prev {
            let step = ((pos.0 - q.0).powi(2) + (pos.1 - q.1).powi(2)).sqrt();
            assert!(
                step <= 2.0,
                "jump of {step}px at frame {frame}: {q:?} -> {pos:?}"
            );
        }
        prev = Some(pos);
    }
}

#[test]
fn camera_is_deterministic() {
    let p = project(
        vec![dot("a", 800.0, 1200.0, Some(1.3))],
        Some(camera(vec![push(1.0, 1.7), track([50.0, 10.0])])),
    );
    for frame in [0, 13, 45, 77] {
        assert_eq!(
            evaluate_frame(&p, frame).unwrap(),
            evaluate_frame(&p, frame).unwrap()
        );
    }
}
