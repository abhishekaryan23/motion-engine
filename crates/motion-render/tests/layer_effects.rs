//! Renderer support for the 0.14-0.16 layer fields (glyph poses, polyline
//! points, blur, homography) and the frame's post list. Frames are resolved
//! by the timeline, then the new `ResolvedLayer` fields are set by hand so
//! these tests exercise the renderer alone.

use std::path::Path;

use motion_core::scene::{GlyphPose, PostKind};
use motion_core::timeline::{Affine, ResolvedFrame, ResolvedPost};
use motion_core::{evaluate_frame, MotionProject};
use motion_render::postfx::apply_post;
use motion_render::{CpuRenderer, Renderer};
use resvg::tiny_skia::Pixmap;

/// The identity transform (content offset 0 in box space).
const ID: Affine = Affine {
    a: 1.0,
    b: 0.0,
    c: 0.0,
    d: 1.0,
    e: 0.0,
    f: 0.0,
};

const W: u32 = 600;
const H: u32 = 300;

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn project(layers: &str) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "layer_effects", "duration_seconds": 1.0 }},
        "canvas": {{ "width": {W}, "height": {H}, "fps": 30, "background": "#FFFFFF" }},
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

fn text_project(text: &str, extra: &str) -> MotionProject {
    project(&format!(
        r##"[ {{ "id": "t", "type": "text", "text": "{text}", "font_role": "number",
            "font_size": 110, "color": "#000000", "x": 20, "y": 10,
            "width": 560, "height": 280 {extra} }} ]"##
    ))
}

fn render_with<'a>(
    r: &CpuRenderer,
    p: &'a MotionProject,
    tweak: impl FnOnce(&mut ResolvedFrame<'a>),
) -> Pixmap {
    let mut f = evaluate_frame(p, 0).expect("evaluate");
    tweak(&mut f);
    r.render(&f).expect("render")
}

fn renderer(p: &MotionProject) -> CpuRenderer {
    CpuRenderer::new(p, repo_root()).expect("renderer")
}

fn rgb(pm: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let p = pm.pixel(x, y).expect("pixel in range");
    [p.red(), p.green(), p.blue()]
}

/// Darkness (0..=255) of a pixel on a white background.
fn dark(pm: &Pixmap, x: u32, y: u32) -> u32 {
    255 - rgb(pm, x, y)[0] as u32
}

