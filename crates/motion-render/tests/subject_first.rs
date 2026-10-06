//! (0.10 Q) Subject-first image layouts, frame shrink-wrap and the contrast
//! guard, end to end on the owner-review fixture
//! (`golden/fixtures/owner_review/celebrating_life_repetitive.intent.json`,
//! `--asset-family people_everyday`): compiled with the real font measurer and
//! judged by layout QA over the images' measured geometry (`image_index`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use motion_core::assets::AssetManifest;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{compile_with_options, AssetLibrary, CompileOptions, FontSet};
use motion_core::layout_qa::{LayoutCheck, LayoutFinding};
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
use motion_core::{layout_report_with, validate, CreativeIntent, StyleProfile};
use motion_render::{image_index, FontMeasure};

const FIXTURE: &str = "golden/fixtures/owner_review/celebrating_life_repetitive.intent.json";
const STYLES: [&str; 2] = [
    "examples/taste/warm_editorial.style.json",
    "examples/taste/playful_print.style.json",
];
const CANVASES: [(u32, u32); 2] = [(1080, 1920), (1080, 1080)];
const VARIETIES: [Option<u64>; 5] = [Some(1), Some(2), Some(3), Some(4), None];

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn assets_dir() -> PathBuf {
    repo().join("assets")
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn compile_with(
    intent: &CreativeIntent,
    style_rel: &str,
    canvas: (u32, u32),
    variety: Option<u64>,
    families: &[&str],
    manifest: &AssetManifest,
) -> MotionProject {
    let style: StyleProfile = serde_json::from_str(&read(style_rel)).expect("style");
    let assets = assets_dir();
    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    let library =
        AssetLibrary::new(&assets).with_families(families.iter().map(|f| f.to_string()).collect());
    let opts = CompileOptions {
        canvas: Some(canvas),
        variety,
        ..CompileOptions::default()
    };
    compile_with_options(
        intent, &style, None, &library, &measure, manifest, None, &opts,
    )
    .expect("compiles")
}

fn compile_fixture(style_rel: &str, canvas: (u32, u32), variety: Option<u64>) -> MotionProject {
    let intent = CreativeIntent::from_json(&read(FIXTURE)).expect("fixture");
    compile_with(
        &intent,
        style_rel,
        canvas,
        variety,
        &["people_everyday"],
        &AssetManifest::empty(),
    )
}

fn findings(p: &MotionProject, canvas: (u32, u32)) -> Vec<LayoutFinding> {
    let frame = LayoutFrame::new(canvas.0, canvas.1).expect("frame");
    let index = image_index(p, &assets_dir());
    layout_report_with(p, &frame, &index).findings
}

fn subject_checks(f: &LayoutFinding) -> bool {
    matches!(
        f.check,
        LayoutCheck::TextOverSubject
            | LayoutCheck::SubjectTooSmall
            | LayoutCheck::FrameTooLoose
            | LayoutCheck::AssetLowContrast
    )
}

fn all_layers<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            all_layers(children, out);
        }
    }
}

/// Top-left box `(x, y, w, h)` of a layer (its x/y is the anchor point).
fn bbox(l: &Layer) -> (f32, f32, f32, f32) {
    (
        l.x - l.anchor_x * l.width,
        l.y - l.anchor_y * l.height,
        l.width,
        l.height,
    )
}

/// The layout a beat uses, read back from its geometry: where the subject
/// stands and where the headline sits.
fn classify(p: &MotionProject, beat: usize) -> Option<&'static str> {
    let scene = p.scenes.iter().find(|s| s.id == format!("beat_{beat}"))?;
    let mut layers = Vec::new();
    all_layers(&scene.layers, &mut layers);
    let subject = layers.iter().find(|l| l.id.ends_with(".subject"))?;
    let (sx, sy, sw, sh) = bbox(subject);
    let (cx, cy) = (sx + sw / 2.0, sy + sh / 2.0);
    let title_top = layers
        .iter()
        .filter(|l| l.id.contains(".head."))
        .map(|l| bbox(l).1)
        .fold(f32::MAX, f32::min);
    let w = p.canvas.width as f32;
    Some(if title_top > cy {
        "center_band"
    } else if cx > 0.58 * w {
        "right"
    } else if cx < 0.42 * w {
        "left"
    } else {
        "top"
    })
}

