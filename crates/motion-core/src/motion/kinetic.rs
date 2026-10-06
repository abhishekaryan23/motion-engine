//! Kinetic typography behaviors. Each expands a measured [`TextRun`] into one
//! Text layer per unit (word, or glyph cluster for tracking) plus motions.
//!
//! Ids: unit layers are `"{run.id}.{line}.{unit}"` (0-based); extra layers use
//! `"{run.id}.{suffix}"`. Times are scene-local seconds. Layers are positioned
//! in the caller's container space from `run.origin`.

use super::stagger::{self, StaggerOrder, StaggerSpec};
use super::Expansion;
use crate::easing::{Easing, PresetParams};
use crate::scene::{
    Axis, Color, Direction, InkBounds, Layer, LayerKind, Motion, MotionOp, RevealMode, TextAlign,
    TextStyle,
};

/// A text block measured by the compiler, broken into units.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    /// Base layer id.
    pub id: String,
    /// Template for every unit layer (`text` is replaced per unit; `align` Left).
    pub style: TextStyle,
    /// Top-left of the block in container space.
    pub origin: [f32; 2],
    /// Distance between line tops in pixels.
    pub line_advance: f32,
    /// Units per line, left to right.
    pub lines: Vec<Vec<UnitBox>>,
    /// Ink extent of one line box (pixels from line top), if measured.
    pub ink: Option<InkBounds>,
}

/// One word (or glyph cluster) measured on its line.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitBox {
    pub text: String,
    /// Left edge relative to the block's left edge.
    pub x: f32,
    pub width: f32,
}

/// Timing/feel for a behavior. `u` is the canvas unit (1.0 at 1080 px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KineticParams {
    pub preset: PresetParams,
    pub stagger: StaggerSpec,
    pub u: f32,
}

// ---------------------------------------------------------------------------
// Timing constants (seconds unless noted). Durations derive from the preset;
// these are the proportions of it.
// ---------------------------------------------------------------------------

/// Per-unit duration variation (+/- fraction), keyed off stagger rank.
const DURATION_JITTER: f64 = 0.06;
/// Fade-in length as a fraction of the entrance duration.
const FADE_IN_FRACTION: f64 = 0.6;
/// Entrance rise as a fraction of the preset travel.
const RISE_FRACTION: f32 = 0.45;
/// Longest total stagger spread for word/line cascades; longer runs are
/// compressed uniformly (keeps the irregular rhythm, bounds the total time).
const CASCADE_SPREAD_CAP: f64 = 2.0;
/// Longest spread for glyph-cluster stagger.
const GLYPH_SPREAD_CAP: f64 = 0.5;
/// Dimmed opacity of the non-keyword units in `type_scale_emphasis`.
const DIM_OPACITY: f32 = 0.45;
/// Keyword scale in `type_scale_emphasis`.
const EMPHASIS_SCALE: f32 = 1.16;
/// Anticipation dip before the emphasis growth.
const EMPHASIS_DIP: f32 = 0.96;
const EMPHASIS_DIP_SECONDS: f64 = 0.10;
/// Delay per unit of distance from the keyword for the dim ripple.
const DIM_RIPPLE_SECONDS: f64 = 0.03;
/// Starting scale of the punch copy.
const PUNCH_SCALE: f32 = 1.35;
const PUNCH_FADE_SECONDS: f64 = 0.07;
/// How long the ink keyword lingers under the punch copy before fading.
const PUNCH_UNDER_DELAY: f64 = 0.04;
const PUNCH_UNDER_FADE: f64 = 0.14;
/// Tracking: fraction of the distance from the line center the clusters start at.
const TRACKING_SPREAD: f32 = 0.6;

