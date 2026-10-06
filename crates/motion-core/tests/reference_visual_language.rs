//! ReferenceStyleProfile v0.2 (`visual_language`, 0.7.1): serialization, v0.1
//! compatibility, validation, alias folding, normalization and schemas.

use std::path::{Path, PathBuf};

use motion_core::assets::AssetRole;
use motion_core::compiler::visual::{
    Balance, Character, Explanation, Medium, Usage, VisualLanguage, Weight,
};
use motion_core::compiler::Grammar;
use motion_core::reference::bundle::response_schema;
use motion_core::reference::normalize::{normalize, Fidelity, NormalizedReference};
use motion_core::reference::profile::{
    ReferenceStyleProfile, REFERENCE_PROFILE_VERSION, REFERENCE_PROFILE_VERSION_V0_1,
};
use motion_core::reference::{parse_profile, ParsedProfile};
use serde_json::{json, Value};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/visual_language")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn parse(doc: &Value) -> Result<ParsedProfile, Vec<(String, String)>> {
    parse_profile(&doc.to_string(), None)
        .map_err(|e| e.into_iter().map(|i| (i.path, i.message)).collect())
}

fn parse_ok(doc: &Value) -> ParsedProfile {
    parse(doc).unwrap_or_else(|e| panic!("expected valid profile, got {e:?}"))
}

fn issue_paths(doc: &Value) -> Vec<String> {
    parse(doc)
        .expect_err("expected rejection")
        .into_iter()
        .map(|(p, _)| p)
        .collect()
}

fn vl(body: Value) -> Value {
    json!({ "version": "0.2", "visual_language": body })
}

fn normalized_fixture(name: &str) -> (ReferenceStyleProfile, NormalizedReference) {
    let parsed = parse_profile(&fixture(name), None)
        .unwrap_or_else(|e| panic!("fixture {name} invalid: {e:?}"));
    let n = normalize(&parsed.profile);
    (parsed.profile, n)
}

fn row<'a>(
    n: &'a NormalizedReference,
    dim: &str,
) -> &'a motion_core::reference::normalize::DimensionMapping {
    n.dimensions
        .iter()
        .find(|d| d.dimension == dim)
        .unwrap_or_else(|| panic!("no row {dim}"))
}

// ---------------------------------------------------------------------------
// v0.1 compatibility
// ---------------------------------------------------------------------------

fn collect_v0_1_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("reference-style") && n.ends_with(".json"))
            {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    // CLI fixtures: every `reference_*.json` except the deliberately invalid one.
    if let Ok(rd) = std::fs::read_dir(root().join("crates/motion-cli/tests/fixtures")) {
        for e in rd.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.starts_with("reference_")
                && name.ends_with(".json")
                && name != "reference_invalid.json"
            {
                files.push(p);
            }
        }
    }
    walk(&root().join("references"), &mut files);
    files.sort();
    files
}

#[test]
fn every_existing_v0_1_profile_still_parses_and_converts() {
    let files = collect_v0_1_files();
    assert!(!files.is_empty(), "no v0.1 fixtures found");
    for f in files {
        let text = std::fs::read_to_string(&f).expect("read");
        let doc: Value = serde_json::from_str(&text).expect("json");
        if doc["version"] != "0.1" {
            continue;
        }
        let parsed = parse_profile(&text, None)
            .unwrap_or_else(|e| panic!("{} no longer parses: {e:?}", f.display()));
        assert_eq!(parsed.profile.version, REFERENCE_PROFILE_VERSION_V0_1);
        assert!(parsed.profile.visual_language.is_none(), "{}", f.display());
        // v0.1 normalizes with a neutral visual language.
        let n = normalize(&parsed.profile);
        assert_eq!(n.principles.visual, VisualLanguage::neutral());
        assert_eq!(n.principles.visual.weight(), Weight::Neutral);
    }
}

