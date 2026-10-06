//! Backdrop (0.6): the piece-wide background system chosen by the
//! TasteDirector's [`BackgroundGrammar`], animated per the style's
//! [`LayerActivityProfile`]. Spans the whole piece under every beat scene.
//!
//! - `PaperCollage` — pre-0.6 paper/flat ground + grain (byte-identical).
//! - `PaperField`   — uncoated paper + grain, quiet.
//! - `TechnicalGrid`— flat ground + precise measurement grid drifting slowly,
//!   with a structured accent pulse at each beat boundary.
//! - `PrintFields`  — bright ground + large graphic color fields that
//!   recompose at beat boundaries (and cross-fade their colors per beat when
//!   the palette trajectory is `FieldCycle`).
//! - `CleanFlat`    — plain ground.
//!
//! Z order: the timeline sorts layers globally by `(z_index, scene, layer)`,
//! so backdrop decoration lives at z 1..5 (under every beat stage, which
//! starts at z 10) and grain at z 90 (over everything). All motion times are
//! piece-time seconds; the backdrop scene starts at 0 so scene-local equals
//! global. No randomness: variation is hashed from the style seed via [`mix`].

use super::explore::contrast_ratio;
use super::taste::{Activity, BackgroundGrammar, PaletteTrajectory};
use super::{base_layer, mix, mo, rect_layer, round3, texture_layer, BeatPlan, Ctx};
use crate::checks::{DISPLAY_TEXT_PX, TEXT_CONTRAST_BODY, TEXT_CONTRAST_DISPLAY};
use crate::easing::Easing;
use crate::layout_qa::is_decorative;
use crate::scene::{
    Canvas, Color, Layer, LayerKind, Lifecycle, Material, Motion, MotionProject, ProjectMeta,
    Scene, TextAlign, TextureSpec, Theme, SCENE_VERSION,
};
use crate::style::{MaterialStyle, TextureStyle};
use crate::timeline::{evaluate_frame, ResolvedLayer};

/// The backdrop scene for the whole piece. `plans` gives beat boundaries for
/// backgrounds that recompose per beat.
pub(crate) fn backdrop_scene(ctx: &Ctx, plans: &[BeatPlan], total: f64) -> Scene {
    backdrop_scene_around(ctx, plans, total, &[])
}

/// (0.23 A4) [`backdrop_scene`] composed once the beats' text is known:
/// `text[i]` are the text boxes of beat `i` (see [`text_zones`]). Print fields
/// keep clear of them, or take a colour the text can be read on. Empty `text`
/// (a preview, a look without print fields) is exactly [`backdrop_scene`].
pub(crate) fn backdrop_scene_around(
    ctx: &Ctx,
    plans: &[BeatPlan],
    total: f64,
    text: &[Vec<TextZone>],
) -> Scene {
    let mut scene = backdrop_grammar(ctx, plans, total, text);
    // (0.10 Q) Art direction: a `grounds` plate (chosen by emotion, never by
    // story words) sits just above the paper, under every other backdrop layer.
    if let Some((art, Some(plate))) = &ctx.art {
        let mut layer = base_layer(
            "backdrop.plate",
            (0.0, 0.0, ctx.w, ctx.h),
            LayerKind::Image {
                asset: format!("asset.plate.{plate}"),
                fit: crate::scene::Fit::Cover,
                treatment: None,
                playback: None,
                insert: None,
            },
            1,
        );
        layer.opacity = art.plate_opacity;
        let at = scene
            .layers
            .iter()
            .position(|l| l.id == "backdrop.paper")
            .map(|i| i + 1)
            .unwrap_or(0);
        scene.layers.insert(at, layer);
    }
    scene
}

fn backdrop_grammar(ctx: &Ctx, plans: &[BeatPlan], total: f64, text: &[Vec<TextZone>]) -> Scene {
    // (0.14) Genre looks bring their own grounds (plates, photos, desks): a
    // clean flat ground + grain, never the playful print fields.
    if ctx
        .art
        .as_ref()
        .is_some_and(|(a, _)| a.fx.grammar.is_some())
    {
        let mut layers = vec![flat_ground(ctx)];
        layers.extend(grain(ctx));
        return assemble(total, layers, Vec::new());
    }
    match ctx.taste.background {
        BackgroundGrammar::PaperCollage | BackgroundGrammar::PaperField => paper(ctx, total),
        BackgroundGrammar::CleanFlat => {
            let mut layers = vec![flat_ground(ctx)];
            layers.extend(grain(ctx));
            assemble(total, layers, Vec::new())
        }
        BackgroundGrammar::TechnicalGrid => technical_grid(ctx, plans, total),
        BackgroundGrammar::PrintFields => print_fields(ctx, plans, total, text),
    }
}

fn assemble(total: f64, layers: Vec<Layer>, motions: Vec<Motion>) -> Scene {
    Scene {
        post: Vec::new(),
        id: "backdrop".into(),
        start_seconds: 0.0,
        duration_seconds: round3(total),
        layers,
        motions,
        camera: None,
        lifecycle: None,
    }
}

/// Flat (no paper texture) ground in the palette's paper color.
fn flat_ground(ctx: &Ctx) -> Layer {
    texture_layer(
        "backdrop.paper",
        (0.0, 0.0, ctx.w, ctx.h),
        TextureSpec {
            material: Material::Flat,
            seed: mix(ctx.style.seed, 0xBAC0),
            color: ctx.palette.paper,
            intensity: 0.0,
            scale: 8.0,
            animated: false,
        },
        0,
    )
}

/// Film grain over everything, only when the style asks for it.
fn grain(ctx: &Ctx) -> Option<Layer> {
    ctx.grain_intensity().map(|grain| {
        texture_layer(
            "backdrop.grain",
            (0.0, 0.0, ctx.w, ctx.h),
            TextureSpec {
                material: Material::Grain,
                seed: mix(ctx.style.seed, 0x6A11),
                color: ctx.palette.ink,
                intensity: grain,
                scale: 2.0,
                animated: true,
            },
            90,
        )
    })
}

// ---------------------------------------------------------------------------
// TechnicalGrid
// ---------------------------------------------------------------------------

/// Cells between major grid lines.
const MAJOR_EVERY: i32 = 4;

fn technical_grid(ctx: &Ctx, plans: &[BeatPlan], total: f64) -> Scene {
    let u = ctx.u;
    let cell = 90.0 * u;
    let (w, h) = (ctx.w, ctx.h);
    // The grid group bleeds one cell on every side so it can drift one cell.
    let (gw, gh) = (w + 2.0 * cell, h + 2.0 * cell);
    let nx = (gw / cell).ceil() as i32;
    let ny = (gh / cell).ceil() as i32;
    let ink = ctx.palette.ink;
    let hair = 1.5 * u;
    let major_w = 2.0 * u;
    // Line k sits at global (k - 1) * cell; major when that index is a multiple of 4.
    let is_major = |k: i32| (k - 1).rem_euclid(MAJOR_EVERY) == 0;

    let mut kids: Vec<Layer> = Vec::new();
    for k in 0..=nx {
        let (t, a, z) = if is_major(k) {
            (major_w, 0x22, 1)
        } else {
            (hair, 0x12, 0)
        };
        kids.push(rect_layer(
            format!("backdrop.grid.v{k}"),
            (k as f32 * cell - t / 2.0, 0.0, t, gh),
            ink.with_alpha(a),
            z,
        ));
    }
    for k in 0..=ny {
        let (t, a, z) = if is_major(k) {
            (major_w, 0x22, 1)
        } else {
            (hair, 0x14, 0)
        };
        kids.push(rect_layer(
            format!("backdrop.grid.h{k}"),
            (0.0, k as f32 * cell - t / 2.0, gw, t),
            ink.with_alpha(a),
            z,
        ));
    }
    // "+" marks at a deterministic subset of interior major intersections.
    let arm = 13.0 * u;
    for i in 1..nx {
        for j in 1..ny {
            if !is_major(i) || !is_major(j) {
                continue;
            }
            if !mix(mix(ctx.style.seed, 0x6217), (i * 64 + j) as u64).is_multiple_of(3) {
                continue;
            }
            let (cx, cy) = (i as f32 * cell, j as f32 * cell);
            let c = ink.with_alpha(0x46);
            kids.push(rect_layer(
                format!("backdrop.grid.x{i}_{j}.h"),
                (cx - arm, cy - u, 2.0 * arm, 2.0 * u),
                c,
                2,
            ));
            kids.push(rect_layer(
                format!("backdrop.grid.x{i}_{j}.v"),
                (cx - u, cy - arm, 2.0 * u, 2.0 * arm),
                c,
                2,
            ));
        }
    }

    let activity = ctx.taste.layers.background;
    let mut motions: Vec<Motion> = Vec::new();
    if activity != Activity::Still {
        // One cell over the whole piece, linear: reads as continuous travel.
        let sign = if ctx.taste.variation.mirror {
            1.0
        } else {
            -1.0
        };
        motions.push(mo::shift(
            "backdrop.grid",
            0.0,
            total.max(0.001),
            [0.0, 0.0],
            [sign * cell, sign * cell],
            Easing::Linear,
        ));
    }
    if matches!(activity, Activity::Structured | Activity::Active) {
        grid_pulses(ctx, plans, cell, gh, &mut kids, &mut motions);
    }

    let group = base_layer(
        "backdrop.grid",
        (-cell, -cell, gw, gh),
        LayerKind::Group { children: kids },
        1,
    );
    let mut layers = vec![flat_ground(ctx), group];
    layers.extend(grain(ctx));
    assemble(total, layers, motions)
}

