//! Focused asset goldens (0.5 Task G3): generated-image analysis, ingestion
//! and the asset-aware grammars, each compiled at test time with the real
//! `FontMeasure` (same as the CLI) so the goldens track the compiler.
//!
//! * Fixtures and snapshots live in `golden/assets/`:
//!   `worker_cutout.png` (a 540x720 downscale of the external fixture
//!   producer's output for the late-night-worker prompt; short side >= 512 so
//!   it passes the ingestion resolution floor), `worker_cutout.analysis.json`,
//!   `worker_cutout.safe_regions.json` and `hashes.json`.
//! * Hashes are FNV-1a of the raw RGBA frame, keyed
//!   `"{name}.{frame label}@{os}-{arch}"` (other platforms skip the hash
//!   check). `MOTION_UPDATE_GOLDEN=1` rewrites this platform's hashes and the
//!   snapshots, and writes the frames to `output/golden_assets/*.png`.
//! * The library root is the repo `assets/` dir (as in the other asset
//!   tests). Manifest paths resolve against it, so the golden fixture is
//!   referenced as `../golden/assets/worker_cutout.png`; the renderer resolves
//!   it against the same root.
//! * `duotone_asset` and `paper_cutout_asset` (image treatments) already live
//!   in `image_treatment.rs` / `golden/treatments/`; they are not repeated here.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use motion_core::assets::{
    AssetManifest, AssetPromptSet, AssetPromptSpec, AssetRole, AssetStyleProfile, Background,
    Framing, ManifestEntry, MissingAsset, NegativeSpace, NormBox, Presentation, Priority,
    PromptComposition, PromptOutput, ASSET_PROMPTS_VERSION,
};
use motion_core::compiler::{compile_with_assets, AssetLibrary, CompileError, FontSet};
use motion_core::scene::{Layer, LayerKind, TreatmentPreset};
use motion_core::{evaluate_frame, validate, CreativeIntent, MotionProject, StyleProfile};
use motion_render::analysis::{analyze, AnalyzeOptions};
use motion_render::decode::decode_file;
use motion_render::ingest::{ingest, IngestOptions};
use motion_render::{CpuRenderer, FontMeasure, Renderer};
use resvg::tiny_skia::Pixmap;
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// paths, hashing, snapshots
// ---------------------------------------------------------------------------

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The library root: manifest paths and project asset paths resolve against it.
fn assets_dir() -> PathBuf {
    repo().join("assets")
}

fn golden_assets_dir() -> PathBuf {
    repo().join("golden/assets")
}

/// Manifest path of the worker fixture, relative to the library root.
const WORKER_PATH: &str = "../golden/assets/worker_cutout.png";

fn read(rel: &str) -> String {
    let p = repo().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

fn update_mode() -> bool {
    std::env::var("MOTION_UPDATE_GOLDEN").is_ok_and(|v| v == "1")
}

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

/// Serialises read-modify-write of `hashes.json` across parallel tests.
static HASH_LOCK: Mutex<()> = Mutex::new(());

fn hashes_path() -> PathBuf {
    golden_assets_dir().join("hashes.json")
}

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
    std::fs::write(hashes_path(), format!("{{\n{}\n}}\n", body.join(",\n"))).expect("write hashes");
}

fn assert_not_uniform(pm: &Pixmap, what: &str) {
    let data = pm.data();
    assert!(
        data.chunks_exact(4).any(|px| px != &data[..4]),
        "{what}: every pixel identical, nothing drew"
    );
}

/// Check (or, in update mode, record) the hash of one rendered frame and dump
/// the frame for review.
fn check_hash(name: &str, label: &str, pm: &Pixmap) {
    assert_not_uniform(pm, &format!("{name}.{label}"));
    let key = format!("{name}.{label}@{}", platform());
    let hash = format!("{:016x}", fnv1a(pm.data()));
    let _guard = HASH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut hashes = read_hashes();
    if update_mode() {
        let out = repo().join("output/golden_assets");
        std::fs::create_dir_all(&out).expect("mkdir output/golden_assets");
        pm.save_png(out.join(format!("{name}_{label}.png")))
            .expect("write frame png");
        if hashes.get(&key) != Some(&hash) {
            hashes.insert(key, hash);
            write_hashes(&hashes);
        }
    } else if let Some(expected) = hashes.get(&key) {
        assert_eq!(
            expected,
            &hash,
            "{name}.{label}: hash changed on {}. If intentional, rerun with \
             MOTION_UPDATE_GOLDEN=1 and review output/golden_assets/.",
            platform()
        );
    }
    // No entry for this platform: skip silently (rasterization may differ).
}

