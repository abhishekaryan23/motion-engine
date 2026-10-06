//! Reference profiles change design, never content or arithmetic.
use motion_core::assets::{AssetManifest, AssetStyleProfile};
use motion_core::compiler::{self, taste::*, ApproxMeasure, AssetLibrary};
use motion_core::intent::CreativeIntent;
use motion_core::reference::coverage::CoverageStatus;
use motion_core::reference::profile::*;
use motion_core::reference::{coverage, normalize, parse_profile, validate_profile};
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject, TreatmentPreset};
use motion_core::style::{
    Density, MaterialStyle, Polarity, StyleProfile, Temperament, Temperature, Tone,
};
use serde_json::{json, Value};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
fn read(path: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/{path}")).unwrap()
}
fn intent(path: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(path)).unwrap()
}
fn profile(value: Value) -> ReferenceStyleProfile {
    parse_profile(&value.to_string(), None).unwrap().profile
}
fn normalized(value: Value) -> motion_core::reference::NormalizedReference {
    normalize(&profile(value))
}
fn library() -> AssetLibrary {
    AssetLibrary::new(format!("{ROOT}/assets"))
}
fn build(
    i: &CreativeIntent,
    s: &StyleProfile,
    r: Option<&ReferencePrinciples>,
    m: &AssetManifest,
) -> MotionProject {
    let project = compiler::compile_full(i, s, r, &library(), &ApproxMeasure, m).unwrap();
    motion_core::validate::validate(&project, None).unwrap();
    project
}
fn layers(layers: &[Layer]) -> Vec<&Layer> {
    let mut out = Vec::new();
    for layer in layers {
        out.push(layer);
        if let LayerKind::Group { children } = &layer.kind {
            out.extend(self::layers(children));
        }
    }
    out
}
fn profiles() -> Vec<motion_core::reference::NormalizedReference> {
    [
        json!({"version":"0.1","tone":{"value":"editorial","confidence":0.9},"polarity":{"value":"light","confidence":0.9},"temperature":{"value":"warm","confidence":0.9},"motion_temperament":{"value":"restrained","confidence":0.9},"composition_rhythm":{"value":"measured_editorial","confidence":0.9}}),
        json!({"version":"0.1","tone":{"value":"technical","confidence":0.9},"polarity":{"value":"dark","confidence":0.9},"temperature":{"value":"cool","confidence":0.9},"motion_temperament":{"value":"snappy","confidence":0.9},"transition_character":{"value":"geometric","confidence":0.9},"composition_rhythm":{"value":"progressive","confidence":0.9}}),
        json!({"version":"0.1","tone":{"value":"playful","confidence":0.9},"motion_temperament":{"value":"energetic","confidence":0.9},"composition_rhythm":{"value":"active","confidence":0.9},"scale_contrast":{"value":"dramatic","confidence":0.9},"layer_activity":{"value":{"foreground":"dominant","midground":"active","background":"active"},"confidence":0.9}})
    ].into_iter().map(normalized).collect()
}

#[test]
fn strict_profile_rejects_content_and_invalid_rules() {
    for value in [
        json!({"version":"0.3"}),
        json!({"version":"0.1","source_text":"copy me"}),
        json!({"version":"0.1","tone":{"value":"cinema","confidence":1}}),
        json!({"version":"0.1","tone":{"value":"editorial","confidence":1.1}}),
        json!({"version":"0.1","tone":{"value":"editorial","confidence":-0.1}}),
        json!({"version":"0.1","tone":{"value":"editorial","confidence":1,"evidence":{"metrics":["audio.bpm"]}}}),
        json!({"version":"0.1","tone":{"value":"editorial","confidence":1,"evidence":{"samples":["source caption"]}}}),
        json!({"version":"0.1","tone":{"value":"editorial","confidence":1,"evidence":{"time_ranges":[[2,1]]}}}),
        json!({"version":"0.1","approximate_palette_evidence":[{"hex":"#GG0000","prevalence":0.3}]}),
        json!({"version":"0.1","unsupported_reference_traits":[{"trait":"true3d","confidence":1},{"trait":"true3d","confidence":1}]}),
    ] {
        assert!(
            parse_profile(&value.to_string(), None).is_err(),
            "accepted {value}"
        );
    }
    let mut p = profile(json!({"version":"0.1"}));
    p.tone = Some(Trait::new(RefTone::Editorial, f32::NAN));
    assert!(!validate_profile(&p, None).is_empty());
}

