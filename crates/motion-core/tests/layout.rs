//! Parent-relative layout binding (`Layer.layout`, `TrackKey.layout`).
//!
//! Scenes are 4 s @ 30 fps; the animations under test run 0..2 s, so frames
//! 0, 15, 30, 45, 60 are 0/25/50/75/100 % of the animation.

use motion_core::scene::MotionProject;
use motion_core::timeline::{evaluate_frame, ResolvedFrame, ResolvedLayer};
use serde_json::{json, Value};

const FRAMES: [u32; 5] = [0, 15, 30, 45, 60];
const TOL: f32 = 0.05;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn project_with(scenes: Value, shared: Value) -> MotionProject {
    let v = json!({
        "version": "0.2",
        "project": { "name": "layout" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#000000" },
        "scenes": scenes,
        "shared": shared,
    });
    MotionProject::from_json(&v.to_string()).expect("test project parses")
}

fn project(layers: Vec<Value>, motions: Vec<Value>) -> MotionProject {
    project_with(scene(layers, motions), json!([]))
}

fn scene(layers: Vec<Value>, motions: Vec<Value>) -> Value {
    json!([{ "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
             "layers": layers, "motions": motions }])
}

/// Parent card: `(x, y, w, h)` base geometry.
fn card(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "id": "card", "type": "rectangle", "x": x, "y": y, "width": w, "height": h,
            "fill": "#FFFFFF" })
}

/// Bound text child (300x120 box, ink 30..90). `x`/`y` are deliberately wrong
/// to prove they are ignored when bound.
fn text_child(binding: Value) -> Value {
    json!({ "id": "label", "type": "text", "text": "Hi", "font_role": "display",
            "font_size": 80, "color": "#FFFFFF", "x": 999, "y": 999,
            "width": 300, "height": 120, "ink": { "top": 30, "bottom": 90 },
            "layout": binding })
}

fn bind(h: &str, v: &str) -> Value {
    json!({ "parent": "card", "horizontal": h, "vertical": v })
}

fn center() -> Value {
    bind("center", "center")
}

fn motion(target: &str, dur: f64, op: Value) -> Value {
    let mut m = json!({ "target": target, "start": 0.0, "duration": dur, "easing": "linear" });
    for (k, v) in op.as_object().expect("op object") {
        m[k] = v.clone();
    }
    m
}

