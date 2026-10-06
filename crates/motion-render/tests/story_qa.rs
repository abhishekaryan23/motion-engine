//! (0.23 W4) The structural story checks `dead_air`, `count_unsettled` and
//! `repeat_template` on synthetic projects: every project is built here, no
//! file is read and nothing is rendered (the checks run on the timeline).

use std::collections::BTreeMap;

use motion_core::checks::{
    CARRY_CONTINUITY, COUNT_HOLD_S, COUNT_SETTLE_S, DEAD_AIR_FIRST_READABLE_S,
};
use motion_core::compiler::direction::{
    BeatDirection, BeatParams, DirectionRecord, EntranceFamily,
};
use motion_core::scene::{Motion, MotionOp};
use motion_core::speech::SpeechMap;
use motion_core::timeline::evaluate_frame;
use motion_core::MotionProject;
use motion_render::speech_qa::CheckStatus;
use motion_render::story_qa::{
    carry_continuity, count_shown_at, count_unsettled, dead_air, motion_checks, repeat_template,
    story_checks, AnchorSource,
};

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// A text layer (540 x 960 canvas).
fn text(id: &str, y: u32, body: &str) -> String {
    format!(
        r##"{{ "id": "{id}", "type": "text", "x": 40, "y": {y}, "width": 460, "height": 120,
              "text": "{body}", "font_role": "number", "font_size": 80, "color": "#111111" }}"##
    )
}

fn fade(target: &str, start: f64, duration: f64, from: f64, to: f64) -> String {
    format!(
        r##"{{ "target": "{target}", "start": {start}, "duration": {duration},
              "op": "fade", "from": {from}, "to": {to} }}"##
    )
}

fn count(target: &str, start: f64, duration: f64, to: u32) -> String {
    format!(
        r##"{{ "target": "{target}", "start": {start}, "duration": {duration}, "easing": "out_quint",
              "op": "count", "from": 0.0, "to": {to}.0, "suffix": "%" }}"##
    )
}

/// One beat scene. `read` and `anticipate` are scene-local seconds.
fn beat(
    n: u32,
    start: f64,
    duration: f64,
    read: f64,
    anticipate: f64,
    layers: &[String],
    motions: &[String],
) -> String {
    format!(
        r##"{{ "id": "beat_{n}", "start_seconds": {start}, "duration_seconds": {duration},
              "layers": [{}], "motions": [{}],
              "lifecycle": {{ "enter": 0.1, "settle": 0.2, "read": {read}, "evolve": {},
                              "anticipate": {anticipate}, "bridge": {duration} }} }}"##,
        layers.join(","),
        motions.join(","),
        (read + anticipate) / 2.0,
    )
}

fn project(scenes: &[String]) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "story", "duration_seconds": 12.0 }},
        "canvas": {{ "width": 540, "height": 960, "fps": 30, "background": "#F4F1EA" }},
        "theme": {{ "fonts": {{ "number": "font.anton", "body": "font.fira" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" }},
            {{ "id": "font.fira", "type": "font", "path": "fonts/FiraSans-Regular.ttf" }}
        ],
        "asset_root": "assets",
        "scenes": [{}]
    }}"##,
        scenes.join(",")
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

/// A speech map whose beat 1 says "about sixty percent": `sixty` starts at
/// 0.5 s.
fn speech_sixty() -> SpeechMap {
    SpeechMap::from_json(
        r##"{ "version": "0.1", "audio": "v.wav", "sample_rate": 48000, "duration": 6.0,
              "provider": "fixture", "model": "fixture", "voice": "fixture",
              "words": [
                { "text": "about",   "start": 0.4, "end": 0.5, "confidence": 1.0 },
                { "text": "sixty",   "start": 0.5, "end": 0.9, "confidence": 1.0 },
                { "text": "percent", "start": 0.9, "end": 1.4, "confidence": 1.0 }
              ],
              "sentences": [ { "beat": 0, "start": 0.35, "end": 1.5 } ] }"##,
    )
    .expect("speech parses")
}

// ---------------------------------------------------------------------------
// dead_air
// ---------------------------------------------------------------------------

