//! Ingestion + preflight QA (0.5). Prompt sets are built by hand and analysis
//! values are supplied by hand where a check reads them, so these tests do not
//! depend on the analysis or prompt-derivation implementations.

use std::path::PathBuf;

use motion_core::assets::*;
use motion_render::asset_qa::{
    check_entry, validate_manifest, AssetCheck, AssetQa, AssetQaReport, CheckLevel,
};
use motion_render::decode::decode_bytes;
use motion_render::ingest::{ingest, IngestOptions};
use resvg::tiny_skia::{Color, Pixmap};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

const PERSON_PNG: &str = "cutout_person_768x1024.png";
const GRADIENT_JPG: &str = "gradient_640x800.jpg";

/// A fresh scratch tree: `<root>/delivery`, `<root>/out`.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("asset_ingest")
            .join(tag);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("delivery")).expect("mkdir");
        std::fs::create_dir_all(root.join("out")).expect("mkdir");
        Scratch { root }
    }
    fn delivery(&self) -> PathBuf {
        self.root.join("delivery")
    }
    fn out(&self) -> PathBuf {
        self.root.join("out")
    }
    fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
    fn deliver_fixture(&self, fixture_name: &str, as_name: &str) {
        std::fs::copy(fixture(fixture_name), self.delivery().join(as_name)).expect("copy");
    }
    fn deliver_bytes(&self, bytes: &[u8], as_name: &str) {
        std::fs::write(self.delivery().join(as_name), bytes).expect("write");
    }
}

fn style() -> AssetStyleProfile {
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

/// A hand-built spec. `alpha` = isolated cutout on transparency; otherwise a
/// full-frame opaque photo.
fn spec(id: &str, stem: &str, priority: Priority, alpha: bool) -> AssetPromptSpec {
    let mut s = AssetPromptSpec {
        id: id.into(),
        fingerprint: String::new(),
        continuity_key: "office_worker".into(),
        serves: vec![id.into()],
        role: AssetRole::HeroSubject,
        priority,
        subject: format!("subject of {id}"),
        context: String::new(),
        presentation: if alpha {
            Presentation::IsolatedCutout
        } else {
            Presentation::FullFrame
        },
        background: if alpha {
            Background::Transparent
        } else {
            Background::Opaque
        },
        style: style(),
        composition: PromptComposition {
            framing: Framing::HalfFigure,
            negative_space: NegativeSpace::None,
            subject_whole: false,
            head_inside_frame: true,
        },
        avoid: vec!["watermark".into()],
        output: PromptOutput {
            file_stem: stem.into(),
            alpha_required: alpha,
            min_short_side: 512,
            aspect: "3:4".into(),
        },
        prompt: String::new(),
    };
    s.fingerprint = s.compute_fingerprint();
    s
}

fn set(specs: Vec<AssetPromptSpec>) -> AssetPromptSet {
    AssetPromptSet {
        version: ASSET_PROMPTS_VERSION.into(),
        specs,
    }
}

fn run(
    s: &Scratch,
    prompts: &AssetPromptSet,
    opts: &IngestOptions,
) -> motion_render::ingest::IngestOutcome {
    ingest(prompts, &s.delivery(), &s.out(), opts).expect("ingest")
}

fn check<'a>(report: &'a AssetQaReport, id: &str, name: &str) -> Option<&'a AssetCheck> {
    report
        .assets
        .iter()
        .find(|a| a.id == id)?
        .checks
        .iter()
        .find(|c| c.name == name)
}

fn small_png(w: u32, h: u32) -> Vec<u8> {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    pm.fill(Color::from_rgba8(10, 200, 10, 255));
    pm.pixels_mut()[0] = resvg::tiny_skia::PremultipliedColorU8::TRANSPARENT;
    pm.encode_png().expect("png")
}

/// A transparent image with a centered opaque block (a plausible cutout).
fn block_cutout_png(w: u32, h: u32) -> Vec<u8> {
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    let mut paint = resvg::tiny_skia::Paint::default();
    paint.set_color_rgba8(90, 40, 200, 255);
    let rect = resvg::tiny_skia::Rect::from_xywh(
        w as f32 * 0.3,
        h as f32 * 0.1,
        w as f32 * 0.4,
        h as f32 * 0.7,
    )
    .expect("rect");
    pm.fill_rect(rect, &paint, resvg::tiny_skia::Transform::identity(), None);
    pm.encode_png().expect("png")
}

fn empty_analysis() -> AssetAnalysis {
    AssetAnalysis {
        subject_bounds: NormBox {
            x: 0.2,
            y: 0.1,
            width: 0.6,
            height: 0.8,
        },
        edges: EdgeContact::default(),
        coverage: 0.4,
        occupancy: OccupancyGrid {
            cols: 16,
            rows: (0..16).map(|_| "0".repeat(16)).collect(),
        },
        safe_regions: Vec::new(),
        head_estimate: None,
        monochrome: false,
        mean_color: None,
    }
}

