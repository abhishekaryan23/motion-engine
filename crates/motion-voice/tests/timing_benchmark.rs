//! (0.20 A4) Word-timing benchmark against ground-truth speech fixtures.
//!
//! Fixtures come from `scripts/build_groundtruth.sh` (macOS speech synthesis;
//! word positions are the synthesiser's own AVSpeechSynthesisMarker byte
//! offsets into the audio it wrote): `<stem>.wav` (48 kHz mono s16),
//! `<stem>.words.json` (`[{"word","start","end"}]`, seconds; `end` is the next
//! word's start, so it includes any pause) and `<stem>.script.txt`.
//!
//! Every arm gets the WAV samples and the script's engine tokens
//! (`statement_words`) and returns `(word, start, end)` in seconds from the
//! clip start. Its words are aligned to the ground truth with a monotone DP
//! over normalised words (lowercase alphanumerics, Levenshtein ratio ≥ 0.6),
//! so an arm that rewrites numbers still scores wherever words match; only
//! matched words are scored. Starts are the primary metric.
//!
//! ```text
//! scripts/build_groundtruth.sh
//! cargo test -p motion-voice --test timing_benchmark -- --ignored --nocapture
//! ```
//! `MOTION_GROUNDTRUTH_DIR` overrides the fixture directory
//! (default `<workspace>/output/groundtruth`). The sprint targets below are
//! for the local aligner; the benchmark only prints PASS/FAIL, it never fails
//! because an arm misses them.

use std::path::{Path, PathBuf};
use std::time::Instant;

use motion_voice::speech_build::statement_words;
use motion_voice::timing::time_words;
use motion_voice::wav;

/// `(word, start, end)`, seconds from the clip start.
type Timed = (String, f64, f64);

/// One word-timing method: `(samples, sample rate, engine words)` → timed words.
type Aligner = Box<dyn Fn(&[i16], u32, &[String]) -> Vec<(String, f64, f64)>>;

/// The benchmark arms, in print order.
fn arms() -> Vec<(&'static str, Aligner)> {
    #[allow(unused_mut)]
    let mut arms = vec![(
        "onset",
        Box::new(|samples: &[i16], rate: u32, words: &[String]| {
            time_words(samples, rate, words)
                .into_iter()
                .map(|w| (w.text, w.start, w.end))
                .collect()
        }) as Aligner,
    )];
    // Sprint 0.20 arms are registered here as they land, each as
    // `(name, Box::new(|samples, rate, words| -> Vec<(word, start, end)>))`:
    //   "local_*"      -- local whisper.cpp aligner (`--align local`), below
    //   "local_*_ctc"  -- whisper's words re-timed by CTC forced alignment
    //                     (`--asr-ctc on`, `--features asr-local,asr-ctc`)
    #[cfg(feature = "asr-local")]
    arms.extend(local::arms());
    arms
}

/// (0.20 A1) The local whisper.cpp arms (`--features asr-local`), through
/// `asr_local::transcribe` with the installed models and a temporary cache:
/// each `<arm>` decodes (real wall time), the following `<arm>_cached`
/// reads the same result back from the cache. An arm whose model is not
/// installed is skipped with a notice. `MOTION_BENCH_TS_SOURCES=1` adds the
/// timestamp-source comparison on base.en, beam 1 (uncached `src_*` arms).
/// (0.20 A3) With `--features asr-local,asr-ctc` the `local_base_ctc` and
/// `local_small_ctc` arms run whisper then CTC forced alignment
/// (`LocalParams.ctc`; wall time is both), skipped when wav2vec2-base-960h is
/// not installed.
#[cfg(feature = "asr-local")]
mod local {
    use super::Aligner;
    use motion_voice::asr_local::{self, LocalParams, TimestampSource};
    use motion_voice::wav;
    use std::path::PathBuf;

    /// The per-run cache, removed by [`cleanup`].
    pub fn cache_dir() -> PathBuf {
        std::env::temp_dir().join(format!("me_timing_bench_{}", std::process::id()))
    }

    pub fn cleanup() {
        let _ = std::fs::remove_dir_all(cache_dir());
    }