#[test]
fn v0_1_conversion_is_lossless_and_keeps_the_version() {
    let doc = json!({
        "version": "0.1",
        "reference_fingerprint": "rf1-0123456789abcdef",
        "tone": { "value": "editorial", "confidence": 0.9 },
        "approximate_palette_evidence": [{ "hex": "#112233", "prevalence": 0.4 }],
        "layer_activity": { "value": { "foreground": "dominant" }, "confidence": 0.6 },
        "unsupported_reference_traits": [{ "trait": "true3d", "confidence": 0.7 }]
    });
    let legacy: motion_core::reference::profile_v0_1::ReferenceStyleProfile =
        serde_json::from_value(doc.clone()).expect("v0.1 parses");
    let current: ReferenceStyleProfile = legacy.clone().into();
    assert_eq!(current.version, "0.1");
    assert!(current.visual_language.is_none());
    // Serializing the converted profile gives exactly what the v0.1 struct serializes
    // (no field gained, lost or renamed; no visual_language key).
    let (c, l) = (
        serde_json::to_value(&current).expect("ser"),
        serde_json::to_value(&legacy).expect("ser"),
    );
    assert_eq!(c, l);
    assert!(c.get("visual_language").is_none());
    // And the current struct reads the same document to the same value.
    let direct: ReferenceStyleProfile =
        serde_json::from_value(doc).expect("current parses v0.1 doc");
    assert_eq!(direct, current);
}

#[test]
fn v0_1_with_visual_language_is_rejected() {
    let doc = json!({
        "version": "0.1",
        "visual_language": { "medium": { "value": "image_led", "confidence": 0.9 } }
    });
    let issues = parse(&doc).expect_err("must be rejected");
    assert!(
        issues.iter().any(|(_, m)| m.contains("visual_language")),
        "{issues:?}"
    );
    // ... even when the section is empty.
    assert!(parse(&json!({ "version": "0.1", "visual_language": {} })).is_err());
}

#[test]
fn unknown_or_missing_version_is_rejected_at_version() {
    for doc in [
        json!({ "version": "0.3" }),
        json!({ "version": "1.0" }),
        json!({ "version": 2 }),
        json!({ "tone": { "value": "editorial", "confidence": 0.9 } }),
    ] {
        assert_eq!(issue_paths(&doc), vec!["version".to_string()], "{doc}");
    }
}

#[test]
fn version_constants_are_v0_2_current_and_v0_1_legacy() {
    assert_eq!(REFERENCE_PROFILE_VERSION, "0.2");
    assert_eq!(REFERENCE_PROFILE_VERSION_V0_1, "0.1");
    assert!(parse(&json!({ "version": "0.2" })).is_ok());
}

// ---------------------------------------------------------------------------
// v0.2 examples
// ---------------------------------------------------------------------------

#[test]
fn type_led_example_normalizes_to_type_weight() {
    let (p, n) = normalized_fixture("type_led");
    assert_eq!(p.version, "0.2");
    let v = &n.principles.visual;
    assert_eq!(v.medium, Medium::TypeLed);
    assert_eq!(v.usage, Some(Usage::Sparse));
    assert_eq!(v.balance, Some(Balance::TypeDominant));
    assert_eq!(v.character, None, "null value stays unknown");
    assert_eq!(v.roles, vec![AssetRole::HeroObject]);
    assert_eq!(
        v.preferred,
        vec![Grammar::KineticPoster, Grammar::EditorialCollage]
    );
    assert_eq!(v.secondary, vec![Grammar::SplitContrast]);
    assert_eq!(v.avoid, vec![Grammar::CinematicMultiplane]);
    assert_eq!(v.explanation, Some(Explanation::Literal));
    assert_eq!(v.weight(), Weight::Type);
    assert_eq!(
        row(&n, "visual.asset_character").fidelity,
        Fidelity::Unknown
    );
    assert_eq!(row(&n, "visual.medium").fidelity, Fidelity::Exact);
}

#[test]
fn image_led_example_normalizes_to_visual_weight() {
    let (_, n) = normalized_fixture("image_led");
    let v = &n.principles.visual;
    assert_eq!(v.medium, Medium::ImageLed);
    assert_eq!(v.usage, Some(Usage::Dense));
    assert_eq!(v.balance, Some(Balance::VisualDominant));
    assert_eq!(v.character, Some(Character::Photographic));
    assert_eq!(
        v.roles,
        vec![
            AssetRole::HeroSubject,
            AssetRole::Environment,
            AssetRole::Portrait
        ]
    );
    assert_eq!(
        v.preferred,
        vec![Grammar::TypeImageInterlock, Grammar::CinematicMultiplane]
    );
    assert_eq!(v.secondary, vec![Grammar::HeroObject]);
    assert_eq!(v.avoid, vec![Grammar::DataStory]);
    assert_eq!(v.explanation, Some(Explanation::Literal));
    assert_eq!(v.weight(), Weight::Visual);
    assert!(!v.prefers_procedural());
}