fn entry_for(name: &str, id: &str) -> (ManifestEntry, Vec<u8>) {
    let bytes = std::fs::read(fixture(name)).expect("fixture");
    let img = decode_bytes(&bytes).expect("decode");
    let entry = ManifestEntry {
        id: id.into(),
        path: name.into(),
        width: img.pixmap.width(),
        height: img.pixmap.height(),
        alpha: img.has_transparency,
        analysis: Some(empty_analysis()),
        ..ManifestEntry::default()
    };
    (entry, bytes)
}

// ---------------------------------------------------------------------------
// ingest
// ---------------------------------------------------------------------------

#[test]
fn delivered_png_becomes_a_manifest_entry() {
    let s = Scratch::new("delivered_png");
    let mut sp = spec(
        "beat_1.hero_subject",
        "worker-aaaa",
        Priority::Required,
        true,
    );
    sp.serves = vec!["beat_1.hero_subject".into(), "beat_3.hero_subject".into()];
    sp.fingerprint = sp.compute_fingerprint();
    s.deliver_fixture(PERSON_PNG, "worker-aaaa.png");
    let out = run(&s, &set(vec![sp.clone()]), &IngestOptions::default());

    assert!(!out.report.has_fail(), "{}", out.report.report());
    assert_eq!(out.manifest.version, ASSET_MANIFEST_VERSION);
    assert!(out.manifest.missing.is_empty());
    assert_eq!(out.manifest.assets.len(), 1);
    let e = &out.manifest.assets[0];
    assert_eq!(e.id, "beat_1.hero_subject");
    assert_eq!(e.serves, sp.serves);
    assert_eq!(e.fingerprint.as_deref(), Some(sp.fingerprint.as_str()));
    assert_eq!(e.continuity_key.as_deref(), Some("office_worker"));
    let bytes = std::fs::read(fixture(PERSON_PNG)).expect("fixture");
    assert_eq!(
        e.content_hash.as_deref(),
        Some(content_hash(&bytes).as_str())
    );
    assert!(e
        .content_hash
        .as_deref()
        .unwrap_or("")
        .starts_with("fnv1a64:"));
    assert_eq!((e.width, e.height), (768, 1024));
    assert!(e.alpha);
    assert!(e.analysis.is_some());
    assert!(e.subject_anchor.is_some());
    // Relative to the manifest dir (<root>/out): ../delivery/worker-aaaa.png
    assert_eq!(e.path, "../delivery/worker-aaaa.png");
    assert!(out.manifest.validate().is_ok());
    // Both served requests resolve to the entry.
    assert!(out.manifest.get("beat_3.hero_subject").is_some());
    assert!(out.cache.is_none());
}

#[test]
fn jpeg_for_opaque_spec_is_ok() {
    let s = Scratch::new("jpeg_ok");
    let sp = spec(
        "beat_2.environment",
        "plate-bbbb",
        Priority::Optional,
        false,
    );
    s.deliver_fixture(GRADIENT_JPG, "plate-bbbb.jpg");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    assert!(!out.report.has_fail(), "{}", out.report.report());
    assert_eq!(out.manifest.assets.len(), 1);
    assert!(out.manifest.missing.is_empty());
    let e = &out.manifest.assets[0];
    assert!(!e.alpha);
    assert_eq!((e.width, e.height), (640, 800));
    assert!(e.path.ends_with("plate-bbbb.jpg"));
    assert_eq!(
        check(&out.report, "beat_2.environment", "decode").map(|c| c.message.as_str()),
        Some("jpeg 640×800")
    );
}

#[test]
fn jpeg_for_alpha_required_spec_is_excluded() {
    let s = Scratch::new("jpeg_alpha");
    let req = spec(
        "beat_1.hero_subject",
        "worker-cccc",
        Priority::Required,
        true,
    );
    let opt = spec(
        "beat_2.hero_subject",
        "worker-dddd",
        Priority::Optional,
        true,
    );
    s.deliver_fixture(GRADIENT_JPG, "worker-cccc.jpg");
    s.deliver_fixture(GRADIENT_JPG, "worker-dddd.jpg");
    let out = run(&s, &set(vec![req, opt]), &IngestOptions::default());

    assert!(out.manifest.assets.is_empty());
    assert_eq!(out.manifest.missing.len(), 2);
    assert_eq!(out.manifest.missing[0].id, "beat_1.hero_subject");
    assert_eq!(out.manifest.missing[0].priority, Priority::Required);
    assert!(out.manifest.missing[0].reason.starts_with("alpha:"));
    assert_eq!(out.manifest.missing[1].priority, Priority::Optional);
    // Required: hard FAIL on `alpha`. Optional: reported, but does not fail the run.
    let c = check(&out.report, "beat_1.hero_subject", "alpha").expect("alpha check");
    assert_eq!(c.level, CheckLevel::Fail);
    assert_eq!(out.report.assets[0].level(), CheckLevel::Fail);
    assert_eq!(out.report.assets[1].level(), CheckLevel::Warning);
    assert!(out.report.has_fail());
}

