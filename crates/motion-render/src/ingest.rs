//! Asset ingestion (0.5): generated files + prompt specs → validated, analyzed
//! `AssetManifest`, through an optional fingerprint cache.
//! Protocol: docs/GENERATED_ASSET_PROTOCOL.md.

use std::path::{Path, PathBuf};

use motion_core::assets::{
    content_hash, AssetAnalysis, AssetCacheIndex, AssetManifest, AssetPromptSet, AssetPromptSpec,
    CacheConflict, CacheEntry, CacheInsert, GeneratedSidecar, ImageFormat, ManifestEntry,
    MissingAsset, NormPoint, Priority, ASSET_MANIFEST_VERSION,
};

use crate::analysis::{analyze, AnalyzeOptions};
use crate::asset_qa::{
    check_entry, entry_is_person, manifest_structure_qa, missing_check, AssetCheck, AssetQa,
    AssetQaReport, CheckLevel,
};
use crate::autokey;
use crate::decode::{decode_bytes, decode_file, DecodedImage};

#[derive(Debug, Clone, Default)]
pub struct IngestOptions {
    /// Cache directory (`index.json` + `<fingerprint without "fp1-">.<ext>` files). `None` = no cache.
    pub cache_dir: Option<PathBuf>,
    /// Spec ids or fingerprints whose cached image may be replaced by a newly
    /// delivered file with different content (explicit regeneration).
    pub replace: Vec<String>,
}

pub struct IngestOutcome {
    /// Paths relative to the manifest directory.
    pub manifest: AssetManifest,
    pub report: AssetQaReport,
    /// Updated cache index (already written to disk when a cache dir is set).
    pub cache: Option<AssetCacheIndex>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Cache(String),
}

/// Accepted delivery extensions (lowercase), in lookup order.
const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Ingest the files an external generator wrote into `delivery_dir` for
/// `prompts`. `manifest_dir` is where the manifest will be written (entry
/// paths are made relative to it).
pub fn ingest(
    prompts: &AssetPromptSet,
    delivery_dir: &Path,
    manifest_dir: &Path,
    opts: &IngestOptions,
) -> Result<IngestOutcome, IngestError> {
    // Paths are made relative to the manifest directory, which must resolve.
    std::fs::create_dir_all(manifest_dir)?;
    let mut cache = match &opts.cache_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            Some(load_index(dir)?)
        }
        None => None,
    };

    let mut manifest = AssetManifest::empty();
    manifest.version = ASSET_MANIFEST_VERSION.to_string();
    let mut report = AssetQaReport::default();

    for spec in &prompts.specs {
        let outcome = ingest_spec(spec, delivery_dir, manifest_dir, opts, cache.as_mut())?;
        match outcome.entry {
            Some(entry) => manifest.assets.push(entry),
            None => manifest.missing.push(MissingAsset {
                id: spec.id.clone(),
                priority: spec.priority,
                reason: outcome
                    .reason
                    .unwrap_or_else(|| "not delivered".to_string()),
            }),
        }
        report.assets.push(AssetQa {
            id: spec.id.clone(),
            checks: outcome.checks,
        });
    }

    if let Some(qa) = manifest_structure_qa(&manifest) {
        report.assets.push(qa);
    }

    if let (Some(dir), Some(index)) = (&opts.cache_dir, &cache) {
        std::fs::write(dir.join("index.json"), index.to_json_pretty() + "\n")?;
    }
    Ok(IngestOutcome {
        manifest,
        report,
        cache,
    })
}

/// Result of processing one spec.
struct SpecOutcome {
    entry: Option<ManifestEntry>,
    /// Why there is no entry.
    reason: Option<String>,
    checks: Vec<AssetCheck>,
}

impl SpecOutcome {
    /// Not usable: record the checks and the reason. An *optional* image never
    /// fails the pipeline (its FAILs are reported as WARNINGS: the beat falls
    /// back to the image-free composition); a *required* one keeps its FAIL.
    fn unusable(spec: &AssetPromptSpec, reason: String, mut checks: Vec<AssetCheck>) -> Self {
        if spec.priority == Priority::Optional {
            for c in &mut checks {
                if c.level == CheckLevel::Fail {
                    c.level = CheckLevel::Warning;
                }
            }
            checks.push(missing_check(Priority::Optional, &reason));
        } else if !checks.iter().any(|c| c.level == CheckLevel::Fail) {
            checks.push(missing_check(Priority::Required, &reason));
        }
        SpecOutcome {
            entry: None,
            reason: Some(reason),
            checks,
        }
    }
}

