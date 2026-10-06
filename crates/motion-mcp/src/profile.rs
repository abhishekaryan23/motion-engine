//! Profiles and server configuration (plan §9).
//!
//! The profile is chosen once, at server start (`motion-mcp --profile …`), in
//! the client's MCP config. It picks the tool list and each tool's input
//! schema ([`crate::schema`]); the same tool names exist in every profile.
//! Dynamic unlocking (`tools/list_changed`) is deliberately not used.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Who is calling: control grows with model strength.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    /// Sub-4B up to Haiku-class: 4 fat tools, the lite story and one tone word.
    #[default]
    Weak,
    /// Flash-class multimodal: full CreativeIntent + StyleProfile, production
    /// options and a look-and-revise loop (≈ 8 tools).
    Creator,
    /// Opus-class or a human: the full toolset (≈ 16 tools).
    Operator,
}

impl Profile {
    pub fn name(self) -> &'static str {
        match self {
            Profile::Weak => "weak",
            Profile::Creator => "creator",
            Profile::Operator => "operator",
        }
    }

    /// Creator and operator accept a full CreativeIntent, a StyleProfile object
    /// and production options; weak accepts the lite story and a tone word.
    pub fn full_control(self) -> bool {
        !matches!(self, Profile::Weak)
    }
}

// Tool names (identical in every profile).
pub const MAKE_VIDEO: &str = "make_video";
pub const REVISE_VIDEO: &str = "revise_video";
pub const GET_VIDEO: &str = "get_video";
pub const FIND_ASSETS: &str = "find_assets";
// creator
pub const VIEW_FRAMES: &str = "view_frames";
pub const EXPLORE_STYLES: &str = "explore_styles";
pub const LIST_OPTIONS: &str = "list_options";
pub const PLAN_ASSETS: &str = "plan_assets";
// operator (Phase 2; listed, answering "not available yet" until then)
pub const RENDER_FRAME: &str = "render_frame";
pub const INSPECT_FRAME: &str = "inspect_frame";
pub const QA_REPORT: &str = "qa_report";
pub const INGEST_ASSETS: &str = "ingest_assets";
pub const MATTE: &str = "matte";
pub const MUSIC_INDEX: &str = "music_index";
pub const SFX_INDEX: &str = "sfx_index";
pub const REFERENCE_EVIDENCE: &str = "reference_evidence";
/// The one place timing or pixel data can be touched: operator only, and only
/// with `--allow-scene-edits` (not listed otherwise; refused if called).
pub const APPLY_SCENE_PATCH: &str = "apply_scene_patch";

pub const WEAK_TOOLS: [&str; 4] = [MAKE_VIDEO, REVISE_VIDEO, GET_VIDEO, FIND_ASSETS];
pub const CREATOR_EXTRA: [&str; 4] = [VIEW_FRAMES, EXPLORE_STYLES, LIST_OPTIONS, PLAN_ASSETS];
pub const OPERATOR_EXTRA: [&str; 8] = [
    RENDER_FRAME,
    INSPECT_FRAME,
    QA_REPORT,
    INGEST_ASSETS,
    MATTE,
    MUSIC_INDEX,
    SFX_INDEX,
    REFERENCE_EVIDENCE,
];

/// The tools a profile lists, in listing order.
pub fn tool_names(profile: Profile, allow_scene_edits: bool) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = WEAK_TOOLS.to_vec();
    if profile.full_control() {
        names.extend(CREATOR_EXTRA);
    }
    if profile == Profile::Operator {
        names.extend(OPERATOR_EXTRA);
        if allow_scene_edits {
            names.push(APPLY_SCENE_PATCH);
        }
    }
    names
}

/// Tool-definition budgets (characters of the serialized `tools/list` entries;
/// ≈ 4 characters per token). Tested in `tests/tool_budget.rs`.
pub const WEAK_BUDGET_CHARS: usize = 4_800;
pub const CREATOR_BUDGET_CHARS: usize = 12_000;
pub const OPERATOR_BUDGET_CHARS: usize = 24_000;

pub fn budget_chars(profile: Profile) -> usize {
    match profile {
        Profile::Weak => WEAK_BUDGET_CHARS,
        Profile::Creator => CREATOR_BUDGET_CHARS,
        Profile::Operator => OPERATOR_BUDGET_CHARS,
    }
}