#[test]
fn content_from_the_first_frame_is_not_dead_air() {
    let p = project(&[
        beat(
            1,
            0.0,
            3.0,
            0.5,
            2.5,
            &[text("b1.title", 200, "HELLO")],
            &[],
        ),
        beat(
            2,
            3.0,
            3.0,
            0.5,
            2.5,
            &[text("b2.title", 200, "WORLD")],
            &[],
        ),
    ]);
    let r = dead_air(&p).expect("dead_air");
    assert_eq!(r.status, CheckStatus::Pass, "{:?}", r.offences);
    assert_eq!(r.first_content, Some(0.0));
    assert_eq!(r.beats.len(), 2);
    assert!(r.beats.iter().all(|b| b.wait == Some(0.0)), "{:?}", r.beats);
    let c = r.check();
    assert_eq!(c.name, "dead_air");
    assert_eq!(c.status, CheckStatus::Pass);
}

#[test]
fn a_late_first_content_fails() {
    // The title fades in over 0.8..1.0 s: readable (opacity 0.5) at about 0.9 s.
    let p = project(&[beat(
        1,
        0.0,
        3.0,
        0.5,
        2.5,
        &[text("b1.title", 200, "HELLO")],
        &[fade("b1.title", 0.8, 0.2, 0.0, 1.0)],
    )]);
    let r = dead_air(&p).expect("dead_air");
    assert_eq!(r.status, CheckStatus::Fail);
    let first = r.first_content.expect("it does appear");
    assert!(
        first > DEAD_AIR_FIRST_READABLE_S && (0.85..=1.0).contains(&first),
        "{first}"
    );
    let c = r.check();
    assert_eq!(c.status, CheckStatus::Fail);
    assert!(c.detail.contains("first content at 0.9"), "{}", c.detail);
    assert!(c.detail.contains("b1.title"), "{}", c.detail);
}

#[test]
fn a_beat_that_waits_too_long_for_content_fails_and_is_named() {
    // Beat 2 shows only a ghost word (decorative) for 1.6 s, then its title.
    let ghost = text("b2.ghost", 400, "GHOST");
    let p = project(&[
        beat(
            1,
            0.0,
            3.0,
            0.5,
            2.5,
            &[text("b1.title", 200, "HELLO")],
            &[],
        ),
        beat(
            2,
            3.0,
            3.5,
            0.5,
            3.0,
            &[ghost, text("b2.title", 200, "WORLD")],
            &[fade("b2.title", 1.6, 0.2, 0.0, 1.0)],
        ),
    ]);
    let r = dead_air(&p).expect("dead_air");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.offences);
    // The opening is fine; only beat 2 offends.
    assert_eq!(r.beats[0].wait, Some(0.0));
    let wait = r.beats[1].wait.expect("beat 2 gets content");
    assert!((1.65..=1.8).contains(&wait), "{wait}");
    assert_eq!(r.beats[1].layer.as_deref(), Some("b2.title"));
    assert_eq!(r.offences.len(), 1, "{:?}", r.offences);
    let detail = r.check().detail;
    assert!(detail.contains("beat 2"), "{detail}");
    assert!(detail.contains("1.7"), "{detail}");
    assert!(!detail.contains("beat 1"), "{detail}");
}

#[test]
fn a_beat_without_any_content_fails() {
    let p = project(&[beat(
        1,
        0.0,
        2.0,
        0.5,
        1.5,
        &[text("b1.ghost", 400, "GHOST")],
        &[],
    )]);
    let r = dead_air(&p).expect("dead_air");
    assert_eq!(r.status, CheckStatus::Fail);
    assert_eq!(r.first_content, None);
    assert_eq!(r.beats[0].wait, None);
    assert!(r.check().detail.contains("never"), "{}", r.check().detail);
}

#[test]
fn a_project_without_beats_skips() {
    // No lifecycle on any scene: every scene is a beat, but `backdrop` and
    // `captions` never count.
    let only_backdrop = r##"{ "id": "backdrop", "start_seconds": 0.0, "duration_seconds": 2.0,
        "layers": [ { "id": "backdrop.paper", "type": "rectangle", "x": 0, "y": 0,
                      "width": 540, "height": 960, "fill": "#FFFFFF" } ] }"##;
    let p = project(&[only_backdrop.to_string()]);
    let r = dead_air(&p).expect("dead_air");
    assert_eq!(r.status, CheckStatus::Skip);
}

// ---------------------------------------------------------------------------
// count_unsettled
// ---------------------------------------------------------------------------

