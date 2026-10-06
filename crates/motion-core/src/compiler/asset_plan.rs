//! AssetPlanner (0.4): decides, per beat, what visual support the chosen
//! composition needs — including "no image needed". Rules: docs/ASSET_PIPELINE.md.

use super::catalog::{request_words, CatalogMatch, MatchKind};
use super::grammar::{self, Composition, Depiction, Grammar, SelectInput, Variant};
use super::{plan_timing, AssetLibrary, CompileError};
use crate::assets::{
    AssetPlan, AssetRequest, AssetRole, AssetSource, AssetStyleProfile, Background,
    BeatAssetDecision, NegativeSpace, Presentation, Priority, ASSET_PLAN_VERSION,
};
use crate::intent::{
    Beat, CreativeIntent, Purpose, Subject, SubjectKind, INTENT_VERSION, INTENT_VERSION_V0_1,
};
use crate::scene::AssetKind;
use crate::style::StyleProfile;

/// Words that mark a phrase as depicting a person (whole words, lowercase).
/// This classifies what a subject *is*, never what a story is about.
const HUMAN_WORDS: &[&str] = &[
    "person",
    "people",
    "worker",
    "workers",
    "employee",
    "employees",
    "woman",
    "women",
    "man",
    "men",
    "customer",
    "customers",
    "user",
    "users",
    "parent",
    "parents",
    "child",
    "children",
    "kid",
    "kids",
    "student",
    "students",
    "doctor",
    "doctors",
    "nurse",
    "nurses",
    "driver",
    "drivers",
    "family",
    "families",
    "team",
    "face",
    "portrait",
    "crowd",
    "commuter",
    "commuters",
    "shopper",
    "shoppers",
    "founder",
    "founders",
    "manager",
    "managers",
    "patient",
    "patients",
    "reader",
    "readers",
    "viewer",
    "viewers",
];

/// Plan the assets a piece needs. Deterministic; calls no generator.
pub fn plan_assets(
    intent: &CreativeIntent,
    style: &StyleProfile,
    library: &AssetLibrary,
) -> Result<AssetPlan, CompileError> {
    plan_assets_with_reference(intent, style, None, library)
}

/// (0.7) Plan assets with optional reference principles: the plan's
/// `AssetStyleProfile` then describes the reference-informed design system
/// (material, image treatment), so generated images match it.
pub fn plan_assets_with_reference(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&super::taste::ReferencePrinciples>,
    library: &AssetLibrary,
) -> Result<AssetPlan, CompileError> {
    plan_assets_for_look(intent, style, reference, library, None)
}

/// (0.14) Plan with the genre grammar of the art-direction look in force, so
/// the requests use the same roles the builders will ask for.
pub(crate) fn plan_assets_for_look(
    intent: &CreativeIntent,
    style: &StyleProfile,
    reference: Option<&super::taste::ReferencePrinciples>,
    library: &AssetLibrary,
    look_grammar: Option<super::art_direction::LookGrammar>,
) -> Result<AssetPlan, CompileError> {
    if intent.version != INTENT_VERSION && intent.version != INTENT_VERSION_V0_1 {
        return Err(CompileError::Version(intent.version.clone()));
    }
    intent
        .validate()
        .map_err(|errs| CompileError::Invalid(errs.join("; ")))?;
    if intent.beats.is_empty() {
        return Err(CompileError::NoBeats);
    }

    let resolved = reference.map(|r| super::resolve_taste(intent, style, Some(r)));
    let plans = match &resolved {
        Some(t) => super::plan_timing_with(intent, t),
        None => plan_timing(intent, style),
    };
    let neutral = super::visual::VisualLanguage::neutral();
    let visual = resolved.as_ref().map_or(&neutral, |t| &t.visual);
    let mut beats = Vec::with_capacity(intent.beats.len());
    let mut requests = Vec::new();
    for (plan, beat) in plans.iter().zip(&intent.beats) {
        let comp = grammar::select(SelectInput {
            beat,
            language: plan.lang.language,
            format: intent.format,
            side_by_side: intent.format == crate::intent::Format::Landscape,
            seed: plan.seed,
            has_subject_image: false,
            has_object_image: false,
            visual,
            look_grammar,
        });
        let mut beat_requests = plan_beat(plan.index, beat, comp, library, visual);
        beat_requests.sort_by_key(|r| r.role);

        let (source, reason) = match strongest(&beat_requests) {
            Some(r) => (r.source, r.reason.clone()),
            None if comp.depiction == Depiction::Entities => (
                AssetSource::Procedural,
                "visual language: engine-drawn entity tokens".to_string(),
            ),
            None => no_image_decision(comp.grammar),
        };
        beats.push(BeatAssetDecision {
            beat: plan.index + 1,
            composition: comp.grammar.name().to_string(),
            source,
            reason,
            requests: beat_requests.iter().map(|r| r.id.clone()).collect(),
        });
        requests.extend(beat_requests);
    }

    Ok(AssetPlan {
        version: ASSET_PLAN_VERSION.to_string(),
        style: match reference {
            Some(r) => {
                AssetStyleProfile::from_resolved(&super::resolve_taste(intent, style, Some(r)))
            }
            None => AssetStyleProfile::from_style(style),
        },
        beats,
        requests,
    })
}

