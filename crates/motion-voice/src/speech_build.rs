//! `build_speech`: one cached synthesis per beat statement, trimmed and laid
//! out on one timeline → voice-over WAV + [`SpeechMap`] with sample-exact
//! sentence times.

use std::path::Path;

use motion_core::compiler::taste::TemperamentKind;
use motion_core::speech::SPEECH_VERSION;

use crate::cache::VoiceCache;
use crate::fixture::strip_punct;
use crate::{
    wav, SpeechMap, SpeechSentence, SpeechWord, TtsModel, VoiceChoice, VoiceError, VoiceProvider,
    SAMPLE_RATE,
};

/// Clips are trimmed where |sample| first/last exceeds −45 dBFS
/// (32768 · 10^(−45/20) ≈ 184).
pub const TRIM_THRESHOLD: i16 = 184;

/// Silences laid around and between sentences (seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct Pauses {
    /// Silence before the first sentence.
    pub lead_in: f64,
    /// Silence between one sentence's end and the next one's start.
    pub gap: f64,
    /// Silence after the last sentence.
    pub tail: f64,
    /// Minimum distance between consecutive sentence STARTS; the gap after a
    /// short sentence is lengthened to honour it (beats retimed to sentences
    /// need room for ENTER/READ).
    pub min_spacing: f64,
    /// Per-boundary minimum start spacing (`[i]` = sentence i → i+1), e.g.
    /// 0.8 × the planned visual beat spacing; combined with `min_spacing` by max.
    pub min_spacings: Vec<f64>,
}

impl Default for Pauses {
    fn default() -> Self {
        pauses_for(TemperamentKind::Editorial)
    }
}

/// Restrained → 0.80 s gaps, balanced (editorial/precise) → 0.60 s,
/// energetic → 0.42 s; lead-in 0.35 s, tail 0.9 s, min spacing 2.6 s.
pub fn pauses_for(kind: TemperamentKind) -> Pauses {
    let gap = match kind {
        TemperamentKind::Restrained => 0.80,
        TemperamentKind::Editorial | TemperamentKind::Precise => 0.60,
        TemperamentKind::Energetic => 0.42,
    };
    Pauses {
        lead_in: 0.35,
        gap,
        tail: 0.9,
        min_spacing: 2.6,
        min_spacings: Vec::new(),
    }
}

/// Word-timing hook: `(clip samples after trimming, sample rate,
/// the statement's words)` → words with times in seconds RELATIVE TO THE CLIP
/// START. Called once per sentence, only when the provider returned no words.
pub type WordTimer = fn(&[i16], u32, &[String]) -> Vec<SpeechWord>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BuildStats {
    /// Provider calls made (0 on a full cache hit).
    pub calls: usize,
    pub cache_hits: usize,
    pub duration: f64,
    pub sentences: usize,
}

