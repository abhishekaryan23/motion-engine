//! State-change composition (CreativeIntent v0.2 `state_change` subject):
//! visible state A transforms into state B; two state changes in one beat
//! (primary + secondary) play in parallel and stay structurally distinct.
//!
//! One change is a row: an entity tag, a card holding the state (typeset large
//! and centered), a reduced strike-through copy of the old state that stays
//! visible after the change, and an accent tag for the meaning. Two changes are
//! two identical rows whose replaces overlap in time.
//!
//! Timing follows the scene lifecycle (docs/SCENE_LIFECYCLE.md): the panel and
//! the old state (`from`) enter in ENTER and stay readable through READ; the
//! TypeReplace from -> to fires at `life.evolve`, the reduced old state with its
//! strike at the second evolve event and the meaning tag at the third.
//!
//! (0.23) A short speech-led beat (a 1-1.5 s sentence leaves 2-3 s) cannot hold
//! that chain: it is laid out again compactly (the panel opens with the
//! headline, each step follows the one before) with the old state still held
//! 0.6 s and the new state readable 0.6 s before the exit. Beats the lifecycle
//! chain fits keep it exactly.
//!
//! (0.23 A4-short) A continuous take leaves 0.25-0.6 s between sentences, so a
//! beat can have only 1.2-2 s between its ENTER and its exit. When even the
//! compact tiers do not fit, the [`TIGHT`] tiers open the panel at ENTER with
//! ever quicker units (down to 0.12 of the preset) and share the time that is
//! left: the old state is held as long as the new state is read, both for at
//! least 0.42 s where the beat allows (1.5 s sentences with 0.25 s pauses:
//! 0.43 s and 0.44 s in every tone). A beat too short for that (a 1.0 s
//! sentence: 1.2-1.6 s in all) gets the quickest tier at its ENTER with the
//! time there is (old state 0.2-0.3 s, new state 0.30-0.36 s before the exit),
//! and the strike and the meaning tag may then begin as the beat anticipates
//! its exit. On the product path (`--variety`) the relaxed lifecycle chain is
//! also left whenever it would show the new state later than 0.6 s before the
//! exit, later than 0.1 s before ANTICIPATE, or start a step at or after
//! ANTICIPATE (editorial, technical and documentary used to clamp the new state
//! there); without `--variety` a beat keeps every layout that validated.
//!
//! Layer ids inside a row named `sc{i}` (prefixed by the beat id):
//! `sc{i}.tag`, `sc{i}.panel`, `sc{i}.from.{line}.{unit}`, `sc{i}.to.{line}.{unit}`,
//! `sc{i}.was` (reduced old state), `sc{i}.strike`, `sc{i}.meaning`,
//! `sc{i}.meaning_text`, `sc{i}.caption`; dual beats add `sc.connective`.

use super::recipes::{absorb, centered_in, with_ink, Region, B};
use super::typeset::{text_layer, Block, Voice};
use super::{base_layer, mo, round3, CompileError, Ctx};
use crate::easing::Easing;
use crate::intent::{StateChange, Subject};
use crate::motion::kinetic::{self, KineticParams, TextRun};
use crate::motion::stagger::{self, StaggerSpec};
use crate::scene::{Direction, Layer, LayerKind, Lifecycle, Stroke, TextAlign};

/// Seconds the secondary change trails the primary in a dual beat.
const DUAL_DELAY: f64 = 0.15;
/// Longest total stagger spread of a kinetic cascade (mirrors `kinetic`).
const CASCADE_SPREAD_CAP: f64 = 2.0;
/// Upper bound of the per-unit duration jitter (`1 + 0.06`) plus slack.
const DURATION_MAX_FACTOR: f64 = 1.07;
/// Pacing candidates `(duration factor, hold seconds)`, most relaxed first.
/// The hold is the minimum time the old state stays readable after it has fully
/// landed, before it is replaced.
const PACING: [(f64, f64); 5] = [
    (1.0, 0.8),
    (0.85, 0.75),
    (0.72, 0.72),
    (0.6, 0.7),
    (0.5, 0.62),
];

/// (0.23) Compact pacing for a beat too short for [`PACING`]. The relaxed chain
/// (the panel opens when the headline is done, the old state lands, is held,
/// then replaced) ends after a short speech-led beat: a sentence of 1-1.5 s
/// leaves a beat only 2-3 s long. Each tier shortens the steps between, never
/// the holds: the old state stays readable for `hold` seconds after it landed,
/// the new state for [`TO_READ`] seconds before the exit.
#[derive(Clone, Copy)]
struct Tier {
    /// Panel open to the old state's first unit.
    lead: f64,
    /// Unit duration factor.
    f: f64,
    /// Seconds the old state stays readable once landed.
    hold: f64,
    /// Replace to the reduced old state.
    was_gap: f64,
    /// Reduced old state to the meaning tag.
    meaning_gap: f64,
    /// (A4-short) Seconds the new state should stay readable before the exit
    /// ([`TO_READ`] for the [`COMPACT`] tiers).
    to_read: f64,
}

