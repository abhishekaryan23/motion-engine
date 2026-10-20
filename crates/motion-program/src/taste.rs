//! Baked-in taste: verb durations, the named springs, sizes and slots.
//! Everything a program does not say comes from here, so a one-line program
//! still moves well. Springs are closed-form (motion-core `spring_progress`)
//! with at most one small overshoot; nothing loops or pulses.

use motion_core::easing::Easing;
use motion_core::scene::SpringSpec;

use crate::program::{Aspect, Size, Slot, Tempo, TitlePreset, Verb};

/// The Golden Ratio φ = (1 + √5) / 2 ≈ 1.618034.
pub const PHI: f32 = 1.618_034;
/// Major Golden Section 1 / φ ≈ 0.618034 (61.8%).
pub const INV_PHI: f32 = 0.618_034;
/// Minor Golden Section 1 - 1 / φ = 1 / φ² ≈ 0.381966 (38.2%).
pub const PHI_MINOR: f32 = 0.381_966;

/// Multiply or divide `base` by φ^step (e.g., step = 1 -> base * 1.618, step = -1 -> base / 1.618).
pub fn phi_step(base: f32, step: i32) -> f32 {
    base * PHI.powi(step)
}

/// The 4 Golden Spiral / Golden Section focal eyes on a `w × h` canvas:
/// `[0.382·w, 0.382·h]`, `[0.618·w, 0.382·h]`, `[0.382·w, 0.618·h]`, `[0.618·w, 0.618·h]`.
pub fn golden_eyes(w: f32, h: f32) -> [[f32; 2]; 4] {
    [
        [w * PHI_MINOR, h * PHI_MINOR],
        [w * INV_PHI, h * PHI_MINOR],
        [w * PHI_MINOR, h * INV_PHI],
        [w * INV_PHI, h * INV_PHI],
    ]
}

/// The named springs (unit mass, at most six by design). ζ = damping / (2·√stiffness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spring {
    /// ζ ≈ 0.8, ~1.5 % overshoot: things settling into place.
    Settle,
    /// ζ = 1, no overshoot: crisp arrivals.
    Snap,
    /// ζ ≈ 0.65, ~7 % overshoot (once): pops and hits.
    Pop,
    /// ζ = 0.55, slow (ω = 6): a spring shown as a spring (graphs of a
    /// spring response): visible overshoot and settle over ~2 s.
    Lively,
}

impl Spring {
    pub fn spec(self) -> SpringSpec {
        let (stiffness, zeta): (f32, f32) = match self {
            Spring::Settle => (260.0, 0.8),
            Spring::Snap => (320.0, 1.0),
            Spring::Pop => (300.0, 0.65),
            Spring::Lively => (36.0, 0.55),
        };
        SpringSpec {
            stiffness,
            damping: 2.0 * zeta * stiffness.sqrt(),
            mass: 1.0,
        }
    }
}

/// How a motion is shaped: an easing curve or a named spring.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Ease(Easing),
    Spring(Spring),
}

impl Tempo {
    pub fn factor(self) -> f64 {
        match self {
            Tempo::Slow => 1.35,
            Tempo::Normal => 1.0,
            Tempo::Fast => 0.75,
        }
    }
}

/// Base seconds of a verb at normal tempo.
pub fn verb_seconds(v: &Verb) -> f64 {
    use crate::program::{EnterStyle as E, ExitStyle as X};
    match v {
        Verb::Enter { style } => match style {
            E::Pop => 0.6,
            E::Rise => 0.7,
            E::Fade => 0.45,
            E::Draw => 1.1,
            E::Wipe => 0.6,
        },
        Verb::Exit { style } => match style {
            X::Cut => 0.034,
            X::Fade => 0.35,
            X::Shrink => 0.4,
            X::Drop => 0.45,
            X::Wipe => 0.45,
        },
        Verb::Morph { .. } => 0.7,
        Verb::Move { .. } => 0.8,
        Verb::Recolour { .. } => 0.4,
        Verb::Hit => 0.5,
        Verb::Cut { .. } => 0.0,
        Verb::PopLabel => 0.6,
        Verb::Bounce => 0.75,
        Verb::Orbit { .. } => 1.0,
        Verb::Fill => 0.8,
        Verb::Unfill => 0.7,
        Verb::Spin => 0.6,
        Verb::Wave { .. } => 0.6,
        Verb::Advance => 0.3,
        Verb::Sheen => 0.9,
        Verb::Tilt { .. } => 0.65,
        Verb::Echo { .. } => 0.55,
    }
}

/// Seconds a verb lasts under `tempo`, rounded to the millisecond so files
/// stay readable and byte-stable.
pub fn duration(v: &Verb, tempo: Tempo) -> f64 {
    ((verb_seconds(v) * tempo.factor()) * 1000.0).round() / 1000.0
}

