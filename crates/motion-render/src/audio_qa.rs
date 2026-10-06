//! Audio QA (0.8): cue placement checks and measured peak
//! alignment on a mixed track. See docs/SOUND_DESIGN.md §5.
//!
//! The report sees only the compiled project and the plan (not the taste), so
//! the per-style limits are checked against their loosest values: the
//! high-frequency minimum spacing and the dense information cap.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use motion_core::audio::{
    beat_scenes, information_cap, min_spacing, snap_tolerance, AudioPlan, CueKind, SfxLibrary,
};
use motion_core::compiler::taste::{CompositionRhythm, DensityLevel};
use motion_core::MotionProject;

use crate::RenderError;

const SAMPLE_RATE: usize = 48_000;
/// Window 10 ms, hop 1 ms.
const WINDOW: usize = 480;
const HOP: usize = 48;
/// Time tolerance for placement checks (cue times are ms-rounded).
const EPS: f64 = 1e-3;
/// A cue is isolated when no other cue lies within this many seconds and no
/// louder cue's audible span covers it ([`masks`]).
const ISOLATION: f64 = 0.25;
/// A cue whose effective peak is within this many dB of (or above) another
/// cue's can mask it while its audible span covers the other's time.
const MASK_MARGIN_DB: f64 = 12.0;

