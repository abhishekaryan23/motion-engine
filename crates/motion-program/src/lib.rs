//! MotionEngine 2: the MotionProgram.
//!
//! A strong model or a recipe writes a closed, declarative program
//! (components + an explicit timeline of verbs); [`validate`] checks it and
//! [`lower`] compiles it deterministically to a v1 [`MotionProject`], so the
//! renderer, QA, audio and export are shared with the CreativeIntent path.
//! Weak models keep writing CreativeIntent only.
//!
//! [`MotionProject`]: motion_core::MotionProject

pub mod bento;
mod camera;
mod chart;
pub mod code;
pub mod device;
mod diagram;
mod field;
pub mod flip;
pub mod flow;
mod kinetic;
pub mod layout;
pub mod ledger;
pub mod lockup;
pub mod lower;
pub mod palette;
pub mod photo;
pub mod program;
pub mod qa;
pub mod recipes;
pub mod repair;
pub mod sketch;
pub mod taste;
pub mod ui_gates;
pub mod validate;

pub use lower::{lower, lower_with, LowerOptions, Typesetter};
pub use program::MotionProgram;
pub use validate::{validate, Issue, Issues};

/// Parse, resolve `beat` references against `beats` (program seconds),
/// validate and lower in one step.
pub fn compile(
    json: &str,
    beats: &[f64],
    asset_root: Option<String>,
    ts: &Typesetter,
) -> Result<motion_core::MotionProject, String> {
    let mut p = MotionProgram::from_json(json).map_err(|e| format!("program JSON: {e}"))?;
    p.resolve_beats(beats);
    validate(&p).map_err(|e| e.to_string())?;
    Ok(lower(&p, asset_root, ts))
}
