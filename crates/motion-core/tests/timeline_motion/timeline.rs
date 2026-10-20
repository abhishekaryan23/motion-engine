//! Integration tests for the timeline evaluator (`evaluate_frame`).
//!
//! Projects are built from JSON via `MotionProject::from_json`. Canvas is
//! 1080x1920 @ 30fps unless stated otherwise. Layers use anchor 0 and no
//! rotation where possible so transform assertions stay simple.

use motion_core::scene::{ClipInset, Direction, MotionProject, RevealMode};
use motion_core::timeline::{
    self, clip_reveal_offset, evaluate_frame, mask_inset, ResolvedFrame, ResolvedLayer,
};
use motion_core::TimelineError;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn project_fps(fps: u32, scenes: Value, shared: Value) -> MotionProject {
    let v = json!({
        "version": "0.2",
        "project": { "name": "test" },
        "canvas": { "width": 1080, "height": 1920, "fps": fps, "background": "#000000" },
        "scenes": scenes,
        "shared": shared,
    });
    MotionProject::from_json(&v.to_string()).expect("test project must parse")
}

fn project(scenes: Value) -> MotionProject {
    project_fps(30, scenes, json!([]))
}

fn project_shared(scenes: Value, shared: Value) -> MotionProject {
    project_fps(30, scenes, shared)
}

fn rect(id: &str, x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "id": id, "type": "rectangle", "x": x, "y": y, "width": w, "height": h, "fill": "#FFFFFF" })
}

fn scene(id: &str, start: f64, dur: f64, layers: Vec<Value>, motions: Vec<Value>) -> Value {
    json!({ "id": id, "start_seconds": start, "duration_seconds": dur, "layers": layers, "motions": motions })
}

/// Linear-eased motion; `op` supplies the `op` tag and its fields.
fn motion(target: &str, start: f64, dur: f64, op: Value) -> Value {
    let mut m = json!({ "target": target, "start": start, "duration": dur, "easing": "linear" });
    for (k, v) in op.as_object().expect("op must be an object") {
        m[k] = v.clone();
    }
    m
}

fn find_in<'a, 'b>(layers: &'b [ResolvedLayer<'a>], id: &str) -> Option<&'b ResolvedLayer<'a>> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let Some(found) = find_in(&l.children, id) {
            return Some(found);
        }
    }
    None
}

fn find<'a, 'b>(frame: &'b ResolvedFrame<'a>, id: &str) -> Option<&'b ResolvedLayer<'a>> {
    find_in(&frame.layers, id)
}

fn layer_at<'a>(p: &'a MotionProject, frame: u32, id: &str) -> Option<ResolvedLayer<'a>> {
    let f = evaluate_frame(p, frame).expect("evaluation must succeed");
    find(&f, id).cloned()
}

fn must_layer<'a>(p: &'a MotionProject, frame: u32, id: &str) -> ResolvedLayer<'a> {
    layer_at(p, frame, id).unwrap_or_else(|| panic!("layer '{id}' missing at frame {frame}"))
}

fn top_ids(p: &MotionProject, frame: u32) -> Vec<String> {
    evaluate_frame(p, frame)
        .unwrap()
        .layers
        .iter()
        .map(|l| l.id.to_string())
        .collect()
}

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

macro_rules! assert_approx {
    ($a:expr, $b:expr) => {
        assert!(approx($a, $b), "{} = {} but expected {}", stringify!($a), $a, $b)
    };
    ($a:expr, $b:expr, $($msg:tt)+) => {
        assert!(approx($a, $b), "{} = {} but expected {}: {}", stringify!($a), $a, $b, format!($($msg)+))
    };
}

// ---------------------------------------------------------------------------
// Frame time and scene activity
// ---------------------------------------------------------------------------

#[test]
fn frame_time_is_frame_over_fps() {
    assert_eq!(timeline::frame_time(30, 45), 1.5);
    assert_eq!(timeline::frame_time(30, 0), 0.0);
    assert_eq!(timeline::frame_time(60, 90), 1.5);
    assert_eq!(timeline::frame_time(24, 48), 2.0);
}

#[test]
fn resolved_frame_reports_time_and_canvas() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 10.0, 10.0)],
        vec![]
    )]));
    let f = evaluate_frame(&p, 45).unwrap();
    assert_eq!(f.frame, 45);
    assert_eq!(f.time_seconds, 1.5);
    assert_eq!((f.width, f.height), (1080, 1920));
}

#[test]
fn scene_window_is_start_inclusive_end_exclusive() {
    let p = project(json!([scene(
        "s",
        1.0,
        2.0,
        vec![rect("a", 0.0, 0.0, 10.0, 10.0)],
        vec![]
    )]));
    for (frame, active) in [
        (0, false),
        (29, false),
        (30, true),
        (89, true),
        (90, false),
        (120, false),
    ] {
        let f = evaluate_frame(&p, frame).unwrap();
        assert_eq!(
            f.active_scenes.contains(&"s"),
            active,
            "scene activity at frame {frame}"
        );
        assert_eq!(
            find(&f, "a").is_some(),
            active,
            "layer presence at frame {frame}"
        );
    }
}

#[test]
fn overlapping_scenes_are_both_active() {
    let p = project(json!([
        scene("A", 0.0, 3.4, vec![rect("a", 0.0, 0.0, 10.0, 10.0)], vec![]),
        scene("B", 3.0, 3.2, vec![rect("b", 0.0, 0.0, 10.0, 10.0)], vec![]),
    ]));
    // t = 3.2
    let f = evaluate_frame(&p, 96).unwrap();
    assert_eq!(f.active_scenes, vec!["A", "B"]);
    assert!(find(&f, "a").is_some());
    assert!(find(&f, "b").is_some());
    assert_eq!(find(&f, "a").unwrap().scene, Some("A"));
    assert_eq!(find(&f, "b").unwrap().scene, Some("B"));

    // Before B starts only A; after A ends only B.
    let early = evaluate_frame(&p, 60).unwrap();
    assert_eq!(early.active_scenes, vec!["A"]);
    let late = evaluate_frame(&p, 120).unwrap();
    assert_eq!(late.active_scenes, vec!["B"]);
}

#[test]
fn invisible_layers_are_omitted() {
    let mut hidden = rect("hidden", 0.0, 0.0, 10.0, 10.0);
    hidden["visible"] = json!(false);
    let p = project(json!([scene(
        "s",
        0.0,
        2.0,
        vec![hidden, rect("shown", 0.0, 0.0, 10.0, 10.0)],
        vec![]
    )]));
    assert_eq!(top_ids(&p, 0), vec!["shown"]);
}

#[test]
fn zero_fps_is_an_error() {
    let p = project_fps(0, json!([scene("s", 0.0, 1.0, vec![], vec![])]), json!([]));
    assert_eq!(evaluate_frame(&p, 0).unwrap_err(), TimelineError::ZeroFps);
    assert_eq!(evaluate_frame(&p, 10).unwrap_err(), TimelineError::ZeroFps);
}

// ---------------------------------------------------------------------------
// Draw order
// ---------------------------------------------------------------------------

#[test]
fn lower_z_index_draws_first_regardless_of_scene_order() {
    let mut hi = rect("hi", 0.0, 0.0, 10.0, 10.0);
    hi["z_index"] = json!(5);
    let mut lo = rect("lo", 0.0, 0.0, 10.0, 10.0);
    lo["z_index"] = json!(1);
    let p = project(json!([
        scene("first", 0.0, 4.0, vec![hi], vec![]),
        scene("second", 0.0, 4.0, vec![lo], vec![]),
    ]));
    assert_eq!(top_ids(&p, 0), vec!["lo", "hi"]);
}

