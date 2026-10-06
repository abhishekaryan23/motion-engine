//! SFX library tooling (0.8): measure pack sounds via ffmpeg,
//! build a self-contained library directory, load it, package it as a zip.
//! See docs/SOUND_DESIGN.md §2.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use motion_core::audio::{
    CurationEntry, SfxCuration, SfxLibrary, SfxSound, LIBRARY_FILE, SFX_CURATION_VERSION,
    SFX_LIBRARY_VERSION,
};
use sha2::{Digest, Sha256};

use crate::RenderError;

/// Measured facts of one file (seconds from file start, ms-rounded).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SfxMeasurement {
    pub duration: f64,
    pub onset: f64,
    pub peak: f64,
    pub audible_end: f64,
    pub peak_db: f64,
    pub lufs: Option<f64>,
}

const SAMPLE_RATE: f64 = 48_000.0;
const WINDOW: usize = 480;
const HOP: usize = 48;
const SILENCE_DB: f64 = -120.0;

fn round_ms(t: f64) -> f64 {
    (t * 1000.0).round() / 1000.0
}

fn round_db(d: f64) -> f64 {
    (d * 10.0).round() / 10.0
}

fn to_db(amplitude: f64) -> f64 {
    if amplitude <= 1e-6 {
        SILENCE_DB
    } else {
        (20.0 * amplitude.log10()).max(SILENCE_DB)
    }
}

/// Envelope analysis of mono 48 kHz samples (`lufs` left `None`).
fn analyze_samples(samples: &[f32]) -> SfxMeasurement {
    let n = samples.len();
    let duration = n as f64 / SAMPLE_RATE;
    let max_abs = samples
        .iter()
        .fold(0.0f64, |m, &x| m.max(f64::from(x).abs()));
    let peak_db = to_db(max_abs);

    // Prefix sums of squares: window energy in O(1).
    let mut prefix = Vec::with_capacity(n + 1);
    let mut acc = 0.0f64;
    prefix.push(0.0);
    for &x in samples {
        acc += f64::from(x) * f64::from(x);
        prefix.push(acc);
    }
    let rms = |a: usize, b: usize| -> f64 {
        let e = (prefix[b] - prefix[a]).max(0.0);
        (e / (b - a) as f64).sqrt()
    };

    // (start, end) sample ranges of each window.
    let windows: Vec<(usize, usize)> = if n < WINDOW {
        vec![(0, n)]
    } else {
        (0..=(n - WINDOW) / HOP)
            .map(|k| (k * HOP, k * HOP + WINDOW))
            .collect()
    };
    let db: Vec<f64> = windows.iter().map(|&(a, b)| to_db(rms(a, b))).collect();
    let mut best = 0usize;
    for (i, &d) in db.iter().enumerate() {
        if d > db[best] {
            best = i;
        }
    }
    let m_db = db[best];
    let centre = |w: (usize, usize)| (w.0 + w.1) as f64 / 2.0 / SAMPLE_RATE;
    let first_on = db.iter().position(|&d| d >= m_db - 30.0).unwrap_or(best);
    let last_au = db.iter().rposition(|&d| d >= m_db - 40.0).unwrap_or(best);

    let peak = centre(windows[best]);
    let onset = centre(windows[first_on]).min(peak);
    let audible_end = (windows[last_au].1 as f64 / SAMPLE_RATE)
        .min(duration)
        .max(peak);

    let duration_r = round_ms(duration);
    let peak_r = round_ms(peak).min(duration_r);
    let onset_r = round_ms(onset).min(peak_r);
    let end_r = round_ms(audible_end).clamp(peak_r, duration_r);
    SfxMeasurement {
        duration: duration_r,
        onset: onset_r,
        peak: peak_r,
        audible_end: end_r,
        peak_db: round_db(peak_db),
        lufs: None,
    }
}

