//! The palette resolver (colour roles: `docs/DESIGN_RULES.md`).
//!
//! Value before hue: every pixel belongs to a role. A program names
//! swatches; this module turns them, per ground, into role colours in
//! OKLCH with fixed contrast bands:
//!
//! | role      | vs ground (WCAG)                         | source                          |
//! |-----------|------------------------------------------|---------------------------------|
//! | texture   | 1.08–1.35                                 | the ground's own hue, L ± 0.05–0.10 |
//! | structure | 1.5–3 decorative, ≥ 3 when it carries meaning | ink toward the ground        |
//! | ink       | ≥ 7 (type < 48 px), ≥ 4.5 (display)        | the extreme value opposite the ground |
//! | accent    | ≥ 3 (≥ 4.5 when it is type)                | the one accent hue              |
//!
//! Hard rules are repaired, not flagged: texture is never the ink colour
//! (ΔL ≥ 0.45), an accent that sinks into its ground moves in lightness or
//! becomes the opposite value, and a saturated ground that cannot carry
//! 4.5:1 type is moved in lightness. Pure and deterministic.

use motion_core::scene::Color;

use crate::program::Palette;

/// Texture contrast band against its ground.
pub const TEXTURE_MIN: f32 = 1.08;
pub const TEXTURE_MAX: f32 = 1.35;
/// Minimum OKLCH lightness gap between texture and ink.
pub const TEXTURE_INK_DL: f32 = 0.45;
/// Decorative structure (hairlines, guides, axes) band.
pub const STRUCTURE_DECOR: f32 = 2.0;
/// Structure that carries meaning (connectors, outlines, series).
pub const STRUCTURE_MEANING: f32 = 3.4;
/// Type contrast: small and display (≥ 48 px).
pub const INK_SMALL: f32 = 7.0;
pub const INK_DISPLAY: f32 = 4.5;
/// Display type starts here (px).
pub const DISPLAY_PX: f32 = 48.0;
/// Accent against its ground.
pub const ACCENT_MIN: f32 = 3.0;
/// At most this many hue families per video (plus neutrals).
pub const HUES_MAX: usize = 3;
/// Head-room the resolver keeps over every type band.
pub const MARGIN: f32 = 0.35;

/// The contrast a run of type needs at `font_size` px.
pub fn ink_need(font_size: f32) -> f32 {
    if font_size >= DISPLAY_PX {
        INK_DISPLAY
    } else {
        INK_SMALL
    }
}

/// The contrast a run of type needs on `ground`: a saturated ground (§2)
/// carries every size at the display band (no extreme reaches 7:1 on a
/// brand orange; the ground is kept, not greyed).
pub fn ink_need_on(font_size: f32, ground: Color) -> f32 {
    if family(ground) == Family::Saturated {
        INK_DISPLAY
    } else {
        ink_need(font_size)
    }
}

// ---------------------------------------------------------------- colour math

/// OKLCH: lightness 0..1, chroma, hue in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
}

fn to_linear(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.040_45 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(v: f32) -> f32 {
    if v <= 0.003_130_8 {
        v * 12.92
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    }
}

/// OKLab `[L, a, b]` of an sRGB colour (Ottosson 2020).
pub fn oklab(c: Color) -> [f32; 3] {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Linear sRGB of an OKLab colour (may be out of gamut).
fn oklab_to_linear(lab: [f32; 3]) -> [f32; 3] {
    let l = lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2];
    let m = lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2];
    let s = lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    [
        4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
        -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
        -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
    ]
}

pub fn oklch(c: Color) -> Oklch {
    let [l, a, b] = oklab(c);
    let h = b.atan2(a).to_degrees();
    Oklch {
        l,
        c: a.hypot(b),
        h: if h < 0.0 { h + 360.0 } else { h },
    }
}

/// The sRGB colour of `o`, chroma reduced until it is in gamut (lightness
/// and hue kept).
pub fn from_oklch(o: Oklch) -> Color {
    let lab = |c: f32| {
        let h = o.h.to_radians();
        [o.l.clamp(0.0, 1.0), c * h.cos(), c * h.sin()]
    };
    let inside = |rgb: [f32; 3]| rgb.iter().all(|v| (-1e-4..=1.0 + 1e-4).contains(v));
    let mut rgb = oklab_to_linear(lab(o.c));
    if !inside(rgb) {
        let (mut lo, mut hi) = (0.0f32, o.c);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if inside(oklab_to_linear(lab(mid))) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = oklab_to_linear(lab(lo));
    }
    let q = |v: f32| (from_linear(v.clamp(0.0, 1.0)) * 255.0).round() as u8;
    Color::rgb(q(rgb[0]), q(rgb[1]), q(rgb[2]))
}

