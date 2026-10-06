//! (0.9 F1) Monochrome line-art assets stay legible on every ground: ingest
//! analysis flags them, the compiler inks them in the palette ink, and the
//! rendered glyphs reach WCAG AA contrast on dark and light styles alike.

use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::{compile_with_assets, AssetLibrary, FontSet};
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::{evaluate_frame, validate, CreativeIntent, StyleProfile};
use motion_render::analysis::{analyze, AnalyzeOptions};
use motion_render::decode::decode_file;
use motion_render::{CpuRenderer, FontMeasure, Renderer};
use resvg::tiny_skia::Pixmap;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assets_dir() -> PathBuf {
    repo().join("assets")
}

fn put(pm: &mut Pixmap, x: usize, y: usize, rgba: [u8; 4]) {
    let w = pm.width() as usize;
    let i = (y * w + x) * 4;
    pm.data_mut()[i..i + 4].copy_from_slice(&rgba);
}

/// Flat dark ink (#1d1b19) strokes on transparency, like a glyph-family asset.
fn black_ink_on_transparent() -> Pixmap {
    let mut pm = Pixmap::new(64, 64).expect("pixmap");
    for y in 20..44 {
        for x in 8..56 {
            if (x + y) % 3 != 0 {
                put(&mut pm, x, y, [0x1d, 0x1b, 0x19, 255]);
            }
        }
    }
    pm
}

#[test]
fn black_ink_on_transparency_is_monochrome() {
    let a = analyze(&black_ink_on_transparent(), &AnalyzeOptions::default());
    assert!(a.monochrome);
}

#[test]
fn half_transparent_ink_is_still_monochrome() {
    let mut pm = black_ink_on_transparent();
    for px in pm.data_mut().chunks_exact_mut(4) {
        if px[3] != 0 {
            // Premultiplied 50 % alpha ink.
            px[0] /= 2;
            px[1] /= 2;
            px[2] /= 2;
            px[3] = 128;
        }
    }
    assert!(analyze(&pm, &AnalyzeOptions::default()).monochrome);
}

#[test]
fn empty_image_is_not_monochrome() {
    let pm = Pixmap::new(32, 32).expect("pixmap");
    assert!(!analyze(&pm, &AnalyzeOptions::default()).monochrome);
}

#[test]
fn coloured_cutout_is_not_monochrome() {
    let pm = decode_file(&repo().join("golden/assets/worker_cutout.png"))
        .expect("decode worker cutout")
        .pixmap;
    assert!(!analyze(&pm, &AnalyzeOptions::default()).monochrome);
}

#[test]
fn saturated_ink_is_not_monochrome() {
    let mut pm = Pixmap::new(32, 32).expect("pixmap");
    for y in 8..24 {
        for x in 8..24 {
            put(&mut pm, x, y, [200, 30, 30, 255]);
        }
    }
    assert!(!analyze(&pm, &AnalyzeOptions::default()).monochrome);
}

/// A greyscale photo cutout (alpha disc filled with a luma ramp 0.1..0.9, zero
/// chroma) has full tonal range: recolouring it would flatten it to a
/// silhouette, so it must NOT count as monochrome line art.
#[test]
fn greyscale_gradient_cutout_is_not_monochrome() {
    let n = 128usize;
    let mut pm = Pixmap::new(n as u32, n as u32).expect("pixmap");
    let c = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            if dx * dx + dy * dy <= (c - 4.0) * (c - 4.0) {
                let l = 0.1 + 0.8 * (x as f32 / (n - 1) as f32);
                let v = (l * 255.0).round() as u8;
                put(&mut pm, x, y, [v, v, v, 255]);
            }
        }
    }
    assert!(!analyze(&pm, &AnalyzeOptions::default()).monochrome);
}

/// A flat light-grey ink on transparency is still flat ink.
#[test]
fn flat_grey_ink_is_monochrome() {
    let mut pm = Pixmap::new(64, 64).expect("pixmap");
    for y in 16..48 {
        for x in 16..48 {
            if (x / 4 + y / 4) % 2 == 0 {
                put(&mut pm, x, y, [180, 180, 180, 255]);
            }
        }
    }
    assert!(analyze(&pm, &AnalyzeOptions::default()).monochrome);
}

#[test]
fn shipped_glyph_manifests_are_flagged_monochrome() {
    for family in [
        "sketch_icons",
        "woodcut_kitchen",
        "retro_cars_a",
        "retro_cars_b",
        "aikakirja_ornaments",
    ] {
        let text =
            std::fs::read_to_string(assets_dir().join(format!("library/{family}/manifest.json")))
                .expect("read manifest");
        let v: serde_json::Value = serde_json::from_str(&text).expect("manifest json");
        let assets = v["assets"].as_array().expect("assets");
        assert!(!assets.is_empty());
        for a in assets {
            assert_eq!(
                a["analysis"]["monochrome"],
                serde_json::Value::Bool(true),
                "{family}: {} not flagged",
                a["id"]
            );
        }
    }
}

// ---------------------------------------------------------------------------
// End to end: contrast of the rendered glyph against the ground
// ---------------------------------------------------------------------------

fn compile_icon_demo(style_file: &str) -> MotionProject {
    let intent: CreativeIntent = serde_json::from_str(
        &std::fs::read_to_string(repo().join("examples/icon_demo/icon_demo.intent.json"))
            .expect("read intent"),
    )
    .expect("intent parses");
    let style: StyleProfile =
        serde_json::from_str(&std::fs::read_to_string(repo().join(style_file)).expect("style"))
            .expect("style parses");
    let assets = assets_dir();
    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    let library = AssetLibrary::new(&assets)
        .with_families(vec!["sketch_icons".into(), "woodcut_kitchen".into()]);
    let p = compile_with_assets(&intent, &style, &library, &measure, &AssetManifest::empty())
        .expect("compiles");
    validate(&p, Some(&assets)).unwrap_or_else(|e| panic!("invalid scene:\n{e}"));
    p
}

