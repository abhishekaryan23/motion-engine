//! StyleProfile v0.1 — high-level visual language consumed by the MotionCompiler.
//!
//! MotionEngine does not care where a profile comes from (hand-written, a model,
//! or a future reference-video interpreter). Every field is a small enum.
//!
//! Doc comments here are the public descriptions in
//! `schema/style-profile-v0.1.schema.json` (generated; see `tests/public_schema.rs`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Visual language for a piece. Every field is optional; `{}` is a valid profile
/// (warm paper, grotesk + serif typography, layered depth, slow push, subtle print, signal red).
///
/// The five taste fields (`tone`, `polarity`, `temperature`, `temperament`, `density`) are the
/// recommended way to steer a piece: the engine's TasteDirector turns them into a complete,
/// coherent design system (palette, background, typography, material, image treatment, motion
/// character, transition character, composition rhythm, scale contrast). The remaining fields are
/// explicit overrides; a field left at its default never overrides a taste decision.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StyleProfile {
    /// Overall design character. `auto` keeps the classic warm editorial collage when no other
    /// taste field is set; with any other taste field set, `auto` means `editorial`.
    #[serde(default)]
    pub tone: Tone,
    /// Light or dark design. `auto` lets the tone decide.
    #[serde(default)]
    pub polarity: Polarity,
    /// Color temperature. `auto` lets the tone decide.
    #[serde(default)]
    pub temperature: Temperature,
    /// How motion broadly feels. `auto` lets the tone decide.
    #[serde(default)]
    pub temperament: Temperament,
    /// How much visual information a frame tends to hold. `auto` lets the tone decide.
    /// Every density keeps one clear primary focus and readable text.
    #[serde(default)]
    pub density: Density,
    /// Overall style family. Accepted but currently has no visible effect.
    #[serde(default)]
    pub family: StyleFamily,
    /// Background material.
    #[serde(default)]
    pub material: MaterialStyle,
    /// Typeface pairing.
    #[serde(default)]
    pub typography_style: TypographyStyle,
    /// Flat or layered collage depth.
    #[serde(default)]
    pub depth: Depth,
    /// Camera behavior during each beat.
    #[serde(default)]
    pub camera_style: CameraStyle,
    /// How things move: the engine's motion behavior family. `auto` (the default)
    /// lets the engine choose per beat from what the beat communicates.
    #[serde(default)]
    pub motion_language: MotionLanguage,
    /// Amount of print texture (grain, paper fibre).
    #[serde(default)]
    pub texture_style: TextureStyle,
    /// The single accent color used for highlights, rules and transitions.
    #[serde(default)]
    pub accent_role: AccentRole,
    /// Non-negative integer that varies the procedural texture patterns only. Same seed, same result.
    #[serde(default)]
    #[schemars(extend("maximum" = 18_446_744_073_709_551_615u64))]
    pub seed: u64,
    /// Brand colours for this piece. They replace the look's colours: `primary` for highlights,
    /// rules, transitions and key figures, `secondary` for large colour fields, `background`
    /// for the ground. Text stays readable whatever the colours: text that would not be
    /// readable on the background is replaced by near-black or near-white. Omit it to let the
    /// engine choose the colours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brand: Option<BrandColors>,
}

/// Brand colours, each written as hex `#RRGGBB` (or `#RGB`). Every field is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BrandColors {
    /// The main brand colour: highlights, rules, transitions and key figures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("pattern" = "^#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})$"))]
    pub primary: Option<String>,
    /// A second brand colour for large colour fields and supporting accents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("pattern" = "^#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})$"))]
    pub secondary: Option<String>,
    /// The background colour. Omit it to keep the look's background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("pattern" = "^#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})$"))]
    pub background: Option<String>,
    /// The text colour. Used only where it is readable on the background.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("pattern" = "^#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})$"))]
    pub text: Option<String>,
}

