//! Subject geometry measured once at ingestion (0.5): alpha bounds, edge
//! contact, occupancy grid, safe type regions, head estimate. Pure,
//! deterministic, no ML. Spec: docs/ASSET_ANALYSIS.md.

use motion_core::assets::{
    AssetAnalysis, EdgeContact, NormBox, OccupancyGrid, RegionName, SafeRegion,
};
use resvg::tiny_skia::Pixmap;

/// Analysis knobs (defaults are the documented values).
#[derive(Debug, Clone, Copy)]
pub struct AnalyzeOptions {
    /// Alpha (0..=255) at or above which a pixel belongs to the subject.
    pub alpha_threshold: u8,
    /// Occupancy grid size (cols = rows).
    pub grid: u32,
    /// The image depicts a person: estimate a head region.
    pub person: bool,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        AnalyzeOptions {
            alpha_threshold: 16,
            grid: 16,
            person: false,
        }
    }
}

/// Maximum hex-cell value that still counts as "free" for a safe region.
const FREE_CELL_MAX: u8 = 2;
/// Minimum normalized area of a reported safe region.
const MIN_REGION_AREA: f32 = 0.04;

/// Mean un-premultiplied chroma below which an image counts as monochrome.
const MONOCHROME_CHROMA_MAX: f64 = 0.06;

/// Fraction of subject pixels that must sit within `FLAT_INK_LUMA_BAND` of the
/// median luma for the artwork to count as flat single-ink line art (a
/// greyscale photo cutout has a wide tonal range and fails this).
const FLAT_INK_FRACTION: f64 = 0.85;
/// Half-width of the flat-ink luma band (luma in 0..=1).
const FLAT_INK_LUMA_BAND: f64 = 0.12;

/// Luma bin (0..=255) of a premultiplied pixel, un-premultiplied (`a > 0`).
fn pixel_luma(r: u8, g: u8, b: u8, a: u8) -> usize {
    let l = (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b))
        / f64::from(a.max(1));
    (l.clamp(0.0, 1.0) * 255.0).round() as usize
}

/// At least [`FLAT_INK_FRACTION`] of the pixels lie within
/// [`FLAT_INK_LUMA_BAND`] of the median luma.
fn is_flat_ink(hist: &[u64; 256], total: u64) -> bool {
    if total == 0 {
        return false;
    }
    let half = total.div_ceil(2);
    let mut acc = 0u64;
    let mut median = 0usize;
    for (bin, &n) in hist.iter().enumerate() {
        acc += n;
        if acc >= half {
            median = bin;
            break;
        }
    }
    let band = FLAT_INK_LUMA_BAND * 255.0;
    let near: u64 = hist
        .iter()
        .enumerate()
        .filter(|(bin, _)| (*bin as f64 - median as f64).abs() <= band)
        .map(|(_, &n)| n)
        .sum();
    near as f64 >= FLAT_INK_FRACTION * total as f64
}

/// Chroma `(max - min) / 255` of a premultiplied pixel, un-premultiplied
/// (`a > 0`).
fn pixel_chroma(r: u8, g: u8, b: u8, a: u8) -> f64 {
    let hi = r.max(g).max(b);
    let lo = r.min(g).min(b);
    (f64::from(hi - lo) / f64::from(a.max(1))).clamp(0.0, 1.0)
}

/// Zones in documented (tie-break) order: name, x, y, w, h (image fractions).
const ZONES: [(RegionName, f32, f32, f32, f32); 8] = [
    (RegionName::LeftUpper, 0.0, 0.0, 0.5, 0.5),
    (RegionName::RightUpper, 0.5, 0.0, 0.5, 0.5),
    (RegionName::LeftMiddle, 0.0, 0.25, 0.5, 0.5),
    (RegionName::RightMiddle, 0.5, 0.25, 0.5, 0.5),
    (RegionName::LowerLeft, 0.0, 0.5, 0.5, 0.5),
    (RegionName::LowerRight, 0.5, 0.5, 0.5, 0.5),
    (RegionName::Top, 0.0, 0.0, 1.0, 1.0 / 3.0),
    (RegionName::Bottom, 0.0, 2.0 / 3.0, 1.0, 1.0 / 3.0),
];

