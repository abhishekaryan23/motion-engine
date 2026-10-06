//! Contact sheet: every sample with its id, timestamp and kinds.
//!
//! Layout: a grid of thumbnails (6 columns of 180x320 cells for vertical and
//! square references, 4 columns of 320x180 for landscape), 8 px gutters, dark
//! grey background. Under each thumbnail a 26 px strip holds a kind-coloured
//! bar and the label `s03 4.20s scene_change` drawn with a built-in 5x7 bitmap
//! font (no fonts, no extra dependencies). Sheet size:
//! `width = cols * cell_w + (cols + 1) * 8`,
//! `height = rows * (cell_h + 26) + (rows + 1) * 8`, `rows = ceil(n / cols)`.

use std::path::Path;

use motion_core::reference::evidence::{Orientation, ReferenceSample, SampleKind};
use resvg::tiny_skia::{Color, FilterQuality, Paint, Pixmap, PixmapPaint, Rect, Transform};

use super::ReferenceError;
use crate::decode::decode_file;

const GUTTER: u32 = 8;
const LABEL_H: u32 = 26;
const BAR_H: u32 = 4;
const BACKGROUND: (u8, u8, u8) = (0x20, 0x20, 0x20);

fn image_err(msg: impl ToString) -> ReferenceError {
    ReferenceError::Image(msg.to_string())
}

fn kind_name(kind: SampleKind) -> &'static str {
    match kind {
        SampleKind::Periodic => "periodic",
        SampleKind::SceneChange => "scene_change",
        SampleKind::HighMotion => "high_motion",
        SampleKind::Stable => "stable",
    }
}

fn kind_color(kind: SampleKind) -> (u8, u8, u8) {
    match kind {
        SampleKind::Periodic => (0x9E, 0x9E, 0x9E),
        SampleKind::SceneChange => (0xFF, 0x8A, 0x00),
        SampleKind::HighMotion => (0xE5, 0x39, 0x35),
        SampleKind::Stable => (0x43, 0xA0, 0x47),
    }
}

/// 5x7 glyph rows; bit 4 is the leftmost column.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_lowercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1E, 0x01, 0x01, 0x0E, 0x01, 0x01, 0x1E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'a' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'b' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'c' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'd' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'e' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'f' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'g' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'h' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'i' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'j' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'k' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'l' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'm' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'n' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'o' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'p' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'r' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        's' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        't' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'u' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'v' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'w' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'x' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04],
        '_' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F],
        _ => [0; 7],
    }
}

fn paint(rgb: (u8, u8, u8)) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color(Color::from_rgba8(rgb.0, rgb.1, rgb.2, 255));
    p.anti_alias = false;
    p
}

fn fill_rect(pm: &mut Pixmap, x: u32, y: u32, w: u32, h: u32, rgb: (u8, u8, u8)) {
    if let Some(r) = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32) {
        pm.fill_rect(r, &paint(rgb), Transform::identity(), None);
    }
}

/// Draw `text` at (x, y) with the 5x7 font at integer `scale`; each dot is a filled rect.
fn draw_text(pm: &mut Pixmap, x: u32, y: u32, text: &str, scale: u32, rgb: (u8, u8, u8)) {
    let mut cx = x;
    for c in text.chars() {
        let rows = glyph(c);
        for (ry, bits) in rows.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (0x10 >> col) != 0 {
                    fill_rect(
                        pm,
                        cx + col * scale,
                        y + ry as u32 * scale,
                        scale,
                        scale,
                        rgb,
                    );
                }
            }
        }
        cx += 6 * scale;
    }
}

/// Width in pixels of `chars` glyphs at `scale` (5 columns + 1 spacing each).
fn text_width(chars: usize, scale: u32) -> u32 {
    (chars as u32 * 6 * scale).saturating_sub(scale)
}

fn label(sample: &ReferenceSample) -> String {
    let kind = sample
        .kinds
        .first()
        .copied()
        .map(kind_name)
        .unwrap_or("periodic");
    format!("{} {:.2}s {}", sample.id, sample.time_seconds, kind)
}

/// Write `out` (PNG) from the sample images under `bundle_dir`.
pub fn contact_sheet(
    bundle_dir: &Path,
    samples: &[ReferenceSample],
    out: &Path,
) -> Result<(), ReferenceError> {
    if samples.is_empty() {
        return Err(image_err("contact sheet needs at least one sample"));
    }
    let mut images = Vec::with_capacity(samples.len());
    for s in samples {
        let path = bundle_dir.join(&s.image);
        let img = decode_file(&path).map_err(|e| image_err(format!("{}: {e}", path.display())))?;
        images.push(img.pixmap);
    }

    let (cell_w, cell_h, cols) = match Orientation::from_size(images[0].width(), images[0].height())
    {
        Orientation::Landscape => (320u32, 180u32, 4u32),
        Orientation::Vertical | Orientation::Square => (180, 320, 6),
    };
    let rows = (samples.len() as u32).div_ceil(cols);
    let width = cols * cell_w + (cols + 1) * GUTTER;
    let height = rows * (cell_h + LABEL_H) + (rows + 1) * GUTTER;
    let mut sheet = Pixmap::new(width, height).ok_or_else(|| image_err("cannot allocate sheet"))?;
    sheet.fill(Color::from_rgba8(
        BACKGROUND.0,
        BACKGROUND.1,
        BACKGROUND.2,
        255,
    ));

    for (i, (sample, img)) in samples.iter().zip(&images).enumerate() {
        let col = i as u32 % cols;
        let row = i as u32 / cols;
        let x0 = GUTTER + col * (cell_w + GUTTER);
        let y0 = GUTTER + row * (cell_h + LABEL_H + GUTTER);

        // Thumbnail: fit inside the cell, preserving aspect, centred.
        let scale = (cell_w as f32 / img.width() as f32).min(cell_h as f32 / img.height() as f32);
        let tw = img.width() as f32 * scale;
        let th = img.height() as f32 * scale;
        let tx = x0 as f32 + (cell_w as f32 - tw) / 2.0;
        let ty = y0 as f32 + (cell_h as f32 - th) / 2.0;
        let pp = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        sheet.draw_pixmap(
            0,
            0,
            img.as_ref(),
            &pp,
            Transform::from_row(scale, 0.0, 0.0, scale, tx, ty),
            None,
        );

        // Label strip: kind bar on top, text below.
        let ly = y0 + cell_h;
        let first = sample
            .kinds
            .first()
            .copied()
            .unwrap_or(SampleKind::Periodic);
        fill_rect(&mut sheet, x0, ly, cell_w, BAR_H, kind_color(first));
        let text = label(sample);
        let text_scale = if text_width(text.chars().count(), 2) + 4 <= cell_w {
            2
        } else {
            1
        };
        draw_text(
            &mut sheet,
            x0 + 2,
            ly + BAR_H + 4,
            &text,
            text_scale,
            (255, 255, 255),
        );
    }

    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    sheet.save_png(out).map_err(image_err)
}
