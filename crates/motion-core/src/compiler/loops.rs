//! (0.11 / 0.10 Q) Living library assets under art direction.
//!
//! Library families may ship seamless loops (`<family>/loops/catalog.json`,
//! built by scripts/build_loops.sh): a `hero_loop` animates one catalog asset
//! (its `host`), a `screen_insert` plays inside a host's `screen_box` (e.g. the
//! retro TV). With `--art`, after the project is built, every delivered library
//! image with a hero loop becomes that sprite sequence, and every placed host
//! with a screen box gets the family's screen-insert loop. Pure given the
//! files on disk; without `--art` nothing here runs.

use std::collections::BTreeMap;
use std::path::Path;

use crate::scene::{
    Asset, AssetKind, Fit, Layer, LayerKind, MotionProject, ScreenInsert, SpriteMode, SpriteSpec,
};

#[derive(Debug, Clone, serde::Deserialize)]
struct LoopEntry {
    id: String,
    #[serde(default)]
    host: Option<String>,
    role: String,
    path: String,
    frame_count: u32,
    fps: f64,
    #[serde(default = "default_pattern")]
    pattern: String,
}

fn default_pattern() -> String {
    "frame_%04d.png".to_string()
}

#[derive(Debug, Clone, serde::Deserialize)]
struct LoopCatalog {
    loops: Vec<LoopEntry>,
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
struct ScreenBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// (family, asset id) → (screen box, image width, height) from the family's
/// catalog + manifest.
fn screen_boxes(root: &Path, family: &str) -> BTreeMap<String, (ScreenBox, f32, f32)> {
    let mut out = BTreeMap::new();
    let dir = root.join("library").join(family);
    let Some(cat) = read_json::<serde_json::Value>(&dir.join("catalog.json")) else {
        return out;
    };
    let man = read_json::<serde_json::Value>(&dir.join("manifest.json"));
    for a in cat
        .get("assets")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
    {
        let (Some(id), Some(sb)) = (a.get("id").and_then(|v| v.as_str()), a.get("screen_box"))
        else {
            continue;
        };
        let Ok(sb) = serde_json::from_value::<ScreenBox>(sb.clone()) else {
            continue;
        };
        let dims = man.as_ref().and_then(|m| {
            m.get("assets")?.as_array()?.iter().find_map(|e| {
                (e.get("id")?.as_str()? == format!("library.{id}")).then(|| {
                    (
                        e.get("width").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                        e.get("height").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
                    )
                })
            })
        });
        if let Some((w, h)) = dims.filter(|(w, h)| *w > 0.0 && *h > 0.0) {
            out.insert(id.to_string(), (sb, w, h));
        }
    }
    out
}

fn sprite_asset(id: String, family: &str, l: &LoopEntry) -> Asset {
    Asset {
        id,
        kind: AssetKind::SpriteSequence,
        path: format!("library/{family}/{}", l.path),
        sprite: Some(SpriteSpec {
            frame_count: l.frame_count.max(1),
            fps: l.fps,
            mode: SpriteMode::Loop,
            pattern: l.pattern.clone(),
        }),
    }
}

/// `library/<family>/<stem>.png` → (family, stem).
fn library_image(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("library/")?;
    let (family, file) = rest.split_once('/')?;
    let stem = file.strip_suffix(".png")?;
    (!stem.contains('/')).then_some((family, stem))
}

fn walk_mut(layers: &mut [Layer], f: &mut dyn FnMut(&mut Layer)) {
    for l in layers {
        f(l);
        if let LayerKind::Group { children } = &mut l.kind {
            walk_mut(children, f);
        }
    }
}

/// Apply hero loops and screen inserts for `families`. Returns how many
/// assets were animated and how many inserts were placed.
pub(crate) fn animate_library_assets(
    project: &mut MotionProject,
    root: &Path,
    families: &[String],
) -> (usize, usize) {
    let mut hero: BTreeMap<(String, String), LoopEntry> = BTreeMap::new();
    let mut inserts: BTreeMap<String, LoopEntry> = BTreeMap::new();
    let mut boxes: BTreeMap<String, BTreeMap<String, (ScreenBox, f32, f32)>> = BTreeMap::new();
    for fam in families {
        let Some(cat) =
            read_json::<LoopCatalog>(&root.join("library").join(fam).join("loops/catalog.json"))
        else {
            continue;
        };
        for l in cat.loops {
            if !root.join("library").join(fam).join(&l.path).is_dir() {
                continue;
            }
            match (l.role.as_str(), &l.host) {
                ("hero_loop", Some(h)) => {
                    hero.insert((fam.clone(), h.clone()), l);
                }
                ("screen_insert", _) => {
                    inserts.entry(fam.clone()).or_insert(l);
                }
                _ => {}
            }
        }
        boxes.insert(fam.clone(), screen_boxes(root, fam));
    }

    // 1. Hero loops: the delivered still becomes its loop.
    let mut animated = 0;
    for asset in project.assets.iter_mut() {
        if asset.kind != AssetKind::Image {
            continue;
        }
        let Some((fam, stem)) = library_image(&asset.path) else {
            continue;
        };
        if let Some(l) = hero.get(&(fam.to_string(), stem.to_string())) {
            *asset = sprite_asset(asset.id.clone(), fam, l);
            animated += 1;
        }
    }

    // 2. Screen inserts on hosts with a screen box (still images only).
    let image_paths: BTreeMap<String, String> = project
        .assets
        .iter()
        .filter(|a| a.kind == AssetKind::Image)
        .map(|a| (a.id.clone(), a.path.clone()))
        .collect();
    let mut new_assets: BTreeMap<String, Asset> = BTreeMap::new();
    let mut placed = 0;
    for scene in project.scenes.iter_mut() {
        walk_mut(&mut scene.layers, &mut |layer: &mut Layer| {
            let (w, h) = (layer.width, layer.height);
            let LayerKind::Image {
                asset, insert, fit, ..
            } = &mut layer.kind
            else {
                return;
            };
            if insert.is_some() || *fit != Fit::Contain {
                return;
            }
            let Some(path) = image_paths.get(asset.as_str()) else {
                return;
            };
            let Some((fam, stem)) = library_image(path) else {
                return;
            };
            let (Some(l), Some(&(sb, iw, ih))) =
                (inserts.get(fam), boxes.get(fam).and_then(|b| b.get(stem)))
            else {
                return;
            };
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            // The still is drawn Contain-fit inside the layer box.
            let s = (w / iw).min(h / ih);
            let (dw, dh) = (iw * s, ih * s);
            let (ox, oy) = ((w - dw) / 2.0, (h - dh) / 2.0);
            let id = format!("asset.loop.{}", l.id);
            new_assets
                .entry(id.clone())
                .or_insert_with(|| sprite_asset(id.clone(), fam, l));
            *insert = Some(Box::new(ScreenInsert {
                asset: id,
                screen_box: [
                    (ox + sb.x * dw) / w,
                    (oy + sb.y * dh) / h,
                    sb.width * dw / w,
                    sb.height * dh / h,
                ],
                fit: Fit::Cover,
                playback: None,
            }));
            placed += 1;
        });
    }
    for (_, a) in new_assets {
        if !project.assets.iter().any(|x| x.id == a.id) {
            project.assets.push(a);
        }
    }
    project.assets.sort_by(|a, b| a.id.cmp(&b.id));
    (animated, placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_paths_parse() {
        assert_eq!(
            library_image("library/clay_props_3d/clock.png"),
            Some(("clay_props_3d", "clock"))
        );
        assert_eq!(library_image("fonts/x.ttf"), None);
        assert_eq!(library_image("library/a/b/c.png"), None);
    }
}
