//! MotionEngine core.
//!
//! Pipeline: `CreativeIntent` → [`compiler`] → `MotionScene` ([`scene`]) →
//! [`validate`] → [`timeline::evaluate_frame`] → `ResolvedFrame` → renderer (other crate).
//!
//! This crate contains no pixel rendering.

pub mod asset_prompts;
pub mod assets;
pub mod audio;
pub mod caption_qa;
pub mod checks;
pub mod compiler;
pub mod easing;
pub mod footage;
pub mod intent;
pub mod layout_qa;
pub mod motion;
pub mod music_mood;
pub mod noise;
pub mod reference;
pub mod scene;
pub mod speech;
pub mod style;
pub mod subject_qa;
pub mod timeline;
pub mod validate;

pub use caption_qa::{caption_report, CaptionReport};
pub use compiler::{
    compile, compile_with_assets, ApproxMeasure, AssetLibrary, CompileError, TextMeasure,
};
pub use easing::{Easing, MotionPreset};
pub use intent::CreativeIntent;
pub use layout_qa::{layout_report, layout_report_with, LayoutReport, LayoutVerdict};
pub use scene::MotionProject;
pub use style::StyleProfile;
pub use subject_qa::{ImageFacts, ImageIndex};
pub use timeline::{evaluate_frame, ResolvedFrame, TimelineError};
pub use validate::{validate, ValidationError, ValidationErrors};
