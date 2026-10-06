//! MotionScene v0.2 — the explicit, renderer-independent scene specification.
//!
//! Produced by the MotionCompiler (or written by hand). Everything here is
//! explicit: positions in canvas pixels, times in seconds, colors as hex.
//! No semantic planning happens past this point.
//!
//! Coordinate conventions:
//! - Canvas origin is top-left, +y down, units are pixels.
//! - A layer's local box is `[0, width] x [0, height]`.
//! - `(x, y)` is where the layer's anchor point lands in its parent's space.
//!   `anchor_x/anchor_y` are fractions of the box (0,0 = top-left, 0.5,0.5 = center).
//! - Layer transform = translate(x, y) · rotate · scale · translate(-anchor·size).
//!
//! Timing conventions:
//! - Scene times are absolute project seconds; scenes may overlap.
//! - Motion `start` is seconds relative to its scene's `start_seconds`.
//! - Shared-element track keys are `(scene id, scene-local seconds)`.
//!
//! Draw order: all visible layers are sorted by `(z_index, scene order, layer order)`.
//! z_index is therefore global, which lets overlapping scenes interleave.
//!
//! Evaluation order per layer (see ARCHITECTURE.md "Transform & layout order"):
//! base geometry → geometry channel (AccentExpand) → layout binding (placement
//! inside the bound sibling's *resolved* box) → local motions (move, scale,
//! rotate) → container (group) transform = scene space → camera at the
//! layer's depth = canvas space. Shared elements resolve each track key into
//! canvas space with the same rules, then interpolate.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::easing::Easing;

pub const SCENE_VERSION: &str = "0.2";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionProject {
    pub version: String,
    pub project: ProjectMeta,
    pub canvas: Canvas,
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub assets: Vec<Asset>,
    /// Asset paths resolve relative to this directory, which is itself
    /// relative to the directory containing the motion file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_root: Option<String>,
    pub scenes: Vec<Scene>,
    /// Logical elements that survive scene boundaries (see [`SharedElement`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared: Vec<SharedElement>,
    /// (0.14) Sampled signals (e.g. the music bed's transient energy) that
    /// `pulse` motions read. Compiler-computed; the Timeline only samples them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub envelopes: Vec<Envelope>,
}

/// (0.14) A signal sampled at `fps` over project time (sample `i` covers
/// `[i/fps, (i+1)/fps)`); values are 0..=1. Reading past the end gives 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub fps: f32,
    pub values: Vec<f32>,
}

