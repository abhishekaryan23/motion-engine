//! (0.24 M8) Motion-safe type system: measured face metadata, the gate,
//! pairing library v2 and level selection.

use std::path::PathBuf;

use motion_core::compiler::taste::{resolve_with, ResolvedStyleProfile};
use motion_core::compiler::typography::{
    display_class, face_meta, gate_exclusions, level1_candidates, motion_safe, pairing_gate,
    pairing_ids, resolve_typography, TypographyRequest,
};
use motion_core::compiler::FontSet;
use motion_core::scene::FontRole;
use motion_core::style::{StyleProfile, Temperament as StyleTemperament, Tone};
use serde_json::Value;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// (x_height / cap_height, tabular digits) measured from a font file.
fn measure(path: &std::path::Path) -> Option<(f64, bool)> {
    let data = std::fs::read(path).ok()?;
    let face = ttf_parser::Face::parse(&data, 0).ok()?;
    let top = |c: char| {
        let g = face.glyph_index(c)?;
        face.glyph_bounding_box(g).map(|b| b.y_max as f64)
    };
    let ratio = top('x')? / top('H')?;
    let advances: Vec<f64> = ('0'..='9')
        .filter_map(|c| {
            face.glyph_index(c)
                .and_then(|g| face.glyph_hor_advance(g))
                .map(f64::from)
        })
        .collect();
    let tabular = advances.len() == 10 && {
        let (lo, hi) = advances
            .iter()
            .fold((f64::MAX, 0f64), |(l, h), a| (l.min(*a), h.max(*a)));
        (hi - lo) / hi <= 0.01
    };
    Some(((ratio * 1000.0).round() / 1000.0, tabular))
}

#[test]
fn dump_measurements() {
    let Ok(out) = std::env::var("MOTION_DUMP_FONT_META") else {
        return;
    };
    let reg: Value = serde_json::from_str(
        &std::fs::read_to_string(repo().join("assets/fonts/registry.json")).unwrap(),
    )
    .unwrap();
    let mut m = serde_json::Map::new();
    for f in reg["faces"].as_array().unwrap() {
        let file = f["file"].as_str().unwrap();
        if let Some((x, t)) = measure(&repo().join("assets").join(file)) {
            m.insert(
                f["id"].as_str().unwrap().to_string(),
                serde_json::json!({"x_height": x, "tabular_digits": t}),
            );
        }
    }
    std::fs::write(out, serde_json::to_string_pretty(&m).unwrap()).unwrap();
}

fn registry() -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(repo().join("assets/fonts/registry.json")).unwrap(),
    )
    .unwrap()
}

#[test]
fn metadata_matches_measured_files() {
    let reg = registry();
    let mut measured = 0;
    for f in reg["faces"].as_array().unwrap() {
        let id = f["id"].as_str().unwrap();
        assert!(
            ["low", "medium", "high"].contains(&f["contrast"].as_str().unwrap_or("")),
            "{id} contrast"
        );
        assert!(
            ["condensed", "normal", "extended"].contains(&f["width"].as_str().unwrap_or("")),
            "{id} width"
        );
        let w = f["weight"].as_u64().unwrap();
        assert!((100..=900).contains(&w), "{id} weight");
        let path = repo().join("assets").join(f["file"].as_str().unwrap());
        match measure(&path) {
            Some((x, t)) => {
                measured += 1;
                let rx = f["x_height"]
                    .as_f64()
                    .unwrap_or_else(|| panic!("{id} x_height"));
                assert!(
                    (rx - x).abs() < 0.0006,
                    "{id}: registry x_height {rx} vs measured {x}"
                );
                assert_eq!(
                    f["tabular_digits"].as_bool(),
                    Some(t),
                    "{id} tabular_digits"
                );
            }
            None => assert!(
                !path.is_file(),
                "{id}: file exists but could not be measured"
            ),
        }
    }
    // 28 open bundled faces; the 21 restricted faces (assets/fonts/RESTRICTED.md)
    // are measured too when they have been fetched.
    assert!(measured >= 28, "only {measured} faces measured");
}

