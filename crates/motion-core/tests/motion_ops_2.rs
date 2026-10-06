//! Timeline side of the 0.14 contract (docs/MOTION_OPS_2.md): spring progress,
//! layer shake, camera shake/roll, glyph cascade, path morph, echo, pulse and
//! post effects, plus determinism and "absent fields change nothing".
//!
//! Canvas 1080x1920 at 30 fps (frame f is at t = f / 30 s). Layers use anchor
//! 0 and no rotation so `transform.{e,f}` is the box origin.

use motion_core::easing::spring_progress;
use motion_core::noise::smooth;
use motion_core::scene::{GlyphPose, MotionProject, PostKind, SpringSpec};
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
use motion_core::validate::validate;
use serde_json::{json, Value};

const FPS: u32 = 30;
const PIVOT: (f32, f32) = (540.0, 960.0);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse(v: &Value) -> MotionProject {
    MotionProject::from_json(&v.to_string()).expect("test project parses")
}

fn project_json(scenes: Vec<Value>, envelopes: Value) -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "motion_ops_2" },
        "canvas": { "width": 1080, "height": 1920, "fps": FPS, "background": "#000000" },
        "theme": { "fonts": { "display": "font_display" } },
        "assets": [ { "id": "font_display", "type": "font", "path": "assets/fonts/display.ttf" } ],
        "scenes": scenes,
        "envelopes": envelopes,
    })
}

fn scene_json(id: &str, start: f64, dur: f64, layers: Vec<Value>, motions: Vec<Value>) -> Value {
    json!({ "id": id, "start_seconds": start, "duration_seconds": dur,
            "layers": layers, "motions": motions })
}

/// One scene `s` of 4 s starting at 0.
fn project_of(layers: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    parse(&project_json(
        vec![scene_json("s", 0.0, 4.0, layers, motions)],
        json!([]),
    ))
}

fn motion(target: &str, start: f64, dur: f64, op: Value) -> Value {
    let mut m = json!({ "target": target, "start": start, "duration": dur, "easing": "linear" });
    for (k, v) in op.as_object().expect("op is an object") {
        m[k] = v.clone();
    }
    m
}

fn rect(id: &str, x: f64, y: f64) -> Value {
    json!({ "id": id, "type": "rectangle", "x": x, "y": y, "width": 100, "height": 100,
            "fill": "#FFFFFF" })
}

fn text(id: &str, s: &str) -> Value {
    json!({ "id": id, "type": "text", "text": s, "font_role": "display", "font_size": 80,
            "color": "#FFFFFF", "width": 600, "height": 120 })
}

fn polyline(id: &str, points: Value) -> Value {
    json!({ "id": id, "type": "polyline", "width": 200, "height": 100, "points": points,
            "stroke": { "color": "#FFFFFF", "width": 4 } })
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

fn layer<'a>(p: &'a MotionProject, f: u32, id: &str) -> ResolvedLayer<'a> {
    let fr = frame(p, f);
    find_in(&fr.layers, id)
        .cloned()
        .unwrap_or_else(|| panic!("layer '{id}' missing at frame {f}"))
}

fn origin(l: &ResolvedLayer<'_>) -> (f32, f32) {
    (l.transform.e, l.transform.f)
}

fn rotation_of(l: &ResolvedLayer<'_>) -> f32 {
    l.transform.b.atan2(l.transform.a).to_degrees()
}

fn near(a: f32, b: f32, tol: f32, what: &str) {
    assert!((a - b).abs() <= tol, "{what}: got {a}, expected {b}");
}

fn valid(p: &MotionProject) {
    if let Err(e) = validate(p, None) {
        panic!("expected a valid project, got:\n{e}");
    }
}

// ---------------------------------------------------------------------------
// 1. Springs
// ---------------------------------------------------------------------------

const BOUNCY: SpringSpec = SpringSpec {
    stiffness: 300.0,
    damping: 10.0,
    mass: 1.0,
};

fn bouncy_json() -> Value {
    json!({ "stiffness": 300.0, "damping": 10.0, "mass": 1.0 })
}

#[test]
fn spring_replaces_easing_and_lands_exactly() {
    let mut m = motion(
        "a",
        0.5,
        1.0,
        json!({ "op": "move", "from": [0, 0], "to": [200, 0] }),
    );
    m["spring"] = bouncy_json();
    let p = project_of(vec![rect("a", 100.0, 50.0)], vec![m]);
    valid(&p);

    // Before the start: the `from` value holds.
    assert_eq!(origin(&layer(&p, 0, "a")), (100.0, 50.0));
    let mut overshot = false;
    for f in 15..=60 {
        let elapsed = f as f64 / FPS as f64 - 0.5;
        let want = 100.0 + 200.0 * spring_progress(elapsed, 1.0, BOUNCY) as f32;
        let got = origin(&layer(&p, f, "a")).0;
        near(got, want, 1e-3, &format!("frame {f}"));
        overshot |= got > 300.0 + 1.0;
    }
    assert!(overshot, "an under-damped spring overshoots its target");
    // Landing: exactly `to` at the end and after.
    for f in [45, 46, 90, 119] {
        assert_eq!(origin(&layer(&p, f, "a")).0, 300.0, "frame {f}");
    }
}

