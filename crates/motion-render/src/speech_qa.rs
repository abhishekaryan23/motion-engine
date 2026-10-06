//! (0.10) Speech QA: do the captions follow the voice-over, do SFX stay off
//! word onsets, and is the mixed track loud enough but not clipping?
//! See docs/VOICE.md.
//!
//! | check | rule |
//! |---|---|
//! | `caption_timing` | every caption word fades in within 1 frame of its spoken start |
//! | `caption_safe_area` | every caption text box lies inside the safe area |
//! | `caption_lines` | at most 2 lines per page, at most 32 characters per line |
//! | `layout` | layout QA (text inside the safe area, minimum size, ...) passes |
//! | `sfx_vs_words` | no planned SFX peak within 80 ms of a word onset (needs a plan) |
//! | `loudness` | the mix's integrated loudness is within 1 LU of the plan target (needs `--mixed`) |
//! | `peak` | true and sample peak at most -1 dBTP (+ [`PEAK_CODEC_MARGIN_DB`] for the AAC decode) |
//! | `bed_duck` | (informational, never FAIL) the legacy sidechain's duck of the music bed under the voice, 10 dB (+-3): PASS inside, WARN outside (needs `--mixed` and a music bed) |
//! | `voice_over_music` | (0.23) on the mix's own stems, over the voiced segments: voice minus bed loudness, PASS at 15 dB or more, WARN from 10, FAIL below (needs `--audio-plan` with a bed) |
//! | `bed_over_voice` | (0.23) the bed's largest 400 ms level in the narration gaps at most 6 dB under the voice's integrated loudness, FAIL above |
//! | `speech_band_masking` | (0.23, WARN) the bed's 300-4000 Hz level while the voice speaks at most 18 dB under the voice's |
//! | `speech_rate` | (0.20) per sentence, syllables / (last word end - first word start): FAIL outside `SPEECH_RATE_FAIL`, WARN outside `SPEECH_RATE_WARN` |
//! | `reveal_before_speech` | (0.20) no anchored layer (`ArtRecord.reveals`; kicker exempt) becomes readable more than `REVEAL_LEAD_MAX` before its anchor word starts |
//! | `reveal_late` | (0.20, WARN) an anchored layer readable more than `REVEAL_LATE_MAX` after its anchor word ends |
//! | `spoken_mismatch` | (0.20) the recognised words (`SpeechMap.recognised`) against the script: FAIL on a missing / changed content word (a number, an anchor word), WARN otherwise; SKIP without recognised words |
//!
//! A check may also WARN: the verdict stays PASS, the finding is printed.
//!
//! (0.23) The three stem checks rebuild the mix's voice and bed stems from the
//! audio plan, the speech map and the music beside the plan ([`crate::mix_qa`]:
//! the voice-relative graph when `plan.levels` is set, the legacy graph
//! otherwise) in a temp directory that is deleted before the report returns.
//!
//! (0.20) The reveal checks time each anchored group whose anchor word is
//! spoken in its beat (`anchor_time` over the beat's words) against the first
//! frame at which any of its layers is readable (opacity, blur, on canvas,
//! glyph cascade landed; see [`crate::reveal_qa`]); a group that never
//! becomes readable is a `reveal_late` finding. `spoken_mismatch` aligns the
//! whole take's recognised words to the script words at once (monotone edit
//! distance; numbers compare by value, written or said) and reports per beat;
//! a content word is a number (digits or number words) or a word of the
//! beat's anchors.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use motion_core::audio::{AudioPlan, SPEECH_CLEARANCE};
use motion_core::caption_qa::{caption_report, CaptionCheck};
use motion_core::checks::{
    BED_OVER_VOICE, BED_OVER_VOICE_MAX_DB, SPEECH_BAND_MASKING, SPEECH_BAND_MASKING_MAX_DB,
    VOICE_OVER_MUSIC, VOICE_OVER_MUSIC_FAIL_DB, VOICE_OVER_MUSIC_PASS_DB,
};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::speech_plan::anchor_time;
use motion_core::speech::{
    estimate_syllables, normalize_word, statement_tokens, RevealAnchor, RevealRole, SpeechMap,
    SpeechSentence, SpeechWord, CUE_MIN_READ, MATCH_RATIO, REVEAL_LATE_MAX, REVEAL_LEAD_MAX,
    SPEECH_RATE_FAIL, SPEECH_RATE_WARN,
};
use motion_core::{layout_report_with, MotionProject};
use serde::Serialize;

use crate::audio_mix::{measure_bed_duck, VoiceTrack, BED_DUCK_DB};
use crate::mix_qa::{
    bed_over_voice_status, speech_band_masking_status, stem_qa, voice_over_music_status,
    StemMeasure, StemQa,
};
use crate::reveal_qa::{first_readable_frames, scene_prefix, GroupRef};
use crate::RenderError;

/// The AAC decode of a limiter-held -1 dBFS mix can overshoot by a fraction of
/// a dB; the peak check allows this much.
pub const PEAK_CODEC_MARGIN_DB: f64 = 0.3;
/// Integrated loudness tolerance (LU) around the plan target.
pub const LUFS_TOLERANCE: f64 = 1.0;
/// Allowed deviation of the measured bed duck from [`BED_DUCK_DB`] (dB).
pub const BED_DUCK_TOLERANCE_DB: f64 = 3.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    /// Not applicable to the inputs given (no plan, no mixed file, no music).
    Skip,
    /// (0.20) Worth a look, but the build passes.
    Warn,
}

