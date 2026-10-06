//! (0.23) `value_dropped`: the compiler reports every value, name or item the
//! intent gives that no text readable during its beat shows, and the looks
//! show them (the cinematic two-number comparison, the cinematic object with
//! a value, the street / studio pairs of figures).

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_report, value_coverage, ApproxMeasure, AssetLibrary, CompileOptions,
    CompileWarning, WARN_VALUE_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_report;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject, Scene};
use motion_core::speech::{
    SpeechMap, SpeechSentence, SpeechWord, READABLE_BLUR_PX, READABLE_OPACITY,
};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn library() -> AssetLibrary {
    AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
        "clay_props_3d".to_string(),
    ])
}

fn style_for(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(r#"{{"tone":"{tone}"}}"#)).expect("style")
}

fn read(path: &str) -> CreativeIntent {
    let path = repo().join(path);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    CreativeIntent::from_json(&text).expect("intent")
}

fn sleep() -> CreativeIntent {
    read("docs/plans/sprint_0_23/stories/sleep_review.intent.json")
}

fn money() -> CreativeIntent {
    read("docs/plans/sprint_0_23/stories/money_review.intent.json")
}

fn from_json(json: &str) -> CreativeIntent {
    CreativeIntent::from_json(json).expect("intent")
}

/// A voice-over that says each beat's narration (else statement), words
/// 0.32 s apart and 0.5 s between beats.
fn speech_for(intent: &CreativeIntent) -> SpeechMap {
    let (mut words, mut sentences) = (Vec::new(), Vec::new());
    let mut t = 0.3;
    for (i, beat) in intent.beats.iter().enumerate() {
        let line = beat
            .narration
            .clone()
            .unwrap_or_else(|| beat.statement.clone());
        let start = t;
        for w in motion_core::speech::statement_tokens(&line) {
            words.push(SpeechWord {
                text: w,
                start: (t * 1000.0_f64).round() / 1000.0,
                end: ((t + 0.27) * 1000.0_f64).round() / 1000.0,
                confidence: 1.0,
            });
            t += 0.32;
        }
        sentences.push(SpeechSentence {
            beat: i,
            start,
            end: t,
        });
        t += 0.5;
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: t,
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

fn compile(
    intent: &CreativeIntent,
    style: &StyleProfile,
    art: Option<ArtMode>,
    speech: Option<SpeechMap>,
) -> (MotionProject, Vec<CompileWarning>) {
    compile_on(intent, style, art, None, speech)
}

/// [`compile`] on a canvas (`None` = the intent's format).
fn compile_on(
    intent: &CreativeIntent,
    style: &StyleProfile,
    art: Option<ArtMode>,
    canvas: Option<(u32, u32)>,
    speech: Option<SpeechMap>,
) -> (MotionProject, Vec<CompileWarning>) {
    let opts = CompileOptions {
        art,
        canvas,
        speech,
        ..CompileOptions::default()
    };
    compile_with_report(
        intent,
        style,
        None,
        &library(),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

/// The cinematic look (`--art auto` with the cinematic tone), with or without
/// the synthetic voice-over.
fn cinematic(intent: &CreativeIntent, voiced: bool) -> (MotionProject, Vec<CompileWarning>) {
    compile(
        intent,
        &style_for("cinematic"),
        Some(ArtMode::Auto),
        voiced.then(|| speech_for(intent)),
    )
}

/// `beat N: message` of every `value_dropped` warning.
fn dropped(w: &[CompileWarning]) -> Vec<String> {
    w.iter()
        .filter(|w| w.code == WARN_VALUE_DROPPED)
        .map(|w| format!("beat {}: {}", w.beat.map_or(0, |b| b + 1), w.message))
        .collect()
}

/// The check on a (possibly edited) project.
fn check(intent: &CreativeIntent, p: &MotionProject) -> Vec<String> {
    dropped(&value_coverage::check(intent, p))
}

// ---------------------------------------------------------------------------
// Editing a compiled project
// ---------------------------------------------------------------------------

/// Lowercase text with every run of whitespace (a wrapped figure has a
/// newline) as one space.
fn norm(t: &str) -> String {
    t.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Remove every text layer of `layers` (at any depth) whose text contains
/// `needle` (case-insensitive); returns how many went.
fn remove_texts(layers: &mut Vec<Layer>, needle: &str) -> usize {
    let needle = norm(needle);
    let before = layers.len();
    layers.retain(|l| !matches!(&l.kind, LayerKind::Text(t) if norm(&t.text).contains(&needle)));
    let mut n = before - layers.len();
    for l in layers.iter_mut() {
        if let LayerKind::Group { children } = &mut l.kind {
            n += remove_texts(children, &needle);
        }
    }
    n
}

/// Replace the text of every layer containing `from` (case-insensitive).
fn replace_texts(layers: &mut [Layer], from: &str, to: &str) -> usize {
    let from = norm(from);
    let mut n = 0;
    for l in layers.iter_mut() {
        match &mut l.kind {
            LayerKind::Text(t) if norm(&t.text).contains(&from) => {
                t.text = to.to_string();
                n += 1;
            }
            LayerKind::Group { children } => n += replace_texts(children, &from, to),
            _ => {}
        }
    }
    n
}

fn find_text<'a>(layers: &'a [Layer], needle: &str) -> Option<&'a Layer> {
    let needle = norm(needle);
    for l in layers {
        match &l.kind {
            LayerKind::Text(t) if norm(&t.text).contains(&needle) => return Some(l),
            LayerKind::Group { children } => {
                if let Some(found) = find_text(children, &needle) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn scene_mut<'a>(p: &'a mut MotionProject, id: &str) -> &'a mut Scene {
    p.scenes
        .iter_mut()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene {id}"))
}

fn scene<'a>(p: &'a MotionProject, id: &str) -> &'a Scene {
    p.scenes
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("scene {id}"))
}

// ---------------------------------------------------------------------------
// Readability on the resolved frames (the test's own copy of the rule)
// ---------------------------------------------------------------------------

/// The texts of `scene`'s text leaves that are readable at absolute time `t`:
/// compounded opacity and every glyph >= 0.5, blur within the readable limit
/// (2 px at 1080, or 12 % of the font size), box on the canvas.
fn readable_at(p: &MotionProject, scene: &str, t: f64) -> Vec<String> {
    fn walk(
        l: &ResolvedLayer<'_>,
        opacity: f32,
        blur_sq: f32,
        canvas: (f32, f32),
        out: &mut Vec<String>,
    ) {
        let opacity = opacity * l.opacity.clamp(0.0, 1.0);
        let blur = l.blur.unwrap_or(0.0).max(0.0);
        let blur_sq = blur_sq + blur * blur;
        if let LayerKind::Group { .. } = l.kind {
            for c in &l.children {
                walk(c, opacity, blur_sq, canvas, out);
            }
            return;
        }
        let LayerKind::Text(style) = l.kind else {
            return;
        };
        let glyph = l
            .glyphs
            .as_ref()
            .and_then(|g| g.iter().map(|p| p.opacity.clamp(0.0, 1.0)).reduce(f32::min))
            .unwrap_or(1.0);
        let limit =
            (READABLE_BLUR_PX * canvas.0.min(canvas.1) / 1080.0).max(0.12 * style.font_size);
        if opacity * glyph < READABLE_OPACITY || blur_sq.sqrt() > limit {
            return;
        }
        let corners = [
            (0.0, 0.0),
            (l.width, 0.0),
            (l.width, l.height),
            (0.0, l.height),
        ];
        let mapped: Vec<(f32, f32)> = corners
            .iter()
            .filter_map(|&(x, y)| match &l.projective {
                Some(m) => {
                    let d = m[6] * x + m[7] * y + m[8];
                    (d > 1e-6).then(|| {
                        (
                            (m[0] * x + m[1] * y + m[2]) / d,
                            (m[3] * x + m[4] * y + m[5]) / d,
                        )
                    })
                }
                None => Some(l.transform.apply(x, y)),
            })
            .collect();
        let on = !mapped.is_empty()
            && mapped.iter().map(|c| c.0).fold(f32::INFINITY, f32::min) < canvas.0
            && mapped.iter().map(|c| c.0).fold(f32::NEG_INFINITY, f32::max) > 0.0
            && mapped.iter().map(|c| c.1).fold(f32::INFINITY, f32::min) < canvas.1
            && mapped.iter().map(|c| c.1).fold(f32::NEG_INFINITY, f32::max) > 0.0;
        if on {
            out.push(l.text.clone().unwrap_or_else(|| style.text.clone()));
        }
    }
    let frame = (t * f64::from(p.canvas.fps)).round() as u32;
    let resolved = evaluate_frame(p, frame).expect("frame");
    let canvas = (p.canvas.width as f32, p.canvas.height as f32);
    let mut out = Vec::new();
    for l in &resolved.layers {
        if l.scene == Some(scene) {
            walk(l, 1.0, 0.0, canvas, &mut out);
        }
    }
    out
}

fn shows(texts: &[String], needle: &str) -> bool {
    let needle = norm(needle);
    texts.iter().any(|t| norm(t).contains(&needle))
}

/// Whether `needle` is readable at some frame sampled every 0.1 s from the
/// scene's ENTER to its ANTICIPATE.
fn shown_between_enter_and_anticipate(p: &MotionProject, scene_id: &str, needle: &str) -> bool {
    let s = scene(p, scene_id);
    let life = s.lifecycle.expect("lifecycle");
    let (t0, t1) = (
        s.start_seconds + life.enter,
        s.start_seconds + life.anticipate,
    );
    let mut k = 0;
    loop {
        let t = t0 + 0.1 * f64::from(k);
        if t > t1 + 1e-9 {
            return false;
        }
        if shows(&readable_at(p, scene_id, t), needle) {
            return true;
        }
        k += 1;
    }
}

// ---------------------------------------------------------------------------
// The check itself
// ---------------------------------------------------------------------------

#[test]
fn the_warning_fires_when_the_second_value_layer_is_removed() {
    for (name, intent, first, second) in [
        ("sleep", sleep(), "7 hours", "5 hours"),
        ("money", money(), "$240,000", "$120,000"),
    ] {
        for voiced in [false, true] {
            let (p, w) = cinematic(&intent, voiced);
            assert!(
                dropped(&w).is_empty(),
                "{name} voice={voiced}: the compile itself shows both: {:?}",
                dropped(&w)
            );
            assert_eq!(check(&intent, &p), Vec::<String>::new());
            // The second value's text layer is taken away.
            let mut q = p.clone();
            let gone = remove_texts(&mut scene_mut(&mut q, "beat_3").layers, second);
            assert!(gone > 0, "{name}: no layer showed {second}");
            assert_eq!(
                check(&intent, &q),
                vec![format!("beat 3: \"{second}\" (compare side) is not shown")],
                "{name} voice={voiced}"
            );
            // The first one likewise.
            let mut q = p.clone();
            remove_texts(&mut scene_mut(&mut q, "beat_3").layers, first);
            assert_eq!(
                check(&intent, &q),
                vec![format!("beat 3: \"{first}\" (compare side) is not shown")]
            );
            // Both: two warnings, in the intent's order.
            let mut q = p.clone();
            remove_texts(&mut scene_mut(&mut q, "beat_3").layers, first);
            remove_texts(&mut scene_mut(&mut q, "beat_3").layers, second);
            assert_eq!(check(&intent, &q).len(), 2);
        }
    }
}

#[test]
fn the_warning_is_the_compile_warning_with_the_documented_code() {
    let intent = sleep();
    let (p, _) = cinematic(&intent, false);
    let mut q = p.clone();
    remove_texts(&mut scene_mut(&mut q, "beat_3").layers, "5 hours");
    let w = value_coverage::check(&intent, &q);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].code, "value_dropped");
    assert_eq!(w[0].code, WARN_VALUE_DROPPED);
    assert_eq!(w[0].beat, Some(2), "0-based; the CLI prints beat 3");
    assert_eq!(w[0].message, "\"5 hours\" (compare side) is not shown");
}

#[test]
fn a_number_shown_only_in_its_compact_form_is_matched() {
    let intent = money();
    let (p, _) = cinematic(&intent, false);
    // "$240,000" written "$240K" by the engine's compact form: still shown.
    let mut q = p.clone();
    let n = replace_texts(&mut scene_mut(&mut q, "beat_3").layers, "$240,000", "$240K");
    assert!(n > 0);
    assert_eq!(check(&intent, &q), Vec::<String>::new());
    // The digit sequence without its separators and symbol.
    let mut q = p.clone();
    replace_texts(
        &mut scene_mut(&mut q, "beat_3").layers,
        "$240,000",
        "240000",
    );
    assert_eq!(check(&intent, &q), Vec::<String>::new());
    // A different quantity is not.
    let mut q = p.clone();
    replace_texts(&mut scene_mut(&mut q, "beat_3").layers, "$240,000", "$250K");
    assert_eq!(
        check(&intent, &q),
        vec!["beat 3: \"$240,000\" (compare side) is not shown".to_string()]
    );
    // A figure the narration says in words ("fifty") shows the value "50".
    let intent = from_json(
        r#"{"version":"0.2","title":"t","format":"vertical","beats":[
        {"purpose":"reveal","statement":"Fifty","primary":{"kind":"number","value":"50 L","meaning":"tank"}}]}"#,
    );
    let (mut p, _) = compile(&intent, &StyleProfile::default(), None, None);
    assert!(check(&intent, &p).is_empty());
    replace_texts(
        &mut scene_mut(&mut p, "beat_1").layers,
        "50",
        "FIFTY LITRES",
    );
    assert!(check(&intent, &p).is_empty(), "said in words");
}

