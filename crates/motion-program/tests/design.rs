//! The design and colour system (`docs/DESIGN_RULES.md`):
//! the palette resolver's bands, and the mistake ledger's rendered
//! fixtures (`assets/qa/fixtures/*.json`, kind `rendered`): each fails its
//! detector before (the defect as it used to render) and passes after (the
//! same program through the engine). Frames are really rendered.

use std::path::PathBuf;

use motion_core::scene::{Color, Fill, LayerKind, MotionProject};
use motion_program::ledger::GateFail;
use motion_program::palette::{self, contrast, oklch, Family};
use motion_program::program::{MotionProgram, Palette};
use motion_program::qa::design::{self, Pixels};
use motion_program::{lower_with, LowerOptions, Typesetter};
use motion_render::Renderer;

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn hex(s: &str) -> Color {
    Color::parse_hex(s).unwrap()
}

fn pal(bg: &str, ink: &str, accent: &str, muted: &str) -> Palette {
    Palette {
        background: bg.into(),
        ink: ink.into(),
        accent: accent.into(),
        muted: muted.into(),
        grounds: Vec::new(),
    }
}

// ------------------------------------------------------------- resolver

#[test]
fn orange_ground_with_black_dots_gets_dark_orange_texture_and_black_ink() {
    // The reported frame: the style asks for black dots and black type on
    // orange. Texture becomes a darker orange; type stays black.
    let r = palette::resolve(&pal("#FF4A1C", "#111111", "#FF4A1C", "#111111"));
    let m = r.main;
    assert_eq!(m.family, Family::Saturated);
    let (g, t, i) = (oklch(m.ground), oklch(m.texture), oklch(m.ink));
    assert!(t.l < g.l, "texture is a shade: {t:?} vs {g:?}");
    assert!(palette::hue_gap(t.h, g.h) < 8.0, "same hue: {t:?} vs {g:?}");
    assert!(
        t.c > 0.1,
        "texture keeps the ground's colour, not grey: {t:?}"
    );
    assert!(i.l <= 0.16, "ink is near-black: {i:?}");
    assert!((t.l - i.l).abs() >= palette::TEXTURE_INK_DL);
    let band = contrast(m.texture, m.ground);
    assert!(
        (palette::TEXTURE_MIN..=palette::TEXTURE_MAX).contains(&band),
        "{band}"
    );
    assert!(contrast(m.ink, m.ground) >= palette::INK_DISPLAY);
    // The brand orange is kept: display type reads on it.
    assert_eq!(m.ground, hex("#FF4A1C"));
    // The accent shares the ground's hue: it becomes the opposite value.
    assert_eq!(m.accent, m.ink);
    assert!(
        r.repairs.iter().any(|x| x.contains("muted")),
        "{:?}",
        r.repairs
    );
}

#[test]
fn every_family_meets_the_bands() {
    for (bg, ink, accent, muted) in [
        ("#0B0E14", "#F2EFE8", "#FF5A1F", "#4A5263"), // dark navy
        ("#F2EEE6", "#111111", "#FF4D00", "#8A857C"), // light paper
        ("#3B2BD6", "#111111", "#FF4A1C", "#111111"), // saturated blue
        ("#FF4A1C", "#F2EFE8", "#3B2BD6", "#F2EFE8"), // orange, light ink asked
        ("#7A7A7A", "#7F7F7F", "#808080", "#7A7A7A"), // everything mid grey
    ] {
        let m = palette::resolve(&pal(bg, ink, accent, muted)).main;
        let tag = format!("{bg} {ink} {accent}");
        let band = contrast(m.texture, m.ground);
        assert!(
            (palette::TEXTURE_MIN..=palette::TEXTURE_MAX).contains(&band),
            "{tag}: texture {band}"
        );
        assert!(
            (oklch(m.texture).l - oklch(m.ink).l).abs() >= palette::TEXTURE_INK_DL,
            "{tag}: texture/ink dL"
        );
        // Saturated grounds carry display type (§2); the rest small type.
        let need = if m.family == Family::Saturated {
            palette::INK_DISPLAY
        } else {
            palette::INK_SMALL
        };
        assert!(contrast(m.ink, m.ground) >= need, "{tag}: ink");
        assert!(
            contrast(m.ink_secondary, m.ground) >= need,
            "{tag}: secondary ink"
        );
        assert!(
            contrast(m.ink_secondary, m.ground) <= contrast(m.ink, m.ground),
            "{tag}: ladder"
        );
        let s = contrast(m.structure, m.ground);
        assert!((1.5..=3.0).contains(&s), "{tag}: structure {s}");
        assert!(
            contrast(m.structure_strong, m.ground) >= 3.0,
            "{tag}: strong structure"
        );
        assert!(
            contrast(m.accent, m.ground) >= palette::ACCENT_MIN,
            "{tag}: accent"
        );
        assert!(
            contrast(m.accent_text, m.ground) >= palette::INK_DISPLAY,
            "{tag}: accent type"
        );
    }
}

