//! Validation rules for MotionScene v0.2.

use motion_core::scene::MotionProject;
use motion_core::validate::{validate, ValidationError};
use serde_json::{json, Value};

fn valid_json() -> Value {
    json!({
        "version": "0.2",
        "project": { "name": "validation_fixture" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#F2EBDD" },
        "theme": { "fonts": { "display": "font_display" } },
        "assets": [
            { "id": "font_display", "type": "font", "path": "assets/fonts/Display.ttf" },
            { "id": "logo_svg", "type": "svg", "path": "assets/library/logo.svg" },
            { "id": "photo", "type": "image", "path": "assets/library/photo.png" }
        ],
        "scenes": [
            {
                "id": "beat_1",
                "start_seconds": 0.0,
                "duration_seconds": 4.0,
                "layers": [
                    { "id": "bg", "type": "rectangle", "width": 1080, "height": 1920, "fill": "#F2EBDD" },
                    {
                        "id": "headline", "type": "text", "x": 80, "y": 400, "width": 900, "height": 300,
                        "text": "HELLO", "font_role": "display", "font_size": 160, "color": "#111111"
                    },
                    {
                        "id": "card", "type": "group", "x": 100, "y": 900, "width": 800, "height": 400,
                        "children": [
                            { "id": "card_bg", "type": "rounded_rectangle", "width": 800, "height": 400,
                              "fill": "#FFFFFF", "radius": 24, "stroke": { "color": "#000000", "width": 2 } }
                        ]
                    },
                    {
                        "id": "grain", "type": "texture", "width": 1080, "height": 1920, "z_index": 10,
                        "material": "grain", "seed": 7, "color": "#000000", "intensity": 0.3, "scale": 4
                    }
                ],
                "motions": [
                    { "id": "headline_move", "target": "headline", "start": 0.0, "duration": 1.0,
                      "easing": "out_cubic", "op": "move", "from": [0, 60], "to": [0, 0] },
                    { "target": "headline", "start": 0.0, "duration": 0.8, "op": "fade", "from": 0, "to": 1 },
                    { "target": "card", "start": 0.5, "duration": 1.0, "op": "mask_reveal", "direction": "right" }
                ]
            }
        ],
        "shared": [
            {
                "id": "marker",
                "layer": { "id": "marker_layer", "type": "rectangle", "width": 40, "height": 40, "fill": "#CC2200" },
                "track": [
                    { "scene": "beat_1", "at": 0.0, "state": { "x": 0, "y": 0, "opacity": 1.0 } },
                    { "scene": "beat_1", "at": 3.0, "easing": "in_out_cubic", "state": { "x": 500, "scale": 2.0 } }
                ]
            }
        ]
    })
}

fn parse(v: &Value) -> MotionProject {
    MotionProject::from_json(&v.to_string()).expect("fixture parses")
}

fn errors_of(v: &Value) -> Vec<ValidationError> {
    match validate(&parse(v), None) {
        Ok(()) => Vec::new(),
        Err(e) => e.0,
    }
}

fn has(errs: &[ValidationError], path: &str, message: &str) -> bool {
    errs.iter()
        .any(|e| e.path.contains(path) && e.message.contains(message))
}

#[track_caller]
fn assert_has(v: &Value, path: &str, message: &str) {
    let errs = errors_of(v);
    assert!(
        has(&errs, path, message),
        "expected error ({path:?}, {message:?}) in {errs:#?}"
    );
}

fn motions(v: &mut Value) -> &mut Vec<Value> {
    v["scenes"][0]["motions"]
        .as_array_mut()
        .expect("motions array")
}

#[test]
fn valid_project_passes() {
    let p = parse(&valid_json());
    assert_eq!(validate(&p, None), Ok(()));
}

#[test]
fn invalid_version_rejected() {
    let mut v = valid_json();
    v["version"] = json!("0.1");
    assert_has(&v, "version", "unsupported version");
}

#[test]
fn duplicate_layer_id_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][3]["id"] = json!("headline");
    assert_has(&v, "layers[headline]", "duplicate layer id");
}

