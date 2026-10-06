//! (0.23 W5a) Mix QA: is the narrator on top of the music, measured on the
//! mix's own stems.
//!
//! The checks of `qa --speech` (names and thresholds in
//! `motion_core::checks`):
//!
//! | check | measurement |
//! |---|---|
//! | `voice_over_music` | voice loudness - bed loudness over the voiced segments |
//! | `bed_over_voice` | the bed's largest momentary (400 ms) loudness in the narration gaps - the voice's integrated loudness |
//! | `speech_band_masking` | the bed's 300-4000 Hz level - the voice's 300-4000 Hz level over the voiced segments |
//!
//! The definitions are those of `docs/plans/sprint_0_23/tools/stem_levels.py`
//! (the bench cross-checks the two). *Voiced segments*: the word spans of the
//! SpeechMap ([`voiced_spans`]) shifted by the voice's offset, merged across
//! gaps shorter than [`VOICED_MERGE_GAP_S`]. *Loudness* over the segments: the
//! K-weighted (BS.1770) energy mean of the samples inside them, ungated.
//! *Momentary*: 400 ms windows on the 100 ms grid (window k is centred at
//! `0.2 + 0.1 k` s), a window belongs to a segment or gap when its centre does,
//! full windows only. *Narration gaps*: the gaps between consecutive voiced
//! segments (each at least 0.3 s). The voice's *integrated* loudness is the
//! gated one (BS.1770-4: absolute gate -70 LUFS, relative gate -10 LU).
//!
//! [`StemMeter`] is the pure measurement (interleaved stereo f32 at 48 kHz in,
//! block sums out). [`stem_qa`] rebuilds the stems of a mix with
//! [`write_stems`] in a temp directory that is removed before it returns (also
//! on error) and measures them.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use motion_core::audio::{
    AudioPlan, DuckEnvelope, MusicPlan, SfxLibrary, MUSIC_PLAN_VERSION, SFX_LIBRARY_VERSION,
};
use motion_core::checks::{
    BED_OVER_VOICE_MAX_DB, SPEECH_BAND_MASKING_MAX_DB, VOICE_OVER_MUSIC_FAIL_DB,
    VOICE_OVER_MUSIC_PASS_DB,
};
use motion_core::speech::SpeechMap;

use crate::audio_mix::{voiced_spans, write_stems, MixOptions, VoiceRelative, VoiceTrack};
use crate::speech_qa::CheckStatus;
use crate::RenderError;

/// The stems are float wav at this rate (`audio_mix::write_stems`).
const SAMPLE_RATE: f64 = 48_000.0;
/// Frames in one 100 ms block (the momentary hop).
const BLOCK_FRAMES: u32 = 4_800;
/// Blocks in one 400 ms momentary window.
const WINDOW_BLOCKS: usize = 4;
/// Seconds between momentary windows (the hop).
const HOP_S: f64 = 0.1;
/// BS.1770 absolute gate (LUFS) and relative gate (LU below the mean).
const ABSOLUTE_GATE_LUFS: f64 = -70.0;
const RELATIVE_GATE_LU: f64 = 10.0;

