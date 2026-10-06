//! (0.20) Timing QA in `speech_report`: `speech_rate`, `reveal_before_speech`,
//! `reveal_late` and `spoken_mismatch` on the cinematic space story compiled
//! with a synthetic voice-over, plus the readable-time rule on hand-built
//! projects.

use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::MotionProject;
use motion_core::speech::{
    estimate_syllables, repair, statement_tokens, RecognisedWord, RevealAnchor, RevealRole,
    SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION,
};
use motion_core::style::StyleProfile;
use motion_render::reveal_qa::{first_readable_frames, GroupRef};
use motion_render::speech_qa::{
    speech_report, CheckStatus, RevealTiming, SpeechQaCheck, SpeechQaReport,
};
use serde_json::{json, Value};

const TIMING_CHECKS: [&str; 4] = [
    "speech_rate",
    "reveal_before_speech",
    "reveal_late",
    "spoken_mismatch",
];

// ---------------------------------------------------------------------------
// The space story with a synthetic voice-over
// ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// The example story. With `named`, each beat's narration also names the
/// pictures its builder anchors (hero, supporting picture and its label), so
/// every kind of anchored group is exercised; otherwise the example's own
/// narration (which names only the titles' words and Saturn).
fn story(named: bool) -> (CreativeIntent, StyleProfile) {
    let read = |p: &str| std::fs::read_to_string(repo().join(p)).expect("read example");
    let mut intent =
        CreativeIntent::from_json(&read("examples/cinematic/space.intent.json")).expect("intent");
    if named {
        let lines = [
            "Everything you have ever known happened on this earth globe, our home, circled by a lone satellite in orbit.",
            "Sunlight takes eight minutes to reach us, so you always see the sun as it was.",
            "Saturn is so light that, in a big enough ocean, it would actually float, though every atom has density.",
            "And we have only just started looking through the telescope at the stars. The universe is waiting.",
        ];
        for (beat, line) in intent.beats.iter_mut().zip(lines) {
            beat.narration = Some(line.to_string());
        }
    }
    let style = serde_json::from_str(&read("examples/cinematic/space.style.json")).expect("style");
    (intent, style)
}

fn spoken_lines(intent: &CreativeIntent) -> Vec<String> {
    intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect()
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Seconds between sentences: enough room for every anchored group of the
/// story to wait for its word (shorter pauses clamp the cues of anchors
/// spoken at the end of a beat, which QA then rightly reports).
const PAUSE: f64 = 1.5;

/// One sentence per beat read at about 5 syllables per second (0.17 s per
/// syllable, 0.06 s between words), [`PAUSE`] between sentences; repaired
/// against the spoken lines like the CLI's `load_speech`.
fn voice(intent: &CreativeIntent) -> SpeechMap {
    let lines = spoken_lines(intent);
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let mut t = 0.4;
    for (beat, line) in lines.iter().enumerate() {
        let start = t;
        for tok in statement_tokens(line) {
            let len = 0.17 * estimate_syllables(&tok) as f64;
            words.push(SpeechWord {
                text: tok,
                start: round3(t),
                end: round3(t + len),
                confidence: 0.9,
            });
            t += len + 0.06;
        }
        sentences.push(SpeechSentence {
            beat,
            start: round3(start),
            end: round3(t),
        });
        t += PAUSE;
    }
    let map = SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "voice.wav".into(),
        sample_rate: 48_000,
        duration: round3(t + 1.0),
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fixture".into(),
        words,
        sentences,
    };
    repair(&map, &lines).0
}

fn compile(
    intent: &CreativeIntent,
    style: &StyleProfile,
    speech: &SpeechMap,
    art: bool,
) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        art: art.then_some(ArtMode::Force(Look::Cinematic3d)),
        speech: Some(speech.clone()),
        ..CompileOptions::default()
    };
    compile_with_options(
        intent,
        style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

/// The project without depth of field. The cinematic look keeps the title
/// plane (z -120) and, while the camera travels, the supporting picture
/// several px out of focus by design, above `READABLE_BLUR_PX`; the timing
/// tests judge timing alone (blur has its own tests below).
fn sharp(mut project: MotionProject) -> MotionProject {
    for scene in &mut project.scenes {
        if let Some(p) = scene.camera.as_mut().and_then(|c| c.perspective.as_mut()) {
            p.aperture = 0.0;
        }
    }
    project
}

/// Story, voice and the compiled (sharp) project.
fn fixture(named: bool) -> (CreativeIntent, SpeechMap, MotionProject) {
    let (intent, style) = story(named);
    let speech = voice(&intent);
    let project = sharp(compile(&intent, &style, &speech, true));
    (intent, speech, project)
}

fn report(project: &MotionProject, speech: &SpeechMap) -> SpeechQaReport {
    speech_report(project, speech, Path::new("."), None, None, None, None).expect("speech qa")
}

fn check<'r>(r: &'r SpeechQaReport, name: &str) -> &'r SpeechQaCheck {
    r.checks
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no check {name}"))
}

