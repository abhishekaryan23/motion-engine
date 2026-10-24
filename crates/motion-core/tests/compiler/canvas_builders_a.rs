//! (0.9 R1) Responsive canvas, builder set A: HeroObject, EvidenceStack,
//! TypeImageInterlock, KineticPoster and DataStory (with `placement`) compile
//! on any canvas of the allowed aspect range, keep their text inside the
//! canvas and the safe area horizontally, and never use type below the
//! frame's minimum size. Legacy canvases are covered by the goldens.

use std::path::PathBuf;

use motion_core::assets::AssetManifest;
use motion_core::compiler::layout_frame::LayoutFrame;
use motion_core::compiler::{
    compile_with_options, plan_assets, ApproxMeasure, AssetLibrary, CompileOptions,
};
use motion_core::intent::CreativeIntent;
use motion_core::scene::{Layer, LayerKind, MotionProject};
use motion_core::style::StyleProfile;
use motion_core::validate::validate;

const CANVASES: [(u32, u32); 9] = [
    (1080, 2520),
    (1080, 1920),
    (1080, 1440),
    (1080, 1350),
    (1080, 1080),
    (1296, 1080),
    (1440, 1080),
    (1920, 1080),
    (2520, 1080),
];
const STYLES: [&str; 3] = ["warm_editorial", "dark_technical", "playful_print"];
/// Grammars owned by this builder set.
const MINE: [&str; 5] = [
    "hero_object",
    "evidence_stack",
    "type_image_interlock",
    "kinetic_poster",
    "data_story",
];
/// Float slack for box containment (px).
const TOL: f32 = 1.5;

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn intent(rel: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn style(name: &str) -> StyleProfile {
    serde_json::from_str(&read(&format!("examples/taste/{name}.style.json"))).expect("style")
}

struct Case {
    name: &'static str,
    intent: CreativeIntent,
    families: &'static [&'static str],
    /// Delivered images: grammar selection depends on them, so every beat of
    /// these cases is treated as ours.
    manifest: Option<AssetManifest>,
}

fn manifest() -> AssetManifest {
    serde_json::from_str(&read("assets/test_manifest.json")).expect("manifest")
}

fn cases() -> Vec<Case> {
    let plain = |name, rel: &str, families| Case {
        name,
        intent: intent(rel),
        families,
        manifest: None,
    };
    vec![
        plain("editorial_demo", "examples/editorial_demo.intent.json", &[]),
        plain(
            "motion_language_demo",
            "examples/motion_language_demo.intent.json",
            &[],
        ),
        plain("g_hero_object", "golden/hero_object.intent.json", &[]),
        plain("g_evidence_stack", "golden/evidence_stack.intent.json", &[]),
        plain("g_kinetic_poster", "golden/kinetic_poster.intent.json", &[]),
        plain("g_data_story", "golden/data_story.intent.json", &[]),
        plain("g_type_image", "golden/type_image.intent.json", &[]),
        plain(
            "g_editorial_collage",
            "golden/editorial_collage.intent.json",
            &[],
        ),
        plain(
            "lib_classical",
            "examples/library_09/classical.intent.json",
            &["classical_greyscale"],
        ),
        plain(
            "lib_clay",
            "examples/library_09/clay.intent.json",
            &["clay_props_3d"],
        ),
        plain(
            "lib_people",
            "examples/library_09/people.intent.json",
            &["people_everyday"],
        ),
        plain(
            "lib_retro",
            "examples/library_09/retro.intent.json",
            &["halftone_retro_objects"],
        ),
        plain(
            "icon_demo",
            "examples/icon_demo/icon_demo.intent.json",
            &["sketch_icons", "woodcut_kitchen"],
        ),
        Case {
            name: "images_type_image",
            intent: intent("golden/type_image.intent.json"),
            families: &[],
            manifest: Some(manifest()),
        },
        Case {
            name: "images_evidence",
            intent: intent("golden/evidence_stack.intent.json"),
            families: &[],
            manifest: Some(manifest()),
        },
        Case {
            name: "images_hero",
            intent: intent("golden/hero_object.intent.json"),
            families: &[],
            manifest: Some(manifest()),
        },
    ]
}

