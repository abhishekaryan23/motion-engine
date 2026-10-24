//! Cue planning (0.8): handoff / information / impact / open rules, collisions,
//! determinism and library edge cases. See docs/SOUND_DESIGN.md §3.

use std::path::PathBuf;

use motion_core::audio::{
    arrival_times, information_cap, palette_for, plan_audio, AudioPlan, CueKind, SfxFamily,
    SfxLibrary, SfxSound, MAX_GAIN_DB, MIN_GAIN_DB, SFX_LIBRARY_VERSION, SMALL_PER_BEAT,
};
use motion_core::compiler::resolve_taste;
use motion_core::compiler::taste::{
    resolve, CompositionRhythm, DensityLevel, ResolvedStyleProfile, TemperamentKind,
    TransitionFamily,
};
use motion_core::intent::Energy;
use motion_core::{
    compile, ApproxMeasure, AssetLibrary, CreativeIntent, MotionProject, StyleProfile,
};

const PEAK: f64 = 0.5;

fn sound(id: &str, family: SfxFamily, peak_db: f64) -> SfxSound {
    SfxSound {
        id: id.to_string(),
        family,
        path: format!("sounds/{}/{id}.wav", family.as_str()),
        duration: 1.0,
        onset: 0.4,
        peak: PEAK,
        audible_end: 0.9,
        peak_db,
        lufs: None,
        sha256: "0".repeat(64),
        tags: vec![],
    }
}

fn library_without(skip: &[SfxFamily]) -> SfxLibrary {
    let mut sounds = Vec::new();
    for f in SfxFamily::ALL {
        if skip.contains(&f) {
            continue;
        }
        for (k, peak) in [-6.0, -10.0, -8.0, -12.0, -7.0, -9.0]
            .into_iter()
            .enumerate()
        {
            let tag = (b'a' + k as u8) as char;
            sounds.push(sound(&format!("{}_{tag}", f.as_str()), f, peak));
        }
    }
    sounds.sort_by(|a, b| a.id.cmp(&b.id));
    SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds,
    }
}

fn library() -> SfxLibrary {
    library_without(&[])
}

/// (id, start, duration, lifecycle [enter, evolve, anticipate], motion starts)
type SceneSpec<'a> = (&'a str, f64, f64, [f64; 3], &'a [f64]);

fn project(scenes: &[SceneSpec]) -> MotionProject {
    let mut out = Vec::new();
    let mut end: f64 = 0.0;
    for (id, start, dur, [enter, evolve, anticipate], motions) in scenes {
        end = end.max(start + dur);
        let ms: Vec<String> = motions
            .iter()
            .map(|t| {
                format!(
                    r#"{{"target":"x","op":"move","start":{t},"duration":0.3,"from":[0,0],"to":[1,1]}}"#
                )
            })
            .collect();
        out.push(format!(
            r#"{{"id":"{id}","start_seconds":{start},"duration_seconds":{dur},"layers":[],
               "motions":[{}],
               "lifecycle":{{"enter":{enter},"settle":{s},"read":{s},"evolve":{evolve},
                             "anticipate":{anticipate},"bridge":{dur}}}}}"#,
            ms.join(","),
            s = enter + 0.1
        ));
    }
    let json = format!(
        r##"{{"version":"0.2","project":{{"name":"t","duration_seconds":{end}}},
            "canvas":{{"width":108,"height":192,"fps":30,"background":"#FFFFFF"}},
            "scenes":[{}]}}"##,
        out.join(",")
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn taste(
    kind: TemperamentKind,
    transition: TransitionFamily,
    density: DensityLevel,
    rhythm: CompositionRhythm,
) -> ResolvedStyleProfile {
    let mut t = resolve(&StyleProfile::default());
    t.motion.kind = kind;
    t.transition.family = transition;
    t.density.level = density;
    t.rhythm = rhythm;
    t
}