#[test]
fn lower_z_index_draws_first_within_a_scene() {
    let mut a = rect("a", 0.0, 0.0, 10.0, 10.0);
    a["z_index"] = json!(3);
    let mut b = rect("b", 0.0, 0.0, 10.0, 10.0);
    b["z_index"] = json!(-2);
    let c = rect("c", 0.0, 0.0, 10.0, 10.0);
    let p = project(json!([scene("s", 0.0, 2.0, vec![a, b, c], vec![])]));
    assert_eq!(top_ids(&p, 0), vec!["b", "c", "a"]);
}

#[test]
fn equal_z_orders_by_scene_then_layer_order() {
    let p = project(json!([
        scene(
            "A",
            0.0,
            4.0,
            vec![
                rect("a1", 0.0, 0.0, 10.0, 10.0),
                rect("a2", 0.0, 0.0, 10.0, 10.0)
            ],
            vec![]
        ),
        scene(
            "B",
            0.0,
            4.0,
            vec![
                rect("b1", 0.0, 0.0, 10.0, 10.0),
                rect("b2", 0.0, 0.0, 10.0, 10.0)
            ],
            vec![]
        ),
    ]));
    assert_eq!(top_ids(&p, 0), vec!["a1", "a2", "b1", "b2"]);
}

#[test]
fn shared_element_draws_after_scene_layers_at_equal_z() {
    let p = project_shared(
        json!([scene(
            "s",
            0.0,
            4.0,
            vec![rect("bg", 0.0, 0.0, 10.0, 10.0)],
            vec![]
        )]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 0.0, 0.0, 10.0, 10.0),
            "track": [
                { "scene": "s", "at": 0.0, "state": {} },
                { "scene": "s", "at": 2.0, "state": {} }
            ]
        }]),
    );
    assert_eq!(top_ids(&p, 30), vec!["bg", "hero"]);
}

#[test]
fn group_children_are_sorted_by_z_then_order() {
    let mut c1 = rect("c1", 0.0, 0.0, 10.0, 10.0);
    c1["z_index"] = json!(2);
    let c2 = rect("c2", 0.0, 0.0, 10.0, 10.0);
    let c3 = rect("c3", 0.0, 0.0, 10.0, 10.0);
    let group = json!({ "id": "g", "type": "group", "width": 100, "height": 100, "children": [c1, c2, c3] });
    let p = project(json!([scene("s", 0.0, 2.0, vec![group], vec![])]));
    let g = must_layer(&p, 0, "g");
    let ids: Vec<&str> = g.children.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec!["c2", "c3", "c1"]);
}

// ---------------------------------------------------------------------------
// Move
// ---------------------------------------------------------------------------

fn move_project() -> MotionProject {
    // Layer base (100, 200). Move offset (0,0) -> (300, -100) over [1.0, 3.0] (scene-local == absolute).
    project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 100.0, 200.0, 50.0, 50.0)],
        vec![motion(
            "a",
            1.0,
            2.0,
            json!({ "op": "move", "from": [0.0, 0.0], "to": [300.0, -100.0] })
        )]
    )]))
}

#[test]
fn move_holds_from_before_start() {
    let p = move_project();
    for frame in [0, 15, 29] {
        let l = must_layer(&p, frame, "a");
        assert_approx!(l.transform.e, 100.0, "frame {frame}");
        assert_approx!(l.transform.f, 200.0, "frame {frame}");
    }
}

#[test]
fn move_offset_is_added_to_base_position_mid_and_end() {
    let p = move_project();
    // Start (t = 1.0).
    let l = must_layer(&p, 30, "a");
    assert_approx!(l.transform.e, 100.0);
    assert_approx!(l.transform.f, 200.0);
    // Mid (t = 2.0).
    let l = must_layer(&p, 60, "a");
    assert_approx!(l.transform.e, 250.0);
    assert_approx!(l.transform.f, 150.0);
    // End (t = 3.0).
    let l = must_layer(&p, 90, "a");
    assert_approx!(l.transform.e, 400.0);
    assert_approx!(l.transform.f, 100.0);
}

#[test]
fn move_holds_to_after_end() {
    let p = move_project();
    for frame in [91, 120, 149] {
        let l = must_layer(&p, frame, "a");
        assert_approx!(l.transform.e, 400.0, "frame {frame}");
        assert_approx!(l.transform.f, 100.0, "frame {frame}");
    }
}

#[test]
fn move_before_start_holds_nonzero_from() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 100.0, 200.0, 50.0, 50.0)],
        vec![motion(
            "a",
            1.0,
            1.0,
            json!({ "op": "move", "from": [40.0, 60.0], "to": [0.0, 0.0] })
        )]
    )]));
    let l = must_layer(&p, 0, "a");
    assert_approx!(l.transform.e, 140.0);
    assert_approx!(l.transform.f, 260.0);
}

#[test]
fn motion_start_is_relative_to_scene_start() {
    // Scene starts at 2.0s, motion at +1.0s -> begins at absolute 3.0s (frame 90), ends at 4.0s (frame 120).
    let p = project(json!([scene(
        "s",
        2.0,
        4.0,
        vec![rect("a", 0.0, 0.0, 10.0, 10.0)],
        vec![motion(
            "a",
            1.0,
            1.0,
            json!({ "op": "move", "from": [0.0, 0.0], "to": [100.0, 0.0] })
        )]
    )]));
    assert_approx!(must_layer(&p, 60, "a").transform.e, 0.0);
    assert_approx!(must_layer(&p, 90, "a").transform.e, 0.0);
    assert_approx!(must_layer(&p, 105, "a").transform.e, 50.0);
    assert_approx!(must_layer(&p, 120, "a").transform.e, 100.0);
    assert_approx!(must_layer(&p, 150, "a").transform.e, 100.0);
}

#[test]
fn sequential_motions_on_one_channel_hold_between_them() {
    let p = project(json!([scene(
        "s",
        0.0,
        6.0,
        vec![rect("a", 0.0, 0.0, 10.0, 10.0)],
        vec![
            motion(
                "a",
                0.0,
                1.0,
                json!({ "op": "move", "from": [0.0, 0.0], "to": [100.0, 0.0] })
            ),
            motion(
                "a",
                2.0,
                1.0,
                json!({ "op": "move", "from": [100.0, 0.0], "to": [300.0, 0.0] })
            ),
        ]
    )]));
    assert_approx!(must_layer(&p, 15, "a").transform.e, 50.0);
    // Gap between motions holds the first motion's `to`.
    assert_approx!(must_layer(&p, 45, "a").transform.e, 100.0);
    assert_approx!(must_layer(&p, 60, "a").transform.e, 100.0);
    assert_approx!(must_layer(&p, 75, "a").transform.e, 200.0);
    assert_approx!(must_layer(&p, 100, "a").transform.e, 300.0);
}

#[test]
fn zero_duration_motion_jumps_at_its_start() {
    let p = project(json!([scene(
        "s",
        0.0,
        4.0,
        vec![rect("a", 0.0, 0.0, 10.0, 10.0)],
        vec![motion(
            "a",
            1.0,
            0.0,
            json!({ "op": "move", "from": [0.0, 0.0], "to": [80.0, 0.0] })
        )]
    )]));
    assert_approx!(must_layer(&p, 29, "a").transform.e, 0.0);
    assert_approx!(must_layer(&p, 30, "a").transform.e, 80.0);
    assert_approx!(must_layer(&p, 60, "a").transform.e, 80.0);
}