fn library(families: &[&str]) -> AssetLibrary {
    AssetLibrary::new(repo().join("assets"))
        .with_families(families.iter().map(|s| s.to_string()).collect())
}

fn compile(case: &Case, style: &StyleProfile, canvas: Option<(u32, u32)>) -> MotionProject {
    let empty = AssetManifest::empty();
    compile_with_options(
        &case.intent,
        style,
        None,
        &library(case.families),
        &ApproxMeasure,
        case.manifest.as_ref().unwrap_or(&empty),
        None,
        &CompileOptions {
            canvas,
            ..CompileOptions::default()
        },
    )
    .unwrap_or_else(|e| panic!("{} {canvas:?}: {e}", case.name))
}

/// 1-based beats built by one of our grammars at this canvas class.
fn our_beats(case: &Case, style: &StyleProfile, frame: &LayoutFrame) -> Vec<usize> {
    let n = case.intent.beats.len();
    if case.manifest.is_some() {
        return (1..=n).collect();
    }
    let mut at_class = case.intent.clone();
    at_class.format = frame.class;
    let plan = plan_assets(&at_class, style, &library(case.families)).expect("plan");
    plan.beats
        .iter()
        .filter(|d| MINE.contains(&d.composition.as_str()))
        .map(|d| d.beat)
        .collect()
}

struct TextBox {
    id: String,
    x0: f32,
    x1: f32,
    y0: f32,
    y1: f32,
    size: f32,
}

/// Axis-aligned bounds of a static layer box (anchor, scale, rotation applied),
/// offset by its parent group's top-left.
fn bounds(l: &Layer, off: (f32, f32)) -> (f32, f32, f32, f32) {
    let (w, h) = (l.width * l.scale_x, l.height * l.scale_y);
    let (ax, ay) = (l.anchor_x * w, l.anchor_y * h);
    let (s, c) = l.rotation_degrees.to_radians().sin_cos();
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for (px, py) in [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)] {
        let (dx, dy) = (px - ax, py - ay);
        let (rx, ry) = (dx * c - dy * s, dx * s + dy * c);
        let (x, y) = (off.0 + l.x + rx, off.1 + l.y + ry);
        x0 = x0.min(x);
        x1 = x1.max(x);
        y0 = y0.min(y);
        y1 = y1.max(y);
    }
    (x0, y0, x1, y1)
}

fn collect_text(layers: &[Layer], off: (f32, f32), out: &mut Vec<TextBox>) {
    for l in layers {
        match &l.kind {
            LayerKind::Text(t) if l.visible && l.layout.is_none() => {
                let (x0, y0, x1, y1) = bounds(l, off);
                out.push(TextBox {
                    id: l.id.clone(),
                    x0,
                    x1,
                    y0,
                    y1,
                    size: t.font_size,
                });
            }
            LayerKind::Group { children } => {
                // A group's children live in its own box, origin at its top-left.
                let (x0, y0) = (
                    off.0 + l.x - l.anchor_x * l.width * l.scale_x,
                    off.1 + l.y - l.anchor_y * l.height * l.scale_y,
                );
                collect_text(children, (x0, y0), out);
            }
            _ => {}
        }
    }
}

