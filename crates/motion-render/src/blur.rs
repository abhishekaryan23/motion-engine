//! Gaussian blur of premultiplied RGBA pixmaps (three box passes).
//!
//! Used for per-layer depth-of-field blur (`ResolvedLayer::blur`). The blur is
//! confined to the bounding box of the non-transparent pixels plus the kernel
//! support, so blurring one small layer on a full-HD offscreen costs about as
//! much as the layer, not the canvas. Pixels outside the pixmap count as
//! transparent, exactly as if the whole canvas had been blurred.

use rayon::prelude::*;
use resvg::tiny_skia::Pixmap;

/// Blurs with a sigma below this are skipped (invisible).
pub const MIN_SIGMA: f32 = 0.3;
/// Largest sigma applied (larger values are clamped).
pub const MAX_SIGMA: f32 = 200.0;

/// Radii of the three boxes whose cascade approximates a Gaussian of `sigma`.
fn box_radii(sigma: f32) -> [usize; 3] {
    let n = 3.0f32;
    let ideal = (12.0 * sigma * sigma / n + 1.0).sqrt();
    let mut wl = ideal.floor() as i32;
    if wl % 2 == 0 {
        wl -= 1;
    }
    let wl = wl.max(1);
    let wlf = wl as f32;
    let m = ((12.0 * sigma * sigma - n * wlf * wlf - 4.0 * n * wlf - 3.0 * n) / (-4.0 * wlf - 4.0))
        .round() as i32;
    let size = |i: i32| if i < m { wl } else { wl + 2 };
    [
        ((size(0) - 1) / 2) as usize,
        ((size(1) - 1) / 2) as usize,
        ((size(2) - 1) / 2) as usize,
    ]
}

/// Fixed-point reciprocal of the window size `2r + 1` (Q24).
fn reciprocal(r: usize) -> u64 {
    let n = (2 * r + 1) as u64;
    ((1u64 << 24) + n / 2) / n
}

fn scale(sum: u32, inv: u64) -> u8 {
    ((sum as u64 * inv + (1 << 23)) >> 24).min(255) as u8
}

/// Horizontal box pass over `h` rows of `w` RGBA pixels (zero outside).
fn box_h(src: &[u8], dst: &mut [u8], w: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let inv = reciprocal(r);
    let stride = w * 4;
    dst.par_chunks_mut(stride)
        .zip(src.par_chunks(stride))
        .for_each(|(out, row)| {
            let mut sum = [0u32; 4];
            for px in row.chunks_exact(4).take(r.min(w - 1) + 1) {
                for c in 0..4 {
                    sum[c] += px[c] as u32;
                }
            }
            for x in 0..w {
                for c in 0..4 {
                    out[x * 4 + c] = scale(sum[c], inv);
                }
                let add = x + r + 1;
                if add < w {
                    for c in 0..4 {
                        sum[c] += row[add * 4 + c] as u32;
                    }
                }
                if x >= r {
                    let sub = x - r;
                    for c in 0..4 {
                        sum[c] -= row[sub * 4 + c] as u32;
                    }
                }
            }
        });
}

/// Vertical box pass (row accumulators: sequential, cache friendly).
fn box_v(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize) {
    if r == 0 {
        dst.copy_from_slice(src);
        return;
    }
    let inv = reciprocal(r);
    let stride = w * 4;
    let mut acc = vec![0u32; stride];
    for y in 0..=r.min(h - 1) {
        for (a, v) in acc.iter_mut().zip(&src[y * stride..(y + 1) * stride]) {
            *a += *v as u32;
        }
    }
    for y in 0..h {
        for (o, a) in dst[y * stride..(y + 1) * stride].iter_mut().zip(&acc) {
            *o = scale(*a, inv);
        }
        let add = y + r + 1;
        if add < h {
            for (a, v) in acc.iter_mut().zip(&src[add * stride..(add + 1) * stride]) {
                *a += *v as u32;
            }
        }
        if y >= r {
            let sub = y - r;
            for (a, v) in acc.iter_mut().zip(&src[sub * stride..(sub + 1) * stride]) {
                *a -= *v as u32;
            }
        }
    }
}

/// Bounding box `[x0, x1) x [y0, y1)` of pixels with non-zero alpha.
fn alpha_bounds(data: &[u8], w: usize, h: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut x1, mut y0, mut y1) = (w, 0usize, h, 0usize);
    for y in 0..h {
        let row = &data[y * w * 4..(y + 1) * w * 4];
        let mut first = None;
        let mut last = 0;
        for (x, px) in row.chunks_exact(4).enumerate() {
            if px[3] != 0 {
                if first.is_none() {
                    first = Some(x);
                }
                last = x;
            }
        }
        if let Some(f) = first {
            x0 = x0.min(f);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > x0 && y1 > y0).then_some((x0, x1, y0, y1))
}

/// Gaussian blur of the whole pixmap by standard deviation `sigma` px.
/// Non-finite or tiny sigmas are a no-op.
pub fn gaussian_blur(pm: &mut Pixmap, sigma: f32) {
    if !sigma.is_finite() || sigma < MIN_SIGMA {
        return;
    }
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if w == 0 || h == 0 {
        return;
    }
    let radii = box_radii(sigma.min(MAX_SIGMA));
    let margin: usize = radii.iter().sum();
    let Some((bx0, bx1, by0, by1)) = alpha_bounds(pm.data(), w, h) else {
        return;
    };
    let (x0, x1) = (bx0.saturating_sub(margin), (bx1 + margin).min(w));
    let (y0, y1) = (by0.saturating_sub(margin), (by1 + margin).min(h));
    let (rw, rh) = (x1 - x0, y1 - y0);

    // Gather the region.
    let mut a = vec![0u8; rw * rh * 4];
    {
        let data = pm.data();
        for y in 0..rh {
            let s = ((y0 + y) * w + x0) * 4;
            a[y * rw * 4..(y + 1) * rw * 4].copy_from_slice(&data[s..s + rw * 4]);
        }
    }
    let mut b = vec![0u8; a.len()];
    for r in radii {
        box_h(&a, &mut b, rw, r);
        std::mem::swap(&mut a, &mut b);
    }
    for r in radii {
        box_v(&a, &mut b, rw, rh, r);
        std::mem::swap(&mut a, &mut b);
    }
    // Scatter back.
    let data = pm.data_mut();
    for y in 0..rh {
        let s = ((y0 + y) * w + x0) * 4;
        data[s..s + rw * 4].copy_from_slice(&a[y * rw * 4..(y + 1) * rw * 4]);
    }
}
