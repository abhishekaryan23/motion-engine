//! Composition grammars (0.4): reusable composition strategies chosen from a
//! beat's semantic *structure* — never its topic. See docs/COMPOSITION_GRAMMAR.md.
//!
//! A grammar decides the spatial idea of a beat (what dominates, how elements
//! relate in the frame, which planes they occupy); the motion language decides
//! how it moves; the lifecycle decides when each piece arrives. Selection is a
//! pure function of the beat, its resolved language, the canvas format, the
//! delivered assets and the style seed.
//!
//! Every builder has the same signature ([`Builder`]) and shares the beat
//! skeleton in `recipes.rs` (kicker, ghost, headline, subject placement with
//! continuity, overlap-aware exit). Builders live one per file.

mod cinematic3d;

/// (0.17) Depth of the cinematic ghost word (the FX director racks focus from it).
pub(crate) fn cinematic_ghost_z() -> f32 {
    cinematic3d::Z_GHOST
}
mod data_story;
mod diagram;
mod dossier;
mod evidence_stack;
mod hero_object;
mod hero_visual;
mod kinetic_poster;
mod kinetic_slam;
mod layer_stack;
mod multiplane;
pub(crate) mod placement;
pub(crate) mod plate;
mod relation_stage;
mod sphere_gallery;
mod split_contrast;
mod stat_pair;
mod type_image;

pub(crate) use cinematic3d::ARRIVAL_ID;
pub(crate) use type_image::plan_image_layouts;

use super::direction;
use super::recipes::{self, Structure, B};
use super::{parse_count, wants_carry, Carry, CompileError, Ctx, Which};
use crate::intent::{Beat, CollectionItem, Format, Purpose, Relationship, Subject, SubjectKind};
use crate::motion::language::Language;

/// A composition strategy.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Grammar {
    EditorialCollage,
    TypeImageInterlock,
    HeroObject,
    SplitContrast,
    EvidenceStack,
    DataStory,
    SequentialStack,
    SpatialCauseEffect,
    CinematicMultiplane,
    KineticPoster,
    /// (0.14) Street/music/sports: the picture owns 70–80 % of the frame.
    HeroVisual,
    /// (0.14) Hype: fast slam cuts timed to the voice.
    KineticSlam,
    /// (0.14) Documentary: evidence documents, stamps, highlighter.
    DocumentaryDossier,
    /// (0.17) Cinematic: the beat staged in depth for a moving 3D camera.
    Cinematic3d,
    /// (0.18) Cinematic collections: the items ride a turning sphere and come
    /// into focus one by one.
    SphereGallery,
    /// (0.19) Layers: the beat's layer is lit in a persistent column and the
    /// secondary subject is pinned inside it.
    LayerStack,
    /// (0.22) Two pictures compared as equals, each with its label and its
    /// stamped figure (the flat looks' relation stage).
    StatPair,
}

impl Grammar {
    pub fn name(self) -> &'static str {
        match self {
            Grammar::EditorialCollage => "editorial_collage",
            Grammar::TypeImageInterlock => "type_image_interlock",
            Grammar::HeroObject => "hero_object",
            Grammar::SplitContrast => "split_contrast",
            Grammar::EvidenceStack => "evidence_stack",
            Grammar::DataStory => "data_story",
            Grammar::SequentialStack => "sequential_stack",
            Grammar::SpatialCauseEffect => "spatial_cause_effect",
            Grammar::CinematicMultiplane => "cinematic_multiplane",
            Grammar::KineticPoster => "kinetic_poster",
            Grammar::HeroVisual => "hero_visual",
            Grammar::KineticSlam => "kinetic_slam",
            Grammar::DocumentaryDossier => "documentary_dossier",
            Grammar::Cinematic3d => "cinematic_3d",
            Grammar::SphereGallery => "sphere_gallery",
            Grammar::LayerStack => "layer_stack",
            Grammar::StatPair => "stat_pair",
        }
    }
}

/// Controlled layout variation inside a grammar. Chosen deterministically from
/// the canvas format and the beat seed; builders ignore variants they don't use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    /// The grammar's default arrangement.
    Standard,
    /// Asymmetric layout mirrored left↔right.
    Mirror,
    /// Two states side by side (split contrast).
    LeftRight,
    /// Two states stacked (split contrast).
    TopBottom,
    /// Two states facing each other across a central divider (split contrast).
    CenterOpposition,
}

/// Which content shape a DataStory / SequentialStack / SplitContrast builder
/// is expressing (so one grammar can serve atomic and structured beats).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// v0.1 atomic subjects.
    Atomic,
    /// Two countable numbers compared (atomic).
    NumberPair,
    /// A collection of ≥ 3 countable numbers without accumulation: a series.
    NumberSeries,
    /// One of the v0.2 structures, composed by its structured composer.
    Structured(Structure),
}

/// (0.7.1) What the beat's subjects look like on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Depiction {
    /// Subjects are typeset (every pre-0.7.1 composition).
    Typographic,
    /// Phrase subjects become procedural entity tokens on a process track and
    /// the relationship plays as a visual operation (`grammar/diagram.rs`).
    /// Only with a visual reference language, only for atomic phrase beats
    /// built by HeroObject or SpatialCauseEffect.
    Entities,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Composition {
    pub grammar: Grammar,
    pub variant: Variant,
    pub shape: Shape,
    pub depiction: Depiction,
    /// Why this grammar (for asset plans and debugging).
    pub reason: &'static str,
}

/// (0.10 Q) Typographic phrase beats (no delivered image, atomic shape) have
/// these three curated alternates (all three render a phrase as the visual;
/// none drops content). (0.23) [`candidates`] lists them after the semantic
/// choice and `choose` picks among them under a direction seed.
pub(crate) const PHRASE_ROTATION: [Grammar; 3] = [
    Grammar::KineticPoster,
    Grammar::EditorialCollage,
    Grammar::CinematicMultiplane,
];

/// (0.10 Q) Subject-first layouts for TypeImageInterlock beats. Under
/// `variety`, image beats cycle through `IMAGE_LAYOUT_ROTATION` from
/// `variety_seed % 4`, skipping a layout that equals the previous image
/// beat's, so consecutive image beats never look the same. (0.23) Under a
/// direction seed each image beat starts at its entry of
/// `direction::seeded_sequence(seed, Dim::ImageLayout, ..)` instead (the same
/// fit checks, the same skip). Without variety the layout is `Legacy`
/// (today's interlock). Every layout keeps type and subject in disjoint
/// regions (taste_rules: no text over subject; subject ≥ SUBJECT_MIN_AREA).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImageLayout {
    /// 0.9 interlock (only without variety).
    Legacy,
    /// Title block at the top, subject large below it.
    TextTopSubjectBelow,
    /// Subject on the right two-thirds, title column left (tall: subject
    /// bottom-right, title top-left).
    SubjectRightTextLeft,
    /// Mirror of `SubjectRightTextLeft`.
    SubjectLeftTextRight,
    /// Subject centred and large, title in a band below (above captions).
    SubjectCenterTextBand,
}

impl ImageLayout {
    /// The option name recorded in `DirectionRecord`.
    pub(crate) fn name(self) -> &'static str {
        match self {
            ImageLayout::Legacy => "legacy",
            ImageLayout::TextTopSubjectBelow => "text_top_subject_below",
            ImageLayout::SubjectRightTextLeft => "subject_right_text_left",
            ImageLayout::SubjectLeftTextRight => "subject_left_text_right",
            ImageLayout::SubjectCenterTextBand => "subject_center_text_band",
        }
    }
}

pub(crate) const IMAGE_LAYOUT_ROTATION: [ImageLayout; 4] = [
    ImageLayout::TextTopSubjectBelow,
    ImageLayout::SubjectRightTextLeft,
    ImageLayout::SubjectCenterTextBand,
    ImageLayout::SubjectLeftTextRight,
];

