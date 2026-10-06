//! (0.23 A5a) The studio, hype and street looks pass the hard layout checks on
//! the benchmark stories: the studio's kinetic words stay inside the safe area
//! and clear of the beat's own type, its disc hugs the picture it sits behind,
//! library pictures that would vanish into their ground carry a sticker, a
//! figure the narrator never says is still shown, and the street starburst does
//! not cover the picture.
//!
//! Every story is compiled the way the product path does (`--art auto
//! --variety <story seed> --speech <committed offline fixture>`), and also
//! without `--variety`. The picture facts the layout QA needs (mean colour,
//! alpha bounds) come from the library's own manifests, as the CLI's image index
//! reads them from the first frame of each loop. The stories the layers look
//! owns (`layers`: a focal layer out of focus, a headline on the strata) are not
//! judged here: they belong to the layer stack.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::assets::{AssetManifest, ManifestEntry};
use motion_core::compiler::art_direction::{ArtMode, Look};
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning,
    WARN_VALUE_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::LayoutFinding;
use motion_core::scene::{AssetKind, Layer, LayerKind, MotionProject};
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord};
use motion_core::style::StyleProfile;
use motion_core::{layout_report_with, ImageFacts, ImageIndex};

/// The benchmark stories the studio, hype and street rows are judged on
/// (`docs/plans/sprint_0_23/tools/variety_report.py`), without `layers`.
const STORIES: [(&str, &str); 7] = [
    (
        "sleep_review",
        "docs/plans/sprint_0_23/stories/sleep_review.intent.json",
    ),
    (
        "money_review",
        "docs/plans/sprint_0_23/stories/money_review.intent.json",
    ),
    (
        "collection-accumulate",
        "examples/public/collection-accumulate.intent.json",
    ),
    ("state-change", "examples/public/state-change.intent.json"),
    (
        "derived-metric",
        "examples/public/derived-metric.intent.json",
    ),
    ("ai_learns", "examples/topics/ai_learns.intent.json"),
    ("space", "examples/cinematic/space.intent.json"),
];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read_repo(path: &str) -> String {
    let full = repo().join(path);
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{}: {e}", full.display()))
}

