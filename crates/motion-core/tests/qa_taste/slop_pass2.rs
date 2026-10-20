//! (0.24 M9) Anti-slop pass 2: no leaked purpose pills, grammatical titles,
//! the accent budget, no placeholder text bars, a keyline for pictures that
//! vanish into their ground, the design read with its dials, and the note on
//! unsourced numbers.

use std::path::PathBuf;

use motion_core::assets::{AssetManifest, NormBox};
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::LayoutCheck;
use motion_core::scene::{LayerKind, MotionProject};
use motion_core::slop_qa::{critique_with, SlopCheck};
use motion_core::style::StyleProfile;
use motion_core::subject_qa::{guard_low_contrast, ImageFacts, ImageIndex};
use motion_core::{layout_report_with, slop_report, SlopRules};
use serde_json::{json, Value};

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(
        r#"{{"tone":"{tone}","polarity":"dark","temperature":"warm","temperament":"energetic",
        "density":"balanced","seed":7}}"#
    ))
    .expect("style")
}

fn compile(intent: &str, tone: &str) -> MotionProject {
    let intent = CreativeIntent::from_json(intent).expect("intent");
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        ..CompileOptions::default()
    };
    let measure: &dyn TextMeasure = &ApproxMeasure;
    compile_with_options(
        &intent,
        &style(tone),
        None,
        &library,
        measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compiles")
}

/// Nine beats, mostly impact, one reveal near the end.
fn nine_beats() -> String {
    let rows: Vec<String> = (0..9)
        .map(|i| {
            let (purpose, energy) = match i {
                7 => ("reveal", "impact"),
                1 | 4 => ("compare", "building"),
                2 | 8 => ("explain", "calm"),
                _ => ("emphasize", "impact"),
            };
            format!(
                r#"{{"purpose":"{purpose}","statement":"Beat number {i}",
                "primary":{{"kind":"number","value":"${}K","meaning":"net worth"}},
                "energy":"{energy}","keyword":"worth"}}"#,
                100 + 25 * i
            )
        })
        .collect();
    format!(
        r#"{{"version":"0.2","title":"nine","format":"vertical","beats":[{}]}}"#,
        rows.join(",")
    )
}

fn all_layers<'a>(
    layers: &'a [motion_core::scene::Layer],
    out: &mut Vec<&'a motion_core::scene::Layer>,
) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            all_layers(children, out);
        }
    }
}

fn layers_of(p: &MotionProject) -> Vec<&motion_core::scene::Layer> {
    let mut out = Vec::new();
    for s in &p.scenes {
        all_layers(&s.layers, &mut out);
    }
    out
}

fn text_of(l: &motion_core::scene::Layer) -> Option<&str> {
    match &l.kind {
        LayerKind::Text(t) => Some(t.text.as_str()),
        _ => None,
    }
}

// --- 1. the purpose pill never renders ---------------------------------------

#[test]
fn no_look_prints_the_beats_purpose_word() {
    for tone in [
        "playful",
        "hype",
        "documentary",
        "editorial",
        "technical",
        "street",
    ] {
        let p = compile(&nine_beats(), tone);
        for l in layers_of(&p) {
            assert!(
                !l.id.contains(".sticker"),
                "{tone}: leaked purpose sticker {}",
                l.id
            );
            if let Some(t) = text_of(l) {
                let t = t.trim().to_lowercase();
                assert!(
                    !["note", "compare", "contrast", "result", "why"].contains(&t.as_str()),
                    "{tone}: {} prints the purpose word {t:?}",
                    l.id
                );
            }
        }
    }
}

// --- 3. titles keep their small words ----------------------------------------