/// What `select` needs to know beyond the beat itself.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SelectInput<'a> {
    pub beat: &'a Beat,
    pub language: Language,
    pub format: Format,
    /// Type column beside the subject (`placement::side_by_side`): landscape
    /// class, and 6:5 / 4:3 canvases of the square class. Decides the
    /// SplitContrast variant.
    pub side_by_side: bool,
    pub seed: u64,
    /// A delivered image (manifest) for this beat's hero subject/portrait.
    pub has_subject_image: bool,
    /// (0.7.1) A delivered `hero_object` image for this beat.
    pub has_object_image: bool,
    /// (0.7.1) The resolved visual language (neutral without a reference).
    pub visual: &'a super::visual::VisualLanguage,
    /// (0.14) Genre grammar of the art-direction look in force, if any.
    pub look_grammar: Option<super::art_direction::LookGrammar>,
}

pub(crate) type Builder =
    fn(&mut Ctx, &mut B, &mut Vec<Carry>, Composition) -> Result<(), CompileError>;

fn countable(s: &Subject) -> bool {
    s.kind() == SubjectKind::Number && s.value().and_then(parse_count).is_some()
}

/// A collection of ≥ 3 items that are all numeric: number items and (0.22)
/// pictures (objects) whose `value` is countable, in any mix.
fn number_series(s: &Subject) -> bool {
    match s {
        Subject::Collection(c) => {
            c.items.len() >= 3
                && c.items.iter().all(|i| {
                    let value = match i {
                        CollectionItem::Number(a) => a.value.as_deref(),
                        CollectionItem::Object(o) => o.value.as_deref(),
                        CollectionItem::Phrase(_) => None,
                    };
                    value.and_then(parse_count).is_some()
                })
        }
        _ => false,
    }
}

/// (0.23) A template a beat may use, with its semantic fit (3 = the semantic
/// choice, 2 = a curated alternate of the same shape). Every candidate must
/// show every value, name and item the semantic choice shows (the
/// `value_dropped` warning enforces it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub composition: Composition,
    pub fit: u8,
}

/// (0.23) The beat's template candidates, the semantic choice first (fit 3),
/// then the curated alternates of the tie table (fit 2), each passed through
/// `genre_bias` and `variant_for` like the semantic one and deduplicated by
/// grammar (a genre look maps every alternate onto its own grammar, so its
/// beats keep one candidate). Compiles without a direction seed use only the
/// first ([`select`]), so they are unchanged; the direction layer picks among
/// the rest ([`choose`]).
///
/// **Every alternate shows what the semantic choice shows.** A row of the tie
/// table applies only to beats whose subjects are all text or numbers a
/// builder of the row sets as text (phrases for the phrase and explain rows,
/// two countable numbers for the number-pair row), never a picture, a
/// collection or a structured subject (those keep their one grammar); the
/// builders of a row all set the statement, the primary, the secondary and the
/// meaning of such a beat in the scene (a beat that carries a subject on into
/// the next keeps its one grammar: the carried subject is a shared element,
/// not text of the beat), and `tests/takes.rs` plus the 0.23 `value_dropped`
/// warning keep that true over every take. A reference's visual language
/// (`visual_bias` moved or re-labelled the choice) decides construction: its
/// choice stands alone.
pub(crate) fn candidates(input: SelectInput) -> Vec<Candidate> {
    let rule = rule_choice(&input);
    let biased = visual_bias(&input, rule);
    let mut out = vec![Candidate {
        composition: finish(&input, biased),
        fit: 3,
    }];
    if biased != rule {
        return out;
    }
    for (grammar, shape, reason) in tie_table(&input, &rule) {
        let alternate = finish(
            &input,
            Composition {
                grammar,
                variant: Variant::Standard,
                shape,
                depiction: Depiction::Typographic,
                reason,
            },
        );
        if !out
            .iter()
            .any(|c| c.composition.grammar == alternate.grammar)
        {
            out.push(Candidate {
                composition: alternate,
                fit: 2,
            });
        }
    }
    out
}

/// (0.23) The beat carries a subject on into the next beat (a shared element,
/// not text of this beat's scene): its builders place it their own way, so such
/// a beat keeps its one grammar.
fn carries_on(beat: &Beat) -> bool {
    wants_carry(beat, Which::Primary) || wants_carry(beat, Which::Secondary)
}

/// (0.23) The curated alternates (grammar, shape, reason) of the rule table's
/// choice for a beat, in table order, the rule's own grammar left out.
fn tie_table(input: &SelectInput, rule: &Composition) -> Vec<(Grammar, Shape, &'static str)> {
    let beat = input.beat;
    let secondary = beat.secondary.as_ref();
    let phrase = |s: &Subject| s.kind() == SubjectKind::Phrase;
    match (rule.grammar, rule.shape) {
        // An atomic phrase that is emphasized or revealed: typography is the
        // visual in all three.
        (g, Shape::Atomic)
            if PHRASE_ROTATION.contains(&g)
                && matches!(beat.purpose, Purpose::Emphasize | Purpose::Reveal)
                && !input.has_subject_image
                && phrase(&beat.primary)
                && secondary.is_none_or(phrase)
                && !carries_on(beat) =>
        {
            PHRASE_ROTATION
                .iter()
                .filter(|alt| **alt != g)
                .map(|&alt| {
                    let reason = match alt {
                        Grammar::KineticPoster => {
                            "alternate for a phrase: the typography is the poster"
                        }
                        Grammar::EditorialCollage => {
                            "alternate for a phrase: an editorial collage around it"
                        }
                        _ => "alternate for a phrase: layered depth with a moving camera",
                    };
                    (alt, Shape::Atomic, reason)
                })
                .collect()
        }
        // Two countable numbers compared: two counters, or the two states
        // facing each other (both set both figures).
        (Grammar::DataStory, Shape::NumberPair) => vec![(
            Grammar::SplitContrast,
            Shape::Atomic,
            "alternate for two numbers: they share the frame as two states",
        )],
        // An explanation with a second phrase: ordered points, or one side
        // acting on the other.
        (Grammar::SequentialStack, Shape::Atomic)
            if beat.purpose == Purpose::Explain
                && phrase(&beat.primary)
                && secondary.is_some_and(phrase)
                && !carries_on(beat) =>
        {
            vec![(
                Grammar::SpatialCauseEffect,
                Shape::Atomic,
                "alternate for an explanation: one side acts on the other",
            )]
        }
        _ => Vec::new(),
    }
}

