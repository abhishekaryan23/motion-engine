//! ReferenceStyleProfile v0.1 contract (0.7): parsing, aliases, validation,
//! normalization, coverage and the interpreter bundle.
//!
//! Assertions are structural (statuses, fidelities, membership), never exact
//! design values. The reference-to-TasteDirector precedence and propagation
//! tests live in `reference_taste.rs`.

use std::path::PathBuf;

use motion_core::compiler::taste::{
    resolve_with, Activity, BackgroundGrammar, MaterialFinish, ReferencePrinciples,
    ResolvedPolarity, ResolvedTemperature, ResolvedTone, TemperamentKind, TransitionFamily,
    TypographyPairing,
};
use motion_core::reference::bundle::{
    interpreter_prompt, interpreter_request, repair_request, response_schema, vocabulary,
    INSTRUCTIONS, MAX_REPAIR_ATTEMPTS, PROTOCOL,
};
use motion_core::reference::coverage::CoverageStatus;
use motion_core::reference::evidence::{
    AnalysisSpec, ChangeKind, ChangeSummary, ColorEvidence, ComplexityEvidence, Distribution,
    DurationTendency, EvidencePolarity, EvidenceTemperature, Orientation, PolarityEstimate,
    ReferenceEvidence, ReferenceMetadata, ReferenceSample, SampleColor, SampleKind, Swatch,
    SwatchRole, TemperatureEstimate, TemporalEvidence,
};
use motion_core::reference::normalize::{normalize_aliases, Fidelity, NormalizedReference};
use motion_core::reference::oklab;
use motion_core::reference::profile::*;
use motion_core::reference::{coverage, normalize, parse_profile, validate_profile};
use motion_core::style::{Polarity, StyleProfile, Tone};
use serde_json::{json, Value};

const FINGERPRINT: &str = "rf1-0123456789abcdef";

/// Style rows (`DIMENSIONS` + accent_hint) are followed by the 7 visual-language rows (0.7.1).
const VISUAL_ROWS: &[&str] = &[
    "visual.medium",
    "visual.asset_usage",
    "visual.type_image_balance",
    "visual.asset_character",
    "visual.explanation_mode",
    "visual.asset_roles",
    "visual.composition_language",
];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn dist(v: f32) -> Distribution {
    Distribution {
        mean: v,
        p10: v,
        p50: v,
        p90: v,
    }
}

/// A minimal hand-made analysis: 3 samples s01..s03, 10 s long.
fn evidence() -> ReferenceEvidence {
    let sample = |n: u32, t: f64| ReferenceSample {
        id: format!("s{n:02}"),
        time_seconds: t,
        frame: (t * 30.0).round() as u64,
        kinds: vec![SampleKind::Periodic],
        image: format!("samples/s{n:02}.jpg"),
    };
    let swatch = |hex: &str, prevalence: f32, role: SwatchRole| Swatch {
        hex: hex.to_string(),
        prevalence,
        lightness: 0.5,
        chroma: 0.1,
        hue_degrees: Some(200.0),
        role,
    };
    ReferenceEvidence {
        version: "0.1".into(),
        analyzer_version: "0.7.0".into(),
        reference_fingerprint: FINGERPRINT.into(),
        media_fingerprint: "fnv1a64-0123456789abcdef".into(),
        metadata: ReferenceMetadata {
            width: 1080,
            height: 1920,
            duration_seconds: 10.0,
            fps: 30.0,
            frame_count: 300,
            aspect_ratio: 1080.0 / 1920.0,
            orientation: Orientation::Vertical,
            video_codec: "h264".into(),
            audio_present: true,
        },
        analysis: AnalysisSpec {
            width: 270,
            height: 480,
            fps: 4.0,
            frames: 40,
            decoder: "test".into(),
        },
        samples: vec![sample(1, 0.5), sample(2, 5.0), sample(3, 9.5)],
        color: ColorEvidence {
            polarity: PolarityEstimate {
                value: EvidencePolarity::Dark,
                confidence: 0.9,
            },
            dark_frame_fraction: 0.8,
            light_frame_fraction: 0.1,
            luminance: dist(0.2),
            contrast: dist(0.6),
            saturation: dist(0.3),
            temperature: TemperatureEstimate {
                value: EvidenceTemperature::Cool,
                confidence: 0.8,
            },
            warm_cool_balance: -0.4,
            palette: vec![
                swatch("#0D1117", 0.6, SwatchRole::Ground),
                swatch("#E6EDF3", 0.2, SwatchRole::Ink),
                swatch("#3CC8F0", 0.1, SwatchRole::Accent),
            ],
            accent_prevalence: 0.1,
            palette_stability: 0.9,
            per_sample: vec![SampleColor {
                id: "s01".into(),
                mean_luma: 0.2,
                mean_saturation: 0.3,
                dominant: "#0D1117".into(),
            }],
        },
        temporal: TemporalEvidence {
            noise_floor: 0.0,
            changes: vec![ChangeSummary {
                time_seconds: 4.0,
                duration_seconds: 0.3,
                kind: ChangeKind::Transition,
                magnitude: 0.4,
            }],
            cuts_per_10s: 1.0,
            changes_per_10s: 3.0,
            median_hold_seconds: 2.0,
            hold_p90_seconds: 4.0,
            static_fraction: 0.5,
            low_motion_fraction: 0.7,
            high_motion_fraction: 0.05,
            activity: dist(0.05),
            activity_variance: 0.5,
            transition_spike_ratio: 3.0,
            median_transition_seconds: 0.3,
            transition_duration: DurationTendency::Medium,
            hard_cut_fraction: 0.2,
            background_activity: 0.02,
            foreground_activity: 0.05,
        },
        complexity: ComplexityEvidence {
            edge_density: dist(0.1),
            flat_area_fraction: dist(0.6),
            large_region_count: dist(3.0),
            entropy: dist(0.5),
            occupied_ratio: dist(0.3),
        },
    }
}

/// A complete valid profile document: all 16 dimensions, palette evidence,
/// unsupported traits, provenance.
fn full_profile_json() -> Value {
    json!({
        "version": "0.1",
        "reference_fingerprint": FINGERPRINT,
        "tone": {
            "value": "technical", "confidence": 0.9,
            "evidence": { "samples": ["s01", "s02"], "metrics": ["color.polarity"], "time_ranges": [[0.0, 4.0]] }
        },
        "polarity": { "value": "dark", "confidence": 0.95 },
        "temperature": { "value": "cool", "confidence": 0.8 },
        "contrast": { "value": "high", "confidence": 0.8 },
        "palette_character": { "value": "restrained_accent", "confidence": 0.7 },
        "approximate_palette_evidence": [
            { "hex": "#0D1117", "prevalence": 0.6 },
            { "hex": "#E6EDF3", "prevalence": 0.2 },
            { "hex": "#3CC8F0", "prevalence": 0.1 }
        ],
        "background_character": { "value": "grid", "confidence": 0.85 },
        "typography_character": { "value": "mono_technical", "confidence": 0.75 },
        "typography_contrast": { "value": "medium", "confidence": 0.6 },
        "material_character": { "value": "screen", "confidence": 0.7 },
        "image_treatment": { "value": "none", "confidence": 0.9 },
        "visual_density": { "value": "balanced", "confidence": 0.6 },
        "composition_rhythm": { "value": "active", "confidence": 0.65, "evidence": { "metrics": ["temporal.changes_per_10s"] } },
        "motion_temperament": { "value": "snappy", "confidence": 0.8 },
        "transition_character": { "value": "geometric", "confidence": 0.7 },
        "scale_contrast": { "value": "moderate", "confidence": 0.6 },
        "layer_activity": {
            "value": { "foreground": "balanced", "midground": "structured", "background": "structured" },
            "confidence": 0.6
        },
        "unsupported_reference_traits": [
            { "trait": "glitch", "confidence": 0.8, "evidence": { "samples": ["s03"] } },
            { "trait": "particle_field", "confidence": 0.6 }
        ]
    })
}

