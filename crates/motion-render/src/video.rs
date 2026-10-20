//! (0.24 M6) Video plates: MP4/MOV/WEBM frames decoded by an `ffmpeg` child
//! that writes raw RGBA to a pipe. No temp files, no PNGs.
//!
//! A cache miss for frame `f` decodes the aligned window containing `f`
//! (up to [`WINDOW`] frames) at the layer's on-screen size, so the parallel
//! frames of one render chunk share a single ffmpeg run. Decoded frames live in
//! a byte-bounded LRU (`MOTION_VIDEO_CACHE_MB`, default 256). Decoding is
//! deterministic, so the cache never changes pixels.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard};

use resvg::tiny_skia::Pixmap;

use crate::RenderError;

/// Frames decoded per cache miss (fewer when the budget is small).
pub const WINDOW: u32 = 32;
/// Default cache budget in MB.
pub const DEFAULT_CACHE_MB: usize = 256;

/// One video asset: file and frame clock (`frame_count` frames at `fps`).
#[derive(Debug, Clone)]
pub struct VideoSource {
    pub path: PathBuf,
    pub fps: f64,
    pub frame_count: u32,
}

/// Facts about a video file from ffprobe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoProbe {
    pub fps: f64,
    pub frame_count: u32,
    pub duration_s: f64,
}

fn missing_tool(tool: &str, e: &std::io::Error) -> RenderError {
    RenderError::Encode(format!(
        "cannot run {tool} ({e}); video plates need ffmpeg and ffprobe on PATH"
    ))
}

/// `r_frame_rate` like `30000/1001` or `30`.
fn parse_rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    (d > 0.0 && n > 0.0).then_some(n / d)
}

