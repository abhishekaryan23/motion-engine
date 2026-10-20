//! (0.23 W5a) Mix QA end to end on synthetic tones: `stem_qa` rebuilds the
//! stems of a mix in a temp directory and measures `voice_over_music`,
//! `bed_over_voice` and `speech_band_masking` (no temp directory survives, also
//! after an error); the long-gap cap holds a bed with loud peaks in a long
//! pause under the voice; the stems summed with the logged makeup gain are the
//! shipped mix; `speech_report` carries the three checks and `bed_duck` never
//! FAILs. Skipped when ffmpeg is missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, DuckEnvelope, MixLevels, MusicBed, SfxFamily, SfxLibrary,
    SfxSound, AUDIO_PLAN_VERSION, SFX_LIBRARY_VERSION,
};
use motion_core::checks::{BED_OVER_VOICE_MAX_DB, SPEECH_BAND_MASKING, VOICE_OVER_MUSIC};
use motion_core::scene::MotionProject;
use motion_core::speech::{SpeechMap, SpeechWord, SPEECH_VERSION};
use motion_render::audio_mix::{
    gap_cap, mix_audio_report, voiced_spans, write_stems, GapCap, MakeupGain, MixOptions,
    VoiceRelative, VoiceTrack,
};
use motion_render::mix_qa::{measure_stems, stem_qa, StemMeasure, StemQa};
use motion_render::speech_qa::{bed_duck_status, speech_report, CheckStatus, SpeechQaReport};

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

/// A scratch directory under the target dir, removed when dropped (also when
/// an assertion fails).
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        Scratch(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
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

/// Integrated loudness (LUFS) of `file`'s first audio stream, or of the
/// `filter_complex` graph over several inputs.
fn integrated_lufs(inputs: &[&Path], graph: &str) -> f64 {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostats", "-hide_banner"]);
    for i in inputs {
        cmd.arg("-i").arg(i);
    }
    cmd.args(["-filter_complex", graph, "-map", "[o]", "-f", "null", "-"]);
    let out = cmd.output().expect("ebur128");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let summary = err.rsplit("Summary:").next().expect("summary");
    summary
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("I:"))
        .and_then(|l| l.trim_start_matches("I:").split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no integrated loudness in {err}"))
}

fn file_lufs(file: &Path) -> f64 {
    integrated_lufs(&[file], "[0:a]ebur128=peak=true[o]")
}

/// 600 Hz "voice" speaking in `spans`, float wav, scaled to `lufs`.
fn make_voice(dir: &Path, name: &str, spans: &[(f64, f64)], seconds: f64, lufs: f64) -> PathBuf {
    let on: Vec<String> = spans
        .iter()
        .map(|(a, b)| format!("between(t,{a},{b})"))
        .collect();
    let base = dir.join(format!("{name}_base.wav"));
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!(
            "aevalsrc='0.1*sin(2*PI*600*t)*({})':d={seconds}:s=48000",
            on.join("+")
        ),
        "-ac",
        "1",
        "-c:a",
        "pcm_f32le",
        s(&base),
    ]);
    let gain = lufs - file_lufs(&base);
    let out = dir.join(name);
    ffmpeg(&[
        "-i",
        s(&base),
        "-af",
        &format!("volume={gain:.3}dB"),
        "-c:a",
        "pcm_f32le",
        s(&out),
    ]);
    let _ = std::fs::remove_file(&base);
    out
}

/// 200 Hz bed of `seconds`: amplitude `quiet`, and `loud` inside `burst`.
fn make_bed(path: &Path, seconds: f64, quiet: f64, burst: Option<(f64, f64, f64)>) {
    let amp = match burst {
        Some((a, b, loud)) => format!("({quiet}+({loud}-{quiet})*between(t,{a},{b}))"),
        None => format!("{quiet}"),
    };
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!("aevalsrc='{amp}*sin(2*PI*200*t)':d={seconds}:s=48000"),
        "-ac",
        "1",
        s(path),
    ]);
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

fn silent_video(dir: &Path, seconds: f64) -> PathBuf {
    let video = dir.join("silent.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=64x64:r=30",
        "-t",
        &seconds.to_string(),
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        s(&video),
    ]);
    video
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

fn plan(cues: Vec<AudioCue>, music: Option<MusicBed>, levels: Option<MixLevels>) -> AudioPlan {
    AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues,
        loudness_target: -16.0,
        true_peak_limit: -1.0,
        min_spacing: 0.15,
        information_cap: 3,
        music,
        speech_adjustments: vec![],
        levels,
    }
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