#[test]
fn optional_missing_warns_required_missing_fails() {
    let s = Scratch::new("missing");
    let opt = spec(
        "beat_1.environment",
        "plate-eeee",
        Priority::Optional,
        false,
    );
    let out = run(&s, &set(vec![opt.clone()]), &IngestOptions::default());
    assert!(out.manifest.assets.is_empty());
    assert_eq!(
        out.manifest.missing,
        vec![MissingAsset {
            id: "beat_1.environment".into(),
            priority: Priority::Optional,
            reason: "not delivered".into()
        }]
    );
    assert!(!out.report.has_fail());
    let c = check(&out.report, "beat_1.environment", "delivery").expect("delivery");
    assert_eq!(c.level, CheckLevel::Warning);
    assert!(c.message.contains("image-free composition"));

    let req = spec(
        "beat_2.hero_subject",
        "worker-ffff",
        Priority::Required,
        true,
    );
    let out = run(&s, &set(vec![opt, req]), &IngestOptions::default());
    assert!(out.report.has_fail());
    assert_eq!(out.manifest.missing_required().len(), 1);
    let c = check(&out.report, "beat_2.hero_subject", "delivery").expect("delivery");
    assert_eq!(c.level, CheckLevel::Fail);
}

#[test]
fn sidecar_metadata_is_copied() {
    let s = Scratch::new("sidecar");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-1111",
        Priority::Required,
        true,
    );
    s.deliver_fixture(PERSON_PNG, "worker-1111.png");
    let sidecar = GeneratedSidecar {
        fingerprint: Some(sp.fingerprint.clone()),
        face_bounds: Some(NormBox {
            x: 0.4,
            y: 0.14,
            width: 0.2,
            height: 0.16,
        }),
        head_bounds: Some(NormBox {
            x: 0.36,
            y: 0.11,
            width: 0.28,
            height: 0.23,
        }),
        face_anchor: Some(NormPoint { x: 0.5, y: 0.22 }),
        subject_anchor: Some(NormPoint { x: 0.5, y: 0.5 }),
        generator: {
            let mut m = serde_json::Map::new();
            m.insert("vendor".into(), serde_json::json!("acme"));
            m.insert("seed".into(), serde_json::json!(7));
            m
        },
    };
    s.deliver_bytes(
        serde_json::to_string_pretty(&sidecar)
            .expect("json")
            .as_bytes(),
        "worker-1111.json",
    );
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    assert!(!out.report.has_fail(), "{}", out.report.report());
    let e = &out.manifest.assets[0];
    assert_eq!(e.face_bounds, sidecar.face_bounds);
    assert_eq!(e.head_bounds, sidecar.head_bounds);
    assert_eq!(e.face_anchor, sidecar.face_anchor);
    assert_eq!(e.subject_anchor, sidecar.subject_anchor);
    assert_eq!(e.generator, sidecar.generator);
}

#[test]
fn sidecar_fingerprint_mismatch_fails() {
    let s = Scratch::new("sidecar_fp");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-2222",
        Priority::Required,
        true,
    );
    s.deliver_fixture(PERSON_PNG, "worker-2222.png");
    s.deliver_bytes(
        br#"{"fingerprint":"fp1-0000000000000000"}"#,
        "worker-2222.json",
    );
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    assert!(out.report.has_fail());
    assert!(out.manifest.assets.is_empty());
    let c = check(&out.report, "beat_1.hero_subject", "fingerprint").expect("fingerprint");
    assert_eq!(c.level, CheckLevel::Fail);
    assert_eq!(out.manifest.missing.len(), 1);
}

#[test]
fn two_files_for_one_stem_fail() {
    let s = Scratch::new("two_files");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-3333",
        Priority::Required,
        true,
    );
    s.deliver_fixture(PERSON_PNG, "worker-3333.png");
    s.deliver_fixture(GRADIENT_JPG, "worker-3333.jpg");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    assert!(out.manifest.assets.is_empty());
    let c = check(&out.report, "beat_1.hero_subject", "delivery").expect("delivery");
    assert_eq!(c.level, CheckLevel::Fail);
    assert!(c.message.contains("worker-3333.png") && c.message.contains("worker-3333.jpg"));
    assert!(out.report.has_fail());
}

#[test]
fn too_small_image_fails_resolution() {
    let s = Scratch::new("small");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-4444",
        Priority::Required,
        true,
    );
    s.deliver_bytes(&small_png(300, 400), "worker-4444.png");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    let c = check(&out.report, "beat_1.hero_subject", "resolution").expect("resolution");
    assert_eq!(c.level, CheckLevel::Fail);
    assert!(out.manifest.assets.is_empty());
    assert!(out.manifest.missing[0].reason.starts_with("resolution:"));
}

