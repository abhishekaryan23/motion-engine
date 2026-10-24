//! Frame sequence export (PNG) and MP4 encoding via the system `ffmpeg`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
#[derive(Debug, Clone, Default)]
pub struct RenderOptions {
    /// Run the post effects on the GPU (`gpu` feature). When no GPU is
    /// available, or this build has no GPU support, rendering falls back to the
    /// CPU post effects and [`RenderReport::gpu_notice`] says why.
    pub gpu_post: bool,
    /// (0.24 M10) Operator brand mark: PNG path and default corner. Drawn as the
    /// top layer of every frame; not part of the scene.
    pub brand_mark: Option<(PathBuf, crate::brand_mark::Corner)>,
}

/// The renderer with the brand mark attached when `options` asks for one.
fn with_mark(
    renderer: CpuRenderer,
    project: &MotionProject,
    options: &RenderOptions,
) -> Result<CpuRenderer, RenderError> {
    let Some((png, corner)) = &options.brand_mark else {
        return Ok(renderer);
    };
    let mut mark = crate::brand_mark::BrandMark::from_png(
        png,
        *corner,
        (project.canvas.width, project.canvas.height),
        project.canvas.fps,
        project.canvas.background,
    )?;
    mark.plan_for(project, *corner);
    Ok(renderer.with_brand_mark(mark))
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
    let renderer = with_mark(CpuRenderer::new(project, base_dir)?, project, &options)?;
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

/// (0.24 M9) How hard the MP4 encode works. Film grain and glitch noise are
/// incompressible, so every tier caps the rate: a 60 s reel at the old
/// crf 17 / 16M came out at 120 MB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    /// crf 17, `-maxrate 16M -bufsize 32M`, preset medium: the archive copy.
    Master,
    /// crf 20, `-maxrate 8M -bufsize 16M`, preset medium: what ships.
    #[default]
    Delivery,
    /// Half width and height, crf 28, preset veryfast: a quick look.
    Proxy,
}

impl Quality {
    pub fn parse(s: &str) -> Option<Quality> {
        match s {
            "master" => Some(Quality::Master),
            "delivery" => Some(Quality::Delivery),
            "proxy" => Some(Quality::Proxy),
            _ => None,
        }
    }

    /// The rate-control arguments (`-crf` ... `-preset`), after the codec.
    pub fn rate_args(self) -> &'static [&'static str] {
        match self {
            Quality::Master => &[
                "-crf", "17", "-maxrate", "16M", "-bufsize", "32M", "-preset", "medium",
            ],
            Quality::Delivery => &[
                "-crf", "20", "-maxrate", "8M", "-bufsize", "16M", "-preset", "medium",
            ],
            Quality::Proxy => &["-crf", "28", "-preset", "veryfast"],
        }
    }
}

/// Encode `frame_%06d.png` in `frames_dir` to H.264 / yuv420p MP4 at
/// [`Quality::Delivery`].
pub fn encode_mp4(frames_dir: &Path, fps: u32, output: &Path) -> Result<(), RenderError> {
    encode_mp4_with(frames_dir, fps, output, Quality::Delivery)
}