fn neutral(rhythm: CompositionRhythm) -> ResolvedStyleProfile {
    taste(
        TemperamentKind::Restrained,
        TransitionFamily::Editorial,
        DensityLevel::Sparse,
        rhythm,
    )
}

fn of_kind(plan: &AudioPlan, kind: CueKind) -> Vec<&motion_core::audio::AudioCue> {
    plan.cues.iter().filter(|c| c.kind == kind).collect()
}

fn peak_db_of(lib: &SfxLibrary, id: &str) -> f64 {
    lib.get(id).expect("sound").peak_db
}

fn two_scenes(overlap: f64) -> MotionProject {
    project(&[
        ("a", 0.0, 3.0 + overlap, [0.2, 1.0, 2.5], &[]),
        ("b", 3.0, 3.0, [0.3, 1.0, 2.5], &[]),
    ])
}

// ---------------------------------------------------------------------------

/// (0.24 M5) The cut cue draws from the look's palette (here editorial), peaks at
/// the overlap midpoint, and a cut that does not move gets none.
#[test]
fn handoff_is_a_palette_transition_cue_at_the_midpoint() {
    let lib = library();
    let (_, palette) = palette_for(None, TemperamentKind::Restrained).expect("palette");
    for tf in [
        TransitionFamily::Subtle,
        TransitionFamily::Editorial,
        TransitionFamily::Geometric,
        TransitionFamily::Kinetic,
    ] {
        for (overlap, at) in [(0.0, 3.0), (1.6, 3.8)] {
            let mut t = neutral(CompositionRhythm::HighFrequency);
            t.transition.family = tf;
            let plan = plan_audio(&two_scenes(overlap), &t, &[], &lib, None);
            let h = of_kind(&plan, CueKind::Handoff);
            assert_eq!(h.len(), 1, "{tf:?}");
            assert!(palette.transition.contains(&h[0].family), "{tf:?}");
            assert_eq!(h[0].scene, "b");
            assert!((h[0].time - at).abs() < 1e-9, "{tf:?} time {}", h[0].time);
            assert_eq!(h[0].priority, 2);
            // Loudness-normalised: transition role -24 LUFS-equiv (lufs-less
            // sounds use peak - 18), within the jitter.
            let loud = peak_db_of(&lib, &h[0].sound_id) - 18.0 + h[0].gain_db;
            assert!((loud - -24.0).abs() <= 1.0 + 1e-9, "{tf:?} loudness {loud}");
        }
    }
    // A hard cut with no overlap does not move: no whoosh.
    let mut t = neutral(CompositionRhythm::HighFrequency);
    t.transition.family = TransitionFamily::Hard;
    let plan = plan_audio(&two_scenes(0.0), &t, &[], &lib, None);
    assert!(of_kind(&plan, CueKind::Handoff).is_empty());
}

fn info_project() -> MotionProject {
    project(&[(
        "a",
        0.0,
        6.0,
        [0.2, 1.0, 4.0],
        &[0.4, 1.0, 1.5, 2.0, 2.5, 3.0, 4.2],
    )])
}