/// One accent per beat boundary, alternating a full major line and a "+"
/// crosshair, at major positions inside the frame. Structured, not busy.
fn grid_pulses(
    ctx: &Ctx,
    plans: &[BeatPlan],
    cell: f32,
    gh: f32,
    kids: &mut Vec<Layer>,
    motions: &mut Vec<Motion>,
) {
    let u = ctx.u;
    let major = MAJOR_EVERY as f32 * cell;
    let positions = |extent: f32| -> Vec<f32> {
        (1..)
            .map(|m| m as f32 * major)
            .take_while(|x| *x <= extent * 0.9)
            .filter(|x| *x >= extent * 0.1)
            .collect()
    };
    let (xs, ys) = (positions(ctx.w), positions(ctx.h));
    if xs.is_empty() || ys.is_empty() {
        return;
    }
    let gw = ctx.w + 2.0 * cell;
    let accent = ctx.palette.accent;
    for plan in plans.iter().skip(1) {
        let i = plan.index;
        let hsh = mix(mix(ctx.style.seed, 0x91D0), i as u64);
        let (px, py) = (
            xs[(hsh & 0xFFFF) as usize % xs.len()],
            ys[((hsh >> 16) & 0xFFFF) as usize % ys.len()],
        );
        // Local (group) coordinates: global + one cell of bleed.
        let (lx, ly) = (px + cell, py + cell);
        let mut ids: Vec<String> = Vec::new();
        if i % 2 == 1 {
            let t = 2.5 * u;
            let id = format!("backdrop.grid.pulse{i}");
            if (i / 2) % 2 == 0 {
                kids.push(rect_layer(
                    id.clone(),
                    (lx - t / 2.0, 0.0, t, gh),
                    accent,
                    3,
                ));
            } else {
                kids.push(rect_layer(
                    id.clone(),
                    (0.0, ly - t / 2.0, gw, t),
                    accent,
                    3,
                ));
            }
            ids.push(id);
        } else {
            let (arm, t) = (30.0 * u, 3.0 * u);
            let a = format!("backdrop.grid.pulse{i}.h");
            let b = format!("backdrop.grid.pulse{i}.v");
            kids.push(rect_layer(
                a.clone(),
                (lx - arm, ly - t / 2.0, 2.0 * arm, t),
                accent,
                3,
            ));
            kids.push(rect_layer(
                b.clone(),
                (lx - t / 2.0, ly - arm, t, 2.0 * arm),
                accent,
                3,
            ));
            ids.push(a);
            ids.push(b);
        }
        let t0 = (plan.start - 0.05).max(0.0);
        for id in &ids {
            motions.push(mo::fade(id, t0, 0.18, 0.0, 0.6, Easing::OutCubic));
            motions.push(mo::fade(id, t0 + 0.18, 0.8, 0.6, 0.0, Easing::InOutCubic));
        }
    }
}

// ---------------------------------------------------------------------------
// PrintFields
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum FieldShape {
    Rect,
    Circle,
}

/// One field in an arrangement, in frame fractions. Rect: `(x, y)` is the
/// top-left and `(w, h)` the size, as fractions of the frame width/height.
/// Circle: `(x, y)` is the centre (fractions of the frame) and `w` the
/// diameter as a fraction of the short side.
#[derive(Clone, Copy)]
struct FieldSpec {
    shape: FieldShape,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

const fn rect(x: f32, y: f32, w: f32, h: f32) -> FieldSpec {
    FieldSpec {
        shape: FieldShape::Rect,
        x,
        y,
        w,
        h,
    }
}

const fn circle(cx: f32, cy: f32, d: f32) -> FieldSpec {
    FieldSpec {
        shape: FieldShape::Circle,
        x: cx,
        y: cy,
        w: d,
        h: d,
    }
}

/// Curated arrangements of (rect, circle, rect). Fields hug edges and corners
/// and bleed off-frame; the central text area stays clear.
const ARRANGEMENTS: [[FieldSpec; 3]; 4] = [
    [
        rect(0.50, -0.05, 0.60, 0.15),
        circle(0.10, 0.93, 0.80),
        rect(0.60, 0.74, 0.55, 0.32),
    ],
    [
        rect(-0.06, 0.05, 0.10, 0.40),
        circle(0.98, 0.04, 0.60),
        rect(0.04, 0.82, 0.72, 0.26),
    ],
    [
        rect(0.40, -0.05, 0.70, 0.13),
        circle(1.00, 0.88, 0.80),
        rect(-0.05, 0.68, 0.30, 0.42),
    ],
    [
        rect(0.90, 0.20, 0.22, 0.36),
        circle(-0.02, 0.00, 0.46),
        rect(0.16, 0.80, 0.90, 0.26),
    ],
];

/// Pixel box `(x, y, w, h)` of a field for this canvas.
fn field_box(ctx: &Ctx, s: &FieldSpec, mirror: bool) -> (f32, f32, f32, f32) {
    match s.shape {
        FieldShape::Rect => {
            let x = if mirror { 1.0 - s.x - s.w } else { s.x };
            (x * ctx.w, s.y * ctx.h, s.w * ctx.w, s.h * ctx.h)
        }
        FieldShape::Circle => {
            let d = s.w * ctx.w.min(ctx.h);
            let cx = if mirror { 1.0 - s.x } else { s.x };
            (cx * ctx.w - d / 2.0, s.y * ctx.h - d / 2.0, d, d)
        }
    }
}

/// Arrangement index used at beat `beat`.
fn arrangement_for(ctx: &Ctx, beat: usize) -> usize {
    (beat + ctx.taste.variation.fields as usize) % ARRANGEMENTS.len()
}

/// Pixel boxes of the three fields of arrangement `arr`.
fn arrangement_boxes_at(ctx: &Ctx, arr: usize) -> [(f32, f32, f32, f32); 3] {
    let arr = &ARRANGEMENTS[arr % ARRANGEMENTS.len()];
    let m = ctx.taste.variation.mirror;
    [
        field_box(ctx, &arr[0], m),
        field_box(ctx, &arr[1], m),
        field_box(ctx, &arr[2], m),
    ]
}

// ---------------------------------------------------------------------------
// (0.23 A4) Print fields that know where the text is
//
// The fields recompose at every beat boundary into one of the curated
// arrangements and take their colours from the palette. Neither used to know
// where a beat's text sits, so black type could land on a black field
// (`--variety 1`: palette 1 has a #111111 field under #111111 ink). The backdrop
// is now composed after the beats: `text_zones` reads each beat's text boxes
// off the timeline (at READ, at the READ/EVOLVE midpoint and at the end of
// EVOLVE) and `plan_fields` picks, per beat, the arrangement and the field
// colours so that no text box touches a field it cannot be read on. A beat
// whose default arrangement and colours are already clean is left exactly as
// it was.
// ---------------------------------------------------------------------------

/// Penetration (px) below which a field and a text box do not touch.
const ZONE_EPS: f32 = 0.5;
/// A text layer below this opacity is not read, so it claims no field.
const ZONE_MIN_OPACITY: f32 = 0.05;
/// Share of a text box a field must cover for the median colour around the
/// text to be the field's (the rendered contrast check reads that median).
const PIXEL_COVER: f32 = 0.6;

/// One text layer a print field must keep clear of or contrast with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TextZone {
    /// The text layer's id.
    pub layer: String,
    /// Canvas box `[x0, y0, x1, y1]` (px).
    pub rect: [f32; 4],
    /// The colour the text is drawn in (tint applied).
    pub ink: Color,
    /// WCAG contrast the ink needs on what it sits on: display text
    /// (`DISPLAY_TEXT_PX` x u and up) 3:1, smaller text 4.5:1.
    pub need: f32,
    /// A field that fails this text is changed. Always the case for text
    /// sampled at READ, where the beat is read and judged; text sampled later
    /// in the beat (the READ/EVOLVE midpoint, the end of EVOLVE) is only
    /// `hard` under variety, else it just steers the choice among the
    /// alternatives.
    pub hard: bool,
    /// Judged the way the rendered check does (no variety): text read at READ
    /// counts as unreadable on a field only when its contrast with the field
    /// is below the limit itself and the field covers at least [`PIXEL_COVER`]
    /// of the layer's box or of one of its words (`words`), as the colour
    /// around the ink would show. A compile the rendered check passes is left
    /// alone.
    pub pixel: bool,
    /// Boxes of the words of a text of several words (see [`word_boxes`]);
    /// empty for a single word.
    pub words: Vec<[f32; 4]>,
}

