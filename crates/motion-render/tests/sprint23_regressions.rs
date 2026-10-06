//! (0.23 W0) Regression tests for the four reproduced defects of plan §0
//! (`docs/plans/SPRINT_0_23_VARIETY_QA_AUDIO.md`).
//!
//! Each test compiles a review story the way the product path does
//! (`--art auto --variety <seed> --speech`, with the committed offline speech
//! fixture under `golden/fixtures/variety/`) and measures the defect. Every
//! one is `#[ignore]`d with its reason because it FAILS on 1a4664e; the task
//! named in the reason fixes the defect and removes the `ignore`. Run them
//! with `cargo test -p motion-render --test sprint23_regressions -- --ignored`;
//! each failure message states the measured value.
//!
//! What is judged, shared by the tests that look at pixels or readable
//! layers: a layer is judged only when it is not decorative
//! (`layout_qa::is_decorative`) and does not belong to the `backdrop` or
//! `captions` scenes; "readable" is the speech QA's own rule
//! (`reveal_qa::readable_leaves`).

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::captions::CAPTION_SCENE_ID;
use motion_core::compiler::speech_plan::anchor_time;
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{compile_with_report, AssetLibrary, CompileOptions, FontSet};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::is_decorative;
use motion_core::scene::{Color, LayerKind, MotionOp, MotionProject, Scene};
use motion_core::speech::{repair, SpeechMap};
use motion_core::style::StyleProfile;
use motion_core::timeline::{
    evaluate_frame, format_count, frame_time, ResolvedFrame, ResolvedLayer,
};
use motion_render::reveal_qa::{readable_leaves, Limits};
use motion_render::text::TextEngine;
use motion_render::{CpuRenderer, FontMeasure, Renderer};
use resvg::tiny_skia::Pixmap;

// ---------------------------------------------------------------------------
// Shared helpers (test-local; no public API)
// ---------------------------------------------------------------------------

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

/// A review story with its committed offline speech map (`say`, onset times),
/// repaired against the spoken lines exactly as `compile --speech` does.
struct Story {
    intent: CreativeIntent,
    json: serde_json::Value,
    speech: SpeechMap,
}

fn story(name: &str) -> Story {
    let text = read_repo(&format!(
        "docs/plans/sprint_0_23/stories/{name}.intent.json"
    ));
    let intent = CreativeIntent::from_json(&text).expect("intent");
    let json: serde_json::Value = serde_json::from_str(&text).expect("intent json");
    let map = SpeechMap::from_json(&read_repo(&format!(
        "golden/fixtures/variety/{name}.speech.json"
    )))
    .expect("speech fixture");
    let spoken: Vec<String> = intent
        .beats
        .iter()
        .map(|b| display_text(b, true).spoken)
        .collect();
    let speech = repair(&map, &spoken).0;
    Story {
        intent,
        json,
        speech,
    }
}

/// A style file `{"tone": "<tone>"}`, as the bench writes it.
fn tone_style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!("{{\"tone\": \"{tone}\"}}")).expect("style")
}

/// Compile like `motion-engine compile --art auto --variety <seed> --speech`
/// (the look's fonts measure the text). `variety: None` = the story's own
/// `--variety auto` seed; `canvas` shrinks the canvas for pixel checks.
fn compile_product(
    s: &Story,
    tone: &str,
    variety: Option<u64>,
    canvas: Option<(u32, u32)>,
) -> MotionProject {
    let assets = repo().join("assets");
    let style = tone_style(tone);
    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        variety: Some(variety.unwrap_or_else(|| story_seed(&s.intent))),
        speech: Some(s.speech.clone()),
        canvas,
        ..CompileOptions::default()
    };
    compile_with_report(
        &s.intent,
        &style,
        None,
        &AssetLibrary::new(&assets),
        &measure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
    .0
}

