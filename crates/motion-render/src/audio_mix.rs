//! Mix an AudioPlan into a rendered MP4 via ffmpeg (0.8).
//! Executes the plan only; no planning here. See docs/SOUND_DESIGN.md §4.
//!
//! (0.23) With `plan.levels = Some(_)` and a voice-over the mix is voice-relative:
//! the voice is normalised at mix time ([`voice_gain_db`]), the bed has no
//! sidechain and follows the speech-led gain envelope (`DuckEnvelope`) against
//! the voice, a wide-range bed is compressed gently (`BED_COMPRESSOR`) and the
//! SFX bus is trimmed to stay under the voice (`probe_sfx_trim`) and the bed's
//! level in long narration gaps is capped so its momentary peaks stay under
//! the voice (`cap_long_gaps`). A peaky voice (its peaks would pass
//! `VOICE_TRUE_PEAK_DB` at the gain that reaches `VOICE_LUFS`) also goes
//! through a transparent limiter on the voice stem, after the static gain and
//! before the mix ([`VOICE_LIMITER_ATTACK_MS`], [`VOICE_LIMIT_MAX_DB`]).
//! Without `levels` the graph is the pre-0.23 one, byte for byte.
//! [`write_stems`] runs the same graph and writes the `voice`, `bed` and `sfx`
//! stems.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use motion_core::audio::{
    AudioPlan, DuckEnvelope, MixLevels, MusicBed, SfxLibrary, BED_LRA_COMPRESS_LU, VOICE_LUFS,
    VOICE_TRUE_PEAK_DB,
};
use motion_core::checks::BED_OVER_VOICE_MAX_DB;

use crate::RenderError;

const SAMPLE_RATE: u32 = 48_000;
const BASE_FORMAT: &str = "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo";

/// (0.9) Optional loudness makeup for the final mix. `Off` (the default) is
/// the 0.8 mix byte-for-byte.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MakeupGain {
    #[default]
    Off,
    /// Two-pass: measure the mix's integrated LUFS, then apply a static gain
    /// toward `plan.loudness_target`, limited by the headroom under -1 dBTP.
    Auto,
}

/// (0.10) Gain reduction of the music bed while the voice-over speaks (dB).
pub const BED_DUCK_DB: f64 = 10.0;
/// (0.10) Gain reduction of the SFX bus while the voice-over speaks (dB).
pub const SFX_DUCK_DB: f64 = 4.0;
const BED_DUCK_RATIO: f64 = 20.0;
const SFX_DUCK_RATIO: f64 = 4.0;
/// The voice key is leveled before it reaches `sidechaincompress`: boosted by
/// [`KEY_BOOST_DB`] and limited at 0.1 (-20 dBFS), so any voiced audio above
/// about -40 dBFS becomes a near-constant key whatever the voice's loudness or
/// dynamics (a fixed-ratio compressor on raw speech ducks 6..12 dB word by
/// word). Measured with ffmpeg's rms detector (attack 15 ms) the leveled key
/// reads [`KEY_EFFECTIVE_DB`], from which the thresholds follow.
const KEY_CHAIN: &str = "volume=20dB,alimiter=limit=0.1:attack=2:release=40:level=disabled";
const KEY_EFFECTIVE_DB: f64 = -21.6;

/// (0.10) A voice-over placed on the mix. It is the KEY of the ducking (music
/// bed -10 dB, SFX bus -4 dB while it speaks) and is itself never ducked.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceTrack {
    /// Audio file (any format ffmpeg reads).
    pub path: PathBuf,
    /// Project seconds at which the file starts (>= 0).
    pub offset: f64,
    /// (0.23) Static gain applied to the voice in the mix (dB): the
    /// normalisation to `audio::VOICE_LUFS` measured at mix time. 0.0 = the
    /// file as recorded (pre-0.23 behaviour).
    pub gain_db: f64,
}

/// Mix `plan` under `video` (copied, not re-encoded) into `output` with an
/// AAC 192k stereo 48 kHz track of exactly `duration` seconds.
pub fn mix_audio(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    output: &Path,
) -> Result<(), RenderError> {
    mix_audio_with(
        video,
        plan,
        sfx_root,
        library,
        music_root,
        duration,
        output,
        MakeupGain::Off,
        None,
    )
}

/// [`mix_audio`] with an explicit [`MakeupGain`] (0.9) and an optional
/// [`VoiceTrack`] (0.10). `voice = None` is the 0.9 graph byte-for-byte. With a
/// voice and [`MakeupGain::Auto`] the static gain may also cut (toward the
/// plan's loudness target) since a voice-over is often hotter than the target.
#[allow(clippy::too_many_arguments)]
pub fn mix_audio_with(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    output: &Path,
    makeup: MakeupGain,
    voice: Option<&VoiceTrack>,
) -> Result<(), RenderError> {
    mix_audio_ex(
        video,
        plan,
        sfx_root,
        library,
        music_root,
        duration,
        output,
        &MixOptions {
            makeup,
            voice,
            ..MixOptions::default()
        },
    )
}

/// (0.23) The speech-led half of a voice-relative mix. It only takes effect
/// when `plan.levels` is `Some` and a voice-over is mixed; without it the mix
/// is the pre-0.23 one (absolute bed, voice sidechain).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoiceRelative {
    /// The bed's level against the voice over the voice timeline
    /// ([`DuckEnvelope::from_speech`]); `None` = flat `levels.under_speech_db`.
    pub envelope: Option<DuckEnvelope>,
    /// Integrated loudness of the bed track (LUFS, `MusicPlan.lufs`); `None` =
    /// measured at mix time.
    pub bed_lufs: Option<f64>,
    /// Loudness range of the bed track (LU, `MusicPlan.lra`); `None` =
    /// measured at mix time.
    pub bed_lra: Option<f64>,
    /// The voiced segments on the voice timeline ([`voiced_spans`]): the SFX
    /// bus is trimmed so its momentary loudness inside them stays at
    /// `levels.sfx_under_speech_db` under the voice. Empty = no trim.
    pub voiced: Vec<(f64, f64)>,
}

/// (0.23) Everything [`mix_audio_ex`] takes beyond the plan, the sounds and
/// the output.
#[derive(Debug, Clone, Copy, Default)]
pub struct MixOptions<'a> {
    pub makeup: MakeupGain,
    pub voice: Option<&'a VoiceTrack>,
    /// Used with `plan.levels = Some(_)` and a voice (see [`VoiceRelative`]).
    pub relative: Option<&'a VoiceRelative>,
    /// [`mix_audio_ex`] only: also write the `voice`, `bed` and `sfx` stems
    /// (see [`write_stems`]) into this directory.
    pub stems: Option<&'a Path>,
}

/// The stem files [`mix_audio_ex`] / [`write_stems`] wrote (each present only
/// when that part exists in the mix).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StemFiles {
    pub voice: Option<PathBuf>,
    pub bed: Option<PathBuf>,
    pub sfx: Option<PathBuf>,
}

/// [`mix_audio_with`] with the (0.23) voice-relative inputs and stems.
#[allow(clippy::too_many_arguments)]
pub fn mix_audio_ex(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    output: &Path,
    opts: &MixOptions,
) -> Result<(), RenderError> {
    mix_audio_report(
        video, plan, sfx_root, library, music_root, duration, output, opts,
    )
    .map(|_| ())
}

/// (0.23) What a mix decided and recorded: the numbers [`mix_audio_report`]
/// also prints to stderr (the mix log).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MixReport {
    /// Makeup gain ahead of the final limiter (dB; 0.0 = none), measured on
    /// the first-pass mix. The stems are pre-makeup: the shipped mix is the
    /// sum of the stems plus this gain, limited.
    pub makeup_db: f64,
    /// Static trim of the SFX bus (dB, <= 0); 0.0 without SFX or a voice.
    pub sfx_trim_db: f64,
    /// The bed's long-gap cap; `None` without a voice-relative bed or without
    /// long narration gaps.
    pub gap_cap: Option<GapCap>,
}

/// (0.23) The long-gap cap of a voice-relative mix (see `cap_long_gaps`). All
/// levels are dB against the voice's integrated loudness in the mix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GapCap {
    /// Worst momentary (400 ms) bed level in the long narration gaps before
    /// the cap.
    pub before_db: f64,
    /// The same after the cap (the last metering pass).
    pub after_db: f64,
    /// Where the cap holds the worst momentary:
    /// `BED_OVER_VOICE_MAX_DB - GAP_CAP_MARGIN_DB`.
    pub ceiling_db: f64,
    /// How far the long-gap level was lowered (dB, <= 0; 0.0 = within the
    /// ceiling already).
    pub lowered_db: f64,
}

impl GapCap {
    fn log_line(&self) -> String {
        format!(
            "bed gap cap: {:+.2} dB (worst bed momentary in long gaps {:.1} -> {:.1} dB over the voice, ceiling {:.1})",
            self.lowered_db, self.before_db, self.after_db, self.ceiling_db
        )
    }
}

/// (A5c) The mix log's line for a peaky voice.
fn voice_limit_log_line(r: &Resolved, l: &VoiceLimit) -> String {
    format!(
        "voice limiter: {:+.2} dB static gain, ceiling {:.1} dBFS ({} ms attack, {} ms release), up to {:.1} dB peak reduction (voice I {:.1} -> {:.1} LUFS, true peak {:.1} dBTP before)",
        r.voice.gain_db,
        voice_limiter_ceiling_db(),
        VOICE_LIMITER_ATTACK_MS,
        VOICE_LIMITER_RELEASE_MS,
        l.peak_reduction_db,
        l.raw_lufs,
        r.voice_lufs,
        l.raw_true_peak
    )
}

