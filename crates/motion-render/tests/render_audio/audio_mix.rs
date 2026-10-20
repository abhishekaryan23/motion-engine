//! ffmpeg mix + audio QA on a synthetic video and synthetic sounds. Skipped
//! when ffmpeg/ffprobe are missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, SfxFamily, SfxLibrary, SfxSound, AUDIO_PLAN_VERSION,
    SFX_LIBRARY_VERSION,
};
use motion_core::MotionProject;
use motion_render::audio_mix::mix_audio;
use motion_render::audio_qa::audio_report;

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

/// `lead_ms` of silence, then a 30 ms 1 kHz burst with a triangular envelope
/// (peak at lead + 15 ms), padded to 1.2 s. Mono 48 kHz wav.
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

fn cue(time: f64, id: &str, kind: CueKind) -> AudioCue {
    AudioCue {
        time,
        scene: "s".into(),
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
        r##"{"version":"0.2","project":{"name":"t","duration_seconds":3.0},
        "canvas":{"width":64,"height":64,"fps":30,"background":"#000000"},
        "scenes":[{"id":"s","start_seconds":0.0,"duration_seconds":3.0,"layers":[],"motions":[],
          "lifecycle":{"enter":0.2,"settle":0.3,"read":0.5,"evolve":1.0,"anticipate":2.8,"bridge":3.0}}]}"##,
    )
    .expect("project")
}

struct Fixture {
    dir: PathBuf,
    video: PathBuf,
    library: SfxLibrary,
}

fn fixture(name: &str) -> Fixture {
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
    make_sound(&dir.join("sounds/a.wav"), 500);
    make_sound(&dir.join("sounds/b.wav"), 800);
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

#[test]
fn mix_places_peaks_on_the_cue_times() {
    if !(have("ffmpeg") && have("ffprobe")) {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
        return;
    }
    let fx = fixture("audio_mix_peaks");
    // 0.3 < peak (trim path), the others use the delay path.
    let p = plan(vec![
        cue(0.3, "a", CueKind::Open),
        cue(1.0, "b", CueKind::Information),
        cue(1.7, "a", CueKind::Information),
        cue(2.4, "b", CueKind::Information),
    ]);
    let out = fx.dir.join("mixed.mp4");
    mix_audio(&fx.video, &p, &fx.dir, &fx.library, None, 3.0, &out).expect("mix");

    assert_eq!(probe(&out, "a:0", "codec_name"), "aac");
    let dur: f64 = probe(&out, "a:0", "duration").parse().expect("duration");
    assert!((dur - 3.0).abs() < 0.06, "audio duration {dur}");
    assert_eq!(
        probe(&fx.video, "v:0", "codec_name,width,height,nb_frames"),
        probe(&out, "v:0", "codec_name,width,height,nb_frames"),
        "video stream is copied"
    );
    assert_eq!(probe(&out, "a:0", "sample_rate,channels"), "48000,2");

    let report = audio_report(&project(), &p, &fx.library, Some(&out)).expect("report");
    eprintln!("{}", report.to_text());
    assert_eq!(report.alignment.len(), 4, "all cues isolated");
    for a in &report.alignment {
        eprintln!(
            "cue {:.3}: measured {:.3} error {:.1} ms",
            a.planned,
            a.measured,
            a.error * 1000.0
        );
        assert!(
            a.error <= 1.0 / 30.0,
            "alignment error {} at {}",
            a.error,
            a.planned
        );
    }
    assert_eq!(report.verdict, "PASS", "{}", report.to_text());
    assert!(report.to_text().starts_with("\naudio\n"));
    assert!(report.to_text().ends_with('\n'));
    assert!(report.sample_peak_db.is_some());
}

#[test]
fn zero_cues_gives_a_silent_track_of_the_right_length() {
    if !(have("ffmpeg") && have("ffprobe")) {
        return;
    }
    let fx = fixture("audio_mix_silent");
    let out = fx.dir.join("mixed.mp4");
    mix_audio(
        &fx.video,
        &plan(vec![]),
        &fx.dir,
        &fx.library,
        None,
        3.0,
        &out,
    )
    .expect("mix");
    assert_eq!(probe(&out, "a:0", "codec_name"), "aac");
    let dur: f64 = probe(&out, "a:0", "duration").parse().expect("duration");
    assert!((dur - 3.0).abs() < 0.06, "audio duration {dur}");
}

#[test]
fn missing_sound_is_an_error_not_a_panic() {
    if !(have("ffmpeg") && have("ffprobe")) {
        return;
    }
    let fx = fixture("audio_mix_missing");
    let out = fx.dir.join("mixed.mp4");
    let p = plan(vec![cue(1.0, "nope", CueKind::Open)]);
    assert!(mix_audio(&fx.video, &p, &fx.dir, &fx.library, None, 3.0, &out).is_err());
}

#[test]
fn information_cue_after_anticipate_fails() {
    // Placement checks need no ffmpeg.
    let lib = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![sound("a", 0.5)],
    };
    let verdict = |p: &AudioPlan| audio_report(&project(), p, &lib, None).expect("report");
    let good = plan(vec![cue(1.5, "a", CueKind::Information)]);
    assert_eq!(verdict(&good).verdict, "PASS");
    let bad = plan(vec![
        cue(1.5, "a", CueKind::Information),
        cue(2.9, "a", CueKind::Information),
    ]);
    let r = verdict(&bad);
    assert_eq!(r.verdict, "FAIL");
    assert!(r.to_text().contains("ANTICIPATE"), "{}", r.to_text());
    // A handoff is allowed there.
    let handoff = plan(vec![cue(2.9, "a", CueKind::Handoff)]);
    assert_eq!(verdict(&handoff).verdict, "PASS");
}
