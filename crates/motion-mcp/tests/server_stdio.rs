//! The real `motion-mcp` binary over stdio, driven by the rmcp client the way
//! an MCP client would, with a fake `motion-engine`
//! (`tests/fixtures/fake_engine.sh`: stage lines, stub files, a 1 s black mp4;
//! no voice, no real render).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientRequest,
    ProgressNotificationParam, Request, ServerResult,
};
use rmcp::service::{NotificationContext, PeerRequestOptions, RunningService};
use rmcp::transport::TokioChildProcess;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{json, Value};

use motion_mcp::profile::Profile;
use motion_mcp::schema::tool_defs;

/// Records progress notifications.
#[derive(Clone, Default)]
struct Client {
    progress: Arc<Mutex<Vec<ProgressNotificationParam>>>,
}

impl ClientHandler for Client {
    async fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.progress.lock().unwrap().push(params);
    }
}

type Session = RunningService<RoleClient, Client>;

/// A temporary repository (`assets/` only) and the fake engine's side files.
struct Env {
    repo: PathBuf,
    counter: PathBuf,
    pidfile: PathBuf,
}

impl Env {
    fn new(name: &str) -> Env {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("server_stdio-{name}"));
        let _ = std::fs::remove_dir_all(&root);
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("assets")).unwrap();
        Env {
            repo,
            counter: root.join("reel_runs.txt"),
            pidfile: root.join("reel.pid"),
        }
    }

    fn jobs(&self) -> PathBuf {
        self.repo.join("output/jobs")
    }

    /// Reel runs so far.
    fn runs(&self) -> Vec<String> {
        std::fs::read_to_string(&self.counter)
            .map(|t| t.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }

    /// The server command on this repository with the fake engine.
    fn command(&self, args: &[&str], vars: &[(&str, &str)]) -> tokio::process::Command {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_engine.sh");
        let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_motion-mcp"));
        cmd.arg("--engine")
            .arg(&fake)
            .arg("--repo")
            .arg(&self.repo)
            .args(args)
            .env("FAKE_ENGINE_COUNTER", &self.counter)
            .env("FAKE_ENGINE_PIDFILE", &self.pidfile);
        for (k, v) in vars {
            cmd.env(k, v);
        }
        cmd
    }

    async fn start(&self, args: &[&str], vars: &[(&str, &str)]) -> (Session, Client) {
        let client = Client::default();
        let session = client
            .clone()
            .serve(TokioChildProcess::new(self.command(args, vars)).unwrap())
            .await
            .unwrap();
        (session, client)
    }

    /// Wait until the only job is rendering; its id and the reel's pid.
    async fn wait_rendering(&self) -> (String, String) {
        let started = Instant::now();
        loop {
            let pid = std::fs::read_to_string(&self.pidfile).ok();
            let job = std::fs::read_dir(self.jobs())
                .ok()
                .and_then(|rd| rd.flatten().map(|e| e.path()).find(|p| p.is_dir()));
            let rendering = job
                .as_ref()
                .and_then(|j| std::fs::read_to_string(j.join("status.json")).ok())
                .is_some_and(|s| s.contains("\"render\""));
            if let (Some(pid), Some(job), true) = (pid, job, rendering) {
                let id = job.file_name().unwrap().to_string_lossy().into_owned();
                return (id, pid.trim().to_string());
            }
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "reel never started"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn make_video_request(title: &str) -> ClientRequest {
    ClientRequest::CallToolRequest(Request::new(params("make_video", story(title))))
}

fn result_of(r: ServerResult) -> CallToolResult {
    match r {
        ServerResult::CallToolResult(r) => r,
        other => panic!("not a tool result: {other:?}"),
    }
}

fn params(name: &str, args: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_string()).with_arguments(args.as_object().unwrap().clone())
}

async fn call(session: &Session, name: &str, args: Value) -> CallToolResult {
    session.call_tool(params(name, args)).await.unwrap()
}

fn text(r: &CallToolResult) -> String {
    r.content[0].as_text().expect("text first").text.clone()
}

fn structured(r: &CallToolResult) -> &Value {
    r.structured_content.as_ref().expect("structured content")
}

/// A 3-beat lite story.
fn story(title: &str) -> Value {
    json!({
        "story": {
            "title": title,
            "beats": [
                {"say": "Octopuses have three hearts and blue blood running through their bodies.", "show": "Three hearts"},
                {"say": "Two of the hearts pump blood through the gills while one feeds the body.", "number": "2", "meaning": "gill hearts"},
                {"say": "The third heart stops beating whenever the octopus swims, so it prefers to crawl.", "show": "Swimming is tiring"}
            ]
        },
        "style": "documentary"
    })
}

#[tokio::test]
async fn tools_match_the_profile_schemas() {
    let env = Env::new("tools");
    for (profile, args, allow) in [
        (Profile::Weak, vec![], false),
        (Profile::Creator, vec!["--profile", "creator"], false),
        (Profile::Operator, vec!["--profile", "operator"], false),
        (
            Profile::Operator,
            vec!["--profile", "operator", "--allow-scene-edits"],
            true,
        ),
    ] {
        let (session, _) = env.start(&args, &[]).await;
        let info = session.peer_info().expect("server info");
        assert_eq!(
            info.server_info.as_ref().map(|i| i.name.as_str()),
            Some("motionengine")
        );
        assert!(info.instructions.as_deref().is_some_and(
            |i| i.contains("make_video") && i.contains("motionengine://guide/lite-story")
        ));
        assert!(info.capabilities.tools.is_some());
        assert!(info.capabilities.resources.is_some());
        assert!(info.capabilities.prompts.is_some());
        let tools = session.list_all_tools().await.unwrap();
        let got: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": Value::Object(t.input_schema.as_ref().clone()),
                })
            })
            .collect();
        let want: Vec<Value> = tool_defs(profile, allow)
            .iter()
            .map(|d| d.wire_json())
            .collect();
        assert_eq!(got, want, "{} tools", profile.name());
        assert!(tools.iter().all(|t| t.output_schema.is_none()));
        // Resources and prompts are wired (content arrives with 1d).
        session.list_all_resources().await.unwrap();
        session.list_all_prompts().await.unwrap();
        session.cancel().await.unwrap();
    }
}

