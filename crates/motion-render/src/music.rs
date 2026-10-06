//! Offline music analysis (0.8): tempo, beat grid, downbeats.
//! Deterministic DSP only (no ML, no network). See docs/SOUND_DESIGN.md §7.
//!
//! Pipeline: ffmpeg decode (mono, 22.05 kHz) -> onset envelope (positive
//! difference of 512-sample RMS) -> coarse tempo by autocorrelation on the
//! 512-sample-hop envelope (parabolic interpolation + octave check) -> tempo
//! and phase refinement by a comb search on a 32-sample-hop envelope (so the
//! beat grid is accurate to a few ms, not one 23 ms frame) -> downbeats from
//! the strongest 4-beat residue.

use std::path::Path;
use std::process::Command;

use motion_core::audio::{MusicPlan, MUSIC_PLAN_VERSION};

use crate::RenderError;

#[derive(Debug, Clone, Default)]
pub struct MusicIndexOptions {
    /// Analysis cache (`<sha256>.json` = MusicPlan without `track`); None = no cache.
    pub cache_dir: Option<std::path::PathBuf>,
}

const SAMPLE_RATE: f64 = 22050.0;
/// Analysis window (23 ms) and hop of the coarse (tempo) envelope.
const WIN: usize = 512;
const COARSE_HOP: usize = 512;
/// Hop of the fine envelope used for beat phase / tempo refinement (1.45 ms).
const FINE_HOP: usize = 32;
const MIN_BPM: f64 = 60.0;
/// Centre of the tempo prior (BPM).
const TEMPO_PRIOR_BPM: f64 = 120.0;
const MAX_BPM: f64 = 180.0;
const MIN_DURATION_S: f64 = 4.0;
/// Bed target loudness (LUFS) used for `gain_db` (the pre-0.23 absolute
/// normalisation; the voice-relative mix places the bed against the voice).
pub const BED_LUFS: f64 = -18.0;

fn round_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

