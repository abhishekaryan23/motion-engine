//! (0.20 A1) Dump whisper's raw segments and tokens (text, DTW mark,
//! heuristic times) to inspect timestamp sources. Not cached; the models come
//! from `asr_local::models_root()` (`motion-engine models fetch <name>`).
//!
//! ```text
//! cargo run --release -p motion-voice --features asr-local --example whisper_raw -- \
//!     whisper-base.en 1 out_dir/ a.wav b.wav ...
//! ```
//! writes `out_dir/<stem>.<model>.beam<n>.json` per WAV (`[RawSegment]`).

#[cfg(feature = "asr-local")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use motion_voice::asr_local::{self, LocalParams};
    use std::path::Path;
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 5 {
        eprintln!("usage: whisper_raw <model> <beam> <out_dir> <clip.wav>...");
        std::process::exit(2);
    }
    let spec = asr_local::model_spec(&args[1]).ok_or("unknown model (see `models list`)")?;
    let beam: u32 = args[2].parse()?;
    let out_dir = Path::new(&args[3]);
    std::fs::create_dir_all(out_dir)?;
    let params = LocalParams {
        beam_size: beam,
        ..LocalParams::default()
    };
    let model = asr_local::model_path(&asr_local::models_root(), spec);
    for wav_path in &args[4..] {
        let wav = motion_voice::wav::decode(&std::fs::read(wav_path)?)?;
        let audio = asr_local::resample_16k(&wav.samples, wav.sample_rate);
        let t0 = std::time::Instant::now();
        let segs = asr_local::recognise_segments(&audio, &model, spec.preset, &params)?;
        let stem = Path::new(wav_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("clip");
        let out = out_dir.join(format!("{stem}.{}.beam{beam}.json", spec.name));
        std::fs::write(&out, serde_json::to_string_pretty(&segs)?)?;
        eprintln!(
            "{stem}: {:.1} s of audio, {} segments in {:.2} s -> {}",
            wav.duration(),
            segs.len(),
            t0.elapsed().as_secs_f64(),
            out.display()
        );
    }
    Ok(())
}

#[cfg(not(feature = "asr-local"))]
fn main() {
    eprintln!("whisper_raw needs `--features asr-local`");
    std::process::exit(2);
}
