//! Type over anything (`DESIGN_COLOR_SYSTEM.md` §3), as lowering passes.
//! Textures are never cut, cleared or faded around elements (owner,
//! 2026-10-13): legibility comes from the palette (texture ≤ 1.35:1, ink
//! ≥ 7:1) and from composition.
//!
//! 1. [`calm_focus`]: a halftone puts its dense end where concurrent type
//!    is not, once per shot (a calm-zone search over the slot grid); the
//!    pattern itself stays whole.
//! 2. [`enforce_roles`]: every run of type takes the role that reads on the
//!    colour actually behind it (≥ 7:1 small, ≥ 4.5:1 display); strokes
//!    are opaque and at least 3:1.

use motion_core::scene::{Color, Fill, Layer, LayerKind, MotionProject};

use crate::lower::Colours;
use crate::program::{ComponentBody, FieldPreset, MotionProgram, Slot};

/// A run of type in canvas space while its scene is live.
#[derive(Debug, Clone)]
pub struct TextRun {
    pub t0: f64,
    pub t1: f64,
    pub z: i32,
    /// Ink rect `[x0, y0, x1, y1]`.
    pub rect: [f32; 4],
    /// Font size in px (scaled).
    pub em: f32,
}

/// Canvas-space box `[x0, y0, x1, y1]` of a layer inside `origin` (the
/// parent box's top-left), scale only.
fn abs_box(l: &Layer, origin: [f32; 2], k: f32) -> [f32; 4] {
    let (w, h) = (l.width * l.scale_x * k, l.height * l.scale_y * k);
    let x0 = origin[0] + l.x * k - l.anchor_x * w;
    let y0 = origin[1] + l.y * k - l.anchor_y * h;
    [x0, y0, x0 + w, y0 + h]
}

fn collect_runs(
    layers: &[Layer],
    origin: [f32; 2],
    k: f32,
    span: (f64, f64),
    z: Option<i32>,
    out: &mut Vec<TextRun>,
) {
    for l in layers {
        let b = abs_box(l, origin, k);
        match &l.kind {
            LayerKind::Text(st) => {
                let sy = l.scale_y * k;
                let (top, bottom) = st.ink.map_or((0.0, l.height), |i| (i.top, i.bottom));
                out.push(TextRun {
                    t0: span.0,
                    t1: span.1,
                    z: z.unwrap_or(l.z_index),
                    rect: [b[0], b[1] + top * sy, b[2], b[1] + bottom * sy],
                    em: st.font_size * sy,
                });
            }
            LayerKind::Group { children } => {
                collect_runs(
                    children,
                    [b[0], b[1]],
                    k * l.scale_x,
                    span,
                    Some(z.unwrap_or(l.z_index)),
                    out,
                );
            }
            _ => {}
        }
    }
}

/// Every text run of the project (top level and inside groups).
pub fn text_runs(project: &MotionProject) -> Vec<TextRun> {
    let mut out = Vec::new();
    for s in &project.scenes {
        collect_runs(
            &s.layers,
            [0.0, 0.0],
            1.0,
            (s.start_seconds, s.end_seconds()),
            None,
            &mut out,
        );
    }
    out
}

/// Mean halftone coverage (dot area per cell) over a rect for a focus at
/// `c` (the field's own size law, sampled on a 5×3 grid).
fn coverage(r: [f32; 4], c: [f32; 2], w: f32, h: f32) -> f32 {
    let reach = [[0.0, 0.0], [w, 0.0], [0.0, h], [w, h]]
        .iter()
        .map(|q: &[f32; 2]| (q[0] - c[0]).hypot(q[1] - c[1]))
        .fold(0.0f32, f32::max);
    let mut sum = 0.0;
    for i in 0..5 {
        for j in 0..3 {
            let x = r[0] + (r[2] - r[0]) * (i as f32 + 0.5) / 5.0;
            let y = r[1] + (r[3] - r[1]) * (j as f32 + 0.5) / 3.0;
            let d = (x - c[0]).hypot(y - c[1]) / reach;
            let s = 0.42 * (1.0 - d).max(0.0).powf(1.3);
            sum += s * s;
        }
    }
    sum / 15.0
}

