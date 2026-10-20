//! (v2.3) Generated assets of a program, pinned: `<stem>.assets.lock.json`
//! beside the program maps each `GenAsset` id to the file the fallback made.
//! Compile, QA and validate read it and never call a provider; an entry whose
//! file still exists with its sha256 is reused without a model call.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::pipeline::{ImageSpec, Resolved};

pub const LOCK_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LockEntry {
    pub provider: String,
    pub model: String,
    pub seed: u32,
    /// sha256 of (prompt, cutout, aspect): a changed prompt is regenerated.
    pub prompt_hash: String,
    pub sha256: String,
    /// Absolute path in the user asset cache.
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lockfile {
    pub version: u32,
    #[serde(default)]
    pub assets: BTreeMap<String, LockEntry>,
}

impl Default for Lockfile {
    fn default() -> Self {
        Self {
            version: LOCK_VERSION,
            assets: BTreeMap::new(),
        }
    }
}

/// `<dir>/<stem>.assets.lock.json` for `<dir>/<stem>.program.json` (or `<stem>.json`).
pub fn lock_path(program: &Path) -> PathBuf {
    let name = program
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name
        .strip_suffix(".program.json")
        .or_else(|| name.strip_suffix(".json"))
        .unwrap_or(&name);
    program.with_file_name(format!("{stem}.assets.lock.json"))
}

pub fn prompt_hash(spec: &ImageSpec) -> String {
    let canon = json!({
        "prompt": spec.prompt.trim(),
        "cutout": spec.cutout,
        "aspect": spec.aspect,
    });
    hex(&Sha256::digest(canon.to_string().as_bytes()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

impl Lockfile {
    /// A missing file is an empty lockfile.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                serde_json::from_str(&s).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string_pretty(self)? + "\n")
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The entry for `id` if it still describes `spec` and its file is intact.
    pub fn valid(&self, id: &str, spec: &ImageSpec) -> Option<&LockEntry> {
        let e = self.assets.get(id)?;
        (e.seed == spec.seed
            && e.prompt_hash == prompt_hash(spec)
            && sha256_file(Path::new(&e.path)).ok().as_deref() == Some(e.sha256.as_str()))
        .then_some(e)
    }

    /// id → absolute path of every intact entry (what compile resolves `gen:` with).
    /// Read-only: no provider is called.
    pub fn paths(&self) -> BTreeMap<String, String> {
        self.assets
            .iter()
            .filter(|(_, e)| {
                sha256_file(Path::new(&e.path)).ok().as_deref() == Some(e.sha256.as_str())
            })
            .map(|(id, e)| (id.clone(), e.path.clone()))
            .collect()
    }
}

/// How one asset was resolved.
#[derive(Debug, Clone, PartialEq)]
pub enum LockAction {
    Reused,
    Made(Resolved),
}

/// Resolve every `(id, spec)`: intact lock entries are reused, the rest are
/// made with `make` (the fallback + cache) unless `offline`, which fails on
/// the first missing one. Stale entries of ids no longer listed are dropped.
pub fn resolve_all(
    lock: &mut Lockfile,
    specs: &[(String, ImageSpec)],
    offline: bool,
    make: &mut dyn FnMut(&str, &ImageSpec) -> Result<Resolved>,
) -> Result<Vec<(String, LockAction)>> {
    let mut out = Vec::new();
    for (id, spec) in specs {
        if lock.valid(id, spec).is_some() {
            out.push((id.clone(), LockAction::Reused));
            continue;
        }
        if offline {
            bail!("generated asset '{id}' is not in the lockfile (or its file changed); run `program assets` without --offline");
        }
        let r = make(id, spec).with_context(|| format!("generated asset '{id}'"))?;
        let path = r.path.canonicalize().unwrap_or_else(|_| r.path.clone());
        lock.assets.insert(
            id.clone(),
            LockEntry {
                provider: r.provider.clone(),
                model: r.model.clone(),
                seed: spec.seed,
                prompt_hash: prompt_hash(spec),
                sha256: sha256_file(&path)?,
                path: path.to_string_lossy().into_owned(),
            },
        );
        out.push((id.clone(), LockAction::Made(r)));
    }
    lock.assets
        .retain(|id, _| specs.iter().any(|(i, _)| i == id));
    Ok(out)
}