#[test]
fn diagrammatic_example_normalizes_to_visual_weight_and_prefers_procedural() {
    let (_, n) = normalized_fixture("diagrammatic");
    let v = &n.principles.visual;
    assert_eq!(v.medium, Medium::Diagrammatic);
    assert_eq!(v.usage, Some(Usage::Balanced));
    assert_eq!(v.balance, Some(Balance::VisualDominant));
    assert_eq!(v.character, Some(Character::Diagrammatic));
    assert_eq!(v.roles, vec![AssetRole::SupportingObject]);
    assert_eq!(
        v.preferred,
        vec![Grammar::SpatialCauseEffect, Grammar::SequentialStack]
    );
    assert_eq!(
        v.secondary,
        vec![Grammar::SplitContrast, Grammar::DataStory]
    );
    assert_eq!(v.avoid, vec![Grammar::KineticPoster]);
    assert_eq!(v.explanation, Some(Explanation::Diagrammatic));
    assert_eq!(v.weight(), Weight::Visual);
    assert!(v.prefers_procedural());
}

#[test]
fn mixed_example_follows_the_mixed_rules() {
    let (_, n) = normalized_fixture("mixed");
    let v = &n.principles.visual;
    assert_eq!(v.medium, Medium::Mixed);
    assert_eq!(v.usage, Some(Usage::Balanced));
    assert_eq!(v.balance, Some(Balance::Balanced));
    assert_eq!(v.character, Some(Character::Mixed));
    // The 0.4-confidence role is recorded, not applied.
    assert_eq!(
        v.roles,
        vec![AssetRole::HeroObject, AssetRole::SupportingObject]
    );
    assert_eq!(v.preferred, vec![Grammar::EditorialCollage]);
    assert_eq!(
        v.secondary,
        vec![Grammar::HeroObject, Grammar::EvidenceStack]
    );
    assert!(v.avoid.is_empty());
    assert_eq!(v.explanation, Some(Explanation::Mixed));
    // mixed medium + balanced usage -> Visual.
    assert_eq!(v.weight(), Weight::Visual);
    assert_eq!(row(&n, "visual.asset_roles").fidelity, Fidelity::Exact);
}

#[test]
fn v0_2_profile_round_trips_through_serde() {
    for name in ["type_led", "image_led", "diagrammatic", "mixed"] {
        let (p, _) = normalized_fixture(name);
        let text = serde_json::to_string(&p).expect("ser");
        let again = parse_profile(&text, None).expect("re-parse").profile;
        assert_eq!(p, again, "{name}");
    }
}

#[test]
fn profile_without_visual_language_normalizes_neutral() {
    let n = normalize(&parse_ok(&json!({ "version": "0.2" })).profile);
    assert_eq!(n.principles.visual, VisualLanguage::neutral());
    for d in [
        "visual.medium",
        "visual.asset_usage",
        "visual.type_image_balance",
        "visual.asset_character",
        "visual.explanation_mode",
        "visual.asset_roles",
        "visual.composition_language",
    ] {
        assert_eq!(row(&n, d).fidelity, Fidelity::Unknown, "{d}");
    }
}

// ---------------------------------------------------------------------------
// Unknown / low confidence
// ---------------------------------------------------------------------------

#[test]
fn unknown_and_null_values_stay_unknown() {
    let doc = vl(json!({
        "medium": { "value": "unclear", "confidence": 0.9 },
        "asset_usage": { "value": null, "confidence": 0.9 },
        "type_image_balance": { "value": "N/A", "confidence": 0.9 },
        "asset_character": { "confidence": 0.9, "value": "not visible" },
        "explanation_mode": { "value": "", "confidence": 0.9 }
    }));
    let parsed = parse_ok(&doc);
    let n = normalize(&parsed.profile);
    assert_eq!(n.principles.visual, VisualLanguage::neutral());
    for d in [
        "visual.medium",
        "visual.asset_usage",
        "visual.type_image_balance",
        "visual.asset_character",
        "visual.explanation_mode",
    ] {
        assert_eq!(row(&n, d).fidelity, Fidelity::Unknown, "{d}");
    }
}