fn expand(to: (f64, f64, f64, f64)) -> Value {
    motion(
        "card",
        2.0,
        json!({ "op": "accent_expand",
                "to": { "x": to.0, "y": to.1, "width": to.2, "height": to.3 } }),
    )
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

fn close(a: (f32, f32), b: (f32, f32), what: &str) {
    assert!(
        (a.0 - b.0).abs() < TOL && (a.1 - b.1).abs() < TOL,
        "{what}: got {a:?}, expected {b:?}"
    );
}

/// Canvas position of the child's alignment point (text uses ink bounds).
fn child_point(c: &ResolvedLayer, hf: f32, vf: f32, ink: Option<(f32, f32)>) -> (f32, f32) {
    let ay = match (ink, vf) {
        (Some((t, _)), 0.0) => t,
        (Some((t, b)), 0.5) => (t + b) * 0.5,
        (Some((_, b)), _) => b,
        (None, v) => v * c.height,
    };
    c.transform.apply(hf * c.width, ay)
}

/// Canvas position of the parent's content-box point.
fn parent_point(p: &ResolvedLayer, hf: f32, vf: f32, pad: [f32; 4], off: (f32, f32)) -> (f32, f32) {
    let [l, t, r, b] = pad;
    p.transform.apply(
        l + hf * (p.width - l - r) + off.0,
        t + vf * (p.height - t - b) + off.1,
    )
}

const INK: Option<(f32, f32)> = Some((30.0, 90.0));

// ---------------------------------------------------------------------------
// Alignment against a geometry-animated parent
// ---------------------------------------------------------------------------

#[test]
fn ink_center_tracks_parent_center_while_height_expands() {
    let p = project(
        vec![card(100.0, 200.0, 400.0, 100.0), text_child(center())],
        vec![expand((100.0, 200.0, 400.0, 500.0))],
    );
    for (i, frame) in FRAMES.iter().enumerate() {
        let f = evaluate_frame(&p, *frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        let expect_h = 100.0 + 400.0 * (i as f32 * 0.25);
        assert!((card.height - expect_h).abs() < 1e-3);
        // Hard numbers: label is unrotated, unscaled, so ink center is origin + 60.
        let ink_center_y = label.transform.apply(0.0, 60.0).1;
        assert!(
            (ink_center_y - (200.0 + expect_h / 2.0)).abs() < 1.0,
            "frame {frame}: ink center {ink_center_y} vs {}",
            200.0 + expect_h / 2.0
        );
        let ink_center_x = label.transform.apply(150.0, 0.0).0;
        assert!((ink_center_x - 300.0).abs() < 1.0);
        close(
            child_point(&label, 0.5, 0.5, INK),
            parent_point(&card, 0.5, 0.5, [0.0; 4], (0.0, 0.0)),
            "center",
        );
    }
}

#[test]
fn every_anchor_combination_hits_the_matching_parent_edge() {
    for h in ["left", "center", "right"] {
        for v in ["top", "center", "bottom"] {
            let hf = match h {
                "left" => 0.0,
                "center" => 0.5,
                _ => 1.0,
            };
            let vf = match v {
                "top" => 0.0,
                "center" => 0.5,
                _ => 1.0,
            };
            let p = project(
                vec![card(100.0, 200.0, 400.0, 100.0), text_child(bind(h, v))],
                vec![expand((100.0, 200.0, 400.0, 500.0))],
            );
            for frame in FRAMES {
                let f = evaluate_frame(&p, frame).unwrap();
                let (card, label) = (get(&f, "card"), get(&f, "label"));
                close(
                    child_point(&label, hf, vf, INK),
                    parent_point(&card, hf, vf, [0.0; 4], (0.0, 0.0)),
                    &format!("{h}/{v} frame {frame}"),
                );
            }
        }
    }
}

#[test]
fn top_and_bottom_use_ink_edges_for_text() {
    let p = project(
        vec![
            card(100.0, 200.0, 400.0, 300.0),
            text_child(bind("left", "top")),
        ],
        vec![],
    );
    let f = evaluate_frame(&p, 0).unwrap();
    let label = get(&f, "label");
    // ink top (y=30 in the box) lands on the parent's top edge (y=200).
    assert!((label.transform.apply(0.0, 30.0).1 - 200.0).abs() < 1e-3);
    assert!((label.transform.apply(0.0, 0.0).0 - 100.0).abs() < 1e-3);

    let p = project(
        vec![
            card(100.0, 200.0, 400.0, 300.0),
            text_child(bind("right", "bottom")),
        ],
        vec![],
    );
    let f = evaluate_frame(&p, 0).unwrap();
    let label = get(&f, "label");
    assert!((label.transform.apply(0.0, 90.0).1 - 500.0).abs() < 1e-3);
    assert!((label.transform.apply(300.0, 0.0).0 - 500.0).abs() < 1e-3);
}

#[test]
fn text_without_ink_uses_the_box() {
    let mut l = text_child(center());
    l.as_object_mut().unwrap().remove("ink");
    let p = project(vec![card(100.0, 200.0, 400.0, 300.0), l], vec![]);
    let f = evaluate_frame(&p, 0).unwrap();
    let label = get(&f, "label");
    // Box center (150, 60) lands on the card center (300, 350).
    close(
        label.transform.apply(150.0, 60.0),
        (300.0, 350.0),
        "box center",
    );
}

#[test]
fn changing_parent_width_replaces_child_without_resizing_it() {
    let p = project(
        vec![
            card(100.0, 200.0, 200.0, 300.0),
            text_child(bind("right", "center")),
        ],
        vec![expand((100.0, 200.0, 800.0, 300.0))],
    );
    for frame in FRAMES {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        close(
            child_point(&label, 1.0, 0.5, INK),
            parent_point(&card, 1.0, 0.5, [0.0; 4], (0.0, 0.0)),
            &format!("frame {frame}"),
        );
        // Right edge of the child's box sits on the parent's right edge.
        assert!((label.transform.apply(300.0, 0.0).0 - (100.0 + card.width)).abs() < 1.0);
        assert_eq!((label.width, label.height), (300.0, 120.0));
        assert!(
            (label.transform.a - 1.0).abs() < 1e-5,
            "child is not resized"
        );
    }
}

// ---------------------------------------------------------------------------
// Parent transforms propagate
// ---------------------------------------------------------------------------

#[test]
fn moving_parent_moves_child() {
    let p = project(
        vec![card(100.0, 200.0, 400.0, 300.0), text_child(center())],
        vec![motion(
            "card",
            2.0,
            json!({ "op": "move", "from": [0, 0], "to": [200, 100] }),
        )],
    );
    for frame in FRAMES {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        close(
            child_point(&label, 0.5, 0.5, INK),
            parent_point(&card, 0.5, 0.5, [0.0; 4], (0.0, 0.0)),
            &format!("frame {frame}"),
        );
    }
    let f = evaluate_frame(&p, 60).unwrap();
    close(
        get(&f, "label").transform.apply(150.0, 60.0),
        (100.0 + 200.0 + 200.0, 200.0 + 100.0 + 150.0),
        "moved center",
    );
}

#[test]
fn scaling_parent_scales_child_and_keeps_it_centered() {
    let p = project(
        vec![card(100.0, 200.0, 400.0, 300.0), text_child(center())],
        vec![motion(
            "card",
            2.0,
            json!({ "op": "scale", "from": 1.0, "to": 2.0 }),
        )],
    );
    for (i, frame) in FRAMES.iter().enumerate() {
        let f = evaluate_frame(&p, *frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        let s = 1.0 + i as f32 * 0.25;
        assert!((label.transform.a - s).abs() < 1e-4, "child scale {s}");
        assert!((label.transform.d - s).abs() < 1e-4);
        close(
            child_point(&label, 0.5, 0.5, INK),
            parent_point(&card, 0.5, 0.5, [0.0; 4], (0.0, 0.0)),
            &format!("frame {frame}"),
        );
    }
}

#[test]
fn rotating_parent_rotates_child() {
    let p = project(
        vec![
            card(300.0, 400.0, 400.0, 300.0),
            text_child(bind("left", "top")),
        ],
        vec![motion(
            "card",
            2.0,
            json!({ "op": "rotate", "from": 0.0, "to": 90.0 }),
        )],
    );
    for frame in FRAMES {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        close(
            child_point(&label, 0.0, 0.0, INK),
            parent_point(&card, 0.0, 0.0, [0.0; 4], (0.0, 0.0)),
            &format!("frame {frame}"),
        );
        assert!((label.transform.a - card.transform.a).abs() < 1e-4);
        assert!((label.transform.b - card.transform.b).abs() < 1e-4);
    }
    // After 90 degrees the child's box x axis points down the canvas.
    let f = evaluate_frame(&p, 60).unwrap();
    let label = get(&f, "label");
    assert!(label.transform.a.abs() < 1e-4 && (label.transform.b - 1.0).abs() < 1e-4);
}

// ---------------------------------------------------------------------------
// Padding, offset, combinations
// ---------------------------------------------------------------------------

#[test]
fn asymmetric_padding_shrinks_the_content_box() {
    let mut b = center();
    b["padding"] = json!({ "left": 10, "top": 20, "right": 50, "bottom": 30 });
    let p = project(
        vec![card(100.0, 200.0, 400.0, 100.0), text_child(b)],
        vec![expand((100.0, 200.0, 400.0, 500.0))],
    );
    for frame in FRAMES {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        let pad = [10.0, 20.0, 50.0, 30.0];
        let want = parent_point(&card, 0.5, 0.5, pad, (0.0, 0.0));
        close(child_point(&label, 0.5, 0.5, INK), want, "padded center");
        // Hard numbers: x = 100 + 10 + (400-60)/2, y = 200 + 20 + (h-50)/2.
        assert!((want.0 - 280.0).abs() < 1e-3);
        assert!((want.1 - (220.0 + (card.height - 50.0) / 2.0)).abs() < 1e-3);
    }
    // Bottom/right anchors respect the padding too.
    let mut b = bind("right", "bottom");
    b["padding"] = json!({ "left": 10, "top": 20, "right": 50, "bottom": 30 });
    let p = project(
        vec![card(100.0, 200.0, 400.0, 300.0), text_child(b)],
        vec![],
    );
    let f = evaluate_frame(&p, 0).unwrap();
    let label = get(&f, "label");
    close(
        child_point(&label, 1.0, 1.0, INK),
        (100.0 + 400.0 - 50.0, 200.0 + 300.0 - 30.0),
        "padded bottom right",
    );
}

#[test]
fn offset_is_added_in_parent_box_space() {
    let mut b = center();
    b["offset"] = json!([15, -25]);
    let p = project(
        vec![card(100.0, 200.0, 400.0, 300.0), text_child(b)],
        vec![],
    );
    let f = evaluate_frame(&p, 0).unwrap();
    close(
        child_point(&get(&f, "label"), 0.5, 0.5, INK),
        (300.0 + 15.0, 350.0 - 25.0),
        "offset",
    );

    // In a scaled parent the offset is scaled with the parent's box space.
    let mut b = center();
    b["offset"] = json!([10, 0]);
    let p = project(
        vec![card(0.0, 0.0, 400.0, 300.0), text_child(b)],
        vec![motion(
            "card",
            2.0,
            json!({ "op": "scale", "from": 2.0, "to": 2.0 }),
        )],
    );
    let f = evaluate_frame(&p, 0).unwrap();
    close(
        child_point(&get(&f, "label"), 0.5, 0.5, INK),
        (400.0 + 20.0, 300.0),
        "scaled offset",
    );
}

#[test]
fn own_move_shifts_a_bound_child() {
    let p = project(
        vec![card(100.0, 200.0, 400.0, 300.0), text_child(center())],
        vec![motion(
            "label",
            2.0,
            json!({ "op": "move", "from": [0, 0], "to": [60, 40] }),
        )],
    );
    let f = evaluate_frame(&p, 60).unwrap();
    close(
        child_point(&get(&f, "label"), 0.5, 0.5, INK),
        (300.0 + 60.0, 350.0 + 40.0),
        "move on bound child",
    );
}

#[test]
fn combined_move_scale_and_geometry() {
    let p = project(
        vec![
            card(100.0, 200.0, 400.0, 100.0),
            text_child(bind("right", "bottom")),
        ],
        vec![
            expand((150.0, 250.0, 600.0, 400.0)),
            motion(
                "card",
                2.0,
                json!({ "op": "move", "from": [0, 0], "to": [-40, 90] }),
            ),
            motion(
                "card",
                2.0,
                json!({ "op": "scale", "from": 1.0, "to": 1.5 }),
            ),
            motion(
                "card",
                2.0,
                json!({ "op": "rotate", "from": 0.0, "to": 20.0 }),
            ),
        ],
    );
    for frame in FRAMES {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        close(
            child_point(&label, 1.0, 1.0, INK),
            parent_point(&card, 1.0, 1.0, [0.0; 4], (0.0, 0.0)),
            &format!("frame {frame}"),
        );
        assert!((label.transform.b - card.transform.b).abs() < 1e-4);
    }
}

#[test]
fn bound_child_inside_a_transformed_group() {
    let group = json!({
        "id": "g", "type": "group", "x": 50, "y": 60, "rotation_degrees": 10,
        "scale_x": 1.5, "scale_y": 1.5, "width": 800, "height": 800,
        "children": [ card(100.0, 100.0, 400.0, 100.0), text_child(center()) ]
    });
    let p = project(vec![group], vec![expand((100.0, 100.0, 400.0, 400.0))]);
    for frame in FRAMES {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, label) = (get(&f, "card"), get(&f, "label"));
        close(
            child_point(&label, 0.5, 0.5, INK),
            parent_point(&card, 0.5, 0.5, [0.0; 4], (0.0, 0.0)),
            &format!("frame {frame}"),
        );
        // The group's scale reaches the child.
        let s = (label.transform.a.powi(2) + label.transform.b.powi(2)).sqrt();
        assert!((s - 1.5).abs() < 1e-4);
    }
}

#[test]
fn invisible_parent_falls_back_to_unbound_placement() {
    let mut c = card(100.0, 200.0, 400.0, 300.0);
    c["visible"] = json!(false);
    let p = project(vec![c, text_child(center())], vec![]);
    let f = evaluate_frame(&p, 0).unwrap();
    let label = get(&f, "label");
    assert!((label.transform.e - 999.0).abs() < 1e-3);
    assert!((label.transform.f - 999.0).abs() < 1e-3);
}

#[test]
fn faded_out_parent_still_positions_child() {
    let p = project(
        vec![card(100.0, 200.0, 400.0, 300.0), text_child(center())],
        vec![motion(
            "card",
            1.0,
            json!({ "op": "fade", "from": 0.0, "to": 0.0 }),
        )],
    );
    let f = evaluate_frame(&p, 0).unwrap();
    assert!(find_in(&f.layers, "card").is_none(), "parent is culled");
    close(
        child_point(&get(&f, "label"), 0.5, 0.5, INK),
        (300.0, 350.0),
        "child still centered on culled parent",
    );
}

#[test]
fn layout_ignores_child_anchor_and_position() {
    let mut l = text_child(center());
    l["anchor_x"] = json!(1.0);
    l["anchor_y"] = json!(1.0);
    let p = project(vec![card(100.0, 200.0, 400.0, 300.0), l], vec![]);
    let f = evaluate_frame(&p, 0).unwrap();
    close(
        child_point(&get(&f, "label"), 0.5, 0.5, INK),
        (300.0, 350.0),
        "anchor ignored",
    );
}

// ---------------------------------------------------------------------------
// Shared elements bound to a layer
// ---------------------------------------------------------------------------

#[test]
fn shared_element_stays_centered_on_a_growing_layer() {
    let shared = json!([{
        "id": "tag",
        "layer": { "id": "tag_layer", "type": "rectangle", "width": 40, "height": 40,
                   "anchor_x": 0.5, "anchor_y": 0.5, "x": 5, "y": 5, "fill": "#CC2200" },
        "track": [
            { "scene": "s", "at": 0.0, "layout": center() },
            { "scene": "s", "at": 3.0, "state": { "scale": 2.0 } }
        ]
    }]);
    let p = project_with(
        scene(
            vec![card(200.0, 300.0, 400.0, 100.0)],
            vec![expand((200.0, 300.0, 400.0, 500.0))],
        ),
        shared,
    );
    // Across the expansion (frames 0..60) and the hold (60..=90).
    for frame in [0, 15, 30, 45, 60, 75, 90] {
        let f = evaluate_frame(&p, frame).unwrap();
        assert_eq!(f.active_shared, vec!["tag"], "frame {frame}");
        let (card, tag) = (get(&f, "card"), get(&f, "tag_layer"));
        close(
            tag.transform.apply(20.0, 20.0),
            card.transform.apply(card.width / 2.0, card.height / 2.0),
            &format!("frame {frame}"),
        );
    }
    // The second key's scale multiplier applies after the blend.
    let f = evaluate_frame(&p, 90).unwrap();
    assert!((get(&f, "tag_layer").transform.a - 2.0).abs() < 1e-4);
}

#[test]
fn shared_key_bound_to_a_scaled_rotated_parent_follows_it() {
    let shared = json!([{
        "id": "tag",
        "layer": { "id": "tag_layer", "type": "rectangle", "width": 40, "height": 40,
                   "fill": "#CC2200" },
        "track": [
            { "scene": "s", "at": 0.0, "layout": bind("right", "bottom") },
            { "scene": "s", "at": 2.0 }
        ]
    }]);
    let p = project_with(
        scene(
            vec![card(200.0, 300.0, 400.0, 200.0)],
            vec![
                motion(
                    "card",
                    2.0,
                    json!({ "op": "scale", "from": 1.0, "to": 2.0 }),
                ),
                motion(
                    "card",
                    2.0,
                    json!({ "op": "rotate", "from": 0.0, "to": 30.0 }),
                ),
            ],
        ),
        shared,
    );
    for frame in [0, 30, 60] {
        let f = evaluate_frame(&p, frame).unwrap();
        let (card, tag) = (get(&f, "card"), get(&f, "tag_layer"));
        // Element anchor (0,0) is the layer origin; its bottom-right corner
        // lands on the card's bottom-right corner.
        close(
            tag.transform.apply(40.0, 40.0),
            card.transform.apply(card.width, card.height),
            &format!("frame {frame}"),
        );
        assert!((tag.transform.b - card.transform.b).abs() < 1e-4);
    }
}

#[test]
fn shared_key_with_missing_parent_falls_back_to_free() {
    // The key's parent is not resolved (invisible layer), so the key behaves as Free
    // at the layer's base position.
    let mut c = card(200.0, 300.0, 400.0, 100.0);
    c["visible"] = json!(false);
    let shared = json!([{
        "id": "tag",
        "layer": { "id": "tag_layer", "type": "rectangle", "x": 11, "y": 22,
                   "width": 40, "height": 40, "fill": "#CC2200" },
        "track": [ { "scene": "s", "at": 0.0, "layout": center() } ]
    }]);
    let p = project_with(scene(vec![c], vec![]), shared);
    let f = evaluate_frame(&p, 0).unwrap();
    let tag = get(&f, "tag_layer");
    close(
        (tag.transform.e, tag.transform.f),
        (11.0, 22.0),
        "free fallback",
    );
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn layout_is_deterministic() {
    let shared = json!([{
        "id": "tag",
        "layer": { "id": "tag_layer", "type": "rectangle", "width": 40, "height": 40,
                   "fill": "#CC2200" },
        "track": [
            { "scene": "s", "at": 0.0, "layout": center() },
            { "scene": "s", "at": 3.0 }
        ]
    }]);
    let p = project_with(
        scene(
            vec![card(100.0, 200.0, 400.0, 100.0), text_child(center())],
            vec![expand((100.0, 200.0, 400.0, 500.0))],
        ),
        shared,
    );
    for frame in FRAMES {
        let a = evaluate_frame(&p, frame).unwrap();
        let b = evaluate_frame(&p, frame).unwrap();
        assert_eq!(a, b, "frame {frame}");
    }
}