#[test]
fn count_motions_with_a_spring_never_overshoot() {
    let mut m = motion(
        "n",
        0.0,
        1.0,
        json!({ "op": "count", "from": 0, "to": 100 }),
    );
    m["spring"] = bouncy_json();
    let p = project_of(vec![text("n", "0")], vec![m]);
    let mut prev = 0.0f64;
    for f in 0..=45 {
        let shown = layer(&p, f, "n").text.expect("count text");
        let v: f64 = shown.parse().expect("integer text");
        assert!((0.0..=100.0).contains(&v), "frame {f}: {v}");
        assert!(v >= prev, "frame {f}: monotone landing");
        prev = v;
    }
    assert_eq!(prev, 100.0);
}

#[test]
fn spring_applies_to_shared_element_motions() {
    let mut m = motion(
        "e_l",
        0.5,
        1.0,
        json!({ "op": "rotate", "from": 0, "to": 90 }),
    );
    m["spring"] = bouncy_json();
    let mut v = project_json(vec![scene_json("s", 0.0, 4.0, vec![], vec![m])], json!([]));
    v["shared"] = json!([{
        "id": "e", "layer": rect("e_l", 0.0, 0.0),
        "track": [
            { "scene": "s", "at": 0.0, "state": { "x": 300, "y": 400 } },
            { "scene": "s", "at": 3.0, "state": { "x": 300, "y": 400 } },
        ],
    }]);
    let p = parse(&v);
    for f in [20, 25, 30, 40, 60] {
        let elapsed = f as f64 / FPS as f64 - 0.5;
        let want = 90.0 * spring_progress(elapsed, 1.0, BOUNCY) as f32;
        near(
            rotation_of(&layer(&p, f, "e_l")),
            want,
            1e-2,
            &format!("frame {f}"),
        );
    }
}

// ---------------------------------------------------------------------------
// 2. Layer shake
// ---------------------------------------------------------------------------

fn shake_op(amp: [f64; 2], rot: f64, freq: f64, decay: f64, seed: u32) -> Value {
    json!({ "op": "shake", "amplitude": amp, "rotation": rot, "frequency": freq,
            "decay": decay, "seed": seed })
}

#[test]
fn shake_is_additive_windowed_and_decaying() {
    let shake = motion("a", 0.5, 1.0, shake_op([10.0, 20.0], 5.0, 4.0, 1.5, 7));
    let mv = motion(
        "a",
        0.0,
        2.0,
        json!({ "op": "move", "from": [0, 0], "to": [200, 0] }),
    );
    let rot = motion(
        "a",
        0.0,
        2.0,
        json!({ "op": "rotate", "from": 0, "to": 20 }),
    );
    let p = project_of(vec![rect("a", 100.0, 50.0)], vec![shake, mv, rot]);
    valid(&p);

    for f in [0, 10, 14, 61, 90] {
        // Outside [0.5, 1.5]: only Move / Rotate.
        let t = f as f64 / FPS as f64;
        let l = layer(&p, f, "a");
        let want_x = 100.0 + 200.0 * (t / 2.0).min(1.0) as f32;
        near(origin(&l).0, want_x, 1e-3, &format!("x f{f}"));
        near(origin(&l).1, 50.0, 1e-4, &format!("y f{f}"));
        near(
            rotation_of(&l),
            20.0 * (t / 2.0).min(1.0) as f32,
            1e-3,
            &format!("rot f{f}"),
        );
    }
    for f in [15, 18, 25, 33, 44, 45] {
        let t = f as f64 / FPS as f64;
        let e = t - 0.5;
        let fall = (-1.5 * e).exp();
        let dx = 10.0 * smooth(7, 4.0 * e) * fall;
        let dy = 20.0 * smooth(7 ^ 0x9E37_79B9, 4.0 * e) * fall;
        let drot = 5.0 * smooth(7 ^ 0x85EB_CA6B, 4.0 * e) * fall;
        let l = layer(&p, f, "a");
        near(
            origin(&l).0,
            100.0 + 200.0 * (t / 2.0) as f32 + dx as f32,
            2e-3,
            &format!("x f{f}"),
        );
        near(origin(&l).1, 50.0 + dy as f32, 2e-3, &format!("y f{f}"));
        near(
            rotation_of(&l),
            20.0 * (t / 2.0) as f32 + drot as f32,
            2e-3,
            &format!("rot f{f}"),
        );
    }
}

#[test]
fn several_shakes_on_one_layer_sum() {
    let s1 = motion("a", 0.0, 2.0, shake_op([10.0, 0.0], 0.0, 3.0, 0.0, 1));
    let s2 = motion("a", 0.0, 2.0, shake_op([4.0, 0.0], 0.0, 5.0, 0.0, 2));
    let both = project_of(vec![rect("a", 0.0, 0.0)], vec![s1.clone(), s2.clone()]);
    let only1 = project_of(vec![rect("a", 0.0, 0.0)], vec![s1]);
    let only2 = project_of(vec![rect("a", 0.0, 0.0)], vec![s2]);
    for f in [7, 20, 41] {
        let sum = origin(&layer(&only1, f, "a")).0 + origin(&layer(&only2, f, "a")).0;
        near(origin(&layer(&both, f, "a")).0, sum, 1e-4, &format!("f{f}"));
    }
}

#[test]
fn shake_on_a_bound_layer_offsets_inside_the_parent_box() {
    let child = json!({ "id": "c", "type": "rectangle", "width": 20, "height": 20,
        "fill": "#FFFFFF",
        "layout": { "parent": "a", "horizontal": "left", "vertical": "top" } });
    let m = motion("c", 0.0, 2.0, shake_op([30.0, 0.0], 0.0, 3.0, 0.0, 5));
    let p = project_of(vec![rect("a", 200.0, 300.0), child], vec![m]);
    let e = 0.5;
    let want = 200.0 + 30.0 * smooth(5, 3.0 * e) as f32;
    near(
        origin(&layer(&p, 15, "c")).0,
        want,
        2e-3,
        "bound child shakes in x",
    );
}

