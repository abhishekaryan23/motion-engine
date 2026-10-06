//! Reference analysis orchestration (0.7): one video file → a
//! ReferenceAnalysisBundle directory. Deterministic for a given file,
//! analyzer version, configuration and ffmpeg build.
//!
//! ```text
//! probe → analysis stream (one ffmpeg decode, audio never decoded)
//!       → temporal signals + evidence → key-frame selection → sample JPEGs
//!       → color + complexity evidence → evidence.json → contact sheet
//!       → interpreter-request.json + interpreter-prompt.md
//! ```

use std::path::{Path, PathBuf};

use motion_core::reference::bundle::{interpreter_prompt, interpreter_request};
use motion_core::reference::evidence::{
    reference_fingerprint, round_ms, AnalysisSpec, ReferenceEvidence, ReferenceSample,
    ANALYZER_VERSION, EVIDENCE_VERSION,
};

use super::{cache, color, complexity, contact, media, sample, temporal, ReferenceError};

pub const EVIDENCE_FILE: &str = "evidence.json";
pub const CONTACT_SHEET_FILE: &str = "contact-sheet.png";
pub const REQUEST_FILE: &str = "interpreter-request.json";
pub const PROMPT_FILE: &str = "interpreter-prompt.md";

/// Analysis configuration; part of the reference fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalysisConfig {
    pub analysis_short_side: u32,
    pub sample_short_side: u32,
    /// `None` = `sample::sample_budget(duration)`.
    pub sample_budget: Option<usize>,
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        AnalysisConfig {
            analysis_short_side: media::ANALYSIS_SHORT_SIDE,
            sample_short_side: media::SAMPLE_SHORT_SIDE,
            sample_budget: None,
        }
    }
}

impl AnalysisConfig {
    /// Stable text form hashed into the reference fingerprint.
    pub fn canonical(&self) -> String {
        format!(
            "analysis_short_side={};sample_short_side={};sample_budget={}",
            self.analysis_short_side,
            self.sample_short_side,
            self.sample_budget
                .map(|b| b.to_string())
                .unwrap_or_else(|| "auto".into())
        )
    }
}