#[test]
fn gate_excludes_known_bad_faces() {
    for id in [
        "font.zodiak_thin",
        "font.zodiak_light",
        "font.satoshi_light",
        "font.cabinet_light",
    ] {
        for role in FontRole::ALL {
            assert!(
                motion_safe(id, role, 2).is_err(),
                "{id} {role:?} must fail (weight < 400..600)"
            );
        }
    }
    // Light body faces fail on weight even though they are 300.
    assert!(motion_safe("font.satoshi_light", FontRole::Body, 20).is_err());
    // Panchang (extended): short display only.
    assert!(motion_safe("font.panchang_bold", FontRole::Display, 3).is_ok());
    assert!(motion_safe("font.panchang_bold", FontRole::Display, 4).is_err());
    assert!(motion_safe("font.panchang_extrabold", FontRole::Number, 1).is_err());
    // Body x-height and number tabular rules.
    assert!(motion_safe("font.satoshi_regular", FontRole::Body, 20).is_ok());
    assert!(motion_safe("font.zodiak_bold", FontRole::Number, 1).is_err());
    assert!(motion_safe("font.lato_bold", FontRole::Number, 1).is_ok());
    // A high-contrast regular may be display only.
    assert!(motion_safe("font.instrument_serif", FontRole::Display, 5).is_ok());
    assert!(motion_safe("font.instrument_serif", FontRole::DisplayCondensed, 5).is_err());
    assert!(motion_safe("font.instrument_serif", FontRole::Body, 5).is_err());
    assert!(face_meta("font.panchang_bold").is_some());

    let excluded: Vec<String> = gate_exclusions().into_iter().map(|e| e.id).collect();
    for id in ["calm.1", "luxury.2", "energy.1", "urgency.2"] {
        assert!(
            excluded.iter().any(|e| e == id),
            "{id} should be excluded: {excluded:?}"
        );
    }
}

#[test]
fn every_pairing_passes_the_gate() {
    let ids = pairing_ids();
    assert_eq!(ids.len(), 5);
    for id in ids {
        assert_eq!(pairing_gate(&id), Ok(()), "{id}");
    }
}

fn taste(tone: Tone, temperament: StyleTemperament) -> ResolvedStyleProfile {
    resolve_with(
        &StyleProfile {
            tone,
            temperament,
            ..StyleProfile::default()
        },
        None,
        0,
    )
}

fn req(level: u8, seed: u64, story_key: u64) -> TypographyRequest {
    TypographyRequest {
        exploration: level,
        seed,
        story_key,
        emotion: None,
    }
}

#[test]
fn level0_is_the_legacy_pairing() {
    let root = repo().join("assets");
    for tone in [
        Tone::Auto,
        Tone::Editorial,
        Tone::Technical,
        Tone::Playful,
        Tone::Documentary,
    ] {
        let t = taste(tone, StyleTemperament::Balanced);
        for seed in 0..4 {
            let (choice, fonts) = resolve_typography(&t, &req(0, seed, seed * 7), &root);
            assert!(choice.option.is_none());
            assert_eq!(
                format!("{fonts:?}"),
                format!("{:?}", FontSet::for_pairing(t.typography))
            );
        }
    }
}

#[test]
fn level1_draws_only_pairings_and_is_deterministic() {
    let root = repo().join("assets");
    let v2 = pairing_ids();
    let mut seen = std::collections::BTreeSet::new();
    for tone in [
        Tone::Editorial,
        Tone::Technical,
        Tone::Playful,
        Tone::Documentary,
        Tone::Cinematic,
        Tone::Hype,
    ] {
        let t = taste(tone, StyleTemperament::Balanced);
        for seed in 0..24 {
            let a = resolve_typography(&t, &req(1, seed, seed * 31 + 5), &root);
            let b = resolve_typography(&t, &req(1, seed, seed * 31 + 5), &root);
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
            match &a.0.option {
                Some(id) => {
                    assert!(v2.contains(id), "{id} is not a library-v2 pairing");
                    // The Classic tone is a category constraint (grotesk display + serif
                    // voice) and may widen beyond the usage matrix.
                    if format!("{:?}", t.tone) != "Classic" {
                        assert!(level1_candidates(&t).contains(id), "{tone:?} {id}");
                    }
                    seen.insert(id.clone());
                }
                None => assert_eq!(
                    format!("{:?}", a.1),
                    format!("{:?}", FontSet::for_pairing(t.typography))
                ),
            }
        }
    }
    assert!(seen.len() >= 4, "level 1 should vary: {seen:?}");
}

#[test]
fn usage_matrix_matches_genre() {
    // Finance / news explainer = documentary genre, technical tone.
    let finance = resolve_with(
        &StyleProfile {
            tone: Tone::Documentary,
            ..StyleProfile::default()
        },
        None,
        0,
    );
    let ids = level1_candidates(&finance);
    assert!(!ids.is_empty());
    for id in &ids {
        let c = display_class(id).unwrap();
        assert!(
            c == "serif" || c == "slab",
            "{id}: documentary history wants serif display, got {c}"
        );
    }
    // Technical + standard genre: grotesk / slab / sans pairings only.
    let tech = taste(Tone::Technical, StyleTemperament::Balanced);
    let ids = level1_candidates(&tech);
    assert!(!ids.is_empty());
    for id in &ids {
        let c = display_class(id).unwrap();
        assert!(
            ["grotesk", "sans", "slab"].contains(&c.as_str()),
            "{id}: {c}"
        );
    }
    // Editorial standard includes a serif-display pairing.
    let ed = level1_candidates(&taste(Tone::Editorial, StyleTemperament::Balanced));
    assert!(ed
        .iter()
        .any(|id| display_class(id).as_deref() == Some("serif")));
}