fn parse(
    doc: &Value,
) -> Result<motion_core::reference::ParsedProfile, Vec<motion_core::reference::ProfileIssue>> {
    parse_profile(&doc.to_string(), None)
}

fn parse_ok(doc: &Value) -> ReferenceStyleProfile {
    parse(doc)
        .unwrap_or_else(|e| panic!("expected valid profile, got {e:?}"))
        .profile
}

fn issues_of(doc: &Value, ev: Option<&ReferenceEvidence>) -> Vec<(String, String)> {
    parse_profile(&doc.to_string(), ev)
        .expect_err("expected rejection")
        .into_iter()
        .map(|i| (i.path, i.message))
        .collect()
}

fn assert_issue_at(doc: &Value, ev: Option<&ReferenceEvidence>, path: &str) {
    let issues = issues_of(doc, ev);
    assert!(
        issues.iter().any(|(p, _)| p == path),
        "no issue at {path:?}; got {issues:?}"
    );
}

fn with(mut doc: Value, key: &str, v: Value) -> Value {
    doc[key] = v;
    doc
}

/// Minimal valid profile (version only), for builder-style edits.
fn bare() -> ReferenceStyleProfile {
    ReferenceStyleProfile {
        version: "0.1".into(),
        ..Default::default()
    }
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

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

// ---------------------------------------------------------------------------
// parse_profile
// ---------------------------------------------------------------------------

#[test]
fn a_complete_valid_profile_parses_and_covers_every_dimension() {
    let doc = full_profile_json();
    let parsed = parse(&doc).expect("valid");
    assert!(parsed.aliases.is_empty(), "canonical words need no aliases");
    let p = &parsed.profile;
    assert_eq!(p.version, "0.1");
    assert_eq!(p.reference_fingerprint.as_deref(), Some(FINGERPRINT));
    for (name, present) in [
        ("tone", p.tone.is_some()),
        ("polarity", p.polarity.is_some()),
        ("temperature", p.temperature.is_some()),
        ("contrast", p.contrast.is_some()),
        ("palette_character", p.palette_character.is_some()),
        ("background_character", p.background_character.is_some()),
        ("typography_character", p.typography_character.is_some()),
        ("typography_contrast", p.typography_contrast.is_some()),
        ("material_character", p.material_character.is_some()),
        ("image_treatment", p.image_treatment.is_some()),
        ("visual_density", p.visual_density.is_some()),
        ("composition_rhythm", p.composition_rhythm.is_some()),
        ("motion_temperament", p.motion_temperament.is_some()),
        ("transition_character", p.transition_character.is_some()),
        ("scale_contrast", p.scale_contrast.is_some()),
        ("layer_activity", p.layer_activity.is_some()),
    ] {
        assert!(present, "{name} missing after parse");
    }
    assert_eq!(DIMENSIONS.len(), 16);
    assert_eq!(p.approximate_palette_evidence.len(), 3);
    assert_eq!(p.unsupported_reference_traits.len(), 2);
    assert_eq!(
        p.tone.as_ref().unwrap().evidence.as_ref().unwrap().samples,
        ["s01", "s02"]
    );
    // Provenance checks against the matching evidence also pass.
    assert!(parse_profile(&doc.to_string(), Some(&evidence())).is_ok());
    assert!(validate_profile(p, Some(&evidence())).is_empty());
}

#[test]
fn a_valid_profile_round_trips_through_serde() {
    let p = parse_ok(&full_profile_json());
    let text = serde_json::to_string(&p).expect("serializes");
    let again = parse_profile(&text, None).expect("re-parses").profile;
    assert_eq!(p, again);
    // Serialization is stable: serialize(parse(serialize(p))) == serialize(p).
    assert_eq!(text, serde_json::to_string(&again).unwrap());
    // Unknown dimensions are omitted, not written as null.
    let minimal = serde_json::to_value(bare()).unwrap();
    assert_eq!(minimal, json!({ "version": "0.1" }));
}

#[test]
fn version_only_profile_is_valid_and_all_unknown() {
    let p = parse_ok(&json!({ "version": "0.1" }));
    assert_eq!(p, bare());
}

// ---------------------------------------------------------------------------
// Aliases
// ---------------------------------------------------------------------------

#[test]
fn aliases_fold_spelling_variants_and_report_paths() {
    let doc = json!({
        "version": "0.1",
        "polarity": { "value": "very dark", "confidence": 0.9 },
        "temperature": { "value": "unknown", "confidence": 0.9 },
        "palette_character": { "value": "grayscale", "confidence": 0.9 },
        "transition_character": { "value": "hard cuts", "confidence": 0.9 },
        "background_character": { "value": "", "confidence": 0.9 }
    });
    let parsed = parse(&doc).expect("aliases resolve");
    let p = &parsed.profile;
    assert_eq!(p.polarity.as_ref().unwrap().value, Some(RefPolarity::Dark));
    assert_eq!(p.temperature.as_ref().unwrap().value, None);
    assert_eq!(
        p.palette_character.as_ref().unwrap().value,
        Some(PaletteCharacter::Monochrome)
    );
    assert_eq!(
        p.transition_character.as_ref().unwrap().value,
        Some(RefTransition::Hard)
    );
    assert_eq!(p.background_character.as_ref().unwrap().value, None);

    let note = |path: &str| {
        parsed
            .aliases
            .iter()
            .find(|n| n.path == path)
            .unwrap_or_else(|| panic!("no alias note for {path}: {:?}", parsed.aliases))
    };
    assert_eq!(note("polarity.value").from, "very dark");
    assert_eq!(note("polarity.value").to.as_deref(), Some("dark"));
    assert_eq!(note("temperature.value").to, None);
    assert_eq!(
        note("palette_character.value").to.as_deref(),
        Some("monochrome")
    );
    assert_eq!(
        note("transition_character.value").to.as_deref(),
        Some("hard")
    );
    assert_eq!(note("background_character.value").to, None);
    assert_eq!(parsed.aliases.len(), 5);
}

#[test]
fn mostly_black_and_case_variants_fold_to_dark() {
    for word in ["Mostly-Black", "MOSTLY BLACK", "  very_dark ", "Dark Mode"] {
        let doc = json!({ "version": "0.1", "polarity": { "value": word, "confidence": 0.9 } });
        let p = parse_ok(&doc);
        assert_eq!(
            p.polarity.unwrap().value,
            Some(RefPolarity::Dark),
            "{word:?}"
        );
    }
}

#[test]
fn layer_activity_parts_are_aliased_independently() {
    let doc = json!({
        "version": "0.1",
        "layer_activity": {
            "value": { "foreground": "balanced", "midground": "unknown", "background": "Still" },
            "confidence": 0.8
        }
    });
    let parsed = parse(&doc).expect("valid");
    let la = parsed.profile.layer_activity.unwrap().value.unwrap();
    assert_eq!(la.foreground, Some(RefPresence::Balanced));
    assert_eq!(la.midground, None);
    assert_eq!(la.background, Some(RefActivity::Still));
    assert!(parsed
        .aliases
        .iter()
        .any(|n| n.path == "layer_activity.value.midground"));
}

#[test]
fn unrecognised_words_are_not_rewritten_and_fail_strict_parsing() {
    // The alias stage leaves the value alone.
    let mut doc = json!({
        "version": "0.1",
        "background_character": { "value": "neon_bloom", "confidence": 0.9 }
    });
    let notes = normalize_aliases(&mut doc);
    assert!(notes.is_empty(), "no rewrite expected: {notes:?}");
    assert_eq!(doc["background_character"]["value"], "neon_bloom");

    // The strict parse rejects it.
    let doc = json!({
        "version": "0.1",
        "background_character": { "value": "neon_bloom", "confidence": 0.9 }
    });
    let issues = issues_of(&doc, None);
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].0, "$");
    assert!(
        issues[0].1.contains("neon_bloom"),
        "message names the word: {}",
        issues[0].1
    );
    // A folded spelling of an unknown word is still unknown, never guessed.
    let doc = json!({
        "version": "0.1",
        "background_character": { "value": "Neon Bloom", "confidence": 0.9 }
    });
    assert_eq!(issues_of(&doc, None)[0].0, "$");
}