/// Calm-zone search: every halftone field moves its dense end (its focus)
/// to the slot where the concurrent type sits on the least texture. The
/// asked focus wins unless another is clearly calmer.
pub fn calm_focus(p: &MotionProgram, project: &mut MotionProject) {
    let (w, h) = (project.canvas.width as f32, project.canvas.height as f32);
    let short = w.min(h);
    let runs = text_runs(project);
    for c in &p.components {
        let ComponentBody::Field {
            preset: FieldPreset::Halftone,
            focus,
            ..
        } = &c.body
        else {
            continue;
        };
        let (s0, s1) = (c.start, p.end_of(c));
        let near: Vec<&TextRun> = runs.iter().filter(|r| r.t0 < s1 && s0 < r.t1).collect();
        if near.is_empty() {
            continue;
        }
        let score = |f: [f32; 2]| -> f32 {
            near.iter()
                .map(|r| {
                    let area = (r.rect[2] - r.rect[0]) * (r.rect[3] - r.rect[1]);
                    coverage(r.rect, f, w, h) * area.sqrt()
                })
                .sum()
        };
        let asked = crate::taste::slot_center(focus.unwrap_or_default(), p.meta.aspect);
        let mut best = (score(asked), asked);
        let corners = [
            [w * 0.15, h * 0.15],
            [w * 0.85, h * 0.15],
            [w * 0.15, h * 0.85],
            [w * 0.85, h * 0.85],
        ];
        let slots = [
            Slot::Center,
            Slot::Lead,
            Slot::Trail,
            Slot::Top,
            Slot::Bottom,
        ]
        .map(|s| crate::taste::slot_center(s, p.meta.aspect));
        let eyes = crate::taste::golden_eyes(w, h);
        for f in slots.iter().chain(corners.iter()).chain(eyes.iter()) {
            let sc = score(*f);
            if sc < best.0 * 0.7 {
                best = (sc, *f);
            }
        }
        if best.1 == asked {
            continue;
        }
        let at = crate::field::instances(FieldPreset::Halftone, w, h, short, best.1);
        for s in &mut project.scenes {
            if s.id != c.id {
                continue;
            }
            for l in &mut s.layers {
                if let LayerKind::Instances { at: a, .. } = &mut l.kind {
                    *a = at.clone();
                }
            }
        }
    }
}

fn canvas_content_transform(
    l: &motion_core::timeline::ResolvedLayer<'_>,
) -> motion_core::timeline::Affine {
    if l.projective.is_some() {
        l.transform.then_apply(l.content_transform)
    } else {
        l.content_transform
    }
}

