//! (0.20) Local word timing: whisper.cpp through `whisper-rs`, on-device and
//! free (Metal on Apple Silicon, CPU elsewhere). Behind the cargo feature
//! `asr-local` (whisper.cpp needs cmake and a C++ toolchain); without it the
//! cache still serves and a miss is [`VoiceError::Unsupported`].
//!
//! Output has the shape of the paid `asr` module ([`AsrWord`]), so
//! `asr::apply` (matched words take measured times, unmatched keep their
//! estimates) and `asr::line_cuts` (word-level cuts) work unchanged.
//!
//! Determinism: results are cached by
//! `sha256(model sha256 ‖ 0 ‖ params ‖ 0 ‖ audio bytes)` (the params key ends
//! with [`WORDS_VERSION`]), so reruns are byte-identical and offline. Models
//! are downloaded only by an explicit `motion-engine models fetch <name>` from
//! a pinned URL with a pinned SHA256, written atomically; nothing downloads
//! silently.
//!
//! ## Word times (measured, sprint 0.20 A1)
//! whisper decodes with DTW token timestamps and `max_len = 1`, so every
//! segment is one word. A token's DTW mark is where the alignment path enters
//! its decoder row, which the model uses to predict the NEXT token: the mark
//! sits inside the token's own audio, about where the following token begins.
//! Measured against the ground-truth fixtures (`tests/timing_benchmark.rs`,
//! 5 fixtures, 118.8 s, 274 words; base.en, beam 1; |Δstart| in ms, signed
//! bias = mean of arm − truth):
//!
//! | source ([`TimestampSource`]) | mean | p95 | max | bias |
//! |---|---:|---:|---:|---:|
//! | `Segment` (whisper's heuristic segment t0/t1) | 207 | 574 | 1097 | +45 |
//! | `DtwOwn` (the word's first token mark) | 206 | 426 | 1470 | +204 |
//! | `DtwSpan` (the previous token's mark) | 129 | 336 | 1096 | −113 |
//! | `DtwSpan` + [`starts_after_pauses`] (shipped) | 120 | 336 | 976 | −108 |
//!
//! `DtwSpan` is used: the start is the previous text token's mark, the end
//! the mark of the word's last alphanumeric token (before trailing
//! punctuation, so a sentence's last word ends where its pause begins). After
//! a pause that mark can lie anywhere in the pause, so a start that has a
//! silent run of at least [`PAUSE_SECONDS`] before the word's own mark moves
//! to where the pause ends. Its early bias is the benign direction (pictures
//! and captions lead the voice slightly). small.en with the same rules:
//! mean 75, p95 207, max 816 ms. No compensation is applied: a constant
//! +113 ms on base.en gives mean 86 / p95 258 ms, interpolating 35 % of the
//! way from the previous mark to the word's own mark mean 70 / p95 208 ms;
//! per-word jitter (sd ≈ 115–137 ms) remains either way, so the sprint
//! target (mean ≤ 40 ms, p95 ≤ 90 ms) needs a finer aligner. Large misses
//! are multi-word numbers written as one token ("$381" read as "three
//! hundred eighty-one": the next word's start comes up to 1 s early).

use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::asr::AsrWord;
use crate::VoiceError;

/// The default local model (`--asr-model`).
pub const DEFAULT_MODEL: &str = "whisper-base.en";

/// Whether this build can run the recogniser (cargo feature `asr-local`).
/// Without it only cached results are served.
pub const AVAILABLE: bool = cfg!(feature = "asr-local");

/// whisper.cpp alignment-heads preset for DTW token timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DtwPreset {
    TinyEn,
    BaseEn,
    SmallEn,
}

/// A downloadable recogniser model: pinned URL and SHA256, never fetched
/// silently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelSpec {
    /// Name on the CLI (`models fetch <name>`, `--asr-model <name>`).
    pub name: &'static str,
    /// File name under the models root.
    pub file: &'static str,
    pub url: &'static str,
    /// Lower-case hex SHA256 of the file.
    pub sha256: &'static str,
    pub bytes: u64,
    pub preset: DtwPreset,
    /// What to tell the operator (size, speed, accuracy).
    pub note: &'static str,
}

/// The models `models fetch` knows (ggml weights from the whisper.cpp
/// repository on Hugging Face, MIT).
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        name: "whisper-tiny.en",
        file: "ggml-tiny.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.en.bin",
        sha256: "921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f",
        bytes: 77_704_715,
        preset: DtwPreset::TinyEn,
        note: "fastest, least accurate",
    },
    ModelSpec {
        name: "whisper-base.en",
        file: "ggml-base.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin",
        sha256: "a03779c86df3323075f5e796cb2ce5029f00ec8869eee3fdfb897afe36c6d002",
        bytes: 147_964_211,
        preset: DtwPreset::BaseEn,
        note: "default",
    },
    ModelSpec {
        name: "whisper-small.en",
        file: "ggml-small.en.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en.bin",
        sha256: "c6138d6d58ecc8322097e0f987c32f1be8bb0a18532a3f88f734d1bbf9c41e5d",
        bytes: 487_614_201,
        preset: DtwPreset::SmallEn,
        note: "more accurate, about 3x slower",
    },
];

/// The spec for a CLI model name.
pub fn model_spec(name: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.name == name)
}

