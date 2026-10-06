//! Image treatments (0.5): deterministic pixel operations executed once per
//! image layer when the renderer prepares assets. Spec: docs/IMAGE_TREATMENTS.md.
//!
//! Colour operations run on un-premultiplied RGB (alpha is never touched),
//! then the silhouette operations (paper edge, contact shadow) grow the output
//! by `pad` pixels on every side. All operations are pure functions of the
//! input pixels and the treatment (grain is hash noise of `(seed, x, y)`), so
//! results are identical across runs and platforms.
//!
//! Behavior on opaque images: edge and shadow act on the alpha silhouette, so
//! an opaque image gets a border / drop shadow around its rectangle. The
//! compiler never asks for that (`treatment_for` sets neither for opaque
//! images) but `apply` handles it consistently.

use motion_core::scene::{Color, ContactShadow, ImageTreatment, PaperEdge, Sticker};
use resvg::tiny_skia::Pixmap;

/// Upper bound for the padding, so a malformed treatment cannot request an
/// enormous canvas.
const MAX_PAD: u32 = 2048;
/// "Far away" for the distance transform (squared pixels).
const FAR: f32 = 1.0e20;

/// (0.10 Q) Opacity of a sticker's soft shadow (10 %, as an 8-bit alpha).
const STICKER_SHADOW_ALPHA: u8 = 26;
/// Sticker shadow offset (downwards) and blur, in multiples of the outline width.
const STICKER_SHADOW_OFFSET: f32 = 0.5;
const STICKER_SHADOW_BLUR: f32 = 1.5;

/// A sticker as the two silhouette operations that draw it: the alpha grown by
/// `width_px` and filled with the sticker colour (the paper-edge operation),
/// under a soft 10 % black shadow. `None` for a degenerate sticker (no
/// positive finite width), which is then ignored.
fn sticker_ops(s: &Sticker) -> Option<(PaperEdge, ContactShadow)> {
    if !(s.width_px.is_finite() && s.width_px > 0.0) {
        return None;
    }
    Some((
        PaperEdge {
            color: s.color,
            width: s.width_px,
        },
        ContactShadow {
            color: Color::rgb(0, 0, 0).with_alpha(STICKER_SHADOW_ALPHA),
            offset: [0.0, STICKER_SHADOW_OFFSET * s.width_px],
            blur: STICKER_SHADOW_BLUR * s.width_px,
        },
    ))
}

/// Apply `t` to `img`. `px_per_unit` = image pixels per layer pixel at the
/// layer's base size (converts edge width / shadow offset / blur). Returns the
/// treated pixmap and the padding (image pixels) added on every side for the
/// paper edge and shadow (0 when neither is set).
///
/// (0.10 Q) A `sticker` replaces the paper edge and the contact shadow: it is
/// the alpha dilated by `width_px`, filled with the sticker colour, over a soft
/// 10 % shadow. Without a sticker the behaviour is unchanged.
pub fn apply(img: &Pixmap, t: &ImageTreatment, px_per_unit: f32) -> (Pixmap, u32) {
    let ppu = if px_per_unit.is_finite() && px_per_unit > 0.0 {
        px_per_unit
    } else {
        1.0
    };

    let mut base = img.clone();
    apply_colour_ops(&mut base, t);

    let sticker_t;
    let t = match t.sticker.as_ref().and_then(sticker_ops) {
        Some((edge, shadow)) => {
            sticker_t = ImageTreatment {
                edge: Some(edge),
                shadow: Some(shadow),
                sticker: None,
                ..t.clone()
            };
            &sticker_t
        }
        None => t,
    };

    if t.edge.is_none() && t.shadow.is_none() {
        return (base, 0);
    }

    let edge_w = t.edge.map_or(0.0, |e| e.width.max(0.0)) * ppu;
    let shadow_reach = t.shadow.map_or(0.0, |s| {
        let off = (s.offset[0] * s.offset[0] + s.offset[1] * s.offset[1]).sqrt();
        off + s.blur.max(0.0)
    }) * ppu;
    let reach = edge_w.max(shadow_reach);
    let pad = if reach.is_finite() {
        (reach.ceil().min(MAX_PAD as f32) as u32 + 2).min(MAX_PAD)
    } else {
        2
    };

    match silhouette_ops(&base, t, ppu, edge_w, pad) {
        Some(out) => (out, pad),
        None => (base, 0),
    }
}

// ---------------------------------------------------------------------------
// Colour operations
// ---------------------------------------------------------------------------

fn luma(c: [f32; 3]) -> f32 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn rgb01(c: Color) -> [f32; 3] {
    [
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
    ]
}