#[test]
fn information_family_cap_and_window() {
    let lib = library();
    // (temperament, palette name's small families)
    for kind in [
        TemperamentKind::Restrained,
        TemperamentKind::Editorial,
        TemperamentKind::Precise,
        TemperamentKind::Energetic,
    ] {
        let (_, palette) = palette_for(None, kind).expect("palette");
        for (density, cap) in [
            (DensityLevel::Sparse, 1),
            (DensityLevel::Balanced, 2),
            (DensityLevel::Dense, 3),
        ] {
            assert_eq!(information_cap(density), cap);
            let t = taste(
                kind,
                TransitionFamily::Editorial,
                density,
                CompositionRhythm::HighFrequency,
            );
            let plan = plan_audio(&info_project(), &t, &[], &lib, None);
            let info = of_kind(&plan, CueKind::Information);
            if kind == TemperamentKind::Restrained && density == DensityLevel::Sparse {
                assert!(info.is_empty());
                continue;
            }
            // The opening cue takes one of the beat's two small slots.
            let want = cap.min(SMALL_PER_BEAT - 1);
            assert_eq!(info.len(), want, "{kind:?} {density:?}");
            for c in info {
                assert!(palette.small.contains(&c.family), "{kind:?} {:?}", c.family);
                assert_ne!(
                    c.family,
                    SfxFamily::Paper,
                    "no page-turn rustle on arrivals"
                );
                let loud = peak_db_of(&lib, &c.sound_id) - 18.0 + c.gain_db;
                assert!((loud - -28.0).abs() <= 1.0 + 1e-9, "loudness {loud}");
                assert!(c.time >= 1.0 - 1e-9 && c.time < 4.0, "time {}", c.time);
            }
        }
    }
}

#[test]
fn arrivals_cluster_and_skip_stage_and_ghost() {
    let json = r##"{"version":"0.2","project":{"name":"t","duration_seconds":6.0},
        "canvas":{"width":108,"height":192,"fps":30,"background":"#FFFFFF"},
        "scenes":[{"id":"a","start_seconds":0.0,"duration_seconds":6.0,"layers":[],
          "motions":[
           {"target":"x","op":"move","start":0.5,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"x","op":"move","start":1.0,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"y","op":"move","start":1.05,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"y","op":"move","start":1.15,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"c.stage","op":"move","start":2.0,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"g.ghost","op":"move","start":2.2,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"z","op":"move","start":3.0,"duration":0.3,"from":[0,0],"to":[1,1]},
           {"target":"z","op":"move","start":4.0,"duration":0.3,"from":[0,0],"to":[1,1]}],
          "lifecycle":{"enter":0.2,"settle":0.3,"read":0.5,"evolve":1.0,"anticipate":4.0,"bridge":6.0}}]}"##;
    let p = MotionProject::from_json(json).expect("parse");
    // 1.0/1.05/1.15 chain into one cluster (each within 0.12 of the previous).
    assert_eq!(arrival_times(&p.scenes[0]), vec![1.0, 3.0]);
    let mut no_life = p.scenes[0].clone();
    no_life.lifecycle = None;
    assert!(arrival_times(&no_life).is_empty());
}

/// (0.24 M5) Each beat's lead reveal gets a hero cue at its settle (the lifecycle
/// settle when no layer is named: enter + 0.1 in this fixture); the biggest beat
/// adds the palette's bed.
#[test]
fn hero_cues_and_open_cue() {
    let lib = library();
    let p = project(&[
        ("a", 0.0, 3.0, [0.2, 1.0, 2.5], &[]),
        ("b", 3.0, 3.0, [0.3, 1.0, 2.5], &[]),
        ("c", 6.0, 3.0, [0.3, 1.0, 2.5], &[]),
    ]);
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let hr = CompositionRhythm::HighFrequency;
    for kind in [
        TemperamentKind::Energetic,
        TemperamentKind::Editorial,
        TemperamentKind::Restrained,
        TemperamentKind::Precise,
    ] {
        let (_, palette) = palette_for(None, kind).expect("palette");
        let t = taste(kind, TransitionFamily::Editorial, DensityLevel::Sparse, hr);
        let plan = plan_audio(&p, &t, &energy, &lib, None);
        let heroes: Vec<_> = plan
            .cues
            .iter()
            .filter(|c| c.reason.contains(": hero "))
            .collect();
        assert_eq!(heroes.len(), 3, "{kind:?} one hero per beat");
        for (h, at) in heroes.iter().zip([0.3, 3.4, 6.4]) {
            assert!((h.time - at).abs() < 1e-9, "{kind:?} hero at {}", h.time);
            assert!(palette.hero.contains(&h.family), "{kind:?}");
        }
        // The biggest beat (the last impact) carries the video's beds.
        let beds: Vec<_> = plan
            .cues
            .iter()
            .filter(|c| c.reason.contains("hero bed"))
            .collect();
        assert_eq!(beds.len(), palette.hero_big_with.len(), "{kind:?}");
        for b in beds {
            assert_eq!(b.scene, "c");
            assert!((b.time - 6.4).abs() < 1e-9);
        }
        // Soft opening cue on the first arrival (no motions: S + enter).
        // Here it sits 0.1 s before the first hero (0.3), so the breath rule
        // drops it; a later hero keeps it (see audio_cues_taste).
        assert!(of_kind(&plan, CueKind::Open).is_empty(), "{kind:?}");
    }
}

