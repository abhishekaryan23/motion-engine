//! `view_frames(job, beats?, times?, sheet?)`: look at a job before revising it.
//!
//! The job's scene is `<job>/reel/<stem>.motion.json` (the reel's output). Each
//! requested beat is shown at its READ moment; explicit `times` are a viewing
//! query. Frames come from `motion-engine render <scene> --frame N`, are cached
//! under `<job>/frames/<scene hash>/`, and are tiled into one contact sheet
//! `<job>/sheet_<hash>.png` (long side <= 1024 px, a small label per tile).

use std::path::{Path, PathBuf};

use motion_core::scene::MotionProject;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::frames::{self, Moment};
use super::sheet::{self, Cell, Row};
use super::{job_dir_of, needs_fix, parse_args, rel, scene_of, ToolOutput};
use crate::args::ViewFramesArgs;
use crate::job::{self, canonical_json, JobId, JobState, JobStatus};
use crate::profile::ServerConfig;
use crate::reply::{Fix, ImageOut, Reply, Status};

/// Most frames of one call (a story has at most 12 beats).
pub const MAX_FRAMES: usize = 12;
/// Most separate images of one call.
pub const MAX_SEPARATE: usize = 6;

pub const NEXT_LOOK: &str = "Check focus, text over faces, empty frames and repetition; then call revise_video with changes.";

/// One frame to show.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    /// `beat 2` or `1.5 s`.
    pub label: String,
    pub beat: Option<usize>,
    pub time_s: f64,
    pub frame: u32,
}

/// The frames a request asks for: `times` when given, else the chosen `beats`
/// (default all) at their READ moments. A fix when a beat does not exist, a
/// time is not a number, or there are too many frames.
pub fn picks_for(
    project: &MotionProject,
    beats: Option<&[usize]>,
    times: Option<&[f64]>,
) -> Result<Vec<Pick>, Fix> {
    let moments = frames::read_moments(project);
    let fps = project.canvas.fps.max(1) as f64;
    let last = project.frame_count().saturating_sub(1);
    let mut picks: Vec<Pick> = Vec::new();
    match times.filter(|t| !t.is_empty()) {
        Some(times) => {
            if times.len() > MAX_FRAMES {
                return Err(Fix::new(
                    None,
                    "times",
                    format!("at most {MAX_FRAMES} times (got {})", times.len()),
                ));
            }
            let end = last as f64 / fps;
            for t in times {
                if !t.is_finite() {
                    return Err(Fix::new(None, "times", "every time is a number of seconds"));
                }
                let t = t.clamp(0.0, end);
                let frame = frames::frame_at(project, t);
                if !picks.iter().any(|p| p.frame == frame) {
                    picks.push(Pick {
                        label: format!("{t:.1} s"),
                        beat: None,
                        time_s: t,
                        frame,
                    });
                }
            }
        }
        None => {
            if moments.is_empty() {
                return Err(Fix::new(None, "job", "the scene has no beats"));
            }
            let mut chosen: Vec<usize> = match beats.filter(|b| !b.is_empty()) {
                Some(b) => b.to_vec(),
                None => (1..=moments.len()).collect(),
            };
            if let Some(bad) = chosen.iter().find(|b| **b < 1 || **b > moments.len()) {
                return Err(Fix::new(
                    None,
                    "beats",
                    format!("no beat {bad}: this video has {} beats", moments.len()),
                )
                .otherwise(format!("use beat numbers 1 to {}", moments.len())));
            }
            chosen.sort_unstable();
            chosen.dedup();
            for b in chosen {
                let Moment {
                    beat,
                    time_s,
                    frame,
                } = moments[b - 1];
                if !picks.iter().any(|p| p.frame == frame) {
                    picks.push(Pick {
                        label: format!("beat {beat}"),
                        beat: Some(beat),
                        time_s,
                        frame,
                    });
                }
            }
        }
    }
    Ok(picks)
}