/// (0.23) The composition beat `beat` is built with, from its
/// [`candidates`]. Without a direction seed (`Ctx.direction_seed`) this is the
/// first candidate, i.e. [`select`] (byte-identical). Under a seed it is
/// `choice(seed, beat, Dim::Template) % candidates.len()`, moved to the next
/// candidate when that is the previous beat's grammar (a beat never repeats
/// its neighbour's template while it has another). Either way the choice is
/// recorded in `Ctx.direction_beats` (written to `ProjectMeta.direction` only
/// under a seed). A beat is composed again when it is laid out on a shorter
/// canvas; the choice depends only on the beat and the previous beat's final
/// record, so every attempt agrees.
pub(crate) fn choose(ctx: &mut Ctx, beat: usize, candidates: Vec<Candidate>) -> Composition {
    let count = candidates.len();
    let mut index = 0;
    if let Some(seed) = ctx.direction_seed.filter(|_| count > 1) {
        index = (direction::choice(seed, beat, direction::Dim::Template) % count as u64) as usize;
        // (0.23 W2d) Another take rotates away from take 0's template (the
        // candidate's draw of take 0, moved `take` steps, never back to it),
        // so a beat with an alternate shows a different layout per take.
        if let Some((variety, take, k)) = ctx.direction_take.filter(|t| t.1 > 0) {
            let base_seed = direction::candidate_seed(direction::take_seed(variety, 0), k);
            let base = (direction::choice(base_seed, beat, direction::Dim::Template) % count as u64)
                as usize;
            let mut shift = (take % count as u64) as usize;
            if shift == 0 {
                shift = 1;
            }
            index = (base + shift) % count;
        }
        let previous = beat
            .checked_sub(1)
            .and_then(|p| ctx.direction_beats.get(&p))
            .map(|d| d.template.as_str());
        if previous == candidates.get(index).map(|c| c.composition.grammar.name()) {
            index = (index + 1) % count;
        }
    }
    let Some(chosen) = candidates.get(index).map(|c| c.composition) else {
        return unreachable_composition();
    };
    let entry = direction_entry(ctx, beat);
    entry.template = chosen.grammar.name().to_string();
    entry.alternate = u8::try_from(index).unwrap_or(u8::MAX);
    entry.alternates = u8::try_from(count).unwrap_or(u8::MAX);
    entry.reason = chosen.reason.to_string();
    chosen
}

/// (0.23) The direction record of `beat` in `Ctx.direction_beats`, created
/// blank (the beat's template is filled in by [`choose`]).
fn direction_entry<'c>(ctx: &'c mut Ctx, beat: usize) -> &'c mut direction::BeatDirection {
    ctx.direction_beats
        .entry(beat)
        .or_insert_with(|| direction::BeatDirection {
            beat,
            template: String::new(),
            alternate: 0,
            alternates: 1,
            params: direction::BeatParams::default(),
            rotations: Default::default(),
            role: String::new(),
            reason: String::new(),
        })
}

/// (0.23) Record that a seeded rotation (`arrival`, `camera`, `slam_layout`,
/// `image_layout`) replaced a position rule for `beat` and chose `option`.
/// Only under a direction seed (without one no rotation applied).
pub(crate) fn note_rotation(ctx: &mut Ctx, beat: usize, dimension: &str, option: &str) {
    if ctx.direction_seed.is_some() {
        direction_entry(ctx, beat)
            .rotations
            .insert(dimension.to_string(), option.to_string());
    }
}

/// A neutral composition for the impossible empty candidate list (candidates
/// always holds the semantic choice).
fn unreachable_composition() -> Composition {
    Composition {
        grammar: Grammar::EditorialCollage,
        variant: Variant::Standard,
        shape: Shape::Atomic,
        depiction: Depiction::Typographic,
        reason: "no candidate",
    }
}

/// Pick the composition for a beat: the first candidate.
pub(crate) fn select(input: SelectInput) -> Composition {
    candidates(input)
        .first()
        .map(|c| c.composition)
        .unwrap_or_else(|| semantic(input))
}

/// The semantic composition for a beat: the rule table's choice, the visual
/// language's bias, the genre look's takeover and the variant.
fn semantic(input: SelectInput) -> Composition {
    finish(&input, visual_bias(&input, rule_choice(&input)))
}

/// The look's takeover of an atomic beat and the layout variant, the last two
/// steps of the semantic choice (shared by the tie table's alternates).
fn finish(input: &SelectInput, chosen: Composition) -> Composition {
    let chosen = genre_bias(input, chosen);
    Composition {
        variant: variant_for(chosen.grammar, input.format, input.side_by_side, input.seed),
        ..chosen
    }
}

/// The rule table's choice for a beat. First matching rule wins; the order is
/// the design (most specific structure first).
fn rule_choice(input: &SelectInput) -> Composition {
    let beat = input.beat;
    let secondary = beat.secondary.as_ref();
    let any = |f: &dyn Fn(&Subject) -> bool| f(&beat.primary) || secondary.is_some_and(f);
    let is_object = |s: &Subject| s.kind() == SubjectKind::Object;
    let comp = |grammar, shape, reason| Composition {
        grammar,
        variant: Variant::Standard,
        shape,
        depiction: Depiction::Typographic,
        reason,
    };

    match recipes::structure_of(beat) {
        Some(Structure::Collection)
            if beat.relationship != Some(Relationship::Accumulate) && any(&number_series) =>
        {
            comp(
                Grammar::DataStory,
                Shape::NumberSeries,
                "numeric series: bars grow, then the trend line draws",
            )
        }
        Some(Structure::Collection) => comp(
            Grammar::SequentialStack,
            Shape::Structured(Structure::Collection),
            "collection: items arrive one by one, then the aggregate",
        ),
        Some(Structure::StateChange) => comp(
            Grammar::SplitContrast,
            Shape::Structured(Structure::StateChange),
            "state change: from and to share the frame",
        ),
        Some(Structure::DerivedMetric) => comp(
            Grammar::DataStory,
            Shape::Structured(Structure::DerivedMetric),
            "derived metric: numerator, denominator, result, comparison",
        ),
        Some(Structure::Layers) => comp(
            Grammar::LayerStack,
            Shape::Structured(Structure::Layers),
            "layers: one persistent labelled column, the beat's layer lit",
        ),
        None => match beat.purpose {
            Purpose::Reveal | Purpose::Explain if any(&is_object) => comp(
                Grammar::EvidenceStack,
                Shape::Atomic,
                "object revealed/explained as evidence",
            ),
            // (0.22) Two pictures compared: equals, each with its figure
            // (before HeroObject, which keeps one picture and one value).
            Purpose::Contrast | Purpose::Compare
                if is_object(&beat.primary) && secondary.is_some_and(is_object) =>
            {
                comp(
                    Grammar::StatPair,
                    Shape::Atomic,
                    "two pictures compared: each with its label and figure",
                )
            }
            _ if is_object(&beat.primary) => comp(
                Grammar::HeroObject,
                Shape::Atomic,
                "object subject dominates and persists",
            ),
            Purpose::Contrast | Purpose::Compare
                if countable(&beat.primary) && secondary.is_some_and(countable) =>
            {
                comp(
                    Grammar::DataStory,
                    Shape::NumberPair,
                    "two countable numbers compared",
                )
            }
            Purpose::Contrast | Purpose::Compare
                if matches!(
                    beat.relationship,
                    Some(Relationship::Compress | Relationship::Grow | Relationship::Accumulate)
                ) || (beat.relationship.is_none() && beat.purpose == Purpose::Contrast) =>
            {
                comp(
                    Grammar::SpatialCauseEffect,
                    Shape::Atomic,
                    "one side physically acts on the other",
                )
            }
            Purpose::Contrast | Purpose::Compare => comp(
                Grammar::SplitContrast,
                Shape::Atomic,
                "two states share the frame",
            ),
            Purpose::Explain => comp(
                Grammar::SequentialStack,
                Shape::Atomic,
                "explanation unfolds as ordered points",
            ),
            Purpose::Reveal if countable(&beat.primary) => {
                comp(Grammar::DataStory, Shape::Atomic, "a number is revealed")
            }
            Purpose::Reveal => comp(
                Grammar::KineticPoster,
                Shape::Atomic,
                "a phrase is revealed typographically",
            ),
            Purpose::Emphasize if input.has_subject_image => comp(
                Grammar::TypeImageInterlock,
                Shape::Atomic,
                "a delivered subject image interlocks with the headline",
            ),
            Purpose::Emphasize if input.language == Language::Kinetic => comp(
                Grammar::KineticPoster,
                Shape::Atomic,
                "strong phrase with a keyword: typography is the visual",
            ),
            Purpose::Emphasize if input.language == Language::Parallax => comp(
                Grammar::CinematicMultiplane,
                Shape::Atomic,
                "layered depth with a moving camera",
            ),
            Purpose::Emphasize => comp(
                Grammar::EditorialCollage,
                Shape::Atomic,
                "emphasis as an editorial collage",
            ),
        },
    }
}

