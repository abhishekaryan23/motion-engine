//! (0.9) Exploration: an operator knob (`--explore 0..=3`, `--seed`) that widens
//! ONLY curated, guard-railed taste sets. Level 0 is canonical (byte-identical
//! to 0.8). Choices are hash(seed, level, story key, dimension) — no RNG, no
//! clocks. Meaning, numbers and entity identity are never touched: exploration
//! only changes HOW a story looks and sounds, never what it says.
//!
//! Frozen contract (0.9 Phase 2):
//! * L1: typography option within the emotion (see `typography`), accent among
//!   the curated accents, background field order / mirror, SFX sound ids
//!   within their families.
//! * L2: + adjacent emotion's options, background grammar neighbour, transition
//!   family ±1, scale contrast ±1, motion amplitude ±1, grammar alternates only
//!   where the semantic table already allows a tie (never number / structured /
//!   data beats), asset-family alternates among equal-score catalog matches.
//! * L3: + any tone-compatible emotion, palette family neighbour, composition
//!   rhythm ±1, layout variants (density ±1, scale contrast up to ±2).
//! * Guardrails at every level: text/ground contrast >= 4.5:1, minimum type
//!   size, validation and layout checks must pass, otherwise the dimensions
//!   introduced at the highest level in use fall back one level (recorded),
//!   repeated until the project passes (level 0 always passes by definition).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::backdrop;
use super::taste::{
    color_director, density_profile, transition_character, BackgroundGrammar, CompositionRhythm,
    DensityLevel, Level, PaletteFamily, PaletteTrajectory, ResolvedPolarity, ResolvedStyleProfile,
    ResolvedTemperature, ResolvedTone, ScaleContrast, TransitionFamily,
};
use super::typography::TypographyChoice;
use super::Palette;
use crate::scene::{Color, Layer, LayerKind, MotionProject};

/// One explorable dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExploreDimension {
    Typography,
    Accent,
    FieldOrder,
    Mirror,
    Sfx,
    Background,
    Transition,
    ScaleContrast,
    MotionAmplitude,
    GrammarAlternate,
    AssetFamily,
    Palette,
    Rhythm,
    Density,
}

impl ExploreDimension {
    /// Every dimension in declaration order.
    pub const ALL: [ExploreDimension; 14] = [
        ExploreDimension::Typography,
        ExploreDimension::Accent,
        ExploreDimension::FieldOrder,
        ExploreDimension::Mirror,
        ExploreDimension::Sfx,
        ExploreDimension::Background,
        ExploreDimension::Transition,
        ExploreDimension::ScaleContrast,
        ExploreDimension::MotionAmplitude,
        ExploreDimension::GrammarAlternate,
        ExploreDimension::AssetFamily,
        ExploreDimension::Palette,
        ExploreDimension::Rhythm,
        ExploreDimension::Density,
    ];

    /// The lowest exploration level at which this dimension varies.
    pub const fn level(self) -> u8 {
        use ExploreDimension::*;
        match self {
            Typography | Accent | FieldOrder | Mirror | Sfx => 1,
            Background | Transition | ScaleContrast | MotionAmplitude | GrammarAlternate
            | AssetFamily => 2,
            Palette | Rhythm | Density => 3,
        }
    }
}

/// What exploration chose for one dimension.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DimensionChoice {
    pub dimension: ExploreDimension,
    /// Level the value was finally resolved at (after guardrail fallbacks).
    pub level: u8,
    /// Chosen value (snake_case name or option id).
    pub value: String,
    /// The canonical (level 0) value.
    pub canonical: String,
    /// Why this dimension fell back, if it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<String>,
}

/// Recorded in `ProjectMeta.exploration` when `explore > 0`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExploreRecord {
    pub level: u8,
    pub seed: u64,
    pub choices: Vec<DimensionChoice>,
}

/// Deterministic choice among `n` candidates (index; 0 when `n <= 1`).
pub fn pick(seed: u64, level: u8, story_key: u64, dim: ExploreDimension, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let h = super::mix(
        super::mix(seed ^ 0xE8_9105E, story_key),
        ((level as u64) << 32) ^ (dim as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15),
    );
    (h % n as u64) as usize
}

