//! MotionProgram: round trips, validation, deterministic lowering, and the
//! checked-in schema (`schema/motion_program.json`; regenerate with
//! `MOTION_UPDATE_SCHEMA=1 cargo test -p motion-program --test program`).

use std::path::PathBuf;

use motion_program::program::{Aspect, MotionProgram};
use motion_program::{compile, lower, validate, Typesetter};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn demo_text() -> String {
    std::fs::read_to_string(root().join("examples/v2/one_shape.program.json")).expect("demo")
}

fn demo() -> MotionProgram {
    MotionProgram::from_json(&demo_text()).expect("demo parses")
}

#[test]
fn demo_round_trips_through_json() {
    let p = demo();
    let again = MotionProgram::from_json(&p.to_json()).expect("re-parse");
    assert_eq!(p, again);
    // And the value view is stable too (no lossy defaults).
    let v1: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
    let v2: serde_json::Value = serde_json::from_str(&again.to_json()).unwrap();
    assert_eq!(v1, v2);
}

#[test]
fn demo_validates_and_lowers_to_a_valid_scene_in_every_aspect() {
    let mut p = demo();
    validate(&p).expect("demo is valid");
    for aspect in [Aspect::Landscape, Aspect::Portrait, Aspect::Square] {
        p.meta.aspect = aspect;
        let scene = lower(&p, Some("../assets".into()), &Typesetter::approx());
        assert_eq!((scene.canvas.width, scene.canvas.height), aspect.size());
        // Asset root is relative to the scene file; validate from a dir
        // one level below the repo root so `../assets` resolves.
        motion_core::validate(&scene, Some(&root().join("examples")))
            .unwrap_or_else(|e| panic!("{aspect:?}: {e}"));
        // The scene survives its own JSON round trip.
        let back = motion_core::MotionProject::from_json(&scene.to_json_pretty()).unwrap();
        assert_eq!(back, scene);
    }
}

#[test]
fn lowering_is_deterministic() {
    let a = compile(&demo_text(), &[], None, &Typesetter::approx())
        .unwrap()
        .to_json_pretty();
    let b = compile(&demo_text(), &[], None, &Typesetter::approx())
        .unwrap()
        .to_json_pretty();
    assert_eq!(a, b);
}

fn issues(json: &str) -> String {
    let p = MotionProgram::from_json(json).expect("parses");
    validate(&p)
        .map(|_| String::new())
        .unwrap_or_else(|e| e.to_string())
}

fn program(components: &str, extra: &str) -> String {
    format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 4}},
        "palette": {{"background": "#F1ECE2", "ink": "#151413", "accent": "#FF5A1F", "muted": "#8E877B"}}
        {extra}, "components": [{components}]}}"##
    )
}

#[test]
fn validator_names_what_is_wrong() {
    // Gradient backdrops are banned by default.
    let e = issues(&program(
        r#"{"id": "g", "kind": "backdrop", "preset": "glow"}"#,
        "",
    ));
    assert!(
        e.contains("components[0].preset") && e.contains("banned"),
        "{e}"
    );
    assert_eq!(
        issues(&program(
            r#"{"id": "g", "kind": "backdrop", "preset": "glow"}"#,
            r#", "allow": ["gradient"]"#
        )),
        ""
    );

    // Two verbs on one channel at once.
    let e = issues(&program(
        r#"{"id": "s", "kind": "shape", "shape": "circle", "timeline": [
            {"at": 0, "do": "enter"}, {"at": 0.2, "do": "hit"}]}"#,
        "",
    ));
    assert!(e.contains("overlaps") && e.contains("Scale"), "{e}");

    // Verb that does not suit the component.
    let e = issues(&program(
        r#"{"id": "t", "kind": "title", "text": "hi", "timeline": [
            {"at": 0, "do": "morph", "to": "star"}]}"#,
        "",
    ));
    assert!(
        e.contains("morph, bounce, orbit, fill and spin are for shapes"),
        "{e}"
    );

    // Events past the component end.
    let e = issues(&program(
        r#"{"id": "s", "kind": "shape", "shape": "circle", "end": 1, "timeline": [
            {"at": 0.8, "do": "morph", "to": "square"}]}"#,
        "",
    ));
    assert!(e.contains("after the component"), "{e}");

    // Morph chains may return to an earlier shape but not stay put.
    assert_eq!(
        issues(&program(
            r#"{"id": "s", "kind": "shape", "shape": "circle", "timeline": [
            {"at": 0, "do": "morph", "to": "square"}, {"at": 1, "do": "morph", "to": "circle"}]}"#,
            ""
        )),
        ""
    );
    let e = issues(&program(
        r#"{"id": "s", "kind": "shape", "shape": "circle", "timeline": [
            {"at": 0, "do": "morph", "to": "circle"}]}"#,
        "",
    ));
    assert!(e.contains("already is"), "{e}");

    // Illegible palette, duplicate ids, bad custom path.
    let low = program(r#"{"id": "a", "kind": "backdrop"}"#, "").replace("#151413", "#D9D2C5");
    assert!(issues(&low).contains("contrast"));
    let e = issues(&program(
        r#"{"id": "a", "kind": "backdrop"}, {"id": "a", "kind": "shape", "shape": {"path": "M0 0 L"}}"#,
        "",
    ));
    assert!(
        e.contains("duplicate id") && e.contains("shape.path"),
        "{e}"
    );
}