#[test]
fn duplicate_nested_child_id_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][2]["children"][0]["id"] = json!("bg");
    let errs = errors_of(&v);
    assert!(
        has(&errs, "layers[card].children[bg]", "duplicate layer id"),
        "{errs:#?}"
    );
}

#[test]
fn duplicate_layer_id_with_shared_layer_rejected() {
    let mut v = valid_json();
    v["shared"][0]["layer"]["id"] = json!("bg");
    assert_has(&v, "shared[marker].layer", "duplicate layer id");
}

#[test]
fn missing_motion_target_rejected() {
    let mut v = valid_json();
    motions(&mut v)[1]["target"] = json!("nope");
    assert_has(
        &v,
        "scenes[beat_1].motions[1].target",
        "motion target missing",
    );
}

#[test]
fn motion_can_target_nested_child() {
    let mut v = valid_json();
    motions(&mut v).push(json!({
        "target": "card_bg", "start": 1.0, "duration": 0.5, "op": "fade", "from": 0.5, "to": 1.0
    }));
    assert!(errors_of(&v).is_empty());
}

#[test]
fn negative_scene_duration_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["duration_seconds"] = json!(-1.0);
    assert_has(&v, "scenes[beat_1].duration_seconds", "negative duration");
}

#[test]
fn zero_scene_duration_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["duration_seconds"] = json!(0.0);
    assert_has(&v, "scenes[beat_1].duration_seconds", "zero duration");
}

#[test]
fn negative_motion_duration_rejected() {
    let mut v = valid_json();
    motions(&mut v)[0]["duration"] = json!(-0.5);
    assert_has(&v, "motions[headline_move].duration", "negative duration");
}

#[test]
fn missing_asset_id_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][0] = json!({
        "id": "bg", "type": "image", "asset": "does_not_exist", "width": 10, "height": 10
    });
    assert_has(&v, "layers[bg].asset", "missing asset");
}

#[test]
fn asset_kind_mismatch_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][0] = json!({
        "id": "bg", "type": "image", "asset": "logo_svg", "width": 10, "height": 10
    });
    assert_has(&v, "layers[bg].asset", "unsupported type");
    assert_has(
        &v,
        "layers[bg].asset",
        "expects image asset but 'logo_svg' is svg",
    );
}

#[test]
fn matching_asset_kinds_pass() {
    let mut v = valid_json();
    let layers = v["scenes"][0]["layers"].as_array_mut().expect("layers");
    layers
        .push(json!({ "id": "pic", "type": "image", "asset": "photo", "width": 10, "height": 10 }));
    layers.push(
        json!({ "id": "mark", "type": "svg", "asset": "logo_svg", "width": 10, "height": 10 }),
    );
    assert!(errors_of(&v).is_empty());
}

#[test]
fn theme_font_must_be_font_asset() {
    let mut v = valid_json();
    v["theme"]["fonts"]["display"] = json!("photo");
    assert_has(&v, "theme.fonts.display", "unsupported type");
}

#[test]
fn text_role_without_font_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][1]["font_role"] = json!("mono");
    assert_has(&v, "layers[headline].font_role", "no font assigned to role");
}

#[test]
fn missing_asset_file_rejected() {
    let p = parse(&valid_json());
    let base = std::env::temp_dir().join("motion_core_validation_no_such_dir");
    let errs = validate(&p, Some(&base)).expect_err("files do not exist").0;
    assert!(
        has(&errs, "assets[font_display].path", "missing asset file"),
        "{errs:#?}"
    );
    assert_eq!(
        errs.iter()
            .filter(|e| e.message.contains("missing asset file"))
            .count(),
        3
    );
}

