//! (0.24 E1) `motion-engine edit <video>`: transcript -> EditPlan -> preview edit
//! (overlays only). The CLI wires ingest
//! (ffprobe/ffmpeg pipes, local ASR), the pure rules in `motion_core::edit`,
//! and the segmented render in `edit_render`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use motion_core::edit::normalize::{
    collapse_loops, normalize, regularize_times, NormKind, NormWord,
};
use motion_core::edit::translit::to_roman;
use motion_core::edit::{analyse, plan_edit, EditPlan, EventKind, FaceSeg, PlanInput, Tok};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::StyleProfile;
use motion_render::brand_mark::{BrandMark, Corner};
use motion_render::FontMeasure;
use motion_voice::asr::AsrWord;
use motion_voice::asr_ctc;
use motion_voice::asr_local::{self, LocalParams};

use crate::edit_cutaway::{build as build_cutaway, video_family, CutCtx};
use crate::edit_render::{self as er, Ctx, Fonts, RenderJob};
use crate::edit_sound as es;

#[derive(clap::Args)]
pub struct EditArgs {
    /// The raw talking-head video (read only).
    pub video: PathBuf,
    /// StyleProfile JSON (accent colour).
    #[arg(long)]
    pub style: Option<PathBuf>,
    /// Brand mark PNG drawn by the renderer as the top layer.
    #[arg(long)]
    pub brand_mark: Option<PathBuf>,
    /// Corner for the brand mark (tl, tr, bl, br).
    #[arg(long, default_value = "tr")]
    pub brand_mark_corner: String,
    /// Stop after the transcript and the EditPlan.
    #[arg(long)]
    pub plan_only: bool,
    /// Never touch the network (E1 has no network step; accepted for the final CLI).
    #[arg(long)]
    pub offline: bool,
    /// Output directory.
    #[arg(short = 'o', long, default_value = "output/edit")]
    pub output: PathBuf,
    /// Asset root (fonts).
    #[arg(long, default_value = "assets")]
    pub assets: PathBuf,
    /// Placeholder text for the lower third (the user fills it).
    #[arg(long, default_value = "\u{2014}")]
    pub lower_third: String,
    /// Local recogniser model (see `models list`). Default: whisper-large-v3-turbo
    /// when installed, else whisper-base.en (English only).
    #[arg(long)]
    pub asr_model: Option<String>,
    /// Spoken language: auto (detect), hi (Hindi/Hinglish) or en.
    #[arg(long, default_value = "auto", value_parser = ["auto", "hi", "en"])]
    pub language: String,
    /// Render from this (edited) EditPlan instead of planning; the transcript
    /// must already be in the output directory.
    #[arg(long)]
    pub plan: Option<PathBuf>,
    /// A brand intro MP4 prepended to the finished talk (0.2 s audio crossfade).
    #[arg(long)]
    pub intro: Option<PathBuf>,
    /// Use this already-rendered picture (a kept `*.edit.mp4`) instead of
    /// rendering the segments again; only the sound step runs.
    #[arg(long)]
    pub picture: Option<PathBuf>,
    /// Keep the silent-mix picture next to the final file (for `--picture`).
    #[arg(long)]
    pub keep_picture: bool,
    /// Skip the sound mix (the preview keeps the source audio untouched).
    #[arg(long)]
    pub no_sound: bool,
    /// Variety seed for the cutaway compiles (the backdrop family follows it).
    #[arg(long, default_value_t = 3)]
    pub seed: u64,
    /// Keep rendered cutaway clips here and reuse them on the next run
    /// (development only; delete the directory when done).
    #[arg(long)]
    pub clip_cache: Option<PathBuf>,
    /// Reuse this transcript (SpeechMap JSON from an earlier run) instead of
    /// recognising the speech again.
    #[arg(long)]
    pub transcript: Option<PathBuf>,
    /// Process only the first N seconds (testing).
    #[arg(long)]
    pub limit_seconds: Option<f64>,
    /// Segment length for the bounded-RAM render, seconds.
    #[arg(long, default_value_t = 60.0)]
    pub segment_seconds: f64,
}

/// A temp directory removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Result<Self> {
        let p = std::env::temp_dir().join(format!("motion_edit_{}", std::process::id()));
        std::fs::create_dir_all(&p)?;
        Ok(TempDir(p))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Probe {
    w: u32,
    h: u32,
    fps: f64,
    frames: u32,
    duration: f64,
}

