//! Shared signal primitives for reference analysis (0.7, frozen).
//!
//! Every measurement runs on one low-resolution RGB [`AnalysisStream`]
//! decoded once by ffmpeg (`media::decode_analysis_stream`). Nothing here is
//! semantic: luma, differences, regions, indices.

/// Low-resolution RGB24 frames at a fixed cadence.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisStream {
    pub width: u32,
    pub height: u32,
    /// Frames per second of the stream (not necessarily the source fps).
    pub fps: f64,
    /// `width * height * 3` bytes each, row-major RGB.
    pub frames: Vec<Vec<u8>>,
}

impl AnalysisStream {
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Time of frame `i` in seconds (`i / fps`).
    pub fn time_of(&self, i: usize) -> f64 {
        i as f64 / self.fps
    }

    /// Nearest frame to `t` seconds, clamped to the stream.
    pub fn frame_at(&self, t: f64) -> usize {
        ((t * self.fps).round().max(0.0) as usize).min(self.len().saturating_sub(1))
    }

    /// Luma of frame `i` (see [`luma`]).
    pub fn luma(&self, i: usize) -> Vec<f32> {
        luma(&self.frames[i])
    }

    /// Indices of a `per_second` subsample (`round(k · fps / per_second)`),
    /// deduplicated, in order; always contains frame 0 when non-empty.
    pub fn subsample(&self, per_second: f64) -> Vec<usize> {
        subsample_indices(self.len(), self.fps, per_second)
    }
}

