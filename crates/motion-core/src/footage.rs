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
    /// Sprite-sequence directory, relative to the asset root.
    pub path: String,
    pub frame_count: u32,
    pub fps: f64,
    #[serde(default)]
    pub in_frame: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_frame: Option<u32>,
    /// Loops wrap (screen inserts); clips play once.
    pub mode: crate::scene::SpriteMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FootageRole {
    /// Full-frame muted clip for the beat, then back to the persistent anchor
    /// scene (a numbered list that grows by one item per interstitial beat).
    Interstitial,
    /// Shown inside the beat's host object's catalog `screen_box`.
    ScreenInsert,
}

impl FootagePlan {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
    pub fn for_beat(&self, beat: usize, role: FootageRole) -> Option<&FootageClip> {
        self.clips.iter().find(|c| c.beat == beat && c.role == role)
    }
}
