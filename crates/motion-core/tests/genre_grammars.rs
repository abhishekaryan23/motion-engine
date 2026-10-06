//! (0.14) The genre grammars of the hype and documentary looks, from in-repo
//! data only: `KineticSlam` (hard-cut punch units) and `DocumentaryDossier`
//! (clipping, photo, figure, stamp, highlighter). Structural assertions on
//! the compiled scenes plus the resolved frames; no pixels.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::art_direction::ArtMode;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, ApproxMeasure, AssetLibrary, CompileOptions, TextMeasure,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{
    FontRole, Layer, LayerKind, Motion, MotionOp, MotionProject, Scene, SpringSpec,
};
use motion_core::style::StyleProfile;
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use motion_core::validate::validate;
use motion_core::{layout_report, Easing};

const FPS: f64 = 30.0;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn style(tone: &str) -> StyleProfile {
    serde_json::from_str(&format!(
        r#"{{"tone":"{tone}","polarity":"dark","temperature":"warm","temperament":"balanced",
        "density":"dense","accent_role":"signal_red","texture_style":"heavy_print",
        "depth":"layered","camera_style":"drift","motion_language":"parallax"}}"#
    ))
    .expect("style")
}

/// Three beats: a figure with a library object, a phrase with a number and an
/// object, and a phrase-only beat. Statements are short titles.
const INTENT: &str = r#"{
  "version": "0.2", "title": "genre_grammars", "format": "vertical",
  "beats": [
    {"purpose": "emphasize", "statement": "Cash is king",
     "primary": {"kind": "phrase", "value": "$381 billion", "meaning": "cash reserves"},
     "secondary": {"kind": "object", "asset": "money_bag", "meaning": "money"},
     "energy": "impact", "keyword": "cash"},
    {"purpose": "emphasize", "statement": "Robots keep selling for 36 months",
     "primary": {"kind": "number", "value": "36 months", "meaning": "selling"},
     "secondary": {"kind": "object", "asset": "robot", "meaning": "robot"},
     "energy": "building", "keyword": "selling"},
    {"purpose": "emphasize", "statement": "Still waiting for the crash",
     "primary": {"kind": "phrase", "value": "still waiting"},
     "energy": "impact", "keyword": "waiting"}
  ]
}"#;

fn compile(tone: &str, canvas: Option<(u32, u32)>) -> MotionProject {
    compile_intent(INTENT, tone, canvas)
}

fn compile_intent(intent: &str, tone: &str, canvas: Option<(u32, u32)>) -> MotionProject {
    let intent = CreativeIntent::from_json(intent).expect("intent");
    let library = AssetLibrary::new(repo().join("assets")).with_families(vec![
        "clay_concepts_3d".to_string(),
        "editorial_concepts".to_string(),
    ]);
    let opts = CompileOptions {
        art: Some(ArtMode::Auto),
        canvas,
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
    .expect("compile")
}

fn beat_scenes(p: &MotionProject) -> Vec<&Scene> {
    p.scenes.iter().filter(|s| s.lifecycle.is_some()).collect()
}

/// The children of a scene's stage group.
fn stage(s: &Scene) -> &[Layer] {
    s.layers
        .iter()
        .find_map(|l| match &l.kind {
            LayerKind::Group { children } if l.id.ends_with(".stage") => Some(children.as_slice()),
            _ => None,
        })
        .expect("stage group")
}

fn all_layers<'a>(layers: &'a [Layer], out: &mut Vec<&'a Layer>) {
    for l in layers {
        out.push(l);
        if let LayerKind::Group { children } = &l.kind {
            all_layers(children, out);
        }
    }
}

/// Every layer of the beat. Under a perspective look the FX director hoists
/// depth cards out of the stage into `<card>.depth` wrappers, so search the
/// whole scene, not just the stage.
fn flat(s: &Scene) -> Vec<&Layer> {
    let _ = stage(s);
    let mut v = Vec::new();
    all_layers(&s.layers, &mut v);
    v
}

