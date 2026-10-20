//! V2 Sketch Component (`kind: "sketch"`).
//!
//! Natural media rendering powered by `p5.brush`: pencil, charcoal, rotring,
//! and marker brushes with procedural vector flow-fields, parametric splines,
//! interior cross-hatching, watercolor washes, and Fibonacci golden-spiral compositions.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use motion_core::easing::Easing;
use motion_core::path::{self, BezPath, Contour, Cubic};
use motion_core::scene::{Fill, FontRole, Layer, LayerKind, MotionOp, PathFillRule, Scene, Stroke};

use crate::lower::{layer, scene, Ctx, Track, Use};
use crate::program::{Component, Size, Slot, Swatch, Verb};
use crate::taste::{self, Shape};

/// Layout and composition presets for a [`Sketch`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SketchPreset {
    /// Renders user-authored `elements` in normalized `[0.0..1.0]` box coordinates.
    #[default]
    Freeform,
    /// Euclid/Fibonacci Golden Rectangle + Logarithmic Spiral study with watercolor wash
    /// chambers, hatched golden squares, and charcoal spiral curve leading into the golden eye.
    GoldenSpiral,
    /// Organic multi-streamline flow field tapestry around a calm central editorial title card.
    FlowField,
    /// Architectural elevation and drafting composition with geometric cross-hatching,
    /// technical rotring callouts, and watercolor wash accents.
    ArchitecturalHatch,
}

/// Natural media brush presets supported by the sketch engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SketchBrush {
    #[default]
    Hb,
    #[serde(rename = "2b", alias = "two_b")]
    TwoB,
    #[serde(rename = "2h", alias = "two_h")]
    TwoH,
    Cpencil,
    Pen,
    Rotring,
    Spray,
    Marker,
    Marker2,
    Charcoal,
    HatchBrush,
}

impl SketchBrush {
    /// Convert authoring brush preset to `motion_core::brush::BrushPreset`.
    pub fn to_core(self) -> motion_core::brush::BrushPreset {
        match self {
            Self::Hb => motion_core::brush::BrushPreset::Hb,
            Self::TwoB => motion_core::brush::BrushPreset::TwoB,
            Self::TwoH => motion_core::brush::BrushPreset::TwoH,
            Self::Cpencil => motion_core::brush::BrushPreset::Cpencil,
            Self::Pen => motion_core::brush::BrushPreset::Pen,
            Self::Rotring => motion_core::brush::BrushPreset::Rotring,
            Self::Spray => motion_core::brush::BrushPreset::Spray,
            Self::Marker => motion_core::brush::BrushPreset::Marker,
            Self::Marker2 => motion_core::brush::BrushPreset::Marker2,
            Self::Charcoal => motion_core::brush::BrushPreset::Charcoal,
            Self::HatchBrush => motion_core::brush::BrushPreset::HatchBrush,
        }
    }
}

/// Procedural vector flow field presets.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SketchField {
    #[default]
    Hand,
    Curved,
    Zigzag,
    Waves,
    Seabed,
    Spiral,
    Columns,
}

impl SketchField {
    /// Convert authoring field preset to `motion_core::brush::VectorFieldPreset`.
    pub fn to_core(self) -> motion_core::brush::VectorFieldPreset {
        match self {
            Self::Hand => motion_core::brush::VectorFieldPreset::Hand,
            Self::Curved => motion_core::brush::VectorFieldPreset::Curved,
            Self::Zigzag => motion_core::brush::VectorFieldPreset::Zigzag,
            Self::Waves => motion_core::brush::VectorFieldPreset::Waves,
            Self::Seabed => motion_core::brush::VectorFieldPreset::Seabed,
            Self::Spiral => motion_core::brush::VectorFieldPreset::Spiral,
            Self::Columns => motion_core::brush::VectorFieldPreset::Columns,
        }
    }
}

fn default_hatch_angle() -> f32 {
    35.0
}

fn default_hatch_rand() -> f32 {
    0.08
}

/// Interior hatching specification (`brush.hatch`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SketchHatch {
    /// Hatch spacing in pixels (default: 14.0 px).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dist: Option<f32>,
    /// Hatch line angle in degrees (default: 35.0).
    #[serde(default = "default_hatch_angle")]
    pub angle_deg: f32,
    /// If true, adds perpendicular cross-hatch lines.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cross: bool,
    /// Randomness of line placement and jitter (default: 0.08).
    #[serde(default = "default_hatch_rand")]
    pub rand: f32,
    /// Whether hatch strokes loop continuously.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub continuous: bool,
    /// Density gradient along hatch direction (0.0 = uniform).
    #[serde(default)]
    pub gradient: f32,
    /// Brush preset used to stroke hatch lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<SketchBrush>,
    /// Hatch stroke color swatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatch: Option<Swatch>,
    /// Hatch stroke weight multiplier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
}

/// Interior watercolor bleed fill specification (`brush.fill` + `fillBleed`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SketchWatercolor {
    /// Color swatch for watercolor wash.
    pub swatch: Swatch,
    /// Wash opacity (0.0..=1.0, default: 0.78).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// Bleed intensity beyond the shape boundary (default: 0.35).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bleed: Option<f32>,
    /// Granular paper texture absorption factor (default: 0.55).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<f32>,
    /// Edge pigment accumulation border intensity (default: 0.45).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub border: Option<f32>,
    /// Optional dominant flow angle of the pigment bleed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle_deg: Option<f32>,
    /// If true, bleed pools inward rather than bleeding out past the contour.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inward: bool,
}

fn default_curv() -> f32 {
    0.5
}

/// The geometric shape kind of a sketch element.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum SketchShapeKind {
    /// Straight line segment between normalized `[0..1]` points.
    Line { from: [f32; 2], to: [f32; 2] },
    /// Smooth cubic spline passing through control points with optional per-point pressures.
    Spline {
        points: Vec<[f32; 2]>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pressures: Vec<f32>,
        #[serde(default = "default_curv")]
        curvature: f32,
    },
    /// Streamline following the sketch vector field from a start coordinate.
    FlowLine {
        start: [f32; 2],
        length: f32,
        #[serde(default)]
        angle_deg: f32,
    },
    /// Normalized rectangle `[x, y, w, h]` with optional corner radius.
    Rect {
        xywh: [f32; 4],
        #[serde(default)]
        radius: f32,
    },
    /// Circle with center and radius in normalized coordinates, plus hand irregularity.
    Circle {
        center: [f32; 2],
        radius: f32,
        #[serde(default)]
        irregularity: f32,
    },
    /// Circular arc between two angles (in degrees, clockwise).
    Arc {
        center: [f32; 2],
        radius: f32,
        start_deg: f32,
        end_deg: f32,
    },
    /// Closed polygon through normalized vertex coordinates.
    Polygon { points: Vec<[f32; 2]> },
    /// Arbitrary SVG path data in normalized `0..1000` space, scaled to slot box.
    Path { d: String },
}

fn default_one() -> f32 {
    1.0
}

fn default_stroke_swatch() -> Swatch {
    Swatch::Ink
}

/// An individual drawable natural-media mark or polygon.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SketchElement {
    #[serde(flatten)]
    pub kind: SketchShapeKind,
    /// Stroke brush (omit if noStroke).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<SketchBrush>,
    /// Stroke colour swatch (default: Ink).
    #[serde(default = "default_stroke_swatch")]
    pub stroke_swatch: Swatch,
    /// Stroke weight multiplier (default: 1.0).
    #[serde(default = "default_one")]
    pub weight: f32,
    /// Optional pressure curve `[start, mid, end]` or `[start, end]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pressure: Vec<f32>,
    /// Optional custom brush tip image asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tip_asset: Option<String>,
    /// Optional per-element hand wiggle override (0..=10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiggle: Option<f32>,
    /// Optional interior hatching (`brush.hatch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hatch: Option<SketchHatch>,
    /// Optional interior watercolor bleed fill (`brush.fill` + `fillBleed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watercolor: Option<SketchWatercolor>,
    /// Optional flat wash fill swatch (`brush.wash`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wash: Option<Swatch>,
}

fn default_wiggle() -> f32 {
    1.5
}

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_default_size(s: &Size) -> bool {
    *s == Size::M
}

/// Sketch natural-media component (`kind: "sketch"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Sketch {
    #[serde(default)]
    pub preset: SketchPreset,
    #[serde(default)]
    pub slot: Slot,
    /// Global vector field for this sketch (`hand`, `waves`, `curved`, `spiral`, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<SketchField>,
    /// Global hand-wiggle intensity (0.0..=10.0, default 1.5).
    #[serde(default = "default_wiggle")]
    pub wiggle: f32,
    /// Draw the framed drafting plate (border, paper ground, title bar).
    /// `false`: strokes only, on a transparent box, so the sketch can sit
    /// `over` a title, photo_stack or chart (title/caption are then
    /// rejected; use a `title` component).
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub plate: bool,
    /// Box size in the slot (default `m` = today's box; `s` 0.6x,
    /// `l` 1.15x, `xl` nearly the whole frame, centred).
    #[serde(default, skip_serializing_if = "is_default_size")]
    pub size: Size,
    /// Optional headline/title displayed in a clean high-contrast plate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional mono caption/annotation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    /// With music: stagger element reveals every this many beats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat_cuts: Option<u32>,
    /// Elements for `Freeform` (or additional overlays on presets).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<SketchElement>,
    #[serde(default)]
    pub seed: u32,
}

