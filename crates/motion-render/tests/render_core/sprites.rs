//! (0.11) Sprite-sequence playback and screen inserts in the CPU renderer.

use std::path::{Path, PathBuf};

use motion_core::validate::validate;
use motion_core::{evaluate_frame, MotionProject};
use motion_render::{CpuRenderer, RenderError, Renderer};
use resvg::tiny_skia::{Color, Pixmap};
use serde_json::{json, Value};

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Fresh scratch directory under cargo's per-target temp dir.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("sprites_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn solid(w: u32, h: u32, rgba: [u8; 4]) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    pm.fill(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]));
    pm
}

fn save(pm: &Pixmap, path: &Path) {
    pm.save_png(path).expect("write png");
}

fn write_sprite(dir: &Path, colours: &[[u8; 4]]) {
    std::fs::create_dir_all(dir).expect("sprite dir");
    for (i, c) in colours.iter().enumerate() {
        save(
            &solid(8, 8, *c),
            &dir.join(format!("frame_{:04}.png", i + 1)),
        );
    }
}

fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pm.pixel(x, y).expect("pixel in range");
    [p.red(), p.green(), p.blue()]
}

fn render(r: &CpuRenderer, p: &MotionProject, frame: u32) -> Pixmap {
    let f = evaluate_frame(p, frame).expect("evaluate");
    r.render(&f).expect("render")
}

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// 40x40 @ 10 fps; sprite plays at 10 fps so frame N shows sprite frame N.
fn sprite_project(layer_extra: Value, frame_count: u32) -> MotionProject {
    let mut layer = json!({
        "id": "spr", "type": "image", "asset": "colours", "fit": "fill",
        "width": 40, "height": 40
    });
    for (k, v) in layer_extra.as_object().expect("object") {
        layer[k] = v.clone();
    }
    let v = json!({
        "version": "0.2",
        "project": { "name": "sprite_render" },
        "canvas": { "width": 40, "height": 40, "fps": 10, "background": "#FFFFFF" },
        "assets": [{
            "id": "colours", "type": "sprite_sequence", "path": "colours",
            "sprite": { "frame_count": frame_count, "fps": 10.0, "mode": "loop" }
        }],
        "scenes": [{ "id": "s", "start_seconds": 0.0, "duration_seconds": 2.0,
                     "layers": [layer], "motions": [] }]
    });
    MotionProject::from_json(&v.to_string()).expect("project parses")
}

#[test]
fn sprite_frames_change_pixels_over_time_and_wrap() {
    let dir = scratch("colours");
    write_sprite(&dir.join("colours"), &[RED, GREEN, BLUE]);
    let p = sprite_project(json!({}), 3);
    assert_eq!(validate(&p, Some(&dir)), Ok(()));
    let r = CpuRenderer::new(&p, &dir).expect("renderer");

    assert_eq!(rgb(&render(&r, &p, 0), 20, 20), [255, 0, 0]);
    assert_eq!(rgb(&render(&r, &p, 1), 20, 20), [0, 255, 0]);
    assert_eq!(rgb(&render(&r, &p, 2), 20, 20), [0, 0, 255]);
    assert_eq!(rgb(&render(&r, &p, 3), 20, 20), [255, 0, 0], "wraps");
    assert_eq!(rgb(&render(&r, &p, 4), 20, 20), [0, 255, 0]);
}

#[test]
fn sprite_rendering_is_deterministic_and_order_independent() {
    let dir = scratch("determinism");
    write_sprite(&dir.join("colours"), &[RED, GREEN, BLUE]);
    let p = sprite_project(json!({}), 3);
    let a = CpuRenderer::new(&p, &dir).expect("renderer");
    let b = CpuRenderer::new(&p, &dir).expect("renderer");
    let forward: Vec<Vec<u8>> = (0..6).map(|f| render(&a, &p, f).data().to_vec()).collect();
    let backward: Vec<Vec<u8>> = (0..6)
        .rev()
        .map(|f| render(&b, &p, f).data().to_vec())
        .collect();
    let backward: Vec<Vec<u8>> = backward.into_iter().rev().collect();
    assert_eq!(forward, backward);
    // Rendering a frame again after the cache is warm is identical.
    assert_eq!(render(&a, &p, 1).data(), forward[1].as_slice());
}

