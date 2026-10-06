//! Reference analysis cache.
//!
//! Layout: `<cache_dir>/<reference_fingerprint>/` holding `evidence.json`,
//! `samples/`, `contact-sheet.png` and, once validated, `reference-style.json`
//! (the normalized-input profile). Never stores the video itself.
//!
//! `evidence.json` is written last (temp name, then rename), so an entry
//! without it — or with a stale one — is never treated as complete.

use std::path::{Component, Path, PathBuf};

use motion_core::reference::evidence::ReferenceEvidence;

use super::analyze::{existing_bundle, CONTACT_SHEET_FILE, EVIDENCE_FILE};
use super::ReferenceError;

/// Name of the cached, validated profile inside an entry.
pub const PROFILE_FILE: &str = "reference-style.json";

fn invalid(msg: impl Into<String>) -> ReferenceError {
    ReferenceError::Io(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        msg.into(),
    ))
}

/// A fingerprint is used as a directory name: plain identifier characters only.
fn safe_fingerprint(fp: &str) -> bool {
    !fp.is_empty()
        && fp
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Samples may only name files under the bundle's samples directory.
pub(super) fn safe_relative(rel: &str) -> bool {
    let p = Path::new(rel);
    !rel.contains('\\')
        && p.starts_with("samples")
        && p.components().count() >= 2
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Nonempty regular artifact whose resolved path stays inside this bundle.
pub(super) fn complete_file(dir: &Path, rel: &str) -> bool {
    let path = dir.join(rel);
    let Ok(root) = dir.canonicalize() else {
        return false;
    };
    let Ok(resolved) = path.canonicalize() else {
        return false;
    };
    resolved.starts_with(root) && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

fn entry_dir(cache_dir: &Path, fingerprint: &str) -> Option<PathBuf> {
    safe_fingerprint(fingerprint).then(|| cache_dir.join(fingerprint))
}

fn copy_file(from: &Path, to: &Path) -> Result<(), ReferenceError> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(from, to)?;
    Ok(())
}

/// Copy a complete cached bundle for `fingerprint` into `out_dir`.
/// `Ok(None)` when the cache has no complete entry.
pub fn restore(
    cache_dir: &Path,
    fingerprint: &str,
    out_dir: &Path,
) -> Result<Option<ReferenceEvidence>, ReferenceError> {
    let Some(entry) = entry_dir(cache_dir, fingerprint) else {
        return Ok(None);
    };
    let Some(evidence) = existing_bundle(&entry, fingerprint) else {
        return Ok(None);
    };
    if !evidence.samples.iter().all(|s| safe_relative(&s.image)) {
        return Ok(None);
    }

    std::fs::create_dir_all(out_dir)?;
    // A failed copy must not leave old evidence marking partial output complete.
    let evidence_path = out_dir.join(EVIDENCE_FILE);
    if evidence_path.exists() {
        std::fs::remove_file(&evidence_path)?;
    }
    let samples_dir = out_dir.join("samples");
    if samples_dir.is_dir() {
        std::fs::remove_dir_all(&samples_dir)?;
    }
    std::fs::create_dir_all(&samples_dir)?;
    for s in &evidence.samples {
        copy_file(&entry.join(&s.image), &out_dir.join(&s.image))?;
    }
    copy_file(
        &entry.join(CONTACT_SHEET_FILE),
        &out_dir.join(CONTACT_SHEET_FILE),
    )?;
    let tmp = out_dir.join(format!("{EVIDENCE_FILE}.tmp"));
    copy_file(&entry.join(EVIDENCE_FILE), &tmp)?;
    std::fs::rename(tmp, evidence_path)?;
    Ok(Some(evidence))
}

/// Store the bundle in `bundle_dir` (evidence, samples, contact sheet).
pub fn store(
    cache_dir: &Path,
    evidence: &ReferenceEvidence,
    bundle_dir: &Path,
) -> Result<(), ReferenceError> {
    let fp = &evidence.reference_fingerprint;
    let entry = entry_dir(cache_dir, fp)
        .ok_or_else(|| invalid(format!("unusable reference fingerprint '{fp}'")))?;
    if let Some(bad) = evidence.samples.iter().find(|s| !safe_relative(&s.image)) {
        return Err(invalid(format!("unsafe sample path '{}'", bad.image)));
    }

    std::fs::create_dir_all(&entry)?;
    // Invalidate any previous entry first: it is complete again only once the
    // new evidence.json lands.
    let evidence_path = entry.join(EVIDENCE_FILE);
    if evidence_path.exists() {
        std::fs::remove_file(&evidence_path)?;
    }
    let samples_dir = entry.join("samples");
    if samples_dir.is_dir() {
        std::fs::remove_dir_all(&samples_dir)?;
    }
    std::fs::create_dir_all(&samples_dir)?;
    for s in &evidence.samples {
        copy_file(&bundle_dir.join(&s.image), &entry.join(&s.image))?;
    }
    copy_file(
        &bundle_dir.join(CONTACT_SHEET_FILE),
        &entry.join(CONTACT_SHEET_FILE),
    )?;

    let text = serde_json::to_string_pretty(evidence)
        .map_err(|e| ReferenceError::Io(std::io::Error::other(e)))?;
    let tmp = entry.join(format!("{EVIDENCE_FILE}.tmp"));
    std::fs::write(&tmp, text + "\n")?;
    std::fs::rename(&tmp, &evidence_path)?;
    Ok(())
}

/// Store the validated profile JSON next to the cached evidence.
pub fn store_profile(
    cache_dir: &Path,
    fingerprint: &str,
    profile_json: &str,
) -> Result<(), ReferenceError> {
    let entry = entry_dir(cache_dir, fingerprint)
        .ok_or_else(|| invalid(format!("unusable reference fingerprint '{fingerprint}'")))?;
    std::fs::create_dir_all(&entry)?;
    let tmp = entry.join(format!("{PROFILE_FILE}.tmp"));
    std::fs::write(&tmp, profile_json)?;
    std::fs::rename(&tmp, entry.join(PROFILE_FILE))?;
    Ok(())
}

/// The profile stored for `fingerprint`, if any.
pub fn cached_profile(cache_dir: &Path, fingerprint: &str) -> Option<String> {
    let entry = entry_dir(cache_dir, fingerprint)?;
    std::fs::read_to_string(entry.join(PROFILE_FILE)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use motion_core::reference::evidence::*;

    const FP: &str = "rf1-0123456789abcdef";

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("motion_ref_cache_{}", std::process::id()))
                .join(tag);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Scratch(dir)
        }
        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn evidence(fp: &str) -> ReferenceEvidence {
        let d = Distribution::default();
        ReferenceEvidence {
            version: EVIDENCE_VERSION.into(),
            analyzer_version: ANALYZER_VERSION.into(),
            reference_fingerprint: fp.into(),
            media_fingerprint: "fnv1a64-0000000000000000".into(),
            metadata: ReferenceMetadata {
                width: 1080,
                height: 1920,
                duration_seconds: 10.0,
                fps: 30.0,
                frame_count: 300,
                aspect_ratio: 0.5625,
                orientation: Orientation::Vertical,
                video_codec: "h264".into(),
                audio_present: false,
            },
            analysis: AnalysisSpec {
                width: 90,
                height: 160,
                fps: 4.0,
                frames: 40,
                decoder: "test".into(),
            },
            samples: ["s01", "s02"]
                .iter()
                .enumerate()
                .map(|(i, id)| ReferenceSample {
                    id: (*id).into(),
                    time_seconds: i as f64,
                    frame: i as u64 * 30,
                    kinds: vec![SampleKind::Periodic],
                    image: format!("samples/{id}.jpg"),
                })
                .collect(),
            color: ColorEvidence {
                polarity: PolarityEstimate {
                    value: EvidencePolarity::Dark,
                    confidence: 0.9,
                },
                dark_frame_fraction: 1.0,
                light_frame_fraction: 0.0,
                luminance: d,
                contrast: d,
                saturation: d,
                temperature: TemperatureEstimate {
                    value: EvidenceTemperature::Neutral,
                    confidence: 0.5,
                },
                warm_cool_balance: 0.0,
                palette: vec![],
                accent_prevalence: 0.0,
                palette_stability: 1.0,
                per_sample: vec![],
            },
            temporal: TemporalEvidence {
                changes: vec![],
                noise_floor: 0.0,
                cuts_per_10s: 0.0,
                changes_per_10s: 0.0,
                median_hold_seconds: 10.0,
                hold_p90_seconds: 10.0,
                static_fraction: 1.0,
                low_motion_fraction: 1.0,
                high_motion_fraction: 0.0,
                activity: d,
                activity_variance: 0.0,
                transition_spike_ratio: 0.0,
                median_transition_seconds: 0.0,
                transition_duration: DurationTendency::Cut,
                hard_cut_fraction: 0.0,
                background_activity: 0.0,
                foreground_activity: 0.0,
            },
            complexity: ComplexityEvidence {
                edge_density: d,
                flat_area_fraction: d,
                large_region_count: d,
                entropy: d,
                occupied_ratio: d,
            },
        }
    }

    /// A bundle directory with tiny fake image files.
    fn bundle(dir: &Path, ev: &ReferenceEvidence) {
        std::fs::create_dir_all(dir.join("samples")).expect("samples dir");
        for s in &ev.samples {
            std::fs::write(dir.join(&s.image), format!("jpg-{}", s.id)).expect("sample");
        }
        std::fs::write(dir.join(CONTACT_SHEET_FILE), b"png").expect("sheet");
        std::fs::write(
            dir.join(EVIDENCE_FILE),
            serde_json::to_string_pretty(ev).expect("json"),
        )
        .expect("evidence");
    }

    #[test]
    fn store_then_restore_round_trips() {
        let s = Scratch::new("roundtrip");
        let ev = evidence(FP);
        bundle(&s.path("bundle"), &ev);
        let cache = s.path("cache");
        store(&cache, &ev, &s.path("bundle")).expect("store");
        assert!(cache.join(FP).join(EVIDENCE_FILE).is_file());
        assert!(cache.join(FP).join("samples/s02.jpg").is_file());

        // A stale sample in the target must be replaced.
        let out = s.path("out");
        std::fs::create_dir_all(out.join("samples")).expect("dir");
        std::fs::write(out.join("samples/stale.jpg"), b"x").expect("stale");
        let got = restore(&cache, FP, &out).expect("restore").expect("hit");
        assert_eq!(got, ev);
        assert_eq!(
            std::fs::read_to_string(out.join("samples/s01.jpg")).expect("s01"),
            "jpg-s01"
        );
        assert!(!out.join("samples/stale.jpg").exists());
        assert!(out.join(CONTACT_SHEET_FILE).is_file());
        assert_eq!(
            existing_bundle(&out, FP).expect("complete out dir"),
            ev,
            "restored dir is a complete bundle"
        );
    }

    #[test]
    fn missing_or_incomplete_entry_is_a_miss() {
        let s = Scratch::new("incomplete");
        let ev = evidence(FP);
        bundle(&s.path("bundle"), &ev);
        let cache = s.path("cache");
        let out = s.path("out");
        assert!(restore(&cache, FP, &out).expect("empty cache").is_none());

        store(&cache, &ev, &s.path("bundle")).expect("store");
        std::fs::remove_file(cache.join(FP).join("samples/s02.jpg")).expect("rm sample");
        assert!(restore(&cache, FP, &out).expect("restore").is_none());

        store(&cache, &ev, &s.path("bundle")).expect("store again");
        std::fs::remove_file(cache.join(FP).join(CONTACT_SHEET_FILE)).expect("rm sheet");
        assert!(restore(&cache, FP, &out).expect("restore").is_none());

        store(&cache, &ev, &s.path("bundle")).expect("store again");
        std::fs::remove_file(cache.join(FP).join(EVIDENCE_FILE)).expect("rm evidence");
        assert!(restore(&cache, FP, &out).expect("restore").is_none());
    }

    #[test]
    fn fingerprint_mismatch_is_a_miss() {
        let s = Scratch::new("fp_mismatch");
        let ev = evidence(FP);
        bundle(&s.path("bundle"), &ev);
        let cache = s.path("cache");
        store(&cache, &ev, &s.path("bundle")).expect("store");
        // The entry directory is asked for under another fingerprint.
        let other = "rf1-ffffffffffffffff";
        std::fs::rename(cache.join(FP), cache.join(other)).expect("rename");
        assert!(restore(&cache, other, &s.path("out"))
            .expect("restore")
            .is_none());
        // Path-like fingerprints never escape the cache.
        assert!(restore(&cache, "../x", &s.path("out"))
            .expect("restore")
            .is_none());
    }

    #[test]
    fn analyzer_version_mismatch_is_a_miss() {
        let s = Scratch::new("version");
        let mut ev = evidence(FP);
        ev.analyzer_version = "0.0.1-old".into();
        bundle(&s.path("bundle"), &ev);
        let cache = s.path("cache");
        store(&cache, &ev, &s.path("bundle")).expect("store");
        assert!(restore(&cache, FP, &s.path("out"))
            .expect("restore")
            .is_none());
    }

    #[test]
    fn store_rejects_unsafe_paths() {
        let s = Scratch::new("unsafe");
        let mut ev = evidence(FP);
        ev.samples[0].image = "../escape.jpg".into();
        bundle(&s.path("bundle"), &evidence(FP));
        assert!(store(&s.path("cache"), &ev, &s.path("bundle")).is_err());
        let bad_fp = evidence("../evil");
        assert!(store(&s.path("cache"), &bad_fp, &s.path("bundle")).is_err());
        for image in [
            "",
            ".",
            "samples",
            "evidence.json",
            "samples/../escape.jpg",
            "/tmp/escape.jpg",
            "samples\\escape.jpg",
        ] {
            ev.samples[0].image = image.into();
            assert!(
                store(&s.path("cache"), &ev, &s.path("bundle")).is_err(),
                "{image}"
            );
        }
    }

    #[test]
    fn malformed_bundle_is_never_reused_directly_or_from_cache() {
        let s = Scratch::new("malformed");
        let cache = s.path("cache");
        let entry = cache.join(FP);
        let valid = evidence(FP);
        bundle(&entry, &valid);
        for image in ["../outside.jpg", "evidence.json", ".", "samples"] {
            let mut ev = valid.clone();
            ev.samples[0].image = image.into();
            std::fs::write(entry.join(EVIDENCE_FILE), serde_json::to_vec(&ev).unwrap()).unwrap();
            assert!(existing_bundle(&entry, FP).is_none(), "{image}");
            assert!(restore(&cache, FP, &s.path("out")).unwrap().is_none());
        }
        let mut empty = valid.clone();
        empty.samples.clear();
        std::fs::write(
            entry.join(EVIDENCE_FILE),
            serde_json::to_vec(&empty).unwrap(),
        )
        .unwrap();
        assert!(existing_bundle(&entry, FP).is_none());

        bundle(&entry, &valid);
        std::fs::write(entry.join("samples/s01.jpg"), b"").unwrap();
        assert!(existing_bundle(&entry, FP).is_none());
        bundle(&entry, &valid);
        std::fs::write(entry.join(CONTACT_SHEET_FILE), b"").unwrap();
        assert!(existing_bundle(&entry, FP).is_none());
        bundle(&entry, &valid);
        std::fs::write(entry.join(EVIDENCE_FILE), b"{broken").unwrap();
        assert!(existing_bundle(&entry, FP).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn sample_symlink_outside_bundle_is_incomplete() {
        let s = Scratch::new("symlink");
        let ev = evidence(FP);
        let dir = s.path("bundle");
        bundle(&dir, &ev);
        let outside = s.path("outside.jpg");
        std::fs::write(&outside, b"image").unwrap();
        std::fs::remove_file(dir.join("samples/s01.jpg")).unwrap();
        std::os::unix::fs::symlink(outside, dir.join("samples/s01.jpg")).unwrap();
        assert!(existing_bundle(&dir, FP).is_none());
    }

    #[test]
    fn profile_store_and_read() {
        let s = Scratch::new("profile");
        let cache = s.path("cache");
        assert!(cached_profile(&cache, FP).is_none());
        store_profile(&cache, FP, "{\"version\":\"0.1\"}").expect("store profile");
        assert_eq!(
            cached_profile(&cache, FP).as_deref(),
            Some("{\"version\":\"0.1\"}")
        );
        // The profile alone does not make the entry restorable.
        assert!(restore(&cache, FP, &s.path("out"))
            .expect("restore")
            .is_none());

        // A later evidence store keeps the profile.
        let ev = evidence(FP);
        bundle(&s.path("bundle"), &ev);
        store(&cache, &ev, &s.path("bundle")).expect("store");
        assert!(cached_profile(&cache, FP).is_some());
        assert!(restore(&cache, FP, &s.path("out"))
            .expect("restore")
            .is_some());
    }
}
