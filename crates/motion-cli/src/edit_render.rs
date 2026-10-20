//! (0.24 E1) Overlay layout + MotionScene construction + segmented render for
//! `motion-engine edit`. The source video is the full-bleed base (M6 plate
//! decode); overlays are small scenes beside the speaker, never over the face
//! box. No semantic decisions here beyond placement: the plan decides what.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use motion_core::edit::rules::caption_chunks;
use motion_core::edit::{
    face_union, place, zone, EditEvent, EditPlan, EventKind, FaceSeg, Rect, Side, Slot, Tok,
};
use motion_core::scene::{
    Asset, AssetKind, Canvas, Color, Direction, Fit, FontRole, Layer, LayerKind, Motion, MotionOp,
    MotionProject, ProjectMeta, RevealMode, Scene, SpriteMode, SpritePlayback, SpriteSpec,
    TextAlign, TextStyle, Theme,
};
use motion_core::{Easing, TextMeasure};
use motion_render::export::{render_to_mp4_stream_quality, Quality, RenderOptions};
use motion_render::FontMeasure;

const WHITE: Color = Color::rgb(0xF2, 0xEF, 0xE8);
const SHADOW: Color = Color::rgba(0, 0, 0, 150);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Face {
    Anton,
    Semi,
}

pub struct Fonts {
    pub anton: PathBuf,
    pub semi: PathBuf,
    pub regular: PathBuf,
}

impl Fonts {
    pub fn in_assets(assets: &Path) -> Self {
        let d = assets.join("fonts");
        Fonts {
            anton: d.join("Anton-Regular.ttf"),
            semi: d.join("FiraSans-SemiBold.ttf"),
            regular: d.join("FiraSans-Regular.ttf"),
        }
    }
    pub fn all(&self) -> [&Path; 3] {
        [&self.anton, &self.semi, &self.regular]
    }
    fn path(&self, f: Face) -> &Path {
        match f {
            Face::Anton => &self.anton,
            Face::Semi => &self.semi,
        }
    }
}

pub struct Ctx<'a> {
    pub canvas: (u32, u32),
    pub fps: u32,
    pub measure: &'a FontMeasure,
    pub fonts: &'a Fonts,
    pub accent: Color,
    pub face: &'a [FaceSeg],
}

impl Ctx<'_> {
    /// Pixel unit: 1.0 at 1080 p.
    pub fn u(&self) -> f32 {
        self.canvas.1 as f32 / 1080.0
    }
    fn cv(&self) -> (f32, f32) {
        (self.canvas.0 as f32, self.canvas.1 as f32)
    }
    fn width(&self, f: Face, text: &str, size: f32, ls: f32) -> f32 {
        let weight = if f == Face::Anton { 400 } else { 600 };
        self.measure
            .line_width(self.fonts.path(f), weight, false, text, size, ls)
    }
}

/// One placed line of overlay text (canvas pixels).
#[derive(Debug, Clone)]
pub struct PlacedLine {
    face: Face,
    text: String,
    size: f32,
    ls: f32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    align: TextAlign,
}

#[derive(Debug, Clone)]
pub struct Placed {
    pub side: Side,
    pub rect: Rect,
    lines: Vec<PlacedLine>,
    rule: Rect,
    /// A proportion bar `(track rect, fill 0..1)` under the lines (side panels).
    bar: Option<(Rect, f32)>,
}

struct Spec {
    face: Face,
    text: String,
    size: f32,
    ls: f32,
    gap: f32,
}

fn wrap_two(text: &str) -> Option<(String, String)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() < 2 {
        return None;
    }
    let best = (1..words.len())
        .min_by_key(|&k| {
            let a = words[..k].join(" ").len() as i64;
            let b = words[k..].join(" ").len() as i64;
            (a - b).abs()
        })
        .unwrap_or(1);
    Some((words[..best].join(" "), words[best..].join(" ")))
}