/// Most relaxed first. The first tier is the last of [`PACING`] with the panel
/// free to open earlier; the others also tighten the steps between.
const COMPACT: [Tier; 3] = [
    Tier {
        lead: 0.28,
        f: 0.5,
        hold: 0.62,
        was_gap: 0.12,
        meaning_gap: 0.3,
        to_read: TO_READ,
    },
    Tier {
        lead: 0.2,
        f: 0.42,
        hold: 0.6,
        was_gap: 0.1,
        meaning_gap: 0.24,
        to_read: TO_READ,
    },
    Tier {
        lead: 0.12,
        f: 0.35,
        hold: 0.6,
        was_gap: 0.08,
        meaning_gap: 0.2,
        to_read: TO_READ,
    },
];
/// (0.23 A4-short) For a beat the [`COMPACT`] tiers do not fit (a continuous
/// take: 1.2-2 s between ENTER and the exit): the same chain with the panel
/// opening at ENTER with its first unit and ever quicker units. A tight tier
/// has no fixed hold: the beat's time is shared so the old state and the new
/// state are readable for the same time ([`fit_timing`]); the most relaxed tier
/// whose time reaches [`TIGHT_READ`] wins. A beat too short for the last tier
/// (units of 0.12 s) gets that tier with the time there is.
const TIGHT: [Tier; 5] = [
    Tier {
        lead: 0.08,
        f: 0.3,
        hold: 0.0,
        was_gap: 0.07,
        meaning_gap: 0.18,
        to_read: TIGHT_READ,
    },
    Tier {
        lead: 0.04,
        f: 0.24,
        hold: 0.0,
        was_gap: 0.06,
        meaning_gap: 0.15,
        to_read: TIGHT_READ,
    },
    Tier {
        lead: 0.0,
        f: 0.2,
        hold: 0.0,
        was_gap: 0.05,
        meaning_gap: 0.12,
        to_read: TIGHT_READ,
    },
    Tier {
        lead: 0.0,
        f: 0.16,
        hold: 0.0,
        was_gap: 0.04,
        meaning_gap: 0.1,
        to_read: TIGHT_READ,
    },
    Tier {
        lead: 0.0,
        f: 0.12,
        hold: 0.0,
        was_gap: 0.04,
        meaning_gap: 0.08,
        to_read: TIGHT_READ,
    },
];
/// Seconds both states of a beat on a [`TIGHT`] tier should stay readable (a
/// little over the 0.4 s a state needs, for the jitter of the unit durations).
const TIGHT_READ: f64 = 0.42;
/// Seconds the new state stays readable between landing and the exit.
const TO_READ: f64 = 0.6;
/// A new state must have landed this long before ANTICIPATE for the beat's
/// value to be read (the value coverage window ends there).
const TO_BEFORE_ANTICIPATE: f64 = 0.1;
/// Ends of the reduced old state's strike and of the meaning tag, after their
/// own start (the longest motion each one owns).
const STRIKE_SPAN: f64 = 0.65;
const MEANING_SPAN: f64 = 0.5;

/// How the rows of one beat are timed.
#[derive(Clone, Copy)]
enum Timing {
    /// The lifecycle-led pacing of [`PACING`]: `(duration factor, hold)`; the
    /// panel opens when the headline is done.
    Relaxed(f64, f64),
    /// (0.23) A [`Tier`] with the panel opening at `start`.
    Compact(Tier, f64),
}