/// The beat scenes (`beat_N`), in order.
fn beat_scenes(p: &MotionProject) -> Vec<&Scene> {
    p.scenes
        .iter()
        .filter(|s| s.id.starts_with("beat_"))
        .collect()
}

/// The resolved layer at `chain` (ids from a top-level layer down).
fn find_layer<'f, 'a>(
    layers: &'f [ResolvedLayer<'a>],
    chain: &[&str],
) -> Option<&'f ResolvedLayer<'a>> {
    let (first, rest) = chain.split_first()?;
    let layer = layers.iter().find(|l| l.id == *first)?;
    if rest.is_empty() {
        Some(layer)
    } else {
        find_layer(&layer.children, rest)
    }
}

/// A leaf that counts as content: readable, not decorative, not in the
/// backdrop or caption scenes.
fn is_content(scene: Option<&str>, id: &str, kind: &LayerKind) -> bool {
    scene != Some("backdrop") && scene != Some(CAPTION_SCENE_ID) && !is_decorative(id, kind)
}

/// Share (0..=1) of a layer's clip window that its content fills this frame.
/// A `clip_reveal` slides the content through a fixed window: until it has
/// moved in, nothing shows, though `readable_leaves` (opacity, blur, box)
/// already counts the layer as readable.
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
    // The content's offset inside the box (box space).
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

/// A content layer is on screen once at least this share of it shows.
const ON_SCREEN_SHARE: f32 = 0.5;

/// The text a text layer shows this frame (a counter's own text included).
fn shown_text(layer: &ResolvedLayer<'_>) -> Option<String> {
    match layer.kind {
        LayerKind::Text(style) => Some(layer.text.clone().unwrap_or_else(|| style.text.clone())),
        _ => None,
    }
}

/// Frame nearest `t` seconds, within the project.
fn frame_at(p: &MotionProject, t: f64) -> u32 {
    let last = p.frame_count().saturating_sub(1);
    ((t * f64::from(p.canvas.fps)).round() as u32).min(last)
}

// ---------------------------------------------------------------------------
// 1. Local contrast of text against what is drawn around it
// ---------------------------------------------------------------------------

/// Largest canvas width the pixel test renders (px).
const PIXEL_TEST_WIDTH: u32 = 540;
/// The ring around a text's ink box: from RING_NEAR to RING_FAR px outside.
const RING_NEAR: i32 = 4;
const RING_FAR: i32 = 12;
/// WCAG contrast needed for display text (>= 24 px x u) and for small text.
const CONTRAST_DISPLAY: f32 = 3.0;
const CONTRAST_SMALL: f32 = 4.5;

fn srgb_to_linear(c: u8) -> f32 {
    let c = f32::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: [u8; 3]) -> f32 {
    0.2126 * srgb_to_linear(c[0]) + 0.7152 * srgb_to_linear(c[1]) + 0.0722 * srgb_to_linear(c[2])
}

/// WCAG contrast ratio, in `1.0..=21.0`.
fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Per-channel median colour of the ring `near..far` px outside `rect`
/// (`[x0, y0, x1, y1]`), clipped to the image. `None` when the ring is empty.
fn ring_median(pm: &Pixmap, rect: [f32; 4], near: i32, far: i32) -> Option<[u8; 3]> {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let grow = |d: i32| {
        [
            rect[0].floor() as i32 - d,
            rect[1].floor() as i32 - d,
            rect[2].ceil() as i32 + d,
            rect[3].ceil() as i32 + d,
        ]
    };
    let (inner, outer) = (grow(near), grow(far));
    let mut chans: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for y in outer[1].max(0)..outer[3].min(h) {
        for x in outer[0].max(0)..outer[2].min(w) {
            let inside = x >= inner[0] && x < inner[2] && y >= inner[1] && y < inner[3];
            if inside {
                continue;
            }
            let i = ((y * w + x) * 4) as usize;
            let px = &pm.data()[i..i + 3];
            for (c, v) in chans.iter_mut().zip(px) {
                c.push(*v);
            }
        }
    }
    if chans[0].is_empty() {
        return None;
    }
    let mut out = [0u8; 3];
    for (o, c) in out.iter_mut().zip(chans.iter_mut()) {
        c.sort_unstable();
        *o = c[c.len() / 2];
    }
    Some(out)
}