/// The colour drawn under `p` before layer `stop` (draw order) on a
/// resolved frame: the last filled layer at ≥ 50 % effective opacity whose
/// box holds `p`; the canvas background if none.
fn backing_resolved(
    rf: &motion_core::timeline::ResolvedFrame<'_>,
    stop: *const motion_core::timeline::ResolvedLayer<'_>,
    p: [f32; 2],
) -> Color {
    use motion_core::timeline::ResolvedLayer;
    fn walk(
        l: &ResolvedLayer<'_>,
        alpha: f32,
        stop: *const ResolvedLayer<'_>,
        p: [f32; 2],
        best: &mut Color,
    ) -> bool {
        if std::ptr::eq(l, stop) {
            return true;
        }
        let a = alpha * l.opacity;
        let fill = match l.kind {
            LayerKind::Rectangle { fill, .. } | LayerKind::RoundedRectangle { fill, .. } => {
                Some(*fill)
            }
            LayerKind::Path {
                fill: Some(Fill::Solid(c)),
                ..
            } => Some(*c),
            LayerKind::Path {
                fill: Some(Fill::Gradient(g)),
                ..
            } => g.stops().first().map(|s| s.color),
            _ => None,
        };
        if let Some(c) = fill.filter(|c| c.a > 200 && a >= 0.5) {
            let (x0, y0) = l.transform.apply(0.0, 0.0);
            let (x1, y1) = l.transform.apply(l.width, l.height);
            let (ax, bx) = (x0.min(x1), x0.max(x1));
            let (ay, by) = (y0.min(y1), y0.max(y1));
            let in_box = p[0] >= ax && p[0] <= bx && p[1] >= ay && p[1] <= by;
            // Paths count only inside their own outline (one path may hold
            // every bar of a chart).
            let ct = canvas_content_transform(l);
            let inside = in_box
                && match l.kind {
                    LayerKind::Path { d, .. } => {
                        let geo = l.path.clone().or_else(|| motion_core::path::parse(d).ok());
                        geo.is_none_or(|g| path_contains(&g, &ct, p))
                    }
                    _ => true,
                };
            if inside {
                *best = match &l.tint {
                    Some(t) => {
                        let m = |u: u8, v: u8| {
                            (u as f32 + (v as f32 - u as f32) * t.amount).round() as u8
                        };
                        Color::rgb(m(c.r, t.color.r), m(c.g, t.color.g), m(c.b, t.color.b))
                    }
                    None => Color { a: 255, ..c },
                };
            }
        }
        for c in &l.children {
            if walk(c, a, stop, p, best) {
                return true;
            }
        }
        false
    }
    let mut best = rf.background;
    for l in &rf.layers {
        if walk(l, 1.0, stop, p, &mut best) {
            break;
        }
    }
    best
}

/// Nonzero winding of `p` (canvas) in `g` (content space under `t`).
fn path_contains(
    g: &motion_core::path::BezPath,
    t: &motion_core::timeline::Affine,
    p: [f32; 2],
) -> bool {
    let det = t.a * t.d - t.b * t.c;
    if det.abs() < 1e-9 {
        return false;
    }
    let (x, y) = (p[0] - t.e, p[1] - t.f);
    let q = [(t.d * x - t.c * y) / det, (-t.b * x + t.a * y) / det];
    let mut wind = 0i32;
    for c in &g.contours {
        let mut pts = vec![c.start];
        let mut p0 = c.start;
        for seg in &c.segs {
            for k in 1..=8 {
                pts.push(seg.at(p0, k as f32 / 8.0));
            }
            p0 = seg.to;
        }
        pts.push(c.start);
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a[1] <= q[1] {
                if b[1] > q[1]
                    && (b[0] - a[0]) * (q[1] - a[1]) - (q[0] - a[0]) * (b[1] - a[1]) > 0.0
                {
                    wind += 1;
                }
            } else if b[1] <= q[1]
                && (b[0] - a[0]) * (q[1] - a[1]) - (q[0] - a[0]) * (b[1] - a[1]) < 0.0
            {
                wind -= 1;
            }
        }
    }
    wind != 0
}

