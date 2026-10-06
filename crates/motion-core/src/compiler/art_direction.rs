//! (0.12) Art-direction systems: whole-piece looks chosen by the ENGINE from
//! emotion × polarity (never authored by weak models). Opt-in operator knob
//! `--art auto|<look>` (CompileOptions.art); `None` = byte-identical to 0.11.
//! See docs/ART_DIRECTION.md.
//!
//! FROZEN: `Look`, `ArtDirection`, `SfxPalette`, `art_direction_for`,
//! `neighbours`, `ArtMode`. Builders: looks, plates, treatment preset, pop-ins,
//! ornaments, journey, end card.

use super::typography::Emotion;

/// Whole-piece look.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Look {
    /// (1) Classical greyscale cutouts on dark grounds + ONE neon accent from
    /// the palette (duotone off). drama · luxury.
    ClassicalNeon,
    /// (2) Halftone B&W photographic cutouts (halftone treatment preset),
    /// bold print grounds. energy · urgency · playful_retro.
    HalftoneCutout,
    /// (3) Soft clay props + glyph icons that pop in with a spring and a soft
    /// contact shadow on EVOLVE. joy.
    ClayPop,
    /// (4) Editorial page: ornaments as furniture (corners, dividers), paper
    /// plates. trust · warmth · handmade.
    OrnamentEditorial,
    /// (5) A stroked ribbon path that draws through the beats; objects travel
    /// along it. calm · precision.
    Journey,
    /// (0.14) Street collage: hero cutouts own the frame, stencil punchwords
    /// behind them, tape strips, halftone photo grounds, hard cuts, camera
    /// shake and chromatic flashes on the hits. tone `street`.
    StreetCollage,
    /// (0.14) Investigative dossier: evidence documents slide in, get stamped
    /// and highlighted on newsprint/desk grounds, film grain, slow drift.
    /// tone `documentary`.
    Dossier,
    /// (0.14) Hype slam: words and pictures slam into frame in fast cuts timed
    /// to the voice, shake + glitch on cuts, colour-field inversions.
    /// tone `hype`.
    HypeSlam,
    /// (0.14) Studio pop: one big cutout over a bold colour disc on a clean
    /// studio ground, the spoken words appearing large behind the subject.
    /// tone `studio`.
    StudioPop,
    /// (0.17) Cinematic 3D: beats staged in depth (far glow, ghost word,
    /// midground props, hero, foreground bokeh), a choreographed perspective
    /// camera (push-in, truck, crane, orbit) with rack focus and fly-through
    /// transitions. tone `cinematic`.
    Cinematic3d,
}

impl Look {
    pub const ALL: [Look; 10] = [
        Look::ClassicalNeon,
        Look::HalftoneCutout,
        Look::ClayPop,
        Look::OrnamentEditorial,
        Look::Journey,
        Look::StreetCollage,
        Look::Dossier,
        Look::HypeSlam,
        Look::StudioPop,
        Look::Cinematic3d,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Look::ClassicalNeon => "classical_neon",
            Look::HalftoneCutout => "halftone_cutout",
            Look::ClayPop => "clay_pop",
            Look::OrnamentEditorial => "ornament_editorial",
            Look::Journey => "journey",
            Look::StreetCollage => "street_collage",
            Look::Dossier => "dossier",
            Look::HypeSlam => "hype_slam",
            Look::StudioPop => "studio_pop",
            Look::Cinematic3d => "cinematic_3d",
        }
    }
    pub fn parse(s: &str) -> Option<Look> {
        Look::ALL.into_iter().find(|l| l.name() == s)
    }
}

/// `--art auto` (engine picks from emotion) or a forced look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtMode {
    Auto,
    Force(Look),
}

/// (7) Emotion-aware SFX palette: which cue families the audio planner
/// prefers for arrivals/handoffs/impacts under this look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SfxPalette {
    /// paper-soft whooshes, quiet ticks (editorial).
    Soft,
    /// clicks, data blips, swipes (journey/precision).
    Crisp,
    /// pops and soft hits (clay).
    Bouncy,
    /// glitch, hard hits, swipes (halftone/retro).
    Punchy,
    /// risers, subdrops, hard hits (classical/drama).
    Cinematic,
}

/// Image treatment preset applied to library cutouts under a look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreatmentPreset {
    /// Keep the asset as delivered.
    None,
    /// Greyscale with lifted contrast (classical marble, duotone off).
    Greyscale,
    /// Coarse halftone dots in ink (halftone B&W cutouts).
    Halftone,
}

/// Everything a look decides. Pure data from [`art_direction_for`].
#[derive(Debug, Clone, PartialEq)]
pub struct ArtDirection {
    pub look: Look,
    /// Asset families the planner searches, in priority order (used when the
    /// operator passed no `--asset-family`).
    pub families: &'static [&'static str],
    /// (8) `grounds` family plate for dark / light polarity (chosen by
    /// emotion, never by story words).
    pub plate_dark: &'static str,
    pub plate_light: &'static str,
    /// Plate opacity over the palette paper.
    pub plate_opacity: f32,
    pub treatment: TreatmentPreset,
    /// (1) Single neon accent: the palette accent at full chroma; every other
    /// colour desaturated.
    pub neon_accent: bool,
    /// (3) Props/icons pop in with spring + soft shadow on EVOLVE.
    pub pop_ins: bool,
    /// (4) Ornaments as furniture (corners on READ, a divider under headlines).
    pub ornaments: bool,
    /// (5) Journey ribbon through the beats.
    pub journey: bool,
    /// (6) Closing end card after the last beat (title + keyword, 2.5 s).
    pub end_card: bool,
    pub sfx: SfxPalette,
    /// (0.14) Genre grammar and finishing effects (all off for the 0.12 looks).
    pub fx: LookFx,
}

/// (0.14) How HeroVisual stages its picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeroStyle {
    /// Street poster: photo ground, stencil punchword behind, tape, sticker
    /// (on light palettes: paper poster with a radial burst and a banner).
    #[default]
    Street,
    /// Studio: a bold colour disc behind the picture, clean ground, floor
    /// shadow; words come from the FX director's kinetic words.
    Disc,
}

/// (0.14) Grammar a genre look prefers for atomic beats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookGrammar {
    HeroVisual,
    KineticSlam,
    DocumentaryDossier,
    Cinematic3d,
}

