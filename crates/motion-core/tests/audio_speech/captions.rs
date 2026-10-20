//! (0.10) Word-synced kinetic captions: a hand-built SpeechMap whose
//! sentences sit inside the compiled beats drives the caption scene. Checks:
//! scene present, every word appears within 1 frame of its start, <= 2 lines
//! and <= 32 chars per line, layout QA + caption report PASS for the three
//! tastes on 9:16, 1:1, 16:9 and 4:5, the treatment follows the taste's
//! emotion, determinism. (Treatment-by-treatment structure is unit-tested in
//! `compiler/captions.rs`.)

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::caption_qa::caption_report;
use motion_core::compiler::captions::{
    caption_treatment, word_layer_id, CaptionTreatment, CAPTION_SCENE_ID, MAX_CHARS_PER_LINE,
    MAX_LINES,
};
use motion_core::compiler::typography::resolve_emotion;
use motion_core::compiler::{
    compile_with_options, resolve_taste, ApproxMeasure, AssetLibrary, CompileOptions,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{LayerKind, MotionProject};
use motion_core::speech::{SpeechMap, SpeechSentence, SpeechWord};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use motion_core::validate::validate;
use motion_core::{layout_report, LayoutVerdict};

const STORIES: &[&str] = &[
    "examples/voice_10/city_water.intent.json",
    "examples/editorial_demo.intent.json",
    "golden/editorial_collage.intent.json",
];

const STYLES: &[&str] = &[
    "examples/taste/warm_editorial.style.json",
    "examples/taste/dark_technical.style.json",
    "examples/taste/playful_print.style.json",
];

/// 9:16, 1:1, 16:9, 4:5.
const CANVASES: &[(u32, u32)] = &[(1080, 1920), (1080, 1080), (1920, 1080), (1080, 1350)];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn compile(
    intent: &CreativeIntent,
    style: &StyleProfile,
    canvas: (u32, u32),
    speech: Option<SpeechMap>,
) -> MotionProject {
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        canvas: Some(canvas),
        speech,
        ..CompileOptions::default()
    };
    compile_with_options(
        intent,
        style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

fn strip(word: &str) -> String {
    word.trim_matches(|c: char| !(c.is_alphanumeric() || matches!(c, '%' | '₹' | '$' | '€' | '£')))
        .to_string()
}

/// A SpeechMap with one sentence inside each beat of `base` (a no-speech
/// compile), words evenly spaced.
fn speech_for(intent: &CreativeIntent, base: &MotionProject) -> SpeechMap {
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    let beats: Vec<_> = base
        .scenes
        .iter()
        .filter(|s| s.lifecycle.is_some())
        .collect();
    assert_eq!(beats.len(), intent.beats.len());
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

fn find<'a, 'b>(layers: &'b [ResolvedLayer<'a>], id: &str) -> Option<&'b ResolvedLayer<'a>> {
    for l in layers {
        if l.id == id {
            return Some(l);
        }
        if let Some(f) = find(&l.children, id) {
            return Some(f);
        }
    }
    None
}

/// Effective opacity of layer `id` at `frame` (0 when not present).
fn opacity_at(project: &MotionProject, frame: u32, id: &str) -> f32 {
    fn walk(layers: &[ResolvedLayer], id: &str, parent: f32) -> Option<f32> {
        for l in layers {
            let o = parent * l.opacity;
            if l.id == id {
                return Some(o);
            }
            if let Some(f) = walk(&l.children, id, o) {
                return Some(f);
            }
        }
        None
    }
    let resolved = evaluate_frame(project, frame).expect("frame");
    let _ = find(&resolved.layers, id);
    walk(&resolved.layers, id, 1.0).unwrap_or(0.0)
}

struct Case {
    label: String,
    base: MotionProject,
    project: MotionProject,
    speech: SpeechMap,
    treatment: CaptionTreatment,
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    for story in STORIES {
        let intent = CreativeIntent::from_json(&read(story)).expect("intent");
        for style_rel in STYLES {
            let style: StyleProfile = serde_json::from_str(&read(style_rel)).expect("style");
            for &canvas in CANVASES {
                let label = format!("{story} · {style_rel} · {}x{}", canvas.0, canvas.1);
                let base = compile(&intent, &style, canvas, None);
                let speech = speech_for(&intent, &base);
                let project = compile(&intent, &style, canvas, Some(speech.clone()));
                let taste = resolve_taste(&intent, &style, None);
                out.push(Case {
                    label,
                    base,
                    project,
                    speech,
                    treatment: caption_treatment(resolve_emotion(&taste)),
                });
            }
        }
    }
    out
}

#[test]
fn caption_scene_is_present_valid_and_synced() {
    for c in cases() {
        let scene = c
            .project
            .scenes
            .iter()
            .find(|s| s.id == CAPTION_SCENE_ID)
            .unwrap_or_else(|| panic!("{}: no caption scene", c.label));
        assert!(!scene.layers.is_empty(), "{}: empty caption scene", c.label);
        assert!(scene.lifecycle.is_none());
        if let Err(e) = validate(&c.project, None) {
            panic!("{}: invalid: {e}", c.label);
        }
        let fps = c.project.canvas.fps as f64;
        let report = caption_report(&c.project, &c.speech);
        assert!(report.passed(), "{}: {}", c.label, report.to_text());
        assert_eq!(report.words.len(), c.speech.words.len());
        assert!(
            report.max_delta <= 1.0 / fps + 1e-6,
            "{}: max word timing error {}",
            c.label,
            report.max_delta
        );
        assert!(report.max_lines <= MAX_LINES, "{}", c.label);
        assert!(report.max_line_chars <= MAX_CHARS_PER_LINE, "{}", c.label);
        // No motion outlives the caption scene.
        for m in &scene.motions {
            assert!(
                m.start + m.duration <= scene.duration_seconds + 1e-6,
                "{}: motion on {} ends after the scene",
                c.label,
                m.target
            );
        }
        // Treatment shows in the structure (the city_water story has "30%").
        let has_bg = scene.layers.iter().any(|p| match &p.kind {
            LayerKind::Group { children } => children
                .iter()
                .any(|l| l.id.ends_with(".bg") && l.id.starts_with("cap.w")),
            _ => false,
        });
        match c.treatment {
            CaptionTreatment::LabelPill | CaptionTreatment::Highlight => {
                if c.label.contains("city_water") {
                    assert!(has_bg, "{}: expected emphasis fields", c.label)
                }
            }
            CaptionTreatment::SerifItalic
            | CaptionTreatment::Handwritten
            | CaptionTreatment::SingleWord => {
                assert!(!has_bg, "{}: unexpected emphasis fields", c.label)
            }
        }
    }
}

#[test]
fn words_appear_on_their_start_frame() {
    // Pixel-level (timeline) check on one canvas per taste: just before a word
    // starts it is invisible; 0.2 s after (before its page leaves) it is fully shown.
    let intent = CreativeIntent::from_json(&read(STORIES[0])).expect("intent");
    for style_rel in STYLES {
        let style: StyleProfile = serde_json::from_str(&read(style_rel)).expect("style");
        let base = compile(&intent, &style, (1080, 1920), None);
        let speech = speech_for(&intent, &base);
        let project = compile(&intent, &style, (1080, 1920), Some(speech.clone()));
        let fps = project.canvas.fps as f64;
        for (i, w) in speech.words.iter().enumerate().step_by(5) {
            let id = word_layer_id(i);
            let before = ((w.start - 1.5 / fps) * fps).floor() as u32;
            assert!(
                opacity_at(&project, before, &id) < 0.05,
                "{style_rel}: word {i} '{}' visible before its start",
                w.text
            );
            let after = ((w.start + 0.13) * fps).ceil() as u32;
            let o = opacity_at(&project, after, &id);
            // The page may be leaving (a following page starts within 0.2 s).
            let next_start = speech.words.get(i + 1).map(|n| n.start).unwrap_or(f64::MAX);
            if next_start > w.start + 0.4 {
                assert!(o > 0.95, "{style_rel}: word {i} '{}' opacity {o}", w.text);
            }
        }
    }
}

#[test]
fn layout_and_caption_qa_pass_for_three_tastes_on_four_canvases() {
    for c in cases() {
        let frame = motion_core::compiler::layout_frame::LayoutFrame::new(
            c.project.canvas.width,
            c.project.canvas.height,
        )
        .expect("frame");
        let report = layout_report(&c.project, &frame);
        // Captions never add a finding. (A few voice_10 beats fail layout QA on
        // some canvases without speech: that is a beat-builder matter. With
        // speech the beats are laid out above the caption lane (0.10 Q), which
        // may fix such a beat but never breaks another one.)
        let base = layout_report(&c.base, &frame);
        if base.passed() {
            assert_eq!(
                report.verdict,
                LayoutVerdict::Pass,
                "{}:\n{}",
                c.label,
                report.to_text()
            );
        } else {
            // Speech retimes beats, so compare what failed, not the frame.
            let key = |r: &motion_core::LayoutReport| {
                r.findings
                    .iter()
                    .map(|f| (f.scene.clone(), f.layer.clone(), format!("{:?}", f.check)))
                    .collect::<Vec<_>>()
            };
            let base_keys = key(&base);
            for k in key(&report) {
                assert!(base_keys.contains(&k), "{}: new finding {k:?}", c.label);
            }
        }
        assert!(
            report.findings.iter().all(|f| f.scene != CAPTION_SCENE_ID),
            "{}:\n{}",
            c.label,
            report.to_text()
        );
        let caps = caption_report(&c.project, &c.speech);
        assert!(caps.passed(), "{}:\n{}", c.label, caps.to_text());
    }
}

#[test]
fn layout_qa_checks_the_caption_scene() {
    // A caption pushed outside the safe area is caught by layout QA.
    let intent = CreativeIntent::from_json(&read(STORIES[0])).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read(STYLES[0])).expect("style");
    let base = compile(&intent, &style, (1080, 1920), None);
    let speech = speech_for(&intent, &base);
    let mut project = compile(&intent, &style, (1080, 1920), Some(speech));
    let frame = motion_core::compiler::layout_frame::LayoutFrame::new(1080, 1920).expect("frame");
    let scene = project
        .scenes
        .iter_mut()
        .find(|s| s.id == CAPTION_SCENE_ID)
        .expect("captions");
    for page in &mut scene.layers {
        if let LayerKind::Group { children } = &mut page.kind {
            for l in children.iter_mut() {
                if matches!(l.kind, LayerKind::Text(_)) {
                    l.x += 300.0;
                }
            }
        }
    }
    let report = layout_report(&project, &frame);
    assert_eq!(report.verdict, LayoutVerdict::Fail);
    assert!(report
        .findings
        .iter()
        .any(|f| f.scene == CAPTION_SCENE_ID && f.layer.starts_with("cap.w")));
}

#[test]
fn compile_is_deterministic() {
    let intent = CreativeIntent::from_json(&read(STORIES[0])).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read(STYLES[2])).expect("style");
    let base = compile(&intent, &style, (1080, 1920), None);
    let speech = speech_for(&intent, &base);
    let a = compile(&intent, &style, (1080, 1920), Some(speech.clone()));
    let b = compile(&intent, &style, (1080, 1920), Some(speech));
    assert_eq!(
        serde_json::to_string(&a).expect("json"),
        serde_json::to_string(&b).expect("json")
    );
}