/// (0.18) A collection with enough items to sit on a sphere.
fn sphere_collection(s: &Subject) -> bool {
    matches!(s, Subject::Collection(c) if c.items.len() >= 3)
}

/// (0.14) A genre look takes over atomic beats; structured shapes (series,
/// collections, state changes, derived metrics) keep their semantic grammar.
/// HeroVisual needs a picture (delivered image or an object subject),
/// otherwise the beat stays typographic (KineticPoster).
fn genre_bias(input: &SelectInput, chosen: Composition) -> Composition {
    use super::art_direction::LookGrammar;
    let Some(g) = input.look_grammar else {
        return chosen;
    };
    // (0.18) A cinematic collection of 3+ items is a turning sphere. Two items
    // keep their semantic grammar, and (0.22) so does a numeric series: its
    // values are compared as bars (a sphere shows one item at a time).
    if g == LookGrammar::Cinematic3d
        && sphere_collection(&input.beat.primary)
        && chosen.shape != Shape::NumberSeries
    {
        return Composition {
            grammar: Grammar::SphereGallery,
            variant: Variant::Standard,
            shape: chosen.shape,
            depiction: Depiction::Typographic,
            reason: "cinematic look: the collection turns on a sphere, one item in focus at a time",
        };
    }
    if !matches!(chosen.shape, Shape::Atomic | Shape::NumberPair) {
        return chosen;
    }
    // (0.22) A pair of pictures stays a stat pair in the street, studio and
    // hype looks (their builders keep one picture); the documentary and
    // cinematic looks stage pairs themselves.
    if chosen.grammar == Grammar::StatPair
        && matches!(g, LookGrammar::HeroVisual | LookGrammar::KineticSlam)
    {
        return chosen;
    }
    let beat = input.beat;
    let has_picture = input.has_subject_image
        || input.has_object_image
        || beat.primary.kind() == SubjectKind::Object
        || beat
            .secondary
            .as_ref()
            .is_some_and(|s| s.kind() == SubjectKind::Object);
    // (0.23) The street and studio look show one picture and one punchword: the
    // figure of the primary phrase / number, else of the pictured object. A
    // beat with another figure to show (the `value_dropped` warning found
    // them) keeps a composition that shows both: two pictures that each carry
    // a figure are a stat pair, any other pair of figures keeps its semantic
    // grammar.
    if g == LookGrammar::HeroVisual && has_picture && punchword_drops_a_figure(beat) {
        if two_valued_pictures(beat) {
            return Composition {
                grammar: Grammar::StatPair,
                variant: Variant::Standard,
                shape: Shape::Atomic,
                depiction: Depiction::Typographic,
                reason: "street / studio look: two pictures with figures are a stat pair",
            };
        }
        return chosen;
    }
    let (grammar, reason) = match g {
        LookGrammar::HeroVisual if has_picture => (
            Grammar::HeroVisual,
            "street look: the picture owns the frame",
        ),
        LookGrammar::HeroVisual => (
            Grammar::KineticPoster,
            "street look without a picture: stencil typography",
        ),
        LookGrammar::KineticSlam => (Grammar::KineticSlam, "hype look: slam cuts on the voice"),
        LookGrammar::DocumentaryDossier => (
            Grammar::DocumentaryDossier,
            "documentary look: evidence on the desk",
        ),
        LookGrammar::Cinematic3d => (
            Grammar::Cinematic3d,
            "cinematic look: the beat staged in depth for the camera",
        ),
    };
    Composition {
        grammar,
        variant: Variant::Standard,
        shape: chosen.shape,
        depiction: Depiction::Typographic,
        reason,
    }
}

/// (0.23) The beat's figures that the street / studio punchword cannot all
/// carry: it shouts one, the primary phrase or number's value, else the
/// pictured object's (`hero_visual::punchword`). True when the beat has a
/// figure to show (a number's or object's value, a compared phrase) that is
/// not that one.
fn punchword_drops_a_figure(beat: &Beat) -> bool {
    let figure = |s: &Subject| {
        s.value()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_lowercase)
    };
    let comparing = matches!(beat.purpose, Purpose::Compare | Purpose::Contrast);
    let required = |s: &Subject| match s.kind() {
        SubjectKind::Number | SubjectKind::Object => figure(s),
        SubjectKind::Phrase if comparing => figure(s),
        _ => None,
    };
    let primary = &beat.primary;
    let pictured = if primary.kind() == SubjectKind::Object {
        Some(primary)
    } else {
        beat.secondary
            .as_ref()
            .filter(|s| s.kind() == SubjectKind::Object)
    };
    let punch = (primary.kind() != SubjectKind::Object)
        .then(|| figure(primary))
        .flatten()
        .or_else(|| pictured.and_then(figure));
    std::iter::once(primary)
        .chain(beat.secondary.as_ref())
        .filter_map(required)
        .any(|f| Some(&f) != punch.as_ref())
}

/// (0.23) Both sides are pictures that carry a figure.
fn two_valued_pictures(beat: &Beat) -> bool {
    let valued = |s: &Subject| {
        s.kind() == SubjectKind::Object && s.value().is_some_and(|v| !v.trim().is_empty())
    };
    valued(&beat.primary) && beat.secondary.as_ref().is_some_and(valued)
}

/// (0.7.1) Constrained visual-language bias over the semantic choice, per
/// docs/VISUAL_LANGUAGE.md §3. Meaning first: structured shapes, numbers,
/// objects and TypeImageInterlock keep the semantic grammar; only atomic
/// phrase beats may be constructed differently. Identity under a neutral
/// visual language. Deterministic: candidates are scored in a fixed order and
/// ties keep the semantic choice, then the table order.
fn visual_bias(input: &SelectInput, semantic: Composition) -> Composition {
    use super::visual::{Explanation, Weight};
    let visual = input.visual;
    let weight = visual.weight();
    let beat = input.beat;
    let secondary = beat.secondary.as_ref();
    let kind_blocks = |s: &Subject| matches!(s.kind(), SubjectKind::Number | SubjectKind::Object);
    if weight == Weight::Neutral
        || semantic.shape != Shape::Atomic
        || kind_blocks(&beat.primary)
        || secondary.is_some_and(kind_blocks)
        || semantic.grammar == Grammar::TypeImageInterlock
    {
        return semantic;
    }

    let s = semantic.grammar;
    // (grammar, semantic fit); the semantic choice is first with fit 3.
    let mut candidates: Vec<(Grammar, i32)> = vec![(s, 3)];
    let mut add = |g: Grammar, fit: i32| {
        if !candidates.iter().any(|(c, _)| *c == g) {
            candidates.push((g, fit));
        }
    };
    match beat.purpose {
        Purpose::Emphasize => {
            add(Grammar::EditorialCollage, 2);
            add(Grammar::KineticPoster, 2);
            add(Grammar::CinematicMultiplane, 2);
            add(Grammar::HeroObject, 2);
        }
        Purpose::Reveal => {
            add(Grammar::HeroObject, 2);
            add(Grammar::CinematicMultiplane, 1);
        }
        Purpose::Contrast | Purpose::Compare => {
            add(Grammar::SpatialCauseEffect, 2);
            add(Grammar::SplitContrast, 2);
        }
        Purpose::Explain => {
            if secondary.is_some() {
                add(Grammar::SpatialCauseEffect, 2);
            }
        }
    }

    let entity_capable =
        |g: Grammar| matches!(g, Grammar::HeroObject | Grammar::SpatialCauseEffect);
    let entity_bonus = secondary.is_some() || visual.prefers_procedural() || input.has_object_image;
    let score = |g: Grammar, fit: i32| -> i32 {
        let mut v = fit + visual.preference(g);
        match weight {
            Weight::Visual => {
                if entity_capable(g) && entity_bonus {
                    v += 2;
                }
                if g == Grammar::CinematicMultiplane {
                    v += 1;
                }
            }
            Weight::Type => {
                if matches!(
                    g,
                    Grammar::KineticPoster | Grammar::EditorialCollage | Grammar::SplitContrast
                ) {
                    v += 1;
                }
            }
            Weight::Neutral => {}
        }
        match (visual.explanation, g) {
            (
                Some(Explanation::Diagrammatic),
                Grammar::SpatialCauseEffect | Grammar::SequentialStack,
            ) => v += 1,
            (Some(Explanation::Literal), Grammar::HeroObject) => v += 1,
            (
                Some(Explanation::Symbolic | Explanation::Metaphorical),
                Grammar::CinematicMultiplane | Grammar::EditorialCollage,
            ) => v += 1,
            _ => {}
        }
        v
    };

    let mut winner = candidates[0].0;
    let mut best = score(candidates[0].0, candidates[0].1);
    for &(g, fit) in &candidates[1..] {
        let v = score(g, fit);
        if v > best {
            best = v;
            winner = g;
        }
    }

    let entities = weight == Weight::Visual
        && entity_capable(winner)
        && beat.primary.kind() == SubjectKind::Phrase;
    let reason = if entities {
        "visual language: entities show the relationship"
    } else if winner == s {
        "visual language: semantic choice kept"
    } else {
        "visual language: composition family preferred"
    };
    Composition {
        grammar: winner,
        shape: Shape::Atomic,
        depiction: if entities {
            Depiction::Entities
        } else {
            Depiction::Typographic
        },
        reason,
        ..semantic
    }
}

