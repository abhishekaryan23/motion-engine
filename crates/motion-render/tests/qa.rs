//! Structural motion QA on tiny hand-built projects.

use std::path::Path;

use std::path::PathBuf;

use motion_core::compiler::{compile, AssetLibrary, FontSet};
use motion_core::scene::Phase;
use motion_core::{CreativeIntent, MotionProject, StyleProfile};
use motion_render::qa::analyze;
use motion_render::{
    lifecycle_report, structural_profile, CpuRenderer, FontMeasure, MotionProfile, PhaseActivity,
    SceneQa, SpikeLevel, Status,
};

fn project(layers: &str, motions: &str) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "qa", "duration_seconds": 2.0 }},
        "canvas": {{ "width": 108, "height": 192, "fps": 30, "background": "#FFFFFF" }},
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 2.0,
                       "layers": {layers}, "motions": {motions} }} ]
    }}"##
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

fn rect() -> &'static str {
    r##"{ "id": "r", "type": "rectangle", "x": 10, "y": 60, "width": 40, "height": 40, "fill": "#000000" }"##
}

fn profile(p: &MotionProject) -> MotionProfile {
    let r = CpuRenderer::new(p, Path::new(".")).expect("renderer");
    structural_profile(p, &r, 1).expect("profile")
}

#[test]
fn hold_then_jump_is_a_spike_after_static_run() {
    let p = project(
        &format!("[{}]", rect()),
        r##"[{ "target": "r", "op": "move", "start": 1.0, "duration": 0.05,
               "easing": "linear", "from": [0, 0], "to": [50, 0] }]"##,
    );
    let prof = profile(&p);
    assert!(!prof.spikes.is_empty(), "no spike: {prof:?}");
    assert!(
        prof.spikes.iter().all(|&f| (31..=32).contains(&f)),
        "spikes {:?}",
        prof.spikes
    );
    assert!(
        prof.static_runs.iter().any(|&(a, b)| a == 0 && b >= 29),
        "runs {:?}",
        prof.static_runs
    );
    assert!(
        prof.verdict.starts_with("static"),
        "verdict {}",
        prof.verdict
    );
    assert!(
        prof.verdict.contains("spike 31\u{2013}32"),
        "verdict {}",
        prof.verdict
    );
}

#[test]
fn ramp_then_spike_after_hold_is_flagged() {
    // Hold 1.2 s, then a 3-frame ease-in (small diffs) before a fast slide.
    let p = project(
        &format!("[{}]", rect()),
        r##"[{ "target": "r", "op": "move", "start": 1.2, "duration": 0.2,
               "easing": "in_cubic", "from": [0, 0], "to": [50, 0] }]"##,
    );
    let prof = profile(&p);
    assert_eq!(prof.spike_clusters.len(), 1, "{prof:?}");
    let c = &prof.spike_clusters[0];
    assert!(
        c.start > 36 && c.end >= c.start && c.peak_multiple > 4.0,
        "{c:?}"
    );
    assert!(
        prof.verdict.starts_with("static 1.2s then spike"),
        "verdict {}",
        prof.verdict
    );
}

#[test]
fn spike_clusters_group_consecutive_frames() {
    let mut d = vec![0.0f32; 60];
    d[30] = 0.05;
    d[31] = 0.09;
    d[32] = 0.04;
    d[50] = 0.06;
    let p = analyze(30, 1, d);
    assert_eq!(p.spikes, vec![31, 32, 33, 51]);
    assert_eq!(p.spike_clusters.len(), 2);
    assert_eq!(
        (
            p.spike_clusters[0].start,
            p.spike_clusters[0].end,
            p.spike_clusters[0].peak_frame
        ),
        (31, 33, 32)
    );
    assert!(p.verdict.contains("spike 31\u{2013}33"), "{}", p.verdict);
}

