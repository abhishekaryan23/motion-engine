//! (0.24 M5) Cue taste rules: hero per beat at the lead reveal's settle, palette
//! by look, loudness-normalised gains, variety, breath, open/close, transition
//! thinning, determinism. See docs/SOUND_DESIGN.md §3.

use std::path::PathBuf;

use motion_core::audio::{
    hero_land, palette_file, palette_for, plan_audio, AudioCue, AudioPlan, CueKind, SfxFamily,
    SfxLibrary, SfxSound, BREATH, FAMILY_SHARE, SFX_LIBRARY_VERSION, SHARE_MIN_CUES,
    SMALL_PER_BEAT, VARIETY_WINDOW,
};
use motion_core::compiler::art_direction::{ArtRecord, Look};
use motion_core::compiler::resolve_taste;
use motion_core::compiler::taste::{
    resolve, CompositionRhythm, ResolvedStyleProfile, TemperamentKind,
};
use motion_core::intent::Energy;
use motion_core::{
    compile, ApproxMeasure, AssetLibrary, CreativeIntent, MotionProject, StyleProfile,
};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Six sounds per family with realistic spread: some with a measured LUFS, some
/// gated out (`None`), peaks from -20 to +3 dBFS.
fn library() -> SfxLibrary {
    let mut sounds = Vec::new();
    for (fi, f) in SfxFamily::ALL.into_iter().enumerate() {
        for k in 0..6usize {
            let peak_db = -20.0 + ((fi * 7 + k * 5) % 24) as f64;
            let lufs = (k % 3 != 2).then_some(peak_db - 14.0 - (k as f64));
            sounds.push(SfxSound {
                id: format!("{}_{k:02}", f.as_str()),
                family: f,
                path: format!("sounds/{}/{}_{k:02}.wav", f.as_str(), f.as_str()),
                duration: 1.0,
                onset: 0.1,
                peak: 0.3,
                audible_end: 0.9,
                peak_db,
                lufs,
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

fn loudness(lib: &SfxLibrary, c: &AudioCue) -> f64 {
    let s = lib.get(&c.sound_id).expect("sound");
    s.lufs.unwrap_or(s.peak_db - 18.0) + c.gain_db
}

fn role_of(c: &AudioCue) -> &'static str {
    if c.reason.starts_with("count ") {
        "count"
    } else if c.reason.contains(": hero ") {
        "hero"
    } else if c.reason.contains(": transition ") {
        "transition"
    } else {
        "small"
    }
}

struct Demo {
    project: MotionProject,
    taste: ResolvedStyleProfile,
    energy: Vec<Energy>,
}

fn demo(style_file: &str) -> Demo {
    let intent = CreativeIntent::from_json(
        &std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json")).expect("i"),
    )
    .expect("intent");
    let style =
        StyleProfile::from_json(&std::fs::read_to_string(repo().join(style_file)).expect("style"))
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

fn with_look(mut p: MotionProject, look: Look) -> MotionProject {
    p.project.art = Some(ArtRecord {
        look: look.name().into(),
        palette: 0,
        plate: None,
        families: vec![],
        sfx: "soft".into(),
        focal: Default::default(),
        reveals: Default::default(),
    });
    p
}

const STYLES: [&str; 4] = [
    "examples/editorial_demo.style.json",
    "examples/taste/dark_technical.style.json",
    "examples/taste/playful_print.style.json",
    "examples/taste/warm_editorial.style.json",
];

fn plan_of(d: &Demo, lib: &SfxLibrary) -> AudioPlan {
    plan_audio(&d.project, &d.taste, &d.energy, lib, None)
}

#[test]
fn palettes_json_parses_and_covers_every_look_and_temperament() {
    let file = palette_file().expect("assets/sfx/palettes.json parses");
    assert_eq!(file.version, "0.1");
    for l in Look::ALL {
        let (name, p) = palette_for(Some(l.name()), TemperamentKind::Restrained).expect("p");
        assert!(p.looks.iter().any(|x| x == l.name()), "{name} lacks {l:?}");
    }
    for k in [
        TemperamentKind::Restrained,
        TemperamentKind::Editorial,
        TemperamentKind::Precise,
        TemperamentKind::Energetic,
    ] {
        assert!(palette_for(None, k).is_some());
    }
    // `click` is the tech palette's choice only.
    for (name, p) in &file.palettes {
        let uses_click = [
            &p.open,
            &p.hero,
            &p.transition,
            &p.small,
            &p.close,
            &p.riser,
        ]
        .iter()
        .any(|v| v.contains(&SfxFamily::Click));
        assert_eq!(uses_click, name == "tech", "{name}");
    }
}

#[test]
fn the_hero_lands_on_the_lead_reveals_settle() {
    let lib = library();
    let d = demo(STYLES[0]);
    let plan = plan_of(&d, &lib);
    for scene in d.project.scenes.iter().filter(|s| s.lifecycle.is_some()) {
        let heroes: Vec<&AudioCue> = plan
            .cues
            .iter()
            .filter(|c| c.scene == scene.id && role_of(c) == "hero")
            .collect();
        assert_eq!(heroes.len(), 1, "{}: exactly one hero", scene.id);
        let want = hero_land(scene, None);
        assert!((heroes[0].time - want).abs() < 1e-3, "{}", scene.id);
        // It is the end of a real motion in the scene, never the start of one.
        assert!(
            scene.motions.iter().any(|m| {
                (scene.start_seconds + m.start + m.duration - heroes[0].time).abs() < 2e-3
            }),
            "{}: hero {} is not a motion end",
            scene.id,
            heroes[0].time
        );
    }
}

#[test]
fn hero_land_prefers_the_focal_layer_then_shared_number_then_hero() {
    let json = r##"{"version":"0.2","project":{"name":"t","duration_seconds":5.0},
        "canvas":{"width":108,"height":192,"fps":30,"background":"#FFFFFF"},
        "scenes":[{"id":"a","start_seconds":2.0,"duration_seconds":5.0,"layers":[],
          "motions":[
           {"target":"b.kicker","op":"move","start":0.2,"duration":0.5,"from":[0,0],"to":[1,1]},
           {"target":"b.hero.1","op":"move","start":0.6,"duration":0.7,"from":[0,0],"to":[1,1]},
           {"target":"shared.9","op":"move","start":1.0,"duration":0.8,"from":[0,0],"to":[1,1]},
           {"target":"b.obj","op":"move","start":1.5,"duration":0.9,"from":[0,0],"to":[1,1]}],
          "lifecycle":{"enter":0.2,"settle":0.5,"read":0.8,"evolve":3.0,"anticipate":4.0,"bridge":5.0}}]}"##;
    let p = MotionProject::from_json(json).expect("parse");
    let s = &p.scenes[0];
    assert!(
        (hero_land(s, Some("b.obj")) - 4.4).abs() < 1e-9,
        "focal wins"
    );
    assert!((hero_land(s, None) - 3.8).abs() < 1e-9, "shared number");
    let mut no_shared = s.clone();
    no_shared.motions.remove(2);
    assert!((hero_land(&no_shared, None) - 3.3).abs() < 1e-9, ".hero");
    let mut bare = no_shared.clone();
    bare.motions.clear();
    assert!(
        (hero_land(&bare, None) - 2.5).abs() < 1e-9,
        "settle fallback"
    );
}

#[test]
fn the_palette_follows_the_look_and_click_is_tech_only() {
    let lib = library();
    let d = demo(STYLES[0]);
    for look in Look::ALL {
        let project = with_look(d.project.clone(), look);
        let (name, p) = palette_for(Some(look.name()), d.taste.motion.kind).expect("p");
        let allowed: Vec<SfxFamily> = [&p.open, &p.hero, &p.hero_big_with, &p.riser]
            .into_iter()
            .chain([&p.transition, &p.small, &p.close, &p.count, &p.count_lock])
            .flatten()
            .copied()
            .collect();
        let plan = plan_audio(&project, &d.taste, &d.energy, &lib, None);
        assert!(!plan.cues.is_empty());
        for c in &plan.cues {
            assert!(allowed.contains(&c.family), "{look:?}/{name}: {c:?}");
            assert!(c.reason.contains(&format!("({name})")), "{}", c.reason);
            assert!(
                c.family != SfxFamily::Click || name == "tech",
                "{look:?}: click outside tech"
            );
        }
    }
    // Without art direction the temperament picks it; no click anywhere on the
    // editorial demo.
    for style in STYLES {
        let d = demo(style);
        let plan = plan_of(&d, &lib);
        let (name, _) = palette_for(None, d.taste.motion.kind).expect("p");
        assert!(
            plan.cues
                .iter()
                .all(|c| c.family != SfxFamily::Click || name == "tech"),
            "{style}"
        );
    }
}

#[test]
fn gains_normalise_loudness_per_role() {
    let lib = library();
    let file = palette_file().expect("palettes");
    for style in STYLES {
        let d = demo(style);
        let plan = plan_of(&d, &lib);
        for role in ["hero", "transition", "small"] {
            let loud: Vec<f64> = plan
                .cues
                .iter()
                .filter(|c| role_of(c) == role)
                .map(|c| loudness(&lib, c))
                .collect();
            if loud.len() < 2 {
                continue;
            }
            let (lo, hi) = loud
                .iter()
                .fold((f64::MAX, f64::MIN), |(l, h), &x| (l.min(x), h.max(x)));
            assert!(hi - lo <= 2.0 + 1e-9, "{style} {role}: spread {}", hi - lo);
            let target = match role {
                "hero" => file.role_lufs.hero,
                "transition" => file.role_lufs.transition,
                _ => file.role_lufs.small,
            };
            assert!(
                loud.iter().all(|l| (l - target).abs() <= 1.0 + 1e-9),
                "{style} {role}: {loud:?} vs {target}"
            );
        }
    }
}

#[test]
fn no_cue_is_louder_than_its_beats_hero() {
    let lib = library();
    for style in STYLES {
        let d = demo(style);
        let plan = plan_of(&d, &lib);
        for scene in d.project.scenes.iter().filter(|s| s.lifecycle.is_some()) {
            let in_beat: Vec<&AudioCue> =
                plan.cues.iter().filter(|c| c.scene == scene.id).collect();
            let Some(hero) = in_beat.iter().find(|c| role_of(c) == "hero") else {
                continue;
            };
            let hl = loudness(&lib, hero);
            for c in &in_beat {
                assert!(
                    loudness(&lib, c) <= hl + 1e-9,
                    "{style} {}: {c:?}",
                    scene.id
                );
            }
            let smalls = in_beat
                .iter()
                .filter(|c| c.kind == CueKind::Information || c.kind == CueKind::Open)
                .count();
            assert!(
                smalls <= SMALL_PER_BEAT,
                "{style} {}: {smalls} small",
                scene.id
            );
        }
    }
}

fn many_beats() -> (MotionProject, Vec<Energy>) {
    let mut scenes = Vec::new();
    let mut energy = Vec::new();
    for i in 0..8usize {
        let start = i as f64 * 3.0;
        scenes.push(format!(
            r#"{{"id":"s{i}","start_seconds":{start},"duration_seconds":3.4,"layers":[],
              "motions":[
                {{"target":"s{i}.kicker","op":"move","start":0.3,"duration":0.4,"from":[0,0],"to":[1,1]}},
                {{"target":"s{i}.hero.1","op":"move","start":0.5,"duration":0.9,"from":[0,0],"to":[1,1]}},
                {{"target":"s{i}.label","op":"move","start":1.9,"duration":0.3,"from":[0,0],"to":[1,1]}},
                {{"target":"s{i}.note","op":"move","start":2.3,"duration":0.3,"from":[0,0],"to":[1,1]}}],
              "lifecycle":{{"enter":0.3,"settle":1.4,"read":1.6,"evolve":1.8,"anticipate":2.8,"bridge":3.2}}}}"#
        ));
        energy.push(if i == 5 {
            Energy::Impact
        } else {
            Energy::Building
        });
    }
    let json = format!(
        r##"{{"version":"0.2","project":{{"name":"t","duration_seconds":24.4}},
            "canvas":{{"width":108,"height":192,"fps":30,"background":"#FFFFFF"}},
            "scenes":[{}]}}"##,
        scenes.join(",")
    );
    (MotionProject::from_json(&json).expect("parse"), energy)
}

