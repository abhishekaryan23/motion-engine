//! Reference normalization (0.7): a validated public
//! [`ReferenceStyleProfile`] → the engine's internal [`ReferencePrinciples`],
//! plus a per-dimension record of how faithfully each value maps.
//!
//! Two conservative stages:
//! 1. [`normalize_aliases`] (before strict parsing): a closed, documented alias
//!    table folds spelling variants ("very dark", "Mostly-Black" → `dark`;
//!    "unknown" → null). Anything not in the table is left alone and fails the
//!    strict parse — values are never guessed.
//! 2. [`normalize`]: maps each value to the nearest engine concept with a
//!    [`Fidelity`]. Unknown stays unknown; values below
//!    [`APPLY_CONFIDENCE`] are recorded and not applied.

use serde::Serialize;
use serde_json::Value;

use super::oklab;
use super::profile::*;
use crate::compiler::taste::{
    Activity, BackgroundGrammar, CompositionRhythm, DensityLevel, ImageTreatmentBias, LayerHints,
    MaterialFinish, Presence, ReferencePrinciples, ResolvedPolarity, ResolvedTemperature,
    ResolvedTone, ScaleContrast, TemperamentKind, TransitionFamily, TypographyPairing,
};
use crate::compiler::visual::VisualLanguage;
use crate::compiler::Grammar;

/// Values with a lower confidence are recorded but not applied.
pub const APPLY_CONFIDENCE: f32 = 0.5;
/// A palette-evidence swatch counts as an accent from this OKLab chroma.
pub const ACCENT_MIN_CHROMA: f32 = 0.10;

// ---------------------------------------------------------------------------
// Stage 1: aliases
// ---------------------------------------------------------------------------

/// One alias rewrite, reported to the caller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AliasNote {
    pub path: String,
    pub from: String,
    pub to: Option<String>,
}

/// Words meaning "unknown" in any dimension.
const UNKNOWN_WORDS: &[&str] = &["unknown", "unclear", "n/a", "na", "null", "", "not_visible"];

/// Per-dimension aliases (already lower-cased, spaces/hyphens → `_`).
fn alias(dimension: &str, v: &str) -> Option<&'static str> {
    Some(match (dimension, v) {
        ("tone", "magazine" | "editorial_design") => "editorial",
        ("tone", "tech" | "systematic" | "data") => "technical",
        ("tone", "fun" | "bold_graphic") => "playful",
        ("polarity", "very_dark" | "mostly_black" | "black" | "dark_mode" | "night") => "dark",
        ("polarity", "very_light" | "mostly_white" | "white" | "bright" | "light_mode") => "light",
        ("polarity", "both" | "alternating" | "light_and_dark") => "mixed",
        ("temperature", "warmer" | "hot") => "warm",
        ("temperature", "cooler" | "cold") => "cool",
        ("temperature", "balanced" | "grey" | "gray" | "achromatic") => "neutral",
        ("contrast" | "typography_contrast", "moderate" | "mid") => "medium",
        ("contrast" | "typography_contrast", "strong" | "very_high") => "high",
        ("contrast" | "typography_contrast", "soft" | "very_low") => "low",
        ("palette_character", "greyscale" | "grayscale" | "black_and_white") => "monochrome",
        ("palette_character", "desaturated") => "muted",
        ("palette_character", "single_accent" | "one_accent") => "restrained_accent",
        ("palette_character", "saturated" | "bright") => "vivid",
        ("palette_character", "multicolour" | "colorful" | "colourful") => "multicolor",
        ("background_character", "solid" | "plain" | "clean") => "flat",
        ("background_character", "vignette" | "gradients") => "gradient",
        ("background_character", "color_fields" | "fields") => "graphic_fields",
        ("background_character", "photo" | "footage" | "photography") => "photographic",
        ("background_character", "noise" | "halftone" | "texture") => "textured",
        ("typography_character", "grotesque" | "neo_grotesk" | "neo_grotesque") => "grotesk",
        ("typography_character", "geometric" | "geometric_sans_serif") => "geometric_sans",
        ("typography_character", "humanist" | "humanist_sans_serif") => "humanist_sans",
        ("typography_character", "condensed_sans" | "compressed") => "condensed",
        ("typography_character", "mono" | "monospace" | "monospaced") => "mono_technical",
        ("typography_character", "serif" | "editorial_serif" | "serif_display") => {
            "serif_editorial"
        }
        ("typography_character", "poster" | "display" | "heavy_display") => "poster_display",
        ("material_character", "uncoated_paper") => "paper",
        ("material_character", "clean_flat" | "vector" | "matte") => "flat",
        ("material_character", "ui" | "digital" | "emissive") => "screen",
        ("material_character", "coated_print" | "printed") => "print",
        ("material_character", "chrome" | "metal") => "metallic",
        ("material_character", "3d" | "rendered_3d" | "cgi") => "rendered3d",
        ("image_treatment", "no_images" | "no_imagery") => "none",
        ("image_treatment", "black_and_white" | "greyscale" | "grayscale" | "bw") => "monochrome",
        ("image_treatment", "desaturated" | "documentary") => "muted",
        ("image_treatment", "cut_out" | "paper_cutout") => "cutout",
        ("visual_density", "minimal" | "airy") => "sparse",
        ("visual_density", "moderate" | "medium") => "balanced",
        ("visual_density", "busy" | "layered" | "rich") => "dense",
        ("composition_rhythm", "slow" | "breathing") => "slow_breathing",
        ("composition_rhythm", "measured" | "editorial") => "measured_editorial",
        ("composition_rhythm", "building") => "progressive",
        ("composition_rhythm", "fast" | "busy") => "active",
        ("composition_rhythm", "very_fast" | "rapid" | "frenetic") => "high_frequency",
        ("motion_temperament", "calm" | "quiet" | "gentle") => "restrained",
        ("motion_temperament", "smooth" | "balanced" | "editorial") => "fluid",
        ("motion_temperament", "precise" | "crisp" | "decisive") => "snappy",
        ("motion_temperament", "dynamic" | "punchy" | "high_energy") => "energetic",
        ("motion_temperament", "bouncy" | "springy" | "elastic") => "playful",
        ("motion_temperament", "robotic" | "linear" | "stepped") => "mechanical",
        ("transition_character", "crossfade" | "dissolve" | "soft") => "subtle",
        ("transition_character", "wipe" | "panel" | "panels") => "geometric",
        ("transition_character", "energetic" | "sweep" | "whip") => "kinetic",
        ("transition_character", "cut" | "cuts" | "hard_cut" | "hard_cuts") => "hard",
        ("transition_character", "match_cut" | "morph" | "shared_element") => "continuous",
        ("scale_contrast", "low") => "subtle",
        ("scale_contrast", "medium") => "moderate",
        ("scale_contrast", "high" | "strong") => "large",
        ("scale_contrast", "extreme" | "very_high") => "dramatic",
        _ => return None,
    })
}