fn library() -> SfxLibrary {
    SfxLibrary {
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
    }
}

/// A SpeechMap with one word per span (`audio` = "voice.wav").
fn speech(spans: &[(f64, f64)], duration: f64) -> SpeechMap {
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "voice.wav".into(),
        sample_rate: 48_000,
        duration,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words: spans
            .iter()
            .enumerate()
            .map(|(i, &(start, end))| SpeechWord {
                text: format!("w{i}"),
                start,
                end,
                confidence: 1.0,
            })
            .collect(),
        sentences: vec![],
        recognised: vec![],
        alignment: None,
    }
}

/// The options a render passes for a voice-relative mix of `map`.
fn relative(map: &SpeechMap, levels: &MixLevels) -> VoiceRelative {
    VoiceRelative {
        envelope: Some(DuckEnvelope::from_speech(map, levels)),
        voiced: voiced_spans(map),
        ..VoiceRelative::default()
    }
}

fn voice_track(path: &Path) -> VoiceTrack {
    VoiceTrack {
        path: path.to_path_buf(),
        offset: 0.0,
        gain_db: 0.0,
    }
}

/// The two-sentence fixture: speech in 0.5..3.5 and 5.5..6.5 s (a 2 s gap).
const SPANS: [(f64, f64); 2] = [(0.5, 3.5), (5.5, 6.5)];
const DURATION: f64 = 8.0;

struct Fixture {
    dir: Scratch,
    map: SpeechMap,
    voice: PathBuf,
}

impl Fixture {
    fn new(name: &str, voice_lufs: f64) -> Fixture {
        let dir = Scratch::new(name);
        let voice = make_voice(dir.path(), "voice.wav", &SPANS, DURATION, voice_lufs);
        make_bed(&dir.path().join("bed.wav"), DURATION, 0.2, None);
        make_sound(&dir.path().join("a.wav"));
        Fixture {
            dir,
            map: speech(&SPANS, DURATION),
            voice,
        }
    }

    fn qa<'a>(&'a self, plan: &'a AudioPlan, base: &'a Path) -> StemQa<'a> {
        StemQa {
            plan,
            speech: &self.map,
            voice: &self.voice,
            offset: 0.0,
            music_root: self.dir.path(),
            duration: DURATION,
            temp_base: Some(base),
        }
    }
}

