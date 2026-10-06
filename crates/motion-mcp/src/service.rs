//! (1b) Transport-independent tool handlers: argument parsing, the check →
//! prepare → job → engine flow, waiting and replies. The MCP server
//! ([`crate::server`]) only converts to and from `rmcp` types.
//!
//! Everything here is synchronous (the server calls it from a blocking task)
//! and `Send + Sync`; tests drive it without any transport.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::args::{self, FindAssetsArgs, GetVideoArgs, MakeVideoArgs, Mode, ReviseVideoArgs};
use crate::byo;
use crate::creator;
use crate::engine::{self, Engine, EngineJob};
use crate::job::{self, JobId, JobState, JobStatus, StoredRequest};
use crate::pictures::PictureIndex;
use crate::policy::{self, Context, Prepared};
use crate::profile::{self, Profile, ServerConfig};
use crate::reply::{cut, Fix, Progress, Reply, Status, ToolOutput};
use crate::resources::ResourceCtx;
use crate::store::{self, Store, Submitted};

pub const NEXT_DONE: &str = "Done. Call revise_video with changes if needed.";
pub const NEXT_RENDERING: &str = "Rendering. Call get_video with this job and wait_s 120.";
pub const NEXT_CHECKED_MAKE: &str =
    "If the plan looks right, call make_video again with mode auto.";
pub const NEXT_CHECKED_REVISE: &str =
    "If the plan looks right, call revise_video again with mode auto.";
/// `find_assets` takes at most this many words.
pub const MAX_FIND_WORDS: usize = 30;

/// Per-call context: progress reporting and the caller's cancellation.
#[derive(Default)]
pub struct CallCtx {
    progress: Option<Box<dyn Fn(Progress) + Send + Sync>>,
    cancelled: Arc<AtomicBool>,
}

impl CallCtx {
    pub fn new() -> Self {
        CallCtx::default()
    }

    /// Report progress (stage changes of the job the call waits on).
    pub fn with_progress(mut self, f: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        self.progress = Some(Box::new(f));
        self
    }

    /// Set to true when the caller cancels (then call [`Service::wake`]). The
    /// call stops waiting; the job is cancelled only when this call queued it
    /// and no other call waits on it. `get_video` never cancels a job.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// The tools of one server.
pub struct Service {
    config: ServerConfig,
    pictures: PictureIndex,
    store: Arc<Store>,
}

impl Service {
    /// Load the picture index, open the job store and start its worker.
    pub fn new(config: ServerConfig, engine: Arc<dyn Engine>) -> std::io::Result<Service> {
        let pictures = PictureIndex::load(&config.assets);
        let store = Store::open(config.clone(), engine)?;
        Ok(Service {
            config,
            pictures,
            store,
        })
    }

    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    pub fn profile(&self) -> Profile {
        self.config.profile
    }

    pub fn pictures(&self) -> &PictureIndex {
        &self.pictures
    }

    pub fn resource_ctx(&self) -> ResourceCtx<'_> {
        ResourceCtx {
            config: &self.config,
            pictures: &self.pictures,
        }
    }

    /// Wake waiting calls (after a caller's cancel flag was set).
    pub fn wake(&self) {
        self.store.wake();
    }

    /// Stop the queue and the running render (server shutdown).
    pub fn shutdown(&self) {
        self.store.shutdown();
    }

    /// Call tool `name` with JSON `args`.
    pub fn call(&self, name: &str, args: Value, ctx: &CallCtx) -> ToolOutput {
        let p = self.config.profile;
        let allow = self.config.allow_scene_edits;
        if name == profile::APPLY_SCENE_PATCH && p == Profile::Operator && !allow {
            return refused(
                "apply_scene_patch is off: start motion-mcp with --allow-scene-edits to allow scene edits.",
            );
        }
        let names = profile::tool_names(p, allow);
        if !names.contains(&name) {
            return unknown_tool(name, &names);
        }
        let out = match name {
            profile::MAKE_VIDEO => self.make_video(args, ctx),
            profile::REVISE_VIDEO => self.revise_video(args, ctx),
            profile::GET_VIDEO => self.get_video(args, ctx),
            profile::FIND_ASSETS => self.find_assets(args),
            profile::VIEW_FRAMES
            | profile::EXPLORE_STYLES
            | profile::LIST_OPTIONS
            | profile::PLAN_ASSETS => creator::call(&self.config, &self.pictures, name, args),
            _ => creator::not_available(name),
        };
        within_budget(out, profile::reply_budget_chars(p))
    }