/// The perspective depth and tilt of a card: on its `.depth` wrapper when
/// the FX director hoisted it, else on the card itself.
fn depth_of(s: &Scene, suffix: &str) -> (Option<f32>, Option<[f32; 2]>) {
    let id = format!("{}.{suffix}", s.id.replace("beat_", "b"));
    let wrapped = format!("{id}.depth");
    flat(s)
        .into_iter()
        .find(|l| l.id == wrapped)
        .or_else(|| flat(s).into_iter().find(|l| l.id == id))
        .map(|l| (l.z, l.tilt))
        .unwrap_or((None, None))
}

fn motions_of<'a>(s: &'a Scene, id: &str) -> Vec<&'a Motion> {
    s.motions.iter().filter(|m| m.target == id).collect()
}

fn text_of(l: &Layer) -> Option<&str> {
    match &l.kind {
        LayerKind::Text(t) => Some(t.text.as_str()),
        _ => None,
    }
}

/// Leaf layers of a resolved frame with their effective opacity and the
/// axis-aligned box of their resolved transform: `(id, opacity, [x0, y0, x1, y1])`.
fn resolved_boxes(p: &MotionProject, frame: u32) -> Vec<(String, f32, [f32; 4])> {
    fn walk(layers: &[ResolvedLayer], parent_opacity: f32, out: &mut Vec<(String, f32, [f32; 4])>) {
        for l in layers {
            let o = parent_opacity * l.opacity;
            if matches!(l.kind, LayerKind::Group { .. }) {
                walk(&l.children, o, out);
                continue;
            }
            let t = l.transform;
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (cx, cy) in [
                (0.0, 0.0),
                (l.width, 0.0),
                (0.0, l.height),
                (l.width, l.height),
            ] {
                let x = t.a * cx + t.c * cy + t.e;
                let y = t.b * cx + t.d * cy + t.f;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            out.push((l.id.to_string(), o, [x0, y0, x1, y1]));
        }
    }
    let f = evaluate_frame(p, frame).expect("frame");
    let mut out = Vec::new();
    walk(&f.layers, 1.0, &mut out);
    out
}

fn frame_at(t: f64) -> u32 {
    (t * FPS).round() as u32
}

/// Every visible layer these builders add (cut groups, cards, stamp) lies inside
/// the canvas (a little slack for the drift camera), except full-bleed fills
/// and plates; transition panels and furniture are not theirs.
fn assert_inside_canvas(p: &MotionProject, scene: &Scene, local_t: f64, label: &str) {
    let (w, h) = (p.canvas.width as f32, p.canvas.height as f32);
    let slack = 0.04;
    let frame = frame_at(scene.start_seconds + local_t);
    let prefix = format!("{}.", scene.id.replace("beat_", "b"));
    for (id, opacity, [x0, y0, x1, y1]) in resolved_boxes(p, frame) {
        let mine = ["cut.", "clip", "photo", "figure", "stamp"]
            .iter()
            .any(|n| id.starts_with(&format!("{prefix}{n}")));
        if !mine || opacity < 0.05 {
            continue;
        }
        let bleed = x0 <= 1.0 && y0 <= 1.0 && x1 >= w - 1.0 && y1 >= h - 1.0;
        if bleed {
            continue;
        }
        assert!(
            x0 >= -slack * w
                && y0 >= -slack * h
                && x1 <= (1.0 + slack) * w
                && y1 <= (1.0 + slack) * h,
            "{label}: {id} at {local_t:.2}s sticks out of the {w}x{h} canvas: \
             x {x0:.0}..{x1:.0} y {y0:.0}..{y1:.0}"
        );
    }
}

fn fades<'a>(s: &'a Scene, id: &str) -> Vec<&'a Motion> {
    motions_of(s, id)
        .into_iter()
        .filter(|m| matches!(m.op, MotionOp::Fade { .. }))
        .collect()
}

// ---------------------------------------------------------------------------
// KineticSlam (hype)
// ---------------------------------------------------------------------------