/// Compose a beat whose structure is a state change. Same contract as
/// `collection::compose`.
pub(crate) fn compose(ctx: &mut Ctx, b: &mut B, region: Region) -> Result<(), CompileError> {
    let beat = b.beat;
    let mut changes: Vec<&StateChange> = Vec::new();
    let mut other: Option<&Subject> = None;
    for s in [Some(&beat.primary), beat.secondary.as_ref()]
        .into_iter()
        .flatten()
    {
        match s {
            Subject::StateChange(c) => changes.push(c),
            o if other.is_none() => other = Some(o),
            _ => {}
        }
    }
    if changes.is_empty() {
        return Err(CompileError::Beat {
            beat: b.plan.index + 1,
            message: "state change beat has no state_change subject".into(),
        });
    }
    let dual = changes.len() > 1;
    let caption = if dual {
        None
    } else {
        other.and_then(super::subject_summary)
    };

    let (w, h, u, m) = (ctx.w, ctx.h, ctx.u, region.margin);
    let width = w - 2.0 * m;
    let has_foot = caption.is_some() || changes.iter().any(|c| c.meaning.is_some());
    let geom = Geom {
        m,
        width,
        th: 72.0 * u,
        gap1: 14.0 * u,
        foot_h: 60.0 * u,
        gap2: 16.0 * u,
        has_foot,
        ph: 0.0,
    };
    let connective_h = 84.0 * u;
    let fixed = geom.th
        + geom.gap1
        + if has_foot {
            geom.gap2 + geom.foot_h
        } else {
            0.0
        };
    let avail = (region.bottom - region.top).max(1.0);
    let row_avail = if dual {
        (avail - connective_h) / 2.0
    } else {
        avail
    };
    let cap = if dual { 0.17 * h } else { 0.24 * h };
    let ph = (row_avail - fixed).min(cap).max(48.0 * u);
    let geom = Geom { ph, ..geom };
    let row_h = fixed + ph;
    let stack = if dual {
        2.0 * row_h + connective_h
    } else {
        row_h
    };
    let y0 = region.top + (avail - stack).max(0.0) * if dual { 0.4 } else { 0.5 };

    // Pacing: the most relaxed candidate whose last motion still ends before
    // the beat's exit (dual rows are offset, so they need a little more room).
    let base = KineticParams {
        preset: b.plan.lang.preset,
        stagger: b.plan.lang.stagger,
        u,
    };
    let delay = if dual { DUAL_DELAY } else { 0.0 };
    let limit = b.plan.exit_at() - 0.05;
    let mut plan = Vec::new();
    for c in &changes {
        let block = shared_blocks(ctx, c, &geom);
        plan.push(block);
    }
    let units = plan
        .iter()
        .map(|(f, t)| (unit_count(f), unit_count(t)))
        .fold((0, 0), |a, x| (a.0.max(x.0), a.1.max(x.1)));
    let has_meaning = changes.iter().any(|c| c.meaning.is_some());
    let life = b.plan.life;
    let mut chosen = None;
    for (f, hold) in PACING {
        let s = schedule(
            &base,
            f,
            hold,
            region.start,
            units,
            has_meaning,
            &life,
            delay,
        );
        chosen = Some((f, hold));
        let last_start = if has_meaning { s.meaning_at } else { s.was_at } + delay;
        if s.end + delay <= limit && last_start < life.anticipate {
            break;
        }
    }
    let (f, hold) = chosen.unwrap_or(PACING[PACING.len() - 1]);

    let layout = Layout {
        changes: &changes,
        caption: caption.as_deref(),
        geom,
        y0,
        row_h,
        connective_h,
        delay,
        units,
        has_meaning,
        life,
        start: region.start,
    };
    let (stage_len, motion_len) = (b.stage.len(), b.motions.len());
    lay_out(ctx, b, &base, &layout, Timing::Relaxed(f, hold));
    // (0.23) The relaxed pacing stays wherever it fits the beat. When a motion
    // of it would end after the scene (a validation error: a short speech-led
    // beat), the rows are laid out again, compactly.
    // (0.23 A4-short) On the product path it also gives way when it would show
    // the new state too late to read it ([`reads_in_time`]; the clamp of
    // `recipes::clamp_to_scene` hides that in editorial, technical and
    // documentary), and a compact layout that still would not validate is
    // laid out again on the [`TIGHT`] tiers.
    let product = ctx.direction_seed.is_some();
    if overruns(b, motion_len) || (product && !reads_in_time(b, motion_len, &layout)) {
        b.stage.truncate(stage_len);
        b.motions.truncate(motion_len);
        let (enter, exit) = (b.plan.enter_at(), b.plan.exit_at());
        let timing = if product {
            fit_timing(&base, &layout, b.plan.duration, enter, exit)
        } else {
            compact_timing(&base, &layout, enter, exit)
        };
        lay_out(ctx, b, &base, &layout, timing);
        if !product && overruns(b, motion_len) {
            b.stage.truncate(stage_len);
            b.motions.truncate(motion_len);
            let timing = fit_timing(&base, &layout, b.plan.duration, enter, exit);
            lay_out(ctx, b, &base, &layout, timing);
        }
    }
    Ok(())
}

