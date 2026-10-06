//! Scene lifecycle planning (0.4): where a beat's phases fall in time.
//!
//! The compiler plans one [`Lifecycle`] per beat before any composition runs;
//! compositions schedule into it (primary during ENTER, secondary information
//! at EVOLVE events, preparation during ANTICIPATE) and the plan is written to
//! `Scene.lifecycle` for validation and QA. Pure and closed-form: the same
//! inputs always give the same boundaries. See docs/SCENE_LIFECYCLE.md.

use crate::intent::Energy;
use crate::motion::language::Language;
use crate::scene::Lifecycle;

/// Everything the lifecycle depends on. All times are scene-local seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LifecycleInput {
    pub duration: f64,
    /// When the beat's own content starts entering (end of PRE_ENTER).
    pub enter_at: f64,
    /// Overlap with the next scene; 0 for the last scene.
    pub overlap_out: f64,
    pub energy: Energy,
    pub language: Language,
    /// Content density: visible units that must arrive (statement words +
    /// subject units, e.g. collection items). Longer entrances for denser beats.
    pub units: usize,
    /// Units that arrive after the primary has been read (supporting items,
    /// state changes, derived steps). More secondary units → more EVOLVE time.
    pub secondary_units: usize,
    /// (0.6) Composition-rhythm bias on the READ share: negative brings
    /// EVOLVE (secondary information, hierarchy changes) sooner. 0 = neutral.
    pub read_bias: f64,
}

/// Seconds of ENTER per language before density: how long the primary visual
/// takes to arrive.
fn enter_base(language: Language) -> f64 {
    match language {
        Language::Minimal => 0.75,
        Language::Kinetic => 0.9,
        Language::Parallax => 0.85,
        Language::Sequential => 0.8,
        Language::Data => 0.8,
    }
}

/// Seconds of SETTLE per language (hierarchy stabilizes; springs land).
fn settle_base(language: Language) -> f64 {
    match language {
        Language::Minimal => 0.25,
        Language::Kinetic => 0.3,
        Language::Parallax => 0.35,
        Language::Sequential => 0.3,
        Language::Data => 0.35,
    }
}

/// Share of the post-settle hold given to READ before EVOLVE begins. Data and
/// sequential beats unfold their reasoning, so they read less up front.
fn read_share(language: Language, secondary_units: usize) -> f64 {
    let base = match language {
        Language::Minimal => 0.5,
        Language::Kinetic => 0.42,
        Language::Parallax => 0.5,
        Language::Sequential => 0.34,
        Language::Data => 0.34,
    };
    // Every secondary unit claims some reading time for itself.
    (base - 0.04 * secondary_units.min(4) as f64).max(0.22)
}

fn energy_factor(energy: Energy) -> f64 {
    match energy {
        Energy::Calm => 1.15,
        Energy::Building => 1.0,
        Energy::Impact => 0.82,
    }
}