/// `(group id, cut time)` of every cut group of a beat scene, in order.
fn cut_groups(s: &Scene) -> Vec<(String, f64)> {
    let mut cuts: Vec<(String, f64)> = stage(s)
        .iter()
        .filter(|l| {
            l.id.split('.').nth(1) == Some("cut")
                && l.id.split('.').count() == 3
                && matches!(l.kind, LayerKind::Group { .. })
        })
        .map(|l| {
            let start = fades(s, &l.id)
                .iter()
                .find(|m| matches!(m.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0))
                .map(|m| m.start)
                .expect("cut-in fade");
            (l.id.clone(), start)
        })
        .collect();
    cuts.sort_by(|a, b| a.1.total_cmp(&b.1));
    cuts
}

#[test]
fn slam_builds_hard_cut_groups_that_alternate_one_at_a_time() {
    let p = compile("hype", None);
    validate(&p, None).expect("validate");
    for s in beat_scenes(&p) {
        let life = s.lifecycle.expect("lifecycle");
        let cuts = cut_groups(s);
        assert!(!cuts.is_empty(), "{}: no cut groups", s.id);
        assert!(
            cuts[0].1 >= life.enter - 2e-3,
            "{}: first cut before ENTER",
            s.id
        );
        for (k, (id, at)) in cuts.iter().enumerate() {
            assert_eq!(id, &format!("{}.cut.{k}", s.id.replace("beat_", "b")));
            let f = fades(s, id);
            // Cut in: 0 -> 1 in 0.01 s.
            assert!(f.iter().any(|m| (m.duration - 0.01).abs() < 1e-9
                && matches!(m.op, MotionOp::Fade { from, to } if from == 0.0 && to == 1.0)));
            match cuts.get(k + 1) {
                // Cut out exactly at the next cut: 1 -> 0 in 0.01 s.
                Some((_, next)) => assert!(
                    f.iter().any(|m| (m.start - next).abs() < 1e-6
                        && (m.duration - 0.01).abs() < 1e-9
                        && matches!(m.op, MotionOp::Fade { from, to } if from == 1.0 && to == 0.0)),
                    "{id}: no hard cut out at the next cut"
                ),
                // The last unit holds to the scene end.
                None => assert!(
                    !f.iter()
                        .any(|m| matches!(m.op, MotionOp::Fade { to, .. } if to == 0.0)),
                    "{id}: the last cut must hold"
                ),
            }
            // Unit length: 0.4..=1.8 s for every cut that is followed by another.
            if let Some((_, next)) = cuts.get(k + 1) {
                let len = next - at;
                assert!((0.4..=1.8).contains(&len), "{id}: unit lasts {len:.2}s");
            }
        }
        // Exactly one cut group is on screen at a time, from the first cut to
        // the scene end (the last one holds).
        let end = life.anticipate - 0.05;
        let mut t = cuts[0].1 + 0.1;
        while t < end {
            let frame = frame_at(s.start_seconds + t);
            let f = evaluate_frame(&p, frame).expect("frame");
            let visible = visible_cut_groups(&f.layers, 1.0, &s.id.replace("beat_", "b"));
            assert_eq!(visible.len(), 1, "{} at {t:.2}s shows {visible:?}", s.id);
            t += 0.17;
        }
    }
}

fn visible_cut_groups(layers: &[ResolvedLayer], parent: f32, prefix: &str) -> Vec<String> {
    let mut out = Vec::new();
    for l in layers {
        let o = parent * l.opacity;
        if matches!(l.kind, LayerKind::Group { .. }) {
            let is_cut =
                l.id.split('.').count() == 3 && l.id.starts_with(&format!("{prefix}.cut."));
            if is_cut && o > 0.6 {
                out.push(l.id.to_string());
            }
            out.extend(visible_cut_groups(&l.children, o, prefix));
        }
    }
    out
}