/// One second-order section, direct form II transposed, `a0` normalised to 1.
#[derive(Debug, Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl Biquad {
    /// ffmpeg's `highpass` / `lowpass` (two poles, Q 0.707): the RBJ cookbook
    /// filters of the BS.1770-style speech band.
    fn highpass(freq: f64, q: f64) -> Biquad {
        let w0 = 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE;
        let (cos, alpha) = (w0.cos(), w0.sin() / (2.0 * q));
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 + cos) / 2.0 / a0,
            b1: -(1.0 + cos) / a0,
            b2: (1.0 + cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    fn lowpass(freq: f64, q: f64) -> Biquad {
        let w0 = 2.0 * std::f64::consts::PI * freq / SAMPLE_RATE;
        let (cos, alpha) = (w0.cos(), w0.sin() / (2.0 * q));
        let a0 = 1.0 + alpha;
        Biquad {
            b0: (1.0 - cos) / 2.0 / a0,
            b1: (1.0 - cos) / a0,
            b2: (1.0 - cos) / 2.0 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }

    fn step(&self, state: &mut [f64; 2], x: f64) -> f64 {
        let y = self.b0 * x + state[0];
        state[0] = self.b1 * x - self.a1 * y + state[1];
        state[1] = self.b2 * x - self.a2 * y;
        y
    }
}

/// BS.1770-4 K-weighting at 48 kHz: the high-shelf pre-filter, then the RLB
/// high-pass (the coefficients of the standard, as in `stem_levels.py`).
const K_PRE: Biquad = Biquad {
    b0: 1.535_124_859_586_97,
    b1: -2.691_696_189_406_38,
    b2: 1.198_392_810_852_85,
    a1: -1.690_659_293_182_41,
    a2: 0.732_480_774_215_85,
};
const K_RLB: Biquad = Biquad {
    b0: 1.0,
    b1: -2.0,
    b2: 1.0,
    a1: -1.990_047_454_833_98,
    a2: 0.990_072_250_366_21,
};
/// The speech band: 300 Hz and 4 kHz, two poles each (ffmpeg's default Q).
const BAND_Q: f64 = 0.707;
const BAND_LOW_HZ: f64 = 300.0;
const BAND_HIGH_HZ: f64 = 4000.0;

/// Filter state of one channel: K-weighting (2 sections) and speech band (2).
#[derive(Debug, Clone, Copy, Default)]
struct ChannelState {
    k_pre: [f64; 2],
    k_rlb: [f64; 2],
    hp: [f64; 2],
    lp: [f64; 2],
}

fn lufs_of(power: f64) -> f64 {
    -0.691 + 10.0 * power.max(1e-12).log10()
}

/// Streaming measurement of one stem (interleaved stereo f32, 48 kHz): the
/// K-weighted and speech-band energy inside the voiced segments and the
/// K-weighted energy of every 100 ms block. Feed it with [`StemMeter::push`],
/// read it with [`StemMeter::finish`].
#[derive(Debug, Clone)]
pub struct StemMeter {
    /// The voiced segments as frame ranges `[start, end)`, ascending.
    segments: Vec<(u64, u64)>,
    seg: usize,
    frame: u64,
    channels: [ChannelState; 2],
    hp: Biquad,
    lp: Biquad,
    voiced_k: f64,
    voiced_band: f64,
    voiced_frames: u64,
    block_sum: f64,
    block_n: u32,
    blocks: Vec<f64>,
}

impl StemMeter {
    /// `segments` are voiced segments in seconds on the project timeline,
    /// ascending and disjoint (see [`voiced_segments`]).
    pub fn new(segments: &[(f64, f64)]) -> StemMeter {
        StemMeter {
            segments: segments
                .iter()
                .map(|&(a, b)| {
                    (
                        (a.max(0.0) * SAMPLE_RATE) as u64,
                        (b.max(0.0) * SAMPLE_RATE) as u64,
                    )
                })
                .collect(),
            seg: 0,
            frame: 0,
            channels: [ChannelState::default(); 2],
            hp: Biquad::highpass(BAND_LOW_HZ, BAND_Q),
            lp: Biquad::lowpass(BAND_HIGH_HZ, BAND_Q),
            voiced_k: 0.0,
            voiced_band: 0.0,
            voiced_frames: 0,
            block_sum: 0.0,
            block_n: 0,
            blocks: Vec::new(),
        }
    }

    /// Feed interleaved stereo frames (`L R L R ...`); a trailing odd sample
    /// is ignored.
    pub fn push(&mut self, samples: &[f32]) {
        for pair in samples.chunks_exact(2) {
            let (mut k_power, mut band_power) = (0.0, 0.0);
            for (ch, &x) in pair.iter().enumerate() {
                let x = f64::from(x);
                let st = &mut self.channels[ch];
                let k = K_RLB.step(&mut st.k_rlb, K_PRE.step(&mut st.k_pre, x));
                let band = self.lp.step(&mut st.lp, self.hp.step(&mut st.hp, x));
                k_power += k * k;
                band_power += band * band;
            }
            while self.seg < self.segments.len() && self.frame >= self.segments[self.seg].1 {
                self.seg += 1;
            }
            if self.seg < self.segments.len() && self.frame >= self.segments[self.seg].0 {
                self.voiced_k += k_power;
                self.voiced_band += band_power;
                self.voiced_frames += 1;
            }
            self.block_sum += k_power;
            self.block_n += 1;
            if self.block_n == BLOCK_FRAMES {
                self.blocks.push(self.block_sum);
                self.block_sum = 0.0;
                self.block_n = 0;
            }
            self.frame += 1;
        }
    }

    pub fn finish(self) -> StemLevels {
        StemLevels {
            frames: self.frame,
            voiced_frames: self.voiced_frames,
            voiced_k_power: self.voiced_k,
            voiced_band_power: self.voiced_band,
            blocks: self.blocks,
        }
    }
}

/// What [`StemMeter`] measured on one stem.
#[derive(Debug, Clone, PartialEq)]
pub struct StemLevels {
    /// Frames fed.
    pub frames: u64,
    voiced_frames: u64,
    voiced_k_power: f64,
    voiced_band_power: f64,
    /// Summed K-weighted power of each complete 100 ms block.
    blocks: Vec<f64>,
}

impl StemLevels {
    /// K-weighted energy mean over the voiced segments (LUFS, ungated); `None`
    /// when no sample lies in a segment or they are silent.
    pub fn voiced_lufs(&self) -> Option<f64> {
        let power = self.voiced_k_power / self.voiced_frames.max(1) as f64;
        (self.voiced_frames > 0 && power > 0.0).then(|| lufs_of(power))
    }

    /// 300-4000 Hz energy mean over the voiced segments (dB re full scale,
    /// plain energy summed over channels).
    pub fn voiced_band_db(&self) -> Option<f64> {
        let power = self.voiced_band_power / self.voiced_frames.max(1) as f64;
        (self.voiced_frames > 0 && power > 0.0).then(|| 10.0 * power.log10())
    }

    /// Momentary loudness (LUFS) of every full 400 ms window; window `k` is
    /// centred at `0.2 + 0.1 k` s.
    pub fn momentary_lufs(&self) -> Vec<f64> {
        self.window_powers().into_iter().map(lufs_of).collect()
    }

    fn window_powers(&self) -> Vec<f64> {
        let window = f64::from(BLOCK_FRAMES) * WINDOW_BLOCKS as f64;
        self.blocks
            .windows(WINDOW_BLOCKS)
            .map(|w| w.iter().sum::<f64>() / window)
            .collect()
    }

    /// The largest momentary loudness over the windows whose centre lies in
    /// one of `segments` (seconds); `None` when there is none.
    pub fn momentary_max_in(&self, segments: &[(f64, f64)]) -> Option<f64> {
        self.momentary_lufs()
            .into_iter()
            .enumerate()
            .filter(|&(k, _)| {
                let centre = WINDOW_BLOCKS as f64 / 2.0 * HOP_S + k as f64 * HOP_S;
                segments
                    .iter()
                    .any(|&(a, b)| centre >= a - 1e-6 && centre <= b + 1e-6)
            })
            .map(|(_, l)| l)
            .fold(None, |best, l| Some(best.map_or(l, |b: f64| b.max(l))))
    }

    /// Gated integrated loudness (BS.1770-4); `None` when every window is
    /// below the absolute gate.
    pub fn integrated_lufs(&self) -> Option<f64> {
        let powers = self.window_powers();
        let mean = |kept: &[f64]| kept.iter().sum::<f64>() / kept.len() as f64;
        let above: Vec<f64> = powers
            .iter()
            .copied()
            .filter(|&p| lufs_of(p) > ABSOLUTE_GATE_LUFS)
            .collect();
        if above.is_empty() {
            return None;
        }
        let relative = lufs_of(mean(&above)) - RELATIVE_GATE_LU;
        let gated: Vec<f64> = above
            .into_iter()
            .filter(|&p| lufs_of(p) > relative)
            .collect();
        (!gated.is_empty()).then(|| lufs_of(mean(&gated)))
    }
}

/// The voiced segments of a voice-over placed `offset` seconds into the
/// project: [`voiced_spans`] shifted by the offset (they are already merged
/// across gaps shorter than `VOICED_MERGE_GAP_S`).
pub fn voiced_segments(speech: &SpeechMap, offset: f64) -> Vec<(f64, f64)> {
    voiced_spans(speech)
        .into_iter()
        .map(|(a, b)| (a + offset, b + offset))
        .collect()
}

/// The narration gaps: between consecutive voiced segments.
pub fn narration_gaps(segments: &[(f64, f64)]) -> Vec<(f64, f64)> {
    segments.windows(2).map(|w| (w[0].1, w[1].0)).collect()
}

/// The three measured values of one mix (dB) and what they come from. A value
/// is `None` when its input is missing (no gap, silent stem).
#[derive(Debug, Clone, PartialEq)]
pub struct StemMeasure {
    pub voiced_segments: usize,
    pub voiced_seconds: f64,
    pub gaps: usize,
    /// The voice's gated integrated loudness (LUFS): `bed_over_voice`'s reference.
    pub voice_integrated_lufs: Option<f64>,
    pub voice_voiced_lufs: Option<f64>,
    pub bed_voiced_lufs: Option<f64>,
    /// The bed's largest momentary loudness in the narration gaps (LUFS).
    pub bed_gap_momentary_max_lufs: Option<f64>,
    pub voice_over_music: Option<f64>,
    pub bed_over_voice: Option<f64>,
    pub speech_band_masking: Option<f64>,
}

/// The measured values from the two stems' levels.
pub fn evaluate(voice: &StemLevels, bed: &StemLevels, segments: &[(f64, f64)]) -> StemMeasure {
    let gaps = narration_gaps(segments);
    let voice_integrated = voice.integrated_lufs();
    let (voice_voiced, bed_voiced) = (voice.voiced_lufs(), bed.voiced_lufs());
    let gap_max = if gaps.is_empty() {
        None
    } else {
        bed.momentary_max_in(&gaps)
    };
    StemMeasure {
        voiced_segments: segments.len(),
        voiced_seconds: segments.iter().map(|&(a, b)| b - a).sum(),
        gaps: gaps.len(),
        voice_integrated_lufs: voice_integrated,
        voice_voiced_lufs: voice_voiced,
        bed_voiced_lufs: bed_voiced,
        bed_gap_momentary_max_lufs: gap_max,
        voice_over_music: voice_voiced.zip(bed_voiced).map(|(v, b)| v - b),
        bed_over_voice: gap_max.zip(voice_integrated).map(|(m, v)| m - v),
        speech_band_masking: bed
            .voiced_band_db()
            .zip(voice.voiced_band_db())
            .map(|(b, v)| b - v),
    }
}

/// `voice_over_music`: PASS at or above 15 dB, WARN from 10, FAIL below.
pub fn voice_over_music_status(db: f64) -> CheckStatus {
    if db >= VOICE_OVER_MUSIC_PASS_DB {
        CheckStatus::Pass
    } else if db >= VOICE_OVER_MUSIC_FAIL_DB {
        CheckStatus::Warn
    } else {
        CheckStatus::Fail
    }
}

/// `bed_over_voice`: PASS at or below voice - 6 dB, FAIL above.
pub fn bed_over_voice_status(db: f64) -> CheckStatus {
    if db <= BED_OVER_VOICE_MAX_DB {
        CheckStatus::Pass
    } else {
        CheckStatus::Fail
    }
}

/// `speech_band_masking`: PASS at or below voice - 18 dB, WARN above.
pub fn speech_band_masking_status(db: f64) -> CheckStatus {
    if db <= SPEECH_BAND_MASKING_MAX_DB {
        CheckStatus::Pass
    } else {
        CheckStatus::Warn
    }
}

/// Meter `file` (any audio ffmpeg reads; decoded to float stereo 48 kHz and
/// streamed, so a long stem is never held in memory).
pub fn meter_file(file: &Path, segments: &[(f64, f64)]) -> Result<StemLevels, RenderError> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(file)
        .args([
            "-map", "0:a:0", "-f", "f32le", "-ac", "2", "-ar", "48000", "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    let mut meter = StemMeter::new(segments);
    let mut read_error = None;
    if let Some(mut out) = child.stdout.take() {
        let mut buf = vec![0u8; 1 << 16];
        let mut carry: Vec<u8> = Vec::new();
        let mut samples: Vec<f32> = Vec::with_capacity(buf.len() / 4);
        loop {
            match out.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    let whole = carry.len() / 8 * 8;
                    samples.clear();
                    samples.extend(
                        carry[..whole]
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    );
                    meter.push(&samples);
                    carry.drain(..whole);
                }
                Err(e) => {
                    read_error = Some(e);
                    break;
                }
            }
        }
    }
    let status = child.wait()?;
    if let Some(e) = read_error {
        return Err(RenderError::Encode(format!(
            "decoding {} failed: {e}",
            file.display()
        )));
    }
    if !status.success() {
        return Err(RenderError::Encode(format!(
            "decoding {} failed ({status})",
            file.display()
        )));
    }
    Ok(meter.finish())
}

