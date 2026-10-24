//! (0.11) Footage plan: an OPERATOR file (`<name>.footage.json`) beside the
//! intent, like speech or music. Weak models never author footage; clips are
//! extracted outside the engine into sprite-sequence directories
//! (scripts/extract_sprite.sh). See docs/FOOTAGE.md.
//!
//! FROZEN: types and field names.

use serde::{Deserialize, Serialize};

pub const FOOTAGE_VERSION: &str = "0.1";
/// Footage clips play at most this long (seconds).
pub const MAX_CLIP_SECONDS: f64 = 6.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FootagePlan {
    pub version: String,
    pub clips: Vec<FootageClip>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FootageClip {
    /// Beat index (0-based) this clip serves.
    pub beat: usize,
    /// How the clip is used.
    pub role: FootageRole,
    /// Sprite-sequence directory, relative to the asset root; for `plate` the
    /// video file (absolute, or relative to the asset root).
    pub path: String,
    /// Native frame count. Plates: probed by the CLI when 0/absent.
    #[serde(default)]
    pub frame_count: u32,
    /// Native fps. Plates: probed by the CLI when 0/absent.
    #[serde(default)]
    pub fps: f64,
    #[serde(default)]
    pub in_frame: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_frame: Option<u32>,
    /// Loops wrap (screen inserts); clips play once.
    #[serde(default = "default_mode")]
    pub mode: crate::scene::SpriteMode,
    /// (0.24 plate) In point in seconds of the video (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_s: Option<f64>,
    /// (0.24 plate) Out point in seconds (default: the clip's end).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_s: Option<f64>,
}

fn default_mode() -> crate::scene::SpriteMode {
    crate::scene::SpriteMode::Once
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FootageRole {
    /// Full-frame muted clip for the beat, then back to the persistent anchor
    /// scene (a numbered list that grows by one item per interstitial beat).
    Interstitial,
    /// Shown inside the beat's host object's catalog `screen_box`.
    ScreenInsert,
    /// (0.24) Full-bleed muted MP4/MOV/WEBM backdrop under the beat's type (`beat` is 1-based),
    /// treated to the look, with a scrim for text contrast (compiler/plates.rs).
    Plate,
}

impl FootagePlan {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
    pub fn for_beat(&self, beat: usize, role: FootageRole) -> Option<&FootageClip> {
        self.clips.iter().find(|c| c.beat == beat && c.role == role)
    }
}
