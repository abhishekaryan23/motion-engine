//! Render-speed probe (ignored): wall time to render every frame of a few
//! golden scenes on the CPU backend, single-threaded, release build:
//! `cargo test --release -p motion-render --test render_speed -- --ignored --nocapture`.
//! Used to check that renderer additions keep existing scenes within 1.1x.

use std::path::{Path, PathBuf};
use std::time::Instant;

use motion_core::{evaluate_frame, MotionProject};
use motion_render::{CpuRenderer, Renderer};

const FILES: [&str; 4] = [
    "kinetic_poster.motion.json",
    "data_viz.motion.json",
    "multiplane.motion.json",
    "editorial_collage.motion.json",
];

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

#[test]
#[ignore]
fn render_speed_probe() {
    let mut total = 0.0;
    for file in FILES {
        let text = std::fs::read_to_string(golden_dir().join(file)).expect("read");
        let project = MotionProject::from_json(&text).expect("parse");
        let renderer = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
        let t0 = Instant::now();
        for frame in 0..project.frame_count() {
            let resolved = evaluate_frame(&project, frame).expect("evaluate");
            renderer.render(&resolved).expect("render");
        }
        let s = t0.elapsed().as_secs_f64();
        total += s;
        println!("{file}: {} frames {s:.2}s", project.frame_count());
    }
    println!("total {total:.2}s");
}
