//! (0.23) The direction layer: takes, per-beat direction parameters, the
//! direction record and best-of-N candidate scores (plan
//! `docs/plans/SPRINT_0_23_VARIETY_QA_AUDIO.md` §4).
//!
//! Everything here runs only under `CompileOptions.variety`: a compile
//! without a variety seed never reads a take, never builds a direction and
//! writes no `ProjectMeta.direction`, so default output stays byte-identical.
//!
//! Every choice is a pure function of (take seed, beat, dimension): the take
//! seed already folds in the story key ([`take_seed`]), and [`choice`] keeps
//! each dimension on its own stream so adding a dimension never reshuffles
//! another. No RNG, no clocks.
//!
//! Frozen contract (sprint 0.23 A1): implementers fill in behaviour behind
//! these types and functions; the names, fields and signatures change only
//! through the coordinator.

use serde::{Deserialize, Serialize};

use super::art_direction::Look;
use super::mix;
use crate::easing::Easing;
use crate::intent::Energy;
use crate::motion::kinetic::KineticParams;
use crate::motion::stagger::StaggerSpec;
use crate::scene::{Direction, Layer, LayerKind, Motion, MotionOp};
use crate::style::Tone;

/// The take seed: take 0 is the story-keyed variety seed itself (today's
/// product output), take > 0 is `mix(variety, take)`.
pub fn take_seed(variety: u64, take: u64) -> u64 {
    if take == 0 {
        variety
    } else {
        mix(variety, take)
    }
}

/// The seed of best-of-N candidate `k` (candidate 0 is the take itself).
pub fn candidate_seed(take_seed: u64, k: u8) -> u64 {
    if k == 0 {
        take_seed
    } else {
        mix(take_seed, 0x0C0F_FEE0 ^ k as u64)
    }
}

/// One independent stream per kind of choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dim {
    /// Template alternate (`grammar::candidates`).
    Template,
    /// Cinematic picture arrival side (`cinematic3d::Arrival`).
    Arrival,
    /// FX-director camera move.
    Camera,
    /// Hype (kinetic_slam) layout.
    SlamLayout,
    /// Subject-first image layout (`grammar::ImageLayout`).
    ImageLayout,
    Entrance,
    Travel,
    Stagger,
    Easing,
    Accent,
    Exit,
    Read,
    /// Music bed among the compatible beds.
    Bed,
}

impl Dim {
    fn salt(self) -> u64 {
        // Fixed per dimension (never derived from declaration order).
        match self {
            Dim::Template => 0x7E3A_11C5,
            Dim::Arrival => 0xA441_7A10,
            Dim::Camera => 0xCA3E_7A00,
            Dim::SlamLayout => 0x51A3_1A70,
            Dim::ImageLayout => 0x13A6_E1A7,
            Dim::Entrance => 0xE7E2_A9C3,
            Dim::Travel => 0x72A7_E100,
            Dim::Stagger => 0x57A6_6E20,
            Dim::Easing => 0xEA51_9600,
            Dim::Accent => 0xACCE_9700,
            Dim::Exit => 0xE1D7_0000,
            Dim::Read => 0x2EAD_0000,
            Dim::Bed => 0xBED0_0000,
        }
    }
}

/// The deterministic value of one choice: hash(take seed, beat, dimension).
pub fn choice(seed: u64, beat: usize, dim: Dim) -> u64 {
    mix(mix(seed, dim.salt()), beat as u64 + 1)
}

/// Per-beat option indices in `0..options` for `beats` beats, chosen from the
/// seed with no two consecutive beats equal (when `options >= 2`). Replaces
/// the position rotations (`index % N`). `options == 0` gives an empty list;
/// `options == 1` gives all zeros.
pub fn seeded_sequence(seed: u64, dim: Dim, beats: usize, options: usize) -> Vec<usize> {
    if options == 0 {
        return Vec::new();
    }
    let mut out: Vec<usize> = Vec::with_capacity(beats);
    for beat in 0..beats {
        let h = choice(seed, beat, dim);
        let pick = match out.last() {
            Some(&prev) if options >= 2 => {
                // One of the other options-1 values, uniformly.
                let k = (h % (options as u64 - 1)) as usize;
                if k >= prev {
                    k + 1
                } else {
                    k
                }
            }
            _ => (h % options as u64) as usize,
        };
        out.push(pick);
    }
    out
}

// ---------------------------------------------------------------------------
// BeatParams: builder constants lifted into named, curated choices
// ---------------------------------------------------------------------------

/// How the beat's content enters. `Builder` = the builder's own entrance
/// (what it did before 0.23).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntranceFamily {
    #[default]
    Builder,
    Rise,
    SlideLeft,
    SlideRight,
    WipeUp,
    WipeLeft,
    WipeRight,
    ScalePop,
    GlyphCascade,
    FocusPull,
}

/// The beat's camera move (FX director). `Builder` = the director's own pick.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CameraMove {
    #[default]
    Builder,
    Still,
    PushIn,
    PullOut,
    Truck,
    Crane,
    Orbit,
}

/// The mark that lands the beat's emphasis. `Builder` = the builder's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentMark {
    #[default]
    Builder,
    None,
    Rule,
    Slab,
    Underline,
    Stamp,
}

/// How the beat leaves at ANTICIPATE. `Builder` = the builder's own exit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitFamily {
    #[default]
    Builder,
    Fade,
    SlideUp,
    SlideLeft,
    WipeOut,
    ScaleDown,
    Cut,
}

/// Stagger between the parts of an arrival. `Builder` = the builder's own.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaggerPreset {
    #[default]
    Builder,
    Tight,
    Even,
    Loose,
}

/// Order in which staggered parts arrive.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StaggerOrder {
    #[default]
    Forward,
    Reverse,
    CenterOut,
}

/// Easing family, always inside the style's temperament. `Builder` = the
/// temperament's default curve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EasingFamily {
    #[default]
    Builder,
    Smooth,
    Snappy,
    Spring,
    Linear,
}

/// Per-beat direction. `BeatParams::default()` is the identity: every field
/// says "what the builder did before 0.23", so a builder handed the default
/// produces exactly its old output. Compiler-internal (never in intent or
/// style); numbers here are multipliers on builder constants, not authored
/// pixels or times.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BeatParams {
    pub entrance: EntranceFamily,
    /// Multiplier on the builder's entrance travel distance (1.0 = unchanged;
    /// the planner keeps it in 0.6..=1.6).
    pub travel: f32,
    pub stagger: StaggerPreset,
    pub stagger_order: StaggerOrder,
    pub easing: EasingFamily,
    pub accent: AccentMark,
    pub exit: ExitFamily,
    pub camera: CameraMove,
    /// Multiplier on the share of the beat the READ phase holds (1.0 =
    /// unchanged; speech-led lifecycles clamp it as they clamp everything).
    pub read_share: f32,
    /// Motion budget of the beat: 1.0 = the look's normal amplitude; the
    /// payoff beat gets the most, calm beats stay restrained (0.6..=1.4).
    pub amplitude: f32,
}

impl Default for BeatParams {
    fn default() -> Self {
        BeatParams {
            entrance: EntranceFamily::Builder,
            travel: 1.0,
            stagger: StaggerPreset::Builder,
            stagger_order: StaggerOrder::Forward,
            easing: EasingFamily::Builder,
            accent: AccentMark::Builder,
            exit: ExitFamily::Builder,
            camera: CameraMove::Builder,
            read_share: 1.0,
            amplitude: 1.0,
        }
    }
}

impl BeatParams {
    /// True when every field is the builder's own choice.
    pub fn is_identity(&self) -> bool {
        *self == BeatParams::default()
    }
}

// ---------------------------------------------------------------------------
// Applying BeatParams: the pattern every builder copies (W2c)
// ---------------------------------------------------------------------------
//
// A builder reads `ctx.params_for(beat)` once and passes each of its own
// constants through the matching helper. Every helper returns the builder's
// own value for the identity (`Builder`, `Forward`, 1.0), so a compile with no
// planned params (every compile without a variety seed) is byte-identical.
// Helpers never invent a gesture a builder cannot draw: a family a helper
// does not know for that slot falls back to the builder's value.

/// How an entrance uncovers its layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reveal {
    /// `ClipReveal` (a line rising or sliding in from its own edge).
    Clip,
    /// `MaskReveal` (a wipe across the layer).
    Mask,
}

/// The direction of an edge entrance (clip or mask reveal).
pub fn entrance_direction(p: &BeatParams, builder: Direction) -> Direction {
    match p.entrance {
        EntranceFamily::Rise | EntranceFamily::WipeUp => Direction::Up,
        EntranceFamily::SlideLeft | EntranceFamily::WipeLeft => Direction::Left,
        EntranceFamily::SlideRight | EntranceFamily::WipeRight => Direction::Right,
        _ => builder,
    }
}

/// Clip or mask for an edge entrance.
pub fn entrance_reveal(p: &BeatParams, builder: Reveal) -> Reveal {
    match p.entrance {
        EntranceFamily::WipeUp | EntranceFamily::WipeLeft | EntranceFamily::WipeRight => {
            Reveal::Mask
        }
        EntranceFamily::Rise | EntranceFamily::SlideLeft | EntranceFamily::SlideRight => {
            Reveal::Clip
        }
        _ => builder,
    }
}

