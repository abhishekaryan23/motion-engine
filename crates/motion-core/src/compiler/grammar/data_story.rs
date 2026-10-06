//! DataStory: numbers, ratios, comparisons and series unfolding as data.
//!
//! Atomic / number shapes (structured derived metrics are composed by
//! `derived::compose`):
//! - **NumberPair**: headline, two big counters (primary in ENTER, secondary at
//!   the first EVOLVE event), a comparison bar, then a computed direction line.
//! - **NumberSeries** over time (years, quarters, months, or unnamed
//!   numbers): headline, a bar chart whose bars grow one per event, the trend
//!   line drawn through the bar tops, the peak emphasized.
//! - **NumberSeries** of named things (0.22: any picture, or meanings that
//!   are not times): a ranking of horizontal bars in the written order, each
//!   row with its picture, name and value, growing when it is named
//!   ([`ranked_bars`]).
//! - **Atomic**: a counting hero number on an accent slab; its meaning line and
//!   (for percentages) a progress bar arrive in EVOLVE.
//!
//! Nothing new starts at or after `life.anticipate`.

use super::{sphere_gallery, Composition, Shape};
use crate::compiler::recipes::{self, Entrance, Region, Slot, B};
use crate::compiler::typeset::{text_layer, Block, Typesetter, Voice};
use crate::compiler::{mo, parse_count, round3, wants_carry, Carry, CompileError, Ctx, Which};
use crate::easing::{Easing, PresetParams};
use crate::intent::{CollectionItem, ObjectAtom, Subject};
use crate::motion::dataviz::{self, DataStyle};
use crate::scene::{BoxRect, Color, Direction, Layer, MotionOp, TextAlign};
use crate::speech::RevealRole;

type Count = (f64, u8, bool, String, String);