/// Recursive JSON comparison: numbers within `tol`, everything else exact.
fn json_close(a: &Value, b: &Value, tol: f64, path: &str, errs: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (
                x.as_f64().unwrap_or(f64::NAN),
                y.as_f64().unwrap_or(f64::NAN),
            );
            if (x - y).abs() > tol {
                errs.push(format!("{path}: {x} vs {y}"));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                errs.push(format!("{path}: length {} vs {}", x.len(), y.len()));
                return;
            }
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                json_close(p, q, tol, &format!("{path}[{i}]"), errs);
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => json_close(p, q, tol, &format!("{path}.{k}"), errs),
                    _ => errs.push(format!("{path}.{k}: present on one side only")),
                }
            }
        }
        _ => {
            if a != b {
                errs.push(format!("{path}: {a} vs {b}"));
            }
        }
    }
}

/// Compare `actual` with `golden/assets/<file>` (1e-4 tolerance), or write it.
fn check_snapshot(file: &str, actual: &Value) {
    let path = golden_assets_dir().join(file);
    if update_mode() {
        let text = serde_json::to_string_pretty(actual).expect("json") + "\n";
        std::fs::write(&path, text).expect("write snapshot");
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}. Run with MOTION_UPDATE_GOLDEN=1 to create it.",
            path.display()
        )
    });
    let expected: Value = serde_json::from_str(&text).expect("snapshot parses");
    let mut errs = Vec::new();
    json_close(&expected, actual, 1e-4, "$", &mut errs);
    assert!(
        errs.is_empty(),
        "{file} differs (rerun with MOTION_UPDATE_GOLDEN=1 if intentional):\n{}",
        errs.join("\n")
    );
}

// ---------------------------------------------------------------------------
// compile + render helpers
// ---------------------------------------------------------------------------

fn intent_from(rel: &str) -> CreativeIntent {
    CreativeIntent::from_json(&read(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn style_from(rel: &str, seed: Option<u64>) -> StyleProfile {
    let mut v: Value = serde_json::from_str(&read(rel)).expect("style json");
    if let Some(seed) = seed {
        v["seed"] = json!(seed);
    }
    serde_json::from_value(v).expect("style parses")
}

/// Compile with the real font measurer (fonts chosen by the style, as the CLI does).
fn compile_real(
    intent: &CreativeIntent,
    style: &StyleProfile,
    manifest: &AssetManifest,
) -> Result<MotionProject, CompileError> {
    let assets = assets_dir();
    let font_paths: Vec<PathBuf> = FontSet::for_style(style)
        .faces
        .iter()
        .map(|f| assets.join(f.path))
        .collect();
    let measure =
        FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path)).expect("load fonts");
    compile_with_assets(
        intent,
        style,
        &AssetLibrary::new(&assets),
        &measure,
        manifest,
    )
}

fn compile_ok(
    intent: &CreativeIntent,
    style: &StyleProfile,
    manifest: &AssetManifest,
) -> MotionProject {
    let p = compile_real(intent, style, manifest).expect("compiles");
    validate(&p, Some(&assets_dir())).unwrap_or_else(|e| panic!("compiled scene invalid:\n{e}"));
    p
}

fn render(p: &MotionProject, frame: u32) -> Pixmap {
    let renderer = CpuRenderer::new(p, &assets_dir()).expect("renderer");
    let resolved = evaluate_frame(p, frame).expect("evaluate frame");
    let pm = renderer.render(&resolved).expect("render");
    assert_eq!((pm.width(), pm.height()), (p.canvas.width, p.canvas.height));
    pm
}

/// Frame at `scene`-local `seconds` (clamped into the project).
fn frame_at(p: &MotionProject, scene: &str, seconds: f64) -> u32 {
    let s = p
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .unwrap_or_else(|| panic!("scene {scene}"));
    let f = ((s.start_seconds + seconds) * f64::from(p.canvas.fps)).round() as u32;
    f.min(p.frame_count().saturating_sub(1))
}

/// Frames (name, local seconds) of `scene`: mid-ENTER-to-SETTLE, READ, EVOLVE.
fn lifecycle_frames(p: &MotionProject, scene: &str) -> Vec<(&'static str, u32)> {
    let life = p
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .and_then(|s| s.lifecycle)
        .expect("lifecycle");
    vec![
        (
            "settle",
            frame_at(p, scene, (life.settle + life.read) * 0.5),
        ),
        ("read", frame_at(p, scene, (life.read + life.evolve) * 0.5)),
        (
            "evolve",
            frame_at(p, scene, (life.evolve + life.anticipate) * 0.5),
        ),
    ]
}

fn check_frames(name: &str, p: &MotionProject, frames: &[(&str, u32)]) {
    for (label, frame) in frames {
        let pm = render(p, *frame);
        check_hash(name, label, &pm);
    }
}

fn for_each_layer<'a>(layers: &'a [Layer], f: &mut dyn FnMut(&'a Layer)) {
    for l in layers {
        f(l);
        if let LayerKind::Group { children } = &l.kind {
            for_each_layer(children, f);
        }
    }
}

fn scene_layers<'a>(p: &'a MotionProject, scene: &str) -> Vec<&'a Layer> {
    let s = p
        .scenes
        .iter()
        .find(|s| s.id == scene)
        .unwrap_or_else(|| panic!("scene {scene}"));
    let mut out = Vec::new();
    for_each_layer(&s.layers, &mut |l| out.push(l));
    out
}

