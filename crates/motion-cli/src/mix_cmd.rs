//! `mix` (0.23): replace the audio of an existing MP4 through the code path
//! `render` uses; the audio-only bench and A/B tool of the voice-relative mix.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Args;

use crate::audio_cmd::{run_mix, MixJob};
use crate::MakeupArg;

#[derive(Args, Debug)]
pub struct MixArgs {
    /// The MP4 whose video stream is kept (copied, not re-encoded). Its own
    /// audio, if any, is dropped.
    pub video: PathBuf,
    /// AudioPlan JSON (`plan-audio`). Without `levels` (a plan made without
    /// `--speech`) the mix is the pre-0.23 one.
    #[arg(long)]
    pub audio_plan: PathBuf,
    /// SpeechMap JSON (`voice`): the voice-over to mix and the word times the
    /// bed follows.
    #[arg(long)]
    pub speech: PathBuf,
    /// MusicPlan JSON (`music-index`): a bed replacing the plan's own.
    #[arg(long)]
    pub music: Option<PathBuf>,
    /// SFX library root. Without one the plan's cues are not mixed (SFX off).
    #[arg(long)]
    pub sfx_library: Option<PathBuf>,
    /// Also write the `voice.wav`, `bed.wav` and `sfx.wav` stems (float wav,
    /// post-gain, post-envelope, pre-makeup) into this directory. Delete them
    /// when done.
    #[arg(long)]
    pub stems: Option<PathBuf>,
    /// The pre-0.23 mix: ignore the plan's `levels` (voice as recorded,
    /// absolute bed, sidechain ducking). The baseline of `mix_bench`.
    #[arg(long)]
    pub legacy: bool,
    /// Loudness makeup: auto (default with a voice-over) or off.
    #[arg(long, value_enum)]
    pub makeup_gain: Option<MakeupArg>,
    /// The compiled MotionScene JSON the video was rendered from. The mix is
    /// then exactly as long as `render` mixes it (`project.duration_seconds()`),
    /// so `mix` and `mix --legacy` are the graphs `render` runs. Without it
    /// (or `--duration`) the length is the video stream's, read by ffprobe,
    /// which can differ from the project's by up to a frame (26.900 s vs
    /// 26.867 s) and so is not sample-identical to render's mix.
    #[arg(long, conflicts_with = "duration")]
    pub scene: Option<PathBuf>,
    /// Mix length in seconds, overriding the video stream's (see `--scene`).
    #[arg(long)]
    pub duration: Option<f64>,
    /// The MP4 to write.
    #[arg(long, short)]
    pub output: PathBuf,
}

/// Duration (s) of the video stream of `path` via ffprobe.
fn video_duration(path: &Path) -> Result<f64> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .context("cannot run ffprobe (is ffmpeg installed?)")?;
    if !out.status.success() {
        bail!(
            "ffprobe {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let d: f64 = text
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .parse()
        .with_context(|| format!("{}: no video stream duration", path.display()))?;
    if !(d.is_finite() && d > 0.0) {
        bail!("{}: video stream duration {d}", path.display());
    }
    Ok(d)
}

pub fn cmd_mix(args: &MixArgs) -> Result<()> {
    if args.output == args.video {
        bail!("mix: -o must not be the input video (write a new file)");
    }
    let duration = match (&args.scene, args.duration) {
        (Some(scene), _) => crate::audio_cmd::load_project(scene)?.duration_seconds(),
        (None, Some(d)) if d.is_finite() && d > 0.0 => d,
        (None, Some(d)) => bail!("mix: --duration must be a positive number of seconds, got {d}"),
        (None, None) => video_duration(&args.video)?,
    };
    let summary = run_mix(&MixJob {
        video: &args.video,
        duration,
        output: &args.output,
        sfx_library: args.sfx_library.as_deref(),
        audio_plan: Some(args.audio_plan.clone()),
        speech: Some(&args.speech),
        music: args.music.as_deref(),
        makeup: args.makeup_gain.map(Into::into),
        legacy: args.legacy,
        stems: args.stems.as_deref(),
    })?;
    println!("{summary} -> {}", args.output.display());
    Ok(())
}
