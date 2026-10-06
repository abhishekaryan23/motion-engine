//! `sfx-index` and `sfx-pack` (0.8).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use motion_core::audio::{SfxCuration, LIBRARY_FILE};
use motion_render::sfx::{build_pack, index_pack, IndexOptions};

pub fn cmd_sfx_index(
    pack: &Path,
    curation: &Path,
    output: &Path,
    cache: Option<&Path>,
    json: bool,
) -> Result<()> {
    let text = std::fs::read_to_string(curation)
        .with_context(|| format!("reading {}", curation.display()))?;
    let curation: SfxCuration =
        serde_json::from_str(&text).with_context(|| "parsing curation json".to_string())?;
    let opts = IndexOptions {
        cache_dir: cache.map(Path::to_path_buf),
    };
    let report = index_pack(pack, &curation, output, &opts)?;

    let mut families: BTreeMap<&'static str, usize> = BTreeMap::new();
    for s in &report.library.sounds {
        *families.entry(s.family.as_str()).or_default() += 1;
    }
    let n = report.library.sounds.len();

    if json {
        let failed: Vec<_> = report
            .failed
            .iter()
            .map(|(id, error)| serde_json::json!({ "id": id, "error": error }))
            .collect();
        let value = serde_json::json!({
            "library_path": output.join(LIBRARY_FILE).display().to_string(),
            "measured": report.measured,
            "cached": report.cached,
            "families": families,
            "failed": failed,
        });
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!(
            "indexed {n} sound(s) ({} measured, {} cached) -> {}",
            report.measured,
            report.cached,
            output.display()
        );
        if !families.is_empty() {
            let line: Vec<String> = families.iter().map(|(f, c)| format!("{f}={c}")).collect();
            println!("families: {}", line.join(" "));
        }
        for (id, err) in &report.failed {
            println!("failed {id}: {err}");
        }
    }
    if n == 0 {
        anyhow::bail!("sfx-index: library is empty (no curated sound could be indexed)");
    }
    Ok(())
}

pub fn cmd_sfx_pack(library: &Path, output: &Path) -> Result<()> {
    let n = build_pack(library, output)?;
    println!("packed {n} file(s) -> {}", output.display());
    Ok(())
}