fn load_index(dir: &Path) -> Result<AssetCacheIndex, IngestError> {
    let path = dir.join("index.json");
    if !path.exists() {
        return Ok(AssetCacheIndex::new());
    }
    let text = std::fs::read_to_string(&path)?;
    AssetCacheIndex::from_json(&text)
        .map_err(|e| IngestError::Cache(format!("{}: {e}", path.display())))
}

/// Delivered image files for `stem`, in extension order.
fn find_delivered(delivery_dir: &Path, stem: &str) -> Vec<PathBuf> {
    EXTENSIONS
        .iter()
        .map(|ext| delivery_dir.join(format!("{stem}.{ext}")))
        .filter(|p| p.is_file())
        .collect()
}

fn first_failure(checks: &[AssetCheck]) -> Option<String> {
    checks
        .iter()
        .find(|c| c.level == CheckLevel::Fail)
        .map(|c| format!("{}: {}", c.name, c.message))
}

fn ingest_spec(
    spec: &AssetPromptSpec,
    delivery_dir: &Path,
    manifest_dir: &Path,
    opts: &IngestOptions,
    cache: Option<&mut AssetCacheIndex>,
) -> Result<SpecOutcome, IngestError> {
    let stem = &spec.output.file_stem;
    let found = find_delivered(delivery_dir, stem);

    if found.len() > 1 {
        let names: Vec<String> = found
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        let msg = format!("several files delivered for '{stem}': {}", names.join(", "));
        // Ambiguous delivery is always a hard failure, whatever the priority.
        return Ok(SpecOutcome {
            entry: None,
            reason: Some(msg.clone()),
            checks: vec![AssetCheck::fail("delivery", msg)],
        });
    }

    let Some(file) = found.into_iter().next() else {
        return Ok(from_cache_or_missing(
            spec,
            manifest_dir,
            opts,
            cache.as_deref(),
        ));
    };
    let file_name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let mut checks = vec![AssetCheck::pass(
        "delivery",
        format!("delivered {file_name}"),
    )];

    // Decode.
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) => {
            let c = AssetCheck::fail("file", format!("cannot read {file_name}: {e}"));
            checks.push(c);
            let reason = first_failure(&checks).unwrap_or_default();
            return Ok(SpecOutcome::unusable(spec, reason, checks));
        }
    };
    let delivered = match decode_bytes(&bytes) {
        Ok(d) => d,
        Err(e) => {
            checks.push(AssetCheck::fail("decode", format!("{file_name}: {e}")));
            let reason = first_failure(&checks).unwrap_or_default();
            return Ok(SpecOutcome::unusable(spec, reason, checks));
        }
    };

    // Sidecar.
    let sidecar_path = delivery_dir.join(format!("{stem}.json"));
    let mut sidecar = GeneratedSidecar::default();
    if sidecar_path.is_file() {
        match std::fs::read_to_string(&sidecar_path)
            .map_err(|e| e.to_string())
            .and_then(|t| serde_json::from_str::<GeneratedSidecar>(&t).map_err(|e| e.to_string()))
        {
            Ok(s) => {
                if let Some(fp) = &s.fingerprint {
                    if *fp != spec.fingerprint {
                        checks.push(AssetCheck::fail(
                            "fingerprint",
                            format!(
                                "sidecar fingerprint {fp} differs from spec fingerprint {}",
                                spec.fingerprint
                            ),
                        ));
                    }
                }
                sidecar = s;
            }
            Err(e) => checks.push(AssetCheck::fail(
                "file",
                format!("sidecar {stem}.json is invalid: {e}"),
            )),
        }
    }

    // (0.10 Q) Auto-key: an opaque delivery of a cutout request whose border is
    // one flat colour is keyed into an alpha cutout cropped to the subject
    // (`autokey`); the keyed PNG replaces the delivered file from here on.
    // Photographs with busy borders, and requests that want an opaque image,
    // are untouched.
    let keyed = (spec.output.alpha_required && !delivered.has_transparency)
        .then(|| autokey::key_flat_ground(&delivered.pixmap))
        .flatten()
        .and_then(|k| k.pixmap.encode_png().ok().map(|png| (k, png)));
    let source_size = (delivered.pixmap.width(), delivered.pixmap.height());
    let decoded = match &keyed {
        Some((k, _)) => DecodedImage {
            pixmap: k.pixmap.clone(),
            format: ImageFormat::Png,
            has_transparency: true,
        },
        None => delivered,
    };
    // Sidecar geometry is normalized to the delivered image: re-express it in
    // the cutout.
    let (face_bounds, head_bounds, face_anchor, subject_anchor) = match &keyed {
        Some((k, _)) => (
            sidecar.face_bounds.and_then(|b| k.remap_box(b)),
            sidecar.head_bounds.and_then(|b| k.remap_box(b)),
            sidecar.face_anchor.map(|p| k.remap_point(p)),
            sidecar.subject_anchor.map(|p| k.remap_point(p)),
        ),
        None => (
            sidecar.face_bounds,
            sidecar.head_bounds,
            sidecar.face_anchor,
            sidecar.subject_anchor,
        ),
    };

    // Manifest entry (analysis measured once, here). `content_hash` is the
    // delivered file's hash (a keyed cutout derives from it deterministically),
    // so cache hits and fresh deliveries record the same value.
    let mut entry = ManifestEntry {
        id: spec.id.clone(),
        path: String::new(),
        width: decoded.pixmap.width(),
        height: decoded.pixmap.height(),
        alpha: decoded.has_transparency,
        safe_bounds: None,
        subject_anchor: None,
        face_anchor: None,
        face_bounds,
        head_bounds,
        serves: spec.serves.clone(),
        fingerprint: Some(spec.fingerprint.clone()),
        continuity_key: Some(spec.continuity_key.clone()),
        content_hash: Some(content_hash(&bytes)),
        analysis: None,
        keyed: keyed.is_some(),
        generator: sidecar.generator.clone(),
    };
    let analysis = analyze(
        &decoded.pixmap,
        &AnalyzeOptions {
            person: entry_is_person(&entry, Some(spec)),
            ..AnalyzeOptions::default()
        },
    );
    entry.analysis = Some(analysis.clone());
    entry.subject_anchor = subject_anchor.or_else(|| {
        let b = analysis.subject_bounds;
        Some(NormPoint {
            x: b.x + b.width * 0.5,
            y: b.y + b.height * 0.5,
        })
    });
    entry.face_anchor = face_anchor.or_else(|| {
        entry.head_region().map(|h| NormPoint {
            x: h.x + h.width * 0.5,
            y: h.y + h.height * 0.5,
        })
    });

    if let Some((k, _)) = &keyed {
        checks.push(AssetCheck::pass(
            "autokey",
            format!(
                "flat ground #{:02x}{:02x}{:02x} keyed into a {}×{} cutout (from {}×{})",
                k.ground[0],
                k.ground[1],
                k.ground[2],
                k.pixmap.width(),
                k.pixmap.height(),
                source_size.0,
                source_size.1
            ),
        ));
    }
    checks.extend(check_entry(&entry, Some(spec), Some(&decoded)));
    if let Some(reason) = first_failure(&checks) {
        return Ok(SpecOutcome::unusable(spec, reason, checks));
    }

    // Cache.
    let keyed_png: Option<&[u8]> = keyed.as_ref().map(|(_, png)| png.as_slice());
    let mut final_path = file.clone();
    if let (Some(index), Some(dir)) = (cache, opts.cache_dir.as_deref()) {
        // A keyed cutout is always a PNG.
        let ext = if keyed_png.is_some() {
            "png".to_string()
        } else {
            file.extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_else(|| "png".to_string())
        };
        let cache_file = format!(
            "{}.{ext}",
            spec.fingerprint
                .strip_prefix("fp1-")
                .unwrap_or(&spec.fingerprint)
        );
        let replace = opts
            .replace
            .iter()
            .any(|r| *r == spec.id || *r == spec.fingerprint);
        // An entry cached before keying existed holds the opaque original:
        // drop it so the keyed cutout takes its place (not a regeneration).
        if keyed_png.is_some() {
            if let Some(old) = index
                .lookup(&spec.fingerprint)
                .filter(|e| !e.keyed)
                .map(|e| e.file.clone())
            {
                index.entries.retain(|e| e.fingerprint != spec.fingerprint);
                let _ = std::fs::remove_file(dir.join(old));
            }
        }
        let previous_file = index.lookup(&spec.fingerprint).map(|e| e.file.clone());
        let cache_entry = CacheEntry {
            fingerprint: spec.fingerprint.clone(),
            continuity_key: spec.continuity_key.clone(),
            file: cache_file.clone(),
            content_hash: content_hash(&bytes),
            width: entry.width,
            height: entry.height,
            alpha: entry.alpha,
            analysis,
            face_bounds: entry.face_bounds,
            head_bounds: entry.head_bounds,
            face_anchor: entry.face_anchor,
            subject_anchor: entry.subject_anchor,
            keyed: entry.keyed,
            generator: entry.generator.clone(),
        };
        match index.insert(cache_entry, replace) {
            Ok(result) => {
                let target = dir.join(&cache_file);
                match keyed_png {
                    Some(png) => write_if_changed(&target, png)?,
                    None => {
                        if result != CacheInsert::Unchanged || !target.is_file() {
                            std::fs::copy(&file, &target)?;
                        }
                    }
                }
                if result == CacheInsert::Replaced {
                    if let Some(old) = previous_file.filter(|old| *old != cache_file) {
                        let _ = std::fs::remove_file(dir.join(old));
                    }
                }
                let note = match result {
                    CacheInsert::Added => "cached",
                    CacheInsert::Unchanged => "cache unchanged",
                    CacheInsert::Replaced => "cache entry replaced (regenerated)",
                };
                checks.push(AssetCheck::pass("fingerprint", note));
                final_path = target;
            }
            Err(conflict @ CacheConflict::ContentChanged { .. }) => {
                checks.push(AssetCheck::fail("fingerprint", conflict.to_string()));
                let reason = first_failure(&checks).unwrap_or_default();
                return Ok(SpecOutcome::unusable(spec, reason, checks));
            }
        }
    } else if let Some(png) = keyed_png {
        // No cache: the keyed cutout sits next to the manifest.
        let keyed_path = manifest_dir.join(format!("{stem}.keyed.png"));
        write_if_changed(&keyed_path, png)?;
        final_path = keyed_path;
    }

    entry.path = relative_path(&final_path, manifest_dir);
    Ok(SpecOutcome {
        entry: Some(entry),
        reason: None,
        checks,
    })
}

