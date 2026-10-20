//! (0.24 E2) Sound for `motion-engine edit`: the speaker's voice is the anchor
//! (untouched apart from the mixer's gentle loudness normalisation), a music
//! bed from `assets/music` ducks under it, and SFX cues land on cutaway and
//! side-panel entries at the M7 levels. The mix itself is the existing
//! voice-relative mixer (`audio_cmd::run_mix`); this file only builds its inputs
//! and measures the result.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use motion_core::audio::{
    AudioCue, AudioPlan, CueKind, MixLevels, MusicPlan, SfxFamily, SfxSound, AUDIO_PLAN_VERSION,
    LOUDNESS_TARGET_LUFS, MAX_GAIN_DB, MIN_GAIN_DB, TRUE_PEAK_LIMIT_DB,
};
use motion_core::edit::{EditPlan, EventKind, Tok};
use motion_core::speech::SpeechMap;

use crate::audio_cmd::{run_mix, MixJob};

/// Beds that sit under a voice (see `assets/music/catalog.json`).
const BEDS: [&str; 3] = ["calm_ambient", "warm_piano", "tech_pulse"];
const PEAK_CEILING_DB: f64 = -2.0;
/// Marks a cue placed in a tight spot (the first to go quieter or be dropped).
const TIGHT: &str = " [tight]";
/// Loudness the cues are set to (LUFS of the file): well under the voice,
/// so sfx_over_voice holds its 10 LU margin.
const SFX_HERO_LUFS: f64 = -42.0;
const SFX_SMALL_LUFS: f64 = -45.0;

pub struct SoundInputs<'a> {
    pub video: &'a Path,
    pub source: &'a Path,
    pub transcript: &'a SpeechMap,
    pub plan: &'a EditPlan,
    pub toks: &'a [Tok],
    /// 10 ms voice-band loudness (dB) of the source.
    pub env: &'a [f64],
    /// Event indices of the cutaways that actually rendered.
    pub cut_events: &'a [usize],
    pub assets: &'a Path,
    pub tmp: &'a Path,
    pub duration: f64,
    pub output: &'a Path,
    pub name: &'a str,
}

pub struct SoundReport {
    pub summary: String,
    pub bed: String,
    pub cues: usize,
    pub dropped_cues: usize,
    pub sfx_margin_lu: Option<f64>,
    pub lufs: Option<f64>,
    pub true_peak: Option<f64>,
}

fn pick(lib: &[SfxSound], fam: SfxFamily, k: usize) -> Option<&SfxSound> {
    let v: Vec<&SfxSound> = lib.iter().filter(|s| s.family == fam).collect();
    (!v.is_empty()).then(|| v[k % v.len()])
}

fn cue(
    lib: &[SfxSound],
    fam: SfxFamily,
    kind: CueKind,
    k: usize,
    time: f64,
    scene: String,
    why: &str,
) -> Option<AudioCue> {
    let s = pick(lib, fam, k)?;
    let lufs = s.lufs.unwrap_or(-23.0);
    // Under a voice the family target is a ceiling.
    let target = if kind == CueKind::Information {
        SFX_SMALL_LUFS
    } else {
        SFX_HERO_LUFS
    };
    let gain = (target - lufs)
        .min(PEAK_CEILING_DB - s.peak_db)
        .clamp(MIN_GAIN_DB, MAX_GAIN_DB);
    Some(AudioCue {
        time: (time * 1000.0).round() / 1000.0,
        scene,
        kind,
        family: fam,
        sound_id: s.id.clone(),
        gain_db: (gain * 10.0).round() / 10.0,
        priority: kind.priority(),
        reason: why.to_string(),
    })
}

fn ffmpeg(args: &[&str]) -> Result<()> {
    let st = Command::new("ffmpeg")
        .args(["-y", "-v", "error", "-nostdin"])
        .args(args)
        .status()
        .context("cannot run ffmpeg")?;
    if !st.success() {
        bail!("ffmpeg failed: {args:?}");
    }
    Ok(())
}

