//! The `tint` motion channel: timeline resolution (scene layers, groups,
//! shared elements), validation, and serde.
//!
//! Canvas 1000x1000 at 30 fps (frame f is at t = f / 30 s exactly).

use motion_core::scene::{Color, MotionOp, MotionProject};
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer, ResolvedTint};
use motion_core::validate::{validate, ValidationErrors};
use serde_json::{json, Value};

fn tint_m(target: &str, start: f64, dur: f64, color: &str, from: f64, to: f64) -> Value {
    json!({ "target": target, "start": start, "duration": dur, "easing": "linear",
            "op": "tint", "color": color, "from": from, "to": to })
}

fn fade_m(target: &str, start: f64, dur: f64, from: f64, to: f64) -> Value {
    json!({ "target": target, "start": start, "duration": dur, "easing": "linear",
            "op": "fade", "from": from, "to": to })
}

fn rect(id: &str) -> Value {
    json!({ "id": id, "type": "rectangle", "width": 100, "height": 100, "fill": "#336699" })
}

fn project_json(scenes: Vec<Value>, shared: Vec<Value>) -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "tint" },
        "canvas": { "width": 1000, "height": 1000, "fps": 30, "background": "#000000" },
        "theme": { "fonts": {} },
        "assets": [],
        "scenes": scenes,
        "shared": shared,
    })
}

fn scene(id: &str, start: f64, dur: f64, layers: Vec<Value>, motions: Vec<Value>) -> Value {
    json!({ "id": id, "start_seconds": start, "duration_seconds": dur,
            "layers": layers, "motions": motions })
}

fn parse(v: &Value) -> MotionProject {
    MotionProject::from_json(&v.to_string()).expect("test project parses")
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

fn find<'a, 'p>(layers: &'a [ResolvedLayer<'p>], id: &str) -> Option<&'a ResolvedLayer<'p>> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let Some(c) = find(&l.children, id) {
            return Some(c);
        }
    }
    None
}

fn tint_of(p: &MotionProject, f: u32, id: &str) -> Option<ResolvedTint> {
    let fr = frame(p, f);
    find(&fr.layers, id)
        .unwrap_or_else(|| panic!("layer '{id}' not visible at frame {f}"))
        .tint
}

fn near(a: f32, b: f32, what: &str) {
    assert!((a - b).abs() < 1e-4, "{what}: {a} vs {b}");
}

const RED: Color = Color::rgb(255, 0, 0);
const BLUE: Color = Color::rgb(0, 0, 255);

// ---------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------

fn single() -> MotionProject {
    // One tint motion 1.0 s .. 2.0 s, amount 0.2 -> 0.8, toward red.
    parse(&project_json(
        vec![scene(
            "s",
            0.0,
            4.0,
            vec![rect("a")],
            vec![tint_m("a", 1.0, 1.0, "#FF0000", 0.2, 0.8)],
        )],
        vec![],
    ))
}

#[test]
fn tint_holds_from_before_start_eases_and_holds_to_after_end() {
    let p = single();
    let before = tint_of(&p, 0, "a").expect("from holds before start");
    near(before.amount, 0.2, "before");
    assert_eq!(before.color, RED);
    let mid = tint_of(&p, 45, "a").expect("mid");
    near(mid.amount, 0.5, "mid");
    let after = tint_of(&p, 100, "a").expect("after");
    near(after.amount, 0.8, "after");
    assert_eq!(after.color, RED);
}

#[test]
fn no_tint_motion_or_negligible_amount_resolves_to_none() {
    let p = parse(&project_json(
        vec![scene(
            "s",
            0.0,
            4.0,
            vec![rect("a"), rect("b")],
            vec![tint_m("b", 0.0, 1.0, "#FF0000", 0.0, 0.0)],
        )],
        vec![],
    ));
    assert_eq!(tint_of(&p, 10, "a"), None);
    assert_eq!(tint_of(&p, 10, "b"), None);
}