/// The easing of an entrance or landing.
pub fn entrance_easing(p: &BeatParams, builder: Easing) -> Easing {
    match p.easing {
        EasingFamily::Builder => builder,
        EasingFamily::Smooth => Easing::OutCubic,
        EasingFamily::Snappy => Easing::OutQuint,
        EasingFamily::Spring => Easing::EditorialSpring,
        EasingFamily::Linear => Easing::Linear,
    }
}

/// An entrance travel distance (× `travel`).
pub fn travel(p: &BeatParams, builder: f32) -> f32 {
    builder * p.travel
}

/// A motion amplitude: drift, reframe step, push (× `amplitude`).
pub fn amplitude(p: &BeatParams, builder: f32) -> f32 {
    builder * p.amplitude
}

/// Stagger offsets for `offsets.len()` parts, rescaled by the preset and
/// reassigned by the order. The builder's offsets come back unchanged for the
/// identity.
pub fn stagger_offsets(p: &BeatParams, offsets: Vec<f64>) -> Vec<f64> {
    let k = match p.stagger {
        StaggerPreset::Builder | StaggerPreset::Even => 1.0,
        StaggerPreset::Tight => 0.6,
        StaggerPreset::Loose => 1.5,
    };
    let scaled: Vec<f64> = if k == 1.0 {
        offsets
    } else {
        offsets.into_iter().map(|o| o * k).collect()
    };
    let n = scaled.len();
    match p.stagger_order {
        StaggerOrder::Forward => scaled,
        StaggerOrder::Reverse => scaled.into_iter().rev().collect(),
        StaggerOrder::CenterOut => {
            // Part i takes the slot of its distance from the middle.
            let mut sorted = scaled.clone();
            sorted.sort_by(|a, b| a.total_cmp(b));
            let mid = (n as f64 - 1.0) / 2.0;
            let mut rank: Vec<usize> = (0..n).collect();
            rank.sort_by(|&a, &b| {
                ((a as f64 - mid).abs())
                    .total_cmp(&(b as f64 - mid).abs())
                    .then(a.cmp(&b))
            });
            let mut out = vec![0.0; n];
            for (slot, &part) in rank.iter().enumerate() {
                out[part] = sorted[slot];
            }
            out
        }
    }
}

/// A kinetic-type stagger steered by the beat: `Tight` / `Loose` pick the
/// impact / calm pattern, the order replaces the builder's unless `Forward`.
pub fn stagger_spec(p: &BeatParams, builder: StaggerSpec) -> StaggerSpec {
    use crate::motion::stagger::{StaggerOrder as Order, StaggerPreset as Pattern};
    StaggerSpec {
        preset: match p.stagger {
            StaggerPreset::Builder | StaggerPreset::Even => builder.preset,
            StaggerPreset::Tight => Pattern::Impact,
            StaggerPreset::Loose => Pattern::Calm,
        },
        order: match p.stagger_order {
            StaggerOrder::Forward => builder.order,
            StaggerOrder::Reverse => Order::Reverse,
            StaggerOrder::CenterOut => Order::CenterOut,
        },
    }
}

/// Kinetic-type parameters (word cascades, slams, replaces) steered by the
/// beat: entrance easing, travel and stagger. The identity returns `builder`.
pub fn kinetic_params(p: &BeatParams, builder: KineticParams) -> KineticParams {
    let mut kp = builder;
    kp.preset.easing = entrance_easing(p, kp.preset.easing);
    kp.preset.travel = travel(p, kp.preset.travel);
    kp.stagger = stagger_spec(p, kp.stagger);
    kp
}

// ---------------------------------------------------------------------------
// (0.23 C2b) Helpers for builders that place layers through shared code
// ---------------------------------------------------------------------------
//
// Same identity rule as above: the default params return the builder's own
// value, or leave the motions exactly as written.

/// A scale factor around 1.0 (a pop's start, a crop's end): its distance from
/// 1.0 is multiplied by `amplitude`. The identity (amplitude 1.0) returns
/// `builder` untouched; a result never drops below 0.05.
pub fn scale_amplitude(p: &BeatParams, builder: f32) -> f32 {
    if p.amplitude == 1.0 {
        builder
    } else {
        (1.0 + (builder - 1.0) * p.amplitude).max(0.05)
    }
}

/// The shape of a decoration mark a builder draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkShape {
    /// A short, thick bar.
    Rule,
    /// A thin line as long as what it underlines.
    Underline,
}

/// A decoration-only mark of a builder (never a value or a label) under the
/// beat's accent: `None` removes it (`Option::None`), `Rule` / `Underline`
/// redraw it in that shape, and every other accent (`Builder`, and `Slab` /
/// `Stamp`, which a builder with no slab or stamp decoration cannot draw)
/// keeps the builder's `own` shape.
pub fn accent_shape(p: &BeatParams, own: MarkShape) -> Option<MarkShape> {
    match p.accent {
        AccentMark::None => None,
        AccentMark::Rule => Some(MarkShape::Rule),
        AccentMark::Underline => Some(MarkShape::Underline),
        AccentMark::Builder | AccentMark::Slab | AccentMark::Stamp => Some(own),
    }
}

/// A layer a builder placed through shared code (`place_subject`,
/// `image_plate`) whose entrance the beat steers afterwards.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub id: String,
    /// The glyph count when the layer is text (a glyph cascade needs it).
    pub glyphs: Option<usize>,
}

impl Placed {
    pub fn of(layer: &Layer) -> Placed {
        Placed {
            id: layer.id.clone(),
            glyphs: match &layer.kind {
                LayerKind::Text(t) => Some(t.text.chars().filter(|c| !c.is_whitespace()).count()),
                _ => None,
            },
        }
    }
}

/// Seconds a pop (`ScalePop`) takes, as a multiple of the entrance it replaces.
const POP_LENGTH: f64 = 1.4;
/// A pop starts this small (x `amplitude` around 1.0).
const POP_FROM: f32 = 0.88;
/// Seconds between two glyphs of a cascade (x the stagger preset), and the
/// share of the entrance the glyph starts may spread over.
const GLYPH_STAGGER: f64 = 0.03;
const GLYPH_SPREAD: f64 = 0.4;

/// The `ScalePop` entrance: a scale pop on `target` (from `POP_FROM` x
/// `amplitude` around 1.0 up to 1.0, `POP_LENGTH` x `entrance` seconds). `None`
/// for any other entrance family, which keeps the builder's own.
pub fn pop_entrance(
    p: &BeatParams,
    target: &str,
    start: f64,
    entrance: f64,
    easing: Easing,
) -> Option<Motion> {
    (p.entrance == EntranceFamily::ScalePop).then(|| Motion {
        spring: None,
        id: None,
        target: target.to_string(),
        start: super::round3(start),
        duration: super::round3(entrance * POP_LENGTH),
        easing,
        op: MotionOp::Scale {
            from: scale_amplitude(p, POP_FROM),
            to: 1.0,
            axis: Default::default(),
        },
    })
}

/// The `GlyphCascade` entrance: every glyph of the text layer `target` (`glyphs`
/// non-space characters) rises in from `from` (dx, dy in px) with the stagger
/// preset's pace and the order (`Reverse` = last glyph first, `CenterOut` =
/// from the middle). The glyph starts spread over `GLYPH_SPREAD` of the
/// `entrance` at most. `None` for any other family, or for a layer of fewer
/// than two glyphs: the builder keeps its own entrance.
pub fn glyph_entrance(
    p: &BeatParams,
    target: &str,
    start: f64,
    entrance: f64,
    from: [f32; 2],
    glyphs: usize,
    easing: Easing,
) -> Option<Motion> {
    use crate::scene::{GlyphOrder, GlyphPose};
    if p.entrance != EntranceFamily::GlyphCascade || glyphs < 2 {
        return None;
    }
    let k = match p.stagger {
        StaggerPreset::Builder | StaggerPreset::Even => 1.0,
        StaggerPreset::Tight => 0.6,
        StaggerPreset::Loose => 1.5,
    };
    let stagger = (GLYPH_STAGGER * k).min(GLYPH_SPREAD * entrance / (glyphs as f64 - 1.0));
    Some(Motion {
        spring: None,
        id: None,
        target: target.to_string(),
        start: super::round3(start),
        duration: super::round3(entrance),
        easing,
        op: MotionOp::GlyphCascade {
            // Rounded down to the millisecond: the glyph starts never spread past the share.
            stagger: ((stagger * 1000.0).floor() / 1000.0).max(0.001),
            from: GlyphPose {
                dx: from[0],
                dy: from[1],
                scale: 1.0,
                rotation: 0.0,
                opacity: 0.0,
            },
            order: match p.stagger_order {
                StaggerOrder::Forward => GlyphOrder::Forward,
                StaggerOrder::Reverse => GlyphOrder::Backward,
                StaggerOrder::CenterOut => GlyphOrder::Center,
            },
            seed: 0,
        },
    })
}

