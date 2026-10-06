//! MotionCompiler: `CreativeIntent + StyleProfile -> MotionProject`.
//!
//! This is where MotionEngine's design intelligence lives. The compiler is
//! deterministic: the same intent, style, asset library and font metrics
//! always produce the same MotionScene (no clocks, no unseeded randomness,
//! ordered collections only).
//!
//! Stages:
//! 0. taste      — TasteDirector resolves the StyleProfile into a complete
//!    design system (palette, background, type, material, motion temperament,
//!    transition character, rhythm, density, scale contrast); see `taste.rs`
//! 1. theme      — palette + font roles from the resolved style
//! 2. timing     — beat durations from reading load and energy; overlaps from
//!    the incoming beat's energy (scenes overlap, never butt-cut)
//! 3. recipes    — per-purpose layout + motion (see `recipes.rs`)
//! 4. continuity — carried subjects become SharedElements whose track keys are
//!    contributed by each beat they appear in
//! 5. backdrop   — paper + grain spanning the whole piece

pub mod art_direction;
mod asset_plan;
mod backdrop;
pub mod brand;
pub mod captions;
pub mod catalog;
pub mod choreography;
mod collection;
mod derived;
pub mod direction;
pub mod explore;
mod furniture;
pub mod fx;
mod grammar;
mod layer_stack;
pub mod layout_frame;
mod loops;
mod measure;
pub mod preview;
mod recipes;
pub mod sequence;
pub mod speech_lifecycle;
pub mod speech_plan;
mod state_change;
pub mod story_warnings;
pub mod subject_rules;
pub mod taste;
pub mod taste_rules;
mod temporal;
mod theme;
pub(crate) mod transition;
pub(crate) mod treatment;
mod typeset;
pub mod typography;
pub mod value_coverage;
pub mod visual;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub use asset_plan::{plan_assets, plan_assets_with_reference};
pub use derived::{compute as compute_metric, format_metric, FormattedMetric};
pub use grammar::Grammar;
pub use measure::{ApproxMeasure, TextMeasure};
pub use theme::{FontFace, FontSet, Palette};
pub use typeset::{Block, Typesetter, Voice};

use crate::assets::AssetManifest;
use crate::easing::Easing;
use crate::intent::{
    Beat, Continuity, CreativeIntent, Energy, Format, Purpose, Relationship, Subject, SubjectKind,
    INTENT_VERSION, INTENT_VERSION_V0_1,
};
use crate::motion::language::{self, Language, MotionLanguageProfile};
use crate::motion::lifecycle::{self, LifecycleInput};
use crate::scene::{
    Asset, AssetKind, BoxRect, Camera, CameraMotion, CameraOp, Canvas, Color, Direction, KeyState,
    Layer, LayerKind, Lifecycle, Motion, MotionOp, MotionProject, ProjectMeta, RevealMode,
    SharedElement, TextureSpec, Theme, TrackKey, SCENE_VERSION,
};
use crate::style::{CameraStyle, Depth, StyleProfile, TextureStyle};

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("unsupported intent version '{0}' (expected {INTENT_VERSION}, or legacy {INTENT_VERSION_V0_1})")]
    Version(String),
    #[error("intent has no beats")]
    NoBeats,
    #[error("beat {beat}: {message}")]
    Beat { beat: usize, message: String },
    #[error("invalid intent: {0}")]
    Invalid(String),
    /// (0.5) Generation was attempted and a required image is unusable.
    #[error("required asset(s) missing: {0}")]
    MissingRequiredAsset(String),
}

/// Named assets available to the compiler. Paths in the compiled project are
/// relative to `root` (fonts under `fonts/`, objects under `library/`).
#[derive(Debug, Clone, Default)]
pub struct AssetLibrary {
    pub root: PathBuf,
    /// (0.8) Opt-in asset families under `library/<family>/` (catalog.json +
    /// manifest.json) the AssetPlanner may match before requesting a generated
    /// image. Empty = no catalogs (the pre-0.8 behaviour).
    pub families: Vec<String>,
    /// (0.9) Exploration `(level, seed)` for catalog matching: at level >= 2 a
    /// cross-family tie for the best score is picked with `explore::pick`
    /// instead of family order. `None` = canonical (family order).
    pub explore: Option<(u8, u64)>,
}

impl AssetLibrary {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        AssetLibrary {
            root: root.into(),
            families: Vec::new(),
            explore: None,
        }
    }

    /// (0.9) Let catalog matching explore cross-family ties (level >= 2 only).
    pub fn with_exploration(mut self, level: u8, seed: u64) -> Self {
        self.explore = (level >= 2).then_some((level, seed));
        self
    }

    /// Enable catalog families (read in the given order).
    pub fn with_families(mut self, families: Vec<String>) -> Self {
        self.families = families;
        self
    }

    /// Locate a named object (`library/<name>.svg` preferred over `.png`).
    pub fn find_object(&self, name: &str) -> Option<(AssetKind, String)> {
        let safe = name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if !safe {
            return None;
        }
        [("svg", AssetKind::Svg), ("png", AssetKind::Image)]
            .into_iter()
            .find_map(|(ext, kind)| {
                let rel = format!("library/{name}.{ext}");
                self.root.join(&rel).is_file().then_some((kind, rel))
            })
    }
}

/// Compile semantic intent into an explicit MotionScene (no delivered images:
/// compositions that want one draw a procedural placeholder).
pub fn compile(
    intent: &CreativeIntent,
    style: &StyleProfile,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
) -> Result<MotionProject, CompileError> {
    compile_with_assets(intent, style, library, measure, &AssetManifest::empty())
}

/// Compile with delivered images. `manifest` entry paths must be relative to
/// `library.root` (the CLI rewrites paths relative to the manifest file).
pub fn compile_with_assets(
    intent: &CreativeIntent,
    style: &StyleProfile,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
) -> Result<MotionProject, CompileError> {
    compile_full(intent, style, None, library, measure, manifest)
}

/// Resolve the style's taste for this story (deterministic variation keyed by
/// the story), optionally informed by a reference profile.
/// The story key of an intent (title + statements), as used by [`resolve_taste`].
pub fn story_key_of(intent: &CreativeIntent) -> u64 {
    let statements: Vec<&str> = intent.beats.iter().map(|b| b.statement.as_str()).collect();
    taste::story_key(&intent.title, &statements)
}

pub fn resolve_taste(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
) -> taste::ResolvedStyleProfile {
    let statements: Vec<&str> = intent.beats.iter().map(|b| b.statement.as_str()).collect();
    taste::resolve_with(
        style,
        reference,
        taste::story_key(&intent.title, &statements),
    )
}

/// Compile with delivered images and optional reference principles (0.7:
/// normalized from a ReferenceStyleProfile by `reference::normalize`).
pub fn compile_full(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
) -> Result<MotionProject, CompileError> {
    compile_with_music(intent, style, reference, library, measure, manifest, None)
}

/// (0.20) A compile warning: the intent compiled, but something the author
/// asked for was dropped, or a structure would read better. The CLI prints
/// them; they are never written into the scene, so default compiles stay
/// byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CompileWarning {
    /// Stable code, one of the `WARN_*` constants.
    pub code: String,
    /// The beat the warning is about (0-based), when it is about one beat.
    pub beat: Option<usize>,
    pub message: String,
}

/// A story of three or more beats where every beat is one unrelated object
/// and nothing (`layers`, a collection, compare / contrast, carry) ties them.
pub const WARN_UNRELATED_BEATS: &str = "unrelated_beats";
/// `continuity: carry_*` was requested but no shared element carried over.
pub const WARN_CARRY_IGNORED: &str = "carry_ignored";
/// The display title states the beat's number or verdict word that the
/// narration only says later, under a `TitleReveal::Free` look.
pub const WARN_TITLE_STATES_CONCLUSION: &str = "title_states_conclusion";
/// A word cue could not move a group as far as its word (delay limited by
/// the read-time floor, or an earlier move held at ENTER).
pub const WARN_CUE_CLAMPED: &str = "cue_clamped";
/// (0.22) An anchored group whose word is spoken could not be moved toward
/// it at all (no room before the beat ends, even with a shortened entrance):
/// it keeps its planned entrance and shows early (or late).
pub const WARN_CUE_DROPPED: &str = "cue_dropped";
/// (0.21) A brand colour was changed to keep text or highlights readable
/// (`brand::apply`); the message names the colour and what was used.
pub const WARN_BRAND_CONTRAST: &str = "brand_contrast";
/// (0.22) A beat's picture (an object subject, or a delivered image) is never
/// named in that beat's narration, so it shows before anyone says what it is.
pub const WARN_UNNAMED_PICTURE: &str = "unnamed_picture";
/// (0.23) A value, name or item the intent gives (a number value, either
/// side of a compare, a list item, a derived result, a state's from / to)
/// appears in no text layer visible during its beat.
pub const WARN_VALUE_DROPPED: &str = "value_dropped";
/// (0.23) A music mood forced by the `music` word conflicts with the mood
/// read from the story's content (`audio::story_mood`).
pub const WARN_MUSIC_FIT: &str = "music_fit";
/// (0.23, W8) A compare / contrast of two subjects names a `relationship`
/// that the composition does not draw (no connector, plus, VS or link).
pub const WARN_RELATIONSHIP_DROPPED: &str = "relationship_dropped";

