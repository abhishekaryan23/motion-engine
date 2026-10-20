//! (0.24 M6) Video plate decoding: ffmpeg pipe, window cache, byte budget,
//! no files left behind.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_render::video::{probe, VideoCache, VideoSource};

/// A scratch dir removed on drop.
struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("me-video-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Scratch(d)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 64x48 @ 30 fps, 60 frames; red channel of frame N is N*4, blue 255.
fn make_clip(dir: &Path) -> VideoSource {
    let path = dir.join("clip.mp4");
    let st = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("color=c=black:s=64x48:r=30,format=gbrp,geq=r='N*4':g='0':b='255'")
        .args([
            "-t", "2", "-c:v", "libx264", "-pix_fmt", "yuv444p", "-crf", "0",
        ])
        .arg(&path)
        .status()
        .expect("ffmpeg must be installed to run the video tests");
    assert!(st.success());
    let p = probe(&path).unwrap();
    assert_eq!((p.fps, p.frame_count), (30.0, 60));
    VideoSource {
        path,
        fps: p.fps,
        frame_count: p.frame_count,
    }
}

fn red(pm: &motion_render::tiny_skia::Pixmap) -> i32 {
    i32::from(pm.data()[(3 * pm.width() as usize + 3) * 4])
}

#[test]
fn frame_n_has_expected_colour_and_cover_crops() {
    let s = Scratch::new("colour");
    let src = make_clip(&s.0);
    let cache = VideoCache::new(64 * 1024 * 1024);
    for n in [0u32, 7, 33, 59] {
        let f = cache.frame("v", &src, n, 64, 48).unwrap();
        assert_eq!((f.width(), f.height()), (64, 48));
        assert!(
            (red(&f) - (n * 4) as i32).abs() <= 6,
            "frame {n}: {}",
            red(&f)
        );
    }
    // Different on-screen size (cover crop to a tall box).
    let tall = cache.frame("v", &src, 20, 24, 48).unwrap();
    assert_eq!((tall.width(), tall.height()), (24, 48));
    assert!((red(&tall) - 80).abs() <= 6);
    // Past the end holds the last frame.
    let held = cache.frame("v", &src, 500, 64, 48).unwrap();
    assert!((red(&held) - 59 * 4).abs() <= 6);
}

#[test]
fn cache_stays_within_budget_and_leaves_no_files() {
    let s = Scratch::new("budget");
    let src = make_clip(&s.0);
    let frame_bytes = 64 * 48 * 4;
    let budget = frame_bytes * 5;
    let cache = VideoCache::new(budget);
    for n in (0..60).step_by(3) {
        let f = cache.frame("v", &src, n, 64, 48).unwrap();
        assert!((red(&f) - (n * 4) as i32).abs() <= 6);
        assert!(cache.bytes() <= budget, "{} > {budget}", cache.bytes());
    }
    let files: Vec<_> = std::fs::read_dir(&s.0)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["clip.mp4"], "decoding must not write files");
    let dir = s.0.clone();
    drop(s);
    assert!(!dir.exists());
}

#[test]
fn missing_file_is_an_error() {
    let src = VideoSource {
        path: PathBuf::from("/nonexistent/none.mp4"),
        fps: 30.0,
        frame_count: 10,
    };
    assert!(VideoCache::new(1 << 20).frame("v", &src, 0, 8, 8).is_err());
}