#[test]
fn confidence_below_half_is_recorded_but_not_applied() {
    let doc = vl(json!({
        "medium": { "value": "image_led", "confidence": 0.4 },
        "asset_usage": { "value": "dense", "confidence": 0.4 },
        "asset_roles": [{ "role": "portrait", "confidence": 0.4 }]
    }));
    let n = normalize(&parse_ok(&doc).profile);
    assert_eq!(n.principles.visual, VisualLanguage::neutral());
    assert_eq!(row(&n, "visual.medium").fidelity, Fidelity::LowConfidence);
    assert_eq!(
        row(&n, "visual.medium").reference.as_deref(),
        Some("image_led")
    );
    assert_eq!(
        row(&n, "visual.asset_usage").fidelity,
        Fidelity::LowConfidence
    );
    assert_eq!(
        row(&n, "visual.asset_roles").fidelity,
        Fidelity::LowConfidence
    );
}

#[test]
fn low_confidence_composition_language_leaves_lists_empty() {
    let doc = vl(json!({
        "composition_language": {
            "preferred": ["kinetic_poster"],
            "secondary": ["split_contrast"],
            "avoid": ["data_story"],
            "confidence": 0.4
        }
    }));
    let n = normalize(&parse_ok(&doc).profile);
    let v = &n.principles.visual;
    assert!(v.preferred.is_empty() && v.secondary.is_empty() && v.avoid.is_empty());
    assert_eq!(
        row(&n, "visual.composition_language").fidelity,
        Fidelity::LowConfidence
    );
}

// ---------------------------------------------------------------------------
// Aliases
// ---------------------------------------------------------------------------

fn note<'a>(
    aliases: &'a [motion_core::reference::normalize::AliasNote],
    path: &str,
) -> &'a motion_core::reference::normalize::AliasNote {
    aliases
        .iter()
        .find(|a| a.path == path)
        .unwrap_or_else(|| panic!("no alias note at {path}: {aliases:?}"))
}

#[test]
fn visual_aliases_are_folded_and_reported() {
    let doc = vl(json!({
        "medium": { "value": "Diagram", "confidence": 0.9 },
        "asset_usage": { "value": "heavy", "confidence": 0.9 },
        "type_image_balance": { "value": "Image Dominant", "confidence": 0.9 },
        "asset_character": { "value": "photo", "confidence": 0.9 },
        "explanation_mode": { "value": "Proof", "confidence": 0.9 },
        "asset_roles": [{ "role": "Hero Subject", "confidence": 0.8 }],
        "composition_language": {
            "preferred": ["Kinetic Poster", "unknown"],
            "secondary": ["split-contrast"],
            "confidence": 0.8
        }
    }));
    let parsed = parse_ok(&doc);
    let a = &parsed.aliases;
    let to = |path: &str| note(a, path).to.clone();
    assert_eq!(
        to("visual_language.medium.value").as_deref(),
        Some("diagrammatic")
    );
    assert_eq!(note(a, "visual_language.medium.value").from, "Diagram");
    assert_eq!(
        to("visual_language.asset_usage.value").as_deref(),
        Some("dense")
    );
    assert_eq!(
        to("visual_language.type_image_balance.value").as_deref(),
        Some("visual_dominant")
    );
    assert_eq!(
        to("visual_language.asset_character.value").as_deref(),
        Some("photographic")
    );
    assert_eq!(
        to("visual_language.explanation_mode.value").as_deref(),
        Some("evidence_based")
    );
    assert_eq!(
        to("visual_language.asset_roles[0].role").as_deref(),
        Some("hero_subject")
    );
    assert_eq!(
        to("visual_language.composition_language.preferred[0]").as_deref(),
        Some("kinetic_poster")
    );
    // The unknown list word is dropped and reported with `to: None`.
    let dropped = note(a, "visual_language.composition_language.preferred[1]");
    assert_eq!(dropped.from, "unknown");
    assert_eq!(dropped.to, None);
    assert_eq!(
        to("visual_language.composition_language.secondary[0]").as_deref(),
        Some("split_contrast")
    );

    let v = normalize(&parsed.profile).principles.visual;
    assert_eq!(v.medium, Medium::Diagrammatic);
    assert_eq!(v.usage, Some(Usage::Dense));
    assert_eq!(v.balance, Some(Balance::VisualDominant));
    assert_eq!(v.character, Some(Character::Photographic));
    assert_eq!(v.explanation, Some(Explanation::EvidenceBased));
    assert_eq!(v.roles, vec![AssetRole::HeroSubject]);
    assert_eq!(v.preferred, vec![Grammar::KineticPoster]);
    assert_eq!(v.secondary, vec![Grammar::SplitContrast]);
}

