//! Cue planning (0.8): handoff / information / impact / open rules, collisions,
//! determinism and library edge cases. See docs/SOUND_DESIGN.md §3.

use std::path::PathBuf;

use motion_core::audio::{
    arrival_times, handoff_cue, information_cap, plan_audio, AudioPlan, CueKind, SfxFamily,
    SfxLibrary, SfxSound, MAX_GAIN_DB, MIN_GAIN_DB, SFX_LIBRARY_VERSION,
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
        sounds.push(sound(&format!("{}_a", f.as_str()), f, -6.0));
        sounds.push(sound(&format!("{}_b", f.as_str()), f, -10.0));
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

#[test]
fn handoff_family_per_transition_at_the_midpoint() {
    let lib = library();
    let expect = [
        (TransitionFamily::Subtle, SfxFamily::WhooshSoft, -24.0),
        (TransitionFamily::Editorial, SfxFamily::WhooshSoft, -14.0),
        (TransitionFamily::Geometric, SfxFamily::Swipe, -15.0),
        (TransitionFamily::Kinetic, SfxFamily::WhooshHard, -14.0),
        (TransitionFamily::Hard, SfxFamily::Click, -17.0),
    ];
    for (tf, family, target) in expect {
        assert_eq!(handoff_cue(tf), Some((family, target)));
        for (overlap, at) in [(0.0, 3.0), (0.4, 3.2)] {
            let mut t = neutral(CompositionRhythm::HighFrequency);
            t.transition.family = tf;
            let plan = plan_audio(&two_scenes(overlap), &t, &[], &lib, None);
            let h = of_kind(&plan, CueKind::Handoff);
            assert_eq!(h.len(), 1, "{tf:?}");
            assert_eq!(h[0].family, family);
            assert_eq!(h[0].scene, "b");
            assert!((h[0].time - at).abs() < 1e-9, "{tf:?} time {}", h[0].time);
            let want = ((target - peak_db_of(&lib, &h[0].sound_id)) * 10.0).round() / 10.0;
            assert!((h[0].gain_db - want).abs() < 1e-9, "{tf:?} gain");
            assert_eq!(h[0].priority, 2);
        }
    }
    // Subtle really is -24 dBFS: peak_db -6 or -10 -> gain -18 or -14.
    let mut t = neutral(CompositionRhythm::HighFrequency);
    t.transition.family = TransitionFamily::Subtle;
    let plan = plan_audio(&two_scenes(0.0), &t, &[], &lib, None);
    assert!([-18.0, -14.0].contains(&plan.cues[0].gain_db));
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
    let fams = [
        (TemperamentKind::Restrained, SfxFamily::Tick, -24.0),
        (TemperamentKind::Editorial, SfxFamily::Click, -17.0),
        (TemperamentKind::Precise, SfxFamily::Tick, -18.0),
        (TemperamentKind::Energetic, SfxFamily::Pop, -16.0),
    ];
    for (kind, family, target) in fams {
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
            assert_eq!(info.len(), cap, "{kind:?} {density:?}");
            for c in info {
                assert_eq!(c.family, family);
                assert_ne!(
                    c.family,
                    SfxFamily::Paper,
                    "no page-turn rustle on arrivals"
                );
                let expected = ((target - peak_db_of(&lib, &c.sound_id)) * 10.0).round() / 10.0;
                assert!((c.gain_db - expected.clamp(-40.0, 18.0)).abs() < 1e-9);
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

#[test]
fn impact_cues_and_open_cue() {
    let lib = library();
    let p = project(&[
        ("a", 0.0, 3.0, [0.2, 1.0, 2.5], &[]),
        ("b", 3.0, 3.0, [0.3, 1.0, 2.5], &[]),
        ("c", 6.0, 3.0, [0.3, 1.0, 2.5], &[]),
    ]);
    let energy = [Energy::Building, Energy::Impact, Energy::Impact];
    let hr = CompositionRhythm::HighFrequency;

    // (temperament, transition, expected hit family)
    let cases = [
        (
            TemperamentKind::Energetic,
            TransitionFamily::Editorial,
            SfxFamily::HitHard,
        ),
        (
            TemperamentKind::Editorial,
            TransitionFamily::Kinetic,
            SfxFamily::HitHard,
        ),
        (
            TemperamentKind::Editorial,
            TransitionFamily::Editorial,
            SfxFamily::HitSoft,
        ),
        (
            TemperamentKind::Restrained,
            TransitionFamily::Subtle,
            SfxFamily::HitSoft,
        ),
    ];
    for (kind, tf, hit) in cases {
        let t = taste(kind, tf, DensityLevel::Sparse, hr);
        let plan = plan_audio(&p, &t, &energy, &lib, None);
        let impacts = of_kind(&plan, CueKind::Impact);
        let hits: Vec<_> = impacts.iter().filter(|c| c.family == hit).collect();
        assert_eq!(hits.len(), 2, "{kind:?} {tf:?}");
        assert!((hits[0].time - 3.3).abs() < 1e-9);
        assert!((hits[1].time - 6.3).abs() < 1e-9);
        let subs: Vec<_> = impacts
            .iter()
            .filter(|c| c.family == SfxFamily::Subdrop)
            .collect();
        assert_eq!(subs.len(), 1, "one impact subdrop overall");
        assert_eq!(subs[0].scene, "b");
        assert!((subs[0].time - 3.3).abs() < 1e-9);
    }

    // Open cue by temperament (scene a is not an impact scene).
    for (kind, want) in [
        (TemperamentKind::Energetic, Some(SfxFamily::Subdrop)),
        (TemperamentKind::Precise, Some(SfxFamily::Riser)),
        (TemperamentKind::Editorial, None),
        (TemperamentKind::Restrained, None),
    ] {
        let t = taste(kind, TransitionFamily::Editorial, DensityLevel::Sparse, hr);
        let plan = plan_audio(&p, &t, &energy, &lib, None);
        let open = of_kind(&plan, CueKind::Open);
        assert_eq!(open.first().map(|c| c.family), want, "{kind:?}");
        if let Some(o) = open.first() {
            assert!((o.time - 0.2).abs() < 1e-9);
            assert_eq!(o.scene, "a");
        }
    }

    // Energetic piece that opens on an impact scene: the open subdrop already
    // covers the bed, so no second subdrop within 1 s.
    let t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Editorial,
        DensityLevel::Sparse,
        hr,
    );
    let plan = plan_audio(
        &p,
        &t,
        &[Energy::Impact, Energy::Calm, Energy::Calm],
        &lib,
        None,
    );
    let subs: Vec<_> = plan
        .cues
        .iter()
        .filter(|c| c.family == SfxFamily::Subdrop)
        .collect();
    assert_eq!(subs.len(), 1);
    assert_eq!(subs[0].kind, CueKind::Open);
    // the hit itself is still planned
    assert!(plan
        .cues
        .iter()
        .any(|c| c.kind == CueKind::Impact && c.family == SfxFamily::HitHard));
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
    assert_eq!(
        plan.cues
            .iter()
            .filter(|c| c.kind == CueKind::Impact && c.family == SfxFamily::HitSoft)
            .count(),
        1
    );
    let fast = neutral(CompositionRhythm::HighFrequency);
    let plan = plan_audio(&p, &fast, &energy, &lib, None);
    assert_eq!(of_kind(&plan, CueKind::Handoff).len(), 1);

    // Two information cues 0.3 s apart: equal priority -> earlier kept when the
    // spacing is wider than the gap; both kept for high_frequency.
    let p = project(&[("a", 0.0, 4.0, [0.2, 1.0, 3.5], &[1.0, 1.3])]);
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
        assert!((info[0].time - 1.0).abs() < 1e-9, "earlier kept");
    }

    // Beds are exempt: open subdrop and the hit at the same instant coexist.
    let p = project(&[("a", 0.0, 3.0, [0.2, 1.0, 2.5], &[])]);
    let t = taste(
        TemperamentKind::Energetic,
        TransitionFamily::Editorial,
        DensityLevel::Sparse,
        CompositionRhythm::SlowBreathing,
    );
    let plan = plan_audio(&p, &t, &[Energy::Impact], &lib, None);
    assert!(plan.cues.iter().any(|c| c.family == SfxFamily::Subdrop));
    assert!(plan.cues.iter().any(|c| c.family == SfxFamily::HitHard));
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
    assert!(base.cues.len() >= 8);
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
    // Handoffs and information are both click here.
    let with = plan_audio(&p, &t, &energy, &library(), None);
    assert!(with.cues.iter().any(|c| c.family == SfxFamily::Click));
    let lib = library_without(&[SfxFamily::Click]);
    let plan = plan_audio(&p, &t, &energy, &lib, None);
    assert!(plan.cues.iter().all(|c| c.family != SfxFamily::Click));
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
    // No downbeat in [anchor, settle]: the hit stays at S + enter.
    let off = plan_audio(&p, &t, &energy, &library(), Some(&music(vec![3.1, 3.5])));
    assert_eq!(hits(&off), vec![3.3, 3.3]);
    // Without music the plan is unchanged from the no-music path.
    let none = plan_audio(&p, &t, &energy, &library(), None);
    assert_eq!(hits(&none), vec![3.3, 3.3]);
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
    assert_eq!(far.to_json_pretty(), old);
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
