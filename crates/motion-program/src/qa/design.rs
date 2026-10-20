//! Rendered design-system detectors (`DESIGN_COLOR_SYSTEM.md` §6.3). Each
//! one is a mistake-ledger entry (`assets/qa/mistakes.json`, detector
//! `design.<id>`) and reports [`GateFail`]s with the ledger id; the
//! measured value and the limit are in the detail, and details end with
//! `(component)` so repairs can find what to edit.
//!
//! They judge frames sampled at ≥ 2 fps, three renders each: the composite,
//! the frame with every run of type hidden (what is *behind* the type) and
//! the frame with type and strokes hidden (what is behind a stroke).
//! Rendering is the caller's (`render` callback), so this stays
//! renderer-free and deterministic.

use motion_core::scene::{Color, LayerKind, MotionProject};
use motion_core::timeline::{ResolvedFrame, ResolvedLayer};

use crate::ledger::GateFail;
use crate::palette::{self, contrast, oklab, oklch};

pub const ACCENT_MAX: f64 = 0.10;
pub const HUES_MAX: usize = palette::HUES_MAX;
/// OKLab L standard deviation under a run of type.
pub const BUSY_MAX: f32 = 0.06;
/// Strokes and curves against what is behind them.
pub const STROKE_MIN: f32 = 3.0;

/// The ledger ids these detectors report.
pub const IDS: [&str; 11] = [
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

/// An RGB frame (row-major, 3 bytes per pixel).
pub struct Pixels {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
}

impl Pixels {
    pub fn at(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y.min(self.h - 1) * self.w + x.min(self.w - 1)) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }
}

fn col(p: [u8; 3]) -> Color {
    Color::rgb(p[0], p[1], p[2])
}

/// The colour a layer is drawn in this frame (tint applied).
fn drawn(c: Color, l: &ResolvedLayer<'_>) -> Color {
    match &l.tint {
        Some(t) => {
            let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t.amount).round() as u8;
            Color::rgb(m(c.r, t.color.r), m(c.g, t.color.g), m(c.b, t.color.b))
        }
        None => Color { a: 255, ..c },
    }
}

/// Canvas rect of box-space `[x0, y0, x1, y1]` under `l`'s transform (or projective homography).
fn rect_of(l: &ResolvedLayer<'_>, b: [f32; 4]) -> [f32; 4] {
    let apply = |x: f32, y: f32| -> (f32, f32) {
        if let Some(h) = l.projective {
            let w = (h[6] * x + h[7] * y + h[8]).max(1e-6);
            (
                (h[0] * x + h[1] * y + h[2]) / w,
                (h[3] * x + h[4] * y + h[5]) / w,
            )
        } else {
            l.transform.apply(x, y)
        }
    };
    let pts = [
        apply(b[0], b[1]),
        apply(b[2], b[1]),
        apply(b[0], b[3]),
        apply(b[2], b[3]),
    ];
    pts.iter()
        .fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |q, (x, y)| {
            [q[0].min(*x), q[1].min(*y), q[2].max(*x), q[3].max(*y)]
        })
}

