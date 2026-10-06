//! (0.21) Brand colours: `StyleProfile.brand` replaces the look's colours.
//!
//! Applied once, after the look (or tone) has chosen its palette, so every
//! grammar and look inherits it: `primary` becomes the accent (highlights,
//! rules, transitions, key figures), `secondary` leads the large colour fields,
//! `background` becomes the ground (the look's paper plate is dropped) and
//! `text` the ink. Readability is guaranteed, not hoped for: ink and the text
//! on the accent are kept at ≥ [`TEXT_CONTRAST`] (WCAG AA for body text),
//! replaced by near-black or near-white when a brand colour cannot do it. The
//! brand's accent is used exactly unless it is faint on the background (under
//! 2:1, e.g. yellow on white): then a readable `secondary` leads, or the
//! colour is nudged toward the ink just until it reaches [`ACCENT_CONTRAST`].
//! Every change is reported as a note (a compile warning). Pure and
//! deterministic.

use super::theme::Palette;
use crate::scene::Color;
use crate::style::BrandColors;

/// Minimum contrast of text on its ground (WCAG AA, body text).
pub const TEXT_CONTRAST: f64 = 4.5;
/// Minimum contrast of the accent on the background (WCAG for large text and
/// graphics): rules, figures and highlights in the accent stay visible.
pub const ACCENT_CONTRAST: f64 = 3.0;
/// Below this the brand's `primary` is too faint to lead; a `secondary` that
/// reads well takes over the accent.
const FAINT: f64 = 2.0;

const NEAR_BLACK: Color = Color::rgb(0x17, 0x15, 0x13);
const NEAR_WHITE: Color = Color::rgb(0xF6, 0xF3, 0xEC);

/// Brand colours, parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Brand {
    pub primary: Option<Color>,
    pub secondary: Option<Color>,
    pub background: Option<Color>,
    pub text: Option<Color>,
}

/// Parse `#RRGGBB` or `#RGB` (case-insensitive).
pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    let h = s.strip_prefix('#')?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match h.len() {
        6 => Color::parse_hex(s).map(|c| c.with_alpha(255)),
        3 => {
            let long: String = h.chars().flat_map(|c| [c, c]).collect();
            Color::parse_hex(&format!("#{long}"))
        }
        _ => None,
    }
}

/// Parse every colour; the error names the field and the value.
pub fn parse(brand: &BrandColors) -> Result<Brand, String> {
    let field = |name: &str, v: &Option<String>| -> Result<Option<Color>, String> {
        match v {
            None => Ok(None),
            Some(s) => parse_color(s).map(Some).ok_or_else(|| {
                format!("style.brand.{name} '{s}' is not a hex colour like #0A84FF")
            }),
        }
    };
    Ok(Brand {
        primary: field("primary", &brand.primary)?,
        secondary: field("secondary", &brand.secondary)?,
        background: field("background", &brand.background)?,
        text: field("text", &brand.text)?,
    })
}

