//! Structural typography-dominance QA (docs/VISUAL_LANGUAGE.md section 6).
//!
//! Pure MotionScene structure: no OCR, no pixels, no scene metadata. Each beat
//! scene (a scene with a `lifecycle`) is evaluated through the timeline at the
//! middle of its readable window, `(read + anticipate) / 2`, so what is measured
//! is what is on screen then: layers that enter with a fade or a scale are
//! counted at their resolved opacity and size, layers that already left are not.
//! Text area is compared against figure area (images, svgs, polylines,
//! non-container shapes). A beat is type-dominant when it has no image/svg layer
//! and its figure share is below [`TYPE_DOMINANT_SHARE`]. The pure-type ratio is
//! then compared against the reference's visual weight.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use motion_core::compiler::visual::Weight;
use motion_core::scene::{Layer, LayerKind, MotionProject, Scene};
use motion_core::timeline::{evaluate_frame, Affine, ResolvedLayer};
use serde::Serialize;

/// Text layers at or below this effective opacity are faint ghost words.
pub const GHOST_OPACITY: f32 = 0.25;
/// Figures need at least this effective opacity to count.
pub const SHAPE_MIN_OPACITY: f32 = 0.3;
/// A plain rectangle counts as a figure only at object scale: larger
/// fields (empty panels, slabs, wipes, backgrounds) are layout, not visual
/// elements (0.7.1 review: an empty split-contrast panel is not imagery).
pub const SHAPE_MAX_CANVAS_FRACTION: f32 = 0.15;
/// Shapes must have a min side of at least this fraction of the canvas short side (not a rule).
pub const SHAPE_MIN_SIDE_FRACTION: f32 = 0.03;
/// A beat is type-dominant below this figure share.
pub const TYPE_DOMINANT_SHARE: f32 = 0.25;
/// Weight Visual: warn when the pure-type ratio exceeds this.
pub const VISUAL_WARN_RATIO: f32 = 0.6;
/// Weight Type: warn when the pure-type ratio is below this.
pub const TYPE_WARN_RATIO: f32 = 0.4;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BeatVisual {
    pub scene: String,
    pub text_area: f32,
    pub figure_area: f32,
    pub figure_share: f32,
    pub images: usize,
    pub svgs: usize,
    pub polylines: usize,
    /// Every figure-contributing layer: images, svgs, polylines and qualifying shapes.
    pub figures: usize,
    pub type_dominant: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualVerdict {
    Consistent,
    VisualTransferLikelyFailed,
    TypeLedButVisualHeavy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VisualReport {
    pub beats: Vec<BeatVisual>,
    pub pure_type_ratio: f32,
    pub verdict: Option<VisualVerdict>,
    /// Beat scenes whose sample frame could not be evaluated ("scene: reason").
    pub skipped: Vec<String>,
}

/// Analyse every beat scene of `project`. `weight` is the reference's visual
/// weight (None or Neutral produces no verdict).
pub fn visual_report(project: &MotionProject, weight: Option<Weight>) -> VisualReport {
    let canvas_area = project.canvas.width as f32 * project.canvas.height as f32;
    let short_side = project.canvas.width.min(project.canvas.height) as f32;
    let mut beats = Vec::new();
    let mut skipped = Vec::new();
    for scene in project.scenes.iter().filter(|s| s.lifecycle.is_some()) {
        match analyse_scene(project, scene, canvas_area, short_side) {
            Ok(b) => beats.push(b),
            Err(e) => skipped.push(format!("{}: {e}", scene.id)),
        }
    }
    let pure_type_ratio = if beats.is_empty() {
        0.0
    } else {
        beats.iter().filter(|b| b.type_dominant).count() as f32 / beats.len() as f32
    };
    let verdict = match weight {
        Some(Weight::Visual) if pure_type_ratio > VISUAL_WARN_RATIO => {
            Some(VisualVerdict::VisualTransferLikelyFailed)
        }
        Some(Weight::Type) if pure_type_ratio < TYPE_WARN_RATIO => {
            Some(VisualVerdict::TypeLedButVisualHeavy)
        }
        Some(Weight::Visual | Weight::Type) => Some(VisualVerdict::Consistent),
        Some(Weight::Neutral) | None => None,
    };
    VisualReport {
        beats,
        pure_type_ratio,
        verdict,
        skipped,
    }
}

impl VisualReport {
    fn type_beats(&self) -> usize {
        self.beats.iter().filter(|b| b.type_dominant).count()
    }

    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "\nvisual language");
        for b in &self.beats {
            let _ = writeln!(
                out,
                "  {:<14} {:<6} share {:.2}  text {:.0}  figure {:.0}  images {} svgs {} polylines {} figures {}",
                b.scene,
                if b.type_dominant { "TYPE" } else { "VISUAL" },
                b.figure_share,
                b.text_area,
                b.figure_area,
                b.images,
                b.svgs,
                b.polylines,
                b.figures
            );
        }
        for s in &self.skipped {
            let _ = writeln!(out, "  skipped {s}");
        }
        let _ = writeln!(
            out,
            "pure-type ratio: {:.2} ({}/{} beats type-dominant)",
            self.pure_type_ratio,
            self.type_beats(),
            self.beats.len()
        );
        match self.verdict {
            Some(VisualVerdict::VisualTransferLikelyFailed) => {
                let _ = writeln!(
                    out,
                    "WARNING: reference visual language is visual/object-led, but {}/{} beats resolved to type-dominant compositions. Visual-language transfer likely failed.",
                    self.type_beats(),
                    self.beats.len()
                );
            }
            Some(VisualVerdict::TypeLedButVisualHeavy) => {
                let _ = writeln!(
                    out,
                    "WARNING: reference visual language is type-led, but only {}/{} beats are type-dominant.",
                    self.type_beats(),
                    self.beats.len()
                );
            }
            Some(VisualVerdict::Consistent) | None => {}
        }
        out
    }
}

