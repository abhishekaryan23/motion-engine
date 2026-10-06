//! Creator tools (plan §9), Phase 1 parts: no new pipeline, only the engine's
//! own subcommands.
//!
//! * `view_frames`: the job's frames at the READ moments (or chosen times) as
//!   one contact sheet (`motion-engine render <scene> --frame N`, ffmpeg tiling);
//! * `explore_styles`: k looks of one story as one sheet, one row per look
//!   (`voice` once, then `compile --art <look> --variety auto` and single
//!   frames per look);
//! * `list_options`: looks, asset families with their medium, music beds and
//!   tones with their best uses (static; also `motionengine://options`);
//! * `plan_assets`: the pictures a job is missing, as generator prompts
//!   (`plan-assets` and `asset-prompts`).
//!
//! Images are one PNG with the long side at most 1024 px. Every tool answers a
//! [`ToolOutput`] with a short text (within the profile's reply budget) and the
//! full data in `structured`.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde_json::Value;

pub mod explore;
pub mod frames;
pub mod options;
pub mod plan;
pub mod proc;
pub mod sheet;
pub mod view;

pub use explore::{auto_look, explore_looks};
pub use frames::{read_moments, Moment};
pub use options::options_json;
pub use view::{picks_for, Pick};

use crate::engine::stem_of;
use crate::job::{self, JobId};
use crate::pictures::PictureIndex;
use crate::profile::{self, ServerConfig};
use crate::reply::{cut, Fix, Reply, Status, ToolOutput};

/// Run creator tool `name`. `pictures` is the library index the story checks
/// use (`explore_styles` runs the same checks as `make_video`).
pub fn call(config: &ServerConfig, pictures: &PictureIndex, name: &str, args: Value) -> ToolOutput {
    match name {
        profile::VIEW_FRAMES => view::view_frames(config, args),
        profile::EXPLORE_STYLES => explore::explore_styles(config, pictures, args),
        profile::LIST_OPTIONS => options::list_options(config, args),
        profile::PLAN_ASSETS => plan::plan_assets(config, args),
        other => not_available(other),
    }
}

/// The answer of a listed tool that is not implemented yet.
pub fn not_available(name: &str) -> ToolOutput {
    let mut r = Reply::new(
        Status::Failed,
        "Use make_video, revise_video and get_video meanwhile.",
    );
    r.error = Some(format!("{name} is not available yet"));
    ToolOutput::from(r)
}

/// Lenient typed arguments (`null` = no arguments); a parse error is a fix.
fn parse_args<T: DeserializeOwned + Default>(args: Value) -> Result<T, Fix> {
    if args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(args).map_err(|e| {
        Fix::new(None, "arguments", cut(&e.to_string()))
            .otherwise("send the fields as the tool's input schema shows")
    })
}

/// `needs_fix` with one fix.
fn needs_fix(fix: Fix) -> ToolOutput {
    ToolOutput::from(Reply::needs_fix(vec![fix]))
}

/// The job id and directory of a model-supplied `job`, or the fix-it reply.
fn job_dir_of(config: &ServerConfig, raw: &str) -> Result<(JobId, PathBuf), ToolOutput> {
    let id = JobId::parse(raw).map_err(needs_fix)?;
    let dir = id.dir(&config.jobs);
    if !dir.is_dir() {
        return Err(needs_fix(
            Fix::new(None, "job", format!("unknown job '{id}'"))
                .otherwise("use the job value from an earlier reply, or call make_video"),
        ));
    }
    Ok((id, dir))
}

/// A path as the replies show it: relative to the repository when inside it.
fn rel(config: &ServerConfig, path: &Path) -> String {
    path.strip_prefix(&config.repo)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

/// The job's scene `<job>/reel/<stem>.motion.json` (`stem` from the intent's
/// title, as the reel names it); any `*.motion.json` of the reel folder when
/// that file is not there. `None` while the job has not compiled yet.
pub fn scene_of(dir: &Path) -> Option<PathBuf> {
    let reel = dir.join(job::REEL_DIR);
    let title = std::fs::read_to_string(dir.join(job::INTENT_JSON))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["title"].as_str().map(str::to_string));
    if let Some(title) = title {
        let scene = reel.join(format!("{}.motion.json", stem_of(&title)));
        if scene.is_file() {
            return Some(scene);
        }
    }
    let mut scenes: Vec<PathBuf> = std::fs::read_dir(&reel)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".motion.json"))
        })
        .collect();
    scenes.sort();
    scenes.into_iter().next()
}
