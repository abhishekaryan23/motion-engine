//! Shared-element motion targets (0.4): scene motions whose `target` is a
//! shared element's layer id compose on top of the element's track, but only
//! while their scene is active. Also the validation rules that guard them and
//! the lifecycle ordering check.
//!
//! Canvas 1080x1920 at 30 fps (frame f is at t = f / 30 s exactly).
//!
//! Main project (absolute seconds):
//!
//! ```text
//! s1  [0.0, 3.0)   s2  [2.0, 5.0)   s3  [4.5, 7.5)
//! track keys of "num" (layer "num_l"): s1@0.5 -> abs 0.5, s2@1.0 -> abs 3.0,
//! s3@0.5 -> abs 5.0, s3@2.0 -> abs 6.5   (all linear)
//! scale: 1 -> 2 -> 2 -> 2     x/y: (300,500) -> (400,560) -> (400,560) -> (450,600)
//! ```

use motion_core::scene::MotionProject;
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
use motion_core::validate::{validate, ValidationErrors};
use serde_json::{json, Value};

const FPS: u32 = 30;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Motion lists per scene plus an optional camera on scene 1.
#[derive(Default, Clone)]
struct Setup {
    s1: Vec<Value>,
    s2: Vec<Value>,
    s3: Vec<Value>,
}

fn motion(target: &str, start: f64, duration: f64, op: Value) -> Value {
    let mut m =
        json!({ "target": target, "start": start, "duration": duration, "easing": "linear" });
    for (k, v) in op.as_object().expect("op is an object") {
        m[k] = v.clone();
    }
    m
}

fn scale_m(start: f64, dur: f64, from: f64, to: f64) -> Value {
    motion(
        "num_l",
        start,
        dur,
        json!({ "op": "scale", "from": from, "to": to }),
    )
}

fn count_m(start: f64, dur: f64, to: f64) -> Value {
    motion(
        "num_l",
        start,
        dur,
        json!({ "op": "count", "from": 0, "to": to, "grouping": true }),
    )
}

fn fade_m(start: f64, dur: f64, from: f64, to: f64) -> Value {
    motion(
        "num_l",
        start,
        dur,
        json!({ "op": "fade", "from": from, "to": to }),
    )
}

fn move_m(start: f64, dur: f64, from: [f64; 2], to: [f64; 2]) -> Value {
    motion(
        "num_l",
        start,
        dur,
        json!({ "op": "move", "from": from, "to": to }),
    )
}

fn rotate_m(start: f64, dur: f64, from: f64, to: f64) -> Value {
    motion(
        "num_l",
        start,
        dur,
        json!({ "op": "rotate", "from": from, "to": to }),
    )
}

fn bg(id: &str) -> Value {
    json!({ "id": id, "type": "rectangle", "width": 1080, "height": 1920, "fill": "#F2EBDD" })
}

fn scene(id: &str, start: f64, dur: f64, motions: Vec<Value>) -> Value {
    json!({
        "id": id, "start_seconds": start, "duration_seconds": dur,
        "layers": [ bg(&format!("bg_{id}")) ],
        "motions": motions,
    })
}

fn num_layer() -> Value {
    json!({
        "id": "num_l", "type": "text", "text": "1,250", "font_role": "number",
        "font_size": 100, "color": "#111111", "width": 400, "height": 120,
        "anchor_x": 0.5, "anchor_y": 0.5,
    })
}

fn key(scene: &str, at: f64, x: f64, y: f64, scale: f64) -> Value {
    json!({ "scene": scene, "at": at, "easing": "linear",
            "state": { "x": x, "y": y, "scale": scale, "opacity": 0.8 } })
}

fn project_json(scenes: Vec<Value>, shared: Vec<Value>) -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "shared_motion" },
        "canvas": { "width": 1080, "height": 1920, "fps": FPS, "background": "#000000" },
        "theme": { "fonts": { "number": "font_number" } },
        "assets": [ { "id": "font_number", "type": "font", "path": "assets/fonts/number.ttf" } ],
        "scenes": scenes,
        "shared": shared,
    })
}

