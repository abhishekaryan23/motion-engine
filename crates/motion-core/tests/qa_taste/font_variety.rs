//! (0.24) Font variety: `--explore 1` (the CLI default) reaches the curated
//! emotion options, and an explicit typography style is a category
//! constraint, not a pin. Level 0 stays the exact 0.8 pairing.

use std::collections::BTreeSet;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::taste::resolve_with;
use motion_core::compiler::typography::{option_faces, resolve_typography, TypographyRequest};
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{FontRole, MotionProject};
use motion_core::style::{StyleProfile, Tone, TypographyStyle};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

const PAIRS: [(&str, &str); 6] = [
    (
        "examples/editorial_demo.intent.json",
        "examples/editorial_demo.style.json",
    ),
    (
        "examples/motion_language_demo.intent.json",
        "examples/motion_language_demo.style.json",
    ),
    (
        "examples/facts/five_wait_what.intent.json",
        "examples/facts/five_wait_what.style.json",
    ),
    (
        "examples/topics/ai_learns.intent.json",
        "examples/topics/ai_learns.style.json",
    ),
    (
        "examples/topics/ai_opener.intent.json",
        "examples/topics/ai_opener.style.json",
    ),
    (
        "examples/topics/vaccines.intent.json",
        "examples/topics/vaccines.style.json",
    ),
];

fn compile(intent: &CreativeIntent, style: &StyleProfile, level: u8, seed: u64) -> MotionProject {
    let opts = CompileOptions {
        explore: level,
        seed,
        ..CompileOptions::default()
    };
    compile_with_options(
        intent,
        style,
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compiles")
}

/// Display face id of the project's typography (legacy pairing when no option).
fn display_face(p: &MotionProject) -> String {
    match p.theme.typography.as_ref().and_then(|c| c.option.as_ref()) {
        Some(o) => option_faces(o).expect("option")[&FontRole::Display]
            .asset_id
            .to_string(),
        None => "legacy".to_string(),
    }
}

fn load(pair: (&str, &str)) -> (CreativeIntent, StyleProfile) {
    let intent = CreativeIntent::from_json(&read(pair.0)).expect("intent");
    let style = serde_json::from_str(&read(pair.1)).expect("style");
    (intent, style)
}

#[test]
fn level_one_uses_at_least_three_display_faces_across_six_intents() {
    let faces: Vec<String> = PAIRS
        .iter()
        .map(|p| {
            let (i, s) = load(*p);
            display_face(&compile(&i, &s, 1, 0))
        })
        .collect();
    let distinct: BTreeSet<&String> = faces.iter().collect();
    // (0.24 M8) Level 1 draws the 5 motion-safe library-v2 pairings by genre.
    assert!(distinct.len() >= 3, "only {distinct:?} from {faces:?}");
    // The Classic tone keeps its legacy pairing as one candidate among many.
    let legacy = faces.iter().filter(|f| *f == "legacy").count();
    assert!(legacy <= 1, "level 1 fell back to legacy: {faces:?}");
    eprintln!("display faces: {faces:?}");
}

#[test]
fn same_intent_and_seed_give_the_same_faces() {
    let (i, s) = load(PAIRS[0]);
    let a = compile(&i, &s, 1, 7);
    let b = compile(&i, &s, 1, 7);
    assert_eq!(a.theme.typography, b.theme.typography);
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
}

#[test]
fn level_zero_keeps_the_legacy_pairing() {
    for p in PAIRS {
        let (i, s) = load(p);
        let project = compile(&i, &s, 0, 0);
        assert!(project.theme.typography.is_none(), "{p:?}");
    }
}

#[test]
fn grotesk_serif_style_picks_a_grotesk_display_with_a_serif_voice() {
    // Classic tone: GroteskSerif is the pairing the style asks for.
    let style = StyleProfile {
        tone: Tone::Studio,
        typography_style: TypographyStyle::GroteskSerif,
        ..StyleProfile::default()
    };
    let (intent, _) = load(PAIRS[0]);
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for seed in 0..12 {
        let project = compile(&intent, &style, 1, seed);
        let Some(option) = project.theme.typography.and_then(|c| c.option) else {
            // The legacy GroteskSerif pairing is itself a grotesk + serif voice.
            continue;
        };
        let faces = option_faces(&option).expect("option");
        let class = |r: FontRole| {
            let id = faces[&r].asset_id;
            let reg: serde_json::Value =
                serde_json::from_str(&read("assets/fonts/registry.json")).expect("registry");
            reg["faces"]
                .as_array()
                .expect("faces")
                .iter()
                .find(|f| f["id"] == id)
                .and_then(|f| f["class"].as_str().map(str::to_string))
                .expect("class")
        };
        assert!(
            matches!(
                class(FontRole::Display).as_str(),
                "grotesk" | "condensed" | "geometric"
            ),
            "{option}: display {}",
            class(FontRole::Display)
        );
        assert!(
            matches!(class(FontRole::SerifEmotional).as_str(), "serif" | "slab"),
            "{option}: voice {}",
            class(FontRole::SerifEmotional)
        );
        seen.insert(option);
    }
    assert!(!seen.is_empty(), "never left the legacy pairing");
}

#[test]
fn explicit_condensed_mono_is_a_constraint_not_a_pin() {
    let style = StyleProfile {
        tone: Tone::Technical,
        typography_style: TypographyStyle::CondensedMono,
        ..StyleProfile::default()
    };
    let taste = resolve_with(&style, None, 0);
    assert!(taste.typography_explicit);
    let root = repo().join("assets");
    let mut options = BTreeSet::new();
    for seed in 0..16 {
        let req = TypographyRequest {
            exploration: 1,
            seed,
            story_key: 5,
            emotion: None,
        };
        let (choice, _) = resolve_typography(&taste, &req, &root);
        options.insert(choice.option.expect("a registry option, not the pin"));
    }
    assert!(options.len() > 1, "{options:?}");
    let zero = TypographyRequest {
        exploration: 0,
        seed: 0,
        story_key: 5,
        emotion: None,
    };
    assert!(resolve_typography(&taste, &zero, &root).0.option.is_none());
}