#[test]
fn closed_visual_alias_table() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "medium",
            &[
                ("text_only", "type_only"),
                ("typography_led", "type_led"),
                ("type", "type_led"),
                ("image", "image_led"),
                ("images", "image_led"),
                ("photo_led", "image_led"),
                ("illustration_led", "image_led"),
                ("objects", "object_led"),
                ("object", "object_led"),
                ("diagram", "diagrammatic"),
                ("diagrams", "diagrammatic"),
                ("schematic", "diagrammatic"),
                ("ui", "interface_led"),
                ("interface", "interface_led"),
                ("screen_led", "interface_led"),
            ],
        ),
        (
            "asset_usage",
            &[
                ("heavy", "dense"),
                ("high", "dense"),
                ("frequent", "dense"),
                ("light", "sparse"),
                ("low", "sparse"),
                ("minimal", "sparse"),
                ("rare", "sparse"),
                ("medium", "balanced"),
                ("moderate", "balanced"),
            ],
        ),
        (
            "type_image_balance",
            &[
                ("type", "type_dominant"),
                ("text_dominant", "type_dominant"),
                ("typography_dominant", "type_dominant"),
                ("image_dominant", "visual_dominant"),
                ("visual", "visual_dominant"),
                ("images_dominant", "visual_dominant"),
                ("even", "balanced"),
            ],
        ),
        (
            "asset_character",
            &[
                ("photo", "photographic"),
                ("photography", "photographic"),
                ("photos", "photographic"),
                ("illustration", "illustrative"),
                ("illustrated", "illustrative"),
                ("drawn", "illustrative"),
                ("ui", "interface"),
                ("screens", "interface"),
                ("screen", "interface"),
                ("objects", "object_centric"),
                ("object", "object_centric"),
                ("diagram", "diagrammatic"),
                ("schematic", "diagrammatic"),
                ("shapes", "procedural"),
                ("graphic", "procedural"),
                ("cut_out", "cutout"),
                ("cutouts", "cutout"),
            ],
        ),
        (
            "explanation_mode",
            &[
                ("evidence", "evidence_based"),
                ("proof", "evidence_based"),
                ("metaphor", "metaphorical"),
                ("diagram", "diagrammatic"),
                ("schematic", "diagrammatic"),
                ("icons", "symbolic"),
                ("abstract", "symbolic"),
            ],
        ),
    ];
    for (field, pairs) in cases {
        for (from, want) in *pairs {
            let doc = vl(json!({ *field: { "value": from, "confidence": 0.9 } }));
            let parsed = parse_ok(&doc);
            let v = serde_json::to_value(&parsed.profile).expect("ser");
            assert_eq!(
                v["visual_language"][*field]["value"], *want,
                "{field}: {from} -> {want}"
            );
        }
    }
}

#[test]
fn aliases_do_not_leak_into_other_dimensions() {
    // "heavy" is a visual_language alias only; elsewhere it still fails the strict parse.
    let doc =
        json!({ "version": "0.2", "visual_density": { "value": "heavy", "confidence": 0.9 } });
    assert!(parse(&doc).is_err());
}

// ---------------------------------------------------------------------------
// Rule violations
// ---------------------------------------------------------------------------