#[test]
fn move_applies_easing() {
    let mut m = motion(
        "a",
        0.0,
        2.0,
        json!({ "op": "move", "from": [0.0, 0.0], "to": [80.0, 0.0] }),
    );
    m["easing"] = json!("in_cubic");
    let p = project(json!([scene(
        "s",
        0.0,
        4.0,
        vec![rect("a", 0.0, 0.0, 10.0, 10.0)],
        vec![m]
    )]));
    // t = 1.0 -> p = 0.5 -> in_cubic = 0.125 -> 10
    assert_approx!(must_layer(&p, 30, "a").transform.e, 10.0);
}

// ---------------------------------------------------------------------------
// Scale / Rotate / Fade
// ---------------------------------------------------------------------------

#[test]
fn scale_both_axes_scales_a_and_d() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "scale", "from": 1.0, "to": 3.0, "axis": "both" })
        )]
    )]));
    let mid = must_layer(&p, 30, "a");
    assert_approx!(mid.transform.a, 2.0);
    assert_approx!(mid.transform.d, 2.0);
    let end = must_layer(&p, 60, "a");
    assert_approx!(end.transform.a, 3.0);
    assert_approx!(end.transform.d, 3.0);
}

#[test]
fn scale_axis_x_only_changes_a() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "scale", "from": 1.0, "to": 3.0, "axis": "x" })
        )]
    )]));
    let mid = must_layer(&p, 30, "a");
    assert_approx!(mid.transform.a, 2.0);
    assert_approx!(mid.transform.d, 1.0);
}

#[test]
fn scale_axis_y_only_changes_d_and_multiplies_base_scale() {
    let mut layer = rect("a", 0.0, 0.0, 50.0, 50.0);
    layer["scale_x"] = json!(2.0);
    layer["scale_y"] = json!(2.0);
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![layer],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "scale", "from": 1.0, "to": 3.0, "axis": "y" })
        )]
    )]));
    let mid = must_layer(&p, 30, "a");
    assert_approx!(mid.transform.a, 2.0);
    assert_approx!(mid.transform.d, 4.0);
}

#[test]
fn scale_holds_from_before_start() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            1.0,
            1.0,
            json!({ "op": "scale", "from": 0.5, "to": 1.0 })
        )]
    )]));
    assert_approx!(must_layer(&p, 0, "a").transform.a, 0.5);
}

#[test]
fn rotate_zero_to_ninety_reaches_quarter_turn() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "rotate", "from": 0.0, "to": 90.0 })
        )]
    )]));
    let start = must_layer(&p, 0, "a");
    assert_approx!(start.transform.a, 1.0);
    assert_approx!(start.transform.b, 0.0);
    let end = must_layer(&p, 60, "a");
    assert_approx!(end.transform.b, 1.0);
    assert_approx!(end.transform.a, 0.0);
    assert_approx!(end.transform.c, -1.0);
    assert_approx!(end.transform.d, 0.0);
    let mid = must_layer(&p, 30, "a");
    assert_approx!(mid.transform.b, std::f32::consts::FRAC_1_SQRT_2);
}

#[test]
fn rotate_adds_to_base_rotation() {
    let mut layer = rect("a", 0.0, 0.0, 50.0, 50.0);
    layer["rotation_degrees"] = json!(90.0);
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![layer],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "rotate", "from": 0.0, "to": 90.0 })
        )]
    )]));
    let end = must_layer(&p, 60, "a");
    // 90 + 90 = 180 degrees.
    assert_approx!(end.transform.a, -1.0);
    assert_approx!(end.transform.b, 0.0);
}

#[test]
fn transform_composes_translate_rotate_scale_anchor() {
    // anchor at center of a 100x50 box, positioned at (500, 500), scaled 2x, rotated 90 degrees.
    let mut layer = rect("a", 500.0, 500.0, 100.0, 50.0);
    layer["anchor_x"] = json!(0.5);
    layer["anchor_y"] = json!(0.5);
    layer["scale_x"] = json!(2.0);
    layer["scale_y"] = json!(2.0);
    layer["rotation_degrees"] = json!(90.0);
    let p = project(json!([scene("s", 0.0, 2.0, vec![layer], vec![])]));
    let l = must_layer(&p, 0, "a");
    // The anchor point (50, 25) in local space must land on (500, 500).
    let (ax, ay) = l.transform.apply(50.0, 25.0);
    assert_approx!(ax, 500.0);
    assert_approx!(ay, 500.0);
    // Local (100, 25) is 50 px right of anchor; scaled 2x -> 100; rotated 90deg -> straight down (+y).
    let (px, py) = l.transform.apply(100.0, 25.0);
    assert_approx!(px, 500.0);
    assert_approx!(py, 600.0);
}

#[test]
fn fade_zero_to_one_is_half_at_midpoint() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "fade", "from": 0.0, "to": 1.0 })
        )]
    )]));
    assert_approx!(must_layer(&p, 30, "a").opacity, 0.5);
    assert_approx!(must_layer(&p, 60, "a").opacity, 1.0);
    assert_approx!(must_layer(&p, 100, "a").opacity, 1.0);
    // Fully transparent at t=0 -> omitted.
    assert!(layer_at(&p, 0, "a").is_none());
}

#[test]
fn fade_multiplies_base_opacity() {
    let mut layer = rect("a", 0.0, 0.0, 50.0, 50.0);
    layer["opacity"] = json!(0.5);
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![layer],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "fade", "from": 0.0, "to": 1.0 })
        )]
    )]));
    assert_approx!(must_layer(&p, 30, "a").opacity, 0.25);
    assert_approx!(must_layer(&p, 60, "a").opacity, 0.5);
}

#[test]
fn layer_faded_to_zero_is_omitted_from_frame() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![
            rect("a", 0.0, 0.0, 50.0, 50.0),
            rect("b", 0.0, 0.0, 50.0, 50.0)
        ],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "fade", "from": 1.0, "to": 0.0 })
        )]
    )]));
    assert!(layer_at(&p, 30, "a").is_some());
    assert!(layer_at(&p, 59, "a").is_some());
    assert!(layer_at(&p, 60, "a").is_none());
    assert!(layer_at(&p, 120, "a").is_none());
    assert_eq!(top_ids(&p, 120), vec!["b"]);
    // The scene is still active.
    assert_eq!(evaluate_frame(&p, 120).unwrap().active_scenes, vec!["s"]);
}

#[test]
fn opacity_is_clamped_to_unit_range() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            0.0,
            1.0,
            json!({ "op": "fade", "from": 1.0, "to": 3.0 })
        )]
    )]));
    assert_approx!(must_layer(&p, 30, "a").opacity, 1.0);
}

#[test]
fn zero_scale_layer_is_omitted() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 0.0, 0.0, 50.0, 50.0)],
        vec![motion(
            "a",
            0.0,
            1.0,
            json!({ "op": "scale", "from": 0.0, "to": 1.0 })
        )]
    )]));
    assert!(layer_at(&p, 0, "a").is_none());
    assert!(layer_at(&p, 15, "a").is_some());
}

// ---------------------------------------------------------------------------
// MaskReveal
// ---------------------------------------------------------------------------

fn clip_eq(c: ClipInset, l: f32, t: f32, r: f32, b: f32) -> bool {
    approx(c.left, l) && approx(c.top, t) && approx(c.right, r) && approx(c.bottom, b)
}