/// The CLI's `--variety auto` seed (FNV-1a over the title and statements).
fn story_seed(intent: &CreativeIntent) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut eat = |s: &str| {
        for b in s.bytes().chain([0u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    };
    eat(&intent.title);
    for b in &intent.beats {
        eat(&b.statement);
    }
    h
}

fn style_for(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(r#"{{"tone":"{tone}"}}"#)).expect("style")
}

fn from_json(json: &str) -> CreativeIntent {
    CreativeIntent::from_json(json).expect("intent")
}

/// The story's committed offline speech map, repaired against its spoken lines.
fn fixture_speech(name: &str, intent: &CreativeIntent) -> SpeechMap {
    let map = SpeechMap::from_json(&read_repo(&format!(
        "golden/fixtures/variety/{name}.speech.json"
    )))
    .expect("speech fixture");
    let spoken: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    repair(&map, &spoken).0
}

/// A voice-over that says each beat's narration (else statement), words 0.32 s
/// apart and 0.5 s between beats.
fn speech_for(intent: &CreativeIntent) -> SpeechMap {
    let (mut words, mut sentences) = (Vec::new(), Vec::new());
    let mut t = 0.3;
    for (i, beat) in intent.beats.iter().enumerate() {
        let line = beat
            .narration
            .clone()
            .unwrap_or_else(|| beat.statement.clone());
        let start = t;
        for w in motion_core::speech::statement_tokens(&line) {
            words.push(SpeechWord {
                text: w,
                start: (t * 1000.0_f64).round() / 1000.0,
                end: ((t + 0.27) * 1000.0_f64).round() / 1000.0,
                confidence: 1.0,
            });
            t += 0.32;
        }
        sentences.push(SpeechSentence {
            beat: i,
            start,
            end: t,
        });
        t += 0.5;
    }
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: "0.1".to_string(),
        audio: "voice.wav".to_string(),
        sample_rate: 48_000,
        duration: t,
        provider: "fixture".to_string(),
        model: "fixture".to_string(),
        voice: "fixture".to_string(),
        words,
        sentences,
    }
}

/// `--art auto` (or a forced look) with a voice-over; `variety` is the product
/// path's seed.
fn compile(
    intent: &CreativeIntent,
    tone: &str,
    look: Option<Look>,
    variety: Option<u64>,
    speech: SpeechMap,
) -> (MotionProject, Vec<CompileWarning>) {
    compile_in(
        AssetLibrary::new(repo().join("assets")),
        intent,
        tone,
        look,
        variety,
        speech,
    )
}

/// [`compile`] with a given library (`--asset-family` enables families).
fn compile_in(
    library: AssetLibrary,
    intent: &CreativeIntent,
    tone: &str,
    look: Option<Look>,
    variety: Option<u64>,
    speech: SpeechMap,
) -> (MotionProject, Vec<CompileWarning>) {
    let opts = CompileOptions {
        art: Some(look.map_or(ArtMode::Auto, ArtMode::Force)),
        variety,
        speech: Some(speech),
        ..CompileOptions::default()
    };
    compile_with_report(
        intent,
        &style_for(tone),
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

/// What the CLI's image index measures for the library pictures of a project: the
/// facts of the family manifest entry each asset was served from (a still, or
/// the loop whose first frame stands for it).
fn library_index(project: &MotionProject) -> ImageIndex {
    let mut entries: BTreeMap<(String, String), ManifestEntry> = BTreeMap::new();
    let root = repo().join("assets/library");
    for dir in std::fs::read_dir(&root).expect("library").flatten() {
        let family = dir.file_name().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(dir.path().join("manifest.json")) else {
            continue;
        };
        let Ok(manifest) = AssetManifest::from_json(&text) else {
            continue;
        };
        for e in manifest.assets {
            let stem = e.path.trim_end_matches(".png").to_string();
            entries.insert((family.clone(), stem), e);
        }
    }
    let mut index = ImageIndex::new();
    for a in &project.assets {
        let parts: Vec<&str> = a.path.split('/').collect();
        let (family, stem) = match (a.kind, parts.as_slice()) {
            (AssetKind::SpriteSequence, ["library", fam, "loops", stem]) => (*fam, *stem),
            (AssetKind::Image, ["library", fam, file]) => (*fam, file.trim_end_matches(".png")),
            _ => continue,
        };
        if let Some(e) = entries.get(&(family.to_string(), stem.to_string())) {
            index.insert(a.path.clone(), ImageFacts::from_entry(e));
        }
    }
    index
}

/// The layout QA findings (every check, with the picture facts) of a project.
fn findings(project: &MotionProject) -> Vec<LayoutFinding> {
    let frame = LayoutFrame::new(project.canvas.width, project.canvas.height).expect("frame");
    layout_report_with(project, &frame, &library_index(project)).findings
}

fn show(found: &[LayoutFinding]) -> Vec<String> {
    found
        .iter()
        .map(|f| format!("{}:{}:{:?}: {}", f.scene, f.layer, f.check, f.detail))
        .collect()
}

/// Ink boxes `(x0, y0, x1, y1)` of every text layer of `layers` whose id passes
/// `keep` (stage children are in canvas coordinates; groups offset them).
fn text_boxes(layers: &[Layer], keep: &dyn Fn(&str) -> bool) -> Vec<(String, [f32; 4])> {
    fn walk(
        ls: &[Layer],
        ox: f32,
        oy: f32,
        keep: &dyn Fn(&str) -> bool,
        out: &mut Vec<(String, [f32; 4])>,
    ) {
        for l in ls.iter().filter(|l| l.visible) {
            let (w, h) = (l.width * l.scale_x.abs(), l.height * l.scale_y.abs());
            let (x0, y0) = (ox + l.x - l.anchor_x * w, oy + l.y - l.anchor_y * h);
            match &l.kind {
                LayerKind::Group { children } => walk(children, x0, y0, keep, out),
                LayerKind::Text(style) if keep(&l.id) && !style.text.trim().is_empty() => {
                    let (top, bottom) = style.ink.map_or((0.0, h), |i| (i.top, i.bottom));
                    out.push((l.id.clone(), [x0, y0 + top, x0 + w, y0 + bottom]));
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(layers, 0.0, 0.0, keep, &mut out);
    out
}

fn is_kinetic(id: &str) -> bool {
    id.contains(".kw.")
}

fn is_beat_type(id: &str) -> bool {
    !is_kinetic(id) && !id.contains("ghost")
}

fn dropped(w: &[CompileWarning]) -> Vec<String> {
    w.iter()
        .filter(|w| w.code == WARN_VALUE_DROPPED)
        .map(|w| w.message.clone())
        .collect()
}

#[test]
fn studio_hype_and_street_pass_the_layout_checks_on_the_benchmark_stories() {
    let mut bad: Vec<String> = Vec::new();
    for (name, path) in STORIES {
        let intent = from_json(&read_repo(path));
        let seed = story_seed(&intent);
        for tone in ["studio", "hype", "street"] {
            for variety in [Some(seed), None] {
                let (p, w) = compile(&intent, tone, None, variety, fixture_speech(name, &intent));
                let key = format!("{name} x {tone} variety={}", variety.is_some());
                for f in findings(&p).iter() {
                    bad.push(format!("{key}: {}", show(std::slice::from_ref(f))[0]));
                }
                // The product path drops no value. (Without `--variety` the late
                // aggregate of `collection-accumulate` and `state-change`'s second
                // state are not shown in any look: the layer stack and collection
                // fixes of A5b, not the looks judged here.)
                if variety.is_some() {
                    for d in dropped(&w) {
                        bad.push(format!("{key}: value_dropped {d}"));
                    }
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} finding(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

#[test]
fn the_studio_words_stay_in_the_safe_area_and_clear_of_the_beats_own_type() {
    let mut bad: Vec<String> = Vec::new();
    let mut words = 0usize;
    for (name, path) in STORIES {
        let intent = from_json(&read_repo(path));
        let seed = story_seed(&intent);
        for variety in [Some(seed), None] {
            let (p, _) = compile(
                &intent,
                "studio",
                None,
                variety,
                fixture_speech(name, &intent),
            );
            let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
            let safe = frame.safe;
            for scene in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
                let kw = text_boxes(&scene.layers, &is_kinetic);
                let other = text_boxes(&scene.layers, &is_beat_type);
                words += kw.len();
                for (id, b) in &kw {
                    // Every kinetic word lies in the safe area, with what the
                    // layout QA grants: 2 u and the padding the layer box adds
                    // round the glyphs (`layout_qa::box_padding`).
                    let tol = 2.0 * frame.u + (0.02 * (b[2] - b[0]) + 2.0) / 1.02;
                    if b[0] < safe.x - tol
                        || b[2] > safe.x + safe.w + tol
                        || b[1] < safe.y - tol
                        || b[3] > safe.y + safe.h + tol
                    {
                        bad.push(format!("{name} {id}: outside the safe area {b:?}"));
                    }
                    // Under a direction seed the words also keep off the headline,
                    // the stat and the kicker.
                    if variety.is_some() {
                        for (oid, o) in &other {
                            if b[0] < o[2] && o[0] < b[2] && b[1] < o[3] && o[1] < b[3] {
                                bad.push(format!("{name} {id} overlaps {oid}: {b:?} {o:?}"));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(words > 100, "the stories have spoken words to set: {words}");
    assert!(
        bad.is_empty(),
        "{} problem(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// A narrated single object with a value the narrator never says.
const OBJECT_VALUE_NARRATED: &str = r#"{
  "version": "0.2", "title": "single", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Savings grow",
     "narration": "Put a little away every month and watch your savings grow.",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "energy": "impact", "keyword": "grow"}
  ]
}"#;

#[test]
fn a_figure_the_narrator_never_says_is_shown_under_the_kicker_and_clear_of_the_words() {
    let intent = from_json(OBJECT_VALUE_NARRATED);
    for variety in [Some(story_seed(&intent)), None] {
        let (p, w) = compile(
            &intent,
            "auto",
            Some(Look::StudioPop),
            variety,
            speech_for(&intent),
        );
        assert!(dropped(&w).is_empty(), "{:?}", dropped(&w));
        let scene = p.scenes.iter().find(|s| s.id == "beat_1").expect("beat 1");
        let figure = text_boxes(&scene.layers, &|id| id.ends_with(".figure"));
        assert_eq!(figure.len(), 1, "one figure label: {figure:?}");
        let (_, f) = figure[0];
        let safe = LayoutFrame::new(p.canvas.width, p.canvas.height)
            .expect("frame")
            .safe;
        assert!(
            f[0] >= safe.x - 1.0 && f[2] <= safe.x + safe.w + 1.0,
            "{f:?}"
        );
        for (id, o) in text_boxes(&scene.layers, &|id| {
            !id.ends_with(".figure") && !id.contains("ghost")
        }) {
            assert!(
                !(f[0] < o[2] && o[0] < f[2] && f[1] < o[3] && o[1] < f[3]),
                "the figure overlaps {id}: {f:?} {o:?}"
            );
        }
        // The narrator's words are not replaced by it: the beat still has them.
        assert!(!text_boxes(&scene.layers, &is_kinetic).is_empty());
    }
}

#[test]
fn a_figure_the_narrator_says_is_set_as_words_and_gets_no_label() {
    let intent = from_json(
        r#"{
  "version": "0.2", "title": "single", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Savings grow",
     "narration": "You put away $58 and watch your savings grow.",
     "primary": {"kind": "object", "asset": "piggy_bank", "value": "$58", "meaning": "savings"},
     "energy": "impact", "keyword": "grow"}
  ]
}"#,
    );
    let (p, w) = compile(
        &intent,
        "auto",
        Some(Look::StudioPop),
        None,
        speech_for(&intent),
    );
    assert!(dropped(&w).is_empty(), "{:?}", dropped(&w));
    let scene = p.scenes.iter().find(|s| s.id == "beat_1").expect("beat 1");
    assert!(text_boxes(&scene.layers, &|id| id.ends_with(".figure")).is_empty());
}

#[test]
fn habit_math_in_the_studio_passes_the_layout_checks() {
    // The picture-relationship story (carried and replaced pictures, ranked
    // pictures, a state change), compiled with `--asset-family clay_props_3d`.
    let intent = from_json(&read_repo(
        "docs/plans/sprint_0_23/stories/habit_math.intent.json",
    ));
    let mut bad: Vec<String> = Vec::new();
    for variety in [Some(story_seed(&intent)), None] {
        let library = AssetLibrary::new(repo().join("assets"))
            .with_families(vec!["clay_props_3d".to_string()]);
        let (p, w) = compile_in(
            library,
            &intent,
            "studio",
            None,
            variety,
            speech_for(&intent),
        );
        for f in findings(&p) {
            bad.push(format!("variety={}: {}", variety.is_some(), show(&[f])[0]));
        }
        if variety.is_some() {
            bad.extend(dropped(&w));
        }
    }
    assert!(
        bad.is_empty(),
        "{} finding(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Kinetic words never sit on a beat's content
// ---------------------------------------------------------------------------

/// A readable box at a sampled frame: the layer's id, whether it is type, and
/// its box `[x0, y0, x1, y1]` on the canvas (ink rows for type, else the layer
/// box).
struct Shown {
    id: String,
    text: bool,
    rect: [f32; 4],
}

/// Every readable, non-decorative leaf of the beat `scene` at `frame`: a layer
/// whose own opacity compounded over its parents is at least 0.5. The look's own
/// ghost word and the studio disc and floor shadow are decoration; so is any
/// non-text layer that spans most of the canvas (a ground).
fn shown_at(project: &MotionProject, frame: u32, scene: &str) -> Vec<Shown> {
    use motion_core::timeline::{evaluate_frame, ResolvedLayer};
    fn walk(
        layers: &[ResolvedLayer<'_>],
        scene: &str,
        parent: f32,
        canvas: (f32, f32),
        out: &mut Vec<Shown>,
    ) {
        for l in layers {
            let opacity = parent * l.opacity;
            if !matches!(l.kind, LayerKind::Group { .. }) && l.scene == Some(scene) {
                let decor = motion_core::layout_qa::is_decorative(l.id, l.kind)
                    || l.id.contains("disc")
                    || l.id.contains("shadow")
                    || l.id.contains("ghost");
                if opacity >= 0.5 && !decor && l.width > 0.0 && l.height > 0.0 {
                    let (top, bottom, has_text) = match l.kind {
                        LayerKind::Text(style) => {
                            let ink = style.ink.map_or((0.0, l.height), |i| (i.top, i.bottom));
                            let shown = l.text.as_deref().unwrap_or(&style.text);
                            (ink.0, ink.1, !shown.trim().is_empty())
                        }
                        _ => (0.0, l.height, true),
                    };
                    let is_text = matches!(l.kind, LayerKind::Text(_));
                    let corners = [(0.0, top), (l.width, top), (0.0, bottom), (l.width, bottom)]
                        .map(|(x, y)| l.transform.apply(x, y));
                    let rect = [
                        corners.iter().map(|c| c.0).fold(f32::MAX, f32::min),
                        corners.iter().map(|c| c.1).fold(f32::MAX, f32::min),
                        corners.iter().map(|c| c.0).fold(f32::MIN, f32::max),
                        corners.iter().map(|c| c.1).fold(f32::MIN, f32::max),
                    ];
                    let area = (rect[2] - rect[0]) * (rect[3] - rect[1]);
                    let ground = !is_text && area > 0.5 * canvas.0 * canvas.1;
                    if !ground && has_text {
                        out.push(Shown {
                            id: l.id.to_string(),
                            text: is_text,
                            rect,
                        });
                    }
                }
            }
            walk(&l.children, scene, opacity, canvas, out);
        }
    }
    let mut out = Vec::new();
    if let Ok(f) = evaluate_frame(project, frame) {
        let canvas = (project.canvas.width as f32, project.canvas.height as f32);
        walk(&f.layers, scene, 1.0, canvas, &mut out);
    }
    out
}

/// Share of the smaller box that two boxes share.
fn shared_share(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let (ix, iy) = (
        a[2].min(b[2]) - a[0].max(b[0]),
        a[3].min(b[3]) - a[1].max(b[1]),
    );
    if ix <= 0.0 || iy <= 0.0 {
        return 0.0;
    }
    let area = |r: &[f32; 4]| ((r[2] - r[0]) * (r[3] - r[1])).max(1e-3);
    ix * iy / area(a).min(area(b))
}

/// The overlaps of a project's beats, sampled every 0.1 s from ENTER to
/// ANTICIPATE: (a kinetic word over any other readable content layer, a
/// non-kinetic text over another non-kinetic text), each more than 2 % of the
/// smaller box.
fn overlaps_of(name: &str, project: &MotionProject) -> (Vec<String>, Vec<String>) {
    let fps = project.canvas.fps as f64;
    let (mut words, mut pairs): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
    for scene in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let Some(life) = scene.lifecycle else {
            continue;
        };
        let mut local = life.enter + 0.05;
        while local < life.anticipate {
            let frame = ((scene.start_seconds + local) * fps).round() as u32;
            let shown = shown_at(project, frame, &scene.id);
            for (i, a) in shown.iter().enumerate() {
                let a_kinetic = a.id.contains(".kw.");
                for b in shown.iter().skip(i + 1) {
                    let b_kinetic = b.id.contains(".kw.");
                    if (a_kinetic && b_kinetic) || shared_share(&a.rect, &b.rect) <= 0.02 {
                        continue;
                    }
                    let line = format!("{name} {} t={local:.1}: {} x {}", scene.id, a.id, b.id);
                    if a_kinetic || b_kinetic {
                        words.push(line);
                    } else if a.text && b.text {
                        pairs.push(line);
                    }
                }
            }
            local += 0.1;
        }
    }
    (words, pairs)
}

#[test]
fn the_studio_words_never_overlap_the_beats_content_at_any_sampled_frame() {
    let mut stories: Vec<(String, CreativeIntent, SpeechMap)> = Vec::new();
    for (name, path) in [
        ("sleep_review", STORIES[0].1),
        ("money_review", STORIES[1].1),
        ("ai_learns", STORIES[5].1),
    ] {
        let intent = from_json(&read_repo(path));
        let speech = fixture_speech(name, &intent);
        stories.push((name.to_string(), intent, speech));
    }
    // The picture-relationship story has no committed speech map: a synthetic
    // voice-over that says each beat's narration, 0.32 s a word.
    let habit = from_json(&read_repo(
        "docs/plans/sprint_0_23/stories/habit_math.intent.json",
    ));
    let speech = speech_for(&habit);
    stories.push(("habit_math".to_string(), habit, speech));

    let (mut words, mut kinetic, mut pairs) = (Vec::new(), 0usize, Vec::new());
    for (name, intent, speech) in stories {
        let library = AssetLibrary::new(repo().join("assets"))
            .with_families(vec!["clay_props_3d".to_string()]);
        let library = if name == "habit_math" {
            library
        } else {
            AssetLibrary::new(repo().join("assets"))
        };
        let (p, _) = compile_in(
            library,
            &intent,
            "studio",
            None,
            Some(story_seed(&intent)),
            speech,
        );
        let (w, t) = overlaps_of(&name, &p);
        pairs.extend(t);
        kinetic += p
            .scenes
            .iter()
            .map(|s| text_boxes(&s.layers, &is_kinetic).len())
            .sum::<usize>();
        words.extend(w);
    }
    // The builders' own type (a word chain, a state change's before and after)
    // is not the looks' to move: reported, not asserted.
    println!(
        "kinetic words compiled: {kinetic}; on content: {}; other text pairs: {}",
        words.len(),
        pairs.len()
    );
    for p in pairs.iter().take(12) {
        println!("  other text pair: {p}");
    }
    assert!(kinetic > 100, "the stories have spoken words to set");
    assert!(
        words.is_empty(),
        "{} kinetic word(s) on content:\n{}",
        words.len(),
        words.join("\n")
    );
}