/// [`mix_audio_ex`] that also returns the [`MixReport`].
#[allow(clippy::too_many_arguments)]
pub fn mix_audio_report(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    output: &Path,
    opts: &MixOptions,
) -> Result<MixReport, RenderError> {
    if opts.stems.is_some() && opts.voice.is_none() {
        return Err(RenderError::Encode(
            "mix: stems need a voice-over".to_string(),
        ));
    }
    if let Some(dir) = opts.stems {
        std::fs::create_dir_all(dir)?;
    }
    let prep = prepare(video, plan, sfx_root, library, music_root, duration, opts)?;
    let voice = prep.voice(opts);
    let allow_cut = voice.is_some();
    let mut report = MixReport {
        makeup_db: 0.0,
        sfx_trim_db: prep.resolved.as_ref().map_or(0.0, |r| r.sfx_trim_db),
        gap_cap: prep.resolved.as_ref().and_then(|r| r.gap_cap),
    };
    if let Some(limit) = prep
        .resolved
        .as_ref()
        .and_then(|r| r.voice_limit.map(|l| (r, l)))
    {
        eprintln!("{}", voice_limit_log_line(limit.0, &limit.1));
    }
    if let Some(cap) = report.gap_cap {
        eprintln!("{}", cap.log_line());
    }
    let spec = |sink: Sink<'_>, makeup_db: Option<f64>, stems: Option<&Path>| {
        build_args_for(&BuildSpec {
            video,
            plan,
            sfx_root,
            library,
            music: prep.music.as_ref(),
            duration,
            sink,
            makeup_db,
            voice,
            rel: prep.resolved.as_ref(),
            stems,
            probe: false,
        })
    };
    match opts.makeup {
        MakeupGain::Off => {
            let (args, _) = spec(Sink::Mp4(output), None, opts.stems)?;
            run_ffmpeg(&args)?;
            Ok(report)
        }
        MakeupGain::Auto => {
            // Pass 1: the ordinary mix, to a sibling temp file, then measured.
            let temp = temp_sibling(output);
            let (args, _) = spec(Sink::Mp4(&temp), None, opts.stems)?;
            let result = (|| {
                run_ffmpeg(&args)?;
                let measured = measure(&temp);
                let gain = measured.and_then(|m| {
                    makeup_gain_db_with(m, plan.loudness_target, allow_cut).map(|g| (m, g))
                });
                match gain {
                    Some((m, g)) => {
                        eprintln!(
                            "makeup gain: {g:+.1} dB (I {:.1} -> {:.1} LUFS, true peak {:.1} dBTP before)",
                            m.integrated,
                            m.integrated + g,
                            m.true_peak
                        );
                        report.makeup_db = g;
                        // Pass 2: the same graph with the gain before the limiter
                        // (the stems, if any, were written by pass 1).
                        let (args, _) = spec(Sink::Mp4(output), Some(g), None)?;
                        run_ffmpeg(&args)
                    }
                    None => {
                        match measured {
                            Some(m) => eprintln!(
                                "makeup gain: +0.0 dB (I {:.1} LUFS, true peak {:.1} dBTP)",
                                m.integrated, m.true_peak
                            ),
                            None => eprintln!("makeup gain: +0.0 dB (mix unmeasurable or silent)"),
                        }
                        // Pass 1 is already the final mix.
                        std::fs::rename(&temp, output)
                            .or_else(|_| std::fs::copy(&temp, output).map(|_| ()))
                            .map_err(|e| {
                                RenderError::Encode(format!(
                                    "mix: could not write {}: {e}",
                                    output.display()
                                ))
                            })
                    }
                }
            })();
            let _ = std::fs::remove_file(&temp);
            result.map(|()| report)
        }
    }
}

/// (0.23) What the long-gap cap decides for this mix (`None`: no
/// voice-relative bed, or no long narration gap). Runs the same measurements
/// as the mix, without mixing.
#[allow(clippy::too_many_arguments)]
pub fn gap_cap(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    opts: &MixOptions,
) -> Result<Option<GapCap>, RenderError> {
    let prep = prepare(video, plan, sfx_root, library, music_root, duration, opts)?;
    Ok(prep.resolved.and_then(|r| r.gap_cap))
}

/// (0.23) Run the mix graph and write only the stems (`voice.wav`, `bed.wav`,
/// `sfx.wav` in `dir`; float wav, 48 kHz stereo, post-gain, post-envelope,
/// pre-makeup, project duration). The graph is the one [`mix_audio_ex`] runs;
/// `video` only stands in as input 0. Needs a voice-over.
#[allow(clippy::too_many_arguments)]
pub fn write_stems(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    dir: &Path,
    opts: &MixOptions,
) -> Result<StemFiles, RenderError> {
    if opts.voice.is_none() {
        return Err(RenderError::Encode(
            "mix: stems need a voice-over".to_string(),
        ));
    }
    let prep = prepare(video, plan, sfx_root, library, music_root, duration, opts)?;
    std::fs::create_dir_all(dir)?;
    let (args, files) = build_args_for(&BuildSpec {
        video,
        plan,
        sfx_root,
        library,
        music: prep.music.as_ref(),
        duration,
        sink: Sink::Null,
        makeup_db: None,
        voice: prep.voice(opts),
        rel: prep.resolved.as_ref(),
        stems: Some(dir),
        probe: false,
    })?;
    run_ffmpeg(&args)?;
    Ok(files)
}

/// (0.23) The filter graph [`mix_audio_ex`] would run (no makeup gain), for
/// tests and diagnostics. With `plan.levels` it measures the voice (and the
/// bed when its loudness is not given) with ffmpeg; otherwise it is pure.
#[allow(clippy::too_many_arguments)]
pub fn mix_graph(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    opts: &MixOptions,
) -> Result<String, RenderError> {
    let prep = prepare(video, plan, sfx_root, library, music_root, duration, opts)?;
    let (args, _) = build_args_for(&BuildSpec {
        video,
        plan,
        sfx_root,
        library,
        music: prep.music.as_ref(),
        duration,
        sink: Sink::Null,
        makeup_db: None,
        voice: prep.voice(opts),
        rel: prep.resolved.as_ref(),
        stems: None,
        probe: false,
    })?;
    graph_of(&args)
}

fn graph_of(args: &[String]) -> Result<String, RenderError> {
    args.iter()
        .position(|a| a == "-filter_complex")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .ok_or_else(|| RenderError::Encode("mix: no filter graph".to_string()))
}

/// The plan's music bed with its file (when a music root was given).
fn music_file<'a>(
    plan: &'a AudioPlan,
    music_root: Option<&Path>,
) -> Option<(&'a MusicBed, PathBuf)> {
    match (plan.music.as_ref(), music_root) {
        (Some(m), Some(root)) => Some((m, root.join(&m.track))),
        _ => None,
    }
}

/// What every entry point resolves before it builds the graph.
struct Prepared<'a> {
    music: Option<(&'a MusicBed, PathBuf)>,
    /// `Some` for a voice-relative mix (`plan.levels` with a voice-over).
    resolved: Option<Resolved>,
}

impl<'a> Prepared<'a> {
    /// The voice the graph mixes: the normalised copy, else the caller's.
    fn voice<'b>(&'b self, opts: &'b MixOptions<'_>) -> Option<&'b VoiceTrack> {
        self.resolved.as_ref().map(|r| &r.voice).or(opts.voice)
    }
}

/// Validate the inputs and, for a voice-relative mix, measure the voice and
/// the bed and trim the SFX bus ([`resolve_relative`], [`probe_sfx_trim`]).
fn prepare<'a>(
    video: &Path,
    plan: &'a AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    opts: &MixOptions,
) -> Result<Prepared<'a>, RenderError> {
    check_inputs(duration, opts.voice)?;
    let music = music_file(plan, music_root);
    let mut resolved = resolve_relative(plan, opts.voice, opts.relative, music.as_ref())?;
    if let (Some(r), Some(levels), Some(rel)) = (resolved.as_mut(), plan.levels, opts.relative) {
        if !plan.cues.is_empty() && !rel.voiced.is_empty() {
            let trim = probe_sfx_trim(
                &BuildSpec {
                    video,
                    plan,
                    sfx_root,
                    library,
                    music: music.as_ref(),
                    duration,
                    sink: Sink::Null,
                    makeup_db: None,
                    voice: Some(&r.voice),
                    rel: Some(r),
                    stems: None,
                    probe: true,
                },
                &levels,
                &rel.voiced,
                r.voice_lufs,
                r.voice.offset,
            )?;
            r.sfx_trim_db = trim;
        }
    }
    // (0.23) The bed's momentary peaks in long narration gaps.
    if let (Some(r), Some((bed, file))) = (resolved.as_mut(), music.as_ref()) {
        if file.is_file() {
            cap_long_gaps(r, bed, file, duration)?;
        }
    }
    Ok(Prepared { music, resolved })
}

fn check_inputs(duration: f64, voice: Option<&VoiceTrack>) -> Result<(), RenderError> {
    if !(duration.is_finite() && duration > 0.0) {
        return Err(RenderError::Encode(format!(
            "mix: invalid duration {duration}"
        )));
    }
    if let Some(v) = voice {
        if !(v.offset.is_finite() && v.offset >= 0.0) {
            return Err(RenderError::Encode(format!(
                "mix: invalid voice offset {}",
                v.offset
            )));
        }
        if !v.path.is_file() {
            return Err(RenderError::Asset(format!(
                "voice file {} not found",
                v.path.display()
            )));
        }
    }
    Ok(())
}

/// Loudness of a finished mix (ebur128 summary).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Measured {
    /// Integrated loudness, LUFS.
    integrated: f64,
    /// True peak, dBTP.
    true_peak: f64,
}

/// Peak reduction the final limiter may do for voice-over loudness (dB).
pub const LIMITER_ALLOWANCE_DB: f64 = 3.0;

/// Static makeup gain in dB toward `target` LUFS without pushing the true peak
/// above -1.0 dBTP (0.1 dB margin), clamped to [0, 24]. `None` when it would
/// be below 0.05 dB or the mix is silent / unmeasurable.
#[cfg(test)]
fn makeup_gain_db(m: Measured, target: f64) -> Option<f64> {
    makeup_gain_db_with(m, target, false)
}

/// [`makeup_gain_db`]; `allow_cut` lets the gain go negative (down to -24 dB)
/// when the mix is hotter than `target` (voice-over mixes).
fn makeup_gain_db_with(m: Measured, target: f64, allow_cut: bool) -> Option<f64> {
    if !m.integrated.is_finite() || m.integrated <= -70.0 || !target.is_finite() {
        return None;
    }
    // A missing/-inf true peak means no peak constraint from the measurement.
    let headroom = if m.true_peak.is_finite() {
        -1.0 - m.true_peak - 0.1
    } else {
        24.0
    };
    let lo = if allow_cut { -24.0 } else { 0.0 };
    // (0.10 Q) Voice-over mixes may push up to LIMITER_ALLOWANCE_DB of peaks
    // into the final −1 dBTP limiter (the gain sits before it), so a peaky
    // narrator still reaches the loudness target.
    let headroom = if allow_cut {
        headroom + LIMITER_ALLOWANCE_DB
    } else {
        headroom
    };
    let gain = (target - m.integrated).min(headroom).clamp(lo, 24.0);
    (gain.abs() >= 0.05).then_some(gain)
}

fn temp_sibling(output: &Path) -> std::path::PathBuf {
    let name = output
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mix.mp4".into());
    output.with_file_name(format!(".{name}.makeup1.mp4"))
}

/// Integrated loudness and true peak of `file`'s first audio stream.
fn measure(file: &Path) -> Option<Measured> {
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
        .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_ebur128_summary(&String::from_utf8_lossy(&out.stderr))
}

/// Parse "I:" and "Peak:" from the ebur128 summary block (the last one).
fn parse_ebur128_summary(stderr: &str) -> Option<Measured> {
    let summary = &stderr[stderr.rfind("Summary:")?..];
    let value = |label: &str, unit: &str| -> Option<f64> {
        let line = summary
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(label) && l.ends_with(unit))?;
        let text = line[label.len()..line.len() - unit.len()].trim();
        match text {
            "-inf" => Some(f64::NEG_INFINITY),
            t => t.parse().ok(),
        }
    };
    Some(Measured {
        integrated: value("I:", "LUFS")?,
        true_peak: value("Peak:", "dBFS")?,
    })
}

// ---------------------------------------------------------------------------
// (0.23) Voice-relative mix
// ---------------------------------------------------------------------------

/// Gentle compressor of a bed whose loudness range exceeds
/// `BED_LRA_COMPRESS_LU` (a swelling bed would rise out of its place under the
/// narration): threshold 0.1 (-20 dBFS RMS), ratio 3:1, attack 50 ms, release
/// 500 ms, soft knee, no makeup. Measured on the 7 committed beds it takes
/// cinematic_strings from 16 LU to 7.6 LU and tech_pulse from 12.5 to 5.8 LU
/// and removes the swells that peak 6..8 dB above the bed's integrated level.
const BED_COMPRESSOR: &str =
    "acompressor=threshold=0.1:ratio=3:attack=50:release=500:knee=4:makeup=1";