#[test]
fn the_decimal_point_keeps_its_place_on_a_compiled_project() {
    // "4.1%" is not "41%", "$0.14" is not "14" and "0.5" is not "5".
    let intent = from_json(&stat_pair("compare", "separate"));
    let (p, _) = compile(
        &intent,
        &StyleProfile::default(),
        Some(ArtMode::Force(Look::StreetCollage)),
        None,
    );
    assert!(check(&intent, &p).is_empty());
    let mut q = p.clone();
    assert!(replace_texts(&mut scene_mut(&mut q, "beat_1").layers, "4.1%", "41%") > 0);
    assert_eq!(
        check(&intent, &q),
        vec!["beat 1: \"4.1%\" (compare side) is not shown".to_string()]
    );
    // The same figure with a decimal comma or a trailing zero is the same number.
    let mut q = p.clone();
    replace_texts(&mut scene_mut(&mut q, "beat_1").layers, "4.1%", "4,10%");
    assert!(check(&intent, &q).is_empty());

    let intent = from_json(
        r#"{"version":"0.2","title":"tax","format":"vertical","beats":[
        {"purpose":"reveal","statement":"The US tax","primary":{"kind":"number","value":"$0.14","meaning":"per litre"}},
        {"purpose":"reveal","statement":"Half","primary":{"kind":"number","value":"0.5","meaning":"share"}}]}"#,
    );
    let (p, _) = compile(&intent, &StyleProfile::default(), None, None);
    assert!(check(&intent, &p).is_empty());
    for (beat, value, wrong) in [(1, "$0.14", "14"), (2, "0.5", "5")] {
        let mut q = p.clone();
        let scene = scene_mut(&mut q, &format!("beat_{beat}"));
        // A counting text is judged on its final value: take the count away.
        scene
            .motions
            .retain(|m| !matches!(m.op, MotionOp::Count { .. }));
        let n = replace_texts(&mut scene.layers, value, wrong);
        assert!(n > 0, "no layer showed {value}");
        assert_eq!(
            check(&intent, &q),
            vec![format!("beat {beat}: \"{value}\" (value) is not shown")],
            "{value} must not match {wrong}"
        );
    }
}

