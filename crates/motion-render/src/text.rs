//! Typography backend: cosmic-text shaping -> glyph outlines -> tiny-skia paths.
//!
//! Text is converted to vector paths once (in layer-local pixels), so it stays
//! crisp under any animated scale/rotation and is never re-shaped per frame.
//! Only fonts registered from the project's assets are visible (no system
//! fonts), which keeps output identical across machines.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cosmic_text::{
    fontdb, Attrs, Buffer, CacheKeyFlags, Command, Family, FontSystem, Metrics, Shaping, Style,
    SwashCache, Weight,
};
use motion_core::compiler::TextMeasure;
use motion_core::scene::{TextAlign, TextStyle};
use resvg::tiny_skia::{Path as SkPath, PathBuilder};

use crate::RenderError;

pub struct TextEngine {
    fonts: FontSystem,
    swash: SwashCache,
    /// Font file -> family name.
    families: HashMap<PathBuf, String>,
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEngine {
    pub fn new() -> Self {
        TextEngine {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), fontdb::Database::new()),
            swash: SwashCache::new(),
            families: HashMap::new(),
        }
    }

    /// Load a font file (once) and return its family name.
    pub fn load(&mut self, path: &Path) -> Result<String, RenderError> {
        if let Some(f) = self.families.get(path) {
            return Ok(f.clone());
        }
        let data = std::fs::read(path)
            .map_err(|e| RenderError::Asset(format!("font {}: {e}", path.display())))?;
        let ids = self
            .fonts
            .db_mut()
            .load_font_source(fontdb::Source::Binary(Arc::new(data)));
        let family = ids
            .first()
            .and_then(|id| self.fonts.db().face(*id))
            .and_then(|face| face.families.first())
            .map(|(name, _)| name.clone())
            .ok_or_else(|| {
                RenderError::Asset(format!("font {}: no usable face", path.display()))
            })?;
        self.families.insert(path.to_path_buf(), family.clone());
        Ok(family)
    }

    fn buffer(&mut self, family: &str, style: &TextStyle, text: &str) -> Buffer {
        let size = style.font_size.max(0.5);
        let mut buf = Buffer::new(
            &mut self.fonts,
            Metrics::new(size, size * style.line_height),
        );
        buf.set_size(style.max_width, None);
        let attrs = Attrs::new()
            .family(Family::Name(family))
            .weight(Weight(style.font_weight))
            .style(if style.italic {
                Style::Italic
            } else {
                Style::Normal
            })
            .cache_key_flags(CacheKeyFlags::DISABLE_HINTING);
        buf.set_text(text, &attrs, Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.fonts, false);
        buf
    }

    /// Width of the widest line, including letter spacing.
    pub fn measure(&mut self, family: &str, style: &TextStyle) -> f32 {
        let text = display_text(style);
        let buf = self.buffer(family, style, &text);
        let ls = style.letter_spacing * style.font_size;
        buf.layout_runs()
            .map(|run| run.line_w + ls * run.glyphs.len().saturating_sub(1) as f32)
            .fold(0.0, f32::max)
    }

    /// Outline path of the text laid out in a box of `box_width`, in local pixels
    /// (top of first line at y = 0). `None` for empty / whitespace-only text.
    pub fn outline(&mut self, family: &str, style: &TextStyle, box_width: f32) -> Option<SkPath> {
        let text = display_text(style);
        let buf = self.buffer(family, style, &text);
        let ls = style.letter_spacing * style.font_size;
        let mut pb = PathBuilder::new();
        for run in buf.layout_runs() {
            let n = run.glyphs.len();
            let line_w = run.line_w + ls * n.saturating_sub(1) as f32;
            let ox = line_offset(style, box_width, line_w);
            for (i, g) in run.glyphs.iter().enumerate() {
                let physical = g.physical((0.0, 0.0), 1.0);
                let Some(cmds) = self
                    .swash
                    .get_outline_commands(&mut self.fonts, physical.cache_key)
                else {
                    continue;
                };
                let gx = g.x + g.font_size * g.x_offset + ox + ls * i as f32;
                let gy = run.line_y + g.y - g.font_size * g.y_offset;
                push_glyph(&mut pb, cmds, gx, gy);
            }
        }
        pb.finish()
    }

    /// Per-glyph outlines of the same layout as [`outline`](Self::outline)
    /// (identical positions), for animating glyphs individually.
    ///
    /// Each [`GlyphOutline::index`] is the 0-based position of the glyph's
    /// first character among the displayed text's non-whitespace characters in
    /// reading order, so it lines up with `ResolvedLayer::glyphs` even when a
    /// ligature or cluster covers several characters. Glyphs without an
    /// outline (spaces) are omitted.
    pub fn glyph_outlines(
        &mut self,
        family: &str,
        style: &TextStyle,
        box_width: f32,
    ) -> Vec<GlyphOutline> {
        let text = display_text(style);
        let buf = self.buffer(family, style, &text);
        let ls = style.letter_spacing * style.font_size;
        // Non-whitespace characters before each source line.
        let mut line_base = Vec::with_capacity(buf.lines.len());
        let mut seen = 0usize;
        for line in &buf.lines {
            line_base.push(seen);
            seen += non_ws(line.text());
        }
        let mut out = Vec::new();
        for run in buf.layout_runs() {
            let n = run.glyphs.len();
            let line_w = run.line_w + ls * n.saturating_sub(1) as f32;
            let ox = line_offset(style, box_width, line_w);
            let base = line_base.get(run.line_i).copied().unwrap_or(0);
            for (i, g) in run.glyphs.iter().enumerate() {
                let cluster = run.text.get(g.start..g.end).unwrap_or("");
                if cluster.chars().all(char::is_whitespace) {
                    continue;
                }
                let index = base + non_ws(run.text.get(..g.start).unwrap_or(""));
                let physical = g.physical((0.0, 0.0), 1.0);
                let Some(cmds) = self
                    .swash
                    .get_outline_commands(&mut self.fonts, physical.cache_key)
                else {
                    continue;
                };
                let gx = g.x + g.font_size * g.x_offset + ox + ls * i as f32;
                let gy = run.line_y + g.y - g.font_size * g.y_offset;
                let mut pb = PathBuilder::new();
                push_glyph(&mut pb, cmds, gx, gy);
                if let Some(path) = pb.finish() {
                    let b = path.bounds();
                    let center = ((b.left() + b.right()) * 0.5, (b.top() + b.bottom()) * 0.5);
                    out.push(GlyphOutline {
                        index,
                        path,
                        center,
                    });
                }
            }
        }
        out
    }
}