fn status(r: &SpeechQaReport, name: &str) -> CheckStatus {
    check(r, name).status
}

/// Every word and sentence moved by `dt` seconds.
fn shifted(speech: &SpeechMap, dt: f64) -> SpeechMap {
    let mut s = speech.clone();
    for w in &mut s.words {
        w.start += dt;
        w.end += dt;
    }
    for x in &mut s.sentences {
        x.start += dt;
        x.end += dt;
    }
    s
}

/// What a recogniser would report for the script: lower-case words with a
/// trailing comma here and there, "eight" written as "8".
fn heard(speech: &SpeechMap) -> Vec<RecognisedWord> {
    speech
        .words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let mut word = w.text.to_lowercase();
            if word == "eight" {
                word = "8".into();
            } else if i % 5 == 4 {
                word.push(',');
            }
            RecognisedWord {
                word,
                start: w.start,
                end: w.end,
            }
        })
        .collect()
}

fn timing<'r>(r: &'r SpeechQaReport, beat: usize, group: &str) -> &'r RevealTiming {
    r.reveal_timings
        .iter()
        .find(|t| t.beat == beat && t.group == group)
        .unwrap_or_else(|| panic!("no timing for beat {beat} {group}: {:?}", r.reveal_timings))
}

// ---------------------------------------------------------------------------
// The four checks on the space story
// ---------------------------------------------------------------------------

#[test]
fn timing_checks_pass_when_reveals_follow_the_voice() {
    for named in [false, true] {
        let (_, speech, project) = fixture(named);
        let r = report(&project, &speech);
        for name in TIMING_CHECKS {
            assert!(
                matches!(status(&r, name), CheckStatus::Pass | CheckStatus::Skip),
                "named={named} {name}: {}",
                r.to_text()
            );
        }
        assert_eq!(status(&r, "speech_rate"), CheckStatus::Pass);
        assert_eq!(status(&r, "reveal_before_speech"), CheckStatus::Pass);
        assert_eq!(status(&r, "spoken_mismatch"), CheckStatus::Skip);
        assert!(r.passed(), "{}", r.to_text());
        // Every group whose word is spoken becomes readable, within the
        // limits on both sides.
        assert!(!r.reveal_timings.is_empty());
        for t in &r.reveal_timings {
            let at = t
                .readable_at
                .unwrap_or_else(|| panic!("{t:?} never readable"));
            assert!(t.word_start - at <= 0.35 + 1e-9, "{t:?}");
            assert!(at - t.word_end <= 0.8 + 1e-9, "{t:?}");
        }
        // The example's own narration says the titles' words and Saturn; the
        // named one also the pictures and their labels.
        let groups: Vec<&str> = r.reveal_timings.iter().map(|t| t.group.as_str()).collect();
        assert!(groups.contains(&"title"), "{groups:?}");
        assert!(groups.contains(&"hero"), "{groups:?}");
        if named {
            assert!(groups.contains(&"prop") && groups.contains(&"prop_label"));
            assert_eq!(timing(&r, 1, "hero").word, "earth");
            assert_eq!(timing(&r, 1, "hero").role, RevealRole::Content);
        }
        // Unspoken anchors are counted, not judged.
        assert!(
            check(&r, "reveal_before_speech")
                .detail
                .contains("unspoken"),
            "{}",
            check(&r, "reveal_before_speech").detail
        );
    }
}