#[test]
fn unknown_fields_and_free_colours_are_rejected() {
    // Components name swatches, never colours.
    let bad = program(
        r##"{"id": "s", "kind": "shape", "shape": "circle", "colour": "#00FF00"}"##,
        "",
    );
    assert!(MotionProgram::from_json(&bad).is_err());
    let bad = program(r#"{"id": "a", "kind": "backdrop"}"#, r#", "seconds": 3"#);
    assert!(MotionProgram::from_json(&bad).is_err());
}

#[test]
fn schema_matches_rust_types() {
    let path = root().join("schema/motion_program.json");
    let fresh = serde_json::to_value(schemars::schema_for!(MotionProgram)).unwrap();
    if std::env::var("MOTION_UPDATE_SCHEMA").as_deref() == Ok("1") {
        let text = serde_json::to_string_pretty(&fresh).unwrap() + "\n";
        std::fs::write(&path, text).expect("write schema");
        return;
    }
    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("schema on disk")).unwrap();
    assert_eq!(
        on_disk, fresh,
        "schema/motion_program.json is stale; regenerate with \
         MOTION_UPDATE_SCHEMA=1 cargo test -p motion-program --test program"
    );
}

fn beats(track: &str, start: f64) -> Vec<f64> {
    let text = std::fs::read_to_string(root().join(format!("assets/music/{track}.music.json")))
        .expect("music analysis");
    let plan: motion_core::audio::MusicPlan = serde_json::from_str(&text).unwrap();
    plan.beat_times
        .iter()
        .map(|t| t - start)
        .filter(|t| *t >= 0.0)
        .collect()
}

fn downbeats(track: &str, start: f64) -> Vec<f64> {
    let text = std::fs::read_to_string(root().join(format!("assets/music/{track}.music.json")))
        .expect("music analysis");
    let plan: motion_core::audio::MusicPlan = serde_json::from_str(&text).unwrap();
    plan.downbeat_times
        .iter()
        .map(|t| t - start)
        .filter(|t| *t >= 0.0)
        .collect()
}

fn example(name: &str) -> String {
    std::fs::read_to_string(root().join(format!("examples/v2/{name}.program.json"))).unwrap()
}