/// Whether the print fields of this compile need the beats' text boxes: the
/// backdrop is `PrintFields` and no genre look replaced it with a flat ground.
pub(crate) fn wants_text_zones(ctx: &Ctx) -> bool {
    ctx.taste.background == BackgroundGrammar::PrintFields
        && ctx.art.as_ref().is_none_or(|(a, _)| a.fx.grammar.is_none())
}

/// The empty backdrop scene: stands in at index 0 while the beats are built.
pub(crate) fn placeholder(total: f64) -> Scene {
    assemble(total, Vec::new(), Vec::new())
}

/// The text boxes of every beat scene (those with a lifecycle, in order), at
/// READ, at the READ/EVOLVE midpoint and at the end of EVOLVE, as the timeline
/// resolves them (camera push and stage reframes included). Decorative layers
/// (`layout_qa::is_decorative`) and layers that are not visible claim nothing.
pub(crate) fn text_zones(ctx: &Ctx, scenes: &[Scene]) -> Vec<Vec<TextZone>> {
    let project = MotionProject {
        version: SCENE_VERSION.to_string(),
        project: ProjectMeta {
            name: String::new(),
            duration_seconds: None,
            exploration: None,
            speech: None,
            art: None,
            direction: None,
        },
        canvas: Canvas {
            width: ctx.w as u32,
            height: ctx.h as u32,
            fps: 30,
            background: ctx.palette.paper,
        },
        theme: Theme::default(),
        assets: Vec::new(),
        asset_root: None,
        scenes: scenes.to_vec(),
        shared: Vec::new(),
        envelopes: Vec::new(),
    };
    // Without variety only text read at READ moves a field (a compile whose
    // READ is clean stays byte-identical); under variety nothing has to stay
    // identical, so text at every sampled time of the beat must read.
    let every_sample = ctx.direction_seed.is_some();
    let mut out = Vec::new();
    for scene in &project.scenes {
        let Some(lc) = scene.lifecycle else { continue };
        let mut zones: Vec<TextZone> = Vec::new();
        for (i, local) in sample_times(&lc).into_iter().enumerate() {
            if let Ok(resolved) = evaluate_frame(&project, sample_frame(&project, scene, local)) {
                let mut found = Vec::new();
                collect_zones(
                    &resolved.layers,
                    &scene.id,
                    ctx.u,
                    i == 0 || every_sample,
                    &mut found,
                );
                // A box that has not moved since an earlier sample adds nothing.
                for mut z in found {
                    z.pixel = !every_sample;
                    let same = |o: &TextZone| {
                        o.layer == z.layer
                            && o.rect
                                .iter()
                                .zip(&z.rect)
                                .all(|(a, b)| (a - b).abs() < ZONE_EPS)
                    };
                    if !zones.iter().any(same) {
                        zones.push(z);
                    }
                }
            }
        }
        out.push(zones);
    }
    out
}

/// Scene-local times at which a beat's text is judged: READ, the READ/EVOLVE
/// midpoint and the end of EVOLVE (when EVOLVE arrivals have settled), as
/// layout QA samples a beat.
fn sample_times(lc: &Lifecycle) -> Vec<f64> {
    let mut times = vec![lc.read];
    for t in [
        (lc.read + lc.evolve) / 2.0,
        (lc.anticipate - 0.05).max(lc.read),
    ] {
        if times.iter().all(|x| (x - t).abs() > 1e-9) {
            times.push(t);
        }
    }
    times
}

/// The frame of `project` showing scene-local time `local` of `scene`.
fn sample_frame(project: &MotionProject, scene: &Scene, local: f64) -> u32 {
    let last = project.frame_count().saturating_sub(1);
    (((scene.start_seconds + local) * f64::from(project.canvas.fps)).round() as u32).min(last)
}

/// A visible print field of a resolved frame: its shape, pixel box
/// `(x, y, w, h)` and fill.
type DrawnField = (FieldShape, (f32, f32, f32, f32), Color);

/// A text layer drawn over a print field it cannot be read on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FieldFinding {
    pub scene: String,
    pub layer: String,
    /// The text colour as drawn and the field's fill.
    pub ink: Color,
    pub field: Color,
    pub ratio: f32,
    pub need: f32,
}

/// Every text layer of a compiled project whose box touches a print field
/// (`backdrop.field*`) that its ink cannot be read on, judged where the solver
/// judges the beats (READ, the READ/EVOLVE midpoint, the end of EVOLVE). Each
/// (scene, layer, field colour) is reported once. Empty without print fields.
pub(crate) fn field_findings(project: &MotionProject) -> Vec<FieldFinding> {
    let has_fields = project
        .scenes
        .iter()
        .any(|s| s.layers.iter().any(|l| l.id.starts_with("backdrop.field")));
    if !has_fields {
        return Vec::new();
    }
    let u = project.canvas.width.min(project.canvas.height) as f32 / 1080.0;
    let mut out: Vec<FieldFinding> = Vec::new();
    for scene in &project.scenes {
        let Some(lc) = scene.lifecycle else { continue };
        for local in sample_times(&lc) {
            let Ok(resolved) = evaluate_frame(project, sample_frame(project, scene, local)) else {
                continue;
            };
            let fields: Vec<DrawnField> = resolved
                .layers
                .iter()
                .filter(|l| l.id.starts_with("backdrop.field") && l.opacity >= ZONE_MIN_OPACITY)
                .filter_map(|l| {
                    let (shape, fill) = match l.kind {
                        LayerKind::Rectangle { fill, .. } => (FieldShape::Rect, *fill),
                        LayerKind::RoundedRectangle { fill, .. } => (FieldShape::Circle, *fill),
                        _ => return None,
                    };
                    let b = canvas_box(l)?;
                    Some((shape, (b[0], b[1], b[2] - b[0], b[3] - b[1]), fill))
                })
                .collect();
            if fields.is_empty() {
                continue;
            }
            let mut zones: Vec<TextZone> = Vec::new();
            collect_zones(&resolved.layers, &scene.id, u, true, &mut zones);
            for z in &zones {
                for (shape, b, fill) in &fields {
                    let ratio = contrast_ratio(z.ink, *fill);
                    if ratio < z.need
                        && field_touches(*shape, *b, &z.rect)
                        && !out
                            .iter()
                            .any(|f| f.scene == scene.id && f.layer == z.layer && f.field == *fill)
                    {
                        out.push(FieldFinding {
                            scene: scene.id.clone(),
                            layer: z.layer.clone(),
                            ink: z.ink,
                            field: *fill,
                            ratio,
                            need: z.need,
                        });
                    }
                }
            }
        }
    }
    out
}

