//! (v2.2) Type styles on Title / KineticText: fill (solid, gradient, image),
//! outline, letter case, and the `xl` title measure. Lowering only; pixel
//! tests live in `motion-render/tests/render_core/type_styles.rs`.

use std::path::PathBuf;

use motion_core::scene::{Gradient, LayerKind, MotionProject, TextFill, TextStyle};
use motion_program::program::MotionProgram;
use motion_program::{lower, validate, Typesetter};

fn root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// Lowers with real type measurement (the renderer's shaping).
fn lower_measured(p: &MotionProgram) -> MotionProject {
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
    lower(p, Some(assets.to_string_lossy().into_owned()), &ts)
}

fn program(aspect: &str, components: &str) -> MotionProgram {
    let json = format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "aspect": "{aspect}", "duration": 4}},
        "palette": {{"background": "#0B0D12", "ink": "#F1EEE6", "accent": "#FFB36B", "muted": "#5A6070"}},
        "components": [{components}]}}"##
    );
    let p = MotionProgram::from_json(&json).expect("parses");
    validate(&p).expect("valid");
    p
}

/// The text layer `id` (top level) with its box width.
fn text<'a>(scene: &'a MotionProject, id: &str) -> (f32, &'a TextStyle) {
    for s in &scene.scenes {
        for l in &s.layers {
            if l.id == id {
                if let LayerKind::Text(t) = &l.kind {
                    return (l.width, t);
                }
            }
        }
    }
    panic!("no text layer {id}");
}

fn title(id: &str, preset: &str, text: &str, size: &str, style: &str) -> String {
    format!(
        r#"{{"id": "{id}", "kind": "title", "preset": "{preset}", "text": "{text}",
            "size": "{size}", "slot": "center" {style}}}"#
    )
}

#[test]
fn xl_title_spans_at_least_55_percent_of_the_frame() {
    for (aspect, w) in [("16:9", 1920.0), ("9:16", 1080.0)] {
        for preset in ["display", "serif", "chapter", "stacked"] {
            for word in ["Glow", "Brands", "Sunrise", "Northern"] {
                let p = program(aspect, &title("t", preset, word, "xl", ""));
                let scene = lower_measured(&p);
                let width = if preset == "stacked" {
                    // The stacked title is a group; its type child is `t.type`.
                    scene.scenes[0]
                        .layers
                        .iter()
                        .find(|l| l.id == "t")
                        .and_then(|g| match &g.kind {
                            LayerKind::Group { children } => {
                                children.iter().find(|c| c.id == "t.type").map(|c| c.width)
                            }
                            _ => None,
                        })
                        .expect("stacked type")
                } else {
                    text(&scene, "t").0
                };
                assert!(
                    width / w >= 0.55,
                    "{aspect} {preset} '{word}': line spans {:.0} % of the frame",
                    width / w * 100.0
                );
                assert!(width <= w * 0.9, "{aspect} {preset} '{word}' too wide");
            }
        }
    }
}

#[test]
fn style_absent_is_identical_to_the_default_style() {
    for name in ["one_shape", "board_reel", "coffee_story"] {
        let json = std::fs::read_to_string(root().join(format!("examples/v2/{name}.program.json")))
            .expect("sample");
        let plain = MotionProgram::from_json(&json).expect("parses");
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut n = 0;
        for c in v["components"].as_array_mut().unwrap() {
            if matches!(c["kind"].as_str(), Some("title" | "kinetic_text")) {
                c["style"] = serde_json::json!({"fill": {"kind": "solid"}, "case": "preset"});
                n += 1;
            }
        }
        assert!(n > 0, "{name} has type");
        let styled = MotionProgram::from_json(&v.to_string()).expect("styled parses");
        let ts = Typesetter::approx();
        let a = serde_json::to_string(&lower(&plain, None, &ts)).unwrap();
        let b = serde_json::to_string(&lower(&styled, None, &ts)).unwrap();
        assert_eq!(a, b, "{name}: a default style changed the scene");
        assert!(
            !a.contains("\"outline\"") && !a.contains("\"fill\":{\"image\""),
            "{name}: no styled-type fields when no style is set"
        );
    }
}

#[test]
fn case_overrides_the_preset() {
    let p = program(
        "16:9",
        &[
            title(
                "a",
                "chapter",
                "Made in Studio",
                "l",
                r#", "style": {"case": "as_written"}"#,
            ),
            title(
                "b",
                "display",
                "loud",
                "l",
                r#", "style": {"case": "upper"}"#,
            ),
            title("c", "chapter", "default", "l", ""),
        ]
        .join(","),
    );
    let scene = lower(&p, None, &Typesetter::approx());
    assert!(!text(&scene, "a").1.uppercase);
    assert!(text(&scene, "b").1.uppercase);
    assert!(text(&scene, "c").1.uppercase, "preset case unchanged");
    let k = program(
        "16:9",
        r#"{"id": "k", "kind": "kinetic_text", "words": ["Hello"], "style": {"case": "as_written"}}"#,
    );
    let scene = lower(&k, None, &Typesetter::approx());
    assert!(!text(&scene, "k.w0.text").1.uppercase);
}

