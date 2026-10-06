//! `explore_styles(story, k?, style?, format?)`: the same story in k looks,
//! one row per look on one sheet.
//!
//! The story goes through the same checks and auto-fixes as `make_video`
//! (`policy::prepare`), so what is explored is what would be made. The voice
//! is made once (`motion-engine voice`, cached by the engine), then for every
//! look `motion-engine compile … --art <look> --variety auto` and up to 4
//! frames (READ moments of beats spread evenly) are rendered. The first row is
//! the look `--art auto` would pick for this story, then its curated
//! neighbours, then the other looks in the engine's order.
//!
//! Reproducibility: a later `make_video(options.art = <look>)` compiles with
//! the same `--art <look> --variety auto` and the same voice (same text, same
//! voice model, same cache), so the chosen look reproduces what the sheet
//! showed (the sheet is stills of the READ moments; the video adds motion).
//! Numeric variety seeds are Phase 2: the reel cannot take them yet.
//!
//! Work folder `<jobs>/_explore/<hash of story, style, format, k, voice,
//! engine>/`: the same request is answered from it instantly.

use std::path::{Path, PathBuf};
use std::time::Duration;

use motion_core::compiler::art_direction::{self, look_defaults, ArtMode, Look};
use motion_core::compiler::resolve_taste;
use motion_core::compiler::typography::{emotion_name, resolve_emotion};
use motion_core::scene::MotionProject;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::frames;
use super::options::look_best_use;
use super::proc;
use super::sheet::{self, Cell, Row};
use super::{needs_fix, parse_args, rel, ToolOutput};
use crate::args::{ExploreStylesArgs, MakeVideoArgs, Mode};
use crate::engine::stem_of;
use crate::job::{canonical_json, INTENT_JSON, STYLE_JSON};
use crate::pictures::PictureIndex;
use crate::policy::{self, Context};
use crate::profile::ServerConfig;
use crate::reply::{Fix, ImageOut, Reply};

/// Folder of explore work under the jobs root.
pub const EXPLORE_DIR: &str = "_explore";
/// Fewest and most looks of one call.
pub const MIN_LOOKS: usize = 2;
pub const MAX_LOOKS: usize = 4;
pub const DEFAULT_LOOKS: usize = 3;
/// Frames per row.
pub const FRAMES_PER_ROW: usize = 4;
/// Explore folders kept (newest by modification time).
pub const KEEP_EXPLORES: usize = 24;
pub const NEXT: &str = "Pass options.art (the look) to make_video.";

const VOICE_TIMEOUT: Duration = Duration::from_secs(600);
const COMPILE_TIMEOUT: Duration = Duration::from_secs(180);
const MUSIC_TIMEOUT: Duration = Duration::from_secs(30);

/// The looks to show: `auto` first, then its curated neighbours, then the
/// rest of [`Look::ALL`] in order; no duplicates; `k` clamped to 2–4.
pub fn explore_looks(auto: Look, k: usize) -> Vec<Look> {
    let k = k.clamp(MIN_LOOKS, MAX_LOOKS);
    let mut out: Vec<Look> = vec![auto];
    for l in art_direction::neighbours(auto).into_iter().chain(Look::ALL) {
        if !out.contains(&l) {
            out.push(l);
        }
    }
    out.truncate(k);
    out
}

/// The look `--art auto` picks for this story (what `compile` does: the
/// emotion the taste evokes, or the genre's own look).
pub fn auto_look(
    intent: &motion_core::CreativeIntent,
    style: &motion_core::StyleProfile,
) -> (Look, &'static str) {
    let taste = resolve_taste(intent, style, None);
    let emotion = resolve_emotion(&taste);
    let look = art_direction::resolve_with_genre(ArtMode::Auto, emotion, taste.genre).look;
    (look, emotion_name(emotion))
}