#[test]
fn sprite_frames_render_in_parallel_threads() {
    let dir = scratch("threads");
    write_sprite(&dir.join("colours"), &[RED, GREEN, BLUE]);
    let p = sprite_project(json!({}), 3);
    let r = CpuRenderer::new(&p, &dir).expect("renderer");
    let expected = [[255, 0, 0], [0, 255, 0], [0, 0, 255]];
    std::thread::scope(|s| {
        for t in 0..6u32 {
            let (r, p) = (&r, &p);
            s.spawn(move || {
                for i in 0..12u32 {
                    let frame = (t + i) % 6;
                    let want = expected[(frame % 3) as usize];
                    assert_eq!(rgb(&render(r, p, frame), 20, 20), want);
                }
            });
        }
    });
}

#[test]
fn missing_sprite_frame_reports_its_path() {
    let dir = scratch("missing");
    write_sprite(&dir.join("colours"), &[RED, GREEN, BLUE]);
    let p = sprite_project(json!({}), 4); // spec claims 4 frames, 3 on disk
    let r = CpuRenderer::new(&p, &dir).expect("renderer");
    let f = evaluate_frame(&p, 3).expect("evaluate");
    match r.render(&f) {
        Err(RenderError::Asset(msg)) => assert!(msg.contains("frame_0004.png"), "{msg}"),
        other => panic!("expected asset error, got {:?}", other.map(|_| ())),
    }
    // Validation with a base dir flags the missing last frame too.
    let errs = validate(&p, Some(&dir)).expect_err("last frame missing");
    assert!(errs.0.iter().any(|e| e.message.contains("frame_0004.png")));
}

#[test]
fn sprite_layer_treatment_applies_per_frame() {
    let dir = scratch("treated");
    write_sprite(&dir.join("colours"), &[RED, GREEN, BLUE]);
    let p = sprite_project(json!({ "treatment": { "desaturate": 1.0 } }), 3);
    let r = CpuRenderer::new(&p, &dir).expect("renderer");
    let mut lumas = Vec::new();
    for f in 0..3 {
        let c = rgb(&render(&r, &p, f), 20, 20);
        assert!(
            c[0].abs_diff(c[1]) <= 1 && c[1].abs_diff(c[2]) <= 1,
            "frame {f} grey: {c:?}"
        );
        lumas.push(c[0]);
    }
    // Rec.709 luma: green brightest, blue darkest.
    assert!(lumas[1] > lumas[0] && lumas[0] > lumas[2], "{lumas:?}");
    // Cache hit equals first computation.
    assert_eq!(rgb(&render(&r, &p, 1), 20, 20)[0], lumas[1]);
}

// -- screen inserts ----------------------------------------------------------