/// Bounding box `(x0, y0, x1, y1)` (inclusive) of pixels darker than `min`.
fn ink_bounds(pm: &Pixmap, min: u32) -> Option<(u32, u32, u32, u32)> {
    let mut b: Option<(u32, u32, u32, u32)> = None;
    for y in 0..pm.height() {
        for x in 0..pm.width() {
            if dark(pm, x, y) >= min {
                b = Some(match b {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
    }
    b
}

fn total_dark(pm: &Pixmap) -> u64 {
    (0..pm.height())
        .flat_map(|y| (0..pm.width()).map(move |x| (x, y)))
        .map(|(x, y)| dark(pm, x, y) as u64)
        .sum()
}

fn column_has_ink(pm: &Pixmap, x: u32) -> bool {
    (0..pm.height()).any(|y| dark(pm, x, y) >= 128)
}

/// Column ranges of ink separated by at least `gap` empty columns.
fn clusters(pm: &Pixmap, gap: u32) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    for x in 0..pm.width() {
        if column_has_ink(pm, x) {
            match out.last_mut() {
                Some(last) if x - last.1 <= gap => last.1 = x,
                _ => out.push((x, x)),
            }
        }
    }
    out
}

fn pose(dx: f32, dy: f32, scale: f32, rotation: f32, opacity: f32) -> GlyphPose {
    GlyphPose {
        dx,
        dy,
        scale,
        rotation,
        opacity,
    }
}

// --- glyph poses --------------------------------------------------------------

#[test]
fn rest_glyph_poses_draw_exactly_like_plain_text() {
    let p = text_project("AB CD", "");
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    assert!(ink_bounds(&plain, 128).is_some());
    for poses in [
        vec![GlyphPose::REST; 4],
        vec![GlyphPose::REST; 40],
        vec![GlyphPose::REST; 2],
        vec![],
    ] {
        let posed = render_with(&r, &p, |f| f.layers[0].glyphs = Some(poses.clone()));
        assert_eq!(posed.data(), plain.data(), "{} poses", poses.len());
    }
}

#[test]
fn an_offset_pose_moves_only_its_glyph() {
    // Glyph 0 = A, glyph 1 = B (the space is not a glyph).
    let p = text_project("A B", "");
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    let cl = clusters(&plain, 6);
    assert_eq!(cl.len(), 2, "two glyph clusters: {cl:?}");
    let rest = pose(0.0, 0.0, 1.0, 0.0, 1.0);
    let down = pose(0.0, 60.0, 1.0, 0.0, 1.0);
    let posed = render_with(&r, &p, |f| f.layers[0].glyphs = Some(vec![rest, down]));

    // The A columns are pixel-identical; the B columns carry the same ink 60 px lower.
    for x in cl[0].0.saturating_sub(3)..=cl[0].1 + 3 {
        for y in 0..H {
            assert_eq!(rgb(&posed, x, y), rgb(&plain, x, y), "A at ({x},{y})");
        }
    }
    let band = |pm: &Pixmap| {
        let mut top = H;
        let mut bottom = 0;
        let mut sum = 0u64;
        for x in cl[1].0..=cl[1].1 {
            for y in 0..H {
                if dark(pm, x, y) >= 128 {
                    top = top.min(y);
                    bottom = bottom.max(y);
                }
                sum += dark(pm, x, y) as u64;
            }
        }
        (top, bottom, sum)
    };
    let (pt, pb, ps) = band(&plain);
    let (qt, qb, qs) = band(&posed);
    assert_eq!((qt as i32 - pt as i32, qb as i32 - pb as i32), (60, 60));
    let ratio = qs as f64 / ps as f64;
    assert!((0.97..1.03).contains(&ratio), "ink conserved: {ratio}");
}

#[test]
fn scale_rotation_and_opacity_act_about_the_glyph_centre() {
    let p = text_project("H", "");
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    let (x0, y0, x1, y1) = ink_bounds(&plain, 128).expect("ink");
    let (w0, h0) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
    let (cx0, cy0) = ((x0 + x1) as f32 / 2.0, (y0 + y1) as f32 / 2.0);
    let bounds_of = |pm: &Pixmap| {
        let (a, b, c, d) = ink_bounds(pm, 128).expect("ink");
        (
            (c - a + 1) as f32,
            (d - b + 1) as f32,
            (a + c) as f32 / 2.0,
            (b + d) as f32 / 2.0,
        )
    };
    let one = |g: GlyphPose| render_with(&r, &p, |f| f.layers[0].glyphs = Some(vec![g]));

    // Scale 0.5 keeps the centre.
    let (w, h, cx, cy) = bounds_of(&one(pose(0.0, 0.0, 0.5, 0.0, 1.0)));
    assert!(
        (w - w0 / 2.0).abs() <= 2.0 && (h - h0 / 2.0).abs() <= 2.0,
        "{w} {h}"
    );
    assert!(
        (cx - cx0).abs() <= 1.5 && (cy - cy0).abs() <= 1.5,
        "{cx} {cy}"
    );

    // Rotation by 90 degrees swaps width and height, same centre.
    let (w, h, cx, cy) = bounds_of(&one(pose(0.0, 0.0, 1.0, 90.0, 1.0)));
    assert!(
        (w - h0).abs() <= 2.0 && (h - w0).abs() <= 2.0,
        "{w} {h} vs {w0} {h0}"
    );
    assert!(
        (cx - cx0).abs() <= 1.5 && (cy - cy0).abs() <= 1.5,
        "{cx} {cy}"
    );

    // Offset moves the centre by exactly (dx, dy) and keeps the size.
    let (w, h, cx, cy) = bounds_of(&one(pose(50.0, -20.0, 1.0, 0.0, 1.0)));
    assert!((w - w0).abs() <= 1.0 && (h - h0).abs() <= 1.0);
    assert!((cx - cx0 - 50.0).abs() <= 1.0 && (cy - cy0 + 20.0).abs() <= 1.0);

    // Half opacity: the stem of the H is mid grey; zero opacity: nothing.
    let half = one(pose(0.0, 0.0, 1.0, 0.0, 0.5));
    let stem = dark(&half, x0 + 4, (y0 + y1) / 2);
    assert!((120..=135).contains(&stem), "half-opacity ink {stem}");
    let none = one(pose(0.0, 0.0, 1.0, 0.0, 0.0));
    assert_eq!(total_dark(&none), 0);
}

#[test]
fn glyph_indices_run_in_reading_order_across_lines() {
    let p = text_project(r"AB\nCD", "");
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    // Move glyph 2 (C, the first glyph of line two) far right.
    let posed = render_with(&r, &p, |f| {
        f.layers[0].glyphs = Some(vec![
            GlyphPose::REST,
            GlyphPose::REST,
            pose(300.0, 0.0, 1.0, 0.0, 1.0),
        ])
    });
    let (_, y0, _, y1) = ink_bounds(&plain, 128).expect("ink");
    let mid = (y0 + y1) / 2;
    // First line (above `mid`) is untouched.
    for y in 0..mid {
        for x in 0..W {
            assert_eq!(rgb(&posed, x, y), rgb(&plain, x, y));
        }
    }
    // The second line changed.
    assert!((mid..H).any(|y| (0..W).any(|x| rgb(&posed, x, y) != rgb(&plain, x, y))));
}

#[test]
fn glyph_poses_apply_to_count_overrides_too() {
    let p = text_project("100", "");
    let r = renderer(&p);
    let shown = |poses: Option<Vec<GlyphPose>>| {
        render_with(&r, &p, |f| {
            f.layers[0].text = Some("873".to_string());
            f.layers[0].glyphs = poses;
        })
    };
    let plain = shown(None);
    assert_eq!(plain.data(), shown(Some(vec![GlyphPose::REST; 3])).data());
    let moved = shown(Some(vec![
        GlyphPose::REST,
        pose(0.0, 50.0, 1.0, 0.0, 1.0),
        GlyphPose::REST,
    ]));
    assert_ne!(moved.data(), plain.data());
    let ratio = total_dark(&moved) as f64 / total_dark(&plain) as f64;
    assert!((0.97..1.03).contains(&ratio));
}

// --- polyline points ----------------------------------------------------------

#[test]
fn polyline_points_override_replaces_the_authored_points() {
    let p = project(
        r##"[ { "id": "line", "type": "polyline", "x": 0, "y": 0, "width": 600, "height": 300,
              "points": [[50, 60], [550, 60]], "stroke": { "color": "#000000", "width": 12 } } ]"##,
    );
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    assert!(dark(&plain, 300, 60) > 200 && dark(&plain, 300, 200) == 0);
    let same = render_with(&r, &p, |f| {
        f.layers[0].points = Some(vec![[50.0, 60.0], [550.0, 60.0]])
    });
    assert_eq!(same.data(), plain.data());
    let moved = render_with(&r, &p, |f| {
        f.layers[0].points = Some(vec![[50.0, 200.0], [550.0, 200.0]])
    });
    assert!(dark(&moved, 300, 200) > 200 && dark(&moved, 300, 60) == 0);
    // Trim applies to the replaced points.
    let trimmed = render_with(&r, &p, |f| {
        f.layers[0].points = Some(vec![[50.0, 200.0], [550.0, 200.0]]);
        f.layers[0].trim = Some(0.5);
    });
    assert!(dark(&trimmed, 200, 200) > 200 && dark(&trimmed, 450, 200) == 0);
}

// --- blur ---------------------------------------------------------------------

const RECT: &str = r##"[ { "id": "r", "type": "rectangle", "x": 200, "y": 100, "width": 200,
    "height": 100, "fill": "#000000" } ]"##;

