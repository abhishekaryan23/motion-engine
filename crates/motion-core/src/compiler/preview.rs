//! Style preview (0.6): a single static frame showing what a StyleProfile
//! resolves to — palette swatches, type pairing specimen, background grammar,
//! material — plus a textual summary of the temporal character (motion
//! temperament, transition character, density, composition rhythm, scale
//! contrast). No animation: temporal traits are described, not played.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use super::taste::{self, ResolvedStyleProfile};
use super::typeset::{text_layer, Block};
use super::{
    backdrop, plan_timing_with, rect_layer, round3, temporal, AssetLibrary, BeatPlan, Ctx, FontSet,
    TextMeasure, Typesetter, Voice,
};
use crate::assets::AssetManifest;
use crate::intent::CreativeIntent;
use crate::scene::{
    Asset, AssetKind, Canvas, Color, Layer, LayerKind, MotionProject, ProjectMeta, Scene, Stroke,
    TextAlign, Theme, SCENE_VERSION,
};
use crate::style::StyleProfile;

const W: f32 = 1080.0;
const H: f32 = 1920.0;
/// Card inset from the canvas edge, and content inset inside the card.
const CARD_X: f32 = 60.0;
const PAD: f32 = 48.0;
const SWATCH_COLS: usize = 5;
const SWATCH_GAP: f32 = 16.0;
const CHIP_H: f32 = 120.0;
/// Vertical pitch of one swatch row (chip + name + hex + gap).
const CHIP_ROW: f32 = CHIP_H + 78.0;

/// A one-scene, one-second project previewing `style` (render frame 0).
/// Returns the project and the resolved profile it shows.
pub fn style_preview(
    style: &StyleProfile,
    library: &AssetLibrary,
    measure: &dyn TextMeasure,
) -> (MotionProject, ResolvedStyleProfile) {
    let taste = taste::resolve(style);
    let effective = &taste.effective;
    let fonts = FontSet::for_pairing(taste.typography);
    let ts = Typesetter::new(measure, &library.root, &fonts)
        .with_scale(temporal::scale_gains(taste.scale));
    let manifest = AssetManifest::empty();
    let mut ctx = Ctx {
        reveals: Default::default(),
        warnings: Vec::new(),
        direction_seed: None,
        beat_params: BTreeMap::new(),
        direction_take: None,
        direction_beats: BTreeMap::new(),
        emotion: None,
        art: None,
        w: W,
        h: H,
        u: 1.0,
        style: effective,
        taste: taste.clone(),
        palette: taste.palette.colors.clone(),
        ts,
        library,
        assets: BTreeMap::new(),
        format: crate::intent::Format::Vertical,
        frame: crate::compiler::layout_frame::LayoutFrame::for_format(
            crate::intent::Format::Vertical,
        ),
        manifest: &manifest,
        image_carry: BTreeSet::new(),
        focal: BTreeMap::new(),
        stack: None,
        spoken: Vec::new(),
        beat_count: 1,
        sequence: Default::default(),
    };
    for face in &fonts.faces {
        ctx.assets.insert(
            face.asset_id.to_string(),
            Asset {
                id: face.asset_id.to_string(),
                kind: AssetKind::Font,
                path: face.path.to_string(),
                sprite: None,
            },
        );
    }

    // Backdrops that recompose per beat see one representative beat.
    let plans: Vec<BeatPlan> = synthetic_intent()
        .map(|intent| plan_timing_with(&intent, &taste))
        .unwrap_or_default();
    let scenes = vec![
        backdrop::backdrop_scene(&ctx, &plans, 1.0),
        specimen_scene(&ctx),
    ];

    let theme = Theme {
        fonts: fonts
            .roles
            .iter()
            .map(|(role, face)| (*role, face.asset_id.to_string()))
            .collect(),
        palette: ctx.palette.named(),
        typography: None,
    };
    let project = MotionProject {
        envelopes: Vec::new(),
        version: SCENE_VERSION.to_string(),
        project: ProjectMeta {
            name: "style_preview".into(),
            duration_seconds: Some(1.0),
            exploration: None,
            speech: None,
            art: None,
            direction: None,
        },
        canvas: Canvas {
            width: W as u32,
            height: H as u32,
            fps: 30,
            background: ctx.palette.paper,
        },
        theme,
        assets: ctx.assets.into_values().collect(),
        asset_root: None,
        scenes,
        shared: vec![],
    };
    (project, taste)
}

/// A one-beat intent used only to plan a representative beat for backdrops.
fn synthetic_intent() -> Option<CreativeIntent> {
    CreativeIntent::from_json(
        r#"{
          "version": "0.1",
          "title": "style_preview",
          "format": "vertical",
          "beats": [{
            "purpose": "emphasize",
            "statement": "Taste is temporal",
            "primary": { "kind": "number", "value": "42%", "meaning": "sample" },
            "energy": "calm"
          }]
        }"#,
    )
    .ok()
}

/// Serde snake_case name of a unit-like enum value.
fn name<T: Serialize>(v: &T) -> String {
    match serde_json::to_value(v) {
        Ok(serde_json::Value::String(s)) => s,
        Ok(other) => other.to_string(),
        Err(_) => String::from("?"),
    }
}