/// The entrance of a value text a builder draws itself (a stamp: it rises
/// through a clip reveal toward `Up`, `easing` being the builder's strike). The
/// identity returns that one clip reveal. Planned: the side, the wipe kind and
/// the easing steer the reveal ([`entrance_direction`], [`entrance_reveal`],
/// [`entrance_easing`]); `ScalePop` adds a scale pop; `GlyphCascade` swaps the
/// reveal for a fade and a per-glyph cascade rising `rise_px` (`glyphs` is the
/// glyph count, 0 for a text that counts up: it falls back to the reveal).
/// `FocusPull` falls back.
pub fn stamp_entrance(
    p: &BeatParams,
    target: &str,
    start: f64,
    duration: f64,
    easing: Easing,
    glyphs: usize,
    rise_px: f32,
) -> Vec<Motion> {
    let easing = entrance_easing(p, easing);
    if let Some(cascade) = glyph_entrance(
        p,
        target,
        start,
        duration,
        [0.0, travel(p, rise_px)],
        glyphs,
        easing,
    ) {
        return vec![
            super::mo::fade(target, start, duration * 0.6, 0.0, 1.0, Easing::OutCubic),
            cascade,
        ];
    }
    let dir = entrance_direction(p, Direction::Up);
    let mut out = vec![match entrance_reveal(p, Reveal::Clip) {
        Reveal::Clip => super::mo::line_in(target, start, duration, dir, easing),
        Reveal::Mask => super::mo::mask(target, start, duration, dir, easing),
    }];
    out.extend(pop_entrance(p, target, start, duration, easing));
    out
}

/// Steer the entrance of layers already placed, from the motions the shared
/// code wrote for them (call it once the builder has pushed all its motions):
///
/// - a clip / mask reveal takes the planned direction, reveal kind and easing;
/// - an entrance move (a non-zero offset easing to rest) takes the planned
///   travel and easing;
/// - a scale pop (from above 1.0 down to 1.0) takes the planned amplitude and
///   easing;
/// - `ScalePop` swaps the entrance move for a scale pop ([`pop_entrance`]),
///   `GlyphCascade` swaps it for a per-glyph cascade on a text layer
///   ([`glyph_entrance`]). Each falls back to the builder's own entrance when
///   there is no entrance move to swap, a scale on the layer leaves the pop
///   less than the entrance's own length (a pop is cut short where the next
///   scale starts), or (cascade) the layer is not text or counts up.
///   `FocusPull` falls back too (these builders blur nothing).
///
/// Fades, counts and every motion that does not leave the layer's entrance
/// (pressure, drift) are left as written. The identity changes nothing.
pub fn steer_entrances(p: &BeatParams, motions: &mut [Motion], placed: &[Placed]) {
    for pl in placed {
        steer_entrance(p, motions, pl);
    }
}

fn steer_entrance(p: &BeatParams, motions: &mut [Motion], pl: &Placed) {
    use crate::scene::{Channel, MotionOp as Op};
    let mut entry: Option<usize> = None;
    for (i, m) in motions.iter_mut().enumerate() {
        if m.target != pl.id {
            continue;
        }
        match &mut m.op {
            Op::ClipReveal { .. } | Op::MaskReveal { .. } => {
                let (reveal, dir, mode) = match &m.op {
                    Op::ClipReveal { direction, mode } => (Reveal::Clip, *direction, *mode),
                    Op::MaskReveal { direction, mode } => (Reveal::Mask, *direction, *mode),
                    _ => continue,
                };
                let direction = entrance_direction(p, dir);
                m.op = match entrance_reveal(p, reveal) {
                    Reveal::Clip => Op::ClipReveal { direction, mode },
                    Reveal::Mask => Op::MaskReveal { direction, mode },
                };
                m.easing = entrance_easing(p, m.easing);
            }
            Op::Move { from, to } if *to == [0.0, 0.0] && *from != [0.0, 0.0] => {
                *from = [travel(p, from[0]), travel(p, from[1])];
                m.easing = entrance_easing(p, m.easing);
                entry.get_or_insert(i);
            }
            Op::Scale { from, to, .. } if *to == 1.0 && *from > 1.0 => {
                *from = scale_amplitude(p, *from);
                m.easing = entrance_easing(p, m.easing);
            }
            _ => {}
        }
    }
    let Some(i) = entry else { return };
    let (start, duration, easing) = (motions[i].start, motions[i].duration, motions[i].easing);
    let swap = match p.entrance {
        EntranceFamily::ScalePop => {
            // The pop runs up to `POP_LENGTH` x the entrance, but not into the
            // next scale on the layer; a scale already running, or one that
            // leaves the pop less than the entrance's own length, is a clash.
            let mut room = duration * POP_LENGTH;
            let mut clash = false;
            for o in motions
                .iter()
                .filter(|o| o.target == pl.id && o.op.channel() == Channel::Scale)
            {
                if o.start + o.duration <= start + 1e-9 {
                    continue;
                }
                if o.start <= start + 1e-9 {
                    clash = true;
                    break;
                }
                room = room.min(o.start - start);
            }
            if clash || room < duration - 1e-9 {
                None
            } else {
                pop_entrance(p, &pl.id, start, room / POP_LENGTH, easing)
            }
        }
        EntranceFamily::GlyphCascade => {
            let counts = motions
                .iter()
                .any(|o| o.target == pl.id && matches!(o.op, Op::Count { .. }));
            match (&motions[i].op, pl.glyphs) {
                (Op::Move { from, .. }, Some(n)) if !counts => {
                    glyph_entrance(p, &pl.id, start, duration, *from, n, easing)
                }
                _ => None,
            }
        }
        _ => None,
    };
    if let Some(m) = swap {
        motions[i] = m;
    }
}

// ---------------------------------------------------------------------------
// The direction planner (W2d) and the look vocabularies (W2e)
// ---------------------------------------------------------------------------
//
// The planner runs after `plan_timing` and before the builders, only under a
// variety seed. It gives every beat a role in the arc and draws its
// BeatParams from the look's vocabulary with one seeded stream per dimension.
// Constraints: no two consecutive beats share an entrance, exit or camera
// move (when the vocabulary has an alternative); the hook opens fast; the
// payoff gets the largest motion budget; calm beats stay restrained.

/// A beat's role in the arc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The opening beat: content within 0.3 s, a fast entrance.
    Hook,
    /// The last reveal / impact beat, else the highest-energy beat of the
    /// last third: the largest motion budget.
    Payoff,
    /// A calm beat: restrained motion.
    Calm,
    Body,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Hook => "hook",
            Role::Payoff => "payoff",
            Role::Calm => "calm",
            Role::Body => "body",
        }
    }
    fn index(self) -> usize {
        match self {
            Role::Hook => 0,
            Role::Payoff => 1,
            Role::Calm => 2,
            Role::Body => 3,
        }
    }
}

/// What the planner reads about one beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BeatSlot {
    pub energy: Energy,
    /// The beat's purpose is `reveal`.
    pub reveal: bool,
}

fn energy_rank(e: Energy) -> u8 {
    match e {
        Energy::Calm => 0,
        Energy::Building => 1,
        Energy::Impact => 2,
    }
}

/// Every beat's role. Beat 0 is the hook; the payoff is never beat 0 unless
/// the story has one beat.
pub fn roles(slots: &[BeatSlot]) -> Vec<Role> {
    let n = slots.len();
    let mut out: Vec<Role> = slots
        .iter()
        .map(|s| {
            if s.energy == Energy::Calm {
                Role::Calm
            } else {
                Role::Body
            }
        })
        .collect();
    if n == 0 {
        return out;
    }
    let payoff = (1..n)
        .rev()
        .find(|&i| slots[i].reveal || slots[i].energy == Energy::Impact)
        .or_else(|| {
            let from = ((2 * n) / 3).max(1);
            (from..n).max_by_key(|&i| (energy_rank(slots[i].energy), i))
        });
    if let Some(p) = payoff {
        out[p] = Role::Payoff;
    }
    out[0] = Role::Hook;
    out
}

/// A look's motion vocabulary: what the planner may pick, with weights, and
/// the motion budget per role (hook, payoff, calm, body).
#[derive(Debug, Clone, Copy)]
pub struct Vocabulary {
    pub name: &'static str,
    pub entrances: &'static [(EntranceFamily, u8)],
    /// Entrances for the hook (fast, edge-led).
    pub hook_entrances: &'static [(EntranceFamily, u8)],
    pub exits: &'static [(ExitFamily, u8)],
    pub cameras: &'static [(CameraMove, u8)],
    pub easings: &'static [(EasingFamily, u8)],
    pub staggers: &'static [(StaggerPreset, u8)],
    pub orders: &'static [(StaggerOrder, u8)],
    pub accents: &'static [(AccentMark, u8)],
    pub amplitude: [f32; 4],
    pub travel: [f32; 4],
}

use EntranceFamily as En;
use ExitFamily as Ex;

/// Calm editorial (editorial, restrained moods): rises, soft wipes, fades.
pub const CALM: Vocabulary = Vocabulary {
    name: "calm",
    entrances: &[
        (En::Rise, 3),
        (En::WipeRight, 2),
        (En::FocusPull, 1),
        (En::SlideRight, 1),
    ],
    hook_entrances: &[(En::WipeRight, 2), (En::SlideRight, 2), (En::Rise, 1)],
    exits: &[(Ex::Fade, 3), (Ex::SlideUp, 2), (Ex::SlideLeft, 1)],
    cameras: &[
        (CameraMove::Still, 2),
        (CameraMove::PushIn, 2),
        (CameraMove::PullOut, 1),
    ],
    easings: &[(EasingFamily::Smooth, 3), (EasingFamily::Builder, 2)],
    staggers: &[
        (StaggerPreset::Builder, 2),
        (StaggerPreset::Loose, 2),
        (StaggerPreset::Even, 1),
    ],
    orders: &[(StaggerOrder::Forward, 4), (StaggerOrder::CenterOut, 1)],
    accents: &[
        (AccentMark::Builder, 3),
        (AccentMark::Rule, 2),
        (AccentMark::Underline, 2),
    ],
    amplitude: [1.0, 1.25, 0.7, 0.9],
    travel: [1.0, 1.3, 0.7, 1.0],
};