/// The bed's gain automation is evaluated per frame of this many samples
/// (5.3 ms at 48 kHz), so a 150 ms ramp is 28 steps, not 6.
const BED_GAIN_FRAME_SAMPLES: u32 = 256;
/// Largest static gain the voice normalisation may apply (dB).
const VOICE_GAIN_LIMIT_DB: f64 = 30.0;

/// The measured, resolved half of a voice-relative mix (`plan.levels = Some`
/// with a voice-over).
#[derive(Debug, Clone)]
struct Resolved {
    /// The voice with its normalising gain set (`gain_db`).
    voice: VoiceTrack,
    /// The voice's integrated loudness in the mix (LUFS): the reference the
    /// bed and the SFX bus sit under.
    voice_lufs: f64,
    /// (A5c) The voice goes through the voice limiter (`None`: plain gain).
    voice_limit: Option<VoiceLimit>,
    /// Gain (dB) applied to the bed file against the project timeline, as
    /// (seconds, dB) points; linear between points, held at the ends.
    bed_gain: Vec<(f64, f64)>,
    /// A gentle compressor sits ahead of the bed's gain (`BED_COMPRESSOR`).
    compress: bool,
    /// Static trim of the SFX bus (dB, <= 0).
    sfx_trim_db: f64,
    /// Narration gaps of at least `levels.long_gap_s` on the project timeline
    /// (from `VoiceRelative.voiced`; empty without it).
    long_gaps: Vec<(f64, f64)>,
    /// Indexes into `bed_gain` of the points that hold the bed at
    /// `levels.long_gap_db` inside a long gap: the points the cap lowers.
    gap_points: Vec<usize>,
    /// What `cap_long_gaps` decided (`None`: not metered).
    gap_cap: Option<GapCap>,
}

/// The static gain (dB) that brings a voice measured at `integrated` LUFS and
/// `true_peak` dBTP to [`VOICE_LUFS`], reduced when needed so the voice's true
/// peak after the gain is at most [`VOICE_TRUE_PEAK_DB`]. Rounded to 0.01 dB.
pub fn voice_gain_db(integrated: f64, true_peak: f64) -> f64 {
    let mut gain = VOICE_LUFS - integrated;
    if true_peak.is_finite() {
        gain = gain.min(VOICE_TRUE_PEAK_DB - true_peak);
    }
    let gain = gain.clamp(-VOICE_GAIN_LIMIT_DB, VOICE_GAIN_LIMIT_DB);
    (gain * 100.0).floor() / 100.0
}

/// (A5c) The voice limiter's look-ahead / attack (ms). `alimiter` delays its
/// output by this much; the graph trims it again, as for the final limiter.
pub const VOICE_LIMITER_ATTACK_MS: f64 = 5.0;
/// (A5c) The voice limiter's release (ms): long enough that a plosive's
/// reduction recovers smoothly instead of ringing at the voice's pitch.
pub const VOICE_LIMITER_RELEASE_MS: f64 = 100.0;
/// (A5c) The limiter's sample-peak ceiling sits this far (dB) under
/// [`VOICE_TRUE_PEAK_DB`]: the inter-sample peaks of a limited voice add about
/// 0.1..0.3 dB, so its true peak stays at or under the ceiling.
pub const VOICE_LIMITER_MARGIN_DB: f64 = 0.3;
/// (A5c) Most peak reduction (dB) the voice limiter may do. A voice that would
/// need more lands under [`VOICE_LUFS`] and the final makeup gain (limited by
/// `LIMITER_ALLOWANCE_DB`) makes up the rest.
pub const VOICE_LIMIT_MAX_DB: f64 = 6.0;
/// (A5c) A voice whose peak-capped gain is within this many dB of the loudness
/// gain is left to the plain static gain (no limiter): it lands that close to
/// [`VOICE_LUFS`].
const VOICE_LIMIT_MIN_DB: f64 = 0.5;
/// (A5c) Measure-and-correct passes of the limited voice's gain (each one
/// moves the gain by the remaining loudness error; the first guess is within
/// a fraction of a dB, so two corrections converge).
const VOICE_LIMIT_PASSES: usize = 3;

/// (A5c) The voice limiter's sample-peak ceiling in dBFS.
pub fn voice_limiter_ceiling_db() -> f64 {
    VOICE_TRUE_PEAK_DB - VOICE_LIMITER_MARGIN_DB
}

/// The voice limiter as an ffmpeg filter (no delay compensation).
fn voice_limiter() -> String {
    format!(
        "alimiter=limit={:.4}:attack={}:release={}:level=disabled",
        10f64.powf(voice_limiter_ceiling_db() / 20.0),
        VOICE_LIMITER_ATTACK_MS,
        VOICE_LIMITER_RELEASE_MS
    )
}

/// The voice limiter inside the mix graph: padded by its look-ahead before it
/// (padding after an `alimiter` flakes under ffmpeg 8.1) and trimmed by the
/// same afterwards, so the voice stays on the timeline.
fn voice_limiter_stage() -> String {
    let delay = VOICE_LIMITER_ATTACK_MS / 1000.0;
    format!(
        ",apad=pad_dur={delay:.3},{},atrim=start={delay:.3},asetpts=PTS-STARTPTS",
        voice_limiter()
    )
}

/// (A5c) The static gain (dB) of a peaky voice that goes through the voice
/// limiter: `Some(gain)` when the gain that reaches [`VOICE_LUFS`] would put
/// the voice's true peak more than 0.5 dB over [`VOICE_TRUE_PEAK_DB`]; the
/// gain is then the loudness gain, reduced so the limiter takes at most
/// [`VOICE_LIMIT_MAX_DB`] off the peak (rounded down to 0.01 dB). `None`: the
/// plain [`voice_gain_db`] does it. The limiter costs a little loudness, so
/// the mix corrects this first guess by measuring (`normalise_voice`).
pub fn voice_limited_gain_db(integrated: f64, true_peak: f64) -> Option<f64> {
    if !true_peak.is_finite() {
        return None;
    }
    let want = VOICE_LUFS - integrated;
    if want - (VOICE_TRUE_PEAK_DB - true_peak) <= VOICE_LIMIT_MIN_DB {
        return None;
    }
    Some(limited_gain(want, true_peak))
}

/// `want` dB of gain, capped at [`VOICE_LIMIT_MAX_DB`] of peak reduction
/// (against the limiter's ceiling) and at the gain range; floored to 0.01 dB.
fn limited_gain(want: f64, true_peak: f64) -> f64 {
    let cap = voice_limiter_ceiling_db() + VOICE_LIMIT_MAX_DB - true_peak;
    let gain = want
        .min(cap)
        .clamp(-VOICE_GAIN_LIMIT_DB, VOICE_GAIN_LIMIT_DB);
    (gain * 100.0).floor() / 100.0
}

/// Integrated loudness, true peak and loudness range of `file`'s first audio
/// stream after `chain` (filters ahead of the meter).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Loudness {
    integrated: f64,
    true_peak: f64,
    lra: f64,
}

fn measure_chain(file: &Path, chain: &str) -> Option<Loudness> {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(file)
        .args([
            "-map",
            "0:a:0",
            "-af",
            &format!("{chain},ebur128=peak=true"),
            "-f",
            "null",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stderr);
    let m = parse_ebur128_summary(&text)?;
    let summary = &text[text.rfind("Summary:")?..];
    let lra = summary
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("LRA:") && l.ends_with("LU"))
        .and_then(|l| l["LRA:".len()..l.len() - "LU".len()].trim().parse().ok())
        .unwrap_or(0.0);
    Some(Loudness {
        integrated: m.integrated,
        true_peak: m.true_peak,
        lra,
    })
}

/// (A5c) What the voice limiter does to this voice.
#[derive(Debug, Clone, Copy, PartialEq)]
struct VoiceLimit {
    /// The raw voice's integrated loudness and true peak (dB).
    raw_lufs: f64,
    raw_true_peak: f64,
    /// Peak reduction (dB) at the voice's loudest peak: raw true peak plus
    /// the static gain minus the limiter's ceiling.
    peak_reduction_db: f64,
}

/// A voice's static gain, the loudness it ends at in the mix and the limiter
/// it needs (if any).
#[derive(Debug, Clone, Copy, PartialEq)]
struct VoiceNorm {
    gain_db: f64,
    lufs: f64,
    limit: Option<VoiceLimit>,
}

/// The voice's gain for a raw voice measured as `m`. A voice whose peaks stay
/// under [`VOICE_TRUE_PEAK_DB`] at the gain that reaches [`VOICE_LUFS`] gets
/// the plain gain ([`voice_gain_db`]). A peaky one gets the gain that reaches
/// [`VOICE_LUFS`] (reduced only when the limiter would take more than
/// [`VOICE_LIMIT_MAX_DB`] off the peak) and the voice limiter after it. The
/// limiter takes a little loudness with the peaks, so the gain is corrected
/// by measuring the limited voice (monotone: each correction adds the
/// remaining error, up to [`VOICE_LIMIT_PASSES`] times).
fn normalise_voice(file: &Path, m: Loudness) -> VoiceNorm {
    let Some(first) = voice_limited_gain_db(m.integrated, m.true_peak) else {
        let g = voice_gain_db(m.integrated, m.true_peak);
        return VoiceNorm {
            gain_db: g,
            lufs: m.integrated + g,
            limit: None,
        };
    };
    let mut gain = first;
    // The limiter costs loudness: unmeasured, the unlimited estimate stands.
    let mut lufs = m.integrated + gain;
    for pass in 0..=VOICE_LIMIT_PASSES {
        let chain = format!("{BASE_FORMAT},volume={gain:.2}dB,{}", voice_limiter());
        let Some(l) = measure_chain(file, &chain).filter(|l| l.integrated.is_finite()) else {
            break;
        };
        lufs = l.integrated;
        let next = limited_gain(gain + (VOICE_LUFS - lufs), m.true_peak);
        if pass == VOICE_LIMIT_PASSES || (next - gain).abs() < 0.05 {
            break;
        }
        gain = next;
    }
    VoiceNorm {
        gain_db: gain,
        lufs,
        limit: Some(VoiceLimit {
            raw_lufs: m.integrated,
            raw_true_peak: m.true_peak,
            peak_reduction_db: (m.true_peak + gain - voice_limiter_ceiling_db()).max(0.0),
        }),
    }
}