#[test]
fn five_roles_are_rejected() {
    let roles: Vec<Value> = [
        "hero_subject",
        "hero_object",
        "supporting_object",
        "environment",
        "portrait",
    ]
    .iter()
    .map(|r| json!({ "role": r, "confidence": 0.8 }))
    .collect();
    let paths = issue_paths(&vl(json!({ "asset_roles": roles })));
    assert_eq!(paths, vec!["visual_language.asset_roles".to_string()]);
}

#[test]
fn duplicate_role_is_rejected() {
    let paths = issue_paths(&vl(json!({
        "asset_roles": [
            { "role": "portrait", "confidence": 0.8 },
            { "role": "portrait", "confidence": 0.6 }
        ]
    })));
    assert_eq!(
        paths,
        vec!["visual_language.asset_roles[1].role".to_string()]
    );
}

#[test]
fn four_entries_in_a_composition_list_are_rejected() {
    let paths = issue_paths(&vl(json!({
        "composition_language": {
            "preferred": ["kinetic_poster", "editorial_collage", "split_contrast", "data_story"],
            "confidence": 0.8
        }
    })));
    assert_eq!(
        paths,
        vec!["visual_language.composition_language".to_string()]
    );
}

#[test]
fn duplicate_within_a_composition_list_is_rejected() {
    let paths = issue_paths(&vl(json!({
        "composition_language": { "avoid": ["data_story", "data_story"], "confidence": 0.8 }
    })));
    assert_eq!(
        paths,
        vec!["visual_language.composition_language".to_string()]
    );
}

#[test]
fn overlapping_composition_lists_are_rejected() {
    for body in [
        json!({ "preferred": ["kinetic_poster"], "secondary": ["kinetic_poster"], "confidence": 0.8 }),
        json!({ "preferred": ["kinetic_poster"], "avoid": ["kinetic_poster"], "confidence": 0.8 }),
        json!({ "secondary": ["data_story"], "avoid": ["data_story"], "confidence": 0.8 }),
    ] {
        let paths = issue_paths(&vl(json!({ "composition_language": body })));
        assert_eq!(
            paths,
            vec!["visual_language.composition_language".to_string()],
            "{body}"
        );
    }
}

#[test]
fn confidence_out_of_range_is_rejected_with_its_path() {
    let cases = [
        (
            json!({ "medium": { "value": "image_led", "confidence": 1.3 } }),
            "visual_language.medium.confidence",
        ),
        (
            json!({ "asset_usage": { "value": "dense", "confidence": -0.1 } }),
            "visual_language.asset_usage.confidence",
        ),
        (
            json!({ "type_image_balance": { "value": "balanced", "confidence": 2 } }),
            "visual_language.type_image_balance.confidence",
        ),
        (
            json!({ "asset_character": { "value": "cutout", "confidence": 1.01 } }),
            "visual_language.asset_character.confidence",
        ),
        (
            json!({ "explanation_mode": { "value": "literal", "confidence": 5 } }),
            "visual_language.explanation_mode.confidence",
        ),
        (
            json!({ "asset_roles": [{ "role": "portrait", "confidence": 1.3 }] }),
            "visual_language.asset_roles[0].confidence",
        ),
        (
            json!({ "composition_language": { "preferred": ["data_story"], "confidence": 1.3 } }),
            "visual_language.composition_language.confidence",
        ),
    ];
    for (body, path) in cases {
        assert_eq!(
            issue_paths(&vl(body.clone())),
            vec![path.to_string()],
            "{body}"
        );
    }
}

#[test]
fn visual_language_evidence_links_are_checked_like_other_dimensions() {
    let cases = [
        (
            json!({ "medium": { "value": "image_led", "confidence": 0.9,
                    "evidence": { "samples": ["the caption"] } } }),
            "visual_language.medium.evidence.samples",
        ),
        (
            json!({ "asset_usage": { "value": "dense", "confidence": 0.9,
                    "evidence": { "metrics": ["audio.bpm"] } } }),
            "visual_language.asset_usage.evidence.metrics",
        ),
        (
            json!({ "composition_language": { "preferred": ["data_story"], "confidence": 0.9,
                    "evidence": { "time_ranges": [[3, 1]] } } }),
            "visual_language.composition_language.evidence.time_ranges",
        ),
    ];
    for (body, path) in cases {
        assert_eq!(
            issue_paths(&vl(body.clone())),
            vec![path.to_string()],
            "{body}"
        );
    }
}