fn probe(video: &Path) -> Result<Probe> {
    let p = motion_render::video::probe(video).map_err(|e| anyhow::anyhow!("{e}"))?;
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=p=0",
        ])
        .arg(video)
        .output()
        .context("cannot run ffprobe")?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (w, h) = text
        .trim()
        .split_once(',')
        .context("ffprobe: no frame size")?;
    Ok(Probe {
        w: w.trim().parse()?,
        h: h.trim().parse()?,
        fps: p.fps,
        frames: p.frame_count,
        duration: p.duration_s,
    })
}

/// 16 kHz mono s16 audio, streamed from ffmpeg (no temp file).
fn extract_audio(video: &Path, limit: Option<f64>) -> Result<Vec<i16>> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-i"]).arg(video);
    if let Some(l) = limit {
        cmd.args(["-t", &format!("{l:.3}")]);
    }
    cmd.args(["-vn", "-ac", "1", "-ar", "16000", "-f", "s16le", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = cmd.spawn().context("cannot run ffmpeg")?;
    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .context("ffmpeg stdout")?
        .read_to_end(&mut raw)?;
    if !child.wait()?.success() {
        bail!("ffmpeg could not extract the audio");
    }
    Ok(raw
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect())
}

/// The quietest 100 ms window within +-3 s of `target` (sample index).
fn quiet_cut(samples: &[i16], target: usize) -> usize {
    let (lo, hi) = (
        target.saturating_sub(48_000),
        (target + 48_000).min(samples.len()),
    );
    let win = 1600;
    let (mut best, mut at) = (f64::INFINITY, target);
    let mut i = lo;
    while i + win <= hi {
        let e: f64 = samples[i..i + win]
            .iter()
            .map(|&s| (s as f64).powi(2))
            .sum();
        if e < best {
            best = e;
            at = i + win / 2;
        }
        i += 800;
    }
    at
}

struct Heard {
    words: Vec<AsrWord>,
    label: String,
    language: String,
    ctc_skipped: bool,
}

fn transcribe(
    samples: &[i16],
    spec: &'static asr_local::ModelSpec,
    language: &str,
) -> Result<Heard> {
    let models = asr_local::models_root();
    if !asr_local::AVAILABLE {
        bail!("this build has no local recogniser; build with `--features asr-local,asr-ctc`");
    }
    if asr_local::installed(&models, spec).is_none() {
        bail!(
            "model {} is not installed; run `motion-engine models fetch {}`",
            spec.name,
            spec.name
        );
    }
    let english_only = spec.name.ends_with(".en");
    let ctc = asr_ctc::AVAILABLE && asr_ctc::installed(&models, &asr_ctc::MODEL).is_some();
    let mut params = LocalParams {
        ctc,
        guard: true,
        language: if english_only {
            "en".into()
        } else {
            language.to_string()
        },
        ..LocalParams::default()
    };
    let cache = motion_voice::cache::default_root();
    // Bounded RAM: decode in chunks of about 4 minutes, cut at the quietest moment.
    const CHUNK: usize = 240 * 16_000;
    let mut words = Vec::new();
    let mut a = 0usize;
    let mut detected = String::new();
    let mut ctc_applied = ctc;
    while a < samples.len() {
        let b = if samples.len() - a <= CHUNK + 48_000 {
            samples.len()
        } else {
            quiet_cut(samples, a + CHUNK)
        };
        let wav = motion_voice::wav::encode_at(&samples[a..b], 16_000);
        let t = asr_local::transcribe_detect(&wav, spec, &models, &params, &cache)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        // The first chunk decides the language for the rest.
        if detected.is_empty() {
            detected = t.language.clone();
            if params.language == "auto" {
                params.language = detected.clone();
            }
        }
        ctc_applied &= t.ctc_applied;
        let off = a as f64 / 16_000.0;
        words.extend(t.words.into_iter().map(|w| AsrWord {
            word: w.word,
            start: w.start + off,
            end: w.end + off,
        }));
        a = b;
    }
    let label_params = LocalParams {
        ctc: ctc_applied,
        ..params.clone()
    };
    Ok(Heard {
        words,
        label: asr_local::alignment_label(spec, &label_params),
        language: if detected.is_empty() {
            "en".into()
        } else {
            detected
        },
        ctc_skipped: ctc && !ctc_applied,
    })
}

fn speech_map(
    name: &str,
    raw: &[AsrWord],
    norm: &[NormWord],
    duration: f64,
    model: &str,
    alignment: String,
    toks: &[Tok],
) -> SpeechMap {
    let analysis = analyse(toks, &[]);
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: name.to_string(),
        sample_rate: 16_000,
        duration,
        provider: "local".into(),
        model: model.to_string(),
        voice: String::new(),
        // The normalised, Roman-script words (what captions show).
        words: norm
            .iter()
            .map(|w| SpeechWord {
                text: w.text.trim().to_string(),
                start: w.start,
                end: w.end,
                confidence: 1.0,
            })
            .collect(),
        sentences: analysis
            .sentences
            .iter()
            .enumerate()
            .map(|(i, &(a, z))| SpeechSentence {
                beat: i,
                start: toks[a].start,
                end: toks[z].end,
            })
            .collect(),
        // What the recogniser said, before transliteration and normalisation.
        recognised: raw
            .iter()
            .map(|w| motion_core::speech::RecognisedWord {
                word: w.word.trim().to_string(),
                start: w.start,
                end: w.end,
            })
            .collect(),
        alignment: Some(alignment),
    }
}

