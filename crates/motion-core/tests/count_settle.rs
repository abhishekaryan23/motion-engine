//! (0.23 W4) A count settles when its number is said, and holds before the
//! exit.
//!
//! The eight bench stories (the variety bench's matrix: the two review stories
//! and six examples, each with its committed offline speech fixture under
//! `golden/fixtures/variety/`) are compiled in all nine tones the way the
//! product path compiles them (`--art auto --variety auto --speech`). Every
//! text layer driven by a `Count` motion must show its final value (the first
//! frame whose text is the final text, `checks::COUNT_UNSETTLED`) within
//! [`COUNT_SETTLE_S`] of the word it is said with, and keep it at least
//! [`COUNT_HOLD_S`] before the layer leaves or the beat's ANTICIPATE.
//!
//! The anchor of a counter is the start of the word its number is said with
//! (the spoken number, else the earliest spoken word of its reveal group),
//! else READ (no speech names it); never earlier than the count's own start:
//! a number that arrives after READ (an EVOLVE-staged second value) has until
//! 0.8 s after it arrives. A running total (`accumulate`) steps up as each
//! card lands, so it is anchored to the last card it counts.
//!
//! The hold is limited by the beat when the layer arrives too close to its
//! exit for a real count plus the hold (`exit - start < COUNT_HOLD_S +
//! COUNT_MIN_S`): such counters must still land on their word; the test lists
//! them. Set `COUNT_SETTLE_REPORT=<mode>` (`product`, `plain`, `speech`, `art`,
//! `artspeech`; `--nocapture`) to print every counter of a mode's matrix, or
//! `COUNT_SETTLE_DIR=<dir>` to measure scenes the CLI wrote as
//! `<story>__<tone>__<mode>.motion.json` instead of compiling.

use std::collections::BTreeMap;
use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::checks::{COUNT_HOLD_S, COUNT_SETTLE_S};
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::speech_plan::{anchor_time, COUNT_MIN_S};
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{
    compile_with_report, ApproxMeasure, AssetLibrary, CompileOptions, CompileWarning,
    WARN_CUE_CLAMPED, WARN_CUE_DROPPED,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionOp, MotionProject};
use motion_core::speech::{repair, SpeechMap, SpeechSentence, SpeechWord, SPEECH_VERSION};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, format_count, frame_time, ResolvedLayer};

/// The variety bench's stories (`scripts/variety_bench.sh --list-stories`).
const STORIES: [(&str, &str); 8] = [
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
    ("layers", "examples/public/layers.intent.json"),
    ("ai_learns", "examples/topics/ai_learns.intent.json"),
    ("space", "examples/cinematic/space.intent.json"),
];