/// Lay `ev` out beside the face on `side`; `None` if it cannot fit.
fn layout_side(ctx: &Ctx, ev: &EditEvent, side: Side) -> Option<Placed> {
    let u = ctx.u();
    let face = face_union(ctx.face, ctx.cv(), ev.start, ev.end);
    let (z0, z1) = zone(ctx.cv(), face, side)?;
    let zw = z1 - z0;
    let (slot, main_face, mut main_size, main_text, sub): (
        Slot,
        Face,
        f32,
        String,
        Option<String>,
    ) = match &ev.kind {
        EventKind::NumberCallout { value } => {
            let (m, s) = split_value(value);
            (
                Slot::Mid,
                Face::Anton,
                150.0 * u,
                m,
                s.map(|s| s.to_uppercase()),
            )
        }
        EventKind::Keyword { text } => {
            (Slot::Top, Face::Anton, 130.0 * u, text.to_uppercase(), None)
        }
        EventKind::ChapterCard { title, index } => (
            Slot::Top,
            Face::Anton,
            92.0 * u,
            title.to_uppercase(),
            Some(format!("CHAPTER {index:02}")),
        ),
        EventKind::LowerThird { text } => (Slot::Bottom, Face::Semi, 50.0 * u, text.clone(), None),
        EventKind::SidePanel { value, label, .. } => (
            Slot::Mid,
            Face::Anton,
            if value.chars().count() > 8 {
                96.0 * u
            } else {
                150.0 * u
            },
            value.clone(),
            Some(label.clone()),
        ),
        EventKind::Captions {} | EventKind::Cutaway { .. } | EventKind::PunchIn {} => return None,
    };
    let bar_frac = match &ev.kind {
        EventKind::SidePanel { bar, .. } => *bar,
        _ => None,
    };
    let chapter = matches!(ev.kind, EventKind::ChapterCard { .. });
    // Fit the main line (wrapping a chapter title onto two lines when it helps).
    let mut main_lines: Vec<String>;
    loop {
        let w1 = ctx.width(main_face, &main_text, main_size, 0.0);
        if w1 <= zw {
            main_lines = vec![main_text.clone()];
            break;
        }
        if let Some((a, b)) = wrap_two(&main_text).filter(|_| chapter || main_face == Face::Semi) {
            let w2 = ctx
                .width(main_face, &a, main_size, 0.0)
                .max(ctx.width(main_face, &b, main_size, 0.0));
            if w2 <= zw {
                main_lines = vec![a, b];
                break;
            }
        }
        main_size *= 0.92;
        if main_size < 34.0 * u {
            return None;
        }
    }
    let mut specs: Vec<Spec> = Vec::new();
    if chapter {
        if let Some(k) = &sub {
            specs.push(Spec {
                face: Face::Semi,
                text: k.clone(),
                size: 32.0 * u,
                ls: 0.14,
                gap: 0.0,
            });
        }
    }
    for (i, l) in main_lines.drain(..).enumerate() {
        specs.push(Spec {
            face: main_face,
            text: l,
            size: main_size,
            ls: 0.0,
            gap: if i == 0 && chapter { 6.0 * u } else { 0.0 },
        });
    }
    if !chapter {
        if let Some(s) = sub {
            let mut size = 50.0 * u;
            while ctx.width(Face::Semi, &s, size, 0.06) > zw && size > 26.0 * u {
                size *= 0.92;
            }
            specs.push(Spec {
                face: Face::Semi,
                text: s,
                size,
                ls: 0.06,
                gap: 8.0 * u,
            });
        }
    }
    let rule_h = 8.0 * u;
    let rule_gap = 16.0 * u;
    let bar_h = if bar_frac.is_some() { 30.0 * u } else { 0.0 };
    let widths: Vec<f32> = specs
        .iter()
        .map(|s| ctx.width(s.face, &s.text, s.size, s.ls) + 4.0)
        .collect();
    let bw = widths.iter().cloned().fold(100.0 * u, f32::max).min(zw);
    let mut bh = rule_h + rule_gap + bar_h;
    for s in &specs {
        bh += s.gap + s.size * 1.2;
    }
    let rect = place(ctx.cv(), face, side, slot, (bw, bh))?;
    let align = if side == Side::Right {
        TextAlign::Right
    } else {
        TextAlign::Left
    };
    let rule_w = 96.0 * u;
    let rule = [
        if side == Side::Right {
            rect[0] + rect[2] - rule_w
        } else {
            rect[0]
        },
        rect[1],
        rule_w,
        rule_h,
    ];
    let mut y = rect[1] + rule_h + rule_gap;
    let mut lines = Vec::new();
    for (s, w) in specs.iter().zip(&widths) {
        y += s.gap;
        let h = s.size * 1.2;
        lines.push(PlacedLine {
            face: s.face,
            text: s.text.clone(),
            size: s.size,
            ls: s.ls,
            x: if side == Side::Right {
                rect[0] + rect[2] - w
            } else {
                rect[0]
            },
            y,
            w: *w,
            h,
            align,
        });
        y += h;
    }
    let bar = bar_frac.map(|f| ([rect[0], y + 14.0 * u, rect[2], 12.0 * u], f));
    Some(Placed {
        side,
        rect,
        lines,
        rule,
        bar,
    })
}

/// `₹40 lakh crore` -> (`₹40`, `lakh crore`).
fn split_value(v: &str) -> (String, Option<String>) {
    match v.split_once(' ') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (v.to_string(), None),
    }
}

/// Layout for `ev`, trying its own side first, then the other. The side used
/// is in the result.
pub fn layout_overlay(ctx: &Ctx, ev: &EditEvent) -> Option<Placed> {
    let other = if ev.side == Side::Left {
        Side::Right
    } else {
        Side::Left
    };
    layout_side(ctx, ev, ev.side).or_else(|| layout_side(ctx, ev, other))
}

// ---------------------------------------------------------------------------
// Captions
// ---------------------------------------------------------------------------

pub struct CaptionWord {
    text: String,
    x: f32,
    w: f32,
    start: f64,
    end: f64,
}

pub struct CaptionChunk {
    pub start: f64,
    pub end: f64,
    y: f32,
    size: f32,
    words: Vec<CaptionWord>,
    pub rect: Rect,
}

