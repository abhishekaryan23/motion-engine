//! (0.10 Q) `image_index`: the analysis facts layout QA judges subjects with,
//! built from the images a compiled project draws.

use std::path::{Path, PathBuf};

use motion_core::scene::MotionProject;
use motion_render::image_index;
use resvg::tiny_skia::{Paint, Pixmap, Rect, Transform};
use serde_json::json;

fn scratch(tag: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("image_index")
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

/// A 200x300 cutout: a dark 100x200 block with a head-like knob on top.
fn write_figure(dir: &Path) {
    let mut pm = Pixmap::new(200, 300).expect("pixmap");
    let mut paint = Paint::default();
    paint.set_color_rgba8(40, 30, 20, 255);
    paint.anti_alias = false;
    for (x, y, w, h) in [(50.0, 80.0, 100.0, 200.0), (80.0, 20.0, 40.0, 60.0)] {
        let r = Rect::from_xywh(x, y, w, h).expect("rect");
        pm.fill_rect(r, &paint, Transform::identity(), None);
    }
    pm.save_png(dir.join("fig.png")).expect("png");
}

fn project(layer_id: &str) -> MotionProject {
    serde_json::from_value(json!({
        "version": "0.2",
        "project": {"name": "t", "duration_seconds": 1.0},
        "canvas": {"width": 1080, "height": 1920, "fps": 30, "background": "#ECE3D2"},
        "theme": {"fonts": {}, "palette": {}},
        "assets": [
            {"id": "asset.fig", "type": "image", "path": "fig.png"},
            {"id": "asset.unused", "type": "image", "path": "nope.png"},
        ],
        "scenes": [{
            "id": "beat_1", "start_seconds": 0.0, "duration_seconds": 1.0,
            "layers": [{
                "id": layer_id, "x": 0.0, "y": 0.0, "width": 200.0, "height": 300.0,
                "type": "image", "asset": "asset.fig", "fit": "contain",
            }],
            "motions": [],
        }],
    }))
    .expect("project")
}

#[test]
fn indexes_drawn_images_with_alpha_bounds_head_and_mean_colour() {
    let dir = scratch("basic");
    write_figure(&dir);
    let index = image_index(&project("b1.subject"), &dir);
    let f = index.get("fig.png").expect("facts");
    assert_eq!((f.width, f.height, f.alpha), (200, 300, true));
    // Bounds of the knob + block: x 50..150, y 20..280.
    assert!((f.subject.x - 0.25).abs() < 0.01 && (f.subject.width - 0.5).abs() < 0.01);
    assert!((f.subject.y - 20.0 / 300.0).abs() < 0.01);
    // A person layer gets a head estimate in the top of the silhouette.
    let head = f.head.expect("head estimate");
    assert!(head.y < 0.1 && head.height < 0.35, "{head:?}");
    // Mean colour of the (single-colour) subject.
    assert_eq!(f.mean_color, Some([40, 30, 20]));
    // Unused and missing assets are not indexed.
    assert!(index.get("nope.png").is_none());
}

#[test]
fn only_person_layers_get_a_head() {
    let dir = scratch("object");
    write_figure(&dir);
    // A delivered object (`hero`) has no head region.
    let index = image_index(&project("b1.hero"), &dir);
    let f = index.get("fig.png").expect("facts");
    assert!(f.head.is_none());
}

#[test]
fn an_animated_asset_is_indexed_by_its_first_frame() {
    // Under art direction a library hero becomes a sprite sequence: layout QA
    // still judges it, by the silhouette and colours of frame 1.
    let dir = scratch("sprite");
    std::fs::create_dir_all(dir.join("loops/fig")).expect("mkdir");
    write_figure(&dir.join("loops/fig"));
    std::fs::rename(
        dir.join("loops/fig/fig.png"),
        dir.join("loops/fig/frame_0001.png"),
    )
    .expect("rename");
    let mut p = project("b1.subject");
    p.assets = vec![serde_json::from_value(json!({
        "id": "asset.fig", "type": "sprite_sequence", "path": "loops/fig",
        "sprite": {"frame_count": 2, "fps": 12.0, "mode": "loop"},
    }))
    .expect("asset")];
    let index = image_index(&p, &dir);
    let f = index
        .get("loops/fig")
        .expect("indexed by the sequence path");
    assert_eq!((f.width, f.height, f.alpha), (200, 300, true));
    assert_eq!(f.mean_color, Some([40, 30, 20]));
    assert!(f.head.is_some(), "a person layer keeps its head region");
}

#[test]
fn a_missing_file_is_skipped_not_fatal() {
    let dir = scratch("missing");
    assert!(image_index(&project("b1.subject"), &dir).is_empty());
}

#[test]
fn the_index_follows_asset_root() {
    let dir = scratch("asset_root");
    std::fs::create_dir_all(dir.join("art")).expect("mkdir");
    write_figure(&dir.join("art"));
    let mut p = project("b1.subject");
    p.asset_root = Some("art".into());
    assert!(image_index(&p, &dir).get("fig.png").is_some());
}
