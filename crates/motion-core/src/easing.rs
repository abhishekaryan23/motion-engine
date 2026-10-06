//! Easing curves and semantic motion presets.
//!
//! All curves map progress `t` (clamped to 0..=1) to eased progress with
//! `ease(0) == 0` and `ease(1) == 1` exactly. Springs may overshoot in between.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Easing {
    #[default]
    Linear,
    InCubic,
    OutCubic,
    InOutCubic,
    OutQuint,
    /// Near-critically damped spring, ~1% overshoot. Confident, settled.
    EditorialSpring,
    /// Under-damped spring, visible overshoot. For hits and reveals.
    ImpactSpring,
}

impl Easing {
    /// The curve for values that must land exactly (counters, data): springs
    /// overshoot and would briefly show a wrong number, so they become OutQuint.
    pub fn landing(self) -> Easing {
        match self {
            Easing::EditorialSpring | Easing::ImpactSpring => Easing::OutQuint,
            e => e,
        }
    }

    pub fn apply(self, t: f64) -> f64 {
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        match self {
            Easing::Linear => t,
            Easing::InCubic => t * t * t,
            Easing::OutCubic => 1.0 - (1.0 - t).powi(3),
            Easing::InOutCubic => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
            Easing::OutQuint => 1.0 - (1.0 - t).powi(5),
            Easing::EditorialSpring => spring(t, 0.78, 9.0),
            Easing::ImpactSpring => spring(t, 0.42, 13.0),
        }
    }
}

/// Analytic damped spring from 0 to 1 over normalized time, corrected so the
/// curve lands exactly on 1 at t = 1 (residual oscillation is folded in linearly).
fn spring(t: f64, zeta: f64, omega: f64) -> f64 {
    let raw = |t: f64| {
        let wd = omega * (1.0 - zeta * zeta).sqrt();
        let decay = (-zeta * omega * t).exp();
        1.0 - decay * ((wd * t).cos() + (zeta * omega / wd) * (wd * t).sin())
    };
    let residual = 1.0 - raw(1.0);
    raw(t) + residual * t
}

/// Semantic motion presets. The compiler reaches for these instead of
/// hand-picking durations and curves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotionPreset {
    Calm,
    Editorial,
    Impact,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresetParams {
    /// Seconds for a primary entrance.
    pub duration: f64,
    /// Entrance travel distance in pixels (at 1080 px canvas width).
    pub travel: f32,
    /// Curve for entrances.
    pub easing: Easing,
    /// Curve for secondary / settling motion.
    pub settle: Easing,
    /// Seconds between staggered elements.
    pub stagger: f64,
}

impl MotionPreset {
    pub fn params(self) -> PresetParams {
        match self {
            MotionPreset::Calm => PresetParams {
                duration: 1.1,
                travel: 40.0,
                easing: Easing::OutCubic,
                settle: Easing::InOutCubic,
                stagger: 0.18,
            },
            MotionPreset::Editorial => PresetParams {
                duration: 0.8,
                travel: 90.0,
                easing: Easing::OutQuint,
                settle: Easing::EditorialSpring,
                stagger: 0.12,
            },
            MotionPreset::Impact => PresetParams {
                duration: 0.55,
                travel: 160.0,
                easing: Easing::ImpactSpring,
                settle: Easing::OutQuint,
                stagger: 0.07,
            },
        }
    }
}

/// (0.14) Progress of a [`crate::scene::SpringSpec`] motion `elapsed` seconds
/// after its start: the closed-form step response of a damped harmonic
/// oscillator (x(0) = 0, x'(0) = 0, target 1). Springs settle on their own
/// clock, so the residual `1 - x(duration)` is blended out with a
/// smootherstep over the motion: progress is exactly 0 at the start and
/// exactly 1 at `duration` and after, with no jump. Pure (no accumulated time).
pub fn spring_progress(elapsed: f64, duration: f64, s: crate::scene::SpringSpec) -> f64 {
    if elapsed.is_nan() || elapsed <= 0.0 {
        return 0.0;
    }
    if duration <= 0.0 || elapsed >= duration {
        return 1.0;
    }
    let x = |t: f64| spring_response(t, s);
    let u = elapsed / duration;
    let smoother = u * u * u * (u * (u * 6.0 - 15.0) + 10.0);
    x(elapsed) + (1.0 - x(duration)) * smoother
}

/// Raw oscillator response x(t) for unit step (may overshoot; tends to 1).
pub fn spring_response(t: f64, s: crate::scene::SpringSpec) -> f64 {
    let (k, c, m) = (
        s.stiffness as f64,
        s.damping as f64,
        (s.mass as f64).max(1e-6),
    );
    if k <= 0.0 {
        return 1.0;
    }
    let w0 = (k / m).sqrt();
    let zeta = c / (2.0 * (k * m).sqrt());
    if (zeta - 1.0).abs() < 1e-6 {
        1.0 - (-w0 * t).exp() * (1.0 + w0 * t)
    } else if zeta < 1.0 {
        let wd = w0 * (1.0 - zeta * zeta).sqrt();
        1.0 - (-zeta * w0 * t).exp() * ((wd * t).cos() + zeta * w0 / wd * (wd * t).sin())
    } else {
        let r = (zeta * zeta - 1.0).sqrt();
        let (r1, r2) = (-w0 * (zeta - r), -w0 * (zeta + r));
        1.0 + (r2 * (r1 * t).exp() - r1 * (r2 * t).exp()) / (r1 - r2)
    }
}

#[cfg(test)]
mod spring_tests {
    use super::*;
    use crate::scene::SpringSpec;

    #[test]
    fn spring_progress_starts_at_zero_and_lands_exactly() {
        for (k, c) in [(170.0, 26.0), (300.0, 10.0), (120.0, 40.0)] {
            let s = SpringSpec {
                stiffness: k,
                damping: c,
                mass: 1.0,
            };
            assert_eq!(spring_progress(0.0, 0.8, s), 0.0);
            assert_eq!(spring_progress(0.8, 0.8, s), 1.0);
            assert_eq!(spring_progress(2.0, 0.8, s), 1.0);
            // continuous near the end
            assert!((spring_progress(0.7999, 0.8, s) - 1.0).abs() < 1e-3);
        }
        // under-damped overshoots
        let bouncy = SpringSpec {
            stiffness: 300.0,
            damping: 10.0,
            mass: 1.0,
        };
        assert!((0..80).any(|i| spring_progress(i as f64 * 0.01, 0.8, bouncy) > 1.02));
    }
}