/// The requests of one beat (unordered).
fn plan_beat(
    index: usize,
    beat: &Beat,
    comp: Composition,
    library: &AssetLibrary,
    visual: &super::visual::VisualLanguage,
) -> Vec<AssetRequest> {
    let mut out: Vec<AssetRequest> = Vec::new();
    let grammar_name = comp.grammar.name();

    // Rule 2: object subjects (primary, then secondary).
    for subject in [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
    {
        let Subject::Object(obj) = subject else {
            continue;
        };
        let is_primary = std::ptr::eq(subject, &beat.primary);
        let preferred = if comp.grammar == Grammar::EvidenceStack {
            AssetRole::EvidenceImage
        } else if (matches!(comp.grammar, Grammar::HeroObject | Grammar::Cinematic3d)
            || super::recipes::picture_pair(beat))
            && is_primary
        {
            AssetRole::HeroObject
        } else {
            AssetRole::SupportingObject
        };
        // Ids must be unique: a second object falls back to supporting_object.
        let role = [preferred, AssetRole::SupportingObject]
            .into_iter()
            .find(|r| !out.iter().any(|q| q.role == *r));
        let Some(role) = role else {
            continue;
        };
        let (source, library_asset, reason) = match library.find_object(&obj.asset) {
            Some((AssetKind::Svg, _)) => (
                AssetSource::Svg,
                Some(obj.asset.clone()),
                "Object subject: the library has a vector asset for it.",
            ),
            Some(_) => (
                AssetSource::UserAsset,
                Some(obj.asset.clone()),
                "Object subject: the library has a PNG for it.",
            ),
            None => (
                AssetSource::GeneratedImage,
                None,
                "Object subject: not in the library, so an image is requested.",
            ),
        };
        // (0.8) No exact-name asset: prefer an exact-word catalog match.
        let catalog = if source == AssetSource::GeneratedImage {
            let words = request_words(&[
                obj.asset.as_str(),
                obj.value.as_deref().unwrap_or_default(),
                obj.meaning.as_deref().unwrap_or_default(),
            ]);
            library.catalog_match(&words, MatchKind::Object)
        } else {
            None
        };
        let evidence = role == AssetRole::EvidenceImage;
        let text = obj
            .meaning
            .as_deref()
            .or(obj.value.as_deref())
            .map(clean)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| clean(&obj.asset));
        let mut request = AssetRequest {
            id: grammar::plate::request_id(index, role),
            beat: index + 1,
            source,
            role,
            subject: text,
            presentation: if evidence {
                Presentation::FlatArtifact
            } else {
                Presentation::IsolatedCutout
            },
            negative_space: NegativeSpace::None,
            background: if evidence {
                Background::Opaque
            } else {
                Background::Transparent
            },
            priority: Priority::Required,
            composition: grammar_name.to_string(),
            reason: reason.to_string(),
            context: (source == AssetSource::GeneratedImage).then(|| clean(&beat.statement)),
            continuity_key: Some(continuity_key(
                library_asset.as_deref().unwrap_or(&obj.asset),
            )),
            library_asset,
        };
        if let Some(m) = &catalog {
            apply_catalog(&mut request, m);
        }
        out.push(request);
    }

    // (0.7.1) Entities beats: engine-drawn tokens; at most one optional hero
    // image when the reference is image-led and the grammar is HeroObject.
    if comp.depiction == Depiction::Entities {
        if comp.grammar == Grammar::HeroObject
            && visual.wants_images()
            && beat.primary.kind() == SubjectKind::Phrase
        {
            out.push(visual_image_request(
                index,
                beat,
                comp,
                library,
                "visual language: image-led hero (optional; procedural token otherwise)",
            ));
        }
        return out;
    }

    // Rule 3: a human phrase in an emphasize beat. A procedural-preferring
    // visual language (0.7.1) requests no style-driven images (rules 3 and 4).
    let procedural = visual.prefers_procedural();
    let human = if !procedural
        && beat.purpose == Purpose::Emphasize
        && matches!(
            comp.grammar,
            Grammar::EditorialCollage | Grammar::KineticPoster | Grammar::CinematicMultiplane
        ) {
        [Some(&beat.primary), beat.secondary.as_ref()]
            .into_iter()
            .flatten()
            .find(|s| s.kind() == SubjectKind::Phrase && is_human(s))
    } else {
        None
    };
    let image_led_emphasis = !procedural
        && human.is_none()
        && visual.wants_images()
        && beat.purpose == Purpose::Emphasize
        && beat.primary.kind() == SubjectKind::Phrase;
    if let Some(subject) = human {
        let face = phrase_text(subject).to_lowercase();
        let role = if face.contains("face") || face.contains("portrait") {
            AssetRole::Portrait
        } else {
            AssetRole::HeroSubject
        };
        let mut request = AssetRequest {
            id: grammar::plate::request_id(index, role),
            beat: index + 1,
            source: AssetSource::GeneratedImage,
            role,
            subject: human_subject_text(subject),
            presentation: Presentation::IsolatedCutout,
            negative_space: match comp.variant {
                Variant::Mirror => NegativeSpace::Left,
                _ => NegativeSpace::Right,
            },
            background: Background::Transparent,
            priority: Priority::Optional,
            composition: Grammar::TypeImageInterlock.name().to_string(),
            reason: "Human subject in an emphasized beat: a cutout lets type and image interlock."
                .to_string(),
            library_asset: None,
            context: Some(clean(&beat.statement)),
            continuity_key: Some(continuity_key(
                subject.value().or(subject.meaning()).unwrap_or_default(),
            )),
        };
        // (0.8) A figure in a library catalog replaces the generated cutout.
        let words = request_words(&[
            subject.value().unwrap_or_default(),
            subject.meaning().unwrap_or_default(),
        ]);
        if let Some(m) = library.catalog_match(&words, MatchKind::Human) {
            apply_catalog(&mut request, &m);
        }
        out.push(request);
    } else if !procedural && comp.grammar == Grammar::CinematicMultiplane {
        // Rule 4: multiplane without a human subject gets an environment plate.
        let base = beat
            .keyword
            .as_deref()
            .map(clean)
            .filter(|k| !k.is_empty())
            .or_else(|| {
                beat.primary
                    .display_text()
                    .map(clean)
                    .filter(|k| !k.is_empty())
            })
            .unwrap_or_else(|| clean(&beat.statement));
        out.push(AssetRequest {
            id: grammar::plate::request_id(index, AssetRole::Environment),
            beat: index + 1,
            source: AssetSource::GeneratedImage,
            role: AssetRole::Environment,
            subject: format!("{base} \u{2014} environment"),
            presentation: Presentation::BackgroundPlate,
            negative_space: NegativeSpace::None,
            background: Background::Opaque,
            priority: Priority::Optional,
            composition: grammar_name.to_string(),
            reason: "Multiplane beat: an environment plate deepens the background.".to_string(),
            library_asset: None,
            context: Some(clean(&beat.statement)),
            continuity_key: Some(format!("{}_environment", continuity_key(&base))),
        });
    }
    // (0.7.1) Image-led reference: an optional hero image for a single
    // emphasized phrase (the engine's typography is complete without it).
    if image_led_emphasis {
        out.push(visual_image_request(
            index,
            beat,
            comp,
            library,
            "visual language: image-led emphasis (optional)",
        ));
    }
    out
}