fn channel(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// WCAG relative luminance.
pub fn luminance(c: Color) -> f64 {
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

/// WCAG contrast ratio (1..=21).
pub fn contrast(a: Color, b: Color) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// `a` moved `t` (0..=1) of the way to `b`, per channel, opaque.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let m = |x: u8, y: u8| {
        (x as f64 + (y as f64 - x as f64) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::rgb(m(a.r, b.r), m(a.g, b.g), m(a.b, b.b))
}

/// Near-black or near-white, whichever reads better on `ground`.
fn readable_on(ground: Color) -> Color {
    if contrast(NEAR_BLACK, ground) >= contrast(NEAR_WHITE, ground) {
        NEAR_BLACK
    } else {
        NEAR_WHITE
    }
}

/// `c` moved toward `toward` in small steps until it reaches `min` contrast on
/// `ground` (the smallest change that does).
fn nudge(c: Color, toward: Color, ground: Color, min: f64) -> Color {
    (0..=20)
        .map(|i| mix(c, toward, i as f64 / 20.0))
        .find(|x| contrast(*x, ground) >= min)
        .unwrap_or(toward)
}

/// The card stock on `paper`: a quiet step lighter (darker on near-white).
fn card_on(paper: Color) -> Color {
    let l = luminance(paper);
    if l > 0.85 {
        mix(paper, Color::rgb(0, 0, 0), 0.05)
    } else if l > 0.4 {
        mix(paper, Color::rgb(255, 255, 255), 0.45)
    } else {
        mix(paper, Color::rgb(255, 255, 255), 0.10)
    }
}

/// The colour for a look's own fixed marks (e.g. the documentary look's red
/// underline) when the piece has a brand: the brand's `secondary`, else its
/// `primary`, else `fallback` (no brand: the look's own colour, unchanged).
pub fn mark_color(style: &crate::style::StyleProfile, fallback: Color) -> Color {
    style
        .brand
        .as_ref()
        .and_then(|b| {
            b.secondary
                .as_deref()
                .and_then(parse_color)
                .or_else(|| b.primary.as_deref().and_then(parse_color))
        })
        .unwrap_or(fallback)
}

/// `base` with the brand's colours, and a note for every colour the
/// readability rules had to change.
pub fn apply(base: &Palette, brand: &Brand) -> (Palette, Vec<String>) {
    let mut p = base.clone();
    let mut notes = Vec::new();

    // Ground.
    if let Some(bg) = brand.background {
        p.paper = bg;
        p.card = card_on(bg);
    }

    // Ink: the brand's text colour when it reads; else the look's; else the
    // better of near-black / near-white.
    let ink_reads = |c: Color| contrast(c, p.paper) >= TEXT_CONTRAST;
    p.ink = match brand.text {
        Some(t) if ink_reads(t) => t,
        Some(t) => {
            let ink = readable_on(p.paper);
            notes.push(format!(
                "brand text {} on {} is {:.1}:1 (needs {TEXT_CONTRAST}:1); text uses {}",
                t.to_hex(),
                p.paper.to_hex(),
                contrast(t, p.paper),
                ink.to_hex()
            ));
            ink
        }
        None if ink_reads(base.ink) => base.ink,
        None => readable_on(p.paper),
    };
    if brand.background.is_some() || brand.text.is_some() {
        p.muted = mix(p.ink, p.paper, 0.4);
    }

    // Accent: the primary leads; a primary too faint on the ground hands the
    // lead to a secondary that reads, else it is nudged toward the ink.
    let faint = |c: Color| contrast(c, p.paper) < FAINT;
    let (lead, support) = match (brand.primary, brand.secondary) {
        (Some(pr), Some(se)) if faint(pr) && contrast(se, p.paper) >= ACCENT_CONTRAST => {
            notes.push(format!(
                "brand primary {} is faint on {} ({:.1}:1); secondary {} leads",
                pr.to_hex(),
                p.paper.to_hex(),
                contrast(pr, p.paper),
                se.to_hex()
            ));
            (Some(se), Some(pr))
        }
        other => other,
    };
    if let Some(a) = lead {
        // The brand's exact colour unless it is faint: a brand blue at 2.9:1
        // on paper still reads in large figures and fills, and owners expect
        // their colour, not a near miss.
        let accent = if !faint(a) {
            a
        } else {
            let n = nudge(a, p.ink, p.paper, ACCENT_CONTRAST);
            notes.push(format!(
                "brand {} on {} is {:.1}:1; highlights use {} ({:.1}:1)",
                a.to_hex(),
                p.paper.to_hex(),
                contrast(a, p.paper),
                n.to_hex(),
                contrast(n, p.paper)
            ));
            n
        };
        p.accent = accent;
    } else if brand.background.is_some() && contrast(p.accent, p.paper) < ACCENT_CONTRAST {
        // The look's accent on the brand's ground.
        p.accent = nudge(p.accent, p.ink, p.paper, ACCENT_CONTRAST);
    }
    if contrast(p.on_accent, p.accent) < TEXT_CONTRAST {
        p.on_accent = readable_on(p.accent);
    }

    // Large colour fields: the brand's colours, secondary first.
    let colours: Vec<Color> = [support.or(brand.secondary), lead]
        .into_iter()
        .flatten()
        .collect();
    match colours.len() {
        0 => {}
        1 => {
            if let Some(f) = p.fields.first_mut() {
                *f = colours[0];
            }
        }
        n => {
            for (i, f) in p.fields.iter_mut().enumerate() {
                *f = colours[i % n];
            }
        }
    }
    (p, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Palette {
        Palette {
            paper: Color::rgb(0xEC, 0xE3, 0xD2),
            card: Color::rgb(0xF6, 0xF1, 0xE6),
            ink: NEAR_BLACK,
            muted: Color::rgb(0x6F, 0x66, 0x5A),
            accent: Color::rgb(0xE0, 0x33, 0x1F),
            on_accent: Color::rgb(0xF6, 0xF1, 0xE6),
            fields: vec![
                Color::rgb(1, 2, 3),
                Color::rgb(4, 5, 6),
                Color::rgb(7, 8, 9),
            ],
        }
    }

    fn c(s: &str) -> Color {
        parse_color(s).unwrap()
    }

    #[test]
    fn colours_parse_long_and_short() {
        assert_eq!(parse_color("#0a84ff"), Some(Color::rgb(0x0A, 0x84, 0xFF)));
        assert_eq!(parse_color(" #0AF "), Some(Color::rgb(0x00, 0xAA, 0xFF)));
        for bad in ["0A84FF", "#0A84F", "#GGGGGG", "blue", "#0A84FF80", ""] {
            assert_eq!(parse_color(bad), None, "{bad}");
        }
        let err = parse(&BrandColors {
            primary: Some("blue".into()),
            ..Default::default()
        })
        .unwrap_err();
        assert!(err.contains("style.brand.primary 'blue'"), "{err}");
    }

    #[test]
    fn a_primary_becomes_the_accent_and_leads_the_fields() {
        let (p, notes) = apply(
            &base(),
            &Brand {
                primary: Some(c("#0A84FF")),
                ..Default::default()
            },
        );
        assert_eq!(p.accent, c("#0A84FF"));
        assert_eq!(p.fields[0], c("#0A84FF"));
        assert_eq!(p.fields[1], Color::rgb(4, 5, 6));
        assert_eq!((p.paper, p.ink), (base().paper, base().ink));
        assert!(contrast(p.on_accent, p.accent) >= TEXT_CONTRAST);
        assert!(notes.is_empty(), "{notes:?}");
    }

    #[test]
    fn text_always_reads_on_the_brand_background() {
        // A dark brand ground with a dark text colour: the text is replaced.
        let (p, notes) = apply(
            &base(),
            &Brand {
                primary: Some(c("#FF375F")),
                secondary: Some(c("#FFD60A")),
                background: Some(c("#101828")),
                text: Some(c("#1D2939")),
            },
        );
        assert_eq!(p.paper, c("#101828"));
        assert!(contrast(p.ink, p.paper) >= TEXT_CONTRAST);
        assert!(contrast(p.on_accent, p.accent) >= TEXT_CONTRAST);
        assert!(contrast(p.accent, p.paper) >= ACCENT_CONTRAST);
        assert_eq!(p.fields, vec![c("#FFD60A"), c("#FF375F"), c("#FFD60A")]);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].starts_with("brand text #1D2939"), "{notes:?}");
    }

    #[test]
    fn a_faint_primary_hands_over_or_is_nudged() {
        // Yellow on white: faint. With a readable secondary, the secondary leads.
        let brand = Brand {
            primary: Some(c("#FFE600")),
            secondary: Some(c("#1F3A93")),
            background: Some(c("#FFFFFF")),
            text: None,
        };
        let (p, notes) = apply(&base(), &brand);
        assert_eq!(p.accent, c("#1F3A93"));
        assert!(notes[0].contains("secondary #1F3A93 leads"), "{notes:?}");
        // Alone, the yellow is darkened just enough to be seen.
        let (p, notes) = apply(
            &base(),
            &Brand {
                secondary: None,
                ..brand
            },
        );
        assert!(contrast(p.accent, p.paper) >= ACCENT_CONTRAST);
        assert!(contrast(p.accent, p.paper) < ACCENT_CONTRAST + 1.0);
        assert!(notes[0].contains("highlights use"), "{notes:?}");
        // Deterministic.
        assert_eq!(apply(&base(), &brand), apply(&base(), &brand));
    }
}
