//! (0.10 Q) Image facts for layout QA: what the analysis measures about every
//! raster a compiled project draws, keyed by the project's asset path.
//!
//! `motion_core::layout_report_with` judges text over subjects, subject sizes,
//! loose frames and low contrast, which need alpha bounds, head regions and
//! mean colours; a MotionScene carries no pixels, so the CLI (and tests) build
//! the index here by decoding each image once and running the same
//! [`analyze`](crate::analysis::analyze) the ingestion step used. Pure and
//! deterministic.

use std::collections::BTreeMap;
use std::path::Path;

use motion_core::assets::NormBox;
use motion_core::scene::{sprite_file_name, AssetKind, Layer, LayerKind, MotionProject};
use motion_core::subject_qa::{ImageFacts, ImageIndex};

use crate::analysis::{analyze, AnalyzeOptions};
use crate::decode::decode_file;

/// Visit every layer (scene layers, group children, shared elements).
fn walk<'a>(layers: &'a [Layer], f: &mut dyn FnMut(&'a Layer)) {
    for l in layers {
        f(l);
        if let LayerKind::Group { children } = &l.kind {
            walk(children, f);
        }
    }
}

/// A layer that depicts a person: a beat's hero subject (`<beat>.subject`) or a
/// carried subject image (`shared.asset.*`).
fn is_person_layer(id: &str) -> bool {
    id.starts_with("shared.asset.") || id.rsplit('.').next() == Some("subject")
}

/// Facts for every image asset of `project` that a layer draws. `base_dir` is
/// the directory of the motion file (asset paths resolve against it and the
/// project's `asset_root`, as the renderer does). Images that cannot be read
/// or decoded are left out (the renderer reports them).
pub fn image_index(project: &MotionProject, base_dir: &Path) -> ImageIndex {
    let root = base_dir.join(project.asset_root.as_deref().unwrap_or(""));
    // Asset id -> whether a person layer draws it.
    let mut used: BTreeMap<String, bool> = BTreeMap::new();
    let mut visit = |l: &Layer| {
        if let LayerKind::Image { asset, .. } = &l.kind {
            let person = used.entry(asset.clone()).or_insert(false);
            *person |= is_person_layer(&l.id);
        }
    };
    for scene in &project.scenes {
        walk(&scene.layers, &mut visit);
    }
    for shared in &project.shared {
        walk(std::slice::from_ref(&shared.layer), &mut visit);
    }

    let mut index = ImageIndex::new();
    for asset in &project.assets {
        let Some(&person) = used.get(asset.id.as_str()) else {
            continue;
        };
        // A still, or the first frame of a sprite sequence (an animated library
        // asset keeps the silhouette and colours of the still it replaced).
        let file = match (asset.kind, &asset.sprite) {
            (AssetKind::Image, _) => root.join(&asset.path),
            (AssetKind::SpriteSequence, Some(sprite)) => {
                let Some(name) = sprite_file_name(&sprite.pattern, 1) else {
                    continue;
                };
                root.join(&asset.path).join(name)
            }
            _ => continue,
        };
        let Ok(decoded) = decode_file(&file) else {
            continue;
        };
        let analysis = analyze(
            &decoded.pixmap,
            &AnalyzeOptions {
                person,
                ..AnalyzeOptions::default()
            },
        );
        index.insert(
            asset.path.clone(),
            ImageFacts {
                width: decoded.pixmap.width(),
                height: decoded.pixmap.height(),
                alpha: decoded.has_transparency,
                subject: analysis.subject_bounds,
                head: analysis
                    .head_estimate
                    .filter(|h: &NormBox| person && h.width > 0.0),
                mean_color: analysis.mean_color,
            },
        );
    }
    index
}