#[test]
fn phase2_samples_validate_and_lower_in_both_aspects() {
    for (name, music) in [
        ("board_reel", Some(("driving_pulse", 0.107))),
        ("paper_blob", Some(("retro_groove", 0.132))),
        ("data_cut", Some(("bright_upbeat", 0.508))),
    ] {
        let b = music.map(|(t, s)| beats(t, s)).unwrap_or_default();
        // Every sample keeps landing on beats to a final downbeat.
        if let Some((t, s)) = music {
            let mut p = MotionProgram::from_json(&example(name)).unwrap();
            p.resolve_beats(&b);
            let downs = downbeats(t, s);
            let cov = motion_program::qa::beat_coverage(&p, &b, &downs);
            assert!(cov.passes(), "{name}: {}", cov.report());
        }
        for aspect in ["16:9", "9:16"] {
            let json =
                example(name).replace(r#""aspect": "16:9""#, &format!(r#""aspect": "{aspect}""#));
            let scene = compile(&json, &b, Some("../assets".into()), &Typesetter::approx())
                .unwrap_or_else(|e| panic!("{name} {aspect}: {e}"));
            motion_core::validate(&scene, Some(&root().join("examples")))
                .unwrap_or_else(|e| panic!("{name} {aspect}: {e}"));
        }
    }
}

#[test]
fn kinetic_cuts_land_within_a_frame_of_the_beat() {
    let b = beats("driving_pulse", 0.107);
    let mut p = MotionProgram::from_json(&example("board_reel")).unwrap();
    p.resolve_beats(&b);
    let scene = compile(&example("board_reel"), &b, None, &Typesetter::approx()).unwrap();
    let fps = scene.canvas.fps as f64;
    let poster = p.components.iter().find(|c| c.id == "open").unwrap();
    let mut checked = 0;
    for e in &poster.timeline {
        if !matches!(e.verb, motion_program::program::Verb::Cut { .. }) {
            continue;
        }
        // Resolved cut time and the beat it was snapped from.
        let t = poster.start + e.at.unwrap();
        let n = (0..b.len())
            .min_by(|x, y| (b[*x] - t).abs().total_cmp(&(b[*y] - t).abs()))
            .unwrap();
        let beat = b[n];
        // The word shown from this cut is the scene starting nearest it.
        let start = scene
            .scenes
            .iter()
            .filter(|s| s.id.starts_with("open.w"))
            .map(|s| s.start_seconds)
            .min_by(|x, y| (x - beat).abs().total_cmp(&(y - beat).abs()))
            .unwrap();
        // Cuts lead their beat by CUT_LEAD_FRAMES (read on the beat).
        let lead = motion_program::taste::CUT_LEAD_FRAMES;
        let first = (start * fps - 1e-9).ceil();
        assert_eq!(
            first,
            (beat * fps).round() - lead,
            "beat {n} at {beat}s, cut at {start}s"
        );
        checked += 1;
    }
    assert_eq!(checked, 7);
}

#[test]
fn recipes_parse_and_fill_in_the_palette() {
    use motion_program::program::Recipe;
    for r in [Recipe::PosterKinetic, Recipe::ThinAxis, Recipe::OrangeDot] {
        let spec = motion_program::recipes::spec(r);
        assert!(!spec.description.is_empty());
        let json = program(r#"{"id": "a", "kind": "backdrop"}"#, "")
            .replace(r##""palette": {"background": "#F1ECE2", "ink": "#151413", "accent": "#FF5A1F", "muted": "#8E877B"}"##, "")
            .replace(r#""meta": {"name": "t", "duration": 4},"#, &format!(r#""meta": {{"name": "t", "duration": 4}}, "recipe": {}"#, serde_json::to_string(&r).unwrap()));
        assert_eq!(issues(&json), "", "{r:?}");
    }
}

#[test]
fn phase2_validator_rules() {
    // More cuts than words.
    let e = issues(&program(
        r#"{"id": "k", "kind": "kinetic_text", "words": ["A", "B"], "timeline": [
            {"at": 0.5, "do": "cut"}, {"at": 1.0, "do": "cut"}]}"#,
        "",
    ));
    assert!(e.contains("2 cuts but only 2 words"), "{e}");
    // Flip only on posters; kinetic text takes no morph.
    let e = issues(&program(
        r#"{"id": "k", "kind": "kinetic_text", "preset": "type_on", "words": ["A", "B"], "timeline": [
            {"at": 0.5, "do": "cut", "flip": true}, {"at": 1.0, "do": "hit"}]}"#,
        "",
    ));
    assert!(
        e.contains("flip needs the poster_cuts") && e.contains("kinetic text takes"),
        "{e}"
    );
    // Charts: pop_label needs a hero; donut values non-negative.
    let e = issues(&program(
        r#"{"id": "c", "kind": "chart", "chart": "donut", "values": [3, -1],
            "timeline": [{"at": 0, "do": "pop_label"}]}"#,
        "",
    ));
    assert!(
        e.contains("needs a hero") && e.contains("donut values"),
        "{e}"
    );
    // Beats without music stay unresolved.
    let e = issues(&program(
        r#"{"id": "s", "kind": "shape", "shape": "circle", "timeline": [{"beat": 2, "do": "bounce"}]}"#,
        "",
    ));
    assert!(e.contains("beat not in the music"), "{e}");
}

