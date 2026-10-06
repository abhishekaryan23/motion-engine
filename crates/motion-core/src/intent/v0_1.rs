//! CreativeIntent v0.1 — FROZEN legacy contract. Do not edit.
//!
//! These types generate `schema/creative-intent-v0.1.schema.json` and parse
//! documents with `"version": "0.1"`, which are then converted losslessly into
//! the current (v0.2) types. Keeping them frozen guarantees v0.1 behavior
//! cannot drift as v0.2 evolves.
//!
//! Original header:
//! CreativeIntent v0.1 — the small, model-facing semantic representation.
//!
//! A weak model describes WHAT each beat should communicate. It never supplies
//! coordinates, sizes, timings, easing or z-order: those belong to the
//! MotionCompiler. Keep this vocabulary deliberately small.
//!
//! Doc comments on these types are the public descriptions in
//! `schema/creative-intent-v0.1.schema.json` (generated; see
//! `tests/public_schema.rs`). Keep them semantic and implementation-free.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const INTENT_VERSION: &str = "0.1";

/// A short motion-graphics story: a title, an output format and 1+ beats.
/// Describes WHAT to communicate; MotionEngine decides layout, animation and rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreativeIntent {
    /// Contract version. Must be exactly "0.1".
    #[schemars(extend("const" = "0.1"))]
    pub version: String,
    /// Short name of the piece. Also used as the render output folder and video
    /// file name, so prefer letters, digits, '_' and '-'.
    pub title: String,
    /// Output aspect ratio. Defaults to "vertical".
    #[serde(default)]
    pub format: Format,
    /// The story, in order. One beat = one idea. At least one beat is required.
    #[schemars(length(min = 1))]
    pub beats: Vec<Beat>,
}

/// Output aspect ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// 9:16 portrait (1080 x 1920) for shorts / reels. The primary, best-supported format.
    #[default]
    Vertical,
    /// 1:1 (1080 x 1080).
    Square,
    /// 16:9 (1920 x 1080).
    Landscape,
}

/// One idea on screen for a few seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Beat {
    /// What this beat is for.
    pub purpose: Purpose,
    /// The one sentence this beat communicates, as the viewer should read it. Keep it short (about 3–8 words).
    pub statement: String,
    /// The main subject of the beat.
    pub primary: Subject,
    /// An optional second subject (the other side of a comparison, a supporting phrase or object).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<Subject>,
    /// How primary and secondary relate. Meaningful for "compare" and "contrast" beats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relationship: Option<Relationship>,
    /// Pace and intensity of the beat. Defaults to "building".
    #[serde(default)]
    pub energy: Energy,
    /// Whether a subject of this beat stays on screen into the next beat. Defaults to "none".
    #[serde(default)]
    pub continuity: Continuity,
    /// Optional single word shown very large and faint in the background.
    /// Defaults to the primary subject's meaning (or its value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyword: Option<String>,
}

/// A thing the beat is about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(extend("allOf" = [{
    "if": { "properties": { "kind": { "const": "object" } }, "required": ["kind"] },
    "then": {
        "required": ["asset"],
        "properties": { "asset": { "type": "string", "enum": ["shopping_basket"] } }
    }
}]))]
pub struct Subject {
    /// What kind of subject this is.
    pub kind: SubjectKind,
    /// Text shown for the subject exactly as written: the phrase, or the number with its
    /// unit/symbol (e.g. "120", "+40%", "3.5 km"). For objects: a short figure stamped next to the object
    /// in compare/contrast beats (unused for objects elsewhere).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// What the subject stands for, in 1–3 words (e.g. "passengers"). Shown as a small label and
    /// used as the default background keyword. Shown instead of `value` when `value` is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meaning: Option<String>,
    /// Name of a built-in object asset. Required when kind is "object"; ignored otherwise.
    /// Available assets: "shopping_basket".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset: Option<String>,
}

/// Kind of subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    /// Words (a name, a claim, a short phrase).
    Phrase,
    /// A figure: count, amount, percentage, measurement.
    Number,
    /// A pictured thing from the built-in asset library (requires `asset`).
    Object,
}

/// What a beat is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Purpose {
    /// Make one idea land: a bold headline plus a hero subject.
    Emphasize,
    /// Put two subjects side by side neutrally. Default relationship: "separate".
    Compare,
    /// Put two subjects in tension. Default relationship: "compress".
    Contrast,
    /// Deliver a payoff or conclusion: the primary (usually a number) is unveiled as the answer.
    Reveal,
    /// Explain something: the primary becomes the headline and the statement is read as body text.
    Explain,
}

/// How the primary and secondary subjects relate (used by compare / contrast beats).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    /// The secondary gains weight or size over the beat.
    Grow,
    /// The secondary creates pressure: the space and presence of the primary shrink while the secondary looms larger.
    Compress,
    /// The two subjects pull apart, with a visible divide between them.
    Separate,
    /// The secondary takes over from the primary, which recedes.
    Replace,
    /// The primary persists steadily; also keeps the primary on screen into the next beat (like continuity "carry_primary").
    Carry,
}

/// Pace and intensity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Energy {
    /// Unhurried, gentle, reflective.
    Calm,
    /// Steady editorial momentum. The default.
    #[default]
    Building,
    /// Punchy and fast. A "reveal" beat with impact energy that follows another beat arrives with a bold
    /// full-screen accent-color transition.
    Impact,
}

/// Whether a subject of this beat stays on screen into the next beat.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Continuity {
    /// Nothing carries over. The default.
    #[default]
    None,
    /// The primary subject stays on screen and travels into the next beat.
    CarryPrimary,
    /// The secondary subject stays on screen and travels into the next beat.
    CarrySecondary,
}

impl CreativeIntent {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}

impl Subject {
    /// Text to display for this subject, if any.
    pub fn display_text(&self) -> Option<&str> {
        self.value.as_deref().or(self.meaning.as_deref())
    }
}