    fn transcribe_arm(model: &'static str, beam: u32, ctc: bool) -> Aligner {
        Box::new(move |samples: &[i16], rate: u32, _words: &[String]| {
            let Some(spec) = asr_local::model_spec(model) else {
                return Vec::new();
            };
            let params = LocalParams {
                beam_size: beam,
                ctc,
                ..LocalParams::default()
            };
            let wav = wav::encode_at(samples, rate);
            match asr_local::transcribe(
                &wav,
                spec,
                &asr_local::models_root(),
                &params,
                &cache_dir(),
            ) {
                Ok((words, _)) => words
                    .into_iter()
                    .map(|w| (w.word, w.start, w.end))
                    .collect(),
                Err(e) => {
                    println!("{model} beam {beam}: {e}");
                    Vec::new()
                }
            }
        })
    }

    fn source_arm(source: TimestampSource) -> Aligner {
        Box::new(move |samples: &[i16], rate: u32, _words: &[String]| {
            let Some(spec) = asr_local::model_spec("whisper-base.en") else {
                return Vec::new();
            };
            let audio = asr_local::resample_16k(samples, rate);
            let model = asr_local::model_path(&asr_local::models_root(), spec);
            match asr_local::recognise_segments(
                &audio,
                &model,
                spec.preset,
                &LocalParams::default(),
            ) {
                Ok(segs) => asr_local::words_from_segments(&segs, source)
                    .into_iter()
                    .map(|w| (w.word, w.start, w.end))
                    .collect(),
                Err(e) => {
                    println!("timestamp source {source:?}: {e}");
                    Vec::new()
                }
            }
        })
    }

    pub fn arms() -> Vec<(&'static str, Aligner)> {
        cleanup();
        let root = asr_local::models_root();
        let mut out: Vec<(&'static str, Aligner)> = Vec::new();
        #[allow(unused_mut)]
        let mut configs: Vec<(&str, &str, &str, u32, bool)> = vec![
            (
                "local_base_beam1",
                "local_base_beam1_cached",
                "whisper-base.en",
                1,
                false,
            ),
            (
                "local_base_beam5",
                "local_base_beam5_cached",
                "whisper-base.en",
                5,
                false,
            ),
            (
                "local_small_beam1",
                "local_small_beam1_cached",
                "whisper-small.en",
                1,
                false,
            ),
            (
                "local_tiny_beam1",
                "local_tiny_beam1_cached",
                "whisper-tiny.en",
                1,
                false,
            ),
        ];
        #[cfg(feature = "asr-ctc")]
        {
            use motion_voice::asr_ctc;
            if asr_ctc::installed(&root, &asr_ctc::MODEL).is_some() {
                configs.push((
                    "local_base_ctc",
                    "local_base_ctc_cached",
                    "whisper-base.en",
                    1,
                    true,
                ));
                configs.push((
                    "local_small_ctc",
                    "local_small_ctc_cached",
                    "whisper-small.en",
                    1,
                    true,
                ));
            } else {
                println!(
                    "local_*_ctc: skipped, {} is not installed under {} \
                     (cargo run -p motion-cli --features asr-local,asr-ctc -- models fetch {})",
                    asr_ctc::MODEL.name,
                    root.display(),
                    asr_ctc::MODEL.name
                );
            }
        }
        for (name, cached, model, beam, ctc) in configs {
            let installed = asr_local::model_spec(model)
                .and_then(|spec| asr_local::installed(&root, spec))
                .is_some();
            if !installed {
                println!(
                    "{name}: skipped, {model} is not installed under {} \
                     (cargo run -p motion-cli --features asr-local -- models fetch {model})",
                    root.display()
                );
                continue;
            }
            out.push((name, transcribe_arm(model, beam, ctc)));
            out.push((cached, transcribe_arm(model, beam, ctc)));
        }
        let base_installed = asr_local::model_spec("whisper-base.en")
            .and_then(|spec| asr_local::installed(&root, spec))
            .is_some();
        if base_installed && std::env::var_os("MOTION_BENCH_TS_SOURCES").is_some() {
            out.push(("src_base_segment", source_arm(TimestampSource::Segment)));
            out.push(("src_base_dtw_own", source_arm(TimestampSource::DtwOwn)));
            out.push(("src_base_dtw_span", source_arm(TimestampSource::DtwSpan)));
        }
        out
    }
}

// Sprint targets (local aligner), on |Δstart| over matched words.
const TARGET_MEAN_MS: f64 = 40.0;
const TARGET_P95_MS: f64 = 90.0;
const TARGET_MAX_MS: f64 = 150.0;
const TARGET_MATCHED_PCT: f64 = 97.0;
/// Minimum normalised-word similarity for a DP match (as `asr::align_pairs`).
const MIN_SIMILARITY: f64 = 0.6;

// ---------------------------------------------------------------- alignment

fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 1 for equal normalised forms, else the Levenshtein ratio; 0 when either is empty.
fn similarity(a: &str, b: &str) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb))
                .min(prev[j + 1] + 1)
                .min(cur[j] + 1);
        }
        prev = cur;
    }
    1.0 - prev[b.len()] as f64 / a.len().max(b.len()) as f64
}