#[test]
fn crammed_sentence_fails_speech_rate_and_a_fast_one_warns() {
    let (_, speech, project) = fixture(false);
    let rate_of = |s: &SpeechMap, beat: usize| -> f64 {
        let sentence = s
            .sentences
            .iter()
            .find(|x| x.beat == beat)
            .expect("sentence");
        let words = s.words_in(sentence);
        let syl: usize = words.iter().map(|w| estimate_syllables(&w.text)).sum();
        syl as f64 / (words[words.len() - 1].end - words[0].start)
    };
    // Squeeze beat 2's words toward its first word to `target` syllables/s.
    let squeezed = |target: f64| -> SpeechMap {
        let mut s = speech.clone();
        let k = rate_of(&speech, 1) / target;
        let sentence = s.sentences[1].clone();
        let first = speech.words_in(&sentence)[0].start;
        for w in s
            .words
            .iter_mut()
            .filter(|w| w.start >= sentence.start - 1e-9 && w.start < sentence.end + 1e-9)
        {
            w.start = first + (w.start - first) * k;
            w.end = first + (w.end - first) * k;
        }
        assert!((rate_of(&s, 1) - target).abs() < 1e-6);
        s
    };

    let r = report(&project, &squeezed(10.0));
    assert_eq!(
        status(&r, "speech_rate"),
        CheckStatus::Fail,
        "{}",
        r.to_text()
    );
    assert_eq!(
        r.speech_rate_findings,
        ["beat 2: 10.0 syll/s (Sunlight takes eight minutes…)"]
    );
    assert!(!r.passed());

    let r = report(&project, &squeezed(8.0));
    assert_eq!(
        status(&r, "speech_rate"),
        CheckStatus::Warn,
        "{}",
        r.to_text()
    );
    assert_eq!(
        r.speech_rate_findings,
        ["beat 2: 8.0 syll/s (Sunlight takes eight minutes…)"]
    );
}

#[test]
fn voice_a_second_late_fails_reveal_before_speech() {
    let (_, speech, project) = fixture(true);
    let r = report(&project, &shifted(&speech, 1.0));
    assert_eq!(
        status(&r, "reveal_before_speech"),
        CheckStatus::Fail,
        "{}",
        r.to_text()
    );
    // The hero entered on "earth"; the word now comes a second later.
    let hero = timing(&r, 1, "hero");
    let at = hero.readable_at.expect("hero readable");
    let finding = format!(
        "beat 1: hero (content) readable at {at:.2} s, \"earth\" starts at {:.2} s (lead {:.0} ms)",
        hero.word_start,
        (hero.word_start - at) * 1000.0
    );
    assert!(
        r.reveal_before_speech_findings.contains(&finding),
        "{finding}\n{:#?}",
        r.reveal_before_speech_findings
    );
    assert!(hero.word_start - at > 1.0 - 0.35);
    // Later words never make a reveal late.
    assert_eq!(
        status(&r, "reveal_late"),
        CheckStatus::Pass,
        "{}",
        r.to_text()
    );
    // The text block prints the findings under the check.
    let text = r.to_text();
    assert!(text.contains("FAIL reveal_before_speech"), "{text}");
    assert!(text.contains(&format!("      {finding}")), "{text}");
}

#[test]
fn voice_early_warns_reveal_late_without_failing() {
    let (_, speech, project) = fixture(true);
    let base = report(&project, &speech);
    let hero = timing(&base, 3, "hero").clone();
    let at = hero.readable_at.expect("hero readable");
    // Move the voice earlier so Saturn's hero is readable 1.5 s after
    // "Saturn" ends.
    let dt = 1.5 - (at - hero.word_end);
    let r = report(&project, &shifted(&speech, -dt));
    assert_eq!(
        status(&r, "reveal_late"),
        CheckStatus::Warn,
        "{}",
        r.to_text()
    );
    let finding = format!(
        "beat 3: hero (content) readable at {at:.2} s, \"Saturn\" ends at {:.2} s (late 1500 ms)",
        hero.word_end - dt
    );
    assert!(
        r.reveal_late_findings.contains(&finding),
        "{finding}\n{:#?}",
        r.reveal_late_findings
    );
    // Earlier words never make a reveal early; a WARN never fails a check.
    assert_eq!(status(&r, "reveal_before_speech"), CheckStatus::Pass);
    for name in TIMING_CHECKS {
        assert_ne!(status(&r, name), CheckStatus::Fail, "{name}");
    }
}

