//! One brief: the conversation between the model and the server, and its metrics.

use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::ValueEnum;
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::briefs::{match_brief, Brief, BriefMatch};
use crate::mcp::{ToolHost, ToolReply};
use crate::openai::{clip, parse_arguments, parse_reply, Chat, ToolCall, Usage};

/// The system message of every conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SystemMode {
    /// "You are a helpful assistant. Use the available tools when they help."
    Generic,
    /// No system message at all.
    #[value(name = "none")]
    NoSystem,
}

pub const GENERIC_SYSTEM: &str =
    "You are a helpful assistant. Use the available tools when they help.";

impl SystemMode {
    pub fn name(self) -> &'static str {
        match self {
            SystemMode::Generic => "generic",
            SystemMode::NoSystem => "none",
        }
    }

    pub fn text(self) -> Option<&'static str> {
        match self {
            SystemMode::Generic => Some(GENERIC_SYSTEM),
            SystemMode::NoSystem => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunConfig {
    pub model: String,
    pub temperature: f64,
    pub seed: u64,
    pub max_tokens: u64,
    /// Most model requests per brief.
    pub max_turns: usize,
    pub system: SystemMode,
    /// Rewrite every `make_video` to `mode: check` and stop at `checked`.
    pub no_render: bool,
    /// The server's job store (`<jobs>/<job>/request.json` holds the stored story).
    pub jobs: PathBuf,
}

/// One tool call of the model, as sent and as answered.
#[derive(Debug, Clone, Serialize)]
pub struct CallRecord {
    pub turn: usize,
    pub name: String,
    /// The arguments exactly as the model wrote them.
    pub arguments: String,
    /// The arguments forwarded to the server when they differ (`--no-render`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forwarded: Option<Value>,
    /// The arguments were not a JSON object; the server was not called.
    pub bad_args: bool,
    /// The MCP layer refused the call (unknown tool, protocol error).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub refused: bool,
    pub status: Option<String>,
    pub is_error: bool,
    /// What the model was told.
    pub reply: String,
    pub reply_chars: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct BriefResult {
    pub id: String,
    pub topic: String,
    pub structure: String,
    pub success: bool,
    /// `status` of the last video reply.
    pub final_status: Option<String>,
    pub qa: Option<String>,
    /// Tool calls the model made (including ones with unusable arguments).
    pub tool_calls: usize,
    /// Calls whose arguments were not a JSON object.
    pub bad_args: usize,
    /// Replies with `needs_fix`: the fix retries the model had to make.
    pub needs_fix: usize,
    /// Calls that repeat an earlier call of the same brief word for word
    /// (a model stuck on a fix-it message).
    pub repeated_calls: usize,
    /// Calls the MCP layer refused (unknown tool, protocol error).
    pub tool_errors: usize,
    /// Model requests.
    pub turns: usize,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// `prompt_tokens` of the first request (system + brief + tool definitions).
    pub first_prompt_tokens: Option<u64>,
    pub wall_s: f64,
    /// Length of the finished video.
    pub duration_s: Option<f64>,
    pub job: Option<String>,
    pub beats: usize,
    /// The server's auto-fix lines.
    pub changed: Vec<String>,
    pub findings: Vec<String>,
    pub brief_match: BriefMatch,
    pub note: String,
    /// What the model said when it stopped calling tools.
    pub final_text: String,
    /// `finish_reason` of the model's last answer (`stop`, `length`, `tool_calls` …).
    pub finish_reason: Option<String>,
    /// The server's raw last message when it had no tool call (clipped), to see
    /// what a model that "did not call" actually sent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_raw: Option<String>,
    pub calls: Vec<CallRecord>,
}

/// The request body of one turn.
pub fn request_body(cfg: &RunConfig, messages: &[Value], tools: &[Value]) -> Value {
    let mut body = json!({
        "model": cfg.model,
        "messages": messages,
        "temperature": cfg.temperature,
        "seed": cfg.seed,
        "max_tokens": cfg.max_tokens,
    });
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools.to_vec());
    }
    body
}

/// The conversation before the model's first turn.
pub fn opening_messages(cfg: &RunConfig, brief: &Brief) -> Vec<Value> {
    let mut messages = Vec::new();
    if let Some(text) = cfg.system.text() {
        messages.push(json!({"role": "system", "content": text}));
    }
    messages.push(json!({"role": "user", "content": brief.brief}));
    messages
}

