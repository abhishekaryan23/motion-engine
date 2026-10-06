//! (0.10) Voice-over mix: the VO keys the ducking (bed about -10 dB, SFX bus
//! about -4 dB), stays on top, and `MakeupGain::Auto` lands the integrated
//! loudness at -16 LUFS. Synthetic tones, measured through narrow band-pass
//! filters. Skipped when ffmpeg/ffprobe are missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, MusicBed, SfxFamily, SfxLibrary, SfxSound, AUDIO_PLAN_VERSION,
    SFX_LIBRARY_VERSION,
};
use motion_render::audio_mix::{mix_audio_with, MakeupGain, VoiceTrack};

const DURATION: f64 = 8.0;

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tools() -> bool {
    let ok = have("ffmpeg") && have("ffprobe");
    if !ok {
        eprintln!("skipping: ffmpeg/ffprobe not available");
    }
    ok
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

fn s(p: &Path) -> &str {
    p.to_str().expect("utf8")
}

/// 30 ms 1 kHz burst, peak at 15 ms, mono wav of 1.2 s.
fn make_sound(path: &Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:duration=0.03:sample_rate=48000",
        "-af",
        "afade=t=in:d=0.015,afade=t=out:st=0.015:d=0.015,apad=whole_dur=1.2",
        "-ac",
        "1",
        "-t",
        "1.2",
        s(path),
    ]);
}

/// 600 Hz "voice" (amplitude 0.1) speaking in [0.5, 3.5) and [5.5, 6.5).
fn make_voice(path: &Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "aevalsrc='0.1*sin(2*PI*600*t)*(between(t,0.5,3.5)+between(t,5.5,6.5))':d=8:s=48000",
        "-ac",
        "1",
        s(path),
    ]);
}

/// 200 Hz bed, amplitude 0.2.
fn make_bed(path: &Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.2*sin(2*PI*200*t):d=8:s=48000",
        "-ac",
        "1",
        s(path),
    ]);
}

fn cue(time: f64) -> AudioCue {
    AudioCue {
        time,
        scene: "s".into(),
        kind: CueKind::Information,
        family: SfxFamily::Click,
        sound_id: "a".into(),
        gain_db: 6.0,
        priority: 1,
        reason: "test".into(),
    }
}

fn plan(cues: Vec<AudioCue>, music: Option<MusicBed>) -> AudioPlan {
    AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues,
        loudness_target: -16.0,
        true_peak_limit: -1.0,
        min_spacing: 0.15,
        information_cap: 3,
        music,
        speech_adjustments: vec![],
        levels: None,
    }
}

struct Fx {
    dir: PathBuf,
    video: PathBuf,
    voice: VoiceTrack,
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
        "8",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        s(&video),
    ]);
    make_sound(&dir.join("a.wav"));
    make_bed(&dir.join("bed.wav"));
    make_voice(&dir.join("vo.wav"));
    let library = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![SfxSound {
            id: "a".into(),
            family: SfxFamily::Click,
            path: "a.wav".into(),
            duration: 1.2,
            onset: 0.005,
            peak: 0.015,
            audible_end: 0.03,
            peak_db: -18.0,
            lufs: None,
            sha256: "0".repeat(64),
            tags: vec![],
        }],
    };
    Fx {
        voice: VoiceTrack {
            path: dir.join("vo.wav"),
            offset: 0.0,
            gain_db: 0.0,
        },
        dir,
        video,
        library,
    }
}

fn bed() -> MusicBed {
    MusicBed {
        track: "bed.wav".into(),
        gain_db: -6.0,
        fade_in: 0.5,
        fade_out: 1.0,
        duck: true,
        start: 0.0,
        fade_out_at: None,
    }
}

fn mix(fx: &Fx, p: &AudioPlan, music: bool, makeup: MakeupGain, name: &str) -> PathBuf {
    let out = fx.dir.join(name);
    mix_audio_with(
        &fx.video,
        p,
        &fx.dir,
        &fx.library,
        music.then_some(fx.dir.as_path()),
        DURATION,
        &out,
        makeup,
        Some(&fx.voice),
    )
    .expect("mix");
    out
}

/// Mono 48 kHz samples of `file` after `filter`.
fn decode(file: &Path, filter: &str) -> Vec<f32> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-af", filter, "-ac", "1", "-ar", "48000"])
        .args(["-f", "f32le", "-"])
        .output()
        .expect("ffmpeg decode");
    assert!(out.status.success());
    out.stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn band(freq: u32) -> String {
    let one = format!("bandpass=f={freq}:width_type=q:w=6");
    format!("{one},{one}")
}

