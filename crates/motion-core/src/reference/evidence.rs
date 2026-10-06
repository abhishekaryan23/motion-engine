//! Deterministic reference evidence (0.7): what the analyzer measured on a
//! reference video without any model. Frozen contract; computed by
//! `motion-render::reference`.
//!
//! Evidence is *measurement*, never a style decision: it says "72 % of frames
//! are dark, 4.1 structural changes per 10 s", not "dark technical". The
//! external interpreter reads it together with the sampled frames and writes a
//! [`super::ReferenceStyleProfile`]; the TasteDirector never reads evidence.
//!
//! All ratios are `0..=1` unless stated. Times are seconds from the start of
//! the reference, rounded to milliseconds. Audio is never analysed (0.7): the
//! metadata records only whether an audio stream exists.

use serde::{Deserialize, Serialize};

/// Version of the analyzer's algorithms. Part of the cache key: bump it when
/// any measurement changes.
pub const ANALYZER_VERSION: &str = "0.7.3";
/// Version of the evidence document layout.
pub const EVIDENCE_VERSION: &str = "0.1";

/// Everything measured on one reference video (`evidence.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceEvidence {
    /// [`EVIDENCE_VERSION`].
    pub version: String,
    /// [`ANALYZER_VERSION`].
    pub analyzer_version: String,
    /// `rf1-<16 hex>`: FNV-1a 64 over the media bytes, analyzer version and
    /// analysis configuration. Links a profile to the evidence it came from.
    pub reference_fingerprint: String,
    /// `fnv1a64-<16 hex>` of the media file bytes alone.
    pub media_fingerprint: String,
    pub metadata: ReferenceMetadata,
    pub analysis: AnalysisSpec,
    /// Key frames for the interpreter, in time order.
    pub samples: Vec<ReferenceSample>,
    pub color: ColorEvidence,
    pub temporal: TemporalEvidence,
    pub complexity: ComplexityEvidence,
}

/// Container/stream facts from ffprobe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_seconds: f64,
    /// Average frame rate of the video stream.
    pub fps: f64,
    pub frame_count: u64,
    /// `width / height`.
    pub aspect_ratio: f64,
    pub orientation: Orientation,
    /// ffprobe codec name of the video stream (e.g. `h264`).
    pub video_codec: String,
    /// An audio stream exists. It is ignored in 0.7 (no BPM, beats, speech).
    pub audio_present: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Vertical,
    Square,
    Landscape,
}

impl Orientation {
    /// Square when the aspect ratio is within 5 % of 1.
    pub fn from_size(width: u32, height: u32) -> Orientation {
        let r = width as f64 / height.max(1) as f64;
        if (r - 1.0).abs() <= 0.05 {
            Orientation::Square
        } else if r < 1.0 {
            Orientation::Vertical
        } else {
            Orientation::Landscape
        }
    }
}

/// The low-resolution stream every measurement runs on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisSpec {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// Number of analysis frames decoded.
    pub frames: u64,
    /// First line of `ffmpeg -version` (decoding is deterministic per build).
    pub decoder: String,
}

/// Why a frame was sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleKind {
    /// Evenly spaced, for general visual identity.
    Periodic,
    /// Just after a meaningful change settled (new composition).
    SceneChange,
    /// Inside an interval of unusually strong visual change.
    HighMotion,
    /// Middle of a stable hold (a composition meant to be read).
    Stable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSample {
    /// `s01`, `s02`, ... in time order.
    pub id: String,
    pub time_seconds: f64,
    /// Source frame index (`round(time * metadata.fps)`).
    pub frame: u64,
    /// Every reason this frame was chosen, sorted (a frame serves several).
    pub kinds: Vec<SampleKind>,
    /// Path relative to the bundle directory, e.g. `samples/s01.jpg`.
    pub image: String,
}

/// Summary of a per-frame quantity.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Distribution {
    pub mean: f32,
    pub p10: f32,
    pub p50: f32,
    pub p90: f32,
}

