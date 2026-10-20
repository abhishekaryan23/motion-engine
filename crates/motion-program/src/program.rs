//! The MotionProgram schema (v2.0): what a strong model or a recipe writes.
//!
//! A program is closed and declarative: components with a preset, content and
//! an explicit timeline of verbs. Event times are seconds from the
//! component's start; every verb's duration, easing and spring come from the
//! taste tables (`taste.rs`) scaled by the program `tempo`. Nothing is in
//! pixels: placement is a [`Slot`], size a [`Size`], colour a [`Swatch`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub use crate::bento::BentoPreset;
pub use crate::lockup::LockupPreset;

pub const PROGRAM_VERSION: &str = "2.0";

/// A MotionEngine 2 program: palette, type, tempo and components on one
/// explicit timeline. Compiles deterministically to a MotionScene.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MotionProgram {
    /// Schema version, `"2.0"`.
    pub version: String,
    pub meta: Meta,
    /// A style recipe: palette, type, tempo and allowances in one name.
    /// Explicit `palette`/`fonts`/`tempo`/`allow` override it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<Recipe>,
    /// Required unless a recipe gives one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
    /// Type faces per role; default: the recipe's, else the heavy grotesk +
    /// serif italic + mono set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<Fonts>,
    /// Global speed of every verb: slow ×1.35, normal ×1, fast ×0.75.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<Tempo>,
    /// Opt-in features that the taste rules ban by default (added to the
    /// recipe's).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<Feature>,
    /// A bundled music bed; its offline beat analysis lets events and
    /// components use `beat` instead of seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<Music>,
    /// Drawn back to front in list order.
    pub components: Vec<Component>,
    /// Program camera: pushes, pans and framing over every component
    /// (grounds stay put), on the same clock as the timeline.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub camera: Vec<CameraEvent>,
    /// Shot transitions at program times (or beats).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<TransitionEvent>,
    /// A film finish over the whole program: faint grain and a soft
    /// chromatic fringe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub film: Option<Film>,
    /// Sound design without a beat clock: a bed and SFX hits at program
    /// seconds (a beat-led program uses `music` instead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<SoundDesign>,
    /// Images generated before rendering, from a prompt. Any image path in
    /// the program may name one as `"gen:<id>"`. Generated once per
    /// (prompt, kind, aspect, seed) and cached; the render never waits on a
    /// network or model call.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<GenAsset>,
}

/// One image to generate before rendering (see `MotionProgram.assets`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenAsset {
    /// Referenced as `"gen:<id>"`; lowercase letters, digits and `_`.
    pub id: String,
    /// What to draw: subject, medium and light, no on-screen words.
    pub prompt: String,
    #[serde(default)]
    pub kind: GenKind,
    /// Canvas shape (default: the program's aspect for `plate`, 1:1 for `cutout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<Aspect>,
    /// Explicit seed: the same prompt and seed give the same image.
    #[serde(default)]
    pub seed: u32,
}

/// What a generated asset is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GenKind {
    /// A full-frame picture or texture (backdrop image, image-filled type).
    #[default]
    Plate,
    /// One isolated object with a transparent background.
    Cutout,
}

/// Faint, constant film texture (applied after everything else).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Film {
    /// Grain amount 0..=0.2 (0.04 is faint).
    #[serde(default)]
    pub grain: f32,
    /// Red/blue fringe in px at 1080 p, 0..=3.
    #[serde(default)]
    pub fringe: f32,
}

/// A bed and hits, placed by hand at program seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoundDesign {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bed: Option<Bed>,
    /// SFX hits, each peaking at `at`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hits: Vec<Hit>,
}

/// A bundled track (`assets/music/<track>.mp3`) played from `start` seconds
/// into the file; it fades out over the last `fade_out` seconds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bed {
    pub track: String,
    #[serde(default)]
    pub start: f64,
    /// Level offset on the track's own analysed gain (dB).
    #[serde(default)]
    pub gain_db: f64,
    #[serde(default = "Bed::default_fade_out")]
    pub fade_out: f64,
}

impl Bed {
    fn default_fade_out() -> f64 {
        1.0
    }
}

/// One SFX hit: `sound` is a library family (`hit_hard`, `whoosh_soft`,
/// `subdrop`, ...) or an exact sound id (`hit_hard_03`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Hit {
    pub at: f64,
    pub sound: String,
    /// Offset from the family's target peak (dB).
    #[serde(default)]
    pub gain_db: f64,
}

