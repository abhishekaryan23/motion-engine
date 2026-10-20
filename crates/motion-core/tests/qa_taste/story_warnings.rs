//! (0.20) Story-level compile warnings, written the way a weak model writes
//! intents: `unrelated_beats` (a "five facts" story with nothing tying the
//! beats together), `carry_ignored` (`continuity: carry_*` the builder
//! dropped) and `title_states_conclusion` (a title that spoils the line).
//! Warnings are returned beside the project and never written into it.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{
    compile_with_options, compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions,
    CompileWarning, TextMeasure, WARN_CARRY_IGNORED, WARN_TITLE_STATES_CONCLUSION,
    WARN_UNRELATED_BEATS,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::MotionProject;
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::style::StyleProfile;
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn library() -> AssetLibrary {
    AssetLibrary::new(repo().join("assets")).with_families(vec!["editorial_cutout".to_string()])
}

fn intent_of(beats: Vec<Value>) -> CreativeIntent {
    serde_json::from_value(json!({
        "version": "0.2", "title": "story_warnings", "format": "vertical", "beats": beats
    }))
    .expect("intent")
}

fn options(art: Option<ArtMode>, speech: Option<SpeechMap>) -> CompileOptions {
    CompileOptions {
        art,
        speech,
        ..CompileOptions::default()
    }
}

fn report(intent: &CreativeIntent, opts: &CompileOptions) -> (MotionProject, Vec<CompileWarning>) {
    let style: StyleProfile = serde_json::from_value(json!({})).expect("style");
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_report(
        intent,
        &style,
        None,
        &library(),
        measure,
        &AssetManifest::empty(),
        None,
        opts,
    )
    .expect("compile")
}

fn warnings(intent: &CreativeIntent, opts: &CompileOptions) -> Vec<CompileWarning> {
    report(intent, opts).1
}

fn with_code<'a>(w: &'a [CompileWarning], code: &str) -> Vec<&'a CompileWarning> {
    w.iter().filter(|w| w.code == code).collect()
}

fn object(asset: &str, meaning: &str) -> Value {
    json!({ "kind": "object", "asset": asset, "meaning": meaning })
}

fn phrase(value: &str) -> Value {
    json!({ "kind": "phrase", "value": value })
}

fn number(value: &str, meaning: &str) -> Value {
    json!({ "kind": "number", "value": value, "meaning": meaning })
}

fn beat(statement: &str, primary: Value) -> Value {
    json!({ "purpose": "emphasize", "statement": statement, "primary": primary })
}

/// "Five facts": four beats, each a different object, nothing between them.
fn five_facts() -> Vec<Value> {
    vec![
        beat("Coffee is a fruit seed", object("coffee_cup", "coffee")),
        beat("The Earth is not a sphere", object("globe", "earth")),
        beat("Clocks run slower up high", object("clock", "time")),
        beat("Ideas arrive in the shower", object("lightbulb", "idea")),
    ]
}

fn layers_subject(focus: &str) -> Value {
    json!({
        "kind": "layers", "meaning": "ocean water column", "focus": focus,
        "layers": [
            { "name": "Photic zone", "note": "sunlit" },
            { "name": "Twilight zone", "note": "dim" },
            { "name": "Dark zone", "note": "no light" }
        ]
    })
}

// ---------------------------------------------------------------------------
// unrelated_beats
// ---------------------------------------------------------------------------

#[test]
fn five_unrelated_object_facts_warn_once() {
    let w = warnings(&intent_of(five_facts()), &options(None, None));
    let hits = with_code(&w, WARN_UNRELATED_BEATS);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert_eq!(hits[0].beat, None, "one warning per story");
    let m = &hits[0].message;
    assert!(m.contains("4 beats"), "{m}");
    assert!(m.contains("objects") || m.contains("object"), "{m}");
    // No figures, sequence or nesting words: the generic list.
    for structure in [
        "layers",
        "compare",
        "steps",
        "cycle",
        "hierarchy",
        "flow",
        "timeline",
    ] {
        assert!(m.contains(structure), "generic list names {structure}: {m}");
    }
    assert!(m.contains("Which structure fits which topic"), "{m}");
}

