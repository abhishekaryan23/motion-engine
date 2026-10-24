//! Prompt-spec derivation (docs/GENERATED_ASSET_PROTOCOL.md) and the asset
//! cache index. Intents are synthetic.

use std::path::PathBuf;

use motion_core::asset_prompts::prompt_specs;
use motion_core::assets::AssetSource;
use motion_core::assets::{
    AssetAnalysis, AssetCacheIndex, AssetPlan, AssetPromptSpec, AssetRequest, AssetRole,
    AssetStyleProfile, Background, CacheConflict, CacheEntry, CacheInsert, EdgeContact, Framing,
    NegativeSpace, NormBox, OccupancyGrid, Presentation, Priority, ASSET_PLAN_VERSION,
    ASSET_PROMPTS_VERSION,
};
use motion_core::compiler::plan_assets;
use motion_core::intent::CreativeIntent;
use motion_core::style::StyleProfile;
use motion_core::AssetLibrary;
use serde_json::{json, Value};

fn repo_assets() -> AssetLibrary {
    AssetLibrary::new(PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets"
    )))
}

fn plan_of(beats: Vec<Value>) -> AssetPlan {
    let intent: CreativeIntent = serde_json::from_value(json!({
        "version": "0.2",
        "title": "asset_prompts_test",
        "beats": beats,
    }))
    .expect("intent parses");
    plan_assets(&intent, &StyleProfile::default(), &repo_assets()).expect("plan")
}

fn worker_beat(value: &str, meaning: &str, statement: &str) -> Value {
    json!({
        "purpose": "emphasize",
        "statement": statement,
        "primary": { "kind": "phrase", "value": value, "meaning": meaning },
        "keyword": "night"
    })
}

// ---------------------------------------------------------------------------
// Hand-built plans
// ---------------------------------------------------------------------------

fn style() -> AssetStyleProfile {
    AssetStyleProfile::from_style(&StyleProfile::default())
}

#[allow(clippy::too_many_arguments)]
fn req(
    id: &str,
    role: AssetRole,
    subject: &str,
    presentation: Presentation,
    negative_space: NegativeSpace,
    background: Background,
    priority: Priority,
    key: Option<&str>,
) -> AssetRequest {
    AssetRequest {
        id: id.into(),
        beat: 1,
        source: AssetSource::GeneratedImage,
        role,
        subject: subject.into(),
        presentation,
        negative_space,
        background,
        priority,
        composition: "type_image_interlock".into(),
        reason: "test".into(),
        library_asset: None,
        context: Some("A statement".into()),
        continuity_key: key.map(str::to_string),
    }
}

fn cutout(id: &str, role: AssetRole, priority: Priority) -> AssetRequest {
    req(
        id,
        role,
        "Office worker, worker",
        Presentation::IsolatedCutout,
        NegativeSpace::Right,
        Background::Transparent,
        priority,
        Some("office_worker"),
    )
}

fn plan_with(requests: Vec<AssetRequest>) -> AssetPlan {
    AssetPlan {
        version: ASSET_PLAN_VERSION.into(),
        style: style(),
        beats: Vec::new(),
        requests,
    }
}

/// A literal style so the pinned fingerprint does not depend on `from_style`.
fn fixed_style() -> AssetStyleProfile {
    AssetStyleProfile {
        medium: "editorial photo collage, printed on paper".into(),
        realism: "photographic, unretouched, documentary".into(),
        lighting: "soft even daylight".into(),
        contrast: "medium contrast".into(),
        palette_tendency: "muted neutrals with a signal red accent".into(),
        edge_treatment: "hand-cut paper edge".into(),
        shadow_treatment: "soft contact shadow, slight lift off the page".into(),
        camera_feel: "35-50mm, eye level, subject centered in its own frame".into(),
        background_behavior: "isolated subject on transparency unless a plate is requested".into(),
    }
}

fn fixed_spec() -> AssetPromptSpec {
    let mut plan = plan_with(vec![cutout(
        "beat_1.hero_subject",
        AssetRole::HeroSubject,
        Priority::Optional,
    )]);
    plan.style = fixed_style();
    prompt_specs(&plan).specs.remove(0)
}

// ---------------------------------------------------------------------------
// Derivation
// ---------------------------------------------------------------------------