#[test]
fn spoken_mismatch_compares_what_was_heard_with_the_script() {
    let (_, speech, project) = fixture(false);

    // Everything heard ("8" for "eight", commas, lower case): PASS.
    let mut ok = speech.clone();
    ok.recognised = heard(&speech);
    let r = report(&project, &ok);
    assert_eq!(
        status(&r, "spoken_mismatch"),
        CheckStatus::Pass,
        "{}",
        r.to_text()
    );
    assert!(r.spoken_mismatch_findings.is_empty());

    // The number was not said: a content word is missing.
    let mut dropped = ok.clone();
    dropped.recognised.retain(|w| w.word != "8");
    let r = report(&project, &dropped);
    assert_eq!(
        status(&r, "spoken_mismatch"),
        CheckStatus::Fail,
        "{}",
        r.to_text()
    );
    assert_eq!(r.spoken_mismatch_findings, ["beat 2: missing \"eight\""]);
    assert!(!r.passed());

    // A misread number is a changed content word.
    let mut misread = ok.clone();
    for w in &mut misread.recognised {
        if w.word == "8" {
            w.word = "eighty".into();
        }
    }
    let r = report(&project, &misread);
    assert_eq!(status(&r, "spoken_mismatch"), CheckStatus::Fail);
    assert_eq!(
        r.spoken_mismatch_findings,
        ["beat 2: \"eight\" heard as \"eighty\""]
    );

    // Only beat 2's "the" changed: WARN, the build passes.
    let mut article = ok.clone();
    if let Some(w) = article.recognised.iter_mut().find(|w| w.word == "the") {
        w.word = "a".into();
    }
    let r = report(&project, &article);
    assert_eq!(
        status(&r, "spoken_mismatch"),
        CheckStatus::Warn,
        "{}",
        r.to_text()
    );
    assert_eq!(
        r.spoken_mismatch_findings,
        ["beat 2: \"the\" heard as \"a\""]
    );
    assert!(r.passed(), "{}", r.to_text());
}

#[test]
fn an_anchored_word_that_is_not_a_number_is_a_content_word() {
    // "satellite" names beat 1's supporting picture (an anchor word).
    let (_, speech, project) = fixture(true);
    let mut s = speech.clone();
    s.recognised = heard(&speech);
    s.recognised
        .retain(|w| w.word.trim_end_matches(',') != "satellite");
    let r = report(&project, &s);
    assert_eq!(
        status(&r, "spoken_mismatch"),
        CheckStatus::Fail,
        "{}",
        r.to_text()
    );
    assert_eq!(
        r.spoken_mismatch_findings,
        ["beat 1: missing \"satellite\""]
    );
}

#[test]
fn the_kicker_is_exempt() {
    let (_, speech, mut project) = fixture(true);
    // The kicker enters with the beat; anchor it (and its rule) to the last
    // word of beat 1, said seconds later.
    let late_word = vec!["orbit".to_string()];
    let art = project.project.art.as_mut().expect("art");
    let beat1 = art.reveals.get_mut("beat_1").expect("beat 1 anchors");
    for group in ["kicker", "kicker_rule"] {
        beat1.push(RevealAnchor {
            group: group.into(),
            words: late_word.clone(),
            role: RevealRole::Label,
        });
    }
    let r = report(&project, &speech);
    assert_eq!(
        status(&r, "reveal_before_speech"),
        CheckStatus::Pass,
        "{}",
        r.to_text()
    );
    let all: Vec<&String> = r
        .reveal_before_speech_findings
        .iter()
        .chain(&r.reveal_late_findings)
        .collect();
    assert!(all.iter().all(|f| !f.contains("kicker")), "{all:?}");
    assert!(r
        .reveal_timings
        .iter()
        .all(|t| !t.group.starts_with("kicker")));

    // The same layers under another group name are judged (and early).
    let art = project.project.art.as_mut().expect("art");
    art.reveals
        .get_mut("beat_1")
        .expect("beat 1 anchors")
        .push(RevealAnchor {
            group: "stage".into(),
            words: late_word,
            role: RevealRole::Label,
        });
    let r = report(&project, &speech);
    assert_eq!(status(&r, "reveal_before_speech"), CheckStatus::Fail);
    assert!(
        r.reveal_before_speech_findings
            .iter()
            .any(|f| f.starts_with("beat 1: stage (label) readable at")),
        "{:?}",
        r.reveal_before_speech_findings
    );
}

#[test]
fn a_project_without_art_skips_the_reveal_checks() {
    let (intent, style) = story(false);
    let speech = voice(&intent);
    let project = compile(&intent, &style, &speech, false);
    assert!(project.project.art.is_none());
    let r = report(&project, &speech);
    for name in ["reveal_before_speech", "reveal_late"] {
        assert_eq!(status(&r, name), CheckStatus::Skip, "{name}");
        assert!(check(&r, name).detail.contains("no reveal anchors"));
    }
    assert_eq!(status(&r, "speech_rate"), CheckStatus::Pass);
    assert!(r.reveal_timings.is_empty());
}

