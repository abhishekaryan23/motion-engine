//! Text measurement used by the compiler for layout decisions.
//!
//! The compiler never touches font files itself; it asks a [`TextMeasure`].
//! The renderer crate provides a font-accurate implementation (same shaping
//! engine as rendering), and [`ApproxMeasure`] is a deterministic fallback for
//! tests and tooling without fonts.

use std::path::Path;

/// Measures a single line of text.
pub trait TextMeasure {
    /// Advance width in pixels of `text` set on one line in the font file at
    /// `font_path` (weight / italic select a face within that file's family),
    /// including `letter_spacing_em * font_size` between glyphs.
    fn line_width(
        &self,
        font_path: &Path,
        weight: u16,
        italic: bool,
        text: &str,
        font_size: f32,
        letter_spacing_em: f32,
    ) -> f32;

    /// Vertical ink extent `(top, bottom)` in pixels from the top of the text
    /// box, for `text` (lines separated by `\n`) laid out exactly as the
    /// renderer lays out a Text layer with these parameters. `None` when the
    /// measure has no glyph outlines (layout then falls back to the box).
    #[allow(clippy::too_many_arguments)]
    fn ink_bounds(
        &self,
        _font_path: &Path,
        _weight: u16,
        _italic: bool,
        _text: &str,
        _font_size: f32,
        _line_height: f32,
        _letter_spacing_em: f32,
    ) -> Option<(f32, f32)> {
        None
    }
}

/// Font-free estimate: average advance per character by font file name.
#[derive(Debug, Clone, Copy, Default)]
pub struct ApproxMeasure;

impl TextMeasure for ApproxMeasure {
    fn line_width(
        &self,
        font_path: &Path,
        _weight: u16,
        _italic: bool,
        text: &str,
        font_size: f32,
        letter_spacing_em: f32,
    ) -> f32 {
        let name = font_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let em = if name.contains("anton") || name.contains("bebas") {
            0.46
        } else if name.contains("archivoblack") {
            0.78
        } else if name.contains("mono") {
            0.61
        } else if name.contains("serif") {
            0.47
        } else {
            0.55
        };
        let n = text.chars().count() as f32;
        n * em * font_size + (n - 1.0).max(0.0) * letter_spacing_em * font_size
    }
}