#[test]
fn human_emphasize_beat_yields_one_half_figure_cutout_spec() {
    let plan = plan_of(vec![worker_beat(
        "Office worker",
        "worker",
        "An office worker sits alone at 11:47 PM.",
    )]);
    let set = prompt_specs(&plan);
    assert_eq!(set.version, ASSET_PROMPTS_VERSION);
    assert_eq!(set.specs.len(), 1, "{:?}", plan.requests);
    let s = &set.specs[0];
    assert_eq!(s.id, "beat_1.hero_subject");
    assert_eq!(s.serves, vec!["beat_1.hero_subject".to_string()]);
    assert_eq!(s.continuity_key, "office_worker");
    assert_eq!(s.role, AssetRole::HeroSubject);
    assert_eq!(s.composition.framing, Framing::HalfFigure);
    assert_eq!(s.composition.negative_space, NegativeSpace::None);
    assert!(!s.composition.subject_whole);
    assert!(s.composition.head_inside_frame);
    assert!(s.output.alpha_required);
    assert_eq!(s.output.min_short_side, 1024);
    assert_eq!(s.output.aspect, "3:4");
    assert_eq!(s.context, "An office worker sits alone at 11:47 PM.");
    assert_eq!(s.fingerprint, s.compute_fingerprint());
    let hex = s.fingerprint.strip_prefix("fp1-").expect("fp1 prefix");
    assert_eq!(hex.len(), 16);
    assert_eq!(s.output.file_stem, format!("office_worker-{}", &hex[..10]));
    assert!(s.prompt.starts_with("Office worker, worker."));
    assert!(s.prompt.contains("transparent"));
    assert!(s.prompt.contains("Context: An office worker sits alone"));
    assert!(s.prompt.contains("Half figure, head to waist"));
    assert!(!s.prompt.contains("Leave empty space"));
    assert!(s.prompt.contains("Style: "));
    assert!(s.prompt.ends_with("distorted hands."));
    assert_eq!(
        s.avoid,
        vec![
            "embedded text",
            "letters or numbers",
            "logos",
            "watermark",
            "signature",
            "border or frame",
            "background scenery",
            "cast shadow on the ground",
            "cropped head",
            "extra fingers",
            "distorted hands",
        ]
    );
}

#[test]
fn same_human_phrase_in_two_beats_is_one_spec_serving_both() {
    let plan = plan_of(vec![
        worker_beat("Office worker", "worker", "First statement here"),
        worker_beat("Office worker", "worker", "Second statement there"),
    ]);
    assert_eq!(plan.requests.len(), 2, "{:?}", plan.requests);
    let set = prompt_specs(&plan);
    assert_eq!(set.specs.len(), 1);
    let s = &set.specs[0];
    assert_eq!(s.id, "beat_1.hero_subject");
    assert_eq!(
        s.serves,
        vec![
            "beat_1.hero_subject".to_string(),
            "beat_2.hero_subject".to_string()
        ]
    );
    // Copied from the FIRST request of the group.
    assert_eq!(s.context, "First statement here");
    assert!(set.for_request("beat_2.hero_subject").is_some());
}

#[test]
fn same_subject_with_different_framing_is_two_specs() {
    let plan = plan_of(vec![
        worker_beat("Office worker", "worker", "First statement here"),
        worker_beat("Office worker", "portrait", "Second statement there"),
    ]);
    let roles: Vec<AssetRole> = plan.requests.iter().map(|r| r.role).collect();
    assert_eq!(roles, vec![AssetRole::HeroSubject, AssetRole::Portrait]);
    let set = prompt_specs(&plan);
    assert_eq!(set.specs.len(), 2);
    let (a, b) = (&set.specs[0], &set.specs[1]);
    assert_eq!(a.continuity_key, b.continuity_key);
    assert_eq!(a.composition.framing, Framing::HalfFigure);
    assert_eq!(b.composition.framing, Framing::HeadAndShoulders);
    assert_eq!(b.output.aspect, "4:5");
    assert_ne!(a.fingerprint, b.fingerprint);
    assert_ne!(a.output.file_stem, b.output.file_stem);
    assert_eq!(a.serves, vec!["beat_1.hero_subject".to_string()]);
    assert_eq!(b.serves, vec!["beat_2.portrait".to_string()]);
}