// ---------------------------------------------------------------------------
// Neighbour tables (curated; the only values exploration may move to)
// ---------------------------------------------------------------------------

/// Background grammars that keep the tone's legibility (a plain ground is the
/// only safe neighbour of the technical grid).
pub fn background_neighbours(b: BackgroundGrammar) -> &'static [BackgroundGrammar] {
    use BackgroundGrammar::*;
    match b {
        PaperCollage => &[PaperField],
        PaperField => &[PaperCollage, CleanFlat],
        CleanFlat => &[PaperField],
        TechnicalGrid => &[CleanFlat],
        PrintFields => &[PaperCollage],
    }
}

/// Transition families ordered from quiet to kinetic (±1 steps).
pub const TRANSITION_ORDER: [TransitionFamily; 4] = [
    TransitionFamily::Subtle,
    TransitionFamily::Editorial,
    TransitionFamily::Geometric,
    TransitionFamily::Kinetic,
];

pub const SCALE_ORDER: [ScaleContrast; 4] = [
    ScaleContrast::Subtle,
    ScaleContrast::Moderate,
    ScaleContrast::Large,
    ScaleContrast::Dramatic,
];

pub const AMPLITUDE_ORDER: [Level; 3] = [Level::Low, Level::Medium, Level::High];

pub const RHYTHM_ORDER: [CompositionRhythm; 5] = [
    CompositionRhythm::SlowBreathing,
    CompositionRhythm::MeasuredEditorial,
    CompositionRhythm::Progressive,
    CompositionRhythm::Active,
    CompositionRhythm::HighFrequency,
];

pub const DENSITY_ORDER: [DensityLevel; 3] = [
    DensityLevel::Sparse,
    DensityLevel::Balanced,
    DensityLevel::Dense,
];

/// Palette families one step away (same polarity first; print families swap
/// polarity only through the contrast guardrail).
pub fn palette_neighbours(p: PaletteFamily) -> &'static [PaletteFamily] {
    use PaletteFamily::*;
    match p {
        WarmPaper => &[CoolPaper],
        CoolPaper => &[WarmPaper],
        DarkWarm => &[DarkCool],
        DarkCool => &[DarkWarm],
        PrintBright => &[WarmPaper],
        PrintDark => &[DarkWarm],
    }
}

/// Candidates `canonical ± 1` (or ± `reach`) along an ordered scale, canonical
/// first, then lower, then higher; out-of-range steps are dropped.
pub fn steps<T: Copy + PartialEq>(order: &[T], canonical: T, reach: usize) -> Vec<T> {
    let Some(i) = order.iter().position(|v| *v == canonical) else {
        return vec![canonical];
    };
    let mut out = vec![canonical];
    for d in 1..=reach {
        if i >= d {
            out.push(order[i - d]);
        }
        if i + d < order.len() {
            out.push(order[i + d]);
        }
    }
    out
}

/// snake_case name of a serializable vocabulary value.
fn name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::from("?"),
    }
}

/// Pick among `candidates` (canonical first) for `dim` and record the choice.
fn choose<T: Copy + Serialize>(
    choices: &mut Vec<DimensionChoice>,
    dim: ExploreDimension,
    level: u8,
    seed: u64,
    story_key: u64,
    candidates: &[T],
) -> T {
    let i = pick(seed, level, story_key, dim, candidates.len());
    let value = candidates[i];
    choices.push(DimensionChoice {
        dimension: dim,
        level,
        value: name(&value),
        canonical: name(&candidates[0]),
        fallback: None,
    });
    value
}

/// Placeholder for the typography choice until [`set_typography_choice`] runs
/// (typography is chosen by `typography::resolve_typography`).
const TYPOGRAPHY_PENDING: &str = "unresolved";

/// Placeholder for the grammar-alternate choice until [`set_grammar_alternates`]
/// runs (the beats' template candidates exist only once they are built).
const GRAMMAR_PENDING: &str = "unresolved";