// ---------------------------------------------------------------------------
// 3. Camera shake / roll
// ---------------------------------------------------------------------------

fn cam_motion(start: f64, dur: f64, op: Value) -> Value {
    let mut m = json!({ "start": start, "duration": dur, "easing": "linear" });
    for (k, v) in op.as_object().expect("op is an object") {
        m[k] = v.clone();
    }
    m
}

fn project_cam(layers: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    let mut sc = scene_json("s", 0.0, 4.0, layers, vec![]);
    sc["camera"] = json!({ "motions": motions });
    parse(&project_json(vec![sc], json!([])))
}

fn dot(id: &str, x: f64, y: f64, depth: f64) -> Value {
    let mut l = rect(id, x, y);
    l["depth"] = json!(depth);
    l
}

#[test]
fn camera_roll_rotates_every_depth_about_the_pivot() {
    let p = project_cam(
        vec![
            dot("near", 800.0, 1200.0, 1.0),
            dot("far", 800.0, 1200.0, 0.0),
        ],
        vec![cam_motion(
            0.0,
            2.0,
            json!({ "op": "roll", "from": 0, "to": 90 }),
        )],
    );
    valid(&p);
    // At rest (frame 0): identity.
    assert_eq!(origin(&layer(&p, 0, "near")), (800.0, 1200.0));
    // Half way (45 degrees) and done (90 degrees): same rotation for both depths.
    for (f, deg) in [(30u32, 45.0f32), (60, 90.0), (90, 90.0)] {
        let (s, c) = deg.to_radians().sin_cos();
        let (vx, vy) = (800.0 - PIVOT.0, 1200.0 - PIVOT.1);
        let want = (PIVOT.0 + c * vx - s * vy, PIVOT.1 + s * vx + c * vy);
        for id in ["near", "far"] {
            let l = layer(&p, f, id);
            near(origin(&l).0, want.0, 0.02, &format!("{id} x f{f}"));
            near(origin(&l).1, want.1, 0.02, &format!("{id} y f{f}"));
            near(rotation_of(&l), deg, 0.01, &format!("{id} rot f{f}"));
        }
    }
}

#[test]
fn camera_shake_is_windowed_squared_in_trauma_and_after_parallax() {
    let shake = |trauma: f64| {
        cam_motion(
            0.2,
            1.0,
            json!({ "op": "shake", "trauma": trauma, "frequency": 5.0, "decay": 0.0, "seed": 3 }),
        )
    };
    // A layer at the pivot with anchor 0 stays at pivot + shake (roll is about it).
    let at_pivot = |trauma: f64| {
        project_cam(
            vec![dot("a", PIVOT.0 as f64, PIVOT.1 as f64, 1.0)],
            vec![shake(trauma)],
        )
    };
    let full = at_pivot(1.0);
    let half = at_pivot(0.5);
    valid(&full);
    // Outside the window: identity.
    for f in [0, 5, 37, 100] {
        assert_eq!(origin(&layer(&full, f, "a")), PIVOT, "frame {f}");
    }
    for f in [6, 10, 20, 30, 36] {
        let e = f as f64 / FPS as f64 - 0.2;
        let reach = 0.025 * 1080.0;
        let dx = (reach * smooth(3, 5.0 * e)) as f32;
        let dy = (reach * smooth(3 ^ 0x9E37_79B9, 5.0 * e)) as f32;
        let roll = (3.0 * smooth(3 ^ 0x85EB_CA6B, 5.0 * e)) as f32;
        let l = layer(&full, f, "a");
        near(origin(&l).0, PIVOT.0 + dx, 0.02, &format!("x f{f}"));
        near(origin(&l).1, PIVOT.1 + dy, 0.02, &format!("y f{f}"));
        near(rotation_of(&l), roll, 0.01, &format!("roll f{f}"));
        // trauma 0.5 -> a quarter of the displacement.
        let h = layer(&half, f, "a");
        near(
            origin(&h).0 - PIVOT.0,
            dx * 0.25,
            0.02,
            &format!("half x f{f}"),
        );
    }
    // Shake displaces every depth equally, after the parallax: a push and a
    // depth-0 layer both move by the same shake vector.
    let mut sc = scene_json(
        "s",
        0.0,
        4.0,
        vec![dot("d0", 100.0, 100.0, 0.0), dot("d1", 700.0, 900.0, 1.0)],
        vec![],
    );
    sc["camera"] = json!({ "motions": [
        cam_motion(0.0, 2.0, json!({ "op": "push", "from": 1.0, "to": 1.5 })),
        shake(1.0),
    ] });
    let both = parse(&project_json(vec![sc], json!([])));
    let mut calm = scene_json(
        "s",
        0.0,
        4.0,
        vec![dot("d0", 100.0, 100.0, 0.0), dot("d1", 700.0, 900.0, 1.0)],
        vec![],
    );
    calm["camera"] = json!({ "motions": [
        cam_motion(0.0, 2.0, json!({ "op": "push", "from": 1.0, "to": 1.5 })),
    ] });
    let calm = parse(&project_json(vec![calm], json!([])));
    let f = 20;
    let e = f as f64 / FPS as f64 - 0.2;
    let dx = (0.025 * 1080.0 * smooth(3, 5.0 * e)) as f32;
    let dy = (0.025 * 1080.0 * smooth(3 ^ 0x9E37_79B9, 5.0 * e)) as f32;
    let roll = ((3.0 * smooth(3 ^ 0x85EB_CA6B, 5.0 * e)) as f32).to_radians();
    for id in ["d0", "d1"] {
        let base = origin(&layer(&calm, f, id));
        let (vx, vy) = (base.0 - PIVOT.0, base.1 - PIVOT.1);
        let (s, c) = roll.sin_cos();
        let want = (
            PIVOT.0 + c * vx - s * vy + dx,
            PIVOT.1 + s * vx + c * vy + dy,
        );
        let got = origin(&layer(&both, f, id));
        near(got.0, want.0, 0.05, &format!("{id} x"));
        near(got.1, want.1, 0.05, &format!("{id} y"));
    }
}

