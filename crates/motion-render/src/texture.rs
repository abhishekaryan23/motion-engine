//! Deterministic procedural materials (Flat, Paper, Grain, Halftone).
//!
//! Every pixel is a pure function of `(spec, width, height)`: all randomness comes from an
//! integer hash of `(seed, x, y, channel)`, never from time, the environment or a global RNG.

use motion_core::scene::{Color, Material, TextureSpec};
use resvg::tiny_skia::{Color as SkColor, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// Generate the texture for `spec` at `width x height` pixels.
/// Returns `None` only if the size is zero.
pub fn generate(spec: &TextureSpec, width: u32, height: u32) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(width, height)?;
    let intensity = clamp01(spec.intensity);
    match spec.material {
        Material::Flat => flat(&mut pixmap, spec.color),
        Material::Paper => paper(&mut pixmap, spec, intensity),
        Material::Grain => grain(&mut pixmap, spec, intensity),
        Material::Halftone => halftone(&mut pixmap, spec, intensity),
    }
    Some(pixmap)
}

// ---------------------------------------------------------------------------
// Hashing
// ---------------------------------------------------------------------------

/// Channel tags so independent features never share a random stream.
const CH_LATTICE_A: u64 = 1;
const CH_LATTICE_B: u64 = 2;
const CH_SPECKLE: u64 = 3;
const CH_FIBER: u64 = 4;
const CH_GRAIN: u64 = 5;
const CH_DOT: u64 = 6;

/// splitmix64 finalizer.
#[inline]
fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Hash of `(seed, a, b, channel)`.
#[inline]
fn hash(seed: u64, a: u64, b: u64, channel: u64) -> u64 {
    let mut h = mix64(seed.wrapping_add(0x9e37_79b9_7f4a_7c15));
    h = mix64(h ^ a.wrapping_mul(0xd1b5_4a32_d192_ed03));
    h = mix64(h ^ b.wrapping_mul(0x8cb9_2ba7_2f3d_8dd7));
    mix64(h ^ channel.wrapping_mul(0xa24b_aed4_963e_e407))
}

/// Uniform in `[0, 1)`.
#[inline]
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / 16_777_216.0
}

/// Uniform in `[-1, 1)`.
#[inline]
fn signed(h: u64) -> f32 {
    unit(h) * 2.0 - 1.0
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

#[inline]
fn to_u8(v: f32) -> u8 {
    if v.is_nan() {
        0
    } else {
        v.clamp(0.0, 255.0).round() as u8
    }
}

/// Premultiplied RGBA bytes for a straight-alpha color scaled by `alpha` (0..=255).
#[inline]
fn premul(r: u8, g: u8, b: u8, alpha: u8) -> [u8; 4] {
    let p = |c: u8| ((u32::from(c) * u32::from(alpha) + 127) / 255) as u8;
    [p(r), p(g), p(b), alpha]
}

// ---------------------------------------------------------------------------
// Flat
// ---------------------------------------------------------------------------

fn flat(pixmap: &mut Pixmap, color: Color) {
    let px = premul(color.r, color.g, color.b, color.a);
    for chunk in pixmap.data_mut().chunks_exact_mut(4) {
        chunk.copy_from_slice(&px);
    }
}

// ---------------------------------------------------------------------------
// Value noise
// ---------------------------------------------------------------------------

/// Smoothstep-interpolated lattice noise in `[-1, 1]`.
struct ValueNoise {
    inv_cell: f32,
    stride: usize,
    values: Vec<f32>,
}

impl ValueNoise {
    fn new(seed: u64, channel: u64, cell: f32, width: u32, height: u32) -> Self {
        let cell = cell.max(1.0);
        let gw = (width as f32 / cell).ceil() as usize + 2;
        let gh = (height as f32 / cell).ceil() as usize + 2;
        let mut values = Vec::with_capacity(gw * gh);
        for gy in 0..gh {
            for gx in 0..gw {
                values.push(signed(hash(seed, gx as u64, gy as u64, channel)));
            }
        }
        ValueNoise {
            inv_cell: 1.0 / cell,
            stride: gw,
            values,
        }
    }

    #[inline]
    fn sample(&self, x: f32, y: f32) -> f32 {
        let fx = x * self.inv_cell;
        let fy = y * self.inv_cell;
        let ix = fx.floor();
        let iy = fy.floor();
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        let tx = smooth(fx - ix);
        let ty = smooth(fy - iy);
        let base = iy as usize * self.stride + ix as usize;
        let v00 = self.values[base];
        let v10 = self.values[base + 1];
        let v01 = self.values[base + self.stride];
        let v11 = self.values[base + self.stride + 1];
        let top = v00 + (v10 - v00) * tx;
        let bottom = v01 + (v11 - v01) * tx;
        top + (bottom - top) * ty
    }
}

// ---------------------------------------------------------------------------
// Paper
// ---------------------------------------------------------------------------

fn paper(pixmap: &mut Pixmap, spec: &TextureSpec, intensity: f32) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let seed = spec.seed;
    let big = w.max(h) as f32;
    let coarse = ValueNoise::new(seed, CH_LATTICE_A, big / 5.0, w, h);
    let fine = ValueNoise::new(seed, CH_LATTICE_B, big / 20.0, w, h);

    let blotch_amp = intensity * 0.07 * 255.0;
    let speckle_amp = intensity * 0.03 * 255.0;
    let (r0, g0, b0) = (
        f32::from(spec.color.r),
        f32::from(spec.color.g),
        f32::from(spec.color.b),
    );

    let data = pixmap.data_mut();
    for y in 0..h {
        let row = y as usize * w as usize * 4;
        for x in 0..w {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let blotch = 0.65 * coarse.sample(fx, fy) + 0.35 * fine.sample(fx, fy);
            let speckle = signed(hash(seed, u64::from(x), u64::from(y), CH_SPECKLE));
            let delta = blotch * blotch_amp + speckle * speckle_amp;
            let i = row + x as usize * 4;
            data[i] = to_u8(r0 + delta);
            data[i + 1] = to_u8(g0 + delta);
            data[i + 2] = to_u8(b0 + delta);
            data[i + 3] = 255;
        }
    }

    draw_fibers(pixmap, spec, intensity);
}

