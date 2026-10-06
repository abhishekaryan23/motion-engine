//! (0.19) Layers (`Subject::Layers`): the persistent strata column and the
//! geometry the beat builder shares with it.
//!
//! A story about layers (zones, levels, stages) used to be N unrelated
//! pictures. Here the layers are drawn once, by the engine, as full-width
//! strata that live in the BACKDROP scene under every beat of a run (a run =
//! consecutive beats whose primary is the same stack). Between beats the
//! column glides vertically so the focused layer sits at the same place on
//! screen (the descent); the focused layer is lit, the others dim, and every
//! layer carries its name and note. The beat scene on top only adds what the
//! beat is about: the headline and the secondary subject pinned inside the
//! focused layer (`grammar/layer_stack.rs`).
//!
//! The column never zooms in (`scale <= 1`): labels sit at the screen margin
//! and zooming would push them off the canvas. Only the vertical position
//! moves. Everything here is a pure function of the intent and the canvas.

use crate::compiler::typeset::{text_layer, Voice};
use crate::compiler::{base_layer, mo, rect_layer, BeatPlan, Ctx};
use crate::easing::Easing;
use crate::intent::{Beat, Layers, Relationship, Subject};
use crate::scene::{Color, Layer, LayerKind, Motion, Stroke, TextAlign};

/// Strata are wider than the canvas so a scaled column still bleeds off it.
pub(crate) const STRATA_W: f32 = 1.7;
/// Canvas z-index of the column (under every beat stage, over the paper).
const Z_COLUMN: i32 = 3;
/// Opacity of a dimmed (not focused) layer's veil and of its label.
const DIM_VEIL: f32 = 0.58;
const DIM_LABEL: f32 = 0.72;
/// Opacity of a seam between layers at rest, and when lit.
const SEAM_REST: f32 = 0.22;
/// The top share of the content canvas the beat's kicker and headline use: a
/// layer card that would sit there for a beat is hidden for that beat.
pub(crate) const TITLE_ZONE: f32 = 0.38;
/// Offset of a card from its layer's edge, and a card's approximate height
/// (both shares of the content canvas).
const EDGE: f32 = 0.035;
const CARD_H: f32 = 0.085;
const SEAM_LIT: f32 = 0.95;
/// Seconds the column takes to glide to the next beat's layer (clamped).
const GLIDE_MIN: f64 = 0.9;
const GLIDE_MAX: f64 = 1.4;
/// Seconds the column fades in at the start of a run and out at its end.
const FADE_IN: f64 = 0.9;
const FADE_OUT: f64 = 0.5;

/// Canvas geometry the column and the beat builder agree on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StackGeo {
    pub w: f32,
    pub h: f32,
    pub u: f32,
    /// Canvas y the beat content may reach (above the caption lane).
    pub content_h: f32,
    /// Left text margin.
    pub margin: f32,
}

impl StackGeo {
    /// Where the focused layer's centre sits.
    pub fn target_y(&self) -> f32 {
        0.60 * self.content_h
    }
    fn regular_h(&self) -> f32 {
        0.40 * self.content_h
    }
    fn boundary_h(&self) -> f32 {
        0.11 * self.content_h
    }
    pub fn strata_w(&self) -> f32 {
        STRATA_W * self.w
    }
}

/// What the compiler knows about the layers of the piece, decided once before
/// any beat is built.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StackPlan {
    pub geo: StackGeo,
    pub runs: Vec<Run>,
}

/// Consecutive beats about the same stack.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Run {
    /// First and last beat index (0-based, inclusive).
    pub first: usize,
    pub last: usize,
    pub stack: Layers,
}

impl StackPlan {
    pub fn new(geo: StackGeo, beats: &[Beat]) -> Option<StackPlan> {
        let runs = runs(beats);
        (!runs.is_empty()).then_some(StackPlan { geo, runs })
    }

    pub fn run_of(&self, beat: usize) -> Option<(usize, &Run)> {
        self.runs
            .iter()
            .enumerate()
            .find(|(_, r)| (r.first..=r.last).contains(&beat))
    }

    /// True when the previous beat is in the same run (the column glides in
    /// from where it was instead of appearing).
    pub fn continues(&self, beat: usize) -> bool {
        self.run_of(beat).is_some_and(|(_, r)| beat > r.first)
    }
}