#[test]
fn blur_spreads_alpha_and_conserves_ink() {
    let p = project(RECT);
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    assert_eq!(dark(&plain, 190, 150), 0);
    let blurred = render_with(&r, &p, |f| f.layers[0].blur = Some(20.0));
    // Beyond the edge there is now a soft halo; deep inside it stays solid.
    let halo = dark(&blurred, 190, 150);
    assert!(halo > 10 && halo < 200, "halo {halo}");
    assert!(dark(&blurred, 300, 150) >= 250);
    // Falls off with distance.
    assert!(dark(&blurred, 185, 150) < halo);
    // A blur conserves the total coverage (nothing reaches the canvas border).
    let (a, b) = (total_dark(&plain) as f64, total_dark(&blurred) as f64);
    assert!((b / a - 1.0).abs() < 0.015, "ink {a} -> {b}");
    // A larger radius spreads further.
    let wide = render_with(&r, &p, |f| f.layers[0].blur = Some(60.0));
    let reach = |pm: &Pixmap| (0..200u32).filter(|&d| dark(pm, 199 - d, 150) > 2).count();
    assert!(
        reach(&wide) > reach(&blurred) + 10,
        "{} {}",
        reach(&wide),
        reach(&blurred)
    );
    // None / sub-visible radii draw exactly as before.
    for b in [None, Some(0.0), Some(0.01), Some(f32::NAN), Some(-4.0)] {
        let same = render_with(&r, &p, |f| f.layers[0].blur = b);
        assert_eq!(same.data(), plain.data(), "{b:?}");
    }
}

