//! Frame sequence export (PNG) and MP4 encoding via the system `ffmpeg`.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::scene::MotionProject;
use motion_core::timeline::evaluate_frame;
use rayon::prelude::*;

use crate::{CpuRenderer, RenderError, Renderer};

pub fn frame_file_name(frame: u32) -> String {
    format!("frame_{frame:06}.png")
}

/// Which frames to render.
#[derive(Debug, Clone, Copy)]
pub enum FrameRange {
    All,
    Only(u32),
}

/// Options of [`render_frames_with`]. The default is the CPU reference path.
#[derive(Debug, Clone, Copy, Default)]
pub struct RenderOptions {
    /// Run the post effects on the GPU (`gpu` feature). When no GPU is
    /// available, or this build has no GPU support, rendering falls back to the
    /// CPU post effects and [`RenderReport::gpu_notice`] says why.
    pub gpu_post: bool,
}

/// What [`render_frames_with`] produced.
#[derive(Debug, Clone, Default)]
pub struct RenderReport {
    /// Written PNG paths in frame order.
    pub frames: Vec<PathBuf>,
    /// One line to show the user when the GPU post backend was requested but
    /// not used (unavailable at start, or it failed mid-render and the CPU took
    /// over). `None` when the CPU was asked for or the GPU worked throughout.
    pub gpu_notice: Option<String>,
}

/// Render frames of `project` to `out_dir` as PNGs, in parallel. Existing
/// `frame_*.png` files in `out_dir` are removed first so stale frames never
/// leak into the encode. Returns the written paths in frame order.
pub fn render_frames(
    project: &MotionProject,
    base_dir: &Path,
    out_dir: &Path,
    range: FrameRange,
) -> Result<Vec<PathBuf>, RenderError> {
    Ok(render_frames_with(project, base_dir, out_dir, range, RenderOptions::default())?.frames)
}

/// Attach the GPU post backend when requested; the notice says why not.
#[cfg(feature = "gpu")]
fn attach_gpu(renderer: CpuRenderer) -> (CpuRenderer, Option<String>) {
    match crate::gpu::GpuPost::new() {
        Ok(gpu) => (renderer.with_gpu_post(gpu), None),
        Err(e) => (
            renderer,
            Some(format!("gpu: unavailable ({e}), using CPU post effects")),
        ),
    }
}

#[cfg(not(feature = "gpu"))]
fn attach_gpu(renderer: CpuRenderer) -> (CpuRenderer, Option<String>) {
    (
        renderer,
        Some(
            "gpu: unavailable (built without the `gpu` feature), using CPU post effects"
                .to_string(),
        ),
    )
}

/// [`render_frames`] with [`RenderOptions`] (GPU post effects) and a report.
pub fn render_frames_with(
    project: &MotionProject,
    base_dir: &Path,
    out_dir: &Path,
    range: FrameRange,
    options: RenderOptions,
) -> Result<RenderReport, RenderError> {
    std::fs::create_dir_all(out_dir)?;
    if matches!(range, FrameRange::All) {
        for entry in std::fs::read_dir(out_dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("frame_") && name.ends_with(".png") {
                std::fs::remove_file(&path)?;
            }
        }
    }
    let renderer = CpuRenderer::new(project, base_dir)?;
    let (renderer, gpu_notice) = if options.gpu_post {
        attach_gpu(renderer)
    } else {
        (renderer, None)
    };
    let frame_numbers: Vec<u32> = match range {
        FrameRange::All => (0..project.frame_count()).collect(),
        FrameRange::Only(f) => vec![f],
    };
    let frames = frame_numbers
        .par_iter()
        .map(|&f| {
            let resolved = evaluate_frame(project, f)?;
            let pm = renderer.render(&resolved)?;
            let path = out_dir.join(frame_file_name(f));
            pm.save_png(&path)
                .map_err(|e| RenderError::Png(format!("{}: {e}", path.display())))?;
            Ok(path)
        })
        .collect::<Result<Vec<PathBuf>, RenderError>>()?;
    #[cfg(feature = "gpu")]
    let gpu_notice = renderer
        .gpu_post_failure()
        .map(|why| format!("gpu: post effects failed ({why}), used CPU post effects"))
        .or(gpu_notice);
    Ok(RenderReport { frames, gpu_notice })
}

/// Encode `frame_%06d.png` in `frames_dir` to H.264 / yuv420p MP4.
pub fn encode_mp4(frames_dir: &Path, fps: u32, output: &Path) -> Result<(), RenderError> {
    let pattern = frames_dir.join("frame_%06d.png");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-framerate"])
        .arg(fps.to_string())
        .arg("-i")
        .arg(&pattern)
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-crf",
            "17",
            // Cap the rate: film grain and glitch noise are incompressible and
            // would otherwise push a 30 s reel past 100 Mbps.
            "-maxrate",
            "16M",
            "-bufsize",
            "32M",
            "-preset",
            "medium",
            "-movflags",
            "+faststart",
        ])
        .arg("-r")
        .arg(fps.to_string())
        .arg(output)
        .status()
        .map_err(|e| {
            RenderError::Encode(format!("could not run ffmpeg (is it installed?): {e}"))
        })?;
    if !status.success() {
        return Err(RenderError::Encode(format!("ffmpeg exited with {status}")));
    }
    Ok(())
}
