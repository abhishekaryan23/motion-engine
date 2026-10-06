//! Full-frame post effects (0.15) on a finished, premultiplied RGBA pixmap.
//!
//! [`apply_post`] runs the timeline's [`ResolvedPost`] list in order. Every
//! effect is a pure function of the pixels, its parameters, its strength and
//! (for noise) `seed` plus the frame time, so the result is deterministic and a
//! GPU backend can match it. Strength 0 (or any non-positive/NaN strength) is
//! an exact no-op and skips the effect entirely; strength is clamped to 1.
//! Row passes run in parallel (rayon) but every output pixel depends only on
//! the input snapshot, so the thread count never changes the pixels.
//!
//! Alpha: the frame is normally opaque. Colour channels are always kept
//! `<= alpha` so the pixmap stays a valid premultiplied image. See
//! `docs/POST_EFFECTS.md` for the parameters and costs.

use motion_core::noise::{hash, hash01};
use motion_core::scene::PostKind;
use motion_core::timeline::ResolvedPost;
use rayon::prelude::*;
use resvg::tiny_skia::Pixmap;

/// Samples along each ray of [`PostKind::Rays`].
const RAY_SAMPLES: usize = 24;
/// Deepest bloom pyramid.
const BLOOM_MAX_LEVELS: usize = 7;

/// Apply `posts` to `pixmap` in order.
pub fn apply_post(pixmap: &mut Pixmap, posts: &[ResolvedPost<'_>]) {
    for p in posts {
        let s = p.strength;
        if s.is_nan() || s <= 0.0 {
            continue;
        }
        let s = s.min(1.0);
        match p.kind {
            PostKind::Bloom { threshold, radius } => bloom(pixmap, *threshold, *radius, s),
            PostKind::ChromaticAberration { shift, angle_deg } => {
                chromatic_aberration(pixmap, *shift, *angle_deg, s)
            }
            PostKind::Glitch {
                bands,
                max_shift,
                seed,
            } => glitch(pixmap, *bands, *max_shift, *seed, p.time, s),
            PostKind::Rays { center, length } => rays(pixmap, *center, *length, s),
            PostKind::DirectionalBlur { angle_deg, length } => {
                directional_blur(pixmap, *angle_deg, *length, s)
            }
            PostKind::Grain { amount, seed } => grain(pixmap, *amount, *seed, p.time, s),
            PostKind::Vignette { amount } => vignette(pixmap, *amount, s),
        }
    }
}

fn dims(pm: &Pixmap) -> (usize, usize) {
    (pm.width() as usize, pm.height() as usize)
}

#[inline]
fn to_u8(v: f32) -> u8 {
    (v + 0.5).clamp(0.0, 255.0) as u8
}

/// Time bucket of a per-frame noise effect running at `rate` Hz.
fn time_bucket(time: f64, rate: f64) -> u32 {
    (time * rate).floor() as i64 as u32
}

// ---------------------------------------------------------------------------
// Chromatic aberration
// ---------------------------------------------------------------------------

/// Red is sampled at `p + d` and blue at `p - d`, `d = shift * strength` px
/// along `angle_deg` (bilinear, edge clamped). Red content therefore appears
/// displaced by `-d`, blue by `+d`. Green and alpha are untouched.
pub fn chromatic_aberration(pm: &mut Pixmap, shift: f32, angle_deg: f32, strength: f32) {
    let d = shift * strength;
    let (w, h) = dims(pm);
    if w == 0 || h == 0 || !d.is_finite() || !angle_deg.is_finite() || d == 0.0 {
        return;
    }
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let snap = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
    let (dx, dy) = (snap(d * cos), snap(d * sin));
    let src = pm.data().to_vec();

    // The fractional offset is the same for every pixel: precompute columns.
    struct Plan {
        cx0: Vec<u32>,
        cx1: Vec<u32>,
        iy: i32,
        fx: f32,
        fy: f32,
    }
    let plan = |ox: f32, oy: f32| {
        let ix = ox.floor();
        let iy = oy.floor();
        let clampx = |x: i64| x.clamp(0, w as i64 - 1) as u32;
        Plan {
            cx0: (0..w).map(|x| clampx(x as i64 + ix as i64)).collect(),
            cx1: (0..w).map(|x| clampx(x as i64 + ix as i64 + 1)).collect(),
            iy: iy as i32,
            fx: ox - ix,
            fy: oy - iy,
        }
    };
    let red = plan(dx, dy);
    let blue = plan(-dx, -dy);

    let sample = |p: &Plan, x: usize, y0: usize, y1: usize, ch: usize| -> f32 {
        let at = |yy: usize, xx: u32| src[(yy * w + xx as usize) * 4 + ch] as f32;
        let (x0, x1) = (p.cx0[x], p.cx1[x]);
        let top = at(y0, x0) * (1.0 - p.fx) + at(y0, x1) * p.fx;
        let bot = at(y1, x0) * (1.0 - p.fx) + at(y1, x1) * p.fx;
        top * (1.0 - p.fy) + bot * p.fy
    };
    let rows_of = |p: &Plan, y: usize| {
        let clampy = |v: i64| v.clamp(0, h as i64 - 1) as usize;
        (
            clampy(y as i64 + p.iy as i64),
            clampy(y as i64 + p.iy as i64 + 1),
        )
    };

    pm.data_mut()
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let (r0, r1) = rows_of(&red, y);
            let (b0, b1) = rows_of(&blue, y);
            for x in 0..w {
                let a = row[x * 4 + 3];
                row[x * 4] = to_u8(sample(&red, x, r0, r1, 0)).min(a);
                row[x * 4 + 2] = to_u8(sample(&blue, x, b0, b1, 2)).min(a);
            }
        });
}