fn collect_zones(
    layers: &[ResolvedLayer<'_>],
    scene: &str,
    u: f32,
    hard: bool,
    out: &mut Vec<TextZone>,
) {
    for l in layers {
        if l.scene == Some(scene) {
            if let LayerKind::Text(style) = l.kind {
                let shown = l.text.as_deref().unwrap_or(style.text.as_str());
                if l.opacity >= ZONE_MIN_OPACITY
                    && !shown.trim().is_empty()
                    && !is_decorative(l.id, l.kind)
                {
                    if let Some(rect) = canvas_box(l) {
                        let t = l.transform;
                        let scale = (t.a * t.d - t.b * t.c).abs().sqrt();
                        let display = style.font_size * scale >= DISPLAY_TEXT_PX * u;
                        let need = if display {
                            TEXT_CONTRAST_DISPLAY
                        } else {
                            TEXT_CONTRAST_BODY
                        };
                        out.push(TextZone {
                            layer: l.id.to_string(),
                            rect,
                            ink: drawn_ink(l, style.color),
                            need: need as f32,
                            hard,
                            pixel: false,
                            words: word_boxes(shown, rect, style.align),
                        });
                    }
                }
            }
        }
        collect_zones(&l.children, scene, u, hard, out);
    }
}

/// Boxes of the words of `text` laid out in `rect` (`[x0, y0, x1, y1]`): the
/// lines (split at `\n`) share the box height, a line is as wide as its share
/// of the longest line and sits where `align` puts it, and a word takes the
/// characters it spans (glyphs taken as equally wide). Empty for a single word.
fn word_boxes(text: &str, rect: [f32; 4], align: TextAlign) -> Vec<[f32; 4]> {
    let lines: Vec<Vec<char>> = text.split('\n').map(|l| l.chars().collect()).collect();
    let longest = lines.iter().map(Vec::len).max().unwrap_or(0).max(1) as f32;
    let (width, height) = (rect[2] - rect[0], rect[3] - rect[1]);
    let mut out = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let n = line.len().max(1) as f32;
        let line_w = width * n / longest;
        let x0 = match align {
            TextAlign::Left => rect[0],
            TextAlign::Center => rect[0] + (width - line_w) / 2.0,
            TextAlign::Right => rect[2] - line_w,
        };
        let (y0, y1) = (
            rect[1] + height * i as f32 / lines.len() as f32,
            rect[1] + height * (i + 1) as f32 / lines.len() as f32,
        );
        let mut at = 0;
        while at < line.len() {
            if line[at].is_whitespace() {
                at += 1;
                continue;
            }
            let start = at;
            while at < line.len() && !line[at].is_whitespace() {
                at += 1;
            }
            out.push([
                x0 + line_w * start as f32 / n,
                y0,
                x0 + line_w * at as f32 / n,
                y1,
            ]);
        }
    }
    if out.len() < 2 {
        out.clear();
    }
    out
}