    fn ctx(&self) -> Context<'_> {
        Context {
            config: &self.config,
            pictures: &self.pictures,
        }
    }

    /// (0.22) The `checked` reply plus the story review: the compiler's story
    /// warnings for this exact story (no voice, no render), first among the
    /// findings, so the model can fix the story before rendering.
    fn reviewed(&self, prepared: &Prepared, next: &str) -> Reply {
        let mut r = checked(prepared, next);
        let id = job::job_id(&prepared.key);
        let intent = serde_json::to_value(&prepared.intent).unwrap_or(Value::Null);
        let style = serde_json::to_value(&prepared.style).unwrap_or(Value::Null);
        let mut review = engine::story_review(
            &self.config,
            &id,
            &prepared.request,
            &intent,
            &style,
            prepared.images.as_ref(),
        );
        review.append(&mut r.findings);
        r.findings = review;
        r
    }

    fn make_video(&self, args: Value, ctx: &CallCtx) -> ToolOutput {
        // Lenient: arguments a small model put in the wrong place are moved
        // (and noted in `changed`) before the story is checked.
        let (args, prepared): (MakeVideoArgs, _) = policy::prepare_raw(&args, &self.ctx());
        let prepared = match prepared {
            Ok(p) => p,
            Err(fixes) => return self.fit(Reply::needs_fix(fixes)),
        };
        if args.mode == Mode::Check {
            return self.fit(self.reviewed(&prepared, NEXT_CHECKED_MAKE));
        }
        let force = args.force && self.config.profile == Profile::Operator;
        self.fit(self.run_prepared(&prepared, None, force, ctx))
    }

    fn revise_video(&self, args: Value, ctx: &CallCtx) -> ToolOutput {
        let args: ReviseVideoArgs = match parse_args(args) {
            Ok(a) => a,
            Err(fix) => return self.fit(Reply::needs_fix(vec![fix])),
        };
        let parent = match JobId::parse(&args.job) {
            Ok(id) => id,
            Err(fix) => return self.fit(Reply::needs_fix(vec![fix])),
        };
        let stored = match self.stored_request(&parent) {
            Some(s) => s,
            None => return self.fit(Reply::needs_fix(vec![unknown_job(&parent)])),
        };
        let prepared = match policy::revise(&stored, &args, &self.ctx()) {
            Ok(p) => p,
            Err(fixes) => return self.fit(Reply::needs_fix(fixes)),
        };
        if args.mode == Mode::Check {
            return self.fit(self.reviewed(&prepared, NEXT_CHECKED_REVISE));
        }
        self.fit(self.run_prepared(&prepared, Some(parent), false, ctx))
    }

    fn get_video(&self, args: Value, ctx: &CallCtx) -> ToolOutput {
        let args: GetVideoArgs = match parse_args(args) {
            Ok(a) => a,
            Err(fix) => return self.fit(Reply::needs_fix(vec![fix])),
        };
        let id = match JobId::parse(&args.job) {
            Ok(id) => id,
            Err(fix) => return self.fit(Reply::needs_fix(vec![fix])),
        };
        let wait = Duration::from_secs(args.wait_s.min(args::MAX_WAIT_S));
        let take = self.stored_request(&id).map_or(0, |s| s.take);
        // get_video never cancels a job: a cancelled call only stops waiting.
        self.fit(self.wait_reply(&id, take, wait, false, ctx))
    }

    fn find_assets(&self, args: Value) -> ToolOutput {
        let args: FindAssetsArgs = match parse_args(args) {
            Ok(a) => a,
            Err(fix) => return self.fit(Reply::needs_fix(vec![fix])),
        };
        let mut words: Vec<String> = Vec::new();
        for w in args
            .words
            .iter()
            .map(|w| w.trim())
            .filter(|w| !w.is_empty())
        {
            if !words.iter().any(|x| x == w) {
                words.push(w.to_string());
            }
        }
        if words.is_empty() {
            return self.fit(Reply::needs_fix(vec![Fix::new(
                None,
                "words",
                "give 1 to 30 words to look up",
            )]));
        }
        let dropped = words.len().saturating_sub(MAX_FIND_WORDS);
        words.truncate(MAX_FIND_WORDS);
        let k = args.k.unwrap_or(3).clamp(1, 5);
        let found = self.pictures.find(&words, k);
        let mut lines: Vec<String> = words
            .iter()
            .map(|w| match found.get(w).filter(|v| !v.is_empty()) {
                Some(names) => format!("{w}: {}", names.join(", ")),
                None => format!("{w}: (text)"),
            })
            .collect();
        if dropped > 0 {
            lines.push(format!("({dropped} more words ignored: at most 30)"));
        }
        ToolOutput {
            structured: serde_json::to_value(&found).unwrap_or_else(|_| json!({})),
            text: lines.join("; "),
            images: Vec::new(),
            links: Vec::new(),
            is_error: false,
        }
    }

    /// `request.json` of a job (None when the job or the file is unknown).
    fn stored_request(&self, id: &JobId) -> Option<StoredRequest> {
        let text =
            std::fs::read_to_string(id.dir(&self.config.jobs).join(job::REQUEST_JSON)).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Queue (or reuse) the job for `prepared` and wait for it.
    fn run_prepared(
        &self,
        prepared: &Prepared,
        parent: Option<JobId>,
        force: bool,
        ctx: &CallCtx,
    ) -> Reply {
        let id = job::job_id(&prepared.key);
        let mut findings = prepared.findings.clone();
        findings.extend(engine::variety_note(&prepared.options));
        let initial = JobStatus {
            job: id.clone(),
            state: JobState::Queued,
            stage: None,
            percent: None,
            video: None,
            preview: None,
            duration_s: None,
            qa: None,
            changed: prepared.changed.clone(),
            findings,
            error: None,
            parent: parent.filter(|p| p != &id),
            created_unix: store::now_unix(),
            finished_unix: None,
            // Set by the store when it queues the job.
            owner_pid: None,
        };
        let engine_job = EngineJob {
            id: id.clone(),
            dir: id.dir(&self.config.jobs),
            request: prepared.request.clone(),
            title: prepared.intent.title.clone(),
            has_manifest: prepared.images.is_some(),
        };
        let write = |dir: &Path| write_job_files(prepared, dir);
        match self.store.submit(engine_job, initial, force, &write) {
            Err(e) => Reply::failed(Some(id.to_string()), e),
            Ok(Submitted::Done(status)) => {
                reply_of(&status, None).with_take_hint(prepared.request.take)
            }
            // Only the call that queued the job may cancel it (and only while
            // no other call waits on it).
            Ok(submitted) => self.wait_reply(
                &id,
                prepared.request.take,
                Duration::from_secs(self.config.max_wait_s),
                submitted == Submitted::Queued,
                ctx,
            ),
        }
    }

    /// Wait for job `id` (reporting progress) and reply with its status.
    /// `may_cancel`: this call submitted the job, so the caller's cancellation
    /// cancels it (see [`Store::wait`]). `take` is the job's take (a finished
    /// video's `next` mentions the following one).
    fn wait_reply(
        &self,
        id: &JobId,
        take: u64,
        wait: Duration,
        may_cancel: bool,
        ctx: &CallCtx,
    ) -> Reply {
        let mut last: Option<Progress> = None;
        let mut on_change = |s: &JobStatus, _pos: Option<usize>| {
            let Some(report) = &ctx.progress else {
                return;
            };
            let Some(p) = progress_of(s) else {
                return;
            };
            let newer = match &last {
                None => true,
                Some(l) => l.stage != p.stage && p.percent >= l.percent,
            };
            if newer {
                report(p.clone());
                last = Some(p);
            }
        };
        match self
            .store
            .wait(id, wait, may_cancel, &|| ctx.is_cancelled(), &mut on_change)
        {
            None => Reply::needs_fix(vec![unknown_job(id)]),
            Some((status, pos)) => reply_of(&status, pos).with_take_hint(take),
        }
    }

    /// The reply within the profile's budget.
    fn fit(&self, reply: Reply) -> ToolOutput {
        fit_reply(reply, profile::reply_budget_chars(self.config.profile))
    }
}