// ---------------------------------------------------------------------------
// Bloom
// ---------------------------------------------------------------------------

/// A float RGB image (a pyramid level).
struct Level {
    w: usize,
    h: usize,
    px: Vec<[f32; 3]>,
}

impl Level {
    /// Bilinear sample at `(u, v)` in corner coordinates (texel `i` spans
    /// `[i, i + 1]`), edge clamped.
    fn bilinear(&self, u: f32, v: f32) -> [f32; 3] {
        let fx = u - 0.5;
        let fy = v - 0.5;
        let (x0f, y0f) = (fx.floor(), fy.floor());
        let (wx, wy) = (fx - x0f, fy - y0f);
        let cx = |x: f32| (x as i64).clamp(0, self.w as i64 - 1) as usize;
        let cy = |y: f32| (y as i64).clamp(0, self.h as i64 - 1) as usize;
        let (x0, x1, y0, y1) = (cx(x0f), cx(x0f + 1.0), cy(y0f), cy(y0f + 1.0));
        let (a, b) = (&self.px[y0 * self.w + x0], &self.px[y0 * self.w + x1]);
        let (c, d) = (&self.px[y1 * self.w + x0], &self.px[y1 * self.w + x1]);
        let mut o = [0.0; 3];
        for k in 0..3 {
            let top = a[k] * (1.0 - wx) + b[k] * wx;
            let bot = c[k] * (1.0 - wx) + d[k] * wx;
            o[k] = top * (1.0 - wy) + bot * wy;
        }
        o
    }
}

/// Dual-filter downsample of a `sw x sh` image given by `fetch` (4x4 taps:
/// centre 2x2 weight 5, ring weight 1, over 32).
fn downsample<F: Fn(i64, i64) -> [f32; 3] + Sync>(sw: usize, sh: usize, fetch: F) -> Level {
    let (dw, dh) = (sw.div_ceil(2).max(1), sh.div_ceil(2).max(1));
    let mut px = vec![[0.0f32; 3]; dw * dh];
    px.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        for (x, out) in row.iter_mut().enumerate() {
            let (cx, cy) = (2 * x as i64, 2 * y as i64);
            let mut acc = [0.0f32; 3];
            for j in -1..=2i64 {
                for i in -1..=2i64 {
                    let wgt = if (0..=1).contains(&i) && (0..=1).contains(&j) {
                        5.0
                    } else {
                        1.0
                    };
                    let t = fetch(cx + i, cy + j);
                    for k in 0..3 {
                        acc[k] += t[k] * wgt;
                    }
                }
            }
            for k in 0..3 {
                out[k] = acc[k] / 32.0;
            }
        }
    });
    Level { w: dw, h: dh, px }
}