/// Default models root: `$MOTION_MODELS_DIR`, else `~/.cache/motionengine/models/`.
pub fn models_root() -> PathBuf {
    models_root_from(
        std::env::var_os("MOTION_MODELS_DIR").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`models_root`] from explicit inputs (tests).
pub fn models_root_from(env: Option<PathBuf>, home: Option<PathBuf>) -> PathBuf {
    env.unwrap_or_else(|| {
        home.unwrap_or_else(|| PathBuf::from("."))
            .join(".cache")
            .join("motionengine")
            .join("models")
    })
}

/// Where `spec` lives under `root`.
pub fn model_path(root: &Path, spec: &ModelSpec) -> PathBuf {
    root.join(spec.file)
}

/// The installed file's size in bytes, when `spec` is present under `root`.
pub fn installed(root: &Path, spec: &ModelSpec) -> Option<u64> {
    std::fs::metadata(model_path(root, spec))
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

/// Lower-case hex SHA256 of a reader's bytes.
fn sha256_of(mut r: impl Read) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex(&h.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Check that the file at `path` is exactly `spec` (SHA256).
pub fn verify(path: &Path, spec: &ModelSpec) -> Result<(), VoiceError> {
    verify_file(path, spec.sha256)
}

/// Check that the file at `path` has the lower-case hex SHA256 `sha256`.
pub fn verify_file(path: &Path, sha256: &str) -> Result<(), VoiceError> {
    let actual = sha256_of(std::fs::File::open(path)?)?;
    if actual != sha256 {
        return Err(VoiceError::Checksum {
            file: path.display().to_string(),
            expected: sha256.to_string(),
            actual,
        });
    }
    Ok(())
}

/// Download `spec` into `root` (`<file>.part`, SHA256 checked, then renamed
/// atomically). `progress(done, total)` is called as bytes arrive. Returns
/// the installed path; an already installed and verified file is returned
/// without a download.
pub fn fetch(
    spec: &ModelSpec,
    root: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, VoiceError> {
    fetch_file(
        spec.url,
        spec.sha256,
        spec.bytes,
        &model_path(root, spec),
        progress,
    )
}

/// (0.20 A3) Download one pinned file to `dest`: written to `<dest>.part`,
/// its SHA256 checked against `sha256`, then renamed atomically (a mismatch
/// removes the part file and is [`VoiceError::Checksum`]). `bytes` is the
/// expected size, used for progress when the server sends no length;
/// `progress(done, total)` is called as bytes arrive. An already present
/// and verified `dest` is returned without a download. Every model kind
/// (whisper weights, the CTC aligner's files) is fetched through this.
pub fn fetch_file(
    url: &str,
    sha256: &str,
    bytes: u64,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<PathBuf, VoiceError> {
    if dest.is_file() && verify_file(dest, sha256).is_ok() {
        progress(bytes, bytes);
        return Ok(dest.to_path_buf());
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .build();
    let resp = match agent.get(url).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            return Err(VoiceError::Http {
                provider: "huggingface",
                status: code,
                message: r.status_text().to_string(),
            })
        }
        Err(ureq::Error::Transport(t)) => {
            return Err(VoiceError::Network(crate::openrouter::scrub_urls(
                &t.to_string(),
            )))
        }
    };
    let total = resp
        .header("Content-Length")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(bytes);
    let part = part_path(dest);
    let mut out = std::fs::File::create(&part)?;
    let mut reader = resp.into_reader();
    let mut h = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    let mut done = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n])?;
        done += n as u64;
        progress(done, total);
    }
    out.flush()?;
    drop(out);
    let actual = hex(&h.finalize());
    if actual != sha256 {
        let _ = std::fs::remove_file(&part);
        return Err(VoiceError::Checksum {
            file: url.to_string(),
            expected: sha256.to_string(),
            actual,
        });
    }
    std::fs::rename(&part, dest)?;
    Ok(dest.to_path_buf())
}

/// `<dest>.part` beside `dest` (the whole file name plus `.part`).
fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Decode parameters. Part of the cache key, so every field is explicit.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LocalParams {
    /// whisper language code (`en`); never translated.
    pub language: String,
    /// 1 = greedy; `n > 1` = beam search of width `n`.
    pub beam_size: u32,
    /// DTW token timestamps (the model's alignment-heads preset); false =
    /// whisper's heuristic token timestamps.
    pub dtw: bool,
    /// Decoder threads; 0 = whisper's default for this machine (threads do
    /// not change the result).
    pub threads: u32,
    /// (0.20 A3) Refine whisper's word times with CTC forced alignment
    /// ([`crate::asr_ctc::refine`]): whisper supplies what was said, the
    /// wav2vec2 aligner when, on its 20 ms frame grid. Needs the cargo
    /// feature `asr-ctc` and the `wav2vec2-base-960h` model.
    #[serde(default)]
    pub ctc: bool,
}

impl Default for LocalParams {
    fn default() -> Self {
        Self {
            language: "en".into(),
            beam_size: 1,
            dtw: true,
            threads: 0,
            ctc: false,
        }
    }
}

impl LocalParams {
    /// The stable text form that enters the cache key (threads excluded:
    /// they never change the output). It carries [`WORDS_VERSION`], so a
    /// change to how words are built from whisper's tokens never serves a
    /// stale cached result; with `ctc` it ends with `;ctc=1;ctcalign=`
    /// [`crate::asr_ctc::ALIGN_VERSION`] (the aligner's model and algorithm
    /// version), and without it the key is the whisper-only key unchanged.
    pub fn key(&self) -> String {
        let mut key = format!(
            "lang={};beam={};dtw={};words={WORDS_VERSION}",
            self.language,
            self.beam_size.max(1),
            u8::from(self.dtw)
        );
        if self.ctc {
            key.push_str(";ctc=1;ctcalign=");
            key.push_str(crate::asr_ctc::ALIGN_VERSION);
        }
        key
    }
}

/// Version of [`words_from_segments`] with [`TIMESTAMP_SOURCE`]; part of the
/// cache key.
pub const WORDS_VERSION: &str = "dtwspan-pause.1";

/// `SpeechMap.alignment` for a local run: `local:<model>:beam<n>`, plus
/// `+ctc` when the times were refined by CTC forced alignment.
pub fn alignment_label(spec: &ModelSpec, params: &LocalParams) -> String {
    format!(
        "local:{}:beam{}{}",
        spec.name,
        params.beam_size.max(1),
        if params.ctc { "+ctc" } else { "" }
    )
}

/// Cache path for a transcription: `sha256(model sha256 ‖ 0 ‖ params key ‖ 0 ‖ wav)`.
pub fn cache_path(root: &Path, model_sha256: &str, params: &LocalParams, wav: &[u8]) -> PathBuf {
    let mut h = Sha256::new();
    h.update(model_sha256.as_bytes());
    h.update([0]);
    h.update(params.key().as_bytes());
    h.update([0]);
    h.update(wav);
    root.join(format!("{}.local.json", hex(&h.finalize())))
}

/// whisper's input rate.
pub const WHISPER_RATE: u32 = 16_000;

/// Resample 16-bit mono PCM at `rate` to 16 kHz f32 in [-1, 1] with a
/// windowed-sinc low-pass (Hann, 24 taps per side), keeping the original
/// timebase: output sample `i` sits at `i / 16000` seconds of the input. Pure
/// and deterministic. The identity when `rate` is already 16 kHz.
pub fn resample_16k(samples: &[i16], rate: u32) -> Vec<f32> {
    let to_f = |s: i16| f32::from(s) / 32768.0;
    if rate == WHISPER_RATE || samples.is_empty() {
        return samples.iter().map(|&s| to_f(s)).collect();
    }
    let ratio = f64::from(rate) / f64::from(WHISPER_RATE); // input samples per output sample
    let cutoff = 0.5 / ratio.max(1.0); // cycles per input sample (Nyquist of the slower rate)
    const TAPS: i64 = 24;
    let n_out = ((samples.len() as f64) / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let center = i as f64 * ratio;
        let c0 = center.floor() as i64;
        let mut acc = 0.0f64;
        let mut norm = 0.0f64;
        for k in (c0 - TAPS + 1)..=(c0 + TAPS) {
            let x = k as f64 - center; // distance in input samples
            let w = 0.5 + 0.5 * (std::f64::consts::PI * x / TAPS as f64).cos(); // Hann
            let s = if x.abs() < 1e-12 {
                2.0 * cutoff
            } else {
                (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
            };
            let coef = w * s;
            norm += coef;
            if k >= 0 && (k as usize) < samples.len() {
                acc += coef * f64::from(samples[k as usize]);
            }
        }
        let v = if norm.abs() > 1e-9 { acc / norm } else { 0.0 };
        out.push((v / 32768.0) as f32);
    }
    out
}

/// Recognised words for `wav` (16-bit PCM, any rate) with the local model,
/// cached under `cache_root`. Returns `(words, cached)`. A cache miss needs
/// the `asr-local` feature and the model file at `model_path(root, spec)`
/// (else [`VoiceError::ModelMissing`] / [`VoiceError::Unsupported`]); with
/// `params.ctc` it also needs the `asr-ctc` feature
/// (`Unsupported("CTC refinement")`) and the aligner model under
/// `models_root` (`ModelMissing("wav2vec2-base-960h")`), both checked before
/// whisper runs. Everything runs on-device, so a cache miss is never a
/// network call.
pub fn transcribe(
    wav: &[u8],
    spec: &ModelSpec,
    models_root: &Path,
    params: &LocalParams,
    cache_root: &Path,
) -> Result<(Vec<AsrWord>, bool), VoiceError> {
    let path = cache_path(cache_root, spec.sha256, params, wav);
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Ok(words) = serde_json::from_str::<Vec<AsrWord>>(&text) {
            return Ok((words, true));
        }
    }
    let model = model_path(models_root, spec);
    if !model.is_file() {
        return Err(VoiceError::ModelMissing(spec.name.to_string()));
    }
    if params.ctc {
        if !crate::asr_ctc::AVAILABLE {
            return Err(VoiceError::Unsupported("CTC refinement"));
        }
        let aligner = &crate::asr_ctc::MODEL;
        if crate::asr_ctc::installed(models_root, aligner).is_none() {
            return Err(VoiceError::ModelMissing(aligner.name.to_string()));
        }
    }
    let decoded = crate::wav::decode(wav)?;
    let audio = resample_16k(&decoded.samples, decoded.sample_rate);
    let mut words = recognise(&audio, &model, spec.preset, params)?;
    if params.ctc {
        // whisper supplies WHAT was said; CTC forced alignment WHEN.
        words = crate::asr_ctc::refine(&audio, &words, models_root)?;
    }
    std::fs::create_dir_all(cache_root)?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string(&words).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)?;
    Ok((words, false))
}

