//! `plan-audio`, render mixing and the qa audio section (0.8).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use motion_core::audio::{
    plan_audio_with_speech, AudioPlan, DuckEnvelope, MixLevels, MusicBed, MusicPlan, SfxLibrary,
    AUDIO_PLAN_VERSION, LIBRARY_FILE, LOUDNESS_TARGET_LUFS, MUSIC_PLAN_VERSION,
    SFX_LIBRARY_VERSION, TRUE_PEAK_LIMIT_DB,
};
use motion_core::compiler::explore::apply_exploration;
use motion_core::compiler::{resolve_taste, story_key_of};
use motion_core::{CreativeIntent, MotionProject, StyleProfile};

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))
}

/// Library path: explicit flag, else `$MOTION_SFX_LIBRARY`.
fn library_path(flag: Option<&Path>) -> Option<PathBuf> {
    flag.map(Path::to_path_buf).or_else(|| {
        std::env::var_os("MOTION_SFX_LIBRARY")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
}

/// Load a library from a directory (holding `sfx-library.json`) or a manifest
/// file. Returns the library and its root directory.
fn load_library(path: &Path) -> Result<(SfxLibrary, PathBuf)> {
    let (file, root) = if path.is_dir() {
        (path.join(LIBRARY_FILE), path.to_path_buf())
    } else {
        let root = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        (path.to_path_buf(), root)
    };
    let library = SfxLibrary::from_json(&read(&file)?)
        .with_context(|| format!("parsing sfx library {}", file.display()))?;
    Ok((library, root))
}

fn required_library(flag: Option<&Path>) -> Result<(SfxLibrary, PathBuf)> {
    match library_path(flag) {
        Some(p) => load_library(&p),
        None => bail!("no SFX library: pass --sfx-library DIR or set MOTION_SFX_LIBRARY"),
    }
}

/// Read and version-check a MusicPlan json.
pub fn load_music_plan(path: &Path) -> Result<MusicPlan> {
    let m: MusicPlan = serde_json::from_str(&read(path)?)
        .with_context(|| format!("parsing music plan {}", path.display()))?;
    if m.version != MUSIC_PLAN_VERSION {
        bail!(
            "unsupported music plan version '{}' (expected {MUSIC_PLAN_VERSION})",
            m.version
        );
    }
    Ok(m)
}

fn dir_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf()
}

/// `p` made absolute with every symlink on the way resolved (like
/// `fs::canonicalize`), but tolerant of a tail that does not exist yet (a plan
/// directory about to be created, a bed not copied in): that part stays as
/// written. `..` is applied to the resolved prefix, which has no symlinks left,
/// so it is the physical parent, and never climbs above the filesystem root.
/// `None` when the working directory is unknown.
fn resolve_path(p: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in std::path::absolute(p).ok()?.components() {
        match c {
            Component::Prefix(_) | Component::RootDir => out.push(c.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                out.push(name);
                if let Ok(real) = std::fs::canonicalize(&out) {
                    out = real;
                }
            }
        }
    }
    Some(out)
}

/// `track` (relative to the music plan's directory) re-expressed relative to
/// `plan_dir`, forward slashes. (0.23) Both ends are symlink-resolved first: a
/// `..` in the written path is followed physically when the plan is read, so a
/// plan under a symlinked directory (`/tmp/x` on macOS is `/private/tmp/x`)
/// must be written from the real directory, or it points at a missing file.
/// When a path cannot be resolved the written path is the absolute one.
fn track_relative(music_json: &Path, track: &str, plan_dir: &Path) -> String {
    let written = dir_of(music_json).join(track);
    let Some(track_abs) = resolve_path(&written) else {
        return written.to_string_lossy().replace('\\', "/");
    };
    let rel = resolve_path(plan_dir)
        .and_then(|base| pathdiff::diff_paths(&track_abs, base))
        .unwrap_or(track_abs);
    rel.to_string_lossy().replace('\\', "/")
}

pub fn load_project(path: &Path) -> Result<MotionProject> {
    MotionProject::from_json(&read(path)?)
        .with_context(|| format!("parsing scene {}", path.display()))
}

/// `<scene stem>.audio.json` beside the scene (stem = file name minus
/// `.motion.json` / `.json`).
fn default_plan_path(scene: &Path) -> PathBuf {
    let name = scene
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scene".to_string());
    let stem = name
        .strip_suffix(".motion.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(&name);
    scene.with_file_name(format!("{stem}.audio.json"))
}

fn load_plan(path: &Path) -> Result<AudioPlan> {
    AudioPlan::from_json(&read(path)?)
        .with_context(|| format!("parsing audio plan {}", path.display()))
}

#[allow(clippy::too_many_arguments)]
pub fn cmd_plan_audio(
    scene: &Path,
    intent: &Path,
    style: Option<&Path>,
    reference_style: Option<&Path>,
    sfx_library: Option<&Path>,
    music: Option<&Path>,
    output: Option<&Path>,
    json: bool,
    explore: u8,
    seed: u64,
    speech: Option<&Path>,
) -> Result<()> {
    let project = load_project(scene)?;
    let intent = CreativeIntent::from_json(&read(intent)?).context("parsing intent")?;
    let style = match style {
        Some(p) => StyleProfile::from_json(&read(p)?)
            .with_context(|| format!("parsing style {}", p.display()))?,
        None => StyleProfile::default(),
    };
    let reference = reference_style
        .map(crate::reference_cmd::load_reference)
        .transpose()?;
    let taste = resolve_taste(&intent, &style, reference.as_ref().map(|n| &n.principles));
    // (0.9) Plan against the taste the scene was explored at: the level it
    // finally resolved at (a guardrail fallback may be below `--explore`).
    let level = project
        .project
        .exploration
        .as_ref()
        .and_then(|r| r.choices.first())
        .map_or(explore, |c| c.level.min(explore));
    let (taste, explore_seed) = if level >= 1 {
        let explored = apply_exploration(&taste, level, seed, story_key_of(&intent)).0;
        (explored, Some(seed))
    } else {
        (taste, None)
    };
    let (library, _root) = required_library(sfx_library)?;
    let music_plan: Option<MusicPlan> = music.map(load_music_plan).transpose()?;
    let energies: Vec<_> = intent.beats.iter().map(|b| b.energy).collect();
    // (0.10) With a speech file, cues keep 80 ms off every word onset.
    let speech_map = match speech {
        Some(p) => {
            let statements = crate::voice_cmd::spoken_lines(&intent);
            Some(crate::voice_cmd::load_speech(p, &statements)?)
        }
        None => None,
    };
    let mut plan = plan_audio_with_speech(
        &project,
        &taste,
        &energies,
        &library,
        music_plan.as_ref(),
        explore_seed,
        speech_map.as_ref(),
    );
    // (0.23) A planned voice-over sets the bed relative to the voice: the look's
    // levels ride in the plan (core's planner leaves `levels` empty so a speech
    // plan stays byte-identical to the 0.10 one apart from this field).
    if speech_map.is_some() {
        plan.levels = Some(MixLevels::for_style(&taste));
    }

    let out = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_plan_path(scene));
    // plan_audio copies MusicPlan.track; make it relative to the plan's directory.
    if let (Some(bed), Some(mp), Some(mj)) = (plan.music.as_mut(), music_plan.as_ref(), music) {
        bed.track = track_relative(mj, &mp.track, &dir_of(&out));
    }
    let text = plan.to_json_pretty() + "\n";
    if let Some(dir) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, &text).with_context(|| format!("writing {}", out.display()))?;

    if json {
        print!("{text}");
        return Ok(());
    }
    if !plan.speech_adjustments.is_empty() {
        let dropped = plan
            .speech_adjustments
            .iter()
            .filter(|a| a.to.is_none())
            .count();
        println!(
            "speech: moved {} cue(s) off word onsets, dropped {dropped}",
            plan.speech_adjustments.len() - dropped
        );
    }
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    let mut families: BTreeMap<&str, usize> = BTreeMap::new();
    for c in &plan.cues {
        *kinds.entry(c.kind.as_str()).or_default() += 1;
        *families.entry(c.family.as_str()).or_default() += 1;
    }
    let join = |m: &BTreeMap<&str, usize>| {
        if m.is_empty() {
            "none".to_string()
        } else {
            m.iter()
                .map(|(k, v)| format!("{k} {v}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    println!(
        "planned {} cue(s) -> {} (kinds: {}; families: {})",
        plan.cues.len(),
        out.display(),
        join(&kinds),
        join(&families)
    );
    Ok(())
}

/// (0.10) What `render` hands the mixer besides the video.
#[derive(Default)]
pub struct RenderAudio<'a> {
    pub sfx_library: Option<&'a Path>,
    pub audio_plan: Option<&'a Path>,
    /// `None` = Off, or Auto when a speech file is given.
    pub makeup: Option<motion_render::audio_mix::MakeupGain>,
    pub speech: Option<&'a Path>,
    /// MusicPlan json (`music-index` output) for a bed without an AudioPlan.
    pub music: Option<&'a Path>,
}

/// A plan with no cues (a voice-over, optionally over a music bed). With a
/// voice-over (`levels`) the bed is placed against the voice
/// ([`MixLevels::STANDARD`]: there is no style to look the table up from).
fn voice_only_plan(music: Option<(&MusicPlan, &Path)>, levels: Option<MixLevels>) -> AudioPlan {
    AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues: Vec::new(),
        loudness_target: LOUDNESS_TARGET_LUFS,
        true_peak_limit: TRUE_PEAK_LIMIT_DB,
        min_spacing: 0.0,
        information_cap: 0,
        music: music.map(|(m, _)| MusicBed {
            track: m.track.clone(),
            gain_db: m.gain_db,
            fade_in: 0.5,
            fade_out: 1.0,
            duck: true,
            start: 0.0,
            fade_out_at: None,
        }),
        speech_adjustments: Vec::new(),
        levels,
    }
}

/// Load a speech file and the voice-over track it points at (relative to the
/// speech file's directory).
fn load_voice(
    path: &Path,
) -> Result<(
    motion_render::audio_mix::VoiceTrack,
    motion_core::speech::SpeechMap,
)> {
    let map = motion_core::speech::SpeechMap::from_json(&read(path)?)
        .with_context(|| format!("parsing speech file {}", path.display()))?;
    let audio = motion_render::speech_qa::voice_path(path, &map);
    if !audio.is_file() {
        bail!(
            "voice-over audio {} not found (speech.audio is relative to the speech file)",
            audio.display()
        );
    }
    Ok((
        motion_render::audio_mix::VoiceTrack {
            path: audio,
            offset: 0.0,
            gain_db: 0.0,
        },
        map,
    ))
}

/// The `<track stem>.music.json` beside a bed's track, when there is one.
fn music_plan_beside(track: &Path) -> Option<MusicPlan> {
    let stem = track.file_stem()?.to_string_lossy().into_owned();
    load_music_plan(&track.with_file_name(format!("{stem}.music.json"))).ok()
}

/// What `render` and `mix` hand the mixer (0.23: the one code path of both).
pub struct MixJob<'a> {
    /// The MP4 whose video stream is kept (copied).
    pub video: &'a Path,
    /// Project duration (s).
    pub duration: f64,
    /// Where the new MP4 goes (not `video` itself).
    pub output: &'a Path,
    pub sfx_library: Option<&'a Path>,
    /// The audio plan JSON (needed with `sfx_library`).
    pub audio_plan: Option<PathBuf>,
    pub speech: Option<&'a Path>,
    pub music: Option<&'a Path>,
    pub makeup: Option<motion_render::audio_mix::MakeupGain>,
    /// Ignore `plan.levels`: the pre-0.23 mix (absolute bed, voice sidechain).
    pub legacy: bool,
    /// Also write the voice / bed / sfx stems here.
    pub stems: Option<&'a Path>,
}