impl Distribution {
    /// Nearest-rank percentiles (`p = values[ceil(q·n) − 1]` of the sorted
    /// values); all zero when `values` is empty. NaNs are ignored.
    pub fn of(values: &[f32]) -> Distribution {
        let mut v: Vec<f32> = values.iter().copied().filter(|x| x.is_finite()).collect();
        if v.is_empty() {
            return Distribution::default();
        }
        v.sort_by(f32::total_cmp);
        let n = v.len();
        let q = |p: f32| v[((p * n as f32).ceil() as usize).clamp(1, n) - 1];
        Distribution {
            mean: v.iter().sum::<f32>() / n as f32,
            p10: q(0.10),
            p50: q(0.50),
            p90: q(0.90),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidencePolarity {
    Light,
    Dark,
    /// Substantial shares of both light and dark frames.
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolarityEstimate {
    pub value: EvidencePolarity,
    /// `0..=1`; ambiguous references report low confidence.
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTemperature {
    Warm,
    Cool,
    /// Grey or balanced.
    Neutral,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemperatureEstimate {
    pub value: EvidenceTemperature,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwatchRole {
    /// Most prevalent low-chroma color (the ground).
    Ground,
    /// High-contrast low-chroma color against the ground (text/ink).
    Ink,
    /// Saturated minority color.
    Accent,
    /// Any other color.
    Neutral,
}

/// One quantized color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Swatch {
    /// `#RRGGBB`.
    pub hex: String,
    /// Share of analysed pixels.
    pub prevalence: f32,
    /// Perceptual lightness `0..=1` (OKLab L).
    pub lightness: f32,
    /// Chroma (OKLab, `0..≈0.37`).
    pub chroma: f32,
    /// OKLab hue angle in degrees; `None` below chroma 0.02.
    pub hue_degrees: Option<f32>,
    pub role: SwatchRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleColor {
    /// Sample id (`s01`).
    pub id: String,
    pub mean_luma: f32,
    pub mean_saturation: f32,
    /// Most prevalent quantized color of that frame, `#RRGGBB`.
    pub dominant: String,
}

/// Color evidence over the analysis stream (4 frames/s subsample).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorEvidence {
    pub polarity: PolarityEstimate,
    /// Share of analysed frames with mean luma < 0.35.
    pub dark_frame_fraction: f32,
    /// Share of analysed frames with mean luma > 0.65.
    pub light_frame_fraction: f32,
    /// Per-frame mean luma (Rec. 709, `0..=1`).
    pub luminance: Distribution,
    /// Per-frame tonal contrast: luma p90 − p10 of the frame.
    pub contrast: Distribution,
    /// Per-frame mean saturation (HSV S).
    pub saturation: Distribution,
    pub temperature: TemperatureEstimate,
    /// `-1` (cool) .. `+1` (warm), chroma-weighted over all analysed pixels.
    pub warm_cool_balance: f32,
    /// 3..=8 swatches, prevalence descending.
    pub palette: Vec<Swatch>,
    /// Share of pixels whose color is a saturated minority (accent) color.
    pub accent_prevalence: f32,
    /// `0..=1`: how similar each frame's palette is to the piece palette
    /// (1 = one palette throughout; low = palette changes scene to scene).
    pub palette_stability: f32,
    /// Per key sample, aligned with `ReferenceEvidence.samples`.
    pub per_sample: Vec<SampleColor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// A single-frame discontinuity.
    Cut,
    /// A short run of strong change (wipe, dissolve, fast move) that settles.
    Transition,
    /// Slower change that accumulates into a visibly different composition.
    Evolution,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeSummary {
    pub time_seconds: f64,
    pub duration_seconds: f64,
    pub kind: ChangeKind,
    /// Peak mean absolute RGB difference, `0..=1`.
    pub magnitude: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationTendency {
    /// Changes are mostly single-frame cuts.
    Cut,
    /// Typical change under ~0.3 s.
    Short,
    /// ~0.3–0.8 s.
    Medium,
    /// Longer than ~0.8 s.
    Long,
}

/// Visual-change evidence (no audio). Evidence, not timing instructions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalEvidence {
    /// Meaningful changes in time order (at most 200).
    pub changes: Vec<ChangeSummary>,
    /// Frame-to-frame difference present when nothing moves (grain, noise);
    /// subtracted from every activity measure below.
    #[serde(default)]
    pub noise_floor: f32,
    pub cuts_per_10s: f32,
    /// All meaningful changes (cuts, transitions, evolutions) per 10 s.
    pub changes_per_10s: f32,
    /// Median time between the starts of consecutive meaningful changes (the
    /// start and end of the video bound the first and last interval).
    pub median_hold_seconds: f32,
    pub hold_p90_seconds: f32,
    /// Share of frame transitions below the static threshold.
    pub static_fraction: f32,
    /// Share of frame transitions below the low-motion threshold (includes static).
    pub low_motion_fraction: f32,
    /// Share of frame transitions above the high-motion threshold (cuts excluded).
    pub high_motion_fraction: f32,
    /// Per-transition mean absolute RGB difference (equals the luma difference
    /// for grey content), cuts excluded.
    pub activity: Distribution,
    /// Coefficient of variation (std / mean) of `activity` samples.
    pub activity_variance: f32,
    /// p95 / max(median, 0.005) of all transition diffs (as in Motion QA).
    pub transition_spike_ratio: f32,
    /// Median duration of non-cut changes, seconds (0 when there are none).
    pub median_transition_seconds: f32,
    pub transition_duration: DurationTendency,
    /// Cuts / all meaningful changes (0 when there are none).
    pub hard_cut_fraction: f32,
    /// Mean activity in the outer ring (outer 15 % on every side), cuts excluded.
    pub background_activity: f32,
    /// Mean activity in the central region, cuts excluded.
    pub foreground_activity: f32,
}

/// Lightweight static-complexity proxies (no OCR, no semantics). Computed on
/// the analysis stream (4 frames/s subsample).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComplexityEvidence {
    /// Share of pixels with a strong luma gradient.
    pub edge_density: Distribution,
    /// Share of 8×8 blocks with near-zero luma variance.
    pub flat_area_fraction: Distribution,
    /// Connected quantized-color regions covering ≥ 2 % of the frame (a count, not a ratio).
    pub large_region_count: Distribution,
    /// Normalized luma-histogram entropy (`0..=1`, 32 bins).
    pub entropy: Distribution,
    /// Share of 8×8 blocks that differ from the frame's dominant background color.
    pub occupied_ratio: Distribution,
}

/// Metric keys a profile may cite as provenance (`evidence.metrics`).
pub const EVIDENCE_METRICS: &[&str] = &[
    "metadata.duration_seconds",
    "metadata.orientation",
    "color.polarity",
    "color.dark_frame_fraction",
    "color.light_frame_fraction",
    "color.luminance",
    "color.contrast",
    "color.saturation",
    "color.temperature",
    "color.warm_cool_balance",
    "color.palette",
    "color.accent_prevalence",
    "color.palette_stability",
    "temporal.changes",
    "temporal.noise_floor",
    "temporal.cuts_per_10s",
    "temporal.changes_per_10s",
    "temporal.median_hold_seconds",
    "temporal.hold_p90_seconds",
    "temporal.static_fraction",
    "temporal.low_motion_fraction",
    "temporal.high_motion_fraction",
    "temporal.activity",
    "temporal.activity_variance",
    "temporal.transition_spike_ratio",
    "temporal.median_transition_seconds",
    "temporal.transition_duration",
    "temporal.hard_cut_fraction",
    "temporal.background_activity",
    "temporal.foreground_activity",
    "complexity.edge_density",
    "complexity.flat_area_fraction",
    "complexity.large_region_count",
    "complexity.entropy",
    "complexity.occupied_ratio",
];

/// Round to milliseconds (all serialized times).
pub fn round_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

/// FNV-1a 64 streaming hasher used for media and analysis fingerprints.
#[derive(Debug, Clone, Copy)]
pub struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Fnv64(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv64 {
    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }
    pub fn finish(&self) -> u64 {
        self.0
    }
}

/// `rf1-…`: the analysis identity of (media, analyzer version, configuration).
pub fn reference_fingerprint(media_fingerprint: &str, config_canonical: &str) -> String {
    let mut h = Fnv64::default();
    for part in [media_fingerprint, ANALYZER_VERSION, config_canonical] {
        h.write(part.as_bytes());
        h.write(&[0xff]);
    }
    format!("rf1-{:016x}", h.finish())
}
