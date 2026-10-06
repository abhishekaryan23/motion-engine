//! The provider-neutral interpreter protocol (0.7).
//!
//! MotionEngine writes a ReferenceAnalysisBundle directory:
//!
//! ```text
//! <out>/evidence.json              ReferenceEvidence (deterministic)
//! <out>/samples/sNN.jpg            key frames
//! <out>/contact-sheet.png          samples + timestamp + sample kind (for humans)
//! <out>/interpreter-request.json   InterpreterRequest (this module)
//! <out>/interpreter-prompt.md      the same request as one text prompt
//! ```
//!
//! Any multimodal model (hosted or local) receives the prompt plus the sample
//! images and returns ONE ReferenceStyleProfile JSON document. MotionEngine
//! validates it; on failure the orchestrator may send ONE [`RepairRequest`].
//! The bundle never contains a CreativeIntent, a target story or a layout:
//! reference analysis is independent of the video that will use it.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::evidence::{ReferenceEvidence, ReferenceMetadata, SampleKind, EVIDENCE_METRICS};
use super::profile::ReferenceStyleProfile;
use super::validate::ProfileIssue;

pub const PROTOCOL: &str = "motionengine.reference-interpreter/0.1";
/// The protocol allows exactly one repair round trip.
pub const MAX_REPAIR_ATTEMPTS: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestSample {
    pub id: String,
    pub time_seconds: f64,
    pub kinds: Vec<SampleKind>,
    /// Relative to the bundle directory.
    pub image: String,
}

/// `interpreter-request.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InterpreterRequest {
    pub protocol: String,
    pub analyzer_version: String,
    pub reference_fingerprint: String,
    /// The interpreter instructions (same text as the prompt's rules).
    pub instructions: String,
    pub metadata: ReferenceMetadata,
    pub samples: Vec<RequestSample>,
    pub contact_sheet: String,
    pub evidence_file: String,
    pub evidence_metrics: Vec<String>,
    pub max_repair_attempts: u32,
    /// JSON Schema of the only acceptable response (ReferenceStyleProfile v0.1).
    pub response_schema: Value,
}

/// A one-time correction request after validation failed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepairRequest {
    pub protocol: String,
    pub repair_attempt: u32,
    pub reference_fingerprint: Option<String>,
    pub instructions: String,
    pub issues: Vec<String>,
    pub previous_response: String,
    pub response_schema: Value,
}

/// The ReferenceStyleProfile v0.1 JSON Schema (generated from the Rust types).
pub fn response_schema() -> Value {
    serde_json::to_value(schemars::schema_for!(ReferenceStyleProfile)).expect("schema serializes")
}

/// The interpreter rules. Core-owned wording; keep in sync with
/// docs/REFERENCE_STYLE_INTERPRETER.md.
pub const INSTRUCTIONS: &str = "\
You are analyzing MOTION-DESIGN STYLE and VISUAL CONSTRUCTION LANGUAGE only.

You receive key frames sampled from a reference video (each with its timestamp and why it was \
sampled), a contact sheet, and deterministic measurements of the whole video (color, temporal \
change, visual complexity). Audio was not analysed.

Describe the reusable principles of the reference so that a different video, about a \
completely different subject, can be designed with the same visual and temporal character AND \
built the same way: two categories, STYLE (how it looks and how motion feels) and VISUAL \
CONSTRUCTION LANGUAGE (what fills the frame, the role of type versus imagery/objects, which \
composition families dominate, how ideas are explained visually; the `visual_language` section).