fn plain(dim: ExploreDimension, level: u8, value: &str, canonical: &str) -> DimensionChoice {
    DimensionChoice {
        dimension: dim,
        level,
        value: value.to_string(),
        canonical: canonical.to_string(),
        fallback: None,
    }
}

/// Apply exploration to a resolved taste (typography is chosen separately by
/// `typography::resolve_typography`; its [`DimensionChoice`] is a placeholder
/// until [`set_typography_choice`] fills it). Level 0 returns the taste
/// unchanged and no record.
///
/// Choices use `pick(seed, level, story_key, dim, n)` over curated candidates,
/// canonical first. The Classic tone's taste never varies (its record lists
/// every dimension with the canonical value); typography, sound and asset
/// ties are decided elsewhere (`resolve_typography`, `plan_audio_explored`,
/// `AssetLibrary::catalog_match`). `GrammarAlternate` is a placeholder until
/// [`set_grammar_alternates`] fills it from the built beats (exploration never
/// moves a beat off its semantic template; it reports the alternates the beats
/// have).
pub fn apply_exploration(
    taste: &ResolvedStyleProfile,
    level: u8,
    seed: u64,
    story_key: u64,
) -> (ResolvedStyleProfile, Option<ExploreRecord>) {
    if level == 0 {
        return (taste.clone(), None);
    }
    let level = level.min(3);
    let mut t = taste.clone();
    let classic = taste.tone == ResolvedTone::Classic;
    let mut choices: Vec<DimensionChoice> = Vec::new();
    let mut colors_dirty = false;
    let pk = |dim: ExploreDimension, n: usize| pick(seed, level, story_key, dim, n);

    for dim in ExploreDimension::ALL {
        if dim.level() > level {
            continue;
        }
        use ExploreDimension as D;
        match dim {
            D::Typography => {
                choices.push(plain(dim, level, TYPOGRAPHY_PENDING, TYPOGRAPHY_PENDING))
            }
            D::Accent => {
                let canonical = taste.variation.accent;
                let chosen = if classic { canonical } else { pk(dim, 2) as u8 };
                t.variation.accent = chosen;
                colors_dirty |= chosen != canonical;
                choices.push(plain(
                    dim,
                    level,
                    &format!("accent_{chosen}"),
                    &format!("accent_{canonical}"),
                ));
            }
            D::FieldOrder => {
                let canonical = taste.variation.fields;
                let chosen = if classic { canonical } else { pk(dim, 3) as u8 };
                t.variation.fields = chosen;
                colors_dirty |= chosen != canonical;
                choices.push(plain(
                    dim,
                    level,
                    &format!("fields_{chosen}"),
                    &format!("fields_{canonical}"),
                ));
            }
            D::Mirror => {
                let canonical = taste.variation.mirror;
                let chosen = if classic { canonical } else { pk(dim, 2) == 1 };
                t.variation.mirror = chosen;
                choices.push(plain(
                    dim,
                    level,
                    &chosen.to_string(),
                    &canonical.to_string(),
                ));
            }
            D::Sfx => choices.push(plain(dim, level, "seeded", "canonical")),
            D::Background => {
                let mut cands = vec![taste.background];
                if !classic {
                    cands.extend_from_slice(background_neighbours(taste.background));
                }
                let b = choose(&mut choices, dim, level, seed, story_key, &cands);
                t.background = b;
                t.palette.trajectory = match b {
                    BackgroundGrammar::PrintFields => PaletteTrajectory::FieldCycle,
                    _ => PaletteTrajectory::Steady,
                };
            }
            D::Transition => {
                let cands = if classic {
                    vec![taste.transition.family]
                } else {
                    steps(&TRANSITION_ORDER, taste.transition.family, 1)
                };
                let f = choose(&mut choices, dim, level, seed, story_key, &cands);
                if f != taste.transition.family {
                    t.transition = transition_character(f);
                }
            }
            D::ScaleContrast => {
                let cands = if classic {
                    vec![taste.scale]
                } else {
                    // Level 3 adds the oversized "layout variant" (+-2).
                    steps(&SCALE_ORDER, taste.scale, if level >= 3 { 2 } else { 1 })
                };
                t.scale = choose(&mut choices, dim, level, seed, story_key, &cands);
            }
            D::MotionAmplitude => {
                let cands = if classic {
                    vec![taste.motion.amplitude]
                } else {
                    steps(&AMPLITUDE_ORDER, taste.motion.amplitude, 1)
                };
                t.motion.amplitude = choose(&mut choices, dim, level, seed, story_key, &cands);
            }
            D::GrammarAlternate => {
                choices.push(plain(dim, level, GRAMMAR_PENDING, GRAMMAR_PENDING));
            }
            D::AssetFamily => choices.push(plain(dim, level, "seeded", "family_order")),
            D::Palette => {
                let mut cands = vec![taste.palette.family];
                if !classic {
                    cands.extend_from_slice(palette_neighbours(taste.palette.family));
                }
                let f = choose(&mut choices, dim, level, seed, story_key, &cands);
                if f != taste.palette.family {
                    colors_dirty = true;
                    t.palette.family = f;
                    t.palette.polarity = match f {
                        PaletteFamily::DarkWarm
                        | PaletteFamily::DarkCool
                        | PaletteFamily::PrintDark => ResolvedPolarity::Dark,
                        _ => ResolvedPolarity::Light,
                    };
                    match f {
                        PaletteFamily::WarmPaper | PaletteFamily::DarkWarm => {
                            t.palette.temperature = ResolvedTemperature::Warm
                        }
                        PaletteFamily::CoolPaper | PaletteFamily::DarkCool => {
                            t.palette.temperature = ResolvedTemperature::Cool
                        }
                        _ => {}
                    }
                }
            }
            D::Rhythm => {
                let cands = if classic {
                    vec![taste.rhythm]
                } else {
                    steps(&RHYTHM_ORDER, taste.rhythm, 1)
                };
                t.rhythm = choose(&mut choices, dim, level, seed, story_key, &cands);
            }
            D::Density => {
                let cands = if classic {
                    vec![taste.density.level]
                } else {
                    steps(&DENSITY_ORDER, taste.density.level, 1)
                };
                let l = choose(&mut choices, dim, level, seed, story_key, &cands);
                if l != taste.density.level {
                    t.density = density_profile(t.tone, l, t.effective.depth);
                    // Furniture annotations carry text (folio numerals, labels):
                    // exploration never adds or removes words, so the
                    // annotation style stays canonical.
                    t.density.annotation = taste.density.annotation;
                }
            }
        }
    }

    if colors_dirty && !classic {
        // The same ColorDirector path `resolve_with` uses (no reference hue:
        // an explored accent / field order / palette family replaces it).
        t.palette.colors =
            color_director(t.palette.family, t.effective.accent_role, t.variation, None);
    }
    (
        t,
        Some(ExploreRecord {
            level,
            seed,
            choices,
        }),
    )
}