/// splitmix64 finalizer.
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Deterministic noise in `[0, 1)` from `(seed, x, y)`.
fn hash_noise(seed: u64, x: u32, y: u32) -> f32 {
    let packed = (u64::from(x) << 32) | u64::from(y);
    let h = splitmix64(splitmix64(seed) ^ packed);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn apply_colour_ops(img: &mut Pixmap, t: &ImageTreatment) {
    let duotone = t.duotone.filter(|d| d.amount != 0.0);
    let tint = t.tint.filter(|k| k.amount != 0.0);
    let active = t.desaturate != 0.0
        || t.brightness != 0.0
        || t.contrast != 1.0
        || duotone.is_some()
        || tint.is_some()
        || t.grain != 0.0;
    if !active {
        return;
    }
    let (w, h) = (img.width() as usize, img.height() as usize);
    let duo = duotone.map(|d| (rgb01(d.shadow), rgb01(d.highlight), d.amount));
    let tin = tint.map(|k| (rgb01(k.color), k.amount));
    let data = img.data_mut();
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 4;
            let a = data[i + 3];
            if a == 0 {
                continue;
            }
            let af = f32::from(a) / 255.0;
            let mut c = [
                (f32::from(data[i]) / af / 255.0).min(1.0),
                (f32::from(data[i + 1]) / af / 255.0).min(1.0),
                (f32::from(data[i + 2]) / af / 255.0).min(1.0),
            ];
            // 1. desaturate
            if t.desaturate != 0.0 {
                let l = luma(c);
                c = mix3(c, [l, l, l], t.desaturate);
            }
            // 2. brightness / contrast
            if t.brightness != 0.0 || t.contrast != 1.0 {
                for v in &mut c {
                    *v = ((*v - 0.5) * t.contrast + 0.5 + t.brightness).clamp(0.0, 1.0);
                }
            }
            // 3. duotone
            if let Some((shadow, highlight, amount)) = duo {
                let target = mix3(shadow, highlight, luma(c).clamp(0.0, 1.0));
                c = mix3(c, target, amount);
            }
            // 4. tint
            if let Some((color, amount)) = tin {
                c = mix3(c, color, amount);
            }
            // 5. grain (same noise on R, G, B)
            if t.grain != 0.0 {
                let n = hash_noise(t.seed, x as u32, y as u32);
                let d = t.grain * (n - 0.5) * 0.5;
                for v in &mut c {
                    *v += d;
                }
            }
            for k in 0..3 {
                let v = c[k].clamp(0.0, 1.0) * af * 255.0;
                data[i + k] = (v + 0.5).floor().min(f32::from(a)) as u8;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Silhouette operations
// ---------------------------------------------------------------------------

/// Squared Euclidean distance transform, 1D (Felzenszwalb & Huttenlocher).
fn dt1d(f: &[f32], d: &mut [f32], v: &mut [usize], z: &mut [f32]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0usize;
    v[0] = 0;
    z[0] = -FAR;
    z[1] = FAR;
    for q in 1..n {
        let qf = q as f32;
        loop {
            let p = v[k];
            let pf = p as f32;
            let s = ((f[q] + qf * qf) - (f[p] + pf * pf)) / (2.0 * qf - 2.0 * pf);
            if s <= z[k] && k > 0 {
                k -= 1;
            } else {
                k += 1;
                v[k] = q;
                z[k] = s;
                z[k + 1] = FAR;
                break;
            }
        }
    }
    k = 0;
    for (q, out) in d.iter_mut().enumerate() {
        while z[k + 1] < q as f32 {
            k += 1;
        }
        let dq = q as f32 - v[k] as f32;
        *out = dq * dq + f[v[k]];
    }
}

/// Grow `alpha` (0..=1, `w x h`) by a disc of `radius` pixels. Opaque-ish
/// pixels (alpha >= 0.5) are the features; coverage falls off over one pixel
/// at the disc boundary so the grown edge is anti-aliased.
fn dilate(alpha: &[f32], w: usize, h: usize, radius: f32) -> Vec<f32> {
    let mut grid: Vec<f32> = alpha
        .iter()
        .map(|&a| if a >= 0.5 { 0.0 } else { FAR })
        .collect();
    let m = w.max(h);
    let (mut col_in, mut col_out) = (vec![0.0f32; m], vec![0.0f32; m]);
    let (mut v, mut z) = (vec![0usize; m], vec![0.0f32; m + 1]);
    // Columns.
    for x in 0..w {
        for y in 0..h {
            col_in[y] = grid[y * w + x];
        }
        dt1d(&col_in[..h], &mut col_out[..h], &mut v, &mut z);
        for y in 0..h {
            grid[y * w + x] = col_out[y];
        }
    }
    // Rows.
    for y in 0..h {
        let row = &mut grid[y * w..(y + 1) * w];
        col_in[..w].copy_from_slice(row);
        dt1d(&col_in[..w], &mut col_out[..w], &mut v, &mut z);
        row.copy_from_slice(&col_out[..w]);
    }
    alpha
        .iter()
        .zip(&grid)
        .map(|(&a, &d2)| {
            let cov = (radius + 0.5 - d2.sqrt()).clamp(0.0, 1.0);
            a.max(cov)
        })
        .collect()
}

/// One box-blur pass (radius `r`, zero outside) along rows then columns.
fn box_blur(buf: &mut [f32], tmp: &mut [f32], w: usize, h: usize, r: usize) {
    if r == 0 || w == 0 || h == 0 {
        return;
    }
    let inv = 1.0 / (2 * r + 1) as f32;
    // Horizontal: buf -> tmp.
    for y in 0..h {
        let src = &buf[y * w..(y + 1) * w];
        let dst = &mut tmp[y * w..(y + 1) * w];
        let mut sum: f32 = src[..(r + 1).min(w)].iter().sum();
        for x in 0..w {
            dst[x] = sum * inv;
            if x + r + 1 < w {
                sum += src[x + r + 1];
            }
            if x >= r {
                sum -= src[x - r];
            }
        }
    }
    // Vertical: tmp -> buf, sliding whole rows.
    let mut sum = vec![0.0f32; w];
    for y in 0..(r + 1).min(h) {
        for x in 0..w {
            sum[x] += tmp[y * w + x];
        }
    }
    for y in 0..h {
        for x in 0..w {
            buf[y * w + x] = sum[x].max(0.0) * inv;
        }
        if y + r + 1 < h {
            for x in 0..w {
                sum[x] += tmp[(y + r + 1) * w + x];
            }
        }
        if y >= r {
            for x in 0..w {
                sum[x] -= tmp[(y - r) * w + x];
            }
        }
    }
}

fn silhouette_ops(
    base: &Pixmap,
    t: &ImageTreatment,
    ppu: f32,
    edge_w: f32,
    pad: u32,
) -> Option<Pixmap> {
    let (iw, ih) = (base.width() as usize, base.height() as usize);
    let p = pad as usize;
    let (w, h) = (iw + 2 * p, ih + 2 * p);
    let mut out = Pixmap::new(w as u32, h as u32)?;

    // Alpha of the image placed in the padded canvas.
    let mut alpha = vec![0.0f32; w * h];
    let src = base.data();
    for y in 0..ih {
        for x in 0..iw {
            alpha[(y + p) * w + x + p] = f32::from(src[(y * iw + x) * 4 + 3]) / 255.0;
        }
    }

    // Edge-dilated silhouette (also the shadow caster).
    let silhouette = match t.edge {
        Some(_) if edge_w > 0.0 => dilate(&alpha, w, h, edge_w),
        _ => alpha.clone(),
    };

    // Under layers as premultiplied f32 RGBA, painted bottom-up.
    let mut under = vec![[0.0f32; 4]; w * h];

    if let Some(s) = t.shadow {
        let dx = (s.offset[0] * ppu).round() as i64;
        let dy = (s.offset[1] * ppu).round() as i64;
        let mut buf = vec![0.0f32; w * h];
        for y in 0..h as i64 {
            let sy = y - dy;
            if sy < 0 || sy >= h as i64 {
                continue;
            }
            for x in 0..w as i64 {
                let sx = x - dx;
                if sx >= 0 && sx < w as i64 {
                    buf[y as usize * w + x as usize] = silhouette[sy as usize * w + sx as usize];
                }
            }
        }
        let r = (s.blur.max(0.0) * ppu / 2.0).round() as usize;
        if r > 0 {
            let mut tmp = vec![0.0f32; w * h];
            for _ in 0..3 {
                box_blur(&mut buf, &mut tmp, w, h, r);
            }
        }
        let c = rgb01(s.color);
        let ca = f32::from(s.color.a) / 255.0;
        for (u, &b) in under.iter_mut().zip(&buf) {
            let a = (b * ca).clamp(0.0, 1.0);
            *u = [c[0] * a, c[1] * a, c[2] * a, a];
        }
    }

    if let Some(e) = t.edge {
        let c = rgb01(e.color);
        let ca = f32::from(e.color.a) / 255.0;
        for (u, &s) in under.iter_mut().zip(&silhouette) {
            let a = (s * ca).clamp(0.0, 1.0);
            let k = 1.0 - a;
            *u = [
                c[0] * a + u[0] * k,
                c[1] * a + u[1] * k,
                c[2] * a + u[2] * k,
                a + u[3] * k,
            ];
        }
    }

    // The image over everything.
    let dst = out.data_mut();
    for y in 0..h {
        for x in 0..w {
            let u = under[y * w + x];
            let mut px = [0.0f32; 3];
            let a;
            let inside = x >= p && x < p + iw && y >= p && y < p + ih;
            let o = (y * w + x) * 4;
            if inside {
                let s = ((y - p) * iw + (x - p)) * 4;
                let ia = src[s + 3];
                if ia == 255 {
                    dst[o..o + 4].copy_from_slice(&src[s..s + 4]);
                    continue;
                }
                let k = 1.0 - f32::from(ia) / 255.0;
                for c in 0..3 {
                    px[c] = f32::from(src[s + c]) + u[c] * 255.0 * k;
                }
                a = f32::from(ia) + u[3] * 255.0 * k;
            } else {
                for c in 0..3 {
                    px[c] = u[c] * 255.0;
                }
                a = u[3] * 255.0;
            }
            let a8 = (a + 0.5).floor().clamp(0.0, 255.0) as u8;
            dst[o + 3] = a8;
            for c in 0..3 {
                dst[o + c] = (px[c] + 0.5).floor().clamp(0.0, f32::from(a8)) as u8;
            }
        }
    }
    Some(out)
}
