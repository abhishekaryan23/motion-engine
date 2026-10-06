//! (0.14) `matte`: on-device subject isolation for photos (Apple Vision,
//! macOS 14+). The Swift helper `scripts/matte_vision.swift` is compiled once
//! into `<cache>/tools/matte_vision` and run per image. No network, no model
//! download. Operator tooling only: the compiler never calls it.
//!
//! * `matte photo.jpg -o cutout.png [--crop]`
//! * `matte photos/ -o cutouts/ [--crop]` (every .jpg/.jpeg/.png/.webp)
//! * `matte --manifest m.json` — opaque hero / portrait / object images are
//!   cut out into `<manifest dir>/matted/<stem>.png`; writes `m.matted.json`
//!   with those entries pointing at the cutouts (`alpha: true`).

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use clap::Args;
use motion_core::assets::AssetManifest;

#[derive(Args, Debug)]
pub struct MatteArgs {
    /// Image file or directory of images (omit with --manifest).
    pub input: Option<PathBuf>,
    /// Output PNG (file input) or directory (directory input).
    #[arg(long, short)]
    pub output: Option<PathBuf>,
    /// AssetManifest whose opaque hero/portrait/object images to cut out.
    #[arg(long)]
    pub manifest: Option<PathBuf>,
    /// Crop the cutout to the subject's bounds.
    #[arg(long)]
    pub crop: bool,
}

/// Manifest roles whose images read best as cutouts.
const CUTOUT_ROLES: [&str; 4] = [
    "hero_subject",
    "hero_object",
    "supporting_object",
    "portrait",
];

fn helper() -> Result<PathBuf> {
    if !cfg!(target_os = "macos") {
        bail!("matting needs macOS 14+ (Apple Vision); on other systems supply cutouts with alpha");
    }
    let root = std::env::var_os("MOTION_TOOLS_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache/motionengine/tools"))
        })
        .context("no HOME for the tools cache")?;
    let bin = root.join("matte_vision");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/matte_vision.swift");
    let stale = match (std::fs::metadata(&bin), std::fs::metadata(&src)) {
        (Ok(b), Ok(s)) => match (b.modified(), s.modified()) {
            (Ok(bm), Ok(sm)) => sm > bm,
            _ => false,
        },
        (Err(_), _) => true,
        _ => false,
    };
    if stale {
        std::fs::create_dir_all(&root)?;
        println!("matte: compiling the Vision helper (once)");
        let status = Command::new("swiftc")
            .arg("-O")
            .arg(&src)
            .arg("-o")
            .arg(&bin)
            .status()
            .context("running swiftc (install the Xcode command line tools)")?;
        if !status.success() {
            bail!("swiftc failed to build {}", src.display());
        }
    }
    Ok(bin)
}

/// Cut out the subject of `input` into the RGBA PNG `output`.
pub fn matte_file(helper: &Path, input: &Path, output: &Path, crop: bool) -> Result<()> {
    if let Some(dir) = output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut c = Command::new(helper);
    c.arg(input).arg(output);
    if crop {
        c.arg("--crop");
    }
    let out = c.output().context("running the Vision helper")?;
    match out.status.code() {
        Some(0) => {
            if magenta_ground(input) {
                despill_magenta(output)?;
            }
            Ok(())
        }
        Some(3) => bail!("{}: no subject found", input.display()),
        _ => bail!(
            "{}: {}",
            input.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ),
    }
}

/// True when `src` is a PNG whose border is mostly the magenta key ground
/// (library generations): its cutout then gets the key-spill cleanup. Real
/// photos (JPEG, or PNG on any other ground) are left untouched.
fn magenta_ground(src: &Path) -> bool {
    if !src
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        return false;
    }
    let Ok(pm) = motion_render::tiny_skia::Pixmap::load_png(src) else {
        return false;
    };
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    if w < 2 || h < 2 {
        return false;
    }
    let px = pm.pixels();
    let border = (0..w)
        .flat_map(|x| [x, (h - 1) * w + x])
        .chain((0..h).flat_map(|y| [y * w, y * w + w - 1]));
    let (mut n, mut magenta) = (0usize, 0usize);
    for i in border {
        let c = px[i].demultiply();
        n += 1;
        if c.red() > 150 && c.blue() > 150 && c.green() < 100 {
            magenta += 1;
        }
    }
    magenta * 10 >= n * 6
}