/// What `compose` decided before timing: the same for every pass.
struct Layout<'a> {
    changes: &'a [&'a StateChange],
    caption: Option<&'a str>,
    geom: Geom,
    y0: f32,
    row_h: f32,
    connective_h: f32,
    /// Seconds the second row of a dual beat trails the first.
    delay: f64,
    /// Most units of any `(from, to)` state.
    units: (usize, usize),
    has_meaning: bool,
    life: Lifecycle,
    /// Scene-local seconds when the headline has read in.
    start: f64,
}

/// Mirrors the validator's tolerance on a motion's end.
const OVERRUN_EPS: f64 = 1e-6;

/// Whether a motion added from index `first` on would end after the scene once
/// `recipes::clamp_to_scene` has run: that pass shortens a motion that starts
/// inside the scene to land on its end (so every compile it rescues stays as
/// it is), but cannot help one that starts at or after the end.
fn overruns(b: &B, first: usize) -> bool {
    let end = b.plan.duration;
    b.motions[first..].iter().any(|m| {
        // The nearest millisecond, as the clamp rounded before 0.23 A4-short:
        // a relaxed layout that this rounding pushed over the end still takes
        // the compact path it always took (every compile that validated stays).
        let duration = if m.start < end && m.start + m.duration > end {
            round3(end - m.start)
        } else {
            m.duration
        };
        m.start + duration > end + OVERRUN_EPS
    })
}

/// The compact timing for a beat the relaxed pacing does not fit: the most
/// relaxed [`Tier`] whose rows end before the exit, with the old state landing
/// and being held and the new state readable for [`TO_READ`] before it. The
/// panel opens as late as that allows (never after the headline is done, never
/// before the beat's own ENTER, `enter`). A beat too short even for the last
/// tier gets the last tier at `enter`.
fn compact_timing(base: &KineticParams, l: &Layout, enter: f64, exit_at: f64) -> Timing {
    let limit = exit_at - 0.05;
    let mut pick = Timing::Compact(COMPACT[COMPACT.len() - 1], enter);
    for tier in COMPACT {
        // Every step is relative to the panel: time the rows from 0.
        let s = schedule_compact(base, tier, 0.0, l.units, l.has_meaning);
        let latest = (limit - (s.end + l.delay))
            .min(exit_at - (s.to_end + l.delay + TO_READ))
            .min(l.start);
        if latest >= enter {
            pick = Timing::Compact(tier, latest);
            break;
        }
    }
    pick
}

/// (0.23 A4-short) Whether the rows laid out from motion `first` on read in the
/// time the beat has: the new state has landed [`TO_READ`] before the exit and
/// [`TO_BEFORE_ANTICIPATE`] before ANTICIPATE, and no step of a row starts at
/// or after ANTICIPATE (nothing new begins once the beat anticipates its exit).
fn reads_in_time(b: &B, first: usize, l: &Layout) -> bool {
    let (exit, anticipate) = (b.plan.exit_at(), l.life.anticipate);
    let mut landed = 0.0_f64;
    for m in &b.motions[first..] {
        if m.start >= anticipate - 1e-3 {
            return false;
        }
        if m.target.contains(".to.") {
            landed = landed.max(m.start + m.duration);
        }
    }
    landed + TO_READ <= exit + 1e-9 && landed + TO_BEFORE_ANTICIPATE <= anticipate + 1e-9
}

