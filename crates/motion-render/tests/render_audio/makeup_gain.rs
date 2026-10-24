//! (0.9) Opt-in makeup gain on the final mix. Synthetic video and sounds;
//! skipped when ffmpeg is missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, SfxFamily, SfxLibrary, SfxSound, AUDIO_PLAN_VERSION,
    SFX_LIBRARY_VERSION,
};
use motion_render::audio_mix::{mix_audio, mix_audio_with, MakeupGain};

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn ffmpeg(args: &[&str]) {
    let out = Command::new("ffmpeg")
        .args(["-y", "-v", "error"])
        .args(args)
        .output()
        .expect("ffmpeg runs");
    assert!(
        out.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// (integrated LUFS, true peak dBTP) of the first audio stream.
fn measure(file: &Path) -> (f64, f64) {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-hide_banner", "-i"])
        .arg(file)
        .args([
            "-map",
            "0:a:0",
            "-af",
            "ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ffmpeg runs");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let summary = &err[err.rfind("Summary:").expect("summary")..];
    let value = |label: &str, unit: &str| -> f64 {
        let line = summary
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(label) && l.ends_with(unit))
            .unwrap_or_else(|| panic!("no {label} in {summary}"));
        line[label.len()..line.len() - unit.len()]
            .trim()
            .parse()
            .expect("number")
    };
    (value("I:", "LUFS"), value("Peak:", "dBFS"))
}

/// A one-second 440 Hz tone, `boost_db` above ffmpeg's default sine level.
fn make_tone(path: &Path, boost_db: i32) {
    let chain = format!("volume={boost_db}dB,afade=t=in:d=0.05,afade=t=out:st=0.95:d=0.05");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=1:sample_rate=48000",
        "-af",
        &chain,
        "-ac",
        "1",
        path.to_str().expect("utf8"),
    ]);
}

fn tone(id: &str) -> SfxSound {
    SfxSound {
        id: id.to_string(),
        family: SfxFamily::Click,
        path: format!("sounds/{id}.wav"),
        duration: 1.0,
        onset: 0.0,
        peak: 0.5,
        audible_end: 1.0,
        peak_db: -18.0,
        lufs: None,
        sha256: "0".repeat(64),
        tags: vec![],
    }
}

fn cue(time: f64, id: &str, gain_db: f64) -> AudioCue {
    AudioCue {
        time,
        scene: "s".into(),
        kind: CueKind::Open,
        family: SfxFamily::Click,
        sound_id: id.into(),
        gain_db,
        priority: CueKind::Open.priority(),
        reason: "test".into(),
    }
}

fn plan(cues: Vec<AudioCue>) -> AudioPlan {
    AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues,
        loudness_target: -16.0,
        true_peak_limit: -1.0,
        min_spacing: 0.15,
        information_cap: 3,
        music: None,
        speech_adjustments: vec![],
        levels: None,
    }
}

struct Fx {
    dir: PathBuf,
    video: PathBuf,
    library: SfxLibrary,
}

fn fixture(name: &str) -> Fx {
    let dir = scratch(name);
    let video = dir.join("silent.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=64x64:r=30",
        "-t",
        "3",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        video.to_str().expect("utf8"),
    ]);
    std::fs::create_dir_all(dir.join("sounds")).expect("sounds dir");
    make_tone(&dir.join("sounds/quiet.wav"), 0);
    make_tone(&dir.join("sounds/loud.wav"), 17);
    let library = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![tone("quiet"), tone("loud")],
    };
    library.validate().expect("library valid");
    Fx {
        dir,
        video,
        library,
    }
}

fn run(fx: &Fx, p: &AudioPlan, makeup: Option<MakeupGain>, out: &Path) {
    match makeup {
        None => mix_audio(&fx.video, p, &fx.dir, &fx.library, None, 3.0, out),
        Some(m) => mix_audio_with(&fx.video, p, &fx.dir, &fx.library, None, 3.0, out, m, None),
    }
    .expect("mix");
}

#[test]
fn off_is_the_plain_mix() {
    if !(have("ffmpeg") && have("ffprobe")) {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
        return;
    }
    let fx = fixture("makeup_off");
    let p = plan(vec![cue(1.0, "quiet", -12.0)]);
    let a = fx.dir.join("plain.mp4");
    let b = fx.dir.join("off.mp4");
    run(&fx, &p, None, &a);
    run(&fx, &p, Some(MakeupGain::Off), &b);
    assert_eq!(
        std::fs::read(&a).expect("plain"),
        std::fs::read(&b).expect("off"),
        "Off must equal mix_audio byte for byte"
    );
    assert_eq!(MakeupGain::default(), MakeupGain::Off);
}

#[test]
fn auto_raises_a_quiet_sfx_only_mix_and_keeps_headroom() {
    if !(have("ffmpeg") && have("ffprobe")) {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
        return;
    }
    let fx = fixture("makeup_quiet");
    let p = plan(vec![cue(0.8, "quiet", -12.0), cue(1.8, "quiet", -12.0)]);
    let off = fx.dir.join("off.mp4");
    let auto = fx.dir.join("auto.mp4");
    run(&fx, &p, Some(MakeupGain::Off), &off);
    run(&fx, &p, Some(MakeupGain::Auto), &auto);
    let (i_off, tp_off) = measure(&off);
    let (i_auto, tp_auto) = measure(&auto);
    eprintln!("quiet: off I {i_off:.1} LUFS / {tp_off:.1} dBTP -> auto I {i_auto:.1} LUFS / {tp_auto:.1} dBTP");
    assert!(
        i_auto - i_off > 6.0,
        "loudness {i_off:.1} -> {i_auto:.1} LUFS"
    );
    assert!(tp_auto <= -0.9, "true peak {tp_auto:.2} dBTP");
    // No temp files left behind.
    let leftovers: Vec<_> = std::fs::read_dir(&fx.dir)
        .expect("dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("makeup1"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn auto_never_lowers_loudness() {
    if !(have("ffmpeg") && have("ffprobe")) {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
        return;
    }
    let fx = fixture("makeup_loud");
    // Already above the target (and near the limiter): Auto must not attenuate.
    let p = plan(vec![cue(0.8, "loud", 0.0), cue(1.8, "loud", 0.0)]);
    let off = fx.dir.join("off.mp4");
    let auto = fx.dir.join("auto.mp4");
    run(&fx, &p, Some(MakeupGain::Off), &off);
    run(&fx, &p, Some(MakeupGain::Auto), &auto);
    let (i_off, _) = measure(&off);
    let (i_auto, tp_auto) = measure(&auto);
    eprintln!("loud: off I {i_off:.1} LUFS -> auto I {i_auto:.1} LUFS ({tp_auto:.1} dBTP)");
    assert!(i_auto >= i_off - 0.1, "{i_off:.1} -> {i_auto:.1} LUFS");
    assert!(tp_auto <= -0.9, "true peak {tp_auto:.2} dBTP");
    // A silent plan stays silent-ish: no gain applied, still a valid file.
    let silent = fx.dir.join("silent_auto.mp4");
    run(&fx, &plan(vec![]), Some(MakeupGain::Auto), &silent);
    assert!(silent.is_file());
}
