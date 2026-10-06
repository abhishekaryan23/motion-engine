//! The model side: OpenAI-compatible `/chat/completions`.
//!
//! Plain HTTP to a local server (LM Studio, llama.cpp, Ollama). No API keys
//! are read from anywhere: paid hosted models are out of scope for this
//! harness (they need the owner's OK, see the README).

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};

/// Convert MCP tools (`{name, description, inputSchema}` as serialized by the
/// rmcp client) to the OpenAI `tools` array.
pub fn to_openai_tools(mcp_tools: &[Value]) -> Vec<Value> {
    mcp_tools
        .iter()
        .map(|t| {
            let schema = t
                .get("inputSchema")
                .filter(|s| s.is_object())
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            json!({
                "type": "function",
                "function": {
                    "name": t.get("name").and_then(Value::as_str).unwrap_or_default(),
                    "description": t.get("description").and_then(Value::as_str).unwrap_or_default(),
                    "parameters": schema,
                }
            })
        })
        .collect()
}

/// One tool call the model made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// The arguments exactly as the model wrote them (a JSON string, valid or not).
    pub arguments: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// One model answer.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatReply {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub finish_reason: Option<String>,
    /// `choices[0].message` as the server sent it (for diagnosing empty answers).
    pub raw_message: Value,
}

impl ChatReply {
    /// The assistant message to append to the conversation (clean OpenAI shape).
    pub fn assistant_message(&self) -> Value {
        let mut m = json!({"role": "assistant", "content": self.content});
        if !self.tool_calls.is_empty() {
            m["tool_calls"] = Value::Array(
                self.tool_calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": {"name": c.name, "arguments": c.arguments},
                        })
                    })
                    .collect(),
            );
        }
        m
    }
}

/// Read `choices[0].message` and `usage` of a `/chat/completions` body.
/// `turn` only seeds ids for calls the server left without one.
pub fn parse_reply(body: &Value, turn: usize) -> Result<ChatReply> {
    let choice = body
        .pointer("/choices/0")
        .ok_or_else(|| anyhow!("no choices in the reply: {}", clip(&body.to_string(), 300)))?;
    let message = choice
        .get("message")
        .ok_or_else(|| anyhow!("choice without a message"))?;
    let content = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        // Some servers send content parts.
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let mut tool_calls = Vec::new();
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        for (i, c) in calls.iter().enumerate() {
            let f = c.get("function").unwrap_or(c);
            let name = f
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let arguments = match f.get("arguments") {
                Some(Value::String(s)) => s.clone(),
                // An already-parsed object (some servers).
                Some(v @ Value::Object(_)) => v.to_string(),
                _ => String::new(),
            };
            let id = c
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("call_{turn}_{i}"));
            tool_calls.push(ToolCall {
                id,
                name,
                arguments,
            });
        }
    }
    let usage = Usage {
        prompt_tokens: body
            .pointer("/usage/prompt_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        completion_tokens: body
            .pointer("/usage/completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };
    Ok(ChatReply {
        content,
        tool_calls,
        usage,
        finish_reason: choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string),
        raw_message: message.clone(),
    })
}

/// Parse the arguments string of a tool call: a JSON object (empty text means
/// no arguments). The error is written for the model.
pub fn parse_arguments(raw: &str) -> Result<Map<String, Value>, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(other) => Err(format!(
            "the arguments must be one JSON object, not {}",
            json_kind(&other)
        )),
        Err(e) => Err(format!("the arguments are not valid JSON ({e})")),
    }
}

fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Cut to at most `max` characters (on a char boundary), adding "…".
pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Something that answers `/chat/completions` requests (HTTP, or a script in tests).
pub trait Chat {
    /// Send one request body, return the raw JSON body of the answer.
    async fn complete(&self, request: &Value) -> Result<Value>;
}

/// `POST <endpoint>/chat/completions` over plain HTTP(S).
pub struct HttpChat {
    agent: ureq::Agent,
    url: String,
}

impl HttpChat {
    pub fn new(endpoint: &str) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            // A 2.6B model on a laptop can take a while; a long story is slow.
            .timeout_read(Duration::from_secs(900))
            .timeout_write(Duration::from_secs(60))
            .build();
        HttpChat {
            agent,
            url: format!("{}/chat/completions", endpoint.trim_end_matches('/')),
        }
    }
}

impl Chat for HttpChat {
    async fn complete(&self, request: &Value) -> Result<Value> {
        let agent = self.agent.clone();
        let url = self.url.clone();
        let body = request.clone();
        tokio::task::spawn_blocking(move || post_json(&agent, &url, &body))
            .await
            .context("the HTTP task panicked")?
    }
}