#[test]
fn a_short_statement_is_shown_as_written() {
    for statement in ["Around the world", "In the red", "Out of time"] {
        let intent = format!(
            r#"{{"version":"0.2","title":"t","format":"vertical","beats":[
            {{"purpose":"explain","statement":"{statement}",
              "primary":{{"kind":"phrase","value":"Other countries","meaning":"x"}},
              "energy":"building","keyword":"countries"}}]}}"#
        );
        let p = compile(&intent, "hype");
        let shown: Vec<String> = layers_of(&p)
            .into_iter()
            .filter(|l| l.id.contains(".cut.") && l.id.ends_with(".word"))
            .filter_map(|l| match &l.kind {
                // (0.24 Wave 2) A multi-line word is a group of its lines.
                LayerKind::Group { children } => Some(
                    children
                        .iter()
                        .filter_map(text_of)
                        .collect::<Vec<_>>()
                        .join(" "),
                ),
                _ => text_of(l).map(str::to_string),
            })
            .map(|t| t.replace('\n', " ").to_lowercase())
            .collect();
        assert!(
            shown.iter().any(|t| t == &statement.to_lowercase()),
            "{statement:?} -> {shown:?}"
        );
    }
}

// --- 4. the accent budget -----------------------------------------------------

#[test]
fn the_hype_look_spends_one_field_and_few_blocks() {
    let p = compile(&nine_beats(), "hype");
    let ids: Vec<&str> = layers_of(&p).iter().map(|l| l.id.as_str()).collect();
    let fields = ids.iter().filter(|i| i.ends_with(".field")).count();
    let wipes = ids.iter().filter(|i| i.ends_with(".wipe_a")).count();
    let bands = ids.iter().filter(|i| i.ends_with(".band")).count();
    assert!(fields + wipes <= 1, "fields {fields} wipes {wipes}");
    assert!(bands <= 3, "bands {bands}");
    let r = slop_report(&p, &SlopRules::bundled());
    assert!(
        !r.findings
            .iter()
            .any(|f| f.check == SlopCheck::AccentOveruse),
        "{:?}",
        r.findings
    );
    // The payoff (the reveal, beat 8) holds the field.
    assert!(
        ids.iter()
            .any(|i| i.starts_with("b8.") && i.ends_with(".field")),
        "the field goes to the payoff beat"
    );
}

#[test]
fn the_documentary_look_has_no_text_bars_and_few_stamps() {
    let p = compile(&nine_beats(), "documentary");
    let layers = layers_of(&p);
    assert!(
        !layers.iter().any(|l| l.id.contains(".clip.bar.")),
        "placeholder text bars"
    );
    let stamps = layers
        .iter()
        .filter(|l| l.id.ends_with(".stamp") && matches!(l.kind, LayerKind::Group { .. }))
        .count();
    assert!(stamps <= 3, "{stamps} stamps in 9 beats");
    let blocks = layers
        .iter()
        .filter(|l| l.id.ends_with(".clip.highlight"))
        .count();
    assert!(blocks <= 3, "{blocks} highlighters in 9 beats");
    let r = slop_report(&p, &SlopRules::bundled());
    for check in [SlopCheck::AccentOveruse, SlopCheck::SkeletonLines] {
        assert!(
            !r.findings.iter().any(|f| f.check == check),
            "{check:?}: {:?}",
            r.findings
        );
    }
}

#[test]
fn the_budget_is_deterministic() {
    let a = compile(&nine_beats(), "hype").to_json_pretty();
    let b = compile(&nine_beats(), "hype").to_json_pretty();
    assert_eq!(a, b);
}

// --- 2. no corner label -------------------------------------------------------

#[test]
fn a_figure_labelled_with_what_the_picture_shows_prints_no_corner_label() {
    let intent = r#"{"version":"0.2","title":"t","format":"vertical","beats":[
      {"purpose":"emphasize","statement":"Taxed",
       "primary":{"kind":"number","value":"£200K","meaning":"United Kingdom"},
       "energy":"impact","keyword":"tax"},
      {"purpose":"emphasize","statement":"Top 1%",
       "primary":{"kind":"number","value":"$731,000","meaning":"a year"},
       "energy":"impact","keyword":"top"}]}"#;
    for tone in ["hype", "street"] {
        let p = compile(intent, tone);
        for l in layers_of(&p) {
            if let Some(t) = text_of(l) {
                assert!(
                    t.trim() != t.trim().to_uppercase() || !t.to_lowercase().contains("kingdom"),
                    "{tone}: corner label {t:?} in {}",
                    l.id
                );
                assert!(
                    !(l.id.ends_with(".marker") && l.y < 0.15 * 1920.0),
                    "{tone}: {}",
                    l.id
                );
            }
        }
    }
}

