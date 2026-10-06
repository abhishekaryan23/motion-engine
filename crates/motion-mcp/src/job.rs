//! Jobs (plan §6): hash-addressed, idempotent, stored under `output/jobs/<id>/`.
//!
//! The id is `"j_"` + the first 10 hex characters of sha256 over the canonical
//! JSON of [`JobKey`]: the normalised input after auto-fix, the engine version
//! and the voice model. Same input → same id → same files, no re-render.

use std::path::{Path, PathBuf};

use motion_core::audio::MusicWord;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::reply::{Fix, Progress, Qa};

// Files of a job directory.
/// The tool arguments as received, after normalisation ([`StoredRequest`]).
pub const REQUEST_JSON: &str = "request.json";
pub const INTENT_JSON: &str = "intent.json";
pub const STYLE_JSON: &str = "style.json";
/// The job's user-image manifest, when the story uses the user's images.
pub const MANIFEST_JSON: &str = "assets.manifest.json";
pub const STATUS_JSON: &str = "status.json";
/// Full engine output; never returned in full.
pub const LOG_TXT: &str = "log.txt";
pub const VIDEO_MP4: &str = "video.mp4";
pub const PREVIEW_MP4: &str = "preview_720p.mp4";
/// `motion-engine reel -o` directory (scene, speech, audio plan, frames dir).
pub const REEL_DIR: &str = "reel";
/// Shared cache for measured / matted user images, keyed by file sha256.
/// Lives inside the jobs root so the server writes nowhere else.
pub const IMAGE_CACHE_DIR: &str = "_cache/images";

/// A job id: `j_` + 10 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JobId(String);

impl JobId {
    /// Parse a model-supplied id. Rejects anything that is not exactly
    /// `j_[0-9a-f]{10}` (so ids are always safe path components).
    pub fn parse(s: &str) -> Result<JobId, Fix> {
        let s = s.trim();
        let ok = s.len() == 12
            && s.starts_with("j_")
            && s[2..]
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase());
        if ok {
            Ok(JobId(s.to_string()))
        } else {
            Err(Fix::new(None, "job", format!("'{s}' is not a job id"))
                .otherwise("use the job value from an earlier reply, like j_3f9a1c27b0"))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `<jobs root>/<id>`.
    pub fn dir(&self, jobs_root: &Path) -> PathBuf {
        jobs_root.join(&self.0)
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a job id is computed from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JobKey {
    /// The normalised input after auto-fix: `{"intent", "style", "options",
    /// "assets"}` where `assets` holds the user images' content hashes and
    /// roles (never absolute paths), so a changed image is a new job.
    pub input: Value,
    /// [`crate::profile::engine_version`].
    pub engine: String,
    /// Voice model (`reel --tts-model`), e.g. `auto`.
    pub voice: String,
}

/// JSON with object keys sorted at every level and no whitespace. Independent
/// of serde_json's `preserve_order` feature.
pub fn canonical_json(v: &Value) -> String {
    fn write(v: &Value, out: &mut String) {
        match v {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                out.push('{');
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String((*k).clone()).to_string());
                    out.push(':');
                    write(&map[*k], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut out = String::new();
    write(v, &mut out);
    out
}

/// The id for `key`.
pub fn job_id(key: &JobKey) -> JobId {
    let value = serde_json::to_value(key).expect("JobKey serializes");
    let digest = Sha256::digest(canonical_json(&value).as_bytes());
    let hex: String = digest.iter().take(5).map(|b| format!("{b:02x}")).collect();
    JobId(format!("j_{hex}"))
}

/// Lifecycle: queued → running → done | failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
}

/// Pipeline stages, in order (the `reel` steps plus the server's own).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Check,
    Voice,
    Music,
    Compile,
    Render,
    Qa,
    Preview,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Check => "check",
            Stage::Voice => "voice",
            Stage::Music => "music",
            Stage::Compile => "compile",
            Stage::Render => "render",
            Stage::Qa => "qa",
            Stage::Preview => "preview",
        }
    }

    /// Coarse overall percent when a stage starts (render dominates).
    pub fn start_percent(self) -> u8 {
        match self {
            Stage::Check => 0,
            Stage::Voice => 5,
            Stage::Music => 20,
            Stage::Compile => 22,
            Stage::Render => 30,
            Stage::Qa => 90,
            Stage::Preview => 95,
        }
    }

    pub fn progress(self) -> Progress {
        Progress {
            stage: self.name().to_string(),
            percent: self.start_percent(),
        }
    }
}

/// `status.json`: the durable state of a job (what `get_video` reads).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobStatus {
    pub job: JobId,
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<Stage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<u8>,
    /// Repository-relative paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qa: Option<Qa>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// One-line reason with `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The job this one revises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<JobId>,
    pub created_unix: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_unix: Option<u64>,
    /// The pid of the server that queued / runs the job (cleared when it
    /// finishes). Several servers (e.g. a weak and a creator one) share one
    /// jobs folder: a live owner's job is waited on, never re-run or pruned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_pid: Option<u32>,
}

