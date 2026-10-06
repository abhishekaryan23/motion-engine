//! Host-side planning of a post-effect list: pure functions of the canvas size
//! and the [`ResolvedPost`] list, no GPU objects.
//!
//! Every effect here mirrors the parameter handling of its CPU twin in
//! `postfx.rs` (same early-outs, same f32 operation order for derived values)
//! and emits compute passes whose uniform words the WGSL shaders read. Strength
//! 0 (or non-positive / NaN) skips the effect before anything is planned, so a
//! list of skipped effects plans zero passes and the pixmap is never touched.

use motion_core::scene::PostKind;
use motion_core::timeline::ResolvedPost;

/// Samples along each ray of `PostKind::Rays` (matches `postfx::RAY_SAMPLES`).
const RAY_SAMPLES: usize = 24;
/// Deepest bloom pyramid (matches `postfx::BLOOM_MAX_LEVELS`).
const BLOOM_MAX_LEVELS: usize = 7;
/// Workgroup edge of the 2-D kernels (the shaders use `@workgroup_size(16, 16)`).
const TILE: u32 = 16;
/// Invocations per workgroup of the row-parallel glitch kernel.
const ROW_GROUP: u32 = 64;
/// Largest glitch displacement the shader's i32 arithmetic holds (px).
const GLITCH_MAX_REACH: f32 = 1.0e9;

/// The compute pipelines (one per kernel; compiled lazily on first use).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Pipe {
    Chroma,
    GlitchCopy,
    GlitchRows,
    Rays,
    DBlur,
    Grain,
    Vignette,
    BloomBright,
    BloomDown,
    BloomMerge,
    BloomFinal,
}

pub(crate) const PIPE_COUNT: usize = 11;

/// Which buffer a binding points at.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Buf {
    /// One of the two ping-pong frame buffers (packed RGBA8).
    Ping(usize),
    /// Bloom pyramid level (vec4 f32 per texel).
    Level(usize),
    /// The dir-blur tap list.
    Taps,
    /// Placeholder for an unused binding.
    Dummy,
}

/// Uniform words per pass (padded to 64 bytes).
pub(crate) const PARAM_WORDS: usize = 16;

/// One compute dispatch.
#[derive(Clone, Debug)]
pub(crate) struct Pass {
    pub pipe: Pipe,
    pub src: Buf,
    pub dst: Buf,
    pub aux: Buf,
    pub params: [u32; PARAM_WORDS],
    pub groups: [u32; 2],
}

/// One bilinear tap of a directional blur (the layout the shader reads).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct GpuTap {
    pub ox: i32,
    pub oy: i32,
    pub wgt: f32,
}

/// Everything the executor needs for one frame.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub passes: Vec<Pass>,
    pub taps: Vec<GpuTap>,
    /// Bloom pyramid levels needed (0 = no bloom).
    pub levels: usize,
    /// Ping-pong index that holds the result after the last pass.
    pub result: usize,
}

/// Uniform word builder (`u32` / `i32` / `f32` bit patterns, in struct order).
struct Words {
    w: [u32; PARAM_WORDS],
    n: usize,
}

impl Words {
    fn new() -> Self {
        Words {
            w: [0; PARAM_WORDS],
            n: 0,
        }
    }
    fn push(mut self, bits: u32) -> Self {
        if self.n < PARAM_WORDS {
            self.w[self.n] = bits;
            self.n += 1;
        }
        self
    }
    fn u(self, v: u32) -> Self {
        self.push(v)
    }
    fn i(self, v: i32) -> Self {
        self.push(v as u32)
    }
    fn f(self, v: f32) -> Self {
        self.push(v.to_bits())
    }
}

/// Time bucket of a per-frame noise effect running at `rate` Hz (as postfx).
fn time_bucket(time: f64, rate: f64) -> u32 {
    (time * rate).floor() as i64 as u32
}

/// Pyramid level sizes for a `w x h` canvas with up to `wanted` levels
/// (`postfx::bloom` stops early once a level is 1 x 1).
pub(crate) fn level_dims(w: u32, h: u32, wanted: usize) -> Vec<(u32, u32)> {
    let mut dims = vec![(w.div_ceil(2).max(1), h.div_ceil(2).max(1))];
    while dims.len() < wanted {
        let (lw, lh) = dims[dims.len() - 1];
        if lw <= 1 && lh <= 1 {
            break;
        }
        dims.push((lw.div_ceil(2).max(1), lh.div_ceil(2).max(1)));
    }
    dims
}

