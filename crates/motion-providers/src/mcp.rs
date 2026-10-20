//! Minimal streamable-HTTP MCP client: `initialize`, `tools/list`, `tools/call`.
//! Responses may be plain JSON or an SSE stream; `Mcp-Session-Id` is echoed.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::transport::Transport;

pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The server rejected the bearer token (HTTP 401); callers may refresh and retry.
#[derive(Debug)]
pub struct Unauthorized;

impl std::fmt::Display for Unauthorized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("unauthorized (401): reconnect with `motion-engine providers connect`")
    }
}
impl std::error::Error for Unauthorized {}

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub is_error: bool,
    pub content: Vec<Value>,
    pub structured: Option<Value>,
}

impl ToolResult {
    /// Concatenated text content.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| c.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n")
    }
    /// Structured content, else the text parsed as JSON, else the text as a string.
    pub fn json(&self) -> Value {
        if let Some(s) = &self.structured {
            return s.clone();
        }
        let text = self.text();
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    }
}

pub struct McpClient<'t> {
    url: String,
    transport: &'t dyn Transport,
    token: Option<String>,
    session: Option<String>,
    next_id: u64,
    initialized: bool,
}

impl<'t> McpClient<'t> {
    pub fn new(url: &str, transport: &'t dyn Transport, token: Option<String>) -> Self {
        Self {
            url: url.to_string(),
            transport,
            token,
            session: None,
            next_id: 1,
            initialized: false,
        }
    }

    pub fn set_token(&mut self, token: Option<String>) {
        self.token = token;
        self.session = None;
        self.initialized = false;
    }

    fn send(&mut self, msg: &Value) -> Result<crate::transport::HttpResponse> {
        let auth = self.token.as_ref().map(|t| format!("Bearer {t}"));
        let mut headers: Vec<(&str, &str)> = vec![
            ("Content-Type", "application/json"),
            ("Accept", "application/json, text/event-stream"),
            ("MCP-Protocol-Version", PROTOCOL_VERSION),
        ];
        if let Some(a) = &auth {
            headers.push(("Authorization", a));
        }
        if let Some(s) = &self.session {
            headers.push(("Mcp-Session-Id", s));
        }
        let body = serde_json::to_vec(msg)?;
        let resp = self.transport.post(&self.url, &headers, &body)?;
        if resp.status == 401 {
            return Err(Unauthorized.into());
        }
        if let Some(s) = resp.header("mcp-session-id") {
            self.session = Some(s.to_string());
        }
        Ok(resp)
    }

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        if !self.initialized && method != "initialize" {
            self.initialize()?;
        }
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let resp = self.send(&msg)?;
        if !(200..300).contains(&resp.status) {
            bail!("{method}: HTTP {} {}", resp.status, snippet(&resp.text()));
        }
        let reply = parse_reply(&resp, id).with_context(|| format!("{method}: bad reply"))?;
        if let Some(err) = reply.get("error") {
            bail!("{method}: server error {}", snippet(&err.to_string()));
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("{method}: reply without result"))
    }

    pub fn initialize(&mut self) -> Result<Value> {
        let info = self.rpc(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "motion-engine", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;
        self.initialized = true;
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let resp = self.send(&note)?;
        if resp.status >= 400 {
            bail!("notifications/initialized: HTTP {}", resp.status);
        }
        Ok(info)
    }

    pub fn list_tools(&mut self) -> Result<Vec<Tool>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..50 {
            let params = match &cursor {
                Some(c) => json!({"cursor": c}),
                None => json!({}),
            };
            let page = self.rpc("tools/list", params)?;
            for t in page
                .get("tools")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                tools.push(Tool {
                    name: t
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    description: t
                        .get("description")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                    input_schema: t.get("inputSchema").cloned().unwrap_or(Value::Null),
                });
            }
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
    }

    pub fn call_tool(&mut self, name: &str, args: Value) -> Result<ToolResult> {
        let r = self.rpc("tools/call", json!({"name": name, "arguments": args}))?;
        Ok(ToolResult {
            is_error: r.get("isError").and_then(Value::as_bool).unwrap_or(false),
            content: r
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            structured: r.get("structuredContent").cloned(),
        })
    }
}

/// The JSON-RPC message with `id`, from a JSON body or an SSE stream.
fn parse_reply(resp: &crate::transport::HttpResponse, id: u64) -> Result<Value> {
    let text = resp.text();
    let is_sse = resp
        .header("content-type")
        .is_some_and(|c| c.contains("text/event-stream"));
    let candidates: Vec<Value> = if is_sse {
        let mut out = Vec::new();
        let mut data = String::new();
        for line in text.lines().chain(std::iter::once("")) {
            if let Some(d) = line.strip_prefix("data:") {
                data.push_str(d.trim_start());
            } else if line.trim().is_empty() && !data.is_empty() {
                if let Ok(v) = serde_json::from_str(&data) {
                    out.push(v);
                }
                data.clear();
            }
        }
        out
    } else {
        match serde_json::from_str::<Value>(&text)? {
            Value::Array(a) => a,
            v => vec![v],
        }
    };
    candidates
        .into_iter()
        .find(|m| m.get("id").and_then(Value::as_u64) == Some(id))
        .ok_or_else(|| anyhow!("no reply with id {id}"))
}

pub fn snippet(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() > 200 {
        format!("{}…", s.chars().take(200).collect::<String>())
    } else {
        s.to_string()
    }
}
