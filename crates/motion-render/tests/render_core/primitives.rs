//! Renderer support for the Polyline layer (with Trim) and Count text overrides.
//!
//! Frames are resolved with the timeline and then the `trim` / `text` fields
//! are set by hand, so these tests exercise the renderer alone.

use std::path::Path;

use motion_core::timeline::ResolvedFrame;
use motion_core::{evaluate_frame, MotionProject};
use motion_render::{CpuRenderer, Renderer};
use resvg::tiny_skia::Pixmap;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn project(layers: &str) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "primitives", "duration_seconds": 1.0 }},
        "canvas": {{ "width": 1000, "height": 400, "fps": 30, "background": "#FFFFFF" }},
        "theme": {{ "fonts": {{ "number": "font.anton", "body": "font.fira" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" }},
            {{ "id": "font.fira", "type": "font", "path": "fonts/FiraSans-Regular.ttf" }}
        ],
        "asset_root": "assets",
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0, "layers": {layers} }} ]
    }}"##
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn line_project() -> MotionProject {
    project(
        r##"[
        { "id": "line", "type": "polyline", "x": 0, "y": 0, "width": 1000, "height": 400,
          "points": [[100, 200], [900, 200]], "stroke": { "color": "#000000", "width": 20 } },
        { "id": "ell", "type": "polyline", "x": 0, "y": 0, "width": 1000, "height": 400,
          "points": [[100, 40], [500, 40], [500, 240]], "stroke": { "color": "#FF0000", "width": 10 } }
    ]"##,
    )
}

fn render_with(
    r: &CpuRenderer,
    p: &MotionProject,
    tweak: impl FnOnce(&mut ResolvedFrame<'_>),
) -> Pixmap {
    let mut f = evaluate_frame(p, 0).expect("evaluate");
    tweak(&mut f);
    r.render(&f).expect("render")
}

fn is_ink(pm: &Pixmap, x: u32, y: u32) -> bool {
    // Background is white: any darkening counts as painted.
    let p = pm.pixel(x, y).expect("pixel in range");
    p.red() < 128 || p.green() < 128 || p.blue() < 128
}

fn red_at(pm: &Pixmap, x: u32, y: u32) -> bool {
    let p = pm.pixel(x, y).expect("pixel in range");
    p.red() > 200 && p.green() < 100
}

fn trimmed(r: &CpuRenderer, p: &MotionProject, t: Option<f32>) -> Pixmap {
    render_with(r, p, |f| {
        for l in &mut f.layers {
            l.trim = t;
        }
    })
}

#[test]
fn polyline_trim_reveals_from_the_first_point() {
    let p = line_project();
    let r = CpuRenderer::new(&p, repo_root()).expect("renderer");

    // Trim 0: nothing at all.
    let none = trimmed(&r, &p, Some(0.0));
    assert!(!is_ink(&none, 100, 200) && !is_ink(&none, 500, 200) && !is_ink(&none, 895, 200));
    assert!(!red_at(&none, 300, 40));

    // Trim 0.5: line from x=100 to x=500 (plus a 10 px round cap).
    let half = trimmed(&r, &p, Some(0.5));
    assert!(is_ink(&half, 120, 200) && is_ink(&half, 300, 200) && is_ink(&half, 490, 200));
    assert!(!is_ink(&half, 530, 200) && !is_ink(&half, 700, 200) && !is_ink(&half, 890, 200));

    // Trim 1 and "no trim" are identical and paint the far end.
    let full = trimmed(&r, &p, Some(1.0));
    let untouched = trimmed(&r, &p, None);
    assert!(is_ink(&full, 890, 200) && is_ink(&full, 700, 200));
    assert_eq!(full.data(), untouched.data());

    // Progressive: painted pixel count grows monotonically with trim.
    let count = |pm: &Pixmap| {
        pm.pixels()
            .iter()
            .filter(|p| p.alpha() > 0 && p.red() < 128)
            .count()
    };
    let counts: Vec<usize> = [0.0, 0.25, 0.5, 0.75, 1.0]
        .iter()
        .map(|t| count(&trimmed(&r, &p, Some(*t))))
        .collect();
    assert!(
        counts.windows(2).all(|w| w[0] < w[1] || w[0] == 0),
        "{counts:?}"
    );
    let ratio = counts[2] as f32 / counts[4] as f32;
    assert!((0.45..0.6).contains(&ratio), "half trim ratio {ratio}");
}

#[test]
fn polyline_trim_cuts_the_last_segment_exactly() {
    // L shape: 400 px right then 200 px down (total 600). Trim 0.5 = 300 px.
    let p = line_project();
    let r = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let half = trimmed(&r, &p, Some(0.5));
    assert!(red_at(&half, 300, 40), "first segment painted");
    assert!(!red_at(&half, 460, 40), "past the cut (x=400) is empty");
    assert!(!red_at(&half, 500, 150), "second segment untouched");
    // Trim 0.8 = 480 px: whole first segment + 80 px down the second.
    let most = trimmed(&r, &p, Some(0.8));
    assert!(red_at(&most, 495, 40) && red_at(&most, 500, 100));
    assert!(!red_at(&most, 500, 200));
    // Trim 1: the corner turns and reaches the end.
    let full = trimmed(&r, &p, Some(1.0));
    assert!(red_at(&full, 500, 200));
}

#[test]
fn polyline_respects_opacity() {
    let p = line_project();
    let r = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let faded = render_with(&r, &p, |f| {
        for l in &mut f.layers {
            l.opacity = 0.5;
        }
    });
    let px = faded.pixel(300, 200).expect("pixel");
    assert!((100..160).contains(&px.red()), "{}", px.red());
}

fn text_project() -> MotionProject {
    project(
        r##"[
        { "id": "num", "type": "text", "x": 50, "y": 50, "width": 900, "height": 300,
          "text": "0000", "font_role": "number", "font_size": 260, "color": "#000000",
          "align": "center" }
    ]"##,
    )
}