/// A resolved layer with its effective opacity (ancestors x own x text alpha).
struct Seen<'a, 'f> {
    layer: &'f ResolvedLayer<'a>,
    effective: f32,
}

fn collect<'a, 'f>(
    layers: &'f [ResolvedLayer<'a>],
    scene_id: &str,
    parent_opacity: f32,
    out: &mut Vec<Seen<'a, 'f>>,
) {
    for l in layers {
        if l.scene.is_some_and(|s| s != scene_id) {
            continue;
        }
        let mut effective = parent_opacity * l.opacity;
        if let LayerKind::Text(t) = l.kind {
            effective *= t.color.a as f32 / 255.0;
        }
        out.push(Seen {
            layer: l,
            effective,
        });
        collect(&l.children, scene_id, parent_opacity * l.opacity, out);
    }
}

fn analyse_scene(
    project: &MotionProject,
    scene: &Scene,
    canvas_area: f32,
    short_side: f32,
) -> Result<BeatVisual, String> {
    let life = scene.lifecycle.ok_or_else(|| "no lifecycle".to_string())?;
    let fps = project.canvas.fps as f64;
    let t = scene.start_seconds + (life.read + life.anticipate) / 2.0;
    let first = (scene.start_seconds * fps).round().max(0.0);
    let last = ((scene.end_seconds() * fps).round() - 1.0).max(first);
    let frame = (t * fps).round().clamp(first, last) as u32;
    let resolved = evaluate_frame(project, frame).map_err(|e| e.to_string())?;

    let mut seen = Vec::new();
    collect(&resolved.layers, &scene.id, 1.0, &mut seen);

    let mut layout_parent: BTreeMap<&str, &str> = BTreeMap::new();
    layout_parents(&scene.layers, &mut layout_parent);

    // Visible text layers (their canvas-space box centres decide containers).
    let texts: Vec<&Seen> = seen
        .iter()
        .filter(|s| matches!(s.layer.kind, LayerKind::Text(_)) && s.effective > GHOST_OPACITY)
        .collect();

    let mut text_area = 0.0f32;
    let mut figure_area = 0.0f32;
    let (mut images, mut svgs, mut polylines, mut figures) = (0, 0, 0, 0);

    for s in &seen {
        let l = s.layer;
        match l.kind {
            LayerKind::Text(_) => {
                if s.effective > GHOST_OPACITY {
                    text_area += capped(box_area(l), canvas_area);
                }
            }
            LayerKind::Image { .. } if s.effective >= SHAPE_MIN_OPACITY => {
                images += 1;
                figures += 1;
                figure_area += capped(box_area(l), canvas_area);
            }
            LayerKind::Svg { .. } if s.effective >= SHAPE_MIN_OPACITY => {
                svgs += 1;
                figures += 1;
                figure_area += capped(box_area(l), canvas_area);
            }
            LayerKind::Polyline { points, stroke, .. } if s.effective >= SHAPE_MIN_OPACITY => {
                polylines += 1;
                figures += 1;
                figure_area += capped(polyline_area(l, points, stroke.width), canvas_area);
            }
            LayerKind::Rectangle { .. } | LayerKind::RoundedRectangle { .. } => {
                let area = box_area(l);
                let (sx, sy) = axis_scales(&l.transform);
                let min_side = (l.width * sx).min(l.height * sy);
                let qualifies = s.effective >= SHAPE_MIN_OPACITY
                    && area < SHAPE_MAX_CANVAS_FRACTION * canvas_area
                    && min_side >= SHAPE_MIN_SIDE_FRACTION * short_side
                    && !is_text_container(l, &texts, &layout_parent);
                if qualifies {
                    figures += 1;
                    figure_area += capped(area, canvas_area);
                }
            }
            _ => {}
        }
    }

    let total = figure_area + text_area;
    let figure_share = if total > 0.0 {
        figure_area / total
    } else {
        0.0
    };
    Ok(BeatVisual {
        scene: scene.id.clone(),
        text_area,
        figure_area,
        figure_share,
        images,
        svgs,
        polylines,
        figures,
        type_dominant: images == 0 && svgs == 0 && figure_share < TYPE_DOMINANT_SHARE,
    })
}

