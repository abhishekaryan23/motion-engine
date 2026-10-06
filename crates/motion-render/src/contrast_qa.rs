//! (0.23 W4) `text_local_contrast`: does every text layer the viewer reads at
//! a beat's READ moment contrast with what is actually drawn around it?
//!
//! The structural layout QA judges boxes; it never looks at pixels, so a
//! black headline on a black print field passed. This check renders each beat
//! scene's READ frame in memory (one [`CpuRenderer`] prepared per project,
//! frames never written to disk) and compares every readable text layer
//! with the rendered pixels around its ink box.
//!
//! * **Frame.** The beat's READ moment (`lifecycle.read`, scene-local ->
//!   project time, the frame layout QA samples). The frame is rendered at the
//!   project's own size, then box-filtered down by an integer factor until it
//!   is at most [`MAX_WIDTH_PX`] wide, so the ring below is measured on the
//!   shipped frame at a fixed scale.
//! * **Layers judged.** Text layers readable then ([`readable_leaves`]) of the
//!   beat scene itself with their clip window at least half filled, that are
//!   not decorative (`layout_qa::is_decorative`) and not captions (the
//!   `captions` scene is not a beat scene).
//! * **Ink colour.** The layer's fill, moved toward its tint as drawn. Text
//!   has plain colour fills only, so no layer is skipped for a gradient or a
//!   textured fill; layers that cannot be measured (no font, degenerate
//!   perspective, ring outside the canvas) are reported as `skipped`.
//! * **Ground.** The per-channel median of the rendered pixels in a ring
//!   [`RING_NEAR_PX`]..[`RING_FAR_PX`] px outside the ink box (at the analysis
//!   scale), dropping the glyphs of neighbouring text: pixels within
//!   [`NEAR_INK_DISTANCE`] (RGB distance, out of 255) of the ink that belong to
//!   thin strokes. An ink-coloured pixel whose surroundings (a square window of
//!   [`GLYPH_WINDOW_EM`] em either side, wider than any stroke) average to the
//!   ink's colour within [`FIELD_MEAN_DISTANCE`] sits inside a solid field, not
//!   a glyph, and stays in the ring: a black headline half over a black print
//!   field reads as half black ground. (Dropping every ink-coloured pixel would
//!   hide exactly that defect.)
//! * **Words.** A layer is judged on the ring around its whole ink box (the
//!   rule above). When it clears that and holds several words, each word is
//!   judged on the ring around its own ink box too and the worst word counts:
//!   the first word of a line that slides under a black circle must not pass
//!   because the rest of the line sits on paper.
//! * **Threshold.** WCAG contrast at least `TEXT_CONTRAST_DISPLAY` when the
//!   drawn font size is at least `DISPLAY_TEXT_PX` x u (u = short side / 1080),
//!   else `TEXT_CONTRAST_BODY`.

use std::collections::BTreeMap;
use std::path::Path;

use motion_core::checks::{
    DISPLAY_TEXT_PX, TEXT_CONTRAST_BODY, TEXT_CONTRAST_DISPLAY, TEXT_LOCAL_CONTRAST,
};
use motion_core::layout_qa::{LayoutCheck, LayoutFinding};
use motion_core::scene::{AssetKind, Color, LayerKind, MotionProject};
use motion_core::timeline::{evaluate_frame, ResolvedLayer};
use resvg::tiny_skia::Pixmap;

use crate::reveal_qa::{readable_leaves, Limits};
use crate::speech_qa::{CheckStatus, SpeechQaCheck};
use crate::story_qa::{find_layer, is_content_chain, revealed_share, story_beats, ON_SCREEN_SHARE};
use crate::text::TextEngine;
use crate::{CpuRenderer, RenderError, Renderer};