impl Envelope {
    /// The sample covering project time `t` (seconds); 0 outside the signal.
    pub fn at(&self, t: f64) -> f32 {
        if t.is_nan() || t < 0.0 || self.fps <= 0.0 {
            return 0.0;
        }
        let i = (t * self.fps as f64).floor() as usize;
        self.values.get(i).copied().unwrap_or(0.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub name: String,
    /// Total duration in seconds. If absent, the end of the last scene/shared key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<f64>,
    /// (0.9) What `--explore` changed (absent at level 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exploration: Option<crate::compiler::explore::ExploreRecord>,
    /// (0.10) What a speech-led compile did (absent without `--speech`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speech: Option<crate::speech::SpeechRecord>,
    /// (0.10 Q) Art direction applied (`--art`); absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub art: Option<crate::compiler::art_direction::ArtRecord>,
    /// (0.23) What the direction layer chose: take, per-beat template and
    /// parameters, best-of-N scores. Written only under variety; absent
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<crate::compiler::direction::DirectionRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Canvas {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub background: Color,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// Font role -> font asset id.
    #[serde(default)]
    pub fonts: BTreeMap<FontRole, String>,
    /// Named palette, informational for tooling; layers carry explicit colors.
    #[serde(default)]
    pub palette: BTreeMap<String, Color>,
    /// (0.9) Emotion typography record when the registry chose the faces (or
    /// fell back); absent for the 0.8 tone pairings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typography: Option<crate::compiler::typography::TypographyChoice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Asset {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: AssetKind,
    /// File path; for `sprite_sequence` the directory holding the frames.
    pub path: String,
    /// (0.11) Present iff `kind == sprite_sequence`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<SpriteSpec>,
}

/// (0.11) A frame-sequence asset: `<path>/<pattern>` with 1-based `%04d`
/// numbering (`frame_0001.png` …). Frames are extracted outside the engine
/// (scripts/extract_sprite.sh); the Timeline picks the frame, the renderer
/// draws it like a still image.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpriteSpec {
    pub frame_count: u32,
    pub fps: f64,
    pub mode: SpriteMode,
    /// printf-style file pattern, default `frame_%04d.png`.
    #[serde(default = "default_sprite_pattern")]
    pub pattern: String,
}

fn default_sprite_pattern() -> String {
    "frame_%04d.png".to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpriteMode {
    /// Wraps around (seamless loops).
    Loop,
    /// Plays once and holds the last frame (footage clips).
    Once,
}

/// (0.11) When and which part of a sprite sequence an image layer plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpritePlayback {
    /// Scene-local seconds at which frame `in_frame` shows.
    #[serde(default)]
    pub start: f64,
    /// First frame (0-based) — the clip's in point.
    #[serde(default)]
    pub in_frame: u32,
    /// Last frame (0-based, inclusive) — the clip's out point; `None` = last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub out_frame: Option<u32>,
}

/// (0.11) An image (still or sprite sequence) shown inside a host image's
/// screen hole, clipped to `screen_box` and following the host's transform.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenInsert {
    pub asset: String,
    /// [x, y, w, h] as fractions of the host layer's box (from the catalog
    /// `screen_box`, mapped through the host's fit).
    pub screen_box: [f32; 4],
    #[serde(default)]
    pub fit: Fit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playback: Option<SpritePlayback>,
}

/// (0.11) Frame index (0-based) a sprite shows at scene-local time `t_local`.
/// Pure: floor((t_local − start) × fps) from `in_frame`; before `start` →
/// `in_frame`; `Loop` wraps within [in, out], `Once` holds `out`.
pub fn sprite_frame(spec: &SpriteSpec, playback: Option<&SpritePlayback>, t_local: f64) -> u32 {
    let default = SpritePlayback {
        start: 0.0,
        in_frame: 0,
        out_frame: None,
    };
    let pb = playback.unwrap_or(&default);
    let last = spec.frame_count.saturating_sub(1);
    let in_f = pb.in_frame.min(last);
    let out_f = pb.out_frame.unwrap_or(last).clamp(in_f, last);
    let span = (out_f - in_f + 1) as i64;
    let elapsed = t_local - pb.start;
    if elapsed <= 0.0 || spec.fps <= 0.0 {
        return in_f;
    }
    // Snap to 1e-6 s so float noise at exact frame boundaries is stable.
    let n = ((elapsed * 1e6).round() / 1e6 * spec.fps + 1e-9).floor() as i64;
    let k = match spec.mode {
        SpriteMode::Loop => n.rem_euclid(span),
        SpriteMode::Once => n.min(span - 1),
    };
    in_f + k as u32
}

/// (0.11) File name of 1-based frame `number` for a printf-style `pattern`
/// (`%d`, `%04d`, …; `%%` is a literal percent). `None` when the pattern has
/// no single integer conversion.
pub fn sprite_file_name(pattern: &str, number: u32) -> Option<String> {
    let mut out = String::new();
    let mut converted = false;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        if converted {
            return None;
        }
        let mut zero = false;
        let mut width = String::new();
        while let Some(&d) = chars.peek() {
            if d.is_ascii_digit() {
                if width.is_empty() && d == '0' {
                    zero = true;
                } else {
                    width.push(d);
                }
                chars.next();
            } else {
                break;
            }
        }
        if chars.next() != Some('d') {
            return None;
        }
        let w: usize = if width.is_empty() {
            0
        } else {
            width.parse().ok()?
        };
        let digits = number.to_string();
        if digits.len() < w {
            let fill = if zero { '0' } else { ' ' };
            out.extend(std::iter::repeat_n(fill, w - digits.len()));
        }
        out.push_str(&digits);
        converted = true;
    }
    converted.then_some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Image,
    Svg,
    Font,
    /// (0.11) Directory of frames, see [`SpriteSpec`].
    SpriteSequence,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub id: String,
    pub start_seconds: f64,
    pub duration_seconds: f64,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub motions: Vec<Motion>,
    /// 2D multi-plane camera for this scene. Absent = identity camera.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<Camera>,
    /// Compiler-planned lifecycle phases. Informational: the Timeline and
    /// Renderer ignore it; validation checks its ordering; QA reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<Lifecycle>,
    /// (0.15) Full-frame post effects while this scene is active, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post: Vec<PostEffect>,
}

/// (0.15) A post effect whose strength animates `from` → `to` (0..=1) over
/// `[start, start + duration]` scene-local seconds with `easing` and is zero
/// outside that window (flashes). `duration <= 0` = persistent: strength `to`
/// from `start` to the scene end (grain, vignette). Strength 0 is a no-op;
/// parameters give the look at strength 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostEffect {
    pub start: f64,
    pub duration: f64,
    #[serde(default)]
    pub easing: Easing,
    pub from: f32,
    pub to: f32,
    #[serde(flatten)]
    pub kind: PostKind,
}

/// (0.15) Post-processing operators. Every one is a pure function of the
/// frame pixels, its strength and (for noise) `seed` + frame time, so CPU and
/// GPU backends can match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum PostKind {
    /// Bright pass above `threshold` (0..=1 luma), blurred with radius
    /// `radius` px (dual downsample), added back.
    Bloom { threshold: f32, radius: f32 },
    /// Red shifted `shift` px one way and blue the other along `angle_deg`.
    ChromaticAberration { shift: f32, angle_deg: f32 },
    /// Horizontal band tearing: `bands` noise bands displaced up to `max_shift`
    /// px, plus an RGB split inside displaced bands.
    Glitch {
        bands: u32,
        max_shift: f32,
        seed: u32,
    },
    /// Light rays: radial blur toward `center` (canvas px) over `length`
    /// (0..=1 of the distance to the centre), added back over the frame.
    Rays { center: [f32; 2], length: f32 },
    /// Motion blur along `angle_deg` over `length` px (whip pans).
    DirectionalBlur { angle_deg: f32, length: f32 },
    /// Film grain of `amount` (0..=1) from seeded noise, changing per frame.
    Grain { amount: f32, seed: u32 },
    /// Darkened corners: `amount` (0..=1) at the corners, smooth falloff.
    Vignette { amount: f32 },
}

impl Scene {
    pub fn end_seconds(&self) -> f64 {
        self.start_seconds + self.duration_seconds
    }
}

