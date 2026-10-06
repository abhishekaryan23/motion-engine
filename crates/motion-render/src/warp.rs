//! Projective (homography) warp of a premultiplied RGBA pixmap onto another.
//!
//! The source (a layer rendered in its own box space) is resampled onto the
//! destination through the *inverse* homography with bilinear filtering in
//! premultiplied space, restricted to the bounding box of the projected quad.
//! Strongly minified quads are supersampled (up to 3x3) to avoid shimmer.
//! Source texels outside the pixmap are transparent, so quad edges are
//! antialiased by the bilinear footprint.

use rayon::prelude::*;
use resvg::tiny_skia::Pixmap;

/// Row-major 3x3 matrix.
pub type Mat3 = [f64; 9];

/// Homogeneous `w` below which a point is treated as at/behind the camera.
const W_EPS: f64 = 1e-6;
/// Supersampling cap (samples per axis).
const MAX_SS: usize = 3;

pub fn mat3_from_f32(m: &[f32; 9]) -> Mat3 {
    let mut o = [0.0; 9];
    for (d, s) in o.iter_mut().zip(m) {
        *d = *s as f64;
    }
    o
}

/// `a * b` (apply `b` first).
pub fn mat3_mul(a: &Mat3, b: &Mat3) -> Mat3 {
    let mut o = [0.0; 9];
    for r in 0..3 {
        for c in 0..3 {
            o[r * 3 + c] = (0..3).map(|k| a[r * 3 + k] * b[k * 3 + c]).sum();
        }
    }
    o
}

/// Inverse by cofactors; `None` for (near-)singular or non-finite matrices.
pub fn mat3_inverse(m: &Mat3) -> Option<Mat3> {
    if m.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let [a, b, c, d, e, f, g, h, i] = *m;
    let (co0, co1, co2) = (e * i - f * h, f * g - d * i, d * h - e * g);
    let det = a * co0 + b * co1 + c * co2;
    let scale = m.iter().fold(0.0f64, |s, v| s.max(v.abs())).powi(3);
    if !det.is_finite() || det.abs() <= 1e-12 * scale.max(1e-300) {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        co0 * inv,
        (c * h - b * i) * inv,
        (b * f - c * e) * inv,
        co1 * inv,
        (a * i - c * g) * inv,
        (c * d - a * f) * inv,
        co2 * inv,
        (b * g - a * h) * inv,
        (a * e - b * d) * inv,
    ])
}