#[test]
fn camera_roll_moves_free_shared_elements_too() {
    let mut sc = scene_json("s", 0.0, 4.0, vec![], vec![]);
    sc["camera"] = json!({ "motions": [
        cam_motion(0.0, 2.0, json!({ "op": "roll", "from": 0, "to": 90 })),
    ] });
    let mut v = project_json(vec![sc], json!([]));
    v["shared"] = json!([{
        "id": "e", "layer": rect("e_l", 0.0, 0.0),
        "track": [
            { "scene": "s", "at": 0.0, "state": { "x": 800, "y": 1200 } },
            { "scene": "s", "at": 3.0, "state": { "x": 800, "y": 1200 } },
        ],
    }]);
    let p = parse(&v);
    let l = layer(&p, 60, "e_l");
    near(origin(&l).0, PIVOT.0 - 240.0, 0.05, "rolled x");
    near(origin(&l).1, PIVOT.1 + 260.0, 0.05, "rolled y");
    near(rotation_of(&l), 90.0, 0.01, "rolled rotation");
}

// ---------------------------------------------------------------------------
// 4. Glyph cascade
// ---------------------------------------------------------------------------

fn cascade(start: f64, dur: f64, stagger: f64, order: &str, seed: u32) -> Value {
    motion(
        "t",
        start,
        dur,
        json!({ "op": "glyph_cascade", "stagger": stagger, "order": order, "seed": seed,
                "from": { "dy": 40.0, "scale": 0.5, "rotation": 90.0, "opacity": 0.0 } }),
    )
}

fn poses(p: &MotionProject, f: u32) -> Option<Vec<GlyphPose>> {
    layer(p, f, "t").glyphs
}

#[test]
fn glyph_cascade_staggers_non_whitespace_glyphs() {
    // "AB CD": 4 glyphs. Run = 1.0 - 3 * 0.1 = 0.7 s each, starting every 0.1 s.
    let p = project_of(
        vec![text("t", "AB CD")],
        vec![cascade(0.5, 1.0, 0.1, "forward", 0)],
    );
    valid(&p);
    // Before the start: every glyph at `from`.
    let before = poses(&p, 0).expect("poses before start");
    assert_eq!(before.len(), 4);
    for g in &before {
        assert_eq!(g.dy, 40.0);
        assert_eq!(g.scale, 0.5);
        assert_eq!(g.opacity, 0.0);
    }
    // t = 0.85 s: elapsed 0.35, 0.25, 0.15, 0.05 of 0.7 (linear).
    let mid = poses(&p, 25).expect("poses mid cascade"); // frame 25 = 0.8333 s
    let t = 25.0 / 30.0 - 0.5;
    for (k, g) in mid.iter().enumerate() {
        let elapsed = t - 0.1 * k as f64;
        let prog = (elapsed / 0.7).clamp(0.0, 1.0) as f32;
        near(g.dy, 40.0 * (1.0 - prog), 1e-3, &format!("dy {k}"));
        near(g.scale, 0.5 + 0.5 * prog, 1e-3, &format!("scale {k}"));
        near(g.opacity, prog, 1e-3, &format!("opacity {k}"));
        near(g.rotation, 90.0 * (1.0 - prog), 1e-3, &format!("rot {k}"));
    }
    // Earlier glyphs lead later ones.
    assert!(mid[0].opacity > mid[1].opacity && mid[1].opacity > mid[3].opacity);
    // The last glyph ends at start + 3 * stagger + run = start + duration.
    assert!(
        poses(&p, 44).is_some(),
        "last glyph not yet finished at 1.466 s"
    );
    assert!(poses(&p, 45).is_none(), "everything at rest at 1.5 s");
    assert!(poses(&p, 100).is_none());
}

#[test]
fn glyph_cascade_order_changes_who_moves_first() {
    // Shortly after the start only the first-ranked glyph has moved.
    let first_moved = |order: &str, text_s: &str, seed: u32| -> Vec<usize> {
        let p = project_of(
            vec![text("t", text_s)],
            vec![cascade(0.0, 2.0, 0.2, order, seed)],
        );
        let pz = poses(&p, 3).expect("poses"); // 0.1 s: only rank 0 started
        pz.iter()
            .enumerate()
            .filter(|(_, g)| g.opacity > 0.0)
            .map(|(k, _)| k)
            .collect()
    };
    assert_eq!(first_moved("forward", "abcde", 0), vec![0]);
    assert_eq!(first_moved("backward", "abcde", 0), vec![4]);
    assert_eq!(first_moved("center", "abcde", 0), vec![2]);
    assert_eq!(
        first_moved("center", "abcd", 0),
        vec![1],
        "ties: left first"
    );
    let r1 = first_moved("random", "abcdefgh", 1);
    assert_eq!(r1, first_moved("random", "abcdefgh", 1), "seeded");
    assert_eq!(r1.len(), 1);
    assert!(
        (2u32..40).any(|s| first_moved("random", "abcdefgh", s) != r1),
        "the seed changes the shuffle"
    );
}