/// The folder name of a request.
fn request_hash(
    config: &ServerConfig,
    intent: &motion_core::CreativeIntent,
    style: &motion_core::StyleProfile,
    k: usize,
) -> String {
    let key = json!({
        "intent": intent, "style": style, "k": k,
        "voice": config.tts_model, "engine": config.engine_version,
    });
    let digest = Sha256::digest(canonical_json(&key).as_bytes());
    digest.iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// Voice flags `reel` would pass that `voice` takes too: the reel's extra
/// flags `--offline`, `--align X` and `--asr-ctc X`.
fn voice_flags(config: &ServerConfig) -> Vec<String> {
    let mut out = Vec::new();
    let extra = &config.reel_extra;
    let mut i = 0;
    while i < extra.len() {
        let a = extra[i].as_str();
        if a == "--offline" {
            out.push(a.to_string());
        } else if matches!(a, "--align" | "--asr-ctc") {
            if let Some(v) = extra.get(i + 1) {
                out.push(a.to_string());
                out.push(v.clone());
                i += 1;
            }
        } else if a.starts_with("--align=") || a.starts_with("--asr-ctc=") {
            out.push(a.to_string());
        }
        i += 1;
    }
    out
}

/// The music plan `reel --music auto` would pass to the compile as
/// `--music-energy` (genre looks pulse to it). Asked of the reel itself
/// (`reel --plan-only`: the bed the story's mood picks from the v0.2 catalog,
/// with the same take-0 seed), so the sheet's looks pulse to the bed the video
/// will have. `None` when the reel finds no bed that fits or cannot run.
fn music_plan(config: &ServerConfig, intent: &Path, style: &Path) -> Option<PathBuf> {
    let mut cmd = config.engine_command();
    cmd.arg("reel")
        .arg(intent)
        .arg("--style")
        .arg(style)
        .arg("--assets")
        .arg(&config.assets)
        .arg("--plan-only");
    let ran = proc::run("music", cmd, MUSIC_TIMEOUT).ok()?;
    ran.stdout
        .lines()
        .find_map(|l| l.strip_prefix("music plan: "))
        .map(|p| PathBuf::from(p.trim()))
}

/// Keep the newest [`KEEP_EXPLORES`] work folders (best effort).
fn prune(root: &Path, keep: &Path) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.path().symlink_metadata().ok()?;
            if m.is_dir() {
                Some((m.modified().ok()?, e.path()))
            } else {
                None
            }
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in dirs.into_iter().skip(KEEP_EXPLORES) {
        if p != keep {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// What a finished explore keeps: enough to answer the same request again
/// without any engine call.
fn answer(
    config: &ServerConfig,
    dir: &Path,
    title: &str,
    auto: Look,
    rows: &[(Look, Vec<usize>)],
    notes: &[String],
    cached: bool,
) -> ToolOutput {
    let sheet_path = dir.join("sheet.png");
    let mut text = format!("done · {} looks of \"{title}\"", rows.len());
    let shown: Vec<usize> = rows.first().map(|r| r.1.clone()).unwrap_or_default();
    if !shown.is_empty() {
        text.push_str(&format!(
            " (beats {})",
            shown
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for (i, (look, _)) in rows.iter().enumerate() {
        text.push_str(&format!(
            "\nrow {}: {}{}",
            i + 1,
            look.name(),
            if *look == auto { " (auto)" } else { "" }
        ));
    }
    for n in notes {
        text.push_str(&format!("\nnote: {n}"));
    }
    text.push_str(&format!("\nnext: {NEXT}"));
    let structured = json!({
        "status": "done",
        "title": title,
        "auto": auto.name(),
        "rows": rows.iter().enumerate().map(|(i, (look, beats))| json!({
            "row": i + 1,
            "art": look.name(),
            "auto": *look == auto,
            "best_use": look_best_use(*look),
            "families": look_defaults(*look).families,
            "beats": beats,
        })).collect::<Vec<_>>(),
        "sheet": rel(config, &sheet_path),
        "cached": cached,
        "notes": notes,
        "next": NEXT,
    });
    ToolOutput {
        structured,
        text,
        images: vec![ImageOut {
            path: sheet_path,
            mime: "image/png",
        }],
        links: Vec::new(),
        is_error: false,
    }
}

/// The `explore_styles` tool.
pub fn explore_styles(config: &ServerConfig, pictures: &PictureIndex, args: Value) -> ToolOutput {
    let a: ExploreStylesArgs = match parse_args(args) {
        Ok(a) => a,
        Err(fix) => return needs_fix(fix),
    };
    if a.story.is_null() {
        return needs_fix(
            Fix::new(None, "story", "give the story to explore")
                .otherwise("pass the same story object as make_video"),
        );
    }
    let k = a.k.unwrap_or(DEFAULT_LOOKS).clamp(MIN_LOOKS, MAX_LOOKS);
    let make = MakeVideoArgs {
        story: a.story.clone(),
        style: a.style.clone(),
        brand: a.brand.clone(),
        format: a.format.clone(),
        mode: Mode::Check,
        ..MakeVideoArgs::default()
    };
    let prepared = match policy::prepare(&make, &Context { config, pictures }) {
        Ok(p) => p,
        Err(fixes) => return ToolOutput::from(Reply::needs_fix(fixes)),
    };
    let intent = &prepared.intent;
    let (auto, _) = auto_look(intent, &prepared.style);
    let looks = explore_looks(auto, k);

    let root = config.jobs.join(EXPLORE_DIR);
    let dir = root.join(request_hash(config, intent, &prepared.style, k));
    let meta_path = dir.join("explore.json");
    let sheet_path = dir.join("sheet.png");
    // The same request again: answer from the work folder.
    if let (Some(meta), Some(_)) = (
        std::fs::read_to_string(&meta_path)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok()),
        sheet::png_size(&sheet_path),
    ) {
        let rows: Vec<(Look, Vec<usize>)> = meta["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let look = Look::parse(r["art"].as_str()?)?;
                let beats = r["beats"]
                    .as_array()?
                    .iter()
                    .filter_map(|b| b.as_u64().map(|b| b as usize))
                    .collect();
                Some((look, beats))
            })
            .collect();
        let notes: Vec<String> = meta["notes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|n| n.as_str().map(str::to_string))
            .collect();
        if !rows.is_empty() {
            return answer(config, &dir, &intent.title, auto, &rows, &notes, true);
        }
    }

    match build(config, &dir, &prepared, &looks, auto) {
        Ok((rows, notes)) => {
            let meta = json!({
                "title": intent.title,
                "auto": auto.name(),
                "rows": rows.iter().map(|(l, b)| json!({"art": l.name(), "beats": b})).collect::<Vec<_>>(),
                "notes": notes,
            });
            let _ = std::fs::write(&meta_path, meta.to_string());
            prune(&root, &dir);
            answer(config, &dir, &intent.title, auto, &rows, &notes, false)
        }
        Err(e) => ToolOutput::from(Reply::failed(None, e)),
    }
}

type Built = (Vec<(Look, Vec<usize>)>, Vec<String>);

/// Voice once, then compile and render every look; write `sheet.png`.
fn build(
    config: &ServerConfig,
    dir: &Path,
    prepared: &policy::Prepared,
    looks: &[Look],
    auto: Look,
) -> Result<Built, String> {
    let intent = &prepared.intent;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
    write_json(dir, INTENT_JSON, &prepared.intent)?;
    write_json(dir, STYLE_JSON, &prepared.style)?;
    let intent_path = dir.join(INTENT_JSON);
    let style_path = dir.join(STYLE_JSON);

    // Voice, once (cached by the engine, so a later make_video reuses it).
    let stem = stem_of(&intent.title);
    let speech = dir.join(format!("{stem}.speech.json"));
    if !speech.is_file() {
        let mut cmd = config.engine_command();
        cmd.arg("voice")
            .arg(&intent_path)
            .arg("--style")
            .arg(&style_path)
            .args(["--tts-model", &config.tts_model])
            .args(voice_flags(config))
            .arg("-o")
            .arg(dir);
        proc::run("voice", cmd, VOICE_TIMEOUT)?;
        if !speech.is_file() {
            return Err("voice: no speech file was written".to_string());
        }
    }

    // One compile per look.
    let pulses = looks.iter().any(|l| look_defaults(*l).fx.pulse_gain > 0.0);
    let music = if pulses {
        music_plan(config, &intent_path, &style_path)
    } else {
        None
    };
    let compiled: Vec<Result<PathBuf, String>> =
        proc::par_map(looks, proc::threads(), |look| -> Result<PathBuf, String> {
            let scene = dir.join(format!("{}.motion.json", look.name()));
            if MotionProject::from_json(&std::fs::read_to_string(&scene).unwrap_or_default())
                .is_ok()
            {
                return Ok(scene);
            }
            let mut cmd = config.engine_command();
            cmd.arg("compile")
                .arg(&intent_path)
                .arg("--style")
                .arg(&style_path)
                .arg("--speech")
                .arg(&speech)
                .arg("--assets")
                .arg(&config.assets)
                .args(["--art", look.name(), "--variety", "auto"]);
            // Looks that pulse to the music compile with the bed's energy, as
            // the reel does.
            if look_defaults(*look).fx.pulse_gain > 0.0 {
                if let Some(plan) = &music {
                    cmd.arg("--music-energy").arg(plan);
                }
            }
            cmd.arg("--output").arg(&scene);
            proc::run(&format!("compile {}", look.name()), cmd, COMPILE_TIMEOUT)?;
            Ok(scene)
        });
    let mut notes: Vec<String> = Vec::new();
    let mut variants: Vec<(Look, PathBuf, MotionProject)> = Vec::new();
    for (look, r) in looks.iter().zip(compiled) {
        match r.and_then(|scene| {
            let text = std::fs::read_to_string(&scene).map_err(|e| e.to_string())?;
            let project = MotionProject::from_json(&text).map_err(|e| e.to_string())?;
            Ok((scene, project))
        }) {
            Ok((scene, project)) => variants.push((*look, scene, project)),
            Err(e) => notes.push(format!("{} left out: {e}", look.name())),
        }
    }
    if variants.is_empty() {
        return Err(notes
            .first()
            .cloned()
            .unwrap_or_else(|| "no look compiled".to_string()));
    }

    // Frames: the READ moments of up to 4 beats spread evenly.
    let mut jobs: Vec<(usize, u32)> = Vec::new();
    let mut shown: Vec<Vec<(usize, u32)>> = Vec::new();
    for (i, (_, _, project)) in variants.iter().enumerate() {
        let moments = frames::read_moments(project);
        let pick: Vec<(usize, u32)> = frames::spread(moments.len(), FRAMES_PER_ROW)
            .into_iter()
            .map(|b| (b, moments[b - 1].frame))
            .collect();
        jobs.extend(pick.iter().map(|(_, f)| (i, *f)));
        shown.push(pick);
    }
    // One render pool over every (look, frame): the looks share the CPU.
    let rendered = proc::par_map(&jobs, proc::threads(), |(i, f)| {
        let (look, scene, _) = &variants[*i];
        frames::render_one(config, scene, &dir.join("frames").join(look.name()), *f)
    });
    let mut failed: Vec<Option<String>> = vec![None; variants.len()];
    for ((i, _), r) in jobs.iter().zip(rendered) {
        if let Err(e) = r {
            failed[*i].get_or_insert(e);
        }
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut done: Vec<(Look, Vec<usize>)> = Vec::new();
    for (i, (look, _, _)) in variants.iter().enumerate() {
        if let Some(e) = &failed[i] {
            notes.push(format!("{} left out: {e}", look.name()));
            continue;
        }
        let frames_dir = dir.join("frames").join(look.name());
        rows.push(Row {
            label: Some(format!(
                "{} {}{}",
                rows.len() + 1,
                look.name(),
                if *look == auto { " (auto)" } else { "" }
            )),
            cells: shown[i]
                .iter()
                .map(|(_, f)| Cell {
                    image: frames::frame_path(&frames_dir, *f),
                    label: None,
                })
                .collect(),
        });
        done.push((*look, shown[i].iter().map(|(b, _)| *b).collect()));
    }
    if rows.is_empty() {
        return Err(notes
            .first()
            .cloned()
            .unwrap_or_else(|| "no frames rendered".to_string()));
    }
    sheet::build(&rows, &dir.join("sheet.png"))?;
    Ok((done, notes))
}

fn write_json<T: serde::Serialize>(dir: &Path, name: &str, value: &T) -> Result<(), String> {
    let text =
        serde_json::to_string_pretty(value).map_err(|e| format!("cannot write {name}: {e}"))?;
    std::fs::write(dir.join(name), text).map_err(|e| format!("cannot write {name}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_auto_look_comes_first_then_its_neighbours() {
        for auto in Look::ALL {
            for k in 0..=6 {
                let looks = explore_looks(auto, k);
                assert_eq!(looks.len(), k.clamp(2, 4), "{auto:?} k={k}");
                assert_eq!(looks[0], auto);
                let mut seen = looks.clone();
                seen.sort_by_key(|l| l.name());
                seen.dedup();
                assert_eq!(seen.len(), looks.len(), "duplicates for {auto:?}");
                // Neighbours (in the curated order) follow the auto look.
                let n = art_direction::neighbours(auto);
                if looks.len() >= 2 {
                    assert_eq!(looks[1], n[0]);
                }
                if looks.len() >= 3 {
                    assert_eq!(looks[2], n[1]);
                }
            }
        }
        // Past the neighbours the engine's own order continues.
        assert_eq!(
            explore_looks(Look::Dossier, 4),
            vec![
                Look::Dossier,
                Look::ClassicalNeon,
                Look::OrnamentEditorial,
                Look::HalftoneCutout
            ]
        );
    }

    #[test]
    fn voice_flags_follow_the_reel_extras() {
        let mut c = ServerConfig::new(crate::profile::Profile::Creator, "/repo");
        c.reel_extra = vec![
            "--offline".into(),
            "--align".into(),
            "onset".into(),
            "--gpu".into(),
            "--asr-ctc=off".into(),
        ];
        assert_eq!(
            voice_flags(&c),
            vec!["--offline", "--align", "onset", "--asr-ctc=off"]
        );
    }
}
