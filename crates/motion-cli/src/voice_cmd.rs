//! (0.10) `providers set-key|show|test` and `voice`.
//!
//! The config directory can be redirected with `MOTION_CONFIG_DIR` (it then
//! holds `providers.toml`); the voice cache with `MOTION_VOICE_CACHE` or
//! `--cache`. Keys are never printed in full.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand};
use motion_core::compiler::resolve_taste;
use motion_core::compiler::typography::{emotion_name, resolve_emotion};
use motion_core::speech::{self, RecognisedWord, RepairReport, SpeechMap};
use motion_voice::asr::AsrWord;
use motion_voice::asr_ctc;
use motion_voice::asr_local::{self, LocalParams, ModelSpec};
use motion_voice::config::{self, OPENROUTER};
use motion_voice::openrouter::OpenRouterTts;
use motion_voice::speech_build::{
    build_speech_single_take_with, snap_cuts_to_silence, statement_words,
};
use motion_voice::timing::time_words;
use motion_voice::{
    auto_model, pauses_for, profile_for, voice_for, wav, MacSay, TtsModel, VoiceCache, VoiceError,
    VoiceProvider, FREE_FALLBACKS,
};

#[derive(Subcommand)]
pub enum ProvidersCmd {
    /// Read an API key from stdin and store it in the user config (0600).
    SetKey {
        /// Provider name (only `openrouter`).
        provider: String,
    },
    /// Show the provider, a masked key (or none), where it came from, and the config path.
    Show,
    /// Make one real synthesis call (Deepgram, "Test.") to check the key.
    Test {
        /// Provider name (only `openrouter`).
        provider: String,
    },
}

#[derive(Args, Debug)]
pub struct VoiceArgs {
    /// CreativeIntent JSON whose beat statements are spoken.
    pub intent: PathBuf,
    #[arg(long)]
    pub style: Option<PathBuf>,
    /// auto (default: the free OpenRouter voice, Fish S2.1 Pro), fish, say
    /// (macOS, offline), or the paid opt-ins gemini, mai, deepgram. The whole
    /// voice-over is always one take in one voice.
    #[arg(long, default_value = "auto")]
    pub tts_model: String,
    /// Word timing: onset (estimated from the audio, free), local (0.20:
    /// measured on-device with whisper.cpp, free; needs a build with
    /// `--features asr-local` and `motion-engine models fetch whisper-base.en`),
    /// asr (paid speech recognition, cached), or auto (local when available,
    /// else onset with a notice).
    #[arg(long, default_value = "auto")]
    pub align: String,
    /// (0.20) Local recogniser model for `--align local|auto` (see `models list`).
    #[arg(long, default_value = motion_voice::asr_local::DEFAULT_MODEL)]
    pub asr_model: String,
    /// (0.20) Beam width for the local recogniser (1 = greedy).
    #[arg(long, default_value_t = 1)]
    pub asr_beam: u32,
    /// (0.20 A3) Refine local word times with CTC forced alignment
    /// (wav2vec2, 20 ms frames): auto (default: on when the build has
    /// `--features asr-ctc` and `motion-engine models fetch
    /// wav2vec2-base-960h` was run, else whisper's times with a notice), on,
    /// or off. Only applies to local word timing.
    #[arg(long, default_value = "auto")]
    pub asr_ctc: String,
    /// Provider voice id; overrides the emotion-chosen voice.
    #[arg(long)]
    pub voice: Option<String>,
    /// Voice cache directory (default $MOTION_VOICE_CACHE or ~/.cache/motionengine/voice).
    #[arg(long)]
    pub cache: Option<PathBuf>,
    /// Never call a provider (TTS or the paid `asr`); a provider cache miss is
    /// an error. On-device word timing (`local`, CTC) still runs on a miss.
    #[arg(long)]
    pub offline: bool,
    /// Output directory for <title>.voice.wav and <title>.speech.json.
    #[arg(long, short)]
    pub output: PathBuf,
}

fn require_openrouter(provider: &str) -> Result<()> {
    if provider != OPENROUTER {
        bail!("unknown provider '{provider}' (only `openrouter` is supported)");
    }
    Ok(())
}

