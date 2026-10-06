//! Deterministic color evidence: luminance, polarity, temperature, a
//! quantized palette with roles, and palette stability. Measurement only.

use motion_core::reference::evidence::{
    ColorEvidence, Distribution, EvidencePolarity, EvidenceTemperature, PolarityEstimate,
    SampleColor, Swatch, SwatchRole, TemperatureEstimate,
};
use motion_core::reference::oklab::{lch, oklab, to_hex};

use super::signal::{luma, AnalysisStream};

const DARK_LUMA: f32 = 0.35;
const LIGHT_LUMA: f32 = 0.65;
const MIN_CHROMA_WEIGHT: f32 = 0.012;
const CLUSTER_DELTA_E: f32 = 0.08;
const MAX_CLUSTERS: usize = 24;
const MAX_SWATCHES: usize = 8;
const MIN_PREVALENCE: f64 = 0.01;
const LOW_CHROMA: f32 = 0.06;
const ACCENT_CHROMA: f32 = 0.10;
const ACCENT_MAX_PREVALENCE: f32 = 0.25;
const BINS: usize = 4096;

/// `samples` = `(sample id, analysis frame)` for `per_sample`, in sample order.
pub fn color_evidence(stream: &AnalysisStream, samples: &[(String, usize)]) -> ColorEvidence {
    let idx = stream.subsample(4.0);
    let frames: Vec<&[u8]> = idx
        .iter()
        .filter_map(|&i| stream.frames.get(i))
        .map(|f| f.as_slice())
        .collect();
    if frames.is_empty() {
        return empty_evidence(samples);
    }

    // Per-frame statistics.
    let mut means = Vec::with_capacity(frames.len());
    let mut contrasts = Vec::with_capacity(frames.len());
    let mut sats = Vec::with_capacity(frames.len());
    for f in &frames {
        let (m, c, s) = frame_stats(f);
        means.push(m);
        contrasts.push(c);
        sats.push(s);
    }
    let n_frames = means.len() as f32;
    let dark_frame_fraction = means.iter().filter(|&&m| m < DARK_LUMA).count() as f32 / n_frames;
    let light_frame_fraction = means.iter().filter(|&&m| m > LIGHT_LUMA).count() as f32 / n_frames;
    let mean_of_means = means.iter().sum::<f32>() / n_frames;
    let polarity = polarity(mean_of_means, dark_frame_fraction, light_frame_fraction);

    let (temperature, balance) = temperature(&frames);

    // Palette.
    let model = PaletteModel::build(&frames);
    let palette = model.palette();
    let accent_prevalence = model.accent_prevalence();
    let palette_stability = model.stability(&frames);

    let per_sample = samples
        .iter()
        .map(|(id, frame)| {
            let last = stream.frames.len().saturating_sub(1);
            match stream.frames.get((*frame).min(last)) {
                Some(f) => {
                    let (m, _, s) = frame_stats(f);
                    SampleColor {
                        id: id.clone(),
                        mean_luma: r4(m),
                        mean_saturation: r4(s),
                        dominant: model.dominant_of(f),
                    }
                }
                None => zero_sample(id),
            }
        })
        .collect();

    ColorEvidence {
        polarity,
        dark_frame_fraction: r4(dark_frame_fraction),
        light_frame_fraction: r4(light_frame_fraction),
        luminance: dist(&means),
        contrast: dist(&contrasts),
        saturation: dist(&sats),
        temperature,
        warm_cool_balance: r4(balance),
        palette,
        accent_prevalence: r4(accent_prevalence),
        palette_stability: r4(palette_stability),
        per_sample,
    }
}

fn zero_sample(id: &str) -> SampleColor {
    SampleColor {
        id: id.to_string(),
        mean_luma: 0.0,
        mean_saturation: 0.0,
        dominant: to_hex(0),
    }
}