/// (0.14) Finishing effects a look applies through the FX director
/// (`compiler::fx`). Zero / false = off; the default is all off.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LookFx {
    /// Preferred grammar for atomic beats (`None` = the semantic choice).
    pub grammar: Option<LookGrammar>,
    /// Film grain amount over every beat (0..=1).
    pub grain: f32,
    /// Vignette amount over every beat (0..=1).
    pub vignette: f32,
    /// Chromatic-aberration flash on impact words and cuts.
    pub chroma_hits: bool,
    /// Short glitch burst across beat transitions.
    pub glitch_cuts: bool,
    /// Camera-shake trauma on impact beats / punch words (0 = off).
    pub shake_trauma: f32,
    /// Echo trails on hero entrances.
    pub echo_entrances: bool,
    /// Audio pulse gain on hero/punch layers (needs a music envelope; 0 = off).
    pub pulse_gain: f32,
    /// Soft bloom on bright accents.
    pub bloom: bool,
    /// (0.16) Perspective camera: document tilt, dolly push, depth of field.
    pub perspective: bool,
    /// HeroVisual staging.
    pub hero_style: HeroStyle,
    /// Spoken words appear large behind the hero as they are said (needs a
    /// voice-over; replaces the caption track).
    pub kinetic_words: bool,
    /// (0.17) Choreographed perspective camera per beat (push-in, truck,
    /// crane, orbit), rack focus onto the hero and fly-through transitions.
    pub camera_moves: bool,
    /// (0.20) Whether the display title waits for the word it names.
    pub title_reveal: TitleReveal,
}

/// (0.20) Engine policy for display titles under a voice-over (look-level,
/// never intent): a title that states the beat's number or verdict before
/// the narrator does spoils the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TitleReveal {
    /// The title enters with the beat (the pre-0.20 behaviour; fast slams).
    #[default]
    Free,
    /// The title's arrival is cued to its `RevealRole::Title` anchor word
    /// (fade or glyph cascade on the word), so it never states the
    /// conclusion first. The kicker and the picture still enter on time.
    Hold,
}

// (0.13) Topic families (AI, tech, science, psychology, business) follow the
// look's own medium: people_work after people_everyday, editorial_concepts
// after editorial_cutout, clay_concepts_3d after clay_props_3d,
// classical_concepts after classical_greyscale.
const EDITORIAL_FAMILIES: &[&str] = &[
    "people_everyday",
    "people_work",
    "editorial_cutout",
    "editorial_concepts",
    "sketch_icons",
];
const HANDMADE_FAMILIES: &[&str] = &[
    "woodcut_kitchen",
    "people_everyday",
    "people_work",
    "sketch_icons",
    "editorial_cutout",
    "editorial_concepts",
];
const JOURNEY_FAMILIES: &[&str] = &[
    "people_everyday",
    "people_work",
    "sketch_icons",
    "editorial_cutout",
    "editorial_concepts",
    "clay_props_3d",
    "clay_concepts_3d",
];
const CLAY_FAMILIES: &[&str] = &[
    "clay_props_3d",
    "clay_concepts_3d",
    "people_everyday",
    "people_work",
    "sketch_icons",
    "editorial_cutout",
    "editorial_concepts",
];
const HALFTONE_FAMILIES: &[&str] = &[
    "halftone_retro_objects",
    "people_everyday",
    "people_work",
    "retro_cars_a",
    "retro_cars_b",
    "editorial_cutout",
    "editorial_concepts",
    "clay_props_3d",
    "clay_concepts_3d",
];
const CLASSICAL_FAMILIES: &[&str] = &[
    "classical_greyscale",
    "classical_concepts",
    "people_everyday",
    "people_work",
    "editorial_cutout",
    "editorial_concepts",
    "sketch_icons",
];

const STREET_FAMILIES: &[&str] = &[
    "people_work",
    "people_everyday",
    "editorial_concepts",
    "halftone_retro_objects",
    "editorial_cutout",
    "clay_concepts_3d",
];
const DOSSIER_FAMILIES: &[&str] = &[
    "editorial_concepts",
    "editorial_cutout",
    "people_work",
    "people_everyday",
    "classical_concepts",
    "sketch_icons",
];
const STUDIO_FAMILIES: &[&str] = &[
    "people_work",
    "people_everyday",
    "editorial_concepts",
    "clay_concepts_3d",
    "editorial_cutout",
    "clay_props_3d",
];
const CINEMATIC_FAMILIES: &[&str] = &[
    "clay_concepts_3d",
    "editorial_concepts",
    "people_work",
    "classical_concepts",
    "clay_props_3d",
    "editorial_cutout",
];
const HYPE_FAMILIES: &[&str] = &[
    "clay_concepts_3d",
    "editorial_concepts",
    "people_work",
    "people_everyday",
    "clay_props_3d",
    "editorial_cutout",
];

/// (0.14) The look a genre asks for under `--art auto` (`None` = standard
/// stories keep the emotion table).
pub fn genre_look(genre: super::taste::Genre) -> Option<Look> {
    use super::taste::Genre;
    match genre {
        Genre::Standard => None,
        Genre::Street => Some(Look::StreetCollage),
        Genre::Documentary => Some(Look::Dossier),
        Genre::Hype => Some(Look::HypeSlam),
        Genre::Studio => Some(Look::StudioPop),
        Genre::Cinematic => Some(Look::Cinematic3d),
    }
}

/// Curated table: emotion → art direction (polarity picks the plate).
pub fn art_direction_for(emotion: Emotion) -> ArtDirection {
    use Emotion as E;
    let base = |look: Look| look_defaults(look);
    match emotion {
        E::Trust => ArtDirection {
            plate_dark: "deep_teal",
            plate_light: "linen",
            ..base(Look::OrnamentEditorial)
        },
        E::Warmth => ArtDirection {
            plate_dark: "ink_wash_dark",
            plate_light: "warm_paper",
            ..base(Look::OrnamentEditorial)
        },
        E::Handmade => ArtDirection {
            families: HANDMADE_FAMILIES,
            plate_dark: "ink_wash_dark",
            plate_light: "kraft",
            ..base(Look::OrnamentEditorial)
        },
        E::Calm => ArtDirection {
            plate_dark: "deep_teal",
            plate_light: "sky_gradient",
            ..base(Look::Journey)
        },
        E::Precision => base(Look::Journey),
        E::Joy => base(Look::ClayPop),
        E::Energy => ArtDirection {
            plate_dark: "halftone_dark",
            plate_light: "concrete",
            ..base(Look::HalftoneCutout)
        },
        E::Urgency => ArtDirection {
            plate_dark: "black_grain",
            plate_light: "concrete",
            ..base(Look::HalftoneCutout)
        },
        E::PlayfulRetro => base(Look::HalftoneCutout),
        E::Drama => base(Look::ClassicalNeon),
        E::Luxury => ArtDirection {
            plate_dark: "black_grain",
            plate_light: "grey_studio",
            ..base(Look::ClassicalNeon)
        },
    }
}