#[test]
fn existing_asset_file_accepted_with_asset_root() {
    let dir = std::env::temp_dir().join(format!("motion_core_validation_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("root/sub")).expect("mkdir");
    std::fs::write(dir.join("root/sub/f.ttf"), b"x").expect("write");
    let mut v = valid_json();
    v["asset_root"] = json!("root");
    v["assets"] = json!([{ "id": "font_display", "type": "font", "path": "sub/f.ttf" }]);
    v["scenes"][0]["layers"]
        .as_array_mut()
        .expect("layers")
        .truncate(4);
    let ok = validate(&parse(&v), Some(&dir));
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(ok, Ok(()));
}

#[test]
fn empty_asset_path_and_duplicate_asset_rejected() {
    let mut v = valid_json();
    v["assets"][1]["path"] = json!("");
    v["assets"][2]["id"] = json!("font_display");
    let errs = errors_of(&v);
    assert!(has(&errs, "assets[logo_svg].path", "empty"), "{errs:#?}");
    assert!(
        has(&errs, "assets[font_display]", "duplicate asset id"),
        "{errs:#?}"
    );
}

#[test]
fn invalid_opacity_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][1]["opacity"] = json!(1.5);
    assert_has(
        &v,
        "scenes[beat_1].layers[headline].opacity",
        "invalid opacity",
    );
}

#[test]
fn invalid_clip_rejected() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][0]["clip"] = json!({ "left": 0.7, "right": 0.6 });
    assert_has(&v, "layers[bg].clip", "left + right");
    v["scenes"][0]["layers"][0]["clip"] = json!({ "top": -0.1 });
    assert_has(&v, "layers[bg].clip.top", "invalid clip inset");
}

#[test]
fn layer_geometry_rules() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][0]["width"] = json!(-5.0);
    assert_has(&v, "layers[bg].width", ">= 0");
}

#[test]
fn text_and_texture_rules() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][1]["font_size"] = json!(0);
    v["scenes"][0]["layers"][1]["line_height"] = json!(-1.0);
    v["scenes"][0]["layers"][1]["max_width"] = json!(0);
    v["scenes"][0]["layers"][3]["intensity"] = json!(2.0);
    v["scenes"][0]["layers"][3]["scale"] = json!(0);
    let errs = errors_of(&v);
    for (p, m) in [
        ("layers[headline].font_size", "> 0"),
        ("layers[headline].line_height", "> 0"),
        ("layers[headline].max_width", "> 0"),
        ("layers[grain].intensity", "[0, 1]"),
        ("layers[grain].scale", "> 0"),
    ] {
        assert!(has(&errs, p, m), "missing ({p}, {m}) in {errs:#?}");
    }
}

#[test]
fn rounded_rect_and_stroke_rules() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][2]["children"][0]["radius"] = json!(-1.0);
    v["scenes"][0]["layers"][2]["children"][0]["stroke"]["width"] = json!(-2.0);
    let errs = errors_of(&v);
    assert!(has(&errs, "children[card_bg].radius", ">= 0"), "{errs:#?}");
    assert!(
        has(&errs, "children[card_bg].stroke.width", ">= 0"),
        "{errs:#?}"
    );
}

#[test]
fn fps_zero_rejected() {
    let mut v = valid_json();
    v["canvas"]["fps"] = json!(0);
    assert_has(&v, "canvas.fps", "fps");
    v["canvas"]["fps"] = json!(241);
    assert_has(&v, "canvas.fps", "fps");
}

#[test]
fn zero_canvas_rejected() {
    let mut v = valid_json();
    v["canvas"]["width"] = json!(0);
    v["canvas"]["height"] = json!(16385);
    let errs = errors_of(&v);
    assert!(
        has(&errs, "canvas.width", "invalid canvas size"),
        "{errs:#?}"
    );
    assert!(
        has(&errs, "canvas.height", "invalid canvas size"),
        "{errs:#?}"
    );
}

#[test]
fn project_duration_must_be_positive() {
    let mut v = valid_json();
    v["project"]["duration_seconds"] = json!(0.0);
    assert_has(&v, "project.duration_seconds", "must be finite and > 0");
}

#[test]
fn conflicting_animations_rejected() {
    let mut v = valid_json();
    motions(&mut v).push(json!({
        "target": "headline", "start": 0.4, "duration": 1.0, "op": "fade", "from": 1.0, "to": 0.0
    }));
    assert_has(
        &v,
        "scenes[beat_1].motions[3]",
        "conflicting animations on channel",
    );
}