/// A beat whose "60%" counter runs `0.2 .. 0.2 + duration`; READ at 0.5.
fn counter_beat(duration: f64, anticipate: f64, extra: &[String]) -> MotionProject {
    let mut motions = vec![count("b1.hero.60", 0.2, duration, 60)];
    motions.extend_from_slice(extra);
    project(&[beat(
        1,
        0.0,
        3.0,
        0.5,
        anticipate,
        &[text("b1.hero.60", 200, "0%")],
        &motions,
    )])
}

#[test]
fn a_counter_that_settles_late_fails_against_its_spoken_number() {
    // Out-quint reads 60% at 62% of 2.0 s: 1.43 s, 0.93 s after "sixty".
    let p = counter_beat(2.0, 2.5, &[]);
    let r = count_unsettled(&p, Some(&speech_sixty())).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.counters);
    let c = &r.counters[0];
    assert_eq!(c.layer, "b1.hero.60");
    assert_eq!(c.final_text, "60%");
    assert_eq!(c.source, AnchorSource::SpokenValue("sixty".into()));
    assert!((c.anchor - 0.5).abs() < 1e-9, "{}", c.anchor);
    let settle = c.settle.expect("it settles");
    assert!(settle - c.anchor > COUNT_SETTLE_S, "{settle}");
    let detail = r.check().detail;
    assert!(detail.contains("beat 1 b1.hero.60"), "{detail}");
    assert!(detail.contains("\"sixty\""), "{detail}");
    assert!(detail.contains("reads"), "{detail}");

    // Without a speech map the anchor is READ (0.5 s): the same verdict.
    let r = count_unsettled(&p, None).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Fail);
    assert_eq!(r.counters[0].source, AnchorSource::Read);
}

#[test]
fn a_counter_that_lands_on_its_word_and_holds_passes() {
    let p = counter_beat(0.9, 2.5, &[]);
    for speech in [Some(speech_sixty()), None] {
        let r = count_unsettled(&p, speech.as_ref()).expect("count_unsettled");
        assert_eq!(r.status, CheckStatus::Pass, "{:?}", r.counters);
        let c = &r.counters[0];
        let settle = c.settle.expect("it settles");
        assert!(
            settle > 0.2 && settle - c.anchor <= COUNT_SETTLE_S,
            "{settle}"
        );
        assert!(c.hold.expect("hold") >= COUNT_HOLD_S, "{:?}", c.hold);
        assert!(c.note.is_none());
    }
}

fn r_check_names(p: &MotionProject) -> Vec<String> {
    story_checks(p, None)
        .expect("checks")
        .into_iter()
        .map(|c| c.name)
        .collect()
}

#[test]
fn a_counter_that_leaves_too_soon_fails_when_the_layer_has_room() {
    // The counter lands at about 0.75 s (it starts at 0.2 s) but its layer
    // fades out around 1.7 s: under 1.0 s of hold, while the exit leaves 1.5 s
    // after the count starts (at least a 0.4 s count plus the 1.0 s hold):
    // a count that ends sooner would hold. The count is at fault.
    let p = counter_beat(0.9, 2.5, &[fade("b1.hero.60", 1.55, 0.3, 1.0, 0.0)]);
    let r = count_unsettled(&p, Some(&speech_sixty())).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.counters);
    let c = &r.counters[0];
    assert!(c.hold.expect("hold") < COUNT_HOLD_S, "{:?}", c.hold);
    assert!(
        c.note.as_deref().unwrap_or("").contains("holds"),
        "{:?}",
        c.note
    );
}

#[test]
fn a_short_hold_is_only_a_warning_when_the_layer_leaves_too_soon_for_a_count() {
    // The layer leaves at 1.5 s, 1.3 s after the count starts: less than a
    // 0.4 s count plus the 1.0 s hold. Even the shortest count could not hold
    // for a second: the beat limits it.
    let p = counter_beat(0.9, 1.5, &[]);
    let r = count_unsettled(&p, Some(&speech_sixty())).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Warn, "{:?}", r.counters);
    let c = &r.counters[0];
    assert!(c.hold.expect("hold") < COUNT_HOLD_S);
    assert!(c.settle.expect("settle") - c.anchor <= COUNT_SETTLE_S);
    assert!(c
        .note
        .as_deref()
        .unwrap_or("")
        .contains("after the count starts"));
}