/// One whisper text token (special and timestamp tokens are dropped).
/// Times are whisper's centiseconds from the clip start.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawToken {
    pub text: String,
    /// DTW mark: where the alignment path enters this token's decoder row,
    /// i.e. about where the audio of the NEXT token begins; -1 when
    /// unavailable.
    pub t_dtw: i64,
    /// whisper's heuristic token start/end.
    pub t0: i64,
    pub t1: i64,
}

/// One whisper segment. With `max_len = 1` and `split_on_word` that is one
/// word (plus attached punctuation), or an empty segment holding only
/// timestamp tokens.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RawSegment {
    pub text: String,
    /// Segment start/end (heuristic token timestamps), centiseconds.
    pub t0: i64,
    pub t1: i64,
    /// Text tokens in order.
    pub tokens: Vec<RawToken>,
}

/// Where a word's times come from (see the module docs for the measurement).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampSource {
    /// start = the mark of the word's first token, end = the mark of its last.
    DtwOwn,
    /// start/end = whisper's heuristic segment times.
    Segment,
    /// start = the mark of the previous text token (where this word's audio
    /// begins), end = the mark of the word's last alphanumeric token (where
    /// trailing punctuation, a pause or the next word begins).
    DtwSpan,
}

/// The timestamp source `recognise` uses (measured, see the module docs).
pub const TIMESTAMP_SOURCE: TimestampSource = TimestampSource::DtwSpan;