#[test]
fn blur_composites_with_opacity_and_for_groups() {
    let p = project(RECT);
    let r = renderer(&p);
    let full = render_with(&r, &p, |f| f.layers[0].blur = Some(16.0));
    let half = render_with(&r, &p, |f| {
        f.layers[0].blur = Some(16.0);
        f.layers[0].opacity = 0.5;
    });
    let d = dark(&half, 300, 150) as i32;
    assert!((120..=135).contains(&d), "{d}");
    let ratio = total_dark(&half) as f64 / total_dark(&full) as f64;
    assert!((ratio - 0.5).abs() < 0.02, "{ratio}");

    // A group blurs its composited children as one image.
    let g = project(
        r##"[ { "id": "g", "type": "group", "width": 600, "height": 300, "children": [
            { "id": "a", "type": "rectangle", "x": 200, "y": 100, "width": 100, "height": 100, "fill": "#000000" },
            { "id": "b", "type": "rectangle", "x": 300, "y": 100, "width": 100, "height": 100, "fill": "#000000" }
        ] } ]"##,
    );
    let rg = renderer(&g);
    let plain = render_with(&rg, &g, |_| {});
    let blurred = render_with(&rg, &g, |f| f.layers[0].blur = Some(20.0));
    assert_ne!(blurred.data(), plain.data());
    let halo = dark(&blurred, 190, 150);
    assert!(halo > 10 && halo < 200, "{halo}");
    // The seam between the two children is blurred away (still solid ink).
    assert!(dark(&blurred, 300, 150) >= 250);
    let (a, b) = (total_dark(&plain) as f64, total_dark(&blurred) as f64);
    assert!((b / a - 1.0).abs() < 0.015);
}

// --- projective ---------------------------------------------------------------

fn homography_of(a: Affine) -> [f32; 9] {
    [a.a, a.c, a.e, a.b, a.d, a.f, 0.0, 0.0, 1.0]
}

