//! Reference coverage (0.7): what a ReferenceStyleProfile asked for versus
//! what the TasteDirector resolved, per dimension. Deterministic, no
//! aesthetic score: MATCH / PARTIAL / OVERRIDDEN / UNSUPPORTED / UNKNOWN.

use serde::Serialize;
use serde_json::Value;

use super::normalize::{DimensionMapping, Fidelity, NormalizedReference};
use super::profile::UnsupportedTraitKind;
use crate::compiler::taste::{PaletteFamily, ResolvedStyleProfile};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageStatus {
    /// Applied exactly and present in the resolved style.
    Match,
    /// Applied as the closest supported concept, or expressed only partly.
    Partial,
    /// Applied, but a higher-precedence input (explicit style field, brand
    /// constraint, legibility rule) resolved a different value.
    Overridden,
    /// The engine has no such concept; nothing applied.
    Unsupported,
    /// The profile did not say (null, absent or low confidence).
    Unknown,
}

impl CoverageStatus {
    pub fn label(self) -> &'static str {
        match self {
            CoverageStatus::Match => "MATCH",
            CoverageStatus::Partial => "PARTIAL",
            CoverageStatus::Overridden => "OVERRIDDEN",
            CoverageStatus::Unsupported => "UNSUPPORTED",
            CoverageStatus::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverageRow {
    pub dimension: &'static str,
    /// Reference value (profile vocabulary).
    pub reference: Option<String>,
    pub confidence: Option<f32>,
    /// Engine concept the reference mapped to.
    pub applied: Option<String>,
    /// What the TasteDirector resolved for this dimension.
    pub resolved: String,
    pub status: CoverageStatus,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoverageReport {
    pub rows: Vec<CoverageRow>,
    /// Dimensions with a usable reference value the engine can express
    /// (match + partial + overridden).
    pub supported: Vec<&'static str>,
    pub matched: Vec<&'static str>,
    pub partial: Vec<&'static str>,
    pub overridden: Vec<&'static str>,
    pub unsupported: Vec<&'static str>,
    pub unknown: Vec<&'static str>,
    /// Traits recorded as not renderable (profile + implied by mapping).
    pub unsupported_traits: Vec<UnsupportedTraitKind>,
}

fn name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::new(),
    }
}

fn resolved_value(dim: &str, r: &ResolvedStyleProfile) -> String {
    match dim {
        "tone" => name(&r.tone),
        "polarity" => name(&r.palette.polarity),
        "temperature" => name(&r.palette.temperature),
        // Curated palettes always keep legible text contrast.
        "contrast" => "high".to_string(),
        "palette_character" => name(&r.palette.family),
        "background_character" => name(&r.background),
        "typography_character" | "typography_contrast" => name(&r.typography),
        "material_character" => name(&r.material),
        "image_treatment" => name(&r.image_treatment),
        "visual_density" => name(&r.density.level),
        "composition_rhythm" => name(&r.rhythm),
        "motion_temperament" => name(&r.motion.kind),
        "transition_character" => name(&r.transition.family),
        "scale_contrast" => name(&r.scale),
        "layer_activity" => format!(
            "foreground={} midground={} background={}",
            name(&r.layers.foreground),
            name(&r.layers.midground),
            name(&r.layers.background)
        ),
        "visual.medium" => name(&r.visual.medium),
        "visual.asset_usage" => r.visual.usage.as_ref().map(name).unwrap_or_default(),
        "visual.type_image_balance" => r.visual.balance.as_ref().map(name).unwrap_or_default(),
        "visual.asset_character" => r.visual.character.as_ref().map(name).unwrap_or_default(),
        "visual.asset_roles" => r
            .visual
            .roles
            .iter()
            .map(name)
            .collect::<Vec<_>>()
            .join("+"),
        "visual.composition_language" => crate::reference::normalize::composition_lists(
            &r.visual.preferred,
            &r.visual.secondary,
            &r.visual.avoid,
        ),
        "visual.explanation_mode" => r.visual.explanation.as_ref().map(name).unwrap_or_default(),
        "accent_hint" => format!(
            "#{:02X}{:02X}{:02X}",
            r.palette.colors.accent.r, r.palette.colors.accent.g, r.palette.colors.accent.b
        ),
        _ => String::new(),
    }
}

/// Is the applied engine value what the resolver produced?
fn applied_holds(m: &DimensionMapping, n: &NormalizedReference, r: &ResolvedStyleProfile) -> bool {
    let p = &n.principles;
    match m.dimension {
        "palette_character" => match p.color_fields {
            Some(fields) => {
                let print = matches!(
                    r.palette.family,
                    PaletteFamily::PrintBright | PaletteFamily::PrintDark
                );
                fields == print
            }
            None => true,
        },
        "layer_activity" => {
            p.layers.foreground.is_none_or(|v| v == r.layers.foreground)
                && p.layers.midground.is_none_or(|v| v == r.layers.midground)
                && p.layers.background.is_none_or(|v| v == r.layers.background)
        }
        // The accent follows unless an explicit accent role replaced it; the
        // coverage cannot see the style, so it reports the accent as partial.
        "accent_hint" | "contrast" => true,
        dim => m.engine.as_deref() == Some(resolved_value(dim, r).as_str()),
    }
}

/// Compare a normalized reference with the resolved style.
pub fn coverage(n: &NormalizedReference, resolved: &ResolvedStyleProfile) -> CoverageReport {
    let mut rows = Vec::new();
    for m in &n.dimensions {
        let status = match m.fidelity {
            Fidelity::Unknown | Fidelity::LowConfidence => CoverageStatus::Unknown,
            Fidelity::Unsupported => CoverageStatus::Unsupported,
            _ if m.engine.is_some() && !applied_holds(m, n, resolved) => CoverageStatus::Overridden,
            Fidelity::Exact if m.dimension == "accent_hint" => CoverageStatus::Partial,
            Fidelity::Exact => CoverageStatus::Match,
            Fidelity::Closest | Fidelity::Partial => CoverageStatus::Partial,
        };
        rows.push(CoverageRow {
            dimension: m.dimension,
            reference: m.reference.clone(),
            confidence: m.confidence,
            applied: m.engine.clone(),
            resolved: resolved_value(m.dimension, resolved),
            status,
        });
    }
    let pick = |s: &[CoverageStatus]| -> Vec<&'static str> {
        rows.iter()
            .filter(|r| s.contains(&r.status))
            .map(|r| r.dimension)
            .collect()
    };
    use CoverageStatus::*;
    CoverageReport {
        supported: pick(&[Match, Partial, Overridden]),
        matched: pick(&[Match]),
        partial: pick(&[Partial]),
        overridden: pick(&[Overridden]),
        unsupported: pick(&[Unsupported]),
        unknown: pick(&[Unknown]),
        unsupported_traits: n.unsupported.clone(),
        rows,
    }
}

impl CoverageReport {
    /// Fixed-width text table (CLI output).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for r in &self.rows {
            s.push_str(&format!(
                "{:<22} {:<12} {:<18} -> {:<28} {}\n",
                r.dimension,
                r.status.label(),
                r.reference.as_deref().unwrap_or("-"),
                r.resolved,
                r.applied
                    .as_deref()
                    .map(|a| format!("(applied {a})"))
                    .unwrap_or_default()
            ));
        }
        let traits: Vec<String> = self.unsupported_traits.iter().map(name).collect();
        s.push_str(&format!(
            "matched {} · partial {} · overridden {} · unsupported {} · unknown {}\n",
            self.matched.len(),
            self.partial.len(),
            self.overridden.len(),
            self.unsupported.len(),
            self.unknown.len()
        ));
        s.push_str(&format!(
            "unsupported reference traits: {}\n",
            if traits.is_empty() {
                "none".to_string()
            } else {
                traits.join(", ")
            }
        ));
        s
    }
}
