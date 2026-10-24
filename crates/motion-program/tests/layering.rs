//! Intentional layering (`over`), plate-less sketches and sketch
//! `beat_cuts: 0` (v2.2 task B).

use motion_program::program::{ComponentBody, MotionProgram};
use motion_program::qa::component_overlap;
use motion_program::{lower, validate, Typesetter};

fn program(components: &str) -> MotionProgram {
    let json = format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 6}},
        "palette": {{"background": "#F4EFE6", "ink": "#16130F", "accent": "#D94E28", "muted": "#C9C2B6"}},
        "components": [{components}]}}"##
    );
    MotionProgram::from_json(&json).expect("parses")
}

/// A centred title, a plate-less sketch circling it and a later title, all in
/// the middle of the frame. `over` is spliced into the sketch.
fn trio(over: &str) -> String {
    format!(
        r#"{{"id": "word", "kind": "title", "text": "Ideal", "size": "xl", "slot": "center"}},
        {{"id": "ring", "kind": "sketch", "preset": "freeform", "plate": false, "slot": "center"{over},
          "elements": [{{"shape": "circle", "center": [0.5, 0.5], "radius": 0.4, "brush": "pen"}}]}},
        {{"id": "dot", "kind": "title", "text": "Later", "size": "xl", "slot": "center", "start": 3.0}}"#
    )
}

fn pairs(p: &MotionProgram) -> Vec<(String, String)> {
    let s = lower(p, None, &Typesetter::approx());
    component_overlap(p, &s)
        .into_iter()
        .map(|o| {
            let mut v = [o.a, o.b];
            v.sort();
            (v[0].clone(), v[1].clone())
        })
        .collect()
}

fn has(pairs: &[(String, String)], a: &str, b: &str) -> bool {
    pairs
        .iter()
        .any(|(x, y)| (x == a && y == b) || (x == b && y == a))
}