/// Monotone alignment of `truth` to `arm` words maximising the summed
/// similarity of matched pairs (each ≥ [`MIN_SIMILARITY`]). Returns
/// `(truth index, arm index)` pairs in order.
fn align(truth: &[String], arm: &[String]) -> Vec<(usize, usize)> {
    let (n, m) = (truth.len(), arm.len());
    if n == 0 || m == 0 {
        return Vec::new();
    }
    let a: Vec<String> = truth.iter().map(|w| norm(w)).collect();
    let b: Vec<String> = arm.iter().map(|w| norm(w)).collect();
    let sim: Vec<Vec<f64>> = a
        .iter()
        .map(|x| b.iter().map(|y| similarity(x, y)).collect())
        .collect();
    let mut score = vec![vec![0.0f64; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            let s = sim[i - 1][j - 1];
            let diag = if s >= MIN_SIMILARITY {
                score[i - 1][j - 1] + s
            } else {
                f64::MIN
            };
            score[i][j] = diag.max(score[i - 1][j]).max(score[i][j - 1]);
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        let s = sim[i - 1][j - 1];
        if s >= MIN_SIMILARITY && (score[i][j] - (score[i - 1][j - 1] + s)).abs() < 1e-9 {
            pairs.push((i - 1, j - 1));
            i -= 1;
            j -= 1;
        } else if (score[i][j] - score[i - 1][j]).abs() < 1e-9 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    pairs.reverse();
    pairs
}

// --------------------------------------------------------------- statistics

/// Errors of one arm on one fixture.
#[derive(Debug, Clone, Default, PartialEq)]
struct Score {
    truth_words: usize,
    arm_words: usize,
    matched: usize,
    /// |Δstart| per matched word, ms.
    start_ms: Vec<f64>,
    /// Signed Δstart (arm − truth) per matched word, ms.
    start_signed_ms: Vec<f64>,
    /// |Δend| per matched word, ms.
    end_ms: Vec<f64>,
}

fn score(truth: &[Timed], arm: &[Timed]) -> Score {
    let t: Vec<String> = truth.iter().map(|w| w.0.clone()).collect();
    let a: Vec<String> = arm.iter().map(|w| w.0.clone()).collect();
    let pairs = align(&t, &a);
    Score {
        truth_words: truth.len(),
        arm_words: arm.len(),
        matched: pairs.len(),
        start_ms: pairs
            .iter()
            .map(|&(i, j)| (arm[j].1 - truth[i].1).abs() * 1000.0)
            .collect(),
        start_signed_ms: pairs
            .iter()
            .map(|&(i, j)| (arm[j].1 - truth[i].1) * 1000.0)
            .collect(),
        end_ms: pairs
            .iter()
            .map(|&(i, j)| (arm[j].2 - truth[i].2).abs() * 1000.0)
            .collect(),
    }
}

fn mean(v: &[f64]) -> Option<f64> {
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

/// Nearest-rank percentile (`p` in 0..=100): the smallest value with at
/// least `p` % of the values at or below it.
fn percentile(v: &[f64], p: f64) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * s.len() as f64).ceil() as usize;
    Some(s[rank.clamp(1, s.len()) - 1])
}

fn max(v: &[f64]) -> Option<f64> {
    v.iter().copied().reduce(f64::max)
}

fn pct(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        0.0
    } else {
        100.0 * part as f64 / whole as f64
    }
}

fn ms(x: Option<f64>) -> String {
    x.map_or_else(|| "-".to_string(), |v| format!("{v:.1}"))
}

/// Whether pooled results meet the sprint targets, with the misses.
fn verdict(matched_pct: f64, start_ms: &[f64]) -> (bool, Vec<String>) {
    let mut misses = Vec::new();
    let check = |name: &str, v: Option<f64>, limit: f64, misses: &mut Vec<String>| match v {
        Some(x) if x <= limit => {}
        Some(x) => misses.push(format!("{name} {x:.1} > {limit} ms")),
        None => misses.push(format!("{name} n/a")),
    };
    check("mean", mean(start_ms), TARGET_MEAN_MS, &mut misses);
    check(
        "p95",
        percentile(start_ms, 95.0),
        TARGET_P95_MS,
        &mut misses,
    );
    check("max", max(start_ms), TARGET_MAX_MS, &mut misses);
    if matched_pct < TARGET_MATCHED_PCT {
        misses.push(format!(
            "matched {matched_pct:.1} % < {TARGET_MATCHED_PCT} %"
        ));
    }
    (misses.is_empty(), misses)
}