#[test]
fn corrupt_delivery_fails_decode() {
    let s = Scratch::new("corrupt");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-5555",
        Priority::Required,
        true,
    );
    s.deliver_bytes(b"this is not an image", "worker-5555.png");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    let c = check(&out.report, "beat_1.hero_subject", "decode").expect("decode");
    assert_eq!(c.level, CheckLevel::Fail);
    assert!(out.manifest.assets.is_empty());
}

#[test]
fn ingest_is_deterministic_and_keeps_spec_order() {
    let s = Scratch::new("determinism");
    let a = spec(
        "beat_1.hero_subject",
        "worker-6661",
        Priority::Required,
        true,
    );
    let b = spec(
        "beat_2.environment",
        "plate-6662",
        Priority::Optional,
        false,
    );
    let c = spec(
        "beat_3.environment",
        "plate-6663",
        Priority::Optional,
        false,
    );
    s.deliver_fixture(PERSON_PNG, "worker-6661.png");
    s.deliver_fixture(GRADIENT_JPG, "plate-6662.jpg");
    let prompts = set(vec![a, b, c]);
    let first = run(&s, &prompts, &IngestOptions::default());
    let second = run(&s, &prompts, &IngestOptions::default());
    let j1 = serde_json::to_string_pretty(&first.manifest).expect("json");
    let j2 = serde_json::to_string_pretty(&second.manifest).expect("json");
    assert_eq!(j1, j2);
    assert_eq!(first.report, second.report);
    let ids: Vec<_> = first
        .manifest
        .assets
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    assert_eq!(ids, ["beat_1.hero_subject", "beat_2.environment"]);
    assert_eq!(first.manifest.missing[0].id, "beat_3.environment");
}

// --- cache (needs Task A's AssetCacheIndex::insert) --------------------------

#[test]
fn cache_copies_file_and_writes_sorted_index() {
    let s = Scratch::new("cache_basic");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-7771",
        Priority::Required,
        true,
    );
    s.deliver_fixture(PERSON_PNG, "worker-7771.png");
    let opts = IngestOptions {
        cache_dir: Some(s.cache()),
        replace: Vec::new(),
    };
    let out = run(&s, &set(vec![sp.clone()]), &opts);
    assert!(!out.report.has_fail(), "{}", out.report.report());
    let stem = sp.fingerprint.strip_prefix("fp1-").expect("prefix");
    let cached = s.cache().join(format!("{stem}.png"));
    assert!(cached.is_file());
    assert!(s.cache().join("index.json").is_file());
    let e = &out.manifest.assets[0];
    assert_eq!(e.path, format!("../cache/{stem}.png"));
    let index = AssetCacheIndex::from_json(
        &std::fs::read_to_string(s.cache().join("index.json")).expect("index"),
    )
    .expect("parse");
    assert_eq!(index.entries.len(), 1);
    assert_eq!(index.entries[0].fingerprint, sp.fingerprint);
    assert_eq!(
        index.entries[0].content_hash,
        *e.content_hash.as_ref().expect("hash")
    );
}

#[test]
fn cache_reuses_undelivered_and_refuses_silent_replacement() {
    let s = Scratch::new("cache_reuse");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-8881",
        Priority::Required,
        true,
    );
    let prompts = set(vec![sp.clone()]);
    s.deliver_fixture(PERSON_PNG, "worker-8881.png");
    let opts = IngestOptions {
        cache_dir: Some(s.cache()),
        replace: Vec::new(),
    };
    let first = run(&s, &prompts, &opts);
    assert!(!first.report.has_fail());

    // Delivery removed: the cached image serves the same fingerprint.
    std::fs::remove_file(s.delivery().join("worker-8881.png")).expect("rm");
    let reused = run(&s, &prompts, &opts);
    assert!(!reused.report.has_fail(), "{}", reused.report.report());
    assert_eq!(reused.manifest.assets.len(), 1);
    assert_eq!(
        reused.manifest.assets[0].content_hash,
        first.manifest.assets[0].content_hash
    );

    // Different bytes for the same fingerprint: refused without --replace.
    s.deliver_bytes(&block_cutout_png(700, 900), "worker-8881.png");
    let refused = run(&s, &prompts, &opts);
    assert!(refused.report.has_fail());
    let c = refused.report.assets[0]
        .checks
        .iter()
        .find(|c| c.name == "fingerprint" && c.level == CheckLevel::Fail)
        .expect("fingerprint FAIL");
    assert!(c.message.contains("--replace"));

    // Explicit replacement by spec id.
    let replaced = run(
        &s,
        &prompts,
        &IngestOptions {
            cache_dir: Some(s.cache()),
            replace: vec!["beat_1.hero_subject".into()],
        },
    );
    assert!(!replaced.report.has_fail(), "{}", replaced.report.report());
    assert_ne!(
        replaced.manifest.assets[0].content_hash,
        first.manifest.assets[0].content_hash
    );
}