impl Default for Sketch {
    fn default() -> Self {
        Sketch {
            preset: SketchPreset::default(),
            slot: Slot::default(),
            field: None,
            wiggle: default_wiggle(),
            plate: true,
            size: Size::M,
            title: None,
            caption: None,
            beat_cuts: None,
            elements: Vec::new(),
            seed: 0,
        }
    }
}

impl Sketch {
    /// Static validation of a Sketch component.
    pub fn check(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();

        if let Some(t) = &self.title {
            let count = t.chars().count();
            if t.trim().is_empty() || count > 36 || t.contains('\n') {
                out.push((
                    ".title",
                    format!("title must be 1..=36 characters on one line, got {count}"),
                ));
            }
            if let Some(g) = crate::device::banned_glyph(t) {
                out.push((
                    ".title",
                    format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                ));
            }
        }

        if let Some(c) = &self.caption {
            let count = c.chars().count();
            if c.trim().is_empty() || count > 48 || c.contains('\n') {
                out.push((
                    ".caption",
                    format!("caption must be 1..=48 characters on one line, got {count}"),
                ));
            }
            if let Some(g) = crate::device::banned_glyph(c) {
                out.push((
                    ".caption",
                    format!("'{g}' has no glyph in the type faces (glyph_fallback)"),
                ));
            }
        }

        if self.beat_cuts == Some(0) {
            out.push((
                ".beat_cuts",
                "beat_cuts must be >= 1 (beats between element reveals); omit it to stagger the elements evenly".to_string(),
            ));
        }

        if !self.plate && (self.title.is_some() || self.caption.is_some()) {
            out.push((
                ".plate",
                "a plate-less sketch takes no title or caption; put the words in a `title` component (the sketch can sit `over` it)".to_string(),
            ));
        }

        if !self.wiggle.is_finite() || !(0.0..=10.0).contains(&self.wiggle) {
            out.push((
                ".wiggle",
                format!(
                    "wiggle must be finite and within 0.0..=10.0, got {}",
                    self.wiggle
                ),
            ));
        }

        if self.preset == SketchPreset::Freeform {
            let n = self.elements.len();
            if !(1..=32).contains(&n) {
                out.push((
                    ".elements",
                    format!("Freeform sketch requires 1..=32 elements, got {n}"),
                ));
            }
        } else if self.elements.len() > 32 {
            out.push((
                ".elements",
                format!(
                    "elements list must contain at most 32 elements, got {}",
                    self.elements.len()
                ),
            ));
        }

        for (i, el) in self.elements.iter().enumerate() {
            if let Some(w) = el.wiggle {
                if !w.is_finite() || !(0.0..=10.0).contains(&w) {
                    out.push((
                        ".elements.wiggle",
                        format!("element {i} wiggle must be within 0.0..=10.0, got {w}"),
                    ));
                }
            }
            if !el.weight.is_finite() || el.weight <= 0.0 {
                out.push((
                    ".elements.weight",
                    format!("element {i} weight must be positive, got {}", el.weight),
                ));
            }
            if let Some(hatch) = &el.hatch {
                if let Some(d) = hatch.dist {
                    if !d.is_finite() || d <= 0.0 {
                        out.push((
                            ".elements.hatch.dist",
                            format!("element {i} hatch dist must be positive, got {d}"),
                        ));
                    }
                }
            }
            match &el.kind {
                SketchShapeKind::Spline { points, .. } => {
                    if points.len() < 2 {
                        out.push((
                            ".elements.points",
                            format!(
                                "element {i} spline requires at least 2 points, got {}",
                                points.len()
                            ),
                        ));
                    }
                }
                SketchShapeKind::Polygon { points } => {
                    if points.len() < 3 {
                        out.push((
                            ".elements.points",
                            format!(
                                "element {i} polygon requires at least 3 points, got {}",
                                points.len()
                            ),
                        ));
                    }
                }
                SketchShapeKind::FlowLine { length, .. } => {
                    if !length.is_finite() || *length < 0.0 {
                        out.push((
                            ".elements.length",
                            format!(
                                "element {i} flow line length must be non-negative, got {length}"
                            ),
                        ));
                    }
                }
                SketchShapeKind::Rect { xywh, radius } => {
                    if xywh[2] < 0.0 || xywh[3] < 0.0 || *radius < 0.0 {
                        out.push((
                            ".elements.xywh",
                            format!("element {i} rect dimensions and radius must be non-negative"),
                        ));
                    }
                }
                SketchShapeKind::Circle {
                    radius,
                    irregularity,
                    ..
                } => {
                    if *radius < 0.0 || *irregularity < 0.0 {
                        out.push((
                            ".elements.radius",
                            format!(
                                "element {i} circle radius and irregularity must be non-negative"
                            ),
                        ));
                    }
                }
                SketchShapeKind::Arc { radius, .. } => {
                    if *radius < 0.0 {
                        out.push((
                            ".elements.radius",
                            format!("element {i} arc radius must be non-negative"),
                        ));
                    }
                }
                SketchShapeKind::Path { d } => {
                    if d.trim().is_empty() {
                        out.push((".elements.d", format!("element {i} path d cannot be empty")));
                    }
                }
                _ => {}
            }
        }

        out
    }
}

// ---------------------------------------------------------------------------
// Lowering & Geometry Pipeline
// ---------------------------------------------------------------------------

fn at_origin(id: &str, kind: LayerKind, origin: [f32; 2], size: [f32; 2], z: i32) -> Layer {
    let mut l = layer(id, kind, origin, size, z);
    l.anchor_x = 0.0;
    l.anchor_y = 0.0;
    l
}

fn line_cubic(from: [f32; 2], to: [f32; 2]) -> Cubic {
    Cubic {
        c1: [
            from[0] + (to[0] - from[0]) / 3.0,
            from[1] + (to[1] - from[1]) / 3.0,
        ],
        c2: [
            from[0] + (to[0] - from[0]) * 2.0 / 3.0,
            from[1] + (to[1] - from[1]) * 2.0 / 3.0,
        ],
        to,
    }
}

fn scale_bez_path(path: &BezPath, sx: f32, sy: f32) -> BezPath {
    let mut out = BezPath {
        contours: Vec::with_capacity(path.contours.len()),
    };
    for c in &path.contours {
        let start = [c.start[0] * sx, c.start[1] * sy];
        let mut segs = Vec::with_capacity(c.segs.len());
        for seg in &c.segs {
            segs.push(Cubic {
                c1: [seg.c1[0] * sx, seg.c1[1] * sy],
                c2: [seg.c2[0] * sx, seg.c2[1] * sy],
                to: [seg.to[0] * sx, seg.to[1] * sy],
            });
        }
        out.contours.push(Contour {
            start,
            segs,
            closed: c.closed,
        });
    }
    out
}

fn arc_path(center: [f32; 2], radius: f32, start_deg: f32, end_deg: f32) -> BezPath {
    let deg_diff = end_deg - start_deg;
    if deg_diff.abs() < 1e-4 {
        return BezPath::default();
    }
    let total_rad = deg_diff.to_radians();
    let num_segs = (deg_diff.abs() / 89.99).ceil().max(1.0) as usize;
    let seg_rad = total_rad / num_segs as f32;

    let mut start_rad = start_deg.to_radians();
    let start_pt = [
        center[0] + radius * start_rad.cos(),
        center[1] + radius * start_rad.sin(),
    ];
    let mut segs = Vec::with_capacity(num_segs);

    for _ in 0..num_segs {
        let end_seg_rad = start_rad + seg_rad;
        let delta = seg_rad;
        let alpha = (4.0 / 3.0) * (delta / 4.0).tan();

        let p1 = [
            center[0] + radius * start_rad.cos(),
            center[1] + radius * start_rad.sin(),
        ];
        let p2 = [
            center[0] + radius * end_seg_rad.cos(),
            center[1] + radius * end_seg_rad.sin(),
        ];
        let c1 = [
            p1[0] - alpha * radius * start_rad.sin(),
            p1[1] + alpha * radius * start_rad.cos(),
        ];
        let c2 = [
            p2[0] + alpha * radius * end_seg_rad.sin(),
            p2[1] - alpha * radius * end_seg_rad.cos(),
        ];
        segs.push(Cubic { c1, c2, to: p2 });
        start_rad = end_seg_rad;
    }

    BezPath {
        contours: vec![Contour {
            start: start_pt,
            segs,
            closed: false,
        }],
    }
}

fn make_brush_spec(
    preset: motion_core::brush::BrushPreset,
    weight: f32,
    pressure: motion_core::brush::PressureCurve,
    wiggle: f32,
    field: Option<motion_core::brush::VectorFieldPreset>,
    tip_asset: Option<String>,
    seed: u32,
) -> motion_core::brush::BrushSpec {
    motion_core::brush::BrushSpec {
        preset,
        weight,
        scatter: None,
        sharpness: None,
        grain: None,
        opacity: None,
        spacing: None,
        pressure: Some(pressure),
        wiggle,
        field,
        tip_asset,
        rotate: motion_core::brush::TipRotate::Natural,
        marker_tip: None,
        noise: 0.18,
        seed,
    }
}

