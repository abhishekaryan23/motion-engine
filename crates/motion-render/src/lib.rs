//! MotionEngine rendering: consumes `ResolvedFrame`s, produces pixels.
//!
//! The renderer never plans anything semantic; it draws exactly what the
//! timeline resolved. Backends implement [`Renderer`]; Sprint 1 ships the CPU
//! backend ([`cpu::CpuRenderer`], tiny-skia). A GPU backend (Vello/wgpu) can
//! implement the same trait later.

pub mod analysis;
pub mod asset_qa;
pub mod audio_mix;
pub mod audio_qa;
pub mod autokey;
pub mod blur;
pub mod contrast_qa;
pub mod cpu;
pub mod decode;
pub mod export;
#[cfg(feature = "gpu")]
pub mod gpu;
pub mod image_index;
pub mod ingest;
pub mod mix_qa;
pub mod music;
pub mod postfx;
pub mod qa;
pub mod reference;
pub mod reveal_qa;
pub mod score;
pub mod sfx;
pub mod speech_qa;
pub mod story_qa;
pub mod text;
pub mod texture;
pub mod treatment;
pub mod visual_qa;
pub mod warp;

pub use cpu::CpuRenderer;
pub use image_index::image_index;
pub use qa::{
    lifecycle_report, structural_profile, MotionProfile, PhaseActivity, SceneQa, SpikeLevel, Status,
};
pub use resvg::tiny_skia;
pub use text::FontMeasure;
pub use visual_qa::{visual_report, BeatVisual, VisualReport, VisualVerdict};

use motion_core::timeline::ResolvedFrame;

/// A render backend: one resolved frame in, one image out.
pub trait Renderer {
    type Frame;
    fn render(&self, frame: &ResolvedFrame<'_>) -> Result<Self::Frame, RenderError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("asset error: {0}")]
    Asset(String),
    #[error("cannot allocate {0}x{1} canvas")]
    Canvas(u32, u32),
    #[error("timeline: {0}")]
    Timeline(#[from] motion_core::TimelineError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("png encode: {0}")]
    Png(String),
    #[error("ffmpeg: {0}")]
    Encode(String),
}
