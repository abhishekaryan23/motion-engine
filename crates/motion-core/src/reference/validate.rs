//! ReferenceStyleProfile parsing and validation (0.7).
//!
//! `parse_profile` = alias folding → strict parse (`deny_unknown_fields`,
//! closed enums) → rule checks. Invalid documents are rejected with issues;
//! nothing is silently changed except the documented alias table. The
//! interpreter protocol allows ONE repair request (`bundle::repair_request`).

use serde::Serialize;
use serde_json::Value;

use super::evidence::{ReferenceEvidence, EVIDENCE_METRICS};
use super::normalize::{normalize_aliases, AliasNote};
use super::oklab;
use super::profile::*;
use super::visual_language::{RefGrammar, VisualLanguageProfile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProfileIssue {
    /// Dotted path (`motion_temperament.confidence`), `$` for the document.
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for ProfileIssue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedProfile {
    pub profile: ReferenceStyleProfile,
    /// Alias rewrites applied before the strict parse.
    pub aliases: Vec<AliasNote>,
}

fn issue(path: impl Into<String>, message: impl Into<String>) -> ProfileIssue {
    ProfileIssue {
        path: path.into(),
        message: message.into(),
    }
}

/// Parse interpreter output. `evidence` (when available) enables provenance
/// checks: fingerprint, sample ids, time ranges.
pub fn parse_profile(
    json: &str,
    evidence: Option<&ReferenceEvidence>,
) -> Result<ParsedProfile, Vec<ProfileIssue>> {
    let mut doc: Value =
        serde_json::from_str(json).map_err(|e| vec![issue("$", format!("not JSON: {e}"))])?;
    let aliases = normalize_aliases(&mut doc);
    let schema_err =
        |e: serde_json::Error| vec![issue("$", format!("does not match the schema: {e}"))];
    // The version selects the (strict) contract. v0.1 is frozen: it cannot carry v0.2-only fields.
    let profile: ReferenceStyleProfile = match doc.get("version").and_then(Value::as_str) {
        Some(REFERENCE_PROFILE_VERSION_V0_1) => {
            let legacy: super::profile_v0_1::ReferenceStyleProfile =
                serde_json::from_value(doc).map_err(schema_err)?;
            legacy.into()
        }
        Some(REFERENCE_PROFILE_VERSION) => serde_json::from_value(doc).map_err(schema_err)?,
        _ => {
            return Err(vec![issue(
                "version",
                format!(
                "must be \"{REFERENCE_PROFILE_VERSION}\" (or \"{REFERENCE_PROFILE_VERSION_V0_1}\")"
            ),
            )])
        }
    };
    let issues = validate_profile(&profile, evidence);
    if issues.is_empty() {
        Ok(ParsedProfile { profile, aliases })
    } else {
        Err(issues)
    }
}

/// Rules JSON types cannot carry. Empty = valid.
pub fn validate_profile(
    p: &ReferenceStyleProfile,
    evidence: Option<&ReferenceEvidence>,
) -> Vec<ProfileIssue> {
    let mut out = Vec::new();
    if p.version != REFERENCE_PROFILE_VERSION && p.version != REFERENCE_PROFILE_VERSION_V0_1 {
        out.push(issue(
            "version",
            format!(
                "must be \"{REFERENCE_PROFILE_VERSION}\" (or \"{REFERENCE_PROFILE_VERSION_V0_1}\")"
            ),
        ));
    }
    if p.version == REFERENCE_PROFILE_VERSION_V0_1 && p.visual_language.is_some() {
        out.push(issue("visual_language", "requires version \"0.2\""));
    }
    if let Some(fp) = &p.reference_fingerprint {
        let well_formed = fp.len() == 20
            && fp.starts_with("rf1-")
            && fp[4..].bytes().all(|b| b.is_ascii_hexdigit());
        if !well_formed {
            out.push(issue(
                "reference_fingerprint",
                "must look like rf1-<16 hex>",
            ));
        } else if let Some(ev) = evidence {
            if *fp != ev.reference_fingerprint {
                out.push(issue(
                    "reference_fingerprint",
                    format!("does not match the evidence ({})", ev.reference_fingerprint),
                ));
            }
        }
    }

    macro_rules! dims {
        ($($f:ident),*) => {$(
            if let Some(t) = &p.$f {
                check_dim(&mut out, evidence, stringify!($f), t.confidence, &t.evidence);
            }
        )*};
    }
    dims!(
        tone,
        polarity,
        temperature,
        contrast,
        palette_character,
        background_character,
        typography_character,
        typography_contrast,
        material_character,
        image_treatment,
        visual_density,
        composition_rhythm,
        motion_temperament,
        transition_character,
        scale_contrast,
        layer_activity
    );

    if let Some(vl) = &p.visual_language {
        check_visual_language(&mut out, vl, evidence);
    }

    if p.approximate_palette_evidence.len() > 8 {
        out.push(issue("approximate_palette_evidence", "at most 8 swatches"));
    }
    for (i, s) in p.approximate_palette_evidence.iter().enumerate() {
        if oklab::parse_hex(&s.hex).is_none() {
            out.push(issue(
                format!("approximate_palette_evidence[{i}].hex"),
                "must be #RRGGBB",
            ));
        }
        if !(0.0..=1.0).contains(&s.prevalence) {
            out.push(issue(
                format!("approximate_palette_evidence[{i}].prevalence"),
                "must be within 0..1",
            ));
        }
    }

    let mut seen = Vec::new();
    for (i, t) in p.unsupported_reference_traits.iter().enumerate() {
        let path = format!("unsupported_reference_traits[{i}]");
        check_confidence(&mut out, &format!("{path}.confidence"), t.confidence);
        if let Some(l) = &t.evidence {
            check_links(&mut out, &format!("{path}.evidence"), l, evidence);
        }
        if seen.contains(&t.kind) {
            out.push(issue(format!("{path}.trait"), "listed twice"));
        }
        seen.push(t.kind);
    }
    out
}

fn check_dim(
    out: &mut Vec<ProfileIssue>,
    evidence: Option<&ReferenceEvidence>,
    dim: &str,
    confidence: f32,
    links: &Option<EvidenceLinks>,
) {
    check_confidence(out, &format!("{dim}.confidence"), confidence);
    if let Some(l) = links {
        check_links(out, &format!("{dim}.evidence"), l, evidence);
    }
}

/// Rules for the v0.2 `visual_language` section (paths start with `visual_language.`).
fn check_visual_language(
    out: &mut Vec<ProfileIssue>,
    vl: &VisualLanguageProfile,
    evidence: Option<&ReferenceEvidence>,
) {
    macro_rules! traits {
        ($($f:ident),*) => {$(
            if let Some(t) = &vl.$f {
                check_dim(
                    out,
                    evidence,
                    concat!("visual_language.", stringify!($f)),
                    t.confidence,
                    &t.evidence,
                );
            }
        )*};
    }
    traits!(
        medium,
        asset_usage,
        type_image_balance,
        asset_character,
        explanation_mode
    );

    if vl.asset_roles.len() > MAX_ASSET_ROLES {
        out.push(issue(
            "visual_language.asset_roles",
            format!("at most {MAX_ASSET_ROLES} roles"),
        ));
    }
    for (i, r) in vl.asset_roles.iter().enumerate() {
        check_confidence(
            out,
            &format!("visual_language.asset_roles[{i}].confidence"),
            r.confidence,
        );
        if vl.asset_roles[..i].iter().any(|x| x.role == r.role) {
            out.push(issue(
                format!("visual_language.asset_roles[{i}].role"),
                "listed twice",
            ));
        }
    }

    if let Some(c) = &vl.composition_language {
        let base = "visual_language.composition_language";
        check_confidence(out, &format!("{base}.confidence"), c.confidence);
        if let Some(l) = &c.evidence {
            check_links(out, &format!("{base}.evidence"), l, evidence);
        }
        let lists: [(&str, &Vec<RefGrammar>); 3] = [
            ("preferred", &c.preferred),
            ("secondary", &c.secondary),
            ("avoid", &c.avoid),
        ];
        for (name, list) in lists {
            if list.len() > MAX_COMPOSITION_LIST {
                out.push(issue(
                    base,
                    format!("{name}: at most {MAX_COMPOSITION_LIST} entries"),
                ));
            }
            if list.iter().enumerate().any(|(i, g)| list[..i].contains(g)) {
                out.push(issue(base, format!("{name}: lists a family twice")));
            }
        }
        for (a, (an, al)) in lists.iter().enumerate() {
            for (bn, bl) in &lists[a + 1..] {
                if al.iter().any(|g| bl.contains(g)) {
                    out.push(issue(
                        base,
                        format!("{an} and {bn} must not share a family"),
                    ));
                }
            }
        }
    }
}

const MAX_ASSET_ROLES: usize = 4;
const MAX_COMPOSITION_LIST: usize = 3;

fn check_confidence(out: &mut Vec<ProfileIssue>, path: &str, c: f32) {
    if !c.is_finite() || !(0.0..=1.0).contains(&c) {
        out.push(issue(path, "must be a number within 0..1"));
    }
}

fn check_links(
    out: &mut Vec<ProfileIssue>,
    path: &str,
    l: &EvidenceLinks,
    evidence: Option<&ReferenceEvidence>,
) {
    for s in &l.samples {
        let known = match evidence {
            Some(ev) => ev.samples.iter().any(|x| &x.id == s),
            None => {
                s.len() >= 3 && s.starts_with('s') && s[1..].bytes().all(|b| b.is_ascii_digit())
            }
        };
        if !known {
            out.push(issue(
                format!("{path}.samples"),
                format!("unknown sample id {s:?}"),
            ));
        }
    }
    for m in &l.metrics {
        if !EVIDENCE_METRICS.contains(&m.as_str()) {
            out.push(issue(
                format!("{path}.metrics"),
                format!("unknown metric {m:?}"),
            ));
        }
    }
    let end = evidence.map(|e| e.metadata.duration_seconds + 0.05);
    for r in &l.time_ranges {
        let ok = r[0].is_finite()
            && r[1].is_finite()
            && 0.0 <= r[0]
            && r[0] <= r[1]
            && end.is_none_or(|e| r[1] <= e);
        if !ok {
            out.push(issue(
                format!("{path}.time_ranges"),
                format!("invalid range [{}, {}]", r[0], r[1]),
            ));
        }
    }
}