/// Homography mapping the box corners `(0,0),(w,0),(w,h),(0,h)` to `quad`.
fn quad_homography(w: f32, h: f32, quad: [[f32; 2]; 4]) -> [f32; 9] {
    let src = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    let mut m = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let (x, y) = (src[i][0] as f64, src[i][1] as f64);
        let (u, v) = (quad[i][0] as f64, quad[i][1] as f64);
        m[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        m[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for c in 0..8 {
        let piv = (c..8)
            .max_by(|&a, &b| m[a][c].abs().total_cmp(&m[b][c].abs()))
            .expect("pivot");
        m.swap(c, piv);
        let pivot_row = m[c];
        for (r, row) in m.iter_mut().enumerate() {
            if r != c {
                let k = row[c] / pivot_row[c];
                for (x, p) in row.iter_mut().zip(&pivot_row).skip(c) {
                    *x -= k * p;
                }
            }
        }
    }
    let mut out = [0.0f32; 9];
    for i in 0..8 {
        out[i] = (m[i][8] / m[i][i]) as f32;
    }
    out[8] = 1.0;
    out
}

fn max_channel_diff(a: &Pixmap, b: &Pixmap) -> i32 {
    a.data()
        .iter()
        .zip(b.data())
        .map(|(x, y)| (*x as i32 - *y as i32).abs())
        .max()
        .unwrap_or(0)
}

/// A rectangle with a stroke, a rounded rectangle and text.
const SHAPES: &str = r##"[
    { "id": "r", "type": "rectangle", "x": 40, "y": 30, "width": 200, "height": 120,
      "fill": "#CC2222", "stroke": { "color": "#1133CC", "width": 8 } },
    { "id": "rr", "type": "rounded_rectangle", "x": 300, "y": 40, "width": 180, "height": 90,
      "radius": 24, "fill": "#22AA44" },
    { "id": "t", "type": "text", "text": "Hi", "font_role": "number", "font_size": 90,
      "color": "#101010", "x": 60, "y": 170, "width": 300, "height": 110 }
]"##;

#[test]
fn identity_homography_matches_the_affine_draw() {
    let p = project(SHAPES);
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    // Each layer's own box->canvas affine as the homography, content offset 0.
    let warped = render_with(&r, &p, |f| {
        for l in &mut f.layers {
            l.projective = Some(homography_of(l.transform));
            l.content_transform = ID;
        }
    });
    let diff = max_channel_diff(&plain, &warped);
    assert!(diff <= 2, "max channel diff {diff}");
    assert!(total_dark(&plain) > 0);
}

#[test]
fn identity_homography_at_the_origin_with_opacity_and_clip() {
    let p = project(
        r##"[ { "id": "r", "type": "rectangle", "x": 0, "y": 0, "width": 300, "height": 200,
              "fill": "#000000", "opacity": 0.5, "clip": { "left": 0.5, "right": 0.0, "top": 0.0, "bottom": 0.0 } } ]"##,
    );
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    let warped = render_with(&r, &p, |f| {
        f.layers[0].projective = Some([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]);
    });
    assert!(max_channel_diff(&plain, &warped) <= 2);
    // Clip kept in box space: left half empty, right half 50 % ink.
    assert_eq!(dark(&warped, 100, 100), 0);
    let d = dark(&warped, 200, 100) as i32;
    assert!((125..=130).contains(&d), "{d}");
}

#[test]
fn homography_is_scale_invariant_and_translation_moves_the_layer() {
    let p = project(SHAPES);
    let r = renderer(&p);
    let base = |f: &mut ResolvedFrame<'_>, k: f32| {
        for l in &mut f.layers {
            let mut h = homography_of(l.transform);
            for v in &mut h {
                *v *= k;
            }
            l.projective = Some(h);
            l.content_transform = ID;
        }
    };
    let a = render_with(&r, &p, |f| base(f, 1.0));
    let b = render_with(&r, &p, |f| base(f, 4.0));
    let c = render_with(&r, &p, |f| base(f, -1.0));
    assert!(max_channel_diff(&a, &b) <= 1);
    assert!(max_channel_diff(&a, &c) <= 1);
    // Sub-pixel translation: ink is conserved, interior stays solid.
    let shifted = render_with(&r, &p, |f| {
        for l in &mut f.layers {
            l.projective = Some(homography_of(
                Affine::translate(30.5, 20.25).then_apply(l.transform),
            ));
            l.content_transform = ID;
        }
    });
    let ratio = total_dark(&shifted) as f64 / total_dark(&a) as f64;
    assert!((0.98..1.02).contains(&ratio), "{ratio}");
    assert_eq!(rgb(&shifted, 100 + 30, 70 + 20), rgb(&a, 100, 70));
}