fn rms_db(x: &[f32], from: f64, to: f64) -> f64 {
    let (a, b) = ((from * 48_000.0) as usize, (to * 48_000.0) as usize);
    let seg = &x[a..b.min(x.len())];
    let ms: f64 = seg
        .iter()
        .map(|&v| f64::from(v) * f64::from(v))
        .sum::<f64>()
        / seg.len() as f64;
    10.0 * ms.max(1e-14).log10()
}

fn peak_db(x: &[f32], from: f64, to: f64) -> f64 {
    let (a, b) = ((from * 48_000.0) as usize, (to * 48_000.0) as usize);
    let p = x[a..b.min(x.len())]
        .iter()
        .fold(0.0f64, |m, &v| m.max(f64::from(v).abs()));
    20.0 * p.max(1e-9).log10()
}

fn integrated_lufs(file: &Path) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-v", "info", "-i"])
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
        .expect("ebur128");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let summary = err.rsplit("Summary:").next().expect("summary");
    let line = summary
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("I:"))
        .expect("I line");
    line.trim_start_matches("I:")
        .split_whitespace()
        .next()
        .expect("value")
        .parse()
        .expect("number")
}

#[test]
fn bed_ducks_about_ten_db_under_the_voice() {
    if !tools() {
        return;
    }
    let fx = fixture("voice_mix_bed");
    let out = mix(
        &fx,
        &plan(vec![], Some(bed())),
        true,
        MakeupGain::Off,
        "bed.mp4",
    );
    let x = decode(&out, &band(200));
    let under = rms_db(&x, 1.2, 3.2);
    let clear = rms_db(&x, 4.4, 5.3);
    let duck = clear - under;
    eprintln!("bed duck {duck:.2} dB (under {under:.1}, clear {clear:.1})");
    assert!((duck - 10.0).abs() <= 2.0, "bed duck {duck:.2} dB");
}

#[test]
fn sfx_bus_ducks_about_four_db_under_the_voice() {
    if !tools() {
        return;
    }
    let fx = fixture("voice_mix_sfx");
    let p = plan(vec![cue(2.0), cue(4.6)], None);
    let out = mix(&fx, &p, false, MakeupGain::Off, "sfx.mp4");
    let x = decode(&out, &band(1000));
    let under = peak_db(&x, 1.95, 2.1);
    let clear = peak_db(&x, 4.55, 4.7);
    let duck = clear - under;
    eprintln!("sfx duck {duck:.2} dB (under {under:.1}, clear {clear:.1})");
    assert!((duck - 4.0).abs() <= 1.5, "sfx duck {duck:.2} dB");
    // The voice itself is not ducked: its 600 Hz level equals the source's.
    let vo = decode(&out, &band(600));
    let src = decode(&fx.voice.path, &band(600));
    let (a, b) = (rms_db(&vo, 1.0, 3.0), rms_db(&src, 1.0, 3.0));
    assert!((a - b).abs() < 0.5, "voice level {a:.2} vs source {b:.2}");
}

#[test]
fn auto_makeup_lands_the_voice_mix_at_minus_sixteen_lufs() {
    if !tools() {
        return;
    }
    let fx = fixture("voice_mix_lufs");
    let p = plan(vec![cue(2.0), cue(4.6)], Some(bed()));
    let out = mix(&fx, &p, true, MakeupGain::Auto, "full.mp4");
    let lufs = integrated_lufs(&out);
    eprintln!("integrated {lufs:.2} LUFS");
    assert!((lufs + 16.0).abs() <= 1.0, "{lufs}");
    // VO-only works too (no cues, no music).
    let only = mix(
        &fx,
        &plan(vec![], None),
        false,
        MakeupGain::Auto,
        "only.mp4",
    );
    let lufs = integrated_lufs(&only);
    assert!((lufs + 16.0).abs() <= 1.0, "{lufs}");
    // Duration is exact.
    let dur = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "csv=p=0",
        ])
        .arg(&only)
        .output()
        .expect("ffprobe");
    let secs: f64 = String::from_utf8_lossy(&dur.stdout)
        .trim()
        .parse()
        .expect("duration");
    assert!((secs - DURATION).abs() < 0.1, "{secs}");
}