#[test]
fn collisions_by_priority_then_time_and_beds_exempt() {
    let lib = library();
    // Handoff (prio 2) at 3.0, impact hit (prio 3) at 3.3.
    let p = project(&[
        ("a", 0.0, 3.0, [0.2, 1.0, 2.5], &[]),
        ("b", 3.0, 3.0, [0.3, 1.0, 2.5], &[]),
    ]);
    let energy = [Energy::Building, Energy::Impact];
    let slow = neutral(CompositionRhythm::SlowBreathing);
    let plan = plan_audio(&p, &slow, &energy, &lib, None);
    assert!(
        of_kind(&plan, CueKind::Handoff).is_empty(),
        "lower priority loses"
    );
    assert!(
        plan.cues
            .iter()
            .any(|c| c.kind == CueKind::Impact && c.family == SfxFamily::HitSoft && c.time > 3.0),
        "the hero stays"
    );
    let fast = neutral(CompositionRhythm::HighFrequency);
    let plan = plan_audio(&p, &fast, &energy, &lib, None);
    assert_eq!(of_kind(&plan, CueKind::Handoff).len(), 1);

    // Two information cues 0.3 s apart: equal priority -> earlier kept when the
    // spacing is wider than the gap; both kept for high_frequency.
    let p = project(&[
        ("z", 0.0, 2.0, [0.2, 1.0, 1.5], &[]),
        ("a", 2.0, 4.0, [0.2, 1.0, 3.5], &[1.0, 1.3]),
    ]);
    let mk = |r| {
        taste(
            TemperamentKind::Editorial,
            TransitionFamily::Editorial,
            DensityLevel::Balanced,
            r,
        )
    };
    for (rhythm, count) in [
        (CompositionRhythm::SlowBreathing, 1),
        (CompositionRhythm::MeasuredEditorial, 1),
        (CompositionRhythm::Progressive, 1),
        (CompositionRhythm::Active, 2),
        (CompositionRhythm::HighFrequency, 2),
    ] {
        let plan = plan_audio(&p, &mk(rhythm), &[], &lib, None);
        let info = of_kind(&plan, CueKind::Information);
        assert_eq!(info.len(), count, "{rhythm:?}");
        assert!((info[0].time - 3.0).abs() < 1e-9, "earlier kept");
    }

    // Beds are exempt: the hero hit and its subdrop bed at the same instant coexist.
    let p = project(&[("a", 0.0, 3.0, [0.2, 1.0, 2.5], &[])]);
    let t = neutral(CompositionRhythm::SlowBreathing);
    let plan = plan_audio(&p, &t, &[Energy::Impact], &lib, None);
    assert!(plan.cues.iter().any(|c| c.family == SfxFamily::Subdrop));
    assert!(plan.cues.iter().any(|c| c.family == SfxFamily::HitSoft));
}

fn multi_project() -> MotionProject {
    project(&[
        ("a", 0.0, 3.0, [0.2, 1.0, 2.5], &[1.0, 1.5, 2.0]),
        ("b", 3.0, 3.0, [0.3, 1.0, 2.5], &[1.0, 1.5]),
        ("c", 6.0, 3.0, [0.3, 1.0, 2.5], &[1.0, 1.6, 2.1]),
    ])
}