/// A camera move at program seconds `at` or music `beat`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CameraEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<u32>,
    #[serde(flatten)]
    pub op: CameraVerb,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum CameraVerb {
    /// Zoom to this factor (1..=1.8) about the current centre.
    Push { to: f32 },
    /// Centre the frame on a slot.
    Pan { to: Slot },
    /// Frame a component: centre it and zoom so it fills ~70 % of the frame.
    Frame { component: String },
    /// 2.5D camera orbit `[yaw_deg, pitch_deg]` in `-35..=35`.
    Orbit {
        #[serde(default)]
        yaw: f32,
        #[serde(default)]
        pitch: f32,
    },
    /// 2.5D camera dolly along Z in `-600..=600` px (negative pushes in, positive pulls back).
    Dolly { to: f32 },
    /// Dutch angle camera roll in `-25..=25` degrees.
    Roll { deg: f32 },
    /// Rack depth-of-field focus to `z` px with optional `aperture` (`0.0..=1.0`).
    Focus {
        z: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        aperture: Option<f32>,
    },
    /// Back to the full frame.
    Reset,
}

/// A transition at program seconds `at` or music `beat` (the cut point).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TransitionEvent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<u32>,
    pub kind: TransitionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// The camera whips sideways out of one shot and into the next, with
    /// directional blur at the cut.
    Whip,
    /// An accent panel wipes across and off, carrying the eye over the cut.
    ColourFlip,
    /// A landing: a two-frame shockwave ring from the frame centre, a short
    /// motion blur and a decaying camera shake.
    Impact,
}

/// A bundled track (`assets/music/<track>.mp3` with its `.music.json`
/// analysis). Beat `n` is the n-th analysed beat at or after `start`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Music {
    /// Track name without extension, e.g. `driving_pulse`.
    pub track: String,
    /// Seconds into the track where the program starts.
    #[serde(default)]
    pub start: f64,
}

/// Named style recipes (`assets/recipes/<name>.recipe.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Recipe {
    /// Off-white, black and one orange; heavy grotesk; fast cuts.
    PosterKinetic,
    /// Dark navy, thin axes, one gold series.
    ThinAxis,
    /// Cream ground, one orange dot, muted dot fields.
    OrangeDot,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Meta {
    /// File-safe name: lowercase letters, digits and `_`.
    pub name: String,
    /// Frame shape; layout adapts (see `Slot`).
    #[serde(default)]
    pub aspect: Aspect,
    /// Total length in seconds (0 < duration <= 120).
    pub duration: f64,
    /// Frames per second.
    #[serde(default)]
    pub fps: Fps,
    /// With music: end on this beat (the final hit). `duration` becomes
    /// that beat plus most of one beat of hold, so no beat plays after it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_beat: Option<u32>,
    /// The request this program answers (direction, not script): QA fails
    /// on-screen words copied from it unless they were quoted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Aspect {
    /// 1920×1080.
    #[default]
    #[serde(rename = "16:9")]
    Landscape,
    /// 1080×1920.
    #[serde(rename = "9:16")]
    Portrait,
    /// 1440×1440.
    #[serde(rename = "1:1")]
    Square,
}

impl Aspect {
    pub fn size(self) -> (u32, u32) {
        match self {
            Aspect::Landscape => (1920, 1080),
            Aspect::Portrait => (1080, 1920),
            Aspect::Square => (1440, 1440),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Fps {
    #[default]
    #[serde(rename = "30")]
    Thirty,
    #[serde(rename = "60")]
    Sixty,
}

impl Fps {
    pub fn value(self) -> u32 {
        match self {
            Fps::Thirty => 30,
            Fps::Sixty => 60,
        }
    }
}

/// Four colours, `#RRGGBB`. Components name a swatch, never a colour, so a
/// program has exactly one accent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Palette {
    pub background: String,
    pub ink: String,
    pub accent: String,
    pub muted: String,
    /// Up to 4 extra full-frame grounds (`ground1`..`ground4`): sections
    /// can each live in their own colour world while keeping one accent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grounds: Vec<String>,
}

/// A palette entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Swatch {
    Background,
    Ink,
    Accent,
    Muted,
    Ground1,
    Ground2,
    Ground3,
    Ground4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    #[serde(default = "Face::default_display")]
    pub display: Face,
    #[serde(default = "Face::default_serif")]
    pub serif: Face,
    #[serde(default = "Face::default_mono")]
    pub mono: Face,
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts {
            display: Face::default_display(),
            serif: Face::default_serif(),
            mono: Face::default_mono(),
        }
    }
}

