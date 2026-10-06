//! Golden regression tests: render the hand-written scenes in `golden/` on the
//! CPU backend and check that frames are sane, deterministic and (per
//! platform) unchanged.
//!
//! Hashes live in `golden/hashes.json`, keyed `"{file}@{os}-{arch}"`. Set
//! `MOTION_UPDATE_GOLDEN=1` to (re)write the entries for the current platform.
//! Entries for other platforms are ignored: rasterization may differ.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use motion_core::compiler::{compile, AssetLibrary, FontSet};
use motion_core::{evaluate_frame, validate, CreativeIntent, MotionProject, StyleProfile};
use motion_render::{CpuRenderer, FontMeasure, Renderer};
use resvg::tiny_skia::Pixmap;

const GOLDEN_FILES: [&str; 28] = [
    "typography.motion.json",
    "movement.motion.json",
    "masking.motion.json",
    "texture.motion.json",
    "overlap.motion.json",
    "layout_dynamic_center.motion.json",
    "minimal_motion.motion.json",
    "kinetic_typography.motion.json",
    "stagger.motion.json",
    "parallax.motion.json",
    "data_viz.motion.json",
    "collection_accumulate.motion.json",
    "state_change.motion.json",
    "dual_state_change.motion.json",
    "derived_metric.motion.json",
    "derived_metric_compare.motion.json",
    // 0.4: lifecycle, shared motion targets, composition grammars
    "scene_lifecycle.motion.json",
    "read_phase_evolution.motion.json",
    "shared_motion_target.motion.json",
    "editorial_collage.motion.json",
    "split_contrast.motion.json",
    "sequential_stack.motion.json",
    "data_story.motion.json",
    "kinetic_poster.motion.json",
    "multiplane.motion.json",
    "hero_object.motion.json",
    "evidence_stack.motion.json",
    "type_image.motion.json",
];

/// Scenes whose first frame is intentionally an empty canvas (everything
/// enters via a stagger), so the "drew something" check skips frame 0.
const STARTS_BLANK: [&str; 1] = ["stagger.motion.json"];

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

fn assets_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets")
}

fn hashes_path() -> PathBuf {
    golden_dir().join("hashes.json")
}

fn load(file: &str) -> MotionProject {
    let path = golden_dir().join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    MotionProject::from_json(&text).unwrap_or_else(|e| panic!("{file}: {e}"))
}

fn render(project: &MotionProject, renderer: &CpuRenderer, frame: u32) -> Pixmap {
    let resolved = evaluate_frame(project, frame).expect("evaluate frame");
    renderer.render(&resolved).expect("render frame")
}

/// FNV-1a 64-bit.
fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn hash_key(file: &str) -> String {
    format!("{file}@{}", platform())
}

fn frames_to_check(project: &MotionProject) -> [u32; 3] {
    let last = project.frame_count().saturating_sub(1);
    [0, last / 2, last]
}

fn middle_frame(project: &MotionProject) -> u32 {
    frames_to_check(project)[1]
}