#[test]
fn slam_words_slam_on_a_spring_and_short_ones_cascade() {
    let p = compile("hype", None);
    let spring = SpringSpec {
        stiffness: 520.0,
        damping: 26.0,
        mass: 1.0,
    };
    let mut cascades = 0;
    let mut words = 0;
    for s in beat_scenes(&p) {
        for l in flat(s) {
            if !l.id.ends_with(".word") || !l.id.contains(".cut.") {
                continue;
            }
            words += 1;
            let m = motions_of(s, &l.id);
            let slam = m
                .iter()
                .find(|m| matches!(m.op, MotionOp::Scale { from, to, .. } if from == 1.8 && to == 1.0))
                .unwrap_or_else(|| panic!("{}: no 1.8 -> 1 slam", l.id));
            assert_eq!(slam.spring, Some(spring), "{}", l.id);
            assert!((slam.duration - 0.22).abs() < 1e-9, "{}", l.id);
            let glyphs = text_of(l).map_or(0, |t| t.chars().filter(|c| !c.is_whitespace()).count());
            let cascade = m.iter().find_map(|m| match &m.op {
                MotionOp::GlyphCascade { stagger, from, .. } => Some((*stagger, *from)),
                _ => None,
            });
            if glyphs <= 8 {
                let (stagger, from) = cascade.unwrap_or_else(|| panic!("{}: no cascade", l.id));
                assert_eq!(stagger, 0.02);
                assert_eq!((from.scale, from.opacity), (1.4, 0.0));
                cascades += 1;
            } else {
                assert!(cascade.is_none(), "{}: long word must not cascade", l.id);
            }
        }
    }
    assert!(words >= 6, "expected a run of punch words, got {words}");
    assert!(cascades >= 3, "short words cascade ({cascades})");
}

#[test]
fn slam_layouts_rotate_and_the_picture_comes_with_its_cut() {
    let p = compile("hype", None);
    let mut inverse = 0;
    let mut pictures = 0;
    for s in beat_scenes(&p) {
        let accent = {
            // The accent colour of the project palette is the field colour.
            let mut found = None;
            for l in flat(s) {
                if l.id.ends_with(".field") {
                    if let LayerKind::Rectangle { fill, .. } = &l.kind {
                        found = Some(*fill);
                    }
                }
            }
            found
        };
        for l in flat(s) {
            if l.id.ends_with(".field") && l.id.contains(".cut.") {
                inverse += 1;
                // The inverse word is not ink on paper: it sits on the field.
                assert!(accent.is_some());
                let k: usize =
                    l.id.split('.')
                        .nth(2)
                        .and_then(|k| k.parse().ok())
                        .expect("k");
                assert_eq!(k % 3, 1, "{}: inverse field on a B unit", l.id);
            }
            if l.id.contains(".cut.")
                && l.id.contains(".pic")
                && matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })
            {
                pictures += 1;
            }
        }
    }
    assert!(inverse >= 2, "B layouts have an accent field ({inverse})");
    assert!(
        pictures >= 1,
        "the beat's picture shows on a C cut ({pictures})"
    );
}

#[test]
fn slam_keeps_every_layer_inside_the_canvas_on_every_format() {
    for canvas in [
        None,
        Some((1920, 1080)),
        Some((1080, 1080)),
        Some((1080, 1350)),
    ] {
        let p = compile("hype", canvas);
        validate(&p, None).expect("validate");
        for s in beat_scenes(&p) {
            let cuts = cut_groups(s);
            for (k, (_, at)) in cuts.iter().enumerate() {
                // After the slam and the glyph cascade have landed, mid unit.
                let end = cuts.get(k + 1).map_or(s.duration_seconds, |c| c.1);
                let t = (at + 0.55).min((at + end) / 2.0 + 0.3).min(end - 0.02);
                assert_inside_canvas(&p, s, t, &format!("slam {canvas:?}"));
            }
        }
        let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
        let report = layout_report(&p, &frame);
        assert!(
            report.passed(),
            "layout QA {canvas:?}\n{}",
            report.to_text()
        );
    }
}

// ---------------------------------------------------------------------------
// DocumentaryDossier (documentary)
// ---------------------------------------------------------------------------

