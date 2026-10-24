//! QA and taste checks: slop, motion soul, type, story warnings, subject guardrails.
//! One test binary per area (fewer links); each file is a module.
//! Run one: `cargo test -p motion-core --test qa_taste <module>::`.

mod font_variety;
mod motion_soul;
mod motion_type;
mod slop_pass2;
mod slop_qa;
mod story_warnings;
mod studio_hype_qa;
mod subject_guardrails;
mod subject_qa;
mod tasteful_motion;
mod temporal_style;
mod typography_registry;