fn round_tenth(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Analyse `track`; `track_rel` is stored as `MusicPlan.track` (relative to
/// the music root). See SOUND_DESIGN §7 for the algorithm.
pub fn index_music(
    track: &Path,
    track_rel: &str,
    opts: &MusicIndexOptions,
) -> Result<MusicPlan, RenderError> {
    let sha = crate::sfx::sha256_file(track)?;
    let cache_file = opts
        .cache_dir
        .as_ref()
        .map(|d| d.join(format!("{sha}.json")));
    if let Some(cf) = &cache_file {
        if let Ok(text) = std::fs::read_to_string(cf) {
            if let Ok(mut plan) = serde_json::from_str::<MusicPlan>(&text) {
                if plan.version == MUSIC_PLAN_VERSION && plan.sha256 == sha && plan.lufs.is_some() {
                    plan.track = track_rel.to_string();
                    return Ok(plan);
                }
            }
        }
    }

    let samples = decode_mono(track)?;
    let duration = samples.len() as f64 / SAMPLE_RATE;
    if duration < MIN_DURATION_S {
        return Err(RenderError::Encode(format!(
            "music {}: {:.2} s is shorter than the {} s minimum",
            track.display(),
            duration,
            MIN_DURATION_S
        )));
    }
    if !samples.iter().any(|s| s.abs() > 1e-6) {
        return Err(RenderError::Encode(format!(
            "music {}: no audible audio",
            track.display()
        )));
    }

    let (bpm, beats, fine) = analyse_rhythm(&samples).ok_or_else(|| {
        RenderError::Encode(format!("music {}: no rhythm detected", track.display()))
    })?;
    let downbeats = pick_downbeats(&beats, &fine);

    let loudness = measure_loudness(track);
    let gain_db = match loudness {
        Some((l, _)) => round_tenth((BED_LUFS - l).clamp(-30.0, 0.0)),
        None => 0.0,
    };

    let mut plan = MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: String::new(),
        duration: round_ms(duration),
        bpm: round_tenth(bpm),
        beat_times: beats.iter().map(|&t| round_ms(t)).collect(),
        downbeat_times: downbeats.iter().map(|&t| round_ms(t)).collect(),
        sections: Vec::new(),
        gain_db,
        sha256: sha,
        // (0.23) What the voice-relative mix places the bed from.
        lufs: loudness.map(|(l, _)| round_tenth(l)),
        lra: loudness.map(|(_, r)| round_tenth(r)),
    };

    if let Some(cf) = &cache_file {
        write_cache(cf, &plan);
    }
    plan.track = track_rel.to_string();
    Ok(plan)
}

/// Best-effort atomic cache write (temp file + rename); failures are ignored
/// because the cache is only an accelerator.
fn write_cache(path: &Path, plan: &MusicPlan) {
    let Ok(mut text) = serde_json::to_string_pretty(plan) else {
        return;
    };
    text.push('\n');
    if let Some(dir) = path.parent() {
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

fn decode_mono(path: &Path) -> Result<Vec<f32>, RenderError> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "22050", "-f", "f32le", "-"])
        .output()
        .map_err(|e| RenderError::Encode(format!("cannot run ffmpeg: {e}")))?;
    if !out.status.success() {
        return Err(RenderError::Encode(format!(
            "decode {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    if samples.is_empty() {
        return Err(RenderError::Encode(format!(
            "decode {}: no audio samples",
            path.display()
        )));
    }
    Ok(samples)
}

/// (0.14) Transient-energy envelope of an audio file sampled at `fps`: the
/// onset strength (positive RMS difference) normalised by its 95th
/// percentile, clamped to 0..=1, with an instant attack and a 0.82-per-frame
/// release so pulses read as hits. Deterministic for the same file.
pub fn energy_envelope(path: &Path, fps: f32) -> Result<motion_core::scene::Envelope, RenderError> {
    let samples = decode_mono(path)?;
    let hop = (22_050.0 / f64::from(fps.max(1.0))).round().max(1.0) as usize;
    let raw = onset_envelope(&samples, hop);
    let mut sorted = raw.clone();
    sorted.sort_by(f64::total_cmp);
    let p95 = sorted
        .get(((sorted.len() as f64) * 0.95) as usize)
        .copied()
        .unwrap_or(0.0)
        .max(1e-9);
    let mut level = 0.0f64;
    let values = raw
        .into_iter()
        .map(|v| {
            let x = (v / p95).min(1.0);
            level = if x > level { x } else { level * 0.82 };
            level as f32
        })
        .collect();
    Ok(motion_core::scene::Envelope {
        id: "music".into(),
        fps,
        values,
    })
}

/// Integrated loudness (LUFS) and loudness range (LU) via ffmpeg `ebur128`;
/// `None` when unparsable or gated out.
fn measure_loudness(path: &Path) -> Option<(f64, f64)> {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-nostdin", "-i"])
        .arg(path)
        .args(["-af", "ebur128", "-f", "null", "-"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stderr);
    let summary = &text[text.rfind("Summary:")?..];
    let value = |label: &str| -> Option<f64> {
        let line = summary.lines().find_map(|l| l.trim().strip_prefix(label))?;
        line.split_whitespace().next()?.parse().ok()
    };
    let lufs = value("I:")?;
    if !lufs.is_finite() || lufs <= -70.0 {
        return None;
    }
    let lra = value("LRA:").filter(|v| v.is_finite()).unwrap_or(0.0);
    Some((lufs, lra))
}

/// Onset envelope: positive difference of windowed RMS (window `WIN`, given
/// hop), half-wave rectified. Index `i` (>= 1) corresponds to samples newly
/// entering the window at sample `(i-1)*hop + WIN`; index 0 is 0.
fn onset_envelope(samples: &[f32], hop: usize) -> Vec<f64> {
    if samples.len() < WIN {
        return Vec::new();
    }
    // Prefix sums of squares in f64 (deterministic, O(n)).
    let mut prefix = Vec::with_capacity(samples.len() + 1);
    let mut acc = 0.0f64;
    prefix.push(0.0);
    for &s in samples {
        acc += f64::from(s) * f64::from(s);
        prefix.push(acc);
    }
    let frames = (samples.len() - WIN) / hop + 1;
    let mut env = vec![0.0; frames];
    let mut prev = 0.0;
    for (i, e) in env.iter_mut().enumerate() {
        let start = i * hop;
        let rms = ((prefix[start + WIN] - prefix[start]).max(0.0) / WIN as f64).sqrt();
        if i > 0 {
            *e = (rms - prev).max(0.0);
        }
        prev = rms;
    }
    env
}

fn remove_mean(env: &mut [f64]) {
    if env.is_empty() {
        return;
    }
    let mean = env.iter().sum::<f64>() / env.len() as f64;
    for e in env.iter_mut() {
        *e -= mean;
    }
}

/// Time (s) of fine-envelope index `i`.
fn fine_time(i: f64) -> f64 {
    ((i - 1.0) * FINE_HOP as f64 + WIN as f64) / SAMPLE_RATE
}

/// Fine-envelope index nearest to time `t`.
fn fine_index(t: f64) -> i64 {
    ((t * SAMPLE_RATE - WIN as f64) / FINE_HOP as f64).round() as i64 + 1
}

/// Returns (bpm, beat_times, rectified fine envelope) or None when the track
/// has no usable onsets.
fn analyse_rhythm(samples: &[f32]) -> Option<(f64, Vec<f64>, Vec<f64>)> {
    // 1. Coarse tempo from the autocorrelation of the 23 ms-hop envelope.
    let mut coarse = onset_envelope(samples, COARSE_HOP);
    remove_mean(&mut coarse);
    let energy: f64 = coarse.iter().map(|v| v * v).sum();
    if coarse.len() < 8 || energy <= 1e-18 {
        return None;
    }
    let fps = SAMPLE_RATE / COARSE_HOP as f64;
    let lag_lo = ((fps * 60.0 / MAX_BPM).floor() as usize).max(2);
    let lag_hi = (fps * 60.0 / MIN_BPM).ceil() as usize;
    if coarse.len() <= lag_hi + 2 {
        return None;
    }
    let ac = |lag: usize| -> f64 {
        coarse
            .iter()
            .zip(coarse.iter().skip(lag))
            .map(|(a, b)| a * b)
            .sum()
    };
    let table: Vec<f64> = (0..=lag_hi + 1).map(ac).collect();
    // Log-Gaussian tempo prior (centre 120 BPM, sigma 1 octave): resolves the
    // equal-height 60/120 BPM octave ambiguity of real beds toward the tempo
    // a listener taps (Ellis 2007).
    let prior = |lag: usize| -> f64 {
        let bpm = 60.0 * fps / lag as f64;
        let octaves = (bpm / TEMPO_PRIOR_BPM).log2();
        (-0.5 * octaves * octaves).exp()
    };
    let mut best_lag = lag_lo;
    let mut best_val = f64::NEG_INFINITY;
    for (lag, &v) in table.iter().enumerate().take(lag_hi + 1).skip(lag_lo) {
        if v <= 0.0 {
            continue;
        }
        let w = v * prior(lag);
        if w > best_val {
            best_val = w;
            best_lag = lag;
        }
    }
    if best_val <= 0.0 {
        return None;
    }
    // Parabolic interpolation.
    let (y0, y1, y2) = (table[best_lag - 1], table[best_lag], table[best_lag + 1]);
    let denom = y0 - 2.0 * y1 + y2;
    let delta = if denom.abs() > 1e-18 {
        (0.5 * (y0 - y2) / denom).clamp(-0.5, 0.5)
    } else {
        0.0
    };
    let mut lag = best_lag as f64 + delta;

    // Fine (hop 32) mean-removed envelope: used for the octave check and the
    // refinement below. The 23 ms-hop autocorrelation is too coarse for the
    // octave check (non-integer lags in frames smear the peak).
    let fine_raw = onset_envelope(samples, FINE_HOP);
    let mut fine = fine_raw.clone();
    remove_mean(&mut fine);
    // Octave check: prefer the double tempo (half lag) when its
    // autocorrelation is >= 0.8x and the result stays <= MAX_BPM.
    let half = lag / 2.0;
    if 60.0 * fps / half <= MAX_BPM + 1e-9 {
        let to_fine = (COARSE_HOP / FINE_HOP) as f64;
        let peak_near = |centre: f64| -> f64 {
            let c = (centre * to_fine).round() as i64;
            let r = (COARSE_HOP / FINE_HOP) as i64; // +-1 coarse frame
            let mut m = f64::NEG_INFINITY;
            for l in (c - r).max(1)..=(c + r) {
                let l = l as usize;
                if l < fine.len() {
                    let v: f64 = fine
                        .iter()
                        .zip(fine.iter().skip(l))
                        .map(|(a, b)| a * b)
                        .sum();
                    m = m.max(v);
                }
            }
            m
        };
        if peak_near(half) >= 0.8 * peak_near(lag) {
            lag = half;
        }
    }
    let coarse_bpm = 60.0 * fps / lag;

    // 2. Refine tempo and phase by a comb search over the fine envelope
    //    (+-4 % around the coarse tempo, ~0.05 BPM steps; phase in hops).
    let fine_fps = SAMPLE_RATE / FINE_HOP as f64;
    let n = fine.len();
    let steps = 160i32;
    let mut best: Option<(f64, f64, usize)> = None; // (score, bpm, phase index)
    for s in 0..=steps {
        let bpm = coarse_bpm * (0.96 + 0.08 * f64::from(s) / f64::from(steps));
        let period = 60.0 / bpm * fine_fps;
        let phases = period.ceil() as usize;
        for p in 0..phases {
            let mut score = 0.0;
            let mut k = 0.0f64;
            loop {
                let idx = (p as f64 + k * period).round() as usize;
                if idx >= n {
                    break;
                }
                score += fine[idx];
                k += 1.0;
            }
            if best.is_none_or(|(b, _, _)| score > b) {
                best = Some((score, bpm, p));
            }
        }
    }
    let (_, bpm, phase_idx) = best?;
    let period_s = 60.0 / bpm;
    // Keep the first beat in [0, period).
    let phase = fine_time(phase_idx as f64).rem_euclid(period_s);
    let duration = samples.len() as f64 / SAMPLE_RATE;
    let mut beats = Vec::new();
    let mut k = 0usize;
    loop {
        let t = phase + k as f64 * period_s;
        if t >= duration {
            break;
        }
        beats.push(t);
        k += 1;
    }
    Some((bpm, beats, fine_raw))
}

/// Every 4th beat starting at the residue (0..3) whose beats carry the most
/// envelope energy (max of the fine envelope within +-40 ms of each beat).
fn pick_downbeats(beats: &[f64], fine: &[f64]) -> Vec<f64> {
    if beats.is_empty() {
        return Vec::new();
    }
    let radius = (0.040 * SAMPLE_RATE / FINE_HOP as f64) as i64;
    let mut sums = [0.0f64; 4];
    for (i, &t) in beats.iter().enumerate() {
        let centre = fine_index(t);
        let mut m = 0.0f64;
        for j in (centre - radius)..=(centre + radius) {
            if j >= 0 && (j as usize) < fine.len() {
                m = m.max(fine[j as usize]);
            }
        }
        sums[i % 4] += m;
    }
    let mut best = 0usize;
    for (r, &v) in sums.iter().enumerate().skip(1) {
        if v > sums[best] {
            best = r;
        }
    }
    beats.iter().skip(best).step_by(4).copied().collect()
}
