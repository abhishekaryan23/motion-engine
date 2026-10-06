//! ReferenceStyleProfile v0.1 — FROZEN legacy contract. Do not edit.
//!
//! This is the top-level struct exactly as it was before `visual_language`
//! existed. It generates `schema/reference-style-profile-v0.1.schema.json` and
//! strictly parses documents with `"version": "0.1"`, which are then converted
//! losslessly into the current (v0.2) [`super::profile::ReferenceStyleProfile`]
//! (`visual_language` = None, version kept "0.1"). A v0.1 document that carries
//! `visual_language` is rejected as an unknown field. Only the top-level struct
//! is copied; trait and enum types are shared with the current profile.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::profile::{
    BackgroundCharacter, ImageTreatmentCharacter, MaterialCharacter, PaletteCharacter,
    PaletteEvidenceSwatch, RefDensity, RefLayerActivity, RefLevel, RefPolarity, RefRhythm,
    RefScale, RefTemperament, RefTemperature, RefTone, RefTransition, Trait, TypographyCharacter,
    UnsupportedTrait,
};

/// Reusable style principles observed in a reference video. Every dimension is optional:
/// omit it, or give `"value": null`, when the reference does not show it clearly. Values are
/// tendencies of the whole piece, never instructions for a particular frame.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceStyleProfile {
    /// Must be "0.1".
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
}

impl From<ReferenceStyleProfile> for super::profile::ReferenceStyleProfile {
    fn from(p: ReferenceStyleProfile) -> Self {
        let ReferenceStyleProfile {
            version,
            reference_fingerprint,
            tone,
            polarity,
            temperature,
            contrast,
            palette_character,
            approximate_palette_evidence,
            background_character,
            typography_character,
            typography_contrast,
            material_character,
            image_treatment,
            visual_density,
            composition_rhythm,
            motion_temperament,
            transition_character,
            scale_contrast,
            layer_activity,
            unsupported_reference_traits,
        } = p;
        Self {
            version,
            reference_fingerprint,
            tone,
            polarity,
            temperature,
            contrast,
            palette_character,
            approximate_palette_evidence,
            background_character,
            typography_character,
            typography_contrast,
            material_character,
            image_treatment,
            visual_density,
            composition_rhythm,
            motion_temperament,
            transition_character,
            scale_contrast,
            layer_activity,
            unsupported_reference_traits,
            visual_language: None,
        }
    }
}