/// A look's defaults (used by `--art <look>` and as the table's base rows).
pub fn look_defaults(look: Look) -> ArtDirection {
    let off = ArtDirection {
        look,
        families: EDITORIAL_FAMILIES,
        plate_dark: "black_grain",
        plate_light: "warm_paper",
        plate_opacity: 0.42,
        treatment: TreatmentPreset::None,
        neon_accent: false,
        pop_ins: false,
        ornaments: false,
        journey: false,
        end_card: true,
        sfx: SfxPalette::Soft,
        fx: LookFx::default(),
    };
    match look {
        Look::ClassicalNeon => ArtDirection {
            families: CLASSICAL_FAMILIES,
            plate_dark: "ink_wash_dark",
            plate_light: "concrete",
            treatment: TreatmentPreset::Greyscale,
            neon_accent: true,
            sfx: SfxPalette::Cinematic,
            ..off
        },
        Look::HalftoneCutout => ArtDirection {
            families: HALFTONE_FAMILIES,
            plate_dark: "halftone_dark",
            plate_light: "warm_paper",
            treatment: TreatmentPreset::Halftone,
            pop_ins: true,
            sfx: SfxPalette::Punchy,
            ..off
        },
        Look::ClayPop => ArtDirection {
            families: CLAY_FAMILIES,
            plate_dark: "deep_teal",
            plate_light: "blush",
            pop_ins: true,
            sfx: SfxPalette::Bouncy,
            ..off
        },
        Look::OrnamentEditorial => ArtDirection {
            plate_dark: "ink_wash_dark",
            plate_light: "warm_paper",
            ornaments: true,
            ..off
        },
        Look::Journey => ArtDirection {
            families: JOURNEY_FAMILIES,
            plate_dark: "black_grain",
            plate_light: "grey_studio",
            journey: true,
            sfx: SfxPalette::Crisp,
            ..off
        },
        Look::StreetCollage => ArtDirection {
            families: STREET_FAMILIES,
            plate_dark: "black_grain",
            plate_light: "concrete",
            plate_opacity: 0.55,
            treatment: TreatmentPreset::Halftone,
            sfx: SfxPalette::Punchy,
            fx: LookFx {
                grammar: Some(LookGrammar::HeroVisual),
                grain: 0.35,
                vignette: 0.25,
                chroma_hits: true,
                glitch_cuts: true,
                shake_trauma: 0.55,
                echo_entrances: true,
                pulse_gain: 0.06,
                ..LookFx::default()
            },
            ..off
        },
        Look::Dossier => ArtDirection {
            families: DOSSIER_FAMILIES,
            plate_dark: "ink_wash_dark",
            plate_light: "warm_paper",
            plate_opacity: 0.5,
            sfx: SfxPalette::Cinematic,
            fx: LookFx {
                grammar: Some(LookGrammar::DocumentaryDossier),
                grain: 0.3,
                vignette: 0.4,
                shake_trauma: 0.2,
                perspective: true,
                camera_moves: true,
                title_reveal: TitleReveal::Hold,
                ..LookFx::default()
            },
            ..off
        },
        Look::StudioPop => ArtDirection {
            families: STUDIO_FAMILIES,
            plate_dark: "black_grain",
            plate_light: "grey_studio",
            plate_opacity: 0.35,
            sfx: SfxPalette::Crisp,
            fx: LookFx {
                grammar: Some(LookGrammar::HeroVisual),
                hero_style: HeroStyle::Disc,
                kinetic_words: true,
                grain: 0.08,
                vignette: 0.18,
                pulse_gain: 0.03,
                title_reveal: TitleReveal::Hold,
                ..LookFx::default()
            },
            ..off
        },
        Look::Cinematic3d => ArtDirection {
            families: CINEMATIC_FAMILIES,
            plate_dark: "black_grain",
            plate_light: "grey_studio",
            plate_opacity: 0.3,
            sfx: SfxPalette::Cinematic,
            fx: LookFx {
                grammar: Some(LookGrammar::Cinematic3d),
                perspective: true,
                camera_moves: true,
                grain: 0.12,
                vignette: 0.4,
                bloom: true,
                chroma_hits: true,
                shake_trauma: 0.3,
                // (0.19) No audio pulse: a flat picture kicked 4-5 times a
                // second reads as throbbing, not as a camera move.
                pulse_gain: 0.0,
                title_reveal: TitleReveal::Hold,
                ..LookFx::default()
            },
            ..off
        },
        Look::HypeSlam => ArtDirection {
            families: HYPE_FAMILIES,
            plate_dark: "black_grain",
            plate_light: "grey_studio",
            pop_ins: true,
            sfx: SfxPalette::Punchy,
            fx: LookFx {
                grammar: Some(LookGrammar::KineticSlam),
                grain: 0.15,
                chroma_hits: true,
                glitch_cuts: true,
                shake_trauma: 0.7,
                echo_entrances: true,
                pulse_gain: 0.08,
                bloom: true,
                // Fast slams: the title lands with the beat, never waits.
                title_reveal: TitleReveal::Free,
                ..LookFx::default()
            },
            ..off
        },
    }
}