fn main_json(setup: &Setup) -> Value {
    project_json(
        vec![
            scene("s1", 0.0, 3.0, setup.s1.clone()),
            scene("s2", 2.0, 3.0, setup.s2.clone()),
            scene("s3", 4.5, 3.0, setup.s3.clone()),
        ],
        vec![json!({
            "id": "num",
            "layer": num_layer(),
            "track": [
                key("s1", 0.5, 300.0, 500.0, 1.0),
                key("s2", 1.0, 400.0, 560.0, 2.0),
                key("s3", 0.5, 400.0, 560.0, 2.0),
                key("s3", 2.0, 450.0, 600.0, 2.0),
            ],
        })],
    )
}

fn parse(v: &Value) -> MotionProject {
    MotionProject::from_json(&v.to_string()).expect("test project parses")
}

fn main_project(setup: &Setup) -> MotionProject {
    parse(&main_json(setup))
}

fn errors_of(v: &Value) -> ValidationErrors {
    validate(&parse(v), None).expect_err("expected validation errors")
}

fn ok(v: &Value) {
    if let Err(e) = validate(&parse(v), None) {
        panic!("expected a valid project, got:\n{e}");
    }
}

fn frame(p: &MotionProject, f: u32) -> ResolvedFrame<'_> {
    evaluate_frame(p, f).expect("frame evaluates")
}

/// The shared number's resolved layer in `f`, if visible.
fn num<'a, 'p>(fr: &'a ResolvedFrame<'p>) -> Option<&'a ResolvedLayer<'p>> {
    fr.layers.iter().find(|l| l.id == "num_l")
}

fn num_at<'a>(p: &'a MotionProject, f: u32) -> ResolvedLayer<'a> {
    let fr = frame(p, f);
    num(&fr)
        .cloned()
        .unwrap_or_else(|| panic!("shared number not visible at frame {f}"))
}

/// Canvas position of the layer's anchor point (box center for `num_l`).
fn anchor(l: &ResolvedLayer<'_>) -> (f32, f32) {
    l.transform.apply(l.width * 0.5, l.height * 0.5)
}

fn scale_of(l: &ResolvedLayer<'_>) -> f32 {
    (l.transform.a * l.transform.a + l.transform.b * l.transform.b).sqrt()
}

fn rotation_of(l: &ResolvedLayer<'_>) -> f32 {
    l.transform.b.atan2(l.transform.a).to_degrees()
}

fn near(a: f32, b: f32, tol: f32, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: {a} vs {b} (tol {tol})");
}

// ---------------------------------------------------------------------------
// Existing behavior: shared number moves and scales across scenes
// ---------------------------------------------------------------------------

#[test]
fn shared_number_moves_and_scales_along_its_track() {
    let p = main_project(&Setup::default());
    // Before the first key and after the last key the element does not exist.
    assert!(num(&frame(&p, 0)).is_none());
    assert!(num(&frame(&p, 14)).is_none());
    assert!(num(&frame(&p, 196)).is_none());

    // First key (frame 15 = 0.5 s).
    let a = num_at(&p, 15);
    let (ax, ay) = anchor(&a);
    near(ax, 300.0, 0.05, "x at key 1");
    near(ay, 500.0, 0.05, "y at key 1");
    near(scale_of(&a), 1.0, 1e-3, "scale at key 1");

    // Between key 1 (0.5 s) and key 2 (3.0 s): 40 % of the way at t = 1.5 s.
    let m = num_at(&p, 45);
    let (mx, my) = anchor(&m);
    near(mx, 340.0, 0.05, "x mid");
    near(my, 524.0, 0.05, "y mid");
    near(scale_of(&m), 1.4, 1e-3, "scale mid");

    // Key 2 lives in scene 2 (frame 90 = 3.0 s).
    let b = num_at(&p, 90);
    let (bx, by) = anchor(&b);
    near(bx, 400.0, 0.05, "x at key 2");
    near(by, 560.0, 0.05, "y at key 2");
    near(scale_of(&b), 2.0, 1e-3, "scale at key 2");

    // Last key (6.5 s = frame 195).
    let c = num_at(&p, 195);
    let (cx, cy) = anchor(&c);
    near(cx, 450.0, 0.05, "x at last key");
    near(cy, 600.0, 0.05, "y at last key");

    // It is a shared layer: no owning scene, text untouched without count.
    assert_eq!(a.scene, None);
    assert_eq!(a.text, None);
}

// ---------------------------------------------------------------------------
// Scale motion
// ---------------------------------------------------------------------------

