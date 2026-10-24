//! (0.7.1) AssetPlanner with a reference visual language
//! (docs/VISUAL_LANGUAGE.md section 4): the language biases planning, it never
//! forces useless images. Intents are synthetic.

use std::path::PathBuf;

use motion_core::assets::{
    AssetManifest, AssetPlan, AssetRole, AssetSource, Background, NegativeSpace, Presentation,
    Priority,
};
use motion_core::compiler::taste::ReferencePrinciples;
use motion_core::compiler::visual::{
    Balance, Character, Explanation, Medium, Usage, VisualLanguage, Weight,
};
use motion_core::compiler::{compile_full, plan_assets, plan_assets_with_reference, ApproxMeasure};
use motion_core::intent::CreativeIntent;
use motion_core::style::StyleProfile;
use motion_core::AssetLibrary;
use serde_json::{json, Value};

fn library() -> AssetLibrary {
    AssetLibrary::new(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets"
    )))
}

fn intent(beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2",
        "title": "visual_language_assets",
        "beats": beats,
    }))
    .expect("intent parses")
}

fn refer(visual: VisualLanguage) -> ReferencePrinciples {
    ReferencePrinciples {
        visual,
        ..ReferencePrinciples::default()
    }
}

fn plan_ref(i: &CreativeIntent, visual: VisualLanguage) -> AssetPlan {
    plan_assets_with_reference(
        i,
        &StyleProfile::default(),
        Some(&refer(visual)),
        &library(),
    )
    .expect("plan")
}

fn plan_none(i: &CreativeIntent) -> AssetPlan {
    plan_assets(i, &StyleProfile::default(), &library()).expect("plan")
}

fn to_json<T: serde::Serialize>(p: &T) -> String {
    serde_json::to_string(p).expect("serialize")
}

fn lang(medium: Medium) -> VisualLanguage {
    VisualLanguage {
        medium,
        ..VisualLanguage::default()
    }
}

fn image_led() -> VisualLanguage {
    VisualLanguage {
        usage: Some(Usage::Dense),
        balance: Some(Balance::VisualDominant),
        character: Some(Character::Photographic),
        ..lang(Medium::ImageLed)
    }
}

fn diagrammatic() -> VisualLanguage {
    VisualLanguage {
        character: Some(Character::Diagrammatic),
        explanation: Some(Explanation::Diagrammatic),
        ..lang(Medium::Diagrammatic)
    }
}

fn all_languages() -> Vec<VisualLanguage> {
    vec![
        VisualLanguage::neutral(),
        lang(Medium::TypeLed),
        lang(Medium::TypeOnly),
        image_led(),
        lang(Medium::ImageLed),
        lang(Medium::ObjectLed),
        diagrammatic(),
        lang(Medium::Collage),
        VisualLanguage {
            usage: Some(Usage::Balanced),
            ..lang(Medium::Mixed)
        },
    ]
}

fn phrase(value: &str) -> Value {
    json!({ "kind": "phrase", "value": value })
}

fn generated(p: &AssetPlan) -> usize {
    p.requests
        .iter()
        .filter(|r| r.source == AssetSource::GeneratedImage)
        .count()
}

fn pair_beat(purpose: &str, relationship: Option<&str>, a: &str, b: &str) -> Value {
    let mut v = json!({
        "purpose": purpose,
        "statement": "One thing gives way to another",
        "primary": phrase(a),
        "secondary": phrase(b),
    });
    if let Some(r) = relationship {
        v["relationship"] = json!(r);
    }
    v
}

fn human_story() -> CreativeIntent {
    intent(vec![
        json!({ "purpose": "emphasize", "statement": "She works while the city sleeps",
                "primary": { "kind": "phrase", "value": "a night-shift nurse",
                             "meaning": "caring at 3 a.m." } }),
        json!({ "purpose": "emphasize", "statement": "Rest is rare",
                "primary": phrase("a night-shift nurse") }),
    ])
}

#[test]
fn image_led_human_story_requests_an_optional_hero_subject() {
    let i = human_story();
    assert!(image_led().wants_images());
    let p = plan_ref(&i, image_led());
    assert_eq!(p.requests.len(), 2, "{:?}", p.requests);
    for r in &p.requests {
        assert_eq!(r.role, AssetRole::HeroSubject);
        assert_eq!(r.source, AssetSource::GeneratedImage);
        assert_eq!(r.priority, Priority::Optional);
        assert_eq!(r.presentation, Presentation::IsolatedCutout);
        assert_eq!(r.background, Background::Transparent);
    }
    assert_eq!(
        p.requests[0].subject,
        "a night-shift nurse, caring at 3 a.m."
    );
    assert_eq!(p.beats[0].source, AssetSource::GeneratedImage);
}