// ---------------------------------------------------------------------------
// asset_qa
// ---------------------------------------------------------------------------

#[test]
fn report_text_format() {
    let r = AssetQaReport {
        assets: vec![
            AssetQa {
                id: "beat_1.hero_subject".into(),
                checks: vec![
                    AssetCheck::pass("resolution", "1024×1365 (min 1024)"),
                    AssetCheck::warn("occupancy", "subject occupies 78% width"),
                ],
            },
            AssetQa {
                id: "beat_2.environment".into(),
                checks: vec![AssetCheck::fail("delivery", "required image missing")],
            },
        ],
    };
    assert_eq!(
        r.report(),
        "beat_1.hero_subject  WARNING\n\
         \x20 PASS     resolution: 1024×1365 (min 1024)\n\
         \x20 WARNING  occupancy: subject occupies 78% width\n\
         beat_2.environment  FAIL\n\
         \x20 FAIL     delivery: required image missing\n\
         assets: 2 (1 fail, 1 warning)\n"
    );
}

/// Write a manifest tree on disk and validate it.
fn validate_tree(
    tag: &str,
    manifest: &AssetManifest,
    prompts: Option<&AssetPromptSet>,
) -> AssetQaReport {
    let s = Scratch::new(tag);
    for name in [PERSON_PNG, GRADIENT_JPG] {
        std::fs::copy(fixture(name), s.out().join(name)).expect("copy");
    }
    validate_manifest(manifest, &s.out(), prompts)
}

fn manifest_of(assets: Vec<ManifestEntry>) -> AssetManifest {
    AssetManifest {
        version: ASSET_MANIFEST_VERSION.into(),
        assets,
        missing: Vec::new(),
    }
}

#[test]
fn manifest_size_mismatch_fails() {
    let (mut e, _) = entry_for(PERSON_PNG, "a");
    e.width = 999;
    let report = validate_tree("qa_size", &manifest_of(vec![e]), None);
    let c = check(&report, "a", "manifest").expect("manifest check");
    assert_eq!(c.level, CheckLevel::Fail);
    assert!(c.message.contains("999"));
    assert!(report.has_fail());
}

#[test]
fn duplicate_ids_and_served_requests_fail() {
    let (a, _) = entry_for(PERSON_PNG, "a");
    let (dup, _) = entry_for(GRADIENT_JPG, "a");
    let report = validate_tree("qa_dup", &manifest_of(vec![a.clone(), dup]), None);
    assert!(report.has_fail());
    let dupe = report
        .assets
        .iter()
        .flat_map(|q| q.checks.iter())
        .find(|c| c.name == "duplicate")
        .expect("duplicate check");
    assert_eq!(dupe.level, CheckLevel::Fail);

    let mut x = a.clone();
    x.id = "x".into();
    x.serves = vec!["shared".into()];
    let mut y = a;
    y.id = "y".into();
    y.serves = vec!["shared".into()];
    let report = validate_tree("qa_dup2", &manifest_of(vec![x, y]), None);
    assert!(report
        .assets
        .iter()
        .flat_map(|q| q.checks.iter())
        .any(|c| c.name == "duplicate" && c.level == CheckLevel::Fail));
}

#[test]
fn missing_entries_map_to_delivery_levels() {
    let (a, _) = entry_for(PERSON_PNG, "a");
    let mut m = manifest_of(vec![a]);
    m.missing = vec![
        MissingAsset {
            id: "opt".into(),
            priority: Priority::Optional,
            reason: "not delivered".into(),
        },
        MissingAsset {
            id: "req".into(),
            priority: Priority::Required,
            reason: "not delivered".into(),
        },
    ];
    let report = validate_tree("qa_missing", &m, None);
    assert_eq!(
        check(&report, "opt", "delivery").map(|c| c.level),
        Some(CheckLevel::Warning)
    );
    assert_eq!(
        check(&report, "req", "delivery").map(|c| c.level),
        Some(CheckLevel::Fail)
    );
    assert!(report.has_fail());

    m.missing.remove(1);
    let report = validate_tree("qa_missing2", &m, None);
    assert!(!report.has_fail(), "{}", report.report());
    assert!(report.report().contains("WARNING  delivery:"));
}

#[test]
fn missing_file_and_unknown_prompt_id() {
    let (mut e, _) = entry_for(PERSON_PNG, "not-in-prompts");
    e.path = "gone.png".into();
    let sp = spec("other", "worker-9991", Priority::Optional, true);
    let report = validate_tree("qa_file", &manifest_of(vec![e]), Some(&set(vec![sp])));
    assert_eq!(
        check(&report, "not-in-prompts", "file").map(|c| c.level),
        Some(CheckLevel::Fail)
    );
    assert_eq!(
        check(&report, "not-in-prompts", "manifest").map(|c| c.level),
        Some(CheckLevel::Warning)
    );
    // The optional spec never appears in the manifest: WARNING delivery.
    assert_eq!(
        check(&report, "other", "delivery").map(|c| c.level),
        Some(CheckLevel::Warning)
    );
}