fn taste_of(kind: TemperamentKind, seed: u64) -> ResolvedStyleProfile {
    let mut t = resolve(&StyleProfile::default());
    t.motion.kind = kind;
    t.rhythm = CompositionRhythm::Active;
    t.effective.seed = seed;
    t
}

#[test]
fn variety_no_repeats_within_four_and_no_family_over_forty_percent() {
    let lib = library();
    let (p, energy) = many_beats();
    for look in Look::ALL {
        for seed in 0..8u64 {
            let project = with_look(p.clone(), look);
            let t = taste_of(TemperamentKind::Editorial, seed);
            let mut plan = plan_audio(&project, &t, &energy, &lib, None);
            // (0.24 M7) A count's train and lock are outside the caps.
            plan.cues.retain(|c| !c.reason.starts_with("count "));
            let n = plan.cues.len();
            assert!(n >= SHARE_MIN_CUES, "{look:?} {seed}: {n} cues");
            for (i, c) in plan.cues.iter().enumerate() {
                for o in plan.cues.iter().take(i).rev().take(VARIETY_WINDOW) {
                    assert_ne!(c.sound_id, o.sound_id, "{look:?} seed {seed}");
                }
            }
            for f in SfxFamily::ALL {
                let k = plan.cues.iter().filter(|c| c.family == f).count();
                assert!(
                    k as f64 <= FAMILY_SHARE * n as f64 + 1e-9,
                    "{look:?} seed {seed}: {k} of {n} are {f:?}"
                );
            }
        }
    }
    for style in STYLES {
        let d = demo(style);
        let mut plan = plan_of(&d, &lib);
        plan.cues.retain(|c| !c.reason.starts_with("count "));
        for (i, c) in plan.cues.iter().enumerate() {
            for o in plan.cues.iter().take(i).rev().take(VARIETY_WINDOW) {
                assert_ne!(c.sound_id, o.sound_id, "{style}");
            }
        }
    }
}