#[test]
fn unknown_visual_language_fields_and_values_are_rejected() {
    for body in [
        json!({ "layout": "centered" }),
        json!({ "medium": { "value": "hologram", "confidence": 0.9 } }),
        json!({ "medium": { "value": "image_led", "confidence": 0.9, "note": "x" } }),
        json!({ "asset_roles": [{ "role": "logo", "confidence": 0.9 }] }),
        json!({ "composition_language": { "preferred": ["grid_wall"], "confidence": 0.9 } }),
        json!({ "composition_language": { "preferred": ["data_story"] } }),
    ] {
        assert!(parse(&vl(body.clone())).is_err(), "accepted {body}");
    }
}

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

#[test]
fn response_schema_is_v0_2_and_contains_visual_language() {
    let schema = response_schema();
    assert!(schema["properties"]["visual_language"].is_object());
    assert!(schema["$defs"]["VisualLanguageProfile"].is_object());
    assert!(schema["$defs"]["CompositionLanguage"].is_object());
    let on_disk: Value = serde_json::from_str(
        &std::fs::read_to_string(root().join("schema/reference-style-profile-v0.2.schema.json"))
            .expect("v0.2 schema"),
    )
    .expect("json");
    assert_eq!(schema, on_disk);
}

#[test]
fn v0_1_schema_file_is_frozen_and_has_no_visual_language() {
    let text =
        std::fs::read_to_string(root().join("schema/reference-style-profile-v0.1.schema.json"))
            .expect("v0.1 schema");
    assert!(!text.contains("visual_language"));
    assert!(!text.contains("VisualLanguageProfile"));
    let on_disk: Value = serde_json::from_str(&text).expect("json");
    let frozen = serde_json::to_value(schemars::schema_for!(
        motion_core::reference::profile_v0_1::ReferenceStyleProfile
    ))
    .expect("schema");
    assert_eq!(on_disk, frozen);
}

/// Walk everything reachable from `VisualLanguageProfile` and fail on any plain
/// string leaf (enum/const only). `EvidenceLinks` (sample ids, metric keys) is
/// the shared provenance type used by every dimension and is excluded.
fn assert_no_free_text(v: &Value, defs: &Value, seen: &mut Vec<String>, at: &str) {
    match v {
        Value::Object(o) => {
            if let Some(Value::String(r)) = o.get("$ref") {
                let name = r.rsplit('/').next().unwrap_or_default().to_string();
                if name == "EvidenceLinks" || seen.contains(&name) {
                    return;
                }
                seen.push(name.clone());
                assert_no_free_text(&defs[&name], defs, seen, &name);
                return;
            }
            let is_string = match o.get("type") {
                Some(Value::String(t)) => t == "string",
                Some(Value::Array(ts)) => ts.iter().any(|t| t == "string"),
                _ => false,
            };
            if is_string {
                assert!(
                    o.contains_key("const") || o.contains_key("enum"),
                    "free-text string leaf at {at}: {v}"
                );
            }
            for (k, child) in o {
                if k != "description" && k != "title" {
                    assert_no_free_text(child, defs, seen, at);
                }
            }
        }
        Value::Array(a) => {
            for child in a {
                assert_no_free_text(child, defs, seen, at);
            }
        }
        _ => {}
    }
}

#[test]
fn visual_language_schema_has_no_free_text_leaves() {
    let schema = response_schema();
    let defs = &schema["$defs"];
    let mut seen = vec!["VisualLanguageProfile".to_string()];
    assert_no_free_text(
        &defs["VisualLanguageProfile"],
        defs,
        &mut seen,
        "VisualLanguageProfile",
    );
    // The walk reached the closed vocabularies.
    for want in [
        "VisualMedium",
        "RefAssetRole",
        "RefGrammar",
        "CompositionLanguage",
    ] {
        assert!(
            seen.iter().any(|s| s == want),
            "{want} not reached: {seen:?}"
        );
    }
}