#[test]
fn validate_manifest_round_trips_an_ingested_manifest() {
    let s = Scratch::new("qa_roundtrip");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-9001",
        Priority::Required,
        true,
    );
    s.deliver_fixture(PERSON_PNG, "worker-9001.png");
    let prompts = set(vec![sp]);
    let out = run(&s, &prompts, &IngestOptions::default());
    let report = validate_manifest(&out.manifest, &s.out(), Some(&prompts));
    assert!(!report.has_fail(), "{}", report.report());
    // The stored fingerprint and content hash are re-checked against the prompt set and bytes.
    assert_eq!(
        check(&report, "beat_1.hero_subject", "fingerprint").map(|c| c.level),
        Some(CheckLevel::Pass)
    );
    assert!(!report.report().contains("changed since ingestion"));
}

// --- check_entry thresholds with hand-supplied analysis ------------------------

fn level_of(checks: &[AssetCheck], name: &str) -> Option<CheckLevel> {
    checks.iter().find(|c| c.name == name).map(|c| c.level)
}

#[test]
fn check_entry_thresholds() {
    let (mut e, bytes) = entry_for(PERSON_PNG, "a");
    let img = decode_bytes(&bytes).expect("decode");
    let mut sp = spec("a", "worker-x", Priority::Required, true);
    sp.output.min_short_side = 1024; // image short side is 768
    sp.composition.subject_whole = true;

    let checks = check_entry(&e, Some(&sp), Some(&img));
    assert_eq!(level_of(&checks, "resolution"), Some(CheckLevel::Warning));
    assert_eq!(level_of(&checks, "alpha"), Some(CheckLevel::Pass));
    assert_eq!(level_of(&checks, "edges"), Some(CheckLevel::Pass));
    assert_eq!(level_of(&checks, "occupancy"), Some(CheckLevel::Pass));
    assert_eq!(level_of(&checks, "safe_region"), Some(CheckLevel::Pass));

    // Wide subject, touching edges, head too close to the top, off-subject face box.
    let mut a = empty_analysis();
    a.subject_bounds = NormBox {
        x: 0.05,
        y: 0.0,
        width: 0.93,
        height: 1.0,
    };
    a.edges = EdgeContact {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
    a.head_estimate = Some(NormBox {
        x: 0.4,
        y: 0.01,
        width: 0.2,
        height: 0.2,
    });
    e.analysis = Some(a);
    e.face_bounds = Some(NormBox {
        x: 0.0,
        y: 0.0,
        width: 0.05,
        height: 0.05,
    });
    let checks = check_entry(&e, Some(&sp), Some(&img));
    assert_eq!(level_of(&checks, "edges"), Some(CheckLevel::Warning));
    assert_eq!(level_of(&checks, "occupancy"), Some(CheckLevel::Warning));
    assert!(checks
        .iter()
        .any(|c| c.name == "occupancy" && c.message.contains("subject occupies 93% width")));
    assert_eq!(
        level_of(&checks, "face_metadata"),
        Some(CheckLevel::Warning)
    );
    assert!(!checks.iter().any(|c| c.level == CheckLevel::Fail));

    // Head from the estimate near the top (no supplied head/face box).
    e.face_bounds = None;
    let checks = check_entry(&e, Some(&sp), Some(&img));
    assert_eq!(
        level_of(&checks, "head_headroom"),
        Some(CheckLevel::Warning)
    );

    // Fingerprint mismatch fails.
    e.fingerprint = Some("fp1-deadbeefdeadbeef".into());
    let checks = check_entry(&e, Some(&sp), Some(&img));
    assert_eq!(level_of(&checks, "fingerprint"), Some(CheckLevel::Fail));

    // No decoded image: decode FAIL.
    let checks = check_entry(&e, Some(&sp), None);
    assert_eq!(level_of(&checks, "decode"), Some(CheckLevel::Fail));
}

#[test]
fn check_entry_aspect_and_alpha() {
    let entry = |w: u32, h: u32| ManifestEntry {
        id: "a".into(),
        path: "a.png".into(),
        width: w,
        height: h,
        alpha: true,
        analysis: Some(empty_analysis()),
        ..ManifestEntry::default()
    };
    let sp = spec("a", "worker-y", Priority::Optional, true);
    // Nearly every pixel opaque: not a cutout (alpha FAIL), and an extreme strip.
    let img = decode_bytes(&small_png(3200, 600)).expect("decode");
    let checks = check_entry(&entry(3200, 600), Some(&sp), Some(&img));
    assert_eq!(level_of(&checks, "aspect"), Some(CheckLevel::Fail));
    assert_eq!(level_of(&checks, "alpha"), Some(CheckLevel::Fail));
    let img = decode_bytes(&small_png(2000, 600)).expect("decode");
    let checks = check_entry(&entry(2000, 600), Some(&sp), Some(&img));
    assert_eq!(level_of(&checks, "aspect"), Some(CheckLevel::Warning));
    // Opaque JPEG against an alpha-required spec.
    let (e, bytes) = entry_for(GRADIENT_JPG, "a");
    let img = decode_bytes(&bytes).expect("decode");
    let checks = check_entry(&e, Some(&sp), Some(&img));
    assert_eq!(level_of(&checks, "alpha"), Some(CheckLevel::Fail));
    // Same JPEG, no spec: fine.
    let checks = check_entry(&e, None, Some(&img));
    assert_eq!(level_of(&checks, "alpha"), Some(CheckLevel::Pass));
}

// ---------------------------------------------------------------------------
// (0.10 Q) auto-key on delivery
// ---------------------------------------------------------------------------

const STUDIO_GROUND: [u8; 3] = [236, 230, 218];

/// An opaque 1200x1600 "studio shot": a flat ground with a red block subject.
fn studio_shot_png() -> Vec<u8> {
    let (w, h) = (1200u32, 1600u32);
    let mut pm = Pixmap::new(w, h).expect("pixmap");
    pm.fill(Color::from_rgba8(
        STUDIO_GROUND[0],
        STUDIO_GROUND[1],
        STUDIO_GROUND[2],
        255,
    ));
    let mut paint = resvg::tiny_skia::Paint::default();
    paint.set_color_rgba8(150, 30, 30, 255);
    paint.anti_alias = false;
    let rect = resvg::tiny_skia::Rect::from_xywh(300.0, 400.0, 600.0, 900.0).expect("rect");
    pm.fill_rect(rect, &paint, resvg::tiny_skia::Transform::identity(), None);
    pm.encode_png().expect("png")
}

#[test]
fn flat_ground_delivery_is_keyed_into_a_cutout_next_to_the_manifest() {
    let s = Scratch::new("autokey_basic");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-k001",
        Priority::Required,
        true,
    );
    let delivered = studio_shot_png();
    s.deliver_bytes(&delivered, "worker-k001.png");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());

    assert!(!out.report.has_fail(), "{}", out.report.report());
    assert!(out.manifest.missing.is_empty());
    let e = &out.manifest.assets[0];
    assert!(e.keyed && e.alpha, "keyed cutout: {e:?}");
    // Next to the manifest, as a PNG.
    assert_eq!(e.path, "worker-k001.keyed.png");
    let file = s.out().join(&e.path);
    assert!(file.is_file());
    let img = decode_bytes(&std::fs::read(&file).expect("read")).expect("decode");
    assert!(img.has_transparency);
    assert_eq!(
        (img.pixmap.width(), img.pixmap.height()),
        (e.width, e.height)
    );
    // Cropped to the subject (+6 % pad): far smaller than the 1200x1600 shot.
    assert!(e.width < 800 && e.height < 1100, "{}x{}", e.width, e.height);
    // Analysis describes the cutout, not the photo frame.
    let a = e.analysis.as_ref().expect("analysis");
    assert!(a.subject_bounds.width > 0.8 && a.subject_bounds.height > 0.8);
    assert!(a.coverage > 0.6 && a.coverage < 0.98);
    assert!(a.mean_color.is_some());
    // The report says what happened.
    let c = check(&out.report, "beat_1.hero_subject", "autokey").expect("autokey check");
    assert_eq!(c.level, CheckLevel::Pass);
    assert!(c.message.contains("#ece6da"), "{}", c.message);
    // The recorded content hash is the delivered file's.
    assert_eq!(
        e.content_hash.as_deref(),
        Some(content_hash(&delivered).as_str())
    );
    assert!(out.manifest.validate().is_ok());
}