// ---------------------------------------------------------------------------
// Rejections
// ---------------------------------------------------------------------------

#[test]
fn rejects_unknown_top_level_field() {
    let doc = with(
        full_profile_json(),
        "story_summary",
        json!("a video about cats"),
    );
    let issues = issues_of(&doc, None);
    assert_eq!(issues[0].0, "$");
    assert!(issues[0].1.contains("story_summary"), "{issues:?}");
}

#[test]
fn rejects_unknown_field_inside_a_trait() {
    let doc = with(
        json!({ "version": "0.1" }),
        "tone",
        json!({ "value": "editorial", "confidence": 0.9, "why": "looks like a magazine" }),
    );
    let issues = issues_of(&doc, None);
    assert_eq!(issues[0].0, "$");
    assert!(issues[0].1.contains("why"), "{issues:?}");
}

#[test]
fn rejects_bare_string_instead_of_a_trait_object() {
    let doc = with(json!({ "version": "0.1" }), "tone", json!("editorial"));
    assert_issue_at(&doc, None, "$");
}

#[test]
fn rejects_confidence_outside_zero_one() {
    for bad in [1.4, -0.1, 2.0] {
        let doc = with(
            json!({ "version": "0.1" }),
            "polarity",
            json!({ "value": "dark", "confidence": bad }),
        );
        assert_issue_at(&doc, None, "polarity.confidence");
    }
    // Boundaries are accepted.
    for ok in [0.0, 1.0] {
        let doc = with(
            json!({ "version": "0.1" }),
            "polarity",
            json!({ "value": "dark", "confidence": ok }),
        );
        parse_ok(&doc);
    }
}

#[test]
fn rejects_non_finite_confidence() {
    // JSON cannot carry NaN, so the typed validator is the guard.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut p = bare();
        p.tone = Some(Trait::new(RefTone::Editorial, bad));
        p.unsupported_reference_traits.push(UnsupportedTrait {
            kind: UnsupportedTraitKind::Glitch,
            confidence: bad,
            evidence: None,
        });
        let paths: Vec<String> = validate_profile(&p, None)
            .into_iter()
            .map(|i| i.path)
            .collect();
        assert!(paths.contains(&"tone.confidence".to_string()), "{paths:?}");
        assert!(
            paths.contains(&"unsupported_reference_traits[0].confidence".to_string()),
            "{paths:?}"
        );
    }
    // A textual NaN in JSON is a type error, not a silent zero.
    let doc = with(
        json!({ "version": "0.1" }),
        "tone",
        json!({ "value": "editorial", "confidence": "NaN" }),
    );
    assert_issue_at(&doc, None, "$");
}

#[test]
fn rejects_wrong_version() {
    assert_issue_at(&json!({ "version": "0.3" }), None, "version");
    assert_issue_at(&json!({ "version": "" }), None, "version");
}

#[test]
fn rejects_more_than_eight_palette_swatches() {
    let swatches: Vec<Value> = (0..9)
        .map(
            |i| json!({ "hex": format!("#{:02X}{:02X}{:02X}", i * 20, 10, 10), "prevalence": 0.1 }),
        )
        .collect();
    let doc = with(
        json!({ "version": "0.1" }),
        "approximate_palette_evidence",
        Value::Array(swatches.clone()),
    );
    assert_issue_at(&doc, None, "approximate_palette_evidence");
    // Exactly 8 is fine.
    let doc = with(
        json!({ "version": "0.1" }),
        "approximate_palette_evidence",
        Value::Array(swatches[..8].to_vec()),
    );
    parse_ok(&doc);
}

#[test]
fn rejects_bad_hex_and_prevalence() {
    for bad in ["3CC8F0", "#3CC8F", "#GGGGGG", "#3CC8F0FF", "red"] {
        let doc = with(
            json!({ "version": "0.1" }),
            "approximate_palette_evidence",
            json!([{ "hex": "#111111", "prevalence": 0.5 }, { "hex": bad, "prevalence": 0.1 }]),
        );
        assert_issue_at(&doc, None, "approximate_palette_evidence[1].hex");
    }
    let doc = with(
        json!({ "version": "0.1" }),
        "approximate_palette_evidence",
        json!([{ "hex": "#111111", "prevalence": 1.5 }]),
    );
    assert_issue_at(&doc, None, "approximate_palette_evidence[0].prevalence");
}

#[test]
fn rejects_duplicate_unsupported_trait() {
    let doc = with(
        json!({ "version": "0.1" }),
        "unsupported_reference_traits",
        json!([
            { "trait": "glitch", "confidence": 0.9 },
            { "trait": "particle_field", "confidence": 0.5 },
            { "trait": "glitch", "confidence": 0.4 }
        ]),
    );
    assert_issue_at(&doc, None, "unsupported_reference_traits[2].trait");
}

#[test]
fn rejects_unknown_metric_key() {
    let doc = with(
        json!({ "version": "0.1" }),
        "tone",
        json!({ "value": "editorial", "confidence": 0.8, "evidence": { "metrics": ["color.polarity", "temporal.vibes"] } }),
    );
    assert_issue_at(&doc, None, "tone.evidence.metrics");
    // Every documented key is accepted.
    for m in motion_core::reference::evidence::EVIDENCE_METRICS {
        let doc = with(
            json!({ "version": "0.1" }),
            "tone",
            json!({ "value": "editorial", "confidence": 0.8, "evidence": { "metrics": [m] } }),
        );
        parse_ok(&doc);
    }
}