/// Mix `job`; returns the one-line summary.
pub fn run_mix(job: &MixJob) -> Result<String> {
    use motion_render::audio_mix::{MakeupGain, MixOptions, VoiceRelative};
    let loaded = job.speech.map(load_voice).transpose()?;
    let music_plan: Option<MusicPlan> = job.music.map(load_music_plan).transpose()?;

    let no_library = || SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: Vec::new(),
    };
    let (mut plan, library, root, plan_dir) = match (job.audio_plan.as_ref(), job.sfx_library) {
        (Some(plan_path), lib_path) => {
            if !plan_path.is_file() {
                bail!(
                    "audio plan {} not found: run `motion-engine plan-audio` first",
                    plan_path.display(),
                );
            }
            let mut plan = load_plan(plan_path)?;
            let (library, root) = match lib_path {
                Some(p) => load_library(p)?,
                None => {
                    // No SFX library: the plan's bed and levels, no cues.
                    plan.cues.clear();
                    (no_library(), PathBuf::from("."))
                }
            };
            (plan, library, root, Some(dir_of(plan_path)))
        }
        (None, None) => (
            voice_only_plan(None, loaded.as_ref().map(|_| MixLevels::STANDARD)),
            no_library(),
            PathBuf::from("."),
            None,
        ),
        (None, Some(_)) => bail!("an audio plan is needed with --sfx-library (run `plan-audio`)"),
    };
    // MusicBed.track is relative to the AudioPlan file's directory; an explicit
    // --music replaces the bed and resolves its track beside the music plan.
    let music_root: Option<PathBuf> = match (&music_plan, job.music) {
        (Some(mp), Some(mj)) => {
            let planned = plan.music.take();
            plan.music = voice_only_plan(Some((mp, mj)), None).music;
            // (0.23 W5c) The plan's start offset and end fade belong to the
            // bed it was planned with: `--music` naming that same track keeps
            // them (render and `mix` pass the bed again), another track starts
            // at its first second.
            if let (Some(new), Some(old), Some(base)) =
                (plan.music.as_mut(), planned.as_ref(), plan_dir.as_deref())
            {
                let planned_file = resolve_path(&base.join(&old.track));
                let given_file = resolve_path(&dir_of(mj).join(&mp.track));
                if planned_file.is_some() && planned_file == given_file {
                    new.start = old.start;
                    new.fade_out_at = old.fade_out_at;
                }
            }
            Some(dir_of(mj))
        }
        _ => plan_dir,
    };
    if job.legacy {
        plan.levels = None;
    }
    let makeup = job.makeup.unwrap_or(if loaded.is_some() {
        MakeupGain::Auto
    } else {
        MakeupGain::Off
    });

    // (0.23) The speech-led bed automation and the bed's measured loudness.
    let relative = match (&loaded, plan.levels) {
        (Some((_, map)), Some(levels)) => {
            let bed_plan = match (&music_plan, plan.music.as_ref(), music_root.as_ref()) {
                (Some(mp), _, _) => Some(mp.clone()),
                (None, Some(bed), Some(root)) => music_plan_beside(&root.join(&bed.track)),
                _ => None,
            };
            Some(VoiceRelative {
                envelope: Some(DuckEnvelope::from_speech(map, &levels)),
                bed_lufs: bed_plan.as_ref().and_then(|m| m.lufs),
                bed_lra: bed_plan.as_ref().and_then(|m| m.lra),
                voiced: motion_render::audio_mix::voiced_spans(map),
            })
        }
        _ => None,
    };

    if let Some(dir) = job.stems {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    motion_render::audio_mix::mix_audio_ex(
        job.video,
        &plan,
        &root,
        &library,
        music_root.as_deref(),
        job.duration,
        job.output,
        &MixOptions {
            makeup,
            voice: loaded.as_ref().map(|(v, _)| v),
            relative: relative.as_ref(),
            stems: job.stems,
        },
    )?;
    Ok(format!(
        "mixed {} cue(s){}{}{}",
        plan.cues.len(),
        if loaded.is_some() {
            " + voice-over"
        } else {
            ""
        },
        if plan.music.is_some() { " + music" } else { "" },
        if plan.levels.is_some() && loaded.is_some() {
            " (voice-relative)"
        } else {
            ""
        },
    ))
}