fn is_video_tool(name: &str) -> bool {
    matches!(name, "make_video" | "revise_video" | "get_video")
}

/// The run is over: a video tool reported `done` or `failed`, or (with
/// `--no-render`) `make_video` reported `checked`.
pub fn is_terminal(tool: &str, status: Option<&str>, no_render: bool) -> bool {
    is_video_tool(tool)
        && match status {
            Some("done") | Some("failed") => true,
            Some("checked") => no_render && tool == "make_video",
            _ => false,
        }
}

/// Everything learned from the video replies so far.
#[derive(Default)]
struct Tracker {
    status: Option<String>,
    qa: Option<String>,
    job: Option<String>,
    duration_s: Option<f64>,
    changed: Vec<String>,
    findings: Vec<String>,
    problem: Option<String>,
    /// The `story` argument of the last `make_video` the server accepted.
    accepted_story: Option<Value>,
    /// The `story` argument of the last `make_video` of any kind.
    last_story: Option<Value>,
}

impl Tracker {
    fn video_reply(&mut self, reply: &ToolReply, status: &str) {
        self.status = Some(status.to_string());
        self.qa = reply.qa();
        if let Some(job) = reply.job() {
            self.job = Some(job);
        }
        if let Some(d) = reply.duration_s() {
            self.duration_s = Some(d);
        }
        for line in reply.changed() {
            if !self.changed.contains(&line) {
                self.changed.push(line);
            }
        }
        for line in reply.findings() {
            if !self.findings.contains(&line) && self.findings.len() < 5 {
                self.findings.push(line);
            }
        }
        self.problem = reply.problem();
    }
}

/// The story the server stored for `job` (`<jobs>/<job>/request.json` → `story`).
pub fn stored_story(jobs: &Path, job: &str) -> Option<Value> {
    let text = std::fs::read_to_string(jobs.join(job).join("request.json")).ok()?;
    let mut request: Value = serde_json::from_str(&text).ok()?;
    match request.get_mut("story").map(Value::take) {
        Some(Value::Null) | None => None,
        Some(story) => Some(story),
    }
}