/// Precision (technical, corporate): crisp wipes, tight staggers, cuts, a
/// still, trucking or craning camera.
pub const PRECISION: Vocabulary = Vocabulary {
    name: "precision",
    entrances: &[(En::WipeLeft, 3), (En::WipeUp, 2), (En::SlideLeft, 2)],
    hook_entrances: &[(En::WipeLeft, 2), (En::WipeUp, 2), (En::SlideLeft, 1)],
    exits: &[(Ex::Cut, 2), (Ex::SlideLeft, 2), (Ex::Fade, 1)],
    cameras: &[
        (CameraMove::Still, 3),
        (CameraMove::Truck, 2),
        (CameraMove::Crane, 1),
    ],
    easings: &[(EasingFamily::Snappy, 4), (EasingFamily::Linear, 1)],
    staggers: &[(StaggerPreset::Tight, 3), (StaggerPreset::Even, 1)],
    orders: &[(StaggerOrder::Forward, 3), (StaggerOrder::Reverse, 1)],
    accents: &[
        (AccentMark::Rule, 3),
        (AccentMark::Underline, 2),
        (AccentMark::None, 1),
    ],
    amplitude: [1.0, 1.15, 0.75, 0.9],
    travel: [0.8, 1.0, 0.6, 0.8],
};

/// Journey (path-led slides) for the calm tones.
pub const JOURNEY: Vocabulary = Vocabulary {
    name: "journey",
    entrances: &[(En::SlideRight, 3), (En::SlideLeft, 2), (En::Rise, 2)],
    hook_entrances: &[(En::SlideRight, 2), (En::SlideLeft, 2), (En::Rise, 1)],
    exits: &[(Ex::SlideLeft, 3), (Ex::Fade, 1), (Ex::SlideUp, 1)],
    cameras: &[
        (CameraMove::Truck, 2),
        (CameraMove::Still, 1),
        (CameraMove::PushIn, 1),
    ],
    easings: &[(EasingFamily::Smooth, 3), (EasingFamily::Builder, 1)],
    staggers: &[(StaggerPreset::Loose, 2), (StaggerPreset::Builder, 2)],
    orders: &[(StaggerOrder::Forward, 1)],
    accents: &[
        (AccentMark::Builder, 2),
        (AccentMark::Underline, 2),
        (AccentMark::Rule, 1),
    ],
    amplitude: [1.0, 1.25, 0.7, 0.95],
    travel: [1.1, 1.4, 0.8, 1.1],
};

/// Ornament editorial: mask wipes and serif fades.
pub const ORNAMENT: Vocabulary = Vocabulary {
    name: "ornament_editorial",
    entrances: &[
        (En::WipeLeft, 2),
        (En::WipeRight, 2),
        (En::WipeUp, 2),
        (En::FocusPull, 1),
    ],
    hook_entrances: &[(En::WipeRight, 2), (En::WipeUp, 2), (En::WipeLeft, 1)],
    exits: &[(Ex::Fade, 3), (Ex::SlideUp, 1), (Ex::SlideLeft, 1)],
    cameras: &[
        (CameraMove::Still, 2),
        (CameraMove::PushIn, 1),
        (CameraMove::PullOut, 1),
    ],
    easings: &[(EasingFamily::Smooth, 3), (EasingFamily::Builder, 1)],
    staggers: &[(StaggerPreset::Loose, 2), (StaggerPreset::Builder, 1)],
    orders: &[(StaggerOrder::Forward, 3), (StaggerOrder::CenterOut, 1)],
    accents: &[
        (AccentMark::Rule, 2),
        (AccentMark::Underline, 2),
        (AccentMark::Builder, 1),
    ],
    amplitude: [1.0, 1.2, 0.65, 0.85],
    travel: [1.0, 1.2, 0.7, 0.9],
};

/// Halftone cutout: hard pops and cuts.
pub const HALFTONE: Vocabulary = Vocabulary {
    name: "halftone_cutout",
    entrances: &[(En::ScalePop, 3), (En::SlideLeft, 1), (En::SlideRight, 1)],
    hook_entrances: &[(En::ScalePop, 3), (En::SlideRight, 1), (En::SlideLeft, 1)],
    exits: &[(Ex::Cut, 3), (Ex::ScaleDown, 2), (Ex::SlideLeft, 1)],
    cameras: &[
        (CameraMove::Still, 3),
        (CameraMove::PushIn, 1),
        (CameraMove::Truck, 1),
    ],
    easings: &[(EasingFamily::Snappy, 3), (EasingFamily::Builder, 1)],
    staggers: &[(StaggerPreset::Tight, 3), (StaggerPreset::Builder, 1)],
    orders: &[(StaggerOrder::Forward, 3), (StaggerOrder::Reverse, 1)],
    accents: &[
        (AccentMark::Stamp, 2),
        (AccentMark::Builder, 2),
        (AccentMark::Slab, 1),
    ],
    amplitude: [1.1, 1.35, 0.8, 1.0],
    travel: [1.0, 1.3, 0.8, 1.0],
};

/// Clay pop: springs.
pub const CLAY: Vocabulary = Vocabulary {
    name: "clay_pop",
    entrances: &[(En::ScalePop, 3), (En::Rise, 2), (En::SlideRight, 1)],
    hook_entrances: &[(En::ScalePop, 3), (En::Rise, 1), (En::SlideRight, 1)],
    exits: &[(Ex::ScaleDown, 2), (Ex::SlideUp, 2), (Ex::Fade, 1)],
    cameras: &[
        (CameraMove::PushIn, 2),
        (CameraMove::Still, 1),
        (CameraMove::PullOut, 1),
    ],
    easings: &[(EasingFamily::Spring, 4)],
    staggers: &[
        (StaggerPreset::Even, 2),
        (StaggerPreset::Tight, 1),
        (StaggerPreset::Builder, 1),
    ],
    orders: &[(StaggerOrder::Forward, 2), (StaggerOrder::CenterOut, 1)],
    accents: &[
        (AccentMark::Builder, 2),
        (AccentMark::Underline, 1),
        (AccentMark::Rule, 1),
    ],
    amplitude: [1.1, 1.35, 0.75, 1.0],
    travel: [1.0, 1.3, 0.8, 1.0],
};

/// Classical neon: slow fades and a light sweep.
pub const NEON: Vocabulary = Vocabulary {
    name: "classical_neon",
    entrances: &[(En::FocusPull, 3), (En::Rise, 2), (En::WipeRight, 1)],
    hook_entrances: &[(En::WipeRight, 2), (En::Rise, 2), (En::FocusPull, 1)],
    exits: &[(Ex::Fade, 4), (Ex::SlideUp, 1)],
    cameras: &[
        (CameraMove::PushIn, 2),
        (CameraMove::PullOut, 1),
        (CameraMove::Still, 1),
    ],
    easings: &[(EasingFamily::Smooth, 4)],
    staggers: &[(StaggerPreset::Loose, 3)],
    orders: &[(StaggerOrder::Forward, 3), (StaggerOrder::CenterOut, 1)],
    accents: &[(AccentMark::Builder, 3), (AccentMark::Rule, 1)],
    amplitude: [1.0, 1.2, 0.6, 0.85],
    travel: [0.9, 1.1, 0.6, 0.85],
};

/// Energetic flat (classic playful / street / hype without a genre look).
pub const ENERGETIC: Vocabulary = Vocabulary {
    name: "energetic",
    entrances: &[
        (En::ScalePop, 2),
        (En::SlideLeft, 2),
        (En::SlideRight, 2),
        (En::WipeUp, 1),
    ],
    hook_entrances: &[(En::ScalePop, 2), (En::SlideLeft, 1), (En::SlideRight, 1)],
    exits: &[(Ex::Cut, 2), (Ex::ScaleDown, 1), (Ex::SlideLeft, 1)],
    cameras: &[
        (CameraMove::PushIn, 2),
        (CameraMove::Still, 1),
        (CameraMove::Truck, 1),
    ],
    easings: &[(EasingFamily::Snappy, 3), (EasingFamily::Spring, 1)],
    staggers: &[(StaggerPreset::Tight, 3), (StaggerPreset::Even, 1)],
    orders: &[(StaggerOrder::Forward, 3), (StaggerOrder::Reverse, 1)],
    accents: &[
        (AccentMark::Builder, 2),
        (AccentMark::Stamp, 1),
        (AccentMark::Slab, 1),
    ],
    amplitude: [1.1, 1.4, 0.8, 1.05],
    travel: [1.1, 1.4, 0.8, 1.1],
};

