//! Reference commands (0.7): `reference-evidence`, `validate-reference-style`,
//! `resolve-style`, and the shared `--reference-style` loader. No model or
//! network access: the multimodal interpretation happens outside the engine.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use motion_core::compiler::typography::{
    emotion_name, resolve_typography, role_name, TypographyRequest,
};
use motion_core::compiler::{resolve_taste, story_key_of};
use motion_core::reference::bundle::repair_request;
use motion_core::reference::evidence::{ReferenceEvidence, SampleKind};
use motion_core::reference::{
    coverage, normalize, parse_profile, NormalizedReference, ProfileIssue,
};
use motion_core::{CreativeIntent, StyleProfile};
use motion_render::reference::analyze::{
    CONTACT_SHEET_FILE, EVIDENCE_FILE, PROMPT_FILE, REQUEST_FILE,
};
use motion_render::reference::{analyze_reference, cache, AnalysisConfig, Reuse};
use serde_json::{json, Value};

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn issue_lines(issues: &[ProfileIssue]) -> String {
    issues
        .iter()
        .map(|i| format!("  - {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Serialized snake_case name of a unit enum value (a macro so this crate
/// needs no direct serde dependency).
macro_rules! name {
    ($v:expr) => {
        match serde_json::to_value($v) {
            Ok(Value::String(s)) => s,
            Ok(other) => other.to_string(),
            Err(_) => String::new(),
        }
    };
}

/// Load, validate and normalize a ReferenceStyleProfile file (no evidence
/// provenance checks). Invalid profiles are an error listing every issue.
pub fn load_reference(path: &Path) -> Result<NormalizedReference> {
    let text = read(path)?;
    match parse_profile(&text, None) {
        Ok(parsed) => Ok(normalize(&parsed.profile)),
        Err(issues) => bail!(
            "invalid ReferenceStyleProfile {} ({} issue(s)):\n{}",
            path.display(),
            issues.len(),
            issue_lines(&issues)
        ),
    }
}

// ---------------------------------------------------------------------------
// reference-evidence
// ---------------------------------------------------------------------------

pub fn cmd_reference_evidence(
    video: &Path,
    out_dir: &Path,
    cache_dir: Option<&Path>,
    samples: Option<usize>,
    json_out: bool,
) -> Result<()> {
    if !video.is_file() {
        bail!("video '{}' not found", video.display());
    }
    let config = AnalysisConfig {
        sample_budget: samples,
        ..AnalysisConfig::default()
    };
    let outcome = analyze_reference(video, out_dir, &config, cache_dir)
        .with_context(|| format!("analysing {}", video.display()))?;
    let ev = &outcome.evidence;
    if json_out {
        println!("{}", serde_json::to_string_pretty(ev)?);
        return Ok(());
    }
    print!("{}", evidence_summary(ev));
    println!(
        "reuse:        {}",
        match outcome.reused {
            Reuse::Fresh => "fresh analysis",
            Reuse::OutDir => "out-dir (existing bundle)",
            Reuse::Cache => "cache",
        }
    );
    println!("written to {}:", out_dir.display());
    println!("  {EVIDENCE_FILE}");
    println!("  samples/ ({} image(s))", ev.samples.len());
    println!("  {CONTACT_SHEET_FILE}");
    println!("  {REQUEST_FILE}");
    println!("  {PROMPT_FILE}");
    println!(
        "next: send {} and the samples to any multimodal model, then run `validate-reference-style` on its answer.",
        out_dir.join(PROMPT_FILE).display()
    );
    Ok(())
}

fn evidence_summary(ev: &ReferenceEvidence) -> String {
    let m = &ev.metadata;
    let mut kinds: Vec<(SampleKind, usize)> = Vec::new();
    for s in &ev.samples {
        for k in &s.kinds {
            match kinds.iter_mut().find(|(x, _)| x == k) {
                Some((_, n)) => *n += 1,
                None => kinds.push((*k, 1)),
            }
        }
    }
    kinds.sort();
    let by_kind = kinds
        .iter()
        .map(|(k, n)| format!("{} {}", name!(k), n))
        .collect::<Vec<_>>()
        .join(", ");
    let p = &ev.color.polarity;
    format!(
        "reference:    {}\n\
         duration:     {:.2}s, {}x{} @ {:.2} fps ({} frames)\n\
         audio:        {}\n\
         samples:      {} ({})\n\
         changes/10s:  {:.2}\n\
         polarity:     {} (confidence {:.2})\n",
        ev.reference_fingerprint,
        m.duration_seconds,
        m.width,
        m.height,
        m.fps,
        m.frame_count,
        if m.audio_present {
            "present (ignored)"
        } else {
            "none"
        },
        ev.samples.len(),
        if by_kind.is_empty() { "-" } else { &by_kind },
        ev.temporal.changes_per_10s,
        name!(&p.value),
        p.confidence,
    )
}

// ---------------------------------------------------------------------------
// validate-reference-style
// ---------------------------------------------------------------------------

pub fn cmd_validate_reference_style(
    profile: &Path,
    bundle: Option<&Path>,
    repair_out: Option<&Path>,
    cache_dir: Option<&Path>,
    json_out: bool,
) -> Result<()> {
    let text = read(profile)?;
    let evidence: Option<ReferenceEvidence> = match bundle {
        Some(dir) => {
            let path = dir.join(EVIDENCE_FILE);
            Some(
                serde_json::from_str(&read(&path)?)
                    .with_context(|| format!("parsing {}", path.display()))?,
            )
        }
        None => None,
    };

    let parsed = match parse_profile(&text, evidence.as_ref()) {
        Ok(p) => p,
        Err(issues) => {
            if json_out {
                let v = json!({
                    "valid": false,
                    "issues": issues,
                    "aliases": [],
                    "normalized": null,
                });
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("invalid ReferenceStyleProfile ({} issue(s)):", issues.len());
                for i in &issues {
                    println!("  {i}");
                }
            }
            if let Some(out) = repair_out {
                let fingerprint = serde_json::from_str::<Value>(&text).ok().and_then(|v| {
                    v.get("reference_fingerprint")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                let req = repair_request(fingerprint.as_deref(), &text, &issues);
                if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(out, serde_json::to_string_pretty(&req)? + "\n")
                    .with_context(|| format!("writing {}", out.display()))?;
                if !json_out {
                    println!("repair request: {}", out.display());
                }
            }
            bail!(
                "{} is not a valid ReferenceStyleProfile ({} issue(s))",
                profile.display(),
                issues.len()
            );
        }
    };

    let normalized = normalize(&parsed.profile);

    match (cache_dir, evidence.as_ref()) {
        (Some(cache_dir), Some(ev)) => {
            let canonical = serde_json::to_string_pretty(&parsed.profile)? + "\n";
            cache::store_profile(cache_dir, &ev.reference_fingerprint, &canonical)
                .context("caching the validated profile")?;
        }
        (Some(_), None) => eprintln!("note: --cache-dir needs --bundle to cache the profile"),
        _ => {}
    }

    if json_out {
        let v = json!({
            "valid": true,
            "issues": [],
            "aliases": parsed.aliases,
            "visual_policy": visual_policy(&normalized.principles.visual),
            "normalized": normalized,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    println!("valid ReferenceStyleProfile v{}", parsed.profile.version);
    if !parsed.aliases.is_empty() {
        println!("aliases:");
        for a in &parsed.aliases {
            println!(
                "  {}: {} -> {}",
                a.path,
                a.from,
                a.to.as_deref().unwrap_or("null")
            );
        }
    }
    println!(
        "{:<24} {:<20} {:<6} {:<20} fidelity",
        "dimension", "reference", "conf", "engine"
    );
    for d in &normalized.dimensions {
        println!(
            "{:<24} {:<20} {:<6} {:<20} {}",
            d.dimension,
            d.reference.as_deref().unwrap_or("-"),
            d.confidence
                .map(|c| format!("{c:.2}"))
                .unwrap_or_else(|| "-".into()),
            d.engine.as_deref().unwrap_or("-"),
            name!(&d.fidelity)
        );
    }
    let traits: Vec<String> = normalized.unsupported.iter().map(|t| name!(t)).collect();
    println!(
        "unsupported traits: {}",
        if traits.is_empty() {
            "none".to_string()
        } else {
            traits.join(", ")
        }
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// resolve-style
// ---------------------------------------------------------------------------

pub fn cmd_resolve_style(
    intent: &Path,
    style: Option<&Path>,
    reference: Option<&Path>,
    json_out: bool,
    opts: &motion_core::compiler::CompileOptions,
    assets: &Path,
) -> Result<()> {
    let intent = CreativeIntent::from_json(&read(intent)?)
        .with_context(|| format!("parsing intent {}", intent.display()))?;
    intent
        .validate()
        .map_err(|issues| anyhow::anyhow!("invalid intent: {issues:?}"))?;
    let style = match style {
        Some(p) => StyleProfile::from_json(&read(p)?)
            .with_context(|| format!("parsing style {}", p.display()))?,
        None => StyleProfile::default(),
    };
    let normalized = reference.map(load_reference).transpose()?;
    let resolved = resolve_taste(&intent, &style, normalized.as_ref().map(|n| &n.principles));
    let report = normalized.as_ref().map(|n| coverage(n, &resolved));
    let (choice, fonts) = resolve_typography(
        &resolved,
        &TypographyRequest {
            exploration: opts.explore,
            seed: opts.seed,
            story_key: story_key_of(&intent),
            emotion: opts.emotion,
        },
        assets,
    );
    let faces: BTreeMap<&str, &str> = fonts
        .roles
        .iter()
        .map(|(role, face)| (role_name(*role), face.asset_id))
        .collect();

    if json_out {
        let mut typography = serde_json::to_value(&choice)?;
        typography["faces"] = serde_json::to_value(&faces)?;
        let mut v = json!({
            "resolved": resolved,
            "visual_policy": visual_policy(&resolved.visual),
            "typography": typography,
        });
        if let (Some(n), Some(c)) = (&normalized, &report) {
            v["normalized"] = serde_json::to_value(n)?;
            v["coverage"] = serde_json::to_value(c)?;
        }
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    if let Value::Object(map) = serde_json::to_value(resolved.fingerprint())? {
        for (dimension, value) in map {
            println!("{:<22} {}", dimension, plain(&value));
        }
    }
    let which = match (&choice.option, &choice.legacy) {
        (Some(o), _) => o.clone(),
        (None, Some(l)) => format!("legacy {}", plain(&serde_json::to_value(l)?)),
        (None, None) => "none".to_string(),
    };
    let skipped = if choice.fallback_from.is_empty() {
        String::new()
    } else {
        format!(" (skipped {})", choice.fallback_from.join(", "))
    };
    println!(
        "{:<22} {} [emotion {}]{}",
        "Typography choice",
        which,
        emotion_name(choice.emotion),
        skipped
    );
    if let Some(c) = &report {
        println!();
        print!("{}", c.to_text());
    }
    Ok(())
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        // The density dimension is a struct; its level is the fingerprint value.
        Value::Object(o) => o.get("level").map(plain).unwrap_or_else(|| v.to_string()),
        other => other.to_string(),
    }
}

/// The engine's own derived visual-language policy (compiler::visual), so
/// benchmark scripts never re-implement it.
fn visual_policy(v: &motion_core::compiler::visual::VisualLanguage) -> serde_json::Value {
    json!({
        "weight": v.weight(),
        "prefers_procedural": v.prefers_procedural(),
        "wants_images": v.wants_images(),
    })
}
