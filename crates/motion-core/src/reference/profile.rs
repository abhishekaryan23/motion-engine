//! ReferenceStyleProfile v0.1 — the external interpreter's answer (0.7).
//!
//! Doc comments here are the public descriptions in
//! `schema/reference-style-profile-v0.1.schema.json` (generated; see
//! `tests/public_schema.rs`).
//!
//! The profile describes reusable STYLE PRINCIPLES of a reference video in a
//! small closed vocabulary. It has no free-text field anywhere: it cannot carry
//! source wording, logos, identities, images, coordinates or timings.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The current profile version (0.7.1: adds `visual_language`).
pub const REFERENCE_PROFILE_VERSION: &str = "0.2";
/// The original 0.7 version: still accepted; it cannot carry `visual_language`.
pub const REFERENCE_PROFILE_VERSION_V0_1: &str = "0.1";

/// Reusable style principles observed in a reference video. Every dimension is optional:
/// omit it, or give `"value": null`, when the reference does not show it clearly. Values are
/// tendencies of the whole piece, never instructions for a particular frame.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceStyleProfile {
    /// "0.2" (current) or "0.1" (accepted; cannot carry visual_language).
    pub version: String,
    /// The `reference_fingerprint` of the analysis bundle this profile was written from
    /// (copy it from the request). Optional, but required for provenance checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_fingerprint: Option<String>,
    /// Overall design character.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Trait<RefTone>>,
    /// Light or dark design. `mixed` when both are substantial.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub polarity: Option<Trait<RefPolarity>>,
    /// Color temperature of grounds and dominant colors.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<Trait<RefTemperature>>,
    /// Tonal contrast between grounds and content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contrast: Option<Trait<RefLevel>>,
    /// How color is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette_character: Option<Trait<PaletteCharacter>>,
    /// Up to 8 approximate colors copied from the deterministic evidence (`color.palette`).
    /// Evidence only: the engine maps them into its own curated palette families.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approximate_palette_evidence: Vec<PaletteEvidenceSwatch>,
    /// What typically sits behind the content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_character: Option<Trait<BackgroundCharacter>>,
    /// Dominant typographic voice of headlines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography_character: Option<Trait<TypographyCharacter>>,
    /// How strongly type sizes/weights/styles contrast with each other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography_contrast: Option<Trait<RefLevel>>,
    /// Surface finish of grounds and graphic elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material_character: Option<Trait<MaterialCharacter>>,
    /// How photographs or illustrations are treated. `none` when the reference has no images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_treatment: Option<Trait<ImageTreatmentCharacter>>,
    /// How much visual information a typical frame holds around its main focus.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_density: Option<Trait<RefDensity>>,
    /// How often the composition meaningfully changes (from visual change, never audio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_rhythm: Option<Trait<RefRhythm>>,
    /// How elements generally move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_temperament: Option<Trait<RefTemperament>>,
    /// How one composition hands off to the next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition_character: Option<Trait<RefTransition>>,
    /// Size contrast between the dominant element and supporting elements.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_contrast: Option<Trait<RefScale>>,
    /// How present/active the foreground, midground and background layers are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer_activity: Option<Trait<RefLayerActivity>>,
    /// Visible traits the engine cannot render. Recorded, never imitated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported_reference_traits: Vec<UnsupportedTrait>,
    /// (v0.2) How the reference constructs its scenes: medium, imagery use, composition families,
    /// visual explanation. Patterns only, never copied scenes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_language: Option<super::visual_language::VisualLanguageProfile>,
}

/// One inferred dimension: a value from the closed vocabulary (or null = unknown), how sure
/// the interpreter is, and optionally which evidence supported it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trait<T> {
    /// The observed tendency, or null when unknown.
    pub value: Option<T>,
    /// 0..1. Values below 0.5 are recorded but not applied.
    pub confidence: f32,
    /// Optional provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceLinks>,
}

impl<T> Trait<T> {
    pub fn new(value: T, confidence: f32) -> Self {
        Trait {
            value: Some(value),
            confidence,
            evidence: None,
        }
    }
}

/// Provenance: which samples, time ranges and deterministic metrics supported an inference.
/// Identifiers only; no reasoning text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EvidenceLinks {
    /// Sample ids from the request (e.g. "s03").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub samples: Vec<String>,
    /// Metric keys from the evidence (e.g. "temporal.changes_per_10s").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metrics: Vec<String>,
    /// [start, end] in seconds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub time_ranges: Vec<[f64; 2]>,
}

/// An approximate color from the evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PaletteEvidenceSwatch {
    /// "#RRGGBB".
    pub hex: String,
    /// Share of the frame area, 0..1.
    pub prevalence: f32,
}

