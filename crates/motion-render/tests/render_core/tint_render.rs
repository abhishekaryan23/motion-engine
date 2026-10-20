//! CPU rendering of the `tint` channel: vector layers mix their colours,
//! raster layers are tinted through an offscreen inside their alpha only, and
//! an untinted project renders exactly as before.

use std::path::Path;

use motion_core::{evaluate_frame, MotionProject};
use motion_render::{CpuRenderer, Renderer};
use resvg::tiny_skia::Pixmap;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// White 400x200 canvas; `layers` are JSON layer objects, `motions` JSON motion objects.
fn project(layers: &str, motions: &str) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "tint_render", "duration_seconds": 1.0 }},
        "canvas": {{ "width": 400, "height": 200, "fps": 30, "background": "#FFFFFF" }},
        "theme": {{ "fonts": {{ "number": "font.anton" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "assets/fonts/Anton-Regular.ttf" }},
            {{ "id": "img.person", "type": "image",
               "path": "crates/motion-render/tests/fixtures/cutout_person_768x1024.png" }}
        ],
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0,
                       "layers": {layers}, "motions": {motions} }} ]
    }}"##
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn tint(target: &str, color: &str, amount: f64) -> String {
    format!(
        r##"{{ "target": "{target}", "start": 0.0, "duration": 1.0, "op": "tint",
               "color": "{color}", "from": {amount}, "to": {amount} }}"##
    )
}

fn render(p: &MotionProject) -> Pixmap {
    let r = CpuRenderer::new(p, repo_root()).expect("renderer");
    let f = evaluate_frame(p, 0).expect("evaluate");
    r.render(&f).expect("render")
}

fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pm.pixel(x, y).expect("pixel in range");
    [p.red(), p.green(), p.blue()]
}

fn close(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

const RECT: &str = r##"[
    { "id": "r", "type": "rectangle", "x": 50, "y": 50, "width": 100, "height": 100, "fill": "#0000FF" },
    { "id": "rr", "type": "rounded_rectangle", "x": 250, "y": 50, "width": 100, "height": 100,
      "radius": 10, "fill": "#0000FF", "stroke": { "color": "#00FF00", "width": 6 } }
]"##;

#[test]
fn rectangle_tinted_fully_becomes_the_tint_colour() {
    let pm = render(&project(RECT, &format!("[{}]", tint("r", "#FF0000", 1.0))));
    assert_eq!(rgb(&pm, 100, 100), [255, 0, 0]);
    // Untinted sibling keeps its fill.
    assert_eq!(rgb(&pm, 300, 100), [0, 0, 255]);
    // Background is untouched.
    assert_eq!(rgb(&pm, 10, 10), [255, 255, 255]);
}

#[test]
fn half_tint_mixes_the_channels() {
    let pm = render(&project(RECT, &format!("[{}]", tint("r", "#FF0000", 0.5))));
    // 0000FF -> FF0000 at 0.5.
    assert!(
        close(rgb(&pm, 100, 100), [128, 0, 128], 1),
        "{:?}",
        rgb(&pm, 100, 100)
    );
}

#[test]
fn rounded_rectangle_fill_and_stroke_are_tinted() {
    let pm = render(&project(RECT, &format!("[{}]", tint("rr", "#FF0000", 1.0))));
    assert_eq!(rgb(&pm, 300, 100), [255, 0, 0]);
    // On the stroke (left edge x = 250, stroke centred on it): tinted as well,
    // so no green survives anywhere across the edge.
    for x in 248..252 {
        let c = rgb(&pm, x, 100);
        assert!(c[1] < 5, "green stroke leaked at x={x}: {c:?}");
    }
}

#[test]
fn text_tint_changes_the_glyph_colour() {
    let layers = r##"[ { "id": "t", "type": "text", "text": "H", "font_role": "number",
        "font_size": 180, "color": "#000000", "x": 20, "y": 10, "width": 300, "height": 190 } ]"##;
    let plain = render(&project(layers, "[]"));
    let tinted = render(&project(
        layers,
        &format!("[{}]", tint("t", "#FF0000", 1.0)),
    ));
    let mut solid = 0;
    for y in 0..200 {
        for x in 0..400 {
            let a = rgb(&plain, x, y);
            let b = rgb(&tinted, x, y);
            if a == [0, 0, 0] {
                solid += 1;
                assert_eq!(b, [255, 0, 0], "solid glyph pixel ({x},{y}) not tinted");
            }
            if a == [255, 255, 255] {
                assert_eq!(b, a, "background pixel ({x},{y}) changed");
            }
        }
    }
    assert!(solid > 500, "the glyph must cover pixels ({solid})");
}