#[test]
fn rejects_malformed_sample_ids_without_evidence() {
    for bad in ["sample1", "s1", "S01", "01", "s0a", ""] {
        let doc = with(
            json!({ "version": "0.1" }),
            "tone",
            json!({ "value": "editorial", "confidence": 0.8, "evidence": { "samples": [bad] } }),
        );
        assert_issue_at(&doc, None, "tone.evidence.samples");
    }
    let doc = with(
        json!({ "version": "0.1" }),
        "tone",
        json!({ "value": "editorial", "confidence": 0.8, "evidence": { "samples": ["s01", "s12"] } }),
    );
    parse_ok(&doc);
}

#[test]
fn rejects_reversed_or_negative_time_ranges_without_evidence() {
    for bad in [json!([[5.0, 2.0]]), json!([[-1.0, 2.0]])] {
        let doc = with(
            json!({ "version": "0.1" }),
            "tone",
            json!({ "value": "editorial", "confidence": 0.8, "evidence": { "time_ranges": bad } }),
        );
        assert_issue_at(&doc, None, "tone.evidence.time_ranges");
    }
}

#[test]
fn rejects_not_json_and_malformed_fingerprint() {
    let issues = parse_profile("{ nope", None).expect_err("not json");
    assert_eq!(issues[0].path, "$");
    let doc = json!({ "version": "0.1", "reference_fingerprint": "rf1-XYZ" });
    assert_issue_at(&doc, None, "reference_fingerprint");
}

#[test]
fn issue_display_includes_path_and_message() {
    let issues = parse_profile(&json!({ "version": "9" }).to_string(), None).unwrap_err();
    let text = issues[0].to_string();
    assert!(text.starts_with("version: "), "{text}");
}

// ---------------------------------------------------------------------------
// With evidence
// ---------------------------------------------------------------------------

fn tone_with_links(links: Value) -> Value {
    json!({
        "version": "0.1",
        "tone": { "value": "editorial", "confidence": 0.8, "evidence": links }
    })
}

#[test]
fn evidence_rejects_unknown_sample_id() {
    let ev = evidence();
    // Well-formed but not in this analysis.
    let doc = tone_with_links(json!({ "samples": ["s01", "s04"] }));
    assert_issue_at(&doc, Some(&ev), "tone.evidence.samples");
    // The same id passes without evidence (only the shape is checked).
    parse_ok(&doc);
    for id in ["s01", "s02", "s03"] {
        parse_profile(
            &tone_with_links(json!({ "samples": [id] })).to_string(),
            Some(&ev),
        )
        .unwrap_or_else(|e| panic!("{id}: {e:?}"));
    }
}