impl CheckStatus {
    pub fn label(self) -> &'static str {
        match self {
            CheckStatus::Pass => "PASS",
            CheckStatus::Fail => "FAIL",
            CheckStatus::Skip => "SKIP",
            CheckStatus::Warn => "WARN",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechQaCheck {
    pub name: String,
    pub status: CheckStatus,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpeechQaReport {
    /// PASS when no check FAILs.
    pub verdict: String,
    pub checks: Vec<SpeechQaCheck>,
    /// Largest `|caption word-in - word onset|` in milliseconds.
    pub max_caption_delta_ms: f64,
    /// One frame in milliseconds: the caption timing limit.
    pub caption_limit_ms: f64,
    pub words: usize,
    /// Planned SFX peaks within 80 ms of a word onset: `"<time> s (<scene>)"`.
    pub sfx_conflicts: Vec<String>,
    pub integrated_lufs: Option<f64>,
    pub true_peak_db: Option<f64>,
    pub sample_peak_db: Option<f64>,
    /// Measured music-bed gain reduction under the voice (dB); informational.
    pub bed_duck_db: Option<f64>,
    /// (0.23) Voice minus bed loudness over the voiced segments of the mix's
    /// stems (dB): the value behind `voice_over_music`.
    pub voice_over_music_db: Option<f64>,
    /// (0.23) The bed's largest momentary level in the narration gaps minus the
    /// voice's integrated loudness (dB): the value behind `bed_over_voice`.
    pub bed_over_voice_db: Option<f64>,
    /// (0.23) The bed's 300-4000 Hz level minus the voice's over the voiced
    /// segments (dB): the value behind `speech_band_masking`.
    pub speech_band_masking_db: Option<f64>,
    /// (0.20) `speech_rate` findings: `beat N: X.X syll/s (<first words…>)`.
    pub speech_rate_findings: Vec<String>,
    /// (0.20) `reveal_before_speech` findings: `beat N: <group> (<role>)
    /// readable at <t> s, "<word>" starts at <t> s (lead <ms> ms)`.
    pub reveal_before_speech_findings: Vec<String>,
    /// (0.20) `reveal_late` findings: `beat N: <group> (<role>) readable at
    /// <t> s, "<word>" ends at <t> s (late <ms> ms)`, or `beat N: <group>
    /// (<role>) never readable, "<word>" ends at <t> s`.
    pub reveal_late_findings: Vec<String>,
    /// (0.20) `spoken_mismatch` findings: `beat N: missing "<word>"` /
    /// `beat N: "<script>" heard as "<recognised>"`.
    pub spoken_mismatch_findings: Vec<String>,
    /// (0.20) Every anchored group whose anchor word is spoken: when it
    /// becomes readable against that word (the data behind the reveal checks).
    pub reveal_timings: Vec<RevealTiming>,
}

/// (0.20) One anchored layer group against its spoken anchor word. Times are
/// project seconds (the voice-over timeline).
#[derive(Debug, Clone, Serialize)]
pub struct RevealTiming {
    /// 1-based beat number.
    pub beat: usize,
    pub scene: String,
    /// The anchor's group path (`hero`, `card.0`).
    pub group: String,
    pub role: RevealRole,
    /// The script word where the anchor was first heard (the first word of a
    /// multi-word match).
    pub word: String,
    pub word_start: f64,
    pub word_end: f64,
    /// First frame at which any member layer is readable; `None` when none
    /// is while the scene is active.
    pub readable_frame: Option<u32>,
    pub readable_at: Option<f64>,
}

/// How many findings `to_text` prints per check.
const TEXT_FINDINGS: usize = 8;

impl SpeechQaReport {
    pub fn passed(&self) -> bool {
        self.verdict == "PASS"
    }

    /// (0.20) The findings kept for check `name` (empty for the checks that
    /// keep none).
    pub fn findings(&self, name: &str) -> &[String] {
        match name {
            "speech_rate" => &self.speech_rate_findings,
            "reveal_before_speech" => &self.reveal_before_speech_findings,
            "reveal_late" => &self.reveal_late_findings,
            "spoken_mismatch" => &self.spoken_mismatch_findings,
            _ => &[],
        }
    }

    pub fn to_text(&self) -> String {
        let mut s = String::from("\nSPEECH QA\n");
        for c in &self.checks {
            s.push_str(&format!(
                "  {} {}: {}\n",
                c.status.label(),
                c.name,
                c.detail
            ));
            let findings = self.findings(&c.name);
            for f in findings.iter().take(TEXT_FINDINGS) {
                s.push_str(&format!("      {f}\n"));
            }
            if findings.len() > TEXT_FINDINGS {
                s.push_str(&format!(
                    "      ... {} finding(s) in all, first {TEXT_FINDINGS} shown\n",
                    findings.len()
                ));
            }
        }
        let opt = |v: Option<f64>, unit: &str| {
            v.map(|x| format!("{x:.1} {unit}"))
                .unwrap_or_else(|| "n/a".to_string())
        };
        s.push_str(&format!(
            "  max caption delta {:.1} ms (limit {:.1} ms) over {} word(s); LUFS {}; true peak {}; bed duck {}; voice over music {}; bed over voice {}; speech band {}\n",
            self.max_caption_delta_ms,
            self.caption_limit_ms,
            self.words,
            opt(self.integrated_lufs, "LUFS"),
            opt(self.true_peak_db, "dBTP"),
            opt(self.bed_duck_db, "dB"),
            opt(self.voice_over_music_db, "dB"),
            opt(self.bed_over_voice_db, "dB"),
            opt(self.speech_band_masking_db, "dB"),
        ));
        s.push_str(&format!("speech qa: {}\n", self.verdict));
        s
    }
}

/// What a mixed file measures.
#[derive(Debug, Clone, Copy)]
struct MixMeasure {
    integrated: f64,
    true_peak: f64,
    sample_peak: f64,
}

fn check(name: &str, status: CheckStatus, detail: impl Into<String>) -> SpeechQaCheck {
    SpeechQaCheck {
        name: name.to_string(),
        status,
        detail: detail.into(),
    }
}

/// (0.23) The status of `bed_duck`, which is informational: it measures the
/// depth of the legacy sidechain (`measure_bed_duck`), not the bed's level
/// against the voice (`voice_over_music` and `bed_over_voice` do that on the
/// stems), so it never FAILs: PASS inside the old band
/// ([`BED_DUCK_DB`] +- [`BED_DUCK_TOLERANCE_DB`]), WARN outside it.
pub fn bed_duck_status(depth_db: f64) -> CheckStatus {
    if (depth_db - BED_DUCK_DB).abs() <= BED_DUCK_TOLERANCE_DB {
        CheckStatus::Pass
    } else {
        CheckStatus::Warn
    }
}

fn pass_fail(ok: bool) -> CheckStatus {
    if ok {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    }
}

/// Run every speech check. `plan` enables the SFX-vs-onset check; `mixed`
/// (an MP4/audio file) enables loudness, peak and (with a music bed under
/// `music_root`) the measured bed duck. `speech_dir` is the directory holding
/// the speech file (`speech.audio` is relative to it). (0.23) With a `plan`
/// that has a music bed under `music_root` the stem checks
/// (`voice_over_music`, `bed_over_voice`, `speech_band_masking`) also run, on
/// stems rebuilt from the plan, with or without `mixed`. `scene_dir` is the
/// directory of the motion file: with it the layout check also judges the
/// delivered images (`text_over_subject`, `subject_too_small`,
/// `frame_too_loose`, `asset_low_contrast`; see `motion_core::subject_qa`).
pub fn speech_report(
    project: &MotionProject,
    speech: &SpeechMap,
    speech_dir: &Path,
    plan: Option<&AudioPlan>,
    mixed: Option<&Path>,
    music_root: Option<&Path>,
    scene_dir: Option<&Path>,
) -> Result<SpeechQaReport, RenderError> {
    let mut checks: Vec<SpeechQaCheck> = Vec::new();
    let fps = f64::from(project.canvas.fps.max(1));
    let limit_ms = 1000.0 / fps;

    // Captions (skipped when the project was compiled with --no-captions).
    let has_captions = project
        .scenes
        .iter()
        .any(|s| s.id == motion_core::compiler::captions::CAPTION_SCENE_ID);
    if !has_captions {
        for name in ["caption_timing", "caption_safe_area", "caption_lines"] {
            checks.push(check(name, CheckStatus::Skip, "captions disabled"));
        }
    }
    let cap = caption_report(project, speech);
    let max_delta_ms = if has_captions {
        cap.max_delta * 1000.0
    } else {
        0.0
    };
    let count = |kinds: &[CaptionCheck]| {
        cap.findings
            .iter()
            .filter(|f| kinds.contains(&f.check))
            .count()
    };
    if has_captions {
        let timing = count(&[
            CaptionCheck::SceneMissing,
            CaptionCheck::WordMissing,
            CaptionCheck::WordTiming,
        ]);
        checks.push(check(
        "caption_timing",
        pass_fail(timing == 0),
        format!(
            "{} word(s), max delta {max_delta_ms:.1} ms (limit {limit_ms:.1} ms), {timing} finding(s)",
            cap.words.len()
        ),
    ));
        let safe = count(&[CaptionCheck::OutsideSafe]);
        checks.push(check(
            "caption_safe_area",
            pass_fail(safe == 0),
            format!("{safe} finding(s)"),
        ));
        let lines = count(&[CaptionCheck::TooManyLines, CaptionCheck::LineTooLong]);
        checks.push(check(
            "caption_lines",
            pass_fail(lines == 0),
            format!(
                "max {} line(s), max {} char(s)/line, {lines} finding(s)",
                cap.max_lines, cap.max_line_chars
            ),
        ));
    }

    // (0.20) Timing: the narrator's pace, reveals against their words, and
    // what the recogniser heard against the script.
    let rate = speech_rate_check(speech);
    checks.push(check("speech_rate", rate.status, rate.detail));
    let reveals = reveal_checks(project, speech)?;
    checks.push(check(
        "reveal_before_speech",
        reveals.before.status,
        reveals.before.detail,
    ));
    checks.push(check(
        "reveal_late",
        reveals.late.status,
        reveals.late.detail,
    ));
    let mismatch = spoken_mismatch_check(project, speech);
    checks.push(check("spoken_mismatch", mismatch.status, mismatch.detail));

    // Layout.
    match LayoutFrame::new(project.canvas.width, project.canvas.height) {
        Ok(frame) => {
            let images = scene_dir
                .map(|dir| crate::image_index::image_index(project, dir))
                .unwrap_or_default();
            let layout = layout_report_with(project, &frame, &images);
            checks.push(check(
                "layout",
                pass_fail(layout.passed()),
                format!("{} finding(s)", layout.findings.len()),
            ));
        }
        Err(e) => checks.push(check("layout", CheckStatus::Fail, format!("canvas: {e}"))),
    }

    // SFX peaks against word onsets.
    let mut sfx_conflicts: Vec<String> = Vec::new();
    match plan {
        Some(plan) => {
            for c in &plan.cues {
                let near = speech
                    .words
                    .iter()
                    .map(|w| (c.time - w.start).abs())
                    .fold(f64::INFINITY, f64::min);
                if near < SPEECH_CLEARANCE - 1e-6 {
                    sfx_conflicts.push(format!(
                        "{:.3} s ({}, {:.0} ms from a word)",
                        c.time,
                        c.scene,
                        near * 1000.0
                    ));
                }
            }
            checks.push(check(
                "sfx_vs_words",
                pass_fail(sfx_conflicts.is_empty()),
                format!(
                    "{} cue(s), {} within {:.0} ms of a word onset",
                    plan.cues.len(),
                    sfx_conflicts.len(),
                    SPEECH_CLEARANCE * 1000.0
                ),
            ));
        }
        None => checks.push(check("sfx_vs_words", CheckStatus::Skip, "no audio plan")),
    }

    // The mix.
    let (mut lufs, mut tp, mut sp, mut duck) = (None, None, None, None);
    match mixed {
        Some(file) => {
            let target = plan.map_or(-16.0, |p| p.loudness_target);
            match measure_mix(file)? {
                Some(m) => {
                    lufs = Some(m.integrated);
                    tp = Some(m.true_peak);
                    sp = Some(m.sample_peak);
                    checks.push(check(
                        "loudness",
                        pass_fail((m.integrated - target).abs() <= LUFS_TOLERANCE),
                        format!(
                            "{:.1} LUFS (target {target:.0} +-{LUFS_TOLERANCE:.0})",
                            m.integrated
                        ),
                    ));
                    let ceiling = -1.0 + PEAK_CODEC_MARGIN_DB;
                    checks.push(check(
                        "peak",
                        pass_fail(m.true_peak <= ceiling && m.sample_peak <= ceiling),
                        format!(
                            "true {:.1} dBTP, sample {:.1} dBFS (limit -1.0, codec margin {PEAK_CODEC_MARGIN_DB:.1})",
                            m.true_peak, m.sample_peak
                        ),
                    ));
                }
                None => {
                    checks.push(check(
                        "loudness",
                        CheckStatus::Fail,
                        "mix is silent or unmeasurable",
                    ));
                    checks.push(check(
                        "peak",
                        CheckStatus::Fail,
                        "mix is silent or unmeasurable",
                    ));
                }
            }
            // Bed duck under the voice, when the plan carries a music bed.
            let bed = plan.and_then(|p| p.music.as_ref());
            match (bed, music_root) {
                (Some(bed), Some(root)) => {
                    let track = root.join(&bed.track);
                    let voice = VoiceTrack {
                        path: speech_dir.join(&speech.audio),
                        offset: 0.0,
                        gain_db: 0.0,
                    };
                    // While a word sounds (after the 15 ms attack has settled):
                    // power-averaging over the gaps between words would
                    // weigh the released, shallower reduction.
                    let spans: Vec<(f64, f64)> = speech
                        .words
                        .iter()
                        .map(|w| (w.start + 0.05, w.end))
                        .filter(|(a, b)| b - a > 0.06)
                        .collect();
                    if !track.is_file() {
                        // (0.23) Never a silent skip: the plan names a bed that
                        // is not there.
                        checks.push(check(
                            "bed_duck",
                            CheckStatus::Warn,
                            bed_missing_detail(&track),
                        ));
                    } else if !voice.path.is_file() {
                        checks.push(check(
                            "bed_duck",
                            CheckStatus::Skip,
                            voice_missing_detail(&voice.path),
                        ));
                    } else {
                        duck = measure_bed_duck(
                            &voice,
                            bed,
                            &track,
                            project.duration_seconds(),
                            &spans,
                        )?;
                        // (0.23) Informational ([`bed_duck_status`]).
                        match duck {
                            Some(d) => checks.push(check(
                                "bed_duck",
                                bed_duck_status(d),
                                format!("{d:.1} dB under the voice (informational; legacy sidechain target {BED_DUCK_DB:.0} +-{BED_DUCK_TOLERANCE_DB:.0})"),
                            )),
                            None => checks.push(check("bed_duck", CheckStatus::Skip, "no measurable bed under speech")),
                        }
                    }
                }
                _ => checks.push(check("bed_duck", CheckStatus::Skip, "no music bed")),
            }
        }
        None => {
            for n in ["loudness", "peak", "bed_duck"] {
                checks.push(check(n, CheckStatus::Skip, "no --mixed file"));
            }
        }
    }

    // (0.23) The narrator against the bed, on the mix's own stems.
    let stems = stem_checks(
        plan,
        speech,
        speech_dir,
        music_root,
        project.duration_seconds(),
    )?;
    checks.extend(stems.checks);

    let failed = checks.iter().any(|c| c.status == CheckStatus::Fail);
    Ok(SpeechQaReport {
        verdict: if failed { "FAIL" } else { "PASS" }.to_string(),
        checks,
        max_caption_delta_ms: max_delta_ms,
        caption_limit_ms: limit_ms,
        words: cap.words.len(),
        sfx_conflicts,
        integrated_lufs: lufs,
        true_peak_db: tp,
        sample_peak_db: sp,
        bed_duck_db: duck,
        voice_over_music_db: stems.voice_over_music,
        bed_over_voice_db: stems.bed_over_voice,
        speech_band_masking_db: stems.speech_band_masking,
        speech_rate_findings: rate.findings,
        reveal_before_speech_findings: reveals.before.findings,
        reveal_late_findings: reveals.late.findings,
        spoken_mismatch_findings: mismatch.findings,
        reveal_timings: reveals.timings,
    })
}

// ---------------------------------------------------------------------------
// (0.23) Stem checks
// ---------------------------------------------------------------------------

/// The three stem checks and the values behind them.
struct StemChecks {
    checks: Vec<SpeechQaCheck>,
    voice_over_music: Option<f64>,
    bed_over_voice: Option<f64>,
    speech_band_masking: Option<f64>,
}

/// `voice_over_music`, `bed_over_voice` and `speech_band_masking` from the
/// stems rebuilt out of `plan` (see [`crate::mix_qa::stem_qa`]); SKIP without a
/// plan with a bed (and the files). The voice starts at 0 s, as the CLI's
/// `render` and `mix` place it.
fn stem_checks(
    plan: Option<&AudioPlan>,
    speech: &SpeechMap,
    speech_dir: &Path,
    music_root: Option<&Path>,
    duration: f64,
) -> Result<StemChecks, RenderError> {
    let skipped = |why: &str| StemChecks {
        checks: [VOICE_OVER_MUSIC, BED_OVER_VOICE, SPEECH_BAND_MASKING]
            .iter()
            .map(|name| check(name, CheckStatus::Skip, why))
            .collect(),
        voice_over_music: None,
        bed_over_voice: None,
        speech_band_masking: None,
    };
    let Some(plan) = plan else {
        return Ok(skipped("no audio plan"));
    };
    let (Some(bed), Some(root)) = (plan.music.as_ref(), music_root) else {
        return Ok(skipped("no music bed"));
    };
    let voice = speech_dir.join(&speech.audio);
    let track = root.join(&bed.track);
    if !track.is_file() {
        // Never a silent skip: the plan names a bed that is not there, so the
        // mix was not measured against it.
        return Ok(StemChecks {
            checks: [VOICE_OVER_MUSIC, BED_OVER_VOICE, SPEECH_BAND_MASKING]
                .iter()
                .map(|name| check(name, CheckStatus::Warn, bed_missing_detail(&track)))
                .collect(),
            voice_over_music: None,
            bed_over_voice: None,
            speech_band_masking: None,
        });
    }
    if !voice.is_file() {
        return Ok(skipped(&voice_missing_detail(&voice)));
    }
    if speech.words.is_empty() {
        return Ok(skipped("no spoken words"));
    }
    let m = stem_qa(&StemQa {
        plan,
        speech,
        voice: &voice,
        offset: 0.0,
        music_root: root,
        duration,
        temp_base: None,
    })?;
    Ok(stem_checks_of(&m, plan.levels.is_none()))
}

/// (0.23) Why a bed check did not measure: the plan's bed file is not where
/// the plan (relative to its own directory) says it is.
fn bed_missing_detail(track: &Path) -> String {
    format!(
        "music bed {} not found (the audio plan's bed track, resolved against the plan's directory): the bed was not measured",
        track.display()
    )
}

fn voice_missing_detail(voice: &Path) -> String {
    format!(
        "voice file {} not found (speech.audio is relative to the speech file)",
        voice.display()
    )
}

/// The three checks of one measurement (`legacy`: the plan has no `levels`,
/// so the stems are those of the pre-0.23 graph).
fn stem_checks_of(m: &StemMeasure, legacy: bool) -> StemChecks {
    let mix = if legacy {
        "legacy mix (the plan has no levels; the bed's extra duck under SFX cues is not rebuilt)"
    } else {
        "voice-relative mix"
    };
    let vom = match m.voice_over_music {
        Some(v) => check(
            VOICE_OVER_MUSIC,
            voice_over_music_status(v),
            format!(
                "voice {v:.1} dB over the music while speaking (pass at {VOICE_OVER_MUSIC_PASS_DB:.0} or more, warn from {VOICE_OVER_MUSIC_FAIL_DB:.0}, fail below); {mix}"
            ),
        ),
        None => check(VOICE_OVER_MUSIC, CheckStatus::Skip, "no measurable voice or bed under speech"),
    };
    let bov = match m.bed_over_voice {
        Some(v) => check(
            BED_OVER_VOICE,
            bed_over_voice_status(v),
            format!(
                "bed {v:.1} dB re the voice at its loudest in {} narration gap(s) (limit {BED_OVER_VOICE_MAX_DB:.0}); {mix}",
                m.gaps
            ),
        ),
        None => check(BED_OVER_VOICE, CheckStatus::Skip, "no narration gap to measure"),
    };
    let band = match m.speech_band_masking {
        Some(v) => check(
            SPEECH_BAND_MASKING,
            speech_band_masking_status(v),
            format!(
                "bed {v:.1} dB re the voice in 300-4000 Hz while speaking (WARN above {SPEECH_BAND_MASKING_MAX_DB:.0}); {mix}"
            ),
        ),
        None => check(SPEECH_BAND_MASKING, CheckStatus::Skip, "no measurable speech band"),
    };
    StemChecks {
        checks: vec![vom, bov, band],
        voice_over_music: m.voice_over_music,
        bed_over_voice: m.bed_over_voice,
        speech_band_masking: m.speech_band_masking,
    }
}

// ---------------------------------------------------------------------------
// (0.20) Timing checks
// ---------------------------------------------------------------------------

/// What one timing check concluded.
struct Outcome {
    status: CheckStatus,
    detail: String,
    findings: Vec<String>,
}

/// The worse of two statuses for a check that may PASS, WARN or FAIL.
fn worse(a: CheckStatus, b: CheckStatus) -> CheckStatus {
    let rank = |s: CheckStatus| match s {
        CheckStatus::Skip | CheckStatus::Pass => 0,
        CheckStatus::Warn => 1,
        CheckStatus::Fail => 2,
    };
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

/// Words of a sentence shown in a finding: the first four, then "…".
fn first_words(words: &[&SpeechWord]) -> String {
    const SHOWN: usize = 4;
    let head: Vec<&str> = words.iter().take(SHOWN).map(|w| w.text.as_str()).collect();
    let mut s = head.join(" ");
    if words.len() > SHOWN {
        s.push('…');
    }
    s
}

/// Sentences in voice-over order.
fn sentences_in_order(speech: &SpeechMap) -> Vec<&SpeechSentence> {
    let mut list: Vec<&SpeechSentence> = speech.sentences.iter().collect();
    list.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.beat.cmp(&b.beat)));
    list
}

/// `speech_rate`: per sentence, estimated syllables over the time from its
/// first word's start to its last word's end. Sentences with fewer than two
/// words or shorter than 0.5 s are not measured.
fn speech_rate_check(speech: &SpeechMap) -> Outcome {
    const MIN_WORDS: usize = 2;
    const MIN_SPAN: f64 = 0.5;
    let (fail, warn) = (SPEECH_RATE_FAIL, SPEECH_RATE_WARN);
    let mut status = CheckStatus::Pass;
    let mut findings = Vec::new();
    let mut rates: Vec<f64> = Vec::new();
    for sentence in sentences_in_order(speech) {
        let words = speech.words_in(sentence);
        if words.len() < MIN_WORDS {
            continue;
        }
        let start = words.iter().map(|w| w.start).fold(f64::INFINITY, f64::min);
        let end = words
            .iter()
            .map(|w| w.end)
            .fold(f64::NEG_INFINITY, f64::max);
        let span = end - start;
        if !span.is_finite() || span < MIN_SPAN {
            continue;
        }
        let syllables: usize = words.iter().map(|w| estimate_syllables(&w.text)).sum();
        let rate = syllables as f64 / span;
        rates.push(rate);
        let verdict = if rate < fail.0 || rate > fail.1 {
            CheckStatus::Fail
        } else if rate < warn.0 || rate > warn.1 {
            CheckStatus::Warn
        } else {
            continue;
        };
        status = worse(status, verdict);
        findings.push(format!(
            "beat {}: {rate:.1} syll/s ({})",
            sentence.beat + 1,
            first_words(&words)
        ));
    }
    if rates.is_empty() {
        return Outcome {
            status: CheckStatus::Skip,
            detail: format!("no sentence of {MIN_WORDS}+ words over {MIN_SPAN:.1} s"),
            findings,
        };
    }
    let lo = rates.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = rates.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Outcome {
        status,
        detail: format!(
            "{} sentence(s) at {lo:.1}-{hi:.1} syll/s (fail outside {:.1}-{:.1}, warn outside {:.1}-{:.1}), {} finding(s)",
            rates.len(),
            fail.0,
            fail.1,
            warn.0,
            warn.1,
            findings.len()
        ),
        findings,
    }
}

/// `reveal_before_speech`, `reveal_late` and the timings behind them.
struct RevealOutcome {
    before: Outcome,
    late: Outcome,
    timings: Vec<RevealTiming>,
}

/// The kicker (section label) names no conclusion by design: never judged.
fn exempt_group(group: &str) -> bool {
    matches!(
        group.split('.').next().unwrap_or(""),
        "kicker" | "kicker_rule"
    )
}

fn role_name(role: RevealRole) -> &'static str {
    match role {
        RevealRole::Content => "content",
        RevealRole::Title => "title",
        RevealRole::Label => "label",
        RevealRole::Stamp => "stamp",
        RevealRole::Value => "value",
    }
}

/// 0-based beat index of a `beat_<N>` scene id.
fn beat_of_scene(id: &str) -> Option<usize> {
    id.strip_prefix("beat_")?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

/// The script words of beat `beat` (its sentence), in order.
fn beat_words(speech: &SpeechMap, beat: usize) -> Vec<&SpeechWord> {
    speech
        .sentences
        .iter()
        .find(|s| s.beat == beat)
        .map(|s| speech.words_in(s))
        .unwrap_or_default()
}

/// Times every anchored group of every beat scene against its anchor word
/// (`ArtRecord.reveals`; the kicker is exempt) and judges both reveal checks.
/// Both SKIP when the project carries no anchors.
fn reveal_checks(
    project: &MotionProject,
    speech: &SpeechMap,
) -> Result<RevealOutcome, RenderError> {
    let reveals = project
        .project
        .art
        .as_ref()
        .map(|a| &a.reveals)
        .filter(|r| !r.is_empty());
    let Some(reveals) = reveals else {
        let skip = || Outcome {
            status: CheckStatus::Skip,
            detail: "no reveal anchors (compiled without --art)".to_string(),
            findings: Vec::new(),
        };
        return Ok(RevealOutcome {
            before: skip(),
            late: skip(),
            timings: Vec::new(),
        });
    };

    // Beat scenes with anchors, in beat order.
    let mut beats: Vec<(usize, &motion_core::scene::Scene, &Vec<RevealAnchor>)> = reveals
        .iter()
        .filter_map(|(id, anchors)| {
            let beat = beat_of_scene(id)?;
            let scene = project.scenes.iter().find(|s| s.id == *id)?;
            Some((beat, scene, anchors))
        })
        .collect();
    beats.sort_by_key(|b| b.0);

    let mut targets: Vec<GroupRef> = Vec::new();
    let mut timings: Vec<RevealTiming> = Vec::new();
    let mut unspoken = 0usize;
    for (beat, scene, anchors) in beats {
        let Some(prefix) = scene_prefix(scene) else {
            continue;
        };
        let words = beat_words(speech, beat);
        let spoken: Vec<(String, f64)> = words
            .iter()
            .map(|w| (w.text.clone(), w.start - scene.start_seconds))
            .collect();
        let mut seen: Vec<&str> = Vec::new();
        for anchor in anchors {
            let group = anchor.group.as_str();
            if group.is_empty() || exempt_group(group) || seen.contains(&group) {
                continue;
            }
            seen.push(group);
            // A group declared twice is heard at the earliest of its entries.
            let mut best: Option<(f64, RevealRole)> = None;
            for a in anchors.iter().filter(|a| a.group == group) {
                let entries: Vec<&str> = a.words.iter().map(String::as_str).collect();
                if let Some(at) = anchor_time(&spoken, &entries) {
                    if best.is_none_or(|(b, _)| at < b) {
                        best = Some((at, a.role));
                    }
                }
            }
            let Some((at, role)) = best else {
                unspoken += 1;
                continue;
            };
            let heard = words
                .iter()
                .find(|w| ((w.start - scene.start_seconds) - at).abs() < 1e-6);
            let (word, word_start, word_end) = match heard {
                Some(w) => (w.text.clone(), w.start, w.end),
                None => (
                    String::new(),
                    scene.start_seconds + at,
                    scene.start_seconds + at,
                ),
            };
            targets.push(GroupRef {
                scene: scene.id.clone(),
                prefix: prefix.clone(),
                group: group.to_string(),
            });
            timings.push(RevealTiming {
                beat: beat + 1,
                scene: scene.id.clone(),
                group: group.to_string(),
                role,
                word,
                word_start,
                word_end,
                readable_frame: None,
                readable_at: None,
            });
        }
    }

    let frames = first_readable_frames(project, &targets)?;
    let fps = project.canvas.fps.max(1);
    for (timing, frame) in timings.iter_mut().zip(frames) {
        timing.readable_frame = frame;
        timing.readable_at = frame.map(|f| motion_core::timeline::frame_time(fps, f));
    }

    // A cue the compiler had to clamp (the word comes too late in the beat to
    // wait for, `cue_clamped`) shows early by design: a warning, not a failure.
    let clamped = |t: &RevealTiming| {
        project.project.speech.as_ref().is_some_and(|s| {
            s.word_cues
                .iter()
                .any(|c| c.clamped && c.beat + 1 == t.beat && c.group == t.group)
        })
    };
    // A word that starts within the read floor of ANTICIPATE comes too late
    // in its beat for anything to wait for it and still be read (builders and
    // cues clamp such arrivals): early by design, so a warning too.
    let too_late = |t: &RevealTiming| {
        project
            .scenes
            .iter()
            .find(|s| s.id == t.scene)
            .and_then(|s| s.lifecycle.map(|l| (s.start_seconds, l)))
            .is_some_and(|(start, life)| {
                t.word_start - start > life.anticipate - CUE_MIN_READ - 1e-9
            })
    };
    let mut before = Vec::new();
    let mut before_warn = Vec::new();
    let mut late = Vec::new();
    for t in &timings {
        let head = format!("beat {}: {} ({})", t.beat, t.group, role_name(t.role));
        match t.readable_at {
            Some(at) => {
                let lead = t.word_start - at;
                if lead > REVEAL_LEAD_MAX + 1e-9 {
                    let line = format!(
                        "{head} readable at {at:.2} s, \"{}\" starts at {:.2} s (lead {:.0} ms)",
                        t.word,
                        t.word_start,
                        lead * 1000.0
                    );
                    if clamped(t) {
                        before_warn.push(format!("{line}, cue clamped by the compiler"));
                    } else if too_late(t) {
                        before_warn.push(format!(
                            "{line}, the word comes too late in the beat to wait for"
                        ));
                    } else {
                        before.push(line);
                    }
                }
                let after = at - t.word_end;
                if after > REVEAL_LATE_MAX + 1e-9 {
                    late.push(format!(
                        "{head} readable at {at:.2} s, \"{}\" ends at {:.2} s (late {:.0} ms)",
                        t.word,
                        t.word_end,
                        after * 1000.0
                    ));
                }
            }
            None => late.push(format!(
                "{head} never readable, \"{}\" ends at {:.2} s",
                t.word, t.word_end
            )),
        }
    }
    let summary = |limit: &str, findings: usize| {
        format!(
            "{} group(s) with a spoken anchor word, {unspoken} unspoken, {findings} finding(s) ({limit})",
            timings.len()
        )
    };
    let status = if !before.is_empty() {
        CheckStatus::Fail
    } else if !before_warn.is_empty() {
        CheckStatus::Warn
    } else {
        CheckStatus::Pass
    };
    let mut findings = before;
    findings.extend(before_warn);
    let before = Outcome {
        status,
        detail: summary(
            &format!(
                "readable at most {:.0} ms before the word starts; a clamped cue or a word too late in its beat only warns",
                REVEAL_LEAD_MAX * 1000.0
            ),
            findings.len(),
        ),
        findings,
    };
    let late = Outcome {
        status: if late.is_empty() {
            CheckStatus::Pass
        } else {
            CheckStatus::Warn
        },
        detail: summary(
            &format!(
                "readable at most {:.0} ms after the word ends",
                REVEAL_LATE_MAX * 1000.0
            ),
            late.len(),
        ),
        findings: late,
    };
    Ok(RevealOutcome {
        before,
        late,
        timings,
    })
}

// --- spoken_mismatch -------------------------------------------------------

/// Number words: a token made of these (hyphen-joined allowed) reads as a number.
const NUMBER_WORDS: &[&str] = &[
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
    "twenty",
    "thirty",
    "forty",
    "fifty",
    "sixty",
    "seventy",
    "eighty",
    "ninety",
    "hundred",
    "thousand",
    "million",
    "billion",
    "trillion",
    "lakh",
    "lakhs",
    "crore",
    "crores",
];

/// Currency words: a number phrase may end with one ("... billion dollars").
const CURRENCY_WORDS: &[&str] = &[
    "dollar", "dollars", "rupee", "rupees", "euro", "euros", "pound", "pounds", "cent", "cents",
];

/// Words that may continue a number phrase but not start one.
const NUMBER_JOINERS: &[&str] = &["and", "point", "percent", "per"];

/// Longest number phrase considered on either side (words).
const MAX_NUMBER_RUN: usize = 12;

/// Whether a normalised token reads as a number: it has a digit, or every
/// hyphen-joined part is a number word ("eight", "twenty-three").
fn numeric_core(norm: &str) -> bool {
    if norm.chars().any(|c| c.is_ascii_digit()) {
        return true;
    }
    let mut parts = norm.split('-').filter(|p| !p.is_empty()).peekable();
    parts.peek().is_some() && parts.all(|p| NUMBER_WORDS.contains(&p))
}

/// Whether a normalised token may sit inside a number phrase.
fn numberish(norm: &str) -> bool {
    numeric_core(norm) || NUMBER_JOINERS.contains(&norm) || CURRENCY_WORDS.contains(&norm)
}

/// `1 - distance / max(len)` over the characters of two normalised words.
fn levenshtein_ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 1.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let sub = prev[j - 1] + usize::from(a[i - 1] != b[j - 1]);
            cur[j] = sub.min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / longest as f64
}

/// Whether two number phrases say the same number ("8" = "eight", "381
/// billion" = "three hundred and eighty one billion", "30%" = "thirty
/// percent"); a trailing currency word on either side is ignored. Decided by
/// the shared anchor matcher in both directions, each side as the "spoken"
/// words of the other.
fn same_number(a: &[String], b: &[String]) -> bool {
    fn trim(words: &[String]) -> &[String] {
        let mut end = words.len();
        while end > 0 && CURRENCY_WORDS.contains(&normalize_word(&words[end - 1]).as_str()) {
            end -= 1;
        }
        &words[..end]
    }
    let (a, b) = (trim(a), trim(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let heard = |w: &[String]| -> Vec<(String, f64)> {
        w.iter()
            .enumerate()
            .map(|(i, x)| (x.clone(), i as f64))
            .collect()
    };
    let (ja, jb) = (a.join(" "), b.join(" "));
    anchor_time(&heard(b), &[ja.as_str()]) == Some(0.0)
        && anchor_time(&heard(a), &[jb.as_str()]) == Some(0.0)
}

/// What became of one script word.
#[derive(Debug, Clone, PartialEq)]
enum Fate {
    Heard,
    Missing,
    /// Heard as this other recognised word.
    Changed(String),
}

#[derive(Debug, Clone, Copy)]
enum Step {
    Start,
    Match,
    Substitute,
    Missing,
    Extra,
    /// `a` script words ↔ `b` recognised words saying the same number.
    Number(usize, usize),
}

/// Monotone alignment of the recognised words to the script words (edit
/// distance over words): a pair matches when both read as numbers with the
/// same value, or otherwise when their Levenshtein ratio on normalised words
/// is at least [`MATCH_RATIO`] (cost `1 - ratio`); a number phrase may span
/// several words on either side (cost 0). A substitution, a missing script
/// word and an extra recognised word each cost 1. Ties prefer number phrases,
/// then pairs, then a missing script word.
fn align_heard(script: &[String], heard: &[String]) -> Vec<Fate> {
    let sn: Vec<String> = script.iter().map(|w| normalize_word(w)).collect();
    let hn: Vec<String> = heard.iter().map(|w| normalize_word(w)).collect();
    let runs = |norm: &[String]| -> Vec<usize> {
        let mut out = Vec::with_capacity(norm.len());
        let mut run = 0usize;
        for w in norm {
            run = if numberish(w) {
                (run + 1).min(MAX_NUMBER_RUN)
            } else {
                0
            };
            out.push(run);
        }
        out
    };
    let (s_run, h_run) = (runs(&sn), runs(&hn));
    let (n, m) = (script.len(), heard.len());
    let width = m + 1;
    let mut cost = vec![f64::INFINITY; (n + 1) * width];
    let mut step = vec![Step::Start; (n + 1) * width];
    cost[0] = 0.0;
    for i in 0..=n {
        for j in 0..=m {
            if i == 0 && j == 0 {
                continue;
            }
            // (cost, step) of the best way into (i, j); the first of equal
            // costs wins.
            let mut best = (f64::INFINITY, Step::Start);
            let consider = |best: &mut (f64, Step), c: f64, s: Step| {
                if c < best.0 - 1e-12 {
                    *best = (c, s);
                }
            };
            if i > 0 && j > 0 {
                // Number phrases over several words on either side.
                for a in 1..=s_run[i - 1] {
                    if !numeric_core(&sn[i - a]) {
                        continue;
                    }
                    for b in 1..=h_run[j - 1] {
                        if a + b <= 2 || !numeric_core(&hn[j - b]) {
                            continue;
                        }
                        let base = cost[(i - a) * width + (j - b)];
                        if base < best.0 - 1e-12 && same_number(&script[i - a..i], &heard[j - b..j])
                        {
                            consider(&mut best, base, Step::Number(a, b));
                        }
                    }
                }
                let (s, h) = (&sn[i - 1], &hn[j - 1]);
                let pair = if numeric_core(s) && numeric_core(h) {
                    same_number(&script[i - 1..i], &heard[j - 1..j]).then_some(0.0)
                } else {
                    let r = levenshtein_ratio(s, h);
                    (r >= MATCH_RATIO).then_some(1.0 - r)
                };
                let diag = cost[(i - 1) * width + (j - 1)];
                match pair {
                    Some(c) => consider(&mut best, diag + c, Step::Match),
                    None => consider(&mut best, diag + 1.0, Step::Substitute),
                }
            }
            if i > 0 {
                consider(&mut best, cost[(i - 1) * width + j] + 1.0, Step::Missing);
            }
            if j > 0 {
                consider(&mut best, cost[i * width + (j - 1)] + 1.0, Step::Extra);
            }
            cost[i * width + j] = best.0;
            step[i * width + j] = best.1;
        }
    }
    let mut fates = vec![Fate::Heard; n];
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        match step[i * width + j] {
            Step::Match => {
                i -= 1;
                j -= 1;
            }
            Step::Substitute => {
                fates[i - 1] = Fate::Changed(heard[j - 1].clone());
                i -= 1;
                j -= 1;
            }
            Step::Missing => {
                fates[i - 1] = Fate::Missing;
                i -= 1;
            }
            Step::Extra => j -= 1,
            Step::Number(a, b) => {
                i -= a;
                j -= b;
            }
            Step::Start => break,
        }
    }
    fates
}

/// Whether a script word of a beat carries content: a number (digits or
/// number words) or a word of one of the beat's anchors.
fn content_word(word: &str, anchor_words: &[String]) -> bool {
    if numeric_core(&normalize_word(word)) {
        return true;
    }
    let spoken = [(word.to_string(), 0.0)];
    anchor_words
        .iter()
        .any(|a| anchor_time(&spoken, &[a.as_str()]).is_some())
}

/// The script words of the take in voice-over order, and the beat (0-based)
/// each belongs to.
fn script_of(speech: &SpeechMap) -> (Vec<String>, Vec<usize>) {
    let mut script: Vec<String> = Vec::new();
    let mut beats: Vec<usize> = Vec::new();
    for sentence in sentences_in_order(speech) {
        for w in speech.words_in(sentence) {
            script.push(w.text.clone());
            beats.push(sentence.beat);
        }
    }
    (script, beats)
}

/// What was heard, in time order, as written words (punctuation-only tokens
/// dropped, surrounding punctuation trimmed).
fn heard_of(recognised: &[motion_core::speech::RecognisedWord]) -> Vec<String> {
    let mut sorted: Vec<&motion_core::speech::RecognisedWord> = recognised.iter().collect();
    sorted.sort_by(|a, b| a.start.total_cmp(&b.start));
    sorted
        .iter()
        .map(|r| statement_tokens(&r.word).join(" "))
        .filter(|w| !normalize_word(w).is_empty())
        .collect()
}

/// (0.23) Per script word of the take, in voice-over order, whether
/// `recognised` has it as written: the whole-take alignment `spoken_mismatch`
/// runs (numbers by value, fuzzy spelling at [`MATCH_RATIO`]), for any
/// recognised words, not only `speech.recognised`. `mix_intelligibility`
/// (the CLI's `qa_intel`) compares the share heard on the final mix against the
/// clean voice with it.
pub fn script_words_heard(
    speech: &SpeechMap,
    recognised: &[motion_core::speech::RecognisedWord],
) -> Vec<(String, bool)> {
    let (script, _) = script_of(speech);
    let fates = align_heard(&script, &heard_of(recognised));
    script
        .into_iter()
        .zip(fates)
        .map(|(word, fate)| (word, fate == Fate::Heard))
        .collect()
}

/// `spoken_mismatch`: the recognised words against the script words. The
/// whole take is aligned at once (sentence windows never split a word from
/// its match); differences are reported per beat. SKIP without recognised
/// words.
fn spoken_mismatch_check(project: &MotionProject, speech: &SpeechMap) -> Outcome {
    if speech.recognised.is_empty() {
        return Outcome {
            status: CheckStatus::Skip,
            detail: "no recognised words (onset timing)".to_string(),
            findings: Vec::new(),
        };
    }
    let (script, beats) = script_of(speech);
    let heard = heard_of(&speech.recognised);
    let fates = align_heard(&script, &heard);

    // Anchor words per beat (each entry split into single words).
    let anchors_of = |beat: usize| -> Vec<String> {
        project
            .project
            .art
            .as_ref()
            .and_then(|a| a.reveals.get(&format!("beat_{}", beat + 1)))
            .map(|list| {
                list.iter()
                    .flat_map(|a| a.words.iter())
                    .flat_map(|w| w.split(|c: char| c.is_whitespace() || c == '_'))
                    .filter(|w| !w.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut findings = Vec::new();
    let (mut content, mut other) = (0usize, 0usize);
    let mut anchor_words: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for ((word, beat), fate) in script.iter().zip(&beats).zip(&fates) {
        let text = match fate {
            Fate::Heard => continue,
            Fate::Missing => format!("beat {}: missing \"{word}\"", beat + 1),
            Fate::Changed(h) => format!("beat {}: \"{word}\" heard as \"{h}\"", beat + 1),
        };
        let words = anchor_words
            .entry(*beat)
            .or_insert_with(|| anchors_of(*beat));
        if content_word(word, words) {
            content += 1;
        } else {
            other += 1;
        }
        findings.push(text);
    }
    let status = if content > 0 {
        CheckStatus::Fail
    } else if other > 0 {
        CheckStatus::Warn
    } else {
        CheckStatus::Pass
    };
    Outcome {
        status,
        detail: format!(
            "{} script word(s), {} recognised: {content} content and {other} other word(s) not heard as written",
            script.len(),
            heard.len()
        ),
        findings,
    }
}

/// Integrated loudness, true peak (ebur128) and sample peak (decoded) of the
/// first audio stream. `None` when gated out / silent.
fn measure_mix(file: &Path) -> Result<Option<MixMeasure>, RenderError> {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(file)
        .args([
            "-map",
            "0:a:0",
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    if !out.status.success() {
        return Err(RenderError::Encode(format!(
            "measuring {} failed: {}",
            file.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let Some(at) = err.rfind("Summary:") else {
        return Ok(None);
    };
    let summary = &err[at..];
    let value = |label: &str, unit: &str| -> Option<f64> {
        let line = summary
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(label) && l.ends_with(unit))?;
        line[label.len()..line.len() - unit.len()]
            .trim()
            .parse()
            .ok()
    };
    let (Some(integrated), Some(true_peak)) = (value("I:", "LUFS"), value("Peak:", "dBFS")) else {
        return Ok(None);
    };
    if !integrated.is_finite() || integrated <= -70.0 {
        return Ok(None);
    }
    // Sample peak over the native channels (a mono downmix of dual-mono
    // material reads 3 dB hot).
    let raw = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-f", "f32le", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    let peak = raw
        .stdout
        .chunks_exact(4)
        .map(|b| f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]])).abs())
        .fold(0.0f64, f64::max);
    let sample_peak = 20.0 * peak.max(1e-9).log10();
    Ok(Some(MixMeasure {
        integrated,
        true_peak,
        sample_peak,
    }))
}

/// Resolve the speech audio path (relative to the speech file's directory).
pub fn voice_path(speech_file: &Path, speech: &SpeechMap) -> PathBuf {
    speech_file
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .join(&speech.audio)
}

#[cfg(test)]
mod timing_tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn numbers_match_written_or_said() {
        assert!(same_number(&words("8"), &words("eight")));
        assert!(same_number(
            &words("$381 billion"),
            &words("three hundred and eighty one billion dollars")
        ));
        assert!(same_number(&words("thirty percent"), &words("30%")));
        assert!(!same_number(&words("eight"), &words("eighty")));
        assert!(!same_number(&words("381"), &words("381 billion")));
    }

    #[test]
    fn alignment_reports_missing_and_changed_words() {
        let script = words("Sunlight takes eight minutes to reach us");
        let fates = align_heard(&script, &words("sunlight takes 8 minutes to reach us"));
        assert!(fates.iter().all(|f| *f == Fate::Heard), "{fates:?}");
        let fates = align_heard(&script, &words("sunlight takes minutes to reach us"));
        assert_eq!(fates[2], Fate::Missing, "{fates:?}");
        let fates = align_heard(&script, &words("sunlight takes eighty minutes to reach us"));
        assert_eq!(fates[2], Fate::Changed("eighty".into()), "{fates:?}");
        // Spoken numbers over several words on either side.
        let fates = align_heard(
            &words("he holds three hundred and eighty one billion dollars in cash"),
            &words("he holds $381 billion in cash"),
        );
        assert!(fates.iter().all(|f| *f == Fate::Heard), "{fates:?}");
        // Fuzzy spelling matches.
        let fates = align_heard(&words("the colour red"), &words("the color red"));
        assert!(fates.iter().all(|f| *f == Fate::Heard), "{fates:?}");
    }

    #[test]
    fn content_words_are_numbers_and_anchor_words() {
        let anchors = vec!["earth".to_string(), "globe".to_string(), "8".to_string()];
        assert!(content_word("eight", &anchors));
        assert!(content_word("globes", &anchors));
        assert!(content_word("1969", &[]));
        assert!(!content_word("the", &anchors));
        assert!(!content_word("planet", &anchors));
    }
}
