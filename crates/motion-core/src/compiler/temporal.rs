//! Temporal style (0.6): the MotionCompiler's numeric reading of the
//! TasteDirector's semantic tendencies (see `taste.rs`).
//!
//! TasteDirector says *how motion should feel* (restrained, precise,
//! energetic; editorial vs kinetic handoffs; measured vs active rhythm). This
//! module is the only place those words become multipliers, curves and counts.
//! Every mapping is the identity for the classic profile (Editorial
//! temperament, Editorial transitions, MeasuredEditorial rhythm, Moderate
//! scale), so pre-0.6 styles compile byte-identically.
//!
//! A future ChoreographyEngine constrains *when* events land (beats, speech);
//! it will consume the same semantic inputs and never edit these tables.

use super::taste::{
    CameraActivity, CompositionRhythm, ExitGesture, LanguageLean, Level, MotionTemperament,
    OverlapCharacter, ScaleContrast, SettleCharacter, StaggerCharacter, TransitionCharacter,
    WipeTendency,
};
use crate::easing::Easing;
use crate::intent::Energy;
use crate::motion::language::{Language, MotionLanguageProfile};
use crate::motion::stagger::StaggerPreset;

/// Adapt a beat's motion-language profile to the style's temperament.
pub(crate) fn apply_temperament(p: &mut MotionLanguageProfile, m: &MotionTemperament, e: Energy) {
    p.preset.travel *= match m.amplitude {
        Level::Low => 0.65,
        Level::Medium => 1.0,
        Level::High => 1.4,
    };
    p.preset.duration *= match m.settle {
        SettleCharacter::Soft => 1.1,
        SettleCharacter::Standard => 1.0,
        SettleCharacter::Crisp => 0.8,
        SettleCharacter::Springy => 0.92,
    };
    match m.settle {
        SettleCharacter::Standard => {}
        SettleCharacter::Soft => {
            p.preset.easing = Easing::OutCubic;
            p.settle = Easing::InOutCubic;
        }
        SettleCharacter::Crisp => {
            p.preset.easing = Easing::OutQuint;
            p.settle = Easing::OutQuint;
        }
        SettleCharacter::Springy => {
            // Overshoot on arrival (not for data: numbers must land exactly).
            if p.language != Language::Data {
                p.preset.easing = if e == Energy::Impact {
                    Easing::ImpactSpring
                } else {
                    Easing::EditorialSpring
                };
            }
            p.settle = Easing::EditorialSpring;
        }
    }
    if m.overshoot == Level::Low {
        for ease in [&mut p.preset.easing, &mut p.settle] {
            if matches!(*ease, Easing::EditorialSpring | Easing::ImpactSpring) {
                *ease = Easing::OutQuint;
            }
        }
    }
    p.preset.settle = p.settle;
    p.stagger.preset = match m.stagger {
        StaggerCharacter::Editorial => p.stagger.preset,
        // Tight, even arrivals.
        StaggerCharacter::Structured => StaggerPreset::Impact,
        // Wider gaps: the cascade is visible.
        StaggerCharacter::Cascading => match e {
            Energy::Impact => StaggerPreset::Editorial,
            _ => StaggerPreset::Calm,
        },
    };
    let cam = &mut p.camera;
    let (push_k, track_k) = match m.camera {
        CameraActivity::Still => (0.0, 0.0),
        CameraActivity::Controlled => (0.5, 0.5),
        CameraActivity::Standard => (1.0, 1.0),
        CameraActivity::Active => (1.8, 1.6),
    };
    let moving = (cam.push - 1.0).abs() > 1e-4 || cam.track != [0.0, 0.0];
    cam.push = 1.0 + (cam.push - 1.0) * push_k;
    cam.track = [cam.track[0] * track_k, cam.track[1] * track_k];
    if m.camera == CameraActivity::Active && moving && cam.track == [0.0, 0.0] {
        cam.track = [18.0, -28.0];
        cam.easing = Easing::InOutCubic;
    }
}

/// Lean an `auto` language choice toward the temperament (meaning decided
/// first; the lean only swaps between compatible languages).
pub(crate) fn lean_language(l: Language, lean: LanguageLean, e: Energy, keyword: bool) -> Language {
    match (lean, l) {
        (LanguageLean::Classic, _) => l,
        (LanguageLean::Calm, Language::Kinetic) if e != Energy::Impact => Language::Minimal,
        (LanguageLean::Structured, Language::Parallax) => Language::Minimal,
        (LanguageLean::Kinetic, Language::Minimal | Language::Parallax)
            if keyword || e != Energy::Calm =>
        {
            Language::Kinetic
        }
        _ => l,
    }
}

/// Beat-duration factor from composition rhythm (reading floors still apply).
pub(crate) fn duration_scale(r: CompositionRhythm) -> f64 {
    match r {
        CompositionRhythm::SlowBreathing => 1.12,
        CompositionRhythm::MeasuredEditorial => 1.0,
        CompositionRhythm::Progressive => 0.95,
        CompositionRhythm::Active => 0.88,
        CompositionRhythm::HighFrequency => 0.82,
    }
}

