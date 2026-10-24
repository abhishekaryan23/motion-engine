//! (0.24 M7) Count-up tick trains and SFX levels under a voice-over.
//! See docs/SOUND_DESIGN.md §3.

use std::path::PathBuf;

use motion_core::audio::{
    is_count_tick, palette_file, plan_audio, plan_audio_with_speech, AudioCue, AudioPlan,
    SfxFamily, SfxLibrary, SfxSound, COUNT_MAX_PER_S, COUNT_MIN_GAP, COUNT_SETTLE_CLEAR,
    SFX_LIBRARY_VERSION, SPEECH_CLEARANCE, VOICE_LUFS,
};
use motion_core::compiler::resolve_taste;
use motion_core::compiler::taste::ResolvedStyleProfile;
use motion_core::intent::Energy;
use motion_core::scene::MotionOp;
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::{
    compile, ApproxMeasure, AssetLibrary, CreativeIntent, MotionProject, StyleProfile,
};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn library() -> SfxLibrary {
    let mut sounds = Vec::new();
    for (fi, f) in SfxFamily::ALL.into_iter().enumerate() {
        for k in 0..6usize {
            let peak_db = -20.0 + ((fi * 7 + k * 5) % 24) as f64;
            sounds.push(SfxSound {
                id: format!("{}_{k:02}", f.as_str()),
                family: f,
                path: format!("sounds/{}/{}_{k:02}.wav", f.as_str(), f.as_str()),
                duration: 1.0,
                onset: 0.1,
                peak: 0.3,
                audible_end: 0.9,
                peak_db,
                lufs: Some(peak_db - 14.0 - (k as f64)),
                sha256: "0".repeat(64),
                tags: vec![],
            });
        }
    }
    sounds.sort_by(|a, b| a.id.cmp(&b.id));
    SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds,
    }
}

struct Demo {
    project: MotionProject,
    taste: ResolvedStyleProfile,
    energy: Vec<Energy>,
}

fn demo() -> Demo {
    let intent = CreativeIntent::from_json(
        &std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json")).expect("i"),
    )
    .expect("intent");
    let style = StyleProfile::from_json(
        &std::fs::read_to_string(repo().join("examples/editorial_demo.style.json")).expect("s"),
    )
    .expect("style");
    let project = compile(
        &intent,
        &style,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .expect("compiles");
    Demo {
        project,
        taste: resolve_taste(&intent, &style, None),
        energy: intent.beats.iter().map(|b| b.energy).collect(),
    }
}

/// Words every 0.4 s (0.25 s long), none between `quiet.0` and `quiet.1`.
fn speech(duration: f64, quiet: (f64, f64)) -> SpeechMap {
    let mut words = Vec::new();
    let mut t = 0.3;
    while t + 0.25 < duration {
        if t + 0.25 < quiet.0 || t > quiet.1 {
            words.push(SpeechWord {
                text: "w".into(),
                start: t,
                end: t + 0.25,
                confidence: 1.0,
            });
        }
        t += 0.4;
    }
    SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences: vec![SpeechSentence {
            beat: 0,
            start: 0.3,
            end: duration,
        }],
        recognised: Vec::new(),
        alignment: None,
    }
}

fn ticks(plan: &AudioPlan, scene: &str) -> Vec<AudioCue> {
    plan.cues
        .iter()
        .filter(|c| c.scene == scene && is_count_tick(c))
        .cloned()
        .collect()
}

fn loudness(lib: &SfxLibrary, c: &AudioCue) -> f64 {
    let s = lib.get(&c.sound_id).expect("sound");
    s.lufs.unwrap_or(s.peak_db - 18.0) + c.gain_db
}

/// (scene id, start, duration, easing progress at u) of each Count motion.
fn counts(p: &MotionProject) -> Vec<(String, f64, f64)> {
    let mut out = Vec::new();
    for s in &p.scenes {
        for m in &s.motions {
            if matches!(m.op, MotionOp::Count { .. }) && m.duration >= 0.5 {
                out.push((s.id.clone(), s.start_seconds + m.start, m.duration));
            }
        }
    }
    out
}

