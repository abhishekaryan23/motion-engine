//! Which frames to look at and how to get them: the READ moment of each beat
//! (the same moment `layout_qa` judges: `scene.start + lifecycle.read`), single
//! frames through `motion-engine render <scene> --frame N`, cached by scene
//! hash so a second look at the same job renders nothing.

use std::path::{Path, PathBuf};
use std::time::Duration;

use motion_core::audio::beat_scenes;
use motion_core::compiler::captions::CAPTION_SCENE_ID;
use motion_core::scene::MotionProject;
use sha2::{Digest, Sha256};

use super::proc;
use crate::profile::ServerConfig;

/// One frame of a beat at its READ moment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Moment {
    /// 1-based beat.
    pub beat: usize,
    /// Project time in seconds.
    pub time_s: f64,
    pub frame: u32,
}

/// Deadline of one single-frame render.
const FRAME_TIMEOUT: Duration = Duration::from_secs(180);

/// The frame nearest project time `t` (seconds), kept inside the video.
pub fn frame_at(project: &MotionProject, t: f64) -> u32 {
    let last = project.frame_count().saturating_sub(1);
    let n = (t * project.canvas.fps as f64).round();
    if n.is_nan() || n <= 0.0 {
        0
    } else {
        (n as u32).min(last)
    }
}

/// The READ moment of every beat, in beat order: `scene.start +
/// lifecycle.read` (the middle of the scene when it has no lifecycle), as
/// `layout_qa` does. The caption scene and the backdrop are not beats.
pub fn read_moments(project: &MotionProject) -> Vec<Moment> {
    beat_scenes(project)
        .into_iter()
        .filter(|s| s.id != CAPTION_SCENE_ID)
        .enumerate()
        .map(|(i, scene)| {
            let local = match scene.lifecycle {
                Some(l) => l.read,
                None => scene.duration_seconds * 0.5,
            };
            let time_s = scene.start_seconds + local;
            Moment {
                beat: i + 1,
                time_s,
                frame: frame_at(project, time_s),
            }
        })
        .collect()
}

/// Up to `n` of `total` beats spread evenly, first and last included
/// (1-based). All of them when `total <= n`.
pub fn spread(total: usize, n: usize) -> Vec<usize> {
    if total <= n {
        return (1..=total).collect();
    }
    if n <= 1 {
        return vec![1];
    }
    let mut out: Vec<usize> = (0..n)
        .map(|j| 1 + (j * (total - 1) + (n - 1) / 2) / (n - 1))
        .collect();
    out.dedup();
    out
}

/// The first 8 hex characters of sha256 over a file (keys cached frames and
/// sheets to the scene they were rendered from).
pub fn file_hash(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().take(4).map(|b| format!("{b:02x}")).collect())
}

/// `<dir>/frame_NNNNNN.png`, the name `render --frame` gives.
pub fn frame_path(dir: &Path, frame: u32) -> PathBuf {
    dir.join(format!("frame_{frame:06}.png"))
}

fn is_cached(path: &Path) -> bool {
    super::sheet::png_size(path).is_some()
}

/// Render one frame of `scene` into `dir` as `frame_NNNNNN.png`, unless it is
/// already there. The frame is rendered into its own temporary folder and
/// moved into place, so an interrupted render never leaves a half-written PNG
/// behind (and concurrent renders of different frames never collide).
pub fn render_one(
    config: &ServerConfig,
    scene: &Path,
    dir: &Path,
    frame: u32,
) -> Result<PathBuf, String> {
    let target = frame_path(dir, frame);
    if is_cached(&target) {
        return Ok(target);
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(".tmp_{frame}"));
    let _ = std::fs::remove_dir_all(&tmp);
    let mut cmd = config.engine_command();
    cmd.arg("render")
        .arg(scene)
        .args(["--frame", &frame.to_string(), "--out-dir"])
        .arg(&tmp);
    let r = proc::run(&format!("render frame {frame}"), cmd, FRAME_TIMEOUT).and_then(|_| {
        std::fs::rename(frame_path(&tmp, frame), &target)
            .map_err(|e| format!("render frame {frame}: no PNG written ({e})"))
    });
    let _ = std::fs::remove_dir_all(&tmp);
    r.map(|_| target)
}

/// Render `frames` of `scene` into `dir` (one engine process per frame, a few
/// at a time), skipping frames that are already there. Paths come back in the
/// order of `frames`.
pub fn render_frames(
    config: &ServerConfig,
    scene: &Path,
    dir: &Path,
    frames: &[u32],
) -> Result<Vec<PathBuf>, String> {
    let mut wanted: Vec<u32> = frames.to_vec();
    wanted.sort_unstable();
    wanted.dedup();
    let results = proc::par_map(&wanted, proc::threads(), |&f| {
        render_one(config, scene, dir, f)
    });
    if let Some(e) = results.into_iter().find_map(Result::err) {
        return Err(e);
    }
    Ok(frames.iter().map(|f| frame_path(dir, *f)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spread_picks_evenly_and_keeps_the_ends() {
        assert_eq!(spread(3, 4), vec![1, 2, 3]);
        assert_eq!(spread(4, 4), vec![1, 2, 3, 4]);
        assert_eq!(spread(5, 4), vec![1, 2, 4, 5]);
        assert_eq!(spread(8, 4), vec![1, 3, 6, 8]);
        assert_eq!(spread(12, 4), vec![1, 5, 8, 12]);
        assert_eq!(spread(9, 1), vec![1]);
        for total in 1..=12 {
            let s = spread(total, 4);
            assert!(s.len() <= 4 && s.first() == Some(&1));
            assert_eq!(s.last(), Some(&total));
            assert!(s.windows(2).all(|w| w[0] < w[1]));
        }
    }
}
