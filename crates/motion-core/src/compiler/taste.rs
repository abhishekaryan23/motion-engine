//! TasteDirector (0.6): `StyleProfile` (+ optional reference) → [`ResolvedStyleProfile`].
//!
//! Taste is an engine capability. A weak model names a handful of semantic
//! tendencies (`tone`, `polarity`, `temperature`, `temperament`, `density`);
//! the director resolves them — together, so they stay compatible — into a
//! complete design system: palette, background grammar, typography pairing,
//! material, image treatment, **and** temporal character (motion temperament,
//! transition character, composition rhythm), visual density, scale contrast
//! and layer activity.
//!
//! Boundary: everything resolved here is *semantic* (small enums + curated
//! colors). No durations, frame counts, easing constants or spring parameters:
//! the MotionCompiler (`compiler/temporal.rs`), SceneLifecycle and Timeline map
//! these tendencies to numbers. TasteDirector says HOW motion should feel; a
//! future audio/choreography system says WHEN events happen (see
//! docs/TASTE_DIRECTOR.md).
//!
//! The director is a set of small deterministic resolvers (one per dimension)
//! driven by one tone table. With every taste field `auto` it resolves to
//! [`Tone::Classic`], whose compiler mapping is the identity: pre-0.6 styles
//! compile byte-identically.

use serde::{Deserialize, Serialize};

use super::theme::Palette;
use crate::scene::Color;
use crate::style::{
    AccentRole, CameraStyle, Density, Depth, MaterialStyle, Polarity, StyleProfile, Temperament,
    Temperature, TextureStyle, Tone, TypographyStyle,
};

// ---------------------------------------------------------------------------
// Resolved vocabulary (internal, semantic)
// ---------------------------------------------------------------------------

/// The resolved design character. `Classic` = pre-0.6 warm editorial collage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedTone {
    Classic,
    Editorial,
    Technical,
    Playful,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedPolarity {
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedTemperature {
    Warm,
    Cool,
}

/// Curated palette families. Accent hue alone never changes the family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteFamily {
    WarmPaper,
    CoolPaper,
    DarkWarm,
    DarkCool,
    PrintBright,
    PrintDark,
}

/// How color use develops across the piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteTrajectory {
    /// One ground, one accent throughout.
    Steady,
    /// Graphic field colors rotate from beat to beat.
    FieldCycle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteProfile {
    pub family: PaletteFamily,
    pub polarity: ResolvedPolarity,
    pub temperature: ResolvedTemperature,
    pub trajectory: PaletteTrajectory,
    #[serde(skip)]
    pub colors: Palette,
}

/// Background system spanning the piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundGrammar {
    /// Pre-0.6: paper (or flat) ground + grain; collage depth lives in the beats.
    PaperCollage,
    /// Uncoated paper ground, grain, quiet.
    PaperField,
    /// Dark or light ground with a precise drifting measurement grid.
    TechnicalGrid,
    /// Bright ground with large graphic color fields that recompose per beat.
    PrintFields,
    /// Plain ground, nothing else.
    CleanFlat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypographyPairing {
    /// Heavy grotesk display, condensed figures, italic serif voice, mono labels (pre-0.6 default).
    GroteskSerif,
    /// Serif display headlines over a clean sans (editorial serif/sans contrast).
    SerifSans,
    /// Condensed display + monospaced body.
    CondensedMono,
    /// Black poster display, semibold sans body, bold mono labels.
    PosterBold,
    /// (0.8) Editorial emotion — trust, warmth, reflection: DM Serif display,
    /// Lato humanist body, IBM Plex Mono labels.
    HumanistSerif,
    /// (0.8) Technical emotion — precision, calm clarity: Barlow Condensed
    /// display, Barlow body, IBM Plex Mono labels, Plex Serif italic voice.
    PrecisionGrotesk,
    /// (0.8) Playful emotion — joy, approachability: Poppins display and body,
    /// Zilla Slab italic voice, IBM Plex Mono labels.
    FriendlyGeometric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialFinish {
    UncoatedPaper,
    CleanFlat,
    /// Emissive screen: no paper, no grain.
    Screen,
    /// Coated print: flat color with fine grain.
    CoatedPrint,
}

/// Image-treatment tendency; the per-role preset is still chosen by `compiler/treatment.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageTreatmentBias {
    /// Pre-0.6 mapping from material/texture.
    Classic,
    /// Paper cutouts and duotone environments.
    PaperCutout,
    /// Monochrome, tinted toward the palette.
    Monochrome,
    /// Bold cutouts on color fields.
    PrintCutout,
    /// (0.7, reference only) Images as they are.
    Natural,
    /// (0.7, reference only) Two-ink duotone for environments and objects (people are never duotoned).
    Duotone,
    /// (0.7, reference only) Desaturated documentary look.
    Muted,
}