/// Hard cuts land this many frames before their beat (the eye reads the
/// cut on the beat).
pub const CUT_LEAD_FRAMES: f64 = 2.0;

/// Fade-in seconds that accompany pops and rises (fixed, not tempo-scaled).
pub const QUICK_FADE: f64 = 0.15;

/// Shape side as a share of the frame's short side.
pub fn shape_share(s: Size) -> f32 {
    match s {
        Size::S => 0.16,
        Size::M => 0.28,
        Size::L => 0.42,
        Size::Xl => 0.85,
    }
}

/// Type size as a share of the frame's short side, per preset.
pub fn type_share(p: TitlePreset, s: Size) -> f32 {
    let base = match s {
        Size::S => 0.045,
        Size::M => 0.075,
        Size::L => 0.115,
        Size::Xl => 0.17,
    };
    match p {
        TitlePreset::Display => base,
        TitlePreset::Serif => base * 1.1,
        TitlePreset::Label => base * 0.45,
        TitlePreset::Chapter => 0.065,
        TitlePreset::Stacked => base * 2.2,
    }
}

/// Slot centre in canvas pixels.
pub fn slot_center(slot: Slot, aspect: Aspect) -> [f32; 2] {
    let (w, h) = aspect.size();
    let (w, h) = (w as f32, h as f32);
    let landscape = aspect == Aspect::Landscape;
    match slot {
        Slot::Center => [w / 2.0, h / 2.0],
        Slot::Left => [w * 0.27, h * 0.5],
        Slot::Right => [w * 0.73, h * 0.5],
        Slot::Lead if landscape => [w / 3.0, h / 2.0],
        Slot::Trail if landscape => [w * 2.0 / 3.0, h / 2.0],
        Slot::Lead => [w / 2.0, h * PHI_MINOR],
        Slot::Trail => [w / 2.0, h * INV_PHI],
        Slot::Top => [w / 2.0, h * 0.2],
        Slot::Bottom => [w / 2.0, h * 0.8],
        // Corner slots are left edges (titles align to them).
        Slot::TopLeft => [w.min(h) * 0.06, w.min(h) * 0.12],
        Slot::BottomLeft => [w.min(h) * 0.06, h - w.min(h) * 0.12],
    }
}

/// Width available to a title at `slot` (safe margins included).
pub fn slot_width(slot: Slot, aspect: Aspect) -> f32 {
    let (w, _) = aspect.size();
    let w = w as f32;
    match (slot, aspect) {
        (Slot::Left | Slot::Right, _) => w * 0.40,
        (Slot::Lead | Slot::Trail, Aspect::Landscape) => w * 0.3,
        _ => w * 0.84,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_golden_ratio_identities() {
        assert!((INV_PHI - 1.0 / PHI).abs() < 1e-5);
        assert!((PHI_MINOR - (1.0 - INV_PHI)).abs() < 1e-5);
        assert!((PHI_MINOR - 1.0 / (PHI * PHI)).abs() < 1e-5);
        assert!((INV_PHI + PHI_MINOR - 1.0).abs() < 1e-5);
    }

    #[test]
    fn test_phi_step() {
        let base = 100.0;
        assert_eq!(phi_step(base, 0), 100.0);
        assert!((phi_step(base, 1) - base * PHI).abs() < 1e-4);
        assert!((phi_step(base, -1) - base * INV_PHI).abs() < 1e-3);
        assert!((phi_step(base, -2) - base * PHI_MINOR).abs() < 1e-3);
    }

    #[test]
    fn test_golden_eyes() {
        let eyes = golden_eyes(1000.0, 1000.0);
        assert_eq!(eyes.len(), 4);
        assert!((eyes[0][0] - 381.966).abs() < 1e-3);
        assert!((eyes[0][1] - 381.966).abs() < 1e-3);
        assert!((eyes[1][0] - 618.034).abs() < 1e-3);
        assert!((eyes[1][1] - 381.966).abs() < 1e-3);
        assert!((eyes[2][0] - 381.966).abs() < 1e-3);
        assert!((eyes[2][1] - 618.034).abs() < 1e-3);
        assert!((eyes[3][0] - 618.034).abs() < 1e-3);
        assert!((eyes[3][1] - 618.034).abs() < 1e-3);
    }

    #[test]
    fn test_portrait_slot_center_golden_sections() {
        let lead = slot_center(Slot::Lead, Aspect::Portrait);
        let trail = slot_center(Slot::Trail, Aspect::Portrait);
        let (_, h) = Aspect::Portrait.size();
        let h = h as f32;
        assert!((lead[1] - h * PHI_MINOR).abs() < 1e-3);
        assert!((trail[1] - h * INV_PHI).abs() < 1e-3);
    }
}