#[test]
fn the_warning_boundary_is_the_hold_plus_the_shortest_count() {
    // Exit exactly COUNT_HOLD_S + COUNT_MIN_S = 1.4 s after the count starts
    // (0.2 s): ANTICIPATE at 1.6 s. A 0.9 s count settles at about 0.75 s and
    // holds under a second: there was room for a shorter count, so FAIL.
    let p = counter_beat(0.9, 1.6, &[]);
    let r = count_unsettled(&p, Some(&speech_sixty())).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.counters);
}

#[test]
fn a_counter_that_settles_while_hidden_holds_only_while_shown() {
    // The count runs 0.2..0.6 s with its layer still invisible; the layer
    // fades in over 1.5..1.6 s. The final text is on screen only from about
    // 1.57 s to ANTICIPATE (2.2 s): 0.63 s of hold, not the 1.1 s that
    // counting from the settle would give. The exit leaves 2.0 s after the
    // count starts (at least a 0.4 s count plus the hold), so the count's
    // timing is at fault: FAIL.
    let p = counter_beat(
        0.4,
        2.2,
        &[
            fade("b1.hero.60", 0.0, 0.01, 0.0, 0.0),
            fade("b1.hero.60", 1.5, 0.1, 0.0, 1.0),
        ],
    );
    let r = count_unsettled(&p, None).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.counters);
    let c = &r.counters[0];
    let settle = c.settle.expect("it settles");
    assert!(settle < 0.7, "the count finishes while hidden: {settle}");
    let hold = c.hold.expect("hold");
    assert!(
        (hold - 0.63).abs() <= 1.0 / 30.0 + 1e-9,
        "hold from the first readable frame: {hold}"
    );
    assert!(
        c.note.as_deref().unwrap_or("").contains("holds"),
        "{:?}",
        c.note
    );
}

#[test]
fn a_running_total_is_anchored_to_its_last_item() {
    // Three steps of one total: the last starts at 2.0 s and runs 0.5 s; "60%"
    // is said at 0.5 s, which does not matter for a chain.
    let motions = [
        count("b1.hero.60", 0.5, 0.4, 20),
        count("b1.hero.60", 1.2, 0.4, 40),
        count("b1.hero.60", 2.0, 0.5, 60),
    ];
    let p = project(&[beat(
        1,
        0.0,
        5.0,
        0.5,
        4.5,
        &[text("b1.hero.60", 200, "0%")],
        &motions,
    )]);
    let r = count_unsettled(&p, Some(&speech_sixty())).expect("count_unsettled");
    let c = &r.counters[0];
    assert_eq!(c.source, AnchorSource::LastItem);
    assert!((c.anchor - 2.0).abs() < 1e-9, "{}", c.anchor);
    assert_eq!(r.status, CheckStatus::Pass, "{:?}", r.counters);
    assert!(c.settle.expect("settle") >= 2.0);
}

#[test]
fn an_unreadable_counter_never_holds() {
    // The counter's layer is invisible the whole beat.
    let p = counter_beat(0.9, 2.5, &[fade("b1.hero.60", 0.0, 0.1, 0.0, 0.0)]);
    let r = count_unsettled(&p, None).expect("count_unsettled");
    assert_ne!(r.status, CheckStatus::Pass, "{:?}", r.counters);
    assert_eq!(r.counters[0].hold, Some(0.0));
}

#[test]
fn a_project_without_counters_passes() {
    let p = project(&[beat(
        1,
        0.0,
        3.0,
        0.5,
        2.5,
        &[text("b1.title", 200, "HELLO")],
        &[],
    )]);
    let r = count_unsettled(&p, None).expect("count_unsettled");
    assert_eq!(r.status, CheckStatus::Pass);
    assert!(r.counters.is_empty());
    assert!(r.check().detail.contains("no counting"));
}