/// (0.7.1) Optional isolated cutout of the primary phrase, requested because
/// the reference's visual language is image-led (`hero_subject` for human
/// phrases, else `hero_object`).
fn visual_image_request(
    index: usize,
    beat: &Beat,
    comp: Composition,
    library: &AssetLibrary,
    reason: &str,
) -> AssetRequest {
    let role = if is_human(&beat.primary) {
        AssetRole::HeroSubject
    } else {
        AssetRole::HeroObject
    };
    let mut request = AssetRequest {
        id: grammar::plate::request_id(index, role),
        beat: index + 1,
        source: AssetSource::GeneratedImage,
        role,
        subject: human_subject_text(&beat.primary),
        presentation: Presentation::IsolatedCutout,
        negative_space: NegativeSpace::None,
        background: Background::Transparent,
        priority: Priority::Optional,
        composition: comp.grammar.name().to_string(),
        reason: reason.to_string(),
        library_asset: None,
        context: Some(clean(&beat.statement)),
        continuity_key: Some(continuity_key(
            beat.primary
                .value()
                .or(beat.primary.meaning())
                .unwrap_or_default(),
        )),
    };
    // (0.8) A matching library-catalog asset replaces the generated cutout.
    let words = request_words(&[
        beat.primary.value().unwrap_or_default(),
        beat.primary.meaning().unwrap_or_default(),
    ]);
    let kind = if role == AssetRole::HeroSubject {
        MatchKind::Human
    } else {
        MatchKind::Object
    };
    if let Some(m) = library.catalog_match(&words, kind) {
        apply_catalog(&mut request, &m);
    }
    request
}