/// Integrated loudness via ffmpeg `ebur128`; `None` when unparsable or gated out.
fn measure_lufs(path: &Path) -> Option<f64> {
    let out = Command::new("ffmpeg")
        .args(["-nostats", "-nostdin", "-i"])
        .arg(path)
        .args(["-af", "ebur128", "-f", "null", "-"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stderr);
    let summary = &text[text.rfind("Summary:")?..];
    for line in summary.lines() {
        if let Some(rest) = line.trim().strip_prefix("I:") {
            let v: f64 = rest.split_whitespace().next()?.parse().ok()?;
            if !v.is_finite() || v <= -70.0 {
                return None;
            }
            return Some(round_db(v));
        }
    }
    None
}

/// Decode via ffmpeg (mono, 48 kHz, f32) and measure; see SOUND_DESIGN §2.
pub fn measure_sound(path: &Path) -> Result<SfxMeasurement, RenderError> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"])
        .output()
        .map_err(|e| RenderError::Encode(format!("cannot run ffmpeg: {e}")))?;
    if !out.status.success() {
        return Err(RenderError::Encode(format!(
            "decode {}: {}",
            path.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let samples: Vec<f32> = out
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    if samples.is_empty() {
        return Err(RenderError::Encode(format!(
            "decode {}: no audio samples",
            path.display()
        )));
    }
    let mut m = analyze_samples(&samples);
    m.lufs = measure_lufs(path);
    Ok(m)
}

/// Lowercase hex SHA-256 of the file bytes.
pub fn sha256_file(path: &Path) -> Result<String, RenderError> {
    let bytes = std::fs::read(path)
        .map_err(|e| RenderError::Asset(format!("read {}: {e}", path.display())))?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Debug, Clone, Default)]
pub struct IndexOptions {
    /// Measurement cache (default `<out_dir>/.measure-cache`), keyed by sha256.
    pub cache_dir: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct IndexReport {
    pub library: SfxLibrary,
    pub measured: usize,
    pub cached: usize,
    /// (curation id, error) for entries that could not be copied or measured.
    pub failed: Vec<(String, String)>,
}

fn rel_path_ok(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('/')
        && !p.contains('\\')
        && !p.contains(':')
        && p.split('/').all(|c| c != ".." && !c.is_empty())
}

/// Copy, hash, and measure (or fetch from cache) one curated entry.
/// Returns the sound and whether the measurement came from the cache.
fn index_entry(
    pack_root: &Path,
    entry: &CurationEntry,
    out_dir: &Path,
    cache_dir: &Path,
    dest_abs: &mut Option<PathBuf>,
) -> Result<(SfxSound, bool), String> {
    if entry.id.is_empty()
        || entry.id.starts_with('.')
        || entry
            .id
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
    {
        return Err(format!("invalid sound id '{}'", entry.id));
    }
    if !rel_path_ok(&entry.path) {
        return Err(format!(
            "path '{}' must be relative to the pack root (no absolute or '..')",
            entry.path
        ));
    }
    let src = pack_root.join(&entry.path);
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .ok_or_else(|| format!("'{}' has no usable file extension", entry.path))?;
    let rel = format!("sounds/{}/{}.{ext}", entry.family.as_str(), entry.id);
    let dest = out_dir.join(&rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    *dest_abs = Some(dest.clone());
    std::fs::copy(&src, &dest).map_err(|e| format!("copy {}: {e}", src.display()))?;
    let sha = sha256_file(&dest).map_err(|e| e.to_string())?;

    let cache_file = cache_dir.join(format!("{sha}.json"));
    let hit = std::fs::read_to_string(&cache_file)
        .ok()
        .and_then(|t| serde_json::from_str::<SfxMeasurement>(&t).ok());
    let (m, was_cached) = match hit {
        Some(m) => (m, true),
        None => {
            let m = measure_sound(&dest).map_err(|e| e.to_string())?;
            let json = serde_json::to_string(&m).map_err(|e| e.to_string())?;
            let tmp = cache_dir.join(format!("{sha}.json.tmp"));
            std::fs::write(&tmp, json).map_err(|e| format!("write cache: {e}"))?;
            std::fs::rename(&tmp, &cache_file).map_err(|e| format!("write cache: {e}"))?;
            (m, false)
        }
    };
    Ok((
        SfxSound {
            id: entry.id.clone(),
            family: entry.family,
            path: rel,
            duration: m.duration,
            onset: m.onset,
            peak: m.peak,
            audible_end: m.audible_end,
            peak_db: m.peak_db,
            lufs: m.lufs,
            sha256: sha,
            tags: entry.tags.clone(),
        },
        was_cached,
    ))
}

/// Copy each curated file to `<out_dir>/sounds/<family>/<id>.<ext>`, measure
/// it (cache by sha256), write `<out_dir>/sfx-library.json` sorted by id.
/// Per-entry failures are reported, not fatal.
pub fn index_pack(
    pack_root: &Path,
    curation: &SfxCuration,
    out_dir: &Path,
    opts: &IndexOptions,
) -> Result<IndexReport, RenderError> {
    if curation.version != SFX_CURATION_VERSION {
        return Err(RenderError::Asset(format!(
            "unsupported sfx curation version '{}' (expected {SFX_CURATION_VERSION})",
            curation.version
        )));
    }
    std::fs::create_dir_all(out_dir)?;
    let cache_dir = opts
        .cache_dir
        .clone()
        .unwrap_or_else(|| out_dir.join(".measure-cache"));
    std::fs::create_dir_all(&cache_dir)?;

    let mut seen = BTreeSet::new();
    let mut sounds: Vec<SfxSound> = Vec::new();
    let mut measured = 0usize;
    let mut cached = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();

    for entry in &curation.sounds {
        if !seen.insert(entry.id.clone()) {
            failed.push((entry.id.clone(), "duplicate curation id".to_string()));
            continue;
        }
        let mut dest_abs: Option<PathBuf> = None;
        match index_entry(pack_root, entry, out_dir, &cache_dir, &mut dest_abs) {
            Ok((sound, was_cached)) => {
                if was_cached {
                    cached += 1;
                } else {
                    measured += 1;
                }
                sounds.push(sound);
            }
            Err(msg) => {
                if let Some(d) = dest_abs {
                    let _ = std::fs::remove_file(d);
                }
                failed.push((entry.id.clone(), msg));
            }
        }
    }
    sounds.sort_by(|a, b| a.id.cmp(&b.id));
    let library = SfxLibrary {
        version: SFX_LIBRARY_VERSION.to_string(),
        sounds,
    };
    library
        .validate()
        .map_err(|e| RenderError::Asset(format!("sfx library invalid: {e}")))?;
    std::fs::write(
        out_dir.join(LIBRARY_FILE),
        format!("{}\n", library.to_json_pretty()),
    )?;
    Ok(IndexReport {
        library,
        measured,
        cached,
        failed,
    })
}

/// Load a library from a directory (containing `sfx-library.json`) or from
/// the manifest file itself. Returns (root directory, validated library).
pub fn load_library(dir_or_file: &Path) -> Result<(PathBuf, SfxLibrary), RenderError> {
    let manifest = if dir_or_file.is_dir() {
        dir_or_file.join(LIBRARY_FILE)
    } else {
        dir_or_file.to_path_buf()
    };
    let root = match manifest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| RenderError::Asset(format!("read {}: {e}", manifest.display())))?;
    let library = SfxLibrary::from_json(&text)
        .map_err(|e| RenderError::Asset(format!("{}: {e}", manifest.display())))?;
    for s in &library.sounds {
        let p = root.join(&s.path);
        if !p.is_file() {
            return Err(RenderError::Asset(format!(
                "{}: sound '{}' file missing: {}",
                manifest.display(),
                s.id,
                p.display()
            )));
        }
    }
    Ok((root, library))
}

/// Write a deterministic STORE zip of `sfx-library.json`, every referenced
/// sound and `LICENSE.md`/`README.md` when present. Returns the entry count.
pub fn build_pack(library_root: &Path, out_zip: &Path) -> Result<usize, RenderError> {
    let (root, library) = load_library(library_root)?;
    let mut names: BTreeSet<String> = BTreeSet::new();
    names.insert(LIBRARY_FILE.to_string());
    for s in &library.sounds {
        names.insert(s.path.clone());
    }
    for extra in ["LICENSE.md", "README.md"] {
        if root.join(extra).is_file() {
            names.insert(extra.to_string());
        }
    }

    const DOS_TIME: u16 = 0;
    const DOS_DATE: u16 = (1 << 5) | 1; // 1980-01-01
    const MAX: u64 = 4 * 1024 * 1024 * 1024 - 1;
    let too_big = || RenderError::Asset("zip exceeds 4 GiB (zip64 unsupported)".to_string());

    let mut buf: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for name in &names {
        let path = root.join(name);
        let data = std::fs::read(&path)
            .map_err(|e| RenderError::Asset(format!("read {}: {e}", path.display())))?;
        let size = u32::try_from(data.len()).map_err(|_| too_big())?;
        let offset = u32::try_from(buf.len()).map_err(|_| too_big())?;
        let name_len = u16::try_from(name.len())
            .map_err(|_| RenderError::Asset(format!("zip entry name too long: {name}")))?;
        let crc = crc32fast::hash(&data);

        buf.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        buf.extend_from_slice(&10u16.to_le_bytes()); // version needed
        buf.extend_from_slice(&0u16.to_le_bytes()); // flags
        buf.extend_from_slice(&0u16.to_le_bytes()); // method: STORE
        buf.extend_from_slice(&DOS_TIME.to_le_bytes());
        buf.extend_from_slice(&DOS_DATE.to_le_bytes());
        buf.extend_from_slice(&crc.to_le_bytes());
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&size.to_le_bytes());
        buf.extend_from_slice(&name_len.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // extra len
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(&data);
        if buf.len() as u64 > MAX {
            return Err(too_big());
        }

        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&0x031eu16.to_le_bytes()); // made by: unix, 3.0
        central.extend_from_slice(&10u16.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&DOS_TIME.to_le_bytes());
        central.extend_from_slice(&DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let count = u16::try_from(names.len())
        .map_err(|_| RenderError::Asset("too many zip entries".to_string()))?;
    let cd_offset = u32::try_from(buf.len()).map_err(|_| too_big())?;
    let cd_size = u32::try_from(central.len()).map_err(|_| too_big())?;
    buf.extend_from_slice(&central);
    buf.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // disk
    buf.extend_from_slice(&0u16.to_le_bytes()); // cd disk
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    buf.extend_from_slice(&cd_size.to_le_bytes());
    buf.extend_from_slice(&cd_offset.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // comment len
    if buf.len() as u64 > MAX {
        return Err(too_big());
    }

    if let Some(parent) = out_zip.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(out_zip, &buf)?;
    Ok(names.len())
}