pub fn run_providers(cmd: ProvidersCmd) -> Result<()> {
    let path = config::default_config_path();
    match cmd {
        ProvidersCmd::SetKey { provider } => {
            require_openrouter(&provider)?;
            let mut input = String::new();
            std::io::stdin()
                .read_to_string(&mut input)
                .context("reading key from stdin")?;
            let key = input.trim();
            if key.is_empty() {
                bail!("no key on stdin (pipe it in: `pbpaste | motion-engine providers set-key openrouter`)");
            }
            config::set_key(&provider, key, &path)?;
            println!("saved {provider} key {}", config::mask(key));
            println!("config: {}", path.display());
            Ok(())
        }
        ProvidersCmd::Show => {
            println!("provider: {OPENROUTER}");
            match config::resolve_key(OPENROUTER, &path) {
                Some((key, source)) => {
                    println!("key: {}", config::mask(&key));
                    println!("source: {}", source.as_str());
                }
                None => {
                    println!("key: none");
                    println!("source: none");
                }
            }
            println!("config: {}", path.display());
            if let Some(w) = config::permission_warning(&path) {
                println!("warning: {w}");
            }
            Ok(())
        }
        ProvidersCmd::Test { provider } => {
            require_openrouter(&provider)?;
            let key = config::resolve_key(OPENROUTER, &path).map(|(k, _)| k);
            let tts = OpenRouterTts::with_key(key);
            // The free model with the pinned narrator (Deepgram is paid now).
            let model = motion_voice::auto_model();
            let voice = motion_voice::voice_for(motion_voice::VoiceProfile::CalmNarrator, model);
            match tts.synthesize("Test.", &voice, model) {
                Ok(s) => {
                    let secs = wav::decode(&s.wav).map(|w| w.duration()).unwrap_or(0.0);
                    println!(
                        "OK: {} bytes of wav, {secs:.2} s ({})",
                        s.wav.len(),
                        model.model_id()
                    );
                    Ok(())
                }
                Err(e) => {
                    println!("FAILED: {e}");
                    bail!("provider test failed")
                }
            }
        }
    }
}

/// Title safe for a file name.
fn file_stem(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "voice".to_string()
    } else {
        s
    }
}

/// (0.10 Q) What each beat's narrator says: the beat `narration`, else the
/// statement (taste_rules::display_text). Captions and repair use the same.
pub fn spoken_lines(intent: &motion_core::CreativeIntent) -> Vec<String> {
    intent
        .beats
        .iter()
        .map(|b| motion_core::compiler::taste_rules::display_text(b, true).spoken)
        .collect()
}

fn provider_for(model: TtsModel) -> Box<dyn VoiceProvider> {
    match model {
        TtsModel::Say => Box::new(MacSay),
        _ => {
            let key =
                config::resolve_key(OPENROUTER, &config::default_config_path()).map(|(k, _)| k);
            Box::new(OpenRouterTts::with_key(key))
        }
    }
}

/// (0.20) How this run measures word times.
enum Align {
    /// Estimated from the audio's onsets (free).
    Onset,
    /// Paid speech recognition (OpenRouter, cached).
    Asr,
    /// On-device whisper.cpp (free, cached).
    Local {
        spec: &'static ModelSpec,
        params: LocalParams,
        models: PathBuf,
    },
}

impl Align {
    /// `SpeechMap.alignment` when the recogniser's words were applied.
    fn label(&self) -> String {
        match self {
            Align::Onset => "onset".to_string(),
            Align::Asr => "asr".to_string(),
            Align::Local { spec, params, .. } => asr_local::alignment_label(spec, params),
        }
    }
}

/// (0.20 A3) `--asr-ctc` for a local run: `auto` is on when this build has
/// the CTC aligner and its model is installed under `models`, else off with
/// one line naming the fix; `on` needs both (errors name the fix); `off` as
/// asked.
fn resolve_ctc(choice: &str, models: &Path) -> Result<bool> {
    let installed = asr_ctc::installed(models, &asr_ctc::MODEL).is_some();
    let name = asr_ctc::MODEL.name;
    match choice {
        "off" => Ok(false),
        "on" => {
            if !asr_ctc::AVAILABLE {
                return Err(VoiceError::Unsupported("--asr-ctc on").into());
            }
            if !installed {
                return Err(VoiceError::ModelMissing(name.to_string()).into());
            }
            Ok(true)
        }
        "auto" => {
            if asr_ctc::AVAILABLE && installed {
                return Ok(true);
            }
            if asr_ctc::AVAILABLE {
                println!(
                    "word timing: whisper times only ({name} is not installed; for 20 ms \
                     word times run `motion-engine models fetch {name}`)"
                );
            } else {
                println!(
                    "word timing: whisper times only (no CTC aligner in this build; for 20 ms \
                     word times build with `--features asr-local,asr-ctc` and run \
                     `motion-engine models fetch {name}`)"
                );
            }
            Ok(false)
        }
        other => bail!("unknown --asr-ctc '{other}' (auto, on or off)"),
    }
}