/// The value string recorded for a typography choice: the registry option id,
/// or the legacy pairing name.
pub fn typography_value(choice: &TypographyChoice) -> String {
    match (&choice.option, &choice.legacy) {
        (Some(o), _) => o.clone(),
        (None, Some(l)) => name(l),
        (None, None) => String::from("legacy"),
    }
}

/// Fill the Typography [`DimensionChoice`] of `record` from the director's
/// choice (`canonical` = the level-0 choice).
pub fn set_typography_choice(
    record: &mut ExploreRecord,
    choice: &TypographyChoice,
    canonical: &TypographyChoice,
) {
    for c in &mut record.choices {
        if c.dimension == ExploreDimension::Typography {
            c.value = typography_value(choice);
            c.canonical = typography_value(canonical);
        }
    }
}

/// (0.23) Fill the GrammarAlternate [`DimensionChoice`] of `record` from the
/// built beats: `"<alternate>/<candidates>"` per beat, comma separated, in
/// beat order (`"0/3,0/1,0/2"` = the first of three template candidates, the
/// only one, the first of two). Exploration never moves a beat off its
/// semantic template (alternate 0, the canonical value): the count says how
/// many curated alternates the beat has, which is what a take (`--take`)
/// chooses among under `--variety`.
pub fn set_grammar_alternates(record: &mut ExploreRecord, beats: &[(u8, u8)]) {
    let join = |pick: &dyn Fn(u8) -> u8| {
        beats
            .iter()
            .map(|&(alternate, count)| format!("{}/{}", pick(alternate), count.max(1)))
            .collect::<Vec<_>>()
            .join(",")
    };
    let (value, canonical) = (join(&|a| a), join(&|_| 0));
    for c in &mut record.choices {
        if c.dimension == ExploreDimension::GrammarAlternate {
            c.value = value.clone();
            c.canonical = canonical.clone();
        }
    }
}