/// Measure `img`. Opaque images: subject = whole frame, all edges touched,
/// full occupancy, no safe regions, no head estimate.
pub fn analyze(img: &Pixmap, opts: &AnalyzeOptions) -> AssetAnalysis {
    let w = img.width().max(1);
    let h = img.height().max(1);
    let g = opts.grid.max(1);
    let gs = g as usize;
    let thr = opts.alpha_threshold;

    let col_ranges: Vec<(usize, usize)> = (0..g).map(|c| cell_range(c, g, w)).collect();
    let row_ranges: Vec<(usize, usize)> = (0..g).map(|r| cell_range(r, g, h)).collect();

    let width = w as usize;
    let mut counts = vec![0u64; gs * gs];
    let mut row_cnt = vec![0u64; gs];
    // Per-row (min_x, max_x) of subject pixels.
    let mut spans: Vec<Option<(u32, u32)>> = Vec::with_capacity(h as usize);
    let mut subject_total: u64 = 0;
    let mut any_transparent = false;
    // Sum of un-premultiplied chroma over subject pixels (monochrome test).
    let mut chroma_sum = 0f64;
    // Histogram of un-premultiplied luma (0..=255) over subject pixels.
    let mut luma_hist = [0u64; 256];
    // Premultiplied colour sums and alpha sum over subject pixels (mean colour).
    let (mut sum_rgb, mut sum_alpha) = ([0u64; 3], 0u64);
    let (mut min_x, mut max_x) = (u32::MAX, 0u32);
    let (mut min_y, mut max_y) = (u32::MAX, 0u32);

    for (y, row) in img.pixels().chunks(width).enumerate() {
        let mut first: Option<u32> = None;
        let mut last = 0u32;
        let mut n = 0u64;
        for (x, p) in row.iter().enumerate() {
            let a = p.alpha();
            any_transparent |= a < 255;
            if a >= thr {
                chroma_sum += pixel_chroma(p.red(), p.green(), p.blue(), a);
                luma_hist[pixel_luma(p.red(), p.green(), p.blue(), a)] += 1;
                sum_rgb[0] += u64::from(p.red());
                sum_rgb[1] += u64::from(p.green());
                sum_rgb[2] += u64::from(p.blue());
                sum_alpha += u64::from(a);
                if first.is_none() {
                    first = Some(x as u32);
                }
                last = x as u32;
                n += 1;
            }
        }
        subject_total += n;
        if let Some(f) = first {
            min_x = min_x.min(f);
            max_x = max_x.max(last);
            min_y = min_y.min(y as u32);
            max_y = max_y.max(y as u32);
        }
        spans.push(first.map(|f| (f, last)));
        if n > 0 {
            for (c, &(x0, x1)) in col_ranges.iter().enumerate() {
                row_cnt[c] = row[x0..x1].iter().filter(|p| p.alpha() >= thr).count() as u64;
            }
            for (r, &(y0, y1)) in row_ranges.iter().enumerate() {
                if y >= y0 && y < y1 {
                    for (c, cnt) in row_cnt.iter().enumerate() {
                        counts[r * gs + c] += cnt;
                    }
                }
            }
        }
    }

    let has_subject = subject_total > 0;
    let monochrome = has_subject
        && chroma_sum / (subject_total as f64) < MONOCHROME_CHROMA_MAX
        && is_flat_ink(&luma_hist, subject_total);
    // Alpha-weighted mean of the un-premultiplied colour: Σ premultiplied / Σ alpha.
    let mean_color = (sum_alpha > 0).then(|| {
        let mean = |s: u64| ((255 * s + sum_alpha / 2) / sum_alpha).min(255) as u8;
        [mean(sum_rgb[0]), mean(sum_rgb[1]), mean(sum_rgb[2])]
    });

    if !any_transparent {
        let mut a = opaque_analysis(g);
        a.monochrome = monochrome;
        a.mean_color = mean_color;
        return a;
    }

    let coverage = (subject_total as f64 / (w as f64 * h as f64)) as f32;
    let subject_bounds = if has_subject {
        NormBox {
            x: min_x as f32 / w as f32,
            y: min_y as f32 / h as f32,
            width: (max_x + 1 - min_x) as f32 / w as f32,
            height: (max_y + 1 - min_y) as f32 / h as f32,
        }
    } else {
        NormBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    };

    let edges = if has_subject {
        let mx = margin(w);
        let my = margin(h);
        EdgeContact {
            top: min_y < my,
            bottom: max_y + my >= h,
            left: min_x < mx,
            right: max_x + mx >= w,
        }
    } else {
        EdgeContact::default()
    };

    // Quantize to hex values: round(15 * subject / cell pixels).
    let mut values = vec![0u8; gs * gs];
    for (r, &(y0, y1)) in row_ranges.iter().enumerate() {
        for (c, &(x0, x1)) in col_ranges.iter().enumerate() {
            let cell_px = ((x1 - x0) * (y1 - y0)) as u64;
            let cnt = counts[r * gs + c].min(cell_px);
            values[r * gs + c] = ((30 * cnt + cell_px) / (2 * cell_px)).min(15) as u8;
        }
    }
    let rows: Vec<String> = (0..gs)
        .map(|r| {
            (0..gs)
                .map(|c| char::from_digit(values[r * gs + c] as u32, 16).unwrap_or('0'))
                .collect()
        })
        .collect();

    let safe_regions = safe_regions(&values, gs);

    let head_estimate = if opts.person && has_subject {
        head_estimate(&spans, min_y, max_y, w, h)
    } else {
        None
    };

    AssetAnalysis {
        subject_bounds,
        edges,
        coverage,
        occupancy: OccupancyGrid { cols: g, rows },
        safe_regions,
        head_estimate,
        monochrome,
        mean_color,
    }
}