// ----------------------------------------------------------------- fixtures

#[derive(serde::Deserialize)]
struct TruthWord {
    word: String,
    start: f64,
    end: f64,
}

struct Fixture {
    name: String,
    samples: Vec<i16>,
    rate: u32,
    /// Engine tokens of the spoken script.
    words: Vec<String>,
    truth: Vec<Timed>,
}

impl Fixture {
    fn seconds(&self) -> f64 {
        self.samples.len() as f64 / f64::from(self.rate.max(1))
    }
}

fn fixture_dir() -> PathBuf {
    let dir = match std::env::var_os("MOTION_GROUNDTRUTH_DIR") {
        Some(d) => PathBuf::from(d),
        None => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../output/groundtruth"),
    };
    dir.canonicalize().unwrap_or(dir)
}

fn parse_truth(json: &str) -> Result<Vec<Timed>, String> {
    let words: Vec<TruthWord> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(words
        .into_iter()
        .map(|w| (w.word, w.start, w.end))
        .collect())
}

/// Every `<stem>.words.json` with a `<stem>.wav` beside it, by name.
fn load_fixtures(dir: &Path) -> Result<Vec<Fixture>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut stems: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".words.json"))
                .map(str::to_string)
        })
        .filter(|stem| dir.join(format!("{stem}.wav")).is_file())
        .collect();
    stems.sort();
    let mut out = Vec::new();
    for stem in stems {
        let read = |ext: &str| {
            let p = dir.join(format!("{stem}.{ext}"));
            std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))
        };
        let wav = wav::decode(&read("wav")?).map_err(|e| format!("{stem}.wav: {e}"))?;
        let truth_json = String::from_utf8(read("words.json")?)
            .map_err(|e| format!("{stem}.words.json: {e}"))?;
        let truth = parse_truth(&truth_json).map_err(|e| format!("{stem}.words.json: {e}"))?;
        let script = match read("script.txt") {
            Ok(bytes) => String::from_utf8(bytes).map_err(|e| format!("{stem}.script.txt: {e}"))?,
            Err(_) => {
                println!("{stem}: no script.txt; using the ground-truth words as the script");
                truth
                    .iter()
                    .map(|w| w.0.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            }
        };
        out.push(Fixture {
            name: stem,
            samples: wav.samples,
            rate: wav.sample_rate,
            words: statement_words(&script),
            truth,
        });
    }
    Ok(out)
}

// ------------------------------------------------------------ ground check

/// Ground-truth sanity: for words after a pause, signed ms from the marker to
/// the acoustic onset (first 2 ms frame within 40 dB of the clip peak).
/// Positive = the audio starts after the marker (stop closures, weak onsets).
fn pause_onset_offsets(samples: &[i16], rate: u32, truth: &[Timed]) -> Vec<f64> {
    let hop = ((rate as usize) / 500).max(1); // 2 ms
    let db: Vec<f64> = samples
        .chunks(hop)
        .map(|c| {
            let e: f64 = c.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
            let rms = (e / c.len() as f64).sqrt() / 32768.0;
            if rms > 1e-10 {
                20.0 * rms.log10()
            } else {
                -200.0
            }
        })
        .collect();
    let peak = db.iter().copied().fold(-200.0, f64::max);
    let frame_s = hop as f64 / f64::from(rate.max(1));
    let frame = |t: f64| (t / frame_s).round().max(0.0) as usize;
    let mut out = Vec::new();
    for w in truth.iter().skip(1) {
        let k = frame(w.1);
        // 120..20 ms before the marker silent (50 dB under the peak).
        if k < 60 || k >= db.len() || db[k - 60..k - 10].iter().any(|&d| d > peak - 50.0) {
            continue;
        }
        let from = k - 30;
        let to = (k + 150).min(db.len());
        if let Some(j) = (from..to).find(|&j| db[j] > peak - 40.0) {
            out.push((j as f64 - k as f64) * frame_s * 1000.0);
        }
    }
    out
}

// -------------------------------------------------------------------- tests