// --- 7. a picture that vanishes into its ground gets a keyline ---------------

fn base(id: &str, x: f32, y: f32, w: f32, h: f32, z: i32) -> Value {
    json!({
        "id": id, "x": x, "y": y, "width": w, "height": h,
        "scale_x": 1.0, "scale_y": 1.0, "rotation_degrees": 0.0,
        "anchor_x": 0.0, "anchor_y": 0.0, "opacity": 1.0,
        "z_index": z, "visible": true,
    })
}

fn flag_project(background: &str) -> MotionProject {
    let mut image = base("b1.cut.2.pic.flag", 100.0, 600.0, 800.0, 600.0, 22);
    for (k, v) in json!({"type": "image", "asset": "asset.flag", "fit": "contain"})
        .as_object()
        .expect("object")
    {
        image[k] = v.clone();
    }
    let doc = json!({
        "version": "0.2",
        "project": {"name": "flag", "duration_seconds": 3.0},
        "canvas": {"width": 1080, "height": 1920, "fps": 30, "background": background},
        "theme": {"fonts": {
            "display": "font.archivo_black", "display_condensed": "font.anton",
            "serif_emotional": "font.dm_serif_italic", "body": "font.fira_sans",
            "mono": "font.space_mono", "number": "font.anton"}, "palette": {}},
        "assets": [{"id": "asset.flag", "type": "image", "path": "flag.png"}],
        "scenes": [{
            "id": "beat_1", "start_seconds": 0.0, "duration_seconds": 3.0,
            "layers": [image], "motions": [],
            "lifecycle": {"enter": 0.2, "settle": 0.5, "read": 1.0, "evolve": 1.5,
                          "anticipate": 2.5, "bridge": 3.0},
        }],
    });
    serde_json::from_value(doc).expect("project")
}

fn flag_index(mean: [u8; 3], alpha: bool) -> ImageIndex {
    let mut index = ImageIndex::new();
    index.insert(
        "flag.png",
        ImageFacts {
            width: 800,
            height: 600,
            alpha,
            subject: NormBox {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
            },
            head: None,
            mean_color: Some(mean),
        },
    );
    index
}

fn low_contrast(p: &MotionProject, index: &ImageIndex) -> bool {
    let frame = LayoutFrame::new(1080, 1920).expect("frame");
    layout_report_with(p, &frame, index)
        .findings
        .iter()
        .any(|f| f.check == LayoutCheck::AssetLowContrast)
}

#[test]
fn a_flag_as_dark_as_its_ground_gets_a_light_keyline() {
    // The Australian flag (mean #343875) on navy.
    let index = flag_index([0x34, 0x38, 0x75], false);
    let mut p = flag_project("#0B1A2C");
    assert!(low_contrast(&p, &index), "control: it fails before");
    assert_eq!(guard_low_contrast(&mut p, &index), 1);
    assert!(!low_contrast(&p, &index), "the keyline fixes it");
    let layers = layers_of(&p);
    let LayerKind::Image { treatment, .. } = &layers[0].kind else {
        panic!("image")
    };
    let edge = treatment.as_ref().and_then(|t| t.edge).expect("keyline");
    assert!(edge.color.r > 0xC0, "a light keyline on a dark flag");
    assert!(edge.width >= 2.0);
    // Run again: nothing left to do.
    assert_eq!(guard_low_contrast(&mut p, &index), 0);
}