#[test]
fn mask_inset_reveal_directions() {
    let c = mask_inset(Direction::Right, RevealMode::Reveal, 0.5);
    assert!(clip_eq(c, 0.0, 0.0, 0.5, 0.0), "{c:?}");
    let c = mask_inset(Direction::Left, RevealMode::Reveal, 0.25);
    assert!(clip_eq(c, 0.75, 0.0, 0.0, 0.0), "{c:?}");
    let c = mask_inset(Direction::Down, RevealMode::Reveal, 0.25);
    assert!(clip_eq(c, 0.0, 0.0, 0.0, 0.75), "{c:?}");
    let c = mask_inset(Direction::Up, RevealMode::Reveal, 0.75);
    assert!(clip_eq(c, 0.0, 0.25, 0.0, 0.0), "{c:?}");
}

#[test]
fn mask_inset_conceal_directions() {
    let c = mask_inset(Direction::Right, RevealMode::Conceal, 0.25);
    assert!(clip_eq(c, 0.25, 0.0, 0.0, 0.0), "{c:?}");
    let c = mask_inset(Direction::Left, RevealMode::Conceal, 0.25);
    assert!(clip_eq(c, 0.0, 0.0, 0.25, 0.0), "{c:?}");
    let c = mask_inset(Direction::Down, RevealMode::Conceal, 0.5);
    assert!(clip_eq(c, 0.0, 0.5, 0.0, 0.0), "{c:?}");
    let c = mask_inset(Direction::Up, RevealMode::Conceal, 0.5);
    assert!(clip_eq(c, 0.0, 0.0, 0.0, 0.5), "{c:?}");
}

#[test]
fn mask_inset_endpoints_and_clamping() {
    for dir in [
        Direction::Left,
        Direction::Right,
        Direction::Up,
        Direction::Down,
    ] {
        // Reveal: fully hidden at 0, nothing clipped at 1.
        let h0 = mask_inset(dir, RevealMode::Reveal, 0.0);
        assert!(
            (h0.left + h0.right + h0.top + h0.bottom - 1.0).abs() < 1e-6,
            "{dir:?}: {h0:?}"
        );
        assert_eq!(
            mask_inset(dir, RevealMode::Reveal, 1.0),
            ClipInset::default()
        );
        // Conceal: nothing clipped at 0, fully hidden at 1.
        assert_eq!(
            mask_inset(dir, RevealMode::Conceal, 0.0),
            ClipInset::default()
        );
        let c1 = mask_inset(dir, RevealMode::Conceal, 1.0);
        assert!((c1.left + c1.right + c1.top + c1.bottom - 1.0).abs() < 1e-6);
        // Out-of-range progress clamps.
        assert_eq!(
            mask_inset(dir, RevealMode::Reveal, 2.0),
            mask_inset(dir, RevealMode::Reveal, 1.0)
        );
        assert_eq!(
            mask_inset(dir, RevealMode::Reveal, -1.0),
            mask_inset(dir, RevealMode::Reveal, 0.0)
        );
    }
}

fn mask_project(direction: &str, mode: &str) -> MotionProject {
    project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 10.0, 10.0, 100.0, 100.0)],
        vec![motion(
            "a",
            1.0,
            2.0,
            json!({ "op": "mask_reveal", "direction": direction, "mode": mode })
        )]
    )]))
}

#[test]
fn mask_reveal_right_at_midpoint_clips_half_from_right() {
    let p = mask_project("right", "reveal");
    // t = 2.0 -> p = 0.5
    let c = must_layer(&p, 60, "a")
        .clip
        .expect("clip present mid reveal");
    assert!(clip_eq(c, 0.0, 0.0, 0.5, 0.0), "{c:?}");
}

#[test]
fn mask_reveal_other_directions_at_midpoint() {
    let c = must_layer(&mask_project("left", "reveal"), 60, "a")
        .clip
        .unwrap();
    assert!(clip_eq(c, 0.5, 0.0, 0.0, 0.0), "left: {c:?}");
    let c = must_layer(&mask_project("up", "reveal"), 60, "a")
        .clip
        .unwrap();
    assert!(clip_eq(c, 0.0, 0.5, 0.0, 0.0), "up: {c:?}");
    let c = must_layer(&mask_project("down", "reveal"), 60, "a")
        .clip
        .unwrap();
    assert!(clip_eq(c, 0.0, 0.0, 0.0, 0.5), "down: {c:?}");
}

#[test]
fn mask_reveal_does_not_move_content() {
    let p = mask_project("right", "reveal");
    let l = must_layer(&p, 60, "a");
    assert_eq!(l.transform, l.content_transform);
    assert_approx!(l.transform.e, 10.0);
}

#[test]
fn mask_reveal_before_start_is_fully_clipped_and_omitted() {
    let p = mask_project("right", "reveal");
    assert!(layer_at(&p, 0, "a").is_none());
    assert!(layer_at(&p, 29, "a").is_none());
    assert!(layer_at(&p, 30, "a").is_none(), "p = 0 is fully hidden");
    assert!(layer_at(&p, 31, "a").is_some());
}

#[test]
fn mask_reveal_after_end_shows_everything() {
    let p = mask_project("right", "reveal");
    let c = must_layer(&p, 120, "a")
        .clip
        .expect("clip stays Some after reveal");
    assert_eq!(c, ClipInset::default());
}

#[test]
fn mask_conceal_hides_progressively() {
    let p = mask_project("right", "conceal");
    // Before start: fully visible.
    let c = must_layer(&p, 0, "a").clip.unwrap();
    assert_eq!(c, ClipInset::default());
    // Mid: right-travelling edge has eaten the left half.
    let c = must_layer(&p, 60, "a").clip.unwrap();
    assert!(clip_eq(c, 0.5, 0.0, 0.0, 0.0), "{c:?}");
    // End: fully hidden -> omitted.
    assert!(layer_at(&p, 90, "a").is_none());
    assert!(layer_at(&p, 150, "a").is_none());
}

#[test]
fn mask_reveal_combines_with_static_clip_by_max() {
    let mut layer = rect("a", 0.0, 0.0, 100.0, 100.0);
    layer["clip"] = json!({ "left": 0.2, "right": 0.1 });
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![layer],
        vec![motion(
            "a",
            0.0,
            2.0,
            json!({ "op": "mask_reveal", "direction": "right" })
        )]
    )]));
    // Mid: reveal contributes right = 0.5, static contributes left = 0.2, right = 0.1 -> max.
    let c = must_layer(&p, 30, "a").clip.unwrap();
    assert!(clip_eq(c, 0.2, 0.0, 0.5, 0.0), "{c:?}");
    // End: reveal contributes 0 -> static clip remains.
    let c = must_layer(&p, 60, "a").clip.unwrap();
    assert!(clip_eq(c, 0.2, 0.0, 0.1, 0.0), "{c:?}");
}

#[test]
fn static_clip_without_motion_is_passed_through() {
    let mut layer = rect("a", 0.0, 0.0, 100.0, 100.0);
    layer["clip"] = json!({ "top": 0.3 });
    let p = project(json!([scene("s", 0.0, 2.0, vec![layer], vec![])]));
    let c = must_layer(&p, 0, "a").clip.unwrap();
    assert!(clip_eq(c, 0.0, 0.3, 0.0, 0.0), "{c:?}");
    // And layers without any clip report none.
    let p = project(json!([scene(
        "s",
        0.0,
        2.0,
        vec![rect("b", 0.0, 0.0, 10.0, 10.0)],
        vec![]
    )]));
    assert!(must_layer(&p, 0, "b").clip.is_none());
}

// ---------------------------------------------------------------------------
// ClipReveal
// ---------------------------------------------------------------------------