/// Write the job's input files: `request.json`, `intent.json`, `style.json`
/// and, with user images, the manifest.
fn write_job_files(prepared: &Prepared, dir: &Path) -> Result<(), String> {
    fn put<T: serde::Serialize>(dir: &Path, name: &str, value: &T) -> Result<(), String> {
        let text =
            serde_json::to_string_pretty(value).map_err(|e| format!("cannot write {name}: {e}"))?;
        std::fs::write(dir.join(name), text).map_err(|e| format!("cannot write {name}: {e}"))
    }
    put(dir, job::REQUEST_JSON, &prepared.request)?;
    put(dir, job::INTENT_JSON, &prepared.intent)?;
    put(dir, job::STYLE_JSON, &prepared.style)?;
    if let Some(images) = &prepared.images {
        byo::write_manifest(images, dir).map_err(|e| format!("cannot write the images: {e}"))?;
    }
    Ok(())
}

/// Lenient typed arguments (`null` = no arguments); a parse error is a fix.
fn parse_args<T: DeserializeOwned + Default>(args: Value) -> Result<T, Fix> {
    if args.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(args).map_err(|e| {
        Fix::new(None, "arguments", cut(&e.to_string()))
            .otherwise("send the fields as the tool's input schema shows")
    })
}

fn unknown_job(id: &JobId) -> Fix {
    Fix::new(None, "job", format!("unknown job '{id}'"))
        .otherwise("use the job value from an earlier reply, or call make_video")
}