/// Probe the first video stream of `path` (fps, frame count, duration).
pub fn probe(path: &Path) -> Result<VideoProbe, RenderError> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=r_frame_rate,duration,nb_frames:format=duration",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .map_err(|e| missing_tool("ffprobe", &e))?;
    if !out.status.success() {
        return Err(RenderError::Asset(format!(
            "ffprobe {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let get = |k: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(k).and_then(|v| v.strip_prefix('=')))
            .map(str::trim)
    };
    let fps = get("r_frame_rate")
        .and_then(parse_rate)
        .ok_or_else(|| RenderError::Asset(format!("{}: no video frame rate", path.display())))?;
    let duration_s = text
        .lines()
        .filter_map(|l| l.strip_prefix("duration=")?.trim().parse::<f64>().ok())
        .next()
        .ok_or_else(|| RenderError::Asset(format!("{}: no video duration", path.display())))?;
    let frame_count = get("nb_frames")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or_else(|| (duration_s * fps).round() as u32)
        .max(1);
    Ok(VideoProbe {
        fps,
        frame_count,
        duration_s,
    })
}

/// Decode `n` frames starting at frame `start` of a clip played at `fps`,
/// cover-cropped to exactly `w`x`h`. Returns fewer frames at the clip's end.
pub fn decode_window(
    src: &VideoSource,
    start: u32,
    n: u32,
    w: u32,
    h: u32,
) -> Result<Vec<Pixmap>, RenderError> {
    let t = f64::from(start) / src.fps;
    let vf = format!(
        "fps={fps},scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}",
        fps = src.fps
    );
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-ss"])
        .arg(format!("{t:.6}"))
        .arg("-i")
        .arg(&src.path)
        .args(["-an", "-frames:v"])
        .arg(n.to_string())
        .args(["-vf", &vf, "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| missing_tool("ffmpeg", &e))?;
    // Frame by frame straight into pixmaps: no whole-window byte buffer (a
    // 32-frame 1080p window would be 265 MB held twice).
    let frame_bytes = w as usize * h as usize * 4;
    let mut frames: Vec<Pixmap> = Vec::new();
    if frame_bytes > 0 {
        if let Some(mut so) = child.stdout.take() {
            while frames.len() < n as usize {
                let mut pm = Pixmap::new(w, h).ok_or(RenderError::Canvas(w, h))?;
                if so.read_exact(pm.data_mut()).is_err() {
                    break;
                }
                frames.push(pm);
            }
        }
    }
    let mut err = String::new();
    if frames.len() < n as usize {
        // Short clip or failure: let ffmpeg finish and report.
        if let Some(mut se) = child.stderr.take() {
            let _ = se.read_to_string(&mut err);
        }
    }
    drop(child.stdout.take());
    let status = child.wait()?;
    if frames.is_empty() || (!status.success() && frames.len() < n as usize && !err.is_empty()) {
        return Err(RenderError::Asset(format!(
            "video {}: ffmpeg decode of frame {start} failed: {}",
            src.path.display(),
            err.trim()
        )));
    }
    Ok(frames)
}

type Key = (String, u32, u32, u32);

#[derive(Default)]
struct Inner {
    frames: HashMap<Key, (Arc<Pixmap>, u64)>,
    bytes: usize,
    tick: u64,
}

/// Byte-bounded LRU of decoded video frames shared by parallel renders.
pub struct VideoCache {
    budget: usize,
    inner: Mutex<Inner>,
    /// Serialises decodes so threads missing the same window decode it once.
    decode: Mutex<()>,
}

impl VideoCache {
    pub fn new(budget_bytes: usize) -> Self {
        VideoCache {
            budget: budget_bytes,
            inner: Mutex::new(Inner::default()),
            decode: Mutex::new(()),
        }
    }

    /// Budget from `MOTION_VIDEO_CACHE_MB` (default 256).
    pub fn from_env() -> Self {
        let mb = std::env::var("MOTION_VIDEO_CACHE_MB")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_CACHE_MB);
        Self::new(mb.saturating_mul(1024 * 1024))
    }

    /// Bytes currently held.
    pub fn bytes(&self) -> usize {
        self.lock().bytes
    }

    pub fn budget(&self) -> usize {
        self.budget
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn get(&self, key: &Key) -> Option<Arc<Pixmap>> {
        let mut g = self.lock();
        g.tick += 1;
        let tick = g.tick;
        g.frames.get_mut(key).map(|(pm, t)| {
            *t = tick;
            pm.clone()
        })
    }

    fn insert(&self, key: Key, pm: Arc<Pixmap>) {
        let mut g = self.lock();
        g.tick += 1;
        let tick = g.tick;
        let size = pm.data().len();
        if let Some((old, _)) = g.frames.insert(key, (pm, tick)) {
            g.bytes -= old.data().len();
        }
        g.bytes += size;
        while g.bytes > self.budget {
            let Some(oldest) = g
                .frames
                .iter()
                .min_by_key(|(_, (_, t))| *t)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some((pm, _)) = g.frames.remove(&oldest) {
                g.bytes -= pm.data().len();
            }
        }
    }

    /// Frame `frame` of `asset` at `w`x`h` px: from the cache, else decode the
    /// aligned window containing it (clamped to the clip). A clip shorter than
    /// declared holds its last decoded frame.
    pub fn frame(
        &self,
        asset: &str,
        src: &VideoSource,
        frame: u32,
        w: u32,
        h: u32,
    ) -> Result<Arc<Pixmap>, RenderError> {
        let frame = frame.min(src.frame_count.saturating_sub(1));
        let key = (asset.to_string(), w, h, frame);
        if let Some(hit) = self.get(&key) {
            return Ok(hit);
        }
        let _decoding = self
            .decode
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(hit) = self.get(&key) {
            return Ok(hit);
        }
        let frame_bytes = (w as usize * h as usize * 4).max(1);
        let window = ((self.budget / frame_bytes) as u32).clamp(1, WINDOW);
        let start = frame / window * window;
        let n = window.min(src.frame_count - start);
        let frames = decode_window(src, start, n, w, h)?;
        let mut wanted = None;
        let last = frames.len().saturating_sub(1);
        for (i, pm) in frames.into_iter().enumerate() {
            let f = start + i as u32;
            let pm = Arc::new(pm);
            if f == frame || (i == last && f < frame) {
                wanted = Some(pm.clone());
            }
            self.insert((asset.to_string(), w, h, f), pm);
        }
        wanted.ok_or_else(|| {
            RenderError::Asset(format!(
                "video {}: frame {frame} was not decoded",
                src.path.display()
            ))
        })
    }
}