struct Planner {
    w: u32,
    h: u32,
    /// Ping-pong index of the current frame.
    cur: usize,
    plan: Plan,
}

impl Planner {
    fn tiles(w: u32, h: u32) -> [u32; 2] {
        [w.div_ceil(TILE), h.div_ceil(TILE)]
    }

    /// A full-frame pass `Ping(cur) -> Ping(1 - cur)`; the buffers swap roles.
    fn frame_pass(&mut self, pipe: Pipe, words: Words, aux: Buf) {
        self.plan.passes.push(Pass {
            pipe,
            src: Buf::Ping(self.cur),
            dst: Buf::Ping(1 - self.cur),
            aux,
            params: words.w,
            groups: Self::tiles(self.w, self.h),
        });
        self.cur = 1 - self.cur;
    }

    fn side_pass(&mut self, pipe: Pipe, src: Buf, dst: Buf, words: Words, out: (u32, u32)) {
        self.plan.passes.push(Pass {
            pipe,
            src,
            dst,
            aux: Buf::Dummy,
            params: words.w,
            groups: Self::tiles(out.0, out.1),
        });
    }

    fn chromatic(&mut self, shift: f32, angle_deg: f32, strength: f32) {
        let d = shift * strength;
        if !d.is_finite() || !angle_deg.is_finite() || d == 0.0 {
            return;
        }
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let snap = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
        let (dx, dy) = (snap(d * cos), snap(d * sin));
        // Offsets past the canvas all clamp to an edge, so bounding them
        // keeps the shader's i32 arithmetic safe without changing the result.
        let lim = i64::from(self.w.max(self.h)) + 2;
        let clamp = |v: f32| (v as i64).clamp(-lim, lim) as i32;
        let part = |ox: f32, oy: f32| {
            let (ix, iy) = (ox.floor(), oy.floor());
            (clamp(ix), clamp(iy), ox - ix, oy - iy)
        };
        let (rix, riy, rfx, rfy) = part(dx, dy);
        let (bix, biy, bfx, bfy) = part(-dx, -dy);
        let words = Words::new()
            .u(self.w)
            .u(self.h)
            .i(rix)
            .i(riy)
            .f(rfx)
            .f(rfy)
            .i(bix)
            .i(biy)
            .f(bfx)
            .f(bfy);
        self.frame_pass(Pipe::Chroma, words, Buf::Dummy);
    }

    fn bloom(&mut self, threshold: f32, radius: f32, strength: f32) {
        if !threshold.is_finite() || !radius.is_finite() || radius <= 0.0 {
            return;
        }
        if threshold >= 1.0 || strength <= 0.0 {
            return;
        }
        let threshold = threshold.max(0.0);
        let wanted = ((radius.max(1.0).log2().round() as i64) - 1).clamp(1, BLOOM_MAX_LEVELS as i64)
            as usize;
        let dims = level_dims(self.w, self.h, wanted);
        let m = dims.len();
        self.plan.levels = self.plan.levels.max(m);

        let den = (1.0 - threshold).max(0.02);
        let words = Words::new()
            .u(self.w)
            .u(self.h)
            .u(dims[0].0)
            .u(dims[0].1)
            .f(threshold)
            .f(den);
        let src = Buf::Ping(self.cur);
        self.side_pass(Pipe::BloomBright, src, Buf::Level(0), words, dims[0]);
        for k in 1..m {
            let (sw, sh) = dims[k - 1];
            let (dw, dh) = dims[k];
            let words = Words::new().u(sw).u(sh).u(dw).u(dh);
            self.side_pass(
                Pipe::BloomDown,
                Buf::Level(k - 1),
                Buf::Level(k),
                words,
                (dw, dh),
            );
        }
        // Merge from the coarsest level up: with `count` levels already merged,
        // fine' = (fine + count * up(merged)) / (count + 1), in place.
        let mut count = 1.0f32;
        for k in (0..m.saturating_sub(1)).rev() {
            let (cw, ch) = dims[k + 1];
            let (dw, dh) = dims[k];
            let words = Words::new()
                .u(cw)
                .u(ch)
                .u(dw)
                .u(dh)
                .f(cw as f32 / dw as f32)
                .f(ch as f32 / dh as f32)
                .f(count);
            self.side_pass(
                Pipe::BloomMerge,
                Buf::Level(k + 1),
                Buf::Level(k),
                words,
                (dw, dh),
            );
            count += 1.0;
        }
        let (mw, mh) = dims[0];
        let words = Words::new()
            .u(self.w)
            .u(self.h)
            .u(mw)
            .u(mh)
            .f(mw as f32 / self.w as f32)
            .f(mh as f32 / self.h as f32)
            .f(255.0 * strength);
        self.frame_pass(Pipe::BloomFinal, words, Buf::Level(0));
    }