#[test]
fn image_fill_is_a_text_mask_over_a_registered_asset() {
    let p = program(
        "16:9",
        &title(
            "t",
            "display",
            "Sunrise",
            "xl",
            r#", "style": {"fill": {"kind": "image", "image": "library/grounds/sky_gradient.png"}}"#,
        ),
    );
    let scene = lower(&p, None, &Typesetter::approx());
    let id = "type.library/grounds/sky_gradient.png";
    assert_eq!(text(&scene, "t").1.fill, Some(TextFill::Image(id.into())));
    let a = scene.assets.iter().find(|a| a.id == id).expect("asset");
    assert_eq!(a.path, "library/grounds/sky_gradient.png");
}

#[test]
fn image_fill_with_a_missing_file_is_a_validation_error() {
    for (img, needle) in [
        ("library/grounds/no_such_plate.png", "no image at"),
        ("../secrets.png", "relative to assets"),
        ("library/grounds/catalog.json", "relative to assets"),
    ] {
        let json = format!(
            r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 4}},
            "palette": {{"background": "#0B0D12", "ink": "#F1EEE6", "accent": "#FFB36B", "muted": "#5A6070"}},
            "components": [{}]}}"##,
            title(
                "t",
                "display",
                "Word",
                "l",
                &format!(r#", "style": {{"fill": {{"kind": "image", "image": "{img}"}}}}"#)
            )
        );
        let p = MotionProgram::from_json(&json).expect("parses");
        let e = validate(&p)
            .expect_err("missing image must not validate")
            .to_string();
        assert!(
            e.contains("style.fill.image") && e.contains(needle),
            "{img}: {e}"
        );
    }
}

#[test]
fn gradient_runs_across_the_line_box_and_outline_strokes() {
    let p = program(
        "16:9",
        &[
            title(
                "g",
                "display",
                "Glow",
                "xl",
                r#", "style": {"fill": {"kind": "gradient", "from": "accent", "to": "ink"}}"#,
            ),
            title(
                "v",
                "display",
                "Down",
                "l",
                r#", "style": {"fill": {"kind": "gradient", "from": "accent", "to": "ink", "angle_deg": 90}}"#,
            ),
            title("o", "display", "Hollow", "l", r#", "style": {"outline": true}"#),
        ]
        .join(","),
    );
    let scene = lower(&p, None, &Typesetter::approx());
    let (w, g) = text(&scene, "g");
    let Some(TextFill::Gradient(Gradient::Linear { from, to, stops })) = &g.fill else {
        panic!("gradient fill expected: {:?}", g.fill);
    };
    assert_eq!(stops.len(), 2);
    assert_ne!(stops[0].color, stops[1].color);
    assert!(from[0] <= 0.5 && (to[0] - w).abs() <= 0.5 && from[1] == to[1]);
    let Some(TextFill::Gradient(Gradient::Linear { from, to, .. })) = &text(&scene, "v").1.fill
    else {
        panic!("vertical gradient expected");
    };
    assert!((from[0] - to[0]).abs() < 0.5 && to[1] > from[1]);
    let o = text(&scene, "o").1;
    assert!(o.fill.is_none());
    assert!(o.outline.is_some_and(|w| w >= 2.0 && w < o.font_size * 0.1));
}

/// Design gates on real frames (the same harness as `tests/design.rs`).
fn design_fails(p: &MotionProgram) -> Vec<String> {
    use motion_program::qa::design::{self, Pixels};
    use motion_render::Renderer;
    let scene = lower_measured(p);
    let r = motion_render::CpuRenderer::new(&scene, &root()).expect("renderer");
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
    design::run(p, &scene, 2.0, &mut render)
        .expect("detectors run")
        .into_iter()
        .map(|f| format!("{} {}", f.id, f.detail))
        .collect()
}

fn sky_word_on(background: &str, ink: &str) -> MotionProgram {
    let json = format!(
        r##"{{"version": "2.0", "meta": {{"name": "t", "duration": 2}},
        "palette": {{"background": "{background}", "ink": "{ink}", "accent": "#FF5A1F", "muted": "#8E877B"}},
        "components": [{}]}}"##,
        title(
            "t",
            "display",
            "Sunrise",
            "xl",
            r#", "style": {"fill": {"kind": "image", "image": "library/grounds/sky_gradient.png"}}"#,
        )
    );
    let p = MotionProgram::from_json(&json).expect("parses");
    validate(&p).expect("valid");
    p
}

#[test]
fn image_fill_matching_its_ground_fails_ink_contrast() {
    let pale = design_fails(&sky_word_on("#D5DFE4", "#111111"));
    assert!(
        pale.iter().any(|f| f.starts_with("ink_contrast")),
        "a sky photo on a pale sky ground must fail: {pale:?}"
    );
    let cobalt = design_fails(&sky_word_on("#1B3FE8", "#F1EEE6"));
    assert!(
        !cobalt.iter().any(|f| f.starts_with("ink_contrast")),
        "a sky photo on cobalt reads: {cobalt:?}"
    );
}