/// Statement words as spoken-text tokens (punctuation stripped, no empties).
pub fn statement_words(statement: &str) -> Vec<String> {
    statement
        .split_whitespace()
        .map(strip_punct)
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Half-open sample range of the audible part of `samples` (below
/// [`TRIM_THRESHOLD`] counts as silence). `None` when everything is silent.
pub fn audible_range(samples: &[i16]) -> Option<(usize, usize)> {
    let loud = |s: &i16| s.unsigned_abs() > TRIM_THRESHOLD as u16;
    let first = samples.iter().position(loud)?;
    let last = samples.iter().rposition(loud)?;
    Some((first, last + 1))
}

fn secs(samples: usize) -> f64 {
    samples as f64 / SAMPLE_RATE as f64
}

fn samples_of(seconds: f64) -> usize {
    (seconds.max(0.0) * SAMPLE_RATE as f64).round() as usize
}

/// Build the voice-over for `statements` (beat order) into `out_wav`.
///
/// `offline` makes a cache miss an error. `word_timer` fills `words` when the
/// provider returns none (`None` = leave them empty).
#[allow(clippy::too_many_arguments)]
pub fn build_speech(
    statements: &[String],
    model: TtsModel,
    voice: &VoiceChoice,
    provider: &dyn VoiceProvider,
    cache: &VoiceCache,
    pauses: &Pauses,
    out_wav: &Path,
    offline: bool,
    word_timer: Option<WordTimer>,
) -> Result<(SpeechMap, BuildStats), VoiceError> {
    build_speech_scripted(
        statements, statements, model, voice, provider, cache, pauses, out_wav, offline, word_timer,
    )
}

/// (0.10 Q) `build_speech` where the voice reads `tts[i]` (e.g. a
/// pronunciation-adjusted script) while words and captions follow
/// `statements[i]` (the author's text). Both must have the same length.
#[allow(clippy::too_many_arguments)]
pub fn build_speech_scripted(
    statements: &[String],
    tts: &[String],
    model: TtsModel,
    voice: &VoiceChoice,
    provider: &dyn VoiceProvider,
    cache: &VoiceCache,
    pauses: &Pauses,
    out_wav: &Path,
    offline: bool,
    word_timer: Option<WordTimer>,
) -> Result<(SpeechMap, BuildStats), VoiceError> {
    if tts.len() != statements.len() {
        return Err(VoiceError::Audio(
            "tts script and statements differ in length".into(),
        ));
    }
    if statements.is_empty() {
        return Err(VoiceError::Audio("no statements to speak".into()));
    }
    let calls0 = cache.calls();
    let hits0 = cache.hits();

    let mut pcm: Vec<i16> = vec![0; samples_of(pauses.lead_in)];
    let mut sentences = Vec::with_capacity(statements.len());
    let mut words: Vec<SpeechWord> = Vec::new();
    let spacing = samples_of(pauses.min_spacing);
    let gap = samples_of(pauses.gap);
    let mut prev_start: Option<usize> = None;

    for (beat, statement) in statements.iter().enumerate() {
        if statement.trim().is_empty() {
            // Silent visual beat (empty narration): hold, no words.
            if let Some(ps) = prev_start {
                let want = pauses
                    .min_spacing
                    .max(pauses.min_spacings.get(beat - 1).copied().unwrap_or(0.0));
                let needed = (ps + samples_of(want)).saturating_sub(pcm.len());
                pcm.resize(pcm.len() + needed.max(gap), 0);
            }
            let start = pcm.len();
            pcm.resize(start + samples_of(SILENT_BEAT_SECONDS), 0);
            prev_start = Some(start);
            sentences.push(SpeechSentence {
                beat,
                start: secs(start),
                end: secs(start),
            });
            continue;
        }
        let syn = cache.get_or_synthesize(provider, &tts[beat], voice, model, offline)?;
        let clip = wav::decode(&syn.wav)?;
        if clip.sample_rate != SAMPLE_RATE {
            return Err(VoiceError::Audio(format!(
                "provider clip is {} Hz, expected {SAMPLE_RATE}",
                clip.sample_rate
            )));
        }
        let (lo, hi) = audible_range(&clip.samples).unwrap_or((0, 0));
        let body = &clip.samples[lo..hi];

        // Gap after the previous sentence: at least `gap`, and long enough to
        // keep sentence STARTS `min_spacing` apart.
        if let Some(ps) = prev_start {
            let extra = pauses
                .min_spacings
                .get(beat.saturating_sub(1))
                .map(|&s| samples_of(s))
                .unwrap_or(0);
            let min_len = (ps + spacing.max(extra)).saturating_sub(pcm.len());
            let pad = gap.max(min_len);
            pcm.resize(pcm.len() + pad, 0);
        }
        let start = pcm.len();
        pcm.extend_from_slice(body);
        let end = pcm.len();
        prev_start = Some(start);

        sentences.push(SpeechSentence {
            beat,
            start: secs(start),
            end: secs(end),
        });

        let clip_start = secs(start);
        let clip_len = secs(body.len());
        match &syn.words {
            Some(provider_words) => {
                let trimmed = secs(lo);
                for w in provider_words {
                    let s = (w.start - trimmed).clamp(0.0, clip_len);
                    let e = (w.end - trimmed).clamp(s, clip_len);
                    words.push(SpeechWord {
                        text: w.text.clone(),
                        start: clip_start + s,
                        end: clip_start + e,
                        confidence: 1.0,
                    });
                }
            }
            None => {
                if let Some(timer) = word_timer {
                    for w in timer(body, SAMPLE_RATE, &statement_words(statement)) {
                        words.push(SpeechWord {
                            start: clip_start + w.start,
                            end: clip_start + w.end,
                            ..w
                        });
                    }
                }
            }
        }
    }
    pcm.resize(pcm.len() + samples_of(pauses.tail), 0);

    if let Some(dir) = out_wav.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(out_wav, wav::encode(&pcm))?;

    let duration = secs(pcm.len());
    let audio = out_wav
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let map = SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio,
        sample_rate: SAMPLE_RATE,
        duration,
        provider: provider.id().to_string(),
        model: model.model_id().to_string(),
        voice: voice.voice.clone(),
        words,
        sentences,
    };
    let stats = BuildStats {
        calls: cache.calls() - calls0,
        cache_hits: cache.hits() - hits0,
        duration,
        sentences: statements.len(),
    };
    Ok((map, stats))
}

/// (0.10 Q) Longest extra silence the single-take builder may add after a
/// sentence to honour visual spacing (beyond the narrator's own pause).
pub const MAX_EXTRA_PAUSE: f64 = 1.0;
/// Natural pause between sentences is clamped to this range when re-laid.
pub const NATURAL_PAUSE: (f64, f64) = (0.22, 0.65);

/// Ensure a spoken line ends like a sentence (so the narrator's intonation
/// closes it and pauses).
pub fn as_sentence(line: &str) -> String {
    let t = line.trim();
    if t.ends_with(['.', '?', '!', '…']) {
        t.to_string()
    } else {
        format!("{t}.")
    }
}

/// The single-take script: one paragraph of the non-empty beat lines. A line
/// is closed with a full stop unless it already ends in punctuation or is a
/// fragment that the next line continues (it ends in `,;:—-` or the next line
/// starts in lowercase), so a sentence may span several beats and still be
/// read as one sentence (0.10 Q, many visual beats per spoken sentence).
pub fn single_take_script(lines: &[String]) -> String {
    let spoken: Vec<&str> = lines
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    spoken
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let continues = spoken
                .get(i + 1)
                .and_then(|n| n.chars().next())
                .is_some_and(|c| c.is_lowercase());
            if l.ends_with(['.', '?', '!', '…', ',', ';', ':', '—', '-']) || continues {
                l.to_string()
            } else {
                as_sentence(l)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// (0.10 Q) Finds the N−1 cut times (seconds within the trimmed take) between
/// the spoken lines, e.g. from recognised word timestamps; `None` = unknown
/// (the builder then cuts at the narrator's pauses).
pub type BoundaryFinder<'a> = &'a dyn Fn(&[i16]) -> Option<Vec<f64>>;

/// Silence a beat with empty narration holds (seconds, before spacing rules).
pub const SILENT_BEAT_SECONDS: f64 = 1.2;

/// Pause candidates in `samples` (10 ms frames): runs at least 120 ms below
/// the take's peak − 35 dB, as (start_s, end_s), inside the audible span.
fn pause_candidates(samples: &[i16]) -> Vec<(f64, f64)> {
    let env = crate::timing::envelope_db(samples, SAMPLE_RATE);
    let peak = env.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let floor = peak - 35.0;
    let mut out = Vec::new();
    let mut run: Option<usize> = None;
    for (i, &db) in env.iter().enumerate() {
        let quiet = db < floor;
        match (quiet, run) {
            (true, None) => run = Some(i),
            (false, Some(r)) => {
                if i - r >= 12 {
                    out.push((r as f64 * 0.01, i as f64 * 0.01));
                }
                run = None;
            }
            _ => {}
        }
    }
    out
}

/// Choose `n − 1` sentence boundaries among pause candidates: increasing,
/// close to the syllable-weighted expectation, preferring longer pauses.
/// `None` when the take has too few pauses or the best fit is far off.
fn choose_boundaries(cands: &[(f64, f64)], expected: &[f64], total: f64) -> Option<Vec<usize>> {
    let k = expected.len();
    if k == 0 {
        return Some(Vec::new());
    }
    if cands.len() < k || total <= 0.0 {
        return None;
    }
    let cost = |c: usize, e: usize| {
        let (a, b) = cands[c];
        let mid = (a + b) / 2.0;
        10.0 * (mid - expected[e]).abs() / total - 1.5 * (b - a).min(0.6) / 0.6
    };
    // dp[e][c]: best cost with boundary e at candidate c.
    let m = cands.len();
    let inf = f64::INFINITY;
    let mut dp = vec![vec![inf; m]; k];
    let mut from = vec![vec![usize::MAX; m]; k];
    for (c, slot) in dp[0].iter_mut().enumerate() {
        *slot = cost(c, 0);
    }
    for e in 1..k {
        let mut best = inf;
        let mut arg = usize::MAX;
        for c in 0..m {
            // best over candidates strictly before c (running minimum)
            if c >= 1 && dp[e - 1][c - 1] < best {
                best = dp[e - 1][c - 1];
                arg = c - 1;
            }
            if best < inf {
                dp[e][c] = best + cost(c, e);
                from[e][c] = arg;
            }
        }
    }
    let (mut c, total_cost) = dp[k - 1]
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, v)| (i, *v))?;
    if !total_cost.is_finite() {
        return None;
    }
    let mut picks = vec![0; k];
    for e in (0..k).rev() {
        picks[e] = c;
        if e > 0 {
            c = from[e][c];
        }
    }
    // Reject a fit that lands any boundary far from its expectation.
    let worst = picks
        .iter()
        .zip(expected)
        .map(|(&c, &x)| ((cands[c].0 + cands[c].1) / 2.0 - x).abs() / total)
        .fold(0.0, f64::max);
    (worst <= 0.22).then_some(picks)
}

/// Cut times (seconds) at the quietest 80 ms near each expected boundary:
/// searched within ±35 % of an average line (0.25–1.0 s), strictly
/// increasing and at least 0.3 s from each other and the take's ends. Never
/// fails, so a single take is always split, never re-synthesised.
fn quiet_cuts(samples: &[i16], expected: &[f64], total: f64) -> Vec<f64> {
    let env = crate::timing::envelope_db(samples, SAMPLE_RATE);
    let frames = env.len();
    let win = 8usize; // 80 ms of 10 ms frames
    let line = total / (expected.len() + 1) as f64;
    let reach = (0.35 * line).clamp(0.25, 1.0);
    let mut out: Vec<f64> = Vec::with_capacity(expected.len());
    for (i, &e) in expected.iter().enumerate() {
        let floor = out.last().map_or(0.3, |p| p + 0.3);
        let ceil = total - 0.3 * (expected.len() - i) as f64;
        let lo = (e - reach).max(floor).min(ceil);
        let hi = (e + reach).min(ceil).max(lo);
        let (f_lo, f_hi) = ((lo * 100.0) as usize, (hi * 100.0) as usize);
        let mut best = (f64::INFINITY, e.clamp(lo, hi));
        let mut f = f_lo;
        while f + win <= frames.min(f_hi + win) {
            let mean = env[f..f + win].iter().sum::<f64>() / win as f64;
            if mean < best.0 {
                best = (mean, (f as f64 + win as f64 / 2.0) * 0.01);
            }
            f += 1;
        }
        out.push(best.1.clamp(lo, hi));
    }
    out
}

/// Silence window [`snap_cuts_to_silence`] looks for (seconds).
pub const SILENCE_WINDOW: f64 = 0.040;

/// (0.20 A5) Word-level cuts that never clip a syllable: for each
/// `(previous word end, next word start)` gap (seconds within `samples`), the
/// centre of the quietest [`SILENCE_WINDOW`] lying inside the gap, on the
/// 10 ms envelope of [`crate::timing::envelope_db`] (ties: the window nearest
/// the gap midpoint). A gap shorter than the window, or one outside the
/// audio, cuts at its midpoint. Pure.
pub fn snap_cuts_to_silence(samples: &[i16], sample_rate: u32, gaps: &[(f64, f64)]) -> Vec<f64> {
    let env = crate::timing::envelope_db(samples, sample_rate);
    let hop = crate::timing::HOP_SECONDS;
    let win = (SILENCE_WINDOW / hop).round().max(1.0) as usize;
    gaps.iter()
        .map(|&(a, b)| {
            let mid = (a + b) / 2.0;
            if !(a.is_finite() && b.is_finite()) || b - a < SILENCE_WINDOW - 1e-9 {
                return mid;
            }
            // Frames f..f+win lying inside [a, b].
            let f_lo = (a.max(0.0) / hop - 1e-9).ceil() as usize;
            let f_end = ((b / hop + 1e-9).floor().max(0.0) as usize).min(env.len());
            if f_end < f_lo + win {
                return mid;
            }
            let mut best = (f64::INFINITY, mid);
            for f in f_lo..=f_end - win {
                let level = env[f..f + win].iter().sum::<f64>() / win as f64;
                let centre = (f as f64 + win as f64 / 2.0) * hop;
                let quieter = level < best.0 - 1e-9;
                let tie_nearer =
                    (level - best.0).abs() <= 1e-9 && (centre - mid).abs() < (best.1 - mid).abs();
                if quieter || tie_nearer {
                    best = (level, centre);
                }
            }
            best.1
        })
        .collect()
}

/// (0.10 Q) Single-take voice-over: the whole script is synthesised in ONE
/// call (one continuous read, so intonation flows across sentences instead of
/// resetting per line), then split at the narrator's own pauses into one
/// sentence per beat and re-laid with natural pauses (extended up to
/// [`MAX_EXTRA_PAUSE`] to honour `pauses.min_spacing(s)`). When the take has
/// too few clear pauses it is cut at the quietest moments near the expected
/// boundaries ([`quiet_cuts`]); it is never re-synthesised per sentence.
#[allow(clippy::too_many_arguments)]
pub fn build_speech_single_take(
    lines: &[String],
    model: TtsModel,
    voice: &VoiceChoice,
    provider: &dyn VoiceProvider,
    cache: &VoiceCache,
    pauses: &Pauses,
    out_wav: &Path,
    offline: bool,
    word_timer: Option<WordTimer>,
) -> Result<(SpeechMap, BuildStats), VoiceError> {
    build_speech_single_take_scripted(
        lines, lines, model, voice, provider, cache, pauses, out_wav, offline, word_timer,
    )
}

/// (0.10 Q) Single take where the voice reads `tts` (mixed script) while
/// words, syllable weights and captions follow `lines`.
#[allow(clippy::too_many_arguments)]
pub fn build_speech_single_take_scripted(
    lines: &[String],
    tts: &[String],
    model: TtsModel,
    voice: &VoiceChoice,
    provider: &dyn VoiceProvider,
    cache: &VoiceCache,
    pauses: &Pauses,
    out_wav: &Path,
    offline: bool,
    word_timer: Option<WordTimer>,
) -> Result<(SpeechMap, BuildStats), VoiceError> {
    build_speech_single_take_with(
        lines, tts, model, voice, provider, cache, pauses, out_wav, offline, word_timer, None,
    )
}

/// Single take with an optional [`BoundaryFinder`] (word-level cuts).
#[allow(clippy::too_many_arguments)]
pub fn build_speech_single_take_with(
    lines: &[String],
    tts: &[String],
    model: TtsModel,
    voice: &VoiceChoice,
    provider: &dyn VoiceProvider,
    cache: &VoiceCache,
    pauses: &Pauses,
    out_wav: &Path,
    offline: bool,
    word_timer: Option<WordTimer>,
    finder: Option<BoundaryFinder>,
) -> Result<(SpeechMap, BuildStats), VoiceError> {
    if lines.is_empty() {
        return Err(VoiceError::Audio("no statements to speak".into()));
    }
    if tts.len() != lines.len() {
        return Err(VoiceError::Audio(
            "tts script and lines differ in length".into(),
        ));
    }
    let calls0 = cache.calls();
    let hits0 = cache.hits();
    let script = single_take_script(tts);
    let syn = cache.get_or_synthesize(provider, &script, voice, model, offline)?;
    let take = wav::decode(&syn.wav)?;
    if take.sample_rate != SAMPLE_RATE {
        return Err(VoiceError::Audio(format!(
            "provider take is {} Hz, expected {SAMPLE_RATE}",
            take.sample_rate
        )));
    }
    let (lo, hi) =
        audible_range(&take.samples).ok_or_else(|| VoiceError::Audio("silent take".into()))?;
    let body = &take.samples[lo..hi];
    let total = secs(body.len());

    // Only non-empty lines are spoken; empty narration is a silent beat.
    let spoken_idx: Vec<usize> = (0..lines.len())
        .filter(|&i| !lines[i].trim().is_empty())
        .collect();
    if spoken_idx.is_empty() {
        return Err(VoiceError::Audio("no statements to speak".into()));
    }
    let spoken_lines: Vec<String> = spoken_idx.iter().map(|&i| lines[i].clone()).collect();
    // Expected boundaries from syllable weights.
    let weights: Vec<f64> = spoken_lines
        .iter()
        .map(|l| {
            statement_words(l)
                .iter()
                .map(|w| crate::timing::estimate_syllables(w) as f64)
                .sum::<f64>()
                .max(1.0)
        })
        .collect();
    let sum: f64 = weights.iter().sum();
    let mut acc = 0.0;
    let expected: Vec<f64> = weights[..weights.len() - 1]
        .iter()
        .map(|w| {
            acc += w;
            acc / sum * total
        })
        .collect();
    // Cut points: word-level (finder) first, the narrator's pauses second.
    let mut cuts = vec![0usize];
    let mut natural = Vec::new();
    let word_cuts = finder
        .and_then(|f| f(body))
        .filter(|c| c.len() == expected.len() && c.windows(2).all(|w| w[0] < w[1]));
    match word_cuts {
        Some(times) => {
            for t in times {
                cuts.push(samples_of(t).min(body.len()));
                natural.push(NATURAL_PAUSE.0);
            }
        }
        None => {
            let cands = pause_candidates(body);
            match choose_boundaries(&cands, &expected, total) {
                Some(picks) => {
                    for &c in &picks {
                        let (a, b) = cands[c];
                        cuts.push(samples_of((a + b) / 2.0).min(body.len()));
                        natural.push((b - a).clamp(NATURAL_PAUSE.0, NATURAL_PAUSE.1));
                    }
                }
                // Too few clear pauses: cut at the quietest moment near each
                // expected boundary. The take is never re-synthesised, so the
                // whole voice-over stays one voice and one read.
                None => {
                    for t in quiet_cuts(body, &expected, total) {
                        cuts.push(samples_of(t).min(body.len()));
                        natural.push(NATURAL_PAUSE.0);
                    }
                }
            }
        }
    }
    cuts.push(body.len());

    let mut pcm: Vec<i16> = vec![0; samples_of(pauses.lead_in)];
    let mut sentences = Vec::with_capacity(lines.len());
    let mut words: Vec<SpeechWord> = Vec::new();
    let mut prev_start: Option<usize> = None;
    let mut seg_no = 0usize;
    for (beat, line) in lines.iter().enumerate() {
        let want = |b: usize| {
            pauses
                .min_spacing
                .max(pauses.min_spacings.get(b).copied().unwrap_or(0.0))
        };
        if line.trim().is_empty() {
            // Silent visual beat: hold its share of the timeline, no words.
            if let Some(ps) = prev_start {
                let needed = (ps + samples_of(want(beat - 1))).saturating_sub(pcm.len());
                pcm.resize(pcm.len() + needed.max(samples_of(NATURAL_PAUSE.0)), 0);
            }
            let start = pcm.len();
            pcm.resize(start + samples_of(SILENT_BEAT_SECONDS), 0);
            prev_start = Some(start);
            sentences.push(SpeechSentence {
                beat,
                start: secs(start),
                end: secs(start),
            });
            continue;
        }
        let seg = &body[cuts[seg_no]..cuts[seg_no + 1]];
        let (a, b) = audible_range(seg).unwrap_or((0, seg.len()));
        let clip = &seg[a..b];
        if let Some(ps) = prev_start {
            let gap = samples_of(if seg_no > 0 {
                natural[seg_no - 1]
            } else {
                NATURAL_PAUSE.0
            });
            let needed = (ps + samples_of(want(beat - 1))).saturating_sub(pcm.len());
            let pad = needed.clamp(gap, gap + samples_of(MAX_EXTRA_PAUSE));
            pcm.resize(pcm.len() + pad, 0);
        }
        seg_no += 1;
        let start = pcm.len();
        pcm.extend_from_slice(clip);
        let end = pcm.len();
        prev_start = Some(start);
        sentences.push(SpeechSentence {
            beat,
            start: secs(start),
            end: secs(end),
        });
        if let Some(timer) = word_timer {
            for w in timer(clip, SAMPLE_RATE, &statement_words(line)) {
                words.push(SpeechWord {
                    start: secs(start) + w.start,
                    end: secs(start) + w.end,
                    ..w
                });
            }
        }
    }
    pcm.resize(pcm.len() + samples_of(pauses.tail), 0);

    if let Some(dir) = out_wav.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    std::fs::write(out_wav, wav::encode(&pcm))?;
    let duration = secs(pcm.len());
    let audio = out_wav
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let map = SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio,
        sample_rate: SAMPLE_RATE,
        duration,
        provider: provider.id().to_string(),
        model: model.model_id().to_string(),
        voice: voice.voice.clone(),
        words,
        sentences,
    };
    let stats = BuildStats {
        calls: cache.calls() - calls0,
        cache_hits: cache.hits() - hits0,
        duration,
        sentences: lines.len(),
    };
    Ok((map, stats))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::testutil::TempDir;
    use crate::fixture::{FixtureProvider, LEAD_SILENCE};

    fn voice() -> VoiceChoice {
        VoiceChoice {
            voice: "fx".into(),
            say_rate: None,
        }
    }

    fn statements() -> Vec<String> {
        vec![
            "Orders ship in three days.".to_string(),
            "Now they ship today!".to_string(),
            "That changes everything for every customer we serve.".to_string(),
        ]
    }

    fn build(
        dir: &Path,
        provider: &FixtureProvider,
        pauses: &Pauses,
        name: &str,
    ) -> (SpeechMap, BuildStats) {
        let cache = VoiceCache::new(dir.join("cache"));
        build_speech(
            &statements(),
            TtsModel::Say,
            &voice(),
            provider,
            &cache,
            pauses,
            &dir.join(name),
            false,
            None,
        )
        .unwrap()
    }

    fn no_spacing() -> Pauses {
        Pauses {
            min_spacing: 0.0,
            min_spacings: Vec::new(),
            ..pauses_for(TemperamentKind::Editorial)
        }
    }

    #[test]
    fn pause_table() {
        assert_eq!(pauses_for(TemperamentKind::Restrained).gap, 0.80);
        assert_eq!(pauses_for(TemperamentKind::Editorial).gap, 0.60);
        assert_eq!(pauses_for(TemperamentKind::Precise).gap, 0.60);
        assert_eq!(pauses_for(TemperamentKind::Energetic).gap, 0.42);
        let p = pauses_for(TemperamentKind::Energetic);
        assert_eq!((p.lead_in, p.tail, p.min_spacing), (0.35, 0.9, 2.6));
    }

    #[test]
    fn sentence_times_are_exact_and_monotonic() {
        let dir = TempDir::new("build");
        let pauses = no_spacing();
        let (map, stats) = build(dir.path(), &FixtureProvider::new(false), &pauses, "v.wav");
        assert_eq!(stats.calls, 3);
        assert_eq!(map.sentences.len(), 3);
        assert_eq!(map.audio, "v.wav");
        assert_eq!(map.provider, "fixture");
        assert!(map.words.is_empty(), "no provider words, no timer");

        let w = wav::decode(&std::fs::read(dir.path().join("v.wav")).unwrap()).unwrap();
        assert_eq!(w.duration(), map.duration);

        // Sentence 0 starts exactly at the lead-in; the audible range of the
        // written WAV matches the sentence range sample for sample.
        assert_eq!(map.sentences[0].start, 0.35);
        let mut prev_end = 0.0;
        for (i, s) in map.sentences.iter().enumerate() {
            assert_eq!(s.beat, i);
            assert!(s.start > prev_end - 1e-12 && s.end > s.start);
            let a = (s.start * 48_000.0).round() as usize;
            let b = (s.end * 48_000.0).round() as usize;
            let (lo, hi) = audible_range(&w.samples[a..b]).unwrap();
            assert_eq!((lo, hi), (0, b - a), "sentence {i} not tight");
            // Silence (gap) between this sentence's end and the next start.
            if let Some(next) = map.sentences.get(i + 1) {
                let gap = next.start - s.end;
                assert!((gap - 0.60).abs() < 1e-4, "gap {gap}");
                assert!(w.samples[b..(next.start * 48_000.0).round() as usize]
                    .iter()
                    .all(|v| *v == 0));
            }
            prev_end = s.end;
        }
        let tail = map.duration - map.sentences[2].end;
        assert!((tail - 0.9).abs() < 1e-4, "tail {tail}");
    }

    #[test]
    fn leading_silence_of_the_clip_is_trimmed() {
        // The fixture clip begins with 120 ms of silence; the first sentence
        // must still start at lead_in + 0, not lead_in + 0.12.
        let dir = TempDir::new("trim");
        let (map, _) = build(
            dir.path(),
            &FixtureProvider::new(false),
            &no_spacing(),
            "v.wav",
        );
        assert_eq!(map.sentences[0].start, 0.35);
        let raw_len = FixtureProvider::new(false).render(&statements()[0]).0.len();
        let expect = (raw_len as f64 - LEAD_SILENCE * 48_000.0) / 48_000.0;
        assert!((map.sentences[0].end - map.sentences[0].start - expect).abs() < 0.05);
    }

    #[test]
    fn min_spacing_lengthens_the_gap_after_short_sentences() {
        let dir = TempDir::new("spacing");
        let pauses = pauses_for(TemperamentKind::Energetic); // gap 0.42, spacing 2.6
        let (map, _) = build(dir.path(), &FixtureProvider::new(false), &pauses, "v.wav");
        for pair in map.sentences.windows(2) {
            let d = pair[1].start - pair[0].start;
            assert!(d >= 2.6 - 1e-9, "spacing {d}");
            // The gap is never shorter than the base gap.
            assert!(pair[1].start - pair[0].end >= 0.42 - 1e-9);
        }
        // Statement 0 ("Orders ship in three days.") is short: its gap was
        // stretched so the next start is exactly 2.6 s later.
        let d0 = map.sentences[1].start - map.sentences[0].start;
        assert!((d0 - 2.6).abs() < 1e-4, "{d0}");
        // A long sentence keeps just the base gap when it already exceeds the spacing.
        let long = pauses_for(TemperamentKind::Energetic);
        let (m2, _) = build(
            dir.path(),
            &FixtureProvider::new(false),
            &Pauses {
                min_spacing: 0.5,
                min_spacings: Vec::new(),
                ..long
            },
            "v2.wav",
        );
        let g = m2.sentences[1].start - m2.sentences[0].end;
        assert!((g - 0.42).abs() < 1e-4, "{g}");
    }

    #[test]
    fn provider_words_are_offset_onto_the_timeline() {
        let dir = TempDir::new("words");
        let (map, _) = build(
            dir.path(),
            &FixtureProvider::new(true),
            &no_spacing(),
            "v.wav",
        );
        assert_eq!(map.words.len(), 5 + 4 + 8);
        let first = &map.words[0];
        assert_eq!(first.text, "Orders");
        assert!((first.start - map.sentences[0].start).abs() < 1e-3);
        assert_eq!(first.confidence, 1.0);
        let sentence = &map.sentences[1];
        let inside = map.words_in(sentence);
        assert_eq!(inside.len(), 4);
        assert!(inside.iter().all(|w| w.end <= sentence.end + 1e-3));
        for pair in map.words.windows(2) {
            assert!(pair[1].start >= pair[0].start);
        }
    }

    #[test]
    fn word_timer_hook_is_used_only_without_provider_words() {
        fn timer(clip: &[i16], sr: u32, words: &[String]) -> Vec<SpeechWord> {
            let d = clip.len() as f64 / sr as f64;
            let each = d / words.len() as f64;
            words
                .iter()
                .enumerate()
                .map(|(i, w)| SpeechWord {
                    text: w.clone(),
                    start: i as f64 * each,
                    end: (i + 1) as f64 * each,
                    confidence: 0.5,
                })
                .collect()
        }
        let dir = TempDir::new("timer");
        let cache = VoiceCache::new(dir.path().join("cache"));
        let (map, _) = build_speech(
            &statements(),
            TtsModel::Say,
            &voice(),
            &FixtureProvider::new(false),
            &cache,
            &no_spacing(),
            &dir.path().join("v.wav"),
            false,
            Some(timer),
        )
        .unwrap();
        assert_eq!(map.words.len(), 5 + 4 + 8);
        assert_eq!(map.words[0].confidence, 0.5);
        assert!((map.words[0].start - map.sentences[0].start).abs() < 1e-9);
        // Provider words win over the hook.
        let (map2, _) = build_speech(
            &statements(),
            TtsModel::Say,
            &voice(),
            &FixtureProvider::new(true),
            &VoiceCache::new(dir.path().join("cache2")),
            &no_spacing(),
            &dir.path().join("v2.wav"),
            false,
            Some(timer),
        )
        .unwrap();
        assert_eq!(map2.words[0].confidence, 1.0);
    }

    #[test]
    fn rerun_is_byte_identical_with_zero_calls() {
        let dir = TempDir::new("rerun");
        let p = FixtureProvider::new(true);
        let pauses = pauses_for(TemperamentKind::Restrained);
        let cache = VoiceCache::new(dir.path().join("cache"));
        let run = |name: &str| {
            build_speech(
                &statements(),
                TtsModel::Say,
                &voice(),
                &p,
                &cache,
                &pauses,
                &dir.path().join(name),
                false,
                None,
            )
            .unwrap()
        };
        let (m1, s1) = run("a.wav");
        let (m2, s2) = run("a2.wav");
        assert_eq!((s1.calls, s1.cache_hits), (3, 0));
        assert_eq!((s2.calls, s2.cache_hits), (0, 3));
        let a = std::fs::read(dir.path().join("a.wav")).unwrap();
        let b = std::fs::read(dir.path().join("a2.wav")).unwrap();
        assert_eq!(a, b);
        assert_eq!(m1.sentences, m2.sentences);
        assert_eq!(m1.words, m2.words);

        // Fully offline rerun also works.
        let offline = build_speech(
            &statements(),
            TtsModel::Say,
            &voice(),
            &p,
            &cache,
            &pauses,
            &dir.path().join("a3.wav"),
            true,
            None,
        );
        assert!(offline.is_ok());
    }

    #[test]
    fn offline_miss_is_reported() {
        let dir = TempDir::new("offmiss");
        let cache = VoiceCache::new(dir.path().join("cache"));
        let r = build_speech(
            &statements(),
            TtsModel::Say,
            &voice(),
            &FixtureProvider::new(false),
            &cache,
            &no_spacing(),
            &dir.path().join("v.wav"),
            true,
            None,
        );
        assert!(matches!(r, Err(VoiceError::OfflineMiss(_))));
    }

    #[test]
    fn statement_words_strip_punctuation() {
        assert_eq!(
            statement_words("Save 40% — now, today!"),
            vec!["Save", "40%", "now", "today"]
        );
    }

    #[test]
    fn single_take_splits_one_read_into_sentences() {
        let dir = TempDir::new("single_take");
        let cache = VoiceCache::new(dir.path().join("cache"));
        let provider = FixtureProvider { with_words: false };
        let lines = statements();
        let (map, stats) = build_speech_single_take(
            &lines,
            TtsModel::Say,
            &voice(),
            &provider,
            &cache,
            &no_spacing(),
            &dir.path().join("take.wav"),
            false,
            Some(crate::timing::time_words),
        )
        .unwrap();
        // One provider call for the whole script.
        assert_eq!(stats.calls, 1);
        assert_eq!(map.sentences.len(), 3);
        for (i, s) in map.sentences.iter().enumerate() {
            assert_eq!(s.beat, i);
            assert!(s.end > s.start);
            if i > 0 {
                let gap = s.start - map.sentences[i - 1].end;
                assert!(
                    gap >= NATURAL_PAUSE.0 - 1e-3 && gap <= NATURAL_PAUSE.1 + 1e-3,
                    "gap {gap}"
                );
            }
        }
        // Each sentence holds its own words (fixture word count per line).
        for (i, s) in map.sentences.iter().enumerate() {
            let n = map
                .words
                .iter()
                .filter(|w| w.start >= s.start - 1e-6 && w.start < s.end)
                .count();
            assert_eq!(n, statement_words(&lines[i]).len(), "sentence {i}");
        }
        // Deterministic and cached: a rerun is byte-identical with 0 calls.
        let (again, st2) = build_speech_single_take(
            &lines,
            TtsModel::Say,
            &voice(),
            &provider,
            &cache,
            &no_spacing(),
            &dir.path().join("take2.wav"),
            false,
            Some(crate::timing::time_words),
        )
        .unwrap();
        assert_eq!(st2.calls, 0);
        assert_eq!(again.sentences, map.sentences);
        assert_eq!(
            std::fs::read(dir.path().join("take.wav")).unwrap(),
            std::fs::read(dir.path().join("take2.wav")).unwrap()
        );
    }

    #[test]
    fn single_take_spacing_is_capped() {
        let dir = TempDir::new("single_take_cap");
        let cache = VoiceCache::new(dir.path().join("cache"));
        let provider = FixtureProvider { with_words: false };
        let pauses = Pauses {
            min_spacing: 0.0,
            min_spacings: vec![30.0, 30.0],
            ..pauses_for(TemperamentKind::Editorial)
        };
        let (map, _) = build_speech_single_take(
            &statements(),
            TtsModel::Say,
            &voice(),
            &provider,
            &cache,
            &pauses,
            &dir.path().join("cap.wav"),
            false,
            None,
        )
        .unwrap();
        for w in map.sentences.windows(2) {
            let gap = w[1].start - w[0].end;
            assert!(gap <= NATURAL_PAUSE.1 + MAX_EXTRA_PAUSE + 1e-3, "gap {gap}");
        }
    }

    #[test]
    fn boundary_choice_needs_enough_pauses() {
        assert!(choose_boundaries(&[(1.0, 1.3)], &[1.0, 2.0], 3.0).is_none());
        let picks = choose_boundaries(
            &[(0.4, 0.5), (1.0, 1.4), (1.6, 1.7), (2.0, 2.5)],
            &[1.1, 2.2],
            3.0,
        )
        .unwrap();
        assert_eq!(picks, vec![1, 3]);
        assert_eq!(
            single_take_script(&["Hi there".into(), "Done!".into()]),
            "Hi there. Done!"
        );
    }

    #[test]
    fn fragments_flow_and_silent_beats_hold() {
        assert_eq!(
            single_take_script(&[
                "It's midnight,".into(),
                "you're staring at the ceiling".into(),
                "".into(),
                "and it replays".into(),
                "Again".into(),
            ]),
            "It's midnight, you're staring at the ceiling and it replays. Again."
        );
        let dir = TempDir::new("fragments");
        let cache = VoiceCache::new(dir.path().join("cache"));
        let provider = FixtureProvider { with_words: true };
        let lines: Vec<String> = vec![
            "It's midnight,".into(),
            "you're staring at the ceiling,".into(),
            "".into(),
            "and your brain replays it.".into(),
        ];
        // Word-level finder from the fixture's own word timing (stands in for
        // speech recognition): cut between the lines' boundary words.
        let script = single_take_script(&lines);
        let (_, fx_words) = provider.render(&script);
        let lead = fx_words[0].start;
        let counts: Vec<usize> = lines
            .iter()
            .filter(|l| !l.trim().is_empty())
            .map(|l| statement_words(l).len())
            .collect();
        let finder = move |_: &[i16]| {
            let mut cuts = Vec::new();
            let mut k = 0;
            for c in &counts[..counts.len() - 1] {
                k += c;
                cuts.push((fx_words[k - 1].end + fx_words[k].start) / 2.0 - lead);
            }
            Some(cuts)
        };
        let (map, stats) = build_speech_single_take_with(
            &lines,
            &lines,
            TtsModel::Say,
            &voice(),
            &provider,
            &cache,
            &no_spacing(),
            &dir.path().join("frag.wav"),
            false,
            Some(crate::timing::time_words),
            Some(&finder),
        )
        .unwrap();
        assert_eq!(stats.calls, 1);
        assert_eq!(map.sentences.len(), 4);
        // The silent beat holds time and has no words.
        let silent = &map.sentences[2];
        assert_eq!(silent.start, silent.end);
        assert!(map.sentences[3].start - silent.start >= SILENT_BEAT_SECONDS - 1e-6);
        for (i, s) in map.sentences.iter().enumerate() {
            let n = map
                .words
                .iter()
                .filter(|w| w.start >= s.start - 1e-6 && w.start < s.end.max(s.start + 1e-6))
                .count();
            assert_eq!(n, statement_words(&lines[i]).len(), "beat {i}");
        }
        // Without a finder the comma-joined fragments have no pauses to cut
        // at: the take is still split (quietest points), in one call, with
        // every beat present and increasing.
        let calls_before = cache.calls();
        let (map2, _) = build_speech_single_take_with(
            &lines,
            &lines,
            TtsModel::Say,
            &voice(),
            &provider,
            &cache,
            &no_spacing(),
            &dir.path().join("frag2.wav"),
            false,
            None,
            None,
        )
        .unwrap();
        assert!(cache.calls() - calls_before <= 1);
        assert_eq!(map2.sentences.len(), 4);
        assert!(map2.sentences.windows(2).all(|w| w[1].start >= w[0].start));
    }

    #[test]
    fn quiet_cuts_pick_the_quietest_point_near_each_expectation() {
        // 3 s of tone with a 120 ms dip at 1.4 s and 2.1 s (below the
        // 120 ms-at-−35 dB pause rule only in length, so DP has no pauses).
        let n = SAMPLE_RATE as usize * 3;
        let mut s: Vec<i16> = (0..n)
            .map(|i| ((i as f64 * 0.05).sin() * 12000.0) as i16)
            .collect();
        for dip in [1.4f64, 2.1] {
            let a = samples_of(dip - 0.04);
            for v in &mut s[a..a + samples_of(0.08)] {
                *v /= 40;
            }
        }
        let cuts = quiet_cuts(&s, &[1.2, 2.3], 3.0);
        assert_eq!(cuts.len(), 2);
        assert!((cuts[0] - 1.4).abs() < 0.06, "{cuts:?}");
        assert!((cuts[1] - 2.1).abs() < 0.06, "{cuts:?}");
        // Never fails, even on a flat take.
        let flat = vec![8000i16; n];
        let c = quiet_cuts(&flat, &[1.0, 2.0], 3.0);
        assert!(c.len() == 2 && c[0] < c[1]);
    }

    #[test]
    fn cuts_snap_to_the_quietest_40ms_between_two_words() {
        // 2 s of tone with a 60 ms near-silent dip at 0.70-0.76 s and a
        // 40 ms dip at 1.50-1.54 s.
        let n = SAMPLE_RATE as usize * 2;
        let mut s: Vec<i16> = (0..n)
            .map(|i| ((i as f64 * 0.05).sin() * 12000.0) as i16)
            .collect();
        for (from, len) in [(0.70f64, 0.06f64), (1.50, 0.04)] {
            let a = samples_of(from);
            for v in &mut s[a..a + samples_of(len)] {
                *v /= 200;
            }
        }
        // Word boundaries from a recogniser: the dips lie inside the gaps,
        // away from the gap midpoints.
        let cuts = snap_cuts_to_silence(&s, SAMPLE_RATE, &[(0.60, 1.00), (1.45, 1.70)]);
        assert_eq!(cuts.len(), 2);
        assert!((0.71..=0.75).contains(&cuts[0]), "{cuts:?}");
        assert!((cuts[1] - 1.52).abs() < 1e-9, "{cuts:?}");
        // A gap shorter than 40 ms (or an overlap) cuts at its midpoint.
        let short = snap_cuts_to_silence(&s, SAMPLE_RATE, &[(0.70, 0.73), (1.0, 0.98)]);
        assert_eq!(short, vec![0.715, 0.99]);
        // Uniform silence: the window nearest the midpoint.
        let quiet = vec![0i16; n];
        let c = snap_cuts_to_silence(&quiet, SAMPLE_RATE, &[(0.2, 1.0)]);
        assert!((c[0] - 0.6).abs() < 0.0051, "{c:?}");
        // A gap past the end of the audio falls back to its midpoint.
        assert_eq!(
            snap_cuts_to_silence(&s, SAMPLE_RATE, &[(5.0, 6.0)]),
            vec![5.5]
        );
        // The same gap at another rate lands at the same time.
        let s16: Vec<i16> = s.iter().step_by(3).copied().collect();
        let c16 = snap_cuts_to_silence(&s16, 16_000, &[(1.45, 1.70)]);
        assert!((c16[0] - 1.52).abs() < 1e-9, "{c16:?}");
    }
}