#[test]
fn a_perspective_quad_projects_the_box_with_bilinear_edges() {
    let p = project(
        r##"[ { "id": "r", "type": "rectangle", "x": 0, "y": 0, "width": 200, "height": 100,
              "fill": "#000000" } ]"##,
    );
    let r = renderer(&p);
    // Top edge narrower and lower than the bottom edge.
    let quad = [[260.0, 40.0], [340.0, 40.0], [400.0, 200.0], [200.0, 200.0]];
    let h = quad_homography(200.0, 100.0, quad);
    let img = render_with(&r, &p, |f| {
        f.layers[0].projective = Some(h);
        f.layers[0].content_transform = ID;
    });
    // Inside: solid. Outside the slanted sides and above/below: white.
    assert!(dark(&img, 300, 120) >= 250);
    assert!(dark(&img, 300, 45) >= 250, "just under the top edge");
    assert_eq!(dark(&img, 300, 30), 0, "above the quad");
    assert_eq!(dark(&img, 225, 60), 0, "left of the slanted side");
    assert_eq!(dark(&img, 375, 60), 0, "right of the slanted side");
    assert!(
        dark(&img, 215, 190) >= 250,
        "bottom-left corner region is inside"
    );
    assert_eq!(dark(&img, 300, 210), 0, "below the quad");
    // Area of the trapezoid: (80 + 200) / 2 * 160.
    let area = total_dark(&img) as f64 / 255.0;
    assert!((area - 22400.0).abs() / 22400.0 < 0.01, "area {area}");
    // Edges are antialiased: some partial-coverage pixels exist.
    let partial = (0..W)
        .flat_map(|x| (0..H).map(move |y| (x, y)))
        .filter(|&(x, y)| (20..235).contains(&dark(&img, x, y)))
        .count();
    assert!(partial > 100, "{partial}");
}

#[test]
fn horizon_crossing_and_degenerate_homographies_do_not_panic() {
    let p = project(SHAPES);
    let r = renderer(&p);
    let draw = |h: [f32; 9]| {
        render_with(&r, &p, |f| {
            for l in &mut f.layers {
                l.projective = Some(h);
                l.content_transform = ID;
            }
        })
    };
    // Singular, non-finite and horizon-crossing matrices.
    let zero = draw([0.0; 9]);
    assert_eq!(total_dark(&zero), 0);
    let nan = draw([f32::NAN; 9]);
    assert_eq!(total_dark(&nan), 0);
    // w = 1 - x / 150 crosses zero inside the layer: only the near part draws.
    let crossing = draw([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0 / 150.0, 0.0, 1.0]);
    assert_eq!(crossing.width(), W);
    // Extreme minification supersamples without panicking.
    let tiny = draw([0.05, 0.0, 100.0, 0.0, 0.05, 50.0, 0.0, 0.0, 1.0]);
    assert!(total_dark(&tiny) > 0);
}

#[test]
fn blur_and_homography_combine() {
    let p = project(SHAPES);
    let r = renderer(&p);
    let sharp = render_with(&r, &p, |f| {
        for l in &mut f.layers {
            l.projective = Some(homography_of(l.transform));
            l.content_transform = ID;
        }
    });
    let soft = render_with(&r, &p, |f| {
        for l in &mut f.layers {
            l.projective = Some(homography_of(l.transform));
            l.content_transform = ID;
            l.blur = Some(10.0);
        }
    });
    assert_ne!(sharp.data(), soft.data());
    let (a, b) = (total_dark(&sharp) as f64, total_dark(&soft) as f64);
    assert!((b / a - 1.0).abs() < 0.03);
}

// --- post list ------------------------------------------------------------------

#[test]
fn the_renderer_applies_the_frames_post_list_after_compositing() {
    let p = project(SHAPES);
    let r = renderer(&p);
    let plain = render_with(&r, &p, |_| {});
    let kinds = [
        PostKind::Vignette { amount: 0.8 },
        PostKind::ChromaticAberration {
            shift: 3.0,
            angle_deg: 15.0,
        },
        PostKind::Grain {
            amount: 0.4,
            seed: 5,
        },
    ];
    let with_post = render_with(&r, &p, |f| {
        for k in &kinds {
            f.post.push(ResolvedPost {
                kind: k,
                strength: 0.7,
                time: 0.25,
            });
        }
    });
    let mut expected = plain.clone();
    let posts: Vec<ResolvedPost<'_>> = kinds
        .iter()
        .map(|k| ResolvedPost {
            kind: k,
            strength: 0.7,
            time: 0.25,
        })
        .collect();
    apply_post(&mut expected, &posts);
    assert_eq!(with_post.data(), expected.data());
    assert_ne!(with_post.data(), plain.data());
    // A zero-strength post is invisible.
    let zero = render_with(&r, &p, |f| {
        f.post.push(ResolvedPost {
            kind: &kinds[0],
            strength: 0.0,
            time: 0.0,
        })
    });
    assert_eq!(zero.data(), plain.data());
}