/// Deterministic variant: the format constrains, the seed chooses.
fn variant_for(grammar: Grammar, format: Format, side_by_side: bool, seed: u64) -> Variant {
    let bit = (seed >> 17) & 1 == 1;
    match grammar {
        // Side-by-side canvases (landscape class, 6:5 / 4:3) split left/right;
        // identical to the format match at the three legacy canvases.
        Grammar::SplitContrast if side_by_side => Variant::LeftRight,
        Grammar::SplitContrast => match format {
            Format::Landscape => Variant::LeftRight,
            Format::Square if bit => Variant::CenterOpposition,
            Format::Square => Variant::LeftRight,
            Format::Vertical if bit => Variant::CenterOpposition,
            Format::Vertical => Variant::TopBottom,
        },
        Grammar::EditorialCollage
        | Grammar::HeroObject
        | Grammar::CinematicMultiplane
        | Grammar::KineticPoster
        | Grammar::TypeImageInterlock
        | Grammar::EvidenceStack
            if bit =>
        {
            Variant::Mirror
        }
        _ => Variant::Standard,
    }
}

/// The builder for a composition.
pub(crate) fn builder(c: Composition) -> Builder {
    if c.depiction == Depiction::Entities {
        return diagram::build;
    }
    match (c.grammar, c.shape) {
        (Grammar::SequentialStack, Shape::Structured(_)) => structured,
        (Grammar::SplitContrast, Shape::Structured(_)) => structured,
        (Grammar::DataStory, Shape::Structured(_)) => structured,
        (Grammar::SequentialStack, _) => recipes::explain,
        (Grammar::SpatialCauseEffect, _) => recipes::contrast,
        (Grammar::EditorialCollage, _) => recipes::emphasize,
        (Grammar::SplitContrast, _) => split_contrast::build,
        (Grammar::DataStory, _) => data_story::build,
        (Grammar::KineticPoster, _) => kinetic_poster::build,
        (Grammar::HeroObject, _) => hero_object::build,
        (Grammar::CinematicMultiplane, _) => multiplane::build,
        (Grammar::EvidenceStack, _) => evidence_stack::build,
        (Grammar::TypeImageInterlock, _) => type_image::build,
        (Grammar::HeroVisual, _) => hero_visual::build,
        (Grammar::KineticSlam, _) => kinetic_slam::build,
        (Grammar::DocumentaryDossier, _) => dossier::build,
        (Grammar::Cinematic3d, _) => cinematic3d::build,
        (Grammar::SphereGallery, _) => sphere_gallery::build,
        (Grammar::LayerStack, _) => layer_stack::build,
        (Grammar::StatPair, _) => stat_pair::build,
    }
}

/// v0.2 structured subjects keep their composers (collection / state change /
/// derived metric) under the shared structured skeleton.
fn structured(
    ctx: &mut Ctx,
    b: &mut B,
    _carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    let Shape::Structured(s) = c.shape else {
        unreachable!("structured builder needs a structured shape")
    };
    let region = recipes::structured_head(ctx, b);
    match s {
        Structure::Collection => super::collection::compose(ctx, b, region),
        Structure::StateChange => super::state_change::compose(ctx, b, region),
        Structure::DerivedMetric => super::derived::compose(ctx, b, region),
        // Layers have their own builder (`layer_stack::build`), never this one.
        Structure::Layers => unreachable!("layers are built by the layer_stack grammar"),
    }
}

#[cfg(test)]
mod visual_bias_tests {
    use super::*;
    use crate::compiler::visual::{Explanation, Medium, Usage, VisualLanguage, Weight};
    use serde_json::{json, Value};

    fn beat(v: Value) -> Beat {
        serde_json::from_value(v).unwrap()
    }
    fn phrase(v: &str) -> Value {
        json!({"kind": "phrase", "value": v})
    }
    fn number(v: &str) -> Value {
        json!({"kind": "number", "value": v})
    }
    fn object() -> Value {
        json!({"kind": "object", "asset": "shopping_basket"})
    }
    fn pair(purpose: &str, rel: Option<&str>) -> Beat {
        let mut b = json!({
            "purpose": purpose, "statement": "x",
            "primary": phrase("old way"), "secondary": phrase("new way"),
        });
        if let Some(r) = rel {
            b["relationship"] = json!(r);
        }
        beat(b)
    }
    fn single(purpose: &str) -> Beat {
        beat(json!({"purpose": purpose, "statement": "x", "primary": phrase("big idea")}))
    }
    fn lang(medium: Medium) -> VisualLanguage {
        VisualLanguage {
            medium,
            ..VisualLanguage::default()
        }
    }
    fn pick(b: &Beat, v: &VisualLanguage, language: Language, image: bool) -> Composition {
        select(SelectInput {
            beat: b,
            language,
            format: Format::Vertical,
            side_by_side: false,
            seed: 7,
            has_subject_image: false,
            has_object_image: image,
            visual: v,
            look_grammar: None,
        })
    }
    fn sel(b: &Beat, v: &VisualLanguage) -> Composition {
        pick(b, v, Language::Minimal, false)
    }
    fn is(c: Composition, g: Grammar, d: Depiction) {
        assert_eq!((c.grammar, c.depiction), (g, d), "{}", c.reason);
        assert_eq!(c.shape, Shape::Atomic);
    }