/// Exploration L2/L3 may switch to one of these neighbours (curated table).
pub fn neighbours(look: Look) -> [Look; 2] {
    match look {
        Look::ClassicalNeon => [Look::OrnamentEditorial, Look::HalftoneCutout],
        Look::HalftoneCutout => [Look::ClayPop, Look::ClassicalNeon],
        Look::ClayPop => [Look::HalftoneCutout, Look::Journey],
        Look::OrnamentEditorial => [Look::Journey, Look::ClassicalNeon],
        Look::Journey => [Look::OrnamentEditorial, Look::ClayPop],
        Look::StreetCollage => [Look::HypeSlam, Look::HalftoneCutout],
        Look::Dossier => [Look::ClassicalNeon, Look::OrnamentEditorial],
        Look::HypeSlam => [Look::StreetCollage, Look::HalftoneCutout],
        Look::StudioPop => [Look::StreetCollage, Look::ClayPop],
        Look::Cinematic3d => [Look::Dossier, Look::Journey],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_emotion_has_a_direction_with_known_plates() {
        let grounds = [
            "neon_purple_gradient",
            "black_grain",
            "grey_studio",
            "warm_paper",
            "kraft",
            "sky_gradient",
            "deep_teal",
            "blush",
            "halftone_dark",
            "concrete",
            "linen",
            "ink_wash_dark",
        ];
        for e in Emotion::ALL {
            let d = art_direction_for(e);
            assert!(grounds.contains(&d.plate_dark), "{e:?}");
            assert!(grounds.contains(&d.plate_light), "{e:?}");
            assert!(!d.families.is_empty());
        }
        for l in Look::ALL {
            assert_eq!(Look::parse(l.name()), Some(l));
            assert!(!neighbours(l).contains(&l));
        }
    }
}

// ---------------------------------------------------------------------------
// (0.10 Q) Curated palettes per look — colour variety across stories
// ---------------------------------------------------------------------------

/// One curated palette (hex RGB). `dark` = light text on a dark ground.
/// Contrast pairs are checked by `palettes_meet_contrast` below: ink/paper
/// ≥ 7:1, on_accent/accent ≥ 4.5:1, accent/paper ≥ 3:1 (graphic use).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaletteSpec {
    pub dark: bool,
    pub paper: u32,
    pub card: u32,
    pub ink: u32,
    pub muted: u32,
    pub accent: u32,
    pub on_accent: u32,
    /// Large graphic fields (print looks); empty = none.
    pub fields: &'static [u32],
}

#[allow(clippy::too_many_arguments)]
const fn p(
    dark: bool,
    paper: u32,
    card: u32,
    ink: u32,
    muted: u32,
    accent: u32,
    on_accent: u32,
    fields: &'static [u32],
) -> PaletteSpec {
    PaletteSpec {
        dark,
        paper,
        card,
        ink,
        muted,
        accent,
        on_accent,
        fields,
    }
}

const CLASSICAL_PALETTES: &[PaletteSpec] = &[
    p(
        true,
        0x0E0E10,
        0x1A1A1E,
        0xF2EFEA,
        0x8A8790,
        0xFF2E88,
        0x0E0E10,
        &[],
    ),
    p(
        true,
        0x0B0F1A,
        0x151B2B,
        0xEEF2FF,
        0x8B93A7,
        0x2D7BFF,
        0x0B0F1A,
        &[],
    ),
    p(
        true,
        0x121212,
        0x1E1E1E,
        0xEDEDED,
        0x8C8C8C,
        0xC6F432,
        0x121212,
        &[],
    ),
    p(
        true,
        0x14110F,
        0x211C19,
        0xF3ECE2,
        0x9A8F84,
        0xFF6A1A,
        0x14110F,
        &[],
    ),
    p(
        false,
        0xE9E6E1,
        0xF5F3EF,
        0x161616,
        0x6E6A64,
        0xD4006E,
        0xFFFFFF,
        &[],
    ),
];
const HALFTONE_PALETTES: &[PaletteSpec] = &[
    p(
        false,
        0xF4EEE2,
        0xFFFFFF,
        0x111111,
        0x6B6B6B,
        0xE2231A,
        0xFFFFFF,
        &[0xFFD23F, 0xE2231A],
    ),
    p(
        false,
        0xFFD23F,
        0xFFF4C2,
        0x111111,
        0x5A4A12,
        0x1F3BFF,
        0xFFFFFF,
        &[0x1F3BFF, 0x111111],
    ),
    p(
        false,
        0xEDE6D6,
        0xF8F4EA,
        0x1A1A1A,
        0x707070,
        0x0F7B6C,
        0xFFFFFF,
        &[0x0F7B6C, 0xF2B705],
    ),
    p(
        true,
        0x161616,
        0x242424,
        0xF4F0E8,
        0x9C978E,
        0xFF3B30,
        0x161616,
        &[0xFF3B30, 0xF4F0E8],
    ),
];
const CLAY_PALETTES: &[PaletteSpec] = &[
    p(
        false,
        0xF7DCD4,
        0xFFF1EC,
        0x2B1D1A,
        0x8C6B63,
        0x13797A,
        0xFFFFFF,
        &[0xFFC94D, 0x8ED1C6],
    ),
    p(
        false,
        0xD6EFE3,
        0xF0FAF5,
        0x12302A,
        0x5E7D74,
        0xC63A24,
        0xFFFFFF,
        &[0xF6C445, 0x7FB3E8],
    ),
    p(
        false,
        0xFFEDB0,
        0xFFF8DC,
        0x2A2118,
        0x7A6A4F,
        0x6A4BD8,
        0xFFFFFF,
        &[0xFF8FB1, 0x6A4BD8],
    ),
    p(
        true,
        0x1F2A44,
        0x2A3758,
        0xFFF6E8,
        0xA9B4CF,
        0xFFB84D,
        0x1F2A44,
        &[0xFF8FB1, 0x7ED6C1],
    ),
];
const ORNAMENT_PALETTES: &[PaletteSpec] = &[
    p(
        false,
        0xF1E9DA,
        0xFBF6EC,
        0x1D1A16,
        0x7A6E60,
        0x7A1E2C,
        0xFBF6EC,
        &[],
    ),
    p(
        false,
        0xE7E3D8,
        0xF6F3EC,
        0x1F2321,
        0x6F756F,
        0x2F5D46,
        0xF6F3EC,
        &[],
    ),
    p(
        false,
        0xD9C3A0,
        0xEADBC2,
        0x2A1E14,
        0x6E5843,
        0x1B5E66,
        0xF5EBDD,
        &[],
    ),
    p(
        true,
        0x1C1A17,
        0x2A2622,
        0xF1E9DA,
        0xA39887,
        0xC9A227,
        0x1C1A17,
        &[],
    ),
];
const JOURNEY_PALETTES: &[PaletteSpec] = &[
    p(
        false,
        0xEEF1F4,
        0xFFFFFF,
        0x111827,
        0x64748B,
        0x1D4ED8,
        0xFFFFFF,
        &[],
    ),
    p(
        true,
        0x0B1320,
        0x142033,
        0xE6F0FF,
        0x8DA2BF,
        0x22D3EE,
        0x0B1320,
        &[],
    ),
    p(
        false,
        0xDCEBFA,
        0xF3F8FE,
        0x14213D,
        0x5B6B87,
        0xC2410C,
        0xFFFFFF,
        &[],
    ),
    p(
        true,
        0x0F3D3E,
        0x175556,
        0xF3F7F2,
        0x9DBDB9,
        0xF59E0B,
        0x0F3D3E,
        &[],
    ),
];