/// WCAG 2 relative luminance.
pub fn luminance(c: Color) -> f32 {
    0.2126 * to_linear(c.r) + 0.7152 * to_linear(c.g) + 0.0722 * to_linear(c.b)
}

/// WCAG 2 contrast ratio.
pub fn contrast(a: Color, b: Color) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Shortest distance between two hues, degrees.
pub fn hue_gap(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// `a` → `b` in OKLab at `t`.
pub fn mix_oklab(a: Color, b: Color, t: f32) -> Color {
    let (x, y) = (oklab(a), oklab(b));
    let m = [
        x[0] + (y[0] - x[0]) * t,
        x[1] + (y[1] - x[1]) * t,
        x[2] + (y[2] - x[2]) * t,
    ];
    let o = Oklch {
        l: m[0],
        c: m[1].hypot(m[2]),
        h: m[2].atan2(m[1]).to_degrees().rem_euclid(360.0),
    };
    from_oklch(o)
}

/// The colour on the way from `ground` to `toward` (OKLab) whose contrast
/// with `ground` is closest to `target` (binary search; `toward` itself if
/// it cannot reach it).
pub fn toward_contrast(ground: Color, toward: Color, target: f32) -> Color {
    if contrast(ground, toward) <= target {
        return toward;
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if contrast(ground, mix_oklab(ground, toward, mid)) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    mix_oklab(ground, toward, hi)
}

// ---------------------------------------------------------------- families

/// Ground families (§2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// L ≤ 0.25 (and dim neutrals below the middle).
    Dark,
    /// L ≥ 0.90 (and pale neutrals above the middle).
    Light,
    /// A coloured mid-value ground (orange, red, blue).
    Saturated,
}

pub fn family(ground: Color) -> Family {
    let o = oklch(ground);
    if o.l <= 0.25 {
        Family::Dark
    } else if o.l >= 0.90 {
        Family::Light
    } else if o.c >= 0.06 {
        Family::Saturated
    } else if o.l < 0.6 {
        Family::Dark
    } else {
        Family::Light
    }
}

// ---------------------------------------------------------------- roles

/// Role colours derived for one ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Roles {
    pub ground: Color,
    pub family: Family,
    /// Halftone, dot grid, tiles: a shade/tint of the ground, 1.08–1.35:1.
    pub texture: Color,
    /// Hairlines, guides, axes: 1.5–3:1.
    pub structure: Color,
    /// Connectors, outlines, bars: ≥ 3:1.
    pub structure_strong: Color,
    /// Type and numbers: the extreme value opposite the ground (≥ 7:1).
    pub ink: Color,
    /// Quieter type, one step down the ladder (≥ 7:1 for small type).
    pub ink_secondary: Color,
    /// The one highlight (≥ 3:1).
    pub accent: Color,
    /// The accent when it is type (≥ 4.5:1; the ink if the hue can't).
    pub accent_text: Color,
    /// Panels and cards sitting on the ground (devices, code, cards): a
    /// step away from the ground (~1.25:1), same hue.
    pub surface: Color,
}

impl Roles {
    /// Type colour for `font_size` px, given the colour the author asked
    /// for: kept when it already meets the band, else the role that does
    /// (accent hue first, then ink).
    pub fn text(&self, asked: Color, font_size: f32) -> Color {
        // A small margin over the band edge: anti-aliasing and texture
        // eat a little contrast on screen.
        let need = ink_need_on(font_size, self.ground) + MARGIN;
        if contrast(asked, self.ground) >= need {
            return asked;
        }
        if (asked == self.accent || hue_gap(oklch(asked).h, oklch(self.accent).h) < 20.0)
            && contrast(self.accent_text, self.ground) >= need
        {
            return self.accent_text;
        }
        self.ink
    }

    /// The texture carried into another hue (a field's recolour or wave):
    /// same lightness as the texture, `hue_of`'s hue, still inside the
    /// texture band.
    pub fn texture_in(&self, hue_of: Color) -> Color {
        let (t, h) = (oklch(self.texture), oklch(hue_of));
        if h.c < 0.03 {
            return self.texture;
        }
        // A tint, not a new colour: chroma stays near the ground's, so a
        // recoloured pattern is still surface, not subject.
        let cap = (oklch(self.ground).c + 0.04).max(t.c);
        for k in 0..8 {
            let cand = from_oklch(Oklch {
                l: t.l,
                c: (h.c * (1.0 - 0.12 * k as f32)).clamp(t.c, cap),
                h: h.h,
            });
            let c = contrast(cand, self.ground);
            if (TEXTURE_MIN..=TEXTURE_MAX).contains(&c) {
                return cand;
            }
        }
        self.texture
    }

    /// The `k`-th tonal step from the ground toward the ink (0 = ground, 1 =
    /// ink), in OKLab: surfaces, code syntax and device chrome.
    pub fn tone(&self, k: f32) -> Color {
        mix_oklab(self.ground, self.ink, k.clamp(0.0, 1.0))
    }
}