fn with_text(r: &CpuRenderer, p: &MotionProject, s: Option<&str>) -> Pixmap {
    render_with(r, p, |f| {
        for l in &mut f.layers {
            l.text = s.map(str::to_string);
        }
    })
}

#[test]
fn count_override_renders_new_text_deterministically() {
    let p = text_project();
    let r = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let base = with_text(&r, &p, None);
    assert!(
        base.pixels().iter().any(|p| p.red() < 128),
        "base text renders"
    );

    let a = with_text(&r, &p, Some("1234"));
    let b = with_text(&r, &p, Some("1234"));
    assert_ne!(a.data(), base.data(), "override changes pixels");
    assert_eq!(a.data(), b.data(), "override render is deterministic");
    assert!(a.pixels().iter().any(|p| p.red() < 128));

    // Override equal to the authored text takes the fast path: identical to base.
    let same = with_text(&r, &p, Some("0000"));
    assert_eq!(same.data(), base.data());

    // A fresh renderer (empty cache) produces the same pixels.
    let r2 = CpuRenderer::new(&p, repo_root()).expect("renderer");
    assert_eq!(with_text(&r2, &p, Some("1234")).data(), a.data());

    // Center alignment holds: a narrower string is centered in the same box.
    let narrow = with_text(&r, &p, Some("1"));
    let cols: Vec<u32> = (0..1000)
        .filter(|x| (0..400).any(|y| is_ink(&narrow, *x, y)))
        .collect();
    let (lo, hi) = (cols[0] as f32, *cols.last().unwrap_or(&0) as f32);
    assert!(
        ((lo + hi) / 2.0 - 500.0).abs() < 40.0,
        "centered: {lo}..{hi}"
    );
}

#[test]
fn count_overrides_are_shared_safely_across_threads() {
    let p = text_project();
    let r = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let strings: Vec<String> = (0..16).map(|i| format!("{}", 1000 + i * 37)).collect();
    let serial: Vec<Vec<u8>> = strings
        .iter()
        .map(|s| with_text(&r, &p, Some(s)).data().to_vec())
        .collect();
    let r2 = CpuRenderer::new(&p, repo_root()).expect("renderer");
    let parallel: Vec<Vec<u8>> = std::thread::scope(|scope| {
        let handles: Vec<_> = strings
            .iter()
            .map(|s| {
                let (r2, p) = (&r2, &p);
                scope.spawn(move || with_text(r2, p, Some(s)).data().to_vec())
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("thread"))
            .collect()
    });
    assert_eq!(serial, parallel);
}