Rules:
1. Analyze style and visual construction only: how it looks, how its motion feels, how its \
scenes are constructed. Describe patterns, never particular scenes.
2. Do not summarize or mention the video's subject matter.
3. Do not reproduce any text visible in the video.
4. Do not identify, describe or copy logos, brands, people or characters.
5. Do not recommend or describe specific source photographs, illustrations or assets.
6. Do not reconstruct frames: no layouts, positions, sizes, keyframes, timings, font files, CSS \
or animation code.
7. Use only the vocabulary values listed below. There is no free-text field.
8. If a dimension is not clearly shown, give \"value\": null (or omit it). Never guess. Values \
with confidence below 0.5 are recorded but not applied.
9. Infer motion_temperament, transition_character, composition_rhythm and layer_activity from \
the temporal evidence AND from what changes between consecutive samples. Frequent change is not \
automatically energetic: slow background drift is still restrained motion.
10. Do not infer anything about sound, music, beats or speech.
11. Copy up to 8 colors from the evidence palette into approximate_palette_evidence (hex and \
prevalence only). Do not invent colors.
12. Provenance is optional: cite sample ids, metric keys and time ranges only. No explanations. \
Use metric keys exactly as listed; do not append distribution fields such as .mean or .p90.
13. List visible traits the engine cannot render in unsupported_reference_traits.
14. Copy reference_fingerprint from the request.
15. Return ONLY one JSON object valid against the response schema (version \"0.2\"). No prose, \
no markdown fences.

Visual construction language (`visual_language`), answer from what the samples show:
- Is typography carrying most of the visual story (type_only / type_led), or do images, \
objects, diagrams, collage or interfaces carry it? -> medium.
- Are images/objects/diagrams present in most scenes, about half, occasionally, never? -> \
asset_usage.
- Does type or the visual material take most of the attention? -> type_image_balance.
- What kind of visual material dominates (photographs, cut-outs, illustration, diagrams, \
isolated objects, collage, UI, clean generated shapes)? -> asset_character.
- What roles does the material play (a hero object, a figure, background environment, \
evidence, supporting objects, transition objects)? -> asset_roles.
- Which of the engine's composition families (listed in the vocabulary) most resemble how \
scenes are built, which fit somewhat, which are clearly absent? -> composition_language.
- Are ideas shown literally, as diagrams/processes, through symbols, through metaphors, or \
through evidence? -> explanation_mode.
Do NOT say what image the engine should make for any new topic, and do not describe particular \
objects, characters or scenes: the engine plans its own visuals from the new story.";

/// Heuristics for reading the evidence (advice, not rules).
pub const EVIDENCE_GUIDE: &str = "\
- polarity: color.polarity, dark_frame_fraction, light_frame_fraction. \"mixed\" only when both \
fractions are at least 0.25.
- temperature: color.temperature and warm_cool_balance (|balance| < 0.08 is neutral).
- palette_character: color.saturation, accent_prevalence, palette; several large saturated \
swatches = multicolor; one small saturated swatch = restrained_accent or vivid.
- composition_rhythm: temporal.changes_per_10s (cuts, transitions and slow evolutions all \
count), then what the samples show. Calibrated on engine renders of known rhythm: \
slow_breathing < 4 changes per 10 s; measured_editorial 4-8.5; progressive 8.5-11; active \
11-15; high_frequency > 15.
- transition_character: hard_cut_fraction high = hard; short, spiky changes with graphic \
shapes = kinetic or geometric; long low-spike changes = subtle or cinematic.
- motion_temperament: use neighboring high_motion samples to compare element displacement, \
scale change and settling. Snappy means quick decisive but contained arrivals; energetic means \
large travel, punches or broad active recomposition. Energetic motion need not visibly bounce. \
Playful requires evidence of elastic or bouncy movement, not just bright colors. Use \
temporal.activity and high_motion_fraction as supporting evidence, not a label lookup. \
If sampling cannot establish the distinction, lower confidence or use null.
- layer_activity: temporal.background_activity versus foreground_activity.
- visual_density: complexity.edge_density, occupied_ratio, large_region_count.";

/// Build the request for `evidence`.
pub fn interpreter_request(
    evidence: &ReferenceEvidence,
    contact_sheet: &str,
) -> InterpreterRequest {
    InterpreterRequest {
        protocol: PROTOCOL.to_string(),
        analyzer_version: evidence.analyzer_version.clone(),
        reference_fingerprint: evidence.reference_fingerprint.clone(),
        instructions: INSTRUCTIONS.to_string(),
        metadata: evidence.metadata.clone(),
        samples: evidence
            .samples
            .iter()
            .map(|s| RequestSample {
                id: s.id.clone(),
                time_seconds: s.time_seconds,
                kinds: s.kinds.clone(),
                image: s.image.clone(),
            })
            .collect(),
        contact_sheet: contact_sheet.to_string(),
        evidence_file: "evidence.json".to_string(),
        evidence_metrics: EVIDENCE_METRICS.iter().map(|m| m.to_string()).collect(),
        max_repair_attempts: MAX_REPAIR_ATTEMPTS,
        response_schema: response_schema(),
    }
}

/// The vocabulary per dimension, derived from the response schema (so the
/// prompt can never drift from the types): `(dimension, [(value, description)])`.
pub fn vocabulary() -> Vec<(String, Vec<(String, String)>)> {
    let schema = response_schema();
    let defs = schema.get("$defs").cloned().unwrap_or(Value::Null);
    let props = schema.get("properties").cloned().unwrap_or(Value::Null);
    let resolve = |v: &Value| -> Value {
        match v.get("$ref").and_then(Value::as_str) {
            Some(r) => defs
                .get(r.trim_start_matches("#/$defs/"))
                .cloned()
                .unwrap_or(Value::Null),
            None => v.clone(),
        }
    };
    let enum_values = |def: &Value| -> Vec<(String, String)> {
        let def = resolve(def);
        def.get("oneOf")
            .and_then(Value::as_array)
            .map(|alts| {
                // Documented variants are `{const, description}`; undocumented
                // ones are grouped by schemars as `{enum: [...]}`.
                alts.iter()
                    .flat_map(|a| {
                        let d = a.get("description").and_then(Value::as_str).unwrap_or("");
                        let consts: Vec<String> = match a.get("const").and_then(Value::as_str) {
                            Some(c) => vec![c.to_string()],
                            None => a
                                .get("enum")
                                .and_then(Value::as_array)
                                .map(|vs| {
                                    vs.iter()
                                        .filter_map(|v| v.as_str().map(String::from))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        };
                        consts.into_iter().map(move |c| (c, d.to_string()))
                    })
                    .collect()
            })
            .or_else(|| {
                def.get("enum").and_then(Value::as_array).map(|vs| {
                    vs.iter()
                        .filter_map(|v| v.as_str().map(|s| (s.to_string(), String::new())))
                        .collect()
                })
            })
            .unwrap_or_default()
    };
    // Trait<T>.value is `anyOf [ {$ref: T}, {type: null} ]`.
    let value_enum = |trait_def: &Value| -> Vec<(String, String)> {
        let t = resolve(trait_def);
        let value = t
            .pointer("/properties/value/anyOf")
            .and_then(Value::as_array)
            .and_then(|alts| alts.iter().find(|a| a.get("$ref").is_some()).cloned())
            .unwrap_or(Value::Null);
        enum_values(&value)
    };
    let mut out = Vec::new();
    for &dim in super::profile::DIMENSIONS {
        let Some(p) = props.get(dim) else { continue };
        let trait_ref = p
            .get("anyOf")
            .and_then(Value::as_array)
            .and_then(|alts| alts.iter().find(|a| a.get("$ref").is_some()).cloned())
            .unwrap_or_else(|| p.clone());
        if dim == "layer_activity" {
            let t = resolve(&trait_ref);
            let inner = t
                .pointer("/properties/value/anyOf")
                .and_then(Value::as_array)
                .and_then(|alts| alts.iter().find(|a| a.get("$ref").is_some()).cloned())
                .map(|r| resolve(&r))
                .unwrap_or(Value::Null);
            for part in ["foreground", "midground", "background"] {
                let part_def = inner
                    .pointer(&format!("/properties/{part}/anyOf"))
                    .and_then(Value::as_array)
                    .and_then(|alts| alts.iter().find(|a| a.get("$ref").is_some()).cloned())
                    .unwrap_or(Value::Null);
                out.push((format!("layer_activity.{part}"), enum_values(&part_def)));
            }
            continue;
        }
        out.push((dim.to_string(), value_enum(&trait_ref)));
    }
    // (v0.2) visual construction language.
    for (key, def) in [
        ("visual_language.medium", "VisualMedium"),
        ("visual_language.asset_usage", "AssetUsage"),
        ("visual_language.type_image_balance", "TypeImageBalance"),
        ("visual_language.asset_character", "AssetCharacter"),
        ("visual_language.asset_roles.role", "RefAssetRole"),
        (
            "visual_language.composition_language.(preferred|secondary|avoid)",
            "RefGrammar",
        ),
        ("visual_language.explanation_mode", "ExplanationMode"),
    ] {
        let d = defs.get(def).cloned().unwrap_or(Value::Null);
        out.push((key.to_string(), enum_values(&d)));
    }
    let traits = defs
        .get("UnsupportedTraitKind")
        .cloned()
        .unwrap_or(Value::Null);
    out.push((
        "unsupported_reference_traits.trait".to_string(),
        enum_values(&traits),
    ));
    out
}

/// `interpreter-prompt.md`: rules, vocabulary, samples and the full evidence
/// as one text prompt (attach the sample images alongside).
pub fn interpreter_prompt(req: &InterpreterRequest, evidence: &ReferenceEvidence) -> String {
    let mut s = String::new();
    s.push_str("# Reference style interpretation\n\n");
    s.push_str(&format!(
        "protocol: {}  \nreference_fingerprint: {}\n\n",
        req.protocol, req.reference_fingerprint
    ));
    s.push_str(INSTRUCTIONS);
    s.push_str("\n\n## Reading the evidence\n\n");
    s.push_str(EVIDENCE_GUIDE);
    s.push_str("\n\n## Vocabulary\n\n");
    for (dim, values) in vocabulary() {
        let vs: Vec<String> = values
            .iter()
            .map(|(v, d)| {
                if d.is_empty() {
                    format!("`{v}`")
                } else {
                    format!("`{v}` ({})", d.trim_end_matches('.'))
                }
            })
            .collect();
        s.push_str(&format!("- **{dim}**: {}\n", vs.join(", ")));
    }
    s.push_str("\nEvery dimension is `{\"value\": <vocabulary value or null>, \"confidence\": 0..1, \"evidence\": {\"samples\": [...], \"metrics\": [...], \"time_ranges\": [[a, b]]}}` (evidence optional). `layer_activity.value` is `{\"foreground\": ..., \"midground\": ..., \"background\": ...}`. `unsupported_reference_traits` is a list of `{\"trait\": ..., \"confidence\": ...}`. `visual_language` is an object: `medium`, `asset_usage`, `type_image_balance`, `asset_character`, `explanation_mode` are dimensions like the others; `asset_roles` is a list (max 4) of `{\"role\": ..., \"confidence\": ...}`; `composition_language` is `{\"preferred\": [...], \"secondary\": [...], \"avoid\": [...], \"confidence\": 0..1}` (max 3 per list, lists disjoint).\n");
    s.push_str(&format!(
        "\nMetric keys: {}\n",
        EVIDENCE_METRICS
            .iter()
            .map(|m| format!("`{m}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    s.push_str("\n## Samples (attached images)\n\n| id | time (s) | why sampled | file |\n|---|---|---|---|\n");
    for x in &req.samples {
        let kinds: Vec<String> = x
            .kinds
            .iter()
            .map(|k| {
                serde_json::to_value(k)
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default()
            })
            .collect();
        s.push_str(&format!(
            "| {} | {:.2} | {} | {} |\n",
            x.id,
            x.time_seconds,
            kinds.join(", "),
            x.image
        ));
    }
    s.push_str("\n## Deterministic evidence\n\n```json\n");
    s.push_str(&serde_json::to_string_pretty(evidence).expect("evidence serializes"));
    s.push_str("\n```\n\n## Response\n\nReturn ONLY the ReferenceStyleProfile JSON object (version \"0.2\"). The JSON Schema is in interpreter-request.json (`response_schema`).\n");
    s
}

/// The single permitted repair request.
pub fn repair_request(
    reference_fingerprint: Option<&str>,
    previous_response: &str,
    issues: &[ProfileIssue],
) -> RepairRequest {
    RepairRequest {
        protocol: PROTOCOL.to_string(),
        repair_attempt: MAX_REPAIR_ATTEMPTS,
        reference_fingerprint: reference_fingerprint.map(str::to_string),
        instructions: "Your previous response was not a valid ReferenceStyleProfile (v0.2). Fix \
exactly the listed issues and return ONLY the corrected JSON object. Keep every other value \
unchanged. Use only vocabulary values; use null for anything you cannot support. This is the \
only repair attempt."
            .to_string(),
        issues: issues.iter().map(ToString::to_string).collect(),
        previous_response: previous_response.to_string(),
        response_schema: response_schema(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_covers_every_dimension_from_the_schema() {
        let vocab = vocabulary();
        let get = |d: &str| {
            vocab
                .iter()
                .find(|(k, _)| k == d)
                .map(|(_, v)| v.iter().map(|(x, _)| x.as_str()).collect::<Vec<_>>())
                .unwrap_or_default()
        };
        assert_eq!(get("polarity"), ["light", "dark", "mixed"]);
        assert!(get("motion_temperament").contains(&"snappy"));
        assert!(get("transition_character").contains(&"continuous"));
        assert!(get("layer_activity.background").contains(&"structured"));
        assert!(get("unsupported_reference_traits.trait").contains(&"particle_field"));
        for (dim, values) in &vocab {
            assert!(values.len() >= 3, "{dim} has {} values", values.len());
        }
        assert_eq!(
            vocab.len(),
            super::super::profile::DIMENSIONS.len() + 2 + 1 + 7
        );
        assert!(get("visual_language.medium").contains(&"diagrammatic"));
    }
}
