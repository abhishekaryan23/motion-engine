//! End to end: the real `motion-mcp` binary over stdio with the REAL
//! `motion-engine`, rendering one 2-beat video with the offline macOS voice.
//!
//! Slow (a real render) and machine-dependent, so it is ignored by default:
//!
//! ```text
//! cargo build --release -p motion-cli --features asr-local,asr-ctc
//! cargo test -p motion-mcp --test e2e_render -- --ignored --nocapture
//! ```
//!
//! Needs, and skips with a printed reason without: macOS (`say`), the release
//! engine at `<repo>/target/release/motion-engine`, the sfx library at
//! `<repo>/assets/sfx/library`, `ffprobe`, and the on-device word-timing
//! models (`motion-engine models list`). The voice cache and the jobs folder
//! are temporary, so the first call always synthesises and the run touches
//! nothing under `output/`.
//!
//! What it checks (plan §14): `make_video` ends `done` with a video of at least
//! 3 s that has an audio stream, a 720p preview ≤ 30 MB and a reply of at most
//! 800 characters; the identical call returns the same job in under 2 s; and
//! `revise_video` changing only a title returns a NEW job that is `done` and
//! whose engine log shows the voice step was served from its cache.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rmcp::model::{CallToolRequestParams, CallToolResult};
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{json, Value};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {}

type Session = RunningService<RoleClient, Client>;

fn repo() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository root")
}

/// Why the test cannot run here, if it cannot.
fn skip_reason(repo: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return Some(
            "needs macOS (`motion-mcp --tts-model say` uses the offline `say` voice)".into(),
        );
    }
    let engine = repo.join("target/release/motion-engine");
    if !engine.is_file() {
        return Some(format!(
            "no engine at {}; build it: cargo build --release -p motion-cli --features asr-local,asr-ctc",
            engine.display()
        ));
    }
    let sfx = repo.join("assets/sfx/library");
    if !sfx.is_dir() {
        return Some(format!(
            "no sfx library at {} (it is gitignored: link it, or rebuild it with `motion-engine sfx-index`)",
            sfx.display()
        ));
    }
    let probe = std::process::Command::new("ffprobe")
        .arg("-version")
        .output();
    if probe.is_err() {
        return Some("ffprobe is not on PATH".into());
    }
    None
}

fn call_args(args: Value) -> serde_json::Map<String, Value> {
    args.as_object().expect("arguments object").clone()
}

