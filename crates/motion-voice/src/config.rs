//! Provider API-key resolution and the user config file.
//!
//! Resolution order for OpenRouter: env `OPENROUTER_API_KEY` (non-empty) →
//! `providers.toml` `[openrouter] api_key = "..."`. The config directory is
//! `$MOTION_CONFIG_DIR` (tests / sandboxes), else `$XDG_CONFIG_HOME/motionengine`,
//! else `~/.config/motionengine`. The file is written 0600 in a 0700 directory.
//! Keys are never printed in full: use [`mask`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::VoiceError;

pub const OPENROUTER: &str = "openrouter";
pub const OPENROUTER_ENV: &str = "OPENROUTER_API_KEY";
pub const CONFIG_FILE: &str = "providers.toml";

/// Where a resolved key came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Env,
    Config,
}

impl KeySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Config => "config",
        }
    }
}

/// Env var holding a provider's key.
pub fn env_var_for(provider: &str) -> Option<&'static str> {
    (provider == OPENROUTER).then_some(OPENROUTER_ENV)
}

/// Default config file path from the real environment.
pub fn default_config_path() -> PathBuf {
    config_path_from(
        std::env::var_os("MOTION_CONFIG_DIR").map(PathBuf::from),
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// Pure path resolution (testable without touching the process environment).
pub fn config_path_from(
    motion_config_dir: Option<PathBuf>,
    xdg_config_home: Option<PathBuf>,
    home: Option<PathBuf>,
) -> PathBuf {
    let nonempty = |p: &Option<PathBuf>| p.as_ref().is_some_and(|p| !p.as_os_str().is_empty());
    if nonempty(&motion_config_dir) {
        return motion_config_dir.unwrap_or_default().join(CONFIG_FILE);
    }
    if nonempty(&xdg_config_home) {
        return xdg_config_home
            .unwrap_or_default()
            .join("motionengine")
            .join(CONFIG_FILE);
    }
    home.unwrap_or_else(|| PathBuf::from("."))
        .join(".config")
        .join("motionengine")
        .join(CONFIG_FILE)
}

/// Resolve a key from the real process environment, then `config_path`.
pub fn resolve_key(provider: &str, config_path: &Path) -> Option<(String, KeySource)> {
    let env = env_var_for(provider).and_then(|v| std::env::var(v).ok());
    resolve_key_with(provider, env, config_path)
}

/// Env value (if any) takes precedence over the config file; empty values do not count.
pub fn resolve_key_with(
    provider: &str,
    env_value: Option<String>,
    config_path: &Path,
) -> Option<(String, KeySource)> {
    if let Some(v) = env_value {
        let v = v.trim();
        if !v.is_empty() {
            return Some((v.to_string(), KeySource::Env));
        }
    }
    let doc = read_doc(config_path).ok()?;
    let sectioned = doc.get(provider).and_then(|s| s.get("api_key"));
    // Owner-style top-level `OPENROUTER_API_KEY = "..."` line (before any section).
    let top_level = env_var_for(provider).and_then(|var| doc.get("").and_then(|s| s.get(var)));
    let found = [sectioned, top_level]
        .into_iter()
        .flatten()
        .map(|raw| unquote(raw))
        .find(|k| !k.is_empty());
    found.map(|k| (k, KeySource::Config))
}

/// A warning when the config file is readable by group/others (never chmods).
pub fn permission_warning(config_path: &Path) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(config_path).ok()?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            return Some(format!(
                "{} is mode {mode:04o}, wider than 0600; run `chmod 600 {}`",
                config_path.display(),
                config_path.display()
            ));
        }
    }
    let _ = config_path;
    None
}

/// `sk-or-…` plus at most the last four characters (only when the key is long
/// enough that four characters reveal little).
pub fn mask(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() >= 12 {
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("sk-or-…{tail}")
    } else {
        "sk-or-…".to_string()
    }
}

/// Write `[provider] api_key = "key"` to `config_path`, creating the directory
/// (0700) and file (0600), overwriting the provider's key, preserving every
/// other section and key.
pub fn set_key(provider: &str, key: &str, config_path: &Path) -> Result<(), VoiceError> {
    let key = key.trim();
    if key.is_empty() || key.contains(['\n', '\r']) {
        return Err(VoiceError::Audio("api key is empty or multi-line".into()));
    }
    let existing = std::fs::read_to_string(config_path).unwrap_or_default();
    let updated = upsert_api_key(&existing, provider, &quote(key));
    if let Some(dir) = config_path.parent() {
        create_private_dir(dir)?;
    }
    write_private(config_path, updated.as_bytes())?;
    Ok(())
}