fn english_ratio(toks: &[Tok]) -> f64 {
    const EN: &[&str] = &[
        "the", "and", "is", "to", "of", "a", "in", "that", "it", "you", "for", "this", "on",
        "with", "are", "we", "so", "but", "have", "was", "hai", "ki", "ka", "ke", "ko", "mein",
        "toh", "nahi", "aur", "yeh", "bhi", "kya", "hain", "se", "par", "kuch", "bahut", "hum",
        "aap", "main", "woh", "jo", "ye",
    ];
    if toks.is_empty() {
        return 0.0;
    }
    toks.iter()
        .filter(|t| EN.contains(&t.norm.as_str()))
        .count() as f64
        / toks.len() as f64
}

/// Share of words inside 4-grams that repeat 5+ times (a decoding loop).
fn loop_share(toks: &[Tok]) -> f64 {
    let mut seen: std::collections::BTreeMap<Vec<&str>, usize> = std::collections::BTreeMap::new();
    for w in toks.windows(4) {
        *seen
            .entry(w.iter().map(|t| t.norm.as_str()).collect())
            .or_default() += 1;
    }
    let looped: usize = seen.values().filter(|&&c| c >= 5).sum();
    looped as f64 / toks.len().max(1) as f64
}

/// Whisper's word times for Hindi can sit a few tenths of a second early
/// (no CTC alignment). For a word that follows a gap, move its start onto
/// the nearest voice onset in the loudness envelope (10 ms frames): forward
/// to the first voiced run if the start falls in silence, back to the onset
/// if speech has already begun. Returns how many words moved.
fn snap_to_onsets(toks: &mut [Tok], env: &[f64]) -> usize {
    if env.len() < 100 {
        return 0;
    }
    let mut sorted: Vec<f64> = env.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let floor = sorted[sorted.len() / 10];
    let peak = sorted[sorted.len() * 95 / 100];
    let thr = floor + (peak - floor) * 0.35;
    let voiced = |t: f64| {
        let i = (t * 100.0) as usize;
        env.get(i).is_some_and(|&v| v > thr)
    };
    let run = |t: f64| (0..6).all(|k| voiced(t + k as f64 * 0.01));
    let mut moved = 0;
    for i in 0..toks.len() {
        let prev_end = if i == 0 { 0.0 } else { toks[i - 1].end };
        let s = toks[i].start;
        if i > 0 && s - prev_end < 0.12 {
            continue;
        }
        let mut new = s;
        if !voiced(s + 0.03) {
            let mut t = s;
            while t < s + 0.7 && !run(t) {
                t += 0.01;
            }
            if run(t) {
                new = t;
            }
        } else if voiced(s - 0.05) {
            let mut t = s;
            while t > prev_end.max(s - 0.5) && voiced(t - 0.01) {
                t -= 0.01;
            }
            new = t;
        }
        let new = new.clamp(prev_end, toks[i].end.max(new));
        if (new - s).abs() >= 0.04 && new < toks[i].end + 0.5 {
            if new + 0.08 > toks[i].end {
                toks[i].end = new + 0.15;
            }
            toks[i].start = new;
            moved += 1;
            // Keep the previous word from running into this one.
            if i > 0 && toks[i - 1].end > new {
                toks[i - 1].end = new;
            }
        }
    }
    moved
}

/// 10 ms voice-band (pre-emphasised) loudness in dB.
fn loudness_env(samples: &[i16]) -> Vec<f64> {
    let mut prev = 0.0f64;
    let mut out = Vec::with_capacity(samples.len() / 160 + 1);
    for chunk in samples.chunks(160) {
        let mut e = 0.0;
        for &s in chunk {
            let x = s as f64 / 32768.0;
            let y = x - 0.95 * prev;
            prev = x;
            e += y * y;
        }
        out.push(20.0 * ((e / chunk.len() as f64).sqrt() + 1e-5).log10());
    }
    out
}

