//! Data primitives: small, typographically strong infographic behaviors that
//! expand into rectangles, text and polylines. Values are semantic (numbers,
//! fractions); geometry is derived here. Times are scene-local seconds.
//!
//! Ids: `"{id}.{part}"` (e.g. `"{id}.track"`, `"{id}.fill"`, `"{id}.bar.{i}"`).

use super::stagger::{offsets, StaggerSpec};
use super::Expansion;
use crate::easing::{Easing, PresetParams};
use crate::scene::{
    BoxRect, Color, Direction, Layer, LayerKind, Motion, MotionOp, RevealMode, Stroke, TextStyle,
};
use crate::timeline::format_count;

/// Visual tokens (from the compiler's palette / text voices).
#[derive(Debug, Clone, PartialEq)]
pub struct DataStyle {
    pub ink: Color,
    pub accent: Color,
    pub muted: Color,
    /// Empty track behind bars.
    pub track: Color,
    /// Canvas unit (1.0 at 1080 px).
    pub u: f32,
}

/// A measured number slot for counters: the text template (font, size,
/// color, align) and the box that fits the widest formatted value.
#[derive(Debug, Clone, PartialEq)]
pub struct NumberSlot {
    pub style: TextStyle,
    pub rect: BoxRect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountFormat {
    pub decimals: u8,
    pub grouping: bool,
    pub prefix: String,
    pub suffix: String,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn layer(id: String, rect: BoxRect, kind: LayerKind) -> Layer {
    Layer {
        tilt: None,
        z: None,
        id,
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        scale_x: 1.0,
        scale_y: 1.0,
        rotation_degrees: 0.0,
        anchor_x: 0.0,
        anchor_y: 0.0,
        opacity: 1.0,
        z_index: 0,
        visible: true,
        clip: None,
        depth: None,
        layout: None,
        kind,
    }
}

fn slab(id: String, rect: BoxRect, fill: Color) -> Layer {
    layer(id, rect, LayerKind::Rectangle { fill, stroke: None })
}

fn mo(target: &str, start: f64, duration: f64, easing: Easing, op: MotionOp) -> Motion {
    Motion {
        spring: None,
        id: None,
        target: target.to_string(),
        start: start.max(0.0),
        duration: duration.max(0.0),
        easing,
        op,
    }
}

/// Curves that land on the target without passing it (geometry that must
/// stay inside a track never overshoots).
fn no_overshoot(e: Easing) -> Easing {
    match e {
        Easing::EditorialSpring | Easing::ImpactSpring => Easing::OutQuint,
        other => other,
    }
}

/// Number counting settles like a decelerating dial and never overshoots.
fn count_easing(p: &PresetParams) -> Easing {
    match p.settle {
        Easing::InOutCubic | Easing::Linear | Easing::InCubic => Easing::OutCubic,
        _ => Easing::OutQuint,
    }
}

fn finite_or(v: f64, fallback: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        fallback
    }
}

/// Non-negative, finite magnitude for bar values.
fn magnitude(v: f64) -> f64 {
    finite_or(v, 0.0).max(0.0)
}

/// `value / max` in `0..=1`; 0 when `max` is not positive.
fn ratio(value: f64, max: f64) -> f32 {
    if max > 0.0 {
        (value / max).clamp(0.0, 1.0) as f32
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Counter
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Text layer counting `from → to` (Count op) with a small rise-in.
///
/// Emits one layer with id `id` (no part suffix). The layer's initial text is
/// the formatted `from` value; a short rise + fade brings it in while the
/// number decelerates onto `to` (never overshoots).
pub fn counter(
    id: &str,
    slot: &NumberSlot,
    from: f64,
    to: f64,
    format: &CountFormat,
    start: f64,
    duration: f64,
    p: &PresetParams,
) -> Expansion {
    let from = finite_or(from, 0.0);
    let to = finite_or(to, 0.0);
    let mut style = slot.style.clone();
    style.text = format_count(
        from,
        format.decimals,
        format.grouping,
        &format.prefix,
        &format.suffix,
    );
    let duration = duration.max(0.0);
    let rise = (duration * 0.3).clamp(0.2, 0.45);
    let travel = p.travel * 0.3 * slot_scale(slot);

    let mut e = Expansion::default();
    e.layers
        .push(layer(id.to_string(), slot.rect, LayerKind::Text(style)));
    e.motions.push(mo(
        id,
        start,
        duration,
        count_easing(p).landing(),
        MotionOp::Count {
            from,
            to,
            decimals: format.decimals,
            grouping: format.grouping,
            prefix: format.prefix.clone(),
            suffix: format.suffix.clone(),
        },
    ));
    e.motions.push(mo(
        id,
        start,
        rise,
        Easing::OutCubic,
        MotionOp::Fade { from: 0.0, to: 1.0 },
    ));
    e.motions.push(mo(
        id,
        start,
        rise * 1.6,
        no_overshoot(p.easing),
        MotionOp::Move {
            from: [0.0, travel],
            to: [0.0, 0.0],
        },
    ));
    e
}

/// Rise distance scales with the type size (a 400 px hero rises further than
/// a 40 px label), relative to a 1080 px canvas unit of 120 px type.
fn slot_scale(slot: &NumberSlot) -> f32 {
    (slot.style.font_size / 120.0).clamp(0.25, 2.0)
}

// ---------------------------------------------------------------------------
// Progress bar
// ---------------------------------------------------------------------------

/// Track + fill; the fill grows from zero width to `fraction` of the track.
///
/// Ids: `{id}.track`, `{id}.fill`.
pub fn progress_bar(
    id: &str,
    rect: BoxRect,
    fraction: f32,
    style: &DataStyle,
    start: f64,
    p: &PresetParams,
) -> Expansion {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fill_id = format!("{id}.fill");
    let mut e = Expansion::default();
    e.layers
        .push(slab(format!("{id}.track"), rect, style.track));
    e.layers.push(slab(
        fill_id.clone(),
        BoxRect { width: 0.0, ..rect },
        style.accent,
    ));
    e.motions.push(mo(
        &fill_id,
        start,
        p.duration * 1.6,
        no_overshoot(p.easing),
        MotionOp::AccentExpand {
            to: BoxRect {
                width: rect.width * fraction,
                ..rect
            },
        },
    ));
    e
}

// ---------------------------------------------------------------------------
// Bar chart
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
/// Vertical bars on a baseline inside `rect`, heights proportional to
/// `values / max(values)`, growing up with a stagger. `highlight` bar uses
/// the accent color, others ink/muted.
///
/// Ids: `{id}.base` (baseline rule), `{id}.bar.{i}`. Taller bars take a
/// little longer to grow, so the cascade never lands in lockstep.
pub fn bar_chart(
    id: &str,
    rect: BoxRect,
    values: &[f64],
    highlight: Option<usize>,
    style: &DataStyle,
    start: f64,
    p: &PresetParams,
    stagger: StaggerSpec,
) -> Expansion {
    let n = values.len();
    let baseline = rect.y + rect.height;
    let rule_h = (3.0 * style.u).max(1.0);
    let rule_id = format!("{id}.base");

    let mut e = Expansion::default();
    e.layers.push(slab(
        rule_id.clone(),
        BoxRect {
            x: rect.x,
            y: baseline,
            width: rect.width,
            height: rule_h,
        },
        style.muted,
    ));
    e.motions.push(mo(
        &rule_id,
        start,
        p.duration * 0.7,
        Easing::OutCubic,
        MotionOp::MaskReveal {
            direction: Direction::Right,
            mode: RevealMode::Reveal,
        },
    ));
    if n == 0 {
        return e;
    }

    let mags: Vec<f64> = values.iter().map(|v| magnitude(*v)).collect();
    let max = mags.iter().copied().fold(0.0_f64, f64::max);
    let slot = rect.width / n as f32;
    let bar_w = slot * 0.62;
    let offs = offsets(stagger, n);
    let easing = match p.easing {
        Easing::ImpactSpring => Easing::EditorialSpring,
        other => other,
    };

    for (i, m) in mags.iter().enumerate() {
        let bar_id = format!("{id}.bar.{i}");
        let h = rect.height * ratio(*m, max);
        let x = rect.x + slot * i as f32 + (slot - bar_w) / 2.0;
        let color = if highlight == Some(i) {
            style.accent
        } else {
            style.ink
        };
        e.layers.push(slab(
            bar_id.clone(),
            BoxRect {
                x,
                y: baseline,
                width: bar_w,
                height: 0.0,
            },
            color,
        ));
        let dur = p.duration * 1.3 * (0.8 + 0.4 * ratio(*m, max) as f64);
        let offset = offs.get(i).copied().unwrap_or(0.0);
        e.motions.push(mo(
            &bar_id,
            start + p.duration * 0.25 + offset,
            dur,
            easing,
            MotionOp::AccentExpand {
                to: BoxRect {
                    x,
                    y: baseline - h,
                    width: bar_w,
                    height: h,
                },
            },
        ));
    }
    e
}

// ---------------------------------------------------------------------------
// Horizontal (ranked) bars
// ---------------------------------------------------------------------------

/// (0.22) Each value's share of the largest, in `0..=1` (non-finite and
/// negative values count as 0; all zero when no value is positive). A bar's
/// length is its row's full length times this.
pub fn bar_fractions(values: &[f64]) -> Vec<f32> {
    let mags: Vec<f64> = values.iter().map(|v| magnitude(*v)).collect();
    let max = mags.iter().copied().fold(0.0_f64, f64::max);
    mags.iter().map(|m| ratio(*m, max)).collect()
}

/// (0.22) Horizontal bars, one per row, each growing rightwards from its
/// row's left edge: `rows[i]` is bar `i`'s slot (left edge, top, height, and
/// as width the length a bar of the largest value reaches), so bar `i` ends
/// `rows[i].width × bar_fractions(values)[i]` to the right. Bar `highlight`
/// uses the accent, the others ink. Bar `i` starts growing at `starts[i]`
/// (the last start repeats when `starts` is shorter); a longer bar takes a
/// little longer, and a bar never overshoots its length.
///
/// Ids: `{id}.{i}.bar` (so a caller's row group `{id}.{i}` holds its bar).
pub fn hbar_chart(
    id: &str,
    rows: &[BoxRect],
    values: &[f64],
    highlight: Option<usize>,
    style: &DataStyle,
    starts: &[f64],
    p: &PresetParams,
) -> Expansion {
    let mut e = Expansion::default();
    let fractions = bar_fractions(values);
    let easing = no_overshoot(p.easing);
    for (i, (row, f)) in rows.iter().zip(&fractions).enumerate() {
        let bar_id = format!("{id}.{i}.bar");
        let color = if highlight == Some(i) {
            style.accent
        } else {
            style.ink
        };
        e.layers
            .push(slab(bar_id.clone(), BoxRect { width: 0.0, ..*row }, color));
        let start = starts
            .get(i)
            .or(starts.last())
            .copied()
            .map_or(0.0, |t| finite_or(t, 0.0));
        e.motions.push(mo(
            &bar_id,
            start,
            p.duration * 1.3 * (0.8 + 0.4 * f64::from(*f)),
            easing,
            MotionOp::AccentExpand {
                to: BoxRect {
                    width: row.width * f,
                    ..*row
                },
            },
        ));
    }
    e
}

// ---------------------------------------------------------------------------
// Comparison bar
// ---------------------------------------------------------------------------

/// Two horizontal bars (a above b) scaled to the larger value; `b` grows
/// after `a` so the comparison reads as a sequence. Accent marks the larger.
///
/// Ids: `{id}.a.track`, `{id}.a.fill`, `{id}.b.track`, `{id}.b.fill`. Each row
/// takes 40% of `rect.height` with a 20% gap. A tie accents neither.
pub fn comparison_bar(
    id: &str,
    rect: BoxRect,
    a: f64,
    b: f64,
    style: &DataStyle,
    start: f64,
    p: &PresetParams,
) -> Expansion {
    let (a, b) = (magnitude(a), magnitude(b));
    let max = a.max(b);
    let row_h = rect.height * 0.4;
    let gap = rect.height - 2.0 * row_h;
    let dur_a = p.duration * 1.5;
    let easing = no_overshoot(p.easing);

    let mut e = Expansion::default();
    let rows = [
        ("a", a, rect.y, start, dur_a, a > b),
        (
            "b",
            b,
            rect.y + row_h + gap,
            start + 0.35 * dur_a,
            dur_a * 1.15,
            b > a,
        ),
    ];
    for (name, value, y, t0, dur, larger) in rows {
        let row = BoxRect {
            x: rect.x,
            y,
            width: rect.width,
            height: row_h,
        };
        let fill_id = format!("{id}.{name}.fill");
        e.layers
            .push(slab(format!("{id}.{name}.track"), row, style.track));
        e.layers.push(slab(
            fill_id.clone(),
            BoxRect { width: 0.0, ..row },
            if larger { style.accent } else { style.ink },
        ));
        e.motions.push(mo(
            &fill_id,
            t0,
            dur,
            easing,
            MotionOp::AccentExpand {
                to: BoxRect {
                    width: rect.width * ratio(value, max),
                    ..row
                },
            },
        ));
    }
    e
}

// ---------------------------------------------------------------------------
// Path draw
// ---------------------------------------------------------------------------

/// Line chart path through `values` (evenly spaced, scaled into `rect`),
/// drawn on with a Trim motion.
///
/// Ids: `{id}.line` (polyline, box = `rect`, points in box space) and
/// `{id}.dot` (end marker, anchored at its center, fades and scales in as the
/// line lands). A flat series is drawn along the vertical middle.
pub fn path_draw(
    id: &str,
    rect: BoxRect,
    values: &[f64],
    style: &DataStyle,
    start: f64,
    duration: f64,
    p: &PresetParams,
) -> Expansion {
    let mut e = Expansion::default();
    if values.is_empty() {
        return e;
    }
    let vals: Vec<f64> = values.iter().map(|v| finite_or(*v, 0.0)).collect();
    let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
    let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = max - min;
    let n = vals.len();
    let points: Vec<[f32; 2]> = vals
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = if n > 1 {
                rect.width * i as f32 / (n - 1) as f32
            } else {
                rect.width / 2.0
            };
            let t = if span > 0.0 {
                ((v - min) / span) as f32
            } else {
                0.5
            };
            [x, rect.height * (1.0 - t)]
        })
        .collect();
    let end = points[n - 1];

    let line_id = format!("{id}.line");
    let dot_id = format!("{id}.dot");
    e.layers.push(layer(
        line_id.clone(),
        rect,
        LayerKind::Polyline {
            points,
            stroke: Stroke {
                color: style.accent,
                width: 8.0 * style.u,
            },
            closed: false,
            fill: None,
        },
    ));
    let d = 24.0 * style.u;
    let mut dot = layer(
        dot_id.clone(),
        BoxRect {
            x: rect.x + end[0],
            y: rect.y + end[1],
            width: d,
            height: d,
        },
        LayerKind::RoundedRectangle {
            fill: style.accent,
            radius: d / 2.0,
            stroke: None,
        },
    );
    dot.anchor_x = 0.5;
    dot.anchor_y = 0.5;
    dot.opacity = 1.0;

    e.layers.push(dot);
    e.motions.push(mo(
        &line_id,
        start,
        duration,
        no_overshoot(p.easing),
        MotionOp::Trim { from: 0.0, to: 1.0 },
    ));
    let pop_start = start + duration * 0.92;
    e.motions.push(mo(
        &dot_id,
        pop_start,
        0.22,
        Easing::OutCubic,
        MotionOp::Fade { from: 0.0, to: 1.0 },
    ));
    e.motions.push(mo(
        &dot_id,
        pop_start,
        0.32,
        Easing::EditorialSpring,
        MotionOp::Scale {
            from: 0.3,
            to: 1.0,
            axis: crate::scene::Axis::Both,
        },
    ));
    e
}