#[test]
fn sequential_animations_on_same_channel_pass() {
    let mut v = valid_json();
    // Existing fade on headline ends at 0.8; this one starts exactly there.
    motions(&mut v).push(json!({
        "target": "headline", "start": 0.8, "duration": 1.0, "op": "fade", "from": 1.0, "to": 0.0
    }));
    assert!(errors_of(&v).is_empty());
}

#[test]
fn different_channels_and_targets_do_not_conflict() {
    let mut v = valid_json();
    motions(&mut v).push(json!({
        "target": "card", "start": 0.0, "duration": 1.0, "op": "fade", "from": 0.0, "to": 1.0
    }));
    motions(&mut v).push(json!({
        "target": "bg", "start": 0.0, "duration": 1.0, "op": "fade", "from": 0.0, "to": 1.0
    }));
    assert!(errors_of(&v).is_empty());
}

#[test]
fn zero_duration_motion_conflicts_only_when_strictly_inside() {
    let mut v = valid_json();
    // Headline fade spans [0, 0.8).
    motions(&mut v).push(json!({
        "target": "headline", "start": 0.4, "duration": 0.0, "op": "fade", "from": 1.0, "to": 1.0
    }));
    assert_has(&v, "motions[3]", "conflicting animations");

    let mut v = valid_json();
    motions(&mut v).push(json!({
        "target": "headline", "start": 0.8, "duration": 0.0, "op": "fade", "from": 1.0, "to": 1.0
    }));
    assert!(errors_of(&v).is_empty());
}

#[test]
fn motion_ending_after_scene_rejected() {
    let mut v = valid_json();
    motions(&mut v)[2]["start"] = json!(3.5);
    assert_has(&v, "motions[2]", "motion ends after scene");
}

#[test]
fn motion_value_rules() {
    let mut v = valid_json();
    motions(&mut v)[1]["to"] = json!(2.0);
    motions(&mut v).push(json!({
        "target": "card", "start": 2.0, "duration": 1.0, "op": "accent_expand",
        "to": { "x": 0, "y": 0, "width": -10, "height": 5 }
    }));
    let errs = errors_of(&v);
    assert!(has(&errs, "motions[1].to", "invalid opacity"), "{errs:#?}");
    assert!(has(&errs, "motions[3].to.width", ">= 0"), "{errs:#?}");
}

#[test]
fn shared_element_rules() {
    let mut v = valid_json();
    v["shared"][0]["track"][1]["scene"] = json!("ghost");
    v["shared"][0]["track"][0]["state"]["opacity"] = json!(3.0);
    let errs = errors_of(&v);
    assert!(
        has(&errs, "shared[marker].track[1].scene", "missing scene"),
        "{errs:#?}"
    );
    assert!(
        has(
            &errs,
            "shared[marker].track[0].state.opacity",
            "invalid opacity"
        ),
        "{errs:#?}"
    );

    let mut v = valid_json();
    v["shared"][0]["track"] = json!([]);
    assert_has(&v, "shared[marker].track", "empty");

    let mut v = valid_json();
    v["shared"][0]["track"][1]["state"]["scale"] = json!(0.0);
    assert_has(&v, "track[1].state.scale", "> 0");

    let mut v = valid_json();
    let dup = v["shared"][0].clone();
    v["shared"].as_array_mut().expect("shared").push(dup);
    assert_has(&v, "shared[marker].id", "duplicate shared element id");
}

#[test]
fn shared_keys_must_be_time_ordered() {
    let mut v = valid_json();
    v["shared"][0]["track"][0]["at"] = json!(3.5);
    assert_has(
        &v,
        "shared[marker].track[1].at",
        "earlier than the previous key",
    );
}

#[test]
fn unknown_layer_type_fails_to_parse() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][0]["type"] = json!("hologram");
    let err = MotionProject::from_json(&v.to_string()).expect_err("unknown layer type");
    assert!(err.to_string().contains("hologram"), "{err}");
}