#[test]
fn scale_motion_multiplies_track_scale_only_while_its_scene_is_active() {
    let base = main_project(&Setup::default());
    // s2 is active over [2.0, 5.0); the motion runs local 0.5..1.5 = abs 2.5..3.5.
    let p = main_project(&Setup {
        s2: vec![scale_m(0.5, 1.0, 1.0, 1.2)],
        ..Setup::default()
    });
    let ratio = |f: u32| scale_of(&num_at(&p, f)) / scale_of(&num_at(&base, f));

    // Not started yet (holds `from` = 1): identical to the track.
    near(ratio(60), 1.0, 1e-4, "before scene 2 (frame 60)");
    near(ratio(75), 1.0, 1e-4, "at motion start (frame 75)");
    // Half-way: 1.0 -> 1.2 at p = 0.5 = 1.1x the track scale.
    near(ratio(90), 1.1, 1e-4, "mid motion (frame 90)");
    // Finished, scene still active: holds the end value.
    near(ratio(105), 1.2, 1e-4, "motion end (frame 105)");
    near(ratio(120), 1.2, 1e-4, "held (frame 120)");
    near(ratio(149), 1.2, 1e-4, "last frame of scene 2 (frame 149)");
    // Scene 2 is over at 5.0 s (frame 150): the element is back to its track.
    near(ratio(150), 1.0, 1e-4, "scene 2 ended (frame 150)");
    near(ratio(165), 1.0, 1e-4, "after scene 2 (frame 165)");
    near(ratio(195), 1.0, 1e-4, "end of track (frame 195)");
}

#[test]
fn scale_motion_axis_only_affects_that_axis() {
    let p = main_project(&Setup {
        s2: vec![motion(
            "num_l",
            0.0,
            0.5,
            json!({ "op": "scale", "from": 1.0, "to": 2.0, "axis": "x" }),
        )],
        ..Setup::default()
    });
    let base = main_project(&Setup::default());
    let (w, b) = (num_at(&p, 120), num_at(&base, 120));
    near(w.transform.a / b.transform.a, 2.0, 1e-4, "x axis scaled");
    near(w.transform.d / b.transform.d, 1.0, 1e-4, "y axis untouched");
}

// ---------------------------------------------------------------------------
// Count motion
// ---------------------------------------------------------------------------

#[test]
fn count_motion_sets_text_midway_and_holds_end_value_while_scene_active() {
    // s1 [0, 3): count 0 -> 1250 over local 0.5..1.5 (abs 0.5..1.5).
    let p = main_project(&Setup {
        s1: vec![count_m(0.5, 1.0, 1250.0)],
        ..Setup::default()
    });
    let text = |f: u32| {
        let fr = frame(&p, f);
        num(&fr).map(|l| l.text.clone())
    };
    assert_eq!(text(15), Some(Some("0".to_string())), "start value");
    assert_eq!(text(30), Some(Some("625".to_string())), "half-way");
    assert_eq!(text(45), Some(Some("1,250".to_string())), "end, grouped");
    // Holds the end value while scene 1 is active (through 2.97 s).
    assert_eq!(text(60), Some(Some("1,250".to_string())), "held at 2.0 s");
    assert_eq!(text(75), Some(Some("1,250".to_string())), "held at 2.5 s");
    assert_eq!(text(89), Some(Some("1,250".to_string())), "held at 2.97 s");
    // Scene 1 ends at 3.0 s: no count motion active any more -> plain layer text.
    assert_eq!(text(90), Some(None), "scene 1 over");
    assert_eq!(text(120), Some(None), "later");
    assert_eq!(text(195), Some(None), "end of track");
}

#[test]
fn count_motion_groups_midway_values_and_never_leaks_without_a_motion() {
    let p = main_project(&Setup {
        s1: vec![count_m(0.5, 1.0, 2500.0)],
        ..Setup::default()
    });
    assert_eq!(num_at(&p, 30).text.as_deref(), Some("1,250"));

    // No count motion anywhere: text is None on every frame.
    let none = main_project(&Setup {
        s1: vec![fade_m(0.5, 1.0, 1.0, 0.5)],
        s2: vec![scale_m(0.5, 1.0, 1.0, 1.2)],
        ..Setup::default()
    });
    for f in 15..=195 {
        assert_eq!(num_at(&none, f).text, None, "frame {f}");
    }
}

// ---------------------------------------------------------------------------
// Fade, move, rotate
// ---------------------------------------------------------------------------

