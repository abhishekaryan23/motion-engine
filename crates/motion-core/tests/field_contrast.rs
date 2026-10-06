//! (0.23 A4) Text never sits unreadable on a print field.
//!
//! Under a print-field look the backdrop's fields used to be placed and
//! coloured without knowing where the beat's text is: `--variety 1` put a
//! #111111 field under #111111 headline and kicker text. The compiler now
//! composes the backdrop after the beats and keeps every field either clear
//! of the beats' text boxes or in a colour that text can be read on.
//!
//! The check here is structural and independent of the compiler's own
//! bookkeeping: it compiles the review stories the way the product path does
//! (speech-led, `--art`, `--variety <seed>`), resolves each beat at its READ
//! time through `timeline::evaluate_frame`, and compares the boxes of the
//! readable text layers with the visible `backdrop.field*` layers.

use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::checks::{DISPLAY_TEXT_PX, TEXT_CONTRAST_BODY, TEXT_CONTRAST_DISPLAY};
use motion_core::compiler::art_direction::{
    ground_under_plate, look_defaults, palettes, ArtMode, Look,
};
use motion_core::compiler::taste_rules::display_text;
use motion_core::compiler::{compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions};
use motion_core::intent::CreativeIntent;
use motion_core::layout_qa::is_decorative;
use motion_core::scene::{LayerKind, MotionOp, MotionProject};
use motion_core::speech::{repair, SpeechMap};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// Penetration (px) below which a text box and a field do not overlap.
const EPS: f32 = 0.5;
const SEEDS: std::ops::RangeInclusive<u64> = 0..=7;

const STORIES: [(&str, &str); 3] = [
    (
        "sleep_review",
        "docs/plans/sprint_0_23/stories/sleep_review.intent.json",
    ),
    (
        "money_review",
        "docs/plans/sprint_0_23/stories/money_review.intent.json",
    ),
    ("ai_learns", "examples/topics/ai_learns.intent.json"),
];

struct Story {
    name: &'static str,
    intent: CreativeIntent,
    speech: SpeechMap,
}

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(ROOT).join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn stories() -> Vec<Story> {
    STORIES
        .iter()
        .map(|(name, path)| {
            let intent = CreativeIntent::from_json(&read(path)).expect("intent");
            let map = SpeechMap::from_json(&read(&format!(
                "golden/fixtures/variety/{name}.speech.json"
            )))
            .expect("speech fixture");
            let spoken: Vec<String> = intent
                .beats
                .iter()
                .map(|b| display_text(b, true).spoken)
                .collect();
            Story {
                name,
                intent,
                speech: repair(&map, &spoken).0,
            }
        })
        .collect()
}

/// Compile like `compile --style '{"tone":"playful"}' --speech <fixture>
/// [--art <look>] --variety <seed>`.
fn compile(s: &Story, look: Option<Look>, seed: u64) -> MotionProject {
    compile_with(s, look, Some(seed))
}

