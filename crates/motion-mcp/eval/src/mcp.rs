//! The server side: one `motion-mcp` child process for the whole run, driven
//! by the rmcp client over stdio (the way an MCP client would).

use std::path::PathBuf;

use anyhow::{Context, Result};
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use rmcp::transport::TokioChildProcess;
use rmcp::{RoleClient, ServiceExt};
use serde_json::{Map, Value};

/// What a tool answered: the text the model sees, plus the structured reply.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolReply {
    pub text: String,
    pub structured: Option<Value>,
    pub is_error: bool,
}

/// The statuses of the video tools' replies.
const STATUSES: [&str; 6] = [
    "done",
    "running",
    "queued",
    "checked",
    "needs_fix",
    "failed",
];

impl ToolReply {
    /// `status` of the structured reply, else the first word of the text.
    pub fn status(&self) -> Option<String> {
        let from_structured = self
            .structured
            .as_ref()
            .and_then(|s| s.get("status"))
            .and_then(Value::as_str);
        let word = from_structured.or_else(|| {
            self.text
                .split(|c: char| c.is_whitespace() || c == '·')
                .next()
        })?;
        STATUSES.contains(&word).then(|| word.to_string())
    }

    fn structured_str(&self, key: &str) -> Option<String> {
        self.structured
            .as_ref()?
            .get(key)?
            .as_str()
            .map(str::to_string)
    }

    /// `job` of the reply.
    pub fn job(&self) -> Option<String> {
        self.structured_str("job")
    }

    /// `qa` of the reply (`pass` | `fail`).
    pub fn qa(&self) -> Option<String> {
        self.structured_str("qa")
    }

    /// `duration_s` of the reply.
    pub fn duration_s(&self) -> Option<f64> {
        self.structured.as_ref()?.get("duration_s")?.as_f64()
    }

    /// `changed` lines (the server's auto-fixes).
    pub fn changed(&self) -> Vec<String> {
        self.lines("changed")
    }

    /// `findings` lines.
    pub fn findings(&self) -> Vec<String> {
        self.lines("findings")
    }

    fn lines(&self, key: &str) -> Vec<String> {
        self.structured
            .as_ref()
            .and_then(|s| s.get(key))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The first `needs_fix` line (`beat 2 picture: …`) or the `error` of a failure.
    pub fn problem(&self) -> Option<String> {
        let s = self.structured.as_ref()?;
        if let Some(fix) = s
            .get("fixes")
            .and_then(Value::as_array)
            .and_then(|f| f.first())
        {
            let beat = fix.get("beat").and_then(Value::as_u64);
            let field = fix.get("field").and_then(Value::as_str).unwrap_or("story");
            let problem = fix.get("problem").and_then(Value::as_str).unwrap_or("");
            return Some(match beat {
                Some(n) => format!("beat {n} {field}: {problem}"),
                None => format!("{field}: {problem}"),
            });
        }
        s.get("error").and_then(Value::as_str).map(str::to_string)
    }
}

/// Something that runs MCP tools (the real server, or a script in tests).
pub trait ToolHost {
    async fn call(&self, name: &str, args: Map<String, Value>) -> Result<ToolReply>;
}

/// How to start the server.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub server: PathBuf,
    pub profile: String,
    pub repo: PathBuf,
    pub jobs: PathBuf,
    /// Extra flags, passed through (e.g. `--engine PATH`, `--reel-arg=--offline`).
    pub extra: Vec<String>,
}

impl ServerSpec {
    /// `<server> --profile <p> --repo <repo> --jobs <dir> [extra…]`.
    pub fn command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.server);
        cmd.arg("--profile")
            .arg(&self.profile)
            .arg("--repo")
            .arg(&self.repo)
            .arg("--jobs")
            .arg(&self.jobs)
            .args(&self.extra);
        cmd
    }
}

/// A running server session.
pub struct McpSession {
    service: RunningService<RoleClient, ()>,
}