fn find<'a>(s: &'a Scene, suffix: &str) -> Option<&'a Layer> {
    let id = format!("{}.{suffix}", s.id.replace("beat_", "b"));
    flat(s).into_iter().find(|l| l.id == id)
}

#[test]
fn dossier_lays_out_clipping_photo_figure_stamp_and_highlighter() {
    let p = compile("documentary", None);
    validate(&p, None).expect("validate");
    let scenes = beat_scenes(&p);
    assert_eq!(scenes.len(), 3);
    for (i, s) in scenes.iter().enumerate() {
        let life = s.lifecycle.expect("lifecycle");
        let clip = find(s, "clip").unwrap_or_else(|| panic!("{}: no clipping", s.id));
        // Cards carry the perspective depth and tilt; (0.18) the focal card
        // (recorded in the art record) is on the focus plane, the others behind.
        let _ = clip;
        let focal = p
            .project
            .art
            .as_ref()
            .and_then(|a| a.focal.get(&s.id))
            .unwrap_or_else(|| panic!("{}: no focal layer recorded", s.id));
        let focal_name = focal.split_once('.').map_or(focal.as_str(), |(_, n)| n);
        assert_eq!(
            depth_of(s, focal_name),
            (Some(0.0), Some([6.0, -4.0])),
            "{}",
            s.id
        );
        let (clip_z, _) = depth_of(s, "clip");
        assert!(
            matches!(clip_z, Some(z) if z == 0.0 || z == 70.0 || z == 140.0),
            "{}: clip z {clip_z:?}",
            s.id
        );
        // Serif headline, at most four lines.
        let title = find(s, "clip.title").expect("title");
        let LayerKind::Text(t) = &title.kind else {
            panic!("title is text")
        };
        assert_eq!(t.font_role, FontRole::SerifEmotional);
        assert!(
            (1..=4).contains(&t.text.lines().count()),
            "{}: {}",
            s.id,
            t.text
        );
        // 3-5 redaction bars.
        let bars = flat(s)
            .iter()
            .filter(|l| {
                l.id.starts_with(&format!("{}.clip.bar.", s.id.replace("beat_", "b")))
            })
            .count();
        assert!((3..=5).contains(&bars), "{}: {bars} bars", s.id);
        // The clipping slides in on the 260/24 spring.
        let slide = motions_of(s, &clip.id)
            .into_iter()
            .find(|m| matches!(m.op, MotionOp::Move { .. }))
            .expect("slide");
        assert_eq!(
            slide.spring,
            Some(SpringSpec {
                stiffness: 260.0,
                damping: 24.0,
                mass: 1.0
            })
        );
        // The highlighter grows left to right at EVOLVE, accent at 0.55.
        let hl = find(s, "clip.highlight").unwrap_or_else(|| panic!("{}: no highlighter", s.id));
        assert!((hl.opacity - 0.55).abs() < 1e-6);
        let grow = motions_of(s, &hl.id)
            .into_iter()
            .find(|m| {
                matches!(
                    m.op,
                    MotionOp::MaskReveal {
                        direction: motion_core::scene::Direction::Right,
                        ..
                    }
                )
            })
            .expect("highlighter reveal");
        assert!(
            grow.start >= life.evolve - 2e-3,
            "{}: highlighter before EVOLVE",
            s.id
        );
        // The stamp: keyword uppercased, rotated -8, slams 2.4 -> 1 in 0.18 s.
        let stamp = find(s, "stamp").unwrap_or_else(|| panic!("{}: no stamp", s.id));
        assert!((stamp.rotation_degrees + 8.0).abs() < 1e-6);
        let word = find(s, "stamp.text").and_then(text_of).expect("stamp text");
        let keyword = ["cash", "selling", "waiting"][i];
        assert_eq!(word.replace('\n', " "), keyword.to_uppercase());
        let slam = motions_of(s, &stamp.id)
            .into_iter()
            .find(|m| matches!(m.op, MotionOp::Scale { from, to, .. } if from == 2.4 && to == 1.0))
            .expect("stamp slam");
        assert!((slam.duration - 0.18).abs() < 1e-9);
        assert!(slam.start >= life.evolve - 2e-3);
        // The stamp frame is an accent-stroked rectangle.
        let frame = find(s, "stamp.frame").expect("stamp frame");
        let LayerKind::Rectangle {
            stroke: Some(st), ..
        } = &frame.kind
        else {
            panic!("stamp frame is a stroked rectangle")
        };
        assert!(st.width > 4.0);
    }
    // Beat 1 and 2: the primary value is a figure (digit / currency): a third
    // card, a mono label, a red underline drawn with `trim`.
    for s in &scenes[..2] {
        let figure = find(s, "figure").unwrap_or_else(|| panic!("{}: no figure card", s.id));
        let _ = figure;
        // (0.18) The figure is what the beat is about: on the focus plane.
        assert_eq!(depth_of(s, "figure").0, Some(0.0));
        let value = find(s, "figure.value").and_then(text_of).expect("value");
        assert!(value.chars().any(|c| c.is_ascii_digit()));
        let label = find(s, "figure.label").expect("label");
        assert!(matches!(&label.kind, LayerKind::Text(t) if t.font_role == FontRole::Mono));
        let under = find(s, "figure.underline").expect("underline");
        assert!(matches!(&under.kind, LayerKind::Polyline { .. }));
        let trim = motions_of(s, &under.id)
            .into_iter()
            .find(|m| matches!(m.op, MotionOp::Trim { from, to } if from == 0.0 && to == 1.0))
            .expect("trim");
        assert!(trim.start >= s.lifecycle.expect("life").evolve - 2e-3);
        // Photo card: behind the focal figure (depth 70). (0.22) A library
        // picture is bare (no paper), so no tape floats over its margins; it
        // wears an ink print border instead.
        let photo = find(s, "photo").unwrap_or_else(|| panic!("{}: no photo", s.id));
        let _ = photo;
        assert_eq!(depth_of(s, "photo").0, Some(70.0));
        assert!(find(s, "photo.tape").is_none());
        assert!(
            flat(s).iter().any(|l| l.id.contains(".photo.pic")
                && matches!(&l.kind, LayerKind::Image { treatment: Some(t), .. } if t.sticker.is_some())),
            "{}: the library picture wears a print border",
            s.id
        );
        assert!(
            flat(s).iter().any(|l| l.id.contains(".photo.pic")
                && matches!(l.kind, LayerKind::Image { .. } | LayerKind::Svg { .. })),
            "{}: the picture is on the photo card",
            s.id
        );
        // Cards enter 0.18 s apart.
        let start = |c: &Layer| {
            motions_of(s, &c.id)
                .into_iter()
                .filter(|m| matches!(m.op, MotionOp::Move { .. }))
                .map(|m| m.start)
                .fold(f64::MAX, f64::min)
        };
        let clip = find(s, "clip").expect("clip");
        assert!((start(photo) - start(clip) - 0.18).abs() < 1e-6);
        assert!((start(figure) - start(clip) - 0.36).abs() < 1e-6);
    }
    // Beat 3 has a short phrase value and no object: (0.18) the phrase is the
    // verdict on the figure card (on the focus plane, stamped); no photo.
    let verdict = find(scenes[2], "figure.value")
        .and_then(text_of)
        .expect("verdict card");
    assert!(!verdict.trim().is_empty());
    assert_eq!(depth_of(scenes[2], "figure").0, Some(0.0));
    assert!(find(scenes[2], "photo").is_none());
}