/// The renderer's text engine with the project's own fonts, and the family
/// of each font role.
fn text_engine(p: &MotionProject) -> (TextEngine, std::collections::BTreeMap<String, String>) {
    let assets = repo().join("assets");
    let mut engine = TextEngine::new();
    let mut by_asset = std::collections::BTreeMap::new();
    for a in p
        .assets
        .iter()
        .filter(|a| a.kind == motion_core::scene::AssetKind::Font)
    {
        let family = engine.load(&assets.join(&a.path)).expect("load font");
        by_asset.insert(a.id.clone(), family);
    }
    (engine, by_asset)
}

/// Canvas-space bounds `[x0, y0, x1, y1]` of a text layer's ink (the outline
/// of its glyphs), through the layer's transform (or perspective homography).
fn ink_box(
    engine: &mut TextEngine,
    family: &str,
    layer: &ResolvedLayer<'_>,
    shown: &str,
) -> Option<[f32; 4]> {
    let LayerKind::Text(style) = layer.kind else {
        return None;
    };
    let mut style = style.clone();
    style.text = shown.to_string();
    let b = engine.outline(family, &style, layer.width)?.bounds();
    let mut out = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for (x, y) in [
        (b.left(), b.top()),
        (b.right(), b.top()),
        (b.right(), b.bottom()),
        (b.left(), b.bottom()),
    ] {
        let (cx, cy) = match &layer.projective {
            Some(m) => {
                let d = m[6] * x + m[7] * y + m[8];
                if !(d.is_finite() && d > 1e-6) {
                    return None;
                }
                (
                    (m[0] * x + m[1] * y + m[2]) / d,
                    (m[3] * x + m[4] * y + m[5]) / d,
                )
            }
            None => layer.transform.apply(x, y),
        };
        out = [
            out[0].min(cx),
            out[1].min(cy),
            out[2].max(cx),
            out[3].max(cy),
        ];
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// Text colour as drawn: the layer colour moved toward its tint, if any.
fn drawn_color(layer: &ResolvedLayer<'_>, c: Color) -> [u8; 3] {
    let mut rgb = [c.r, c.g, c.b];
    if let Some(t) = layer.tint {
        let a = t.amount.clamp(0.0, 1.0);
        let to = [t.color.r, t.color.g, t.color.b];
        for (v, to) in rgb.iter_mut().zip(to) {
            *v = (f32::from(*v) + (f32::from(to) - f32::from(*v)) * a).round() as u8;
        }
    }
    rgb
}

/// Every text layer readable at READ whose contrast with the ring around its
/// ink box is below the WCAG limit, as one line each.
fn local_contrast_failures(project: &MotionProject) -> Vec<String> {
    assert!(
        project.canvas.width <= PIXEL_TEST_WIDTH,
        "pixel test canvas is {} px wide",
        project.canvas.width
    );
    let assets = repo().join("assets");
    let renderer = CpuRenderer::new(project, &assets).expect("renderer");
    let (mut engine, by_asset) = text_engine(project);
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let u = project.canvas.width.min(project.canvas.height) as f32 / 1080.0;
    let mut failures = Vec::new();
    for scene in beat_scenes(project) {
        let Some(lc) = scene.lifecycle else { continue };
        let n = frame_at(project, scene.start_seconds + lc.read);
        let resolved: ResolvedFrame<'_> = evaluate_frame(project, n).expect("evaluate");
        let pm = renderer.render(&resolved).expect("render");
        for leaf in readable_leaves(&resolved, &limits) {
            if leaf.scene != Some(scene.id.as_str()) {
                continue;
            }
            let Some(layer) = find_layer(&resolved.layers, &leaf.chain) else {
                continue;
            };
            let LayerKind::Text(style) = layer.kind else {
                continue;
            };
            if !is_content(leaf.scene, layer.id, layer.kind) {
                continue;
            }
            let Some(shown) = shown_text(layer).filter(|t| !t.trim().is_empty()) else {
                continue;
            };
            let family = project
                .theme
                .fonts
                .get(&style.font_role)
                .and_then(|asset| by_asset.get(asset));
            let Some(family) = family else { continue };
            let Some(ink) = ink_box(&mut engine, family, layer, &shown) else {
                continue;
            };
            let Some(ring) = ring_median(&pm, ink, RING_NEAR, RING_FAR) else {
                continue;
            };
            let ink_rgb = drawn_color(layer, style.color);
            let scale = (layer.transform.a * layer.transform.d
                - layer.transform.b * layer.transform.c)
                .abs()
                .sqrt();
            let size = style.font_size * scale;
            let need = if size >= 24.0 * u {
                CONTRAST_DISPLAY
            } else {
                CONTRAST_SMALL
            };
            let ratio = contrast_ratio(ink_rgb, ring);
            if ratio < need {
                failures.push(format!(
                    "{} {} {:?}: text {} on ring {} = {:.2}:1 < {:.1}:1 ({:.0}px at READ {:.2}s)",
                    scene.id,
                    layer.id,
                    shown.chars().take(24).collect::<String>(),
                    hex(ink_rgb),
                    hex(ring),
                    ratio,
                    need,
                    size,
                    lc.read,
                ));
            }
        }
    }
    failures
}

#[test]
fn local_contrast_sleep_playful_variety1() {
    // `--variety 1` puts a print field behind the black beat-1 headline.
    let s = story("sleep_review");
    let project = compile_product(
        &s,
        "playful",
        Some(1),
        Some((PIXEL_TEST_WIDTH, PIXEL_TEST_WIDTH * 16 / 9)),
    );
    let failures = local_contrast_failures(&project);
    assert!(
        failures.is_empty(),
        "sleep_review x playful x variety 1: {} text layer(s) below the local contrast limit \
         (ring {RING_NEAR}-{RING_FAR} px outside the ink box):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// 2. The cinematic comparison shows both of its values
// ---------------------------------------------------------------------------

/// Digits of every number written in `text` (`"$240,000 / yr"` ->
/// `["240000"]`), separators dropped.
fn digit_runs(text: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            cur.push(c);
        } else if (c == ',' || c == '.')
            && !cur.is_empty()
            && chars.peek().is_some_and(|n| n.is_ascii_digit())
        {
            // a separator inside a number
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs
}

fn digits(value: &str) -> String {
    value.chars().filter(char::is_ascii_digit).collect()
}

/// The digit runs of every content text layer of `scene` that is readable at
/// some frame from its ENTER to its ANTICIPATE (every frame sampled).
fn digits_shown(project: &MotionProject, scene: &Scene) -> Vec<String> {
    let lc = scene.lifecycle.expect("beat lifecycle");
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let first = frame_at(project, scene.start_seconds + lc.enter);
    let last = frame_at(project, scene.start_seconds + lc.anticipate);
    let mut seen = std::collections::BTreeSet::new();
    for n in first..=last {
        let resolved = evaluate_frame(project, n).expect("evaluate");
        for leaf in readable_leaves(&resolved, &limits) {
            if leaf.scene != Some(scene.id.as_str()) {
                continue;
            }
            let Some(layer) = find_layer(&resolved.layers, &leaf.chain) else {
                continue;
            };
            if !is_content(leaf.scene, layer.id, layer.kind)
                || revealed_share(layer) < ON_SCREEN_SHARE
            {
                continue;
            }
            if let Some(t) = shown_text(layer) {
                seen.extend(digit_runs(&t));
            }
        }
    }
    seen.into_iter().collect()
}

#[test]
fn cinematic_compare_shows_both_values() {
    let mut missing = Vec::new();
    for name in ["sleep_review", "money_review"] {
        let s = story(name);
        let beat3 = &s.json["beats"][2];
        let values: Vec<String> = ["primary", "secondary"]
            .iter()
            .filter_map(|k| beat3[k]["value"].as_str().map(str::to_string))
            .collect();
        assert_eq!(values.len(), 2, "{name}: beat 3 compares two number values");
        let project = compile_product(&s, "cinematic", None, None);
        let scenes = beat_scenes(&project);
        let scene = scenes.get(2).expect("beat 3");
        let shown = digits_shown(&project, scene);
        for v in &values {
            if !shown.contains(&digits(v)) {
                missing.push(format!(
                    "{name} x cinematic beat 3: value {v:?} (digits {}) is in no readable text layer \
                     between ENTER and ANTICIPATE; numbers shown there: {shown:?}",
                    digits(v)
                ));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} value(s) missing:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// 3. Dead air: the opening and the handoffs show content soon
// ---------------------------------------------------------------------------

/// The opening must show content within this many seconds of the video start.
const FIRST_CONTENT_MAX: f64 = 0.5;
/// Every beat must show content within this many seconds of its start.
const BEAT_CONTENT_MAX: f64 = 1.2;

/// Seconds after the beat starts until a readable content leaf of the beat
/// (or a shared element) first shows; `None` when none ever does.
fn first_content_after_start(project: &MotionProject, scene: &Scene) -> Option<f64> {
    let fps = project.canvas.fps;
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let end = frame_at(project, scene.end_seconds());
    let mut n = (scene.start_seconds * f64::from(fps)).floor() as u32;
    while n <= end {
        if frame_time(fps, n) >= scene.start_seconds {
            let resolved = evaluate_frame(project, n).expect("evaluate");
            for leaf in readable_leaves(&resolved, &limits) {
                if leaf.scene.is_some_and(|s| s != scene.id) {
                    continue;
                }
                let Some(layer) = find_layer(&resolved.layers, &leaf.chain) else {
                    continue;
                };
                if is_content(leaf.scene, layer.id, layer.kind)
                    && revealed_share(layer) >= ON_SCREEN_SHARE
                {
                    return Some(frame_time(fps, n) - scene.start_seconds);
                }
            }
        }
        n += 1;
    }
    None
}

#[test]
fn opening_and_handoffs_have_no_dead_air() {
    let s = story("sleep_review");
    let mut report = Vec::new();
    for tone in ["playful", "cinematic"] {
        let project = compile_product(&s, tone, None, None);
        let mut firsts = Vec::new();
        for (i, scene) in beat_scenes(&project).into_iter().enumerate() {
            let first = first_content_after_start(&project, scene);
            firsts.push(first);
            let limit = if i == 0 {
                FIRST_CONTENT_MAX
            } else {
                BEAT_CONTENT_MAX
            };
            match first {
                Some(t) if t <= limit => {}
                Some(t) => report.push(format!(
                    "sleep_review x {tone} beat {}: first readable content {t:.2}s after {} (limit {limit:.1}s)",
                    i + 1,
                    if i == 0 { "the video start" } else { "the beat start" },
                )),
                None => report.push(format!(
                    "sleep_review x {tone} beat {}: no readable content in the whole beat",
                    i + 1
                )),
            }
        }
        eprintln!("{tone}: seconds to first content per beat {firsts:.2?}");
    }
    assert!(
        report.is_empty(),
        "{} dead-air finding(s):\n  {}",
        report.len(),
        report.join("\n  ")
    );
}

// ---------------------------------------------------------------------------
// 4. A counter settles on its value by READ and holds it
// ---------------------------------------------------------------------------

/// A counting number must show its final value within this many seconds of
/// its anchor word starting...
const COUNT_SETTLE_MAX: f64 = 0.8;
/// ...and hold it this long before the beat's exit (ANTICIPATE).
const COUNT_HOLD_MIN: f64 = 1.0;

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

/// The counting text layers of a beat scene: (layer id, final text).
fn counters(scene: &Scene) -> Vec<(String, String)> {
    scene
        .motions
        .iter()
        .filter_map(|m| match &m.op {
            MotionOp::Count {
                to,
                decimals,
                grouping,
                prefix,
                suffix,
                ..
            } => Some((
                m.target.clone(),
                format_count(*to, *decimals, *grouping, prefix, suffix),
            )),
            _ => None,
        })
        .collect()
}

/// The text of the layer with `id` at `frame`.
fn text_of(project: &MotionProject, frame: u32, id: &str) -> Option<String> {
    fn find(layers: &[ResolvedLayer<'_>], id: &str) -> Option<String> {
        for l in layers {
            if l.id == id {
                return shown_text(l);
            }
            if let Some(t) = find(&l.children, id) {
                return Some(t);
            }
        }
        None
    }
    let resolved = evaluate_frame(project, frame).expect("evaluate");
    find(&resolved.layers, id)
}

#[test]
fn counter_settles_by_read() {
    let s = story("sleep_review");
    let mut report = Vec::new();
    let mut checked = 0;
    for tone in TONES {
        let project = compile_product(&s, tone, None, None);
        let fps = f64::from(project.canvas.fps);
        let look = project
            .project
            .art
            .as_ref()
            .map(|a| a.look.clone())
            .unwrap_or_default();
        let scenes = beat_scenes(&project);
        let scene = scenes.get(1).expect("beat 2 (60%)");
        let lc = scene.lifecycle.expect("lifecycle");
        // "sixty percent" as the narrator says it, scene-local.
        let spoken: Vec<(String, f64)> = s
            .speech
            .words_in(&s.speech.sentences[1])
            .iter()
            .map(|w| (w.text.clone(), w.start - scene.start_seconds))
            .collect();
        let word = anchor_time(&spoken, &["60%"]).expect("\"60%\" is said");
        for (id, final_text) in counters(scene) {
            checked += 1;
            let at_read = text_of(
                &project,
                frame_at(&project, scene.start_seconds + lc.read),
                &id,
            )
            .unwrap_or_default();
            // First frame, from the beat's start, showing the final value.
            let first = (scene.start_seconds * fps).ceil() as u32;
            let last = frame_at(&project, scene.end_seconds());
            let settled = (first..=last)
                .find(|&n| text_of(&project, n, &id).is_some_and(|t| t == final_text));
            let tag = format!("sleep_review x {tone} ({look}) {id}");
            match settled {
                None => report.push(format!(
                    "{tag}: never shows {final_text:?} (READ shows {at_read:?})"
                )),
                Some(n) => {
                    let t = frame_time(project.canvas.fps, n) - scene.start_seconds;
                    let after_word = t - word;
                    let hold = lc.anticipate - t;
                    if after_word > COUNT_SETTLE_MAX {
                        report.push(format!(
                            "{tag}: shows {final_text:?} {after_word:.2}s after \"sixty\" starts \
                             (limit {COUNT_SETTLE_MAX}s); READ ({:.2}s) shows {at_read:?}",
                            lc.read
                        ));
                    } else if hold < COUNT_HOLD_MIN {
                        report.push(format!(
                            "{tag}: shows {final_text:?} but holds it only {hold:.2}s before the \
                             exit (limit {COUNT_HOLD_MIN}s); READ ({:.2}s) shows {at_read:?}",
                            lc.read
                        ));
                    }
                }
            }
        }
    }
    assert!(checked > 0, "no look counts the 60% on beat 2");
    assert!(
        report.is_empty(),
        "{} of {checked} counter(s) unsettled:\n  {}",
        report.len(),
        report.join("\n  ")
    );
}
