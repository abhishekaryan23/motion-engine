//! `style-preview` and `compare-styles`: inspect what a StyleProfile resolves to.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use motion_core::compiler::preview::{describe, style_preview};
use motion_core::compiler::taste;
use motion_core::compiler::FontSet;
use motion_core::{evaluate_frame, validate, AssetLibrary, StyleProfile};
use motion_render::{CpuRenderer, FontMeasure, Renderer};

fn read_style_file(path: &Path) -> Result<StyleProfile> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    StyleProfile::from_json(&text).with_context(|| format!("parsing style {}", path.display()))
}

/// Render frame 0 of the style preview to `output` (PNG) and print either the
/// human description or the resolved profile JSON.
pub fn cmd_style_preview(
    style_path: &Path,
    output: &Path,
    assets: &Path,
    json: bool,
) -> Result<()> {
    let style = read_style_file(style_path)?;
    if !assets.is_dir() {
        bail!(
            "asset library '{}' not found (use --assets)",
            assets.display()
        );
    }
    let assets_abs = assets.canonicalize()?;
    let library = AssetLibrary::new(&assets_abs);

    // Measure with the fonts the resolved typography pairing will render with.
    let pairing = taste::resolve(&style).typography;
    let font_paths: Vec<PathBuf> = FontSet::for_pairing(pairing)
        .faces
        .iter()
        .map(|f| assets_abs.join(f.path))
        .collect();
    let measure = FontMeasure::with_fonts(font_paths.iter().map(PathBuf::as_path))?;

    let (mut project, resolved) = style_preview(&style, &library, &measure);
    // An absolute asset root: joining it onto any base directory keeps it as is.
    project.asset_root = Some(assets_abs.to_string_lossy().into_owned());
    validate(&project, Some(Path::new(".")))
        .map_err(|e| anyhow::anyhow!("style preview failed validation:\n{e}"))?;

    let renderer = CpuRenderer::new(&project, Path::new("."))?;
    let frame = evaluate_frame(&project, 0)?;
    let pixmap = renderer.render(&frame)?;
    if let Some(dir) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    pixmap
        .save_png(output)
        .map_err(|e| anyhow::anyhow!("writing {}: {e}", output.display()))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&resolved)?);
    } else {
        print!("{}", describe(&resolved));
        println!("Preview: {}", output.display());
    }
    Ok(())
}

/// Resolve both styles and print the per-dimension comparison. Always Ok:
/// differing styles are a result, not a failure.
pub fn cmd_compare_styles(a_path: &Path, b_path: &Path, json: bool) -> Result<()> {
    let a = taste::resolve(&read_style_file(a_path)?).fingerprint();
    let b = taste::resolve(&read_style_file(b_path)?).fingerprint();
    let diffs = a.compare(&b);
    let differing = diffs.iter().filter(|d| d.different).count();
    if json {
        let value = serde_json::json!({
            "a": a_path.display().to_string(),
            "b": b_path.display().to_string(),
            "differing": differing,
            "total": diffs.len(),
            "dimensions": diffs,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        for d in &diffs {
            println!(
                "{:<20} {}",
                title_case(d.dimension),
                if d.different { "DIFFERENT" } else { "same" }
            );
        }
        println!("{differing}/{} dimensions differ", diffs.len());
    }
    Ok(())
}

/// `image_treatment` -> `Image treatment`.
fn title_case(dimension: &str) -> String {
    let spaced = dimension.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