async fn call(session: &Session, name: &str, args: Value) -> CallToolResult {
    tokio::time::timeout(
        Duration::from_secs(20 * 60),
        session.call_tool(
            CallToolRequestParams::new(name.to_string()).with_arguments(call_args(args)),
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("{name} took more than 20 minutes"))
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn text(r: &CallToolResult) -> String {
    r.content[0].as_text().expect("text first").text.clone()
}

fn structured(r: &CallToolResult) -> &Value {
    r.structured_content.as_ref().expect("structured content")
}

/// `ffprobe`: the container duration and the stream kinds of `file`.
fn probe(file: &Path) -> (f64, Vec<String>) {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration:stream=codec_type",
            "-of",
            "json",
        ])
        .arg(file)
        .output()
        .expect("ffprobe runs");
    assert!(
        out.status.success(),
        "ffprobe {}: {}",
        file.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: Value = serde_json::from_slice(&out.stdout).expect("ffprobe json");
    let duration = json["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse::<f64>().ok())
        .expect("duration");
    let kinds = json["streams"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter_map(|s| s["codec_type"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    (duration, kinds)
}

/// A 2-beat lite story.
fn story(first_title: &str) -> Value {
    json!({
        "story": {
            "title": "octopus hearts",
            "beats": [
                {
                    "say": "Octopuses have three hearts and blue blood running through their bodies.",
                    "show": first_title,
                    "picture": "fish"
                },
                {
                    "say": "Two of the hearts pump blood through the gills while the third feeds the body.",
                    "number": "3",
                    "meaning": "hearts"
                }
            ]
        },
        "style": "documentary"
    })
}

fn log_of(jobs: &Path, job: &str) -> String {
    std::fs::read_to_string(jobs.join(job).join("log.txt")).unwrap_or_default()
}

/// The engine's own voice line, e.g. `2 sentences, 8.5 s | provider calls: 0 | cache hits: 1`.
fn voice_stats(log: &str) -> Option<String> {
    log.lines()
        .find(|l| l.contains("provider calls:"))
        .map(|l| l.trim().to_string())
}

#[tokio::test]
#[ignore = "renders a real video with the release engine; run with --ignored"]
async fn two_beat_video_renders_repeats_instantly_and_revises_with_a_cached_voice() {
    let repo = repo();
    if let Some(why) = skip_reason(&repo) {
        println!("SKIPPED e2e_render: {why}");
        return;
    }
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e_render");
    let _ = std::fs::remove_dir_all(&root);
    let jobs = root.join("jobs");
    let voice_cache = root.join("voice");
    std::fs::create_dir_all(&jobs).expect("jobs dir");

    let total = Instant::now();
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_motion-mcp"));
    cmd.args(["--tts-model", "say", "--repo"])
        .arg(&repo)
        .arg("--jobs")
        .arg(&jobs)
        .env("MOTION_VOICE_CACHE", &voice_cache);
    let session: Session = Client
        .serve(TokioChildProcess::new(cmd).expect("spawn motion-mcp"))
        .await
        .expect("MCP handshake");

    // 1. make_video renders the whole thing.
    let started = Instant::now();
    let first = call(&session, "make_video", story("Three hearts")).await;
    let first_s = started.elapsed();
    let s = structured(&first).clone();
    assert_eq!(s["status"], "done", "{}", text(&first));
    assert_eq!(first.is_error, Some(false));
    let reply = text(&first);
    assert!(
        reply.chars().count() <= 800,
        "reply is {} chars",
        reply.chars().count()
    );
    let job = s["job"].as_str().expect("job id").to_string();
    let dir = jobs.join(&job);

    let video = dir.join("video.mp4");
    let (duration, kinds) = probe(&video);
    assert!(duration >= 3.0, "video is {duration:.2} s");
    assert!(kinds.iter().any(|k| k == "video"), "{kinds:?}");
    assert!(
        kinds.iter().any(|k| k == "audio"),
        "no audio stream: {kinds:?}"
    );
    let preview = dir.join("preview_720p.mp4");
    let preview_bytes = std::fs::metadata(&preview).expect("preview_720p.mp4").len();
    assert!(
        preview_bytes <= 30 * 1024 * 1024,
        "preview is {preview_bytes} bytes"
    );
    let (preview_s, preview_kinds) = probe(&preview);
    assert!(preview_s >= 3.0 && preview_kinds.iter().any(|k| k == "audio"));
    let shown = Path::new(s["video"].as_str().expect("video path"));
    let shown = if shown.is_absolute() {
        shown.to_path_buf()
    } else {
        repo.join(shown)
    };
    assert_eq!(
        std::fs::canonicalize(&shown).ok(),
        std::fs::canonicalize(&video).ok(),
        "the reply's video path is the job's video"
    );

    // The first call synthesised the voice: one provider call, no cache hit.
    let first_log = log_of(&jobs, &job);
    let first_voice = voice_stats(&first_log).expect("voice stats line in log.txt");
    assert!(
        first_voice.contains("provider calls: 1 | cache hits: 0"),
        "{first_voice}"
    );

    // 2. The identical call returns the same job at once.
    let started = Instant::now();
    let again = call(&session, "make_video", story("Three hearts")).await;
    let again_s = started.elapsed();
    assert_eq!(structured(&again)["job"], job.as_str());
    assert_eq!(structured(&again)["status"], "done");
    assert!(again_s < Duration::from_secs(2), "repeat took {again_s:?}");

    // 3. Changing only beat 1's title is a NEW job, rendered again, with the
    //    recorded voice (the cache is keyed by the narration, which is unchanged).
    let started = Instant::now();
    let revised = call(
        &session,
        "revise_video",
        json!({"job": job, "changes": [{"beat": 1, "show": "A heart for the body"}]}),
    )
    .await;
    let revise_s = started.elapsed();
    let r = structured(&revised).clone();
    assert_eq!(r["status"], "done", "{}", text(&revised));
    let new_job = r["job"].as_str().expect("new job id").to_string();
    assert_ne!(new_job, job, "a revision is a new job");
    assert!(jobs.join(&new_job).join("video.mp4").is_file());
    let (revised_duration, revised_kinds) = probe(&jobs.join(&new_job).join("video.mp4"));
    assert!(revised_duration >= 3.0 && revised_kinds.iter().any(|k| k == "audio"));
    let revised_voice =
        voice_stats(&log_of(&jobs, &new_job)).expect("voice stats line in the revision's log.txt");
    assert!(
        revised_voice.contains("provider calls: 0 | cache hits: 1"),
        "the voice step should hit its cache: {revised_voice}"
    );
    // The first job is untouched.
    assert!(video.is_file());

    session.cancel().await.expect("close the session");
    println!(
        "e2e_render: first make_video {first_s:.1?} ({duration:.1} s video, {} KB preview), \
         repeat {again_s:.2?}, revise_video {revise_s:.1?}, total {:.1?}\n  voice 1st: {first_voice}\n  voice revision: {revised_voice}",
        preview_bytes / 1024,
        total.elapsed()
    );
}