/// Sparse, short, slightly curved dark fibers. Placement is a pure function of the seed.
fn draw_fibers(pixmap: &mut Pixmap, spec: &TextureSpec, intensity: f32) {
    let (w, h) = (pixmap.width() as f32, pixmap.height() as f32);
    // ~300 fibers at 1080x1920.
    let count = ((w * h) / 6_900.0 * (0.4 + 0.6 * intensity)).round() as u64;
    let stroke = Stroke {
        width: 1.0,
        line_cap: LineCap::Round,
        ..Stroke::default()
    };
    let ink = |c: u8| (f32::from(c) * 0.55).round() as u8;
    for n in 0..count {
        let rnd = |k: u64| unit(hash(spec.seed, n, k, CH_FIBER));
        let x0 = rnd(0) * w;
        let y0 = rnd(1) * h;
        let len = 6.0 + rnd(2) * 24.0;
        let angle = rnd(3) * std::f32::consts::TAU;
        let bend = (rnd(4) - 0.5) * len * 0.5;
        let alpha = 0.08 + rnd(5) * 0.07;
        let (dx, dy) = (angle.cos() * len, angle.sin() * len);
        let (mx, my) = (
            x0 + dx * 0.5 - dy / len * bend,
            y0 + dy * 0.5 + dx / len * bend,
        );

        let mut pb = PathBuilder::new();
        pb.move_to(x0, y0);
        pb.quad_to(mx, my, x0 + dx, y0 + dy);
        let Some(path) = pb.finish() else { continue };

        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(SkColor::from_rgba8(
            ink(spec.color.r),
            ink(spec.color.g),
            ink(spec.color.b),
            to_u8(alpha * 255.0),
        ));
        pixmap.stroke_path(&path, &paint, &stroke, Transform::identity(), None);
    }
}

// ---------------------------------------------------------------------------
// Grain
// ---------------------------------------------------------------------------

fn grain(pixmap: &mut Pixmap, spec: &TextureSpec, intensity: f32) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let cell = u64::from(spec.scale.max(1.0).round() as u32).max(1);
    let data = pixmap.data_mut();
    for y in 0..h {
        let row = y as usize * w as usize * 4;
        for x in 0..w {
            let n = signed(hash(
                spec.seed,
                u64::from(x) / cell,
                u64::from(y) / cell,
                CH_GRAIN,
            ));
            // mag^1.5 keeps most specks faint with the occasional stronger one.
            let mag = n.abs();
            let alpha = to_u8(mag * mag.sqrt() * intensity * 60.0);
            let v = if n > 0.0 { 255 } else { 0 };
            let i = row + x as usize * 4;
            data[i..i + 4].copy_from_slice(&premul(v, v, v, alpha));
        }
    }
}