/// Bundled faces (assets/fonts, each under its own licence; restricted faces fall back to open ones, see assets/fonts/RESTRICTED.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Face {
    ArchivoBlack,
    PoppinsBlack,
    Anton,
    CabinetGroteskExtrabold,
    ClashDisplayBold,
    PanchangExtrabold,
    BebasNeue,
    BarlowCondensedBold,
    SatoshiBold,
    InstrumentSerifItalic,
    InstrumentSerifRegular,
    DmSerifItalic,
    DmSerifRegular,
    ZodiakItalic,
    ZodiakBold,
    ZillaSlabBold,
    IbmPlexMono,
    SpaceMono,
    SpaceMonoBold,
}

impl Face {
    fn default_display() -> Face {
        Face::ArchivoBlack
    }
    fn default_serif() -> Face {
        Face::InstrumentSerifItalic
    }
    fn default_mono() -> Face {
        Face::IbmPlexMono
    }

    /// `(asset id, file under assets/)`.
    pub fn asset(self) -> (&'static str, &'static str) {
        match self {
            Face::ArchivoBlack => ("font.archivo_black", "fonts/ArchivoBlack-Regular.ttf"),
            Face::PoppinsBlack => ("font.poppins_black", "fonts/Poppins-Black.ttf"),
            Face::Anton => ("font.anton", "fonts/Anton-Regular.ttf"),
            Face::CabinetGroteskExtrabold => (
                "font.cabinet_grotesk_extrabold",
                "fonts/CabinetGrotesk-Extrabold.otf",
            ),
            Face::ClashDisplayBold => ("font.clash_display_bold", "fonts/ClashDisplay-Bold.otf"),
            Face::PanchangExtrabold => ("font.panchang_extrabold", "fonts/Panchang-Extrabold.otf"),
            Face::BebasNeue => ("font.bebas_neue", "fonts/BebasNeue-Regular.ttf"),
            Face::BarlowCondensedBold => (
                "font.barlow_condensed_bold",
                "fonts/BarlowCondensed-Bold.ttf",
            ),
            Face::SatoshiBold => ("font.satoshi_bold", "fonts/Satoshi-Bold.otf"),
            Face::InstrumentSerifItalic => (
                "font.instrument_serif_italic",
                "fonts/InstrumentSerif-Italic.ttf",
            ),
            Face::InstrumentSerifRegular => (
                "font.instrument_serif_regular",
                "fonts/InstrumentSerif-Regular.ttf",
            ),
            Face::DmSerifItalic => ("font.dm_serif_italic", "fonts/DMSerifDisplay-Italic.ttf"),
            Face::DmSerifRegular => ("font.dm_serif_regular", "fonts/DMSerifDisplay-Regular.ttf"),
            Face::ZodiakItalic => ("font.zodiak_italic", "fonts/Zodiak-Italic.otf"),
            Face::ZodiakBold => ("font.zodiak_bold", "fonts/Zodiak-Bold.otf"),
            Face::ZillaSlabBold => ("font.zilla_slab_bold", "fonts/ZillaSlab-Bold.ttf"),
            Face::IbmPlexMono => ("font.plex_mono", "fonts/IBMPlexMono-Medium.ttf"),
            Face::SpaceMono => ("font.space_mono", "fonts/SpaceMono-Regular.ttf"),
            Face::SpaceMonoBold => ("font.space_mono_bold", "fonts/SpaceMono-Bold.ttf"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tempo {
    Slow,
    #[default]
    Normal,
    Fast,
}

/// Features banned unless listed in `allow`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// Gradient backdrops.
    Gradient,
}

/// One component: on screen from `start` to `end` (default: the program
/// end), with a kind-specific preset and content, animated by `timeline`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Component {
    /// Unique id: lowercase letters, digits and `_`.
    pub id: String,
    /// Seconds from the program start.
    #[serde(default)]
    pub start: f64,
    /// Start on this music beat instead (overrides `start`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_beat: Option<u32>,
    /// End on this music beat (cut there, on the same frame grid as the
    /// next shot's start; overrides `end`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_beat: Option<u32>,
    /// Seconds from the program start; absent = the program end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    /// Optional 2.5D parallax depth factor (`0.0` = pinned ground, `1.0` = subject plane, `>1.0` = foreground).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
    /// Optional true 2.5D Z coordinate in px (`0.0` = focus plane, `>0.0` = farther, `<0.0` = closer).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<f32>,
    /// Optional static 2.5D projective card tilt `[pitch_deg, yaw_deg]` in `-65..=65`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<[f32; 2]>,
    /// Intentional layering: the id of a component this one is composited
    /// over (drawn above it). Exempts that pair from `component_overlap`;
    /// every other overlap is still a defect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<String>,
    #[serde(flatten)]
    pub body: ComponentBody,
    /// Verbs in time order; `at` is seconds from the component start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub timeline: Vec<Event>,
}

/// Phase 1 components. Richer ones (charts, kinetic text, diagrams, device
/// frames, ...) arrive in later phases as new kinds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ComponentBody {
    /// Full-frame ground.
    Backdrop {
        #[serde(default)]
        preset: BackdropPreset,
        /// Ground colour (default: the background swatch).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
        /// Optional relative path under `assets/` to a background image plate
        /// (e.g. `"generated/idea_1_bg.png"`), composited over the ground colour.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image: Option<String>,
        /// Opacity of the background image plate (`0.05..=1.0`, default `0.24`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        image_opacity: Option<f32>,
    },
    /// One hero primitive that keeps its identity and morphs.
    Shape {
        #[serde(default)]
        preset: ShapePreset,
        shape: ShapeSpec,
        #[serde(default)]
        slot: Slot,
        #[serde(default)]
        size: Size,
        /// Default: accent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
        /// The component this shape marks (it is centred on that
        /// component's slot). A shape that neither changes (morph, move,
        /// orbit, bounce, fill, spin) nor anchors to content is decoration
        /// and is rejected (`slop_meaningless_shape`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        anchor: Option<String>,
        /// How an anchored shape marks its anchor's measured bounds: a ring
        /// around them (≤ 1.12×), an underline, or a marker dot at the edge.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mark: Option<Mark>,
    },
    /// A short line of type.
    Title {
        #[serde(default)]
        preset: TitlePreset,
        /// At most 60 characters (24 at size `xl`).
        text: String,
        #[serde(default)]
        slot: Slot,
        #[serde(default)]
        size: Size,
        /// Default: ink.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
        /// Type treatment: fill (solid, gradient, image), outline, letter case.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<TypeStyle>,
    },
    /// Big type, a few words per frame, advanced by `cut` verbs.
    KineticText {
        #[serde(default)]
        preset: KineticPreset,
        /// Shown one after another (1..=40 items of 1..=24 characters);
        /// the first at the component start, each `cut` shows the next.
        words: Vec<String>,
        /// With music: cut to the next word every this many beats from the
        /// component's `start_beat` (adds `cut` events; explicit cuts stay).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beat_cuts: Option<u32>,
        /// Indices of words set in the accent.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stress: Vec<usize>,
        /// Poster ground swatch (default: background; `flip` swaps it with
        /// the type colour).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ground: Option<Swatch>,
        /// An accent dot ends every word like a full stop and hops to the
        /// next word on each cut (one element carried across the cuts).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        dot: bool,
        #[serde(default)]
        slot: Slot,
        /// Default: ink.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
        /// Type treatment: fill (solid, gradient, image), outline, letter case.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        style: Option<TypeStyle>,
    },
    /// A chart with one hero value.
    Chart {
        #[serde(default)]
        preset: ChartPreset,
        #[serde(default)]
        chart: ChartKind,
        /// 2..=24 values (donut: 2..=8, non-negative).
        values: Vec<f64>,
        /// Empty, or one short label per value.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        labels: Vec<String>,
        /// The value the chart is about (dot, accent, callout).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hero: Option<usize>,
        /// Big number shown at the hero, at most 12 characters.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        callout: Option<String>,
        /// Series and hero colour (default: the preset's; set it when the
        /// chart sits on an accent-coloured ground).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
        /// What the chart measures, set above it (at most 32 characters,
        /// e.g. "Weekly signups, thousands").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        /// With music: reveal one value per this many beats from
        /// `start_beat` (bar i on beat i), then the hero lands.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beat_cuts: Option<u32>,
        #[serde(default)]
        slot: Slot,
        #[serde(default)]
        size: Size,
    },
    /// Nodes and edges built step by step: each `advance` reveals the next
    /// node (its edge draws first) and an accent dot travels the edge.
    Diagram {
        #[serde(default)]
        preset: DiagramPreset,
        /// 2..=8 node labels (at most 18 characters each).
        nodes: Vec<String>,
        /// `[from, to]` node indices; default: a chain 0→1→2…
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        edges: Vec<[usize; 2]>,
        /// With music: advance every this many beats from `start_beat`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beat_cuts: Option<u32>,
        /// Node outline/fill and dot colour override (default: preset's).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
        #[serde(default)]
        slot: Slot,
    },
    /// Frame furniture: up to four tiny mono labels in the corners.
    Hud {
        /// `[top-left, top-right, bottom-left, bottom-right]`; empty strings
        /// leave a corner blank. At most 24 characters each.
        corners: Vec<String>,
    },
    /// Mono code typed in with a caret (`steps`: type, retype, dissolve, highlight).
    CodeBlock(crate::code::CodeBlock),
    /// A phone, laptop or bare stage with a morphing UI container, a cursor and a camera.
    DeviceFrame(crate::device::DeviceFrame),
    /// A dense, structured metric and feature card grid with 2.5D card tilt and step reveals.
    Bento(crate::bento::Bento),
    /// A closing typographic lockup: headline, a line under it and the
    /// rules that frame them.
    Lockup(crate::lockup::Lockup),
    /// An editorial engraving / cutout collage sequence with hand-drawn strokes and word-pair morphs.
    PhotoStack(crate::photo::PhotoStack),
    /// A `p5.brush` natural media sketch, flow field, hatched drafting composition, or Golden Spiral study.
    Sketch(crate::sketch::Sketch),
    /// A full-frame field of dots (one instanced primitive).
    Field {
        #[serde(default)]
        preset: FieldPreset,
        /// Halftone: where the dots are largest (default: the centre).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        focus: Option<Slot>,
        /// Default: muted.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        colour: Option<Swatch>,
    },
}

/// How a line of type is painted. Absent = solid colour, the preset's case.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TypeStyle {
    #[serde(default)]
    pub fill: TypeFill,
    /// Stroke the glyph outlines (in the fill colour) instead of filling them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outline: bool,
    #[serde(default)]
    pub case: TypeCase,
}

