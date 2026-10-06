//! (0.22) Ranked bars. A collection of pictures with numeric values (the
//! consumer's "five countries and their inflation" shape) is a bar chart in
//! every look: one horizontal row per item in the written order, each with
//! its picture, its name and its value, growing when that item is named. A
//! series over time keeps its vertical bars; the cinematic look keeps the
//! sphere for collections that are not numeric, and its object cards show
//! their values.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::speech_plan::WORD_CUE_LEAD;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject, Scene};
use motion_core::speech::{RevealRole, SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;
use motion_core::{layout_report, LayoutVerdict};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Catalog pictures every look's families include (`editorial_cutout`):
/// `(asset, meaning, value)`, written high → low like a ranking.
const ITEMS: [(&str, &str, &str); 6] = [
    ("coin_stack", "Savings", "4.1%"),
    ("laptop", "Laptops", "4.0%"),
    ("phone", "Phones", "3.4%"),
    ("plant", "Plants", "3.1%"),
    ("clock", "Clocks", "2.2%"),
    ("globe", "Globes", "1.5%"),
];

fn ranking(n: usize, format: &str) -> CreativeIntent {
    let items: Vec<String> = ITEMS[..n]
        .iter()
        .map(|(asset, meaning, value)| {
            format!(
                r#"{{"kind":"object","asset":"{asset}","value":"{value}","meaning":"{meaning}"}}"#
            )
        })
        .collect();
    CreativeIntent::from_json(&format!(
        r#"{{"version":"0.2","title":"ranking","format":"{format}","beats":[{{
            "purpose":"compare","statement":"Where the money goes",
            "primary":{{"kind":"collection","items":[{}]}}}}]}}"#,
        items.join(",")
    ))
    .expect("intent")
}

fn numbers(items: &[(&str, &str)]) -> CreativeIntent {
    let items: Vec<String> = items
        .iter()
        .map(|(value, meaning)| {
            format!(r#"{{"kind":"number","value":"{value}","meaning":"{meaning}"}}"#)
        })
        .collect();
    CreativeIntent::from_json(&format!(
        r#"{{"version":"0.2","title":"series","format":"vertical","beats":[{{
            "purpose":"emphasize","statement":"Signups over the years",
            "primary":{{"kind":"collection","items":[{}]}}}}]}}"#,
        items.join(",")
    ))
    .expect("intent")
}

fn compile_with(
    intent: &CreativeIntent,
    look: Option<Look>,
    canvas: Option<(u32, u32)>,
    speech: Option<SpeechMap>,
) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        art: look.map(ArtMode::Force),
        canvas,
        speech,
        ..CompileOptions::default()
    };
    compile_with_options(
        intent,
        &StyleProfile::default(),
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn compile(intent: &CreativeIntent, look: Option<Look>) -> MotionProject {
    compile_with(intent, look, None, None)
}

fn beat(p: &MotionProject) -> &Scene {
    p.scenes.iter().find(|s| s.id == "beat_1").expect("beat 1")
}

fn flat(layers: &[Layer]) -> Vec<&Layer> {
    let mut out = Vec::new();
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            out.extend(flat(children));
        }
    }
    out
}

fn layer<'a>(s: &'a Scene, id: &str) -> Option<&'a Layer> {
    flat(&s.layers).into_iter().find(|l| l.id == id)
}