/// The genre looks keep their signature: their builders own entrance, camera
/// and exit; the planner varies only within it (C2c reads these).
const fn genre(
    name: &'static str,
    entrances: &'static [(EntranceFamily, u8)],
    exits: &'static [(ExitFamily, u8)],
    easings: &'static [(EasingFamily, u8)],
    staggers: &'static [(StaggerPreset, u8)],
) -> Vocabulary {
    Vocabulary {
        name,
        entrances,
        hook_entrances: entrances,
        exits,
        cameras: &[(CameraMove::Builder, 1)],
        easings,
        staggers,
        orders: &[(StaggerOrder::Forward, 1)],
        accents: &[(AccentMark::Builder, 1)],
        amplitude: [1.0, 1.2, 0.85, 1.0],
        travel: [1.0, 1.2, 0.85, 1.0],
    }
}

pub const DOSSIER: Vocabulary = genre(
    "dossier",
    &[(En::Builder, 3), (En::SlideLeft, 1), (En::Rise, 1)],
    &[(Ex::Builder, 3), (Ex::SlideLeft, 1)],
    &[(EasingFamily::Builder, 2), (EasingFamily::Smooth, 1)],
    &[(StaggerPreset::Builder, 2), (StaggerPreset::Loose, 1)],
);
pub const STREET: Vocabulary = genre(
    "street_collage",
    &[(En::Builder, 3), (En::ScalePop, 1), (En::SlideRight, 1)],
    &[(Ex::Builder, 2), (Ex::Cut, 1)],
    &[(EasingFamily::Builder, 2), (EasingFamily::Snappy, 1)],
    &[(StaggerPreset::Builder, 2), (StaggerPreset::Tight, 1)],
);
pub const HYPE: Vocabulary = genre(
    "hype_slam",
    &[(En::Builder, 4), (En::ScalePop, 1)],
    &[(Ex::Builder, 3), (Ex::Cut, 2)],
    &[(EasingFamily::Builder, 3), (EasingFamily::Snappy, 1)],
    &[(StaggerPreset::Builder, 2), (StaggerPreset::Tight, 2)],
);
pub const STUDIO: Vocabulary = genre(
    "studio_pop",
    &[(En::Builder, 3), (En::ScalePop, 1), (En::Rise, 1)],
    &[(Ex::Builder, 3), (Ex::ScaleDown, 1)],
    &[(EasingFamily::Builder, 2), (EasingFamily::Spring, 1)],
    &[(StaggerPreset::Builder, 2), (StaggerPreset::Even, 1)],
);
pub const CINEMATIC: Vocabulary = genre(
    "cinematic_3d",
    &[(En::Builder, 4), (En::FocusPull, 1)],
    &[(Ex::Builder, 1)],
    &[(EasingFamily::Builder, 3), (EasingFamily::Smooth, 1)],
    &[(StaggerPreset::Builder, 2), (StaggerPreset::Loose, 1)],
);

/// The vocabulary for a look (or the classic compile's tone when no look is
/// art-directed). Technical tones get the precision vocabulary in every flat
/// look, so calm and precision never share a motion vocabulary.
pub fn vocabulary(look: Option<Look>, tone: Tone) -> &'static Vocabulary {
    if tone == Tone::Technical
        && !matches!(
            look,
            Some(
                Look::Dossier
                    | Look::StreetCollage
                    | Look::HypeSlam
                    | Look::StudioPop
                    | Look::Cinematic3d
            )
        )
    {
        return &PRECISION;
    }
    match look {
        Some(Look::ClassicalNeon) => &NEON,
        Some(Look::HalftoneCutout) => &HALFTONE,
        Some(Look::ClayPop) => &CLAY,
        Some(Look::OrnamentEditorial) => &ORNAMENT,
        Some(Look::Journey) => &JOURNEY,
        Some(Look::StreetCollage) => &STREET,
        Some(Look::Dossier) => &DOSSIER,
        Some(Look::HypeSlam) => &HYPE,
        Some(Look::StudioPop) => &STUDIO,
        Some(Look::Cinematic3d) => &CINEMATIC,
        None => match tone {
            Tone::Playful | Tone::Street | Tone::Hype | Tone::Studio => &ENERGETIC,
            _ => &CALM,
        },
    }
}

/// A deterministic weighted pick; `avoid` is skipped when another option
/// exists.
fn pick<T: Copy + PartialEq>(
    seed: u64,
    beat: usize,
    dim: Dim,
    options: &[(T, u8)],
    avoid: Option<T>,
) -> T {
    let usable: Vec<(T, u8)> = options
        .iter()
        .copied()
        .filter(|&(v, w)| w > 0 && Some(v) != avoid)
        .collect();
    let set: &[(T, u8)] = if usable.is_empty() { options } else { &usable };
    let total: u64 = set.iter().map(|&(_, w)| w as u64).sum::<u64>().max(1);
    let mut r = choice(seed, beat, dim) % total;
    for &(v, w) in set {
        if r < w as u64 {
            return v;
        }
        r -= w as u64;
    }
    set[0].0
}

/// Restrict a pick to the options a role allows (falls back to all).
fn allowed<T: Copy + PartialEq>(options: &[(T, u8)], keep: &[T]) -> Vec<(T, u8)> {
    let v: Vec<(T, u8)> = options
        .iter()
        .copied()
        .filter(|(o, _)| keep.contains(o))
        .collect();
    if v.is_empty() {
        options.to_vec()
    } else {
        v
    }
}

/// The options one beat may draw from, given its role (weights kept).
struct BeatOptions {
    entrances: Vec<(EntranceFamily, u8)>,
    cameras: Vec<(CameraMove, u8)>,
    exits: Vec<(ExitFamily, u8)>,
    easings: Vec<(EasingFamily, u8)>,
    staggers: Vec<(StaggerPreset, u8)>,
    orders: Vec<(StaggerOrder, u8)>,
    accents: Vec<(AccentMark, u8)>,
}

fn beat_options(role: Role, vocab: &Vocabulary) -> BeatOptions {
    let calm = role == Role::Calm;
    let payoff = role == Role::Payoff;
    let entrances = if role == Role::Hook {
        vocab.hook_entrances
    } else {
        vocab.entrances
    };
    let cameras = if calm {
        allowed(
            vocab.cameras,
            &[CameraMove::Still, CameraMove::PushIn, CameraMove::Builder],
        )
    } else if payoff {
        allowed(vocab.cameras, &[CameraMove::PushIn, CameraMove::Builder])
    } else {
        vocab.cameras.to_vec()
    };
    let exits = if calm {
        allowed(vocab.exits, &[ExitFamily::Fade, ExitFamily::Builder])
    } else {
        vocab.exits.to_vec()
    };
    let easings = if calm {
        allowed(
            vocab.easings,
            &[EasingFamily::Smooth, EasingFamily::Builder],
        )
    } else if role == Role::Hook {
        allowed(vocab.easings, &[EasingFamily::Snappy, EasingFamily::Spring])
    } else {
        vocab.easings.to_vec()
    };
    let staggers = if calm {
        allowed(
            vocab.staggers,
            &[StaggerPreset::Loose, StaggerPreset::Builder],
        )
    } else {
        vocab.staggers.to_vec()
    };
    let accents = if payoff {
        // The strongest mark the look allows.
        let strongest = [
            AccentMark::Stamp,
            AccentMark::Slab,
            AccentMark::Underline,
            AccentMark::Rule,
            AccentMark::Builder,
        ]
        .into_iter()
        .find(|m| vocab.accents.iter().any(|(a, _)| a == m))
        .unwrap_or(AccentMark::Builder);
        vec![(strongest, 1)]
    } else {
        vocab.accents.to_vec()
    };
    BeatOptions {
        entrances: entrances.to_vec(),
        cameras,
        exits,
        easings,
        staggers,
        orders: vocab.orders.to_vec(),
        accents,
    }
}

/// Plan every beat's params from the take seed, the roles and the look's
/// vocabulary. Pure and deterministic.
pub fn plan_params(seed: u64, roles: &[Role], vocab: &Vocabulary) -> Vec<BeatParams> {
    let mut out: Vec<BeatParams> = Vec::with_capacity(roles.len());
    for (i, &role) in roles.iter().enumerate() {
        let prev = out.last().copied();
        let o = beat_options(role, vocab);
        out.push(BeatParams {
            entrance: pick(
                seed,
                i,
                Dim::Entrance,
                &o.entrances,
                prev.map(|p| p.entrance),
            ),
            travel: vocab.travel[role.index()],
            stagger: pick(seed, i, Dim::Stagger, &o.staggers, None),
            stagger_order: pick(seed, i, Dim::Travel, &o.orders, None),
            easing: pick(seed, i, Dim::Easing, &o.easings, None),
            accent: pick(seed, i, Dim::Accent, &o.accents, None),
            exit: pick(seed, i, Dim::Exit, &o.exits, prev.map(|p| p.exit)),
            camera: pick(seed, i, Dim::Camera, &o.cameras, prev.map(|p| p.camera)),
            read_share: 1.0,
            amplitude: vocab.amplitude[role.index()],
        });
    }
    out
}