    fn glitch(&mut self, bands: u32, max_shift: f32, seed: u32, time: f64, strength: f32) {
        let reach = max_shift * strength;
        if bands == 0 || !reach.is_finite() || reach <= 0.0 {
            return;
        }
        let reach = reach.min(GLITCH_MAX_REACH);
        let key = seed ^ time_bucket(time, 12.0);
        let params = Words::new()
            .u(self.w)
            .u(self.h)
            .u(bands.min(256))
            .u(key)
            .f(reach)
            .w;
        let (src, dst) = (Buf::Ping(self.cur), Buf::Ping(1 - self.cur));
        // Copy the frame, then tear it in place (the source is the row scratch).
        for (pipe, groups) in [
            (Pipe::GlitchCopy, Self::tiles(self.w, self.h)),
            (Pipe::GlitchRows, [self.h.div_ceil(ROW_GROUP), 1]),
        ] {
            self.plan.passes.push(Pass {
                pipe,
                src,
                dst,
                aux: Buf::Dummy,
                params,
                groups,
            });
        }
        self.cur = 1 - self.cur;
    }

    fn rays(&mut self, center: [f32; 2], length: f32, strength: f32) {
        let reach = (length * strength).clamp(0.0, 1.0);
        // NaN reach is skipped here (the CPU would blend texel (0, 0)); the
        // docs say non-finite parameters skip the effect.
        if !(center[0].is_finite() && center[1].is_finite()) || reach.is_nan() || reach <= 0.0 {
            return;
        }
        let step = reach / (RAY_SAMPLES - 1) as f32;
        let inv_n = 1.0 / RAY_SAMPLES as f32;
        let words = Words::new()
            .u(self.w)
            .u(self.h)
            .f(center[0])
            .f(center[1])
            .f(step)
            .f(inv_n)
            .f(strength)
            .u(RAY_SAMPLES as u32)
            .u(0);
        self.frame_pass(Pipe::Rays, words, Buf::Dummy);
    }

    fn directional_blur(&mut self, angle_deg: f32, length: f32, strength: f32) {
        let len = length * strength;
        if !len.is_finite() || !angle_deg.is_finite() || len <= 0.0 {
            return;
        }
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let snap = |v: f32| if v.abs() < 1e-6 { 0.0 } else { v };
        let (cos, sin) = (snap(cos), snap(sin));
        let x_major = cos.abs() >= sin.abs();
        let (a, b) = if x_major { (cos, sin) } else { (sin, cos) };
        // Same tap list as `postfx::directional_blur`.
        let extent = len * a.abs();
        if extent < 1e-3 {
            return;
        }
        let slope = b / a;
        let n = (extent.ceil() as usize + 1).clamp(8, 4096);
        let lim = i64::from(self.w.max(self.h)) + 2;
        let clamp = |v: f32| (v as i64).clamp(-lim, lim) as i32;
        let first = self.plan.taps.len();
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
                        self.plan.taps.push(GpuTap {
                            ox: clamp(ix) + dx,
                            oy: clamp(iy) + dy,
                            wgt,
                        });
                    }
                }
            }
        }
        let count = self.plan.taps.len() - first;
        let norm = 1.0 / total;
        let words = Words::new()
            .u(self.w)
            .u(self.h)
            .u(first as u32)
            .u(count as u32)
            .f(norm);
        self.frame_pass(Pipe::DBlur, words, Buf::Taps);
    }

    fn grain(&mut self, amount: f32, seed: u32, time: f64, strength: f32) {
        let amp = amount * strength * 0.25 * 255.0;
        if !amp.is_finite() || amp <= 0.0 {
            return;
        }
        let key = seed ^ time_bucket(time, 24.0);
        let words = Words::new().u(self.w).u(self.h).u(key).f(amp);
        self.frame_pass(Pipe::Grain, words, Buf::Dummy);
    }

    fn vignette(&mut self, amount: f32, strength: f32) {
        let k = (amount * strength).clamp(0.0, 1.0);
        if !amount.is_finite() || k <= 0.0 {
            return;
        }
        let (cx, cy) = (self.w as f32 * 0.5, self.h as f32 * 0.5);
        let inv_max = 1.0 / (cx * cx + cy * cy).sqrt();
        let words = Words::new().u(self.w).u(self.h).f(k).f(cx).f(cy).f(inv_max);
        self.frame_pass(Pipe::Vignette, words, Buf::Dummy);
    }
}

