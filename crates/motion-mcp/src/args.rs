//! Typed tool arguments. Deserialization is lenient: unknown fields are
//! ignored (the checker reports them), and every field has a default, so a
//! weak model's near-miss still parses. The `story` stays raw JSON here: it is
//! a lite story or a full CreativeIntent and is parsed by
//! [`crate::policy::parse_story`].

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `make_video` / `revise_video` modes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Validate, auto-fix, render, reply (the default).
    #[default]
    Auto,
    /// Validate and auto-fix only; reply in seconds with the plan.
    Check,
    /// Same as `auto` (kept for clients that want to say so explicitly);
    /// with `strict` nothing is auto-fixed.
    Render,
}

/// `make_video`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MakeVideoArgs {
    /// Lite story or full CreativeIntent v0.2 (raw).
    #[serde(default)]
    pub story: Value,
    /// A tone word (`auto`, `editorial`, … `cinematic`) or, creator and up, a
    /// StyleProfile object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    /// vertical (default) | square | wide
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// (0.21) Brand colours, every profile: `{primary, secondary, background,
    /// text}` as hex (lenient: a list or a comma string in that order; `#` is
    /// optional). Becomes `StyleProfile.brand`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<Value>,
    /// A folder of the user's images inside an asset root (plan §5a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    /// (0.23) Every profile: take number 0–99 ("another version" of the same
    /// story); raw and lenient (a number or a numeric string), normalised by
    /// the policy. Part of the job key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<Value>,
    /// (0.23) Every profile: music mood word (`audio::MusicWord`: auto, none,
    /// calm, upbeat, serious, dramatic, playful, neutral); raw and lenient,
    /// normalised by the policy. Part of the job key. (Creator
    /// `options.music` still names a bed id.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<Value>,
    #[serde(default)]
    pub mode: Mode,
    /// Never auto-fix: return `needs_fix` instead.
    #[serde(default)]
    pub strict: bool,
    /// Creator and up: production options.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ProductionOptions>,
    /// Creator and up: return engine-measured labels of the user images.
    #[serde(default)]
    pub describe: bool,
    /// Operator only: re-render even when the job exists.
    #[serde(default)]
    pub force: bool,
}

/// Production options (creator and operator). Every value is checked against
/// `list_options`; nothing here is a pixel or timing field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProductionOptions {
    /// Look: `auto` or a look name (`cinematic_3d`, `dossier`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<String>,
    /// Asset families to search, in order (style lock); default: the look's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub families: Vec<String>,
    /// Music bed: `auto`, `none` or a bed id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<String>,
    /// Word-synced captions (default on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub captions: Option<bool>,
    /// Aspect preset (`story`, `portrait`, `square`, `landscape`, `4:5` …);
    /// overrides `format`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    /// Story-keyed variety: `auto` (default), `off`, or a seed number (from
    /// `explore_styles`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variety: Option<Value>,
    /// Operator only: a paid voice model (`gemini`, `mai`, `deepgram`). The
    /// weak and creator profiles always use the free pinned male narrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tts_model: Option<String>,
    /// Operator only: keep the rendered PNG frames.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep_frames: bool,
}

impl ProductionOptions {
    pub fn is_empty(&self) -> bool {
        *self == ProductionOptions::default()
    }
}

/// `revise_video`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviseVideoArgs {
    #[serde(default)]
    pub job: String,
    #[serde(default)]
    pub changes: Vec<BeatChange>,
    /// New tone word (or StyleProfile object, creator and up).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    /// (0.21) New brand colours (as in `make_video`); omitted = keep the job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Creator and up: a JSON merge patch (RFC 7396) applied to the stored
    /// story (lite or intent) before `changes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ProductionOptions>,
    /// (0.23) New take number; omitted = keep the job's take.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take: Option<Value>,
    /// (0.23) New music mood word; omitted = keep the job's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<Value>,
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub strict: bool,
}

/// One edit: change beat `beat` (1-based), remove it, or insert a new beat
/// after `insert_after` (0 = at the start). Every other key is a lite-beat
/// field to set (`say`, `show`, `picture`, `number`, `list`, …); `null`
/// clears a field.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BeatChange {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<usize>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub remove: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insert_after: Option<usize>,
    #[serde(flatten)]
    pub set: Map<String, Value>,
}

/// `get_video`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GetVideoArgs {
    #[serde(default)]
    pub job: String,
    /// Wait up to this many seconds for the job to finish (0–300, default 0).
    #[serde(default)]
    pub wait_s: u64,
}

/// Longest `get_video` wait.
pub const MAX_WAIT_S: u64 = 300;

/// `find_assets`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FindAssetsArgs {
    #[serde(default)]
    pub words: Vec<String>,
    /// Suggestions per word (1–5, default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<usize>,
}

/// `view_frames` (creator): frames of a drafted or rendered job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ViewFramesArgs {
    #[serde(default)]
    pub job: String,
    /// 1-based beats to show (each at its READ moment). Default: all beats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beats: Option<Vec<usize>>,
    /// Or explicit moments in seconds (a viewing query, not authoring).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<Vec<f64>>,
    /// One contact sheet (default) or separate images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sheet: Option<bool>,
}

/// `explore_styles` (creator): k art-direction variants of one story.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ExploreStylesArgs {
    #[serde(default)]
    pub story: Value,
    /// 2–4 variants (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// (0.21) Brand colours (as in `make_video`): every variant wears them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<Value>,
}

/// `list_options` (creator): no arguments; `topic` narrows the answer.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListOptionsArgs {
    /// looks | families | music | tones | all (default all).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

/// `plan_assets` (creator): missing pictures of a job as generator prompts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PlanAssetsArgs {
    #[serde(default)]
    pub job: String,
}

/// Operator tools that act on one job (render_frame, inspect_frame, qa_report).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobFrameArgs {
    #[serde(default)]
    pub job: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_s: Option<f64>,
}

/// Operator tools that act on one file (ingest_assets, matte, music_index,
/// sfx_index, reference_evidence): a path inside the repository or `output/`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileArgs {
    #[serde(default)]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Value>,
}

/// `apply_scene_patch` (operator, `--allow-scene-edits` only).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ScenePatchArgs {
    #[serde(default)]
    pub job: String,
    /// JSON merge patch on the job's MotionScene; the result must pass
    /// `validate` and QA.
    #[serde(default)]
    pub patch: Value,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lenient_parsing() {
        let a: MakeVideoArgs = serde_json::from_value(json!({
            "story": {"beats": []}, "style": "cinematic", "mode": "check", "bogus": 1
        }))
        .unwrap();
        assert_eq!(a.mode, Mode::Check);
        assert!(!a.strict);
        let c: BeatChange =
            serde_json::from_value(json!({"beat": 2, "show": "New title"})).unwrap();
        assert_eq!(c.beat, Some(2));
        assert_eq!(c.set["show"], "New title");
        let g: GetVideoArgs = serde_json::from_value(json!({"job": "j_0123456789"})).unwrap();
        assert_eq!(g.wait_s, 0);
    }
}