#[test]
fn error_paths_name_scene_and_layer() {
    let mut v = valid_json();
    v["scenes"][0]["layers"][2]["children"][0]["opacity"] = json!(-1.0);
    let errs = errors_of(&v);
    let e = errs.first().expect("one error");
    assert!(
        e.path.contains("beat_1") && e.path.contains("card") && e.path.contains("card_bg"),
        "{e}"
    );
    assert_eq!(
        e.path,
        "scenes[beat_1].layers[card].children[card_bg].opacity"
    );
}

#[test]
fn all_errors_are_collected() {
    let mut v = valid_json();
    v["version"] = json!("9");
    v["canvas"]["fps"] = json!(0);
    v["scenes"][0]["layers"][1]["opacity"] = json!(7.0);
    motions(&mut v)[0]["target"] = json!("zzz");
    let errs = errors_of(&v);
    assert!(errs.len() >= 4, "{errs:#?}");
}

// ---------------------------------------------------------------------------
// v0.2 extensions: layout binding, depth, ink, camera, count/trim, polyline
// ---------------------------------------------------------------------------

fn layers(v: &mut Value) -> &mut Vec<Value> {
    v["scenes"][0]["layers"].as_array_mut().expect("layers")
}

fn polyline(id: &str) -> Value {
    json!({ "id": id, "type": "polyline", "width": 200, "height": 100,
            "points": [[0, 0], [100, 50], [200, 0]],
            "stroke": { "color": "#000000", "width": 4 } })
}

fn project_errors(p: &MotionProject) -> Vec<ValidationError> {
    match validate(p, None) {
        Ok(()) => Vec::new(),
        Err(e) => e.0,
    }
}

#[track_caller]
fn assert_ok(v: &Value) {
    let errs = errors_of(v);
    assert!(errs.is_empty(), "expected valid, got {errs:#?}");
}

// -- layout binding --

#[test]
fn layout_parent_earlier_sibling_accepted() {
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "bg", "horizontal": "left",
        "vertical": "top", "offset": [4, 5],
        "padding": { "left": 1, "top": 2, "right": 3, "bottom": 4 } });
    assert_ok(&v);
}

#[test]
fn layout_parent_later_sibling_rejected() {
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "grain" });
    assert_has(&v, "layers[headline].layout.parent", "earlier sibling");
}

#[test]
fn layout_parent_self_or_missing_rejected() {
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "headline" });
    assert_has(&v, "layers[headline].layout.parent", "itself");
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "ghost" });
    assert_has(&v, "layers[headline].layout.parent", "earlier sibling");
}

#[test]
fn layout_parent_must_be_in_the_same_children_list() {
    // card_bg is inside `card`; a top-level layer cannot bind to it.
    let mut v = valid_json();
    layers(&mut v)[3]["layout"] = json!({ "parent": "card_bg" });
    assert_has(&v, "layers[grain].layout.parent", "earlier sibling");
    // Inside the group, binding to an earlier child works.
    let mut v = valid_json();
    layers(&mut v)[2]["children"] = json!([
        { "id": "a", "type": "rectangle", "width": 10, "height": 10, "fill": "#000000" },
        { "id": "b", "type": "rectangle", "width": 10, "height": 10, "fill": "#000000",
          "layout": { "parent": "a" } }
    ]);
    assert_ok(&v);
    // ...but a child cannot bind to the group's own sibling outside it.
    layers(&mut v)[2]["children"][1]["layout"] = json!({ "parent": "bg" });
    assert_has(&v, "children[b].layout.parent", "earlier sibling");
}

#[test]
fn layout_offset_must_be_finite() {
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "bg" });
    let mut p = parse(&v);
    assert!(project_errors(&p).is_empty());
    if let Some(b) = p.scenes[0].layers[1].layout.as_mut() {
        b.offset = [f32::NAN, 0.0];
    }
    let errs = project_errors(&p);
    assert!(has(&errs, "layout.offset", "must be finite"), "{errs:#?}");
}