const STREET_PALETTES: &[PaletteSpec] = &[
    p(
        true,
        0x111111,
        0x1C1C1C,
        0xF5F1E8,
        0x9A948A,
        0xE10600,
        0xFFFFFF,
        &[0xFFD400],
    ),
    p(
        true,
        0x0E1116,
        0x171B22,
        0xF2F2F2,
        0x8B95A1,
        0xFFD400,
        0x0E1116,
        &[0xE10600],
    ),
    p(
        false,
        0xECE6DA,
        0xF7F3EA,
        0x141414,
        0x6B655C,
        0xD7261E,
        0xFFFFFF,
        &[0x141414],
    ),
    p(
        false,
        0xF2EFE8,
        0xFFFFFF,
        0x111111,
        0x5F5A52,
        0x1D4ED8,
        0xFFFFFF,
        &[0xE10600],
    ),
];
const DOSSIER_PALETTES: &[PaletteSpec] = &[
    p(
        true,
        0x15130F,
        0x23201A,
        0xF1EADB,
        0xA59C8A,
        0xF4D03F,
        0x15130F,
        &[],
    ),
    p(
        true,
        0x0F1A24,
        0x172634,
        0xEDE6D6,
        0x93A1AE,
        0xF2C14E,
        0x0F1A24,
        &[],
    ),
    p(
        false,
        0xEFE6D2,
        0xFAF5E9,
        0x1A1814,
        0x6E6657,
        0xB3261E,
        0xFFFFFF,
        &[],
    ),
    p(
        false,
        0xE9E4D8,
        0xF6F2E8,
        0x14110D,
        0x655D50,
        0x1F4E79,
        0xFFFFFF,
        &[],
    ),
];
// Studio: the disc colour is the first field (a big shape, not text, so it
// may be light on a light ground); accent/ink keep text contrast.
const STUDIO_PALETTES: &[PaletteSpec] = &[
    p(
        false,
        0xE8E8E4,
        0xF5F5F2,
        0x111111,
        0x5B5B57,
        0x1F4FD8,
        0xFFFFFF,
        &[0xF2DE1F],
    ),
    p(
        false,
        0xEDE6DE,
        0xF8F3EE,
        0x16120F,
        0x635A52,
        0xB3261E,
        0xFFFFFF,
        &[0xF08A5D],
    ),
    p(
        false,
        0xE6EBEF,
        0xF4F7F9,
        0x0F1720,
        0x55606B,
        0x0F766E,
        0xFFFFFF,
        &[0x7DD3C0],
    ),
    p(
        true,
        0x141414,
        0x1E1E1E,
        0xF4F4F0,
        0x9A9A96,
        0xF2DE1F,
        0x141414,
        &[0xF2DE1F],
    ),
];
// Cinematic: deep night grounds with one luminous accent; the fields tint
// the far glow and the foreground bokeh.
const CINEMATIC_PALETTES: &[PaletteSpec] = &[
    p(
        true,
        0x070B1A,
        0x0E1530,
        0xF2F5FF,
        0x8A94B8,
        0x22D3EE,
        0x070B1A,
        &[0x3B82F6, 0x8B5CF6],
    ),
    p(
        true,
        0x0E0A1F,
        0x1A1433,
        0xF6F2FF,
        0x9A90BC,
        0xF5A524,
        0x0E0A1F,
        &[0x8B5CF6, 0xF5A524],
    ),
    p(
        true,
        0x04151A,
        0x0A262D,
        0xEFFAF8,
        0x86A8A4,
        0x2DD4BF,
        0x04151A,
        &[0x0EA5E9, 0x2DD4BF],
    ),
    p(
        false,
        0xEEF1F6,
        0xFFFFFF,
        0x0B1220,
        0x56607A,
        0x1D4ED8,
        0xFFFFFF,
        &[0x93C5FD, 0xC4B5FD],
    ),
];
const HYPE_PALETTES: &[PaletteSpec] = &[
    p(
        true,
        0x050505,
        0x111111,
        0xFFFFFF,
        0x9A9A9A,
        0xE00000,
        0xFFFFFF,
        &[0x2E5BFF],
    ),
    p(
        true,
        0x0A0A1F,
        0x14143A,
        0xF5F5FF,
        0x9696C8,
        0x00E5FF,
        0x0A0A1F,
        &[0xFFE600],
    ),
    p(
        false,
        0xF4F4F0,
        0xFFFFFF,
        0x0A0A0A,
        0x5A5A5A,
        0xD62800,
        0xFFFFFF,
        &[0x0A0A0A],
    ),
    p(
        false,
        0xFFE14D,
        0xFFF1A8,
        0x0A0A0A,
        0x5C5200,
        0x0A0A0A,
        0xFFE14D,
        &[0xFF3B1F],
    ),
];

/// (0.23 W2e) The precision palettes: the technical tone's own family under
/// variety (graphite neutrals with a signal accent), so calm editorial and
/// technical never share look + palette + ground (DECISION 136). Read only
/// through [`precision_palette_for`].
pub const PRECISION_PALETTES: &[PaletteSpec] = &[
    p(
        false,
        0xF2F2EE,
        0xFFFFFF,
        0x121212,
        0x5A5A5A,
        0xFF4F00,
        0xFFFFFF,
        &[],
    ),
    p(
        true,
        0x121212,
        0x1C1C1C,
        0xF2F2EE,
        0x9A9A96,
        0xFF6A1F,
        0x121212,
        &[],
    ),
    p(
        false,
        0xE6E8E3,
        0xF7F7F4,
        0x0F1110,
        0x555B57,
        0x2E7D32,
        0xFFFFFF,
        &[],
    ),
    p(
        true,
        0x0F1412,
        0x18201D,
        0xE8EFEA,
        0x8FA39A,
        0xB6F23A,
        0x0F1412,
        &[],
    ),
];