/// Role enforcement on the lowered scene: each run of type takes the role
/// that reads on the colour actually behind it, judged on the first frame
/// it is fully shown (device states and flips change what is behind);
/// stroke-only paths are opaque and at least 3:1 (a line that reads).
pub(crate) fn enforce_roles(project: &mut MotionProject, col: &Colours) {
    use motion_core::timeline::ResolvedLayer;
    // Every colour each run of type (and each stroke) is shown on, at
    // ≥ 90 % visibility, sampled every other frame.
    let n = project.frame_count();
    type Key = (String, String);
    let mut text: std::collections::BTreeMap<Key, (Vec<(Color, u32)>, f32)> = Default::default();
    let mut strokes: std::collections::BTreeMap<Key, Vec<(Color, u32)>> = Default::default();
    let bump = |v: &mut Vec<(Color, u32)>, c: Color| match v.iter_mut().find(|e| e.0 == c) {
        Some(e) => e.1 += 1,
        None => v.push((c, 1)),
    };
    let mut fr = 0;
    while fr < n {
        if let Ok(rf) = motion_core::evaluate_frame(project, fr) {
            let mut stack: Vec<(&ResolvedLayer<'_>, f32)> =
                rf.layers.iter().rev().map(|l| (l, 1.0)).collect();
            while let Some((l, alpha)) = stack.pop() {
                let a = alpha * l.opacity;
                for c in l.children.iter().rev() {
                    stack.push((c, a));
                }
                // Type under a glyph cascade shows as much as its glyphs do.
                let a = match &l.glyphs {
                    Some(g) if !g.is_empty() => {
                        a * g.iter().map(|p| p.opacity).sum::<f32>() / g.len() as f32
                    }
                    _ => a,
                };
                if a < 0.9 {
                    continue;
                }
                let key = (l.scene.unwrap_or("").to_string(), l.id.to_string());
                match l.kind {
                    LayerKind::Text(st) => {
                        let (top, bottom) = st.ink.map_or((0.0, l.height), |i| (i.top, i.bottom));
                        let (x, y) = l.transform.apply(l.width / 2.0, (top + bottom) / 2.0);
                        let (_, y0) = l.transform.apply(0.0, top);
                        let (_, y1) = l.transform.apply(0.0, bottom);
                        let px = st.font_size * ((y1 - y0).abs() / (bottom - top).max(1.0));
                        let back = backing_resolved(&rf, l, [x, y]);
                        let e = text.entry(key).or_insert((Vec::new(), px));
                        e.1 = e.1.min(px);
                        bump(&mut e.0, back);
                    }
                    LayerKind::Path {
                        fill: None,
                        stroke: Some(_),
                        ..
                    } => {
                        let (x, y) = l.transform.apply(l.width / 2.0, l.height / 2.0);
                        let back = backing_resolved(&rf, l, [x, y]);
                        bump(strokes.entry(key).or_default(), back);
                    }
                    _ => {}
                }
            }
        }
        fr += 2;
    }
    // Backings that last: a colour seen for a sample or two at a cut is a
    // transient, not a ground.
    let lasting = |v: &[(Color, u32)]| -> Vec<Color> {
        let total: u32 = v.iter().map(|e| e.1).sum();
        let keep: Vec<Color> = v
            .iter()
            .filter(|e| e.1 >= 3 || e.1 as f32 >= total as f32 * 0.15)
            .map(|e| e.0)
            .collect();
        if keep.is_empty() {
            v.iter().map(|e| e.0).collect()
        } else {
            keep
        }
    };
    // The colour whose worst contrast over the backings is highest: the
    // asked one when it already reads everywhere, else a role candidate.
    let pick_text = |asked: Color, backs: &[Color], px: f32| -> Color {
        let need = backs
            .iter()
            .map(|b| crate::palette::ink_need_on(px, *b))
            .fold(0.0f32, f32::max)
            + crate::palette::MARGIN;
        let worst = |c: Color| {
            backs
                .iter()
                .map(|b| crate::palette::contrast(c, *b))
                .fold(f32::MAX, f32::min)
        };
        if worst(asked) >= need {
            return asked;
        }
        let mut cands: Vec<Color> = Vec::new();
        for b in backs {
            let r = col.roles(*b);
            cands.extend([r.text(asked, px), r.ink]);
        }
        cands
            .into_iter()
            .max_by(|a, b| worst(*a).total_cmp(&worst(*b)))
            .unwrap_or(asked)
    };
    for s in &mut project.scenes {
        let sid = s.id.clone();
        for_each_layer(&mut s.layers, &mut |l| {
            let key = (sid.clone(), l.id.clone());
            match &mut l.kind {
                // A stacked title's echoes are decoration behind its type.
                LayerKind::Text(_) if l.id.contains(".echo") => {}
                LayerKind::Text(st) => {
                    if let Some((backs, px)) = text.get(&key) {
                        let want = pick_text(Color { a: 255, ..st.color }, &lasting(backs), *px);
                        st.color = Color {
                            a: st.color.a,
                            ..want
                        };
                    }
                }
                LayerKind::Path {
                    fill: None,
                    stroke: Some(sk),
                    ..
                } => {
                    if let Some(backs) = strokes.get(&key) {
                        let opaque = Color { a: 255, ..sk.color };
                        let low = lasting(backs).into_iter().min_by(|a, b| {
                            crate::palette::contrast(opaque, *a)
                                .total_cmp(&crate::palette::contrast(opaque, *b))
                        });
                        sk.color = match low {
                            Some(b) if crate::palette::contrast(opaque, b) < 3.0 => {
                                col.roles(b).structure_strong
                            }
                            _ => opaque,
                        };
                    }
                }
                _ => {}
            }
        });
    }
}

fn for_each_layer(layers: &mut [Layer], f: &mut dyn FnMut(&mut Layer)) {
    for l in layers {
        f(l);
        if let LayerKind::Group { children } = &mut l.kind {
            for_each_layer(children, f);
        }
    }
}

// ---------------------------------------------------------------- content

/// A piece of content (a filled shape, a card, a bar, a stroke or curve) in
/// canvas space: texture keeps clear of it.
#[derive(Debug, Clone)]
pub(crate) struct Solid {
    /// Outline rings (filled shapes) or the stroke's centre line.
    pub rings: Vec<Vec<[f32; 2]>>,
    pub filled: bool,
    /// Half the stroke width (0 for fills).
    pub half: f32,
    pub bbox: [f32; 4],
}

fn seg_dist(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}

/// Nonzero winding number of `p` in closed rings.
fn winding(rings: &[Vec<[f32; 2]>], p: [f32; 2]) -> i32 {
    let mut wind = 0;
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + 1) % n]);
            let cross = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
            if a[1] <= p[1] {
                if b[1] > p[1] && cross > 0.0 {
                    wind += 1;
                }
            } else if b[1] <= p[1] && cross < 0.0 {
                wind -= 1;
            }
        }
    }
    wind
}

