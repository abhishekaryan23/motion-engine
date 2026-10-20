//! (0.24 E2) Cutaway clips for `motion-engine edit`: each planned cutaway is
//! compiled from its generated CreativeIntent beats by the ordinary compiler,
//! with the speaker's own words (rebased to the cutaway's start) as the
//! SpeechMap, so every reveal locks to what is said. The result is rendered
//! to a full-frame clip that the segment renderer lays over the source video.
//! No semantic logic beyond assembling the intent: the planner decided what.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{ground, CompileOptions, FontSet};
use motion_core::edit::{CutawayKind, EditEvent, EventKind, Tok};
use motion_core::intent::{Beat, Format};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::{AssetLibrary, CreativeIntent, StyleProfile};
use motion_render::export::{render_to_mp4_stream_quality, Quality, RenderOptions};
use motion_render::FontMeasure;

use crate::edit_render::CutClip;

/// The compile inputs shared by every cutaway of one video.
pub struct CutCtx<'a> {
    pub style: &'a StyleProfile,
    pub assets: &'a Path,
    pub canvas: (u32, u32),
    pub fps: u32,
    /// Where the clips are written.
    pub dir: &'a Path,
    pub seed: u64,
    /// Stable name of the talk (the story key of every cutaway derives from it).
    pub talk: &'a str,
}

/// The one backdrop family of the video: a pure function of the talk's name
/// and the style, then pinned by avoiding the other three.
pub fn video_family(ctx: &CutCtx) -> ground::Family {
    let title = ctx.talk.to_string();
    let probe = CreativeIntent {
        version: motion_core::intent::INTENT_VERSION.to_string(),
        title,
        format: Format::Landscape,
        beats: Vec::new(),
    };
    motion_core::compiler::backdrop_family(&probe, ctx.style, None, &[])
}

fn strip(raw: &str) -> String {
    raw.trim_matches(|c: char| !(c.is_alphanumeric() || "₹$€£%".contains(c)))
        .to_string()
}

fn intent_of(idx: usize, beats: &[Beat], canvas: (u32, u32)) -> CreativeIntent {
    CreativeIntent {
        version: motion_core::intent::INTENT_VERSION.to_string(),
        title: format!("cut{idx}"),
        format: if canvas.0 >= canvas.1 {
            Format::Landscape
        } else {
            Format::Vertical
        },
        beats: beats.to_vec(),
    }
}

/// The speaker's words over the cutaway as a SpeechMap starting at 0.
fn mini_speech(ev: &EditEvent, beats: &[(usize, usize)], toks: &[Tok]) -> SpeechMap {
    let t0 = ev.start;
    let a = beats.first().map(|b| b.0).unwrap_or(ev.at_word);
    let z = beats.last().map(|b| b.1).unwrap_or(ev.until_word);
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: String::new(),
        sample_rate: 16_000,
        duration: ev.end - ev.start,
        provider: "local".into(),
        model: "edit".into(),
        voice: String::new(),
        words: (a..=z)
            .map(|i| SpeechWord {
                text: strip(&toks[i].raw),
                start: (toks[i].start - t0).max(0.0),
                end: (toks[i].end - t0).max(0.0),
                confidence: 1.0,
            })
            .collect(),
        sentences: beats
            .iter()
            .enumerate()
            .map(|(k, &(f, l))| SpeechSentence {
                beat: k,
                start: (toks[f].start - t0).max(0.0),
                end: (toks[l].end - t0).max(0.0),
            })
            .collect(),
        recognised: Vec::new(),
        alignment: None,
    }
}

/// Words of the compiled cutaway's beats that never showed up in its pictures
/// are the compiler's `value_dropped` warnings; returned for the report.
pub struct Built {
    pub clip: CutClip,
    pub warnings: Vec<String>,
    /// `(spoken word start, shown-at)` for each speech-cued group (s, scene time).
    pub cues: Vec<(String, f64)>,
}

/// Compile and render the cutaway `ev` (plan event `idx`).
pub fn build(
    ctx: &CutCtx,
    idx: usize,
    ev: &EditEvent,
    toks: &[Tok],
    family: ground::Family,
) -> Result<Built> {
    let EventKind::Cutaway { sub, beats } = &ev.kind else {
        anyhow::bail!("event {idx} is not a cutaway");
    };
    let intent = intent_of(
        idx,
        &beats.iter().map(|b| b.beat.clone()).collect::<Vec<_>>(),
        ctx.canvas,
    );
    let ranges: Vec<(usize, usize)> = beats.iter().map(|b| (b.first, b.last)).collect();
    let statements = intent
        .beats
        .iter()
        .map(|b| motion_core::compiler::taste_rules::display_text(b, true).spoken)
        .collect::<Vec<_>>();
    let (speech, _) = motion_core::speech::repair(&mini_speech(ev, &ranges, toks), &statements);

    let assets_abs = ctx.assets.canonicalize()?;
    let library = AssetLibrary::new(&assets_abs);
    let fonts = FontSet::for_style(ctx.style);
    let paths: Vec<PathBuf> = fonts
        .faces
        .iter()
        .map(|f| assets_abs.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(paths.iter().map(PathBuf::as_path))
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    // One backdrop family for the video: avoid the other three.
    let avoid: Vec<String> = ground::FAMILIES
        .iter()
        .filter(|f| **f != family)
        .map(|f| f.name().to_string())
        .collect();
    let opts = CompileOptions {
        speech: Some(speech),
        variety: Some(ctx.seed),
        backdrop_avoid: avoid,
        canvas: Some(ctx.canvas),
        art: match sub {
            CutawayKind::QuoteCard => Some(ArtMode::Force(Look::Dossier)),
            _ => None,
        },
        ..CompileOptions::default()
    };
    let (mut project, warnings) = motion_core::compiler::compile_with_report(
        &intent,
        ctx.style,
        None,
        &library,
        &measure,
        &motion_core::assets::AssetManifest::empty(),
        None,
        &opts,
    )
    .map_err(|e| anyhow::anyhow!("compile: {e}"))?;
    project.asset_root = Some(assets_abs.to_string_lossy().into_owned());
    project.canvas.fps = ctx.fps;
    let out_dir = ctx.dir.canonicalize()?;
    motion_render::guard_pictures(&mut project, &out_dir);
    motion_core::validate(&project, Some(&out_dir))
        .map_err(|e| anyhow::anyhow!("cutaway {idx} invalid: {e}"))?;

    let cues = project
        .project
        .speech
        .as_ref()
        .map(|r| r.word_cues.iter().map(|c| (c.word.clone(), c.to)).collect())
        .unwrap_or_default();
    let path = ctx.dir.join(format!("cut_{idx:03}.mp4"));
    render_to_mp4_stream_quality(
        &project,
        &assets_abs,
        &path,
        RenderOptions {
            gpu_post: false,
            brand_mark: None,
        },
        Quality::Delivery,
    )
    .with_context(|| format!("render cutaway {idx}"))?;
    let frames = project.frame_count();
    Ok(Built {
        clip: CutClip {
            event: idx,
            path,
            frames,
        },
        warnings: warnings
            .iter()
            .map(|w| format!("{}: {}", w.code, w.message))
            .collect(),
        cues,
    })
}