#[test]
fn dossier_keeps_every_layer_inside_the_canvas_on_every_format() {
    for canvas in [
        None,
        Some((1920, 1080)),
        Some((1080, 1080)),
        Some((1080, 1350)),
    ] {
        let p = compile("documentary", canvas);
        validate(&p, None).expect("validate");
        for s in beat_scenes(&p) {
            let life = s.lifecycle.expect("lifecycle");
            let stamp = find(s, "stamp")
                .map(|l| {
                    motions_of(s, &l.id)
                        .iter()
                        .map(|m| m.start + m.duration)
                        .fold(0.0, f64::max)
                })
                .unwrap_or(life.evolve);
            // Cards landed (READ), mid EVOLVE, stamp landed, before ANTICIPATE.
            for t in [
                life.read + 0.3,
                (life.read + life.evolve) / 2.0,
                (stamp + 0.05).min(life.anticipate - 0.05),
            ] {
                assert_inside_canvas(&p, s, t, &format!("dossier {canvas:?}"));
            }
        }
        let frame = LayoutFrame::new(p.canvas.width, p.canvas.height).expect("frame");
        let report = layout_report(&p, &frame);
        assert!(
            report.passed(),
            "layout QA {canvas:?}\n{}",
            report.to_text()
        );
    }
}