#[test]
fn clip_reveal_offset_up_reveal_endpoints() {
    let (x, y) = clip_reveal_offset(Direction::Up, RevealMode::Reveal, 0.0, 200.0, 100.0);
    assert_approx!(x, 0.0);
    assert_approx!(y, 100.0);
    let (x, y) = clip_reveal_offset(Direction::Up, RevealMode::Reveal, 1.0, 200.0, 100.0);
    assert_approx!(x, 0.0);
    assert_approx!(y, 0.0);
    let (x, y) = clip_reveal_offset(Direction::Up, RevealMode::Reveal, 0.5, 200.0, 100.0);
    assert_approx!(x, 0.0);
    assert_approx!(y, 50.0);
}

#[test]
fn clip_reveal_offset_other_directions_start_opposite_to_travel() {
    let (x, y) = clip_reveal_offset(Direction::Down, RevealMode::Reveal, 0.0, 200.0, 100.0);
    assert_approx!(x, 0.0);
    assert_approx!(y, -100.0);
    let (x, y) = clip_reveal_offset(Direction::Right, RevealMode::Reveal, 0.0, 200.0, 100.0);
    assert_approx!(x, -200.0);
    assert_approx!(y, 0.0);
    let (x, y) = clip_reveal_offset(Direction::Left, RevealMode::Reveal, 0.0, 200.0, 100.0);
    assert_approx!(x, 200.0);
    assert_approx!(y, 0.0);
}

#[test]
fn clip_reveal_offset_conceal_leaves_along_travel() {
    let (x, y) = clip_reveal_offset(Direction::Up, RevealMode::Conceal, 0.0, 200.0, 100.0);
    assert_approx!(x, 0.0);
    assert_approx!(y, 0.0);
    let (_, y) = clip_reveal_offset(Direction::Up, RevealMode::Conceal, 1.0, 200.0, 100.0);
    assert_approx!(y, -100.0);
    let (x, _) = clip_reveal_offset(Direction::Right, RevealMode::Conceal, 1.0, 200.0, 100.0);
    assert_approx!(x, 200.0);
}

#[test]
fn clip_reveal_offset_clamps_progress() {
    assert_eq!(
        clip_reveal_offset(Direction::Up, RevealMode::Reveal, -5.0, 200.0, 100.0),
        clip_reveal_offset(Direction::Up, RevealMode::Reveal, 0.0, 200.0, 100.0)
    );
    assert_eq!(
        clip_reveal_offset(Direction::Up, RevealMode::Reveal, 5.0, 200.0, 100.0),
        clip_reveal_offset(Direction::Up, RevealMode::Reveal, 1.0, 200.0, 100.0)
    );
}

fn clip_reveal_project() -> MotionProject {
    project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("a", 50.0, 60.0, 200.0, 100.0)],
        vec![motion(
            "a",
            1.0,
            2.0,
            json!({ "op": "clip_reveal", "direction": "up" })
        )]
    )]))
}

#[test]
fn clip_reveal_resolved_layer_has_clip_and_offset_content() {
    let p = clip_reveal_project();

    // Before start (p = 0): layer present, clip Some, content fully displaced down by h.
    let l = must_layer(&p, 0, "a");
    assert_eq!(l.clip, Some(ClipInset::default()));
    assert_approx!(l.transform.f, 60.0);
    assert_approx!(l.content_transform.f, 160.0);
    assert_approx!(l.content_transform.e, 50.0);

    // Mid (p = 0.5): content displaced by h/2, box transform unchanged.
    let l = must_layer(&p, 60, "a");
    assert!(l.clip.is_some());
    assert_ne!(l.content_transform, l.transform);
    assert_approx!(l.transform.f, 60.0);
    assert_approx!(l.content_transform.f, 110.0);

    // End: content at rest.
    let l = must_layer(&p, 90, "a");
    assert!(l.clip.is_some());
    assert_approx!(l.content_transform.f, l.transform.f);
    assert_approx!(l.content_transform.e, l.transform.e);
}

#[test]
fn clip_reveal_does_not_change_box_size() {
    let p = clip_reveal_project();
    let l = must_layer(&p, 60, "a");
    assert_approx!(l.width, 200.0);
    assert_approx!(l.height, 100.0);
}

// ---------------------------------------------------------------------------
// AccentExpand
// ---------------------------------------------------------------------------

#[test]
fn accent_expand_interpolates_geometry() {
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![rect("bar", 0.0, 0.0, 100.0, 10.0)],
        vec![motion(
            "bar",
            1.0,
            2.0,
            json!({ "op": "accent_expand", "to": { "x": 20.0, "y": 30.0, "width": 300.0, "height": 50.0 } })
        )]
    )]));
    let before = must_layer(&p, 0, "bar");
    assert_approx!(before.width, 100.0);
    assert_approx!(before.height, 10.0);
    assert_approx!(before.transform.e, 0.0);

    let mid = must_layer(&p, 60, "bar");
    assert_approx!(mid.width, 200.0);
    assert_approx!(mid.height, 30.0);
    assert_approx!(mid.transform.e, 10.0);
    assert_approx!(mid.transform.f, 15.0);

    let end = must_layer(&p, 90, "bar");
    assert_approx!(end.width, 300.0);
    assert_approx!(end.height, 50.0);
    assert_approx!(end.transform.e, 20.0);
    assert_approx!(end.transform.f, 30.0);

    let after = must_layer(&p, 140, "bar");
    assert_approx!(after.width, 300.0);
    assert_approx!(after.height, 50.0);
}

#[test]
fn accent_expand_chains_from_previous_end_state() {
    let p = project(json!([scene(
        "s",
        0.0,
        8.0,
        vec![rect("bar", 0.0, 0.0, 100.0, 10.0)],
        vec![
            motion(
                "bar",
                0.0,
                1.0,
                json!({ "op": "accent_expand", "to": { "x": 0.0, "y": 0.0, "width": 200.0, "height": 10.0 } })
            ),
            motion(
                "bar",
                2.0,
                2.0,
                json!({ "op": "accent_expand", "to": { "x": 0.0, "y": 0.0, "width": 600.0, "height": 10.0 } })
            ),
        ]
    )]));
    // Between the two: holds the first `to`.
    assert_approx!(must_layer(&p, 45, "bar").width, 200.0);
    // Mid of the second: halfway between 200 and 600.
    assert_approx!(must_layer(&p, 90, "bar").width, 400.0);
    assert_approx!(must_layer(&p, 150, "bar").width, 600.0);
}

#[test]
fn accent_expand_width_change_moves_anchor_offset() {
    // Anchor at center: growing the box keeps the anchor at (x, y).
    let mut bar = rect("bar", 500.0, 500.0, 100.0, 10.0);
    bar["anchor_x"] = json!(0.5);
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![bar],
        vec![motion(
            "bar",
            0.0,
            1.0,
            json!({ "op": "accent_expand", "to": { "x": 500.0, "y": 500.0, "width": 300.0, "height": 10.0 } })
        )]
    )]));
    let end = must_layer(&p, 30, "bar");
    let (ax, ay) = end.transform.apply(150.0, 0.0);
    assert_approx!(ax, 500.0);
    assert_approx!(ay, 500.0);
}

// ---------------------------------------------------------------------------
// Group
// ---------------------------------------------------------------------------