fn put(layers: &mut Vec<Layer>, id: String, block: &Block, color: Color, x: f32, y: f32) {
    let mut l = text_layer(id, block, color, TextAlign::Left);
    l.x = x;
    l.y = y;
    l.z_index = 10;
    layers.push(l);
}

/// The one static scene: a card carrying the type specimen and swatch row,
/// centered vertically so the backdrop stays visible above and below.
fn specimen_scene(ctx: &Ctx) -> Scene {
    let pal = &ctx.palette;
    let taste = &ctx.taste;
    let x0 = CARD_X + PAD;
    let inner_w = W - 2.0 * (CARD_X + PAD);
    let mut layers: Vec<Layer> = Vec::new();
    let mut n = 0usize;
    let mut id = |prefix: &str| {
        n += 1;
        format!("preview.{prefix}{n}")
    };
    let mut y = PAD;

    // Title (kicker voice) + accent rule.
    let title = ctx
        .ts
        .fit_line(Voice::KICKER, "STYLE PREVIEW", inner_w, 28.0);
    put(&mut layers, id("title"), &title, pal.muted, x0, y);
    y += title.height() + 14.0;
    layers.push(rect_layer(id("rule"), (x0, y, 96.0, 6.0), pal.accent, 10));
    y += 6.0 + 44.0;

    // Headline sample.
    let headline = ctx.ts.fit_block(
        Voice::HEADLINE,
        "Taste is temporal",
        inner_w,
        330.0,
        170.0,
        2,
    );
    put(&mut layers, id("headline"), &headline, pal.ink, x0, y);
    y += headline.height() + 28.0;

    // Serif aside.
    let aside = ctx
        .ts
        .fit_line(Voice::SERIF, "a system, not a filter", inner_w, 74.0);
    put(&mut layers, id("aside"), &aside, pal.accent, x0, y);
    y += aside.height() + 24.0;

    // Body line.
    let body = ctx.ts.set_wrapped(
        Voice::BODY,
        "One style resolves palette, type, material and motion together.",
        34.0,
        inner_w,
    );
    put(&mut layers, id("body"), &body, pal.ink, x0, y);
    y += body.height() + 40.0;

    // Hero number with its label beside it.
    let number = ctx
        .ts
        .fit_line(Voice::HERO_NUMBER, "42%", inner_w * 0.5, 250.0);
    put(&mut layers, id("number"), &number, pal.ink, x0, y);
    let label_x = x0 + number.width() + 28.0;
    let label_w = (x0 + inner_w - label_x).max(60.0);
    let label = ctx
        .ts
        .set_wrapped(Voice::LABEL, "sample figure", 26.0, label_w);
    let label_y = y + number.height() * 0.5 - label.height() * 0.5;
    put(
        &mut layers,
        id("label"),
        &label,
        pal.muted,
        label_x,
        label_y,
    );
    y += number.height() + 44.0;

    // Swatches: chip + name + hex under each.
    let mut chips: Vec<(String, Color)> = vec![
        ("paper".into(), pal.paper),
        ("card".into(), pal.card),
        ("ink".into(), pal.ink),
        ("muted".into(), pal.muted),
        ("accent".into(), pal.accent),
        ("on_accent".into(), pal.on_accent),
    ];
    chips.extend(
        pal.fields
            .iter()
            .enumerate()
            .map(|(i, c)| (format!("field_{}", i + 1), *c)),
    );
    let cell_w = (inner_w - SWATCH_GAP * (SWATCH_COLS as f32 - 1.0)) / SWATCH_COLS as f32;
    for (i, (chip_name, color)) in chips.iter().enumerate() {
        let (col, row) = (i % SWATCH_COLS, i / SWATCH_COLS);
        let cx = x0 + col as f32 * (cell_w + SWATCH_GAP);
        let cy = y + row as f32 * CHIP_ROW;
        let mut chip = rect_layer(id("chip"), (cx, cy, cell_w, CHIP_H), *color, 10);
        chip.kind = LayerKind::Rectangle {
            fill: *color,
            stroke: Some(Stroke {
                color: pal.muted,
                width: 2.0,
            }),
        };
        layers.push(chip);
        let nm = ctx.ts.fit_line(Voice::LABEL, chip_name, cell_w, 20.0);
        let ny = cy + CHIP_H + 10.0;
        put(&mut layers, id("chipname"), &nm, pal.ink, cx, ny);
        let hex = ctx
            .ts
            .fit_line(Voice::KICKER, &color.to_hex(), cell_w, 15.0);
        put(
            &mut layers,
            id("chiphex"),
            &hex,
            pal.muted,
            cx,
            ny + nm.height() + 4.0,
        );
    }
    let rows = chips.len().div_ceil(SWATCH_COLS);
    // Last row: chip, name and hex labels, then a gap before the caption.
    y += (rows - 1) as f32 * CHIP_ROW + CHIP_H + 90.0;

    // Caption: the discrete choices behind what is shown.
    let caption = ctx.ts.fit_line(
        Voice::KICKER,
        &format!(
            "{} / {} / {}",
            name(&taste.typography),
            name(&taste.background),
            name(&taste.material)
        ),
        inner_w,
        18.0,
    );
    put(&mut layers, id("caption"), &caption, pal.muted, x0, y);
    y += caption.height() + PAD;

    // Card behind everything, then center the whole thing vertically.
    let card_h = y;
    let dy = ((H - card_h) * 0.5).max(0.0);
    let mut card = rect_layer(
        "preview.card".into(),
        (CARD_X, 0.0, W - 2.0 * CARD_X, card_h),
        pal.card,
        5,
    );
    card.kind = LayerKind::Rectangle {
        fill: pal.card,
        stroke: Some(Stroke {
            color: pal.ink,
            width: 3.0,
        }),
    };
    layers.insert(0, card);
    for l in &mut layers {
        l.y += dy;
    }

    Scene {
        post: Vec::new(),
        id: "preview".into(),
        start_seconds: 0.0,
        duration_seconds: round3(1.0),
        layers,
        motions: vec![],
        camera: None,
        lifecycle: None,
    }
}