#[test]
fn breath_before_every_hero() {
    let lib = library();
    let (p, energy) = many_beats();
    for kind in [TemperamentKind::Editorial, TemperamentKind::Precise] {
        let plan = plan_audio(&p, &taste_of(kind, 3), &energy, &lib, None);
        let heroes: Vec<f64> = plan
            .cues
            .iter()
            .filter(|c| role_of(c) == "hero")
            .map(|c| c.time)
            .collect();
        assert!(!heroes.is_empty());
        for c in plan.cues.iter().filter(|c| !c.family.is_bed()) {
            for &h in &heroes {
                if role_of(c) != "hero" && c.time < h {
                    assert!(h - c.time >= BREATH - 1e-9, "{kind:?}: {c:?} before {h}");
                }
            }
        }
    }
    for style in STYLES {
        let d = demo(style);
        let plan = plan_of(&d, &lib);
        for h in plan.cues.iter().filter(|c| role_of(c) == "hero") {
            for c in plan.cues.iter().filter(|c| !c.family.is_bed()) {
                if c.time < h.time {
                    assert!(h.time - c.time >= BREATH - 1e-9, "{style}: {c:?}");
                }
            }
        }
    }
}

#[test]
fn at_most_one_whoosh_per_two_cuts() {
    let lib = library();
    let (p, energy) = many_beats();
    // 8 beats overlapping by 0.4 s: 7 cuts, all moving.
    let plan = plan_audio(
        &p,
        &taste_of(TemperamentKind::Editorial, 1),
        &energy,
        &lib,
        None,
    );
    let cuts = 7usize;
    let whooshes = plan
        .cues
        .iter()
        .filter(|c| c.kind == CueKind::Handoff)
        .count();
    assert!(whooshes >= 1, "some cut moves");
    assert!(
        whooshes <= cuts.div_ceil(2),
        "{whooshes} whooshes for {cuts} cuts"
    );
}