// ---------------------------------------------------------------------------
// Guardrails
// ---------------------------------------------------------------------------

/// Minimum WCAG contrast ratio for text on its ground.
pub const MIN_CONTRAST: f32 = 4.5;
/// Minimum text size in canvas units (multiplied by u = short side / 1080).
pub const MIN_FONT_PX: f32 = 20.0;
/// Pixels a text box may overhang the canvas before it counts as outside
/// (float noise and measured-box slack).
pub const BOX_TOLERANCE_PX: f32 = 1.0;
/// Contrast an explored palette may lose against a canonical palette that
/// itself misses 4.5:1 (the curated alternative accents sit within this).
pub const CANONICAL_SLACK: f32 = 0.25;

fn lin(c: u8) -> f32 {
    let v = c as f32 / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: Color) -> f32 {
    0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
}

/// WCAG contrast ratio between two colors (1..=21).
pub fn contrast_ratio(a: Color, b: Color) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Thresholds for [`guardrail_report_with`]. `absolute()` is the contract
/// (4.5:1, 20 x u px, every text box inside the canvas). Canonical (level 0)
/// output is never re-judged: `relative_to` relaxes a check only as far as the
/// canonical project itself already fails it (e.g. a canonical accent button
/// at 4.0:1, or the oversized ghost word that bleeds off the canvas by
/// design), so exploration may not make anything worse than level 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Guardrails {
    pub min_ink_contrast: f32,
    pub min_accent_contrast: f32,
    /// Text layer ids whose box already leaves the canvas at level 0.
    pub known_outside: BTreeSet<String>,
    /// Text layer ids already below the minimum size at level 0.
    pub known_small: BTreeSet<String>,
    /// (0.23) `scene:layer` of text already drawn over a print field it
    /// cannot be read on at level 0.
    pub known_on_field: BTreeSet<String>,
}

impl Guardrails {
    /// The contract thresholds, no exemptions.
    pub fn absolute() -> Self {
        Guardrails {
            min_ink_contrast: MIN_CONTRAST,
            min_accent_contrast: MIN_CONTRAST,
            known_outside: BTreeSet::new(),
            known_small: BTreeSet::new(),
            known_on_field: BTreeSet::new(),
        }
    }

    /// Thresholds relaxed to what the canonical project already satisfies.
    pub fn relative_to(canonical: &MotionProject, palette: &Palette) -> Self {
        let (w, h) = (
            canonical.canvas.width as f32,
            canonical.canvas.height as f32,
        );
        let u = w.min(h) / 1080.0;
        let (sizes, outside) = scan_text(canonical);
        Guardrails {
            min_ink_contrast: MIN_CONTRAST
                .min(contrast_ratio(palette.ink, palette.paper) - CANONICAL_SLACK),
            min_accent_contrast: MIN_CONTRAST
                .min(contrast_ratio(palette.on_accent, palette.accent) - CANONICAL_SLACK),
            known_outside: outside.into_iter().collect(),
            known_small: sizes
                .into_iter()
                .filter(|(_, s)| *s < MIN_FONT_PX * u - 1e-3)
                .map(|(id, _)| id)
                .collect(),
            known_on_field: backdrop::field_findings(canonical)
                .into_iter()
                .map(|f| format!("{}:{}", f.scene, f.layer))
                .collect(),
        }
    }
}

fn scan_text(project: &MotionProject) -> (Vec<(String, f32)>, Vec<String>) {
    let canvas = (project.canvas.width as f32, project.canvas.height as f32);
    let mut sizes = Vec::new();
    let mut outside = Vec::new();
    for scene in &project.scenes {
        walk_text(
            &scene.layers,
            Some((0.0, 0.0, 1.0, 1.0)),
            canvas,
            &mut sizes,
            &mut outside,
        );
    }
    (sizes, outside)
}