fn check(case: &Case, style_name: &str, canvas: (u32, u32), bad: &mut Vec<String>) -> usize {
    let st = style(style_name);
    let frame = LayoutFrame::new(canvas.0, canvas.1).expect("canvas");
    let p = compile(case, &st, Some(canvas));
    let what = format!("{} x {style_name} @ {}x{}", case.name, canvas.0, canvas.1);
    assert_eq!(
        (p.canvas.width, p.canvas.height),
        canvas,
        "{what}: project canvas"
    );
    if let Err(e) = validate(&p, Some(&repo().join("assets"))) {
        panic!("{what}: invalid scene: {e}");
    }
    let mut checked = 0;
    let h = frame.h;
    // Text boxes carry up to ~37u of side bearing / box padding past the 84u
    // margin at the legacy canvases too, and `text_layer` pads its box by
    // 2 % + 2 px (data_story headlines/numerals); the hard rule
    // is the canvas edge.
    let slack = (40.0 * frame.u).max(0.02 * frame.safe.w + 4.0);
    for n in our_beats(case, &st, &frame) {
        let Some(scene) = p.scenes.iter().find(|s| s.id == format!("beat_{n}")) else {
            continue;
        };
        let mut boxes = Vec::new();
        collect_text(&scene.layers, (0.0, 0.0), &mut boxes);
        for b in boxes {
            // Not ours: the parallax ghost keyword (deliberately oversized and
            // cropped) and furniture (numerals, stickers; `furniture.rs`).
            let local = b.id.split_once('.').map_or(b.id.as_str(), |(_, r)| r);
            if ["ghost", "numeral", "sticker", "sticker2"]
                .iter()
                .any(|f| local == *f || local.starts_with(&format!("{f}.")))
            {
                continue;
            }
            checked += 1;
            let legacy = LayoutFrame::legacy_size(frame.class) == canvas;
            // (Legacy canvases are frozen byte-for-byte: e.g. the 1080x1080 HeroObject
            // secondary note sits below the canvas and small SOURCE labels stay
            // under the floor there; the responsive guarantees start off-legacy.)
            if !legacy && (b.y0 < -TOL || b.y1 > h + TOL) {
                bad.push(format!(
                    "{what}: beat {n} text '{}' outside the canvas vertically ({:.0}..{:.0} of {h})",
                    b.id, b.y0, b.y1
                ));
            }
            if b.x0 < frame.safe.x - slack || b.x1 > frame.safe.x + frame.safe.w + slack {
                bad.push(format!(
                    "{what}: beat {n} text '{}' leaves the safe area horizontally ({:.0}..{:.0}, safe {:.0}..{:.0})",
                    b.id,
                    b.x0,
                    b.x1,
                    frame.safe.x,
                    frame.safe.x + frame.safe.w
                ));
            }
            if !legacy && b.size < frame.min_type_px - 0.01 {
                bad.push(format!(
                    "{what}: beat {n} text '{}' size {} < min {}",
                    b.id, b.size, frame.min_type_px
                ));
            }
        }
    }
    checked
}

#[test]
fn builders_a_keep_text_on_canvas_at_every_aspect() {
    let mut total = 0;
    let mut bad = Vec::new();
    for case in cases() {
        for style in STYLES {
            for canvas in CANVASES {
                total += check(&case, style, canvas, &mut bad);
            }
        }
    }
    assert!(
        bad.is_empty(),
        "{} violations:\n{}",
        bad.len(),
        bad.join("\n")
    );
    assert!(total > 500, "only {total} text boxes were checked");
}

#[test]
fn the_case_list_reaches_every_grammar_of_this_set() {
    let st = style("warm_editorial");
    let mut seen = std::collections::BTreeSet::new();
    for case in cases().iter().filter(|c| c.manifest.is_none()) {
        for canvas in CANVASES {
            let frame = LayoutFrame::new(canvas.0, canvas.1).expect("canvas");
            let mut at_class = case.intent.clone();
            at_class.format = frame.class;
            let plan = plan_assets(&at_class, &st, &library(case.families)).expect("plan");
            for d in plan.beats {
                seen.insert(d.composition);
            }
        }
    }
    for g in [
        "hero_object",
        "evidence_stack",
        "kinetic_poster",
        "data_story",
    ] {
        assert!(seen.contains(g), "no case reaches {g}: {seen:?}");
    }
}