// ---------------------------------------------------------------------------
// Halftone
// ---------------------------------------------------------------------------

fn halftone(pixmap: &mut Pixmap, spec: &TextureSpec, intensity: f32) {
    let (w, h) = (pixmap.width(), pixmap.height());
    let (wf, hf) = (w as f32, h as f32);
    let cell = spec.scale.max(2.0);
    let (sin, cos) = std::f32::consts::FRAC_PI_4.sin_cos();
    let (ox, oy) = (wf * 0.5, hf * 0.5);

    // Range of lattice indices whose centers may touch the canvas.
    let (mut imin, mut imax, mut jmin, mut jmax) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    for (cx, cy) in [(0.0, 0.0), (wf, 0.0), (0.0, hf), (wf, hf)] {
        let (px, py) = (cx - ox, cy - oy);
        let qx = (px * cos + py * sin) / cell;
        let qy = (-px * sin + py * cos) / cell;
        imin = imin.min(qx);
        imax = imax.max(qx);
        jmin = jmin.min(qy);
        jmax = jmax.max(qy);
    }
    let (imin, imax) = (imin.floor() as i64 - 1, imax.ceil() as i64 + 1);
    let (jmin, jmax) = (jmin.floor() as i64 - 1, jmax.ceil() as i64 + 1);

    let color = spec.color;
    let data = pixmap.data_mut();
    for j in jmin..=jmax {
        for i in imin..=imax {
            let (lx, ly) = (i as f32 * cell, j as f32 * cell);
            let cx = ox + lx * cos - ly * sin;
            let cy = oy + lx * sin + ly * cos;
            if cx < -cell || cy < -cell || cx > wf + cell || cy > hf + cell {
                continue;
            }
            // Diagonal fade: 1 at the top-left corner, 0 at the bottom-right.
            let t = clamp01((cx / wf + cy / hf) * 0.5);
            let jitter = 1.0 + 0.05 * signed(hash(spec.seed, i as u64, j as u64, CH_DOT));
            let radius = 0.5 * cell * (1.0 - t) * intensity * jitter;
            if radius < 0.15 {
                continue;
            }
            splat_dot(data, w, h, cx, cy, radius, color);
        }
    }
}