/// Scene lifecycle as ordered phase boundaries in scene-local seconds
/// (see docs/SCENE_LIFECYCLE.md):
///
/// ```text
/// 0 ─PRE_ENTER─ enter ─ENTER─ settle ─SETTLE─ read ─READ─ evolve ─EVOLVE─ anticipate ─ANTICIPATE─ bridge ─BRIDGE─ duration
/// ```
///
/// Invariant: `0 <= enter <= settle <= read <= evolve <= anticipate <= bridge <= duration`.
/// Boundaries (not ranges) make overlapping phases unrepresentable. `bridge`
/// is where the outgoing transition starts (the next scene's start); for the
/// last scene it equals the duration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Lifecycle {
    pub enter: f64,
    pub settle: f64,
    pub read: f64,
    pub evolve: f64,
    pub anticipate: f64,
    pub bridge: f64,
}

/// One lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    PreEnter,
    Enter,
    Settle,
    Read,
    Evolve,
    Anticipate,
    Bridge,
}

impl Phase {
    pub const ALL: [Phase; 7] = [
        Phase::PreEnter,
        Phase::Enter,
        Phase::Settle,
        Phase::Read,
        Phase::Evolve,
        Phase::Anticipate,
        Phase::Bridge,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Phase::PreEnter => "PRE_ENTER",
            Phase::Enter => "ENTER",
            Phase::Settle => "SETTLE",
            Phase::Read => "READ",
            Phase::Evolve => "EVOLVE",
            Phase::Anticipate => "ANTICIPATE",
            Phase::Bridge => "BRIDGE",
        }
    }
}

impl Lifecycle {
    /// Boundaries in order: `[0, enter, settle, read, evolve, anticipate, bridge]`.
    pub fn starts(&self) -> [f64; 7] {
        [
            0.0,
            self.enter,
            self.settle,
            self.read,
            self.evolve,
            self.anticipate,
            self.bridge,
        ]
    }

    /// `[start, end)` of `phase` in scene-local seconds, given the scene duration.
    pub fn range(&self, phase: Phase, duration: f64) -> (f64, f64) {
        let s = self.starts();
        let i = Phase::ALL.iter().position(|p| *p == phase).unwrap_or(0);
        let end = if i + 1 < s.len() { s[i + 1] } else { duration };
        (s[i], end)
    }

    /// The phase containing scene-local time `t` (clamped into the scene).
    pub fn phase_at(&self, t: f64) -> Phase {
        let s = self.starts();
        let i = s.iter().rposition(|b| t >= *b).unwrap_or(0);
        Phase::ALL[i]
    }

    /// `n` event times spread through `[from, to)`: the first at `from`, the
    /// rest evenly spaced, leaving a final gap equal to one spacing so the last
    /// event is readable before `to`. Empty when `n == 0`.
    pub fn spread(from: f64, to: f64, n: usize) -> Vec<f64> {
        if n == 0 {
            return Vec::new();
        }
        let span = (to - from).max(0.0);
        let step = span / n as f64;
        (0..n).map(|i| from + step * i as f64).collect()
    }

    /// `n` evolve-event times within `[evolve, anticipate)`: the secondary
    /// information that arrives after the primary has been read.
    pub fn evolve_events(&self, n: usize) -> Vec<f64> {
        Lifecycle::spread(self.evolve, self.anticipate, n)
    }
}

// ---------------------------------------------------------------------------
// Layers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub id: String,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub width: f32,
    #[serde(default)]
    pub height: f32,
    #[serde(default = "one")]
    pub scale_x: f32,
    #[serde(default = "one")]
    pub scale_y: f32,
    #[serde(default)]
    pub rotation_degrees: f32,
    #[serde(default)]
    pub anchor_x: f32,
    #[serde(default)]
    pub anchor_y: f32,
    #[serde(default = "one")]
    pub opacity: f32,
    #[serde(default)]
    pub z_index: i32,
    #[serde(default = "yes")]
    pub visible: bool,
    /// Static rectangular clip on the layer's own box. Animated by MaskReveal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip: Option<ClipInset>,
    /// Parallax depth factor for the scene camera (0 = infinitely far / static,
    /// 1 = subject plane, >1 = foreground). Inherited by children; absent at
    /// the top level means 1.0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
    /// (0.16) True depth in px for a perspective camera (`Camera.perspective`):
    /// 0 = the focus plane at rest, positive = farther away. Ignored without a
    /// perspective camera. Inherited offsets add for children.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z: Option<f32>,
    /// (0.16) Card tilt in degrees about the layer's x and y axes through its
    /// anchor (perspective cameras only; drawn with a projective warp).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tilt: Option<[f32; 2]>,
    /// Dynamic placement relative to an earlier sibling's resolved box. When
    /// present, `x`/`y` are ignored and the alignment point is the pivot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutBinding>,
    #[serde(flatten)]
    pub kind: LayerKind,
}

/// Parent-relative placement. The child's alignment point (chosen by
/// `horizontal`/`vertical`; for text the vertical point uses measured ink
/// bounds) lands on the matching point of the parent's content box
/// (parent box at the current frame minus `padding`), plus `offset`.
/// The child lives in the parent's box space: parent move/scale/rotate
/// propagate, parent width/height changes re-place the child without
/// resizing it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutBinding {
    /// Id of an earlier sibling (same group / same scene top level). For a
    /// shared-element track key: any layer id in the key's scene.
    pub parent: String,
    #[serde(default)]
    pub horizontal: HAlign,
    #[serde(default)]
    pub vertical: VAlign,
    /// Pixels in the parent's box space, added after alignment.
    #[serde(default, skip_serializing_if = "is_zero2")]
    pub offset: [f32; 2],
    /// Pixels in the parent's box space.
    #[serde(default, skip_serializing_if = "Padding::is_zero")]
    pub padding: Padding,
}