/// Called after a full render encoded `mp4`. No-op (byte-identical output)
/// unless an SFX library or a speech file is given; see SOUND_DESIGN §4 and
/// VOICE.md.
pub fn mix_after_render(scene: &Path, mp4: &Path, a: &RenderAudio) -> Result<()> {
    if a.sfx_library.is_none() && a.speech.is_none() {
        if a.music.is_some() {
            bail!("--music needs --speech or --sfx-library to mix into");
        }
        return Ok(());
    }
    let audio_plan = a.sfx_library.map(|_| {
        a.audio_plan
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_plan_path(scene))
    });
    if let Some(plan_path) = audio_plan.as_ref().filter(|p| !p.is_file()) {
        bail!(
            "audio plan {} not found: run `motion-engine plan-audio {} --intent ... --sfx-library ...` first",
            plan_path.display(),
            scene.display()
        );
    }
    let project = load_project(scene)?;
    let tmp = mp4.with_extension("mixing.mp4");
    let result = run_mix(&MixJob {
        video: mp4,
        duration: project.duration_seconds(),
        output: &tmp,
        sfx_library: a.sfx_library,
        audio_plan,
        speech: a.speech,
        music: a.music,
        makeup: a.makeup,
        legacy: false,
        stems: None,
    });
    let summary = match result {
        Ok(s) => s,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    std::fs::rename(&tmp, mp4).with_context(|| format!("replacing {}", mp4.display()))?;
    println!("{summary} -> {}", mp4.display());
    Ok(())
}