/// (0.23 A4-short) The timing for a beat the relaxed pacing does not suit.
///
/// First the most relaxed tier of [`COMPACT`] that
/// * ends every step before the exit,
/// * has the new state landed [`TO_READ`] before the exit and
///   [`TO_BEFORE_ANTICIPATE`] before ANTICIPATE, and
/// * starts no step at or after ANTICIPATE,
///
/// with the panel opening as late as that allows (never after the headline is
/// done, never before the beat's own ENTER, `enter`). Then the most relaxed of
/// [`TIGHT`]: the panel opens at `enter` and the time left is shared, the old
/// state held as long as the new state is read (steps and decorations end
/// before the scene end, nothing starts at or after ANTICIPATE); the tier fits
/// when both are readable for its `to_read`. A beat none fits that way is tried
/// again with the strike and the meaning tag free to begin as the beat
/// anticipates its exit, and finally gets the tier that reads longest.
fn fit_timing(base: &KineticParams, l: &Layout, duration: f64, enter: f64, exit_at: f64) -> Timing {
    let anticipate = l.life.anticipate;
    // The last step of a row starts this long after the panel opens (plus the
    // hold, which every step after the replace follows).
    let last_start = |s: &Schedule| {
        if l.has_meaning {
            s.meaning_at + 0.15
        } else {
            s.was_at + 0.25
        }
        .max(s.was_at + 0.25)
    };
    for tier in COMPACT {
        // Every step is relative to the panel: time the rows from 0.
        let s = schedule_compact(base, tier, 0.0, l.units, l.has_meaning);
        let latest = (exit_at - 0.05 - (s.end + l.delay))
            .min(exit_at - (s.to_end + l.delay + tier.to_read))
            .min(anticipate - TO_BEFORE_ANTICIPATE - (s.to_end + l.delay))
            .min(anticipate - 1e-3 - (last_start(&s) + l.delay))
            .min(l.start);
        if latest >= enter {
            return Timing::Compact(tier, latest);
        }
    }
    // A tight tier's panel opens at ENTER and the time left is shared between
    // the two states: the hold of the old one is as long as the new one is
    // read. `starts_before_anticipate` also asks that no step begins at or
    // after ANTICIPATE; a beat too short for that to leave both states
    // readable drops it (the strike and the meaning tag then begin as the beat
    // anticipates its exit, the states come first).
    let tight = |tier: Tier, starts_before_anticipate: bool| -> (f64, f64) {
        // The rows with no hold at all: every later step is `hold` later.
        let s = schedule_compact(
            base,
            Tier { hold: 0.0, ..tier },
            0.0,
            l.units,
            l.has_meaning,
        );
        let to_end = s.to_end + l.delay;
        // Reading time of the new state when the old one is not held: the old
        // state's own is its hold, the stagger of its units (the replace is
        // staggered alike) and the landing slack of `schedule_compact`.
        let new_read = exit_at - enter - to_end;
        let extra_old = max_offset(s.kp.stagger, l.units.0) + 0.02;
        let mut hold = ((new_read - extra_old) / 2.0)
            .min(anticipate - TO_BEFORE_ANTICIPATE - enter - to_end)
            .min(duration - 0.02 - enter - (s.end + l.delay));
        if starts_before_anticipate {
            hold = hold.min(anticipate - 1e-3 - enter - (last_start(&s) + l.delay));
        }
        let hold = hold.clamp(0.05, TO_READ);
        (hold, (hold + extra_old).min(new_read - hold))
    };
    let mut best: Option<(f64, Tier)> = None;
    for starts_before_anticipate in [true, false] {
        for tier in TIGHT {
            let (hold, read) = tight(tier, starts_before_anticipate);
            let tier = Tier { hold, ..tier };
            if read >= tier.to_read {
                return Timing::Compact(tier, enter);
            }
            if !starts_before_anticipate && best.is_none_or(|(r, _)| read >= r) {
                best = Some((read, tier));
            }
        }
    }
    let tier = best.map_or(TIGHT[TIGHT.len() - 1], |(_, t)| t);
    Timing::Compact(tier, enter)
}

/// The rows (and the connective of a dual beat) with the given timing.
fn lay_out(ctx: &Ctx, b: &mut B, base: &KineticParams, l: &Layout, timing: Timing) {
    let (w, u) = (ctx.w, ctx.u);
    let origin = match timing {
        Timing::Relaxed(..) => l.start,
        Timing::Compact(_, start) => start,
    };
    let mut connective_at = 0.0;
    for (i, (c, row_top)) in l
        .changes
        .iter()
        .zip([l.y0, l.y0 + l.row_h + l.connective_h])
        .enumerate()
    {
        let (from_block, to_block) = shared_blocks(ctx, c, &l.geom);
        let shift = i as f64 * l.delay;
        let start = origin + shift;
        let sched = match timing {
            Timing::Relaxed(f, hold) => {
                schedule(base, f, hold, start, l.units, l.has_meaning, &l.life, shift)
            }
            Timing::Compact(tier, _) => schedule_compact(base, tier, start, l.units, l.has_meaning),
        };
        if i == 1 {
            connective_at = sched.from_at;
        }
        build_row(
            ctx,
            b,
            &Row {
                name: format!("sc{i}"),
                change: c,
                caption: l.caption,
                top: row_top,
                from_block,
                to_block,
            },
            &l.geom,
            &sched,
            start,
        );
    }

    if l.changes.len() > 1 {
        let block = ctx.ts.fit_line(
            Voice::SERIF,
            "at the same time",
            0.6 * l.geom.width,
            (0.5 * l.connective_h).min(40.0 * u),
        );
        let id = b.id("sc.connective");
        let mut layer = text_layer(id.clone(), &block, ctx.palette.muted, TextAlign::Left);
        layer.x = (w - layer.width) / 2.0;
        layer.y = l.y0 + l.row_h + (l.connective_h - block.height()) / 2.0;
        layer.z_index = 22;
        // Arrives with the second row's old state (ENTER), before anything changes.
        let at = connective_at.max(origin);
        b.motions
            .push(mo::fade(&id, at, 0.4, 0.0, 1.0, Easing::OutCubic));
        b.motions.push(mo::shift(
            &id,
            at,
            0.5,
            [0.0, 10.0 * u],
            [0.0, 0.0],
            Easing::OutCubic,
        ));
        b.push(layer);
    }
}