/// Measure what a voice-relative mix needs and place the bed (spec: voice
/// normalised to `VOICE_LUFS`; the bed sits `levels` under the voice, from the
/// speech-led envelope). `None` for a legacy mix (no `plan.levels`, or no
/// voice-over).
fn resolve_relative(
    plan: &AudioPlan,
    voice: Option<&VoiceTrack>,
    relative: Option<&VoiceRelative>,
    music: Option<&(&MusicBed, PathBuf)>,
) -> Result<Option<Resolved>, RenderError> {
    let (Some(levels), Some(voice)) = (plan.levels, voice) else {
        return Ok(None);
    };
    // 1. The voice: integrated loudness and true peak through the same format
    //    chain the graph uses; the cache file is never modified.
    let measured = measure_chain(&voice.path, BASE_FORMAT)
        .filter(|m| m.integrated.is_finite() && m.integrated > -70.0);
    let norm = match measured {
        Some(m) => normalise_voice(&voice.path, m),
        None => VoiceNorm {
            gain_db: 0.0,
            lufs: VOICE_LUFS,
            limit: None,
        },
    };
    let voice_lufs = norm.lufs;
    let mut voice = voice.clone();
    voice.gain_db = norm.gain_db;

    // 2. The bed: loudness and range (MusicPlan values when given), the
    //    compressor for a wide range, and the gain automation.
    let default_relative = VoiceRelative::default();
    let relative = relative.unwrap_or(&default_relative);
    let mut bed_gain: Vec<(f64, f64)> = Vec::new();
    let mut gap_points: Vec<usize> = Vec::new();
    let mut compress = false;
    // The long narration gaps: between consecutive voiced segments, on the
    // voice timeline.
    let long_gap = round3(levels.long_gap_s);
    let voice_gaps: Vec<(f64, f64)> = relative
        .voiced
        .windows(2)
        .map(|w| (w[0].1, w[1].0))
        .filter(|&(a, b)| round3(b - a) >= long_gap)
        .collect();
    if let Some((bed, file)) = music {
        let raw = if relative.bed_lufs.is_none() || relative.bed_lra.is_none() {
            measure_chain(file, BASE_FORMAT)
        } else {
            None
        };
        let lra = relative.bed_lra.or(raw.map(|m| m.lra)).unwrap_or(0.0);
        compress = lra > BED_LRA_COMPRESS_LU;
        let lufs = if compress {
            // The compressor lowers the track's loudness: reference the
            // compressed track.
            measure_chain(file, &format!("{BASE_FORMAT},{BED_COMPRESSOR}"))
                .map(|m| m.integrated)
                .filter(|l| l.is_finite() && *l > -70.0)
        } else {
            relative
                .bed_lufs
                .or(raw.map(|m| m.integrated))
                .filter(|l| l.is_finite() && *l > -70.0)
        }
        // Unmeasurable: the planner's absolute normalisation implies it.
        .unwrap_or(crate::music::BED_LUFS - bed.gain_db);
        let flat;
        let envelope = match relative.envelope.as_ref() {
            Some(e) if !e.points.is_empty() => e,
            _ => {
                flat = DuckEnvelope {
                    points: vec![(0.0, levels.under_speech_db)],
                };
                &flat
            }
        };
        bed_gain = envelope
            .points
            .iter()
            .map(|&(t, level)| (round3(t + voice.offset), round3(voice_lufs + level - lufs)))
            .collect();
        // The points inside a long gap that hold the long-gap level (the rest
        // of the envelope, speech and edges, is never touched by the cap).
        gap_points = envelope
            .points
            .iter()
            .enumerate()
            .filter(|&(_, &(t, level))| {
                (level - levels.long_gap_db).abs() < 1e-9
                    && voice_gaps
                        .iter()
                        .any(|&(a, b)| t > a + 1e-6 && t < b - 1e-6)
            })
            .map(|(i, _)| i)
            .collect();
    }
    let long_gaps = voice_gaps
        .iter()
        .map(|&(a, b)| (a + voice.offset, b + voice.offset))
        .collect();
    Ok(Some(Resolved {
        voice,
        voice_lufs,
        voice_limit: norm.limit,
        bed_gain,
        compress,
        sfx_trim_db: 0.0,
        long_gaps,
        gap_points,
        gap_cap: None,
    }))
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Largest number of operands one `+` chain of the bed-gain expression may hold.
/// ffmpeg's expression parser spends one of its 100 stack levels per operand
/// of a `+` chain (measured: 90 terms parse, 95 fail), exactly as it does per
/// nested `if(`; so long sums are grouped in parentheses, at most this many
/// operands per group, which keeps the depth logarithmic in the point count.
const EXPR_GROUP: usize = 32;

/// `+`-join `terms`, grouped in parentheses [`EXPR_GROUP`] at a time (recursively),
/// so no chain or nesting level of the result is longer than that.
fn grouped_sum(mut terms: Vec<String>) -> String {
    while terms.len() > EXPR_GROUP {
        terms = terms
            .chunks(EXPR_GROUP)
            .map(|c| format!("({})", c.join("+")))
            .collect();
    }
    terms.join("+")
}

/// ffmpeg expression for the bed's linear gain at project time `t`: the
/// piecewise-linear dB automation `points` (held before the first and after
/// the last point), as `pow(10,(V0+sum of (vb-va)*clip((t-ta)/(tb-ta),0,1))/20)`
/// over the consecutive pairs that move (`tb > ta`, `vb != va`; flat pairs add
/// nothing). The expression is flat, not an `if(` chain: its depth does not
/// grow with the number of points, so a long narration (hundreds of points)
/// parses; measured in ffmpeg 8.1, 20 000 terms (a 690 KB filtergraph) still
/// ran. A pair with `tb == ta` is a step (`gte`). Levels and times are
/// rounded to 3 decimals first, so the sum of the printed steps is exactly
/// the last level. Deterministic text; `"1"` without points.
fn bed_gain_expr(points: &[(f64, f64)]) -> String {
    let Some(&(_, first_db)) = points.first() else {
        return "1".to_string();
    };
    let mut terms: Vec<String> = Vec::new();
    for w in points.windows(2) {
        let ((ta, va), (tb, vb)) = (w[0], w[1]);
        let (ta, tb) = (round3(ta), round3(tb));
        let dv = round3(round3(vb) - round3(va));
        if dv.abs() < 1e-9 {
            continue;
        }
        if tb > ta {
            terms.push(format!("({dv:.3})*clip((t-{ta:.3})/{:.3},0,1)", tb - ta));
        } else {
            terms.push(format!("({dv:.3})*gte(t,{ta:.3})"));
        }
    }
    if terms.is_empty() {
        return format!("pow(10,({:.3})/20)", round3(first_db));
    }
    format!(
        "pow(10,({:.3}+{})/20)",
        round3(first_db),
        grouped_sum(terms)
    )
}

/// Word gaps shorter than this stay inside one voiced segment ([`voiced_spans`]).
pub const VOICED_MERGE_GAP_S: f64 = 0.3;
/// Margin under the SFX ceiling the trim leaves (dB).
const SFX_TRIM_MARGIN_DB: f64 = 0.1;

/// The voiced segments of a voice-over on its own timeline: word spans merged
/// across gaps shorter than [`VOICED_MERGE_GAP_S`] (ms-rounded, ascending).
pub fn voiced_spans(speech: &motion_core::speech::SpeechMap) -> Vec<(f64, f64)> {
    let mut words: Vec<(f64, f64)> = speech
        .words
        .iter()
        .filter(|w| w.start.is_finite() && w.end.is_finite())
        .map(|w| (w.start.max(0.0), w.end.max(w.start.max(0.0))))
        .collect();
    words.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for (start, end) in words {
        match spans.last_mut() {
            Some(last) if round3(start - last.1) < VOICED_MERGE_GAP_S => last.1 = last.1.max(end),
            _ => spans.push((start, end)),
        }
    }
    spans
        .into_iter()
        .map(|(a, b)| (round3(a), round3(b)))
        .collect()
}

/// The (time, momentary LUFS) pairs of an ebur128 log (one per 100 ms; `M`
/// covers the 400 ms ending at the time).
fn parse_momentary(log: &str) -> Vec<(f64, f64)> {
    log.lines()
        .filter(|l| l.contains("TARGET:"))
        .filter_map(|l| {
            let field = |key: &str| -> Option<f64> {
                let rest = &l[l.find(key)? + key.len()..];
                match rest.split_whitespace().next()? {
                    "-inf" => Some(f64::NEG_INFINITY),
                    t => t.parse().ok(),
                }
            };
            Some((field(" t:")?, field(" M:")?))
        })
        .collect()
}

/// The largest momentary loudness whose 400 ms window is centred inside one of
/// `segments` (project seconds); full windows only.
fn momentary_max_inside(momentary: &[(f64, f64)], segments: &[(f64, f64)]) -> Option<f64> {
    momentary
        .iter()
        .filter(|&&(t, m)| t >= 0.4 - 1e-6 && m.is_finite())
        .filter(|&&(t, _)| {
            let centre = t - 0.2;
            segments
                .iter()
                .any(|&(a, b)| centre >= a - 1e-6 && centre <= b + 1e-6)
        })
        .map(|&(_, m)| m)
        .fold(None, |best, m| Some(best.map_or(m, |b: f64| b.max(m))))
}

/// The static trim (dB, <= 0) of the SFX bus so that its momentary loudness
/// inside the voiced segments stays `levels.sfx_under_speech_db` under the
/// voice: the mix graph is run once with the bus (post duck) metered, and the
/// worst 400 ms window inside the segments decides. The duck is linear in the
/// bus (its gain follows the voice key), so a trim ahead of it scales the
/// metered loudness exactly. Deterministic for the same inputs.
fn probe_sfx_trim(
    spec: &BuildSpec<'_>,
    levels: &MixLevels,
    voiced: &[(f64, f64)],
    voice_lufs: f64,
    offset: f64,
) -> Result<f64, RenderError> {
    let (mut args, _) = build_args_for(spec)?;
    // The meter logs at info level; no progress lines.
    if let Some(i) = args.iter().position(|a| a == "-v") {
        if let Some(level) = args.get_mut(i + 1) {
            *level = "info".to_string();
        }
    }
    args.insert(1, "-nostats".to_string());
    let out = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    if !out.status.success() {
        return Err(RenderError::Encode(format!(
            "sfx probe failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .join(" | ")
        )));
    }
    let shifted: Vec<(f64, f64)> = voiced
        .iter()
        .map(|&(a, b)| (a + offset, b + offset))
        .collect();
    let momentary = parse_momentary(&String::from_utf8_lossy(&out.stderr));
    let Some(worst) = momentary_max_inside(&momentary, &shifted) else {
        return Ok(0.0);
    };
    let ceiling = voice_lufs + levels.sfx_under_speech_db - SFX_TRIM_MARGIN_DB;
    let trim = (ceiling - worst).min(0.0);
    Ok((trim * 100.0).floor() / 100.0)
}

/// Margin under `BED_OVER_VOICE_MAX_DB` that the long-gap cap holds the bed's
/// worst momentary at (dB): the QA's own K-weighted windows and ffmpeg's meter
/// differ by hundredths of a dB, and the QA's reference is the voice stem's
/// gated loudness, not the mix-time estimate.
pub const GAP_CAP_MARGIN_DB: f64 = 0.5;
/// Metering passes of the cap after the first: each one verifies the lowering
/// the pass before made (a window straddling a ramp does not fall by the full
/// amount, so a second lowering may follow).
const GAP_CAP_PASSES: usize = 3;

/// (0.23 W5c) The bed's trim: the video's length of the track from the bed's
/// start offset (`atrim=0:D` for a bed that starts with the track, as before).
fn bed_trim(bed: &MusicBed, duration: f64) -> String {
    if bed.start > 0.0 {
        format!("atrim={:.3}:{:.3}", bed.start, bed.start + duration)
    } else {
        format!("atrim=0:{duration:.3}")
    }
}

/// (0.23 W5c) Where the bed's end fade starts on the project timeline: it ends
/// at `fade_out_at` (a downbeat), else with the video.
fn bed_fade_out_start(bed: &MusicBed, duration: f64) -> f64 {
    (bed.fade_out_at.unwrap_or(duration).min(duration) - bed.fade_out).max(0.0)
}

/// The bed branch of a voice-relative graph, from the raw track to the stem
/// tap: format, trim, optional compressor, the speech-led gain, fades.
fn rel_bed_chain(r: &Resolved, bed: &MusicBed, duration: f64) -> String {
    let fade_out_start = bed_fade_out_start(bed, duration);
    let compress = if r.compress {
        format!(",{BED_COMPRESSOR}")
    } else {
        String::new()
    };
    format!(
        "{BASE_FORMAT},{},asetpts=PTS-STARTPTS{compress},asetnsamples=n={BED_GAIN_FRAME_SAMPLES}:p=0,volume='{}':eval=frame,afade=t=in:st=0:d={:.3},afade=t=out:st={:.3}:d={:.3}",
        bed_trim(bed, duration),
        bed_gain_expr(&r.bed_gain),
        bed.fade_in,
        fade_out_start,
        bed.fade_out
    )
}

/// The worst momentary loudness (LUFS) of the bed as the mix places it
/// (`rel_bed_chain`) inside the windows centred in `r.long_gaps`; `None` when
/// no window is measurable.
fn probe_bed_gaps(
    r: &Resolved,
    bed: &MusicBed,
    file: &Path,
    duration: f64,
) -> Result<Option<f64>, RenderError> {
    let graph = format!(
        "[0:a]{},ebur128=peak=none[probe]",
        rel_bed_chain(r, bed, duration)
    );
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-v", "info", "-i"])
        .arg(file)
        .args([
            "-filter_complex",
            &graph,
            "-map",
            "[probe]",
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
            "bed gap probe failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .join(" | ")
        )));
    }
    let momentary = parse_momentary(&String::from_utf8_lossy(&out.stderr));
    Ok(momentary_max_inside(&momentary, &r.long_gaps))
}