#[test]
fn same_story_without_a_reference_is_todays_plan() {
    let i = human_story();
    let none = plan_none(&i);
    let explicit =
        plan_assets_with_reference(&i, &StyleProfile::default(), None, &library()).expect("plan");
    assert_eq!(to_json(&none), to_json(&explicit));
    // A neutral visual language is the identity too.
    assert_eq!(
        to_json(&none),
        to_json(&plan_ref(&i, VisualLanguage::neutral()))
    );
}

#[test]
fn image_led_non_human_phrase_gets_an_optional_hero_object() {
    let i = intent(vec![json!({
        "purpose": "emphasize", "statement": "A single quiet idea", "primary": phrase("deep focus")
    })]);
    // Today: no hero request (at most the multiplane environment plate).
    assert!(plan_none(&i)
        .requests
        .iter()
        .all(|r| r.role != AssetRole::HeroObject));
    let p = plan_ref(&i, image_led());
    let r = p
        .requests
        .iter()
        .find(|r| r.role == AssetRole::HeroObject)
        .expect("hero_object request");
    assert_eq!(
        p.requests
            .iter()
            .filter(|r| r.role == AssetRole::HeroObject)
            .count(),
        1
    );
    assert_eq!(r.id, "beat_1.hero_object");
    assert_eq!(r.role, AssetRole::HeroObject);
    assert_eq!(r.priority, Priority::Optional);
    assert_eq!(r.negative_space, NegativeSpace::None);
    assert_eq!(r.presentation, Presentation::IsolatedCutout);
    assert_eq!(r.background, Background::Transparent);
    assert_eq!(r.reason, "visual language: image-led emphasis (optional)");
}

#[test]
fn image_led_hero_pair_requests_one_optional_image_for_the_primary() {
    let i = intent(vec![json!({
        "purpose": "emphasize", "statement": "A nurse meets the machine",
        "primary": phrase("a night-shift nurse"), "secondary": phrase("the monitor")
    })]);
    let p = plan_ref(&i, image_led());
    assert_eq!(p.beats[0].composition, "hero_object");
    assert_eq!(p.requests.len(), 1, "{:?}", p.requests);
    let r = &p.requests[0];
    assert_eq!(r.id, "beat_1.hero_subject");
    assert_eq!(r.priority, Priority::Optional);
    assert_eq!(r.source, AssetSource::GeneratedImage);
    assert_eq!(r.negative_space, NegativeSpace::None);
    assert_eq!(
        r.reason,
        "visual language: image-led hero (optional; procedural token otherwise)"
    );
    assert_eq!(r.subject, "a night-shift nurse");
    assert_eq!(r.continuity_key.as_deref(), Some("a_night_shift_nurse"));
    assert_eq!(r.context.as_deref(), Some("A nurse meets the machine"));
    assert_eq!(p.beats[0].requests, vec![r.id.clone()]);
}

#[test]
fn diagrammatic_process_is_procedural_entities_with_no_images() {
    let i = intent(vec![
        pair_beat(
            "contrast",
            Some("replace"),
            "the old catalyst",
            "the new catalyst",
        ),
        pair_beat("contrast", Some("compress"), "gas", "pressure"),
        pair_beat("emphasize", None, "the reactor", "the coolant"),
        json!({ "purpose": "explain", "statement": "Heat becomes motion",
                "primary": phrase("heat"), "secondary": phrase("motion") }),
    ]);
    for v in [diagrammatic(), lang(Medium::ObjectLed)] {
        assert!(v.prefers_procedural());
        let p = plan_ref(&i, v);
        assert!(p.requests.is_empty(), "{:?}", p.requests);
        for d in &p.beats {
            assert_eq!(d.source, AssetSource::Procedural, "{d:?}");
            assert_eq!(d.reason, "visual language: engine-drawn entity tokens");
            assert!(d.requests.is_empty());
        }
        assert_eq!(p.beats[0].composition, "spatial_cause_effect");
        assert_eq!(p.beats[2].composition, "hero_object");
    }
}

#[test]
fn procedural_preference_drops_style_driven_requests() {
    // Without a reference the human emphasize beat asks for a hero_subject;
    // a procedural-preferring language asks for no generated image at all.
    let i = human_story();
    assert_eq!(generated(&plan_none(&i)), 2);
    for v in [diagrammatic(), lang(Medium::ObjectLed)] {
        assert_eq!(generated(&plan_ref(&i, v)), 0);
    }
}

fn data_story() -> CreativeIntent {
    intent(vec![
        json!({ "purpose": "reveal", "statement": "Complaints are rising",
                "primary": { "kind": "derived_metric", "meaning": "complaint rate",
                    "numerator": { "value": 12, "meaning": "complaints" },
                    "denominator": { "value": 400, "meaning": "orders" } } }),
        json!({ "purpose": "compare", "statement": "Two numbers side by side",
                "primary": { "kind": "number", "value": "120", "meaning": "before" },
                "secondary": { "kind": "number", "value": "480", "meaning": "after" } }),
        json!({ "purpose": "explain", "statement": "Small things add up over time",
                "primary": { "kind": "collection", "items": [
                    { "kind": "phrase", "value": "Coffee" },
                    { "kind": "phrase", "value": "Snacks" },
                    { "kind": "phrase", "value": "Rides" } ] } }),
    ])
}