/// Which story form a request carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    /// A lite story ([`crate::lite::LiteStory`]).
    Lite,
    /// A full CreativeIntent v0.2 (creator / operator, or strong weak-profile clients).
    Intent,
}

/// `request.json`: the normalised request, enough to re-create the job and to
/// apply `revise_video` changes to it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredRequest {
    pub profile: crate::profile::Profile,
    pub kind: StoryKind,
    /// The story after lenient parsing and auto-fix (lite or intent JSON).
    pub story: Value,
    /// The StyleProfile JSON (a tone word is stored as `{"tone": …}`).
    pub style: Value,
    /// vertical | square | wide
    pub format: String,
    /// The user-image folder (`assets`), if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assets: Option<String>,
    /// Production options (creator / operator; `{}` for weak).
    #[serde(default)]
    pub options: crate::args::ProductionOptions,
    /// (0.23) Take number 0-99 ("another version" of the same story); 0 is
    /// the default take and is not written, so a take-0 `request.json` is
    /// exactly what it was before takes existed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub take: u64,
    /// (0.23) The music mood word (`auto` = read it from the story); `auto`
    /// is the default and is not written, so a request without a music word
    /// has exactly the `request.json` it had before the word existed.
    #[serde(default, skip_serializing_if = "is_auto_music")]
    pub music: MusicWord,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn is_auto_music(w: &MusicWord) -> bool {
    *w == MusicWord::Auto
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key(input: Value) -> JobKey {
        JobKey {
            input,
            engine: "v0.20-1-gabc+0123456789ab".into(),
            voice: "auto".into(),
        }
    }

    #[test]
    fn ids_are_stable_and_order_free() {
        let a = job_id(&key(json!({"intent": {"b": 1, "a": [1, 2]}, "style": {}})));
        let b = job_id(&key(json!({"style": {}, "intent": {"a": [1, 2], "b": 1}})));
        assert_eq!(a, b);
        assert_ne!(
            a,
            job_id(&key(json!({"intent": {"b": 2, "a": [1, 2]}, "style": {}})))
        );
        assert!(JobId::parse(a.as_str()).is_ok());
        assert_eq!(a.as_str().len(), 12);
        assert_eq!(
            canonical_json(&json!({"z": 1.5, "a": {"y": null, "x": "q\""}})),
            r#"{"a":{"x":"q\"","y":null},"z":1.5}"#
        );
    }

    #[test]
    fn job_ids_are_safe_path_components() {
        for bad in [
            "../etc",
            "j_123",
            "j_ABCDEF0123",
            "j_0123456789/",
            "j_012345678g",
        ] {
            assert!(JobId::parse(bad).is_err(), "{bad}");
        }
        assert!(JobId::parse(" j_0123456789 ").is_ok());
    }
}