fn image_layers<'a>(p: &'a MotionProject, scene: &str) -> Vec<&'a Layer> {
    scene_layers(p, scene)
        .into_iter()
        .filter(|l| matches!(l.kind, LayerKind::Image { .. }))
        .collect()
}

/// Top-left box of a layer (its x/y is the anchor point).
fn bbox(l: &Layer) -> (f32, f32, f32, f32) {
    (
        l.x - l.anchor_x * l.width,
        l.y - l.anchor_y * l.height,
        l.width,
        l.height,
    )
}

/// A manifest entry for a decoded image file, with analysis computed now.
fn entry_for(id: &str, path: &str, file: &Path, person: bool) -> ManifestEntry {
    let img = decode_file(file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    let analysis = analyze(
        &img.pixmap,
        &AnalyzeOptions {
            person,
            ..AnalyzeOptions::default()
        },
    );
    ManifestEntry {
        id: id.into(),
        path: path.into(),
        width: img.pixmap.width(),
        height: img.pixmap.height(),
        alpha: img.has_transparency,
        subject_anchor: Some(motion_core::assets::NormPoint {
            x: analysis.subject_bounds.x + 0.5 * analysis.subject_bounds.width,
            y: analysis.subject_bounds.y + 0.5 * analysis.subject_bounds.height,
        }),
        analysis: Some(analysis),
        ..ManifestEntry::default()
    }
}

fn worker_entry(id: &str, serves: &[&str]) -> ManifestEntry {
    let mut e = entry_for(
        id,
        WORKER_PATH,
        &golden_assets_dir().join("worker_cutout.png"),
        true,
    );
    e.serves = serves.iter().map(|s| (*s).to_string()).collect();
    e
}

fn manifest_of(entries: Vec<ManifestEntry>) -> AssetManifest {
    AssetManifest {
        assets: entries,
        ..AssetManifest::empty()
    }
}

/// Style seeds whose TypeImageInterlock variant puts the subject on the right /
/// left (variant = bit 17 of the plan seed; found by scanning seeds).
const SEED_SUBJECT_RIGHT: u64 = 1;
const SEED_SUBJECT_LEFT: u64 = 2;

fn demo_style(seed: u64) -> StyleProfile {
    style_from("examples/demo_05/late_night_worker.style.json", Some(seed))
}

fn worker_intent() -> CreativeIntent {
    intent_from("golden/fixtures/benchmarks/late-night-worker-01/attempt-01.intent.json")
}

/// Centre x of the beat-1 subject image relative to the canvas centre.
fn subject_is_right(p: &MotionProject) -> bool {
    let imgs = image_layers(p, "beat_1");
    let img = imgs.first().expect("subject image layer");
    let (x, _, w, _) = bbox(img);
    x + w / 2.0 > p.canvas.width as f32 / 2.0
}

// ---------------------------------------------------------------------------
// analysis goldens
// ---------------------------------------------------------------------------

fn worker_analysis() -> motion_core::assets::AssetAnalysis {
    let img = decode_file(&golden_assets_dir().join("worker_cutout.png")).expect("decode worker");
    assert!(img.has_transparency, "fixture must be a real cutout");
    analyze(
        &img.pixmap,
        &AnalyzeOptions {
            person: true,
            ..AnalyzeOptions::default()
        },
    )
}

#[test]
fn asset_alpha_bounds() {
    let a = worker_analysis();
    // Structural sanity independent of the snapshot.
    let b = a.subject_bounds;
    assert!(
        b.x >= 0.0 && b.y >= 0.0 && b.x + b.width <= 1.0 + 1e-4 && b.y + b.height <= 1.0 + 1e-4
    );
    assert!(b.width > 0.3 && b.height > 0.5, "a real subject: {b:?}");
    assert!(
        a.coverage > 0.1 && a.coverage < 0.9,
        "coverage {}",
        a.coverage
    );
    assert!(!a.edges.top, "headroom above the head");

    check_snapshot(
        "worker_cutout.analysis.json",
        &json!({
            "subject_bounds": a.subject_bounds,
            "edges": a.edges,
            "coverage": a.coverage,
            "occupancy": a.occupancy,
        }),
    );
}

#[test]
fn asset_safe_regions() {
    let a = worker_analysis();
    assert!(
        !a.safe_regions.is_empty(),
        "a cutout with empty margins has safe regions"
    );
    // Documented order: score descending.
    for w in a.safe_regions.windows(2) {
        assert!(w[0].score >= w[1].score - 1e-6, "sorted by score");
    }
    // The head estimate sits in the upper part of the image.
    let head = a
        .head_estimate
        .expect("a person cutout has a head estimate");
    assert!(head.y >= 0.0, "{head:?}");
    assert!(
        head.y + head.height <= 0.35,
        "head estimate must lie in the top 35% of the image: {head:?}"
    );

    let regions: Vec<Value> = a
        .safe_regions
        .iter()
        .map(|r| json!({ "name": r.name, "rect": r.rect }))
        .collect();
    check_snapshot(
        "worker_cutout.safe_regions.json",
        &json!({ "safe_regions": regions, "head_estimate": head }),
    );
}

// ---------------------------------------------------------------------------
// render goldens
// ---------------------------------------------------------------------------

#[test]
fn asset_jpeg() {
    let json = r##"{
        "version": "0.2",
        "project": { "name": "asset_jpeg", "duration_seconds": 1.0 },
        "canvas": { "width": 240, "height": 320, "fps": 30, "background": "#ECE3D2" },
        "assets": [ { "id": "env", "type": "image", "path": "test_images/environment.jpg" } ],
        "scenes": [ { "id": "s", "start_seconds": 0.0, "duration_seconds": 1.0, "layers": [
            { "id": "img", "type": "image", "asset": "env", "fit": "cover",
              "x": 0, "y": 0, "width": 240, "height": 320 }
        ] } ]
    }"##;
    let p = MotionProject::from_json(json).expect("project");
    validate(&p, Some(&assets_dir())).unwrap_or_else(|e| panic!("invalid:\n{e}"));
    let a = render(&p, 0);
    let b = render(&p, 0);
    assert!(a.data() == b.data(), "jpeg render is deterministic");
    // Decoded JPEG pixels reach the canvas: opaque and not a flat colour.
    assert!(
        a.data().chunks_exact(4).all(|px| px[3] == 255),
        "fully covered"
    );
    let distinct: std::collections::BTreeSet<&[u8]> =
        a.data().chunks_exact(4).step_by(37).collect();
    assert!(distinct.len() > 16, "a photograph, not a flat fill");
    check_hash("asset_jpeg", "f0", &a);
}

