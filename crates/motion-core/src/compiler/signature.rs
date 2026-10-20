//! (0.24 M10) Motion signature: HOW things move, per look, as data.
//!
//! Speech-lock (`speech_lifecycle`) says WHEN a beat's text may appear; this
//! table says how it arrives and leaves. Every look shares one small
//! vocabulary and differs in character (springs, travel, ease length), not in
//! stacked effects:
//!
//! | element | motion |
//! |---|---|
//! | headline lines | rise 0.35 line heights from a baseline mask, `line_secs`, OutQuint, staggered `line_stagger` |
//! | hero figure / number | `spring` scale `hero_from` -> 1 plus opacity 0 -> 1 over `hero_secs` (no overshoot beyond 2 %) |
//! | pictures, flags, cards | slide `slide_px` from the side the composition opens toward + fade, `slide_secs`, OutCubic; the side alternates by beat |
//! | support text | fades in `support_secs`, `support_lag` after its lead |
//! | roles (0.24 Wave 1) | `role_lag` / `role_dur_mul` per [`Role`] (lead, support, accent, ambient): the lead lands on its word, the rest overlap it; `anticipate_frac` of the travel is a counter-move first |
//! | exits | fade `exit_secs` + `exit_drift_px` drift opposite to the entrance; never a pop-out |
//! | camera | one slow drift or push per beat, at most `camera_push_max`; no zoom-bursts |
//!
//! No look pulses, glitches, echoes or shakes (`slop_flicker`).

use crate::compiler::art_direction::Look;
use crate::easing::Easing;
use crate::scene::SpringSpec;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSignature {
    /// Spring of the hero figure / word.
    pub spring: SpringSpec,
    /// Hero start scale (ends at 1).
    pub hero_from: f32,
    pub hero_secs: f64,
    /// Picture / flag / card slide travel (u = px at a 1080 px short side).
    pub slide_px: f32,
    pub slide_secs: f64,
    pub slide_ease: Easing,
    /// Headline line reveal.
    pub line_rise: f32,
    pub line_secs: f64,
    pub line_stagger: f64,
    /// Support text fade length and lag after its lead.
    pub support_secs: f64,
    pub support_lag: f64,
    /// Exit fade length and drift (u).
    pub exit_secs: f64,
    pub exit_drift_px: f32,
    /// Largest camera scale change over a beat (0.03 = 3 %).
    pub camera_push_max: f32,
    /// (0.24 Wave 1) Start offset (s) from the lead's speech-locked time per
    /// [`Role`] (lead, support, accent, ambient). Negative = already moving.
    pub role_lag: [f64; 4],
    /// (0.24 Wave 1) Entrance duration multiple of the lead's per role.
    pub role_dur_mul: [f32; 4],
    /// (0.24 Wave 1) Counter-move before a slide, as a share of its travel
    /// (0 = none).
    pub anticipate_frac: f32,
    /// (0.24 Wave 2) Bulge of a picture's arc as a share of its travel
    /// (the control point sits this far off the chord's middle).
    pub arc_frac: f32,
    /// (0.24 Wave 2) Stress punch: peak scale and the seconds it takes to
    /// settle back (the rise to the peak is [`crate::compiler::choreography::LOCK_LEAD`]).
    /// Peak 1 = no punch.
    pub punch: (f32, f64),
    /// (0.24 Wave 2) Camera intent per beat purpose.
    pub camera: CameraIntent,
}

/// (0.24 Wave 2) What the camera does for a beat's purpose: the hook pushes
/// in once and holds; a body beat tracks one way while nothing is being
/// read; the payoff stays still until its stress and then pushes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraIntent {
    pub hook_push: f32,
    pub hook_secs: f64,
    /// Body track speed (u per second), outside the read.
    pub body_speed: f32,
    pub payoff_push: f32,
    pub payoff_secs: f64,
}