#[test]
fn aliases_and_unknowns_are_conservative_including_layer_activity() {
    let parsed = parse_profile(r#"{"version":"0.1","polarity":{"value":" Mostly-Black ","confidence":0.9},"motion_temperament":{"value":"unknown","confidence":0.9},"layer_activity":{"value":"unknown","confidence":0.9}}"#, None).unwrap();
    assert_eq!(parsed.aliases.len(), 3);
    let n = normalize(&parsed.profile);
    assert_eq!(n.principles.polarity, Some(ResolvedPolarity::Dark));
    assert!(n.principles.temperament.is_none());
    assert_eq!(n.principles.layers, LayerHints::default());
    assert!(parse_profile(
        r#"{"version":"0.1","tone":{"value":"premium polished luxury","confidence":1}}"#,
        None
    )
    .is_err());
}

#[test]
fn confidence_threshold_does_not_leak_into_accent_selection() {
    for value in ["muted", "monochrome"] {
        for confidence in [0.49, 0.5] {
            let n = normalized(
                json!({"version":"0.1","palette_character":{"value":value,"confidence":confidence},"approximate_palette_evidence":[{"hex":"#FF3300","prevalence":0.2}]}),
            );
            assert_eq!(n.principles.accent_hue.is_some(), confidence < 0.5);
            assert_eq!(n.principles.color_fields.is_some(), confidence >= 0.5);
        }
    }
    let n = normalized(
        json!({"version":"0.1","tone":{"value":"technical","confidence":0.49},"material_character":{"value":"metallic","confidence":0.49},"unsupported_reference_traits":[{"trait":"particle_field","confidence":0.49}]}),
    );
    assert!(n.principles.is_empty());
    assert!(n.unsupported.is_empty());
}

#[test]
fn all_unknown_reference_keeps_legacy_compile_and_asset_identity() {
    let n = normalized(
        json!({"version":"0.1","tone":{"value":null,"confidence":0.9},"layer_activity":{"value":{"foreground":null,"midground":null,"background":null},"confidence":0.9}}),
    );
    assert!(n.principles.is_empty());
    let i = intent("examples/public/collection-accumulate.intent.json");
    for s in [
        StyleProfile::default(),
        StyleProfile {
            material: MaterialStyle::Flat,
            ..StyleProfile::default()
        },
    ] {
        let empty = AssetManifest::default();
        let before = build(&i, &s, None, &empty);
        let after = build(&i, &s, Some(&n.principles), &empty);
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        let without = compiler::plan_assets(&i, &s, &library()).unwrap();
        let with =
            compiler::plan_assets_with_reference(&i, &s, Some(&n.principles), &library()).unwrap();
        assert_eq!(without.style, with.style);
        let resolved = compiler::resolve_taste(&i, &s, Some(&n.principles));
        assert_eq!(resolved.tone, ResolvedTone::Classic);
        assert!(coverage(&n, &resolved)
            .rows
            .iter()
            .all(|r| r.status == CoverageStatus::Unknown));
    }
}

#[test]
fn explicit_style_precedence_is_reported_in_coverage() {
    let n = normalized(
        json!({"version":"0.1","tone":{"value":"playful","confidence":1},"polarity":{"value":"light","confidence":1},"temperature":{"value":"warm","confidence":1},"motion_temperament":{"value":"energetic","confidence":1},"visual_density":{"value":"dense","confidence":1},"material_character":{"value":"paper","confidence":1}}),
    );
    let s = StyleProfile {
        tone: Tone::Technical,
        polarity: Polarity::Dark,
        temperature: Temperature::Cool,
        temperament: Temperament::Restrained,
        density: Density::Sparse,
        material: MaterialStyle::Flat,
        ..StyleProfile::default()
    };
    let r = resolve_with(&s, Some(&n.principles), 0);
    assert_eq!(r.material, MaterialFinish::CleanFlat);
    assert_eq!(r.effective.material, MaterialStyle::Flat);
    let report = coverage(&n, &r);
    for dim in [
        "tone",
        "polarity",
        "temperature",
        "motion_temperament",
        "visual_density",
        "material_character",
    ] {
        assert!(report.overridden.contains(&dim), "{dim}: {report:?}");
    }
}

#[test]
fn coverage_distinguishes_match_partial_unknown_and_unsupported() {
    let n = normalized(
        json!({"version":"0.1","tone":{"value":"editorial","confidence":1},"background_character":{"value":"photographic","confidence":1},"motion_temperament":{"value":"fluid","confidence":1},"material_character":{"value":"metallic","confidence":1},"unsupported_reference_traits":[{"trait":"chrome_or_metallic","confidence":1}]}),
    );
    let r = resolve_with(&StyleProfile::default(), Some(&n.principles), 0);
    let report = coverage(&n, &r);
    assert!(report.matched.contains(&"tone"));
    assert!(report.partial.contains(&"motion_temperament"));
    assert!(report.partial.contains(&"material_character"));
    assert!(report.unsupported.contains(&"background_character"));
    assert!(report.unknown.contains(&"scale_contrast"));
    assert_eq!(
        report
            .unsupported_traits
            .iter()
            .filter(|t| **t == UnsupportedTraitKind::ChromeOrMetallic)
            .count(),
        1
    );
    assert_eq!(report, coverage(&n, &r));
}

#[test]
fn material_and_image_bias_reach_assets_and_compiled_images() {
    let i: CreativeIntent = serde_json::from_value(json!({"version":"0.2","title":"Unrelated engineering","beats":[{"purpose":"emphasize","statement":"A new method makes room","primary":{"kind":"phrase","value":"Space to learn"}}]})).unwrap();
    let manifest: AssetManifest = serde_json::from_str(&read("assets/test_manifest.json")).unwrap();
    let s = StyleProfile::default();
    for (bias, expected) in [
        ("natural", TreatmentPreset::Natural),
        ("monochrome", TreatmentPreset::EditorialMonochrome),
        ("duotone", TreatmentPreset::MutedDocumentary),
        ("muted", TreatmentPreset::MutedDocumentary),
        ("print_cutout", TreatmentPreset::PaperCutout),
    ] {
        let n = normalized(
            json!({"version":"0.1","material_character":{"value":"flat","confidence":1},"image_treatment":{"value":bias,"confidence":1}}),
        );
        let r = compiler::resolve_taste(&i, &s, Some(&n.principles));
        assert_eq!(r.material, MaterialFinish::CleanFlat);
        assert_eq!(r.effective.material, MaterialStyle::Flat);
        let plan =
            compiler::plan_assets_with_reference(&i, &s, Some(&n.principles), &library()).unwrap();
        assert_eq!(plan.style, AssetStyleProfile::from_resolved(&r));
        if bias == "natural" {
            assert_eq!(plan.style.medium, "clean editorial photography");
        }
        let p = build(&i, &s, Some(&n.principles), &manifest);
        let treatments: Vec<_> = p
            .scenes
            .iter()
            .flat_map(|s| layers(&s.layers))
            .filter_map(|l| match &l.kind {
                LayerKind::Image {
                    treatment: Some(t), ..
                } => Some(t),
                _ => None,
            })
            .collect();
        assert!(!treatments.is_empty(), "image was not compiled for {bias}");
        for t in treatments {
            assert_eq!(t.preset, expected, "{bias}");
            assert!(t.duotone.is_none(), "people must not be duotoned");
        }
    }
}

#[test]
fn reference_preserves_metric_values_labels_order_and_ratios() {
    let i = intent("examples/public/derived-metric.intent.json");
    let s = StyleProfile::default();
    for n in profiles() {
        let p = build(&i, &s, Some(&n.principles), &AssetManifest::default());
        let scene = p.scenes.iter().find(|s| s.id == "beat_1").unwrap();
        let values: Vec<_> = scene
            .motions
            .iter()
            .filter_map(|m| match &m.op {
                MotionOp::Count { to, suffix, .. } => {
                    Some((m.target.as_str(), *to, suffix.as_str(), m.start))
                }
                _ => None,
            })
            .collect();
        let a = values
            .iter()
            .find(|v| v.0.ends_with("row.0.result"))
            .unwrap();
        let b = values
            .iter()
            .find(|v| v.0.ends_with("row.1.result"))
            .unwrap();
        assert_eq!((a.1, a.2, b.1, b.2), (90.0, "%", 95.0, "%"));
        assert!(a.3 < b.3);
        let ls = layers(&scene.layers);
        let text: Vec<_> = ls
            .iter()
            .filter_map(|l| match &l.kind {
                LayerKind::Text(t) => Some(t.text.to_lowercase().replace('\n', " ")),
                _ => None,
            })
            .collect();
        assert!(
            text.iter().any(|t| t == "fewer passed · higher pass rate"),
            "{text:?}"
        );
        for expected in ["540", "600", "190", "200", "students"] {
            assert!(
                text.iter().any(|t| t.contains(expected)),
                "missing {expected}: {text:?}"
            );
        }
        let fill = |suffix: &str| {
            scene
                .motions
                .iter()
                .find_map(|m| match &m.op {
                    MotionOp::AccentExpand { to } if m.target.ends_with(suffix) => Some(to.width),
                    _ => None,
                })
                .unwrap()
        };
        assert!((fill("bars.a.fill") / fill("bars.b.fill") - 90.0 / 95.0).abs() < 1e-5);
        let again = build(&i, &s, Some(&n.principles), &AssetManifest::default());
        assert_eq!(
            serde_json::to_value(&p).unwrap(),
            serde_json::to_value(again).unwrap()
        );
    }
}

#[test]
fn reference_preserves_collection_content_and_insertion_order() {
    let i = intent("examples/public/collection-accumulate.intent.json");
    for n in profiles() {
        let p = build(
            &i,
            &StyleProfile::default(),
            Some(&n.principles),
            &AssetManifest::default(),
        );
        let scene = p.scenes.iter().find(|s| s.id == "beat_1").unwrap();
        let text: Vec<_> = layers(&scene.layers)
            .into_iter()
            .filter_map(|l| match &l.kind {
                LayerKind::Text(t) => Some(t.text.to_lowercase()),
                _ => None,
            })
            .collect();
        let indices: Vec<_> = ["water bottle", "jacket", "laptop", "books"]
            .iter()
            .map(|want| {
                text.iter()
                    .position(|t| t.replace('\n', " ") == *want)
                    .unwrap()
            })
            .collect();
        assert!(indices.windows(2).all(|w| w[0] < w[1]));
        assert!(text.iter().any(|t| t.contains("9 kg")), "{text:?}");
    }
}

#[test]
fn every_schema_vocabulary_value_parses_normalizes_and_reports() {
    for (dimension, values) in motion_core::reference::bundle::vocabulary() {
        for (value, _) in values {
            let mut doc = json!({"version":"0.1"});
            if let Some(field) = dimension.strip_prefix("visual_language.") {
                doc = json!({"version":"0.2"});
                doc["visual_language"] = if field == "asset_roles.role" {
                    json!({"asset_roles":[{"role":value,"confidence":0.5}]})
                } else if field.starts_with("composition_language.") {
                    json!({"composition_language":{"preferred":[value],"confidence":0.5}})
                } else {
                    json!({field: {"value":value,"confidence":0.5}})
                };
            } else if let Some(part) = dimension.strip_prefix("layer_activity.") {
                doc["layer_activity"] = json!({"value":{part: value},"confidence":0.5});
            } else if dimension == "unsupported_reference_traits.trait" {
                doc["unsupported_reference_traits"] = json!([{"trait":value,"confidence":0.5}]);
            } else {
                doc[&dimension] = json!({"value":value,"confidence":0.5});
            }
            let n = normalized(doc);
            let r = resolve_with(&StyleProfile::default(), Some(&n.principles), 0);
            let report = coverage(&n, &r);
            // style dimensions + accent_hint + 7 visual-language rows (0.7.1)
            assert_eq!(report.rows.len(), DIMENSIONS.len() + 1 + 7);
            assert_eq!(
                serde_json::to_value(&report).unwrap(),
                serde_json::to_value(coverage(&n, &r)).unwrap()
            );
        }
    }
}

fn evidence() -> motion_core::reference::ReferenceEvidence {
    let distribution = json!({"mean":0,"p10":0,"p50":0,"p90":0});
    serde_json::from_value(json!({
        "version":"0.1","analyzer_version":"0.7.1","reference_fingerprint":"rf1-0123456789abcdef","media_fingerprint":"fnv1a64-0123456789abcdef",
        "metadata":{"width":32,"height":32,"duration_seconds":2.0,"fps":30,"frame_count":60,"aspect_ratio":1,"orientation":"square","video_codec":"h264","audio_present":true},
        "analysis":{"width":32,"height":32,"fps":4,"frames":8,"decoder":"test fixture"},
        "samples":[{"id":"s01","time_seconds":0.5,"frame":15,"kinds":["periodic"],"image":"samples/s01.jpg"}],
        "color":{"polarity":{"value":"dark","confidence":1},"dark_frame_fraction":1,"light_frame_fraction":0,"luminance":distribution,"contrast":distribution,"saturation":distribution,"temperature":{"value":"neutral","confidence":1},"warm_cool_balance":0,"palette":[],"accent_prevalence":0,"palette_stability":1,"per_sample":[]},
        "temporal":{"changes":[],"cuts_per_10s":0,"changes_per_10s":0,"median_hold_seconds":2,"hold_p90_seconds":2,"static_fraction":1,"low_motion_fraction":1,"high_motion_fraction":0,"activity":distribution,"activity_variance":0,"transition_spike_ratio":0,"median_transition_seconds":0,"transition_duration":"cut","hard_cut_fraction":0,"background_activity":0,"foreground_activity":0},
        "complexity":{"edge_density":distribution,"flat_area_fraction":distribution,"large_region_count":distribution,"entropy":distribution,"occupied_ratio":distribution}
    })).unwrap()
}

#[test]
fn provenance_checks_actual_fingerprint_samples_and_duration() {
    let ev = evidence();
    let valid = json!({"version":"0.1","reference_fingerprint":ev.reference_fingerprint,"tone":{"value":"editorial","confidence":0.8,"evidence":{"samples":["s01"],"metrics":["temporal.changes_per_10s"],"time_ranges":[[0.5,2.0]]}}});
    assert!(parse_profile(&valid.to_string(), Some(&ev)).is_ok());
    for (pointer, replacement) in [
        ("/reference_fingerprint", json!("rf1-0000000000000000")),
        ("/tone/evidence/samples/0", json!("s02")),
        ("/tone/evidence/time_ranges/0/1", json!(2.1)),
    ] {
        let mut invalid = valid.clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            parse_profile(&invalid.to_string(), Some(&ev)).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn provider_neutral_request_and_single_repair_share_exact_schema() {
    use motion_core::reference::bundle::*;
    let ev = evidence();
    let request = interpreter_request(&ev, "contact-sheet.png");
    assert_eq!(request.max_repair_attempts, 1);
    assert_eq!(request.reference_fingerprint, ev.reference_fingerprint);
    assert_eq!(request.samples[0].image, ev.samples[0].image);
    assert_eq!(request.response_schema, response_schema());
    let prompt = interpreter_prompt(&request, &ev);
    assert!(prompt.contains("Do not reproduce any text"));
    assert!(prompt.contains("Do not infer anything about sound"));
    let issues = parse_profile("not JSON", Some(&ev)).unwrap_err();
    let repair = repair_request(Some(&ev.reference_fingerprint), "not JSON", &issues);
    assert_eq!(repair.repair_attempt, 1);
    assert_eq!(repair.response_schema, request.response_schema);
    assert_eq!(repair.previous_response, "not JSON");
    assert_eq!(repair.issues.len(), 1);
}

#[test]
fn reference_asset_planning_follows_compiler_grammar_selection() {
    let i: CreativeIntent = serde_json::from_value(json!({"version":"0.2","title":"Room to learn","beats":[{"purpose":"emphasize","statement":"Leave room for possibility","primary":{"kind":"phrase","value":"New possibilities"},"energy":"building"}]})).unwrap();
    let s = StyleProfile::default();
    let baseline = compiler::plan_assets(&i, &s, &library()).unwrap();
    assert_eq!(baseline.beats[0].composition, "cinematic_multiplane");
    assert!(baseline
        .requests
        .iter()
        .any(|r| r.role == motion_core::assets::AssetRole::Environment));
    let n = normalized(
        json!({"version":"0.1","tone":{"value":"technical","confidence":1},"motion_temperament":{"value":"snappy","confidence":1}}),
    );
    let plan =
        compiler::plan_assets_with_reference(&i, &s, Some(&n.principles), &library()).unwrap();
    assert_eq!(plan.beats[0].composition, "editorial_collage");
    assert!(!plan
        .requests
        .iter()
        .any(|r| r.role == motion_core::assets::AssetRole::Environment));
    // The matching compile needs no delivered background image and remains valid.
    build(&i, &s, Some(&n.principles), &AssetManifest::default());
}