/// `--align`: `auto` is local when this build has the recogniser and the
/// model is installed, else onset with one line saying why; `local` needs
/// both (errors name the fix); `asr` and `onset` as asked. A local run
/// resolves `--asr-ctc` too ([`resolve_ctc`]).
fn resolve_align(args: &VoiceArgs) -> Result<Align> {
    if !matches!(args.asr_ctc.as_str(), "auto" | "on" | "off") {
        bail!("unknown --asr-ctc '{}' (auto, on or off)", args.asr_ctc);
    }
    let local = |explicit: bool| -> Result<Align> {
        let spec = asr_local::model_spec(&args.asr_model).with_context(|| {
            format!(
                "unknown --asr-model '{}' (one of: {})",
                args.asr_model,
                asr_local::MODELS
                    .iter()
                    .map(|m| m.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
        let models = asr_local::models_root();
        let mut params = LocalParams {
            beam_size: args.asr_beam.max(1),
            ..LocalParams::default()
        };
        let installed = asr_local::installed(&models, spec).is_some();
        if explicit {
            if !asr_local::AVAILABLE {
                return Err(VoiceError::Unsupported("--align local").into());
            }
            if !installed {
                return Err(VoiceError::ModelMissing(spec.name.to_string()).into());
            }
        } else if !asr_local::AVAILABLE || !installed {
            let fetch = format!("`motion-engine models fetch {}`", spec.name);
            if asr_local::AVAILABLE {
                println!(
                    "word timing: onset estimates ({} is not installed; for measured \
                     word times run {fetch})",
                    spec.name
                );
            } else {
                println!(
                    "word timing: onset estimates (no local recogniser in this build; for \
                     measured word times build with `--features asr-local` and run {fetch})"
                );
            }
            return Ok(Align::Onset);
        }
        params.ctc = resolve_ctc(&args.asr_ctc, &models)?;
        Ok(Align::Local {
            spec,
            params,
            models,
        })
    };
    match args.align.as_str() {
        "auto" => local(false),
        "local" => local(true),
        "onset" => Ok(Align::Onset),
        "asr" => Ok(Align::Asr),
        other => bail!("unknown --align '{other}' (auto, local, onset or asr)"),
    }
}

/// Local recognition of `wav` (cached under `cache_root`). Allowed under
/// `--offline`: whisper and the CTC aligner run on-device, so a cache miss
/// just runs them (only provider calls, TTS and the paid `asr`, are blocked).
fn local_words(
    wav: &[u8],
    spec: &ModelSpec,
    models: &Path,
    params: &LocalParams,
    cache_root: &Path,
) -> Result<(Vec<AsrWord>, bool), VoiceError> {
    asr_local::transcribe(wav, spec, models, params, cache_root)
}

/// The beat a recognised word at `t` belongs to (1-based): the last spoken
/// sentence starting at or before `t` (else the first one).
fn beat_at(sentences: &[motion_core::speech::SpeechSentence], t: f64) -> usize {
    let spoken = || sentences.iter().filter(|s| s.end > s.start);
    spoken()
        .rfind(|s| s.start <= t + 1e-6)
        .or_else(|| spoken().next())
        .map_or(1, |s| s.beat + 1)
}

/// (0.20 A2) One line: how many script words took measured times, and what
/// the recogniser heard that the script does not contain (`beat N: ~word`).
fn mismatch_summary(map: &SpeechMap, heard: &[AsrWord], measured: usize, how: &str) -> String {
    let texts: Vec<String> = map.words.iter().map(|w| w.text.clone()).collect();
    let extra: Vec<String> = motion_voice::asr::unmatched_recognised(&texts, heard)
        .into_iter()
        .map(|j| {
            format!(
                "beat {}: ~{}",
                beat_at(&map.sentences, heard[j].start),
                heard[j].word
            )
        })
        .collect();
    format!(
        "word timing: {measured}/{} script words measured ({how}); {}",
        map.words.len(),
        if extra.is_empty() {
            "every recognised word is in the script".to_string()
        } else {
            format!("heard but not in the script: {}", extra.join(", "))
        }
    )
}

/// A failure worth retrying on the next (free) model: payment, auth, rate
/// limit, provider outage, or no key.
fn retryable(e: &VoiceError) -> bool {
    match e {
        VoiceError::MissingKey(_) | VoiceError::Network(_) => true,
        VoiceError::Http { status, .. } => matches!(status, 401..=403 | 429 | 500..=599),
        _ => false,
    }
}

pub fn run_voice(args: VoiceArgs) -> Result<()> {
    let intent = crate::read_intent(&args.intent)?;
    let style = crate::read_style(args.style.as_deref())?;
    let lines = spoken_lines(&intent);
    let first = if args.tts_model == "auto" {
        auto_model()
    } else {
        TtsModel::parse(&args.tts_model).with_context(|| {
            format!(
                "unknown --tts-model '{}' (auto, fish, say, gemini, mai or deepgram)",
                args.tts_model
            )
        })?
    };
    // A paid opt-in falls back to the free voice (whole take); free is used as is.
    let mut chain = vec![first];
    if first.is_paid() {
        chain.extend(FREE_FALLBACKS);
    }

    let resolved = resolve_taste(&intent, &style, None);
    let emotion = resolve_emotion(&resolved);
    let profile = profile_for(emotion);
    let mut pauses = pauses_for(resolved.motion.kind);
    // Speech-led beats keep room for their visuals: sentence starts stay at
    // least 0.9× the planned (no-speech) beat spacing apart (single takes cap
    // the added silence at MAX_EXTRA_PAUSE so the read keeps flowing).
    pauses.min_spacings = motion_core::compiler::planned_start_spacing(&intent, &style)
        .into_iter()
        .map(|s| 0.9 * s)
        .collect();
    let cache = match &args.cache {
        Some(dir) => VoiceCache::new(dir),
        None => VoiceCache::with_default_root(),
    };

    let stem = file_stem(&intent.title);
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("creating {}", args.output.display()))?;
    let wav_path = args.output.join(format!("{stem}.voice.wav"));
    let json_path = args.output.join(format!("{stem}.speech.json"));

    let key = config::resolve_key(OPENROUTER, &config::default_config_path()).map(|(k, _)| k);
    // (0.20) Word timing: auto = local (free, on-device) when available, else onset.
    let align = resolve_align(&args)?;
    // Recognised words for a WAV (cached): the paid service or the local model.
    let asr_root = cache.root().join("asr");
    let recognise = |wav_bytes: &[u8]| -> Option<Result<(Vec<AsrWord>, bool), VoiceError>> {
        match &align {
            Align::Onset => None,
            Align::Asr => Some(motion_voice::asr::transcribe(
                wav_bytes,
                "en",
                key.as_deref(),
                &motion_voice::UreqTransport,
                &asr_root,
                args.offline,
            )),
            Align::Local {
                spec,
                params,
                models,
            } => Some(local_words(wav_bytes, spec, models, params, &asr_root)),
        }
    };
    // Word-level cuts for the single take from recognised words: each cut is
    // the quietest 40 ms between a line's last word and the next line's first.
    let spoken_words: Vec<Vec<String>> = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| statement_words(l))
        .collect();
    let finder = |body: &[i16]| -> Option<Vec<f64>> {
        let (words, _) = recognise(&wav::encode(body))?.ok()?;
        let gaps = motion_voice::asr::line_gaps(&spoken_words, &words)?;
        Some(snap_cuts_to_silence(body, motion_voice::SAMPLE_RATE, &gaps))
    };

    let mut last_err: Option<VoiceError> = None;
    let mut done = None;
    for (i, &model) in chain.iter().enumerate() {
        let mut voice = voice_for(profile, model);
        if i == 0 {
            if let Some(v) = &args.voice {
                voice.voice = v.clone();
            }
        }
        println!(
            "voice: model {} voice '{}' (emotion {}, single take, offline={})",
            model.model_id(),
            voice.voice,
            emotion_name(emotion),
            args.offline
        );
        let provider = provider_for(model);
        // One call for the whole voice-over; the take is split, never re-read.
        let result = build_speech_single_take_with(
            &lines,
            &lines,
            model,
            &voice,
            provider.as_ref(),
            &cache,
            &pauses,
            &wav_path,
            args.offline,
            Some(time_words),
            Some(&finder),
        );
        match result {
            Ok(r) => {
                done = Some(r);
                break;
            }
            Err(e) if retryable(&e) && i + 1 < chain.len() => {
                println!(
                    "{} failed ({e}); falling back to {}",
                    model.model_id(),
                    chain[i + 1].model_id()
                );
                last_err = Some(e);
            }
            Err(VoiceError::MissingKey(p)) => {
                bail!(
                    "no API key for {p}. Set OPENROUTER_API_KEY or run \
                     `motion-engine providers set-key {p}`, or use `--tts-model say` \
                     (macOS, no key needed)."
                )
            }
            Err(e) => return Err(e.into()),
        }
    }
    let Some((map, stats)) = done else {
        return Err(last_err
            .map(anyhow::Error::from)
            .unwrap_or_else(|| anyhow::anyhow!("no voice model succeeded")));
    };
    // Recognised word times (cached by the audio bytes) replace onset
    // estimates where the words align; misses keep their estimates. The take
    // is never re-synthesised: a mismatch is reported for the operator.
    let mut map = map;
    let wav_bytes = std::fs::read(&wav_path)?;
    let t0 = std::time::Instant::now();
    let mut heard: Vec<AsrWord> = Vec::new();
    let mut alignment = Align::Onset.label();
    match recognise(&wav_bytes) {
        None => {}
        Some(Ok((words, hit))) => {
            let n = motion_voice::asr::apply(&mut map.words, &words);
            alignment = align.label();
            let how = if hit {
                format!("{alignment}, cached")
            } else {
                format!("{alignment}, {:.1} s", t0.elapsed().as_secs_f64())
            };
            println!("{}", mismatch_summary(&map, &words, n, &how));
            heard = words;
        }
        // The paid service may be unreachable: keep the onset estimates.
        Some(Err(e)) if matches!(align, Align::Asr) => {
            println!("word timing: onset estimates (speech recognition unavailable: {e})")
        }
        Some(Err(e)) => return Err(e).context("local word timing"),
    }
    // One repair pass against the spoken lines: monotonic times, minimum word
    // length, written forms; unmatched words are reported, never invented.
    let (mut map, report) = speech::repair(&map, &lines);
    println!("{}", repair_summary(&report));
    // What was actually said, and how the times were measured (set after the
    // repair pass, which rebuilds the map without them).
    map.recognised = heard
        .into_iter()
        .map(|w| RecognisedWord {
            word: w.word,
            start: w.start,
            end: w.end,
        })
        .collect();
    map.alignment = Some(alignment);
    let json = serde_json::to_string_pretty(&map)?;
    std::fs::write(&json_path, json + "\n")
        .with_context(|| format!("writing {}", json_path.display()))?;

    println!(
        "{} sentences, {:.2} s | provider calls: {} | cache hits: {}",
        stats.sentences, stats.duration, stats.calls, stats.cache_hits
    );
    println!("wrote {}", display(&wav_path));
    println!("wrote {}", display(&json_path));
    Ok(())
}

/// One-line summary of a repair pass (plus the unmatched words, if any).
pub fn repair_summary(r: &RepairReport) -> String {
    let mut out = format!(
        "speech repair: {} reordered, {} stretched, {} long gaps, {} statement words unmatched, {} spoken words unmatched",
        r.reordered,
        r.stretched,
        r.gaps_clamped,
        r.unmatched_statement_words.len(),
        r.unmatched_spoken_words.len()
    );
    for w in &r.unmatched_statement_words {
        out.push_str(&format!("\n  unmatched statement word: {w}"));
    }
    for w in &r.unmatched_spoken_words {
        out.push_str(&format!("\n  unmatched spoken word: {w}"));
    }
    out
}

/// Load a `.speech.json` and repair it against the beats' spoken lines
/// (`spoken_lines`: narration, else statement) — what `compile --speech`
/// feeds the compiler.
pub fn load_speech(path: &Path, statements: &[String]) -> Result<SpeechMap> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading speech file {}", path.display()))?;
    let map = SpeechMap::from_json(&text)
        .with_context(|| format!("parsing speech file {}", path.display()))?;
    let (map, report) = speech::repair(&map, statements);
    println!("{}", repair_summary(&report));
    Ok(map)
}

fn display(p: &Path) -> String {
    p.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asr_ctc_resolution_names_the_fix() {
        let empty = std::env::temp_dir().join(format!("me_voice_ctc_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&empty);
        assert!(!resolve_ctc("off", &empty).unwrap());
        // auto without the aligner (or its model): whisper's times, no error.
        assert!(!resolve_ctc("auto", &empty).unwrap());
        // on without it: the error names the build flag or the fetch command.
        let err = resolve_ctc("on", &empty).unwrap_err().to_string();
        if asr_ctc::AVAILABLE {
            assert!(err.contains("models fetch wav2vec2-base-960h"), "{err}");
        } else {
            assert!(err.contains("asr-ctc"), "{err}");
        }
        assert!(resolve_ctc("maybe", &empty).is_err());
    }
}
