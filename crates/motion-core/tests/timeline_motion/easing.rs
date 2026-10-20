//! Integration tests for easing curves and motion presets.

use motion_core::easing::{Easing, MotionPreset};

const ALL: [Easing; 7] = [
    Easing::Linear,
    Easing::InCubic,
    Easing::OutCubic,
    Easing::InOutCubic,
    Easing::OutQuint,
    Easing::EditorialSpring,
    Easing::ImpactSpring,
];

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn max_over_samples(e: Easing) -> f64 {
    (0..=200)
        .map(|i| e.apply(i as f64 / 200.0))
        .fold(f64::MIN, f64::max)
}

#[test]
fn endpoints_are_exact_for_every_variant() {
    for e in ALL {
        assert!(
            approx(e.apply(0.0), 0.0),
            "{e:?}: apply(0) = {}",
            e.apply(0.0)
        );
        assert!(
            approx(e.apply(1.0), 1.0),
            "{e:?}: apply(1) = {}",
            e.apply(1.0)
        );
    }
}

#[test]
fn input_is_clamped_to_unit_range() {
    for e in ALL {
        assert_eq!(e.apply(-1.0), e.apply(0.0), "{e:?} below 0");
        assert_eq!(e.apply(2.0), e.apply(1.0), "{e:?} above 1");
        assert_eq!(e.apply(f64::NEG_INFINITY), e.apply(0.0), "{e:?} -inf");
        assert_eq!(e.apply(f64::INFINITY), e.apply(1.0), "{e:?} +inf");
    }
}

#[test]
fn nan_maps_to_zero() {
    for e in ALL {
        let v = e.apply(f64::NAN);
        assert!(!v.is_nan(), "{e:?} returned NaN");
        assert!(approx(v, 0.0), "{e:?}: NaN -> {v}");
    }
}

#[test]
fn every_curve_is_finite_across_the_range() {
    for e in ALL {
        for i in 0..=1000 {
            let v = e.apply(i as f64 / 1000.0);
            assert!(v.is_finite(), "{e:?} at {i}/1000 -> {v}");
        }
    }
}

#[test]
fn known_midpoint_values() {
    assert!(approx(Easing::Linear.apply(0.5), 0.5));
    assert!(approx(Easing::InCubic.apply(0.5), 0.125));
    assert!(approx(Easing::OutCubic.apply(0.5), 0.875));
    assert!(approx(Easing::InOutCubic.apply(0.5), 0.5));
    assert!(approx(Easing::OutQuint.apply(0.5), 0.96875));
}

#[test]
fn in_out_cubic_is_symmetric() {
    for i in 0..=20 {
        let t = i as f64 / 20.0;
        let sum = Easing::InOutCubic.apply(t) + Easing::InOutCubic.apply(1.0 - t);
        assert!(approx(sum, 1.0), "t={t}: sum {sum}");
    }
}

#[test]
fn non_spring_curves_are_monotonic_and_bounded() {
    for e in [
        Easing::Linear,
        Easing::InCubic,
        Easing::OutCubic,
        Easing::InOutCubic,
        Easing::OutQuint,
    ] {
        let mut prev = e.apply(0.0);
        for i in 1..=200 {
            let v = e.apply(i as f64 / 200.0);
            assert!(v >= prev - 1e-12, "{e:?} not monotonic at {i}/200");
            assert!(v <= 1.0 + 1e-12, "{e:?} exceeds 1 at {i}/200");
            prev = v;
        }
    }
}

#[test]
fn impact_spring_overshoots() {
    let max = max_over_samples(Easing::ImpactSpring);
    assert!(max > 1.02, "ImpactSpring max was {max}");
}

#[test]
fn editorial_spring_overshoot_is_small() {
    let max = max_over_samples(Easing::EditorialSpring);
    assert!(max < 1.05, "EditorialSpring max was {max}");
}

#[test]
fn every_curve_makes_progress_early() {
    for e in ALL {
        let v = e.apply(0.1);
        assert!(v > 0.0, "{e:?}: apply(0.1) = {v}");
    }
}

#[test]
fn easing_is_deterministic() {
    for e in ALL {
        for i in 0..=50 {
            let t = i as f64 / 50.0;
            assert_eq!(e.apply(t).to_bits(), e.apply(t).to_bits());
        }
    }
}

#[test]
fn easing_deserializes_from_snake_case_and_defaults_to_linear() {
    let cases = [
        ("linear", Easing::Linear),
        ("in_cubic", Easing::InCubic),
        ("out_cubic", Easing::OutCubic),
        ("in_out_cubic", Easing::InOutCubic),
        ("out_quint", Easing::OutQuint),
        ("editorial_spring", Easing::EditorialSpring),
        ("impact_spring", Easing::ImpactSpring),
    ];
    for (name, expected) in cases {
        let parsed: Easing = serde_json::from_str(&format!("\"{name}\"")).unwrap();
        assert_eq!(parsed, expected, "{name}");
    }
    assert_eq!(Easing::default(), Easing::Linear);
}

#[test]
fn preset_durations_are_ordered_impact_editorial_calm() {
    let impact = MotionPreset::Impact.params().duration;
    let editorial = MotionPreset::Editorial.params().duration;
    let calm = MotionPreset::Calm.params().duration;
    assert!(impact < editorial, "{impact} !< {editorial}");
    assert!(editorial < calm, "{editorial} !< {calm}");
}

#[test]
fn preset_params_are_sane() {
    for p in [
        MotionPreset::Calm,
        MotionPreset::Editorial,
        MotionPreset::Impact,
    ] {
        let params = p.params();
        assert!(params.duration > 0.0);
        assert!(params.stagger > 0.0);
        assert!(params.travel > 0.0);
    }
}