/// The long-gap cap. The bed rises to `levels.long_gap_db` under the voice in
/// the narration gaps of at least `long_gap_s`, but a bed's 400 ms peaks sit up
/// to 5 dB above its integrated level (measured: calm_ambient with 4 s pauses
/// reaches voice - 4.7 dB at -10), past the QA's `bed_over_voice` limit. The
/// mix graph's bed branch is metered over the windows centred in those gaps
/// (like the SFX trim); when the worst momentary exceeds `voice +
/// BED_OVER_VOICE_MAX_DB - GAP_CAP_MARGIN_DB`, the points of the envelope that
/// hold the long-gap level are lowered by the excess (rounded up to 0.01 dB)
/// and the bed is metered again, up to `GAP_CAP_PASSES` times. Speech and edge
/// levels never move. Deterministic: the same inputs meter the same.
fn cap_long_gaps(
    r: &mut Resolved,
    bed: &MusicBed,
    file: &Path,
    duration: f64,
) -> Result<(), RenderError> {
    if r.long_gaps.is_empty() || r.gap_points.is_empty() {
        return Ok(());
    }
    let ceiling = BED_OVER_VOICE_MAX_DB - GAP_CAP_MARGIN_DB;
    let mut before: Option<f64> = None;
    let mut lowered = 0.0;
    for pass in 0..=GAP_CAP_PASSES {
        let Some(worst) = probe_bed_gaps(r, bed, file, duration)? else {
            return Ok(());
        };
        let over = worst - r.voice_lufs;
        let before_db = *before.get_or_insert(over);
        let excess = over - ceiling;
        if excess <= 0.0 || pass == GAP_CAP_PASSES {
            r.gap_cap = Some(GapCap {
                before_db,
                after_db: over,
                ceiling_db: ceiling,
                lowered_db: lowered,
            });
            return Ok(());
        }
        let cut = (excess * 100.0).ceil() / 100.0;
        for &i in &r.gap_points {
            if let Some(point) = r.bed_gain.get_mut(i) {
                point.1 = round3(point.1 - cut);
            }
        }
        lowered -= cut;
    }
    Ok(())
}

/// Linear `sidechaincompress` threshold that yields `duck_db` of gain
/// reduction at ratio `ratio` for the leveled voice key:
/// `GR = (1 - 1/ratio) * (KEY_EFFECTIVE_DB - threshold)`.
fn duck_threshold(duck_db: f64, ratio: f64) -> f64 {
    let t_db = KEY_EFFECTIVE_DB - duck_db / (1.0 - 1.0 / ratio);
    10f64.powf(t_db / 20.0).clamp(0.000_976_563, 1.0)
}

/// The voice-over branch of the graph (0.10). The VO (unducked, on top) keys
/// `sidechaincompress` on the music bed (-10 dB) and on the SFX bus (-4 dB).
/// Returns the label of the summed mix. `next_input` is the first free input
/// index; the music bed (if any) already took it.
///
/// (0.23) With `rel` (a voice-relative mix) the voice carries its normalising
/// gain, the bed has no sidechain at all (its gain follows the speech-led
/// envelope instead) and only the SFX bus is keyed on the voice. With `stems`
/// each branch is also tapped into `outs` as (label, wav path).
#[allow(clippy::too_many_arguments)]
fn voiced_mix(
    args: &mut Vec<String>,
    graph: &mut Vec<String>,
    voice: &VoiceTrack,
    music: Option<&(&MusicBed, PathBuf)>,
    has_sfx: bool,
    next_input: usize,
    duration: f64,
    rel: Option<&Resolved>,
    stems: Option<&Path>,
    probe: bool,
    outs: &mut Vec<(String, PathBuf)>,
) -> Result<String, RenderError> {
    let mut mix_inputs: Vec<String> = Vec::new();
    // The music bed.
    let mut bed_duck_by_sfx = false;
    let mut input = next_input;
    if let Some((bed, file)) = music {
        if !file.is_file() {
            return Err(RenderError::Asset(format!(
                "music file {} not found",
                file.display()
            )));
        }
        args.push("-i".into());
        args.push(file.to_string_lossy().into_owned());
        let fade_out_start = bed_fade_out_start(bed, duration);
        match rel {
            None => {
                graph.push(format!(
                    "[{input}:a]{BASE_FORMAT},{},asetpts=PTS-STARTPTS,volume={:.1}dB,afade=t=in:st=0:d={:.3},afade=t=out:st={:.3}:d={:.3}[bed]",
                    bed_trim(bed, duration),
                    bed.gain_db,
                    bed.fade_in,
                    fade_out_start,
                    bed.fade_out
                ));
                bed_duck_by_sfx = has_sfx && bed.duck;
            }
            Some(r) => {
                // The bed's gain is the speech-led automation (no sidechain).
                graph.push(format!(
                    "[{input}:a]{}[bed]",
                    rel_bed_chain(r, bed, duration)
                ));
            }
        }
        input += 1;
    }
    // The voice-over input: the mix copy and the key (leveled, then one copy
    // per ducked bus).
    args.push("-i".into());
    args.push(voice.path.to_string_lossy().into_owned());
    let mut keys: Vec<&str> = Vec::new();
    if music.is_some() && rel.is_none() {
        keys.push("vkbed");
    }
    if has_sfx {
        keys.push("vksfx");
    }
    let delay = if voice.offset > 0.0 {
        format!(",adelay={:.3}:all=1", voice.offset * 1000.0)
    } else {
        String::new()
    };
    // (0.23) The voice's normalising gain (absent = the file as recorded).
    let gain = if voice.gain_db != 0.0 {
        format!(",volume={:.2}dB", voice.gain_db)
    } else {
        String::new()
    };
    // (A5c) A peaky voice goes through the voice limiter right after the gain,
    // before the delay, the key and the mix (so the voice stem has it too).
    let limit = match rel {
        Some(r) if r.voice_limit.is_some() => voice_limiter_stage(),
        _ => String::new(),
    };
    // The key is padded to the project duration: sidechaincompress ends with it.
    if keys.is_empty() {
        graph.push(format!(
            "[{input}:a]{BASE_FORMAT}{gain}{limit}{delay},asetpts=PTS-STARTPTS,apad=whole_dur={duration:.3},atrim=0:{duration:.3}[vomix]"
        ));
    } else {
        graph.push(format!(
            "[{input}:a]{BASE_FORMAT}{gain}{limit}{delay},asetpts=PTS-STARTPTS,apad=whole_dur={duration:.3},atrim=0:{duration:.3},asplit=2[vomix][vkey]"
        ));
        let leveled = if keys.len() == 1 {
            format!("[{}]", keys[0])
        } else {
            format!(",asplit={}[{}][{}]", keys.len(), keys[0], keys[1])
        };
        graph.push(format!("[vkey]{KEY_CHAIN}{leveled}"));
    }
    mix_inputs.push("vomix".into());

    // Stem names of the mixed branches, by label.
    let mut stem_of: Vec<(&str, &str)> = vec![("vomix", "voice")];
    if has_sfx {
        let t = duck_threshold(SFX_DUCK_DB, SFX_DUCK_RATIO);
        let trim = match rel {
            Some(r) if r.sfx_trim_db != 0.0 => format!("volume={:.2}dB,", r.sfx_trim_db),
            _ => String::new(),
        };
        if bed_duck_by_sfx {
            graph.push(format!(
                "[sfx]apad=whole_dur={duration:.3},asplit=2[sfxa][sfxkey]"
            ));
        } else {
            graph.push(format!("[sfx]{trim}apad=whole_dur={duration:.3}[sfxa]"));
        }
        graph.push(format!(
            "[sfxa][vksfx]sidechaincompress=threshold={t:.6}:ratio={SFX_DUCK_RATIO}:attack=15:release=250:knee=1[sfxd]"
        ));
        mix_inputs.push("sfxd".into());
        stem_of.push(("sfxd", "sfx"));
    }
    if music.is_some() {
        if rel.is_some() {
            // No sidechain: the envelope already set the bed's level.
            mix_inputs.push("bed".into());
            stem_of.push(("bed", "bed"));
        } else {
            let t = duck_threshold(BED_DUCK_DB, BED_DUCK_RATIO);
            graph.push(format!(
                "[bed][vkbed]sidechaincompress=threshold={t:.6}:ratio={BED_DUCK_RATIO}:attack=15:release=250:knee=1[bedv]"
            ));
            if bed_duck_by_sfx {
                graph.push(
                    "[bedv][sfxkey]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=300[bedd]"
                        .into(),
                );
                mix_inputs.push("bedd".into());
                stem_of.push(("bedd", "bed"));
            } else {
                mix_inputs.push("bedv".into());
                stem_of.push(("bedv", "bed"));
            }
        }
    }
    // (0.23) Stems: tap each mixed branch (post-gain, post-envelope) into a wav.
    if let Some(dir) = stems {
        for label in mix_inputs.iter_mut() {
            let Some(&(_, name)) = stem_of.iter().find(|(l, _)| *l == label.as_str()) else {
                continue;
            };
            graph.push(format!("[{label}]asplit=2[{label}m][{label}s]"));
            outs.push((format!("{label}s"), dir.join(format!("{name}.wav"))));
            *label = format!("{label}m");
        }
    }
    // (0.23) The SFX probe: the bus is also metered (ebur128), see
    // [`probe_sfx_trim`]; the meter's output is mapped to a null muxer.
    if probe && has_sfx {
        if let Some(label) = mix_inputs.iter_mut().find(|l| l.starts_with("sfxd")) {
            graph.push(format!("[{label}]asplit=2[{label}p][sfxprobe_in]"));
            graph.push("[sfxprobe_in]ebur128=peak=none[sfxprobe]".to_string());
            *label = format!("{label}p");
        }
    }
    if mix_inputs.len() == 1 {
        return Ok(mix_inputs.remove(0));
    }
    graph.push(format!(
        "{}amix=inputs={}:normalize=0:duration=longest[mix]",
        mix_inputs
            .iter()
            .map(|l| format!("[{l}]"))
            .collect::<String>(),
        mix_inputs.len()
    ));
    Ok("mix".into())
}

