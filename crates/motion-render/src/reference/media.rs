//! ffprobe metadata, the analysis stream and sample extraction.

use std::io::Read;
use std::path::Path;
use std::process::Command;

use motion_core::reference::evidence::{Fnv64, Orientation, ReferenceMetadata};

use super::signal::AnalysisStream;
use super::ReferenceError;

/// Short side of the analysis stream in pixels.
pub const ANALYSIS_SHORT_SIDE: u32 = 96;
/// Maximum analysis frame rate.
pub const ANALYSIS_MAX_FPS: f64 = 30.0;
/// Maximum number of analysis frames (longer videos lower the analysis fps).
pub const ANALYSIS_MAX_FRAMES: u64 = 3600;
/// Short side of extracted sample images.
pub const SAMPLE_SHORT_SIDE: u32 = 540;

fn probe_err(msg: impl ToString) -> ReferenceError {
    ReferenceError::Probe(msg.to_string())
}

/// Parse an ffprobe rational such as `30000/1001`. `None` for `0/0`, zero or garbage.
fn parse_rate(s: &str) -> Option<f64> {
    let v = match s.split_once('/') {
        Some((n, d)) => {
            let (n, d) = (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?);
            if d == 0.0 {
                return None;
            }
            n / d
        }
        None => s.trim().parse::<f64>().ok()?,
    };
    (v.is_finite() && v > 0.0).then_some(v)
}

fn json_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