fn empty_evidence(samples: &[(String, usize)]) -> ColorEvidence {
    ColorEvidence {
        polarity: PolarityEstimate {
            value: EvidencePolarity::Light,
            confidence: 0.3,
        },
        dark_frame_fraction: 0.0,
        light_frame_fraction: 0.0,
        luminance: Distribution::default(),
        contrast: Distribution::default(),
        saturation: Distribution::default(),
        temperature: TemperatureEstimate {
            value: EvidenceTemperature::Neutral,
            confidence: 0.3,
        },
        warm_cool_balance: 0.0,
        palette: Vec::new(),
        accent_prevalence: 0.0,
        palette_stability: 0.0,
        per_sample: samples.iter().map(|(id, _)| zero_sample(id)).collect(),
    }
}

/// Round to 4 decimals (stable JSON).
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

/// `(mean luma, p90 − p10 luma, mean HSV saturation)` of one RGB24 frame.
fn frame_stats(rgb: &[u8]) -> (f32, f32, f32) {
    let l = luma(rgb);
    if l.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mean = l.iter().sum::<f32>() / l.len() as f32;
    let d = Distribution::of(&l);
    let mut sat = 0.0f64;
    for p in rgb.chunks_exact(3) {
        let max = p[0].max(p[1]).max(p[2]);
        let min = p[0].min(p[1]).min(p[2]);
        if max > 0 {
            sat += (max - min) as f64 / max as f64;
        }
    }
    (
        mean,
        (d.p90 - d.p10).max(0.0),
        (sat / l.len() as f64) as f32,
    )
}

fn polarity(mean: f32, dark: f32, light: f32) -> PolarityEstimate {
    if dark >= 0.25 && light >= 0.25 {
        let (lo, hi) = if dark < light {
            (dark, light)
        } else {
            (light, dark)
        };
        return PolarityEstimate {
            value: EvidencePolarity::Mixed,
            confidence: r4(0.5 + 0.45 * (lo / hi)),
        };
    }
    let (value, side, other) = if mean < 0.5 {
        (EvidencePolarity::Dark, dark, light)
    } else {
        (EvidencePolarity::Light, light, dark)
    };
    let c_mean = ((mean - 0.5).abs() / 0.3).clamp(0.0, 1.0);
    let c_frac = (side - other).clamp(0.0, 1.0);
    let clarity = c_mean.max(c_frac);
    PolarityEstimate {
        value,
        confidence: r4((0.3 + 0.67 * clarity).clamp(0.3, 0.97)),
    }
}

fn temperature(frames: &[&[u8]]) -> (TemperatureEstimate, f32) {
    // Direct-mapped cache of (color key, chroma, hue); purely an optimisation.
    const SLOTS: usize = 1 << 14;
    let mut cache: Vec<(u32, f32, f32)> = vec![(u32::MAX, 0.0, 0.0); SLOTS];
    let mut signed = 0.0f64;
    let mut total = 0.0f64;
    for f in frames {
        for p in f.chunks_exact(3) {
            let key = (p[0] as u32) << 16 | (p[1] as u32) << 8 | p[2] as u32;
            let slot = (key.wrapping_mul(0x9E37_79B1) >> 18) as usize % SLOTS;
            let (c, h) = if cache[slot].0 == key {
                (cache[slot].1, cache[slot].2)
            } else {
                let (_, c, h) = lch(p[0], p[1], p[2]);
                cache[slot] = (key, c, h);
                (c, h)
            };
            total += c.max(MIN_CHROMA_WEIGHT) as f64;
            let w = if c >= MIN_CHROMA_WEIGHT {
                c as f64
            } else {
                0.0
            };
            let sign = if !(120.0..330.0).contains(&h) {
                1.0
            } else if (150.0..300.0).contains(&h) {
                -1.0
            } else {
                0.0
            };
            signed += sign * w;
        }
    }
    let balance = if total > 0.0 {
        (signed / total).clamp(-1.0, 1.0) as f32
    } else {
        0.0
    };
    let a = balance.abs();
    let est = if a < 0.08 {
        TemperatureEstimate {
            value: EvidenceTemperature::Neutral,
            confidence: r4((0.9 - 5.0 * a).clamp(0.5, 0.9)),
        }
    } else {
        TemperatureEstimate {
            value: if balance > 0.0 {
                EvidenceTemperature::Warm
            } else {
                EvidenceTemperature::Cool
            },
            confidence: r4((0.5 + 2.0 * a).clamp(0.5, 0.95)),
        }
    };
    (est, balance)
}

