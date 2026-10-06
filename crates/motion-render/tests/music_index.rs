//! `music-index` (SOUND_DESIGN §7): tempo / beat / downbeat analysis on
//! synthetic click tracks generated with ffmpeg lavfi.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use motion_render::music::{index_music, MusicIndexOptions};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn ffmpeg_ok() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "motion_music_index_{}_{}_{}",
            std::process::id(),
            tag,
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        Scratch(dir)
    }
    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Click every beat, louder accent every 4th beat starting at `off`, with a
/// quiet low drone underneath.
fn click_track(bpm: f64, off: f64, dur: f64, out: &Path) -> bool {
    let p = 60.0 / bpm;
    let expr = format!(
        "(lt(mod(t-{off},{p}),0.03)*(0.4+0.5*lt(mod(t-{off},{q}),0.03)))*sin(2*PI*1000*t)*gte(t,{off})+0.03*sin(2*PI*110*t)",
        q = 4.0 * p
    );
    let src = format!("aevalsrc='{expr}':s=22050:d={dur}");
    Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-f", "lavfi", "-i", &src])
        .arg(out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn nearest_err(t: f64, truth: &[f64]) -> f64 {
    truth
        .iter()
        .map(|c| (c - t).abs())
        .fold(f64::INFINITY, f64::min)
}

#[test]
fn tempo_beats_downbeats_on_click_tracks() {
    if !ffmpeg_ok() {
        return;
    }
    let dur = 20.0;
    // (bpm, accent offset): offset < one period so beat 0 is the first click.
    for (bpm, off) in [(90.0, 0.25), (120.0, 0.2), (128.0, 0.31), (150.0, 0.17)] {
        let s = Scratch::new("click");
        let wav = s.path("click.wav");
        assert!(click_track(bpm, off, dur, &wav), "synth {bpm}");
        let plan = index_music(&wav, "click.wav", &MusicIndexOptions::default())
            .unwrap_or_else(|e| panic!("analyse {bpm}: {e}"));
        let period = 60.0 / bpm;

        let bpm_err = (plan.bpm - bpm).abs();
        assert!(bpm_err <= 1.0, "bpm {bpm}: got {}", plan.bpm);

        let n_true = ((dur - off) / period).ceil() as usize;
        let truth: Vec<f64> = (0..n_true).map(|k| off + k as f64 * period).collect();
        assert!(
            plan.beat_times.len().abs_diff(n_true) <= 1,
            "bpm {bpm}: {} beats vs {n_true}",
            plan.beat_times.len()
        );
        let mut max_err: f64 = 0.0;
        for &t in plan.beat_times.iter().filter(|&&t| t > 2.0) {
            max_err = max_err.max(nearest_err(t, &truth));
        }
        assert!(max_err <= 0.020, "bpm {bpm}: beat error {max_err}");

        let accents: Vec<f64> = (0..n_true)
            .step_by(4)
            .map(|k| off + k as f64 * period)
            .collect();
        assert!(!plan.downbeat_times.is_empty());
        for &d in &plan.downbeat_times {
            let e = nearest_err(d, &accents);
            assert!(e <= 0.020, "bpm {bpm}: downbeat {d} off by {e}");
        }
        assert!(plan.downbeat_times.len().abs_diff(accents.len()) <= 1);

        assert_eq!(plan.track, "click.wav");
        assert_eq!(plan.duration, 20.0);
        assert!((-30.0..=0.0).contains(&plan.gain_db));
        eprintln!(
            "bpm {bpm}: got {} (err {bpm_err:.2}), beat max err {:.1} ms, {} beats, {} downbeats",
            plan.bpm,
            max_err * 1000.0,
            plan.beat_times.len(),
            plan.downbeat_times.len()
        );
    }
}

#[test]
fn deterministic_and_cached() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("det");
    let wav = s.path("a.wav");
    assert!(click_track(120.0, 0.2, 12.0, &wav));
    let cache = s.path("cache");
    let opts = MusicIndexOptions {
        cache_dir: Some(cache.clone()),
    };
    let none = MusicIndexOptions::default();

    let plain1 = index_music(&wav, "a.wav", &none).expect("run1");
    let plain2 = index_music(&wav, "a.wav", &none).expect("run2");
    assert_eq!(
        serde_json::to_string(&plain1).expect("json"),
        serde_json::to_string(&plain2).expect("json")
    );

    let miss = index_music(&wav, "a.wav", &opts).expect("miss");
    assert_eq!(miss, plain1);
    let cached_files: Vec<_> = std::fs::read_dir(&cache).expect("cache dir").collect();
    assert_eq!(cached_files.len(), 1);
    let stored = std::fs::read_to_string(cached_files[0].as_ref().expect("entry").path())
        .expect("read cache");
    assert!(!stored.contains("a.wav"), "cache stores track = \"\"");

    let hit = index_music(&wav, "sub/b.wav", &opts).expect("hit");
    assert_eq!(hit.track, "sub/b.wav");
    let mut expected = plain1.clone();
    expected.track = "sub/b.wav".to_string();
    assert_eq!(hit, expected);
}

#[test]
fn rejects_short_and_non_audio() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("err");
    let short = s.path("short.wav");
    assert!(click_track(120.0, 0.2, 2.0, &short));
    let err = index_music(&short, "short.wav", &MusicIndexOptions::default())
        .expect_err("2 s is too short");
    assert!(err.to_string().contains("short.wav"), "{err}");

    let junk = s.path("junk.mp3");
    std::fs::write(&junk, b"this is not audio at all").expect("write");
    let err = index_music(&junk, "junk.mp3", &MusicIndexOptions::default()).expect_err("non-audio");
    assert!(err.to_string().contains("junk.mp3"), "{err}");

    let missing = s.path("missing.wav");
    assert!(index_music(&missing, "missing.wav", &MusicIndexOptions::default()).is_err());
}

fn run_cli(root: &Path, args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO"))
        .current_dir(root)
        .args([
            "run",
            "--offline",
            "-q",
            "-p",
            "motion-cli",
            "--",
            "music-index",
        ])
        .args(args)
        .output()
        .expect("run motion-engine")
}

#[test]
fn cli_default_output_beside_track() {
    if !ffmpeg_ok() {
        return;
    }
    let s = Scratch::new("cli");
    std::fs::create_dir_all(s.path("music")).expect("mkdir");
    let wav = s.path("music/bed.wav");
    assert!(click_track(120.0, 0.2, 10.0, &wav));
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = run_cli(&root, &[wav.as_os_str()]);
    assert!(
        out.status.success(),
        "cli failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("bpm 120"), "{stdout}");
    let plan_path = s.path("music/bed.music.json");
    let text = std::fs::read_to_string(&plan_path).expect("plan written beside track");
    assert!(text.ends_with("}\n"));
    let v: serde_json::Value = serde_json::from_str(&text).expect("json");
    assert_eq!(v["track"], "bed.wav");

    // Explicit output elsewhere: track path is relative to that directory.
    std::fs::create_dir_all(s.path("plans")).expect("mkdir");
    let out2 = s.path("plans/x.json");
    let out = run_cli(
        &root,
        &[wav.as_os_str(), "--output".as_ref(), out2.as_os_str()],
    );
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out2).expect("read")).expect("json");
    assert_eq!(v["track"], "../music/bed.wav");
}