fn entries(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn stem_qa_measures_a_voice_relative_mix_and_removes_its_temp_dir() {
    if !tools() {
        return;
    }
    let fx = Fixture::new("mix_qa_basic", -26.0);
    let base = fx.dir.path().join("qa_tmp");
    let p = plan(vec![], Some(bed()), Some(MixLevels::STANDARD));
    let m = stem_qa(&fx.qa(&p, &base)).expect("stem qa");
    // The bed sits 18 dB under the voice while it speaks.
    let vom = m.voice_over_music.expect("voice over music");
    assert!((vom - 18.0).abs() < 1.0, "voice over music {vom:.2}");
    // And 10 dB under in the 2 s gap (a steady tone: its momentary is its level).
    let bov = m.bed_over_voice.expect("bed over voice");
    assert!((bov + 10.0).abs() < 1.0, "bed over voice {bov:.2}");
    assert_eq!((m.voiced_segments, m.gaps), (2, 1));
    // The bed is a 200 Hz tone, the voice a 600 Hz one: the 300 Hz high-pass of
    // the band takes 7.8 dB off the bed (2 poles, 1.5 octaves under 300 Hz...
    // |H|^2 = 1 / (1 + (300/200)^4)) and almost none off the voice.
    let band = m.speech_band_masking.expect("speech band");
    assert!((band + 18.0 + 7.8).abs() < 1.0, "speech band {band:.2}");
    // The normalised voice stem is the reference: -18 LUFS whatever the take was.
    let voice = m.voice_integrated_lufs.expect("voice");
    assert!((voice + 18.0).abs() < 0.6, "voice integrated {voice:.2}");
    // The temp directory is gone (the base directory is left empty).
    assert!(base.is_dir(), "the base dir was created");
    assert_eq!(entries(&base), 0, "stems left in {}", base.display());
}

#[test]
fn a_failed_rebuild_leaves_no_temp_dir() {
    if !tools() {
        return;
    }
    let fx = Fixture::new("mix_qa_error", -26.0);
    let base = fx.dir.path().join("qa_tmp");
    let p = plan(vec![], Some(bed()), Some(MixLevels::STANDARD));
    // A bed that is not audio: the mix fails after the temp dir exists.
    std::fs::write(fx.dir.path().join("bed.wav"), b"not a wav file").expect("garbage");
    let err = stem_qa(&fx.qa(&p, &base)).expect_err("a garbage bed cannot be mixed");
    eprintln!("forced error: {err}");
    assert!(base.is_dir(), "the temp dir was created before the error");
    assert_eq!(entries(&base), 0, "stems left in {}", base.display());
    // A missing voice file fails the same way.
    let mut q = fx.qa(&p, &base);
    let missing = fx.dir.path().join("missing.wav");
    q.voice = &missing;
    make_bed(&fx.dir.path().join("bed.wav"), DURATION, 0.2, None);
    stem_qa(&q).expect_err("no voice");
    assert_eq!(entries(&base), 0);
    // No bed at all is an error too (the caller skips the checks before this).
    let nobed = plan(vec![], None, Some(MixLevels::STANDARD));
    stem_qa(&fx.qa(&nobed, &base)).expect_err("no bed");
    assert_eq!(entries(&base), 0);
}

#[test]
fn the_legacy_graph_buries_a_quiet_voice_and_the_new_one_does_not() {
    if !tools() {
        return;
    }
    let fx = Fixture::new("mix_qa_legacy", -26.0);
    let base = fx.dir.path().join("qa_tmp");
    let legacy = plan(vec![], Some(bed()), None);
    let m = stem_qa(&fx.qa(&legacy, &base)).expect("legacy stem qa");
    let vom = m.voice_over_music.expect("legacy voice over music");
    assert!(vom < 10.0, "legacy voice over music {vom:.2}");
    let fresh = plan(vec![], Some(bed()), Some(MixLevels::STANDARD));
    let m = stem_qa(&fx.qa(&fresh, &base)).expect("stem qa");
    assert!(m.voice_over_music.expect("vom") >= 15.0);
    assert_eq!(entries(&base), 0);
}

/// Voice 600 Hz in 0.5..3.5 and 8.5..9.5 (a 5 s gap), 11 s; a bed whose
/// 200 Hz level jumps from 0.05 to 0.3 for 2 s in the middle of the gap.
const HOT_SPANS: [(f64, f64); 2] = [(0.5, 3.5), (8.5, 9.5)];
const HOT_DURATION: f64 = 11.0;

#[test]
fn the_gap_cap_holds_loud_bed_peaks_in_a_long_gap_under_the_voice() {
    if !tools() {
        return;
    }
    let dir = Scratch::new("mix_qa_cap");
    let voice = make_voice(dir.path(), "voice.wav", &HOT_SPANS, HOT_DURATION, -26.0);
    make_bed(
        &dir.path().join("bed.wav"),
        HOT_DURATION,
        0.05,
        Some((5.0, 7.0, 0.3)),
    );
    let video = silent_video(dir.path(), HOT_DURATION);
    let map = speech(&HOT_SPANS, HOT_DURATION);
    let levels = MixLevels::STANDARD;
    let rel = relative(&map, &levels);
    let vt = voice_track(&voice);
    let p = plan(vec![], Some(bed()), Some(levels));
    let opts = MixOptions {
        voice: Some(&vt),
        relative: Some(&rel),
        ..MixOptions::default()
    };
    let cap = gap_cap(
        &video,
        &p,
        dir.path(),
        &library(),
        Some(dir.path()),
        HOT_DURATION,
        &opts,
    )
    .expect("gap cap")
    .expect("a long gap was metered");
    let ceiling = BED_OVER_VOICE_MAX_DB - 0.5;
    eprintln!("gap cap: {cap:?}");
    assert_eq!(cap.ceiling_db, ceiling);
    // Uncapped, the burst is well past voice - 6 dB (the plateau at -10 plus
    // the burst's height over the bed's integrated level) ...
    assert!(cap.before_db > BED_OVER_VOICE_MAX_DB + 1.0, "{cap:?}");
    // ... the long-gap level was lowered, and the metered worst is now at the
    // ceiling (never above it by more than the metering tolerance).
    assert!(cap.lowered_db < -1.0, "{cap:?}");
    assert!(cap.after_db <= ceiling + 0.05, "{cap:?}");
    assert!(cap.after_db > ceiling - 1.0, "not over-cut: {cap:?}");

    // The stems of that mix, measured independently: the QA's own value
    // honours the limit, and the speech level is untouched (18 dB under).
    let stems = dir.path().join("stems");
    let files = write_stems(
        &video,
        &p,
        dir.path(),
        &library(),
        Some(dir.path()),
        HOT_DURATION,
        &stems,
        &opts,
    )
    .expect("stems");
    let m: StemMeasure = measure_stems(
        &files.voice.expect("voice"),
        &files.bed.expect("bed"),
        &voiced_spans(&map),
        0.0,
    )
    .expect("measure");
    let bov = m.bed_over_voice.expect("bed over voice");
    assert!(bov <= BED_OVER_VOICE_MAX_DB, "bed over voice {bov:.2}");
    assert!((bov - ceiling).abs() < 0.6, "bed over voice {bov:.2}");
    // The bed is quiet outside the burst, so under speech it is further down
    // than the 18 dB the burst-heavy integrated loudness places it at.
    let vom = m.voice_over_music.expect("voice over music");
    assert!(vom >= 18.0, "voice over music {vom:.2}");

    // A steady bed does not need the cap: it is metered and left alone.
    make_bed(&dir.path().join("bed.wav"), HOT_DURATION, 0.2, None);
    let cap = gap_cap(
        &video,
        &p,
        dir.path(),
        &library(),
        Some(dir.path()),
        HOT_DURATION,
        &opts,
    )
    .expect("gap cap")
    .expect("metered");
    assert_eq!(cap.lowered_db, 0.0, "{cap:?}");
    assert!(cap.before_db < ceiling, "{cap:?}");
    // No voiced spans given (the caller has none): no cap, no metering.
    let unvoiced = VoiceRelative {
        envelope: rel.envelope.clone(),
        ..VoiceRelative::default()
    };
    let none = MixOptions {
        relative: Some(&unvoiced),
        ..opts
    };
    let cap: Option<GapCap> = gap_cap(
        &video,
        &p,
        dir.path(),
        &library(),
        Some(dir.path()),
        HOT_DURATION,
        &none,
    )
    .expect("gap cap");
    assert_eq!(cap, None);
}

#[test]
fn the_stems_summed_with_the_logged_makeup_gain_are_the_shipped_mix() {
    if !tools() {
        return;
    }
    let fx = Fixture::new("mix_qa_sum", -26.0);
    let dir = fx.dir.path();
    let video = silent_video(dir, DURATION);
    let levels = MixLevels::STANDARD;
    let rel = relative(&fx.map, &levels);
    let vt = voice_track(&dir.join("voice.wav"));
    let p = plan(vec![cue(2.0), cue(4.5)], Some(bed()), Some(levels));
    let out = dir.join("mixed.mp4");
    let stems = dir.join("stems");
    let report = mix_audio_report(
        &video,
        &p,
        dir,
        &library(),
        Some(dir),
        DURATION,
        &out,
        &MixOptions {
            makeup: MakeupGain::Auto,
            voice: Some(&vt),
            relative: Some(&rel),
            stems: Some(&stems),
        },
    )
    .expect("mix");
    // A tone voice at -18 LUFS needs about +2 dB to reach the -16 target.
    assert!(
        report.makeup_db > 0.5 && report.makeup_db < 6.0,
        "{report:?}"
    );
    assert!(report.sfx_trim_db <= 0.0, "{report:?}");
    assert_eq!(report.gap_cap.map(|c| c.lowered_db), Some(0.0));
    let final_lufs = file_lufs(&out);
    let (v, b, x) = (
        stems.join("voice.wav"),
        stems.join("bed.wav"),
        stems.join("sfx.wav"),
    );
    let sum = integrated_lufs(
        &[&v, &b, &x],
        &format!(
            "[0:a][1:a][2:a]amix=inputs=3:normalize=0:duration=longest,volume={:.2}dB,ebur128=peak=true[o]",
            report.makeup_db
        ),
    );
    assert!(
        (sum - final_lufs).abs() <= 0.5,
        "stems {sum:.2} LUFS, shipped mix {final_lufs:.2} LUFS (makeup {:.1} dB)",
        report.makeup_db
    );
    // The stems QA rebuilds on its own (no SFX bus, a temp dir) are the voice
    // and bed stems of this mix: the three measures agree to a few hundredths.
    let spans = voiced_spans(&fx.map);
    let shipped = measure_stems(&v, &b, &spans, 0.0).expect("shipped stems");
    let base = dir.join("qa_tmp");
    let rebuilt = stem_qa(&fx.qa(&p, &base)).expect("stem qa");
    for (name, a, b) in [
        (
            "voice_over_music",
            shipped.voice_over_music,
            rebuilt.voice_over_music,
        ),
        (
            "bed_over_voice",
            shipped.bed_over_voice,
            rebuilt.bed_over_voice,
        ),
        (
            "speech_band_masking",
            shipped.speech_band_masking,
            rebuilt.speech_band_masking,
        ),
    ] {
        let (a, b) = (a.expect(name), b.expect(name));
        assert!(
            (a - b).abs() < 0.05,
            "{name}: shipped {a:.3}, rebuilt {b:.3}"
        );
    }
    assert_eq!(entries(&base), 0);
    // Without makeup the mix reports none.
    let off = mix_audio_report(
        &video,
        &p,
        dir,
        &library(),
        Some(dir),
        DURATION,
        &dir.join("off.mp4"),
        &MixOptions {
            voice: Some(&vt),
            relative: Some(&rel),
            ..MixOptions::default()
        },
    )
    .expect("mix");
    assert_eq!(off.makeup_db, 0.0);
}

/// A one-scene project of `DURATION` seconds (no text: layout QA has nothing to judge).
fn project() -> MotionProject {
    let doc = serde_json::json!({
        "version": "0.2",
        "project": { "name": "mix_qa" },
        "canvas": { "width": 360, "height": 640, "fps": 30, "background": "#000000" },
        "theme": { "fonts": {} },
        "assets": [],
        "scenes": [{
            "id": "beat_1", "start_seconds": 0.0, "duration_seconds": DURATION,
            "layers": [{ "id": "b1.card", "type": "rectangle", "x": 100, "y": 200,
                         "width": 100, "height": 100, "fill": "#FFFFFF" }],
            "motions": []
        }],
    });
    MotionProject::from_json(&doc.to_string()).expect("project parses")
}

fn status(r: &SpeechQaReport, name: &str) -> CheckStatus {
    r.checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"))
        .status
}

#[test]
fn speech_report_carries_the_three_stem_checks() {
    if !tools() {
        return;
    }
    let fx = Fixture::new("mix_qa_report", -26.0);
    let dir = fx.dir.path();
    let project = project();
    let names = [VOICE_OVER_MUSIC, "bed_over_voice", SPEECH_BAND_MASKING];
    let report = |plan: Option<&AudioPlan>| -> SpeechQaReport {
        speech_report(&project, &fx.map, dir, plan, None, Some(dir), None).expect("speech qa")
    };

    // No plan, or a plan without a bed: skipped, with the reason.
    for (plan, why) in [
        (None, "no audio plan"),
        (
            Some(plan(vec![], None, Some(MixLevels::STANDARD))),
            "no music bed",
        ),
    ] {
        let r = report(plan.as_ref());
        for name in names {
            let c = r.checks.iter().find(|c| c.name == name).expect(name);
            assert_eq!(c.status, CheckStatus::Skip, "{name}");
            assert!(c.detail.contains(why), "{}", c.detail);
        }
        assert_eq!(r.voice_over_music_db, None);
    }

    // The voice-relative plan: PASS PASS PASS (the band check is a WARN
    // threshold: a bed at 18 dB under in the speech band passes).
    let fresh = plan(vec![], Some(bed()), Some(MixLevels::STANDARD));
    let r = report(Some(&fresh));
    assert_eq!(status(&r, VOICE_OVER_MUSIC), CheckStatus::Pass, "{r:?}");
    assert_eq!(status(&r, "bed_over_voice"), CheckStatus::Pass, "{r:?}");
    assert_ne!(status(&r, SPEECH_BAND_MASKING), CheckStatus::Fail);
    let vom = r.voice_over_music_db.expect("value");
    assert!((vom - 18.0).abs() < 1.0, "{vom}");
    assert!(r.bed_over_voice_db.expect("value") < -6.0);
    assert!(r.speech_band_masking_db.is_some());
    let text = r.to_text();
    assert!(text.contains("PASS voice_over_music"), "{text}");
    assert!(text.contains("voice over music 18."), "{text}");
    let json = serde_json::to_value(&r).expect("json");
    for key in [
        "voice_over_music_db",
        "bed_over_voice_db",
        "speech_band_masking_db",
    ] {
        assert!(json[key].is_number(), "{key}");
    }

    // The same scene planned without `levels` (the pre-0.23 mix): the -26 LUFS
    // voice is buried, QA FAILs and the verdict with it.
    let legacy = plan(vec![], Some(bed()), None);
    let r = report(Some(&legacy));
    assert_eq!(status(&r, VOICE_OVER_MUSIC), CheckStatus::Fail, "{r:?}");
    assert_eq!(r.verdict, "FAIL");
    assert!(r.voice_over_music_db.expect("value") < 10.0);
    assert!(r
        .checks
        .iter()
        .any(|c| c.name == VOICE_OVER_MUSIC && c.detail.contains("legacy mix")));
    // QA left nothing behind in the system temp directory.
    let leftovers = std::fs::read_dir(std::env::temp_dir())
        .map(|d| {
            d.filter_map(Result::ok)
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with(&format!("motion-stem-qa-{}-", std::process::id()))
                })
                .count()
        })
        .unwrap_or(0);
    assert_eq!(leftovers, 0, "QA temp dirs left in the temp dir");
}

