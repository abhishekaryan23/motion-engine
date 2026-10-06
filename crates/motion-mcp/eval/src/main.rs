//! `motion-mcp-eval`: how well does a small local model drive `motion-mcp`?
//!
//! Dev-only. It starts the MCP server once (stdio, the rmcp client), gives the
//! server's tools to a model behind an OpenAI-compatible endpoint (LM Studio),
//! plays every brief of `briefs.jsonl` as a tool-calling conversation and
//! writes a JSON file and a markdown table of the metrics (plan section 11).
//!
//! A separate package so that `motion-mcp` stays free of network code. It
//! speaks only to the endpoint you pass; it reads no API keys.

mod briefs;
mod mcp;
mod openai;
mod report;
mod runner;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::Parser;

use crate::mcp::{McpSession, ServerSpec};
use crate::openai::{to_openai_tools, HttpChat};
use crate::report::{Meta, ToolInfo};
use crate::runner::{run_brief, RunConfig, SystemMode};

/// Drive motion-mcp with a local model and report success, calls, tokens and time.
#[derive(Parser, Debug)]
#[command(name = "motion-mcp-eval", version)]
struct Cli {
    /// OpenAI-compatible base URL (LM Studio: http://localhost:1234/v1).
    #[arg(long, default_value = "http://localhost:1234/v1")]
    endpoint: String,
    /// Model id as the endpoint knows it (LM Studio: the `--identifier` it was loaded with).
    #[arg(long)]
    model: String,
    /// motion-mcp profile: weak | creator | operator.
    #[arg(long, default_value = "weak")]
    profile: String,
    /// The briefs, one JSON object per line.
    #[arg(long, default_value = "crates/motion-mcp/eval/briefs.jsonl")]
    briefs: PathBuf,
    /// Only these brief ids (comma separated).
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,
    /// The motion-mcp binary.
    #[arg(long, default_value = "target/release/motion-mcp")]
    server: PathBuf,
    /// Repository the server works on (default: the enclosing MotionEngine checkout).
    #[arg(long)]
    repo: Option<PathBuf>,
    /// The server's job store (default <repo>/output/jobs).
    #[arg(long)]
    jobs: Option<PathBuf>,
    /// Check only: every make_video is forwarded with mode "check" and the run
    /// ends at "checked" (story quality, no render, no QA). Fast.
    #[arg(long)]
    no_render: bool,
    /// Most model requests per brief.
    #[arg(long, default_value_t = 6)]
    max_turns: usize,
    #[arg(long, default_value_t = 0.2)]
    temperature: f64,
    /// Longest model answer, in tokens (stops a runaway generation).
    #[arg(long, default_value_t = 3000)]
    max_tokens: u64,
    /// System message: generic ("You are a helpful assistant. …") or none.
    #[arg(long, value_enum, default_value_t = SystemMode::Generic)]
    system: SystemMode,
    /// Where the results go.
    #[arg(long, default_value = "crates/motion-mcp/eval/results")]
    out: PathBuf,
    /// Extra flag for the server, repeatable (e.g. --server-arg=--engine --server-arg=/path/motion-engine).
    #[arg(long = "server-arg", allow_hyphen_values = true)]
    server_arg: Vec<String>,
    /// Date in the result file names (default today, UTC).
    #[arg(long)]
    date: Option<String>,
}

fn absolute(p: &Path) -> PathBuf {
    std::fs::canonicalize(p)
        .or_else(|_| std::path::absolute(p))
        .unwrap_or_else(|_| p.to_path_buf())
}

fn is_repo(dir: &Path) -> bool {
    dir.join("assets").is_dir() && dir.join("crates/motion-cli").is_dir()
}

