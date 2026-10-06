//! StyleProfile -> concrete palette and font assignments.

use std::collections::BTreeMap;

use super::taste::TypographyPairing;
use crate::scene::{Color, FontRole};
use crate::style::{AccentRole, MaterialStyle, StyleProfile, TypographyStyle};

#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub paper: Color,
    /// Lighter stock for collage cards.
    pub card: Color,
    pub ink: Color,
    pub muted: Color,
    pub accent: Color,
    /// Text color on top of the accent.
    pub on_accent: Color,
    /// (0.6) Large graphic field colors (print backgrounds, panel wipes).
    /// Empty for palettes without graphic fields.
    pub fields: Vec<Color>,
}

impl Default for Palette {
    fn default() -> Self {
        Palette::for_style(&StyleProfile::default())
    }
}

impl Palette {
    pub fn for_style(style: &StyleProfile) -> Self {
        let (paper, card) = match style.material {
            MaterialStyle::Paper => (Color::rgb(0xEC, 0xE3, 0xD2), Color::rgb(0xF6, 0xF1, 0xE6)),
            MaterialStyle::Flat => (Color::rgb(0xF2, 0xF0, 0xEB), Color::rgb(0xFF, 0xFF, 0xFF)),
        };
        let (accent, on_accent) = match style.accent_role {
            AccentRole::SignalRed => (Color::rgb(0xE0, 0x33, 0x1F), Color::rgb(0xF6, 0xF1, 0xE6)),
            AccentRole::Cobalt => (Color::rgb(0x23, 0x3F, 0xD1), Color::rgb(0xF6, 0xF1, 0xE6)),
            AccentRole::Acid => (Color::rgb(0xC9, 0xE4, 0x2B), Color::rgb(0x16, 0x14, 0x12)),
        };
        Palette {
            paper,
            card,
            ink: Color::rgb(0x17, 0x15, 0x13),
            muted: Color::rgb(0x6F, 0x66, 0x5A),
            accent,
            on_accent,
            fields: Vec::new(),
        }
    }

    /// Graphic field color `i` (cycling); the accent when there are no fields.
    pub fn field(&self, i: usize) -> Color {
        if self.fields.is_empty() {
            self.accent
        } else {
            self.fields[i % self.fields.len()]
        }
    }