#[test]
fn a_count_gets_a_monotonic_train_that_stops_before_the_settle() {
    let lib = library();
    let d = demo();
    let plan = plan_audio(&d.project, &d.taste, &d.energy, &lib, None);
    let counts = counts(&d.project);
    assert!(!counts.is_empty(), "the demo has Count ops");
    let mut trains = 0;
    for (scene, start, dur) in counts {
        let t = ticks(&plan, &scene);
        if t.is_empty() {
            continue;
        }
        trains += 1;
        let end = start + dur;
        assert!(t.len() >= 3);
        for w in t.windows(2) {
            assert!(w[1].time - w[0].time >= COUNT_MIN_GAP - 1e-9, "{scene}");
            assert!(
                w[1].time - w[0].time >= 1.0 / COUNT_MAX_PER_S - 1e-9,
                "{scene}"
            );
            assert_ne!(w[0].sound_id, w[1].sound_id, "{scene}: same sound twice");
        }
        for c in &t {
            assert!(c.time >= start - 1e-9 && c.time <= end, "{scene}");
        }
        let last = t.last().expect("last").time;
        assert!(
            end - last >= COUNT_SETTLE_CLEAR - 1e-3,
            "{scene}: last tick {last} vs settle {end}"
        );
        // The settle itself holds the hero hit or the soft lock.
        assert!(
            plan.cues
                .iter()
                .any(|c| !is_count_tick(c) && (c.time - end).abs() < 0.2),
            "{scene}: nothing on the settle {end}"
        );
        // Quiet at the slow ends: the last tick is quieter than the loudest.
        let loud = t.iter().map(|c| loudness(&lib, c)).fold(f64::MIN, f64::max);
        assert!(loudness(&lib, &t[t.len() - 1]) < loud - 1.0, "{scene}");
    }
    assert!(trains > 0, "at least one train on the editorial demo");
}

fn long_count(easing: &str) -> MotionProject {
    let json = format!(
        r##"{{"version":"0.2","project":{{"name":"t","duration_seconds":9.0}},
        "canvas":{{"width":108,"height":192,"fps":30,"background":"#FFFFFF"}},
        "scenes":[{{"id":"beat_1","start_seconds":0.0,"duration_seconds":9.0,"layers":[],
          "motions":[
           {{"target":"beat_1.num","op":"count","start":1.0,"duration":4.0,"easing":"{easing}","from":0,"to":1000}}],
          "lifecycle":{{"enter":0.2,"settle":0.6,"read":1.0,"evolve":5.5,"anticipate":7.5,"bridge":9.0}}}}]}}"##
    );
    MotionProject::from_json(&json).expect("parse")
}

#[test]
fn tick_density_follows_the_easing() {
    let lib = library();
    let d = demo();
    // OutQuint (the counter's landing curve) is fastest at the start and
    // crawls at the end: more ticks in the first half, widening spacing.
    // InOutCubic is fastest in the middle: more ticks mid than at the ends.
    for (easing, start, dur) in [("out_quint", 1.0, 4.0), ("in_out_cubic", 1.0, 4.0)] {
        let p = long_count(easing);
        let plan = plan_audio(&p, &d.taste, &d.energy, &lib, None);
        let t = ticks(&plan, "beat_1");
        assert!(t.len() >= 8, "{easing}: {} ticks", t.len());
        for w in t.windows(2) {
            assert!(w[1].time > w[0].time);
        }
        let gaps: Vec<f64> = t.windows(2).map(|w| w[1].time - w[0].time).collect();
        if easing == "out_quint" {
            let mid = start + dur / 2.0;
            let first = t.iter().filter(|c| c.time < mid).count();
            assert!(first * 2 > t.len(), "{easing}: {first} of {}", t.len());
            assert!(gaps[gaps.len() - 1] > gaps[0] * 2.0, "{gaps:?}");
        } else {
            let q = |a: f64, b: f64| {
                t.iter()
                    .filter(|c| c.time >= start + a * dur && c.time < start + b * dur)
                    .count()
            };
            assert!(q(0.35, 0.65) > q(0.0, 0.3), "{easing}");
            assert!(q(0.35, 0.65) > q(0.7, 1.0), "{easing}");
        }
        // Peak rate stays inside the cap; the last tick is clear of the settle.
        assert!(gaps.iter().all(|g| *g >= 1.0 / COUNT_MAX_PER_S - 1e-9));
        assert!(start + dur - t[t.len() - 1].time >= COUNT_SETTLE_CLEAR - 1e-3);
    }
}

