//! Reference media analysis (0.7): video file → deterministic evidence,
//! key-frame samples and a contact sheet. Measurement only — the style
//! interpretation is done by an external multimodal model
//! (`motion_core::reference`). Audio is never decoded.

pub mod analyze;
pub mod cache;
pub mod color;
pub mod complexity;
pub mod contact;
pub mod media;
pub mod sample;
pub mod signal;
pub mod temporal;

pub use analyze::{analyze_reference, AnalysisConfig, AnalysisOutcome, Reuse};

#[derive(Debug, thiserror::Error)]
pub enum ReferenceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("ffprobe: {0}")]
    Probe(String),
    #[error("ffmpeg: {0}")]
    Ffmpeg(String),
    #[error("no video stream in {0}")]
    NoVideo(String),
    #[error("image: {0}")]
    Image(String),
}
