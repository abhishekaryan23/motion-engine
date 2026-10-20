//! (0.11) Validation rules for sprite-sequence assets, playback and screen inserts.

use motion_core::scene::MotionProject;
use motion_core::validate::{validate, ValidationError};
use serde_json::{json, Value};

fn base() -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "sprite_validation" },
        "canvas": { "width": 200, "height": 200, "fps": 30, "background": "#000000" },
        "theme": { "fonts": { "display": "font" } },
        "assets": [
            { "id": "font", "type": "font", "path": "f.ttf" },
            { "id": "logo", "type": "svg", "path": "l.svg" },
            { "id": "still", "type": "image", "path": "still.png" },
            { "id": "spr", "type": "sprite_sequence", "path": "sprites/spr",
              "sprite": { "frame_count": 10, "fps": 12.0, "mode": "loop" } }
        ],
        "scenes": [{
            "id": "s", "start_seconds": 0.0, "duration_seconds": 2.0,
            "layers": [
                { "id": "img", "type": "image", "asset": "spr", "width": 100, "height": 100 }
            ],
            "motions": []
        }]
    })
}

fn errors_of(v: &Value) -> Vec<ValidationError> {
    let p = MotionProject::from_json(&v.to_string()).expect("parses");
    match validate(&p, None) {
        Ok(()) => Vec::new(),
        Err(e) => e.0,
    }
}

#[track_caller]
fn assert_has(v: &Value, path: &str, message: &str) {
    let errs = errors_of(v);
    assert!(
        errs.iter()
            .any(|e| e.path.contains(path) && e.message.contains(message)),
        "expected ({path:?}, {message:?}) in {errs:#?}"
    );
}

fn set_layer(v: &mut Value, patch: Value) {
    for (k, val) in patch.as_object().expect("object") {
        v["scenes"][0]["layers"][0][k] = val.clone();
    }
}

#[test]
fn valid_sprite_project_passes() {
    assert_eq!(errors_of(&base()), Vec::new());
    let mut v = base();
    set_layer(
        &mut v,
        json!({
            "playback": { "start": 0.5, "in_frame": 2, "out_frame": 5 },
            "insert": { "asset": "spr", "screen_box": [0.1, 0.2, 0.5, 0.4],
                        "playback": { "in_frame": 1 } }
        }),
    );
    assert_eq!(errors_of(&v), Vec::new());
}

#[test]
fn sprite_asset_needs_spec() {
    let mut v = base();
    v["assets"][3]["sprite"] = Value::Null;
    assert_has(&v, "assets[spr].sprite", "needs a 'sprite' spec");
}

#[test]
fn sprite_frame_count_and_fps_checked() {
    let mut v = base();
    v["assets"][3]["sprite"]["frame_count"] = json!(0);
    assert_has(&v, "assets[spr].sprite.frame_count", ">= 1");
    let mut v = base();
    v["assets"][3]["sprite"]["fps"] = json!(0.0);
    assert_has(&v, "assets[spr].sprite.fps", "> 0");
    let mut v = base();
    v["assets"][3]["sprite"]["fps"] = json!(-3.0);
    assert_has(&v, "assets[spr].sprite.fps", "> 0");
}

#[test]
fn sprite_pattern_needs_integer_conversion() {
    let mut v = base();
    v["assets"][3]["sprite"]["pattern"] = json!("frame.png");
    assert_has(&v, "assets[spr].sprite.pattern", "integer conversion");
}

#[test]
fn non_sprite_asset_must_not_carry_sprite() {
    let mut v = base();
    v["assets"][2]["sprite"] = json!({ "frame_count": 2, "fps": 10.0, "mode": "loop" });
    assert_has(&v, "assets[still].sprite", "only sprite_sequence");
}

#[test]
fn playback_only_on_sprite_assets() {
    let mut v = base();
    set_layer(
        &mut v,
        json!({ "asset": "still", "playback": { "start": 0.0 } }),
    );
    assert_has(&v, "layers[img].playback", "needs a sprite_sequence");
}

#[test]
fn playback_frames_must_be_in_range() {
    let mut v = base();
    set_layer(&mut v, json!({ "playback": { "in_frame": 10 } }));
    assert_has(&v, "playback.in_frame", "outside");
    let mut v = base();
    set_layer(
        &mut v,
        json!({ "playback": { "in_frame": 5, "out_frame": 3 } }),
    );
    assert_has(&v, "playback.out_frame", "out_frame 3");
    let mut v = base();
    set_layer(&mut v, json!({ "playback": { "out_frame": 10 } }));
    assert_has(&v, "playback.out_frame", "out_frame 10");
}

#[test]
fn image_layer_rejects_font_and_svg_assets() {
    let mut v = base();
    set_layer(&mut v, json!({ "asset": "logo" }));
    assert_has(&v, "layers[img].asset", "unsupported type");
}

#[test]
fn insert_asset_must_exist_and_be_image_or_sprite() {
    let mut v = base();
    set_layer(
        &mut v,
        json!({ "insert": { "asset": "nope", "screen_box": [0.0, 0.0, 1.0, 1.0] } }),
    );
    assert_has(&v, "insert.asset", "missing asset 'nope'");
    for bad in ["font", "logo"] {
        let mut v = base();
        set_layer(
            &mut v,
            json!({ "insert": { "asset": bad, "screen_box": [0.0, 0.0, 1.0, 1.0] } }),
        );
        assert_has(&v, "insert.asset", "unsupported type");
    }
}

#[test]
fn insert_screen_box_checked() {
    for (b, msg) in [
        (json!([-0.1, 0.0, 0.5, 0.5]), "in [0, 1]"),
        (json!([0.0, 0.0, 1.5, 0.5]), "in [0, 1]"),
        (json!([0.0, 0.0, 0.0, 0.5]), "> 0"),
        (json!([0.0, 0.0, 0.5, 0.0]), "> 0"),
    ] {
        let mut v = base();
        set_layer(
            &mut v,
            json!({ "insert": { "asset": "still", "screen_box": b } }),
        );
        assert_has(&v, "insert.screen_box", msg);
    }
}

#[test]
fn insert_playback_only_on_sprite_assets() {
    let mut v = base();
    set_layer(
        &mut v,
        json!({ "insert": { "asset": "still", "screen_box": [0.0, 0.0, 1.0, 1.0],
                            "playback": { "start": 0.0 } } }),
    );
    assert_has(&v, "insert.playback", "needs a sprite_sequence");
}