fn has_alnum(s: &str) -> bool {
    s.chars().any(char::is_alphanumeric)
}

/// A token that carries speech: alphanumerics, or `%` (read "percent").
fn spoken_token(t: &RawToken) -> bool {
    has_alnum(&t.text) || t.text.contains('%')
}

/// One word being assembled from segments.
struct WordAcc {
    text: String,
    tokens: Vec<RawToken>,
    t0: i64,
    t1: i64,
    /// Mark of the last text token before this word.
    prev_mark: Option<i64>,
}

/// whisper writes the spoken "percent" as a `%` token after the number
/// ("23%"); split it into its own word with its own token times, so a script
/// that says "23 percent" matches both words.
fn split_percent(w: WordAcc) -> Vec<WordAcc> {
    let pos = w
        .tokens
        .iter()
        .position(|t| t.text.trim_start().starts_with('%'));
    let Some(i) = pos.filter(|&i| i > 0 && has_alnum(&w.text)) else {
        return vec![w];
    };
    let Some(cut) = w.text.find('%') else {
        return vec![w];
    };
    let at = w.tokens[i].t0;
    let number_mark = w.tokens[..i]
        .iter()
        .rev()
        .map(|t| t.t_dtw)
        .find(|&m| m >= 0)
        .or(w.prev_mark);
    let percent = WordAcc {
        text: " percent".to_string(),
        tokens: w.tokens[i..].to_vec(),
        t0: at,
        t1: w.t1,
        prev_mark: number_mark,
    };
    let number = WordAcc {
        text: w.text[..cut].to_string(),
        tokens: w.tokens[..i].to_vec(),
        t0: w.t0,
        t1: at.max(w.t0),
        prev_mark: w.prev_mark,
    };
    vec![number, percent]
}

/// Whole words from whisper segments. A segment whose text does not start
/// with whitespace continues the previous word (whisper's sub-word pieces:
/// "$381", hyphenated words); empty and punctuation-only words are dropped;
/// a `%` token becomes the word "percent". Times are seconds, in time order
/// (starts clamped monotonic) with `end >= start`; an unavailable DTW mark
/// (-1) falls back to the segment times, and so does a mark more than
/// [`MARK_SANITY_CS`] outside its segment (a degenerate DTW path). Pure.
pub fn words_from_segments(segs: &[RawSegment], source: TimestampSource) -> Vec<AsrWord> {
    build_words(segs, source).0
}

/// A DTW mark this far (centiseconds) outside its segment's heuristic times
/// is discarded. Measured on the ground-truth fixtures, base.en and small.en
/// marks lie at most 0.84 s outside (p99 0.73 s); a degenerate decode
/// (tiny.en on one fixture) put every mark of a clip at its end, 19 s away.
pub const MARK_SANITY_CS: i64 = 150;