#[test]
fn indian_grouping_matches_the_same_amount_in_other_grouping() {
    // "₹1,20,000" (lakh grouping) is 120000: shown as "₹120,000" it is
    // shown; read as 1.2 and 0 it would not be.
    let intent = from_json(
        r#"{"version":"0.2","title":"lakh","format":"vertical","beats":[
        {"purpose":"reveal","statement":"The salary","primary":{"kind":"number","value":"₹1,20,000","meaning":"a year"}}]}"#,
    );
    let (p, w) = compile(&intent, &StyleProfile::default(), None, None);
    assert!(dropped(&w).is_empty(), "{:?}", dropped(&w));
    let edit = |to: &str| {
        let mut q = p.clone();
        let scene = scene_mut(&mut q, "beat_1");
        scene
            .motions
            .retain(|m| !matches!(m.op, MotionOp::Count { .. }));
        assert!(
            replace_texts(&mut scene.layers, "1,20,000", to) > 0,
            "a layer showed it"
        );
        check(&intent, &q)
    };
    assert!(edit("₹120,000").is_empty());
    assert!(edit("120000").is_empty());
    assert_eq!(
        edit("₹1.2"),
        vec!["beat 1: \"₹1,20,000\" (value) is not shown".to_string()]
    );
}

#[test]
fn a_value_readable_only_after_anticipate_is_not_shown() {
    let intent = sleep();
    let (p, _) = cinematic(&intent, false);
    let life = scene(&p, "beat_3").lifecycle.expect("lifecycle");
    let late = |start: f64| {
        let mut q = p.clone();
        let s = scene_mut(&mut q, "beat_3");
        let mut moved = 0;
        for m in s
            .motions
            .iter_mut()
            .filter(|m| m.target.ends_with(".fig.1"))
        {
            if matches!(m.op, MotionOp::GlyphCascade { .. }) {
                m.start = start;
                moved += 1;
            }
        }
        assert_eq!(moved, 1, "the second figure enters with one cascade");
        check(&intent, &q)
    };
    // After ANTICIPATE: not shown.
    assert_eq!(
        late(life.anticipate + 0.2),
        vec!["beat 3: \"5 hours\" (compare side) is not shown".to_string()]
    );
    // Just as ANTICIPATE starts the cascade has not landed (glyphs below
    // half opacity): not shown either.
    assert_eq!(late(life.anticipate).len(), 1);
    // Landed a second before ANTICIPATE: shown.
    assert!(late(life.anticipate - 1.5).is_empty());
}

#[test]
fn caption_backdrop_and_decorative_text_never_counts() {
    let intent = sleep();
    let (p, _) = cinematic(&intent, true);
    assert!(
        p.scenes.iter().any(|s| s.id == "captions"),
        "a voice-over compile has the caption scene"
    );
    let mut base = p.clone();
    let template = find_text(&scene(&base, "beat_3").layers, "5 hours")
        .expect("the second figure")
        .clone();
    remove_texts(&mut scene_mut(&mut base, "beat_3").layers, "5 hours");
    assert_eq!(check(&intent, &base).len(), 1);
    // The same text, fully visible for the whole beat, in the caption scene,
    // the backdrop scene or a decorative layer of the beat: still dropped.
    let mut shown = template.clone();
    shown.id = "b3.extra".to_string();
    shown.opacity = 1.0;
    let beat3 = scene(&base, "beat_3").clone();
    for target in ["captions", "backdrop"] {
        let mut q = base.clone();
        let mut l = shown.clone();
        l.id = format!("{target}.extra");
        let s = scene_mut(&mut q, target);
        s.start_seconds = beat3.start_seconds;
        s.duration_seconds = beat3.duration_seconds;
        s.layers.push(l);
        assert_eq!(check(&intent, &q).len(), 1, "{target} text does not count");
    }
    let mut q = base.clone();
    let mut l = shown.clone();
    l.id = "b3.ghost_word".to_string();
    scene_mut(&mut q, "beat_3").layers.push(l);
    assert_eq!(check(&intent, &q).len(), 1, "a ghost word does not count");
    // Control: the same layer as a content layer of the beat counts.
    let mut q = base.clone();
    let mut l = shown.clone();
    l.id = "b3.extra_figure".to_string();
    scene_mut(&mut q, "beat_3").layers.push(l);
    assert!(
        check(&intent, &q).is_empty(),
        "the control layer is readable"
    );
}

#[test]
fn a_value_on_a_deeply_blurred_plane_is_not_shown() {
    // The cinematic hero's value card (the figure in 75 px type) on the focus
    // plane is sharp; pushed far from the camera's focus, depth of field
    // blurs it by 24 px, far more than the readable limit (2 px, or 12 % of
    // the type size: 9 px here). Its text is still in the scene: nobody can
    // read it.
    let intent = from_json(OBJECT_VALUE);
    let (p, _) = cinematic(&intent, false);
    assert!(check(&intent, &p).is_empty(), "sharp on the focus plane");
    let set_z = |z: f32| {
        let mut q = p.clone();
        let wrapper = scene_mut(&mut q, "beat_1")
            .layers
            .iter_mut()
            .find(|l| l.id.ends_with(".hero_value.depth"))
            .expect("the value card's depth wrapper");
        wrapper.z = Some(z);
        q
    };
    assert_eq!(
        check(&intent, &set_z(6000.0)),
        vec!["beat 1: \"$58\" (value) is not shown".to_string()]
    );
    // On the focus plane it is shown (the limit is a blur radius, not a
    // depth): the check only moved the layer.
    assert!(check(&intent, &set_z(0.0)).is_empty());
    // A big headline stays legible under more blur than small type: the
    // second figure of a comparison (259 px type, 12 % = 31 px) at the same
    // 24 px blur still counts.
    let intent = sleep();
    let (p, _) = cinematic(&intent, false);
    let mut q = p.clone();
    let wrapper = scene_mut(&mut q, "beat_3")
        .layers
        .iter_mut()
        .find(|l| l.id.ends_with(".fig.1.depth"))
        .expect("the second figure's depth wrapper");
    wrapper.z = Some(6000.0);
    assert!(check(&intent, &q).is_empty());
}

