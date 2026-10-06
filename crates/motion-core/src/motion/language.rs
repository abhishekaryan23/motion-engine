//! Motion languages: coherent behavior profiles (timing, stagger, depth,
//! camera, typography, settle, energy). A language is *not* a layout: recipes
//! keep deciding composition from meaning; the language decides how things
//! move. It is selected by `StyleProfile.motion_language`, or resolved per beat
//! by the MotionCompiler when `auto` (see `compiler::language_for`).
//!
//! Profiles are plain data with strong defaults; weak models only ever name the
//! language (or leave it `auto`).

use super::stagger::{StaggerOrder, StaggerPreset, StaggerSpec};
use crate::easing::{Easing, MotionPreset, PresetParams};
use crate::intent::Energy;
use crate::style::{CameraStyle, MotionLanguage};

/// A concrete (non-auto) motion language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Minimal,
    Kinetic,
    Parallax,
    Sequential,
    Data,
}

impl Language {
    /// `None` for `auto` (the compiler resolves it per beat).
    pub fn from_style(l: MotionLanguage) -> Option<Language> {
        match l {
            MotionLanguage::Auto => None,
            MotionLanguage::Minimal => Some(Language::Minimal),
            MotionLanguage::Kinetic => Some(Language::Kinetic),
            MotionLanguage::Parallax => Some(Language::Parallax),
            MotionLanguage::Sequential => Some(Language::Sequential),
            MotionLanguage::Data => Some(Language::Data),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Language::Minimal => "minimal",
            Language::Kinetic => "kinetic",
            Language::Parallax => "parallax",
            Language::Sequential => "sequential",
            Language::Data => "data",
        }
    }
}

/// How a headline block arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeadlineMotion {
    /// Each line slides up from behind its own line mask (editorial default).
    LineReveal,
    /// Words rise in one after another.
    WordCascade,
    /// A mask edge uncovers each line; content stays still.
    MaskReveal,
}

/// What happens to the beat's keyword inside the headline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmphasisMotion {
    None,
    /// The keyword grows and holds; the rest of the line recedes.
    ScaleEmphasis,
    /// An accent copy of the keyword strikes over it.
    KeywordPunch,
}

/// Depth factors for the camera planes (1 = subject plane).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthPlanes {
    pub background: f32,
    pub midground: f32,
    pub subject: f32,
    pub foreground: f32,
}

impl DepthPlanes {
    /// Everything on one plane: the camera moves the whole stage together.
    pub const FLAT: DepthPlanes = DepthPlanes {
        background: 1.0,
        midground: 1.0,
        subject: 1.0,
        foreground: 1.0,
    };
}

/// Camera behaviour over one beat. `track` is in canvas units (1.0 = 1080 px
/// short side) travelled by the camera across the beat.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraMove {
    /// End zoom of the push (1 = none).
    pub push: f32,
    pub track: [f32; 2],
    pub easing: Easing,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TypographyBehavior {
    pub headline: HeadlineMotion,
    pub emphasis: EmphasisMotion,
}

/// Everything a recipe needs to know about *how* a beat moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionLanguageProfile {
    pub language: Language,
    /// Energy preset after the language's adjustment (travel, curves, pace).
    pub preset: PresetParams,
    pub stagger: StaggerSpec,
    pub depth: DepthPlanes,
    pub camera: CameraMove,
    pub typography: TypographyBehavior,
    /// Curve for secondary/settling motion (no overshoot in minimal/data).
    pub settle: Easing,
    /// Numbers count and quantities grow as data primitives.
    pub data_numbers: bool,
}

fn stagger_preset(energy: Energy) -> StaggerPreset {
    match energy {
        Energy::Calm => StaggerPreset::Calm,
        Energy::Building => StaggerPreset::Editorial,
        Energy::Impact => StaggerPreset::Impact,
    }
}

fn energy_preset(energy: Energy) -> MotionPreset {
    match energy {
        Energy::Calm => MotionPreset::Calm,
        Energy::Building => MotionPreset::Editorial,
        Energy::Impact => MotionPreset::Impact,
    }
}

/// Base camera from the style's camera preference, before the language adapts it.
fn base_camera(style: CameraStyle) -> CameraMove {
    match style {
        CameraStyle::Static => CameraMove {
            push: 1.0,
            track: [0.0, 0.0],
            easing: Easing::Linear,
        },
        CameraStyle::SlowPush => CameraMove {
            push: 1.045,
            track: [0.0, 0.0],
            easing: Easing::Linear,
        },
        CameraStyle::Drift => CameraMove {
            push: 1.015,
            track: [24.0, -10.0],
            easing: Easing::InOutCubic,
        },
    }
}

