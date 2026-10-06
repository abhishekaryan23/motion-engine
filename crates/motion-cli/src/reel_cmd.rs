//! (0.10 Q) `motion-engine reel`: the one-command path for weak models.
//!
//! intent (+ taste) → expressive voice-over (single take, word timing from
//! speech recognition) → compile with art direction, story-keyed variety and
//! speech-led timing → story-matched music bed (the mood read from the story's
//! content, 0.23) → speech-aware SFX plan → render (VO on top, the bed under
//! it, SFX −4 dB, −16 LUFS) → speech QA.
//! Every step is the ordinary subcommand (so each stays independently
//! reproducible); this command only chooses good defaults and wires paths.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Args;
use motion_core::audio::{select_bed, story_mood, MusicCatalog, MusicWord};
use motion_core::compiler::direction::take_seed;
use motion_core::compiler::resolve_taste;
use motion_core::compiler::typography::{emotion_name, resolve_emotion};
use motion_core::intent::CreativeIntent;
use motion_core::style::StyleProfile;

#[derive(Args, Debug)]
pub struct ReelArgs {
    /// CreativeIntent JSON.
    pub intent: PathBuf,
    #[arg(long)]
    pub style: Option<PathBuf>,
    /// Output directory (default output/<title>/).
    #[arg(long, short)]
    pub output: Option<PathBuf>,
    /// Voice model: auto (default, the free OpenRouter voice), fish, say, or the
    /// paid opt-ins gemini, mai, deepgram. Always one take in one voice.
    #[arg(long, default_value = "auto")]
    pub tts_model: String,
    /// Music bed: auto (default: the mood read from the story's content picks
    /// a bed of the catalog; none when no bed fits), none, or a bed id from
    /// assets/music/catalog.json.
    #[arg(long, default_value = "auto")]
    pub music: String,
    /// (0.23) Force the music mood with --music auto: auto (default: read it
    /// from the story), none, calm, upbeat, serious, dramatic, playful or
    /// neutral. A word that conflicts with the story's content still plays,
    /// with a `warning[music_fit]` line. Ignored when --music names a bed or
    /// none.
    #[arg(long, default_value = "auto")]
    pub music_mood: String,
    /// (0.23) Choose and print the music (the `[3/5] music:` line, its
    /// `warning[music_fit]` and the MusicPlan path), then stop: no voice, no
    /// compile, no render, nothing written.
    #[arg(long)]
    pub plan_only: bool,
    /// (0.20) Word timing: auto (default: local when the build has
    /// `asr-local` and the model is installed, else onset with a notice),
    /// local, onset or asr (paid).
    #[arg(long, default_value = "auto")]
    pub align: String,
    /// (0.20 A3) CTC refinement of local word times: auto (default: on when
    /// the build has `asr-ctc` and wav2vec2-base-960h is installed, else
    /// whisper's times with a notice), on, or off.
    #[arg(long, default_value = "auto")]
    pub asr_ctc: String,
    /// Art direction: auto (default), none, or a look name.
    #[arg(long, default_value = "auto")]
    pub art: String,
    /// Disable story-keyed variety (palette / layout rotation).
    #[arg(long)]
    pub no_variety: bool,
    /// (0.23) Take: another version of the same story (0 = the default).
    /// Forwarded to `compile --take`; read only with variety (no effect with
    /// --no-variety).
    #[arg(long, default_value_t = 0)]
    pub take: u64,
    /// (0.23) Best-of-N: compile N candidate directions (1..=8), drop any
    /// with a hard QA failure and ship the best by soft score. Forwarded to
    /// `compile --candidates` (only above 1, and only with variety); 1 is a
    /// plain compile.
    #[arg(long, default_value_t = 4)]
    pub candidates: u8,
    /// Leave out the word-synced captions.
    #[arg(long)]
    pub no_captions: bool,
    /// Asset families (repeatable); default: the look's families.
    #[arg(long = "asset-family")]
    pub asset_family: Vec<String>,
    /// Canvas, e.g. 1080x1350 (default: the intent's format).
    #[arg(long)]
    pub canvas: Option<String>,
    /// Aspect preset or a:b (story, portrait, square, landscape, 4:5 …).
    #[arg(long)]
    pub aspect: Option<String>,
    /// Delivered images (AssetManifest JSON) for the compile.
    #[arg(long)]
    pub asset_manifest: Option<PathBuf>,
    /// Asset library root.
    #[arg(long, default_value = "assets")]
    pub assets: PathBuf,
    /// Never call a provider (voice and recognition must be cached).
    #[arg(long)]
    pub offline: bool,
    /// Keep the rendered PNG frames (deleted by default).
    #[arg(long)]
    pub keep_frames: bool,
    /// Run the post effects on the GPU (forwarded to `render --gpu`; needs a
    /// build with `--features gpu`, falls back to the CPU with one line).
    #[arg(long)]
    pub gpu: bool,
    /// (0.21) Brand colours, forwarded to `compile --brand`.
    #[command(flatten)]
    pub brand: crate::BrandArgs,
}