/// `1–4`, `1, 3, 5`, `2` (consecutive runs collapse).
pub fn beat_list(beats: &[usize]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    while i < beats.len() {
        let mut j = i;
        while j + 1 < beats.len() && beats[j + 1] == beats[j] + 1 {
            j += 1;
        }
        parts.push(match j - i {
            0 => beats[i].to_string(),
            1 => format!("{}, {}", beats[i], beats[j]),
            _ => format!("{}–{}", beats[i], beats[j]),
        });
        i = j + 1;
    }
    parts.join(", ")
}

/// The job is not ready (still rendering, failed, or has no scene).
fn not_ready(id: &JobId, dir: &Path) -> ToolOutput {
    let status: Option<JobStatus> = std::fs::read_to_string(dir.join(job::STATUS_JSON))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let mut r = match status {
        Some(s) if s.state == JobState::Failed => Reply::failed(
            Some(id.to_string()),
            s.error.unwrap_or_else(|| "the job failed".to_string()),
        ),
        Some(s) if s.state == JobState::Done => Reply::failed(
            Some(id.to_string()),
            "the job has no scene file (reel/<title>.motion.json)",
        ),
        _ => Reply::new(
            Status::Running,
            "The job has no scene yet (still rendering). Call get_video with this job and wait_s 120, then view_frames again.",
        ),
    };
    r.job = Some(id.to_string());
    ToolOutput::from(r)
}

fn sheet_hash(scene_hash: &str, picks: &[Pick]) -> String {
    let key = json!({
        "scene": scene_hash,
        "frames": picks.iter().map(|p| (p.frame, &p.label)).collect::<Vec<_>>(),
    });
    let digest = Sha256::digest(canonical_json(&key).as_bytes());
    digest.iter().take(5).map(|b| format!("{b:02x}")).collect()
}

/// The `view_frames` tool.
pub fn view_frames(config: &ServerConfig, args: Value) -> ToolOutput {
    let a: ViewFramesArgs = match parse_args(args) {
        Ok(a) => a,
        Err(fix) => return needs_fix(fix),
    };
    let (id, dir) = match job_dir_of(config, &a.job) {
        Ok(x) => x,
        Err(out) => return out,
    };
    let Some(scene_path) = scene_of(&dir) else {
        return not_ready(&id, &dir);
    };
    let project = match std::fs::read_to_string(&scene_path)
        .map_err(|e| e.to_string())
        .and_then(|t| MotionProject::from_json(&t).map_err(|e| e.to_string()))
    {
        Ok(p) => p,
        Err(e) => {
            return ToolOutput::from(Reply::failed(
                Some(id.to_string()),
                format!("cannot read the scene: {e}"),
            ))
        }
    };
    let mut picks = match picks_for(&project, a.beats.as_deref(), a.times.as_deref()) {
        Ok(p) => p,
        Err(fix) => return needs_fix(fix),
    };
    let ignored_beats = a.times.as_ref().is_some_and(|t| !t.is_empty())
        && a.beats.as_ref().is_some_and(|b| !b.is_empty());
    let separate = a.sheet == Some(false) || picks.len() == 1;
    let mut notes: Vec<String> = Vec::new();
    if ignored_beats {
        notes.push("times were given, so beats were ignored".to_string());
    }
    if separate && picks.len() > MAX_SEPARATE {
        notes.push(format!(
            "showing the first {MAX_SEPARATE} of {} frames as separate images; ask for the rest with beats or times",
            picks.len()
        ));
        picks.truncate(MAX_SEPARATE);
    }

    let scene_hash = match frames::file_hash(&scene_path) {
        Ok(h) => h,
        Err(e) => {
            return ToolOutput::from(Reply::failed(
                Some(id.to_string()),
                format!("cannot read the scene: {e}"),
            ))
        }
    };
    let frames_dir = dir.join("frames").join(&scene_hash);
    let wanted: Vec<u32> = picks.iter().map(|p| p.frame).collect();
    if let Err(e) = frames::render_frames(config, &scene_path, &frames_dir, &wanted) {
        return ToolOutput::from(Reply::failed(Some(id.to_string()), e));
    }

    let built = if separate {
        separate_images(&frames_dir, &picks)
    } else {
        contact_sheet(&dir, &frames_dir, &scene_hash, &picks).map(|p| vec![p])
    };
    let images = match built {
        Ok(i) => i,
        Err(e) => return ToolOutput::from(Reply::failed(Some(id.to_string()), e)),
    };

    let beat_numbers: Vec<usize> = picks.iter().filter_map(|p| p.beat).collect();
    let what = if beat_numbers.is_empty() {
        let at: Vec<String> = picks.iter().map(|p| p.label.clone()).collect();
        format!("frames at {}", at.join(", "))
    } else if beat_numbers.len() == 1 {
        format!("beat {} at its reading moment", beat_numbers[0])
    } else {
        format!("beats {} at their reading moment", beat_list(&beat_numbers))
    };
    let shape = if separate {
        format!("{} image(s)", images.len())
    } else {
        let order = if sheet::row_split(picks.len()).len() > 1 {
            "left to right, top to bottom"
        } else {
            "left to right"
        };
        format!("one sheet, {order}, {} tiles", picks.len())
    };
    let mut text = format!("done · job {id} · {what} · {shape}");
    for n in &notes {
        text.push_str(&format!("\nnote: {n}"));
    }
    text.push_str(&format!("\nnext: {NEXT_LOOK}"));

    let rels: Vec<String> = images.iter().map(|p| rel(config, p)).collect();
    let mut structured = json!({
        "status": "done",
        "job": id.as_str(),
        "frames": picks.iter().map(|p| json!({
            "label": p.label, "beat": p.beat, "time_s": (p.time_s * 1000.0).round() / 1000.0, "frame": p.frame,
        })).collect::<Vec<_>>(),
        "next": NEXT_LOOK,
    });
    if separate {
        structured["images"] = json!(rels);
    } else if let Some(s) = rels.first() {
        structured["sheet"] = json!(s);
    }
    if !notes.is_empty() {
        structured["notes"] = json!(notes);
    }
    ToolOutput {
        structured,
        text,
        images: images
            .into_iter()
            .map(|path| ImageOut {
                path,
                mime: "image/png",
            })
            .collect(),
        links: Vec::new(),
        is_error: false,
    }
}

