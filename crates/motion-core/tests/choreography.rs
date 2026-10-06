//! Choreography v0 (0.8): handoff -> downbeat snapping and `compile_with_music`.
//! See docs/SOUND_DESIGN.md §7.

use std::path::Path;

use motion_core::assets::AssetManifest;
use motion_core::audio::{beat_scenes, snap_tolerance, MusicPlan, MUSIC_PLAN_VERSION};
use motion_core::compiler::choreography::{snap_handoffs, MAX_STRETCH, MIN_STRETCH};
use motion_core::compiler::{
    compile_full, compile_with_music, resolve_taste, ApproxMeasure, AssetLibrary,
};
use motion_core::scene::MotionProject;
use motion_core::validate::validate;
use motion_core::{CreativeIntent, StyleProfile};

const ASSETS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets");
const INTENT: &str = include_str!("../../../examples/editorial_demo.intent.json");
const STYLES: [(&str, &str); 4] = [
    (
        "editorial_demo",
        include_str!("../../../examples/editorial_demo.style.json"),
    ),
    (
        "warm_editorial",
        include_str!("../../../examples/taste/warm_editorial.style.json"),
    ),
    (
        "dark_technical",
        include_str!("../../../examples/taste/dark_technical.style.json"),
    ),
    (
        "playful_print",
        include_str!("../../../examples/taste/playful_print.style.json"),
    ),
];

/// Handoff anchors (overlap midpoints) of a duration/overlap plan.
fn anchors(durations: &[f64], overlaps: &[f64]) -> Vec<f64> {
    let mut start = 0.0;
    let mut out = Vec::new();
    for i in 1..durations.len() {
        start += durations[i - 1] - overlaps[i - 1];
        out.push(start + overlaps[i - 1] / 2.0);
    }
    out
}

// ---- snap_handoffs -------------------------------------------------------

#[test]
fn snaps_a_close_handoff_exactly() {
    // Beat 0 = 6 s, overlap 0.6: anchor 5.7. Downbeat 5.8 is 0.1 s away.
    let out = snap_handoffs(&[6.0, 5.0], &[0.6, 0.0], &[0.0, 2.0, 5.8, 8.0], 0.22);
    assert_eq!(out, vec![6.1, 5.0]);
    let a = anchors(&out, &[0.6, 0.0]);
    assert!((a[0] - 5.8).abs() < 1e-9);
}

#[test]
fn outside_tolerance_is_unchanged() {
    let d = [6.0, 5.0];
    let out = snap_handoffs(&d, &[0.6, 0.0], &[5.7 + 0.23], 0.22);
    assert_eq!(out, d.to_vec());
}

#[test]
fn empty_downbeats_and_single_beat_are_identity() {
    let d = [6.0, 5.0, 4.0];
    assert_eq!(snap_handoffs(&d, &[0.5, 0.5, 0.0], &[], 0.3), d.to_vec());
    assert_eq!(snap_handoffs(&[6.0], &[0.0], &[6.0], 0.3), vec![6.0]);
}

#[test]
fn stretch_bounds_are_respected() {
    // Grow > +20 %: d = 2.5, overlap 0.5 -> anchor 2.25; downbeat 3.0 needs
    // +0.75 (> +20 % = 0.5).
    let out = snap_handoffs(&[2.5, 5.0], &[0.5, 0.0], &[3.0], 1.0);
    assert_eq!(out, vec![2.5, 5.0]);
    // Within +20 %: downbeat 2.7 needs +0.45 (<= 0.5) -> snaps.
    let out = snap_handoffs(&[2.5, 5.0], &[0.5, 0.0], &[2.7], 1.0);
    assert_eq!(out, vec![2.95, 5.0]);
    // Shrink < -15 %: planned 10 s, anchor 9.75; downbeat 8.0 needs -1.75 (> 1.5).
    let out = snap_handoffs(&[10.0, 5.0], &[0.5, 0.0], &[8.0], 5.0);
    assert_eq!(out, vec![10.0, 5.0]);
    // Within -15 %: downbeat 8.5 needs -1.25 -> 8.75.
    let out = snap_handoffs(&[10.0, 5.0], &[0.5, 0.0], &[8.5], 5.0);
    assert_eq!(out, vec![8.75, 5.0]);
    // Below the 2 s floor: planned 2.2, anchor 1.95; downbeat 1.7 -> 1.95 < 2.0
    // (within -15 % = 1.87) is skipped.
    let out = snap_handoffs(&[2.2, 5.0], &[0.5, 0.0], &[1.7], 1.0);
    assert_eq!(out, vec![2.2, 5.0]);
}

#[test]
fn sequential_processing_uses_snapped_durations() {
    // Beat 0 = 6.0 (overlap 0.5): anchor 1 = 5.75; downbeat 6.0 -> d0 = 6.25.
    // Beat 1 then starts at 5.75 (not 5.5), so anchor 2 = 5.75 + 5.5 + 0.25 =
    // 11.5 (unsnapped 11.25, 0.45 from downbeat 11.7: outside tolerance 0.25).
    // Using the snapped duration, 11.7 is 0.2 away -> d1 = 6.2.
    let out = snap_handoffs(&[6.0, 6.0, 5.0], &[0.5, 0.5, 0.0], &[6.0, 11.7], 0.25);
    assert_eq!(out, vec![6.25, 6.2, 5.0]);
    let a = anchors(&out, &[0.5, 0.5, 0.0]);
    assert!((a[0] - 6.0).abs() < 1e-9);
    assert!((a[1] - 11.7).abs() < 1e-9);
}

