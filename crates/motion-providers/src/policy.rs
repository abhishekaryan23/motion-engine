//! Owner spend rules as data (`providers.policy.json`) and the spend log.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::jsonx::norm;
use crate::provider::{AssetKind, AssetRequest};

pub const POLICY_FILE: &str = "providers.policy.json";
pub const SPEND_LOG_FILE: &str = "spend.jsonl";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowRule {
    pub kind: AssetKind,
    /// Matched against the request model, ignoring case and punctuation
    /// (`gpt-image-2.5-flare` matches `openai/GPT Image 2.5 Flare`).
    pub model: String,
    /// Required quality (e.g. `low`); `None` = any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Policy {
    pub allow: Vec<AllowRule>,
    /// Credits one request may cost.
    pub max_per_request: f64,
    /// Credits one video (`--video`) may spend in total.
    pub per_video_cap: f64,
    /// Credits per calendar month (UTC), all videos.
    pub monthly_cap: f64,
    /// Preferred provider order.
    pub provider_order: Vec<String>,
}

impl Default for Policy {
    fn default() -> Self {
        let rule = |kind, model: &str, quality: Option<&str>| AllowRule {
            kind,
            model: model.into(),
            quality: quality.map(str::to_string),
        };
        Self {
            allow: vec![
                rule(AssetKind::Image, "gpt-image-2.5-flare", Some("low")),
                rule(AssetKind::Image, "gpt-image-2.5-sunburst", Some("low")),
                // (v2.3) A local MOSS server: 0 credits, the last resort.
                rule(AssetKind::Image, "moss-image-sft", None),
                // MiniMax H3 Max and PixVerse V6 on OpenArt; PixVerse C1 is allowed
                // ahead of time and works once it appears in the catalog.
                rule(AssetKind::Video, "fal-h3-max", None),
                rule(AssetKind::Video, "pixverseV6", None),
                rule(AssetKind::Video, "pixverse-c1", None),
            ],
            max_per_request: 40.0,
            per_video_cap: 150.0,
            monthly_cap: 1500.0,
            provider_order: vec!["openart".into(), "higgsfield".into(), "moss".into()],
        }
    }
}

impl Policy {
    /// Load the policy; a missing file is written with the defaults.
    pub fn load_or_init(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let p = Self::default();
                if let Some(d) = path.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(path, serde_json::to_string_pretty(&p)? + "\n")?;
                Ok(p)
            }
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn check_model(&self, req: &AssetRequest) -> Result<()> {
        let m = norm(&req.model);
        let ok = self.allow.iter().any(|r| {
            r.kind == req.kind
                && !m.is_empty()
                && m.contains(&norm(&r.model))
                && r.quality
                    .as_ref()
                    .is_none_or(|q| req.quality.as_deref().map(norm) == Some(norm(q)))
        });
        if !ok {
            let list: Vec<String> = self
                .allow
                .iter()
                .filter(|r| r.kind == req.kind)
                .map(|r| match &r.quality {
                    Some(q) => format!("{} ({q})", r.model),
                    None => r.model.clone(),
                })
                .collect();
            bail!(
                "model '{}' quality {:?} is not allowed for {:?}; allowed: {}",
                req.model,
                req.quality.as_deref().unwrap_or("-"),
                req.kind,
                list.join(", ")
            );
        }
        Ok(())
    }

    /// Stop when the quoted credits would break a cap.
    pub fn check_spend(&self, credits: f64, totals: &SpendTotals) -> Result<()> {
        if !credits.is_finite() || credits < 0.0 {
            bail!("unusable quote ({credits}); not spending");
        }
        if credits > self.max_per_request {
            bail!(
                "quote {credits} credits > max_per_request {}; stopped",
                self.max_per_request
            );
        }
        if totals.video + credits > self.per_video_cap {
            bail!(
                "video would spend {} > per_video_cap {}; stopped",
                totals.video + credits,
                self.per_video_cap
            );
        }
        if totals.month + credits > self.monthly_cap {
            bail!(
                "month would spend {} > monthly_cap {}; stopped",
                totals.month + credits,
                self.monthly_cap
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SpendEntry {
    /// Unix seconds.
    pub ts: u64,
    /// `YYYY-MM` (UTC).
    pub month: String,
    pub video: String,
    pub provider: String,
    pub model: String,
    pub credits: f64,
    pub cache_key: String,
    #[serde(default)]
    pub job: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpendTotals {
    pub video: f64,
    pub month: f64,
}

pub struct SpendLog {
    pub path: PathBuf,
}

impl SpendLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn entries(&self) -> Result<Vec<SpendEntry>> {
        let s = match std::fs::read_to_string(&self.path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        s.lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).context("bad spend log line"))
            .collect()
    }

    pub fn totals(&self, video: &str, month: &str) -> Result<SpendTotals> {
        let mut t = SpendTotals::default();
        for e in self.entries()? {
            if e.video == video {
                t.video += e.credits;
            }
            if e.month == month {
                t.month += e.credits;
            }
        }
        Ok(t)
    }

    pub fn append(&self, e: &SpendEntry) -> Result<()> {
        if let Some(d) = self.path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(e)?)?;
        Ok(())
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `YYYY-MM` for unix seconds (UTC, proleptic Gregorian).
pub fn month_of(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_allow_list() {
        let p = Policy::default();
        let r = |model: &str| AssetRequest {
            kind: AssetKind::Video,
            model: model.into(),
            prompt: "p".into(),
            quality: None,
            aspect: None,
            params: Default::default(),
        };
        for ok in ["fal-h3-max", "pixverseV6", "pixverse-c1"] {
            assert!(p.check_model(&r(ok)).is_ok(), "{ok}");
        }
        for no in ["minimax-h3", "pixverseV5", "kling-3"] {
            assert!(p.check_model(&r(no)).is_err(), "{no}");
        }
    }

    #[test]
    fn months() {
        assert_eq!(month_of(0), "1970-01");
        assert_eq!(month_of(1_791_000_000), "2026-10");
        assert_eq!(month_of(951_782_400), "2000-02"); // 2000-02-29
    }
}