fn is_library_glyph(p: &MotionProject, l: &Layer) -> bool {
    let LayerKind::Image { asset, .. } = &l.kind else {
        return false;
    };
    p.assets
        .iter()
        .any(|a| &a.id == asset && a.path.starts_with("library/"))
}

fn count_glyphs(p: &MotionProject, layers: &[Layer]) -> usize {
    layers
        .iter()
        .map(|l| match &l.kind {
            LayerKind::Group { children } => count_glyphs(p, children),
            _ => usize::from(is_library_glyph(p, l)),
        })
        .sum()
}

/// Hide library glyph layers (by index path, since `p` is borrowed).
fn glyph_paths(
    p: &MotionProject,
    layers: &[Layer],
    prefix: &mut Vec<usize>,
    out: &mut Vec<Vec<usize>>,
) {
    for (i, l) in layers.iter().enumerate() {
        prefix.push(i);
        match &l.kind {
            LayerKind::Group { children } => glyph_paths(p, children, prefix, out),
            _ if is_library_glyph(p, l) => out.push(prefix.clone()),
            _ => {}
        }
        prefix.pop();
    }
}

fn hide_at(layers: &mut [Layer], path: &[usize]) {
    let Some((&first, rest)) = path.split_first() else {
        return;
    };
    let l = &mut layers[first];
    if rest.is_empty() {
        l.visible = false;
    } else if let LayerKind::Group { children } = &mut l.kind {
        hide_at(children, rest);
    }
}

fn render(p: &MotionProject, frame: u32) -> Pixmap {
    let renderer = CpuRenderer::new(p, &assets_dir()).expect("renderer");
    let resolved = evaluate_frame(p, frame).expect("evaluate frame");
    renderer.render(&resolved).expect("render")
}

fn lin(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn rel_luminance(rgb: [f32; 3]) -> f32 {
    0.2126 * lin(rgb[0] / 255.0) + 0.7152 * lin(rgb[1] / 255.0) + 0.0722 * lin(rgb[2] / 255.0)
}

fn contrast(a: [f32; 3], b: [f32; 3]) -> f32 {
    let (la, lb) = (rel_luminance(a), rel_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn rgb_of(pm: &Pixmap, i: usize) -> [f32; 3] {
    let c = pm.pixels()[i].demultiply();
    [
        f32::from(c.red()),
        f32::from(c.green()),
        f32::from(c.blue()),
    ]
}

/// Contrast of the glyph ink against the ground at the same pixels, from a
/// render with and without the library glyph layers. Returns
/// (ratio, ink pixel count).
fn glyph_contrast(p: &MotionProject, scene: usize, frame: u32) -> (f32, usize) {
    let with = render(p, frame);
    let mut bare = p.clone();
    let mut paths = Vec::new();
    glyph_paths(p, &p.scenes[scene].layers, &mut Vec::new(), &mut paths);
    for path in &paths {
        hide_at(&mut bare.scenes[scene].layers, path);
    }
    let without = render(&bare, frame);
    let (mut ink, mut ground) = ([0f64; 3], [0f64; 3]);
    let mut n = 0usize;
    for i in 0..(with.width() * with.height()) as usize {
        let (a, b) = (rgb_of(&with, i), rgb_of(&without, i));
        let diff = (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs();
        // Core ink pixels only: anti-aliased edges blend with the ground.
        if diff >= 96.0 {
            n += 1;
            for k in 0..3 {
                ink[k] += f64::from(a[k]);
                ground[k] += f64::from(b[k]);
            }
        }
    }
    if n == 0 {
        return (0.0, 0);
    }
    let avg = |v: [f64; 3]| [0, 1, 2].map(|k| (v[k] / n as f64) as f32);
    (contrast(avg(ink), avg(ground)), n)
}

fn check_style(style_file: &str) {
    let p = compile_icon_demo(style_file);
    let mut measured = 0;
    for (si, s) in p.scenes.iter().enumerate() {
        if count_glyphs(&p, &s.layers) == 0 {
            continue;
        }
        let life = s.lifecycle.expect("lifecycle");
        let t = s.start_seconds + (life.read + life.evolve) * 0.5;
        let frame =
            ((t * f64::from(p.canvas.fps)).round() as u32).min(p.frame_count().saturating_sub(1));
        let (ratio, n) = glyph_contrast(&p, si, frame);
        println!(
            "{style_file} scene {} frame {frame}: contrast {ratio:.2}:1 ({n} ink px)",
            s.id
        );
        assert!(
            n >= 300,
            "{style_file} {}: glyph not visible ({n} px)",
            s.id
        );
        assert!(
            ratio >= 4.5,
            "{style_file} {}: glyph contrast {ratio:.2} < 4.5",
            s.id
        );
        measured += 1;
    }
    assert!(measured > 0, "{style_file}: no beat shows a library glyph");
}

#[test]
fn glyph_contrast_dark_technical() {
    check_style("examples/taste/dark_technical.style.json");
}

#[test]
fn glyph_contrast_warm_editorial() {
    check_style("examples/taste/warm_editorial.style.json");
}

#[test]
fn glyph_contrast_playful_print() {
    check_style("examples/taste/playful_print.style.json");
}