/// Run one brief to its end (a terminal reply, a plain answer, the turn limit
/// or a model error) and measure it.
pub async fn run_brief<C: Chat, H: ToolHost>(
    chat: &C,
    host: &H,
    cfg: &RunConfig,
    tools: &[Value],
    brief: &Brief,
) -> BriefResult {
    let started = Instant::now();
    let mut messages = opening_messages(cfg, brief);
    let mut calls: Vec<CallRecord> = Vec::new();
    let mut tracker = Tracker::default();
    let mut usage = Usage::default();
    let mut first_prompt_tokens = None;
    let mut turns = 0;
    let mut bad_args = 0;
    let mut needs_fix = 0;
    let mut repeated_calls = 0;
    let mut tool_errors = 0;
    let mut final_text = String::new();
    let mut finish_reason: Option<String> = None;
    let mut final_raw: Option<String> = None;
    let mut model_error: Option<String> = None;
    let mut terminal = false;

    'turns: while turns < cfg.max_turns && !terminal {
        turns += 1;
        let body = request_body(cfg, &messages, tools);
        let reply = match chat
            .complete(&body)
            .await
            .and_then(|b| parse_reply(&b, turns))
        {
            Ok(r) => r,
            Err(e) => {
                model_error = Some(format!("{e:#}"));
                break;
            }
        };
        usage.prompt_tokens += reply.usage.prompt_tokens;
        usage.completion_tokens += reply.usage.completion_tokens;
        first_prompt_tokens.get_or_insert(reply.usage.prompt_tokens);
        messages.push(reply.assistant_message());
        finish_reason = reply.finish_reason.clone();
        if reply.tool_calls.is_empty() {
            final_raw = Some(clip(&reply.raw_message.to_string(), 1000));
            final_text = reply.content;
            break;
        }
        for call in &reply.tool_calls {
            if calls
                .iter()
                .any(|c| c.name == call.name && c.arguments == call.arguments)
            {
                repeated_calls += 1;
            }
            let record = run_call(host, cfg, call, turns, &mut tracker).await;
            if record.bad_args {
                bad_args += 1;
            }
            if record.status.as_deref() == Some("needs_fix") {
                needs_fix += 1;
            }
            if record.refused {
                tool_errors += 1;
            }
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call.id,
                "name": call.name,
                "content": record.reply,
            }));
            let done = is_terminal(&record.name, record.status.as_deref(), cfg.no_render);
            calls.push(record);
            if done {
                terminal = true;
                // Calls after the finishing one are not run (no second render).
                break;
            }
            if calls.len() > 64 {
                break 'turns;
            }
        }
    }
    let wall_s = started.elapsed().as_secs_f64();

    let story = tracker
        .job
        .as_deref()
        .filter(|_| !cfg.no_render)
        .and_then(|job| stored_story(&cfg.jobs, job))
        .or_else(|| tracker.accepted_story.clone())
        .or_else(|| tracker.last_story.clone())
        .unwrap_or(Value::Null);
    let brief_match = match_brief(&story, &brief.expect);

    let status = tracker.status.as_deref();
    let video_ok = if cfg.no_render {
        status == Some("checked")
    } else {
        status == Some("done") && tracker.qa.as_deref() == Some("pass")
    };
    let success = video_ok && brief_match.ok;
    let note = if success {
        match tracker.changed.len() {
            0 => String::new(),
            n => format!("{n} auto-fix"),
        }
    } else {
        let mut why = failure_note(
            model_error.as_deref(),
            &calls,
            &final_text,
            finish_reason.as_deref(),
            &tracker,
            &brief_match,
            cfg.no_render,
        );
        if repeated_calls > 0 {
            why.push_str(&format!("; same call repeated {repeated_calls}x"));
        }
        if !terminal && model_error.is_none() && turns >= cfg.max_turns && !calls.is_empty() {
            why.push_str("; max turns");
        }
        why
    };

    // A render the model walked away from keeps the next brief waiting: let
    // it finish (not counted in the metrics).
    if !cfg.no_render && matches!(status, Some("running") | Some("queued")) {
        if let Some(job) = &tracker.job {
            let mut args = Map::new();
            args.insert("job".into(), json!(job));
            args.insert("wait_s".into(), json!(300));
            let _ = host.call("get_video", args).await;
        }
    }

    BriefResult {
        id: brief.id.clone(),
        topic: brief.topic.clone(),
        structure: brief.structure.clone(),
        success,
        final_status: tracker.status.clone(),
        qa: tracker.qa.clone(),
        tool_calls: calls.len(),
        bad_args,
        needs_fix,
        repeated_calls,
        tool_errors,
        turns,
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        first_prompt_tokens,
        wall_s,
        duration_s: tracker.duration_s,
        job: tracker.job.clone(),
        beats: brief_match.beats,
        changed: tracker.changed.clone(),
        findings: tracker.findings.clone(),
        brief_match,
        note,
        final_text: clip(final_text.trim(), 600),
        finish_reason,
        final_raw,
        calls,
    }
}

/// Run one tool call of the model: unusable arguments are answered with an
/// explanation (the server is not called); `--no-render` turns `make_video`
/// into a check.
async fn run_call<H: ToolHost>(
    host: &H,
    cfg: &RunConfig,
    call: &ToolCall,
    turn: usize,
    tracker: &mut Tracker,
) -> CallRecord {
    let mut record = CallRecord {
        turn,
        name: call.name.clone(),
        arguments: call.arguments.clone(),
        forwarded: None,
        bad_args: false,
        refused: false,
        status: None,
        is_error: false,
        reply: String::new(),
        reply_chars: 0,
    };
    let mut args = match parse_arguments(&call.arguments) {
        Ok(a) => a,
        Err(why) => {
            record.bad_args = true;
            record.is_error = true;
            record.reply = format!(
                "Error: {why}. Call {} again with valid JSON arguments.",
                call.name
            );
            record.reply_chars = record.reply.chars().count();
            return record;
        }
    };
    if cfg.no_render && call.name == "make_video" {
        args.insert("mode".into(), json!("check"));
        record.forwarded = Some(Value::Object(args.clone()));
    }
    let story = (call.name == "make_video")
        .then(|| args.get("story").cloned())
        .flatten();
    match host.call(&call.name, args).await {
        Ok(reply) => {
            record.status = reply.status();
            record.is_error = reply.is_error;
            record.reply = if reply.text.is_empty() {
                reply
                    .structured
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_default()
            } else {
                reply.text.clone()
            };
            if is_video_tool(&call.name) {
                if let Some(status) = record.status.clone() {
                    tracker.video_reply(&reply, &status);
                    if call.name == "make_video" {
                        tracker.last_story = story.clone();
                        if status != "needs_fix" && status != "failed" {
                            tracker.accepted_story = story;
                        }
                    }
                } else if call.name == "make_video" {
                    tracker.last_story = story;
                }
            }
        }
        Err(e) => {
            record.is_error = true;
            record.refused = true;
            record.reply = format!("Error: {}", clip(&format!("{e:#}"), 300));
            if call.name == "make_video" {
                tracker.last_story = story;
            }
        }
    }
    record.reply_chars = record.reply.chars().count();
    record
}