#[test]
fn evidence_rejects_time_ranges_beyond_the_duration() {
    let ev = evidence();
    let doc = tone_with_links(json!({ "time_ranges": [[2.0, 10.5]] }));
    assert_issue_at(&doc, Some(&ev), "tone.evidence.time_ranges");
    parse_ok(&doc); // no evidence: no upper bound
    for ok in [
        json!([[0.0, 10.0]]),
        json!([[3.0, 3.0]]),
        json!([[9.0, 10.04]]),
    ] {
        parse_profile(
            &tone_with_links(json!({ "time_ranges": ok })).to_string(),
            Some(&ev),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
    }
}

#[test]
fn evidence_fingerprint_must_match() {
    let ev = evidence();
    let other = "rf1-fedcba9876543210";
    let doc = json!({ "version": "0.1", "reference_fingerprint": other });
    assert_issue_at(&doc, Some(&ev), "reference_fingerprint");
    // Without evidence a well-formed fingerprint is accepted.
    parse_ok(&doc);
    let doc = json!({ "version": "0.1", "reference_fingerprint": FINGERPRINT });
    parse_profile(&doc.to_string(), Some(&ev)).expect("matching fingerprint accepted");
}

// ---------------------------------------------------------------------------
// normalize
// ---------------------------------------------------------------------------

#[test]
fn absent_and_null_dimensions_are_unknown_and_never_become_values() {
    // Absent.
    let n = normalize(&bare());
    assert_eq!(n.principles, ReferencePrinciples::default());
    assert!(n.principles.is_empty());
    assert_eq!(n.dimensions.len(), DIMENSIONS.len() + 1 + VISUAL_ROWS.len());
    assert!(n.dimensions.iter().all(|d| d.fidelity == Fidelity::Unknown));
    assert!(n.unsupported.is_empty());

    // Null value with confidence 1.0: still unknown (property-style: every
    // dimension null).
    let mut p = bare();
    p.tone = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.polarity = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.temperature = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.contrast = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.palette_character = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.background_character = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.typography_character = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.typography_contrast = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.material_character = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.image_treatment = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.visual_density = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.composition_rhythm = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.motion_temperament = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.transition_character = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.scale_contrast = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    p.layer_activity = Some(Trait {
        value: None,
        confidence: 1.0,
        evidence: None,
    });
    let n = normalize(&p);
    assert_eq!(n.principles, ReferencePrinciples::default());
    assert!(n.dimensions.iter().all(|d| d.fidelity == Fidelity::Unknown));
    assert!(n.dimensions.iter().all(|d| d.engine.is_none()));
    // ... and it resolves exactly like no reference at all.
    let style = StyleProfile::default();
    assert_eq!(
        resolve_with(&style, Some(&n.principles), 3),
        resolve_with(&style, None, 3)
    );
}

#[test]
fn null_valued_json_dimension_is_unknown() {
    let doc = json!({
        "version": "0.1",
        "tone": { "value": null, "confidence": 0.9 },
        "polarity": { "value": "unknown", "confidence": 0.9 }
    });
    let n = normalize(&parse_ok(&doc));
    assert_eq!(row(&n, "tone").fidelity, Fidelity::Unknown);
    assert_eq!(row(&n, "polarity").fidelity, Fidelity::Unknown);
    assert!(n.principles.is_empty());
}

#[test]
fn low_confidence_is_recorded_but_not_applied() {
    let mut p = bare();
    p.tone = Some(Trait::new(RefTone::Technical, 0.4));
    p.motion_temperament = Some(Trait::new(RefTemperament::Snappy, 0.49));
    let n = normalize(&p);
    let r = row(&n, "tone");
    assert_eq!(r.fidelity, Fidelity::LowConfidence);
    assert_eq!(r.reference.as_deref(), Some("technical"));
    assert_eq!(r.engine, None);
    assert_eq!(n.principles.tone, None);
    assert_eq!(n.principles.temperament, None);
    assert!(n.principles.is_empty());
    // The threshold itself applies.
    let mut p = bare();
    p.tone = Some(Trait::new(RefTone::Technical, 0.5));
    let n = normalize(&p);
    assert_eq!(n.principles.tone, Some(ResolvedTone::Technical));
    assert_eq!(row(&n, "tone").fidelity, Fidelity::Exact);
}

#[test]
fn mixed_polarity_is_partial_and_sets_no_polarity() {
    let mut p = bare();
    p.polarity = Some(Trait::new(RefPolarity::Mixed, 0.9));
    let n = normalize(&p);
    assert_eq!(n.principles.polarity, None);
    assert_eq!(row(&n, "polarity").fidelity, Fidelity::Partial);
    assert_eq!(row(&n, "polarity").reference.as_deref(), Some("mixed"));
}

#[test]
fn temperature_mapping() {
    let map = |t: RefTemperature| {
        let mut p = bare();
        p.temperature = Some(Trait::new(t, 0.9));
        normalize(&p)
    };
    let n = map(RefTemperature::Neutral);
    assert_eq!(n.principles.temperature, Some(ResolvedTemperature::Cool));
    assert_eq!(row(&n, "temperature").fidelity, Fidelity::Closest);
    let n = map(RefTemperature::Warm);
    assert_eq!(n.principles.temperature, Some(ResolvedTemperature::Warm));
    assert_eq!(row(&n, "temperature").fidelity, Fidelity::Exact);
    let n = map(RefTemperature::Cool);
    assert_eq!(n.principles.temperature, Some(ResolvedTemperature::Cool));
    assert_eq!(row(&n, "temperature").fidelity, Fidelity::Exact);
}

#[test]
fn motion_temperament_mapping() {
    let map = |t: RefTemperament| {
        let mut p = bare();
        p.motion_temperament = Some(Trait::new(t, 0.9));
        let n = normalize(&p);
        (
            n.principles.temperament,
            row(&n, "motion_temperament").fidelity,
        )
    };
    assert_eq!(
        map(RefTemperament::Fluid),
        (Some(TemperamentKind::Editorial), Fidelity::Closest)
    );
    assert_eq!(
        map(RefTemperament::Snappy),
        (Some(TemperamentKind::Precise), Fidelity::Exact)
    );
    assert_eq!(
        map(RefTemperament::Restrained),
        (Some(TemperamentKind::Restrained), Fidelity::Exact)
    );
    assert_eq!(
        map(RefTemperament::Energetic),
        (Some(TemperamentKind::Energetic), Fidelity::Exact)
    );
    // Non-exact spellings are never reported as Exact.
    for t in [
        RefTemperament::Mechanical,
        RefTemperament::Playful,
        RefTemperament::Cinematic,
    ] {
        let (v, f) = map(t);
        assert!(v.is_some());
        assert_eq!(f, Fidelity::Closest, "{t:?}");
    }
}

#[test]
fn transition_mapping() {
    let map = |t: RefTransition| {
        let mut p = bare();
        p.transition_character = Some(Trait::new(t, 0.9));
        let n = normalize(&p);
        (
            n.principles.transition,
            row(&n, "transition_character").fidelity,
        )
    };
    for t in [RefTransition::Continuous, RefTransition::Cinematic] {
        assert_eq!(
            map(t),
            (Some(TransitionFamily::Subtle), Fidelity::Closest),
            "{t:?}"
        );
    }
    assert_eq!(
        map(RefTransition::Hard),
        (Some(TransitionFamily::Hard), Fidelity::Exact)
    );
    assert_eq!(
        map(RefTransition::Geometric),
        (Some(TransitionFamily::Geometric), Fidelity::Exact)
    );
}

#[test]
fn background_mapping_and_implied_unsupported_traits() {
    let map = |b: BackgroundCharacter| {
        let mut p = bare();
        p.background_character = Some(Trait::new(b, 0.9));
        normalize(&p)
    };
    let n = map(BackgroundCharacter::Gradient);
    assert_eq!(n.principles.background, Some(BackgroundGrammar::CleanFlat));
    assert_eq!(row(&n, "background_character").fidelity, Fidelity::Closest);
    assert_eq!(n.unsupported, [UnsupportedTraitKind::GradientBackground]);

    let n = map(BackgroundCharacter::Photographic);
    assert_eq!(n.principles.background, None);
    assert_eq!(
        row(&n, "background_character").fidelity,
        Fidelity::Unsupported
    );
    assert_eq!(
        n.unsupported,
        [UnsupportedTraitKind::PhotographicBackground]
    );

    let n = map(BackgroundCharacter::Grid);
    assert_eq!(
        n.principles.background,
        Some(BackgroundGrammar::TechnicalGrid)
    );
    assert_eq!(row(&n, "background_character").fidelity, Fidelity::Exact);
    assert!(n.unsupported.is_empty());

    let n = map(BackgroundCharacter::GraphicFields);
    assert_eq!(
        n.principles.background,
        Some(BackgroundGrammar::PrintFields)
    );
}

#[test]
fn script_and_handwritten_typography_are_unsupported() {
    for t in [
        TypographyCharacter::Script,
        TypographyCharacter::Handwritten,
    ] {
        let mut p = bare();
        p.typography_character = Some(Trait::new(t, 0.9));
        let n = normalize(&p);
        assert_eq!(n.principles.typography, None, "{t:?}");
        assert_eq!(
            row(&n, "typography_character").fidelity,
            Fidelity::Unsupported
        );
        assert_eq!(n.unsupported, [UnsupportedTraitKind::HandLettering]);
    }
    let mut p = bare();
    p.typography_character = Some(Trait::new(TypographyCharacter::SerifEditorial, 0.9));
    let n = normalize(&p);
    assert_eq!(n.principles.typography, Some(TypographyPairing::SerifSans));
}

#[test]
fn metallic_and_glossy_materials_map_to_flat_and_record_the_trait() {
    for m in [MaterialCharacter::Metallic, MaterialCharacter::Glossy] {
        let mut p = bare();
        p.material_character = Some(Trait::new(m, 0.9));
        let n = normalize(&p);
        assert_eq!(
            n.principles.material,
            Some(MaterialFinish::CleanFlat),
            "{m:?}"
        );
        assert_eq!(row(&n, "material_character").fidelity, Fidelity::Closest);
        assert_eq!(n.unsupported, [UnsupportedTraitKind::ChromeOrMetallic]);
    }
    let mut p = bare();
    p.material_character = Some(Trait::new(MaterialCharacter::Rendered3d, 0.9));
    let n = normalize(&p);
    assert_eq!(n.unsupported, [UnsupportedTraitKind::True3d]);
}

#[test]
fn palette_character_sets_color_fields() {
    let fields = |c: PaletteCharacter| {
        let mut p = bare();
        p.palette_character = Some(Trait::new(c, 0.9));
        normalize(&p).principles.color_fields
    };
    assert_eq!(fields(PaletteCharacter::Multicolor), Some(true));
    assert_eq!(fields(PaletteCharacter::Vivid), Some(false));
    assert_eq!(fields(PaletteCharacter::RestrainedAccent), Some(false));
}

#[test]
fn partial_layer_activity_sets_only_the_given_hints() {
    let mut p = bare();
    p.layer_activity = Some(Trait::new(
        RefLayerActivity {
            foreground: None,
            midground: None,
            background: Some(RefActivity::Structured),
        },
        0.8,
    ));
    let n = normalize(&p);
    assert_eq!(row(&n, "layer_activity").fidelity, Fidelity::Partial);
    assert_eq!(n.principles.layers.background, Some(Activity::Structured));
    assert_eq!(n.principles.layers.foreground, None);
    assert_eq!(n.principles.layers.midground, None);

    // All three parts given: Exact.
    p.layer_activity = Some(Trait::new(
        RefLayerActivity {
            foreground: Some(RefPresence::Dominant),
            midground: Some(RefActivity::Active),
            background: Some(RefActivity::Still),
        },
        0.8,
    ));
    let n = normalize(&p);
    assert_eq!(row(&n, "layer_activity").fidelity, Fidelity::Exact);
}

fn swatch(hex: &str, prevalence: f32) -> PaletteEvidenceSwatch {
    PaletteEvidenceSwatch {
        hex: hex.into(),
        prevalence,
    }
}

#[test]
fn accent_hint_uses_the_most_prevalent_saturated_swatch() {
    let mut p = bare();
    p.approximate_palette_evidence = vec![
        swatch("#EEEEEE", 0.6),  // most prevalent but grey: never an accent
        swatch("#2B50FF", 0.05), // saturated, rare
        swatch("#FF4A1C", 0.2),  // saturated, most prevalent of the saturated
    ];
    let n = normalize(&p);
    let hue = n.principles.accent_hue.expect("accent hue");
    let expected = oklab::hue_of(0xFF4A1C);
    assert!((hue - expected).abs() < 1e-3, "{hue} vs {expected}");
    let r = row(&n, "accent_hint");
    assert_eq!(r.fidelity, Fidelity::Closest);
    assert_eq!(r.reference.as_deref(), Some("#FF4A1C"));

    // Swatch order does not matter.
    p.approximate_palette_evidence.reverse();
    assert_eq!(normalize(&p).principles.accent_hue, Some(hue));
}

#[test]
fn accent_hint_needs_a_chromatic_swatch() {
    let mut p = bare();
    p.approximate_palette_evidence = vec![swatch("#101010", 0.7), swatch("#DDDDDD", 0.3)];
    let n = normalize(&p);
    assert_eq!(n.principles.accent_hue, None);
    assert_eq!(row(&n, "accent_hint").fidelity, Fidelity::Unknown);
}

#[test]
fn monochrome_and_muted_palettes_suppress_the_accent_hint() {
    for c in [PaletteCharacter::Monochrome, PaletteCharacter::Muted] {
        let mut p = bare();
        p.palette_character = Some(Trait::new(c, 0.9));
        p.approximate_palette_evidence = vec![swatch("#FF4A1C", 0.3), swatch("#111111", 0.7)];
        let n = normalize(&p);
        assert_eq!(n.principles.accent_hue, None, "{c:?}");
        assert_eq!(row(&n, "accent_hint").fidelity, Fidelity::Partial, "{c:?}");
    }
    // A vivid palette keeps it.
    let mut p = bare();
    p.palette_character = Some(Trait::new(PaletteCharacter::Vivid, 0.9));
    p.approximate_palette_evidence = vec![swatch("#FF4A1C", 0.3)];
    assert!(normalize(&p).principles.accent_hue.is_some());
}

#[test]
fn unsupported_traits_are_thresholded_sorted_and_deduplicated() {
    let mut p = bare();
    let t = |kind, confidence| UnsupportedTrait {
        kind,
        confidence,
        evidence: None,
    };
    p.unsupported_reference_traits = vec![
        t(UnsupportedTraitKind::Glitch, 0.9),
        t(UnsupportedTraitKind::True3d, 0.8),
        t(UnsupportedTraitKind::ParticleField, 0.3), // below threshold: not listed
        t(UnsupportedTraitKind::Glitch, 0.7),        // duplicate
    ];
    // Implied by mappings, overlapping with the listed ones.
    p.material_character = Some(Trait::new(MaterialCharacter::Rendered3d, 0.9)); // True3d
    p.background_character = Some(Trait::new(BackgroundCharacter::Gradient, 0.9));
    let n = normalize(&p);
    assert!(!n.unsupported.contains(&UnsupportedTraitKind::ParticleField));
    assert!(n.unsupported.contains(&UnsupportedTraitKind::Glitch));
    assert!(n.unsupported.contains(&UnsupportedTraitKind::True3d));
    assert!(n
        .unsupported
        .contains(&UnsupportedTraitKind::GradientBackground));
    assert!(
        n.unsupported.windows(2).all(|w| w[0] < w[1]),
        "strictly sorted, no duplicates: {:?}",
        n.unsupported
    );
    assert_eq!(n.unsupported.len(), 3);
}

#[test]
fn normalize_is_deterministic() {
    let p = parse_ok(&full_profile_json());
    let a = normalize(&p);
    let b = normalize(&p);
    assert_eq!(a, b);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn full_profile_normalizes_into_the_expected_principles() {
    let n = normalize(&parse_ok(&full_profile_json()));
    let p = &n.principles;
    assert_eq!(p.tone, Some(ResolvedTone::Technical));
    assert_eq!(p.polarity, Some(ResolvedPolarity::Dark));
    assert_eq!(p.temperature, Some(ResolvedTemperature::Cool));
    assert_eq!(p.background, Some(BackgroundGrammar::TechnicalGrid));
    assert_eq!(p.typography, Some(TypographyPairing::CondensedMono));
    assert_eq!(p.material, Some(MaterialFinish::Screen));
    assert_eq!(p.temperament, Some(TemperamentKind::Precise));
    assert_eq!(p.transition, Some(TransitionFamily::Geometric));
    // `image_treatment: none` teaches nothing about images.
    assert_eq!(p.image_treatment, None);
    assert_eq!(row(&n, "image_treatment").fidelity, Fidelity::Partial);
    assert!(p.accent_hue.is_some());
    // Rows are in report order with the accent hint last.
    let order: Vec<&str> = n.dimensions.iter().map(|d| d.dimension).collect();
    let mut expected: Vec<&str> = DIMENSIONS.to_vec();
    expected.push("accent_hint");
    expected.extend(VISUAL_ROWS);
    assert_eq!(order, expected);
    // The profile's own confident unsupported traits are recorded.
    assert!(n.unsupported.contains(&UnsupportedTraitKind::Glitch));
    assert!(n.unsupported.contains(&UnsupportedTraitKind::ParticleField));
}

// ---------------------------------------------------------------------------
// coverage
// ---------------------------------------------------------------------------

#[test]
fn coverage_statuses_follow_fidelity_and_the_resolved_style() {
    let mut p = bare();
    p.tone = Some(Trait::new(RefTone::Technical, 0.9)); // exact, holds
    p.polarity = Some(Trait::new(RefPolarity::Dark, 0.9)); // exact, but style says light
    p.motion_temperament = Some(Trait::new(RefTemperament::Snappy, 0.9)); // exact, holds
    p.transition_character = Some(Trait::new(RefTransition::Continuous, 0.9)); // closest
    p.background_character = Some(Trait::new(BackgroundCharacter::Photographic, 0.9)); // unsupported
    p.composition_rhythm = Some(Trait::new(RefRhythm::HighFrequency, 0.3)); // low confidence
    p.material_character = Some(Trait::new(MaterialCharacter::Screen, 0.9)); // exact, holds
                                                                             // visual_density absent -> unknown
    let n = normalize(&p);
    let style = StyleProfile {
        polarity: Polarity::Light, // explicit style beats the reference
        ..Default::default()
    };
    let resolved = resolve_with(&style, Some(&n.principles), 0);
    let report = coverage(&n, &resolved);

    let status = |dim: &str| {
        report
            .rows
            .iter()
            .find(|r| r.dimension == dim)
            .unwrap_or_else(|| panic!("no row {dim}"))
            .status
    };
    assert_eq!(status("tone"), CoverageStatus::Match);
    assert_eq!(status("motion_temperament"), CoverageStatus::Match);
    assert_eq!(status("material_character"), CoverageStatus::Match);
    assert_eq!(status("transition_character"), CoverageStatus::Partial);
    assert_eq!(status("polarity"), CoverageStatus::Overridden);
    assert_eq!(status("background_character"), CoverageStatus::Unsupported);
    assert_eq!(status("composition_rhythm"), CoverageStatus::Unknown);
    assert_eq!(status("visual_density"), CoverageStatus::Unknown);
    assert_eq!(status("accent_hint"), CoverageStatus::Unknown);

    // The report keeps what the director actually resolved.
    let pol = report
        .rows
        .iter()
        .find(|r| r.dimension == "polarity")
        .unwrap();
    assert_eq!(pol.reference.as_deref(), Some("dark"));
    assert_eq!(pol.applied.as_deref(), Some("dark"));
    assert_eq!(pol.resolved, "light");
}

#[test]
fn coverage_summary_lists_are_consistent_with_the_rows() {
    let n = normalize(&parse_ok(&full_profile_json()));
    let style = StyleProfile {
        temperament: motion_core::style::Temperament::Restrained, // overrides snappy
        ..Default::default()
    };
    let resolved = resolve_with(&style, Some(&n.principles), 0);
    let report = coverage(&n, &resolved);

    assert_eq!(report.rows.len(), DIMENSIONS.len() + 1 + VISUAL_ROWS.len());
    let with_status = |s: CoverageStatus| -> Vec<&'static str> {
        report
            .rows
            .iter()
            .filter(|r| r.status == s)
            .map(|r| r.dimension)
            .collect()
    };
    assert_eq!(report.matched, with_status(CoverageStatus::Match));
    assert_eq!(report.partial, with_status(CoverageStatus::Partial));
    assert_eq!(report.overridden, with_status(CoverageStatus::Overridden));
    assert_eq!(report.unsupported, with_status(CoverageStatus::Unsupported));
    assert_eq!(report.unknown, with_status(CoverageStatus::Unknown));
    let mut supported = report.matched.clone();
    supported.extend(&report.partial);
    supported.extend(&report.overridden);
    let mut got = report.supported.clone();
    supported.sort_unstable();
    got.sort_unstable();
    assert_eq!(supported, got);
    // Every row lands in exactly one bucket.
    let total = report.matched.len()
        + report.partial.len()
        + report.overridden.len()
        + report.unsupported.len()
        + report.unknown.len();
    assert_eq!(total, report.rows.len());
    // Explicit temperament conflicts with the reference's snappy.
    assert!(report.overridden.contains(&"motion_temperament"));
    assert_eq!(report.unsupported_traits, n.unsupported);
}

#[test]
fn coverage_text_report_names_statuses_and_unsupported_traits() {
    let n = normalize(&parse_ok(&full_profile_json()));
    let resolved = resolve_with(&StyleProfile::default(), Some(&n.principles), 0);
    let text = coverage(&n, &resolved).to_text();
    assert!(text.contains("MATCH"), "{text}");
    assert!(text.contains("unsupported reference traits"), "{text}");
    assert!(text.contains("glitch"), "{text}");
    // One line per row plus two summary lines.
    assert_eq!(
        text.lines().count(),
        DIMENSIONS.len() + 1 + VISUAL_ROWS.len() + 2
    );
    // No unsupported traits: says so.
    let n = normalize(&bare());
    let resolved = resolve_with(&StyleProfile::default(), Some(&n.principles), 0);
    assert!(coverage(&n, &resolved)
        .to_text()
        .contains("unsupported reference traits: none"));
}

#[test]
fn coverage_is_deterministic() {
    let n = normalize(&parse_ok(&full_profile_json()));
    let resolved = resolve_with(&StyleProfile::default(), Some(&n.principles), 5);
    assert_eq!(coverage(&n, &resolved), coverage(&n, &resolved));
}

#[test]
fn coverage_palette_character_matches_print_family() {
    let mut p = bare();
    p.palette_character = Some(Trait::new(PaletteCharacter::Multicolor, 0.9));
    let n = normalize(&p);
    let resolved = resolve_with(&StyleProfile::default(), Some(&n.principles), 0);
    let report = coverage(&n, &resolved);
    let r = report
        .rows
        .iter()
        .find(|r| r.dimension == "palette_character")
        .unwrap();
    assert_eq!(r.status, CoverageStatus::Match);
    // An explicit dark polarity still picks a print family (PrintDark); fields hold.
    let dark = StyleProfile {
        polarity: Polarity::Dark,
        tone: Tone::Editorial,
        ..Default::default()
    };
    let resolved = resolve_with(&dark, Some(&n.principles), 0);
    let report = coverage(&n, &resolved);
    let r = report
        .rows
        .iter()
        .find(|r| r.dimension == "palette_character")
        .unwrap();
    assert_eq!(r.status, CoverageStatus::Match);
}

// ---------------------------------------------------------------------------
// bundle
// ---------------------------------------------------------------------------

#[test]
fn repair_request_carries_issues_and_previous_response_once() {
    let previous = r#"{"version":"0.3"}"#;
    let issues = parse_profile(previous, None).unwrap_err();
    assert!(!issues.is_empty());
    let r = repair_request(Some(FINGERPRINT), previous, &issues);
    assert_eq!(MAX_REPAIR_ATTEMPTS, 1);
    assert_eq!(r.repair_attempt, 1);
    assert_eq!(r.repair_attempt, MAX_REPAIR_ATTEMPTS);
    assert_eq!(r.protocol, PROTOCOL);
    assert_eq!(r.previous_response, previous);
    assert_eq!(r.reference_fingerprint.as_deref(), Some(FINGERPRINT));
    assert_eq!(r.issues.len(), issues.len());
    for (text, issue) in r.issues.iter().zip(&issues) {
        assert_eq!(text, &issue.to_string());
    }
    assert!(r.instructions.contains("only repair attempt"));
    assert_eq!(r.response_schema, response_schema());
    // No fingerprint is allowed.
    assert_eq!(repair_request(None, "x", &[]).reference_fingerprint, None);
}

#[test]
fn response_schema_equals_the_checked_in_schema() {
    let path = repo().join("schema/reference-style-profile-v0.2.schema.json");
    let on_disk: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("schema file")).unwrap();
    assert_eq!(response_schema(), on_disk);
}