#[test]
fn over_exempts_exactly_the_declared_pair() {
    let p = program(&trio(r#", "over": "word""#));
    validate(&p).expect("valid");
    let got = pairs(&p);
    assert!(!has(&got, "ring", "word"), "declared pair exempt: {got:?}");
    assert!(
        has(&got, "dot", "word") && has(&got, "dot", "ring"),
        "undeclared overlaps still fail: {got:?}"
    );
}

#[test]
fn undeclared_overlap_still_fails() {
    let p = program(&trio(""));
    validate(&p).expect("valid");
    let got = pairs(&p);
    assert!(has(&got, "ring", "word"), "no `over`, so a defect: {got:?}");
}

#[test]
fn over_rejects_bad_targets() {
    let msg = |over: &str| {
        let p = program(&trio(over));
        validate(&p).expect_err("must fail").to_string()
    };
    assert!(msg(r#", "over": "nope""#).contains("no component 'nope'"));
    assert!(msg(r#", "over": "ring""#).contains("cannot be over itself"));
    // `dot` starts at 3 s; a ring that ends at 2 s never meets it.
    assert!(msg(r#", "end": 2.0, "over": "dot""#).contains("never meets"));
    // A loop: ring over word, word over ring.
    let looped = trio(r#", "over": "word""#).replacen(
        r#""slot": "center"}"#,
        r#""slot": "center", "over": "ring"}"#,
        1,
    );
    let p = program(&looped);
    let e = validate(&p).expect_err("loop").to_string();
    assert!(e.contains("loops back"), "{e}");
}

#[test]
fn over_draws_above_its_target_whatever_the_order() {
    // The sketch is declared first (lower z) but sits over the title.
    let json = r#"
        {"id": "ring", "kind": "sketch", "preset": "freeform", "plate": false, "over": "word",
         "elements": [{"shape": "circle", "center": [0.5, 0.5], "radius": 0.4, "brush": "pen"}]},
        {"id": "late", "kind": "title", "text": "Later", "slot": "bottom", "start": 3.0},
        {"id": "word", "kind": "title", "text": "Ideal", "size": "xl", "slot": "center"}"#;
    let p = program(json);
    validate(&p).expect("valid");
    let s = lower(&p, None, &Typesetter::approx());
    let z = |prefix: &str| -> Vec<i32> {
        s.scenes
            .iter()
            .filter(|sc| sc.id == prefix || sc.id.starts_with(&format!("{prefix}.")))
            .flat_map(|sc| sc.layers.iter().map(|l| l.z_index))
            .collect()
    };
    let (ring, word) = (z("ring"), z("word"));
    assert!(!ring.is_empty() && !word.is_empty());
    assert!(
        ring.iter().min() > word.iter().max(),
        "ring {ring:?} must sit above word {word:?}"
    );
    // Without `over` the declaration order wins: ring stays below word.
    let q = program(&json.replace(r#", "over": "word""#, ""));
    let s2 = lower(&q, None, &Typesetter::approx());
    assert_eq!(s2.scenes[0].id, "ring");
    let plain: Vec<i32> = s2.scenes[0].layers.iter().map(|l| l.z_index).collect();
    assert!(
        plain.iter().max() < word.iter().min(),
        "{plain:?} vs {word:?}"
    );
}

const SPIRAL: &str = r#"{"id": "sk", "kind": "sketch", "preset": "golden_spiral", "seed": 7"#;

#[test]
fn plate_less_sketch_has_no_plate_layers() {
    let p = program(&format!(r#"{SPIRAL}, "plate": false}}"#));
    validate(&p).expect("valid");
    let s = lower(&p, None, &Typesetter::approx());
    let ids: Vec<&str> = s
        .scenes
        .iter()
        .flat_map(|sc| sc.layers.iter().map(|l| l.id.as_str()))
        .collect();
    assert!(ids.iter().any(|i| i.starts_with("sk.")), "{ids:?}");
    assert!(
        !ids.iter()
            .any(|i| i.contains("plate") || i.contains(".card")),
        "no plate or title bar: {ids:?}"
    );
    // The plated default keeps its plate.
    let p = program(&format!(r#"{SPIRAL}, "title": "Golden"}}"#));
    let s = lower(&p, None, &Typesetter::approx());
    let ids: Vec<&str> = s.scenes[0].layers.iter().map(|l| l.id.as_str()).collect();
    assert!(ids.contains(&"sk.plate") && ids.contains(&"sk.card.plate"));
}

#[test]
fn plate_less_sketch_rejects_title_and_caption() {
    let p = program(&format!(r#"{SPIRAL}, "plate": false, "title": "Golden"}}"#));
    let e = validate(&p)
        .expect_err("title on a plate-less sketch")
        .to_string();
    assert!(e.contains(".plate") && e.contains("title"), "{e}");
}

#[test]
fn plate_default_is_byte_identical() {
    let compile = |extra: &str| {
        let p = program(&format!(
            r#"{SPIRAL}, "title": "Golden", "caption": "phi"{extra}}}"#
        ));
        serde_json::to_string(&lower(&p, None, &Typesetter::approx())).unwrap()
    };
    let base = compile("");
    assert_eq!(base, compile(r#", "plate": true"#));
    assert_eq!(base, compile(r#", "plate": true, "size": "m""#));
    assert_ne!(base, compile(r#", "size": "s""#));
    // Defaults are not written back out.
    let p = program(&format!("{SPIRAL}}}"));
    let ComponentBody::Sketch(sk) = &p.components[0].body else {
        panic!("sketch")
    };
    let v = serde_json::to_value(sk).unwrap();
    assert!(v.get("plate").is_none() && v.get("size").is_none(), "{v}");
}

#[test]
fn sketch_beat_cuts_zero_is_one_clear_error() {
    let json = r##"{"version": "2.0", "meta": {"name": "t", "duration": 4},
        "palette": {"background": "#F4EFE6", "ink": "#16130F", "accent": "#D94E28", "muted": "#C9C2B6"},
        "music": {"track": "driving_pulse"},
        "components": [{"id": "sk", "kind": "sketch", "preset": "freeform", "start_beat": 0,
          "beat_cuts": 0, "elements": [
            {"shape": "line", "from": [0.1, 0.1], "to": [0.9, 0.1], "brush": "pen"},
            {"shape": "line", "from": [0.1, 0.3], "to": [0.9, 0.3], "brush": "pen"},
            {"shape": "line", "from": [0.1, 0.5], "to": [0.9, 0.5], "brush": "pen"},
            {"shape": "line", "from": [0.1, 0.7], "to": [0.9, 0.7], "brush": "pen"},
            {"shape": "line", "from": [0.1, 0.9], "to": [0.9, 0.9], "brush": "pen"}]}]}"##;
    let mut p = MotionProgram::from_json(json).expect("parses");
    // Beats every 0.9 s: the helper's one-per-element advances overrun 4 s.
    let beats: Vec<f64> = (0..8).map(|i| i as f64 * 0.9).collect();
    p.resolve_beats(&beats);
    let e = validate(&p).expect_err("beat_cuts 0");
    assert_eq!(e.0.len(), 1, "one issue, not one per advance: {e}");
    assert!(e.0[0].path.ends_with(".beat_cuts"), "{e}");
    assert!(e.0[0].message.contains(">= 1"), "{e}");
}

#[test]
fn over_is_for_non_type_components() {
    let err = |json: &str| validate(&program(json)).expect_err("type over").to_string();
    // (a) a title over a title.
    let e = err(
        r#"{"id": "a", "kind": "title", "text": "Ideal", "slot": "center"},
        {"id": "b", "kind": "title", "text": "Real", "slot": "center", "over": "a"}"#,
    );
    assert!(e.contains("only a sketch"), "{e}");
    // (b) an xl title over a chart.
    let e = err(
        r#"{"id": "c", "kind": "chart", "chart": "bar", "values": [3, 5, 8],
          "labels": ["A", "B", "C"]},
        {"id": "t", "kind": "title", "text": "Growth", "size": "xl", "slot": "center", "over": "c"}"#,
    );
    assert!(e.contains("only a sketch"), "{e}");
    // (c) a sketch over a title is still fine.
    validate(&program(&trio(r#", "over": "word""#))).expect("sketch over title");
}
