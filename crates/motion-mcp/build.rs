//! Bakes the engine version (`git describe`) into the binary: it is part of
//! every job id, so a new engine never serves an old video from the cache.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn main() {
    let describe = git(&["describe", "--always", "--dirty", "--tags"])
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=MOTION_MCP_GIT_DESCRIBE={describe}");
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when HEAD moves (works in worktrees, where .git is a file).
    for what in ["HEAD", "index"] {
        if let Some(path) = git(&["rev-parse", "--path-format=absolute", "--git-path", what]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