/// Every layer of a resolved frame with its effective opacity (parents
/// multiplied), in draw order.
fn flatten<'a, 'b>(layers: &'b [ResolvedLayer<'a>]) -> Vec<(&'b ResolvedLayer<'a>, f32)> {
    let mut out = Vec::new();
    let mut stack: Vec<(&ResolvedLayer<'a>, f32)> = layers.iter().rev().map(|l| (l, 1.0)).collect();
    while let Some((l, a)) = stack.pop() {
        let e = a * l.opacity;
        out.push((l, e));
        stack.extend(l.children.iter().rev().map(|c| (c, e)));
    }
    out
}

fn is_stroke(l: &ResolvedLayer<'_>) -> bool {
    matches!(
        l.kind,
        LayerKind::Path {
            fill: None,
            stroke: Some(_),
            ..
        } | LayerKind::Polyline { fill: None, .. }
    )
}

/// Hides every text layer (and, with `strokes`, every stroke-only path and
/// polyline), children included.
pub fn hide(layers: &mut [ResolvedLayer<'_>], strokes: bool) {
    for l in layers {
        if matches!(l.kind, LayerKind::Text(_)) || (strokes && is_stroke(l)) {
            l.opacity = 0.0;
        }
        hide(&mut l.children, strokes);
    }
}

/// The component a layer belongs to: its scene id up to the first `.`.
fn owner(l: &ResolvedLayer<'_>) -> String {
    l.scene
        .unwrap_or(l.id)
        .split('.')
        .next()
        .unwrap_or("")
        .to_string()
}

/// A run of type judged on one frame.
#[derive(Debug, Clone)]
struct Run {
    label: String,
    owner: String,
    rect: [f32; 4],
    colour: Color,
    px: f32,
    /// Median colour behind the run (type hidden).
    back: Color,
    /// (v2.2) How the glyphs are painted (styled fills are judged on the
    /// colour they actually show, see [`Acc::observe`]).
    fill: RunFill,
}

/// (v2.2) The paint of a run of type, for the contrast gate.
#[derive(Debug, Clone)]
enum RunFill {
    /// `colour` as is.
    Solid,
    /// A gradient: judged on its worst stop against the ground.
    Stops(Vec<Color>),
    /// An image seen through the glyphs: judged on the median-luminance
    /// pixel the glyphs actually show (composite pixels that differ from
    /// the type-hidden frame).
    Sampled,
}

fn runs(
    all: &[(&ResolvedLayer<'_>, f32)],
    settled: &dyn Fn(&ResolvedLayer<'_>) -> bool,
) -> Vec<Run> {
    let mut out = Vec::new();
    for (l, a) in all {
        let LayerKind::Text(st) = l.kind else {
            continue;
        };
        // Settled, fully shown type only; a stacked title's echoes and a
        // lockup's sheen (light passing over the headline) are part of its
        // type, not its ground.
        if *a < 0.95
            || l.glyphs.is_some()
            || st.color.a < 200
            || l.id.contains(".echo")
            || l.id.contains(".sheen")
            || !settled(l)
        {
            continue;
        }
        let (top, bottom) = st.ink.map_or((0.0, l.height), |i| (i.top, i.bottom));
        let rect = rect_of(l, [0.0, top, l.width, bottom]);
        let sy = (rect[3] - rect[1]) / (bottom - top).max(1.0);
        let text = l.text.clone().unwrap_or_else(|| st.text.clone());
        let fill = match &st.fill {
            None => RunFill::Solid,
            Some(motion_core::scene::TextFill::Gradient(g)) => {
                RunFill::Stops(g.stops().iter().map(|s| drawn(s.color, l)).collect())
            }
            Some(motion_core::scene::TextFill::Image(_)) => RunFill::Sampled,
        };
        out.push(Run {
            label: format!("'{}'", text.chars().take(16).collect::<String>()),
            owner: owner(l),
            rect,
            colour: drawn(st.color, l),
            px: st.font_size * sy,
            back: Color::rgb(0, 0, 0),
            fill,
        });
    }
    out
}

/// Pixel samples (about 60 per side) inside `r`.
fn samples(p: &Pixels, r: [f32; 4]) -> Vec<[u8; 3]> {
    let (w, h) = (p.w as f32, p.h as f32);
    let (x0, y0) = (r[0].max(0.0), r[1].max(0.0));
    let (x1, y1) = (r[2].min(w - 1.0), r[3].min(h - 1.0));
    if x1 <= x0 || y1 <= y0 {
        return Vec::new();
    }
    let step = ((x1 - x0).max(y1 - y0) / 60.0).max(1.0);
    let mut out = Vec::new();
    let mut y = y0;
    while y <= y1 {
        let mut x = x0;
        while x <= x1 {
            out.push(p.at(x as u32, y as u32));
            x += step;
        }
        y += step;
    }
    out
}

/// The most common colour of a frame (its ground), 5-bit quantised.
fn mode_colour(p: &Pixels) -> Color {
    let mut counts: std::collections::HashMap<[u8; 3], (usize, [u8; 3])> = Default::default();
    for y in (0..p.h).step_by(8) {
        for x in (0..p.w).step_by(8) {
            let c = p.at(x, y);
            let e = counts
                .entry([c[0] >> 3, c[1] >> 3, c[2] >> 3])
                .or_insert((0, c));
            e.0 += 1;
        }
    }
    counts
        .values()
        .max_by_key(|v| (v.0, v.1))
        .map_or(Color::rgb(0, 0, 0), |v| col(v.1))
}

/// Failures of one detector, deduplicated by subject.
#[derive(Debug, Clone, Default)]
struct Acc {
    fails: Vec<(String, String)>,
}

impl Acc {
    /// `subject` dedupes (first time seen wins); `detail` is reported.
    fn fail(&mut self, subject: String, detail: String) {
        if self.fails.len() < 6 && !self.fails.iter().any(|f| f.0 == subject) {
            self.fails.push((subject, detail));
        }
    }
}

/// Accumulates every detector over the sampled frames of one video.
pub struct Detectors {
    /// The program's accent colours (main and per-ground variants).
    accents: Vec<Color>,
    acc: std::collections::BTreeMap<&'static str, Acc>,
    accent_sum: f64,
    accent_max: f64,
    frames: usize,
    hues: [usize; 12],
    hue_total: usize,
}

impl Detectors {
    pub fn new(accents: Vec<Color>) -> Detectors {
        Detectors {
            accents,
            acc: Default::default(),
            accent_sum: 0.0,
            accent_max: 0.0,
            frames: 0,
            hues: [0; 12],
            hue_total: 0,
        }
    }

    fn fail(&mut self, id: &'static str, subject: String, detail: String) {
        self.acc.entry(id).or_default().fail(subject, detail);
    }

    /// Accent colours of a program: every role-set accent and its type
    /// variant (an accent that became the ink on some ground is not one).
    pub fn accents_of(p: &crate::MotionProgram) -> Vec<Color> {
        let Some(pal) = p.palette() else {
            return Vec::new();
        };
        let r = palette::resolve(&pal);
        let mut v = vec![r.main.accent, r.main.accent_text];
        for g in &r.grounds {
            v.push(g.accent);
            v.push(g.accent_text);
        }
        v.retain(|a| *a != r.main.ink && !r.grounds.iter().any(|g| g.ink == *a));
        v.dedup();
        v
    }

    /// Judges one sampled frame. `settled` says whether a layer holds
    /// still (same pose a few frames later); `comp` is the composite,
    /// `behind` the render without type, `bare` without type and strokes.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn observe(
        &mut self,
        t: f64,
        rf: &ResolvedFrame<'_>,
        settled: &dyn Fn(&ResolvedLayer<'_>) -> bool,
        comp: &Pixels,
        behind: &Pixels,
        bare: &Pixels,
    ) {
        let all = flatten(&rf.layers);
        let mut rs = runs(&all, settled);
        let (w, h) = (comp.w as f32, comp.h as f32);
        let ground = mode_colour(behind);
        self.frames += 1;

        // Type: contrast against the pixels behind it, busy ground, inset,
        // clipping.
        let mut focal: Option<(f32, f32)> = None; // (px, contrast)
        for r in rs.iter_mut() {
            let back = samples(behind, r.rect);
            if back.len() >= 8 {
                let mut by_l: Vec<(f32, [u8; 3])> = back
                    .iter()
                    .map(|b| (palette::luminance(col(*b)), *b))
                    .collect();
                by_l.sort_by(|a, b| a.0.total_cmp(&b.0));
                r.back = col(by_l[by_l.len() / 2].1);
                // (v2.2) Styled fills: the colour judged is the gradient's
                // worst stop on this ground, or the glyph pixels an image
                // fill actually shows (median luminance).
                match &r.fill {
                    RunFill::Solid => {}
                    RunFill::Stops(stops) => {
                        if let Some(c) = stops
                            .iter()
                            .min_by(|a, b| contrast(**a, r.back).total_cmp(&contrast(**b, r.back)))
                        {
                            r.colour = *c;
                        }
                    }
                    RunFill::Sampled => {
                        let shown = samples(comp, r.rect);
                        let mut glyph: Vec<(f32, [u8; 3])> = shown
                            .iter()
                            .zip(&back)
                            .filter(|(c, b)| {
                                (0..3).map(|i| c[i].abs_diff(b[i]) as u32).sum::<u32>() > 48
                            })
                            .map(|(c, _)| (palette::luminance(col(*c)), *c))
                            .collect();
                        if glyph.len() >= 8 {
                            glyph.sort_by(|a, b| a.0.total_cmp(&b.0));
                            r.colour = col(glyph[glyph.len() / 2].1);
                        } else {
                            // The image matches the ground: the word is
                            // invisible, judged as ground on ground.
                            r.colour = r.back;
                        }
                    }
                }
                let mut ratios: Vec<f32> =
                    back.iter().map(|b| contrast(r.colour, col(*b))).collect();
                ratios.sort_by(f32::total_cmp);
                // The band on the ground behind the run (median), and the
                // band over its texture on the worst pixels (texture is
                // capped at 1.35:1, so 7:1 type still reads 5.2:1 on it).
                let worst = ratios[ratios.len() / 10];
                let median = ratios[ratios.len() / 2];
                let need = palette::ink_need_on(r.px, r.back);
                let floor = need / palette::TEXTURE_MAX;
                if median < need || worst < floor {
                    self.fail(
                        "ink_contrast",
                        r.label.clone(),
                        format!(
                            "{} {median:.2}:1 on its ground (limit {need}), {worst:.2}:1 worst (limit {floor:.2}) at {t:.2}s ({})",
                            r.label, r.owner
                        ),
                    );
                }
                if focal.is_none_or(|f| r.px > f.0) {
                    focal = Some((r.px, median));
                }
                let ls: Vec<f32> = back.iter().map(|b| oklab(col(*b))[0]).collect();
                let mean = ls.iter().sum::<f32>() / ls.len() as f32;
                let sd =
                    (ls.iter().map(|l| (l - mean).powi(2)).sum::<f32>() / ls.len() as f32).sqrt();
                let busy_limit = if median >= 8.0 {
                    0.16
                } else if median >= 5.5 {
                    0.11
                } else {
                    BUSY_MAX
                };
                if sd > busy_limit {
                    self.fail(
                        "busy_under_type",
                        r.label.clone(),
                        format!(
                            "{} L sd {sd:.3}, limit {busy_limit} at {t:.2}s ({})",
                            r.label, r.owner
                        ),
                    );
                }
            }
            if r.rect[0] < -1.0 || r.rect[1] < -1.0 || r.rect[2] > w + 1.0 || r.rect[3] > h + 1.0 {
                self.fail(
                    "text_clipped",
                    r.label.clone(),
                    format!("{} cut by the frame at {t:.2}s ({})", r.label, r.owner),
                );
            }
            let m = w.min(h) * 0.05 - 2.0;
            let out = (m - r.rect[0])
                .max(m - r.rect[1])
                .max(r.rect[2] - (w - m))
                .max(r.rect[3] - (h - m));
            if out > 0.0 {
                self.fail(
                    "safe_inset",
                    r.label.clone(),
                    format!(
                        "{} {out:.0}px outside the 5% inset at {t:.2}s ({})",
                        r.label, r.owner
                    ),
                );
            }
        }
        for (l, a) in &all {
            if let (LayerKind::Text(st), Some(c)) = (l.kind, l.clip) {
                if *a >= 0.95
                    && settled(l)
                    && (c.left > 0.5 || c.right > 0.5 || c.top > 0.5 || c.bottom > 0.5)
                {
                    let label = format!("'{}'", st.text.chars().take(16).collect::<String>());
                    self.fail(
                        "text_clipped",
                        label.clone(),
                        format!("{label} clipped at {t:.2}s ({})", owner(l)),
                    );
                }
            }
        }

        // Content (shapes, bars, cards, strokes) with its draw position.
        let area = w * h;
        let solids: Vec<(String, crate::layout::Solid, usize)> = all
            .iter()
            .enumerate()
            .filter(|(_, (l, _))| {
                !matches!(l.kind, LayerKind::Instances { .. } | LayerKind::Text(_))
            })
            .filter_map(|(i, (l, a))| {
                crate::layout::solid_of(l, *a, area).map(|s| (owner(l), s, i))
            })
            .collect();

        // Strokes and curves: >= 3:1 against what is behind them.
        for (l, a) in &all {
            if *a < 0.95 || !is_stroke(l) || !settled(l) {
                continue;
            }
            let colour = match l.kind {
                LayerKind::Path {
                    stroke: Some(s), ..
                } => s.color,
                LayerKind::Polyline { stroke, .. } => stroke.color,
                _ => continue,
            };
            let colour = drawn(colour, l);
            let Some(s) = crate::layout::solid_of(l, *a, area) else {
                continue;
            };
            let me = owner(l);
            let own_fills: Vec<&crate::layout::Solid> = solids
                .iter()
                .filter(|(o, sd, _)| *o == me && sd.filled)
                .map(|(_, sd, _)| sd)
                .collect();
            // What separates a line from its surroundings: the pixels just
            // outside both of its edges (strokes and type hidden).
            let off = s.half + (s.half * 0.5).max(1.5);
            let mut ratios: Vec<f32> = Vec::new();
            for r in &s.rings {
                for wpair in r.windows(2) {
                    let (dx, dy) = (wpair[1][0] - wpair[0][0], wpair[1][1] - wpair[0][1]);
                    let len = dx.hypot(dy);
                    if len < 1e-3 {
                        continue;
                    }
                    let (nx, ny) = (-dy / len, dx / len);
                    for k in 0..4 {
                        let u = k as f32 / 4.0;
                        let (x, y) = (wpair[0][0] + dx * u, wpair[0][1] + dy * u);
                        for side in [-1.0f32, 1.0] {
                            let (px, py) = (x + nx * off * side, y + ny * off * side);
                            // A line meeting its own component's fill (a
                            // base under its bars) is joined, not lost.
                            let joined = own_fills.iter().any(|f| f.clearance([px, py]) <= 2.5);
                            if !joined && px >= 0.0 && py >= 0.0 && px < w && py < h {
                                ratios.push(contrast(colour, col(bare.at(px as u32, py as u32))));
                            }
                        }
                    }
                }
            }
            if ratios.len() < 8 {
                continue;
            }
            ratios.sort_by(f32::total_cmp);
            // 3:1 on the ground beside it, 3 / 1.35 over its texture.
            let worst = ratios[ratios.len() / 10];
            let median = ratios[ratios.len() / 2];
            let floor = STROKE_MIN / palette::TEXTURE_MAX;
            if median < STROKE_MIN || worst < floor {
                self.fail(
                    "stroke_contrast",
                    l.id.to_string(),
                    format!(
                        "{} {median:.2}:1 beside it (limit {STROKE_MIN}), {worst:.2}:1 worst (limit {floor:.2}) at {t:.2}s ({})",
                        l.id,
                        owner(l)
                    ),
                );
            }
        }

        // Texture: separation from ink, band, holes, draw order;
        // decorative contrast for the ladder.
        let mut decor: f32 = 1.0;
        for (idx, (l, a)) in all.iter().enumerate() {
            if *a < 0.05 {
                continue;
            }
            match l.kind {
                LayerKind::Instances { fill, at, .. } => {
                    let tex = match fill {
                        motion_core::scene::Fill::Solid(c)
                        | motion_core::scene::Fill::Watercolor { color: c, .. } => drawn(*c, l),
                        motion_core::scene::Fill::Gradient(g) => {
                            g.stops().first().map_or(ground, |s| s.color)
                        }
                    };
                    // Only texture that shows: dot centres whose pixel (type
                    // hidden) is the texture colour.
                    let (mut shown, mut tried) = (0usize, 0usize);
                    for d in at.iter().filter(|d| d[2] >= 2.0).step_by(7) {
                        let (x, y) = l.transform.apply(d[0], d[1]);
                        if x < 0.0 || y < 0.0 || x >= w || y >= h {
                            continue;
                        }
                        tried += 1;
                        let px = behind.at(x as u32, y as u32);
                        let dist = (px[0] as i32 - tex.r as i32).abs()
                            + (px[1] as i32 - tex.g as i32).abs()
                            + (px[2] as i32 - tex.b as i32).abs();
                        if dist <= 24 {
                            shown += 1;
                        }
                    }
                    if tried == 0 || (shown as f32) < tried as f32 * 0.2 {
                        continue;
                    }
                    let tex_ground = all[..idx]
                        .iter()
                        .rev()
                        .find_map(|(gl, ga)| match gl.kind {
                            LayerKind::Rectangle { fill, .. }
                                if *ga >= 0.95 && gl.width * gl.height >= area * 0.8 =>
                            {
                                Some(drawn(*fill, gl))
                            }
                            _ => None,
                        })
                        .unwrap_or(ground);
                    let band = contrast(tex, tex_ground);
                    decor = decor.max(band);
                    // The ink role (neutral type) on this texture's ground;
                    // accent type is judged by its contrast alone.
                    for r in rs
                        .iter()
                        .filter(|r| oklch(r.colour).c < 0.06 && contrast(r.back, tex_ground) < 1.4)
                    {
                        let dl = (oklch(tex).l - oklch(r.colour).l).abs();
                        if dl < palette::TEXTURE_INK_DL {
                            self.fail(
                                "text_on_texture",
                                r.label.clone(),
                                format!(
                                    "{} shares value with texture {} (dL {dl:.2}, limit {}) at {t:.2}s ({})",
                                    r.label,
                                    l.id,
                                    palette::TEXTURE_INK_DL,
                                    r.owner
                                ),
                            );
                        }
                    }
                    if band > palette::TEXTURE_MAX + 0.02 {
                        self.fail(
                            "text_on_texture",
                            format!("band {}", l.id),
                            format!(
                                "texture {} {band:.2}:1 on its ground, limit {}:1 at {t:.2}s ({})",
                                l.id,
                                palette::TEXTURE_MAX,
                                owner(l)
                            ),
                        );
                    }
                    // A texture is never cut, cleared or faded around an
                    // element (owner): any local hole fails.
                    if let Some((n, at_xy)) = holes(at) {
                        let (hx, hy) = l.transform.apply(at_xy[0], at_xy[1]);
                        self.fail(
                            "texture_hole",
                            l.id.to_string(),
                            format!(
                                "{} is cleared in {n} place(s), e.g. ({hx:.0},{hy:.0}) at {t:.2}s ({})",
                                l.id,
                                owner(l)
                            ),
                        );
                    }
                    // Texture drawn over content (dots on a bar, a card, a
                    // line): texture belongs under what it surrounds.
                    let k = (l.transform.a * l.transform.a + l.transform.b * l.transform.b).sqrt();
                    'dots: for d in at.iter().filter(|d| d[2] >= 1.0) {
                        let (x, y) = l.transform.apply(d[0], d[1]);
                        for (who, sd, _) in solids.iter().filter(|x| x.2 < idx) {
                            let c = sd.clearance([x, y]);
                            if (sd.filled && c == 0.0) || (!sd.filled && c < d[2] * k) {
                                self.fail(
                                    "pattern_over_solid",
                                    format!("{} {who}", l.id),
                                    format!(
                                        "texture {} is drawn over {who} at ({x:.0},{y:.0}) at {t:.2}s ({})",
                                        l.id,
                                        owner(l)
                                    ),
                                );
                                break 'dots;
                            }
                        }
                    }
                }
                LayerKind::Path {
                    fill: None,
                    stroke: Some(s),
                    ..
                } if s.width <= 3.0 => {
                    decor = decor.max(contrast(drawn(s.color, l), ground));
                }
                _ => {}
            }
        }
        if let Some((_, fc)) = focal {
            if decor > fc + 0.35 {
                self.fail(
                    "contrast_ladder",
                    format!("{t:.2}"),
                    format!(
                        "decoration {decor:.2}:1 out-contrasts the focal type {fc:.2}:1 at {t:.2}s"
                    ),
                );
            }
        }

        // Accent area and hues over the composite.
        let g = oklab(ground);
        let accents: Vec<[f32; 3]> = self.accents.iter().map(|a| oklab(*a)).collect();
        let near = |a: [f32; 3], b: [f32; 3]| {
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() < 0.06
        };
        let (mut acc, mut n) = (0usize, 0usize);
        for y in (0..comp.h).step_by(6) {
            for x in (0..comp.w).step_by(6) {
                let px = col(comp.at(x, y));
                let lab = oklab(px);
                n += 1;
                if !near(lab, g) && accents.iter().any(|a| near(lab, *a)) {
                    acc += 1;
                }
                let o = oklch(px);
                if o.c >= 0.07 {
                    self.hues[((o.h / 30.0) as usize).min(11)] += 1;
                    self.hue_total += 1;
                }
            }
        }
        let share = acc as f64 / n.max(1) as f64;
        self.accent_sum += share;
        self.accent_max = self.accent_max.max(share);
    }

    /// Every failure, ledger ids in [`IDS`] order.
    pub fn finish(&self) -> Vec<GateFail> {
        let mut out = Vec::new();
        for id in IDS {
            if let Some(a) = self.acc.get(id) {
                out.extend(a.fails.iter().map(|(_, d)| GateFail {
                    id: id.to_string(),
                    detail: d.clone(),
                }));
            }
        }
        let accent_mean = self.accent_sum / self.frames.max(1) as f64;
        if accent_mean > ACCENT_MAX {
            out.push(GateFail {
                id: "accent_area".into(),
                detail: format!(
                    "accent covers {:.1} % (max frame {:.1} %), limit {:.0} %",
                    accent_mean * 100.0,
                    self.accent_max * 100.0,
                    ACCENT_MAX * 100.0
                ),
            });
        }
        // Hue families: runs of adjacent 30° bins holding >= 1 % of all
        // chromatic samples (the circle wraps).
        let strong: Vec<bool> = self
            .hues
            .iter()
            .map(|c| self.hue_total > 0 && *c as f64 / self.hue_total as f64 >= 0.01)
            .collect();
        let mut families = (0..12)
            .filter(|&i| strong[i] && !strong[(i + 11) % 12])
            .count();
        if families == 0 && strong.iter().all(|s| *s) {
            families = 12;
        }
        if families > HUES_MAX {
            out.push(GateFail {
                id: "hue_count".into(),
                detail: format!("{families} hue families, limit {HUES_MAX}"),
            });
        }
        out
    }
}

/// Grid step of a dot field: the smallest positive gap between x positions.
fn grid_step(at: &[[f32; 3]]) -> f32 {
    let mut xs: Vec<f32> = at.iter().map(|a| a[0]).collect();
    xs.sort_by(f32::total_cmp);
    let s = xs
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.5)
        .fold(f32::MAX, f32::min);
    if s == f32::MAX {
        20.0
    } else {
        s
    }
}

/// Local holes in a dot field: a dot much smaller than every one of its
/// grid neighbours (< 60 % of the smallest), or a gone dot (< 0.3 px)
/// beside full ones. A field's own size law (a halftone's falloff, a
/// uniform grid) never does that; a knockout does. Returns the count and
/// one position (box space).
pub fn holes(at: &[[f32; 3]]) -> Option<(usize, [f32; 2])> {
    let max = at.iter().map(|a| a[2]).fold(0.0f32, f32::max);
    if at.len() < 9 || max < 1.0 {
        return None;
    }
    let step = grid_step(at);
    let key = |x: f32, y: f32| ((x / step).round() as i64, (y / step).round() as i64);
    let grid: std::collections::HashMap<(i64, i64), f32> =
        at.iter().map(|a| (key(a[0], a[1]), a[2])).collect();
    let mut n = 0;
    let mut first = None;
    for a in at {
        let (i, j) = key(a[0], a[1]);
        let nb: Vec<f32> = [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)]
            .iter()
            .filter_map(|k| grid.get(k).copied())
            .collect();
        let gone_beside_full = a[2] < 0.3 && nb.iter().filter(|s| **s >= 0.5 * max).count() >= 2;
        let local_min = nb.len() == 4
            && nb.iter().all(|s| *s >= 1.0)
            && a[2] < 0.6 * nb.iter().copied().fold(f32::MAX, f32::min);
        if gone_beside_full || local_min {
            n += 1;
            first.get_or_insert([a[0], a[1]]);
        }
    }
    first.map(|p| (n, p))
}

/// Runs every design detector over `project`, sampling `per_second`
/// frames a second (≥ 2). `render` turns a resolved frame into pixels.
pub fn run(
    p: &crate::MotionProgram,
    project: &MotionProject,
    per_second: f64,
    render: &mut dyn FnMut(&ResolvedFrame<'_>) -> Result<Pixels, String>,
) -> Result<Vec<GateFail>, String> {
    let fps = project.canvas.fps as f64;
    let last = project.frame_count().saturating_sub(1);
    let step = (fps / per_second.max(2.0)).floor().max(1.0) as u32;
    let mut d = Detectors::new(Detectors::accents_of(p));
    let mut f = step / 2;
    while f <= last {
        let rf = motion_core::evaluate_frame(project, f).map_err(|e| e.to_string())?;
        let later =
            motion_core::evaluate_frame(project, (f + 3).min(last)).map_err(|e| e.to_string())?;
        type Pose = (
            f32,
            f32,
            f32,
            f32,
            f32,
            Option<(f32, f32, f32, f32)>,
            Option<f32>,
            Option<[i32; 4]>,
        );
        let pose = |l: &ResolvedLayer<'_>| -> Pose {
            let (a, b) = (
                l.transform.apply(0.0, 0.0),
                l.transform.apply(l.width, l.height),
            );
            (
                a.0,
                a.1,
                b.0,
                b.1,
                l.opacity,
                l.clip.map(|c| (c.left, c.top, c.right, c.bottom)),
                l.trim,
                // A moving reveal mask is motion too.
                l.mask
                    .as_ref()
                    .and_then(|m| m.path.bounds())
                    .map(|b| b.map(|v| v.round() as i32)),
            )
        };
        // At rest: the same pose ~0.2 s before and a few frames after.
        let lag = ((0.2 * fps).ceil() as u32).min(f);
        let earlier = motion_core::evaluate_frame(project, f - lag).map_err(|e| e.to_string())?;
        let then: std::collections::HashMap<&str, Pose> = flatten(&later.layers)
            .into_iter()
            .map(|(l, _)| (l.id, pose(l)))
            .collect();
        let before: std::collections::HashMap<&str, Pose> = flatten(&earlier.layers)
            .into_iter()
            .map(|(l, _)| (l.id, pose(l)))
            .collect();
        let same = |a: &Pose, q: &Pose| {
            (a.0 - q.0).abs() < 1.0
                && (a.1 - q.1).abs() < 1.0
                && (a.2 - q.2).abs() < 1.0
                && (a.3 - q.3).abs() < 1.0
                && (a.4 - q.4).abs() < 0.02
                && a.5 == q.5
                && a.6.zip(q.6).is_none_or(|(x, y)| (x - y).abs() < 0.01)
                && a.7 == q.7
        };
        let settled = |l: &ResolvedLayer<'_>| {
            let a = pose(l);
            then.get(l.id).is_some_and(|q| same(&a, q))
                && before.get(l.id).is_some_and(|q| same(&a, q))
        };
        let comp = render(&rf)?;
        let mut hidden = rf.clone();
        hide(&mut hidden.layers, false);
        let behind = render(&hidden)?;
        hide(&mut hidden.layers, true);
        let bare = render(&hidden)?;
        d.observe(f as f64 / fps, &rf, &settled, &comp, &behind, &bare);
        f += step;
    }
    Ok(d.finish())
}