#[tokio::test]
async fn make_video_renders_once_then_serves_the_cache() {
    let env = Env::new("make");
    let (session, client) = env.start(&[], &[]).await;
    let r = call(&session, "make_video", story("octopus hearts")).await;
    let s = structured(&r).clone();
    assert_eq!(s["status"], "done", "{}", text(&r));
    assert_eq!(s["qa"], "pass");
    assert_eq!(r.is_error, Some(false));
    let job = s["job"].as_str().unwrap().to_string();
    assert_eq!(s["video"], format!("output/jobs/{job}/video.mp4"));
    assert_eq!(s["preview"], format!("output/jobs/{job}/preview_720p.mp4"));
    assert!(s["duration_s"].as_f64().is_some_and(|d| d > 0.5 && d < 2.0));
    let dir = env.jobs().join(&job);
    for f in [
        "video.mp4",
        "preview_720p.mp4",
        "status.json",
        "request.json",
        "intent.json",
        "style.json",
        "log.txt",
    ] {
        assert!(dir.join(f).is_file(), "{f} missing");
    }
    let t = text(&r);
    assert!(t.chars().count() <= 800, "{} chars", t.chars().count());
    assert!(t.starts_with(&format!("done · job {job}")), "{t}");
    assert!(t.contains("next: Done. Call revise_video with changes if needed."));
    // Video and preview also come back as resource links (absolute file URIs).
    let links: Vec<String> = r
        .content
        .iter()
        .filter_map(|c| c.as_resource_link())
        .map(|l| l.uri.clone())
        .collect();
    assert_eq!(links.len(), 2, "{links:?}");
    assert!(links[0].starts_with("file:///") && links[0].ends_with(&format!("{job}/video.mp4")));
    // The engine got the reel defaults and the job's files.
    let runs = env.runs();
    assert_eq!(runs.len(), 1);
    for flag in [
        "--style",
        "--tts-model auto",
        "--music auto",
        "--art auto",
        "--assets",
        &format!("{job}/intent.json"),
    ] {
        assert!(runs[0].contains(flag), "{flag} not in {}", runs[0]);
    }
    // Stage progress arrived for the request's progress token.
    let progress = client.progress.lock().unwrap().clone();
    assert!(!progress.is_empty(), "no progress notifications");
    assert!(progress.iter().all(|p| p.total == Some(100.0)));
    let stages: Vec<String> = progress.iter().filter_map(|p| p.message.clone()).collect();
    assert!(stages.iter().any(|m| m == "render"), "{stages:?}");
    assert_eq!(stages.last().map(String::as_str), Some("done"));

    // The same call again: the same job at once, no second render.
    let started = Instant::now();
    let again = call(&session, "make_video", story("octopus hearts")).await;
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(structured(&again)["job"], job.as_str());
    assert_eq!(structured(&again)["status"], "done");
    assert_eq!(env.runs().len(), 1);

    let got = call(&session, "get_video", json!({"job": job, "wait_s": 5})).await;
    assert_eq!(structured(&got)["status"], "done");
    assert_eq!(structured(&got)["video"], s["video"]);
    session.cancel().await.unwrap();
}