/// Container and video-stream facts via `ffprobe`.
pub fn probe(path: &Path) -> Result<ReferenceMetadata, ReferenceError> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-show_entries"])
        .arg("stream=index,codec_type,codec_name,width,height,avg_frame_rate,r_frame_rate,nb_frames,duration:format=duration")
        .args(["-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| probe_err(format!("cannot run ffprobe: {e}")))?;
    if !out.status.success() {
        return Err(ReferenceError::Probe(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(probe_err)?;
    let streams = json
        .get("streams")
        .and_then(|s| s.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let codec_type = |s: &serde_json::Value| {
        s.get("codec_type")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let video = streams
        .iter()
        .find(|s| codec_type(s) == "video")
        .ok_or_else(|| ReferenceError::NoVideo(path.display().to_string()))?;
    let audio_present = streams.iter().any(|s| codec_type(s) == "audio");

    let dim = |k: &str| {
        video
            .get(k)
            .and_then(|v| v.as_u64())
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0)
    };
    let (width, height) = (dim("width"), dim("height"));
    if width == 0 || height == 0 {
        return Err(probe_err("video stream has no dimensions"));
    }
    let rate = |k: &str| video.get(k).and_then(|v| v.as_str()).and_then(parse_rate);
    let fps = rate("avg_frame_rate")
        .or_else(|| rate("r_frame_rate"))
        .ok_or_else(|| probe_err("video stream has no frame rate"))?;
    let duration_seconds = video
        .get("duration")
        .and_then(json_f64)
        .filter(|d| d.is_finite() && *d > 0.0)
        .or_else(|| {
            json.get("format")
                .and_then(|f| f.get("duration"))
                .and_then(json_f64)
                .filter(|d| d.is_finite() && *d >= 0.0)
        })
        .ok_or_else(|| probe_err("cannot determine duration"))?;
    let frame_count = video
        .get("nb_frames")
        .and_then(json_f64)
        .filter(|n| *n >= 0.0)
        .map(|n| n as u64)
        .unwrap_or_else(|| (duration_seconds * fps).round() as u64);
    let video_codec = video
        .get("codec_name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(ReferenceMetadata {
        width,
        height,
        duration_seconds,
        fps,
        frame_count,
        aspect_ratio: width as f64 / height as f64,
        orientation: Orientation::from_size(width, height),
        video_codec,
        audio_present,
    })
}

/// First line of `ffmpeg -version`.
pub fn ffmpeg_version() -> Result<String, ReferenceError> {
    let out = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map_err(|e| ReferenceError::Ffmpeg(format!("cannot run ffmpeg: {e}")))?;
    if !out.status.success() {
        return Err(ReferenceError::Ffmpeg(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .to_string())
}

/// `fnv1a64-<16 hex>` over the file bytes (streamed in 64 KiB chunks).
pub fn media_fingerprint(path: &Path) -> Result<String, ReferenceError> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Fnv64::default();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.write(&buf[..n]);
    }
    Ok(format!("fnv1a64-{:016x}", hasher.finish()))
}

/// Analysis frame size: the short side is `short_side`, the long side keeps the
/// aspect ratio and is rounded to the nearest even number (min 2).
pub fn analysis_size(meta: &ReferenceMetadata, short_side: u32) -> (u32, u32) {
    let (w, h) = (meta.width.max(1), meta.height.max(1));
    let short = w.min(h);
    let long = w.max(h);
    let long_scaled = long as f64 * short_side as f64 / short as f64;
    let even = (((long_scaled / 2.0).round() as u32) * 2).max(2);
    let short_side = short_side.max(1);
    match w.cmp(&h) {
        std::cmp::Ordering::Equal => (short_side, short_side),
        std::cmp::Ordering::Less => (short_side, even),
        std::cmp::Ordering::Greater => (even, short_side),
    }
}

/// Analysis frame rate: `min(fps, 30)`, lowered so at most 3600 frames result.
pub fn analysis_fps(meta: &ReferenceMetadata) -> f64 {
    let cap = if meta.duration_seconds > 0.0 {
        ANALYSIS_MAX_FRAMES as f64 / meta.duration_seconds
    } else {
        f64::INFINITY
    };
    let f = meta.fps.min(ANALYSIS_MAX_FPS).min(cap);
    let rounded = (f * 1000.0).round() / 1000.0;
    if rounded > 0.0 {
        rounded.min(cap)
    } else {
        f
    }
}

fn ffmpeg_err(stderr: &[u8]) -> ReferenceError {
    ReferenceError::Ffmpeg(String::from_utf8_lossy(stderr).trim().to_string())
}

/// Decode the whole video (no audio) to the low-resolution RGB analysis stream.
pub fn decode_analysis_stream(
    path: &Path,
    meta: &ReferenceMetadata,
    short_side: u32,
) -> Result<AnalysisStream, ReferenceError> {
    let (w, h) = analysis_size(meta, short_side);
    let fps = analysis_fps(meta);
    let filter = format!("fps={fps},scale={w}:{h}:flags=area,format=rgb24");
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
        .arg(&filter)
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .output()
        .map_err(|e| ReferenceError::Ffmpeg(format!("cannot run ffmpeg: {e}")))?;
    if !out.status.success() {
        return Err(ffmpeg_err(&out.stderr));
    }
    let frame_bytes = w as usize * h as usize * 3;
    let frames: Vec<Vec<u8>> = out
        .stdout
        .chunks_exact(frame_bytes)
        .map(<[u8]>::to_vec)
        .collect();
    Ok(AnalysisStream {
        width: w,
        height: h,
        fps,
        frames,
    })
}

/// Extract one frame at `time_seconds` as a JPEG whose short side is
/// `min(short_side, source short side)` (even dimensions).
pub fn extract_sample(
    path: &Path,
    time_seconds: f64,
    out: &Path,
    short_side: u32,
) -> Result<(), ReferenceError> {
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let s = short_side.max(2);
    // Landscape/square-wide sources fix the height, portrait/square fix the width;
    // the other side follows the aspect ratio (-2 keeps it even).
    let filter = format!(
        "scale='if(gt(iw,ih),-2,trunc(min({s},iw)/2)*2)':'if(gt(iw,ih),trunc(min({s},ih)/2)*2,-2)':flags=lanczos"
    );
    let result = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-ss"])
        .arg(format!("{:.3}", time_seconds.max(0.0)))
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-an", "-frames:v", "1", "-vf"])
        .arg(&filter)
        .args(["-q:v", "3", "-update", "1"])
        .arg(out)
        .output()
        .map_err(|e| ReferenceError::Ffmpeg(format!("cannot run ffmpeg: {e}")))?;
    if !result.status.success() {
        return Err(ffmpeg_err(&result.stderr));
    }
    if !out.is_file() {
        return Err(ReferenceError::Ffmpeg(format!(
            "no frame at {time_seconds:.3}s in {}",
            path.display()
        )));
    }
    Ok(())
}
