//! (v2 phase 1) Path layers on the CPU backend: gradients follow the layer's
//! box, `morph` reaches its target exactly (holes included), `trim` draws
//! strokes on and `mask_morph` uncovers a masked layer.

use std::path::{Path, PathBuf};

use motion_core::{evaluate_frame, MotionProject};
use motion_render::{CpuRenderer, Renderer};
use resvg::tiny_skia::Pixmap;

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

fn frame(n: u32) -> Pixmap {
    let text = std::fs::read_to_string(golden_dir().join("v2_morph.motion.json")).expect("read");
    let project = MotionProject::from_json(&text).expect("parse");
    motion_core::validate(&project, Some(&golden_dir())).expect("valid");
    let r = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
    r.render(&evaluate_frame(&project, n).expect("eval"))
        .expect("render")
}

fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pm.pixel(x, y).expect("in bounds").demultiply();
    [p.red(), p.green(), p.blue()]
}

fn near(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

const ORANGE: [u8; 3] = [0xFF, 0x5A, 0x1F];
const INK: [u8; 3] = [0x15, 0x14, 0x13];

#[test]
fn linear_gradient_runs_top_to_bottom_in_box_space() {
    let pm = frame(0);
    assert!(near(rgb(&pm, 20, 1), [0xF1, 0xEC, 0xE2], 3));
    assert!(near(rgb(&pm, 20, 268), [0xDC, 0xD3, 0xC4], 3));
}

#[test]
fn morph_lands_on_target_with_its_hole() {
    // Start: a solid circle. End (square with a square hole): the centre is
    // background, the ring between 30 and 60 px out is orange.
    let start = frame(0);
    assert!(near(rgb(&start, 140, 135), ORANGE, 2));
    let end = frame(59);
    assert!(!near(rgb(&end, 140, 135), ORANGE, 40), "hole is open");
    assert!(near(rgb(&end, 140 - 45, 135 - 45), ORANGE, 2));
}

#[test]
fn trim_draws_the_outline_on() {
    // The outline's top-left quadrant comes last on the clockwise draw.
    let quadrant = |pm: &Pixmap| rgb(pm, 140 - 60, 135 - 60);
    assert!(!near(quadrant(&frame(0)), INK, 60));
    assert!(near(quadrant(&frame(59)), INK, 60));
}

#[test]
fn mask_morph_uncovers_the_layer() {
    assert!(!near(rgb(&frame(0), 360, 135), INK, 10), "hidden at first");
    assert!(
        near(rgb(&frame(59), 360, 135), INK, 2),
        "fully shown at the end"
    );
}

#[test]
fn instances_draw_every_copy_and_trim_builds_them_in_order() {
    let json = r##"{"version": "0.2", "project": {"name": "inst", "duration_seconds": 1.0},
      "canvas": {"width": 100, "height": 40, "fps": 10, "background": "#FFFFFF"},
      "scenes": [{"id": "s", "start_seconds": 0, "duration_seconds": 1.0,
        "layers": [{"id": "dots", "width": 100, "height": 40, "type": "instances",
          "d": "M-1 0A1 1 0 1 1 1 0A1 1 0 1 1 -1 0Z", "fill": "#000000",
          "at": [[10, 20, 6], [50, 20, 6], [90, 20, 6]]}],
        "motions": [{"target": "dots", "start": 0, "duration": 1.0, "op": "trim", "from": 0, "to": 1}]}]}"##;
    let project = MotionProject::from_json(json).expect("parse");
    motion_core::validate(&project, None).expect("valid");
    let r = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
    let at = |n: u32| r.render(&evaluate_frame(&project, n).unwrap()).unwrap();
    let black = |pm: &Pixmap, x: u32| near(rgb(pm, x, 20), [0, 0, 0], 10);
    let early = at(4); // trim 0.4: the first dot whole, the second scaled in
    assert!(black(&early, 10) && !black(&early, 90));
    let late = at(9);
    assert!(black(&late, 10) && black(&late, 50) && black(&late, 90));
}