/// Every guardrail violation of a compiled project (empty = passes), at the
/// contract thresholds: see [`guardrail_report_with`].
pub fn guardrail_report(project: &MotionProject, palette: &Palette) -> Vec<String> {
    guardrail_report_with(project, palette, &Guardrails::absolute())
}

/// Every guardrail violation of a compiled project (empty = passes):
/// (a) WCAG contrast of ink on paper and on_accent on accent,
/// (b) every Text layer's font size >= 20 x u px,
/// (c) `validate` reports no errors,
/// (d) every text layer box lies inside the canvas. The box check uses the
/// layer's static x/y/size/scale/anchor (groups only when unrotated); layers
/// with a dynamic `layout` binding or a rotation, and layers inside a rotated
/// or layout-bound group, are skipped (not computable without rendering).
/// Animated offsets (entrances, camera) are ignored.
pub fn guardrail_report_with(
    project: &MotionProject,
    palette: &Palette,
    g: &Guardrails,
) -> Vec<String> {
    let mut out = Vec::new();
    let ink = contrast_ratio(palette.ink, palette.paper);
    if ink < g.min_ink_contrast {
        out.push(format!(
            "contrast ink/paper {ink:.2}:1 < {:.2}:1",
            g.min_ink_contrast
        ));
    }
    let on = contrast_ratio(palette.on_accent, palette.accent);
    if on < g.min_accent_contrast {
        out.push(format!(
            "contrast on_accent/accent {on:.2}:1 < {:.2}:1",
            g.min_accent_contrast
        ));
    }
    // (0.23) Not only the paper: every print field the palette puts under a
    // beat's text must carry that text's ink (3:1 for display text, 4.5:1 for
    // smaller). A palette (or field order / mirror) that cannot is rejected
    // here, so the dimension falls back as for any other guardrail.
    if let Some(f) = backdrop::field_findings(project).into_iter().find(|f| {
        !g.known_on_field
            .contains(&format!("{}:{}", f.scene, f.layer))
    }) {
        out.push(format!(
            "contrast text/field: {} {} ink #{:02X}{:02X}{:02X} on field #{:02X}{:02X}{:02X} {:.2}:1 < {:.1}:1",
            f.scene, f.layer, f.ink.r, f.ink.g, f.ink.b, f.field.r, f.field.g, f.field.b, f.ratio, f.need
        ));
    }
    let u = (project.canvas.width.min(project.canvas.height) as f32) / 1080.0;
    let (sizes, outside) = scan_text(project);
    if let Some((id, size)) = sizes
        .iter()
        .find(|(id, s)| *s < MIN_FONT_PX * u - 1e-3 && !g.known_small.contains(id))
    {
        out.push(format!(
            "min font size: layer {id} is {size:.1}px < {:.1}px",
            MIN_FONT_PX * u
        ));
    }
    if let Err(errs) = crate::validate::validate(project, None) {
        out.push(format!("validation: {} error(s)", errs.0.len()));
    }
    if let Some(id) = outside.iter().find(|id| !g.known_outside.contains(*id)) {
        out.push(format!("layout: text layer {id} lies outside the canvas"));
    }
    out
}

