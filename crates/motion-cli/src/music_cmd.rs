//! `music-index` (0.8).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use motion_render::music::{index_music, MusicIndexOptions};

fn absolute(p: &Path) -> PathBuf {
    std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Writes `<track stem>.music.json` beside the track unless `output` is given
/// (the track path is stored relative to the output file's directory).
pub fn cmd_music_index(
    track: &Path,
    output: Option<&Path>,
    cache: Option<&Path>,
    json: bool,
) -> Result<()> {
    let out_path: PathBuf = match output {
        Some(o) => o.to_path_buf(),
        None => {
            let stem = track
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "track".to_string());
            track
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(format!("{stem}.music.json"))
        }
    };
    let out_dir = match out_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let track_rel = pathdiff::diff_paths(absolute(track), absolute(&out_dir))
        .unwrap_or_else(|| track.to_path_buf())
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");

    let opts = MusicIndexOptions {
        cache_dir: cache.map(Path::to_path_buf),
    };
    let plan = index_music(track, &track_rel, &opts)
        .with_context(|| format!("music-index {}", track.display()))?;

    let mut text = serde_json::to_string_pretty(&plan)?;
    text.push('\n');
    std::fs::create_dir_all(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    std::fs::write(&out_path, &text).with_context(|| format!("write {}", out_path.display()))?;

    if json {
        println!("{}", text.trim_end());
    } else {
        println!(
            "bpm {:.1}, {} beats, {} downbeats, gain {:.1} dB -> {}",
            plan.bpm,
            plan.beat_times.len(),
            plan.downbeat_times.len(),
            plan.gain_db,
            out_path.display()
        );
    }
    Ok(())
}