    fn sweep() -> Vec<Beat> {
        let sc = json!({"kind": "state_change", "entity": "time", "from": "slow", "to": "fast"});
        let dm = json!({"kind": "derived_metric", "meaning": "complaint rate",
            "numerator": {"value": 12, "meaning": "complaints"},
            "denominator": {"value": 400, "meaning": "orders"}});
        let col = json!({"kind": "collection", "items": [
            {"kind": "phrase", "value": "a"}, {"kind": "phrase", "value": "b"}]});
        let nums = json!({"kind": "collection", "items": [
            {"kind": "number", "value": "1"}, {"kind": "number", "value": "2"},
            {"kind": "number", "value": "3"}]});
        let mut out = Vec::new();
        for p in ["emphasize", "reveal", "explain"] {
            out.push(single(p));
            out.push(pair(p, None));
            out.push(beat(
                json!({"purpose": p, "statement": "x", "primary": number("5")}),
            ));
            out.push(beat(
                json!({"purpose": p, "statement": "x", "primary": object()}),
            ));
        }
        for p in ["contrast", "compare"] {
            for r in [
                None,
                Some("grow"),
                Some("compress"),
                Some("separate"),
                Some("replace"),
                Some("carry"),
            ] {
                out.push(pair(p, r));
            }
            out.push(beat(
                json!({"purpose": p, "statement": "x", "primary": number("5"),
                "secondary": number("20"), "relationship": "grow"}),
            ));
            out.push(beat(
                json!({"purpose": p, "statement": "x", "primary": phrase("a"),
                "secondary": object()}),
            ));
            out.push(beat(
                json!({"purpose": p, "statement": "x", "primary": sc.clone()}),
            ));
            out.push(beat(
                json!({"purpose": p, "statement": "x", "primary": dm.clone(),
                "secondary": dm.clone()}),
            ));
        }
        out.push(beat(
            json!({"purpose": "emphasize", "statement": "x", "primary": sc}),
        ));
        out.push(beat(
            json!({"purpose": "reveal", "statement": "x", "primary": dm}),
        ));
        out.push(beat(
            json!({"purpose": "emphasize", "statement": "x", "primary": col,
            "relationship": "accumulate"}),
        ));
        out.push(beat(
            json!({"purpose": "emphasize", "statement": "x", "primary": nums}),
        ));
        out
    }

    fn all_languages() -> Vec<VisualLanguage> {
        let mut v: Vec<VisualLanguage> = [
            Medium::TypeOnly,
            Medium::TypeLed,
            Medium::ImageLed,
            Medium::ObjectLed,
            Medium::Diagrammatic,
            Medium::Collage,
            Medium::InterfaceLed,
        ]
        .into_iter()
        .map(lang)
        .collect();
        v.push(VisualLanguage {
            medium: Medium::Mixed,
            usage: Some(Usage::Dense),
            preferred: vec![Grammar::HeroObject, Grammar::SpatialCauseEffect],
            explanation: Some(Explanation::Diagrammatic),
            ..VisualLanguage::default()
        });
        v.push(VisualLanguage {
            medium: Medium::ImageLed,
            preferred: vec![Grammar::CinematicMultiplane],
            avoid: vec![Grammar::DataStory, Grammar::SplitContrast],
            ..VisualLanguage::default()
        });
        v
    }

    #[test]
    fn neutral_is_identity_and_typographic() {
        let neutral = VisualLanguage::neutral();
        // Weight Neutral with other fields set must also be the identity.
        let inert = VisualLanguage {
            medium: Medium::Mixed,
            preferred: vec![Grammar::HeroObject],
            avoid: vec![Grammar::KineticPoster],
            explanation: Some(Explanation::Diagrammatic),
            ..VisualLanguage::default()
        };
        assert_eq!(inert.weight(), Weight::Neutral);
        for b in sweep() {
            for l in [Language::Minimal, Language::Kinetic, Language::Parallax] {
                let a = pick(&b, &neutral, l, false);
                assert_eq!(a.depiction, Depiction::Typographic);
                assert!(!a.reason.starts_with("visual language"));
                assert_eq!(a, pick(&b, &inert, l, false));
                assert_eq!(a, pick(&b, &inert, l, true));
            }
        }
    }

    #[test]
    fn type_led_keeps_semantic_grammar() {
        for medium in [Medium::TypeOnly, Medium::TypeLed] {
            let v = lang(medium);
            for language in [Language::Minimal, Language::Kinetic, Language::Parallax] {
                for purpose in ["emphasize", "reveal"] {
                    for b in [single(purpose), pair(purpose, None)] {
                        let n = pick(&b, &VisualLanguage::neutral(), language, false);
                        let c = pick(&b, &v, language, false);
                        assert_eq!(c.grammar, n.grammar, "{purpose} {language:?}");
                        assert_eq!(c.depiction, Depiction::Typographic);
                    }
                }
                for rel in [
                    None,
                    Some("replace"),
                    Some("compress"),
                    Some("grow"),
                    Some("separate"),
                ] {
                    for purpose in ["contrast", "compare"] {
                        let b = pair(purpose, rel);
                        let n = pick(&b, &VisualLanguage::neutral(), language, false);
                        let c = pick(&b, &v, language, false);
                        assert_eq!(c.grammar, n.grammar, "{purpose} {rel:?}");
                        assert_eq!(c.depiction, Depiction::Typographic);
                    }
                }
            }
        }
    }

    #[test]
    fn object_led_and_diagrammatic_show_relationships_as_entities() {
        let diag = VisualLanguage {
            medium: Medium::Diagrammatic,
            explanation: Some(Explanation::Diagrammatic),
            ..VisualLanguage::default()
        };
        for v in [lang(Medium::ObjectLed), lang(Medium::Diagrammatic), diag] {
            assert!(v.prefers_procedural());
            let e = Depiction::Entities;
            let sce = Grammar::SpatialCauseEffect;
            is(sel(&pair("contrast", Some("replace")), &v), sce, e);
            is(sel(&pair("contrast", Some("compress")), &v), sce, e);
            is(sel(&pair("emphasize", None), &v), Grammar::HeroObject, e);
            is(sel(&single("reveal"), &v), Grammar::HeroObject, e);
            is(sel(&pair("explain", None), &v), sce, e);
            let kinetic = pick(&pair("emphasize", None), &v, Language::Kinetic, false);
            is(kinetic, Grammar::HeroObject, e);
        }
    }

    #[test]
    fn image_led_needs_a_delivered_image_for_a_single_phrase() {
        let v = lang(Medium::ImageLed);
        assert!(!v.prefers_procedural());
        let b = single("emphasize");
        // No hero image: the single phrase stays a typographic composition.
        let c = pick(&b, &v, Language::Kinetic, false);
        assert_eq!(c.depiction, Depiction::Typographic);
        assert!(!matches!(
            c.grammar,
            Grammar::HeroObject | Grammar::SpatialCauseEffect
        ));
        // A reference that prefers the kinetic poster keeps it.
        let kinetic = VisualLanguage {
            preferred: vec![Grammar::KineticPoster],
            ..lang(Medium::ImageLed)
        };
        is(
            pick(&b, &kinetic, Language::Kinetic, false),
            Grammar::KineticPoster,
            Depiction::Typographic,
        );
        // A delivered hero_object image makes it a hero entity.
        is(
            pick(&b, &v, Language::Kinetic, true),
            Grammar::HeroObject,
            Depiction::Entities,
        );
        // A two-entity relationship is entities regardless.
        is(
            sel(&pair("contrast", Some("replace")), &v),
            Grammar::SpatialCauseEffect,
            Depiction::Entities,
        );
    }

    #[test]
    fn avoid_list_blocks_a_grammar() {
        let v = VisualLanguage {
            avoid: vec![Grammar::SpatialCauseEffect],
            ..lang(Medium::ObjectLed)
        };
        is(
            sel(&pair("contrast", Some("replace")), &v),
            Grammar::SplitContrast,
            Depiction::Typographic,
        );
    }

    #[test]
    fn preferred_list_can_move_a_beat() {
        let v = VisualLanguage {
            preferred: vec![Grammar::CinematicMultiplane],
            ..lang(Medium::ImageLed)
        };
        is(
            sel(&single("emphasize"), &v),
            Grammar::CinematicMultiplane,
            Depiction::Typographic,
        );
    }