#[test]
fn group_child_transform_composes_with_parent() {
    let child = rect("child", 10.0, 20.0, 30.0, 30.0);
    let group = json!({ "id": "g", "type": "group", "x": 100, "y": 50, "width": 200, "height": 200, "children": [child] });
    let p = project(json!([scene("s", 0.0, 2.0, vec![group], vec![])]));
    let f = evaluate_frame(&p, 0).unwrap();
    // Children are nested, not top-level.
    assert_eq!(f.layers.len(), 1);
    let g = &f.layers[0];
    assert_eq!(g.id, "g");
    assert_eq!(g.children.len(), 1);
    let c = &g.children[0];
    assert_eq!(c.id, "child");
    assert_approx!(c.transform.e, 110.0);
    assert_approx!(c.transform.f, 70.0);
    assert_eq!(c.scene, Some("s"));
}

#[test]
fn group_child_follows_parent_motion() {
    let child = rect("child", 10.0, 0.0, 30.0, 30.0);
    let group = json!({ "id": "g", "type": "group", "x": 100, "y": 0, "width": 200, "height": 200, "children": [child] });
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![group],
        vec![motion(
            "g",
            0.0,
            2.0,
            json!({ "op": "move", "from": [0.0, 0.0], "to": [200.0, 0.0] })
        )]
    )]));
    assert_approx!(must_layer(&p, 0, "child").transform.e, 110.0);
    assert_approx!(must_layer(&p, 30, "child").transform.e, 210.0);
    assert_approx!(must_layer(&p, 60, "child").transform.e, 310.0);
}

#[test]
fn group_scale_scales_child_position() {
    let child = rect("child", 10.0, 0.0, 30.0, 30.0);
    let group = json!({
        "id": "g", "type": "group", "x": 100, "y": 0, "width": 200, "height": 200,
        "scale_x": 2.0, "scale_y": 2.0, "children": [child]
    });
    let p = project(json!([scene("s", 0.0, 2.0, vec![group], vec![])]));
    let c = must_layer(&p, 0, "child");
    assert_approx!(c.transform.a, 2.0);
    assert_approx!(c.transform.e, 120.0);
}

#[test]
fn group_faded_out_drops_the_whole_subtree() {
    let child = rect("child", 0.0, 0.0, 30.0, 30.0);
    let group = json!({ "id": "g", "type": "group", "width": 200, "height": 200, "opacity": 0.0, "children": [child] });
    let p = project(json!([scene("s", 0.0, 2.0, vec![group], vec![])]));
    assert!(layer_at(&p, 0, "g").is_none());
    assert!(layer_at(&p, 0, "child").is_none());
}

#[test]
fn group_child_motions_target_child_id() {
    let child = rect("child", 0.0, 0.0, 30.0, 30.0);
    let group = json!({ "id": "g", "type": "group", "x": 100, "width": 200, "height": 200, "children": [child] });
    let p = project(json!([scene(
        "s",
        0.0,
        5.0,
        vec![group],
        vec![motion(
            "child",
            0.0,
            2.0,
            json!({ "op": "move", "from": [0.0, 0.0], "to": [40.0, 0.0] })
        )]
    )]));
    assert_approx!(must_layer(&p, 30, "child").transform.e, 120.0);
    assert_approx!(must_layer(&p, 30, "g").transform.e, 100.0);
}

// ---------------------------------------------------------------------------
// Shared elements
// ---------------------------------------------------------------------------

/// Scenes s1 [0,2) and s2 [2,4). Shared "hero" (base x=10, y=20).
/// Keys: s1@0.5 (abs 0.5) {x:100, y:200, opacity:0.4}; s2@0.5 (abs 2.5) {x:300}.
fn shared_project(second_easing: &str) -> MotionProject {
    project_shared(
        json!([
            scene(
                "s1",
                0.0,
                2.0,
                vec![rect("bg1", 0.0, 0.0, 10.0, 10.0)],
                vec![]
            ),
            scene(
                "s2",
                2.0,
                2.0,
                vec![rect("bg2", 0.0, 0.0, 10.0, 10.0)],
                vec![]
            ),
        ]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 10.0, 20.0, 80.0, 80.0),
            "track": [
                { "scene": "s1", "at": 0.5, "role": "start", "easing": "linear",
                  "state": { "x": 100.0, "y": 200.0, "opacity": 0.4 } },
                { "scene": "s2", "at": 0.5, "role": "end", "easing": second_easing,
                  "state": { "x": 300.0 } }
            ]
        }]),
    )
}

#[test]
fn shared_element_absent_before_first_key_and_after_last_key() {
    let p = shared_project("linear");
    for frame in [0, 10, 14] {
        let f = evaluate_frame(&p, frame).unwrap();
        assert!(
            find(&f, "hero").is_none(),
            "frame {frame} should be before the first key"
        );
        assert!(!f.active_shared.contains(&"hero"), "frame {frame}");
    }
    // Last key at absolute 2.5s = frame 75; frame 76 is past it.
    for frame in [76, 90, 119] {
        let f = evaluate_frame(&p, frame).unwrap();
        assert!(
            find(&f, "hero").is_none(),
            "frame {frame} should be after the last key"
        );
        assert!(!f.active_shared.contains(&"hero"), "frame {frame}");
    }
}

#[test]
fn shared_element_equals_key_state_at_key_times() {
    let p = shared_project("linear");
    let first = must_layer(&p, 15, "hero");
    assert_approx!(first.transform.e, 100.0);
    assert_approx!(first.transform.f, 200.0);
    assert_approx!(first.opacity, 0.4);

    let last = must_layer(&p, 75, "hero");
    assert_approx!(last.transform.e, 300.0);
}

#[test]
fn shared_element_interpolates_between_keys() {
    let p = shared_project("linear");
    // t = 1.5 -> halfway between 0.5 and 2.5.
    let mid = must_layer(&p, 45, "hero");
    assert_approx!(mid.transform.e, 200.0);
    // y carried over from first key (200) -> unchanged.
    assert_approx!(mid.transform.f, 200.0);
}

#[test]
fn shared_element_omitted_fields_carry_over_from_previous_key() {
    let p = shared_project("linear");
    // Second key omits y and opacity: both stay at first key's values.
    let last = must_layer(&p, 75, "hero");
    assert_approx!(last.transform.f, 200.0);
    assert_approx!(last.opacity, 0.4);
    let mid = must_layer(&p, 45, "hero");
    assert_approx!(mid.opacity, 0.4);
}

#[test]
fn shared_element_first_key_falls_back_to_base_layer_values() {
    let p = project_shared(
        json!([scene("s1", 0.0, 4.0, vec![], vec![])]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 10.0, 20.0, 80.0, 80.0),
            "track": [
                { "scene": "s1", "at": 0.0, "state": {} },
                { "scene": "s1", "at": 2.0, "state": { "x": 110.0 } }
            ]
        }]),
    );
    let start = must_layer(&p, 0, "hero");
    assert_approx!(start.transform.e, 10.0);
    assert_approx!(start.transform.f, 20.0);
    assert_approx!(start.opacity, 1.0);
    assert_approx!(start.transform.a, 1.0);
    // Halfway: x from 10 to 110.
    assert_approx!(must_layer(&p, 30, "hero").transform.e, 60.0);
}

#[test]
fn shared_element_uses_destination_key_easing() {
    let p = shared_project("in_cubic");
    // Halfway in time -> in_cubic(0.5) = 0.125 -> 100 + 200 * 0.125 = 125.
    assert_approx!(must_layer(&p, 45, "hero").transform.e, 125.0);
}

#[test]
fn shared_element_is_listed_active_and_has_no_scene() {
    let p = shared_project("linear");
    let f = evaluate_frame(&p, 45).unwrap();
    assert_eq!(f.active_shared, vec!["hero"]);
    let hero = find(&f, "hero").unwrap();
    assert_eq!(hero.scene, None);
    // Scene layers still resolve alongside it.
    assert_eq!(find(&f, "bg1").unwrap().scene, Some("s1"));
}