/// Human-readable metadata for a resolved style, one `Label: value` per line.
pub fn describe(resolved: &ResolvedStyleProfile) -> String {
    let r = resolved;
    let mut out = String::new();
    let mut line = |label: &str, value: String| {
        out.push_str(label);
        out.push_str(": ");
        out.push_str(&value);
        out.push('\n');
    };
    line("Tone", name(&r.tone));
    line(
        "Palette",
        format!(
            "{} ({}, {}, {})",
            name(&r.palette.family),
            name(&r.palette.polarity),
            name(&r.palette.temperature),
            name(&r.palette.trajectory)
        ),
    );
    line("Background", name(&r.background));
    line("Typography", name(&r.typography));
    line("Material", name(&r.material));
    line("Image treatment", name(&r.image_treatment));
    let m = &r.motion;
    line(
        "Motion temperament",
        format!(
            "{} (amplitude {}, settle {}, overshoot {}, stagger {}, camera {})",
            name(&m.kind),
            name(&m.amplitude),
            name(&m.settle),
            name(&m.overshoot),
            name(&m.stagger),
            name(&m.camera)
        ),
    );
    let t = &r.transition;
    line(
        "Transition character",
        format!(
            "{} (wipe {}, exit {}, overlap {})",
            name(&t.family),
            name(&t.wipe),
            name(&t.exit),
            name(&t.overlap)
        ),
    );
    line(
        "Density",
        format!(
            "{} (annotation {})",
            name(&r.density.level),
            name(&r.density.annotation)
        ),
    );
    line("Composition rhythm", name(&r.rhythm));
    line("Scale contrast", name(&r.scale));
    line(
        "Layer activity",
        format!(
            "fg {}, mid {}, bg {}",
            name(&r.layers.foreground),
            name(&r.layers.midground),
            name(&r.layers.background)
        ),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler::ApproxMeasure;

    fn styles() -> Vec<(String, StyleProfile)> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/taste");
        let mut out = vec![("{}".to_string(), StyleProfile::default())];
        for entry in std::fs::read_dir(dir).expect("examples/taste") {
            let path = entry.expect("entry").path();
            if path.to_string_lossy().ends_with(".style.json") {
                let text = std::fs::read_to_string(&path).expect("read style");
                let style = StyleProfile::from_json(&text).expect("parse style");
                out.push((path.display().to_string(), style));
            }
        }
        out
    }

    #[test]
    fn preview_validates_for_every_taste_example_and_default() {
        let library = AssetLibrary::new("assets");
        let all = styles();
        assert!(all.len() >= 4);
        for (label, style) in all {
            let (project, resolved) = style_preview(&style, &library, &ApproxMeasure);
            crate::validate(&project, None).unwrap_or_else(|e| panic!("{label}: {e}"));
            assert_eq!(project.canvas.width, 1080);
            assert_eq!(project.canvas.height, 1920);
            assert_eq!(project.frame_count(), 30, "{label}");
            assert_eq!(resolved, taste::resolve(&style));
        }
    }

    #[test]
    fn preview_is_deterministic() {
        let library = AssetLibrary::new("assets");
        let style = StyleProfile::default();
        let a = style_preview(&style, &library, &ApproxMeasure)
            .0
            .to_json_pretty();
        let b = style_preview(&style, &library, &ApproxMeasure)
            .0
            .to_json_pretty();
        assert_eq!(a, b);
    }

    #[test]
    fn describe_mentions_all_temporal_dimensions() {
        for (label, style) in styles() {
            let text = describe(&taste::resolve(&style));
            for key in [
                "Motion temperament:",
                "Transition character:",
                "Density:",
                "Composition rhythm:",
                "Scale contrast:",
                "Layer activity:",
                "Tone:",
                "Palette:",
                "Background:",
                "Typography:",
                "Material:",
                "Image treatment:",
            ] {
                assert!(text.contains(key), "{label}: missing {key}\n{text}");
            }
        }
    }
}