/// (0.10) Measured voice-over duck depth on the music bed, in dB (positive =
/// gain reduction). Renders the bed alone twice with the mix's own graph, once
/// raw and once ducked by `voice`, and returns the median over `spans`
/// (project seconds while the voice speaks) of `rms(raw) - rms(ducked)`.
/// `None` when no span holds measurable bed audio.
pub fn measure_bed_duck(
    voice: &VoiceTrack,
    bed: &motion_core::audio::MusicBed,
    music_file: &Path,
    duration: f64,
    spans: &[(f64, f64)],
) -> Result<Option<f64>, RenderError> {
    let t = duck_threshold(BED_DUCK_DB, BED_DUCK_RATIO);
    let fade_out_start = bed_fade_out_start(bed, duration);
    let bed_chain = format!(
        "{BASE_FORMAT},{},asetpts=PTS-STARTPTS,volume={:.1}dB,afade=t=in:st=0:d={:.3},afade=t=out:st={:.3}:d={:.3}",
        bed_trim(bed, duration),
        bed.gain_db,
        bed.fade_in,
        fade_out_start,
        bed.fade_out
    );
    let delay = if voice.offset > 0.0 {
        format!(",adelay={:.3}:all=1", voice.offset * 1000.0)
    } else {
        String::new()
    };
    let render = |graph: String, inputs: &[&Path]| -> Result<Vec<f32>, RenderError> {
        let mut args: Vec<String> = vec!["-v".into(), "error".into()];
        for i in inputs {
            args.push("-i".into());
            args.push(i.to_string_lossy().into_owned());
        }
        args.extend([
            "-filter_complex".into(),
            graph,
            "-map".into(),
            "[o]".into(),
            "-ac".into(),
            "1".into(),
            "-ar".into(),
            SAMPLE_RATE.to_string(),
            "-f".into(),
            "f32le".into(),
            "-".into(),
        ]);
        let out = Command::new("ffmpeg")
            .args(&args)
            .stdin(Stdio::null())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| {
                RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
            })?;
        if !out.status.success() {
            return Err(RenderError::Encode(format!(
                "bed duck measurement failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(out
            .stdout
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect())
    };
    let raw = render(format!("[0:a]{bed_chain}[o]"), &[music_file])?;
    let ducked = render(
        format!(
            "[0:a]{bed_chain}[bed];[1:a]{BASE_FORMAT}{delay},asetpts=PTS-STARTPTS,apad=whole_dur={duration:.3},atrim=0:{duration:.3},{KEY_CHAIN}[key];[bed][key]sidechaincompress=threshold={t:.6}:ratio={BED_DUCK_RATIO}:attack=15:release=250:knee=1[o]"
        ),
        &[music_file, &voice.path],
    )?;
    let rms = |x: &[f32], a: f64, b: f64| -> Option<f64> {
        let (i, j) = (
            (a * f64::from(SAMPLE_RATE)) as usize,
            (b * f64::from(SAMPLE_RATE)) as usize,
        );
        let seg = x.get(i..j.min(x.len()))?;
        if seg.len() < 480 {
            return None;
        }
        let ms = seg
            .iter()
            .map(|&v| f64::from(v) * f64::from(v))
            .sum::<f64>()
            / seg.len() as f64;
        (ms > 1e-12).then(|| 10.0 * ms.log10())
    };
    let mut depths: Vec<f64> = spans
        .iter()
        .filter_map(|&(a, b)| Some(rms(&raw, a, b)? - rms(&ducked, a, b)?))
        .collect();
    if depths.is_empty() {
        return Ok(None);
    }
    depths.sort_by(|a, b| a.total_cmp(b));
    Ok(Some(depths[depths.len() / 2]))
}

fn run_ffmpeg(args: &[String]) -> Result<(), RenderError> {
    let out = Command::new("ffmpeg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(8).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(RenderError::Encode(format!(
            "audio mix failed ({}): {}",
            out.status,
            tail.join(" | ")
        )));
    }
    Ok(())
}

/// Where the finished mix goes.
#[derive(Clone, Copy)]
enum Sink<'a> {
    /// The video stream copied, the mix as AAC (the product).
    Mp4(&'a Path),
    /// Discarded (stems-only runs and graph inspection).
    Null,
}

/// Everything one mix graph is built from.
struct BuildSpec<'a> {
    video: &'a Path,
    plan: &'a AudioPlan,
    sfx_root: &'a Path,
    library: &'a SfxLibrary,
    music: Option<&'a (&'a MusicBed, PathBuf)>,
    duration: f64,
    sink: Sink<'a>,
    makeup_db: Option<f64>,
    voice: Option<&'a VoiceTrack>,
    /// (0.23) The resolved voice-relative automation (`plan.levels = Some`).
    rel: Option<&'a Resolved>,
    /// (0.23) Also write the stems into this directory.
    stems: Option<&'a Path>,
    /// (0.23) Meter the SFX bus (`probe_sfx_trim`): tap it into ebur128.
    probe: bool,
}

/// The pre-0.23 graph builder (no voice-relative automation, no stems).
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn build_args(
    video: &Path,
    plan: &AudioPlan,
    sfx_root: &Path,
    library: &SfxLibrary,
    music_root: Option<&Path>,
    duration: f64,
    output: &Path,
    makeup_db: Option<f64>,
    voice: Option<&VoiceTrack>,
) -> Result<Vec<String>, RenderError> {
    let music = music_file(plan, music_root);
    build_args_for(&BuildSpec {
        video,
        plan,
        sfx_root,
        library,
        music: music.as_ref(),
        duration,
        sink: Sink::Mp4(output),
        makeup_db,
        voice,
        rel: None,
        stems: None,
        probe: false,
    })
    .map(|(args, _)| args)
}