/// Nothing was delivered: reuse the cached image for this fingerprint, else
/// record the request as missing.
fn from_cache_or_missing(
    spec: &AssetPromptSpec,
    manifest_dir: &Path,
    opts: &IngestOptions,
    cache: Option<&AssetCacheIndex>,
) -> SpecOutcome {
    let cached = cache.and_then(|c| c.lookup(&spec.fingerprint));
    if let (Some(cached), Some(dir)) = (cached, opts.cache_dir.as_deref()) {
        let path = dir.join(&cached.file);
        if let Ok(decoded) = decode_file(&path) {
            let entry = entry_from_cache(spec, cached, &path, manifest_dir);
            let mut checks = vec![AssetCheck::pass(
                "delivery",
                format!("not delivered; reused cached image {}", cached.file),
            )];
            checks.extend(check_entry(&entry, Some(spec), Some(&decoded)));
            if let Some(reason) = first_failure(&checks) {
                return SpecOutcome::unusable(spec, reason, checks);
            }
            return SpecOutcome {
                entry: Some(entry),
                reason: None,
                checks,
            };
        }
        let reason = format!("not delivered; cached file {} is unreadable", cached.file);
        return SpecOutcome::unusable(spec, reason, Vec::new());
    }
    SpecOutcome::unusable(spec, "not delivered".to_string(), Vec::new())
}