#[test]
fn instructions_forbid_copying_text_logos_and_audio_and_demand_json_only() {
    let lower = INSTRUCTIONS.to_lowercase();
    assert!(lower.contains("do not reproduce any text"), "text");
    assert!(lower.contains("logos"), "logos");
    assert!(lower.contains("audio"), "audio");
    assert!(INSTRUCTIONS.contains("ONLY"), "ONLY");
    assert!(lower.contains("style and visual construction"));
    // No positions, timings or frame reconstruction requested.
    assert!(lower.contains("no free-text field"));
}

#[test]
fn request_is_independent_of_any_story_and_carries_the_protocol_limits() {
    let ev = evidence();
    let req = interpreter_request(&ev, "contact-sheet.png");
    assert_eq!(req.protocol, PROTOCOL);
    assert_eq!(req.reference_fingerprint, ev.reference_fingerprint);
    assert_eq!(req.max_repair_attempts, MAX_REPAIR_ATTEMPTS);
    assert_eq!(req.samples.len(), 3);
    assert_eq!(req.samples[1].id, "s02");
    assert_eq!(req.evidence_file, "evidence.json");
    assert_eq!(req.contact_sheet, "contact-sheet.png");
    assert_eq!(req.instructions, INSTRUCTIONS);
    assert_eq!(req.response_schema, response_schema());
    assert!(req
        .evidence_metrics
        .iter()
        .any(|m| m == "temporal.changes_per_10s"));
    // Serializes and round-trips.
    let text = serde_json::to_string(&req).unwrap();
    let back: motion_core::reference::InterpreterRequest = serde_json::from_str(&text).unwrap();
    assert_eq!(back, req);
}

