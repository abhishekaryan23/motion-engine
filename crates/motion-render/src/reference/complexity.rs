//! Lightweight static-complexity evidence: edges, flat areas, large color
//! regions, tonal entropy, occupancy. Measurement only.

use motion_core::reference::evidence::{ComplexityEvidence, Distribution};

use super::signal::{luma, AnalysisStream};

const EDGE_THRESHOLD: f32 = 0.08;
const FLAT_VARIANCE: f32 = 0.0004;
const REGION_MIN_SHARE: f32 = 0.02;
const HIST_BINS: usize = 32;
const BLOCK: usize = 8;

pub fn complexity_evidence(stream: &AnalysisStream) -> ComplexityEvidence {
    let (w, h) = (stream.width as usize, stream.height as usize);
    let mut edge = Vec::new();
    let mut flat = Vec::new();
    let mut regions = Vec::new();
    let mut entropy = Vec::new();
    let mut occupied = Vec::new();
    for i in stream.subsample(4.0) {
        let Some(rgb) = stream.frames.get(i) else {
            continue;
        };
        if w == 0 || h == 0 || rgb.len() < w * h * 3 {
            continue;
        }
        let rgb = &rgb[..w * h * 3];
        let l = luma(rgb);
        edge.push(edge_density(&l, w, h));
        flat.push(flat_fraction(&l, w, h));
        entropy.push(luma_entropy(&l));
        let q: Vec<u16> = rgb
            .chunks_exact(3)
            .map(|p| ((p[0] >> 5) as u16) << 6 | ((p[1] >> 5) as u16) << 3 | (p[2] >> 5) as u16)
            .collect();
        regions.push(large_regions(&q, w, h) as f32);
        occupied.push(occupied_ratio(&q, w, h));
    }
    ComplexityEvidence {
        edge_density: dist(&edge),
        flat_area_fraction: dist(&flat),
        large_region_count: dist(&regions),
        entropy: dist(&entropy),
        occupied_ratio: dist(&occupied),
    }
}

fn r4(x: f32) -> f32 {
    ((x as f64 * 1e4).round() / 1e4) as f32
}

fn dist(values: &[f32]) -> Distribution {
    let d = Distribution::of(values);
    Distribution {
        mean: r4(d.mean),
        p10: r4(d.p10),
        p50: r4(d.p50),
        p90: r4(d.p90),
    }
}

/// Share of interior pixels with Sobel `(|gx| + |gy|) / 8 > 0.08`.
fn edge_density(l: &[f32], w: usize, h: usize) -> f32 {
    if w < 3 || h < 3 {
        return 0.0;
    }
    let mut strong = 0u32;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let p = |dx: usize, dy: usize| l[(y + dy - 1) * w + (x + dx - 1)];
            let gx = (p(2, 0) + 2.0 * p(2, 1) + p(2, 2)) - (p(0, 0) + 2.0 * p(0, 1) + p(0, 2));
            let gy = (p(0, 2) + 2.0 * p(1, 2) + p(2, 2)) - (p(0, 0) + 2.0 * p(1, 0) + p(2, 0));
            if (gx.abs() + gy.abs()) / 8.0 > EDGE_THRESHOLD {
                strong += 1;
            }
        }
    }
    strong as f32 / ((w - 2) * (h - 2)) as f32
}

/// Share of complete 8×8 blocks with luma variance < 0.0004.
fn flat_fraction(l: &[f32], w: usize, h: usize) -> f32 {
    let (bw, bh) = (w / BLOCK, h / BLOCK);
    if bw == 0 || bh == 0 {
        return 0.0;
    }
    let mut flat = 0u32;
    for by in 0..bh {
        for bx in 0..bw {
            let mut sum = 0.0f64;
            let mut sq = 0.0f64;
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    let v = l[(by * BLOCK + y) * w + bx * BLOCK + x] as f64;
                    sum += v;
                    sq += v * v;
                }
            }
            let n = (BLOCK * BLOCK) as f64;
            let mean = sum / n;
            let var = (sq / n - mean * mean).max(0.0);
            if var < FLAT_VARIANCE as f64 {
                flat += 1;
            }
        }
    }
    flat as f32 / (bw * bh) as f32
}

/// 32-bin luma histogram entropy, normalised by log2(32).
fn luma_entropy(l: &[f32]) -> f32 {
    if l.is_empty() {
        return 0.0;
    }
    let mut hist = [0u32; HIST_BINS];
    for &v in l {
        let b = ((v.clamp(0.0, 1.0) * HIST_BINS as f32) as usize).min(HIST_BINS - 1);
        hist[b] += 1;
    }
    let n = l.len() as f64;
    let e: f64 = hist
        .iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum();
    (e / (HIST_BINS as f64).log2()).clamp(0.0, 1.0) as f32
}