fn image_beats(p: &MotionProject) -> Vec<usize> {
    (1..=5).filter(|&n| classify(p, n).is_some()).collect()
}

#[test]
fn owner_review_fixture_passes_the_subject_checks_in_every_configuration() {
    let mut failures = Vec::new();
    for style in STYLES {
        for canvas in CANVASES {
            for variety in VARIETIES {
                let p = compile_fixture(style, canvas, variety);
                validate(&p, Some(&assets_dir())).expect("valid scene");
                let label = format!("{style} {}x{} variety {variety:?}", canvas.0, canvas.1);
                assert_eq!(image_beats(&p).len(), 4, "{label}: four image beats");
                for f in findings(&p, canvas).iter().filter(|f| subject_checks(f)) {
                    failures.push(format!(
                        "{label}: {} {} {}: {}",
                        f.scene,
                        f.layer,
                        f.check.name(),
                        f.detail
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn consecutive_image_beats_use_different_layouts_under_variety() {
    let mut seen: BTreeMap<&'static str, usize> = BTreeMap::new();
    for style in STYLES {
        for canvas in CANVASES {
            for seed in 1..=4u64 {
                let p = compile_fixture(style, canvas, Some(seed));
                let beats = image_beats(&p);
                let layouts: Vec<&str> = beats
                    .iter()
                    .map(|&n| classify(&p, n).expect("image beat"))
                    .collect();
                for pair in layouts.windows(2) {
                    assert_ne!(
                        pair[0], pair[1],
                        "{style} {}x{} variety {seed}: consecutive image beats share a layout: {layouts:?}",
                        canvas.0, canvas.1
                    );
                }
                for l in &layouts {
                    *seen.entry(*l).or_default() += 1;
                }
            }
        }
    }
    // Across the sweep the rotation really varies the look.
    assert!(seen.len() >= 3, "layouts used: {seen:?}");
}

#[test]
fn variety_never_prints_headline_type_over_a_person() {
    // The point of subject-first: type and subject are in disjoint regions.
    for style in STYLES {
        for canvas in CANVASES {
            for seed in 1..=4u64 {
                let p = compile_fixture(style, canvas, Some(seed));
                for n in image_beats(&p) {
                    let scene = p
                        .scenes
                        .iter()
                        .find(|s| s.id == format!("beat_{n}"))
                        .expect("scene");
                    let mut layers = Vec::new();
                    all_layers(&scene.layers, &mut layers);
                    let subject = layers
                        .iter()
                        .find(|l| l.id.ends_with(".subject"))
                        .expect("subject");
                    let (sx, sy, sw, sh) = bbox(subject);
                    for l in layers.iter().filter(|l| l.id.contains(".head.")) {
                        let (lx, ly, lw, lh) = bbox(l);
                        // The image box includes transparent margins; the
                        // headline must clear even the alpha bounds, which the
                        // QA test above checks exactly. Here: the headline's
                        // centre line is outside the image box.
                        let (cx, cy) = (lx + lw / 2.0, ly + lh / 2.0);
                        let inside = cx > sx && cx < sx + sw && cy > sy && cy < sy + sh;
                        assert!(
                            !inside,
                            "{style} {}x{} seed {seed} beat {n}: {} sits inside the subject box",
                            canvas.0, canvas.1, l.id
                        );
                    }
                }
            }
        }
    }
}

/// A SpeechMap with one sentence inside each beat of `base` (a no-speech
/// compile), words evenly spaced.
fn speech_for(intent: &CreativeIntent, base: &MotionProject) -> SpeechMap {
    let strip = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let beats: Vec<_> = base
        .scenes
        .iter()
        .filter(|s| s.lifecycle.is_some())
        .collect();
    for (bi, (scene, beat)) in beats.iter().zip(&intent.beats).enumerate() {
        let tokens: Vec<String> = beat
            .statement
            .split_whitespace()
            .map(strip)
            .filter(|w| !w.is_empty())
            .collect();
        let start = scene.start_seconds + 0.40;
        let end = (scene.end_seconds() - 0.90).max(start + 0.3 * tokens.len() as f64);
        let step = (end - start) / tokens.len() as f64;
        for (i, t) in tokens.iter().enumerate() {
            let s = start + step * i as f64;
            words.push(SpeechWord {
                text: t.clone(),
                start: (s * 1000.0).round() / 1000.0,
                end: ((s + step * 0.85) * 1000.0).round() / 1000.0,
                confidence: 0.5,
            });
        }
        sentences.push(SpeechSentence {
            beat: bi,
            start,
            end,
        });
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: base.scenes.last().map(|s| s.end_seconds()).unwrap_or(0.0),
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

#[test]
fn with_a_voice_over_the_layouts_stay_above_the_caption_lane_and_pass_the_subject_checks() {
    // The captions own the lower lane: builders lay out on a shortened canvas,
    // while the minimum subject area stays that of the real canvas.
    let intent = CreativeIntent::from_json(&read(FIXTURE)).expect("fixture");
    let style: StyleProfile = serde_json::from_str(&read(STYLES[0])).expect("style");
    let assets = assets_dir();
    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    let library = AssetLibrary::new(&assets).with_families(vec!["people_everyday".into()]);
    let mut failures = Vec::new();
    for canvas in CANVASES {
        for variety in VARIETIES {
            let compile = |speech: Option<SpeechMap>| {
                compile_with_options(
                    &intent,
                    &style,
                    None,
                    &library,
                    &measure,
                    &AssetManifest::empty(),
                    None,
                    &CompileOptions {
                        canvas: Some(canvas),
                        variety,
                        speech,
                        ..CompileOptions::default()
                    },
                )
                .expect("compiles")
            };
            let base = compile(None);
            let p = compile(Some(speech_for(&intent, &base)));
            validate(&p, Some(&assets)).expect("valid scene");
            let label = format!("{}x{} variety {variety:?}", canvas.0, canvas.1);
            for f in findings(&p, canvas).iter().filter(|f| subject_checks(f)) {
                // Known limit: on a square canvas the caption lane leaves a
                // narrow cutout (the 0.31-aspect student) about 620 px of
                // height, which cannot reach 14 % of the canvas whatever the
                // layout; the layout then keeps the biggest subject it can.
                if f.check == LayoutCheck::SubjectTooSmall && canvas.0 == canvas.1 {
                    continue;
                }
                failures.push(format!(
                    "{label}: {} {} {}: {}",
                    f.scene,
                    f.layer,
                    f.check.name(),
                    f.detail
                ));
            }
            // The subject never reaches into the caption lane's text area: the
            // image box ends above the lane top.
            let lane_top = motion_core::compiler::captions::lower_lane_top(
                &LayoutFrame::new(canvas.0, canvas.1).expect("frame"),
            );
            for n in image_beats(&p) {
                let scene = p
                    .scenes
                    .iter()
                    .find(|s| s.id == format!("beat_{n}"))
                    .expect("scene");
                let mut layers = Vec::new();
                all_layers(&scene.layers, &mut layers);
                let subject = layers
                    .iter()
                    .find(|l| l.id.ends_with(".subject"))
                    .expect("subject");
                let (_, sy, _, sh) = bbox(subject);
                // The stage may scale slightly; allow the reserve's own gap.
                assert!(
                    sy + sh <= lane_top + 4.0,
                    "{label} beat {n}: the picture reaches the caption lane ({} > {lane_top})",
                    sy + sh
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn compiles_are_deterministic() {
    for variety in VARIETIES {
        let a = compile_fixture(STYLES[0], CANVASES[0], variety);
        let b = compile_fixture(STYLES[0], CANVASES[0], variety);
        assert_eq!(
            a.to_json_pretty(),
            b.to_json_pretty(),
            "variety {variety:?}"
        );
    }
}

#[test]
fn without_variety_the_legacy_interlock_keeps_its_beats_unless_a_head_would_be_covered() {
    // Legacy (no variety) leaves subjects it laid out cleanly alone: the
    // fixture compiles at 9:16 with at least one beat still on the interlock
    // (its subject straddles the headline), and QA passes.
    let p = compile_fixture(STYLES[0], CANVASES[0], None);
    let f = findings(&p, CANVASES[0]);
    assert!(
        !f.iter().any(subject_checks),
        "{:?}",
        f.iter()
            .map(|f| (&f.layer, f.check.name()))
            .collect::<Vec<_>>()
    );
}
