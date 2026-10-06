//! `motion-mcp`: MotionEngine as MCP tools over stdio.
//!
//! stdout is the MCP channel: everything this program logs goes to stderr,
//! and every subprocess has its output piped, never inherited.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use rmcp::ServiceExt;

use motion_mcp::engine::SubprocessEngine;
use motion_mcp::profile::{self, Profile, ServerConfig};
use motion_mcp::server::McpServer;
use motion_mcp::service::Service;

/// MotionEngine as MCP tools (stdio). Profiles: weak (default, 4 tools, lite
/// story), creator (full intent + style + options), operator (everything).
#[derive(Parser, Debug)]
#[command(name = "motion-mcp", version)]
struct Cli {
    /// Who is calling: weak | creator | operator.
    #[arg(long, value_enum)]
    profile: Option<Profile>,
    /// Repository root (default: $MOTION_REPO, else the enclosing MotionEngine
    /// checkout of the current directory, else the one this binary was built in).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// The motion-engine binary (default <repo>/target/release/motion-engine).
    #[arg(long)]
    engine: Option<PathBuf>,
    /// Asset library root (default <repo>/assets).
    #[arg(long)]
    assets: Option<PathBuf>,
    /// Folder user images may be referenced from (repeatable; default <repo>/assets/inbox).
    #[arg(long = "asset-root")]
    asset_root: Vec<PathBuf>,
    /// Job store (default <repo>/output/jobs).
    #[arg(long)]
    jobs: Option<PathBuf>,
    /// Operator only: list and allow apply_scene_patch.
    #[arg(long)]
    allow_scene_edits: bool,
    /// Longest a make_video / revise_video call waits before replying "running".
    #[arg(long)]
    max_wait_s: Option<u64>,
    /// Extra flag for every `motion-engine reel` call (repeatable, e.g. --reel-arg=--offline).
    #[arg(long = "reel-arg", allow_hyphen_values = true)]
    reel_arg: Vec<String>,
    /// Voice model for every video: `auto` (default; the free pinned male narrator).
    /// Weak and creator also accept `say` (the macOS offline voice, for tests and
    /// offline demos); operator accepts anything `motion-engine reel --tts-model` does.
    #[arg(long, value_name = "MODEL", default_value = "auto")]
    tts_model: String,
}

/// The voice model a profile may start with: weak and creator only `auto` (the
/// free pinned narrator) or `say` (the offline macOS voice); operator anything
/// the engine accepts (it names a bad one itself), so long as it cannot pass
/// for a flag.
fn check_tts_model(profile: Profile, model: &str) -> Result<String, String> {
    let model = model.trim();
    if profile == Profile::Operator {
        if model.is_empty() || model.starts_with('-') || model.contains(char::is_whitespace) {
            return Err(format!("--tts-model '{model}' is not a voice model name"));
        }
        return Ok(model.to_string());
    }
    match model {
        "auto" | "say" => Ok(model.to_string()),
        other => Err(format!(
            "--tts-model '{other}' is not allowed in the {} profile (auto or say); \
             paid voices need --profile operator",
            profile.name()
        )),
    }
}

/// An absolute path (canonical when it exists).
fn absolute(p: &Path) -> PathBuf {
    std::fs::canonicalize(p)
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf())
}

fn is_repo(dir: &Path) -> bool {
    dir.join("assets").is_dir() && dir.join("crates/motion-cli").is_dir()
}

/// `--repo`, else `$MOTION_REPO`, else a parent of the current directory that
/// looks like the repository, else `<repo>/target/<profile>/motion-mcp`.
fn find_repo(cli: Option<&Path>) -> Option<PathBuf> {
    if let Some(r) = cli {
        return Some(r.to_path_buf());
    }
    if let Some(r) = std::env::var_os("MOTION_REPO").filter(|r| !r.is_empty()) {
        return Some(PathBuf::from(r));
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(dir) = cwd.ancestors().find(|d| is_repo(d)) {
            return Some(dir.to_path_buf());
        }
    }
    let exe = std::env::current_exe().ok()?;
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    exe.parent()?.parent()?.parent().map(Path::to_path_buf)
}

fn config_of(cli: Cli) -> Result<ServerConfig, String> {
    let repo = find_repo(cli.repo.as_deref())
        .ok_or("cannot find the MotionEngine repository; pass --repo DIR or set MOTION_REPO")?;
    let repo = absolute(&repo);
    if !repo.is_dir() {
        return Err(format!("repository {} does not exist", repo.display()));
    }
    let profile = cli.profile.unwrap_or_default();
    let tts_model = check_tts_model(profile, &cli.tts_model)?;
    let mut config = ServerConfig::new(profile, &repo);
    config.tts_model = tts_model;
    if let Some(e) = cli.engine {
        config.engine = absolute(&e);
    }
    if let Some(a) = cli.assets {
        config.assets = absolute(&a);
    }
    if !cli.asset_root.is_empty() {
        config.asset_roots = cli.asset_root.iter().map(|r| absolute(r)).collect();
    }
    if let Some(j) = cli.jobs {
        config.jobs = absolute(&j);
    }
    config.allow_scene_edits = cli.allow_scene_edits;
    if let Some(w) = cli.max_wait_s {
        config.max_wait_s = w;
    }
    config.reel_extra = cli.reel_arg;
    if !config.engine.is_file() {
        return Err(format!(
            "no engine binary at {}; build it: cargo build --release -p motion-cli",
            config.engine.display()
        ));
    }
    config.engine_version = profile::engine_version(&config.engine);
    Ok(config)
}

/// SIGINT or SIGTERM.
async fn stop_signal() {
    use tokio::signal::unix::{signal, SignalKind};
    match signal(SignalKind::terminate()) {
        Ok(mut term) => {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match config_of(Cli::parse()) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("motion-mcp: {msg}");
            return ExitCode::from(2);
        }
    };
    let banner = format!(
        "motion-mcp: profile {}, repo {}, engine {}, jobs {}",
        config.profile.name(),
        config.repo.display(),
        config.engine.display(),
        config.jobs.display()
    );
    let service = match Service::new(config, Arc::new(SubprocessEngine)) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("motion-mcp: cannot open the job store: {e}");
            return ExitCode::FAILURE;
        }
    };
    eprintln!("{banner}");
    let server = McpServer::new(Arc::clone(&service));
    let code = match server.serve(rmcp::transport::stdio()).await {
        Ok(running) => {
            tokio::select! {
                done = running.waiting() => match done {
                    Ok(_) => ExitCode::SUCCESS,
                    Err(e) => {
                        eprintln!("motion-mcp: {e}");
                        ExitCode::FAILURE
                    }
                },
                _ = stop_signal() => ExitCode::SUCCESS,
            }
        }
        Err(e) => {
            eprintln!("motion-mcp: cannot start the MCP session: {e}");
            ExitCode::FAILURE
        }
    };
    // Stop a render still in progress (its process group) before exiting.
    let stopping = Arc::clone(&service);
    let _ = tokio::task::spawn_blocking(move || stopping.shutdown()).await;
    code
}