/// [`encode_mp4`] at `quality` (a proxy halves the frame size).
pub fn encode_mp4_with(
    frames_dir: &Path,
    fps: u32,
    output: &Path,
    quality: Quality,
) -> Result<(), RenderError> {
    let pattern = frames_dir.join("frame_%06d.png");
    let status = Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-framerate"])
        .arg(fps.to_string())
        .arg("-i")
        .arg(&pattern)
        .args(if quality == Quality::Proxy {
            &["-vf", "scale=iw/2:-2"][..]
        } else {
            &[][..]
        })
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .args(quality.rate_args())
        .args(["-movflags", "+faststart"])
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

/// Run `f` on a local rayon pool of `n` threads (at least 1).
pub fn with_threads<T: Send>(n: usize, f: impl FnOnce() -> T + Send) -> Result<T, RenderError> {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(n.max(1))
        .build()
        .map_err(|e| RenderError::Encode(format!("could not build the thread pool: {e}")))?;
    Ok(pool.install(f))
}

/// Direct zero-disk streaming render to MP4 via FFmpeg stdin pipe.
/// Frames are rendered in parallel chunks with Rayon and streamed directly to FFmpeg.
/// No intermediate PNG files are written to disk, reducing disk consumption to 0 bytes.
pub fn render_to_mp4_stream(
    project: &MotionProject,
    base_dir: &Path,
    output: &Path,
    options: RenderOptions,
) -> Result<RenderReport, RenderError> {
    render_to_mp4_stream_ext(project, base_dir, output, options, false)
}

/// [`render_to_mp4_stream`] with `proxy`: half width/height, crf 28, veryfast.
pub fn render_to_mp4_stream_ext(
    project: &MotionProject,
    base_dir: &Path,
    output: &Path,
    options: RenderOptions,
    proxy: bool,
) -> Result<RenderReport, RenderError> {
    let quality = if proxy {
        Quality::Proxy
    } else {
        Quality::Delivery
    };
    render_to_mp4_stream_quality(project, base_dir, output, options, quality)
}

/// [`render_to_mp4_stream`] at `quality`.
/// On any failure ffmpeg is killed and reaped and the partial MP4 removed.
pub fn render_to_mp4_stream_quality(
    project: &MotionProject,
    base_dir: &Path,
    output: &Path,
    options: RenderOptions,
    quality: Quality,
) -> Result<RenderReport, RenderError> {
    let proxy = quality == Quality::Proxy;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let renderer = with_mark(CpuRenderer::new(project, base_dir)?, project, &options)?;
    let (renderer, gpu_notice) = if options.gpu_post {
        attach_gpu(renderer)
    } else {
        (renderer, None)
    };

    let w = project.canvas.width;
    let h = project.canvas.height;
    let fps = project.canvas.fps;
    let total_frames = project.frame_count();

    let fps_s = fps.to_string();
    let size = format!("{w}x{h}");
    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-y",
        "-loglevel",
        "error",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "rgba",
        "-s",
        &size,
        "-r",
        &fps_s,
        "-i",
        "pipe:0",
    ]);
    if proxy {
        cmd.args(["-vf", "scale=iw/2:-2"]);
    }
    cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p"]);
    cmd.args(quality.rate_args());
    cmd.args(["-movflags", "+faststart", "-r", &fps_s])
        .arg(output)
        .stdin(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        RenderError::Encode(format!("could not spawn ffmpeg (is it installed?): {e}"))
    })?;

    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(output);
        return Err(RenderError::Encode(
            "could not acquire ffmpeg stdin pipe".to_string(),
        ));
    };

    // Render in bounded parallel chunks (16 frames) to keep RAM footprint low (<250 MB)
    // while saturating CPU cores across rayon workers.
    let streamed = (|| -> Result<(), RenderError> {
        let chunk_size = 16usize;
        for chunk_start in (0..total_frames).step_by(chunk_size) {
            let chunk_end = (chunk_start + chunk_size as u32).min(total_frames);
            let frame_numbers: Vec<u32> = (chunk_start..chunk_end).collect();
            let rendered_chunk = frame_numbers
                .par_iter()
                .map(|&f| {
                    let resolved = evaluate_frame(project, f)?;
                    renderer.render(&resolved)
                })
                .collect::<Result<Vec<_>, RenderError>>()?;
            for pm in rendered_chunk {
                stdin.write_all(pm.data()).map_err(|e| {
                    RenderError::Encode(format!("failed to stream frame to ffmpeg: {e}"))
                })?;
            }
        }
        Ok(())
    })();
    drop(stdin);
    if let Err(e) = streamed {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(output);
        return Err(e);
    }

    let status = child
        .wait()
        .map_err(|e| RenderError::Encode(format!("ffmpeg wait error: {e}")))?;
    if !status.success() {
        let _ = std::fs::remove_file(output);
        return Err(RenderError::Encode(format!("ffmpeg exited with {status}")));
    }

    #[cfg(feature = "gpu")]
    let gpu_notice = renderer
        .gpu_post_failure()
        .map(|why| format!("gpu: post effects failed ({why}), used CPU post effects"))
        .or(gpu_notice);

    Ok(RenderReport {
        frames: Vec::new(),
        gpu_notice,
    })
}