impl McpSession {
    pub async fn start(spec: &ServerSpec) -> Result<Self> {
        let transport = TokioChildProcess::new(spec.command())
            .with_context(|| format!("cannot start {}", spec.server.display()))?;
        let service =
            ().serve(transport)
                .await
                .context("the MCP handshake with motion-mcp failed")?;
        Ok(McpSession { service })
    }

    /// `tools/list`, each tool as its wire JSON (`name`, `description`, `inputSchema`).
    pub async fn tools(&self) -> Result<Vec<Value>> {
        let tools = self
            .service
            .list_all_tools()
            .await
            .context("tools/list failed")?;
        tools
            .iter()
            .map(|t| serde_json::to_value(t).context("a tool does not serialize"))
            .collect()
    }

    /// Stop the server (it stops a render in progress before exiting).
    pub async fn close(self) {
        let _ = self.service.cancel().await;
    }
}

impl ToolHost for McpSession {
    async fn call(&self, name: &str, args: Map<String, Value>) -> Result<ToolReply> {
        let params = CallToolRequestParams::new(name.to_string()).with_arguments(args);
        let result = self
            .service
            .call_tool(params)
            .await
            .with_context(|| format!("tools/call {name} failed"))?;
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text())
            .map(|t| t.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(ToolReply {
            text,
            structured: result.structured_content.clone(),
            is_error: result.is_error.unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reply(structured: Value, text: &str) -> ToolReply {
        ToolReply {
            text: text.to_string(),
            structured: Some(structured),
            is_error: false,
        }
    }

    #[test]
    fn reply_fields() {
        let r = reply(
            json!({"status": "done", "job": "j_0123456789", "qa": "pass", "duration_s": 32.5,
                   "changed": ["beat 2: picture 'berry_bush' → 'strawberry'"]}),
            "done · job j_0123456789",
        );
        assert_eq!(r.status().as_deref(), Some("done"));
        assert_eq!(r.job().as_deref(), Some("j_0123456789"));
        assert_eq!(r.qa().as_deref(), Some("pass"));
        assert_eq!(r.duration_s(), Some(32.5));
        assert_eq!(r.changed().len(), 1);
        assert!(r.findings().is_empty());
        assert_eq!(r.problem(), None);
    }

    #[test]
    fn status_falls_back_to_the_text() {
        let r = ToolReply {
            text: "needs_fix\nfix: beat 2 say: too short".into(),
            structured: None,
            is_error: true,
        };
        assert_eq!(r.status().as_deref(), Some("needs_fix"));
        let dotted = ToolReply {
            text: "running · job j_0123456789 · render 40%".into(),
            structured: None,
            is_error: false,
        };
        assert_eq!(dotted.status().as_deref(), Some("running"));
        let odd = ToolReply {
            text: "Tool not found".into(),
            structured: None,
            is_error: true,
        };
        assert_eq!(odd.status(), None);
    }

    #[test]
    fn problems_name_the_beat() {
        let fix = reply(
            json!({"status": "needs_fix", "fixes": [
                {"beat": 2, "field": "picture", "problem": "no picture for 'zorb'"}]}),
            "",
        );
        assert_eq!(
            fix.problem().as_deref(),
            Some("beat 2 picture: no picture for 'zorb'")
        );
        let story = reply(
            json!({"status": "needs_fix", "fixes": [{"field": "beats", "problem": "need 2 to 12 beats"}]}),
            "",
        );
        assert_eq!(
            story.problem().as_deref(),
            Some("beats: need 2 to 12 beats")
        );
        let failed = reply(json!({"status": "failed", "error": "voice: 401"}), "");
        assert_eq!(failed.problem().as_deref(), Some("voice: 401"));
    }

    #[test]
    fn the_server_command_line() {
        let spec = ServerSpec {
            server: "target/release/motion-mcp".into(),
            profile: "weak".into(),
            repo: "/repo".into(),
            jobs: "/jobs".into(),
            extra: vec!["--reel-arg=--offline".into()],
        };
        let cmd = spec.command();
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--profile",
                "weak",
                "--repo",
                "/repo",
                "--jobs",
                "/jobs",
                "--reel-arg=--offline"
            ]
        );
    }
}
