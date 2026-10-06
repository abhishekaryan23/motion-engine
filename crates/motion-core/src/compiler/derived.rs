//! Derived-metric composition and arithmetic (CreativeIntent v0.2
//! `derived_metric` subject). MotionEngine computes the value; the model only
//! supplies numerator, denominator and meaning.
//!
//! # Arithmetic
//! [`compute`] is `numerator.value / denominator.value` in `f64` (so
//! `80 / 1000 == 0.08`). Callers pass validated metrics (finite, denominator
//! != 0); [`compose`] rejects the rest with a `CompileError`.
//!
//! # Formatting and rounding
//! [`format_metric`] scales the value by the format, rounds, then trims:
//!
//! | format         | scale  | suffix         | base decimals |
//! |----------------|--------|----------------|---------------|
//! | `percent`      | x 100  | `%`            | 2             |
//! | `per_thousand` | x 1000 | ` per 1,000`   | 1             |
//! | `decimal`      | x 1    | (none)         | 3             |
//!
//! 1. Round the *magnitude* half away from zero (`f64::round` on
//!    `|scaled| * 10^d`) to `d` decimals; the sign is applied afterwards, so
//!    `-0.125` and `0.125` round symmetrically.
//! 2. If that rounds to 0 while the value is not 0, increase `d` (up to 6)
//!    until two significant digits show (`|scaled| * 10^d` rounds to >= 10);
//!    trailing zeros are trimmed afterwards, so 1/200000 as a percent is
//!    `0.0005%`, not `0%`. Below 1e-6 of the scale it is honestly `0`.
//! 3. Trim trailing zeros (and a trailing `.`): `13.30 -> 13.3`, `250.00 -> 250`.
//! 4. Thousands grouping with `,` when the displayed `|shown| >= 1000`.
//! 5. Negative values get a leading `-`; zero is never `-0`.
//!
//! `shown` is the displayed number, `decimals` the digits after trimming and
//! `prefix` is empty, so `Count { from: 0, to: shown, decimals, grouping,
//! prefix, suffix }` lands exactly on `text` (`text` is produced by
//! `timeline::format_count`, the same routine the timeline uses).
//!
//! # Composition
//! * One metric: a fraction block (numerator value + meaning, division rule,
//!   denominator value + meaning), then `=` and the result as a hero counter;
//!   a percent in (0, 100] also fills a progress bar (the part of the whole).
//!   The result counter and bar start last and land together, then hold.
//! * Two metrics (primary = first, secondary = second): two rows of identical
//!   structure (small `num meaning ÷ den meaning` line + result counter, both
//!   formatted with the primary's format so they are comparable), a
//!   `comparison_bar` of the two computed values (each bar tagged with its
//!   result) and a conclusion computed only from the numbers, e.g.
//!   `fewer sales · higher conversion`. The words are `higher | lower | same`
//!   (decided on the displayed values), prefixed by `fewer | more <numerator
//!   meaning>` when the numerator moved against the rate. Never a value
//!   judgement.
//! * Any other atomic subject of the beat becomes a small caption.
//!
//! # Timing (scene lifecycle, docs/SCENE_LIFECYCLE.md)
//! The first value arrives in ENTER; the rest unfolds one step per evolve event
//! (`life.evolve_events(3)`, see [`evolve_steps`]):
//! * One metric: numerator + meaning (ENTER); division rule + denominator at
//!   event 0; `=` and the counted result at event 1; result label, progress bar
//!   and caption at event 2.
//! * Two metrics: row 1 (ENTER); row 2 at event 0; comparison bars at event 1;
//!   the computed conclusion at event 2.
//!
//! Every motion starts at or after `region.start`, stays inside the region and
//! ends by `plan.exit_at()`; the pace scales down (never the content) when the
//! beat is short.

use std::cmp::Ordering;

use super::recipes::{absorb, Region, B};
use super::typeset::{round2, Block, Voice};
use super::{mo, rect_layer, round3, CompileError, Ctx};
use crate::easing::{Easing, PresetParams};
use crate::intent::{DerivedMetric, MetricFormat, Operation, Subject};
use crate::motion::dataviz::{self, CountFormat, DataStyle, NumberSlot};
use crate::motion::Expansion;
use crate::scene::{BoxRect, Color, Direction, TextAlign, TextStyle};
use crate::timeline::format_count;

// ---------------------------------------------------------------------------
// Arithmetic + formatting
// ---------------------------------------------------------------------------

/// The computed value (`numerator / denominator` for `ratio`). Callers must
/// only pass validated metrics (finite, denominator != 0).
pub fn compute(metric: &DerivedMetric) -> f64 {
    match metric.operation {
        Operation::Ratio => metric.numerator.value / metric.denominator.value,
    }
}

/// A formatted metric: display text plus the pieces a `count` motion needs to
/// land exactly on that text.
#[derive(Debug, Clone, PartialEq)]
pub struct FormattedMetric {
    /// e.g. "8%", "1.33%", "0.125", "13.3 per 1,000".
    pub text: String,
    /// The displayed number (already scaled and rounded), e.g. 8.0, 1.33, 13.3.
    pub shown: f64,
    pub decimals: u8,
    pub prefix: String,
    pub suffix: String,
}