// ---------------------------------------------------------------------------
// Speaker face box: skin-tone blob per sampled second, smoothed.
// ---------------------------------------------------------------------------

const FW: usize = 160;
const FH: usize = 90;

/// Normalised `[x0, y0, x1, y1]` of the skin blob in the central region.
fn skin_box(rgb: &[u8]) -> Option<[f32; 4]> {
    let (mut xs, mut ys) = (Vec::new(), Vec::new());
    for y in 0..(FH * 72 / 100) {
        for x in (FW / 5)..(FW * 4 / 5) {
            let i = (y * FW + x) * 3;
            let (r, g, b) = (rgb[i] as f32, rgb[i + 1] as f32, rgb[i + 2] as f32);
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            let cb = 128.0 - 0.168736 * r - 0.331264 * g + 0.5 * b;
            let cr = 128.0 + 0.5 * r - 0.418688 * g - 0.081312 * b;
            if luma > 35.0
                && (77.0..=127.0).contains(&cb)
                && (133.0..=173.0).contains(&cr)
                && r > g
                && g > b * 0.8
            {
                xs.push(x as f32);
                ys.push(y as f32);
            }
        }
    }
    if xs.len() < 60 {
        return None;
    }
    let pct = |v: &mut Vec<f32>, p: f32| {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v[((v.len() - 1) as f32 * p) as usize]
    };
    let (x0, x1) = (pct(&mut xs, 0.05), pct(&mut xs, 0.95));
    let (y0, y1) = (pct(&mut ys, 0.03), pct(&mut ys, 0.95));
    let (w, h) = (x1 - x0, y1 - y0);
    if w < FW as f32 * 0.06 || w > FW as f32 * 0.55 {
        return None;
    }
    let (px, py) = (w * 0.15, h * 0.15);
    Some([
        (x0 - px).max(0.0) / FW as f32,
        (y0 - py).max(0.0) / FH as f32,
        (x1 + px).min(FW as f32) / FW as f32,
        (y1 + py).min(FH as f32 * 0.70) / FH as f32,
    ])
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// Face boxes per 5 s (union of the smoothed per-second boxes), canvas pixels.
fn face_segments(
    video: &Path,
    limit: Option<f64>,
    canvas: (u32, u32),
    duration: f64,
) -> Result<(Vec<FaceSeg>, f64)> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-i"]).arg(video);
    if let Some(l) = limit {
        cmd.args(["-t", &format!("{l:.3}")]);
    }
    cmd.args([
        "-an",
        "-vf",
        &format!("fps=1,scale={FW}:{FH}"),
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgb24",
        "pipe:1",
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit());
    let mut child = cmd.spawn().context("cannot run ffmpeg")?;
    let mut out = child.stdout.take().context("ffmpeg stdout")?;
    let mut boxes: Vec<Option<[f32; 4]>> = Vec::new();
    let mut buf = vec![0u8; FW * FH * 3];
    while out.read_exact(&mut buf).is_ok() {
        boxes.push(skin_box(&buf));
    }
    let _ = child.wait();
    let found = boxes.iter().flatten().count();
    let rate = if boxes.is_empty() {
        0.0
    } else {
        found as f64 / boxes.len() as f64
    };
    let (cw, ch) = (canvas.0 as f32, canvas.1 as f32);
    let fixed = [cw / 3.0, ch * 0.05, cw / 3.0, ch * 0.65];
    let n = boxes.len();
    // Median over +-2 s of the frames that found a face.
    let smooth: Vec<Option<[f32; 4]>> = (0..n)
        .map(|i| {
            let near: Vec<[f32; 4]> = boxes[i.saturating_sub(2)..(i + 3).min(n)]
                .iter()
                .flatten()
                .cloned()
                .collect();
            if near.is_empty() {
                return None;
            }
            let mut r = [0.0; 4];
            for k in 0..4 {
                let mut v: Vec<f32> = near.iter().map(|b| b[k]).collect();
                r[k] = median(&mut v);
            }
            Some(r)
        })
        .collect();
    let mut segs = Vec::new();
    let mut t = 0.0;
    while t < duration {
        let end = (t + 5.0).min(duration);
        let (a, z) = (t as usize, (end.ceil() as usize).min(n));
        let mut u: Option<[f32; 4]> = None;
        if rate >= 0.5 {
            for b in smooth[a.min(n)..z.max(a.min(n))].iter().flatten() {
                u = Some(match u {
                    None => *b,
                    Some(c) => [
                        c[0].min(b[0]),
                        c[1].min(b[1]),
                        c[2].max(b[2]),
                        c[3].max(b[3]),
                    ],
                });
            }
        }
        let (rect, detected) = match u {
            Some(b) => (
                [b[0] * cw, b[1] * ch, (b[2] - b[0]) * cw, (b[3] - b[1]) * ch],
                true,
            ),
            None => (fixed, false),
        };
        segs.push(FaceSeg {
            start: t,
            end,
            rect,
            detected,
        });
        t = end;
    }
    Ok((segs, rate))
}

fn stem_of(video: &Path) -> String {
    let s = video
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("talk")
        .trim();
    let s: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        "talk".into()
    } else {
        s
    }
}

