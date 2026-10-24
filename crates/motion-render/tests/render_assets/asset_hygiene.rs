//! (0.14) Asset hygiene: no library picture or loop frame may carry magenta
//! chroma-key leftovers. Library assets are generated on a #FF00FF ground and
//! their prompts forbid pink/purple/magenta, so any visible magenta cast is
//! key spill. Same thresholds as `scripts/despill_magenta.py --check`; run that
//! script (without `--check`) to clean a failing file.

use std::path::{Path, PathBuf};

use motion_render::tiny_skia::Pixmap;

fn pngs(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            pngs(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("png"))
        {
            out.push(p);
        }
    }
}

/// Palette quantization may round a handful of edge pixels back to a faint
/// cast; residue (near-ground colour) is never tolerated.
const CAST_TOLERANCE: usize = 10;

/// (residue pixels, magenta-cast pixels) of a cutout PNG. Opaque pictures
/// (ground plates such as `neon_purple_gradient`) are not keyed and skip.
fn magenta_stats(path: &Path) -> (usize, usize) {
    let Ok(pm) = Pixmap::load_png(path) else {
        return (0, 0);
    };
    if pm.pixels().iter().all(|p| p.alpha() == 255) {
        return (0, 0);
    }
    let (mut residue, mut cast_px) = (0, 0);
    for px in pm.pixels() {
        let c = px.demultiply();
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
    (residue, cast_px)
}

#[test]
fn library_pictures_and_loops_have_no_magenta_key_leftovers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/library");
    let mut files = Vec::new();
    pngs(&root, &mut files);
    assert!(files.len() > 100, "library not found at {}", root.display());
    let bad: Vec<String> = files
        .iter()
        .filter_map(|f| {
            let (r, c) = magenta_stats(f);
            (r > 0 || c > CAST_TOLERANCE)
                .then(|| format!("{}: {r} residue px, {c} cast px", f.display()))
        })
        .collect();
    assert!(
        bad.is_empty(),
        "{} picture(s) carry magenta key leftovers (clean with scripts/despill_magenta.py):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Families whose palette has no violet or pink at all (spec flag
/// `neutralize_violet`, scripts/neutralize_tint.py): no visible pixel may sit
/// in the violet..pink hue band or carry any red+blue-over-green cast.
const NO_VIOLET_FAMILIES: &[&str] = &["ocean_layers"];

/// Visible, non-black pixels of a picture that are tinted violet/pink.
fn tinted_px(path: &Path) -> usize {
    let Ok(pm) = Pixmap::load_png(path) else {
        return 0;
    };
    let mut n = 0;
    for px in pm.pixels() {
        let c = px.demultiply();
        if c.alpha() <= 25 {
            continue;
        }
        let (r, g, b) = (
            f32::from(c.red()) / 255.0,
            f32::from(c.green()) / 255.0,
            f32::from(c.blue()) / 255.0,
        );
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        if mx <= 0.15 {
            continue;
        }
        let d = mx - mn;
        let sat = d / mx;
        let hue = if d <= 0.0 {
            0.0
        } else if mx == r {
            (((g - b) / d).rem_euclid(6.0)) * 60.0
        } else if mx == g {
            ((b - r) / d + 2.0) * 60.0
        } else {
            ((r - g) / d + 4.0) * 60.0
        };
        let in_band = sat > 0.04 && (270.0..=340.0).contains(&hue);
        let cast = r.min(b) - g > 3.0 / 255.0;
        if in_band || cast {
            n += 1;
        }
    }
    n
}

#[test]
fn violet_free_families_have_no_pink_or_violet_tint() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/library");
    for family in NO_VIOLET_FAMILIES {
        let mut files = Vec::new();
        pngs(&root.join(family), &mut files);
        if files.is_empty() {
            continue; // family not built on this machine (assets are not redistributed)
        }
        let bad: Vec<String> = files
            .iter()
            .filter_map(|f| {
                let n = tinted_px(f);
                (n > CAST_TOLERANCE).then(|| format!("{}: {n} tinted px", f.display()))
            })
            .collect();
        assert!(
            bad.is_empty(),
            "{} {family} picture(s) carry a violet/pink tint (clean with scripts/neutralize_tint.py):\n{}",
            bad.len(),
            bad.join("\n")
        );
    }
}
