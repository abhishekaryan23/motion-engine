//! `motion-engine` — compile semantic intent, validate scenes, render video.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use motion_core::assets::{AssetManifest, AssetPlan, AssetPromptSet};
use motion_core::compiler::layout_frame::{preset_size, LayoutFrame};
use motion_core::compiler::typography::{resolve_typography, TypographyRequest};
use motion_core::compiler::FontSet;
use motion_core::compiler::{
    compile_with_report, plan_assets_with_reference, resolve_taste, story_key_of,
};
use motion_core::{validate, AssetLibrary, CreativeIntent, MotionProject, StyleProfile};
use motion_render::asset_qa::{validate_manifest, AssetQaReport};
use motion_render::export::{encode_mp4, render_frames_with, FrameRange, RenderOptions};
use motion_render::ingest::{ingest, IngestOptions};
use motion_render::{
    lifecycle_report, structural_profile, visual_report, CpuRenderer, FontMeasure, MotionProfile,
    SceneQa,
};

mod audio_cmd;
mod explore_cmd;
mod fonts_cmd;
mod matte_cmd;
mod mix_cmd;
mod models_cmd;
mod music_cmd;
mod qa_intel;
mod reel_cmd;
mod reference_cmd;
mod sfx_cmd;
mod style_cmd;
mod voice_cmd;

#[derive(Parser)]
#[command(
    name = "motion-engine",
    version,
    about = "Semantic editorial motion graphics engine"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

/// `render --makeup-gain` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MakeupArg {
    Off,
    Auto,
}

impl From<MakeupArg> for motion_render::audio_mix::MakeupGain {
    fn from(m: MakeupArg) -> Self {
        match m {
            MakeupArg::Off => Self::Off,
            MakeupArg::Auto => Self::Auto,
        }
    }
}

/// (0.9) Canvas override: `--canvas WxH` or `--aspect <preset|a:b>` (exclusive).
/// No flag = the intent's format (byte-identical to 0.8).
#[derive(clap::Args, Debug, Clone, Default)]
pub struct CanvasArgs {
    /// Canvas size in pixels, e.g. 1080x1350 (even sides, short side >= 320,
    /// aspect 0.4..=2.5).
    #[arg(long, conflicts_with = "aspect")]
    pub canvas: Option<String>,
    /// Canvas aspect (short side 1080): story, portrait, square, landscape,
    /// cinema, tall, tablet, fold_inner, fold_cover, or any a:b (e.g. 4:5).
    #[arg(long)]
    pub aspect: Option<String>,
}

impl CanvasArgs {
    /// The requested canvas, validated through `LayoutFrame::new`.
    pub fn resolve(&self) -> Result<Option<(u32, u32)>> {
        let size = match (&self.canvas, &self.aspect) {
            (Some(c), _) => parse_wxh(c)
                .with_context(|| format!("--canvas '{c}': expected WxH, e.g. 1080x1350"))?,
            (None, Some(a)) => preset_size(a).with_context(|| {
                format!(
                    "--aspect '{a}': expected story, portrait, square, landscape, cinema, \
                     tall, tablet, fold_inner, fold_cover or a:b (e.g. 4:5)"
                )
            })?,
            (None, None) => return Ok(None),
        };
        LayoutFrame::new(size.0, size.1).map_err(|e| anyhow::anyhow!("invalid canvas: {e}"))?;
        Ok(Some(size))
    }
}

/// (0.21) Brand colours for this generation (`StyleProfile.brand`).
#[derive(clap::Args, Debug, Clone, Default)]
pub struct BrandArgs {
    /// Brand colours as hex, overriding the style's `brand`: in order
    /// `primary[,secondary[,background[,text]]]` (e.g. `#0A84FF,#FFD60A`), or
    /// named (`primary=#0A84FF,background=#101828`). Text stays readable.
    #[arg(long)]
    pub brand: Option<String>,
}

impl BrandArgs {
    /// Merge `--brand` into `style.brand` (given colours replace the style's).
    pub fn apply(&self, style: &mut StyleProfile) -> Result<()> {
        let Some(spec) = &self.brand else {
            return Ok(());
        };
        const FIELDS: [&str; 4] = ["primary", "secondary", "background", "text"];
        let mut brand = style.brand.clone().unwrap_or_default();
        for (i, item) in spec.split(',').map(str::trim).enumerate() {
            if item.is_empty() {
                continue;
            }
            let (field, value) = match item.split_once('=') {
                Some((k, v)) => (k.trim(), v.trim()),
                None => (*FIELDS.get(i).with_context(|| {
                    format!("--brand: at most 4 colours (primary, secondary, background, text), got '{spec}'")
                })?, item),
            };
            if motion_core::compiler::brand::parse_color(value).is_none() {
                bail!("--brand {field} '{value}': expected a hex colour like #0A84FF");
            }
            let slot = match field {
                "primary" => &mut brand.primary,
                "secondary" => &mut brand.secondary,
                "background" => &mut brand.background,
                "text" => &mut brand.text,
                other => bail!(
                    "--brand: unknown colour '{other}' (primary, secondary, background, text)"
                ),
            };
            *slot = Some(value.to_string());
        }
        style.brand = (!brand.is_empty()).then_some(brand);
        Ok(())
    }
}

/// (0.10 Q) Deterministic variety seed from the story text (FNV-1a over the
/// title and statements): same story → same look, different stories differ.
pub fn story_seed(intent: &motion_core::CreativeIntent) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    eat(&intent.title);
    for b in &intent.beats {
        eat(&b.statement);
    }
    h
}

/// (0.23) The most candidate directions `compile --candidates` accepts.
const MAX_CANDIDATES: u8 = 8;

