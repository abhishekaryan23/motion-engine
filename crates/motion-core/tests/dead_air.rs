//! (0.23 W4) The first content is on screen within `DEAD_AIR_FIRST_READABLE_S`
//! of the video start, and no beat holds longer than `DEAD_AIR_MAX_HOLD_S`
//! after its start with only the backdrop and the captions.
//!
//! The eight bench stories (the variety bench's matrix, each with its
//! committed offline speech fixture under `golden/fixtures/variety/`, and the
//! `tight/` fixtures when that folder exists) are compiled in all nine tones
//! the way the product path compiles them (`--art auto --variety auto
//! --speech`). Every beat's first *content leaf* must show in time. A content
//! leaf is the rule of `sprint23_regressions::opening_and_handoffs_have_no_dead_air`
//! (which `story_qa`'s `dead_air` follows): a leaf of the beat (or a shared
//! element) that is readable, is not decorative (`layout_qa::is_decorative`:
//! ghost words, dust, glows, wipes, stickers, ...) and is not in the
//! `backdrop` or caption scenes, with at least half of its clip window
//! revealed. "Readable" is the speech QA's own rule (`reveal_qa`, which
//! lives in `motion-render`; ported here): compounded opacity and the least
//! visible glyph at least `READABLE_OPACITY`, blur within the limit of its
//! kind (text 12 % of its font size, a picture 3 % of its short side, at
//! least `READABLE_BLUR_PX`), and its box on the canvas.
//!
//! Set `DEAD_AIR_REPORT=1` (`--nocapture`) to print the per-tone seconds to
//! the first content of every beat, and `DEAD_AIR_DIR=<dir>` to measure
//! scenes the CLI wrote as `<story>__<tone>.motion.json` instead of
//! compiling them (the before / after tables of the task report).
//!
//! The fixes apply on the product path only (`--variety`): `compile` without
//! it stays byte-identical, which two tests below keep an eye on.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::checks::{DEAD_AIR_FIRST_READABLE_S, DEAD_AIR_MAX_HOLD_S};
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::captions::CAPTION_SCENE_ID;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::is_decorative;
use motion_core::scene::{CameraOp, Layer, LayerKind, MotionProject, PostKind, Scene};
use motion_core::speech::{repair, SpeechMap, READABLE_BLUR_PX, READABLE_OPACITY, REVEAL_LEAD_MAX};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, frame_time, ResolvedFrame, ResolvedLayer};

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

/// Float noise allowed when a frame time is compared with a limit (the
/// timeline is sampled on frames, 30 fps).
const FRAME_TOLERANCE: f64 = 1e-9;

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
/// The same without `--variety` (default timing: byte-identical to before).
const NO_VARIETY: Mode = Mode {
    speech: true,
    art: true,
    variety: false,
};

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style")
}

