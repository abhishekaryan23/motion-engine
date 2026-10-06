//! (0.20 A1) Local word timing on an existing take: reads a
//! `<name>.speech.json` (its `audio` beside it), recognises the WAV with the
//! local model through `asr_local::transcribe` (a fresh temporary cache, so
//! the first run is a real decode and the second a cache hit), applies the
//! words with `asr::apply`, and prints how many script words matched, the
//! alignment label, wall times and sample words (onset vs local).
//!
//! (0.20 A3) With a fourth argument `ctc` (`--features asr-local,asr-ctc`
//! and `models fetch wav2vec2-base-960h`) it also runs the CTC refinement
//! (`LocalParams.ctc`): matched words and wall times for both, the
//! per-window report of `asr_ctc::refine_detailed`, and sample words as
//! onset vs whisper vs whisper + CTC.
//!
//! ```text
//! cargo run --release -p motion-voice --features asr-local,asr-ctc --example local_take -- \
//!     output/reel/reel.speech.json [whisper-base.en] [beam] [ctc]
//! ```

use motion_voice::asr::{self, AsrWord};
use motion_voice::asr_local::{self, LocalParams};
use motion_voice::{SpeechMap, SpeechWord};
use std::path::Path;
use std::time::Instant;

/// One local run: words, cold wall s, cached wall s, identical on rerun.
type Run = (Vec<AsrWord>, f64, f64, bool);