/// The resolved palette: role sets for the main ground and every extra
/// ground, plus what the resolver had to change.
#[derive(Debug, Clone)]
pub struct Resolved {
    /// The program accent after repairs (one hue per video).
    pub accent: Color,
    /// The author's ink (kept where it meets the bands).
    pub ink: Color,
    pub main: Roles,
    pub grounds: Vec<Roles>,
    /// Human-readable repairs (`texture re-derived as ...`).
    pub repairs: Vec<String>,
}

fn hex(c: Color) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Near-black and off-white extremes that keep a trace of `hue`.
fn extremes(hue: f32) -> (Color, Color) {
    (
        from_oklch(Oklch {
            l: 0.14,
            c: 0.008,
            h: hue,
        }),
        from_oklch(Oklch {
            l: 0.965,
            c: 0.008,
            h: hue,
        }),
    )
}

/// A ground that can carry type: a ground where neither extreme reaches its
/// band is moved in lightness, same hue, away from the middle until one
/// does. Dark and light grounds carry small type (7:1); a saturated ground
/// (§2) is judged at the display band (4.5:1), so a brand orange stays
/// orange and small type on it is left to the `ink_contrast` gate.
fn fix_ground(ground: Color, repairs: &mut Vec<String>) -> Color {
    let o = oklch(ground);
    let (dark, light) = extremes(o.h);
    let best = contrast(ground, dark).max(contrast(ground, light));
    let need = if family(ground) == Family::Saturated {
        INK_DISPLAY + MARGIN
    } else {
        INK_SMALL + MARGIN
    };
    if best >= need {
        return ground;
    }
    // Move toward whichever end is nearer.
    let dir = if o.l >= 0.55 { 1.0 } else { -1.0 };
    let mut g = ground;
    for k in 1..=40 {
        let cand = from_oklch(Oklch {
            l: (o.l + dir * 0.01 * k as f32).clamp(0.05, 0.98),
            ..o
        });
        g = cand;
        if contrast(cand, dark).max(contrast(cand, light)) >= need {
            break;
        }
    }
    repairs.push(format!(
        "ground {} cannot carry {need}:1 type; moved to {}",
        hex(ground),
        hex(g)
    ));
    g
}

/// Ink for `ground`: the author's ink when it is the opposite extreme and
/// clears 7:1, else the extreme (near-black or off-white) with the higher
/// contrast.
fn ink_for(ground: Color, asked: Color, fam: Family) -> Color {
    let o = oklch(asked);
    let ok_family = match fam {
        Family::Dark => o.l >= 0.93,
        Family::Light => o.l <= 0.20,
        Family::Saturated => o.l <= 0.16 || o.l >= 0.95,
    };
    let need = if fam == Family::Saturated {
        INK_DISPLAY + MARGIN
    } else {
        INK_SMALL
    };
    if ok_family && contrast(asked, ground) >= need {
        return asked;
    }
    let (dark, light) = extremes(o.h);
    let dark = if o.l <= 0.16 { asked } else { dark };
    let light = if o.l >= 0.95 { asked } else { light };
    match fam {
        Family::Dark => light,
        Family::Light => dark,
        Family::Saturated => {
            if contrast(dark, ground) >= contrast(light, ground) {
                dark
            } else {
                light
            }
        }
    }
}

/// Texture: the ground's hue, L ± 0.05–0.10 toward the middle (darker on
/// light and saturated grounds, lighter on dark ones), contrast inside
/// 1.08–1.35 and ΔL ≥ 0.45 from the ink.
fn texture_for(ground: Color, ink: Color, fam: Family) -> Color {
    let g = oklch(ground);
    let ink_l = oklch(ink).l;
    let dir = match fam {
        Family::Dark => 1.0,
        Family::Light | Family::Saturated => -1.0,
    };
    let target = 1.2f32;
    let mut best: Option<(f32, Color)> = None;
    for k in 0..=12 {
        let dl = 0.04 + 0.005 * k as f32;
        let t = from_oklch(Oklch {
            l: (g.l + dir * dl).clamp(0.0, 1.0),
            // A dark ground's texture keeps its hue but lifts a touch of
            // chroma so it reads as the ground's own colour, not grey.
            c: if fam == Family::Dark {
                (g.c + 0.01).min(0.06)
            } else {
                g.c
            },
            h: g.h,
        });
        let c = contrast(t, ground);
        if !(TEXTURE_MIN..=TEXTURE_MAX).contains(&c) || (oklch(t).l - ink_l).abs() < TEXTURE_INK_DL
        {
            continue;
        }
        let score = (c - target).abs();
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, t));
        }
    }
    best.map_or_else(
        || {
            // Fallback: the smallest legal step on the far side from ink.
            from_oklch(Oklch {
                l: (g.l + dir * 0.05).clamp(0.0, 1.0),
                ..g
            })
        },
        |(_, t)| t,
    )
}