fn layout_parents<'a>(layers: &'a [Layer], out: &mut BTreeMap<&'a str, &'a str>) {
    for l in layers {
        if let Some(b) = &l.layout {
            out.insert(&l.id, &b.parent);
        }
        if let LayerKind::Group { children } = &l.kind {
            layout_parents(children, out);
        }
    }
}

fn capped(area: f32, canvas_area: f32) -> f32 {
    area.max(0.0).min(canvas_area)
}

fn det(a: &Affine) -> f32 {
    (a.a * a.d - a.b * a.c).abs()
}

/// Length of the transformed unit x and y axes.
fn axis_scales(a: &Affine) -> (f32, f32) {
    (a.a.hypot(a.b), a.c.hypot(a.d))
}

/// Resolved box area in canvas space.
fn box_area(l: &ResolvedLayer) -> f32 {
    l.width * l.height * det(&l.transform)
}

/// Canvas-space bounding box of the transformed points, at least the stroke
/// width on the thin side, scaled by the visible trim fraction.
fn polyline_area(l: &ResolvedLayer, points: &[[f32; 2]], stroke_width: f32) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let (mut min_x, mut max_x) = (f32::MAX, f32::MIN);
    let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
    for p in points {
        let (x, y) = l.transform.apply(p[0], p[1]);
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }
    let sw = stroke_width.max(0.0);
    let area = (max_x - min_x).max(sw) * (max_y - min_y).max(sw);
    area * l.trim.map_or(1.0, |t| t.clamp(0.0, 1.0))
}