#[test]
fn sequential_tints_switch_colour_at_the_second_start() {
    let p = parse(&project_json(
        vec![scene(
            "s",
            0.0,
            5.0,
            vec![rect("a")],
            vec![
                tint_m("a", 0.0, 1.0, "#FF0000", 0.0, 0.5),
                tint_m("a", 2.0, 1.0, "#0000FF", 0.5, 1.0),
            ],
        )],
        vec![],
    ));
    // Between the two: first motion's end value and colour hold.
    let gap = tint_of(&p, 45, "a").expect("gap");
    assert_eq!(gap.color, RED);
    near(gap.amount, 0.5, "gap");
    // Just before the second starts (frame 59 = 1.9667 s).
    assert_eq!(tint_of(&p, 59, "a").expect("pre").color, RED);
    // At the second's start it switches.
    let start = tint_of(&p, 60, "a").expect("start");
    assert_eq!(start.color, BLUE);
    near(start.amount, 0.5, "start");
    let end = tint_of(&p, 120, "a").expect("end");
    assert_eq!(end.color, BLUE);
    near(end.amount, 1.0, "end");
}

#[test]
fn group_tint_is_inherited_unless_the_child_has_its_own() {
    let group = json!({
        "id": "g", "type": "group", "width": 500, "height": 500,
        "children": [ rect("c1"), rect("c2") ],
    });
    let p = parse(&project_json(
        vec![scene(
            "s",
            0.0,
            4.0,
            vec![group],
            vec![
                tint_m("g", 0.0, 1.0, "#FF0000", 0.4, 0.4),
                tint_m("c2", 0.0, 1.0, "#0000FF", 0.9, 0.9),
            ],
        )],
        vec![],
    ));
    let c1 = tint_of(&p, 10, "c1").expect("inherits");
    assert_eq!(c1.color, RED);
    near(c1.amount, 0.4, "c1");
    let c2 = tint_of(&p, 10, "c2").expect("own");
    assert_eq!(c2.color, BLUE);
    near(c2.amount, 0.9, "c2");
}

fn shared_project(motions: Vec<Value>) -> MotionProject {
    parse(&project_json(
        vec![
            scene("s1", 0.0, 3.0, vec![rect("bg1")], motions),
            scene("s2", 3.0, 3.0, vec![rect("bg2")], vec![]),
        ],
        vec![json!({
            "id": "el",
            "layer": rect("el_l"),
            "track": [
                { "scene": "s1", "at": 0.0, "easing": "linear", "state": { "x": 100, "y": 100 } },
                { "scene": "s2", "at": 2.0, "easing": "linear", "state": { "x": 200, "y": 200 } },
            ],
        })],
    ))
}

#[test]
fn tint_on_a_shared_element_applies_only_while_its_scene_is_active() {
    let p = shared_project(vec![tint_m("el_l", 0.5, 1.0, "#FF0000", 0.0, 1.0)]);
    // Before start: `from` (0) holds -> none.
    assert_eq!(tint_of(&p, 0, "el_l"), None);
    // Midway through the motion.
    let mid = tint_of(&p, 30, "el_l").expect("mid");
    near(mid.amount, 0.5, "mid");
    assert_eq!(mid.color, RED);
    // After the motion, still inside s1: holds `to`.
    near(tint_of(&p, 80, "el_l").expect("hold").amount, 1.0, "hold");
    // In s2 (no tint motion there) the element is untinted again.
    assert_eq!(tint_of(&p, 100, "el_l"), None);
}