/// The axis-aligned canvas box of a resolved layer (through its perspective
/// homography when it has one).
fn canvas_box(l: &ResolvedLayer<'_>) -> Option<[f32; 4]> {
    let (w, h) = (l.width, l.height);
    if !(w > 0.0 && h > 0.0) {
        return None;
    }
    let mut out = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for (x, y) in [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)] {
        let (cx, cy) = match &l.projective {
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
            None => l.transform.apply(x, y),
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

/// The text colour as drawn: moved toward its tint, if any.
fn drawn_ink(l: &ResolvedLayer<'_>, c: Color) -> Color {
    match l.tint {
        Some(t) => {
            let a = t.amount.clamp(0.0, 1.0);
            let toward =
                |v: u8, to: u8| (f32::from(v) + (f32::from(to) - f32::from(v)) * a).round() as u8;
            Color::rgb(
                toward(c.r, t.color.r),
                toward(c.g, t.color.g),
                toward(c.b, t.color.b),
            )
        }
        None => c,
    }
}

/// Whether a field of `shape` with pixel box `b` touches the text box `z`.
fn field_touches(shape: FieldShape, b: (f32, f32, f32, f32), z: &[f32; 4]) -> bool {
    let (x, y, w, h) = b;
    match shape {
        FieldShape::Rect => {
            (x + w).min(z[2]) - x.max(z[0]) > ZONE_EPS && (y + h).min(z[3]) - y.max(z[1]) > ZONE_EPS
        }
        FieldShape::Circle => {
            let (cx, cy, r) = (x + w / 2.0, y + h / 2.0, w.min(h) / 2.0);
            let (nx, ny) = (cx.clamp(z[0], z[2]), cy.clamp(z[1], z[3]));
            ((cx - nx).powi(2) + (cy - ny).powi(2)).sqrt() < r - ZONE_EPS
        }
    }
}

/// Share (0..=1) of the text box `z` that a field of `shape` and box `b` covers
/// (sampled on an 8 x 8 grid).
fn covered_share(shape: FieldShape, b: (f32, f32, f32, f32), z: &[f32; 4]) -> f32 {
    const N: usize = 8;
    let mut hit = 0;
    for iy in 0..N {
        for ix in 0..N {
            let px = z[0] + (z[2] - z[0]) * (ix as f32 + 0.5) / N as f32;
            let py = z[1] + (z[3] - z[1]) * (iy as f32 + 0.5) / N as f32;
            let inside = match shape {
                FieldShape::Rect => px >= b.0 && px <= b.0 + b.2 && py >= b.1 && py <= b.1 + b.3,
                FieldShape::Circle => {
                    let (cx, cy, r) = (b.0 + b.2 / 2.0, b.1 + b.3 / 2.0, b.2.min(b.3) / 2.0);
                    (px - cx).powi(2) + (py - cy).powi(2) <= r * r
                }
            };
            hit += usize::from(inside);
        }
    }
    hit as f32 / (N * N) as f32
}

/// Whether text `z`, touched by a field of `shape` and box `b` drawn in
/// `color`, cannot be read on it.
fn blocks(z: &TextZone, shape: FieldShape, b: (f32, f32, f32, f32), color: Color) -> bool {
    let ratio = contrast_ratio(z.ink, color);
    if z.pixel && z.hard {
        let covers = |r: &[f32; 4]| covered_share(shape, b, r) >= PIXEL_COVER;
        ratio < z.need && (covers(&z.rect) || z.words.iter().any(covers))
    } else {
        ratio < z.need
    }
}

/// The colours a field may take: the palette's field colours (indices
/// `0..n`), then the card, the accent, the ink and the paper (each once, when
/// not already among them). Returns the colours and `n`.
fn field_colors(ctx: &Ctx) -> (Vec<Color>, usize) {
    let n = ctx.palette.fields.len().max(1);
    let mut out: Vec<Color> = (0..n).map(|j| ctx.palette.field(j)).collect();
    for c in [
        ctx.palette.card,
        ctx.palette.accent,
        ctx.palette.ink,
        ctx.palette.paper,
    ] {
        if !out.contains(&c) {
            out.push(c);
        }
    }
    (out, n)
}

/// How a choice ranks (lower is better): fields left unreadable at READ,
/// fields recoloured, conflicts later in the beat, repeats of a neighbouring
/// beat's arrangement, distance from the default arrangement.
type FieldScore = (usize, usize, usize, usize, usize);

/// What one beat shows: the arrangement and the colour (an index into
/// [`field_colors`]) of each of the three fields.
#[derive(Debug, Clone, Copy, PartialEq)]
struct FieldChoice {
    arr: usize,
    color: [usize; 3],
}

/// Colour indices to try for a field whose own colour is `own`, best first:
/// its own, the other field colours in cycle order, then the extras.
fn color_order(own: usize, n: usize, total: usize) -> Vec<usize> {
    std::iter::once(own)
        .chain((1..n).map(|d| (own + d) % n))
        .chain(n..total)
        .collect()
}

/// The arrangement and colours of one beat. `own` is what the beat shows
/// without any text (its default arrangement and colours): when no text read
/// at READ (`TextZone::hard`) is on a colour that cannot carry it, that is the
/// answer, and the beat is left exactly as it was. Otherwise the choice with
/// the fewest fields left unreadable at READ, then the fewest fields
/// recoloured, then the fewest later-in-the-beat conflicts, then the fewest
/// repeats of a neighbouring beat's arrangement, then the nearest arrangement
/// after the default.
fn solve_beat(
    ctx: &Ctx,
    zones: &[TextZone],
    own: FieldChoice,
    prev: Option<usize>,
    next: Option<usize>,
    colors: &[Color],
    n: usize,
) -> FieldChoice {
    let mut best: Option<(FieldScore, FieldChoice)> = None;
    for rank in 0..ARRANGEMENTS.len() {
        let arr = (own.arr + rank) % ARRANGEMENTS.len();
        let boxes = arrangement_boxes_at(ctx, arr);
        let mut choice = FieldChoice {
            arr,
            color: own.color,
        };
        let (mut failed, mut recolored, mut later) = (0, 0, 0);
        for (k, b) in boxes.iter().enumerate() {
            let shape = ARRANGEMENTS[0][k].shape;
            let hit: Vec<&TextZone> = zones
                .iter()
                .filter(|z| field_touches(shape, *b, &z.rect))
                .collect();
            if hit.is_empty() {
                continue;
            }
            // Whether `ci` carries the text of this field read at READ
            // (`hard`) or later in the beat (not `hard`).
            let reads = |ci: usize, hard: bool| {
                hit.iter()
                    .filter(|z| z.hard == hard)
                    .all(|z| !blocks(z, shape, *b, colors[ci]))
            };
            if reads(own.color[k], true) {
                later += usize::from(!reads(own.color[k], false));
                continue;
            }
            let order = color_order(own.color[k], n, colors.len());
            let pick = order
                .iter()
                .find(|&&ci| reads(ci, true) && reads(ci, false))
                .or_else(|| order.iter().find(|&&ci| reads(ci, true)));
            match pick {
                Some(&ci) => {
                    choice.color[k] = ci;
                    recolored += 1;
                    later += usize::from(!reads(ci, false));
                }
                None => {
                    // Nothing in the palette carries this text: the least bad
                    // colour (the one the worst-off text reads best on).
                    let slack = |ci: usize| {
                        hit.iter()
                            .filter(|z| z.hard)
                            .map(|z| contrast_ratio(z.ink, colors[ci]) / z.need)
                            .fold(f32::INFINITY, f32::min)
                    };
                    let mut best_ci = own.color[k];
                    let mut top = f32::NEG_INFINITY;
                    for &ci in &order {
                        if slack(ci) > top + 1e-6 {
                            top = slack(ci);
                            best_ci = ci;
                        }
                    }
                    choice.color[k] = best_ci;
                    failed += 1;
                }
            }
        }
        if rank == 0 && failed == 0 && recolored == 0 {
            return choice;
        }
        let repeats = usize::from(prev == Some(arr)) + usize::from(next == Some(arr));
        let score = (failed, recolored, later, repeats, rank);
        if best.as_ref().is_none_or(|(s, _)| score < *s) {
            best = Some((score, choice));
        }
    }
    best.map(|(_, c)| c).unwrap_or(own)
}

/// The choice of every beat. Without recomposition the fields never move, so
/// one choice has to carry the text of every beat.
fn plan_fields(
    ctx: &Ctx,
    beats: usize,
    zones: &[Vec<TextZone>],
    cycles: bool,
    recompose: bool,
) -> Vec<FieldChoice> {
    let (colors, n) = field_colors(ctx);
    let own = |i: usize| FieldChoice {
        arr: arrangement_for(ctx, i),
        color: std::array::from_fn(|k| if cycles { (k + i) % n } else { k % n }),
    };
    if !recompose {
        let all: Vec<TextZone> = zones.iter().flatten().cloned().collect();
        let one = solve_beat(ctx, &all, own(0), None, None, &colors, n);
        return vec![one; beats];
    }
    let mut out: Vec<FieldChoice> = Vec::with_capacity(beats);
    for i in 0..beats {
        let here = zones.get(i).map(Vec::as_slice).unwrap_or(&[]);
        let prev = out.last().map(|c| c.arr);
        let next = (i + 1 < beats).then(|| own(i + 1).arr);
        out.push(solve_beat(ctx, here, own(i), prev, next, &colors, n));
    }
    out
}

fn print_fields(ctx: &Ctx, plans: &[BeatPlan], total: f64, text: &[Vec<TextZone>]) -> Scene {
    let activity = ctx.taste.layers.background;
    let recompose = activity != Activity::Still;
    // Snappy when active; calmer otherwise.
    let (dur, easing) = if activity == Activity::Active {
        (0.6, Easing::OutQuint)
    } else {
        (0.9, Easing::OutCubic)
    };
    let (colors, n_colors) = field_colors(ctx);
    let cycles =
        recompose && ctx.taste.palette.trajectory == PaletteTrajectory::FieldCycle && n_colors > 1;
    // Clamped to half the box by the renderer: a circle at any field size.
    let radius = 0.5 * ctx.w.max(ctx.h);

    let beats = plans.len().max(1);
    let choices = plan_fields(ctx, beats, text, cycles, recompose);
    let choice_at = |i: usize| choices[i.min(choices.len() - 1)];
    let base = arrangement_boxes_at(ctx, choice_at(0).arr);
    let mut layers = vec![flat_ground(ctx)];
    let mut motions: Vec<Motion> = Vec::new();
    // Handoff windows: centered on the middle of each incoming overlap.
    let windows: Vec<(usize, f64)> = plans
        .iter()
        .skip(1)
        .map(|p| (p.index, (p.start + p.overlap_in * 0.5 - dur * 0.5).max(0.0)))
        .collect();

    for (k, b) in base.iter().enumerate() {
        // The colours this field shows over the piece.
        let mut shown: Vec<usize> = (0..beats).map(|i| choice_at(i).color[k]).collect();
        shown.sort_unstable();
        shown.dedup();
        // One layer per colour when it changes (cycling: every palette colour,
        // so a cycle keeps its layers), else a single layer.
        let steady = !cycles && shown.iter().all(|&c| c == k % n_colors);
        let variants: Vec<(String, Color, Option<usize>)> = if cycles {
            (0..n_colors)
                .chain(shown.iter().copied().filter(|&c| c >= n_colors))
                .map(|j| (format!("backdrop.field{k}.c{j}"), colors[j], Some(j)))
                .collect()
        } else if steady || !recompose {
            vec![(
                format!("backdrop.field{k}"),
                colors[choice_at(0).color[k]],
                None,
            )]
        } else {
            shown
                .iter()
                .map(|&j| (format!("backdrop.field{k}.c{j}"), colors[j], Some(j)))
                .collect()
        };
        for (id, color, variant) in variants {
            let kind = match ARRANGEMENTS[0][k].shape {
                FieldShape::Rect => LayerKind::Rectangle {
                    fill: color,
                    stroke: None,
                },
                FieldShape::Circle => LayerKind::RoundedRectangle {
                    fill: color,
                    radius,
                    stroke: None,
                },
            };
            layers.push(base_layer(id.clone(), *b, kind, 1 + k as i32));
            if !recompose {
                continue;
            }
            if variant.is_some_and(|j| choice_at(0).color[k] != j) {
                // Not the beat-0 color: hidden until its first cross-fade.
                motions.push(mo::fade(&id, 0.0, 0.001, 0.0, 0.0, Easing::Linear));
            }
            for &(i, t0) in &windows {
                let to = arrangement_boxes_at(ctx, choice_at(i).arr)[k];
                motions.push(mo::expand(&id, t0, dur, to, easing));
                if let Some(j) = variant {
                    // Colour `j` shows at beat i when this beat's choice is j.
                    let was = choice_at(i - 1).color[k] == j;
                    let is = choice_at(i).color[k] == j;
                    if is && !was {
                        motions.push(mo::fade(&id, t0, dur * 0.75, 0.0, 1.0, Easing::InOutCubic));
                    } else if was && !is {
                        motions.push(mo::fade(&id, t0, dur * 0.75, 1.0, 0.0, Easing::InOutCubic));
                    }
                }
            }
        }
    }
    layers.extend(grain(ctx));
    assemble(total, layers, motions)
}

/// Paper (or flat) ground + grain — the pre-0.6 backdrop, unchanged.
fn paper(ctx: &Ctx, total: f64) -> Scene {
    let paper_intensity = match (ctx.style.material, ctx.style.texture_style) {
        (MaterialStyle::Flat, _) | (_, TextureStyle::None) => 0.0,
        (MaterialStyle::Paper, TextureStyle::SubtlePrint) => 0.55,
        (MaterialStyle::Paper, TextureStyle::HeavyPrint) => 0.9,
    };
    let mut layers = vec![texture_layer(
        "backdrop.paper",
        (0.0, 0.0, ctx.w, ctx.h),
        TextureSpec {
            material: if paper_intensity > 0.0 {
                Material::Paper
            } else {
                Material::Flat
            },
            seed: mix(ctx.style.seed, 0xBAC0),
            color: ctx.palette.paper,
            intensity: paper_intensity,
            scale: 8.0,
            animated: false,
        },
        0,
    )];
    if let Some(grain) = ctx.grain_intensity() {
        layers.push(texture_layer(
            "backdrop.grain",
            (0.0, 0.0, ctx.w, ctx.h),
            TextureSpec {
                material: Material::Grain,
                seed: mix(ctx.style.seed, 0x6A11),
                color: ctx.palette.ink,
                intensity: grain,
                scale: 2.0,
                animated: true,
            },
            90,
        ));
    }
    Scene {
        post: Vec::new(),
        id: "backdrop".into(),
        start_seconds: 0.0,
        duration_seconds: round3(total),
        layers,
        motions: vec![],
        camera: None,
        lifecycle: None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::assets::AssetManifest;
    use crate::compiler::{
        plan_timing_with, resolve_taste, ApproxMeasure, AssetLibrary, FontSet, Typesetter,
    };
    use crate::intent::{CreativeIntent, Format};
    use crate::scene::MotionOp;
    use crate::style::StyleProfile;

    fn intent_json(format: &str) -> String {
        let beat = r#"{"purpose":"emphasize","statement":"Hello there","primary":{"kind":"phrase","value":"hello"}}"#;
        format!(
            r#"{{"version":"0.2","title":"t","format":"{format}","beats":[{beat},{beat},{beat},{beat}]}}"#
        )
    }

    /// Build a Ctx (+ plans, total) the way the compiler does and run `f`.
    fn with_ctx<R>(
        style_json: &str,
        format: &str,
        f: impl FnOnce(&mut Ctx, &[BeatPlan], f64) -> R,
    ) -> R {
        let style: StyleProfile = serde_json::from_str(style_json).expect("style");
        let intent: CreativeIntent = serde_json::from_str(&intent_json(format)).expect("intent");
        let taste = resolve_taste(&intent, &style, None);
        let eff = taste.effective.clone();
        let plans = plan_timing_with(&intent, &taste);
        let total = plans.last().map(|p| p.start + p.duration).unwrap_or(0.0);
        let library = AssetLibrary::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets"));
        let fonts = FontSet::for_pairing(taste.typography);
        let ts = Typesetter::new(&ApproxMeasure, &library.root, &fonts);
        let manifest = AssetManifest::empty();
        let (w, h, fmt) = match format {
            "vertical" => (1080.0, 1920.0, Format::Vertical),
            "square" => (1080.0, 1080.0, Format::Square),
            _ => (1920.0, 1080.0, Format::Landscape),
        };
        let mut ctx = Ctx {
            reveals: Default::default(),
            warnings: Vec::new(),
            direction_seed: None,
            beat_params: BTreeMap::new(),
            direction_take: None,
            direction_beats: BTreeMap::new(),
            emotion: None,
            art: None,
            w,
            h,
            u: 1.0,
            style: &eff,
            taste: taste.clone(),
            palette: taste.palette.colors.clone(),
            ts,
            library: &library,
            assets: BTreeMap::new(),
            format: fmt,
            frame: crate::compiler::layout_frame::LayoutFrame::for_format(fmt),
            manifest: &manifest,
            image_carry: Default::default(),
            focal: Default::default(),
            stack: None,
            spoken: Default::default(),
            beat_count: 1,
            sequence: Default::default(),
        };
        f(&mut ctx, &plans, total)
    }

    const TECH: &str = r#"{"tone":"technical"}"#;
    const PLAY: &str = r#"{"tone":"playful"}"#;

    fn group_children(scene: &Scene) -> &[Layer] {
        scene
            .layers
            .iter()
            .find_map(|l| match &l.kind {
                LayerKind::Group { children } if l.id == "backdrop.grid" => {
                    Some(children.as_slice())
                }
                _ => None,
            })
            .expect("grid group")
    }

    #[test]
    fn classic_backdrop_is_the_paper_scene() {
        with_ctx("{}", "vertical", |ctx, plans, total| {
            assert_eq!(ctx.taste.background, BackgroundGrammar::PaperCollage);
            assert_eq!(backdrop_scene(ctx, plans, total), paper(ctx, total));
        });
    }

    #[test]
    fn technical_grid_has_flat_ground_grid_and_linear_drift() {
        for format in ["vertical", "square", "landscape"] {
            with_ctx(TECH, format, |ctx, plans, total| {
                assert_eq!(ctx.taste.background, BackgroundGrammar::TechnicalGrid);
                let scene = backdrop_scene(ctx, plans, total);
                // Flat ground, no grain (Screen material), one grid group.
                assert!(matches!(&scene.layers[0].kind,
                    LayerKind::Texture(t) if t.material == Material::Flat));
                assert!(scene.layers.iter().all(|l| !matches!(&l.kind,
                    LayerKind::Texture(t) if t.material == Material::Grain)));
                let kids = group_children(&scene);
                assert!(kids.len() > 20, "{format}: {} grid layers", kids.len());
                // Drift: one linear cell across the whole piece.
                let drift = scene
                    .motions
                    .iter()
                    .find(|m| m.target == "backdrop.grid")
                    .expect("drift");
                assert_eq!(drift.easing, Easing::Linear);
                assert!((drift.duration - round3(total)).abs() < 2e-3);
                match &drift.op {
                    MotionOp::Move { from, to } => {
                        assert_eq!(*from, [0.0, 0.0]);
                        assert_eq!(to[0].abs(), 90.0 * ctx.u);
                        assert_eq!(to[1].abs(), 90.0 * ctx.u);
                    }
                    other => panic!("expected Move, got {other:?}"),
                }
                // Group bleeds a cell past every edge.
                let g = scene
                    .layers
                    .iter()
                    .find(|l| l.id == "backdrop.grid")
                    .expect("group");
                assert_eq!((g.x, g.y), (-90.0, -90.0));
                assert_eq!((g.width, g.height), (ctx.w + 180.0, ctx.h + 180.0));
                // Structured activity: one accent per beat boundary, all under z 5.
                let pulses = kids
                    .iter()
                    .filter(|l| l.id.starts_with("backdrop.grid.pulse"))
                    .count();
                assert!(pulses >= plans.len() - 1, "{format}: {pulses} pulse layers");
                assert!(scene
                    .layers
                    .iter()
                    .all(|l| l.z_index <= 5 || l.z_index == 90));
                assert!(kids.iter().all(|l| l.z_index <= 5));
                for i in 1..plans.len() {
                    let prefix = format!("backdrop.grid.pulse{i}");
                    let fades = scene
                        .motions
                        .iter()
                        .filter(|m| {
                            m.target.starts_with(&prefix)
                                && matches!(m.op, MotionOp::Fade { to, .. } if to > 0.5)
                        })
                        .count();
                    assert!(fades >= 1, "{format}: no pulse for boundary {i}");
                }
            });
        }
    }

    #[test]
    fn still_activity_means_no_motion() {
        for style in [TECH, PLAY] {
            with_ctx(style, "vertical", |ctx, plans, total| {
                ctx.taste.layers.background = Activity::Still;
                let scene = backdrop_scene(ctx, plans, total);
                assert!(scene.motions.is_empty(), "{style}");
            });
        }
    }

    #[test]
    fn clean_flat_is_ground_only() {
        with_ctx(TECH, "square", |ctx, plans, total| {
            ctx.taste.background = BackgroundGrammar::CleanFlat;
            let scene = backdrop_scene(ctx, plans, total);
            assert!(scene.motions.is_empty());
            assert_eq!(
                scene.layers.len(),
                1 + ctx.grain_intensity().is_some() as usize
            );
        });
    }

    #[test]
    fn print_fields_recompose_once_per_beat_boundary() {
        with_ctx(PLAY, "vertical", |ctx, plans, total| {
            assert_eq!(ctx.taste.background, BackgroundGrammar::PrintFields);
            assert_eq!(ctx.taste.layers.background, Activity::Active);
            let scene = backdrop_scene(ctx, plans, total);
            let field_layers: Vec<&Layer> = scene
                .layers
                .iter()
                .filter(|l| l.id.starts_with("backdrop.field"))
                .collect();
            assert!(field_layers.len() >= 3);
            // Ground first, grain (subtle print) last, fields between (z 1..5).
            assert!(field_layers.iter().all(|l| (1..=5).contains(&l.z_index)));
            assert_eq!(scene.layers[0].z_index, 0);
            assert_eq!(scene.layers.last().map(|l| l.z_index), Some(90));
            for l in &field_layers {
                let expands: Vec<&Motion> = scene
                    .motions
                    .iter()
                    .filter(|m| m.target == l.id && matches!(m.op, MotionOp::AccentExpand { .. }))
                    .collect();
                assert_eq!(expands.len(), plans.len() - 1, "{}", l.id);
                for (m, p) in expands.iter().zip(plans.iter().skip(1)) {
                    // The window straddles the handoff.
                    assert!(m.start <= p.start + p.overlap_in, "{}", l.id);
                    assert!(m.start + m.duration >= p.start, "{}", l.id);
                    assert!((0.4..=1.0).contains(&m.duration));
                }
            }
        });
    }

    #[test]
    fn field_cycle_crossfades_colors_per_beat() {
        with_ctx(PLAY, "vertical", |ctx, plans, total| {
            assert_eq!(ctx.taste.palette.trajectory, PaletteTrajectory::FieldCycle);
            let scene = backdrop_scene(ctx, plans, total);
            let n = ctx.palette.fields.len();
            // Exactly one color layer per field shows at every beat, and it is
            // the cycled color (opacity settled just before the next handoff).
            for k in 0..3 {
                for beat in 0..plans.len() {
                    let at = plans
                        .get(beat + 1)
                        .map(|p| p.start - 0.4)
                        .unwrap_or(f64::MAX);
                    let mut showing = Vec::new();
                    for j in 0..n {
                        let id = format!("backdrop.field{k}.c{j}");
                        let mut opacity = None;
                        for m in scene
                            .motions
                            .iter()
                            .filter(|m| m.target == id && m.start <= at)
                        {
                            if let MotionOp::Fade { from, to } = m.op {
                                opacity.get_or_insert(from);
                                opacity = Some(to);
                            }
                        }
                        // No fade started yet: the layer is at its base (1.0).
                        if opacity.unwrap_or(1.0) > 0.5 {
                            showing.push(j);
                        }
                    }
                    assert_eq!(showing, vec![(k + beat) % n], "field {k} beat {beat}");
                }
            }
        });
    }

    #[test]
    fn fields_leave_the_text_area_clear_and_cover_a_quarter_to_half() {
        for format in ["vertical", "square", "landscape"] {
            for mirror in [false, true] {
                with_ctx(PLAY, format, |ctx, _, _| {
                    for (a, arr) in ARRANGEMENTS.iter().enumerate() {
                        let boxes: Vec<(FieldShape, (f32, f32, f32, f32))> = arr
                            .iter()
                            .map(|s| (s.shape, field_box(ctx, s, mirror)))
                            .collect();
                        let covered = |px: f32, py: f32| {
                            boxes.iter().any(|(shape, (x, y, w, h))| match shape {
                                FieldShape::Rect => {
                                    px >= *x && px <= x + w && py >= *y && py <= y + h
                                }
                                FieldShape::Circle => {
                                    let (cx, cy, r) = (x + w / 2.0, y + h / 2.0, w / 2.0);
                                    (px - cx).powi(2) + (py - cy).powi(2) <= r * r
                                }
                            })
                        };
                        let (mut total, mut all, mut center, mut center_all) = (0, 0, 0, 0);
                        let n = 60;
                        for iy in 0..n {
                            for ix in 0..n {
                                let (fx, fy) =
                                    ((ix as f32 + 0.5) / n as f32, (iy as f32 + 0.5) / n as f32);
                                let hit = covered(fx * ctx.w, fy * ctx.h);
                                all += 1;
                                total += hit as u32;
                                if (0.2..0.8).contains(&fx) && (0.3..0.65).contains(&fy) {
                                    center_all += 1;
                                    center += hit as u32;
                                }
                            }
                        }
                        let frac = total as f32 / all as f32;
                        let cfrac = center as f32 / center_all as f32;
                        assert!(
                            (0.18..=0.5).contains(&frac),
                            "{format} mirror={mirror} arrangement {a}: coverage {frac:.2}"
                        );
                        assert!(
                            cfrac <= 0.08,
                            "{format} mirror={mirror} arrangement {a}: center coverage {cfrac:.2}"
                        );
                    }
                });
            }
        }
    }

    // -- (0.23 A4) text-aware fields ------------------------------------------

    const BLACK: Color = Color::rgb(0x11, 0x11, 0x11);
    const BLUE: Color = Color::rgb(0x1F, 0x3B, 0xFF);

    /// Playful print fields with halftone palette 1's colours: a blue field
    /// and a field in the ink colour itself (#111111).
    fn black_and_blue(ctx: &mut Ctx) {
        ctx.palette.fields = vec![BLUE, BLACK];
        ctx.palette.ink = BLACK;
        ctx.palette.card = Color::rgb(0xFF, 0xF4, 0xC2);
        ctx.palette.paper = Color::rgb(0xFF, 0xD2, 0x3F);
    }

    fn cycles(ctx: &Ctx) -> bool {
        ctx.taste.layers.background != Activity::Still
            && ctx.taste.palette.trajectory == PaletteTrajectory::FieldCycle
            && ctx.palette.fields.len() > 1
    }

    fn ink_zone(rect: [f32; 4]) -> TextZone {
        TextZone {
            layer: "b1.kicker".into(),
            rect,
            ink: BLACK,
            need: 3.0,
            hard: true,
            pixel: false,
            words: Vec::new(),
        }
    }

    /// Every (field, text) pair the choice leaves touching reads.
    fn assert_reads(ctx: &Ctx, choices: &[FieldChoice], zones: &[Vec<TextZone>]) {
        let (colors, _) = field_colors(ctx);
        for (i, c) in choices.iter().enumerate() {
            let boxes = arrangement_boxes_at(ctx, c.arr);
            for z in zones.get(i).map(Vec::as_slice).unwrap_or(&[]) {
                for k in 0..3 {
                    if field_touches(ARRANGEMENTS[0][k].shape, boxes[k], &z.rect) {
                        let ratio = contrast_ratio(z.ink, colors[c.color[k]]);
                        assert!(
                            ratio >= z.need,
                            "beat {i} field {k} touches {:?} at {ratio:.2}:1",
                            z.rect
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn without_text_the_backdrop_is_the_plain_one() {
        with_ctx(PLAY, "vertical", |ctx, plans, total| {
            let plain = backdrop_scene(ctx, plans, total);
            assert_eq!(backdrop_scene_around(ctx, plans, total, &[]), plain);
            let empty: Vec<Vec<TextZone>> = vec![Vec::new(); plans.len()];
            assert_eq!(backdrop_scene_around(ctx, plans, total, &empty), plain);
        });
    }

    #[test]
    fn text_over_a_field_it_cannot_be_read_on_moves_the_field_away() {
        with_ctx(PLAY, "vertical", |ctx, plans, _| {
            black_and_blue(ctx);
            // A kicker and a headline in the upper left, #111111 ink (the
            // layout of a vertical beat: both sit above the lower fields).
            let zones: Vec<Vec<TextZone>> = (0..plans.len())
                .map(|_| {
                    vec![
                        ink_zone([44.0, 143.0, 228.0, 176.0]),
                        ink_zone([44.0, 244.0, 1018.0, 432.0]),
                    ]
                })
                .collect();
            let cyc = cycles(ctx);
            let recompose = ctx.taste.layers.background != Activity::Still;
            let choices = plan_fields(ctx, plans.len(), &zones, cyc, recompose);
            assert_reads(ctx, &choices, &zones);
            // Some arrangement is clear of the block: no field is recoloured.
            let (_, n) = field_colors(ctx);
            for (i, c) in choices.iter().enumerate() {
                let own: [usize; 3] =
                    std::array::from_fn(|k| if cyc { (k + i) % n } else { k % n });
                assert_eq!(c.color, own, "beat {i} is recoloured, not moved");
            }
        });
    }

    #[test]
    fn a_beat_with_no_conflict_keeps_its_arrangement_and_colours() {
        with_ctx(PLAY, "vertical", |ctx, plans, _| {
            black_and_blue(ctx);
            let rect = [40.0, 140.0, 760.0, 450.0];
            let mut zones: Vec<Vec<TextZone>> = vec![Vec::new(); plans.len()];
            zones[1].push(ink_zone(rect));
            let cyc = cycles(ctx);
            let clean = plan_fields(ctx, plans.len(), &[], cyc, true);
            let aware = plan_fields(ctx, plans.len(), &zones, cyc, true);
            assert_reads(ctx, &aware, &zones);
            for i in 0..plans.len() {
                if i != 1 {
                    assert_eq!(aware[i], clean[i], "beat {i} had no text to avoid");
                }
            }
        });
    }

    #[test]
    fn text_that_only_conflicts_later_in_the_beat_does_not_change_a_clean_beat() {
        with_ctx(PLAY, "vertical", |ctx, plans, _| {
            black_and_blue(ctx);
            let cyc = cycles(ctx);
            let clean = plan_fields(ctx, plans.len(), &[], cyc, true);
            // For every beat: a text box over each of its default fields that
            // the field's colour cannot carry, sampled after READ (not `hard`).
            let (colors, _) = field_colors(ctx);
            let zones: Vec<Vec<TextZone>> = clean
                .iter()
                .map(|c| {
                    let boxes = arrangement_boxes_at(ctx, c.arr);
                    (0..3)
                        .map(|k| {
                            let (x, y, w, h) = boxes[k];
                            let mut z =
                                ink_zone([x + w * 0.4, y + h * 0.4, x + w * 0.6, y + h * 0.6]);
                            z.hard = false;
                            z.ink = colors[c.color[k]];
                            z
                        })
                        .collect()
                })
                .collect();
            let aware = plan_fields(ctx, plans.len(), &zones, cyc, true);
            assert_eq!(aware, clean, "READ is clean: nothing changes");
            // The same boxes read at READ do change the beats.
            let hard: Vec<Vec<TextZone>> = zones
                .iter()
                .map(|zs| {
                    zs.iter()
                        .cloned()
                        .map(|z| TextZone { hard: true, ..z })
                        .collect()
                })
                .collect();
            assert_ne!(plan_fields(ctx, plans.len(), &hard, cyc, true), clean);
        });
    }

    #[test]
    fn without_variety_only_what_the_rendered_check_would_fail_moves_a_field() {
        with_ctx(PLAY, "vertical", |ctx, plans, _| {
            black_and_blue(ctx);
            let cyc = cycles(ctx);
            let clean = plan_fields(ctx, plans.len(), &[], cyc, true);
            // Beat 0's first field (a rect), its default colour, #111111 ink on it.
            let (x, y, w, h) = arrangement_boxes_at(ctx, clean[0].arr)[0];
            let (colors, _) = field_colors(ctx);
            let pixel = |rect: [f32; 4], ink: Color| TextZone {
                pixel: true,
                ink,
                ..ink_zone(rect)
            };
            let at = |zone: TextZone| {
                let mut zones: Vec<Vec<TextZone>> = vec![Vec::new(); plans.len()];
                zones[0].push(zone);
                plan_fields(ctx, plans.len(), &zones, cyc, true)
            };
            let on_it = [x + 0.25 * w, y + 0.25 * h, x + 0.75 * w, y + 0.75 * h];
            // Half of the text over the field: the median colour around it is
            // not the field's, the rendered check passes: left alone.
            let half = [x + w - 20.0, y + 0.25 * h, x + w + 20.0, y + 0.75 * h];
            assert_eq!(at(pixel(half, colors[clean[0].color[0]])), clean);
            // The same text wholly on a field it cannot be read on: moved.
            assert_ne!(at(pixel(on_it, colors[clean[0].color[0]])), clean);
            // Wholly on a field that carries it by more than the margin: left.
            assert_eq!(at(pixel(on_it, Color::rgb(0xFF, 0xFF, 0xFF))), clean);
        });
    }

    #[test]
    fn without_variety_a_word_on_the_field_counts_as_well_as_the_whole_label() {
        with_ctx(PLAY, "vertical", |ctx, plans, _| {
            black_and_blue(ctx);
            let cyc = cycles(ctx);
            let clean = plan_fields(ctx, plans.len(), &[], cyc, true);
            let (x, y, w, h) = arrangement_boxes_at(ctx, clean[0].arr)[0];
            let (colors, _) = field_colors(ctx);
            // A label that starts on the field and runs far past its edge: the
            // field covers its first word, not 60 % of the label.
            let label = [x + w - 40.0, y + 0.3 * h, x + w + 360.0, y + 0.7 * h];
            let zone = |words: Vec<[f32; 4]>| TextZone {
                pixel: true,
                ink: colors[clean[0].color[0]],
                words,
                ..ink_zone(label)
            };
            let plan = |zone: TextZone| {
                let mut zones: Vec<Vec<TextZone>> = vec![Vec::new(); plans.len()];
                zones[0].push(zone);
                plan_fields(ctx, plans.len(), &zones, cyc, true)
            };
            assert_eq!(
                plan(zone(Vec::new())),
                clean,
                "the label alone is 10 % covered"
            );
            let first_word = [label[0], label[1], x + w, label[3]];
            let rest = [x + w + 10.0, label[1], label[2], label[3]];
            assert_ne!(
                plan(zone(vec![first_word, rest])),
                clean,
                "a word is on the field"
            );
        });
    }

    #[test]
    fn words_share_the_box_by_their_characters() {
        let rect = [0.0, 0.0, 180.0, 30.0];
        let w = word_boxes("WHAT SLEEP REPAIRS", rect, TextAlign::Left);
        assert_eq!(w.len(), 3);
        assert_eq!((w[0][0], w[0][2]), (0.0, 40.0));
        assert_eq!((w[2][0], w[2][2]), (110.0, 180.0));
        assert!(word_boxes("SLEEP", rect, TextAlign::Left).is_empty());
        // Lines share the height; a shorter line is narrower and centred.
        let two = word_boxes("TOP LINE\nA B", rect, TextAlign::Center);
        assert_eq!(two.len(), 4);
        assert_eq!((two[2][1], two[2][3]), (15.0, 30.0));
        assert!(two[2][0] > 0.0 && two[3][2] < 180.0);
    }

    #[test]
    fn unavoidable_text_recolours_the_fields_and_keeps_one_crossfade_per_boundary() {
        for activity in [Activity::Active, Activity::Quiet, Activity::Still] {
            let style = format!("{PLAY} {activity:?}");
            with_ctx(PLAY, "vertical", |ctx, plans, total| {
                ctx.taste.layers.background = activity;
                black_and_blue(ctx);
                // Text over the whole frame: no arrangement can clear it.
                let all = ink_zone([0.0, 0.0, ctx.w, ctx.h]);
                let zones: Vec<Vec<TextZone>> =
                    (0..plans.len()).map(|_| vec![all.clone()]).collect();
                let recompose = ctx.taste.layers.background != Activity::Still;
                let choices = plan_fields(ctx, plans.len(), &zones, cycles(ctx), recompose);
                assert_reads(ctx, &choices, &zones);
                let scene = backdrop_scene_around(ctx, plans, total, &zones);
                let fields: Vec<&Layer> = scene
                    .layers
                    .iter()
                    .filter(|l| l.id.starts_with("backdrop.field"))
                    .collect();
                assert!(fields.len() >= 3, "{style}");
                if !recompose {
                    // Static fields: one layer each, all readable under the text.
                    assert_eq!(fields.len(), 3, "{style}");
                    assert!(scene.motions.is_empty(), "{style}");
                    return;
                }
                // Every colour layer of a field follows each recomposition.
                for l in &fields {
                    let expands = scene
                        .motions
                        .iter()
                        .filter(|m| {
                            m.target == l.id && matches!(m.op, MotionOp::AccentExpand { .. })
                        })
                        .count();
                    assert_eq!(expands, plans.len() - 1, "{style} {}", l.id);
                }
                // Exactly one colour layer of each field shows at each beat: the
                // opacity after the last fade that started before the next handoff.
                for k in 0..3 {
                    let ids: Vec<&str> = fields
                        .iter()
                        .map(|l| l.id.as_str())
                        .filter(|id| {
                            *id == format!("backdrop.field{k}")
                                || id.starts_with(&format!("backdrop.field{k}."))
                        })
                        .collect();
                    for beat in 0..plans.len() {
                        let at = plans
                            .get(beat + 1)
                            .map(|p| p.start - 0.4)
                            .unwrap_or(f64::MAX);
                        let showing = ids
                            .iter()
                            .filter(|id| {
                                let mut opacity = None;
                                for m in scene
                                    .motions
                                    .iter()
                                    .filter(|m| m.target == **id && m.start <= at)
                                {
                                    if let MotionOp::Fade { from, to } = m.op {
                                        opacity.get_or_insert(from);
                                        opacity = Some(to);
                                    }
                                }
                                opacity.unwrap_or(1.0) > 0.5
                            })
                            .count();
                        assert_eq!(showing, 1, "{style} field {k} beat {beat}");
                    }
                }
            });
        }
    }

    #[test]
    fn text_aware_fields_are_deterministic() {
        with_ctx(PLAY, "vertical", |ctx, plans, total| {
            black_and_blue(ctx);
            let zones: Vec<Vec<TextZone>> = (0..plans.len())
                .map(|i| vec![ink_zone([40.0, 140.0 + 60.0 * i as f32, 760.0, 450.0])])
                .collect();
            let a = backdrop_scene_around(ctx, plans, total, &zones);
            let b = backdrop_scene_around(ctx, plans, total, &zones);
            assert_eq!(a, b);
        });
    }

    #[test]
    fn backdrops_are_deterministic() {
        for style in [TECH, PLAY, "{}", r#"{"tone":"editorial"}"#] {
            let a = with_ctx(style, "landscape", |c, p, t| backdrop_scene(c, p, t));
            let b = with_ctx(style, "landscape", |c, p, t| backdrop_scene(c, p, t));
            assert_eq!(a, b, "{style}");
            assert_eq!(
                serde_json::to_string(&a).expect("json"),
                serde_json::to_string(&b).expect("json")
            );
        }
    }
}