// ---------------------------------------------------------------------------
// Both
// ---------------------------------------------------------------------------

#[test]
fn genre_grammars_are_deterministic() {
    for tone in ["hype", "documentary"] {
        let a = serde_json::to_string(&compile(tone, None)).expect("json");
        let b = serde_json::to_string(&compile(tone, None)).expect("json");
        assert_eq!(a, b, "{tone}");
    }
}

#[test]
fn only_the_genre_tones_reach_the_genre_grammars() {
    // Without a tone the same story keeps its semantic grammars.
    let intent = CreativeIntent::from_json(INTENT).expect("intent");
    let library = AssetLibrary::new(repo().join("assets"))
        .with_families(vec!["clay_concepts_3d".to_string()]);
    let neutral: StyleProfile = serde_json::from_str(
        r#"{"material":"paper","typography_style":"grotesk_serif","depth":"layered",
        "camera_style":"slow_push","motion_language":"auto","texture_style":"subtle_print",
        "accent_role":"cobalt","seed":1}"#,
    )
    .expect("style");
    let p = compile_with_options(
        &intent,
        &neutral,
        None,
        &library,
        &ApproxMeasure,
        &AssetManifest::empty(),
        None,
        &CompileOptions {
            art: Some(ArtMode::Auto),
            ..CompileOptions::default()
        },
    )
    .expect("compile");
    for s in beat_scenes(&p) {
        assert!(
            find(s, "clip").is_none() && find(s, "stamp").is_none(),
            "{}",
            s.id
        );
        assert!(cut_groups(s).is_empty(), "{}", s.id);
    }
    let _ = Easing::Linear;
}

/// A comparison: both numbers are shown (the secondary is not lost).
const COMPARISON: &str = r#"{
  "version": "0.2", "title": "comparison", "format": "vertical",
  "beats": [
    {"purpose": "contrast", "statement": "Then versus now",
     "primary": {"kind": "number", "value": "3%", "meaning": "2019"},
     "secondary": {"kind": "number", "value": "41%", "meaning": "2024"},
     "relationship": "grow", "energy": "impact"}
  ]
}"#;

#[test]
fn a_comparison_keeps_both_numbers_in_both_grammars() {
    let doc = compile_intent(COMPARISON, "documentary", None);
    validate(&doc, None).expect("validate");
    let s = beat_scenes(&doc)[0];
    assert_eq!(find(s, "figure.value").and_then(text_of), Some("3%"));
    assert_eq!(find(s, "figure.value2").and_then(text_of), Some("41%"));
    assert!(find(s, "figure.underline2").is_some());

    let hype = compile_intent(COMPARISON, "hype", None);
    validate(&hype, None).expect("validate");
    let s = beat_scenes(&hype)[0];
    let words: Vec<String> = flat(s)
        .iter()
        .filter(|l| l.id.ends_with(".word"))
        .filter_map(|l| text_of(l).map(str::to_string))
        .collect();
    assert!(words.iter().any(|w| w == "3%"), "{words:?}");
    assert!(words.iter().any(|w| w == "41%"), "{words:?}");
    // The payoff is last and holds.
    assert_eq!(words.last().map(String::as_str), Some("41%"));
}