/// Overall design character.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefTone {
    /// Magazine-like: typographic hierarchy, restraint, print sensibility.
    Editorial,
    /// Precise, systematic, data/UI-like: grids, mono or condensed type.
    Technical,
    /// Bold graphic, colorful, bouncy.
    Playful,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefPolarity {
    /// Light grounds, dark content.
    Light,
    /// Dark grounds, light content.
    Dark,
    /// Both light and dark compositions are substantial.
    Mixed,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefTemperature {
    Warm,
    Cool,
    /// Grey / balanced.
    Neutral,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefLevel {
    Low,
    Medium,
    High,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PaletteCharacter {
    /// Essentially one hue or greyscale.
    Monochrome,
    /// Desaturated colors throughout.
    Muted,
    /// Neutral grounds with one sparing accent.
    RestrainedAccent,
    /// Neutral grounds with strong saturated accents.
    Vivid,
    /// Several saturated colors used as large fields.
    Multicolor,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundCharacter {
    /// Plain flat ground.
    Flat,
    /// Paper-like ground with fibre/grain.
    Paper,
    /// Smooth gradients or vignettes.
    Gradient,
    /// Measurement grids, guides, technical lines.
    Grid,
    /// Large flat graphic color fields that recompose.
    GraphicFields,
    /// Photographs or footage fill the background.
    Photographic,
    /// Visible texture (noise, halftone, pattern) over a ground.
    Textured,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TypographyCharacter {
    /// Heavy grotesk sans headlines.
    Grotesk,
    /// Geometric sans (round, even strokes).
    GeometricSans,
    /// Humanist sans (calligraphic proportions).
    HumanistSans,
    /// Tall condensed display type.
    Condensed,
    /// Monospaced / technical labels dominate.
    MonoTechnical,
    /// Serif display headlines.
    SerifEditorial,
    /// Very heavy poster display type.
    PosterDisplay,
    /// Script lettering.
    Script,
    /// Hand-drawn lettering.
    Handwritten,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MaterialCharacter {
    /// Uncoated paper, print grain.
    Paper,
    /// Clean flat vector surfaces.
    Flat,
    /// Emissive screen / UI surfaces.
    Screen,
    /// Coated print: flat saturated ink, fine grain.
    Print,
    /// Glossy highlights and reflections.
    Glossy,
    /// Chrome or metal.
    Metallic,
    /// Rendered 3D surfaces and lighting.
    Rendered3d,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ImageTreatmentCharacter {
    /// The reference shows no photographs or illustrations.
    None,
    /// Images shown as they are.
    Natural,
    /// Black and white / single ink.
    Monochrome,
    /// Two-ink duotone.
    Duotone,
    /// Desaturated, low-contrast documentary look.
    Muted,
    /// Cut-out subjects with paper edges and shadows.
    Cutout,
    /// Bold cut-outs on flat color fields.
    PrintCutout,
    /// Crushed blacks, strong contrast.
    HighContrast,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefDensity {
    Sparse,
    Balanced,
    Dense,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefRhythm {
    /// Long holds, few slow changes.
    SlowBreathing,
    /// Steady editorial pace: read, then change.
    MeasuredEditorial,
    /// Compositions keep building within a scene.
    Progressive,
    /// Frequent changes.
    Active,
    /// Constant rapid change.
    HighFrequency,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefTemperament {
    /// Small, slow, soft moves.
    Restrained,
    /// Smooth continuous easing.
    Fluid,
    /// Quick decisive arrivals, short settles.
    Snappy,
    /// Big fast moves, visible overshoot.
    Energetic,
    /// Slow dramatic camera-like moves.
    Cinematic,
    /// Bouncy, elastic.
    Playful,
    /// Linear, stepwise, machine-like.
    Mechanical,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefTransition {
    /// Soft crossfades.
    Subtle,
    /// Crossfade/lift with occasional graphic accents.
    Editorial,
    /// Straight-edged panel wipes and lateral handoffs.
    Geometric,
    /// Fast sweeps, punches, energetic wipes.
    Kinetic,
    /// Elements continue from one composition into the next.
    Continuous,
    /// Hard cuts.
    Hard,
    /// Slow dissolves, camera-driven handoffs.
    Cinematic,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefScale {
    Subtle,
    Moderate,
    Large,
    Dramatic,
}

/// Presence of the foreground (main content).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefPresence {
    Recessive,
    Balanced,
    Dominant,
}

/// Activity of a supporting layer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefActivity {
    Still,
    Quiet,
    /// Regular, systematic motion (grids, ticks).
    Structured,
    Active,
}

/// Per-layer activity; omit or null any part that is unclear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefLayerActivity {
    #[serde(default)]
    pub foreground: Option<RefPresence>,
    #[serde(default)]
    pub midground: Option<RefActivity>,
    #[serde(default)]
    pub background: Option<RefActivity>,
}

/// A visible trait the engine cannot render.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedTrait {
    #[serde(rename = "trait")]
    pub kind: UnsupportedTraitKind,
    /// 0..1.
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceLinks>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedTraitKind {
    /// True 3D geometry or 3D camera moves.
    True3d,
    /// Chrome, metal or glossy reflective surfaces.
    ChromeOrMetallic,
    /// Particle fields, sparks, dust.
    ParticleField,
    /// Liquid or smoke simulation.
    FluidSimulation,
    /// Animated characters.
    CharacterAnimation,
    /// Frame-by-frame drawn animation.
    FrameByFrameIllustration,
    /// Live-action video footage.
    LiveActionFootage,
    /// Photographs or footage filling the background.
    PhotographicBackground,
    /// Recorded app/UI screens.
    UiScreenRecording,
    /// Phones, laptops or other device mockups.
    DeviceMockup,
    /// Smooth gradients, vignettes or glows as grounds.
    GradientBackground,
    /// Glitch, datamosh, RGB split.
    Glitch,
    /// VHS, film damage, light leaks.
    FilmOrVhsArtifacts,
    /// Script or hand-drawn lettering.
    HandLettering,
    /// Hand-drawn illustration elements (doodles, scribbles).
    HandDrawnElements,
    /// Motion blur or speed ramps.
    MotionBlur,
    /// Anything else the engine cannot render.
    Other,
}

/// Every dimension name, in report order.
pub const DIMENSIONS: &[&str] = &[
    "tone",
    "polarity",
    "temperature",
    "contrast",
    "palette_character",
    "background_character",
    "typography_character",
    "typography_contrast",
    "material_character",
    "image_treatment",
    "visual_density",
    "composition_rhythm",
    "motion_temperament",
    "transition_character",
    "scale_contrast",
    "layer_activity",
];
