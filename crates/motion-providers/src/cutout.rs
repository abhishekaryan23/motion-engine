//! (v2.3) Generated cutouts: a single object on flat magenta (#FF00FF), keyed
//! to RGBA with the repo's own pipeline (`scripts/key_cutout.py`, then
//! `scripts/despill_magenta.py`). Design rule: no magenta may remain; the
//! check is the one the library hygiene test uses.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use resvg::tiny_skia::Pixmap;

/// Text wrapper for providers without MOSS layout prompts (OpenArt GPT Image).
pub fn magenta_ground_prompt(desc: &str) -> String {
    format!(
        "{}. One single complete isolated object, centred, with a 10% empty margin on every side, \
         on a pure flat solid #FF00FF chroma key magenta background filling the whole frame. \
         Zero cast shadows, no floor, no horizon, no gradient, no vignette, no text. \
         No pink, purple or magenta in the object.",
        desc.trim().trim_end_matches('.')
    )
}

/// `$MOTION_SCRIPTS_DIR`, else the repo's `scripts/`.
pub fn scripts_dir() -> PathBuf {
    std::env::var_os("MOTION_SCRIPTS_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts"))
}

/// Palette quantization may leave a handful of faint edge pixels; residue
/// (near-ground colour) is never tolerated (`asset_hygiene` thresholds).
pub const CAST_TOLERANCE: usize = 10;

/// (residue px, magenta-cast px, transparent px) of an RGBA PNG.
pub fn magenta_stats(png: &[u8]) -> Result<(usize, usize, usize)> {
    let pm = Pixmap::decode_png(png).context("decoding PNG")?;
    let (mut residue, mut cast_px, mut clear) = (0, 0, 0);
    for px in pm.pixels() {
        let c = px.demultiply();
        if c.alpha() < 255 {
            clear += 1;
        }
        if c.alpha() <= 8 {
            continue;
        }
        let (r, g, b) = (
            f32::from(c.red()) / 255.0,
            f32::from(c.green()) / 255.0,
            f32::from(c.blue()) / 255.0,
        );
        let cast = r.min(b) - g;
        if cast > 0.35 && (r - b).abs() < 0.35 {
            residue += 1;
        } else if cast > 0.18 {
            cast_px += 1;
        }
    }
    Ok((residue, cast_px, clear))
}

/// Fails unless the PNG is a keyed cutout (has transparency) with no
/// magenta residue and at most [`CAST_TOLERANCE`] cast pixels.
pub fn check_magenta_free(png: &[u8]) -> Result<()> {
    let (residue, cast, clear) = magenta_stats(png)?;
    if clear == 0 {
        bail!("cutout has no transparent pixels: the key failed");
    }
    if residue > 0 || cast > CAST_TOLERANCE {
        bail!("cutout keeps magenta: {residue} residue px, {cast} cast px");
    }
    Ok(())
}

/// Key a magenta-ground PNG to a tight RGBA PNG and check it is magenta-free.
pub fn key_magenta(png: &[u8], scripts: &Path) -> Result<Vec<u8>> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("motion-key-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;
    let r = key_in(&tmp, png, scripts);
    let _ = std::fs::remove_dir_all(&tmp);
    r
}

fn key_in(tmp: &Path, png: &[u8], scripts: &Path) -> Result<Vec<u8>> {
    let src = tmp.join("src.png");
    std::fs::write(&src, png)?;
    let stem = tmp.join("cut");
    let run = |script: &str, args: &[&Path]| -> Result<()> {
        let out = Command::new("python3")
            .arg("-I")
            .arg(scripts.join(script))
            .args(args)
            .output()
            .with_context(|| format!("running python3 {script}"))?;
        if !out.status.success() {
            bail!(
                "{script} failed: {}",
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .last()
                    .unwrap_or("")
            );
        }
        Ok(())
    };
    run("key_cutout.py", &[&src, &stem])?;
    let cut = tmp.join("cut.png");
    run("despill_magenta.py", &[&cut])?;
    let bytes = std::fs::read(&cut).context("keyed cutout missing")?;
    check_magenta_free(&bytes)?;
    Ok(bytes)
}