/// Progress as reported to the caller.
fn progress_of(s: &JobStatus) -> Option<Progress> {
    match s.state {
        JobState::Queued => Some(Progress {
            stage: "queued".into(),
            percent: 0,
        }),
        JobState::Running => Some(Progress {
            stage: s.stage.map(|st| st.name()).unwrap_or("check").to_string(),
            percent: s.percent.unwrap_or(0),
        }),
        JobState::Done => Some(Progress {
            stage: "done".into(),
            percent: 100,
        }),
        JobState::Failed => None,
    }
}

/// The reply for a job status.
pub fn reply_of(s: &JobStatus, queue_position: Option<usize>) -> Reply {
    let job = Some(s.job.to_string());
    let mut r = match s.state {
        JobState::Done => {
            let mut r = Reply::new(Status::Done, NEXT_DONE);
            r.video = s.video.clone();
            r.preview = s.preview.clone();
            r.duration_s = s.duration_s;
            r.qa = s.qa;
            r.findings = s.findings.clone();
            r
        }
        JobState::Failed => Reply::failed(
            None,
            s.error.clone().unwrap_or_else(|| "unknown error".into()),
        ),
        JobState::Running => {
            let mut r = Reply::new(Status::Running, NEXT_RENDERING);
            r.progress = progress_of(s);
            r
        }
        JobState::Queued => {
            let mut r = Reply::new(Status::Queued, NEXT_RENDERING);
            r.queue_position = queue_position;
            r
        }
    };
    r.job = job;
    r.changed = s.changed.clone();
    r
}

/// The `checked` reply: the plan, what was changed, the warnings.
fn checked(prepared: &Prepared, next: &str) -> Reply {
    let mut r = Reply::new(Status::Checked, next);
    r.plan = prepared.plan.clone();
    if let Some(images) = &prepared.images {
        r.plan.extend(images.lines.iter().cloned());
    }
    r.changed = prepared.changed.clone();
    r.findings = prepared.findings.clone();
    r.findings.extend(engine::variety_note(&prepared.options));
    r
}

/// The reply as tool output, dropping list lines (findings, then changed,
/// then plan, then fixes) until the text fits `budget` characters.
pub fn fit_reply(reply: Reply, budget: usize) -> ToolOutput {
    let mut r = reply.capped();
    loop {
        let out = ToolOutput::from(r.clone());
        if out.text.chars().count() <= budget {
            return out;
        }
        if r.findings.pop().is_some()
            || r.changed.pop().is_some()
            || r.plan.pop().is_some()
            || r.fixes.pop().is_some()
        {
            continue;
        }
        return within_budget(out, budget);
    }
}

/// Last resort: cut the text to the budget.
fn within_budget(mut out: ToolOutput, budget: usize) -> ToolOutput {
    if out.text.chars().count() > budget {
        let mut t: String = out.text.chars().take(budget.saturating_sub(1)).collect();
        t.push('…');
        out.text = t;
    }
    out
}

fn refused(text: &str) -> ToolOutput {
    ToolOutput {
        structured: json!({ "status": "refused", "error": text }),
        text: text.to_string(),
        images: Vec::new(),
        links: Vec::new(),
        is_error: true,
    }
}