fn parse_wxh(text: &str) -> Option<(u32, u32)> {
    let lower = text.to_ascii_lowercase();
    let (w, h) = lower.split_once('x')?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile a CreativeIntent (+ StyleProfile) into a MotionScene JSON.
    Compile {
        intent: PathBuf,
        #[arg(long)]
        style: Option<PathBuf>,
        #[arg(long, short)]
        output: PathBuf,
        /// Asset library root (contains fonts/ and library/).
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// Delivered images (AssetManifest JSON; paths relative to the manifest).
        #[arg(long)]
        asset_manifest: Option<PathBuf>,
        /// ReferenceStyleProfile JSON: style principles borrowed from a reference video.
        #[arg(long)]
        reference_style: Option<PathBuf>,
        /// (0.8) Enable an asset family (library/<family>/catalog.json + manifest.json);
        /// repeatable. The planner prefers an exact-word catalog match to a generated image.
        #[arg(long = "asset-family")]
        asset_family: Vec<String>,
        /// (0.8) MusicPlan JSON (`music-index`): handoffs snap to its downbeats.
        #[arg(long)]
        music: Option<PathBuf>,
        /// (0.10) SpeechMap JSON (`voice`): beats follow the spoken sentences,
        /// EVOLVE snaps to word starts, captions are word-synced. Overrides music snapping.
        #[arg(long)]
        speech: Option<PathBuf>,
        /// (0.10 Q) Story-keyed variety: `auto` (seed from the story text) or a
        /// number. Curated palette per look, grammar rotation for repeated
        /// beats, rotating subject-first image layouts. Off = byte-identical.
        #[arg(long)]
        variety: Option<String>,
        /// (0.23) Take: another version of the same story (0 = the story-keyed
        /// default). Read only with --variety; without it the flag is accepted
        /// and ignored (byte-identical).
        #[arg(long, default_value_t = 0)]
        take: u64,
        /// (0.23) Best-of-N: compile N candidate directions of the take (1..=8),
        /// drop any with a hard QA failure and ship the best by soft score. Needs
        /// --variety when above 1; the default 1 is a plain compile.
        #[arg(long, default_value_t = 1)]
        candidates: u8,
        /// (0.10 Q) Art direction: `auto` (look chosen from the emotion the
        /// taste evokes) or classical_neon, halftone_cutout, clay_pop,
        /// ornament_editorial, journey. Curated palette, ground plate, asset
        /// families, SFX palette. Off = byte-identical.
        #[arg(long)]
        art: Option<String>,
        /// (0.10 Q) With --speech, leave out the word-synced captions.
        #[arg(long)]
        no_captions: bool,
        /// (0.14) Music bed (audio file or MusicPlan JSON) whose transient
        /// energy genre looks pulse to.
        #[arg(long)]
        music_energy: Option<PathBuf>,
        #[command(flatten)]
        typography: fonts_cmd::TypographyArgs,
        #[command(flatten)]
        canvas: CanvasArgs,
        #[command(flatten)]
        brand: BrandArgs,
    },
    /// (0.9) Emotion typography registry: list faces/options, render specimens.
    Fonts {
        #[command(subcommand)]
        cmd: fonts_cmd::FontsCmd,
    },
    /// (0.10) TTS provider keys: `set-key`, `show`, `test`.
    Providers {
        #[command(subcommand)]
        cmd: voice_cmd::ProvidersCmd,
    },
    /// (0.10) Synthesize the intent's statements into <title>.voice.wav + <title>.speech.json.
    Voice(voice_cmd::VoiceArgs),
    /// (0.20) Local recogniser models for `voice --align local`: `list`, `fetch`, `path`.
    Models {
        #[command(subcommand)]
        cmd: models_cmd::ModelsCmd,
    },
    /// (0.14) Cut the subject out of photos on-device (Apple Vision, macOS 14+):
    /// one image, a directory, or the opaque hero images of a manifest.
    Matte(matte_cmd::MatteArgs),
    /// (0.10 Q) One command for weak models: voice-over, art-directed compile,
    /// music bed, sound design, render and speech QA.
    Reel(reel_cmd::ReelArgs),
    /// Plan the images a CreativeIntent needs (AssetPlan JSON). Calls no generator.
    PlanAssets {
        intent: PathBuf,
        #[arg(long)]
        style: Option<PathBuf>,
        /// Write the plan here instead of stdout.
        #[arg(long, short)]
        output: Option<PathBuf>,
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// ReferenceStyleProfile JSON: style principles borrowed from a reference video.
        #[arg(long)]
        reference_style: Option<PathBuf>,
        /// (0.8) Enable an asset family (library/<family>/catalog.json + manifest.json);
        /// repeatable. The planner prefers an exact-word catalog match to a generated image.
        #[arg(long = "asset-family")]
        asset_family: Vec<String>,
        /// (0.9) Exploration 0..=3: at >= 2 a tie between families for the best
        /// catalog match is picked by --seed instead of family order.
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
        explore: u8,
        /// (0.9) Exploration seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
    },
    /// Turn an AssetPlan into generator prompts (AssetPromptSet JSON). Calls no generator.
    AssetPrompts {
        plan: PathBuf,
        /// Write the prompts here instead of stdout.
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Ingest the images a generator delivered for a prompt set into an AssetManifest.
    /// Prints the preflight report; exits 1 when any check FAILs.
    IngestAssets {
        prompts: PathBuf,
        delivery_dir: PathBuf,
        /// Manifest to write (paths inside are relative to its directory).
        #[arg(long, short)]
        output: PathBuf,
        /// Fingerprint cache directory (index.json + cached images).
        #[arg(long)]
        cache: Option<PathBuf>,
        /// Spec id or fingerprint whose cached image may be replaced (repeatable).
        #[arg(long)]
        replace: Vec<String>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Re-run preflight QA on an AssetManifest. Exits 1 when any check FAILs.
    ValidateAssets {
        manifest: PathBuf,
        /// The prompt set the manifest was generated for.
        #[arg(long)]
        prompts: Option<PathBuf>,
        /// Print the report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate a MotionScene JSON.
    Validate { scene: PathBuf },
    /// Render a MotionScene to PNG frames and an MP4.
    Render {
        scene: PathBuf,
        /// Output directory (default: <scene dir>/<project name>/).
        #[arg(long)]
        out_dir: Option<PathBuf>,
        /// Render only this frame (no video).
        #[arg(long)]
        frame: Option<u32>,
        /// Skip MP4 encoding.
        #[arg(long)]
        no_video: bool,
        /// (0.22) Keep the PNG frames once the MP4 is encoded (deleted by
        /// default). `--no-video` and `--frame` always keep theirs.
        #[arg(long)]
        keep_frames: bool,
        /// SFX library root (dir with sfx-library.json): mix the AudioPlan into the MP4.
        #[arg(long)]
        sfx_library: Option<PathBuf>,
        /// AudioPlan to mix (default: <scene stem>.audio.json beside the scene).
        #[arg(long)]
        audio_plan: Option<PathBuf>,
        /// Loudness makeup for the mixed audio: off (default) or auto (two-pass
        /// static gain toward the plan's loudness target, under -1 dBTP).
        /// With --speech the default is auto (-16 LUFS).
        #[arg(long, value_enum)]
        makeup_gain: Option<MakeupArg>,
        /// (0.10) SpeechMap JSON (`voice`): mix the voice-over (unducked, on top)
        /// into the MP4. The music bed ducks 10 dB and the SFX bus 4 dB under it.
        /// Works without --sfx-library (voice-over only).
        #[arg(long)]
        speech: Option<PathBuf>,
        /// (0.10) MusicPlan JSON (`music-index`): a music bed under the voice-over
        /// (an AudioPlan's own bed is used otherwise).
        #[arg(long)]
        music: Option<PathBuf>,
        /// Run the post effects (bloom, glitch, grain, ...) on the GPU. Needs a
        /// build with `--features gpu`; falls back to the CPU (one line on
        /// stdout) when the build or machine has no GPU support. Default: CPU.
        #[arg(long)]
        gpu: bool,
    },
    /// Measure curated pack sounds into a self-contained SFX library directory.
    SfxIndex {
        /// Pack root (curation paths are relative to it).
        pack: PathBuf,
        #[arg(long)]
        curation: PathBuf,
        /// Output library directory (sfx-library.json + sounds/).
        #[arg(short, long)]
        output: PathBuf,
        /// Measurement cache directory (default: <output>/.measure-cache).
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Package an SFX library directory as a deterministic add-on zip.
    SfxPack {
        /// Library root (dir with sfx-library.json).
        library: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
    /// Analyse a music track (tempo, beats, downbeats) into a MusicPlan JSON.
    MusicIndex {
        track: PathBuf,
        /// Output MusicPlan (default: <track stem>.music.json beside the track).
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Plan sound-design cues for a compiled scene (writes <name>.audio.json).
    PlanAudio {
        scene: PathBuf,
        /// The CreativeIntent the scene was compiled from (beat energy).
        #[arg(long)]
        intent: PathBuf,
        #[arg(long)]
        style: Option<PathBuf>,
        #[arg(long)]
        reference_style: Option<PathBuf>,
        /// SFX library root (default: $MOTION_SFX_LIBRARY).
        #[arg(long)]
        sfx_library: Option<PathBuf>,
        /// MusicPlan JSON (Phase 3).
        #[arg(long)]
        music: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long)]
        json: bool,
        /// (0.9) Exploration 0..=3: at >= 1 --seed picks sounds within their
        /// families (cue families and times are unchanged).
        #[arg(long, default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=3))]
        explore: u8,
        /// (0.9) Exploration seed.
        #[arg(long, default_value_t = 0)]
        seed: u64,
        /// (0.10) SpeechMap JSON: cues keep 80 ms off every word onset.
        #[arg(long)]
        speech: Option<PathBuf>,
    },
    /// (0.23) Replace the audio of an existing MP4 (video copied) with the
    /// voice-relative mix `render` uses: the bench and A/B tool for the mix.
    Mix(mix_cmd::MixArgs),
    /// (0.9) Compile K exploration variants of one story, render a contact
    /// sheet (one row per variant) and write explore.json.
    Explore(explore_cmd::ExploreArgs),
    /// Print the resolved state of one frame as JSON (debugging).
    Inspect {
        scene: PathBuf,
        #[arg(long)]
        frame: u32,
    },
    /// Render a one-frame preview of what a StyleProfile resolves to and describe it.
    StylePreview {
        style: PathBuf,
        /// PNG to write.
        #[arg(long, short, default_value = "output/style_preview.png")]
        output: PathBuf,
        /// Asset library root (contains fonts/ and library/).
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// Print the ResolvedStyleProfile as JSON instead of the description.
        #[arg(long)]
        json: bool,
    },
    /// Compare two StyleProfiles dimension by dimension (exit code 0 always).
    CompareStyles {
        a: PathBuf,
        b: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Analyse a reference video into an evidence bundle (evidence, samples,
    /// contact sheet, interpreter request/prompt). Calls no model.
    ReferenceEvidence {
        video: PathBuf,
        /// Bundle directory to write.
        #[arg(long)]
        out_dir: PathBuf,
        /// Analysis cache directory (reused across runs for the same video).
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Number of key-frame samples (default: chosen from the duration).
        #[arg(long)]
        samples: Option<usize>,
        /// Print the evidence as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Validate a ReferenceStyleProfile written by an external multimodal model.
    /// Exits 1 when invalid.
    ValidateReferenceStyle {
        profile: PathBuf,
        /// Bundle directory (enables provenance checks against its evidence.json).
        #[arg(long)]
        bundle: Option<PathBuf>,
        /// Write the one permitted repair request here when the profile is invalid.
        #[arg(long)]
        repair_request: Option<PathBuf>,
        /// With --bundle: store the validated profile in this analysis cache.
        #[arg(long)]
        cache_dir: Option<PathBuf>,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show what a StyleProfile (+ optional reference) resolves to, and the coverage.
    ResolveStyle {
        intent: PathBuf,
        #[arg(long)]
        style: Option<PathBuf>,
        #[arg(long)]
        reference_style: Option<PathBuf>,
        /// Asset library root (typography availability check).
        #[arg(long, default_value = "assets")]
        assets: PathBuf,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        typography: fonts_cmd::TypographyArgs,
    },
    /// Structural motion QA: detect static -> spike -> static pacing.
    Qa {
        scene: PathBuf,
        /// Sample every Nth frame (default 1).
        #[arg(long, default_value_t = 1)]
        step: u32,
        /// Print the profile and per-scene lifecycle diagnostics as JSON.
        #[arg(long)]
        json: bool,
        /// ReferenceStyleProfile: compare typography dominance against its visual language.
        #[arg(long)]
        reference_style: Option<PathBuf>,
        /// AudioPlan: add the audio section.
        #[arg(long)]
        audio_plan: Option<PathBuf>,
        /// SFX library root for the audio section (default: $MOTION_SFX_LIBRARY).
        #[arg(long)]
        sfx_library: Option<PathBuf>,
        /// Mixed MP4/audio file: measure peak alignment.
        #[arg(long)]
        mixed: Option<PathBuf>,
        /// (0.9) Layout QA instead of motion QA: text inside the safe area, no
        /// clipped text, minimum type size, subject images on canvas. Evaluated
        /// at each beat's READ time on the scene's own canvas. Exit code 0
        /// even on FAIL (like the rest of `qa`); `--json` prints the report.
        #[arg(long)]
        layout: bool,
        /// (0.10) SpeechMap JSON: SPEECH QA instead of motion QA. Caption timing
        /// (<= 1 frame), safe area, <= 2 lines / 32 chars, layout QA, no SFX
        /// peak within 80 ms of a word onset (needs --audio-plan), and with
        /// --mixed integrated LUFS (-16 +-1), peak <= -1 dBTP and the measured
        /// bed duck. Exit code 0 even on FAIL; `--json` prints the report.
        #[arg(long)]
        speech: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Compile {
            intent,
            style,
            output,
            assets,
            asset_manifest,
            reference_style,
            asset_family,
            music,
            speech,
            variety,
            take,
            candidates,
            art,
            no_captions,
            music_energy,
            typography,
            canvas,
            brand,
        } => cmd_compile(
            &intent,
            style.as_deref(),
            &output,
            &assets,
            asset_manifest.as_deref(),
            reference_style.as_deref(),
            &asset_family,
            music.as_deref(),
            speech.as_deref(),
            variety.as_deref(),
            take,
            candidates,
            art.as_deref(),
            no_captions,
            music_energy.as_deref(),
            &typography,
            &canvas,
            &brand,
        ),
        Cmd::Fonts { cmd } => fonts_cmd::run(cmd),
        Cmd::Providers { cmd } => voice_cmd::run_providers(cmd),
        Cmd::Voice(args) => voice_cmd::run_voice(args),
        Cmd::Models { cmd } => models_cmd::run(cmd),
        Cmd::Matte(args) => matte_cmd::run(args),
        Cmd::Reel(args) => reel_cmd::run_reel(args),
        Cmd::PlanAssets {
            intent,
            style,
            output,
            assets,
            reference_style,
            asset_family,
            explore,
            seed,
        } => {
            let intent = read_intent(&intent)?;
            let style = read_style(style.as_deref())?;
            let library = AssetLibrary::new(
                assets
                    .canonicalize()
                    .with_context(|| format!("asset library '{}' not found", assets.display()))?,
            )
            .with_families(asset_family)
            .with_exploration(explore, seed);
            let reference = reference_style
                .as_deref()
                .map(reference_cmd::load_reference)
                .transpose()?;
            let plan = plan_assets_with_reference(
                &intent,
                &style,
                reference.as_ref().map(|n| &n.principles),
                &library,
            )?;
            let json = plan.to_json_pretty() + "\n";
            match output {
                Some(path) => {
                    std::fs::write(&path, json)?;
                    println!(
                        "planned {} request(s) for {} beat(s) -> {}",
                        plan.requests.len(),
                        plan.beats.len(),
                        path.display()
                    );
                }
                None => print!("{json}"),
            }
            Ok(())
        }
        Cmd::AssetPrompts { plan, output } => cmd_asset_prompts(&plan, output.as_deref()),
        Cmd::IngestAssets {
            prompts,
            delivery_dir,
            output,
            cache,
            replace,
            json,
        } => {
            let opts = IngestOptions {
                cache_dir: cache,
                replace,
            };
            let report = cmd_ingest_assets(&prompts, &delivery_dir, &output, &opts, json)?;
            exit_on_fail(&report)
        }
        Cmd::ValidateAssets {
            manifest,
            prompts,
            json,
        } => {
            let report = cmd_validate_assets(&manifest, prompts.as_deref(), json)?;
            exit_on_fail(&report)
        }
        Cmd::Validate { scene } => {
            let (project, base) = load_scene(&scene)?;
            println!(
                "ok: {} scene(s), {} shared element(s), {:.2}s, {} frames",
                project.scenes.len(),
                project.shared.len(),
                project.duration_seconds(),
                project.frame_count()
            );
            let _ = base;
            Ok(())
        }
        Cmd::Render {
            scene,
            out_dir,
            frame,
            no_video,
            keep_frames,
            sfx_library,
            audio_plan,
            makeup_gain,
            speech,
            music,
            gpu,
        } => {
            let mp4 = cmd_render(&scene, out_dir, frame, no_video, keep_frames, gpu)?;
            match mp4 {
                Some(mp4) => audio_cmd::mix_after_render(
                    &scene,
                    &mp4,
                    &audio_cmd::RenderAudio {
                        sfx_library: sfx_library.as_deref(),
                        audio_plan: audio_plan.as_deref(),
                        makeup: makeup_gain.map(Into::into),
                        speech: speech.as_deref(),
                        music: music.as_deref(),
                    },
                ),
                None => Ok(()),
            }
        }
        Cmd::SfxIndex {
            pack,
            curation,
            output,
            cache,
            json,
        } => sfx_cmd::cmd_sfx_index(&pack, &curation, &output, cache.as_deref(), json),
        Cmd::SfxPack { library, output } => sfx_cmd::cmd_sfx_pack(&library, &output),
        Cmd::MusicIndex {
            track,
            output,
            cache,
            json,
        } => music_cmd::cmd_music_index(&track, output.as_deref(), cache.as_deref(), json),
        Cmd::PlanAudio {
            scene,
            intent,
            style,
            reference_style,
            sfx_library,
            music,
            output,
            json,
            explore,
            seed,
            speech,
        } => audio_cmd::cmd_plan_audio(
            &scene,
            &intent,
            style.as_deref(),
            reference_style.as_deref(),
            sfx_library.as_deref(),
            music.as_deref(),
            output.as_deref(),
            json,
            explore,
            seed,
            speech.as_deref(),
        ),
        Cmd::Mix(args) => mix_cmd::cmd_mix(&args),
        Cmd::Explore(args) => explore_cmd::cmd_explore(&args),
        Cmd::Inspect { scene, frame } => {
            let (project, _) = load_scene(&scene)?;
            let resolved = motion_core::evaluate_frame(&project, frame)?;
            println!("{}", serde_json::to_string_pretty(&resolved)?);
            Ok(())
        }
        Cmd::StylePreview {
            style,
            output,
            assets,
            json,
        } => style_cmd::cmd_style_preview(&style, &output, &assets, json),
        Cmd::CompareStyles { a, b, json } => style_cmd::cmd_compare_styles(&a, &b, json),
        Cmd::ReferenceEvidence {
            video,
            out_dir,
            cache_dir,
            samples,
            json,
        } => reference_cmd::cmd_reference_evidence(
            &video,
            &out_dir,
            cache_dir.as_deref(),
            samples,
            json,
        ),
        Cmd::ValidateReferenceStyle {
            profile,
            bundle,
            repair_request,
            cache_dir,
            json,
        } => reference_cmd::cmd_validate_reference_style(
            &profile,
            bundle.as_deref(),
            repair_request.as_deref(),
            cache_dir.as_deref(),
            json,
        ),
        Cmd::ResolveStyle {
            intent,
            style,
            reference_style,
            assets,
            json,
            typography,
        } => reference_cmd::cmd_resolve_style(
            &intent,
            style.as_deref(),
            reference_style.as_deref(),
            json,
            &typography.options(),
            &assets,
        ),
        Cmd::Qa {
            scene,
            step,
            json,
            reference_style,
            audio_plan,
            sfx_library,
            mixed,
            layout,
            speech,
        } => {
            if let Some(speech) = speech.as_deref() {
                let (project, base) = load_scene(&scene)?;
                let report = audio_cmd::speech_qa(
                    &project,
                    &base,
                    speech,
                    audio_plan.as_deref(),
                    mixed.as_deref(),
                )?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", report.to_text());
                }
                return Ok(());
            }
            if layout {
                let (project, base) = load_scene(&scene)?;
                let frame = LayoutFrame::new(project.canvas.width, project.canvas.height)
                    .map_err(|e| anyhow::anyhow!("scene canvas: {e}"))?;
                // (0.10 Q) Subject checks (text over subjects, subject size,
                // loose frames, low contrast) judge each drawn image by its
                // measured alpha bounds, head region and mean colour.
                let images = motion_render::image_index(&project, &base);
                let mut report = motion_core::layout_report_with(&project, &frame, &images);
                // (0.23) The rendered check: text against the pixels around it.
                match motion_render::contrast_qa::text_local_contrast(&project, &base) {
                    Ok(contrast) => report.findings.extend(contrast.layout_findings()),
                    Err(e) => report.findings.push(motion_core::layout_qa::LayoutFinding {
                        scene: "-".to_string(),
                        layer: "-".to_string(),
                        check: motion_core::layout_qa::LayoutCheck::Evaluation,
                        detail: format!(
                            "text_local_contrast could not render the READ frames: {e}"
                        ),
                    }),
                }
                if !report.findings.is_empty() {
                    report.verdict = motion_core::LayoutVerdict::Fail;
                }
                if json {
                    println!("{}", serde_json::to_string_pretty(&report)?);
                } else {
                    print!("{}", report.to_text());
                }
                return Ok(());
            }
            let weight = reference_style
                .as_deref()
                .map(reference_cmd::load_reference)
                .transpose()?
                .map(|n| n.principles.visual.weight());
            let (project, base) = load_scene(&scene)?;
            let renderer = CpuRenderer::new(&project, &base)?;
            let profile = structural_profile(&project, &renderer, step)?;
            let scenes = lifecycle_report(&project, &profile);
            let visual = visual_report(&project, weight);
            // (0.23) Story checks on the timeline: no speech map here, so a
            // count is anchored on READ (or its own start). (0.23 W8) With
            // `carry_continuity`: a carried picture stays readable across
            // the handoff.
            let story = motion_render::story_qa::motion_checks(&project, None)
                .map_err(|e| anyhow::anyhow!("story checks: {e}"))?;
            if json {
                let mut value = qa_json(&profile, &scenes);
                value["visual"] = serde_json::to_value(&visual)?;
                value["story"] = serde_json::to_value(&story)?;
                if let Some(plan) = audio_plan.as_deref() {
                    value["audio"] = audio_cmd::audio_qa_json(
                        &project,
                        plan,
                        sfx_library.as_deref(),
                        mixed.as_deref(),
                    )?;
                }
                println!("{}", serde_json::to_string_pretty(&value)?);
                return Ok(());
            }
            print!("{}", profile.report());
            println!("\nlifecycle");
            for s in &scenes {
                print!("{}", s.report());
            }
            let warnings: usize = scenes.iter().map(|s| s.warnings.len()).sum();
            println!("lifecycle: {} scenes, {warnings} warnings", scenes.len());
            print!("{}", visual.to_text());
            println!("story");
            for c in &story {
                println!("  {} {}: {}", c.status.label(), c.name, c.detail);
            }
            if let Some(plan) = audio_plan.as_deref() {
                print!(
                    "{}",
                    audio_cmd::audio_qa_text(
                        &project,
                        plan,
                        sfx_library.as_deref(),
                        mixed.as_deref()
                    )?
                );
            }
            Ok(())
        }
    }
}

/// Exit code 1 when the report has a FAIL (after the report was printed).
fn exit_on_fail(report: &AssetQaReport) -> Result<()> {
    if report.has_fail() {
        std::process::exit(1);
    }
    Ok(())
}

fn dir_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

fn print_asset_report(report: &AssetQaReport, json: bool) -> Result<()> {
    if json {
        let value = serde_json::json!({ "ok": !report.has_fail(), "assets": report.assets });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        print!("{}", report.report());
    }
    Ok(())
}

fn cmd_asset_prompts(plan: &Path, output: Option<&Path>) -> Result<()> {
    let plan: AssetPlan = serde_json::from_str(&read(plan)?)
        .with_context(|| format!("parsing asset plan {}", plan.display()))?;
    let prompts = motion_core::asset_prompts::prompt_specs(&plan);
    let json = prompts.to_json_pretty() + "\n";
    match output {
        Some(path) => {
            std::fs::write(path, json)?;
            println!(
                "derived {} generator prompt(s) from {} request(s) -> {}",
                prompts.specs.len(),
                plan.requests.len(),
                path.display()
            );
        }
        None => print!("{json}"),
    }
    Ok(())
}

fn cmd_ingest_assets(
    prompts_path: &Path,
    delivery_dir: &Path,
    output: &Path,
    opts: &IngestOptions,
    json: bool,
) -> Result<AssetQaReport> {
    let prompts = AssetPromptSet::from_json(&read(prompts_path)?)
        .with_context(|| format!("parsing prompts {}", prompts_path.display()))?;
    if !delivery_dir.is_dir() {
        bail!("delivery directory '{}' not found", delivery_dir.display());
    }
    let manifest_dir = dir_of(output);
    let outcome = ingest(&prompts, delivery_dir, &manifest_dir, opts)?;
    // Written even when checks FAIL: `missing` records what is unusable.
    std::fs::write(
        output,
        serde_json::to_string_pretty(&outcome.manifest)? + "\n",
    )?;
    print_asset_report(&outcome.report, json)?;
    if !json {
        println!(
            "manifest: {} ({} image(s), {} missing)",
            output.display(),
            outcome.manifest.assets.len(),
            outcome.manifest.missing.len()
        );
    }
    Ok(outcome.report)
}

fn cmd_validate_assets(
    manifest_path: &Path,
    prompts_path: Option<&Path>,
    json: bool,
) -> Result<AssetQaReport> {
    let manifest = AssetManifest::from_json(&read(manifest_path)?)
        .with_context(|| format!("parsing asset manifest {}", manifest_path.display()))?;
    let prompts = match prompts_path {
        Some(p) => Some(
            AssetPromptSet::from_json(&read(p)?)
                .with_context(|| format!("parsing prompts {}", p.display()))?,
        ),
        None => None,
    };
    let report = validate_manifest(&manifest, &dir_of(manifest_path), prompts.as_ref());
    print_asset_report(&report, json)?;
    Ok(report)
}

/// JSON view of the QA results (built here so motion-render needs no serde).
fn qa_json(profile: &MotionProfile, scenes: &[SceneQa]) -> serde_json::Value {
    use serde_json::json;
    let clusters: Vec<_> = profile
        .spike_clusters
        .iter()
        .map(|c| {
            json!({
                "start": c.start, "end": c.end, "peak_frame": c.peak_frame,
                "peak_diff": c.peak_diff, "peak_multiple": c.peak_multiple,
            })
        })
        .collect();
    let scene_json: Vec<_> = scenes
        .iter()
        .map(|s| {
            let phases: Vec<_> = s
                .phases
                .iter()
                .map(|p| {
                    json!({
                        "phase": p.phase.name(), "start_frame": p.start_frame,
                        "end_frame": p.end_frame, "mean": p.mean, "max": p.max,
                        "accum": p.accum, "status": p.status.name(),
                    })
                })
                .collect();
            json!({
                "scene": s.scene, "phases": phases, "read_activity": s.read_activity,
                "evolve_events": s.evolve_events,
                "longest_static_hold": s.longest_static_hold,
                "spike_ratio": s.spike_ratio, "spike": s.spike.name(),
                "thirds": s.thirds, "warnings": s.warnings,
            })
        })
        .collect();
    json!({
        "profile": {
            "fps": profile.fps, "step": profile.step, "diffs": profile.diffs,
            "median": profile.median, "p95": profile.p95, "max": profile.max,
            "spikes": profile.spikes, "spike_clusters": clusters,
            "static_runs": profile.static_runs, "verdict": profile.verdict,
            "accum": profile.accum,
        },
        "scenes": scene_json,
    })
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

fn read_intent(path: &Path) -> Result<CreativeIntent> {
    CreativeIntent::from_json(&read(path)?)
        .with_context(|| format!("parsing intent {}", path.display()))
}

fn read_style(path: Option<&Path>) -> Result<StyleProfile> {
    Ok(match path {
        Some(p) => StyleProfile::from_json(&read(p)?)
            .with_context(|| format!("parsing style {}", p.display()))?,
        None => StyleProfile::default(),
    })
}

/// Load a manifest and rewrite its paths from manifest-relative to
/// library-root-relative (what the compiler expects).
fn read_manifest(path: &Path, library_root: &Path) -> Result<AssetManifest> {
    let mut manifest = AssetManifest::from_json(&read(path)?)
        .with_context(|| format!("parsing asset manifest {}", path.display()))?;
    manifest.validate().map_err(|e| {
        anyhow::anyhow!(
            "invalid asset manifest {}:\n{}",
            path.display(),
            e.join("\n")
        )
    })?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()?;
    for entry in &mut manifest.assets {
        let abs = dir.join(&entry.path);
        let abs = abs
            .canonicalize()
            .with_context(|| format!("asset '{}' not found at {}", entry.id, abs.display()))?;
        // (0.14) Hand-written manifests often carry no analysis: measure the
        // picture (subject bounds, coverage, mean colour, head estimate) so
        // layouts follow what it depicts, not its transparent margins.
        if entry.analysis.is_none() {
            if let Ok(img) = motion_render::decode::decode_file(&abs) {
                let person = ["hero_subject", "portrait"]
                    .iter()
                    .any(|r| entry.id.ends_with(r));
                let opts = motion_render::analysis::AnalyzeOptions {
                    person,
                    ..Default::default()
                };
                entry.analysis = Some(motion_render::analysis::analyze(&img.pixmap, &opts));
            }
        }
        let rel = pathdiff::diff_paths(&abs, library_root).unwrap_or(abs);
        entry.path = rel.to_string_lossy().replace('\\', "/");
    }
    Ok(manifest)
}

#[allow(clippy::too_many_arguments)]
fn cmd_compile(
    intent_path: &Path,
    style_path: Option<&Path>,
    output: &Path,
    assets: &Path,
    manifest_path: Option<&Path>,
    reference_path: Option<&Path>,
    asset_families: &[String],
    music_path: Option<&Path>,
    speech_path: Option<&Path>,
    variety: Option<&str>,
    take: u64,
    candidates: u8,
    art: Option<&str>,
    no_captions: bool,
    music_energy: Option<&Path>,
    typography: &fonts_cmd::TypographyArgs,
    canvas: &CanvasArgs,
    brand: &BrandArgs,
) -> Result<()> {
    if !(1..=MAX_CANDIDATES).contains(&candidates) {
        bail!("--candidates {candidates}: expected 1 to {MAX_CANDIDATES}");
    }
    if candidates > 1 && variety.is_none() {
        bail!(
            "--candidates {candidates} needs --variety: the candidates are seeded directions \
             of the story's variety seed (use --variety auto)"
        );
    }
    let canvas = canvas.resolve()?;
    let intent = read_intent(intent_path)?;
    let music = music_path.map(audio_cmd::load_music_plan).transpose()?;
    let mut style = read_style(style_path)?;
    brand.apply(&mut style)?;
    let reference = reference_path
        .map(reference_cmd::load_reference)
        .transpose()?;
    let principles = reference.as_ref().map(|n| &n.principles);
    if !assets.is_dir() {
        bail!(
            "asset library '{}' not found (use --assets)",
            assets.display()
        );
    }
    let assets_abs = assets.canonicalize()?;
    let library = AssetLibrary::new(&assets_abs).with_families(asset_families.to_vec());

    // Measure with the exact fonts (and load order) the renderer will use.
    let mut opts = typography.options();
    opts.canvas = canvas;
    opts.no_captions = no_captions;
    opts.art = match art {
        None => None,
        Some("auto") => Some(motion_core::compiler::art_direction::ArtMode::Auto),
        Some(name) => Some(motion_core::compiler::art_direction::ArtMode::Force(
            motion_core::compiler::art_direction::Look::parse(name).with_context(|| {
                format!(
                    "--art '{name}': expected auto, classical_neon, halftone_cutout, clay_pop, \
                     ornament_editorial, journey, street_collage, dossier or hype_slam"
                )
            })?,
        )),
    };
    opts.variety = match variety {
        None => None,
        Some("auto") => Some(story_seed(&intent)),
        Some(n) => Some(
            n.parse::<u64>()
                .with_context(|| format!("--variety '{n}': expected auto or a number"))?,
        ),
    };
    // (0.23) The take reaches the compiler only under a variety seed; a take
    // other than 0 is confirmed in one stdout line (default output unchanged).
    opts.take = take;
    if take != 0 {
        if opts.variety.is_some() {
            println!("take {take}");
        } else {
            println!("take {take} ignored (no --variety)");
        }
    }
    if let Some(path) = speech_path {
        let statements = voice_cmd::spoken_lines(&intent);
        opts.speech = Some(voice_cmd::load_speech(path, &statements)?);
    }
    if let Some(path) = music_energy {
        // A MusicPlan names its track relative to the plan file.
        let audio = if path.extension().is_some_and(|e| e == "json") {
            let plan = audio_cmd::load_music_plan(path)?;
            path.parent().unwrap_or(Path::new(".")).join(&plan.track)
        } else {
            path.to_path_buf()
        };
        opts.music_envelope = Some(
            motion_render::music::energy_envelope(&audio, 30.0)
                .with_context(|| format!("measuring music energy of {}", audio.display()))?,
        );
    }
    let fonts = if typography.is_active() {
        // (0.9) The registry may pick the faces: measure with exactly those.
        let taste = resolve_taste(&intent, &style, principles);
        resolve_typography(
            &taste,
            &TypographyRequest {
                exploration: opts.explore,
                seed: opts.seed,
                story_key: story_key_of(&intent),
                emotion: opts.emotion,
            },
            &assets_abs,
        )
        .1
    } else {
        match principles {
            Some(_) => FontSet::for_pairing(resolve_taste(&intent, &style, principles).typography),
            None => FontSet::for_style(&style),
        }
    };
    let font_paths: Vec<PathBuf> = fonts
        .faces
        .iter()
        .map(|f| assets_abs.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path))?;

    let manifest = match manifest_path {
        Some(p) => read_manifest(p, &assets_abs)?,
        None => AssetManifest::empty(),
    };
    // (0.23) The best-of-N summary line and the shipped-with-failure notice.
    let mut best_lines: Vec<String> = Vec::new();
    let (mut project, warnings) = if candidates > 1 {
        // (0.23) Best-of-N: the candidates compile in parallel, each with its
        // own font measure (the measure is not shareable across threads).
        opts.candidates = candidates;
        let out_dir = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(out_dir)?;
        let out_abs = out_dir.canonicalize()?;
        let asset_root = pathdiff::diff_paths(&assets_abs, &out_abs)
            .unwrap_or(assets_abs.clone())
            .to_string_lossy()
            .replace('\\', "/");
        let take_seed = motion_core::compiler::direction::take_seed(
            opts.variety.unwrap_or_default(),
            opts.take,
        );
        let input = motion_render::score::BestOfInput {
            candidates,
            take_seed,
            speech: opts.speech.as_ref(),
            base_dir: &out_abs,
            asset_root: &asset_root,
        };
        let result = motion_render::score::best_of(&input, |k| {
            let m = FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path))
                .map_err(|e| motion_core::CompileError::Invalid(format!("fonts: {e}")))?;
            motion_core::compiler::compile_candidate(
                &intent,
                &style,
                principles,
                &library,
                &m,
                &manifest,
                music.as_ref(),
                &opts,
                k,
            )
        })?;
        best_lines.push(result.summary());
        if let Some(why) = &result.shipped_with_failure {
            best_lines.push(format!(
                "warning[shipped_with_failure]: {why} (shipped candidate {})",
                result.chosen
            ));
        }
        (result.project, result.warnings)
    } else {
        compile_with_report(
            &intent,
            &style,
            principles,
            &library,
            &measure,
            &manifest,
            music.as_ref(),
            &opts,
        )?
    };

    let out_dir = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(out_dir)?;
    let out_abs = out_dir.canonicalize()?;
    let rel = pathdiff::diff_paths(&assets_abs, &out_abs).unwrap_or(assets_abs.clone());
    project.asset_root = Some(rel.to_string_lossy().replace('\\', "/"));

    validate(&project, Some(&out_abs))
        .map_err(|e| anyhow::anyhow!("compiled scene failed validation:\n{e}"))?;
    std::fs::write(output, project.to_json_pretty() + "\n")?;
    println!(
        "compiled {} beat(s) -> {} ({} scenes, {} shared, {:.2}s)",
        intent.beats.len(),
        output.display(),
        project.scenes.len(),
        project.shared.len(),
        project.duration_seconds()
    );
    // (0.23) What best-of-N chose, and (never silent) when nothing was clean.
    for line in &best_lines {
        println!("{line}");
    }
    // (0.20) What the compiler dropped or could not tie together; stdout, so
    // `reel` (which echoes the compile step's output) shows it too.
    for w in &warnings {
        println!("{}", format_warning(w));
    }
    Ok(())
}

/// `warning[<code>]: beat <N>: <message>` (1-based beat), or
/// `warning[<code>]: <message>` for a warning about the whole story.
fn format_warning(w: &motion_core::compiler::CompileWarning) -> String {
    match w.beat {
        Some(b) => format!("warning[{}]: beat {}: {}", w.code, b + 1, w.message),
        None => format!("warning[{}]: {}", w.code, w.message),
    }
}

fn load_scene(path: &Path) -> Result<(MotionProject, PathBuf)> {
    let project = MotionProject::from_json(&read(path)?)
        .with_context(|| format!("parsing scene {}", path.display()))?;
    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    validate(&project, Some(&base)).map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
    Ok((project, base))
}

fn cmd_render(
    scene: &Path,
    out_dir: Option<PathBuf>,
    frame: Option<u32>,
    no_video: bool,
    keep_frames: bool,
    gpu: bool,
) -> Result<Option<PathBuf>> {
    let (project, base) = load_scene(scene)?;
    let out_dir = out_dir.unwrap_or_else(|| base.join(&project.project.name));
    let started = Instant::now();
    let range = match frame {
        Some(f) => FrameRange::Only(f),
        None => FrameRange::All,
    };
    let report = render_frames_with(
        &project,
        &base,
        &out_dir,
        range,
        RenderOptions { gpu_post: gpu },
    )?;
    if let Some(notice) = &report.gpu_notice {
        println!("{notice}");
    }
    let written = report.frames;
    println!(
        "rendered {} frame(s) to {} in {:.1}s",
        written.len(),
        out_dir.display(),
        started.elapsed().as_secs_f32()
    );
    if frame.is_none() && !no_video {
        let mp4 = out_dir.join(format!("{}.mp4", project.project.name));
        encode_mp4(&out_dir, project.canvas.fps, &mp4)?;
        println!("encoded {}", mp4.display());
        // (0.22) The frames are 1-4 MB each and the MP4 now holds them.
        if !keep_frames {
            for path in &written {
                let _ = std::fs::remove_file(path);
            }
        }
        return Ok(Some(mp4));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brand(spec: &str, style: &str) -> Result<StyleProfile> {
        let mut s: StyleProfile = serde_json::from_str(style).unwrap();
        BrandArgs {
            brand: Some(spec.into()),
        }
        .apply(&mut s)?;
        Ok(s)
    }

    #[test]
    fn brand_flag_positional_named_and_merged() {
        let s = brand("#0A84FF,#FFD60A", "{}").unwrap();
        let b = s.brand.unwrap();
        assert_eq!(b.primary.as_deref(), Some("#0A84FF"));
        assert_eq!(b.secondary.as_deref(), Some("#FFD60A"));
        assert_eq!(b.background, None);
        // Named colours merge into the style's own brand.
        let s = brand("background=#101828", r##"{"brand":{"primary":"#FF375F"}}"##).unwrap();
        let b = s.brand.unwrap();
        assert_eq!(b.primary.as_deref(), Some("#FF375F"));
        assert_eq!(b.background.as_deref(), Some("#101828"));
        for bad in ["blue", "#12345", "logo=#FFFFFF", "#111,#222,#333,#444,#555"] {
            assert!(brand(bad, "{}").is_err(), "{bad}");
        }
    }
}