/// Collect (layer id, font size) of text layers and ids of text layers whose
/// static box leaves the canvas. `origin` = (x, y, sx, sy) maps the
/// container's box space to the canvas; `None` = not computable (sizes only).
fn walk_text(
    layers: &[Layer],
    origin: Option<(f32, f32, f32, f32)>,
    canvas: (f32, f32),
    sizes: &mut Vec<(String, f32)>,
    outside: &mut Vec<String>,
) {
    for l in layers {
        let placed = origin.filter(|_| l.layout.is_none() && l.rotation_degrees == 0.0);
        let boxed = placed.map(|(ox, oy, osx, osy)| {
            let left = ox + (l.x - l.anchor_x * l.width * l.scale_x) * osx;
            let top = oy + (l.y - l.anchor_y * l.height * l.scale_y) * osy;
            (
                left,
                top,
                left + l.width * l.scale_x * osx,
                top + l.height * l.scale_y * osy,
            )
        });
        match &l.kind {
            LayerKind::Text(style) => {
                sizes.push((l.id.clone(), style.font_size));
                if let (Some((left, top, right, bottom)), true) = (boxed, l.width > 0.0) {
                    let t = BOX_TOLERANCE_PX;
                    if left < -t || top < -t || right > canvas.0 + t || bottom > canvas.1 + t {
                        outside.push(l.id.clone());
                    }
                }
            }
            LayerKind::Group { children } => {
                let child_origin = boxed.map(|(left, top, _, _)| {
                    let (_, _, osx, osy) = origin.unwrap_or((0.0, 0.0, 1.0, 1.0));
                    (left, top, osx * l.scale_x, osy * l.scale_y)
                });
                walk_text(children, child_origin, canvas, sizes, outside);
            }
            _ => {}
        }
    }
}

/// Result of [`guardrail_loop`].
#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailOutcome<T> {
    pub value: T,
    /// The level the accepted compile ran at.
    pub level: u8,
    /// `(level that failed, first failure)` in order.
    pub fallbacks: Vec<(u8, String)>,
}

/// The guardrail fallback loop, independent of the compiler: compile at
/// `requested`, run `check`; on failures drop one level (recording
/// `(level that failed, first failure)`) and repeat. Level 0 is canonical and
/// always accepted. Deterministic.
pub fn guardrail_loop<T, E>(
    requested: u8,
    mut compile: impl FnMut(u8) -> Result<T, E>,
    mut check: impl FnMut(&T) -> Vec<String>,
) -> Result<GuardrailOutcome<T>, E> {
    let mut level = requested;
    let mut fallbacks: Vec<(u8, String)> = Vec::new();
    loop {
        let value = compile(level)?;
        if level == 0 {
            return Ok(GuardrailOutcome {
                value,
                level,
                fallbacks,
            });
        }
        match check(&value).into_iter().next() {
            None => {
                return Ok(GuardrailOutcome {
                    value,
                    level,
                    fallbacks,
                })
            }
            Some(first) => {
                fallbacks.push((level, first));
                level -= 1;
            }
        }
    }
}

/// Merge the record of the first (requested-level) compile with the accepted
/// one: dimensions introduced above `final_level` are reset to canonical and
/// marked `fallback: "guardrail: <check>"` with `level = final_level`.
/// `final_record` is `None` when the accepted level is 0; `typography_final`
/// is the typography value the accepted compile actually used.
pub fn finalize_record(
    first: ExploreRecord,
    final_level: u8,
    final_record: Option<ExploreRecord>,
    typography_final: &str,
    fallbacks: &[(u8, String)],
) -> ExploreRecord {
    if fallbacks.is_empty() {
        return final_record.unwrap_or(first);
    }
    let reason = |dim_level: u8| {
        fallbacks
            .iter()
            .find(|(l, _)| *l == dim_level)
            .or_else(|| fallbacks.first())
            .map(|(_, r)| format!("guardrail: {r}"))
            .unwrap_or_else(|| String::from("guardrail"))
    };
    let choices = first
        .choices
        .iter()
        .map(|c| {
            let kept = if c.dimension.level() <= final_level {
                final_record
                    .as_ref()
                    .and_then(|r| r.choices.iter().find(|x| x.dimension == c.dimension))
            } else {
                None
            };
            match kept {
                Some(k) => DimensionChoice {
                    level: final_level,
                    ..k.clone()
                },
                None => DimensionChoice {
                    dimension: c.dimension,
                    level: final_level,
                    value: if c.dimension == ExploreDimension::Typography {
                        typography_final.to_string()
                    } else {
                        c.canonical.clone()
                    },
                    canonical: c.canonical.clone(),
                    fallback: Some(reason(c.dimension.level())),
                },
            }
        })
        .collect();
    ExploreRecord {
        level: first.level,
        seed: first.seed,
        choices,
    }
}