/// (0.10) `qa --speech`: the SPEECH QA report.
pub fn speech_qa(
    project: &MotionProject,
    scene_dir: &Path,
    speech: &Path,
    audio_plan: Option<&Path>,
    mixed: Option<&Path>,
) -> Result<motion_render::speech_qa::SpeechQaReport> {
    let map = motion_core::speech::SpeechMap::from_json(&read(speech)?)
        .with_context(|| format!("parsing speech file {}", speech.display()))?;
    let plan = audio_plan.map(load_plan).transpose()?;
    let music_root = audio_plan.map(dir_of);
    let mut report = motion_render::speech_qa::speech_report(
        project,
        &map,
        &dir_of(speech),
        plan.as_ref(),
        mixed,
        music_root.as_deref(),
        Some(scene_dir),
    )?;
    // (0.23 W4) The four checks that see what ships: dead air, counters still
    // running, repeated templates (timeline) and text against the pixels
    // around it (rendered READ frames).
    use motion_render::speech_qa::{CheckStatus, SpeechQaCheck};
    // (0.23 W8) And `carry_continuity`: a carried picture stays readable
    // across the handoff.
    match motion_render::story_qa::motion_checks(project, Some(&map)) {
        Ok(checks) => report.checks.extend(checks),
        Err(e) => {
            for name in [
                motion_core::checks::DEAD_AIR,
                motion_core::checks::COUNT_UNSETTLED,
                motion_core::checks::REPEAT_TEMPLATE,
                motion_core::checks::CARRY_CONTINUITY,
            ] {
                report.checks.push(SpeechQaCheck {
                    name: name.to_string(),
                    status: CheckStatus::Fail,
                    detail: format!("timeline: {e}"),
                });
            }
        }
    }
    match motion_render::contrast_qa::text_local_contrast(project, scene_dir) {
        Ok(contrast) => report.checks.push(contrast.check()),
        Err(e) => report.checks.push(SpeechQaCheck {
            name: motion_core::checks::TEXT_LOCAL_CONTRAST.to_string(),
            status: CheckStatus::Fail,
            detail: format!("could not render the READ frames: {e}"),
        }),
    }
    // (0.23 W5a) Words recognised on the final mix against the clean voice
    // (`asr-local` builds; a SKIP with its reason otherwise).
    report.checks.push(crate::qa_intel::mix_intelligibility(
        &map,
        &dir_of(speech),
        mixed,
    ));
    let failed = report.checks.iter().any(|c| c.status == CheckStatus::Fail);
    report.verdict = if failed { "FAIL" } else { "PASS" }.to_string();
    Ok(report)
}