#[test]
fn shared_element_scale_and_rotation_tracks() {
    let p = project_shared(
        json!([scene("s1", 0.0, 4.0, vec![], vec![])]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 0.0, 0.0, 80.0, 80.0),
            "track": [
                { "scene": "s1", "at": 0.0, "state": { "scale": 1.0, "rotation_degrees": 0.0 } },
                { "scene": "s1", "at": 2.0, "state": { "scale": 3.0, "rotation_degrees": 90.0 } }
            ]
        }]),
    );
    let mid = must_layer(&p, 30, "hero");
    // Scale 2, rotation 45 degrees -> a = 2 cos45, b = 2 sin45.
    assert_approx!(mid.transform.a, 2.0 * std::f32::consts::FRAC_1_SQRT_2);
    assert_approx!(mid.transform.b, 2.0 * std::f32::consts::FRAC_1_SQRT_2);
}

#[test]
fn shared_element_retimed_scene_moves_the_track() {
    // Same track keyed to scene-local times; shifting the scene start shifts the element.
    let make = |start: f64| {
        project_shared(
            json!([scene("s1", start, 4.0, vec![], vec![])]),
            json!([{
                "id": "hero",
                "layer": rect("hero", 0.0, 0.0, 10.0, 10.0),
                "track": [
                    { "scene": "s1", "at": 0.0, "state": { "x": 0.0 } },
                    { "scene": "s1", "at": 2.0, "state": { "x": 100.0 } }
                ]
            }]),
        )
    };
    let a = make(0.0);
    let b = make(1.0);
    assert_approx!(must_layer(&a, 30, "hero").transform.e, 50.0);
    assert!(layer_at(&b, 0, "hero").is_none());
    assert_approx!(must_layer(&b, 60, "hero").transform.e, 50.0);
}

#[test]
fn shared_element_unknown_scene_is_an_error() {
    let p = project_shared(
        json!([scene("s1", 0.0, 4.0, vec![], vec![])]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 0.0, 0.0, 10.0, 10.0),
            "track": [ { "scene": "nope", "at": 0.0, "state": {} } ]
        }]),
    );
    match evaluate_frame(&p, 0) {
        Err(TimelineError::UnknownScene { element, scene }) => {
            assert_eq!(element, "hero");
            assert_eq!(scene, "nope");
        }
        other => panic!("expected UnknownScene, got {other:?}"),
    }
}

#[test]
fn shared_element_with_empty_track_is_never_active() {
    let p = project_shared(
        json!([scene("s1", 0.0, 4.0, vec![], vec![])]),
        json!([{ "id": "hero", "layer": rect("hero", 0.0, 0.0, 10.0, 10.0), "track": [] }]),
    );
    for frame in [0, 30, 100] {
        let f = evaluate_frame(&p, frame).unwrap();
        assert!(f.active_shared.is_empty());
        assert!(f.layers.is_empty());
    }
}

#[test]
fn shared_element_faded_out_is_active_but_not_drawn() {
    let p = project_shared(
        json!([scene("s1", 0.0, 4.0, vec![], vec![])]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 0.0, 0.0, 10.0, 10.0),
            "track": [
                { "scene": "s1", "at": 0.0, "state": { "opacity": 0.0 } },
                { "scene": "s1", "at": 2.0, "state": { "opacity": 1.0 } }
            ]
        }]),
    );
    let f = evaluate_frame(&p, 0).unwrap();
    assert_eq!(f.active_shared, vec!["hero"]);
    assert!(find(&f, "hero").is_none());
    assert!(find(&evaluate_frame(&p, 30).unwrap(), "hero").is_some());
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

fn busy_project() -> MotionProject {
    let child = rect("child", 5.0, 5.0, 40.0, 40.0);
    let group = json!({ "id": "g", "type": "group", "x": 200, "y": 200, "width": 100, "height": 100, "children": [child] });
    let mut anchored = rect("anchored", 400.0, 400.0, 100.0, 100.0);
    anchored["anchor_x"] = json!(0.5);
    anchored["anchor_y"] = json!(0.5);
    let mut eased = motion(
        "a",
        0.2,
        1.5,
        json!({ "op": "move", "from": [-100.0, 50.0], "to": [0.0, 0.0] }),
    );
    eased["easing"] = json!("impact_spring");
    let mut eased_fade = motion(
        "a",
        0.0,
        1.0,
        json!({ "op": "fade", "from": 0.0, "to": 1.0 }),
    );
    eased_fade["easing"] = json!("out_quint");
    project_shared(
        json!([
            scene(
                "s1",
                0.0,
                2.5,
                vec![
                    rect("a", 100.0, 100.0, 200.0, 120.0),
                    group,
                    anchored,
                    rect("bar", 0.0, 900.0, 50.0, 8.0)
                ],
                vec![
                    eased,
                    eased_fade,
                    motion(
                        "a",
                        0.0,
                        2.0,
                        json!({ "op": "mask_reveal", "direction": "right" })
                    ),
                    motion(
                        "anchored",
                        0.3,
                        1.0,
                        json!({ "op": "rotate", "from": -20.0, "to": 20.0 })
                    ),
                    motion(
                        "anchored",
                        0.0,
                        2.0,
                        json!({ "op": "scale", "from": 0.8, "to": 1.2 })
                    ),
                    motion(
                        "g",
                        0.5,
                        1.0,
                        json!({ "op": "clip_reveal", "direction": "up" })
                    ),
                    motion(
                        "child",
                        0.0,
                        2.0,
                        json!({ "op": "move", "from": [0.0, 0.0], "to": [30.0, 30.0] })
                    ),
                    motion(
                        "bar",
                        0.1,
                        1.0,
                        json!({ "op": "accent_expand", "to": { "x": 0.0, "y": 900.0, "width": 1080.0, "height": 8.0 } })
                    ),
                ]
            ),
            scene(
                "s2",
                2.0,
                2.0,
                vec![rect("b", 0.0, 0.0, 300.0, 300.0)],
                vec![motion(
                    "b",
                    0.0,
                    1.0,
                    json!({ "op": "fade", "from": 0.0, "to": 1.0 })
                )]
            ),
        ]),
        json!([{
            "id": "hero",
            "layer": rect("hero", 0.0, 0.0, 80.0, 80.0),
            "track": [
                { "scene": "s1", "at": 0.5, "state": { "x": 10.0, "y": 10.0, "scale": 1.0 } },
                { "scene": "s2", "at": 1.0, "easing": "editorial_spring", "state": { "x": 500.0, "scale": 2.0, "rotation_degrees": 15.0 } }
            ]
        }]),
    )
}

#[test]
fn evaluating_same_frame_twice_is_identical() {
    let p = busy_project();
    for frame in [0, 1, 17, 45, 59, 60, 75, 89] {
        let a = evaluate_frame(&p, frame).unwrap();
        let b = evaluate_frame(&p, frame).unwrap();
        assert_eq!(a, b, "frame {frame}");
    }
}

#[test]
fn evaluating_frames_in_sequence_twice_is_identical() {
    let p = busy_project();
    let run = |p: &MotionProject| -> Vec<String> {
        (0..90)
            .map(|f| serde_json::to_string(&evaluate_frame(p, f).unwrap()).unwrap())
            .collect()
    };
    assert_eq!(run(&p), run(&p));
}