/// Shared vertical/horizontal geometry of a row.
#[derive(Clone, Copy)]
struct Geom {
    m: f32,
    width: f32,
    /// Tag row height (entity tag left, reduced old state right).
    th: f32,
    gap1: f32,
    foot_h: f32,
    gap2: f32,
    has_foot: bool,
    /// Panel height.
    ph: f32,
}

struct Row<'a> {
    name: String,
    change: &'a StateChange,
    caption: Option<&'a str>,
    top: f32,
    from_block: Block,
    to_block: Block,
}

/// Absolute (scene-local) timing of one row.
struct Schedule {
    kp: KineticParams,
    from_at: f64,
    at: f64,
    was_at: f64,
    meaning_at: f64,
    /// When the new state has fully arrived (an upper bound).
    to_end: f64,
    end: f64,
}

fn unit_count(block: &Block) -> usize {
    block
        .lines
        .iter()
        .map(|l| l.split_whitespace().count())
        .sum()
}

/// Largest stagger offset of a cascade over `n` units (spread is capped).
fn max_offset(spec: StaggerSpec, n: usize) -> f64 {
    stagger::offsets(spec, n)
        .into_iter()
        .fold(0.0, f64::max)
        .min(CASCADE_SPREAD_CAP)
}

/// Timing of one row starting at `start` (panel opens), with unit durations
/// scaled by `f`. `units` = (from units, to units). The replace fires at
/// `life.evolve` (or, when the old state lands late, once it has been readable
/// for `hold`); the reduced old state and the meaning follow at the next
/// evolve events. `shift` delays the row's evolve steps (dual beats).
#[allow(clippy::too_many_arguments)]
fn schedule(
    base: &KineticParams,
    f: f64,
    hold: f64,
    start: f64,
    units: (usize, usize),
    has_meaning: bool,
    life: &Lifecycle,
    shift: f64,
) -> Schedule {
    let mut kp = *base;
    let dur = base.preset.duration * f;
    kp.preset.duration = dur;
    let from_at = start + 0.28;
    let landed = from_at + max_offset(kp.stagger, units.0) + dur * DURATION_MAX_FACTOR + 0.02;
    // Without a meaning label there are two evolve steps, not three: no empty slot.
    let events = life.evolve_events(if has_meaning { 3 } else { 2 });
    let at = (events[0] + shift).max(landed + hold);
    let arrive = at + dur * 0.35;
    let to_off = max_offset(kp.stagger, units.1);
    let to_end = arrive + to_off + dur * DURATION_MAX_FACTOR;
    let old_end = at + max_offset(kp.stagger, units.0) + 0.1 + dur * DURATION_MAX_FACTOR;
    let was_at = (events[1] + shift).max(at + 0.12);
    let meaning_at = (events.get(2).copied().unwrap_or(life.anticipate) + shift).max(was_at + 0.3);
    let mut end = to_end.max(old_end).max(was_at + 0.7);
    if has_meaning {
        end = end.max(meaning_at + 0.62);
    }
    Schedule {
        kp,
        from_at,
        at,
        was_at,
        meaning_at,
        to_end,
        end,
    }
}

/// (0.23) Timing of one row under a compact [`Tier`], starting at `start`
/// (panel opens). Unlike [`schedule`] it does not wait for the lifecycle's
/// evolve events: a beat this short has none to spare. The steps follow in
/// order, each as soon as the one before allows: the old state lands, is held
/// for `tier.hold`, is replaced; the reduced old state and the meaning follow.
fn schedule_compact(
    base: &KineticParams,
    tier: Tier,
    start: f64,
    units: (usize, usize),
    has_meaning: bool,
) -> Schedule {
    let mut kp = *base;
    let dur = base.preset.duration * tier.f;
    kp.preset.duration = dur;
    let from_at = start + tier.lead;
    let landed = from_at + max_offset(kp.stagger, units.0) + dur * DURATION_MAX_FACTOR + 0.02;
    let at = landed + tier.hold;
    let arrive = at + dur * 0.35;
    let to_end = arrive + max_offset(kp.stagger, units.1) + dur * DURATION_MAX_FACTOR;
    let old_end = at + max_offset(kp.stagger, units.0) + 0.1 + dur * DURATION_MAX_FACTOR;
    let was_at = at + tier.was_gap;
    let meaning_at = was_at + tier.meaning_gap;
    let mut end = to_end.max(old_end).max(was_at + STRIKE_SPAN);
    if has_meaning {
        end = end.max(meaning_at + MEANING_SPAN);
    }
    Schedule {
        kp,
        from_at,
        at,
        was_at,
        meaning_at,
        to_end,
        end,
    }
}