/// 4-connected components of equal quantized color covering ≥ 2 % of the frame.
fn large_regions(q: &[u16], w: usize, h: usize) -> u32 {
    let n = w * h;
    let min_size = REGION_MIN_SHARE * n as f32;
    let mut seen = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut count = 0u32;
    for start in 0..n {
        if seen[start] {
            continue;
        }
        let color = q[start];
        seen[start] = true;
        stack.push(start);
        let mut size = 0usize;
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                if !seen[j] && q[j] == color {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if size as f32 >= min_size {
            count += 1;
        }
    }
    count
}

/// Share of complete 8×8 blocks where more than half the pixels are not the
/// frame's dominant quantized color.
fn occupied_ratio(q: &[u16], w: usize, h: usize) -> f32 {
    let (bw, bh) = (w / BLOCK, h / BLOCK);
    if bw == 0 || bh == 0 {
        return 0.0;
    }
    let mut hist = [0u32; 512];
    for &c in q {
        hist[c as usize] += 1;
    }
    let mut dominant = 0usize;
    for (c, &n) in hist.iter().enumerate() {
        if n > hist[dominant] {
            dominant = c;
        }
    }
    let mut occupied = 0u32;
    for by in 0..bh {
        for bx in 0..bw {
            let mut other = 0usize;
            for y in 0..BLOCK {
                for x in 0..BLOCK {
                    if q[(by * BLOCK + y) * w + bx * BLOCK + x] as usize != dominant {
                        other += 1;
                    }
                }
            }
            if other * 2 > BLOCK * BLOCK {
                occupied += 1;
            }
        }
    }
    occupied as f32 / (bw * bh) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 32;
    const H: usize = 48;

    fn stream_of(frame: Vec<u8>) -> AnalysisStream {
        AnalysisStream {
            width: W as u32,
            height: H as u32,
            fps: 8.0,
            frames: vec![frame; 8],
        }
    }

    fn build(mut f: impl FnMut(usize, usize) -> [u8; 3]) -> AnalysisStream {
        let mut px = Vec::with_capacity(W * H * 3);
        for y in 0..H {
            for x in 0..W {
                px.extend_from_slice(&f(x, y));
            }
        }
        stream_of(px)
    }

    #[test]
    fn solid_frame() {
        let c = complexity_evidence(&build(|_, _| [200, 200, 200]));
        assert_eq!(c.edge_density.mean, 0.0);
        assert_eq!(c.flat_area_fraction.mean, 1.0);
        assert_eq!(c.large_region_count.mean, 1.0);
        assert_eq!(c.entropy.mean, 0.0);
        assert_eq!(c.occupied_ratio.mean, 0.0);
    }

    #[test]
    fn empty_stream_is_zero() {
        let s = AnalysisStream {
            width: 32,
            height: 48,
            fps: 8.0,
            frames: Vec::new(),
        };
        let c = complexity_evidence(&s);
        assert_eq!(c.edge_density, Distribution::default());
    }

    #[test]
    fn noise_is_busy() {
        let mut state = 12345u32;
        let noise = build(|_, _| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let v = (state >> 24) as u8;
            [v, v, v]
        });
        let c = complexity_evidence(&noise);
        assert!(c.edge_density.mean > 0.5, "{}", c.edge_density.mean);
        assert!(c.flat_area_fraction.mean < 0.1);
        assert!(c.entropy.mean > 0.8);
    }

    #[test]
    fn checkerboard_is_busy() {
        let c = complexity_evidence(&build(|x, y| {
            if (x / 4 + y / 4) % 2 == 0 {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            }
        }));
        assert!(c.edge_density.mean > 0.3, "{}", c.edge_density.mean);
        assert!(c.flat_area_fraction.mean < 0.1);
    }

    #[test]
    fn two_color_split_has_two_regions() {
        let c = complexity_evidence(&build(|x, _| {
            if x < W / 2 {
                [255, 255, 255]
            } else {
                [0, 0, 0]
            }
        }));
        assert_eq!(c.large_region_count.mean, 2.0);
        assert!(c.entropy.mean > 0.1 && c.entropy.mean < 0.3);
    }

    #[test]
    fn sparse_vs_dense() {
        let sparse = complexity_evidence(&build(|x, y| {
            if (10..20).contains(&x) && (10..20).contains(&y) {
                [200, 30, 30]
            } else {
                [245, 245, 245]
            }
        }));
        // 6x6 squares on an 8-px pitch, alternating colors; ground stays dominant.
        let dense = complexity_evidence(&build(|x, y| {
            if (1..7).contains(&(x % 8)) && (1..7).contains(&(y % 8)) {
                if (x / 8 + y / 8) % 2 == 0 {
                    [200, 30, 30]
                } else {
                    [30, 30, 200]
                }
            } else {
                [245, 245, 245]
            }
        }));
        assert!(sparse.occupied_ratio.mean < dense.occupied_ratio.mean);
        assert!(sparse.large_region_count.mean < dense.large_region_count.mean);
        assert!(sparse.large_region_count.mean >= 2.0);
        assert!(dense.occupied_ratio.mean > 0.9);
    }

    #[test]
    fn deterministic() {
        let s = build(|x, y| [(x * 8) as u8, (y * 5) as u8, 90]);
        assert_eq!(complexity_evidence(&s), complexity_evidence(&s));
    }
}