fn fold(s: &str) -> String {
    s.trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c == ' ' || c == '-' { '_' } else { c })
        .collect()
}

/// Fold string values of every known dimension's `value` through the alias
/// table, in place. Returns what changed. Unknown words become `null`.
pub fn normalize_aliases(doc: &mut Value) -> Vec<AliasNote> {
    let mut notes = Vec::new();
    let Some(obj) = doc.as_object_mut() else {
        return notes;
    };
    for &dim in DIMENSIONS {
        let Some(Value::Object(t)) = obj.get_mut(dim) else {
            continue;
        };
        let Some(v) = t.get_mut("value") else {
            continue;
        };
        if dim == "layer_activity" {
            if let Value::Object(parts) = v {
                for part in ["foreground", "midground", "background"] {
                    if let Some(Value::String(s)) = parts.get(part).cloned() {
                        let f = fold(&s);
                        let to = if UNKNOWN_WORDS.contains(&f.as_str()) {
                            None
                        } else {
                            Some(f)
                        };
                        rewrite(
                            &mut notes,
                            format!("{dim}.value.{part}"),
                            &s,
                            to,
                            parts.get_mut(part).expect("present"),
                        );
                    }
                }
            }
            if v.is_object() {
                continue;
            }
        }
        let Value::String(s) = v.clone() else {
            continue;
        };
        let f = fold(&s);
        let to = if UNKNOWN_WORDS.contains(&f.as_str()) {
            None
        } else {
            Some(alias(dim, &f).map(str::to_string).unwrap_or(f))
        };
        rewrite(&mut notes, format!("{dim}.value"), &s, to, v);
    }
    if let Some(vl) = obj.get_mut("visual_language") {
        normalize_visual_aliases(vl, &mut notes);
    }
    notes
}