#[test]
fn data_only_and_library_plans_yield_no_specs() {
    let numbers = plan_of(vec![json!({
        "purpose": "compare",
        "statement": "Two numbers side by side",
        "primary": { "kind": "number", "value": "120", "meaning": "before" },
        "secondary": { "kind": "number", "value": "480", "meaning": "after" }
    })]);
    assert!(prompt_specs(&numbers).specs.is_empty());

    let metric = plan_of(vec![json!({
        "purpose": "reveal",
        "statement": "Most visitors never sign up",
        "primary": {
            "kind": "derived_metric",
            "numerator": { "value": 80, "meaning": "sign-ups" },
            "denominator": { "value": 1000, "meaning": "visitors" }
        }
    })]);
    assert!(prompt_specs(&metric).specs.is_empty());

    let svg = plan_of(vec![json!({
        "purpose": "reveal",
        "statement": "The basket tells the story",
        "primary": { "kind": "object", "asset": "shopping_basket", "meaning": "the basket" }
    })]);
    assert_eq!(svg.requests.len(), 1);
    assert_eq!(svg.requests[0].source, AssetSource::Svg);
    assert!(prompt_specs(&svg).specs.is_empty());
}

#[test]
fn priority_is_required_if_any_served_request_is_required() {
    let optional_only = plan_with(vec![
        cutout(
            "beat_1.hero_subject",
            AssetRole::HeroSubject,
            Priority::Optional,
        ),
        cutout(
            "beat_2.hero_subject",
            AssetRole::HeroSubject,
            Priority::Optional,
        ),
    ]);
    assert_eq!(
        prompt_specs(&optional_only).specs[0].priority,
        Priority::Optional
    );

    let mixed = plan_with(vec![
        cutout(
            "beat_1.hero_subject",
            AssetRole::HeroSubject,
            Priority::Optional,
        ),
        cutout(
            "beat_2.hero_subject",
            AssetRole::HeroSubject,
            Priority::Required,
        ),
    ]);
    let set = prompt_specs(&mixed);
    assert_eq!(set.specs.len(), 1);
    assert_eq!(set.specs[0].priority, Priority::Required);
    // Role and id come from the first request.
    assert_eq!(set.specs[0].id, "beat_1.hero_subject");
}

#[test]
fn foreground_occluder_and_hero_subject_share_a_half_figure_group() {
    let plan = plan_with(vec![
        cutout(
            "beat_1.hero_subject",
            AssetRole::HeroSubject,
            Priority::Optional,
        ),
        cutout(
            "beat_2.foreground_occluder",
            AssetRole::ForegroundOccluder,
            Priority::Optional,
        ),
    ]);
    let set = prompt_specs(&plan);
    assert_eq!(set.specs.len(), 1);
    assert_eq!(set.specs[0].serves.len(), 2);
}

#[test]
fn negative_space_is_kept_only_for_non_cutouts() {
    let plate = req(
        "beat_1.environment",
        AssetRole::Environment,
        "Night street \u{2014} environment",
        Presentation::BackgroundPlate,
        NegativeSpace::Left,
        Background::Opaque,
        Priority::Optional,
        Some("night_street_environment"),
    );
    let artifact = req(
        "beat_1.evidence_image",
        AssetRole::EvidenceImage,
        "A receipt",
        Presentation::FlatArtifact,
        NegativeSpace::Top,
        Background::Opaque,
        Priority::Required,
        Some("receipt"),
    );
    let object = req(
        "beat_1.hero_object",
        AssetRole::HeroObject,
        "A red kettle",
        Presentation::IsolatedCutout,
        NegativeSpace::Bottom,
        Background::Transparent,
        Priority::Required,
        Some("red_kettle"),
    );
    let set = prompt_specs(&plan_with(vec![plate, artifact, object]));
    assert_eq!(set.specs.len(), 3);

    let plate = &set.specs[0];
    assert_eq!(plate.composition.framing, Framing::Scene);
    assert_eq!(plate.composition.negative_space, NegativeSpace::Left);
    assert!(!plate.composition.subject_whole);
    assert!(!plate.composition.head_inside_frame);
    assert!(!plate.output.alpha_required);
    assert_eq!(plate.output.min_short_side, 1080);
    assert_eq!(plate.output.aspect, "3:4");
    assert!(plate.prompt.contains("Leave empty space on the left."));
    assert!(plate
        .prompt
        .contains("Soft environmental background plate."));
    assert!(!plate.avoid.iter().any(|a| a == "background scenery"));

    let artifact = &set.specs[1];
    assert_eq!(artifact.composition.framing, Framing::Artifact);
    assert_eq!(artifact.composition.negative_space, NegativeSpace::None);
    assert!(artifact.composition.subject_whole);
    assert_eq!(artifact.output.min_short_side, 900);
    assert!(!artifact.output.alpha_required);
    assert_eq!(artifact.priority, Priority::Required);

    let object = &set.specs[2];
    assert_eq!(object.composition.framing, Framing::Object);
    assert_eq!(object.composition.negative_space, NegativeSpace::None);
    assert!(object.composition.subject_whole);
    assert!(!object.composition.head_inside_frame);
    assert_eq!(object.output.aspect, "1:1");
    assert!(object.output.alpha_required);
    assert!(!object.avoid.iter().any(|a| a == "extra fingers"));
    assert!(object
        .avoid
        .iter()
        .any(|a| a == "cast shadow on the ground"));
}