/// Plan `posts` for a `w x h` canvas. Effects with strength 0 (or NaN /
/// negative) and effects whose parameters are invalid plan nothing.
pub(crate) fn plan_posts(w: u32, h: u32, posts: &[ResolvedPost<'_>]) -> Plan {
    let mut pl = Planner {
        w,
        h,
        cur: 0,
        plan: Plan::default(),
    };
    if w == 0 || h == 0 {
        return pl.plan;
    }
    for p in posts {
        let s = p.strength;
        if s.is_nan() || s <= 0.0 {
            continue;
        }
        let s = s.min(1.0);
        match p.kind {
            PostKind::Bloom { threshold, radius } => pl.bloom(*threshold, *radius, s),
            PostKind::ChromaticAberration { shift, angle_deg } => {
                pl.chromatic(*shift, *angle_deg, s)
            }
            PostKind::Glitch {
                bands,
                max_shift,
                seed,
            } => pl.glitch(*bands, *max_shift, *seed, p.time, s),
            PostKind::Rays { center, length } => pl.rays(*center, *length, s),
            PostKind::DirectionalBlur { angle_deg, length } => {
                pl.directional_blur(*angle_deg, *length, s)
            }
            PostKind::Grain { amount, seed } => pl.grain(*amount, *seed, p.time, s),
            PostKind::Vignette { amount } => pl.vignette(*amount, s),
        }
    }
    pl.plan.result = pl.cur;
    pl.plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(kind: &PostKind, strength: f32) -> ResolvedPost<'_> {
        ResolvedPost {
            kind,
            strength,
            time: 0.5,
        }
    }

    #[test]
    fn skipped_effects_plan_nothing() {
        let kinds = [
            PostKind::Bloom {
                threshold: 0.5,
                radius: 24.0,
            },
            PostKind::ChromaticAberration {
                shift: 4.0,
                angle_deg: 10.0,
            },
            PostKind::Glitch {
                bands: 8,
                max_shift: 40.0,
                seed: 1,
            },
            PostKind::Rays {
                center: [10.0, 10.0],
                length: 0.5,
            },
            PostKind::DirectionalBlur {
                angle_deg: 0.0,
                length: 30.0,
            },
            PostKind::Grain {
                amount: 0.5,
                seed: 1,
            },
            PostKind::Vignette { amount: 0.5 },
        ];
        for k in &kinds {
            for s in [0.0, -1.0, f32::NAN] {
                let plan = plan_posts(64, 64, &[post(k, s)]);
                assert!(plan.passes.is_empty(), "{k:?} at {s}");
            }
            assert!(
                !plan_posts(64, 64, &[post(k, 1.0)]).passes.is_empty(),
                "{k:?}"
            );
        }
    }

    #[test]
    fn ping_pong_alternates_and_result_tracks_it() {
        let v = PostKind::Vignette { amount: 0.5 };
        let g = PostKind::Grain {
            amount: 0.5,
            seed: 1,
        };
        let plan = plan_posts(32, 32, &[post(&v, 1.0), post(&g, 1.0), post(&v, 1.0)]);
        assert_eq!(plan.passes.len(), 3);
        assert_eq!(plan.passes[0].src, Buf::Ping(0));
        assert_eq!(plan.passes[1].src, Buf::Ping(1));
        assert_eq!(plan.passes[2].src, Buf::Ping(0));
        assert_eq!(plan.result, 1);
    }

    #[test]
    fn bloom_levels_follow_the_cpu_pyramid() {
        assert_eq!(level_dims(1080, 1920, 7).len(), 7);
        assert_eq!(level_dims(1080, 1920, 7)[0], (540, 960));
        // Tiny canvases stop at 1 x 1.
        assert_eq!(level_dims(2, 2, 7), vec![(1, 1)]);
        assert_eq!(level_dims(5, 3, 7), vec![(3, 2), (2, 1), (1, 1)]);
        let b = PostKind::Bloom {
            threshold: 0.5,
            radius: 100.0,
        };
        let plan = plan_posts(256, 256, &[post(&b, 1.0)]);
        // bright + (m - 1) downs + (m - 1) merges + final
        assert_eq!(plan.levels, 6);
        assert_eq!(plan.passes.len(), 1 + 5 + 5 + 1);
    }
}