#[test]
fn smooth_motion_has_no_spikes() {
    let p = project(
        &format!("[{}]", rect()),
        r##"[{ "target": "r", "op": "move", "start": 0.0, "duration": 2.0,
               "easing": "linear", "from": [0, 0], "to": [50, 0] }]"##,
    );
    let prof = profile(&p);
    assert!(prof.spikes.is_empty(), "spikes {:?}", prof.spikes);
    assert_eq!(prof.verdict, "ok");
    assert!(prof.median > 0.0);
}

#[test]
fn animated_grain_is_ignored() {
    let p = project(
        r##"[{ "id": "g", "type": "texture", "x": 0, "y": 0, "width": 108, "height": 192,
               "material": "grain", "seed": 3, "color": "#000000", "intensity": 1.0,
               "scale": 1, "animated": true }]"##,
        "[]",
    );
    let prof = profile(&p);
    assert!(prof.max < 1e-6, "max {}", prof.max);
    assert!(prof.spikes.is_empty());
}

#[test]
fn profile_is_deterministic() {
    let p = project(
        &format!("[{}]", rect()),
        r##"[{ "target": "r", "op": "move", "start": 1.0, "duration": 0.05,
               "easing": "linear", "from": [0, 0], "to": [50, 0] }]"##,
    );
    assert_eq!(profile(&p), profile(&p));
}

#[test]
fn analyze_handles_empty_input() {
    let p = analyze(30, 1, vec![]);
    assert_eq!(p.verdict, "ok");
    assert!(p.spikes.is_empty() && p.static_runs.is_empty());
}

// ---------------------------------------------------------------------------
// Lifecycle diagnostics
// ---------------------------------------------------------------------------

/// Two scenes at 30 fps.
/// A: start 0, 6 s. enter 0.5, settle 1.0, read 2.0, evolve 3.0, anticipate 5.0, bridge 5.5
///    -> frames: enter 15, settle 30, read 60, evolve 90, anticipate 150, bridge 165, end 180.
/// B: start 5.5 (frame 165), 4 s (last). enter 0.5, settle 1.0, read 1.5, evolve 2.0,
///    anticipate 3.0, bridge 4.0 == duration.
fn two_scene_project(motions_a: &str) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "qa", "duration_seconds": 9.5 }},
        "canvas": {{ "width": 108, "height": 192, "fps": 30, "background": "#FFFFFF" }},
        "scenes": [
          {{ "id": "beat_a", "start_seconds": 0.0, "duration_seconds": 6.0,
             "layers": [], "motions": {motions_a},
             "lifecycle": {{ "enter": 0.5, "settle": 1.0, "read": 2.0, "evolve": 3.0,
                             "anticipate": 5.0, "bridge": 5.5 }} }},
          {{ "id": "beat_b", "start_seconds": 5.5, "duration_seconds": 4.0,
             "layers": [], "motions": [],
             "lifecycle": {{ "enter": 0.5, "settle": 1.0, "read": 1.5, "evolve": 2.0,
                             "anticipate": 3.0, "bridge": 4.0 }} }},
          {{ "id": "backdrop", "start_seconds": 0.0, "duration_seconds": 9.5,
             "layers": [], "motions": [] }}
        ]
    }}"##
    );
    MotionProject::from_json(&json).expect("two-scene fixture parses")
}

/// Diffs for 285 frames (284 transitions); `f` maps the incoming frame to a diff.
fn synth(step: u32, f: impl Fn(u32) -> f32) -> MotionProfile {
    let n = (285 / step).saturating_sub(1);
    analyze(30, step, (1..=n).map(|i| f(i * step)).collect())
}

fn phase(qa: &SceneQa, p: Phase) -> &PhaseActivity {
    qa.phases.iter().find(|a| a.phase == p).expect("phase")
}

fn mv(target: &str, start: f64) -> String {
    format!(
        r##"{{ "target": "{target}", "op": "move", "start": {start}, "duration": 0.3,
              "easing": "linear", "from": [0, 0], "to": [5, 0] }}"##
    )
}