#[test]
fn ties_pick_the_earlier_downbeat() {
    // Anchor 5.75; downbeats 5.65 and 5.85 are both 0.1 away. Unsorted input.
    let out = snap_handoffs(&[6.0, 5.0], &[0.5, 0.0], &[5.85, 5.65], 0.2);
    assert_eq!(out, vec![5.9, 5.0]);
}

// ---- compile_with_music --------------------------------------------------

fn grid(seconds: f64) -> MusicPlan {
    let downbeats: Vec<f64> = (0..)
        .map(|k| f64::from(k) * 2.0)
        .take_while(|t| *t <= seconds)
        .collect();
    let beats: Vec<f64> = (0..)
        .map(|k| f64::from(k) * 0.5)
        .take_while(|t| *t <= seconds)
        .collect();
    MusicPlan {
        version: MUSIC_PLAN_VERSION.to_string(),
        track: "grid.wav".to_string(),
        duration: seconds,
        bpm: 120.0,
        beat_times: beats,
        downbeat_times: downbeats,
        sections: vec![],
        gain_db: -18.0,
        sha256: "0".repeat(64),
        lufs: None,
        lra: None,
    }
}

fn build(style: &StyleProfile, music: Option<&MusicPlan>) -> (MotionProject, CreativeIntent) {
    let intent = CreativeIntent::from_json(INTENT).expect("intent");
    let p = compile_with_music(
        &intent,
        style,
        None,
        &AssetLibrary::new(ASSETS),
        &ApproxMeasure,
        &AssetManifest::empty(),
        music,
    )
    .expect("compiles");
    (p, intent)
}

#[test]
fn compile_with_music_none_equals_compile_full() {
    let intent = CreativeIntent::from_json(INTENT).expect("intent");
    for (name, json) in STYLES {
        let style = StyleProfile::from_json(json).expect("style");
        let lib = AssetLibrary::new(ASSETS);
        let a = compile_full(
            &intent,
            &style,
            None,
            &lib,
            &ApproxMeasure,
            &AssetManifest::empty(),
        )
        .expect("compile_full");
        let (b, _) = build(&style, None);
        assert_eq!(a.to_json_pretty(), b.to_json_pretty(), "{name}");
    }
}

#[test]
fn handoffs_land_on_downbeats_within_bounds() {
    let music = grid(240.0);
    let mut total_snaps = 0;
    for (name, json) in STYLES {
        let style = StyleProfile::from_json(json).expect("style");
        let (base, intent) = build(&style, None);
        let (snapped, _) = build(&style, Some(&music));
        validate(&snapped, Some(Path::new(ASSETS)))
            .unwrap_or_else(|e| panic!("{name}: validation failed:\n{e}"));
        let tol = snap_tolerance(resolve_taste(&intent, &style, None).rhythm);

        let (bb, sb) = (beat_scenes(&base), beat_scenes(&snapped));
        assert_eq!(bb.len(), sb.len(), "{name}");
        let mut errors = Vec::new();
        for i in 0..sb.len() {
            let ratio = sb[i].duration_seconds / bb[i].duration_seconds;
            assert!(
                (MIN_STRETCH - 1e-6..=MAX_STRETCH + 1e-6).contains(&ratio),
                "{name}: beat {i} stretched {ratio}"
            );
        }
        for i in 1..sb.len() {
            let s = sb[i].start_seconds;
            let anchor = s + (sb[i - 1].end_seconds() - s).max(0.0) / 2.0;
            let nearest = music
                .downbeat_times
                .iter()
                .copied()
                .min_by(|a, b| (a - anchor).abs().total_cmp(&(b - anchor).abs()))
                .expect("downbeats");
            let err = (nearest - anchor).abs();
            errors.push(err);
            if err > 1.0e-3 {
                // Not snapped: the snap must have been out of tolerance or bounds.
                let needed = sb[i - 1].duration_seconds + (nearest - anchor);
                let planned = bb[i - 1].duration_seconds;
                let blocked = err > tol + 1e-9
                    || needed < 2.0
                    || needed < planned * MIN_STRETCH - 1e-6
                    || needed > planned * MAX_STRETCH + 1e-6;
                assert!(
                    blocked,
                    "{name}: handoff {i} at {anchor} was snappable ({err})"
                );
            } else {
                total_snaps += 1;
            }
        }
        eprintln!(
            "{name}: tol {tol} handoff->downbeat errors (ms): {:?}",
            errors
                .iter()
                .map(|e| (e * 1000.0).round())
                .collect::<Vec<_>>()
        );

        // No motion outlives its scene (validate enforces; check explicitly).
        for s in &snapped.scenes {
            for m in &s.motions {
                assert!(m.start + m.duration <= s.duration_seconds + 1e-6, "{name}");
            }
            for cm in s.camera.iter().flat_map(|c| &c.motions) {
                assert!(
                    cm.start + cm.duration <= s.duration_seconds + 1e-6,
                    "{name}"
                );
            }
        }
    }
    assert!(total_snaps >= 1, "the sweep never snapped anything");
}

#[test]
fn compile_with_music_is_deterministic() {
    let music = grid(240.0);
    let style = StyleProfile::from_json(STYLES[1].1).expect("style");
    let (a, _) = build(&style, Some(&music));
    let (b, _) = build(&style, Some(&music));
    assert_eq!(a.to_json_pretty(), b.to_json_pretty());
}