#[test]
fn layout_padding_must_be_finite_and_non_negative() {
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "bg", "padding": { "left": 10 } });
    assert_ok(&v);
    layers(&mut v)[1]["layout"] = json!({ "parent": "bg", "padding": { "top": -1 } });
    assert_has(&v, "layout.padding.top", "must be finite and >= 0");
    let mut v = valid_json();
    layers(&mut v)[1]["layout"] = json!({ "parent": "bg" });
    let mut p = parse(&v);
    if let Some(b) = p.scenes[0].layers[1].layout.as_mut() {
        b.padding.right = f32::INFINITY;
    }
    let errs = project_errors(&p);
    assert!(has(&errs, "layout.padding.right", "finite"), "{errs:#?}");
}

#[test]
fn track_key_layout_parent_must_exist_in_the_key_scene() {
    let mut v = valid_json();
    v["shared"][0]["track"][0]["layout"] = json!({ "parent": "card_bg" });
    assert_ok(&v);
    v["shared"][0]["track"][0]["layout"] = json!({ "parent": "ghost" });
    assert_has(
        &v,
        "shared[marker].track[0].layout.parent",
        "not found in scene",
    );
    // A layer of another scene does not count.
    let mut v = valid_json();
    v["scenes"].as_array_mut().unwrap().push(json!({
        "id": "beat_2", "start_seconds": 4.0, "duration_seconds": 2.0,
        "layers": [ { "id": "other", "type": "rectangle", "width": 5, "height": 5, "fill": "#000000" } ]
    }));
    v["shared"][0]["track"][0]["layout"] = json!({ "parent": "other" });
    assert_has(&v, "track[0].layout.parent", "not found in scene 'beat_1'");
}

#[test]
fn track_key_layout_values_are_checked() {
    let mut v = valid_json();
    v["shared"][0]["track"][0]["layout"] = json!({ "parent": "bg", "padding": { "bottom": -3 } });
    assert_has(&v, "track[0].layout.padding.bottom", ">= 0");
}

// -- depth --

#[test]
fn depth_must_be_finite_and_non_negative() {
    let mut v = valid_json();
    layers(&mut v)[0]["depth"] = json!(0.0);
    layers(&mut v)[1]["depth"] = json!(2.5);
    assert_ok(&v);
    layers(&mut v)[1]["depth"] = json!(-0.5);
    assert_has(&v, "layers[headline].depth", "finite and >= 0");
    let mut p = parse(&valid_json());
    p.scenes[0].layers[0].depth = Some(f32::NAN);
    let errs = project_errors(&p);
    assert!(has(&errs, "layers[bg].depth", "finite"), "{errs:#?}");
}

// -- ink --

#[test]
fn ink_bounds_must_be_ordered_and_finite() {
    let mut v = valid_json();
    layers(&mut v)[1]["ink"] = json!({ "top": 20, "bottom": 120 });
    assert_ok(&v);
    layers(&mut v)[1]["ink"] = json!({ "top": 20, "bottom": 20 });
    assert_ok(&v);
    layers(&mut v)[1]["ink"] = json!({ "top": 120, "bottom": 20 });
    assert_has(&v, "layers[headline].ink", "top <= bottom");
    let mut p = parse(&valid_json());
    if let motion_core::scene::LayerKind::Text(t) = &mut p.scenes[0].layers[1].kind {
        t.ink = Some(motion_core::scene::InkBounds {
            top: f32::NAN,
            bottom: 1.0,
        });
    }
    let errs = project_errors(&p);
    assert!(has(&errs, "headline].ink", "finite"), "{errs:#?}");
}

// -- camera --

fn set_camera(v: &mut Value, camera: Value) {
    v["scenes"][0]["camera"] = camera;
}