/// `o` masks `c` when `o` sounds (onset..audible_end, from the library) at
/// `c.time` and its effective peak is not more than MASK_MARGIN_DB below `c`'s.
/// Unknown sounds never mask.
fn masks(
    o: &motion_core::audio::AudioCue,
    c: &motion_core::audio::AudioCue,
    library: &SfxLibrary,
) -> bool {
    let (Some(os), Some(cs)) = (library.get(&o.sound_id), library.get(&c.sound_id)) else {
        return false;
    };
    let from = o.time - (os.peak - os.onset);
    let to = o.time + (os.audible_end - os.peak);
    let o_peak = os.peak_db + o.gain_db;
    let c_peak = cs.peak_db + c.gain_db;
    c.time >= from && c.time <= to && o_peak >= c_peak - MASK_MARGIN_DB
}
/// Search radius around a cue time for the measured peak.
const SEARCH: f64 = 0.1;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SceneCueCount {
    pub scene: String,
    pub cues: usize,
    pub information: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CueAlignment {
    pub scene: String,
    pub planned: f64,
    pub measured: f64,
    /// Seconds, absolute.
    pub error: f64,
    pub ok: bool,
}

/// (Phase 3) Distance of each beat-scene handoff anchor to the nearest
/// downbeat of the music bed. Informational; never a FAIL.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HandoffToDownbeat {
    /// Seconds, one per handoff (scene order).
    pub errors: Vec<f64>,
    pub max_error: f64,
    pub handoffs: usize,
    /// Handoffs within the loosest snap tolerance (slow_breathing, 0.35 s).
    pub within_tolerance: usize,
    pub tolerance: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AudioQaReport {
    /// PASS / WARN / FAIL.
    pub verdict: String,
    pub cue_count: usize,
    pub per_scene: Vec<SceneCueCount>,
    /// Smallest gap between two transient (non-bed) cues, if there are two.
    pub min_transient_gap: Option<f64>,
    /// Loosest per-style spacing floor the gap is checked against.
    pub spacing_floor: f64,
    /// Loosest per-style information cap per scene.
    pub information_cap: usize,
    /// Placement violations (each makes the verdict FAIL).
    pub violations: Vec<String>,
    /// Frame duration (seconds): the alignment tolerance.
    pub alignment_tolerance: f64,
    /// Present only when a mixed file was measured.
    pub alignment: Vec<CueAlignment>,
    pub max_alignment_error: Option<f64>,
    pub integrated_lufs: Option<f64>,
    pub sample_peak_db: Option<f64>,
    pub mixed_measured: bool,
    /// Present only when the plan has a music bed and its downbeats are known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub handoff_to_downbeat: Option<HandoffToDownbeat>,
}

impl AudioQaReport {
    pub fn to_text(&self) -> String {
        let mut s = String::from("\naudio\n");
        s.push_str(&format!("  cues: {}\n", self.cue_count));
        for sc in &self.per_scene {
            s.push_str(&format!(
                "  scene {}: {} cue(s), {} information\n",
                sc.scene, sc.cues, sc.information
            ));
        }
        match self.min_transient_gap {
            Some(g) => s.push_str(&format!(
                "  min transient gap: {g:.3}s (floor {:.2}s)\n",
                self.spacing_floor
            )),
            None => s.push_str("  min transient gap: n/a\n"),
        }
        for v in &self.violations {
            s.push_str(&format!("  FAIL {v}\n"));
        }
        if let Some(h) = &self.handoff_to_downbeat {
            s.push_str(&format!(
                "  music: handoff→downbeat max {:.0} ms ({} handoffs, {} within tolerance)\n",
                h.max_error * 1000.0,
                h.handoffs,
                h.within_tolerance
            ));
        }
        if self.mixed_measured {
            let iso = self.alignment.len();
            let misses = self.alignment.iter().filter(|a| !a.ok).count();
            match self.max_alignment_error {
                Some(e) => s.push_str(&format!(
                    "  alignment: {iso} isolated cue(s), max error {:.1} ms (limit {:.1} ms), {misses} miss(es)\n",
                    e * 1000.0,
                    self.alignment_tolerance * 1000.0
                )),
                None => s.push_str("  alignment: no isolated cues\n"),
            }
            for a in self.alignment.iter().filter(|a| !a.ok) {
                s.push_str(&format!(
                    "  WARN cue {} @ {:.3}s: peak measured at {:.3}s ({:+.1} ms)\n",
                    a.scene,
                    a.planned,
                    a.measured,
                    (a.measured - a.planned) * 1000.0
                ));
            }
            let lufs = self
                .integrated_lufs
                .map(|l| format!("{l:.1} LUFS"))
                .unwrap_or_else(|| "n/a LUFS".to_string());
            let peak = self
                .sample_peak_db
                .map(|p| format!("{p:.1} dBFS"))
                .unwrap_or_else(|| "n/a".to_string());
            s.push_str(&format!("  mix: {lufs}, sample peak {peak}\n"));
        }
        s.push_str(&format!(
            "audio: {} ({} cue(s), {} violation(s){})\n",
            self.verdict,
            self.cue_count,
            self.violations.len(),
            if self.mixed_measured {
                format!(
                    ", {} alignment miss(es)",
                    self.alignment.iter().filter(|a| !a.ok).count()
                )
            } else {
                String::new()
            }
        ));
        s
    }
}

pub fn audio_report(
    project: &MotionProject,
    plan: &AudioPlan,
    library: &SfxLibrary,
    mixed: Option<&Path>,
) -> Result<AudioQaReport, RenderError> {
    audio_report_with_music(project, plan, library, mixed, None)
}

/// [`audio_report`] plus, when the plan has a music bed and `downbeats` (the
/// MusicPlan's `downbeat_times`) are given, the handoff→downbeat distances.
pub fn audio_report_with_music(
    project: &MotionProject,
    plan: &AudioPlan,
    library: &SfxLibrary,
    mixed: Option<&Path>,
    downbeats: Option<&[f64]>,
) -> Result<AudioQaReport, RenderError> {
    let duration = project.duration_seconds();
    // The plan records the style's limits; fall back to the loosest ones.
    let floor = if plan.min_spacing > 0.0 {
        plan.min_spacing
    } else {
        min_spacing(CompositionRhythm::HighFrequency)
    };
    let cap = if plan.information_cap > 0 {
        plan.information_cap
    } else {
        information_cap(DensityLevel::Dense)
    };
    let mut violations: Vec<String> = Vec::new();

    // Cues per scene (project scene order, then unknown scenes).
    let mut per_scene: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for c in &plan.cues {
        let e = per_scene.entry(c.scene.as_str()).or_default();
        e.0 += 1;
        if c.kind == CueKind::Information {
            e.1 += 1;
        }
    }
    let mut counts: Vec<SceneCueCount> = Vec::new();
    for sc in &project.scenes {
        let (cues, information) = per_scene.remove(sc.id.as_str()).unwrap_or((0, 0));
        counts.push(SceneCueCount {
            scene: sc.id.clone(),
            cues,
            information,
        });
        if information > cap {
            violations.push(format!(
                "scene {}: {information} information cues (max {cap})",
                sc.id
            ));
        }
    }

    for c in &plan.cues {
        let where_ = format!("{} cue '{}' @ {:.3}s", c.kind.as_str(), c.sound_id, c.time);
        if c.time < -EPS || c.time > duration + EPS {
            violations.push(format!("{where_}: outside the project [0, {duration:.3}]"));
        }
        match project.scenes.iter().find(|s| s.id == c.scene) {
            None => violations.push(format!("{where_}: unknown scene '{}'", c.scene)),
            Some(sc) => {
                if c.time < sc.start_seconds - EPS || c.time > sc.end_seconds() + EPS {
                    violations.push(format!(
                        "{where_}: outside scene {} [{:.3}, {:.3}]",
                        sc.id,
                        sc.start_seconds,
                        sc.end_seconds()
                    ));
                }
                if let Some(life) = sc.lifecycle {
                    let anticipate = sc.start_seconds + life.anticipate;
                    // Cue times are ms-rounded: half a millisecond of slack.
                    if c.kind != CueKind::Handoff && c.time > anticipate + 5.0e-4 + 1e-9 {
                        violations.push(format!(
                            "{where_}: scene {} allows only handoffs from ANTICIPATE ({anticipate:.3}s)",
                            sc.id
                        ));
                    }
                }
            }
        }
        if !library.sounds.is_empty() {
            match library.get(&c.sound_id) {
                None => violations.push(format!("{where_}: sound not in the library")),
                Some(snd) if snd.family != c.family => violations.push(format!(
                    "{where_}: family {} does not match the library ({})",
                    c.family.as_str(),
                    snd.family.as_str()
                )),
                Some(_) => {}
            }
        }
    }

    let mut transient: Vec<f64> = plan
        .cues
        .iter()
        .filter(|c| !c.family.is_bed())
        .map(|c| c.time)
        .collect();
    transient.sort_by(|a, b| a.total_cmp(b));
    let min_gap = transient
        .windows(2)
        .map(|w| w[1] - w[0])
        .min_by(|a, b| a.total_cmp(b));
    if let Some(g) = min_gap {
        if g < floor - 1e-9 {
            violations.push(format!(
                "transient cues {g:.3}s apart (minimum {floor:.2}s)"
            ));
        }
    }

    let tolerance = 1.0 / f64::from(project.canvas.fps.max(1));
    let mut report = AudioQaReport {
        verdict: String::new(),
        cue_count: plan.cues.len(),
        per_scene: counts,
        min_transient_gap: min_gap,
        spacing_floor: floor,
        information_cap: cap,
        violations,
        alignment_tolerance: tolerance,
        alignment: Vec::new(),
        max_alignment_error: None,
        integrated_lufs: None,
        sample_peak_db: None,
        mixed_measured: false,
        handoff_to_downbeat: match (plan.music.as_ref(), downbeats) {
            (Some(_), Some(d)) => handoff_to_downbeat(project, d),
            _ => None,
        },
    };

    if let Some(path) = mixed {
        let samples = decode_mono(path)?;
        let env = rms_envelope(&samples);
        for (i, c) in plan.cues.iter().enumerate() {
            let isolated = plan.cues.iter().enumerate().all(|(j, o)| {
                j == i || (!masks(o, c, library) && (o.time - c.time).abs() > ISOLATION)
            });
            if !isolated {
                continue;
            }
            if let Some(measured) = peak_near(&env, c.time, SEARCH) {
                let error = (measured - c.time).abs();
                report.alignment.push(CueAlignment {
                    scene: c.scene.clone(),
                    planned: c.time,
                    measured,
                    error,
                    ok: error <= tolerance + 1e-9,
                });
            }
        }
        report.max_alignment_error = report
            .alignment
            .iter()
            .map(|a| a.error)
            .max_by(|a, b| a.total_cmp(b));
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        report.sample_peak_db = (peak > 0.0).then(|| 20.0 * f64::from(peak).log10());
        report.integrated_lufs = integrated_lufs(path);
        report.mixed_measured = true;
    }

    report.verdict = if !report.violations.is_empty() {
        "FAIL"
    } else if report.alignment.iter().any(|a| !a.ok) {
        "WARN"
    } else {
        "PASS"
    }
    .to_string();
    Ok(report)
}

/// Handoff anchors (overlap midpoints, as `plan_audio`) vs the nearest downbeat.
fn handoff_to_downbeat(project: &MotionProject, downbeats: &[f64]) -> Option<HandoffToDownbeat> {
    if downbeats.is_empty() {
        return None;
    }
    let beats = beat_scenes(project);
    let errors: Vec<f64> = (1..beats.len())
        .map(|i| {
            let s = beats[i].start_seconds;
            let anchor = s + (beats[i - 1].end_seconds() - s).max(0.0) / 2.0;
            downbeats
                .iter()
                .map(|b| (b - anchor).abs())
                .min_by(|a, b| a.total_cmp(b))
                .unwrap_or(f64::INFINITY)
        })
        .collect();
    let tolerance = snap_tolerance(CompositionRhythm::SlowBreathing);
    Some(HandoffToDownbeat {
        max_error: errors.iter().copied().fold(0.0, f64::max),
        handoffs: errors.len(),
        within_tolerance: errors.iter().filter(|e| **e <= tolerance + 1e-9).count(),
        tolerance,
        errors,
    })
}

/// RMS per window `k` covering `[k ms, k ms + 10 ms)` of 48 kHz mono samples.
pub(crate) fn rms_envelope(samples: &[f32]) -> Vec<f64> {
    if samples.len() < WINDOW {
        return Vec::new();
    }
    let mut prefix = Vec::with_capacity(samples.len() + 1);
    let mut acc = 0.0f64;
    prefix.push(0.0);
    for &s in samples {
        acc += f64::from(s) * f64::from(s);
        prefix.push(acc);
    }
    let windows = (samples.len() - WINDOW) / HOP + 1;
    (0..windows)
        .map(|k| {
            let a = k * HOP;
            ((prefix[a + WINDOW] - prefix[a]).max(0.0) / WINDOW as f64).sqrt()
        })
        .collect()
}

/// Centre (seconds) of window `k`.
fn window_centre(k: usize) -> f64 {
    (k * HOP) as f64 / SAMPLE_RATE as f64 + (WINDOW as f64 / 2.0) / SAMPLE_RATE as f64
}

/// Loudest window centre within `radius` of `time` (first on ties). `None`
/// when the neighbourhood is empty or silent.
fn peak_near(env: &[f64], time: f64, radius: f64) -> Option<f64> {
    let mut best: Option<(usize, f64)> = None;
    for (k, &v) in env.iter().enumerate() {
        let c = window_centre(k);
        if c < time - radius - 1e-9 {
            continue;
        }
        if c > time + radius + 1e-9 {
            break;
        }
        if best.map(|(_, b)| v > b).unwrap_or(true) {
            best = Some((k, v));
        }
    }
    best.filter(|&(_, v)| v > 0.0)
        .map(|(k, _)| (window_centre(k) * 1000.0).round() / 1000.0)
}

fn decode_mono(path: &Path) -> Result<Vec<f32>, RenderError> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "-",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    if !out.status.success() {
        return Err(RenderError::Encode(format!(
            "decoding {} failed: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect())
}

/// Integrated loudness from the ebur128 summary; `None` when gated out or
/// unavailable.
fn integrated_lufs(path: &Path) -> Option<f64> {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-v", "info", "-i"])
        .arg(path)
        .args(["-map", "0:a:0", "-af", "ebur128", "-f", "null", "-"])
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let err = String::from_utf8_lossy(&out.stderr);
    let summary = err.rsplit("Summary:").next()?;
    let line = summary
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("I:"))?;
    let v: f64 = line
        .trim_start_matches("I:")
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    (v.is_finite() && v > -70.0).then_some(v)
}