/// `from` and `to` typeset at one shared size inside the panel, so the
/// replace swaps like for like.
fn shared_blocks(ctx: &Ctx, c: &StateChange, geom: &Geom) -> (Block, Block) {
    let u = ctx.u;
    let inner_w = geom.width - 80.0 * u;
    let inner_h = geom.ph * 0.62;
    let a = ctx
        .ts
        .fit_block(Voice::HEADLINE, &c.from, inner_w, inner_h, 150.0 * u, 2);
    let b = ctx
        .ts
        .fit_block(Voice::HEADLINE, &c.to, inner_w, inner_h, 150.0 * u, 2);
    let size = a.size.min(b.size);
    (
        ctx.ts
            .set_wrapped(Voice::HEADLINE, &c.from, size, inner_w * 1.001),
        ctx.ts
            .set_wrapped(Voice::HEADLINE, &c.to, size, inner_w * 1.001),
    )
}

/// A text run centered on `(cx, cy)`: every line centered horizontally, the
/// block's ink centered vertically. Runs are unit layers (no layout binding),
/// so centering is computed from measured widths.
fn centered_run(
    ctx: &Ctx,
    id: String,
    block: &Block,
    color: crate::scene::Color,
    cx: f32,
    cy: f32,
) -> TextRun {
    let bw = block.width();
    let mut run = ctx
        .ts
        .text_run(id, block, color, [cx - bw / 2.0, 0.0], false);
    for (line, lw) in run.lines.iter_mut().zip(&block.line_widths) {
        let dx = (bw - lw) / 2.0;
        for unit in line.iter_mut() {
            unit.x += dx;
        }
    }
    let adv = run.line_advance;
    let n = run.lines.len() as f32;
    let ink_mid = run
        .ink
        .map(|i| (i.top + i.bottom) * 0.5)
        .unwrap_or(adv * 0.5);
    run.origin[1] = cy - (n - 1.0) * adv * 0.5 - ink_mid;
    run
}

fn text_at(id: String, block: &Block, color: crate::scene::Color, x: f32, y: f32, z: i32) -> Layer {
    let mut layer = text_layer(id, block, color, TextAlign::Left);
    layer.x = x;
    layer.y = y;
    layer.z_index = z;
    layer
}