/// The bed looped (crossfaded) to `duration` as a wav beside a copy of its
/// `.music.json`; returns the plan path.
fn looped_bed(assets: &Path, id: &str, duration: f64, tmp: &Path) -> Result<PathBuf> {
    let src = assets.join("music").join(format!("{id}.mp3"));
    let plan_src = assets.join("music").join(format!("{id}.music.json"));
    let mut plan: MusicPlan = serde_json::from_str(&std::fs::read_to_string(&plan_src)?)?;
    let xf = 3.0;
    let n = ((duration / (plan.duration - xf).max(10.0)).ceil() as usize + 1).max(1);
    let wav = tmp.join("bed_loop.wav");
    let mut args: Vec<String> = Vec::new();
    for _ in 0..n {
        args.push("-i".into());
        args.push(src.to_string_lossy().into_owned());
    }
    let mut graph = String::new();
    let mut last = "0:a".to_string();
    for k in 1..n {
        let out = format!("x{k}");
        graph.push_str(&format!("[{last}][{k}:a]acrossfade=d={xf}[{out}];"));
        last = out;
    }
    graph.push_str(&format!(
        "[{last}]atrim=0:{:.2},asetpts=PTS-STARTPTS[o]",
        duration + 2.0
    ));
    args.extend(["-filter_complex".into(), graph, "-map".into(), "[o]".into()]);
    args.push(wav.to_string_lossy().into_owned());
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    ffmpeg(&refs)?;
    plan.track = "bed_loop.wav".into();
    plan.duration = duration + 2.0;
    let out = tmp.join("bed.music.json");
    std::fs::write(&out, serde_json::to_string_pretty(&plan)?)?;
    Ok(out)
}