/// What the tool definitions cost on this model: the `prompt_tokens` of one
/// request with the tools minus one without (a one-token answer each time).
/// None when either request fails or the server reports no usage.
pub async fn probe_tool_tokens<C: Chat>(
    chat: &C,
    cfg: &RunConfig,
    tools: &[Value],
    brief: &Brief,
) -> Option<u64> {
    let messages = opening_messages(cfg, brief);
    let ask = |with_tools: &[Value]| {
        let mut body = request_body(cfg, &messages, with_tools);
        body["max_tokens"] = json!(1);
        body
    };
    let prompt_tokens = |body: Value| async move {
        chat.complete(&body)
            .await
            .ok()
            .and_then(|b| parse_reply(&b, 0).ok())
            .map(|r| r.usage.prompt_tokens)
            .filter(|&t| t > 0)
    };
    let with = prompt_tokens(ask(tools)).await?;
    let without = prompt_tokens(ask(&[])).await?;
    Some(with.saturating_sub(without))
}

/// Why a brief did not succeed, in one line.
fn failure_note(
    model_error: Option<&str>,
    calls: &[CallRecord],
    final_text: &str,
    finish_reason: Option<&str>,
    tracker: &Tracker,
    brief_match: &BriefMatch,
    no_render: bool,
) -> String {
    if let Some(e) = model_error {
        return format!("model error: {}", clip(e, 90));
    }
    if calls.is_empty() {
        let text = final_text.replace('\n', " ");
        return if text.trim().is_empty() {
            format!(
                "no tool call: empty answer (finish_reason {})",
                finish_reason.unwrap_or("none")
            )
        } else {
            format!("no tool call: {}", clip(&text, 70))
        };
    }
    let problem = || {
        tracker
            .problem
            .as_deref()
            .map(|p| clip(p, 90))
            .unwrap_or_default()
    };
    match tracker.status.as_deref() {
        None => {
            if calls.iter().all(|c| c.bad_args) {
                "bad arguments every time".to_string()
            } else {
                "no video reply".to_string()
            }
        }
        Some("needs_fix") => format!("needs_fix: {}", problem()),
        Some("failed") => format!("failed: {}", problem()),
        Some(s @ ("running" | "queued")) => format!("stopped while {s}"),
        Some("checked") if !no_render => "only checked, never rendered".to_string(),
        Some("done") if tracker.qa.as_deref() != Some("pass") => {
            let finding = tracker
                .findings
                .first()
                .map(|f| format!(": {}", clip(f, 70)))
                .unwrap_or_default();
            format!("qa {}{finding}", tracker.qa.as_deref().unwrap_or("missing"))
        }
        Some(_) if !brief_match.ok => format!("brief: {}", brief_match.problem()),
        Some(s) => format!("stopped at {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::briefs::Expect;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// A model that answers from a script and remembers the requests.
    struct ScriptedChat {
        answers: Mutex<VecDeque<anyhow::Result<Value>>>,
        requests: Mutex<Vec<Value>>,
    }

    impl ScriptedChat {
        fn new(answers: Vec<anyhow::Result<Value>>) -> Self {
            ScriptedChat {
                answers: Mutex::new(answers.into()),
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl Chat for ScriptedChat {
        async fn complete(&self, request: &Value) -> anyhow::Result<Value> {
            self.requests.lock().unwrap().push(request.clone());
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(anyhow::anyhow!("the script ran out")))
        }
    }

    /// A server that answers from a script and remembers the calls.
    struct ScriptedHost {
        replies: Mutex<VecDeque<ToolReply>>,
        calls: Mutex<Vec<(String, Map<String, Value>)>>,
    }

    impl ScriptedHost {
        fn new(replies: Vec<Value>) -> Self {
            ScriptedHost {
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .map(|s| ToolReply {
                            text: format!("{} · text", s["status"].as_str().unwrap_or("?")),
                            is_error: s["status"] == "needs_fix" || s["status"] == "failed",
                            structured: Some(s),
                        })
                        .collect(),
                ),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    impl ToolHost for ScriptedHost {
        async fn call(&self, name: &str, args: Map<String, Value>) -> anyhow::Result<ToolReply> {
            self.calls.lock().unwrap().push((name.to_string(), args));
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| anyhow::anyhow!("unknown tool {name}"))
        }
    }

    fn brief() -> Brief {
        Brief {
            id: "ai_agent_parts".into(),
            topic: "ai".into(),
            structure: "list".into(),
            brief: "Make a video about agents.".into(),
            expect: Expect {
                beats: [2, 6],
                key_terms: vec!["memory".into(), "10,000".into()],
            },
        }
    }

    fn cfg(no_render: bool) -> RunConfig {
        RunConfig {
            model: "lfm-eval".into(),
            temperature: 0.2,
            seed: 7,
            max_tokens: 3000,
            max_turns: 4,
            system: SystemMode::Generic,
            no_render,
            jobs: std::env::temp_dir().join("motion-mcp-eval-no-such-jobs"),
        }
    }

    fn story_args() -> String {
        json!({"story": {"title": "agents", "beats": [
            {"say": "An agent needs memory to keep track of its work."},
            {"say": "It can cost 10000 dollars to run for a month.", "number": "10,000"}
        ]}})
        .to_string()
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> Value {
        json!({"id": id, "type": "function", "function": {"name": name, "arguments": arguments}})
    }

    fn answer(calls: Vec<Value>, prompt: u64, completion: u64) -> anyhow::Result<Value> {
        Ok(json!({
            "choices": [{"finish_reason": "tool_calls",
                         "message": {"role": "assistant", "content": null, "tool_calls": calls}}],
            "usage": {"prompt_tokens": prompt, "completion_tokens": completion}
        }))
    }

    fn text_answer(text: &str) -> anyhow::Result<Value> {
        Ok(json!({
            "choices": [{"message": {"role": "assistant", "content": text}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 10}
        }))
    }

    fn tools() -> Vec<Value> {
        vec![json!({"type": "function", "function": {"name": "make_video"}})]
    }

    #[test]
    fn requests_carry_the_settings() {
        let c = cfg(false);
        let messages = opening_messages(&c, &brief());
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], GENERIC_SYSTEM);
        assert_eq!(messages[1]["content"], "Make a video about agents.");
        let body = request_body(&c, &messages, &tools());
        assert_eq!(body["model"], "lfm-eval");
        assert_eq!(body["seed"], 7);
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
        let none = RunConfig {
            system: SystemMode::NoSystem,
            ..c
        };
        let messages = opening_messages(&none, &brief());
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert!(request_body(&none, &messages, &[]).get("tools").is_none());
    }

    #[tokio::test]
    async fn the_tool_cost_is_the_prompt_difference() {
        let chat = ScriptedChat::new(vec![text_answer_with(1480), text_answer_with(310)]);
        let t = probe_tool_tokens(&chat, &cfg(false), &tools(), &brief()).await;
        assert_eq!(t, Some(1170));
        {
            let requests = chat.requests.lock().unwrap();
            assert_eq!(requests[0]["max_tokens"], 1);
            assert!(requests[0].get("tools").is_some());
            assert!(requests[1].get("tools").is_none());
        }
        // A failing or usage-less endpoint gives no number.
        let chat = ScriptedChat::new(vec![Err(anyhow::anyhow!("down"))]);
        assert_eq!(
            probe_tool_tokens(&chat, &cfg(false), &tools(), &brief()).await,
            None
        );
        let chat = ScriptedChat::new(vec![text_answer_with(0), text_answer_with(0)]);
        assert_eq!(
            probe_tool_tokens(&chat, &cfg(false), &tools(), &brief()).await,
            None
        );
    }

    fn text_answer_with(prompt_tokens: u64) -> anyhow::Result<Value> {
        Ok(json!({
            "choices": [{"message": {"role": "assistant", "content": "x"}}],
            "usage": {"prompt_tokens": prompt_tokens, "completion_tokens": 1}
        }))
    }

    #[test]
    fn terminal_states() {
        assert!(is_terminal("make_video", Some("done"), false));
        assert!(is_terminal("get_video", Some("failed"), false));
        assert!(!is_terminal("make_video", Some("running"), false));
        assert!(!is_terminal("get_video", Some("queued"), false));
        assert!(!is_terminal("make_video", Some("needs_fix"), true));
        // `checked` ends only a no-render make_video.
        assert!(!is_terminal("make_video", Some("checked"), false));
        assert!(is_terminal("make_video", Some("checked"), true));
        assert!(!is_terminal("find_assets", Some("done"), false));
    }

    #[tokio::test]
    async fn one_call_one_video() {
        let chat = ScriptedChat::new(vec![answer(
            vec![tool_call("c1", "make_video", &story_args())],
            1400,
            200,
        )]);
        let host = ScriptedHost::new(vec![json!({
            "status": "done", "job": "j_0123456789", "qa": "pass", "duration_s": 21.5,
            "changed": ["beat 2: picture 'zorb' → 'ball'"]
        })]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(r.success, "{}", r.note);
        assert_eq!(r.tool_calls, 1);
        assert_eq!((r.bad_args, r.needs_fix, r.turns), (0, 0, 1));
        assert_eq!((r.prompt_tokens, r.completion_tokens), (1400, 200));
        assert_eq!(r.first_prompt_tokens, Some(1400));
        assert_eq!(r.job.as_deref(), Some("j_0123456789"));
        assert_eq!(r.duration_s, Some(21.5));
        assert_eq!(r.beats, 2);
        assert_eq!(r.changed.len(), 1);
        assert_eq!(r.note, "1 auto-fix");
        assert_eq!(r.calls[0].arguments, story_args());
        assert!(r.calls[0].forwarded.is_none());
        // Render mode forwards what the model sent.
        assert!(host.calls.lock().unwrap()[0].1.get("mode").is_none());
    }

    #[tokio::test]
    async fn qa_fail_or_a_wrong_story_is_no_success() {
        let make = |id: &str| vec![tool_call(id, "make_video", &story_args())];
        let chat = ScriptedChat::new(vec![answer(make("a"), 1, 1)]);
        let host = ScriptedHost::new(vec![json!({
            "status": "done", "job": "j_0123456789", "qa": "fail",
            "findings": ["beat 3: title covers 6% of the picture"]
        })]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(!r.success);
        assert_eq!(r.note, "qa fail: beat 3: title covers 6% of the picture");

        // QA passes but the story lacks a key term.
        let other = Brief {
            expect: Expect {
                beats: [2, 6],
                key_terms: vec!["guardrails".into()],
            },
            ..brief()
        };
        let chat = ScriptedChat::new(vec![answer(make("a"), 1, 1)]);
        let host = ScriptedHost::new(vec![json!({"status": "done", "qa": "pass"})]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &other).await;
        assert!(!r.success);
        assert_eq!(r.note, "brief: missing: guardrails");
    }

    #[tokio::test]
    async fn bad_json_is_answered_and_retried() {
        let chat = ScriptedChat::new(vec![
            answer(
                vec![tool_call("c1", "make_video", "{\"story\": {\"beats\": [}")],
                1000,
                50,
            ),
            answer(
                vec![tool_call("c2", "make_video", &story_args())],
                1100,
                150,
            ),
        ]);
        let host = ScriptedHost::new(vec![
            json!({"status": "done", "qa": "pass", "job": "j_0123456789"}),
        ]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(r.success, "{}", r.note);
        assert_eq!((r.tool_calls, r.bad_args, r.turns), (2, 1, 2));
        assert_eq!(r.tool_errors, 0);
        assert!(r.calls[0].bad_args && r.calls[0].status.is_none());
        assert!(
            r.calls[0].reply.contains("not valid JSON"),
            "{}",
            r.calls[0].reply
        );
        // Only the second call reached the server.
        assert_eq!(host.calls.lock().unwrap().len(), 1);
        // The model saw its error as a tool message with the call id.
        let second = chat.requests.lock().unwrap()[1].clone();
        let last = second["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(last["role"], "tool");
        assert_eq!(last["tool_call_id"], "c1");
        assert!(last["content"].as_str().unwrap().contains("valid JSON"));
        assert_eq!((r.prompt_tokens, r.completion_tokens), (2100, 200));
    }

    #[tokio::test]
    async fn needs_fix_counts_as_a_retry() {
        let chat = ScriptedChat::new(vec![
            answer(vec![tool_call("c1", "make_video", &story_args())], 10, 10),
            answer(vec![tool_call("c2", "make_video", &story_args())], 10, 10),
        ]);
        let host = ScriptedHost::new(vec![
            json!({"status": "needs_fix",
                   "fixes": [{"beat": 2, "field": "say", "problem": "too short"}]}),
            json!({"status": "done", "qa": "pass", "job": "j_0123456789"}),
        ]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(r.success);
        assert_eq!((r.tool_calls, r.needs_fix), (2, 1));
    }

    #[tokio::test]
    async fn no_render_forces_check_and_stops_at_checked() {
        let chat = ScriptedChat::new(vec![answer(
            vec![tool_call("c1", "make_video", &story_args())],
            10,
            10,
        )]);
        let host = ScriptedHost::new(vec![json!({"status": "checked",
            "plan": ["beat 1: picture"], "changed": ["beat 1: title shortened"]})]);
        let r = run_brief(&chat, &host, &cfg(true), &tools(), &brief()).await;
        assert!(r.success, "{}", r.note);
        assert_eq!(r.final_status.as_deref(), Some("checked"));
        let (name, args) = host.calls.lock().unwrap()[0].clone();
        assert_eq!(name, "make_video");
        assert_eq!(args["mode"], "check");
        // The raw arguments stay as the model wrote them; the forwarded ones are kept apart.
        assert!(!r.calls[0].arguments.contains("check"));
        assert_eq!(r.calls[0].forwarded.as_ref().unwrap()["mode"], "check");
        // The brief is matched against the story the model sent (no job files in check mode).
        assert_eq!(r.beats, 2);
    }

    #[tokio::test]
    async fn a_model_that_renders_in_no_render_mode_is_still_checked() {
        // The model asks for mode "render"; it is rewritten before forwarding.
        let args =
            json!({"mode": "render", "story": {"beats": [{"say": "memory"}, {"say": "10000"}]}});
        let chat = ScriptedChat::new(vec![answer(
            vec![tool_call("c1", "make_video", &args.to_string())],
            1,
            1,
        )]);
        let host = ScriptedHost::new(vec![json!({"status": "checked"})]);
        let r = run_brief(&chat, &host, &cfg(true), &tools(), &brief()).await;
        assert!(r.success);
        assert_eq!(host.calls.lock().unwrap()[0].1["mode"], "check");
    }

    #[tokio::test]
    async fn running_continues_with_get_video() {
        let get = json!({"job": "j_0123456789", "wait_s": 120}).to_string();
        let chat = ScriptedChat::new(vec![
            answer(vec![tool_call("c1", "make_video", &story_args())], 10, 10),
            answer(vec![tool_call("c2", "get_video", &get)], 10, 10),
        ]);
        let host = ScriptedHost::new(vec![
            json!({"status": "running", "job": "j_0123456789"}),
            json!({"status": "done", "qa": "pass", "job": "j_0123456789", "duration_s": 30.0}),
        ]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(r.success, "{}", r.note);
        assert_eq!(r.tool_calls, 2);
        assert_eq!(r.duration_s, Some(30.0));
    }

    #[tokio::test]
    async fn walking_away_from_a_render_is_a_failure_and_drains_it() {
        let chat = ScriptedChat::new(vec![
            answer(vec![tool_call("c1", "make_video", &story_args())], 10, 10),
            text_answer("The video is rendering, I will wait."),
        ]);
        let host = ScriptedHost::new(vec![
            json!({"status": "running", "job": "j_0123456789"}),
            json!({"status": "done", "qa": "pass", "job": "j_0123456789"}),
        ]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(!r.success);
        assert_eq!(r.note, "stopped while running");
        assert_eq!(r.final_text, "The video is rendering, I will wait.");
        // The harness waited for the job itself (not counted as a model call).
        let calls = host.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].0, "get_video");
        assert_eq!(r.tool_calls, 1);
    }

    #[tokio::test]
    async fn no_tool_call_and_model_errors() {
        let chat = ScriptedChat::new(vec![text_answer("Here is an idea for a video.")]);
        let host = ScriptedHost::new(vec![]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(!r.success);
        assert_eq!(r.tool_calls, 0);
        assert_eq!(r.note, "no tool call: Here is an idea for a video.");
        assert_eq!(r.finish_reason, None);

        // An empty answer says why it stopped.
        let chat = ScriptedChat::new(vec![Ok(json!({
            "choices": [{"finish_reason": "length",
                         "message": {"role": "assistant", "content": ""}}],
            "usage": {"prompt_tokens": 1, "completion_tokens": 3000}
        }))]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert_eq!(r.note, "no tool call: empty answer (finish_reason length)");
        assert_eq!(r.finish_reason.as_deref(), Some("length"));

        let chat = ScriptedChat::new(vec![Err(anyhow::anyhow!("HTTP 500: model crashed"))]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(!r.success);
        assert_eq!(r.turns, 1);
        assert_eq!(r.note, "model error: HTTP 500: model crashed");
    }

    #[tokio::test]
    async fn the_turn_limit_ends_a_loop() {
        let bad = || answer(vec![tool_call("c", "make_video", "{nope")], 5, 5);
        let chat = ScriptedChat::new(vec![bad(), bad(), bad(), bad(), bad()]);
        let host = ScriptedHost::new(vec![]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert!(!r.success);
        assert_eq!((r.turns, r.tool_calls, r.bad_args), (4, 4, 4));
        assert_eq!(r.repeated_calls, 3);
        assert_eq!(
            r.note,
            "bad arguments every time; same call repeated 3x; max turns"
        );
    }

    #[tokio::test]
    async fn an_unknown_tool_is_an_error_not_a_crash() {
        let chat = ScriptedChat::new(vec![
            answer(vec![tool_call("c1", "teleport", "{}")], 5, 5),
            text_answer("Sorry."),
        ]);
        let host = ScriptedHost::new(vec![]);
        let r = run_brief(&chat, &host, &cfg(false), &tools(), &brief()).await;
        assert_eq!(r.tool_errors, 1);
        assert!(r.calls[0].reply.starts_with("Error: unknown tool teleport"));
        assert!(r.calls[0].is_error);
    }

    #[tokio::test]
    async fn the_stored_story_beats_the_arguments() {
        let jobs =
            std::env::temp_dir().join(format!("motion-mcp-eval-test-{}", std::process::id()));
        let dir = jobs.join("j_0123456789");
        std::fs::create_dir_all(&dir).unwrap();
        // The server's auto-fixed story has the key terms the model's own lacked.
        std::fs::write(
            dir.join("request.json"),
            json!({"story": {"beats": [{"say": "Memory."}, {"say": "10,000 homes."}]}}).to_string(),
        )
        .unwrap();
        let sent =
            json!({"story": {"beats": [{"say": "nothing"}, {"say": "relevant"}]}}).to_string();
        let chat = ScriptedChat::new(vec![answer(
            vec![tool_call("c1", "make_video", &sent)],
            1,
            1,
        )]);
        let host = ScriptedHost::new(vec![
            json!({"status": "done", "qa": "pass", "job": "j_0123456789"}),
        ]);
        let c = RunConfig {
            jobs: jobs.clone(),
            ..cfg(false)
        };
        let r = run_brief(&chat, &host, &c, &tools(), &brief()).await;
        assert!(r.success, "{}", r.note);
        assert_eq!(
            stored_story(&jobs, "j_0123456789").unwrap()["beats"][0]["say"],
            "Memory."
        );
        assert!(stored_story(&jobs, "j_9999999999").is_none());
        let _ = std::fs::remove_dir_all(&jobs);
    }
}