/// The option `shift` places after `current` in `options` (distinct values in
/// weight order), skipping `avoid` when another value exists.
fn rotate<T: Copy + PartialEq>(
    options: &[(T, u8)],
    current: T,
    shift: usize,
    avoid: Option<T>,
) -> T {
    let mut distinct: Vec<T> = Vec::new();
    for &(v, _) in options {
        if !distinct.contains(&v) {
            distinct.push(v);
        }
    }
    let n = distinct.len();
    if n <= 1 {
        return current;
    }
    let at = distinct.iter().position(|&v| v == current).unwrap_or(0);
    // A shift that would come back to take 0's value moves one step further:
    // every take differs from take 0 wherever an alternative exists.
    let shift = if shift.is_multiple_of(n) {
        shift + 1
    } else {
        shift
    };
    for extra in 0..n {
        let v = distinct[(at + shift + extra) % n];
        if Some(v) != avoid || n == 1 {
            return v;
        }
    }
    current
}

/// The params of take `take`, candidate `k`: candidate `k`'s draw of take 0
/// (the story's variety seed; candidate 0 is take 0 itself) with every
/// dimension rotated `take` steps through the beat's options, neighbours kept
/// distinct. So another take changes every dimension the look offers an
/// alternative for, instead of redrawing it and often landing on the same
/// choice, and best-of-N candidates are independent draws. Take 0, candidate
/// 0 is `plan_params(take_seed(variety, 0))`.
pub fn plan_take(
    variety: u64,
    take: u64,
    k: u8,
    roles: &[Role],
    vocab: &Vocabulary,
) -> Vec<BeatParams> {
    let base = plan_params(candidate_seed(take_seed(variety, 0), k), roles, vocab);
    let shift = take as usize;
    if shift == 0 {
        return base;
    }
    let mut out: Vec<BeatParams> = Vec::with_capacity(base.len());
    for (i, (&role, b)) in roles.iter().zip(&base).enumerate() {
        let prev = out.last().copied();
        let o = beat_options(role, vocab);
        out.push(BeatParams {
            entrance: rotate(&o.entrances, b.entrance, shift, prev.map(|p| p.entrance)),
            stagger: rotate(&o.staggers, b.stagger, shift, None),
            stagger_order: rotate(&o.orders, b.stagger_order, shift, None),
            easing: rotate(&o.easings, b.easing, shift, None),
            accent: rotate(&o.accents, b.accent, shift, None),
            exit: rotate(&o.exits, b.exit, shift, prev.map(|p| p.exit)),
            camera: rotate(&o.cameras, b.camera, shift, prev.map(|p| p.camera)),
            ..*b
        });
        let _ = i;
    }
    out
}

// ---------------------------------------------------------------------------
// The record written to ProjectMeta.direction (variety compiles only)
// ---------------------------------------------------------------------------

/// What the direction layer chose for one beat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeatDirection {
    /// Beat index (0-based).
    pub beat: usize,
    /// Grammar name of the template used (`Grammar::name`).
    pub template: String,
    /// Index of the template among `grammar::candidates` (0 = the semantic
    /// choice).
    pub alternate: u8,
    /// How many template candidates the beat had.
    pub alternates: u8,
    #[serde(default, skip_serializing_if = "BeatParams::is_identity")]
    pub params: BeatParams,
    /// Seeded rotations that replaced a position rotation, by dimension
    /// name (`arrival`, `camera`, `slam_layout`, `image_layout`) → option
    /// name.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub rotations: std::collections::BTreeMap<String, String>,
    /// The beat's role in the arc: `hook`, `payoff`, `calm` or `body`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    pub reason: String,
}

/// One best-of-N candidate's score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateScore {
    pub k: u8,
    pub seed: u64,
    /// Names of the hard checks it failed (empty = eligible).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hard: Vec<String>,
    /// Total soft score (higher is better).
    pub soft: f64,
    /// Soft score parts by name (variety, focal, density, balance, dead_air).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub parts: std::collections::BTreeMap<String, f64>,
    /// True when pixel checks ran (the structural top 2).
    #[serde(default)]
    pub pixel: bool,
}

