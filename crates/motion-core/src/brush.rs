//! Deterministic `p5.brush` Tier 1 + Tier 2 geometry, vector field,
//! scanline hatching, massing, watercolor wet-bleed, and Golden Spiral engine.
//!
//! Provides variable-width pressure-tapered brush ribbons, multi-strand filaments,
//! aerosol spray droplets, Catmull-Rom spline curves, vector field streamlines,
//! even-odd scanline polygon hatching with holes, multi-pass massing,
//! iterative wet-bleed polygon deformation, and Golden Ratio / Fibonacci geometry.

use serde::{Deserialize, Serialize};

use crate::noise::{hash, hash01, smooth};
use crate::path::{BezPath, Contour, Cubic};

// ---------------------------------------------------------------------------
// Constants & Defaults
// ---------------------------------------------------------------------------

pub const PHI: f32 = 1.618_034;
pub const INV_PHI: f32 = 0.618_034;
const KAPPA: f32 = 0.552_284_8;

fn default_one() -> f32 {
    1.0
}

fn default_noise() -> f32 {
    0.18
}

fn default_precision() -> f32 {
    1.0
}

fn default_strength() -> f32 {
    1.0
}

fn default_bleed() -> f32 {
    0.35
}

fn default_half() -> f32 {
    0.5
}

fn default_border() -> f32 {
    0.4
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Math & Vector Helpers
// ---------------------------------------------------------------------------

#[inline]
fn lerp2(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

#[inline]
fn dist2(a: [f32; 2], b: [f32; 2]) -> f32 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}

#[inline]
fn make_line(from: [f32; 2], to: [f32; 2]) -> Cubic {
    Cubic {
        c1: [
            from[0] + (to[0] - from[0]) / 3.0,
            from[1] + (to[1] - from[1]) / 3.0,
        ],
        c2: [
            from[0] + (to[0] - from[0]) * (2.0 / 3.0),
            from[1] + (to[1] - from[1]) * (2.0 / 3.0),
        ],
        to,
    }
}

/// Deterministic standard normal sample N(0, 1) using Box-Muller transform.
fn deterministic_gaussian(seed: u32, idx: u32) -> f32 {
    let u1 = (hash01(seed, idx * 2) as f32).clamp(1e-6, 1.0);
    let u2 = (hash01(seed, idx * 2 + 1) as f32).clamp(0.0, 1.0);
    (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
}

/// Converts a slice of 2D points into a C1 Catmull-Rom cubic Bezier contour.
fn points_to_cubic_contour(pts: &[[f32; 2]], closed: bool, curvature: f32) -> Contour {
    if pts.is_empty() {
        return Contour {
            start: [0.0, 0.0],
            segs: Vec::new(),
            closed,
        };
    }
    if pts.len() == 1 {
        return Contour {
            start: pts[0],
            segs: Vec::new(),
            closed,
        };
    }
    let n = pts.len();
    let tension = curvature.clamp(0.0, 1.0) * 0.5;
    let mut segs = Vec::with_capacity(if closed { n } else { n - 1 });

    if closed {
        for i in 0..n {
            let p_prev = pts[(i + n - 1) % n];
            let p_curr = pts[i];
            let p_next = pts[(i + 1) % n];
            let p_next2 = pts[(i + 2) % n];

            let v_curr = [
                (p_next[0] - p_prev[0]) * tension,
                (p_next[1] - p_prev[1]) * tension,
            ];
            let v_next = [
                (p_next2[0] - p_curr[0]) * tension,
                (p_next2[1] - p_curr[1]) * tension,
            ];

            let line_c1 = lerp2(p_curr, p_next, 1.0 / 3.0);
            let line_c2 = lerp2(p_curr, p_next, 2.0 / 3.0);
            let cat_c1 = [
                p_curr[0] + v_curr[0] * (2.0 / 3.0),
                p_curr[1] + v_curr[1] * (2.0 / 3.0),
            ];
            let cat_c2 = [
                p_next[0] - v_next[0] * (2.0 / 3.0),
                p_next[1] - v_next[1] * (2.0 / 3.0),
            ];

            let c1 = lerp2(line_c1, cat_c1, curvature.clamp(0.0, 1.0));
            let c2 = lerp2(line_c2, cat_c2, curvature.clamp(0.0, 1.0));

            segs.push(Cubic { c1, c2, to: p_next });
        }
    } else {
        for i in 0..n - 1 {
            let p_prev = if i == 0 { pts[0] } else { pts[i - 1] };
            let p_curr = pts[i];
            let p_next = pts[i + 1];
            let p_next2 = if i + 2 < n { pts[i + 2] } else { pts[n - 1] };

            let v_curr = if i == 0 {
                [
                    (p_next[0] - p_curr[0]) * tension,
                    (p_next[1] - p_curr[1]) * tension,
                ]
            } else {
                [
                    (p_next[0] - p_prev[0]) * tension,
                    (p_next[1] - p_prev[1]) * tension,
                ]
            };
            let v_next = if i + 1 == n - 1 {
                [
                    (p_next[0] - p_curr[0]) * tension,
                    (p_next[1] - p_curr[1]) * tension,
                ]
            } else {
                [
                    (p_next2[0] - p_curr[0]) * tension,
                    (p_next2[1] - p_curr[1]) * tension,
                ]
            };

            let line_c1 = lerp2(p_curr, p_next, 1.0 / 3.0);
            let line_c2 = lerp2(p_curr, p_next, 2.0 / 3.0);
            let cat_c1 = [
                p_curr[0] + v_curr[0] * (2.0 / 3.0),
                p_curr[1] + v_curr[1] * (2.0 / 3.0),
            ];
            let cat_c2 = [
                p_next[0] - v_next[0] * (2.0 / 3.0),
                p_next[1] - v_next[1] * (2.0 / 3.0),
            ];

            let c1 = lerp2(line_c1, cat_c1, curvature.clamp(0.0, 1.0));
            let c2 = lerp2(line_c2, cat_c2, curvature.clamp(0.0, 1.0));

            segs.push(Cubic { c1, c2, to: p_next });
        }
    }

    Contour {
        start: pts[0],
        segs,
        closed,
    }
}

// ---------------------------------------------------------------------------
// 1. Types & Enums
// ---------------------------------------------------------------------------

/// 11 built-in brush presets matching `p5.brush`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrushPreset {
    #[default]
    Hb,
    #[serde(rename = "2b", alias = "two_b")]
    TwoB,
    #[serde(rename = "2h", alias = "two_h")]
    TwoH,
    Cpencil,
    Pen,
    Rotring,
    Spray,
    Marker,
    Marker2,
    Charcoal,
    HatchBrush,
}

/// Tuned default parameters for a brush preset.
#[derive(Debug, Clone, PartialEq)]
pub struct BrushParams {
    pub weight_mul: f32,
    pub scatter: f32,
    pub sharpness: f32,
    pub grain: f32,
    pub opacity: f32,
    pub spacing: f32,
    pub pressure_curve: PressureCurve,
    pub marker_tip: bool,
}

impl BrushPreset {
    /// Returns tuned defaults matching `p5.brush`'s 11 built-in presets.
    pub fn default_params(self) -> BrushParams {
        match self {
            Self::Hb => BrushParams {
                weight_mul: 1.0,
                scatter: 0.8,
                sharpness: 0.7,
                grain: 0.45,
                opacity: 0.75,
                spacing: 1.2,
                pressure_curve: PressureCurve::ThreePoint([0.6, 1.0, 0.7]),
                marker_tip: false,
            },
            Self::TwoB => BrushParams {
                weight_mul: 1.3,
                scatter: 1.2,
                sharpness: 0.55,
                grain: 0.65,
                opacity: 0.85,
                spacing: 1.0,
                pressure_curve: PressureCurve::ThreePoint([0.7, 1.1, 0.8]),
                marker_tip: false,
            },
            Self::TwoH => BrushParams {
                weight_mul: 0.7,
                scatter: 0.4,
                sharpness: 0.9,
                grain: 0.25,
                opacity: 0.55,
                spacing: 1.0,
                pressure_curve: PressureCurve::Linear([0.8, 0.9]),
                marker_tip: false,
            },
            Self::Cpencil => BrushParams {
                weight_mul: 1.1,
                scatter: 1.0,
                sharpness: 0.6,
                grain: 0.5,
                opacity: 0.7,
                spacing: 1.1,
                pressure_curve: PressureCurve::ThreePoint([0.5, 1.0, 0.6]),
                marker_tip: false,
            },
            Self::Pen => BrushParams {
                weight_mul: 0.9,
                scatter: 0.1,
                sharpness: 0.95,
                grain: 0.05,
                opacity: 0.95,
                spacing: 0.8,
                pressure_curve: PressureCurve::ThreePoint([0.7, 1.0, 0.85]),
                marker_tip: false,
            },
            Self::Rotring => BrushParams {
                weight_mul: 0.8,
                scatter: 0.0,
                sharpness: 1.0,
                grain: 0.0,
                opacity: 1.0,
                spacing: 0.6,
                pressure_curve: PressureCurve::Uniform,
                marker_tip: false,
            },
            Self::Spray => BrushParams {
                weight_mul: 2.5,
                scatter: 4.5,
                sharpness: 0.2,
                grain: 0.9,
                opacity: 0.35,
                spacing: 2.0,
                pressure_curve: PressureCurve::Uniform,
                marker_tip: false,
            },
            Self::Marker => BrushParams {
                weight_mul: 2.2,
                scatter: 0.2,
                sharpness: 0.8,
                grain: 0.1,
                opacity: 0.68,
                spacing: 1.5,
                pressure_curve: PressureCurve::Uniform,
                marker_tip: true,
            },
            Self::Marker2 => BrushParams {
                weight_mul: 1.8,
                scatter: 0.3,
                sharpness: 0.75,
                grain: 0.15,
                opacity: 0.72,
                spacing: 1.2,
                pressure_curve: PressureCurve::ThreePoint([0.8, 1.0, 0.9]),
                marker_tip: true,
            },
            Self::Charcoal => BrushParams {
                weight_mul: 1.8,
                scatter: 2.2,
                sharpness: 0.4,
                grain: 0.85,
                opacity: 0.65,
                spacing: 1.4,
                pressure_curve: PressureCurve::ThreePoint([0.6, 1.1, 0.5]),
                marker_tip: false,
            },
            Self::HatchBrush => BrushParams {
                weight_mul: 0.75,
                scatter: 0.05,
                sharpness: 0.92,
                grain: 0.08,
                opacity: 0.92,
                spacing: 0.8,
                pressure_curve: PressureCurve::Uniform,
                marker_tip: false,
            },
        }
    }
}

/// Pressure envelope along a stroke path.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureCurve {
    #[default]
    Uniform,
    Linear([f32; 2]),
    ThreePoint([f32; 3]),
    Gaussian {
        curve: [f32; 2],
        min_max: [f32; 2],
    },
}

