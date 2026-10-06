//! (0.20) Builders declare which layer groups stand for which spoken words
//! (`B::reveal` -> `ArtRecord.reveals`), so word cues and speech QA know what
//! each element means instead of guessing from layer text.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::MotionProject;
use motion_core::speech::{RevealAnchor, RevealRole};
use motion_core::style::StyleProfile;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn compile(intent: &str, style: Option<&str>, art: ArtMode) -> MotionProject {
    let read =
        |p: &str| std::fs::read_to_string(repo().join(p)).unwrap_or_else(|e| panic!("{p}: {e}"));
    let intent = CreativeIntent::from_json(&read(intent)).expect("intent");
    let style: StyleProfile = match style {
        Some(s) => serde_json::from_str(&read(s)).expect("style"),
        None => serde_json::from_str("{}").expect("style"),
    };
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        art: Some(art),
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

fn reveals(p: &MotionProject) -> BTreeMap<String, Vec<RevealAnchor>> {
    p.project.art.as_ref().expect("art record").reveals.clone()
}

fn anchor<'a>(list: &'a [RevealAnchor], group: &str) -> Option<&'a RevealAnchor> {
    list.iter().find(|a| a.group == group)
}

#[test]
fn cinematic_beats_anchor_hero_prop_label_and_a_spoiling_title() {
    let p = compile(
        "examples/cinematic/space.intent.json",
        Some("examples/cinematic/space.style.json"),
        ArtMode::Force(Look::Cinematic3d),
    );
    let r = reveals(&p);
    // Beat 1: "A pale blue dot" — earth_globe hero, satellite prop, topic title.
    let b1 = &r["beat_1"];
    let hero = anchor(b1, "hero").expect("hero anchor");
    assert_eq!(hero.role, RevealRole::Content);
    assert!(
        hero.words.iter().any(|w| w == "earth globe"),
        "{:?}",
        hero.words
    );
    let prop = anchor(b1, "prop").expect("prop anchor");
    assert!(prop.words.iter().any(|w| w == "satellite"));
    assert_eq!(
        anchor(b1, "prop_label").map(|a| a.role),
        Some(RevealRole::Label)
    );
    // A topic title states no number or keyword: no title anchor.
    assert!(anchor(b1, "title").is_none(), "{b1:?}");
    // Beat 2: "Light takes 8 minutes" — the title says the number and the
    // keyword, so it waits for them under a holding look.
    let b2 = &r["beat_2"];
    let title = anchor(b2, "title").expect("title anchor");
    assert_eq!(title.role, RevealRole::Title);
    assert_eq!(title.words, vec!["8".to_string(), "minutes".to_string()]);
    let value = anchor(b2, "hero_word").expect("value anchor");
    assert_eq!(value.role, RevealRole::Value);
    assert_eq!(value.words, vec!["8 min".to_string()]);
}

#[test]
fn dossier_beats_anchor_figure_stamp_and_clip() {
    let p = compile(
        "examples/cinematic/space.intent.json",
        Some("examples/cinematic/space.style.json"),
        ArtMode::Force(Look::Dossier),
    );
    let r = reveals(&p);
    let b2 = &r["beat_2"];
    let figure = anchor(b2, "figure").expect("figure anchor");
    assert_eq!(figure.role, RevealRole::Value);
    assert_eq!(figure.words, vec!["8 min".to_string()]);
    let stamp = anchor(b2, "stamp").expect("stamp anchor");
    assert_eq!(stamp.role, RevealRole::Stamp);
    assert_eq!(stamp.words, vec!["minutes".to_string()]);
    let clip = anchor(b2, "clip").expect("clip title anchor");
    assert_eq!(clip.role, RevealRole::Title);
    assert!(clip.words.contains(&"8".to_string()));
    // Every anchor names a group that exists in the scene.
    let scene = p.scenes.iter().find(|s| s.id == "beat_2").expect("scene");
    fn ids(layers: &[motion_core::scene::Layer], out: &mut Vec<String>) {
        for l in layers {
            out.push(l.id.clone());
            if let motion_core::scene::LayerKind::Group { children } = &l.kind {
                ids(children, out);
            }
        }
    }
    let mut all = Vec::new();
    ids(&scene.layers, &mut all);
    for a in b2 {
        let prefix = format!("b2.{}", a.group);
        assert!(
            all.iter()
                .any(|id| *id == prefix || id.starts_with(&format!("{prefix}."))),
            "anchor group {} names no layer in beat_2",
            a.group
        );
    }
}

#[test]
fn layer_beats_anchor_the_pinned_subject_and_the_headline() {
    let p = compile(
        "examples/public/layers.intent.json",
        None,
        ArtMode::Force(Look::Cinematic3d),
    );
    let r = reveals(&p);
    let with_pinned: Vec<&Vec<RevealAnchor>> = r
        .values()
        .filter(|list| anchor(list, "pinned").is_some())
        .collect();
    assert!(!with_pinned.is_empty(), "{r:?}");
    for list in with_pinned {
        assert_eq!(
            anchor(list, "pinned").map(|a| a.role),
            Some(RevealRole::Content)
        );
        // The caption exists only when the secondary carries a meaning.
        if let Some(label) = anchor(list, "pinned_label") {
            assert_eq!(label.role, RevealRole::Label);
            assert_eq!(label.words, anchor(list, "pinned").expect("pinned").words);
        }
    }
}

#[test]
fn no_anchors_without_art_direction() {
    let read = std::fs::read_to_string(repo().join("examples/editorial_demo.intent.json"))
        .expect("intent");
    let intent = CreativeIntent::from_json(&read).expect("intent");
    let style: StyleProfile = serde_json::from_str("{}").expect("style");
    let library = AssetLibrary::new(repo().join("assets"));
    let measure: &dyn TextMeasure = &ApproxMeasure;
    let p = compile_with_options(
        &intent,
        &style,
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &CompileOptions::default(),
    )
    .expect("compile");
    assert!(p.project.art.is_none());
    assert!(!serde_json::to_string(&p).unwrap().contains("\"reveals\""));
}