fn opaque_analysis(g: u32) -> AssetAnalysis {
    AssetAnalysis {
        subject_bounds: NormBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        },
        edges: EdgeContact {
            top: true,
            bottom: true,
            left: true,
            right: true,
        },
        coverage: 1.0,
        occupancy: OccupancyGrid {
            cols: g,
            rows: (0..g).map(|_| "f".repeat(g as usize)).collect(),
        },
        safe_regions: Vec::new(),
        head_estimate: None,
        monochrome: false,
        mean_color: None,
    }
}

/// `m = max(1, round(0.01 * side))`.
fn margin(side: u32) -> u32 {
    ((0.01 * side as f64).round() as u32).max(1)
}

/// Pixel range `[floor(i*n/g), floor((i+1)*n/g))` of grid cell `i`; empty
/// ranges clamp to one pixel.
fn cell_range(i: u32, g: u32, n: u32) -> (usize, usize) {
    let g = g as u64;
    let n = n as u64;
    let mut a = i as u64 * n / g;
    let mut b = (i as u64 + 1) * n / g;
    if a >= n {
        a = n - 1;
    }
    if b <= a {
        b = a + 1;
    }
    (a as usize, b as usize)
}

/// Best all-free rectangle per zone, filtered by area and sorted by score.
fn safe_regions(values: &[u8], g: usize) -> Vec<SafeRegion> {
    let gf = g as f32;
    let mut out: Vec<SafeRegion> = Vec::new();
    for &(name, zx, zy, zw, zh) in ZONES.iter() {
        let in_zone = |i: usize, z0: f32, zl: f32| {
            let center = (i as f32 + 0.5) / gf;
            center >= z0 && center < z0 + zl
        };
        let cols: Vec<usize> = (0..g).filter(|&c| in_zone(c, zx, zw)).collect();
        let rows: Vec<usize> = (0..g).filter(|&r| in_zone(r, zy, zh)).collect();
        let (Some(&c0), Some(&r0)) = (cols.first(), rows.first()) else {
            continue;
        };
        let (zc, zr) = (cols.len(), rows.len());
        // Best rectangle (area, top, left, width, height), zone-local cells;
        // ties go to the top-most, then left-most.
        let mut heights = vec![0usize; zc];
        let mut best: Option<(usize, usize, usize, usize, usize)> = None;
        for lr in 0..zr {
            for (lc, height) in heights.iter_mut().enumerate() {
                let v = values[(r0 + lr) * g + c0 + lc];
                *height = if v <= FREE_CELL_MAX { *height + 1 } else { 0 };
            }
            for l in 0..zc {
                let mut min_h = usize::MAX;
                for (r, &hgt) in heights.iter().enumerate().skip(l) {
                    min_h = min_h.min(hgt);
                    if min_h == 0 {
                        break;
                    }
                    let (rw, rh) = (r - l + 1, min_h);
                    let area = rw * rh;
                    let top = lr + 1 - rh;
                    let better = match best {
                        None => true,
                        Some((ba, bt, bl, _, _)) => {
                            area > ba || (area == ba && (top, l) < (bt, bl))
                        }
                    };
                    if better {
                        best = Some((area, top, l, rw, rh));
                    }
                }
            }
        }
        let Some((_, top, left, rw, rh)) = best else {
            continue;
        };
        let rect = NormBox {
            x: (c0 + left) as f32 / gf,
            y: (r0 + top) as f32 / gf,
            width: rw as f32 / gf,
            height: rh as f32 / gf,
        };
        let area = rect.width * rect.height;
        if area < MIN_REGION_AREA {
            continue;
        }
        let mut sum = 0u32;
        for r in 0..rh {
            for c in 0..rw {
                sum += values[(r0 + top + r) * g + c0 + left + c] as u32;
            }
        }
        let occupancy = sum as f32 / (rw * rh) as f32 / 15.0;
        out.push(SafeRegion {
            name,
            rect,
            occupancy,
            score: area * (1.0 - occupancy),
        });
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.name.cmp(&b.name)));
    out
}