/// `--repo`, else the enclosing MotionEngine checkout of the working directory.
fn find_repo(cli: Option<&Path>) -> Result<PathBuf> {
    if let Some(r) = cli {
        return Ok(absolute(r));
    }
    let cwd = std::env::current_dir().context("no working directory")?;
    match cwd.ancestors().find(|d| is_repo(d)) {
        Some(dir) => Ok(dir.to_path_buf()),
        None => bail!("cannot find the MotionEngine repository; pass --repo DIR"),
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("motion-mcp-eval: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    if !["weak", "creator", "operator"].contains(&cli.profile.as_str()) {
        bail!("--profile must be weak, creator or operator");
    }
    if cli.max_turns == 0 {
        bail!("--max-turns must be at least 1");
    }
    let all = briefs::load(&cli.briefs)?;
    let selected = briefs::select(all, &cli.only)?;
    if selected.is_empty() {
        bail!("no briefs to run");
    }

    let repo = find_repo(cli.repo.as_deref())?;
    let jobs = absolute(&cli.jobs.clone().unwrap_or_else(|| repo.join("output/jobs")));
    let server = absolute(&cli.server);
    if !server.is_file() {
        bail!(
            "no server binary at {}; build it: cargo build --release -p motion-mcp",
            server.display()
        );
    }
    let spec = ServerSpec {
        server: server.clone(),
        profile: cli.profile.clone(),
        repo,
        jobs: jobs.clone(),
        extra: cli.server_arg.clone(),
    };
    let session = McpSession::start(&spec).await?;
    let outcome = play(&cli, &selected, &session, jobs, &server).await;
    session.close().await;
    outcome
}

async fn play(
    cli: &Cli,
    selected: &[briefs::Brief],
    session: &McpSession,
    jobs: PathBuf,
    server: &Path,
) -> Result<()> {
    let mcp_tools = session.tools().await?;
    let tools = to_openai_tools(&mcp_tools);
    let chars = serde_json::to_string(&mcp_tools)
        .map(|s| s.chars().count())
        .unwrap_or(0);
    let names: Vec<String> = mcp_tools
        .iter()
        .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
        .map(str::to_string)
        .collect();
    eprintln!(
        "motion-mcp-eval: {} tools ({}), {chars} chars of tool definitions",
        names.len(),
        names.join(", ")
    );

    let cfg = RunConfig {
        model: cli.model.clone(),
        temperature: cli.temperature,
        seed: 7,
        max_tokens: cli.max_tokens,
        max_turns: cli.max_turns,
        system: cli.system,
        no_render: cli.no_render,
        jobs,
    };
    let chat = HttpChat::new(&cli.endpoint);
    let measured_tokens = runner::probe_tool_tokens(&chat, &cfg, &tools, &selected[0]).await;
    if let Some(t) = measured_tokens {
        eprintln!("motion-mcp-eval: the tool definitions cost {t} prompt tokens on this model");
    }

    let date = cli.date.clone().unwrap_or_else(report::today);
    let meta = Meta {
        date: date.clone(),
        model: cli.model.clone(),
        endpoint: cli.endpoint.clone(),
        profile: cli.profile.clone(),
        no_render: cli.no_render,
        system: cli.system.name().to_string(),
        temperature: cli.temperature,
        seed: cfg.seed,
        max_turns: cli.max_turns,
        max_tokens: cli.max_tokens,
        server: server.display().to_string(),
    };
    let stem = report::result_stem(&date, &cli.model, &cli.profile, cli.no_render);

    let mut results = Vec::new();
    for (i, brief) in selected.iter().enumerate() {
        let r = run_brief(&chat, session, &cfg, &tools, brief).await;
        eprintln!(
            "[{}/{}] {:<24} {} · {} calls · {:.1} s{}",
            i + 1,
            selected.len(),
            r.id,
            if r.success { "ok  " } else { "FAIL" },
            r.tool_calls,
            r.wall_s,
            if r.note.is_empty() {
                String::new()
            } else {
                format!(" · {}", r.note)
            }
        );
        results.push(r);
        // Written after every brief: a long run that is interrupted keeps its results.
        let info = tool_info(&names, chars, measured_tokens, &results);
        let summary = report::summarize(&results, &info);
        let json = report::to_json(&meta, &info, &summary, &results);
        let md = report::to_markdown(&meta, &info, &summary, &results);
        let (json_path, md_path) = report::write_results(&cli.out, &stem, &json, &md)
            .with_context(|| format!("cannot write into {}", cli.out.display()))?;
        if i + 1 == selected.len() {
            println!("{md}");
            eprintln!(
                "motion-mcp-eval: wrote {} and {}",
                json_path.display(),
                md_path.display()
            );
        }
    }
    Ok(())
}

fn tool_info(
    names: &[String],
    chars: usize,
    measured_tokens: Option<u64>,
    results: &[runner::BriefResult],
) -> ToolInfo {
    ToolInfo {
        tool_count: names.len(),
        names: names.to_vec(),
        chars,
        measured_tokens,
        first_request_prompt_tokens: results.first().and_then(|r| r.first_prompt_tokens),
    }
}
