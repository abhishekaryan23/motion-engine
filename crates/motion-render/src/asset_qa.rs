//! Asset preflight QA (0.5): PASS / WARNING / FAIL per delivered image.
//! Checks and thresholds: docs/ASSET_ANALYSIS.md §Preflight.

use std::path::Path;

use motion_core::assets::{
    content_hash, AssetAnalysis, AssetManifest, AssetPromptSet, AssetPromptSpec, Framing,
    ImageFormat, ManifestEntry, NormBox, Presentation, Priority, RegionName,
};
use serde::Serialize;

use crate::analysis::{analyze, AnalyzeOptions};
use crate::decode::{decode_bytes, DecodedImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckLevel {
    Pass,
    Warning,
    Fail,
}

impl CheckLevel {
    /// Upper-case label used in reports.
    pub fn label(self) -> &'static str {
        match self {
            CheckLevel::Pass => "PASS",
            CheckLevel::Warning => "WARNING",
            CheckLevel::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetCheck {
    /// Stable check name (`file`, `decode`, `resolution`, `aspect`, `alpha`,
    /// `manifest`, `fingerprint`, `edges`, `head_headroom`, `occupancy`,
    /// `safe_region`, `face_metadata`, `duplicate`, `delivery`).
    pub name: &'static str,
    pub level: CheckLevel,
    pub message: String,
}

impl AssetCheck {
    pub fn new(name: &'static str, level: CheckLevel, message: impl Into<String>) -> Self {
        AssetCheck {
            name,
            level,
            message: message.into(),
        }
    }
    pub fn pass(name: &'static str, message: impl Into<String>) -> Self {
        Self::new(name, CheckLevel::Pass, message)
    }
    pub fn warn(name: &'static str, message: impl Into<String>) -> Self {
        Self::new(name, CheckLevel::Warning, message)
    }
    pub fn fail(name: &'static str, message: impl Into<String>) -> Self {
        Self::new(name, CheckLevel::Fail, message)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetQa {
    /// Manifest entry id or missing request id.
    pub id: String,
    pub checks: Vec<AssetCheck>,
}

impl AssetQa {
    /// Worst level (Pass when there are no checks).
    pub fn level(&self) -> CheckLevel {
        self.checks
            .iter()
            .map(|c| c.level)
            .max()
            .unwrap_or(CheckLevel::Pass)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct AssetQaReport {
    pub assets: Vec<AssetQa>,
}

impl AssetQaReport {
    pub fn has_fail(&self) -> bool {
        self.assets.iter().any(|a| a.level() == CheckLevel::Fail)
    }

    /// `(assets with a FAIL, assets whose worst level is WARNING)`.
    pub fn counts(&self) -> (usize, usize) {
        let count = |l: CheckLevel| self.assets.iter().filter(|a| a.level() == l).count();
        (count(CheckLevel::Fail), count(CheckLevel::Warning))
    }

    /// Human-readable report (one block per asset: id, then `PASS|WARNING|FAIL name: message`).
    pub fn report(&self) -> String {
        let mut out = String::new();
        for a in &self.assets {
            out.push_str(&format!("{}  {}\n", a.id, a.level().label()));
            for c in &a.checks {
                out.push_str(&format!(
                    "  {:<8} {}: {}\n",
                    c.level.label(),
                    c.name,
                    c.message
                ));
            }
        }
        let (fail, warn) = self.counts();
        out.push_str(&format!(
            "assets: {} ({} fail, {} {})\n",
            self.assets.len(),
            fail,
            warn,
            if warn == 1 { "warning" } else { "warnings" }
        ));
        out
    }
}

/// Framings that depict a person (head/face checks and the head estimate apply).
pub fn is_people_framing(f: Framing) -> bool {
    matches!(
        f,
        Framing::FullFigure | Framing::HalfFigure | Framing::HeadAndShoulders
    )
}

/// Whether the delivered subject is a person: the spec's framing says so, or
/// the entry carries face/head metadata.
pub fn entry_is_person(entry: &ManifestEntry, spec: Option<&AssetPromptSpec>) -> bool {
    spec.map(|s| is_people_framing(s.composition.framing))
        .unwrap_or(false)
        || entry.face_anchor.is_some()
        || entry.face_bounds.is_some()
        || entry.head_bounds.is_some()
}

/// Alpha at or above which a pixel counts as subject (the analysis default).
const SUBJECT_ALPHA: u8 = 16;

fn pixel_coverage(img: &DecodedImage) -> f32 {
    let px = img.pixmap.pixels();
    if px.is_empty() {
        return 0.0;
    }
    let n = px.iter().filter(|p| p.alpha() >= SUBJECT_ALPHA).count();
    n as f32 / px.len() as f32
}

fn pct(v: f32) -> u32 {
    (v * 100.0).round().max(0.0) as u32
}

fn box_inside(inner: NormBox, outer: NormBox) -> bool {
    const EPS: f32 = 1e-4;
    inner.x >= outer.x - EPS
        && inner.y >= outer.y - EPS
        && inner.x + inner.width <= outer.x + outer.width + EPS
        && inner.y + inner.height <= outer.y + outer.height + EPS
}

fn region_name(n: RegionName) -> &'static str {
    match n {
        RegionName::LeftUpper => "left_upper",
        RegionName::RightUpper => "right_upper",
        RegionName::LeftMiddle => "left_middle",
        RegionName::RightMiddle => "right_middle",
        RegionName::LowerLeft => "lower_left",
        RegionName::LowerRight => "lower_right",
        RegionName::Top => "top",
        RegionName::Bottom => "bottom",
    }
}

/// Checks for one delivered entry. `decoded` is `None` when the file could not
/// be read/decoded (the caller records that as `file`/`decode` FAIL).
pub fn check_entry(
    entry: &ManifestEntry,
    spec: Option<&AssetPromptSpec>,
    decoded: Option<&DecodedImage>,
) -> Vec<AssetCheck> {
    let Some(img) = decoded else {
        return vec![AssetCheck::fail("decode", "image could not be decoded")];
    };
    let (w, h) = (img.pixmap.width(), img.pixmap.height());
    let fmt = match img.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
    };
    let mut checks = vec![AssetCheck::pass("decode", format!("{fmt} {w}×{h}"))];

    // manifest: recorded size must match the file.
    if entry.width != w || entry.height != h {
        checks.push(AssetCheck::fail(
            "manifest",
            format!(
                "manifest says {}×{} but the file is {w}×{h}",
                entry.width, entry.height
            ),
        ));
    } else {
        checks.push(AssetCheck::pass("manifest", "size matches the file"));
    }

    // fingerprint: the recorded one must equal the spec's.
    if let Some(spec) = spec {
        match entry.fingerprint.as_deref() {
            Some(fp) if fp != spec.fingerprint => checks.push(AssetCheck::fail(
                "fingerprint",
                format!(
                    "entry fingerprint {fp} differs from spec fingerprint {}",
                    spec.fingerprint
                ),
            )),
            Some(fp) => checks.push(AssetCheck::pass("fingerprint", fp.to_string())),
            None => checks.push(AssetCheck::pass(
                "fingerprint",
                "not recorded in the manifest",
            )),
        }
    }

    // resolution
    let short = w.min(h);
    let min_short = spec.map(|s| s.output.min_short_side).unwrap_or(1024);
    if short < 512 {
        checks.push(AssetCheck::fail(
            "resolution",
            format!("{w}×{h}: short side {short}px is below the 512px minimum"),
        ));
    } else if short < min_short {
        checks.push(AssetCheck::warn(
            "resolution",
            format!("{w}×{h}: short side {short}px is below the requested {min_short}px"),
        ));
    } else {
        checks.push(AssetCheck::pass(
            "resolution",
            format!("{w}×{h} (min {min_short})"),
        ));
    }

    // aspect
    let ratio = w.max(h) as f32 / short.max(1) as f32;
    if ratio > 5.0 {
        checks.push(AssetCheck::fail(
            "aspect",
            format!("aspect {ratio:.1}:1 is extreme (limit 5:1)"),
        ));
    } else if ratio > 3.0 {
        checks.push(AssetCheck::warn(
            "aspect",
            format!("aspect {ratio:.1}:1 is very elongated (prefer under 3:1)"),
        ));
    } else {
        checks.push(AssetCheck::pass("aspect", format!("{ratio:.2}:1")));
    }

    // Analysis: the stored one, else measured from the pixels.
    let analysis: AssetAnalysis = match &entry.analysis {
        Some(a) => a.clone(),
        None => analyze(
            &img.pixmap,
            &AnalyzeOptions {
                person: entry_is_person(entry, spec),
                ..AnalyzeOptions::default()
            },
        ),
    };
    let cutout = img.has_transparency;
    // Coverage is read from the pixels: cheap, and exact even for hand-written analysis.
    let coverage = if cutout { pixel_coverage(img) } else { 1.0 };

    // alpha
    let alpha_required = spec.map(|s| s.output.alpha_required).unwrap_or(false);
    if alpha_required && !cutout {
        checks.push(AssetCheck::fail(
            "alpha",
            "transparent cutout required but the image has no transparency",
        ));
    } else if cutout && coverage > 0.98 {
        checks.push(AssetCheck::fail(
            "alpha",
            format!(
                "alpha covers {}% of the frame: not a real cutout",
                pct(coverage)
            ),
        ));
    } else if cutout && spec.is_some() && !alpha_required {
        checks.push(AssetCheck::warn(
            "alpha",
            "image has transparency but an opaque image was requested",
        ));
    } else if cutout {
        checks.push(AssetCheck::pass(
            "alpha",
            format!("transparent cutout (coverage {}%)", pct(coverage)),
        ));
    } else {
        checks.push(AssetCheck::pass("alpha", "opaque image"));
    }

    // edges (cutouts, with a spec to say what may be cut)
    if let (true, Some(spec)) = (cutout, spec) {
        let e = analysis.edges;
        let c = &spec.composition;
        let mut touched: Vec<&str> = Vec::new();
        if e.top && c.head_inside_frame {
            touched.push("top");
        }
        if c.subject_whole {
            if e.left {
                touched.push("left");
            }
            if e.right {
                touched.push("right");
            }
            if e.bottom {
                touched.push("bottom");
            }
        }
        if touched.is_empty() {
            checks.push(AssetCheck::pass("edges", "subject clear of edges"));
        } else {
            checks.push(AssetCheck::warn(
                "edges",
                format!("subject touches the {} edge", touched.join(", ")),
            ));
        }
    }

    // occupancy (cutouts)
    if cutout {
        let b = analysis.subject_bounds;
        let mut notes: Vec<String> = Vec::new();
        if b.width > 0.9 {
            notes.push(format!("subject occupies {}% width", pct(b.width)));
        }
        if b.height > 0.97 {
            notes.push(format!("subject occupies {}% height", pct(b.height)));
        }
        if coverage > 0.75 {
            notes.push(format!("subject covers {}% of the image", pct(coverage)));
        }
        if notes.is_empty() {
            checks.push(AssetCheck::pass(
                "occupancy",
                format!("subject occupies {}% × {}%", pct(b.width), pct(b.height)),
            ));
        } else {
            checks.push(AssetCheck::warn("occupancy", notes.join("; ")));
        }
    }

    // safe_region
    if cutout {
        let best = analysis
            .safe_regions
            .first()
            .map(|r| region_name(r.name))
            .unwrap_or("none");
        checks.push(AssetCheck::pass(
            "safe_region",
            format!(
                "type placed outside the silhouette by the engine; best in-image region: {best}"
            ),
        ));
    } else if spec.map(|s| s.presentation) == Some(Presentation::BackgroundPlate) {
        // Plates sit behind the type on their own (treated, receding) plane.
        checks.push(AssetCheck::pass(
            "safe_region",
            "background plate: type stays on the engine's planes above it",
        ));
    } else if spec.map(|s| s.presentation) == Some(Presentation::FullFrame) {
        match analysis.safe_regions.iter().find(|r| r.score >= 0.08) {
            Some(r) => checks.push(AssetCheck::pass(
                "safe_region",
                format!("calm region for type: {}", region_name(r.name)),
            )),
            None => checks.push(AssetCheck::warn(
                "safe_region",
                "no calm region (score >= 0.08) for type inside the image",
            )),
        }
    }

    // head_headroom
    let head_expected = spec
        .map(|s| s.composition.head_inside_frame)
        .unwrap_or(false);
    let mut with_analysis = entry.clone();
    with_analysis.analysis = Some(analysis.clone());
    if let Some(head) = with_analysis.head_region() {
        if head_expected || entry_is_person(entry, spec) {
            if head.y < 0.03 {
                checks.push(AssetCheck::warn(
                    "head_headroom",
                    "head is close to the upper edge",
                ));
            } else {
                checks.push(AssetCheck::pass(
                    "head_headroom",
                    format!("headroom {}%", pct(head.y)),
                ));
            }
        }
    }

    // face_metadata: supplied boxes must lie on the subject.
    let sb = analysis.subject_bounds;
    let grown = NormBox {
        x: sb.x - 0.02,
        y: sb.y - 0.02,
        width: sb.width + 0.04,
        height: sb.height + 0.04,
    };
    let bad: Vec<&str> = [
        ("face_bounds", entry.face_bounds),
        ("head_bounds", entry.head_bounds),
    ]
    .into_iter()
    .filter_map(|(name, b)| b.filter(|b| !box_inside(*b, grown)).map(|_| name))
    .collect();
    if !bad.is_empty() {
        checks.push(AssetCheck::warn(
            "face_metadata",
            format!("{} lies outside the subject bounds", bad.join(" and ")),
        ));
    } else if entry.face_bounds.is_some() || entry.head_bounds.is_some() {
        checks.push(AssetCheck::pass(
            "face_metadata",
            "inside the subject bounds",
        ));
    }

    checks
}

/// `AssetManifest::validate()` errors as one report block (`duplicate` /
/// `manifest` FAIL checks), or `None` when the manifest is structurally valid.
pub fn manifest_structure_qa(manifest: &AssetManifest) -> Option<AssetQa> {
    let errs = manifest.validate().err()?;
    let checks = errs
        .into_iter()
        .map(|e| {
            let name = if e.contains("duplicate id") || e.contains("more than one asset") {
                "duplicate"
            } else {
                "manifest"
            };
            AssetCheck::fail(name, e)
        })
        .collect();
    Some(AssetQa {
        id: "manifest".to_string(),
        checks,
    })
}

/// The `delivery` check for a request with no usable image.
pub fn missing_check(priority: Priority, reason: &str) -> AssetCheck {
    match priority {
        Priority::Required => {
            AssetCheck::fail("delivery", format!("required image missing: {reason}"))
        }
        Priority::Optional => AssetCheck::warn(
            "delivery",
            format!("optional image missing: {reason}; falls back to the image-free composition"),
        ),
    }
}

/// Re-validate a manifest from disk (paths relative to `manifest_dir`),
/// optionally against the prompt set it was generated for.
pub fn validate_manifest(
    manifest: &AssetManifest,
    manifest_dir: &Path,
    prompts: Option<&AssetPromptSet>,
) -> AssetQaReport {
    let mut report = AssetQaReport::default();
    if let Some(qa) = manifest_structure_qa(manifest) {
        report.assets.push(qa);
    }

    for entry in &manifest.assets {
        let spec = prompts.and_then(|p| p.for_request(&entry.id));
        let mut checks: Vec<AssetCheck> = Vec::new();
        let path = manifest_dir.join(&entry.path);
        match std::fs::read(&path) {
            Err(e) => checks.push(AssetCheck::fail(
                "file",
                format!("cannot read {}: {e}", path.display()),
            )),
            Ok(bytes) => match decode_bytes(&bytes) {
                Err(e) => checks.push(AssetCheck::fail("decode", e.to_string())),
                Ok(img) => {
                    checks = check_entry(entry, spec, Some(&img));
                    if let Some(recorded) = &entry.content_hash {
                        let actual = content_hash(&bytes);
                        if *recorded != actual {
                            checks.push(AssetCheck::warn(
                                "manifest",
                                format!(
                                    "file content changed since ingestion ({recorded} != {actual})"
                                ),
                            ));
                        }
                    }
                }
            },
        }
        if prompts.is_some() && spec.is_none() {
            checks.push(AssetCheck::warn("manifest", "id not in the prompt set"));
        }
        report.assets.push(AssetQa {
            id: entry.id.clone(),
            checks,
        });
    }

    for m in &manifest.missing {
        report.assets.push(AssetQa {
            id: m.id.clone(),
            checks: vec![missing_check(m.priority, &m.reason)],
        });
    }

    // Prompt specs that appear nowhere in the manifest.
    if let Some(prompts) = prompts {
        for spec in &prompts.specs {
            let listed = manifest.assets.iter().any(|a| a.serves_request(&spec.id))
                || manifest.missing.iter().any(|m| m.id == spec.id);
            if !listed {
                report.assets.push(AssetQa {
                    id: spec.id.clone(),
                    checks: vec![missing_check(spec.priority, "not in the manifest")],
                });
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_format() {
        let r = AssetQaReport::default();
        assert_eq!(r.report(), "assets: 0 (0 fail, 0 warnings)\n");
        assert!(!r.has_fail());
    }

    #[test]
    fn block_format() {
        let r = AssetQaReport {
            assets: vec![AssetQa {
                id: "a".into(),
                checks: vec![
                    AssetCheck::pass("resolution", "ok"),
                    AssetCheck::warn("occupancy", "wide"),
                ],
            }],
        };
        assert_eq!(
            r.report(),
            "a  WARNING\n  PASS     resolution: ok\n  WARNING  occupancy: wide\nassets: 1 (0 fail, 1 warning)\n"
        );
    }
}