#[test]
fn polyline_stroke_and_fill_are_tinted() {
    let layers = r##"[ { "id": "p", "type": "polyline", "x": 0, "y": 0, "width": 400, "height": 200,
        "points": [[50, 50], [150, 50], [150, 150], [50, 150]], "closed": true,
        "fill": "#0000FF", "stroke": { "color": "#000000", "width": 8 } } ]"##;
    let pm = render(&project(
        layers,
        &format!("[{}]", tint("p", "#FF0000", 1.0)),
    ));
    assert_eq!(rgb(&pm, 100, 100), [255, 0, 0], "fill");
    assert_eq!(rgb(&pm, 100, 50), [255, 0, 0], "stroke");
}

#[test]
fn group_tint_reaches_children() {
    let layers = r##"[ { "id": "g", "type": "group", "width": 400, "height": 200, "children": [
        { "id": "c", "type": "rectangle", "x": 50, "y": 50, "width": 100, "height": 100, "fill": "#0000FF" }
    ] } ]"##;
    let pm = render(&project(
        layers,
        &format!("[{}]", tint("g", "#FF0000", 1.0)),
    ));
    assert_eq!(rgb(&pm, 100, 100), [255, 0, 0]);
}

const IMAGE: &str = r##"[ { "id": "img", "type": "image", "asset": "img.person",
    "x": 20, "y": 10, "width": 150, "height": 180, "fit": "contain" } ]"##;

#[test]
fn image_tint_is_flat_inside_the_alpha_and_transparent_stays_transparent() {
    let plain = render(&project(IMAGE, "[]"));
    let tinted = render(&project(
        IMAGE,
        &format!("[{}]", tint("img", "#FF0000", 1.0)),
    ));
    let (mut flat, mut clear) = (0, 0);
    for y in 0..200 {
        for x in 0..400 {
            let a = rgb(&plain, x, y);
            let b = rgb(&tinted, x, y);
            if a == [255, 255, 255] {
                // Nothing drawn here without tint: nothing with it.
                clear += 1;
                assert_eq!(b, a, "transparent pixel ({x},{y}) changed");
            } else if b == [255, 0, 0] {
                flat += 1;
            }
        }
    }
    assert!(clear > 1000, "expected transparent regions ({clear})");
    assert!(flat > 1000, "expected flat tinted interior ({flat})");
}

#[test]
fn image_half_tint_moves_colours_toward_the_tint() {
    let plain = render(&project(IMAGE, "[]"));
    let tinted = render(&project(
        IMAGE,
        &format!("[{}]", tint("img", "#FF0000", 0.5)),
    ));
    let mut checked = 0;
    for y in 0..200 {
        for x in 0..400 {
            let a = rgb(&plain, x, y);
            if a == [255, 255, 255] {
                continue;
            }
            let b = rgb(&tinted, x, y);
            // Red never decreases toward a red tint, green/blue never increase.
            assert!(b[0] as i32 >= a[0] as i32 - 1, "({x},{y}) {a:?} -> {b:?}");
            assert!(b[1] as i32 <= a[1] as i32 + 1, "({x},{y}) {a:?} -> {b:?}");
            checked += 1;
        }
    }
    assert!(checked > 1000);
}

#[test]
fn no_tint_motion_renders_identically_to_a_zero_amount_tint() {
    // Amount 0 resolves to no tint, so both take the untinted draw path.
    for layers in [RECT, IMAGE] {
        let base = render(&project(layers, "[]"));
        let zero = render(&project(
            layers,
            &format!(
                "[{}]",
                tint(if layers == RECT { "r" } else { "img" }, "#FF0000", 0.0)
            ),
        ));
        assert_eq!(base.data(), zero.data());
    }
}

#[test]
fn tinted_image_respects_layer_opacity() {
    let layers = r##"[ { "id": "img", "type": "image", "asset": "img.person", "opacity": 0.5,
        "x": 20, "y": 10, "width": 150, "height": 180, "fit": "contain" } ]"##;
    let tinted = render(&project(
        layers,
        &format!("[{}]", tint("img", "#FF0000", 1.0)),
    ));
    let mut half = 0;
    for y in 0..200 {
        for x in 0..400 {
            let b = rgb(&tinted, x, y);
            // Fully covered pixels: red at 50% over white = (255, 128, 128).
            if close(b, [255, 128, 128], 1) {
                half += 1;
            }
            assert!(
                b[0] >= 254,
                "({x},{y}): red channel can only be white or tinted, got {b:?}"
            );
        }
    }
    assert!(
        half > 1000,
        "expected half-opacity tinted interior ({half})"
    );
}
