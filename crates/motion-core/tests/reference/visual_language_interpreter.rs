//! (0.7.1) Interpreter contract for the visual construction language: the
//! instructions ask for both categories, the prompt lists the vocabulary, and
//! complete v0.2 interpreter responses (fixtures/interpreter/) parse, validate
//! and normalize into the expected visual policy. Structural assertions only.

use std::path::PathBuf;

use motion_core::compiler::visual::Weight;
use motion_core::reference::bundle::{
    interpreter_prompt, interpreter_request, vocabulary, INSTRUCTIONS,
};
use motion_core::reference::evidence::{
    AnalysisSpec, ChangeKind, ChangeSummary, ColorEvidence, ComplexityEvidence, Distribution,
    DurationTendency, EvidencePolarity, EvidenceTemperature, Orientation, PolarityEstimate,
    ReferenceEvidence, ReferenceMetadata, ReferenceSample, SampleColor, SampleKind, Swatch,
    SwatchRole, TemperatureEstimate, TemporalEvidence,
};
use motion_core::reference::{normalize, parse_profile};

const FINGERPRINT: &str = "rf1-0123456789abcdef";

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

// ---------------------------------------------------------------------------
// Instructions and prompt
// ---------------------------------------------------------------------------

#[test]
fn instructions_ask_for_both_categories() {
    assert!(INSTRUCTIONS.contains("STYLE"));
    assert!(INSTRUCTIONS.contains("VISUAL CONSTRUCTION LANGUAGE"));
    assert!(INSTRUCTIONS.contains("visual_language"));
}

#[test]
fn instructions_contain_the_visual_questions() {
    let lower = INSTRUCTIONS.to_lowercase();
    // typography carrying the visual story
    assert!(lower.contains("typography carrying"), "typography question");
    // how frequent images / objects are
    assert!(
        lower.contains("present in most scenes"),
        "frequency question"
    );
    assert!(lower.contains("asset_usage"));
    // literal versus metaphor / diagram
    for word in ["literally", "diagrams", "metaphors"] {
        assert!(lower.contains(word), "explanation question lacks {word:?}");
    }
    assert!(lower.contains("explanation_mode"));
    // composition families
    assert!(lower.contains("composition families"));
    assert!(lower.contains("composition_language"));
}

#[test]
fn instructions_forbid_deciding_the_image_for_a_new_topic() {
    assert!(
        INSTRUCTIONS.contains("Do NOT say what image the engine should make"),
        "the interpreter must never choose images for a new topic"
    );
    assert!(INSTRUCTIONS.contains("plans its own visuals from the new story"));
}

#[test]
fn prompt_lists_every_visual_language_value_and_says_version_0_2() {
    let ev = evidence();
    let req = interpreter_request(&ev, "contact-sheet.png");
    let prompt = interpreter_prompt(&req, &ev);

    let visual: Vec<_> = vocabulary()
        .into_iter()
        .filter(|(dim, _)| dim.starts_with("visual_language."))
        .collect();
    // medium, usage, balance, character, roles, composition families, explanation.
    assert_eq!(
        visual.len(),
        7,
        "{:?}",
        visual.iter().map(|v| &v.0).collect::<Vec<_>>()
    );
    for (dim, values) in &visual {
        assert!(!values.is_empty(), "{dim} has no values");
        assert!(prompt.contains(&format!("**{dim}**")), "dimension {dim}");
        for (value, _) in values {
            assert!(
                prompt.contains(&format!("`{value}`")),
                "visual vocabulary value {value} of {dim} missing from the prompt"
            );
        }
    }
    assert!(prompt.contains("version \"0.2\""));
    assert!(!prompt.contains("version \"0.1\""));
}

// ---------------------------------------------------------------------------
// Interpreter response fixtures
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/interpreter")
        .join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

struct Expect {
    file: &'static str,
    weight: Weight,
    procedural: bool,
    images: bool,
}

const EXPECT: &[Expect] = &[
    Expect {
        file: "type_led_poster",
        weight: Weight::Type,
        procedural: false,
        images: false,
    },
    Expect {
        file: "image_led_editorial",
        weight: Weight::Visual,
        procedural: false,
        images: true,
    },
    Expect {
        file: "diagrammatic_explainer",
        weight: Weight::Visual,
        procedural: true,
        images: false,
    },
    Expect {
        file: "mixed_promo",
        weight: Weight::Visual,
        procedural: false,
        images: true,
    },
];

#[test]
fn interpreter_responses_parse_validate_and_normalize_to_the_expected_policy() {
    for e in EXPECT {
        let parsed = parse_profile(&fixture(e.file), None)
            .unwrap_or_else(|err| panic!("{}: invalid: {err:?}", e.file));
        assert_eq!(parsed.profile.version, "0.2", "{}", e.file);
        assert!(parsed.profile.visual_language.is_some(), "{}", e.file);
        // A complete response also carries style.
        assert!(parsed.profile.tone.is_some(), "{} style", e.file);

        let n = normalize(&parsed.profile);
        let v = &n.principles.visual;
        assert_eq!(v.weight(), e.weight, "{} weight", e.file);
        assert_eq!(
            v.prefers_procedural(),
            e.procedural,
            "{} procedural",
            e.file
        );
        assert_eq!(v.wants_images(), e.images, "{} wants_images", e.file);
        // Normalization records a row for every visual dimension.
        assert!(
            n.dimensions.iter().any(|d| d.dimension == "visual.medium"),
            "{}",
            e.file
        );
    }
}

#[test]
fn interpreter_responses_carry_the_declared_composition_preferences() {
    use motion_core::compiler::Grammar::*;
    let pref = |file: &str| {
        let n = normalize(&parse_profile(&fixture(file), None).unwrap().profile);
        n.principles.visual
    };
    let v = pref("type_led_poster");
    assert!(v.preference(KineticPoster) > 0 && v.preference(EditorialCollage) > 0);
    assert!(v.preference(CinematicMultiplane) < 0);
    let v = pref("image_led_editorial");
    assert!(v.preference(TypeImageInterlock) > 0 && v.preference(HeroObject) > 0);
    let v = pref("diagrammatic_explainer");
    assert!(v.preference(SpatialCauseEffect) > 0 && v.preference(SequentialStack) > 0);
    assert!(v.preference(KineticPoster) < 0);
    let v = pref("mixed_promo");
    assert!(v.preference(EditorialCollage) > 0);
}

#[test]
fn a_v0_1_response_still_parses_and_is_visually_neutral() {
    let doc = serde_json::json!({
        "version": "0.1",
        "reference_fingerprint": FINGERPRINT,
        "tone": { "value": "technical", "confidence": 0.9 },
        "polarity": { "value": "dark", "confidence": 0.9 },
        "motion_temperament": { "value": "snappy", "confidence": 0.8 }
    });
    let parsed = parse_profile(&doc.to_string(), None).expect("v0.1 accepted");
    assert_eq!(parsed.profile.version, "0.1");
    assert!(parsed.profile.visual_language.is_none());
    let n = normalize(&parsed.profile);
    let v = &n.principles.visual;
    assert_eq!(v.weight(), Weight::Neutral);
    assert!(!v.prefers_procedural());
    assert!(!v.wants_images());
}

#[test]
fn a_v0_1_response_cannot_carry_visual_language() {
    let doc = serde_json::json!({
        "version": "0.1",
        "visual_language": { "medium": { "value": "image_led", "confidence": 0.9 } }
    });
    assert!(parse_profile(&doc.to_string(), None).is_err());
}