#[test]
fn fade_motion_multiplies_track_opacity_only_while_scene_active() {
    // Track opacity is 0.8. Fade 1 -> 0.5 over local 0.5..1.5 in s1.
    let p = main_project(&Setup {
        s1: vec![fade_m(0.5, 1.0, 1.0, 0.5)],
        ..Setup::default()
    });
    near(num_at(&p, 15).opacity, 0.8, 1e-3, "start");
    near(num_at(&p, 30).opacity, 0.6, 1e-3, "half-way");
    near(num_at(&p, 45).opacity, 0.4, 1e-3, "end");
    near(num_at(&p, 75).opacity, 0.4, 1e-3, "held while scene active");
    near(
        num_at(&p, 90).opacity,
        0.8,
        1e-3,
        "scene over: back to track",
    );
}

#[test]
fn fade_to_zero_hides_the_element_while_active() {
    let p = main_project(&Setup {
        s1: vec![fade_m(0.5, 0.5, 1.0, 0.0)],
        ..Setup::default()
    });
    assert!(num(&frame(&p, 60)).is_none(), "faded out");
    assert!(num(&frame(&p, 90)).is_some(), "back after scene 1");
}

#[test]
fn move_motion_offsets_the_anchor_in_scene_pixels() {
    let base = main_project(&Setup::default());
    // s1: offset 0 -> (40, -20) over local 0.5..1.5.
    let p = main_project(&Setup {
        s1: vec![move_m(0.5, 1.0, [0.0, 0.0], [40.0, -20.0])],
        ..Setup::default()
    });
    let delta = |f: u32| {
        let (a, b) = (anchor(&num_at(&p, f)), anchor(&num_at(&base, f)));
        (a.0 - b.0, a.1 - b.1)
    };
    let d0 = delta(15);
    near(d0.0, 0.0, 1e-3, "dx at start");
    near(d0.1, 0.0, 1e-3, "dy at start");
    let d1 = delta(30);
    near(d1.0, 20.0, 1e-3, "dx half-way");
    near(d1.1, -10.0, 1e-3, "dy half-way");
    let d2 = delta(60);
    near(d2.0, 40.0, 1e-3, "dx held");
    near(d2.1, -20.0, 1e-3, "dy held");
    // Scene 1 over: offset is gone.
    let d3 = delta(90);
    near(d3.0, 0.0, 1e-3, "dx after scene");
    near(d3.1, 0.0, 1e-3, "dy after scene");
    // Move does not change scale.
    near(
        scale_of(&num_at(&p, 45)),
        scale_of(&num_at(&base, 45)),
        1e-4,
        "scale unaffected",
    );
}

#[test]
fn rotate_motion_adds_degrees() {
    let base = main_project(&Setup::default());
    let p = main_project(&Setup {
        s1: vec![rotate_m(0.5, 1.0, 0.0, 30.0)],
        ..Setup::default()
    });
    let d = |f: u32| rotation_of(&num_at(&p, f)) - rotation_of(&num_at(&base, f));
    near(d(15), 0.0, 1e-2, "start");
    near(d(30), 15.0, 1e-2, "half-way");
    near(d(45), 30.0, 1e-2, "end");
    near(d(75), 30.0, 1e-2, "held");
    near(d(90), 0.0, 1e-2, "scene over");
    // Rotation is about the anchor: the anchor does not move.
    let (a, b) = (anchor(&num_at(&p, 30)), anchor(&num_at(&base, 30)));
    near(a.0, b.0, 1e-2, "anchor x");
    near(a.1, b.1, 1e-2, "anchor y");
}

// ---------------------------------------------------------------------------
// Camera coherence
// ---------------------------------------------------------------------------

/// One scene with a linear push 1 -> 1.2 over its 4 s; the element's keys are
/// both in that scene at the same spot, so its camera unit equals the zoom.
fn camera_json(motions: Vec<Value>) -> Value {
    let mut s = scene("cam", 0.0, 4.0, motions);
    s["camera"] = json!({ "motions": [
        { "start": 0.0, "duration": 4.0, "easing": "linear", "op": "push", "from": 1.0, "to": 1.2 }
    ] });
    project_json(
        vec![s],
        vec![json!({
            "id": "num",
            "layer": num_layer(),
            "track": [
                key("cam", 0.0, 300.0, 500.0, 1.0),
                key("cam", 3.5, 300.0, 500.0, 1.0),
            ],
        })],
    )
}