/// Integrated LUFS and true peak of `file` (ebur128).
pub fn measure(file: &Path) -> Result<(f64, f64)> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-v", "info", "-i"])
        .arg(file)
        .args(["-af", "ebur128=peak=true", "-f", "null", "-"])
        .output()
        .context("cannot run ffmpeg")?;
    let text = String::from_utf8_lossy(&out.stderr);
    let tail = text.rsplit("Summary:").next().unwrap_or("");
    let grab = |key: &str| -> Option<f64> {
        tail.lines()
            .find(|l| l.trim_start().starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    let i = grab("I:").context("no integrated loudness")?;
    let p = grab("Peak:").context("no true peak")?;
    Ok((i, p))
}

/// Runs of voiced 10 ms frames (voice-band loudness), merged across gaps
/// under 0.25 s.
fn voiced_runs(env: &[f64]) -> Vec<(f64, f64)> {
    let mut sorted: Vec<f64> = env.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let n = sorted.len().max(1);
    let floor = sorted.get(n / 10).copied().unwrap_or(-80.0);
    let peak = sorted.get(n * 95 / 100).copied().unwrap_or(-20.0);
    let thr = floor + (peak - floor) * 0.55;
    let mut runs: Vec<(f64, f64)> = Vec::new();
    for (i, &v) in env.iter().enumerate() {
        if v <= thr {
            continue;
        }
        let (a, b) = (i as f64 / 100.0, (i + 1) as f64 / 100.0);
        match runs.last_mut() {
            Some(l) if a - l.1 < 0.25 => l.1 = b,
            _ => runs.push((a, b)),
        }
    }
    runs.retain(|r| r.1 - r.0 >= 0.1);
    runs
}

/// The time nearest `t` (within 0.4 s) whose sound window `[x - 0.1, x + tail]`
/// holds no voice; `None` when there is no such quiet spot.
fn quiet_spot(runs: &[(f64, f64)], t: f64, tail: f64, m: f64) -> Option<f64> {
    (0..=20)
        .flat_map(|k| {
            let d = k as f64 * 0.02;
            [t + d, t - d]
        })
        .find(|&x| x > 0.0 && runs.iter().all(|r| r.1 <= x - m || r.0 >= x + tail + m))
}

pub fn run(inp: &SoundInputs) -> Result<SoundReport> {
    let tmp = inp.tmp.join("sound");
    std::fs::create_dir_all(&tmp)?;
    // The voice: the source's own audio.
    let voice = tmp.join("voice.wav");
    ffmpeg(&[
        "-i",
        &inp.source.to_string_lossy(),
        "-vn",
        "-ac",
        "1",
        "-ar",
        "48000",
        "-c:a",
        "pcm_s16le",
        &voice.to_string_lossy(),
    ])?;
    let mut map = inp.transcript.clone();
    map.audio = "voice.wav".into();
    // The mixer follows measured voice activity, not recogniser word times
    // (those can span a pause).
    let runs = voiced_runs(inp.env);
    map.words = runs
        .iter()
        .map(|&(a, b)| motion_core::speech::SpeechWord {
            text: "~".into(),
            start: a,
            end: b,
            confidence: 1.0,
        })
        .collect();
    map.sentences = runs
        .iter()
        .enumerate()
        .map(|(k, &(a, b))| motion_core::speech::SpeechSentence {
            beat: k,
            start: a,
            end: b,
        })
        .collect();
    let speech = tmp.join("voice.speech.json");
    std::fs::write(&speech, serde_json::to_string(&map)?)?;

    let bed = BEDS[(inp
        .name
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32))
        % BEDS.len() as u32) as usize];
    let music = looped_bed(inp.assets, bed, inp.duration, &tmp)?;

    let sfx_dir = inp.assets.join("sfx/library");
    let (sfx_root, library) =
        motion_render::sfx::load_library(&sfx_dir).map_err(|e| anyhow::anyhow!("{e}"))?;
    let sounds = &library.sounds;
    let mut cues: Vec<AudioCue> = Vec::new();
    let mut n = 0usize;
    for (i, ev) in inp.plan.events.iter().enumerate() {
        match &ev.kind {
            EventKind::Cutaway { .. } if inp.cut_events.contains(&i) => {
                // The cut lands in a gap: a soft whoosh peaking on it, a
                // soft hit as the key word lands.
                cues.extend(cue(
                    sounds,
                    SfxFamily::WhooshSoft,
                    CueKind::Handoff,
                    n,
                    ev.start + 0.12,
                    format!("cut{i}"),
                    "cutaway entry",
                ));
                let land = inp.toks[ev.at_word].start + 0.25;
                if land + 0.1 < ev.end {
                    cues.extend(cue(
                        sounds,
                        SfxFamily::HitSoft,
                        CueKind::Impact,
                        n,
                        land,
                        format!("cut{i}"),
                        "cutaway landing",
                    ));
                }
                n += 1;
            }
            EventKind::SidePanel { .. } => {
                cues.extend(cue(
                    sounds,
                    SfxFamily::Pop,
                    CueKind::Information,
                    n,
                    ev.start + 0.1,
                    format!("panel{i}"),
                    "side panel",
                ));
                n += 1;
            }
            _ => {}
        }
    }
    // Each cue peaks in a voice-free spot (or is dropped): the mix keeps the
    // bed under the voice, and the SFX never competes with a word.
    let before = cues.len();
    cues = cues
        .into_iter()
        .filter_map(|mut c| {
            let tail = sounds
                .iter()
                .find(|s| s.id == c.sound_id)
                .map_or(0.4, |s| (s.audible_end - s.peak).clamp(0.1, 0.4));
            // A clear gap first; else a tighter spot with the cue 9 dB down.
            let (t, drop_db) = match quiet_spot(&runs, c.time, tail, 0.25) {
                Some(t) => (t, 0.0),
                None => (quiet_spot(&runs, c.time, tail.min(0.25), 0.06)?, 9.0),
            };
            c.time = (t * 1000.0).round() / 1000.0;
            c.gain_db = ((c.gain_db - drop_db) * 10.0).round() / 10.0;
            if drop_db > 0.0 {
                c.reason.push_str(TIGHT);
            }
            Some(c)
        })
        .collect();
    let dropped_cues = before - cues.len();
    cues.sort_by(|a, b| a.time.total_cmp(&b.time));
    let mut plan = AudioPlan {
        version: AUDIO_PLAN_VERSION.to_string(),
        cues,
        loudness_target: LOUDNESS_TARGET_LUFS,
        true_peak_limit: TRUE_PEAK_LIMIT_DB,
        min_spacing: 0.0,
        information_cap: 0,
        music: None,
        speech_adjustments: Vec::new(),
        levels: Some(MixLevels::STANDARD),
    };
    // sfx_over_voice must pass: measure the SFX stem against the voice stem
    // and, until it does, take the tight-spot cues down, then out, then every
    // cue down.
    let qa_of = |plan: &AudioPlan| {
        motion_render::mix_qa::sfx_over_voice_qa(&motion_render::mix_qa::SfxQa {
            plan,
            speech: &map,
            voice: &voice,
            sfx_root: &sfx_root,
            library: &library,
            duration: inp.duration,
            temp_base: Some(&tmp),
        })
        .map_err(|e| anyhow::anyhow!("{e}"))
    };
    let mut qa = qa_of(&plan)?;
    let mut dropped_cues = dropped_cues;
    for round in 0..6 {
        let ok = qa
            .worst_margin_lu
            .is_none_or(|m| m >= motion_render::mix_qa::SFX_OVER_VOICE_MIN_LU + 0.5);
        if ok {
            break;
        }
        match round {
            0..=2 => plan
                .cues
                .iter_mut()
                .filter(|c| c.reason.ends_with(TIGHT))
                .for_each(|c| c.gain_db -= 6.0),
            3 => {
                let n = plan.cues.len();
                plan.cues.retain(|c| !c.reason.ends_with(TIGHT));
                dropped_cues += n - plan.cues.len();
            }
            _ => plan.cues.iter_mut().for_each(|c| c.gain_db -= 6.0),
        }
        qa = qa_of(&plan)?;
    }
    let plan_path = tmp.join("edit.audio.json");
    std::fs::write(&plan_path, plan.to_json_pretty() + "\n")?;

    let summary = run_mix(&MixJob {
        video: inp.video,
        duration: inp.duration,
        output: inp.output,
        sfx_library: Some(&sfx_dir),
        audio_plan: Some(plan_path.clone()),
        speech: Some(&speech),
        music: Some(&music),
        makeup: None,
        legacy: false,
        stems: None,
    })?;

    let (lufs, tp) = match measure(inp.output) {
        Ok((i, p)) => (Some(i), Some(p)),
        Err(_) => (None, None),
    };
    Ok(SoundReport {
        summary,
        bed: bed.to_string(),
        cues: plan.cues.len(),
        dropped_cues,
        sfx_margin_lu: qa.worst_margin_lu,
        lufs,
        true_peak: tp,
    })
}