/// The ffmpeg arguments of one mix, and the stem files they write.
fn build_args_for(spec: &BuildSpec<'_>) -> Result<(Vec<String>, StemFiles), RenderError> {
    let BuildSpec {
        video,
        plan,
        sfx_root,
        library,
        music,
        duration,
        sink,
        makeup_db,
        voice,
        rel,
        stems,
        probe,
    } = *spec;
    let mut stem_outs: Vec<(String, PathBuf)> = Vec::new();
    let mut args: Vec<String> = vec![
        "-y".into(),
        "-v".into(),
        "error".into(),
        "-i".into(),
        video.to_string_lossy().into_owned(),
    ];
    let mut graph: Vec<String> = Vec::new();
    let mut labels: Vec<String> = Vec::new();

    for (k, cue) in plan.cues.iter().enumerate() {
        let sound = library.get(&cue.sound_id).ok_or_else(|| {
            RenderError::Asset(format!(
                "cue sound '{}' is not in the library",
                cue.sound_id
            ))
        })?;
        let file = sfx_root.join(&sound.path);
        if !file.is_file() {
            return Err(RenderError::Asset(format!(
                "sound file {} not found",
                file.display()
            )));
        }
        args.push("-i".into());
        args.push(file.to_string_lossy().into_owned());
        let input = k + 1;
        let mut chain = format!("[{input}:a]{BASE_FORMAT}");
        let lead = sound.peak - cue.time;
        if lead > 0.0 {
            chain.push_str(&format!(",atrim=start={lead:.3},asetpts=PTS-STARTPTS"));
        }
        chain.push_str(&format!(",volume={:.1}dB", cue.gain_db));
        let delay_ms = (cue.time - sound.peak) * 1000.0;
        if delay_ms > 0.0 {
            chain.push_str(&format!(",adelay={delay_ms:.3}:all=1"));
        }
        let label = format!("c{k}");
        chain.push_str(&format!("[{label}]"));
        graph.push(chain);
        labels.push(label);
    }

    let n = plan.cues.len();

    let next_input = n + 1;
    // The SFX bus (None when there are no cues).
    let sfx_bus = if n > 0 {
        let joined: String = labels.iter().map(|l| format!("[{l}]")).collect();
        graph.push(format!(
            "{joined}amix=inputs={n}:normalize=0:duration=longest[sfx]"
        ));
        Some("sfx")
    } else {
        None
    };

    let mix_label: String = if let Some(vt) = voice {
        voiced_mix(
            &mut args,
            &mut graph,
            vt,
            music,
            sfx_bus.is_some(),
            next_input,
            duration,
            rel,
            stems,
            probe,
            &mut stem_outs,
        )?
    } else {
        match (music, sfx_bus) {
            (Some((bed, file)), bus) => {
                if !file.is_file() {
                    return Err(RenderError::Asset(format!(
                        "music file {} not found",
                        file.display()
                    )));
                }
                args.push("-i".into());
                args.push(file.to_string_lossy().into_owned());
                let m = next_input;
                let fade_out_start = bed_fade_out_start(bed, duration);
                graph.push(format!(
                "[{m}:a]{BASE_FORMAT},{},asetpts=PTS-STARTPTS,volume={:.1}dB,afade=t=in:st=0:d={:.3},afade=t=out:st={:.3}:d={:.3}[bed]",
                bed_trim(bed, duration),
                bed.gain_db, bed.fade_in, fade_out_start, bed.fade_out
            ));
                match bus {
                    Some(_) if bed.duck => {
                        // sidechaincompress ends when its key ends, which would cut
                        // the bed after the last cue: extend the bus (and so the key)
                        // with silence to the project duration, like the bed.
                        graph.push(format!(
                            "[sfx]apad=whole_dur={duration:.3},asplit=2[sfxmix][sfxkey]"
                        ));
                        graph.push(
                        "[bed][sfxkey]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=300[ducked]"
                            .into(),
                    );
                        graph.push(
                            "[sfxmix][ducked]amix=inputs=2:normalize=0:duration=longest[mix]"
                                .into(),
                        );
                        "mix".into()
                    }
                    Some(_) => {
                        graph.push(
                            "[sfx][bed]amix=inputs=2:normalize=0:duration=longest[mix]".into(),
                        );
                        "mix".into()
                    }
                    None => "bed".into(),
                }
            }
            (None, Some(_)) => "sfx".into(),
            (None, None) => {
                // Silent track of the right length.
                args.push("-f".into());
                args.push("lavfi".into());
                args.push("-i".into());
                args.push(format!("anullsrc=r={SAMPLE_RATE}:cl=stereo"));
                let s = next_input;
                graph.push(format!("[{s}:a]{BASE_FORMAT}[mix]"));
                "mix".into()
            }
        }
    };
    // alimiter delays its output by its look-ahead (attack, 5 ms): trim it so
    // cue peaks stay on the timeline.
    // Pad BEFORE the limiter (padding after it flakes under ffmpeg 8.1: the
    // track sometimes ends early or the process spins); 5 ms for the trim.
    let padded = duration + 0.005;
    // (0.9) Opt-in makeup gain on the summed mix, ahead of the limiter.
    let gain = makeup_db
        .map(|g| format!("volume={g:.2}dB,"))
        .unwrap_or_default();
    // (0.22) The limiter holds sample peaks at -1.5 dBFS (0.841): inter-sample
    // peaks and the AAC encode add up to ~0.5 dB, and at the old -1.0 (0.891)
    // a hot voice-over measured -0.7 dBTP, past QA's -1 dBTP + 0.3 margin.
    graph.push(format!(
        "[{mix_label}]{gain}apad=whole_dur={padded:.3},alimiter=limit=0.841:attack=5:level=disabled,atrim=start=0.005,asetpts=PTS-STARTPTS,atrim=0:{duration:.3}[aout]"
    ));

    args.extend(["-filter_complex".into(), graph.join(";")]);
    match sink {
        Sink::Mp4(output) => args.extend([
            "-map".into(),
            "0:v".into(),
            "-map".into(),
            "[aout]".into(),
            "-c:v".into(),
            "copy".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "192k".into(),
            "-ar".into(),
            SAMPLE_RATE.to_string(),
            "-ac".into(),
            "2".into(),
            "-t".into(),
            format!("{duration:.3}"),
            "-movflags".into(),
            "+faststart".into(),
            "-f".into(),
            "mp4".into(),
            output.to_string_lossy().into_owned(),
        ]),
        Sink::Null => args.extend([
            "-map".into(),
            "[aout]".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ]),
    }
    if probe {
        args.extend([
            "-map".into(),
            "[sfxprobe]".into(),
            "-f".into(),
            "null".into(),
            "-".into(),
        ]);
    }
    // (0.23) Stems: one float wav per tapped branch, after the main output.
    let mut files = StemFiles::default();
    for (label, path) in &stem_outs {
        args.extend([
            "-map".into(),
            format!("[{label}]"),
            "-c:a".into(),
            "pcm_f32le".into(),
            "-ar".into(),
            SAMPLE_RATE.to_string(),
            "-ac".into(),
            "2".into(),
            "-t".into(),
            format!("{duration:.3}"),
            "-f".into(),
            "wav".into(),
            path.to_string_lossy().into_owned(),
        ]);
        match path.file_stem().and_then(|s| s.to_str()) {
            Some("voice") => files.voice = Some(path.clone()),
            Some("bed") => files.bed = Some(path.clone()),
            Some("sfx") => files.sfx = Some(path.clone()),
            _ => {}
        }
    }
    Ok((args, files))
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::audio::{
        AudioCue, CueKind, MusicBed, SfxFamily, SfxSound, AUDIO_PLAN_VERSION, SFX_LIBRARY_VERSION,
    };

    fn plan(cues: Vec<AudioCue>, music: Option<MusicBed>) -> AudioPlan {
        AudioPlan {
            version: AUDIO_PLAN_VERSION.to_string(),
            cues,
            loudness_target: -16.0,
            true_peak_limit: -1.0,
            min_spacing: 0.15,
            information_cap: 3,
            music,
            speech_adjustments: vec![],
            levels: None,
        }
    }

    fn cue(time: f64) -> AudioCue {
        AudioCue {
            time,
            scene: "s".into(),
            kind: CueKind::Information,
            family: SfxFamily::Click,
            sound_id: "a".into(),
            gain_db: 6.0,
            priority: 1,
            reason: "t".into(),
        }
    }

    fn library() -> SfxLibrary {
        SfxLibrary {
            version: SFX_LIBRARY_VERSION.to_string(),
            sounds: vec![SfxSound {
                id: "a".into(),
                family: SfxFamily::Click,
                path: "a.wav".into(),
                duration: 1.0,
                onset: 0.1,
                peak: 0.2,
                audible_end: 0.4,
                peak_db: -18.0,
                lufs: None,
                sha256: "0".repeat(64),
                tags: vec![],
            }],
        }
    }

    fn filter_graph(args: &[String]) -> String {
        let i = args
            .iter()
            .position(|a| a == "-filter_complex")
            .expect("filter_complex");
        args[i + 1].clone()
    }

    /// Scratch dir with empty stand-ins for the sound, bed and voice files
    /// (build_args only checks they exist).
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("motion-mix-unit-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&d).expect("dir");
        for f in ["a.wav", "bed.wav", "vo.wav"] {
            std::fs::write(d.join(f), b"").expect("file");
        }
        d
    }

    const TAIL: &str = "apad=whole_dur=3.005,alimiter=limit=0.841:attack=5:level=disabled,atrim=start=0.005,asetpts=PTS-STARTPTS,atrim=0:3.000[aout]";

    #[test]
    fn graph_without_a_voice_is_the_pre_voice_graph() {
        let dir = scratch("novoice");
        let out = dir.join("o.mp4");
        let video = dir.join("v.mp4");
        let lib = library();
        // No cues, no music: a silent track.
        let g = filter_graph(
            &build_args(
                &video,
                &plan(vec![], None),
                &dir,
                &lib,
                None,
                3.0,
                &out,
                None,
                None,
            )
            .expect("args"),
        );
        assert_eq!(g, format!("[1:a]{BASE_FORMAT}[mix];[mix]{TAIL}"));
        // One cue.
        let g = filter_graph(
            &build_args(
                &video,
                &plan(vec![cue(1.0)], None),
                &dir,
                &lib,
                None,
                3.0,
                &out,
                None,
                None,
            )
            .expect("args"),
        );
        assert_eq!(
            g,
            format!(
                "[1:a]{BASE_FORMAT},volume=6.0dB,adelay=800.000:all=1[c0];[c0]amix=inputs=1:normalize=0:duration=longest[sfx];[sfx]{TAIL}"
            )
        );
        // One cue + ducked bed + makeup.
        let bed = MusicBed {
            track: "bed.wav".into(),
            gain_db: -6.0,
            fade_in: 0.5,
            fade_out: 1.0,
            duck: true,
            start: 0.0,
            fade_out_at: None,
        };
        let g = filter_graph(
            &build_args(
                &video,
                &plan(vec![cue(1.0)], Some(bed)),
                &dir,
                &lib,
                Some(&dir),
                3.0,
                &out,
                Some(2.5),
                None,
            )
            .expect("args"),
        );
        assert_eq!(
            g,
            format!(
                "[1:a]{BASE_FORMAT},volume=6.0dB,adelay=800.000:all=1[c0];[c0]amix=inputs=1:normalize=0:duration=longest[sfx];[2:a]{BASE_FORMAT},atrim=0:3.000,asetpts=PTS-STARTPTS,volume=-6.0dB,afade=t=in:st=0:d=0.500,afade=t=out:st=2.000:d=1.000[bed];[sfx]apad=whole_dur=3.000,asplit=2[sfxmix][sfxkey];[bed][sfxkey]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=300[ducked];[sfxmix][ducked]amix=inputs=2:normalize=0:duration=longest[mix];[mix]volume=2.50dB,{TAIL}"
            )
        );
    }

    #[test]
    fn voiced_graph_keys_both_buses_on_the_voice() {
        let dir = scratch("voiced");
        let out = dir.join("o.mp4");
        let video = dir.join("v.mp4");
        let lib = library();
        let bed = MusicBed {
            track: "bed.wav".into(),
            gain_db: -6.0,
            fade_in: 0.5,
            fade_out: 1.0,
            duck: true,
            start: 0.0,
            fade_out_at: None,
        };
        let vt = VoiceTrack {
            path: dir.join("vo.wav"),
            offset: 0.0,
            gain_db: 0.0,
        };
        let g = filter_graph(
            &build_args(
                &video,
                &plan(vec![cue(1.0)], Some(bed)),
                &dir,
                &lib,
                Some(&dir),
                3.0,
                &out,
                None,
                Some(&vt),
            )
            .expect("args"),
        );
        assert!(g.contains("[3:a]"), "{g}");
        assert!(g.contains("asplit=2[vomix][vkey]"), "{g}");
        assert!(
            g.contains(&format!("[vkey]{KEY_CHAIN},asplit=2[vkbed][vksfx]")),
            "{g}"
        );
        assert!(g.contains("[sfxa][vksfx]sidechaincompress="), "{g}");
        assert!(g.contains("[bed][vkbed]sidechaincompress="), "{g}");
        assert!(g.contains("[vomix][sfxd][bedd]amix=inputs=3"), "{g}");
        assert!(g.ends_with(TAIL), "{g}");
        // VO only: no silent source, no sidechain.
        let g = filter_graph(
            &build_args(
                &video,
                &plan(vec![], None),
                &dir,
                &lib,
                None,
                3.0,
                &out,
                None,
                Some(&vt),
            )
            .expect("args"),
        );
        assert!(!g.contains("sidechaincompress"), "{g}");
        assert!(g.contains("[vomix];") || g.contains("[vomix]"), "{g}");
        assert!(g.contains(&format!("[vomix]{TAIL}")), "{g}");
    }

    #[test]
    fn duck_thresholds_follow_the_leveled_key() {
        // threshold = key - duck/(1-1/ratio).
        let t = duck_threshold(10.0, 20.0);
        assert!((20.0 * t.log10() - (KEY_EFFECTIVE_DB - 10.0 / 0.95)).abs() < 1e-6);
        let t = duck_threshold(4.0, 4.0);
        assert!((20.0 * t.log10() - (KEY_EFFECTIVE_DB - 4.0 / 0.75)).abs() < 1e-6);
        // Clamped to ffmpeg's range.
        assert_eq!(duck_threshold(80.0, 20.0), 0.000_976_563);
    }

    #[test]
    fn voice_mixes_may_cut_loudness() {
        assert_eq!(makeup_gain_db_with(m(-10.0, -3.0), -16.0, false), None);
        let g = makeup_gain_db_with(m(-10.0, -3.0), -16.0, true).expect("cut");
        assert!((g + 6.0).abs() < 1e-9);
        // Voice-over: up to LIMITER_ALLOWANCE_DB of peaks go into the limiter.
        let g = makeup_gain_db_with(m(-17.9, -1.1), -16.0, true).expect("boost");
        assert!((g - 1.9).abs() < 1e-9);
        let capped = makeup_gain_db_with(m(-22.0, -1.0), -16.0, true).expect("capped");
        assert!((capped - (-0.1 + LIMITER_ALLOWANCE_DB)).abs() < 1e-9);
    }

    #[test]
    fn the_voice_limiter_stage_is_padded_and_trimmed_by_its_look_ahead() {
        assert_eq!(
            voice_limiter_stage(),
            ",apad=pad_dur=0.005,alimiter=limit=0.6839:attack=5:release=100:level=disabled,atrim=start=0.005,asetpts=PTS-STARTPTS"
        );
        // The ceiling is VOICE_TRUE_PEAK_DB less the margin.
        assert!((voice_limiter_ceiling_db() - (VOICE_TRUE_PEAK_DB - 0.3)).abs() < 1e-9);
    }

    #[test]
    fn the_limited_gain_is_capped_by_the_peak_reduction() {
        // 5 dB wanted, 7.7 dB of room under the cap: all of it.
        assert_eq!(limited_gain(5.0, -5.0), 5.0);
        // The cap is ceiling + VOICE_LIMIT_MAX_DB - true peak = -3.3 + 6 + 5
        // (to the 0.01 dB the gain is floored to).
        assert!((limited_gain(20.0, -5.0) - 7.7).abs() <= 0.011);
        // Never past the gain range.
        assert_eq!(limited_gain(100.0, -80.0), VOICE_GAIN_LIMIT_DB);
    }

    fn m(integrated: f64, true_peak: f64) -> Measured {
        Measured {
            integrated,
            true_peak,
        }
    }

    #[test]
    fn gain_targets_loudness_within_headroom() {
        // Toward the target.
        assert_eq!(makeup_gain_db(m(-26.0, -20.0), -16.0), Some(10.0));
        // Peak-limited: -1.0 - (-6.0) - 0.1 = 4.9.
        let g = makeup_gain_db(m(-30.0, -6.0), -16.0).expect("gain");
        assert!((g - 4.9).abs() < 1e-9);
        // Clamped to 24 dB.
        assert_eq!(makeup_gain_db(m(-60.0, -80.0), -16.0), Some(24.0));
        // Never attenuates; negligible gains are skipped.
        assert_eq!(makeup_gain_db(m(-10.0, -3.0), -16.0), None);
        assert_eq!(makeup_gain_db(m(-16.03, -10.0), -16.0), None);
        // Silent / unmeasurable.
        assert_eq!(
            makeup_gain_db(m(f64::NEG_INFINITY, f64::NEG_INFINITY), -16.0),
            None
        );
        assert_eq!(makeup_gain_db(m(-70.0, -90.0), -16.0), None);
    }

    #[test]
    fn parses_the_ebur128_summary() {
        let log = "[Parsed_ebur128_0 @ 0x1] t: 1.0   TARGET:-23 LUFS    M: -21.1 S:-120.7     I: -21.1 LUFS\n\
[Parsed_ebur128_0 @ 0x1] Summary:\n\n  Integrated loudness:\n    I:         -26.4 LUFS\n    Threshold: -36.4 LUFS\n\n  Loudness range:\n    LRA:         0.0 LU\n\n  True peak:\n    Peak:       -3.2 dBFS\n";
        assert_eq!(parse_ebur128_summary(log), Some(m(-26.4, -3.2)));
        assert_eq!(parse_ebur128_summary("no summary"), None);
    }
}