/// A rect is a text container when a visible text layer's canvas-space box
/// centre lies inside it, or a text layer is layout-bound to it.
fn is_text_container(
    rect: &ResolvedLayer,
    texts: &[&Seen],
    layout_parent: &BTreeMap<&str, &str>,
) -> bool {
    let d = rect.transform.a * rect.transform.d - rect.transform.b * rect.transform.c;
    texts.iter().any(|t| {
        let tl = t.layer;
        if layout_parent.get(tl.id).is_some_and(|p| *p == rect.id) {
            return true;
        }
        if d.abs() < 1e-6 {
            return false;
        }
        let (cx, cy) = tl.transform.apply(tl.width / 2.0, tl.height / 2.0);
        // Canvas -> rect box space (inverse affine).
        let m = &rect.transform;
        let (dx, dy) = (cx - m.e, cy - m.f);
        let lx = (m.d * dx - m.c * dy) / d;
        let ly = (-m.b * dx + m.a * dy) / d;
        lx >= 0.0 && lx <= rect.width && ly >= 0.0 && ly <= rect.height
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::easing::Easing;
    use motion_core::scene::*;

    fn base(id: &str, x: f32, y: f32, w: f32, h: f32, kind: LayerKind) -> Layer {
        Layer {
            tilt: None,
            z: None,
            id: id.into(),
            x,
            y,
            width: w,
            height: h,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            anchor_x: 0.0,
            anchor_y: 0.0,
            opacity: 1.0,
            z_index: 0,
            visible: true,
            clip: None,
            depth: None,
            layout: None,
            kind,
        }
    }

    fn text(id: &str, x: f32, y: f32, w: f32, h: f32) -> Layer {
        base(
            id,
            x,
            y,
            w,
            h,
            LayerKind::Text(TextStyle {
                text: "word".into(),
                font_role: FontRole::Display,
                font_size: 80.0,
                font_weight: 700,
                italic: false,
                color: Color::rgb(0, 0, 0),
                align: TextAlign::default(),
                line_height: 1.1,
                letter_spacing: 0.0,
                max_width: None,
                uppercase: false,
                ink: None,
            }),
        )
    }

    fn rect(id: &str, x: f32, y: f32, w: f32, h: f32) -> Layer {
        base(
            id,
            x,
            y,
            w,
            h,
            LayerKind::Rectangle {
                fill: Color::rgb(200, 0, 0),
                stroke: None,
            },
        )
    }

    fn image(id: &str, x: f32, y: f32, w: f32, h: f32) -> Layer {
        base(
            id,
            x,
            y,
            w,
            h,
            LayerKind::Image {
                asset: "a".into(),
                fit: Fit::Contain,
                treatment: None,
                playback: None,
                insert: None,
            },
        )
    }

    fn polyline(id: &str, points: Vec<[f32; 2]>) -> Layer {
        base(
            id,
            0.0,
            0.0,
            0.0,
            0.0,
            LayerKind::Polyline {
                points,
                stroke: Stroke {
                    color: Color::rgb(0, 0, 0),
                    width: 6.0,
                },
                closed: false,
                fill: None,
            },
        )
    }

    fn fade(target: &str, start: f64, duration: f64, from: f32, to: f32) -> Motion {
        Motion {
            spring: None,
            id: None,
            target: target.into(),
            start,
            duration,
            easing: Easing::default(),
            op: MotionOp::Fade { from, to },
        }
    }

    /// Sampled at (read + anticipate) / 2 = 1.9 s.
    fn lifecycle() -> Lifecycle {
        Lifecycle {
            enter: 0.5,
            settle: 0.8,
            read: 1.2,
            evolve: 2.0,
            anticipate: 2.6,
            bridge: 3.0,
        }
    }

    fn scene_with(id: &str, beat: bool, layers: Vec<Layer>, motions: Vec<Motion>) -> Scene {
        Scene {
            post: Vec::new(),
            id: id.into(),
            start_seconds: 0.0,
            duration_seconds: 3.0,
            layers,
            motions,
            camera: None,
            lifecycle: beat.then(lifecycle),
        }
    }

    fn scene(id: &str, beat: bool, layers: Vec<Layer>) -> Scene {
        scene_with(id, beat, layers, vec![])
    }

    fn project(scenes: Vec<Scene>) -> MotionProject {
        MotionProject {
            envelopes: Vec::new(),
            version: SCENE_VERSION.into(),
            project: ProjectMeta {
                name: "t".into(),
                duration_seconds: None,
                exploration: None,
                speech: None,
                art: None,
                direction: None,
            },
            canvas: Canvas {
                width: 1080,
                height: 1920,
                fps: 30,
                background: Color::rgb(255, 255, 255),
            },
            theme: Theme::default(),
            assets: vec![],
            asset_root: None,
            scenes,
            shared: vec![],
        }
    }

    fn one_with(layers: Vec<Layer>, motions: Vec<Motion>) -> BeatVisual {
        let p = project(vec![scene_with("s", true, layers, motions)]);
        let r = visual_report(&p, None);
        assert!(r.skipped.is_empty(), "{:?}", r.skipped);
        r.beats.into_iter().next().expect("one beat")
    }

    fn one(layers: Vec<Layer>) -> BeatVisual {
        one_with(layers, vec![])
    }

    #[test]
    fn headline_with_text_card_is_type_dominant() {
        let mut card = rect("card", 100.0, 800.0, 880.0, 400.0);
        card.opacity = 0.9;
        let b = one(vec![
            card,
            text("headline", 140.0, 860.0, 800.0, 200.0),
            text("sub", 140.0, 1080.0, 800.0, 80.0),
        ]);
        assert_eq!(b.figures, 0, "card is a text container");
        assert!(b.type_dominant);
        assert_eq!(b.figure_share, 0.0);
    }

    #[test]
    fn layout_bound_text_makes_container() {
        let card = rect("card", 100.0, 100.0, 600.0, 300.0);
        let mut t = text("t", 0.0, 0.0, 400.0, 100.0);
        t.layout = Some(LayoutBinding {
            parent: "card".into(),
            horizontal: HAlign::Center,
            vertical: VAlign::Center,
            offset: [0.0, 0.0],
            padding: Padding::default(),
        });
        let b = one(vec![card, t]);
        assert_eq!(b.figures, 0);
        assert!(b.type_dominant);
    }

    #[test]
    fn image_layer_is_never_type_dominant() {
        let b = one(vec![
            text("headline", 100.0, 1400.0, 800.0, 300.0),
            image("pic", 400.0, 100.0, 40.0, 40.0),
        ]);
        assert_eq!(b.images, 1);
        assert!(b.figure_share < TYPE_DOMINANT_SHARE);
        assert!(!b.type_dominant);
    }

    #[test]
    fn polylines_and_shapes_with_small_label_are_visual() {
        let b = one(vec![
            polyline(
                "rail",
                vec![[100.0, 900.0], [900.0, 900.0], [900.0, 1300.0]],
            ),
            rect("token", 200.0, 500.0, 300.0, 300.0),
            text("label", 200.0, 1500.0, 300.0, 60.0),
        ]);
        assert_eq!(b.polylines, 1);
        assert_eq!(b.figures, 2);
        assert!(b.figure_share > TYPE_DOMINANT_SHARE);
        assert!(!b.type_dominant);
    }

    #[test]
    fn faint_ghost_text_is_ignored() {
        let mut ghost = text("ghost", 0.0, 0.0, 1080.0, 1500.0);
        ghost.opacity = 0.25;
        let b = one(vec![ghost, text("real", 100.0, 800.0, 400.0, 100.0)]);
        assert_eq!(b.text_area, 400.0 * 100.0);
    }

    #[test]
    fn thin_rules_backdrops_and_faint_shapes_are_not_figures() {
        let mut faint = rect("faint", 100.0, 100.0, 300.0, 300.0);
        faint.opacity = 0.29;
        let b = one(vec![
            rect("rule", 100.0, 700.0, 800.0, 4.0),
            rect("bg", 0.0, 0.0, 1080.0, 1920.0),
            faint,
            text("t", 100.0, 1000.0, 600.0, 200.0),
        ]);
        assert_eq!(b.figures, 0);
        assert!(b.type_dominant);
    }

    #[test]
    fn areas_capped_at_canvas_and_scaled() {
        let mut img = image("pic", 0.0, 0.0, 5000.0, 5000.0);
        img.scale_x = 2.0;
        let b = one(vec![img]);
        assert_eq!(b.figure_area, 1080.0 * 1920.0);
        let mut t = text("t", 0.0, 0.0, 100.0, 50.0);
        t.scale_x = 2.0;
        t.scale_y = 2.0;
        assert_eq!(one(vec![t]).text_area, 20000.0);
    }

    #[test]
    fn group_children_are_walked_with_inherited_opacity() {
        let group = |opacity: f32| {
            let mut g = base(
                "g",
                0.0,
                0.0,
                0.0,
                0.0,
                LayerKind::Group {
                    children: vec![image("pic", 0.0, 0.0, 300.0, 300.0)],
                },
            );
            g.opacity = opacity;
            g
        };
        assert_eq!(one(vec![group(1.0)]).images, 1);
        assert_eq!(one(vec![group(0.2)]).images, 0, "faint group hides child");
    }

    #[test]
    fn token_fading_in_before_read_counts_as_figure() {
        // Base opacity 1 x Fade 0 -> 1 over 0.1..0.6 s: fully in by READ.
        let tokens = vec![
            base(
                "tok_a",
                100.0,
                600.0,
                400.0,
                400.0,
                LayerKind::RoundedRectangle {
                    fill: Color::rgb(0, 100, 200),
                    radius: 200.0,
                    stroke: None,
                },
            ),
            rect("tok_b", 580.0, 600.0, 400.0, 400.0),
        ];
        let layers = || {
            let mut l = tokens.clone();
            l.push(text("head", 100.0, 1400.0, 600.0, 120.0));
            l
        };
        let faded_in = one_with(
            layers(),
            vec![
                fade("tok_a", 0.1, 0.5, 0.0, 1.0),
                fade("tok_b", 0.1, 0.5, 0.0, 1.0),
            ],
        );
        assert_eq!(faded_in.figures, 2);
        assert!(!faded_in.type_dominant, "share {}", faded_in.figure_share);
        // Control: tokens whose fade never completes before the sample stay out.
        let late = one_with(
            layers(),
            vec![
                fade("tok_a", 2.8, 0.1, 0.0, 1.0),
                fade("tok_b", 2.8, 0.1, 0.0, 1.0),
            ],
        );
        assert_eq!(late.figures, 0);
        assert!(late.type_dominant);
    }

    #[test]
    fn large_empty_panel_is_layout_not_a_figure() {
        // An empty colour field of a third of the canvas (e.g. a split
        // panel) is not a visual element.
        let r = one(vec![
            rect("panel", 0.0, 1100.0, 1080.0, 700.0),
            text("head", 100.0, 200.0, 800.0, 300.0),
        ]);
        assert_eq!(r.figures, 0);
        assert!(r.type_dominant);
    }

    #[test]
    fn card_fading_out_before_read_does_not_count() {
        // The card is not a text container (text elsewhere) and would be a figure
        // at full opacity; it is gone by READ.
        let layers = || {
            vec![
                rect("card", 100.0, 300.0, 400.0, 400.0),
                text("head", 100.0, 1400.0, 700.0, 300.0),
            ]
        };
        assert_eq!(one(layers()).figures, 1, "visible card counts");
        let gone = one_with(layers(), vec![fade("card", 0.2, 0.5, 1.0, 0.0)]);
        assert_eq!(gone.figures, 0);
        assert!(gone.type_dominant);
    }

    #[test]
    fn scaled_in_token_uses_resolved_size() {
        let mut tok = rect("tok", 100.0, 100.0, 100.0, 100.0);
        tok.scale_x = 1.0;
        let m = Motion {
            spring: None,
            id: None,
            target: "tok".into(),
            start: 0.1,
            duration: 0.5,
            easing: Easing::default(),
            op: MotionOp::Scale {
                from: 1.0,
                to: 4.0,
                axis: Axis::Both,
            },
        };
        let b = one_with(vec![tok, text("t", 0.0, 1500.0, 200.0, 100.0)], vec![m]);
        assert_eq!(b.figure_area, 400.0 * 400.0);
    }

    #[test]
    fn other_scene_layers_are_not_counted() {
        let p = project(vec![
            scene("a", true, vec![text("ta", 0.0, 0.0, 400.0, 100.0)]),
            scene("b", true, vec![image("pb", 0.0, 0.0, 400.0, 400.0)]),
        ]);
        let r = visual_report(&p, None);
        assert_eq!(r.beats[0].images, 0);
        assert_eq!(r.beats[1].images, 1);
    }

    #[test]
    fn backdrop_scene_without_lifecycle_is_ignored() {
        let p = project(vec![
            scene(
                "backdrop",
                false,
                vec![image("bg", 0.0, 0.0, 1080.0, 1920.0)],
            ),
            scene("b1", true, vec![text("t", 0.0, 0.0, 400.0, 100.0)]),
        ]);
        let r = visual_report(&p, None);
        assert_eq!(r.beats.len(), 1);
        assert_eq!(r.beats[0].scene, "b1");
        assert_eq!(r.pure_type_ratio, 1.0);
        assert_eq!(r.verdict, None);
    }

    #[test]
    fn unevaluable_project_is_skipped_and_noted() {
        let mut p = project(vec![scene("b1", true, vec![])]);
        p.canvas.fps = 0;
        let r = visual_report(&p, None);
        assert!(r.beats.is_empty());
        assert_eq!(r.skipped.len(), 1);
        assert!(r.skipped[0].starts_with("b1:"));
        assert!(r.to_text().contains("skipped b1:"));
    }

    fn ratio_project(type_beats: usize, visual_beats: usize) -> MotionProject {
        let mut scenes = Vec::new();
        for i in 0..type_beats {
            scenes.push(scene(
                &format!("t{i}"),
                true,
                vec![text(&format!("t{i}_text"), 0.0, 0.0, 400.0, 100.0)],
            ));
        }
        for i in 0..visual_beats {
            scenes.push(scene(
                &format!("v{i}"),
                true,
                vec![image(&format!("v{i}_img"), 0.0, 0.0, 400.0, 400.0)],
            ));
        }
        project(scenes)
    }

    #[test]
    fn verdict_boundaries() {
        // Visual: ratio must be strictly above 0.6.
        let r = visual_report(&ratio_project(3, 2), Some(Weight::Visual));
        assert_eq!(r.pure_type_ratio, 0.6);
        assert_eq!(r.verdict, Some(VisualVerdict::Consistent));
        let r = visual_report(&ratio_project(4, 1), Some(Weight::Visual));
        assert_eq!(r.verdict, Some(VisualVerdict::VisualTransferLikelyFailed));
        assert!(r.to_text().contains(
            "WARNING: reference visual language is visual/object-led, but 4/5 beats resolved to type-dominant compositions. Visual-language transfer likely failed."
        ));
        // Type: ratio must be strictly below 0.4.
        let r = visual_report(&ratio_project(2, 3), Some(Weight::Type));
        assert_eq!(r.pure_type_ratio, 0.4);
        assert_eq!(r.verdict, Some(VisualVerdict::Consistent));
        let r = visual_report(&ratio_project(1, 4), Some(Weight::Type));
        assert_eq!(r.verdict, Some(VisualVerdict::TypeLedButVisualHeavy));
        assert!(r.to_text().contains(
            "WARNING: reference visual language is type-led, but only 1/5 beats are type-dominant."
        ));
        // Neutral / none: no verdict.
        assert_eq!(
            visual_report(&ratio_project(5, 0), Some(Weight::Neutral)).verdict,
            None
        );
        assert_eq!(visual_report(&ratio_project(5, 0), None).verdict, None);
        // No beats: ratio 0.
        assert_eq!(visual_report(&project(vec![]), None).pure_type_ratio, 0.0);
    }
}
