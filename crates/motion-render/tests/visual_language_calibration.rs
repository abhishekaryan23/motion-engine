//! (0.7.1) Self-calibration of the visual construction language
//! (docs/VISUAL_LANGUAGE.md): the same neutral story is compiled with no
//! reference and with each interpreter-response fixture, with real font
//! measurement, and the measured outcome must follow the reference:
//!
//! - type-led stays typographic (no generated image, semantic grammars kept),
//! - image-led asks for (optional) images and diversifies away from type,
//! - diagrammatic draws entities procedurally and never asks for images,
//! - data stories stay data stories whatever the reference.
//!
//! The intent is deliberately generic (a delivery process), not any benchmark
//! story. Assertions are structural ratios / memberships, never pixel values.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use motion_core::assets::{AssetManifest, AssetPlan, AssetSource, Priority};
use motion_core::compiler::taste::ReferencePrinciples;
use motion_core::compiler::visual::Weight;
use motion_core::compiler::{
    compile_full, plan_assets_with_reference, resolve_taste, AssetLibrary, FontSet,
};
use motion_core::reference::{normalize, parse_profile};
use motion_core::{CreativeIntent, MotionProject, StyleProfile};
use motion_render::{visual_report, FontMeasure, VisualReport, VisualVerdict};
use serde_json::json;

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assets_root() -> PathBuf {
    repo().join("assets")
}

fn library() -> AssetLibrary {
    AssetLibrary::new(assets_root())
}

fn style() -> StyleProfile {
    serde_json::from_str("{}").expect("empty style")
}

/// Generic five-beat process story: one beat per recipe the visual language
/// can bias (calm pair, replace, compress, single impact phrase, reveal).
fn process_intent() -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2",
        "title": "calibration_process",
        "format": "vertical",
        "beats": [
            {
                "purpose": "emphasize",
                "statement": "Every parcel starts at a local depot.",
                "primary": { "kind": "phrase", "value": "local depot" },
                "secondary": { "kind": "phrase", "value": "delivery route" },
                "energy": "calm"
            },
            {
                "purpose": "contrast",
                "statement": "Handheld scanners replaced paper manifests.",
                "primary": { "kind": "phrase", "value": "paper manifests" },
                "secondary": { "kind": "phrase", "value": "handheld scanners" },
                "relationship": "replace",
                "energy": "building"
            },
            {
                "purpose": "contrast",
                "statement": "Rising volume squeezes every delivery window.",
                "primary": { "kind": "phrase", "value": "delivery window" },
                "secondary": { "kind": "phrase", "value": "rising volume" },
                "relationship": "compress",
                "energy": "building"
            },
            {
                "purpose": "emphasize",
                "statement": "One missed handoff stalls the whole route.",
                "primary": { "kind": "phrase", "value": "missed handoff" },
                "energy": "impact",
                "keyword": "missed"
            },
            {
                "purpose": "reveal",
                "statement": "The sorting hub sends it onward.",
                "primary": { "kind": "phrase", "value": "sorting hub" },
                "secondary": { "kind": "phrase", "value": "regional lanes" },
                "energy": "calm"
            }
        ]
    }))
    .expect("process intent parses")
}

fn compile(
    intent: &CreativeIntent,
    principles: Option<&ReferencePrinciples>,
) -> (MotionProject, AssetPlan) {
    let style = style();
    // Measure with the exact fonts the renderer will use (as the CLI does).
    let fonts = match principles {
        Some(_) => FontSet::for_pairing(resolve_taste(intent, &style, principles).typography),
        None => FontSet::for_style(&style),
    };
    let root = assets_root();
    let paths: Vec<PathBuf> = fonts.faces.iter().map(|f| root.join(f.path)).collect();
    let measure =
        FontMeasure::with_fonts(paths.iter().map(PathBuf::as_path)).expect("font measure");
    let project = compile_full(
        intent,
        &style,
        principles,
        &library(),
        &measure,
        &AssetManifest::empty(),
    )
    .expect("compile");
    let plan =
        plan_assets_with_reference(intent, &style, principles, &library()).expect("asset plan");
    (project, plan)
}