#[test]
fn move_offset_is_scaled_by_camera_zoom() {
    let base = parse(&camera_json(vec![]));
    let moved = parse(&camera_json(vec![move_m(
        0.0,
        0.5,
        [10.0, 0.0],
        [10.0, 0.0],
    )]));
    // zoom(t) = 1 + 0.2 * t / 4; frames 0, 60, 90, 105 -> t = 0, 2, 3, 3.5.
    for f in [0u32, 60, 90, 105] {
        let z = 1.0 + 0.2 * (f as f32 / FPS as f32) / 4.0;
        let (a, b) = (anchor(&num_at(&moved, f)), anchor(&num_at(&base, f)));
        near(
            a.0 - b.0,
            10.0 * z,
            0.01,
            &format!("dx at frame {f} (z={z})"),
        );
        near(a.1 - b.1, 0.0, 0.01, &format!("dy at frame {f}"));
        // The element itself is scaled by the zoom (depth 1).
        near(scale_of(&num_at(&base, f)), z, 1e-4, "base scale = zoom");
    }
}

#[test]
fn move_offset_reads_the_same_size_on_a_scene_layer_and_a_shared_layer() {
    // A scene layer with the same move under the same camera moves by 10 * z too.
    let mut v = camera_json(vec![]);
    v["scenes"][0]["layers"].as_array_mut().expect("layers").push(json!(
        { "id": "probe", "type": "rectangle", "x": 100, "y": 100, "width": 10, "height": 10, "fill": "#FF0000" }
    ));
    let base = parse(&v);
    v["scenes"][0]["motions"] = json!([
        motion(
            "probe",
            0.0,
            0.5,
            json!({ "op": "move", "from": [10, 0], "to": [10, 0] })
        ),
        move_m(0.0, 0.5, [10.0, 0.0], [10.0, 0.0]),
    ]);
    let moved = parse(&v);
    let f = 60;
    let probe = |p: &MotionProject| {
        let fr = frame(p, f);
        fr.layers
            .iter()
            .find(|l| l.id == "probe")
            .map(|l| l.transform.apply(0.0, 0.0))
            .expect("probe visible")
    };
    let probe_dx = probe(&moved).0 - probe(&base).0;
    let shared_dx = anchor(&num_at(&moved, f)).0 - anchor(&num_at(&base, f)).0;
    near(probe_dx, shared_dx, 0.01, "scene layer vs shared layer");
}

// ---------------------------------------------------------------------------
// Overlap: one logical element through a transition
// ---------------------------------------------------------------------------

#[test]
fn overlap_window_shows_exactly_one_element_with_continuous_position() {
    // Every motion kind except move (move is tested against a baseline above).
    let p = main_project(&Setup {
        s1: vec![count_m(0.5, 1.0, 1250.0), fade_m(0.5, 1.0, 1.0, 0.9)],
        s2: vec![scale_m(0.5, 1.0, 1.0, 1.2), rotate_m(0.0, 1.0, 0.0, 5.0)],
        s3: vec![fade_m(0.0, 1.0, 0.9, 1.0)],
    });

    // Overlap of s1/s2 is [2.0, 3.0) = frames 60..90.
    for f in 60..90 {
        let fr = frame(&p, f);
        assert!(fr.active_scenes.contains(&"s1") && fr.active_scenes.contains(&"s2"));
        assert_eq!(fr.active_shared, vec!["num"], "frame {f}");
        assert_eq!(
            fr.layers.iter().filter(|l| l.id == "num_l").count(),
            1,
            "frame {f}: exactly one resolved layer"
        );
    }
    // The other overlap, s2/s3 in [4.5, 5.0) = frames 135..150.
    for f in 135..150 {
        let fr = frame(&p, f);
        assert_eq!(fr.layers.iter().filter(|l| l.id == "num_l").count(), 1);
    }

    // Continuity: the anchor never jumps more than a few px between frames
    // over the element's whole life (0.5 s .. 6.5 s), scene boundaries included.
    let mut prev: Option<(f32, f32)> = None;
    let mut max_step = 0.0f32;
    for f in 15..=195 {
        let a = anchor(&num_at(&p, f));
        if let Some(q) = prev {
            max_step = max_step.max(((a.0 - q.0).powi(2) + (a.1 - q.1).powi(2)).sqrt());
        }
        prev = Some(a);
    }
    assert!(max_step < 3.0, "max per-frame step {max_step} px");
    assert!(max_step > 0.0, "the element does move");
}