/// How motion generally feels (never exact timing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemperamentKind {
    /// Quiet, controlled, soft settles.
    Restrained,
    /// Balanced editorial motion (pre-0.6 behavior).
    Editorial,
    /// Precise/snappy: decisive arrivals, short settles, structured stagger.
    Precise,
    /// Energetic: large moves, visible anticipation, lively overshoot.
    Energetic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettleCharacter {
    Soft,
    Standard,
    Crisp,
    Springy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaggerCharacter {
    /// Even, grid-like arrival.
    Structured,
    /// Irregular editorial rhythm.
    Editorial,
    /// Pronounced cascade.
    Cascading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraActivity {
    Still,
    Controlled,
    Standard,
    Active,
}

/// Which motion languages `auto` beats lean toward (meaning still decides first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LanguageLean {
    Classic,
    Calm,
    Structured,
    Kinetic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionTemperament {
    pub kind: TemperamentKind,
    pub amplitude: Level,
    pub settle: SettleCharacter,
    pub overshoot: Level,
    pub stagger: StaggerCharacter,
    pub camera: CameraActivity,
    pub anticipation: Level,
    pub secondary_motion: Level,
    /// Liveliness during READ (after the entrance has settled).
    pub read_activity: Level,
    pub lean: LanguageLean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransitionFamily {
    /// Soft crossfades, no graphic wipes.
    Subtle,
    /// Crossfade + lift; accent wipes on impact (pre-0.6 behavior).
    Editorial,
    /// Straight-edged panel wipes and lateral handoffs.
    Geometric,
    /// Fast accent wipes, scale punches, short overlaps.
    Kinetic,
    /// Near-cuts.
    Hard,
}

/// How often scene handoffs use a graphic wipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WipeTendency {
    Never,
    ImpactOnly,
    Frequent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitGesture {
    /// Fade in place.
    Fade,
    /// Ease back, lift and fade (pre-0.6).
    Lift,
    /// Slide sideways out of frame.
    Slide,
    /// Scale up and snap away.
    Punch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlapCharacter {
    Long,
    Standard,
    Short,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionCharacter {
    pub family: TransitionFamily,
    pub wipe: WipeTendency,
    pub exit: ExitGesture,
    pub overlap: OverlapCharacter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DensityLevel {
    Sparse,
    Balanced,
    Dense,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationStyle {
    /// No furniture beyond content.
    None,
    /// Folio/index marks, thin rules.
    Editorial,
    /// Coordinates, tick marks, data labels.
    Structured,
    /// Bold stickers, arrows, big index numerals.
    Graphic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualDensityProfile {
    pub level: DensityLevel,
    pub annotation: AnnotationStyle,
    /// Collage patches (halftone) behind cards.
    pub collage_patches: bool,
    /// Large faint background keyword.
    pub ghost_word: Level,
}

/// How often the composition meaningfully evolves (a style bias, not timing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositionRhythm {
    SlowBreathing,
    MeasuredEditorial,
    Progressive,
    Active,
    HighFrequency,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScaleContrast {
    Subtle,
    Moderate,
    Large,
    Dramatic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Presence {
    Recessive,
    Balanced,
    Dominant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activity {
    Still,
    Quiet,
    Structured,
    Active,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerActivityProfile {
    pub foreground: Presence,
    pub midground: Activity,
    pub background: Activity,
}

/// Deterministic, constrained variation: curated alternatives picked from
/// (style seed, story key). Classic never varies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Variation {
    pub accent: u8,
    pub fields: u8,
    /// Mirror asymmetric background arrangements.
    pub mirror: bool,
}

/// Everything the MotionCompiler needs to express a style. Internal: weak
/// models never author this.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedStyleProfile {
    pub tone: ResolvedTone,
    /// (0.14) Genre the tone asks for (street / documentary / hype pick their
    /// own art direction look and grammar under `--art auto`).
    #[serde(default, skip_serializing_if = "Genre::is_standard")]
    pub genre: Genre,
    pub palette: PaletteProfile,
    pub background: BackgroundGrammar,
    pub typography: TypographyPairing,
    pub material: MaterialFinish,
    pub image_treatment: ImageTreatmentBias,
    pub motion: MotionTemperament,
    pub transition: TransitionCharacter,
    pub density: VisualDensityProfile,
    pub rhythm: CompositionRhythm,
    pub scale: ScaleContrast,
    pub layers: LayerActivityProfile,
    pub variation: Variation,
    /// (0.7.1) Visual-construction language (from a reference only; neutral otherwise).
    pub visual: super::visual::VisualLanguage,
    /// The StyleProfile the rest of the compiler reads: explicit fields kept,
    /// defaulted fields filled from the tone, taste fields resolved.
    #[serde(skip)]
    pub effective: StyleProfile,
}

/// (0.14) Genre requested by the style's tone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Genre {
    #[default]
    Standard,
    Street,
    Documentary,
    Hype,
    Studio,
    Cinematic,
}

impl Genre {
    pub fn is_standard(&self) -> bool {
        *self == Genre::Standard
    }
    pub fn of(tone: Tone) -> Genre {
        match tone {
            Tone::Street => Genre::Street,
            Tone::Documentary => Genre::Documentary,
            Tone::Hype => Genre::Hype,
            Tone::Studio => Genre::Studio,
            Tone::Cinematic => Genre::Cinematic,
            _ => Genre::Standard,
        }
    }
}

// ---------------------------------------------------------------------------
// Reference principles (0.6 contract; 0.7 fills it from reference videos)
// ---------------------------------------------------------------------------

/// Style principles from a reference, already normalized into the engine's own
/// vocabulary (`reference::normalize` builds it from a public
/// `ReferenceStyleProfile`). Every field is optional; present fields fill
/// dimensions the StyleProfile left `auto` (explicit style fields always win).
/// No frames, layouts or assets are ever copied — only principles.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferencePrinciples {
    pub tone: Option<ResolvedTone>,
    pub polarity: Option<ResolvedPolarity>,
    pub temperature: Option<ResolvedTemperature>,
    /// Large saturated color fields (`true` → print families) or grounds with
    /// one accent (`false` → paper/dark families), whatever the tone.
    pub color_fields: Option<bool>,
    /// OKLab hue (degrees) of the reference's dominant accent: picks the
    /// nearest curated accent of the family (never a copied color).
    pub accent_hue: Option<f32>,
    pub background: Option<BackgroundGrammar>,
    pub typography: Option<TypographyPairing>,
    pub material: Option<MaterialFinish>,
    pub image_treatment: Option<ImageTreatmentBias>,
    pub density: Option<DensityLevel>,
    pub rhythm: Option<CompositionRhythm>,
    pub temperament: Option<TemperamentKind>,
    pub transition: Option<TransitionFamily>,
    pub scale: Option<ScaleContrast>,
    #[serde(default)]
    pub layers: LayerHints,
    /// (0.7.1) How the reference constructs scenes; neutral when unknown.
    #[serde(default)]
    pub visual: super::visual::VisualLanguage,
}

/// Partial layer activity; missing parts take the tone's default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerHints {
    pub foreground: Option<Presence>,
    pub midground: Option<Activity>,
    pub background: Option<Activity>,
}

impl ReferencePrinciples {
    /// True when no principle is set (an all-unknown reference).
    pub fn is_empty(&self) -> bool {
        *self == ReferencePrinciples::default()
    }
}

// ---------------------------------------------------------------------------
// Director
// ---------------------------------------------------------------------------

/// Resolve a style with no reference and no story key.
pub fn resolve(style: &StyleProfile) -> ResolvedStyleProfile {
    resolve_with(style, None, 0)
}

/// Resolve `style`, letting `reference` fill `auto` dimensions. `story_key`
/// (a stable hash of the story, see [`story_key`]) drives deterministic
/// variation so different stories do not collapse into identical palettes.
pub fn resolve_with(
    style: &StyleProfile,
    reference: Option<&ReferencePrinciples>,
    story_key: u64,
) -> ResolvedStyleProfile {
    let r = reference.cloned().unwrap_or_default();
    let tone = resolve_tone(style, &r);
    let polarity = match style.polarity {
        Polarity::Light => ResolvedPolarity::Light,
        Polarity::Dark => ResolvedPolarity::Dark,
        Polarity::Auto => r.polarity.unwrap_or(match tone {
            ResolvedTone::Technical => ResolvedPolarity::Dark,
            _ => ResolvedPolarity::Light,
        }),
    };
    let temperature = match style.temperature {
        Temperature::Warm => ResolvedTemperature::Warm,
        Temperature::Cool => ResolvedTemperature::Cool,
        Temperature::Auto => r.temperature.unwrap_or(match tone {
            ResolvedTone::Technical => ResolvedTemperature::Cool,
            _ => ResolvedTemperature::Warm,
        }),
    };
    let variation = variation_director(tone, style.seed, story_key);
    let fields = r.color_fields.unwrap_or(tone == ResolvedTone::Playful);
    let family = palette_family(fields, polarity, temperature);
    let colors = if tone == ResolvedTone::Classic {
        Palette::for_style(style)
    } else {
        color_director(family, style.accent_role, variation, r.accent_hue)
    };
    let background = r.background.unwrap_or(match tone {
        ResolvedTone::Classic => BackgroundGrammar::PaperCollage,
        ResolvedTone::Editorial => BackgroundGrammar::PaperField,
        ResolvedTone::Technical => BackgroundGrammar::TechnicalGrid,
        ResolvedTone::Playful => BackgroundGrammar::PrintFields,
    });
    let palette = PaletteProfile {
        family,
        polarity,
        temperature,
        // Field colors rotate wherever the background is made of print fields.
        trajectory: match background {
            BackgroundGrammar::PrintFields => PaletteTrajectory::FieldCycle,
            _ => PaletteTrajectory::Steady,
        },
        colors,
    };
    let typography = typography_director(tone, style.typography_style, r.typography);
    // Preserve the legacy no-reference mapping; when a reference supplies a
    // finish, a non-default legacy material remains an explicit constraint.
    let material = if r.material.is_some() && style.material != StyleProfile::default().material {
        match style.material {
            MaterialStyle::Paper => MaterialFinish::UncoatedPaper,
            MaterialStyle::Flat => MaterialFinish::CleanFlat,
        }
    } else {
        r.material.unwrap_or(match tone {
            ResolvedTone::Classic => match style.material {
                MaterialStyle::Paper => MaterialFinish::UncoatedPaper,
                MaterialStyle::Flat => MaterialFinish::CleanFlat,
            },
            ResolvedTone::Editorial => MaterialFinish::UncoatedPaper,
            ResolvedTone::Technical => MaterialFinish::Screen,
            ResolvedTone::Playful => MaterialFinish::CoatedPrint,
        })
    };
    let image_treatment = r.image_treatment.unwrap_or(tone_image_treatment(tone));
    let kind = temperament_kind(tone, style.temperament, r.temperament);
    let motion = motion_temperament(kind);
    let transition = transition_character(r.transition.unwrap_or(match (tone, kind) {
        (ResolvedTone::Classic, _) => TransitionFamily::Editorial,
        (_, TemperamentKind::Restrained) => TransitionFamily::Editorial,
        (ResolvedTone::Editorial, _) => TransitionFamily::Editorial,
        (ResolvedTone::Technical, _) => TransitionFamily::Geometric,
        (ResolvedTone::Playful, _) => TransitionFamily::Kinetic,
    }));
    let level = density_level(tone, style.density, r.density);
    let density = density_profile(tone, level, style.depth);
    let rhythm = r.rhythm.unwrap_or(match (tone, kind) {
        (ResolvedTone::Classic, _) => CompositionRhythm::MeasuredEditorial,
        (_, TemperamentKind::Restrained) if tone != ResolvedTone::Editorial => {
            CompositionRhythm::MeasuredEditorial
        }
        (ResolvedTone::Editorial, TemperamentKind::Energetic) => CompositionRhythm::Progressive,
        (ResolvedTone::Editorial, _) => CompositionRhythm::MeasuredEditorial,
        (ResolvedTone::Technical, TemperamentKind::Energetic) => CompositionRhythm::Active,
        (ResolvedTone::Technical, _) => CompositionRhythm::Progressive,
        (ResolvedTone::Playful, TemperamentKind::Energetic) => CompositionRhythm::Active,
        (ResolvedTone::Playful, _) => CompositionRhythm::Progressive,
    });
    let scale = r.scale.unwrap_or(match tone {
        ResolvedTone::Classic => ScaleContrast::Moderate,
        ResolvedTone::Editorial => ScaleContrast::Large,
        ResolvedTone::Technical => ScaleContrast::Moderate,
        ResolvedTone::Playful => ScaleContrast::Dramatic,
    });
    let tone_layers = match tone {
        ResolvedTone::Classic => LayerActivityProfile {
            foreground: Presence::Balanced,
            midground: Activity::Quiet,
            background: Activity::Quiet,
        },
        ResolvedTone::Editorial => LayerActivityProfile {
            foreground: Presence::Balanced,
            midground: Activity::Quiet,
            background: Activity::Still,
        },
        ResolvedTone::Technical => LayerActivityProfile {
            foreground: Presence::Balanced,
            midground: Activity::Structured,
            background: Activity::Structured,
        },
        ResolvedTone::Playful => LayerActivityProfile {
            foreground: Presence::Dominant,
            midground: Activity::Active,
            background: Activity::Active,
        },
    };
    let layers = LayerActivityProfile {
        foreground: r.layers.foreground.unwrap_or(tone_layers.foreground),
        midground: r.layers.midground.unwrap_or(tone_layers.midground),
        background: r.layers.background.unwrap_or(tone_layers.background),
    };
    let effective = effective_style(style, tone, polarity, material, density.collage_patches);
    ResolvedStyleProfile {
        genre: Genre::of(style.tone),
        tone,
        palette,
        background,
        typography,
        material,
        image_treatment,
        motion,
        transition,
        density,
        rhythm,
        scale,
        layers,
        variation,
        visual: r.visual.clone(),
        effective,
    }
}

/// The image-treatment bias a tone implies by itself. A resolved bias equal to
/// this keeps the 0.6 legacy-aware preset mapping (`compiler/treatment.rs`); a
/// different bias (from a reference) uses the direct bias table.
pub fn tone_image_treatment(tone: ResolvedTone) -> ImageTreatmentBias {
    match tone {
        ResolvedTone::Classic => ImageTreatmentBias::Classic,
        ResolvedTone::Editorial => ImageTreatmentBias::PaperCutout,
        ResolvedTone::Technical => ImageTreatmentBias::Monochrome,
        ResolvedTone::Playful => ImageTreatmentBias::PrintCutout,
    }
}

/// Stable story hash for variation (FNV-1a over the title and beat statements).
pub fn story_key(title: &str, statements: &[&str]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for s in std::iter::once(title).chain(statements.iter().copied()) {
        for b in s.bytes().chain(std::iter::once(0xff)) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    }
    h
}

fn resolve_tone(style: &StyleProfile, r: &ReferencePrinciples) -> ResolvedTone {
    match style.tone {
        Tone::Editorial | Tone::Documentary => ResolvedTone::Editorial,
        // Studio: heavy grotesk display (the classic pairing).
        Tone::Studio => ResolvedTone::Classic,
        Tone::Technical => ResolvedTone::Technical,
        Tone::Playful | Tone::Street | Tone::Hype => ResolvedTone::Playful,
        Tone::Cinematic => ResolvedTone::Classic,
        Tone::Auto => {
            let any_taste = style.polarity != Polarity::Auto
                || style.temperature != Temperature::Auto
                || style.temperament != Temperament::Auto
                || style.density != Density::Auto;
            match r.tone {
                Some(t) => t,
                None if any_taste || !r.is_empty() => ResolvedTone::Editorial,
                None => ResolvedTone::Classic,
            }
        }
    }
}

/// Palette family from (color fields, polarity, temperature). Without a
/// reference, `fields` is "the tone is playful".
fn palette_family(
    fields: bool,
    polarity: ResolvedPolarity,
    temperature: ResolvedTemperature,
) -> PaletteFamily {
    use ResolvedPolarity::*;
    use ResolvedTemperature::*;
    match (fields, polarity, temperature) {
        (true, Light, _) => PaletteFamily::PrintBright,
        (true, Dark, _) => PaletteFamily::PrintDark,
        (_, Light, Warm) => PaletteFamily::WarmPaper,
        (_, Light, Cool) => PaletteFamily::CoolPaper,
        (_, Dark, Warm) => PaletteFamily::DarkWarm,
        (_, Dark, Cool) => PaletteFamily::DarkCool,
    }
}

const fn rgb(v: u32) -> Color {
    Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// ColorDirector: curated grounds, inks and accents per family. Classic keeps
/// the exact pre-0.6 palette (via `Palette::for_style`).
pub(crate) fn color_director(
    family: PaletteFamily,
    role: AccentRole,
    v: Variation,
    accent_hue: Option<f32>,
) -> Palette {
    // (paper, card, ink, muted, accent options [(accent, on_accent)], fields)
    type Accent = (u32, u32);
    let (paper, card, ink, muted, accents, fields): (u32, u32, u32, u32, &[Accent], &[u32]) =
        match family {
            PaletteFamily::WarmPaper => (
                0xECE3D2,
                0xF6F1E6,
                0x171513,
                0x6F665A,
                &[(0xE0331F, 0xF6F1E6), (0xC8561E, 0xF6F1E6)],
                &[],
            ),
            PaletteFamily::CoolPaper => (
                0xE4E8EC,
                0xF5F7F9,
                0x12161B,
                0x5B6570,
                &[(0x233FD1, 0xF5F7F9), (0x0E7C74, 0xF5F7F9)],
                &[],
            ),
            PaletteFamily::DarkWarm => (
                0x17130F,
                0x231D17,
                0xF2E8D8,
                0x9A8E7C,
                &[(0xF0A23A, 0x17130F), (0xE8553A, 0x17130F)],
                &[0x2C241C],
            ),
            PaletteFamily::DarkCool => (
                0x0D1117,
                0x161C24,
                0xE6EDF3,
                0x8391A0,
                &[(0x3CC8F0, 0x0D1117), (0xB8F23A, 0x0D1117)],
                &[0x1A2430],
            ),
            PaletteFamily::PrintBright => (
                0xFFF2DC,
                0xFFFFFF,
                0x1C1633,
                0x5E5873,
                &[(0xFF4A1C, 0xFFF2DC), (0xFF3D7F, 0xFFF2DC)],
                &[0xFFC53D, 0x2B50FF, 0xFF9EC0],
            ),
            PaletteFamily::PrintDark => (
                0x1C1633,
                0x2A2248,
                0xFFF2DC,
                0xA79FC0,
                &[(0xFF4A1C, 0xFFF2DC), (0xFFC53D, 0x1C1633)],
                &[0x2B50FF, 0xFF4A1C, 0x3A2F66],
            ),
        };
    let explicit = match role {
        AccentRole::SignalRed => None,
        AccentRole::Cobalt => Some((0x233FD1, 0xF6F1E6)),
        AccentRole::Acid => Some((0xC9E42B, 0x161412)),
    };
    // Precedence: explicit accent role > reference accent hue (nearest curated
    // option of this family) > story variation.
    let from_reference = accent_hue.map(|hue| {
        *accents
            .iter()
            .min_by(|a, b| {
                hue_distance(hue, crate::reference::oklab::hue_of(a.0))
                    .total_cmp(&hue_distance(hue, crate::reference::oklab::hue_of(b.0)))
            })
            .expect("families have accents")
    });
    let (accent, on_accent) = explicit
        .or(from_reference)
        .unwrap_or(accents[v.accent as usize % accents.len()]);
    let mut fields: Vec<Color> = fields.iter().map(|&c| rgb(c)).collect();
    if !fields.is_empty() {
        let n = fields.len();
        fields.rotate_left(v.fields as usize % n);
    }
    Palette {
        paper: rgb(paper),
        card: rgb(card),
        ink: rgb(ink),
        muted: rgb(muted),
        accent: rgb(accent),
        on_accent: rgb(on_accent),
        fields,
    }
}

/// Angular distance in degrees (0..=180).
fn hue_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn typography_director(
    tone: ResolvedTone,
    explicit: TypographyStyle,
    reference: Option<TypographyPairing>,
) -> TypographyPairing {
    if explicit == TypographyStyle::CondensedMono {
        return TypographyPairing::CondensedMono;
    }
    // A reference's typography principle keeps its structural pairing; a tone
    // picks the readable face set for the emotion it evokes (0.8). Classic
    // stays the pre-0.6 identity.
    reference.unwrap_or(match tone {
        ResolvedTone::Classic => TypographyPairing::GroteskSerif,
        ResolvedTone::Editorial => TypographyPairing::HumanistSerif,
        ResolvedTone::Technical => TypographyPairing::PrecisionGrotesk,
        ResolvedTone::Playful => TypographyPairing::FriendlyGeometric,
    })
}

fn temperament_kind(
    tone: ResolvedTone,
    t: Temperament,
    reference: Option<TemperamentKind>,
) -> TemperamentKind {
    match t {
        Temperament::Restrained => TemperamentKind::Restrained,
        Temperament::Energetic => TemperamentKind::Energetic,
        Temperament::Balanced => match tone {
            ResolvedTone::Technical => TemperamentKind::Precise,
            _ => TemperamentKind::Editorial,
        },
        Temperament::Auto => reference.unwrap_or(match tone {
            ResolvedTone::Classic => TemperamentKind::Editorial,
            ResolvedTone::Editorial => TemperamentKind::Restrained,
            ResolvedTone::Technical => TemperamentKind::Precise,
            ResolvedTone::Playful => TemperamentKind::Energetic,
        }),
    }
}

/// MotionTemperamentDirector: the trait table for each temperament.
pub fn motion_temperament(kind: TemperamentKind) -> MotionTemperament {
    use Level::*;
    let (amplitude, settle, overshoot, stagger, camera, anticipation, secondary, read, lean) =
        match kind {
            TemperamentKind::Restrained => (
                Low,
                SettleCharacter::Soft,
                Low,
                StaggerCharacter::Editorial,
                CameraActivity::Controlled,
                Low,
                Low,
                Low,
                LanguageLean::Calm,
            ),
            TemperamentKind::Editorial => (
                Medium,
                SettleCharacter::Standard,
                Medium,
                StaggerCharacter::Editorial,
                CameraActivity::Standard,
                Medium,
                Medium,
                Medium,
                LanguageLean::Classic,
            ),
            TemperamentKind::Precise => (
                Medium,
                SettleCharacter::Crisp,
                Low,
                StaggerCharacter::Structured,
                CameraActivity::Controlled,
                Medium,
                Medium,
                Medium,
                LanguageLean::Structured,
            ),
            TemperamentKind::Energetic => (
                High,
                SettleCharacter::Springy,
                High,
                StaggerCharacter::Cascading,
                CameraActivity::Active,
                High,
                High,
                High,
                LanguageLean::Kinetic,
            ),
        };
    MotionTemperament {
        kind,
        amplitude,
        settle,
        overshoot,
        stagger,
        camera,
        anticipation,
        secondary_motion: secondary,
        read_activity: read,
        lean,
    }
}

/// TransitionCharacterDirector.
pub fn transition_character(family: TransitionFamily) -> TransitionCharacter {
    let (wipe, exit, overlap) = match family {
        TransitionFamily::Subtle => (
            WipeTendency::Never,
            ExitGesture::Fade,
            OverlapCharacter::Long,
        ),
        TransitionFamily::Editorial => (
            WipeTendency::ImpactOnly,
            ExitGesture::Lift,
            OverlapCharacter::Standard,
        ),
        TransitionFamily::Geometric => (
            WipeTendency::Frequent,
            ExitGesture::Slide,
            OverlapCharacter::Standard,
        ),
        TransitionFamily::Kinetic => (
            WipeTendency::Frequent,
            ExitGesture::Punch,
            OverlapCharacter::Short,
        ),
        TransitionFamily::Hard => (
            WipeTendency::Never,
            ExitGesture::Fade,
            OverlapCharacter::Short,
        ),
    };
    TransitionCharacter {
        family,
        wipe,
        exit,
        overlap,
    }
}

fn density_level(tone: ResolvedTone, d: Density, reference: Option<DensityLevel>) -> DensityLevel {
    match d {
        Density::Sparse => DensityLevel::Sparse,
        Density::Balanced => DensityLevel::Balanced,
        Density::Dense => DensityLevel::Dense,
        Density::Auto => reference.unwrap_or(match tone {
            ResolvedTone::Playful => DensityLevel::Dense,
            _ => DensityLevel::Balanced,
        }),
    }
}

/// VisualDensityDirector. Density adds supporting layers around one primary
/// focus; it never removes hierarchy or safe text areas.
pub(crate) fn density_profile(
    tone: ResolvedTone,
    level: DensityLevel,
    depth: Depth,
) -> VisualDensityProfile {
    if tone == ResolvedTone::Classic && level == DensityLevel::Balanced {
        return VisualDensityProfile {
            level,
            annotation: AnnotationStyle::None,
            collage_patches: depth == Depth::Layered,
            ghost_word: Level::Medium,
        };
    }
    let annotation = match (level, tone) {
        (DensityLevel::Sparse, _) => AnnotationStyle::None,
        (_, ResolvedTone::Technical) => AnnotationStyle::Structured,
        (_, ResolvedTone::Playful) => AnnotationStyle::Graphic,
        _ => AnnotationStyle::Editorial,
    };
    VisualDensityProfile {
        level,
        annotation,
        collage_patches: match level {
            DensityLevel::Sparse => false,
            DensityLevel::Balanced => {
                tone != ResolvedTone::Technical && tone != ResolvedTone::Editorial
            }
            DensityLevel::Dense => true,
        },
        ghost_word: match (level, tone) {
            (DensityLevel::Sparse, _) => Level::Low,
            (_, ResolvedTone::Editorial) => Level::Low,
            (DensityLevel::Dense, _) => Level::High,
            _ => Level::Medium,
        },
    }
}

/// VariationDirector.
fn variation_director(tone: ResolvedTone, seed: u64, story_key: u64) -> Variation {
    if tone == ResolvedTone::Classic {
        return Variation {
            accent: 0,
            fields: 0,
            mirror: false,
        };
    }
    let h = super::mix(seed ^ 0x7A57E, story_key);
    Variation {
        accent: (h & 0xFF) as u8 % 2,
        fields: ((h >> 8) & 0xFF) as u8 % 3,
        mirror: (h >> 16) & 1 == 1,
    }
}

/// The StyleProfile the pre-0.6 compiler paths read. Explicit (non-default)
/// fields always win; default fields take the tone's preset.
fn effective_style(
    style: &StyleProfile,
    tone: ResolvedTone,
    polarity: ResolvedPolarity,
    material_finish: MaterialFinish,
    patches: bool,
) -> StyleProfile {
    let mut e = style.clone();
    e.tone = match tone {
        ResolvedTone::Classic => Tone::Auto,
        ResolvedTone::Editorial => Tone::Editorial,
        ResolvedTone::Technical => Tone::Technical,
        ResolvedTone::Playful => Tone::Playful,
    };
    if tone == ResolvedTone::Classic {
        return e;
    }
    let d = StyleProfile::default();
    let (camera, typography) = match tone {
        ResolvedTone::Technical => (CameraStyle::Drift, TypographyStyle::CondensedMono),
        _ => (CameraStyle::SlowPush, TypographyStyle::GroteskSerif),
    };
    // (0.7) Ground and grain follow the resolved material, so a reference's
    // material reaches the compiler. Each tone's own material maps to exactly
    // its 0.6 preset (editorial paper/subtle, technical flat/none, playful
    // flat/subtle).
    let (material, texture) = match material_finish {
        MaterialFinish::UncoatedPaper => (MaterialStyle::Paper, TextureStyle::SubtlePrint),
        MaterialFinish::CleanFlat | MaterialFinish::Screen => {
            (MaterialStyle::Flat, TextureStyle::None)
        }
        MaterialFinish::CoatedPrint => (MaterialStyle::Flat, TextureStyle::SubtlePrint),
    };
    if e.material == d.material {
        e.material = if polarity == ResolvedPolarity::Dark {
            MaterialStyle::Flat
        } else {
            material
        };
    }
    if e.texture_style == d.texture_style {
        e.texture_style = texture;
    }
    if e.camera_style == d.camera_style {
        e.camera_style = camera;
    }
    if e.typography_style == d.typography_style {
        e.typography_style = typography;
    }
    if e.depth == d.depth {
        e.depth = if patches { Depth::Layered } else { Depth::Flat };
    }
    e
}

// ---------------------------------------------------------------------------
// Fingerprint + diversity comparison
// ---------------------------------------------------------------------------

/// The dimensions that make two styles genuinely different. Accent hue is
/// deliberately absent: red → blue is not a new design system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StyleFingerprint {
    pub palette_family: PaletteFamily,
    pub polarity: ResolvedPolarity,
    pub background: BackgroundGrammar,
    pub typography: TypographyPairing,
    pub material: MaterialFinish,
    pub image_treatment: ImageTreatmentBias,
    pub motion_temperament: TemperamentKind,
    pub transition_character: TransitionFamily,
    pub visual_density: VisualDensityProfile,
    pub composition_rhythm: CompositionRhythm,
    pub scale_contrast: ScaleContrast,
}

impl ResolvedStyleProfile {
    pub fn fingerprint(&self) -> StyleFingerprint {
        StyleFingerprint {
            palette_family: self.palette.family,
            polarity: self.palette.polarity,
            background: self.background,
            typography: self.typography,
            material: self.material,
            image_treatment: self.image_treatment,
            motion_temperament: self.motion.kind,
            transition_character: self.transition.family,
            visual_density: self.density,
            composition_rhythm: self.rhythm,
            scale_contrast: self.scale,
        }
    }
}

/// One compared dimension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DimensionDiff {
    pub dimension: &'static str,
    pub different: bool,
}

impl StyleFingerprint {
    /// Per-dimension comparison, in a fixed order.
    pub fn compare(&self, other: &StyleFingerprint) -> Vec<DimensionDiff> {
        let d = |dimension, different| DimensionDiff {
            dimension,
            different,
        };
        vec![
            d("palette", self.palette_family != other.palette_family),
            d("polarity", self.polarity != other.polarity),
            d("background", self.background != other.background),
            d("typography", self.typography != other.typography),
            d("material", self.material != other.material),
            d(
                "image_treatment",
                self.image_treatment != other.image_treatment,
            ),
            d(
                "motion_character",
                self.motion_temperament != other.motion_temperament,
            ),
            d(
                "transition_style",
                self.transition_character != other.transition_character,
            ),
            d("density", self.visual_density != other.visual_density),
            d(
                "composition_rhythm",
                self.composition_rhythm != other.composition_rhythm,
            ),
            d(
                "scale_contrast",
                self.scale_contrast != other.scale_contrast,
            ),
        ]
    }

    /// Number of meaningfully different dimensions (0..=11).
    pub fn distance(&self, other: &StyleFingerprint) -> usize {
        self.compare(other).iter().filter(|d| d.different).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_style_is_classic_and_unchanged() {
        let s = StyleProfile::default();
        let r = resolve(&s);
        assert_eq!(r.tone, ResolvedTone::Classic);
        assert_eq!(r.effective, s);
        assert_eq!(r.palette.colors, Palette::for_style(&s));
    }

    #[test]
    fn accent_change_is_not_a_new_style() {
        let a = StyleProfile::default();
        let b = StyleProfile {
            accent_role: AccentRole::Cobalt,
            ..a.clone()
        };
        assert_eq!(
            resolve(&a)
                .fingerprint()
                .distance(&resolve(&b).fingerprint()),
            0
        );
    }

    #[test]
    fn explicit_legacy_field_wins_over_tone() {
        let s = StyleProfile {
            tone: Tone::Technical,
            texture_style: TextureStyle::HeavyPrint,
            ..Default::default()
        };
        assert_eq!(
            resolve(&s).effective.texture_style,
            TextureStyle::HeavyPrint
        );
    }
}