#[test]
fn asset_optional_fallback() {
    let manifest = AssetManifest {
        missing: vec![MissingAsset {
            id: "beat_1.hero_subject".into(),
            priority: Priority::Optional,
            reason: "not delivered".into(),
        }],
        ..AssetManifest::empty()
    };
    let p = compile_ok(&worker_intent(), &demo_style(1), &manifest);
    for s in p.scenes.iter().filter(|s| s.id != "backdrop") {
        assert!(
            image_layers(&p, &s.id).is_empty(),
            "{}: no Image layer",
            s.id
        );
    }
    assert!(
        !p.assets.iter().any(|a| a.id.starts_with("asset.beat_")),
        "no delivered asset registered"
    );
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("asset_optional_fallback", &p, &frames[1..]);
}

#[test]
fn asset_required_failure() {
    let manifest = AssetManifest {
        missing: vec![MissingAsset {
            id: "beat_1.hero_subject".into(),
            priority: Priority::Required,
            reason: "failed decode".into(),
        }],
        ..AssetManifest::empty()
    };
    match compile_real(&worker_intent(), &demo_style(1), &manifest) {
        Err(CompileError::MissingRequiredAsset(msg)) => {
            assert!(msg.contains("beat_1.hero_subject"), "{msg}");
        }
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("a required missing asset must fail before rendering"),
    }
}