/// The job's contact sheet `<job>/sheet_<hash>.png` (reused when it exists).
fn contact_sheet(
    dir: &Path,
    frames_dir: &Path,
    scene_hash: &str,
    picks: &[Pick],
) -> Result<PathBuf, String> {
    let out = dir.join(format!("sheet_{}.png", sheet_hash(scene_hash, picks)));
    if sheet::png_size(&out).is_some() {
        return Ok(out);
    }
    let mut at = 0;
    let rows: Vec<Row> = sheet::row_split(picks.len())
        .into_iter()
        .map(|n| {
            let cells = picks[at..at + n]
                .iter()
                .map(|p| Cell {
                    image: frames::frame_path(frames_dir, p.frame),
                    label: Some(match p.beat {
                        Some(b) => format!("beat {b} {:.1}s", p.time_s),
                        None => p.label.clone(),
                    }),
                })
                .collect();
            at += n;
            Row { label: None, cells }
        })
        .collect();
    sheet::build(&rows, &out)?;
    Ok(out)
}

/// One image per frame, long side <= 1024 px.
fn separate_images(frames_dir: &Path, picks: &[Pick]) -> Result<Vec<PathBuf>, String> {
    picks
        .iter()
        .map(|p| {
            let dst = frames_dir.join(format!("view_{:06}.png", p.frame));
            if sheet::png_size(&dst).is_none() {
                sheet::fit_image(&frames::frame_path(frames_dir, p.frame), &dst)?;
            }
            Ok(dst)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_lists_collapse_runs() {
        assert_eq!(beat_list(&[1, 2, 3, 4, 5]), "1–5");
        assert_eq!(beat_list(&[1, 3, 5]), "1, 3, 5");
        assert_eq!(beat_list(&[2]), "2");
        assert_eq!(beat_list(&[1, 2, 4]), "1, 2, 4");
        assert_eq!(beat_list(&[1, 2, 3, 6, 7]), "1–3, 6, 7");
    }
}