/// Reads the flat `{"key": "hex", ...}` object (no JSON crate needed: keys and
/// values are plain strings without escapes, one entry per line).
fn read_hashes() -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(hashes_path()) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter_map(|line| {
            let mut parts = line.trim().trim_end_matches(',').split("\": \"");
            let key = parts.next()?.strip_prefix('"')?;
            let value = parts.next()?.strip_suffix('"')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn write_hashes(hashes: &BTreeMap<String, String>) {
    let body: Vec<String> = hashes
        .iter()
        .map(|(k, v)| format!("  \"{k}\": \"{v}\""))
        .collect();
    let json = format!("{{\n{}\n}}\n", body.join(",\n"));
    std::fs::write(hashes_path(), json).expect("write hashes.json");
}

fn assert_not_uniform(pm: &Pixmap, what: &str) {
    let data = pm.data();
    let first = &data[..4];
    assert!(
        data.chunks_exact(4).any(|px| px != first),
        "{what}: every pixel identical, nothing drew"
    );
}

#[test]
fn golden_files_validate() {
    for file in GOLDEN_FILES {
        let project = load(file);
        validate(&project, Some(&golden_dir())).unwrap_or_else(|e| panic!("{file}:\n{e}"));
    }
}

#[test]
fn golden_frames_render_and_draw_something() {
    for file in GOLDEN_FILES {
        let project = load(file);
        let renderer = CpuRenderer::new(&project, &golden_dir()).unwrap_or_else(|e| {
            panic!("{file}: {e}");
        });
        for frame in frames_to_check(&project) {
            let pm = render(&project, &renderer, frame);
            assert_eq!(
                (pm.width(), pm.height()),
                (project.canvas.width, project.canvas.height),
                "{file} frame {frame}: size"
            );
            // Staggered entrances legitimately start on a blank canvas.
            if frame == 0 && STARTS_BLANK.contains(&file) {
                continue;
            }
            assert_not_uniform(&pm, &format!("{file} frame {frame}"));
        }
    }
}

#[test]
fn golden_rendering_is_deterministic() {
    for file in GOLDEN_FILES {
        let project = load(file);
        let renderer = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
        let frame = middle_frame(&project);
        let a = render(&project, &renderer, frame);
        let b = render(&project, &renderer, frame);
        assert!(a.data() == b.data(), "{file}: same frame rendered twice");

        // A freshly built renderer must agree too (no hidden state).
        let renderer2 = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
        let c = render(&project, &renderer2, frame);
        assert!(a.data() == c.data(), "{file}: new renderer, same frame");
    }
}

#[test]
fn golden_frames_change_over_time() {
    // Sanity that the timeline drives pixels. Typography and texture are static
    // by design, so only the animated scenes are checked.
    for file in [
        "movement.motion.json",
        "masking.motion.json",
        "overlap.motion.json",
        "layout_dynamic_center.motion.json",
        "minimal_motion.motion.json",
        "kinetic_typography.motion.json",
        "stagger.motion.json",
        "parallax.motion.json",
        "data_viz.motion.json",
        "collection_accumulate.motion.json",
        "state_change.motion.json",
        "dual_state_change.motion.json",
        "derived_metric.motion.json",
        "derived_metric_compare.motion.json",
    ] {
        let project = load(file);
        let renderer = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
        let [first, _, last] = frames_to_check(&project);
        let a = render(&project, &renderer, first);
        let b = render(&project, &renderer, last);
        assert!(
            a.data() != b.data(),
            "{file}: first and last frame identical"
        );
    }
}

#[test]
fn golden_hashes_match_or_update() {
    let update = std::env::var("MOTION_UPDATE_GOLDEN").is_ok_and(|v| v == "1");
    let mut hashes = read_hashes();
    let mut changed = false;

    for file in GOLDEN_FILES {
        let project = load(file);
        let renderer = CpuRenderer::new(&project, &golden_dir()).expect("renderer");
        let pm = render(&project, &renderer, middle_frame(&project));
        let hash = format!("{:016x}", fnv1a(pm.data()));
        let key = hash_key(file);

        if update {
            if hashes.get(&key) != Some(&hash) {
                hashes.insert(key, hash);
                changed = true;
            }
        } else if let Some(expected) = hashes.get(&key) {
            assert_eq!(
                expected,
                &hash,
                "{file}: middle-frame hash changed on {}. If intentional, rerun with \
                 MOTION_UPDATE_GOLDEN=1 and review the frames.",
                platform()
            );
        }
        // No entry for this platform: skip silently.
    }

    if update && changed {
        write_hashes(&hashes);
    }
}

#[test]
fn semantic_intent_compiles_and_renders() {
    let intent_path = golden_dir().join("semantic_compile.intent.json");
    let intent = CreativeIntent::from_json(&std::fs::read_to_string(&intent_path).expect("read"))
        .expect("intent parses");
    let style = StyleProfile::default();
    let assets = assets_dir();

    let font_paths: Vec<PathBuf> = FontSet::for_style(&style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    let project =
        compile(&intent, &style, &AssetLibrary::new(&assets), &measure).expect("compiles");

    // asset_root is None, so asset paths resolve against the assets root.
    validate(&project, Some(&assets)).unwrap_or_else(|e| panic!("compiled scene invalid:\n{e}"));

    let renderer = CpuRenderer::new(&project, &assets).expect("renderer");
    let frame = (1.5 * f64::from(project.canvas.fps)).round() as u32;
    let a = render(&project, &renderer, frame);
    assert_eq!(
        (a.width(), a.height()),
        (project.canvas.width, project.canvas.height)
    );
    assert_not_uniform(&a, "semantic compile @1.5s");

    let b = render(&project, &renderer, frame);
    assert!(
        a.data() == b.data(),
        "semantic compile render deterministic"
    );
}