/// [`words_from_segments`] plus each word's own first-token mark (seconds;
/// its end when no mark is available), which bounds [`starts_after_pauses`].
fn build_words(segs: &[RawSegment], source: TimestampSource) -> (Vec<AsrWord>, Vec<f64>) {
    let mut words: Vec<WordAcc> = Vec::new();
    let mut last_mark: Option<i64> = None;
    for seg in segs {
        if seg.text.trim().is_empty() && seg.tokens.iter().all(|t| t.text.trim().is_empty()) {
            continue;
        }
        let (lo, hi) = (seg.t0.min(seg.t1), seg.t0.max(seg.t1));
        let tokens: Vec<RawToken> = seg
            .tokens
            .iter()
            .map(|t| {
                let sane = t.t_dtw >= lo - MARK_SANITY_CS && t.t_dtw <= hi + MARK_SANITY_CS;
                RawToken {
                    t_dtw: if t.t_dtw >= 0 && sane { t.t_dtw } else { -1 },
                    ..t.clone()
                }
            })
            .collect();
        let seg = &RawSegment {
            tokens,
            ..seg.clone()
        };
        let continues = !seg.text.starts_with(char::is_whitespace) && !words.is_empty();
        match words.last_mut() {
            Some(w) if continues => {
                w.text.push_str(&seg.text);
                w.tokens.extend(seg.tokens.iter().cloned());
                w.t1 = w.t1.max(seg.t1);
            }
            _ => words.push(WordAcc {
                text: seg.text.clone(),
                tokens: seg.tokens.clone(),
                t0: seg.t0,
                t1: seg.t1,
                prev_mark: last_mark,
            }),
        }
        if let Some(m) = seg.tokens.iter().rev().map(|t| t.t_dtw).find(|&m| m >= 0) {
            last_mark = Some(m);
        }
    }
    let cs = |v: i64| v as f64 / 100.0;
    let mark = |t: &RawToken| (t.t_dtw >= 0).then(|| cs(t.t_dtw));
    let finite = |v: f64| if v.is_finite() { v.max(0.0) } else { 0.0 };
    let mut out: Vec<AsrWord> = Vec::with_capacity(words.len());
    let mut own: Vec<f64> = Vec::with_capacity(words.len());
    for w in words.into_iter().flat_map(split_percent) {
        let text = w.text.trim().to_string();
        if !has_alnum(&text) {
            continue;
        }
        let (seg_start, seg_end) = (cs(w.t0), cs(w.t1.max(w.t0)));
        let (start, end) = match source {
            TimestampSource::Segment => (seg_start, seg_end),
            TimestampSource::DtwOwn => (
                w.tokens.iter().find_map(mark).unwrap_or(seg_start),
                w.tokens.iter().rev().find_map(mark).unwrap_or(seg_end),
            ),
            TimestampSource::DtwSpan => (
                w.prev_mark.map(cs).unwrap_or(seg_start),
                w.tokens
                    .iter()
                    .rev()
                    .filter(|t| spoken_token(t))
                    .find_map(mark)
                    .unwrap_or(seg_end),
            ),
        };
        own.push(finite(w.tokens.iter().find_map(mark).unwrap_or(end)));
        out.push(AsrWord {
            word: text,
            start: finite(start),
            end: finite(end),
        });
    }
    in_time_order(&mut out);
    (out, own)
}

/// Starts never go backwards, ends never precede starts.
pub(crate) fn in_time_order(words: &mut [AsrWord]) {
    let mut lo = 0.0f64;
    for w in words {
        w.start = w.start.max(lo);
        w.end = w.end.max(w.start);
        lo = w.start;
    }
}

/// A silent run at least this long (seconds) between a word's start and its
/// own first-token mark is a pause the word cannot start in.
pub const PAUSE_SECONDS: f64 = 0.080;
/// 10 ms frames this far below the clip's loudest frame are silence (as
/// `timing::time_words`).
pub const SILENCE_BELOW_PEAK_DB: f64 = 40.0;

/// 10 ms RMS envelope (dBFS, floor -100) of f32 audio in [-1, 1] (the
/// envelope [`starts_after_pauses`] reads).
pub fn envelope_db_f32(audio: &[f32], rate: u32) -> Vec<f64> {
    let hop = ((f64::from(rate) * 0.010).round() as usize).max(1);
    audio
        .chunks(hop)
        .map(|c| {
            let e: f64 = c.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
            let rms = (e / c.len() as f64).sqrt();
            if rms <= 1e-5 {
                -100.0
            } else {
                (20.0 * rms.log10()).max(-100.0)
            }
        })
        .collect()
}

/// A word's span start is the previous token's DTW mark, which after a pause
/// can lie anywhere in the pause (or in the previous sentence's tail). When
/// `[start, own first-token mark)` holds a pause (a silent run of at least
/// [`PAUSE_SECONDS`] on the 10 ms envelope `env_db`, or silence reaching
/// back to the start), the start moves to where the last such pause ends:
/// a word never starts in silence. Only ever later, never past its own mark;
/// order is kept. Pure.
pub fn starts_after_pauses(words: &mut [AsrWord], own_marks: &[f64], env_db: &[f64]) {
    let peak = env_db.iter().copied().fold(-100.0f64, f64::max);
    let thr = peak - SILENCE_BELOW_PEAK_DB;
    let silent = |k: usize| env_db.get(k).is_some_and(|&d| d < thr);
    let min_run = (PAUSE_SECONDS / 0.010).round() as usize;
    for (w, &own) in words.iter_mut().zip(own_marks) {
        let lo = (w.start / 0.010).round().max(0.0) as usize;
        let hi = ((own.max(w.start) / 0.010).round().max(0.0) as usize).min(env_db.len());
        // Scan back from the own mark for the last qualifying silent run.
        let mut k = hi;
        while k > lo {
            if !silent(k - 1) {
                k -= 1;
                continue;
            }
            let run_end = k; // first voiced frame after the run (or hi)
            while k > lo && silent(k - 1) {
                k -= 1;
            }
            if run_end - k >= min_run || k == lo {
                let t = run_end as f64 / 100.0;
                if t > w.start {
                    w.start = t;
                }
                break;
            }
        }
    }
    in_time_order(words);
}

/// Run whisper over 16 kHz mono audio and return one [`AsrWord`] per spoken
/// word (seconds from the clip start, in order).
#[cfg(feature = "asr-local")]
fn recognise(
    audio16k: &[f32],
    model: &Path,
    preset: DtwPreset,
    params: &LocalParams,
) -> Result<Vec<AsrWord>, VoiceError> {
    let segs = recognise_segments(audio16k, model, preset, params)?;
    let (mut words, own) = build_words(&segs, TIMESTAMP_SOURCE);
    starts_after_pauses(&mut words, &own, &envelope_db_f32(audio16k, WHISPER_RATE));
    Ok(words)
}

