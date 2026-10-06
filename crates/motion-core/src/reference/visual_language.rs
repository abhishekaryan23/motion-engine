//! ReferenceVisualLanguage (profile v0.2, 0.7.1): HOW a reference constructs
//! its scenes — what fills the frame, what role type and imagery play, which
//! composition strategies dominate, how ideas are explained visually.
//!
//! Doc comments are public descriptions in
//! `schema/reference-style-profile-v0.2.schema.json`. Like the rest of the
//! profile it is a closed vocabulary: patterns, never copied scenes (no
//! coordinates, layouts, source text, objects, characters, brands or images).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::profile::{EvidenceLinks, Trait};

/// How the reference builds its scenes. Every field is optional; omit or null what is unclear.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VisualLanguageProfile {
    /// What primarily carries the visual story.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<Trait<VisualMedium>>,
    /// How much non-typographic visual material a typical scene holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_usage: Option<Trait<AssetUsage>>,
    /// Typography versus imagery/objects in a typical frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_image_balance: Option<Trait<TypeImageBalance>>,
    /// The dominant kind of visual material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_character: Option<Trait<AssetCharacter>>,
    /// Up to 4 roles visual material typically plays, most typical first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asset_roles: Vec<RoleTendency>,
    /// Which engine composition families most resemble how the reference constructs scenes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_language: Option<CompositionLanguage>,
    /// How ideas are explained visually.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation_mode: Option<Trait<ExplanationMode>>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum VisualMedium {
    /// Typography is the only visual.
    TypeOnly,
    /// Typography leads; imagery is occasional support.
    TypeLed,
    /// Photographs or illustrations lead most scenes.
    ImageLed,
    /// Distinct objects (things, products, symbols) lead most scenes.
    ObjectLed,
    /// Diagrams, schematics, labelled shapes and processes lead.
    Diagrammatic,
    /// Layered cut-outs, textures and type assembled together.
    Collage,
    /// Screens, app interfaces and UI elements lead.
    InterfaceLed,
    /// Several of the above in comparable measure.
    Mixed,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssetUsage {
    /// No images, objects or diagrams beyond type.
    None,
    /// Occasional.
    Sparse,
    /// In about half the scenes.
    Balanced,
    /// In most scenes, often several at once.
    Dense,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TypeImageBalance {
    /// Type takes most of the frame's attention.
    TypeDominant,
    /// Type and visuals share attention.
    Balanced,
    /// Visuals take most of the attention; type supports.
    VisualDominant,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AssetCharacter {
    /// Photographs or footage stills.
    Photographic,
    /// Cut-out subjects on grounds.
    Cutout,
    /// Drawn or painted illustration.
    Illustrative,
    /// Diagrams, schematics, labelled shapes.
    Diagrammatic,
    /// Isolated hero objects.
    ObjectCentric,
    /// Assembled collage.
    Collage,
    /// Screens and UI.
    Interface,
    /// Clean generated shapes and graphic forms.
    Procedural,
    /// Several kinds in comparable measure.
    Mixed,
}

/// A role visual material typically plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleTendency {
    pub role: RefAssetRole,
    /// 0..1. Below 0.5 is recorded but not applied.
    pub confidence: f32,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefAssetRole {
    /// A person or figure carries the scene.
    HeroSubject,
    /// One object dominates the scene.
    HeroObject,
    /// Objects support the main message.
    SupportingObject,
    /// A place or backdrop fills the frame.
    Environment,
    /// Documents, screenshots or artifacts shown as proof.
    EvidenceImage,
    /// A face or portrait.
    Portrait,
    /// An object that carries the eye from one scene to the next.
    TransitionObject,
    /// A foreground element passing in front of the scene.
    ForegroundOccluder,
}

/// An engine composition family (the engine's own reusable scene strategies).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RefGrammar {
    /// One object dominates and persists.
    HeroObject,
    /// A subject image and the headline interlock as one composition.
    TypeImageInterlock,
    /// Oversized type, overlapping cards and paper layers.
    EditorialCollage,
    /// Two states share the frame side by side or stacked.
    SplitContrast,
    /// Documents and artifacts stacked as proof.
    EvidenceStack,
    /// Numbers, charts and calculations.
    DataStory,
    /// Points arrive one by one in order.
    SequentialStack,
    /// One element physically acts on another (pushes, replaces, grows).
    SpatialCauseEffect,
    /// Layered depth planes under a moving camera.
    CinematicMultiplane,
    /// Typography is the main visual.
    KineticPoster,
}

/// Preferences over engine composition families. Lists are disjoint, at most 3 entries each.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompositionLanguage {
    /// Families that most resemble the reference.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred: Vec<RefGrammar>,
    /// Families that fit somewhat.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary: Vec<RefGrammar>,
    /// Families the reference clearly does not use.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub avoid: Vec<RefGrammar>,
    /// 0..1 for the whole preference set. Below 0.5 is recorded but not applied.
    pub confidence: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<EvidenceLinks>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ExplanationMode {
    /// Shows the things themselves.
    Literal,
    /// Explains through diagrams, schematics and labelled processes.
    Diagrammatic,
    /// Uses abstract shapes or icons to stand for ideas.
    Symbolic,
    /// Uses visual metaphors.
    Metaphorical,
    /// Shows documents, data or artifacts as proof.
    EvidenceBased,
    /// Several of the above.
    Mixed,
}
