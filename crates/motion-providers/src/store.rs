//! Provider connections in the user config (`connections.json`, mode 0600 in a
//! 0700 directory). Tokens never go in the repo and are never printed in full.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CONNECTIONS_FILE: &str = "connections.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Auth {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_endpoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    /// Adapter name (`openart`, or `mcp` for a bare URL).
    pub adapter: String,
    pub url: String,
    pub auth: Auth,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Connections {
    #[serde(default)]
    pub providers: BTreeMap<String, Connection>,
}

/// `$MOTION_CONFIG_DIR`, else `$XDG_CONFIG_HOME/motionengine`, else `~/.config/motionengine`
/// (the same directory as the TTS `providers.toml`).
pub fn config_dir() -> PathBuf {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(d) = var("MOTION_CONFIG_DIR") {
        return d;
    }
    if let Some(d) = var("XDG_CONFIG_HOME") {
        return d.join("motionengine");
    }
    var("HOME")
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("motionengine")
}

impl Connections {
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
        write_private(path, serde_json::to_string_pretty(self)?.as_bytes())
    }
}

/// Write `bytes` at mode 0600 (parent 0700), replacing atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// First four and last four characters at most.
pub fn mask(token: &str) -> String {
    let n = token.chars().count();
    if n < 16 {
        return "****".into();
    }
    let head: String = token.chars().take(4).collect();
    let tail: String = token.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}