#[test]
fn shared_tint_composes_with_fade() {
    let p = shared_project(vec![
        tint_m("el_l", 0.0, 1.0, "#0000FF", 0.5, 0.5),
        fade_m("el_l", 0.0, 1.0, 0.5, 0.5),
    ]);
    let fr = frame(&p, 10);
    let l = find(&fr.layers, "el_l").expect("visible");
    near(l.opacity, 0.5, "opacity");
    near(l.tint.expect("tint").amount, 0.5, "tint");
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn one_scene(motions: Vec<Value>) -> Value {
    project_json(vec![scene("s", 0.0, 4.0, vec![rect("a")], motions)], vec![])
}

#[test]
fn tint_amounts_must_be_within_zero_and_one() {
    ok(&one_scene(vec![tint_m("a", 0.0, 1.0, "#FF0000", 0.0, 1.0)]));
    for (from, to) in [(-0.1, 0.5), (0.5, 1.2)] {
        let e = errors_of(&one_scene(vec![tint_m("a", 0.0, 1.0, "#FF0000", from, to)]));
        assert!(
            e.to_string().contains("tint"),
            "expected a tint range error, got:\n{e}"
        );
    }
}

#[test]
fn overlapping_tint_motions_are_rejected() {
    let e = errors_of(&one_scene(vec![
        tint_m("a", 0.0, 2.0, "#FF0000", 0.0, 1.0),
        tint_m("a", 1.0, 2.0, "#0000FF", 0.0, 1.0),
    ]));
    assert!(e.to_string().contains("conflicting"), "{e}");
    // Back to back is fine; other channels don't conflict.
    ok(&one_scene(vec![
        tint_m("a", 0.0, 1.0, "#FF0000", 0.0, 1.0),
        tint_m("a", 1.0, 1.0, "#0000FF", 1.0, 0.0),
        fade_m("a", 0.0, 2.0, 1.0, 0.5),
    ]));
}

#[test]
fn tint_is_valid_on_every_layer_kind_and_on_shared_elements() {
    ok(&project_json(
        vec![scene(
            "s",
            0.0,
            4.0,
            vec![
                rect("a"),
                json!({ "id": "g", "type": "group", "width": 10, "height": 10, "children": [] }),
                json!({ "id": "p", "type": "polyline", "width": 100, "height": 100,
                        "points": [[0, 0], [10, 10]], "stroke": { "color": "#000000", "width": 2 } }),
            ],
            vec![
                tint_m("a", 0.0, 1.0, "#FF0000", 0.0, 1.0),
                tint_m("g", 0.0, 1.0, "#FF0000", 0.0, 1.0),
                tint_m("p", 0.0, 1.0, "#FF0000", 0.0, 1.0),
            ],
        )],
        vec![],
    ));
    let mut shared = project_json(
        vec![scene(
            "s1",
            0.0,
            3.0,
            vec![rect("bg1")],
            vec![tint_m("el_l", 0.0, 1.0, "#FF0000", 0.0, 1.0)],
        )],
        vec![json!({
            "id": "el", "layer": rect("el_l"),
            "track": [ { "scene": "s1", "at": 0.0, "easing": "linear", "state": { "x": 1, "y": 1 } } ],
        })],
    );
    ok(&shared);
    // ...but not outside the scenes where the element lives.
    shared["scenes"].as_array_mut().expect("scenes").push(scene(
        "s2",
        3.0,
        2.0,
        vec![rect("bg2")],
        vec![tint_m("el_l", 0.0, 1.0, "#FF0000", 0.0, 1.0)],
    ));
    assert!(errors_of(&shared).to_string().contains("no track key"));
}

// ---------------------------------------------------------------------------
// Serde
// ---------------------------------------------------------------------------

#[test]
fn tint_motion_round_trips() {
    let p = single();
    let m = &p.scenes[0].motions[0];
    assert!(matches!(
        m.op,
        MotionOp::Tint { color, from, to } if color == RED && from == 0.2 && to == 0.8
    ));
    assert_eq!(m.op.op_name(), "tint");
    let text = serde_json::to_string(&p.scenes[0].motions[0]).expect("serialize");
    let v: Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["op"], "tint");
    assert_eq!(v["color"], "#FF0000");
    let back = MotionProject::from_json(&serde_json::to_string(&p).expect("ser")).expect("parse");
    assert_eq!(back, p);
}

#[test]
fn projects_without_tint_serialize_identically() {
    // Compiler-emitted goldens are written by `to_json_pretty`: parsing and
    // re-serializing must reproduce them byte for byte (no tint noise).
    let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden");
    for name in ["editorial_collage.motion.json", "hero_object.motion.json"] {
        let text = std::fs::read_to_string(golden.join(name)).expect("read golden");
        let p = MotionProject::from_json(&text).expect("parse golden");
        assert!(!text.contains("\"tint\""));
        assert_eq!(
            p.to_json_pretty().trim_end(),
            text.trim_end(),
            "{name} round-trips unchanged"
        );
        // The resolved frame does not mention tint either.
        let fr = evaluate_frame(&p, 10).expect("frame");
        let resolved = serde_json::to_string(&fr).expect("ser frame");
        assert!(!resolved.contains("tint"), "{name}");
    }
}