/// Reply budgets in characters (text line; ≈ 4 characters per token).
pub fn reply_budget_chars(profile: Profile) -> usize {
    match profile {
        Profile::Weak => 800,
        Profile::Creator => 1_000,
        Profile::Operator => 1_600,
    }
}

/// Job retention (plan §6): newest `max_jobs`, at most `max_bytes` in total.
/// Pruning never touches anything outside the jobs directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retention {
    pub max_jobs: usize,
    pub max_bytes: u64,
}

impl Default for Retention {
    fn default() -> Self {
        Retention {
            max_jobs: 50,
            max_bytes: 20 * 1024 * 1024 * 1024,
        }
    }
}

/// Everything the server needs, resolved at start. Paths are absolute.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub profile: Profile,
    /// Repository root: the working directory of every engine subprocess.
    pub repo: PathBuf,
    /// The `motion-engine` binary.
    pub engine: PathBuf,
    /// Asset library root handed to the engine (`<repo>/assets`).
    pub assets: PathBuf,
    /// Sandboxed folders user images may be referenced from (plan §5a);
    /// default `[<repo>/assets/inbox]`.
    pub asset_roots: Vec<PathBuf>,
    /// Job store root (`<repo>/output/jobs`). The server writes nowhere else.
    pub jobs: PathBuf,
    /// Operator only: list and allow `apply_scene_patch`.
    pub allow_scene_edits: bool,
    /// Extra flags appended to every `motion-engine reel` call (tests pass
    /// `--offline`; 0.20 defaults such as `--align` are left to the binary).
    pub reel_extra: Vec<String>,
    /// Longest a `make_video` / `revise_video` call waits for its render before
    /// replying `running` (seconds). `get_video(wait_s)` picks up from there.
    pub max_wait_s: u64,
    pub retention: Retention,
    /// Mixed into every job id: the build's `git describe` plus a hash of the
    /// engine binary ([`engine_version`]).
    pub engine_version: String,
    /// Voice model forwarded as `reel --tts-model` (weak/creator: always `auto`,
    /// the free pinned male narrator). Mixed into job ids.
    pub tts_model: String,
}

impl ServerConfig {
    /// Defaults under `repo` (binary `target/release/motion-engine`).
    pub fn new(profile: Profile, repo: impl Into<PathBuf>) -> Self {
        let repo = repo.into();
        ServerConfig {
            profile,
            engine: repo.join("target/release/motion-engine"),
            assets: repo.join("assets"),
            asset_roots: vec![repo.join("assets/inbox")],
            jobs: repo.join("output/jobs"),
            repo,
            allow_scene_edits: false,
            reel_extra: Vec::new(),
            max_wait_s: 600,
            retention: Retention::default(),
            engine_version: build_version().to_string(),
            tts_model: "auto".to_string(),
        }
    }

    /// A `motion-engine` command with the repository root as working directory.
    /// Every engine call (reel, render --frame, explore, qa, matte …) starts here.
    pub fn engine_command(&self) -> Command {
        let mut c = Command::new(&self.engine);
        c.current_dir(&self.repo);
        c
    }

    /// The directory of job `id` (no existence check).
    pub fn job_dir(&self, id: &str) -> PathBuf {
        self.jobs.join(id)
    }
}

/// `git describe` of the motion-mcp build.
pub fn build_version() -> &'static str {
    env!("MOTION_MCP_GIT_DESCRIBE")
}

/// The version string mixed into job ids: build version + the first 12 hex
/// characters of the engine binary's sha256 (`nobin` when it cannot be read).
pub fn engine_version(engine: &Path) -> String {
    use sha2::{Digest, Sha256};
    let hash = std::fs::read(engine)
        .map(|bytes| {
            let digest = Sha256::digest(&bytes);
            digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
        })
        .unwrap_or_else(|_| "nobin".to_string());
    format!("{}+{hash}", build_version())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_grow_and_keep_names() {
        let weak = tool_names(Profile::Weak, true);
        let creator = tool_names(Profile::Creator, true);
        let operator = tool_names(Profile::Operator, false);
        assert_eq!(weak.len(), 4);
        assert_eq!(creator.len(), 8);
        assert_eq!(operator.len(), 16);
        assert!(creator.starts_with(&weak) && operator.starts_with(&creator));
        assert!(!operator.contains(&APPLY_SCENE_PATCH));
        assert!(tool_names(Profile::Operator, true).contains(&APPLY_SCENE_PATCH));
        assert!(!tool_names(Profile::Creator, true).contains(&APPLY_SCENE_PATCH));
    }
}
