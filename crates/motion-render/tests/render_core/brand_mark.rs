//! (0.24 M10) The operator brand mark: drawn as the top layer, a backing disc
//! only where its contrast with the picture under it is below 2:1, and a
//! corner that moves away from text.

use std::path::{Path, PathBuf};

use motion_core::scene::{Color, MotionProject};
use motion_render::brand_mark::{disc_strength, ratio, BrandMark, Corner, CROSSFADE_SECONDS};
use resvg::tiny_skia::{Paint, PathBuilder, Pixmap, Transform};

const GOLD: Color = Color {
    r: 244,
    g: 185,
    b: 63,
    a: 255,
};
const NAVY: Color = Color {
    r: 11,
    g: 26,
    b: 44,
    a: 255,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("m10_brand_mark_{name}"));
    std::fs::create_dir_all(&dir).expect("dir");
    dir
}

/// A gold disc on a transparent square, like a channel mark.
fn gold_mark(dir: &Path) -> PathBuf {
    let mut pm = Pixmap::new(120, 120).expect("pixmap");
    let mut p = Paint::default();
    p.set_color_rgba8(GOLD.r, GOLD.g, GOLD.b, 255);
    p.anti_alias = true;
    let mut pb = PathBuilder::new();
    pb.push_circle(60.0, 60.0, 58.0);
    pm.fill_path(
        &pb.finish().expect("circle"),
        &p,
        resvg::tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    let path = dir.join("mark.png");
    pm.save_png(&path).expect("png");
    path
}

fn project(json_scenes: &str) -> MotionProject {
    let text = format!(
        r##"{{"version":"0.2","project":{{"name":"t"}},"canvas":{{"width":1080,"height":1920,"fps":30,"background":"#0B1A2C"}},"assets":[],"scenes":{json_scenes}}}"##
    );
    MotionProject::from_json(&text).expect("project")
}

fn fill(w: u32, h: u32, c: Color) -> Pixmap {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    pm.fill(resvg::tiny_skia::Color::from_rgba8(c.r, c.g, c.b, 255));
    pm
}

#[test]
fn the_guard_ramps_between_1_8_and_2_2() {
    assert_eq!(disc_strength(1.0), 1.0);
    assert_eq!(disc_strength(1.8), 1.0);
    assert_eq!(disc_strength(2.2), 0.0);
    assert_eq!(disc_strength(5.0), 0.0);
    let mid = disc_strength(2.0);
    assert!(mid > 0.3 && mid < 0.7, "{mid}");
    assert!(ratio(1.0, 0.0) > 20.0);
}

#[test]
fn a_gold_mark_on_a_gold_field_gets_a_backing_disc_and_on_navy_does_not() {
    let dir = scratch("guard");
    let mark =
        BrandMark::from_png(&gold_mark(&dir), Corner::Tr, (1080, 1920), 30, NAVY).expect("mark");
    // On gold the contrast is ~1:1: the disc is on and is the navy ground.
    let mut gold = fill(1080, 1920, GOLD);
    assert!(mark.contrast_at(&gold, Corner::Tr) < 1.8);
    let r = mark.rect(Corner::Tr);
    let before = gold
        .pixel(r[0] as u32 + 1, (r[1] + r[3] / 2.0) as u32)
        .expect("px");
    mark.draw(&mut gold, 1.0);
    // The disc reaches past the mark's own box: a point just outside the box
    // but inside 1.3x turns navy-ish.
    let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
    let outside = gold
        .pixel((cx + r[2] * 0.6) as u32, cy as u32)
        .expect("px outside the mark");
    assert!(
        outside.red() < 120,
        "the disc darkens the field behind the mark: {outside:?}, before {before:?}"
    );
    // The mark itself is still drawn on top (gold centre).
    let centre = gold.pixel(cx as u32, cy as u32).expect("centre");
    assert!(centre.red() > 200 && centre.green() > 150, "{centre:?}");
    // On navy the contrast is high: no disc, the surroundings stay navy.
    let mut navy = fill(1080, 1920, NAVY);
    assert!(mark.contrast_at(&navy, Corner::Tr) >= 2.2);
    mark.draw(&mut navy, 1.0);
    let ring = navy.pixel((cx + r[2] * 0.6) as u32, cy as u32).expect("px");
    assert_eq!((ring.red(), ring.green(), ring.blue()), (11, 26, 44));
}

#[test]
fn the_mark_is_on_every_frame_at_the_inset_of_its_corner() {
    let dir = scratch("inset");
    let mark =
        BrandMark::from_png(&gold_mark(&dir), Corner::Br, (1080, 1920), 30, NAVY).expect("mark");
    let r = mark.rect(Corner::Br);
    assert!((r[2] - 0.07 * 1080.0).abs() < 0.01, "7% of the short side");
    assert!(r[0] + r[2] <= 1080.0 - 0.05 * 1080.0 + 0.01);
    assert!(r[1] + r[3] <= 1920.0 - 0.05 * 1080.0 + 0.01);
    for t in [0.0, 0.5, 3.3, 57.0] {
        let mut pm = fill(1080, 1920, NAVY);
        mark.draw(&mut pm, t);
        let (cx, cy) = (r[0] + r[2] / 2.0, r[1] + r[3] / 2.0);
        let p = pm.pixel(cx as u32, cy as u32).expect("px");
        assert!(p.red() > 200, "mark visible at t={t}: {p:?}");
    }
}

#[test]
fn the_corner_moves_off_text_with_one_crossfade_per_beat() {
    let dir = scratch("corner");
    let mut mark =
        BrandMark::from_png(&gold_mark(&dir), Corner::Tr, (1080, 1920), 30, NAVY).expect("mark");
    // Beat 1 has a headline across the top right; beat 2 has none.
    let scenes = r##"[
      {"id":"beat_1","start_seconds":0.0,"duration_seconds":3.0,
       "lifecycle":{"enter":0.0,"settle":0.4,"read":0.8,"evolve":2.0,"anticipate":2.6,"bridge":3.0},
       "layers":[{"id":"b1.headline","x":300,"y":40,"width":700,"height":160,"type":"text",
                  "text":"HEADLINE","font_role":"display","font_size":120,"color":"#FFFFFF"}],
       "motions":[]},
      {"id":"beat_2","start_seconds":3.0,"duration_seconds":3.0,
       "lifecycle":{"enter":0.0,"settle":0.4,"read":0.8,"evolve":2.0,"anticipate":2.6,"bridge":3.0},
       "layers":[{"id":"b2.headline","x":40,"y":40,"width":500,"height":160,"type":"text",
                  "text":"HEADLINE","font_role":"display","font_size":120,"color":"#FFFFFF"}],
       "motions":[]}
    ]"##;
    let p = project(scenes);
    mark.plan_for(&p, Corner::Tr);
    let plan = mark.plan();
    assert_eq!(plan.len(), 2, "{plan:?}");
    assert_ne!(plan[0].corner, Corner::Tr, "beat 1 text sits top right");
    assert_eq!(plan[0].corner, Corner::Tl);
    assert_eq!(plan[1].corner, Corner::Tr, "beat 2 text sits top left");
    assert_eq!(plan[1].start, 3.0);
    // One crossfade at the beat start, then a single corner.
    let mid = mark.corners_at(3.0 + CROSSFADE_SECONDS / 2.0);
    assert_eq!(mid.len(), 2);
    assert!((mid[0].1 + mid[1].1 - 1.0).abs() < 1e-5);
    assert_eq!(mark.corners_at(2.5), vec![(Corner::Tl, 1.0)]);
    assert_eq!(mark.corners_at(3.5), vec![(Corner::Tr, 1.0)]);
}