#[test]
fn a_cutout_gets_a_sticker_and_a_readable_picture_is_left_alone() {
    let mut p = flag_project("#0B1A2C");
    let cutout = flag_index([0x34, 0x38, 0x75], true);
    assert_eq!(guard_low_contrast(&mut p, &cutout), 1);
    let LayerKind::Image { treatment, .. } = &layers_of(&p)[0].kind else {
        panic!("image")
    };
    assert!(treatment.as_ref().is_some_and(|t| t.sticker.is_some()));

    let mut p = flag_project("#0B1A2C");
    let bright = flag_index([0xF0, 0xE8, 0xD0], false);
    assert_eq!(guard_low_contrast(&mut p, &bright), 0);
    let LayerKind::Image { treatment, .. } = &layers_of(&p)[0].kind else {
        panic!("image")
    };
    assert!(treatment.is_none());
}

// --- 9 and 10. the design read, dials and the unsourced note -----------------

#[test]
fn critique_reads_the_design_in_one_line() {
    let p = compile(&nine_beats(), "hype");
    let c = critique_with(&p, &SlopRules::bundled(), Some(&style("hype")));
    assert!(
        c.reading
            .starts_with("Reading this as: hype (hype slam) for vertical 1080x1920, dial ENERGY "),
        "{}",
        c.reading
    );
    assert!(c.reading.contains(" / RHYTHM ") && c.reading.contains(" / MOTION "));
    assert!(c.to_text().contains(&c.reading));
    let d = c.dials.expect("dials");
    assert!(
        (1..=10).contains(&d.energy)
            && (1..=10).contains(&d.rhythm)
            && (1..=10).contains(&d.motion)
    );
    assert!(c.measured.is_some());
}

#[test]
fn a_style_that_declares_calm_against_a_busy_scene_is_flagged() {
    let p = compile(&nine_beats(), "hype");
    let measured = critique_with(&p, &SlopRules::bundled(), None)
        .measured
        .expect("measured");
    // Declare the opposite end of a dial the scene sits at.
    let calm: StyleProfile = serde_json::from_str(if measured.energy >= 6 {
        r#"{"temperament":"restrained","motion_language":"minimal","density":"sparse"}"#
    } else {
        r#"{"temperament":"energetic","motion_language":"kinetic","density":"dense"}"#
    })
    .expect("style");
    let c = critique_with(&p, &SlopRules::bundled(), Some(&calm));
    let hits: Vec<_> = c
        .findings
        .iter()
        .filter(|f| f.check == "dial_mismatch")
        .collect();
    assert!(!hits.is_empty(), "{}", c.to_text());
    // No style, no declaration, no mismatch.
    let none = critique_with(&p, &SlopRules::bundled(), None);
    assert!(!none.findings.iter().any(|f| f.check == "dial_mismatch"));
}

#[test]
fn figures_without_a_source_earn_a_note_not_a_failure() {
    let p = compile(&nine_beats(), "editorial");
    let c = critique_with(&p, &SlopRules::bundled(), None);
    assert!(
        c.notes
            .iter()
            .any(|n| n.starts_with("note_unsourced_numbers")),
        "{:?}",
        c.notes
    );
    assert!(!c.findings.iter().any(|f| f.check.contains("unsourced")));
    // A beat that carries a source silences it.
    let intent = r#"{"version":"0.2","title":"t","format":"vertical","beats":[
      {"purpose":"emphasize","statement":"Median pay",
       "primary":{"kind":"number","value":"$74K","meaning":"SmartAsset, 2024"},
       "energy":"building","keyword":"pay"}]}"#;
    let p = compile(intent, "editorial");
    let c = critique_with(&p, &SlopRules::bundled(), None);
    let mut text = String::new();
    for l in layers_of(&p) {
        text.push_str(text_of(l).unwrap_or(""));
        text.push('\n');
    }
    if text.contains("SmartAsset") {
        assert!(c.notes.is_empty(), "{:?}", c.notes);
    }
}