#[test]
fn a_counting_text_matches_on_its_final_value() {
    let intent = sleep();
    // Beat 2's "60%" counts in the data looks.
    let (p, _) = compile(&intent, &StyleProfile::default(), None, None);
    let s = scene(&p, "beat_2");
    let counting = s
        .motions
        .iter()
        .any(|m| matches!(&m.op, MotionOp::Count { to, .. } if (*to - 60.0).abs() < 1e-9));
    assert!(counting, "beat 2 counts up to 60");
    assert!(
        check(&intent, &p).is_empty(),
        "{:?}: the text is mid-count at most frames, its final value is 60%",
        check(&intent, &p)
    );
    // Without the count, a layer that says 0% shows nothing of 60%.
    let mut q = p.clone();
    let s = scene_mut(&mut q, "beat_2");
    s.motions
        .retain(|m| !matches!(m.op, MotionOp::Count { .. }));
    let n = replace_texts(&mut s.layers, "%", "0%");
    assert!(n > 0);
    assert_eq!(
        check(&intent, &q),
        vec!["beat 2: \"60%\" (value) is not shown".to_string()]
    );
}

#[test]
fn every_role_of_a_value_is_named_in_the_message() {
    // collection items, a derived result and a state change's from / to.
    let intent = from_json(
        r#"{"version":"0.2","title":"roles","format":"vertical","beats":[
        {"purpose":"explain","statement":"Four things","primary":{"kind":"collection","meaning":"things","items":[
          {"kind":"phrase","value":"Memory"},{"kind":"phrase","value":"Mood"},{"kind":"phrase","value":"Focus"}]}},
        {"purpose":"reveal","statement":"The rate","primary":{"kind":"derived_metric","numerator":{"value":80,"meaning":"sign-ups"},
          "denominator":{"value":1000,"meaning":"visitors"},"meaning":"sign-up rate"}},
        {"purpose":"emphasize","statement":"Faster","primary":{"kind":"state_change","entity":"delivery time","from":"4 hours","to":"90 minutes"}}
        ]}"#,
    );
    let (p, w) = compile(&intent, &StyleProfile::default(), None, None);
    assert!(dropped(&w).is_empty(), "{:?}", dropped(&w));
    for (beat, needle, message) in [
        (1, "Mood", "beat 1: \"Mood\" (list item) is not shown"),
        (2, "%", "beat 2: \"8%\" (result) is not shown"),
        (3, "4", "beat 3: \"4 hours\" (from) is not shown"),
        (3, "90", "beat 3: \"90 minutes\" (to) is not shown"),
    ] {
        let mut q = p.clone();
        let n = remove_texts(
            &mut scene_mut(&mut q, &format!("beat_{beat}")).layers,
            needle,
        );
        assert!(n > 0, "no layer showed {needle}");
        let got = check(&intent, &q);
        assert!(got.contains(&message.to_string()), "{needle}: {got:?}");
    }
}

#[test]
fn phrase_primaries_and_lone_pictures_are_not_required() {
    // An emphasize beat's phrase and a picture's meaning need not appear.
    let intent = from_json(
        r#"{"version":"0.2","title":"free","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"Hello","primary":{"kind":"phrase","value":"Maintenance","meaning":"sleep"}},
        {"purpose":"emphasize","statement":"Brain","primary":{"kind":"object","asset":"brain","meaning":"the brain"}}]}"#,
    );
    let (mut p, _) = compile(&intent, &StyleProfile::default(), None, None);
    for id in ["beat_1", "beat_2"] {
        remove_texts(&mut scene_mut(&mut p, id).layers, "maintenance");
        remove_texts(&mut scene_mut(&mut p, id).layers, "brain");
    }
    assert!(check(&intent, &p).is_empty());
}

/// Compile with a library that has no asset family and no art look: the plain
/// `compile` of the CLI.
fn compile_plain(intent: &CreativeIntent) -> (MotionProject, Vec<CompileWarning>) {
    compile_with_report(
        intent,
        &StyleProfile::default(),
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .expect("compile")
}

#[test]
fn a_value_lost_with_a_missing_picture_is_reported() {
    // An object naming an asset that exists nowhere is degraded to a phrase
    // (its meaning), and its value goes with it: the check judges the intent
    // the author wrote, not the degraded one.
    let intent = from_json(
        r#"{"version":"0.2","title":"miss","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"Savings grow",
         "primary":{"kind":"object","asset":"golden_unicorn","value":"$58","meaning":"savings"}}]}"#,
    );
    let (p, w) = compile_plain(&intent);
    assert!(find_text(&scene(&p, "beat_1").layers, "$58").is_none());
    assert_eq!(
        dropped(&w),
        vec!["beat 1: \"$58\" (value) is not shown".to_string()]
    );
    // Both sides of a pair, the first picture missing: its figure is the one
    // reported (the second stays shown).
    let intent = from_json(
        r#"{"version":"0.2","title":"miss2","format":"vertical","beats":[
        {"purpose":"compare","statement":"Two",
         "primary":{"kind":"object","asset":"golden_unicorn","value":"$58","meaning":"savings"},
         "secondary":{"kind":"number","value":"$127","meaning":"groceries"}}]}"#,
    );
    let (_, w) = compile_plain(&intent);
    assert!(
        dropped(&w).contains(&"beat 1: \"$58\" (compare side) is not shown".to_string()),
        "{:?}",
        dropped(&w)
    );
    assert!(
        !dropped(&w).iter().any(|d| d.contains("$127")),
        "{:?}",
        dropped(&w)
    );
    // TWO_PRIZES with a library that has no families: the catalog picture of
    // the piggy bank does not exist there, so "$90" is lost; with its primary
    // a phrase the hero layout no longer stamps the second picture's "$12"
    // either. Both are reported.
    let intent = from_json(TWO_PRIZES);
    let (_, w) = compile_plain(&intent);
    let lost = dropped(&w);
    assert!(
        lost.contains(&"beat 1: \"$90\" (value) is not shown".to_string()),
        "{lost:?}"
    );
    assert!(
        lost.iter().all(|d| d.ends_with("(value) is not shown")),
        "{lost:?}"
    );
    // With the families the same story shows both.
    let (_, w) = compile(&intent, &StyleProfile::default(), None, None);
    assert!(dropped(&w).is_empty());
}

#[test]
fn the_check_is_deterministic() {
    let intent = sleep();
    let (p, _) = cinematic(&intent, true);
    let mut q = p.clone();
    remove_texts(&mut scene_mut(&mut q, "beat_3").layers, "5 hours");
    assert_eq!(check(&intent, &q), check(&intent, &q));
}