impl Solid {
    /// Distance from `p` to the content (0 inside a filled shape).
    pub fn clearance(&self, p: [f32; 2]) -> f32 {
        let far = (self.bbox[0] - p[0])
            .max(p[0] - self.bbox[2])
            .max(self.bbox[1] - p[1])
            .max(p[1] - self.bbox[3]);
        if far > 400.0 {
            return far;
        }
        let mut d = f32::MAX;
        let mut wind = 0i32;
        for r in &self.rings {
            for w in r.windows(2) {
                d = d.min(seg_dist(p, w[0], w[1]));
                if self.filled {
                    let (a, b) = (w[0], w[1]);
                    let cross = (b[0] - a[0]) * (p[1] - a[1]) - (p[0] - a[0]) * (b[1] - a[1]);
                    if a[1] <= p[1] {
                        if b[1] > p[1] && cross > 0.0 {
                            wind += 1;
                        }
                    } else if b[1] <= p[1] && cross < 0.0 {
                        wind -= 1;
                    }
                }
            }
        }
        if self.filled && wind != 0 {
            0.0
        } else {
            (d - self.half).max(0.0)
        }
    }
}

/// Flattened contours of a path in canvas space; with `trim`, only the
/// first `trim` of the total length.
fn flatten_path(
    g: &motion_core::path::BezPath,
    t: &motion_core::timeline::Affine,
    trim: Option<f32>,
    close: bool,
) -> Vec<Vec<[f32; 2]>> {
    let mut rings: Vec<Vec<[f32; 2]>> = Vec::new();
    for c in &g.contours {
        let mut pts = vec![c.start];
        let mut p0 = c.start;
        for seg in &c.segs {
            for k in 1..=8 {
                pts.push(seg.at(p0, k as f32 / 8.0));
            }
            p0 = seg.to;
        }
        if close && c.closed {
            pts.push(c.start);
        }
        rings.push(
            pts.iter()
                .map(|q| {
                    let (x, y) = t.apply(q[0], q[1]);
                    [x, y]
                })
                .collect(),
        );
    }
    match trim {
        Some(f) if f < 0.999 => trim_rings(rings, f),
        _ => rings,
    }
}