/// Plan the lifecycle of one scene.
///
/// Rules (all clamped so no phase is negative and every boundary stays in
/// `[0, duration]`):
/// - PRE_ENTER = `[0, enter_at)`: the previous scene's bridge overlaps it.
/// - BRIDGE = `[duration - overlap_out, duration)`: aligned with the outgoing
///   transition; empty for the last scene.
/// - ENTER = language base × energy + 0.03 s per unit (≤ 12 units), at most
///   40 % of the live span (`bridge - enter_at`).
/// - SETTLE = language base × energy, at most 12 % of the live span.
/// - ANTICIPATE = 14 % of the live span clamped to 0.25–0.6 s (× energy);
///   for the last scene a short 0.2–0.4 s resolve.
/// - The rest is split READ / EVOLVE by [`read_share`], READ ≥ 0.5 s when it fits.
pub fn plan(input: LifecycleInput) -> Lifecycle {
    let duration = input.duration.max(0.0);
    let bridge = (duration - input.overlap_out.max(0.0)).clamp(0.0, duration);
    let enter = input.enter_at.clamp(0.0, bridge);
    let live = bridge - enter;
    let e = energy_factor(input.energy);

    let enter_span =
        ((enter_base(input.language) + 0.03 * input.units.min(12) as f64) * e).min(0.4 * live);
    let settle_span = (settle_base(input.language) * e).min(0.12 * live);
    let is_last = input.overlap_out <= 0.0;
    let anticipate_span = if is_last {
        (0.08 * live).clamp(0.2, 0.4)
    } else {
        (0.14 * live * e).clamp(0.25, 0.6)
    }
    .min((live - enter_span - settle_span).max(0.0));

    let settle = enter + enter_span;
    let read = settle + settle_span;
    let anticipate = (bridge - anticipate_span).max(read);
    let hold = anticipate - read;
    let share = read_share(input.language, input.secondary_units);
    let share = if input.read_bias == 0.0 {
        share
    } else {
        (share + input.read_bias).clamp(0.18, 0.7)
    };
    let read_span = (hold * share).max(0.5f64.min(hold)).min(hold);
    let evolve = read + read_span;

    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    // Rounding is monotone, so the ordering invariant survives it.
    Lifecycle {
        enter: r(enter),
        settle: r(settle),
        read: r(read),
        evolve: r(evolve),
        anticipate: r(anticipate),
        bridge: r(bridge),
    }
}

// ---------------------------------------------------------------------------
// (0.20) Speech-aware placement: targets clamped into the plan's legal ranges
// ---------------------------------------------------------------------------

/// (0.20) READ keeps at least this long (seconds) when phases are placed from
/// speech (less only when the planned READ was already shorter).
pub const TARGET_MIN_READ: f64 = 0.5;
/// (0.20) ... and at least this share of its planned span.
pub const TARGET_READ_KEEP: f64 = 0.4;
/// (0.20) EVOLVE keeps at least this long (seconds; less only when the
/// planned EVOLVE was already shorter).
pub const TARGET_MIN_EVOLVE: f64 = 0.4;
/// (0.20) ... and at least this share of its planned span: a dense beat's
/// EVOLVE events (one item per event, a sphere's dwells) keep their room.
pub const TARGET_EVOLVE_KEEP: f64 = 0.4;

/// (0.20) Where something outside the planner (the beat's spoken words) wants
/// the phase boundaries, in scene-local seconds. `None` keeps the planned
/// boundary (the closed-form fraction).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PhaseTargets {
    pub enter: Option<f64>,
    pub read: Option<f64>,
    pub evolve: Option<f64>,
    pub anticipate: Option<f64>,
}

/// (0.20) Which boundaries [`retarget`] placed from their target: the target
/// existed and the clamped result differs from the planned boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TargetsUsed {
    pub enter: bool,
    pub read: bool,
    pub evolve: bool,
    pub anticipate: bool,
}

/// (0.20) The shortest READ and EVOLVE spans [`retarget`] keeps for a
/// planned lifecycle: `(read_min, evolve_min)` with `read_min` =
/// max([`TARGET_MIN_READ`], [`TARGET_READ_KEEP`] × planned READ span) and
/// `evolve_min` = max([`TARGET_MIN_EVOLVE`], [`TARGET_EVOLVE_KEEP`] ×
/// planned EVOLVE span), each at most its planned span.
pub fn target_minimums(planned: &Lifecycle) -> (f64, f64) {
    let read_span = (planned.evolve - planned.read).max(0.0);
    let evolve_span = (planned.anticipate - planned.evolve).max(0.0);
    (
        TARGET_MIN_READ
            .max(TARGET_READ_KEEP * read_span)
            .min(read_span),
        TARGET_MIN_EVOLVE
            .max(TARGET_EVOLVE_KEEP * evolve_span)
            .min(evolve_span),
    )
}