// --- ingestion cache -------------------------------------------------------

fn art_style() -> AssetStyleProfile {
    AssetStyleProfile {
        medium: "photograph".into(),
        realism: "natural".into(),
        lighting: "soft daylight".into(),
        contrast: "medium".into(),
        palette_tendency: "neutral".into(),
        edge_treatment: "clean".into(),
        shadow_treatment: "none".into(),
        camera_feel: "documentary".into(),
        background_behavior: "plain".into(),
    }
}

fn worker_spec(stem: &str) -> AssetPromptSpec {
    let mut s = AssetPromptSpec {
        id: "beat_1.hero_subject".into(),
        fingerprint: String::new(),
        continuity_key: "office_worker".into(),
        serves: vec!["beat_1.hero_subject".into()],
        role: AssetRole::HeroSubject,
        priority: Priority::Required,
        subject: "Office worker".into(),
        context: String::new(),
        presentation: Presentation::IsolatedCutout,
        background: Background::Transparent,
        style: art_style(),
        composition: PromptComposition {
            framing: Framing::HalfFigure,
            negative_space: NegativeSpace::None,
            subject_whole: false,
            head_inside_frame: true,
        },
        avoid: vec!["watermark".into()],
        output: PromptOutput {
            file_stem: stem.into(),
            alpha_required: true,
            min_short_side: 512,
            aspect: "3:4".into(),
        },
        prompt: String::new(),
    };
    s.fingerprint = s.compute_fingerprint();
    s
}

#[test]
fn asset_cache_reuse() {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("golden_assets_cache_reuse");
    let _ = std::fs::remove_dir_all(&root);
    let (delivery, empty, out, cache) = (
        root.join("delivery"),
        root.join("empty_delivery"),
        root.join("out"),
        root.join("cache"),
    );
    for d in [&delivery, &empty, &out] {
        std::fs::create_dir_all(d).expect("mkdir");
    }
    let stem = "office_worker-golden0001";
    let prompts = AssetPromptSet {
        version: ASSET_PROMPTS_VERSION.into(),
        specs: vec![worker_spec(stem)],
    };
    let opts = IngestOptions {
        cache_dir: Some(cache.clone()),
        replace: Vec::new(),
    };
    let run = |dir: &Path, opts: &IngestOptions| ingest(&prompts, dir, &out, opts).expect("ingest");
    let index_len = || {
        let text = std::fs::read_to_string(cache.join("index.json")).expect("index.json");
        let v: Value = serde_json::from_str(&text).expect("index parses");
        v["entries"].as_array().expect("entries").len()
    };

    // 1. First delivery.
    std::fs::copy(
        golden_assets_dir().join("worker_cutout.png"),
        delivery.join(format!("{stem}.png")),
    )
    .expect("deliver worker");
    let first = run(&delivery, &opts);
    assert!(!first.report.has_fail(), "{}", first.report.report());
    assert_eq!(first.manifest.assets.len(), 1);
    assert!(first.manifest.missing.is_empty());
    assert_eq!(index_len(), 1);
    let entry = &first.manifest.assets[0];
    assert!(
        entry.path.contains("cache/"),
        "served from the cache: {}",
        entry.path
    );
    assert!(entry.analysis.is_some());

    // 2. Second run: nothing delivered, everything comes from the cache.
    let second = run(&empty, &opts);
    assert!(!second.report.has_fail(), "{}", second.report.report());
    assert_eq!(
        second.manifest.assets, first.manifest.assets,
        "identical manifest assets"
    );
    assert_eq!(second.manifest.assets[0].content_hash, entry.content_hash);
    assert_eq!(second.manifest.assets[0].path, entry.path);
    assert_eq!(index_len(), 1, "still one cache entry");

    // 3. A different image for the same stem is refused without `replace`.
    let other = golden_assets_dir().join("../../assets/test_images/figure.png");
    std::fs::copy(&other, delivery.join(format!("{stem}.png"))).expect("deliver other");
    let refused = run(&delivery, &opts);
    assert!(refused.report.has_fail(), "silent replacement must FAIL");
    assert!(
        refused.report.report().contains("fingerprint"),
        "{}",
        refused.report.report()
    );
    assert!(refused.manifest.assets.is_empty());
    assert_eq!(refused.manifest.missing.len(), 1);
    assert_eq!(index_len(), 1);

    // 4. Explicit replacement by spec id.
    let replaced = run(
        &delivery,
        &IngestOptions {
            cache_dir: Some(cache.clone()),
            replace: vec!["beat_1.hero_subject".into()],
        },
    );
    assert!(!replaced.report.has_fail(), "{}", replaced.report.report());
    assert_eq!(replaced.manifest.assets.len(), 1);
    assert_ne!(
        replaced.manifest.assets[0].content_hash, entry.content_hash,
        "the cached image was replaced"
    );
    assert_eq!(index_len(), 1);
}