#[test]
fn resolver_is_deterministic_and_keeps_valid_palettes() {
    let p = pal("#F2EEE6", "#111111", "#C83A00", "#8A857C");
    let (a, b) = (palette::resolve(&p), palette::resolve(&p));
    assert_eq!(a.main, b.main);
    // A valid ink and accent are kept as the author wrote them.
    assert_eq!(a.main.ink, hex("#111111"));
    assert_eq!(a.main.accent, hex("#C83A00"));
    // An accent that sinks into its ground (2.9:1) moves in lightness only.
    let q = palette::resolve(&pal("#F2EEE6", "#111111", "#FF4D00", "#8A857C"));
    assert!(contrast(q.main.accent, q.main.ground) >= palette::ACCENT_MIN);
    assert!(palette::hue_gap(oklch(q.main.accent).h, oklch(hex("#FF4D00")).h) < 6.0);
}

#[test]
fn a_fourth_hue_becomes_a_neutral() {
    let mut p = pal("#F2EEE6", "#111111", "#FF4A1C", "#8A857C");
    p.grounds = vec![
        "#3B2BD6".into(), // blue
        "#18A558".into(), // green
        "#C21DB7".into(), // magenta: the fourth family
    ];
    let r = palette::resolve(&p);
    assert!(oklch(r.grounds[0].ground).c > 0.1);
    assert!(oklch(r.grounds[1].ground).c > 0.1);
    assert!(oklch(r.grounds[2].ground).c < 0.03, "{:?}", r.grounds[2]);
    assert!(r.repairs.iter().any(|x| x.contains("fourth hue")));
}

// ------------------------------------------------------------- fixtures

/// Lowers with real type measurement; `design` toggles the layout passes.
fn lower(p: &MotionProgram, design: bool) -> MotionProject {
    let fonts = p.fonts();
    let assets = root().join("assets");
    let faces: Vec<PathBuf> = [fonts.display, fonts.serif, fonts.mono]
        .iter()
        .map(|f| assets.join(f.asset().1))
        .collect();
    let measure =
        motion_render::FontMeasure::with_fonts(faces.iter().map(PathBuf::as_path)).expect("fonts");
    let ts = Typesetter {
        measure: &measure,
        assets: assets.clone(),
    };
    lower_with(
        p,
        Some(assets.to_string_lossy().into_owned()),
        &ts,
        LowerOptions {
            design_passes: design,
            ..Default::default()
        },
    )
}

fn detect(p: &MotionProgram, scene: &MotionProject) -> Vec<GateFail> {
    let r = motion_render::CpuRenderer::new(scene, &root()).expect("renderer");
    let mut render = |rf: &motion_core::timeline::ResolvedFrame<'_>| {
        let pm = r.render(rf).map_err(|e| e.to_string())?;
        let mut rgb = Vec::with_capacity((pm.width() * pm.height() * 3) as usize);
        for px in pm.pixels() {
            let c = px.demultiply();
            rgb.extend_from_slice(&[c.red(), c.green(), c.blue()]);
        }
        Ok(Pixels {
            w: pm.width(),
            h: pm.height(),
            rgb,
        })
    };
    design::run(p, scene, 2.0, &mut render).expect("detectors run")
}