/// (0.9) Operator inputs that sit beside CreativeIntent and StyleProfile
/// (CLI flags, never authored by weak models). `Default` reproduces the 0.8
/// output byte-for-byte.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompileOptions {
    /// `--explore 0..=3` (0 = canonical).
    pub explore: u8,
    /// `--seed`: exploration seed (independent of `StyleProfile.seed`).
    pub seed: u64,
    /// `--emotion`: typography emotion override.
    pub emotion: Option<typography::Emotion>,
    /// `--canvas WxH` / `--aspect`: canvas override; `None` = `intent.format`.
    pub canvas: Option<(u32, u32)>,
    /// (0.10) `--speech file.speech.json` (already repaired with
    /// `speech::repair`): beats follow the spoken sentences, EVOLVE snaps to
    /// word starts and a word-synced caption scene is added. `None` =
    /// byte-identical to 0.9.
    pub speech: Option<crate::speech::SpeechMap>,
    /// (0.11) `--footage file.footage.json`: interstitial clips + screen
    /// inserts. `None` = byte-identical.
    pub footage: Option<crate::footage::FootagePlan>,
    /// (0.12) `--art auto|<look>`: engine-chosen art direction
    /// (`art_direction`). `None` = byte-identical.
    pub art: Option<art_direction::ArtMode>,
    /// (0.10 Q) Story-keyed variety seed (`reel` sets it from the story):
    /// curated palette per look, grammar rotation for repeated typographic
    /// beats, rotating subject-first image layouts. `None` = byte-identical.
    pub variety: Option<u64>,
    /// (0.10 Q) With `speech`, do not add the caption scene (clean motion
    /// graphics without subtitles); beats then use the full canvas.
    pub no_captions: bool,
    /// (0.14) Transient-energy envelope of the music bed (operator-measured,
    /// `motion-render::music::energy_envelope`), stored in the project as
    /// envelope `"music"`; genre looks pulse hero layers to it. `None` =
    /// byte-identical.
    pub music_envelope: Option<crate::scene::Envelope>,
    /// (0.23) Take: "another version" of the same story. Read only under
    /// `variety`: take 0 is the story-keyed variety seed (today's product
    /// output), take > 0 compiles with `direction::take_seed(variety, take)`.
    /// Ignored without `variety` (byte-identical).
    pub take: u64,
    /// (0.23) Best-of-N: how many candidate directions are compiled and
    /// scored (0 and 1 both mean a single compile). Read only under
    /// `variety`; candidate k uses `direction::candidate_seed(take seed, k)`.
    pub candidates: u8,
}

/// (0.8) Compile with an optional MusicPlan: handoffs snap to its downbeats
/// (choreography v0). `None` is byte-identical to [`compile_full`].
pub fn compile_with_music(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
) -> Result<MotionProject, CompileError> {
    compile_with_options(
        intent,
        style,
        reference,
        library,
        measure,
        manifest,
        music,
        &CompileOptions::default(),
    )
}

/// (0.9) The full compile entry point with operator options.
///
/// `opts.explore == 0` is the canonical compile (byte-identical to 0.8). For
/// `explore > 0` the project is compiled at the requested level, checked
/// against the guardrails ([`explore::guardrail_report`]) and, on failure,
/// recompiled one level lower until it passes (level 0 always passes); the
/// outcome is recorded in `ProjectMeta.exploration`.
#[allow(clippy::too_many_arguments)]
pub fn compile_with_options(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
    opts: &CompileOptions,
) -> Result<MotionProject, CompileError> {
    compile_with_report(
        intent, style, reference, library, measure, manifest, music, opts,
    )
    .map(|(project, _)| project)
}

/// (0.20) [`compile_with_options`] plus the compile warnings
/// ([`CompileWarning`]): what was dropped or could read better. The project
/// is identical to the one `compile_with_options` returns.
#[allow(clippy::too_many_arguments)]
pub fn compile_with_report(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
    opts: &CompileOptions,
) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> {
    compile_candidate(
        intent, style, reference, library, measure, manifest, music, opts, 0,
    )
}

/// (0.23) Best-of-N: [`compile_with_report`] for candidate direction `k`. The
/// direction seed (`Ctx.direction_seed`) is
/// `direction::candidate_seed(direction::take_seed(variety, take), k)`, so
/// candidate 0 is the take itself: `k = 0` is exactly [`compile_with_report`].
/// Read only under `opts.variety` (without it there is no direction seed and
/// every `k` gives the same project).
#[allow(clippy::too_many_arguments)]
pub fn compile_candidate(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
    opts: &CompileOptions,
    k: u8,
) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> {
    compile_candidate_with(
        intent,
        style,
        reference,
        library,
        measure,
        manifest,
        music,
        opts,
        Candidate { k, params: None },
    )
}

/// (0.23 W2c) Test hook: [`compile_with_report`] with the beats' BeatParams
/// given (beat index → params) instead of planned. Beats not in `params` get
/// the identity; the planner does not run. Lets builder tests steer one beat
/// with a known preset; never used by the product path.
#[doc(hidden)]
#[allow(clippy::too_many_arguments)]
pub fn compile_with_beat_params(
    intent: &CreativeIntent,
    style: &StyleProfile,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    opts: &CompileOptions,
    params: &BTreeMap<usize, direction::BeatParams>,
) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> {
    compile_candidate_with(
        intent,
        style,
        None,
        library,
        measure,
        manifest,
        None,
        opts,
        Candidate {
            k: 0,
            params: Some(params),
        },
    )
}

/// Which candidate direction a compile builds: the best-of-N index, and (tests
/// only) BeatParams that replace the planner's.
#[derive(Clone, Copy)]
struct Candidate<'a> {
    k: u8,
    params: Option<&'a BTreeMap<usize, direction::BeatParams>>,
}

#[allow(clippy::too_many_arguments)]
fn compile_candidate_with(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
    opts: &CompileOptions,
    cand: Candidate<'_>,
) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> {
    // (0.10) Weak-model safety: an object whose asset exists nowhere (library
    // file, enabled family, manifest) used to abort the compile. Degrade every
    // subject naming that asset to a phrase (its meaning, else the humanised
    // name) and retry. Only paths that previously errored change.
    let mut owned: Option<CreativeIntent> = None;
    for _ in 0..=intent.beats.len() * 2 {
        let current = owned.as_ref().unwrap_or(intent);
        match compile_with_options_once(
            current, style, reference, library, measure, manifest, music, opts, cand,
        ) {
            Err(CompileError::Beat { message, .. })
                if message.starts_with("asset '") && message.contains("' not found in") =>
            {
                let name = message["asset '".len()..]
                    .split('\'')
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let mut next = current.clone();
                // Only well-formed names (the schema pattern) degrade; empty
                // or path-like names stay errors.
                let well_formed = name.len() <= 41
                    && name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if !well_formed || !degrade_missing_object(&mut next, &name) {
                    return Err(CompileError::Beat { beat: 0, message });
                }
                owned = Some(next);
            }
            Ok((project, mut warnings)) => {
                // (0.23) Values the intent gives that no look shows, judged on
                // the finished project against the intent the author wrote:
                // `current` may be the degraded one (a missing picture's
                // value is gone from it), and that loss is what is reported.
                warnings.extend(value_coverage::check(intent, &project));
                return Ok((project, warnings));
            }
            Err(e) => return Err(e),
        }
    }
    Err(CompileError::Invalid(
        "missing-asset fallback did not converge".into(),
    ))
}

/// Replace object subjects naming `asset` with phrases. Returns false when
/// nothing changed.
fn degrade_missing_object(intent: &mut CreativeIntent, asset: &str) -> bool {
    use crate::intent::{Atom, Subject};
    let mut changed = false;
    let fix = |s: &mut Subject, changed: &mut bool| {
        if let Subject::Object(o) = s {
            if o.asset == asset {
                let text = o
                    .meaning
                    .clone()
                    .unwrap_or_else(|| o.asset.replace(['_', '-'], " "));
                *s = Subject::Phrase(Atom {
                    value: Some(text),
                    meaning: None,
                });
                *changed = true;
            }
        }
    };
    let fix_items = |s: &mut Subject, changed: &mut bool| {
        if let Subject::Collection(c) = s {
            for item in &mut c.items {
                if let crate::intent::CollectionItem::Object(o) = item {
                    if o.asset == asset {
                        let text = o
                            .meaning
                            .clone()
                            .unwrap_or_else(|| o.asset.replace(['_', '-'], " "));
                        *item = crate::intent::CollectionItem::Phrase(Atom {
                            value: Some(text),
                            meaning: None,
                        });
                        *changed = true;
                    }
                }
            }
        }
    };
    for beat in &mut intent.beats {
        fix(&mut beat.primary, &mut changed);
        fix_items(&mut beat.primary, &mut changed);
        if let Some(sec) = beat.secondary.as_mut() {
            fix(sec, &mut changed);
            fix_items(sec, &mut changed);
        }
    }
    changed
}