#[test]
fn deterministic_and_seed_changes_only_sound_ids() {
    let lib = library();
    let p = multi_project();
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let mut t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Kinetic,
        DensityLevel::Dense,
        CompositionRhythm::HighFrequency,
    );
    let a = plan_audio(&p, &t, &energy, &lib, None).to_json_pretty();
    let b = plan_audio(&p, &t, &energy, &lib, None).to_json_pretty();
    assert_eq!(a, b);

    let base = plan_audio(&p, &t, &energy, &lib, None);
    assert!(base.cues.len() >= 7, "{}", base.cues.len());
    let mut sound_sets = std::collections::BTreeSet::new();
    for seed in 0..24u64 {
        t.effective.seed = seed;
        let plan = plan_audio(&p, &t, &energy, &lib, None);
        assert_eq!(plan.cues.len(), base.cues.len());
        for (x, y) in plan.cues.iter().zip(&base.cues) {
            assert_eq!(x.time, y.time);
            assert_eq!(x.family, y.family);
            assert_eq!(x.kind, y.kind);
        }
        sound_sets.insert(
            plan.cues
                .iter()
                .map(|c| c.sound_id.clone())
                .collect::<Vec<_>>(),
        );
    }
    assert!(sound_sets.len() > 1, "seed never changed a sound choice");
}

#[test]
fn missing_family_and_empty_library() {
    let p = multi_project();
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let t = taste(
        TemperamentKind::Editorial,
        TransitionFamily::Hard,
        DensityLevel::Dense,
        CompositionRhythm::HighFrequency,
    );
    let with = plan_audio(&p, &t, &energy, &library(), None);
    assert!(with.cues.iter().any(|c| c.family == SfxFamily::Tick));
    let lib = library_without(&[SfxFamily::Tick]);
    let plan = plan_audio(&p, &t, &energy, &lib, None);
    assert!(plan.cues.iter().all(|c| c.family != SfxFamily::Tick));
    assert!(plan.cues.iter().any(|c| c.family == SfxFamily::HitSoft));

    let empty = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds: vec![],
    };
    let plan = plan_audio(&p, &t, &energy, &empty, None);
    assert!(plan.cues.is_empty());
}

#[test]
fn sorted_bounded_and_gain_clamped() {
    let p = multi_project();
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Kinetic,
        DensityLevel::Dense,
        CompositionRhythm::Active,
    );
    let plan = plan_audio(&p, &t, &energy, &library(), None);
    let dur = p.duration_seconds();
    for w in plan.cues.windows(2) {
        let key = |c: &motion_core::audio::AudioCue| (c.time, -(c.priority as i32));
        let (a, b) = (key(&w[0]), key(&w[1]));
        assert!(
            a.0 < b.0
                || (a.0 == b.0 && (a.1 < b.1 || (a.1 == b.1 && w[0].sound_id <= w[1].sound_id)))
        );
    }
    for c in &plan.cues {
        assert!(c.time >= 0.0 && c.time <= dur);
        assert!(c.gain_db >= MIN_GAIN_DB && c.gain_db <= MAX_GAIN_DB);
        assert_eq!(c.priority, c.kind.priority());
        assert!(c.reason.starts_with(c.kind.as_str()), "{}", c.reason);
    }

    // Extremes clamp.
    let mut lib = library();
    for s in &mut lib.sounds {
        s.peak_db = if s.family == SfxFamily::HitHard {
            40.0
        } else {
            -80.0
        };
    }
    let plan = plan_audio(&p, &t, &energy, &lib, None);
    assert!(plan.cues.iter().all(|c| c.gain_db
        == if c.family == SfxFamily::HitHard {
            MIN_GAIN_DB
        } else {
            MAX_GAIN_DB
        }));

    // Cues beyond the project duration are dropped.
    let mut short = p.clone();
    short.project.duration_seconds = Some(2.0);
    let plan = plan_audio(&short, &t, &energy, &library(), None);
    assert!(plan.cues.iter().all(|c| c.time <= 2.0));
}

