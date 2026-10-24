//! Integration tests for spring physics presets, settling duration, and layer motion evaluation.

use motion_core::easing::{spring_progress, spring_response, spring_velocity, Easing};
use motion_core::scene::{Motion, MotionOp, MotionProject, SpringSpec};
use motion_core::timeline::evaluate_frame;
use serde_json::json;

#[test]
fn test_snappy_monotonic_rise_no_overshoot() {
    // SNAPPY is critically damped (stiffness 320, damping 32, mass 1) for a crisp editorial snap.
    // Over its standard fast duration (0.35s), spring progress rises monotonically
    // and never overshoots past 1.0001.
    let dur = 0.35;
    let mut prev = 0.0;
    for step in 0..=350 {
        let t = step as f64 * dur / 350.0;
        let p = spring_progress(t, dur, SpringSpec::SNAPPY);
        assert!(
            p >= prev - 1e-6,
            "SNAPPY must rise monotonically: decreased from {prev} to {p} at t={t}"
        );
        assert!(
            p <= 1.0001,
            "SNAPPY must not have overshoot > 1.0001: got {p} at t={t}"
        );
        prev = p;
    }
    assert!((spring_progress(dur, dur, SpringSpec::SNAPPY) - 1.0).abs() < 1e-6);
}

#[test]
fn test_bouncy_overshoot_peak_between_1_05_and_1_25() {
    // BOUNCY is under-damped (stiffness 220, damping 14, mass 1) for tactile pop and overshoot.
    // Raw oscillator response peak must be strictly between 1.05 and 1.25.
    let mut peak_response = 0.0f64;
    for step in 0..=1000 {
        let t = step as f64 * 0.001;
        let val = spring_response(t, SpringSpec::BOUNCY);
        if val > peak_response {
            peak_response = val;
        }
    }
    assert!(
        (1.05..=1.25).contains(&peak_response),
        "BOUNCY peak response must be between 1.05 and 1.25, got {peak_response}"
    );

    // Spring progress across 1.0s duration also reflects the same peak between 1.05 and 1.25.
    let dur = 1.0;
    let mut peak_progress = 0.0f64;
    for step in 0..=1000 {
        let t = step as f64 * dur / 1000.0;
        let p = spring_progress(t, dur, SpringSpec::BOUNCY);
        if p > peak_progress {
            peak_progress = p;
        }
    }
    assert!(
        (1.05..=1.25).contains(&peak_progress),
        "BOUNCY peak progress must be between 1.05 and 1.25, got {peak_progress}"
    );
}

#[test]
fn test_heavy_slower_rise_time_than_snappy() {
    // HEAVY has cinematic weight (stiffness 90, damping 28, mass 2.5).
    // Verify it rises substantially slower than SNAPPY.
    let time_to_reach = |s: SpringSpec, target: f64| -> f64 {
        for step in 0..=3000 {
            let t = step as f64 * 0.001;
            if spring_response(t, s) >= target {
                return t;
            }
        }
        f64::MAX
    };

    let snappy_t50 = time_to_reach(SpringSpec::SNAPPY, 0.5);
    let heavy_t50 = time_to_reach(SpringSpec::HEAVY, 0.5);
    let snappy_t80 = time_to_reach(SpringSpec::SNAPPY, 0.8);
    let heavy_t80 = time_to_reach(SpringSpec::HEAVY, 0.8);

    assert!(
        heavy_t50 > snappy_t50 * 2.0,
        "HEAVY 50% rise time ({heavy_t50}s) must be > 2x slower than SNAPPY ({snappy_t50}s)"
    );
    assert!(
        heavy_t80 > snappy_t80 * 2.0,
        "HEAVY 80% rise time ({heavy_t80}s) must be > 2x slower than SNAPPY ({snappy_t80}s)"
    );
}

