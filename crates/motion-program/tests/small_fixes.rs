//! Number grouping in bento stats and chart callouts, and the near-blank
//! message detail (frame number and components on screen).

use motion_core::scene::{Layer, LayerKind, MotionOp, Scene};
use motion_core::timeline::format_count;
use motion_core::MotionProject;
use motion_program::program::MotionProgram;
use motion_program::{lower, validate, Typesetter};

fn program(components: &str) -> MotionProgram {
    let json = format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 4, "aspect": "16:9"}},
        "palette": {{"background": "#101010", "ink": "#FFFFFF", "accent": "#00FFCC", "muted": "#888888"}},
        "components": [{components}]}}"##
    );
    MotionProgram::from_json(&json).expect("parses")
}

fn lowered(components: &str) -> (MotionProgram, MotionProject) {
    let p = program(components);
    validate(&p).expect("valid");
    let scene = lower(&p, None, &Typesetter::approx());
    (p, scene)
}

/// Lowered without the slop gates: the near-blank probes use tiny static dots.
fn lowered_unchecked(components: &str) -> (MotionProgram, MotionProject) {
    let p = program(components);
    let scene = lower(&p, None, &Typesetter::approx());
    (p, scene)
}

fn scene_of<'a>(project: &'a MotionProject, id: &str) -> &'a Scene {
    project
        .scenes
        .iter()
        .find(|s| s.id == id)
        .expect("component scene")
}

fn text_of(layer: &Layer) -> String {
    let LayerKind::Text(style) = &layer.kind else {
        panic!("{} is not text", layer.id);
    };
    style.text.clone()
}

/// The count-up targeting `target`: `(grouping, shown at 0, 0.5 and 1.0 of the way)`.
fn count_of(scene: &Scene, target: &str) -> Option<(bool, [String; 3])> {
    scene.motions.iter().find_map(|m| match &m.op {
        MotionOp::Count {
            to,
            decimals,
            grouping,
            prefix,
            suffix,
            ..
        } if m.target == target => {
            let at = |p: f64| format_count(to * p, *decimals, *grouping, prefix, suffix);
            Some((*grouping, [at(0.0), at(0.5), at(1.0)]))
        }
        _ => None,
    })
}

#[test]
fn bento_stats_group_five_digit_integers_and_leave_the_rest() {
    let (_, project) = lowered(
        r#"{"id": "bento", "kind": "bento", "start": 0.0, "tiles": [
            {"stat": "1284960", "title": "ORDERS"},
            {"stat": "12345", "title": "USERS"},
            {"stat": "1234", "title": "PLAIN"},
            {"stat": "3.2x", "title": "SPEEDUP"},
            {"stat": "98%", "title": "UPTIME"}],
          "timeline": [{"at": 0.0, "do": "enter", "style": "pop"}]}"#,
    );
    let scene = scene_of(&project, "bento");
    let stat = |i: usize| {
        let group = scene
            .layers
            .iter()
            .find(|l| l.id == format!("bento.tile{i}"))
            .expect("tile group");
        let LayerKind::Group { children } = &group.kind else {
            panic!("tile is a group");
        };
        text_of(
            children
                .iter()
                .find(|l| l.id == format!("bento.tile{i}.stat"))
                .expect("stat layer"),
        )
    };
    assert_eq!(stat(0), "1,284,960");
    assert_eq!(stat(1), "12,345");
    assert_eq!(stat(2), "1234");
    assert_eq!(stat(3), "3.2x");
    assert_eq!(stat(4), "98%");
    // The count-up groups every intermediate value like the final one.
    let (grouped, [start, half, end]) = count_of(scene, "bento.tile0.stat").expect("count-up");
    assert!(grouped);
    assert_eq!((start.as_str(), half.as_str()), ("0", "642,480"));
    assert_eq!(end, "1,284,960");
    let (grouped, [_, _, end]) = count_of(scene, "bento.tile1.stat").expect("count-up");
    assert!(grouped);
    assert_eq!(end, "12,345");
    for (i, shown) in [(2, "1234"), (3, "3.2"), (4, "98")] {
        let (grouped, [_, _, end]) =
            count_of(scene, &format!("bento.tile{i}.stat")).expect("count");
        assert!(!grouped, "tile {i}");
        assert!(end.starts_with(shown), "tile {i}: {end}");
    }
}