#[test]
fn keyed_ingest_is_deterministic_across_runs() {
    let s = Scratch::new("autokey_determinism");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-k002",
        Priority::Required,
        true,
    );
    s.deliver_bytes(&studio_shot_png(), "worker-k002.png");
    let prompts = set(vec![sp]);
    let first = run(&s, &prompts, &IngestOptions::default());
    let bytes1 = std::fs::read(s.out().join("worker-k002.keyed.png")).expect("keyed");
    let second = run(&s, &prompts, &IngestOptions::default());
    let bytes2 = std::fs::read(s.out().join("worker-k002.keyed.png")).expect("keyed");
    assert_eq!(bytes1, bytes2);
    assert_eq!(
        serde_json::to_string_pretty(&first.manifest).expect("json"),
        serde_json::to_string_pretty(&second.manifest).expect("json")
    );
}

#[test]
fn busy_border_delivery_is_not_keyed() {
    // The gradient fixture has no flat ground: it stays opaque, so the
    // transparent-cutout request still FAILs the alpha check (as before).
    let s = Scratch::new("autokey_busy");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-k003",
        Priority::Required,
        true,
    );
    s.deliver_fixture(GRADIENT_JPG, "worker-k003.jpg");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    assert!(out.manifest.assets.is_empty());
    let c = check(&out.report, "beat_1.hero_subject", "alpha").expect("alpha");
    assert_eq!(c.level, CheckLevel::Fail);
    assert!(check(&out.report, "beat_1.hero_subject", "autokey").is_none());
    assert!(!s.out().join("worker-k003.keyed.png").exists());
}