#[test]
fn count_shown_at_matches_the_timeline_frame_by_frame() {
    let p = counter_beat(1.4, 2.5, &[]);
    let motions: Vec<&Motion> = p.scenes[0]
        .motions
        .iter()
        .filter(|m| matches!(m.op, MotionOp::Count { .. }))
        .collect();
    for n in 0..90u32 {
        let t = f64::from(n) / 30.0;
        let frame = evaluate_frame(&p, n).expect("evaluate");
        let mut shown = None;
        fn find(layers: &[motion_core::timeline::ResolvedLayer<'_>], id: &str) -> Option<String> {
            for l in layers {
                if l.id == id {
                    return l.text.clone();
                }
                if let Some(t) = find(&l.children, id) {
                    return Some(t);
                }
            }
            None
        }
        if let Some(t) = find(&frame.layers, "b1.hero.60") {
            shown = Some(t);
        }
        assert_eq!(count_shown_at(&motions, t), shown, "frame {n}");
    }
}

// ---------------------------------------------------------------------------
// repeat_template
// ---------------------------------------------------------------------------

fn beat_direction(
    beat: usize,
    template: &str,
    alternates: u8,
    entrance: EntranceFamily,
) -> BeatDirection {
    BeatDirection {
        beat,
        template: template.to_string(),
        alternate: 0,
        alternates,
        params: BeatParams {
            entrance,
            ..BeatParams::default()
        },
        rotations: BTreeMap::new(),
        role: "body".to_string(),
        reason: String::new(),
    }
}

fn with_direction(beats: Vec<BeatDirection>) -> MotionProject {
    let mut p = project(&[beat(
        1,
        0.0,
        3.0,
        0.5,
        2.5,
        &[text("b1.title", 200, "HELLO")],
        &[],
    )]);
    p.project.direction = Some(DirectionRecord {
        take: 0,
        seed: 7,
        chosen: 0,
        beats,
        candidates: Vec::new(),
        shipped_with_failure: None,
    });
    p
}

#[test]
fn a_repeat_with_an_alternative_warns() {
    let p = with_direction(vec![
        beat_direction(0, "hero_stat", 3, EntranceFamily::Rise),
        beat_direction(1, "hero_stat", 3, EntranceFamily::Rise),
        beat_direction(2, "list", 2, EntranceFamily::Rise),
    ]);
    let r = repeat_template(&p);
    assert_eq!(r.status, CheckStatus::Warn);
    assert_eq!(r.repeats.len(), 1);
    assert_eq!(r.repeats[0].beats, (1, 2));
    assert_eq!(r.repeats[0].template, "hero_stat");
    assert_eq!(r.repeats[0].entrance, "rise");
    let c = r.check();
    assert_eq!(c.name, "repeat_template");
    assert_eq!(c.status, CheckStatus::Warn);
    assert!(c.detail.contains("beats 1-2"), "{}", c.detail);
}

#[test]
fn a_repeat_without_an_alternative_passes() {
    // The later beat had no other template to use.
    let p = with_direction(vec![
        beat_direction(0, "hero_stat", 3, EntranceFamily::Rise),
        beat_direction(1, "hero_stat", 1, EntranceFamily::Rise),
    ]);
    let r = repeat_template(&p);
    assert_eq!(r.status, CheckStatus::Pass, "{:?}", r.repeats);
    assert!(r.repeats.is_empty());
    assert_eq!(r.pairs, 1);
}

#[test]
fn a_different_entrance_or_template_passes() {
    let p = with_direction(vec![
        beat_direction(0, "hero_stat", 3, EntranceFamily::Rise),
        beat_direction(1, "hero_stat", 3, EntranceFamily::SlideLeft),
        beat_direction(2, "list", 3, EntranceFamily::SlideLeft),
    ]);
    assert_eq!(repeat_template(&p).status, CheckStatus::Pass);
}

#[test]
fn without_a_direction_record_the_check_skips() {
    let p = project(&[beat(
        1,
        0.0,
        3.0,
        0.5,
        2.5,
        &[text("b1.title", 200, "HELLO")],
        &[],
    )]);
    let r = repeat_template(&p);
    assert_eq!(r.status, CheckStatus::Skip);
    let c = r.check();
    assert_eq!(c.status, CheckStatus::Skip);
    assert_eq!(c.detail, "no direction record");
}

#[test]
fn the_three_checks_come_in_the_documented_order() {
    let p = project(&[beat(
        1,
        0.0,
        3.0,
        0.5,
        2.5,
        &[text("b1.title", 200, "HELLO")],
        &[],
    )]);
    let names = r_check_names(&p);
    assert_eq!(names, ["dead_air", "count_unsettled", "repeat_template"]);
}

// ---------------------------------------------------------------------------
// (0.23 W8) carry_continuity
// ---------------------------------------------------------------------------

/// Two beat scenes (beat 1: 0 - 6 s, beat 2: 5 - 11 s, READ at 1 s into each,
/// so the span judged is 1.0 - 6.0 s) and the shared elements `shared` (JSON
/// objects of `MotionProject.shared`).
fn carry_project(shared: &[String]) -> MotionProject {
    let scenes = [
        beat(1, 0.0, 6.0, 1.0, 4.5, &[text("b1.title", 200, "ONE")], &[]),
        beat(2, 5.0, 6.0, 1.0, 4.5, &[text("b2.title", 200, "TWO")], &[]),
    ];
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "carry", "duration_seconds": 11.0 }},
        "canvas": {{ "width": 540, "height": 960, "fps": 30, "background": "#F4F1EA" }},
        "theme": {{ "fonts": {{ "number": "font.anton", "body": "font.fira" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" }},
            {{ "id": "font.fira", "type": "font", "path": "fonts/FiraSans-Regular.ttf" }}
        ],
        "asset_root": "assets",
        "scenes": [{}],
        "shared": [{}]
    }}"##,
        scenes.join(","),
        shared.join(",")
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

/// A shared picture (a rectangle) with the given track keys, each
/// `(scene, at, state JSON)`.
fn picture(id: &str, keys: &[(&str, f64, &str)]) -> String {
    let track: Vec<String> = keys
        .iter()
        .map(|(scene, at, state)| {
            format!(r##"{{ "scene": "{scene}", "at": {at}, "state": {state} }}"##)
        })
        .collect();
    format!(
        r##"{{ "id": "{id}",
              "layer": {{ "id": "{id}", "type": "rectangle", "x": 100, "y": 400, "width": 200,
                          "height": 200, "fill": "#2244AA" }},
              "track": [{}] }}"##,
        track.join(",")
    )
}