fn entry_from_cache(
    spec: &AssetPromptSpec,
    cached: &CacheEntry,
    path: &Path,
    manifest_dir: &Path,
) -> ManifestEntry {
    let analysis: AssetAnalysis = cached.analysis.clone();
    ManifestEntry {
        id: spec.id.clone(),
        path: relative_path(path, manifest_dir),
        width: cached.width,
        height: cached.height,
        alpha: cached.alpha,
        safe_bounds: None,
        subject_anchor: cached.subject_anchor,
        face_anchor: cached.face_anchor,
        face_bounds: cached.face_bounds,
        head_bounds: cached.head_bounds,
        serves: spec.serves.clone(),
        fingerprint: Some(spec.fingerprint.clone()),
        continuity_key: Some(spec.continuity_key.clone()),
        content_hash: Some(cached.content_hash.clone()),
        analysis: Some(analysis),
        keyed: cached.keyed,
        generator: cached.generator.clone(),
    }
}

/// Write `bytes` to `path` unless the file already holds exactly them (keeps
/// reruns from touching unchanged keyed cutouts).
fn write_if_changed(path: &Path, bytes: &[u8]) -> Result<(), IngestError> {
    if std::fs::read(path).map_or(true, |old| old != bytes) {
        std::fs::write(path, bytes)?;
    }
    Ok(())
}

/// `path` relative to `base`, with forward slashes.
fn relative_path(path: &Path, base: &Path) -> String {
    let (abs_path, abs_base) = (absolute(path), absolute(base));
    let rel = pathdiff::diff_paths(&abs_path, &abs_base).unwrap_or(abs_path);
    rel.to_string_lossy().replace('\\', "/")
}

fn absolute(p: &Path) -> PathBuf {
    p.canonicalize()
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf())
}
