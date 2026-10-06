//! sRGB → OKLab (Björn Ottosson, 2020). Shared by the color evidence
//! (`motion-render::reference::color`) and the TasteDirector's accent choice
//! so both speak about hue in the same space.

/// sRGB 8-bit → linear-light component.
fn linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// OKLab `(L, a, b)`; `L` in `0..=1`.
pub fn oklab(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
    let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
    let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;
    let (l, m, s) = (l.cbrt(), m.cbrt(), s.cbrt());
    (
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    )
}

/// OKLCh `(lightness, chroma, hue degrees in 0..360)`.
pub fn lch(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (l, a, b) = oklab(r, g, b);
    let c = (a * a + b * b).sqrt();
    let h = b.atan2(a).to_degrees().rem_euclid(360.0);
    (l, c, h)
}

/// Hue (degrees) of a `0xRRGGBB` color.
pub fn hue_of(rgb: u32) -> f32 {
    lch((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8).2
}

/// Parse `#RRGGBB` (case-insensitive) into `0xRRGGBB`.
pub fn parse_hex(hex: &str) -> Option<u32> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(h, 16).ok()
}

/// `0xRRGGBB` → `#RRGGBB`.
pub fn to_hex(rgb: u32) -> String {
    format!("#{:06X}", rgb & 0xFF_FFFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_and_black_have_expected_lightness() {
        assert!((oklab(255, 255, 255).0 - 1.0).abs() < 1e-3);
        assert!(oklab(0, 0, 0).0.abs() < 1e-6);
        assert!(lch(128, 128, 128).1 < 1e-3);
    }

    #[test]
    fn primary_hues_are_ordered() {
        let red = hue_of(0xFF0000);
        let green = hue_of(0x00FF00);
        let blue = hue_of(0x0000FF);
        assert!(red < green && green < blue, "{red} {green} {blue}");
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#3cc8f0"), Some(0x3CC8F0));
        assert_eq!(to_hex(0x3CC8F0), "#3CC8F0");
        assert_eq!(parse_hex("3CC8F0"), None);
        assert_eq!(parse_hex("#3CC8F"), None);
    }
}