/// Aliases for `visual_language` values only (dimension = `visual_language.<field>`).
fn visual_alias(dimension: &str, v: &str) -> Option<&'static str> {
    Some(match (dimension, v) {
        ("visual_language.medium", "text_only") => "type_only",
        ("visual_language.medium", "typography_led" | "type") => "type_led",
        ("visual_language.medium", "image" | "images" | "photo_led" | "illustration_led") => {
            "image_led"
        }
        ("visual_language.medium", "objects" | "object") => "object_led",
        ("visual_language.medium", "diagram" | "diagrams" | "schematic") => "diagrammatic",
        ("visual_language.medium", "ui" | "interface" | "screen_led") => "interface_led",
        ("visual_language.asset_usage", "heavy" | "high" | "frequent") => "dense",
        ("visual_language.asset_usage", "light" | "low" | "minimal" | "rare") => "sparse",
        ("visual_language.asset_usage", "medium" | "moderate") => "balanced",
        (
            "visual_language.type_image_balance",
            "type" | "text_dominant" | "typography_dominant",
        ) => "type_dominant",
        ("visual_language.type_image_balance", "image_dominant" | "visual" | "images_dominant") => {
            "visual_dominant"
        }
        ("visual_language.type_image_balance", "even") => "balanced",
        ("visual_language.asset_character", "photo" | "photography" | "photos") => "photographic",
        ("visual_language.asset_character", "illustration" | "illustrated" | "drawn") => {
            "illustrative"
        }
        ("visual_language.asset_character", "ui" | "screens" | "screen") => "interface",
        ("visual_language.asset_character", "objects" | "object") => "object_centric",
        ("visual_language.asset_character", "diagram" | "schematic") => "diagrammatic",
        ("visual_language.asset_character", "shapes" | "graphic") => "procedural",
        ("visual_language.asset_character", "cut_out" | "cutouts") => "cutout",
        ("visual_language.explanation_mode", "evidence" | "proof") => "evidence_based",
        ("visual_language.explanation_mode", "metaphor") => "metaphorical",
        ("visual_language.explanation_mode", "diagram" | "schematic") => "diagrammatic",
        ("visual_language.explanation_mode", "icons" | "abstract") => "symbolic",
        _ => return None,
    })
}

/// Trait fields of `visual_language` whose `value` is folded.
const VISUAL_TRAITS: &[&str] = &[
    "medium",
    "asset_usage",
    "type_image_balance",
    "asset_character",
    "explanation_mode",
];

/// Fold the string values inside `visual_language`: trait values, role names and
/// composition list entries. Unknown words null a trait/role value; in lists they
/// are dropped (and reported with `to: None`).
fn normalize_visual_aliases(vl: &mut Value, notes: &mut Vec<AliasNote>) {
    let Some(vl) = vl.as_object_mut() else {
        return;
    };
    for &field in VISUAL_TRAITS {
        let dim = format!("visual_language.{field}");
        let Some(Value::Object(t)) = vl.get_mut(field) else {
            continue;
        };
        let Some(v) = t.get_mut("value") else {
            continue;
        };
        let Value::String(s) = v.clone() else {
            continue;
        };
        let f = fold(&s);
        let to = if UNKNOWN_WORDS.contains(&f.as_str()) {
            None
        } else {
            Some(visual_alias(&dim, &f).map(str::to_string).unwrap_or(f))
        };
        rewrite(notes, format!("{dim}.value"), &s, to, v);
    }
    if let Some(Value::Array(roles)) = vl.get_mut("asset_roles") {
        for (i, r) in roles.iter_mut().enumerate() {
            let Some(v) = r.get_mut("role") else {
                continue;
            };
            let Value::String(s) = v.clone() else {
                continue;
            };
            let f = fold(&s);
            let to = (!UNKNOWN_WORDS.contains(&f.as_str())).then_some(f);
            rewrite(
                notes,
                format!("visual_language.asset_roles[{i}].role"),
                &s,
                to,
                v,
            );
        }
    }
    if let Some(Value::Object(c)) = vl.get_mut("composition_language") {
        for list in ["preferred", "secondary", "avoid"] {
            let Some(Value::Array(items)) = c.get_mut(list) else {
                continue;
            };
            let mut kept = Vec::with_capacity(items.len());
            for (i, item) in std::mem::take(items).into_iter().enumerate() {
                let Value::String(s) = &item else {
                    kept.push(item);
                    continue;
                };
                let f = fold(s);
                let path = format!("visual_language.composition_language.{list}[{i}]");
                if UNKNOWN_WORDS.contains(&f.as_str()) {
                    notes.push(AliasNote {
                        path,
                        from: s.clone(),
                        to: None,
                    });
                } else {
                    if &f != s {
                        notes.push(AliasNote {
                            path,
                            from: s.clone(),
                            to: Some(f.clone()),
                        });
                    }
                    kept.push(Value::String(f));
                }
            }
            *items = kept;
        }
    }
}

fn rewrite(
    notes: &mut Vec<AliasNote>,
    path: String,
    from: &str,
    to: Option<String>,
    v: &mut Value,
) {
    if to.as_deref() == Some(from) {
        return;
    }
    *v = match &to {
        Some(t) => Value::String(t.clone()),
        None => Value::Null,
    };
    notes.push(AliasNote {
        path,
        from: from.to_string(),
        to,
    });
}