#[test]
fn interpreter_prompt_lists_every_vocabulary_value_and_no_story_words() {
    let ev = evidence();
    let req = interpreter_request(&ev, "contact-sheet.png");
    let prompt = interpreter_prompt(&req, &ev);

    let vocab = vocabulary();
    assert!(!vocab.is_empty());
    for (dim, values) in &vocab {
        assert!(prompt.contains(&format!("**{dim}**")), "dimension {dim}");
        for (value, _) in values {
            assert!(
                prompt.contains(&format!("`{value}`")),
                "vocabulary value {value} of {dim} missing from the prompt"
            );
        }
    }
    for id in ["s01", "s02", "s03"] {
        assert!(prompt.contains(id), "sample {id}");
    }
    assert!(prompt.contains(FINGERPRINT));

    // No CreativeIntent / story vocabulary. Rule 10 of the interpreter
    // instructions legitimately forbids inferring "beats" (musical), so that
    // one sentence is removed before scanning.
    let scanned = prompt.replace("sound, music, beats or speech", "");
    for word in [
        "CreativeIntent",
        "creative_intent",
        "statement",
        "beats",
        "narrative",
    ] {
        assert!(
            !scanned.contains(word),
            "prompt contains story word {word:?}"
        );
    }
    let request_json = serde_json::to_string(&req)
        .unwrap()
        .replace("sound, music, beats or speech", "");
    for word in ["CreativeIntent", "statement"] {
        assert!(!request_json.contains(word), "request contains {word:?}");
    }
}