#[test]
fn glyph_cascade_counts_displayed_count_text_and_follows_springs() {
    let mut casc = cascade(0.0, 2.0, 0.05, "forward", 0);
    casc["spring"] = bouncy_json();
    let count = motion(
        "t",
        0.0,
        1.0,
        json!({ "op": "count", "from": 0, "to": 1234, "grouping": true }),
    );
    let p = project_of(vec![text("t", "x")], vec![casc, count]);
    // "1,234" once the count is done (1 s): 5 non-whitespace glyphs.
    let pz = poses(&p, 36).expect("poses");
    assert_eq!(pz.len(), 5);
    // Glyph 2 (rank 2): spring progress over run = 2.0 - 4 * 0.05 = 1.8.
    let elapsed = 36.0 / 30.0 - 0.1;
    let prog = spring_progress(elapsed, 1.8, BOUNCY) as f32;
    near(pz[2].dy, 40.0 * (1.0 - prog), 1e-3, "spring dy");
}

#[test]
fn glyphs_none_when_text_layer_has_no_cascade() {
    let p = project_of(vec![text("t", "plain")], vec![]);
    assert!(poses(&p, 10).is_none());
}

// ---------------------------------------------------------------------------
// 5. Path morph
// ---------------------------------------------------------------------------

fn morph(start: f64, dur: f64, to: Value) -> Value {
    motion("pl", start, dur, json!({ "op": "path_morph", "to": to }))
}

#[test]
fn path_morph_resamples_by_arc_length_and_holds_the_target() {
    let own = json!([[0, 0], [100, 0]]);
    let to = json!([[0, 0], [50, 50], [100, 0]]);
    let p = project_of(vec![polyline("pl", own)], vec![morph(0.5, 1.0, to.clone())]);
    valid(&p);
    assert!(layer(&p, 0, "pl").points.is_none(), "None before the start");
    let at = |f: u32| layer(&p, f, "pl").points.expect("morphing points");
    // At the start: own path resampled to 3 points.
    let a = at(15);
    assert_eq!(a.len(), 3);
    for (got, want) in a.iter().zip([[0.0, 0.0], [50.0, 0.0], [100.0, 0.0]]) {
        near(got[0], want[0], 1e-3, "start x");
        near(got[1], want[1], 1e-3, "start y");
    }
    // Half way (linear): own and target blended.
    let m = at(30);
    for (got, want) in m.iter().zip([[0.0, 0.0], [50.0, 25.0], [100.0, 0.0]]) {
        near(got[0], want[0], 1e-3, "mid x");
        near(got[1], want[1], 1e-3, "mid y");
    }
    // After the end: `to`, held.
    for f in [45, 46, 100] {
        let e = at(f);
        for (got, want) in e.iter().zip([[0.0, 0.0], [50.0, 50.0], [100.0, 0.0]]) {
            near(got[0], want[0], 1e-3, "end x");
            near(got[1], want[1], 1e-3, "end y");
        }
    }
    // Non-polyline layers never get points.
    assert!(
        layer(&project_of(vec![rect("a", 0.0, 0.0)], vec![]), 0, "a")
            .points
            .is_none()
    );
}

#[test]
fn path_morph_resamples_the_longer_path_down_and_up() {
    // own has 5 collinear points, to has 2: N = 5, `to` resampled evenly.
    let own = json!([[0, 0], [10, 0], [20, 0], [30, 0], [40, 0]]);
    let to = json!([[0, 100], [40, 100]]);
    let p = project_of(vec![polyline("pl", own)], vec![morph(0.0, 1.0, to)]);
    let end = layer(&p, 30, "pl").points.expect("points");
    assert_eq!(end.len(), 5);
    for (i, pt) in end.iter().enumerate() {
        near(pt[0], 10.0 * i as f32, 1e-3, "x");
        near(pt[1], 100.0, 1e-3, "y");
    }
}

// ---------------------------------------------------------------------------
// 6. Echo
// ---------------------------------------------------------------------------

fn echo_motion(target: &str, start: f64, dur: f64, count: u8, spacing: f64, decay: f64) -> Value {
    motion(
        target,
        start,
        dur,
        json!({ "op": "echo", "count": count, "spacing": spacing, "decay": decay }),
    )
}

fn echo_project() -> MotionProject {
    let mut back = rect("back", 0.0, 0.0);
    back["z_index"] = json!(-1);
    let mut front = rect("front", 0.0, 0.0);
    front["z_index"] = json!(5);
    let sibling = rect("sib", 0.0, 0.0); // same z as "box", later in the list
    project_of(
        vec![back, rect("box", 100.0, 0.0), sibling, front],
        vec![
            motion(
                "box",
                0.0,
                2.0,
                json!({ "op": "move", "from": [0, 0], "to": [600, 0] }),
            ),
            echo_motion("box", 0.5, 0.5, 3, 0.1, 0.5),
        ],
    )
}