fn is_zero2(v: &[f32; 2]) -> bool {
    v[0] == 0.0 && v[1] == 0.0
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HAlign {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VAlign {
    Top,
    #[default]
    Center,
    Bottom,
}

impl HAlign {
    pub fn fraction(self) -> f32 {
        match self {
            HAlign::Left => 0.0,
            HAlign::Center => 0.5,
            HAlign::Right => 1.0,
        }
    }
}

impl VAlign {
    pub fn fraction(self) -> f32 {
        match self {
            VAlign::Top => 0.0,
            VAlign::Center => 0.5,
            VAlign::Bottom => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Padding {
    #[serde(default)]
    pub left: f32,
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub bottom: f32,
}

impl Padding {
    pub fn uniform(v: f32) -> Self {
        Padding {
            left: v,
            top: v,
            right: v,
            bottom: v,
        }
    }
    pub fn is_zero(&self) -> bool {
        *self == Padding::default()
    }
}

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayerKind {
    Rectangle {
        fill: Color,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Stroke>,
    },
    RoundedRectangle {
        fill: Color,
        radius: f32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stroke: Option<Stroke>,
    },
    Text(TextStyle),
    Image {
        asset: String,
        #[serde(default)]
        fit: Fit,
        /// (0.5) Deterministic pixel treatment applied once when the renderer
        /// prepares this layer's image.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        treatment: Option<ImageTreatment>,
        /// (0.11) Playback window when `asset` is a sprite sequence.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        playback: Option<SpritePlayback>,
        /// (0.11) Content shown in this image's screen hole.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        insert: Option<Box<ScreenInsert>>,
    },
    Svg {
        asset: String,
        #[serde(default)]
        fit: Fit,
    },
    Group {
        #[serde(default)]
        children: Vec<Layer>,
    },
    Texture(TextureSpec),
    /// Open or closed polyline in box space (pixels). Animated by `trim`.
    Polyline {
        points: Vec<[f32; 2]>,
        stroke: Stroke,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        closed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        fill: Option<Color>,
    },
}

impl LayerKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            LayerKind::Rectangle { .. } => "rectangle",
            LayerKind::RoundedRectangle { .. } => "rounded_rectangle",
            LayerKind::Text(_) => "text",
            LayerKind::Image { .. } => "image",
            LayerKind::Svg { .. } => "svg",
            LayerKind::Group { .. } => "group",
            LayerKind::Texture(_) => "texture",
            LayerKind::Polyline { .. } => "polyline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Scale to fit inside the box, preserving aspect ratio, centered.
    #[default]
    Contain,
    /// Scale to cover the box, preserving aspect ratio, centered (cropped by box).
    Cover,
    /// Stretch to the box.
    Fill,
}

// ---------------------------------------------------------------------------
// Image treatments (0.5) — docs/IMAGE_TREATMENTS.md
// ---------------------------------------------------------------------------

/// Named treatment family. Informational for the renderer (which executes the
/// explicit parameters of [`ImageTreatment`]); the compiler picks it from style.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TreatmentPreset {
    #[default]
    Natural,
    EditorialMonochrome,
    EditorialDuotone,
    PaperCutout,
    MutedDocumentary,
}

/// Luma mapped between two colours.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Duotone {
    pub shadow: Color,
    pub highlight: Color,
    /// 0 = original, 1 = full duotone.
    pub amount: f32,
}

/// A flat colour mixed over the image (alpha preserved).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TintSpec {
    pub color: Color,
    /// 0..=1.
    pub amount: f32,
}

/// A solid border that follows the alpha silhouette (hand-cut paper edge).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PaperEdge {
    pub color: Color,
    /// Border width in layer (canvas) pixels at the layer's base size.
    pub width: f32,
}

/// A soft shadow cast by the alpha silhouette onto the page below.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContactShadow {
    pub color: Color,
    /// Offset in layer (canvas) pixels at the layer's base size.
    pub offset: [f32; 2],
    /// Blur radius in layer (canvas) pixels.
    pub blur: f32,
}

/// (0.10 Q) A die-cut sticker outline for a subject that would vanish into its
/// ground: the alpha silhouette grown by `width_px`, filled with `color` and
/// drawn under the image together with a soft 10 % shadow. When present it
/// replaces `edge` and `shadow`. Opaque images get a plain `edge` keyline
/// instead (the compiler never puts a sticker on them).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Sticker {
    pub color: Color,
    /// Outline width in layer (canvas) pixels at the layer's base size.
    pub width_px: f32,
}

/// Explicit, deterministic pixel operations for an image layer, applied in
/// this order: desaturate → brightness/contrast → duotone → tint → grain
/// (colour pixels only, alpha untouched) → paper edge → contact shadow (both
/// grow the drawn area beyond the image box; the renderer pads accordingly).
/// (0.10 Q) A `sticker` replaces the paper edge and the contact shadow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageTreatment {
    #[serde(default)]
    pub preset: TreatmentPreset,
    /// 0 = original colour, 1 = grey (Rec.709 luma).
    #[serde(default)]
    pub desaturate: f32,
    /// Added to every channel, -1..=1 (0 = unchanged).
    #[serde(default)]
    pub brightness: f32,
    /// Contrast multiplier around mid-grey (1 = unchanged).
    #[serde(default = "one_f32")]
    pub contrast: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duotone: Option<Duotone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tint: Option<TintSpec>,
    /// Static monochrome grain amplitude 0..=1 (seeded, per image pixel).
    #[serde(default)]
    pub grain: f32,
    #[serde(default)]
    pub seed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<PaperEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ContactShadow>,
    /// (0.10 Q) Contrast treatment for subjects that vanish into their ground.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sticker: Option<Sticker>,
}