#[allow(clippy::too_many_arguments)]
fn compile_with_options_once(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
    opts: &CompileOptions,
    cand: Candidate<'_>,
) -> Result<(MotionProject, Vec<CompileWarning>), CompileError> {
    let requested = opts.explore.min(3);
    if requested == 0 {
        return compile_at_level(
            intent, style, reference, library, measure, manifest, music, opts, 0, cand,
        )
        .map(|c| (c.project, c.warnings));
    }
    // Guardrails never judge a project harsher than its own canonical compile.
    let canonical = compile_at_level(
        intent, style, reference, library, measure, manifest, music, opts, 0, cand,
    )?;
    let guards = explore::Guardrails::relative_to(&canonical.project, &canonical.palette);
    let mut first_record: Option<explore::ExploreRecord> = None;
    let outcome = explore::guardrail_loop(
        requested,
        |level| {
            let c = compile_at_level(
                intent, style, reference, library, measure, manifest, music, opts, level, cand,
            )?;
            if level == requested {
                first_record = c.record.clone();
            }
            Ok(c)
        },
        |c| explore::guardrail_report_with(&c.project, &c.palette, &guards),
    )?;
    let explore::GuardrailOutcome {
        value: compiled,
        level: final_level,
        fallbacks,
    } = outcome;
    let mut project = compiled.project;
    project.project.exploration = first_record.map(|first| {
        explore::finalize_record(
            first,
            final_level,
            compiled.record,
            &compiled.typography_value,
            &fallbacks,
        )
    });
    Ok((project, compiled.warnings))
}

/// One compile at a fixed exploration `level` (no guardrail loop).
struct Compiled {
    project: MotionProject,
    palette: Palette,
    record: Option<explore::ExploreRecord>,
    typography_value: String,
    /// (0.20) Warnings raised while compiling (see [`CompileWarning`]).
    warnings: Vec<CompileWarning>,
}