/// [`compile`] with or without variety.
fn compile_with(s: &Story, look: Option<Look>, variety: Option<u64>) -> MotionProject {
    let style: StyleProfile = serde_json::from_str(r#"{"tone": "playful"}"#).expect("style");
    let opts = CompileOptions {
        art: look.map(ArtMode::Force),
        variety,
        speech: Some(s.speech.clone()),
        ..CompileOptions::default()
    };
    compile_with_options(
        &s.intent,
        &style,
        None,
        &AssetLibrary::new(Path::new(ROOT).join("assets")),
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &opts,
    )
    .expect("compile")
}

/// The looks that draw print fields under tone `playful`: no look at all, and
/// every look with field colours in its palettes that is not a genre look (a
/// genre look draws a flat ground).
fn looks_with_fields() -> Vec<Option<Look>> {
    let mut out = vec![None];
    for look in Look::ALL {
        let has_fields = palettes(look).iter().any(|p| !p.fields.is_empty());
        if has_fields && look_defaults(look).fx.grammar.is_none() {
            out.push(Some(look));
        }
    }
    out
}

fn linear(c: u8) -> f64 {
    let v = f64::from(c) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(c: [u8; 3]) -> f64 {
    0.2126 * linear(c[0]) + 0.7152 * linear(c[1]) + 0.0722 * linear(c[2])
}

fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

/// Axis-aligned canvas box `[x0, y0, x1, y1]` of a resolved layer.
fn canvas_box(l: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    let mut out = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for (x, y) in [
        (0.0, 0.0),
        (l.width, 0.0),
        (l.width, l.height),
        (0.0, l.height),
    ] {
        let (cx, cy) = match &l.projective {
            Some(m) => {
                let d = m[6] * x + m[7] * y + m[8];
                if d <= 1e-6 {
                    return None;
                }
                (
                    (m[0] * x + m[1] * y + m[2]) / d,
                    (m[3] * x + m[4] * y + m[5]) / d,
                )
            }
            None => l.transform.apply(x, y),
        };
        out = [
            out[0].min(cx),
            out[1].min(cy),
            out[2].max(cx),
            out[3].max(cy),
        ];
    }
    Some(out)
}

struct Field {
    id: String,
    circle: bool,
    rect: [f32; 4],
    fill: [u8; 3],
}

struct Text {
    id: String,
    shown: String,
    rect: [f32; 4],
    ink: [u8; 3],
    need: f64,
}

fn fields_of(layers: &[ResolvedLayer<'_>]) -> Vec<Field> {
    layers
        .iter()
        .filter(|l| l.id.starts_with("backdrop.field") && l.opacity >= 0.05)
        .filter_map(|l| {
            let (circle, fill) = match l.kind {
                LayerKind::Rectangle { fill, .. } => (false, fill),
                LayerKind::RoundedRectangle { fill, .. } => (true, fill),
                _ => return None,
            };
            Some(Field {
                id: l.id.to_string(),
                circle,
                rect: canvas_box(l)?,
                fill: [fill.r, fill.g, fill.b],
            })
        })
        .collect()
}

/// Readable, non-decorative text layers of beat scene `scene`.
fn texts_of(layers: &[ResolvedLayer<'_>], scene: &str, u: f32, out: &mut Vec<Text>) {
    for l in layers {
        if l.scene == Some(scene) {
            if let LayerKind::Text(style) = l.kind {
                let shown = l.text.as_deref().unwrap_or(style.text.as_str());
                if l.opacity >= 0.05 && !shown.trim().is_empty() && !is_decorative(l.id, l.kind) {
                    if let Some(rect) = canvas_box(l) {
                        let t = l.transform;
                        let size = style.font_size * (t.a * t.d - t.b * t.c).abs().sqrt();
                        let mut ink = [style.color.r, style.color.g, style.color.b];
                        if let Some(tint) = l.tint {
                            let a = tint.amount.clamp(0.0, 1.0);
                            let to = [tint.color.r, tint.color.g, tint.color.b];
                            for (v, to) in ink.iter_mut().zip(to) {
                                *v = (f32::from(*v) + (f32::from(to) - f32::from(*v)) * a).round()
                                    as u8;
                            }
                        }
                        out.push(Text {
                            id: l.id.to_string(),
                            shown: shown.chars().take(24).collect(),
                            rect,
                            ink,
                            need: if size >= DISPLAY_TEXT_PX * u {
                                TEXT_CONTRAST_DISPLAY
                            } else {
                                TEXT_CONTRAST_BODY
                            },
                        });
                    }
                }
            }
        }
        texts_of(&l.children, scene, u, out);
    }
}

/// Whether the field overlaps the text box (the circle exactly, not by its box).
fn overlaps(f: &Field, t: &[f32; 4]) -> bool {
    let r = &f.rect;
    if f.circle {
        let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
        let radius = (r[2] - r[0]).min(r[3] - r[1]) / 2.0;
        let (nx, ny) = (cx.clamp(t[0], t[2]), cy.clamp(t[1], t[3]));
        ((cx - nx).powi(2) + (cy - ny).powi(2)).sqrt() < radius - EPS
    } else {
        r[2].min(t[2]) - r[0].max(t[0]) > EPS && r[3].min(t[3]) - r[1].max(t[1]) > EPS
    }
}

/// Every text layer whose box overlaps a visible field it cannot be read on,
/// at each beat's READ; also returns how many (text, field) pairs overlapped
/// at all and how many text layers were judged.
fn violations(p: &MotionProject) -> (Vec<String>, usize, usize) {
    violations_at(p, false)
}

/// [`violations`] at READ and, with `later`, also at the READ/EVOLVE midpoint
/// and the end of EVOLVE.
fn violations_at(p: &MotionProject, later: bool) -> (Vec<String>, usize, usize) {
    let u = p.canvas.width.min(p.canvas.height) as f32 / 1080.0;
    let (mut out, mut overlapping, mut judged) = (Vec::new(), 0, 0);
    for scene in p.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
        let lc = scene.lifecycle.expect("beat lifecycle");
        let mut times = vec![lc.read];
        if later {
            times.push((lc.read + lc.evolve) / 2.0);
            times.push((lc.anticipate - 0.05).max(lc.read));
        }
        for local in times {
            let frame = ((scene.start_seconds + local) * f64::from(p.canvas.fps)).round() as u32;
            let resolved = evaluate_frame(p, frame).expect("evaluate");
            let fields = fields_of(&resolved.layers);
            let mut texts = Vec::new();
            texts_of(&resolved.layers, &scene.id, u, &mut texts);
            judged += texts.len();
            for t in &texts {
                for f in fields.iter().filter(|f| overlaps(f, &t.rect)) {
                    overlapping += 1;
                    let ratio = contrast(t.ink, f.fill);
                    if ratio < t.need {
                        out.push(format!(
                            "{} {} {:?}: ink {} on {} {} = {:.2}:1 < {:.1}:1 (at {local:.2}s, READ {:.2}s)",
                            scene.id,
                            t.id,
                            t.shown,
                            hex(t.ink),
                            f.id,
                            hex(f.fill),
                            ratio,
                            t.need,
                            lc.read
                        ));
                    }
                }
            }
        }
    }
    (out, overlapping, judged)
}

/// Every look with print fields x variety seeds 0..=7 x the three review
/// stories: no text layer's box overlaps a field below the contrast limit.
#[test]
fn no_text_box_overlaps_a_field_it_cannot_be_read_on() {
    let stories = stories();
    let looks = looks_with_fields();
    assert!(
        looks.contains(&Some(Look::HalftoneCutout)),
        "halftone_cutout draws print fields"
    );
    let (mut failures, mut compiles, mut judged, mut overlapping) = (Vec::new(), 0, 0, 0);
    for look in &looks {
        for story in &stories {
            for seed in SEEDS {
                let project = compile(story, *look, seed);
                let fields = project
                    .scenes
                    .first()
                    .map(|s| {
                        s.layers
                            .iter()
                            .filter(|l| l.id.starts_with("backdrop.field"))
                    })
                    .map_or(0, Iterator::count);
                assert!(
                    fields >= 3,
                    "{} {look:?} seed {seed}: no print fields in the backdrop",
                    story.name
                );
                compiles += 1;
                // Variety is on: the text of every sampled time of the beat
                // reads, not only the text at READ.
                let (found, o, j) = violations_at(&project, true);
                judged += j;
                overlapping += o;
                failures.extend(found.into_iter().map(|f| {
                    format!(
                        "{} {} seed {seed}: {f}",
                        story.name,
                        look.map_or("no look", |l| l.name())
                    )
                }));
            }
        }
    }
    assert!(compiles >= 3 * 8 * 2, "only {compiles} compiles");
    assert!(judged > 500, "only {judged} text layers judged");
    // The check is not vacuous: text does sit over fields (in colours it can
    // be read on) in this matrix.
    assert!(overlapping > 0, "no text overlapped any field");
    assert!(
        failures.is_empty(),
        "{} text layer(s) on a field they cannot be read on ({compiles} compiles, {judged} \
         layers, {overlapping} overlaps):\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}

/// The looks left out of the matrix above draw no print fields at all: a genre
/// look has its own ground (plates, photos, desks), so there is nothing to keep
/// off the text.
#[test]
fn genre_looks_draw_a_flat_ground_without_print_fields() {
    let s = stories().remove(0);
    let mut genre_looks = 0;
    for look in Look::ALL {
        let has_fields = palettes(look).iter().any(|p| !p.fields.is_empty());
        if has_fields && look_defaults(look).fx.grammar.is_some() {
            genre_looks += 1;
            let project = compile(&s, Some(look), 1);
            assert!(
                project.scenes[0]
                    .layers
                    .iter()
                    .all(|l| !l.id.starts_with("backdrop.field")),
                "{} draws print fields",
                look.name()
            );
        }
    }
    assert!(genre_looks > 0, "no genre look has field colours");
}

/// The regression the sprint found: sleep_review x playful x `--variety 1`
/// (halftone palette 1: a #111111 field under #111111 ink).
#[test]
fn sleep_review_playful_variety_1_keeps_ink_off_the_black_field() {
    let s = stories().remove(0);
    assert_eq!(s.name, "sleep_review");
    let project = compile(&s, Some(Look::HalftoneCutout), 1);
    let art = project.project.art.as_ref().expect("art record");
    assert_eq!(art.palette, 1, "variety 1 picks halftone palette 1");
    let (found, overlapping, _) = violations(&project);
    assert!(found.is_empty(), "{}", found.join("\n"));
    assert!(overlapping > 0, "no text over a field: the case is vacuous");
}

/// The checker is sensitive: recolour the field a text layer sits on to the
/// text's own colour and it is reported.
#[test]
fn the_check_flags_black_text_on_a_black_field() {
    let s = stories().remove(0);
    let mut project = compile(&s, Some(Look::HalftoneCutout), 1);
    assert!(violations(&project).0.is_empty());
    // Paint every field the ink colour of the palette.
    let ink = project.theme.palette["ink"];
    for layer in &mut project.scenes[0].layers {
        if layer.id.starts_with("backdrop.field") {
            match &mut layer.kind {
                LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => {
                    *fill = ink;
                }
                _ => {}
            }
        }
    }
    let (found, overlapping, _) = violations(&project);
    assert!(overlapping > 0);
    assert!(
        found.iter().any(|f| f.contains("ink #111111 on")),
        "ink-coloured fields under ink-coloured text were not reported: {found:?}"
    );
}

/// With variety the palette's text colours read on every ground of the look:
/// `muted` (small text) at 4.5:1 and `accent` (display text) at 3:1 against the
/// card, the paper and the paper under the plate at its worst. Without variety
/// the curated palette is used as it is.
#[test]
fn variety_text_colours_read_on_the_card_and_the_paper_under_the_plate() {
    let ratio = |a: motion_core::scene::Color, b: u32| {
        contrast([a.r, a.g, a.b], [(b >> 16) as u8, (b >> 8) as u8, b as u8])
    };
    let hex_u32 = |c: motion_core::scene::Color| {
        (u32::from(c.r) << 16) | (u32::from(c.g) << 8) | u32::from(c.b)
    };
    let mut tuned = 0;
    for story in &stories() {
        for seed in SEEDS {
            let p = compile(story, Some(Look::HalftoneCutout), seed);
            let art = p.project.art.as_ref().expect("art record");
            let spec = &palettes(Look::HalftoneCutout)[art.palette];
            let named = &p.theme.palette;
            let opacity = p.scenes[0]
                .layers
                .iter()
                .find(|l| l.id == "backdrop.plate")
                .map(|l| l.opacity);
            let plate = art.plate.as_deref().zip(opacity);
            let mut grounds = vec![spec.card, spec.paper];
            grounds.extend(ground_under_plate(spec, plate));
            assert_eq!(
                hex_u32(named["ink"]),
                spec.ink,
                "{} seed {seed}: the ink is never touched",
                story.name
            );
            for g in grounds {
                assert!(
                    ratio(named["muted"], g) >= 4.5,
                    "{} seed {seed}: muted on {g:06X} {:.2}:1",
                    story.name,
                    ratio(named["muted"], g)
                );
                assert!(
                    ratio(named["accent"], g) >= 3.0,
                    "{} seed {seed}: accent on {g:06X} {:.2}:1",
                    story.name,
                    ratio(named["accent"], g)
                );
            }
            tuned += usize::from(hex_u32(named["muted"]) != spec.muted);
        }
    }
    assert!(tuned > 0, "no palette needed its muted colour moved");
    // Without variety nothing moves: palette 0, its curated text colours.
    for story in &stories() {
        let p = compile_with(story, Some(Look::HalftoneCutout), None);
        let spec = &palettes(Look::HalftoneCutout)[0];
        assert_eq!(
            hex_u32(p.theme.palette["muted"]),
            spec.muted,
            "{}",
            story.name
        );
        assert_eq!(
            hex_u32(p.theme.palette["accent"]),
            spec.accent,
            "{}",
            story.name
        );
    }
}

/// The field boxes of the first backdrop: for each of the three fields, its
/// initial box and the box it expands to at each beat boundary (rounded to
/// px), as every colour layer of the field agrees on.
fn field_boxes(p: &MotionProject) -> Vec<([i32; 4], Vec<[i32; 4]>)> {
    let backdrop = &p.scenes[0];
    let round = |v: f32| v.round() as i32;
    (0..3)
        .map(|k| {
            let prefix = format!("backdrop.field{k}");
            let layers: Vec<_> = backdrop
                .layers
                .iter()
                .filter(|l| l.id == prefix || l.id.starts_with(&format!("{prefix}.")))
                .collect();
            let first = layers[0];
            let expands = |id: &str| -> Vec<[i32; 4]> {
                let mut m: Vec<_> = backdrop
                    .motions
                    .iter()
                    .filter(|m| m.target == id)
                    .filter_map(|m| match &m.op {
                        MotionOp::AccentExpand { to } => Some((m.start, to)),
                        _ => None,
                    })
                    .collect();
                m.sort_by(|a, b| a.0.total_cmp(&b.0));
                m.iter()
                    .map(|(_, b)| [round(b.x), round(b.y), round(b.width), round(b.height)])
                    .collect()
            };
            let boxes = expands(&first.id);
            for l in &layers {
                assert_eq!(expands(&l.id), boxes, "{}: colour layers disagree", l.id);
            }
            (
                [
                    round(first.x),
                    round(first.y),
                    round(first.width),
                    round(first.height),
                ],
                boxes,
            )
        })
        .collect()
}

/// Without variety a compile the rendered check passes keeps its default field
/// arrangements. money_review x playful, no art: beat 5's kicker "PATIENCE"
/// (#1C1633 on the blue field, 3.04:1) passes the 3:1 limit, so no beat moves
/// a field; the boxes are those of the 2b3fcb3 build (the arrangement of beat
/// i is `(i + fields) % 4`, mirrored).
#[test]
fn without_variety_a_clean_compile_keeps_the_default_field_arrangements() {
    let s = stories().remove(1);
    assert_eq!(s.name, "money_review");
    let p = compile_with(&s, None, None);
    let boxes = field_boxes(&p);
    let expect: [([i32; 4], [[i32; 4]; 4]); 3] = [
        (
            [-108, -96, 648, 288],
            [
                [1037, 96, 108, 768],
                [-108, -96, 756, 250],
                [-130, 384, 238, 691],
                [-108, -96, 648, 288],
            ],
        ),
        (
            [540, 1354, 864, 864],
            [
                [-302, -247, 648, 648],
                [-432, 1258, 864, 864],
                [853, -248, 497, 497],
                [540, 1354, 864, 864],
            ],
        ),
        (
            [-162, 1421, 594, 614],
            [
                [259, 1574, 778, 499],
                [810, 1306, 324, 806],
                [-65, 1536, 972, 499],
                [-162, 1421, 594, 614],
            ],
        ),
    ];
    for (k, ((first, moves), (want_first, want_moves))) in boxes.iter().zip(expect).enumerate() {
        assert_eq!(*first, want_first, "field {k} initial box");
        assert_eq!(moves.as_slice(), want_moves, "field {k}: beat 2..5 boxes");
        // Beat 5 comes back to beat 1's arrangement: its fields do not move off it.
        assert_eq!(moves[3], *first, "field {k} at beat 5");
    }
}

/// Without variety a word of a label on a field it cannot be read on is what
/// the rendered check flags (per word): sleep_review x playful, `--art`:
/// beat 4's label "WHAT SLEEP REPAIRS" (muted #6B6B6B) starts on the red field
/// (1.11:1). That beat's fields move; the other beats keep their default
/// arrangements (boxes of the 2b3fcb3 build).
#[test]
fn without_variety_a_word_on_a_field_moves_only_that_beats_fields() {
    let s = stories().remove(0);
    assert_eq!(s.name, "sleep_review");
    let p = compile_with(&s, Some(Look::HalftoneCutout), None);
    let boxes = field_boxes(&p);
    // The default arrangements: boxes[k].1[b - 2] is field k's box at beat b.
    let default: [([i32; 4], [[i32; 4]; 4]); 3] = [
        (
            [1037, 96, 108, 768],
            [
                [-108, -96, 756, 250],
                [-130, 384, 238, 691],
                [-108, -96, 648, 288],
                [1037, 96, 108, 768],
            ],
        ),
        (
            [-302, -247, 648, 648],
            [
                [-432, 1258, 864, 864],
                [853, -248, 497, 497],
                [540, 1354, 864, 864],
                [-302, -247, 648, 648],
            ],
        ),
        (
            [259, 1574, 778, 499],
            [
                [810, 1306, 324, 806],
                [-65, 1536, 972, 499],
                [-162, 1421, 594, 614],
                [259, 1574, 778, 499],
            ],
        ),
    ];
    for (k, ((first, moves), (want_first, want))) in boxes.iter().zip(default).enumerate() {
        assert_eq!(*first, want_first, "field {k} initial box");
        for beat in [2usize, 3, 5] {
            assert_eq!(moves[beat - 2], want[beat - 2], "field {k} at beat {beat}");
        }
        assert_ne!(
            moves[2], want[2],
            "field {k} at beat 4 stays under \"WHAT\""
        );
    }
}

/// The look keeps its character: three fields per beat, and each field
/// recomposes once per beat boundary (one expansion per boundary on every one
/// of its colour layers).
#[test]
fn three_fields_recompose_once_per_beat_boundary() {
    let stories = stories();
    for story in &stories {
        for seed in SEEDS {
            let project = compile(story, Some(Look::HalftoneCutout), seed);
            let beats = project
                .scenes
                .iter()
                .filter(|s| s.id.starts_with("beat_"))
                .count();
            let backdrop = &project.scenes[0];
            for k in 0..3 {
                let prefix = format!("backdrop.field{k}");
                let layers: Vec<&str> = backdrop
                    .layers
                    .iter()
                    .map(|l| l.id.as_str())
                    .filter(|id| *id == prefix || id.starts_with(&format!("{prefix}.")))
                    .collect();
                assert!(
                    !layers.is_empty(),
                    "{} seed {seed}: no field {k}",
                    story.name
                );
                for id in layers {
                    let expands = backdrop
                        .motions
                        .iter()
                        .filter(|m| m.target == id && matches!(m.op, MotionOp::AccentExpand { .. }))
                        .count();
                    assert_eq!(
                        expands,
                        beats - 1,
                        "{} seed {seed}: {id} expands {expands} times for {beats} beats",
                        story.name
                    );
                }
            }
            // Exactly three fields are visible at every beat's READ.
            for scene in project.scenes.iter().filter(|s| s.id.starts_with("beat_")) {
                let lc = scene.lifecycle.expect("lifecycle");
                let frame = ((scene.start_seconds + lc.read) * 30.0).round() as u32;
                let resolved = evaluate_frame(&project, frame).expect("evaluate");
                let visible = fields_of(&resolved.layers).len();
                assert_eq!(
                    visible, 3,
                    "{} seed {seed} {}: {visible} fields visible at READ",
                    story.name, scene.id
                );
            }
        }
    }
}

/// The compare beat's second panel (accent, with the on-accent text) is in
/// before its text starts: a voice-over that names "5 hours" pulls the text
/// forward, and the text must not read on the bare ground (white on paper,
/// 1.8:1) until its panel comes in.
#[test]
fn the_second_panel_comes_in_before_its_text() {
    let s = stories().remove(0);
    assert_eq!(s.name, "sleep_review");
    for look in [None, Some(Look::HalftoneCutout)] {
        for seed in [0, 1] {
            let p = compile(&s, look, seed);
            let scene = p.scenes.iter().find(|s| s.id == "beat_3").expect("beat 3");
            let lc = scene.lifecycle.expect("lifecycle");
            let start_of = |pred: &dyn Fn(&motion_core::scene::Motion) -> bool| {
                scene
                    .motions
                    .iter()
                    .filter(|m| pred(m))
                    .map(|m| m.start)
                    .fold(f64::MAX, f64::min)
            };
            let panel = start_of(&|m| {
                m.target == "b3.panel_b" && matches!(m.op, MotionOp::MaskReveal { .. })
            });
            let text = start_of(&|m| {
                m.target.starts_with("b3.side_b.") && matches!(m.op, MotionOp::Fade { .. })
            });
            assert!(
                panel < f64::MAX && text < f64::MAX,
                "{look:?} seed {seed}: no panel / text motions"
            );
            assert!(
                text < lc.read,
                "{look:?} seed {seed}: the narrator names the second value before READ \
                 ({text:.2}s < {:.2}s): the case is vacuous otherwise",
                lc.read
            );
            assert!(
                panel <= text + 1e-9,
                "{look:?} seed {seed}: panel b starts at {panel:.2}s, after its text ({text:.2}s)"
            );
        }
    }
}

/// Deterministic: the same compile twice gives the same scene.
#[test]
fn the_backdrop_is_deterministic() {
    let s = stories().remove(0);
    for seed in [1, 4] {
        let a = serde_json::to_string(&compile(&s, Some(Look::HalftoneCutout), seed)).expect("a");
        let b = serde_json::to_string(&compile(&s, Some(Look::HalftoneCutout), seed)).expect("b");
        assert_eq!(a, b, "seed {seed}");
    }
}
