//! Scene lifecycle (0.4): the planner's invariants across a parameter matrix,
//! the `Lifecycle` helpers, and lifecycles as written by the compiler.

use std::path::PathBuf;

use motion_core::intent::Energy;
use motion_core::motion::language::Language;
use motion_core::motion::lifecycle::{plan, LifecycleInput};
use motion_core::scene::{Lifecycle, MotionProject, Phase};
use motion_core::{compile, validate, ApproxMeasure, AssetLibrary, CreativeIntent, StyleProfile};

const TOL: f64 = 1e-3;

const DURATIONS: [f64; 3] = [3.2, 5.0, 7.5];
const ENERGIES: [Energy; 3] = [Energy::Calm, Energy::Building, Energy::Impact];
const LANGUAGES: [Language; 5] = [
    Language::Minimal,
    Language::Kinetic,
    Language::Parallax,
    Language::Sequential,
    Language::Data,
];
const OVERLAPS: [f64; 2] = [0.0, 0.55];
const ENTER_ATS: [f64; 2] = [0.25, 0.42];
const UNITS: [usize; 4] = [3, 8, 12, 20];
const SECONDARY: [usize; 4] = [0, 1, 3, 5];

fn input(
    duration: f64,
    energy: Energy,
    language: Language,
    overlap_out: f64,
    enter_at: f64,
    units: usize,
    secondary_units: usize,
) -> LifecycleInput {
    LifecycleInput {
        duration,
        enter_at,
        overlap_out,
        energy,
        language,
        units,
        secondary_units,
        read_bias: 0.0,
    }
}

/// Every input in the matrix, in a fixed order.
fn matrix() -> Vec<LifecycleInput> {
    let mut all = Vec::new();
    for &d in &DURATIONS {
        for &e in &ENERGIES {
            for &l in &LANGUAGES {
                for &o in &OVERLAPS {
                    for &at in &ENTER_ATS {
                        for &u in &UNITS {
                            for &s in &SECONDARY {
                                all.push(input(d, e, l, o, at, u, s));
                            }
                        }
                    }
                }
            }
        }
    }
    all
}

fn ordered(l: &Lifecycle, duration: f64) -> bool {
    let s = l.starts();
    s[0] >= 0.0 && s.windows(2).all(|w| w[0] <= w[1]) && l.bridge <= duration + 1e-9
}

// ---------------------------------------------------------------------------
// Planner: invariants over the matrix
// ---------------------------------------------------------------------------

#[test]
fn planner_ordering_invariant_holds_across_matrix() {
    for i in matrix() {
        let l = plan(i);
        assert!(
            ordered(&l, i.duration),
            "ordering violated for {i:?}: {l:?}"
        );
        // No phase has negative length.
        for p in Phase::ALL {
            let (a, b) = l.range(p, i.duration);
            assert!(b - a >= -1e-9, "negative {p:?} in {i:?}: {l:?}");
        }
    }
}

#[test]
fn planner_bridge_aligns_with_transition() {
    for i in matrix() {
        let l = plan(i);
        assert!(
            (l.bridge - (i.duration - i.overlap_out)).abs() <= TOL,
            "bridge {} != duration {} - overlap {} for {i:?}",
            l.bridge,
            i.duration,
            i.overlap_out
        );
        if i.overlap_out == 0.0 {
            assert!(
                (l.bridge - i.duration).abs() <= TOL,
                "last scene bridge must equal duration: {l:?}"
            );
        }
    }
}

#[test]
fn planner_enter_starts_at_enter_at() {
    for i in matrix() {
        let l = plan(i);
        assert!(
            (l.enter - i.enter_at).abs() <= TOL,
            "PRE_ENTER must end at enter_at for {i:?}: {l:?}"
        );
    }
}

#[test]
fn planner_is_deterministic() {
    for i in matrix() {
        assert_eq!(plan(i), plan(i), "non-deterministic for {i:?}");
    }
}

#[test]
fn planner_enter_is_non_empty_when_live_span_positive() {
    for i in matrix() {
        let l = plan(i);
        let live = l.bridge - l.enter;
        assert!(live > 0.0, "matrix inputs always have a live span");
        assert!(
            l.settle - l.enter > 0.0,
            "ENTER empty despite live span {live} for {i:?}: {l:?}"
        );
    }
}