/// A precision palette matching the polarity, picked by the variety seed.
pub fn precision_palette_for(dark: bool, seed: u64) -> PaletteSpec {
    let pool: Vec<&PaletteSpec> = PRECISION_PALETTES
        .iter()
        .filter(|p| p.dark == dark)
        .collect();
    *pool[(seed % pool.len() as u64) as usize]
}

/// The curated palettes of a look.
pub fn palettes(look: Look) -> &'static [PaletteSpec] {
    match look {
        Look::ClassicalNeon => CLASSICAL_PALETTES,
        Look::HalftoneCutout => HALFTONE_PALETTES,
        Look::ClayPop => CLAY_PALETTES,
        Look::OrnamentEditorial => ORNAMENT_PALETTES,
        Look::Journey => JOURNEY_PALETTES,
        Look::StreetCollage => STREET_PALETTES,
        Look::Dossier => DOSSIER_PALETTES,
        Look::HypeSlam => HYPE_PALETTES,
        Look::StudioPop => STUDIO_PALETTES,
        Look::Cinematic3d => CINEMATIC_PALETTES,
    }
}

/// Rule: the palettes of `look` matching the resolved polarity (all of
/// them when none match), picked by `seed` (the story's variety seed).
pub fn palette_for(look: Look, dark: bool, seed: u64) -> PaletteSpec {
    let all = palettes(look);
    let matching: Vec<&PaletteSpec> = all.iter().filter(|p| p.dark == dark).collect();
    let pool: Vec<&PaletteSpec> = if matching.is_empty() {
        all.iter().collect()
    } else {
        matching
    };
    *pool[(seed % pool.len() as u64) as usize]
}

// ---------------------------------------------------------------------------
// (0.23 A4) Variety palettes: text colours that read on the grounds of the look
// ---------------------------------------------------------------------------

/// The darkest and the lightest pixel colour (10th and 90th percentile of the
/// luminance) of each `grounds` plate, measured on the library's PNGs. The plate
/// sits over the palette paper at the look's plate opacity, so these bound the
/// ground a text colour can sit on. A plate not listed is treated as absent.
const PLATE_EXTREMES: &[(&str, u32, u32)] = &[
    ("black_grain", 0x060605, 0x0F100C),
    ("blush", 0xFDD5C4, 0xFEDBCB),
    ("concrete", 0x99989A, 0xAAACAA),
    ("deep_teal", 0x014645, 0x035350),
    ("grey_studio", 0xACACAE, 0xCECFD1),
    ("halftone_dark", 0x1F2022, 0x222224),
    ("ink_wash_dark", 0x030C1F, 0x152649),
    ("kraft", 0xAF845E, 0xC19D76),
    ("linen", 0xD6CDC1, 0xEBE6D6),
    ("neon_purple_gradient", 0x020118, 0x5903EF),
    ("sky_gradient", 0xEDC1A4, 0xE9ECEC),
    ("warm_paper", 0xEAE0CB, 0xF6EDD8),
];

/// The paper of `spec` as the plate `(id, opacity)` leaves it at its worst: on
/// a light palette the plate's darkest colour over the paper, on a dark palette
/// its lightest. `None` when there is no plate (or no measurement of it).
pub fn ground_under_plate(spec: &PaletteSpec, plate: Option<(&str, f32)>) -> Option<u32> {
    let (id, opacity) = plate?;
    let (_, darkest, lightest) = PLATE_EXTREMES.iter().find(|(p, _, _)| *p == id)?;
    let over = if spec.dark { *lightest } else { *darkest };
    let (a, b) = (rgb_of(spec.paper), rgb_of(over));
    let o = opacity.clamp(0.0, 1.0);
    let mix = |p: u8, q: u8| (f32::from(p) * (1.0 - o) + f32::from(q) * o).round() as u8;
    Some(hex_of([mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2])]))
}

fn rgb_of(hex: u32) -> [u8; 3] {
    [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8]
}

fn hex_of(c: [u8; 3]) -> u32 {
    u32::from(c[0]) << 16 | u32::from(c[1]) << 8 | u32::from(c[2])
}

fn contrast_hex(a: u32, b: u32) -> f32 {
    let (a, b) = (rgb_of(a), rgb_of(b));
    super::explore::contrast_ratio(
        crate::scene::Color::rgb(a[0], a[1], a[2]),
        crate::scene::Color::rgb(b[0], b[1], b[2]),
    )
}

/// RGB (0..=255) to hue (degrees), saturation and lightness (both 0..=1).
fn to_hsl(c: [u8; 3]) -> (f64, f64, f64) {
    let (r, g, b) = (
        f64::from(c[0]) / 255.0,
        f64::from(c[1]) / 255.0,
        f64::from(c[2]) / 255.0,
    );
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d < 1e-12 {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h * 60.0, s, l)
}