/// The defect as it used to render: texture in the ink colour.
fn black_dots(scene: &mut MotionProject) {
    for s in &mut scene.scenes {
        for l in &mut s.layers {
            if let LayerKind::Instances { fill, .. } = &mut l.kind {
                *fill = Fill::Solid(hex("#111111"));
            }
        }
    }
}

/// The first fix: dots shrink to nothing in the text *box* + 0.25 em over
/// 1 em, on a field whose dots are larger than that ramp.
fn box_knockout(scene: &mut MotionProject) {
    let mut boxes = Vec::new();
    for s in &scene.scenes {
        for l in &s.layers {
            if let LayerKind::Text(st) = &l.kind {
                let (x0, y0) = (l.x - l.anchor_x * l.width, l.y - l.anchor_y * l.height);
                let pad = st.font_size * 0.25;
                boxes.push((
                    [x0 - pad, y0 - pad, x0 + l.width + pad, y0 + l.height + pad],
                    st.font_size,
                ));
            }
        }
    }
    for s in &mut scene.scenes {
        for l in &mut s.layers {
            let (ox, oy) = (l.x - l.anchor_x * l.width, l.y - l.anchor_y * l.height);
            if let LayerKind::Instances { at, .. } = &mut l.kind {
                for a in at.iter_mut() {
                    let (px, py) = (ox + a[0], oy + a[1]);
                    for (r, em) in &boxes {
                        let dx = (r[0] - px).max(px - r[2]).max(0.0);
                        let dy = (r[1] - py).max(py - r[3]).max(0.0);
                        let u = ((dx.hypot(dy) - a[2]) / em).clamp(0.0, 1.0);
                        a[2] *= u * u * (3.0 - 2.0 * u);
                    }
                }
            }
        }
    }
}

/// Adds a layer to the first scene.
fn add_layer(scene: &mut MotionProject, kind: LayerKind, centre: [f32; 2], size: [f32; 2], z: i32) {
    let mut l = scene.scenes[0].layers[0].clone();
    l.id = "fixture".into();
    l.kind = kind;
    (l.x, l.y, l.width, l.height) = (centre[0], centre[1], size[0], size[1]);
    (l.anchor_x, l.anchor_y) = (0.5, 0.5);
    l.z_index = z;
    l.depth = None;
    scene.scenes[0].layers.push(l);
}

fn move_text(scene: &mut MotionProject, id: &str, x: impl Fn(f32) -> f32) {
    for s in &mut scene.scenes {
        for l in &mut s.layers {
            if l.id == id {
                l.x = x(l.width);
            }
        }
    }
}

fn mutate(name: &str, scene: &mut MotionProject) {
    match name {
        "none" => {}
        "black_dots" => black_dots(scene),
        "box_knockout" => box_knockout(scene),
        "accent_slab" => add_layer(
            scene,
            LayerKind::Rectangle {
                fill: hex("#FF4D00"),
                stroke: None,
            },
            [480.0, 540.0],
            [960.0, 760.0],
            5,
        ),
        "grey_type_black_hairline" => {
            for s in &mut scene.scenes {
                for l in &mut s.layers {
                    if let LayerKind::Text(st) = &mut l.kind {
                        st.color = hex("#8A857C");
                    }
                }
            }
            add_layer(
                scene,
                LayerKind::Path {
                    d: "M0 0H1600M0 120H1600".into(),
                    fill: None,
                    stroke: Some(motion_core::scene::Stroke {
                        color: hex("#111111"),
                        width: 2.0,
                        brush: None,
                    }),
                    fill_rule: Default::default(),
                    shadows: Vec::new(),
                },
                [960.0, 520.0],
                [1600.0, 120.0],
                1,
            );
        }
        "raw_grounds" => {
            let raw = ["#3B2BD6", "#18A558", "#C21DB7", "#E8C21A"];
            for (k, s) in scene.scenes.iter_mut().take(4).enumerate() {
                if let LayerKind::Rectangle { fill, .. } = &mut s.layers[0].kind {
                    *fill = hex(raw[k]);
                }
            }
        }
        "texture_on_top" => {
            for sc in &mut scene.scenes {
                for l in &mut sc.layers {
                    if matches!(l.kind, LayerKind::Instances { .. }) {
                        l.z_index = 100_000;
                    }
                }
            }
        }
        "edge_text" => move_text(scene, "head", |w| w / 2.0 + 8.0),
        "off_frame_text" => move_text(scene, "head", |_| 1920.0),
        other => panic!("unknown mutation {other}"),
    }
}