/// Host 40x40: opaque grey with a fully transparent hole at x,y in 5..35.
fn write_host(path: &Path) {
    let mut pm = solid(40, 40, [128, 128, 128, 255]);
    for y in 5..35 {
        for x in 5..35 {
            let i = ((y * 40 + x) * 4) as usize;
            pm.data_mut()[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
        }
    }
    save(&pm, path);
}

/// 100x40 canvas; host drawn at x=50. Insert rect = [0.25, 0.25, 0.5, 0.5] of
/// the host box, i.e. canvas x 60..80, y 10..30.
fn insert_project(insert: Option<Value>) -> MotionProject {
    let mut host = json!({
        "id": "host", "type": "image", "asset": "host", "fit": "fill",
        "x": 50, "y": 0, "width": 40, "height": 40
    });
    if let Some(i) = insert {
        host["insert"] = i;
    }
    let v = json!({
        "version": "0.2",
        "project": { "name": "insert_render" },
        "canvas": { "width": 100, "height": 40, "fps": 10, "background": "#FFFFFF" },
        "assets": [
            { "id": "host", "type": "image", "path": "host.png" },
            { "id": "wide", "type": "image", "path": "wide.png" },
            { "id": "colours", "type": "sprite_sequence", "path": "colours",
              "sprite": { "frame_count": 3, "fps": 10.0, "mode": "loop" } }
        ],
        "scenes": [{ "id": "s", "start_seconds": 0.0, "duration_seconds": 2.0,
                     "layers": [host], "motions": [] }]
    });
    MotionProject::from_json(&v.to_string()).expect("project parses")
}

fn insert_dir(name: &str) -> PathBuf {
    let dir = scratch(name);
    write_host(&dir.join("host.png"));
    // Very wide green image: Cover on the square rect overflows horizontally.
    save(&solid(80, 10, GREEN), &dir.join("wide.png"));
    write_sprite(&dir.join("colours"), &[RED, GREEN, BLUE]);
    dir
}

#[test]
fn insert_shows_through_the_hole_and_is_clipped_to_its_rect() {
    let dir = insert_dir("insert_still");
    let with = insert_project(Some(json!({
        "asset": "wide", "screen_box": [0.25, 0.25, 0.5, 0.5], "fit": "cover"
    })));
    let without = insert_project(None);
    assert_eq!(validate(&with, Some(&dir)), Ok(()));
    let rw = CpuRenderer::new(&with, &dir).expect("renderer");
    let ro = CpuRenderer::new(&without, &dir).expect("renderer");
    let a = render(&rw, &with, 0);
    let b = render(&ro, &without, 0);

    // Inside the rect (canvas x 60..80, y 10..30) the hole shows the insert.
    for (x, y) in [(60, 10), (70, 20), (79, 29), (65, 12)] {
        assert_eq!(rgb(&a, x, y), [0, 255, 0], "({x},{y})");
        assert_eq!(rgb(&b, x, y), [255, 255, 255], "baseline ({x},{y})");
    }
    // Everywhere outside the rect the image is exactly the host-only render:
    // the hole's remainder stays background (no bleed), grey frame untouched.
    let mut changed = 0;
    for y in 0..40 {
        for x in 0..100 {
            let inside = (60..80).contains(&x) && (10..30).contains(&y);
            if inside {
                changed += 1;
            } else {
                assert_eq!(rgb(&a, x, y), rgb(&b, x, y), "({x},{y}) bled");
            }
        }
    }
    assert_eq!(changed, 400);
    // Hole outside the rect but inside the hole: background, not insert.
    assert_eq!(rgb(&a, 57, 20), [255, 255, 255]);
    assert_eq!(rgb(&a, 82, 20), [255, 255, 255]);
    // Host frame pixels are grey.
    assert_eq!(rgb(&a, 52, 20), [128, 128, 128]);
}

#[test]
fn insert_sprite_advances_with_time() {
    let dir = insert_dir("insert_sprite");
    let p = insert_project(Some(json!({
        "asset": "colours", "screen_box": [0.25, 0.25, 0.5, 0.5], "fit": "fill"
    })));
    let r = CpuRenderer::new(&p, &dir).expect("renderer");
    assert_eq!(rgb(&render(&r, &p, 0), 70, 20), [255, 0, 0]);
    assert_eq!(rgb(&render(&r, &p, 1), 70, 20), [0, 255, 0]);
    assert_eq!(rgb(&render(&r, &p, 2), 70, 20), [0, 0, 255]);
    assert_eq!(rgb(&render(&r, &p, 3), 70, 20), [255, 0, 0]);
    // Deterministic across renderers and repeated renders.
    let r2 = CpuRenderer::new(&p, &dir).expect("renderer");
    assert_eq!(render(&r, &p, 1).data(), render(&r2, &p, 1).data());
    // Left of the host (x < 50) stays background.
    assert_eq!(rgb(&render(&r, &p, 1), 20, 20), [255, 255, 255]);
}

// -- real assets -------------------------------------------------------------

#[test]
fn real_clay_loop_sprite_renders_differently_over_time() {
    let v = json!({
        "version": "0.2",
        "project": { "name": "rocket_smoke" },
        "canvas": { "width": 192, "height": 300, "fps": 30, "background": "#FFFFFF" },
        "assets": [{
            "id": "rocket", "type": "sprite_sequence",
            "path": "assets/library/clay_props_3d/loops/rocket",
            "sprite": { "frame_count": 47, "fps": 12.0, "mode": "loop" }
        }],
        "scenes": [{ "id": "s", "start_seconds": 0.0, "duration_seconds": 2.0,
            "layers": [{ "id": "r", "type": "image", "asset": "rocket",
                         "width": 192, "height": 300 }],
            "motions": [] }]
    });
    let p = MotionProject::from_json(&v.to_string()).expect("parses");
    assert_eq!(validate(&p, Some(repo_root())), Ok(()));
    let r = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let f0 = render(&r, &p, 0);
    let f6 = render(&r, &p, 6);
    assert_ne!(f0.data(), f6.data(), "frame 0 and 6 must differ");
    assert_eq!(render(&r, &p, 0).data(), f0.data());
    let layer = evaluate_frame(&p, 6).expect("evaluate").layers[0].sprite_frame;
    assert_eq!(layer, Some(2));
}