#[test]
fn music_bed_is_carried_from_the_music_plan() {
    use motion_core::audio::{MusicPlan, MUSIC_PLAN_VERSION};
    let m = MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: "bed.wav".into(),
        duration: 30.0,
        bpm: 100.0,
        beat_times: vec![],
        downbeat_times: vec![],
        sections: vec![],
        gain_db: -20.0,
        sha256: "0".repeat(64),
        lufs: None,
        lra: None,
    };
    let p = multi_project();
    let t = neutral(CompositionRhythm::Active);
    let with = plan_audio(&p, &t, &[], &library(), Some(&m));
    let without = plan_audio(&p, &t, &[], &library(), None);
    let bed = with.music.expect("bed");
    assert_eq!(
        (bed.gain_db, bed.fade_in, bed.fade_out, bed.duck),
        (-20.0, 0.5, 1.0, true)
    );
    assert_eq!(with.cues, without.cues);
    assert!(without.music.is_none());
}

#[test]
fn impact_hits_move_to_a_downbeat_from_the_handoff_anchor_to_settle() {
    use motion_core::audio::{MusicPlan, MUSIC_PLAN_VERSION};
    use motion_core::intent::Energy;
    let music = |downbeats: Vec<f64>| MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: "bed.wav".into(),
        duration: 30.0,
        bpm: 120.0,
        beat_times: vec![],
        downbeat_times: downbeats,
        sections: vec![],
        gain_db: -18.0,
        sha256: "0".repeat(64),
        lufs: None,
        lra: None,
    };
    // b starts at 3.0, a ends at 3.4: handoff anchor 3.2; b enter 3.3, settle 3.4.
    let p = two_scenes(0.4);
    let t = neutral(CompositionRhythm::Active);
    let energy = [Energy::Building, Energy::Impact];
    let hits = |plan: &AudioPlan| -> Vec<f64> {
        of_kind(plan, CueKind::Impact)
            .iter()
            .filter(|c| c.scene == "b")
            .map(|c| c.time)
            .collect()
    };
    // A downbeat on the anchor: the hit and the subdrop bed land on it.
    let on = plan_audio(
        &p,
        &t,
        &energy,
        &library(),
        Some(&music(vec![1.2, 3.2, 5.2])),
    );
    assert_eq!(hits(&on), vec![3.2, 3.2]);
    assert!(on.cues.iter().any(|c| c.reason.ends_with("(on downbeat)")));
    // No downbeat in [anchor, settle]: the hit stays at the hero's settle.
    let off = plan_audio(&p, &t, &energy, &library(), Some(&music(vec![3.1, 3.5])));
    assert_eq!(hits(&off), vec![3.4, 3.4]);
    // Without music the plan is unchanged from the no-music path.
    let none = plan_audio(&p, &t, &energy, &library(), None);
    assert_eq!(hits(&none), vec![3.4, 3.4]);
}