// ---------------------------------------------------------------------------
// The cinematic look shows what it is given
// ---------------------------------------------------------------------------

#[test]
fn cinematic_compare_shows_both_values_readable_at_read() {
    for (name, intent, a, b) in [
        ("sleep", sleep(), "7 hours", "5 hours"),
        ("money", money(), "$240,000", "$120,000"),
    ] {
        // Without a voice-over both figures (and the VS connector) have
        // landed by READ, where layout QA looks.
        let (p, w) = cinematic(&intent, false);
        assert!(dropped(&w).is_empty(), "{name}: {:?}", dropped(&w));
        let s = scene(&p, "beat_3");
        let life = s.lifecycle.expect("lifecycle");
        let at_read = readable_at(&p, "beat_3", s.start_seconds + life.read);
        assert!(shows(&at_read, a), "{name}: {a} at READ in {at_read:?}");
        assert!(shows(&at_read, b), "{name}: {b} at READ in {at_read:?}");
        assert!(shown_between_enter_and_anticipate(&p, "beat_3", a));
        assert!(shown_between_enter_and_anticipate(&p, "beat_3", b));
        // Both figures are on the focus plane, with their meanings as labels.
        let labels: Vec<String> = at_read.iter().map(|t| norm(t.as_str())).collect();
        let meanings = match name {
            "sleep" => ["needed", "typical"],
            _ => ["start at 25", "start at 35"],
        };
        for m in meanings {
            assert!(
                labels.iter().any(|t| t == m),
                "{name}: label {m} in {labels:?}"
            );
        }

        // With one, the second arrives as the narrator says it: readable
        // before ANTICIPATE, together with the first.
        let (p, w) = cinematic(&intent, true);
        assert!(dropped(&w).is_empty(), "{name} voiced: {:?}", dropped(&w));
        let s = scene(&p, "beat_3");
        let life = s.lifecycle.expect("lifecycle");
        let end = readable_at(&p, "beat_3", s.start_seconds + life.anticipate - 0.05);
        assert!(
            shows(&end, a) && shows(&end, b),
            "{name}: both before ANTICIPATE in {end:?}"
        );
        assert!(shown_between_enter_and_anticipate(&p, "beat_3", b));
    }
}

#[test]
fn cinematic_compare_keeps_the_pictures_pair_and_other_beats_unchanged() {
    // The figure pair is the numbers' own stage: it leaves the pictures'
    // relation stage (`pair_*`) alone.
    let (p, _) = cinematic(&sleep(), false);
    let ids = |s: &Scene| -> Vec<String> {
        fn walk(ls: &[Layer], out: &mut Vec<String>) {
            for l in ls {
                out.push(l.id.clone());
                if let LayerKind::Group { children } = &l.kind {
                    walk(children, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&s.layers, &mut out);
        out
    };
    let b3 = ids(scene(&p, "beat_3"));
    assert!(b3.iter().any(|i| i.ends_with(".fig.0")) && b3.iter().any(|i| i.ends_with(".fig.1")));
    assert!(!b3.iter().any(|i| i.contains("pair_")), "{b3:?}");
}

const OBJECT_VALUE: &str = r#"{
  "version": "0.2", "title": "single", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Savings grow",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "energy": "impact", "keyword": "grow"}
  ]
}"#;

#[test]
fn a_cinematic_object_with_a_value_shows_the_value_at_read() {
    let intent = from_json(OBJECT_VALUE);
    for voiced in [false, true] {
        let (p, w) = cinematic(&intent, voiced);
        assert!(dropped(&w).is_empty(), "voice={voiced}: {:?}", dropped(&w));
        let s = scene(&p, "beat_1");
        let life = s.lifecycle.expect("lifecycle");
        let at_read = readable_at(&p, "beat_1", s.start_seconds + life.read);
        assert!(
            shows(&at_read, "$58"),
            "voice={voiced}: $58 at READ in {at_read:?}"
        );
        // The picture is still there: the value sits on a card under it.
        assert!(find_text(&s.layers, "$58").is_some());
        assert!(
            s.layers
                .iter()
                .any(|l| matches!(l.kind, LayerKind::Group { .. })),
            "the staged scene"
        );
    }
    // The hero's card, a supporting picture's card (value over meaning) and a
    // figure beside the hero all show.
    let intent = from_json(
        r#"{"version":"0.2","title":"two","format":"vertical","beats":[
        {"purpose":"emphasize","statement":"Two prizes",
         "narration":"The piggy bank holds ninety dollars, and the shopping basket twelve dollars.",
         "primary":{"kind":"object","asset":"piggy_bank","value":"$90","meaning":"savings"},
         "secondary":{"kind":"object","asset":"shopping_basket","value":"$12","meaning":"groceries"}},
        {"purpose":"emphasize","statement":"A bank and a rate",
         "narration":"A piggy bank earns four percent interest if you leave it alone for a year.",
         "primary":{"kind":"object","asset":"piggy_bank","meaning":"savings"},
         "secondary":{"kind":"number","value":"4%","meaning":"interest"}},
        {"purpose":"reveal","statement":"Fifty litres",
         "narration":"Fifty litres of fuel fill the tank, and that costs seven dollars here.",
         "primary":{"kind":"number","value":"50 L","meaning":"same tank"},
         "secondary":{"kind":"object","asset":"piggy_bank","value":"$7","meaning":"cost"}}
        ]}"#,
    );
    for voiced in [false, true] {
        let (p, w) = cinematic(&intent, voiced);
        assert!(dropped(&w).is_empty(), "voice={voiced}: {:?}", dropped(&w));
        for (beat, needles) in [
            (1, vec!["$90", "$12"]),
            (2, vec!["4%"]),
            (3, vec!["50 L", "$7"]),
        ] {
            let id = format!("beat_{beat}");
            for n in needles {
                assert!(
                    shown_between_enter_and_anticipate(&p, &id, n),
                    "voice={voiced} beat {beat}: {n}"
                );
            }
        }
    }
}

#[test]
fn a_cinematic_collection_shows_the_total_it_adds_up_to() {
    let intent = read("examples/public/collection-accumulate.intent.json");
    for voiced in [false, true] {
        let (p, w) = cinematic(&intent, voiced);
        assert!(dropped(&w).is_empty(), "voice={voiced}: {:?}", dropped(&w));
        assert!(shown_between_enter_and_anticipate(&p, "beat_1", "9 kg"));
        for item in ["Water bottle", "Jacket", "Laptop", "Books"] {
            assert!(
                shown_between_enter_and_anticipate(&p, "beat_1", &item.replace(' ', "\n")),
                "{item}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// No look drops a value (and the check has no false positives)
// ---------------------------------------------------------------------------

/// Two pictures with values compared, then a figure against a picture with
/// its figure (genre_values `PAIR`).
const PAIR: &str = r#"{
  "version": "0.2", "title": "pair", "format": "vertical",
  "beats": [
    {"purpose": "compare", "statement": "Piggy bank versus basket",
     "narration": "The piggy bank holds 58 dollars. The shopping basket costs 127 dollars.",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "value": "$127", "meaning": "groceries"},
     "relationship": "separate", "energy": "impact", "keyword": "price"},
    {"purpose": "contrast", "statement": "Salary against groceries",
     "narration": "Your salary is flat. Groceries rose 38 percent.",
     "primary": {"kind": "number", "value": "₹50,000", "meaning": "salary"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "value": "+38%", "meaning": "groceries"},
     "energy": "impact", "keyword": "groceries"}
  ]
}"#;

/// Two objects with values in an emphasize beat (genre_values hero stamps).
const TWO_PRIZES: &str = r#"{
  "version": "0.2", "title": "two", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Two prizes",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$90", "meaning": "savings"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "value": "$12", "meaning": "groceries"},
     "energy": "building"}
  ]
}"#;