/// `ProjectMeta.direction`: what the direction layer did (variety only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirectionRecord {
    pub take: u64,
    /// The take seed ([`take_seed`]).
    pub seed: u64,
    /// The candidate that shipped (0 when N = 1).
    #[serde(default)]
    pub chosen: u8,
    pub beats: Vec<BeatDirection>,
    /// Every candidate's score (empty when N = 1).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<CandidateScore>,
    /// Set when every candidate failed a hard check and the best one shipped
    /// anyway (never silent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shipped_with_failure: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_zero_is_the_variety_seed() {
        assert_eq!(take_seed(42, 0), 42);
        assert_ne!(take_seed(42, 1), 42);
        assert_ne!(take_seed(42, 1), take_seed(42, 2));
        assert_eq!(candidate_seed(7, 0), 7);
        assert_ne!(candidate_seed(7, 1), candidate_seed(7, 2));
    }

    #[test]
    fn dimensions_are_independent_streams() {
        assert_ne!(choice(1, 0, Dim::Arrival), choice(1, 0, Dim::Camera));
        assert_ne!(choice(1, 0, Dim::Arrival), choice(1, 1, Dim::Arrival));
        assert_eq!(choice(1, 3, Dim::Bed), choice(1, 3, Dim::Bed));
    }

    #[test]
    fn seeded_sequences_never_repeat_neighbours() {
        for seed in 0..200u64 {
            for options in 2..6 {
                let s = seeded_sequence(seed, Dim::Arrival, 12, options);
                assert_eq!(s.len(), 12);
                assert!(s.iter().all(|&i| i < options));
                assert!(s.windows(2).all(|w| w[0] != w[1]), "{seed} {options} {s:?}");
            }
        }
        assert!(seeded_sequence(3, Dim::Camera, 4, 0).is_empty());
        assert_eq!(seeded_sequence(3, Dim::Camera, 3, 1), vec![0, 0, 0]);
        // Different takes give different sequences.
        let a = seeded_sequence(take_seed(9, 0), Dim::Arrival, 8, 4);
        let b = seeded_sequence(take_seed(9, 1), Dim::Arrival, 8, 4);
        assert_ne!(a, b);
    }

    #[test]
    fn default_params_are_the_identity_and_skip_in_json() {
        let d = BeatDirection {
            beat: 0,
            template: "kinetic_poster".into(),
            alternate: 0,
            alternates: 1,
            params: BeatParams::default(),
            rotations: Default::default(),
            role: String::new(),
            reason: "semantic".into(),
        };
        let j = serde_json::to_string(&d).unwrap();
        assert!(!j.contains("params"), "{j}");
        let back: BeatDirection = serde_json::from_str(&j).unwrap();
        assert!(back.params.is_identity());
    }

    #[test]
    fn identity_params_return_the_builders_own_values() {
        let p = BeatParams::default();
        assert_eq!(entrance_direction(&p, Direction::Down), Direction::Down);
        assert_eq!(entrance_reveal(&p, Reveal::Mask), Reveal::Mask);
        assert_eq!(entrance_easing(&p, Easing::InCubic), Easing::InCubic);
        assert_eq!(travel(&p, 37.5), 37.5);
        assert_eq!(amplitude(&p, 0.04), 0.04);
        let o = vec![0.0, 0.07, 0.14, 0.21];
        assert_eq!(stagger_offsets(&p, o.clone()), o);
    }

    #[test]
    fn params_steer_each_slot() {
        let p = BeatParams {
            entrance: EntranceFamily::WipeLeft,
            easing: EasingFamily::Snappy,
            travel: 1.5,
            amplitude: 0.5,
            stagger: StaggerPreset::Tight,
            stagger_order: StaggerOrder::Reverse,
            ..BeatParams::default()
        };
        assert_eq!(entrance_direction(&p, Direction::Up), Direction::Left);
        assert_eq!(entrance_reveal(&p, Reveal::Clip), Reveal::Mask);
        assert_eq!(entrance_easing(&p, Easing::OutCubic), Easing::OutQuint);
        assert_eq!(travel(&p, 40.0), 60.0);
        assert_eq!(amplitude(&p, 16.0), 8.0);
        let o = stagger_offsets(&p, vec![0.0, 0.1, 0.2]);
        assert!((o[0] - 0.12).abs() < 1e-9 && o[2].abs() < 1e-9, "{o:?}");
        // A family with no edge (scale pop) keeps the builder's direction.
        let pop = BeatParams {
            entrance: EntranceFamily::ScalePop,
            ..BeatParams::default()
        };
        assert_eq!(entrance_direction(&pop, Direction::Up), Direction::Up);
        assert_eq!(entrance_reveal(&pop, Reveal::Clip), Reveal::Clip);
    }

    #[test]
    fn center_out_starts_in_the_middle() {
        let p = BeatParams {
            stagger_order: StaggerOrder::CenterOut,
            ..BeatParams::default()
        };
        let o = stagger_offsets(&p, vec![0.0, 0.1, 0.2, 0.3, 0.4]);
        assert_eq!(o[2], 0.0);
        assert!(o[1] < o[0] && o[3] < o[4], "{o:?}");
        let mut sorted = o.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        assert_eq!(sorted, vec![0.0, 0.1, 0.2, 0.3, 0.4]);
    }

    fn slots(e: &[Energy], reveal_last: bool) -> Vec<BeatSlot> {
        let n = e.len();
        e.iter()
            .enumerate()
            .map(|(i, &energy)| BeatSlot {
                energy,
                reveal: reveal_last && i + 1 == n,
            })
            .collect()
    }

    #[test]
    fn roles_mark_hook_payoff_and_calm() {
        use Energy::*;
        let r = roles(&slots(&[Building, Calm, Building, Impact, Building], false));
        assert_eq!(
            r,
            vec![Role::Hook, Role::Calm, Role::Body, Role::Payoff, Role::Body]
        );
        // A final reveal is the payoff.
        let r = roles(&slots(&[Calm, Building, Building], true));
        assert_eq!(r, vec![Role::Hook, Role::Body, Role::Payoff]);
        // No reveal or impact: the highest-energy beat of the last third.
        let r = roles(&slots(&[Building, Calm, Calm, Building, Calm, Calm], false));
        assert_eq!(r[3], Role::Body);
        assert_eq!(r.iter().filter(|&&x| x == Role::Payoff).count(), 1);
        assert_eq!(roles(&slots(&[Impact], false)), vec![Role::Hook]);
    }

    #[test]
    fn plans_are_deterministic_and_never_repeat_neighbours() {
        use Energy::*;
        let r = roles(&slots(
            &[
                Building, Building, Calm, Building, Impact, Building, Building, Calm,
            ],
            true,
        ));
        for vocab in [
            &CALM, &PRECISION, &JOURNEY, &ORNAMENT, &HALFTONE, &CLAY, &NEON, &ENERGETIC, &DOSSIER,
            &STREET, &HYPE, &STUDIO, &CINEMATIC,
        ] {
            for take in 0..6u64 {
                let seed = take_seed(0xABCD, take);
                let a = plan_params(seed, &r, vocab);
                assert_eq!(a, plan_params(seed, &r, vocab), "{}", vocab.name);
                for (k, w) in a.windows(2).enumerate() {
                    let options = |n: usize| n > 1;
                    if options(vocab.entrances.len()) && options(vocab.hook_entrances.len()) {
                        assert_ne!(w[0].entrance, w[1].entrance, "{} {take}", vocab.name);
                    }
                    // Calm beats may repeat a fade: restraint outranks exit variety.
                    let calm = r[k] == Role::Calm || r[k + 1] == Role::Calm;
                    if options(vocab.exits.len()) && !calm {
                        assert_ne!(w[0].exit, w[1].exit, "{} {take}", vocab.name);
                    }
                }
                for (p, role) in a.iter().zip(&r) {
                    if *role == Role::Calm {
                        assert!(p.amplitude <= 0.85, "{}", vocab.name);
                        assert!(matches!(
                            p.camera,
                            CameraMove::Still | CameraMove::PushIn | CameraMove::Builder
                        ));
                    }
                }
                let payoff = r.iter().position(|&x| x == Role::Payoff).unwrap();
                assert!(a.iter().all(|p| p.amplitude <= a[payoff].amplitude + 1e-6));
            }
        }
    }

    #[test]
    fn takes_change_the_plan() {
        use Energy::*;
        let r = roles(&slots(&[Building, Building, Building, Impact, Calm], true));
        let distinct: std::collections::BTreeSet<String> = (0..4u64)
            .map(|t| format!("{:?}", plan_params(take_seed(77, t), &r, &CALM)))
            .collect();
        assert!(distinct.len() >= 3, "{distinct:?}");
    }

    #[test]
    fn another_take_rotates_every_dimension_with_an_alternative() {
        use Energy::*;
        let r = roles(&slots(
            &[Building, Building, Calm, Building, Impact, Building],
            true,
        ));
        let flat = [
            &CALM, &PRECISION, &JOURNEY, &ORNAMENT, &HALFTONE, &CLAY, &NEON, &ENERGETIC,
        ];
        for vocab in flat.into_iter().chain([&HYPE, &DOSSIER, &STUDIO]) {
            let t0 = plan_take(99, 0, 0, &r, vocab);
            assert_eq!(t0, plan_params(take_seed(99, 0), &r, vocab));
            for take in 1..4u64 {
                let t = plan_take(99, take, 0, &r, vocab);
                assert_eq!(t, plan_take(99, take, 0, &r, vocab));
                assert_ne!(t, t0, "{} take {take}", vocab.name);
                for w in t.windows(2) {
                    assert_ne!(w[0].entrance, w[1].entrance, "{}", vocab.name);
                }
                if flat.iter().any(|v| v.name == vocab.name) && take < 3 {
                    // Flat looks offer 3+ options per motion dimension: most
                    // beats change entrance, camera or exit.
                    let changed = t0
                        .iter()
                        .zip(&t)
                        .filter(|(a, b)| {
                            (a.entrance, a.camera, a.exit) != (b.entrance, b.camera, b.exit)
                        })
                        .count();
                    assert!(
                        changed * 3 >= r.len() * 2,
                        "{} take {take}: {changed}",
                        vocab.name
                    );
                }
            }
            // Candidates of a take differ from the take too.
            assert_ne!(
                plan_take(99, 1, 1, &r, vocab),
                plan_take(99, 1, 0, &r, vocab)
            );
        }
    }

    #[test]
    fn calm_and_precision_never_share_a_vocabulary() {
        assert_eq!(
            vocabulary(Some(Look::Journey), Tone::Technical).name,
            "precision"
        );
        assert_eq!(
            vocabulary(Some(Look::Journey), Tone::Editorial).name,
            "journey"
        );
        assert_eq!(vocabulary(None, Tone::Editorial).name, "calm");
        assert_eq!(vocabulary(None, Tone::Technical).name, "precision");
        // Genre looks keep their own vocabulary under any tone.
        assert_eq!(
            vocabulary(Some(Look::Dossier), Tone::Technical).name,
            "dossier"
        );
        let set = |v: &Vocabulary| {
            v.entrances
                .iter()
                .map(|e| format!("{:?}", e.0))
                .collect::<std::collections::BTreeSet<_>>()
        };
        for calm in [&CALM, &JOURNEY] {
            assert!(set(calm).is_disjoint(&set(&PRECISION)) || calm.easings != PRECISION.easings);
            assert_ne!(
                calm.easings.iter().map(|e| e.0).collect::<Vec<_>>(),
                PRECISION.easings.iter().map(|e| e.0).collect::<Vec<_>>()
            );
        }
    }
}

#[cfg(test)]
mod c2b_tests {
    //! (0.23 C2b) The helpers for builders that place layers through shared
    //! code: each returns the builder's own value for the identity.
    use super::*;
    use crate::scene::{GlyphOrder, RevealMode};

    fn planned(f: impl FnOnce(&mut BeatParams)) -> BeatParams {
        let mut p = BeatParams::default();
        f(&mut p);
        p
    }

    fn motion(target: &str, start: f64, duration: f64, easing: Easing, op: MotionOp) -> Motion {
        Motion {
            spring: None,
            id: None,
            target: target.to_string(),
            start,
            duration,
            easing,
            op,
        }
    }

    fn scale(target: &str, start: f64, duration: f64, from: f32, to: f32) -> Motion {
        motion(
            target,
            start,
            duration,
            Easing::InOutCubic,
            MotionOp::Scale {
                from,
                to,
                axis: Default::default(),
            },
        )
    }

    /// A placed text layer's motions as `place_subject` writes them (a fade
    /// and a rise), a wipe, a scale pop, and a later event that is not an
    /// entrance (a pressure move).
    fn written() -> Vec<Motion> {
        vec![
            motion(
                "t",
                1.0,
                0.5,
                Easing::OutCubic,
                MotionOp::Fade { from: 0.0, to: 1.0 },
            ),
            motion(
                "t",
                1.0,
                0.8,
                Easing::OutQuint,
                MotionOp::Move {
                    from: [0.0, 60.0],
                    to: [0.0, 0.0],
                },
            ),
            motion(
                "t",
                3.0,
                0.7,
                Easing::InOutCubic,
                MotionOp::Move {
                    from: [0.0, 0.0],
                    to: [0.0, 40.0],
                },
            ),
            motion(
                "w",
                1.0,
                0.8,
                Easing::OutQuint,
                MotionOp::MaskReveal {
                    direction: Direction::Down,
                    mode: RevealMode::Reveal,
                },
            ),
            scale("p", 1.0, 1.0, 1.12, 1.0),
        ]
    }

    fn text(id: &str, s: &str) -> Placed {
        Placed {
            id: id.to_string(),
            glyphs: Some(s.chars().filter(|c| !c.is_whitespace()).count()),
        }
    }

    fn all() -> Vec<Placed> {
        vec![
            text("t", "one idea"),
            Placed {
                id: "w".into(),
                glyphs: None,
            },
            Placed {
                id: "p".into(),
                glyphs: None,
            },
        ]
    }

    #[test]
    fn scale_amplitude_scales_the_distance_from_one() {
        let id = BeatParams::default();
        for f in [0.3_f32, 0.9, 0.975, 1.04, 1.08, 1.2, 2.5] {
            assert_eq!(scale_amplitude(&id, f), f);
        }
        let big = planned(|p| p.amplitude = 1.3);
        assert!((scale_amplitude(&big, 1.2) - 1.26).abs() < 1e-6);
        assert!((scale_amplitude(&big, 0.9) - 0.87).abs() < 1e-6);
        let small = planned(|p| p.amplitude = 0.7);
        assert!((scale_amplitude(&small, 1.2) - 1.14).abs() < 1e-6);
        // Never collapses.
        assert!(scale_amplitude(&planned(|p| p.amplitude = 40.0), 0.5) >= 0.05);
    }

