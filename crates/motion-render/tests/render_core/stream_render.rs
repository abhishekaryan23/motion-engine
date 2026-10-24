//! Test zero-disk streaming render to MP4 via direct FFmpeg pipe.

use motion_core::scene::MotionProject;
use motion_render::export::{render_to_mp4_stream, RenderOptions};
use std::path::{Path, PathBuf};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

fn load(file: &str) -> MotionProject {
    let path = golden_dir().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    MotionProject::from_json(&text).unwrap_or_else(|e| panic!("{file}: {e}"))
}

#[test]
fn test_render_to_mp4_stream_zero_disk_footprint() {
    let mut project = load("minimal_motion.motion.json");
    // Limit duration to 15 frames for quick deterministic test execution
    project.project.duration_seconds = Some(15.0 / project.canvas.fps as f64);

    let temp_dir = std::env::temp_dir().join(format!("motion_stream_test_{}", std::process::id()));
    std::fs::create_dir_all(&temp_dir).expect("create temp dir");
    let out_mp4 = temp_dir.join("streamed_video.mp4");

    let report = render_to_mp4_stream(&project, &golden_dir(), &out_mp4, RenderOptions::default())
        .expect("streaming render must succeed");

    // 1. Report must show 0 intermediate frames written to disk
    assert!(
        report.frames.is_empty(),
        "streaming render should not return written PNG paths"
    );

    // 2. The output MP4 must exist and be non-empty
    assert!(
        out_mp4.exists(),
        "streamed MP4 file must be created by FFmpeg"
    );
    let metadata = std::fs::metadata(&out_mp4).expect("read metadata");
    assert!(
        metadata.len() > 1000,
        "streamed MP4 should have reasonable encoded byte size, got {} bytes",
        metadata.len()
    );

    // 3. Verify zero PNG files were generated on disk in the output directory
    let png_count = std::fs::read_dir(&temp_dir)
        .expect("read dir")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
        .count();
    assert_eq!(
        png_count, 0,
        "zero PNG frames should exist on disk after streaming render"
    );

    // Cleanup test artifact
    let _ = std::fs::remove_file(&out_mp4);
    let _ = std::fs::remove_dir_all(&temp_dir);
}