/// A figure with a picture of its own (genre_values `STAMPS`).
const STAMPS: &str = r#"{
  "version": "0.2", "title": "stamps", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Fifty litres",
     "narration": "Fifty litres of fuel fill the same tank.",
     "primary": {"kind": "number", "value": "50 L", "meaning": "same tank"},
     "secondary": {"kind": "object", "asset": "piggy_bank", "meaning": "piggy bank"},
     "energy": "impact", "keyword": "pump"},
    {"purpose": "reveal", "statement": "A forecast",
     "narration": "The bank warns the piggy bank could lose four percent.",
     "primary": {"kind": "number", "value": "4%", "meaning": "forecast"},
     "secondary": {"kind": "object", "asset": "piggy_bank", "meaning": "piggy bank"},
     "energy": "impact", "keyword": "warning"}
  ]
}"#;

/// The Wallet Atlas fuel story (genre_values `unnamed_picture`).
const FUEL: &str = r#"{
  "version": "0.2", "title": "fuel", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Same tank. Five prices.",
     "narration": "Fifty litres of fuel. Same tank. Five countries. The price difference will surprise you.",
     "primary": {"kind": "number", "value": "50 L", "meaning": "same tank"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "meaning": "United States"},
     "energy": "impact", "keyword": "fuel"},
    {"purpose": "compare", "statement": "US vs Germany",
     "narration": "In the United States that tank costs just $58. But in Germany the same fill-up runs $127.",
     "primary": {"kind": "object", "asset": "shopping_basket", "value": "$58", "meaning": "United States"},
     "secondary": {"kind": "object", "asset": "piggy_bank", "value": "$127", "meaning": "Germany"},
     "relationship": "separate", "energy": "impact", "keyword": "price"},
    {"purpose": "contrast", "statement": "Tax is the gap",
     "narration": "Germany adds over $1.15 of tax per litre. The US adds just $0.14.",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$1.15 / L", "meaning": "Germany's fuel tax"},
     "secondary": {"kind": "object", "asset": "shopping_basket", "value": "$0.14 / L", "meaning": "US fuel tax"},
     "relationship": "compress", "energy": "impact", "keyword": "tax"}
  ]
}"#;

/// stat_values' two catalog pictures with figures, per purpose.
fn stat_pair(purpose: &str, relationship: &str) -> String {
    format!(
        r#"{{"version":"0.2","title":"pair","format":"vertical","beats":[{{
            "purpose":"{purpose}","statement":"Savings against spending",
            "primary":{{"kind":"object","asset":"coin_stack","value":"4.1%","meaning":"Savings"}},
            "secondary":{{"kind":"object","asset":"laptop","value":"3.1%","meaning":"Spending"}},
            "relationship":"{relationship}"}}]}}"#
    )
}

/// Every art mode a story is judged under: each of the ten looks forced, no
/// look (the classic compile) and the look the tone picks (`auto`): twelve.
fn modes() -> Vec<(String, Option<ArtMode>)> {
    let mut v: Vec<(String, Option<ArtMode>)> = Look::ALL
        .iter()
        .map(|l| (l.name().to_string(), Some(ArtMode::Force(*l))))
        .collect();
    v.push(("classic".into(), None));
    v.push(("auto".into(), Some(ArtMode::Auto)));
    v
}

/// The tones a mode is compiled with: a forced look or the classic compile
/// does not depend on the tone; `auto` picks its look from it.
fn tones(mode: &str) -> &'static [&'static str] {
    if mode == "auto" {
        &[
            "auto",
            "cinematic",
            "documentary",
            "street",
            "studio",
            "hype",
        ]
    } else {
        &["auto"]
    }
}

/// Whether every beat has a narration (a voice-over reads the statement when
/// it has none, which gives a beat a few seconds that no figure was timed to).
fn narrated(intent: &CreativeIntent) -> bool {
    intent.beats.iter().all(|b| b.narration.is_some())
}

/// Zero `value_dropped` warnings for every story, in every mode, without a
/// voice-over and (for a narrated story) with one.
fn assert_no_drops(stories: &[(&str, CreativeIntent)]) {
    let mut bad = Vec::new();
    for (name, intent) in stories {
        for (mode, art) in modes() {
            for tone in tones(&mode) {
                for voiced in [false, true] {
                    if voiced && !narrated(intent) {
                        continue;
                    }
                    let (_, w) = compile(
                        intent,
                        &style_for(tone),
                        art,
                        voiced.then(|| speech_for(intent)),
                    );
                    for d in dropped(&w) {
                        bad.push(format!("{name} {mode} tone={tone} voice={voiced}: {d}"));
                    }
                }
            }
        }
    }
    assert!(bad.is_empty(), "values dropped:\n{}", bad.join("\n"));
}

#[test]
fn the_022_stories_show_every_value_in_every_look() {
    // stat_values.rs and genre_values.rs: no warning in any of the 12 looks
    // (the ten genre / art looks, the classic compile and `auto`).
    assert_no_drops(&[
        ("pair", from_json(PAIR)),
        ("single", from_json(OBJECT_VALUE)),
        ("two_prizes", from_json(TWO_PRIZES)),
        ("stamps", from_json(STAMPS)),
        ("fuel", from_json(FUEL)),
        ("stat_compare", from_json(&stat_pair("compare", "separate"))),
        (
            "stat_contrast",
            from_json(&stat_pair("contrast", "compress")),
        ),
    ]);
}

#[test]
fn the_benchmark_stories_show_every_value_in_every_look() {
    assert_no_drops(&[
        ("sleep_review", sleep()),
        ("money_review", money()),
        (
            "collection-accumulate",
            read("examples/public/collection-accumulate.intent.json"),
        ),
        (
            "state-change",
            read("examples/public/state-change.intent.json"),
        ),
        (
            "derived-metric",
            read("examples/public/derived-metric.intent.json"),
        ),
        ("layers", read("examples/public/layers.intent.json")),
        ("ai_learns", read("examples/topics/ai_learns.intent.json")),
        ("space", read("examples/cinematic/space.intent.json")),
    ]);
}