#[test]
fn frame_evaluation_is_order_independent() {
    // No accumulated state: evaluating frame 60 first or last yields the same result.
    let p = busy_project();
    let direct = evaluate_frame(&p, 60).unwrap();
    for f in 0..60 {
        let _ = evaluate_frame(&p, f).unwrap();
    }
    assert_eq!(direct, evaluate_frame(&p, 60).unwrap());
    let reversed: Vec<_> = (0..90)
        .rev()
        .map(|f| evaluate_frame(&p, f).unwrap())
        .collect();
    let forward: Vec<_> = (0..90).map(|f| evaluate_frame(&p, f).unwrap()).collect();
    let mut reversed = reversed;
    reversed.reverse();
    assert_eq!(forward, reversed);
}

#[test]
fn evaluation_reparsed_from_json_roundtrip_is_identical() {
    let p = busy_project();
    let q = MotionProject::from_json(&p.to_json_pretty()).unwrap();
    assert_eq!(p, q);
    for frame in [0, 30, 60, 89] {
        let a = serde_json::to_string(&evaluate_frame(&p, frame).unwrap()).unwrap();
        let b = serde_json::to_string(&evaluate_frame(&q, frame).unwrap()).unwrap();
        assert_eq!(a, b, "frame {frame}");
    }
}

// ---------------------------------------------------------------------------
// Count / Trim channels and format_count
// ---------------------------------------------------------------------------

fn counter(id: &str) -> Value {
    json!({ "id": id, "type": "text", "text": "0", "font_role": "display", "font_size": 80,
            "color": "#FFFFFF", "width": 300, "height": 100 })
}

fn polyline_layer(id: &str) -> Value {
    json!({ "id": id, "type": "polyline", "width": 200, "height": 100,
            "points": [[0, 0], [100, 50], [200, 0]],
            "stroke": { "color": "#FFFFFF", "width": 4 } })
}

fn count_op(from: f64, to: f64, decimals: u8, grouping: bool, prefix: &str, suffix: &str) -> Value {
    json!({ "op": "count", "from": from, "to": to, "decimals": decimals,
            "grouping": grouping, "prefix": prefix, "suffix": suffix })
}

#[test]
fn count_replaces_text_with_the_interpolated_number() {
    let p = project(json!([scene(
        "s",
        0.0,
        4.0,
        vec![counter("n")],
        vec![motion(
            "n",
            1.0,
            2.0,
            count_op(0.0, 1000.0, 0, true, "$", "%")
        )]
    )]));
    // Before the motion: holds `from`.
    assert_eq!(must_layer(&p, 0, "n").text.as_deref(), Some("$0%"));
    assert_eq!(must_layer(&p, 30, "n").text.as_deref(), Some("$0%"));
    // Halfway (t = 2.0): linear 500.
    assert_eq!(must_layer(&p, 60, "n").text.as_deref(), Some("$500%"));
    // End and after: holds `to`.
    assert_eq!(must_layer(&p, 90, "n").text.as_deref(), Some("$1,000%"));
    assert_eq!(must_layer(&p, 110, "n").text.as_deref(), Some("$1,000%"));
}

#[test]
fn count_without_motion_leaves_text_unset_and_only_applies_to_text() {
    let p = project(json!([scene(
        "s",
        0.0,
        4.0,
        vec![counter("n"), rect("r", 0.0, 0.0, 10.0, 10.0)],
        vec![motion("r", 0.0, 1.0, count_op(0.0, 5.0, 0, false, "", ""))]
    )]));
    assert_eq!(must_layer(&p, 0, "n").text, None);
    assert_eq!(must_layer(&p, 0, "r").text, None, "non-text ignores count");
    assert_eq!(must_layer(&p, 0, "n").trim, None);
}

#[test]
fn trim_interpolates_and_clamps() {
    let p = project(json!([scene(
        "s",
        0.0,
        4.0,
        vec![polyline_layer("line"), rect("r", 0.0, 0.0, 10.0, 10.0)],
        vec![
            motion(
                "line",
                0.0,
                2.0,
                json!({ "op": "trim", "from": 0.0, "to": 1.0 })
            ),
            motion(
                "r",
                0.0,
                2.0,
                json!({ "op": "trim", "from": 0.0, "to": 1.0 })
            ),
        ]
    )]));
    assert_approx!(must_layer(&p, 0, "line").trim.unwrap(), 0.0);
    assert_approx!(must_layer(&p, 30, "line").trim.unwrap(), 0.5);
    assert_approx!(must_layer(&p, 60, "line").trim.unwrap(), 1.0);
    assert_approx!(must_layer(&p, 100, "line").trim.unwrap(), 1.0);
    assert_eq!(
        must_layer(&p, 30, "r").trim,
        None,
        "non-polyline ignores trim"
    );

    let p = project(json!([scene(
        "s",
        0.0,
        4.0,
        vec![polyline_layer("line")],
        vec![motion(
            "line",
            0.0,
            2.0,
            json!({ "op": "trim", "from": -0.5, "to": 1.5 })
        )]
    )]));
    assert_approx!(must_layer(&p, 0, "line").trim.unwrap(), 0.0);
    assert_approx!(must_layer(&p, 60, "line").trim.unwrap(), 1.0);
}

#[test]
fn format_count_rounds_half_away_from_zero() {
    assert_eq!(timeline::format_count(2.5, 0, false, "", ""), "3");
    assert_eq!(timeline::format_count(-2.5, 0, false, "", ""), "-3");
    assert_eq!(timeline::format_count(0.4, 0, false, "", ""), "0");
    assert_eq!(timeline::format_count(1.25, 1, false, "", ""), "1.3");
    assert_eq!(timeline::format_count(1.23456, 2, false, "", ""), "1.23");
    assert_eq!(timeline::format_count(7.0, 3, false, "", ""), "7.000");
}

#[test]
fn format_count_groups_thousands() {
    assert_eq!(timeline::format_count(999.0, 0, true, "", ""), "999");
    assert_eq!(timeline::format_count(1000.0, 0, true, "", ""), "1,000");
    assert_eq!(
        timeline::format_count(1234567.0, 0, true, "", ""),
        "1,234,567"
    );
    assert_eq!(
        timeline::format_count(1234567.0, 0, false, "", ""),
        "1234567"
    );
    assert_eq!(timeline::format_count(1234.5, 2, true, "", ""), "1,234.50");
    assert_eq!(timeline::format_count(123456.0, 0, true, "", ""), "123,456");
}

#[test]
fn format_count_negative_prefix_and_suffix() {
    assert_eq!(
        timeline::format_count(-1234.5, 1, true, "$", "%"),
        "-$1,234.5%"
    );
    assert_eq!(
        timeline::format_count(42.0, 0, false, "~", " units"),
        "~42 units"
    );
    assert_eq!(timeline::format_count(-7.0, 0, false, "", "x"), "-7x");
}

#[test]
fn format_count_never_produces_negative_zero() {
    assert_eq!(timeline::format_count(-0.0, 0, false, "", ""), "0");
    assert_eq!(timeline::format_count(-0.4, 0, false, "$", ""), "$0");
    assert_eq!(timeline::format_count(-0.001, 2, true, "", "%"), "0.00%");
    assert_eq!(timeline::format_count(-0.005, 2, false, "", ""), "-0.01");
}

#[test]
fn format_count_small_values_and_decimals() {
    assert_eq!(timeline::format_count(0.05, 2, false, "", ""), "0.05");
    assert_eq!(timeline::format_count(0.0, 6, false, "", ""), "0.000000");
    assert_eq!(
        timeline::format_count(1e9, 0, true, "", ""),
        "1,000,000,000"
    );
    assert_eq!(timeline::format_count(f64::NAN, 1, false, "", ""), "0.0");
}
