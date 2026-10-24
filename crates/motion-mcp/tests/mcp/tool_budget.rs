//! Tool-definition budgets and profile separation (plan §1, §4, §14).

use motion_core::audio::MusicWord;
use motion_mcp::profile::{budget_chars, tool_names, Profile};
use motion_mcp::schema::{defs_chars, tool_defs};
use serde_json::{json, Value};

#[test]
fn every_profile_fits_its_budget() {
    for p in [Profile::Weak, Profile::Creator, Profile::Operator] {
        let defs = tool_defs(p, true);
        let chars = defs_chars(&defs);
        eprintln!("{}: {} tools, {chars} chars", p.name(), defs.len());
        assert!(
            chars <= budget_chars(p),
            "{} tool definitions are {chars} chars, budget {}",
            p.name(),
            budget_chars(p)
        );
    }
}

/// Fields and tools only stronger profiles may see.
const NOT_WEAK: &[&str] = &[
    "\"options\"",
    "\"art\"",
    "\"families\"",
    // (0.23) The top-level `music` word is offered to every profile (see
    // `music_word_is_offered_to_every_profile_on_make_and_revise_only`); the
    // creator's `options.music` (a bed id) stays behind `"options"`.
    "\"captions\"",
    "\"aspect\"",
    "\"variety\"",
    "\"tts_model\"",
    "\"keep_frames\"",
    "\"force\"",
    "\"patch\"",
    "\"purpose\"",
    "\"statement\"",
    "\"narration\"",
    "\"continuity\"",
    "\"polarity\"",
    "\"motion_language\"",
    "\"seed\"",
    "\"version\"",
];

#[test]
fn weak_never_sees_creator_or_operator_fields() {
    let defs = tool_defs(Profile::Weak, true);
    let names: Vec<&str> = defs.iter().map(|d| d.name).collect();
    assert_eq!(names, tool_names(Profile::Weak, true));
    for d in &defs {
        let wire = d.wire_json().to_string();
        for field in NOT_WEAK {
            assert!(!wire.contains(field), "weak {} exposes {field}", d.name);
        }
    }
    // A lite beat has no intent subject (`primary`); the only `primary` a
    // weak model sees is the brand colour (0.21).
    let make = defs.iter().find(|d| d.name == "make_video").unwrap();
    let beat = &make.input_schema["properties"]["story"]["properties"]["beats"]["items"];
    assert!(beat["properties"].get("primary").is_none(), "{beat}");
    assert!(make.input_schema["properties"]["brand"]["properties"]
        .get("primary")
        .is_some());
}

/// (0.23) `take` is offered to every profile, on `make_video` and
/// `revise_video` only, with one short description, and is never required.
#[test]
fn take_is_offered_to_every_profile_on_make_and_revise_only() {
    for p in [Profile::Weak, Profile::Creator, Profile::Operator] {
        for d in tool_defs(p, true) {
            let take = &d.input_schema["properties"]["take"];
            if d.name == "make_video" || d.name == "revise_video" {
                assert_eq!(take["type"], "integer", "{} {}", p.name(), d.name);
                assert_eq!(take["minimum"], 0);
                assert_eq!(take["maximum"], 99);
                assert_eq!(
                    take["description"],
                    "another version of the same story: 1, 2, 3 …"
                );
                let required = d.input_schema["required"].to_string();
                assert!(!required.contains("take"), "{} {}", p.name(), d.name);
            } else {
                assert!(
                    !d.wire_json().to_string().contains("\"take\""),
                    "{} {} mentions take",
                    p.name(),
                    d.name
                );
            }
        }
    }
}

/// (0.23 C4b) `music` is offered to every profile, on `make_video` and
/// `revise_video` only, as the eight words of `audio::MusicWord` with one
/// short description, and is never required. The creator's `options.music`
/// (a bed id) is a different field and stays creator-only.
#[test]
fn music_word_is_offered_to_every_profile_on_make_and_revise_only() {
    let words: Vec<Value> = MusicWord::ALL.iter().map(|w| json!(w.as_str())).collect();
    assert_eq!(words.len(), 8);
    for p in [Profile::Weak, Profile::Creator, Profile::Operator] {
        for d in tool_defs(p, true) {
            let music = &d.input_schema["properties"]["music"];
            if d.name == "make_video" || d.name == "revise_video" {
                assert_eq!(music["type"], "string", "{} {}", p.name(), d.name);
                assert_eq!(music["enum"], json!(words), "{} {}", p.name(), d.name);
                assert_eq!(
                    music["description"],
                    "music mood; auto reads the story, none = no music"
                );
                let required = d.input_schema["required"].to_string();
                assert!(!required.contains("music"), "{} {}", p.name(), d.name);
            } else {
                // (`list_options` has a topic called "music"; it is not this field.)
                assert!(
                    d.input_schema["properties"].get("music").is_none(),
                    "{} {} takes a music word",
                    p.name(),
                    d.name
                );
            }
        }
    }
    // The weak profile's only `music` is that word: no bed ids, no options.
    let weak = tool_defs(Profile::Weak, true);
    for d in &weak {
        let wire = d.wire_json().to_string();
        assert!(!wire.contains("bed id"), "{}", d.name);
        let n = wire.matches("\"music\"").count();
        let want = usize::from(d.name == "make_video" || d.name == "revise_video");
        assert_eq!(n, want, "weak {}: {wire}", d.name);
    }
}

#[test]
fn pixel_and_timing_fields_never_appear_outside_the_escape_hatch() {
    // `time_s`/`frame` on operator frame tools and `times` on view_frames are
    // viewing queries; nothing lets a model author coordinates or timings.
    for p in [Profile::Weak, Profile::Creator, Profile::Operator] {
        for d in tool_defs(p, false) {
            let wire = d.wire_json().to_string();
            for field in [
                "\"x\"",
                "\"y\"",
                "\"width\"",
                "\"height\"",
                "\"duration\"",
                "\"start\"",
                "\"easing\"",
                "\"keyframes\"",
                "\"fps\"",
            ] {
                assert!(
                    !wire.contains(field),
                    "{} {} exposes {field}",
                    p.name(),
                    d.name
                );
            }
        }
    }
}

#[test]
fn creator_sees_the_full_intent_and_style() {
    let defs = tool_defs(Profile::Creator, false);
    let make = defs.iter().find(|d| d.name == "make_video").unwrap();
    let wire = make.wire_json().to_string();
    for field in [
        "\"purpose\"",
        "\"primary\"",
        "\"relationship\"",
        "\"continuity\"",
        "\"motion_language\"",
        "\"options\"",
    ] {
        assert!(wire.contains(field), "creator make_video lacks {field}");
    }
}