/// A story with the speech map `speech_path` (repo-relative) repaired against
/// its spoken lines, as `compile --speech` does.
fn compile_with(
    story: usize,
    tone: &str,
    mode: Mode,
    speech_path: &str,
) -> (MotionProject, Option<SpeechMap>) {
    let (_, path) = STORIES[story];
    let intent = CreativeIntent::from_json(&read(path)).expect("intent");
    let speech = mode.speech.then(|| {
        let map = SpeechMap::from_json(&read(speech_path)).expect("speech fixture");
        let spoken: Vec<String> = intent
            .beats
            .iter()
            .map(|b| display_text(b, true).spoken)
            .collect();
        repair(&map, &spoken).0
    });
    let opts = CompileOptions {
        art: mode.art.then_some(ArtMode::Auto),
        variety: mode.variety.then(|| story_seed(&intent)),
        speech: speech.clone(),
        ..CompileOptions::default()
    };
    let project = compile_with_options(
        &intent,
        &style(tone),
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .unwrap_or_else(|e| panic!("{} x {tone}: {e}", STORIES[story].0));
    (project, speech)
}

fn compile_story(story: usize, tone: &str, mode: Mode) -> (MotionProject, Option<SpeechMap>) {
    let speech_path = format!("golden/fixtures/variety/{}.speech.json", STORIES[story].0);
    compile_with(story, tone, mode, &speech_path)
}

fn beat_scenes(p: &MotionProject) -> Vec<&Scene> {
    p.scenes
        .iter()
        .filter(|s| s.id.starts_with("beat_"))
        .collect()
}

// ---------------------------------------------------------------------------
// What counts as content: the readable-leaf rule of `motion-render::reveal_qa`
// ---------------------------------------------------------------------------

/// Blur a text leaf may carry and still read, as a share of its font size.
const TEXT_BLUR_SHARE: f32 = 0.12;
/// Blur a picture may carry and still read, as a share of its short side.
const PICTURE_BLUR_SHARE: f32 = 0.03;
/// A content layer is on screen once at least this share of its clip window
/// shows (a `clip_reveal` slides the content through a fixed window).
const ON_SCREEN_SHARE: f32 = 0.5;

struct Limits {
    width: f32,
    height: f32,
    blur_px: f32,
}

impl Limits {
    fn for_canvas(width: u32, height: u32) -> Limits {
        let short = width.min(height) as f32;
        Limits {
            width: width as f32,
            height: height as f32,
            blur_px: READABLE_BLUR_PX * short / 1080.0,
        }
    }
}

/// The drawn part of a leaf in its local box space; `None` when nothing is
/// drawn (empty text, zero trim, no points).
fn local_bounds(layer: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    match layer.kind {
        LayerKind::Polyline { points, stroke, .. } => {
            if layer.trim.is_some_and(|t| t <= 1e-3) {
                return None;
            }
            let pts = layer.points.as_deref().unwrap_or(points);
            let half = (stroke.width * 0.5).max(0.0);
            let mut b = [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ];
            for p in pts.iter().filter(|p| p[0].is_finite() && p[1].is_finite()) {
                b = [
                    b[0].min(p[0] - half),
                    b[1].min(p[1] - half),
                    b[2].max(p[0] + half),
                    b[3].max(p[1] + half),
                ];
            }
            (b[0] < b[2] && b[1] < b[3]).then_some(b)
        }
        LayerKind::Text(style) => {
            let text = layer.text.as_deref().unwrap_or(&style.text);
            if text.trim().is_empty() {
                return None;
            }
            clipped_box(layer)
        }
        _ => clipped_box(layer),
    }
}

/// The layer box minus its clip inset.
fn clipped_box(layer: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    let (w, h) = (layer.width, layer.height);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let c = layer.clip.unwrap_or_default();
    let b = [
        c.left.max(0.0) * w,
        c.top.max(0.0) * h,
        (1.0 - c.right.max(0.0)) * w,
        (1.0 - c.bottom.max(0.0)) * h,
    ];
    (b[0] < b[2] && b[1] < b[3]).then_some(b)
}

/// Whether the local rectangle, mapped to canvas px, overlaps the canvas.
fn on_canvas(layer: &ResolvedLayer<'_>, rect: [f32; 4], limits: &Limits) -> bool {
    let corners = [
        (rect[0], rect[1]),
        (rect[2], rect[1]),
        (rect[2], rect[3]),
        (rect[0], rect[3]),
    ];
    let mut b = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    let mut any = false;
    for (x, y) in corners {
        let mapped = match &layer.projective {
            Some(m) => {
                let d = m[6] * x + m[7] * y + m[8];
                if !(d.is_finite() && d > 1e-6) {
                    continue;
                }
                (
                    (m[0] * x + m[1] * y + m[2]) / d,
                    (m[3] * x + m[4] * y + m[5]) / d,
                )
            }
            None => layer.transform.apply(x, y),
        };
        if !(mapped.0.is_finite() && mapped.1.is_finite()) {
            continue;
        }
        any = true;
        b = [
            b[0].min(mapped.0),
            b[1].min(mapped.1),
            b[2].max(mapped.0),
            b[3].max(mapped.1),
        ];
    }
    any && b[0] < limits.width && b[2] > 0.0 && b[1] < limits.height && b[3] > 0.0
}

/// Whether a leaf is readable given the compounded opacity (ancestors and
/// own) and the summed squared blur radii (post, ancestors and own).
fn leaf_readable(layer: &ResolvedLayer<'_>, opacity: f32, blur_sq: f32, limits: &Limits) -> bool {
    let glyph = layer
        .glyphs
        .as_ref()
        .and_then(|g| g.iter().map(|p| p.opacity.clamp(0.0, 1.0)).reduce(f32::min))
        .unwrap_or(1.0);
    if opacity * glyph < READABLE_OPACITY {
        return false;
    }
    let limit = match layer.kind {
        LayerKind::Text(style) => limits.blur_px.max(TEXT_BLUR_SHARE * style.font_size),
        LayerKind::Image { .. } | LayerKind::Svg { .. } => limits
            .blur_px
            .max(PICTURE_BLUR_SHARE * layer.width.min(layer.height)),
        _ => limits.blur_px,
    };
    if blur_sq.sqrt() > limit + 1e-4 {
        return false;
    }
    local_bounds(layer).is_some_and(|rect| on_canvas(layer, rect, limits))
}

/// Share (0..=1) of a layer's clip window that its content fills this frame.
fn revealed_share(layer: &ResolvedLayer<'_>) -> f32 {
    let (w, h) = (layer.width, layer.height);
    let c = layer.clip.unwrap_or_default();
    let win = [
        c.left * w,
        c.top * h,
        (1.0 - c.right) * w,
        (1.0 - c.bottom) * h,
    ];
    let win_area = (win[2] - win[0]) * (win[3] - win[1]);
    if win_area <= 0.0 {
        return 0.0;
    }
    let ct = layer.content_transform;
    let (ox, oy) = if layer.projective.is_some() {
        (ct.e, ct.f)
    } else {
        let t = layer.transform;
        let det = t.a * t.d - t.b * t.c;
        if det.abs() < 1e-9 {
            (0.0, 0.0)
        } else {
            let (dx, dy) = (ct.e - t.e, ct.f - t.f);
            ((t.d * dx - t.c * dy) / det, (-t.b * dx + t.a * dy) / det)
        }
    };
    let content = [ox, oy, ox + w, oy + h];
    let iw = (win[2].min(content[2]) - win[0].max(content[0])).max(0.0);
    let ih = (win[3].min(content[3]) - win[1].max(content[1])).max(0.0);
    iw * ih / win_area
}

/// Depth-first walk carrying the compounded opacity and the summed squared
/// blur radii of the ancestors; `found` is set by a readable content leaf that
/// `accept` takes.
fn walk(
    layer: &ResolvedLayer<'_>,
    opacity: f32,
    blur_sq: f32,
    limits: &Limits,
    beat: &str,
    accept: &dyn Fn(&ResolvedLayer<'_>) -> bool,
    found: &mut bool,
) {
    let opacity = opacity * layer.opacity.clamp(0.0, 1.0);
    let own_blur = layer.blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0);
    let blur_sq = blur_sq + own_blur * own_blur;
    if let LayerKind::Group { .. } = layer.kind {
        for child in &layer.children {
            walk(child, opacity, blur_sq, limits, beat, accept, found);
        }
        return;
    }
    let scene = layer.scene;
    if scene.is_some_and(|s| s != beat)
        || scene == Some("backdrop")
        || scene == Some(CAPTION_SCENE_ID)
        || is_decorative(layer.id, layer.kind)
    {
        return;
    }
    if accept(layer)
        && leaf_readable(layer, opacity, blur_sq, limits)
        && revealed_share(layer) >= ON_SCREEN_SHARE
    {
        *found = true;
    }
}

/// Whether the frame shows a content leaf of `beat` that `accept` takes.
fn shows_content(
    frame: &ResolvedFrame<'_>,
    limits: &Limits,
    beat: &str,
    accept: &dyn Fn(&ResolvedLayer<'_>) -> bool,
) -> bool {
    let post_blur = frame
        .post
        .iter()
        .map(|p| match p.kind {
            PostKind::DirectionalBlur { length, .. } => {
                let spread = (length * p.strength).max(0.0);
                spread / 3f32.sqrt()
            }
            _ => 0.0,
        })
        .map(|r| r * r)
        .sum::<f32>();
    let mut found = false;
    for layer in &frame.layers {
        walk(layer, 1.0, post_blur, limits, beat, accept, &mut found);
        if found {
            break;
        }
    }
    found
}

/// Seconds after `scene` starts until it first shows a content leaf, looking
/// no further than `until` seconds after its start; `None` when it does not.
fn first_content(project: &MotionProject, scene: &Scene, until: f64) -> Option<f64> {
    first_content_with(project, scene, until, &|_| true)
}

/// A beat's kicker (and its rule): the small line that every beat has, which
/// is content for the dead-air limit but not what a held hero waits behind.
fn is_kicker(layer: &ResolvedLayer<'_>) -> bool {
    layer
        .id
        .split('.')
        .nth(1)
        .is_some_and(|name| name.starts_with("kicker"))
}

/// As [`first_content`], for the leaves `accept` takes.
fn first_content_with(
    project: &MotionProject,
    scene: &Scene,
    until: f64,
    accept: &dyn Fn(&ResolvedLayer<'_>) -> bool,
) -> Option<f64> {
    let fps = project.canvas.fps;
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let last = project.frame_count().saturating_sub(1);
    let mut n = (scene.start_seconds * f64::from(fps)).floor() as u32;
    while n <= last {
        let t = frame_time(fps, n);
        if t - scene.start_seconds > until + 1.0 / f64::from(fps) {
            return None;
        }
        if t >= scene.start_seconds {
            let frame = evaluate_frame(project, n).expect("evaluate");
            if shows_content(&frame, &limits, &scene.id, accept) {
                return Some(t - scene.start_seconds);
            }
        }
        n += 1;
    }
    None
}

/// The beat's limit: the first beat's content within `DEAD_AIR_FIRST_READABLE_S`
/// of the video start, every other beat's within `DEAD_AIR_MAX_HOLD_S` of its start.
fn limit_of(beat: usize) -> f64 {
    if beat == 0 {
        DEAD_AIR_FIRST_READABLE_S
    } else {
        DEAD_AIR_MAX_HOLD_S
    }
}

/// Every beat of `project` that shows its first content too late, as one
/// line each, and the seconds measured per beat.
fn judge(label: &str, project: &MotionProject) -> (Vec<String>, Vec<Option<f64>>) {
    let mut findings = Vec::new();
    let mut firsts = Vec::new();
    for (i, scene) in beat_scenes(project).into_iter().enumerate() {
        let limit = limit_of(i);
        // Look past the limit, so a finding says how late the content is.
        let first = first_content(project, scene, limit + 2.0);
        firsts.push(first);
        if first.is_none_or(|t| t > limit + FRAME_TOLERANCE) {
            findings.push(format!(
                "{label} beat {}: {} (limit {limit:.1}s after {})",
                i + 1,
                first.map_or("no content within the limit".to_string(), |t| format!(
                    "first content {t:.2}s"
                )),
                if i == 0 {
                    "the video start"
                } else {
                    "the beat start"
                }
            ));
        }
    }
    (findings, firsts)
}

fn report_enabled() -> bool {
    std::env::var("DEAD_AIR_REPORT").is_ok_and(|v| v != "0")
}

fn print_row(story: &str, tone: &str, firsts: &[Option<f64>]) {
    let cells: Vec<String> = firsts
        .iter()
        .map(|f| f.map_or("none".to_string(), |t| format!("{t:.2}")))
        .collect();
    eprintln!("{story:22} {tone:12} {}", cells.join(" "));
}

/// The scene of `story` x `tone` the matrix judges: compiled on the product
/// path, or read from `DEAD_AIR_DIR` (`<story>__<tone>.motion.json`, as the CLI
/// writes it). `None` when the CLI refused to compile it (a validation error
/// on a very short beat has no scene: it is not a dead-air finding).
fn matrix_scene(si: usize, tone: &str) -> Option<MotionProject> {
    let Ok(dir) = std::env::var("DEAD_AIR_DIR") else {
        return Some(compile_story(si, tone, PRODUCT).0);
    };
    let path = PathBuf::from(dir).join(format!("{}__{tone}.motion.json", STORIES[si].0));
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("no scene for {} x {tone}", STORIES[si].0);
        return None;
    };
    Some(MotionProject::from_json(&text).expect("scene"))
}

/// Judge a matrix of compiled scenes: the product path's, or the CLI's scenes
/// in `DEAD_AIR_DIR`.
fn run_matrix(speech_dir: Option<&str>) {
    let mut findings = Vec::new();
    let mut compiled = 0usize;
    for (si, (story, _)) in STORIES.iter().enumerate() {
        for tone in TONES {
            let project = match speech_dir {
                Some(dir) => {
                    let path = format!("{dir}/{story}.speech.json");
                    if !repo().join(&path).exists() {
                        continue;
                    }
                    compile_with(si, tone, PRODUCT, &path).0
                }
                None => match matrix_scene(si, tone) {
                    Some(project) => project,
                    None => continue,
                },
            };
            compiled += 1;
            let label = format!("{story} x {tone}");
            let (f, firsts) = judge(&label, &project);
            if report_enabled() {
                print_row(story, tone, &firsts);
            }
            findings.extend(f);
        }
    }
    assert!(compiled > 0, "nothing was measured");
    assert!(
        findings.is_empty(),
        "{} dead-air finding(s) over {compiled} compiles:\n  {}",
        findings.len(),
        findings.join("\n  ")
    );
}

#[test]
fn the_bench_stories_show_content_in_time_in_every_tone() {
    run_matrix(None);
}

/// The looks that hold a beat's title and hero until the narrator says them.
const HOLD_TONES: [&str; 3] = ["cinematic", "documentary", "studio"];

/// A holding look shows more than the kicker, within `DEAD_AIR_MAX_HOLD_S` of
/// every beat's start: a hero that waits for its word leaves a placeholder at
/// its own scale (a card with its label, a label line), not just the kicker.
/// `DEAD_AIR_REPORT=1` prints the seconds to the first non-kicker content.
#[test]
fn a_holding_look_shows_more_than_its_kicker_in_time() {
    let mut findings = Vec::new();
    for (si, (story, _)) in STORIES.iter().enumerate() {
        for tone in HOLD_TONES {
            let Some(project) = matrix_scene(si, tone) else {
                continue;
            };
            let mut cells = Vec::new();
            for (i, scene) in beat_scenes(&project).into_iter().enumerate() {
                let first = first_content_with(&project, scene, DEAD_AIR_MAX_HOLD_S + 3.0, &|l| {
                    !is_kicker(l)
                });
                cells.push(first.map_or("none".to_string(), |t| format!("{t:.2}")));
                if first.is_none_or(|t| t > DEAD_AIR_MAX_HOLD_S + FRAME_TOLERANCE) {
                    findings.push(format!(
                        "{story} x {tone} beat {}: only the kicker for {} (limit {DEAD_AIR_MAX_HOLD_S}s)",
                        i + 1,
                        first.map_or("the whole beat".to_string(), |t| format!("{t:.2}s")),
                    ));
                }
            }
            if report_enabled() {
                eprintln!("{story:22} {tone:12} {}", cells.join(" "));
            }
        }
    }
    assert!(
        findings.is_empty(),
        "{} beat(s) of a holding look show nothing but a kicker for too long:\n  {}",
        findings.len(),
        findings.join("\n  ")
    );
}

/// The `tight/` speech fixtures (A4-short: sentences pushed to the limit of
/// their beat), when the folder exists.
#[test]
fn the_tight_fixtures_show_content_in_time_too() {
    let dir = "golden/fixtures/variety/tight";
    if !repo().join(dir).is_dir() {
        eprintln!("no {dir}: nothing to check");
        return;
    }
    run_matrix(Some(dir));
}

// ---------------------------------------------------------------------------
// The mechanisms, and what stays as it was
// ---------------------------------------------------------------------------

/// Index of the story in [`STORIES`].
fn story_index(name: &str) -> usize {
    STORIES
        .iter()
        .position(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no story {name}"))
}

/// Duration of each beat's fly-in (the first camera dolly of its scene) and
/// when it starts, in scene-local seconds.
fn fly_ins(project: &MotionProject) -> Vec<(f64, f64)> {
    beat_scenes(project)
        .into_iter()
        .filter_map(|s| {
            s.camera
                .as_ref()?
                .motions
                .iter()
                .find(|m| matches!(m.op, CameraOp::Dolly { .. }))
                .map(|m| (m.start, m.duration))
        })
        .collect()
}

#[test]
fn cinematic_still_flies_in_but_lands_in_time() {
    let si = story_index("sleep_review");
    let (project, _) = compile_story(si, "cinematic", PRODUCT);
    let flights = fly_ins(&project);
    assert_eq!(flights.len(), 5, "every beat flies in: {flights:?}");
    for (i, (start, len)) in flights.iter().enumerate() {
        // On the product path a fly-in lasts 0.45 s, 0.4 s where the limit
        // leaves no more (the old 0.75 s keeps the stage's text soft too long).
        assert!(
            (0.4 - 1e-9..=0.45 + 1e-9).contains(len),
            "beat {} flies in for {len:.2}s",
            i + 1
        );
        // It is still a fly-in the camera lands from: the focus is on the
        // stage, and the text reads, before the dead-air limit.
        assert!(
            start + len <= limit_of(i) + 1e-9,
            "beat {} lands at {:.2}s",
            i + 1,
            start + len
        );
    }
    // The opening camera moves from the first frames; later beats fly in from ENTER.
    assert!(flights[0].0 <= 0.1, "{flights:?}");
}

#[test]
fn a_handoff_bloom_and_wipe_last_at_most_four_tenths_of_a_second() {
    let si = story_index("sleep_review");
    let (project, _) = compile_story(si, "cinematic", PRODUCT);
    let mut blooms = 0;
    for scene in beat_scenes(&project) {
        for p in &scene.post {
            if matches!(p.kind, PostKind::Rays { .. }) {
                blooms += 1;
                assert!(
                    p.duration <= 0.4 + 1e-9,
                    "{}: rays burst of {:.2}s",
                    scene.id,
                    p.duration
                );
            }
        }
    }
    assert!(blooms >= 5, "the cinematic handoffs still bloom: {blooms}");

    // Panel wipes (technical: geometric; playful: kinetic bars): they cover
    // the frame and clear within 0.4 s of the beat start.
    for tone in ["technical", "playful"] {
        let (project, _) = compile_story(si, tone, PRODUCT);
        let mut wipes = 0;
        for scene in beat_scenes(&project) {
            let wipe_ids: Vec<&str> = scene
                .layers
                .iter()
                .map(|l| l.id.as_str())
                .filter(|id| id.contains(".wipe"))
                .collect();
            for m in scene
                .motions
                .iter()
                .filter(|m| wipe_ids.contains(&m.target.as_str()))
            {
                wipes += 1;
                assert!(
                    m.start + m.duration <= 0.4 + 1e-9,
                    "{} x {tone}: the wipe runs to {:.2}s",
                    scene.id,
                    m.start + m.duration
                );
            }
        }
        assert!(wipes >= 4, "{tone} still wipes between beats: {wipes}");
    }
}

#[test]
fn without_variety_the_default_timing_is_untouched() {
    let si = story_index("sleep_review");
    // The late ENTER of a sentence that opens on a function word, the 0.75 s
    // fly-in and the 0.55 s zoom burst are what a compile without --variety
    // always did (policy F2: only the product path changes).
    let (project, _) = compile_story(si, "cinematic", NO_VARIETY);
    let first = beat_scenes(&project)[0];
    let enter = first.lifecycle.expect("lifecycle").enter;
    assert!(enter > 0.9, "ENTER of beat 1 without --variety: {enter}");
    assert!(
        fly_ins(&project)
            .iter()
            .any(|(_, len)| (len - 0.75).abs() < 1e-9),
        "the 0.75 s fly-in is still there without --variety"
    );
    let longest = beat_scenes(&project)
        .iter()
        .flat_map(|s| s.post.iter())
        .filter(|p| matches!(p.kind, PostKind::Rays { .. }))
        .map(|p| p.duration)
        .fold(0.0, f64::max);
    assert!(longest > 0.4, "zoom bursts without --variety: {longest}");
}

/// The sleep_review hero ("60%") is said as "sixty" in the second sentence.
fn sixty_starts(speech: &SpeechMap) -> f64 {
    let sentence = speech
        .sentences
        .iter()
        .find(|s| s.beat == 1)
        .expect("sentence of beat 2");
    speech
        .words_in(sentence)
        .iter()
        .find(|w| w.text.to_lowercase().starts_with("sixty"))
        .map(|w| w.start)
        .expect("\"sixty\" is spoken")
}

#[test]
fn a_value_still_waits_for_its_word() {
    let si = story_index("sleep_review");
    for tone in ["playful", "cinematic", "documentary"] {
        let (project, speech) = compile_story(si, tone, PRODUCT);
        let speech = speech.expect("speech");
        let word = sixty_starts(&speech);
        let scene = beat_scenes(&project)[1];
        let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
        let fps = project.canvas.fps;
        // From the beat start until the word (less the reveal tolerance), no
        // layer that carries the number is readable: only the placeholder
        // (the kicker, the label, the headline) is.
        let mut n = (scene.start_seconds * f64::from(fps)).ceil() as u32;
        while frame_time(fps, n) < word - REVEAL_LEAD_MAX {
            let frame = evaluate_frame(&project, n).expect("evaluate");
            assert!(
                !value_readable(&frame, &limits),
                "{tone}: the number reads at {:.2}s, before \"sixty\" at {word:.2}s",
                frame_time(fps, n)
            );
            n += 1;
        }
    }
}

/// Whether a text layer showing the number (60) is readable this frame.
fn value_readable(frame: &ResolvedFrame<'_>, limits: &Limits) -> bool {
    fn go(layer: &ResolvedLayer<'_>, opacity: f32, blur_sq: f32, limits: &Limits) -> bool {
        let opacity = opacity * layer.opacity.clamp(0.0, 1.0);
        let own = layer.blur.filter(|b| b.is_finite()).unwrap_or(0.0).max(0.0);
        let blur_sq = blur_sq + own * own;
        if let LayerKind::Group { .. } = layer.kind {
            return layer
                .children
                .iter()
                .any(|c| go(c, opacity, blur_sq, limits));
        }
        let LayerKind::Text(style) = layer.kind else {
            return false;
        };
        let text = layer.text.as_deref().unwrap_or(&style.text);
        text.contains("60") && leaf_readable(layer, opacity, blur_sq, limits)
    }
    frame.layers.iter().any(|l| go(l, 1.0, 0.0, limits))
}

#[test]
fn a_beat_with_a_late_voice_still_opens_in_time() {
    // The voice-over starts a second into the video (a slow TTS lead-in): the
    // first beat opens with its kicker on the dead-air limit anyway.
    let si = story_index("sleep_review");
    let mut map = SpeechMap::from_json(&read("golden/fixtures/variety/sleep_review.speech.json"))
        .expect("speech fixture");
    for w in &mut map.words {
        w.start += 0.65;
        w.end += 0.65;
    }
    for s in &mut map.sentences {
        s.start += 0.65;
        s.end += 0.65;
    }
    let intent = CreativeIntent::from_json(&read(STORIES[si].1)).expect("intent");
    let spoken: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    let speech = repair(&map, &spoken).0;
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: Some(story_seed(&intent)),
        speech: Some(speech),
        ..CompileOptions::default()
    };
    for tone in ["playful", "editorial", "documentary", "cinematic"] {
        let project = compile_with_options(
            &intent,
            &style(tone),
            None,
            &AssetLibrary::new(repo().join("assets")),
            &ApproxMeasure,
            &AssetManifest::empty(),
            None,
            &opts,
        )
        .expect("compile");
        let (findings, firsts) = judge(&format!("late voice x {tone}"), &project);
        assert!(findings.is_empty(), "{firsts:?}\n{}", findings.join("\n"));
    }
}

// ---------------------------------------------------------------------------
// The placeholder of a hero that waits for its word
// ---------------------------------------------------------------------------

/// The text of the layer `id` anywhere in a scene.
fn text_of(scene: &Scene, id: &str) -> Option<String> {
    fn find(layers: &[Layer], id: &str) -> Option<String> {
        for l in layers {
            if l.id == id {
                return match &l.kind {
                    LayerKind::Text(t) => Some(t.text.clone()),
                    _ => None,
                };
            }
            if let LayerKind::Group { children } = &l.kind {
                if let Some(t) = find(children, id) {
                    return Some(t);
                }
            }
        }
        None
    }
    find(&scene.layers, id)
}

/// Whether a text leaf of the frame that is not a kicker carries `needle`
/// (lower case) and is readable.
fn label_readable(frame: &ResolvedFrame<'_>, limits: &Limits, beat: &str, needle: &str) -> bool {
    shows_content(frame, limits, beat, &|l| {
        let LayerKind::Text(style) = l.kind else {
            return false;
        };
        let text = l.text.as_deref().unwrap_or(&style.text);
        !is_kicker(l) && text.to_lowercase().contains(needle)
    })
}

#[test]
fn a_waiting_hero_leaves_a_placeholder_at_its_own_scale() {
    // sleep_review beat 2: "60%" (meaning "faster clearance") is said as
    // "sixty percent" about three seconds into the beat, and the holding looks
    // cue the number and the title to their words. From half a second after
    // ENTER until the narrator gets there the beat shows more than its kicker:
    // the label "faster clearance" in a leaf of its own (the documentary card
    // that will hold the value, the cinematic line under the hero), while no
    // layer with the number is readable. The studio look sets the narration
    // itself as kinetic words and has no label: it shows its words instead.
    let si = story_index("sleep_review");
    for tone in HOLD_TONES {
        let (project, speech) = compile_story(si, tone, PRODUCT);
        let word = sixty_starts(&speech.expect("speech"));
        let scene = beat_scenes(&project)[1];
        let enter = scene.lifecycle.expect("lifecycle").enter;
        let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
        let fps = project.canvas.fps;
        let from = scene.start_seconds + enter + 0.5;
        let until = word - REVEAL_LEAD_MAX;
        assert!(
            until - from > 1.0,
            "{tone}: the window is {from:.2}..{until:.2}"
        );
        let mut n = (from * f64::from(fps)).ceil() as u32;
        let mut frames = 0;
        while frame_time(fps, n) < until {
            let frame = evaluate_frame(&project, n).expect("evaluate");
            let at = frame_time(fps, n);
            assert!(
                !value_readable(&frame, &limits),
                "{tone}: the number reads at {at:.2}s, before \"sixty\" at {word:.2}s"
            );
            let placeholder = if tone == "studio" {
                shows_content(&frame, &limits, &scene.id, &|l| !is_kicker(l))
            } else {
                label_readable(&frame, &limits, &scene.id, "faster clearance")
            };
            assert!(
                placeholder,
                "{tone}: nothing but the kicker at {at:.2}s ({:.2}s after ENTER)",
                at - scene.start_seconds - enter
            );
            n += 1;
            frames += 1;
        }
        assert!(frames > 30, "{tone}: {frames} frames checked");
    }
    // The kicker keeps saying "the result" (the label is not said twice), and
    // without --variety nothing of this exists.
    let (project, _) = compile_story(si, "documentary", PRODUCT);
    let kicker = text_of(beat_scenes(&project)[1], "b2.kicker").expect("a kicker");
    assert_eq!(kicker.to_lowercase(), "the result");
    let (project, _) = compile_story(si, "cinematic", NO_VARIETY);
    assert!(text_of(beat_scenes(&project)[1], "b2.hero_label").is_none());
}

/// A two-beat story whose first (reveal) beat is a figure with the meaning
/// `meaning`, spoken with hand-placed word times; compiled in the cinematic
/// look on the product path.
fn synthetic_reveal(meaning: &str) -> MotionProject {
    use motion_core::speech::{SpeechSentence, SpeechWord, SPEECH_VERSION};
    let intent = CreativeIntent::from_json(
        &serde_json::json!({
            "version": "0.2",
            "title": "label",
            "format": "vertical",
            "beats": [
                {
                    "purpose": "reveal",
                    "statement": "Waste clears much faster",
                    "narration": "During deep sleep waste clears sixty percent faster.",
                    "primary": { "kind": "number", "value": "60%", "meaning": meaning },
                    "energy": "building"
                },
                {
                    "purpose": "emphasize",
                    "statement": "Sleep is not lazy",
                    "primary": { "kind": "phrase", "value": "Maintenance", "meaning": "sleep" },
                    "energy": "calm"
                }
            ]
        })
        .to_string(),
    )
    .expect("intent");
    let line = |beat: usize, from: f64, words: &[&str]| -> (SpeechSentence, Vec<SpeechWord>) {
        let list: Vec<SpeechWord> = words
            .iter()
            .enumerate()
            .map(|(i, w)| SpeechWord {
                text: w.to_string(),
                start: from + 0.4 * i as f64,
                end: from + 0.4 * i as f64 + 0.35,
                confidence: 1.0,
            })
            .collect();
        let end = list.last().map_or(from, |w| w.end);
        (
            SpeechSentence {
                beat,
                start: from,
                end,
            },
            list,
        )
    };
    let (s0, w0) = line(
        0,
        0.35,
        &[
            "During", "deep", "sleep", "waste", "clears", "sixty", "percent", "faster",
        ],
    );
    let (s1, w1) = line(1, 4.2, &["Sleep", "is", "not", "lazy"]);
    let map = SpeechMap {
        version: SPEECH_VERSION.to_string(),
        audio: "v.wav".into(),
        sample_rate: 48_000,
        duration: 6.5,
        provider: "fixture".into(),
        model: "fixture/model".into(),
        voice: "fx".into(),
        words: w0.into_iter().chain(w1).collect(),
        sentences: vec![s0, s1],
        recognised: Vec::new(),
        alignment: None,
    };
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: Some(7),
        speech: Some(map),
        ..CompileOptions::default()
    };
    compile_with_options(
        &intent,
        &style("cinematic"),
        None,
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

#[test]
fn a_label_that_would_give_the_value_away_is_not_shown() {
    // A meaning that is a label becomes the line under the waiting figure ...
    let project = synthetic_reveal("faster clearance");
    let scene = beat_scenes(&project)[0];
    let label = text_of(scene, "b1.hero_label").expect("a label under the hero");
    assert_eq!(label.to_lowercase(), "faster clearance");
    assert_eq!(
        text_of(scene, "b1.kicker")
            .expect("a kicker")
            .to_lowercase(),
        "the result"
    );
    // ... one that repeats the figure ("60% faster") never is: the number
    // waits for its word with nothing of it on screen.
    let project = synthetic_reveal("60% faster");
    let scene = beat_scenes(&project)[0];
    assert!(text_of(scene, "b1.hero_label").is_none());
    let (findings, _) = judge("label", &project);
    assert!(findings.is_empty(), "{}", findings.join("\n"));
}