/// The analysis frame is at most this wide (px).
pub const MAX_WIDTH_PX: u32 = 540;
/// The ring around a text's ink box: from `RING_NEAR_PX` to `RING_FAR_PX` px
/// outside it (analysis scale).
pub const RING_NEAR_PX: i32 = 4;
pub const RING_FAR_PX: i32 = 12;
/// A ring pixel within this RGB distance (out of 255) of the ink is a glyph of
/// a neighbouring text, not ground.
pub const NEAR_INK_DISTANCE: f32 = 8.0;
/// Half-width, in em of the text, of the window that tells a glyph stroke from
/// a field of the ink's colour (the window is `2 * round(GLYPH_WINDOW_EM * em)
/// + 1` px square, at least 5): no stroke fills it, a field does.
pub const GLYPH_WINDOW_EM: f32 = 0.15;
/// An ink-coloured ring pixel is part of a field (ground, kept) when the mean
/// colour of its window is within this RGB distance of the ink (grain on a
/// field averages out; a stroke on paper does not); otherwise it is a glyph
/// (dropped).
pub const FIELD_MEAN_DISTANCE: f32 = 16.0;

// ---------------------------------------------------------------------------
// Colour
// ---------------------------------------------------------------------------

fn srgb_to_linear(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance.
pub fn luminance(c: [u8; 3]) -> f64 {
    0.2126 * srgb_to_linear(c[0]) + 0.7152 * srgb_to_linear(c[1]) + 0.0722 * srgb_to_linear(c[2])
}

/// WCAG contrast ratio of two colours, in `1.0..=21.0`.
pub fn contrast_ratio(a: [u8; 3], b: [u8; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `#RRGGBB`.
pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])
}

// ---------------------------------------------------------------------------
// The analysis image and the ring
// ---------------------------------------------------------------------------

/// An opaque RGB image (the analysis frame).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    pub width: u32,
    pub height: u32,
    /// Row-major RGB.
    pub rgb: Vec<u8>,
}

impl Grid {
    /// A uniform grid.
    pub fn filled(width: u32, height: u32, c: [u8; 3]) -> Grid {
        let mut rgb = Vec::with_capacity((width * height * 3) as usize);
        for _ in 0..width * height {
            rgb.extend_from_slice(&c);
        }
        Grid { width, height, rgb }
    }

    pub fn put(&mut self, x: u32, y: u32, c: [u8; 3]) {
        if x < self.width && y < self.height {
            let i = ((y * self.width + x) * 3) as usize;
            self.rgb[i..i + 3].copy_from_slice(&c);
        }
    }

    pub fn get(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 3) as usize;
        [self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]]
    }

    /// The pixmap box-filtered down by the integer factor `k` (1 = copy).
    pub fn from_pixmap(pm: &Pixmap, k: u32) -> Grid {
        let k = k.max(1);
        let (w, h) = ((pm.width() / k).max(1), (pm.height() / k).max(1));
        let src = pm.data();
        let (sw, sh) = (pm.width(), pm.height());
        let mut rgb = Vec::with_capacity((w * h * 3) as usize);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 3];
                let mut n = 0u32;
                for dy in 0..k {
                    for dx in 0..k {
                        let (sx, sy) = ((x * k + dx).min(sw - 1), (y * k + dy).min(sh - 1));
                        let i = ((sy * sw + sx) * 4) as usize;
                        for (s, v) in sum.iter_mut().zip(&src[i..i + 3]) {
                            *s += u32::from(*v);
                        }
                        n += 1;
                    }
                }
                for s in sum {
                    rgb.push(((s + n / 2) / n) as u8);
                }
            }
        }
        Grid {
            width: w,
            height: h,
            rgb,
        }
    }
}

/// What the ring around an ink box measured.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ground {
    /// Per-channel median of the ground pixels.
    pub color: [u8; 3],
    /// Share of the ring that is ink-coloured (glyphs of neighbouring text
    /// and fields of the ink's own colour).
    pub ink_share: f32,
    /// Share of the ring dropped as neighbouring glyphs.
    pub glyph_share: f32,
    /// Ring pixels inside the image.
    pub pixels: usize,
}

fn dist(a: [u8; 3], b: [u8; 3]) -> f32 {
    let d = |x: u8, y: u8| {
        let v = f32::from(x) - f32::from(y);
        v * v
    };
    (d(a[0], b[0]) + d(a[1], b[1]) + d(a[2], b[2])).sqrt()
}

fn median(mut v: Vec<u8>) -> u8 {
    v.sort_unstable();
    v[v.len() / 2]
}