/// (0.8) Serve `request` from a library-catalog asset instead of a generator.
fn apply_catalog(request: &mut AssetRequest, m: &CatalogMatch) {
    request.source = AssetSource::UserAsset;
    request.library_asset = Some(format!("{}/{}", m.family, m.id));
    request.context = None;
    request.reason = format!(
        "Library catalog: '{}/{}' matches {}.",
        m.family,
        m.id,
        m.matched.join(", ")
    );
}

/// Decision for a beat with no request (rule 1 and the grammar defaults).
fn no_image_decision(g: Grammar) -> (AssetSource, String) {
    match g {
        Grammar::DataStory => (
            AssetSource::None,
            "Data is typographic/procedural: no image needed.".to_string(),
        ),
        Grammar::SequentialStack | Grammar::SplitContrast | Grammar::SpatialCauseEffect => (
            AssetSource::Procedural,
            "Structure carries itself: the engine draws the cards and panels.".to_string(),
        ),
        Grammar::EditorialCollage => (
            AssetSource::Procedural,
            "Collage is complete with engine-drawn print plates: no image needed.".to_string(),
        ),
        Grammar::SphereGallery => (
            AssetSource::None,
            "The sphere's items come from the library at build time (catalog match, else a text card): no image requested."
                .to_string(),
        ),
        _ => (
            AssetSource::None,
            "Typography carries this beat: no image needed.".to_string(),
        ),
    }
}

/// The request with the strongest source (first wins on ties).
fn strongest(requests: &[AssetRequest]) -> Option<&AssetRequest> {
    let rank = |s: AssetSource| match s {
        AssetSource::None => 0,
        AssetSource::Procedural => 1,
        AssetSource::Svg => 2,
        AssetSource::UserAsset => 3,
        AssetSource::GeneratedImage => 4,
    };
    requests
        .iter()
        .fold(None, |best: Option<&AssetRequest>, r| match best {
            Some(b) if rank(b.source) >= rank(r.source) => Some(b),
            _ => Some(r),
        })
}

/// Identity of a depicted entity (0.5): the first four alphanumeric words of
/// its name, lowercased, joined by `_` (`"Office worker"` → `office_worker`).
/// Same words ⇒ same key ⇒ one generated image serves every scene showing it.
pub(crate) fn continuity_key(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(4)
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        "subject".to_string()
    } else {
        words.join("_")
    }
}

/// Trimmed, whitespace-collapsed text.
fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Value and meaning of a phrase, joined for classification.
fn phrase_text(s: &Subject) -> String {
    [s.value(), s.meaning()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

fn human_subject_text(s: &Subject) -> String {
    let value = s.value().map(clean).filter(|t| !t.is_empty());
    let meaning = s.meaning().map(clean).filter(|t| !t.is_empty());
    match (value, meaning) {
        (Some(v), Some(m)) => format!("{v}, {m}"),
        (Some(t), None) | (None, Some(t)) => t,
        (None, None) => String::new(),
    }
}

/// Whether a phrase depicts a person (closed word list, whole words).
fn is_human(s: &Subject) -> bool {
    phrase_text(s)
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| HUMAN_WORDS.contains(&w))
}