#[test]
fn a_layers_primary_ties_the_beats() {
    // Every beat about the same layers (the guide's recipe).
    let layered: Vec<Value> = ["Photic zone", "Twilight zone", "Dark zone", "Photic zone"]
        .iter()
        .map(|focus| beat("Where it happens", layers_subject(focus)))
        .collect();
    let w = warnings(&intent_of(layered), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");

    // Even one structured beat means the story has a structure.
    let mut facts = five_facts();
    facts[1] = beat("The twilight zone", layers_subject("Twilight zone"));
    let w = warnings(&intent_of(facts), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");
}

#[test]
fn compare_beats_tie_the_beats() {
    let mut facts = five_facts();
    facts[1] = json!({
        "purpose": "compare", "statement": "Coffee against tea",
        "primary": object("coffee_cup", "coffee"),
        "secondary": object("globe", "tea")
    });
    let w = warnings(&intent_of(facts), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");

    // A compare beat without a second side compares nothing.
    let mut lonely = five_facts();
    lonely[1]["purpose"] = json!("compare");
    let w = warnings(&intent_of(lonely), &options(None, None));
    assert_eq!(with_code(&w, WARN_UNRELATED_BEATS).len(), 1, "{w:?}");
}

#[test]
fn carry_primary_ties_the_beats() {
    let mut facts = five_facts();
    facts[0]["continuity"] = json!("carry_primary");
    let w = warnings(&intent_of(facts), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");

    // So does a relationship on any beat.
    let mut related = five_facts();
    related[2]["relationship"] = json!("accumulate");
    let w = warnings(&intent_of(related), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");
}

#[test]
fn a_subject_repeated_in_consecutive_beats_is_a_tie() {
    let mut facts = five_facts();
    facts[1] = beat("Coffee, again", object("coffee_cup", "coffee"));
    let w = warnings(&intent_of(facts), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");

    // The same text in a number or phrase counts too.
    let figures = vec![
        beat("The bridge opened", number("6 min", "by bridge")),
        beat("Six minutes", number("6 MIN", "crossing")),
        beat("A new commute", phrase("commute")),
    ];
    let w = warnings(&intent_of(figures), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");

    // A repeat that is not consecutive ties nothing.
    let mut far = five_facts();
    far[2] = beat("Coffee at the end", object("coffee_cup", "coffee"));
    let w = warnings(&intent_of(far), &options(None, None));
    assert_eq!(with_code(&w, WARN_UNRELATED_BEATS).len(), 1, "{w:?}");
}

#[test]
fn a_story_told_in_phrases_and_numbers_is_narrative_not_a_slideshow() {
    let narrative = vec![
        beat("Sales fell to 800", number("800", "sales")),
        beat("Visitors rose to 5,000", number("5,000", "visitors")),
        beat("Something had to give", phrase("tension")),
        beat("So we changed the hours", phrase("change")),
    ];
    let w = warnings(&intent_of(narrative), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");
}

#[test]
fn fewer_than_three_beats_are_never_unrelated() {
    let two: Vec<Value> = five_facts().into_iter().take(2).collect();
    let w = warnings(&intent_of(two), &options(None, None));
    assert!(with_code(&w, WARN_UNRELATED_BEATS).is_empty(), "{w:?}");
}

#[test]
fn suggestions_follow_the_words_of_the_beats() {
    let message = |beats: Vec<Value>| -> String {
        let w = warnings(&intent_of(beats), &options(None, None));
        let hits = with_code(&w, WARN_UNRELATED_BEATS);
        assert_eq!(hits.len(), 1, "{w:?}");
        hits[0].message.clone()
    };

    // (The warning needs picture beats; the figures live in the statements.)
    // Figures across beats: compare / ratio.
    let figures = message(vec![
        beat("Sales fell to 800", object("chart_printout", "sales")),
        beat("Visitors rose to 5,000", object("receipt", "visitors")),
        beat("Staff stayed at 12", object("calendar_page", "staff")),
    ]);
    assert!(figures.contains("compare / ratio"), "{figures}");
    assert!(figures.contains("derived_metric"), "{figures}");
    assert!(!figures.contains("steps"), "{figures}");

    // A sequence: steps with carry_primary.
    let steps = message(vec![
        beat("First, pick a goal", object("magnifier", "goal")),
        beat("Then save a little", object("coin_stack", "saving")),
        beat("Finally, let it grow", object("plant", "growth")),
    ]);
    assert!(steps.contains("steps (beats in order"), "{steps}");
    assert!(steps.contains("carry_primary"), "{steps}");
    assert!(!steps.contains("compare / ratio"), "{steps}");

    // Nested things: layers.
    let nested = message(vec![
        beat("The sunlit zone", object("globe", "sunlight")),
        beat("Deeper, the twilight zone", object("lightbulb", "twilight")),
        beat("Below that, the dark level", object("door", "darkness")),
    ]);
    assert!(nested.contains("layers (one `layers` primary"), "{nested}");
    assert!(!nested.contains("cycle"), "{nested}");
}

// ---------------------------------------------------------------------------
// carry_ignored
// ---------------------------------------------------------------------------

fn cinematic() -> Option<ArtMode> {
    Some(ArtMode::Force(Look::Cinematic3d))
}

#[test]
fn a_carried_library_picture_in_the_cinematic_look_is_reported() {
    let mut first = beat("Savings pile up", object("coin_stack", "savings"));
    first["continuity"] = json!("carry_primary");
    let intent = intent_of(vec![
        first,
        beat("Time passes", object("clock", "time")),
        beat("Then it pays off", object("lightbulb", "idea")),
    ]);
    let (project, w) = report(&intent, &options(cinematic(), None));
    assert!(project.shared.is_empty(), "the cinematic look carried it");
    let hits = with_code(&w, WARN_CARRY_IGNORED);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert_eq!(hits[0].beat, Some(0));
    let m = &hits[0].message;
    assert!(m.contains("carry_primary"), "{m}");
    assert!(m.contains("coin_stack"), "which subject: {m}");
    assert!(m.contains("cinematic_3d"), "the look is named: {m}");
}

#[test]
fn a_carry_that_worked_is_not_reported() {
    // The guide's bridge story: the number really is carried (one shared
    // element) under the default look.
    let mut first = beat("The ferry took forty minutes", number("40 min", "crossing"));
    first["continuity"] = json!("carry_primary");
    let intent = intent_of(vec![
        first,
        json!({
            "purpose": "contrast", "statement": "Then the bridge opened",
            "primary": number("40 min", "crossing"),
            "secondary": number("6 min", "by bridge"), "relationship": "replace"
        }),
    ]);
    let (project, w) = report(&intent, &options(None, None));
    assert_eq!(project.shared.len(), 1, "{w:?}");
    assert!(with_code(&w, WARN_CARRY_IGNORED).is_empty(), "{w:?}");
}

#[test]
fn other_dropped_carries_say_why() {
    // carry_secondary without a secondary.
    let mut a = beat("One", phrase("one"));
    a["continuity"] = json!("carry_secondary");
    // carry_primary on an explain beat (the primary is the headline).
    let mut b = beat("How it works", phrase("pressure"));
    b["purpose"] = json!("explain");
    b["continuity"] = json!("carry_primary");
    // carry on the last beat: nothing follows it.
    let mut c = beat("The end", phrase("end"));
    c["continuity"] = json!("carry_primary");
    let w = warnings(&intent_of(vec![a, b, c]), &options(None, None));
    let hits = with_code(&w, WARN_CARRY_IGNORED);
    let by_beat = |i: usize| -> &str {
        hits.iter()
            .find(|h| h.beat == Some(i))
            .map(|h| h.message.as_str())
            .unwrap_or_else(|| panic!("beat {i}: {w:?}"))
    };
    assert!(by_beat(0).contains("no secondary"), "{}", by_beat(0));
    assert!(by_beat(1).contains("explain"), "{}", by_beat(1));
    assert!(by_beat(2).contains("last beat"), "{}", by_beat(2));
    assert_eq!(hits.len(), 3, "{w:?}");
}

// ---------------------------------------------------------------------------
// title_states_conclusion
// ---------------------------------------------------------------------------

/// A voice-over for `lines` (one per beat): each word 0.4 s apart, 0.3 s of
/// silence between sentences; repaired the way the CLI loads it.
fn speech_for(lines: &[&str]) -> SpeechMap {
    let (mut words, mut sentences, mut t) = (Vec::new(), Vec::new(), 0.3);
    for (beat, line) in lines.iter().enumerate() {
        let start = t;
        for token in line.split_whitespace() {
            let text: String = token
                .trim_matches(|c: char| !c.is_alphanumeric())
                .to_string();
            words.push(SpeechWord {
                text,
                start: t,
                end: t + 0.3,
                confidence: 1.0,
            });
            t += 0.4;
        }
        sentences.push(SpeechSentence {
            beat,
            start,
            end: t,
        });
        t += 0.3;
    }
    let map = SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: t + 0.9,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
        recognised: Vec::new(),
        alignment: None,
    };
    let spoken: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
    repair(&map, &spoken).0
}

const SUN_LINE: &str =
    "Sunlight has a long way to go before it reaches us. It takes eight minutes.";

fn sun_story(title: &str, keyword: Option<&str>) -> (CreativeIntent, SpeechMap) {
    let mut b = beat(title, number("8 min", "travel time"));
    b["narration"] = json!(SUN_LINE);
    if let Some(k) = keyword {
        b["keyword"] = json!(k);
    }
    (intent_of(vec![b]), speech_for(&[SUN_LINE]))
}

#[test]
fn a_title_that_states_the_number_before_the_narrator_does_is_reported() {
    let (intent, speech) = sun_story("Light takes 8 minutes", None);
    let w = warnings(&intent, &options(None, Some(speech)));
    let hits = with_code(&w, WARN_TITLE_STATES_CONCLUSION);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert_eq!(hits[0].beat, Some(0));
    let m = &hits[0].message;
    assert!(
        m.starts_with("title \"Light takes 8 minutes\" says \"8\" "),
        "{m}"
    );
    assert!(m.contains(" s before the narrator does; "), "{m}");
    assert!(m.contains("keep the title to the topic"), "{m}");
}

#[test]
fn a_number_in_words_is_still_the_same_number() {
    let (intent, speech) = sun_story("Eight minutes from the Sun", None);
    let w = warnings(&intent, &options(None, Some(speech)));
    assert_eq!(
        with_code(&w, WARN_TITLE_STATES_CONCLUSION).len(),
        1,
        "{w:?}"
    );
}

#[test]
fn a_keyword_in_the_title_that_the_narrator_says_late_is_reported() {
    let line = "Sunlight has a long way to go before it reaches us. Then it simply arrives.";
    let mut b = beat("Sunlight arrives", phrase("sun"));
    b["narration"] = json!(line);
    b["keyword"] = json!("arrives");
    let w = warnings(
        &intent_of(vec![b]),
        &options(None, Some(speech_for(&[line]))),
    );
    let hits = with_code(&w, WARN_TITLE_STATES_CONCLUSION);
    assert_eq!(hits.len(), 1, "{w:?}");
    assert!(
        hits[0].message.contains("says \"arrives\""),
        "{:?}",
        hits[0]
    );
}

#[test]
fn a_topic_title_is_not_reported() {
    let (intent, speech) = sun_story("Light from the Sun", None);
    let w = warnings(&intent, &options(None, Some(speech)));
    assert!(
        with_code(&w, WARN_TITLE_STATES_CONCLUSION).is_empty(),
        "{w:?}"
    );

    // The number said at the start of the line is no spoiler either.
    let early = "Eight minutes is all it takes for sunlight to reach us.";
    let mut b = beat("Light takes 8 minutes", number("8 min", "travel time"));
    b["narration"] = json!(early);
    let w = warnings(
        &intent_of(vec![b]),
        &options(None, Some(speech_for(&[early]))),
    );
    assert!(
        with_code(&w, WARN_TITLE_STATES_CONCLUSION).is_empty(),
        "{w:?}"
    );
}

#[test]
fn the_title_check_needs_speech_and_a_free_title_policy() {
    let (intent, speech) = sun_story("Light takes 8 minutes", None);
    // No voice-over: nothing to compare with.
    let w = warnings(&intent, &options(None, None));
    assert!(
        with_code(&w, WARN_TITLE_STATES_CONCLUSION).is_empty(),
        "{w:?}"
    );
    // The cinematic look holds titles for their word.
    let w = warnings(&intent, &options(cinematic(), Some(speech)));
    assert!(
        with_code(&w, WARN_TITLE_STATES_CONCLUSION).is_empty(),
        "{w:?}"
    );
}

// ---------------------------------------------------------------------------
// Warnings never reach the scene
// ---------------------------------------------------------------------------

#[test]
fn warnings_are_never_written_into_the_project() {
    let (spoiler, speech) = sun_story("Light takes 8 minutes", None);
    let mut carry = five_facts();
    carry[0]["continuity"] = json!("carry_primary");
    // Stories that raise every warning code, with and without art.
    let cases: Vec<(CreativeIntent, CompileOptions)> = vec![
        (intent_of(five_facts()), options(None, None)),
        (intent_of(carry), options(cinematic(), None)),
        (spoiler, options(None, Some(speech))),
    ];
    let style: StyleProfile = serde_json::from_value(json!({})).expect("style");
    let measure: &dyn TextMeasure = &ApproxMeasure;
    let mut seen = std::collections::BTreeSet::new();
    for (intent, opts) in &cases {
        let plain = compile_with_options(
            intent,
            &style,
            None,
            &library(),
            measure,
            &AssetManifest::empty(),
            None,
            opts,
        )
        .expect("compile");
        let (reported, w) = report(intent, opts);
        assert!(!w.is_empty(), "this case raises a warning");
        seen.extend(w.iter().map(|w| w.code.clone()));
        assert_eq!(
            plain.to_json_pretty(),
            reported.to_json_pretty(),
            "the report changes nothing"
        );
        let json = reported.to_json_pretty();
        for code in [
            WARN_UNRELATED_BEATS,
            WARN_CARRY_IGNORED,
            WARN_TITLE_STATES_CONCLUSION,
        ] {
            assert!(!json.contains(code), "{code} leaked into the scene");
        }
    }
    for code in [
        WARN_UNRELATED_BEATS,
        WARN_CARRY_IGNORED,
        WARN_TITLE_STATES_CONCLUSION,
    ] {
        assert!(seen.contains(code), "{code} never raised: {seen:?}");
    }
}

// ---------------------------------------------------------------------------
// The authoring guide's "Which structure fits which topic" examples
// ---------------------------------------------------------------------------

fn snippet(json: &str) -> Value {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("{e}: {json}"))
}

fn guide_story(beats: &[&str]) -> CreativeIntent {
    intent_of(beats.iter().map(|b| snippet(b)).collect())
}

#[test]
fn the_guides_structure_examples_compile_without_story_warnings() {
    let layers = |focus: &str| {
        format!(
            r#"{{ "purpose": "explain", "statement": "Where the light runs out",
              "primary": {{ "kind": "layers", "meaning": "ocean", "focus": "{focus}",
                "layers": [ {{ "name": "Sunlit zone" }}, {{ "name": "Twilight zone" }}, {{ "name": "Dark zone" }} ] }} }}"#
        )
    };
    let layered: Vec<String> = ["Sunlit zone", "Twilight zone", "Dark zone"]
        .iter()
        .map(|f| layers(f))
        .collect();
    let compare = r#"{ "purpose": "compare", "statement": "Ferry against bridge",
        "primary":   { "kind": "number", "value": "40 min", "meaning": "by ferry" },
        "secondary": { "kind": "number", "value": "6 min",  "meaning": "by bridge" } }"#;
    let steps = [
        r#"{ "purpose": "emphasize", "statement": "Step 1: mix the dough", "primary": { "kind": "phrase", "value": "Dough" }, "continuity": "carry_primary" }"#,
        r#"{ "purpose": "emphasize", "statement": "Step 2: let it rise", "primary": { "kind": "phrase", "value": "Dough" }, "continuity": "carry_primary" }"#,
        r#"{ "purpose": "emphasize", "statement": "Step 3: bake", "primary": { "kind": "phrase", "value": "Dough" } }"#,
    ];
    let hierarchy = [
        r#"{ "purpose": "explain", "statement": "A cell is mostly parts", "primary": { "kind": "phrase", "value": "Cell" },
            "secondary": { "kind": "phrase", "value": "Nucleus" }, "continuity": "carry_secondary" }"#,
        r#"{ "purpose": "emphasize", "statement": "The nucleus holds the DNA", "primary": { "kind": "phrase", "value": "Nucleus" } }"#,
    ];
    let flow = r#"{ "purpose": "contrast", "relationship": "compress", "statement": "The firewall stops the traffic",
        "primary":   { "kind": "object", "asset": "cloud_data", "meaning": "traffic" },
        "secondary": { "kind": "object", "asset": "shield",     "meaning": "firewall" } }"#;
    let timeline = r#"{ "purpose": "explain", "statement": "Nine centuries of firsts",
        "primary": { "kind": "collection", "meaning": "timeline",
          "items": [ { "kind": "phrase", "value": "1096: Oxford starts teaching" },
                     { "kind": "phrase", "value": "1325: Tenochtitlan is founded" },
                     { "kind": "phrase", "value": "1969: the first Moon landing" } ] } }"#;

    let layered_refs: Vec<&str> = layered.iter().map(String::as_str).collect();
    // (name, story, art, shared elements the carry should produce)
    let cases: Vec<(&str, CreativeIntent, Option<ArtMode>, usize)> = vec![
        ("layers", guide_story(&layered_refs), None, 0),
        ("compare", guide_story(&[compare]), None, 0),
        ("steps", guide_story(&steps), None, 1),
        ("hierarchy", guide_story(&hierarchy), None, 1),
        ("flow", guide_story(&[flow]), cinematic(), 0),
        ("timeline", guide_story(&[timeline]), None, 0),
    ];
    for (name, intent, art, shared) in cases {
        let (project, w) = report(&intent, &options(art, None));
        assert!(w.is_empty(), "{name}: {w:?}");
        assert_eq!(project.shared.len(), shared, "{name}: shared elements");
    }
}