fn texts(s: &Scene) -> Vec<String> {
    flat(&s.layers)
        .into_iter()
        .filter_map(|l| match &l.kind {
            LayerKind::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect()
}

fn row_ids(s: &Scene, i: usize) -> Vec<String> {
    let prefix = format!("b1.rank.{i}.");
    flat(&s.layers)
        .into_iter()
        .filter(|l| l.id.starts_with(&prefix))
        .map(|l| l.id.clone())
        .collect()
}

#[test]
fn a_ranking_of_pictures_is_bars_with_pictures_names_and_values_in_every_look() {
    let story = ranking(4, "vertical");
    for look in Look::ALL {
        let p = compile(&story, Some(look));
        validate(&p, Some(&repo().join("assets"))).expect("valid");
        let s = beat(&p);
        let life = s.lifecycle.expect("lifecycle");
        let all = texts(s);
        let mut last_y = f32::MIN;
        for (i, (_, meaning, value)) in ITEMS[..4].iter().enumerate() {
            let ids = row_ids(s, i);
            for part in ["pic", "name", "bar", "value"] {
                let id = format!("b1.rank.{i}.{part}");
                assert!(ids.contains(&id), "{look:?}: {id} missing from {ids:?}");
            }
            assert!(
                all.iter().any(|t| t == value),
                "{look:?}: value {value} missing from {all:?}"
            );
            assert!(
                all.iter().any(|t| t.eq_ignore_ascii_case(meaning)),
                "{look:?}: name {meaning} missing from {all:?}"
            );
            // The picture is a picture.
            let pic = layer(s, &format!("b1.rank.{i}.pic")).expect("pic");
            assert!(
                matches!(pic.kind, LayerKind::Image { .. } | LayerKind::Svg { .. }),
                "{look:?}: row {i} picture is {:?}",
                pic.kind
            );
            // Rows in the written order, top down.
            let bar = layer(s, &format!("b1.rank.{i}.bar")).expect("bar");
            assert!(bar.y > last_y, "{look:?}: row {i} is above row {}", i - 1);
            last_y = bar.y;
            // The bar grows to the right and has landed before ANTICIPATE.
            let grow = s
                .motions
                .iter()
                .find(|m| m.target == bar.id && matches!(m.op, MotionOp::AccentExpand { .. }))
                .unwrap_or_else(|| panic!("{look:?}: row {i} bar does not grow"));
            assert!(
                grow.start + grow.duration <= life.anticipate + 1e-6,
                "{look:?}: row {i} lands at {} after ANTICIPATE {}",
                grow.start + grow.duration,
                life.anticipate
            );
            // No bars of the time-series chart.
            assert!(layer(s, &format!("b1.chart.bar.{i}")).is_none());
        }
        // Longer value, longer bar; the largest one wears the accent.
        let target = |i: usize| -> f32 {
            s.motions
                .iter()
                .find_map(|m| match &m.op {
                    MotionOp::AccentExpand { to } if m.target == format!("b1.rank.{i}.bar") => {
                        Some(to.width)
                    }
                    _ => None,
                })
                .expect("bar target")
        };
        assert!(
            (0..3).all(|i| target(i) >= target(i + 1)),
            "{look:?}: bar lengths do not follow the values"
        );
        // Each row is a reveal anchor on its name and value.
        let reveals = &p.project.art.as_ref().expect("art record").reveals["beat_1"];
        for (i, (_, meaning, value)) in ITEMS[..4].iter().enumerate() {
            let a = reveals
                .iter()
                .find(|a| a.group == format!("rank.{i}"))
                .unwrap_or_else(|| panic!("{look:?}: no anchor for row {i}"));
            assert_eq!(a.role, RevealRole::Content);
            assert_eq!(a.words, vec![meaning.to_string(), value.to_string()]);
        }
    }
}

#[test]
fn rows_arrive_one_by_one_in_the_written_order_without_a_voice() {
    let p = compile(&ranking(4, "vertical"), Some(Look::Dossier));
    let s = beat(&p);
    let life = s.lifecycle.expect("lifecycle");
    let first = |i: usize| -> f64 {
        let ids = row_ids(s, i);
        s.motions
            .iter()
            .filter(|m| ids.contains(&m.target))
            .map(|m| m.start)
            .fold(f64::MAX, f64::min)
    };
    let starts: Vec<f64> = (0..4).map(first).collect();
    assert!(starts.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
    assert!(starts[0] >= life.settle - 1e-6, "{starts:?}");
    assert!(starts[1..].iter().all(|t| *t >= life.evolve - 1e-6));
    // Every row element is hidden until its row arrives: each has a fade in.
    for i in 0..4 {
        for part in ["pic", "name", "value"] {
            let id = format!("b1.rank.{i}.{part}");
            assert!(
                s.motions.iter().any(|m| m.target == id
                    && matches!(m.op, MotionOp::Fade { from, .. } if from == 0.0)),
                "{id} has no entrance"
            );
        }
    }
    // Nothing new starts at or after ANTICIPATE.
    for m in s.motions.iter().filter(|m| m.target.contains(".rank.")) {
        assert!(m.start < life.anticipate, "{} at {}", m.target, m.start);
    }
}

#[test]
fn named_numbers_rank_and_a_series_over_time_stays_vertical() {
    // Numbers that name things (no pictures): ranked rows without pictures.
    let named = numbers(&[
        ("4.1%", "United Kingdom"),
        ("4.0%", "United States"),
        ("3.4%", "Australia"),
    ]);
    let p = compile(&named, None);
    let s = beat(&p);
    for i in 0..3 {
        assert!(layer(s, &format!("b1.rank.{i}.bar")).is_some());
        assert!(layer(s, &format!("b1.rank.{i}.pic")).is_none());
    }
    // Years, quarters and months read left to right: the vertical chart.
    for series in [
        &[("120", "2019"), ("180", "2020"), ("260", "2021")][..],
        &[("120", "Q1"), ("180", "Q2"), ("260", "Q3"), ("410", "Q4")][..],
        &[("10", "Jan 2024"), ("40", "Feb 2024"), ("25", "March 2024")][..],
        &[("5", "FY24"), ("7", "FY25"), ("9", "FY26e")][..],
        &[("5", "Week 1"), ("7", "Week 2"), ("9", "Week 3")][..],
        &[("5", ""), ("7", ""), ("9", "")][..],
    ] {
        let p = compile(&numbers(series), None);
        let s = beat(&p);
        for i in 0..series.len() {
            assert!(
                layer(s, &format!("b1.chart.bar.{i}")).is_some(),
                "{series:?}: vertical bar {i}"
            );
        }
        assert!(layer(s, "b1.trend.line").is_some(), "{series:?}");
        assert!(row_ids(s, 0).is_empty(), "{series:?}");
    }
    // The cinematic look keeps a numeric series as a chart too.
    let p = compile(&ranking(3, "vertical"), Some(Look::Cinematic3d));
    assert!(layer(beat(&p), "b1.rank.2.bar").is_some());
    assert!(layer(beat(&p), "b1.item.0").is_none(), "no sphere");
}

#[test]
fn sphere_object_cards_show_their_values() {
    // A value that is not a number keeps the collection on the sphere.
    let story = CreativeIntent::from_json(
        r#"{"version":"0.2","title":"sphere","format":"vertical","beats":[{
            "purpose":"emphasize","statement":"What we keep",
            "primary":{"kind":"collection","items":[
              {"kind":"object","asset":"coin_stack","value":"4.1%","meaning":"Savings"},
              {"kind":"object","asset":"laptop","value":"3.1%","meaning":"Laptops"},
              {"kind":"object","asset":"phone","value":"most used","meaning":"Phones"}]}}]}"#,
    )
    .expect("intent");
    let p = compile(&story, Some(Look::Cinematic3d));
    let s = beat(&p);
    assert!(
        s.motions
            .iter()
            .any(|m| matches!(m.op, MotionOp::Revolve { .. })),
        "the collection rides a sphere"
    );
    for (k, value) in [(0, "4.1%"), (1, "3.1%"), (2, "most used")] {
        // The item is its picture and its figure, riding the sphere as one.
        let item = layer(s, &format!("b1.item.{k}")).expect("item");
        let LayerKind::Group { children } = &item.kind else {
            panic!("item {k} is not a group: {:?}", item.kind);
        };
        assert!(children
            .iter()
            .any(|c| matches!(c.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })));
        let chip = layer(s, &format!("b1.item.{k}.value.text")).expect("value chip");
        match &chip.kind {
            LayerKind::Text(t) => assert_eq!(t.text, value),
            other => panic!("value chip is {other:?}"),
        }
    }
    // An object without a value is still a bare picture.
    let bare = CreativeIntent::from_json(
        r#"{"version":"0.2","title":"sphere","format":"vertical","beats":[{
            "purpose":"emphasize","statement":"What we keep",
            "primary":{"kind":"collection","items":[
              {"kind":"object","asset":"coin_stack"},
              {"kind":"object","asset":"laptop"},
              {"kind":"object","asset":"phone"}]}}]}"#,
    )
    .expect("intent");
    let p = compile(&bare, Some(Look::Cinematic3d));
    let item = layer(beat(&p), "b1.item.0").expect("item");
    assert!(matches!(
        item.kind,
        LayerKind::Image { .. } | LayerKind::Svg { .. }
    ));
}