struct Profile {
    principles: ReferencePrinciples,
    weight: Weight,
}

fn profile(name: &str) -> Profile {
    let path = repo()
        .join("crates/motion-core/tests/fixtures/interpreter")
        .join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let parsed = parse_profile(&text, None).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let n = normalize(&parsed.profile);
    Profile {
        weight: n.principles.visual.weight(),
        principles: n.principles,
    }
}

fn generated(plan: &AssetPlan) -> Vec<&motion_core::assets::AssetRequest> {
    plan.requests
        .iter()
        .filter(|r| r.source == AssetSource::GeneratedImage)
        .collect()
}

fn grammars(plan: &AssetPlan) -> Vec<String> {
    plan.beats.iter().map(|b| b.composition.clone()).collect()
}

struct Measured {
    report: VisualReport,
    plan: AssetPlan,
}

fn measure(intent: &CreativeIntent, p: &Profile) -> Measured {
    let (project, plan) = compile(intent, Some(&p.principles));
    let report = visual_report(&project, Some(p.weight));
    assert!(
        report.skipped.is_empty(),
        "scenes skipped: {:?}",
        report.skipped
    );
    assert_eq!(report.beats.len(), intent.beats.len(), "one scene per beat");
    assert_eq!(plan.beats.len(), intent.beats.len());
    eprintln!(
        "[calibration] weight {:?} pure_type_ratio {:.2} verdict {:?} grammars {:?} generated {}",
        p.weight,
        report.pure_type_ratio,
        report.verdict,
        grammars(&plan),
        generated(&plan).len()
    );
    Measured { report, plan }
}

/// Grammars of the beats whose rendered scene is not type-dominant.
fn visual_beat_grammars(m: &Measured) -> Vec<String> {
    m.report
        .beats
        .iter()
        .zip(&m.plan.beats)
        .filter(|(b, _)| !b.type_dominant)
        .map(|(_, d)| d.composition.clone())
        .collect()
}

fn baseline(intent: &CreativeIntent) -> (MotionProject, AssetPlan) {
    compile(intent, None)
}

// ---------------------------------------------------------------------------
// Process story
// ---------------------------------------------------------------------------

#[test]
fn type_led_reference_keeps_the_story_typographic() {
    let intent = process_intent();
    let m = measure(&intent, &profile("type_led_poster"));
    let (_, base_plan) = baseline(&intent);

    assert_eq!(m.report.verdict, Some(VisualVerdict::Consistent));
    assert!(
        m.report.pure_type_ratio >= 0.6,
        "pure_type_ratio {} for a type-led reference",
        m.report.pure_type_ratio
    );
    assert!(
        generated(&m.plan).is_empty(),
        "type-led reference requested images: {:?}",
        generated(&m.plan)
    );
    // The type-led bias never overrides the semantic choice.
    assert_eq!(grammars(&m.plan), grammars(&base_plan));
}

#[test]
fn image_led_reference_asks_for_optional_images_and_diversifies() {
    let intent = process_intent();
    let m = measure(&intent, &profile("image_led_editorial"));

    assert_ne!(
        m.report.verdict,
        Some(VisualVerdict::VisualTransferLikelyFailed)
    );
    assert!(
        m.report.pure_type_ratio <= 0.6,
        "pure_type_ratio {} for an image-led reference",
        m.report.pure_type_ratio
    );
    let images = generated(&m.plan);
    assert!(
        !images.is_empty(),
        "image-led reference produced no generated_image request"
    );
    assert!(
        images.iter().all(|r| r.priority == Priority::Optional),
        "image requests must be optional: {images:?}"
    );
    let distinct: BTreeSet<String> = visual_beat_grammars(&m).into_iter().collect();
    assert!(
        distinct.len() >= 2,
        "non-type-dominant beats use only {distinct:?}"
    );
}