#[test]
fn echo_ghosts_trail_behind_the_layer_with_decaying_opacity() {
    let p = echo_project();
    valid(&p);
    // Frame 21 = 0.7 s, inside [0.5, 1.0].
    let fr = frame(&p, 21);
    let ids: Vec<&str> = fr.layers.iter().map(|l| l.id).collect();
    assert_eq!(
        ids,
        ["back", "box", "box", "box", "box", "sib", "front"],
        "ghosts sit immediately behind the layer, before later siblings"
    );
    let t = 21.0 / 30.0;
    // Draw order: farthest ghost first, the layer itself last.
    let stack = &fr.layers[1..5];
    for (i, k) in [3u32, 2, 1, 0].iter().enumerate() {
        let tk = t - *k as f64 * 0.1;
        let want_x = 100.0 + 300.0 * tk as f32;
        near(stack[i].transform.e, want_x, 1e-3, &format!("x k{k}"));
        near(
            stack[i].opacity,
            0.5f32.powi(*k as i32),
            1e-6,
            &format!("opacity k{k}"),
        );
        assert_eq!(stack[i].z_index, 0);
    }
    // Outside the window: no ghosts.
    for f in [0, 14, 31, 60] {
        let n = frame(&p, f).layers.iter().filter(|l| l.id == "box").count();
        assert_eq!(n, 1, "frame {f}");
    }
}

#[test]
fn echo_skips_ghosts_from_before_the_scene_start() {
    let p = project_of(
        vec![rect("box", 0.0, 0.0)],
        vec![echo_motion("box", 0.0, 1.0, 3, 0.1, 0.5)],
    );
    let boxes = |f: u32| frame(&p, f).layers.iter().filter(|l| l.id == "box").count();
    assert_eq!(boxes(0), 1, "t = 0: every ghost is before the scene");
    assert_eq!(boxes(4), 2, "t = 0.133: only k = 1 (0.033 s) exists");
    assert_eq!(boxes(8), 3, "t = 0.267: k = 1, 2");
    assert_eq!(boxes(12), 4, "t = 0.4: all three");
}

#[test]
fn echo_copies_children_and_never_recurses() {
    let group = json!({ "id": "g", "type": "group", "children": [
        { "id": "c", "type": "rectangle", "x": 10, "width": 50, "height": 50,
          "fill": "#FFFFFF" } ] });
    let p = project_of(
        vec![group],
        vec![
            motion(
                "c",
                0.0,
                2.0,
                json!({ "op": "move", "from": [0, 0], "to": [100, 0] }),
            ),
            echo_motion("g", 0.0, 2.0, 2, 0.2, 0.5),
            // The child echoes too: its ghosts exist in the live group only.
            echo_motion("c", 0.0, 2.0, 1, 0.2, 0.5),
        ],
    );
    let fr = frame(&p, 30); // 1.0 s
    let groups: Vec<&ResolvedLayer> = fr.layers.iter().filter(|l| l.id == "g").collect();
    assert_eq!(groups.len(), 3, "2 group ghosts + the group");
    // Live group (last): child ghost + child. Ghost groups: just the child.
    assert_eq!(groups[2].children.len(), 2);
    assert_eq!(groups[2].children[0].opacity, 0.5);
    assert_eq!(groups[2].children[1].opacity, 1.0);
    assert_eq!(groups[0].children.len(), 1);
    assert_eq!(groups[1].children.len(), 1);
    // Ghost groups carry the decayed opacity; their child is resolved in the past.
    near(groups[0].opacity, 0.25, 1e-6, "k=2 group opacity");
    near(groups[1].opacity, 0.5, 1e-6, "k=1 group opacity");
    near(
        groups[1].children[0].transform.e,
        10.0 + 100.0 * 0.8 / 2.0,
        1e-3,
        "ghost child position at t - 0.2",
    );
}

// ---------------------------------------------------------------------------
// 7. Pulse
// ---------------------------------------------------------------------------

fn pulse_project(envelope_id: &str) -> MotionProject {
    // Envelope: 6 samples per second; sample i covers frames 5i..5i+4.
    let values: Vec<f64> = vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.2, 0.0, 0.8, 0.4, 0.6];
    let v = project_json(
        vec![scene_json(
            "s",
            0.5, // scene starts at project time 0.5 s (frame 15)
            3.0,
            vec![rect("a", 0.0, 0.0)],
            vec![
                motion(
                    "a",
                    0.0,
                    2.0,
                    json!({ "op": "scale", "from": 1.0, "to": 2.0 }),
                ),
                motion(
                    "a",
                    0.4,
                    1.0,
                    json!({ "op": "pulse", "envelope": envelope_id, "gain": 0.5 }),
                ),
            ],
        )],
        json!([{ "id": "kick", "fps": 6.0, "values": values }]),
    );
    parse(&v)
}

#[test]
fn pulse_samples_the_envelope_at_project_time_inside_its_window() {
    let p = pulse_project("kick");
    valid(&p);
    let scale = |f: u32| layer(&p, f, "a").transform.a;
    // Window = scene-local [0.4, 1.4] = project [0.9, 1.9] = frames 27..=57
    // (sampled away from the exact edges and sample boundaries).
    // Project frame -> envelope sample = frame / 5 (fps 6 -> 5 frames each).
    let values = [
        0.0f32, 0.0, 0.0, 0.0, 0.0, 1.0, 0.5, 0.2, 0.0, 0.8, 0.4, 0.6,
    ];
    for f in [28u32, 29, 32, 34, 37, 42, 47, 52, 56] {
        let local = f as f64 / 30.0 - 0.5;
        let base = 1.0 + (local / 2.0) as f32;
        let env = values[(f / 5) as usize];
        near(
            scale(f),
            base * (1.0 + 0.5 * env),
            1e-4,
            &format!("frame {f}"),
        );
    }
    // The pulse really fires (env 1.0 at frames 25..29 -> in window from 27).
    assert!(scale(28) > scale(26) + 0.3);
    // Outside the window: the plain scale channel.
    for f in [15u32, 20, 26, 58, 80] {
        let local = f as f64 / 30.0 - 0.5;
        let base = (1.0 + (local / 2.0).min(1.0)) as f32;
        near(scale(f), base, 1e-4, &format!("frame {f}"));
    }
    // Both axes.
    let l = layer(&p, 28, "a");
    near(l.transform.a, l.transform.d, 1e-6, "both axes");
}