#[test]
fn ranked_bars_pass_layout_qa_on_tall_square_and_wide_canvases() {
    let canvases = [(1080, 1920), (1080, 1080), (1920, 1080)];
    let looks = std::iter::once(None).chain(Look::ALL.into_iter().map(Some));
    let looks: Vec<Option<Look>> = looks.collect();
    for n in [3, 6] {
        let story = ranking(n, "vertical");
        for &(w, h) in &canvases {
            for &look in &looks {
                let p = compile_with(&story, look, Some((w, h)), None);
                let s = beat(&p);
                assert!(
                    layer(s, &format!("b1.rank.{}.bar", n - 1)).is_some(),
                    "{look:?} {w}x{h}: {n} rows"
                );
                let frame = LayoutFrame::new(w, h).expect("frame");
                let report = layout_report(&p, &frame);
                assert_eq!(
                    report.verdict,
                    LayoutVerdict::Pass,
                    "{look:?} {n} rows on {w}x{h}:\n{}",
                    report.to_text()
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Speech
// ---------------------------------------------------------------------------

const LINE: &str = "Savings lead at four point one percent. Laptops follow at four percent. \
                    Phones sit at three point four percent. Plants trail at three point one percent.";
const START: f64 = 0.35;
const STEP: f64 = 0.32;

fn speech() -> SpeechMap {
    let toks: Vec<&str> = LINE.split_whitespace().collect();
    let words: Vec<SpeechWord> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let at = START + i as f64 * STEP;
            SpeechWord {
                text: t.to_string(),
                start: (at * 1000.0).round() / 1000.0,
                end: ((at + 0.8 * STEP) * 1000.0).round() / 1000.0,
                confidence: 0.9,
            }
        })
        .collect();
    let end = START + toks.len() as f64 * STEP;
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: end + 0.9,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences: vec![SpeechSentence {
            beat: 0,
            start: START,
            end,
        }],
    }
}

/// Scene-local time the narrator says `word`.
fn spoken(scene: &Scene, word: &str) -> f64 {
    let k = LINE
        .split_whitespace()
        .position(|t| t == word)
        .expect("word in line");
    START + k as f64 * STEP - scene.start_seconds
}

#[test]
fn each_row_grows_when_its_name_is_spoken() {
    for look in [None, Some(Look::Dossier), Some(Look::Cinematic3d)] {
        let p = compile_with(&ranking(4, "vertical"), look, None, Some(speech()));
        validate(&p, Some(&repo().join("assets"))).expect("valid");
        let s = beat(&p);
        let cues = &p.project.speech.as_ref().expect("speech record").word_cues;
        let mut last = f64::MIN;
        for (i, name) in ["Savings", "Laptops", "Phones", "Plants"]
            .iter()
            .enumerate()
        {
            let group = format!("rank.{i}");
            let cue = cues
                .iter()
                .find(|c| c.group == group)
                .unwrap_or_else(|| panic!("{look:?}: no cue for {group}: {cues:?}"));
            assert_eq!(cue.role, Some(RevealRole::Content), "{cue:?}");
            assert!(
                cue.word.eq_ignore_ascii_case(name),
                "{look:?}: {group} cued on {:?}, not {name}",
                cue.word
            );
            assert!(!cue.clamped, "{look:?}: {cue:?}");
            // (Nothing enters before ENTER, however early the word.)
            let life = s.lifecycle.expect("lifecycle");
            let expect = (spoken(s, name) - WORD_CUE_LEAD).max(life.enter);
            assert!(
                (cue.to - expect).abs() < 2e-3,
                "{look:?}: {cue:?}, expected {expect}"
            );
            // The row's first motion is on the cue, rows still in order.
            let ids = row_ids(s, i);
            let start = s
                .motions
                .iter()
                .filter(|m| ids.contains(&m.target))
                .map(|m| m.start)
                .fold(f64::MAX, f64::min);
            assert!(
                (start - cue.to).abs() < 2e-3,
                "{look:?}: {group} at {start}"
            );
            assert!(start > last);
            last = start;
            // The bar still lands before ANTICIPATE.
            let bar = format!("b1.rank.{i}.bar");
            let grow = s
                .motions
                .iter()
                .find(|m| m.target == bar && matches!(m.op, MotionOp::AccentExpand { .. }))
                .expect("grow");
            assert!(grow.start + grow.duration <= life.anticipate + 1e-6);
        }
    }
}

#[test]
fn ranked_bars_are_deterministic() {
    let story = ranking(5, "vertical");
    let a = serde_json::to_string(&compile_with(
        &story,
        Some(Look::Dossier),
        None,
        Some(speech()),
    ))
    .expect("a");
    let b = serde_json::to_string(&compile_with(
        &story,
        Some(Look::Dossier),
        None,
        Some(speech()),
    ))
    .expect("b");
    assert_eq!(a, b);
}