#[test]
fn diagrammatic_reference_draws_entities_procedurally() {
    let intent = process_intent();
    let m = measure(&intent, &profile("diagrammatic_explainer"));

    assert_eq!(m.report.verdict, Some(VisualVerdict::Consistent));
    assert!(
        generated(&m.plan).is_empty(),
        "diagrammatic reference requested images: {:?}",
        generated(&m.plan)
    );
    let non_type = m.report.beats.iter().filter(|b| !b.type_dominant).count();
    assert!(
        non_type >= 3,
        "only {non_type} non-type-dominant beats: {:?}",
        m.report.beats
    );
    // Entity beats are engine-drawn: procedural decisions, no external request.
    let entity: Vec<_> = m
        .plan
        .beats
        .iter()
        .filter(|d| d.composition == "hero_object" || d.composition == "spatial_cause_effect")
        .collect();
    assert!(
        !entity.is_empty(),
        "no entity beat: {:?}",
        grammars(&m.plan)
    );
    for d in entity {
        assert_eq!(d.source, AssetSource::Procedural, "beat {}", d.beat);
        assert!(d.requests.is_empty(), "beat {}: {:?}", d.beat, d.requests);
    }
}

#[test]
fn mixed_reference_sits_between_the_extremes() {
    let intent = process_intent();
    let m = measure(&intent, &profile("mixed_promo"));
    // Optional images only; never required, never a failed-transfer verdict.
    assert!(generated(&m.plan)
        .iter()
        .all(|r| r.priority == Priority::Optional));
    assert_ne!(
        m.report.verdict,
        Some(VisualVerdict::VisualTransferLikelyFailed)
    );
}

#[test]
fn bias_is_not_more_images_everywhere() {
    // Same story, three references: the image request counts must order
    // image-led > 0 and type-led == diagrammatic == 0.
    let intent = process_intent();
    let type_led = measure(&intent, &profile("type_led_poster"));
    let image_led = measure(&intent, &profile("image_led_editorial"));
    let diagram = measure(&intent, &profile("diagrammatic_explainer"));
    assert!(generated(&image_led.plan).len() > generated(&type_led.plan).len());
    assert_eq!(generated(&type_led.plan).len(), 0);
    assert_eq!(generated(&diagram.plan).len(), 0);
    assert!(type_led.report.pure_type_ratio > diagram.report.pure_type_ratio);
}

#[test]
fn no_reference_equals_default_principles() {
    let intent = process_intent();
    let (none, none_plan) = compile(&intent, None);
    let default = ReferencePrinciples::default();
    let (with_default, default_plan) = compile(&intent, Some(&default));
    assert_eq!(none.to_json_pretty(), with_default.to_json_pretty());
    assert_eq!(none_plan, default_plan);
    assert!(generated(&none_plan).is_empty());
}

// ---------------------------------------------------------------------------
// Data regression
// ---------------------------------------------------------------------------

fn complaint_intent() -> CreativeIntent {
    let path: &Path = &repo()
        .join("golden/fixtures/benchmarks/complaint-rate-reversal-02/attempt-01.intent.json");
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).expect("complaint intent parses")
}

#[test]
fn data_story_stays_data_under_every_visual_reference() {
    let intent = complaint_intent();
    let style = style();
    let base = plan_assets_with_reference(&intent, &style, None, &library()).expect("plan");
    let base_grammars = grammars(&base);
    assert!(
        base_grammars.iter().all(|g| g == "data_story"),
        "baseline is not all data_story: {base_grammars:?}"
    );

    for name in [
        "type_led_poster",
        "image_led_editorial",
        "diagrammatic_explainer",
        "mixed_promo",
    ] {
        let p = profile(name);
        let plan = plan_assets_with_reference(&intent, &style, Some(&p.principles), &library())
            .expect("plan");
        assert_eq!(grammars(&plan), base_grammars, "{name}");
        assert!(
            generated(&plan).is_empty(),
            "{name}: {:?}",
            generated(&plan)
        );

        // The compiled project is a valid scene too (real font measurement).
        let (project, _) = compile(&intent, Some(&p.principles));
        assert!(!project.scenes.is_empty(), "{name}");
    }
}