/// The ground around `rect` (`[x0, y0, x1, y1]`, analysis pixels) for a text of
/// `ink` colour and `em_px` font size (analysis pixels): the median of the ring
/// `near..far` px outside it, without the glyph pixels of neighbouring text
/// (see the module docs). `None` when no ring pixel lies inside the image.
pub fn ring_ground(
    grid: &Grid,
    rect: [f32; 4],
    near: i32,
    far: i32,
    ink: [u8; 3],
    em_px: f32,
) -> Option<Ground> {
    let (w, h) = (grid.width as i32, grid.height as i32);
    let grow = |d: i32| {
        [
            rect[0].floor() as i32 - d,
            rect[1].floor() as i32 - d,
            rect[2].ceil() as i32 + d,
            rect[3].ceil() as i32 + d,
        ]
    };
    let (inner, outer) = (grow(near), grow(far));
    let m = ((GLYPH_WINDOW_EM * em_px).round() as i32).max(2);
    // The window region: the ring's bounds plus the window half-width.
    let reg = [
        (outer[0] - m).max(0),
        (outer[1] - m).max(0),
        (outer[2] + m).min(w),
        (outer[3] + m).min(h),
    ];
    if reg[0] >= reg[2] || reg[1] >= reg[3] {
        return None;
    }
    let (rw, rh) = ((reg[2] - reg[0]) as usize, (reg[3] - reg[1]) as usize);
    // Integral images of the three colour channels over the region.
    let stride = rw + 1;
    let mut sat = vec![[0u32; 3]; stride * (rh + 1)];
    for y in 0..rh {
        let mut row = [0u32; 3];
        for x in 0..rw {
            let p = grid.get((reg[0] + x as i32) as u32, (reg[1] + y as i32) as u32);
            for c in 0..3 {
                row[c] += u32::from(p[c]);
                sat[(y + 1) * stride + x + 1][c] = sat[y * stride + x + 1][c] + row[c];
            }
        }
    }
    // Mean colour of the (2m+1)-square window around (x, y), clipped to the
    // region.
    let window_mean = |x: i32, y: i32| -> [u8; 3] {
        let x0 = (x - m - reg[0]).max(0) as usize;
        let y0 = (y - m - reg[1]).max(0) as usize;
        let x1 = ((x + m + 1 - reg[0]) as usize).min(rw);
        let y1 = ((y + m + 1 - reg[1]) as usize).min(rh);
        let n = ((x1 - x0) * (y1 - y0)).max(1) as f32;
        let mut out = [0u8; 3];
        for c in 0..3 {
            let sum = sat[y1 * stride + x1][c] + sat[y0 * stride + x0][c]
                - sat[y0 * stride + x1][c]
                - sat[y1 * stride + x0][c];
            out[c] = (sum as f32 / n).round() as u8;
        }
        out
    };

    let mut all = 0usize;
    let mut inked = 0usize;
    let mut glyphs = 0usize;
    let mut pool: Vec<[u8; 3]> = Vec::new();
    let mut every: Vec<[u8; 3]> = Vec::new();
    for y in outer[1].max(0)..outer[3].min(h) {
        for x in outer[0].max(0)..outer[2].min(w) {
            if x >= inner[0] && x < inner[2] && y >= inner[1] && y < inner[3] {
                continue;
            }
            let p = grid.get(x as u32, y as u32);
            all += 1;
            every.push(p);
            if dist(p, ink) <= NEAR_INK_DISTANCE {
                inked += 1;
                if dist(window_mean(x, y), ink) > FIELD_MEAN_DISTANCE {
                    glyphs += 1;
                    continue;
                }
            }
            pool.push(p);
        }
    }
    if all == 0 {
        return None;
    }
    // Nothing but glyphs around the text: judge against everything.
    let pool = if pool.is_empty() { &every } else { &pool };
    let channel = |c: usize| median(pool.iter().map(|p| p[c]).collect());
    Some(Ground {
        color: [channel(0), channel(1), channel(2)],
        ink_share: inked as f32 / all as f32,
        glyph_share: glyphs as f32 / all as f32,
        pixels: all,
    })
}

/// The WCAG ratio a text of `size_px` (at `u` = short side / 1080) needs.
pub fn needed_ratio(size_px: f32, u: f32) -> f64 {
    if size_px >= DISPLAY_TEXT_PX * u {
        TEXT_CONTRAST_DISPLAY
    } else {
        TEXT_CONTRAST_BODY
    }
}

// ---------------------------------------------------------------------------
// The check
// ---------------------------------------------------------------------------