impl MotionSignature {
    /// The shared editorial base.
    pub const BASE: MotionSignature = MotionSignature {
        spring: SpringSpec::SNAPPY,
        hero_from: 0.94,
        hero_secs: 0.45,
        slide_px: 32.0,
        slide_secs: 0.5,
        slide_ease: Easing::OutCubic,
        line_rise: 0.35,
        line_secs: 0.42,
        line_stagger: 0.07,
        support_secs: 0.3,
        support_lag: 0.09,
        exit_secs: 0.25,
        exit_drift_px: 12.0,
        camera_push_max: 0.03,
        role_lag: [0.0, 0.11, 0.18, -0.3],
        role_dur_mul: [1.0, 1.3, 0.8, 2.5],
        anticipate_frac: 0.2,
        arc_frac: 0.18,
        punch: (1.04, 0.25),
        camera: CameraIntent {
            hook_push: 0.03,
            hook_secs: 1.2,
            body_speed: 1.5,
            payoff_push: 0.02,
            payoff_secs: 0.6,
        },
    };
    /// Hype: snappier springs, bigger slides, quicker lines.
    pub const HYPE: MotionSignature = MotionSignature {
        spring: SpringSpec {
            stiffness: 380.0,
            damping: 34.0,
            mass: 1.0,
        },
        hero_from: 0.92,
        hero_secs: 0.4,
        slide_px: 40.0,
        slide_secs: 0.45,
        line_secs: 0.38,
        line_stagger: 0.06,
        role_lag: [0.0, 0.06, 0.12, -0.3],
        anticipate_frac: 0.25,
        arc_frac: 0.1,
        punch: (1.06, 0.2),
        ..MotionSignature::BASE
    };
    /// Documentary: slower and smaller.
    pub const DOCUMENTARY: MotionSignature = MotionSignature {
        hero_from: 0.96,
        hero_secs: 0.55,
        slide_px: 24.0,
        slide_secs: 0.6,
        line_secs: 0.5,
        line_stagger: 0.09,
        support_lag: 0.12,
        camera_push_max: 0.02,
        role_lag: [0.0, 0.14, 0.22, -0.3],
        anticipate_frac: 0.15,
        arc_frac: 0.25,
        punch: (1.03, 0.3),
        camera: CameraIntent {
            hook_push: 0.02,
            hook_secs: 1.6,
            body_speed: 1.0,
            payoff_push: 0.015,
            payoff_secs: 0.8,
        },
        ..MotionSignature::BASE
    };
    /// Cinematic: the longest eases.
    pub const CINEMATIC: MotionSignature = MotionSignature {
        spring: SpringSpec::HEAVY,
        hero_from: 0.96,
        hero_secs: 0.7,
        slide_px: 32.0,
        slide_secs: 0.75,
        slide_ease: Easing::OutQuint,
        line_secs: 0.6,
        line_stagger: 0.1,
        support_secs: 0.4,
        support_lag: 0.12,
        exit_secs: 0.3,
        role_lag: [0.0, 0.16, 0.26, -0.3],
        anticipate_frac: 0.0,
        arc_frac: 0.3,
        punch: (1.0, 0.3),
        ..MotionSignature::BASE
    };
}

impl Default for MotionSignature {
    fn default() -> Self {
        MotionSignature::BASE
    }
}

/// The signature of a look (`None` = no art direction: the base).
pub fn for_look(look: Option<Look>) -> MotionSignature {
    match look {
        Some(Look::HypeSlam | Look::StreetCollage) => MotionSignature::HYPE,
        Some(Look::Dossier) => MotionSignature::DOCUMENTARY,
        Some(Look::Cinematic3d) => MotionSignature::CINEMATIC,
        _ => MotionSignature::BASE,
    }
}

/// Side a picture arrives from on beat `index`: -1 left, +1 right. Alternates
/// by beat so neighbouring pictures never come from the same side.
pub fn slide_side(index: usize) -> f32 {
    if index.is_multiple_of(2) {
        1.0
    } else {
        -1.0
    }
}