/// The mistake ledger: every fixture reproduces its mistake (the detector
/// fails) and is cleared by its repair or by the compiler defaults.
#[test]
fn ledger_fixtures_fail_then_pass() {
    use motion_program::ledger::{ledger, static_gates};
    let gates_on = |p: &MotionProgram, scene: &motion_core::MotionProject, id: &str| {
        static_gates(p, scene)
            .into_iter()
            .filter(|f| f.id == id)
            .count()
    };
    let lowered = |p: &MotionProgram| lower(p, None, &Typesetter::approx());
    let mut checked = 0;
    for e in ledger().entries {
        let Some(path) = &e.fixture else { continue };
        let fx: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(root().join(path)).expect("fixture"))
                .unwrap();
        let prog = |k: &str| -> Option<MotionProgram> {
            fx.get(k)
                .map(|v| serde_json::from_value(v.clone()).expect("fixture program"))
        };
        match fx["kind"].as_str().unwrap() {
            "program" => {
                let mut bad = prog("program").unwrap();
                assert!(
                    gates_on(&bad, &lowered(&bad), &e.id) > 0,
                    "{}: the fixture must fail its detector",
                    e.id
                );
                let good = if fx.get("repair").is_some() {
                    let fails = static_gates(&bad, &lowered(&bad));
                    assert!(
                        motion_program::repair::repair(&mut bad, &fails),
                        "{}: repair",
                        e.id
                    );
                    bad
                } else {
                    prog("fixed").unwrap()
                };
                assert_eq!(
                    gates_on(&good, &lowered(&good), &e.id),
                    0,
                    "{}: must pass after the fix",
                    e.id
                );
            }
            // A compiler default: the detector fails the program lowered
            // with the default switched off, and passes it with defaults.
            "option" => {
                use motion_program::ledger::anticipatory_space;
                let p = prog("program").unwrap();
                let off = motion_program::LowerOptions {
                    follow_content: false,
                    ..Default::default()
                };
                assert!(
                    !anticipatory_space(&p, off).is_empty(),
                    "{}: the fixture must fail with the default off",
                    e.id
                );
                assert_eq!(
                    anticipatory_space(&p, Default::default()),
                    vec![],
                    "{}: must pass with the default",
                    e.id
                );
            }
            "scene" => {
                let raw: motion_core::MotionProject =
                    serde_json::from_value(fx["scene"].clone()).expect("fixture scene");
                let shell = prog("fixed").unwrap();
                assert!(
                    gates_on(&shell, &raw, &e.id) > 0,
                    "{}: the raw scene must fail",
                    e.id
                );
                assert_eq!(
                    gates_on(&shell, &lowered(&shell), &e.id),
                    0,
                    "{}: the compiler's lowering must pass",
                    e.id
                );
            }
            "beats" => {
                let (track, start) = (
                    fx["track"][0].as_str().unwrap(),
                    fx["track"][1].as_f64().unwrap(),
                );
                let (b, d) = (beats(track, start), downbeats(track, start));
                let cov = |k: &str| {
                    let mut p = prog(k).unwrap();
                    p.resolve_beats(&b);
                    motion_program::qa::beat_coverage(&p, &b, &d).passes()
                };
                assert!(!cov("program"), "beats_dropped: fixture must fail");
                assert!(cov("fixed"), "beats_dropped: fixed must pass");
            }
            // Rendered fixtures (design-system detectors) are checked with
            // real renders in tests/design.rs.
            "rendered" => continue,
            other => panic!("unknown fixture kind {other}"),
        }
        checked += 1;
    }
    assert!(checked >= 11, "{checked} fixtures checked");
}

#[test]
fn duplicate_cutout_in_shot_gate_flags_repeated_cutouts() {
    use motion_program::ledger::static_gates;

    let p_json = program(r##"{"id": "a", "kind": "backdrop"}"##, "");
    let p = MotionProgram::from_json(&p_json).unwrap();
    let mut proj = lower(&p, None, &Typesetter::approx());

    if let Some(scene) = proj.scenes.first_mut() {
        let l1 = motion_core::scene::Layer {
            kind: motion_core::scene::LayerKind::Image {
                asset: "orrery_engraving_01".into(),
                fit: motion_core::scene::Fit::Contain,
                treatment: None,
                playback: None,
                insert: None,
            },
            id: "img1".into(),
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            anchor_x: 0.5,
            anchor_y: 0.5,
            opacity: 1.0,
            z_index: 1,
            visible: true,
            clip: None,
            mask: None,
            depth: None,
            z: None,
            tilt: None,
            layout: None,
        };
        let mut l2 = l1.clone();
        l2.id = "img2".into();
        scene.layers.push(l1);
        scene.layers.push(l2);
    }

    let fails = static_gates(&p, &proj);
    let dup_fails: Vec<_> = fails
        .iter()
        .filter(|f| f.id == "duplicate_cutout_in_shot")
        .collect();
    assert_eq!(dup_fails.len(), 1);
    assert!(dup_fails[0].detail.contains("repeats orrery_engraving_01"));
}