#[test]
#[ignore = "needs fixtures from scripts/build_groundtruth.sh (macOS); run with --ignored --nocapture"]
fn word_timing_benchmark() {
    let dir = fixture_dir();
    let fixtures = if dir.is_dir() {
        load_fixtures(&dir).expect("readable ground-truth fixtures")
    } else {
        Vec::new()
    };
    if fixtures.is_empty() {
        println!(
            "timing_benchmark: no ground-truth fixtures in {}.\n\
             Generate them (macOS 13+): scripts/build_groundtruth.sh\n\
             then run: cargo test -p motion-voice --test timing_benchmark -- --ignored --nocapture\n\
             (MOTION_GROUNDTRUTH_DIR reads fixtures from another directory)",
            dir.display()
        );
        return;
    }

    println!("ground truth: {}", dir.display());
    println!("\nground-truth check (marker -> acoustic onset after pauses, signed ms):");
    for f in &fixtures {
        let d = pause_onset_offsets(&f.samples, f.rate, &f.truth);
        let abs: Vec<f64> = d.iter().map(|x| x.abs()).collect();
        println!(
            "  {:<22} {:>5.1} s  {:>3} truth words / {:>3} script tokens  pauses {:>2}  mean {:>6}  median |d| {:>6}  max |d| {:>6}",
            f.name,
            f.seconds(),
            f.truth.len(),
            f.words.len(),
            d.len(),
            ms(mean(&d)),
            ms(percentile(&abs, 50.0)),
            ms(max(&abs)),
        );
    }

    let arms = arms();
    // Per arm: pooled matched / truth words, |Δstart|, wall ms, audio
    // seconds, signed Δstart.
    #[allow(clippy::type_complexity)]
    let mut pooled: Vec<(usize, usize, Vec<f64>, f64, f64, Vec<f64>)> =
        vec![(0, 0, Vec::new(), 0.0, 0.0, Vec::new()); arms.len()];
    println!("\nper fixture (ms; Δ = arm - truth over matched words):\n");
    println!(
        "| fixture | arm | truth words | arm words | matched % | mean abs Δstart | p50 | p95 | max | mean abs Δend | wall ms |"
    );
    println!("|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for f in &fixtures {
        for (k, (name, arm)) in arms.iter().enumerate() {
            let t0 = Instant::now();
            let out = arm(&f.samples, f.rate, &f.words);
            let wall = t0.elapsed().as_secs_f64() * 1000.0;
            let s = score(&f.truth, &out);
            println!(
                "| {} | {} | {} | {} | {:.1} | {} | {} | {} | {} | {} | {:.1} |",
                f.name,
                name,
                s.truth_words,
                s.arm_words,
                pct(s.matched, s.truth_words),
                ms(mean(&s.start_ms)),
                ms(percentile(&s.start_ms, 50.0)),
                ms(percentile(&s.start_ms, 95.0)),
                ms(max(&s.start_ms)),
                ms(mean(&s.end_ms)),
                wall,
            );
            let p = &mut pooled[k];
            p.0 += s.matched;
            p.1 += s.truth_words;
            p.2.extend(s.start_ms);
            p.3 += wall;
            p.4 += f.seconds();
            p.5.extend(s.start_signed_ms);
        }
    }

    println!(
        "\nall fixtures ({} fixtures, {:.1} s of audio):\n",
        fixtures.len(),
        fixtures.iter().map(Fixture::seconds).sum::<f64>()
    );
    println!(
        "| arm | matched % | mean abs Δstart ms | p95 ms | max ms | bias ms | wall ms per 30 s audio | targets |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|---|");
    let mut lines = Vec::new();
    for ((name, _), (matched, words, start, wall, secs, signed)) in arms.iter().zip(&pooled) {
        let m = pct(*matched, *words);
        let (ok, misses) = verdict(m, start);
        let per30 = if *secs > 0.0 { wall / secs * 30.0 } else { 0.0 };
        println!(
            "| {} | {:.1} | {} | {} | {} | {} | {:.1} | {} |",
            name,
            m,
            ms(mean(start)),
            ms(percentile(start, 95.0)),
            ms(max(start)),
            ms(mean(signed)),
            per30,
            if ok { "PASS" } else { "FAIL" },
        );
        lines.push(if ok {
            format!("{name}: PASS")
        } else {
            format!("{name}: FAIL ({})", misses.join("; "))
        });
    }
    println!(
        "\ntargets: mean <= {TARGET_MEAN_MS} ms, p95 <= {TARGET_P95_MS} ms, max <= {TARGET_MAX_MS} ms, matched >= {TARGET_MATCHED_PCT} %"
    );
    for l in lines {
        println!("{l}");
    }
    #[cfg(feature = "asr-local")]
    local::cleanup();
}