    #[test]
    fn data_and_structure_are_never_moved() {
        for v in all_languages() {
            for b in sweep() {
                let neutral = sel(&b, &VisualLanguage::neutral());
                let fixed = neutral.shape != Shape::Atomic
                    || b.primary.kind() != SubjectKind::Phrase
                    || b.secondary
                        .as_ref()
                        .is_some_and(|s| s.kind() == SubjectKind::Object);
                if fixed {
                    assert_eq!(sel(&b, &v), neutral, "{:?} {:?}", v.medium, b.purpose);
                }
            }
        }
        // The complaint-rate derived metric stays a DataStory under every language.
        let dm = beat(json!({"purpose": "reveal", "statement": "x", "primary": {
            "kind": "derived_metric", "meaning": "complaint rate",
            "numerator": {"value": 12, "meaning": "complaints"},
            "denominator": {"value": 400, "meaning": "orders"}}}));
        for v in all_languages() {
            let c = sel(&dm, &v);
            assert_eq!(
                (c.grammar, c.depiction),
                (Grammar::DataStory, Depiction::Typographic)
            );
            assert_eq!(c.shape, Shape::Structured(Structure::DerivedMetric));
        }
    }

    #[test]
    fn entities_only_for_visual_weight_and_phrase_primaries() {
        for v in all_languages() {
            for b in sweep() {
                let c = sel(&b, &v);
                if c.depiction == Depiction::Entities {
                    assert_eq!(v.weight(), Weight::Visual);
                    assert!(matches!(
                        c.grammar,
                        Grammar::HeroObject | Grammar::SpatialCauseEffect
                    ));
                    assert_eq!(b.primary.kind(), SubjectKind::Phrase);
                    assert_eq!(c.shape, Shape::Atomic);
                }
            }
        }
    }

    #[test]
    fn selection_is_deterministic() {
        for v in all_languages() {
            for b in sweep() {
                let a = sel(&b, &v);
                for _ in 0..3 {
                    assert_eq!(a, sel(&b, &v));
                }
            }
        }
    }
}

#[cfg(test)]
mod variant_tests {
    use super::*;
    use crate::compiler::layout_frame::LayoutFrame;

    fn variant(w: u32, h: u32, seed: u64) -> Variant {
        let f = LayoutFrame::new(w, h).unwrap();
        variant_for(
            Grammar::SplitContrast,
            f.class,
            placement::side_by_side(&f),
            seed,
        )
    }

    #[test]
    fn split_contrast_variant_follows_side_by_side() {
        let (zero, one) = (0u64, 1u64 << 17);
        // Legacy canvases keep the format-driven choice.
        assert_eq!(variant(1920, 1080, zero), Variant::LeftRight);
        assert_eq!(variant(1920, 1080, one), Variant::LeftRight);
        assert_eq!(variant(1080, 1080, zero), Variant::LeftRight);
        assert_eq!(variant(1080, 1080, one), Variant::CenterOpposition);
        assert_eq!(variant(1080, 1920, zero), Variant::TopBottom);
        assert_eq!(variant(1080, 1920, one), Variant::CenterOpposition);
        // 6:5 and 4:3 are square class but lay out side by side.
        assert_eq!(variant(1296, 1080, one), Variant::LeftRight);
        assert_eq!(variant(1440, 1080, one), Variant::LeftRight);
        // 4:5 stays stacked.
        assert_eq!(variant(1080, 1350, one), Variant::CenterOpposition);
    }
}

#[cfg(test)]
mod sphere_selection_tests {
    use super::*;
    use crate::compiler::art_direction::LookGrammar;
    use crate::compiler::visual::VisualLanguage;
    use serde_json::{json, Value};

    fn beat(primary: Value) -> Beat {
        serde_json::from_value(json!({
            "purpose": "emphasize", "statement": "x", "primary": primary,
        }))
        .unwrap()
    }

    fn objects(n: usize) -> Beat {
        let names = ["robot", "microchip", "brain", "atom", "gears", "heart"];
        let items: Vec<Value> = names[..n]
            .iter()
            .map(|a| json!({"kind": "object", "asset": a}))
            .collect();
        beat(json!({"kind": "collection", "items": items}))
    }

    fn pick(b: &Beat, look: Option<LookGrammar>) -> Composition {
        select(SelectInput {
            beat: b,
            language: Language::Minimal,
            format: Format::Vertical,
            side_by_side: false,
            seed: 7,
            has_subject_image: false,
            has_object_image: false,
            visual: &VisualLanguage::default(),
            look_grammar: look,
        })
    }

    #[test]
    fn cinematic_collections_of_three_or_more_turn_on_a_sphere() {
        for n in 3..=6 {
            let c = pick(&objects(n), Some(LookGrammar::Cinematic3d));
            assert_eq!(c.grammar, Grammar::SphereGallery, "{n} items");
        }
        assert_eq!(
            pick(&objects(3), Some(LookGrammar::Cinematic3d))
                .grammar
                .name(),
            "sphere_gallery"
        );
        // (0.22) A numeric series stays a chart: numbers, and pictures with
        // countable values.
        let series = beat(json!({"kind": "collection", "items": [
            {"kind": "number", "value": "1"}, {"kind": "number", "value": "2"},
            {"kind": "number", "value": "3"}]}));
        let ranked = beat(json!({"kind": "collection", "items": [
            {"kind": "object", "asset": "robot", "value": "4.1%"},
            {"kind": "object", "asset": "brain", "value": "3.4%"},
            {"kind": "number", "value": "3.1%", "meaning": "atoms"}]}));
        for b in [&series, &ranked] {
            assert_eq!(pick(b, None).shape, Shape::NumberSeries);
            let c = pick(b, Some(LookGrammar::Cinematic3d));
            assert_eq!(
                (c.grammar, c.shape),
                (Grammar::DataStory, Shape::NumberSeries)
            );
        }
        // A value that is not a number keeps the sphere.
        let mixed = beat(json!({"kind": "collection", "items": [
            {"kind": "object", "asset": "robot", "value": "4.1%"},
            {"kind": "object", "asset": "brain", "value": "top pick"},
            {"kind": "object", "asset": "atom", "value": "3.1%"}]}));
        assert_eq!(
            pick(&mixed, Some(LookGrammar::Cinematic3d)).grammar,
            Grammar::SphereGallery
        );
    }

    #[test]
    fn everything_else_keeps_its_grammar() {
        // Two items, another look, no look: the structured collection composer.
        let keep = |b: &Beat, look| pick(b, look).grammar;
        assert_eq!(
            keep(&objects(2), Some(LookGrammar::Cinematic3d)),
            Grammar::SequentialStack
        );
        assert_eq!(
            keep(&objects(4), Some(LookGrammar::HeroVisual)),
            Grammar::SequentialStack
        );
        assert_eq!(keep(&objects(4), None), Grammar::SequentialStack);
        // An atomic object beat stays the cinematic hero.
        let one = beat(json!({"kind": "object", "asset": "robot"}));
        assert_eq!(
            keep(&one, Some(LookGrammar::Cinematic3d)),
            Grammar::Cinematic3d
        );
    }
}

/// (0.23) The curated tie table of `candidates`.
#[cfg(test)]
mod alternates_tests {
    use super::*;
    use crate::compiler::art_direction::LookGrammar;
    use crate::compiler::visual::{Medium, VisualLanguage};
    use serde_json::{json, Value};