// --- TypeImageInterlock ------------------------------------------------------

fn interlock_project(seed: u64) -> MotionProject {
    compile_ok(
        &worker_intent(),
        &demo_style(seed),
        &manifest_of(vec![worker_entry("beat_1.hero_subject", &[])]),
    )
}

fn assert_interlock(p: &MotionProject) {
    let imgs = image_layers(p, "beat_1");
    assert_eq!(imgs.len(), 1, "one subject image");
    assert!(
        matches!(&imgs[0].kind, LayerKind::Image { asset, .. } if asset == "asset.beat_1.hero_subject")
    );
}

#[test]
fn type_image_interlock_right() {
    let p = interlock_project(SEED_SUBJECT_RIGHT);
    assert_interlock(&p);
    assert!(subject_is_right(&p), "subject on the right");
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("type_image_interlock_right", &p, &frames[..2]);
}

#[test]
fn type_image_interlock_left() {
    let p = interlock_project(SEED_SUBJECT_LEFT);
    assert_interlock(&p);
    assert!(!subject_is_right(&p), "subject on the left");
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("type_image_interlock_left", &p, &frames[..2]);
}

#[test]
fn type_image_interlock_head_safe() {
    let mut crossings = 0;
    let mut last = None;
    for seed in [SEED_SUBJECT_RIGHT, SEED_SUBJECT_LEFT] {
        let p = interlock_project(seed);
        assert_interlock(&p);
        let img = image_layers(&p, "beat_1")[0];
        let entry = p
            .assets
            .iter()
            .find(|a| a.id == "asset.beat_1.hero_subject");
        assert!(entry.is_some(), "asset registered");
        let head: NormBox = worker_analysis().head_estimate.expect("head estimate");
        let (ix, iy, iw, ih) = bbox(img);
        let (hx0, hy0) = (ix + head.x * iw, iy + head.y * ih);
        let (hx1, hy1) = (hx0 + head.width * iw, hy0 + head.height * ih);

        let headline: Vec<&Layer> = scene_layers(&p, "beat_1")
            .into_iter()
            .filter(|l| l.id.starts_with("b1.head."))
            .collect();
        assert!(!headline.is_empty(), "seed {seed}: headline present");
        for l in &headline {
            let (lx, ly, lw, lh) = bbox(l);
            let crosses = lx < hx1 && lx + lw > hx0 && ly < hy1 && ly + lh > hy0;
            if crosses {
                crossings += 1;
                assert!(
                    l.z_index < img.z_index,
                    "seed {seed}: {} crosses the head and must sit behind the image ({} vs {})",
                    l.id,
                    l.z_index,
                    img.z_index
                );
            }
        }
        last = Some(p);
    }
    // The interlock is only interesting if type does reach the head; report
    // (do not fail) when the placement keeps every line clear of it.
    eprintln!("head-crossing headline lines across both variants: {crossings}");

    // READ-phase frame of the last variant.
    let p = last.expect("project");
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("type_image_interlock_head_safe", &p, &frames[1..2]);
}