/// Dual-filter upsample of `coarse` to `dw x dh` (8 bilinear taps / 12).
fn upsample(coarse: &Level, dw: usize, dh: usize) -> Level {
    let mut px = vec![[0.0f32; 3]; dw * dh];
    let (kx, ky) = (coarse.w as f32 / dw as f32, coarse.h as f32 / dh as f32);
    const TAPS: [(f32, f32, f32); 8] = [
        (-1.0, 0.0, 1.0),
        (1.0, 0.0, 1.0),
        (0.0, -1.0, 1.0),
        (0.0, 1.0, 1.0),
        (-0.5, -0.5, 2.0),
        (0.5, -0.5, 2.0),
        (-0.5, 0.5, 2.0),
        (0.5, 0.5, 2.0),
    ];
    px.par_chunks_mut(dw).enumerate().for_each(|(y, row)| {
        let v = (y as f32 + 0.5) * ky;
        for (x, out) in row.iter_mut().enumerate() {
            let u = (x as f32 + 0.5) * kx;
            let mut acc = [0.0f32; 3];
            for (ox, oy, wgt) in TAPS {
                let t = coarse.bilinear(u + ox, v + oy);
                for k in 0..3 {
                    acc[k] += t[k] * wgt;
                }
            }
            for k in 0..3 {
                out[k] = acc[k] / 12.0;
            }
        }
    });
    Level { w: dw, h: dh, px }
}

/// Bright-pass weight of a luma value: 0 at or below `threshold`, rising
/// linearly to 1 at luma 1.
#[inline]
fn bright_weight(luma: f32, threshold: f32) -> f32 {
    ((luma - threshold) / (1.0 - threshold).max(0.02)).clamp(0.0, 1.0)
}