/// Row-major 3x3 of the affine `[a b c d e f]` (x' = a x + c y + e, y' = b x + d y + f).
pub fn mat3_affine(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Mat3 {
    [
        a as f64, c as f64, e as f64, b as f64, d as f64, f as f64, 0.0, 0.0, 1.0,
    ]
}

/// Image of `(x, y)` under `m`, or `None` at/behind the horizon (`w <= 0`).
fn project(m: &Mat3, x: f64, y: f64) -> Option<(f64, f64)> {
    let w = m[6] * x + m[7] * y + m[8];
    if w <= W_EPS {
        return None;
    }
    Some((
        (m[0] * x + m[1] * y + m[2]) / w,
        (m[3] * x + m[4] * y + m[5]) / w,
    ))
}

/// Premultiplied bilinear sample at `(u, v)` in source pixel coordinates
/// (texel centres at +0.5); texels outside the pixmap are transparent.
#[inline]
fn sample(src: &[u8], sw: i32, sh: i32, u: f32, v: f32) -> [f32; 4] {
    let fx = u - 0.5;
    let fy = v - 0.5;
    let x0f = fx.floor();
    let y0f = fy.floor();
    let (wx, wy) = (fx - x0f, fy - y0f);
    let (x0, y0) = (x0f as i32, y0f as i32);
    let mut out = [0.0f32; 4];
    for (dy, wyy) in [(0, 1.0 - wy), (1, wy)] {
        let y = y0 + dy;
        if y < 0 || y >= sh || wyy == 0.0 {
            continue;
        }
        for (dx, wxx) in [(0, 1.0 - wx), (1, wx)] {
            let x = x0 + dx;
            if x < 0 || x >= sw || wxx == 0.0 {
                continue;
            }
            let wgt = wxx * wyy;
            let i = ((y * sw + x) * 4) as usize;
            out[0] += src[i] as f32 * wgt;
            out[1] += src[i + 1] as f32 * wgt;
            out[2] += src[i + 2] as f32 * wgt;
            out[3] += src[i + 3] as f32 * wgt;
        }
    }
    out
}

/// Source pixels per destination pixel at the worst of the quad's corners and
/// centre (1 = no minification).
fn minification(h: &Mat3, sw: f64, sh: f64) -> f64 {
    let mut worst = 1.0f64;
    for (x, y) in [
        (0.0, 0.0),
        (sw, 0.0),
        (0.0, sh),
        (sw, sh),
        (sw * 0.5, sh * 0.5),
    ] {
        let (Some(p), Some(px), Some(py)) = (
            project(h, x, y),
            project(h, x + 1.0, y),
            project(h, x, y + 1.0),
        ) else {
            continue;
        };
        // Destination pixels travelled per source pixel along each axis.
        let ex = ((px.0 - p.0).powi(2) + (px.1 - p.1).powi(2)).sqrt();
        let ey = ((py.0 - p.0).powi(2) + (py.1 - p.1).powi(2)).sqrt();
        let m = ex.min(ey);
        if m > 1e-6 {
            worst = worst.max(1.0 / m);
        }
    }
    worst
}

/// Composite `src` onto `dst` (source-over) through the homography
/// `src_to_dst` (source pixel space -> destination pixel space).
pub fn warp_over(dst: &mut Pixmap, src: &Pixmap, src_to_dst: &Mat3) {
    let (dw, dh) = (dst.width() as usize, dst.height() as usize);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    if dw == 0 || dh == 0 || sw == 0 || sh == 0 {
        return;
    }
    // A homography is defined up to scale: orient it so the source centre has
    // positive `w` (in front of the camera).
    let mut oriented = *src_to_dst;
    if oriented[6] * (sw as f64 * 0.5) + oriented[7] * (sh as f64 * 0.5) + oriented[8] < 0.0 {
        for v in &mut oriented {
            *v = -*v;
        }
    }
    let src_to_dst = &oriented;
    let Some(inv) = mat3_inverse(src_to_dst) else {
        return;
    };

    // Destination bounding box of the projected quad.
    let (fw, fh) = (sw as f64, sh as f64);
    let corners: Vec<Option<(f64, f64)>> = [(0.0, 0.0), (fw, 0.0), (fw, fh), (0.0, fh)]
        .iter()
        .map(|&(x, y)| project(src_to_dst, x, y))
        .collect();
    let (bx0, by0, bx1, by1) = if corners.iter().all(Option::is_some) {
        let pts: Vec<(f64, f64)> = corners.into_iter().flatten().collect();
        let min = |f: fn(&(f64, f64)) -> f64| pts.iter().map(f).fold(f64::INFINITY, f64::min);
        let max = |f: fn(&(f64, f64)) -> f64| pts.iter().map(f).fold(f64::NEG_INFINITY, f64::max);
        (
            min(|p| p.0).floor().max(0.0),
            min(|p| p.1).floor().max(0.0),
            max(|p| p.0).ceil().min(dw as f64),
            max(|p| p.1).ceil().min(dh as f64),
        )
    } else {
        // Part of the quad is behind the camera: scan the whole target (the
        // per-pixel horizon test below keeps it correct).
        (0.0, 0.0, dw as f64, dh as f64)
    };
    if !(bx1 > bx0 && by1 > by0) {
        return;
    }
    let (x0, y0, x1, y1) = (bx0 as usize, by0 as usize, bx1 as usize, by1 as usize);

    let ss = (minification(src_to_dst, fw, fh).ceil() as usize).clamp(1, MAX_SS);
    let samples = (ss * ss) as f32;
    let offsets: Vec<f64> = (0..ss).map(|i| (i as f64 + 0.5) / ss as f64).collect();
    let src_data = src.data();
    let stride = dw * 4;
    let rows = &mut dst.data_mut()[y0 * stride..y1 * stride];
    rows.par_chunks_mut(stride)
        .enumerate()
        .for_each(|(ri, row)| {
            let y = (y0 + ri) as f64;
            for x in x0..x1 {
                let mut acc = [0.0f32; 4];
                let mut hit = false;
                for &oy in &offsets {
                    for &ox in &offsets {
                        let (px, py) = (x as f64 + ox, y + oy);
                        let w = inv[6] * px + inv[7] * py + inv[8];
                        if w.abs() <= W_EPS {
                            continue;
                        }
                        let u = (inv[0] * px + inv[1] * py + inv[2]) / w;
                        let v = (inv[3] * px + inv[4] * py + inv[5]) / w;
                        if u < -1.0 || v < -1.0 || u > fw + 1.0 || v > fh + 1.0 {
                            continue;
                        }
                        // In front of the camera? (forward w of the source point)
                        if src_to_dst[6] * u + src_to_dst[7] * v + src_to_dst[8] <= W_EPS {
                            continue;
                        }
                        let s = sample(src_data, sw, sh, u as f32, v as f32);
                        for c in 0..4 {
                            acc[c] += s[c];
                        }
                        hit = true;
                    }
                }
                if !hit {
                    continue;
                }
                let a = (acc[3] / samples + 0.5).min(255.0) as u32;
                if a == 0 {
                    continue;
                }
                let i = x * 4;
                let inv_a = 255 - a;
                for c in 0..3 {
                    let s = ((acc[c] / samples + 0.5) as u32).min(a);
                    row[i + c] = (s + div255(row[i + c] as u32 * inv_a)).min(255) as u8;
                }
                row[i + 3] = (a + div255(row[i + 3] as u32 * inv_a)).min(255) as u8;
            }
        });
}

#[inline]
fn div255(x: u32) -> u32 {
    let t = x + 128;
    (t + (t >> 8)) >> 8
}
