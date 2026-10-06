//! Shared test setup: a server config over the real repository and library,
//! with the job store (image cache) in the test's own temporary folder and
//! the user-image root at `tests/fixtures/inbox`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use motion_mcp::pictures::PictureIndex;
use motion_mcp::policy::Context;
use motion_mcp::profile::{Profile, ServerConfig};

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

pub fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// The real library index (loaded once per test binary).
pub fn pictures() -> &'static PictureIndex {
    static INDEX: OnceLock<PictureIndex> = OnceLock::new();
    INDEX.get_or_init(|| PictureIndex::load(&repo().join("assets")))
}

/// `MOTION_ENGINE` when set (a built release engine), else the default path.
pub fn engine() -> PathBuf {
    std::env::var_os("MOTION_ENGINE")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo().join("target/release/motion-engine"))
}

pub fn config(profile: Profile, name: &str) -> ServerConfig {
    let mut c = ServerConfig::new(profile, repo());
    c.asset_roots = vec![fixtures().join("inbox")];
    c.jobs = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("motion-mcp-tests")
        .join(name)
        .join("jobs");
    c.engine = engine();
    c.engine_version = "test-engine".into();
    c
}

pub fn ctx(config: &ServerConfig) -> Context<'_> {
    Context {
        config,
        pictures: pictures(),
    }
}

/// True on macOS with `swiftc` (Apple Vision helper tests).
pub fn has_vision() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("swiftc")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
}