/// Dev aid: `MOTION_CAPTIONS_DUMP=<dir> cargo test -p motion-core --test captions dump`
/// writes one 9:16 project per taste (for `motion-engine render --frame N`).
#[test]
fn dump_projects_for_rendering() {
    let Ok(dir) = std::env::var("MOTION_CAPTIONS_DUMP") else {
        return;
    };
    let intent = CreativeIntent::from_json(&read(STORIES[0])).expect("intent");
    for style_rel in STYLES {
        let style: StyleProfile = serde_json::from_str(&read(style_rel)).expect("style");
        let base = compile(&intent, &style, (1080, 1920), None);
        let speech = speech_for(&intent, &base);
        let project = compile(&intent, &style, (1080, 1920), Some(speech.clone()));
        let name = std::path::Path::new(style_rel)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("style")
            .replace(".style.json", "");
        std::fs::write(
            format!("{dir}/{name}.motion.json"),
            serde_json::to_string(&project).expect("json"),
        )
        .expect("write");
        std::fs::write(
            format!("{dir}/{name}.speech.json"),
            serde_json::to_string(&speech).expect("json"),
        )
        .expect("write");
    }
}

#[test]
fn no_speech_compile_has_no_caption_scene() {
    let intent = CreativeIntent::from_json(&read(STORIES[0])).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read(STYLES[0])).expect("style");
    let p = compile(&intent, &style, (1080, 1920), None);
    assert!(p.scenes.iter().all(|s| s.id != CAPTION_SCENE_ID));
}