/// The profile for `language` at a beat's `energy`, honoring the style's camera.
pub fn profile(language: Language, energy: Energy, camera: CameraStyle) -> MotionLanguageProfile {
    let mut preset = energy_preset(energy).params();
    let mut cam = base_camera(camera);
    let still = camera == CameraStyle::Static;
    let mut stagger = StaggerSpec {
        preset: stagger_preset(energy),
        order: StaggerOrder::Forward,
    };
    let mut depth = DepthPlanes::FLAT;
    let mut typography = TypographyBehavior {
        headline: HeadlineMotion::LineReveal,
        emphasis: EmphasisMotion::None,
    };
    let mut settle = preset.settle;
    let mut data_numbers = false;

    match language {
        Language::Minimal => {
            // MinimalCalm / MinimalEditorial / MinimalImpact: hierarchy first,
            // short travel, decisive curves, never an overshoot.
            preset.travel *= 0.45;
            preset.easing = match energy {
                Energy::Calm => Easing::OutCubic,
                Energy::Building | Energy::Impact => Easing::OutQuint,
            };
            preset.duration *= match energy {
                Energy::Calm => 1.05,
                Energy::Building => 0.95,
                Energy::Impact => 0.9,
            };
            settle = Easing::InOutCubic;
            preset.settle = settle;
            if !still {
                cam.push = 1.0 + (cam.push - 1.0) * 0.5;
                cam.track = [cam.track[0] * 0.5, cam.track[1] * 0.5];
            }
            typography.headline = HeadlineMotion::MaskReveal;
        }
        Language::Kinetic => {
            typography.headline = HeadlineMotion::WordCascade;
            typography.emphasis = match energy {
                Energy::Impact => EmphasisMotion::KeywordPunch,
                _ => EmphasisMotion::ScaleEmphasis,
            };
            preset.travel *= 0.8;
        }
        Language::Parallax => {
            depth = DepthPlanes {
                background: 0.35,
                midground: 0.7,
                subject: 1.0,
                foreground: 1.45,
            };
            if !still {
                // Larger, observational camera so the planes separate visibly.
                cam.push = 1.0 + (cam.push - 1.0).max(0.04) * 1.8;
                cam.track = [cam.track[0] + 36.0, cam.track[1] - 64.0];
                cam.easing = Easing::InOutCubic;
            }
        }
        Language::Sequential => {
            stagger.order = StaggerOrder::Forward;
            // Sequences read slower: widen the rhythm one step.
            stagger.preset = match energy {
                Energy::Impact => StaggerPreset::Editorial,
                _ => StaggerPreset::Calm,
            };
            typography.headline = HeadlineMotion::LineReveal;
        }
        Language::Data => {
            data_numbers = true;
            typography.headline = HeadlineMotion::MaskReveal;
            settle = Easing::OutQuint;
            preset.settle = settle;
            preset.travel *= 0.6;
            if !still {
                cam.push = 1.0 + (cam.push - 1.0) * 0.6;
            }
        }
    }

    MotionLanguageProfile {
        language,
        preset,
        stagger,
        depth,
        camera: cam,
        typography,
        settle,
        data_numbers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Language; 5] = [
        Language::Minimal,
        Language::Kinetic,
        Language::Parallax,
        Language::Sequential,
        Language::Data,
    ];

    #[test]
    fn minimal_never_overshoots() {
        for e in [Energy::Calm, Energy::Building, Energy::Impact] {
            let p = profile(Language::Minimal, e, CameraStyle::SlowPush);
            for ease in [p.preset.easing, p.preset.settle, p.settle] {
                assert!(
                    !matches!(ease, Easing::EditorialSpring | Easing::ImpactSpring),
                    "{e:?}: {ease:?}"
                );
            }
        }
    }

    #[test]
    fn parallax_separates_planes() {
        let p = profile(Language::Parallax, Energy::Building, CameraStyle::SlowPush);
        let d = p.depth;
        assert!(d.background < d.midground && d.midground < d.subject && d.subject < d.foreground);
        assert!(p.camera.push > 1.0);
    }

    #[test]
    fn static_camera_stays_static() {
        for l in ALL {
            let p = profile(l, Energy::Impact, CameraStyle::Static);
            assert_eq!(p.camera.push, 1.0, "{l:?}");
            assert_eq!(p.camera.track, [0.0, 0.0], "{l:?}");
        }
    }

    #[test]
    fn profiles_are_deterministic() {
        for l in ALL {
            assert_eq!(
                profile(l, Energy::Calm, CameraStyle::Drift),
                profile(l, Energy::Calm, CameraStyle::Drift)
            );
        }
    }
}