    pub fn named(&self) -> BTreeMap<String, Color> {
        [
            ("paper", self.paper),
            ("card", self.card),
            ("ink", self.ink),
            ("muted", self.muted),
            ("accent", self.accent),
            ("on_accent", self.on_accent),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .chain(
            self.fields
                .iter()
                .enumerate()
                .map(|(i, c)| (format!("field_{}", i + 1), *c)),
        )
        .collect()
    }
}

/// One font file known to the compiler, relative to the asset library root.
#[derive(Debug, Clone, PartialEq)]
pub struct FontFace {
    pub asset_id: &'static str,
    pub path: &'static str,
    pub weight: u16,
    pub italic: bool,
}

const fn face(asset_id: &'static str, path: &'static str, weight: u16, italic: bool) -> FontFace {
    FontFace {
        asset_id,
        path,
        weight,
        italic,
    }
}

const ARCHIVO_BLACK: FontFace = face(
    "font.archivo_black",
    "fonts/ArchivoBlack-Regular.ttf",
    900,
    false,
);
const ANTON: FontFace = face("font.anton", "fonts/Anton-Regular.ttf", 400, false);
const BEBAS: FontFace = face("font.bebas_neue", "fonts/BebasNeue-Regular.ttf", 400, false);
const DM_SERIF: FontFace = face(
    "font.dm_serif",
    "fonts/DMSerifDisplay-Regular.ttf",
    400,
    false,
);
const DM_SERIF_ITALIC: FontFace = face(
    "font.dm_serif_italic",
    "fonts/DMSerifDisplay-Italic.ttf",
    400,
    true,
);
const FIRA: FontFace = face("font.fira_sans", "fonts/FiraSans-Regular.ttf", 400, false);
const FIRA_SEMIBOLD: FontFace = face(
    "font.fira_sans_semibold",
    "fonts/FiraSans-SemiBold.ttf",
    600,
    false,
);
const SPACE_MONO: FontFace = face("font.space_mono", "fonts/SpaceMono-Regular.ttf", 400, false);
const SPACE_MONO_BOLD: FontFace = face(
    "font.space_mono_bold",
    "fonts/SpaceMono-Bold.ttf",
    700,
    false,
);

// 0.8 emotion face sets (OFL, static instances from google/fonts).
const LATO: FontFace = face("font.lato", "fonts/Lato-Regular.ttf", 400, false);
const PLEX_MONO: FontFace = face(
    "font.plex_mono",
    "fonts/IBMPlexMono-Regular.ttf",
    400,
    false,
);
const PLEX_MONO_MEDIUM: FontFace = face(
    "font.plex_mono_medium",
    "fonts/IBMPlexMono-Medium.ttf",
    500,
    false,
);
const PLEX_MONO_SEMIBOLD: FontFace = face(
    "font.plex_mono_semibold",
    "fonts/IBMPlexMono-SemiBold.ttf",
    600,
    false,
);
const PLEX_SERIF_ITALIC: FontFace = face(
    "font.plex_serif_italic",
    "fonts/IBMPlexSerif-Italic.ttf",
    400,
    true,
);
const BARLOW_COND_BOLD: FontFace = face(
    "font.barlow_condensed_bold",
    "fonts/BarlowCondensed-Bold.ttf",
    700,
    false,
);
const BARLOW_COND_SEMIBOLD: FontFace = face(
    "font.barlow_condensed_semibold",
    "fonts/BarlowCondensed-SemiBold.ttf",
    600,
    false,
);
const BARLOW_MEDIUM: FontFace = face("font.barlow_medium", "fonts/Barlow-Medium.ttf", 500, false);
const POPPINS_BLACK: FontFace = face("font.poppins_black", "fonts/Poppins-Black.ttf", 900, false);
const POPPINS_EXTRABOLD: FontFace = face(
    "font.poppins_extrabold",
    "fonts/Poppins-ExtraBold.ttf",
    800,
    false,
);
const POPPINS_BOLD: FontFace = face("font.poppins_bold", "fonts/Poppins-Bold.ttf", 700, false);
const POPPINS_MEDIUM: FontFace = face(
    "font.poppins_medium",
    "fonts/Poppins-Medium.ttf",
    500,
    false,
);
const ZILLA_SEMIBOLD_ITALIC: FontFace = face(
    "font.zilla_slab_semibold_italic",
    "fonts/ZillaSlab-SemiBoldItalic.ttf",
    600,
    true,
);

/// Role -> face assignment plus every face that should be available to the renderer.
#[derive(Debug, Clone)]
pub struct FontSet {
    pub roles: BTreeMap<FontRole, FontFace>,
    pub faces: Vec<FontFace>,
}

impl FontSet {
    pub fn for_style(style: &StyleProfile) -> Self {
        Self::for_pairing(match style.typography_style {
            TypographyStyle::GroteskSerif => TypographyPairing::GroteskSerif,
            TypographyStyle::CondensedMono => TypographyPairing::CondensedMono,
        })
    }

