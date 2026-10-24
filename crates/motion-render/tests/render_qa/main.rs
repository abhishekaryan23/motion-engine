//! Frame and story QA, scores and regressions.
//! One test binary per area (fewer links, fewer first-launch scans); each
//! file is a module. Run one: `cargo test -p motion-render --test render_qa <module>::`.

mod contrast_qa;
mod qa;
mod qa_drift;
mod score;
mod slam_sync_qa;
mod speech_qa_timing;
mod sprint23_regressions;
mod story_qa;