/// Measure the `voice` and `bed` stems (project timeline; `spans` are the
/// voiced spans on the voice's own timeline, `offset` its start).
pub fn measure_stems(
    voice: &Path,
    bed: &Path,
    spans: &[(f64, f64)],
    offset: f64,
) -> Result<StemMeasure, RenderError> {
    let segments: Vec<(f64, f64)> = spans
        .iter()
        .map(|&(a, b)| (a + offset, b + offset))
        .collect();
    let v = meter_file(voice, &segments)?;
    let b = meter_file(bed, &segments)?;
    Ok(evaluate(&v, &b, &segments))
}

/// A directory removed when dropped (also on an error return or a panic).
struct TempDir(PathBuf);

impl TempDir {
    fn create(base: &Path) -> Result<TempDir, RenderError> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        std::fs::create_dir_all(base)?;
        // The name never reaches an output: it only keeps concurrent QA runs apart.
        let name = format!(
            "motion-stem-qa-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let dir = base.join(name);
        std::fs::create_dir(&dir)?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The `<track stem>.music.json` beside a bed's track (the `music-index`
/// output `render` and `mix` take the bed's loudness and range from).
fn music_plan_beside(track: &Path) -> Option<MusicPlan> {
    let stem = track.file_stem()?.to_string_lossy().into_owned();
    let text = std::fs::read_to_string(track.with_file_name(format!("{stem}.music.json"))).ok()?;
    let plan: MusicPlan = serde_json::from_str(&text).ok()?;
    (plan.version == MUSIC_PLAN_VERSION).then_some(plan)
}

/// What [`stem_qa`] rebuilds the stems from.
#[derive(Debug, Clone, Copy)]
pub struct StemQa<'a> {
    /// The audio plan the mix was made from (`plan.levels = None`: the legacy
    /// graph; its cues are not mixed: the voice and bed stems do not depend
    /// on the SFX bus, except through the legacy bed's extra SFX duck).
    pub plan: &'a AudioPlan,
    pub speech: &'a SpeechMap,
    /// The voice-over audio file.
    pub voice: &'a Path,
    /// Where the voice starts on the project timeline (s).
    pub offset: f64,
    /// The directory the plan's bed track is relative to.
    pub music_root: &'a Path,
    /// Project duration (s).
    pub duration: f64,
    /// Where the temp directory goes (`None`: the system temp directory).
    pub temp_base: Option<&'a Path>,
}

/// Rebuild the stems as the render did (`plan.levels`: the voice-relative
/// graph with the speech-led envelope, the bed loudness and range of the
/// `.music.json` beside the track and the gap cap; without: the legacy graph)
/// in a temp directory, measure them and delete the directory. Errors when the
/// plan has no bed or the mix cannot be built.
pub fn stem_qa(q: &StemQa) -> Result<StemMeasure, RenderError> {
    let Some(bed) = q.plan.music.as_ref() else {
        return Err(RenderError::Asset(
            "mix qa: the plan has no music bed".into(),
        ));
    };
    let track = q.music_root.join(&bed.track);
    let mut plan = q.plan.clone();
    plan.cues.clear();
    let relative = plan.levels.map(|levels| {
        let beside = music_plan_beside(&track);
        VoiceRelative {
            envelope: Some(DuckEnvelope::from_speech(q.speech, &levels)),
            bed_lufs: beside.as_ref().and_then(|m| m.lufs),
            bed_lra: beside.as_ref().and_then(|m| m.lra),
            voiced: voiced_spans(q.speech),
        }
    });
    let voice = VoiceTrack {
        path: q.voice.to_path_buf(),
        offset: q.offset,
        gain_db: 0.0,
    };
    let library = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: Vec::new(),
    };
    let default_base = std::env::temp_dir();
    let dir = TempDir::create(q.temp_base.unwrap_or(&default_base))?;
    // The voice file stands in for the video (input 0 of the graph, unused by
    // a stems-only run).
    let files = write_stems(
        q.voice,
        &plan,
        Path::new("."),
        &library,
        Some(q.music_root),
        q.duration,
        &dir.0,
        &MixOptions {
            voice: Some(&voice),
            relative: relative.as_ref(),
            ..MixOptions::default()
        },
    )?;
    let (Some(voice_stem), Some(bed_stem)) = (files.voice, files.bed) else {
        return Err(RenderError::Encode(
            "mix qa: the stems were not written".into(),
        ));
    };
    measure_stems(&voice_stem, &bed_stem, &voiced_spans(q.speech), q.offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Interleaved stereo of `seconds` with `f(t)` on both channels.
    fn stereo(seconds: f64, f: impl Fn(f64) -> f64) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE) as usize;
        let mut out = Vec::with_capacity(n * 2);
        for i in 0..n {
            let x = f(i as f64 / SAMPLE_RATE) as f32;
            out.push(x);
            out.push(x);
        }
        out
    }

    fn sine(freq: f64, amp: f64) -> impl Fn(f64) -> f64 {
        move |t| amp * (2.0 * std::f64::consts::PI * freq * t).sin()
    }

    fn meter(samples: &[f32], segments: &[(f64, f64)]) -> StemLevels {
        let mut m = StemMeter::new(segments);
        // Odd chunk sizes: the chunking never changes the result.
        for chunk in samples.chunks(7_001 * 2) {
            m.push(chunk);
        }
        m.finish()
    }

    #[test]
    fn a_k_weighted_1khz_sine_reads_its_reference_loudness() {
        // Both channels at amplitude 0.1: power 2 * 0.005, K gain +0.691 dB at
        // 1 kHz: -0.691 + 10 log10(0.01) + 0.691 = -20 LUFS.
        let s = stereo(3.0, sine(1000.0, 0.1));
        let l = meter(&s, &[(0.5, 2.5)]);
        let v = l.voiced_lufs().expect("voiced");
        assert!((v + 20.0).abs() < 0.05, "{v}");
        let i = l.integrated_lufs().expect("integrated");
        assert!((i + 20.0).abs() < 0.05, "{i}");
        let m = l.momentary_max_in(&[(0.0, 3.0)]).expect("momentary");
        assert!((m + 20.0).abs() < 0.05, "{m}");
    }

    #[test]
    fn the_speech_band_passes_1khz_and_removes_the_extremes() {
        let at = |f: f64| {
            meter(&stereo(2.0, sine(f, 0.1)), &[(0.5, 1.5)])
                .voiced_band_db()
                .expect("band")
        };
        let mid = at(1000.0);
        // 2 channels x 0.1^2 / 2 = 0.01: -20 dB.
        assert!((mid + 20.0).abs() < 0.1, "{mid}");
        // Two poles: 12 dB per octave (28 dB at 60 Hz, 19 dB or more at 12 kHz).
        assert!(at(60.0) < mid - 25.0, "{}", at(60.0));
        assert!(at(12_000.0) < mid - 15.0, "{}", at(12_000.0));
        // The corners are half power (-3 dB), as ffmpeg's two-pole filters are.
        assert!((at(300.0) - mid + 3.0).abs() < 0.3, "{}", at(300.0));
        assert!((at(4000.0) - mid + 3.0).abs() < 0.3, "{}", at(4000.0));
    }

    #[test]
    fn momentary_windows_are_on_the_100ms_grid_and_belong_by_their_centre() {
        // Silent 0..2 s, a tone 2..4 s: windows centred in the silence read the
        // floor, the first window centred at 2.2 s (covers 2.0..2.4) is fully
        // inside the tone.
        let tone = sine(1000.0, 0.1);
        let s = stereo(5.0, |t| {
            if (2.0..4.0).contains(&t) {
                tone(t)
            } else {
                0.0
            }
        });
        let l = meter(&s, &[]);
        let m = l.momentary_lufs();
        // Window k covers [0.1 k, 0.1 k + 0.4]: full windows up to k = 46.
        assert_eq!(m.len(), 47);
        assert!(m[10] < -100.0, "{}", m[10]); // 1.0..1.4 s
        assert!((m[20] + 20.0).abs() < 0.1, "{}", m[20]); // 2.0..2.4 s
        assert!(m[19] < -20.0 && m[19] > -30.0, "{}", m[19]); // 1.9..2.3 s: 3/4 tone
                                                              // A segment picks the windows whose centre lies inside it, ends included.
        let only_tone_start = l.momentary_max_in(&[(2.2, 2.2)]).expect("one window");
        assert!((only_tone_start + 20.0).abs() < 0.1, "{only_tone_start}");
        let silence = l.momentary_max_in(&[(0.5, 1.5)]).expect("silence");
        assert!(silence < -100.0, "{silence}");
        assert_eq!(l.momentary_max_in(&[(9.0, 9.5)]), None);
    }

    #[test]
    fn integrated_loudness_gates_the_silence_and_the_quiet_parts() {
        // 4 s of -20 LUFS tone in 8 s of digital silence: the silence is gated
        // out (an ungated mean would read -24.8); the six windows straddling the
        // tone's edges are partly silent and pull the mean a little under -20.
        let tone = sine(1000.0, 0.1);
        let s = stereo(12.0, |t| {
            if (1.0..5.0).contains(&t) {
                tone(t)
            } else {
                0.0
            }
        });
        let l = meter(&s, &[]);
        let i = l.integrated_lufs().expect("integrated");
        assert!(i < -20.0 && i > -20.6, "{i}");
        // A quiet passage more than 10 LU under the loud one is gated out too.
        let loud = sine(1000.0, 0.1);
        let quiet = sine(1000.0, 0.01);
        let s = stereo(8.0, |t| if t < 4.0 { loud(t) } else { quiet(t) });
        let i = meter(&s, &[]).integrated_lufs().expect("integrated");
        assert!(i < -20.0 && i > -20.6, "{i}");
        // Pure silence has none.
        assert_eq!(meter(&stereo(2.0, |_| 0.0), &[]).integrated_lufs(), None);
    }

    #[test]
    fn the_voiced_energy_mean_is_ungated_and_sample_exact() {
        // Tone in [1, 2) s, the segment [0.5, 2.5): half of it is silent, so the
        // mean power is half of the tone's: 3 dB under.
        let tone = sine(1000.0, 0.1);
        let s = stereo(3.0, |t| {
            if (1.0..2.0).contains(&t) {
                tone(t)
            } else {
                0.0
            }
        });
        let v = meter(&s, &[(0.5, 2.5)]).voiced_lufs().expect("voiced");
        assert!((v + 20.0 + 3.01).abs() < 0.1, "{v}");
        // A segment past the end of the stem has no samples.
        assert_eq!(meter(&s, &[(5.0, 6.0)]).voiced_lufs(), None);
    }

    #[test]
    fn the_measures_follow_the_definitions() {
        // Voice: 1 kHz at 0.1 in 0.5..2.5 and 5.5..6.5 (-20 LUFS); bed: 1 kHz at
        // 0.01 while the voice speaks (-40) and 0.03 in the gap (-30.5).
        let voice = sine(1000.0, 0.1);
        let bed = sine(1000.0, 0.01);
        let loud = sine(1000.0, 0.03);
        let speaking = |t: f64| (0.5..2.5).contains(&t) || (5.5..6.5).contains(&t);
        let v = stereo(8.0, |t| if speaking(t) { voice(t) } else { 0.0 });
        let b = stereo(8.0, |t| {
            if speaking(t) {
                bed(t)
            } else if (3.0..5.0).contains(&t) {
                loud(t)
            } else {
                0.0
            }
        });
        let segments = [(0.5, 2.5), (5.5, 6.5)];
        let m = evaluate(&meter(&v, &segments), &meter(&b, &segments), &segments);
        assert_eq!((m.voiced_segments, m.gaps), (2, 1));
        assert!((m.voiced_seconds - 3.0).abs() < 1e-9);
        // 20 dB over the bed while speaking.
        let vom = m.voice_over_music.expect("vom");
        assert!((vom - 20.0).abs() < 0.1, "{vom}");
        // The gap [2.5, 5.5]: the loudest window (-30.46) against the voice's
        // gated integrated loudness (a little under -20: the windows on the
        // edges of the 3 s of speech are partly silent).
        let gap_max = m.bed_gap_momentary_max_lufs.expect("gap max");
        assert!((gap_max + 30.46).abs() < 0.05, "{gap_max}");
        let integrated = m.voice_integrated_lufs.expect("integrated");
        assert!(integrated < -20.0 && integrated > -21.5, "{integrated}");
        let bov = m.bed_over_voice.expect("bov");
        assert!((bov - (gap_max - integrated)).abs() < 1e-9, "{bov}");
        // Same band, same difference as the loudness: -20 dB.
        let band = m.speech_band_masking.expect("band");
        assert!((band + 20.0).abs() < 0.2, "{band}");
        // One segment: no gap, no `bed_over_voice`.
        let one = [(0.5, 2.5)];
        let m = evaluate(&meter(&v, &one), &meter(&b, &one), &one);
        assert_eq!((m.gaps, m.bed_over_voice), (0, None));
    }

    #[test]
    fn the_status_thresholds_are_the_frozen_ones() {
        use CheckStatus::{Fail, Pass, Warn};
        assert_eq!(voice_over_music_status(15.0), Pass);
        assert_eq!(voice_over_music_status(14.99), Warn);
        assert_eq!(voice_over_music_status(10.0), Warn);
        assert_eq!(voice_over_music_status(9.99), Fail);
        assert_eq!(bed_over_voice_status(-6.0), Pass);
        assert_eq!(bed_over_voice_status(-5.99), Fail);
        assert_eq!(speech_band_masking_status(-18.0), Pass);
        assert_eq!(speech_band_masking_status(-17.99), Warn);
    }

    #[test]
    fn segments_shift_by_the_offset_and_gaps_sit_between_them() {
        use motion_core::speech::{SpeechWord, SPEECH_VERSION};
        let word = |t: &str, start: f64, end: f64| SpeechWord {
            text: t.into(),
            start,
            end,
            confidence: 1.0,
        };
        let map = SpeechMap {
            version: SPEECH_VERSION.to_string(),
            audio: "v.wav".into(),
            sample_rate: 48_000,
            duration: 6.0,
            provider: "fixture".into(),
            model: "fixture/model".into(),
            voice: "fx".into(),
            // 0.2 s between the first two words: one segment; 1.5 s to the third.
            words: vec![
                word("a", 0.5, 1.0),
                word("b", 1.2, 1.7),
                word("c", 3.2, 4.0),
            ],
            sentences: vec![],
            recognised: vec![],
            alignment: None,
        };
        let seg = voiced_segments(&map, 0.5);
        let want = [(1.0, 2.2), (3.7, 4.5)];
        assert_eq!(seg.len(), 2);
        for (got, want) in seg.iter().zip(want) {
            assert!((got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9);
        }
        let gaps = narration_gaps(&seg);
        assert_eq!(gaps.len(), 1);
        assert!((gaps[0].0 - 2.2).abs() < 1e-9 && (gaps[0].1 - 3.7).abs() < 1e-9);
        assert!(narration_gaps(&seg[..1]).is_empty());
    }
}