impl PressureCurve {
    /// Evaluates the pressure multiplier for `t in 0.0..=1.0`, including
    /// subtle deterministic seeded organic variation.
    pub fn eval(&self, t: f32, seed: u32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let base = match self {
            Self::Uniform => 1.0,
            Self::Linear([start, end]) => start + (end - start) * t,
            Self::ThreePoint([start, mid, end]) => {
                if t <= 0.5 {
                    let u = t * 2.0;
                    start + (mid - start) * u
                } else {
                    let u = (t - 0.5) * 2.0;
                    mid + (end - mid) * u
                }
            }
            Self::Gaussian { curve, min_max } => {
                let mean = curve[0];
                let sigma = curve[1].abs().max(1e-4);
                let g = (-0.5 * ((t - mean) / sigma).powi(2)).exp();
                min_max[0] + (min_max[1] - min_max[0]) * g
            }
        };
        let wobble = (smooth(seed, t as f64 * 4.0) * 0.06) as f32;
        (base + wobble).max(0.01)
    }
}

/// Tip rotation behavior along a stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TipRotate {
    #[default]
    Natural,
    None,
    Random,
}

/// Vector field presets for directional path warping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VectorFieldPreset {
    #[default]
    Hand,
    Curved,
    Zigzag,
    Waves,
    Seabed,
    Spiral,
    Columns,
}

impl VectorFieldPreset {
    /// Returns the field heading angle in radians at normalized canvas coordinates `(u, v) in 0.0..=1.0`.
    pub fn angle_rad(self, u: f32, v: f32, time: f32, seed: u32) -> f32 {
        let u_clamped = u.clamp(0.0, 1.0);
        let v_clamped = v.clamp(0.0, 1.0);
        match self {
            Self::Hand => {
                let n1 = smooth(seed, u_clamped as f64 * 2.5 + time as f64 * 0.2) as f32;
                let n2 = smooth(
                    seed.wrapping_add(101),
                    v_clamped as f64 * 2.5 + time as f64 * 0.2,
                ) as f32;
                n1 * 0.4 + n2 * 0.2
            }
            Self::Curved => {
                let dx = u_clamped - 0.5;
                let dy = v_clamped - 0.5;
                dy.atan2(dx) + std::f32::consts::FRAC_PI_2
            }
            Self::Zigzag => {
                let phase = (v_clamped * 6.0 * std::f32::consts::PI).sin();
                if phase >= 0.0 {
                    0.45 * std::f32::consts::PI
                } else {
                    -0.45 * std::f32::consts::PI
                }
            }
            Self::Waves => {
                let wave = (u_clamped * 4.0 * std::f32::consts::PI + time).sin();
                wave * 0.65
            }
            Self::Seabed => {
                -std::f32::consts::FRAC_PI_2
                    + (v_clamped * 5.0 + (u_clamped * 3.0 + time).sin()).sin() * 0.5
            }
            Self::Spiral => {
                let dx = u_clamped - 0.5;
                let dy = v_clamped - 0.5;
                dy.atan2(dx) + 0.75
            }
            Self::Columns => {
                let col = (u_clamped * 8.0 * std::f32::consts::PI).cos() * 0.25;
                std::f32::consts::FRAC_PI_2 + col
            }
        }
    }
}

/// Complete brush specification for stroke compilation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BrushSpec {
    #[serde(default)]
    pub preset: BrushPreset,
    /// Multiplier on the stroke width (default 1.0).
    #[serde(default = "default_one")]
    pub weight: f32,
    /// Lateral scatter / bristle spread in pixels (None = preset default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scatter: Option<f32>,
    /// Edge sharpness 0.0..=1.0 (None = preset default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sharpness: Option<f32>,
    /// Grain density 0.0..=1.0 (None = preset default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grain: Option<f32>,
    /// Per-mark opacity multiplier 0.0..=1.0 (None = preset default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// Stamp spacing along path (None = preset default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f32>,
    /// Pressure envelope along the path (None = preset default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<PressureCurve>,
    /// Hand-drawn wiggle intensity (0.0 = off, 1.0..=10.0 = p5.brush wiggle).
    #[serde(default)]
    pub wiggle: f32,
    /// Optional vector field that bends the stroke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<VectorFieldPreset>,
    /// Optional custom image asset id used as a stamp tip (`type: "image"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip_asset: Option<String>,
    #[serde(default)]
    pub rotate: TipRotate,
    /// Extra soft ink buildup at start/end of marker/custom strokes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker_tip: Option<bool>,
    /// Per-stroke opacity noise variation (0.0..=1.0, default 0.18).
    #[serde(default = "default_noise")]
    pub noise: f32,
    #[serde(default)]
    pub seed: u32,
}