#[test]
fn pulse_with_an_unknown_envelope_is_a_no_op() {
    let known = pulse_project("kick");
    let unknown = pulse_project("missing");
    let plain = {
        let v = project_json(
            vec![scene_json(
                "s",
                0.5,
                3.0,
                vec![rect("a", 0.0, 0.0)],
                vec![motion(
                    "a",
                    0.0,
                    2.0,
                    json!({ "op": "scale", "from": 1.0, "to": 2.0 }),
                )],
            )],
            json!([]),
        );
        parse(&v)
    };
    for f in [20u32, 28, 33, 45] {
        assert_eq!(layer(&unknown, f, "a"), layer(&plain, f, "a"), "frame {f}");
    }
    assert_ne!(layer(&known, 28, "a"), layer(&plain, 28, "a"));
}

// ---------------------------------------------------------------------------
// 8. Post
// ---------------------------------------------------------------------------

fn post_json(effect: Value, start: f64, dur: f64, from: f64, to: f64) -> Value {
    let mut e = json!({ "start": start, "duration": dur, "easing": "linear",
                        "from": from, "to": to });
    for (k, v) in effect.as_object().expect("effect is an object") {
        e[k] = v.clone();
    }
    e
}

#[test]
fn post_strength_is_windowed_and_persistent_with_zero_duration() {
    let mut sc = scene_json("s", 0.0, 4.0, vec![rect("a", 0.0, 0.0)], vec![]);
    sc["post"] = json!([
        post_json(
            json!({ "effect": "vignette", "amount": 0.6 }),
            0.5,
            1.0,
            0.0,
            0.8
        ),
        post_json(
            json!({ "effect": "grain", "amount": 0.3, "seed": 9 }),
            1.0,
            0.0,
            0.6,
            0.6
        ),
    ]);
    let p = parse(&project_json(vec![sc], json!([])));
    valid(&p);
    let post = |f: u32| frame(&p, f).post;

    // Frame 0: nothing has started.
    assert!(post(0).is_empty());
    // Frame 30 (1.0 s): vignette half way = 0.4; persistent grain starts at 0.6.
    let f30 = post(30);
    assert_eq!(f30.len(), 2, "scene order, then list order");
    assert!(matches!(f30[0].kind, PostKind::Vignette { .. }));
    near(f30[0].strength, 0.4, 1e-6, "vignette mid");
    assert!(matches!(f30[1].kind, PostKind::Grain { .. }));
    near(f30[1].strength, 0.6, 1e-6, "grain start");
    assert_eq!(f30[1].time, 1.0);
    // Frame 60 (2.0 s): the vignette window has ended; grain persists.
    let f60 = post(60);
    assert_eq!(f60.len(), 1);
    assert!(matches!(f60[0].kind, PostKind::Grain { .. }));
    assert_eq!(post(110).len(), 1, "grain persists to the scene end");
}

#[test]
fn post_spans_active_scenes_in_order_and_uses_project_time() {
    let mut a = scene_json("a", 0.0, 2.0, vec![], vec![]);
    a["post"] = json!([post_json(
        json!({ "effect": "vignette", "amount": 0.5 }),
        0.0,
        0.0,
        1.0,
        1.0
    )]);
    let mut b = scene_json("b", 1.0, 2.0, vec![], vec![]);
    b["post"] = json!([post_json(
        json!({ "effect": "glitch", "bands": 4, "max_shift": 20.0, "seed": 1 }),
        0.0,
        1.0,
        0.0,
        1.0
    )]);
    let p = parse(&project_json(vec![a, b], json!([])));
    let fr = frame(&p, 45); // 1.5 s: both active
    assert_eq!(fr.post.len(), 2);
    assert!(matches!(fr.post[0].kind, PostKind::Vignette { .. }));
    assert!(matches!(fr.post[1].kind, PostKind::Glitch { .. }));
    near(fr.post[0].strength, 1.0, 1e-6, "zero-duration holds `to`");
    near(fr.post[1].strength, 0.5, 1e-6, "scene b local 0.5 s");
    assert_eq!(fr.post[1].time, 1.5, "project time, not scene-local");
}

// ---------------------------------------------------------------------------
// Determinism and regressions
// ---------------------------------------------------------------------------