#[test]
fn cinematic_depth_of_field_only_withholds_readability() {
    // As compiled (aperture 9), with the same voice: a group may be judged
    // never readable (out of focus), never readable at another time.
    let (intent, style) = story(true);
    let speech = voice(&intent);
    let shipped = compile(&intent, &style, &speech, true);
    let r = report(&shipped, &speech);
    let s = report(&sharp(shipped.clone()), &speech);
    // Depth of field can only delay readability (or withhold it), never
    // bring it forward; big type and pictures stay legible under the look's
    // deliberate softness (the limit scales with their size).
    assert!(matches!(
        status(&r, "reveal_late"),
        CheckStatus::Pass | CheckStatus::Warn
    ));
    for (blurred, sharp) in r.reveal_timings.iter().zip(&s.reveal_timings) {
        assert_eq!((blurred.beat, &blurred.group), (sharp.beat, &sharp.group));
        if let Some(at) = blurred.readable_at {
            assert!(at >= sharp.readable_at.expect("sharp readable") - 1e-9);
        }
    }
    // WARN alone leaves the verdict at PASS; a clamped cue only warns.
    assert_ne!(
        status(&r, "reveal_late"),
        CheckStatus::Fail,
        "{}",
        r.to_text()
    );
}

#[test]
fn json_report_carries_the_timing_findings() {
    let (_, speech, project) = fixture(true);
    let r = report(&project, &shifted(&speech, 1.0));
    let v = serde_json::to_value(&r).expect("json");
    for key in [
        "speech_rate_findings",
        "reveal_before_speech_findings",
        "reveal_late_findings",
        "spoken_mismatch_findings",
        "reveal_timings",
    ] {
        assert!(v.get(key).is_some_and(Value::is_array), "{key}");
    }
    assert!(!v["reveal_before_speech_findings"]
        .as_array()
        .expect("array")
        .is_empty());
    let t = &v["reveal_timings"][0];
    for key in [
        "beat",
        "group",
        "role",
        "word",
        "word_start",
        "word_end",
        "readable_frame",
    ] {
        assert!(t.get(key).is_some(), "{key}: {t}");
    }
    // The check list keeps its order: captions, timing, layout.
    let names: Vec<&str> = r.checks.iter().map(|c| c.name.as_str()).collect();
    let at = |n: &str| names.iter().position(|x| *x == n).expect(n);
    assert!(at("caption_lines") < at("speech_rate"));
    assert!(at("speech_rate") < at("reveal_before_speech"));
    assert!(at("reveal_before_speech") < at("reveal_late"));
    assert!(at("reveal_late") < at("spoken_mismatch"));
    assert!(at("spoken_mismatch") < at("layout"));
}

#[test]
fn text_report_prints_the_first_eight_findings_and_a_count() {
    let (_, speech, project) = fixture(true);
    let mut r = report(&project, &speech);
    r.reveal_late_findings = (1..=11).map(|i| format!("finding {i}")).collect();
    let text = r.to_text();
    assert!(text.contains("      finding 8\n"), "{text}");
    assert!(!text.contains("finding 9"), "{text}");
    assert!(
        text.contains("11 finding(s) in all, first 8 shown"),
        "{text}"
    );
}

// ---------------------------------------------------------------------------
// Readable time on hand-built projects (30 fps, 1080 x 1920)
// ---------------------------------------------------------------------------