#[test]
fn test_settling_duration_strict_threshold_convergence() {
    let presets = [
        ("SNAPPY", SpringSpec::SNAPPY),
        ("BOUNCY", SpringSpec::BOUNCY),
        ("HEAVY", SpringSpec::HEAVY),
        ("GENTLE", SpringSpec::GENTLE),
    ];

    let threshold = 0.01f32;
    for (name, spec) in presets {
        let t_settle = spec.settling_duration(threshold);
        assert!(
            t_settle > 0.0 && t_settle < 10.0,
            "{name}: settling duration must be positive and bounded, got {t_settle}"
        );

        // Verify that for all t > t_settle, error |x(t) - 1.0| is strictly < threshold
        // and velocity |v(t)| is strictly < threshold.
        for step in 1..=1000 {
            let t = t_settle + step as f64 * 0.002;
            let x = spring_response(t, spec);
            let pos_err = (x - 1.0).abs();
            assert!(
                pos_err < threshold as f64,
                "{name}: position error {pos_err} at t={t} exceeds threshold {threshold}"
            );

            let v = spring_velocity(t, spec);
            let vel_err = v.abs();
            assert!(
                vel_err < threshold as f64,
                "{name}: velocity {vel_err} at t={t} exceeds threshold {threshold}"
            );
        }
    }
}

#[test]
fn test_layer_motion_evaluation_with_sprung_motion() {
    // Build a minimal Motion with spring = Some(SpringSpec::BOUNCY) using mo::sprung
    let mut sprung_motion = Motion {
        spring: None,
        id: None,
        target: "hero".to_string(),
        start: 0.0,
        duration: 1.0,
        easing: Easing::Linear,
        op: MotionOp::Move {
            from: [0.0, 0.0],
            to: [200.0, 0.0],
            via: None,
        },
    };
    sprung_motion.spring = Some(SpringSpec::BOUNCY);

    let motion_val = serde_json::to_value(&sprung_motion).expect("motion serializes");

    let project_val = json!({
        "version": "0.2",
        "project": { "name": "spring_layer_test" },
        "canvas": { "width": 1080, "height": 1920, "fps": 30, "background": "#000000" },
        "scenes": [{
            "id": "s1",
            "start_seconds": 0.0,
            "duration_seconds": 3.0,
            "layers": [{
                "id": "hero",
                "type": "rectangle",
                "x": 100.0,
                "y": 100.0,
                "width": 100.0,
                "height": 100.0,
                "fill": "#FFFFFF"
            }],
            "motions": [motion_val]
        }]
    });

    let project = MotionProject::from_json(&project_val.to_string()).expect("test project parses");

    // Frame 0 (t = 0.0s): layer starts at x = 100
    let f0 = evaluate_frame(&project, 0).expect("frame 0 evaluates");
    let l0 = f0
        .layers
        .iter()
        .find(|l| l.id == "hero")
        .expect("hero layer exists");
    assert!(
        (l0.transform.e - 100.0).abs() < 1e-4,
        "starts at base x=100"
    );

    // Sample across the 30 fps frames for 1.0s motion (frames 0 to 30)
    let mut x_positions = Vec::new();
    let mut overshot = false;
    for frame_idx in 0..=30 {
        let f = evaluate_frame(&project, frame_idx).expect("frame evaluates");
        let l = f
            .layers
            .iter()
            .find(|l| l.id == "hero")
            .expect("hero layer exists");
        let x = l.transform.e;
        x_positions.push(x);
        if x > 300.0 + 10.0 {
            overshot = true;
        }
    }

    // 1. BOUNCY motion must overshoot past 300.0 (e.g. up to ~337)
    assert!(
        overshot,
        "layer motion with BOUNCY spring must overshoot target 300.0"
    );

    // 2. Non-linear interpolation: at t = 0.5s (frame 15), linear would be 100 + 0.5 * 200 = 200
    let mid_x = x_positions[15];
    assert!(
        (mid_x - 200.0).abs() > 5.0,
        "spring interpolation must be non-linear (midpoint was {mid_x})"
    );

    // 3. Smooth motion: delta between consecutive frames is smooth and continuous
    for i in 1..x_positions.len() {
        let delta = (x_positions[i] - x_positions[i - 1]).abs();
        assert!(
            delta < 65.0,
            "step between frame {} and {} was {delta} (too abrupt)",
            i - 1,
            i
        );
    }

    // 4. Lands exactly on 300.0 at end of duration (frame 30) and holds it after
    for frame_idx in [30, 35, 45, 60] {
        let f = evaluate_frame(&project, frame_idx).expect("frame evaluates");
        let l = f
            .layers
            .iter()
            .find(|l| l.id == "hero")
            .expect("hero layer exists");
        assert!(
            (l.transform.e - 300.0).abs() < 1e-3,
            "must land and hold target 300.0 at frame {frame_idx}, got {}",
            l.transform.e
        );
    }
}