pub fn run(args: EditArgs) -> Result<()> {
    let wall = Instant::now();
    let name = stem_of(&args.video);
    let src_name = args
        .video
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("source")
        .to_string();
    std::fs::create_dir_all(&args.output)?;
    let tmp = TempDir::new()?;
    let pr = probe(&args.video)?;
    let fps = pr.fps.round().max(1.0) as u32;
    let duration = args
        .limit_seconds
        .map(|l| l.min(pr.duration))
        .unwrap_or(pr.duration);
    let total_frames = if args.limit_seconds.is_some() {
        (duration * fps as f64).round() as u32
    } else {
        pr.frames
    };
    println!(
        "source: {}x{} @ {:.2} fps, {:.1} s",
        pr.w, pr.h, pr.fps, duration
    );
    let transcript_path = args.output.join(format!("{name}.transcript.json"));
    let plan_path = args.output.join(format!("{name}.edit.json"));

    // 1. transcript (or reload).
    let samples;
    let toks: Vec<Tok>;
    let mut language = "en".to_string();
    let mut env: Vec<f64> = Vec::new();
    if args.plan.is_some() || args.transcript.is_some() {
        let from = args.transcript.as_ref().unwrap_or(&transcript_path);
        let map = SpeechMap::from_json(
            &std::fs::read_to_string(from)
                .with_context(|| format!("reading {}", from.display()))?,
        )?;
        toks = map
            .words
            .iter()
            .map(|w| Tok::new(&w.text, w.start, w.end))
            .collect();
        if args.transcript.is_some() {
            std::fs::write(&transcript_path, serde_json::to_string_pretty(&map)?)?;
            if args.plan.is_none() {
                let s = extract_audio(&args.video, args.limit_seconds)?;
                env = loudness_env(&s);
                language = if args.language == "auto" {
                    "hi".into()
                } else {
                    args.language.clone()
                };
            }
        }
    } else {
        samples = extract_audio(&args.video, args.limit_seconds)?;
        let models = asr_local::models_root();
        let name_of = match &args.asr_model {
            Some(m) => m.clone(),
            None => {
                let turbo = asr_local::model_spec("whisper-large-v3-turbo");
                if turbo.is_some_and(|m| asr_local::installed(&models, m).is_some()) {
                    "whisper-large-v3-turbo".to_string()
                } else {
                    println!("warning: whisper-large-v3-turbo is not installed (`motion-engine models fetch whisper-large-v3-turbo`); using whisper-base.en, English only");
                    asr_local::DEFAULT_MODEL.to_string()
                }
            }
        };
        let spec = asr_local::model_spec(&name_of)
            .with_context(|| format!("unknown --asr-model {name_of}"))?;
        let t0 = Instant::now();
        let heard = transcribe(&samples, spec, &args.language)?;
        let transcribe_s = t0.elapsed().as_secs_f64();
        let label = heard.label.clone();
        // Decode loops: collapse any n-gram repeated more than three times.
        let raw_pairs: Vec<(String, f64, f64)> = heard
            .words
            .iter()
            .map(|w| (w.word.clone(), w.start, w.end))
            .collect();
        let (clean, removed) = collapse_loops(&raw_pairs);
        println!(
            "transcribed {} words in {:.1}s ({label}); language {}; decode loops removed: {removed}",
            raw_pairs.len(),
            transcribe_s,
            heard.language
        );
        if heard.ctc_skipped {
            println!(
                "ctc: skipped (the aligner is English-only; language is {})",
                heard.language
            );
        }
        // Devanagari -> Roman Hinglish, then spoken forms -> written forms.
        let roman: Vec<(String, f64, f64)> = clean
            .iter()
            .map(|(w, s, e)| (to_roman(w), *s, *e))
            .collect();
        let mut norm = normalize(&roman);
        regularize_times(&mut norm);
        let raw_words: Vec<AsrWord> = clean
            .iter()
            .map(|(w, s, e)| AsrWord {
                word: w.clone(),
                start: *s,
                end: *e,
            })
            .collect();
        toks = norm
            .iter()
            .map(|w| Tok::new(&w.text, w.start, w.end))
            .collect();
        let map = speech_map(
            &src_name, &raw_words, &norm, duration, spec.name, label, &toks,
        );
        std::fs::write(&transcript_path, serde_json::to_string_pretty(&map)?)?;
        let changed = norm.iter().filter(|w| w.kind != NormKind::Kept).count();
        println!("normalised: {changed} spans rewritten (numbers, dates, times, ordinals)");
        env = loudness_env(&samples);
        let looped = loop_share(&toks);
        if looped > 0.2 {
            println!("warning: {:.0}% of the transcript is still a repeating loop (the recogniser is lost)", looped * 100.0);
        }
        let ratio = english_ratio(&toks);
        language = if heard.language == "hi" {
            "hi".into()
        } else if ratio >= 0.18 {
            "en".into()
        } else {
            "unknown".into()
        };
        if language == "unknown" {
            println!("note: the transcript does not look English or Hinglish (stop-word share {:.0}%); pass --language to force one", ratio * 100.0);
        } else {
            println!(
                "language: {language} (detected {}, stop-word share {:.0}%)",
                heard.language,
                ratio * 100.0
            );
        }
    }

    let mut toks = toks;
    if env.is_empty() {
        env = loudness_env(&extract_audio(&args.video, args.limit_seconds)?);
    }
    let moved = snap_to_onsets(&mut toks, &env);
    println!("word times: {moved} word starts moved onto the measured voice onset");

    // 2. face box, 3. plan.
    let canvas = (pr.w, pr.h);
    let fonts = Fonts::in_assets(&args.assets);
    let measure = FontMeasure::with_fonts(fonts.all()).map_err(|e| anyhow::anyhow!("{e}"))?;
    let style = match &args.style {
        Some(p) => StyleProfile::from_json(&std::fs::read_to_string(p)?)?,
        None => StyleProfile::default(),
    };
    use motion_core::scene::Color;
    use motion_core::style::AccentRole;
    let accent = match style.accent_role {
        AccentRole::SignalRed => Color::rgb(0xE0, 0x33, 0x1F),
        AccentRole::Cobalt => Color::rgb(0x23, 0x3F, 0xD1),
        AccentRole::Acid => Color::rgb(0xC9, 0xE4, 0x2B),
    };
    let accent = style
        .brand
        .as_ref()
        .and_then(|b| motion_core::compiler::brand::parse(b).ok())
        .and_then(|b| b.primary)
        .unwrap_or(accent);
    let mut plan = match &args.plan {
        Some(p) => EditPlan::from_json(&std::fs::read_to_string(p)?)?,
        None => {
            let (face, rate) = face_segments(&args.video, args.limit_seconds, canvas, duration)?;
            println!(
                "face box: detected in {:.0}% of sampled seconds{}",
                rate * 100.0,
                if rate < 0.5 {
                    " (fixed central third used)"
                } else {
                    ""
                }
            );
            let analysis = analyse(&toks, &env);
            plan_edit(&PlanInput {
                toks: &toks,
                analysis: &analysis,
                face_boxes: face,
                canvas: [pr.w, pr.h],
                fps,
                duration,
                source: &src_name,
                language: &language,
                lower_third_text: &args.lower_third,
            })
        }
    };
    let problems = plan.problems();
    if !problems.is_empty() {
        bail!("invalid EditPlan: {}", problems.join("; "));
    }
    let face_copy = plan.face_boxes.clone();
    let ctx = Ctx {
        canvas,
        fps,
        measure: &measure,
        fonts: &fonts,
        accent,
        face: &face_copy,
    };
    let (placement, dropped) = er::lay_out(&ctx, &mut plan, &toks);
    if !dropped.is_empty() {
        println!("dropped (no room beside the face): {}", dropped.join(", "));
    }
    std::fs::write(&plan_path, serde_json::to_string_pretty(&plan)?)?;

    // 4. QA + summary.
    let brand = match &args.brand_mark {
        Some(p) => {
            let corner = Corner::parse(&args.brand_mark_corner).unwrap_or(Corner::Tr);
            let m = BrandMark::from_png(
                p,
                corner,
                canvas,
                fps,
                motion_core::scene::Color::rgb(0, 0, 0),
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            Some((p.clone(), corner, m.rect(corner)))
        }
        None => None,
    };
    let bad = er::overlay_on_face(&ctx, &plan, &placement, brand.as_ref().map(|b| b.2));
    let kinds = [
        "lower_third",
        "number_callout",
        "keyword",
        "chapter_card",
        "captions",
        "cutaway",
        "side_panel",
        "punch_in",
    ];
    let counts: Vec<String> = kinds
        .iter()
        .map(|k| format!("{k} {}", plan.count(k)))
        .collect();
    let g = er::overlay_screen_time(&plan);
    let cap_s: f64 = placement.captions.iter().map(|c| c.end - c.start).sum();
    println!(
        "events: {} | graphics {:.1}% of {:.0}s (overlays; {:.1}% with captions)",
        counts.join(", "),
        100.0 * g / duration,
        duration,
        100.0 * (g + cap_s) / duration
    );
    let mut subs: std::collections::BTreeMap<&str, usize> = Default::default();
    for e in &plan.events {
        if let EventKind::Cutaway { sub, .. } = &e.kind {
            *subs.entry(sub.name()).or_default() += 1;
        }
    }
    let longest = plan
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::Cutaway { .. }))
        .map(|e| e.end - e.start)
        .fold(0.0, f64::max);
    println!(
        "cutaways {:?}; graphics (cutaway + side panel) {:.1}% of {:.0}s; longest cutaway {:.1}s",
        subs,
        100.0 * plan.graphics_seconds() / duration,
        duration,
        longest
    );
    let deltas: Vec<f64> = plan
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::NumberCallout { .. }))
        .map(|e| {
            let shown = (e.start * fps as f64 - 1e-9).ceil() / fps as f64;
            (shown - toks[e.at_word].start) * 1000.0
        })
        .collect();
    if !deltas.is_empty() {
        let mean = deltas.iter().sum::<f64>() / deltas.len() as f64;
        let max = deltas.iter().cloned().fold(f64::MIN, f64::max);
        let min = deltas.iter().cloned().fold(f64::MAX, f64::min);
        println!("number callouts vs spoken word (first visible frame): n={} min {min:.0} ms, mean {mean:.0} ms, max {max:.0} ms", deltas.len());
    }
    if bad.is_empty() {
        println!("overlay_on_face: PASS");
    } else {
        println!("overlay_on_face: FAIL {}", bad.join(", "));
    }
    println!("plan: {}", plan_path.display());
    if !bad.is_empty() {
        bail!("overlay_on_face failed");
    }
    if args.plan_only {
        println!("wall {:.1}s", wall.elapsed().as_secs_f64());
        return Ok(());
    }

    // 5a. cutaway clips: compile each from its beats, render full-frame.
    let mut placement = placement;
    let clip_dir = args
        .clip_cache
        .clone()
        .unwrap_or_else(|| tmp.0.join("clips"));
    std::fs::create_dir_all(&clip_dir)?;
    let cctx = CutCtx {
        style: &style,
        assets: &args.assets,
        canvas,
        fps,
        dir: &clip_dir,
        seed: args.seed,
        talk: &name,
    };
    let family = video_family(&cctx);
    let t_cut = Instant::now();
    let mut cut_fail: Vec<String> = Vec::new();
    let mut cut_cues: Vec<(usize, Vec<(String, f64)>)> = Vec::new();
    let mut n_warn = 0usize;
    for (i, ev) in plan.events.iter().enumerate() {
        if !matches!(ev.kind, EventKind::Cutaway { .. }) {
            continue;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            serde_json::to_string(&ev.kind)?.hash(&mut h);
            (ev.start.to_bits(), ev.end.to_bits(), args.seed).hash(&mut h);
            family.name().hash(&mut h);
            h.finish()
        };
        let cached = clip_dir.join(format!("cut_{key:016x}.mp4"));
        if args.clip_cache.is_some() && cached.is_file() {
            let frames = motion_render::video::probe(&cached)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .frame_count;
            placement.cutaways.push(er::CutClip {
                event: i,
                path: cached,
                frames,
            });
            continue;
        }
        match build_cutaway(&cctx, i, ev, &toks, family) {
            Ok(b) => {
                n_warn += b.warnings.len();
                cut_cues.push((i, b.cues));
                let mut clip = b.clip;
                if args.clip_cache.is_some() {
                    std::fs::rename(&clip.path, &cached)?;
                    clip.path = cached;
                }
                placement.cutaways.push(clip);
            }
            Err(e) => cut_fail.push(format!("{} at {:.1}s: {e:#}", ev.kind.name(), ev.start)),
        }
    }
    for c in &mut placement.cutaways {
        c.path = c.path.canonicalize()?;
    }
    println!(
        "cutaway clips: {} rendered in {:.0}s, backdrop {}, compile warnings {n_warn}",
        placement.cutaways.len(),
        t_cut.elapsed().as_secs_f64(),
        family.name()
    );
    for f in &cut_fail {
        println!("cutaway FAILED (speaker kept on screen): {f}");
    }

    // 5b. segmented render.
    let out = args.output.join(format!("{name}.edit.mp4"));
    let video_abs = args.video.canonicalize()?;
    let assets_abs = args
        .assets
        .canonicalize()
        .with_context(|| format!("assets {}", args.assets.display()))?;
    let job = RenderJob {
        ctx: &ctx,
        plan: &plan,
        video: &video_abs,
        video_frames: pr.frames,
        video_fps: pr.fps,
        total_frames,
        seg_seconds: args.segment_seconds,
        brand: brand.map(|b| (b.0, b.1)),
        assets_dir: &assets_abs,
        tmp: &tmp.0,
    };
    let n = match &args.picture {
        Some(p) => {
            std::fs::copy(p, &out)?;
            0
        }
        None => er::render_all(&job, &placement, &video_abs, &out)?,
    };
    println!(
        "picture: {} ({n} segments), wall {:.1}s",
        out.display(),
        wall.elapsed().as_secs_f64()
    );

    // 6. figure cues: when each speech-cued group of a cutaway shows vs its word.
    let mut fig: Vec<f64> = Vec::new();
    for (i, cues) in &cut_cues {
        let ev = &plan.events[*i];
        if !matches!(&ev.kind, EventKind::Cutaway { sub, .. } if *sub == motion_core::edit::CutawayKind::Figure)
        {
            continue;
        }
        for (word, shown) in cues {
            let w = motion_core::speech::normalize_word(word);
            if let Some(t) = (ev.at_word..=ev.until_word)
                .map(|k| &toks[k])
                .find(|t| motion_core::speech::normalize_word(&t.raw) == w)
            {
                fig.push((ev.start + shown - t.start) * 1000.0);
            }
        }
    }
    if !fig.is_empty() {
        let mean = fig.iter().sum::<f64>() / fig.len() as f64;
        let min = fig.iter().cloned().fold(f64::MAX, f64::min);
        let max = fig.iter().cloned().fold(f64::MIN, f64::max);
        println!(
            "figure cues vs spoken word: n={} min {min:.0} ms, mean {mean:.0} ms, max {max:.0} ms",
            fig.len()
        );
    }

    // 7. sound, then the optional intro.
    let mut final_path = out.clone();
    if !args.no_sound {
        let mut mapped = SpeechMap::from_json(&std::fs::read_to_string(&transcript_path)?)?;
        for (w, t) in mapped.words.iter_mut().zip(&toks) {
            w.start = t.start;
            w.end = t.end;
        }
        let sound_out = args.output.join(format!("{name}.mixed.mp4"));
        let cut_ids: Vec<usize> = placement.cutaways.iter().map(|c| c.event).collect();
        let rep = es::run(&es::SoundInputs {
            video: &out,
            source: &video_abs,
            transcript: &mapped,
            plan: &plan,
            toks: &toks,
            env: &env,
            cut_events: &cut_ids,
            assets: &assets_abs,
            tmp: &tmp.0,
            duration: total_frames as f64 / fps as f64,
            output: &sound_out,
            name: &name,
        })?;
        println!(
            "sound: {}; bed {}, {} sfx cues ({} dropped: no quiet spot); sfx_over_voice worst margin {} LU ({}); loudness {} LUFS, true peak {} dBTP",
            rep.summary,
            rep.bed,
            rep.cues,
            rep.dropped_cues,
            rep.sfx_margin_lu.map_or("n/a".into(), |m| format!("{m:.1}")),
            match rep.sfx_margin_lu {
                Some(m) if m >= motion_render::mix_qa::SFX_OVER_VOICE_MIN_LU => "PASS",
                Some(_) => "FAIL",
                None => "no overlap",
            },
            rep.lufs.map_or("?".into(), |v| format!("{v:.1}")),
            rep.true_peak.map_or("?".into(), |v| format!("{v:.1}")),
        );
        if !args.keep_picture {
            std::fs::remove_file(&out)?;
        }
        final_path = sound_out;
    }
    if let Some(intro) = &args.intro {
        let with = args.output.join(format!("{name}.with_intro.mp4"));
        es::prepend_intro(intro, &final_path, canvas, fps, &with)?;
        std::fs::rename(&with, &final_path)?;
        println!(
            "intro: {} prepended (0.2 s audio crossfade)",
            intro.display()
        );
    }
    println!(
        "final: {}, wall {:.1}s",
        final_path.display(),
        wall.elapsed().as_secs_f64()
    );
    Ok(())
}