/// (0.23) The pure helpers of the voice-relative mix.
#[cfg(test)]
mod relative_tests {
    use super::*;
    use motion_core::speech::{SpeechMap, SpeechWord, SPEECH_VERSION};

    #[test]
    fn the_bed_gain_expression_is_the_piecewise_linear_db_curve() {
        // Held, ramped up 10 dB over 0.5 s, held: the flat pair adds nothing.
        let e = bed_gain_expr(&[(0.0, -20.0), (1.0, -20.0), (1.5, -10.0)]);
        assert_eq!(e, "pow(10,(-20.000+(10.000)*clip((t-1.000)/0.500,0,1))/20)");
        // Two ramps: down 5 dB over 0.15 s, then up 2.5 dB over 0.5 s.
        let e = bed_gain_expr(&[
            (0.5, -4.0),
            (0.85, -4.0),
            (1.0, -9.0),
            (3.0, -9.0),
            (3.5, -6.5),
        ]);
        assert_eq!(
            e,
            "pow(10,(-4.000+(-5.000)*clip((t-0.850)/0.150,0,1)+(2.500)*clip((t-3.000)/0.500,0,1))/20)"
        );
        // Before the first point (a voice that starts late) the first level
        // holds: every clip is 0 until its pair starts.
        let e = bed_gain_expr(&[(2.0, -30.0), (3.0, -20.0)]);
        assert_eq!(e, "pow(10,(-30.000+(10.000)*clip((t-2.000)/1.000,0,1))/20)");
        // A step (two points at one time) is a gte term.
        let e = bed_gain_expr(&[(1.0, -30.0), (1.0, -20.0)]);
        assert_eq!(e, "pow(10,(-30.000+(10.000)*gte(t,1.000))/20)");
        // One point (or only flat pairs) is a constant; no points is unity.
        assert_eq!(bed_gain_expr(&[(0.0, -12.0)]), "pow(10,(-12.000)/20)");
        assert_eq!(
            bed_gain_expr(&[(0.0, -12.0), (4.0, -12.0)]),
            "pow(10,(-12.000)/20)"
        );
        assert_eq!(bed_gain_expr(&[]), "1");
        // Deterministic text.
        let pts = [(0.0, -18.5), (0.85, -18.5), (1.0, -31.2)];
        assert_eq!(bed_gain_expr(&pts), bed_gain_expr(&pts));
        // No if( chain: the depth does not grow with the points.
        assert!(!bed_gain_expr(&pts).contains("if("));
    }

    #[test]
    fn long_sums_are_grouped_so_no_chain_or_nesting_gets_long() {
        // The deepest `+` chain and parenthesis nesting of the result.
        fn shape(e: &str) -> (usize, usize) {
            let mut chains = vec![0usize];
            let (mut max_depth, mut max_chain) = (0usize, 0usize);
            for c in e.chars() {
                match c {
                    '(' => {
                        chains.push(0);
                        max_depth = max_depth.max(chains.len() - 1);
                    }
                    ')' => {
                        chains.pop();
                    }
                    '+' => {
                        if let Some(last) = chains.last_mut() {
                            *last += 1;
                            max_chain = max_chain.max(*last);
                        }
                    }
                    _ => {}
                }
            }
            (max_depth, max_chain)
        }
        let pts: Vec<(f64, f64)> = (0..5000)
            .map(|i| (f64::from(i) * 0.5, if i % 2 == 0 { -18.0 } else { -10.0 }))
            .collect();
        let e = bed_gain_expr(&pts);
        assert!(
            e.matches("clip(").count() == 4999,
            "{}",
            e.matches("clip(").count()
        );
        let (max_depth, max_chain) = shape(&e);
        assert!(max_chain <= 32, "chain of {max_chain}");
        assert!(max_depth <= 12, "nesting of {max_depth}");
    }

    /// A voice-over of 60 one-word sentences, 1.0 s apart: every gap is long
    /// (0.85 s), so the envelope has more than 200 points.
    fn long_envelope() -> motion_core::audio::DuckEnvelope {
        let words: Vec<(f64, f64)> = (0..60)
            .map(|i| (1.0 + f64::from(i), 1.15 + f64::from(i)))
            .collect();
        DuckEnvelope::from_speech(&speech(&words), &MixLevels::STANDARD)
    }

    #[test]
    fn a_long_narration_envelope_parses_in_ffmpeg_and_matches_level_at() {
        if !Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            eprintln!("skipping: ffmpeg not available");
            return;
        }
        let env = long_envelope();
        assert!(env.points.len() > 200, "{} points", env.points.len());
        let expr = bed_gain_expr(&env.points);
        // The gain (dB) of a constant 0.5 signal through the same stage the
        // mix uses: frames of 256 samples, the expression evaluated per frame.
        let out = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "aevalsrc=0.5:d=70:s=48000",
            ])
            .args([
                "-af",
                &format!("asetnsamples=n={BED_GAIN_FRAME_SAMPLES}:p=0,volume='{expr}':eval=frame"),
            ])
            .args(["-f", "f32le", "-"])
            .output()
            .expect("ffmpeg runs");
        assert!(
            out.status.success(),
            "ffmpeg rejected the {}-point expression: {}",
            env.points.len(),
            String::from_utf8_lossy(&out.stderr)
        );
        let x: Vec<f32> = out
            .stdout
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        assert!(x.len() >= 70 * 48_000, "{} samples", x.len());
        // Gain at the first sample of the frame holding `t`.
        let frame = f64::from(BED_GAIN_FRAME_SAMPLES);
        let measured = |t: f64| -> (f64, f64) {
            let n = ((t * 48_000.0 / frame).floor() * frame) as usize;
            (20.0 * (f64::from(x[n]) / 0.5).log10(), n as f64 / 48_000.0)
        };
        // Mid-hold (inside a span), mid-ramp-up of a gap, mid-hold of the
        // gap, mid-ramp-down, before the first word and after the end; then
        // every 0.37 s over the whole narration.
        let mut times = vec![31.07, 31.40, 31.75, 31.92, 0.5, 45.5, 66.0, 69.5];
        times.extend((0..180).map(|i| 0.3 + f64::from(i) * 0.37));
        for t in times {
            let (got, at) = measured(t);
            let want = env.level_at(at);
            assert!(
                (got - want).abs() <= 0.1,
                "t={t}: ffmpeg {got:.3} dB vs level_at {want:.3} dB (frame at {at:.4})"
            );
        }
        // The late gaps are as deep as the early ones: -18 inside the word at
        // 59.0 s, -10 held in the gap after it.
        let (under, _) = measured(59.07);
        let (gap, _) = measured(59.75);
        assert!((gap - under - 8.0).abs() <= 0.2, "{under} -> {gap}");
    }

    fn speech(words: &[(f64, f64)]) -> SpeechMap {
        SpeechMap {
            version: SPEECH_VERSION.to_string(),
            audio: "v.wav".into(),
            sample_rate: 48_000,
            duration: 30.0,
            provider: "fixture".into(),
            model: "fixture/model".into(),
            voice: "fx".into(),
            words: words
                .iter()
                .map(|&(start, end)| SpeechWord {
                    text: "w".into(),
                    start,
                    end,
                    confidence: 1.0,
                })
                .collect(),
            sentences: vec![],
            recognised: vec![],
            alignment: None,
        }
    }

    #[test]
    fn voiced_spans_merge_gaps_shorter_than_three_tenths_of_a_second() {
        let m = speech(&[(1.0, 1.5), (1.75, 2.0), (2.31, 2.8), (4.0, 4.4)]);
        // 0.25 s merges; 0.31 s and 1.2 s do not.
        assert_eq!(voiced_spans(&m), vec![(1.0, 2.0), (2.31, 2.8), (4.0, 4.4)]);
        assert!(voiced_spans(&speech(&[])).is_empty());
    }

    #[test]
    fn momentary_values_are_read_from_the_ebur128_log() {
        let log = "[Parsed_ebur128_0 @ 0x6] t: 0.1        TARGET:-23 LUFS    M:-120.7 S:-120.7     I: -70.0 LUFS       LRA:   0.0 LU\n\
[Parsed_ebur128_0 @ 0x6] t: 0.5        TARGET:-23 LUFS    M: -30.5 S:-120.7     I: -30.4 LUFS       LRA:   0.0 LU\n\
[Parsed_ebur128_0 @ 0x6] t: 1.2        TARGET:-23 LUFS    M: -22.0 S: -25.0     I: -26.0 LUFS       LRA:   0.0 LU\n\
[Parsed_ebur128_0 @ 0x6] t: 2.0        TARGET:-23 LUFS    M:-inf S:-120.7     I: -26.0 LUFS       LRA:   0.0 LU\n\
size=N/A time=00:00:02.00";
        let m = parse_momentary(log);
        assert_eq!(m.len(), 4);
        assert_eq!(m[1], (0.5, -30.5));
        assert_eq!(m[2], (1.2, -22.0));
        assert!(m[3].1.is_infinite());
        // Windows centred (t - 0.2) inside the segment; partial windows (t < 0.4)
        // and silent ones do not count.
        assert_eq!(momentary_max_inside(&m, &[(0.0, 1.5)]), Some(-22.0));
        assert_eq!(momentary_max_inside(&m, &[(0.2, 0.4)]), Some(-30.5));
        assert_eq!(momentary_max_inside(&m, &[(1.5, 3.0)]), None);
        assert_eq!(momentary_max_inside(&m, &[(0.0, 0.1)]), None);
    }

    #[test]
    fn without_a_voice_there_is_nothing_to_resolve() {
        let plan = AudioPlan {
            version: motion_core::audio::AUDIO_PLAN_VERSION.to_string(),
            cues: vec![],
            loudness_target: -16.0,
            true_peak_limit: -1.0,
            min_spacing: 0.0,
            information_cap: 0,
            music: None,
            speech_adjustments: vec![],
            levels: Some(MixLevels::STANDARD),
        };
        assert!(resolve_relative(&plan, None, None, None)
            .expect("resolve")
            .is_none());
    }
}