#[test]
fn trains_are_deterministic_and_do_not_disturb_other_cues() {
    let lib = library();
    let d = demo();
    let a = plan_audio(&d.project, &d.taste, &d.energy, &lib, None);
    let b = plan_audio(&d.project, &d.taste, &d.energy, &lib, None);
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
    // The QA floor (minimum transient spacing) ignores ticks.
    let rest: Vec<f64> = a
        .cues
        .iter()
        .filter(|c| !c.family.is_bed() && !is_count_tick(c))
        .map(|c| c.time)
        .collect();
    for w in rest.windows(2) {
        assert!(w[1] - w[0] >= a.min_spacing - 1e-9);
    }
}

#[test]
fn under_a_voice_over_every_cue_sits_twelve_lu_under_the_voice() {
    let lib = library();
    let d = demo();
    let dur = d.project.duration_seconds();
    let quiet = (3.0, 4.2);
    let sp = speech(dur, quiet);
    let plan = plan_audio_with_speech(&d.project, &d.taste, &d.energy, &lib, None, None, Some(&sp));
    let file = palette_file().expect("palettes");
    let v = file.voice.as_ref().expect("voice levels in palettes.json");
    // The table itself: the speaking targets are at least 12 LU under the voice.
    for t in [
        v.speaking.hero,
        v.speaking.transition,
        v.speaking.small,
        v.speaking.count,
    ] {
        assert!(t <= VOICE_LUFS - 12.0 + 1e-9, "{t}");
    }
    assert!(v.speaking.hero + v.gap_boost_db <= VOICE_LUFS - 8.0 + 1e-9);
    let speaking = |t: f64| {
        sp.words.iter().any(|w| t >= w.start && t <= w.end)
            || sp
                .words
                .windows(2)
                .any(|w| t >= w[0].end && t <= w[1].start && w[1].start - w[0].end < v.phrase_gap_s)
    };
    let mut n_speaking = 0;
    for c in &plan.cues {
        let level = loudness(&lib, c);
        if speaking(c.time) {
            n_speaking += 1;
            assert!(level <= VOICE_LUFS - 12.0 + 0.11, "{c:?}: {level}");
        } else {
            assert!(level <= VOICE_LUFS - 8.0 + 0.11, "{c:?}: {level}");
        }
        // The 80 ms rule still holds, ticks included.
        for w in &sp.words {
            assert!(
                (c.time - w.start).abs() >= SPEECH_CLEARANCE - 1e-3,
                "{c:?} near {}",
                w.start
            );
        }
    }
    assert!(n_speaking > 0);
    // Louder without a voice: the same plan with no speech is unchanged M5.
    let bare = plan_audio(&d.project, &d.taste, &d.energy, &lib, None);
    let hero = |p: &AudioPlan| {
        p.cues
            .iter()
            .filter(|c| c.reason.contains(": hero "))
            .map(|c| loudness(&lib, c))
            .fold(f64::MIN, f64::max)
    };
    assert!(hero(&bare) > hero(&plan) + 5.0);
}

#[test]
fn a_train_under_speech_is_capped_at_ten_per_second() {
    let lib = library();
    let d = demo();
    let sp = speech(d.project.duration_seconds(), (-1.0, -1.0));
    let plan = plan_audio_with_speech(&d.project, &d.taste, &d.energy, &lib, None, None, Some(&sp));
    let mut seen = 0;
    for (scene, _, _) in counts(&d.project) {
        let t = ticks(&plan, &scene);
        for w in t.windows(2) {
            assert!(w[1].time - w[0].time >= 0.1 - 1e-9, "{scene}");
        }
        seen += t.len();
    }
    assert!(seen > 0, "ticks survive the onset rule");
}