fn one_f32() -> f32 {
    1.0
}

impl ImageTreatment {
    /// No-op treatment.
    pub fn natural() -> Self {
        ImageTreatment {
            preset: TreatmentPreset::Natural,
            desaturate: 0.0,
            brightness: 0.0,
            contrast: 1.0,
            duotone: None,
            tint: None,
            grain: 0.0,
            seed: 0,
            edge: None,
            shadow: None,
            sticker: None,
        }
    }
}

/// Rectangular clip expressed as insets (fractions 0..=1 of the layer box).
/// `ClipInset::default()` clips nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ClipInset {
    #[serde(default)]
    pub left: f32,
    #[serde(default)]
    pub top: f32,
    #[serde(default)]
    pub right: f32,
    #[serde(default)]
    pub bottom: f32,
}

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// Semantic font roles. Scenes pick a role; the theme maps roles to font assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontRole {
    Display,
    DisplayCondensed,
    SerifEmotional,
    Body,
    Mono,
    Number,
}

impl FontRole {
    pub const ALL: [FontRole; 6] = [
        FontRole::Display,
        FontRole::DisplayCondensed,
        FontRole::SerifEmotional,
        FontRole::Body,
        FontRole::Mono,
        FontRole::Number,
    ];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    pub text: String,
    pub font_role: FontRole,
    pub font_size: f32,
    #[serde(default = "default_weight")]
    pub font_weight: u16,
    #[serde(default)]
    pub italic: bool,
    pub color: Color,
    #[serde(default)]
    pub align: TextAlign,
    /// Multiplier of font_size.
    #[serde(default = "default_line_height")]
    pub line_height: f32,
    /// Extra advance between glyphs, in em (e.g. -0.02).
    #[serde(default)]
    pub letter_spacing: f32,
    /// Wrap width in pixels. When absent, lines break only at explicit `\n`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_width: Option<f32>,
    #[serde(default)]
    pub uppercase: bool,
    /// Measured vertical ink extent inside the box (pixels from the box top),
    /// written by the compiler from real glyph outlines. Layout uses it to
    /// center text optically; absent = use the box.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink: Option<InkBounds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InkBounds {
    pub top: f32,
    pub bottom: f32,
}

fn default_weight() -> u16 {
    400
}
fn default_line_height() -> f32 {
    1.1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

// ---------------------------------------------------------------------------
// Textures / materials
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureSpec {
    pub material: Material,
    /// Explicit seed; procedural materials must be fully determined by it.
    #[serde(default)]
    pub seed: u64,
    /// Base / ink color of the material.
    pub color: Color,
    /// 0..=1 strength of the procedural variation.
    #[serde(default = "half")]
    pub intensity: f32,
    /// Feature size in pixels (grain size, halftone cell size, fiber scale).
    #[serde(default = "default_texture_scale")]
    pub scale: f32,
    /// Re-sample the pattern every frame (film grain). The per-frame variation
    /// is derived from `seed` and the frame number, so it stays deterministic.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub animated: bool,
}

fn half() -> f32 {
    0.5
}
fn default_texture_scale() -> f32 {
    8.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Material {
    /// Solid fill of `color`.
    Flat,
    /// Opaque warm paper: `color` base with low-frequency fibers and blotches.
    Paper,
    /// Transparent film grain: light/dark specks, meant as an overlay.
    Grain,
    /// Transparent halftone dot field in `color`, dots fade across the box.
    Halftone,
}

// ---------------------------------------------------------------------------
// Motions
// ---------------------------------------------------------------------------

/// A single animation applied to one layer (by id) within a scene.
///
/// Each operator writes one property channel (see [`MotionOp::channel`]).
/// Two motions on the same layer+channel must not overlap in time.
/// Before its start a motion holds its `from` value (if it is the first motion
/// on that channel); after its end it holds its `to` value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Motion {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub target: String,
    /// Seconds relative to the scene start.
    pub start: f64,
    pub duration: f64,
    #[serde(default)]
    pub easing: Easing,
    /// (0.14) Physical spring replacing `easing` for this motion: progress is
    /// the closed-form response of a damped harmonic oscillator released at
    /// `start` (see [`crate::easing::spring_progress`]), forced to exactly 1 at
    /// the end of `duration`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spring: Option<SpringSpec>,
    #[serde(flatten)]
    pub op: MotionOp,
}

/// (0.14) Mass–spring–damper parameters (unitless, per second).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpringSpec {
    pub stiffness: f32,
    pub damping: f32,
    #[serde(default = "one")]
    pub mass: f32,
}

/// (0.14) A glyph's offset from its rest pose: `dx`/`dy` px, `scale`
/// multiplier, `rotation` degrees about the glyph centre, `opacity` multiplier.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GlyphPose {
    #[serde(default)]
    pub dx: f32,
    #[serde(default)]
    pub dy: f32,
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub rotation: f32,
    #[serde(default = "one")]
    pub opacity: f32,
}

impl GlyphPose {
    pub const REST: GlyphPose = GlyphPose {
        dx: 0.0,
        dy: 0.0,
        scale: 1.0,
        rotation: 0.0,
        opacity: 1.0,
    };
}