impl FormattedMetric {
    /// Whether `text` groups thousands (true when `|shown| >= 1000`); pass to
    /// a `count` motion together with `decimals`, `prefix` and `suffix`.
    pub fn grouping(&self) -> bool {
        self.shown.abs() >= 1000.0
    }
}

/// Largest number of decimals a derived value is ever shown with.
const MAX_DECIMALS: u8 = 6;

/// Round `scaled` half away from zero to `base` decimals (more when that
/// would show 0 for a non-zero value), trim trailing zeros. Returns the
/// displayed number and its decimals.
fn round_trim(scaled: f64, base: u8) -> (f64, u8) {
    if !scaled.is_finite() {
        return (0.0, 0);
    }
    let mag = scaled.abs();
    let mut d = base.min(MAX_DECIMALS);
    let mut r = (mag * 10f64.powi(i32::from(d))).round();
    if r == 0.0 && mag != 0.0 {
        while d < MAX_DECIMALS {
            d += 1;
            r = (mag * 10f64.powi(i32::from(d))).round();
            if r >= 10.0 {
                break;
            }
        }
    }
    while d > 0 && r % 10.0 == 0.0 {
        r /= 10.0;
        d -= 1;
    }
    let shown = r / 10f64.powi(i32::from(d));
    if r == 0.0 || !shown.is_finite() {
        return (0.0, 0);
    }
    (if scaled < 0.0 { -shown } else { shown }, d)
}

/// Format a computed value (rules in the module docs).
pub fn format_metric(value: f64, format: MetricFormat) -> FormattedMetric {
    let (scale, base, suffix) = match format {
        MetricFormat::Percent => (100.0, 2, "%"),
        MetricFormat::PerThousand => (1000.0, 1, " per 1,000"),
        MetricFormat::Decimal => (1.0, 3, ""),
    };
    let (shown, decimals) = round_trim(value * scale, base);
    let text = format_count(shown, decimals, shown.abs() >= 1000.0, "", suffix);
    FormattedMetric {
        text,
        shown,
        decimals,
        prefix: String::new(),
        suffix: suffix.to_string(),
    }
}

/// A raw term (numerator / denominator) as `(shown, decimals, text)`: at most
/// three decimals, trimmed, grouped from 1,000.
fn term(value: f64) -> (f64, u8, String) {
    let (shown, decimals) = round_trim(value, 3);
    let text = format_count(shown, decimals, shown.abs() >= 1000.0, "", "");
    (shown, decimals, text)
}

fn ordering(a: f64, b: f64) -> Ordering {
    a.partial_cmp(&b).unwrap_or(Ordering::Equal)
}

