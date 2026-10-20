//! (0.11) Sprite-sequence playback and screen-insert frames in the Timeline.

use motion_core::scene::MotionProject;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use serde_json::{json, Value};

fn project(layers: Vec<Value>, shared: Value) -> MotionProject {
    let v = json!({
        "version": "0.2",
        "project": { "name": "sprites" },
        "canvas": { "width": 200, "height": 200, "fps": 30, "background": "#000000" },
        "assets": [
            { "id": "loop_sprite", "type": "sprite_sequence", "path": "sprites/loop",
              "sprite": { "frame_count": 10, "fps": 10.0, "mode": "loop" } },
            { "id": "once_sprite", "type": "sprite_sequence", "path": "sprites/once",
              "sprite": { "frame_count": 10, "fps": 10.0, "mode": "once" } },
            { "id": "still", "type": "image", "path": "still.png" }
        ],
        "scenes": [
            { "id": "a", "start_seconds": 0.0, "duration_seconds": 1.0, "layers": [], "motions": [] },
            { "id": "s", "start_seconds": 1.0, "duration_seconds": 3.0, "layers": layers, "motions": [] }
        ],
        "shared": shared,
    });
    MotionProject::from_json(&v.to_string()).expect("project parses")
}

fn img(id: &str, asset: &str, extra: Value) -> Value {
    let mut v = json!({ "id": id, "type": "image", "asset": asset, "width": 100, "height": 100 });
    for (k, val) in extra.as_object().expect("object") {
        v[k] = val.clone();
    }
    v
}

fn layer<'a>(p: &'a MotionProject, frame: u32, id: &str) -> ResolvedLayer<'a> {
    let f = evaluate_frame(p, frame).expect("evaluates");
    f.layers
        .into_iter()
        .find(|l| l.id == id)
        .unwrap_or_else(|| panic!("layer {id} missing at frame {frame}"))
}

#[test]
fn loop_sprite_wraps_with_scene_local_time() {
    let p = project(vec![img("l", "loop_sprite", json!({}))], json!([]));
    // Scene starts at t = 1.0 = frame 30; sprite runs at 10 fps.
    for (frame, want) in [
        (30, 0),
        (32, 0),
        (33, 1),
        (45, 5),
        (59, 9),
        (60, 0),
        (63, 1),
    ] {
        assert_eq!(
            layer(&p, frame, "l").sprite_frame,
            Some(want),
            "frame {frame}"
        );
    }
}

#[test]
fn once_sprite_holds_last_frame() {
    let p = project(vec![img("o", "once_sprite", json!({}))], json!([]));
    assert_eq!(layer(&p, 33, "o").sprite_frame, Some(1));
    assert_eq!(layer(&p, 59, "o").sprite_frame, Some(9));
    assert_eq!(layer(&p, 90, "o").sprite_frame, Some(9));
    assert_eq!(layer(&p, 119, "o").sprite_frame, Some(9));
}

#[test]
fn playback_start_in_out() {
    let p = project(
        vec![img(
            "l",
            "loop_sprite",
            json!({ "playback": { "start": 0.5, "in_frame": 2, "out_frame": 4 } }),
        )],
        json!([]),
    );
    // Local time = (frame - 30) / 30.
    for (frame, want) in [
        (30, 2), // before start: in point
        (44, 2),
        (45, 2), // local 0.5
        (48, 3), // 0.6
        (51, 4), // 0.7
        (54, 2), // 0.8 wraps within [2, 4]
        (57, 3),
    ] {
        assert_eq!(
            layer(&p, frame, "l").sprite_frame,
            Some(want),
            "frame {frame}"
        );
    }
}

#[test]
fn still_images_and_other_layers_have_no_frame() {
    let p = project(
        vec![
            img("st", "still", json!({})),
            json!({ "id": "r", "type": "rectangle", "width": 10, "height": 10, "fill": "#FFFFFF" }),
        ],
        json!([]),
    );
    assert_eq!(layer(&p, 40, "st").sprite_frame, None);
    assert_eq!(layer(&p, 40, "st").insert_frame, None);
    assert_eq!(layer(&p, 40, "r").sprite_frame, None);
    let json = serde_json::to_string(&evaluate_frame(&p, 40).unwrap()).unwrap();
    assert!(!json.contains("sprite_frame") && !json.contains("insert_frame"));
}

#[test]
fn shared_element_sprite_uses_time_since_element_start() {
    let shared = json!([{
        "id": "e",
        "layer": img("sh", "loop_sprite", json!({})),
        "track": [
            { "scene": "s", "at": 0.5, "state": { "x": 0, "y": 0 } },
            { "scene": "s", "at": 2.0, "state": { "x": 10, "y": 0 } }
        ]
    }]);
    let p = project(vec![], shared);
    // Element starts at 1.0 + 0.5 = 1.5 s = frame 45.
    for (frame, want) in [(45, 0), (47, 0), (48, 1), (60, 5), (74, 9), (75, 0)] {
        assert_eq!(
            layer(&p, frame, "sh").sprite_frame,
            Some(want),
            "frame {frame}"
        );
    }
}

#[test]
fn insert_frame_follows_insert_playback() {
    let p = project(
        vec![
            img(
                "host",
                "still",
                json!({ "insert": { "asset": "loop_sprite", "screen_box": [0.1, 0.1, 0.5, 0.5],
                                    "playback": { "start": 0.2, "in_frame": 3 } } }),
            ),
            img(
                "host2",
                "still",
                json!({ "insert": { "asset": "still", "screen_box": [0.1, 0.1, 0.5, 0.5] } }),
            ),
        ],
        json!([]),
    );
    let h = layer(&p, 30, "host");
    assert_eq!((h.sprite_frame, h.insert_frame), (None, Some(3)));
    assert_eq!(layer(&p, 39, "host").insert_frame, Some(4)); // local 0.3, elapsed 0.1
    assert_eq!(layer(&p, 45, "host").insert_frame, Some(6)); // elapsed 0.3 -> 3 + 3
    assert_eq!(layer(&p, 45, "host2").insert_frame, None);
}