// ---------------------------------------------------------------------------
// Stage 2: mapping
// ---------------------------------------------------------------------------

/// How faithfully a reference value is expressed by the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    /// The engine has this concept; applied as is.
    Exact,
    /// Applied as the nearest supported concept.
    Closest,
    /// Recognized; the engine expresses it only partly or keeps its own rule
    /// (e.g. legibility contrast); no constraint or a weaker one is applied.
    Partial,
    /// No engine concept; not applied (recorded as an unsupported trait).
    Unsupported,
    /// Absent or null.
    Unknown,
    /// Present with confidence below [`APPLY_CONFIDENCE`]; not applied.
    LowConfidence,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DimensionMapping {
    pub dimension: &'static str,
    /// The profile's value (snake_case), if any.
    pub reference: Option<String>,
    pub confidence: Option<f32>,
    /// The engine concept applied (snake_case), if any.
    pub engine: Option<String>,
    pub fidelity: Fidelity,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NormalizedReference {
    pub principles: ReferencePrinciples,
    /// One row per [`DIMENSIONS`] entry plus `accent_hint`, in report order.
    pub dimensions: Vec<DimensionMapping>,
    /// Sorted, deduplicated: the profile's own unsupported traits (confident
    /// ones) plus traits implied by unsupported/closest mappings.
    pub unsupported: Vec<UnsupportedTraitKind>,
}

fn name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

/// Map one dimension. `f` returns `(engine value, fidelity, implied trait)`;
/// `None` engine value = recognized but not applied.
fn map_trait<T: Serialize + Copy, E: Serialize + Copy>(
    rows: &mut Vec<DimensionMapping>,
    implied: &mut Vec<UnsupportedTraitKind>,
    dimension: &'static str,
    t: &Option<Trait<T>>,
    f: impl Fn(T) -> (Option<E>, Fidelity, Option<UnsupportedTraitKind>),
) -> Option<E> {
    let row = |reference, confidence, engine, fidelity| DimensionMapping {
        dimension,
        reference,
        confidence,
        engine,
        fidelity,
    };
    let Some(t) = t else {
        rows.push(row(None, None, None, Fidelity::Unknown));
        return None;
    };
    let Some(v) = t.value else {
        rows.push(row(None, Some(t.confidence), None, Fidelity::Unknown));
        return None;
    };
    if t.confidence < APPLY_CONFIDENCE {
        rows.push(row(
            Some(name(&v)),
            Some(t.confidence),
            None,
            Fidelity::LowConfidence,
        ));
        return None;
    }
    let (engine, fidelity, trait_kind) = f(v);
    implied.extend(trait_kind);
    rows.push(row(
        Some(name(&v)),
        Some(t.confidence),
        engine.as_ref().map(name),
        fidelity,
    ));
    engine
}

/// Map a validated profile into engine principles. Pure and deterministic.
pub fn normalize(p: &ReferenceStyleProfile) -> NormalizedReference {
    use Fidelity::*;
    use UnsupportedTraitKind as U;
    let mut rows = Vec::new();
    let mut implied = Vec::new();
    let (r, i) = (&mut rows, &mut implied);

    let tone = map_trait(r, i, "tone", &p.tone, |v| {
        let t = match v {
            RefTone::Editorial => ResolvedTone::Editorial,
            RefTone::Technical => ResolvedTone::Technical,
            RefTone::Playful => ResolvedTone::Playful,
        };
        (Some(t), Exact, None)
    });
    let polarity = map_trait(r, i, "polarity", &p.polarity, |v| match v {
        RefPolarity::Light => (Some(ResolvedPolarity::Light), Exact, None),
        RefPolarity::Dark => (Some(ResolvedPolarity::Dark), Exact, None),
        // One polarity per piece: the tone decides.
        RefPolarity::Mixed => (None, Partial, None),
    });
    let temperature = map_trait(r, i, "temperature", &p.temperature, |v| match v {
        RefTemperature::Warm => (Some(ResolvedTemperature::Warm), Exact, None),
        RefTemperature::Cool => (Some(ResolvedTemperature::Cool), Exact, None),
        // The curated cool grounds are the near-neutral ones.
        RefTemperature::Neutral => (Some(ResolvedTemperature::Cool), Closest, None),
    });
    // Text contrast is fixed by legibility rules (every curated palette is high contrast).
    map_trait::<_, RefLevel>(r, i, "contrast", &p.contrast, |v| match v {
        RefLevel::High => (Some(RefLevel::High), Exact, None),
        _ => (None, Partial, None),
    });
    let color_fields = map_trait(r, i, "palette_character", &p.palette_character, |v| {
        match v {
            PaletteCharacter::Multicolor => (Some(true), Exact, None),
            PaletteCharacter::RestrainedAccent | PaletteCharacter::Vivid => {
                (Some(false), Exact, None)
            }
            // One ground + one accent: the engine keeps a single accent for emphasis.
            PaletteCharacter::Monochrome | PaletteCharacter::Muted => (Some(false), Partial, None),
        }
    });
    let background = map_trait(
        r,
        i,
        "background_character",
        &p.background_character,
        |v| match v {
            BackgroundCharacter::Flat => (Some(BackgroundGrammar::CleanFlat), Exact, None),
            BackgroundCharacter::Paper => (Some(BackgroundGrammar::PaperField), Exact, None),
            BackgroundCharacter::Grid => (Some(BackgroundGrammar::TechnicalGrid), Exact, None),
            BackgroundCharacter::GraphicFields => {
                (Some(BackgroundGrammar::PrintFields), Exact, None)
            }
            BackgroundCharacter::Textured => (Some(BackgroundGrammar::PaperField), Closest, None),
            BackgroundCharacter::Gradient => (
                Some(BackgroundGrammar::CleanFlat),
                Closest,
                Some(U::GradientBackground),
            ),
            BackgroundCharacter::Photographic => {
                (None, Unsupported, Some(U::PhotographicBackground))
            }
        },
    );
    let typography = map_trait(r, i, "typography_character", &p.typography_character, |v| {
        use TypographyCharacter as T;
        match v {
            T::Grotesk => (Some(TypographyPairing::GroteskSerif), Exact, None),
            T::SerifEditorial => (Some(TypographyPairing::SerifSans), Exact, None),
            T::Condensed | T::MonoTechnical => {
                (Some(TypographyPairing::CondensedMono), Exact, None)
            }
            T::PosterDisplay => (Some(TypographyPairing::PosterBold), Exact, None),
            // Heavy sans display + clean sans body is the nearest bundled pairing.
            T::GeometricSans | T::HumanistSans => {
                (Some(TypographyPairing::PosterBold), Closest, None)
            }
            T::Script | T::Handwritten => (None, Unsupported, Some(U::HandLettering)),
        }
    });
    // Expressed only through the pairing and scale contrast.
    map_trait::<_, RefLevel>(r, i, "typography_contrast", &p.typography_contrast, |_| {
        (None, Partial, None)
    });
    let material = map_trait(r, i, "material_character", &p.material_character, |v| {
        use MaterialCharacter as M;
        match v {
            M::Paper => (Some(MaterialFinish::UncoatedPaper), Exact, None),
            M::Flat => (Some(MaterialFinish::CleanFlat), Exact, None),
            M::Screen => (Some(MaterialFinish::Screen), Exact, None),
            M::Print => (Some(MaterialFinish::CoatedPrint), Exact, None),
            M::Glossy | M::Metallic => (
                Some(MaterialFinish::CleanFlat),
                Closest,
                Some(U::ChromeOrMetallic),
            ),
            M::Rendered3d => (Some(MaterialFinish::CleanFlat), Closest, Some(U::True3d)),
        }
    });
    let image_treatment = map_trait(r, i, "image_treatment", &p.image_treatment, |v| {
        use ImageTreatmentCharacter as I;
        match v {
            // Nothing to learn about images.
            I::None => (None, Partial, None),
            I::Natural => (Some(ImageTreatmentBias::Natural), Exact, None),
            I::Monochrome => (Some(ImageTreatmentBias::Monochrome), Exact, None),
            I::Duotone => (Some(ImageTreatmentBias::Duotone), Exact, None),
            I::Muted => (Some(ImageTreatmentBias::Muted), Exact, None),
            I::Cutout => (Some(ImageTreatmentBias::PaperCutout), Exact, None),
            I::PrintCutout => (Some(ImageTreatmentBias::PrintCutout), Exact, None),
            I::HighContrast => (Some(ImageTreatmentBias::Monochrome), Closest, None),
        }
    });
    let density = map_trait(r, i, "visual_density", &p.visual_density, |v| {
        let d = match v {
            RefDensity::Sparse => DensityLevel::Sparse,
            RefDensity::Balanced => DensityLevel::Balanced,
            RefDensity::Dense => DensityLevel::Dense,
        };
        (Some(d), Exact, None)
    });
    let rhythm = map_trait(r, i, "composition_rhythm", &p.composition_rhythm, |v| {
        let d = match v {
            RefRhythm::SlowBreathing => CompositionRhythm::SlowBreathing,
            RefRhythm::MeasuredEditorial => CompositionRhythm::MeasuredEditorial,
            RefRhythm::Progressive => CompositionRhythm::Progressive,
            RefRhythm::Active => CompositionRhythm::Active,
            RefRhythm::HighFrequency => CompositionRhythm::HighFrequency,
        };
        (Some(d), Exact, None)
    });
    let temperament = map_trait(r, i, "motion_temperament", &p.motion_temperament, |v| {
        use RefTemperament as T;
        match v {
            T::Restrained => (Some(TemperamentKind::Restrained), Exact, None),
            T::Snappy => (Some(TemperamentKind::Precise), Exact, None),
            T::Energetic => (Some(TemperamentKind::Energetic), Exact, None),
            T::Fluid => (Some(TemperamentKind::Editorial), Closest, None),
            T::Mechanical => (Some(TemperamentKind::Precise), Closest, None),
            T::Playful => (Some(TemperamentKind::Energetic), Closest, None),
            T::Cinematic => (Some(TemperamentKind::Restrained), Closest, None),
        }
    });
    let transition = map_trait(r, i, "transition_character", &p.transition_character, |v| {
        use RefTransition as T;
        match v {
            T::Subtle => (Some(TransitionFamily::Subtle), Exact, None),
            T::Editorial => (Some(TransitionFamily::Editorial), Exact, None),
            T::Geometric => (Some(TransitionFamily::Geometric), Exact, None),
            T::Kinetic => (Some(TransitionFamily::Kinetic), Exact, None),
            T::Hard => (Some(TransitionFamily::Hard), Exact, None),
            // Shared-object continuation and camera handoffs are not modelled yet.
            T::Continuous | T::Cinematic => (Some(TransitionFamily::Subtle), Closest, None),
        }
    });
    let scale = map_trait(r, i, "scale_contrast", &p.scale_contrast, |v| {
        let s = match v {
            RefScale::Subtle => ScaleContrast::Subtle,
            RefScale::Moderate => ScaleContrast::Moderate,
            RefScale::Large => ScaleContrast::Large,
            RefScale::Dramatic => ScaleContrast::Dramatic,
        };
        (Some(s), Exact, None)
    });
    let layers = map_trait(r, i, "layer_activity", &p.layer_activity, |v| {
        let act = |a: RefActivity| match a {
            RefActivity::Still => Activity::Still,
            RefActivity::Quiet => Activity::Quiet,
            RefActivity::Structured => Activity::Structured,
            RefActivity::Active => Activity::Active,
        };
        let hints = LayerHints {
            foreground: v.foreground.map(|f| match f {
                RefPresence::Recessive => Presence::Recessive,
                RefPresence::Balanced => Presence::Balanced,
                RefPresence::Dominant => Presence::Dominant,
            }),
            midground: v.midground.map(act),
            background: v.background.map(act),
        };
        let complete =
            hints.foreground.is_some() && hints.midground.is_some() && hints.background.is_some();
        if hints == LayerHints::default() {
            (None, Unknown, None)
        } else {
            (Some(hints), if complete { Exact } else { Partial }, None)
        }
    });

    // Accent hint: the most prevalent saturated evidence swatch, unless the
    // palette is monochrome/muted (then the engine's own accent stays).
    let accent_allowed = !matches!(
        p.palette_character
            .as_ref()
            .filter(|t| t.confidence >= APPLY_CONFIDENCE)
            .and_then(|t| t.value),
        Some(PaletteCharacter::Monochrome | PaletteCharacter::Muted)
    );
    let accent_swatch = p
        .approximate_palette_evidence
        .iter()
        .filter_map(|s| {
            let rgb = oklab::parse_hex(&s.hex)?;
            let (_, c, h) = oklab::lch((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
            (c >= ACCENT_MIN_CHROMA).then_some((s.prevalence, h, s.hex.clone()))
        })
        .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.2.cmp(&a.2)));
    let accent_hue = accent_swatch
        .as_ref()
        .filter(|_| accent_allowed)
        .map(|a| a.1);
    rows.push(DimensionMapping {
        dimension: "accent_hint",
        reference: accent_swatch.as_ref().map(|a| a.2.clone()),
        confidence: None,
        engine: accent_hue.map(|h| format!("nearest_curated_accent_to_hue_{}", h.round() as i32)),
        fidelity: match (&accent_swatch, accent_hue) {
            (None, _) => Unknown,
            (Some(_), Some(_)) => Closest,
            (Some(_), None) => Partial,
        },
    });

    let visual = normalize_visual(p.visual_language.as_ref(), &mut rows);

    let mut unsupported: Vec<UnsupportedTraitKind> = p
        .unsupported_reference_traits
        .iter()
        .filter(|t| t.confidence >= APPLY_CONFIDENCE)
        .map(|t| t.kind)
        .chain(implied)
        .collect();
    unsupported.sort();
    unsupported.dedup();

    NormalizedReference {
        principles: ReferencePrinciples {
            tone,
            polarity,
            temperature,
            color_fields,
            accent_hue,
            background,
            typography,
            material,
            image_treatment,
            density,
            rhythm,
            temperament,
            transition,
            scale,
            layers: layers.unwrap_or_default(),
            visual,
        },
        dimensions: rows,
        unsupported,
    }
}

// ---------------------------------------------------------------------------
// Visual language (profile v0.2, 0.7.1)
// ---------------------------------------------------------------------------

/// Map the public `visual_language` section 1:1 into the engine's
/// [`VisualLanguage`] (same vocabulary, so every applied value is `Exact`).
/// Low-confidence and null values stay unknown; composition lists are made
/// disjoint (preferred > secondary > avoid). One row per visual dimension.
fn normalize_visual(
    v: Option<&super::visual_language::VisualLanguageProfile>,
    rows: &mut Vec<DimensionMapping>,
) -> VisualLanguage {
    use super::visual_language as pv;
    use crate::compiler::visual as iv;
    let empty = pv::VisualLanguageProfile::default();
    let v = v.unwrap_or(&empty);
    let mut sink = Vec::new();
    let medium = map_trait(rows, &mut sink, "visual.medium", &v.medium, |m| {
        let x = match m {
            pv::VisualMedium::TypeOnly => iv::Medium::TypeOnly,
            pv::VisualMedium::TypeLed => iv::Medium::TypeLed,
            pv::VisualMedium::ImageLed => iv::Medium::ImageLed,
            pv::VisualMedium::ObjectLed => iv::Medium::ObjectLed,
            pv::VisualMedium::Diagrammatic => iv::Medium::Diagrammatic,
            pv::VisualMedium::Collage => iv::Medium::Collage,
            pv::VisualMedium::InterfaceLed => iv::Medium::InterfaceLed,
            pv::VisualMedium::Mixed => iv::Medium::Mixed,
        };
        (Some(x), Fidelity::Exact, None)
    });
    let usage = map_trait(rows, &mut sink, "visual.asset_usage", &v.asset_usage, |u| {
        let x = match u {
            pv::AssetUsage::None => iv::Usage::None,
            pv::AssetUsage::Sparse => iv::Usage::Sparse,
            pv::AssetUsage::Balanced => iv::Usage::Balanced,
            pv::AssetUsage::Dense => iv::Usage::Dense,
        };
        (Some(x), Fidelity::Exact, None)
    });
    let balance = map_trait(
        rows,
        &mut sink,
        "visual.type_image_balance",
        &v.type_image_balance,
        |b| {
            let x = match b {
                pv::TypeImageBalance::TypeDominant => iv::Balance::TypeDominant,
                pv::TypeImageBalance::Balanced => iv::Balance::Balanced,
                pv::TypeImageBalance::VisualDominant => iv::Balance::VisualDominant,
            };
            (Some(x), Fidelity::Exact, None)
        },
    );
    let character = map_trait(
        rows,
        &mut sink,
        "visual.asset_character",
        &v.asset_character,
        |c| {
            use pv::AssetCharacter as A;
            let x = match c {
                A::Photographic => iv::Character::Photographic,
                A::Cutout => iv::Character::Cutout,
                A::Illustrative => iv::Character::Illustrative,
                A::Diagrammatic => iv::Character::Diagrammatic,
                A::ObjectCentric => iv::Character::ObjectCentric,
                A::Collage => iv::Character::Collage,
                A::Interface => iv::Character::Interface,
                A::Procedural => iv::Character::Procedural,
                A::Mixed => iv::Character::Mixed,
            };
            (Some(x), Fidelity::Exact, None)
        },
    );
    let explanation = map_trait(
        rows,
        &mut sink,
        "visual.explanation_mode",
        &v.explanation_mode,
        |e| {
            use pv::ExplanationMode as E;
            let x = match e {
                E::Literal => iv::Explanation::Literal,
                E::Diagrammatic => iv::Explanation::Diagrammatic,
                E::Symbolic => iv::Explanation::Symbolic,
                E::Metaphorical => iv::Explanation::Metaphorical,
                E::EvidenceBased => iv::Explanation::EvidenceBased,
                E::Mixed => iv::Explanation::Mixed,
            };
            (Some(x), Fidelity::Exact, None)
        },
    );

    let mut roles: Vec<crate::assets::AssetRole> = Vec::new();
    for t in v
        .asset_roles
        .iter()
        .filter(|t| t.confidence >= APPLY_CONFIDENCE)
    {
        let r = asset_role(t.role);
        if !roles.contains(&r) && roles.len() < 4 {
            roles.push(r);
        }
    }
    rows.push(DimensionMapping {
        dimension: "visual.asset_roles",
        reference: (!v.asset_roles.is_empty()).then(|| {
            v.asset_roles
                .iter()
                .map(|t| name(&t.role))
                .collect::<Vec<_>>()
                .join("+")
        }),
        confidence: None,
        engine: (!roles.is_empty()).then(|| roles.iter().map(name).collect::<Vec<_>>().join("+")),
        fidelity: match (v.asset_roles.is_empty(), roles.is_empty()) {
            (true, _) => Fidelity::Unknown,
            (false, true) => Fidelity::LowConfidence,
            (false, false) => Fidelity::Exact,
        },
    });

    let (mut preferred, mut secondary, mut avoid) = (Vec::new(), Vec::new(), Vec::new());
    let comp_row = match &v.composition_language {
        None => (None, None, Fidelity::Unknown),
        Some(c) if c.confidence < APPLY_CONFIDENCE => (
            Some(composition_text(c)),
            Some(c.confidence),
            Fidelity::LowConfidence,
        ),
        Some(c) => {
            for (src, dst) in [
                (&c.preferred, &mut preferred),
                (&c.secondary, &mut secondary),
                (&c.avoid, &mut avoid),
            ] {
                for g in src.iter().take(3) {
                    dst.push(grammar(*g));
                }
            }
            secondary.retain(|g| !preferred.contains(g));
            avoid.retain(|g| !preferred.contains(g) && !secondary.contains(g));
            preferred.dedup();
            secondary.dedup();
            avoid.dedup();
            (
                Some(composition_text(c)),
                Some(c.confidence),
                Fidelity::Exact,
            )
        }
    };
    let engine_comp =
        (comp_row.2 == Fidelity::Exact).then(|| composition_lists(&preferred, &secondary, &avoid));
    rows.push(DimensionMapping {
        dimension: "visual.composition_language",
        reference: comp_row.0,
        confidence: comp_row.1,
        engine: engine_comp,
        fidelity: comp_row.2,
    });

    VisualLanguage {
        medium: medium.unwrap_or_default(),
        usage,
        balance,
        character,
        roles,
        preferred,
        secondary,
        avoid,
        explanation,
    }
}

fn asset_role(r: super::visual_language::RefAssetRole) -> crate::assets::AssetRole {
    use super::visual_language::RefAssetRole as R;
    use crate::assets::AssetRole as A;
    match r {
        R::HeroSubject => A::HeroSubject,
        R::HeroObject => A::HeroObject,
        R::SupportingObject => A::SupportingObject,
        R::Environment => A::Environment,
        R::EvidenceImage => A::EvidenceImage,
        R::Portrait => A::Portrait,
        R::TransitionObject => A::TransitionObject,
        R::ForegroundOccluder => A::ForegroundOccluder,
    }
}

/// Public composition family → engine grammar (identical vocabulary).
pub fn grammar(g: super::visual_language::RefGrammar) -> Grammar {
    use super::visual_language::RefGrammar as R;
    match g {
        R::HeroObject => Grammar::HeroObject,
        R::TypeImageInterlock => Grammar::TypeImageInterlock,
        R::EditorialCollage => Grammar::EditorialCollage,
        R::SplitContrast => Grammar::SplitContrast,
        R::EvidenceStack => Grammar::EvidenceStack,
        R::DataStory => Grammar::DataStory,
        R::SequentialStack => Grammar::SequentialStack,
        R::SpatialCauseEffect => Grammar::SpatialCauseEffect,
        R::CinematicMultiplane => Grammar::CinematicMultiplane,
        R::KineticPoster => Grammar::KineticPoster,
    }
}

fn composition_text(c: &super::visual_language::CompositionLanguage) -> String {
    let g: Vec<Grammar> = c.preferred.iter().map(|x| grammar(*x)).collect();
    let s: Vec<Grammar> = c.secondary.iter().map(|x| grammar(*x)).collect();
    let a: Vec<Grammar> = c.avoid.iter().map(|x| grammar(*x)).collect();
    composition_lists(&g, &s, &a)
}

/// `preferred=a+b;secondary=c;avoid=d` (the coverage representation).
pub fn composition_lists(p: &[Grammar], s: &[Grammar], a: &[Grammar]) -> String {
    let j = |v: &[Grammar]| v.iter().map(name).collect::<Vec<_>>().join("+");
    format!("preferred={};secondary={};avoid={}", j(p), j(s), j(a))
}