/// One text layer below its limit.
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastFinding {
    pub scene: String,
    pub layer: String,
    /// The text as shown at READ (at most 24 characters).
    pub text: String,
    pub ratio: f64,
    pub need: f64,
    pub ink: [u8; 3],
    pub ground: [u8; 3],
    /// Drawn font size (px on the project canvas).
    pub size_px: f32,
    /// Scene-local READ time the frame was sampled at.
    pub read_s: f64,
    /// Share of the ring that is ink-coloured.
    pub ink_share: f32,
    /// Set when the layer as a whole clears its limit but one of its words
    /// does not (a word of a longer line over a field): `text` is that word
    /// and `of` the layer's text.
    pub of: Option<String>,
}

impl ContrastFinding {
    pub fn detail(&self) -> String {
        let word = self
            .of
            .as_ref()
            .map(|of| format!(" (word of \"{of}\")"))
            .unwrap_or_default();
        format!(
            "\"{}\"{word}: text {} on ground {} = {:.2}:1 < {:.1}:1 ({:.0}px at READ {:.2}s)",
            self.text,
            hex(self.ink),
            hex(self.ground),
            self.ratio,
            self.need,
            self.size_px,
            self.read_s,
        )
    }
}

/// A text layer the check could not measure.
#[derive(Debug, Clone, PartialEq)]
pub struct ContrastSkip {
    pub scene: String,
    pub layer: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ContrastReport {
    pub findings: Vec<ContrastFinding>,
    /// Every text layer measured, passing or not (what the check saw).
    pub measured: Vec<ContrastFinding>,
    /// Text layers measured.
    pub checked: usize,
    pub skipped: Vec<ContrastSkip>,
    /// Worst (lowest) margin over its limit across all measured layers:
    /// `ratio - need`.
    pub min_margin: Option<f64>,
}

impl ContrastReport {
    /// The `qa --speech` entry: one check listing the failing layers.
    pub fn check(&self) -> SpeechQaCheck {
        let (status, detail) = if self.checked == 0 && self.findings.is_empty() {
            (
                CheckStatus::Skip,
                format!(
                    "no readable text layer to measure ({} skipped)",
                    self.skipped.len()
                ),
            )
        } else if self.findings.is_empty() {
            (
                CheckStatus::Pass,
                format!(
                    "{} text layer(s) at READ clear their limit (tightest margin {:+.2}); {} skipped",
                    self.checked,
                    self.min_margin.unwrap_or(0.0),
                    self.skipped.len()
                ),
            )
        } else {
            let lines: Vec<String> = self
                .findings
                .iter()
                .map(|f| format!("{} {} {}", f.scene, f.layer, f.detail()))
                .collect();
            (CheckStatus::Fail, lines.join("; "))
        };
        SpeechQaCheck {
            name: TEXT_LOCAL_CONTRAST.to_string(),
            status,
            detail,
        }
    }

