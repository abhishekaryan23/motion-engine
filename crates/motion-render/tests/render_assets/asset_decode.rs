//! PNG + JPEG decoding (0.5): magic-byte sniffing, alpha detection, error variants.

use std::path::PathBuf;

use motion_core::assets::ImageFormat;
use motion_render::decode::{decode_bytes, decode_file, DecodeError};
use resvg::tiny_skia::{Color, Pixmap};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn png_bytes(w: u32, h: u32, transparent_pixel: bool) -> Vec<u8> {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    pm.fill(Color::from_rgba8(200, 30, 30, 255));
    if transparent_pixel {
        pm.pixels_mut()[0] = resvg::tiny_skia::PremultipliedColorU8::TRANSPARENT;
    }
    pm.encode_png().expect("encode")
}

#[test]
fn png_with_alpha_reports_transparency() {
    let img = decode_bytes(&png_bytes(16, 12, true)).expect("decode");
    assert_eq!(img.format, ImageFormat::Png);
    assert!(img.has_transparency);
    assert_eq!((img.pixmap.width(), img.pixmap.height()), (16, 12));
}

#[test]
fn opaque_png_is_opaque() {
    let img = decode_bytes(&png_bytes(8, 8, false)).expect("decode");
    assert_eq!(img.format, ImageFormat::Png);
    assert!(!img.has_transparency);
}

#[test]
fn jpeg_decodes_opaque_with_right_size_and_colors() {
    let img = decode_file(&fixture("gradient_640x800.jpg")).expect("decode jpeg");
    assert_eq!(img.format, ImageFormat::Jpeg);
    assert!(!img.has_transparency);
    assert_eq!((img.pixmap.width(), img.pixmap.height()), (640, 800));
    assert!(img.pixmap.pixels().iter().all(|p| p.alpha() == 255));
    // Gradient: red grows with x, green with y, blue is constant 128.
    let px = img.pixmap.pixels();
    let at = |x: usize, y: usize| px[y * 640 + x];
    let (tl, br) = (at(2, 2), at(637, 797));
    assert!(tl.red() < 20 && tl.green() < 20, "top-left {tl:?}");
    assert!(br.red() > 235 && br.green() > 235, "bottom-right {br:?}");
    assert!((i32::from(tl.blue()) - 128).abs() < 12);
}

#[test]
fn corrupt_png_is_corrupt() {
    let mut bytes = png_bytes(16, 16, false);
    bytes.truncate(40);
    match decode_bytes(&bytes) {
        Err(DecodeError::Corrupt { format, .. }) => assert_eq!(format, ImageFormat::Png),
        other => panic!("expected Corrupt(Png), got {:?}", other.map(|_| "image")),
    }
}

#[test]
fn corrupt_jpeg_is_corrupt() {
    let jpg = std::fs::read(fixture("gradient_640x800.jpg")).expect("fixture");
    for cut in [3usize, 20, 60] {
        match decode_bytes(&jpg[..cut]) {
            Err(DecodeError::Corrupt { format, .. }) => assert_eq!(format, ImageFormat::Jpeg),
            other => panic!(
                "cut {cut}: expected Corrupt(Jpeg), got {:?}",
                other.map(|_| "image")
            ),
        }
    }
    let mut garbage = vec![0xff, 0xd8, 0xff];
    garbage.extend((0..200u32).map(|i| (i * 37 % 251) as u8));
    assert!(matches!(
        decode_bytes(&garbage),
        Err(DecodeError::Corrupt { .. })
    ));
}

#[test]
fn unknown_bytes_are_unsupported() {
    let random: Vec<u8> = (0..512u32).map(|i| (i * 131 % 253) as u8).collect();
    assert!(matches!(
        decode_bytes(&random),
        Err(DecodeError::Unsupported)
    ));
    assert!(matches!(
        decode_bytes(b"GIF89a\x01\x00\x01\x00"),
        Err(DecodeError::Unsupported)
    ));
}

#[test]
fn zero_length_is_an_error() {
    assert!(decode_bytes(&[]).is_err());
}

#[test]
fn missing_file_is_io_error() {
    match decode_file(&fixture("does_not_exist.png")) {
        Err(DecodeError::Io { .. }) => {}
        other => panic!("expected Io, got {:?}", other.map(|_| "image")),
    }
}