fn qa_report(
    project: &MotionProject,
    plan: &Path,
    sfx_library: Option<&Path>,
    mixed: Option<&Path>,
) -> Result<motion_render::audio_qa::AudioQaReport> {
    let plan_path = plan;
    let plan = load_plan(plan_path)?;
    // The MusicPlan sits beside the track: `<track stem>.music.json`.
    let downbeats: Option<Vec<f64>> = plan.music.as_ref().and_then(|m| {
        let track = dir_of(plan_path).join(&m.track);
        let stem = track.file_stem()?.to_string_lossy().into_owned();
        let json = track.with_file_name(format!("{stem}.music.json"));
        // (0.23 W5c) The bed starts `start` s into the track: its downbeats on
        // the project timeline are that much earlier.
        load_music_plan(&json).ok().map(|p| {
            p.downbeat_times
                .iter()
                .map(|b| b - m.start)
                .collect::<Vec<f64>>()
        })
    });
    let library = match library_path(sfx_library) {
        Some(p) => load_library(&p)?.0,
        None => SfxLibrary {
            version: SFX_LIBRARY_VERSION.to_string(),
            sounds: Vec::new(),
        },
    };
    Ok(motion_render::audio_qa::audio_report_with_music(
        project,
        &plan,
        &library,
        mixed,
        downbeats.as_deref(),
    )?)
}