#[test]
fn flat_ground_is_kept_when_an_opaque_image_was_requested() {
    // A full-frame photo request is never keyed, whatever its border looks like.
    let s = Scratch::new("autokey_opaque_request");
    let sp = spec(
        "beat_1.environment",
        "plate-k004",
        Priority::Optional,
        false,
    );
    s.deliver_bytes(&studio_shot_png(), "plate-k004.png");
    let out = run(&s, &set(vec![sp]), &IngestOptions::default());
    let e = &out.manifest.assets[0];
    assert!(!e.keyed && !e.alpha);
    assert_eq!((e.width, e.height), (1200, 1600));
    assert_eq!(e.path, "../delivery/plate-k004.png");
}

#[test]
fn keyed_delivery_through_the_cache_is_reused_without_the_original() {
    let s = Scratch::new("autokey_cache");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-k005",
        Priority::Required,
        true,
    );
    s.deliver_bytes(&studio_shot_png(), "worker-k005.png");
    let opts = IngestOptions {
        cache_dir: Some(s.cache()),
        replace: Vec::new(),
    };
    let prompts = set(vec![sp.clone()]);
    let first = run(&s, &prompts, &opts);
    assert!(!first.report.has_fail(), "{}", first.report.report());
    let e1 = &first.manifest.assets[0];
    assert!(e1.keyed && e1.alpha);
    let cached = first
        .cache
        .as_ref()
        .and_then(|c| c.lookup(&sp.fingerprint))
        .expect("cache entry");
    assert!(cached.keyed && cached.alpha);
    assert!(cached.file.ends_with(".png"));
    // The cached file is the keyed cutout, not the opaque original.
    let cached_img = decode_bytes(&std::fs::read(s.cache().join(&cached.file)).expect("cached"))
        .expect("decode");
    assert!(cached_img.has_transparency);

    // The generator's file is gone: the cached cutout serves the request.
    std::fs::remove_file(s.delivery().join("worker-k005.png")).expect("rm");
    let second = run(&s, &prompts, &opts);
    assert!(!second.report.has_fail(), "{}", second.report.report());
    let e2 = &second.manifest.assets[0];
    assert!(e2.keyed && e2.alpha);
    assert_eq!(
        serde_json::to_string(e1).expect("json"),
        serde_json::to_string(e2).expect("json"),
        "cache reuse reproduces the fresh entry"
    );
}

#[test]
fn a_cache_entry_from_before_keying_is_upgraded() {
    // The opaque original was cached by an engine without auto-key. Re-ingesting
    // the same delivery replaces it with the keyed cutout.
    let s = Scratch::new("autokey_upgrade");
    let sp = spec(
        "beat_1.hero_subject",
        "worker-k006",
        Priority::Required,
        true,
    );
    let delivered = studio_shot_png();
    s.deliver_bytes(&delivered, "worker-k006.png");
    std::fs::create_dir_all(s.cache()).expect("mkdir");
    let file = format!("{}.png", sp.fingerprint.trim_start_matches("fp1-"));
    std::fs::write(s.cache().join(&file), &delivered).expect("old cache file");
    let mut index = AssetCacheIndex::new();
    index
        .insert(
            CacheEntry {
                fingerprint: sp.fingerprint.clone(),
                continuity_key: sp.continuity_key.clone(),
                file: file.clone(),
                content_hash: content_hash(&delivered),
                width: 1200,
                height: 1600,
                alpha: false,
                analysis: empty_analysis(),
                face_bounds: None,
                head_bounds: None,
                face_anchor: None,
                subject_anchor: None,
                keyed: false,
                generator: serde_json::Map::new(),
            },
            false,
        )
        .expect("insert");
    std::fs::write(s.cache().join("index.json"), index.to_json_pretty()).expect("index");

    let opts = IngestOptions {
        cache_dir: Some(s.cache()),
        replace: Vec::new(),
    };
    let out = run(&s, &set(vec![sp.clone()]), &opts);
    assert!(!out.report.has_fail(), "{}", out.report.report());
    let e = &out.manifest.assets[0];
    assert!(e.keyed && e.alpha, "{e:?}");
    let cached = out
        .cache
        .as_ref()
        .and_then(|c| c.lookup(&sp.fingerprint))
        .expect("entry");
    assert!(cached.keyed);
    let img =
        decode_bytes(&std::fs::read(s.cache().join(&cached.file)).expect("file")).expect("decode");
    assert!(img.has_transparency, "the cache file is now the cutout");
}