/// The computed conclusion line for two metrics, from the numbers only.
fn conclusion(
    a: &DerivedMetric,
    b: &DerivedMetric,
    fa: &FormattedMetric,
    fb: &FormattedMetric,
) -> String {
    let rate = a
        .meaning
        .as_deref()
        .or(b.meaning.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("rate");
    let rate_dir = ordering(fb.shown, fa.shown);
    let word = match rate_dir {
        Ordering::Greater => "higher",
        Ordering::Less => "lower",
        Ordering::Equal => "same",
    };
    let num_dir = ordering(b.numerator.value, a.numerator.value);
    let prefix = if num_dir != Ordering::Equal && num_dir != rate_dir {
        let more = if num_dir == Ordering::Less {
            "fewer"
        } else {
            "more"
        };
        format!("{more} {} · ", a.numerator.meaning.trim())
    } else {
        String::new()
    };
    format!("{prefix}{word} {rate}")
}

// ---------------------------------------------------------------------------
// Composition
// ---------------------------------------------------------------------------

/// Compose a beat whose structure is a derived metric (single, or primary vs
/// secondary comparison). Same contract as `collection::compose`.
pub(crate) fn compose(ctx: &mut Ctx, b: &mut B, region: Region) -> Result<(), CompileError> {
    let beat = b.beat;
    let mut metrics: Vec<&DerivedMetric> = Vec::new();
    let mut caption: Option<String> = None;
    for s in [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
    {
        match s {
            Subject::DerivedMetric(m) if metrics.len() < 2 => metrics.push(m),
            other if !matches!(other, Subject::DerivedMetric(_)) && caption.is_none() => {
                caption = super::subject_summary(other);
            }
            _ => {}
        }
    }
    for m in &metrics {
        let (n, d) = (m.numerator.value, m.denominator.value);
        if !n.is_finite() || !d.is_finite() || d == 0.0 {
            return Err(CompileError::Beat {
                beat: b.plan.index,
                message: "derived_metric needs finite numbers and a non-zero denominator".into(),
            });
        }
    }
    let first = b.motions.len();
    let env = Env::new(ctx, &region);
    match metrics.as_slice() {
        [m] => single(ctx, b, &env, &region, m, caption.as_deref()),
        [a, c] => pair(ctx, b, &env, &region, a, c),
        _ => {
            return Err(CompileError::Beat {
                beat: b.plan.index,
                message: "derived_metric beat has no derived_metric subject".into(),
            })
        }
    }
    settle_motions(b, first, b.plan.exit_at());
    Ok(())
}

/// The free area of the beat, in canvas pixels.
struct Env {
    left: f32,
    width: f32,
    top: f32,
    height: f32,
}

impl Env {
    fn new(ctx: &Ctx, region: &Region) -> Env {
        Env {
            left: region.margin,
            width: (ctx.w - 2.0 * region.margin).max(1.0),
            top: region.top,
            height: (region.bottom - region.top).max(1.0),
        }
    }
    /// Wide areas (landscape/square) lay columns side by side.
    fn wide(&self) -> bool {
        self.width > 1.25 * self.height
    }
}

/// Pace for the beat: `(start, params)`. Durations shrink (never below 45%)
/// when `units` of nominal pace plus a reading hold do not fit before the exit.
fn pace(b: &B, region: &Region, units: f64) -> (f64, PresetParams) {
    const HOLD: f64 = 0.9;
    let mut p = b.plan.lang.preset;
    let t0 = region.start;
    let avail = b.plan.exit_at() - t0;
    let k = ((avail - HOLD) / (units * p.duration).max(1e-6)).clamp(0.45, 1.0);
    p.duration *= k;
    p.stagger *= k;
    (t0, p)
}

/// The three evolve-event times of a derived beat: `life.evolve_events(3)`,
/// kept strictly ordered and at least 0.35 s after the first value (`t0`) even
/// when the first value lands late.
fn evolve_steps(b: &B, t0: f64) -> [f64; 3] {
    let ev = b.plan.life.evolve_events(3);
    let mut out = [0.0; 3];
    let mut prev = t0;
    for (o, e) in out.iter_mut().zip(ev) {
        *o = round3(e.max(prev + 0.35));
        prev = *o;
    }
    out
}

/// Safety net: round motion times and keep every motion inside `[0, exit]`.
fn settle_motions(b: &mut B, from: usize, exit: f64) {
    let exit = exit.max(0.0);
    for m in b.motions.iter_mut().skip(from) {
        m.start = round3(m.start.max(0.0).min(exit));
        m.duration = round3(m.duration.max(0.0).min((exit - m.start).max(0.0)));
    }
}

fn no_overshoot(e: Easing) -> Easing {
    match e {
        Easing::EditorialSpring | Easing::ImpactSpring => Easing::OutQuint,
        other => other,
    }
}

/// Vertical ink extent `(top, bottom)` of a block (box when unmeasured).
fn ink(ctx: &Ctx, blk: &Block) -> (f32, f32) {
    ctx.ts
        .ink(blk)
        .map(|i| (i.top, i.bottom))
        .unwrap_or((0.0, blk.height()))
}

/// Fade + short rise.
fn rise(b: &mut B, id: &str, start: f64, p: &PresetParams, u: f32) {
    b.motions.push(mo::fade(
        id,
        start,
        p.duration * 0.7,
        0.0,
        1.0,
        Easing::OutCubic,
    ));
    b.motions.push(mo::shift(
        id,
        start,
        p.duration,
        [0.0, p.travel * 0.3 * u],
        [0.0, 0.0],
        no_overshoot(p.easing),
    ));
}

/// Push a plain text layer for `blk` at `(x, y)` and return its id.
fn put_text(b: &mut B, name: &str, blk: &Block, color: Color, x: f32, y: f32, z: i32) -> String {
    let id = b.id(name);
    let mut l = super::typeset::text_layer(id.clone(), blk, color, TextAlign::Left);
    l.x = x;
    l.y = y;
    l.z_index = z;
    b.push(l);
    id
}

/// A counter slot: the block's text style and a box wide enough for `widest`.
fn count_slot(blk: &Block, color: Color, x: f32, y: f32, widest: f32) -> NumberSlot {
    let v = &blk.voice;
    NumberSlot {
        style: TextStyle {
            text: blk.text(),
            font_role: v.role,
            font_size: round2(blk.size),
            font_weight: v.weight,
            italic: v.italic,
            color,
            align: TextAlign::Left,
            line_height: v.line_height,
            letter_spacing: v.letter_spacing,
            max_width: None,
            uppercase: false,
            ink: None,
        },
        rect: BoxRect {
            x,
            y,
            width: widest * 1.04 + 4.0,
            height: blk.height(),
        },
    }
}

/// Widest of the block's own text and its count-start text.
fn widest(ctx: &Ctx, blk: &Block, start_text: &str) -> f32 {
    blk.width()
        .max(ctx.ts.width(&blk.voice, start_text, blk.size))
}

fn data_style(ctx: &Ctx) -> DataStyle {
    DataStyle {
        ink: ctx.palette.ink,
        accent: ctx.palette.accent,
        muted: ctx.palette.muted,
        track: ctx.palette.muted.with_alpha(0x44),
        u: ctx.u,
    }
}

/// Lay out at scale 1; if it does not fit `height`, rebuild scaled to fit.
fn fit_plan<T>(height: f32, plan: impl Fn(f32) -> (T, f32)) -> (T, f32, f32) {
    let (first, total) = plan(1.0);
    if total <= height {
        return (first, total, 1.0);
    }
    let s = ((height / total) * 0.98).clamp(0.35, 1.0);
    let (p, t) = plan(s);
    (p, t, s)
}

/// Scale (>= 1) that lifts a block of font `size` to the legibility floor
/// (`frame.min_type_px`) on non-legacy canvases; exactly 1 on the frozen
/// legacy canvases and for text already at or above the floor.
fn floor_scale(ctx: &Ctx, size: f32) -> f32 {
    let floor = ctx.frame.min_type_px;
    if super::grammar::placement::is_legacy_canvas(&ctx.frame) || size >= floor || size <= 0.0 {
        1.0
    } else {
        floor / size
    }
}

/// Build a text block at `nominal` max size; a block the plan's shrink-to-fit
/// pushed under the legibility floor is rebuilt larger (the other plan
/// elements give the room back).
fn floor_type(ctx: &Ctx, nominal: f32, build: impl Fn(f32) -> Block) -> Block {
    let blk = build(nominal);
    let k = floor_scale(ctx, blk.size);
    if k == 1.0 {
        blk
    } else {
        build(nominal * k)
    }
}

/// Label block next to a value (LABEL voice), if there is a meaning.
fn label_block(ctx: &Ctx, text: &str, max_w: f32, size: f32) -> Option<Block> {
    let t = text.trim();
    (!t.is_empty()).then(|| {
        ctx.ts
            .fit_line(Voice::LABEL, t, max_w.max(40.0 * ctx.u), size)
    })
}

// ---------------------------------------------------------------------------
// Single metric
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Role {
    NumVal,
    NumLabel,
    DenVal,
    DenLabel,
    Eq,
    Result,
    ResultLabel,
    Caption,
}

struct Placed {
    role: Role,
    block: Block,
    x: f32,
    /// Relative to the composition's top.
    y: f32,
    widest: f32,
}

struct SinglePlan {
    items: Vec<Placed>,
    /// Division rule `(x, y, w, h)`, y relative.
    rule: (f32, f32, f32, f32),
    /// Progress bar rect, y relative.
    bar: Option<BoxRect>,
}

struct SingleText<'a> {
    num: &'a str,
    den: &'a str,
    num_meaning: &'a str,
    den_meaning: &'a str,
    result: &'a str,
    result_label: &'a str,
    caption: Option<&'a str>,
    bar: bool,
}

fn plan_single(ctx: &Ctx, env: &Env, t: &SingleText, s: f32) -> (SinglePlan, f32) {
    let u = ctx.u * s;
    let wide = env.wide();
    let fw = if wide { 0.46 * env.width } else { env.width };
    let col_gap = 0.06 * env.width;
    let rw = if wide {
        env.width - fw - col_gap
    } else {
        env.width
    };
    let rx = if wide {
        env.left + fw + col_gap
    } else {
        env.left
    };
    let vs = 170.0 * u;
    let gap = 0.2 * vs;
    let mut items: Vec<Placed> = Vec::new();

    // Fraction rows (value + label), ink-aligned.
    let row = |text: &str, meaning: &str| {
        let vb = ctx.ts.fit_line(Voice::HERO_NUMBER, text, fw * 0.6, vs);
        let lb = label_block(
            ctx,
            meaning,
            fw - vb.width() - 0.22 * vs,
            (0.26 * vs).max(14.0 * ctx.u),
        );
        let ext = vb.width() + lb.as_ref().map_or(0.0, |l| 0.22 * vs + l.width());
        (vb, lb, ext)
    };
    let (nv, nl, next) = row(t.num, t.num_meaning);
    let (dv, dl, dext) = row(t.den, t.den_meaning);
    let (nt, nb) = ink(ctx, &nv);
    let (dt, dbm) = ink(ctx, &dv);
    let rule_h = (0.04 * vs).max(3.0 * ctx.u);
    let rule_y = (nb - nt) + gap;
    let den_top = rule_y + rule_h + gap;
    let frac_h = den_top + (dbm - dt);
    let rule_w = next.max(dext).min(fw);

    // Result column (relative to its own ink top).
    let rs_cap = 380.0 * u;
    let rb0 = ctx
        .ts
        .fit_line(Voice::HERO_NUMBER, t.result, rw * 0.8, rs_cap);
    let e_size = 0.7 * rb0.size;
    let eq = ctx
        .ts
        .fit_line(Voice::HERO_NUMBER, "=", rw, e_size.max(8.0));
    let gapx = 0.12 * e_size;
    let rb = ctx.ts.fit_line(
        Voice::HERO_NUMBER,
        t.result,
        (rw - eq.width() - gapx).max(40.0),
        rs_cap,
    );
    let (rt, rbm) = ink(ctx, &rb);
    let (et, ebm) = ink(ctx, &eq);
    let rih = rbm - rt;
    let res_x = rx + eq.width() + gapx;
    let rlab = label_block(
        ctx,
        t.result_label,
        rw - eq.width() - gapx,
        (0.15 * rb.size).clamp(18.0 * u, 44.0 * u),
    );
    let mut col_h = rih;
    let lab_y = rih + 0.16 * rb.size;
    if let Some(l) = &rlab {
        let (lt, lb_) = ink(ctx, l);
        col_h = lab_y + (lb_ - lt);
    }
    let bar_h = (14.0 * u).max(6.0 * ctx.u);
    let bar_y = col_h + 0.18 * rb.size;
    if t.bar {
        col_h = bar_y + bar_h;
    }

    // Vertical arrangement.
    let (frac_off, col_off, content_h) = if wide {
        let c = frac_h.max(col_h);
        ((c - frac_h) / 2.0, (c - col_h) / 2.0, c)
    } else {
        let off = frac_h + 0.7 * vs;
        (0.0, off, off + col_h)
    };

    let mid = |top: f32, bot: f32| (top + bot) / 2.0;
    let num_y = frac_off - nt;
    let den_y = frac_off + den_top - dt;
    let lx = |vb: &Block| env.left + vb.width() + 0.22 * vs;
    let ly = |val_y: f32, v_ink: (f32, f32), l: &Block| {
        let (lt, lb_) = ink(ctx, l);
        val_y + mid(v_ink.0, v_ink.1) - mid(lt, lb_)
    };
    if let Some(l) = nl {
        let (x, y) = (lx(&nv), ly(num_y, (nt, nb), &l));
        items.push(Placed {
            role: Role::NumLabel,
            widest: l.width(),
            block: l,
            x,
            y,
        });
    }
    if let Some(l) = dl {
        let (x, y) = (lx(&dv), ly(den_y, (dt, dbm), &l));
        items.push(Placed {
            role: Role::DenLabel,
            widest: l.width(),
            block: l,
            x,
            y,
        });
    }
    items.push(Placed {
        role: Role::NumVal,
        widest: widest(ctx, &nv, "0"),
        block: nv,
        x: env.left,
        y: num_y,
    });
    items.push(Placed {
        role: Role::DenVal,
        widest: widest(ctx, &dv, "0"),
        block: dv,
        x: env.left,
        y: den_y,
    });
    let eq_y = col_off + rih / 2.0 - mid(et, ebm);
    items.push(Placed {
        role: Role::Eq,
        widest: eq.width(),
        block: eq,
        x: rx,
        y: eq_y,
    });
    if let Some(l) = rlab {
        let (lt, _) = ink(ctx, &l);
        items.push(Placed {
            role: Role::ResultLabel,
            widest: l.width(),
            block: l,
            x: res_x,
            y: col_off + lab_y - lt,
        });
    }
    let res_w = widest(ctx, &rb, "0");
    items.push(Placed {
        role: Role::Result,
        widest: res_w,
        block: rb,
        x: res_x,
        y: col_off - rt,
    });
    let bar = t.bar.then_some(BoxRect {
        x: rx,
        y: col_off + bar_y,
        width: rw,
        height: bar_h,
    });

    let mut total = content_h;
    if let Some(c) = t.caption {
        let size = 38.0 * u;
        let blk = ctx
            .ts
            .fit_block(Voice::BODY, c, env.width, 2.0 * size * 1.3, size, 2);
        let y = total + 0.5 * vs;
        total = y + blk.height();
        items.push(Placed {
            role: Role::Caption,
            widest: blk.width(),
            block: blk,
            x: env.left,
            y,
        });
    }
    (
        SinglePlan {
            items,
            rule: (env.left, rule_y + frac_off, rule_w, rule_h),
            bar,
        },
        total,
    )
}

fn single(
    ctx: &mut Ctx,
    b: &mut B,
    env: &Env,
    region: &Region,
    m: &DerivedMetric,
    caption: Option<&str>,
) {
    let ctx: &Ctx = ctx;
    let value = compute(m);
    let fm = format_metric(value, m.format);
    let (nshown, ndec, ntext) = term(m.numerator.value);
    let (dshown, ddec, dtext) = term(m.denominator.value);
    let default_label = match m.format {
        MetricFormat::Decimal => "ratio",
        _ => "rate",
    };
    let result_label = m.meaning.as_deref().unwrap_or(default_label);
    let text = SingleText {
        num: &ntext,
        den: &dtext,
        num_meaning: &m.numerator.meaning,
        den_meaning: &m.denominator.meaning,
        result: &fm.text,
        result_label,
        caption,
        bar: m.format == MetricFormat::Percent && value > 0.0 && value <= 1.0,
    };
    let (plan, total, _) = fit_plan(env.height, |s| plan_single(ctx, env, &text, s));
    let y0 = env.top + 0.25 * (env.height - total).max(0.0);

    let units = if caption.is_some() { 4.4 } else { 3.6 };
    let (t0, p) = pace(b, region, units);
    let d = p.duration;
    let [t_den, t_res, t_rest] = evolve_steps(b, t0);
    let u = ctx.u;
    let ink_c = ctx.palette.ink;
    let mut exp = Expansion::default();
    let term_fmt = |decimals: u8, shown: f64| CountFormat {
        decimals,
        grouping: shown.abs() >= 1000.0,
        prefix: String::new(),
        suffix: String::new(),
    };

    // Rule first so it sits under the rows in draw order.
    let rule_id = b.id("frac.rule");
    b.push(rect_layer(
        rule_id.clone(),
        (plan.rule.0, y0 + plan.rule.1, plan.rule.2, plan.rule.3),
        ctx.palette.ink,
        22,
    ));
    b.motions.push(mo::mask(
        &rule_id,
        t_den,
        0.9 * d,
        Direction::Right,
        Easing::OutQuint,
    ));

    for it in &plan.items {
        let y = y0 + it.y;
        match it.role {
            Role::NumVal | Role::DenVal => {
                let (name, start, shown, dec) = if it.role == Role::NumVal {
                    ("frac.num", t0, nshown, ndec)
                } else {
                    ("frac.den", t_den, dshown, ddec)
                };
                let slot = count_slot(&it.block, ink_c, it.x, y, it.widest);
                exp.extend(dataviz::counter(
                    &b.id(name),
                    &slot,
                    0.0,
                    shown,
                    &term_fmt(dec, shown),
                    start,
                    1.1 * d,
                    &p,
                ));
            }
            Role::NumLabel | Role::DenLabel => {
                let (name, start) = if it.role == Role::NumLabel {
                    ("frac.num.label", t0 + 0.2 * d)
                } else {
                    ("frac.den.label", t_den + 0.2 * d)
                };
                let id = put_text(b, name, &it.block, ctx.palette.muted, it.x, y, 22);
                rise(b, &id, start, &p, u);
            }
            Role::Eq => {
                let id = put_text(b, "eq", &it.block, ctx.palette.muted, it.x, y, 22);
                rise(b, &id, t_res, &p, u);
            }
            Role::Result => {
                let slot = count_slot(&it.block, ink_c, it.x, y, it.widest);
                exp.extend(dataviz::counter(
                    &b.id("result"),
                    &slot,
                    0.0,
                    fm.shown,
                    &CountFormat {
                        decimals: fm.decimals,
                        grouping: fm.grouping(),
                        prefix: fm.prefix.clone(),
                        suffix: fm.suffix.clone(),
                    },
                    t_res,
                    1.6 * d,
                    &p,
                ));
            }
            Role::ResultLabel => {
                let id = put_text(b, "result.label", &it.block, ctx.palette.ink, it.x, y, 22);
                rise(b, &id, t_rest, &p, u);
            }
            Role::Caption => {
                let id = put_text(b, "caption", &it.block, ctx.palette.muted, it.x, y, 22);
                rise(b, &id, t_rest, &p, u);
            }
        }
    }
    if let Some(mut rect) = plan.bar {
        rect.y += y0;
        exp.extend(dataviz::progress_bar(
            &b.id("progress"),
            rect,
            value as f32,
            &data_style(ctx),
            t_rest,
            &p,
        ));
    }
    if plan.bar.is_some() {
        // The empty track arrives with the bar instead of sitting there from frame 0.
        let id = b.id("progress.track");
        b.motions
            .push(mo::fade(&id, t_rest, 0.5 * d, 0.0, 1.0, Easing::OutCubic));
    }
    absorb(b, exp, 22);
}

// ---------------------------------------------------------------------------
// Two metrics
// ---------------------------------------------------------------------------

struct PairText<'a> {
    cap: [String; 2],
    result: [&'a str; 2],
    label: &'a str,
    tags: [&'a str; 2],
    conclusion: &'a str,
}

enum PairRole {
    Cap(usize),
    Label(usize),
    Result(usize),
    Tag(usize),
    Conclusion,
}

struct PairItem {
    role: PairRole,
    block: Block,
    x: f32,
    y: f32,
    widest: f32,
}

struct PairPlan {
    items: Vec<PairItem>,
    bars: BoxRect,
    /// Conclusion slab, y relative.
    slab: BoxRect,
}

fn plan_pair(ctx: &Ctx, env: &Env, t: &PairText, s: f32) -> (PairPlan, f32) {
    let u = ctx.u * s;
    let wide = env.wide();
    let col_gap = 0.06 * env.width;
    let col_w = if wide {
        (env.width - col_gap) / 2.0
    } else {
        env.width
    };
    let mut items: Vec<PairItem> = Vec::new();

    // Shared sizes so both rows are identical in structure.
    let cs = 34.0 * u;
    let widest_res = if t.result[0].chars().count() >= t.result[1].chars().count() {
        t.result[0]
    } else {
        t.result[1]
    };
    let rb0 = ctx
        .ts
        .fit_line(Voice::HERO_NUMBER, widest_res, col_w * 0.62, 230.0 * u);
    let rs = rb0.size;
    let ls = (0.17 * rs).clamp(18.0 * u, 40.0 * u);

    struct Row {
        cap: Block,
        cap_ink: (f32, f32),
        res: Block,
        res_ink: (f32, f32),
        label: Option<Block>,
        height: f32,
    }
    let rows: Vec<Row> = (0..2)
        .map(|i| {
            let cap = floor_type(ctx, cs, |sz| {
                ctx.ts
                    .fit_block(Voice::LABEL, &t.cap[i], col_w, 2.0 * sz * 1.2, sz, 2)
            });
            let cap_ink = ink(ctx, &cap);
            let res = ctx
                .ts
                .fit_line(Voice::HERO_NUMBER, t.result[i], col_w * 0.62, rs);
            let res_ink = ink(ctx, &res);
            let label = label_block(ctx, t.label, col_w - res.width() - 0.15 * rs, ls);
            let height = (cap_ink.1 - cap_ink.0) + 0.3 * cs + (res_ink.1 - res_ink.0);
            Row {
                cap,
                cap_ink,
                res,
                res_ink,
                label,
                height,
            }
        })
        .collect();
    let row_h = rows[0].height.max(rows[1].height);
    let row_gap = 0.3 * rs;
    let rows_h = if wide { row_h } else { 2.0 * row_h + row_gap };

    for (i, r) in rows.into_iter().enumerate() {
        let (rx, ry) = if wide {
            (env.left + i as f32 * (col_w + col_gap), 0.0)
        } else {
            (env.left, i as f32 * (row_h + row_gap))
        };
        let cap_h = r.cap_ink.1 - r.cap_ink.0;
        let res_top = ry + cap_h + 0.3 * cs;
        let rih = r.res_ink.1 - r.res_ink.0;
        if let Some(l) = r.label {
            let (_, lb) = ink(ctx, &l);
            items.push(PairItem {
                role: PairRole::Label(i),
                widest: l.width(),
                x: rx + r.res.width() + 0.15 * rs,
                y: res_top + rih - lb,
                block: l,
            });
        }
        items.push(PairItem {
            role: PairRole::Cap(i),
            widest: r.cap.width(),
            x: rx,
            y: ry - r.cap_ink.0,
            block: r.cap,
        });
        items.push(PairItem {
            role: PairRole::Result(i),
            widest: widest(ctx, &r.res, "0"),
            x: rx,
            y: res_top - r.res_ink.0,
            block: r.res,
        });
    }

    // Comparison bars with a result tag on each row.
    let bh = 120.0 * u;
    let tag_size = {
        let longest = if t.tags[0].chars().count() >= t.tags[1].chars().count() {
            t.tags[0]
        } else {
            t.tags[1]
        };
        ctx.ts
            .fit_line(Voice::LABEL, longest, 0.3 * env.width, 36.0 * u)
            .size
    };
    // Both tags share one size (the longest decides); on non-legacy canvases
    // it is lifted to the legibility floor when the plan shrank it below.
    let tag_size = {
        let longest = if t.tags[0].chars().count() >= t.tags[1].chars().count() {
            t.tags[0]
        } else {
            t.tags[1]
        };
        let probe = ctx
            .ts
            .fit_line(Voice::LABEL, longest, 0.3 * env.width, tag_size);
        tag_size * floor_scale(ctx, probe.size)
    };
    let tags: Vec<Block> = t
        .tags
        .iter()
        .map(|s| ctx.ts.fit_line(Voice::LABEL, s, 0.3 * env.width, tag_size))
        .collect();
    let gutter = tags.iter().map(Block::width).fold(0.0, f32::max) + 0.03 * env.width;
    let bars_y = rows_h + 0.45 * rs;
    let bars = BoxRect {
        x: env.left + gutter,
        y: bars_y,
        width: (env.width - gutter).max(1.0),
        height: bh,
    };
    let bar_row_h = 0.4 * bh;
    for (i, blk) in tags.into_iter().enumerate() {
        let (tt, tb) = ink(ctx, &blk);
        let cy = bars_y + bar_row_h / 2.0 + i as f32 * (bar_row_h + 0.2 * bh);
        items.push(PairItem {
            role: PairRole::Tag(i),
            widest: blk.width(),
            x: env.left,
            y: cy - (tt + tb) / 2.0,
            block: blk,
        });
    }

    // Conclusion on an accent slab.
    let pad = 26.0 * u;
    let size = 84.0 * u;
    let cblk = ctx.ts.fit_block(
        Voice::SERIF,
        t.conclusion,
        env.width - 2.8 * pad,
        2.0 * size * 1.08,
        size,
        2,
    );
    let (ct, cb) = ink(ctx, &cblk);
    let slab_y = bars_y + bh + 0.35 * rs;
    let slab_h = (cb - ct) + 2.0 * pad;
    items.push(PairItem {
        role: PairRole::Conclusion,
        widest: cblk.width(),
        x: env.left + 1.4 * pad,
        y: slab_y + pad - ct,
        block: cblk,
    });
    (
        PairPlan {
            items,
            bars,
            slab: BoxRect {
                x: env.left,
                y: slab_y,
                width: env.width,
                height: slab_h,
            },
        },
        slab_y + slab_h,
    )
}

fn pair(
    ctx: &mut Ctx,
    b: &mut B,
    env: &Env,
    region: &Region,
    a: &DerivedMetric,
    c: &DerivedMetric,
) {
    let ctx: &Ctx = ctx;
    let (va, vc) = (compute(a), compute(c));
    // Comparable: both use the primary's format.
    let fa = format_metric(va, a.format);
    let fc = format_metric(vc, a.format);
    let concl = conclusion(a, c, &fa, &fc);
    let label = a
        .meaning
        .as_deref()
        .or(c.meaning.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(match a.format {
            MetricFormat::Decimal => "ratio",
            _ => "rate",
        });
    let line = |m: &DerivedMetric| {
        format!(
            "{} {} ÷ {} {}",
            term(m.numerator.value).2,
            m.numerator.meaning.trim(),
            term(m.denominator.value).2,
            m.denominator.meaning.trim()
        )
    };
    let text = PairText {
        cap: [line(a), line(c)],
        result: [&fa.text, &fc.text],
        label,
        tags: [&fa.text, &fc.text],
        conclusion: &concl,
    };
    let (plan, total, _) = fit_plan(env.height, |s| plan_pair(ctx, env, &text, s));
    let y0 = env.top + 0.25 * (env.height - total).max(0.0);

    let (t0, p) = pace(b, region, 5.4);
    let d = p.duration;
    let u = ctx.u;
    let [t_row2, t_bar, t_concl] = evolve_steps(b, t0);
    let row_start = |i: usize| if i == 0 { t0 } else { t_row2 };
    let mut exp = Expansion::default();

    // Slab first so the conclusion text draws over it.
    let slab_id = b.id("conclusion.slab");
    b.push(rect_layer(
        slab_id.clone(),
        (
            plan.slab.x,
            y0 + plan.slab.y,
            plan.slab.width,
            plan.slab.height,
        ),
        ctx.palette.accent,
        21,
    ));
    b.motions.push(mo::mask(
        &slab_id,
        t_concl,
        1.0 * d,
        Direction::Right,
        Easing::OutQuint,
    ));

    let fmts = [&fa, &fc];
    for it in &plan.items {
        let y = y0 + it.y;
        match it.role {
            PairRole::Cap(i) => {
                let id = put_text(
                    b,
                    &format!("row.{i}.caption"),
                    &it.block,
                    ctx.palette.muted,
                    it.x,
                    y,
                    22,
                );
                rise(b, &id, row_start(i), &p, u);
            }
            PairRole::Result(i) => {
                let f = fmts[i];
                let slot = count_slot(&it.block, ctx.palette.ink, it.x, y, it.widest);
                exp.extend(dataviz::counter(
                    &b.id(&format!("row.{i}.result")),
                    &slot,
                    0.0,
                    f.shown,
                    &CountFormat {
                        decimals: f.decimals,
                        grouping: f.grouping(),
                        prefix: f.prefix.clone(),
                        suffix: f.suffix.clone(),
                    },
                    row_start(i) + 0.15 * d,
                    1.4 * d,
                    &p,
                ));
            }
            PairRole::Label(i) => {
                let id = put_text(
                    b,
                    &format!("row.{i}.label"),
                    &it.block,
                    ctx.palette.ink,
                    it.x,
                    y,
                    22,
                );
                rise(b, &id, row_start(i) + 0.6 * d, &p, u);
            }
            PairRole::Tag(i) => {
                let id = put_text(
                    b,
                    &format!("bars.{i}.tag"),
                    &it.block,
                    ctx.palette.ink,
                    it.x,
                    y,
                    22,
                );
                // Row b of the comparison bar starts 0.35 * (1.5 d) later.
                let start = t_bar + i as f64 * 0.525 * d;
                b.motions
                    .push(mo::fade(&id, start, 0.5 * d, 0.0, 1.0, Easing::OutCubic));
            }
            PairRole::Conclusion => {
                let id = put_text(
                    b,
                    "conclusion",
                    &it.block,
                    ctx.palette.on_accent,
                    it.x,
                    y,
                    23,
                );
                rise(b, &id, t_concl + 0.35 * d, &p, u);
            }
        }
    }
    let mut bars = plan.bars;
    bars.y += y0;
    exp.extend(dataviz::comparison_bar(
        &b.id("bars"),
        bars,
        va,
        vc,
        &data_style(ctx),
        t_bar,
        &p,
    ));
    for row in ["a", "b"] {
        // Empty tracks arrive with the bars, not from frame 0.
        let id = b.id(&format!("bars.{row}.track"));
        b.motions
            .push(mo::fade(&id, t_bar, 0.5 * d, 0.0, 1.0, Easing::OutCubic));
    }
    absorb(b, exp, 22);
}