#[test]
fn chart_callouts_group_five_digit_integers_and_leave_the_rest() {
    let shown = |callout: &str| -> (String, Option<(bool, [String; 3])>) {
        let (_, project) = lowered(&format!(
            r#"{{"id": "c", "kind": "chart", "chart": "line", "values": [3, 5, 4, 7, 9, 8, 12, 14],
                "hero": 7, "callout": "{callout}", "start": 0.0,
                "timeline": [{{"at": 0.0, "do": "enter", "style": "draw"}},
                             {{"at": 1.0, "do": "pop_label"}}]}}"#
        ));
        let scene = scene_of(&project, "c");
        let layer = scene
            .layers
            .iter()
            .find(|l| l.id == "c.callout")
            .expect("callout layer");
        (text_of(layer), count_of(scene, "c.callout"))
    };
    let (text, count) = shown("1284960");
    assert_eq!(text, "1,284,960");
    let (grouped, [start, half, end]) = count.expect("count-up");
    assert!(grouped);
    assert_eq!((start.as_str(), half.as_str()), ("0", "642,480"));
    assert_eq!(end, "1,284,960");
    assert_eq!(shown("12345").0, "12,345");
    for same in ["1234", "3.2x", "98%", "14k"] {
        let (text, count) = shown(same);
        assert_eq!(text, same);
        assert!(!count.expect("count-up").0, "{same}");
    }
}

#[test]
fn near_blank_names_the_frame_and_the_components_on_screen() {
    let (p, project) = lowered_unchecked(
        r#"{"id": "bg", "kind": "backdrop"},
           {"id": "dot_a", "kind": "shape", "shape": "circle", "size": "s", "start": 0.0, "end": 4.0},
           {"id": "dot_b", "kind": "shape", "shape": "circle", "size": "s", "slot": "trail",
            "start": 3.0, "end": 4.0}"#,
    );
    let fps = project.canvas.fps as f64;
    let blank = motion_program::qa::near_blank(&p, &project, &[2.38, 3.5]);
    assert_eq!(blank.len(), 2, "{blank:?}");
    let f = |t: f64| (t * fps).round() as u32;
    assert_eq!(blank[0].frame, f(2.38));
    assert_eq!(blank[0].on_screen, ["dot_a"]);
    assert_eq!(
        blank[0].describe(),
        format!("2.38 s (f{}; on screen: dot_a)", f(2.38))
    );
    assert_eq!(blank[1].frame, f(3.5));
    assert_eq!(blank[1].on_screen, ["dot_a", "dot_b"]);
    assert_eq!(
        blank[1].describe(),
        format!("3.5 s (f{}; on screen: dot_a, dot_b)", f(3.5))
    );
}

#[test]
fn near_blank_with_nothing_visible_says_none_and_filled_frames_pass() {
    let (p, project) = lowered_unchecked(
        r#"{"id": "bg", "kind": "backdrop"},
           {"id": "late", "kind": "shape", "shape": "circle", "size": "s", "start": 3.0, "end": 4.0}"#,
    );
    let blank = motion_program::qa::near_blank(&p, &project, &[0.5]);
    assert_eq!(blank.len(), 1, "{blank:?}");
    assert!(blank[0].on_screen.is_empty());
    assert!(blank[0].describe().ends_with("on screen: none)"));

    let (p, project) = lowered_unchecked(
        r#"{"id": "bg", "kind": "backdrop"},
           {"id": "head", "kind": "title", "text": "Hello", "size": "xl", "start": 0.0, "end": 4.0}"#,
    );
    assert!(motion_program::qa::near_blank(&p, &project, &[1.0, 2.0]).is_empty());
}