/// (0.14) Order in which glyphs of a `glyph_cascade` start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlyphOrder {
    #[default]
    Forward,
    Backward,
    /// From the middle glyph outwards.
    Center,
    /// Seeded shuffle (deterministic).
    Random,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum MotionOp {
    /// Offset (pixels) added to the layer's base (x, y).
    Move { from: [f32; 2], to: [f32; 2] },
    /// Multiplier on the layer's base scale.
    Scale {
        from: f32,
        to: f32,
        #[serde(default)]
        axis: Axis,
    },
    /// Degrees added to the layer's base rotation.
    Rotate { from: f32, to: f32 },
    /// Multiplier on the layer's base opacity.
    Fade { from: f32, to: f32 },
    /// Content stays still; a clip edge travels in `direction`, uncovering it.
    MaskReveal {
        direction: Direction,
        #[serde(default)]
        mode: RevealMode,
    },
    /// Content slides in travelling in `direction` from behind a fixed clip at the box edge.
    ClipReveal {
        direction: Direction,
        #[serde(default)]
        mode: RevealMode,
    },
    /// Animates the layer's box geometry (x, y, width, height) from its base
    /// values to `to`. Used for accent bars that grow into transition wipes.
    AccentExpand { to: BoxRect },
    /// Text layers only: replaces the text with a number counting from `from`
    /// to `to`, formatted with `decimals`, optional thousands grouping and
    /// prefix/suffix (see `timeline::format_count`).
    Count {
        from: f64,
        to: f64,
        #[serde(default)]
        decimals: u8,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        grouping: bool,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        prefix: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        suffix: String,
    },
    /// Polyline layers only: visible fraction of the path length, drawn from
    /// the first point (0 = nothing, 1 = whole path).
    Trim { from: f32, to: f32 },
    /// Moves the layer's rendered colours toward `color` by an amount that
    /// animates from `from` to `to` (each 0..=1; 0 = untouched, 1 = flat
    /// `color`). RGB only: alpha is unchanged. Applies to every layer kind;
    /// a group's tint is inherited by descendants without a tint of their own.
    Tint { color: Color, from: f32, to: f32 },
    /// (0.14) Deterministic shake added on top of the offset/rotation
    /// channels: smooth value noise of (`seed`, time) at `frequency` Hz scaled
    /// by `amplitude` px (x, y) and `rotation` degrees, multiplied by
    /// `exp(-decay * elapsed)`. Zero outside the motion window.
    Shake {
        amplitude: [f32; 2],
        #[serde(default)]
        rotation: f32,
        frequency: f32,
        #[serde(default)]
        decay: f32,
        #[serde(default)]
        seed: u32,
    },
    /// (0.14) Text layers only: every glyph moves from `from` to rest. Glyph
    /// `k` (in `order`, whitespace skipped) starts `k * stagger` after the
    /// motion start and runs for `duration - (n - 1) * stagger` (at least
    /// one frame) with the motion's easing/spring.
    GlyphCascade {
        stagger: f64,
        from: GlyphPose,
        #[serde(default)]
        order: GlyphOrder,
        #[serde(default)]
        seed: u32,
    },
    /// (0.14) Polyline layers only: points morph to `to`. Both paths are
    /// resampled to the same count by arc length before interpolating.
    PathMorph { to: Vec<[f32; 2]> },
    /// (0.14) Motion trail: `count` ghosts of the layer drawn behind it as it
    /// was `k * spacing` seconds earlier, opacity × `decay^k`. Active over the
    /// motion window only.
    Echo { count: u8, spacing: f64, decay: f32 },
    /// (0.14) Audio-reactive scale: multiplies the layer scale by
    /// `1 + gain * envelope(t)` (project time) over the motion window.
    Pulse { envelope: String, gain: f32 },
    /// (0.18) The layer rides a sphere of `radius` px centred on its own
    /// position. Its point `at` = `[lon, lat]` degrees has the unit direction
    /// `dir = (sin lon cos lat, -sin lat, -cos lon cos lat)` (`[0, 0]` faces
    /// the camera: the nearest point; positive lat is up) and the sphere turns
    /// by `[yaw, pitch]` degrees animating from → to: the offset is
    /// `radius * Rx(pitch) * Ry(yaw) * dir` with
    /// `Ry(a)(x, y, z) = (x cos a - z sin a, y, x sin a + z cos a)` and
    /// `Rx(b)(x, y, z) = (x, y cos b + z sin b, -y sin b + z cos b)`.
    /// x/y add to the offset channel (after `move`); z adds to the layer's
    /// `z` (top-level layers of perspective scenes; ignored elsewhere).
    /// `[yaw, pitch] = [-lon, -lat]` brings the point to the front.
    Revolve {
        radius: f32,
        at: [f32; 2],
        from: [f32; 2],
        to: [f32; 2],
    },
    /// (0.19) Turn the layer's plane in 3D: `[pitch, yaw]` degrees about its x
    /// and y axes through its anchor (the same angles as `Layer.tilt`),
    /// animating from → to. It replaces the static `tilt` while the motion is
    /// active. Top-level layers of perspective scenes only. Under a
    /// `billboard` camera the plane turns about its own centre with a local
    /// perspective and never shears with the orbit, so a picture can swing
    /// into view from an angle and settle flat.
    Tilt { from: [f32; 2], to: [f32; 2] },
}