#[tokio::test]
async fn check_mode_renders_nothing() {
    let env = Env::new("check");
    let (session, _) = env.start(&[], &[]).await;
    let mut args = story("octopus check");
    args["mode"] = json!("check");
    let r = call(&session, "make_video", args).await;
    let s = structured(&r);
    assert_eq!(s["status"], "checked", "{}", text(&r));
    assert!(s.get("job").is_none());
    assert!(s["plan"].as_array().is_some_and(|p| !p.is_empty()));
    assert!(text(&r).contains("call make_video again with mode auto"));
    assert!(env.runs().is_empty());
    let entries: Vec<_> = std::fs::read_dir(env.jobs())
        .map(|rd| rd.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(entries.is_empty(), "{entries:?}");
    session.cancel().await.unwrap();
}

#[tokio::test]
async fn unknown_jobs_and_closed_tools_are_errors() {
    let env = Env::new("errors");
    let (weak, _) = env.start(&[], &[]).await;
    let r = call(&weak, "get_video", json!({"job": "j_0123456789"})).await;
    assert_eq!(structured(&r)["status"], "needs_fix");
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("unknown job"));
    let r = call(&weak, "get_video", json!({"job": "../etc"})).await;
    assert_eq!(structured(&r)["status"], "needs_fix");
    let r = call(&weak, "view_frames", json!({"job": "j_0123456789"})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("make_video"), "{}", text(&r));
    weak.cancel().await.unwrap();

    let (operator, _) = env.start(&["--profile", "operator"], &[]).await;
    let r = call(
        &operator,
        "apply_scene_patch",
        json!({"job": "j_0123456789", "patch": {}}),
    )
    .await;
    assert_eq!(r.is_error, Some(true));
    assert!(
        text(&r).contains("start motion-mcp with --allow-scene-edits"),
        "{}",
        text(&r)
    );
    operator.cancel().await.unwrap();
    assert!(env.runs().is_empty());
}

#[tokio::test]
async fn a_failing_engine_gives_one_scrubbed_line() {
    let env = Env::new("fail");
    let (session, _) = env.start(&[], &[("FAKE_ENGINE_FAIL", "1")]).await;
    let r = call(&session, "make_video", story("octopus fails")).await;
    let s = structured(&r).clone();
    assert_eq!(s["status"], "failed", "{}", text(&r));
    assert_eq!(r.is_error, Some(true));
    let error = s["error"].as_str().unwrap();
    assert_eq!(
        error,
        "voice: provider returned 401 Unauthorized for key [redacted]"
    );
    assert!(!text(&r).contains("sk-or"));
    let job = s["job"].as_str().unwrap();
    let log = std::fs::read_to_string(env.jobs().join(job).join("log.txt")).unwrap();
    assert!(log.contains("401 Unauthorized") && !log.contains("sk-or"));
    // A failed job runs again when asked again.
    let again = call(&session, "make_video", story("octopus fails")).await;
    assert_eq!(structured(&again)["job"], job);
    assert_eq!(env.runs().len(), 2);
    session.cancel().await.unwrap();
}