pub(super) fn build(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    c: Composition,
) -> Result<(), CompileError> {
    match c.shape {
        Shape::NumberPair => number_pair(ctx, b, carries),
        Shape::NumberSeries => number_series(ctx, b),
        _ => number_reveal(ctx, b, carries),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn data_style(ctx: &Ctx) -> DataStyle {
    DataStyle {
        ink: ctx.palette.ink,
        accent: ctx.palette.accent,
        muted: ctx.palette.muted,
        track: ctx.palette.muted.with_alpha(0x44),
        u: ctx.u,
    }
}

/// Last moment any scheduled data motion may still be running: just before the
/// outgoing transition.
fn end_limit(b: &B) -> f64 {
    let life = b.plan.life;
    (life.bridge - 0.05)
        .max(life.anticipate)
        .min(b.plan.duration)
}

/// Preset whose entrance duration is short enough that an animation needing
/// `span` × duration seconds, started at `start`, ends before `limit`.
fn paced(p: PresetParams, start: f64, limit: f64, span: f64) -> PresetParams {
    let room = (limit - start).max(0.2);
    PresetParams {
        duration: p.duration.min(room / span).max(0.15),
        ..p
    }
}

/// Make sure the number layer `id` counts (the data language already does it
/// inside `place_subject`; other languages get the same counter), and keep
/// every count started since `mark` inside `limit`.
fn count_up(b: &mut B, mark: usize, id: Option<&str>, text: &str, start: f64, limit: f64) {
    let p = b.plan.lang.preset;
    if let Some(id) = id {
        let has = b.motions[mark..]
            .iter()
            .any(|m| m.target == id && matches!(m.op, MotionOp::Count { .. }));
        if !has {
            if let Some(m) = mo::count(
                id,
                start + p.duration * 0.15,
                (p.duration * 2.0).max(1.1),
                text,
                b.plan.lang.settle,
            ) {
                b.motions.push(m);
            }
        }
    }
    for m in b.motions[mark..].iter_mut() {
        if matches!(m.op, MotionOp::Count { .. }) {
            m.duration = m.duration.min((limit - m.start).max(0.3));
        }
    }
}

/// Text layer centered on `(cx, cy)` (anchored at its center, so scale pulses
/// grow from the middle).
fn centered_label(
    ctx: &Ctx,
    id: String,
    block: &Block,
    color: Color,
    (cx, cy): (f32, f32),
    z: i32,
) -> Layer {
    let mut l = recipes::with_ink(ctx, text_layer(id, block, color, TextAlign::Center), block);
    l.x = cx;
    l.y = cy;
    l.anchor_x = 0.5;
    l.anchor_y = 0.5;
    l.z_index = z;
    l
}

/// Empty tracks are information too: they arrive with the bars that fill them.
fn fade_tracks_in(b: &mut B, layers: &[Layer], at: f64) {
    for l in layers.iter().filter(|l| l.id.ends_with(".track")) {
        b.motions
            .push(mo::fade(&l.id, at, 0.3, 0.0, 1.0, Easing::OutCubic));
    }
}

fn beat_error(b: &B, message: &str) -> CompileError {
    CompileError::Beat {
        beat: b.plan.index + 1,
        message: message.into(),
    }
}

/// `12.30` → `12.3`, `4.0` → `4`.
fn trim_num(x: f64, decimals: usize) -> String {
    let s = format!("{x:.decimals$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

/// The engine-written direction line of a pair: a ratio when both numbers
/// share a unit and are positive, a difference in points when they are
/// percentages, nothing (no invented claim) otherwise.
fn direction_text(a: &Count, b: &Count) -> Option<String> {
    if a.3 != b.3 || a.4 != b.4 {
        return None;
    }
    if a.4 == "%" {
        let diff = (a.0 - b.0).abs();
        if diff < 1e-9 {
            return Some("the same".into());
        }
        let d = trim_num(diff, a.1.max(b.1) as usize);
        return Some(format!("{d} points higher"));
    }
    if a.0 <= 0.0 || b.0 <= 0.0 {
        return None;
    }
    let ratio = a.0.max(b.0) / a.0.min(b.0);
    if ratio < 1.05 {
        return Some("about the same".into());
    }
    let r = if ratio >= 10.0 {
        format!("{}", ratio.round())
    } else {
        trim_num(ratio, 1)
    };
    Some(format!("{r}× larger"))
}

// ---------------------------------------------------------------------------
// NumberPair
// ---------------------------------------------------------------------------

/// One counter row: a small meaning label above a big left-aligned number.
#[allow(clippy::too_many_arguments)]
fn number_row(
    ctx: &mut Ctx,
    b: &mut B,
    carries: &mut Vec<Carry>,
    which: Which,
    subject: &Subject,
    top: f32,
    row_h: f32,
    color: Color,
    enter: f64,
) -> Result<(), CompileError> {
    let (w, u, m) = (ctx.w, ctx.u, ctx.margin());
    let (role, name) = match which {
        Which::Primary => ("hero", "label_a"),
        Which::Secondary => ("support", "label_b"),
    };
    let mut y = top;
    if let Some(meaning) = subject.meaning() {
        let label = ctx
            .ts
            .fit_line(Voice::LABEL, meaning, w - 2.0 * m, 30.0 * u);
        let lh = label.height();
        recipes::headline_lines(
            ctx,
            b,
            name,
            &label,
            y,
            TextAlign::Left,
            ctx.palette.ink,
            enter,
            false,
        );
        y += lh + 12.0 * u;
    }
    let text = subject.display_text().unwrap_or("");
    let block = ctx.ts.fit_line(
        Voice::HERO_NUMBER,
        text,
        w - 2.0 * m,
        ((top + row_h - y) * 0.86).max(24.0 * u),
    );
    let bw = block.width() * 1.02 + 2.0;
    let slot = Slot {
        cx: m + bw / 2.0,
        cy: y + block.height() / 2.0,
        w: bw,
        h: block.height(),
    };
    let mark = b.motions.len();
    let limit = end_limit(b);
    let placed = recipes::place_subject(
        ctx,
        b,
        carries,
        which,
        subject,
        slot,
        (0.0, 0.0),
        color,
        enter + 0.1,
        Entrance::Rise,
        None,
        None,
        role,
    )?;
    if let Some(mut l) = placed {
        l.z_index = 22;
        count_up(b, mark, Some(&l.id), text, enter + 0.1, limit);
        b.push(l);
    } else {
        count_up(b, mark, None, text, enter + 0.1, limit);
    }
    Ok(())
}

fn number_pair(ctx: &mut Ctx, b: &mut B, carries: &mut Vec<Carry>) -> Result<(), CompileError> {
    let region = recipes::structured_head(ctx, b);
    let plan = b.plan;
    let life = plan.life;
    let beat = b.beat;
    let (w, u, m) = (ctx.w, ctx.u, ctx.margin());
    let sec = beat
        .secondary
        .as_ref()
        .ok_or_else(|| beat_error(b, "number pair needs a secondary number"))?;
    let (Some(a), Some(c)) = (
        beat.primary.value().and_then(parse_count),
        sec.value().and_then(parse_count),
    ) else {
        return Err(beat_error(b, "number pair needs two countable numbers"));
    };

    let hr = (region.bottom - region.top).max(1.0);
    let row_h = 0.3 * hr;
    let ev = life.evolve_events(3);
    let t1 = (region.start - 0.3).clamp(life.enter, life.settle.max(life.enter));
    let limit = end_limit(b);

    // The larger number wears the accent (a tie keeps both in ink).
    let (col_a, col_b) = match a.0.partial_cmp(&c.0) {
        Some(std::cmp::Ordering::Greater) => (ctx.palette.accent, ctx.palette.ink),
        Some(std::cmp::Ordering::Less) => (ctx.palette.ink, ctx.palette.accent),
        _ => (ctx.palette.ink, ctx.palette.ink),
    };
    number_row(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        region.top,
        row_h,
        col_a,
        t1,
    )?;
    number_row(
        ctx,
        b,
        carries,
        Which::Secondary,
        sec,
        region.top + row_h,
        row_h,
        col_b,
        ev[0],
    )?;

    // The comparison bar, then the direction line.
    let bar_rect = BoxRect {
        x: m,
        y: region.top + 2.0 * row_h + 0.04 * hr,
        width: w - 2.0 * m,
        height: 0.2 * hr,
    };
    let p = paced(plan.lang.preset, ev[1], limit, 2.3);
    let mut bar = dataviz::comparison_bar(
        &b.id("compare"),
        bar_rect,
        a.0,
        c.0,
        &data_style(ctx),
        ev[1],
        &p,
    );
    let depth = recipes::plane(plan.lang.depth.midground);
    for l in bar.layers.iter_mut() {
        l.depth = depth;
    }
    fade_tracks_in(b, &bar.layers, ev[1]);
    recipes::absorb(b, bar, 12);

    if let Some(text) = direction_text(&a, &c) {
        let line = ctx.ts.fit_line(Voice::SERIF, &text, w - 2.0 * m, 84.0 * u);
        recipes::headline_lines(
            ctx,
            b,
            "direction",
            &line,
            bar_rect.y + bar_rect.height + 0.05 * hr,
            TextAlign::Left,
            ctx.palette.ink,
            ev[2],
            false,
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// NumberSeries
// ---------------------------------------------------------------------------

struct Bar {
    value: String,
    meaning: Option<String>,
    magnitude: f64,
    /// (0.22) What the bar stands for: the item's meaning, else an object's
    /// humanised asset name.
    name: Option<String>,
    /// (0.22) An object item: its picture rides the bar's row.
    object: Option<ObjectAtom>,
}

fn series_items(subject: &Subject) -> Option<Vec<Bar>> {
    let Subject::Collection(c) = subject else {
        return None;
    };
    fn clean(s: &Option<String>) -> Option<&str> {
        s.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
    c.items
        .iter()
        .map(|i| {
            let (value, meaning, object) = match i {
                CollectionItem::Number(a) => (clean(&a.value)?, a.meaning.clone(), None),
                CollectionItem::Object(o) => (clean(&o.value)?, o.meaning.clone(), Some(o)),
                CollectionItem::Phrase(_) => return None,
            };
            let n = parse_count(value)?.0;
            let name = clean(&meaning)
                .map(str::to_string)
                .or_else(|| object.map(|o| sphere_gallery::humanise(&o.asset)))
                .filter(|s| !s.is_empty());
            Some(Bar {
                value: value.to_string(),
                meaning,
                magnitude: if n.is_finite() { n.max(0.0) } else { 0.0 },
                name,
                object: object.cloned(),
            })
        })
        .collect()
}

/// (0.22) Whether a label names a point or span of time — a year ("2019",
/// "FY24", "1990s"), a quarter or half ("Q1", "H2 2024"), a month or weekday
/// ("Jan", "March 2024", "Mon"), a numbered period ("Week 3", "Year 1"), a
/// date ("2024-01-05") — so its series reads left to right in time.
fn time_like(label: &str) -> bool {
    const MONTHS: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    const DAYS: [&str; 7] = [
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
    ];
    const PERIODS: [&str; 22] = [
        "year", "years", "yr", "quarter", "qtr", "half", "month", "months", "week", "weeks", "wk",
        "day", "days", "hour", "hours", "decade", "today", "now", "ytd", "est", "forecast", "fy",
    ];
    let named =
        |t: &str, list: &[&str]| t.len() >= 3 && list.iter().any(|full| full.starts_with(t));
    // "2019", "1990s", "2025e", "'24"
    let year = |t: &str| {
        let core = t.strip_suffix(['s', 'e', 'f']).unwrap_or(t);
        !core.is_empty() && core.chars().all(|c| c.is_ascii_digit())
    };
    // "q1", "h2", "fy24", "fy26e", "cy2023", "1q24"
    let tagged = |t: &str| {
        let digits = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
        ["q", "h", "fy", "cy"].iter().any(|p| {
            t.strip_prefix(p)
                .is_some_and(|rest| digits(rest.strip_suffix(['e', 'f']).unwrap_or(rest)))
        }) || t
            .split_once('q')
            .is_some_and(|(a, b)| digits(a) && digits(b))
    };
    let tokens: Vec<String> = label
        .split(|c: char| {
            c.is_whitespace() || matches!(c, '-' | '–' | '/' | ',' | '.' | '\'' | '’' | '(' | ')')
        })
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();
    !tokens.is_empty()
        && tokens.iter().all(|t| {
            year(t)
                || tagged(t)
                || named(t, &MONTHS)
                || named(t, &DAYS)
                || PERIODS.contains(&t.as_str())
        })
}

/// (0.22) Named things — any picture, or a label that is not a time — are
/// compared as a ranking (horizontal bars); a series over time keeps its
/// vertical bars and trend line.
fn categorical(items: &[Bar]) -> bool {
    items.iter().any(|i| i.object.is_some())
        || items
            .iter()
            .filter_map(|i| i.meaning.as_deref())
            .map(str::trim)
            .any(|m| !m.is_empty() && !time_like(m))
}

fn number_series(ctx: &mut Ctx, b: &mut B) -> Result<(), CompileError> {
    let region = recipes::structured_head(ctx, b);
    let beat = b.beat;
    let items = series_items(&beat.primary)
        .or_else(|| beat.secondary.as_ref().and_then(series_items))
        .filter(|i| !i.is_empty())
        .ok_or_else(|| beat_error(b, "number series needs a collection of numbers"))?;
    if categorical(&items) {
        return ranked_bars(ctx, b, region, &items);
    }
    let plan = b.plan;
    let life = plan.life;
    let (w, u, m) = (ctx.w, ctx.u, ctx.margin());
    let n = items.len();
    let mags: Vec<f64> = items.iter().map(|i| i.magnitude).collect();
    let max = mags.iter().copied().fold(0.0_f64, f64::max);
    // The peak (the last one on ties) is what gets emphasized.
    let peak = mags
        .iter()
        .enumerate()
        .fold(0, |best, (i, v)| if *v >= mags[best] { i } else { best });

    let hr = (region.bottom - region.top).max(1.0);
    let chart = BoxRect {
        x: m,
        y: region.top + 0.05 * hr,
        width: w - 2.0 * m,
        height: 0.66 * hr,
    };
    let baseline = chart.y + chart.height;
    let slot_w = chart.width / n as f32;
    let limit = end_limit(b);
    let p = plan.lang.preset;
    let style = data_style(ctx);
    let depth = recipes::plane(plan.lang.depth.midground);

    // Timing: baseline in ENTER, bar 0 at SETTLE, one bar per EVOLVE event,
    // the trend line on the last event.
    let t1 = (region.start - 0.3).clamp(life.enter, life.settle.max(life.enter));
    let t_first = life.settle.max(t1 + 0.25).min(life.evolve);
    let ev = life.evolve_events(n);
    let bar_start: Vec<f64> = (0..n)
        .map(|i| if i == 0 { t_first } else { ev[i - 1] })
        .collect();

    let mut chart_exp = dataviz::bar_chart(
        &b.id("chart"),
        chart,
        &mags,
        Some(peak),
        &style,
        t1,
        &plan.lang.preset,
        plan.lang.stagger,
    );
    let chart_id = b.id("chart");
    for motion in chart_exp.motions.iter_mut() {
        let Some(i) = (0..n).find(|i| motion.target == format!("{chart_id}.bar.{i}")) else {
            continue;
        };
        motion.start = bar_start[i];
        motion.duration = motion
            .duration
            .min(0.75)
            .min((limit - motion.start).max(0.2));
    }
    for l in chart_exp.layers.iter_mut() {
        l.depth = depth;
    }
    recipes::absorb(b, chart_exp, 12);

    // Labels under the baseline: the value, then what it is. One size for all.
    let inner = slot_w * 0.94;
    let value_size = items
        .iter()
        .map(|i| {
            ctx.ts
                .fit_line(Voice::HERO_NUMBER, &i.value, inner, 56.0 * u)
                .size
        })
        .fold(56.0 * u, f32::min);
    let meaning_lines = 2.0 * Voice::LABEL.line_height;
    let meaning_size = items
        .iter()
        .filter_map(|i| i.meaning.as_deref())
        .map(|t| {
            ctx.ts
                .fit_block(
                    Voice::LABEL,
                    t,
                    inner,
                    30.0 * u * meaning_lines,
                    30.0 * u,
                    2,
                )
                .size
        })
        .fold(30.0 * u, f32::min);
    let label_top = baseline + 3.0 * u + 16.0 * u;
    let mut pulse_target = None;
    for (i, item) in items.iter().enumerate() {
        let cx = chart.x + slot_w * (i as f32 + 0.5);
        let t = bar_start[i] + 0.2;
        let vb = ctx
            .ts
            .fit_line(Voice::HERO_NUMBER, &item.value, inner, value_size);
        let vid = b.id(&format!("value.{i}"));
        let color = if i == peak {
            ctx.palette.accent
        } else {
            ctx.palette.ink
        };
        let mut vl = centered_label(
            ctx,
            vid.clone(),
            &vb,
            color,
            (cx, label_top + vb.height() / 2.0),
            22,
        );
        vl.depth = depth;
        b.motions
            .push(mo::fade(&vid, t, 0.4, 0.0, 1.0, Easing::OutCubic));
        b.push(vl);
        if i == peak {
            pulse_target = Some((vid, cx, label_top + vb.height() / 2.0));
        }
        if let Some(meaning) = item.meaning.as_deref() {
            let mb = ctx.ts.fit_block(
                Voice::LABEL,
                meaning,
                inner,
                meaning_size * meaning_lines,
                meaning_size,
                2,
            );
            let mid = b.id(&format!("meaning.{i}"));
            let mut ml = centered_label(
                ctx,
                mid.clone(),
                &mb,
                ctx.palette.muted,
                (cx, label_top + vb.height() + 10.0 * u + mb.height() / 2.0),
                22,
            );
            ml.depth = depth;
            b.motions
                .push(mo::fade(&mid, t, 0.4, 0.0, 1.0, Easing::OutCubic));
            b.push(ml);
        }
    }

    // The trend line runs through the bar tops (same x centers, same heights).
    let t_path = ev[n - 1];
    let path_dur = ((limit - t_path - 0.35) / 0.92).clamp(0.2, 1.2);
    let span = max - mags.iter().copied().fold(f64::INFINITY, f64::min);
    let min = mags.iter().copied().fold(f64::INFINITY, f64::min);
    let path_rect = if max > 0.0 && span > 0.0 {
        BoxRect {
            x: chart.x + slot_w / 2.0,
            y: chart.y,
            width: chart.width - slot_w,
            height: chart.height * ((max - min) / max) as f32,
        }
    } else {
        // Flat series: a line along the shared top.
        BoxRect {
            x: chart.x + slot_w / 2.0,
            y: chart.y - u,
            width: chart.width - slot_w,
            height: 2.0 * u,
        }
    };
    let mut path = dataviz::path_draw(
        &b.id("trend"),
        path_rect,
        &mags,
        &style,
        t_path,
        path_dur,
        &p,
    );
    for l in path.layers.iter_mut() {
        l.depth = depth;
    }
    recipes::absorb(b, path, 24);

    // Emphasize the peak value as the line lands (still inside EVOLVE).
    if let Some((target, ..)) = pulse_target {
        let (up, down) = (0.2, 0.3);
        let at = (t_path + path_dur * 0.6)
            .min(life.anticipate - 0.05)
            .min(plan.duration - 0.02 - up - down);
        if at >= t_path {
            b.motions
                .push(mo::scale(&target, at, up, 1.0, 1.12, Easing::OutCubic));
            b.motions.push(mo::scale(
                &target,
                at + up,
                down,
                1.12,
                1.0,
                Easing::InOutCubic,
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Ranked bars (0.22)
// ---------------------------------------------------------------------------

/// A row's height as a share of its pitch (the rest is the gap under it).
const ROW_FILL: f32 = 0.76;
/// The tallest a row gets (canvas units): three rows on a tall canvas stay
/// a list, not three posters.
const ROW_MAX_U: f32 = 190.0;
/// From this much content width per unit of row height a row's name sits
/// beside its bar (wide rows); below it, above the bar (tall canvases).
const INLINE_RATIO: f32 = 6.0;
/// Smallest name size (canvas units; the type floor is 20u).
const NAME_FLOOR_U: f32 = 22.0;
const NAME_MAX_U: f32 = 34.0;

/// One line of `text` set at exactly `size`.
fn line_at(ctx: &Ctx, voice: Voice, text: &str, size: f32) -> Block {
    let line = Typesetter::prepare(&voice, text);
    let width = ctx.ts.width(&voice, &line, size);
    Block {
        lines: vec![line],
        size,
        line_widths: vec![width],
        voice,
    }
}

/// One label size for every name: the largest at most `cap` at which each
/// sets in at most `lines` lines of `width`, never below `floor`.
fn label_size(ctx: &Ctx, names: &[&str], width: f32, cap: f32, floor: f32, lines: usize) -> f32 {
    let fits = |s: f32| {
        names.iter().filter(|t| !t.trim().is_empty()).all(|t| {
            let bl = ctx.ts.set_wrapped(Voice::LABEL, t, s, width);
            bl.lines.len() <= lines && bl.width() * 1.02 + 2.0 <= width
        })
    };
    let mut s = cap.max(floor);
    while s > floor && !fits(s) {
        s = (s * 0.94).max(floor);
    }
    s
}

/// (0.22) A ranking: a collection of named things with numbers (pictures
/// with values, or numbers whose meanings are not times) as horizontal bars.
///
/// One row per item in the WRITTEN order (the narrator names them in that
/// order; a story written high → low reads as a ranking), left to right: the
/// picture (a square thumbnail of the row height, when the item has one), the
/// name (beside the bar on wide rows, above it on tall canvases), the bar
/// (length ∝ value / largest value) and the value riding the bar's tip as it
/// grows and counts up. The largest bar and its value wear the accent. Rows
/// share the free band under the headline (at most [`ROW_MAX_U`] tall).
///
/// Timing: row 0 at SETTLE, then one row per EVOLVE event; each row is one
/// group `rank.{i}` (`rank.{i}.pic`, `.name`, `.bar`, `.value`) declared as a
/// reveal anchor on its name and value, so with a voice-over it arrives when
/// the narrator names it. Every bar has landed before ANTICIPATE.
fn ranked_bars(
    ctx: &mut Ctx,
    b: &mut B,
    region: Region,
    items: &[Bar],
) -> Result<(), CompileError> {
    let plan = b.plan;
    let life = plan.life;
    let (w, u, m) = (ctx.w, ctx.u, ctx.margin());
    let count = items.len().max(1);
    let safe = ctx.frame.safe;
    let top = region.top;
    let band = (region.bottom.min(safe.y + safe.h) - top).max(0.0);
    let pitch = (band / count as f32).min(ROW_MAX_U * u / ROW_FILL);
    let row_h = ROW_FILL * pitch;
    let y0 = top + 0.3 * (band - pitch * count as f32);
    let row_top = |i: usize| y0 + pitch * i as f32;
    let depth = recipes::plane(plan.lang.depth.midground);
    let gap = 24.0 * u;

    // Pictures: square thumbnails of the row height at the left.
    let mut pictures: Vec<Option<Layer>> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let picture = match &item.object {
            Some(o) => {
                let slot = Slot {
                    cx: m + row_h / 2.0,
                    cy: row_top(i) + row_h / 2.0,
                    w: row_h,
                    h: row_h,
                };
                let id = b.id(&format!("rank.{i}.pic"));
                let library = sphere_gallery::library_picture(ctx, b.plan.index, i, o);
                let mut pic = sphere_gallery::item_picture(ctx, b, id, o, i, slot, 16)?;
                // (0.22) A library picture's colour is unknown: the print
                // border keeps a dark flag readable on a dark (brand) ground.
                if let Some(l) = pic.as_mut().filter(|_| library) {
                    super::dossier::print_border(l, ctx.palette.ink, ctx.u);
                }
                pic
            }
            None => None,
        };
        pictures.push(picture);
    }
    let x0 = if pictures.iter().any(Option::is_some) {
        m + row_h + gap
    } else {
        m
    };
    let avail = (w - m - x0).max(1.0);
    let inline = avail >= INLINE_RATIO * row_h;

    // Names: one size for every row.
    let floor = NAME_FLOOR_U * u;
    let names: Vec<&str> = items
        .iter()
        .map(|i| i.name.as_deref().unwrap_or_default())
        .collect();
    let (name_w, name_cap, lines) = if inline {
        (0.3 * avail, (0.26 * row_h).clamp(floor, NAME_MAX_U * u), 2)
    } else {
        (avail, (0.2 * row_h).clamp(floor, NAME_MAX_U * u), 1)
    };
    let size = label_size(ctx, &names, name_w, name_cap, floor, lines);
    let name_blocks: Vec<Option<Block>> = names
        .iter()
        .map(|t| (!t.trim().is_empty()).then(|| ctx.ts.set_wrapped(Voice::LABEL, t, size, name_w)))
        .collect();
    let name_h = name_blocks
        .iter()
        .flatten()
        .map(Block::height)
        .fold(0.0, f32::max);
    let name_col = name_blocks
        .iter()
        .flatten()
        .map(|bl| bl.width() * 1.02 + 2.0)
        .fold(0.0, f32::max);

    // Bars, and the values at their tips (one size for every value).
    let (bar_h, area_x) = if inline {
        let x = if name_col > 0.0 {
            x0 + name_col + gap
        } else {
            x0
        };
        (0.56 * row_h, x)
    } else {
        let gap_y = if name_h > 0.0 { 10.0 * u } else { 0.0 };
        ((row_h - name_h - gap_y).max(0.3 * row_h), x0)
    };
    let area_w = (w - m - area_x).max(1.0);
    let value_cap = if inline { 0.72 * row_h } else { 0.9 * bar_h };
    let value_size = items
        .iter()
        .map(|i| {
            ctx.ts
                .fit_line(Voice::HERO_NUMBER, &i.value, 0.34 * area_w, value_cap)
                .size
        })
        .fold(f32::MAX, f32::min);
    let value_blocks: Vec<Block> = items
        .iter()
        .map(|i| line_at(ctx, Voice::HERO_NUMBER, &i.value, value_size))
        .collect();
    let value_w = value_blocks
        .iter()
        .map(|bl| bl.width() * 1.02 + 2.0)
        .fold(0.0, f32::max);
    let value_gap = 16.0 * u;
    let full = (area_w - value_w - value_gap).max(0.3 * area_w);
    let bar_rows: Vec<BoxRect> = (0..items.len())
        .map(|i| BoxRect {
            x: area_x,
            y: if inline {
                row_top(i) + (row_h - bar_h) / 2.0
            } else {
                row_top(i) + row_h - bar_h
            },
            width: full,
            height: bar_h,
        })
        .collect();

    // Timing: row 0 at SETTLE, then one row per EVOLVE event; every bar
    // lands before ANTICIPATE.
    let t1 = (region.start - 0.3).clamp(life.enter, life.settle.max(life.enter));
    let t_first = life.settle.max(t1 + 0.25).min(life.evolve);
    let ev = life.evolve_events(items.len());
    let starts: Vec<f64> = (0..items.len())
        .map(|i| if i == 0 { t_first } else { ev[i - 1] })
        .collect();
    let limit = end_limit(b).min(life.anticipate) - 0.02;
    let mags: Vec<f64> = items.iter().map(|i| i.magnitude).collect();
    // The largest value (the first on ties: a ranking reads top down).
    let peak = mags
        .iter()
        .enumerate()
        .fold(0, |best, (i, v)| if *v > mags[best] { i } else { best });
    let grow_at: Vec<f64> = starts.iter().map(|s| s + 0.12).collect();
    let rank_id = b.id("rank");
    let mut chart = dataviz::hbar_chart(
        &rank_id,
        &bar_rows,
        &mags,
        Some(peak),
        &data_style(ctx),
        &grow_at,
        &plan.lang.preset,
    );
    let mut grows: Vec<(f64, f64, Easing)> = vec![(0.0, 0.0, Easing::OutCubic); items.len()];
    for motion in chart.motions.iter_mut() {
        let Some(i) = (0..items.len()).find(|i| motion.target == format!("{rank_id}.{i}.bar"))
        else {
            continue;
        };
        let dur = motion.duration.clamp(0.35, 0.9);
        let dur = dur.min((limit - motion.start).max(0.25));
        motion.start = round3(motion.start.min(limit - dur));
        motion.duration = round3(dur);
        grows[i] = (motion.start, motion.duration, motion.easing);
    }
    for l in chart.layers.iter_mut() {
        l.depth = depth;
    }
    recipes::absorb(b, chart, 12);

    let fractions = dataviz::bar_fractions(&mags);
    for (i, item) in items.iter().enumerate() {
        let s = starts[i];
        let (g, d, easing) = grows[i];
        if let Some(mut pic) = pictures[i].take() {
            pic.depth = depth;
            b.motions
                .push(mo::fade(&pic.id, s, 0.35, 0.0, 1.0, Easing::OutCubic));
            b.motions.push(mo::shift(
                &pic.id,
                s,
                0.5,
                [-24.0 * u, 0.0],
                [0.0, 0.0],
                Easing::OutCubic,
            ));
            b.push(pic);
        }
        if let Some(block) = &name_blocks[i] {
            let id = b.id(&format!("rank.{i}.name"));
            let mut l = recipes::with_ink(
                ctx,
                text_layer(id.clone(), block, ctx.palette.ink, TextAlign::Left),
                block,
            );
            l.x = x0;
            l.y = if inline {
                row_top(i) + (row_h - block.height()) / 2.0
            } else {
                row_top(i)
            };
            l.z_index = 22;
            l.depth = depth;
            b.motions
                .push(mo::fade(&id, s + 0.06, 0.35, 0.0, 1.0, Easing::OutCubic));
            b.motions.push(mo::shift(
                &id,
                s + 0.06,
                0.45,
                [-16.0 * u, 0.0],
                [0.0, 0.0],
                Easing::OutCubic,
            ));
            b.push(l);
        }
        // The value rides the bar's tip and counts up with it.
        let block = &value_blocks[i];
        let id = b.id(&format!("rank.{i}.value"));
        let color = if i == peak {
            ctx.palette.accent
        } else {
            ctx.palette.ink
        };
        let len = bar_rows[i].width * fractions.get(i).copied().unwrap_or(0.0);
        let mut l = recipes::with_ink(
            ctx,
            text_layer(id.clone(), block, color, TextAlign::Left),
            block,
        );
        l.x = area_x + len + value_gap;
        l.y = bar_rows[i].y + (bar_h - block.height()) / 2.0;
        l.z_index = 22;
        l.depth = depth;
        b.motions
            .push(mo::fade(&id, g, 0.2, 0.0, 1.0, Easing::OutCubic));
        b.motions
            .push(mo::shift(&id, g, d, [-len, 0.0], [0.0, 0.0], easing));
        if let Some(c) = mo::count(&id, g, d, &item.value, easing) {
            b.motions.push(c);
        }
        b.push(l);

        let mut words: Vec<String> = item.name.iter().cloned().collect();
        words.push(item.value.clone());
        b.reveal(&format!("rank.{i}"), words, RevealRole::Content);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Atomic number reveal
// ---------------------------------------------------------------------------

/// A countable number revealed big on an accent slab. The number counts up in
/// ENTER; its meaning line arrives at the first EVOLVE event and, for a
/// percentage, a progress bar fills at the second.
fn number_reveal(ctx: &mut Ctx, b: &mut B, carries: &mut Vec<Carry>) -> Result<(), CompileError> {
    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, ctx.margin());
    let plan = b.plan;
    let life = plan.life;
    let beat = b.beat;
    let t = plan.enter_at();
    let limit = end_limit(b);
    let ev = life.evolve_events(2);
    recipes::ghost(ctx, b, 0.86 * h);

    // A number carried into later beats becomes a SharedElement whose colour
    // persists across backgrounds: set it in ink and turn the slab into an
    // accent rule under it instead of light-on-accent type.
    let carried_forward = wants_carry(beat, Which::Primary) && !plan.is_last;
    let band = (0.0, 0.31 * h, w, 0.29 * h);
    let slab = if carried_forward {
        (m, band.1 + band.3 * 0.9, w - 2.0 * m, 12.0 * u)
    } else {
        band
    };
    let slab_id = b.id("accent_slab");
    if plan.accent_in {
        // Accent-driven transition: a bar rises to flood the frame, then
        // contracts into the slab that holds this beat's number.
        b.push(crate::compiler::rect_layer(
            slab_id.clone(),
            (0.0, h, w, 0.0),
            ctx.palette.accent,
            5,
        ));
        b.motions.push(mo::expand(
            &slab_id,
            0.0,
            0.36,
            (0.0, 0.0, w, h),
            Easing::InOutCubic,
        ));
        b.motions
            .push(mo::expand(&slab_id, 0.5, 0.75, slab, Easing::OutQuint));
    } else {
        b.push(crate::compiler::rect_layer(
            slab_id.clone(),
            slab,
            ctx.palette.accent,
            5,
        ));
        b.motions.push(mo::mask(
            &slab_id,
            t,
            0.8,
            Direction::Right,
            Easing::OutQuint,
        ));
    }
    let hero_at = if plan.accent_in { 0.62 } else { t + 0.35 };

    let percent = beat
        .primary
        .value()
        .and_then(parse_count)
        .filter(|c| c.4 == "%" && c.0 >= 0.0 && c.0 <= 100.0);

    // The meaning line (EVOLVE), above the slab.
    if let Some(meaning) = beat.primary.meaning() {
        let label = ctx
            .ts
            .fit_line(Voice::LABEL, meaning, w - 2.0 * m, 34.0 * u);
        recipes::headline_lines(
            ctx,
            b,
            "label",
            &label,
            band.1 - label.height() - 26.0 * u,
            TextAlign::Center,
            ctx.palette.ink,
            ev[0],
            false,
        );
    }

    // The hero number (with room for the progress bar when there is one).
    let (slot_h, slot_cy) = if percent.is_some() {
        (band.3 * 0.68, band.1 + band.3 * 0.44)
    } else {
        (band.3 * 0.86, band.1 + band.3 / 2.0)
    };
    let slot = Slot {
        cx: w / 2.0,
        cy: slot_cy,
        w: w * 0.84,
        h: slot_h,
    };
    let mark = b.motions.len();
    let placed = recipes::place_subject(
        ctx,
        b,
        carries,
        Which::Primary,
        &beat.primary,
        slot,
        (0.0, 0.0),
        if carried_forward {
            ctx.palette.ink
        } else {
            ctx.palette.on_accent
        },
        hero_at,
        Entrance::Pop,
        None,
        None,
        "hero",
    )?;
    if let Some(mut l) = placed {
        l.z_index = 22;
        let text = beat.primary.value().unwrap_or("");
        count_up(b, mark, Some(&l.id), text, hero_at, limit);
        b.push(l);
    }

    // A percentage also fills a progress bar along the slab (EVOLVE).
    if let Some(c) = percent {
        let style = DataStyle {
            ink: ctx.palette.on_accent,
            accent: ctx.palette.on_accent,
            muted: ctx.palette.on_accent.with_alpha(0x55),
            track: ctx.palette.on_accent.with_alpha(0x33),
            u,
        };
        let rect = BoxRect {
            x: m,
            y: band.1 + band.3 - 44.0 * u,
            width: w - 2.0 * m,
            height: 12.0 * u,
        };
        let p = paced(plan.lang.preset, ev[1], limit, 1.6);
        let mut bar = dataviz::progress_bar(
            &b.id("progress"),
            rect,
            (c.0 / 100.0) as f32,
            &style,
            ev[1],
            &p,
        );
        for l in bar.layers.iter_mut() {
            l.depth = recipes::plane(plan.lang.depth.midground);
        }
        fade_tracks_in(b, &bar.layers, ev[1]);
        recipes::absorb(b, bar, 21);
    }

    let st_top = band.1 + band.3 + 0.05 * h;
    // (0.10 Q) This closing line is set in the serif voice, not display size:
    // a statement too long to be a title is shown whole (without a voice-over;
    // it shrinks to fit three lines) or as its derived title (with one).
    let line = b.plan.display.body.as_deref().unwrap_or(&beat.statement);
    let statement = ctx
        .ts
        .fit_block(Voice::SERIF, line, w - 2.0 * m, 0.17 * h, 104.0 * u, 3);
    let start = hero_at + 0.45;
    let (sh, _) = recipes::headline_lines(
        ctx,
        b,
        "statement",
        &statement,
        st_top,
        TextAlign::Center,
        ctx.palette.ink,
        start,
        false,
    );
    recipes::underline(
        ctx,
        b,
        "statement_rule",
        (
            w / 2.0 - 60.0 * u,
            st_top + sh + 26.0 * u,
            120.0 * u,
            8.0 * u,
        ),
        ctx.palette.ink,
        start + 0.5,
    );

    // A secondary subject arrives with the second evolve step.
    if let Some(sec) = &beat.secondary {
        let slot = Slot {
            cx: w / 2.0,
            cy: st_top + sh + 0.12 * h,
            w: 0.3 * w,
            h: 0.12 * h,
        };
        if let Some(mut l) = recipes::place_subject(
            ctx,
            b,
            carries,
            Which::Secondary,
            sec,
            slot,
            (0.0, 0.0),
            ctx.palette.muted,
            ev[1],
            Entrance::Rise,
            None,
            None,
            "support",
        )? {
            l.z_index = 22;
            b.push(l);
        }
    }
    b.anchor = Some(Slot {
        cx: w / 2.0,
        cy: 0.215 * h,
        w: 0.5 * w,
        h: 84.0 * u,
    });
    Ok(())
}