#[test]
fn harness_aligns_and_scores_synthetic_words() {
    let t = |w: &str, s: f64, e: f64| (w.to_string(), s, e);
    let truth = vec![
        t("In", 0.0, 0.2),
        t("1969", 0.2, 1.0),
        t("three", 1.0, 1.3),
        t("astronauts", 1.3, 2.0),
        t("Armstrong's", 2.0, 2.6),
    ];
    // An arm that spells the number out and keeps punctuation.
    let arm = vec![
        t("in", 0.01, 0.2),
        t("nineteen", 0.25, 0.6),
        t("sixty", 0.6, 0.8),
        t("nine,", 0.8, 1.0),
        t("three", 1.05, 1.3),
        t("Astronauts,", 1.2, 2.1),
        t("Armstrongs", 2.1, 2.5),
    ];
    let names = |v: &[Timed]| v.iter().map(|w| w.0.clone()).collect::<Vec<_>>();
    assert_eq!(
        align(&names(&truth), &names(&arm)),
        vec![(0, 0), (2, 4), (3, 5), (4, 6)]
    );

    let s = score(&truth, &arm);
    assert_eq!((s.truth_words, s.arm_words, s.matched), (5, 7, 4));
    assert!((pct(s.matched, s.truth_words) - 80.0).abs() < 1e-9);
    let close = |a: Option<f64>, b: f64| a.is_some_and(|x| (x - b).abs() < 1e-6);
    // |Δstart| = 10, 50, 100, 100 ms; |Δend| = 0, 0, 100, 100 ms.
    assert!(close(mean(&s.start_ms), 65.0));
    assert!(close(percentile(&s.start_ms, 50.0), 50.0));
    assert!(close(percentile(&s.start_ms, 95.0), 100.0));
    assert!(close(max(&s.start_ms), 100.0));
    assert!(close(mean(&s.end_ms), 50.0));
    // Signed: +10, +50, -100 (Astronauts, early), +100 ms.
    assert!(close(mean(&s.start_signed_ms), 15.0));
    assert!(mean(&[]).is_none() && percentile(&[], 50.0).is_none() && max(&[]).is_none());
    assert!(close(percentile(&[5.0, 1.0, 3.0, 2.0, 4.0], 0.0), 1.0));
    assert!(close(percentile(&[5.0, 1.0, 3.0, 2.0, 4.0], 60.0), 3.0));

    // Order is monotone: a word that moved is dropped, not crossed.
    let w = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        align(&w(&["a1", "b2", "c3"]), &w(&["c3", "a1", "b2"])),
        vec![(0, 1), (1, 2)]
    );
    // Similarity: near spellings match, different words do not.
    assert!(similarity(&norm("billions"), &norm("billion")) >= MIN_SIMILARITY);
    assert!(similarity(&norm("percent"), &norm("per")) < MIN_SIMILARITY);
    assert_eq!(norm("$381"), "381");
    assert!(align(&[], &w(&["a"])).is_empty());

    // Verdict against the sprint targets.
    assert!(verdict(100.0, &[10.0, 20.0, 30.0]).0);
    let (ok, misses) = verdict(90.0, &[10.0, 20.0, 200.0]);
    assert!(!ok);
    assert_eq!(misses.len(), 4); // mean 76.7, p95 200, max 200, matched 90 %
    assert!(!verdict(100.0, &[]).0);

    // Fixture JSON shape and the ground-truth onset check.
    let truth =
        parse_truth(r#"[{"word":"In","start":0,"end":0.5},{"word":"time","start":0.5,"end":1}]"#)
            .expect("valid json");
    assert_eq!(truth[1], ("time".to_string(), 0.5, 1.0));
    assert!(parse_truth(r#"[{"word":"In"}]"#).is_err());
    let rate = 48_000u32;
    let mut samples = vec![0i16; rate as usize];
    // Silence, then a tone from 0.506 s: the marker at 0.5 s is 6 ms early.
    for (i, s) in samples.iter_mut().enumerate().skip(24_288) {
        *s = ((i as f64 * 0.05).sin() * 12_000.0) as i16;
    }
    let truth = vec![t("a", 0.0, 0.5), t("b", 0.5, 1.0)];
    let d = pause_onset_offsets(&samples, rate, &truth);
    assert_eq!(d.len(), 1);
    assert!((d[0] - 6.0).abs() <= 2.0, "{d:?}");
}