/// Replace (or add) `api_key` in `[provider]`, leaving every other line as is.
fn upsert_api_key(text: &str, provider: &str, quoted: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let header = format!("[{provider}]");
    let new_line = format!("api_key = {quoted}");
    let is_header = |l: &str| {
        let t = l.trim();
        t.starts_with('[') && t.ends_with(']')
    };
    match lines.iter().position(|l| l.trim() == header) {
        Some(h) => {
            let end = lines[h + 1..]
                .iter()
                .position(|l| is_header(l))
                .map_or(lines.len(), |p| h + 1 + p);
            let existing = (h + 1..end).find(|&i| {
                lines[i]
                    .split_once('=')
                    .is_some_and(|(k, _)| k.trim() == "api_key")
            });
            match existing {
                Some(i) => lines[i] = new_line,
                None => lines.insert(h + 1, new_line),
            }
        }
        None => {
            if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push(header);
            lines.push(new_line);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

// ---- tiny TOML subset: [section] and key = value, values kept as raw text ----

type Doc = BTreeMap<String, BTreeMap<String, String>>;

fn read_doc(path: &Path) -> Result<Doc, VoiceError> {
    let text = std::fs::read_to_string(path)?;
    Ok(parse_doc(&text))
}

fn parse_doc(text: &str) -> Doc {
    let mut doc = Doc::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            if let Some(name) = rest.strip_suffix(']') {
                section = name.trim().to_string();
                doc.entry(section.clone()).or_default();
            }
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            doc.entry(section.clone())
                .or_default()
                .insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    doc
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn unquote(raw: &str) -> String {
    let raw = raw.trim();
    match raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(inner) => {
            let mut out = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    if let Some(n) = chars.next() {
                        out.push(n);
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }
        None => raw.trim_matches('\'').to_string(),
    }
}

#[cfg(unix)]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
pub(crate) mod testutil {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Unique temp dir, removed on drop.
    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "motion-voice-test-{}-{}-{tag}",
                std::process::id(),
                N.fetch_add(1, Ordering::SeqCst)
            ));
            std::fs::create_dir_all(&p).expect("tempdir");
            Self(p)
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempDir;
    use super::*;

    const FAKE: &str = "sk-or-v1-abcdef0123456789wxyz";

    #[test]
    fn env_beats_config_and_empty_env_falls_through() {
        let dir = TempDir::new("cfg");
        let path = dir.path().join("motionengine").join(CONFIG_FILE);
        set_key(OPENROUTER, "sk-or-v1-fromconfigfile1234", &path).unwrap();

        let (k, src) = resolve_key_with(OPENROUTER, Some(FAKE.into()), &path).unwrap();
        assert_eq!((k.as_str(), src), (FAKE, KeySource::Env));

        for empty in [Some(String::new()), Some("   ".into()), None] {
            let (k, src) = resolve_key_with(OPENROUTER, empty, &path).unwrap();
            assert_eq!(src, KeySource::Config);
            assert_eq!(k, "sk-or-v1-fromconfigfile1234");
        }
    }

    #[test]
    fn no_key_anywhere_is_none() {
        let dir = TempDir::new("nokey");
        let path = dir.path().join(CONFIG_FILE);
        assert!(resolve_key_with(OPENROUTER, None, &path).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn written_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("perm");
        let path = dir
            .path()
            .join("new")
            .join("motionengine")
            .join(CONFIG_FILE);
        set_key(OPENROUTER, FAKE, &path).unwrap();
        let file_mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        // Overwriting keeps 0600.
        set_key(OPENROUTER, "sk-or-v1-second-key-9999", &path).unwrap();
        let again = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(again, 0o600);
    }

    #[test]
    fn mask_never_reveals_more_than_four_chars() {
        let m = mask(FAKE);
        assert_eq!(m, "sk-or-…wxyz");
        assert!(!m.contains("abcdef"));
        assert_eq!(mask("short"), "sk-or-…");
        assert_eq!(mask(""), "sk-or-…");
    }

    #[test]
    fn round_trip_preserves_other_sections_and_overwrites() {
        let dir = TempDir::new("rt");
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(
            &path,
            "# my config\n[other]\nfoo = \"bar\"\nn = 3\n\n[openrouter]\napi_key = \"old\"\nextra = \"keep\"\n",
        )
        .unwrap();
        set_key(OPENROUTER, "sk-or-v1-newnewnewnew1234", &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my config\n[other]\nfoo = \"bar\"\nn = 3\n"));
        let doc = parse_doc(&text);
        assert_eq!(doc["other"]["foo"], "\"bar\"");
        assert_eq!(doc["other"]["n"], "3");
        assert_eq!(doc["openrouter"]["extra"], "\"keep\"");
        assert_eq!(text.matches("api_key").count(), 1);
        let (k, _) = resolve_key_with(OPENROUTER, None, &path).unwrap();
        assert_eq!(k, "sk-or-v1-newnewnewnew1234");
    }

    #[test]
    fn set_key_appends_section_and_keeps_unrelated_lines() {
        let dir = TempDir::new("append");
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(
            &path,
            "# note\nOPENROUTER_API_KEY = \"top-level-key-1234\"\n[a]\nx = 1",
        )
        .unwrap();
        set_key(OPENROUTER, "sk-or-v1-appendedkey5678", &path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.starts_with("# note\nOPENROUTER_API_KEY = \"top-level-key-1234\"\n[a]\nx = 1\n")
        );
        // The section key now wins over the top-level line.
        let (k, _) = resolve_key_with(OPENROUTER, None, &path).unwrap();
        assert_eq!(k, "sk-or-v1-appendedkey5678");
    }

    #[test]
    fn top_level_openrouter_api_key_line_is_accepted() {
        let dir = TempDir::new("top");
        let path = dir.path().join(CONFIG_FILE);
        for (body, want) in [
            ("OPENROUTER_API_KEY = \"sk-or-v1-quotedquoted0001\"\n", "sk-or-v1-quotedquoted0001"),
            ("OPENROUTER_API_KEY=sk-or-v1-unquotedunq0002\n", "sk-or-v1-unquotedunq0002"),
            (
                "OPENROUTER_API_KEY = \"sk-or-v1-toplevelloses03\"\n[openrouter]\napi_key = \"sk-or-v1-sectionwins0003\"\n",
                "sk-or-v1-sectionwins0003",
            ),
        ] {
            std::fs::write(&path, body).unwrap();
            let (k, src) = resolve_key_with(OPENROUTER, None, &path).unwrap();
            assert_eq!((k.as_str(), src), (want, KeySource::Config));
        }
        // Env still beats everything.
        let (k, src) = resolve_key_with(OPENROUTER, Some(FAKE.into()), &path).unwrap();
        assert_eq!((k.as_str(), src), (FAKE, KeySource::Env));
        // A top-level line after a section header is NOT the top-level key.
        std::fs::write(&path, "[other]\nOPENROUTER_API_KEY = \"nope\"\n").unwrap();
        assert!(resolve_key_with(OPENROUTER, None, &path).is_none());
    }

    #[test]
    #[cfg(unix)]
    fn wide_permissions_warn_but_are_not_changed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new("warn");
        let path = dir.path().join(CONFIG_FILE);
        std::fs::write(&path, "OPENROUTER_API_KEY = \"x\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let w = permission_warning(&path).unwrap();
        assert!(w.contains("0644") && w.contains("chmod 600"), "{w}");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o644, "must not chmod on read");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(permission_warning(&path).is_none());
        assert!(permission_warning(&dir.path().join("missing")).is_none());
    }

    #[test]
    fn quoting_round_trips() {
        let dir = TempDir::new("q");
        let path = dir.path().join(CONFIG_FILE);
        set_key(OPENROUTER, r#"ab"c\d"#, &path).unwrap();
        let (k, _) = resolve_key_with(OPENROUTER, None, &path).unwrap();
        assert_eq!(k, r#"ab"c\d"#);
    }

    #[test]
    fn config_path_precedence() {
        let p = |s: &str| Some(PathBuf::from(s));
        assert_eq!(
            config_path_from(p("/m"), p("/x"), p("/h")),
            PathBuf::from("/m/providers.toml")
        );
        assert_eq!(
            config_path_from(None, p("/x"), p("/h")),
            PathBuf::from("/x/motionengine/providers.toml")
        );
        assert_eq!(
            config_path_from(None, None, p("/h")),
            PathBuf::from("/h/.config/motionengine/providers.toml")
        );
    }
}
