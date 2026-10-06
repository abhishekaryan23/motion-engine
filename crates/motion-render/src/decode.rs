//! Image decoding for ingestion and rendering (0.5): PNG and JPEG → premultiplied
//! RGBA `Pixmap`. The format is sniffed from the file's magic bytes (the
//! extension is only a hint). JPEG decodes through `zune-jpeg` (already in the
//! dependency tree via resvg); JPEG images are always opaque.

use std::path::Path;

use motion_core::assets::ImageFormat;
use resvg::tiny_skia::Pixmap;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

/// A decoded raster.
pub struct DecodedImage {
    pub pixmap: Pixmap,
    pub format: ImageFormat,
    /// At least one pixel is not fully opaque.
    pub has_transparency: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    #[error("cannot read {path}: {msg}")]
    Io { path: String, msg: String },
    #[error("unsupported image format (expected PNG or JPEG)")]
    Unsupported,
    #[error("corrupt {format:?} data: {msg}")]
    Corrupt { format: ImageFormat, msg: String },
    #[error("image has zero width or height")]
    Empty,
}

/// Decode a PNG or JPEG file.
pub fn decode_file(path: &Path) -> Result<DecodedImage, DecodeError> {
    let bytes = std::fs::read(path).map_err(|e| DecodeError::Io {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;
    decode_bytes(&bytes)
}

/// Decode PNG or JPEG bytes (format sniffed from magic bytes).
pub fn decode_bytes(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    if bytes.starts_with(&PNG_MAGIC) {
        decode_png(bytes)
    } else if bytes.starts_with(&JPEG_MAGIC) {
        decode_jpeg(bytes)
    } else {
        Err(DecodeError::Unsupported)
    }
}

const PNG_MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const JPEG_MAGIC: [u8; 3] = [0xff, 0xd8, 0xff];

fn decode_png(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    let pixmap = Pixmap::decode_png(bytes).map_err(|e| DecodeError::Corrupt {
        format: ImageFormat::Png,
        msg: e.to_string(),
    })?;
    if pixmap.width() == 0 || pixmap.height() == 0 {
        return Err(DecodeError::Empty);
    }
    let has_transparency = pixmap.pixels().iter().any(|p| p.alpha() < 255);
    Ok(DecodedImage {
        pixmap,
        format: ImageFormat::Png,
        has_transparency,
    })
}

fn corrupt_jpeg(msg: impl ToString) -> DecodeError {
    DecodeError::Corrupt {
        format: ImageFormat::Jpeg,
        msg: msg.to_string(),
    }
}

fn decode_jpeg(bytes: &[u8]) -> Result<DecodedImage, DecodeError> {
    // The decoder is third-party code fed untrusted files: a panic on a
    // malformed stream must surface as `Corrupt`, never abort ingestion.
    let raw = std::panic::catch_unwind(|| {
        let options = DecoderOptions::default().jpeg_set_out_colorspace(ColorSpace::RGB);
        let mut decoder = JpegDecoder::new_with_options(ZCursor::new(bytes), options);
        decoder.decode_headers().map_err(corrupt_jpeg)?;
        let info = decoder
            .info()
            .ok_or_else(|| corrupt_jpeg("missing image header"))?;
        let (w, h) = (u32::from(info.width), u32::from(info.height));
        if w == 0 || h == 0 {
            return Err(DecodeError::Empty);
        }
        let size = decoder
            .output_buffer_size()
            .ok_or_else(|| corrupt_jpeg("image dimensions overflow"))?;
        let mut rgb = vec![0u8; size];
        decoder.decode_into(&mut rgb).map_err(corrupt_jpeg)?;
        Ok((w, h, rgb))
    })
    .map_err(|_| corrupt_jpeg("decoder failed on malformed data"))??;
    let (w, h, rgb) = raw;
    let mut pixmap = Pixmap::new(w, h).ok_or(DecodeError::Empty)?;
    if rgb.len() < (w as usize) * (h as usize) * 3 {
        return Err(corrupt_jpeg("truncated pixel data"));
    }
    for (dst, src) in pixmap
        .data_mut()
        .chunks_exact_mut(4)
        .zip(rgb.chunks_exact(3))
    {
        dst.copy_from_slice(&[src[0], src[1], src[2], 255]);
    }
    Ok(DecodedImage {
        pixmap,
        format: ImageFormat::Jpeg,
        has_transparency: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_unknown() {
        assert!(matches!(decode_bytes(&[]), Err(DecodeError::Unsupported)));
        assert!(matches!(
            decode_bytes(b"GIF89a....."),
            Err(DecodeError::Unsupported)
        ));
    }
}