    fn beat(v: Value) -> Beat {
        serde_json::from_value(v).unwrap()
    }
    fn phrase(v: &str) -> Value {
        json!({"kind": "phrase", "value": v, "meaning": "m"})
    }
    fn number(v: &str) -> Value {
        json!({"kind": "number", "value": v, "meaning": "m"})
    }
    fn make(purpose: &str, primary: Value, secondary: Option<Value>, extra: Value) -> Beat {
        let mut b = json!({"purpose": purpose, "statement": "x", "primary": primary});
        if let Some(s) = secondary {
            b["secondary"] = s;
        }
        for (k, v) in extra.as_object().into_iter().flatten() {
            b[k] = v.clone();
        }
        beat(b)
    }
    fn names(
        b: &Beat,
        language: Language,
        look: Option<LookGrammar>,
        visual: &VisualLanguage,
        image: bool,
    ) -> Vec<&'static str> {
        candidates(SelectInput {
            beat: b,
            language,
            format: Format::Vertical,
            side_by_side: false,
            seed: 7,
            has_subject_image: image,
            has_object_image: false,
            visual,
            look_grammar: look,
        })
        .iter()
        .map(|c| c.composition.grammar.name())
        .collect()
    }
    fn plain(b: &Beat, language: Language) -> Vec<&'static str> {
        names(b, language, None, &VisualLanguage::neutral(), false)
    }

    #[test]
    fn a_phrase_that_is_emphasized_or_revealed_has_the_three_typographic_templates() {
        let one = make("emphasize", phrase("Rest"), None, json!({}));
        let two = make(
            "emphasize",
            phrase("Rest"),
            Some(phrase("Sleep")),
            json!({}),
        );
        let reveal = make("reveal", phrase("Rest"), Some(phrase("Sleep")), json!({}));
        // The semantic choice first, the others of PHRASE_ROTATION after it.
        for b in [&one, &two] {
            assert_eq!(
                plain(b, Language::Minimal),
                [
                    "editorial_collage",
                    "kinetic_poster",
                    "cinematic_multiplane"
                ]
            );
            assert_eq!(
                plain(b, Language::Kinetic),
                [
                    "kinetic_poster",
                    "editorial_collage",
                    "cinematic_multiplane"
                ]
            );
            assert_eq!(
                plain(b, Language::Parallax),
                [
                    "cinematic_multiplane",
                    "kinetic_poster",
                    "editorial_collage"
                ]
            );
        }
        assert_eq!(
            plain(&reveal, Language::Minimal),
            [
                "kinetic_poster",
                "editorial_collage",
                "cinematic_multiplane"
            ]
        );
        // Without a direction seed the first candidate is `select`.
        let input = SelectInput {
            beat: &one,
            language: Language::Kinetic,
            format: Format::Vertical,
            side_by_side: false,
            seed: 7,
            has_subject_image: false,
            has_object_image: false,
            visual: &VisualLanguage::neutral(),
            look_grammar: None,
        };
        assert_eq!(select(input), candidates(input)[0].composition);
        let fits: Vec<u8> = candidates(input).iter().map(|c| c.fit).collect();
        assert_eq!(fits, [3, 2, 2]);
    }

    #[test]
    fn two_countable_numbers_compare_as_counters_or_as_two_states() {
        let pair = make("compare", number("5"), Some(number("20")), json!({}));
        assert_eq!(
            plain(&pair, Language::Minimal),
            ["data_story", "split_contrast"]
        );
        let contrast = make("contrast", number("5"), Some(number("20")), json!({}));
        assert_eq!(
            plain(&contrast, Language::Minimal),
            ["data_story", "split_contrast"]
        );
        // The alternate is the two-state split, not a second counter layout.
        let all = candidates(SelectInput {
            beat: &pair,
            language: Language::Minimal,
            format: Format::Vertical,
            side_by_side: false,
            seed: 7,
            has_subject_image: false,
            has_object_image: false,
            visual: &VisualLanguage::neutral(),
            look_grammar: None,
        });
        assert_eq!(all[0].composition.shape, Shape::NumberPair);
        assert_eq!(all[1].composition.shape, Shape::Atomic);
        // A number against a phrase keeps the semantic split.
        let mixed = make("compare", number("5"), Some(phrase("a lot")), json!({}));
        assert_eq!(plain(&mixed, Language::Minimal), ["split_contrast"]);
    }

    #[test]
    fn an_explanation_with_a_second_phrase_is_ordered_points_or_one_side_acting() {
        let b = make("explain", phrase("Cue"), Some(phrase("Reward")), json!({}));
        assert_eq!(
            plain(&b, Language::Minimal),
            ["sequential_stack", "spatial_cause_effect"]
        );
        let alone = make("explain", phrase("Cue"), None, json!({}));
        assert_eq!(plain(&alone, Language::Minimal), ["sequential_stack"]);
    }

    #[test]
    fn what_a_template_shows_that_another_would_not_keeps_its_one_grammar() {
        let min = Language::Minimal;
        // A picture, a number or a collection is shown by its own grammar only.
        let object = json!({"kind": "object", "asset": "shopping_basket"});
        let col = json!({"kind": "collection", "items": [
            {"kind": "phrase", "value": "a"}, {"kind": "phrase", "value": "b"}]});
        for b in [
            make("emphasize", object.clone(), None, json!({})),
            make("emphasize", phrase("Rest"), Some(object.clone()), json!({})),
            make("emphasize", number("5"), None, json!({})),
            make("reveal", number("5"), None, json!({})),
            make("emphasize", col.clone(), None, json!({})),
            make("explain", phrase("a"), Some(number("5")), json!({})),
            make("explain", object.clone(), Some(phrase("a")), json!({})),
        ] {
            assert_eq!(plain(&b, min).len(), 1, "{:?}", b.primary);
        }
        // A delivered subject image makes the beat an interlock.
        let phrase_beat = make("emphasize", phrase("Rest"), None, json!({}));
        assert_eq!(
            names(&phrase_beat, min, None, &VisualLanguage::neutral(), true),
            ["type_image_interlock"]
        );
        // A beat that carries a subject on keeps the grammar that places it.
        let carry = make(
            "explain",
            phrase("Cue"),
            Some(phrase("Reward")),
            json!({"relationship": "carry"}),
        );
        assert_eq!(plain(&carry, min).len(), 1);
        let carry_out = make(
            "emphasize",
            phrase("Rest"),
            Some(phrase("Sleep")),
            json!({"continuity": "carry_secondary"}),
        );
        assert_eq!(plain(&carry_out, min).len(), 1);
    }

    #[test]
    fn a_look_or_a_reference_decides_construction_alone() {
        let b = make(
            "emphasize",
            phrase("Rest"),
            Some(phrase("Sleep")),
            json!({}),
        );
        let neutral = VisualLanguage::neutral();
        // A genre look maps every alternate onto its own grammar: one candidate.
        for look in [
            LookGrammar::HeroVisual,
            LookGrammar::KineticSlam,
            LookGrammar::DocumentaryDossier,
            LookGrammar::Cinematic3d,
        ] {
            assert_eq!(
                names(&b, Language::Minimal, Some(look), &neutral, false).len(),
                1
            );
        }
        let pair = make("compare", number("5"), Some(number("20")), json!({}));
        assert_eq!(
            names(
                &pair,
                Language::Minimal,
                Some(LookGrammar::Cinematic3d),
                &neutral,
                false
            ),
            ["cinematic_3d"]
        );
        // A reference's visual language chose the construction: no alternates.
        let visual = VisualLanguage {
            medium: Medium::TypeLed,
            ..VisualLanguage::default()
        };
        assert_eq!(names(&b, Language::Minimal, None, &visual, false).len(), 1);
    }

    #[test]
    fn candidates_are_deduplicated_and_deterministic() {
        for purpose in ["emphasize", "reveal", "explain", "compare", "contrast"] {
            let b = make(purpose, phrase("a"), Some(phrase("b")), json!({}));
            let n = plain(&b, Language::Kinetic);
            let unique: std::collections::BTreeSet<_> = n.iter().collect();
            assert_eq!(n.len(), unique.len(), "{purpose}: {n:?}");
            assert_eq!(n, plain(&b, Language::Kinetic));
        }
    }
}