fn bin_of(p: &[u8]) -> usize {
    ((p[0] >> 4) as usize) << 8 | ((p[1] >> 4) as usize) << 4 | (p[2] >> 4) as usize
}

#[derive(Debug, Clone)]
struct Cluster {
    count: u64,
    sum: [u64; 3],
    rgb: [u8; 3],
    lab: (f32, f32, f32),
}

impl Cluster {
    fn refresh(&mut self) {
        let n = self.count.max(1) as f64;
        for k in 0..3 {
            self.rgb[k] = (self.sum[k] as f64 / n).round().clamp(0.0, 255.0) as u8;
        }
        self.lab = oklab(self.rgb[0], self.rgb[1], self.rgb[2]);
    }

    fn hex(&self) -> String {
        to_hex((self.rgb[0] as u32) << 16 | (self.rgb[1] as u32) << 8 | self.rgb[2] as u32)
    }

    fn lch(&self) -> (f32, f32, f32) {
        lch(self.rgb[0], self.rgb[1], self.rgb[2])
    }
}

fn delta_e(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2) + (a.2 - b.2).powi(2)).sqrt()
}

/// Index of the cluster nearest to `lab` (ties: earlier cluster).
fn nearest(clusters: &[Cluster], lab: (f32, f32, f32)) -> usize {
    let mut best = 0usize;
    let mut best_d = f32::INFINITY;
    for (i, c) in clusters.iter().enumerate() {
        let d = delta_e(c.lab, lab);
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

struct PaletteModel {
    clusters: Vec<Cluster>,
    total: u64,
    /// Nearest final cluster of every 4-bit bin (meaningful for non-empty bins).
    bin_cluster: Vec<usize>,
}

impl PaletteModel {
    fn build(frames: &[&[u8]]) -> PaletteModel {
        let mut counts = vec![0u64; BINS];
        let mut sums = vec![[0u64; 3]; BINS];
        let mut total = 0u64;
        for f in frames {
            for p in f.chunks_exact(3) {
                let b = bin_of(p);
                counts[b] += 1;
                for k in 0..3 {
                    sums[b][k] += p[k] as u64;
                }
                total += 1;
            }
        }
        let mut order: Vec<usize> = (0..BINS).filter(|&b| counts[b] > 0).collect();
        order.sort_by(|&a, &b| counts[b].cmp(&counts[a]).then(a.cmp(&b)));

        let bin_rgb = |b: usize| {
            let n = counts[b] as f64;
            [
                (sums[b][0] as f64 / n).round() as u8,
                (sums[b][1] as f64 / n).round() as u8,
                (sums[b][2] as f64 / n).round() as u8,
            ]
        };

        let mut clusters: Vec<Cluster> = Vec::new();
        for &b in &order {
            let rgb = bin_rgb(b);
            let lab = oklab(rgb[0], rgb[1], rgb[2]);
            let within = clusters
                .iter()
                .position(|c| delta_e(c.lab, lab) <= CLUSTER_DELTA_E);
            let target = match within {
                Some(i) => Some(i),
                None if clusters.len() >= MAX_CLUSTERS => Some(nearest(&clusters, lab)),
                None => None,
            };
            match target {
                Some(i) => {
                    let c = &mut clusters[i];
                    c.count += counts[b];
                    for (s, add) in c.sum.iter_mut().zip(sums[b]) {
                        *s += add;
                    }
                    c.refresh();
                }
                None => {
                    let mut c = Cluster {
                        count: counts[b],
                        sum: sums[b],
                        rgb,
                        lab,
                    };
                    c.refresh();
                    clusters.push(c);
                }
            }
        }

        let mut bin_cluster = vec![0usize; BINS];
        if !clusters.is_empty() {
            for &b in &order {
                let rgb = bin_rgb(b);
                bin_cluster[b] = nearest(&clusters, oklab(rgb[0], rgb[1], rgb[2]));
            }
        }
        PaletteModel {
            clusters,
            total,
            bin_cluster,
        }
    }

    fn prevalence(&self, i: usize) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.clusters[i].count as f64 / self.total as f64
        }
    }

    fn is_accent(&self, i: usize) -> bool {
        let (_, c, _) = self.clusters[i].lch();
        c >= ACCENT_CHROMA && (self.prevalence(i) as f32) < ACCENT_MAX_PREVALENCE
    }

    fn accent_prevalence(&self) -> f32 {
        (0..self.clusters.len())
            .filter(|&i| self.is_accent(i))
            .map(|i| self.prevalence(i))
            .sum::<f64>() as f32
    }

    fn palette(&self) -> Vec<Swatch> {
        let mut keep: Vec<usize> = (0..self.clusters.len())
            .filter(|&i| self.prevalence(i) >= MIN_PREVALENCE)
            .collect();
        // Stable sort: ties keep the earlier cluster first.
        keep.sort_by(|&a, &b| self.prevalence(b).total_cmp(&self.prevalence(a)));
        keep.truncate(MAX_SWATCHES);
        if keep.is_empty() {
            return Vec::new();
        }

        let lchs: Vec<(f32, f32, f32)> = keep.iter().map(|&i| self.clusters[i].lch()).collect();
        let low = |k: usize| lchs[k].1 < LOW_CHROMA;
        let ground = (0..keep.len()).find(|&k| low(k)).unwrap_or(0);
        let l_ground = lchs[ground].0;
        let mut ink: Option<usize> = None;
        let mut ink_diff = 0.4f32;
        for (k, lc) in lchs.iter().enumerate() {
            if k == ground || !low(k) {
                continue;
            }
            let d = (lc.0 - l_ground).abs();
            if d >= ink_diff && (ink.is_none() || d > ink_diff) {
                ink = Some(k);
                ink_diff = d;
            }
        }

        keep.iter()
            .enumerate()
            .map(|(k, &i)| {
                let prevalence = self.prevalence(i);
                let (l, c, h) = lchs[k];
                let role = if k == ground {
                    SwatchRole::Ground
                } else if Some(k) == ink {
                    SwatchRole::Ink
                } else if self.is_accent(i) {
                    SwatchRole::Accent
                } else {
                    SwatchRole::Neutral
                };
                Swatch {
                    hex: self.clusters[i].hex(),
                    // Floor so rounded prevalences never sum past 1.
                    prevalence: ((prevalence * 1e4 + 1e-6).floor() / 1e4) as f32,
                    lightness: r4(l),
                    chroma: r4(c),
                    hue_degrees: if c < 0.02 { None } else { Some(r4(h)) },
                    role,
                }
            })
            .collect()
    }

    /// Pixels of `rgb` per nearest cluster.
    fn histogram(&self, rgb: &[u8]) -> Vec<u64> {
        let mut h = vec![0u64; self.clusters.len()];
        for p in rgb.chunks_exact(3) {
            h[self.bin_cluster[bin_of(p)]] += 1;
        }
        h
    }

    fn dominant_of(&self, rgb: &[u8]) -> String {
        let h = self.histogram(rgb);
        let mut best = 0usize;
        for (i, &n) in h.iter().enumerate() {
            if n > h[best] {
                best = i;
            }
        }
        match self.clusters.get(best) {
            Some(c) => c.hex(),
            None => to_hex(0),
        }
    }

    fn stability(&self, frames: &[&[u8]]) -> f32 {
        if self.clusters.is_empty() || frames.is_empty() {
            return 0.0;
        }
        let hists: Vec<Vec<u64>> = frames.iter().map(|f| self.histogram(f)).collect();
        let mut global = vec![0u64; self.clusters.len()];
        for h in &hists {
            for (g, n) in global.iter_mut().zip(h) {
                *g += n;
            }
        }
        let global_total: u64 = global.iter().sum();
        if global_total == 0 {
            return 0.0;
        }
        let mut acc = 0.0f64;
        for h in &hists {
            let ft: u64 = h.iter().sum();
            if ft == 0 {
                continue;
            }
            for (n, g) in h.iter().zip(&global) {
                acc += (*n as f64 / ft as f64).min(*g as f64 / global_total as f64);
            }
        }
        (acc / frames.len() as f64).clamp(0.0, 1.0) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 32;
    const H: usize = 48;

    fn solid(rgb: [u8; 3]) -> Vec<u8> {
        (0..W * H).flat_map(|_| rgb).collect()
    }

    fn stream(frames: Vec<Vec<u8>>) -> AnalysisStream {
        AnalysisStream {
            width: W as u32,
            height: H as u32,
            fps: 8.0,
            frames,
        }
    }

    fn repeat(rgb: [u8; 3], n: usize) -> Vec<Vec<u8>> {
        (0..n).map(|_| solid(rgb)).collect()
    }

    #[test]
    fn empty_stream_is_all_zero() {
        let e = color_evidence(&stream(Vec::new()), &[("s01".into(), 0)]);
        assert_eq!(e.polarity.value, EvidencePolarity::Light);
        assert_eq!(e.polarity.confidence, 0.3);
        assert_eq!(e.temperature.value, EvidenceTemperature::Neutral);
        assert_eq!(e.temperature.confidence, 0.3);
        assert!(e.palette.is_empty());
        assert_eq!(e.luminance, Distribution::default());
        assert_eq!(e.per_sample.len(), 1);
    }

    #[test]
    fn static_light_is_light_and_neutral() {
        let e = color_evidence(&stream(repeat([0xF0; 3], 16)), &[]);
        assert_eq!(e.polarity.value, EvidencePolarity::Light);
        assert!(e.polarity.confidence >= 0.85, "{}", e.polarity.confidence);
        assert_eq!(e.dark_frame_fraction, 0.0);
        assert_eq!(e.light_frame_fraction, 1.0);
        assert_eq!(e.temperature.value, EvidenceTemperature::Neutral);
        assert!(e.warm_cool_balance.abs() < 0.08);
    }

    #[test]
    fn static_dark_is_dark() {
        let e = color_evidence(&stream(repeat([0x10; 3], 16)), &[]);
        assert_eq!(e.polarity.value, EvidencePolarity::Dark);
        assert!(e.polarity.confidence >= 0.85, "{}", e.polarity.confidence);
        assert_eq!(e.dark_frame_fraction, 1.0);
        assert_eq!(e.light_frame_fraction, 0.0);
    }

    #[test]
    fn light_then_dark_halves_are_mixed() {
        let mut frames = repeat([0xF0; 3], 8);
        frames.extend(repeat([0x10; 3], 8));
        let e = color_evidence(&stream(frames), &[]);
        assert_eq!(e.polarity.value, EvidencePolarity::Mixed);
        assert!(e.polarity.confidence > 0.9);
    }

    #[test]
    fn mid_grey_is_low_confidence() {
        let e = color_evidence(&stream(repeat([0x80; 3], 16)), &[]);
        assert!(e.polarity.confidence < 0.6, "{}", e.polarity.confidence);
    }

    #[test]
    fn saturation_distribution_is_ordered() {
        let mut vivid = repeat([255, 0, 0], 4);
        vivid.extend(repeat([0, 0, 255], 4));
        let vivid = color_evidence(&stream(vivid), &[]);
        let grey = color_evidence(&stream(repeat([0x80; 3], 8)), &[]);
        assert!(vivid.saturation.mean > 0.9);
        assert!(grey.saturation.mean < 0.01);
        assert!(vivid.saturation.p50 > grey.saturation.p50);
    }

    #[test]
    fn warm_and_cool_temperature() {
        let warm = color_evidence(&stream(repeat([0xE0, 0x70, 0x20], 16)), &[]);
        assert_eq!(warm.temperature.value, EvidenceTemperature::Warm);
        assert!(warm.warm_cool_balance > 0.3, "{}", warm.warm_cool_balance);
        let cool = color_evidence(&stream(repeat([0x20, 0x50, 0xD0], 16)), &[]);
        assert_eq!(cool.temperature.value, EvidenceTemperature::Cool);
        assert!(cool.warm_cool_balance < -0.3, "{}", cool.warm_cool_balance);
    }

    /// 90 % grey rows, bottom 10 % red.
    fn grey_red_frame() -> Vec<u8> {
        let red_from = H - H / 10;
        let mut f = Vec::new();
        for y in 0..H {
            for _ in 0..W {
                f.extend_from_slice(if y >= red_from {
                    &[0xD0, 0x20, 0x20]
                } else {
                    &[0x80, 0x80, 0x80]
                });
            }
        }
        f
    }

    #[test]
    fn palette_shape_and_roles() {
        let e = color_evidence(&stream(vec![grey_red_frame(); 16]), &[]);
        let p = &e.palette;
        assert!(!p.is_empty() && p.len() <= 8);
        for w in p.windows(2) {
            assert!(w[0].prevalence >= w[1].prevalence);
        }
        assert!(p.iter().map(|s| s.prevalence).sum::<f32>() <= 1.0001);
        for s in p {
            assert_eq!(s.hex.len(), 7);
            assert!(s.hex.starts_with('#'));
            assert!(s.hex[1..].chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert_eq!(p[0].role, SwatchRole::Ground);
        assert!(p[0].chroma < 0.06);
        assert_eq!(p[0].hue_degrees, None);
        let accent = p.iter().find(|s| s.role == SwatchRole::Accent).unwrap();
        assert!(accent.chroma >= 0.10);
        assert!(accent.hue_degrees.is_some());
        assert!(e.accent_prevalence > 0.05 && e.accent_prevalence < 0.2);
    }

    #[test]
    fn ink_is_high_contrast_low_chroma() {
        // 70 % white ground, 30 % near-black.
        let mut f = Vec::new();
        for y in 0..H {
            for _ in 0..W {
                f.extend_from_slice(if y < H * 7 / 10 { &[250; 3] } else { &[10; 3] });
            }
        }
        let e = color_evidence(&stream(vec![f; 8]), &[]);
        assert_eq!(e.palette[0].role, SwatchRole::Ground);
        assert_eq!(e.palette[1].role, SwatchRole::Ink);
    }

    #[test]
    fn many_colors_cap_at_eight_swatches() {
        // 16 horizontal bands with well separated colors.
        let mut f = Vec::new();
        for y in 0..H {
            let band = (y * 16 / H) as u32;
            for _ in 0..W {
                f.extend_from_slice(&[
                    (band * 16) as u8,
                    (255 - band * 16) as u8,
                    ((band * 37) % 256) as u8,
                ]);
            }
        }
        let e = color_evidence(&stream(vec![f; 8]), &[]);
        assert!(e.palette.len() <= 8);
        assert!(e.palette.len() >= 3);
    }

    #[test]
    fn stability_constant_vs_alternating() {
        let constant = color_evidence(&stream(vec![grey_red_frame(); 16]), &[]);
        assert!(constant.palette_stability > 0.999);
        let mut frames = repeat([0x20, 0x40, 0xC0], 8);
        frames.extend(repeat([0xE0, 0xB0, 0x20], 8));
        let alt = color_evidence(&stream(frames), &[]);
        assert!(alt.palette_stability < 0.7, "{}", alt.palette_stability);
    }

    #[test]
    fn per_sample_is_aligned() {
        let mut frames = repeat([0xF0; 3], 8);
        frames.extend(repeat([0x10, 0x10, 0xE0], 8));
        let samples = vec![("s01".to_string(), 0usize), ("s02".to_string(), 12usize)];
        let e = color_evidence(&stream(frames), &samples);
        assert_eq!(e.per_sample.len(), 2);
        assert_eq!(e.per_sample[0].id, "s01");
        assert_eq!(e.per_sample[1].id, "s02");
        assert!(e.per_sample[0].mean_luma > 0.9);
        assert!(e.per_sample[1].mean_luma < 0.2);
        assert!(e.per_sample[1].mean_saturation > 0.9);
        assert_ne!(e.per_sample[0].dominant, e.per_sample[1].dominant);
        assert_eq!(e.per_sample[0].dominant, "#F0F0F0");
    }

    #[test]
    fn deterministic() {
        let s = stream(vec![grey_red_frame(); 12]);
        let samples = vec![("s01".to_string(), 3usize)];
        assert_eq!(color_evidence(&s, &samples), color_evidence(&s, &samples));
    }
}