// ---------------------------------------------------------------------------
// Real compiled projects
// ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn compiled_editorial_demo_across_taste_styles() {
    let intent_text = std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json"))
        .expect("intent");
    let intent = CreativeIntent::from_json(&intent_text).expect("intent parses");
    let energy: Vec<Energy> = intent.beats.iter().map(|b| b.energy).collect();
    let lib = library();
    let mut report = Vec::new();
    for name in ["dark_technical", "playful_print", "warm_editorial"] {
        let style_text =
            std::fs::read_to_string(repo().join(format!("examples/taste/{name}.style.json")))
                .expect("style");
        let style = StyleProfile::from_json(&style_text).expect("style parses");
        let project = compile(
            &intent,
            &style,
            &AssetLibrary::new(repo().join("assets")),
            &ApproxMeasure,
        )
        .expect("compiles");
        let taste = resolve_taste(&intent, &style, None);
        let plan = plan_audio(&project, &taste, &energy, &lib, None);
        let again = plan_audio(&project, &taste, &energy, &lib, None);
        assert_eq!(plan.to_json_pretty(), again.to_json_pretty());
        let dur = project.duration_seconds();
        assert!(plan.cues.windows(2).all(|w| w[0].time <= w[1].time));
        assert!(plan.cues.iter().all(|c| c.time >= 0.0 && c.time <= dur));
        // Beats map to the compiler's beat scenes, not its backdrop scene.
        assert!(plan.cues.iter().all(|c| c.scene != "backdrop"));
        for c in &plan.cues {
            let sc = project
                .scenes
                .iter()
                .find(|s| s.id == c.scene)
                .expect("scene");
            assert!(
                c.time >= sc.start_seconds - 1e-3 && c.time <= sc.end_seconds() + 1e-3,
                "{c:?} outside {}",
                sc.id
            );
        }
        // Every non-handoff cue precedes its scene's ANTICIPATE.
        for c in &plan.cues {
            let sc = project
                .scenes
                .iter()
                .find(|s| s.id == c.scene)
                .expect("scene");
            if let (Some(l), true) = (sc.lifecycle, c.kind != CueKind::Handoff) {
                assert!(c.time <= sc.start_seconds + l.anticipate + 1e-3, "{c:?}");
            }
        }
        assert!(!plan.cues.is_empty(), "{name}: no cues");
        report.push((name, plan.cues.len()));
    }
    eprintln!("editorial_demo cue counts: {report:?}");
}

// ---------------------------------------------------------------------------
// (0.10) speech-aware placement

use motion_core::audio::{plan_audio_with_speech, SPEECH_CLEARANCE};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};

fn speech_with_onsets(onsets: &[f64]) -> SpeechMap {
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".into(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: 9.0,
        provider: "fixture".into(),
        model: "fixture".into(),
        voice: "fixture".into(),
        words: onsets
            .iter()
            .map(|&s| SpeechWord {
                text: "w".into(),
                start: s,
                end: s + 0.2,
                confidence: 1.0,
            })
            .collect(),
        sentences: vec![SpeechSentence {
            beat: 0,
            start: 0.0,
            end: 9.0,
        }],
    }
}

fn clear_of(plan: &AudioPlan, sp: &SpeechMap) -> bool {
    plan.cues.iter().all(|c| {
        sp.words
            .iter()
            .all(|w| (c.time - w.start).abs() >= SPEECH_CLEARANCE - 1e-9)
    })
}

#[test]
fn speech_moves_cues_off_word_onsets_deterministically() {
    let lib = library();
    let p = multi_project();
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Kinetic,
        DensityLevel::Dense,
        CompositionRhythm::HighFrequency,
    );
    let base = plan_audio(&p, &t, &energy, &lib, None);
    // A word onset on top of every planned cue, or 30 ms off it.
    let mut onsets: Vec<f64> = Vec::new();
    for (i, c) in base.cues.iter().enumerate() {
        onsets.push(c.time + [0.0, 0.03, -0.03][i % 3]);
    }
    let sp = speech_with_onsets(&onsets);
    let a = plan_audio_with_speech(&p, &t, &energy, &lib, None, None, Some(&sp));
    let b = plan_audio_with_speech(&p, &t, &energy, &lib, None, None, Some(&sp));
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
    assert!(!a.speech_adjustments.is_empty());
    assert!(clear_of(&a, &sp), "{:?}", a.cues);
    for adj in &a.speech_adjustments {
        if let Some(to) = adj.to {
            assert!((0.0..=9.0).contains(&to));
            assert!((to - adj.from).abs() < 1.0, "{adj:?}");
        }
    }
    assert_eq!(
        a.cues.len()
            + a.speech_adjustments
                .iter()
                .filter(|x| x.to.is_none())
                .count(),
        base.cues.len()
    );
    // Transient spacing survives the moves.
    let mut times: Vec<f64> = a
        .cues
        .iter()
        .filter(|c| !c.family.is_bed())
        .map(|c| c.time)
        .collect();
    times.sort_by(|x, y| x.total_cmp(y));
    for w in times.windows(2) {
        assert!(w[1] - w[0] >= a.min_spacing - 1e-9, "{times:?}");
    }
}