#[test]
fn open_close_and_riser_into_the_biggest_hero() {
    let lib = library();
    let (p, mut energy) = many_beats();
    // Open: a soft cue on the first arrival (kicker at 0.3).
    let plan = plan_audio(
        &p,
        &taste_of(TemperamentKind::Editorial, 2),
        &energy,
        &lib,
        None,
    );
    let open: Vec<&AudioCue> = plan
        .cues
        .iter()
        .filter(|c| c.kind == CueKind::Open)
        .collect();
    assert_eq!(open.len(), 1);
    assert!((open[0].time - 0.3).abs() < 1e-9);
    // Close: the biggest beat is the first; the last beat's hero gets the tail bed.
    energy = vec![Energy::Calm; 8];
    energy[0] = Energy::Impact;
    let plan = plan_audio(
        &p,
        &taste_of(TemperamentKind::Editorial, 2),
        &energy,
        &lib,
        None,
    );
    let last = plan
        .cues
        .iter()
        .filter(|c| c.scene == "s7" && c.family == SfxFamily::Subdrop)
        .count();
    assert_eq!(last, 1, "tail bed under the last hero: {:#?}", plan.cues);
    // Cinematic: riser peaks just before the biggest hero, then the hit.
    let project = with_look(p.clone(), Look::Cinematic3d);
    let plan = plan_audio(
        &project,
        &taste_of(TemperamentKind::Editorial, 2),
        &energy,
        &lib,
        None,
    );
    let hero = plan
        .cues
        .iter()
        .find(|c| role_of(c) == "hero" && c.scene == "s0")
        .expect("hero");
    let riser = plan
        .cues
        .iter()
        .find(|c| c.family == SfxFamily::Riser && c.scene == "s0")
        .expect("riser");
    assert!(riser.time < hero.time && hero.time - riser.time < 0.05);
}

#[test]
fn deterministic_and_the_seed_only_changes_sounds() {
    let lib = library();
    let (p, energy) = many_beats();
    let a = plan_audio(
        &p,
        &taste_of(TemperamentKind::Editorial, 5),
        &energy,
        &lib,
        None,
    );
    let b = plan_audio(
        &p,
        &taste_of(TemperamentKind::Editorial, 5),
        &energy,
        &lib,
        None,
    );
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
    let c = plan_audio(
        &p,
        &taste_of(TemperamentKind::Editorial, 6),
        &energy,
        &lib,
        None,
    );
    assert_eq!(a.cues.len(), c.cues.len());
    for (x, y) in a.cues.iter().zip(&c.cues) {
        assert_eq!((x.time, x.family, x.kind), (y.time, y.family, y.kind));
    }
    assert_ne!(
        a.cues.iter().map(|c| &c.sound_id).collect::<Vec<_>>(),
        c.cues.iter().map(|c| &c.sound_id).collect::<Vec<_>>()
    );
}