/// (0.20) Move a planned lifecycle's boundaries to `targets`, each clamped
/// into a legal range derived from the plan's own spans, so no phase starves:
///
/// - ENTER ∈ `[planned enter, A − evolve_min − read_min − settle − enter]`,
///   where `A` is the planned ANTICIPATE boundary and `enter` / `settle` are
///   the planned ENTER and SETTLE spans; never before the planned ENTER (the
///   previous scene's overlap).
/// - SETTLE = ENTER + the planned ENTER span (the arrival keeps its length);
///   READ ≥ SETTLE + the planned SETTLE span.
/// - EVOLVE ≥ READ + `read_min`, ANTICIPATE ≥ EVOLVE + `evolve_min`
///   ([`target_minimums`]: 0.5 s / 0.4 s or 40 % of the planned span,
///   whichever is longer, never more than planned).
/// - ANTICIPATE ≤ `A`: it may come earlier than planned, never later, so the
///   anticipation never gets shorter and never runs into the bridge.
/// - BRIDGE is unchanged.
///
/// Boundaries without a target keep their planned value, clamped into the
/// same ranges (a later ENTER can push them). Every range contains the
/// planned value, so `retarget(p, &PhaseTargets::default())` returns `p`.
/// Pure; boundaries are rounded to the millisecond and stay ordered.
pub fn retarget(planned: &Lifecycle, targets: &PhaseTargets) -> (Lifecycle, TargetsUsed) {
    let p = *planned;
    let r = |v: f64| (v * 1000.0).round() / 1000.0;
    // Clamp without panicking: an empty range collapses onto its floor, a
    // non-finite target is ignored.
    let place = |target: Option<f64>, planned: f64, lo: f64, hi: f64| -> f64 {
        let v = target.filter(|t| t.is_finite()).unwrap_or(planned);
        r(v.min(hi.max(lo)).max(lo))
    };
    let enter_span = (p.settle - p.enter).max(0.0);
    let settle_span = (p.read - p.settle).max(0.0);
    let (read_min, evolve_min) = target_minimums(&p);
    let last = p.anticipate;

    let enter = place(
        targets.enter,
        p.enter,
        p.enter,
        last - evolve_min - read_min - settle_span - enter_span,
    );
    let settle = r(enter + enter_span);
    let read = place(
        targets.read,
        p.read,
        settle + settle_span,
        last - evolve_min - read_min,
    );
    let evolve = place(targets.evolve, p.evolve, read + read_min, last - evolve_min);
    let anticipate = place(targets.anticipate, p.anticipate, evolve + evolve_min, last);

    // Rounding noise must never break the ordering invariant.
    let bridge = p.bridge;
    let anticipate = anticipate.min(bridge);
    let evolve = evolve.min(anticipate);
    let read = read.min(evolve);
    let settle = settle.min(read);
    let enter = enter.min(settle);

    let moved = |t: Option<f64>, v: f64, planned: f64| t.is_some() && (v - planned).abs() >= 5e-4;
    let used = TargetsUsed {
        enter: moved(targets.enter, enter, p.enter),
        read: moved(targets.read, read, p.read),
        evolve: moved(targets.evolve, evolve, p.evolve),
        anticipate: moved(targets.anticipate, anticipate, p.anticipate),
    };
    (
        Lifecycle {
            enter,
            settle,
            read,
            evolve,
            anticipate,
            bridge,
        },
        used,
    )
}

#[cfg(test)]
mod retarget_tests {
    use super::*;

    fn planned() -> Lifecycle {
        // A 6 s building beat: ENTER 0.9, SETTLE 0.3, READ 2.0, EVOLVE 1.65,
        // ANTICIPATE 0.6, bridge at 5.7.
        Lifecycle {
            enter: 0.25,
            settle: 1.15,
            read: 1.45,
            evolve: 3.45,
            anticipate: 5.1,
            bridge: 5.7,
        }
    }