fn element_geometry(
    kind: &SketchShapeKind,
    bw: f32,
    bh: f32,
    global_wiggle: f32,
    element_wiggle: Option<f32>,
    field: Option<SketchField>,
    seed: u32,
) -> (BezPath, Option<motion_core::brush::PressureCurve>) {
    let wiggle = element_wiggle.unwrap_or(global_wiggle);
    let core_field = field.map(|f| f.to_core());
    match kind {
        SketchShapeKind::Line { from, to } => {
            let from_px = [from[0] * bw, from[1] * bh];
            let to_px = [to[0] * bw, to[1] * bh];
            let raw = BezPath {
                contours: vec![Contour {
                    start: from_px,
                    segs: vec![line_cubic(from_px, to_px)],
                    closed: false,
                }],
            };
            let path = if wiggle > 0.0 || core_field.is_some() {
                motion_core::brush::warp_path(&raw, core_field, wiggle, [bw, bh], seed)
            } else {
                raw
            };
            (path, None)
        }
        SketchShapeKind::Spline {
            points,
            pressures,
            curvature,
        } => {
            let pts_3d: Vec<[f32; 3]> = points
                .iter()
                .enumerate()
                .map(|(idx, pt)| {
                    let p = pressures.get(idx).copied().unwrap_or(1.0);
                    [pt[0] * bw, pt[1] * bh, p]
                })
                .collect();
            let (path, pressure) = motion_core::brush::spline(&pts_3d, *curvature);
            (path, Some(pressure))
        }
        SketchShapeKind::FlowLine {
            start,
            length,
            angle_deg,
        } => {
            let start_px = [start[0] * bw, start[1] * bh];
            let len_px = if *length <= 1.0 {
                *length * bw.max(bh)
            } else {
                *length
            };
            let f = core_field.unwrap_or(motion_core::brush::VectorFieldPreset::Hand);
            let path = motion_core::brush::flow_line(
                start_px,
                len_px,
                *angle_deg,
                f,
                wiggle,
                [bw, bh],
                seed,
            );
            (path, None)
        }
        SketchShapeKind::Rect { xywh, radius } => {
            let rx = xywh[0] * bw;
            let ry = xywh[1] * bh;
            let rw = xywh[2] * bw;
            let rh = xywh[3] * bh;
            let r = *radius * bw.min(bh);
            let raw = path::rounded_rect(rx, ry, rw, rh, r);
            let path = if wiggle > 0.0 {
                motion_core::brush::warp_path(&raw, core_field, wiggle, [bw, bh], seed)
            } else {
                raw
            };
            (path, None)
        }
        SketchShapeKind::Circle {
            center,
            radius,
            irregularity,
        } => {
            let cx_px = center[0] * bw;
            let cy_px = center[1] * bh;
            let r_px = *radius * bw.min(bh);
            let raw = path::ellipse(cx_px - r_px, cy_px - r_px, r_px * 2.0, r_px * 2.0);
            let total_wiggle = (irregularity * 10.0) + wiggle;
            let path = if total_wiggle > 0.0 {
                motion_core::brush::warp_path(&raw, core_field, total_wiggle, [bw, bh], seed)
            } else {
                raw
            };
            (path, None)
        }
        SketchShapeKind::Arc {
            center,
            radius,
            start_deg,
            end_deg,
        } => {
            let cx_px = center[0] * bw;
            let cy_px = center[1] * bh;
            let r_px = *radius * bw.min(bh);
            let raw = arc_path([cx_px, cy_px], r_px, *start_deg, *end_deg);
            let path = if wiggle > 0.0 {
                motion_core::brush::warp_path(&raw, core_field, wiggle, [bw, bh], seed)
            } else {
                raw
            };
            (path, None)
        }
        SketchShapeKind::Polygon { points } => {
            let pts_px: Vec<[f32; 2]> = points.iter().map(|p| [p[0] * bw, p[1] * bh]).collect();
            let raw = path::polygon(&pts_px);
            let path = if wiggle > 0.0 {
                motion_core::brush::warp_path(&raw, core_field, wiggle, [bw, bh], seed)
            } else {
                raw
            };
            (path, None)
        }
        SketchShapeKind::Path { d } => {
            let parsed = path::parse(d).unwrap_or_default();
            let scaled = scale_bez_path(&parsed, bw / 1000.0, bh / 1000.0);
            let path = if wiggle > 0.0 {
                motion_core::brush::warp_path(&scaled, core_field, wiggle, [bw, bh], seed)
            } else {
                scaled
            };
            (path, None)
        }
    }
}

/// The framed drafting plate behind a plated sketch.
#[allow(clippy::too_many_arguments)]
fn plate_layer(
    layers: &mut Vec<Layer>,
    track: &mut Track,
    id: &str,
    r: &crate::palette::Roles,
    origin: [f32; 2],
    size: [f32; 2],
    plate_r: f32,
    z: i32,
    tempo: f64,
    enter_at: f64,
) {
    let [bw, bh] = size;
    let plate_d = path::rounded_rect(0.0, 0.0, bw, bh, plate_r).to_svg();
    layers.push(at_origin(
        &format!("{id}.plate"),
        LayerKind::Path {
            d: plate_d,
            fill: Some(Fill::Solid(r.ground)),
            stroke: Some(Stroke {
                color: r.structure_strong,
                width: 2.0,
                brush: None,
            }),
            fill_rule: PathFillRule::Nonzero,
            shadows: Vec::new(),
        },
        origin,
        [bw, bh],
        z,
    ));

    track.push(
        &format!("{id}.plate"),
        enter_at,
        0.35 * tempo,
        Shape::Spring(taste::Spring::Snap),
        MotionOp::Scale {
            from: 0.94,
            to: 1.0,
            axis: Default::default(),
        },
    );
}

