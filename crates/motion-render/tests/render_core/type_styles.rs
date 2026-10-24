//! (v2.2) Styled type: an image seen through the glyphs (text mask), a
//! gradient across the line's box, and outline (stroke-only) glyphs. Plain
//! text layers render exactly as before.

use std::path::{Path, PathBuf};

use motion_core::{evaluate_frame, MotionProject};
use motion_render::{CpuRenderer, Renderer};
use resvg::tiny_skia::{Color, Pixmap, Rect, Transform};

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// A temp dir removed when the fixture is dropped (after its render).
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A 64x32 image: left half pure red, right half pure green (absolute
/// path), in its own temp dir.
fn split_image() -> (PathBuf, TempDir) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("me_type_styles_{}_{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("split.png");
    let mut pm = Pixmap::new(64, 32).unwrap();
    let mut paint = resvg::tiny_skia::Paint::default();
    paint.set_color(Color::from_rgba8(255, 0, 0, 255));
    pm.fill_rect(
        Rect::from_xywh(0.0, 0.0, 32.0, 32.0).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    paint.set_color(Color::from_rgba8(0, 255, 0, 255));
    pm.fill_rect(
        Rect::from_xywh(32.0, 0.0, 32.0, 32.0).unwrap(),
        &paint,
        Transform::identity(),
        None,
    );
    pm.save_png(&path).unwrap();
    (path, TempDir(dir))
}

/// White 400x200 canvas with one text layer "HH" (Anton, 180 px, black)
/// plus `extra` JSON fields on that layer.
fn project(extra: &str) -> (MotionProject, TempDir) {
    let (img, tmp) = split_image();
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "type_styles", "duration_seconds": 1.0 }},
        "canvas": {{ "width": 400, "height": 200, "fps": 30, "background": "#FFFFFF" }},
        "theme": {{ "fonts": {{ "number": "font.anton" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "assets/fonts/Anton-Regular.ttf" }},
            {{ "id": "img.split", "type": "image", "path": "{}" }}
        ],
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0,
            "layers": [ {{ "id": "t", "type": "text", "text": "HH", "font_role": "number",
                "font_size": 180, "color": "#000000", "align": "center",
                "x": 50, "y": 0, "width": 300, "height": 200 {extra} }} ],
            "motions": [] }} ]
    }}"##,
        img.display()
    );
    (
        MotionProject::from_json(&json).expect("fixture parses"),
        tmp,
    )
}

fn render((p, _tmp): &(MotionProject, TempDir)) -> Pixmap {
    let r = CpuRenderer::new(p, repo_root()).expect("renderer");
    let f = evaluate_frame(p, 0).expect("evaluate");
    r.render(&f).expect("render")
}

fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pm.pixel(x, y).expect("pixel in range");
    [p.red(), p.green(), p.blue()]
}

/// Pixels that are solid glyph in the plain render.
fn glyph_pixels(plain: &Pixmap) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in 0..200 {
        for x in 0..400 {
            if rgb(plain, x, y) == [0, 0, 0] {
                out.push((x, y));
            }
        }
    }
    out
}

#[test]
fn image_fill_shows_the_image_inside_glyphs_and_the_ground_outside() {
    let plain = render(&project(""));
    let filled = render(&project(r#", "fill": { "image": "img.split" }"#));
    let glyph = glyph_pixels(&plain);
    assert!(
        glyph.len() > 2000,
        "glyphs must cover pixels ({})",
        glyph.len()
    );
    let (mut red, mut green) = (0, 0);
    for &(x, y) in &glyph {
        match rgb(&filled, x, y) {
            [r, g, b] if r > 200 && g < 40 && b < 40 => red += 1,
            [r, g, b] if g > 200 && r < 40 && b < 40 => green += 1,
            c => panic!("glyph pixel ({x},{y}) not an image colour: {c:?}"),
        }
    }
    // Left glyph shows the left (red) half, right glyph the right (green).
    assert!(red > 800 && green > 800, "red {red}, green {green}");
    for y in 0..200 {
        for x in 0..400 {
            if rgb(&plain, x, y) == [255, 255, 255] {
                assert_eq!(
                    rgb(&filled, x, y),
                    [255, 255, 255],
                    "ground ({x},{y}) changed"
                );
            }
        }
    }
}

#[test]
fn gradient_fill_differs_left_to_right() {
    let plain = render(&project(""));
    let g = render(&project(
        r##", "fill": { "gradient": { "gradient": "linear", "from": [0, 100], "to": [300, 100],
            "stops": [ { "at": 0.0, "color": "#FF0000" }, { "at": 1.0, "color": "#0000FF" } ] } }"##,
    ));
    let glyph = glyph_pixels(&plain);
    let left: Vec<_> = glyph.iter().filter(|p| p.0 < 150).collect();
    let right: Vec<_> = glyph.iter().filter(|p| p.0 > 250).collect();
    assert!(!left.is_empty() && !right.is_empty());
    let mean = |ps: &[&(u32, u32)], ch: usize| {
        ps.iter()
            .map(|&&(x, y)| rgb(&g, x, y)[ch] as f32)
            .sum::<f32>()
            / ps.len() as f32
    };
    assert!(
        mean(&left, 0) > mean(&right, 0) + 80.0,
        "left must be redder"
    );
    assert!(
        mean(&right, 2) > mean(&left, 2) + 80.0,
        "right must be bluer"
    );
}

#[test]
fn outline_type_is_hollow() {
    let plain = render(&project(""));
    let hollow = render(&project(r#", "outline": 4.0"#));
    let glyph = glyph_pixels(&plain);
    let opened = glyph
        .iter()
        .filter(|&&(x, y)| rgb(&hollow, x, y) == [255, 255, 255])
        .count();
    let inked = glyph
        .iter()
        .filter(|&&(x, y)| rgb(&hollow, x, y)[0] < 64)
        .count();
    assert!(
        opened > glyph.len() / 3,
        "glyph interiors must be open ({opened} of {})",
        glyph.len()
    );
    assert!(inked > 300, "outlines must be drawn ({inked})");
}

#[test]
fn plain_text_is_unchanged_by_absent_style_fields() {
    let a = render(&project(""));
    let b = render(&project(r#", "fill": null, "outline": null"#));
    assert_eq!(a.data(), b.data());
}