    #[test]
    fn the_accent_picks_the_shape_of_a_decoration() {
        use MarkShape::{Rule, Underline};
        let of = |a| accent_shape(&planned(|p| p.accent = a), Rule);
        assert_eq!(of(AccentMark::Builder), Some(Rule));
        assert_eq!(of(AccentMark::None), None);
        assert_eq!(of(AccentMark::Rule), Some(Rule));
        assert_eq!(of(AccentMark::Underline), Some(Underline));
        // A builder with no slab or stamp decoration keeps its own mark.
        assert_eq!(of(AccentMark::Slab), Some(Rule));
        assert_eq!(of(AccentMark::Stamp), Some(Rule));
        assert_eq!(
            accent_shape(&planned(|p| p.accent = AccentMark::Stamp), Underline),
            Some(Underline)
        );
    }

    #[test]
    fn steering_the_identity_changes_nothing() {
        let mut m = written();
        steer_entrances(&BeatParams::default(), &mut m, &all());
        assert_eq!(m, written());
        // `Even` is the identity of the stagger too.
        let mut m = written();
        let near = planned(|p| p.stagger = StaggerPreset::Even);
        steer_entrances(&near, &mut m, &all());
        assert_eq!(m, written());
    }

    #[test]
    fn steering_rewrites_the_entrance_and_nothing_else() {
        let p = planned(|p| {
            p.entrance = EntranceFamily::WipeLeft;
            p.easing = EasingFamily::Snappy;
            p.travel = 1.5;
            p.amplitude = 1.3;
        });
        let mut m = written();
        steer_entrances(&p, &mut m, &all());
        // The wipe: planned side, kind and easing.
        assert!(matches!(
            m[3].op,
            MotionOp::MaskReveal {
                direction: Direction::Left,
                ..
            }
        ));
        assert_eq!(m[3].easing, Easing::OutQuint);
        // The rise: travel x1.5, planned easing.
        assert_eq!(
            m[1].op,
            MotionOp::Move {
                from: [0.0, 90.0],
                to: [0.0, 0.0]
            }
        );
        assert_eq!(m[1].easing, Easing::OutQuint);
        // The pop: amplitude on its start.
        assert!(matches!(m[4].op, MotionOp::Scale { from, .. } if (from - 1.156).abs() < 1e-4));
        // Not entrances: the fade, and the later pressure move.
        assert_eq!(m[0], written()[0]);
        assert_eq!(m[2], written()[2]);
        // A slide turns the mask into a clip reveal.
        let slide = planned(|p| p.entrance = EntranceFamily::SlideRight);
        let mut m = written();
        steer_entrances(&slide, &mut m, &all());
        assert!(matches!(
            m[3].op,
            MotionOp::ClipReveal {
                direction: Direction::Right,
                ..
            }
        ));
    }

    #[test]
    fn scale_pop_replaces_the_rise_unless_a_scale_runs_there() {
        let p = planned(|p| {
            p.entrance = EntranceFamily::ScalePop;
            p.amplitude = 1.3;
        });
        let mut m = written();
        steer_entrances(&p, &mut m, &all());
        let MotionOp::Scale { from, to, .. } = m[1].op else {
            panic!("{:?}", m[1])
        };
        assert!((from - (1.0 - 0.12 * 1.3)).abs() < 1e-6 && to == 1.0);
        assert_eq!((m[1].start, m[1].duration), (1.0, 1.12));
        // Another scale in the pop's window: the rise stays.
        let mut m = written();
        m.push(scale("t", 1.5, 0.5, 1.0, 0.7));
        steer_entrances(&p, &mut m, &all());
        assert!(matches!(m[1].op, MotionOp::Move { .. }));
        // A scale from 2.5 on leaves the whole pop (1.0 to 2.12) room.
        let mut m = written();
        m.push(scale("t", 2.5, 0.5, 1.0, 1.04));
        steer_entrances(&p, &mut m, &all());
        assert!(matches!(m[1].op, MotionOp::Scale { .. }));
        assert_eq!(m[1].duration, 1.12);
        // One at 1.9 cuts the pop short, at the entrance's own length at least.
        let mut m = written();
        m.push(scale("t", 1.9, 0.5, 1.0, 1.04));
        steer_entrances(&p, &mut m, &all());
        assert!(matches!(m[1].op, MotionOp::Scale { .. }));
        assert_eq!(m[1].duration, 0.9);
        // One already running when the rise starts is a clash.
        let mut m = written();
        m.push(scale("t", 0.5, 1.0, 1.0, 1.04));
        steer_entrances(&p, &mut m, &all());
        assert!(matches!(m[1].op, MotionOp::Move { .. }));
    }

    #[test]
    fn glyph_cascade_needs_text_that_does_not_count() {
        let p = planned(|p| {
            p.entrance = EntranceFamily::GlyphCascade;
            p.stagger = StaggerPreset::Tight;
            p.stagger_order = StaggerOrder::CenterOut;
        });
        let mut m = written();
        steer_entrances(&p, &mut m, &all());
        let MotionOp::GlyphCascade {
            stagger,
            from,
            order,
            ..
        } = m[1].op
        else {
            panic!("{:?}", m[1])
        };
        assert_eq!(order, GlyphOrder::Center);
        assert!((stagger - 0.018).abs() < 1e-9, "{stagger}");
        assert_eq!((from.dy, from.opacity), (60.0, 0.0));
        assert_eq!((m[1].start, m[1].duration), (1.0, 0.8));
        // The fade stays: the text is hidden until its time.
        assert_eq!(m[0], written()[0]);
        // A counting number, a picture and a one-glyph text keep their rise.
        let mut counted = written();
        counted.push(motion(
            "t",
            1.0,
            1.0,
            Easing::OutQuint,
            MotionOp::Count {
                from: 0.0,
                to: 5.0,
                decimals: 0,
                grouping: false,
                prefix: String::new(),
                suffix: String::new(),
            },
        ));
        steer_entrances(&p, &mut counted, &all());
        assert!(matches!(counted[1].op, MotionOp::Move { .. }));
        let mut picture = written();
        let only = [Placed {
            id: "t".into(),
            glyphs: None,
        }];
        steer_entrances(&p, &mut picture, &only);
        assert!(matches!(picture[1].op, MotionOp::Move { .. }));
        let mut one = written();
        steer_entrances(&p, &mut one, &[text("t", "x")]);
        assert!(matches!(one[1].op, MotionOp::Move { .. }));
        // A long text spreads its glyph starts over a share of the entrance.
        let mut long = written();
        steer_entrances(&p, &mut long, &[text("t", &"abcdefghij".repeat(5))]);
        let MotionOp::GlyphCascade { stagger, .. } = long[1].op else {
            panic!("{:?}", long[1])
        };
        assert!(stagger * 49.0 <= 0.4 * 0.8 + 1e-3, "{stagger}");
    }

    #[test]
    fn focus_pull_falls_back() {
        let mut m = written();
        let p = planned(|p| p.entrance = EntranceFamily::FocusPull);
        steer_entrances(&p, &mut m, &all());
        assert_eq!(m, written());
    }

    #[test]
    fn a_stamp_enters_through_one_clip_reveal_at_the_identity() {
        let one = stamp_entrance(
            &BeatParams::default(),
            "s",
            2.0,
            0.7,
            Easing::ImpactSpring,
            3,
            20.0,
        );
        assert_eq!(
            one,
            vec![crate::compiler::mo::line_in(
                "s",
                2.0,
                0.7,
                Direction::Up,
                Easing::ImpactSpring
            )]
        );
        // Planned: the side, the wipe kind and the easing steer it; a pop adds.
        let p = planned(|p| {
            p.entrance = EntranceFamily::ScalePop;
            p.easing = EasingFamily::Smooth;
        });
        let two = stamp_entrance(&p, "s", 2.0, 0.7, Easing::ImpactSpring, 3, 20.0);
        assert_eq!(two.len(), 2);
        assert_eq!(two[0].easing, Easing::OutCubic);
        assert!(matches!(two[1].op, MotionOp::Scale { .. }));
        let wipe = planned(|p| p.entrance = EntranceFamily::WipeRight);
        let m = stamp_entrance(&wipe, "s", 2.0, 0.7, Easing::ImpactSpring, 3, 20.0);
        assert!(matches!(
            m[0].op,
            MotionOp::MaskReveal {
                direction: Direction::Right,
                ..
            }
        ));
        // A cascade rises glyph by glyph behind a fade; a counting stamp (no
        // glyphs given) keeps its reveal.
        let c = planned(|p| {
            p.entrance = EntranceFamily::GlyphCascade;
            p.travel = 1.5;
        });
        let m = stamp_entrance(&c, "s", 2.0, 0.7, Easing::ImpactSpring, 3, 20.0);
        assert!(matches!(m[0].op, MotionOp::Fade { .. }));
        assert!(matches!(
            m[1].op,
            MotionOp::GlyphCascade { from, .. } if from.dy == 30.0
        ));
        let m = stamp_entrance(&c, "s", 2.0, 0.7, Easing::ImpactSpring, 0, 20.0);
        assert!(matches!(m[0].op, MotionOp::ClipReveal { .. }) && m.len() == 1);
    }
}