/// Every layer id of a scene (groups and their children).
fn layer_ids(s: &Scene) -> Vec<String> {
    fn walk(ls: &[Layer], out: &mut Vec<String>) {
        for l in ls {
            out.push(l.id.clone());
            if let LayerKind::Group { children } = &l.kind {
                walk(children, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(&s.layers, &mut out);
    out
}

#[test]
fn a_figure_beside_a_picture_stays_the_focal_layer_and_the_camera_pivots_on_the_middle() {
    let intent = from_json(STAMPS);
    for voiced in [false, true] {
        for (canvas, beside) in [
            ((1080, 1920), false),
            ((1080, 1080), true),
            ((1920, 1080), true),
        ] {
            let (p, w) = compile_on(
                &intent,
                &style_for("cinematic"),
                Some(ArtMode::Auto),
                Some(canvas),
                voiced.then(|| speech_for(&intent)),
            );
            assert!(dropped(&w).is_empty(), "{canvas:?}: {:?}", dropped(&w));
            let art = p.project.art.as_ref().expect("art record");
            for beat in 1..=2 {
                let id = format!("beat_{beat}");
                let focal = art.focal.get(&id).expect("focal").clone();
                let s = scene(&p, &id);
                let ids = layer_ids(s);
                assert!(
                    ids.contains(&focal),
                    "{canvas:?}: the focal layer {focal} is drawn"
                );
                // The focal layer is (or frames) the visible figure, never an
                // invisible node: it holds the text "50 L" / "4%".
                let figure = if beat == 1 { "50 L" } else { "4%" };
                let holder = fn_find_layer(&s.layers, &focal).expect("focal layer");
                assert!(
                    find_text(std::slice::from_ref(holder), figure).is_some(),
                    "{canvas:?} beat {beat}: the focal layer {focal} holds {figure}"
                );
                if beside {
                    assert_eq!(focal, format!("b{beat}.hero_word.frame"));
                    // The anchored group `hero_word` holds the frame, so the
                    // arrival-by-READ floor of the focal group guards the figure.
                    assert!(focal.starts_with(&format!("b{beat}.hero_word.")));
                    let pivot = s.camera.as_ref().and_then(|c| c.pivot).expect("pivot");
                    assert!(
                        (pivot[0] - canvas.0 as f32 / 2.0).abs() < 0.02 * canvas.0 as f32,
                        "{canvas:?}: pivot {pivot:?} on the middle of the composition"
                    );
                } else {
                    assert_eq!(focal, format!("b{beat}.hero_word"));
                }
                assert!(
                    !ids.iter().any(|i| i.ends_with(".pivot")),
                    "no invisible pivot node"
                );
            }
        }
    }
}

fn fn_find_layer<'a>(layers: &'a [Layer], id: &str) -> Option<&'a Layer> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let LayerKind::Group { children } = &l.kind {
            if let Some(f) = fn_find_layer(children, id) {
                return Some(f);
            }
        }
    }
    None
}

#[test]
fn two_pictures_with_figures_are_versus_only_in_a_comparison() {
    // TWO_PRIZES is an emphasize beat: street and studio show its two valued
    // pictures side by side, with no VS badge or divider; the compared pair
    // (PAIR beat 1) keeps its VS, and an emphasize beat that names a
    // relationship keeps that relationship's own connector.
    let has = |s: &Scene, part: &str| layer_ids(s).iter().any(|i| i.contains(part));
    for look in [Look::StreetCollage, Look::StudioPop] {
        let intent = from_json(TWO_PRIZES);
        let (p, _) = compile(
            &intent,
            &style_for("auto"),
            Some(ArtMode::Force(look)),
            None,
        );
        let s = scene(&p, "beat_1");
        assert!(has(s, "pair_label"), "{look:?}: the stat pair");
        assert!(
            !has(s, "pair_badge") && !has(s, "pair_divider"),
            "{look:?}: no VS"
        );
        assert!(
            !has(s, "pair_arrow") && !has(s, "pair_link"),
            "{look:?}: no connector"
        );

        let intent = from_json(PAIR);
        let (p, _) = compile(
            &intent,
            &style_for("auto"),
            Some(ArtMode::Force(look)),
            None,
        );
        assert!(
            has(scene(&p, "beat_1"), "pair_badge"),
            "{look:?}: compared pictures are VS"
        );

        let grows = TWO_PRIZES.replace(
            "\"energy\": \"building\"",
            "\"relationship\": \"grow\", \"energy\": \"building\"",
        );
        let intent = from_json(&grows);
        let (p, _) = compile(
            &intent,
            &style_for("auto"),
            Some(ArtMode::Force(look)),
            None,
        );
        let s = scene(&p, "beat_1");
        assert!(
            has(s, "pair_arrow") && !has(s, "pair_badge"),
            "{look:?}: its own connector"
        );
    }
}

#[test]
fn street_and_studio_show_both_figures_of_a_pair() {
    // The street / studio punchword shouts one figure: a beat with two keeps
    // a composition that shows both (genre_bias), and that composition passes
    // layout QA on every canvas.
    for look in [Look::StreetCollage, Look::StudioPop] {
        for story in [PAIR, TWO_PRIZES] {
            let intent = from_json(story);
            for canvas in CANVASES {
                for voiced in [false, true] {
                    let (p, w) = compile_on(
                        &intent,
                        &style_for("auto"),
                        Some(ArtMode::Force(look)),
                        Some(canvas),
                        voiced.then(|| speech_for(&intent)),
                    );
                    assert!(
                        dropped(&w).is_empty(),
                        "{look:?} {canvas:?} voice={voiced}: {:?}",
                        dropped(&w)
                    );
                    // Layout: strictly clean without a voice-over; with one,
                    // only what a4cd21b already failed (the studio look's
                    // kinetic words) may remain. A story with no narration
                    // read by a voice-over is the documented exception below.
                    if voiced && !narrated(&intent) {
                        continue;
                    }
                    let key = format!(
                        "{}|{}|{}x{}|{voiced}",
                        if story == PAIR { "pair" } else { "two_prizes" },
                        look.name(),
                        canvas.0,
                        canvas.1
                    );
                    let found = layout_findings(&p, &key);
                    let new: Vec<&String> = found
                        .iter()
                        .filter(|f| !LAYOUT_FAILS_AT_A4CD21B.contains(&f.as_str()))
                        .collect();
                    assert!(new.is_empty(), "new layout findings: {new:?}");
                    if !voiced {
                        assert!(found.is_empty(), "layout: {found:?}");
                    }
                }
            }
        }
    }
}

/// A narrated single object with a value the narrator never says.
const OBJECT_VALUE_NARRATED: &str = r#"{
  "version": "0.2", "title": "single", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Savings grow",
     "narration": "Put a little away every month and watch your savings grow.",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "energy": "impact", "keyword": "grow"}
  ]
}"#;

#[test]
fn studio_with_a_voice_over_shows_a_figure_the_narrator_never_says() {
    let intent = from_json(OBJECT_VALUE_NARRATED);
    let (_, w) = compile(
        &intent,
        &style_for("studio"),
        Some(ArtMode::Force(Look::StudioPop)),
        Some(speech_for(&intent)),
    );
    assert!(dropped(&w).is_empty(), "{:?}", dropped(&w));
}

#[test]
fn studio_kinetic_words_keep_a_stat_pair_inside_the_safe_area() {
    let intent = from_json(TWO_PRIZES);
    let (p, _) = compile_on(
        &intent,
        &style_for("auto"),
        Some(ArtMode::Force(Look::StudioPop)),
        Some((1080, 1920)),
        Some(speech_for(&intent)),
    );
    let found = layout_findings(&p, "two_prizes|studio_pop|1080x1920|true");
    assert!(found.is_empty(), "{found:?}");
}