/// The content of one resolved layer, if it is content: filled shapes and
/// strokes that are visible (≥ 50 % effective opacity) and not full-frame.
pub(crate) fn solid_of(
    l: &motion_core::timeline::ResolvedLayer<'_>,
    alpha: f32,
    frame_area: f32,
) -> Option<Solid> {
    if alpha * l.opacity < 0.5 {
        return None;
    }
    // Transition wipes pass over everything; they are not content.
    if l.scene.is_some_and(|s| s.starts_with("transition")) {
        return None;
    }
    let (x0, y0) = l.transform.apply(0.0, 0.0);
    let (x1, y1) = l.transform.apply(l.width, l.height);
    if ((x1 - x0) * (y1 - y0)).abs() >= frame_area * 0.9 {
        return None;
    }
    let sx = (l.transform.a * l.transform.a + l.transform.b * l.transform.b).sqrt();
    let (rings, filled, half) = match l.kind {
        LayerKind::Rectangle { .. }
        | LayerKind::RoundedRectangle { .. }
        | LayerKind::Image { .. }
        | LayerKind::Svg { .. } => {
            let c = [
                l.transform.apply(0.0, 0.0),
                l.transform.apply(l.width, 0.0),
                l.transform.apply(l.width, l.height),
                l.transform.apply(0.0, l.height),
                l.transform.apply(0.0, 0.0),
            ];
            (vec![c.iter().map(|(x, y)| [*x, *y]).collect()], true, 0.0)
        }
        LayerKind::Path {
            d, fill, stroke, ..
        } => {
            let g = l
                .path
                .clone()
                .or_else(|| motion_core::path::parse(d).ok())?;
            let ct = canvas_content_transform(l);
            match (fill, stroke) {
                (Some(_), _) => (flatten_path(&g, &ct, None, true), true, 0.0),
                (None, Some(s)) => (
                    flatten_path(&g, &ct, l.trim, true),
                    false,
                    s.width * sx / 2.0,
                ),
                _ => return None,
            }
        }
        LayerKind::Polyline {
            points,
            stroke,
            closed,
            fill,
        } => {
            let pts = l.points.as_ref().unwrap_or(points);
            let ct = canvas_content_transform(l);
            let mut ring: Vec<[f32; 2]> = pts
                .iter()
                .map(|q| {
                    let (x, y) = ct.apply(q[0], q[1]);
                    [x, y]
                })
                .collect();
            if *closed && !ring.is_empty() {
                ring.push(ring[0]);
            }
            if fill.is_some() && *closed {
                (vec![ring], true, 0.0)
            } else {
                let g = vec![ring];
                // Trim of a polyline: keep the visible fraction.
                let g = match l.trim {
                    Some(f) if f < 0.999 => trim_rings(g, f),
                    _ => g,
                };
                (g, false, stroke.width * sx / 2.0)
            }
        }
        _ => return None,
    };
    // A mask reveals part of the content: only what shows counts (a line
    // drawn on by a moving mask clears texture as it is drawn, not before).
    let rings = match &l.mask {
        Some(m) => {
            let poly = flatten_path(&m.path, &l.transform, None, true);
            let shows = |p: &[f32; 2]| (winding(&poly, *p) != 0) != m.invert;
            let mut out: Vec<Vec<[f32; 2]>> = Vec::new();
            for r in rings {
                let mut cur: Vec<[f32; 2]> = Vec::new();
                for p in r {
                    if shows(&p) {
                        cur.push(p);
                    } else if cur.len() > 1 {
                        out.push(std::mem::take(&mut cur));
                    } else {
                        cur.clear();
                    }
                }
                if cur.len() > 1 {
                    out.push(cur);
                }
            }
            if out.is_empty() {
                return None;
            }
            out
        }
        None => rings,
    };
    let mut bbox = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    for r in &rings {
        for p in r {
            bbox = [
                bbox[0].min(p[0]),
                bbox[1].min(p[1]),
                bbox[2].max(p[0]),
                bbox[3].max(p[1]),
            ];
        }
    }
    if bbox[0] > bbox[2] {
        return None;
    }
    let pad = half;
    Some(Solid {
        rings,
        filled,
        half,
        bbox: [bbox[0] - pad, bbox[1] - pad, bbox[2] + pad, bbox[3] + pad],
    })
}