fn layers_of(beat: &Beat) -> Option<&Layers> {
    match &beat.primary {
        Subject::Layers(l) => Some(l),
        _ => None,
    }
}

/// Group consecutive beats about the same stack into runs.
pub(crate) fn runs(beats: &[Beat]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for (i, beat) in beats.iter().enumerate() {
        let Some(stack) = layers_of(beat) else {
            continue;
        };
        match out.last_mut() {
            Some(run) if run.last + 1 == i && run.stack.same_stack(stack) => run.last = i,
            _ => out.push(Run {
                first: i,
                last: i,
                stack: stack.clone(),
            }),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

/// One layer of the column, in column space (top-left origin, scale 1).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Band {
    pub top: f32,
    pub h: f32,
    pub boundary: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct Bands {
    pub items: Vec<Band>,
    pub total: f32,
}

pub(crate) fn bands(geo: &StackGeo, stack: &Layers) -> Bands {
    let mut top = 0.0;
    let items = stack
        .layers
        .iter()
        .map(|l| {
            let h = if l.boundary {
                geo.boundary_h()
            } else {
                geo.regular_h()
            };
            let band = Band {
                top,
                h,
                boundary: l.boundary,
            };
            top += h;
            band
        })
        .collect();
    Bands { items, total: top }
}

/// Where the column's centre is on the canvas, and how big it is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pose {
    pub scale: f32,
    /// Canvas y of the column's centre.
    pub cy: f32,
}

impl Pose {
    /// Canvas y of a column-space y.
    pub fn y(&self, bands: &Bands, local: f32) -> f32 {
        self.cy + self.scale * (local - bands.total / 2.0)
    }
}

/// The pose that puts layer `focus` (else the whole stack) at the target.
pub(crate) fn pose(geo: &StackGeo, b: &Bands, focus: Option<usize>) -> Pose {
    match focus.and_then(|i| b.items.get(i)) {
        Some(band) => Pose {
            scale: 1.0,
            cy: geo.target_y() - (band.top + band.h / 2.0 - b.total / 2.0),
        },
        None => Pose {
            scale: (0.88 * geo.content_h / b.total).clamp(0.6, 1.0),
            cy: 0.56 * geo.content_h,
        },
    }
}

/// Canvas y range `(top, bottom)` of layer `i` under `pose`.
pub(crate) fn band_span(b: &Bands, pose: &Pose, i: usize) -> (f32, f32) {
    let band = b.items[i];
    (pose.y(b, band.top), pose.y(b, band.top + band.h))
}

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

pub(crate) fn lum(c: Color) -> f32 {
    (0.2126 * f32::from(c.r) + 0.7152 * f32::from(c.g) + 0.0722 * f32::from(c.b)) / 255.0
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color::rgb(ch(a.r, b.r), ch(a.g, b.g), ch(a.b, b.b))
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `(shallow, deep)` colours from the palette: the accent over the lighter of
/// paper and ink at the top, the darker of the two at the bottom.
fn depth_colors(ctx: &Ctx) -> (Color, Color) {
    let p = &ctx.palette;
    let (hi, lo) = if lum(p.paper) >= lum(p.ink) {
        (p.paper, p.ink)
    } else {
        (p.ink, p.paper)
    };
    (mix(p.accent, hi, 0.22), mix(lo, p.accent, 0.05))
}

/// The colour of layer `i`'s middle, before any veil: what a subject pinned
/// inside the lit layer sits on.
pub(crate) fn band_color(ctx: &Ctx, bands: &Bands, i: usize) -> Color {
    let (shallow, deep) = depth_colors(ctx);
    let band = bands.items[i];
    let t = (band.top + band.h / 2.0) / bands.total;
    let mid = mix(shallow, deep, smooth(t));
    if band.boundary {
        // (0.23) The seam is brightest in its middle (`column`: a share of
        // 0.28 + 0.5 of the bright colour there), where a subject pinned in
        // the lit seam sits: on the product path its type is coloured for
        // that, not for the halfway tone, which read as dark in a bright
        // palette (yellow type on the yellow seam, 1.9:1).
        let share = if ctx.direction_seed.is_some() {
            0.78
        } else {
            0.5
        };
        mix(mid, on_color(ctx, deep), share)
    } else {
        mid
    }
}

/// Text colour that reads on `bg`: the lighter of paper and ink on dark, the
/// darker on light.
pub(crate) fn on_color(ctx: &Ctx, bg: Color) -> Color {
    let p = &ctx.palette;
    let (hi, lo) = if lum(p.paper) >= lum(p.ink) {
        (p.paper, p.ink)
    } else {
        (p.ink, p.paper)
    };
    if lum(bg) > 0.5 {
        lo
    } else {
        hi
    }
}

// ---------------------------------------------------------------------------
// The column (backdrop)
// ---------------------------------------------------------------------------

/// A beat's emphasis: which layer is lit (`None` = the whole stack), and
/// whether the layers are shown pulled apart.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Emphasis {
    focus: Option<usize>,
    apart: bool,
}

/// A beat's own emphasis: the focus is the beat's, not the run's (a run
/// shares its layers, each beat names its own focus).
fn emphasis(beat: &Beat) -> Emphasis {
    Emphasis {
        focus: layers_of(beat).and_then(Layers::focus_index),
        apart: beat.relationship == Some(Relationship::Separate),
    }
}

/// Veil opacity of layer `i` under `e`.
fn veil(e: Emphasis, i: usize) -> f32 {
    match e.focus {
        Some(f) if f != i => DIM_VEIL,
        _ => 0.0,
    }
}

fn label_alpha(e: Emphasis, i: usize) -> f32 {
    match e.focus {
        Some(f) if f != i => DIM_LABEL,
        _ => 1.0,
    }
}

/// Opacity of layer `i`'s card at its top edge and at its bottom edge. A
/// layer above the lit one is named at its bottom (next to the action, clear
/// of the headline); every other layer is named at its top. A card that would
/// land in the headline zone under `pose` is hidden for the beat.
fn label_sides(geo: &StackGeo, bands: &Bands, pose: &Pose, e: Emphasis, i: usize) -> (f32, f32) {
    let a = label_alpha(e, i);
    let band = bands.items[i];
    let edge = EDGE * geo.content_h;
    let clear = |local_top: f32, h: f32| {
        let top = pose.y(bands, local_top);
        top + h > 0.0 && top > TITLE_ZONE * geo.content_h
    };
    let card_h = CARD_H * geo.content_h;
    let (top_a, bottom_a) = match e.focus {
        Some(f) if i < f && !band.boundary => (0.0, a),
        _ => (a, 0.0),
    };
    let top_ok = clear(band.top + edge, card_h);
    let bottom_ok = clear(band.top + band.h - edge - card_h, card_h);
    // A boundary's single card is vertically centred in its band.
    let mid_ok = clear(band.top + (band.h - card_h) / 2.0, card_h);
    if band.boundary {
        return (if mid_ok { top_a } else { 0.0 }, 0.0);
    }
    (
        if top_ok { top_a } else { 0.0 },
        if bottom_ok { bottom_a } else { 0.0 },
    )
}

/// Seam `k` of `n` layers (0 = the top edge, `n` = the bottom edge) is lit when
/// it borders the focused layer, or, when the layers are shown apart, when it
/// lies between two layers (the outer edges separate nothing).
fn seam_alpha(e: Emphasis, k: usize, n: usize) -> f32 {
    let inner = k > 0 && k < n;
    let lit = (e.apart && inner) || e.focus.is_some_and(|f| k == f || k == f + 1);
    if lit {
        SEAM_LIT
    } else {
        SEAM_REST
    }
}

/// The glide window of the transition into beat `plan`.
fn glide(plan: &BeatPlan) -> (f64, f64) {
    let dur = (plan.overlap_in + 0.55).clamp(GLIDE_MIN, GLIDE_MAX);
    (plan.start, dur.min(0.4 * plan.duration))
}

/// A label: a dark rounded card holding a layer's name and note, as a group
/// positioned by the caller (`x`, `y`). `inline` puts the note beside the name.
#[allow(clippy::too_many_arguments)]
fn label_card(
    ctx: &Ctx,
    geo: &StackGeo,
    id: &str,
    name: &str,
    note: Option<&str>,
    inline: bool,
    light: Color,
    deep: Color,
    max_w: f32,
) -> Layer {
    let u = geo.u;
    let pad = 16.0 * u;
    let gap = 0.010 * geo.content_h;
    let name_b = ctx.ts.fit_line(Voice::LABEL, name, max_w, 38.0 * u);
    let note_b = note.map(|n| ctx.ts.fit_line(Voice::BODY, n, max_w, 31.0 * u));
    let (nw, nh) = (name_b.width() * 1.02 + 2.0, name_b.height());
    let (tw, th) = note_b
        .as_ref()
        .map_or((0.0, 0.0), |b| (b.width() * 1.02 + 2.0, b.height()));
    let (w, h) = match (&note_b, inline) {
        (Some(_), true) => (nw + 2.0 * pad + tw + pad, nh.max(th) + 1.4 * pad),
        (Some(_), false) => (nw.max(tw) + 2.0 * pad, nh + gap + th + 1.6 * pad),
        (None, _) => (nw + 2.0 * pad, nh + 1.4 * pad),
    };
    let mut kids = vec![base_layer(
        format!("{id}.bg"),
        (0.0, 0.0, w, h),
        LayerKind::RoundedRectangle {
            fill: deep.with_alpha(0xDC),
            radius: 12.0 * u,
            stroke: Some(Stroke {
                color: ctx.palette.accent.with_alpha(0xB0),
                width: (2.5 * u).max(1.5),
            }),
        },
        0,
    )];
    let mut name_l = text_layer(format!("{id}.name"), &name_b, light, TextAlign::Left);
    name_l.x = pad;
    name_l.y = if inline { (h - nh) / 2.0 } else { 0.8 * pad };
    name_l.z_index = 1;
    kids.push(name_l);
    if let Some(b) = &note_b {
        let mut note_l = text_layer(format!("{id}.note"), b, light, TextAlign::Left);
        if inline {
            note_l.x = pad + nw + pad;
            note_l.y = (h - th) / 2.0;
        } else {
            note_l.x = pad;
            note_l.y = 0.8 * pad + nh + gap;
        }
        note_l.opacity = 0.85;
        note_l.z_index = 1;
        kids.push(note_l);
    }
    base_layer(
        id.to_string(),
        (0.0, 0.0, w, h),
        LayerKind::Group { children: kids },
        8,
    )
}

/// (0.23) The colours of the column's label cards: the type (the lighter of
/// paper and ink) and the card's fill (the deepest strata colour).
pub(crate) fn card_colors(ctx: &Ctx) -> (Color, Color) {
    let p = &ctx.palette;
    let light = if lum(p.paper) >= lum(p.ink) {
        p.paper
    } else {
        p.ink
    };
    (light, depth_colors(ctx).1)
}

/// (0.23) Canvas x of the right edge of layer `i`'s label card: the cards of
/// the column sit at the left margin, so this is where a picture sliding in
/// from the left must stay to the right of.
pub(crate) fn card_right(ctx: &Ctx, geo: &StackGeo, stack: &Layers, i: usize) -> f32 {
    let Some(spec) = stack.layers.get(i) else {
        return geo.margin;
    };
    let note = spec
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    // The colours do not change a card's size.
    let ink = ctx.palette.ink;
    let card = label_card(
        ctx,
        geo,
        "probe",
        &spec.name,
        note,
        false,
        ink,
        ink,
        0.5 * geo.w,
    );
    geo.margin + card.width
}

/// The column layers and motions for the backdrop scene (piece time).
pub(crate) fn columns(
    ctx: &Ctx,
    plan: &StackPlan,
    plans: &[BeatPlan],
    beats: &[Beat],
) -> (Vec<Layer>, Vec<Motion>) {
    let mut layers = Vec::new();
    let mut motions = Vec::new();
    for (ri, run) in plan.runs.iter().enumerate() {
        column(ctx, plan, ri, run, plans, beats, &mut layers, &mut motions);
    }
    (layers, motions)
}

#[allow(clippy::too_many_arguments)]
fn column(
    ctx: &Ctx,
    plan: &StackPlan,
    ri: usize,
    run: &Run,
    plans: &[BeatPlan],
    beats: &[Beat],
    layers: &mut Vec<Layer>,
    motions: &mut Vec<Motion>,
) {
    let geo = &plan.geo;
    let b = bands(geo, &run.stack);
    let n = b.items.len();
    let poses: Vec<Pose> = (run.first..=run.last)
        .map(|i| pose(geo, &b, emphasis(&beats[i]).focus))
        .collect();
    let emph: Vec<Emphasis> = (run.first..=run.last)
        .map(|i| emphasis(&beats[i]))
        .collect();
    let first_pose = poses[0];
    let (shallow, deep) = depth_colors(ctx);
    let sw = geo.strata_w();
    // In the backdrop namespace, so layout QA treats the column as decoration.
    let id = |s: &str| format!("backdrop.stack{ri}.{s}");

    let mut kids: Vec<Layer> = Vec::new();
    // 1. Strata: a vertical gradient strip by strip, per layer.
    for (i, band) in b.items.iter().enumerate() {
        let strips = ((band.h / (22.0 * geo.u)).round() as usize).clamp(4, 28);
        let sh = band.h / strips as f32;
        for m in 0..strips {
            let t_local = (m as f32 + 0.5) / strips as f32;
            let t = (band.top + t_local * band.h) / b.total;
            let mut c = mix(shallow, deep, smooth(t));
            if band.boundary {
                // A bright seam across the layer, brightest in its middle.
                let peak = 1.0 - (2.0 * t_local - 1.0).abs();
                c = mix(c, on_color(ctx, deep), 0.28 + 0.5 * peak * peak);
            } else if i % 2 == 1 {
                c = mix(c, deep, 0.07);
            }
            // Overlap strips by a pixel: no hairline gaps between them.
            kids.push(rect_layer(
                id(&format!("band{i}.s{m}")),
                (0.0, band.top + m as f32 * sh, sw, sh + 1.0),
                c,
                1,
            ));
        }
    }
    // 2. Veils: dim every layer that is not the focus.
    for (i, band) in b.items.iter().enumerate() {
        let mut veil_layer = rect_layer(
            id(&format!("veil{i}")),
            (0.0, band.top, sw, band.h),
            deep,
            4,
        );
        veil_layer.opacity = veil(emph[0], i);
        kids.push(veil_layer);
    }
    // 3. Seams between layers (and the outer edges).
    let seam_h = (5.0 * geo.u).max(2.0);
    let seam_color = on_color(ctx, deep);
    for k in 0..=n {
        let y = if k == n { b.total } else { b.items[k].top };
        let mut seam = rect_layer(
            id(&format!("seam{k}")),
            (0.0, y - seam_h / 2.0, sw, seam_h),
            mix(ctx.palette.accent, seam_color, 0.55),
            6,
        );
        seam.opacity = seam_alpha(emph[0], k, n);
        kids.push(seam);
    }
    // 4. Labels: a dark card with the name over the note (a boundary's card
    // is one line), at the screen margin. Light text on a dark card reads on
    // every layer, lit or dimmed. Every ordinary layer has a card at its top
    // edge and one at its bottom edge; which is shown depends on the beat
    // (`label_sides`).
    let x0 = (sw - geo.w) / 2.0 + geo.margin;
    let max_w = 0.5 * geo.w;
    let edge = EDGE * geo.content_h;
    let mut made: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let light = {
        let p = &ctx.palette;
        if lum(p.paper) >= lum(p.ink) {
            p.paper
        } else {
            p.ink
        }
    };
    for (i, spec) in run.stack.layers.iter().enumerate() {
        let band = b.items[i];
        let note = spec
            .note
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty());
        let (top_a, bottom_a) = label_sides(geo, &b, &poses[0], emph[0], i);
        let card = |name: &str, note: Option<&str>, inline: bool, id: String| -> Layer {
            label_card(ctx, geo, &id, name, note, inline, light, deep, max_w)
        };
        if band.boundary {
            let mut c = card(&spec.name, note, false, id(&format!("card{i}")));
            c.x = x0;
            c.y = band.top + (band.h - c.height) / 2.0;
            c.opacity = top_a;
            made.insert(c.id.clone());
            kids.push(c);
            continue;
        }
        let mut top = card(&spec.name, note, false, id(&format!("card{i}")));
        top.x = x0;
        top.y = band.top + edge;
        top.opacity = top_a;
        let mut bottom = card(&spec.name, note, false, id(&format!("bcard{i}")));
        bottom.x = x0;
        bottom.y = band.top + band.h - edge - bottom.height;
        bottom.opacity = bottom_a;
        for c in [top, bottom] {
            made.insert(c.id.clone());
            kids.push(c);
        }
    }

    // A fade motion multiplies the layer's own opacity, so a layer that is
    // animated takes its starting value from a hold-fade at the start of the
    // piece and keeps a static opacity of 1 (a static 0 would never show).
    let holds: Vec<(String, f32)> = kids
        .iter_mut()
        .filter(|k| (k.opacity - 1.0).abs() > 1e-4)
        .map(|k| {
            let v = k.opacity;
            k.opacity = 1.0;
            (k.id.clone(), v)
        })
        .collect();
    let mut group = base_layer(
        id("column"),
        (geo.w / 2.0, first_pose.cy, sw, b.total),
        LayerKind::Group { children: kids },
        Z_COLUMN,
    );
    group.anchor_x = 0.5;
    group.anchor_y = 0.5;
    group.scale_x = first_pose.scale;
    group.scale_y = first_pose.scale;
    group.opacity = 1.0;
    layers.push(group);

    // --- Motion (piece time) -------------------------------------------
    let col = id("column");
    let first_plan = &plans[run.first];
    let last_plan = &plans[run.last];
    for (hold_id, v) in holds {
        motions.push(mo::fade(&hold_id, 0.0, 0.01, v, v, Easing::Linear));
    }
    // Appear.
    motions.push(mo::fade(
        &col,
        first_plan.start,
        FADE_IN,
        0.0,
        1.0,
        Easing::OutCubic,
    ));
    // Each following beat: glide to its layer and re-light.
    for (k, i) in (run.first + 1..=run.last).enumerate() {
        let (from, to) = (poses[k], poses[k + 1]);
        let (start, dur) = glide(&plans[i]);
        let rel_from = from.cy - first_pose.cy;
        let rel_to = to.cy - first_pose.cy;
        if (rel_from - rel_to).abs() > 0.5 {
            motions.push(mo::shift(
                &col,
                start,
                dur,
                [0.0, rel_from],
                [0.0, rel_to],
                Easing::InOutCubic,
            ));
        }
        if (from.scale - to.scale).abs() > 1e-4 {
            motions.push(mo::scale(
                &col,
                start,
                dur,
                from.scale / first_pose.scale,
                to.scale / first_pose.scale,
                Easing::InOutCubic,
            ));
        }
        let (e0, e1) = (emph[k], emph[k + 1]);
        for li in 0..n {
            let (v0, v1) = (veil(e0, li), veil(e1, li));
            if (v0 - v1).abs() > 1e-4 {
                motions.push(mo::fade(
                    &id(&format!("veil{li}")),
                    start,
                    dur,
                    v0,
                    v1,
                    Easing::InOutCubic,
                ));
            }
            let (t0, b0) = label_sides(geo, &b, &from, e0, li);
            let (t1, b1) = label_sides(geo, &b, &to, e1, li);
            for (part, from, to) in [("card", t0, t1), ("bcard", b0, b1)] {
                let target = id(&format!("{part}{li}"));
                if (from - to).abs() > 1e-4 && made.contains(&target) {
                    motions.push(mo::fade(&target, start, dur, from, to, Easing::InOutCubic));
                }
            }
        }
        for k_seam in 0..=n {
            let (s0, s1) = (seam_alpha(e0, k_seam, n), seam_alpha(e1, k_seam, n));
            if (s0 - s1).abs() > 1e-4 {
                motions.push(mo::fade(
                    &id(&format!("seam{k_seam}")),
                    start,
                    dur,
                    s0,
                    s1,
                    Easing::InOutCubic,
                ));
            }
        }
    }
    // Disappear after the run, unless it is the end of the piece.
    if run.last + 1 < plans.len() {
        let end = last_plan.start + last_plan.duration;
        motions.push(mo::fade(
            &col,
            end - FADE_OUT,
            FADE_OUT,
            1.0,
            0.0,
            Easing::InCubic,
        ));
    }
}

/// Scene-local seconds after which the beat's own content may arrive: the
/// column has glided to the beat's layer (or has faded in).
pub(crate) fn settled_at(plan: &BeatPlan, continues: bool) -> f64 {
    if continues {
        let (_, dur) = glide(plan);
        dur + 0.05
    } else {
        0.55 * FADE_IN
    }
}