fn post_json(agent: &ureq::Agent, url: &str, body: &Value) -> Result<Value> {
    match agent.post(url).send_json(body) {
        Ok(resp) => resp
            .into_json::<Value>()
            .with_context(|| format!("{url}: the reply is not JSON")),
        Err(ureq::Error::Status(code, resp)) => {
            let text = resp.into_string().unwrap_or_default();
            Err(anyhow!("{url}: HTTP {code}: {}", clip(text.trim(), 300)))
        }
        Err(e) => Err(anyhow!("{url}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp_tool() -> Value {
        json!({
            "name": "make_video",
            "description": "Create a narrated video.",
            "inputSchema": {
                "type": "object",
                "properties": {"story": {"type": "object"}},
                "required": ["story"]
            }
        })
    }

    #[test]
    fn mcp_tools_become_openai_functions() {
        let tools = to_openai_tools(&[mcp_tool(), json!({"name": "ping"})]);
        assert_eq!(tools.len(), 2);
        assert_eq!(
            tools[0],
            json!({
                "type": "function",
                "function": {
                    "name": "make_video",
                    "description": "Create a narrated video.",
                    "parameters": {
                        "type": "object",
                        "properties": {"story": {"type": "object"}},
                        "required": ["story"]
                    }
                }
            })
        );
        // Missing description and schema: empty text and an empty object schema.
        assert_eq!(tools[1]["function"]["name"], "ping");
        assert_eq!(tools[1]["function"]["description"], "");
        assert_eq!(
            tools[1]["function"]["parameters"],
            json!({"type": "object", "properties": {}})
        );
    }

    #[test]
    fn arguments_parse_or_explain() {
        let ok = parse_arguments(r#"{"story": {"beats": []}, "mode": "check"}"#).unwrap();
        assert_eq!(ok["mode"], "check");
        assert!(parse_arguments("  ").unwrap().is_empty());
        let bad = parse_arguments(r#"{"story": {"beats": [}"#).unwrap_err();
        assert!(bad.starts_with("the arguments are not valid JSON"), "{bad}");
        let array = parse_arguments("[1, 2]").unwrap_err();
        assert!(array.contains("not an array"), "{array}");
        // A JSON object written as a JSON string (double encoding) is bad too.
        let double = parse_arguments(r#""{\"a\": 1}""#).unwrap_err();
        assert!(double.contains("not a string"), "{double}");
    }

    #[test]
    fn replies_with_tool_calls_and_usage() {
        let body = json!({
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {"id": "abc", "type": "function",
                         "function": {"name": "make_video", "arguments": "{\"story\": {}}"}},
                        {"type": "function",
                         "function": {"name": "get_video", "arguments": {"job": "j_0123456789"}}}
                    ]
                }
            }],
            "usage": {"prompt_tokens": 1500, "completion_tokens": 120, "total_tokens": 1620}
        });
        let r = parse_reply(&body, 3).unwrap();
        assert_eq!(r.content, "");
        assert_eq!(r.usage.prompt_tokens, 1500);
        assert_eq!(r.usage.completion_tokens, 120);
        assert_eq!(r.finish_reason.as_deref(), Some("tool_calls"));
        assert_eq!(r.raw_message["tool_calls"][0]["id"], "abc");
        assert_eq!(r.tool_calls.len(), 2);
        assert_eq!(r.tool_calls[0].id, "abc");
        assert_eq!(r.tool_calls[0].arguments, "{\"story\": {}}");
        // No id: made up from the turn and position. Object arguments: serialized.
        assert_eq!(r.tool_calls[1].id, "call_3_1");
        assert_eq!(r.tool_calls[1].arguments, "{\"job\":\"j_0123456789\"}");
        let m = r.assistant_message();
        assert_eq!(m["role"], "assistant");
        assert_eq!(m["tool_calls"][0]["function"]["name"], "make_video");
        assert_eq!(m["tool_calls"][1]["id"], "call_3_1");
    }

    #[test]
    fn plain_answers_and_broken_bodies() {
        let body = json!({
            "choices": [{"message": {"role": "assistant", "content": "Sure, here is a video idea."}}]
        });
        let r = parse_reply(&body, 0).unwrap();
        assert!(r.tool_calls.is_empty());
        assert_eq!(r.content, "Sure, here is a video idea.");
        assert_eq!(r.usage, Usage::default());
        assert_eq!(
            r.assistant_message(),
            json!({"role": "assistant", "content": "Sure, here is a video idea."})
        );
        assert!(parse_reply(&json!({"error": {"message": "no model"}}), 0).is_err());
    }

    #[test]
    fn clip_is_char_safe() {
        assert_eq!(clip("short", 10), "short");
        assert_eq!(clip("äöüäöüäöü", 4), "äöü…");
    }
}