#[test]
fn a_carried_picture_that_stays_readable_across_the_handoff_passes() {
    let p = carry_project(&[picture(
        "shared.pic.apple",
        &[
            ("beat_1", 0.2, r#"{ "x": 100, "y": 400, "opacity": 1.0 }"#),
            ("beat_1", 4.8, "{}"),
            ("beat_2", 1.0, r#"{ "x": 300, "y": 500, "opacity": 1.0 }"#),
            ("beat_2", 4.8, "{}"),
        ],
    )]);
    let r = carry_continuity(&p).expect("carry_continuity");
    assert_eq!(r.status, CheckStatus::Pass, "{:?}", r.gaps);
    assert_eq!(r.spans.len(), 1);
    assert_eq!(r.spans[0].beats, (1, 2));
    assert!((r.spans[0].from - 1.0).abs() < 1e-9 && (r.spans[0].to - 6.0).abs() < 1e-9);
    let c = r.check();
    assert_eq!(c.name, CARRY_CONTINUITY);
    assert_eq!(c.status, CheckStatus::Pass);
}

#[test]
fn a_carried_picture_that_fades_out_across_the_handoff_fails_and_lists_the_gap() {
    // The picture fades out at 4.6 - 4.9 s (beat 1's exit) and is back at 6.2 s
    // (beat 2's key at 1.2 s): it is gone between those, across the handoff and
    // up to beat 2's READ at 6.0 s.
    let p = carry_project(&[picture(
        "shared.pic.apple",
        &[
            ("beat_1", 0.2, r#"{ "x": 100, "y": 400, "opacity": 1.0 }"#),
            ("beat_1", 4.6, "{}"),
            ("beat_1", 4.9, r#"{ "opacity": 0.0 }"#),
            ("beat_2", 1.2, r#"{ "x": 300, "y": 500, "opacity": 1.0 }"#),
            ("beat_2", 4.8, "{}"),
        ],
    )]);
    let r = carry_continuity(&p).expect("carry_continuity");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.spans);
    let gap = &r.gaps[0];
    assert_eq!(gap.element, "shared.pic.apple");
    assert_eq!(gap.beats, (1, 2));
    // It stops being readable partway through the fade out and is not readable
    // again until beat 2's key brings it back (opacity 0.5 at about 5.5 s).
    assert!(gap.from > 4.6 && gap.from < 5.0, "{gap:?}");
    assert!(gap.to > 5.3 && gap.to < 6.0, "{gap:?}");
    assert!(gap.reason.contains("opacity"), "{}", gap.reason);
    let c = r.check();
    assert_eq!(c.status, CheckStatus::Fail);
    assert!(
        c.detail.contains("shared.pic.apple") && c.detail.contains("beats 1-2"),
        "{}",
        c.detail
    );
}

#[test]
fn a_carried_picture_that_leaves_the_canvas_across_the_handoff_fails() {
    let p = carry_project(&[picture(
        "shared.pic.apple",
        &[
            ("beat_1", 0.2, r#"{ "x": 100, "y": 400, "opacity": 1.0 }"#),
            ("beat_1", 4.5, r#"{ "x": 100, "y": 400 }"#),
            // Slides far off the canvas, and arrives in beat 2 by READ.
            ("beat_1", 5.2, r#"{ "x": 5000, "y": 400 }"#),
            ("beat_2", 0.9, r#"{ "x": 5000, "y": 400 }"#),
            ("beat_2", 1.0, r#"{ "x": 300, "y": 500 }"#),
        ],
    )]);
    let r = carry_continuity(&p).expect("carry_continuity");
    assert_eq!(r.status, CheckStatus::Fail, "{:?}", r.spans);
    assert!(r.gaps[0].reason.contains("canvas"), "{:?}", r.gaps);
}

#[test]
fn elements_without_keys_in_two_consecutive_beats_are_not_judged() {
    // No shared element at all, and one that lives in a single beat.
    let none = carry_project(&[]);
    let r = carry_continuity(&none).expect("carry_continuity");
    assert_eq!(r.status, CheckStatus::Skip);
    assert_eq!(r.check().status, CheckStatus::Skip);
    let single = carry_project(&[picture(
        "shared.pic.apple",
        &[
            ("beat_1", 0.2, r#"{ "x": 100, "y": 400, "opacity": 1.0 }"#),
            ("beat_1", 0.4, r#"{ "opacity": 0.0 }"#),
        ],
    )]);
    let r = carry_continuity(&single).expect("carry_continuity");
    assert_eq!(r.status, CheckStatus::Skip, "{:?}", r.gaps);
}

#[test]
fn the_motion_checks_add_carry_continuity_after_the_three_story_checks() {
    let p = carry_project(&[]);
    let names: Vec<String> = motion_checks(&p, None)
        .expect("checks")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(
        names,
        [
            "dead_air",
            "count_unsettled",
            "repeat_template",
            CARRY_CONTINUITY
        ]
    );
}

/// habit_math (a five dollar treat, a day versus a year, spend it or save it,
/// ten years of saving): beat 1 carries the shopping bag and beat 3 the piggy
/// bank. Compiled in a flat look with the clay props family.
fn habit_math(tone: &str) -> MotionProject {
    use motion_core::assets::AssetManifest;
    use motion_core::compiler::art_direction::ArtMode;
    use motion_core::compiler::{
        compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
    };
    use motion_core::intent::CreativeIntent;
    use motion_core::style::StyleProfile;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let intent = CreativeIntent::from_json(
        &std::fs::read_to_string(
            root.join("docs/plans/sprint_0_23/stories/habit_math.intent.json"),
        )
        .expect("habit_math intent"),
    )
    .expect("intent");
    let style: StyleProfile =
        serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style");
    let library =
        AssetLibrary::new(root.join("assets")).with_families(vec!["clay_props_3d".to_string()]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: Some(7),
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

#[test]
fn habit_maths_carried_pictures_stay_readable_across_both_handoffs() {
    // playful hands off with kinetic wipes, technical with geometric ones,
    // editorial with fades.
    for tone in ["playful", "technical", "editorial", "auto"] {
        let p = habit_math(tone);
        let r = carry_continuity(&p).expect("carry_continuity");
        assert_eq!(r.status, CheckStatus::Pass, "{tone}: {:?}", r.gaps);
        let spans: Vec<(&str, (usize, usize))> = r
            .spans
            .iter()
            .map(|s| (s.element.as_str(), s.beats))
            .collect();
        assert_eq!(
            spans,
            [
                ("shared.pic.shopping_bag", (1, 2)),
                ("shared.pic.piggy_bank", (3, 4))
            ],
            "{tone}"
        );
    }
}