#[test]
fn valid_camera_accepted() {
    let mut v = valid_json();
    set_camera(
        &mut v,
        json!({ "pivot": [500, 900], "motions": [
            { "start": 0.0, "duration": 2.0, "op": "push", "from": 1.0, "to": 1.4 },
            { "start": 2.0, "duration": 1.0, "op": "push", "from": 1.4, "to": 1.0 },
            { "start": 0.5, "duration": 2.0, "op": "track", "from": [0, 0], "to": [30, -10] }
        ] }),
    );
    // Push and track are separate channels; back-to-back push motions do not overlap.
    assert_ok(&v);
}

#[test]
fn camera_motion_timing_is_checked() {
    let mut v = valid_json();
    set_camera(
        &mut v,
        json!({ "motions": [ { "start": -1.0, "duration": 1.0, "op": "push", "from": 1, "to": 2 } ] }),
    );
    assert_has(&v, "camera.motions[0].start", "must be finite and >= 0");
    set_camera(
        &mut v,
        json!({ "motions": [ { "start": 0.0, "duration": -1.0, "op": "push", "from": 1, "to": 2 } ] }),
    );
    assert_has(&v, "camera.motions[0].duration", ">= 0");
    // Zero duration is allowed.
    set_camera(
        &mut v,
        json!({ "motions": [ { "start": 0.0, "duration": 0.0, "op": "push", "from": 1, "to": 2 } ] }),
    );
    assert_ok(&v);
}

#[test]
fn camera_push_values_must_be_positive() {
    let mut v = valid_json();
    set_camera(
        &mut v,
        json!({ "motions": [ { "start": 0.0, "duration": 1.0, "op": "push", "from": 0.0, "to": 2 } ] }),
    );
    assert_has(&v, "camera.motions[0].from", "> 0");
    set_camera(
        &mut v,
        json!({ "motions": [ { "start": 0.0, "duration": 1.0, "op": "push", "from": 1, "to": -2 } ] }),
    );
    assert_has(&v, "camera.motions[0].to", "> 0");
}

#[test]
fn camera_track_values_must_be_finite() {
    let mut v = valid_json();
    set_camera(
        &mut v,
        json!({ "motions": [ { "start": 0.0, "duration": 1.0, "op": "track", "from": [0, 0], "to": [5, 5] } ] }),
    );
    assert_ok(&v);
    let mut p = parse(&v);
    if let Some(cam) = p.scenes[0].camera.as_mut() {
        cam.motions[0].op = motion_core::scene::CameraOp::Track {
            from: [0.0, 0.0],
            to: [f32::INFINITY, 0.0],
        };
    }
    let errs = project_errors(&p);
    assert!(has(&errs, "camera.motions[0]", "finite"), "{errs:#?}");
}

#[test]
fn overlapping_camera_motions_on_one_channel_rejected() {
    let mut v = valid_json();
    set_camera(
        &mut v,
        json!({ "motions": [
            { "start": 0.0, "duration": 2.0, "op": "push", "from": 1, "to": 2 },
            { "start": 1.0, "duration": 2.0, "op": "push", "from": 2, "to": 1 }
        ] }),
    );
    assert_has(&v, "camera.motions[1]", "conflicting camera animations");
    set_camera(
        &mut v,
        json!({ "motions": [
            { "start": 0.0, "duration": 2.0, "op": "track", "from": [0, 0], "to": [1, 1] },
            { "start": 1.0, "duration": 2.0, "op": "track", "from": [1, 1], "to": [0, 0] }
        ] }),
    );
    assert_has(&v, "camera.motions[1]", "channel track");
}

// -- count / trim --

#[test]
fn count_targets_text_layers_only() {
    let mut v = valid_json();
    motions(&mut v).push(json!({ "target": "headline", "start": 0.0, "duration": 1.0,
        "op": "count", "from": 0, "to": 1234.5, "decimals": 1, "grouping": true,
        "prefix": "$", "suffix": "%" }));
    assert_ok(&v);
    motions(&mut v).push(json!({ "target": "bg", "start": 0.0, "duration": 1.0,
        "op": "count", "from": 0, "to": 10 }));
    assert_has(&v, "motions[4].target", "count requires a text layer");
}