/// One glyph's outline in layer-local pixels, ready to be posed.
#[derive(Debug, Clone)]
pub struct GlyphOutline {
    /// Position among the non-whitespace characters (see
    /// [`TextEngine::glyph_outlines`]).
    pub index: usize,
    pub path: SkPath,
    /// Centre of the outline's bounds (the pivot for scale and rotation).
    pub center: (f32, f32),
}

fn non_ws(s: &str) -> usize {
    s.chars().filter(|c| !c.is_whitespace()).count()
}

fn line_offset(style: &TextStyle, box_width: f32, line_w: f32) -> f32 {
    match style.align {
        TextAlign::Left => 0.0,
        TextAlign::Center => (box_width - line_w) / 2.0,
        TextAlign::Right => box_width - line_w,
    }
}

/// Append one glyph's outline commands at `(gx, gy)` (y flipped to screen).
fn push_glyph(pb: &mut PathBuilder, cmds: &[Command], gx: f32, gy: f32) {
    for cmd in cmds {
        match *cmd {
            Command::MoveTo(p) => pb.move_to(gx + p.x, gy - p.y),
            Command::LineTo(p) => pb.line_to(gx + p.x, gy - p.y),
            Command::QuadTo(c, p) => pb.quad_to(gx + c.x, gy - c.y, gx + p.x, gy - p.y),
            Command::CurveTo(c1, c2, p) => pb.cubic_to(
                gx + c1.x,
                gy - c1.y,
                gx + c2.x,
                gy - c2.y,
                gx + p.x,
                gy - p.y,
            ),
            Command::Close => pb.close(),
        }
    }
}

fn display_text(style: &TextStyle) -> String {
    if style.uppercase {
        style.text.to_uppercase()
    } else {
        style.text.clone()
    }
}

/// Font-accurate [`TextMeasure`] for the compiler, sharing the render text engine.
#[derive(Default)]
pub struct FontMeasure {
    engine: RefCell<TextEngine>,
}

impl FontMeasure {
    pub fn new() -> Self {
        Self::default()
    }

    /// Preload fonts in order so glyph fallback matches the renderer, which
    /// loads the project's font assets in the same (asset id) order.
    pub fn with_fonts<'p>(paths: impl IntoIterator<Item = &'p Path>) -> Result<Self, RenderError> {
        let m = Self::default();
        for p in paths {
            m.engine.borrow_mut().load(p)?;
        }
        Ok(m)
    }
}

impl TextMeasure for FontMeasure {
    fn line_width(
        &self,
        font_path: &Path,
        weight: u16,
        italic: bool,
        text: &str,
        font_size: f32,
        ls: f32,
    ) -> f32 {
        let mut engine = self.engine.borrow_mut();
        let Ok(family) = engine.load(font_path) else {
            // Unknown font: fall back to the compiler's estimate.
            return motion_core::ApproxMeasure
                .line_width(font_path, weight, italic, text, font_size, ls);
        };
        let style = TextStyle {
            text: text.to_string(),
            font_role: motion_core::scene::FontRole::Body,
            font_size,
            font_weight: weight,
            italic,
            color: motion_core::scene::Color::rgb(0, 0, 0),
            align: TextAlign::Left,
            line_height: 1.0,
            letter_spacing: ls,
            max_width: None,
            uppercase: false,
            ink: None,
        };
        engine.measure(&family, &style)
    }

    fn ink_bounds(
        &self,
        font_path: &Path,
        weight: u16,
        italic: bool,
        text: &str,
        font_size: f32,
        line_height: f32,
        ls: f32,
    ) -> Option<(f32, f32)> {
        let mut engine = self.engine.borrow_mut();
        let family = engine.load(font_path).ok()?;
        let style = TextStyle {
            text: text.to_string(),
            font_role: motion_core::scene::FontRole::Body,
            font_size,
            font_weight: weight,
            italic,
            color: motion_core::scene::Color::rgb(0, 0, 0),
            align: TextAlign::Left,
            line_height,
            letter_spacing: ls,
            max_width: None,
            uppercase: false,
            ink: None,
        };
        let path = engine.outline(&family, &style, 0.0)?;
        let b = path.bounds();
        Some((b.top(), b.bottom()))
    }
}