/// Same rule as scripts/despill_magenta.py: near-ground residue becomes
/// transparent, every visible pixel loses its magenta cast.
fn despill_magenta(path: &Path) -> Result<()> {
    use motion_render::tiny_skia::{ColorU8, Pixmap};
    let mut pm = Pixmap::load_png(path).with_context(|| format!("reading {}", path.display()))?;
    for px in pm.pixels_mut() {
        let c = px.demultiply();
        if c.alpha() == 0 {
            continue;
        }
        let (r, g, b) = (
            f32::from(c.red()) / 255.0,
            f32::from(c.green()) / 255.0,
            f32::from(c.blue()) / 255.0,
        );
        let cast = r.min(b) - g;
        let out = if cast > 0.35 && (r - b).abs() < 0.35 {
            ColorU8::from_rgba(0, 0, 0, 0)
        } else {
            let s = if cast > 0.02 { cast } else { 0.0 };
            let q = |v: f32| ((v - s).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            ColorU8::from_rgba(q(r), c.green(), q(b), c.alpha())
        };
        *px = out.premultiply();
    }
    pm.save_png(path)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn is_image(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(
            e.to_ascii_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp"
        )
    })
}

pub fn run(args: MatteArgs) -> Result<()> {
    let bin = helper()?;
    if let Some(mpath) = &args.manifest {
        let text = std::fs::read_to_string(mpath)
            .with_context(|| format!("reading {}", mpath.display()))?;
        let mut manifest: AssetManifest =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", mpath.display()))?;
        let dir = mpath.parent().unwrap_or(Path::new("."));
        let mut n = 0;
        for e in manifest.assets.iter_mut() {
            let role_ok = CUTOUT_ROLES.iter().any(|r| e.id.ends_with(r));
            if e.alpha || !role_ok {
                continue;
            }
            let src = dir.join(&e.path);
            let stem = Path::new(&e.path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("image");
            let rel = format!("matted/{stem}.png");
            matte_file(&bin, &src, &dir.join(&rel), args.crop)?;
            let (w, h) = image_size(&dir.join(&rel))?;
            e.path = rel;
            e.alpha = true;
            e.width = w;
            e.height = h;
            n += 1;
            println!("matte: {} -> {}", src.display(), e.path);
        }
        let out = mpath.with_extension("matted.json");
        std::fs::write(&out, serde_json::to_string_pretty(&manifest)? + "\n")?;
        println!("matte: {n} image(s) cut out; wrote {}", out.display());
        return Ok(());
    }
    let input = args
        .input
        .context("give an image, a directory, or --manifest")?;
    let output = args.output.context("--output is required")?;
    if input.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&input)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_image(p))
            .collect();
        files.sort();
        for f in &files {
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
            let out = output.join(format!("{stem}.png"));
            matte_file(&bin, f, &out, args.crop)?;
            println!("matte: {} -> {}", f.display(), out.display());
        }
        println!("matte: {} image(s)", files.len());
    } else {
        matte_file(&bin, &input, &output, args.crop)?;
        println!("matte: wrote {}", output.display());
    }
    Ok(())
}

fn image_size(p: &Path) -> Result<(u32, u32)> {
    let bytes = std::fs::read(p)?;
    // PNG IHDR: width/height big-endian at bytes 16..24.
    if bytes.len() >= 24 && &bytes[1..4] == b"PNG" {
        let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        return Ok((w, h));
    }
    bail!("{} is not a PNG", p.display())
}