#[test]
fn lifecycle_skips_scenes_without_lifecycle_and_has_seven_phases() {
    let p = two_scene_project("[]");
    let prof = synth(1, |_| 0.01);
    let report = lifecycle_report(&p, &prof);
    assert_eq!(report.len(), 2, "backdrop has no lifecycle");
    assert_eq!(report[0].scene, "beat_a");
    assert_eq!(report[1].scene, "beat_b");
    for q in &report {
        assert_eq!(q.phases.len(), 7);
        let order: Vec<Phase> = q.phases.iter().map(|a| a.phase).collect();
        assert_eq!(order, Phase::ALL.to_vec());
    }
}

#[test]
fn phase_frames_map_scene_time_to_absolute_frames() {
    let p = two_scene_project("[]");
    let prof = synth(1, |_| 0.01);
    let a = &lifecycle_report(&p, &prof)[0];
    let bounds: Vec<(u32, u32)> = a
        .phases
        .iter()
        .map(|x| (x.start_frame, x.end_frame))
        .collect();
    assert_eq!(
        bounds,
        vec![
            (0, 15),
            (15, 30),
            (30, 60),
            (60, 90),
            (90, 150),
            (150, 165),
            (165, 180)
        ]
    );
    // Scene B starts at frame 165 (5.5 s): phases are offset by 165.
    let b = &lifecycle_report(&p, &prof)[1];
    assert_eq!(
        (
            phase(b, Phase::Enter).start_frame,
            phase(b, Phase::Enter).end_frame
        ),
        (180, 195)
    );
    assert_eq!(
        (
            phase(b, Phase::Bridge).start_frame,
            phase(b, Phase::Bridge).end_frame
        ),
        (285, 285)
    );
}