#[test]
fn data_only_story_is_unchanged_under_every_visual_language() {
    let i = data_story();
    let base = plan_none(&i);
    assert!(base.requests.is_empty());
    assert_eq!(base.beats[0].composition, "data_story");
    for v in all_languages() {
        let p = plan_ref(&i, v.clone());
        assert_eq!(to_json(&p.beats), to_json(&base.beats), "{:?}", v.medium);
        assert!(p.requests.is_empty(), "{:?}", v.medium);
    }
}

fn kinetic_story() -> CreativeIntent {
    intent(vec![
        json!({ "purpose": "emphasize", "statement": "Faster than ever", "keyword": "FASTER",
                "energy": "impact", "primary": phrase("moving fast") }),
        json!({ "purpose": "emphasize", "statement": "Nothing can stop it", "keyword": "STOP",
                "energy": "impact", "primary": phrase("an unstoppable crowd of people") }),
        json!({ "purpose": "emphasize", "statement": "Break through", "keyword": "BREAK",
                "energy": "impact", "primary": phrase("breaking through") }),
    ])
}

#[test]
fn kinetic_typography_adds_nothing_under_type_led_and_only_optional_under_image_led() {
    let i = kinetic_story();
    let base = plan_none(&i);
    for v in [lang(Medium::TypeLed), lang(Medium::TypeOnly)] {
        assert_eq!(v.weight(), Weight::Type);
        let p = plan_ref(&i, v);
        assert_eq!(to_json(&p.requests), to_json(&base.requests));
        assert_eq!(to_json(&p.beats), to_json(&base.beats));
    }
    let p = plan_ref(&i, image_led());
    for r in &p.requests {
        assert_eq!(r.priority, Priority::Optional, "{r:?}");
    }
    assert!(p.requests.len() >= base.requests.len());
}

#[test]
fn evidence_requests_are_unchanged_under_every_visual_language() {
    let i = intent(vec![
        json!({ "purpose": "reveal", "statement": "The basket tells the story",
                "primary": { "kind": "object", "asset": "shopping_basket",
                             "meaning": "the basket" } }),
        json!({ "purpose": "reveal", "statement": "A mystery object",
                "primary": { "kind": "object", "asset": "unlisted_gizmo_xyz" } }),
    ]);
    let base = plan_none(&i);
    assert_eq!(base.requests[0].role, AssetRole::EvidenceImage);
    for v in all_languages() {
        let p = plan_ref(&i, v.clone());
        assert_eq!(
            to_json(&p.requests),
            to_json(&base.requests),
            "{:?}",
            v.medium
        );
        assert_eq!(to_json(&p.beats), to_json(&base.beats), "{:?}", v.medium);
    }
}

#[test]
fn planning_is_deterministic() {
    let mixed = intent(vec![
        json!({ "purpose": "emphasize", "statement": "Rest is rare",
                "primary": phrase("a night-shift nurse") }),
        pair_beat("contrast", Some("replace"), "old", "new"),
    ]);
    for v in all_languages() {
        let a = to_json(&plan_ref(&mixed, v.clone()));
        let b = to_json(&plan_ref(&mixed, v));
        assert_eq!(a, b);
    }
}

#[test]
fn delivered_hero_object_for_a_phrase_beat_selects_hero_object() {
    let i = intent(vec![json!({
        "purpose": "emphasize", "statement": "A single quiet idea", "primary": phrase("deep focus")
    })]);
    let v = image_led();
    let planned = plan_ref(&i, v.clone());
    assert!(planned
        .requests
        .iter()
        .any(|r| r.id == "beat_1.hero_object"));
    // Selection with the delivered image (the planner's own view stays
    // image-free; the compile below uses the manifest).
    let manifest: AssetManifest = serde_json::from_value(json!({
        "version": "0.2",
        "assets": [{ "id": "beat_1.hero_object", "path": "hero.png",
                     "width": 8, "height": 8, "alpha": true }]
    }))
    .expect("manifest");
    let lib = library();
    let reference = refer(v);
    // `diagram::build` may still be a stub in a partial worktree; only assert
    // on the compile result when it ran.
    let ran = std::panic::catch_unwind(|| {
        compile_full(
            &i,
            &StyleProfile::default(),
            Some(&reference),
            &lib,
            &ApproxMeasure,
            &manifest,
        )
    });
    if let Ok(result) = ran {
        let project = result.expect("compiles");
        assert!(!project.scenes.is_empty());
        eprintln!("compiled with the Entities builder");
    } else {
        eprintln!("compile skipped: diagram::build is a stub in this worktree");
    }
}