fn from_hsl(h: f64, s: f64, l: f64) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0).floor().rem_euclid(6.0) as u8 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |v: f64| ((v + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    [q(r), q(g), q(b)]
}

/// Spare contrast a text colour is given over its limit, for the grain and the
/// print overlay the rendered check sees on top of the ground.
const TOKEN_MARGIN: f32 = 1.08;

/// `color` moved toward `ink` in lightness (hue and saturation kept) until it
/// reaches `need` : 1 against every ground; unchanged when it already does.
/// Deterministic: a bisection over the share of the way to the ink's lightness.
/// `keep` = `(other, min)` bounds the move to colours that still contrast with
/// `other` by `min` (an accent must still carry its on-accent text); when the
/// bound stops the move short, the colour goes as far as it may.
fn reach(color: u32, ink: u32, grounds: &[u32], need: f32, keep: Option<(u32, f32)>) -> u32 {
    let worst = |c: u32| {
        grounds
            .iter()
            .map(|&g| contrast_hex(c, g))
            .fold(f32::INFINITY, f32::min)
    };
    let need = need * TOKEN_MARGIN;
    if worst(color) >= need {
        return color;
    }
    let (h, s, l0) = to_hsl(rgb_of(color));
    let (_, _, l_ink) = to_hsl(rgb_of(ink));
    let at = |t: f64| hex_of(from_hsl(h, s, l0 + (l_ink - l0) * t));
    let allowed = |c: u32| keep.is_none_or(|(other, min)| contrast_hex(c, other) >= min);
    // The furthest the bound lets the colour go.
    let far = if allowed(at(1.0)) {
        1.0
    } else {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if allowed(at(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    };
    if worst(at(far)) < need {
        // As far as it may go and still short: the ink itself when nothing
        // bounds the move (even at the ink's lightness this hue cannot).
        return if keep.is_none() { ink } else { at(far) };
    }
    let (mut lo, mut hi) = (0.0, far);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if worst(at(mid)) >= need {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    at(hi)
}

/// The palette a `--variety` compile uses: the curated `spec` with its text
/// colours adjusted so they read on every ground the look puts them on.
///
/// `muted` (small text) must reach [`crate::checks::TEXT_CONTRAST_BODY`] and `accent` (display
/// numbers and words) [`crate::checks::TEXT_CONTRAST_DISPLAY`] against the card, the paper and
/// the paper under the plate at its worst ([`ground_under_plate`]). A colour that
/// does not is moved toward the ink in lightness, keeping its hue, by the least
/// that does; the accent never moves so far that its on-accent text drops under
/// [`crate::checks::TEXT_CONTRAST_BODY`]. The ink, the paper, the card, the on-accent colour and
/// the fields are never touched, and a palette that already reads is returned
/// as it is.
/// Without variety the curated palette is used as it is (byte-identical output).
pub fn variety_text_tokens(spec: &PaletteSpec, plate: Option<(&str, f32)>) -> PaletteSpec {
    let mut grounds = vec![spec.card, spec.paper];
    grounds.extend(ground_under_plate(spec, plate));
    PaletteSpec {
        muted: reach(
            spec.muted,
            spec.ink,
            &grounds,
            crate::checks::TEXT_CONTRAST_BODY as f32,
            None,
        ),
        accent: reach(
            spec.accent,
            spec.ink,
            &grounds,
            crate::checks::TEXT_CONTRAST_DISPLAY as f32,
            Some((spec.on_accent, crate::checks::TEXT_CONTRAST_BODY as f32)),
        ),
        ..*spec
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    fn lum(c: u32) -> f64 {
        let ch = |v: u32| {
            let s = v as f64 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * ch(c >> 16 & 255) + 0.7152 * ch(c >> 8 & 255) + 0.0722 * ch(c & 255)
    }
    fn ratio(a: u32, b: u32) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn palettes_meet_contrast() {
        for look in Look::ALL {
            for (i, pal) in palettes(look).iter().enumerate() {
                let tag = format!("{} #{i}", look.name());
                assert!(ratio(pal.ink, pal.paper) >= 7.0, "{tag} ink/paper");
                assert!(ratio(pal.ink, pal.card) >= 7.0, "{tag} ink/card");
                assert!(
                    ratio(pal.on_accent, pal.accent) >= 4.5,
                    "{tag} on_accent/accent {:.2}",
                    ratio(pal.on_accent, pal.accent)
                );
                assert!(
                    ratio(pal.accent, pal.paper) >= 3.0,
                    "{tag} accent/paper {:.2}",
                    ratio(pal.accent, pal.paper)
                );
                assert!(
                    ratio(pal.muted, pal.paper) >= 3.0,
                    "{tag} muted/paper {:.2}",
                    ratio(pal.muted, pal.paper)
                );
                assert_eq!(lum(pal.paper) < 0.2, pal.dark, "{tag} dark flag");
            }
            assert!(palettes(look).len() >= 4);
        }
    }

    #[test]
    fn palette_choice_follows_polarity_and_seed() {
        let a = palette_for(Look::ClayPop, false, 0);
        let b = palette_for(Look::ClayPop, false, 1);
        assert_ne!(a, b);
        assert!(!a.dark && !b.dark);
        assert!(palette_for(Look::ClayPop, true, 7).dark);
        assert_eq!(
            palette_for(Look::Journey, false, 5),
            palette_for(Look::Journey, false, 5)
        );
    }

    // -- (0.23 A4) variety text tokens ----------------------------------------

    /// Every (plate, opacity) a compile can draw under a palette: none, and
    /// each measured plate at the opacities the looks use.
    fn plates() -> Vec<Option<(&'static str, f32)>> {
        let mut out = vec![None];
        for (id, _, _) in PLATE_EXTREMES {
            for opacity in [0.3, 0.42, 0.55] {
                out.push(Some((*id, opacity)));
            }
        }
        out
    }

    fn hue_gap(a: u32, b: u32) -> f64 {
        let d = (to_hsl(rgb_of(a)).0 - to_hsl(rgb_of(b)).0).abs();
        d.min(360.0 - d)
    }

    #[test]
    fn variety_text_tokens_read_on_every_ground() {
        for look in Look::ALL {
            for (i, spec) in palettes(look).iter().enumerate() {
                for plate in plates() {
                    // A compile never draws a light plate under a dark palette
                    // (or the reverse): the plate follows the polarity.
                    let dark_plates = [
                        "black_grain",
                        "deep_teal",
                        "halftone_dark",
                        "ink_wash_dark",
                        "neon_purple_gradient",
                    ];
                    if plate.is_some_and(|(id, _)| dark_plates.contains(&id) != spec.dark) {
                        continue;
                    }
                    let tag = format!("{} #{i} {plate:?}", look.name());
                    let t = variety_text_tokens(spec, plate);
                    let mut grounds = vec![spec.card, spec.paper];
                    grounds.extend(ground_under_plate(spec, plate));
                    for &g in &grounds {
                        assert!(
                            ratio(t.muted, g) >= 4.5,
                            "{tag} muted {:06X} on {g:06X} {:.2}",
                            t.muted,
                            ratio(t.muted, g)
                        );
                        // (Unless the accent cannot get there and keep carrying
                        // its on-accent text: that bound wins.)
                        let free = reach(spec.accent, spec.ink, &grounds, 3.0, None);
                        if ratio(spec.on_accent, free) >= 4.5 {
                            assert!(
                                ratio(t.accent, g) >= 3.0,
                                "{tag} accent {:06X} on {g:06X} {:.2}",
                                t.accent,
                                ratio(t.accent, g)
                            );
                        }
                    }
                    // Only the two text colours move; the accent still carries
                    // its on-accent text.
                    assert_eq!(
                        (t.dark, t.paper, t.card, t.ink, t.on_accent, t.fields),
                        (
                            spec.dark,
                            spec.paper,
                            spec.card,
                            spec.ink,
                            spec.on_accent,
                            spec.fields
                        ),
                        "{tag}"
                    );
                    assert!(
                        ratio(t.on_accent, t.accent) >= 4.5,
                        "{tag} on_accent/accent {:.2}",
                        ratio(t.on_accent, t.accent)
                    );
                    // Hue kept (the ink itself is the fallback of last resort).
                    for (new, old) in [(t.muted, spec.muted), (t.accent, spec.accent)] {
                        if new != old && new != spec.ink && to_hsl(rgb_of(old)).1 > 0.1 {
                            assert!(hue_gap(new, old) < 4.0, "{tag} {new:06X} vs {old:06X}");
                        }
                    }
                    // Idempotent: a palette that reads is returned as it is.
                    assert_eq!(variety_text_tokens(&t, plate), t, "{tag}");
                }
            }
        }
    }

    #[test]
    fn variety_text_tokens_leave_a_palette_that_reads() {
        // Halftone palette 1 (yellow paper, #5A4A12 muted, blue accent) already
        // reads on its paper and card: nothing moves without a plate.
        assert_eq!(
            variety_text_tokens(&HALFTONE_PALETTES[1], None),
            HALFTONE_PALETTES[1]
        );
        // Palette 0 needs a darker muted under the concrete plate, and gets the
        // same answer every time.
        let plate = Some(("concrete", 0.42));
        let t = variety_text_tokens(&HALFTONE_PALETTES[0], plate);
        assert_ne!(t.muted, HALFTONE_PALETTES[0].muted);
        assert_eq!(t, variety_text_tokens(&HALFTONE_PALETTES[0], plate));
    }

    #[test]
    fn the_plate_under_halftone_palette_0_is_what_the_pixel_check_saw() {
        // concrete at 0.42 over #F4EEE2: the ring around text measured #D1CEC7.
        let g = ground_under_plate(&HALFTONE_PALETTES[0], Some(("concrete", 0.42))).expect("g");
        let (a, b) = (rgb_of(g), rgb_of(0xD1CEC7));
        assert!(
            a.iter()
                .zip(b)
                .all(|(x, y)| (i32::from(*x) - i32::from(y)).abs() <= 12),
            "{g:06X}"
        );
        assert_eq!(ground_under_plate(&HALFTONE_PALETTES[0], None), None);
        assert_eq!(
            ground_under_plate(&HALFTONE_PALETTES[0], Some(("nope", 0.4))),
            None
        );
    }

    #[test]
    fn every_plate_the_looks_use_is_measured() {
        for e in Emotion::ALL {
            let d = art_direction_for(e);
            for id in [d.plate_dark, d.plate_light] {
                assert!(
                    PLATE_EXTREMES.iter().any(|(p, _, _)| *p == id),
                    "{e:?} {id}"
                );
            }
        }
        for look in Look::ALL {
            let d = look_defaults(look);
            for id in [d.plate_dark, d.plate_light] {
                assert!(
                    PLATE_EXTREMES.iter().any(|(p, _, _)| *p == id),
                    "{look:?} {id}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// (0.10 Q) Applied art direction: record + palette conversion
// ---------------------------------------------------------------------------

/// What art direction a project was compiled with (`ProjectMeta.art`). The
/// audio planner reads `sfx`; tooling reads the rest.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ArtRecord {
    pub look: String,
    /// Index into `palettes(look)`.
    pub palette: usize,
    /// `grounds` plate id, when one is drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plate: Option<String>,
    pub families: Vec<String>,
    pub sfx: String,
    /// (0.18) Beat scene id -> the layer the beat is about (its focal
    /// element: the amount, the hero picture). The camera focuses on it, the
    /// emphasis lands on it and layout QA checks both.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub focal: std::collections::BTreeMap<String, String>,
    /// (0.20) Beat scene id -> the reveal anchors its builder declared
    /// (`B::reveal`): which layer groups stand for which spoken words, and
    /// their roles. Word cues move them; speech QA (`reveal_before_speech`,
    /// `reveal_late`, `spoken_mismatch`) judges them. Absent outside `--art`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub reveals: std::collections::BTreeMap<String, Vec<crate::speech::RevealAnchor>>,
}

impl SfxPalette {
    pub fn name(self) -> &'static str {
        match self {
            SfxPalette::Soft => "soft",
            SfxPalette::Crisp => "crisp",
            SfxPalette::Bouncy => "bouncy",
            SfxPalette::Punchy => "punchy",
            SfxPalette::Cinematic => "cinematic",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        [
            SfxPalette::Soft,
            SfxPalette::Crisp,
            SfxPalette::Bouncy,
            SfxPalette::Punchy,
            SfxPalette::Cinematic,
        ]
        .into_iter()
        .find(|p| p.name() == s)
    }
}

fn color(hex: u32) -> crate::scene::Color {
    crate::scene::Color::rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Engine palette from a curated spec.
pub fn palette_of(spec: &PaletteSpec) -> super::theme::Palette {
    super::theme::Palette {
        paper: color(spec.paper),
        card: color(spec.card),
        ink: color(spec.ink),
        muted: color(spec.muted),
        accent: color(spec.accent),
        on_accent: color(spec.on_accent),
        fields: spec.fields.iter().map(|&f| color(f)).collect(),
    }
}

/// Index of `spec` within `palettes(look)` (for the record).
pub fn palette_index(look: Look, spec: &PaletteSpec) -> usize {
    palettes(look).iter().position(|p| p == spec).unwrap_or(0)
}

/// Resolve a mode against the story's emotion.
pub fn resolve(mode: ArtMode, emotion: Emotion) -> ArtDirection {
    resolve_with_genre(mode, emotion, super::taste::Genre::Standard)
}

/// (0.14) `--art auto` with a genre tone picks the genre's look; standard
/// stories keep the emotion table; a forced look always wins.
pub fn resolve_with_genre(
    mode: ArtMode,
    emotion: Emotion,
    genre: super::taste::Genre,
) -> ArtDirection {
    match mode {
        ArtMode::Auto => match genre_look(genre) {
            Some(look) => look_defaults(look),
            None => art_direction_for(emotion),
        },
        ArtMode::Force(look) => look_defaults(look),
    }
}