#[test]
fn shared_element_is_never_duplicated_in_any_frame() {
    let p = main_project(&Setup::default());
    for f in 0..=220 {
        let fr = frame(&p, f);
        assert!(fr.layers.iter().filter(|l| l.id == "num_l").count() <= 1);
        assert!(fr.active_shared.len() <= 1);
    }
}

// ---------------------------------------------------------------------------
// Shared phrase with a layout-bound key
// ---------------------------------------------------------------------------

#[test]
fn shared_phrase_travels_between_layout_bound_keys_as_one_element() {
    let frame_layer = |id: &str, x: f64, y: f64| {
        json!({ "id": id, "type": "rectangle", "x": x, "y": y, "width": 400, "height": 200,
                "fill": "#DDDDDD" })
    };
    let mut a = scene("a", 0.0, 2.5, vec![]);
    a["layers"] = json!([bg("bg_a"), frame_layer("frame_a", 100.0, 300.0)]);
    let mut b = scene("b", 2.0, 2.5, vec![]);
    b["layers"] = json!([bg("bg_b"), frame_layer("frame_b", 500.0, 900.0)]);
    let bound = |scene: &str, at: f64, parent: &str| {
        json!({ "scene": scene, "at": at, "easing": "linear",
                "layout": { "parent": parent, "horizontal": "center", "vertical": "center" } })
    };
    let v = project_json(
        vec![a, b],
        vec![json!({
            "id": "phrase",
            "layer": { "id": "phrase_l", "type": "text", "text": "on time", "font_role": "number",
                       "font_size": 60, "color": "#111111", "width": 300, "height": 80 },
            "track": [ bound("a", 0.5, "frame_a"), bound("b", 1.0, "frame_b") ],
        })],
    );
    ok(&v);
    let p = parse(&v);
    let center = |f: u32| {
        let fr = frame(&p, f);
        let l = fr
            .layers
            .iter()
            .find(|l| l.id == "phrase_l")
            .unwrap_or_else(|| panic!("phrase missing at frame {f}"));
        assert_eq!(l.scene, None);
        l.transform.apply(l.width * 0.5, l.height * 0.5)
    };
    // Key 1 (0.5 s): centered in frame_a = (300, 400).
    let c1 = center(15);
    near(c1.0, 300.0, 0.5, "x at key 1");
    near(c1.1, 400.0, 0.5, "y at key 1");
    // Key 2 (3.0 s): centered in frame_b = (700, 1000).
    let c2 = center(90);
    near(c2.0, 700.0, 0.5, "x at key 2");
    near(c2.1, 1000.0, 0.5, "y at key 2");
    // 2.25 s is inside the overlap [2.0, 2.5): p = 0.7 of the way.
    let mid = |f: u32| {
        let fr = frame(&p, f);
        assert_eq!(
            fr.active_scenes,
            vec!["a", "b"],
            "frame {f} is in the overlap"
        );
        assert_eq!(fr.layers.iter().filter(|l| l.id == "phrase_l").count(), 1);
        assert_eq!(fr.active_shared, vec!["phrase"]);
        center(f)
    };
    // 2.1 s = frame 63: p = (2.1 - 0.5) / 2.5 = 0.64.
    let m = mid(63);
    near(m.0, 300.0 + 0.64 * 400.0, 0.5, "x in overlap");
    near(m.1, 400.0 + 0.64 * 600.0, 0.5, "y in overlap");
    // Continuous through the whole transition.
    let mut prev = center(15);
    for f in 16..=90 {
        let c = center(f);
        let step = ((c.0 - prev.0).powi(2) + (c.1 - prev.1).powi(2)).sqrt();
        assert!(step < 12.0, "frame {f}: jump of {step} px");
        prev = c;
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn has_error(e: &ValidationErrors, path_part: &str, msg_part: &str) -> bool {
    e.0.iter()
        .any(|x| x.path.contains(path_part) && x.message.contains(msg_part))
}

#[test]
fn baseline_project_with_shared_motions_is_valid() {
    ok(&main_json(&Setup {
        s1: vec![
            count_m(0.5, 1.0, 1250.0),
            fade_m(0.5, 1.0, 1.0, 0.5),
            move_m(0.5, 1.0, [0.0, 0.0], [10.0, 0.0]),
            rotate_m(0.5, 1.0, 0.0, 10.0),
        ],
        s2: vec![scale_m(0.5, 1.0, 1.0, 1.2)],
        s3: vec![],
    }));
    ok(&camera_json(vec![move_m(
        0.0,
        0.5,
        [10.0, 0.0],
        [10.0, 0.0],
    )]));
}

#[test]
fn motion_targeting_shared_layer_in_scene_without_a_track_key_is_an_error() {
    let mut v = main_json(&Setup::default());
    // s4 has no key of "num".
    v["scenes"].as_array_mut().expect("scenes").push(scene(
        "s4",
        7.5,
        2.0,
        vec![fade_m(0.0, 0.5, 1.0, 0.5)],
    ));
    let e = errors_of(&v);
    assert!(has_error(&e, "s4", "no track key"), "{e}");
    assert!(
        e.0.iter().any(|x| x.path.ends_with(".target")),
        "error should point at the motion target: {e}"
    );
}

#[test]
fn mask_reveal_on_a_shared_element_is_an_error() {
    let v = main_json(&Setup {
        s1: vec![motion(
            "num_l",
            0.0,
            0.5,
            json!({ "op": "mask_reveal", "direction": "right" }),
        )],
        ..Setup::default()
    });
    let e = errors_of(&v);
    assert!(
        has_error(&e, ".op", "shared element"),
        "expected an op error for mask_reveal on a shared element: {e}"
    );
}

#[test]
fn other_non_transform_ops_on_a_shared_element_are_errors() {
    for op in [
        json!({ "op": "clip_reveal", "direction": "up" }),
        json!({ "op": "trim", "from": 0.0, "to": 1.0 }),
        json!({ "op": "accent_expand", "to": { "x": 0, "y": 0, "width": 10, "height": 10 } }),
    ] {
        let v = main_json(&Setup {
            s1: vec![motion("num_l", 0.0, 0.5, op.clone())],
            ..Setup::default()
        });
        let e = errors_of(&v);
        assert!(has_error(&e, ".op", "shared element"), "{op}: {e}");
    }
}

#[test]
fn overlapping_scale_motions_across_scenes_conflict_in_absolute_time() {
    // s2's scale motion is abs 2.5..3.5. s1's local 1.8..2.8 is abs 1.8..2.8.
    let v = main_json(&Setup {
        s1: vec![scale_m(1.8, 1.0, 1.0, 1.1)],
        s2: vec![scale_m(0.5, 1.0, 1.0, 1.2)],
        ..Setup::default()
    });
    let e = errors_of(&v);
    assert!(
        has_error(&e, "motions", "conflicting animations on shared element"),
        "{e}"
    );
}

#[test]
fn back_to_back_or_different_channel_motions_across_scenes_do_not_conflict() {
    // s1 scale ends at abs 2.5 exactly when s2's starts.
    ok(&main_json(&Setup {
        s1: vec![scale_m(1.0, 1.5, 1.0, 1.1)],
        s2: vec![scale_m(0.5, 1.0, 1.0, 1.2)],
        ..Setup::default()
    }));
    // Overlapping in time but on different channels.
    ok(&main_json(&Setup {
        s1: vec![scale_m(1.8, 1.0, 1.0, 1.1)],
        s2: vec![fade_m(0.5, 1.0, 1.0, 0.9)],
        ..Setup::default()
    }));
}

#[test]
fn lifecycle_with_evolve_before_read_is_an_error_and_a_valid_one_is_ok() {
    let lifecycle = |evolve: f64| {
        json!({ "enter": 0.5, "settle": 1.0, "read": 1.5, "evolve": evolve,
                "anticipate": 2.0, "bridge": 2.5 })
    };
    let mut v = main_json(&Setup::default());
    v["scenes"][0]["lifecycle"] = lifecycle(1.2);
    let e = errors_of(&v);
    assert!(has_error(&e, "lifecycle", "0 <= enter"), "{e}");

    v["scenes"][0]["lifecycle"] = lifecycle(1.8);
    ok(&v);

    // Bridge beyond the scene duration (3.0 s) is also invalid.
    v["scenes"][0]["lifecycle"] = json!({ "enter": 0.5, "settle": 1.0, "read": 1.5,
        "evolve": 1.8, "anticipate": 2.0, "bridge": 3.5 });
    let e = errors_of(&v);
    assert!(has_error(&e, "lifecycle", "duration"), "{e}");
}