#[test]
fn missing_continuity_key_falls_back_to_the_subject() {
    let mut r = cutout(
        "beat_1.hero_subject",
        AssetRole::HeroSubject,
        Priority::Optional,
    );
    r.continuity_key = None;
    r.subject = "Tired night nurse at the desk today".into();
    let s = prompt_specs(&plan_with(vec![r])).specs.remove(0);
    assert_eq!(s.continuity_key, "tired_night_nurse_at");
    assert!(s.output.file_stem.starts_with("tired_night_nurse_at-"));
}

#[test]
fn prompt_specs_is_deterministic() {
    let plan = plan_of(vec![
        worker_beat("Office worker", "worker", "First statement here"),
        worker_beat("Office worker", "portrait", "Second statement there"),
    ]);
    let a = prompt_specs(&plan).to_json_pretty();
    let b = prompt_specs(&plan).to_json_pretty();
    assert_eq!(a, b);
    assert!(!a.is_empty());
}

// ---------------------------------------------------------------------------
// Fingerprint
// ---------------------------------------------------------------------------

#[test]
fn fingerprint_ignores_bookkeeping_fields_and_tracks_art_direction() {
    let base = fixed_spec();
    let fp = base.compute_fingerprint();
    assert_eq!(fp, base.fingerprint);

    let mut s = base.clone();
    s.id = "beat_9.hero_subject".into();
    s.serves = vec!["beat_9.hero_subject".into(), "beat_10.hero_subject".into()];
    s.role = AssetRole::ForegroundOccluder;
    s.priority = Priority::Required;
    s.continuity_key = "someone_else".into();
    s.context = "A completely different statement".into();
    s.prompt = "another prompt".into();
    s.output.file_stem = "whatever".into();
    s.fingerprint = "fp1-0000000000000000".into();
    assert_eq!(s.compute_fingerprint(), fp);

    // Whitespace / case in the subject is normalized.
    let mut s = base.clone();
    s.subject = "  OFFICE   worker, WORKER ".into();
    assert_eq!(s.compute_fingerprint(), fp);

    let mut s = base.clone();
    s.style.lighting = "hard flash".into();
    assert_ne!(s.compute_fingerprint(), fp);

    let mut s = base.clone();
    s.subject = "Office worker, standing".into();
    assert_ne!(s.compute_fingerprint(), fp);

    let mut s = base.clone();
    s.composition.framing = Framing::HeadAndShoulders;
    assert_ne!(s.compute_fingerprint(), fp);

    let mut s = base;
    s.avoid.push("hats".into());
    assert_ne!(s.compute_fingerprint(), fp);
}

#[test]
fn style_change_changes_the_fingerprint_through_the_plan() {
    let a = fixed_spec();
    let mut plan = plan_with(vec![cutout(
        "beat_1.hero_subject",
        AssetRole::HeroSubject,
        Priority::Optional,
    )]);
    plan.style = fixed_style();
    plan.style.palette_tendency = "muted neutrals with a cobalt blue accent".into();
    let b = prompt_specs(&plan).specs.remove(0);
    assert_ne!(a.fingerprint, b.fingerprint);
    assert_ne!(a.output.file_stem, b.output.file_stem);
    // Editing the statement does not.
    let mut plan = plan_with(vec![cutout(
        "beat_1.hero_subject",
        AssetRole::HeroSubject,
        Priority::Optional,
    )]);
    plan.style = fixed_style();
    plan.requests[0].context = Some("Edited statement".into());
    assert_eq!(prompt_specs(&plan).specs[0].fingerprint, a.fingerprint);
}

#[test]
fn fingerprint_literal_is_pinned() {
    // FROZEN v1: if this changes, every cached asset is invalidated.
    let s = fixed_spec();
    assert_eq!(s.fingerprint, PINNED_FINGERPRINT);
    assert_eq!(
        s.output.file_stem,
        format!("office_worker-{}", &PINNED_FINGERPRINT[4..14])
    );
}

const PINNED_FINGERPRINT: &str = "fp1-4c5899b614002780";

// ---------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------