/// Filename-safe stem, the same rule `voice` uses.
fn stem_of(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "reel".to_string()
    } else {
        s
    }
}

/// The bed for an emotion from `assets/music/catalog.json` (`None` when the
/// catalog is missing or has no bed for it).
pub fn bed_for(assets: &Path, emotion: &str) -> Option<(String, PathBuf)> {
    let text = std::fs::read_to_string(assets.join("music/catalog.json")).ok()?;
    let cat: serde_json::Value = serde_json::from_str(&text).ok()?;
    let beds = cat.get("beds")?.as_array()?;
    beds.iter()
        .find(|b| {
            b.get("emotions")
                .and_then(|e| e.as_array())
                .is_some_and(|e| e.iter().any(|x| x.as_str() == Some(emotion)))
        })
        .or_else(|| beds.first())
        .and_then(|b| {
            Some((
                b.get("id")?.as_str()?.to_string(),
                assets.join("music").join(b.get("plan")?.as_str()?),
            ))
        })
}

fn bed_by_id(assets: &Path, id: &str) -> Option<PathBuf> {
    let text = std::fs::read_to_string(assets.join("music/catalog.json")).ok()?;
    let cat: serde_json::Value = serde_json::from_str(&text).ok()?;
    let bed = cat
        .get("beds")?
        .as_array()?
        .iter()
        .find(|b| b.get("id").and_then(|v| v.as_str()) == Some(id))?;
    Some(assets.join("music").join(bed.get("plan")?.as_str()?))
}

/// (0.23) What the music step decided.
#[derive(Debug, Clone, PartialEq)]
pub struct MusicDecision {
    /// The bed's id and MusicPlan path; `None` = no bed.
    pub bed: Option<(String, PathBuf)>,
    /// The text of the `[3/5] music: …` line: the bed and the reason
    /// ("tech_pulse (neutral explainer, money story)"), or "none (…)".
    pub line: String,
    /// The `music_fit` warning message, printed as `warning[music_fit]: …`.
    pub warning: Option<String>,
    /// A `--music-mood` that was given but not read, and why.
    pub note: Option<String>,
}

/// (0.23) Choose the bed. `music` is `--music` (auto, none or a bed id),
/// `mood_word` is `--music-mood`, `seed` rotates among equally fitting beds
/// (the take seed). With `auto` the story's mood (`audio::story_mood`, its
/// tone as the secondary bias) picks the bed through `audio::select_bed`;
/// when no bed fits, there is no bed and the line says so. A catalog without
/// v0.2 mood tags (a v0.1 file) still picks by emotion.
pub fn decide_music(
    assets: &Path,
    intent: &CreativeIntent,
    style: &StyleProfile,
    music: &str,
    mood_word: MusicWord,
    seed: u64,
) -> Result<MusicDecision> {
    let ignored = |why: &str| {
        (mood_word != MusicWord::Auto)
            .then(|| format!("--music-mood {} ignored ({why})", mood_word.as_str()))
    };
    match music {
        "none" => Ok(MusicDecision {
            bed: None,
            line: "none".to_string(),
            warning: None,
            note: ignored("--music none"),
        }),
        "auto" => {
            let catalog_path = assets.join("music/catalog.json");
            let catalog = std::fs::read_to_string(&catalog_path)
                .ok()
                .and_then(|t| MusicCatalog::from_json(&t).ok());
            let Some(catalog) = catalog else {
                return Ok(MusicDecision {
                    bed: None,
                    line: format!("none (no music catalog at {})", catalog_path.display()),
                    warning: None,
                    note: None,
                });
            };
            let taste = resolve_taste(intent, style, None);
            if catalog.beds.iter().all(|b| b.moods.is_empty()) {
                // A v0.1 catalog has no mood tags: choose by emotion as before.
                let emotion = emotion_name(resolve_emotion(&taste));
                let bed = bed_for(assets, emotion);
                let line = match &bed {
                    Some((id, _)) => format!("{id} (emotion {emotion}; catalog without moods)"),
                    None => "none".to_string(),
                };
                return Ok(MusicDecision {
                    bed,
                    line,
                    warning: None,
                    note: ignored("the catalog has no mood tags"),
                });
            }
            let mood = story_mood(intent, &taste);
            let choice = select_bed(&catalog, &mood, mood_word, seed);
            let bed = choice.bed.as_ref().and_then(|id| {
                let b = catalog.beds.iter().find(|b| &b.id == id)?;
                Some((b.id.clone(), assets.join("music").join(&b.plan)))
            });
            // `reason` starts with the bed id when there is a bed.
            let line = if bed.is_some() {
                choice.reason
            } else {
                format!("none ({})", choice.reason)
            };
            Ok(MusicDecision {
                bed,
                line,
                warning: choice.warning,
                note: None,
            })
        }
        id => {
            let plan = bed_by_id(assets, id).with_context(|| {
                format!(
                    "--music '{id}': not in {}",
                    assets.join("music/catalog.json").display()
                )
            })?;
            Ok(MusicDecision {
                bed: Some((id.to_string(), plan)),
                line: format!("{id} (as asked)"),
                warning: None,
                note: ignored(&format!("--music names the bed {id}")),
            })
        }
    }
}

