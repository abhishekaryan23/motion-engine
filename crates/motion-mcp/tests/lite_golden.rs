//! Lite story → CreativeIntent v0.2 goldens (plan §5 mapping table).
//!
//! Every `tests/golden/<case>.lite.json` is parsed leniently
//! (`policy::parse_story`) and mapped with `lite::to_intent` over a FIXED
//! picture set (not the real library, so the goldens stay stable when the
//! library grows); the result must equal `<case>.intent.json` and validate.
//! `MOTION_UPDATE_GOLDEN=1` rewrites the goldens (alongside a
//! `<case>.notes.json` with the parsing and mapping notes).

use std::path::{Path, PathBuf};

use motion_core::intent::Format;
use motion_mcp::lite::{self, Pictures};
use motion_mcp::policy::{self, Story};

/// The library the goldens pretend to have.
struct Fixed;

const FIXED: &[&str] = &[
    "rocket",
    "robot",
    "microchip",
    "brain_circuit",
    "chat_bubbles",
    "money_stack",
    "piggy_bank",
    "coffee_cup",
    "clock",
    "fish",
];

impl Pictures for Fixed {
    fn is_picture(&self, noun: &str) -> bool {
        FIXED.contains(&lite::snake(noun).as_str())
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn cases() -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = std::fs::read_dir(golden_dir())
        .expect("golden dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| {
            let name = p
                .file_name()?
                .to_str()?
                .strip_suffix(".lite.json")?
                .to_string();
            Some((name, p))
        })
        .collect();
    out.sort();
    out
}

fn map_case(path: &Path) -> (serde_json::Value, Vec<String>) {
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).expect("read case")).expect("json");
    let parsed = policy::parse_story(&raw).unwrap_or_else(|f| panic!("{path:?}: {f:?}"));
    let Story::Lite(story) = parsed.story else {
        panic!("{path:?} parsed as a full intent")
    };
    let mapped = lite::to_intent(&story, Format::Vertical, &Fixed);
    assert!(
        mapped.intent.validate().is_ok(),
        "{path:?}: {:?}",
        mapped.intent.validate()
    );
    let mut notes = parsed.notes;
    notes.extend(mapped.notes);
    (
        serde_json::to_value(&mapped.intent).expect("intent json"),
        notes,
    )
}

#[test]
fn lite_stories_map_to_their_goldens() {
    let update = std::env::var("MOTION_UPDATE_GOLDEN").is_ok_and(|v| v == "1");
    let cases = cases();
    assert!(cases.len() >= 8, "only {} golden cases", cases.len());
    let mut failed = Vec::new();
    for (name, path) in &cases {
        let (intent, notes) = map_case(path);
        let text = serde_json::to_string_pretty(&intent).expect("pretty") + "\n";
        let notes_text = serde_json::to_string_pretty(&notes).expect("pretty") + "\n";
        let golden = golden_dir().join(format!("{name}.intent.json"));
        let golden_notes = golden_dir().join(format!("{name}.notes.json"));
        if update {
            std::fs::write(&golden, &text).expect("write golden");
            std::fs::write(&golden_notes, &notes_text).expect("write notes");
            continue;
        }
        let want = std::fs::read_to_string(&golden).unwrap_or_default();
        let want_notes = std::fs::read_to_string(&golden_notes).unwrap_or_default();
        if want != text || want_notes != notes_text {
            failed.push(name.clone());
        }
    }
    assert!(
        failed.is_empty(),
        "goldens differ: {failed:?} (MOTION_UPDATE_GOLDEN=1 after an intended change)"
    );
}

#[test]
fn mapping_is_pure() {
    for (_, path) in cases() {
        assert_eq!(map_case(&path), map_case(&path));
    }
}

/// Spot checks of the table rows, so a wrong golden cannot hide them.
#[test]
fn table_rows_hold() {
    let get = |name: &str| map_case(&golden_dir().join(format!("{name}.lite.json")));
    let (i, _) = get("number_picture");
    assert_eq!(i["title"], "buffett_cash");
    assert_eq!(i["beats"][0]["primary"]["kind"], "number");
    assert_eq!(i["beats"][0]["secondary"]["asset"], "money_stack");
    assert_eq!(i["beats"][0]["purpose"], "emphasize");
    assert_eq!(i["beats"][2]["purpose"], "reveal");
    assert_eq!(i["beats"][0]["statement"], "Buffett's cash pile");
    assert_eq!(
        i["beats"][0]["narration"],
        "Warren Buffett is sitting on 381 billion dollars in cash."
    );

    let (i, _) = get("list_items");
    let items = i["beats"][0]["primary"]["items"].as_array().expect("items");
    assert_eq!(items[0]["kind"], "object");
    assert_eq!(items[4]["kind"], "phrase");

    let (i, _) = get("compare_how");
    assert_eq!(i["beats"][0]["purpose"], "compare");
    assert_eq!(i["beats"][0]["relationship"], "separate");
    assert_eq!(i["beats"][1]["relationship"], "grow");
    assert_eq!(i["beats"][1]["secondary"]["kind"], "object");

    let (i, _) = get("layers_focus");
    assert_eq!(i["beats"][1]["primary"]["kind"], "layers");
    assert_eq!(i["beats"][1]["primary"]["focus"], "twilight");
    assert_eq!(i["beats"][1]["secondary"]["asset"], "fish");
    assert_eq!(i["beats"][2]["secondary"]["kind"], "number");

    let (i, notes) = get("priority_conflicts");
    assert_eq!(i["beats"][0]["primary"]["kind"], "collection");
    assert_eq!(i["beats"][1]["purpose"], "compare");
    assert_eq!(i["beats"][2]["primary"]["kind"], "layers");
    assert_eq!(i["beats"][3]["primary"]["kind"], "state_change");
    assert!(notes
        .iter()
        .any(|n| n == "beat 1: 'number' not shown (the beat shows a list)"));

    let (i, _) = get("phrase_only");
    assert_eq!(
        i["title"], "the_bakery_won",
        "a missing title comes from beat 1"
    );
    assert_eq!(i["beats"][1]["primary"]["value"], "craft");

    let (i, _) = get("file_pictures");
    // A person photo is a phrase (the compiler then uses beat_N.hero_subject).
    assert_eq!(i["beats"][0]["primary"]["kind"], "phrase");
    assert_eq!(i["beats"][1]["primary"]["asset"], "photo");
    assert_eq!(i["beats"][2]["primary"]["asset"], "beach_day");
    assert_eq!(i["beats"][3]["secondary"]["kind"], "phrase");

    let (i, notes) = get("lenient");
    assert_eq!(
        i["beats"][0]["narration"],
        "Plain strings are read as what the narrator says out loud."
    );
    assert_eq!(i["beats"][1]["primary"]["value"], "381");
    assert_eq!(i["beats"][3]["purpose"], "compare");
    assert_eq!(i["beats"][3]["energy"], "impact");
    assert_eq!(i["beats"][4]["primary"]["asset"], "money_stack");
    assert_eq!(i["beats"][5]["narration"], "Say taken from show");
    assert!(notes
        .iter()
        .any(|n| n.contains("ignored unknown field 'colour'")));
}