impl MotionOp {
    pub fn channel(&self) -> Channel {
        match self {
            MotionOp::Move { .. } => Channel::Offset,
            MotionOp::Scale { .. } => Channel::Scale,
            MotionOp::Rotate { .. } => Channel::Rotation,
            MotionOp::Fade { .. } => Channel::Opacity,
            MotionOp::MaskReveal { .. } => Channel::Clip,
            MotionOp::ClipReveal { .. } => Channel::ContentOffset,
            MotionOp::AccentExpand { .. } => Channel::Geometry,
            MotionOp::Count { .. } => Channel::Text,
            MotionOp::Trim { .. } => Channel::Trim,
            MotionOp::Tint { .. } => Channel::Tint,
            MotionOp::Shake { .. } => Channel::Shake,
            MotionOp::GlyphCascade { .. } => Channel::Glyph,
            MotionOp::PathMorph { .. } => Channel::Path,
            MotionOp::Echo { .. } => Channel::Echo,
            MotionOp::Pulse { .. } => Channel::Pulse,
            MotionOp::Revolve { .. } => Channel::Revolve,
            MotionOp::Tilt { .. } => Channel::Tilt,
        }
    }

    pub fn op_name(&self) -> &'static str {
        match self {
            MotionOp::Move { .. } => "move",
            MotionOp::Scale { .. } => "scale",
            MotionOp::Rotate { .. } => "rotate",
            MotionOp::Fade { .. } => "fade",
            MotionOp::MaskReveal { .. } => "mask_reveal",
            MotionOp::ClipReveal { .. } => "clip_reveal",
            MotionOp::AccentExpand { .. } => "accent_expand",
            MotionOp::Count { .. } => "count",
            MotionOp::Trim { .. } => "trim",
            MotionOp::Tint { .. } => "tint",
            MotionOp::Shake { .. } => "shake",
            MotionOp::GlyphCascade { .. } => "glyph_cascade",
            MotionOp::PathMorph { .. } => "path_morph",
            MotionOp::Echo { .. } => "echo",
            MotionOp::Pulse { .. } => "pulse",
            MotionOp::Revolve { .. } => "revolve",
            MotionOp::Tilt { .. } => "tilt",
        }
    }
}

/// The property channel an operator writes. Used for conflict detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Channel {
    Offset,
    Scale,
    Rotation,
    Opacity,
    Clip,
    ContentOffset,
    Geometry,
    Text,
    Trim,
    Tint,
    Shake,
    Glyph,
    Path,
    Echo,
    Pulse,
    Revolve,
    Tilt,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    #[default]
    Both,
    X,
    Y,
}

/// Direction of travel of a reveal edge / content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevealMode {
    /// Hidden -> visible.
    #[default]
    Reveal,
    /// Visible -> hidden (the edge continues travelling in `direction`).
    Conceal,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// ---------------------------------------------------------------------------
// Camera (2D multi-plane)
// ---------------------------------------------------------------------------

/// Per-scene 2D camera. State at scene-local time `t`: `zoom` (1 = none) and
/// `pan` (canvas px the camera has travelled; content appears to move the
/// opposite way). A layer at depth `k` sees
/// `zoom_k = zoom^k`, `pan_k = pan * k` and is mapped
/// `p' = pivot + zoom_k * (p - pan_k - pivot)`.
/// Channels: `push` → zoom, `track` → pan. Same hold rules as motions;
/// overlapping motions on one channel are a validation error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Camera {
    /// Zoom pivot in canvas pixels. Absent = canvas center.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pivot: Option<[f32; 2]>,
    #[serde(default)]
    pub motions: Vec<CameraMotion>,
    /// (0.16) Perspective camera. When present, layers project by their `z`
    /// (px, 0 = focus plane at rest) through a pinhole with `fov_deg` vertical
    /// field of view looking at the canvas centre; `dolly` moves the camera
    /// along its axis, `orbit` rotates it about the pivot; depth of field
    /// blurs a layer by `aperture * |z_view - focus_z| / 100` px (capped at
    /// 24). Absent = the 2D multi-plane camera above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<Perspective>,
}

/// (0.16) Pinhole camera settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Perspective {
    pub fov_deg: f32,
    #[serde(default)]
    pub focus_z: f32,
    #[serde(default)]
    pub aperture: f32,
    /// (0.18) Sprites face the camera: the orbit moves a top-level layer's
    /// box centre through 3D (parallax, depth, blur) but does not turn its plane,
    /// so pictures never shear like printed cards. Explicit `tilt` still
    /// turns the plane.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub billboard: bool,
    /// (0.18) Within one `z_index`, this scene's top-level layers draw far to
    /// near by their view distance (stable for ties), so layers that
    /// travel in depth (`revolve`) pass in front of and behind each other.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub depth_sort: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CameraMotion {
    /// Seconds relative to the scene start.
    pub start: f64,
    pub duration: f64,
    #[serde(default)]
    pub easing: Easing,
    /// (0.18) Cubic Hermite progress replacing `easing`: normalised start and
    /// end slopes `[v0, v1]` (d progress / d(t / duration)), so chained moves
    /// hand their speed over without stopping. 1 = linear speed; slopes in
    /// 0..=3 keep progress monotonic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<[f32; 2]>,
    #[serde(flatten)]
    pub op: CameraOp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CameraOp {
    /// Zoom from → to (multiplier, 1 = none).
    Push { from: f32, to: f32 },
    /// Pan from → to (canvas pixels travelled by the camera).
    Track { from: [f32; 2], to: [f32; 2] },
    /// (0.14) Camera shake: `trauma` (0..=1) squared scales smooth noise of
    /// (`seed`, time) at `frequency` Hz — up to 2.5 % of the canvas width in
    /// x/y and 3° of roll at trauma 1 — decaying by `exp(-decay * elapsed)`.
    Shake {
        trauma: f32,
        frequency: f32,
        #[serde(default)]
        decay: f32,
        #[serde(default)]
        seed: u32,
    },
    /// (0.14) Roll (degrees about the pivot) from → to.
    Roll { from: f32, to: f32 },
    /// (0.16) Perspective cameras: travel along the view axis in px
    /// (positive = toward the scene) from → to.
    Dolly { from: f32, to: f32 },
    /// (0.16) Perspective cameras: yaw/pitch degrees about the pivot from → to.
    Orbit { from: [f32; 2], to: [f32; 2] },
    /// (0.17) Perspective cameras: rack focus. The in-focus distance
    /// (`Perspective.focus_z`, measured like `z` from the rest plane and
    /// moving with the camera: a plane is sharp when `z - dolly == focus`)
    /// animates from → to.
    Focus { from: f32, to: f32 },
}