/// Anti-aliased disc by analytic edge coverage; overlapping dots keep the max coverage.
fn splat_dot(data: &mut [u8], w: u32, h: u32, cx: f32, cy: f32, radius: f32, color: Color) {
    let x0 = ((cx - radius - 1.0).floor().max(0.0)) as u32;
    let y0 = ((cy - radius - 1.0).floor().max(0.0)) as u32;
    let x1 = ((cx + radius + 1.0).ceil().min(w as f32)) as u32;
    let y1 = ((cy + radius + 1.0).ceil().min(h as f32)) as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let dist = (dx * dx + dy * dy).sqrt();
            let cov = clamp01(radius + 0.5 - dist);
            if cov <= 0.0 {
                continue;
            }
            let alpha = to_u8(cov * f32::from(color.a));
            let i = (y as usize * w as usize + x as usize) * 4;
            if alpha > data[i + 3] {
                data[i..i + 4].copy_from_slice(&premul(color.r, color.g, color.b, alpha));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn spec(material: Material, seed: u64) -> TextureSpec {
        TextureSpec {
            material,
            seed,
            color: match material {
                Material::Paper => Color::rgb(0xEF, 0xE6, 0xD6),
                Material::Halftone => Color::rgb(0x1A, 0x1A, 0x1A),
                _ => Color::rgb(0x33, 0x66, 0x99),
            },
            intensity: 0.5,
            scale: if material == Material::Grain {
                2.0
            } else {
                8.0
            },
            animated: false,
        }
    }

    const ALL: [Material; 4] = [
        Material::Flat,
        Material::Paper,
        Material::Grain,
        Material::Halftone,
    ];

    #[test]
    fn same_spec_is_byte_identical() {
        for m in ALL {
            let a = generate(&spec(m, 7), 200, 300).unwrap();
            let b = generate(&spec(m, 7), 200, 300).unwrap();
            assert_eq!(a.data(), b.data(), "{m:?}");
        }
    }

    #[test]
    fn different_seed_differs() {
        for m in [Material::Paper, Material::Grain, Material::Halftone] {
            let a = generate(&spec(m, 1), 200, 300).unwrap();
            let b = generate(&spec(m, 2), 200, 300).unwrap();
            assert_ne!(a.data(), b.data(), "{m:?}");
        }
    }

    #[test]
    fn flat_is_uniform() {
        let s = spec(Material::Flat, 0);
        let p = generate(&s, 64, 48).unwrap();
        for px in p.pixels() {
            assert_eq!(
                (px.red(), px.green(), px.blue(), px.alpha()),
                (0x33, 0x66, 0x99, 255)
            );
        }
    }

    #[test]
    fn paper_is_opaque_and_near_base_color() {
        let p = generate(&spec(Material::Paper, 3), 300, 400).unwrap();
        assert!(p.pixels().iter().all(|px| px.alpha() == 255));
        let n = p.pixels().len() as f32;
        let mean_r = p.pixels().iter().map(|px| f32::from(px.red())).sum::<f32>() / n;
        assert!((mean_r - 239.0).abs() < 15.0, "mean red {mean_r}");
    }

    #[test]
    fn grain_is_mostly_transparent() {
        let p = generate(&spec(Material::Grain, 5), 300, 300).unwrap();
        let n = p.pixels().len() as f32;
        let mean = p
            .pixels()
            .iter()
            .map(|px| f32::from(px.alpha()))
            .sum::<f32>()
            / n;
        assert!(mean > 0.0 && mean < 40.0, "mean alpha {mean}");
    }

    #[test]
    fn halftone_fades_toward_bottom_right() {
        let (w, h) = (400u32, 400u32);
        let p = generate(&spec(Material::Halftone, 9), w, h).unwrap();
        let region = |x0: u32, y0: u32| -> u64 {
            let mut sum = 0u64;
            for y in y0..y0 + 100 {
                for x in x0..x0 + 100 {
                    sum += u64::from(p.pixels()[(y * w + x) as usize].alpha());
                }
            }
            sum
        };
        let tl = region(0, 0);
        let br = region(w - 100, h - 100);
        assert!(tl > 0);
        assert!(tl > br * 2, "top-left {tl} vs bottom-right {br}");
    }

    #[test]
    fn zero_size_is_none() {
        for m in ALL {
            assert!(generate(&spec(m, 0), 0, 10).is_none());
            assert!(generate(&spec(m, 0), 10, 0).is_none());
        }
    }

    #[test]
    fn degenerate_parameters_do_not_panic() {
        for m in ALL {
            for (intensity, scale) in [(f32::NAN, f32::NAN), (-3.0, 0.0), (9.0, 1e9)] {
                let mut s = spec(m, 1);
                s.intensity = intensity;
                s.scale = scale;
                assert!(generate(&s, 37, 23).is_some());
            }
        }
        assert!(generate(&spec(Material::Paper, 1), 1, 1).is_some());
    }

    /// Human inspection: `cargo test -p motion-render texture_samples -- --ignored`.
    #[test]
    #[ignore]
    fn save_texture_samples() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../output/texture_samples");
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (540, 960);
        let paper_spec = spec(Material::Paper, 42);
        let paper_px = generate(&paper_spec, w, h).unwrap();
        for (name, mut s) in [
            ("flat", spec(Material::Flat, 42)),
            ("paper", paper_spec),
            ("grain", spec(Material::Grain, 42)),
            ("halftone", spec(Material::Halftone, 42)),
        ] {
            if name == "grain" {
                s.intensity = 1.0;
            }
            let p = generate(&s, w, h).unwrap();
            p.save_png(dir.join(format!("{name}.png"))).unwrap();
            if name == "grain" || name == "halftone" {
                // Overlays are easier to judge on paper.
                let mut base = paper_px.clone();
                base.draw_pixmap(
                    0,
                    0,
                    p.as_ref(),
                    &resvg::tiny_skia::PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
                base.save_png(dir.join(format!("{name}_on_paper.png")))
                    .unwrap();
            }
        }
    }
}