/// The paint inside the glyphs.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypeFill {
    /// The component's colour.
    #[default]
    Solid,
    /// A linear gradient between two swatches across the line's box.
    Gradient {
        from: Swatch,
        to: Swatch,
        /// Gradient direction in degrees (0 = left to right).
        #[serde(default)]
        angle_deg: f32,
    },
    /// An image seen through the glyphs (a text mask), cover-fit to the
    /// line's box. Relative path under `assets/`.
    Image { image: String },
}

/// Letter case of displayed type.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TypeCase {
    /// The preset's case (most presets set display type in capitals).
    #[default]
    Preset,
    /// Exactly as written.
    AsWritten,
    Upper,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KineticPreset {
    /// One giant word per frame on a full-frame ground; hard cuts, slow push.
    #[default]
    PosterCuts,
    /// Letters drop in with one small overshoot.
    GlyphDrop,
    /// Letters type on.
    TypeOn,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartPreset {
    /// Thin muted axis, one accent series.
    #[default]
    ThinAxis,
    /// Ink series on paper; the hero in accent.
    Paper,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChartKind {
    #[default]
    Line,
    Area,
    Bar,
    Donut,
    /// A spring response from `values[0]` to `values[1]`: the curve is
    /// drawn by the same closed-form spring that moves its dot.
    Spring,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    #[default]
    Ring,
    Underline,
    Marker,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DiagramPreset {
    /// Ink outlines on the ground, mono labels, thin connectors.
    #[default]
    Outline,
    /// Filled cards (ink on light / light on dark), bold labels.
    Cards,
    /// Round nodes on a line, like a process rail.
    Rail,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FieldPreset {
    /// Dots that shrink away from the centre.
    #[default]
    Halftone,
    /// An even dot grid.
    Grid,
    /// Dots on concentric rings.
    Rings,
    /// Rounded squares on a coarse grid (a tile board).
    Tiles,
}

impl ComponentBody {
    pub fn kind_name(&self) -> &'static str {
        match self {
            ComponentBody::Backdrop { .. } => "backdrop",
            ComponentBody::Shape { .. } => "shape",
            ComponentBody::Title { .. } => "title",
            ComponentBody::KineticText { .. } => "kinetic_text",
            ComponentBody::Chart { .. } => "chart",
            ComponentBody::Field { .. } => "field",
            ComponentBody::Hud { .. } => "hud",
            ComponentBody::Diagram { .. } => "diagram",
            ComponentBody::CodeBlock(_) => "code_block",
            ComponentBody::DeviceFrame(_) => "device_frame",
            ComponentBody::Bento(_) => "bento",
            ComponentBody::Lockup(_) => "lockup",
            ComponentBody::PhotoStack(_) => "photo_stack",
            ComponentBody::Sketch(_) => "sketch",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BackdropPreset {
    /// Flat colour.
    #[default]
    Flat,
    /// Soft radial falloff from the centre (needs `allow: ["gradient"]`).
    Glow,
    /// Gentle top-to-bottom falloff (needs `allow: ["gradient"]`).
    Fall,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShapePreset {
    /// Solid fill.
    #[default]
    Solid,
    /// Stroke only (can `draw` on).
    Outline,
}

/// A named primitive, or `{"path": "<svg path data>"}` in a 100×100 box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShapeSpec {
    Circle,
    Square,
    Rounded,
    Pill,
    Triangle,
    Diamond,
    Star,
    Ring,
    Blob,
    /// SVG path data in a 100×100 box (`M L H V C S Q T A Z`).
    Path(String),
}

/// Where a component sits. `lead`/`trail` are the left/right thirds in
/// landscape and the upper/lower thirds in portrait and square frames.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    #[default]
    Center,
    Lead,
    Trail,
    Top,
    Bottom,
    Left,
    Right,
    /// Titles: left-aligned near the top-left corner (chapter labels).
    TopLeft,
    /// Titles: left-aligned near the bottom-left corner.
    BottomLeft,
}

/// Relative size (shapes: share of the short side; titles: type size).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    S,
    #[default]
    M,
    L,
    Xl,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TitlePreset {
    /// Heavy grotesk, tight.
    #[default]
    Display,
    /// Serif italic, quiet.
    Serif,
    /// Small mono caps, tracked.
    Label,
    /// Bold small caps: a section/chapter name (use with `top_left`).
    Chapter,
    /// Big heavy caps with two offset echoes (accent, then muted) behind.
    Stacked,
}

/// A verb at `at` seconds from the component start, or on music `beat` n
/// (snapped to the nearest frame).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<u32>,
    #[serde(flatten)]
    pub verb: Verb,
}

/// Timeline verbs. Durations and springs are baked in (taste tables × tempo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Verb {
    /// Arrive. Without an `enter`, a component cuts in at its start.
    Enter {
        #[serde(default)]
        style: EnterStyle,
    },
    /// Leave. Without an `exit`, a component cuts out at its end.
    Exit {
        #[serde(default)]
        style: ExitStyle,
    },
    /// Shapes: become another shape (one shape, never cut).
    Morph { to: ShapeSpec },
    /// Travel to another slot on a spring.
    Move { to: Slot },
    /// Change colour to another swatch (once per component).
    Recolour { to: Swatch },
    /// A short struck-and-settle scale hit (one overshoot, no repeats).
    Hit,
    /// Kinetic text: show the next word (hard cut); `flip` swaps ground and
    /// ink from this word on.
    Cut {
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        flip: bool,
    },
    /// Charts: the hero dot and callout pop in.
    PopLabel,
    /// Shapes: a hop up and a spring landing.
    Bounce,
    /// Shapes: travel to a slot along an arc.
    Orbit { to: Slot },
    /// Shapes: grow until the shape covers the whole frame.
    Fill,
    /// Shapes: shrink back from `fill` to their own size.
    Unfill,
    /// Shapes: turn a half turn and settle.
    Spin,
    /// Fields: a recolour that travels outward from the focus as a wave.
    Wave { to: Swatch },
    /// Diagrams: reveal the next node. Charts: reveal the next value.
    Advance,
    /// Lockups: a specular highlight travels across the headline once,
    /// left to right.
    Sheen,
    /// 2.5D projective tilt from `[pitch_deg, yaw_deg]` to `[pitch_deg, yaw_deg]`.
    Tilt {
        #[serde(default)]
        from: [f32; 2],
        to: [f32; 2],
    },
    /// Motion trail echoes (`2..=6` copies behind a moving/scaling layer).
    Echo { count: u8 },
}

impl Verb {
    pub fn name(&self) -> &'static str {
        match self {
            Verb::Enter { .. } => "enter",
            Verb::Exit { .. } => "exit",
            Verb::Morph { .. } => "morph",
            Verb::Move { .. } => "move",
            Verb::Recolour { .. } => "recolour",
            Verb::Hit => "hit",
            Verb::Cut { .. } => "cut",
            Verb::PopLabel => "pop_label",
            Verb::Bounce => "bounce",
            Verb::Orbit { .. } => "orbit",
            Verb::Fill => "fill",
            Verb::Unfill => "unfill",
            Verb::Spin => "spin",
            Verb::Wave { .. } => "wave",
            Verb::Advance => "advance",
            Verb::Sheen => "sheen",
            Verb::Tilt { .. } => "tilt",
            Verb::Echo { .. } => "echo",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EnterStyle {
    /// Scale up from nothing on a small-overshoot spring.
    #[default]
    Pop,
    /// Rise a short way while fading in.
    Rise,
    /// Fade in.
    Fade,
    /// Outline shapes: the stroke draws on; charts build; fields grow
    /// out from the centre.
    Draw,
    /// Titles: a mask wipes left to right.
    Wipe,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExitStyle {
    /// Gone on the next frame: a hard cut (the default; crossfades read as
    /// slop).
    #[default]
    Cut,
    Fade,
    /// Scale down to nothing.
    Shrink,
    /// Drop a short way while fading out.
    Drop,
    /// Titles: a mask wipes off to the right.
    Wipe,
}

impl MotionProgram {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("program serializes")
    }

    /// End of `c` in program seconds.
    pub fn end_of(&self, c: &Component) -> f64 {
        c.end.unwrap_or(self.meta.duration)
    }

    fn recipe_spec(&self) -> Option<crate::recipes::RecipeSpec> {
        self.recipe.map(crate::recipes::spec)
    }

    /// Effective palette (explicit, else the recipe's).
    pub fn palette(&self) -> Option<Palette> {
        self.palette
            .clone()
            .or_else(|| self.recipe_spec().map(|r| r.palette))
    }

    pub fn fonts(&self) -> Fonts {
        self.fonts
            .or_else(|| self.recipe_spec().and_then(|r| r.fonts))
            .unwrap_or_default()
    }

    pub fn tempo(&self) -> Tempo {
        self.tempo
            .or_else(|| self.recipe_spec().and_then(|r| r.tempo))
            .unwrap_or_default()
    }

    pub fn allows(&self, f: Feature) -> bool {
        self.allow.contains(&f) || self.recipe_spec().is_some_and(|r| r.allow.contains(&f))
    }

    /// Replaces every `beat` reference with seconds: component `start_beat`
    /// becomes `start`, event `beat` becomes `at` (component-relative).
    /// Beat times are program seconds, snapped to the nearest frame.
    /// References past the analysis stay unresolved (the validator names
    /// them).
    pub fn resolve_beats(&mut self, beats: &[f64]) {
        let fps = self.meta.fps.value() as f64;
        // Beat-resolved times land a little before the beat (cut lead).
        let lead = crate::taste::CUT_LEAD_FRAMES / fps;
        let snap = |t: f64| ((t * fps).round() / fps - lead).max(0.0);
        if let Some(b) = self.meta.end_beat.and_then(|n| beats.get(n as usize)) {
            let beat = if beats.len() > 1 {
                (beats[beats.len() - 1] - beats[0]) / (beats.len() - 1) as f64
            } else {
                0.5
            };
            self.meta.duration = ((snap(*b) + lead + beat * 0.85) * 1e3).round() / 1e3;
            self.meta.end_beat = None;
        }
        let r6 = |t: f64| (t * 1e6).round() / 1e6;
        for e in &mut self.camera {
            if let Some(b) = e.beat.and_then(|n| beats.get(n as usize)) {
                e.at = Some(r6(snap(*b)));
                e.beat = None;
            }
        }
        for e in &mut self.transitions {
            if let Some(b) = e.beat.and_then(|n| beats.get(n as usize)) {
                e.at = Some(r6(snap(*b)));
                e.beat = None;
            }
        }
        for c in &mut self.components {
            // Rhythm helper: one step per `beat_cuts` beats (kinetic cuts,
            // diagram advances), as many as the content has.
            let rhythm = match &c.body {
                ComponentBody::KineticText {
                    beat_cuts: Some(e),
                    words,
                    ..
                } => Some((*e, words.len().saturating_sub(1), true)),
                ComponentBody::Diagram {
                    beat_cuts: Some(e),
                    nodes,
                    ..
                } => Some((*e, nodes.len().saturating_sub(1), false)),
                // A line's draw already reaches its second value, so it
                // needs one step fewer than bars (one per bar after the first).
                ComponentBody::Chart {
                    beat_cuts: Some(e),
                    values,
                    chart,
                    ..
                } => {
                    let line = matches!(chart, ChartKind::Line | ChartKind::Area);
                    let n = values.len().saturating_sub(if line { 2 } else { 1 });
                    Some((*e, n, false))
                }
                ComponentBody::Bento(b) if b.beat_cuts.is_some() => {
                    Some((b.beat_cuts.unwrap(), b.tiles.len().saturating_sub(1), false))
                }
                ComponentBody::Sketch(sk) if sk.beat_cuts.is_some() => {
                    let every = sk.beat_cuts.unwrap().max(1);
                    let n = if sk.elements.is_empty() {
                        c.end_beat
                            .or(self.meta.end_beat)
                            .zip(c.start_beat)
                            .map_or(3, |(eb, sb)| {
                                eb.saturating_sub(sb)
                                    .saturating_div(every)
                                    .saturating_sub(1) as usize
                            })
                    } else {
                        sk.elements.len().saturating_sub(1)
                    };
                    Some((every, n, false))
                }
                _ => None,
            };
            if let (Some(first), Some((every, steps, cut))) = (c.start_beat, rhythm) {
                let every = every.max(1);
                let is_step = |v: &Verb| {
                    matches!(v, Verb::Cut { .. }) && cut || matches!(v, Verb::Advance) && !cut
                };
                let taken: Vec<u32> = c
                    .timeline
                    .iter()
                    .filter(|e| is_step(&e.verb))
                    .filter_map(|e| e.beat)
                    .collect();
                let mut have = c.timeline.iter().filter(|e| is_step(&e.verb)).count();
                let mut k = 1u32;
                while have < steps && k < 4096 {
                    let b = first + every * k;
                    if !taken.contains(&b) {
                        c.timeline.push(Event {
                            at: None,
                            beat: Some(b),
                            verb: if cut {
                                Verb::Cut { flip: false }
                            } else {
                                Verb::Advance
                            },
                        });
                        have += 1;
                    }
                    k += 1;
                }
                if matches!(&c.body, ComponentBody::Chart { hero: Some(_), .. })
                    && !c.timeline.iter().any(|e| matches!(e.verb, Verb::PopLabel))
                {
                    let b = first + every * k;
                    let fits = match (c.end_beat, c.end, beats.get(b as usize)) {
                        (Some(eb), _, _) => b < eb,
                        (None, Some(end), Some(tb)) => snap(*tb) + 0.5 <= end,
                        (None, None, Some(tb)) => snap(*tb) + 0.5 <= self.meta.duration,
                        _ => false,
                    };
                    if fits {
                        c.timeline.push(Event {
                            at: None,
                            beat: Some(b),
                            verb: Verb::PopLabel,
                        });
                    }
                }
            }
            if let Some(b) = c.end_beat.and_then(|n| beats.get(n as usize)) {
                c.end = Some(r6(snap(*b)));
                c.end_beat = None;
            }
            let first_beat = c.start_beat;
            if let Some(b) = c.start_beat.and_then(|n| beats.get(n as usize)) {
                c.start = r6(snap(*b));
                c.start_beat = None;
            }
            match &mut c.body {
                ComponentBody::CodeBlock(b) => {
                    crate::code::resolve_steps(&mut b.steps, beats, c.start, fps)
                }
                ComponentBody::DeviceFrame(f) => {
                    crate::device::resolve_steps(&mut f.steps, beats, c.start, fps)
                }
                ComponentBody::PhotoStack(ps) => {
                    crate::photo::resolve_steps(ps, first_beat, beats, c.start, fps)
                }
                _ => {}
            }
            for e in &mut c.timeline {
                if let Some(b) = e.beat.and_then(|n| beats.get(n as usize)) {
                    e.at = Some(r6(snap(*b) - c.start));
                    e.beat = None;
                }
            }
        }
    }
}
