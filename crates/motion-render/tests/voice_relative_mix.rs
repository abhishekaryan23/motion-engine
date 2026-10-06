//! (0.23 W5a) The voice-relative mix: the voice is normalised at mix time
//! (a -26 and a -14 LUFS take come out together), the bed follows the
//! speech-led envelope with no sidechain, the SFX bus stays under the voice,
//! and a plan without `levels` is the exact pre-0.23 graph. Synthetic tones;
//! skipped when ffmpeg is missing.

use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, DuckEnvelope, MixLevels, MusicBed, SfxFamily, SfxLibrary,
    SfxSound, AUDIO_PLAN_VERSION, SFX_LIBRARY_VERSION, VOICE_LUFS, VOICE_TRUE_PEAK_DB,
};
use motion_render::audio_mix::{
    mix_audio_ex, mix_graph, voice_gain_db, voice_limited_gain_db, voice_limiter_ceiling_db,
    write_stems, MakeupGain, MixOptions, VoiceRelative, VoiceTrack, VOICE_LIMIT_MAX_DB,
};

const DURATION: f64 = 8.0;
/// Where the synthetic voice speaks (s).
const SPEECH: [(f64, f64); 2] = [(0.5, 3.5), (5.5, 6.5)];

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

/// ffmpeg ebur128 log of `file` (info level).
fn ebur128_log(file: &Path) -> String {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-v", "info", "-i"])
        .arg(file)
        .args(["-map", "0:a:0", "-af", "ebur128", "-f", "null", "-"])
        .output()
        .expect("ebur128");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn integrated_lufs(file: &Path) -> f64 {
    let err = ebur128_log(file);
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

/// Max momentary loudness over the windows centred inside `spans`.
fn momentary_max(file: &Path, spans: &[(f64, f64)]) -> f64 {
    let mut best = f64::NEG_INFINITY;
    for line in ebur128_log(file).lines().filter(|l| l.contains("TARGET:")) {
        let field = |key: &str| -> Option<f64> {
            let rest = &line[line.find(key)? + key.len()..];
            rest.split_whitespace().next()?.parse().ok()
        };
        let (Some(t), Some(m)) = (field(" t:"), field(" M:")) else {
            continue;
        };
        let centre = t - 0.2;
        if t >= 0.4 && spans.iter().any(|&(a, b)| centre >= a && centre <= b) {
            best = best.max(m);
        }
    }
    best
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

/// 600 Hz "voice" speaking in [`SPEECH`], float wav, scaled to `lufs`.
fn make_voice(dir: &Path, name: &str, lufs: f64) -> PathBuf {
    let base = dir.join("voice_base.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "aevalsrc='0.1*sin(2*PI*600*t)*(between(t,0.5,3.5)+between(t,5.5,6.5))':d=8:s=48000",
        "-ac",
        "1",
        "-c:a",
        "pcm_f32le",
        s(&base),
    ]);
    let gain = lufs - integrated_lufs(&base);
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
    out
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

fn cue(time: f64, gain_db: f64) -> AudioCue {
    AudioCue {
        time,
        scene: "s".into(),
        kind: CueKind::Information,
        family: SfxFamily::Click,
        sound_id: "a".into(),
        gain_db,
        priority: 1,
        reason: "test".into(),
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

fn library(peak: f64) -> SfxLibrary {
    SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![SfxSound {
            id: "a".into(),
            family: SfxFamily::Click,
            path: "a.wav".into(),
            duration: 1.2,
            onset: 0.005,
            peak,
            audible_end: 0.03,
            peak_db: -18.0,
            lufs: None,
            sha256: "0".repeat(64),
            tags: vec![],
        }],
    }
}

fn silent_video(dir: &Path, seconds: &str) -> PathBuf {
    let video = dir.join("silent.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=64x64:r=30",
        "-t",
        seconds,
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        s(&video),
    ]);
    video
}

/// The envelope of the synthetic voice (two spans, a 2 s gap between them).
fn envelope(levels: &MixLevels) -> DuckEnvelope {
    envelope_for(&SPEECH, DURATION, levels)
}

/// The envelope of a voice speaking the given (start, end) word spans.
fn envelope_for(spans: &[(f64, f64)], duration: f64, levels: &MixLevels) -> DuckEnvelope {
    use motion_core::speech::{SpeechMap, SpeechWord, SPEECH_VERSION};
    let words = spans
        .iter()
        .enumerate()
        .map(|(i, &(start, end))| SpeechWord {
            text: format!("w{i}"),
            start,
            end,
            confidence: 1.0,
        })
        .collect();
    let map = SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences: vec![],
        recognised: vec![],
        alignment: None,
    };
    DuckEnvelope::from_speech(&map, levels)
}

fn relative(levels: &MixLevels) -> VoiceRelative {
    VoiceRelative {
        envelope: Some(envelope(levels)),
        voiced: SPEECH.to_vec(),
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

#[test]
fn a_quiet_and_a_loud_voice_come_out_of_the_mix_together() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_levels");
    let video = silent_video(&dir, "8");
    make_sound(&dir.join("a.wav"));
    make_bed(&dir.join("bed.wav"));
    let quiet = make_voice(&dir, "quiet.wav", -26.0);
    let loud = make_voice(&dir, "loud.wav", -14.0);
    let before = (
        std::fs::read(&quiet).expect("quiet"),
        std::fs::read(&loud).expect("loud"),
    );
    let levels = MixLevels::STANDARD;
    let p = plan(vec![cue(2.0, 6.0)], Some(bed()), Some(levels));
    let rel = relative(&levels);
    let mut stem_lufs = Vec::new();
    for (name, voice) in [("quiet", &quiet), ("loud", &loud)] {
        let stems = dir.join(format!("stems_{name}"));
        let vt = voice_track(voice);
        let files = write_stems(
            &video,
            &p,
            &dir,
            &library(0.015),
            Some(&dir),
            DURATION,
            &stems,
            &MixOptions {
                voice: Some(&vt),
                relative: Some(&rel),
                ..MixOptions::default()
            },
        )
        .expect("stems");
        let voice_stem = files.voice.expect("voice stem");
        let bed_stem = files.bed.expect("bed stem");
        assert!(files.sfx.expect("sfx stem").is_file());
        stem_lufs.push(integrated_lufs(&voice_stem));

        // The bed sits 18 dB under the voice while it speaks and rises 8 dB
        // (to 10 dB under) in the 2 s gap.
        let v = decode(&voice_stem, &band(600));
        let b = decode(&bed_stem, &band(200));
        let under = rms_db(&b, 1.0, 3.0) - rms_db(&v, 1.0, 3.0);
        assert!(
            (under + 18.0).abs() < 1.5,
            "{name}: bed under speech {under:.2} dB"
        );
        let gap = rms_db(&b, 4.2, 5.2) - rms_db(&b, 1.0, 3.0);
        assert!(
            (gap - 8.0).abs() < 1.5,
            "{name}: bed rise in the gap {gap:.2} dB"
        );
    }
    let (q, l) = (stem_lufs[0], stem_lufs[1]);
    assert!((q - l).abs() <= 0.5, "voice stems {q:.2} vs {l:.2} LUFS");
    assert!((q + 18.0).abs() <= 0.5, "voice stem {q:.2} LUFS");
    // The voice cache files are never modified.
    assert_eq!(std::fs::read(&quiet).expect("quiet"), before.0);
    assert_eq!(std::fs::read(&loud).expect("loud"), before.1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_sfx_bus_stays_six_db_under_the_voice() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_sfx");
    let video = silent_video(&dir, "8");
    make_sound(&dir.join("a.wav"));
    let voice = make_voice(&dir, "voice.wav", -22.0);
    let levels = MixLevels::STANDARD;
    // A very hot cue inside the first sentence.
    let p = plan(vec![cue(2.0, 18.0)], None, Some(levels));
    let rel = relative(&levels);
    let vt = voice_track(&voice);
    let files = write_stems(
        &video,
        &p,
        &dir,
        &library(0.015),
        None,
        DURATION,
        &dir.join("stems"),
        &MixOptions {
            voice: Some(&vt),
            relative: Some(&rel),
            ..MixOptions::default()
        },
    )
    .expect("stems");
    let voice_lufs = integrated_lufs(&files.voice.expect("voice"));
    let sfx = momentary_max(&files.sfx.expect("sfx"), &SPEECH);
    let over = sfx - voice_lufs;
    assert!(
        (-6.5..=-5.9).contains(&over),
        "sfx {sfx:.2} LUFS is {over:.2} dB over the voice ({voice_lufs:.2})"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The graphs of the 0.22 mix (`build_args` at a4cd21b), for the plan of
/// `legacy_inputs`: one cue at 1.0 s, a ducked bed, a voice at 0 s / 0.5 s.
const LEGACY_HEAD: &str = "[1:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,volume=6.0dB,adelay=800.000:all=1[c0];[c0]amix=inputs=1:normalize=0:duration=longest[sfx];[2:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,atrim=0:3.000,asetpts=PTS-STARTPTS,volume=-6.0dB,afade=t=in:st=0:d=0.500,afade=t=out:st=2.000:d=1.000[bed];";
const LEGACY_TAIL: &str = ";[vkey]volume=20dB,alimiter=limit=0.1:attack=2:release=40:level=disabled,asplit=2[vkbed][vksfx];[sfx]apad=whole_dur=3.000,asplit=2[sfxa][sfxkey];[sfxa][vksfx]sidechaincompress=threshold=0.045013:ratio=4:attack=15:release=250:knee=1[sfxd];[bed][vkbed]sidechaincompress=threshold=0.024756:ratio=20:attack=15:release=250:knee=1[bedv];[bedv][sfxkey]sidechaincompress=threshold=0.05:ratio=8:attack=20:release=300[bedd];[vomix][sfxd][bedd]amix=inputs=3:normalize=0:duration=longest[mix];[mix]apad=whole_dur=3.005,alimiter=limit=0.841:attack=5:level=disabled,atrim=start=0.005,asetpts=PTS-STARTPTS,atrim=0:3.000[aout]";
const LEGACY_VOICE: &str =
    "[3:a]aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo";
const LEGACY_VOICE_REST: &str =
    "asetpts=PTS-STARTPTS,apad=whole_dur=3.000,atrim=0:3.000,asplit=2[vomix][vkey]";

/// Empty stand-ins for the files (building a graph without `levels` only
/// checks that they exist).
fn legacy_inputs(name: &str) -> (PathBuf, SfxLibrary) {
    let dir = scratch(name);
    for f in ["a.wav", "bed.wav", "vo.wav", "v.mp4"] {
        std::fs::write(dir.join(f), b"").expect("file");
    }
    (dir, library(0.2))
}

#[test]
fn a_plan_without_levels_is_the_exact_pre_0_23_graph() {
    // No ffmpeg needed: without `levels` the graph is pure.
    let (dir, lib) = legacy_inputs("voice_relative_legacy");
    let p = plan(vec![cue(1.0, 6.0)], Some(bed()), None);
    for (offset, delay) in [(0.0, ""), (0.5, "adelay=500.000:all=1,")] {
        let vt = VoiceTrack {
            path: dir.join("vo.wav"),
            offset,
            gain_db: 0.0,
        };
        let g = mix_graph(
            &dir.join("v.mp4"),
            &p,
            &dir,
            &lib,
            Some(&dir),
            3.0,
            &MixOptions {
                voice: Some(&vt),
                ..MixOptions::default()
            },
        )
        .expect("graph");
        let want = format!("{LEGACY_HEAD}{LEGACY_VOICE},{delay}{LEGACY_VOICE_REST}{LEGACY_TAIL}");
        assert_eq!(g, want);
    }
    // A caller-set `gain_db` only adds a volume stage on the voice; the rest
    // of the legacy graph (absolute bed, sidechain) stays.
    let vt = VoiceTrack {
        path: dir.join("vo.wav"),
        offset: 0.0,
        gain_db: -3.0,
    };
    let g = mix_graph(
        &dir.join("v.mp4"),
        &p,
        &dir,
        &lib,
        Some(&dir),
        3.0,
        &MixOptions {
            voice: Some(&vt),
            ..MixOptions::default()
        },
    )
    .expect("graph");
    let want =
        format!("{LEGACY_HEAD}{LEGACY_VOICE},volume=-3.00dB,{LEGACY_VOICE_REST}{LEGACY_TAIL}");
    assert_eq!(g, want);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn with_levels_the_bed_has_no_sidechain_and_the_graph_is_deterministic() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_graph");
    let video = silent_video(&dir, "8");
    make_sound(&dir.join("a.wav"));
    make_bed(&dir.join("bed.wav"));
    let voice = make_voice(&dir, "voice.wav", -24.0);
    let vt = voice_track(&voice);
    let levels = MixLevels::STANDARD;
    let rel = relative(&levels);
    let graph = |cues: Vec<AudioCue>| {
        mix_graph(
            &video,
            &plan(cues, Some(bed()), Some(levels)),
            &dir,
            &library(0.015),
            Some(&dir),
            DURATION,
            &MixOptions {
                voice: Some(&vt),
                relative: Some(&rel),
                ..MixOptions::default()
            },
        )
        .expect("graph")
    };
    // Voice over a bed: no sidechain anywhere, the bed carries the envelope.
    let g = graph(vec![]);
    assert!(!g.contains("sidechaincompress"), "{g}");
    assert!(g.contains(",volume='pow(10,("), "{g}");
    assert!(g.contains(":eval=frame"), "{g}");
    assert!(g.contains("afade=t=in:st=0:d=0.500"), "{g}");
    assert!(g.contains("afade=t=out:st=7.000:d=1.000"), "{g}");
    assert!(g.contains(",volume="), "voice gain stage: {g}");
    // With SFX only the SFX bus is keyed on the voice; the bed never is.
    let g = graph(vec![cue(2.0, 6.0)]);
    assert!(g.contains("[sfxa][vksfx]sidechaincompress="), "{g}");
    for gone in ["[bed][vkbed]", "[bedv]", "[bedd]", "[vkbed]", "[sfxkey]"] {
        assert!(!g.contains(gone), "{gone} in {g}");
    }
    assert!(g.contains("[vomix][sfxd][bed]amix=inputs=3"), "{g}");
    // The same inputs give the same graph.
    assert_eq!(g, graph(vec![cue(2.0, 6.0)]));
    // A voice that starts 0.5 s into the project shifts the envelope with it
    // (the first point at 0.5 s, the first ramp ending at 0.35 + 0.5 s).
    let late = VoiceTrack {
        offset: 0.5,
        ..voice_track(&voice)
    };
    let g = mix_graph(
        &video,
        &plan(vec![], Some(bed()), Some(levels)),
        &dir,
        &library(0.015),
        Some(&dir),
        DURATION,
        &MixOptions {
            voice: Some(&late),
            relative: Some(&rel),
            ..MixOptions::default()
        },
    )
    .expect("graph");
    assert!(g.contains("adelay=500.000:all=1"), "{g}");
    assert!(g.contains("clip((t-0.850)/0.150,0,1)"), "{g}");
    assert!(!g.contains("if("), "{g}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_voice_gain_reaches_the_target_without_exceeding_the_peak_ceiling() {
    // Toward -18 LUFS.
    assert_eq!(voice_gain_db(-26.0, -20.0), 8.0);
    assert_eq!(voice_gain_db(-14.0, -10.0), -4.0);
    assert_eq!(voice_gain_db(-18.0, -10.0), 0.0);
    // Reduced so the true peak after the gain is at most -3 dBTP.
    assert_eq!(voice_gain_db(-20.0, -5.0), 2.0);
    assert_eq!(voice_gain_db(-20.0, -2.0), -1.0);
    assert_eq!(voice_gain_db(-24.0, 0.5), -3.5);
    // Unmeasurable peak: the loudness target alone.
    assert_eq!(voice_gain_db(-22.0, f64::NEG_INFINITY), 4.0);
}

/// A 60-sentence narration (0.85 s gaps, all long: 240 envelope points, 122
/// moving pairs) must mix: the bed's gain expression used to be a nested `if(`
/// chain, and a flat `+` chain would still be too long for ffmpeg's parser
/// (about 90 operands): both fail past ~100 levels.
#[test]
fn a_long_narration_with_many_long_gaps_mixes_end_to_end() {
    if !tools() {
        return;
    }
    const SECONDS: f64 = 63.0;
    let dir = scratch("voice_relative_long");
    let video = silent_video(&dir, "63");
    make_sound(&dir.join("a.wav"));
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.2*sin(2*PI*200*t):d=63:s=48000",
        "-ac",
        "1",
        s(&dir.join("bed.wav")),
    ]);
    let voice = dir.join("voice.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "aevalsrc=0.1*sin(2*PI*600*t):d=63:s=48000",
        "-ac",
        "1",
        "-c:a",
        "pcm_f32le",
        s(&voice),
    ]);
    let spans: Vec<(f64, f64)> = (0..60)
        .map(|i| (1.0 + f64::from(i), 1.15 + f64::from(i)))
        .collect();
    let levels = MixLevels::STANDARD;
    let envelope = envelope_for(&spans, SECONDS, &levels);
    assert!(
        envelope.points.len() > 200,
        "{} points",
        envelope.points.len()
    );
    let rel = VoiceRelative {
        envelope: Some(envelope),
        voiced: spans.clone(),
        ..VoiceRelative::default()
    };
    let p = plan(
        vec![cue(2.5, 6.0), cue(20.5, 6.0), cue(58.5, 6.0)],
        Some(bed()),
        Some(levels),
    );
    let vt = voice_track(&voice);
    let out = dir.join("mixed.mp4");
    let stems = dir.join("stems");
    motion_render::audio_mix::mix_audio_ex(
        &video,
        &p,
        &dir,
        &library(0.015),
        Some(&dir),
        SECONDS,
        &out,
        &MixOptions {
            voice: Some(&vt),
            relative: Some(&rel),
            stems: Some(&stems),
            ..MixOptions::default()
        },
    )
    .expect("a long narration mixes");
    assert!(out.is_file());
    // The late gaps rise as the early ones do: -18 dB inside a word, -10 dB
    // held in the 0.2 s after the 0.5 s ramp.
    let bed_stem = decode(&stems.join("bed.wav"), &band(200));
    for k in [3.0, 30.0, 57.0] {
        let under = rms_db(&bed_stem, 1.0 + k, 1.15 + k);
        let gap = rms_db(&bed_stem, 1.15 + k + 0.54, 1.15 + k + 0.66);
        assert!(
            (gap - under - 8.0).abs() < 1.5,
            "sentence {k}: bed {under:.2} dB under the word, {gap:.2} dB in the gap"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The format stage every input of the graph goes through.
const BASE: &str = "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,";

/// A 12 s bed that changes pitch at 4 s: 200 Hz, then 300 Hz.
fn make_stepped_bed(path: &Path) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "aevalsrc='0.2*if(lt(t,4),sin(2*PI*200*t),sin(2*PI*300*t))':d=12:s=48000",
        "-ac",
        "1",
        s(path),
    ]);
}

#[test]
fn a_bed_start_offset_and_end_fade_shape_the_graph_text() {
    // No ffmpeg needed: a plan without `levels` builds a pure graph.
    let (dir, lib) = legacy_inputs("voice_relative_bed_start_graph");
    let moved = MusicBed {
        start: 1.5,
        fade_out_at: Some(2.5),
        ..bed()
    };
    let p = plan(vec![cue(1.0, 6.0)], Some(moved), None);
    let vt = VoiceTrack {
        path: dir.join("vo.wav"),
        offset: 0.0,
        gain_db: 0.0,
    };
    let graph = |p: &AudioPlan, voice: Option<&VoiceTrack>| {
        mix_graph(
            &dir.join("v.mp4"),
            p,
            &dir,
            &lib,
            Some(&dir),
            3.0,
            &MixOptions {
                voice,
                ..MixOptions::default()
            },
        )
        .expect("graph")
    };
    // With a voice-over: the bed is trimmed from the offset and its fade ends
    // at the downbeat (2.5 s into the video, so it starts at 1.5 s).
    let g = graph(&p, Some(&vt));
    assert!(
        g.contains(&format!(
            "[2:a]{BASE}atrim=1.500:4.500,asetpts=PTS-STARTPTS,volume=-6.0dB,afade=t=in:st=0:d=0.500,afade=t=out:st=1.500:d=1.000[bed]"
        )),
        "{g}"
    );
    // Apart from those two stages it is the 0.22 graph.
    let unmoved = plan(vec![cue(1.0, 6.0)], Some(bed()), None);
    let legacy = graph(&unmoved, Some(&vt));
    assert!(legacy.contains("atrim=0:3.000,asetpts=PTS-STARTPTS,volume=-6.0dB"));
    let swapped = g
        .replace("atrim=1.500:4.500", "atrim=0:3.000")
        .replace("afade=t=out:st=1.500", "afade=t=out:st=2.000");
    assert_eq!(swapped, legacy);
    // Without a voice-over the same bed stages are used.
    let g = graph(&p, None);
    assert!(
        g.contains("atrim=1.500:4.500,asetpts=PTS-STARTPTS,volume=-6.0dB,afade=t=in:st=0:d=0.500,afade=t=out:st=1.500:d=1.000[bed]"),
        "{g}"
    );
    // A fade position past the end is clamped to the video's end.
    let late = MusicBed {
        fade_out_at: Some(9.0),
        ..bed()
    };
    let g = graph(&plan(vec![], Some(late), None), None);
    assert!(g.contains("afade=t=out:st=2.000:d=1.000"), "{g}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_bed_started_into_the_track_stays_under_the_voice_by_the_same_margins() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_bed_start");
    let video = silent_video(&dir, "8");
    make_stepped_bed(&dir.join("bed.wav"));
    let voice = make_voice(&dir, "voice.wav", -22.0);
    let levels = MixLevels::STANDARD;
    let rel = relative(&levels);
    let vt = voice_track(&voice);
    // The bed starts 3 s into the track (its pitch change at 4 s lands at 1 s
    // of the video) and its fade ends at 6.8 s, as a downbeat there would.
    let moved = MusicBed {
        start: 3.0,
        fade_out_at: Some(6.8),
        ..bed()
    };
    let p = plan(vec![], Some(moved), Some(levels));
    let opts = MixOptions {
        voice: Some(&vt),
        relative: Some(&rel),
        ..MixOptions::default()
    };
    let lib = library(0.015);
    let g = mix_graph(&video, &p, &dir, &lib, Some(&dir), DURATION, &opts).expect("graph");
    assert!(g.contains("atrim=3.000:11.000,asetpts=PTS-STARTPTS"), "{g}");
    assert!(g.contains("afade=t=out:st=5.800:d=1.000"), "{g}");
    assert!(g.contains("afade=t=in:st=0:d=0.500"), "{g}");
    let files = write_stems(
        &video,
        &p,
        &dir,
        &lib,
        Some(&dir),
        DURATION,
        &dir.join("stems"),
        &opts,
    )
    .expect("stems");
    let v = decode(&files.voice.expect("voice stem"), &band(600));
    let bed_stem = files.bed.expect("bed stem");
    let (b200, b300) = (decode(&bed_stem, &band(200)), decode(&bed_stem, &band(300)));
    // The track's first 3 s are skipped: 200 Hz for the first second of the
    // video, 300 Hz after (without the offset it would be 200 Hz until 4 s).
    assert!(
        rms_db(&b200, 0.6, 0.9) - rms_db(&b300, 0.6, 0.9) > 15.0,
        "0.6-0.9 s is not the track's 3-4 s part"
    );
    assert!(
        rms_db(&b300, 1.2, 3.0) - rms_db(&b200, 1.2, 3.0) > 15.0,
        "1.2-3 s is not the track's 4-6 s part"
    );
    // The same margins as an unmoved bed: 18 dB under the voice while it
    // speaks, +8 dB in the 2 s gap.
    let under = rms_db(&b300, 1.2, 3.0) - rms_db(&v, 1.2, 3.0);
    assert!((under + 18.0).abs() < 1.5, "bed under speech {under:.2} dB");
    let gap = rms_db(&b300, 4.2, 5.2) - rms_db(&b300, 1.2, 3.0);
    assert!((gap - 8.0).abs() < 1.5, "bed rise in the gap {gap:.2} dB");
    // The fade finished at 6.8 s: the bed is silent after it.
    let after = rms_db(&b300, 7.0, 7.9);
    assert!(after < -100.0, "bed after the fade {after:.1} dB");
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// (A5c) Peaky voices: the voice limiter
// ---------------------------------------------------------------------------

/// Where the plosives of the peaky voice fall (s): each is a 2 ms-wide 150 Hz
/// burst of amplitude `burst`, peaking exactly there.
const PLOSIVES: [f64; 5] = [0.8, 1.6, 2.4, 3.2, 5.8];

/// A 600 Hz "voice" speaking in [`SPEECH`] at amplitude `tone`, with a plosive
/// burst of amplitude `burst` at each of [`PLOSIVES`]: float wav, mono, no
/// randomness.
fn make_peaky_voice(dir: &Path, name: &str, tone: f64, burst: f64) -> PathBuf {
    let bursts: String = PLOSIVES
        .iter()
        .map(|t0| format!("+{burst}*cos(2*PI*150*(t-{t0}))*exp(-pow((t-{t0})/0.002,2))"))
        .collect();
    let out = dir.join(name);
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &format!(
            "aevalsrc='{tone}*sin(2*PI*600*t)*(between(t,0.5,3.5)+between(t,5.5,6.5)){bursts}':d=8:s=48000"
        ),
        "-ac",
        "1",
        "-c:a",
        "pcm_f32le",
        s(&out),
    ]);
    out
}

/// Integrated loudness and true peak (dBTP) of `file`'s first audio stream
/// through the mix's own format chain (stereo, 48 kHz).
fn loudness_and_peak(file: &Path) -> (f64, f64) {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-v", "info", "-i"])
        .arg(file)
        .args([
            "-map",
            "0:a:0",
            "-af",
            "aformat=sample_fmts=fltp:sample_rates=48000:channel_layouts=stereo,ebur128=peak=true",
            "-f",
            "null",
            "-",
        ])
        .output()
        .expect("ebur128");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    let summary = err.rsplit("Summary:").next().expect("summary");
    let value = |label: &str| -> f64 {
        let line = summary
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(label))
            .expect("summary line");
        line[label.len()..]
            .split_whitespace()
            .next()
            .expect("value")
            .parse()
            .expect("number")
    };
    (value("I:"), value("Peak:"))
}

/// The time (s) of the largest absolute value of `x` between `from` and `to`
/// seconds.
fn peak_time(x: &[f32], from: f64, to: f64) -> f64 {
    let (a, b) = ((from * 48_000.0) as usize, (to * 48_000.0) as usize);
    let mut best = (0.0f32, a);
    for (i, v) in x.iter().enumerate().take(b.min(x.len())).skip(a) {
        if v.abs() > best.0 {
            best = (v.abs(), i);
        }
    }
    best.1 as f64 / 48_000.0
}

/// Sample peak of `x` in dBFS.
fn peak_db(x: &[f32]) -> f64 {
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    20.0 * f64::from(peak).max(1e-9).log10()
}

#[test]
fn the_voice_limiter_gain_follows_the_peak_cap() {
    let ceiling = voice_limiter_ceiling_db();
    assert!((ceiling + 3.3).abs() < 1e-9, "ceiling {ceiling}");
    // The loudness gain keeps the peaks under the ceiling: plain gain, no limiter.
    assert_eq!(voice_limited_gain_db(-20.0, -5.0), None);
    assert_eq!(voice_limited_gain_db(-26.0, -20.0), None);
    assert_eq!(voice_limited_gain_db(-14.0, -2.0), None);
    // Within 0.5 dB of the ceiling: left to the plain gain.
    assert_eq!(voice_limited_gain_db(-20.0, -4.6), None);
    // A peaky take (the real one: -25.2 LUFS, -5.9 dBTP): the loudness gain,
    // 4.3 dB of peak reduction (against the -3.3 dB ceiling) instead of the
    // 2.9 dB the plain gain allows.
    assert!((voice_gain_db(-25.2, -5.9) - 2.9).abs() <= 0.011);
    let peaky = voice_limited_gain_db(-25.2, -5.9).expect("limited");
    assert!((peaky - 7.2).abs() <= 0.011, "{peaky}");
    // Capped at VOICE_LIMIT_MAX_DB of peak reduction: -30 LUFS, -2 dBTP.
    let capped = voice_limited_gain_db(-30.0, -2.0).expect("limited");
    assert!((capped - (ceiling + VOICE_LIMIT_MAX_DB + 2.0)).abs() <= 0.01);
    assert!(capped < 12.0);
    // Unmeasurable peak: the plain gain.
    assert_eq!(voice_limited_gain_db(-22.0, f64::NEG_INFINITY), None);
}

#[test]
fn a_voice_that_needs_no_limiter_has_no_limiter_stage() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_no_limiter");
    let video = silent_video(&dir, "8");
    make_bed(&dir.join("bed.wav"));
    // Quiet and loud sines (crest factor 3 dB): the plain gain reaches -18.
    for (name, lufs) in [("quiet.wav", -26.0), ("loud.wav", -14.0)] {
        let voice = make_voice(&dir, name, lufs);
        let vt = voice_track(&voice);
        let levels = MixLevels::STANDARD;
        let g = mix_graph(
            &video,
            &plan(vec![], Some(bed()), Some(levels)),
            &dir,
            &library(0.015),
            Some(&dir),
            DURATION,
            &MixOptions {
                voice: Some(&vt),
                relative: Some(&relative(&levels)),
                ..MixOptions::default()
            },
        )
        .expect("graph");
        assert!(!g.contains("alimiter=limit=0.6839"), "{name}: {g}");
        assert!(g.contains(",volume="), "{name}: {g}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_peaky_voice_reaches_the_voice_target_and_the_final_loudness() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_peaky");
    let video = silent_video(&dir, "8");
    make_sound(&dir.join("a.wav"));
    make_bed(&dir.join("bed.wav"));
    let voice = make_peaky_voice(&dir, "peaky.wav", 0.1, 0.95);
    let before = std::fs::read(&voice).expect("voice");

    // The voice is peaky by any measure: crest factor (peak over the RMS of
    // the speech) and peak-to-loudness ratio are both above 20 dB.
    let raw = decode(&voice, "anull");
    let crest = peak_db(&raw) - 0.5 * (rms_db(&raw, 1.0, 3.0) + rms_db(&raw, 5.5, 6.5));
    let (raw_lufs, raw_tp) = loudness_and_peak(&voice);
    assert!(crest >= 20.0, "crest factor {crest:.1} dB");
    assert!(
        raw_tp - raw_lufs >= 20.0,
        "peak-to-loudness {:.1} dB",
        raw_tp - raw_lufs
    );
    // The plain gain (peaks at -3 dBTP) would stop well short of the target.
    let plain = voice_gain_db(raw_lufs, raw_tp);
    assert!(VOICE_LUFS - (raw_lufs + plain) > 2.0, "plain gain {plain}");

    let levels = MixLevels::STANDARD;
    let p = plan(vec![cue(2.0, 6.0)], Some(bed()), Some(levels));
    let rel = relative(&levels);
    let vt = voice_track(&voice);
    let opts = MixOptions {
        voice: Some(&vt),
        relative: Some(&rel),
        ..MixOptions::default()
    };
    let lib = library(0.015);
    let g = mix_graph(&video, &p, &dir, &lib, Some(&dir), DURATION, &opts).expect("graph");
    // The limiter sits after the gain and before the mix (the stem has it).
    assert!(
        g.contains("alimiter=limit=0.6839:attack=5:release=100:level=disabled,atrim=start=0.005,asetpts=PTS-STARTPTS"),
        "{g}"
    );
    let gain_at = g.find(",volume=").expect("voice gain");
    let limiter_at = g.find("alimiter=limit=0.6839").expect("voice limiter");
    let final_at = g.find("alimiter=limit=0.841").expect("final limiter");
    assert!(gain_at < limiter_at && limiter_at < final_at, "{g}");

    // The voice stem: at the voice target within 1 dB, true peak under the
    // ceiling, the plosives where they were, the tone raised, not squashed.
    let stems = dir.join("stems");
    let files =
        write_stems(&video, &p, &dir, &lib, Some(&dir), DURATION, &stems, &opts).expect("stems");
    let voice_stem = files.voice.expect("voice stem");
    let (stem_lufs, stem_tp) = loudness_and_peak(&voice_stem);
    assert!(
        (stem_lufs - VOICE_LUFS).abs() <= 1.0,
        "voice stem {stem_lufs:.2} LUFS"
    );
    assert!(
        stem_tp <= VOICE_TRUE_PEAK_DB + 0.1,
        "voice stem true peak {stem_tp:.2} dBTP"
    );
    let v = decode(&voice_stem, "anull");
    for t in PLOSIVES {
        let at = peak_time(&v, t - 0.02, t + 0.02);
        assert!((at - t).abs() <= 0.001, "plosive {t} s lands at {at:.4} s");
    }
    // Peak reduction: the gain is read off a burst-free stretch of the tone
    // (the rest of the stem is the same tone, raised by that gain).
    let gain = rms_db(&v, 1.2, 1.5) - rms_db(&raw, 1.2, 1.5);
    let reduction = peak_db(&raw) + gain - peak_db(&v);
    assert!(
        reduction > 3.0 && reduction <= VOICE_LIMIT_MAX_DB + 0.2,
        "gain {gain:.2} dB, peak reduction {reduction:.2} dB"
    );
    // The bed keeps its place under the voice (18 dB under while it speaks).
    let bed_stem = files.bed.expect("bed stem");
    let (vb, bb) = (
        decode(&voice_stem, &band(600)),
        decode(&bed_stem, &band(200)),
    );
    let under = rms_db(&bb, 1.0, 3.0) - rms_db(&vb, 1.0, 3.0);
    assert!((under + 18.0).abs() < 1.5, "bed under speech {under:.2} dB");

    // The shipped mix (AAC) with the automatic makeup gain: -16 +- 1 LUFS and
    // under -1 dBTP.
    let out = dir.join("mixed.mp4");
    mix_audio_ex(
        &video,
        &p,
        &dir,
        &lib,
        Some(&dir),
        DURATION,
        &out,
        &MixOptions {
            makeup: MakeupGain::Auto,
            ..opts
        },
    )
    .expect("mix");
    let (lufs, tp) = loudness_and_peak(&out);
    assert!((lufs + 16.0).abs() <= 1.0, "final {lufs:.2} LUFS");
    assert!(tp <= -1.0, "final true peak {tp:.2} dBTP");
    // The voice cache file is never modified.
    assert_eq!(std::fs::read(&voice).expect("voice"), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_voice_too_peaky_for_the_limiter_stops_at_the_peak_reduction_cap() {
    if !tools() {
        return;
    }
    let dir = scratch("voice_relative_too_peaky");
    let video = silent_video(&dir, "8");
    // Tone 0.03: peak-to-loudness above 30 dB, so the target needs more than
    // 15 dB of peak reduction and the limiter may do VOICE_LIMIT_MAX_DB.
    let voice = make_peaky_voice(&dir, "peaky.wav", 0.03, 0.95);
    let levels = MixLevels::STANDARD;
    let vt = voice_track(&voice);
    let p = plan(vec![], None, Some(levels));
    let rel = relative(&levels);
    let files = write_stems(
        &video,
        &p,
        &dir,
        &library(0.015),
        None,
        DURATION,
        &dir.join("stems"),
        &MixOptions {
            voice: Some(&vt),
            relative: Some(&rel),
            ..MixOptions::default()
        },
    )
    .expect("stems");
    let voice_stem = files.voice.expect("voice stem");
    let raw = decode(&voice, "anull");
    let v = decode(&voice_stem, "anull");
    let (lufs, tp) = loudness_and_peak(&voice_stem);
    assert!(tp <= VOICE_TRUE_PEAK_DB + 0.1, "true peak {tp:.2} dBTP");
    let gain = rms_db(&v, 1.2, 1.5) - rms_db(&raw, 1.2, 1.5);
    let reduction = peak_db(&raw) + gain - peak_db(&v);
    assert!(
        reduction <= VOICE_LIMIT_MAX_DB + 0.2,
        "gain {gain:.2} dB, peak reduction {reduction:.2} dB"
    );
    // It lands under the target (the final makeup makes up the rest) and
    // louder than the plain gain would have left it.
    let (raw_lufs, raw_tp) = loudness_and_peak(&voice);
    assert!(lufs < VOICE_LUFS - 0.5, "voice stem {lufs:.2} LUFS");
    assert!(
        lufs > raw_lufs + voice_gain_db(raw_lufs, raw_tp) + 1.0,
        "voice stem {lufs:.2} LUFS"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