#[allow(clippy::too_many_arguments)]
fn compile_at_level(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
    manifest: &AssetManifest,
    music: Option<&crate::audio::MusicPlan>,
    opts: &CompileOptions,
    level: u8,
    cand: Candidate<'_>,
) -> Result<Compiled, CompileError> {
    if intent.version != INTENT_VERSION && intent.version != INTENT_VERSION_V0_1 {
        return Err(CompileError::Version(intent.version.clone()));
    }
    intent
        .validate()
        .map_err(|errs| CompileError::Invalid(errs.join("; ")))?;
    if intent.beats.is_empty() {
        return Err(CompileError::NoBeats);
    }
    let missing = manifest.missing_required();
    if !missing.is_empty() {
        return Err(CompileError::MissingRequiredAsset(
            missing
                .iter()
                .map(|m| format!("{} ({})", m.id, m.reason))
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }

    // (0.10 Q) Art direction: the look follows the emotion the taste evokes
    // (or the operator's forced look); its asset families serve object names
    // when the operator enabled none, so weak-model nouns find pictures.
    let art = opts.art.map(|mode| {
        let resolved = resolve_taste(intent, style, reference);
        let emotion = opts
            .emotion
            .unwrap_or_else(|| typography::resolve_emotion(&resolved));
        art_direction::resolve_with_genre(mode, emotion, resolved.genre)
    });
    let art_library;
    let library = match &art {
        Some(a) if library.families.is_empty() => {
            art_library = library
                .clone()
                .with_families(a.families.iter().map(|f| f.to_string()).collect());
            &art_library
        }
        _ => library,
    };

    // (0.8) Library catalog matches are delivered exactly like an external
    // generator's images: through the manifest, so grammar selection sees them.
    let explored_library;
    let library = if level >= 2 && !library.families.is_empty() {
        explored_library = library.clone().with_exploration(level, opts.seed);
        &explored_library
    } else {
        library
    };
    let augmented = library_delivery(
        intent,
        style,
        reference,
        library,
        manifest,
        art.as_ref().and_then(|a| a.fx.grammar),
    );
    let manifest = augmented.as_ref().unwrap_or(manifest);

    // (0.9) The operator canvas overrides the intent format; the LayoutFrame
    // is derived from (W, H) only.
    let (w, h) = opts
        .canvas
        .unwrap_or_else(|| layout_frame::LayoutFrame::legacy_size(intent.format));
    let frame =
        layout_frame::LayoutFrame::new(w, h).map_err(|e| CompileError::Invalid(e.to_string()))?;
    let story_key = story_key_of(intent);
    // (0.21) The piece's brand colours (applied once the look has its palette).
    let brand = match style.brand.as_ref().filter(|b| !b.is_empty()) {
        Some(b) => Some(brand::parse(b).map_err(CompileError::Invalid)?),
        None => None,
    };
    let (taste, mut record) = explore::apply_exploration(
        &resolve_taste(intent, style, reference),
        level,
        opts.seed,
        story_key,
    );
    let style = &taste.effective;
    let mut palette = taste.palette.colors.clone();
    // (0.10 Q) A curated palette of the look (variety seed picks among those
    // matching the resolved polarity) replaces the tone's single palette.
    let mut art_record = None;
    let mut plate_id = None;
    if let Some(a) = &art {
        let dark = taste.palette.polarity == taste::ResolvedPolarity::Dark;
        // (0.23 W2e, DECISION 136) Under variety the technical tone in the
        // journey look takes the precision variant: its own palettes and no
        // plate (the measurement grid on a clean ground), so calm editorial
        // and technical never share look + palette + ground.
        let precision = opts.variety.is_some()
            && a.look == art_direction::Look::Journey
            && style.tone == crate::style::Tone::Technical;
        let spec = if precision {
            art_direction::precision_palette_for(dark, opts.variety.unwrap_or(0))
        } else {
            art_direction::palette_for(a.look, dark, opts.variety.unwrap_or(0))
        };
        palette = art_direction::palette_of(&spec);
        let plate = if spec.dark {
            a.plate_dark
        } else {
            a.plate_light
        };
        let plate_path = format!("library/grounds/{plate}.png");
        if !precision && library.root.join(&plate_path).is_file() {
            plate_id = Some(plate.to_string());
        }
        art_record = Some(art_direction::ArtRecord {
            look: a.look.name().to_string(),
            // Precision palettes are recorded as 100 + their index.
            palette: if precision {
                100 + art_direction::PRECISION_PALETTES
                    .iter()
                    .position(|p| *p == spec)
                    .unwrap_or(0)
            } else {
                art_direction::palette_index(a.look, &spec)
            },
            plate: plate_id.clone(),
            families: library.families.clone(),
            sfx: a.sfx.name().to_string(),
            focal: Default::default(),
            reveals: Default::default(),
        });
        // (0.23 A4) With variety the palette's text colours (muted, accent) are
        // moved toward the ink, hue kept, until they read on the card, the paper
        // and the paper under this look's plate. Without variety the curated
        // palette is used as it is.
        if opts.variety.is_some() {
            let tuned = art_direction::variety_text_tokens(
                &spec,
                plate_id.as_deref().map(|p| (p, a.plate_opacity)),
            );
            palette = art_direction::palette_of(&tuned);
        }
    }
    // (0.21) Brand colours replace the look's; a brand background replaces
    // the look's paper plate too. Readability fixes become compile warnings.
    let mut brand_notes = Vec::new();
    if let Some(b) = &brand {
        let (branded, notes) = brand::apply(&palette, b);
        palette = branded;
        brand_notes = notes;
        if b.background.is_some() {
            plate_id = None;
            if let Some(r) = art_record.as_mut() {
                r.plate = None;
            }
        }
    }
    let (typography_choice, fonts) = typography::resolve_typography(
        &taste,
        &typography::TypographyRequest {
            exploration: level,
            seed: opts.seed,
            story_key,
            emotion: opts.emotion,
        },
        &library.root,
    );
    if let Some(r) = record.as_mut() {
        // The canonical (level 0, no override) choice for the record.
        let (canonical, _) = typography::resolve_typography(
            &taste,
            &typography::TypographyRequest {
                exploration: 0,
                seed: opts.seed,
                story_key,
                emotion: None,
            },
            &library.root,
        );
        explore::set_typography_choice(r, &typography_choice, &canonical);
    }
    let typography_value = explore::typography_value(&typography_choice);
    let ts = Typesetter::new(measure, &library.root, &fonts)
        .with_scale(temporal::scale_gains(taste.scale));
    let mut ctx = Ctx {
        reveals: Default::default(),
        warnings: Vec::new(),
        direction_seed: opts
            .variety
            .map(|v| direction::candidate_seed(direction::take_seed(v, opts.take), cand.k)),
        beat_params: BTreeMap::new(),
        direction_take: opts.variety.map(|v| (v, opts.take, cand.k)),
        direction_beats: BTreeMap::new(),
        emotion: Some(typography_choice.emotion),
        art: art.clone().map(|a| (a, plate_id.clone())),
        w: w as f32,
        h: h as f32,
        u: (w.min(h)) as f32 / 1080.0,
        style,
        taste: taste.clone(),
        palette,
        ts,
        library,
        assets: BTreeMap::new(),
        format: frame.class,
        frame,
        manifest,
        image_carry: BTreeSet::new(),
        focal: BTreeMap::new(),
        stack: None,
        spoken: Vec::new(),
        beat_count: intent.beats.len(),
        sequence: sequence::detect(intent),
    };
    ctx.warnings
        .extend(brand_notes.into_iter().map(|message| CompileWarning {
            code: WARN_BRAND_CONTRAST.to_string(),
            beat: None,
            message,
        }));
    if let Some(plate) = &plate_id {
        let id = format!("asset.plate.{plate}");
        ctx.assets.insert(
            id.clone(),
            Asset {
                id,
                kind: AssetKind::Image,
                path: format!("library/grounds/{plate}.png"),
                sprite: None,
            },
        );
    }
    for face in &fonts.faces {
        ctx.assets.insert(
            face.asset_id.to_string(),
            Asset {
                id: face.asset_id.to_string(),
                kind: AssetKind::Font,
                path: face.path.to_string(),
                sprite: None,
            },
        );
    }

    // (0.10) A voice-over leads: beats follow the sentences, music snapping is
    // ignored, and each EVOLVE event snaps to a word start.
    let (mut plans, mut speech_record) = match &opts.speech {
        Some(speech) => {
            let mut plans = plan_timing_speech(intent, &taste, speech)?;
            // (0.20) Phases from the beat's own words, then EVOLVE onto a word.
            let phases = speech_lifecycle::apply(&mut plans, intent, speech);
            // Builders enter at the ENTER speech placed; an EVOLVE placed on a
            // word is not snapped again (the snap could pull it off its
            // minimum spans).
            let mut speech_evolves = Vec::new();
            for ph in &phases {
                if let Some(plan) = plans.get_mut(ph.beat) {
                    if ph.enter == crate::speech::PhaseSource::Speech {
                        plan.enter_override = Some(plan.life.enter);
                    }
                    if ph.evolve == crate::speech::PhaseSource::Speech {
                        speech_evolves.push(ph.beat);
                    }
                }
            }
            let evolve_snaps = speech_plan::snap_evolve(&mut plans, speech, &speech_evolves);
            // Unmatched words are measured against what is spoken (narration,
            // else statement), the same lines the speech map was repaired with.
            let statements: Vec<String> = intent
                .beats
                .iter()
                .map(|b| taste_rules::display_text(b, true).spoken)
                .collect();
            let record = crate::speech::SpeechRecord {
                model: speech.model.clone(),
                voice: speech.voice.clone(),
                words: speech.words.len(),
                evolve_snaps,
                unmatched: crate::speech::unmatched_words(speech, &statements),
                music_ignored: music.is_some(),
                word_cues: Vec::new(),
                phases,
            };
            (plans, Some(record))
        }
        None => (
            plan_timing_music(intent, &taste, music.map(|m| m.downbeat_times.as_slice())),
            None,
        ),
    };
    let total = plans.last().map(|p| p.start + p.duration).unwrap_or(0.0);
    ctx.image_carry = grammar::plate::image_carries(&ctx, &plans, &intent.beats);
    // (0.10 Q) With a voice-over the captions own the lower lane: beat content
    // is laid out above it (furniture and captions keep the real canvas).
    // A look whose words are kinetic type (studio) replaces the caption track.
    let kinetic = ctx.art.as_ref().is_some_and(|(a, _)| a.fx.kinetic_words);
    let captions_on = opts.speech.is_some() && !opts.no_captions && !kinetic;
    let caption_reserve = captions_on.then(|| captions::beat_bottom_reserve(&ctx.frame));
    // (0.19) Layers: one geometry for the persistent column (backdrop) and
    // the beat builders, on the canvas above the caption lane.
    ctx.stack = layer_stack::StackPlan::new(
        layer_stack::StackGeo {
            w: ctx.w,
            h: ctx.h,
            u: ctx.u,
            content_h: if captions_on {
                captions::content_limit(&ctx.frame).min(ctx.h)
            } else {
                ctx.h
            },
            margin: ctx.margin(),
        },
        &intent.beats,
    );
    // (0.10 Q) Subject-first layout per image beat (rotation under `variety`,
    // legacy interlock otherwise unless it would cover a head), judged on the
    // canvas the builders will see.
    grammar::plan_image_layouts(
        &mut ctx,
        &mut plans,
        &intent.beats,
        opts.variety,
        caption_reserve,
    );

    let mut carries: Vec<Carry> = Vec::new();
    // (0.23 A4) Index 0 is the backdrop. Print fields keep clear of the beats'
    // text, so the backdrop is composed once the beats exist (below); a
    // backdrop without print fields does not depend on them and is built here.
    let text_aware = backdrop::wants_text_zones(&ctx);
    let mut backdrop_scene = if text_aware {
        backdrop::placeholder(total)
    } else {
        backdrop::backdrop_scene(&ctx, &plans, total)
    };
    // The persistent column is laid out before the beats either way: laying
    // it out leaves the typesetter in a state the beats' fits depend on.
    let mut stacked = None;
    if let Some(stack) = &ctx.stack {
        let (layers, motions) = layer_stack::columns(&ctx, stack, &plans, &intent.beats);
        if text_aware {
            stacked = Some((layers, motions));
        } else {
            backdrop_scene.layers.extend(layers);
            backdrop_scene.motions.extend(motions);
        }
    }
    // (0.23 W2d) The direction planner: under a direction seed every beat gets
    // a role in the arc and BeatParams drawn from the look's vocabulary (a
    // test may give the params instead). Without a seed nothing is planned and
    // every builder takes its own constants.
    let mut roles: Vec<direction::Role> = Vec::new();
    if let Some(given) = cand.params {
        ctx.beat_params = given.clone();
    } else if let (Some(variety), Some(_)) = (opts.variety, ctx.direction_seed) {
        let slots: Vec<direction::BeatSlot> = intent
            .beats
            .iter()
            .map(|b| direction::BeatSlot {
                energy: b.energy,
                reveal: b.purpose == Purpose::Reveal,
            })
            .collect();
        roles = direction::roles(&slots);
        let vocab = direction::vocabulary(ctx.art.as_ref().map(|(a, _)| a.look), style.tone);
        ctx.beat_params = direction::plan_take(variety, opts.take, cand.k, &roles, vocab)
            .into_iter()
            .enumerate()
            .collect();
    }
    let mut scenes = vec![backdrop_scene];
    for (plan, beat) in plans.iter().zip(&intent.beats) {
        ctx.spoken = match &opts.speech {
            Some(speech) => speech
                .sentences
                .iter()
                .find(|s| s.beat == plan.index)
                .map(|sentence| {
                    speech
                        .words_in(sentence)
                        .iter()
                        .map(|w| (w.text.clone(), round3(w.start - plan.start)))
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let mut scene = recipes::build_beat(&mut ctx, plan, beat, &mut carries, caption_reserve)?;
        // (0.10 Q) With a voice-over, content enters when it is spoken.
        if let Some(speech) = &opts.speech {
            if let Some(sentence) = speech.sentences.iter().find(|s| s.beat == plan.index) {
                let words: Vec<(String, f64)> = speech
                    .words_in(sentence)
                    .iter()
                    .map(|w| (w.text.clone(), w.start - scene.start_seconds))
                    .collect();
                // (0.20) Explicit anchors first (`B::reveal`), name matching
                // as the fallback; the look decides whether the title waits.
                let anchors = ctx.reveals.get(&plan.index).cloned().unwrap_or_default();
                let policy = speech_plan::CuePolicy {
                    allow_delay: true,
                    min_read: crate::speech::CUE_MIN_READ,
                    title: ctx
                        .art
                        .as_ref()
                        .map(|(a, _)| a.fx.title_reveal)
                        .unwrap_or_default(),
                    // (0.22) The focal card must have arrived by READ.
                    focal: ctx.focal.get(&plan.index).cloned(),
                };
                let cues = speech_plan::apply_word_cues(&mut scene, &words, &anchors, &policy);
                for m in cues.moves.iter().filter(|m| m.clamped) {
                    ctx.warnings.push(CompileWarning {
                        code: WARN_CUE_CLAMPED.into(),
                        beat: Some(plan.index),
                        message: format!(
                            "\"{}\" could not move fully onto \"{}\" (planned {:.2} s, moved to {:.2} s): the read-time floor or ENTER limited it",
                            m.group, m.word, m.from, m.to
                        ),
                    });
                }
                // (0.22) An anchored group no cue could move is reported, never
                // skipped silently.
                for d in &cues.dropped {
                    let early = d.early();
                    let message = if early >= 0.0 {
                        format!(
                            "\"{}\" could not wait for \"{}\" (no room before the beat ends); it shows {:.1} s early",
                            d.group, d.word, early
                        )
                    } else {
                        format!(
                            "\"{}\" could not come forward to \"{}\" (its motions run past the beat's end); it shows {:.1} s late",
                            d.group, d.word, -early
                        )
                    };
                    ctx.warnings.push(CompileWarning {
                        code: WARN_CUE_DROPPED.into(),
                        beat: Some(plan.index),
                        message,
                    });
                }
                if let Some(rec) = speech_record.as_mut() {
                    for m in cues.moves {
                        rec.word_cues.push(crate::speech::WordCue {
                            beat: plan.index,
                            group: m.group,
                            word: m.word,
                            from: m.from,
                            to: m.to,
                            role: m.role,
                            clamped: m.clamped,
                        });
                    }
                }
            }
        }
        scenes.push(scene);
    }
    // (0.14) Genre looks: finishing effects (post, shakes, echoes, pulses).
    let camera_picks = match &ctx.art {
        Some((a, _)) => fx::apply_look_fx(
            &ctx,
            &mut scenes[1..],
            &plans,
            &intent.beats,
            opts.speech.as_ref(),
            &a.fx,
            opts.music_envelope.as_ref().map(|_| "music"),
        ),
        None => Vec::new(),
    };
    // (0.23 B2a) The camera moves the seeded rotation chose join the beats'
    // direction records.
    for (beat, mv) in camera_picks {
        grammar::note_rotation(&mut ctx, beat, "camera", mv);
    }
    // (0.10) Word-synced captions sit above every beat scene.
    if let Some(speech) = opts.speech.as_ref().filter(|_| captions_on) {
        let beat_scenes = scenes[1..].to_vec();
        scenes.push(captions::caption_scene(
            &mut ctx,
            &plans,
            &intent.beats,
            speech,
            &beat_scenes,
        )?);
    }
    // (0.23 A4) The backdrop of a print-field look, composed around the text.
    if text_aware {
        let zones = backdrop::text_zones(&ctx, &scenes[1..]);
        let mut scene = backdrop::backdrop_scene_around(&ctx, &plans, total, &zones);
        if let Some((layers, motions)) = stacked {
            scene.layers.extend(layers);
            scene.motions.extend(motions);
        }
        scenes[0] = scene;
    }

    let theme = Theme {
        fonts: fonts
            .roles
            .iter()
            .map(|(role, face)| (*role, face.asset_id.to_string()))
            .collect(),
        palette: ctx.palette.named(),
        // Recorded only when the registry (not the 0.8 pairing) chose the faces.
        typography: (typography_choice.option.is_some()
            || !typography_choice.fallback_from.is_empty())
        .then_some(typography_choice),
    };
    let final_palette = ctx.palette.clone();
    let art_families = art.as_ref().map(|_| library.families.clone());
    let library_root = library.root.clone();
    // (0.18) What each beat is about, by scene id (scenes[0] is the backdrop).
    if let Some(rec) = art_record.as_mut() {
        for (i, id) in &ctx.focal {
            if let Some(scene) = scenes.get(i + 1) {
                rec.focal.insert(scene.id.clone(), id.clone());
            }
        }
        // (0.20) The reveal anchors each builder declared, by scene id.
        for (i, anchors) in &ctx.reveals {
            if let Some(scene) = scenes.get(i + 1) {
                rec.reveals.insert(scene.id.clone(), anchors.clone());
            }
        }
    }
    // (0.20) Compile warnings: what builders dropped, plus the story-level
    // checks. Returned beside the project, never written into it.
    let mut warnings = std::mem::take(&mut ctx.warnings);
    warnings.extend(story_warnings::check(
        intent,
        &ctx,
        &scenes,
        &carries,
        opts.speech.as_ref(),
    ));
    // (0.23 B2a) Exploration reports each beat's template candidates; under a
    // direction seed the choices (template alternate, seeded rotations) are
    // written to `ProjectMeta.direction`.
    if let Some(r) = record.as_mut() {
        let alternates: Vec<(u8, u8)> = ctx
            .direction_beats
            .values()
            .map(|d| (d.alternate, d.alternates))
            .collect();
        explore::set_grammar_alternates(r, &alternates);
    }
    for (i, d) in ctx.direction_beats.iter_mut() {
        d.params = ctx.beat_params.get(i).copied().unwrap_or_default();
        if let Some(role) = roles.get(*i) {
            d.role = role.name().to_string();
        }
    }
    let direction = opts.variety.map(|variety| direction::DirectionRecord {
        take: opts.take,
        seed: direction::take_seed(variety, opts.take),
        chosen: 0,
        beats: ctx.direction_beats.values().cloned().collect(),
        candidates: Vec::new(),
        shipped_with_failure: None,
    });
    let mut project = MotionProject {
        envelopes: opts
            .music_envelope
            .clone()
            .map(|e| crate::scene::Envelope {
                id: "music".into(),
                ..e
            })
            .into_iter()
            .collect(),
        version: SCENE_VERSION.to_string(),
        project: ProjectMeta {
            name: intent.title.clone(),
            duration_seconds: Some(round3(total)),
            exploration: record.clone(),
            speech: speech_record,
            art: art_record,
            direction,
        },
        canvas: Canvas {
            width: w,
            height: h,
            fps: 30,
            background: ctx.palette.paper,
        },
        theme,
        assets: ctx.assets.into_values().collect(),
        asset_root: None,
        scenes,
        shared: carries.into_iter().map(|c| c.element).collect(),
    };
    // (0.10 Q) Art direction brings library assets to life: hero loops and
    // screen inserts from the families' loop catalogs.
    if let Some(families) = art_families {
        loops::animate_library_assets(&mut project, &library_root, &families);
    }
    Ok(Compiled {
        project,
        palette: final_palette,
        record,
        typography_value,
        warnings,
    })
}

/// (0.8) The caller's manifest plus one entry per planned library-catalog
/// match the caller did not already deliver. `None` when nothing is added
/// (always, when no families are enabled).
fn library_delivery(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&taste::ReferencePrinciples>,
    library: &AssetLibrary,
    manifest: &AssetManifest,
    look_grammar: Option<art_direction::LookGrammar>,
) -> Option<AssetManifest> {
    if library.families.is_empty() {
        return None;
    }
    let plan =
        asset_plan::plan_assets_for_look(intent, style, reference, library, look_grammar).ok()?;
    let mut out = manifest.clone();
    for request in &plan.requests {
        let Some(lib) = request.library_asset.as_deref().filter(|l| l.contains('/')) else {
            continue;
        };
        if manifest.get(&request.id).is_some() {
            continue;
        }
        if let Some(entry) = library.catalog_delivery(lib, &request.id) {
            out.assets.push(entry);
        }
    }
    (out.assets.len() != manifest.assets.len()).then_some(out)
}

// ---------------------------------------------------------------------------
// Shared compile context
// ---------------------------------------------------------------------------

pub(crate) struct Ctx<'a> {
    /// (0.10) The typography emotion actually used (override/exploration
    /// included); `None` in unit-test contexts → derived from the taste.
    pub emotion: Option<typography::Emotion>,
    /// (0.10 Q) Art direction in force (`--art`), with its plate asset id.
    pub art: Option<(art_direction::ArtDirection, Option<String>)>,
    pub w: f32,
    pub h: f32,
    /// Unit: 1.0 at 1080 px short side.
    pub u: f32,
    /// The effective style (explicit fields + the tone's presets); see `taste.rs`.
    pub style: &'a StyleProfile,
    /// (0.6) The resolved design system, including temporal character.
    pub taste: taste::ResolvedStyleProfile,
    pub palette: Palette,
    pub ts: Typesetter<'a>,
    pub library: &'a AssetLibrary,
    pub assets: BTreeMap<String, Asset>,
    pub format: Format,
    /// (0.9) Canvas geometry (class, continuous aspect weights, safe area).
    pub frame: layout_frame::LayoutFrame,
    /// Delivered images (paths relative to `library.root`).
    pub manifest: &'a AssetManifest,
    /// (0.5) Beats (0-based) whose delivered subject image continues into the
    /// next beat as one SharedElement (same manifest entry, both beats
    /// TypeImageInterlock). Decided before any beat is built.
    pub image_carry: BTreeSet<usize>,
    /// (0.7.1) Number of beats in the piece (process progress = index / count).
    pub beat_count: usize,
    /// (0.21) Whether the story counts (a countdown or a list in order) and the
    /// rank each beat shows. Beat numbers appear only on ranked beats.
    pub sequence: sequence::Sequence,
    /// (0.18) Beat index -> the layer the beat is about, as its builder
    /// declared it (`B::focal`). The FX director focuses the camera on it.
    pub focal: BTreeMap<usize, String>,
    /// (0.19) The layers of the piece (`Subject::Layers`): the geometry the
    /// persistent column and the beat builders share, and which beats form
    /// a run. `None` when no beat is about layers.
    pub stack: Option<layer_stack::StackPlan>,
    /// (0.19) The words the narrator speaks in the beat being built, with their
    /// start times in scene-local seconds (empty without a voice-over). Builders
    /// that bring a picture in as it is named read it through
    /// `speech_plan::name_time`.
    pub spoken: Vec<(String, f64)>,
    /// (0.20) Beat index -> the reveal anchors its builder declared
    /// (`B::reveal`), recorded in `ArtRecord.reveals` by scene id.
    pub reveals: BTreeMap<usize, Vec<crate::speech::RevealAnchor>>,
    /// (0.20) Compile warnings builders raise (a dropped request, a title
    /// that spoils its line); returned beside the project, never written
    /// into it.
    pub warnings: Vec<CompileWarning>,
    /// (0.23) Seed of this compile's direction choices: the take seed
    /// (`direction::take_seed`), or a best-of-N candidate's seed. `None`
    /// without variety: every seeded choice keeps its pre-0.23 rule.
    #[allow(dead_code)] // read by the 0.23 rotation / alternate tasks
    pub direction_seed: Option<u64>,
    /// (0.23) Per-beat direction parameters chosen by the direction planner;
    /// a beat without an entry uses `BeatParams::default()` (the identity).
    #[allow(dead_code)] // filled by the 0.23 direction planner
    pub beat_params: BTreeMap<usize, direction::BeatParams>,
    /// (0.23 W2d) `(variety, take, candidate)` under a direction seed: lets a
    /// take rotate its choices away from take 0's (`grammar::choose`).
    pub direction_take: Option<(u64, u64, u8)>,
    /// (0.23 B2a) What the direction layer chose for each beat, by beat index:
    /// the template alternate (`grammar::choose`) and every seeded rotation that
    /// replaced a position rule. Filled by the builders; written to
    /// `ProjectMeta.direction` only under a direction seed (the template part is
    /// also what exploration reports for its `GrammarAlternate` dimension).
    pub direction_beats: BTreeMap<usize, direction::BeatDirection>,
}

impl Ctx<'_> {
    /// (0.23) The direction parameters of beat `index` (identity when the
    /// planner set none).
    #[allow(dead_code)] // read by the 0.23 builder tasks
    pub fn params_for(&self, index: usize) -> direction::BeatParams {
        self.beat_params.get(&index).copied().unwrap_or_default()
    }

    pub fn margin(&self) -> f32 {
        // 84u: the LayoutFrame safe-area inset.
        self.frame.safe.x
    }

    /// Register a library object as a project asset; returns (asset id, kind).
    pub fn object_asset(
        &mut self,
        name: &str,
        beat: usize,
    ) -> Result<(String, AssetKind), CompileError> {
        self.object_asset_for(name, beat, None)
    }

    /// (0.9) Manifest-first object resolution. When `request_id` is the id the
    /// asset planner gave this object and the manifest has an entry serving it
    /// (a catalog match or a delivered image), that entry is registered as an
    /// image asset `asset.<entry id>` (as `plate::image_plate` does); otherwise
    /// the object is looked up by library name.
    pub fn object_asset_for(
        &mut self,
        name: &str,
        beat: usize,
        request_id: Option<&str>,
    ) -> Result<(String, AssetKind), CompileError> {
        if let Some(entry) = request_id.and_then(|id| self.manifest.get(id)) {
            let id = format!("asset.{}", entry.id);
            self.assets.insert(
                id.clone(),
                Asset {
                    id: id.clone(),
                    kind: AssetKind::Image,
                    path: entry.path.clone(),
                    sprite: None,
                },
            );
            return Ok((id, AssetKind::Image));
        }
        let (kind, path) = self
            .library
            .find_object(name)
            .ok_or_else(|| CompileError::Beat {
                beat,
                message: format!(
                    "asset '{name}' not found in {}/library (svg or png)",
                    self.library.root.display()
                ),
            })?;
        let id = format!("asset.{name}");
        self.assets.insert(
            id.clone(),
            Asset {
                id: id.clone(),
                kind,
                path,
                sprite: None,
            },
        );
        Ok((id, kind))
    }

    pub fn grain_intensity(&self) -> Option<f32> {
        match self.style.texture_style {
            TextureStyle::None => None,
            TextureStyle::SubtlePrint => Some(0.32),
            TextureStyle::HeavyPrint => Some(0.6),
        }
    }

    pub fn layered(&self) -> bool {
        self.style.depth == Depth::Layered
    }
}

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) struct BeatPlan {
    pub index: usize,
    pub id: String,
    /// Layer id prefix, e.g. `b1`.
    pub prefix: String,
    pub start: f64,
    pub duration: f64,
    /// Overlap with the previous beat (the incoming transition).
    pub overlap_in: f64,
    /// Overlap with the next beat (the outgoing transition).
    pub overlap_out: f64,
    /// Resolved motion language for this beat (how things move).
    pub lang: MotionLanguageProfile,
    /// This beat enters with an accent wipe (incoming energy = impact).
    pub accent_in: bool,
    /// The next beat enters with an accent wipe.
    pub accent_out: bool,
    /// (0.6) This beat enters behind a panel wipe (transition character).
    pub wipe_in: bool,
    /// (0.6) The next beat enters behind a panel wipe.
    pub wipe_out: bool,
    pub is_last: bool,
    pub seed: u64,
    /// (0.20) ENTER as the speech-aware lifecycle placed it (scene-local),
    /// when it moved; builders read it through [`BeatPlan::enter_at`]. `None`
    /// keeps the overlap-based entrance (byte-identical without speech).
    pub enter_override: Option<f64>,
    /// Planned lifecycle phases (scene-local seconds). Compositions schedule
    /// secondary information into EVOLVE and preparation into ANTICIPATE.
    pub life: Lifecycle,
    /// (0.10 Q) Layout of a TypeImageInterlock beat (`Legacy` = the 0.9
    /// interlock; set by `grammar::plan_image_layouts` once the manifest is
    /// known, `Legacy` for every other beat).
    pub image_layout: grammar::ImageLayout,
    /// (0.10 Q) Smallest subject alpha-bounds area (canvas pixels²) the beat's
    /// image layout must reach: `SUBJECT_MIN_AREA` of the REAL canvas, which
    /// stays the measure when builders lay out on a canvas shortened by the
    /// caption lane. `0.0` = not planned (derived from the layout frame).
    pub image_min_px: f32,
    /// (0.10 Q) What the beat shows and says (`taste_rules::display_text`):
    /// the on-screen title (the statement when it is short), the small body
    /// copy of a long statement without a voice-over, and the spoken line.
    pub display: taste_rules::DisplayText,
}

impl BeatPlan {
    /// Scene-local time when this beat's own content starts entering.
    pub fn enter_at(&self) -> f64 {
        if let Some(t) = self.enter_override {
            return t;
        }
        if self.index == 0 {
            0.25
        } else if self.accent_in {
            0.42
        } else {
            (self.overlap_in * 0.75).max(0.15)
        }
    }
    /// Scene-local time when the outgoing transition begins.
    pub fn exit_at(&self) -> f64 {
        self.duration - self.overlap_out
    }
}

/// The motion language for a beat: the style's explicit choice, or (auto) a
/// choice from what the beat communicates — never from its topic.
pub(crate) fn language_for(beat: &Beat, style: &StyleProfile) -> Language {
    if let Some(l) = Language::from_style(style.motion_language) {
        return l;
    }
    // v0.2 structured subjects: the structure decides how it moves.
    match recipes::structure_of(beat) {
        Some(recipes::Structure::Collection) => {
            let all_numbers = [Some(&beat.primary), beat.secondary.as_ref()]
                .into_iter()
                .flatten()
                .filter_map(|s| match s {
                    Subject::Collection(c) => Some(c),
                    _ => None,
                })
                .all(|c| {
                    c.items.iter().all(|i| {
                        matches!(i, crate::intent::CollectionItem::Number(a)
                            if a.value.as_deref().and_then(parse_count).is_some())
                    })
                });
            return if all_numbers {
                Language::Data
            } else {
                Language::Sequential
            };
        }
        Some(recipes::Structure::StateChange) => return Language::Kinetic,
        Some(recipes::Structure::DerivedMetric) => return Language::Data,
        // The column is the motion; the headline stays restrained.
        Some(recipes::Structure::Layers) => return Language::Minimal,
        None => {}
    }
    let countable = [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
        .any(|s| s.kind() == SubjectKind::Number && s.value().and_then(parse_count).is_some());
    let keyword_in_statement = beat.keyword.as_deref().is_some_and(|k| {
        let k = k.to_lowercase();
        beat.statement.split_whitespace().any(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
                == k
        })
    });
    match beat.purpose {
        _ if countable => Language::Data,
        Purpose::Explain => Language::Sequential,
        Purpose::Emphasize if beat.energy != Energy::Calm && keyword_in_statement => {
            Language::Kinetic
        }
        // Calm beats stay restrained.
        _ if beat.energy == Energy::Calm => Language::Minimal,
        _ if style.depth == Depth::Layered && style.camera_style != CameraStyle::Static => {
            Language::Parallax
        }
        _ => Language::Minimal,
    }
}

/// A number a counter can count to: `(value, decimals, grouping, prefix, suffix)`.
/// Accepts an optional non-digit prefix/suffix around one number, e.g.
/// `₹42,000`, `40%`, `3.5x`, `-12`. Anything else (ranges, words) is `None`.
pub(crate) fn parse_count(s: &str) -> Option<(f64, u8, bool, String, String)> {
    let s = s.trim();
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let end = s.rfind(|c: char| c.is_ascii_digit())? + 1;
    let (mut prefix, core, suffix) = (&s[..start], &s[start..end], &s[end..]);
    let negative = prefix.ends_with('-');
    if negative {
        prefix = &prefix[..prefix.len() - 1];
    }
    let is_affix = |a: &str| {
        a.chars().count() <= 3 && !a.chars().any(|c| c.is_ascii_digit() || c.is_whitespace())
    };
    if !is_affix(prefix) || !is_affix(suffix) {
        return None;
    }
    if !core
        .chars()
        .all(|c| c.is_ascii_digit() || c == ',' || c == '.')
        || core.matches('.').count() > 1
    {
        return None;
    }
    let grouping = core.contains(',');
    let decimals = core.split_once('.').map(|(_, d)| d.len()).unwrap_or(0);
    if decimals > 3 {
        return None;
    }
    let value: f64 = core.replace(',', "").parse().ok()?;
    let value = if negative { -value } else { value };
    Some((
        value,
        decimals as u8,
        grouping,
        prefix.to_string(),
        suffix.to_string(),
    ))
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// (0.10 Q) What a beat shows and says: `taste_rules::display_text`, except
/// that a derived title which only repeats a phrase the beat shows anyway (its
/// primary phrase is the usual pick) gives way to the next candidate (the
/// keyword, then the first words of the statement), so the screen never prints
/// the same words as headline and as subject. Short statements are untouched.
fn beat_display(beat: &Beat, speech: bool) -> taste_rules::DisplayText {
    let display = taste_rules::display_text(beat, speech);
    if !display.derived {
        return display;
    }
    let key = |s: &str| {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let shown: Vec<String> = [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(Subject::display_text)
        .map(key)
        .collect();
    let repeats = |title: &str| shown.contains(&key(title));
    if !repeats(&display.title) {
        return display;
    }
    // Candidates after the primary phrase: the keyword, then the clause.
    let mut rest = beat.clone();
    rest.primary = Subject::Number(crate::intent::Atom::default());
    let mut next = taste_rules::display_text(&rest, speech);
    if repeats(&next.title) {
        rest.keyword = None;
        next = taste_rules::display_text(&rest, speech);
    }
    next
}

/// Seconds a beat needs: a base hold by energy, reading time, and extra time
/// for relationships that play out on screen.
fn beat_duration(beat: &Beat) -> f64 {
    let base = match beat.energy {
        Energy::Calm => 2.6,
        Energy::Building => 2.5,
        Energy::Impact => 2.3,
    };
    let subject_words = [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
        .filter_map(Subject::display_text)
        .map(words)
        .sum::<usize>();
    let reading = words(&beat.statement) as f64 * 0.28 + subject_words as f64 * 0.2;
    let relation = match (beat.purpose, beat.relationship) {
        (Purpose::Contrast | Purpose::Compare, _) => 0.9,
        (
            _,
            Some(
                Relationship::Compress
                | Relationship::Grow
                | Relationship::Separate
                | Relationship::Replace,
            ),
        ) => 0.6,
        _ => 0.0,
    };
    let structure = structure_time(beat);
    let cap = if structure > 0.0 { 7.5 } else { 6.0 };
    (base + reading + relation + structure).clamp(3.2, cap)
}

/// Extra on-screen time a v0.2 structured subject needs to play out and be read.
fn structure_time(beat: &Beat) -> f64 {
    let one = |s: &Subject| match s {
        Subject::Collection(c) => {
            let accumulate = if beat.relationship == Some(Relationship::Accumulate) {
                0.4
            } else {
                0.0
            };
            0.5 * c.items.len() as f64 + 1.0 + accumulate
        }
        Subject::StateChange(_) => 1.6,
        Subject::DerivedMetric(_) => 2.0,
        _ => 0.0,
    };
    let primary = one(&beat.primary);
    let secondary = beat.secondary.as_ref().map(one).unwrap_or(0.0);
    // A second structure plays in parallel with the first: it adds less.
    primary.max(secondary) + primary.min(secondary) * 0.4
}

/// `(units, secondary units)` a beat must show: statement words + subject
/// units, and how many of them arrive after the primary has been read.
fn density(beat: &Beat) -> (usize, usize) {
    let one = |s: &Subject| match s {
        Subject::Collection(c) => (c.items.len(), c.items.len().saturating_sub(1)),
        Subject::StateChange(_) => (2, 1),
        Subject::DerivedMetric(_) => (3, 2),
        other => (other.display_text().map(words).unwrap_or(1).min(3), 0),
    };
    let (pu, ps) = one(&beat.primary);
    let (su, ss) = beat
        .secondary
        .as_ref()
        .map(|s| {
            let (u, s2) = one(s);
            (u, s2 + 1)
        })
        .unwrap_or((0, 0));
    (words(&beat.statement) + pu + su, ps + ss)
}

fn overlap_for(incoming: Energy) -> f64 {
    match incoming {
        Energy::Calm => 0.7,
        Energy::Building => 0.55,
        Energy::Impact => 0.5,
    }
}

/// (0.10) Visual beat spacing without speech: `start[i+1] − start[i]` of the
/// planned beats (no assets needed). `motion-engine voice` keeps sentence
/// starts at least 0.9× this apart so speech-led beats keep room for their
/// visuals. Pure.
pub fn planned_start_spacing(intent: &CreativeIntent, style: &StyleProfile) -> Vec<f64> {
    let plans = plan_timing(intent, style);
    plans.windows(2).map(|w| w[1].start - w[0].start).collect()
}

/// Beat timing for a raw style (resolves the taste first).
pub(crate) fn plan_timing(intent: &CreativeIntent, style: &StyleProfile) -> Vec<BeatPlan> {
    plan_timing_with(intent, &resolve_taste(intent, style, None))
}

pub(crate) fn plan_timing_with(
    intent: &CreativeIntent,
    taste: &taste::ResolvedStyleProfile,
) -> Vec<BeatPlan> {
    plan_timing_music(intent, taste, None)
}

/// Beat timing, optionally choreographed to a music grid (0.8): with
/// `downbeats`, handoffs snap to downbeats by retiming the previous beat
/// (`choreography::snap_handoffs`) BEFORE lifecycle planning. `None` is
/// exactly the unsnapped plan.
pub(crate) fn plan_timing_music(
    intent: &CreativeIntent,
    taste: &taste::ResolvedStyleProfile,
    downbeats: Option<&[f64]>,
) -> Vec<BeatPlan> {
    let (overlaps_out, mut durations, _pace) = planned_timing(intent, taste);
    if let Some(downbeats) = downbeats {
        durations = choreography::snap_handoffs(
            &durations,
            &overlaps_out,
            downbeats,
            crate::audio::snap_tolerance(taste.rhythm),
        );
    }
    build_plans(intent, taste, &overlaps_out, &durations, false)
}

/// (0.10) Beat timing led by a voice-over: beat starts follow the spoken
/// sentences (`speech_plan::speech_durations`); music downbeats are ignored.
pub(crate) fn plan_timing_speech(
    intent: &CreativeIntent,
    taste: &taste::ResolvedStyleProfile,
    speech: &crate::speech::SpeechMap,
) -> Result<Vec<BeatPlan>, CompileError> {
    let (overlaps_out, _planned, pace) = planned_timing(intent, taste);
    let durations = speech_plan::speech_durations(speech, &overlaps_out, pace)?;
    Ok(build_plans(intent, taste, &overlaps_out, &durations, true))
}

/// `(outgoing overlaps, planned durations, rhythm pace)` per beat.
fn planned_timing(
    intent: &CreativeIntent,
    taste: &taste::ResolvedStyleProfile,
) -> (Vec<f64>, Vec<f64>, f64) {
    let overlap_k = temporal::overlap_scale(&taste.transition);
    let pace = temporal::duration_scale(taste.rhythm);
    let n = intent.beats.len();
    let overlaps_out: Vec<f64> = (0..n)
        .map(|i| {
            intent
                .beats
                .get(i + 1)
                .map(|b| round3(overlap_for(b.energy) * overlap_k))
                .unwrap_or(0.0)
        })
        .collect();
    let durations: Vec<f64> = intent
        .beats
        .iter()
        .map(|beat| {
            if pace == 1.0 {
                round3(beat_duration(beat))
            } else {
                round3((beat_duration(beat) * pace).max(3.0))
            }
        })
        .collect();
    (overlaps_out, durations, pace)
}

/// Beat plans (starts, overlaps, language, lifecycle) for fixed durations.
/// `speech`: a voice-over speaks the beats (a long statement is then left to
/// the captions instead of being set as body copy).
fn build_plans(
    intent: &CreativeIntent,
    taste: &taste::ResolvedStyleProfile,
    overlaps_out: &[f64],
    durations: &[f64],
    speech: bool,
) -> Vec<BeatPlan> {
    let style = &taste.effective;
    let overlap_k = temporal::overlap_scale(&taste.transition);
    let n = intent.beats.len();
    let mut plans = Vec::with_capacity(n);
    let mut start = 0.0;
    for (i, beat) in intent.beats.iter().enumerate() {
        let next = intent.beats.get(i + 1);
        let overlap_in = if i == 0 {
            0.0
        } else {
            round3(overlap_for(beat.energy) * overlap_k)
        };
        let overlap_out = overlaps_out[i];
        let duration = durations[i];
        let mut language = language_for(beat, style);
        if style.motion_language == crate::style::MotionLanguage::Auto {
            language = temporal::lean_language(
                language,
                taste.motion.lean,
                beat.energy,
                beat.keyword.is_some(),
            );
        }
        let mut lang = language::profile(language, beat.energy, style.camera_style);
        temporal::apply_temperament(&mut lang, &taste.motion, beat.energy);
        temporal::apply_layers(&mut lang, &taste.layers);
        let (accent_in, wipe_in) = if i == 0 {
            (false, false)
        } else {
            temporal::handoff(&taste.transition, beat.energy)
        };
        let (accent_out, wipe_out) = next
            .map(|b| temporal::handoff(&taste.transition, b.energy))
            .unwrap_or((false, false));
        let mut plan = BeatPlan {
            index: i,
            id: format!("beat_{}", i + 1),
            prefix: format!("b{}", i + 1),
            start: round3(start),
            duration,
            overlap_in,
            overlap_out,
            lang,
            accent_in,
            accent_out,
            wipe_in,
            wipe_out,
            is_last: i + 1 == n,
            seed: mix(style.seed, i as u64 + 1),
            life: Lifecycle {
                enter: 0.0,
                settle: 0.0,
                read: 0.0,
                evolve: 0.0,
                anticipate: 0.0,
                bridge: 0.0,
            },
            image_layout: grammar::ImageLayout::Legacy,
            image_min_px: 0.0,
            display: beat_display(beat, speech),
            enter_override: None,
        };
        let (units, secondary_units) = density(beat);
        plan.life = lifecycle::plan(LifecycleInput {
            duration,
            enter_at: plan.enter_at(),
            overlap_out,
            energy: beat.energy,
            language: lang.language,
            units,
            secondary_units,
            read_bias: temporal::read_bias(taste.rhythm),
        });
        plans.push(plan);
        start += duration - overlap_out;
    }
    plans
}

// ---------------------------------------------------------------------------
// Continuity (shared elements)
// ---------------------------------------------------------------------------

/// A subject carried across beats as a SharedElement.
#[derive(Debug, Clone)]
pub(crate) struct Carry {
    pub element: SharedElement,
    /// Normalized identity used to match the subject in later beats.
    pub key: String,
    /// Base layer size, to compute scale for new slots.
    pub base_w: f32,
    pub base_h: f32,
    /// Still being carried forward (accepts keys from the next beat).
    pub open: bool,
    /// Last beat index that contributed keys.
    pub last_beat: usize,
    /// Current (x, y, scale) at the end of the last contributed key.
    pub state: (f32, f32, f32),
}

/// Short engine-written text for any subject, used when a composition shows a
/// second subject as a caption: atomic → its display text; state change →
/// "entity: from to to"; derived metric → "meaning: <computed value>";
/// collection → its items joined (plus meaning).
pub(crate) fn subject_summary(s: &Subject) -> Option<String> {
    let text = match s {
        Subject::StateChange(c) => format!("{}: {} to {}", c.entity, c.from, c.to),
        Subject::DerivedMetric(m) => {
            let v = derived::format_metric(derived::compute(m), m.format).text;
            match m.meaning.as_deref() {
                Some(meaning) => format!("{meaning}: {v}"),
                None => v,
            }
        }
        Subject::Collection(c) => {
            let items: Vec<&str> = c
                .items
                .iter()
                .filter_map(|i| match i {
                    crate::intent::CollectionItem::Phrase(a)
                    | crate::intent::CollectionItem::Number(a) => {
                        a.value.as_deref().or(a.meaning.as_deref())
                    }
                    crate::intent::CollectionItem::Object(o) => {
                        o.meaning.as_deref().or(o.value.as_deref())
                    }
                })
                .collect();
            items.join(", ")
        }
        atomic => atomic.display_text()?.to_string(),
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!text.is_empty()).then_some(text)
}

pub(crate) fn subject_key(s: &Subject) -> String {
    s.value()
        .or(s.meaning())
        .or(s.asset())
        .unwrap_or("")
        .trim()
        .to_lowercase()
}

pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "subject".into()
    } else {
        out
    }
}

pub(crate) fn wants_carry(beat: &Beat, which: Which) -> bool {
    matches!(
        (beat.continuity, which),
        (Continuity::CarryPrimary, Which::Primary) | (Continuity::CarrySecondary, Which::Secondary)
    ) || (beat.relationship == Some(Relationship::Carry) && which == Which::Primary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Which {
    Primary,
    Secondary,
}

pub(crate) fn track_key(
    scene: &str,
    at: f64,
    role: &str,
    easing: Easing,
    state: KeyState,
) -> TrackKey {
    TrackKey {
        scene: scene.to_string(),
        at: round3(at.max(0.0)),
        role: Some(role.to_string()),
        easing,
        state,
        layout: None,
    }
}

// ---------------------------------------------------------------------------
// Small builders
// ---------------------------------------------------------------------------

pub(crate) fn mix(a: u64, b: u64) -> u64 {
    // splitmix64 finalizer over a combined value.
    let mut z = a ^ b.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub(crate) fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

pub(crate) fn base_layer(
    id: impl Into<String>,
    rect: (f32, f32, f32, f32),
    kind: LayerKind,
    z: i32,
) -> Layer {
    Layer {
        tilt: None,
        z: None,
        id: id.into(),
        x: rect.0,
        y: rect.1,
        width: rect.2,
        height: rect.3,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.0,
        anchor_y: 0.0,
        opacity: 1.0,
        z_index: z,
        visible: true,
        clip: None,
        depth: None,
        layout: None,
        kind,
    }
}

pub(crate) fn texture_layer(
    id: &str,
    rect: (f32, f32, f32, f32),
    spec: TextureSpec,
    z: i32,
) -> Layer {
    base_layer(id, rect, LayerKind::Texture(spec), z)
}

pub(crate) fn rect_layer(id: String, rect: (f32, f32, f32, f32), fill: Color, z: i32) -> Layer {
    base_layer(id, rect, LayerKind::Rectangle { fill, stroke: None }, z)
}

/// Motion helpers. Times are scene-local seconds.
pub(crate) mod mo {
    use super::*;

    fn m(target: &str, start: f64, duration: f64, easing: Easing, op: MotionOp) -> Motion {
        Motion {
            spring: None,
            id: None,
            target: target.to_string(),
            start: round3(start),
            duration: round3(duration),
            easing,
            op,
        }
    }
    pub fn fade(target: &str, start: f64, dur: f64, from: f32, to: f32, easing: Easing) -> Motion {
        m(target, start, dur, easing, MotionOp::Fade { from, to })
    }
    pub fn shift(
        target: &str,
        start: f64,
        dur: f64,
        from: [f32; 2],
        to: [f32; 2],
        easing: Easing,
    ) -> Motion {
        m(target, start, dur, easing, MotionOp::Move { from, to })
    }
    pub fn scale(target: &str, start: f64, dur: f64, from: f32, to: f32, easing: Easing) -> Motion {
        m(
            target,
            start,
            dur,
            easing,
            MotionOp::Scale {
                from,
                to,
                axis: Default::default(),
            },
        )
    }
    pub fn rotate(
        target: &str,
        start: f64,
        dur: f64,
        from: f32,
        to: f32,
        easing: Easing,
    ) -> Motion {
        m(target, start, dur, easing, MotionOp::Rotate { from, to })
    }
    pub fn mask(
        target: &str,
        start: f64,
        dur: f64,
        direction: Direction,
        easing: Easing,
    ) -> Motion {
        m(
            target,
            start,
            dur,
            easing,
            MotionOp::MaskReveal {
                direction,
                mode: RevealMode::Reveal,
            },
        )
    }
    pub fn line_in(
        target: &str,
        start: f64,
        dur: f64,
        direction: Direction,
        easing: Easing,
    ) -> Motion {
        m(
            target,
            start,
            dur,
            easing,
            MotionOp::ClipReveal {
                direction,
                mode: RevealMode::Reveal,
            },
        )
    }
    /// Count a text layer up to the number `text` parses as (see [`parse_count`]).
    pub fn count(target: &str, start: f64, dur: f64, text: &str, easing: Easing) -> Option<Motion> {
        let (to, decimals, grouping, prefix, suffix) = parse_count(text)?;
        Some(m(
            target,
            start,
            dur,
            easing.landing(),
            MotionOp::Count {
                from: 0.0,
                to,
                decimals,
                grouping,
                prefix,
                suffix,
            },
        ))
    }
    pub fn expand(
        target: &str,
        start: f64,
        dur: f64,
        to: (f32, f32, f32, f32),
        easing: Easing,
    ) -> Motion {
        m(
            target,
            start,
            dur,
            easing,
            MotionOp::AccentExpand {
                to: BoxRect {
                    x: to.0,
                    y: to.1,
                    width: to.2,
                    height: to.3,
                },
            },
        )
    }
}

/// The scene camera for a beat's language: a push and/or track across the
/// whole beat. `None` when the language keeps the camera still.
pub(crate) fn beat_camera(ctx: &Ctx, plan: &BeatPlan) -> Option<Camera> {
    let cam = plan.lang.camera;
    let mut motions = Vec::new();
    if (cam.push - 1.0).abs() > 1e-4 {
        motions.push(CameraMotion {
            start: 0.0,
            duration: plan.duration,
            easing: cam.easing,
            velocity: None,
            op: CameraOp::Push {
                from: 1.0,
                to: cam.push,
            },
        });
    }
    if cam.track != [0.0, 0.0] {
        motions.push(CameraMotion {
            start: 0.0,
            duration: plan.duration,
            easing: Easing::InOutCubic,
            velocity: None,
            op: CameraOp::Track {
                from: [0.0, 0.0],
                to: [cam.track[0] * ctx.u, cam.track[1] * ctx.u],
            },
        });
    }
    (!motions.is_empty()).then_some(Camera {
        perspective: None,
        pivot: None,
        motions,
    })
}