pub fn audio_qa_json(
    project: &MotionProject,
    plan: &Path,
    sfx_library: Option<&Path>,
    mixed: Option<&Path>,
) -> Result<serde_json::Value> {
    let report = qa_report(project, plan, sfx_library, mixed)?;
    Ok(serde_json::to_value(&report)?)
}

pub fn audio_qa_text(
    project: &MotionProject,
    plan: &Path,
    sfx_library: Option<&Path>,
    mixed: Option<&Path>,
) -> Result<String> {
    Ok(qa_report(project, plan, sfx_library, mixed)?.to_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn music_track_is_rewritten_relative_to_the_audio_plan_dir() {
        let rel = track_relative(
            Path::new("/p/music/bed.music.json"),
            "bed.wav",
            Path::new("/p/out"),
        );
        assert_eq!(rel, "../music/bed.wav");
        let same = track_relative(
            Path::new("/p/out/bed.music.json"),
            "sub/bed.wav",
            Path::new("/p/out"),
        );
        assert_eq!(same, "sub/bed.wav");
    }

    /// (0.23) The plan directory is a symlink to a deeper directory: the
    /// written path must climb from the real directory, because that is where
    /// a `..` goes when the plan is read.
    #[cfg(unix)]
    #[test]
    fn music_track_is_resolved_through_a_symlinked_plan_dir() {
        let base = std::env::temp_dir().join(format!("me-track-rel-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let real = base.join("real/a/b");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(base.join("music")).unwrap();
        std::fs::write(base.join("music/bed.wav"), b"x").unwrap();
        let link = base.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let rel = track_relative(&base.join("music/bed.music.json"), "bed.wav", &link);
        assert_eq!(rel, "../../../music/bed.wav");
        // What a consumer does: the plan's directory (the link) joined with it.
        assert_eq!(
            link.join(&rel).canonicalize().unwrap(),
            base.join("music/bed.wav").canonicalize().unwrap()
        );
        // The plan directory not created yet, the bed not copied in yet: the
        // existing part is still resolved, the rest kept as written.
        let later = track_relative(
            &base.join("music/bed.music.json"),
            "missing.wav",
            &link.join("sub/dir"),
        );
        assert_eq!(later, "../../../../../music/missing.wav");
        // A `..` in the track is followed on the resolved path.
        let up = track_relative(
            &base.join("real/a/bed.music.json"),
            "../../music/bed.wav",
            &link,
        );
        assert_eq!(up, "../../../music/bed.wav");
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn resolve_path_never_climbs_above_the_root() {
        assert_eq!(
            resolve_path(Path::new("/../../x")).unwrap(),
            Path::new("/x")
        );
        assert_eq!(
            resolve_path(Path::new("/p/./q/../r")).unwrap(),
            Path::new("/p/r")
        );
    }
}