    /// The `qa --layout` findings.
    pub fn layout_findings(&self) -> Vec<LayoutFinding> {
        self.findings
            .iter()
            .map(|f| LayoutFinding {
                scene: f.scene.clone(),
                layer: f.layer.clone(),
                check: LayoutCheck::TextLocalContrast,
                detail: f.detail(),
            })
            .collect()
    }
}

/// The text a text layer shows this frame (a counter's own text included).
fn shown_text(layer: &ResolvedLayer<'_>) -> Option<String> {
    match layer.kind {
        LayerKind::Text(style) => Some(layer.text.clone().unwrap_or_else(|| style.text.clone())),
        _ => None,
    }
}

/// Canvas-space bounds `[x0, y0, x1, y1]` of the layer-local rectangle `b`,
/// through the layer's transform (or perspective homography).
fn to_canvas(layer: &ResolvedLayer<'_>, b: [f32; 4]) -> Option<[f32; 4]> {
    let mut out = [
        f32::INFINITY,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::NEG_INFINITY,
    ];
    for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[2], b[3]), (b[0], b[3])] {
        let (cx, cy) = match &layer.projective {
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
            None => layer.transform.apply(x, y),
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

/// Canvas-space bounds of a text layer's ink (the outline of its glyphs).
fn ink_box(
    engine: &mut TextEngine,
    family: &str,
    layer: &ResolvedLayer<'_>,
    shown: &str,
) -> Option<[f32; 4]> {
    let LayerKind::Text(style) = layer.kind else {
        return None;
    };
    let mut style = style.clone();
    style.text = shown.to_string();
    let b = engine.outline(family, &style, layer.width)?.bounds();
    to_canvas(layer, [b.left(), b.top(), b.right(), b.bottom()])
}

/// Canvas-space ink bounds of each word of a text layer (shown text split on
/// whitespace; words without an outline are left out).
fn word_boxes(
    engine: &mut TextEngine,
    family: &str,
    layer: &ResolvedLayer<'_>,
    shown: &str,
) -> Vec<(String, [f32; 4])> {
    let LayerKind::Text(style) = layer.kind else {
        return Vec::new();
    };
    let mut style = style.clone();
    style.text = shown.to_string();
    let displayed = if style.uppercase {
        shown.to_uppercase()
    } else {
        shown.to_string()
    };
    // The word of each non-whitespace character, in reading order (the
    // numbering `glyph_outlines` uses).
    let mut words: Vec<String> = Vec::new();
    let mut word_of: Vec<usize> = Vec::new();
    let mut inside = false;
    for ch in displayed.chars() {
        if ch.is_whitespace() {
            inside = false;
            continue;
        }
        if !inside {
            words.push(String::new());
            inside = true;
        }
        word_of.push(words.len() - 1);
        if let Some(w) = words.last_mut() {
            w.push(ch);
        }
    }
    let mut bounds: Vec<Option<[f32; 4]>> = vec![None; words.len()];
    for g in engine.glyph_outlines(family, &style, layer.width) {
        let Some(&w) = word_of.get(g.index) else {
            continue;
        };
        let b = g.path.bounds();
        let r = [b.left(), b.top(), b.right(), b.bottom()];
        bounds[w] = Some(match bounds[w] {
            None => r,
            Some(u) => [
                u[0].min(r[0]),
                u[1].min(r[1]),
                u[2].max(r[2]),
                u[3].max(r[3]),
            ],
        });
    }
    words
        .into_iter()
        .zip(bounds)
        .filter_map(|(w, b)| Some((w, to_canvas(layer, b?)?)))
        .collect()
}

/// Text colour as drawn: the layer colour moved toward its tint, if any.
fn drawn_color(layer: &ResolvedLayer<'_>, c: Color) -> [u8; 3] {
    let mut rgb = [c.r, c.g, c.b];
    if let Some(t) = layer.tint {
        let a = t.amount.clamp(0.0, 1.0);
        let to = [t.color.r, t.color.g, t.color.b];
        for (v, to) in rgb.iter_mut().zip(to) {
            *v = (f32::from(*v) + (f32::from(to) - f32::from(*v)) * a).round() as u8;
        }
    }
    rgb
}

/// Frame nearest `t` seconds, within the project.
fn frame_at(project: &MotionProject, t: f64) -> u32 {
    let last = project.frame_count().saturating_sub(1);
    ((t * f64::from(project.canvas.fps)).round() as u32).min(last)
}

/// Run `text_local_contrast` over every beat scene of `project`. `base_dir` is
/// the directory the asset paths resolve against (the motion file's).
pub fn text_local_contrast(
    project: &MotionProject,
    base_dir: &Path,
) -> Result<ContrastReport, RenderError> {
    let mut report = ContrastReport::default();
    let beats = story_beats(project);
    if beats.is_empty() {
        return Ok(report);
    }
    let renderer = CpuRenderer::new(project, base_dir)?;
    let root = base_dir.join(project.asset_root.as_deref().unwrap_or(""));
    let mut engine = TextEngine::new();
    let mut family_of: BTreeMap<String, String> = BTreeMap::new();
    for a in project.assets.iter().filter(|a| a.kind == AssetKind::Font) {
        family_of.insert(a.id.clone(), engine.load(&root.join(&a.path))?);
    }
    let limits = Limits::for_canvas(project.canvas.width, project.canvas.height);
    let u = project.canvas.width.min(project.canvas.height) as f32 / 1080.0;
    let k = project.canvas.width.div_ceil(MAX_WIDTH_PX).max(1);

    for scene in beats {
        let read = scene
            .lifecycle
            .map_or(scene.duration_seconds * 0.5, |l| l.read);
        let n = frame_at(project, scene.start_seconds + read);
        let resolved = evaluate_frame(project, n)?;
        // The text layers to judge, before any pixel is drawn.
        let mut jobs = Vec::new();
        for leaf in readable_leaves(&resolved, &limits) {
            if leaf.scene != Some(scene.id.as_str()) {
                continue;
            }
            let Some(layer) = find_layer(&resolved.layers, &leaf.chain) else {
                continue;
            };
            let LayerKind::Text(style) = layer.kind else {
                continue;
            };
            if !is_content_chain(&leaf.chain, layer.kind) || revealed_share(layer) < ON_SCREEN_SHARE
            {
                continue;
            }
            let Some(shown) = shown_text(layer).filter(|t| !t.trim().is_empty()) else {
                continue;
            };
            jobs.push((layer, style, shown));
        }
        if jobs.is_empty() {
            continue;
        }
        let pm = renderer.render(&resolved)?;
        let grid = Grid::from_pixmap(&pm, k);
        for (layer, style, shown) in jobs {
            let skip = |reason: &str| ContrastSkip {
                scene: scene.id.clone(),
                layer: layer.id.to_string(),
                reason: reason.to_string(),
            };
            let family = project
                .theme
                .fonts
                .get(&style.font_role)
                .and_then(|asset| family_of.get(asset));
            let Some(family) = family else {
                report.skipped.push(skip("no font for the layer's role"));
                continue;
            };
            let Some(ink_rect) = ink_box(&mut engine, family, layer, &shown) else {
                report.skipped.push(skip("ink box not measurable"));
                continue;
            };
            let rect = ink_rect.map(|v| v / k as f32);
            let ink = drawn_color(layer, style.color);
            let t = layer.transform;
            let scale = (t.a * t.d - t.b * t.c).abs().sqrt();
            let size_px = style.font_size * scale;
            let em = size_px / k as f32;
            let Some(ground) = ring_ground(&grid, rect, RING_NEAR_PX, RING_FAR_PX, ink, em) else {
                report.skipped.push(skip("ring outside the canvas"));
                continue;
            };
            let need = needed_ratio(size_px, u);
            let ratio = contrast_ratio(ink, ground.color);
            report.checked += 1;
            let margin = ratio - need;
            if report.min_margin.is_none_or(|m| margin < m) {
                report.min_margin = Some(margin);
            }
            let finding = ContrastFinding {
                scene: scene.id.clone(),
                layer: layer.id.to_string(),
                text: shown.chars().take(24).collect(),
                ratio,
                need,
                ink,
                ground: ground.color,
                size_px,
                read_s: read,
                ink_share: ground.ink_share,
                of: None,
            };
            let layer_failed = ratio < need;
            if layer_failed {
                report.findings.push(finding.clone());
            }
            report.measured.push(finding);

            // A line of several words judged as one passes while its first
            // word hides under a field: judge each word on its own ring when
            // the layer as a whole is clear.
            if layer_failed {
                continue;
            }
            let words = word_boxes(&mut engine, family, layer, &shown);
            if words.len() < 2 {
                continue;
            }
            let mut worst: Option<(f64, String, Ground)> = None;
            for (word, wb) in words {
                let wr = wb.map(|v| v / k as f32);
                let Some(g) = ring_ground(&grid, wr, RING_NEAR_PX, RING_FAR_PX, ink, em) else {
                    continue;
                };
                let r = contrast_ratio(ink, g.color);
                if worst.as_ref().is_none_or(|w| r < w.0) {
                    worst = Some((r, word, g));
                }
            }
            if let Some((r, word, g)) = worst {
                if r - need < report.min_margin.unwrap_or(f64::INFINITY) {
                    report.min_margin = Some(r - need);
                }
                if r < need {
                    report.findings.push(ContrastFinding {
                        scene: scene.id.clone(),
                        layer: layer.id.to_string(),
                        text: word.chars().take(24).collect(),
                        ratio: r,
                        need,
                        ink,
                        ground: g.color,
                        size_px,
                        read_s: read,
                        ink_share: g.ink_share,
                        of: Some(shown.chars().take(24).collect()),
                    });
                }
            }
        }
    }
    Ok(report)
}