fn trim_rings(rings: Vec<Vec<[f32; 2]>>, f: f32) -> Vec<Vec<[f32; 2]>> {
    let len = |r: &Vec<[f32; 2]>| -> f32 {
        r.windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum()
    };
    let mut left = rings.iter().map(len).sum::<f32>() * f.max(0.0);
    let mut out = Vec::new();
    for r in rings {
        if left <= 0.0 || r.is_empty() {
            break;
        }
        let mut kept = vec![r[0]];
        for w in r.windows(2) {
            let l = (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
            if l <= left {
                kept.push(w[1]);
                left -= l;
            } else {
                let u = left / l.max(1e-6);
                kept.push([
                    w[0][0] + (w[1][0] - w[0][0]) * u,
                    w[0][1] + (w[1][1] - w[0][1]) * u,
                ]);
                left = 0.0;
                break;
            }
        }
        out.push(kept);
    }
    out
}

// ---------------------------------------------------------------- repair

/// The design-system repair (ledger entries with repair `design_system`).
/// The resolver and the layout passes already run at every lowering; this
/// edits the program where only the author's choices can be undone:
/// colour overrides back to roles, busy halftones to quiet grids (else no
/// field), oversized accents and type one step down.
pub struct DesignSystem;

const DESIGN_IDS: [&str; 11] = [
    "ink_contrast",
    "text_on_texture",
    "busy_under_type",
    "texture_hole",
    "pattern_over_solid",
    "stroke_contrast",
    "accent_area",
    "hue_count",
    "contrast_ladder",
    "safe_inset",
    "text_clipped",
];

impl crate::repair::Repair for DesignSystem {
    fn name(&self) -> &'static str {
        "design_system"
    }

    fn handles(&self, id: &str) -> bool {
        DESIGN_IDS.contains(&id)
    }

    fn apply(&self, p: &mut crate::MotionProgram, fails: &[crate::ledger::GateFail]) -> bool {
        use crate::program::{FieldPreset, ShapePreset, Size, Swatch};
        // The components named in the details: "... (component)".
        let named = |ids: &[&str]| -> Vec<String> {
            fails
                .iter()
                .filter(|f| ids.contains(&f.id.as_str()))
                .filter_map(|f| {
                    f.detail
                        .rsplit_once('(')
                        .map(|(_, r)| r.trim_end_matches(')').to_string())
                })
                .collect()
        };
        let has = |id: &str| fails.iter().any(|f| f.id == id);
        let down = |s: Size| match s {
            Size::Xl => Size::L,
            Size::L => Size::M,
            _ => Size::S,
        };
        let mut changed = false;

        // Type and strokes back to their roles.
        let recolour = named(&["ink_contrast", "contrast_ladder", "stroke_contrast"]);
        for c in p.components.iter_mut().filter(|c| recolour.contains(&c.id)) {
            match &mut c.body {
                ComponentBody::Title { colour, .. }
                | ComponentBody::KineticText { colour, .. }
                | ComponentBody::Chart { colour, .. }
                | ComponentBody::Diagram { colour, .. } => {
                    changed |= colour.take().is_some();
                }
                ComponentBody::Shape { colour, preset, .. } => {
                    changed |= colour.take().is_some();
                    if *preset == ShapePreset::Outline && has("stroke_contrast") {
                        *preset = ShapePreset::Solid;
                        changed = true;
                    }
                }
                _ => {}
            }
        }

        // Texture fighting type or content: a halftone near the named
        // components becomes a grid; a grid (or a field named itself) goes.
        let busy = named(&[
            "text_on_texture",
            "busy_under_type",
            "texture_hole",
            "pattern_over_solid",
        ]);
        if !busy.is_empty() {
            let spans: Vec<(f64, f64)> = p
                .components
                .iter()
                .filter(|c| busy.contains(&c.id))
                .map(|c| (c.start, p.end_of(c)))
                .collect();
            let end = p.meta.duration;
            let mut drop: Vec<String> = Vec::new();
            for c in p.components.iter_mut() {
                let (s0, s1) = (c.start, c.end.unwrap_or(end));
                let near = busy.contains(&c.id) || spans.iter().any(|(a, b)| *a < s1 && s0 < *b);
                if let ComponentBody::Field { preset, .. } = &mut c.body {
                    if !near {
                        continue;
                    }
                    if *preset == FieldPreset::Halftone {
                        *preset = FieldPreset::Grid;
                    } else {
                        drop.push(c.id.clone());
                    }
                    changed = true;
                }
            }
            p.components.retain(|c| !drop.contains(&c.id));
        }

        // Accent over its share: accent shapes step down, stress goes.
        if has("accent_area") {
            for c in p.components.iter_mut() {
                match &mut c.body {
                    // A large accent shape turns ink rather than shrink: a
                    // smaller hero would fail `small_composition`.
                    ComponentBody::Shape { size, colour, .. }
                        if colour.unwrap_or(Swatch::Accent) == Swatch::Accent
                            && matches!(size, Size::L | Size::Xl) =>
                    {
                        *colour = Some(Swatch::Ink);
                        for e in c.timeline.iter_mut() {
                            if let crate::program::Verb::Recolour { to } = &mut e.verb {
                                if *to == Swatch::Ink {
                                    *to = Swatch::Muted;
                                }
                            }
                        }
                        changed = true;
                    }
                    ComponentBody::KineticText { stress, .. } if !stress.is_empty() => {
                        stress.clear();
                        changed = true;
                    }
                    _ => {}
                }
            }
        }

        // A fourth hue: the extra grounds go (their sections take the
        // background).
        if has("hue_count") {
            if let Some(pal) = &mut p.palette {
                if pal.grounds.len() > 2 {
                    pal.grounds.truncate(2);
                    for c in p.components.iter_mut() {
                        if let ComponentBody::Backdrop { colour, .. } = &mut c.body {
                            if matches!(colour, Some(Swatch::Ground3 | Swatch::Ground4)) {
                                *colour = None;
                            }
                        }
                    }
                    changed = true;
                }
            }
        }

        // Type outside the safe area or cut: one size down.
        let tight = named(&["safe_inset", "text_clipped"]);
        for c in p.components.iter_mut().filter(|c| tight.contains(&c.id)) {
            if let ComponentBody::Title { size, .. } = &mut c.body {
                if *size != Size::S {
                    *size = down(*size);
                    changed = true;
                }
            }
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use crate::lower::{lower, Typesetter};
    use crate::program::MotionProgram;
    use crate::validate::validate;

    #[test]
    fn test_calm_focus_considers_golden_eyes() {
        let json = r##"{
            "version": "2.0",
            "meta": {"name": "calm_focus_test", "duration": 4, "aspect": "16:9"},
            "palette": {"background": "#0D0E11", "ink": "#EDEDED", "accent": "#4E80EE", "muted": "#7A7E85"},
            "components": [
                {
                    "id": "bg_field",
                    "kind": "field",
                    "preset": "halftone",
                    "focus": "center",
                    "start": 0.0
                },
                {
                    "id": "title",
                    "kind": "title",
                    "text": "MASSIVE CENTER HEADLINE OCCUPYING THE CANVAS",
                    "slot": "center",
                    "start": 0.0
                }
            ]
        }"##;
        let p = MotionProgram::from_json(json).expect("parses");
        validate(&p).expect("validates");
        let project = lower(&p, None, &Typesetter::approx());
        motion_core::validate(&project, None).expect("motion-core valid");

        // Background field scene should exist and have instances
        let bg_scene = project
            .scenes
            .iter()
            .find(|s| s.id == "bg_field")
            .expect("bg scene");
        assert!(!bg_scene.layers.is_empty());
    }
}
