//! Reference media ingestion (0.7): ffprobe metadata, analysis stream, sample
//! extraction, deterministic sample selection and the contact sheet.
//! Fixtures are generated with ffmpeg lavfi into a temp dir (never committed).

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::reference::evidence::{Orientation, ReferenceSample, SampleKind};
use motion_render::decode::decode_file;
use motion_render::reference::contact::contact_sheet;
use motion_render::reference::media::{
    analysis_fps, analysis_size, decode_analysis_stream, extract_sample, ffmpeg_version,
    media_fingerprint, probe, ANALYSIS_SHORT_SIDE, SAMPLE_SHORT_SIDE,
};
use motion_render::reference::sample::{sample_budget, select_samples};
use motion_render::reference::signal::{ChangeEvent, ChangeKind, TemporalSignals};

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("motion_ref_media_{}_{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Encode a lavfi source to `out` (h264, yuv420p); `with_audio` adds a sine track.
fn make_video(out: &Path, size: &str, rate: u32, seconds: u32, with_audio: bool) {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
        .arg(format!(
            "testsrc2=size={size}:rate={rate}:duration={seconds}"
        ));
    if with_audio {
        cmd.args(["-f", "lavfi", "-i"])
            .arg(format!("sine=duration={seconds}"))
            .arg("-shortest");
    }
    let status = cmd
        .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
        .arg(out)
        .status()
        .expect("run ffmpeg");
    assert!(status.success(), "ffmpeg failed for {}", out.display());
}

#[test]
fn probe_short_vertical_clip() {
    let dir = temp_dir("probe");
    let silent = dir.join("silent.mp4");
    make_video(&silent, "360x640", 30, 3, false);
    let m = probe(&silent).expect("probe");
    assert_eq!((m.width, m.height), (360, 640));
    assert!((m.fps - 30.0).abs() < 1e-6, "fps {}", m.fps);
    assert!(
        (m.duration_seconds - 3.0).abs() < 0.1,
        "{}",
        m.duration_seconds
    );
    assert_eq!(m.frame_count, 90);
    assert_eq!(m.orientation, Orientation::Vertical);
    assert_eq!(m.video_codec, "h264");
    assert!(!m.audio_present);
    assert!((m.aspect_ratio - 360.0 / 640.0).abs() < 1e-9);

    let with_audio = dir.join("audio.mp4");
    make_video(&with_audio, "360x640", 30, 3, true);
    assert!(probe(&with_audio).expect("probe").audio_present);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn probe_rejects_non_video() {
    let dir = temp_dir("probe_bad");
    let path = dir.join("notes.txt");
    std::fs::write(&path, b"not a video").expect("write");
    assert!(probe(&path).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn probe_uses_video_duration_when_audio_runs_longer() {
    let dir = temp_dir("long_audio");
    let clip = dir.join("clip.mp4");
    let status = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-y", "-f", "lavfi", "-i"])
        .arg("testsrc2=size=64x64:rate=30:duration=1")
        .args([
            "-f",
            "lavfi",
            "-i",
            "sine=duration=2",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&clip)
        .status()
        .expect("run ffmpeg");
    assert!(status.success());
    let m = probe(&clip).expect("probe");
    assert!(m.audio_present);
    assert!(
        (m.duration_seconds - 1.0).abs() < 0.1,
        "{}",
        m.duration_seconds
    );
    assert_eq!(m.frame_count, 30);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ffmpeg_version_line() {
    let v = ffmpeg_version().expect("version");
    assert!(v.starts_with("ffmpeg version"), "{v}");
}

#[test]
fn longer_clip_budget_and_analysis_stream() {
    let dir = temp_dir("long");
    let long = dir.join("long.mp4");
    make_video(&long, "64x114", 24, 40, false);
    let m = probe(&long).expect("probe");
    assert!((m.duration_seconds - 40.0).abs() < 0.2);
    assert_eq!(m.frame_count, 960);
    assert_eq!(sample_budget(m.duration_seconds), 28);
    assert_eq!(analysis_fps(&m), 24.0);
    let stream = decode_analysis_stream(&long, &m, 32).expect("stream");
    assert_eq!((stream.width, stream.height), (32, 58));
    let expected = (m.duration_seconds * stream.fps).round() as i64;
    assert!(
        (stream.len() as i64 - expected).abs() <= 1,
        "{} vs {expected}",
        stream.len()
    );
    assert!(stream
        .frames
        .iter()
        .all(|f| f.len() == (stream.width * stream.height * 3) as usize));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analysis_stream_is_96_by_170_for_360_by_640() {
    let dir = temp_dir("stream");
    let clip = dir.join("clip.mp4");
    make_video(&clip, "360x640", 30, 2, false);
    let m = probe(&clip).expect("probe");
    let a = decode_analysis_stream(&clip, &m, ANALYSIS_SHORT_SIDE).expect("stream");
    let b = decode_analysis_stream(&clip, &m, ANALYSIS_SHORT_SIDE).expect("stream");
    assert_eq!((a.width, a.height), (96, 170));
    assert_eq!(a.fps, 30.0);
    assert!((a.len() as i64 - 60).abs() <= 1, "{}", a.len());
    assert_eq!(a, b, "decoding is deterministic");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn landscape_and_square_orientation_and_size() {
    let dir = temp_dir("orient");
    let wide = dir.join("wide.mp4");
    let square = dir.join("square.mp4");
    make_video(&wide, "640x360", 30, 1, false);
    make_video(&square, "200x200", 30, 1, false);
    let w = probe(&wide).expect("probe");
    let s = probe(&square).expect("probe");
    assert_eq!(w.orientation, Orientation::Landscape);
    assert_eq!(s.orientation, Orientation::Square);
    assert_eq!(analysis_size(&w, 96), (170, 96));
    assert_eq!(analysis_size(&s, 96), (96, 96));
    let stream = decode_analysis_stream(&wide, &w, 96).expect("stream");
    assert_eq!((stream.width, stream.height), (170, 96));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn analysis_fps_caps_rate_and_frame_count() {
    let dir = temp_dir("fps");
    let clip = dir.join("clip.mp4");
    make_video(&clip, "64x64", 60, 1, false);
    let mut m = probe(&clip).expect("probe");
    assert_eq!(analysis_fps(&m), 30.0);
    m.duration_seconds = 600.0;
    m.fps = 30.0;
    assert_eq!(analysis_fps(&m), 6.0);
    m.duration_seconds = 7000.0;
    assert_eq!(analysis_fps(&m), 0.514);
    m.duration_seconds = 7004.0;
    assert!(analysis_fps(&m) * m.duration_seconds <= 3600.0);
    m.duration_seconds = 10_000_000.0;
    assert!(analysis_fps(&m) > 0.0);
    assert!(analysis_fps(&m) * m.duration_seconds <= 3600.0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn extract_sample_is_decodable_jpeg_with_expected_size() {
    let dir = temp_dir("extract");
    let big = dir.join("big.mp4");
    let small = dir.join("small.mp4");
    make_video(&big, "720x1280", 30, 2, false);
    make_video(&small, "360x640", 30, 2, false);

    let out_big = dir.join("samples/big.jpg");
    extract_sample(&big, 1.0, &out_big, SAMPLE_SHORT_SIDE).expect("extract");
    let img = decode_file(&out_big).expect("decode");
    assert_eq!((img.pixmap.width(), img.pixmap.height()), (540, 960));

    let out_small = dir.join("small.jpg");
    extract_sample(&small, 0.5, &out_small, SAMPLE_SHORT_SIDE).expect("extract");
    let img = decode_file(&out_small).expect("decode");
    assert_eq!((img.pixmap.width(), img.pixmap.height()), (360, 640));

    let wide = dir.join("wide.mp4");
    make_video(&wide, "1280x720", 30, 1, false);
    let out_wide = dir.join("wide.jpg");
    extract_sample(&wide, 0.2, &out_wide, SAMPLE_SHORT_SIDE).expect("extract");
    let img = decode_file(&out_wide).expect("decode");
    assert_eq!((img.pixmap.width(), img.pixmap.height()), (960, 540));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fingerprint_is_stable_and_content_sensitive() {
    let dir = temp_dir("fp");
    let a = dir.join("a.bin");
    let b = dir.join("b.bin");
    // Larger than one 64 KiB chunk to exercise chunking.
    let bytes: Vec<u8> = (0..150_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&a, &bytes).expect("write");
    let mut other = bytes.clone();
    other[100_000] ^= 1;
    std::fs::write(&b, &other).expect("write");
    let fa1 = media_fingerprint(&a).expect("fp");
    let fa2 = media_fingerprint(&a).expect("fp");
    let fb = media_fingerprint(&b).expect("fp");
    assert_eq!(fa1, fa2);
    assert_ne!(fa1, fb);
    assert!(fa1.starts_with("fnv1a64-") && fa1.len() == "fnv1a64-".len() + 16);
    assert!(media_fingerprint(&dir.join("missing")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- sample selection on synthetic signals ----

fn cut(at: usize) -> ChangeEvent {
    ChangeEvent {
        start: at,
        end: at,
        peak: at,
        kind: ChangeKind::Cut,
        magnitude: 0.5,
    }
}

fn flat_diffs(frames: usize) -> Vec<f32> {
    vec![0.001; frames.saturating_sub(1)]
}

fn assert_well_formed(plans: &[motion_render::reference::sample::SamplePlan], frames: usize) {
    assert!(plans.windows(2).all(|w| w[0].frame < w[1].frame));
    assert!(plans
        .iter()
        .all(|p| p.frame < frames && !p.kinds.is_empty()));
    assert!(plans.iter().all(|p| {
        let mut k = p.kinds.clone();
        k.sort();
        k.dedup();
        k == p.kinds
    }));
}

#[test]
fn static_clip_is_periodic_and_stable_only() {
    let signals = TemporalSignals {
        diffs: flat_diffs(300),
        events: vec![],
        noise_floor: 0.0,
    };
    let plans = select_samples(&signals, 30.0, 300, 15);
    assert_well_formed(&plans, 300);
    assert!(plans.len() <= 15 && plans.len() >= 4);
    for p in &plans {
        assert!(p
            .kinds
            .iter()
            .all(|k| matches!(k, SampleKind::Periodic | SampleKind::Stable)));
    }
    assert!(plans.iter().any(|p| p.kinds.contains(&SampleKind::Stable)));
    // Stable sample sits in the middle of the clip.
    let stable = plans
        .iter()
        .find(|p| p.kinds.contains(&SampleKind::Stable))
        .expect("stable");
    assert!((stable.frame as i64 - 150).abs() <= 8, "{}", stable.frame);
}

#[test]
fn scene_cuts_get_a_settled_sample_after_each_cut() {
    let cuts = [60usize, 150, 240];
    let mut diffs = flat_diffs(300);
    for c in cuts {
        diffs[c - 1] = 0.5;
    }
    let signals = TemporalSignals {
        diffs,
        events: cuts.iter().map(|c| cut(*c)).collect(),
        noise_floor: 0.0,
    };
    let plans = select_samples(&signals, 30.0, 300, 20);
    assert_well_formed(&plans, 300);
    assert!(plans.len() <= 20);
    for c in cuts {
        let hit = plans
            .iter()
            .filter(|p| p.kinds.contains(&SampleKind::SceneChange))
            .filter(|p| p.frame >= c && p.frame < c + 30)
            .count();
        assert_eq!(hit, 1, "cut at {c}: {plans:?}");
    }
    // Never a scene_change before the first cut settles.
    assert!(plans
        .iter()
        .filter(|p| p.kinds.contains(&SampleKind::SceneChange))
        .all(|p| p.frame >= 60));
}

#[test]
fn scene_change_quota_keeps_largest_magnitudes() {
    let mut events: Vec<ChangeEvent> = (1..=10).map(|i| cut(i * 40)).collect();
    for (i, e) in events.iter_mut().enumerate() {
        e.magnitude = 0.1 + 0.01 * i as f32;
    }
    let signals = TemporalSignals {
        diffs: flat_diffs(500),
        events,
        noise_floor: 0.0,
    };
    // Budget 12 -> scene quota ceil(4.2) = 5.
    let plans = select_samples(&signals, 30.0, 500, 12);
    assert_well_formed(&plans, 500);
    assert!(plans.len() <= 12);
    let n = plans
        .iter()
        .filter(|p| p.kinds.contains(&SampleKind::SceneChange))
        .count();
    assert!(n <= 5, "{n}");
}

#[test]
fn high_motion_burst_is_sampled_inside_the_burst() {
    let mut diffs = flat_diffs(300);
    for d in diffs.iter_mut().take(180).skip(120) {
        *d = 0.08;
    }
    diffs[150] = 0.2;
    let signals = TemporalSignals {
        diffs,
        events: vec![],
        noise_floor: 0.0,
    };
    let plans = select_samples(&signals, 30.0, 300, 15);
    assert_well_formed(&plans, 300);
    let hm: Vec<_> = plans
        .iter()
        .filter(|p| p.kinds.contains(&SampleKind::HighMotion))
        .collect();
    assert!(!hm.is_empty(), "{plans:?}");
    assert!(hm.iter().all(|p| p.frame > 120 && p.frame <= 181));
}

#[test]
fn closing_motion_in_partial_window_is_sampled() {
    // 99 diffs at 30 fps: the last 9 are a partial half-second window.
    let mut diffs = flat_diffs(100);
    diffs[90..].fill(0.08);
    let signals = TemporalSignals {
        diffs,
        ..Default::default()
    };
    let plans = select_samples(&signals, 30.0, 100, 12);
    assert_well_formed(&plans, 100);
    assert!(
        plans
            .iter()
            .any(|p| p.frame >= 91 && p.kinds.contains(&SampleKind::HighMotion)),
        "{plans:?}"
    );
}

#[test]
fn long_clip_respects_budget_and_is_deterministic() {
    let frames = 3600;
    let mut diffs = flat_diffs(frames);
    let events: Vec<ChangeEvent> = (1..60).map(|i| cut(i * 60)).collect();
    for e in &events {
        diffs[e.start - 1] = 0.4;
    }
    for d in diffs.iter_mut().take(2000).skip(1900) {
        *d = 0.1;
    }
    let signals = TemporalSignals {
        diffs,
        events,
        noise_floor: 0.0,
    };
    let budget = sample_budget(120.0);
    assert_eq!(budget, 31);
    let a = select_samples(&signals, 30.0, frames, budget);
    let b = select_samples(&signals, 30.0, frames, budget);
    assert_eq!(a, b);
    assert!(a.len() <= budget && a.len() >= 4, "{}", a.len());
    assert_well_formed(&a, frames);
    for p in &a {
        assert_eq!(
            p.time_seconds,
            (p.frame as f64 / 30.0 * 1000.0).round() / 1000.0
        );
    }
}

#[test]
fn degenerate_inputs_do_not_panic() {
    let empty = TemporalSignals::default();
    assert!(select_samples(&empty, 30.0, 0, 12).is_empty());
    assert!(select_samples(&empty, 30.0, 10, 0).is_empty());
    let one = select_samples(&empty, 30.0, 1, 12);
    assert_eq!(one.len(), 1);
    let tiny = select_samples(&empty, 30.0, 3, 12);
    assert_well_formed(&tiny, 3);
}

// ---- contact sheet ----

#[test]
fn contact_sheet_has_expected_dimensions() {
    let dir = temp_dir("sheet");
    let clip = dir.join("clip.mp4");
    make_video(&clip, "360x640", 30, 3, false);
    let bundle = dir.join("bundle");
    let mut samples = Vec::new();
    for (i, (t, kind)) in [
        (0.5, SampleKind::Periodic),
        (1.5, SampleKind::SceneChange),
        (2.5, SampleKind::Stable),
    ]
    .into_iter()
    .enumerate()
    {
        let image = format!("samples/s{:02}.jpg", i + 1);
        extract_sample(&clip, t, &bundle.join(&image), SAMPLE_SHORT_SIDE).expect("extract");
        samples.push(ReferenceSample {
            id: format!("s{:02}", i + 1),
            time_seconds: t,
            frame: (t * 30.0) as u64,
            kinds: vec![kind],
            image,
        });
    }
    let out = dir.join("contact.png");
    contact_sheet(&bundle, &samples, &out).expect("sheet");
    let sheet = decode_file(&out).expect("decode sheet");
    // 6 columns of 180 + 7 gutters; 1 row of 320 + 26 label + 2 gutters.
    assert_eq!(sheet.pixmap.width(), 6 * 180 + 7 * 8);
    assert_eq!(sheet.pixmap.height(), 320 + 26 + 2 * 8);
    // Deterministic bytes.
    let again = dir.join("contact2.png");
    contact_sheet(&bundle, &samples, &again).expect("sheet");
    assert_eq!(
        std::fs::read(&out).expect("r"),
        std::fs::read(&again).expect("r")
    );
    // The scene_change bar is orange under the second thumbnail.
    let px = sheet
        .pixmap
        .pixel(8 + (180 + 8) + 90, 8 + 320 + 1)
        .expect("pixel");
    assert_eq!((px.red(), px.green(), px.blue()), (0xFF, 0x8A, 0x00));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn contact_sheet_rejects_empty_and_missing() {
    let dir = temp_dir("sheet_err");
    assert!(contact_sheet(&dir, &[], &dir.join("x.png")).is_err());
    let missing = ReferenceSample {
        id: "s01".into(),
        time_seconds: 0.0,
        frame: 0,
        kinds: vec![SampleKind::Periodic],
        image: "nope.jpg".into(),
    };
    assert!(contact_sheet(&dir, &[missing], &dir.join("x.png")).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