/// Lower a [`Sketch`] component into an animated [`Scene`].
pub(crate) fn lower(cx: &Ctx, c: &Component, sk: &Sketch, z: i32, length: f64) -> Scene {
    let id = c.id.as_str();
    let tempo = cx.tempo.factor();
    let r = cx.roles(id);

    // 1. Slot bounding box with Golden Ratio (0.618) proportions on Center slot
    let (bw, bh) = if sk.slot == Slot::Center {
        match cx.aspect {
            crate::program::Aspect::Portrait => ((cx.w * 0.90).round(), (cx.h * 0.80).round()),
            _ => ((cx.w * 0.88).round(), (cx.h * 0.76).round()),
        }
    } else {
        let w = taste::slot_width(sk.slot, cx.aspect).round();
        let h = match (sk.slot, cx.aspect) {
            (Slot::Top | Slot::Lead, crate::program::Aspect::Portrait) => (cx.h * 0.34).round(),
            (Slot::Bottom | Slot::Trail, crate::program::Aspect::Portrait) => (cx.h * 0.54).round(),
            _ => (w * 0.618).round().min((cx.h * 0.78).round()),
        };
        (w, h)
    };

    // `size` scales the slot box (m = as above); xl fills the frame.
    let (bw, bh, center) = match sk.size {
        Size::M => (bw, bh, taste::slot_center(sk.slot, cx.aspect)),
        // Nearly the whole frame (QA reads a full-frame box as a ground).
        Size::Xl => (
            (cx.w * 0.94).round(),
            (cx.h * 0.9).round(),
            [cx.w / 2.0, cx.h / 2.0],
        ),
        s => {
            let k = if s == Size::S { 0.6 } else { 1.15 };
            (
                (bw * k).round().min(cx.w),
                (bh * k).round().min(cx.h),
                taste::slot_center(sk.slot, cx.aspect),
            )
        }
    };
    let bx = (center[0] - bw / 2.0).round();
    let by = (center[1] - bh / 2.0).round();
    let origin = [bx, by];

    let has_header = sk.plate && (sk.title.is_some() || sk.caption.is_some());
    let header_h = if has_header {
        (bh * 0.22).clamp(96.0, 176.0).round()
    } else {
        0.0
    };
    let art_h = (bh - header_h).max(120.0);
    let art_origin = [bx, by + header_h];

    let mut layers: Vec<Layer> = Vec::new();
    let mut track = Track::default();
    let enter_at = 0.0;

    // 2. Grounded drafting plate at z for 100% contrast & busy_under_type safety
    // (`plate: false`: none; the box stays transparent so the strokes can
    // sit `over` another component).
    let plate_r = (bw.min(bh) * 0.03).round().clamp(8.0, 24.0);
    if sk.plate {
        plate_layer(
            &mut layers,
            &mut track,
            id,
            &r,
            origin,
            [bw, bh],
            plate_r,
            z,
            tempo,
            enter_at,
        );
    }

    // 3. Preset-specific natural media generation
    match sk.preset {
        SketchPreset::GoldenSpiral => {
            let pad = (bw.min(art_h) * 0.06).round().max(16.0);
            let rect_w = bw - 2.0 * pad;
            let rect_h = art_h - 2.0 * pad;
            let (chambers, squares_path, spiral_path, eye_xy) =
                motion_core::brush::golden_spiral_chambers([pad, pad, rect_w, rect_h], 6);

            // Chamber 0: Delicate single-angle 45° Rotring hatch (leaving background clean negative space)
            if let Some(&c0) = chambers.first() {
                let c0_path = path::rounded_rect(c0[0], c0[1], c0[2], c0[3], 0.0);
                let hatch_spec = motion_core::brush::HatchSpec {
                    dist: 16.0,
                    angle_deg: 45.0,
                    rand: 0.08,
                    continuous: false,
                    gradient: 0.0,
                    cross_angle_deg: None,
                    wiggle: sk.wiggle,
                    seed: sk.seed.wrapping_add(1),
                };
                let hatched = motion_core::brush::hatch_path(&c0_path, &hatch_spec);
                let hatch_id = format!("{id}.hatch");
                layers.push(at_origin(
                    &hatch_id,
                    LayerKind::Path {
                        d: hatched.to_svg(),
                        fill: None,
                        stroke: Some(Stroke {
                            color: cx.paint(id, Swatch::Muted, Use::Line { meaning: true }),
                            width: 1.4,
                            brush: Some(make_brush_spec(
                                motion_core::brush::BrushPreset::Rotring,
                                1.4,
                                motion_core::brush::PressureCurve::Uniform,
                                sk.wiggle,
                                None,
                                None,
                                sk.seed.wrapping_add(1),
                            )),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    art_origin,
                    [bw, art_h],
                    z + 2,
                ));
                let hatch_start = enter_at + (length * 0.08).min(0.25);
                track.push(
                    &hatch_id,
                    hatch_start,
                    (length - hatch_start - 0.05).max(0.5),
                    Shape::Ease(Easing::Linear),
                    MotionOp::Trim { from: 0.0, to: 1.0 },
                );
            }

            // Chamber 1: Luminous Fill::Watercolor wash (wet watercolor pigment shines with no hatching)
            if let Some(&c1) = chambers.get(1) {
                let wash_spec = motion_core::brush::WatercolorSpec {
                    bleed: 0.24,
                    direction: motion_core::brush::BleedDirection::In,
                    angle_deg: Some(45.0),
                    texture: 0.55,
                    border: 0.42,
                    scatter: true,
                    seed: sk.seed,
                };
                let wash_d = path::rounded_rect(c1[0], c1[1], c1[2], c1[3], 0.0).to_svg();
                let wash_id = format!("{id}.wash");
                layers.push(at_origin(
                    &wash_id,
                    LayerKind::Path {
                        d: wash_d,
                        fill: Some(Fill::Watercolor {
                            color: cx.paint(id, Swatch::Accent, Use::Mark),
                            spec: wash_spec,
                        }),
                        stroke: None,
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    art_origin,
                    [bw, art_h],
                    z + 1,
                ));
                track.push(
                    &wash_id,
                    enter_at,
                    (length * 0.45).max(0.45),
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Fade {
                        from: 0.15,
                        to: 0.78,
                    },
                );
            }

            // Chamber 2: Cross-hatch with Rotring brush
            if let Some(&c2) = chambers.get(2) {
                let c2_path = path::rounded_rect(c2[0], c2[1], c2[2], c2[3], 0.0);
                let c2_hatch_spec = motion_core::brush::HatchSpec {
                    dist: 12.0,
                    angle_deg: -35.0,
                    rand: 0.08,
                    continuous: false,
                    gradient: 0.0,
                    cross_angle_deg: Some(55.0),
                    wiggle: sk.wiggle,
                    seed: sk.seed.wrapping_add(21),
                };
                let c2_hatched = motion_core::brush::hatch_path(&c2_path, &c2_hatch_spec);
                let hatch2_id = format!("{id}.hatch2");
                layers.push(at_origin(
                    &hatch2_id,
                    LayerKind::Path {
                        d: c2_hatched.to_svg(),
                        fill: None,
                        stroke: Some(Stroke {
                            color: cx.paint(id, Swatch::Ink, Use::Line { meaning: true }),
                            width: 1.3,
                            brush: Some(make_brush_spec(
                                motion_core::brush::BrushPreset::Rotring,
                                1.3,
                                motion_core::brush::PressureCurve::Uniform,
                                sk.wiggle,
                                None,
                                None,
                                sk.seed.wrapping_add(22),
                            )),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    art_origin,
                    [bw, art_h],
                    z + 2,
                ));
                let h2_start = enter_at + (length * 0.10).min(0.28);
                track.push(
                    &hatch2_id,
                    h2_start,
                    (length - h2_start - 0.05).max(0.5),
                    Shape::Ease(Easing::Linear),
                    MotionOp::Trim { from: 0.0, to: 1.0 },
                );
            }

            // Chamber 3: Subtle Fill::Watercolor wash
            if let Some(&c3) = chambers.get(3) {
                let c3_wash_spec = motion_core::brush::WatercolorSpec {
                    bleed: 0.20,
                    direction: motion_core::brush::BleedDirection::In,
                    angle_deg: Some(45.0),
                    texture: 0.55,
                    border: 0.42,
                    scatter: true,
                    seed: sk.seed.wrapping_add(31),
                };
                let c3_wash_d = path::rounded_rect(c3[0], c3[1], c3[2], c3[3], 0.0).to_svg();
                let wash3_id = format!("{id}.wash3");
                layers.push(at_origin(
                    &wash3_id,
                    LayerKind::Path {
                        d: c3_wash_d,
                        fill: Some(Fill::Watercolor {
                            color: cx.paint(id, Swatch::Muted, Use::Mark),
                            spec: c3_wash_spec,
                        }),
                        stroke: None,
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    art_origin,
                    [bw, art_h],
                    z + 1,
                ));
                track.push(
                    &wash3_id,
                    enter_at,
                    (length * 0.45).max(0.45),
                    Shape::Ease(Easing::OutCubic),
                    MotionOp::Fade {
                        from: 0.15,
                        to: 0.65,
                    },
                );
            }

            // Golden squares subdivision path with Rotring brush
            let squares_id = format!("{id}.squares");
            layers.push(at_origin(
                &squares_id,
                LayerKind::Path {
                    d: squares_path.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Ink, Use::Line { meaning: true }),
                        width: 2.0,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Rotring,
                            2.0,
                            motion_core::brush::PressureCurve::Uniform,
                            sk.wiggle * 0.5,
                            None,
                            None,
                            sk.seed.wrapping_add(2),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 3,
            ));
            track.push(
                &squares_id,
                enter_at,
                (length * 0.72).max(0.5),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );

            // Golden Logarithmic Spiral curve drawing in with Charcoal brush
            let spiral_id = format!("{id}.spiral");
            layers.push(at_origin(
                &spiral_id,
                LayerKind::Path {
                    d: spiral_path.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Accent, Use::Mark),
                        width: 3.8,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Charcoal,
                            3.8,
                            motion_core::brush::PressureCurve::ThreePoint([0.5, 1.3, 0.4]),
                            sk.wiggle,
                            sk.field.map(|f| f.to_core()),
                            None,
                            sk.seed.wrapping_add(3),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 4,
            ));
            let spiral_start = enter_at + (length * 0.14).min(0.35);
            track.push(
                &spiral_id,
                spiral_start,
                (length - spiral_start - 0.05).max(0.6),
                Shape::Ease(Easing::Linear),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );

            // Accent locus dot + precision crosshair ticks at eye_xy
            let dot_r = (bw.min(art_h) * 0.014).clamp(5.0, 10.0);
            let tick_gap = dot_r * 1.4;
            let tick_len = dot_r * 1.6;
            let tick_d = format!(
                "M{:.1},{:.1} L{:.1},{:.1} M{:.1},{:.1} L{:.1},{:.1} M{:.1},{:.1} L{:.1},{:.1} M{:.1},{:.1} L{:.1},{:.1}",
                eye_xy[0],
                eye_xy[1] - tick_gap - tick_len,
                eye_xy[0],
                eye_xy[1] - tick_gap,
                eye_xy[0],
                eye_xy[1] + tick_gap,
                eye_xy[0],
                eye_xy[1] + tick_gap + tick_len,
                eye_xy[0] - tick_gap - tick_len,
                eye_xy[1],
                eye_xy[0] - tick_gap,
                eye_xy[1],
                eye_xy[0] + tick_gap,
                eye_xy[1],
                eye_xy[0] + tick_gap + tick_len,
                eye_xy[1],
            );
            let eye_ticks_id = format!("{id}.eye_ticks");
            layers.push(at_origin(
                &eye_ticks_id,
                LayerKind::Path {
                    d: tick_d,
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Ink, Use::Line { meaning: true }),
                        width: 1.2,
                        brush: None,
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 5,
            ));
            track.push(
                &eye_ticks_id,
                enter_at,
                (length * 0.85).max(0.4),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );

            let dot_path = path::ellipse(
                eye_xy[0] - dot_r,
                eye_xy[1] - dot_r,
                dot_r * 2.0,
                dot_r * 2.0,
            );
            let eye_id = format!("{id}.eye");
            layers.push(at_origin(
                &eye_id,
                LayerKind::Path {
                    d: dot_path.to_svg(),
                    fill: Some(Fill::Solid(cx.paint(id, Swatch::Accent, Use::Mark))),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 6,
            ));
            track.push(
                &eye_id,
                enter_at,
                (length * 0.85).max(0.4),
                Shape::Spring(taste::Spring::Pop),
                MotionOp::Scale {
                    from: 0.25,
                    to: 1.0,
                    axis: Default::default(),
                },
            );
        }
        SketchPreset::FlowField => {
            let field = sk.field.unwrap_or(SketchField::Waves).to_core();
            let pad_x = bw * 0.08;
            let pad_y = art_h * 0.10;
            let usable_w = bw - 2.0 * pad_x;
            let usable_h = art_h - 2.0 * pad_y;
            let is_radial = matches!(
                field,
                motion_core::brush::VectorFieldPreset::Spiral
                    | motion_core::brush::VectorFieldPreset::Curved
            );

            let n_streams = 32;
            for i in 0..n_streams {
                let t = if n_streams > 1 {
                    i as f32 / (n_streams - 1) as f32
                } else {
                    0.5
                };
                let (start_x, start_y, line_len, dir_deg) = if is_radial {
                    let golden_angle = i as f32 * 2.399_963_1;
                    let radius = bw.min(art_h) * (0.14 + 0.28 * t);
                    let sx =
                        (bw * 0.5 + golden_angle.cos() * radius * 1.15).clamp(pad_x, bw - pad_x);
                    let sy = (art_h * 0.5 + golden_angle.sin() * radius * 0.85)
                        .clamp(pad_y, art_h - pad_y);
                    let len =
                        bw.min(art_h) * (0.38 + 0.16 * ((i as f32 * std::f32::consts::E).fract()));
                    (sx, sy, len, 0.0)
                } else {
                    let sy = pad_y + usable_h * t;
                    let x_offset = ((i as f32 * taste::PHI).fract() * 0.14 - 0.07) * usable_w;
                    let sx = (pad_x + x_offset).max(pad_x * 0.5);
                    let len = usable_w * (0.75 + 0.2 * ((i as f32 * std::f32::consts::E).fract()));
                    let deg = (i as f32 * std::f32::consts::PI).fract() * 20.0 - 10.0;
                    (sx, sy, len, deg)
                };
                let seed = sk.seed.wrapping_add(i as u32 * 137 + 11);

                let stream_path = motion_core::brush::flow_line(
                    [start_x, start_y],
                    line_len,
                    dir_deg,
                    field,
                    sk.wiggle,
                    [bw, art_h],
                    seed,
                );

                let (swatch, brush_preset, base_weight) = match i % 4 {
                    0 => (Swatch::Ink, motion_core::brush::BrushPreset::Hb, 1.6),
                    1 => (Swatch::Muted, motion_core::brush::BrushPreset::Pen, 2.2),
                    2 => (Swatch::Ink, motion_core::brush::BrushPreset::Rotring, 1.8),
                    _ => (Swatch::Accent, motion_core::brush::BrushPreset::Marker, 3.0),
                };

                let col = cx.paint(id, swatch, Use::Mark);
                let b_spec = make_brush_spec(
                    brush_preset,
                    base_weight,
                    motion_core::brush::PressureCurve::ThreePoint([0.25, 1.5, 0.2]),
                    sk.wiggle * 0.35,
                    None,
                    None,
                    seed,
                );

                let layer_id = format!("{id}.stream{i}");
                layers.push(at_origin(
                    &layer_id,
                    LayerKind::Path {
                        d: stream_path.to_svg(),
                        fill: None,
                        stroke: Some(Stroke {
                            color: col,
                            width: base_weight,
                            brush: Some(b_spec),
                        }),
                        fill_rule: PathFillRule::Nonzero,
                        shadows: Vec::new(),
                    },
                    art_origin,
                    [bw, art_h],
                    z + 1 + (i % 10),
                ));

                let delay = enter_at + (i as f64 / n_streams as f64) * (length * 0.68);
                let dur = (length - delay - 0.04).max(0.35);
                track.push(
                    &layer_id,
                    delay,
                    dur,
                    Shape::Ease(Easing::Linear),
                    MotionOp::Trim { from: 0.0, to: 1.0 },
                );
            }
        }
        SketchPreset::ArchitecturalHatch => {
            let pad_x = bw * 0.12;
            let pad_y = art_h * 0.12;
            let study_w = bw - 2.0 * pad_x;
            let study_h = art_h - 2.0 * pad_y;

            // Pavilion elevation rectangle
            let rw = (study_w * 0.68).round();
            let rh = (rw * 0.618).round().min(study_h * 0.72);
            let rx = ((bw - rw) / 2.0).round();
            let ry = (art_h * 0.32).round();

            let rect_path = path::rounded_rect(rx, ry, rw, rh, 0.0);

            // Watercolor accent wash inside the pavilion
            let wc_spec = motion_core::brush::WatercolorSpec {
                bleed: 0.30,
                direction: motion_core::brush::BleedDirection::In,
                angle_deg: Some(35.0),
                texture: 0.60,
                border: 0.40,
                scatter: true,
                seed: sk.seed.wrapping_add(21),
            };
            let rect_wash_id = format!("{id}.arch_wash");
            layers.push(at_origin(
                &rect_wash_id,
                LayerKind::Path {
                    d: rect_path.to_svg(),
                    fill: Some(Fill::Watercolor {
                        color: cx.paint(id, Swatch::Accent, Use::Mark),
                        spec: wc_spec,
                    }),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 1,
            ));
            track.push(
                &rect_wash_id,
                enter_at,
                (length * 0.45).max(0.45),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade {
                    from: 0.15,
                    to: 0.85,
                },
            );

            // Cross-hatching inside the pavilion rectangle
            let rect_hatch_spec = motion_core::brush::HatchSpec {
                dist: 14.0,
                angle_deg: -35.0,
                rand: 0.06,
                continuous: false,
                gradient: 0.1,
                cross_angle_deg: Some(55.0),
                wiggle: sk.wiggle * 0.4,
                seed: sk.seed.wrapping_add(27),
            };
            let hatched_rect = motion_core::brush::hatch_path(&rect_path, &rect_hatch_spec);
            let rect_hatch_id = format!("{id}.rect_hatch");
            layers.push(at_origin(
                &rect_hatch_id,
                LayerKind::Path {
                    d: hatched_rect.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Ink, Use::Line { meaning: true }),
                        width: 1.3,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Rotring,
                            1.3,
                            motion_core::brush::PressureCurve::Uniform,
                            sk.wiggle * 0.4,
                            None,
                            None,
                            sk.seed.wrapping_add(28),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 2,
            ));
            let rh_start = enter_at + (length * 0.06).min(0.2);
            track.push(
                &rect_hatch_id,
                rh_start,
                (length - rh_start - 0.04).max(0.5),
                Shape::Ease(Easing::Linear),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );

            // Intersecting arch / circle vault
            let cr = (rw * 0.30).round();
            let circle_cx = bw / 2.0;
            let circle_cy = ry;
            let arch_path = path::ellipse(circle_cx - cr, circle_cy - cr, cr * 2.0, cr * 2.0);

            let hatch_spec = motion_core::brush::HatchSpec {
                dist: 12.0,
                angle_deg: 45.0,
                rand: 0.05,
                continuous: false,
                gradient: 0.2,
                cross_angle_deg: Some(135.0),
                wiggle: sk.wiggle * 0.4,
                seed: sk.seed.wrapping_add(23),
            };
            let hatched_arch = motion_core::brush::hatch_path(&arch_path, &hatch_spec);

            let arch_hatch_id = format!("{id}.arch_hatch");
            layers.push(at_origin(
                &arch_hatch_id,
                LayerKind::Path {
                    d: hatched_arch.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Muted, Use::Line { meaning: true }),
                        width: 1.4,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Hb,
                            1.4,
                            motion_core::brush::PressureCurve::Uniform,
                            sk.wiggle * 0.4,
                            None,
                            None,
                            sk.seed.wrapping_add(24),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 3,
            ));
            let ah_start = enter_at + (length * 0.14).min(0.35);
            track.push(
                &arch_hatch_id,
                ah_start,
                (length - ah_start - 0.05).max(0.5),
                Shape::Ease(Easing::Linear),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );

            // Rotring outline on pavilion
            let rect_stroke_id = format!("{id}.arch_rect");
            layers.push(at_origin(
                &rect_stroke_id,
                LayerKind::Path {
                    d: rect_path.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Ink, Use::Mark),
                        width: 2.2,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Rotring,
                            2.2,
                            motion_core::brush::PressureCurve::Uniform,
                            sk.wiggle * 0.4,
                            None,
                            None,
                            sk.seed.wrapping_add(22),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 4,
            ));
            track.push(
                &rect_stroke_id,
                enter_at,
                (length * 0.65).max(0.5),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );

            // Rotring outline on arch
            let arch_stroke_id = format!("{id}.arch_curve");
            layers.push(at_origin(
                &arch_stroke_id,
                LayerKind::Path {
                    d: arch_path.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Ink, Use::Mark),
                        width: 2.2,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Rotring,
                            2.2,
                            motion_core::brush::PressureCurve::Uniform,
                            sk.wiggle * 0.4,
                            None,
                            None,
                            sk.seed.wrapping_add(25),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 5,
            ));
            let ac_start = enter_at + (length * 0.12).min(0.3);
            track.push(
                &arch_stroke_id,
                ac_start,
                (length * 0.75).max(0.5).min(length - ac_start),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );

            // Technical drafting datum line across bottom
            let datum_y = ry + rh;
            let datum_path = BezPath {
                contours: vec![Contour {
                    start: [rx - rw * 0.12, datum_y],
                    segs: vec![line_cubic(
                        [rx - rw * 0.12, datum_y],
                        [rx + rw * 1.12, datum_y],
                    )],
                    closed: false,
                }],
            };
            let datum_id = format!("{id}.arch_datum");
            layers.push(at_origin(
                &datum_id,
                LayerKind::Path {
                    d: datum_path.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: cx.paint(id, Swatch::Muted, Use::Line { meaning: false }),
                        width: 1.4,
                        brush: Some(make_brush_spec(
                            motion_core::brush::BrushPreset::Rotring,
                            1.4,
                            motion_core::brush::PressureCurve::Uniform,
                            0.0,
                            None,
                            None,
                            sk.seed.wrapping_add(26),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z + 6,
            ));
            track.push(
                &datum_id,
                enter_at,
                (length * 0.45).max(0.4),
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
        SketchPreset::Freeform => {}
    }

    // 4. Stagger element reveals across Advance verbs or beat cuts
    let mut steps: Vec<f64> = c
        .timeline
        .iter()
        .filter(|e| matches!(e.verb, Verb::Advance))
        .filter_map(|e| e.at)
        .collect();
    steps.sort_by(f64::total_cmp);

    let n_el = sk.elements.len();
    let dt = if n_el > 0 {
        (length * 0.6 / n_el as f64).clamp(0.08, 0.4)
    } else {
        0.15
    };

    // Render user-authored elements (Freeform or preset overlays)
    for (i, el) in sk.elements.iter().enumerate() {
        let at_i = if i < steps.len() {
            steps[i]
        } else {
            enter_at + (i as f64 * dt)
        };

        let el_seed = sk.seed.wrapping_add(i as u32 * 73 + 17);
        let (geom_path, spline_curve) =
            element_geometry(&el.kind, bw, art_h, sk.wiggle, el.wiggle, sk.field, el_seed);

        let z_base = z + 10 + (i as i32 * 3);

        // 4a. Element watercolor or wash fill
        if let Some(wc) = &el.watercolor {
            let fill_id = format!("{id}.el{i}.wc");
            let wc_spec = motion_core::brush::WatercolorSpec {
                bleed: wc.bleed.unwrap_or(0.35),
                direction: if wc.inward {
                    motion_core::brush::BleedDirection::In
                } else {
                    motion_core::brush::BleedDirection::Out
                },
                angle_deg: wc.angle_deg,
                texture: wc.texture.unwrap_or(0.55),
                border: wc.border.unwrap_or(0.45),
                scatter: true,
                seed: el_seed,
            };
            layers.push(at_origin(
                &fill_id,
                LayerKind::Path {
                    d: geom_path.to_svg(),
                    fill: Some(Fill::Watercolor {
                        color: cx.paint(id, wc.swatch, Use::Mark),
                        spec: wc_spec,
                    }),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z_base,
            ));
            track.push(
                &fill_id,
                at_i,
                0.4 * tempo,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade {
                    from: 0.0,
                    to: wc.opacity.unwrap_or(0.78),
                },
            );
        } else if let Some(wash) = el.wash {
            let fill_id = format!("{id}.el{i}.wash");
            layers.push(at_origin(
                &fill_id,
                LayerKind::Path {
                    d: geom_path.to_svg(),
                    fill: Some(Fill::Solid(cx.paint(id, wash, Use::Mark))),
                    stroke: None,
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z_base,
            ));
            track.push(
                &fill_id,
                at_i,
                0.35 * tempo,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }

        // 4b. Element interior hatching
        if let Some(hatch) = &el.hatch {
            let hatch_spec = motion_core::brush::HatchSpec {
                dist: hatch.dist.unwrap_or(14.0),
                angle_deg: hatch.angle_deg,
                rand: hatch.rand,
                continuous: hatch.continuous,
                gradient: hatch.gradient,
                cross_angle_deg: if hatch.cross {
                    Some(hatch.angle_deg + 90.0)
                } else {
                    None
                },
                wiggle: el.wiggle.unwrap_or(sk.wiggle),
                seed: el_seed.wrapping_add(3),
            };
            let hatched = motion_core::brush::hatch_path(&geom_path, &hatch_spec);
            let h_brush = hatch.brush.unwrap_or(SketchBrush::HatchBrush).to_core();
            let h_color = cx.paint(
                id,
                hatch.swatch.unwrap_or(Swatch::Ink),
                Use::Line { meaning: true },
            );
            let h_weight = hatch.weight.unwrap_or(1.0);
            let h_id = format!("{id}.el{i}.hatch");

            layers.push(at_origin(
                &h_id,
                LayerKind::Path {
                    d: hatched.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: h_color,
                        width: h_weight,
                        brush: Some(make_brush_spec(
                            h_brush,
                            h_weight,
                            motion_core::brush::PressureCurve::Uniform,
                            el.wiggle.unwrap_or(sk.wiggle),
                            sk.field.map(|f| f.to_core()),
                            None,
                            el_seed.wrapping_add(5),
                        )),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z_base + 1,
            ));
            let hatch_delay = at_i + 0.05;
            let hatch_dur = (0.6 * tempo).min(length - hatch_delay);
            track.push(
                &h_id,
                hatch_delay,
                hatch_dur,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }

        // 4c. Element contour stroke with natural media brush
        if let Some(b) = el.brush {
            let stroke_color = cx.paint(id, el.stroke_swatch, Use::Mark);
            let pressure = if el.pressure.len() == 3 {
                motion_core::brush::PressureCurve::ThreePoint([
                    el.pressure[0],
                    el.pressure[1],
                    el.pressure[2],
                ])
            } else if el.pressure.len() == 2 {
                motion_core::brush::PressureCurve::Linear([el.pressure[0], el.pressure[1]])
            } else if let Some(curve) = spline_curve {
                curve
            } else {
                motion_core::brush::PressureCurve::Uniform
            };

            let stroke_w = (el.weight * 2.0).max(0.5);
            let stroke_spec = make_brush_spec(
                b.to_core(),
                stroke_w,
                pressure,
                el.wiggle.unwrap_or(sk.wiggle),
                sk.field.map(|f| f.to_core()),
                el.tip_asset.clone(),
                el_seed.wrapping_add(7),
            );

            let stroke_id = format!("{id}.el{i}.stroke");
            layers.push(at_origin(
                &stroke_id,
                LayerKind::Path {
                    d: geom_path.to_svg(),
                    fill: None,
                    stroke: Some(Stroke {
                        color: stroke_color,
                        width: stroke_w,
                        brush: Some(stroke_spec),
                    }),
                    fill_rule: PathFillRule::Nonzero,
                    shadows: Vec::new(),
                },
                art_origin,
                [bw, art_h],
                z_base + 2,
            ));

            let stroke_dur = (0.65 * tempo).min(length - at_i);
            track.push(
                &stroke_id,
                at_i,
                stroke_dur,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Trim { from: 0.0, to: 1.0 },
            );
        }
    }

    // 5. Crisp grounded architectural header band & typography
    if has_header {
        let card_d = path::rounded_rect(0.0, 0.0, bw, header_h, plate_r).to_svg();
        let card_plate_id = format!("{id}.card.plate");

        layers.push(layer(
            &card_plate_id,
            LayerKind::Path {
                d: card_d,
                fill: Some(Fill::Solid(r.ground)),
                stroke: Some(Stroke {
                    color: r.structure_strong,
                    width: 1.5,
                    brush: None,
                }),
                fill_rule: PathFillRule::Nonzero,
                shadows: Vec::new(),
            },
            [center[0], by + header_h * 0.5],
            [bw, header_h],
            z + 200,
        ));

        track.push(
            &card_plate_id,
            enter_at + 0.1,
            0.4 * tempo,
            Shape::Spring(taste::Spring::Pop),
            MotionOp::Scale {
                from: 0.85,
                to: 1.0,
                axis: Default::default(),
            },
        );

        let mut title_fs = (cx.short * 0.054).clamp(32.0, 64.0).round();
        if let Some(t) = &sk.title {
            while title_fs > 22.0
                && cx.line_width(t, FontRole::Display, title_fs, -0.01, false) > bw * 0.86
            {
                title_fs -= 2.0;
            }
        }
        let mut caption_fs = (cx.short * 0.022).clamp(14.0, 22.0).round();
        if let Some(c) = &sk.caption {
            while caption_fs > 12.0
                && cx.line_width(c, FontRole::Mono, caption_fs, 0.06, true) > bw * 0.86
            {
                caption_fs -= 1.0;
            }
        }

        let (title_y, caption_y) = if sk.title.is_some() && sk.caption.is_some() {
            (by + header_h * 0.38, by + header_h * 0.72)
        } else {
            (by + header_h * 0.52, by + header_h * 0.52)
        };

        if let Some(t) = &sk.title {
            let title_id = format!("{id}.card.title");
            layers.push(cx.text(
                &title_id,
                t,
                FontRole::Display,
                title_fs,
                r.ink,
                -0.01,
                false,
                [center[0], title_y],
                z + 201,
            ));
            track.push(
                &title_id,
                enter_at + 0.15,
                0.4 * tempo,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }

        if let Some(c) = &sk.caption {
            let caption_id = format!("{id}.card.caption");
            layers.push(cx.text(
                &caption_id,
                c,
                FontRole::Mono,
                caption_fs,
                r.accent_text,
                0.06,
                true,
                [center[0], caption_y],
                z + 201,
            ));
            track.push(
                &caption_id,
                enter_at + 0.25,
                0.4 * tempo,
                Shape::Ease(Easing::OutCubic),
                MotionOp::Fade { from: 0.0, to: 1.0 },
            );
        }
    }

    scene(id, c.start, length, layers, track.motions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lower::{Colours, Typesetter};
    use crate::program::{Aspect, MotionProgram, Tempo};
    use std::collections::BTreeMap;

    #[test]
    fn test_sketch_preset_serde() {
        let json = r#"{"preset":"golden_spiral"}"#;
        let sk: Sketch = serde_json::from_str(json).unwrap();
        assert_eq!(sk.preset, SketchPreset::GoldenSpiral);
        assert_eq!(sk.wiggle, 1.5);
        let s = serde_json::to_string(&sk).unwrap();
        assert!(s.contains("\"golden_spiral\""));
    }

    #[test]
    fn test_sketch_brush_to_core() {
        assert_eq!(
            SketchBrush::Hb.to_core(),
            motion_core::brush::BrushPreset::Hb
        );
        assert_eq!(
            SketchBrush::TwoB.to_core(),
            motion_core::brush::BrushPreset::TwoB
        );
        assert_eq!(
            SketchBrush::TwoH.to_core(),
            motion_core::brush::BrushPreset::TwoH
        );
        assert_eq!(
            SketchBrush::Cpencil.to_core(),
            motion_core::brush::BrushPreset::Cpencil
        );
        assert_eq!(
            SketchBrush::Pen.to_core(),
            motion_core::brush::BrushPreset::Pen
        );
        assert_eq!(
            SketchBrush::Rotring.to_core(),
            motion_core::brush::BrushPreset::Rotring
        );
        assert_eq!(
            SketchBrush::Spray.to_core(),
            motion_core::brush::BrushPreset::Spray
        );
        assert_eq!(
            SketchBrush::Marker.to_core(),
            motion_core::brush::BrushPreset::Marker
        );
        assert_eq!(
            SketchBrush::Marker2.to_core(),
            motion_core::brush::BrushPreset::Marker2
        );
        assert_eq!(
            SketchBrush::Charcoal.to_core(),
            motion_core::brush::BrushPreset::Charcoal
        );
        assert_eq!(
            SketchBrush::HatchBrush.to_core(),
            motion_core::brush::BrushPreset::HatchBrush
        );
    }

    #[test]
    fn test_sketch_field_to_core() {
        assert_eq!(
            SketchField::Hand.to_core(),
            motion_core::brush::VectorFieldPreset::Hand
        );
        assert_eq!(
            SketchField::Curved.to_core(),
            motion_core::brush::VectorFieldPreset::Curved
        );
        assert_eq!(
            SketchField::Zigzag.to_core(),
            motion_core::brush::VectorFieldPreset::Zigzag
        );
        assert_eq!(
            SketchField::Waves.to_core(),
            motion_core::brush::VectorFieldPreset::Waves
        );
        assert_eq!(
            SketchField::Seabed.to_core(),
            motion_core::brush::VectorFieldPreset::Seabed
        );
        assert_eq!(
            SketchField::Spiral.to_core(),
            motion_core::brush::VectorFieldPreset::Spiral
        );
        assert_eq!(
            SketchField::Columns.to_core(),
            motion_core::brush::VectorFieldPreset::Columns
        );
    }

    #[test]
    fn test_sketch_validation_ok() {
        let sk = Sketch {
            preset: SketchPreset::GoldenSpiral,
            title: Some("DIVINE PROPORTION".into()),
            caption: Some("EUCLID'S LOGARITHMIC CURVE".into()),
            ..Default::default()
        };
        assert!(sk.check().is_empty());

        let sk_freeform = Sketch {
            preset: SketchPreset::Freeform,
            elements: vec![SketchElement {
                kind: SketchShapeKind::Line {
                    from: [0.1, 0.1],
                    to: [0.9, 0.9],
                },
                brush: Some(SketchBrush::Hb),
                stroke_swatch: Swatch::Ink,
                weight: 1.0,
                pressure: vec![0.5, 1.0],
                tip_asset: None,
                wiggle: Some(1.2),
                hatch: None,
                watercolor: None,
                wash: None,
            }],
            ..Default::default()
        };
        assert!(sk_freeform.check().is_empty());
    }

    #[test]
    fn test_sketch_validation_errors() {
        // Freeform with empty elements
        let sk_empty = Sketch {
            preset: SketchPreset::Freeform,
            elements: Vec::new(),
            ..Default::default()
        };
        let errors = sk_empty.check();
        assert!(errors.iter().any(|(k, _)| *k == ".elements"));

        // Title too long
        let sk_bad_title = Sketch {
            preset: SketchPreset::GoldenSpiral,
            title: Some("A".repeat(40)),
            ..Default::default()
        };
        let errors = sk_bad_title.check();
        assert!(errors.iter().any(|(k, _)| *k == ".title"));

        // Banned glyph
        let sk_banned = Sketch {
            preset: SketchPreset::GoldenSpiral,
            title: Some("Sent → Next".into()),
            ..Default::default()
        };
        let errors = sk_banned.check();
        assert!(errors.iter().any(|(k, _)| *k == ".title"));

        // Invalid wiggle
        let sk_bad_wiggle = Sketch {
            preset: SketchPreset::GoldenSpiral,
            wiggle: -1.0,
            ..Default::default()
        };
        let errors = sk_bad_wiggle.check();
        assert!(errors.iter().any(|(k, _)| *k == ".wiggle"));
    }

    #[test]
    fn test_arc_path_math() {
        let p = arc_path([100.0, 100.0], 50.0, 0.0, 90.0);
        assert_eq!(p.contours.len(), 1);
        let bounds = p.bounds().expect("arc has finite bounds");
        assert!(bounds[0] >= 99.0 && bounds[0] <= 151.0);
        assert!(bounds[1] >= 99.0 && bounds[1] <= 151.0);
    }

    #[test]
    fn test_scale_bez_path() {
        let raw = arc_path([10.0, 10.0], 5.0, 0.0, 180.0);
        let scaled = scale_bez_path(&raw, 2.0, 3.0);
        assert_eq!(scaled.contours.len(), raw.contours.len());
        assert!((scaled.contours[0].start[0] - raw.contours[0].start[0] * 2.0).abs() < 1e-4);
        assert!((scaled.contours[0].start[1] - raw.contours[0].start[1] * 3.0).abs() < 1e-4);
    }

    fn test_program() -> MotionProgram {
        MotionProgram {
            version: crate::program::PROGRAM_VERSION.to_string(),
            meta: crate::program::Meta {
                name: "test".into(),
                aspect: Aspect::Landscape,
                duration: 4.0,
                fps: Default::default(),
                end_beat: None,
                brief: None,
            },
            recipe: Some(crate::program::Recipe::OrangeDot),
            palette: None,
            fonts: None,
            tempo: None,
            music: None,
            film: None,
            sound: None,
            assets: Vec::new(),
            allow: Vec::new(),
            components: Vec::new(),
            camera: Vec::new(),
            transitions: Vec::new(),
        }
    }

    #[test]
    fn test_lowering_golden_spiral() {
        use crate::program::Aspect;
        let ts = Typesetter::approx();
        let p = test_program();
        let col = Colours::of(&p);
        let (w, h) = Aspect::Landscape.size();
        let cx = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: w as f32,
            h: h as f32,
            short: (w.min(h)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col,
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };

        let sk = Sketch {
            preset: SketchPreset::GoldenSpiral,
            title: Some("DIVINE PROPORTION".into()),
            caption: Some("EUCLID'S LOGARITHMIC CURVE".into()),
            ..Default::default()
        };

        let c = Component {
            id: "sk_spiral".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };

        let s = lower(&cx, &c, &sk, 1, 4.0);
        assert_eq!(s.id, "sk_spiral");
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.plate"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.wash"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.wash3"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.squares"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.hatch"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.hatch2"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.spiral"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.eye_ticks"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.eye"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.card.plate"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.card.title"));
        assert!(s.layers.iter().any(|l| l.id == "sk_spiral.card.caption"));
        assert!(!s.motions.is_empty());
    }

    #[test]
    fn test_lowering_flow_field() {
        use crate::program::Aspect;
        let ts = Typesetter::approx();
        let p = test_program();
        let col = Colours::of(&p);
        let (w, h) = Aspect::Landscape.size();
        let cx = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: w as f32,
            h: h as f32,
            short: (w.min(h)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col,
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };

        let sk = Sketch {
            preset: SketchPreset::FlowField,
            title: Some("FLOW FIELD".into()),
            field: Some(SketchField::Waves),
            ..Default::default()
        };

        let c = Component {
            id: "sk_flow".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };

        let s = lower(&cx, &c, &sk, 1, 4.0);
        assert_eq!(s.id, "sk_flow");
        assert!(s.layers.iter().any(|l| l.id == "sk_flow.stream0"));
        assert!(s.layers.iter().any(|l| l.id == "sk_flow.stream17"));
        assert!(s.layers.iter().any(|l| l.id == "sk_flow.stream31"));
        assert!(s.layers.iter().any(|l| l.id == "sk_flow.card.plate"));
    }

    #[test]
    fn test_lowering_architectural_hatch() {
        use crate::program::Aspect;
        let ts = Typesetter::approx();
        let p = test_program();
        let col = Colours::of(&p);
        let (w, h) = Aspect::Landscape.size();
        let cx = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: w as f32,
            h: h as f32,
            short: (w.min(h)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col,
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };

        let sk = Sketch {
            preset: SketchPreset::ArchitecturalHatch,
            title: Some("PANTHEON ELEVATION".into()),
            caption: Some("CROSS SECTION ARCH".into()),
            ..Default::default()
        };

        let c = Component {
            id: "sk_arch".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };

        let s = lower(&cx, &c, &sk, 1, 4.0);
        assert_eq!(s.id, "sk_arch");
        assert!(s.layers.iter().any(|l| l.id == "sk_arch.arch_wash"));
        assert!(s.layers.iter().any(|l| l.id == "sk_arch.arch_hatch"));
        assert!(s.layers.iter().any(|l| l.id == "sk_arch.arch_rect"));
        assert!(s.layers.iter().any(|l| l.id == "sk_arch.arch_curve"));
        assert!(s.layers.iter().any(|l| l.id == "sk_arch.arch_datum"));
    }

    #[test]
    fn test_lowering_freeform() {
        use crate::program::Aspect;
        let ts = Typesetter::approx();
        let p = test_program();
        let col = Colours::of(&p);
        let (w, h) = Aspect::Landscape.size();
        let cx = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: w as f32,
            h: h as f32,
            short: (w.min(h)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col,
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };

        let sk = Sketch {
            preset: SketchPreset::Freeform,
            elements: vec![
                SketchElement {
                    kind: SketchShapeKind::Rect {
                        xywh: [0.1, 0.1, 0.4, 0.4],
                        radius: 0.05,
                    },
                    brush: Some(SketchBrush::Rotring),
                    stroke_swatch: Swatch::Ink,
                    weight: 1.0,
                    pressure: Vec::new(),
                    tip_asset: None,
                    wiggle: Some(0.8),
                    hatch: Some(SketchHatch {
                        dist: Some(12.0),
                        angle_deg: 45.0,
                        cross: true,
                        rand: 0.05,
                        continuous: false,
                        gradient: 0.0,
                        brush: Some(SketchBrush::Hb),
                        swatch: Some(Swatch::Muted),
                        weight: Some(1.0),
                    }),
                    watercolor: Some(SketchWatercolor {
                        swatch: Swatch::Accent,
                        opacity: Some(0.7),
                        bleed: Some(0.3),
                        texture: Some(0.5),
                        border: Some(0.4),
                        angle_deg: Some(30.0),
                        inward: false,
                    }),
                    wash: None,
                },
                SketchElement {
                    kind: SketchShapeKind::Spline {
                        points: vec![[0.5, 0.5], [0.7, 0.3], [0.9, 0.8]],
                        pressures: vec![0.5, 1.2, 0.4],
                        curvature: 0.5,
                    },
                    brush: Some(SketchBrush::Charcoal),
                    stroke_swatch: Swatch::Accent,
                    weight: 2.0,
                    pressure: Vec::new(),
                    tip_asset: None,
                    wiggle: None,
                    hatch: None,
                    watercolor: None,
                    wash: None,
                },
            ],
            ..Default::default()
        };

        let c = Component {
            id: "sk_free".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };

        let s = lower(&cx, &c, &sk, 1, 4.0);
        assert_eq!(s.id, "sk_free");
        assert!(s.layers.iter().any(|l| l.id == "sk_free.el0.wc"));
        assert!(s.layers.iter().any(|l| l.id == "sk_free.el0.hatch"));
        assert!(s.layers.iter().any(|l| l.id == "sk_free.el0.stroke"));
        assert!(s.layers.iter().any(|l| l.id == "sk_free.el1.stroke"));
    }

    #[test]
    fn test_aspect_aware_plate_dimensions() {
        use crate::program::{Aspect, Slot};
        let ts = Typesetter::approx();
        let p = test_program();

        // Portrait Center: 90% w, 80% h
        let (pw, ph) = Aspect::Portrait.size();
        let cx_portrait = Ctx {
            follow: true,
            aspect: Aspect::Portrait,
            w: pw as f32,
            h: ph as f32,
            short: (pw.min(ph)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col: Colours::of(&p),
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };
        let sk_center = Sketch {
            preset: SketchPreset::GoldenSpiral,
            slot: Slot::Center,
            ..Default::default()
        };
        let comp = Component {
            id: "sk_port".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };
        let s_port = lower(&cx_portrait, &comp, &sk_center, 1, 4.0);
        let plate_port = s_port
            .layers
            .iter()
            .find(|l| l.id == "sk_port.plate")
            .unwrap();
        assert_eq!(plate_port.width, (pw as f32 * 0.90).round());
        assert_eq!(plate_port.height, (ph as f32 * 0.80).round());

        // Landscape Center: 88% w, 76% h
        let (lw, lh) = Aspect::Landscape.size();
        let cx_landscape = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: lw as f32,
            h: lh as f32,
            short: (lw.min(lh)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col: Colours::of(&p),
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };
        let s_land = lower(&cx_landscape, &comp, &sk_center, 1, 4.0);
        let plate_land = s_land
            .layers
            .iter()
            .find(|l| l.id == "sk_port.plate")
            .unwrap();
        assert_eq!(plate_land.width, (lw as f32 * 0.88).round());
        assert_eq!(plate_land.height, (lh as f32 * 0.76).round());
    }

    #[test]
    fn test_dedicated_architectural_header_band() {
        use crate::program::Aspect;
        let ts = Typesetter::approx();
        let p = test_program();
        let col = Colours::of(&p);
        let (w, h) = Aspect::Landscape.size();
        let cx = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: w as f32,
            h: h as f32,
            short: (w.min(h)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col,
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };

        let sk_with_header = Sketch {
            preset: SketchPreset::GoldenSpiral,
            title: Some("SACRED HARMONY".into()),
            caption: Some("GOLDEN SECTION".into()),
            ..Default::default()
        };
        let c = Component {
            id: "sk_head".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };
        let s = lower(&cx, &c, &sk_with_header, 1, 4.0);
        let header_plate = s
            .layers
            .iter()
            .find(|l| l.id == "sk_head.card.plate")
            .unwrap();
        let outer_plate = s.layers.iter().find(|l| l.id == "sk_head.plate").unwrap();
        assert_eq!(header_plate.width, outer_plate.width);
        assert!(header_plate.height >= 96.0 && header_plate.height <= 176.0);
        assert_eq!(header_plate.z_index, 201);

        // Without header: no card plate layer
        let sk_no_header = Sketch {
            preset: SketchPreset::GoldenSpiral,
            title: None,
            caption: None,
            ..Default::default()
        };
        let s_none = lower(&cx, &c, &sk_no_header, 1, 4.0);
        assert!(!s_none.layers.iter().any(|l| l.id == "sk_head.card.plate"));
    }

    #[test]
    fn test_flow_field_32_streamlines_brush_cycling() {
        use crate::program::Aspect;
        let ts = Typesetter::approx();
        let p = test_program();
        let col = Colours::of(&p);
        let (w, h) = Aspect::Landscape.size();
        let cx = Ctx {
            follow: true,
            aspect: Aspect::Landscape,
            w: w as f32,
            h: h as f32,
            short: (w.min(h)) as f32,
            fps: 60.0,
            tempo: Tempo::Normal,
            col,
            fonts: p.fonts(),
            ts: &ts,
            on: BTreeMap::new(),
        };

        let sk = Sketch {
            preset: SketchPreset::FlowField,
            title: Some("FLOW".into()),
            field: Some(SketchField::Waves),
            ..Default::default()
        };
        let c = Component {
            id: "sk_ff".into(),
            start: 0.0,
            start_beat: None,
            end_beat: None,
            end: Some(4.0),
            depth: None,
            z: None,
            tilt: None,
            over: None,
            body: crate::program::ComponentBody::Backdrop {
                preset: Default::default(),
                colour: None,
                image: None,
                image_opacity: None,
            },
            timeline: Vec::new(),
        };
        let s = lower(&cx, &c, &sk, 1, 4.0);
        for i in 0..32 {
            let stream_id = format!("sk_ff.stream{i}");
            assert!(s.layers.iter().any(|l| l.id == stream_id));
        }
    }
}