#[test]
fn bed_duck_is_informational_and_never_fails() {
    // PASS inside the legacy band (10 +- 3 dB), WARN outside: whatever the depth.
    for (depth, want) in [
        (10.0, CheckStatus::Pass),
        (7.0, CheckStatus::Pass),
        (13.0, CheckStatus::Pass),
        (6.9, CheckStatus::Warn),
        (13.1, CheckStatus::Warn),
        (0.0, CheckStatus::Warn),
        (40.0, CheckStatus::Warn),
        (-5.0, CheckStatus::Warn),
    ] {
        assert_eq!(bed_duck_status(depth), want, "{depth}");
    }
}

/// (0.23 W5a) A plan that names a bed whose file is missing is never a silent
/// skip: the three stem checks and `bed_duck` WARN, each with the resolved path
/// of the bed (the plan's directory joined with its track). A missing voice
/// file stays a SKIP that names the file. (No case here rebuilds stems:
/// `speech_report_carries_the_three_stem_checks` counts the stem temp
/// directories of this process and must not see another test's.)
#[test]
fn a_plan_naming_a_missing_bed_warns_with_the_path() {
    if !tools() {
        return;
    }
    let fx = Fixture::new("mix_qa_missing_bed", -16.0);
    let dir = fx.dir.path();
    let project = project();
    let names = [
        VOICE_OVER_MUSIC,
        "bed_over_voice",
        SPEECH_BAND_MASKING,
        "bed_duck",
    ];
    let mut gone = bed();
    gone.track = "nowhere/gone.wav".into();
    let want = dir.join("nowhere/gone.wav");
    let missing = plan(vec![], Some(gone), Some(MixLevels::STANDARD));

    // With a mixed file all four run (the voice file stands in for the mix).
    let r = speech_report(
        &project,
        &fx.map,
        dir,
        Some(&missing),
        Some(&fx.voice),
        Some(dir),
        None,
    )
    .expect("speech qa");
    for name in names {
        let c = r.checks.iter().find(|c| c.name == name).expect(name);
        assert_eq!(c.status, CheckStatus::Warn, "{name}: {}", c.detail);
        assert!(
            c.detail.contains(want.to_str().expect("utf8")),
            "{name}: {}",
            c.detail
        );
    }
    assert_eq!(r.voice_over_music_db, None);
    assert_eq!(r.bed_duck_db, None);

    // Without a mixed file the three stem checks still WARN (they rebuild the
    // stems from the plan); the verdict stays PASS: a WARN is not a FAIL.
    let r = speech_report(
        &project,
        &fx.map,
        dir,
        Some(&missing),
        None,
        Some(dir),
        None,
    )
    .expect("speech qa");
    for name in &names[..3] {
        assert_eq!(status(&r, name), CheckStatus::Warn, "{name}");
    }
    assert_eq!(status(&r, "bed_duck"), CheckStatus::Skip);
    assert_eq!(r.verdict, "PASS", "{r:?}");
    assert!(r.to_text().contains("WARN voice_over_music"));

    // A missing voice file (bed present) is a SKIP that names the file.
    let present = plan(vec![], Some(bed()), Some(MixLevels::STANDARD));
    let mut nomap = fx.map.clone();
    nomap.audio = "no_voice.wav".into();
    let r = speech_report(&project, &nomap, dir, Some(&present), None, Some(dir), None)
        .expect("speech qa");
    for name in &names[..3] {
        let c = r.checks.iter().find(|c| c.name == *name).expect(name);
        assert_eq!(c.status, CheckStatus::Skip, "{name}");
        assert!(c.detail.contains("no_voice.wav"), "{}", c.detail);
    }
}