const TONES: [&str; 9] = [
    "auto",
    "editorial",
    "technical",
    "playful",
    "street",
    "documentary",
    "hype",
    "studio",
    "cinematic",
];

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(path: &str) -> String {
    std::fs::read_to_string(repo().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
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

/// How a story is compiled.
#[derive(Clone, Copy)]
struct Mode {
    speech: bool,
    art: bool,
    variety: bool,
}

/// The product path (`--art auto --variety auto --speech <fixture>`).
const PRODUCT: Mode = Mode {
    speech: true,
    art: true,
    variety: true,
};
/// Default compile, no voice-over (every count is anchored to READ).
const PLAIN: Mode = Mode {
    speech: false,
    art: false,
    variety: false,
};
/// `--art auto`, no voice-over.
const ART: Mode = Mode {
    speech: false,
    art: true,
    variety: false,
};
/// `--art auto --speech` (the reveal anchors are recorded, no variety).
const ART_SPEECH: Mode = Mode {
    speech: true,
    art: true,
    variety: false,
};

struct Compiled {
    project: MotionProject,
    warnings: Vec<CompileWarning>,
    speech: Option<SpeechMap>,
}

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style")
}

fn compile_intent(
    intent: &CreativeIntent,
    style: &StyleProfile,
    speech: Option<SpeechMap>,
    mode: Mode,
) -> Result<Compiled, String> {
    let opts = CompileOptions {
        art: mode.art.then_some(ArtMode::Auto),
        variety: mode.variety.then(|| story_seed(intent)),
        speech: speech.clone(),
        ..CompileOptions::default()
    };
    compile_with_report(
        intent,
        style,
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .map(|(project, warnings)| Compiled {
        project,
        warnings,
        speech,
    })
    .map_err(|e| e.to_string())
}

fn compile_story(story: usize, tone: &str, mode: Mode) -> Result<Compiled, String> {
    let (name, path) = STORIES[story];
    let intent = CreativeIntent::from_json(&read(path)).expect("intent");
    let speech = mode.speech.then(|| {
        let map = SpeechMap::from_json(&read(&format!(
            "golden/fixtures/variety/{name}.speech.json"
        )))
        .expect("speech fixture");
        let spoken: Vec<String> = intent
            .beats
            .iter()
            .map(|b| display_text(b, true).spoken)
            .collect();
        repair(&map, &spoken).0
    });
    compile_intent(&intent, &style(tone), speech, mode)
}

// ---------------------------------------------------------------------------
// Measuring a counter
// ---------------------------------------------------------------------------

/// What the number is anchored to.
#[derive(Debug, Clone, PartialEq)]
enum Anchor {
    /// The narrator says it (scene-local start of the word).
    Word(f64),
    /// No word names it: READ.
    Read(f64),
    /// A running total: the last item it counts lands (scene-local).
    Item(f64),
}

impl Anchor {
    fn at(&self) -> f64 {
        match self {
            Anchor::Word(t) | Anchor::Read(t) | Anchor::Item(t) => *t,
        }
    }
}

#[derive(Debug, Clone)]
struct Row {
    story: String,
    tone: String,
    /// 1-based beat number.
    beat: usize,
    id: String,
    final_text: String,
    /// Start and end of the last count motion on the layer (scene-local).
    start: f64,
    end: f64,
    /// First moment the layer shows its final text (scene-local, on a frame).
    settle: f64,
    anchor: Anchor,
    /// The word it is said with, else READ (scene-local).
    literal: f64,
    /// The layer leaves, or the beat anticipates (scene-local).
    exit: f64,
    read: f64,
}

impl Row {
    /// The anchor, never earlier than the count itself (a running total's
    /// anchor is the card it counts: it may lag that card, never lead it).
    fn anchor_at(&self) -> f64 {
        match self.anchor {
            Anchor::Item(t) => t,
            _ => self.anchor.at().max(self.start),
        }
    }
    /// The plainest reading of the check: the word it is said with (else
    /// READ), no allowance for a late arrival, and the hold with no
    /// exemption. What `checks::COUNT_UNSETTLED` reports on this counter.
    fn literal_failure(&self) -> bool {
        self.settle - self.literal > COUNT_SETTLE_S + 1e-9 || self.hold() < COUNT_HOLD_S - 1e-9
    }
    fn after_anchor(&self) -> f64 {
        self.settle - self.anchor_at()
    }
    fn hold(&self) -> f64 {
        self.exit - self.settle
    }
    /// The layer arrives too close to its exit for a real count and the hold.
    fn arrival_limited(&self) -> bool {
        self.exit - self.start < COUNT_HOLD_S + COUNT_MIN_S
    }
    fn tag(&self) -> String {
        format!(
            "{} x {} beat {} {}",
            self.story, self.tone, self.beat, self.id
        )
    }
    /// What fails, if anything.
    fn finding(&self) -> Option<String> {
        if !self.settle.is_finite() {
            return Some(format!("{}: never shows {:?}", self.tag(), self.final_text));
        }
        if self.after_anchor() > COUNT_SETTLE_S + 1e-9 {
            return Some(format!(
                "{}: shows {:?} {:.2}s after {:?} (limit {COUNT_SETTLE_S}s)",
                self.tag(),
                self.final_text,
                self.after_anchor(),
                self.anchor
            ));
        }
        if self.hold() < COUNT_HOLD_S - 1e-9 && !self.arrival_limited() {
            return Some(format!(
                "{}: holds {:?} only {:.2}s before the exit (limit {COUNT_HOLD_S}s) \
                 though it arrives {:.2}s before it",
                self.tag(),
                self.final_text,
                self.hold(),
                self.exit - self.start
            ));
        }
        None
    }
}

/// Ids from a top-level layer down to `id`.
fn trail<'a>(layers: &'a [Layer], id: &str, out: &mut Vec<&'a str>) -> bool {
    for l in layers {
        out.push(&l.id);
        if l.id == id {
            return true;
        }
        if let LayerKind::Group { children } = &l.kind {
            if trail(children, id, out) {
                return true;
            }
        }
        out.pop();
    }
    false
}

fn text_in(layers: &[ResolvedLayer<'_>], id: &str) -> Option<String> {
    for l in layers {
        if l.id == id {
            return match l.kind {
                LayerKind::Text(style) => {
                    Some(l.text.clone().unwrap_or_else(|| style.text.clone()))
                }
                _ => None,
            };
        }
        if let Some(t) = text_in(&l.children, id) {
            return Some(t);
        }
    }
    None
}

/// The counters of every beat scene, measured on the evaluated timeline.
fn rows(story: &str, tone: &str, c: &Compiled) -> Vec<Row> {
    let project = &c.project;
    let fps = project.canvas.fps;
    let mut out = Vec::new();
    for (bi, scene) in project
        .scenes
        .iter()
        .filter(|s| s.id.starts_with("beat_"))
        .enumerate()
    {
        let Some(life) = scene.lifecycle else {
            continue;
        };
        // Spoken words, scene-local.
        let spoken: Vec<(String, f64)> = match &c.speech {
            Some(speech) => speech
                .sentences
                .iter()
                .find(|s| s.beat == bi)
                .map(|s| {
                    speech
                        .words_in(s)
                        .iter()
                        .map(|w| (w.text.clone(), w.start - scene.start_seconds))
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let prefix = scene
            .layers
            .first()
            .and_then(|l| l.id.split('.').next())
            .unwrap_or("");
        // The last count motion on each layer decides its final value.
        let mut last: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, m) in scene.motions.iter().enumerate() {
            if matches!(m.op, MotionOp::Count { .. }) {
                last.insert(&m.target, i);
            }
        }
        for (id, i) in last {
            let m = &scene.motions[i];
            let MotionOp::Count {
                to,
                decimals,
                grouping,
                prefix: pre,
                suffix,
                ..
            } = &m.op
            else {
                continue;
            };
            let final_text = format_count(*to, *decimals, *grouping, pre, suffix);
            let end = m.start + m.duration;
            // First frame showing the final text.
            let first = ((scene.start_seconds + m.start) * f64::from(fps)).floor() as u32;
            let last_frame = ((scene.start_seconds + end) * f64::from(fps)).ceil() as u32 + 1;
            let mut settle = f64::INFINITY;
            for n in first..=last_frame {
                let frame = evaluate_frame(project, n).expect("evaluate");
                if text_in(&frame.layers, id).as_deref() == Some(final_text.as_str()) {
                    settle = frame_time(fps, n) - scene.start_seconds;
                    break;
                }
            }
            // The word it is said with: the spoken number, else the earliest
            // word of its reveal group, else READ.
            let group_words: Vec<String> = project
                .project
                .art
                .as_ref()
                .and_then(|a| a.reveals.get(&scene.id))
                .into_iter()
                .flatten()
                .filter(|a| {
                    let rest = id.strip_prefix(prefix).and_then(|r| r.strip_prefix('.'));
                    rest.is_some_and(|r| r == a.group || r.starts_with(&format!("{}.", a.group)))
                })
                .flat_map(|a| a.words.clone())
                .collect();
            let words: Vec<&str> = group_words.iter().map(String::as_str).collect();
            let word = if spoken.is_empty() {
                None
            } else {
                anchor_time(&spoken, &[&final_text]).or_else(|| anchor_time(&spoken, &words))
            };
            // A running total (`accumulate`) steps up as each item lands: it
            // settles with the last card it counts.
            let steps = scene
                .motions
                .iter()
                .filter(|o| o.target == id && matches!(o.op, MotionOp::Count { .. }))
                .count();
            let item_arrival = id.strip_suffix(".total").and_then(|pre| {
                let item = format!("{pre}.items.{}", steps.saturating_sub(1));
                let nested = format!("{item}.");
                scene
                    .motions
                    .iter()
                    .filter(|o| o.target == item || o.target.starts_with(&nested))
                    .map(|o| o.start)
                    .reduce(f64::min)
            });
            let anchor = match (item_arrival, word) {
                (Some(t), _) => Anchor::Item(t),
                (None, Some(t)) => Anchor::Word(t),
                (None, None) => Anchor::Read(life.read),
            };
            // The layer's exit: the earliest fade-out of it or a parent.
            let mut ids = Vec::new();
            trail(&scene.layers, id, &mut ids);
            let exit = scene
                .motions
                .iter()
                .filter(|o| ids.contains(&o.target.as_str()) && o.start >= m.start)
                .filter_map(|o| match o.op {
                    MotionOp::Fade { from, to } if to < from => Some(o.start),
                    _ => None,
                })
                .fold(life.anticipate, f64::min);
            out.push(Row {
                story: story.to_string(),
                tone: tone.to_string(),
                beat: bi + 1,
                id: id.to_string(),
                final_text,
                start: m.start,
                end,
                settle,
                anchor,
                literal: word.unwrap_or(life.read),
                exit,
                read: life.read,
            });
        }
    }
    out
}

struct Matrix {
    rows: Vec<Row>,
    failed: Vec<String>,
    warnings: Vec<CompileWarning>,
    compiled: usize,
}

fn matrix(mode: Mode) -> Matrix {
    let mut m = Matrix {
        rows: Vec::new(),
        failed: Vec::new(),
        warnings: Vec::new(),
        compiled: 0,
    };
    for (si, (story, _)) in STORIES.iter().enumerate() {
        for tone in TONES {
            match compile_story(si, tone, mode) {
                Ok(c) => {
                    m.compiled += 1;
                    m.rows.extend(rows(story, tone, &c));
                    m.warnings.extend(c.warnings);
                }
                Err(e) => m.failed.push(format!("{story} x {tone}: {e}")),
            }
        }
    }
    m
}

/// Fail with every finding of `rows`; list the counters whose hold the beat
/// limits.
fn judge(label: &str, m: &Matrix) {
    let findings: Vec<String> = m.rows.iter().filter_map(Row::finding).collect();
    let limited: Vec<String> = m
        .rows
        .iter()
        .filter(|r| r.hold() < COUNT_HOLD_S - 1e-9 && r.arrival_limited())
        .map(|r| {
            format!(
                "{} arrives {:.2}s before its exit, holds {:.2}s",
                r.tag(),
                r.exit - r.start,
                r.hold()
            )
        })
        .collect();
    eprintln!(
        "{label}: {} compiles ({} failed), {} counters, {} hold-limited by the beat, {} findings",
        m.compiled,
        m.failed.len(),
        m.rows.len(),
        limited.len(),
        findings.len()
    );
    for l in &limited {
        eprintln!("  hold-limited: {l}");
    }
    for f in &m.failed {
        eprintln!("  not compiled: {f}");
    }
    assert!(
        m.compiled * 4 >= STORIES.len() * TONES.len() * 3,
        "{label}: only {} of {} compiles succeeded: {:?}",
        m.compiled,
        STORIES.len() * TONES.len(),
        m.failed
    );
    assert!(!m.rows.is_empty(), "{label}: no counters in the matrix");
    assert!(
        findings.is_empty(),
        "{label}: {} of {} counter(s) unsettled:\n  {}",
        findings.len(),
        m.rows.len(),
        findings.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// The matrix
// ---------------------------------------------------------------------------

#[test]
fn product_path_counts_land_on_their_word_and_hold() {
    judge(
        "product (--art auto --variety auto --speech)",
        &matrix(PRODUCT),
    );
}

#[test]
fn art_with_speech_counts_land_on_their_word_and_hold() {
    judge("--art auto --speech", &matrix(ART_SPEECH));
}

#[test]
fn counts_without_a_voice_land_by_read_and_hold() {
    judge("default compile", &matrix(PLAIN));
    judge("--art auto", &matrix(ART));
}

/// A count still reads as a count: no count that is not a running total's
/// step is shorter than `COUNT_MIN_S`; every step of a running total keeps its
/// own length (they are built 0.17-0.5 s and never touched).
#[test]
fn counts_stay_counts() {
    let mut durations = Vec::new();
    for mode in [PRODUCT, PLAIN] {
        for r in matrix(mode)
            .rows
            .iter()
            .filter(|r| !r.id.ends_with(".total"))
        {
            durations.push(r.end - r.start);
            assert!(
                r.end - r.start >= COUNT_MIN_S - 1e-9,
                "{} counts for only {:.2}s",
                r.tag(),
                r.end - r.start
            );
        }
    }
    assert!(durations.len() > 20, "{} counts", durations.len());
}

#[test]
fn report() {
    // Measure compiled scenes (`<story>__<tone>__<mode>.motion.json`, as the
    // CLI wrote them) instead of compiling: `COUNT_SETTLE_DIR=<dir>`.
    if let Ok(dir) = std::env::var("COUNT_SETTLE_DIR") {
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("{dir}: {e}"))
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.to_string_lossy().ends_with(".motion.json"))
            .collect();
        files.sort();
        for path in files {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            let name = name.unwrap_or_default();
            let stem = name.trim_end_matches(".motion.json");
            let parts: Vec<&str> = stem.split("__").collect();
            let [story, tone, mode] = parts[..] else {
                continue;
            };
            let Some(si) = STORIES.iter().position(|(n, _)| *n == story) else {
                continue;
            };
            let project =
                MotionProject::from_json(&std::fs::read_to_string(&path).expect("read scene"))
                    .expect("scene json");
            let speech = matches!(mode, "speech" | "artspeech" | "product").then(|| {
                let intent = CreativeIntent::from_json(&read(STORIES[si].1)).expect("intent");
                let map = SpeechMap::from_json(&read(&format!(
                    "golden/fixtures/variety/{story}.speech.json"
                )))
                .expect("speech fixture");
                let spoken: Vec<String> = intent
                    .beats
                    .iter()
                    .map(|b| display_text(b, true).spoken)
                    .collect();
                repair(&map, &spoken).0
            });
            let c = Compiled {
                project,
                warnings: Vec::new(),
                speech,
            };
            for r in rows(story, tone, &c) {
                eprintln!(
                    "SCENE {mode} {story} {tone} beat {} {} settle {:.3} start {:.3} end {:.3} exit {:.3} anchor {:.3} literal {:.3} {}",
                    r.beat,
                    r.id,
                    r.settle,
                    r.start,
                    r.end,
                    r.exit,
                    r.anchor_at(),
                    r.literal,
                    match (r.finding(), r.literal_failure()) {
                        (Some(_), _) => "FINDING",
                        (None, true) => "LITERAL_ONLY",
                        (None, false) => "OK",
                    }
                );
            }
        }
        return;
    }
    let mode = match std::env::var("COUNT_SETTLE_REPORT").as_deref() {
        Ok("plain") => PLAIN,
        Ok("speech") => Mode {
            speech: true,
            art: false,
            variety: false,
        },
        Ok("art") => ART,
        Ok("artspeech") => ART_SPEECH,
        Ok(_) => PRODUCT,
        Err(_) => return,
    };
    let m = matrix(mode);
    for r in &m.rows {
        eprintln!(
            "{:<58} {:>9} start {:5.2} end {:5.2} dur {:4.2} settle {:5.2} {:?} (+{:5.2}) exit {:5.2} hold {:5.2} read {:5.2}{}",
            r.tag(),
            r.final_text,
            r.start,
            r.end,
            r.end - r.start,
            r.settle,
            r.anchor,
            r.after_anchor(),
            r.exit,
            r.hold(),
            r.read,
            r.finding().map(|f| format!("  FINDING {f}")).unwrap_or_default(),
        );
    }
    let cue = m
        .warnings
        .iter()
        .filter(|w| w.code == WARN_CUE_DROPPED || w.code == WARN_CUE_CLAMPED)
        .count();
    eprintln!(
        "{} counters, {} compiles, {} failed, {} cue warnings",
        m.rows.len(),
        m.compiled,
        m.failed.len(),
        cue
    );
}

// ---------------------------------------------------------------------------
// A beat too short for both
// ---------------------------------------------------------------------------

/// A 1.6 s beat says "sixty percent" at once: the number arrives with its
/// word, about 1.1 s before ANTICIPATE. A real count (0.4 s) plus a 1.0 s hold
/// does not fit, so the hold gives way: the count still lands on its word
/// within 0.8 s, runs at least 0.4 s and is done before the exit.
#[test]
fn a_beat_too_short_for_both_lands_on_its_word_and_the_hold_gives_way() {
    let intent = CreativeIntent::from_json(
        &serde_json::json!({
            "version": "0.2",
            "title": "short_count",
            "format": "vertical",
            "beats": [
                {
                    "purpose": "reveal",
                    "statement": "Sixty percent faster",
                    "narration": "Sixty percent faster.",
                    "primary": { "kind": "number", "value": "60%", "meaning": "faster" },
                    "energy": "building"
                },
                {
                    "purpose": "emphasize",
                    "statement": "Sleep well",
                    "narration": "So sleep well tonight.",
                    "primary": { "kind": "phrase", "value": "Sleep" },
                    "energy": "calm"
                }
            ]
        })
        .to_string(),
    )
    .expect("intent");
    let spoken: [&[(&str, f64, f64)]; 2] = [
        &[
            ("Sixty", 0.40, 0.80),
            ("percent", 0.85, 1.25),
            ("faster", 1.30, 1.70),
        ],
        &[
            ("So", 2.00, 2.10),
            ("sleep", 2.20, 2.60),
            ("well", 2.70, 3.00),
            ("tonight", 3.10, 3.60),
        ],
    ];
    let mut words = Vec::new();
    let mut sentences = Vec::new();
    for (beat, list) in spoken.iter().enumerate() {
        sentences.push(SpeechSentence {
            beat,
            start: list[0].1,
            end: list[list.len() - 1].2,
        });
        words.extend(list.iter().map(|(t, s, e)| SpeechWord {
            text: (*t).to_string(),
            start: *s,
            end: *e,
            confidence: 1.0,
        }));
    }
    let map = SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: 4.6,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences,
        recognised: Vec::new(),
        alignment: None,
    };
    let mut checked = 0;
    for tone in ["editorial", "playful", "technical", "cinematic"] {
        let c = compile_intent(&intent, &style(tone), Some(map.clone()), ART_SPEECH)
            .expect("compile the short beat");
        for r in rows("short_count", "tone", &c)
            .iter()
            .filter(|r| r.beat == 1)
        {
            checked += 1;
            eprintln!(
                "short beat x {tone}: {} start {:.2} end {:.2} settle {:.2} anchor {:?} exit {:.2} hold {:.2}",
                r.id,
                r.start,
                r.end,
                r.settle,
                r.anchor,
                r.exit,
                r.hold()
            );
            assert!(matches!(r.anchor, Anchor::Word(_)), "{:?}", r.anchor);
            assert!(
                r.arrival_limited(),
                "the premise: {} arrives {:.2}s before its exit",
                r.id,
                r.exit - r.start
            );
            assert!(
                r.after_anchor() <= COUNT_SETTLE_S + 1e-9,
                "{tone}: lands {:.2}s after its word",
                r.after_anchor()
            );
            assert!(
                r.end - r.start >= COUNT_MIN_S - 1e-9,
                "{tone}: still a count ({:.2}s)",
                r.end - r.start
            );
            assert!(
                r.settle <= r.exit + 1e-9,
                "{tone}: settles at {:.2}s, after the exit at {:.2}s",
                r.settle,
                r.exit
            );
            assert!(
                r.hold() < COUNT_HOLD_S,
                "{tone}: the hold cannot fit ({:.2}s)",
                r.hold()
            );
        }
    }
    assert!(checked > 0, "no counter on the short beat");
}

// ---------------------------------------------------------------------------
// Ranked bars: the value counts with its bar
// ---------------------------------------------------------------------------

const RANKING_LINE: &str = "Savings lead at four point one percent. Laptops follow at four percent. \
                            Phones sit at three point four percent. Plants trail at three point one percent.";

/// A voice-over at 0.32 s a word: each row's name and value are said in turn.
fn ranking_speech() -> SpeechMap {
    const START: f64 = 0.35;
    const STEP: f64 = 0.32;
    let toks: Vec<&str> = RANKING_LINE.split_whitespace().collect();
    let words: Vec<SpeechWord> = toks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let at = START + i as f64 * STEP;
            SpeechWord {
                text: (*t).to_string(),
                start: (at * 1000.0).round() / 1000.0,
                end: ((at + 0.8 * STEP) * 1000.0).round() / 1000.0,
                confidence: 0.9,
            }
        })
        .collect();
    let end = START + toks.len() as f64 * STEP;
    SpeechMap {
        recognised: Vec::new(),
        alignment: None,
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: end + 0.9,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words,
        sentences: vec![SpeechSentence {
            beat: 0,
            start: START,
            end,
        }],
    }
}

/// Every row's value is counted and shifted onto its bar's tip in the same
/// window; the settle pass leaves a count that already lands on its word as
/// it was built, so the figure never finishes before the bar it rides on.
#[test]
fn ranked_values_land_with_their_bars() {
    let intent = CreativeIntent::from_json(
        r#"{"version":"0.2","title":"ranking","format":"vertical","beats":[{
            "purpose":"compare","statement":"Where the money goes",
            "primary":{"kind":"collection","items":[
              {"kind":"object","asset":"coin_stack","value":"4.1%","meaning":"Savings"},
              {"kind":"object","asset":"laptop","value":"4.0%","meaning":"Laptops"},
              {"kind":"object","asset":"phone","value":"3.4%","meaning":"Phones"},
              {"kind":"object","asset":"plant","value":"3.1%","meaning":"Plants"}]}}]}"#,
    )
    .expect("intent");
    let mut counted = 0;
    for look in std::iter::once(None).chain(
        motion_core::compiler::art_direction::Look::ALL
            .into_iter()
            .map(Some),
    ) {
        let opts = CompileOptions {
            art: look.map(ArtMode::Force),
            speech: Some(ranking_speech()),
            ..CompileOptions::default()
        };
        let (project, warnings) = compile_with_report(
            &intent,
            &StyleProfile::default(),
            None,
            &AssetLibrary::new(repo().join("assets")),
            &ApproxMeasure,
            &AssetManifest::empty(),
            None,
            &opts,
        )
        .expect("compile");
        let c = Compiled {
            project,
            warnings,
            speech: opts.speech.clone(),
        };
        let label = look.map_or("no look", |l| l.name());
        for r in rows("ranking", label, &c) {
            counted += 1;
            assert!(r.finding().is_none(), "{label}: {:?}", r.finding());
            let scene = c
                .project
                .scenes
                .iter()
                .find(|s| s.id == "beat_1")
                .expect("beat 1");
            let shift_end = scene
                .motions
                .iter()
                .filter(|m| m.target == r.id && matches!(m.op, MotionOp::Move { .. }))
                .map(|m| m.start + m.duration)
                .fold(f64::NAN, f64::max);
            if shift_end.is_finite() {
                assert!(
                    (r.end - shift_end).abs() < 1e-3,
                    "{label}: {} counts until {:.3}s but rides its bar until {shift_end:.3}s",
                    r.id,
                    r.end
                );
            }
        }
    }
    assert!(counted >= 4, "{counted} ranked values counted");
}