/// whisper's raw segments for 16 kHz mono audio (diagnostics and the
/// timestamp-source measurement; [`transcribe`] is the cached entry point).
/// Decoding: `params.language`, no translation, temperature 0 without
/// fallback, greedy (`beam_size <= 1`) or beam search, no context carried
/// between windows, token timestamps (DTW with the model's alignment-heads
/// preset when `params.dtw`), one segment per word. whisper.cpp's logging is
/// silenced. Long clips are decoded in whisper's own 30 s windows; times are
/// absolute.
#[cfg(feature = "asr-local")]
pub fn recognise_segments(
    audio16k: &[f32],
    model: &Path,
    preset: DtwPreset,
    params: &LocalParams,
) -> Result<Vec<RawSegment>, VoiceError> {
    use whisper_rs::{
        DtwMode, DtwModelPreset, DtwParameters, FullParams, SamplingStrategy, WhisperContext,
        WhisperContextParameters,
    };
    let err =
        |what: &str, e: whisper_rs::WhisperError| VoiceError::Audio(format!("whisper {what}: {e}"));
    // Route whisper.cpp / ggml logging away from stderr (no log backend: dropped).
    whisper_rs::install_logging_hooks();
    if audio16k.is_empty() {
        return Ok(Vec::new());
    }
    let mut cp = WhisperContextParameters::default();
    cp.use_gpu(true);
    if params.dtw {
        let model_preset = match preset {
            DtwPreset::TinyEn => DtwModelPreset::TinyEn,
            DtwPreset::BaseEn => DtwModelPreset::BaseEn,
            DtwPreset::SmallEn => DtwModelPreset::SmallEn,
        };
        cp.dtw_parameters(DtwParameters {
            mode: DtwMode::ModelPreset { model_preset },
            ..Default::default()
        });
    }
    let ctx = WhisperContext::new_with_params(model, cp).map_err(|e| err("model", e))?;
    let mut state = ctx.create_state().map_err(|e| err("state", e))?;
    let strategy = if params.beam_size <= 1 {
        SamplingStrategy::Greedy { best_of: 1 }
    } else {
        SamplingStrategy::BeamSearch {
            beam_size: i32::try_from(params.beam_size).unwrap_or(i32::MAX),
            patience: -1.0,
        }
    };
    let mut fp = FullParams::new(strategy);
    fp.set_language(Some(&params.language));
    fp.set_detect_language(false);
    fp.set_translate(false);
    fp.set_no_context(true);
    fp.set_temperature(0.0);
    fp.set_temperature_inc(0.0);
    fp.set_token_timestamps(true);
    fp.set_split_on_word(true);
    fp.set_max_len(1);
    fp.set_print_special(false);
    fp.set_print_progress(false);
    fp.set_print_realtime(false);
    fp.set_print_timestamps(false);
    if params.threads > 0 {
        fp.set_n_threads(i32::try_from(params.threads).unwrap_or(i32::MAX));
    }
    state.full(fp, audio16k).map_err(|e| err("decode", e))?;
    let eot = ctx.token_eot();
    let mut out = Vec::new();
    for seg in state.as_iter() {
        let mut tokens = Vec::new();
        for i in 0..seg.n_tokens() {
            let Some(tok) = seg.get_token(i) else {
                continue;
            };
            if tok.token_id() >= eot {
                continue; // special and timestamp tokens
            }
            let data = tok.token_data();
            tokens.push(RawToken {
                text: tok
                    .to_str_lossy()
                    .map(|c| c.into_owned())
                    .unwrap_or_default(),
                t_dtw: data.t_dtw,
                t0: data.t0,
                t1: data.t1,
            });
        }
        out.push(RawSegment {
            text: seg
                .to_str_lossy()
                .map(|c| c.into_owned())
                .unwrap_or_default(),
            t0: seg.start_timestamp(),
            t1: seg.end_timestamp(),
            tokens,
        });
    }
    Ok(out)
}

