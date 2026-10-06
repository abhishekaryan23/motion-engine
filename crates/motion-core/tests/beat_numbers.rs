//! (0.21) Beat numbers appear only when the story counts: a countdown or a
//! list in order. Every other story compiles without "01 — …" kickers,
//! folios, index numerals or "no. 02" tags, in every tone and look.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn beat(statement: &str, narration: &str, primary: &str) -> String {
    format!(
        r#"{{"purpose":"emphasize","statement":{},"narration":{},"primary":{primary}}}"#,
        serde_json::to_string(statement).unwrap(),
        serde_json::to_string(narration).unwrap()
    )
}

fn intent(beats: &[String]) -> CreativeIntent {
    let json = format!(
        r#"{{"version":"0.2","title":"numbers","format":"vertical","beats":[{}]}}"#,
        beats.join(",")
    );
    CreativeIntent::from_json(&json).expect("intent")
}

fn plain() -> CreativeIntent {
    intent(&[
        beat(
            "Leave it alone",
            "Put one thousand dollars away today and leave it alone for thirty years.",
            r#"{"kind":"object","asset":"piggy_bank"}"#,
        ),
        beat(
            "Interest on interest",
            "At seven percent a year, the interest starts earning interest of its own.",
            r#"{"kind":"number","value":"7%","meaning":"every year"}"#,
        ),
        beat(
            "Patience pays",
            "So that one thousand dollars quietly grows into about seven thousand six hundred.",
            r#"{"kind":"number","value":"$7,600","meaning":"after 30 years"}"#,
        ),
    ])
}

fn countdown() -> CreativeIntent {
    intent(&[
        beat(
            "Older than trees",
            "Here are animals that are older than trees.",
            r#"{"kind":"phrase","value":"older than trees"}"#,
        ),
        beat(
            "Sharks",
            "Number three: sharks, swimming for 450 million years.",
            r#"{"kind":"object","asset":"shark","meaning":"sharks"}"#,
        ),
        beat(
            "Jellyfish",
            "Number two, the jellyfish, drifting for 500 million years.",
            r#"{"kind":"object","asset":"jellyfish","meaning":"jellyfish"}"#,
        ),
        beat(
            "Sponges",
            "And number one: sponges, the oldest of them all.",
            r#"{"kind":"object","asset":"sponge","meaning":"sponges"}"#,
        ),
    ])
}

fn compile(intent: &CreativeIntent, style: &str, art: Option<ArtMode>) -> MotionProject {
    let style: StyleProfile = serde_json::from_str(style).expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        art,
        ..CompileOptions::default()
    };
    compile_with_options(
        intent,
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

/// (layer id, text) of every text layer, groups included.
fn texts(project: &MotionProject) -> Vec<(String, String)> {
    fn walk(layers: &[Layer], out: &mut Vec<(String, String)>) {
        for l in layers {
            match &l.kind {
                LayerKind::Text(t) => out.push((l.id.clone(), t.text.clone())),
                LayerKind::Group { children } => walk(children, out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    for s in &project.scenes {
        walk(&s.layers, &mut out);
    }
    out
}

/// Layers that exist only to number a beat.
fn numbering(project: &MotionProject) -> Vec<(String, String)> {
    texts(project)
        .into_iter()
        .filter(|(id, text)| {
            let folio = [".folio", ".index", ".numeral", ".sticker2.text"]
                .iter()
                .any(|s| id.ends_with(s));
            // A rank is "NN — label" or a bare "NN" (a label such as
            // "after 30 years" may hold digits of its own).
            let rank = text.split(" — ").next().unwrap_or_default();
            let kicker_number = id.ends_with(".kicker")
                && rank.len() == 2
                && rank.chars().all(|c| c.is_ascii_digit());
            folio || kicker_number || text.starts_with("SEC ")
        })
        .collect()
}

const STYLES: &[&str] = &[
    "{}",
    r#"{"tone":"editorial"}"#,
    r#"{"tone":"technical","density":"dense"}"#,
    r#"{"tone":"playful","density":"dense"}"#,
];

#[test]
fn a_story_that_does_not_count_shows_no_numbers() {
    let story = plain();
    for style in STYLES {
        let project = compile(&story, style, None);
        assert!(
            numbering(&project).is_empty(),
            "{style}: {:?}",
            numbering(&project)
        );
    }
    for look in [Look::Cinematic3d, Look::Dossier, Look::StudioPop] {
        let project = compile(&story, "{}", Some(ArtMode::Force(look)));
        assert!(
            numbering(&project).is_empty(),
            "{look:?}: {:?}",
            numbering(&project)
        );
    }
    // An emphasize beat with nothing to say has no kicker at all (no bare "NOTE").
    let project = compile(&story, "{}", None);
    assert!(
        !texts(&project)
            .iter()
            .any(|(id, t)| id.ends_with(".kicker") && t.eq_ignore_ascii_case("note")),
        "{:?}",
        texts(&project)
    );
}

#[test]
fn a_countdown_counts_down_on_its_items_only() {
    let story = countdown();
    for style in STYLES {
        let project = compile(&story, style, None);
        let kickers: Vec<String> = texts(&project)
            .into_iter()
            .filter(|(id, _)| id.ends_with(".kicker"))
            .map(|(_, t)| t)
            .collect();
        let numbered: Vec<&String> = kickers
            .iter()
            .filter(|t| t.len() >= 2 && t[..2].chars().all(|c| c.is_ascii_digit()))
            .collect();
        assert_eq!(numbered.len(), 3, "{style}: {kickers:?}");
        for (k, want) in numbered.iter().zip(["03", "02", "01"]) {
            assert!(k.starts_with(want), "{style}: {k} should start with {want}");
        }
    }
    let project = compile(&story, r#"{"tone":"editorial"}"#, None);
    let folios: Vec<String> = texts(&project)
        .into_iter()
        .filter(|(id, _)| id.ends_with(".folio"))
        .map(|(_, t)| t)
        .collect();
    assert_eq!(folios, ["03", "02", "01"]);
}