impl Default for BrushSpec {
    fn default() -> Self {
        Self {
            preset: BrushPreset::default(),
            weight: 1.0,
            scatter: None,
            sharpness: None,
            grain: None,
            opacity: None,
            spacing: None,
            pressure: None,
            wiggle: 0.0,
            field: None,
            tip_asset: None,
            rotate: TipRotate::default(),
            marker_tip: None,
            noise: 0.18,
            seed: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Stroke Compilation Data Structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct BrushRibbon {
    pub path: BezPath,
    pub opacity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrushStamp {
    pub pos: [f32; 2],
    pub radius: f32,
    pub angle_rad: f32,
    pub opacity: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledStroke {
    pub ribbons: Vec<BrushRibbon>,
    pub stamps: Vec<BrushStamp>,
}

impl CompiledStroke {
    pub fn is_empty(&self) -> bool {
        self.ribbons.is_empty() && self.stamps.is_empty()
    }
}

// ---------------------------------------------------------------------------
// 3. Path Sampling & Tangents
// ---------------------------------------------------------------------------

/// Samples a contour at approximately `step_px` increments, returning
/// `(pos, unit_tangent, normalized_t)` along the contour.
pub fn sample_contour(c: &Contour, step_px: f32) -> Vec<([f32; 2], [f32; 2], f32)> {
    let total_len = c.length();
    if total_len <= 1e-4 || c.segs.is_empty() {
        return vec![(c.start, [1.0, 0.0], 0.0)];
    }
    let step = step_px.max(0.5);
    let n_steps = (total_len / step).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(n_steps + 1);

    let mut seg_info = Vec::with_capacity(c.segs.len());
    let mut p_prev = c.start;
    let mut cum_len = 0.0f32;
    for seg in &c.segs {
        let l = seg.length(p_prev);
        seg_info.push((p_prev, *seg, cum_len, l));
        cum_len += l;
        p_prev = seg.to;
    }

    let param_at_seg_len = |s: &Cubic, p0: [f32; 2], target: f32| -> f32 {
        let mut acc = 0.0;
        let mut prev = p0;
        for i in 1..=64 {
            let t = i as f32 / 64.0;
            let p = s.at(p0, t);
            let l = dist2(prev, p);
            if acc + l >= target {
                let u = if l > 0.0 { (target - acc) / l } else { 0.0 };
                return (t - 1.0 / 64.0) + u / 64.0;
            }
            acc += l;
            prev = p;
        }
        1.0
    };

    let eval_at_dist = |dist: f32| -> ([f32; 2], [f32; 2], f32) {
        let target_dist = dist.clamp(0.0, total_len);
        let norm_t = if total_len > 0.0 {
            target_dist / total_len
        } else {
            0.0
        };

        for (i, &(p0, seg, seg_start_dist, seg_len)) in seg_info.iter().enumerate() {
            let is_last = i + 1 == seg_info.len();
            if target_dist <= seg_start_dist + seg_len || is_last {
                let offset = (target_dist - seg_start_dist).clamp(0.0, seg_len);
                let u = if seg_len > 1e-5 {
                    param_at_seg_len(&seg, p0, offset).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let pos = seg.at(p0, u);

                let dt = 1.0 - u;
                let dpos = [
                    3.0 * dt * dt * (seg.c1[0] - p0[0])
                        + 6.0 * dt * u * (seg.c2[0] - seg.c1[0])
                        + 3.0 * u * u * (seg.to[0] - seg.c2[0]),
                    3.0 * dt * dt * (seg.c1[1] - p0[1])
                        + 6.0 * dt * u * (seg.c2[1] - seg.c1[1])
                        + 3.0 * u * u * (seg.to[1] - seg.c2[1]),
                ];
                let dlen = dpos[0].hypot(dpos[1]);
                let tangent = if dlen > 1e-5 {
                    [dpos[0] / dlen, dpos[1] / dlen]
                } else {
                    let fallback = [seg.to[0] - p0[0], seg.to[1] - p0[1]];
                    let flen = fallback[0].hypot(fallback[1]);
                    if flen > 1e-5 {
                        [fallback[0] / flen, fallback[1] / flen]
                    } else {
                        [1.0, 0.0]
                    }
                };
                return (pos, tangent, norm_t);
            }
        }
        (p_prev, [1.0, 0.0], 1.0)
    };

    for i in 0..=n_steps {
        let target_dist = total_len * (i as f32 / n_steps as f32);
        out.push(eval_at_dist(target_dist));
    }

    out
}

// ---------------------------------------------------------------------------
// 4. Vector Field & Hand Wiggle Path Warping
// ---------------------------------------------------------------------------

/// Warps a BezPath by displacing sampled points along contour normals with smooth
/// noise and/or integrating along a vector field, returning a smooth BezPath.
pub fn warp_path(
    path: &BezPath,
    field: Option<VectorFieldPreset>,
    wiggle: f32,
    canvas_size: [f32; 2],
    seed: u32,
) -> BezPath {
    if field.is_none() && wiggle <= 0.0 {
        return path.clone();
    }

    let mut out_contours = Vec::with_capacity(path.contours.len());
    for (c_idx, c) in path.contours.iter().enumerate() {
        if c.segs.is_empty() {
            out_contours.push(c.clone());
            continue;
        }

        let step_px = (c.length() / 24.0).clamp(4.0, 16.0);
        let samples = sample_contour(c, step_px);
        if samples.len() < 2 {
            out_contours.push(c.clone());
            continue;
        }

        let c_seed = hash(seed, (c_idx as u32) * 1013);
        let mut warped_pts = Vec::with_capacity(samples.len());

        for (pos, tangent, t) in samples {
            let normal = [-tangent[1], tangent[0]];
            let mut disp_x = 0.0f32;
            let mut disp_y = 0.0f32;

            if wiggle > 0.0 {
                let n = smooth(
                    c_seed.wrapping_add(19),
                    t as f64 * (4.0 + wiggle as f64 * 0.5),
                ) as f32;
                let mag = n * wiggle * 2.0;
                disp_x += normal[0] * mag;
                disp_y += normal[1] * mag;
            }

            if let Some(preset) = field {
                let u = (pos[0] / canvas_size[0].max(1.0)).clamp(0.0, 1.0);
                let v = (pos[1] / canvas_size[1].max(1.0)).clamp(0.0, 1.0);
                let angle = preset.angle_rad(u, v, 0.0, c_seed);
                let f_mag = (wiggle.max(1.0) * 3.5).min(20.0);
                disp_x += angle.cos() * f_mag;
                disp_y += angle.sin() * f_mag;
            }

            warped_pts.push([pos[0] + disp_x, pos[1] + disp_y]);
        }

        out_contours.push(points_to_cubic_contour(&warped_pts, c.closed, 0.85));
    }

    BezPath {
        contours: out_contours,
    }
}

/// Traces a streamline from `start` of total arc length `length` in steps of ~8px,
/// bending the heading angle `dir_deg` by `field.angle_rad(u, v, 0.0, seed)` + `wiggle`,
/// returning a smooth `BezPath`.
pub fn flow_line(
    start: [f32; 2],
    length: f32,
    dir_deg: f32,
    field: VectorFieldPreset,
    wiggle: f32,
    canvas_size: [f32; 2],
    seed: u32,
) -> BezPath {
    let step_px = 8.0f32;
    let n_steps = (length / step_px).ceil().max(2.0) as usize;
    let dt_step = length / n_steps as f32;
    let mut pts = Vec::with_capacity(n_steps + 1);
    let mut pos = start;
    pts.push(pos);

    let base_rad = dir_deg.to_radians();

    for step in 0..n_steps {
        let t = step as f32 / n_steps as f32;
        let u = (pos[0] / canvas_size[0].max(1.0)).clamp(0.0, 1.0);
        let v = (pos[1] / canvas_size[1].max(1.0)).clamp(0.0, 1.0);
        let field_ang = field.angle_rad(u, v, 0.0, seed);
        let w = if wiggle > 0.0 {
            smooth(seed.wrapping_add(77), t as f64 * 5.0) as f32 * wiggle * 0.25
        } else {
            0.0
        };
        let heading = base_rad + field_ang + w;
        pos[0] =
            (pos[0] + heading.cos() * dt_step).clamp(canvas_size[0] * 0.04, canvas_size[0] * 0.96);
        pos[1] =
            (pos[1] + heading.sin() * dt_step).clamp(canvas_size[1] * 0.06, canvas_size[1] * 0.94);
        pts.push(pos);
    }

    let contour = points_to_cubic_contour(&pts, false, 0.85);
    BezPath {
        contours: vec![contour],
    }
}

/// Takes `[x, y, pressure]` points and `curvature in 0.0..=1.0` (Catmull-Rom cubic
/// conversion) and returns the smooth `BezPath` plus extracted `PressureCurve`.
pub fn spline(points: &[[f32; 3]], curvature: f32) -> (BezPath, PressureCurve) {
    if points.is_empty() {
        return (BezPath::default(), PressureCurve::Uniform);
    }
    let pts2: Vec<[f32; 2]> = points.iter().map(|p| [p[0], p[1]]).collect();
    let curve = match points.len() {
        1 => PressureCurve::Uniform,
        2 => PressureCurve::Linear([points[0][2], points[1][2]]),
        _ => {
            let s = points[0][2];
            let m = points[points.len() / 2][2];
            let e = points[points.len() - 1][2];
            PressureCurve::ThreePoint([s, m, e])
        }
    };
    let contour = points_to_cubic_contour(&pts2, false, curvature.clamp(0.0, 1.0));
    (
        BezPath {
            contours: vec![contour],
        },
        curve,
    )
}

// ---------------------------------------------------------------------------
// 5. Stroke Compiler (`compile_stroke`)
// ---------------------------------------------------------------------------

/// Compiles a path into pressure-tapered ribbons, multi-filament bristle ribbons,
/// and deterministic grain/spray stamps matching the specified brush preset.
pub fn compile_stroke(
    path: &BezPath,
    base_width: f32,
    spec: &BrushSpec,
    canvas_size: [f32; 2],
) -> CompiledStroke {
    let effective_path = if spec.wiggle > 0.0 || spec.field.is_some() {
        warp_path(path, spec.field, spec.wiggle, canvas_size, spec.seed)
    } else {
        path.clone()
    };

    let params = spec.preset.default_params();
    let eff_weight = (spec.weight * params.weight_mul).max(0.1);
    let eff_scatter = spec.scatter.unwrap_or(params.scatter);
    let eff_sharpness = spec.sharpness.unwrap_or(params.sharpness);
    let eff_grain = spec.grain.unwrap_or(params.grain);
    let eff_opacity = spec.opacity.unwrap_or(params.opacity);
    let eff_spacing = spec.spacing.unwrap_or(params.spacing);
    let pressure = spec.pressure.as_ref().unwrap_or(&params.pressure_curve);
    let marker_tip = spec.marker_tip.unwrap_or(params.marker_tip);

    let mut compiled_ribbons = Vec::new();
    let mut compiled_stamps = Vec::new();

    for (c_idx, contour) in effective_path.contours.iter().enumerate() {
        let sample_step = (base_width * eff_weight * 0.35 * eff_spacing).clamp(2.0, 16.0);
        let samples = sample_contour(contour, sample_step);
        if samples.len() < 2 {
            continue;
        }

        let c_seed = hash(spec.seed, c_idx as u32 * 7919);

        let mut left_pts = Vec::with_capacity(samples.len());
        let mut right_pts = Vec::with_capacity(samples.len());

        for (k, (pos, tangent, t)) in samples.iter().enumerate() {
            let p_val = pressure.eval(*t, c_seed.wrapping_add(k as u32));
            let half_w = (0.5 * base_width * eff_weight * p_val).max(0.5);
            let normal = [-tangent[1], tangent[0]];

            left_pts.push([pos[0] + normal[0] * half_w, pos[1] + normal[1] * half_w]);
            right_pts.push([pos[0] - normal[0] * half_w, pos[1] - normal[1] * half_w]);
        }

        let ribbon_curvature = (0.5 + 0.35 * eff_sharpness).clamp(0.2, 0.95);
        let main_ribbon_contour = if contour.closed {
            let outer = points_to_cubic_contour(&left_pts, true, ribbon_curvature);
            let mut rev_right = right_pts.clone();
            rev_right.reverse();
            let inner = points_to_cubic_contour(&rev_right, true, ribbon_curvature);
            BezPath {
                contours: vec![outer, inner],
            }
        } else {
            let mut ribbon_pts = Vec::with_capacity(left_pts.len() + right_pts.len() + 2);
            ribbon_pts.extend_from_slice(&left_pts);

            let end_sample = samples.last().unwrap();
            let end_p_val = pressure.eval(1.0, c_seed.wrapping_add(samples.len() as u32));
            let end_half_w = (0.5 * base_width * eff_weight * end_p_val).max(0.5);
            let end_cap_apex = [
                end_sample.0[0] + end_sample.1[0] * end_half_w,
                end_sample.0[1] + end_sample.1[1] * end_half_w,
            ];
            ribbon_pts.push(end_cap_apex);

            for p in right_pts.iter().rev() {
                ribbon_pts.push(*p);
            }

            let start_sample = &samples[0];
            let start_p_val = pressure.eval(0.0, c_seed);
            let start_half_w = (0.5 * base_width * eff_weight * start_p_val).max(0.5);
            let start_cap_apex = [
                start_sample.0[0] - start_sample.1[0] * start_half_w,
                start_sample.0[1] - start_sample.1[1] * start_half_w,
            ];
            ribbon_pts.push(start_cap_apex);

            let ribbon_closed = points_to_cubic_contour(&ribbon_pts, true, ribbon_curvature * 0.75);
            BezPath {
                contours: vec![ribbon_closed],
            }
        };

        match spec.preset {
            BrushPreset::Spray => {
                compiled_ribbons.push(BrushRibbon {
                    path: main_ribbon_contour,
                    opacity: 0.03 * eff_opacity,
                });
            }
            BrushPreset::Rotring | BrushPreset::Pen | BrushPreset::HatchBrush => {
                compiled_ribbons.push(BrushRibbon {
                    path: main_ribbon_contour,
                    opacity: eff_opacity.min(1.0),
                });
            }
            BrushPreset::Marker | BrushPreset::Marker2 => {
                compiled_ribbons.push(BrushRibbon {
                    path: main_ribbon_contour,
                    opacity: eff_opacity * 0.75,
                });

                let chisel_offset = [base_width * eff_weight * 0.2, base_width * eff_weight * 0.2];
                let mut chisel_left = Vec::with_capacity(left_pts.len());
                let mut chisel_right = Vec::with_capacity(right_pts.len());
                for (lp, rp) in left_pts.iter().zip(&right_pts) {
                    chisel_left.push([lp[0] + chisel_offset[0], lp[1] + chisel_offset[1]]);
                    chisel_right.push([rp[0] + chisel_offset[0], rp[1] + chisel_offset[1]]);
                }
                let mut chisel_pts = Vec::with_capacity(chisel_left.len() + chisel_right.len());
                chisel_pts.extend_from_slice(&chisel_left);
                for p in chisel_right.iter().rev() {
                    chisel_pts.push(*p);
                }
                let chisel_contour = points_to_cubic_contour(&chisel_pts, true, 0.6);
                compiled_ribbons.push(BrushRibbon {
                    path: BezPath {
                        contours: vec![chisel_contour],
                    },
                    opacity: eff_opacity * 0.45,
                });

                if marker_tip {
                    let start_radius = base_width * eff_weight * 0.6;
                    let end_radius = base_width * eff_weight * 0.65;
                    compiled_stamps.push(BrushStamp {
                        pos: samples[0].0,
                        radius: start_radius,
                        angle_rad: 0.0,
                        opacity: eff_opacity * 0.85,
                    });
                    compiled_stamps.push(BrushStamp {
                        pos: samples.last().unwrap().0,
                        radius: end_radius,
                        angle_rad: 0.0,
                        opacity: eff_opacity * 0.88,
                    });
                }
            }
            BrushPreset::Charcoal => {
                compiled_ribbons.push(BrushRibbon {
                    path: main_ribbon_contour,
                    opacity: 0.45 * eff_opacity,
                });
                for strand_i in 0..4 {
                    let strand_seed = c_seed.wrapping_add((strand_i + 1) * 333);
                    let frac = (strand_i as f32 - 1.5) / 1.5;
                    let offset_dist = frac * base_width * eff_weight * 0.45 * eff_scatter;
                    let strand_hw = (base_width * eff_weight * 0.16).max(0.45);
                    let mut s_left = Vec::with_capacity(samples.len());
                    let mut s_right = Vec::with_capacity(samples.len());
                    for (k, (pos, tangent, t)) in samples.iter().enumerate() {
                        let normal = [-tangent[1], tangent[0]];
                        let p_val = pressure.eval(*t, strand_seed.wrapping_add(k as u32));
                        let hw = (strand_hw * p_val).max(0.35);
                        let jitter = (hash01(strand_seed, k as u32) as f32 - 0.5) * 1.5;
                        let center_off = offset_dist + jitter;
                        s_left.push([
                            pos[0] + normal[0] * (center_off + hw),
                            pos[1] + normal[1] * (center_off + hw),
                        ]);
                        s_right.push([
                            pos[0] + normal[0] * (center_off - hw),
                            pos[1] + normal[1] * (center_off - hw),
                        ]);
                    }
                    let mut strand_pts = Vec::with_capacity(s_left.len() + s_right.len());
                    strand_pts.extend_from_slice(&s_left);
                    for p in s_right.iter().rev() {
                        strand_pts.push(*p);
                    }
                    let strand_contour = points_to_cubic_contour(&strand_pts, true, 0.7);
                    compiled_ribbons.push(BrushRibbon {
                        path: BezPath {
                            contours: vec![strand_contour],
                        },
                        opacity: 0.28 * eff_opacity,
                    });
                }
            }
            BrushPreset::Hb | BrushPreset::TwoB | BrushPreset::TwoH | BrushPreset::Cpencil => {
                compiled_ribbons.push(BrushRibbon {
                    path: main_ribbon_contour,
                    opacity: eff_opacity * 0.65,
                });
                for strand_i in 0..2 {
                    let strand_seed = c_seed.wrapping_add((strand_i + 1) * 777);
                    let side = if strand_i == 0 { 1.0f32 } else { -1.0f32 };
                    let offset_dist = side * base_width * eff_weight * 0.22 * eff_scatter;
                    let strand_hw = (base_width * eff_weight * 0.14).max(0.35);
                    let mut s_left = Vec::with_capacity(samples.len());
                    let mut s_right = Vec::with_capacity(samples.len());
                    for (k, (pos, tangent, t)) in samples.iter().enumerate() {
                        let normal = [-tangent[1], tangent[0]];
                        let p_val = pressure.eval(*t, strand_seed.wrapping_add(k as u32));
                        let hw = (strand_hw * p_val).max(0.3);
                        let jitter = (hash01(strand_seed, k as u32) as f32 - 0.5) * 1.0;
                        let center_off = offset_dist + jitter;
                        s_left.push([
                            pos[0] + normal[0] * (center_off + hw),
                            pos[1] + normal[1] * (center_off + hw),
                        ]);
                        s_right.push([
                            pos[0] + normal[0] * (center_off - hw),
                            pos[1] + normal[1] * (center_off - hw),
                        ]);
                    }
                    let mut strand_pts = Vec::with_capacity(s_left.len() + s_right.len());
                    strand_pts.extend_from_slice(&s_left);
                    for p in s_right.iter().rev() {
                        strand_pts.push(*p);
                    }
                    let strand_contour = points_to_cubic_contour(&strand_pts, true, 0.7);
                    compiled_ribbons.push(BrushRibbon {
                        path: BezPath {
                            contours: vec![strand_contour],
                        },
                        opacity: eff_opacity * 0.38,
                    });
                }
            }
        }

        let stamp_angle = |tangent: [f32; 2], k: u32| -> f32 {
            match spec.rotate {
                TipRotate::Natural => tangent[1].atan2(tangent[0]),
                TipRotate::None => 0.0,
                TipRotate::Random => {
                    (hash01(c_seed.wrapping_add(13), k) * std::f64::consts::TAU) as f32
                }
            }
        };

        match spec.preset {
            BrushPreset::Spray => {
                for (k, (pos, tangent, _t)) in samples.iter().enumerate() {
                    let k_u32 = k as u32;
                    let droplets = 16;
                    for d_i in 0..droplets {
                        let d_seed = c_seed.wrapping_add(k_u32 * 64 + d_i);
                        let g1 = deterministic_gaussian(d_seed, 0);
                        let g2 = deterministic_gaussian(d_seed, 1);
                        let r_disp = eff_scatter * base_width * 1.2;
                        let dpos = [pos[0] + g1 * r_disp, pos[1] + g2 * r_disp];
                        let radius = 0.6 + 1.6 * (hash01(d_seed, 4) as f32);
                        let opacity = (0.15 + 0.35 * (hash01(d_seed, 5) as f32)) * eff_opacity;
                        compiled_stamps.push(BrushStamp {
                            pos: dpos,
                            radius,
                            angle_rad: stamp_angle(*tangent, k_u32 * droplets + d_i),
                            opacity,
                        });
                    }
                }
            }
            BrushPreset::Charcoal => {
                for (k, (pos, tangent, _t)) in samples.iter().enumerate() {
                    let k_u32 = k as u32;
                    let normal = [-tangent[1], tangent[0]];
                    let grains = 6;
                    for g_i in 0..grains {
                        let g_seed = c_seed.wrapping_add(k_u32 * 32 + g_i);
                        let g = deterministic_gaussian(g_seed, 0);
                        let dist = g * eff_scatter * base_width * 0.55;
                        let spos = [pos[0] + normal[0] * dist, pos[1] + normal[1] * dist];
                        let radius = 0.8 + 1.4 * (hash01(g_seed, 2) as f32);
                        let opacity = eff_grain * 0.55 * (0.4 + 0.6 * hash01(g_seed, 3) as f32);
                        compiled_stamps.push(BrushStamp {
                            pos: spos,
                            radius,
                            angle_rad: stamp_angle(*tangent, k_u32 * grains + g_i),
                            opacity,
                        });
                    }
                }
            }
            BrushPreset::Hb | BrushPreset::TwoB | BrushPreset::TwoH | BrushPreset::Cpencil => {
                for (k, (pos, tangent, _t)) in samples.iter().enumerate() {
                    let k_u32 = k as u32;
                    let normal = [-tangent[1], tangent[0]];
                    let count = 3;
                    for s_i in 0..count {
                        let s_seed = c_seed.wrapping_add(k_u32 * 16 + s_i);
                        let offset = (hash01(s_seed, 0) as f32 * 2.0 - 1.0)
                            * eff_scatter
                            * base_width
                            * 0.45;
                        let spos = [pos[0] + normal[0] * offset, pos[1] + normal[1] * offset];
                        let radius = 0.6 + 0.8 * (hash01(s_seed, 1) as f32);
                        let opacity = eff_grain * 0.4 * (hash01(s_seed, 2) as f32);
                        compiled_stamps.push(BrushStamp {
                            pos: spos,
                            radius,
                            angle_rad: stamp_angle(*tangent, k_u32 * count + s_i),
                            opacity,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    CompiledStroke {
        ribbons: compiled_ribbons,
        stamps: compiled_stamps,
    }
}

// ---------------------------------------------------------------------------
// 6. Scanline Polygon Hatching & Massing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HatchSpec {
    pub dist: f32,
    pub angle_deg: f32,
    #[serde(default)]
    pub rand: f32,
    #[serde(default)]
    pub continuous: bool,
    #[serde(default)]
    pub gradient: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cross_angle_deg: Option<f32>,
    #[serde(default)]
    pub wiggle: f32,
    #[serde(default)]
    pub seed: u32,
}

impl Default for HatchSpec {
    fn default() -> Self {
        Self {
            dist: 6.0,
            angle_deg: 45.0,
            rand: 0.0,
            continuous: false,
            gradient: 0.0,
            cross_angle_deg: None,
            wiggle: 0.0,
            seed: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MassSpec {
    #[serde(default = "default_precision")]
    pub precision: f32,
    #[serde(default = "default_strength")]
    pub strength: f32,
    #[serde(default)]
    pub gradient: f32,
    #[serde(default)]
    pub outline: bool,
    #[serde(default)]
    pub seed: u32,
}

impl Default for MassSpec {
    fn default() -> Self {
        Self {
            precision: 1.0,
            strength: 1.0,
            gradient: 0.0,
            outline: false,
            seed: 0,
        }
    }
}

/// Computes scanline hatch lines inside `boundary`'s contours respecting inner holes (even-odd rule).
pub fn hatch_path(boundary: &BezPath, spec: &HatchSpec) -> BezPath {
    if boundary.contours.is_empty() {
        return BezPath::default();
    }

    let mut edges = Vec::new();
    for c in &boundary.contours {
        if c.segs.is_empty() {
            continue;
        }
        let mut prev = c.start;
        for s in &c.segs {
            for step in 1..=12 {
                let p = s.at(prev, step as f32 / 12.0);
                if dist2(prev, p) > 1e-4 {
                    edges.push((prev, p));
                }
                prev = p;
            }
        }
        if c.closed && dist2(prev, c.start) > 1e-4 {
            edges.push((prev, c.start));
        }
    }

    if edges.is_empty() {
        return BezPath::default();
    }

    let mut all_contours = hatch_single_angle(&edges, spec.angle_deg, spec, spec.seed);
    if let Some(cross_angle) = spec.cross_angle_deg {
        let cross_contours =
            hatch_single_angle(&edges, cross_angle, spec, spec.seed.wrapping_add(5555));
        all_contours.extend(cross_contours);
    }

    let result = BezPath {
        contours: all_contours,
    };
    if spec.wiggle > 0.0 {
        warp_path(&result, None, spec.wiggle, [1000.0, 1000.0], spec.seed)
    } else {
        result
    }
}

fn hatch_single_angle(
    edges: &[([f32; 2], [f32; 2])],
    angle_deg: f32,
    spec: &HatchSpec,
    seed: u32,
) -> Vec<Contour> {
    let theta = angle_deg.to_radians();
    let cos_neg = (-theta).cos();
    let sin_neg = (-theta).sin();
    let cos_pos = theta.cos();
    let sin_pos = theta.sin();

    let rot_pt =
        |p: [f32; 2], c: f32, s: f32| -> [f32; 2] { [p[0] * c - p[1] * s, p[0] * s + p[1] * c] };

    let rot_edges: Vec<([f32; 2], [f32; 2])> = edges
        .iter()
        .map(|&(p0, p1)| (rot_pt(p0, cos_neg, sin_neg), rot_pt(p1, cos_neg, sin_neg)))
        .collect();

    let mut y_min = f32::INFINITY;
    let mut y_max = f32::NEG_INFINITY;
    for &(p0, p1) in &rot_edges {
        y_min = y_min.min(p0[1]).min(p1[1]);
        y_max = y_max.max(p0[1]).max(p1[1]);
    }

    let height = y_max - y_min;
    if height <= 1e-3 {
        return Vec::new();
    }

    let base_d = spec.dist.max(0.5);
    let mut y_k = y_min + base_d * 0.5;
    let mut line_idx = 0usize;
    let mut raw_lines: Vec<Vec<([f32; 2], [f32; 2])>> = Vec::new();

    while y_k < y_max {
        let progress = ((y_k - y_min) / height).clamp(0.0, 1.0);
        let cur_d = (base_d * (1.0 + spec.gradient * progress)).max(0.5);

        let jitter = if spec.rand > 0.0 {
            (hash01(seed, line_idx as u32 * 3) as f32 - 0.5) * spec.rand * cur_d * 0.4
        } else {
            0.0
        };
        let eval_y = (y_k + jitter).clamp(y_min + 1e-3, y_max - 1e-3);

        let mut x_inters = Vec::new();
        for &(p0, p1) in &rot_edges {
            let (y0, y1) = (p0[1], p1[1]);
            if (y0 <= eval_y && eval_y < y1) || (y1 <= eval_y && eval_y < y0) {
                let dy = y1 - y0;
                if dy.abs() > 1e-5 {
                    let t = (eval_y - y0) / dy;
                    let x = p0[0] + t * (p1[0] - p0[0]);
                    x_inters.push(x);
                }
            }
        }

        x_inters.sort_by(|a, b| a.total_cmp(b));

        let mut segments = Vec::new();
        for chunk in x_inters.chunks_exact(2) {
            let mut x_a = chunk[0];
            let mut x_b = chunk[1];
            if spec.rand > 0.0 {
                let over_a = (hash01(
                    seed.wrapping_add(1),
                    (line_idx * 17 + segments.len() * 2) as u32,
                ) as f32
                    - 0.5)
                    * spec.rand
                    * base_d
                    * 0.4;
                let over_b = (hash01(
                    seed.wrapping_add(2),
                    (line_idx * 17 + segments.len() * 2 + 1) as u32,
                ) as f32
                    - 0.5)
                    * spec.rand
                    * base_d
                    * 0.4;
                x_a += over_a;
                x_b += over_b;
            }
            segments.push(([x_a, eval_y], [x_b, eval_y]));
        }

        if line_idx % 2 == 1 {
            segments.reverse();
            for seg in &mut segments {
                *seg = (seg.1, seg.0);
            }
        }

        if !segments.is_empty() {
            raw_lines.push(segments);
        }

        y_k += cur_d;
        line_idx += 1;
    }

    let mut out_contours = Vec::new();
    if spec.continuous {
        let mut cur_segs = Vec::new();
        let mut cur_start: Option<[f32; 2]> = None;
        let mut prev_end: Option<[f32; 2]> = None;

        for line in raw_lines {
            for (p_start, p_end) in line {
                let p_start_rot = rot_pt(p_start, cos_pos, sin_pos);
                let p_end_rot = rot_pt(p_end, cos_pos, sin_pos);

                if let Some(prev) = prev_end {
                    let d = dist2(prev, p_start_rot);
                    if d > base_d * 4.0 {
                        if let Some(start) = cur_start {
                            out_contours.push(Contour {
                                start,
                                segs: cur_segs,
                                closed: false,
                            });
                            cur_segs = Vec::new();
                        }
                        cur_start = Some(p_start_rot);
                    } else {
                        cur_segs.push(make_line(prev, p_start_rot));
                    }
                } else {
                    cur_start = Some(p_start_rot);
                }

                cur_segs.push(make_line(p_start_rot, p_end_rot));
                prev_end = Some(p_end_rot);
            }
        }

        if let Some(start) = cur_start {
            if !cur_segs.is_empty() {
                out_contours.push(Contour {
                    start,
                    segs: cur_segs,
                    closed: false,
                });
            }
        }
    } else {
        for line in raw_lines {
            for (p_start, p_end) in line {
                let start = rot_pt(p_start, cos_pos, sin_pos);
                let end = rot_pt(p_end, cos_pos, sin_pos);
                out_contours.push(Contour {
                    start,
                    segs: vec![make_line(start, end)],
                    closed: false,
                });
            }
        }
    }

    out_contours
}

/// Builds 1 to 3 layered hatch passes at shifted angles controlled by `spec.strength`.
pub fn mass_path(boundary: &BezPath, spec: &MassSpec) -> BezPath {
    if boundary.contours.is_empty() {
        return BezPath::default();
    }

    let angles: &[f32] = if spec.strength <= 0.35 {
        &[25.0]
    } else if spec.strength <= 0.70 {
        &[25.0, -65.0]
    } else {
        &[25.0, -65.0, 110.0]
    };

    let dist = (8.0 / spec.precision.max(0.2)).clamp(2.5, 35.0);
    let mut all_contours = Vec::new();

    for (i, &angle) in angles.iter().enumerate() {
        let hatch_spec = HatchSpec {
            dist,
            angle_deg: angle,
            rand: 0.18,
            continuous: true,
            gradient: spec.gradient,
            cross_angle_deg: None,
            wiggle: 0.5,
            seed: spec.seed.wrapping_add((i as u32) * 233),
        };
        let p = hatch_path(boundary, &hatch_spec);
        all_contours.extend(p.contours);
    }

    if spec.outline {
        all_contours.extend(boundary.clone().contours);
    }

    BezPath {
        contours: all_contours,
    }
}

// ---------------------------------------------------------------------------
// 7. Watercolor Wet-Bleed Polygon Generator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BleedDirection {
    #[default]
    Out,
    In,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatercolorSpec {
    /// Edge diffusion strength 0.0..=1.0 (default 0.35).
    #[serde(default = "default_bleed")]
    pub bleed: f32,
    #[serde(default)]
    pub direction: BleedDirection,
    /// Optional directional wash angle in degrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle_deg: Option<f32>,
    /// Paper texture contrast 0.0..=1.0 (default 0.5).
    #[serde(default = "default_half")]
    pub texture: f32,
    /// Wet border ring ("coffee ring") intensity 0.0..=1.0 (default 0.4).
    #[serde(default = "default_border")]
    pub border: f32,
    /// Include sparse scattered edge-noise polygons (default true).
    #[serde(default = "default_true")]
    pub scatter: bool,
    #[serde(default)]
    pub seed: u32,
}

impl Default for WatercolorSpec {
    fn default() -> Self {
        Self {
            bleed: 0.35,
            direction: BleedDirection::Out,
            angle_deg: None,
            texture: 0.5,
            border: 0.4,
            scatter: true,
            seed: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WatercolorLayer {
    pub path: BezPath,
    pub opacity: f32,
    pub is_rim: bool,
}

/// Generates layered watercolor wet-bleed deformed polygon layers and pigment rims.
pub fn watercolor_layers(boundary: &BezPath, spec: &WatercolorSpec) -> Vec<WatercolorLayer> {
    if boundary.contours.is_empty() {
        return Vec::new();
    }

    let c = &boundary.contours[0];
    let base_samples = sample_contour(c, (c.length() / 20.0).clamp(3.0, 30.0));
    let base_pts: Vec<[f32; 2]> = base_samples.into_iter().map(|s| s.0).collect();
    if base_pts.len() < 3 {
        return Vec::new();
    }

    let centroid = [
        base_pts.iter().map(|p| p[0]).sum::<f32>() / base_pts.len() as f32,
        base_pts.iter().map(|p| p[1]).sum::<f32>() / base_pts.len() as f32,
    ];

    let mut layers = Vec::new();

    // 14 translucent wash layers
    for i in 0..14 {
        let layer_seed = hash(spec.seed, (i as u32) * 137 + 17);
        let opacity = 0.07 + 0.04 * (hash01(layer_seed, 0) as f32);
        let poly = displace_polygon(
            &base_pts,
            centroid,
            spec.bleed,
            spec.direction,
            spec.angle_deg,
            layer_seed,
            3,
        );
        let contour = points_to_cubic_contour(&poly, true, 0.7);
        layers.push(WatercolorLayer {
            path: BezPath {
                contours: vec![contour],
            },
            opacity,
            is_rim: false,
        });
    }

    // Border pooling rims
    if spec.border > 0.0 {
        let rim1_opacity = (0.28 * spec.border).clamp(0.05, 0.95);
        let poly1 = displace_polygon(
            &base_pts,
            centroid,
            spec.bleed * 0.7,
            spec.direction,
            spec.angle_deg,
            hash(spec.seed, 9001),
            3,
        );
        let contour1 = points_to_cubic_contour(&poly1, true, 0.7);
        layers.push(WatercolorLayer {
            path: BezPath {
                contours: vec![contour1],
            },
            opacity: rim1_opacity,
            is_rim: true,
        });

        let rim2_opacity = (0.42 * spec.border).clamp(0.08, 0.98);
        let poly2 = displace_polygon(
            &base_pts,
            centroid,
            spec.bleed * 0.85,
            spec.direction,
            spec.angle_deg,
            hash(spec.seed, 9002),
            3,
        );
        let contour2 = points_to_cubic_contour(&poly2, true, 0.7);
        layers.push(WatercolorLayer {
            path: BezPath {
                contours: vec![contour2],
            },
            opacity: rim2_opacity,
            is_rim: true,
        });
    }

    // Scatter diffusion layers
    if spec.scatter {
        for s in 0..3 {
            let scatter_seed = hash(spec.seed, (s as u32) * 311 + 77);
            let opacity = 0.05 + 0.03 * (hash01(scatter_seed, 1) as f32);
            let poly = displace_polygon(
                &base_pts,
                centroid,
                spec.bleed * 1.5,
                spec.direction,
                spec.angle_deg,
                scatter_seed,
                3,
            );
            let contour = points_to_cubic_contour(&poly, true, 0.65);
            layers.push(WatercolorLayer {
                path: BezPath {
                    contours: vec![contour],
                },
                opacity,
                is_rim: false,
            });
        }
    }

    layers
}

fn displace_polygon(
    base: &[[f32; 2]],
    centroid: [f32; 2],
    bleed: f32,
    direction: BleedDirection,
    angle_deg: Option<f32>,
    seed: u32,
    iterations: usize,
) -> Vec<[f32; 2]> {
    let mut cur = base.to_vec();
    let dir_sign = match direction {
        BleedDirection::Out => 1.0f32,
        BleedDirection::In => -1.0f32,
    };

    let wash_dir = angle_deg.map(|deg| {
        let rad = deg.to_radians();
        [rad.cos(), rad.sin()]
    });

    for iter in 0..iterations {
        let mut next = Vec::with_capacity(cur.len() * 2);
        let n = cur.len();
        for i in 0..n {
            let p0 = cur[i];
            let p1 = cur[(i + 1) % n];
            next.push(p0);

            let mid = [(p0[0] + p1[0]) * 0.5, (p0[1] + p1[1]) * 0.5];
            let edge = [p1[0] - p0[0], p1[1] - p0[1]];
            let edge_len = edge[0].hypot(edge[1]);

            let mut norm = [-edge[1] / edge_len.max(1e-5), edge[0] / edge_len.max(1e-5)];
            let to_mid = [mid[0] - centroid[0], mid[1] - centroid[1]];
            if norm[0] * to_mid[0] + norm[1] * to_mid[1] < 0.0 {
                norm = [-norm[0], -norm[1]];
            }

            let dir_weight = if let Some(wdir) = wash_dir {
                let dot = norm[0] * wdir[0] + norm[1] * wdir[1];
                (1.0 + 0.65 * dot).max(0.15)
            } else {
                1.0
            };

            let g_noise = deterministic_gaussian(seed.wrapping_add((iter as u32) * 73), i as u32);
            let disp =
                dir_sign * bleed * edge_len * 0.35 * (0.75 + 0.5 * g_noise.abs()) * dir_weight;
            let mid_disp = [mid[0] + norm[0] * disp, mid[1] + norm[1] * disp];
            next.push(mid_disp);
        }
        cur = next;
    }

    cur
}

// ---------------------------------------------------------------------------
// 8. Golden Ratio & Fibonacci Spiral Geometry (`golden_spiral`)
// ---------------------------------------------------------------------------

/// Subdivides `rect = [x, y, w, h]` into `steps` nested Golden Squares rotating
/// clockwise (Left -> Top -> Right -> Bottom for horizontal; Top -> Right ->
/// Bottom -> Left for vertical/portrait) and returns
/// `(chambers_xywh, squares_grid_path, logarithmic_spiral_arc_path, focal_eye_xy)`.
pub fn golden_spiral_chambers(
    rect: [f32; 4],
    steps: usize,
) -> (Vec<[f32; 4]>, BezPath, BezPath, [f32; 2]) {
    let [orig_x, orig_y, orig_w, orig_h] = rect;
    if orig_w <= 0.0 || orig_h <= 0.0 {
        return (
            Vec::new(),
            BezPath::default(),
            BezPath::default(),
            [orig_x, orig_y],
        );
    }

    // Fit an exact 1:PHI (or PHI:1) Golden Rectangle inside `rect` so every
    // recursive square cut leaves a true Golden Rectangle remainder and quarter-circle
    // arcs join with C1 continuity in both 16:9 landscape and 9:16 portrait.
    let (mut rx, mut ry, mut rw, mut rh, dir_offset) = if orig_w >= orig_h {
        let gw = orig_w.min(orig_h * PHI);
        let gh = gw * INV_PHI;
        (
            orig_x + (orig_w - gw) * 0.5,
            orig_y + (orig_h - gh) * 0.5,
            gw,
            gh,
            0usize,
        )
    } else {
        let gh = orig_h.min(orig_w * PHI);
        let gw = gh * INV_PHI;
        (
            orig_x + (orig_w - gw) * 0.5,
            orig_y + (orig_h - gh) * 0.5,
            gw,
            gh,
            1usize,
        )
    };

    let steps = steps.clamp(1, 16);
    let mut chambers = Vec::with_capacity(steps);
    let mut square_contours = Vec::with_capacity(steps);
    let mut arc_segs = Vec::with_capacity(steps);
    let mut spiral_start = [0.0f32, 0.0];

    for i in 0..steps {
        let dir = (i + dir_offset) % 4;
        let s = rw.min(rh);

        let (sq_x, sq_y, arc_center, start_ang, end_ang) = match dir {
            0 => {
                let sq_x = rx;
                let sq_y = ry;
                rx += s;
                rw = (rw - s).max(0.0);
                let center = [sq_x + s, sq_y + s];
                (
                    sq_x,
                    sq_y,
                    center,
                    std::f32::consts::PI,
                    1.5 * std::f32::consts::PI,
                )
            }
            1 => {
                let sq_x = rx;
                let sq_y = ry;
                ry += s;
                rh = (rh - s).max(0.0);
                let center = [sq_x, sq_y + s];
                (
                    sq_x,
                    sq_y,
                    center,
                    1.5 * std::f32::consts::PI,
                    2.0 * std::f32::consts::PI,
                )
            }
            2 => {
                let sq_x = rx + (rw - s).max(0.0);
                let sq_y = ry;
                rw = (rw - s).max(0.0);
                let center = [sq_x, sq_y];
                (sq_x, sq_y, center, 0.0, 0.5 * std::f32::consts::PI)
            }
            _ => {
                let sq_x = rx;
                let sq_y = ry + (rh - s).max(0.0);
                rh = (rh - s).max(0.0);
                let center = [sq_x + s, sq_y];
                (
                    sq_x,
                    sq_y,
                    center,
                    0.5 * std::f32::consts::PI,
                    std::f32::consts::PI,
                )
            }
        };

        chambers.push([sq_x, sq_y, s, s]);

        let p0 = [sq_x, sq_y];
        let p1 = [sq_x + s, sq_y];
        let p2 = [sq_x + s, sq_y + s];
        let p3 = [sq_x, sq_y + s];
        square_contours.push(Contour {
            start: p0,
            segs: vec![
                make_line(p0, p1),
                make_line(p1, p2),
                make_line(p2, p3),
                make_line(p3, p0),
            ],
            closed: true,
        });

        let arc_p0 = [
            arc_center[0] + s * start_ang.cos(),
            arc_center[1] + s * start_ang.sin(),
        ];
        let arc_p1 = [
            arc_center[0] + s * end_ang.cos(),
            arc_center[1] + s * end_ang.sin(),
        ];

        if i == 0 {
            spiral_start = arc_p0;
        }

        let tan0 = [-start_ang.sin(), start_ang.cos()];
        let tan1 = [-end_ang.sin(), end_ang.cos()];
        let c1 = [
            arc_p0[0] + s * KAPPA * tan0[0],
            arc_p0[1] + s * KAPPA * tan0[1],
        ];
        let c2 = [
            arc_p1[0] - s * KAPPA * tan1[0],
            arc_p1[1] - s * KAPPA * tan1[1],
        ];

        arc_segs.push(Cubic { c1, c2, to: arc_p1 });
    }

    let focal_eye = [rx + rw * 0.5, ry + rh * 0.5];

    let squares_grid_path = BezPath {
        contours: square_contours,
    };
    let logarithmic_spiral_arc_path = BezPath {
        contours: vec![Contour {
            start: spiral_start,
            segs: arc_segs,
            closed: false,
        }],
    };

    (
        chambers,
        squares_grid_path,
        logarithmic_spiral_arc_path,
        focal_eye,
    )
}

/// Subdivides `rect = [x, y, w, h]` into `steps` nested Golden Squares and returns
/// `(squares_grid_path, logarithmic_spiral_arc_path, focal_eye_xy)`.
pub fn golden_spiral(rect: [f32; 4], steps: usize) -> (BezPath, BezPath, [f32; 2]) {
    let (_, squares, spiral, eye) = golden_spiral_chambers(rect, steps);
    (squares, spiral, eye)
}

// ---------------------------------------------------------------------------
// Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::{ellipse, polygon, rounded_rect};

    #[test]
    fn test_all_11_brush_presets_produce_valid_strokes() {
        let presets = [
            BrushPreset::Hb,
            BrushPreset::TwoB,
            BrushPreset::TwoH,
            BrushPreset::Cpencil,
            BrushPreset::Pen,
            BrushPreset::Rotring,
            BrushPreset::Spray,
            BrushPreset::Marker,
            BrushPreset::Marker2,
            BrushPreset::Charcoal,
            BrushPreset::HatchBrush,
        ];

        let stroke_path = BezPath {
            contours: vec![Contour {
                start: [50.0, 50.0],
                segs: vec![
                    make_line([50.0, 50.0], [150.0, 100.0]),
                    make_line([150.0, 100.0], [250.0, 50.0]),
                ],
                closed: false,
            }],
        };

        for preset in presets {
            let spec = BrushSpec {
                preset,
                weight: 2.0,
                seed: 42,
                ..Default::default()
            };
            let compiled = compile_stroke(&stroke_path, 10.0, &spec, [800.0, 600.0]);

            assert!(
                !compiled.is_empty(),
                "Preset {:?} produced empty compiled stroke",
                preset
            );

            for ribbon in &compiled.ribbons {
                assert!(
                    ribbon.path.is_finite(),
                    "Preset {:?} has non-finite ribbon path",
                    preset
                );
                assert!(
                    ribbon.opacity >= 0.0 && ribbon.opacity <= 1.0,
                    "Preset {:?} opacity out of bounds",
                    preset
                );
            }

            for stamp in &compiled.stamps {
                assert!(
                    stamp.pos[0].is_finite() && stamp.pos[1].is_finite(),
                    "Preset {:?} non-finite stamp pos",
                    preset
                );
                assert!(
                    stamp.radius.is_finite() && stamp.radius > 0.0,
                    "Preset {:?} invalid stamp radius",
                    preset
                );
                assert!(
                    stamp.opacity.is_finite() && stamp.opacity >= 0.0,
                    "Preset {:?} invalid stamp opacity",
                    preset
                );
            }
        }
    }

    #[test]
    fn test_hatch_path_circle_and_square_with_hole() {
        let circle = ellipse(50.0, 50.0, 100.0, 100.0);
        let hatch_spec = HatchSpec {
            dist: 8.0,
            angle_deg: 45.0,
            rand: 0.1,
            continuous: true,
            gradient: 0.2,
            cross_angle_deg: Some(-45.0),
            wiggle: 0.0,
            seed: 123,
        };

        let hatched_circle = hatch_path(&circle, &hatch_spec);
        assert!(!hatched_circle.contours.is_empty());
        assert!(hatched_circle.is_finite());

        if let Some(bounds) = hatched_circle.bounds() {
            assert!(bounds[0] >= 45.0 && bounds[2] <= 155.0);
            assert!(bounds[1] >= 45.0 && bounds[3] <= 155.0);
        }

        // Square with a hole (even-odd rule)
        let outer = rounded_rect(0.0, 0.0, 200.0, 200.0, 0.0);
        let inner = ellipse(60.0, 60.0, 80.0, 80.0);
        let mut hole_shape = outer;
        hole_shape.contours.push(inner.contours[0].clone());

        let hole_hatch_spec = HatchSpec {
            dist: 10.0,
            angle_deg: 0.0,
            rand: 0.0,
            continuous: false,
            gradient: 0.0,
            cross_angle_deg: None,
            wiggle: 0.0,
            seed: 7,
        };

        let hatched_hole = hatch_path(&hole_shape, &hole_hatch_spec);
        assert!(!hatched_hole.contours.is_empty());
        assert!(hatched_hole.is_finite());

        // Check that segments on y = 100 don't cross through hole center [100, 100]
        for c in &hatched_hole.contours {
            let start = c.start;
            let end = c.segs[0].to;
            if (start[1] - 100.0).abs() < 1.0 {
                // Must be left wall [0..60] or right wall [140..200]
                let inside_hole =
                    (start[0] > 70.0 && start[0] < 130.0) || (end[0] > 70.0 && end[0] < 130.0);
                assert!(!inside_hole, "Hatch line drawn inside hole!");
            }
        }
    }

    #[test]
    fn test_watercolor_layers_produces_14_plus_layers() {
        let boundary = rounded_rect(50.0, 50.0, 150.0, 150.0, 15.0);
        let spec = WatercolorSpec {
            bleed: 0.35,
            direction: BleedDirection::Out,
            angle_deg: Some(30.0),
            texture: 0.5,
            border: 0.4,
            scatter: true,
            seed: 99,
        };

        let layers = watercolor_layers(&boundary, &spec);
        assert!(
            layers.len() >= 14,
            "Expected at least 14 watercolor layers, got {}",
            layers.len()
        );

        let mut rim_count = 0;
        for layer in &layers {
            assert!(layer.path.is_finite());
            assert!(!layer.path.contours.is_empty());
            assert!(layer.opacity > 0.0 && layer.opacity <= 1.0);
            if layer.is_rim {
                rim_count += 1;
            }
        }
        assert_eq!(rim_count, 2, "Expected 2 pigment rim layers");
    }

    #[test]
    fn test_golden_spiral_produces_valid_squares_and_arc() {
        let rect = [100.0, 100.0, 800.0, 500.0];
        let steps = 8;
        let (squares, spiral, focal_eye) = golden_spiral(rect, steps);

        assert_eq!(squares.contours.len(), steps);
        assert_eq!(spiral.contours.len(), 1);
        assert_eq!(spiral.contours[0].segs.len(), steps);

        assert!(squares.is_finite());
        assert!(spiral.is_finite());

        // Focal eye must be strictly inside the original bounding rect
        assert!(
            focal_eye[0] >= rect[0] && focal_eye[0] <= rect[0] + rect[2],
            "Focal eye x outside bounds: {}",
            focal_eye[0]
        );
        assert!(
            focal_eye[1] >= rect[1] && focal_eye[1] <= rect[1] + rect[3],
            "Focal eye y outside bounds: {}",
            focal_eye[1]
        );
    }

    #[test]
    fn test_pressure_curve_eval_and_vector_field() {
        let curves = [
            PressureCurve::Uniform,
            PressureCurve::Linear([0.3, 0.9]),
            PressureCurve::ThreePoint([0.4, 1.2, 0.5]),
            PressureCurve::Gaussian {
                curve: [0.5, 0.2],
                min_max: [0.2, 1.5],
            },
        ];

        for curve in &curves {
            for step in 0..=10 {
                let t = step as f32 / 10.0;
                let val = curve.eval(t, 42);
                assert!(val > 0.0 && val.is_finite());
            }
        }

        let fields = [
            VectorFieldPreset::Hand,
            VectorFieldPreset::Curved,
            VectorFieldPreset::Zigzag,
            VectorFieldPreset::Waves,
            VectorFieldPreset::Seabed,
            VectorFieldPreset::Spiral,
            VectorFieldPreset::Columns,
        ];

        for field in fields {
            let ang = field.angle_rad(0.5, 0.5, 1.0, 999);
            assert!(ang.is_finite());
        }
    }

    #[test]
    fn test_flow_line_and_spline() {
        let flow = flow_line(
            [100.0, 100.0],
            120.0,
            45.0,
            VectorFieldPreset::Waves,
            1.5,
            [800.0, 600.0],
            77,
        );
        assert!(!flow.contours.is_empty());
        assert!(flow.is_finite());
        assert!(flow.length() > 50.0);

        let pts = [
            [10.0, 10.0, 0.5],
            [50.0, 80.0, 1.0],
            [90.0, 20.0, 0.7],
            [140.0, 100.0, 0.4],
        ];
        let (spline_path, pressure) = spline(&pts, 0.75);
        assert!(!spline_path.contours.is_empty());
        assert!(spline_path.is_finite());
        assert!(matches!(pressure, PressureCurve::ThreePoint(_)));
    }

    #[test]
    fn test_mass_path_strength_levels() {
        let poly = polygon(&[[10.0, 10.0], [100.0, 10.0], [100.0, 100.0], [10.0, 100.0]]);
        let spec1 = MassSpec {
            precision: 1.0,
            strength: 0.2,
            gradient: 0.0,
            outline: true,
            seed: 1,
        };
        let m1 = mass_path(&poly, &spec1);
        assert!(!m1.contours.is_empty());

        let spec3 = MassSpec {
            precision: 1.0,
            strength: 0.9,
            gradient: 0.0,
            outline: false,
            seed: 2,
        };
        let m3 = mass_path(&poly, &spec3);
        assert!(m3.contours.len() > m1.contours.len() - 1);
    }

    #[test]
    fn test_serde_json_roundtrips() {
        let spec = BrushSpec {
            preset: BrushPreset::TwoB,
            weight: 2.5,
            scatter: Some(1.5),
            sharpness: Some(0.8),
            grain: Some(0.7),
            opacity: Some(0.9),
            spacing: Some(1.1),
            pressure: Some(PressureCurve::ThreePoint([0.5, 1.0, 0.6])),
            wiggle: 1.2,
            field: Some(VectorFieldPreset::Spiral),
            tip_asset: Some("brush_tip_01".into()),
            rotate: TipRotate::Natural,
            marker_tip: Some(true),
            noise: 0.2,
            seed: 42,
        };

        let json = serde_json::to_string(&spec).expect("serialize BrushSpec");
        let deserialized: BrushSpec = serde_json::from_str(&json).expect("deserialize BrushSpec");
        assert_eq!(spec, deserialized);

        let hatch_spec = HatchSpec {
            dist: 7.5,
            angle_deg: 30.0,
            rand: 0.25,
            continuous: true,
            gradient: 0.1,
            cross_angle_deg: Some(60.0),
            wiggle: 0.5,
            seed: 88,
        };
        let h_json = serde_json::to_string(&hatch_spec).expect("serialize HatchSpec");
        let h_de: HatchSpec = serde_json::from_str(&h_json).expect("deserialize HatchSpec");
        assert_eq!(hatch_spec, h_de);
    }
}