#[test]
fn vocabulary_matches_the_profile_types() {
    let vocab = vocabulary();
    let values = |d: &str| -> Vec<String> {
        vocab
            .iter()
            .find(|(k, _)| k == d)
            .map(|(_, v)| v.iter().map(|(x, _)| x.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(values("polarity"), ["light", "dark", "mixed"]);
    assert_eq!(values("temperature"), ["warm", "cool", "neutral"]);
    // Every documented dimension has a vocabulary entry.
    for &d in DIMENSIONS {
        if d == "layer_activity" {
            for part in ["foreground", "midground", "background"] {
                assert!(
                    !values(&format!("layer_activity.{part}")).is_empty(),
                    "{part}"
                );
            }
        } else {
            assert!(!values(d).is_empty(), "{d}");
        }
    }
    // Every vocabulary value of every dimension parses back as a profile value.
    for (dim, vs) in &vocab {
        // visual_language values are covered by tests/reference_visual_language.rs
        if dim.starts_with("layer_activity")
            || dim.starts_with("unsupported")
            || dim.starts_with("visual_language")
        {
            continue;
        }
        for (v, _) in vs {
            let doc = json!({ "version": "0.1", dim.as_str(): { "value": v, "confidence": 0.9 } });
            parse(&doc).unwrap_or_else(|e| panic!("{dim}={v}: {e:?}"));
        }
    }
    for v in values("unsupported_reference_traits.trait") {
        let doc = json!({ "version": "0.1", "unsupported_reference_traits": [{ "trait": v, "confidence": 0.9 }] });
        parse(&doc).unwrap_or_else(|e| panic!("trait {v}: {e:?}"));
    }
}

// ---------------------------------------------------------------------------
// Copy boundary: the schema has no free-text field
// ---------------------------------------------------------------------------

/// Paths (dotted, `$defs` resolved by name) of string leaves that are allowed
/// to carry arbitrary identifiers rather than vocabulary values.
const ALLOWED_STRING_FIELDS: &[&str] = &[
    "properties.version",
    "properties.reference_fingerprint",
    "$defs.PaletteEvidenceSwatch.properties.hex",
    "$defs.EvidenceLinks.properties.samples.items",
    "$defs.EvidenceLinks.properties.metrics.items",
];

fn is_string_typed(node: &Value) -> bool {
    match node.get("type") {
        Some(Value::String(t)) => t == "string",
        Some(Value::Array(ts)) => ts.iter().any(|t| t == "string"),
        _ => false,
    }
}

fn is_closed(node: &Value) -> bool {
    node.get("enum").is_some() || node.get("const").is_some()
}

fn collect_free_strings(node: &Value, path: &str, out: &mut Vec<String>) {
    let Some(obj) = node.as_object() else { return };
    if is_string_typed(node) && !is_closed(node) {
        out.push(path.to_string());
    }
    for (k, v) in obj {
        match k.as_str() {
            "properties" | "$defs" => {
                if let Some(children) = v.as_object() {
                    for (name, child) in children {
                        collect_free_strings(
                            child,
                            format!("{path}.{k}.{name}").trim_start_matches('.'),
                            out,
                        );
                    }
                }
            }
            "items" => {
                collect_free_strings(v, format!("{path}.items").trim_start_matches('.'), out)
            }
            "anyOf" | "oneOf" | "allOf" => {
                if let Some(alts) = v.as_array() {
                    for alt in alts {
                        collect_free_strings(alt, path, out);
                    }
                }
            }
            _ => {}
        }
    }
}

#[test]
fn schema_has_no_free_text_field() {
    let schema = response_schema();
    let mut free = Vec::new();
    collect_free_strings(&schema, "", &mut free);
    free.sort();
    free.dedup();
    let unexpected: Vec<&String> = free
        .iter()
        .filter(|p| !ALLOWED_STRING_FIELDS.contains(&p.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "free-text string fields outside the identifier allowlist: {unexpected:?}"
    );
    // The allowlist is not vacuous: every allowed identifier field really exists.
    for allowed in ALLOWED_STRING_FIELDS {
        assert!(
            free.iter().any(|p| p == allowed),
            "allowlisted field {allowed} not found in the schema (found {free:?})"
        );
    }
}

#[test]
fn schema_closes_every_enum_dimension() {
    // The walker treats `oneOf` of documented consts as closed: verify each
    // vocabulary type is a closed set (no bare string) under $defs.
    let schema = response_schema();
    let defs = schema["$defs"].as_object().expect("defs");
    for name in [
        "RefTone",
        "RefPolarity",
        "RefTemperature",
        "RefLevel",
        "PaletteCharacter",
        "BackgroundCharacter",
        "TypographyCharacter",
        "MaterialCharacter",
        "ImageTreatmentCharacter",
        "RefDensity",
        "RefRhythm",
        "RefTemperament",
        "RefTransition",
        "RefScale",
        "UnsupportedTraitKind",
    ] {
        let d = defs.get(name).unwrap_or_else(|| panic!("def {name}"));
        let alts = d.get("oneOf").and_then(Value::as_array);
        let closed = d.get("enum").is_some()
            || alts.is_some_and(|a| {
                a.iter()
                    .all(|x| x.get("const").is_some() || x.get("enum").is_some())
            });
        assert!(closed, "{name} is not a closed set: {d}");
    }
    assert_eq!(schema["additionalProperties"], json!(false));
}