#[tokio::test]
async fn cancelling_the_submitting_call_stops_the_render() {
    let env = Env::new("cancel");
    let (session, _) = env.start(&[], &[("FAKE_ENGINE_SLEEP", "60")]).await;
    let started = Instant::now();
    let handle = session
        .send_cancellable_request(
            make_video_request("octopus cancel"),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    let (job, pid) = env.wait_rendering().await;
    handle.cancel(Some("test".into())).await.unwrap();
    // Let the cancellation land before another call waits on the job (a job
    // with another waiter is not cancelled).
    let status = env.jobs().join(&job).join("status.json");
    while !std::fs::read_to_string(&status).is_ok_and(|s| s.contains("\"failed\"")) {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "never cancelled"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let r = call(&session, "get_video", json!({"job": job, "wait_s": 15})).await;
    let s = structured(&r);
    assert_eq!(s["status"], "failed", "{}", text(&r));
    assert_eq!(s["error"], "cancelled");
    // The whole process group is gone (bash and its sleep).
    let alive = std::process::Command::new("kill")
        .args(["-0", &pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(!alive, "reel {pid} still running");
    assert!(started.elapsed() < Duration::from_secs(30));
    session.cancel().await.unwrap();
}

#[tokio::test]
async fn a_cancelled_get_video_only_stops_waiting() {
    let env = Env::new("cancel-get");
    let (session, _) = env.start(&[], &[("FAKE_ENGINE_SLEEP", "2")]).await;
    let make = session
        .send_cancellable_request(
            make_video_request("octopus get"),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    let (job, _) = env.wait_rendering().await;
    let get = session
        .send_cancellable_request(
            ClientRequest::CallToolRequest(Request::new(params(
                "get_video",
                json!({"job": job, "wait_s": 30}),
            ))),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    get.cancel(Some("test".into())).await.unwrap();
    // The render goes on and the submitting call gets the video.
    let r = result_of(make.await_response().await.unwrap());
    assert_eq!(structured(&r)["status"], "done", "{}", text(&r));
    assert_eq!(structured(&r)["job"], job.as_str());
    assert_eq!(env.runs().len(), 1);
    session.cancel().await.unwrap();
}

#[tokio::test]
async fn a_cancelled_submitter_leaves_a_job_others_wait_for() {
    let env = Env::new("cancel-shared");
    let (session, _) = env.start(&[], &[("FAKE_ENGINE_SLEEP", "2")]).await;
    let make = session
        .send_cancellable_request(
            make_video_request("octopus shared"),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    let (job, _) = env.wait_rendering().await;
    let peer = session.peer().clone();
    let waiting_job = job.clone();
    let other = tokio::spawn(async move {
        peer.call_tool_once(params(
            "get_video",
            json!({"job": waiting_job, "wait_s": 30}),
        ))
        .await
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    make.cancel(Some("test".into())).await.unwrap();
    let r = match other.await.unwrap().unwrap() {
        CallToolResponse::Complete(r) => r,
        other => panic!("unexpected response {other:?}"),
    };
    assert_eq!(structured(&r)["status"], "done", "{}", text(&r));
    assert_eq!(structured(&r)["job"], job.as_str());
    assert_eq!(env.runs().len(), 1);
    session.cancel().await.unwrap();
}

#[tokio::test]
async fn two_servers_share_a_jobs_folder_and_render_once() {
    let env = Env::new("two-servers");
    let slow = [("FAKE_ENGINE_SLEEP", "2")];
    let (a, _) = env.start(&[], &slow).await;
    let make_a = a
        .send_cancellable_request(
            make_video_request("octopus shared folder"),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();
    let (job, _) = env.wait_rendering().await;
    let status = env.jobs().join(&job).join("status.json");
    assert!(std::fs::read_to_string(&status)
        .unwrap()
        .contains("\"owner_pid\""));
    // A second server (a creator one, say) starts on the same folder while
    // the first renders: its recovery leaves the live server's job alone.
    let (b, _) = env.start(&["--profile", "creator"], &slow).await;
    assert!(!std::fs::read_to_string(&status)
        .unwrap()
        .contains("\"failed\""));
    // The same video asked of the second server: it waits, no second render.
    let rb = call(&b, "make_video", story("octopus shared folder")).await;
    assert_eq!(structured(&rb)["status"], "done", "{}", text(&rb));
    assert_eq!(structured(&rb)["job"], job.as_str());
    let ra = result_of(make_a.await_response().await.unwrap());
    assert_eq!(structured(&ra)["status"], "done", "{}", text(&ra));
    assert_eq!(env.runs().len(), 1);
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
}

#[tokio::test]
async fn renders_are_one_at_a_time_across_servers() {
    let env = Env::new("render-lock");
    let trace = env.repo.join("../trace.txt");
    let trace_var = trace.to_string_lossy().into_owned();
    let vars = [
        ("FAKE_ENGINE_SLEEP", "1"),
        ("FAKE_ENGINE_TRACE", trace_var.as_str()),
    ];
    let (a, _) = env.start(&[], &vars).await;
    let (b, _) = env.start(&[], &vars).await;
    let (ra, rb) = tokio::join!(
        call(&a, "make_video", story("octopus first")),
        call(&b, "make_video", story("octopus second")),
    );
    assert_eq!(structured(&ra)["status"], "done", "{}", text(&ra));
    assert_eq!(structured(&rb)["status"], "done", "{}", text(&rb));
    assert_eq!(env.runs().len(), 2);
    // The two reels never overlapped.
    let lines: Vec<String> = std::fs::read_to_string(&trace)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 4, "{lines:?}");
    for pair in lines.chunks(2) {
        let title = pair[0].strip_prefix("start ").expect("a start line");
        assert_eq!(pair[1], format!("end {title}"), "{lines:?}");
    }
    a.cancel().await.unwrap();
    b.cancel().await.unwrap();
}