#[cfg(not(feature = "asr-local"))]
fn recognise(
    _audio16k: &[f32],
    _model: &Path,
    _preset: DtwPreset,
    _params: &LocalParams,
) -> Result<Vec<AsrWord>, VoiceError> {
    Err(VoiceError::Unsupported("local word timing"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_table_is_consistent() {
        assert!(model_spec(DEFAULT_MODEL).is_some());
        for m in MODELS {
            assert_eq!(m.sha256.len(), 64);
            assert!(m.url.ends_with(m.file));
            assert!(m.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        }
        assert_eq!(
            models_root_from(None, Some(PathBuf::from("/h"))),
            PathBuf::from("/h/.cache/motionengine/models")
        );
        assert_eq!(
            models_root_from(Some(PathBuf::from("/m")), Some(PathBuf::from("/h"))),
            PathBuf::from("/m")
        );
    }

    #[test]
    fn cache_key_covers_model_params_and_audio() {
        let root = Path::new("/c");
        let p = LocalParams::default();
        let a = cache_path(root, "m1", &p, b"wav");
        assert_ne!(a, cache_path(root, "m2", &p, b"wav"));
        assert_ne!(a, cache_path(root, "m1", &p, b"wav2"));
        let beam = LocalParams {
            beam_size: 5,
            ..LocalParams::default()
        };
        assert_ne!(a, cache_path(root, "m1", &beam, b"wav"));
        // Threads never change the output, so they never change the key.
        let threads = LocalParams {
            threads: 4,
            ..LocalParams::default()
        };
        assert_eq!(a, cache_path(root, "m1", &threads, b"wav"));
        assert_eq!(
            alignment_label(&MODELS[1], &LocalParams::default()),
            "local:whisper-base.en:beam1"
        );
        // (0.20 A3) CTC refinement changes the key (never a stale whisper-only
        // result) and the label; without it the key is the whisper-only key.
        let ctc = LocalParams {
            ctc: true,
            ..LocalParams::default()
        };
        assert_ne!(a, cache_path(root, "m1", &ctc, b"wav"));
        assert_eq!(
            LocalParams::default().key(),
            format!("lang=en;beam=1;dtw=1;words={WORDS_VERSION}")
        );
        assert!(ctc.key().contains(";ctc=1;ctcalign="), "{}", ctc.key());
        assert_eq!(
            alignment_label(&MODELS[1], &ctc),
            "local:whisper-base.en:beam1+ctc"
        );
        // Old serialised params (no `ctc`) still read, as whisper-only.
        let old: LocalParams =
            serde_json::from_str(r#"{"language":"en","beam_size":1,"dtw":true,"threads":0}"#)
                .unwrap();
        assert!(!old.ctc);
    }

    #[test]
    fn resampling_keeps_level_and_time() {
        // 48 kHz DC at half scale -> 16 kHz DC at half scale.
        let dc: Vec<i16> = vec![16384; 4800];
        let out = resample_16k(&dc, 48_000);
        assert_eq!(out.len(), 1600);
        assert!((out[800] - 0.5).abs() < 1e-3, "{}", out[800]);
        // A 1 kHz tone keeps its frequency: zero crossings every 8 samples at 16 kHz.
        let tone: Vec<i16> = (0..9600)
            .map(|i| {
                (0.5 * (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48_000.0).sin() * 32767.0)
                    as i16
            })
            .collect();
        let out = resample_16k(&tone, 48_000);
        let peak = out[100..3000].iter().cloned().fold(0.0f32, f32::max);
        assert!((peak - 0.5).abs() < 0.03, "{peak}");
        // An onset at 0.1 s stays at 0.1 s.
        let mut step = vec![0i16; 9600];
        for s in step.iter_mut().skip(4800) {
            *s = 16384;
        }
        let out = resample_16k(&step, 48_000);
        let rise = out.iter().position(|&v| v > 0.25).unwrap();
        assert!((1598..=1602).contains(&rise), "{rise}");
        // Identity at 16 kHz.
        assert_eq!(resample_16k(&[0, 16384], 16_000), vec![0.0, 0.5]);
    }

    fn tok(text: &str, t_dtw: i64, t0: i64, t1: i64) -> RawToken {
        RawToken {
            text: text.into(),
            t_dtw,
            t0,
            t1,
        }
    }

    fn seg(text: &str, t0: i64, t1: i64, tokens: Vec<RawToken>) -> RawSegment {
        RawSegment {
            text: text.into(),
            t0,
            t1,
            tokens,
        }
    }

    #[test]
    fn words_merge_pieces_and_take_span_times() {
        let segs = vec![
            seg("", 0, 0, vec![]), // timestamp-only segment
            seg(" In", 0, 18, vec![tok(" In", 12, 0, 18)]),
            seg(
                " 1969,",
                18,
                149,
                vec![tok(" 1969", 160, 18, 131), tok(",", 198, 131, 149)],
            ),
            seg(" about", 149, 170, vec![tok(" about", 230, 149, 170)]),
            // A piece without a leading space continues the previous word.
            seg(" $", 170, 180, vec![tok(" $", 250, 170, 180)]),
            seg("381", 180, 260, vec![tok("381", 300, 180, 260)]),
            seg(" -", 260, 262, vec![tok(" -", 305, 260, 262)]), // punctuation only
            seg(" sun", 262, 280, vec![tok(" sun", 320, 262, 280)]),
            seg(
                "-lit.",
                280,
                300,
                vec![
                    tok("-", 330, 280, 285),
                    tok("lit", 350, 285, 300),
                    tok(".", 380, 300, 300),
                ],
            ),
        ];
        let w = words_from_segments(&segs, TimestampSource::DtwSpan);
        let text: Vec<&str> = w.iter().map(|w| w.word.as_str()).collect();
        assert_eq!(text, vec!["In", "1969,", "about", "$381", "sun-lit."]);
        // Span: start = previous text token's mark (the first word: its
        // segment start), end = the mark of the last alphanumeric token.
        let times: Vec<(f64, f64)> = w.iter().map(|w| (w.start, w.end)).collect();
        assert_eq!(
            times,
            vec![
                (0.0, 0.12),
                (0.12, 1.60),
                (1.98, 2.30),
                (2.30, 3.00),
                (3.05, 3.50)
            ]
        );
        let own = words_from_segments(&segs, TimestampSource::DtwOwn);
        assert_eq!((own[1].start, own[1].end), (1.60, 1.98));
        let heuristic = words_from_segments(&segs, TimestampSource::Segment);
        assert_eq!((heuristic[3].start, heuristic[3].end), (1.70, 2.60));
    }

    #[test]
    fn missing_marks_fall_back_and_times_stay_ordered() {
        let segs = vec![
            seg(" a", 100, 120, vec![tok(" a", -1, 100, 120)]),
            seg(" b", 120, 150, vec![tok(" b", 200, 120, 150)]),
            // A mark earlier than the previous word: clamped, never backwards.
            seg(" c", 150, 160, vec![tok(" c", 90, 150, 160)]),
            seg(" d", 160, 170, vec![tok(" d", -1, 160, 170)]),
            seg("   ", 170, 170, vec![]),
        ];
        let w = words_from_segments(&segs, TimestampSource::DtwSpan);
        assert_eq!(w.len(), 4);
        // a: no previous mark -> segment start; no own mark -> segment end.
        assert_eq!((w[0].start, w[0].end), (1.0, 1.2));
        // b: the previous token had no mark -> its segment start.
        assert_eq!((w[1].start, w[1].end), (1.2, 2.0));
        assert!(w.windows(2).all(|p| p[0].start <= p[1].start));
        assert!(w.iter().all(|w| w.end >= w.start && w.start >= 0.0));
        assert!(words_from_segments(&[], TimestampSource::DtwSpan).is_empty());
        // A degenerate DTW path (every mark at the clip end, far outside the
        // segments) is ignored: the segment times stand in.
        let degenerate = vec![
            seg(" Every", 0, 40, vec![tok(" Every", 1968, 0, 40)]),
            seg(" year", 40, 87, vec![tok(" year", 1968, 40, 87)]),
        ];
        let w = words_from_segments(&degenerate, TimestampSource::DtwSpan);
        assert_eq!((w[0].start, w[0].end), (0.0, 0.4));
        assert_eq!((w[1].start, w[1].end), (0.4, 0.87));
    }

    #[test]
    fn starts_move_past_pauses_only() {
        // 1 s envelope: speech, a 300 ms pause at 0.30-0.60 s, speech with a
        // 30 ms closure at 0.80-0.83 s.
        let mut env = vec![-20.0f64; 100];
        for f in env.iter_mut().take(60).skip(30) {
            *f = -100.0;
        }
        for f in env.iter_mut().take(83).skip(80) {
            *f = -75.0;
        }
        let one = |start: f64, own: f64| {
            let mut w = vec![AsrWord {
                word: "w".into(),
                start,
                end: own + 0.1,
            }];
            starts_after_pauses(&mut w, &[own], &env);
            w[0].start
        };
        // Start in the previous sentence's tail, pause before the own mark.
        assert_eq!(one(0.25, 0.70), 0.60);
        // Start inside the pause (silence reaching back to the start).
        assert_eq!(one(0.35, 0.62), 0.60);
        // No pause: unchanged; a short closure is not a pause.
        assert_eq!(one(0.65, 0.78), 0.65);
        assert_eq!(one(0.76, 0.90), 0.76);
        // Never past the own mark, never earlier.
        assert_eq!(one(0.35, 0.40), 0.40);
        assert_eq!(one(0.62, 0.50), 0.62);
        // f32 envelope: 10 ms frames at 16 kHz, silence at the floor.
        let audio: Vec<f32> = (0..1600).map(|i| if i < 800 { 0.0 } else { 0.5 }).collect();
        let e = envelope_db_f32(&audio, WHISPER_RATE);
        assert_eq!(e.len(), 10);
        assert_eq!(e[0], -100.0);
        assert!((e[9] - 20.0 * 0.5f64.log10()).abs() < 1e-6);
    }

    #[test]
    fn percent_becomes_its_own_word() {
        let segs = vec![
            seg(" Only", 0, 30, vec![tok(" Only", 40, 0, 30)]),
            seg(
                " 23%.",
                30,
                120,
                vec![
                    tok(" 23", 90, 30, 80),
                    tok("%", 140, 80, 110),
                    tok(".", 150, 110, 120),
                ],
            ),
        ];
        let w = words_from_segments(&segs, TimestampSource::DtwSpan);
        let got: Vec<(&str, f64, f64)> = w
            .iter()
            .map(|w| (w.word.as_str(), w.start, w.end))
            .collect();
        assert_eq!(
            got,
            vec![("Only", 0.0, 0.4), ("23", 0.4, 0.9), ("percent", 0.9, 1.4)]
        );
        // "%" inside a word that is not a separate token stays as written.
        let one = vec![seg(" 5%", 0, 10, vec![tok(" 5%", 5, 0, 10)])];
        assert_eq!(
            words_from_segments(&one, TimestampSource::DtwSpan)[0].word,
            "5%"
        );
    }

    #[test]
    fn verify_rejects_a_wrong_file_and_a_miss_needs_the_model() {
        let dir = std::env::temp_dir().join(format!("me_asr_local_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let spec = &MODELS[0];
        let path = model_path(&dir, spec);
        std::fs::write(&path, b"not a model").unwrap();
        assert!(matches!(
            verify(&path, spec),
            Err(VoiceError::Checksum { .. })
        ));
        assert_eq!(installed(&dir, spec), Some(11));
        let missing = &MODELS[1];
        assert!(installed(&dir, missing).is_none());
        let wav = crate::wav::encode(&[0i16; 1600]);
        assert!(matches!(
            transcribe(
                &wav,
                missing,
                &dir,
                &LocalParams::default(),
                &dir.join("cache")
            ),
            Err(VoiceError::ModelMissing(_))
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
