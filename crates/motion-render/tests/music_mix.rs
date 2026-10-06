//! Music bed mix (0.8 M2): ducked bed under SFX cues, duration, alignment.
//! Skipped when ffmpeg/ffprobe are missing. See docs/SOUND_DESIGN.md §4, §7.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, MusicBed, MusicPlan, SfxFamily, SfxLibrary, SfxSound,
    AUDIO_PLAN_VERSION, MUSIC_PLAN_VERSION, SFX_LIBRARY_VERSION,
};
use motion_core::MotionProject;
use motion_render::audio_mix::mix_audio;
use motion_render::audio_qa::{audio_report, audio_report_with_music};

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("-version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn tools() -> bool {
    have("ffmpeg") && have("ffprobe")
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

fn probe(file: &Path, stream: &str, entries: &str) -> String {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", stream, "-show_entries"])
        .arg(format!("stream={entries}"))
        .args(["-of", "csv=p=0"])
        .arg(file)
        .output()
        .expect("ffprobe runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// md5 of the decoded audio stream.
fn audio_md5(file: &Path) -> String {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(file)
        .args(["-map", "0:a", "-f", "md5", "-"])
        .output()
        .expect("ffmpeg runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Mean volume (dB) of `[from, to]` of the audio stream.
fn mean_db(file: &Path, from: f64, to: f64) -> f64 {
    let out = Command::new("ffmpeg")
        .args(["-v", "info", "-i"])
        .arg(file)
        .args(["-map", "0:a", "-af"])
        .arg(format!("atrim={from}:{to},volumedetect"))
        .args(["-f", "null", "-"])
        .output()
        .expect("ffmpeg runs");
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines()
        .find_map(|l| l.split("mean_volume:").nth(1))
        .and_then(|v| v.trim().trim_end_matches(" dB").parse().ok())
        .unwrap_or(f64::NEG_INFINITY)
}

/// A 30 ms 1 kHz burst, peak at lead + 15 ms, padded to 1.2 s (mono 48 kHz).
fn make_sound(path: &Path, lead_ms: u32) {
    let chain = format!(
        "afade=t=in:d=0.015,afade=t=out:st=0.015:d=0.015,adelay={lead_ms}:all=1,apad=whole_dur=1.2"
    );
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:duration=0.03:sample_rate=48000",
        "-af",
        &chain,
        "-ac",
        "1",
        "-t",
        "1.2",
        path.to_str().expect("utf8"),
    ]);
}

/// Quiet 120 BPM click bed (50 ms, 220 Hz burst every 0.5 s), mono 48 kHz.
fn make_bed(path: &Path, seconds: f64) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("aevalsrc=0.5*sin(2*PI*220*t)*lt(mod(t\\,0.5)\\,0.05):s=48000:d={seconds}"),
        "-ac",
        "1",
        path.to_str().expect("utf8"),
    ]);
}

fn sound(id: &str, lead: f64) -> SfxSound {
    SfxSound {
        id: id.to_string(),
        family: SfxFamily::Click,
        path: format!("sounds/{id}.wav"),
        duration: 1.2,
        onset: lead + 0.005,
        peak: lead + 0.015,
        audible_end: lead + 0.03,
        peak_db: -18.0,
        lufs: None,
        sha256: "0".repeat(64),
        tags: vec![],
    }
}

fn cue(time: f64, scene: &str, id: &str, kind: CueKind) -> AudioCue {
    AudioCue {
        time,
        scene: scene.into(),
        kind,
        family: SfxFamily::Click,
        sound_id: id.into(),
        gain_db: 6.0,
        priority: kind.priority(),
        reason: "test".into(),
    }
}

fn project() -> MotionProject {
    MotionProject::from_json(
        r##"{"version":"0.2","project":{"name":"t","duration_seconds":6.0},
        "canvas":{"width":64,"height":64,"fps":30,"background":"#000000"},
        "scenes":[
          {"id":"s1","start_seconds":0.0,"duration_seconds":3.0,"layers":[],"motions":[],
           "lifecycle":{"enter":0.2,"settle":0.3,"read":0.5,"evolve":1.0,"anticipate":2.8,"bridge":3.0}},
          {"id":"s2","start_seconds":3.0,"duration_seconds":3.0,"layers":[],"motions":[],
           "lifecycle":{"enter":0.2,"settle":0.3,"read":0.5,"evolve":1.0,"anticipate":2.8,"bridge":3.0}}
        ]}"##,
    )
    .expect("project")
}