    fn ordered(l: &Lifecycle) -> bool {
        0.0 <= l.enter
            && l.enter <= l.settle
            && l.settle <= l.read
            && l.read <= l.evolve
            && l.evolve <= l.anticipate
            && l.anticipate <= l.bridge
    }

    #[test]
    fn no_targets_is_the_plan() {
        let (l, used) = retarget(&planned(), &PhaseTargets::default());
        assert_eq!(l, planned());
        assert_eq!(used, TargetsUsed::default());
    }

    #[test]
    fn targets_inside_the_ranges_are_taken_as_is() {
        let (l, used) = retarget(
            &planned(),
            &PhaseTargets {
                enter: Some(0.48),
                read: Some(2.2),
                evolve: Some(3.4),
                anticipate: Some(4.9),
            },
        );
        assert_eq!(
            (l.enter, l.read, l.evolve, l.anticipate),
            (0.48, 2.2, 3.4, 4.9)
        );
        // The arrival keeps its planned length.
        assert!((l.settle - (0.48 + 0.9)).abs() < 1e-9);
        assert_eq!(l.bridge, 5.7);
        assert!(used.enter && used.read && used.evolve && used.anticipate);
    }

    #[test]
    fn early_targets_never_starve_a_phase() {
        let p = planned();
        // Everything said in the first second of a long beat.
        let (l, used) = retarget(
            &p,
            &PhaseTargets {
                enter: Some(0.0),
                read: Some(0.6),
                evolve: Some(0.7),
                anticipate: Some(1.0),
            },
        );
        assert!(ordered(&l), "{l:?}");
        assert_eq!(l.enter, p.enter, "never before the overlap ends");
        assert!(!used.enter, "clamped back onto the plan");
        assert!((l.settle - l.enter - (p.settle - p.enter)).abs() < 1e-9);
        assert!(l.read - l.settle >= p.read - p.settle - 1e-9);
        let read_min = 0.5f64.max(0.4 * (p.evolve - p.read));
        assert!(l.evolve - l.read >= read_min - 1e-9, "{l:?}");
        let evolve_min = 0.4f64.max(0.4 * (p.anticipate - p.evolve));
        assert!(l.anticipate - l.evolve >= evolve_min - 1e-9, "{l:?}");
        assert_eq!(target_minimums(&p), (read_min, evolve_min));
        assert!(l.anticipate <= p.anticipate);
    }

    #[test]
    fn late_targets_never_pass_the_bridge() {
        let p = planned();
        let (l, used) = retarget(
            &p,
            &PhaseTargets {
                enter: Some(9.0),
                read: Some(9.0),
                evolve: Some(9.0),
                anticipate: Some(9.0),
            },
        );
        assert!(ordered(&l), "{l:?}");
        assert_eq!(l.anticipate, p.anticipate, "never later than planned");
        assert!(!used.anticipate);
        assert!(l.anticipate - l.evolve >= 0.4 * (p.anticipate - p.evolve) - 1e-9);
        let read_min = 0.5f64.max(0.4 * (p.evolve - p.read));
        assert!(l.evolve - l.read >= read_min - 1e-9);
        assert!(l.read - l.settle >= p.read - p.settle - 1e-9);
        assert!(used.enter, "ENTER moved to its latest legal time");
    }

    #[test]
    fn a_tiny_beat_keeps_its_plan() {
        // Spans already below the minimums: the planned value is the only
        // legal one.
        let p = Lifecycle {
            enter: 0.15,
            settle: 0.45,
            read: 0.55,
            evolve: 0.75,
            anticipate: 0.9,
            bridge: 1.1,
        };
        let (l, _) = retarget(
            &p,
            &PhaseTargets {
                enter: Some(0.6),
                read: Some(0.0),
                evolve: Some(f64::NAN),
                anticipate: Some(5.0),
            },
        );
        assert_eq!(l, p);
    }
}