#[test]
fn caption_report_flags_missing_and_late_words() {
    let intent = CreativeIntent::from_json(&read(STORIES[0])).expect("intent");
    let style: StyleProfile = serde_json::from_str(&read(STYLES[0])).expect("style");
    let base = compile(&intent, &style, (1080, 1920), None);
    let mut speech = speech_for(&intent, &base);
    let project = compile(&intent, &style, (1080, 1920), Some(speech.clone()));
    // The speech moved after the captions were built: 0.5 s off.
    speech.words[3].start += 0.5;
    let report = caption_report(&project, &speech);
    assert!(!report.passed());
    assert!(report
        .findings
        .iter()
        .any(|f| f.check == motion_core::caption_qa::CaptionCheck::WordTiming));
    // A speech word nobody captioned.
    let mut extra = speech.clone();
    extra.words.push(SpeechWord {
        text: "ghost".to_string(),
        start: 1.0,
        end: 1.2,
        confidence: 1.0,
    });
    assert!(caption_report(&project, &extra)
        .findings
        .iter()
        .any(|f| f.check == motion_core::caption_qa::CaptionCheck::WordMissing));
    // No caption scene at all.
    let mut bare = project.clone();
    bare.scenes.retain(|s| s.id != CAPTION_SCENE_ID);
    assert!(caption_report(&bare, &speech)
        .findings
        .iter()
        .any(|f| f.check == motion_core::caption_qa::CaptionCheck::SceneMissing));
}

#[test]
fn no_captions_option_drops_the_caption_scene() {
    // (0.10 Q) `--no-captions`: a voice-led compile without subtitles.
    let intent =
        CreativeIntent::from_json(&read("examples/voice_10/city_water.intent.json")).unwrap();
    let style = StyleProfile::from_json(&read("examples/taste/dark_technical.style.json")).unwrap();
    let base = compile(&intent, &style, (1080, 1920), None);
    let speech = speech_for(&intent, &base);
    let library = AssetLibrary::new(repo().join("assets"));
    let opts = CompileOptions {
        canvas: Some((1080, 1920)),
        speech: Some(speech.clone()),
        no_captions: true,
        ..CompileOptions::default()
    };
    let p = compile_with_options(
        &intent,
        &style,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap();
    assert!(p.scenes.iter().all(|s| s.id != CAPTION_SCENE_ID));
    // Beats keep the speech-led timing.
    let with = compile(&intent, &style, (1080, 1920), Some(speech));
    let starts = |p: &MotionProject| {
        p.scenes
            .iter()
            .filter(|s| s.lifecycle.is_some())
            .map(|s| s.start_seconds)
            .collect::<Vec<_>>()
    };
    assert_eq!(starts(&p), starts(&with));
}