fn entry(fp: &str, hash: &str) -> CacheEntry {
    CacheEntry {
        fingerprint: fp.into(),
        continuity_key: "office_worker".into(),
        file: format!("{}.png", fp.trim_start_matches("fp1-")),
        content_hash: hash.into(),
        width: 1024,
        height: 1365,
        alpha: true,
        analysis: AssetAnalysis {
            subject_bounds: NormBox {
                x: 0.1,
                y: 0.0,
                width: 0.8,
                height: 1.0,
            },
            edges: EdgeContact {
                top: false,
                bottom: true,
                left: false,
                right: false,
            },
            coverage: 0.5,
            occupancy: OccupancyGrid {
                cols: 2,
                rows: vec!["0f".into(), "ff".into()],
            },
            safe_regions: Vec::new(),
            head_estimate: None,
            monochrome: false,
            mean_color: None,
        },
        face_bounds: None,
        head_bounds: None,
        face_anchor: None,
        subject_anchor: None,
        keyed: false,
        generator: serde_json::Map::new(),
    }
}

#[test]
fn cache_added_unchanged_conflict_replaced() {
    let mut idx = AssetCacheIndex::new();
    assert_eq!(
        idx.insert(entry("fp1-bbbb", "fnv1a64:1"), false),
        Ok(CacheInsert::Added)
    );
    assert_eq!(idx.entries.len(), 1);
    assert_eq!(
        idx.lookup("fp1-bbbb").map(|e| e.content_hash.as_str()),
        Some("fnv1a64:1")
    );
    assert!(idx.lookup("fp1-zzzz").is_none());

    assert_eq!(
        idx.insert(entry("fp1-bbbb", "fnv1a64:1"), false),
        Ok(CacheInsert::Unchanged)
    );
    // replace=true with identical content is still Unchanged.
    assert_eq!(
        idx.insert(entry("fp1-bbbb", "fnv1a64:1"), true),
        Ok(CacheInsert::Unchanged)
    );
    assert_eq!(idx.entries.len(), 1);

    let err = idx
        .insert(entry("fp1-bbbb", "fnv1a64:2"), false)
        .expect_err("changed content is refused");
    assert_eq!(
        err,
        CacheConflict::ContentChanged {
            fingerprint: "fp1-bbbb".into(),
            cached: "fnv1a64:1".into(),
            incoming: "fnv1a64:2".into(),
        }
    );
    // A refused insert leaves the index untouched.
    assert_eq!(
        idx.lookup("fp1-bbbb").map(|e| e.content_hash.as_str()),
        Some("fnv1a64:1")
    );

    assert_eq!(
        idx.insert(entry("fp1-bbbb", "fnv1a64:2"), true),
        Ok(CacheInsert::Replaced)
    );
    assert_eq!(idx.entries.len(), 1);
    assert_eq!(
        idx.lookup("fp1-bbbb").map(|e| e.content_hash.as_str()),
        Some("fnv1a64:2")
    );
}

#[test]
fn cache_entries_stay_sorted_by_fingerprint() {
    let mut idx = AssetCacheIndex::new();
    for fp in ["fp1-cccc", "fp1-aaaa", "fp1-dddd", "fp1-bbbb"] {
        assert_eq!(
            idx.insert(entry(fp, "fnv1a64:1"), false),
            Ok(CacheInsert::Added)
        );
    }
    let fps: Vec<&str> = idx.entries.iter().map(|e| e.fingerprint.as_str()).collect();
    assert_eq!(fps, vec!["fp1-aaaa", "fp1-bbbb", "fp1-cccc", "fp1-dddd"]);
    idx.insert(entry("fp1-bbbb", "fnv1a64:9"), true)
        .expect("replace");
    let fps: Vec<&str> = idx.entries.iter().map(|e| e.fingerprint.as_str()).collect();
    assert_eq!(fps, vec!["fp1-aaaa", "fp1-bbbb", "fp1-cccc", "fp1-dddd"]);
}

#[test]
fn cache_index_json_round_trips() {
    let mut idx = AssetCacheIndex::new();
    idx.insert(entry("fp1-2222", "fnv1a64:2"), false)
        .expect("add");
    idx.insert(entry("fp1-1111", "fnv1a64:1"), false)
        .expect("add");
    let json = idx.to_json_pretty();
    let back = AssetCacheIndex::from_json(&json).expect("parse");
    assert_eq!(back, idx);
    assert_eq!(back.to_json_pretty(), json);
}

#[test]
fn prompt_set_json_round_trips() {
    let plan = plan_of(vec![worker_beat("Office worker", "worker", "A statement")]);
    let set = prompt_specs(&plan);
    let back =
        motion_core::assets::AssetPromptSet::from_json(&set.to_json_pretty()).expect("parse");
    assert_eq!(back, set);
}