pub fn build_captions(ctx: &Ctx, toks: &[Tok]) -> Vec<CaptionChunk> {
    let u = ctx.u();
    let size = 50.0 * u;
    let space = ctx.width(Face::Semi, " ", size, 0.0).max(size * 0.28);
    let chunks = caption_chunks(toks);
    let mut out: Vec<CaptionChunk> = Vec::new();
    for (ci, &(a, z)) in chunks.iter().enumerate() {
        let texts: Vec<String> = (a..=z)
            .map(|i| {
                toks[i]
                    .raw
                    .trim_matches(|c: char| {
                        !(c.is_alphanumeric() || "₹$€£%'-".contains(c) || c == '.' || c == ',')
                    })
                    .to_string()
            })
            .collect();
        let ws: Vec<f32> = texts
            .iter()
            .map(|t| ctx.width(Face::Semi, t, size, 0.0))
            .collect();
        let total: f32 = ws.iter().sum::<f32>() + space * (ws.len().saturating_sub(1)) as f32;
        let x0 = (ctx.canvas.0 as f32 - total) / 2.0;
        let mut x = x0;
        let mut words = Vec::new();
        for (k, i) in (a..=z).enumerate() {
            words.push(CaptionWord {
                text: texts[k].clone(),
                x,
                w: ws[k] + 4.0,
                start: toks[i].start,
                end: toks[i].end,
            });
            x += ws[k] + space;
        }
        let start = toks[a].start;
        let next = chunks
            .get(ci + 1)
            .map(|&(n, _)| toks[n].start)
            .unwrap_or(f64::INFINITY);
        let end = (toks[z].end + 0.25).min(next).max(toks[z].end);
        let y = ctx.canvas.1 as f32 * 0.86;
        out.push(CaptionChunk {
            start,
            end,
            y,
            size,
            words,
            rect: [x0, y, total, size * 1.2],
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Scene construction
// ---------------------------------------------------------------------------

fn base(id: String, x: f32, y: f32, w: f32, h: f32, z: i32, kind: LayerKind) -> Layer {
    Layer {
        id,
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
        z_index: z,
        visible: true,
        clip: None,
        mask: None,
        depth: None,
        z: None,
        tilt: None,
        layout: None,
        kind,
    }
}

fn text_kind(f: Face, text: &str, size: f32, ls: f32, color: Color, align: TextAlign) -> LayerKind {
    LayerKind::Text(TextStyle {
        text: text.to_string(),
        font_role: if f == Face::Anton {
            FontRole::Number
        } else {
            FontRole::Body
        },
        font_size: size,
        font_weight: if f == Face::Anton { 400 } else { 600 },
        italic: false,
        color,
        align,
        line_height: 1.2,
        letter_spacing: ls,
        max_width: None,
        uppercase: false,
        ink: None,
        fill: None,
        outline: None,
    })
}

fn mo(target: &str, start: f64, duration: f64, easing: Easing, op: MotionOp) -> Motion {
    Motion {
        id: None,
        target: target.to_string(),
        start,
        duration,
        easing,
        spring: None,
        op,
    }
}

fn fade(target: &str, start: f64, duration: f64, from: f32, to: f32) -> Motion {
    mo(
        target,
        start,
        duration,
        Easing::OutCubic,
        MotionOp::Fade { from, to },
    )
}

fn scene(id: String, start: f64, duration: f64, layers: Vec<Layer>, motions: Vec<Motion>) -> Scene {
    Scene {
        id,
        start_seconds: start,
        duration_seconds: duration,
        layers,
        motions,
        camera: None,
        lifecycle: None,
        post: Vec::new(),
    }
}

fn overlay_scene(ctx: &Ctx, idx: usize, ev: &EditEvent, p: &Placed, t0: f64) -> Scene {
    let u = ctx.u();
    let start = (ev.start - t0).max(0.0);
    let dur = ev.end - t0 - start;
    let out_at = (dur - 0.25).max(0.3);
    let mut layers = Vec::new();
    let mut motions = Vec::new();
    let rid = format!("ov{idx}.rule");
    layers.push(base(
        rid.clone(),
        p.rule[0],
        p.rule[1],
        p.rule[2],
        p.rule[3],
        20,
        LayerKind::Rectangle {
            fill: ctx.accent,
            stroke: None,
        },
    ));
    let dir = if p.side == Side::Right {
        Direction::Left
    } else {
        Direction::Right
    };
    motions.push(mo(
        &rid,
        0.0,
        0.28,
        Easing::OutCubic,
        MotionOp::MaskReveal {
            direction: dir,
            mode: RevealMode::Reveal,
        },
    ));
    motions.push(fade(&rid, out_at, 0.25, 1.0, 0.0));
    for (i, l) in p.lines.iter().enumerate() {
        let delay = 0.05 + 0.07 * i as f64;
        for (tag, color, dx, z) in [("s", SHADOW, 3.0 * u, 21), ("t", WHITE, 0.0, 22)] {
            let id = format!("ov{idx}.l{i}{tag}");
            layers.push(base(
                id.clone(),
                l.x + dx,
                l.y + dx,
                l.w,
                l.h,
                z,
                text_kind(l.face, &l.text, l.size, l.ls, color, l.align),
            ));
            motions.push(fade(&id, delay, 0.2, 0.0, 1.0));
            motions.push(mo(
                &id,
                delay,
                0.35,
                Easing::OutCubic,
                MotionOp::Move {
                    from: [0.0, 22.0 * u],
                    to: [0.0, 0.0],
                    via: None,
                },
            ));
            motions.push(fade(&id, out_at, 0.25, 1.0, 0.0));
        }
    }
    if let Some((track, frac)) = p.bar {
        let (tid, fid) = (format!("ov{idx}.bt"), format!("ov{idx}.bf"));
        layers.push(base(
            tid.clone(),
            track[0],
            track[1],
            track[2],
            track[3],
            20,
            LayerKind::Rectangle {
                fill: WHITE.with_alpha(70),
                stroke: None,
            },
        ));
        let fill_w = (track[2] * frac).max(8.0);
        let fill_x = if p.side == Side::Right {
            track[0] + track[2] - fill_w
        } else {
            track[0]
        };
        layers.push(base(
            fid.clone(),
            fill_x,
            track[1],
            fill_w,
            track[3],
            21,
            LayerKind::Rectangle {
                fill: ctx.accent,
                stroke: None,
            },
        ));
        motions.push(fade(&tid, 0.15, 0.25, 0.0, 1.0));
        motions.push(fade(&tid, out_at, 0.25, 1.0, 0.0));
        motions.push(mo(
            &fid,
            0.35,
            0.7,
            Easing::OutCubic,
            MotionOp::MaskReveal {
                direction: dir,
                mode: RevealMode::Reveal,
            },
        ));
        motions.push(fade(&fid, out_at, 0.25, 1.0, 0.0));
    }
    scene(format!("ov{idx}"), start, dur, layers, motions)
}

fn caption_scene(ctx: &Ctx, idx: usize, c: &CaptionChunk, t0: f64) -> Scene {
    let u = ctx.u();
    let start = (c.start - t0).max(0.0);
    let dur = c.end - t0 - start;
    let f = 1.0 / ctx.fps as f64;
    let mut layers = Vec::new();
    let mut motions = Vec::new();
    for (i, w) in c.words.iter().enumerate() {
        let (ws, we) = ((w.start - c.start).max(0.0), (w.end - c.start).max(0.0));
        let h = c.size * 1.2;
        let mk = |tag: &str, color: Color, dx: f32, z: i32, layers: &mut Vec<Layer>| {
            let id = format!("cap{idx}.w{i}{tag}");
            layers.push(base(
                id.clone(),
                w.x + dx,
                c.y + dx,
                w.w,
                h,
                z,
                text_kind(Face::Semi, &w.text, c.size, 0.0, color, TextAlign::Left),
            ));
            id
        };
        let sh = mk("s", SHADOW, 3.0 * u, 30, &mut layers);
        let dim = mk("d", WHITE.with_alpha(150), 0.0, 31, &mut layers);
        let bright = mk("b", WHITE, 0.0, 32, &mut layers);
        let act = mk("a", ctx.accent, 0.0, 33, &mut layers);
        let _ = (&sh, &dim);
        motions.push(mo(
            &bright,
            ws,
            f,
            Easing::Linear,
            MotionOp::Fade { from: 0.0, to: 1.0 },
        ));
        motions.push(mo(
            &act,
            ws,
            f,
            Easing::Linear,
            MotionOp::Fade { from: 0.0, to: 1.0 },
        ));
        let out_at = we.max(ws + f);
        if out_at + f <= dur - 1e-6 {
            motions.push(mo(
                &act,
                out_at,
                f,
                Easing::Linear,
                MotionOp::Fade { from: 1.0, to: 0.0 },
            ));
        }
    }
    scene(format!("cap{idx}"), start, dur, layers, motions)
}

/// All overlay/caption intervals as `(start, end)` (segment cuts avoid them).
fn busy(placed: &[(usize, Placed)], plan: &EditPlan, caps: &[CaptionChunk]) -> Vec<(f64, f64)> {
    placed
        .iter()
        .map(|(i, _)| (plan.events[*i].start, plan.events[*i].end))
        .chain(
            plan.events
                .iter()
                .filter(|e| matches!(e.kind, EventKind::Cutaway { .. } | EventKind::PunchIn {}))
                .map(|e| (e.start, e.end)),
        )
        .chain(caps.iter().map(|c| (c.start, c.end)))
        .collect()
}

/// Segment frame ranges of about `seg_s` seconds whose cuts fall where no
/// overlay or caption is on screen (so no animation is split).
pub fn segments(total: u32, fps: u32, seg_s: f64, busy: &[(f64, f64)]) -> Vec<(u32, u32)> {
    let seg = ((seg_s * fps as f64) as u32).max(fps);
    let clear = |f: u32| {
        let t = f as f64 / fps as f64;
        !busy.iter().any(|&(a, b)| a < t - 1e-9 && t + 1e-9 < b)
    };
    let mut out = Vec::new();
    let mut f0 = 0;
    while f0 < total {
        let mut f1 = f0 + seg;
        if f1 >= total || total - f1 < fps {
            f1 = total;
        } else {
            let limit = (f1 + 20 * fps).min(total);
            let mut c = f1;
            while c < limit && !clear(c) {
                c += 1;
            }
            if c < limit {
                f1 = c;
            }
        }
        out.push((f0, f1));
        f0 = f1;
    }
    out
}

pub struct RenderJob<'a> {
    pub ctx: &'a Ctx<'a>,
    pub plan: &'a EditPlan,
    pub video: &'a Path,
    pub video_frames: u32,
    pub video_fps: f64,
    pub total_frames: u32,
    pub seg_seconds: f64,
    pub brand: Option<(PathBuf, motion_render::brand_mark::Corner)>,
    pub assets_dir: &'a Path,
    pub tmp: &'a Path,
}

/// A pre-rendered cutaway clip for plan event `event`.
#[derive(Debug, Clone)]
pub struct CutClip {
    pub event: usize,
    pub path: PathBuf,
    pub frames: u32,
}

pub struct Placement {
    pub overlays: Vec<(usize, Placed)>,
    pub captions: Vec<CaptionChunk>,
    pub cutaways: Vec<CutClip>,
}

/// Lay out every overlay event (dropping those that cannot be placed beside
/// the face, with the plan updated to the side used) and the captions.
pub fn lay_out(ctx: &Ctx, plan: &mut EditPlan, toks: &[Tok]) -> (Placement, Vec<String>) {
    let mut overlays = Vec::new();
    let mut dropped = Vec::new();
    let mut keep = Vec::new();
    let events = std::mem::take(&mut plan.events);
    for ev in events {
        if matches!(
            ev.kind,
            EventKind::Captions {} | EventKind::Cutaway { .. } | EventKind::PunchIn {}
        ) {
            keep.push(ev);
            continue;
        }
        let mut ev = ev;
        match layout_overlay(ctx, &ev) {
            Some(p) => {
                ev.side = p.side;
                overlays.push((keep.len(), p));
                keep.push(ev);
            }
            None => dropped.push(format!("{} at {:.1}s", ev.kind.name(), ev.start)),
        }
    }
    plan.events = keep;
    let captions = if plan
        .events
        .iter()
        .any(|e| matches!(e.kind, EventKind::Captions {}))
    {
        build_captions(ctx, toks)
    } else {
        Vec::new()
    };
    (
        Placement {
            overlays,
            captions,
            cutaways: Vec::new(),
        },
        dropped,
    )
}

/// `overlay_on_face` QA: every placed overlay rect, caption line and the brand
/// mark box against the face box over its time span. Returns the failures.
pub fn overlay_on_face(
    ctx: &Ctx,
    plan: &EditPlan,
    pl: &Placement,
    brand: Option<Rect>,
) -> Vec<String> {
    use motion_core::edit::intersects;
    let mut bad = Vec::new();
    for (i, p) in &pl.overlays {
        let ev = &plan.events[*i];
        let face = face_union(ctx.face, ctx.cv(), ev.start, ev.end);
        if intersects(p.rect, face) {
            bad.push(format!("{} at {:.1}s", ev.kind.name(), ev.start));
        }
    }
    for c in &pl.captions {
        let face = face_union(ctx.face, ctx.cv(), c.start, c.end);
        if intersects(c.rect, face) {
            bad.push(format!("caption at {:.1}s", c.start));
            break;
        }
    }
    if let Some(b) = brand {
        for f in ctx.face {
            if intersects(b, f.rect) {
                bad.push(format!("brand mark at {:.1}s", f.start));
                break;
            }
        }
    }
    bad
}

/// Seconds the overlays (excluding captions) cover, as a union.
pub fn overlay_screen_time(plan: &EditPlan) -> f64 {
    let mut iv: Vec<(f64, f64)> = plan
        .events
        .iter()
        .filter(|e| !matches!(e.kind, EventKind::Captions {}))
        .map(|e| (e.start, e.end))
        .collect();
    iv.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let (mut total, mut cur): (f64, Option<(f64, f64)>) = (0.0, None);
    for (a, b) in iv {
        cur = match cur {
            Some((s, e)) if a <= e => Some((s, e.max(b))),
            Some((s, e)) => {
                total += e - s;
                Some((a, b))
            }
            None => Some((a, b)),
        };
    }
    total + cur.map(|(s, e)| e - s).unwrap_or(0.0)
}

fn theme() -> Theme {
    let mut fonts = BTreeMap::new();
    for r in FontRole::ALL {
        fonts.insert(r, "font.fira_sans_semibold".to_string());
    }
    fonts.insert(FontRole::Number, "font.anton".to_string());
    Theme {
        fonts,
        palette: BTreeMap::new(),
        typography: None,
    }
}

/// Scale of a punch-in.
pub const PUNCH_SCALE: f32 = 1.12;

/// True when a cutaway is on screen at any time in `[a, b]` (the cutaway
/// carries its own captions).
fn under_cutaway(plan: &EditPlan, pl: &Placement, a: f64, b: f64) -> bool {
    pl.cutaways.iter().any(|c| {
        let e = &plan.events[c.event];
        e.start < b && e.end > a
    })
}

/// The project for frames `[f0, f1)`: the video plate plus the scenes inside.
pub fn segment_project(job: &RenderJob, pl: &Placement, f0: u32, f1: u32) -> MotionProject {
    let ctx = job.ctx;
    let fps = ctx.fps as f64;
    let (t0, t1) = (f0 as f64 / fps, f1 as f64 / fps);
    let mut assets = vec![
        Asset {
            id: "font.anton".into(),
            kind: AssetKind::Font,
            path: "fonts/Anton-Regular.ttf".into(),
            sprite: None,
        },
        Asset {
            id: "font.fira_sans_semibold".into(),
            kind: AssetKind::Font,
            path: "fonts/FiraSans-SemiBold.ttf".into(),
            sprite: None,
        },
    ];
    assets.push(Asset {
        id: "video.src".into(),
        kind: AssetKind::Video,
        path: job.video.to_string_lossy().into_owned(),
        sprite: Some(SpriteSpec {
            frame_count: job.video_frames,
            fps: job.video_fps,
            mode: SpriteMode::Once,
            pattern: "frame_%04d.png".into(),
        }),
    });
    // Punch-ins inside the segment, scaled about the face centre.
    let punches: Vec<&EditEvent> = job
        .plan
        .events
        .iter()
        .filter(|e| matches!(e.kind, EventKind::PunchIn {}) && e.end > t0 && e.start < t1)
        .collect();
    let mut video = base(
        "src.video".into(),
        0.0,
        0.0,
        ctx.canvas.0 as f32,
        ctx.canvas.1 as f32,
        0,
        LayerKind::Image {
            asset: "video.src".into(),
            fit: Fit::Cover,
            treatment: None,
            playback: Some(SpritePlayback {
                start: 0.0,
                in_frame: f0,
                out_frame: Some(f1.saturating_sub(1)),
            }),
            insert: None,
        },
    );
    let mut vmotions = Vec::new();
    if !punches.is_empty() {
        let cv = ctx.cv();
        let face = face_union(ctx.face, cv, t0, t1);
        let (ax, ay) = (
            ((face[0] + face[2] / 2.0) / cv.0).clamp(0.2, 0.8),
            ((face[1] + face[3] / 2.0) / cv.1).clamp(0.2, 0.8),
        );
        video.anchor_x = ax;
        video.anchor_y = ay;
        video.x = ax * cv.0;
        video.y = ay * cv.1;
        for e in &punches {
            let (s, d) = ((e.start - t0).max(0.0), e.end - e.start);
            vmotions.push(mo(
                "src.video",
                s,
                0.4,
                Easing::InOutCubic,
                MotionOp::Scale {
                    from: 1.0,
                    to: PUNCH_SCALE,
                    axis: Default::default(),
                },
            ));
            vmotions.push(mo(
                "src.video",
                s + d - 0.5,
                0.5,
                Easing::InOutCubic,
                MotionOp::Scale {
                    from: PUNCH_SCALE,
                    to: 1.0,
                    axis: Default::default(),
                },
            ));
        }
    }
    let mut scenes = vec![scene("video".into(), 0.0, t1 - t0, vec![video], vmotions)];
    for cc in &pl.cutaways {
        let ev = &job.plan.events[cc.event];
        if ev.end <= t0 + 1e-6 || ev.start >= t1 - 1e-6 {
            continue;
        }
        let aid = format!("cut{}.clip", cc.event);
        assets.push(Asset {
            id: aid.clone(),
            kind: AssetKind::Video,
            path: cc.path.to_string_lossy().into_owned(),
            sprite: Some(SpriteSpec {
                frame_count: cc.frames,
                fps: ctx.fps as f64,
                mode: SpriteMode::Once,
                pattern: "frame_%04d.png".into(),
            }),
        });
        let skip = ((t0 - ev.start).max(0.0) * fps).round() as u32;
        let layer = base(
            format!("cut{}.v", cc.event),
            0.0,
            0.0,
            ctx.canvas.0 as f32,
            ctx.canvas.1 as f32,
            0,
            LayerKind::Image {
                asset: aid,
                fit: Fit::Cover,
                treatment: None,
                playback: Some(SpritePlayback {
                    start: 0.0,
                    in_frame: skip.min(cc.frames.saturating_sub(1)),
                    out_frame: Some(cc.frames.saturating_sub(1)),
                }),
                insert: None,
            },
        );
        scenes.push(scene(
            format!("cut{}", cc.event),
            (ev.start - t0).max(0.0),
            ev.end.min(t1) - ev.start.max(t0),
            vec![layer],
            Vec::new(),
        ));
    }
    for (i, p) in &pl.overlays {
        let ev = &job.plan.events[*i];
        if ev.end > t0 + 1e-6 && ev.start < t1 - 1e-6 {
            let mut e = ev.clone();
            e.end = e.end.min(t1);
            scenes.push(overlay_scene(ctx, *i, &e, p, t0));
        }
    }
    for (i, c) in pl.captions.iter().enumerate() {
        if c.end > t0 + 1e-6 && c.start < t1 - 1e-6 && !under_cutaway(job.plan, pl, c.start, c.end)
        {
            scenes.push(caption_scene(ctx, i, c, t0));
        }
    }
    MotionProject {
        version: "0.2".into(),
        project: ProjectMeta {
            name: "edit".into(),
            duration_seconds: Some(t1 - t0),
            exploration: None,
            speech: None,
            art: None,
            direction: None,
        },
        canvas: Canvas {
            width: ctx.canvas.0,
            height: ctx.canvas.1,
            fps: ctx.fps,
            background: Color::rgb(0, 0, 0),
        },
        theme: theme(),
        assets,
        asset_root: Some(job.assets_dir.to_string_lossy().into_owned()),
        scenes,
        shared: Vec::new(),
        envelopes: Vec::new(),
    }
}

/// Render in segments and concatenate without re-encoding. Returns segment count.
pub fn render_all(
    job: &RenderJob,
    pl: &Placement,
    source_audio: &Path,
    out: &Path,
) -> Result<usize> {
    let ctx = job.ctx;
    let b = busy(&pl.overlays, job.plan, &pl.captions);
    let segs = segments(job.total_frames, ctx.fps, job.seg_seconds, &b);
    let mut files = Vec::new();
    for (k, &(f0, f1)) in segs.iter().enumerate() {
        let project = segment_project(job, pl, f0, f1);
        motion_core::validate(&project, Some(job.assets_dir))
            .map_err(|e| anyhow::anyhow!("segment {k} invalid: {e}"))?;
        let path = job.tmp.join(format!("seg_{k:03}.mp4"));
        render_to_mp4_stream_quality(
            &project,
            job.assets_dir,
            &path,
            RenderOptions {
                gpu_post: false,
                brand_mark: job.brand.clone(),
            },
            Quality::Delivery,
        )
        .with_context(|| format!("render segment {k} ({f0}..{f1})"))?;
        eprintln!("segment {}/{} frames {f0}..{f1}", k + 1, segs.len());
        files.push(path);
    }
    concat(
        &files,
        source_audio,
        job.total_frames as f64 / ctx.fps as f64,
        job.tmp,
        out,
    )?;
    Ok(segs.len())
}

/// Concat demuxer, video stream copied; the source audio muxed untouched.
pub fn concat(
    files: &[PathBuf],
    audio_src: &Path,
    duration: f64,
    tmp: &Path,
    out: &Path,
) -> Result<()> {
    let list = tmp.join("concat.txt");
    let mut text = String::new();
    for f in files {
        text.push_str(&format!("file '{}'\n", f.display()));
    }
    std::fs::write(&list, text)?;
    if let Some(p) = out.parent() {
        std::fs::create_dir_all(p)?;
    }
    let mut cmd = std::process::Command::new("ffmpeg");
    cmd.args([
        "-y", "-v", "error", "-nostdin", "-f", "concat", "-safe", "0", "-i",
    ])
    .arg(&list);
    cmd.arg("-i").arg(audio_src);
    cmd.args([
        "-map",
        "0:v:0",
        "-map",
        "1:a:0?",
        "-c",
        "copy",
        "-t",
        &format!("{duration:.3}"),
        "-movflags",
        "+faststart",
    ])
    .arg(out);
    let st = cmd.status().context("cannot run ffmpeg")?;
    if !st.success() {
        let _ = std::fs::remove_file(out);
        bail!("ffmpeg concat failed ({st})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::edit::{EditEvent, EDIT_PLAN_VERSION};

    fn plan(canvas: [u32; 2], face: [f32; 4], events: Vec<EditEvent>) -> EditPlan {
        EditPlan {
            version: EDIT_PLAN_VERSION.into(),
            source: "t.mp4".into(),
            canvas,
            fps: 30,
            duration: 10.0,
            language: "en".into(),
            words: 20,
            face_boxes: vec![FaceSeg {
                start: 0.0,
                end: 10.0,
                rect: face,
                detected: true,
            }],
            events,
        }
    }

    fn ev(kind: EventKind, start: f64, end: f64, side: Side) -> EditEvent {
        EditEvent {
            kind,
            at_word: 0,
            until_word: 0,
            start,
            end,
            side,
        }
    }

    fn assets() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }

    fn ctx_for<'a>(
        m: &'a FontMeasure,
        f: &'a Fonts,
        face: &'a [FaceSeg],
        canvas: (u32, u32),
    ) -> Ctx<'a> {
        Ctx {
            canvas,
            fps: 30,
            measure: m,
            fonts: f,
            accent: Color::rgb(0xBD, 0x27, 0x26),
            face,
        }
    }

    #[test]
    fn cuts_avoid_busy_spans() {
        let busy = [(58.0, 63.0)];
        let segs = segments(30 * 130, 30, 60.0, &busy);
        assert_eq!(segs.first().unwrap().0, 0);
        assert_eq!(segs.last().unwrap().1, 30 * 130);
        assert!(segs.windows(2).all(|w| w[0].1 == w[1].0));
        let cut = segs[0].1 as f64 / 30.0;
        assert!(
            !(busy[0].0 < cut && cut < busy[0].1),
            "cut inside overlay at {cut}"
        );
    }

    #[test]
    fn overlays_avoid_the_face_box_or_are_dropped() {
        let fonts = Fonts::in_assets(&assets());
        let m = FontMeasure::with_fonts(fonts.all()).unwrap();
        let face = [700.0, 70.0, 520.0, 640.0];
        let events = vec![
            ev(
                EventKind::NumberCallout {
                    value: "\u{20B9}40 lakh crore".into(),
                },
                1.0,
                4.0,
                Side::Right,
            ),
            ev(
                EventKind::Keyword {
                    text: "PRESSURE".into(),
                },
                5.0,
                6.8,
                Side::Left,
            ),
            ev(
                EventKind::ChapterCard {
                    title: "TRADE DEFICIT".into(),
                    index: 1,
                },
                7.0,
                9.6,
                Side::Left,
            ),
            ev(
                EventKind::LowerThird {
                    text: "Name".into(),
                },
                0.9,
                4.9,
                Side::Left,
            ),
        ];
        let mut p = plan([1920, 1080], face, events);
        let faces = p.face_boxes.clone();
        let c = ctx_for(&m, &fonts, &faces, (1920, 1080));
        let (pl, dropped) = lay_out(&c, &mut p, &[]);
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(pl.overlays.len(), 4);
        assert!(overlay_on_face(&c, &p, &pl, None).is_empty());
        // A face that fills the frame leaves no room: events are dropped, never placed on it.
        let wide = [200.0, 70.0, 1500.0, 640.0];
        let mut p2 = plan(
            [1920, 1080],
            wide,
            vec![ev(
                EventKind::Keyword { text: "X".into() },
                1.0,
                2.0,
                Side::Left,
            )],
        );
        let faces2 = p2.face_boxes.clone();
        let c2 = ctx_for(&m, &fonts, &faces2, (1920, 1080));
        let (pl2, dropped2) = lay_out(&c2, &mut p2, &[]);
        assert_eq!(dropped2.len(), 1);
        assert!(pl2.overlays.is_empty() && overlay_on_face(&c2, &p2, &pl2, None).is_empty());
    }

    fn frames_of(path: &Path) -> u32 {
        let out = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=nb_read_frames",
                "-of",
                "csv=p=0",
            ])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
    }

    use std::process::Command;

    #[test]
    fn segmented_render_concats_without_losing_frames() {
        let dir = std::env::temp_dir().join(format!("me_edit_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let clip = dir.join("clip.mp4");
        let st = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=640x360:rate=30:duration=10",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=10",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&clip)
            .status()
            .unwrap();
        assert!(st.success());
        let fonts = Fonts::in_assets(&assets());
        let m = FontMeasure::with_fonts(fonts.all()).unwrap();
        let face = [213.0, 20.0, 214.0, 220.0];
        let mut p = plan(
            [640, 360],
            face,
            vec![
                ev(
                    EventKind::Keyword {
                        text: "HELLO".into(),
                    },
                    1.0,
                    2.8,
                    Side::Left,
                ),
                ev(
                    EventKind::Keyword {
                        text: "WORLD".into(),
                    },
                    6.0,
                    7.8,
                    Side::Right,
                ),
            ],
        );
        let faces = p.face_boxes.clone();
        let c = ctx_for(&m, &fonts, &faces, (640, 360));
        let toks: Vec<Tok> = "one two three four five six"
            .split(' ')
            .enumerate()
            .map(|(i, w)| Tok::new(w, 0.5 + i as f64 * 0.5, 0.9 + i as f64 * 0.5))
            .collect();
        p.events
            .push(ev(EventKind::Captions {}, 0.0, 10.0, Side::Left));
        let (pl, dropped) = lay_out(&c, &mut p, &toks);
        assert!(dropped.is_empty(), "{dropped:?}");
        let assets_abs = assets().canonicalize().unwrap();
        let tmp = dir.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let job = RenderJob {
            ctx: &c,
            plan: &p,
            video: &clip,
            video_frames: 300,
            video_fps: 30.0,
            total_frames: 300,
            seg_seconds: 4.0,
            brand: None,
            assets_dir: &assets_abs,
            tmp: &tmp,
        };
        let out = dir.join("out.mp4");
        let n = render_all(&job, &pl, &clip, &out).unwrap();
        assert!(n >= 2, "segments {n}");
        assert_eq!(frames_of(&out), 300);
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_type",
                "-of",
                "csv=p=0",
            ])
            .arg(&out)
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&probe.stdout).contains("audio"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cutaway_clip_replaces_the_picture_for_its_span() {
        let dir = std::env::temp_dir().join(format!("me_edit_cut_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, src: &str, secs: u32| {
            let p = dir.join(name);
            let st = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-f", "lavfi", "-i"])
                .arg(format!("{src}:size=640x360:rate=30:duration={secs}"))
                .args(["-f", "lavfi", "-i"])
                .arg(format!("sine=frequency=440:duration={secs}"))
                .args([
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                ])
                .arg(&p)
                .status()
                .unwrap();
            assert!(st.success());
            p
        };
        let clip = mk("clip.mp4", "color=c=red", 10);
        let cut = mk("cut.mp4", "color=c=0x00ff00", 3);
        let beat: motion_core::intent::Beat = serde_json::from_str(
            r#"{"purpose":"emphasize","statement":"x","primary":{"kind":"phrase","value":"X"}}"#,
        )
        .unwrap();
        let events = vec![
            ev(
                EventKind::Cutaway {
                    sub: motion_core::edit::CutawayKind::Concept,
                    beats: vec![motion_core::edit::CutBeat {
                        first: 0,
                        last: 0,
                        beat,
                    }],
                },
                3.0,
                6.0,
                Side::Left,
            ),
            ev(EventKind::PunchIn {}, 7.0, 9.0, Side::Left),
        ];
        let mut p = plan([640, 360], [213.0, 20.0, 214.0, 220.0], events);
        let fonts = Fonts::in_assets(&assets());
        let m = FontMeasure::with_fonts(fonts.all()).unwrap();
        let faces = p.face_boxes.clone();
        let c = ctx_for(&m, &fonts, &faces, (640, 360));
        let (mut pl, dropped) = lay_out(&c, &mut p, &[]);
        assert!(dropped.is_empty());
        pl.cutaways.push(CutClip {
            event: 0,
            path: cut.clone(),
            frames: 90,
        });
        let assets_abs = assets().canonicalize().unwrap();
        let tmp = dir.join("tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        let job = RenderJob {
            ctx: &c,
            plan: &p,
            video: &clip,
            video_frames: 300,
            video_fps: 30.0,
            total_frames: 300,
            seg_seconds: 60.0,
            brand: None,
            assets_dir: &assets_abs,
            tmp: &tmp,
        };
        let out = dir.join("out.mp4");
        render_all(&job, &pl, &clip, &out).unwrap();
        assert_eq!(frames_of(&out), 300);
        // Mean colour of the frame at `t`: red source, green cutaway.
        let px = |t: f64| -> (u8, u8, u8) {
            let o = Command::new("ffmpeg")
                .args(["-v", "error", "-ss"])
                .arg(format!("{t}"))
                .arg("-i")
                .arg(&out)
                .args([
                    "-frames:v",
                    "1",
                    "-vf",
                    "scale=1:1",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgb24",
                    "-",
                ])
                .output()
                .unwrap();
            (o.stdout[0], o.stdout[1], o.stdout[2])
        };
        let before = px(1.0);
        let during = px(4.5);
        let after = px(6.5);
        assert!(before.0 > 150 && before.1 < 100, "source before {before:?}");
        assert!(
            during.1 > 150 && during.0 < 100,
            "cutaway during {during:?}"
        );
        assert!(after.0 > 150 && after.1 < 100, "source after {after:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