/// Shift of the READ share (negative = secondary information arrives sooner).
pub(crate) fn read_bias(r: CompositionRhythm) -> f64 {
    match r {
        CompositionRhythm::SlowBreathing => 0.1,
        CompositionRhythm::MeasuredEditorial => 0.0,
        CompositionRhythm::Progressive => -0.06,
        CompositionRhythm::Active => -0.1,
        CompositionRhythm::HighFrequency => -0.14,
    }
}

/// Stage-level hierarchy reframes during EVOLVE: `(count, scale step)`.
pub(crate) fn hierarchy_shifts(r: CompositionRhythm) -> (usize, f32) {
    match r {
        CompositionRhythm::SlowBreathing | CompositionRhythm::MeasuredEditorial => (0, 0.0),
        CompositionRhythm::Progressive => (1, 0.03),
        CompositionRhythm::Active => (1, 0.06),
        CompositionRhythm::HighFrequency => (2, 0.05),
    }
}

pub(crate) fn overlap_scale(t: &TransitionCharacter) -> f64 {
    match t.overlap {
        OverlapCharacter::Long => 1.25,
        OverlapCharacter::Standard => 1.0,
        OverlapCharacter::Short => 0.72,
    }
}

/// `(accent flood, panel wipe)` for a beat entering with `incoming` energy.
pub(crate) fn handoff(t: &TransitionCharacter, incoming: Energy) -> (bool, bool) {
    match t.wipe {
        WipeTendency::Never => (false, false),
        WipeTendency::ImpactOnly => (incoming == Energy::Impact, false),
        WipeTendency::Frequent => {
            let accent = incoming == Energy::Impact;
            (accent, !accent)
        }
    }
}

/// How the outgoing stage leaves.
pub(crate) fn exit_gesture(t: &TransitionCharacter) -> ExitGesture {
    t.exit
}

/// `(stage scale at end of ANTICIPATE, lift multiplier)`.
pub(crate) fn anticipation(m: &MotionTemperament) -> (f32, f32) {
    match m.anticipation {
        Level::Low => (0.985, 0.5),
        Level::Medium => (0.975, 1.0),
        Level::High => (0.95, 2.2),
    }
}

/// Typesetting gains `(display, support)` for scale contrast.
pub(crate) fn scale_gains(s: ScaleContrast) -> (f32, f32) {
    match s {
        ScaleContrast::Subtle => (0.88, 1.06),
        ScaleContrast::Moderate => (1.0, 1.0),
        ScaleContrast::Large => (1.12, 0.94),
        ScaleContrast::Dramatic => (1.26, 0.86),
    }
}

/// Background drift multiplier during READ.
pub(crate) fn background_drift(a: super::taste::Activity) -> f32 {
    use super::taste::Activity;
    match a {
        Activity::Still => 0.4,
        Activity::Quiet => 1.0,
        Activity::Structured => 1.0,
        Activity::Active => 1.8,
    }
}

/// Gain on secondary motion amplitudes (emphasis pulses, hero crop evolution):
/// multiplies the *excess* over neutral, so 1.0 = unchanged.
pub(crate) fn secondary_gain(m: &MotionTemperament) -> f32 {
    match m.secondary_motion {
        Level::Low => 0.5,
        Level::Medium => 1.0,
        Level::High => 1.7,
    }
}

/// READ-phase liveliness on top of background activity (ghost drift).
pub(crate) fn read_drift(m: &MotionTemperament) -> f32 {
    match m.read_activity {
        Level::Low => 0.7,
        Level::Medium => 1.0,
        Level::High => 1.35,
    }
}

/// Foreground/background handoff during EVOLVE: the background word steps
/// forward (scale) as the hierarchy shifts. `None` for calm rhythms.
pub(crate) fn background_handoff(r: CompositionRhythm) -> Option<f32> {
    match r {
        CompositionRhythm::Active => Some(1.08),
        CompositionRhythm::HighFrequency => Some(1.12),
        _ => None,
    }
}

/// Layer activity → camera-plane separation. Classic (balanced/quiet/quiet)
/// is the identity. Active depth separates planes even outside parallax;
/// a dominant foreground rides slightly ahead of the subject.
pub(crate) fn apply_layers(p: &mut MotionLanguageProfile, l: &super::taste::LayerActivityProfile) {
    use super::taste::{Activity, Presence};
    let d = &mut p.depth;
    let spread = |k: f32, s: f32| 1.0 + (k - 1.0) * s;
    match l.midground {
        Activity::Active => {
            if p.language == Language::Parallax {
                d.background = spread(d.background, 1.25);
                d.midground = spread(d.midground, 1.25);
                d.foreground = spread(d.foreground, 1.25);
            } else if d.background == 1.0 && d.midground == 1.0 {
                d.background = 0.75;
                d.midground = 0.9;
            }
        }
        Activity::Still => {
            d.background = spread(d.background, 0.5);
            d.midground = spread(d.midground, 0.5);
        }
        Activity::Quiet | Activity::Structured => {}
    }
    if l.foreground == Presence::Dominant && d.foreground <= 1.0 {
        d.foreground = 1.12;
    }
}