#[test]
fn speech_none_is_byte_identical_and_unrelated_speech_changes_nothing() {
    let lib = library();
    let p = multi_project();
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Kinetic,
        DensityLevel::Dense,
        CompositionRhythm::HighFrequency,
    );
    let old = plan_audio(&p, &t, &energy, &lib, None).to_json_pretty();
    let none = plan_audio_with_speech(&p, &t, &energy, &lib, None, None, None).to_json_pretty();
    assert_eq!(old, none);
    assert!(!old.contains("speech_adjustments"));
    // A word nowhere near any cue: unchanged.
    let sp = speech_with_onsets(&[8.95]);
    let far = plan_audio_with_speech(&p, &t, &energy, &lib, None, None, Some(&sp));
    assert!(far.speech_adjustments.is_empty());
    // (0.24 M7) A voice-over lowers the levels (gain_db) but moves nothing.
    let key = |p: &AudioPlan| {
        p.cues
            .iter()
            .map(|c| (c.time.to_bits(), c.sound_id.clone(), c.kind, c.family))
            .collect::<Vec<_>>()
    };
    assert_eq!(key(&far), key(&plan_audio(&p, &t, &energy, &lib, None)));
}

#[test]
fn speech_drops_a_cue_that_has_nowhere_to_go() {
    // One short scene whose whole window is covered by onsets every 100 ms.
    let lib = library();
    let p = project(&[("a", 0.0, 1.0, [0.2, 0.3, 0.5], &[])]);
    let t = neutral(CompositionRhythm::MeasuredEditorial);
    let base = plan_audio(&p, &t, &[Energy::Impact], &lib, None);
    assert!(!base.cues.is_empty());
    let onsets: Vec<f64> = (0..11).map(|i| f64::from(i) * 0.1).collect();
    let sp = speech_with_onsets(&onsets);
    let plan = plan_audio_with_speech(&p, &t, &[Energy::Impact], &lib, None, None, Some(&sp));
    assert!(plan.cues.is_empty(), "{:?}", plan.cues);
    assert_eq!(plan.speech_adjustments.len(), base.cues.len());
    assert!(plan.speech_adjustments.iter().all(|a| a.to.is_none()));
}

#[test]
fn speech_accent_lands_just_before_the_punch_word() {
    // (0.10 Q) The beat's first number gets one accent cue peaking
    // ACCENT_LEAD before the word (synchronised, not masking it).
    let lib = library();
    let p = multi_project();
    let energy = [Energy::Building, Energy::Building, Energy::Building];
    let t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Kinetic,
        DensityLevel::Balanced,
        CompositionRhythm::HighFrequency,
    );
    let mut sp = speech_with_onsets(&[1.2, 1.6, 2.4]);
    sp.words[1].text = "50%".into();
    let plan = plan_audio_with_speech(&p, &t, &energy, &lib, None, None, Some(&sp));
    let accent: Vec<_> = plan
        .cues
        .iter()
        .filter(|c| c.reason.starts_with("accent"))
        .collect();
    assert_eq!(accent.len(), 1, "{:#?}", plan.cues);
    assert!(accent[0].reason.contains("50%"));
    assert!((accent[0].time - (1.6 - motion_core::audio::ACCENT_LEAD)).abs() < 0.02);
    assert!(clear_of(&plan, &sp));
}