fn project(layers: Vec<Value>, motions: Vec<Value>, camera: Option<Value>) -> MotionProject {
    let mut scene = json!({ "id": "beat_1", "start_seconds": 0.0, "duration_seconds": 4.0,
        "layers": layers, "motions": motions });
    if let Some(c) = camera {
        scene["camera"] = c;
    }
    let doc = json!({
        "version": "0.2",
        "project": { "name": "readable" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#000000" },
        "theme": { "fonts": { "display": "font_display" } },
        "assets": [ { "id": "font_display", "type": "font", "path": "assets/fonts/display.ttf" } ],
        "scenes": [scene],
    });
    MotionProject::from_json(&doc.to_string()).expect("project parses")
}

fn rect(id: &str, x: f64) -> Value {
    json!({ "id": id, "type": "rectangle", "x": x, "y": 800, "width": 100, "height": 100,
            "fill": "#FFFFFF" })
}

fn motion(target: &str, start: f64, duration: f64, op: Value) -> Value {
    let mut m =
        json!({ "target": target, "start": start, "duration": duration, "easing": "linear" });
    for (k, v) in op.as_object().expect("op") {
        m[k] = v.clone();
    }
    m
}

fn card() -> GroupRef {
    GroupRef {
        scene: "beat_1".into(),
        prefix: "b1.".into(),
        group: "card".into(),
    }
}

fn first_frame(p: &MotionProject) -> Option<u32> {
    first_readable_frames(p, &[card()]).expect("frames")[0]
}

#[test]
fn a_layer_fading_in_over_half_a_second_is_readable_at_half_opacity() {
    // Opacity (t - 1) / 0.5 reaches 0.5 at t = 1.25 s = frame 37.5: frame 38.
    let p = project(
        vec![rect("b1.card", 400.0)],
        vec![motion(
            "b1.card",
            1.0,
            0.5,
            json!({ "op": "fade", "from": 0.0, "to": 1.0 }),
        )],
        None,
    );
    assert_eq!(first_frame(&p), Some(38));
    // Another group of the scene is not this one.
    let other = GroupRef {
        group: "car".into(),
        ..card()
    };
    assert_eq!(first_readable_frames(&p, &[other]).expect("frames"), [None]);
}

#[test]
fn ancestor_opacity_compounds_and_children_belong_to_their_group() {
    // A group at 0.6 holds the fading child: 0.6 * p >= 0.5 from p = 5/6,
    // t = 1 + 0.5 * 5/6 = 1.4167 s = frame 42.5: frame 43. The child's id
    // ("b1.card.bg") is in the group by its path.
    let group = json!({ "id": "b1.card", "type": "group", "x": 0, "y": 0, "width": 1080,
        "height": 1920, "opacity": 0.6, "children": [rect("b1.card.bg", 400.0)] });
    let p = project(
        vec![group],
        vec![motion(
            "b1.card.bg",
            1.0,
            0.5,
            json!({ "op": "fade", "from": 0.0, "to": 1.0 }),
        )],
        None,
    );
    assert_eq!(first_frame(&p), Some(43));
}

#[test]
fn a_glyph_cascade_is_readable_once_its_last_glyph_lands() {
    // Two glyphs, stagger 0.2 s, run 0.3 s from opacity 0: the second glyph
    // passes 0.5 at t = 1.2 + 0.15 = 1.35 s = frame 40.5: frame 41.
    let text = json!({ "id": "b1.card", "type": "text", "text": "AB", "font_role": "display",
        "font_size": 80, "color": "#FFFFFF", "x": 300, "y": 800, "width": 600, "height": 120 });
    let p = project(
        vec![text],
        vec![motion(
            "b1.card",
            1.0,
            0.5,
            json!({ "op": "glyph_cascade", "stagger": 0.2, "order": "forward",
                    "from": { "dy": 40.0, "opacity": 0.0 } }),
        )],
        None,
    );
    assert_eq!(first_frame(&p), Some(41));
}

#[test]
fn a_layer_off_canvas_is_not_readable_until_it_enters() {
    // Box x in [-300, -200] + 1000 t: on canvas once its right edge passes 0
    // (t > 0.2 s): frame 7.
    let p = project(
        vec![rect("b1.card", -300.0)],
        vec![motion(
            "b1.card",
            0.0,
            1.0,
            json!({ "op": "move", "from": [0.0, 0.0], "to": [1000.0, 0.0] }),
        )],
        None,
    );
    assert_eq!(first_frame(&p), Some(7));
    // Never on canvas: never readable.
    let p = project(vec![rect("b1.card", 2000.0)], vec![], None);
    assert_eq!(first_frame(&p), None);
}

#[test]
fn depth_of_field_blur_withholds_readability_until_focus_arrives() {
    // A plane at z 100 under aperture 5: blur 5 * |100 - focus| / 100 px.
    let mut layer = rect("b1.card", 400.0);
    layer["z"] = json!(100.0);
    let persp = json!({ "fov_deg": 40.0, "focus_z": 0.0, "aperture": 5.0 });
    // Focus stays at 0: 5 px, never readable.
    let p = project(
        vec![layer.clone()],
        vec![],
        Some(json!({ "perspective": persp, "motions": [] })),
    );
    assert_eq!(first_frame(&p), None);
    // Focus racks 0 -> 100 over 1.2 s: blur <= 2 px once focus >= 60, at
    // t = 0.72 s = frame 21.6: frame 22.
    let rack = json!({ "start": 0.0, "duration": 1.2, "easing": "linear", "op": "focus",
        "from": 0.0, "to": 100.0 });
    let p = project(
        vec![layer],
        vec![],
        Some(json!({ "perspective": persp, "motions": [rack] })),
    );
    assert_eq!(first_frame(&p), Some(22));
}