fn music_plan(track: &str, seconds: f64) -> MusicPlan {
    MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: track.to_string(),
        duration: seconds,
        bpm: 120.0,
        beat_times: (0..(seconds * 2.0) as u32)
            .map(|k| f64::from(k) * 0.5)
            .collect(),
        downbeat_times: (0..(seconds / 2.0) as u32)
            .map(|k| f64::from(k) * 2.0)
            .collect(),
        sections: vec![],
        gain_db: -6.0,
        sha256: "0".repeat(64),
        lufs: None,
        lra: None,
    }
}

struct Fixture {
    dir: PathBuf,
    video: PathBuf,
    library: SfxLibrary,
}

fn fixture(name: &str, bed_seconds: f64) -> Fixture {
    let dir = scratch(name);
    let video = dir.join("silent.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=64x64:r=30",
        "-t",
        "6",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        video.to_str().expect("utf8"),
    ]);
    std::fs::create_dir_all(dir.join("sounds")).expect("sounds dir");
    make_sound(&dir.join("sounds/a.wav"), 500);
    make_sound(&dir.join("sounds/b.wav"), 800);
    std::fs::create_dir_all(dir.join("music")).expect("music dir");
    make_bed(&dir.join("music/bed.wav"), bed_seconds);
    let plan = music_plan("bed.wav", bed_seconds);
    std::fs::write(
        dir.join("music/bed.music.json"),
        serde_json::to_string_pretty(&plan).expect("json"),
    )
    .expect("music plan");
    let library = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![sound("a", 0.5), sound("b", 0.8)],
    };
    library.validate().expect("library valid");
    Fixture {
        dir,
        video,
        library,
    }
}

fn plan(music: Option<MusicBed>) -> AudioPlan {
    AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        // Off the 0.5 s click grid so a bed click never sits on a cue peak.
        cues: vec![
            cue(1.3, "s1", "a", CueKind::Information),
            cue(4.3, "s2", "b", CueKind::Information),
        ],
        loudness_target: -16.0,
        true_peak_limit: -1.0,
        min_spacing: 0.15,
        information_cap: 3,
        music,
        speech_adjustments: vec![],
        levels: None,
    }
}

fn bed(duck: bool) -> MusicBed {
    MusicBed {
        track: "bed.wav".into(),
        gain_db: -6.0,
        fade_in: 0.5,
        fade_out: 1.0,
        duck,
        start: 0.0,
        fade_out_at: None,
    }
}

fn mix(fx: &Fixture, p: &AudioPlan, music_root: Option<&Path>, name: &str) -> PathBuf {
    let out = fx.dir.join(name);
    mix_audio(&fx.video, p, &fx.dir, &fx.library, music_root, 6.0, &out).expect("mix");
    out
}

#[test]
fn music_bed_mix_keeps_duration_and_cue_alignment() {
    if !tools() {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
        return;
    }
    let fx = fixture("music_mix_full", 6.0);
    let p = plan(Some(bed(true)));
    let music_root = fx.dir.join("music");
    let out = mix(&fx, &p, Some(&music_root), "mixed.mp4");

    assert_eq!(probe(&out, "a:0", "codec_name"), "aac");
    let dur: f64 = probe(&out, "a:0", "duration").parse().expect("duration");
    assert!((dur - 6.0).abs() < 0.06, "audio duration {dur}");

    // The bed is audible between cues (beat clicks at 2.0, 2.5, 3.0 s).
    let with_bed = mean_db(&out, 2.0, 3.2);
    assert!(with_bed > -70.0, "bed inaudible: {with_bed} dB");
    // ...and faded out at the end of the project.
    let tail = mean_db(&out, 5.9, 6.0);
    assert!(tail < with_bed - 6.0, "tail {tail} vs body {with_bed}");

    let report = audio_report(&project(), &p, &fx.library, Some(&out)).expect("report");
    eprintln!("{}", report.to_text());
    assert_eq!(report.alignment.len(), 2, "both cues isolated");
    for a in &report.alignment {
        assert!(
            a.error <= 1.0 / 30.0,
            "alignment error {} at {}",
            a.error,
            a.planned
        );
    }
    assert_eq!(report.verdict, "PASS", "{}", report.to_text());
}

#[test]
fn no_music_matches_the_pre_music_mix() {
    if !tools() {
        return;
    }
    let fx = fixture("music_mix_none", 6.0);
    let without = mix(&fx, &plan(None), None, "none.mp4");
    // A plan with a bed but no music root mixes exactly like no bed.
    let no_root = mix(&fx, &plan(Some(bed(true))), None, "no_root.mp4");
    assert_eq!(audio_md5(&without), audio_md5(&no_root));
    // With the root the audio differs.
    let music_root = fx.dir.join("music");
    let with = mix(&fx, &plan(Some(bed(true))), Some(&music_root), "with.mp4");
    assert_ne!(audio_md5(&without), audio_md5(&with));
    // Without a bed the same span is silent.
    assert!(mean_db(&without, 2.0, 3.2) < -80.0);
}