/// The accent on `ground`: kept at ≥ 3:1; else moved in lightness (same
/// hue) away from the ground; an accent that shares the ground's hue
/// becomes the opposite value (the ink).
fn accent_for(ground: Color, accent: Color, ink: Color, need: f32) -> Color {
    if contrast(accent, ground) >= need {
        return accent;
    }
    let (g, a) = (oklch(ground), oklch(accent));
    if hue_gap(g.h, a.h) < 60.0 && g.c >= 0.06 && a.c >= 0.06 {
        return ink;
    }
    let dir = if luminance(ground) > 0.18 { -1.0 } else { 1.0 };
    for k in 1..=60 {
        let cand = from_oklch(Oklch {
            l: (a.l + dir * 0.01 * k as f32).clamp(0.0, 1.0),
            ..a
        });
        if contrast(cand, ground) >= need {
            return cand;
        }
    }
    ink
}

/// Role colours on any ground (a section ground, a poster flip ground, a
/// card fill) for the program's ink and accent.
pub fn roles_on(ground: Color, ink: Color, accent: Color) -> Roles {
    let mut sink = Vec::new();
    roles_with(ground, ink, accent, &mut sink)
}

fn roles_with(ground: Color, ink: Color, accent: Color, repairs: &mut Vec<String>) -> Roles {
    let ground = fix_ground(ground, repairs);
    let fam = family(ground);
    let ink = ink_for(ground, ink, fam);
    let texture = texture_for(ground, ink, fam);
    let need_small = INK_SMALL + MARGIN;
    let ink_secondary = if contrast(ink, ground) > need_small + 1.5 {
        toward_contrast(ground, ink, need_small)
    } else {
        ink
    };
    Roles {
        ground,
        family: fam,
        texture,
        structure: toward_contrast(ground, ink, STRUCTURE_DECOR),
        structure_strong: toward_contrast(ground, ink, STRUCTURE_MEANING),
        ink,
        ink_secondary,
        accent: accent_for(ground, accent, ink, ACCENT_MIN),
        accent_text: accent_for(ground, accent, ink, INK_DISPLAY + MARGIN),
        surface: toward_contrast(ground, ink, 1.25),
    }
}

/// Resolves a palette: role sets for `background` and every extra ground.
/// Invalid entries are re-derived and listed in `repairs`.
pub fn resolve(p: &Palette) -> Resolved {
    let c = |s: &str| Color::parse_hex(s).unwrap_or(Color::rgb(0, 0, 0));
    let (bg, ink, accent, muted) = (c(&p.background), c(&p.ink), c(&p.accent), c(&p.muted));
    let mut repairs = Vec::new();
    let main = roles_with(bg, ink, accent, &mut repairs);
    if main.ink != ink {
        repairs.push(format!(
            "ink {} re-derived as {} on {}",
            hex(ink),
            hex(main.ink),
            hex(main.ground)
        ));
    }
    if (oklch(muted).l - oklch(main.ink).l).abs() < TEXTURE_INK_DL
        || !(TEXTURE_MIN..=TEXTURE_MAX).contains(&contrast(muted, main.ground))
    {
        repairs.push(format!(
            "muted {} is not a texture of {}: fields use {}",
            hex(muted),
            hex(main.ground),
            hex(main.texture)
        ));
    }
    if main.accent != accent {
        repairs.push(format!(
            "accent {} re-derived as {} on {}",
            hex(accent),
            hex(main.accent),
            hex(main.ground)
        ));
    }
    // Hue budget (§2): the accent's hue family first, then the grounds in
    // order; a ground past three families becomes a neutral of the same
    // lightness.
    let mut families: Vec<f32> = Vec::new();
    let mut admit = |col: Color| -> bool {
        let o = oklch(col);
        if o.c < 0.06 || families.iter().any(|h| hue_gap(*h, o.h) <= 30.0) {
            return true;
        }
        if families.len() < HUES_MAX {
            families.push(o.h);
            return true;
        }
        false
    };
    admit(accent);
    admit(main.ground);
    let grounds = p
        .grounds
        .iter()
        .map(|g| {
            let mut gc = c(g);
            if !admit(gc) {
                let o = oklch(gc);
                let n = from_oklch(Oklch { c: 0.01, ..o });
                repairs.push(format!(
                    "ground {} is a fourth hue; set as neutral {}",
                    hex(gc),
                    hex(n)
                ));
                gc = n;
            }
            roles_with(gc, ink, accent, &mut repairs)
        })
        .collect();
    Resolved {
        accent,
        ink,
        main,
        grounds,
        repairs,
    }
}