/// Every rendered fixture (`assets/qa/fixtures/*.json`, kind `rendered`)
/// fails its ledger detector before and passes every design detector
/// after; together they cover each detector, and every id is a ledger
/// entry.
#[test]
fn rendered_fixtures_fail_then_pass() {
    use motion_program::ledger::ledger;
    let ids: Vec<String> = ledger().entries.into_iter().map(|e| e.id).collect();
    let mut covered: Vec<String> = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root().join("assets/qa/fixtures"))
        .expect("fixtures")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    paths.sort();
    for path in paths {
        let fx: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).unwrap();
        if fx["kind"] != "rendered" {
            continue;
        }
        let id = fx["expect_detector"]
            .as_str()
            .expect("expect_detector")
            .to_string();
        assert!(
            design::IDS.contains(&id.as_str()),
            "{id}: not a design detector"
        );
        assert!(ids.contains(&id), "{id}: not in the ledger");
        let p: MotionProgram = serde_json::from_value(fx["program"].clone()).expect("program");
        motion_program::validate(&p).expect("fixture program is valid");
        let mut before = lower(&p, fx["before"]["design_passes"].as_bool().unwrap());
        mutate(fx["before"]["mutate"].as_str().unwrap(), &mut before);
        let b = detect(&p, &before);
        assert!(
            b.iter().any(|f| f.id == id),
            "{}: must fail before; got {b:?}",
            path.display()
        );
        let a = detect(&p, &lower(&p, true));
        assert!(
            a.is_empty(),
            "{}: must pass after; got {a:?}",
            path.display()
        );
        covered.push(id);
    }
    for id in design::IDS {
        assert!(covered.iter().any(|c| c == id), "{id}: no rendered fixture");
    }
}

/// The repair hook turns detector failures into program edits.
#[test]
fn design_repair_edits_the_program() {
    use motion_program::repair::repair;
    let mut p: MotionProgram = serde_json::from_value(serde_json::json!({
        "version": "2.0",
        "meta": { "name": "rp", "duration": 2.0 },
        "palette": { "background": "#F2EEE6", "ink": "#111111", "accent": "#FF4D00", "muted": "#8A857C" },
        "components": [
            { "id": "g", "kind": "backdrop" },
            { "id": "tone", "kind": "field", "preset": "halftone" },
            { "id": "head", "kind": "title", "text": "Hello", "colour": "muted", "size": "xl" },
            { "id": "big", "kind": "shape", "shape": "circle", "size": "xl", "slot": "trail", "timeline": [{ "at": 0.5, "do": "morph", "to": "square" }] }
        ]
    }))
    .unwrap();
    let f = |id: &str, d: &str| GateFail {
        id: id.into(),
        detail: d.into(),
    };
    assert!(repair(
        &mut p,
        &[
            f("ink_contrast", "'Hello' 3.1:1, limit 4.5:1 at 0.5s (head)"),
            f("busy_under_type", "'Hello' L sd 0.09 at 0.5s (head)"),
            f("accent_area", "accent covers 14 %"),
            f("safe_inset", "'Hello' 12px outside at 0.5s (head)"),
        ]
    ));
    let get = |id: &str| {
        p.components
            .iter()
            .find(|c| c.id == id)
            .unwrap()
            .body
            .clone()
    };
    use motion_program::program::{ComponentBody, FieldPreset, Size};
    assert!(matches!(
        get("head"),
        ComponentBody::Title {
            colour: None,
            size: Size::L,
            ..
        }
    ));
    assert!(matches!(
        get("tone"),
        ComponentBody::Field {
            preset: FieldPreset::Grid,
            ..
        }
    ));
    // A large accent shape turns ink (it keeps its size: a smaller hero
    // would fail `small_composition`).
    assert!(matches!(
        get("big"),
        ComponentBody::Shape {
            size: Size::Xl,
            colour: Some(motion_program::program::Swatch::Ink),
            ..
        }
    ));
}