#[test]
fn shorter_music_is_not_looped_and_longer_is_trimmed() {
    if !tools() {
        return;
    }
    // 2 s bed under a 6 s project: silence after it, duration unchanged.
    let fx = fixture("music_mix_short", 2.0);
    let music_root = fx.dir.join("music");
    let out = mix(&fx, &plan(Some(bed(true))), Some(&music_root), "short.mp4");
    let dur: f64 = probe(&out, "a:0", "duration").parse().expect("duration");
    assert!((dur - 6.0).abs() < 0.06, "audio duration {dur}");
    assert!(mean_db(&out, 2.2, 3.2) < -60.0, "music looped");

    // 12 s bed: trimmed to the project.
    let fx = fixture("music_mix_long", 12.0);
    let music_root = fx.dir.join("music");
    let out = mix(&fx, &plan(Some(bed(true))), Some(&music_root), "long.mp4");
    let dur: f64 = probe(&out, "a:0", "duration").parse().expect("duration");
    assert!((dur - 6.0).abs() < 0.06, "audio duration {dur}");
}

#[test]
fn missing_music_file_is_an_error() {
    if !tools() {
        return;
    }
    let fx = fixture("music_mix_missing", 6.0);
    let mut b = bed(true);
    b.track = "nope.wav".into();
    let out = fx.dir.join("x.mp4");
    let music_root = fx.dir.join("music");
    let r = mix_audio(
        &fx.video,
        &plan(Some(b)),
        &fx.dir,
        &fx.library,
        Some(&music_root),
        6.0,
        &out,
    );
    assert!(r.is_err());
}

#[test]
fn qa_reports_handoff_to_downbeat_distance() {
    // No ffmpeg needed. s1 ends at 3.0, s2 starts at 3.0: anchor 3.0; downbeats
    // every 2 s -> nearest 2.0 or 4.0, 1.0 s away.
    let fx_plan = plan(Some(bed(true)));
    let lib = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![sound("a", 0.5), sound("b", 0.8)],
    };
    let m = music_plan("bed.wav", 8.0);
    let r = audio_report_with_music(&project(), &fx_plan, &lib, None, Some(&m.downbeat_times))
        .expect("report");
    let h = r.handoff_to_downbeat.as_ref().expect("music section");
    assert_eq!(h.handoffs, 1);
    assert!((h.max_error - 1.0).abs() < 1e-9);
    assert_eq!(h.within_tolerance, 0);
    assert!(
        r.to_text()
            .contains("music: handoff→downbeat max 1000 ms (1 handoffs, 0 within tolerance)"),
        "{}",
        r.to_text()
    );
    // Not a FAIL condition; without music nothing is reported.
    assert_eq!(r.verdict, "PASS");
    let none = audio_report(&project(), &plan(None), &lib, None).expect("report");
    assert!(none.handoff_to_downbeat.is_none());
}

#[test]
fn ducking_lowers_the_bed_after_a_sustained_cue() {
    if !tools() {
        return;
    }
    let mut fx = fixture("music_mix_duck", 6.0);
    // A 0.4 s tone (the 30 ms clicks are too short for the compressor to react).
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=1000:duration=0.4:sample_rate=48000",
        "-af",
        "apad=whole_dur=1.2",
        "-ac",
        "1",
        "-t",
        "1.2",
        fx.dir.join("sounds/long.wav").to_str().expect("utf8"),
    ]);
    let mut long = sound("long", 0.0);
    long.onset = 0.0;
    long.peak = 0.2;
    long.audible_end = 0.4;
    fx.library.sounds.push(long);
    let mut p = plan(Some(bed(true)));
    p.cues = vec![cue(1.3, "s1", "long", CueKind::Impact)];
    let music_root = fx.dir.join("music");
    let ducked = mix(&fx, &p, Some(&music_root), "ducked.mp4");
    p.music = Some(bed(false));
    let plain = mix(&fx, &p, Some(&music_root), "plain.mp4");
    // The bed click at 1.5 s sits right after the tone ends (inside the release).
    let (d, q) = (mean_db(&ducked, 1.5, 1.55), mean_db(&plain, 1.5, 1.55));
    eprintln!("ducked {d} dB vs plain {q} dB");
    assert!(d < q - 1.0, "ducked {d} dB vs plain {q} dB");
    // Far from any cue the bed is untouched.
    let (d, q) = (mean_db(&ducked, 3.0, 3.05), mean_db(&plain, 3.0, 3.05));
    assert!((d - q).abs() < 0.5, "ducked {d} dB vs plain {q} dB");
}