/// Bright pass above `threshold` (luma 0..=1), dual-filter blurred to about
/// `radius` px and added back (clamped), scaled by `strength`.
pub fn bloom(pm: &mut Pixmap, threshold: f32, radius: f32, strength: f32) {
    let (w, h) = dims(pm);
    if w == 0 || h == 0 || !threshold.is_finite() || !radius.is_finite() || radius <= 0.0 {
        return;
    }
    if threshold >= 1.0 || strength <= 0.0 {
        return;
    }
    let threshold = threshold.max(0.0);
    // Pyramid depth: the glow's standard deviation is about `radius / 2`
    // (calibrated against an impulse; see docs/POST_EFFECTS.md).
    let levels_wanted =
        ((radius.max(1.0).log2().round() as i64) - 1).clamp(1, BLOOM_MAX_LEVELS as i64) as usize;

    let mut pyramid: Vec<Level> = Vec::with_capacity(levels_wanted);
    {
        let data = pm.data();
        let fetch = |x: i64, y: i64| -> [f32; 3] {
            let xi = x.clamp(0, w as i64 - 1) as usize;
            let yi = y.clamp(0, h as i64 - 1) as usize;
            let i = (yi * w + xi) * 4;
            let (r, g, b) = (
                data[i] as f32 / 255.0,
                data[i + 1] as f32 / 255.0,
                data[i + 2] as f32 / 255.0,
            );
            let k = bright_weight(0.2126 * r + 0.7152 * g + 0.0722 * b, threshold);
            [r * k, g * k, b * k]
        };
        pyramid.push(downsample(w, h, fetch));
    }
    while pyramid.len() < levels_wanted {
        let last = &pyramid[pyramid.len() - 1];
        if last.w <= 1 && last.h <= 1 {
            break;
        }
        let next = downsample(last.w, last.h, |x, y| {
            let xi = x.clamp(0, last.w as i64 - 1) as usize;
            let yi = y.clamp(0, last.h as i64 - 1) as usize;
            last.px[yi * last.w + xi]
        });
        pyramid.push(next);
    }
    // Merge from the coarsest level up so every level ends with equal weight
    // (an equal-energy halo per octave): with `m` levels already merged,
    // merged' = (d_k + m * up(merged)) / (m + 1).
    let mut merged = pyramid.pop().unwrap_or(Level {
        w: 1,
        h: 1,
        px: vec![[0.0; 3]],
    });
    let mut count = 1.0f32;
    while let Some(fine) = pyramid.pop() {
        let mut up = upsample(&merged, fine.w, fine.h);
        for (u, f) in up.px.iter_mut().zip(&fine.px) {
            for k in 0..3 {
                u[k] = (f[k] + count * u[k]) / (count + 1.0);
            }
        }
        merged = up;
        count += 1.0;
    }

    // Add back at full resolution (bilinear from the half-res glow).
    let (kx, ky) = (merged.w as f32 / w as f32, merged.h as f32 / h as f32);
    let gain = 255.0 * strength;
    pm.data_mut()
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let v = (y as f32 + 0.5) * ky;
            for x in 0..w {
                let g = merged.bilinear((x as f32 + 0.5) * kx, v);
                let a = row[x * 4 + 3];
                for k in 0..3 {
                    let add = (g[k] * gain + 0.5) as u32;
                    row[x * 4 + k] = (row[x * 4 + k] as u32 + add).min(a as u32) as u8;
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Glitch
// ---------------------------------------------------------------------------

/// `bands` noise bands (frame bucket `floor(time * 12)`) displaced
/// horizontally by up to `max_shift * strength` px with wrap-around, each with
/// a small RGB split.
pub fn glitch(pm: &mut Pixmap, bands: u32, max_shift: f32, seed: u32, time: f64, strength: f32) {
    let (w, h) = dims(pm);
    let reach = max_shift * strength;
    if w == 0 || h == 0 || bands == 0 || !reach.is_finite() || reach <= 0.0 {
        return;
    }
    let key = seed ^ time_bucket(time, 12.0);
    let data = pm.data_mut();
    let mut tmp = vec![0u8; w * 4];
    for i in 0..bands.min(256) {
        let u = |k: u32| hash01(key, i * 4 + k) as f32;
        let y0 = ((u(0) * h as f32) as usize).min(h - 1);
        let band_h = (((0.01 + 0.07 * u(1)) * h as f32) as usize).max(1);
        let y1 = (y0 + band_h).min(h);
        let mag = (0.25 + 0.75 * u(2)) * reach;
        let off = if hash(key, i * 4 + 3) & 1 == 0 {
            mag
        } else {
            -mag
        }
        .round() as i64;
        if off == 0 {
            continue;
        }
        let split = ((off.abs() as f32 * 0.12).round() as i64).max(1);
        let wrap = |x: i64| x.rem_euclid(w as i64) as usize;
        for y in y0..y1 {
            let row = &mut data[y * w * 4..(y + 1) * w * 4];
            tmp.copy_from_slice(row);
            for x in 0..w {
                let base = x as i64 - off;
                let c = wrap(base) * 4;
                let r = wrap(base - split) * 4;
                let b = wrap(base + split) * 4;
                let a = tmp[c + 3];
                row[x * 4] = tmp[r].min(a);
                row[x * 4 + 1] = tmp[c + 1];
                row[x * 4 + 2] = tmp[b + 2].min(a);
                row[x * 4 + 3] = a;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Rays
// ---------------------------------------------------------------------------

/// Radial blur toward `center` (canvas px): [`RAY_SAMPLES`] samples along the
/// segment from each pixel over `length * strength` of the way to the centre,
/// averaged, then screen-blended over the frame with weight `strength`.
pub fn rays(pm: &mut Pixmap, center: [f32; 2], length: f32, strength: f32) {
    let (w, h) = dims(pm);
    let reach = (length * strength).clamp(0.0, 1.0);
    if w == 0 || h == 0 || !(center[0].is_finite() && center[1].is_finite()) || reach <= 0.0 {
        return;
    }
    let src = pm.data().to_vec();
    let step = reach / (RAY_SAMPLES - 1) as f32;
    let inv_n = 1.0 / RAY_SAMPLES as f32;
    let (maxx, maxy) = (w as i32 - 1, h as i32 - 1);
    pm.data_mut()
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let py = y as f32 + 0.5;
            let vy = (center[1] - py) * step;
            for x in 0..w {
                let px = x as f32 + 0.5;
                let vx = (center[0] - px) * step;
                let (mut qx, mut qy) = (px, py);
                let mut acc = [0u32; 3];
                for _ in 0..RAY_SAMPLES {
                    let ix = (qx as i32).clamp(0, maxx) as usize;
                    let iy = (qy as i32).clamp(0, maxy) as usize;
                    let i = (iy * w + ix) * 4;
                    acc[0] += src[i] as u32;
                    acc[1] += src[i + 1] as u32;
                    acc[2] += src[i + 2] as u32;
                    qx += vx;
                    qy += vy;
                }
                let a = row[x * 4 + 3];
                for k in 0..3 {
                    let d = row[x * 4 + k] as f32;
                    let r = acc[k] as f32 * inv_n * strength;
                    row[x * 4 + k] = to_u8(d + r * (255.0 - d) / 255.0).min(a);
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Directional blur
// ---------------------------------------------------------------------------

/// `acc[x] += wgt * row[clamp(x + ox)]` for RGBA rows of `w` pixels.
fn axpy_row(acc: &mut [f32], row: &[u8], w: usize, ox: i32, wgt: f32) {
    let wi = w as i32;
    let lo = (-ox).clamp(0, wi);
    let hi = (wi - ox).clamp(0, wi);
    if lo > 0 {
        let e = [
            row[0] as f32 * wgt,
            row[1] as f32 * wgt,
            row[2] as f32 * wgt,
            row[3] as f32 * wgt,
        ];
        for px in acc[..lo as usize * 4].chunks_exact_mut(4) {
            for k in 0..4 {
                px[k] += e[k];
            }
        }
    }
    if hi > lo {
        let src = &row[((lo + ox) as usize) * 4..((hi + ox) as usize) * 4];
        let dst = &mut acc[lo as usize * 4..hi as usize * 4];
        for (a, s) in dst.iter_mut().zip(src) {
            *a += wgt * *s as f32;
        }
    }
    if hi < wi {
        let l = (w - 1) * 4;
        let e = [
            row[l] as f32 * wgt,
            row[l + 1] as f32 * wgt,
            row[l + 2] as f32 * wgt,
            row[l + 3] as f32 * wgt,
        ];
        for px in acc[hi as usize * 4..].chunks_exact_mut(4) {
            for k in 0..4 {
                px[k] += e[k];
            }
        }
    }
}

/// Box blur along `angle_deg` over `length * strength` px (centred; at least 8
/// bilinear samples, edge clamped). Every sample is a constant 2-D shift of the
/// image, so rows accumulate shifted rows with fixed bilinear weights.
pub fn directional_blur(pm: &mut Pixmap, angle_deg: f32, length: f32, strength: f32) {
    let (w, h) = dims(pm);
    let len = length * strength;
    if w == 0 || h == 0 || !len.is_finite() || !angle_deg.is_finite() || len <= 0.0 {
        return;
    }
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    let snap = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
    let (cos, sin) = (snap(cos), snap(sin));
    let x_major = cos.abs() >= sin.abs();
    let (a, b) = if x_major { (cos, sin) } else { (sin, cos) };
    // Extent of the box along the dominant axis; samples are spaced at most
    // one pixel apart there (so the minor-axis shift is sampled bilinearly).
    let extent = len * a.abs();
    if extent < 1e-3 {
        return;
    }
    let slope = b / a;
    let n = (extent.ceil() as usize + 1).clamp(8, 4096);
    // Trapezoid weights: end samples count half (exact box integral).
    let mut taps: Vec<(i32, i32, f32)> = Vec::with_capacity(n * 2);
    let mut total = 0.0f32;
    for k in 0..n {
        let o = -extent * 0.5 + extent * k as f32 / (n - 1) as f32;
        let wk = if k == 0 || k == n - 1 { 0.5 } else { 1.0 };
        total += wk;
        let (sx, sy) = if x_major {
            (o, o * slope)
        } else {
            (o * slope, o)
        };
        let (ix, iy) = (sx.floor(), sy.floor());
        let (fx, fy) = (sx - ix, sy - iy);
        for (dx, wx) in [(0, 1.0 - fx), (1, fx)] {
            for (dy, wy) in [(0, 1.0 - fy), (1, fy)] {
                let wgt = wk * wx * wy;
                if wgt > 0.0 {
                    taps.push((ix as i32 + dx, iy as i32 + dy, wgt));
                }
            }
        }
    }
    let norm = 1.0 / total;
    let src = pm.data().to_vec();
    let stride = w * 4;
    pm.data_mut()
        .par_chunks_mut(stride)
        .enumerate()
        .for_each(|(y, row)| {
            let mut acc = vec![0.0f32; stride];
            for &(ox, oy, wgt) in &taps {
                let ry = (y as i64 + oy as i64).clamp(0, h as i64 - 1) as usize;
                axpy_row(&mut acc, &src[ry * stride..(ry + 1) * stride], w, ox, wgt);
            }
            for px in 0..w {
                let a = to_u8(acc[px * 4 + 3] * norm);
                row[px * 4] = to_u8(acc[px * 4] * norm).min(a);
                row[px * 4 + 1] = to_u8(acc[px * 4 + 1] * norm).min(a);
                row[px * 4 + 2] = to_u8(acc[px * 4 + 2] * norm).min(a);
                row[px * 4 + 3] = a;
            }
        });
}

// ---------------------------------------------------------------------------
// Grain
// ---------------------------------------------------------------------------

/// Film grain: per-pixel uniform noise from `hash(seed ^ floor(time * 24),
/// pixel index)`, shifting luma by up to `amount * strength * 0.25` of full
/// scale. Alpha is kept.
pub fn grain(pm: &mut Pixmap, amount: f32, seed: u32, time: f64, strength: f32) {
    let (w, h) = dims(pm);
    let amp = amount * strength * 0.25 * 255.0;
    if w == 0 || h == 0 || !amp.is_finite() || amp <= 0.0 {
        return;
    }
    let key = seed ^ time_bucket(time, 24.0);
    pm.data_mut()
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            for x in 0..w {
                let idx = (y * w + x) as u32;
                let u = (hash(key, idx) >> 8) as f32 * (1.0 / 16_777_216.0);
                let a = row[x * 4 + 3];
                let d = (u * 2.0 - 1.0) * amp * (a as f32 / 255.0);
                for k in 0..3 {
                    row[x * 4 + k] = to_u8(row[x * 4 + k] as f32 + d).min(a);
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Vignette
// ---------------------------------------------------------------------------

/// Darkens by `amount * strength * smoothstep(0.45, 1.0, r)`, `r` the distance
/// from the frame centre normalised so the corners are 1 (RGB only).
pub fn vignette(pm: &mut Pixmap, amount: f32, strength: f32) {
    let (w, h) = dims(pm);
    let k = (amount * strength).clamp(0.0, 1.0);
    if w == 0 || h == 0 || !amount.is_finite() || k <= 0.0 {
        return;
    }
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let inv_max = 1.0 / (cx * cx + cy * cy).sqrt();
    pm.data_mut()
        .par_chunks_mut(w * 4)
        .enumerate()
        .for_each(|(y, row)| {
            let dy = y as f32 + 0.5 - cy;
            for x in 0..w {
                let dx = x as f32 + 0.5 - cx;
                let r = (dx * dx + dy * dy).sqrt() * inv_max;
                let t = ((r - 0.45) / 0.55).clamp(0.0, 1.0);
                let f = 1.0 - k * t * t * (3.0 - 2.0 * t);
                if f < 1.0 {
                    for c in 0..3 {
                        row[x * 4 + c] = to_u8(row[x * 4 + c] as f32 * f);
                    }
                }
            }
        });
}