// ---------------------------------------------------------------------------
// Layout QA: no new FAIL anywhere the looks changed
// ---------------------------------------------------------------------------

/// The canvases a layout verdict is taken on: tall, square and wide.
const CANVASES: [(u32, u32); 3] = [(1080, 1920), (1080, 1080), (1920, 1080)];

/// Layout findings of a compiled project (`<key>|<scene>:<layer>:<check>`).
fn layout_findings(p: &MotionProject, key: &str) -> Vec<String> {
    let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
    layout_report(p, &frame)
        .findings
        .iter()
        .map(|f| format!("{key}|{}:{}:{:?}", f.scene, f.layer, f.check))
        .collect()
}

/// Every layout finding the sweep below produced on a4cd21b (before this
/// task), in this test's own terms (`ApproxMeasure`, no image analysis). They
/// are listed so that they are not hidden: none is caused by A2-values (the
/// seven `pair ... beat_2:b2.stamp` ones are fixed by it). The sweep fails on
/// any finding that is not in the list.
const LAYOUT_FAILS_AT_A4CD21B: &[&str] = &[
    "fuel|auto|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|auto|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|auto|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|classical_neon|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|classical_neon|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|classical_neon|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|classic|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|classic|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|classic|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|clay_pop|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|clay_pop|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|clay_pop|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|dossier|1080x1080|true|beat_3:b3.figure.0.label:TextTooSmall",
    "fuel|halftone_cutout|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|halftone_cutout|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|halftone_cutout|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|hype_slam|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|hype_slam|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|hype_slam|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|journey|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|journey|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|journey|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|ornament_editorial|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|ornament_editorial|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|ornament_editorial|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|street_collage|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|street_collage|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|street_collage|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|studio_pop|1080x1080|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|studio_pop|1080x1080|true|beat_2:b2.kw.6:TextOutsideSafe",
    "fuel|studio_pop|1080x1080|true|beat_2:b2.kw.8:TextOutsideSafe",
    "fuel|studio_pop|1080x1080|true|beat_3:b3.kw.12:TextOutsideSafe",
    "fuel|studio_pop|1080x1080|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|studio_pop|1080x1920|false|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "fuel|studio_pop|1080x1920|true|beat_2:b2.kw.6:TextOutsideSafe",
    "fuel|studio_pop|1080x1920|true|beat_2:b2.kw.8:TextOutsideSafe",
    "fuel|studio_pop|1080x1920|true|beat_3:b3.kw.12:TextOutsideSafe",
    "fuel|studio_pop|1080x1920|true|beat_3:b3.pair_stamp.0:TextOutsideSafe",
    "money_review|studio_pop|1080x1080|true|beat_1:b1.kw.9:TextOutsideSafe",
    "money_review|studio_pop|1080x1080|true|beat_2:b2.kw.9:TextOutsideSafe",
    "money_review|studio_pop|1080x1080|true|beat_3:b3.kw.12:TextOutsideSafe",
    "money_review|studio_pop|1080x1080|true|beat_3:b3.kw.9:TextOutsideSafe",
    "money_review|studio_pop|1080x1920|true|beat_1:b1.kw.9:TextOutsideSafe",
    "money_review|studio_pop|1080x1920|true|beat_2:b2.kw.9:TextOutsideSafe",
    "money_review|studio_pop|1080x1920|true|beat_3:b3.kw.12:TextOutsideSafe",
    "pair|auto|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|cinematic_3d|1920x1080|false|beat_2:b2.title:TextClipped",
    "pair|classical_neon|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|classic|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|clay_pop|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|halftone_cutout|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|journey|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|ornament_editorial|1080x1920|false|beat_2:b2.stamp:TextOutsideSafe",
    "pair|studio_pop|1080x1080|true|beat_1:b1.kw.3:TextOutsideSafe",
    "pair|studio_pop|1080x1080|true|beat_1:b1.kw.5:TextOutsideSafe",
    "pair|studio_pop|1080x1080|true|beat_1:b1.kw.9:TextOutsideSafe",
    "pair|studio_pop|1080x1920|true|beat_1:b1.kw.3:TextOutsideSafe",
    "pair|studio_pop|1080x1920|true|beat_1:b1.kw.5:TextOutsideSafe",
    "pair|studio_pop|1080x1920|true|beat_1:b1.kw.9:TextOutsideSafe",
    "sleep_review|cinematic_3d|1920x1080|true|beat_2:b2.title:TextClipped",
    "sleep_review|studio_pop|1080x1080|true|beat_1:b1.kw.10:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1080|true|beat_2:b2.kw.9:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1080|true|beat_5:b5.kw.5:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1080|true|beat_5:b5.kw.7:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1920|true|beat_1:b1.kw.10:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1920|true|beat_2:b2.kw.9:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1920|true|beat_5:b5.kw.5:TextOutsideSafe",
    "sleep_review|studio_pop|1080x1920|true|beat_5:b5.kw.7:TextOutsideSafe",
];

#[test]
fn the_stories_add_no_layout_failure_in_any_look_and_canvas() {
    // The 0.22 stories in the ten looks, the classic compile and `auto`, the
    // review stories in the three looks this task touches: tall, square and
    // wide canvases, without a voice-over and (narrated stories) with one.
    let mut stories: Vec<(&str, CreativeIntent)> = vec![
        ("pair", from_json(PAIR)),
        ("single", from_json(OBJECT_VALUE)),
        ("two_prizes", from_json(TWO_PRIZES)),
        ("stamps", from_json(STAMPS)),
        ("fuel", from_json(FUEL)),
        ("stat_compare", from_json(&stat_pair("compare", "separate"))),
        (
            "stat_contrast",
            from_json(&stat_pair("contrast", "compress")),
        ),
    ];
    stories.push(("sleep_review", sleep()));
    stories.push(("money_review", money()));
    let mut found = std::collections::BTreeSet::new();
    let mut compiled = 0;
    for (name, intent) in &stories {
        let review = name.ends_with("_review");
        for (mode, art) in modes() {
            let touched = matches!(
                mode.as_str(),
                "cinematic_3d" | "street_collage" | "studio_pop"
            );
            if review && !touched {
                continue;
            }
            for canvas in CANVASES {
                for voiced in [false, true] {
                    if voiced && !narrated(intent) {
                        continue;
                    }
                    let (p, _) = compile_on(
                        intent,
                        &style_for("auto"),
                        art,
                        Some(canvas),
                        voiced.then(|| speech_for(intent)),
                    );
                    compiled += 1;
                    let key = format!("{name}|{mode}|{}x{}|{voiced}", canvas.0, canvas.1);
                    found.extend(layout_findings(&p, &key));
                }
            }
        }
    }
    assert!(compiled > 300, "{compiled} compiles");
    let new: Vec<&String> = found
        .iter()
        .filter(|f| !LAYOUT_FAILS_AT_A4CD21B.contains(&f.as_str()))
        .collect();
    assert!(
        new.is_empty(),
        "layout findings a4cd21b did not have:\n{}",
        {
            let lines: Vec<&str> = new.iter().map(|s| s.as_str()).collect();
            lines.join("\n")
        }
    );
}
