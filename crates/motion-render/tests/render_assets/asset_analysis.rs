//! Subject geometry analysis (docs/ASSET_ANALYSIS.md) on synthesized pixmaps
//! and the repo's real figure cutout.

use motion_core::assets::{NormBox, RegionName};
use motion_render::analysis::{analyze, AnalyzeOptions};
use resvg::tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Rect, Transform};

fn canvas(w: u32, h: u32) -> Pixmap {
    Pixmap::new(w, h).expect("pixmap")
}

fn paint() -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(200, 30, 30, 255));
    p.anti_alias = false;
    p
}

fn fill_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32) {
    let path = PathBuilder::from_rect(Rect::from_xywh(x, y, w, h).expect("rect"));
    pm.fill_path(
        &path,
        &paint(),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn fill_circle(pm: &mut Pixmap, cx: f32, cy: f32, r: f32) {
    let path = PathBuilder::from_circle(cx, cy, r).expect("circle");
    pm.fill_path(
        &path,
        &paint(),
        FillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn person_opts() -> AnalyzeOptions {
    AnalyzeOptions {
        person: true,
        ..Default::default()
    }
}

/// 400x600 canvas: circle head (center 200,80 r 50), narrow neck, wide torso.
fn synthetic_person() -> Pixmap {
    let mut pm = canvas(400, 600);
    fill_circle(&mut pm, 200.0, 80.0, 50.0);
    fill_rect(&mut pm, 180.0, 100.0, 40.0, 70.0); // neck, overlaps circle bottom
    fill_rect(&mut pm, 80.0, 170.0, 240.0, 330.0); // shoulders + torso
    pm
}

#[test]
fn alpha_bounds_of_rect_are_exact() {
    let mut pm = canvas(200, 400);
    fill_rect(&mut pm, 20.0, 40.0, 100.0, 200.0);
    let a = analyze(&pm, &AnalyzeOptions::default());
    assert_eq!(
        a.subject_bounds,
        NormBox {
            x: 0.1,
            y: 0.1,
            width: 0.5,
            height: 0.5
        }
    );
    assert!((a.coverage - 0.25).abs() < 1e-6);
}

#[test]
fn edge_contact_top_left_only() {
    let mut pm = canvas(200, 200);
    fill_rect(&mut pm, 0.0, 0.0, 80.0, 80.0);
    let a = analyze(&pm, &AnalyzeOptions::default());
    assert!(a.edges.top && a.edges.left);
    assert!(!a.edges.bottom && !a.edges.right);
}

#[test]
fn edge_contact_uses_one_percent_margin() {
    // 1000 px wide: m = 10. A shape starting at x = 9 touches, x = 10 does not.
    let mut touching = canvas(1000, 1000);
    fill_rect(&mut touching, 9.0, 300.0, 100.0, 100.0);
    assert!(analyze(&touching, &AnalyzeOptions::default()).edges.left);
    let mut clear = canvas(1000, 1000);
    fill_rect(&mut clear, 10.0, 300.0, 100.0, 100.0);
    assert!(!analyze(&clear, &AnalyzeOptions::default()).edges.left);
}

#[test]
fn opaque_image_touches_all_edges() {
    let mut pm = canvas(64, 48);
    pm.fill(Color::from_rgba8(10, 20, 30, 255));
    let a = analyze(&pm, &person_opts());
    assert!(a.edges.top && a.edges.bottom && a.edges.left && a.edges.right);
    assert_eq!(a.coverage, 1.0);
    assert_eq!(
        a.subject_bounds,
        NormBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0
        }
    );
    assert!(a.safe_regions.is_empty());
    assert!(a.head_estimate.is_none());
    assert_eq!(a.occupancy.rows.len(), 16);
    assert!(a.occupancy.rows.iter().all(|r| r == "ffffffffffffffff"));
}

#[test]
fn transparent_image_grid_is_zero() {
    let a = analyze(&canvas(128, 96), &AnalyzeOptions::default());
    assert_eq!(a.occupancy.cols, 16);
    assert_eq!(a.occupancy.rows.len(), 16);
    assert!(a.occupancy.rows.iter().all(|r| r == "0000000000000000"));
    assert_eq!(a.coverage, 0.0);
}

#[test]
fn left_half_filled_grid() {
    let mut pm = canvas(160, 160);
    fill_rect(&mut pm, 0.0, 0.0, 80.0, 160.0);
    let a = analyze(&pm, &AnalyzeOptions::default());
    assert_eq!(a.occupancy.rows.len(), 16);
    for row in &a.occupancy.rows {
        assert_eq!(row, "ffffffff00000000");
    }
    assert!((a.occupancy.cell(0, 0) - 1.0).abs() < 1e-6);
    assert_eq!(a.occupancy.cell(15, 15), 0.0);
}

#[test]
fn partial_cell_rounds_to_hex() {
    // 16x16 image, grid 16: 1 px per cell. Use 32x32 so each cell is 2x2 = 4 px.
    let mut pm = canvas(32, 32);
    fill_rect(&mut pm, 0.0, 0.0, 1.0, 2.0); // 2 of 4 px in cell (0,0)
    let a = analyze(&pm, &AnalyzeOptions::default());
    // round(15 * 2/4) = round(7.5) = 8
    assert_eq!(a.occupancy.rows[0].as_bytes()[0] as char, '8');
}

#[test]
fn safe_region_prefers_free_side() {
    let mut pm = canvas(400, 600);
    fill_rect(&mut pm, 200.0, 0.0, 200.0, 600.0); // right half full
    let a = analyze(&pm, &AnalyzeOptions::default());
    assert!(!a.safe_regions.is_empty());
    let best = &a.safe_regions[0];
    assert!(
        matches!(
            best.name,
            RegionName::LeftUpper | RegionName::LeftMiddle | RegionName::LowerLeft
        ),
        "best was {:?}",
        best.name
    );
    assert!(best.occupancy < 0.01);
    assert!(a
        .safe_regions
        .iter()
        .all(|r| !matches!(r.name, RegionName::RightUpper | RegionName::RightMiddle)));
    for w in a.safe_regions.windows(2) {
        assert!(w[0].score >= w[1].score);
    }
    for r in &a.safe_regions {
        let b = r.rect;
        assert!(b.x >= 0.0 && b.y >= 0.0);
        assert!(b.x + b.width <= 1.0 + 1e-6 && b.y + b.height <= 1.0 + 1e-6);
        assert!(b.width * b.height >= 0.04);
    }
}

#[test]
fn fully_covered_has_no_safe_regions() {
    let mut opaque = canvas(64, 64);
    opaque.fill(Color::from_rgba8(0, 0, 0, 255));
    assert!(analyze(&opaque, &AnalyzeOptions::default())
        .safe_regions
        .is_empty());
    // Not fully opaque (alpha 254) but every pixel is subject.
    let mut nearly = canvas(64, 64);
    nearly.fill(Color::from_rgba8(0, 0, 0, 254));
    let a = analyze(&nearly, &AnalyzeOptions::default());
    assert_eq!(a.coverage, 1.0);
    assert!(a.safe_regions.is_empty());
    assert!(a.head_estimate.is_none());
}

#[test]
fn head_estimate_finds_neck() {
    let pm = synthetic_person();
    let a = analyze(&pm, &person_opts());
    let head = a.head_estimate.expect("head");
    let (w, h) = (400.0, 600.0);
    // Circle bounds: x 150..250, y 30..130.
    assert!(head.x <= 150.0 / w + 1e-4, "{head:?}");
    assert!(head.x + head.width >= 250.0 / w - 1e-4, "{head:?}");
    assert!((head.y - 30.0 / h).abs() < 1e-4, "{head:?}");
    // Bottom at/near the neck (circle bottom at y=130), within 3% of image height.
    let bottom = head.y + head.height;
    assert!((bottom - 130.0 / h).abs() <= 0.03, "bottom {bottom}");
    // It must stop well above the shoulders (y = 170).
    assert!(bottom < 170.0 / h + 1e-4);
}

#[test]
fn head_estimate_falls_back_without_neck() {
    let mut pm = canvas(400, 600);
    fill_rect(&mut pm, 100.0, 100.0, 200.0, 300.0);
    let a = analyze(&pm, &person_opts());
    let head = a.head_estimate.expect("fallback head");
    assert!((head.y - 100.0 / 600.0).abs() < 1e-4);
    assert!(
        (head.height - 0.22 * 300.0 / 600.0).abs() < 2.0 / 600.0,
        "{head:?}"
    );
    assert!((head.x - 0.25).abs() < 1e-4);
    assert!((head.width - 0.5).abs() < 1e-4);
}

#[test]
fn head_estimate_absent_when_not_person() {
    let pm = synthetic_person();
    assert!(analyze(&pm, &AnalyzeOptions::default())
        .head_estimate
        .is_none());
}

#[test]
fn real_figure_has_plausible_head() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/test_images/figure.png"
    );
    let pm = Pixmap::load_png(path).expect("figure.png");
    let a = analyze(&pm, &person_opts());
    let head = a.head_estimate.expect("head estimate");
    assert!(head.y + head.height <= 0.35, "head too low: {head:?}");
    let cx = head.x + head.width / 2.0;
    assert!((0.3..=0.7).contains(&cx), "head off-center: {head:?}");
    assert!(head.width > 0.0 && head.height > 0.0);
}

#[test]
fn analysis_is_deterministic() {
    let pm = synthetic_person();
    assert_eq!(analyze(&pm, &person_opts()), analyze(&pm, &person_opts()));
}

#[test]
fn large_image_is_fast_enough() {
    let mut pm = canvas(1024, 1536);
    fill_circle(&mut pm, 512.0, 400.0, 300.0);
    let start = std::time::Instant::now();
    let a = analyze(&pm, &person_opts());
    let elapsed = start.elapsed();
    assert!(a.coverage > 0.0);
    // Generous bound so debug builds and loaded CI machines do not flake;
    // release is well under 100 ms.
    assert!(elapsed.as_millis() < 3000, "took {elapsed:?}");
}