/// `intro` then `talk`, scaled and re-timed to the talk; the audio fades the
/// intro out and the talk in over 0.2 s (sync kept: nothing overlaps in time).
pub fn prepend_intro(
    intro: &Path,
    talk: &Path,
    canvas: (u32, u32),
    fps: u32,
    out: &Path,
) -> Result<()> {
    let dur = motion_render::video::probe(intro)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .duration_s;
    let (w, h) = canvas;
    let norm = format!("scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,fps={fps},format=yuv420p");
    let graph = format!(
        "[0:v]{norm}[v0];[1:v]{norm}[v1];\
         [0:a]aresample=48000,aformat=channel_layouts=stereo,afade=t=out:st={:.3}:d=0.2[a0];\
         [1:a]aresample=48000,aformat=channel_layouts=stereo,afade=t=in:st=0:d=0.2[a1];\
         [v0][a0][v1][a1]concat=n=2:v=1:a=1[v][a]",
        (dur - 0.2).max(0.0)
    );
    ffmpeg(&[
        "-i",
        &intro.to_string_lossy(),
        "-i",
        &talk.to_string_lossy(),
        "-filter_complex",
        &graph,
        "-map",
        "[v]",
        "-map",
        "[a]",
        "-c:v",
        "libx264",
        "-crf",
        "17",
        "-preset",
        "medium",
        "-c:a",
        "aac",
        "-b:a",
        "256k",
        "-movflags",
        "+faststart",
        &out.to_string_lossy(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sound(peak_db: f64, lufs: f64) -> SfxSound {
        SfxSound {
            id: "w1".into(),
            family: SfxFamily::WhooshSoft,
            path: "w1.wav".into(),
            duration: 0.5,
            onset: 0.0,
            peak: 0.1,
            audible_end: 0.4,
            peak_db,
            lufs: Some(lufs),
            sha256: String::new(),
            tags: Vec::new(),
        }
    }

    #[test]
    fn cues_sit_far_under_the_voice_and_under_the_peak_ceiling() {
        for (peak, lufs) in [(-3.0, -20.0), (1.8, -23.1), (-10.0, -40.0)] {
            let lib = vec![sound(peak, lufs)];
            let c = cue(
                &lib,
                SfxFamily::WhooshSoft,
                CueKind::Handoff,
                0,
                1.0,
                "cut0".into(),
                "t",
            )
            .expect("cue");
            assert!(
                lufs + c.gain_db <= SFX_HERO_LUFS + 0.1
                    || peak + c.gain_db <= PEAK_CEILING_DB + 0.1
            );
            assert!(peak + c.gain_db <= PEAK_CEILING_DB + 0.1);
            assert!(c.gain_db <= 0.0 || peak < -10.0);
        }
    }

    #[test]
    fn the_intro_is_prepended_with_a_short_audio_fade() {
        let dir = std::env::temp_dir().join(format!("me_intro_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, secs: u32| {
            let p = dir.join(name);
            let st = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
                .arg(format!("testsrc=size=320x180:rate=30:duration={secs}"))
                .args(["-f", "lavfi", "-i"])
                .arg(format!("sine=frequency=440:duration={secs}"))
                .args([
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                ])
                .arg(&p)
                .status()
                .unwrap();
            assert!(st.success());
            p
        };
        let (intro, talk) = (mk("i.mp4", 2), mk("t.mp4", 3));
        let out = dir.join("o.mp4");
        prepend_intro(&intro, &talk, (320, 180), 30, &out).unwrap();
        let d = motion_render::video::probe(&out).unwrap().duration_s;
        assert!((d - 5.0).abs() < 0.2, "duration {d}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