    pub fn for_pairing(pairing: TypographyPairing) -> Self {
        let roles: BTreeMap<FontRole, FontFace> = match pairing {
            TypographyPairing::GroteskSerif => [
                (FontRole::Display, ARCHIVO_BLACK),
                (FontRole::DisplayCondensed, ANTON),
                (FontRole::SerifEmotional, DM_SERIF_ITALIC),
                (FontRole::Body, FIRA),
                (FontRole::Mono, SPACE_MONO),
                (FontRole::Number, ANTON),
            ]
            .into_iter()
            .collect(),
            TypographyPairing::SerifSans => [
                (FontRole::Display, DM_SERIF),
                (FontRole::DisplayCondensed, ANTON),
                (FontRole::SerifEmotional, DM_SERIF_ITALIC),
                (FontRole::Body, FIRA),
                (FontRole::Mono, SPACE_MONO),
                (FontRole::Number, DM_SERIF),
            ]
            .into_iter()
            .collect(),
            TypographyPairing::PosterBold => [
                (FontRole::Display, ARCHIVO_BLACK),
                (FontRole::DisplayCondensed, ARCHIVO_BLACK),
                (FontRole::SerifEmotional, DM_SERIF),
                (FontRole::Body, FIRA_SEMIBOLD),
                (FontRole::Mono, SPACE_MONO_BOLD),
                (FontRole::Number, ARCHIVO_BLACK),
            ]
            .into_iter()
            .collect(),
            TypographyPairing::HumanistSerif => [
                (FontRole::Display, DM_SERIF),
                (FontRole::DisplayCondensed, BARLOW_COND_SEMIBOLD),
                (FontRole::SerifEmotional, DM_SERIF_ITALIC),
                (FontRole::Body, LATO),
                (FontRole::Mono, PLEX_MONO),
                (FontRole::Number, DM_SERIF),
            ]
            .into_iter()
            .collect(),
            TypographyPairing::PrecisionGrotesk => [
                (FontRole::Display, BARLOW_COND_BOLD),
                (FontRole::DisplayCondensed, BARLOW_COND_SEMIBOLD),
                (FontRole::SerifEmotional, PLEX_SERIF_ITALIC),
                (FontRole::Body, BARLOW_MEDIUM),
                (FontRole::Mono, PLEX_MONO_MEDIUM),
                (FontRole::Number, BARLOW_COND_BOLD),
            ]
            .into_iter()
            .collect(),
            TypographyPairing::FriendlyGeometric => [
                (FontRole::Display, POPPINS_EXTRABOLD),
                (FontRole::DisplayCondensed, POPPINS_BOLD),
                (FontRole::SerifEmotional, ZILLA_SEMIBOLD_ITALIC),
                (FontRole::Body, POPPINS_MEDIUM),
                (FontRole::Mono, PLEX_MONO_SEMIBOLD),
                (FontRole::Number, POPPINS_BLACK),
            ]
            .into_iter()
            .collect(),
            TypographyPairing::CondensedMono => [
                (FontRole::Display, ANTON),
                (FontRole::DisplayCondensed, BEBAS),
                (FontRole::SerifEmotional, DM_SERIF),
                (FontRole::Body, SPACE_MONO),
                (FontRole::Mono, SPACE_MONO),
                (FontRole::Number, BEBAS),
            ]
            .into_iter()
            .collect(),
        };
        let mut faces: Vec<FontFace> = roles.values().cloned().collect();
        for extra in [DM_SERIF, DM_SERIF_ITALIC, FIRA_SEMIBOLD, SPACE_MONO_BOLD] {
            faces.push(extra);
        }
        // 0.8 sets: a full-coverage face (₹, €, curly quotes) for glyph fallback.
        if matches!(
            pairing,
            TypographyPairing::HumanistSerif
                | TypographyPairing::PrecisionGrotesk
                | TypographyPairing::FriendlyGeometric
        ) {
            faces.push(PLEX_MONO);
        }
        faces.sort_by_key(|f| f.asset_id);
        faces.dedup_by_key(|f| f.asset_id);
        FontSet { roles, faces }
    }

    pub fn role(&self, role: FontRole) -> &FontFace {
        // Every role is assigned in `for_style`; fall back defensively to display.
        self.roles
            .get(&role)
            .or_else(|| self.roles.get(&FontRole::Display))
            .unwrap_or(&ARCHIVO_BLACK)
    }
}