#[test]
fn diff_index_is_assigned_to_the_phase_containing_its_frame() {
    let p = two_scene_project("[]");
    // A single loud transition into frame 30 (index 29): the first SETTLE frame.
    let prof = synth(1, |f| if f == 30 { 0.1 } else { 0.0 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(phase(a, Phase::Enter).status, Status::Static);
    // Mean over the 30 SETTLE transitions is 0.1 / 30 (< 0.004): Low, not Static.
    assert_eq!(phase(a, Phase::Settle).status, Status::Low);
    assert!((phase(a, Phase::Settle).max - 0.1).abs() < 1e-6);
    // Frame 29 is still ENTER.
    let prof = synth(1, |f| if f == 29 { 0.1 } else { 0.0 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(phase(a, Phase::Enter).status, Status::Pass);
    assert_eq!(phase(a, Phase::Settle).status, Status::Static);
}

#[test]
fn status_classification_static_low_pass_empty() {
    let p = two_scene_project("[]");
    let prof = synth(1, |f| match f {
        15..=29 => 0.05,  // ENTER: pass
        30..=59 => 0.003, // SETTLE: below 2 x STATIC, not static -> low
        60..=89 => 0.0,   // READ: static
        _ => 0.01,
    });
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(phase(a, Phase::Enter).status, Status::Pass);
    assert_eq!(phase(a, Phase::Settle).status, Status::Low);
    assert_eq!(phase(a, Phase::Read).status, Status::Static);
    assert_eq!(phase(a, Phase::Evolve).status, Status::Pass);
    assert!((phase(a, Phase::Settle).mean - 0.003).abs() < 1e-6);
    assert_eq!(a.read_activity, 0.0);

    // A mix of one tiny and one quiet diff is Low, not Static.
    let prof = synth(1, |f| if f % 2 == 0 { 0.0035 } else { 0.0 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(phase(a, Phase::Read).status, Status::Low);

    // Sparse sampling leaves short phases with no transition at all.
    let prof = synth(60, |_| 0.01);
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(phase(a, Phase::PreEnter).status, Status::Empty); // frames 0..15
    assert_eq!(phase(a, Phase::Enter).status, Status::Empty); // frames 15..30
    assert_eq!(phase(a, Phase::Settle).status, Status::Empty);
    assert_eq!(phase(a, Phase::Evolve).status, Status::Pass); // frame 120 in [90, 150)
}

#[test]
fn evolve_events_cluster_close_starts_and_exclude_stage_and_ghost() {
    let motions = [
        mv("x", 1.0),       // before READ ends: before `read` (2.0) -> ignored
        mv("x", 3.0),       // event 1
        mv("y", 3.1),       // within 0.12 s of the previous -> same event
        mv("a.stage", 3.3), // excluded
        mv("a.ghost", 3.4), // excluded
        mv("x", 3.5),       // event 2
        mv("x", 3.6),       // same event
        mv("x", 4.0),       // event 3 ...
        mv("x", 4.11),      // ... chains ...
        mv("x", 4.22),      // ... through 0.12 s steps
        mv("x", 5.0),       // at ANTICIPATE: excluded
    ]
    .join(",");
    let p = two_scene_project(&format!("[{motions}]"));
    let prof = synth(1, |_| 0.01);
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(a.evolve_events, 3);
    // Scene B has no motions but a 1.0 s EVOLVE.
    let b = &lifecycle_report(&p, &prof)[1];
    assert_eq!(b.evolve_events, 0);
    assert!(
        b.warnings.iter().any(|w| w == "No evolve events"),
        "{:?}",
        b.warnings
    );
    assert!(!a.warnings.iter().any(|w| w == "No evolve events"));
}

#[test]
fn spike_ratio_high_ok_and_last_scene_rule() {
    let p = two_scene_project("[]");
    // Steady 0.01 body; loud transition inside A's final 0.2 s.
    let high = synth(1, |f| if (160..=170).contains(&f) { 0.2 } else { 0.01 });
    let a = &lifecycle_report(&p, &high)[0];
    assert!((a.spike_ratio - 20.0).abs() < 1e-3, "{}", a.spike_ratio);
    assert_eq!(a.spike, SpikeLevel::High);

    let ok = synth(1, |f| if (160..=170).contains(&f) { 0.05 } else { 0.01 });
    let a = &lifecycle_report(&p, &ok)[0];
    assert!((a.spike_ratio - 5.0).abs() < 1e-3, "{}", a.spike_ratio);
    assert_eq!(a.spike, SpikeLevel::Ok);

    // The last scene never has an outgoing transition.
    let b = &lifecycle_report(&p, &high)[1];
    assert_eq!((b.spike_ratio, b.spike), (0.0, SpikeLevel::Ok));
}

#[test]
fn spike_ratio_uses_reference_floor_for_static_scenes() {
    let p = two_scene_project("[]");
    let prof = synth(1, |f| if f == 165 { 0.06 } else { 0.0 });
    let a = &lifecycle_report(&p, &prof)[0];
    // median 0 -> reference 0.005 -> 0.06 / 0.005 = 12
    assert!((a.spike_ratio - 12.0).abs() < 1e-3, "{}", a.spike_ratio);
    assert_eq!(a.spike, SpikeLevel::High);
}

#[test]
fn longest_static_hold_is_measured_within_settle_to_bridge() {
    let p = two_scene_project("[]");
    // Static frames 45..=104 (2.0 s) inside [settle 30, bridge 165); noise elsewhere.
    let prof = synth(1, |f| if (45..=104).contains(&f) { 0.0 } else { 0.01 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert!(
        (a.longest_static_hold - 2.0).abs() < 1e-4,
        "{}",
        a.longest_static_hold
    );
    assert!(
        a.warnings
            .iter()
            .any(|w| w == "Dead hold 2.0s in READ/EVOLVE"),
        "{:?}",
        a.warnings
    );
    // Stillness before SETTLE (during ENTER) is not counted.
    let prof = synth(1, |f| if f < 30 { 0.0 } else { 0.01 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(a.longest_static_hold, 0.0);
    // With step 2, a run of 10 static transitions is 20 frames = 0.667 s.
    let prof = synth(2, |f| if (60..80).contains(&f) { 0.0 } else { 0.01 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert!(
        (a.longest_static_hold - 10.0 * 2.0 / 30.0).abs() < 1e-4,
        "{}",
        a.longest_static_hold
    );
}

#[test]
fn static_read_before_abrupt_transition_warns() {
    let p = two_scene_project("[]");
    // Quiet READ + EVOLVE, then a hard cut in the last 0.2 s.
    let prof = synth(1, |f| match f {
        0..=59 => 0.02,
        60..=149 => 0.001,
        160..=170 => 0.3,
        _ => 0.02,
    });
    let a = &lifecycle_report(&p, &prof)[0];
    assert_eq!(phase(a, Phase::Read).status, Status::Static);
    assert_eq!(a.spike, SpikeLevel::High);
    assert!(
        a.warnings
            .iter()
            .any(|w| w == "Static read phase before abrupt transition"),
        "{:?}",
        a.warnings
    );
    let text = a.report();
    assert!(text.starts_with("beat_a\n"), "{text}");
    assert!(
        text.contains("  READ structural activity: STATIC (mean 0.0010)"),
        "{text}"
    );
    assert!(text.contains("  transition spike ratio: HIGH (x"), "{text}");
    assert!(
        text.contains("  WARNING: Static read phase before abrupt transition"),
        "{text}"
    );
}

#[test]
fn healthy_scene_has_no_warnings_and_static_enter_warns() {
    // One motion in EVOLVE, healthy diffs everywhere.
    let p = two_scene_project(&format!("[{}]", mv("x", 3.0)));
    let prof = synth(1, |_| 0.02);
    let a = &lifecycle_report(&p, &prof)[0];
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
    let text = a.report();
    assert!(text.contains("  ENTER activity: PASS\n"));
    assert!(text.contains("  SETTLE: PASS\n"));
    assert!(text.contains("  READ structural activity: PASS\n"));
    assert!(text.contains("  EVOLVE events: 1 (activity PASS)\n"));
    assert!(text.contains("  ANTICIPATE: PASS\n"));
    assert!(text.contains("  longest static hold: 0.0s\n"));

    // Nothing moves during ENTER (frames 15..30).
    let prof = synth(1, |f| if (15..30).contains(&f) { 0.0 } else { 0.02 });
    let a = &lifecycle_report(&p, &prof)[0];
    assert!(
        a.warnings.iter().any(|w| w == "ENTER shows no activity"),
        "{:?}",
        a.warnings
    );
}

#[test]
fn lifecycle_report_is_deterministic() {
    let p = two_scene_project(&format!("[{}]", mv("x", 3.0)));
    let prof = synth(1, |f| (f % 7) as f32 * 0.003);
    assert_eq!(lifecycle_report(&p, &prof), lifecycle_report(&p, &prof));
}

#[test]
fn compiled_three_beat_story_has_one_scene_qa_per_beat() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let assets = root.join("assets");
    let intent = CreativeIntent::from_json(
        &std::fs::read_to_string(root.join("examples/public/three-beat-story.intent.json"))
            .expect("read intent"),
    )
    .expect("intent parses");
    let style = StyleProfile::default();
    let fonts: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(fonts.iter().map(PathBuf::as_path)).expect("fonts");
    let project =
        compile(&intent, &style, &AssetLibrary::new(&assets), &measure).expect("compiles");

    let renderer = CpuRenderer::new(&project, &assets).expect("renderer");
    let prof = structural_profile(&project, &renderer, 6).expect("profile");
    let report = lifecycle_report(&project, &prof);
    assert_eq!(report.len(), 3, "one SceneQa per beat");
    for q in &report {
        assert_eq!(q.phases.len(), 7, "{}", q.scene);
        assert!(q.report().starts_with(&format!("{}\n", q.scene)));
    }
    assert_eq!(report, lifecycle_report(&project, &prof), "deterministic");
}