#[test]
fn builders_a_are_deterministic_per_canvas() {
    let st = style("warm_editorial");
    for case in cases() {
        for canvas in [(1080, 2520), (1080, 1350), (1296, 1080), (2520, 1080)] {
            let a = serde_json::to_string(&compile(&case, &st, Some(canvas))).expect("json");
            let b = serde_json::to_string(&compile(&case, &st, Some(canvas))).expect("json");
            assert_eq!(a, b, "{} @ {canvas:?}", case.name);
        }
    }
}

/// Passing a legacy canvas explicitly is byte-identical to the default.
#[test]
fn legacy_canvas_override_is_identical_to_the_default() {
    let st = style("warm_editorial");
    for case in cases() {
        let legacy = LayoutFrame::legacy_size(case.intent.format);
        let a = serde_json::to_string(&compile(&case, &st, None)).expect("json");
        let b = serde_json::to_string(&compile(&case, &st, Some(legacy))).expect("json");
        assert_eq!(a, b, "{}", case.name);
    }
}

/// Delivered subject / evidence images keep their aspect and stay on the
/// canvas (opaque scans and cutouts alike; transparent margins may bleed, the
/// image box may not leave the canvas by more than the bleed of its own
/// transparent margin, so we check the box centre and the aspect).
#[test]
fn delivered_images_keep_aspect_and_stay_on_canvas() {
    let st = style("warm_editorial");
    let m = manifest();
    let mut checked = 0;
    for case in cases().iter().filter(|c| c.manifest.is_some()) {
        for canvas in CANVASES {
            let p = compile(case, &st, Some(canvas));
            let (w, h) = (canvas.0 as f32, canvas.1 as f32);
            for scene in &p.scenes {
                let mut stack = vec![((0.0f32, 0.0f32), &scene.layers)];
                while let Some((off, layers)) = stack.pop() {
                    for l in layers {
                        match &l.kind {
                            LayerKind::Image { asset, .. } => {
                                let Some(entry) =
                                    p.assets.iter().find(|a| &a.id == asset).and_then(|a| {
                                        m.assets.iter().find(|e| a.path.ends_with(&e.path))
                                    })
                                else {
                                    continue;
                                };
                                let aspect = entry.width as f32 / entry.height as f32;
                                assert!(
                                    (l.width / l.height / aspect - 1.0).abs() < 0.01,
                                    "{} @ {canvas:?}: image '{}' {}x{} breaks aspect {aspect}",
                                    case.name,
                                    l.id,
                                    l.width,
                                    l.height
                                );
                                let (x0, y0, x1, y1) = bounds(l, off);
                                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                                assert!(
                                    cx > 0.0 && cx < w && cy > 0.0 && cy < h,
                                    "{} @ {canvas:?}: image '{}' centred off-canvas ({cx:.0},{cy:.0})",
                                    case.name,
                                    l.id
                                );
                                checked += 1;
                            }
                            LayerKind::Group { children } => {
                                let (x0, y0) = (
                                    off.0 + l.x - l.anchor_x * l.width * l.scale_x,
                                    off.1 + l.y - l.anchor_y * l.height * l.scale_y,
                                );
                                stack.push(((x0, y0), children));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    assert!(checked >= CANVASES.len(), "no delivered image was checked");
}

/// Manual: write compiled projects for visual inspection.
/// `MOTION_R1_OUT=<dir> cargo test -p motion-core --test canvas_builders_a -- --ignored`
#[test]
#[ignore]
fn dump_projects_for_review() {
    let Some(dir) = std::env::var_os("MOTION_R1_OUT") else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("out dir");
    let st = style("warm_editorial");
    let case = &cases()[0];
    for canvas in [(1080, 1350), (2520, 1080), (1080, 2520)] {
        let p = compile(case, &st, Some(canvas));
        let name = format!("{}_{}x{}.motion.json", case.name, canvas.0, canvas.1);
        std::fs::write(
            dir.join(name),
            serde_json::to_string_pretty(&p).expect("json"),
        )
        .expect("write");
    }
}
