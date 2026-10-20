//! Slow-drift sensitivity and composition balance (thirds) in the structural QA.
//!
//! All projects are built in the test; the only external files are the
//! repository fonts.

use std::path::Path;

use motion_core::scene::Phase;
use motion_core::MotionProject;
use motion_render::{
    lifecycle_report, structural_profile, CpuRenderer, MotionProfile, SceneQa, Status,
};

fn repo_root() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

/// One 4 s scene, 540x960 @ 30 fps. READ is [0.8, 3.5).
fn project(layers: &str, motions: &str) -> MotionProject {
    let json = format!(
        r##"{{
        "version": "0.2",
        "project": {{ "name": "drift", "duration_seconds": 4.0 }},
        "canvas": {{ "width": 540, "height": 960, "fps": 30, "background": "#F4F1EA" }},
        "theme": {{ "fonts": {{ "number": "font.anton", "body": "font.fira" }} }},
        "assets": [
            {{ "id": "font.anton", "type": "font", "path": "fonts/Anton-Regular.ttf" }},
            {{ "id": "font.fira", "type": "font", "path": "fonts/FiraSans-Regular.ttf" }}
        ],
        "asset_root": "assets",
        "scenes": [ {{ "id": "s", "start_seconds": 0.0, "duration_seconds": 4.0,
                       "layers": {layers}, "motions": {motions},
                       "lifecycle": {{ "enter": 0.3, "settle": 0.6, "read": 0.8,
                                       "evolve": 3.5, "anticipate": 3.7, "bridge": 4.0 }} }} ]
    }}"##
    );
    MotionProject::from_json(&json).expect("fixture parses")
}

/// Big typography in the top and middle of the canvas.
fn headline(id: &str, y: u32, text: &str) -> String {
    format!(
        r##"{{ "id": "{id}", "type": "text", "x": 20, "y": {y}, "width": 500, "height": 260,
              "text": "{text}", "font_role": "number", "font_size": 200, "color": "#111111",
              "align": "center" }}"##
    )
}

fn qa(p: &MotionProject, step: u32) -> (MotionProfile, SceneQa) {
    let r = CpuRenderer::new(p, repo_root()).expect("renderer");
    let profile = structural_profile(p, &r, step).expect("profile");
    let mut scenes = lifecycle_report(p, &profile);
    assert_eq!(scenes.len(), 1);
    (profile, scenes.remove(0))
}

fn read(qa: &SceneQa) -> &motion_render::PhaseActivity {
    qa.phases
        .iter()
        .find(|a| a.phase == Phase::Read)
        .expect("read phase")
}

fn push(target: &str, to: f32) -> String {
    format!(
        r##"[{{ "target": "{target}", "op": "scale", "start": 0.0, "duration": 3.0,
               "easing": "linear", "from": 1.0, "to": {to} }}]"##
    )
}

fn balanced_layers() -> String {
    format!(
        "[{}, {}, {}]",
        headline("a", 40, "ACE"),
        headline("b", 350, "BAR"),
        headline("c", 660, "CUT")
    )
}

#[test]
fn still_scene_reads_static_not_drift() {
    let p = project(&balanced_layers(), "[]");
    let (profile, q) = qa(&p, 1);
    assert_eq!(profile.accum.len(), profile.diffs.len());
    assert!(
        profile.accum.iter().all(|&a| a == 0.0),
        "{:?}",
        profile.accum
    );
    assert_eq!(read(&q).status, Status::Static);
    assert!(q.longest_static_hold > 2.0, "{}", q.longest_static_hold);
}

#[test]
fn slow_push_reads_drift_and_is_not_a_static_hold() {
    let p = project(&balanced_layers(), &push("b", 1.02));
    let (profile, q) = qa(&p, 1);
    let a = read(&q);
    // Per-frame diffs alone are below the static/low noise floor...
    assert!(a.mean < 0.004, "per-frame mean {} should be quiet", a.mean);
    // ...but the one-second accumulated change is visible.
    assert_eq!(a.status, Status::Drift, "{a:?} accum {:?}", a.accum);
    assert!(a.accum >= 0.001, "accum {}", a.accum);
    assert!(
        // Only the start-up lag (the 1 s window is clamped at the scene start) may read static.
        q.longest_static_hold < 1.0,
        "drift must not count as static: {}s",
        q.longest_static_hold
    );
    assert!(
        !q.warnings.iter().any(|w| w.contains("Dead hold")),
        "{:?}",
        q.warnings
    );
    assert!(q.report().contains("READ structural activity: DRIFT"));
    assert!(profile.accum.iter().any(|&x| x > 0.001));
}

#[test]
fn accumulation_window_scales_with_step() {
    // The 1 s window is `round(fps / step)` samples, so a coarser step still sees the drift.
    let p = project(&balanced_layers(), &push("b", 1.02));
    let (_, q) = qa(&p, 3);
    assert_eq!(read(&q).status, Status::Drift);
}

#[test]
fn fast_motion_still_passes() {
    let mv = |t: &str| {
        format!(
            r##"{{ "target": "{t}", "op": "move", "start": 1.0, "duration": 0.8,
                   "easing": "linear", "from": [0, 0], "to": [400, 0] }}"##
        )
    };
    let p = project(
        &balanced_layers(),
        &format!("[{}, {}, {}]", mv("a"), mv("b"), mv("c")),
    );
    let (_, q) = qa(&p, 1);
    assert_eq!(read(&q).status, Status::Pass, "{:?}", read(&q));
}

#[test]
fn top_only_content_warns_empty_lower_third() {
    let layers = format!("[{}]", headline("a", 40, "ACE"));
    let p = project(&layers, "[]");
    let (profile, q) = qa(&p, 1);
    assert_eq!(profile.scene_thirds.len(), 1);
    assert!(q.thirds[0] > 0.1, "top {:?}", q.thirds);
    assert!(q.thirds[2] < 0.04, "bottom {:?}", q.thirds);
    assert!(
        q.warnings
            .iter()
            .any(|w| w.to_lowercase().contains("empty lower third")),
        "{:?}",
        q.warnings
    );
    assert!(q.report().contains("thirds (top/middle/bottom)"));
}

#[test]
fn spread_content_has_no_lower_third_warning() {
    let p = project(&balanced_layers(), "[]");
    let (_, q) = qa(&p, 1);
    assert!(q.thirds.iter().all(|&t| t > 0.04), "{:?}", q.thirds);
    assert!(
        !q.warnings
            .iter()
            .any(|w| w.to_lowercase().contains("lower third")),
        "{:?}",
        q.warnings
    );
}
