use motion_core::intent::CreativeIntent;
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_voice::speech_build::statement_words;
use motion_voice::timing::time_words;
use motion_voice::wav;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: single_take <intent.json> <voice.wav> <out_dir>");
        std::process::exit(1);
    }

    let intent_path = &args[1];
    let wav_path = &args[2];
    let out_dir = Path::new(&args[3]);

    let intent_str = std::fs::read_to_string(intent_path)?;
    let intent: CreativeIntent = serde_json::from_str(&intent_str)?;
    let statements: Vec<String> = intent.beats.iter().map(|b| b.statement.clone()).collect();

    let wav_bytes = std::fs::read(wav_path)?;
    let clip = wav::decode(&wav_bytes)?;
    let sample_rate = clip.sample_rate;
    let total_samples = clip.samples.len();
    let total_duration = total_samples as f64 / sample_rate as f64;

    println!(
        "Audio: {:.2}s, {} samples at {}Hz",
        total_duration, total_samples, sample_rate
    );

    let split_secs: Vec<f64> = if args.len() > 4 {
        let mut v: Vec<f64> = args[4]
            .split(',')
            .filter_map(|s| s.trim().parse::<f64>().ok())
            .collect();
        if v.first() != Some(&0.0) {
            v.insert(0, 0.0);
        }
        if v.last() != Some(&total_duration) {
            v.push(total_duration);
        }
        v
    } else {
        vec![0.0, 4.02, 8.03, 11.28, 15.08, total_duration]
    };

    assert_eq!(
        statements.len(),
        split_secs.len() - 1,
        "statements count must match splits"
    );

    let mut sentences = Vec::new();
    let mut words = Vec::new();

    for i in 0..5 {
        let t_start = split_secs[i];
        let t_end = split_secs[i + 1];
        let idx_start = ((t_start * sample_rate as f64).round() as usize).min(total_samples);
        let idx_end = ((t_end * sample_rate as f64).round() as usize).min(total_samples);

        let slice = &clip.samples[idx_start..idx_end];
        let st_words = statement_words(&statements[i]);

        let timed_words = time_words(slice, sample_rate, &st_words);

        let sent_start = timed_words
            .first()
            .map(|w| t_start + w.start)
            .unwrap_or(t_start);
        let sent_end = timed_words.last().map(|w| t_start + w.end).unwrap_or(t_end);

        sentences.push(SpeechSentence {
            beat: i,
            start: sent_start,
            end: sent_end,
        });

        for w in timed_words {
            words.push(SpeechWord {
                text: w.text,
                start: t_start + w.start,
                end: t_start + w.end,
                confidence: w.confidence,
            });
        }
    }

    let out_wav_name = format!("{}.voice.wav", intent.title);
    let out_json_name = format!("{}.speech.json", intent.title);

    let raw_map = SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: out_wav_name.clone(),
        sample_rate,
        duration: total_duration,
        provider: "openrouter".to_string(),
        model: "deepgram/flux-tts:free".to_string(),
        voice: "flux-marcus-en".to_string(),
        words,
        sentences,
    };

    let (repaired_map, report) = repair(&raw_map, &statements);
    println!(
        "Repair: {} reordered, {} stretched, {} gaps clamped, {} unmatched statement words, {} unmatched spoken words",
        report.reordered,
        report.stretched,
        report.gaps_clamped,
        report.unmatched_statement_words.len(),
        report.unmatched_spoken_words.len()
    );

    std::fs::create_dir_all(out_dir)?;
    let target_wav = out_dir.join(&out_wav_name);
    let target_json = out_dir.join(&out_json_name);

    std::fs::copy(wav_path, &target_wav)?;
    let json_text = serde_json::to_string_pretty(&repaired_map)?;
    std::fs::write(&target_json, json_text + "\n")?;

    println!("Wrote: {}", target_wav.display());
    println!("Wrote: {}", target_json.display());
    Ok(())
}
