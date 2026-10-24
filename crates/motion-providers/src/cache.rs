//! Content-addressed asset cache: the same prompt + model + settings is never paid twice.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::jsonx::norm;
use crate::provider::AssetRequest;

pub struct AssetCache {
    pub dir: PathBuf,
}

impl AssetCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// `$MOTION_ASSET_CACHE`, else `~/.cache/motionengine/assets`.
    pub fn default_dir() -> PathBuf {
        if let Some(d) = std::env::var_os("MOTION_ASSET_CACHE").filter(|v| !v.is_empty()) {
            return PathBuf::from(d);
        }
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cache/motionengine/assets")
    }

    /// sha256 over provider, normalized model, kind, prompt, quality, aspect and params.
    pub fn key(provider: &str, req: &AssetRequest) -> String {
        let canon = json!({
            "v": 1,
            "provider": provider,
            "model": norm(&req.model),
            "kind": req.kind,
            "prompt": req.prompt.trim(),
            "quality": req.quality.as_deref().map(norm),
            "aspect": req.aspect,
            "params": req.params, // BTreeMap: sorted keys
        });
        let h = Sha256::digest(canon.to_string().as_bytes());
        h.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn entry_dir(&self, key: &str) -> PathBuf {
        self.dir.join(&key[..2]).join(key)
    }

    /// The cached asset file, if any.
    pub fn lookup(&self, key: &str) -> Option<PathBuf> {
        let dir = self.entry_dir(key);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .ok()?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.file_stem().is_some_and(|s| s == "asset") && is_complete(p))
            .collect();
        files.sort();
        files.into_iter().next()
    }

    /// Store `bytes` as `asset.<ext>` with `meta.json`; returns the asset path.
    pub fn store(
        &self,
        key: &str,
        ext: &str,
        bytes: &[u8],
        meta: &serde_json::Value,
    ) -> Result<PathBuf> {
        let dir = self.entry_dir(key);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("meta.json"), serde_json::to_string_pretty(meta)?)?;
        let path = dir.join(format!("asset.{ext}"));
        let tmp = dir.join(format!(".asset.{ext}.part"));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(path)
    }
}

fn is_complete(p: &Path) -> bool {
    p.metadata().is_ok_and(|m| m.is_file() && m.len() > 0)
}