/// Deterministic jitter in `[-1, 1]` from an integer key (splitmix-style hash).
fn jitter(key: usize, salt: u64) -> f64 {
    let mut z = (key as u64)
        .wrapping_add(salt.wrapping_mul(0xD1B5_4A32_D192_ED03))
        .wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    ((z >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}

/// Duration factor for the unit with stagger rank `rank`.
fn variation(rank: usize, salt: u64) -> f64 {
    1.0 + DURATION_JITTER * jitter(rank, salt)
}

/// Offsets for `n` targets, uniformly compressed if they spread beyond `cap`.
fn spread(spec: StaggerSpec, n: usize, cap: f64) -> Vec<f64> {
    let mut offs = stagger::offsets(spec, n);
    let max = offs.iter().copied().fold(0.0, f64::max);
    if max > cap {
        let k = cap / max;
        for o in &mut offs {
            *o *= k;
        }
    }
    offs
}

/// Reveals clamp their progress, so springs would only distort them; use a
/// non-overshooting curve for clip/mask motion.
fn reveal_easing(e: Easing) -> Easing {
    match e {
        Easing::EditorialSpring | Easing::ImpactSpring => Easing::OutQuint,
        other => other,
    }
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

fn fade(target: &str, start: f64, duration: f64, easing: Easing, from: f32, to: f32) -> Motion {
    motion(target, start, duration, easing, MotionOp::Fade { from, to })
}

fn shift(
    target: &str,
    start: f64,
    duration: f64,
    easing: Easing,
    from: [f32; 2],
    to: [f32; 2],
) -> Motion {
    motion(target, start, duration, easing, MotionOp::Move { from, to })
}

fn scale(target: &str, start: f64, duration: f64, easing: Easing, from: f32, to: f32) -> Motion {
    motion(
        target,
        start,
        duration,
        easing,
        MotionOp::Scale {
            from,
            to,
            axis: Axis::Both,
        },
    )
}

fn unit_id(run: &TextRun, line: usize, unit: usize) -> String {
    format!("{}.{}.{}", run.id, line, unit)
}

/// One text layer for `unit`, left-aligned at its measured position, with the
/// anchor at the unit's visual center so scales pivot there.
fn unit_layer(
    run: &TextRun,
    id: String,
    line: usize,
    unit: &UnitBox,
    color: Option<Color>,
) -> Layer {
    let mut style = run.style.clone();
    style.text = unit.text.clone();
    style.align = TextAlign::Left;
    style.max_width = None;
    style.ink = run.ink;
    if let Some(c) = color {
        style.color = c;
    }
    let width = unit.width * 1.02 + 2.0;
    let mut height = style.font_size * style.line_height;
    if height <= 0.0 {
        height = run.line_advance;
    }
    let anchor_y = match (run.ink, height > 0.0) {
        (Some(ink), true) => (ink.top + ink.bottom) * 0.5 / height,
        _ => 0.5,
    };
    Layer {
        tilt: None,
        z: None,
        id,
        x: run.origin[0] + unit.x + width * 0.5,
        y: run.origin[1] + line as f32 * run.line_advance + anchor_y * height,
        width,
        height,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.5,
        anchor_y,
        opacity: 1.0,
        z_index: 0,
        visible: true,
        clip: None,
        depth: None,
        layout: None,
        kind: LayerKind::Text(style),
    }
}

/// A unit's place in the run and when its entrance finished.
#[derive(Debug, Clone)]
struct Placed {
    line: usize,
    idx: usize,
    id: String,
    /// When the entrance fade ends (the unit is fully opaque from here).
    fade_end: f64,
}

/// Reading-order list of `(line, unit index)`.
fn reading_order(run: &TextRun) -> Vec<(usize, usize)> {
    run.lines
        .iter()
        .enumerate()
        .flat_map(|(l, units)| (0..units.len()).map(move |u| (l, u)))
        .collect()
}

/// Create all unit layers and their cascade entrance (fade + rise), staggered
/// over units in reading order. Unit `k` starts at `start + offset(k)`.
fn cascade(
    run: &TextRun,
    start: f64,
    p: &KineticParams,
    salt: u64,
    out: &mut Expansion,
) -> Vec<Placed> {
    let order = reading_order(run);
    let n = order.len();
    let offs = spread(p.stagger, n, CASCADE_SPREAD_CAP);
    let ranks = stagger::ranks(p.stagger.order, n);
    let rise = p.preset.travel * p.u * RISE_FRACTION;
    let mut placed = Vec::with_capacity(n);
    for (k, &(line, idx)) in order.iter().enumerate() {
        let unit = &run.lines[line][idx];
        let id = unit_id(run, line, idx);
        out.layers
            .push(unit_layer(run, id.clone(), line, unit, None));
        let t0 = start + offs[k];
        let dur = p.preset.duration * variation(ranks[k], salt);
        let fade_dur = dur * FADE_IN_FRACTION;
        out.motions
            .push(fade(&id, t0, fade_dur, Easing::OutCubic, 0.0, 1.0));
        out.motions.push(shift(
            &id,
            t0,
            dur,
            p.preset.easing,
            [0.0, rise],
            [0.0, 0.0],
        ));
        placed.push(Placed {
            line,
            idx,
            id,
            fade_end: t0 + fade_dur,
        });
    }
    placed
}

fn find_keyword(placed: &[Placed], keyword: (usize, usize)) -> Option<usize> {
    placed.iter().position(|u| (u.line, u.idx) == keyword)
}

/// Words rise + fade in one after another (stagger over words, reading order).
pub fn word_cascade(run: &TextRun, start: f64, p: &KineticParams) -> Expansion {
    let mut out = Expansion::default();
    cascade(run, start, p, 0, &mut out);
    out
}

/// Each line slides up from behind its own line mask (stagger over lines).
pub fn line_reveal(run: &TextRun, start: f64, p: &KineticParams) -> Expansion {
    line_wise(run, start, p, |_| MotionOp::ClipReveal {
        direction: Direction::Up,
        mode: RevealMode::Reveal,
    })
}

/// Each line is uncovered by a travelling mask edge (content still).
pub fn text_mask_reveal(
    run: &TextRun,
    start: f64,
    direction: Direction,
    p: &KineticParams,
) -> Expansion {
    line_wise(run, start, p, move |_| MotionOp::MaskReveal {
        direction,
        mode: RevealMode::Reveal,
    })
}

/// Shared body of the line-based reveals: one motion per unit, all units of a
/// line sharing that line's start and duration.
fn line_wise(
    run: &TextRun,
    start: f64,
    p: &KineticParams,
    op: impl Fn(usize) -> MotionOp,
) -> Expansion {
    let mut out = Expansion::default();
    let n = run.lines.len();
    let offs = spread(p.stagger, n, CASCADE_SPREAD_CAP);
    let ranks = stagger::ranks(p.stagger.order, n);
    let easing = reveal_easing(p.preset.easing);
    for (line, units) in run.lines.iter().enumerate() {
        let t0 = start + offs[line];
        let dur = p.preset.duration * variation(ranks[line], 2);
        for (idx, unit) in units.iter().enumerate() {
            let id = unit_id(run, line, idx);
            out.layers
                .push(unit_layer(run, id.clone(), line, unit, None));
            out.motions.push(motion(&id, t0, dur, easing, op(line)));
        }
    }
    out
}

/// `old` is on screen until `at`; its units leave upward (staggered) while
/// `new` units arrive from below (staggered, slightly later). Both runs must
/// have distinct ids.
pub fn type_replace(old: &TextRun, new: &TextRun, at: f64, p: &KineticParams) -> Expansion {
    let mut out = Expansion::default();
    let order = reading_order(old);
    let n = order.len();
    let offs = spread(p.stagger, n, CASCADE_SPREAD_CAP);
    let ranks = stagger::ranks(p.stagger.order, n);
    let dip = p.preset.travel * p.u * 0.06;
    let lift = p.preset.travel * p.u * 0.5;
    for (k, &(line, idx)) in order.iter().enumerate() {
        let unit = &old.lines[line][idx];
        let id = unit_id(old, line, idx);
        out.layers
            .push(unit_layer(old, id.clone(), line, unit, None));
        let v = variation(ranks[k], 1);
        let t0 = at + offs[k];
        // Anticipation: a small dip, then the lift-out.
        let dip_dur = 0.08 * v;
        let t1 = t0 + dip_dur;
        let dur = p.preset.duration * v;
        out.motions.push(shift(
            &id,
            t0,
            dip_dur,
            Easing::OutCubic,
            [0.0, 0.0],
            [0.0, dip],
        ));
        out.motions.push(shift(
            &id,
            t1,
            dur * 0.7,
            Easing::InOutCubic,
            [0.0, dip],
            [0.0, -lift],
        ));
        out.motions.push(fade(
            &id,
            t1 + dur * 0.05,
            dur * 0.55,
            Easing::InOutCubic,
            1.0,
            0.0,
        ));
    }
    let arrive = at + p.preset.duration * 0.35;
    let entered = cascade(new, arrive, p, 3, &mut out);
    debug_assert_eq!(entered.len(), reading_order(new).len());
    out
}

/// Units enter with a word cascade; at `emphasis_at` the keyword unit grows
/// and settles larger while the other units recede in opacity.
pub fn type_scale_emphasis(
    run: &TextRun,
    keyword: (usize, usize),
    start: f64,
    emphasis_at: f64,
    p: &KineticParams,
) -> Expansion {
    let mut out = Expansion::default();
    let placed = cascade(run, start, p, 0, &mut out);
    let Some(kw) = find_keyword(&placed, keyword) else {
        return out;
    };
    // The keyword grows only once it has fully arrived.
    let grow_at = emphasis_at.max(placed[kw].fade_end);
    let kw_id = placed[kw].id.clone();
    out.motions.push(scale(
        &kw_id,
        grow_at,
        EMPHASIS_DIP_SECONDS,
        Easing::InOutCubic,
        1.0,
        EMPHASIS_DIP,
    ));
    out.motions.push(scale(
        &kw_id,
        grow_at + EMPHASIS_DIP_SECONDS,
        p.preset.duration * 0.8,
        p.preset.settle,
        EMPHASIS_DIP,
        EMPHASIS_SCALE,
    ));
    for (k, unit) in placed.iter().enumerate() {
        if k == kw {
            continue;
        }
        let ripple = k.abs_diff(kw).saturating_sub(1) as f64 * DIM_RIPPLE_SECONDS;
        // Never overlap this unit's own entrance fade.
        let t0 = (emphasis_at + ripple).max(unit.fade_end);
        out.motions.push(fade(
            &unit.id,
            t0,
            p.preset.duration * 0.55,
            Easing::InOutCubic,
            1.0,
            DIM_OPACITY,
        ));
    }
    out
}

/// Units enter with a word cascade; at `punch_at` the keyword is struck: an
/// `accent`-colored copy lands over it with an overshooting scale.
pub fn keyword_punch(
    run: &TextRun,
    keyword: (usize, usize),
    start: f64,
    punch_at: f64,
    accent: Color,
    p: &KineticParams,
) -> Expansion {
    let mut out = Expansion::default();
    let placed = cascade(run, start, p, 0, &mut out);
    let Some(kw) = find_keyword(&placed, keyword) else {
        return out;
    };
    let unit = &run.lines[keyword.0][keyword.1];
    // The strike lands on a fully arrived keyword.
    let hit = punch_at.max(placed[kw].fade_end);
    let punch_id = format!("{}.punch", run.id);
    out.layers.push(unit_layer(
        run,
        punch_id.clone(),
        keyword.0,
        unit,
        Some(accent),
    ));
    out.motions.push(fade(
        &punch_id,
        hit,
        PUNCH_FADE_SECONDS,
        Easing::OutCubic,
        0.0,
        1.0,
    ));
    out.motions.push(scale(
        &punch_id,
        hit,
        p.preset.duration * 0.75,
        Easing::ImpactSpring,
        PUNCH_SCALE,
        1.0,
    ));
    // The ink keyword steps out from under the accent copy.
    out.motions.push(fade(
        &placed[kw].id,
        hit + PUNCH_UNDER_DELAY,
        PUNCH_UNDER_FADE,
        Easing::OutCubic,
        1.0,
        0.0,
    ));
    out
}

/// Glyph-cluster run (one line): clusters start spread apart around the line
/// center and converge to their measured positions while fading in.
pub fn tracking_reveal(run: &TextRun, start: f64, p: &KineticParams) -> Expansion {
    let mut out = Expansion::default();
    let order = StaggerSpec {
        preset: p.stagger.preset,
        order: StaggerOrder::CenterOut,
    };
    for (line, units) in run.lines.iter().enumerate() {
        let n = units.len();
        let left = units.iter().map(|u| u.x).fold(f32::INFINITY, f32::min);
        let right = units
            .iter()
            .map(|u| u.x + u.width)
            .fold(f32::NEG_INFINITY, f32::max);
        let center = if n > 0 { (left + right) * 0.5 } else { 0.0 };
        let offs = spread(order, n, GLYPH_SPREAD_CAP);
        let ranks = stagger::ranks(StaggerOrder::CenterOut, n);
        for (idx, unit) in units.iter().enumerate() {
            let id = unit_id(run, line, idx);
            out.layers
                .push(unit_layer(run, id.clone(), line, unit, None));
            let t0 = start + offs[idx];
            let dur = p.preset.duration * variation(ranks[idx], 4);
            let dx = (unit.x + unit.width * 0.5 - center) * TRACKING_SPREAD;
            out.motions
                .push(fade(&id, t0, dur * 0.55, Easing::OutCubic, 0.0, 1.0));
            out.motions
                .push(shift(&id, t0, dur, p.preset.easing, [dx, 0.0], [0.0, 0.0]));
        }
    }
    out
}