fn unknown_tool(name: &str, names: &[&str]) -> ToolOutput {
    let text = format!(
        "unknown tool '{}'; this server has: {}",
        cut(name),
        names.join(", ")
    );
    ToolOutput {
        structured: json!({ "status": "unknown_tool", "error": text, "tools": names }),
        text,
        images: Vec::new(),
        links: Vec::new(),
        is_error: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Cancel, EngineOutput};
    use crate::job::Stage;
    use crate::reply::Qa;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;

    struct Quick {
        runs: AtomicUsize,
    }

    impl Engine for Quick {
        fn run(
            &self,
            _config: &ServerConfig,
            job: &EngineJob,
            stage: &dyn Fn(Stage),
            _cancel: &Arc<Cancel>,
        ) -> Result<EngineOutput, String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            for s in [Stage::Voice, Stage::Music, Stage::Compile, Stage::Render] {
                stage(s);
                std::thread::sleep(Duration::from_millis(30));
            }
            if job.title.contains("broken") {
                return Err("render: the scene is broken".into());
            }
            let video = job.dir.join(job::VIDEO_MP4);
            std::fs::write(&video, b"mp4").map_err(|e| e.to_string())?;
            Ok(EngineOutput {
                video,
                preview: None,
                duration_s: Some(12.5),
                qa: Some(Qa::Pass),
                findings: Vec::new(),
            })
        }
    }

    fn service(name: &str, profile: Profile) -> (Service, Arc<Quick>, std::path::PathBuf) {
        let repo =
            std::env::temp_dir().join(format!("motion-mcp-service-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&repo);
        std::fs::create_dir_all(repo.join("assets")).unwrap();
        let mut config = ServerConfig::new(profile, &repo);
        config.max_wait_s = 10;
        let engine = Arc::new(Quick {
            runs: AtomicUsize::new(0),
        });
        (Service::new(config, engine.clone()).unwrap(), engine, repo)
    }

    fn story(title: &str) -> Value {
        json!({
            "story": {
                "title": title,
                "beats": [
                    {"say": "Octopuses have three hearts and blue blood in their bodies.", "show": "Three hearts"},
                    {"say": "Two hearts pump blood through the gills while one feeds the body.", "number": "2"},
                    {"say": "The third heart stops beating whenever the octopus swims.", "show": "Swimming is tiring"}
                ]
            }
        })
    }

    #[test]
    fn make_video_flow_and_replies() {
        let (svc, engine, repo) = service("flow", Profile::Weak);
        let reports = Arc::new(Mutex::new(Vec::<Progress>::new()));
        let sink = Arc::clone(&reports);
        let ctx = CallCtx::new().with_progress(move |p| sink.lock().unwrap().push(p));
        let out = svc.call(profile::MAKE_VIDEO, story("octopus hearts"), &ctx);
        assert!(!out.is_error, "{}", out.text);
        assert_eq!(out.structured["status"], "done");
        assert!(out.text.contains(NEXT_DONE));
        assert!(out.text.chars().count() <= 800);
        let stages: Vec<String> = reports
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.stage.clone())
            .collect();
        assert!(stages.contains(&"render".to_string()), "{stages:?}");
        assert_eq!(stages.last().map(String::as_str), Some("done"));
        let job = out.structured["job"].as_str().unwrap().to_string();
        let dir = repo.join("output/jobs").join(&job);
        for f in [
            "request.json",
            "intent.json",
            "style.json",
            "status.json",
            "video.mp4",
        ] {
            assert!(dir.join(f).is_file(), "{f}");
        }
        // What the engine reads back parses as the engine's own types.
        let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
        motion_core::CreativeIntent::from_json(&read("intent.json")).unwrap();
        motion_core::StyleProfile::from_json(&read("style.json")).unwrap();
        let stored: StoredRequest = serde_json::from_str(&read("request.json")).unwrap();
        assert_eq!(stored.profile, Profile::Weak);
        assert_eq!(out.links, [format!("output/jobs/{job}/video.mp4")]);
        // Same input: same job, no second run.
        let again = svc.call(
            profile::MAKE_VIDEO,
            story("octopus hearts"),
            &CallCtx::new(),
        );
        assert_eq!(again.structured["job"], job.as_str());
        assert_eq!(engine.runs.load(Ordering::SeqCst), 1);
        // get_video reads the same.
        let got = svc.call(
            profile::GET_VIDEO,
            json!({"job": job, "wait_s": 0}),
            &CallCtx::new(),
        );
        assert_eq!(got.structured["status"], "done");
        // revise_video (stand-in policy: style change) → a new job with a parent.
        let rev = svc.call(
            profile::REVISE_VIDEO,
            json!({"job": job, "style": "cinematic"}),
            &CallCtx::new(),
        );
        assert_eq!(rev.structured["status"], "done", "{}", rev.text);
        let rev_job = rev.structured["job"].as_str().unwrap();
        assert_ne!(rev_job, job);
        let rev_status = store::read_status(&repo.join("output/jobs").join(rev_job)).unwrap();
        assert_eq!(rev_status.parent.map(|p| p.to_string()), Some(job.clone()));
        // Check mode: nothing written.
        let mut check = story("checked only");
        check["mode"] = json!("check");
        let c = svc.call(profile::MAKE_VIDEO, check, &CallCtx::new());
        assert_eq!(c.structured["status"], "checked");
        assert!(c.structured.get("job").is_none());
        assert!(c.text.contains(NEXT_CHECKED_MAKE));
        assert_eq!(engine.runs.load(Ordering::SeqCst), 2);
        svc.shutdown();
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn errors_are_fixes_or_failures() {
        let (svc, _engine, repo) = service("errors", Profile::Weak);
        let ctx = CallCtx::new();
        let bad = svc.call(profile::GET_VIDEO, json!({"job": "nope"}), &ctx);
        assert!(bad.is_error);
        assert_eq!(bad.structured["status"], "needs_fix");
        let unknown = svc.call(profile::GET_VIDEO, json!({"job": "j_0123456789"}), &ctx);
        assert_eq!(unknown.structured["status"], "needs_fix");
        assert!(unknown.text.contains("unknown job"));
        let rev = svc.call(profile::REVISE_VIDEO, json!({"job": "j_0123456789"}), &ctx);
        assert!(rev.text.contains("unknown job"));
        let typed = svc.call(profile::FIND_ASSETS, json!({"words": "berry"}), &ctx);
        assert_eq!(typed.structured["status"], "needs_fix");
        let weak_creator_tool = svc.call(profile::VIEW_FRAMES, json!({}), &ctx);
        assert!(weak_creator_tool.is_error);
        assert!(weak_creator_tool
            .text
            .contains("make_video, revise_video, get_video, find_assets"));
        let broken = svc.call(profile::MAKE_VIDEO, story("broken story"), &ctx);
        assert_eq!(broken.structured["status"], "failed");
        assert_eq!(broken.structured["error"], "render: the scene is broken");
        let found = svc.call(
            profile::FIND_ASSETS,
            json!({"words": ["berry", " oxygen ", "berry", ""], "k": 9}),
            &ctx,
        );
        assert!(!found.is_error);
        assert_eq!(found.text, "berry: (text); oxygen: (text)");
        assert_eq!(found.structured, json!({"berry": [], "oxygen": []}));
        svc.shutdown();
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn operator_scene_edits_are_gated() {
        let (svc, _engine, repo) = service("operator", Profile::Operator);
        let out = svc.call(profile::APPLY_SCENE_PATCH, json!({}), &CallCtx::new());
        assert!(out.is_error);
        assert!(out
            .text
            .contains("start motion-mcp with --allow-scene-edits"));
        let phase2 = svc.call(profile::MATTE, json!({}), &CallCtx::new());
        assert!(phase2.is_error && phase2.text.contains("not available yet"));
        svc.shutdown();
        let _ = std::fs::remove_dir_all(&repo);
    }

    #[test]
    fn long_replies_are_trimmed_to_the_budget() {
        let mut r = Reply::new(Status::Done, NEXT_DONE);
        r.job = Some("j_0123456789".into());
        r.findings = (0..5)
            .map(|i| format!("finding {i} {}", "x".repeat(120)))
            .collect();
        r.changed = (0..5)
            .map(|i| format!("changed {i} {}", "y".repeat(120)))
            .collect();
        let out = fit_reply(r, 800);
        assert!(out.text.chars().count() <= 800, "{}", out.text.len());
        assert!(out.text.contains("changed:"));
    }
}