fn build_row(ctx: &Ctx, b: &mut B, row: &Row, g: &Geom, s: &Schedule, start: f64) {
    let u = ctx.u;
    let pal = ctx.palette.clone();
    let name = row.name.as_str();
    let c = row.change;
    let panel_y = row.top + g.th + g.gap1;

    // Panel (the card the state lives in).
    let panel_id = b.id(&format!("{name}.panel"));
    b.push(base_layer(
        panel_id.clone(),
        (g.m, panel_y, g.width, g.ph),
        LayerKind::RoundedRectangle {
            fill: pal.card,
            radius: 14.0 * u,
            stroke: Some(Stroke {
                color: pal.ink,
                width: 3.0 * u,
            }),
        },
        12,
    ));
    b.motions.push(mo::mask(
        &panel_id,
        start,
        0.7,
        Direction::Down,
        Easing::OutQuint,
    ));

    // Entity tag.
    let tag = ctx
        .ts
        .fit_line(Voice::LABEL, &c.entity, 0.5 * g.width, 32.0 * u);
    let tag_id = b.id(&format!("{name}.tag"));
    b.push(text_at(
        tag_id.clone(),
        &tag,
        pal.ink,
        g.m,
        row.top + (g.th - tag.height()) / 2.0,
        22,
    ));
    b.motions
        .push(mo::fade(&tag_id, start, 0.5, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(mo::shift(
        &tag_id,
        start,
        0.6,
        [0.0, 12.0 * u],
        [0.0, 0.0],
        Easing::OutCubic,
    ));

    // The state itself: from is read first, then TypeReplace turns it into to.
    let cx = g.m + g.width / 2.0;
    let cy = panel_y + g.ph / 2.0;
    let from_run = centered_run(
        ctx,
        b.id(&format!("{name}.from")),
        &row.from_block,
        pal.ink,
        cx,
        cy,
    );
    let to_run = centered_run(
        ctx,
        b.id(&format!("{name}.to")),
        &row.to_block,
        pal.ink,
        cx,
        cy,
    );
    let entrance = kinetic::word_cascade(&from_run, s.from_at, &s.kp);
    let replace = kinetic::type_replace(&from_run, &to_run, s.at, &s.kp);
    absorb(b, replace, 20);
    // Only the entrance motions: the layers come from the replace expansion.
    b.motions.extend(entrance.motions);

    // Reduced old state with an accent strike, right of the tag.
    let was = ctx
        .ts
        .fit_line(Voice::HEADLINE, &c.from, 0.42 * g.width, 52.0 * u);
    let was_id = b.id(&format!("{name}.was"));
    let mut was_layer = text_at(was_id.clone(), &was, pal.muted, 0.0, 0.0, 24);
    was_layer.x = g.m + g.width - was_layer.width;
    was_layer.y = row.top + (g.th - was.height()) / 2.0;
    let ink_mid = ctx
        .ts
        .ink(&was)
        .map(|i| (i.top + i.bottom) * 0.5)
        .unwrap_or(was.height() * 0.5);
    let rule_h = 5.0 * u;
    let rule_x = was_layer.x - 6.0 * u;
    let rule_w = was.width() + 12.0 * u;
    let rule_y = was_layer.y + ink_mid - rule_h / 2.0;
    b.motions
        .push(mo::fade(&was_id, s.was_at, 0.4, 0.0, 1.0, Easing::OutCubic));
    b.motions.push(mo::shift(
        &was_id,
        s.was_at,
        0.5,
        [0.0, 16.0 * u],
        [0.0, 0.0],
        Easing::OutCubic,
    ));
    b.push(was_layer);
    let strike_id = b.id(&format!("{name}.strike"));
    b.push(base_layer(
        strike_id.clone(),
        (rule_x, rule_y, rule_w, rule_h),
        LayerKind::Rectangle {
            fill: pal.accent,
            stroke: None,
        },
        26,
    ));
    b.motions.push(mo::mask(
        &strike_id,
        s.was_at + 0.25,
        0.4,
        Direction::Right,
        Easing::OutQuint,
    ));

    // Footer: meaning tag (lands last) on the left, caption on the right.
    if !g.has_foot {
        return;
    }
    let foot_y = panel_y + g.ph + g.gap2;
    if let Some(meaning) = c.meaning.as_deref() {
        let pad = 24.0 * u;
        let block = ctx
            .ts
            .fit_line(Voice::LABEL, meaning, 0.5 * g.width - 2.0 * pad, 28.0 * u);
        let pill_w = block.width() * 1.02 + 2.0 + 2.0 * pad;
        let pill_h = block.height() + 22.0 * u;
        let pill_id = b.id(&format!("{name}.meaning"));
        b.push(base_layer(
            pill_id.clone(),
            (g.m, foot_y + (g.foot_h - pill_h) / 2.0, pill_w, pill_h),
            LayerKind::RoundedRectangle {
                fill: pal.accent,
                radius: pill_h / 2.0,
                stroke: None,
            },
            24,
        ));
        b.motions.push(mo::mask(
            &pill_id,
            s.meaning_at,
            0.45,
            Direction::Right,
            Easing::OutQuint,
        ));
        let text_id = b.id(&format!("{name}.meaning_text"));
        let mut text = with_ink(
            ctx,
            text_layer(text_id.clone(), &block, pal.on_accent, TextAlign::Center),
            &block,
        );
        text.anchor_x = 0.5;
        text.anchor_y = 0.5;
        text.z_index = 26;
        text.layout = Some(centered_in(&pill_id, [0.0, 0.0]));
        b.motions.push(mo::fade(
            &text_id,
            s.meaning_at + 0.15,
            0.35,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        b.push(text);
    }
    if let Some(cap) = row.caption {
        let block = ctx.ts.fit_line(Voice::SERIF, cap, 0.4 * g.width, 34.0 * u);
        let id = b.id(&format!("{name}.caption"));
        let mut layer = text_at(id.clone(), &block, pal.muted, 0.0, 0.0, 22);
        layer.x = g.m + g.width - layer.width;
        layer.y = foot_y + (g.foot_h - block.height()) / 2.0;
        b.motions.push(mo::fade(
            &id,
            s.from_at + 0.2,
            0.5,
            0.0,
            1.0,
            Easing::OutCubic,
        ));
        b.push(layer);
    }
}
