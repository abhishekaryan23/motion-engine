//! Picture lookup against the real `assets/library` (plan §8 Phase 1): the
//! index agrees with the compiler, suggestions round-trip, nonsense finds
//! nothing.

mod common;

use motion_core::compiler::catalog::{request_words, MatchKind};
use motion_core::AssetLibrary;
use motion_mcp::lite::snake;
use motion_mcp::pictures::Strength;

fn families(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// Object ids of one family catalog (role object, qa PASS/WARN).
fn catalog_ids(family: &str) -> Vec<String> {
    let path = common::repo().join(format!("assets/library/{family}/catalog.json"));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    v["assets"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|a| a["role"] == "object" && matches!(a["qa"].as_str(), Some("PASS" | "WARN")))
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_index_loads_the_library() {
    let p = common::pictures();
    assert!(p.len() > 500, "{} pictures", p.len());
    assert!(p.family_names().contains(&"clay_props_3d".to_string()));
    // The editorial look's families come first in `find`.
    assert_eq!(p.default_families()[0], "people_everyday");
    assert!(!p.default_families().contains(&"grounds".to_string()));
}

#[test]
fn known_catalog_ids_are_pictures() {
    let p = common::pictures();
    let fams = families(&["clay_props_3d", "sketch_icons"]);
    for id in catalog_ids("clay_props_3d") {
        assert!(p.is_picture_in(&id, &fams), "{id}");
        assert_eq!(
            p.shown_as(&id, &[], &fams).as_deref(),
            Some(id.as_str()),
            "{id}"
        );
    }
    // Only in the families asked for (`--art none`: named objects only).
    let clay = &catalog_ids("clay_props_3d")[0];
    assert!(!p.is_picture_in(clay, &[]), "{clay} without its family");
    assert!(
        p.is_picture_in("shopping_basket", &[]),
        "named objects need no family"
    );
    assert!(!p.is_picture_in("zxqv_blorp", &fams));
}

/// The index answers exactly what the compiler would do.
#[test]
fn the_index_agrees_with_the_compiler() {
    let p = common::pictures();
    let assets = common::repo().join("assets");
    let words = [
        "rocket",
        "robot",
        "money_stack",
        "brain_network",
        "alarm_clock",
        "person_exercising",
        "smiley_face",
        "cycle_diagram",
        "berries",
        "zorb",
        "coffee_cup",
        "piggy_bank",
        "chess",
        "rupee",
        "shopping_basket",
        "growth_chart",
        "the",
        "a_b",
        "heart",
        "light_bulb",
    ];
    let looks = [
        families(&["clay_props_3d", "clay_concepts_3d", "people_everyday"]),
        families(&[
            "classical_greyscale",
            "classical_concepts",
            "editorial_cutout",
        ]),
        p.default_families().to_vec(),
        Vec::new(),
    ];
    for fams in &looks {
        let lib = AssetLibrary::new(&assets).with_families(fams.clone());
        for w in words {
            let asset = snake(w);
            let named = lib.find_object(&asset).map(|_| asset.clone());
            let catalog = lib
                .catalog_match(&request_words(&[asset.as_str()]), MatchKind::Object)
                .map(|m| m.id);
            let compiler = named.or(catalog);
            assert_eq!(p.shown_as(w, &[], fams), compiler, "{w} in {fams:?}");
            assert_eq!(p.is_picture_in(w, fams), compiler.is_some(), "{w}");
        }
    }
}

#[test]
fn suggestions_round_trip() {
    let p = common::pictures();
    let fams = p.default_families().to_vec();
    for w in [
        "rokcet",
        "rockets",
        "berries",
        "piggybank",
        "boxes",
        "coins",
        "brains",
        "clocks",
    ] {
        let s = p.suggest_scored(w, 3, &fams);
        assert!(!s.is_empty(), "{w}: no suggestion");
        for (name, _) in &s {
            assert!(
                p.is_picture_in(name, &fams),
                "{w} → {name} is not a picture"
            );
        }
        let mut names: Vec<&String> = s.iter().map(|(n, _)| n).collect();
        names.dedup();
        assert_eq!(names.len(), s.len(), "{w}: duplicates");
        // Ranked by strength.
        assert!(s.windows(2).all(|x| x[0].1 >= x[1].1), "{w}: {s:?}");
    }
    let s = p.suggest_scored("rokcet", 3, &fams);
    assert_eq!(s[0], ("rocket".to_string(), Strength::Spelling));
    assert!(
        !s[0].1.is_strong(),
        "a typo is never applied without asking"
    );
    let s = p.suggest_scored("piggybank", 3, &fams);
    assert_eq!(s[0], ("piggy_bank".to_string(), Strength::SameWords));
    // Deterministic.
    assert_eq!(
        p.suggest_in("berries", 3, &fams),
        p.suggest_in("berries", 3, &fams)
    );
}

#[test]
fn find_returns_pictures_or_nothing() {
    let p = common::pictures();
    let words: Vec<String> = ["rocket", "zxqvblorp", "piggybank", ""]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let found = p.find(&words, 3);
    assert_eq!(found["rocket"][0], "rocket");
    assert!(found["zxqvblorp"].is_empty());
    assert_eq!(found["piggybank"][0], "piggy_bank");
    assert!(found[""].is_empty());
    for names in found.values() {
        assert!(names.len() <= 3);
        for n in names {
            assert!(p.is_picture_in(n, p.default_families()), "{n}");
        }
    }
}

#[test]
fn a_tag_one_letter_away_is_not_a_typo() {
    // "shark" is one letter from the pie chart's tag "share": not a suggestion.
    let p = common::pictures();
    let fams = p.default_families().to_vec();
    let suggestions = p.suggest_in("shark", 3, &fams);
    assert!(
        !suggestions.iter().any(|s| s.contains("chart")),
        "{suggestions:?}"
    );
}