// ---------------------------------------------------------------------------
// Shared / persistent elements
// ---------------------------------------------------------------------------

/// A logical visual element that survives scene boundaries.
///
/// The element exists from its first track key to its last key. Keys are
/// anchored to scenes (`scene` id + scene-local `at` seconds) so the track stays
/// correct if scenes are retimed. Between keys every state field is interpolated
/// with the *destination* key's easing; fields a key omits carry over from the
/// previous key (the first key falls back to the base layer values).
/// This is the `SharedTrack` operator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SharedElement {
    pub id: String,
    pub layer: Layer,
    pub track: Vec<TrackKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackKey {
    pub scene: String,
    pub at: f64,
    /// Informational label for the element's role at this key ("evidence", "divider", ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default)]
    pub easing: Easing,
    #[serde(default)]
    pub state: KeyState,
    /// Bind the element to a layer of this key's scene (see [`LayoutBinding`]).
    /// Carries over to following keys until a key sets `x`/`y` or a new binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutBinding>,
}

/// Track key state. `x`, `y`, `rotation_degrees`, `opacity` are absolute;
/// `scale` multiplies the layer's base scale.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct KeyState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation_degrees: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
}

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

/// sRGB color with straight (non-premultiplied) alpha. Serialized as `#RRGGBB` or `#RRGGBBAA`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }
    pub fn with_alpha(self, a: u8) -> Self {
        Color { a, ..self }
    }

    pub fn parse_hex(s: &str) -> Option<Color> {
        let h = s.strip_prefix('#')?;
        if !h.is_ascii() {
            return None;
        }
        let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
        match h.len() {
            6 => Some(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Color::rgba(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Color::parse_hex(&s).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "invalid color '{s}', expected #RRGGBB or #RRGGBBAA"
            ))
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

impl MotionProject {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    pub fn to_json_pretty(&self) -> String {
        // Serialization of these plain data types cannot fail.
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Absolute start time of a scene by id.
    pub fn scene_start(&self, id: &str) -> Option<f64> {
        self.scenes
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.start_seconds)
    }

    /// Total duration: explicit, else the latest scene end / shared key.
    pub fn duration_seconds(&self) -> f64 {
        if let Some(d) = self.project.duration_seconds {
            return d;
        }
        let scenes = self
            .scenes
            .iter()
            .map(Scene::end_seconds)
            .fold(0.0, f64::max);
        let shared = self
            .shared
            .iter()
            .flat_map(|e| e.track.iter())
            .filter_map(|k| self.scene_start(&k.scene).map(|s| s + k.at))
            .fold(0.0, f64::max);
        scenes.max(shared)
    }

    /// Number of frames to render: `ceil(duration * fps)`.
    pub fn frame_count(&self) -> u32 {
        let n = self.duration_seconds() * self.canvas.fps as f64;
        // Guard against float noise like 11.999999 * 30.
        (n - 1e-6).ceil().max(0.0) as u32
    }

    pub fn asset(&self, id: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }
}

#[cfg(test)]
mod sprite_tests {
    #[test]
    fn sprite_file_names() {
        use super::sprite_file_name as f;
        assert_eq!(f("frame_%04d.png", 1).as_deref(), Some("frame_0001.png"));
        assert_eq!(f("f%d.png", 12).as_deref(), Some("f12.png"));
        assert_eq!(f("f%3d.png", 7).as_deref(), Some("f  7.png"));
        assert_eq!(f("a%%_%02d", 123).as_deref(), Some("a%_123"));
        assert_eq!(f("none.png", 1), None);
        assert_eq!(f("%d%d", 1), None);
        assert_eq!(f("%s", 1), None);
    }

    use super::*;

    fn spec(mode: SpriteMode) -> SpriteSpec {
        SpriteSpec {
            frame_count: 10,
            fps: 10.0,
            mode,
            pattern: default_sprite_pattern(),
        }
    }

    #[test]
    fn loop_wraps_and_once_holds() {
        let l = spec(SpriteMode::Loop);
        let o = spec(SpriteMode::Once);
        assert_eq!(sprite_frame(&l, None, -1.0), 0);
        assert_eq!(sprite_frame(&l, None, 0.0), 0);
        assert_eq!(sprite_frame(&l, None, 0.1), 1);
        assert_eq!(sprite_frame(&l, None, 0.95), 9);
        assert_eq!(sprite_frame(&l, None, 1.0), 0);
        assert_eq!(sprite_frame(&o, None, 5.0), 9);
        let pb = SpritePlayback {
            start: 1.0,
            in_frame: 2,
            out_frame: Some(4),
        };
        assert_eq!(sprite_frame(&l, Some(&pb), 0.5), 2);
        assert_eq!(sprite_frame(&l, Some(&pb), 1.25), 4);
        assert_eq!(sprite_frame(&l, Some(&pb), 1.3), 2);
        assert_eq!(sprite_frame(&o, Some(&pb), 9.0), 4);
    }
}