/// Head box from the row-span silhouette (spec: "Head estimate").
fn head_estimate(
    spans: &[Option<(u32, u32)>],
    y0: u32,
    y1: u32,
    w: u32,
    h: u32,
) -> Option<NormBox> {
    let hs = (y1 + 1 - y0) as f32;
    let y0f = y0 as f32;
    let base = y0 as usize;
    // Raw span width per subject row (rows without subject count as 0), then
    // a 3-row moving average clipped to the subject rows.
    let n = (y1 + 1 - y0) as usize;
    let raw: Vec<f32> = (0..n)
        .map(|i| {
            spans
                .get(base + i)
                .copied()
                .flatten()
                .map_or(0.0, |(a, b)| (b - a) as f32)
        })
        .collect();
    let smooth: Vec<f32> = (0..n)
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(n - 1);
            let s: f32 = raw[lo..=hi].iter().sum();
            s / (hi - lo + 1) as f32
        })
        .collect();

    let lo = (y0f + 0.08 * hs).ceil() as usize;
    let hi = ((y0f + 0.45 * hs).floor() as usize).min(y1 as usize);
    let mut neck: Option<usize> = None;
    if lo <= hi {
        let mut min_w = f32::INFINITY;
        for y in lo..=hi {
            let wy = smooth[y - base];
            if wy < min_w {
                min_w = wy;
                neck = Some(y);
            }
        }
    }
    let accepted = neck.filter(|&yn| {
        let i = yn - base;
        let wn = smooth[i];
        let max_above = smooth[..=i].iter().cloned().fold(0.0f32, f32::max);
        let below_end = ((yn as f32 + 0.5 * hs).floor() as usize).min(y1 as usize);
        let shoulders = (yn + 1..=below_end).any(|y| smooth[y - base] >= 1.3 * wn);
        wn <= 0.8 * max_above && shoulders
    });
    let last_row = match accepted {
        Some(yn) => yn,
        None => ((y0f + 0.22 * hs).floor() as usize).min(y1 as usize),
    };

    let (mut min_x, mut max_x) = (u32::MAX, 0u32);
    for span in spans.iter().take(last_row + 1).skip(base).flatten() {
        min_x = min_x.min(span.0);
        max_x = max_x.max(span.1);
    }
    if min_x > max_x {
        return None;
    }
    Some(NormBox {
        x: min_x as f32 / w as f32,
        y: y0 as f32 / h as f32,
        width: (max_x + 1 - min_x) as f32 / w as f32,
        height: (last_row as u32 + 1 - y0) as f32 / h as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use resvg::tiny_skia::{Color, FillRule, Paint, PathBuilder, Rect, Transform};

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

    #[test]
    fn opaque_image_is_full() {
        let mut pm = canvas(40, 30);
        pm.fill(Color::from_rgba8(1, 2, 3, 255));
        let a = analyze(
            &pm,
            &AnalyzeOptions {
                person: true,
                ..Default::default()
            },
        );
        assert_eq!(a.coverage, 1.0);
        assert!(a.edges.top && a.edges.bottom && a.edges.left && a.edges.right);
        assert!(a.safe_regions.is_empty());
        assert!(a.head_estimate.is_none());
        assert!(a.occupancy.rows.iter().all(|r| r == &"f".repeat(16)));
    }

    #[test]
    fn empty_image_has_zero_coverage() {
        let a = analyze(&canvas(64, 64), &AnalyzeOptions::default());
        assert_eq!(a.coverage, 0.0);
        assert_eq!(a.subject_bounds.width, 1.0);
        assert!(!a.edges.top && !a.edges.left);
    }

    #[test]
    fn bounds_are_exact() {
        let mut pm = canvas(100, 200);
        fill_rect(&mut pm, 10.0, 20.0, 30.0, 50.0);
        let a = analyze(&pm, &AnalyzeOptions::default());
        assert_eq!(a.subject_bounds.x, 0.10);
        assert_eq!(a.subject_bounds.y, 0.10);
        assert_eq!(a.subject_bounds.width, 0.30);
        assert_eq!(a.subject_bounds.height, 0.25);
        assert!((a.coverage - 1500.0 / 20000.0).abs() < 1e-6);
    }

    #[test]
    fn tiny_image_grid_is_well_formed() {
        let mut pm = canvas(5, 3);
        fill_rect(&mut pm, 0.0, 0.0, 2.0, 3.0);
        let a = analyze(&pm, &AnalyzeOptions::default());
        assert_eq!(a.occupancy.rows.len(), 16);
        assert!(a.occupancy.rows.iter().all(|r| r.len() == 16));
    }
}