fn kitchen_sink() -> MotionProject {
    let mut sc = scene_json(
        "s",
        0.0,
        4.0,
        vec![
            text("t", "HELLO WORLD"),
            polyline("pl", json!([[0, 0], [100, 80], [200, 0]])),
            rect("a", 100.0, 200.0),
        ],
        vec![
            cascade(0.0, 1.5, 0.05, "random", 11),
            morph(0.2, 1.0, json!([[0, 0], [50, 90], [150, 90], [200, 0]])),
            motion("a", 0.0, 3.0, shake_op([8.0, 8.0], 2.0, 6.0, 0.8, 4)),
            echo_motion("a", 0.5, 2.0, 3, 0.08, 0.6),
            {
                let mut m = motion(
                    "a",
                    0.0,
                    1.2,
                    json!({ "op": "move", "from": [0, 0], "to": [300, 100] }),
                );
                m["spring"] = bouncy_json();
                m
            },
            motion(
                "a",
                0.0,
                3.0,
                json!({ "op": "pulse", "envelope": "kick", "gain": 0.3 }),
            ),
        ],
    );
    sc["camera"] = json!({ "motions": [
        cam_motion(0.0, 3.0, json!({ "op": "roll", "from": 0, "to": 10 })),
        cam_motion(0.5, 1.0, json!({ "op": "shake", "trauma": 0.7, "frequency": 8.0,
                                     "decay": 1.0, "seed": 21 })),
    ] });
    sc["post"] = json!([post_json(
        json!({ "effect": "vignette", "amount": 0.4 }),
        0.0,
        2.0,
        0.0,
        1.0
    )]);
    parse(&project_json(
        vec![sc],
        json!([{ "id": "kick", "fps": 12.0, "values": [0.0, 0.5, 1.0, 0.25, 0.75, 0.0] }]),
    ))
}

#[test]
fn evaluation_is_deterministic_and_order_independent() {
    let p = kitchen_sink();
    valid(&p);
    let frames = [0u32, 7, 15, 22, 30, 41, 60, 89, 119];
    let first: Vec<_> = frames.iter().map(|f| frame(&p, *f)).collect();
    // Same frame twice, and in reverse order (no accumulated state).
    for (f, a) in frames.iter().zip(&first).rev() {
        assert_eq!(&frame(&p, *f), a, "frame {f}");
    }
    // A reparsed copy resolves identically.
    let q = parse(&serde_json::to_value(&p).expect("serializes"));
    for (f, a) in frames.iter().zip(&first) {
        assert_eq!(&frame(&q, *f), a, "reparsed frame {f}");
    }
}

#[test]
fn inert_new_fields_leave_a_scene_untouched() {
    // Base scene with ordinary motions and a camera push.
    let layers = || vec![rect("a", 100.0, 200.0), text("t", "Hello there")];
    let base_motions = || {
        vec![
            motion(
                "a",
                0.0,
                1.0,
                json!({ "op": "move", "from": [0, 0], "to": [100, 50] }),
            ),
            motion(
                "a",
                0.0,
                1.0,
                json!({ "op": "fade", "from": 0.2, "to": 1.0 }),
            ),
        ]
    };
    let make = |extra: Vec<Value>, camera_extra: Vec<Value>, post: Value| {
        let mut motions = base_motions();
        motions.extend(extra);
        let mut sc = scene_json("s", 0.0, 4.0, layers(), motions);
        let mut cam = vec![cam_motion(
            0.0,
            2.0,
            json!({ "op": "push", "from": 1.0, "to": 1.2 }),
        )];
        cam.extend(camera_extra);
        sc["camera"] = json!({ "motions": cam });
        sc["post"] = post;
        parse(&project_json(vec![sc], json!([])))
    };
    let plain = make(vec![], vec![], json!([]));
    // New ops that are inactive at the sampled frames, or no-ops by contract.
    let inert = make(
        vec![
            motion("a", 3.0, 0.5, shake_op([20.0, 20.0], 9.0, 5.0, 0.0, 1)),
            echo_motion("a", 3.0, 0.5, 2, 0.1, 0.5),
            motion(
                "a",
                0.0,
                2.0,
                json!({ "op": "pulse", "envelope": "nope", "gain": 0.5 }),
            ),
            // Finished cascade: all glyphs at rest -> glyphs stay None.
            motion(
                "t",
                0.0,
                0.5,
                json!({ "op": "glyph_cascade", "stagger": 0.02,
                        "from": { "dy": 30.0, "opacity": 0.0 } }),
            ),
        ],
        vec![
            cam_motion(
                3.0,
                0.5,
                json!({ "op": "shake", "trauma": 1.0, "frequency": 6.0 }),
            ),
            cam_motion(3.0, 0.5, json!({ "op": "roll", "from": 0, "to": 0 })),
        ],
        json!([post_json(
            json!({ "effect": "vignette", "amount": 0.5 }),
            0.0,
            1.0,
            0.0,
            0.0
        )]),
    );
    for f in [16u32, 20, 30, 45, 61, 80] {
        let a = frame(&plain, f);
        let b = frame(&inert, f);
        assert_eq!(a.layers, b.layers, "frame {f}");
        assert_eq!(a.layers.len(), 2);
        assert!(b
            .layers
            .iter()
            .all(|l| l.glyphs.is_none() && l.points.is_none()));
        assert!(b.post.is_empty());
        assert!(a.post.is_empty());
    }
}

#[test]
fn plain_scenes_have_no_new_outputs() {
    let p = project_of(
        vec![
            rect("a", 0.0, 0.0),
            text("t", "abc"),
            polyline("pl", json!([[0, 0], [10, 10]])),
        ],
        vec![motion(
            "a",
            0.0,
            1.0,
            json!({ "op": "move", "from": [0, 0], "to": [10, 10] }),
        )],
    );
    for f in [0u32, 15, 30, 100] {
        let fr = frame(&p, f);
        assert!(fr.post.is_empty());
        for l in &fr.layers {
            assert!(l.glyphs.is_none() && l.points.is_none(), "{} f{f}", l.id);
        }
    }
    let json = serde_json::to_string(&frame(&p, 10)).expect("serializes");
    assert!(!json.contains("glyphs"));
    assert!(!json.contains("\"post\""));
}