#[derive(Debug, Clone)]
pub struct AnalysisOutcome {
    pub evidence: ReferenceEvidence,
    pub dir: PathBuf,
    /// Where the evidence came from without re-extracting frames.
    pub reused: Reuse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reuse {
    /// Analysed now.
    Fresh,
    /// `out_dir` already held a complete bundle for this fingerprint.
    OutDir,
    /// Copied from the reference cache.
    Cache,
}

/// Read `dir/evidence.json` if it exists and belongs to `fingerprint`, and
/// every sample image and the contact sheet are present.
pub fn existing_bundle(dir: &Path, fingerprint: &str) -> Option<ReferenceEvidence> {
    let text = std::fs::read_to_string(dir.join(EVIDENCE_FILE)).ok()?;
    let ev: ReferenceEvidence = serde_json::from_str(&text).ok()?;
    let complete = ev.reference_fingerprint == fingerprint
        && ev.version == EVIDENCE_VERSION
        && ev.analyzer_version == ANALYZER_VERSION
        && !ev.samples.is_empty()
        && cache::complete_file(dir, CONTACT_SHEET_FILE)
        && ev
            .samples
            .iter()
            .all(|s| cache::safe_relative(&s.image) && cache::complete_file(dir, &s.image));
    complete.then_some(ev)
}

/// Analyse `video` into `out_dir`. With `cache_dir`, a bundle analysed
/// before (same media bytes, analyzer version and configuration) is reused
/// instead of re-extracting frames, and a fresh analysis is stored there.
pub fn analyze_reference(
    video: &Path,
    out_dir: &Path,
    config: &AnalysisConfig,
    cache_dir: Option<&Path>,
) -> Result<AnalysisOutcome, ReferenceError> {
    let media_fp = media::media_fingerprint(video)?;
    let fingerprint = reference_fingerprint(&media_fp, &config.canonical());
    std::fs::create_dir_all(out_dir)?;

    let (evidence, reused) = if let Some(ev) = existing_bundle(out_dir, &fingerprint) {
        (ev, Reuse::OutDir)
    } else if let Some(ev) =
        cache_dir.and_then(|c| cache::restore(c, &fingerprint, out_dir).ok().flatten())
    {
        (ev, Reuse::Cache)
    } else {
        let ev = analyze_fresh(video, out_dir, config, media_fp, fingerprint)?;
        if let Some(c) = cache_dir {
            cache::store(c, &ev, out_dir)?;
        }
        (ev, Reuse::Fresh)
    };

    // The request and prompt are cheap and always rewritten from the evidence.
    let request = interpreter_request(&evidence, CONTACT_SHEET_FILE);
    write_json(&out_dir.join(REQUEST_FILE), &request)?;
    std::fs::write(
        out_dir.join(PROMPT_FILE),
        interpreter_prompt(&request, &evidence),
    )?;
    Ok(AnalysisOutcome {
        evidence,
        dir: out_dir.to_path_buf(),
        reused,
    })
}

fn analyze_fresh(
    video: &Path,
    out_dir: &Path,
    config: &AnalysisConfig,
    media_fingerprint: String,
    reference_fingerprint: String,
) -> Result<ReferenceEvidence, ReferenceError> {
    let metadata = media::probe(video)?;
    let decoder = media::ffmpeg_version()?;
    let stream = media::decode_analysis_stream(video, &metadata, config.analysis_short_side)?;
    if stream.is_empty() {
        return Err(ReferenceError::NoVideo(video.display().to_string()));
    }

    let signals = temporal::temporal_signals(&stream);
    let temporal = temporal::temporal_evidence(&stream, &signals);

    let budget = config
        .sample_budget
        .unwrap_or_else(|| sample::sample_budget(metadata.duration_seconds));
    let plans = sample::select_samples(&signals, stream.fps, stream.len(), budget);

    // Invalidate an old completion marker before replacing any artifacts.
    let evidence_path = out_dir.join(EVIDENCE_FILE);
    if evidence_path.exists() {
        std::fs::remove_file(&evidence_path)?;
    }
    let samples_dir = out_dir.join("samples");
    if samples_dir.is_dir() {
        std::fs::remove_dir_all(&samples_dir)?;
    }
    let mut samples = Vec::with_capacity(plans.len());
    for (k, plan) in plans.iter().enumerate() {
        let id = format!("s{:02}", k + 1);
        let image = format!("samples/{id}.jpg");
        let time = round_ms(plan.time_seconds);
        media::extract_sample(video, time, &out_dir.join(&image), config.sample_short_side)?;
        samples.push(ReferenceSample {
            id,
            time_seconds: time,
            frame: (time * metadata.fps).round().max(0.0) as u64,
            kinds: plan.kinds.clone(),
            image,
        });
    }
    let sample_frames: Vec<(String, usize)> = samples
        .iter()
        .zip(&plans)
        .map(|(s, p)| (s.id.clone(), p.frame))
        .collect();
    let color = color::color_evidence(&stream, &sample_frames);
    let complexity = complexity::complexity_evidence(&stream);

    let evidence = ReferenceEvidence {
        version: EVIDENCE_VERSION.to_string(),
        analyzer_version: ANALYZER_VERSION.to_string(),
        reference_fingerprint,
        media_fingerprint,
        analysis: AnalysisSpec {
            width: stream.width,
            height: stream.height,
            fps: stream.fps,
            frames: stream.len() as u64,
            decoder,
        },
        metadata,
        samples,
        color,
        temporal,
        complexity,
    };
    contact::contact_sheet(
        out_dir,
        &evidence.samples,
        &out_dir.join(CONTACT_SHEET_FILE),
    )?;
    let tmp = out_dir.join(format!("{EVIDENCE_FILE}.tmp"));
    write_json(&tmp, &evidence)?;
    std::fs::rename(tmp, evidence_path)?;
    Ok(evidence)
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), ReferenceError> {
    let text = serde_json::to_string_pretty(value)
        .map_err(|e| ReferenceError::Io(std::io::Error::other(e)))?;
    std::fs::write(path, text + "\n")?;
    Ok(())
}