#[test]
fn planner_read_is_at_least_half_a_second_when_hold_fits() {
    for i in matrix() {
        let l = plan(i);
        let hold = l.anticipate - l.read;
        if hold >= 0.5 {
            assert!(
                l.evolve - l.read >= 0.5 - 2.0 * TOL,
                "READ shorter than 0.5s with hold {hold} for {i:?}: {l:?}"
            );
        } else {
            // When it does not fit, READ takes the whole hold and EVOLVE is empty.
            assert!(l.evolve - l.read <= hold + 1e-9);
        }
    }
}

#[test]
fn planner_evolve_is_non_empty_for_long_scenes() {
    for i in matrix().into_iter().filter(|i| i.duration >= 7.5) {
        let l = plan(i);
        assert!(
            l.anticipate - l.evolve > 0.1,
            "EVOLVE empty in a long scene for {i:?}: {l:?}"
        );
    }
}

#[test]
fn planner_impact_enters_no_slower_than_calm_and_faster_when_uncapped() {
    // Across the matrix: never slower. (When both are clamped by the 40 % live
    // span cap they can tie, so strictness is checked below where uncapped.)
    for &d in &DURATIONS {
        for &l in &LANGUAGES {
            for &o in &OVERLAPS {
                for &at in &ENTER_ATS {
                    for &u in &UNITS {
                        let calm = plan(input(d, Energy::Calm, l, o, at, u, 1));
                        let impact = plan(input(d, Energy::Impact, l, o, at, u, 1));
                        let calm_enter = calm.settle - calm.enter;
                        let impact_enter = impact.settle - impact.enter;
                        assert!(
                            impact_enter <= calm_enter + 1e-9,
                            "impact ENTER {impact_enter} longer than calm {calm_enter} \
                             ({d}s {l:?} overlap {o} enter_at {at} units {u})"
                        );
                        let live = calm.bridge - calm.enter;
                        if calm_enter < 0.4 * live - TOL {
                            assert!(
                                impact_enter < calm_enter,
                                "impact ENTER {impact_enter} not shorter than uncapped calm \
                                 {calm_enter} ({d}s {l:?} overlap {o} enter_at {at} units {u})"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn planner_impact_enter_strictly_shorter_for_a_roomy_scene() {
    for &l in &LANGUAGES {
        let calm = plan(input(7.5, Energy::Calm, l, 0.55, 0.25, 3, 1));
        let impact = plan(input(7.5, Energy::Impact, l, 0.55, 0.25, 3, 1));
        assert!(
            impact.settle - impact.enter < calm.settle - calm.enter,
            "{l:?}: impact {impact:?} vs calm {calm:?}"
        );
    }
}

#[test]
fn planner_more_secondary_units_start_evolve_earlier_or_equal() {
    for &d in &DURATIONS {
        for &e in &ENERGIES {
            for &l in &LANGUAGES {
                for &o in &OVERLAPS {
                    let mut prev: Option<Lifecycle> = None;
                    for s in 0..=5usize {
                        let cur = plan(input(d, e, l, o, 0.25, 8, s));
                        if let Some(p) = prev {
                            assert!(
                                cur.evolve <= p.evolve + 1e-9,
                                "secondary {s} started EVOLVE later ({} > {}) for {d}s {e:?} {l:?} overlap {o}",
                                cur.evolve,
                                p.evolve
                            );
                            // Secondary count only moves the READ/EVOLVE split.
                            assert_eq!(cur.read, p.read);
                            assert_eq!(cur.anticipate, p.anticipate);
                            assert_eq!(cur.bridge, p.bridge);
                        }
                        prev = Some(cur);
                    }
                }
            }
        }
    }
}

#[test]
fn planner_more_secondary_units_give_strictly_more_evolve_in_a_long_scene() {
    let few = plan(input(
        7.5,
        Energy::Building,
        Language::Sequential,
        0.55,
        0.25,
        8,
        0,
    ));
    let many = plan(input(
        7.5,
        Energy::Building,
        Language::Sequential,
        0.55,
        0.25,
        8,
        4,
    ));
    assert!(many.evolve < few.evolve, "{many:?} vs {few:?}");
}

#[test]
fn planner_degenerate_inputs_still_ordered() {
    let cases = [
        input(0.0, Energy::Building, Language::Minimal, 0.0, 0.0, 0, 0),
        input(0.3, Energy::Impact, Language::Kinetic, 0.55, 0.42, 20, 5),
        input(1.0, Energy::Calm, Language::Data, 2.0, 0.42, 3, 0),
        input(2.0, Energy::Calm, Language::Parallax, 0.0, 5.0, 3, 0),
    ];
    for i in cases {
        let l = plan(i);
        assert!(ordered(&l, i.duration.max(0.0)), "{i:?} -> {l:?}");
    }
}

// ---------------------------------------------------------------------------
// Lifecycle helpers
// ---------------------------------------------------------------------------

fn sample() -> Lifecycle {
    Lifecycle {
        enter: 0.5,
        settle: 1.0,
        read: 1.5,
        evolve: 3.0,
        anticipate: 4.0,
        bridge: 4.5,
    }
}

#[test]
fn range_returns_half_open_phase_bounds_and_tiles_the_scene() {
    let l = sample();
    let d = 5.0;
    assert_eq!(l.range(Phase::PreEnter, d), (0.0, 0.5));
    assert_eq!(l.range(Phase::Enter, d), (0.5, 1.0));
    assert_eq!(l.range(Phase::Settle, d), (1.0, 1.5));
    assert_eq!(l.range(Phase::Read, d), (1.5, 3.0));
    assert_eq!(l.range(Phase::Evolve, d), (3.0, 4.0));
    assert_eq!(l.range(Phase::Anticipate, d), (4.0, 4.5));
    assert_eq!(l.range(Phase::Bridge, d), (4.5, 5.0));

    // Consecutive phases share a boundary; together they cover [0, duration].
    let ranges: Vec<(f64, f64)> = Phase::ALL.iter().map(|p| l.range(*p, d)).collect();
    assert_eq!(ranges[0].0, 0.0);
    assert_eq!(ranges[6].1, d);
    for w in ranges.windows(2) {
        assert_eq!(w[0].1, w[1].0);
    }
}

#[test]
fn phase_at_finds_the_containing_phase() {
    let l = sample();
    assert_eq!(l.phase_at(0.0), Phase::PreEnter);
    assert_eq!(l.phase_at(0.49), Phase::PreEnter);
    assert_eq!(l.phase_at(0.5), Phase::Enter);
    assert_eq!(l.phase_at(0.99), Phase::Enter);
    assert_eq!(l.phase_at(1.0), Phase::Settle);
    assert_eq!(l.phase_at(1.5), Phase::Read);
    assert_eq!(l.phase_at(2.9), Phase::Read);
    assert_eq!(l.phase_at(3.0), Phase::Evolve);
    assert_eq!(l.phase_at(4.0), Phase::Anticipate);
    assert_eq!(l.phase_at(4.5), Phase::Bridge);
    assert_eq!(l.phase_at(4.999), Phase::Bridge);
    // Clamped into the scene.
    assert_eq!(l.phase_at(-1.0), Phase::PreEnter);
    assert_eq!(l.phase_at(99.0), Phase::Bridge);
}

#[test]
fn phase_at_agrees_with_range() {
    let l = sample();
    for p in Phase::ALL {
        let (a, b) = l.range(p, 5.0);
        assert_eq!(l.phase_at(a), p, "start of {p:?}");
        assert_eq!(l.phase_at((a + b) * 0.5), p, "middle of {p:?}");
    }
}

#[test]
fn phase_names_are_stable() {
    let names: Vec<&str> = Phase::ALL.iter().map(|p| p.name()).collect();
    assert_eq!(
        names,
        [
            "PRE_ENTER",
            "ENTER",
            "SETTLE",
            "READ",
            "EVOLVE",
            "ANTICIPATE",
            "BRIDGE"
        ]
    );
}

#[test]
fn spread_places_first_at_from_and_leaves_a_final_gap() {
    assert!(Lifecycle::spread(1.0, 3.0, 0).is_empty());
    assert_eq!(Lifecycle::spread(1.0, 3.0, 1), vec![1.0]);
    assert_eq!(Lifecycle::spread(1.0, 3.0, 4), vec![1.0, 1.5, 2.0, 2.5]);
    // Reversed / empty range collapses onto `from` instead of going backwards.
    assert_eq!(Lifecycle::spread(2.0, 1.0, 3), vec![2.0, 2.0, 2.0]);
    for n in 1..8 {
        let v = Lifecycle::spread(0.5, 4.5, n);
        assert_eq!(v.len(), n);
        assert_eq!(v[0], 0.5);
        assert!(v.windows(2).all(|w| w[0] < w[1]));
        assert!(v.iter().all(|t| *t < 4.5));
    }
}

#[test]
fn evolve_events_lie_inside_evolve_and_increase() {
    let l = sample();
    assert!(l.evolve_events(0).is_empty());
    for n in 1..=6 {
        let ev = l.evolve_events(n);
        assert_eq!(ev.len(), n);
        assert_eq!(ev[0], l.evolve, "first event at evolve");
        assert!(ev.windows(2).all(|w| w[0] < w[1]), "strictly increasing");
        assert!(
            ev.iter().all(|t| *t >= l.evolve && *t < l.anticipate),
            "{ev:?} outside [{}, {})",
            l.evolve,
            l.anticipate
        );
        assert!(ev.iter().all(|t| l.phase_at(*t) == Phase::Evolve));
    }
}

#[test]
fn evolve_events_of_planned_lifecycles_stay_in_evolve() {
    for i in matrix() {
        let l = plan(i);
        if l.anticipate - l.evolve <= 0.0 {
            continue;
        }
        for n in 0..=5 {
            let ev = l.evolve_events(n);
            assert_eq!(ev.len(), n);
            if n > 0 {
                assert_eq!(ev[0], l.evolve);
            }
            assert!(ev.windows(2).all(|w| w[0] < w[1]), "{i:?}");
            assert!(
                ev.iter().all(|t| *t >= l.evolve && *t < l.anticipate),
                "{i:?}: {ev:?} vs {l:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Compiler output
// ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn compile_example(name: &str) -> MotionProject {
    let path = repo().join("examples/public").join(name);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let intent: CreativeIntent =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    compile(
        &intent,
        &StyleProfile::default(),
        &AssetLibrary::new(repo().join("assets")),
        &ApproxMeasure,
    )
    .unwrap_or_else(|e| panic!("compile {name}: {e}"))
}

const EXAMPLES: [&str; 4] = [
    "three-beat-story.intent.json",
    "collection-accumulate.intent.json",
    "state-change.intent.json",
    "derived-metric.intent.json",
];

#[test]
fn compiled_beats_carry_a_valid_lifecycle() {
    for name in EXAMPLES {
        let project = compile_example(name);
        let mut beats = 0;
        for s in &project.scenes {
            if s.id.starts_with("beat_") {
                beats += 1;
                let life = s
                    .lifecycle
                    .unwrap_or_else(|| panic!("{name}: scene {} has no lifecycle", s.id));
                assert!(
                    ordered(&life, s.duration_seconds),
                    "{name}/{}: {life:?} in {}s",
                    s.id,
                    s.duration_seconds
                );
                assert!(life.settle > life.enter, "{name}/{}: empty ENTER", s.id);
            } else {
                assert!(
                    s.lifecycle.is_none(),
                    "{name}: non-beat scene {} must not carry a lifecycle",
                    s.id
                );
            }
        }
        assert!(beats > 0, "{name}: no beat scenes");
        assert!(
            project.scenes.iter().any(|s| s.id == "backdrop"),
            "{name}: expected a backdrop scene"
        );
        let backdrop = project.scenes.iter().find(|s| s.id == "backdrop");
        assert!(backdrop.is_some_and(|s| s.lifecycle.is_none()));
    }
}

#[test]
fn compiled_bridge_aligns_with_the_next_scene_start() {
    for name in EXAMPLES {
        let project = compile_example(name);
        let beats: Vec<_> = project
            .scenes
            .iter()
            .filter(|s| s.id.starts_with("beat_"))
            .collect();
        for (i, s) in beats.iter().enumerate() {
            let life = s.lifecycle.expect("beat lifecycle");
            match beats.get(i + 1) {
                Some(next) => assert!(
                    (s.start_seconds + life.bridge - next.start_seconds).abs() <= TOL,
                    "{name}: {} bridge at {} but {} starts at {}",
                    s.id,
                    s.start_seconds + life.bridge,
                    next.id,
                    next.start_seconds
                ),
                None => assert!(
                    (life.bridge - s.duration_seconds).abs() <= TOL,
                    "{name}: last beat {} bridge {} != duration {}",
                    s.id,
                    life.bridge,
                    s.duration_seconds
                ),
            }
        }
    }
}

#[test]
fn compiled_projects_validate() {
    for name in EXAMPLES {
        let project = compile_example(name);
        if let Err(e) = validate(&project, None) {
            panic!("{name} failed validation:\n{e}");
        }
    }
}

#[test]
fn compiling_twice_is_byte_identical() {
    for name in EXAMPLES {
        let a = compile_example(name).to_json_pretty();
        let b = compile_example(name).to_json_pretty();
        assert_eq!(a, b, "{name}: compile is not deterministic");
        assert!(a.contains("\"lifecycle\""), "{name}: lifecycle not emitted");
    }
}