/// Rec. 709 luma of gamma-encoded RGB24, `0..=1` per pixel.
pub fn luma(rgb: &[u8]) -> Vec<f32> {
    rgb.chunks_exact(3)
        .map(|p| (0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0)
        .collect()
}

/// Mean absolute difference of two equally sized luma planes.
pub fn mean_abs_diff(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    if a.is_empty() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
}

/// Mean absolute difference of two equally sized RGB24 frames, `0..=1`:
/// `Σ(|ΔR| + |ΔG| + |ΔB|) / (3 · 255 · pixels)`. Equals the luma difference
/// for grey content, and also sees isoluminant changes (a red → blue cut)
/// that a luma difference misses. This is the structural-change measure of
/// all temporal evidence.
pub fn rgb_mean_abs_diff(a: &[u8], b: &[u8]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    if a.is_empty() {
        return 0.0;
    }
    let sum: u64 = a.iter().zip(b).map(|(&x, &y)| x.abs_diff(y) as u64).sum();
    sum as f32 / (a.len() as f32 * 255.0)
}

/// [`rgb_mean_abs_diff`] restricted to the pixels inside (`inside = true`) or
/// outside (`false`) `rect`. 0 when the selection is empty.
pub fn region_rgb_mean_abs_diff(a: &[u8], b: &[u8], width: u32, rect: Rect, inside: bool) -> f32 {
    let mut sum = 0u64;
    let mut n = 0u64;
    for (i, (pa, pb)) in a.chunks_exact(3).zip(b.chunks_exact(3)).enumerate() {
        let (px, py) = (i as u32 % width, i as u32 / width);
        if rect.contains(px, py) == inside {
            sum += pa
                .iter()
                .zip(pb)
                .map(|(&x, &y)| x.abs_diff(y) as u64)
                .sum::<u64>();
            n += 3;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum as f32 / (n as f32 * 255.0)
    }
}

/// Pixel rectangle `[x0, x1) × [y0, y1)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Rect {
    /// The central region left after removing `margin` (fraction of each side).
    pub fn inset(width: u32, height: u32, margin: f32) -> Rect {
        let mx = (width as f32 * margin).round() as u32;
        let my = (height as f32 * margin).round() as u32;
        Rect {
            x0: mx,
            y0: my,
            x1: width.saturating_sub(mx).max(mx),
            y1: height.saturating_sub(my).max(my),
        }
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x0 && x < self.x1 && y >= self.y0 && y < self.y1
    }
}

/// Mean absolute luma difference over the pixels inside (`inside = true`) or
/// outside (`false`) `rect`. 0 when the selection is empty.
pub fn region_mean_abs_diff(a: &[f32], b: &[f32], width: u32, rect: Rect, inside: bool) -> f32 {
    let mut sum = 0.0f32;
    let mut n = 0u32;
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let px = i as u32 % width;
        let py = i as u32 / width;
        if rect.contains(px, py) == inside {
            sum += (x - y).abs();
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f32
    }
}

/// See [`AnalysisStream::subsample`].
pub fn subsample_indices(len: usize, fps: f64, per_second: f64) -> Vec<usize> {
    if len == 0 || fps <= 0.0 || per_second <= 0.0 {
        return Vec::new();
    }
    let step = fps / per_second;
    let mut out: Vec<usize> = Vec::new();
    let mut k = 0usize;
    loop {
        let i = (k as f64 * step).round() as usize;
        if i >= len {
            break;
        }
        if out.last() != Some(&i) {
            out.push(i);
        }
        k += 1;
    }
    out
}

pub use motion_core::reference::evidence::ChangeKind;

/// One meaningful visual change, in analysis-frame indices.
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeEvent {
    /// First frame whose *incoming* difference is elevated (the change begins
    /// between `start - 1` and `start`).
    pub start: usize,
    /// Last frame of the change (inclusive); the new composition is visible
    /// from `end`. A cut has `start == end`.
    pub end: usize,
    /// Frame with the largest incoming difference.
    pub peak: usize,
    pub kind: ChangeKind,
    /// Peak incoming mean absolute RGB difference (0..1); for evolutions the
    /// accumulated difference across the change.
    pub magnitude: f32,
}

/// Per-transition differences and the meaningful changes derived from them.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TemporalSignals {
    /// `diffs[i]` = mean absolute RGB difference ([`rgb_mean_abs_diff`]) from frame `i` to `i + 1`
    /// (`len = frames - 1`).
    pub diffs: Vec<f32>,
    /// Time-ordered, non-overlapping.
    pub events: Vec<ChangeEvent>,
    /// Estimated per-transition noise (film grain, sensor/compression noise):
    /// the 10th-percentile raw difference, at most 0.01. Already subtracted
    /// from `diffs`; event detection and magnitudes retain raw differences.
    pub noise_floor: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subsample_is_ordered_and_bounded() {
        assert_eq!(subsample_indices(10, 30.0, 4.0), vec![0, 8]);
        assert_eq!(subsample_indices(31, 30.0, 4.0), vec![0, 8, 15, 23, 30]);
        assert!(subsample_indices(0, 30.0, 4.0).is_empty());
        assert_eq!(subsample_indices(3, 2.0, 4.0), vec![0, 1, 2]);
    }

    #[test]
    fn luma_and_diff() {
        let white = vec![255u8; 12];
        let black = vec![0u8; 12];
        assert!((luma(&white)[0] - 1.0).abs() < 1e-6);
        assert!((mean_abs_diff(&luma(&white), &luma(&black)) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn rgb_diff_sees_isoluminant_cuts() {
        // Red and a blue of similar luma: luma barely moves, RGB does.
        let red = [224u8, 48, 32].repeat(4);
        let blue = [32u8, 64, 224].repeat(4);
        let dl = mean_abs_diff(&luma(&red), &luma(&blue));
        let drgb = rgb_mean_abs_diff(&red, &blue);
        assert!(dl < 0.1 && drgb > 0.5, "luma {dl} rgb {drgb}");
        let grey_a = [100u8; 12];
        let grey_b = [150u8; 12];
        let d = mean_abs_diff(&luma(&grey_a), &luma(&grey_b));
        assert!((rgb_mean_abs_diff(&grey_a, &grey_b) - d).abs() < 1e-5);
        let r = Rect::inset(2, 2, 0.0);
        assert!(region_rgb_mean_abs_diff(&red, &blue, 2, r, true) > 0.5);
        assert_eq!(region_rgb_mean_abs_diff(&red, &blue, 2, r, false), 0.0);
    }

    #[test]
    fn region_diff_splits_inside_and_outside() {
        // 4x4, only the centre 2x2 differs.
        let a = vec![0.0f32; 16];
        let mut b = a.clone();
        for i in [5, 6, 9, 10] {
            b[i] = 1.0;
        }
        let r = Rect::inset(4, 4, 0.25);
        assert_eq!(region_mean_abs_diff(&a, &b, 4, r, true), 1.0);
        assert_eq!(region_mean_abs_diff(&a, &b, 4, r, false), 0.0);
    }
}