#[test]
fn count_decimals_and_values_are_checked() {
    let mut v = valid_json();
    motions(&mut v).push(json!({ "target": "headline", "start": 0.0, "duration": 1.0,
        "op": "count", "from": 0, "to": 10, "decimals": 6 }));
    assert_ok(&v);
    motions(&mut v)[3]["decimals"] = json!(7);
    assert_has(&v, "motions[3].decimals", "<= 6");

    let mut v = valid_json();
    motions(&mut v).push(json!({ "target": "headline", "start": 0.0, "duration": 1.0,
        "op": "count", "from": 0, "to": 10 }));
    let mut p = parse(&v);
    if let Some(m) = p.scenes[0].motions.last_mut() {
        if let motion_core::scene::MotionOp::Count { from, .. } = &mut m.op {
            *from = f64::NAN;
        }
    }
    let errs = project_errors(&p);
    assert!(has(&errs, "motions[3].from", "finite"), "{errs:#?}");
}

#[test]
fn trim_targets_polylines_with_unit_values() {
    let mut v = valid_json();
    layers(&mut v).push(polyline("line"));
    motions(&mut v).push(json!({ "target": "line", "start": 0.0, "duration": 1.0,
        "op": "trim", "from": 0.0, "to": 1.0 }));
    assert_ok(&v);
    motions(&mut v)[3]["to"] = json!(1.5);
    assert_has(&v, "motions[3].to", "[0, 1]");
    motions(&mut v)[3]["to"] = json!(1.0);
    motions(&mut v)[3]["target"] = json!("bg");
    assert_has(&v, "motions[3].target", "trim requires a polyline layer");
}

#[test]
fn count_and_trim_follow_the_same_channel_overlap_rule() {
    let mut v = valid_json();
    motions(&mut v).push(json!({ "target": "headline", "start": 0.0, "duration": 2.0,
        "op": "count", "from": 0, "to": 10 }));
    motions(&mut v).push(json!({ "target": "headline", "start": 1.0, "duration": 2.0,
        "op": "count", "from": 10, "to": 20 }));
    assert_has(&v, "motions[4]", "conflicting animations on channel Text");

    let mut v = valid_json();
    layers(&mut v).push(polyline("line"));
    motions(&mut v).push(json!({ "target": "line", "start": 0.0, "duration": 2.0,
        "op": "trim", "from": 0, "to": 1 }));
    motions(&mut v).push(json!({ "target": "line", "start": 1.0, "duration": 2.0,
        "op": "trim", "from": 1, "to": 0 }));
    assert_has(&v, "motions[4]", "conflicting animations on channel Trim");
    // Count and trim on different layers never conflict.
    let mut v = valid_json();
    layers(&mut v).push(polyline("line"));
    motions(&mut v).push(json!({ "target": "line", "start": 0.0, "duration": 2.0,
        "op": "trim", "from": 0, "to": 1 }));
    motions(&mut v).push(json!({ "target": "headline", "start": 0.0, "duration": 2.0,
        "op": "count", "from": 0, "to": 10 }));
    assert_ok(&v);
}

// -- polyline --

#[test]
fn polyline_points_and_stroke_are_checked() {
    let mut v = valid_json();
    layers(&mut v).push(polyline("line"));
    assert_ok(&v);

    layers(&mut v)[4]["points"] = json!([[0, 0]]);
    assert_has(&v, "layers[line].points", "at least 2 points");

    layers(&mut v)[4]["points"] = json!([[0, 0], [1, 1]]);
    layers(&mut v)[4]["stroke"]["width"] = json!(0);
    assert_has(&v, "layers[line].stroke.width", "> 0");

    let mut v = valid_json();
    layers(&mut v).push(polyline("line"));
    let mut p = parse(&v);
    if let motion_core::scene::LayerKind::Polyline { points, .. } = &mut p.scenes[0].layers[4].kind
    {
        points[1] = [f32::NAN, 0.0];
    }
    let errs = project_errors(&p);
    assert!(has(&errs, "layers[line].points[1]", "finite"), "{errs:#?}");
}