/// One local run (a fresh temporary cache: a cold decode, then a cache hit).
fn run(
    wav: &[u8],
    spec: &asr_local::ModelSpec,
    params: &LocalParams,
    tag: &str,
) -> Result<Run, Box<dyn std::error::Error>> {
    let cache = std::env::temp_dir().join(format!("me_local_take_{}_{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&cache);
    let root = asr_local::models_root();
    let t0 = Instant::now();
    let (heard, hit1) = asr_local::transcribe(wav, spec, &root, params, &cache)?;
    let cold = t0.elapsed().as_secs_f64();
    let t1 = Instant::now();
    let (again, hit2) = asr_local::transcribe(wav, spec, &root, params, &cache)?;
    let warm = t1.elapsed().as_secs_f64();
    let _ = std::fs::remove_dir_all(&cache);
    Ok((heard.clone(), cold, warm, !hit1 && hit2 && heard == again))
}

fn report(map: &SpeechMap, heard: &[AsrWord], label: &str) -> Vec<SpeechWord> {
    let mut words = map.words.clone();
    let matched = asr::apply(&mut words, heard);
    let texts: Vec<String> = map.words.iter().map(|w| w.text.clone()).collect();
    let extra: Vec<&AsrWord> = asr::unmatched_recognised(&texts, heard)
        .into_iter()
        .map(|j| &heard[j])
        .collect();
    println!(
        "alignment: {label} | script words measured: {matched}/{} ({:.1} %) | recognised words: {}",
        map.words.len(),
        100.0 * matched as f64 / map.words.len().max(1) as f64,
        heard.len()
    );
    println!(
        "heard but not in the script: {}",
        if extra.is_empty() {
            "none".to_string()
        } else {
            extra
                .iter()
                .map(|w| format!("~{} @{:.2}", w.word, w.start))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    words
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let Some(speech_path) = args.get(1) else {
        eprintln!("usage: local_take <name.speech.json> [model] [beam] [ctc]");
        std::process::exit(2);
    };
    let model = args
        .get(2)
        .map(String::as_str)
        .unwrap_or(asr_local::DEFAULT_MODEL);
    let spec = asr_local::model_spec(model).ok_or("unknown model (see `models list`)")?;
    let beam: u32 = args.get(3).map(|b| b.parse()).transpose()?.unwrap_or(1);
    let with_ctc = args.get(4).is_some_and(|a| a == "ctc");
    let params = LocalParams {
        beam_size: beam,
        ..LocalParams::default()
    };
    let map = SpeechMap::from_json(&std::fs::read_to_string(speech_path)?)?;
    let wav_path = Path::new(speech_path)
        .parent()
        .unwrap_or(Path::new("."))
        .join(&map.audio);
    let wav = std::fs::read(&wav_path)?;
    let seconds = motion_voice::wav::decode(&wav)?.duration();
    println!("take: {} ({seconds:.1} s)", wav_path.display());

    let (heard, cold, warm, same) = run(&wav, spec, &params, "w")?;
    let words = report(&map, &heard, &asr_local::alignment_label(spec, &params));
    println!("wall: {cold:.2} s decode, {warm:.3} s cached (identical={same})");

    let mut refined: Option<Vec<SpeechWord>> = None;
    if with_ctc {
        let ctc_params = LocalParams {
            ctc: true,
            ..params.clone()
        };
        let (heard_ctc, cold, warm, same) = run(&wav, spec, &ctc_params, "c")?;
        println!();
        let w = report(
            &map,
            &heard_ctc,
            &asr_local::alignment_label(spec, &ctc_params),
        );
        println!("wall: {cold:.2} s whisper + CTC, {warm:.3} s cached (identical={same})");
        // The CTC pass alone, with its per-window report.
        let decoded = motion_voice::wav::decode(&wav)?;
        let audio = asr_local::resample_16k(&decoded.samples, decoded.sample_rate);
        let t0 = Instant::now();
        let detail =
            motion_voice::asr_ctc::refine_detailed(&audio, &heard, &asr_local::models_root())?;
        println!(
            "CTC pass alone: {:.2} s, {} windows",
            t0.elapsed().as_secs_f64(),
            detail.windows.len()
        );
        for win in &detail.windows {
            println!(
                "  window {:>6.2}-{:>6.2} s, words {:>3}..{:<3} {:?}",
                win.start, win.end, win.words.start, win.words.end, win.outcome
            );
        }
        let moved: Vec<f64> = heard
            .iter()
            .zip(&heard_ctc)
            .map(|(a, b)| (b.start - a.start) * 1000.0)
            .collect();
        let abs: Vec<f64> = moved.iter().map(|d| d.abs()).collect();
        println!(
            "whisper -> +ctc start shift over recognised words: mean {:+.0} ms, mean |d| {:.0} ms, max |d| {:.0} ms",
            moved.iter().sum::<f64>() / moved.len().max(1) as f64,
            abs.iter().sum::<f64>() / abs.len().max(1) as f64,
            abs.iter().copied().fold(0.0, f64::max)
        );
        refined = Some(w);
    }

    let changed: Vec<f64> = map
        .words
        .iter()
        .zip(&words)
        .map(|(a, b)| (b.start - a.start) * 1000.0)
        .collect();
    let abs: Vec<f64> = changed.iter().map(|d| d.abs()).collect();
    println!(
        "\nonset -> local start shift: mean |d| {:.0} ms, max |d| {:.0} ms",
        abs.iter().sum::<f64>() / abs.len().max(1) as f64,
        abs.iter().copied().fold(0.0, f64::max)
    );
    let n = map.words.len();
    let step = (n / 10).max(1);
    match &refined {
        None => {
            println!("\n| word | onset start | local start | d ms |");
            println!("|---|---:|---:|---:|");
            for i in (0..n).step_by(step).take(10) {
                println!(
                    "| {} | {:.3} | {:.3} | {:+.0} |",
                    map.words[i].text, map.words[i].start, words[i].start, changed[i]
                );
            }
        }
        Some(r) => {
            println!("\n| word | onset start | whisper start | +ctc start | +ctc end | ctc - whisper ms |");
            println!("|---|---:|---:|---:|---:|---:|");
            for i in (0..n).step_by(step).take(10) {
                println!(
                    "| {} | {:.3} | {:.3} | {:.3} | {:.3} | {:+.0} |",
                    map.words[i].text,
                    map.words[i].start,
                    words[i].start,
                    r[i].start,
                    r[i].end,
                    (r[i].start - words[i].start) * 1000.0
                );
            }
        }
    }
    Ok(())
}