#[test]
fn editorial_cutout() {
    let p = compile_ok(
        &worker_intent(),
        &style_from("examples/editorial_demo.style.json", None),
        &manifest_of(vec![worker_entry("beat_1.hero_subject", &[])]),
    );
    assert_interlock(&p);
    let img = image_layers(&p, "beat_1")[0];
    match &img.kind {
        LayerKind::Image { treatment, .. } => {
            let t = treatment.as_ref().expect("treated image");
            assert_eq!(t.preset, TreatmentPreset::PaperCutout);
        }
        _ => unreachable!(),
    }
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("editorial_cutout", &p, &frames[1..2]);
}

// --- other asset-aware grammars ------------------------------------------------

#[test]
fn hero_object_asset() {
    let entry = entry_for(
        "beat_1.hero_object",
        "test_images/object.png",
        &assets_dir().join("test_images/object.png"),
        false,
    );
    let p = compile_ok(
        &intent_from("golden/hero_object.intent.json"),
        &style_from("examples/public/minimal.style.json", Some(1)),
        &manifest_of(vec![entry]),
    );
    let imgs = image_layers(&p, "beat_1");
    assert_eq!(imgs.len(), 1, "the delivered object is the one image");
    assert!(
        matches!(&imgs[0].kind, LayerKind::Image { asset, .. } if asset == "asset.beat_1.hero_object")
    );
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("hero_object_asset", &p, &frames[1..]);
}

#[test]
fn multiplane_asset() {
    let entry = entry_for(
        "beat_1.environment",
        "test_images/environment.jpg",
        &assets_dir().join("test_images/environment.jpg"),
        false,
    );
    // Parallax language: a beat without a subject image picks the multiplane.
    let mut style = style_from("examples/public/minimal.style.json", Some(1));
    style.motion_language = motion_core::style::MotionLanguage::Parallax;
    let p = compile_ok(
        &intent_from("golden/multiplane.intent.json"),
        &style,
        &manifest_of(vec![entry]),
    );
    let imgs = image_layers(&p, "beat_1");
    assert_eq!(imgs.len(), 1, "only the environment image");
    assert!(
        matches!(&imgs[0].kind, LayerKind::Image { asset, .. } if asset == "asset.beat_1.environment")
    );
    let (cw, ch) = (p.canvas.width as f32, p.canvas.height as f32);
    let (x, y, w, h) = bbox(imgs[0]);
    assert!(
        x <= 0.0 && y <= 0.0 && x + w >= cw && y + h >= ch,
        "full-bleed: {x},{y} {w}x{h} on {cw}x{ch}"
    );
    let frames = lifecycle_frames(&p, "beat_1");
    check_frames("multiplane_asset", &p, &frames[1..]);
}

#[test]
fn shared_asset_carry() {
    let mut intent = intent_from("examples/demo_05/late_night_worker.intent.json");
    intent.beats.truncate(2);
    let manifest = manifest_of(vec![worker_entry(
        "beat_1.hero_subject",
        &["beat_1.hero_subject", "beat_2.hero_subject"],
    )]);
    let p = compile_ok(&intent, &demo_style(1), &manifest);

    let shared: Vec<_> = p
        .shared
        .iter()
        .filter(|e| matches!(&e.layer.kind, LayerKind::Image { .. }))
        .collect();
    assert_eq!(shared.len(), 1, "exactly one shared Image element");
    let scenes: std::collections::BTreeSet<&str> =
        shared[0].track.iter().map(|k| k.scene.as_str()).collect();
    assert!(
        scenes.contains("beat_1") && scenes.contains("beat_2"),
        "{scenes:?}"
    );
    for n in ["beat_1", "beat_2"] {
        assert!(image_layers(&p, n).is_empty(), "{n}: no scene-local copy");
    }
    assert_eq!(
        p.assets.iter().filter(|a| a.path == WORKER_PATH).count(),
        1,
        "one registered asset"
    );

    // A frame inside the bridge between the two scenes.
    let s1 = p.scenes.iter().find(|s| s.id == "beat_1").expect("beat_1");
    let life = s1.lifecycle.expect("lifecycle");
    let mid = (life.bridge + s1.duration_seconds) * 0.5;
    let frame = frame_at(&p, "beat_1", mid);
    check_frames("shared_asset_carry", &p, &[("bridge", frame)]);
}