/// The music step's output: the `[3/5]` line, the `music_fit` warning (the
/// same `warning[code]: message` shape as the compile's) and a note when a
/// `--music-mood` was not read.
fn print_music(d: &MusicDecision) {
    println!("[3/5] music: {}", d.line);
    if let Some(w) = &d.warning {
        println!("warning[{}]: {w}", motion_core::compiler::WARN_MUSIC_FIT);
    }
    if let Some(n) = &d.note {
        println!("  {n}");
    }
}

fn run(step: &str, mut cmd: Command, quiet: bool) -> Result<String> {
    let out = cmd
        .output()
        .with_context(|| format!("{step}: cannot start motion-engine"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        bail!(
            "{step} failed:\n{}{}",
            stdout,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    if !quiet {
        for line in stdout.lines() {
            println!("  {line}");
        }
    }
    Ok(stdout)
}

pub fn run_reel(args: ReelArgs) -> Result<()> {
    let exe = std::env::current_exe().context("locating the motion-engine binary")?;
    let intent = crate::read_intent(&args.intent)?;
    let style = crate::read_style(args.style.as_deref())?;
    // A bad --candidates fails before the voice is paid for, like the music.
    if !(1..=crate::MAX_CANDIDATES).contains(&args.candidates) {
        bail!(
            "--candidates {}: expected 1 to {}",
            args.candidates,
            crate::MAX_CANDIDATES
        );
    }

    // The music is a pure function of the story and the take, so it is chosen
    // first: a bad --music or --music-mood fails before the voice is paid for.
    let mood_word = MusicWord::parse(&args.music_mood).with_context(|| {
        format!(
            "--music-mood '{}': expected one of {}",
            args.music_mood,
            MusicWord::ALL.map(MusicWord::as_str).join(", ")
        )
    })?;
    // The seed of the compile's direction (the story-keyed variety seed, then
    // the take); without variety the compile has none, and neither has the bed.
    let seed = if args.no_variety {
        0
    } else {
        take_seed(crate::story_seed(&intent), args.take)
    };
    let decision = decide_music(&args.assets, &intent, &style, &args.music, mood_word, seed)?;
    if args.plan_only {
        print_music(&decision);
        if let Some((_, plan)) = &decision.bed {
            println!("music plan: {}", plan.display());
        }
        return Ok(());
    }

    let stem = stem_of(&intent.title);
    let out = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from("output").join(&stem));
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let speech = out.join(format!("{stem}.speech.json"));
    let scene = out.join(format!("{stem}.motion.json"));
    let audio = out.join(format!("{stem}.audio.json"));
    let sfx = args.assets.join("sfx/library");
    let emotion = emotion_name(resolve_emotion(&resolve_taste(&intent, &style, None))).to_string();

    let style_args = |cmd: &mut Command| {
        if let Some(s) = &args.style {
            cmd.arg("--style").arg(s);
        }
    };

    // 1. Voice-over.
    println!("[1/5] voice ({} · emotion {emotion})", args.tts_model);
    let mut c = Command::new(&exe);
    c.arg("voice")
        .arg(&args.intent)
        .arg("--tts-model")
        .arg(&args.tts_model)
        .arg("--align")
        .arg(&args.align)
        .arg("--asr-ctc")
        .arg(&args.asr_ctc)
        .arg("-o")
        .arg(&out);
    style_args(&mut c);
    if args.offline {
        c.arg("--offline");
    }
    run("voice", c, false)?;

    // 3. Music bed (chosen before the compile: genre looks pulse to it).
    print_music(&decision);
    let music = decision.bed.map(|(_, plan)| plan);

    // 2. Compile.
    println!(
        "[2/5] compile (art {}, variety {})",
        args.art,
        if args.no_variety { "off" } else { "auto" }
    );
    let mut c = Command::new(&exe);
    c.arg("compile")
        .arg(&args.intent)
        .arg("--speech")
        .arg(&speech)
        .arg("--assets")
        .arg(&args.assets)
        .arg("--output")
        .arg(&scene);
    style_args(&mut c);
    if args.art != "none" {
        c.arg("--art").arg(&args.art);
    }
    if !args.no_variety {
        c.arg("--variety").arg("auto");
    }
    // (0.23) Take 0 is the default and adds nothing to the compile command.
    if args.take != 0 {
        c.arg("--take").arg(args.take.to_string());
    }
    // (0.23) Best-of-N needs the variety seed; with --no-variety (or N = 1)
    // the compile is the plain one and gets no flag.
    if !args.no_variety && args.candidates > 1 {
        c.arg("--candidates").arg(args.candidates.to_string());
    }
    if args.no_captions {
        c.arg("--no-captions");
    }
    if let Some(plan) = &music {
        c.arg("--music-energy").arg(plan);
    }
    if let Some(m) = &args.asset_manifest {
        c.arg("--asset-manifest").arg(m);
    }
    for f in &args.asset_family {
        c.arg("--asset-family").arg(f);
    }
    if let Some(cv) = &args.canvas {
        c.arg("--canvas").arg(cv);
    }
    if let Some(a) = &args.aspect {
        c.arg("--aspect").arg(a);
    }
    if let Some(b) = &args.brand.brand {
        c.arg("--brand").arg(b);
    }
    run("compile", c, false)?;

    // 4. Sound design plan + render.
    println!("[4/5] sound design + render");
    let mut c = Command::new(&exe);
    c.arg("plan-audio")
        .arg(&scene)
        .arg("--intent")
        .arg(&args.intent)
        .arg("--sfx-library")
        .arg(&sfx)
        .arg("--speech")
        .arg(&speech)
        .arg("-o")
        .arg(&audio);
    style_args(&mut c);
    if let Some(m) = &music {
        c.arg("--music").arg(m);
    }
    run("plan-audio", c, true)?;
    let mut c = Command::new(&exe);
    c.arg("render")
        .arg(&scene)
        .arg("--speech")
        .arg(&speech)
        .arg("--sfx-library")
        .arg(&sfx)
        .arg("--audio-plan")
        .arg(&audio)
        .arg("--makeup-gain")
        .arg("auto");
    if let Some(m) = &music {
        c.arg("--music").arg(m);
    }
    if args.gpu {
        c.arg("--gpu");
    }
    let rendered = run("render", c, true)?;
    // The render step is quiet; surface its one-line GPU fallback notice.
    for line in rendered.lines().filter(|l| l.starts_with("gpu:")) {
        println!("  {line}");
    }
    let frames_dir = out.join(&intent.title);
    let video = frames_dir.join(format!("{}.mp4", intent.title));
    if !args.keep_frames {
        if let Ok(rd) = std::fs::read_dir(&frames_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "png") {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    }

    // 5. QA.
    println!("[5/5] speech QA");
    let mut c = Command::new(&exe);
    c.arg("qa")
        .arg(&scene)
        .arg("--speech")
        .arg(&speech)
        .arg("--audio-plan")
        .arg(&audio)
        .arg("--mixed")
        .arg(&video);
    let qa = run("qa", c, true)?;
    for line in qa.lines().filter(|l| {
        l.contains("FAIL") || l.starts_with("  max caption") || l.starts_with("speech qa")
    }) {
        println!("  {}", line.trim());
    }
    println!("video: {}", video.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beds_follow_the_emotion_catalog() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        let (id, plan) = bed_for(&assets, "warmth").expect("catalog");
        assert_eq!(id, "warm_piano");
        assert!(plan.is_file());
        assert_eq!(bed_for(&assets, "precision").unwrap().0, "tech_pulse");
        assert!(bed_by_id(&assets, "retro_groove").unwrap().is_file());
        assert!(bed_by_id(&assets, "nope").is_none());
        assert_eq!(stem_of("city water!"), "city_water_");
    }
}