impl BrandColors {
    /// True when no colour is set.
    pub fn is_empty(&self) -> bool {
        self.primary.is_none()
            && self.secondary.is_none()
            && self.background.is_none()
            && self.text.is_none()
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StyleFamily {
    /// Editorial collage (the default). Currently no visible effect.
    #[default]
    EditorialCollage,
    /// Minimal. Currently no visible effect.
    Minimal,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaterialStyle {
    /// Warm uncoated paper (textured unless texture_style is "none"). The default.
    #[default]
    Paper,
    /// Plain light background without paper texture.
    Flat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TypographyStyle {
    /// Heavy grotesk headlines, condensed figures, an italic serif voice, monospaced labels. The default.
    #[default]
    GroteskSerif,
    /// Condensed display headlines and figures, monospaced body text and labels.
    CondensedMono,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    /// No halftone print patches, no background parallax.
    Flat,
    /// Collage depth: halftone print patches behind cards and a slowly drifting background word. The default.
    #[default]
    Layered,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CameraStyle {
    /// No camera movement.
    Static,
    /// Gentle continuous push-in during each beat. The default.
    #[default]
    SlowPush,
    /// A barely perceptible push-in.
    Drift,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MotionLanguage {
    /// The engine picks a motion language per beat from its meaning (numbers, emphasis, explanation). The default.
    #[default]
    Auto,
    /// Clean, restrained motion: small moves, fades and reveals, gentle settles, no bounce.
    Minimal,
    /// Kinetic typography: words cascade in, key words are punched or enlarged, text replaces text.
    Kinetic,
    /// Multi-plane depth: background, subject and foreground move at different rates as the camera pushes and drifts.
    Parallax,
    /// Sequential: lines, words and items arrive one after another in irregular, readable rhythms.
    Sequential,
    /// Data motion: numbers count up and quantities grow as bars, progress and lines.
    Data,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextureStyle {
    /// Clean: no grain and untextured paper.
    None,
    /// Light film grain and soft paper texture. The default.
    #[default]
    SubtlePrint,
    /// Strong grain and paper texture.
    HeavyPrint,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccentRole {
    /// Signal red. The default.
    #[default]
    SignalRed,
    /// Cobalt blue.
    Cobalt,
    /// Acid yellow-green (dark text on accent).
    Acid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    /// The classic warm editorial collage (the default).
    #[default]
    Auto,
    /// Editorial magazine design: serif/sans contrast, generous space, measured pacing.
    Editorial,
    /// Technical/data design: structured grid, condensed and monospaced type, precise motion.
    Technical,
    /// Playful print design: bold type, large graphic color fields, energetic motion.
    Playful,
    /// Music, sports and street culture: the picture owns the frame (big cutouts, tape, stencil punchwords), hard cuts, camera shake on the hits. Best with object or figure pictures.
    Street,
    /// Investigative explainer (Vox style): evidence documents slide, get stamped and highlighted; measured camera drift, film grain. Best for facts, money, history and science stories.
    Documentary,
    /// Hype reel or opener: words and pictures slam into frame in fast cuts timed to the voice. Best for short, punchy scripts.
    Hype,
    /// Creator / brand storytelling: one big cutout (best a person) in front of a bold colour disc on a clean studio ground; the spoken words appear large behind the subject as they are said.
    Studio,
    /// Cinematic 3D: the story is staged in depth and a moving camera flies through it (parallax, dolly, orbit, rack focus, fly-through transitions). Best for big ideas: technology, science, space, the future.
    Cinematic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Polarity {
    /// Chosen by the tone (the default).
    #[default]
    Auto,
    /// Dark text on a light ground.
    Light,
    /// Light text on a dark ground.
    Dark,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Temperature {
    /// Chosen by the tone (the default).
    #[default]
    Auto,
    /// Warm neutrals and warm accents.
    Warm,
    /// Cool neutrals and cool accents.
    Cool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Temperament {
    /// Chosen by the tone (the default).
    #[default]
    Auto,
    /// Quiet, controlled motion: small moves, soft settles, no bounce.
    Restrained,
    /// Balanced editorial motion.
    Balanced,
    /// Energetic motion: bigger moves, visible anticipation, lively settles.
    Energetic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// Chosen by the tone (the default).
    #[default]
    Auto,
    /// Few elements, lots of negative space, quiet background.
    Sparse,
    /// A primary focus with some supporting detail.
    Balanced,
    /// Many supporting layers and annotations around one clear primary focus.
    Dense,
}

impl StyleProfile {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
}
