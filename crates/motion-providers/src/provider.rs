//! The engine-side provider verbs and the generic MCP implementation.

use std::collections::BTreeMap;

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::jsonx::{find_number, find_string, first_object_array, media_urls};
use crate::mcp::{snippet, McpClient, Tool, Unauthorized};
use crate::oauth;
use crate::store::Connection;
use crate::transport::Transport;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Image,
    Video,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssetRequest {
    pub kind: AssetKind,
    pub model: String,
    pub prompt: String,
    #[serde(default)]
    pub quality: Option<String>,
    #[serde(default)]
    pub aspect: Option<String>,
    /// Extra tool arguments passed through verbatim (sorted, so cache keys are stable).
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Quote {
    pub provider: String,
    pub model: String,
    pub credits: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: String,
    /// Media URLs already in the submit reply (synchronous servers).
    pub urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobState {
    Pending,
    Done(Vec<String>),
    Failed(String),
}

pub trait AssetProvider {
    fn name(&self) -> &str;
    fn catalog(&mut self) -> Result<Vec<ModelInfo>>;
    /// Remaining credits, when the platform reports them.
    fn balance(&mut self) -> Result<Option<f64>>;
    fn quote(&mut self, req: &AssetRequest) -> Result<Quote>;
    fn submit(&mut self, req: &AssetRequest) -> Result<Job>;
    fn poll(&mut self, job: &Job) -> Result<JobState>;
    fn download(&mut self, url: &str) -> Result<Vec<u8>>;
}

/// Maps engine verbs to a server's tools (matched by exact name, then by
/// `_<pattern>` suffix, against `tools/list`) and engine fields to argument
/// names (the first candidate the tool's input schema declares).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Adapter {
    pub name: String,
    pub url: String,
    pub catalog: Vec<String>,
    pub balance: Vec<String>,
    pub quote: Vec<String>,
    pub image: Vec<String>,
    pub video: Vec<String>,
    pub status: Vec<String>,
    pub model_field: Vec<String>,
    pub prompt_field: Vec<String>,
    pub quality_field: Vec<String>,
    pub aspect_field: Vec<String>,
    pub job_field: Vec<String>,
    /// Mode argument (`text2image` …), sent when the tool declares it.
    pub mode_field: String,
    pub image_mode: String,
    pub video_mode: String,
    /// Object argument the form values (prompt, quality, aspect, `--param`) nest
    /// under, when the tool declares it; else they are top-level arguments.
    pub params_field: String,
}

fn strs(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

impl Adapter {
    /// Any MCP server with conventional tool names.
    pub fn generic(name: &str, url: &str) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            catalog: strs(&["model_list", "list_models", "models"]),
            balance: strs(&["account_get", "get_balance", "balance", "account"]),
            quote: strs(&["model_cost", "get_cost", "estimate_cost", "quote", "cost"]),
            image: strs(&["generate_image", "image_generate", "create_image"]),
            video: strs(&["generate_video", "video_generate", "create_video"]),
            status: strs(&[
                "creation_get",
                "get_generation",
                "generation_status",
                "job_status",
                "get_status",
            ]),
            model_field: strs(&["model", "model_id", "modelId", "model_name"]),
            prompt_field: strs(&["prompt", "text", "positive_prompt"]),
            quality_field: strs(&["quality"]),
            aspect_field: strs(&["aspectRatio", "aspect_ratio", "aspect", "ratio"]),
            job_field: strs(&[
                "historyId",
                "history_id",
                "creation_id",
                "creationId",
                "job_id",
                "task_id",
                "generation_id",
                "id",
            ]),
            mode_field: "mode".into(),
            image_mode: "text2image".into(),
            video_mode: "text2video".into(),
            params_field: "params".into(),
        }
    }

    pub fn openart() -> Self {
        Self::generic("openart", "https://mcp.openart.ai/mcp")
    }

    pub fn for_name(name: &str) -> Option<Self> {
        match name {
            "openart" => Some(Self::openart()),
            _ => None,
        }
    }

    pub fn resolve<'a>(&self, tools: &'a [Tool], patterns: &[String]) -> Option<&'a Tool> {
        patterns
            .iter()
            .find_map(|p| tools.iter().find(|t| &t.name == p))
            .or_else(|| {
                patterns
                    .iter()
                    .find_map(|p| tools.iter().find(|t| t.name.ends_with(&format!("_{p}"))))
            })
    }
}

/// Argument name for an engine field: the first candidate the schema declares,
/// else (schema without properties) the first candidate; `None` if the schema
/// declares properties but none of the candidates.
fn field<'a>(tool: &Tool, candidates: &'a [String]) -> Option<&'a str> {
    match tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
    {
        Some(props) if !props.is_empty() => candidates
            .iter()
            .find(|c| props.contains_key(c.as_str()))
            .map(String::as_str),
        _ => candidates.first().map(String::as_str),
    }
}

const CREDIT_KEYS: &[&str] = &[
    "totalCredits",
    "total_credits_cost",
    "credits",
    "cost",
    "credit_cost",
    "creditCost",
    "total_credits",
    "credits_required",
    "estimated_credits",
    "price",
];
const BALANCE_KEYS: &[&str] = &[
    "balance",
    "credit_balance",
    "creditBalance",
    "remaining_credits",
    "available_credits",
    "credits",
];
const PENDING: &[&str] = &[
    "pending",
    "queued",
    "queue",
    "processing",
    "running",
    "in_progress",
    "inprogress",
    "generating",
    "submitted",
    "starting",
    "created",
];
const FAILED: &[&str] = &[
    "failed",
    "failure",
    "error",
    "cancelled",
    "canceled",
    "rejected",
];

pub struct McpProvider<'t> {
    pub adapter: Adapter,
    pub conn: Connection,
    transport: &'t dyn Transport,
    client: McpClient<'t>,
    tools: Option<Vec<Tool>>,
    /// Set when tokens were refreshed; the caller should save `conn`.
    pub auth_changed: bool,
}

impl<'t> McpProvider<'t> {
    pub fn new(adapter: Adapter, conn: Connection, transport: &'t dyn Transport) -> Self {
        let client = McpClient::new(&conn.url, transport, Some(conn.auth.access_token.clone()));
        Self {
            adapter,
            conn,
            transport,
            client,
            tools: None,
            auth_changed: false,
        }
    }

    fn refresh(&mut self) -> Result<bool> {
        match oauth::refresh(self.transport, &self.conn.auth, &self.conn.url)? {
            Some(a) => {
                self.client.set_token(Some(a.access_token.clone()));
                self.conn.auth = a;
                self.auth_changed = true;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn with_auth<T>(&mut self, f: impl Fn(&mut McpClient<'t>) -> Result<T>) -> Result<T> {
        if oauth::expiring(&self.conn.auth) {
            self.refresh()?;
        }
        match f(&mut self.client) {
            Err(e) if e.is::<Unauthorized>() && self.refresh()? => f(&mut self.client),
            r => r,
        }
    }

    pub fn tools(&mut self) -> Result<&[Tool]> {
        if self.tools.is_none() {
            let t = self.with_auth(|c| c.list_tools())?;
            self.tools = Some(t);
        }
        Ok(self.tools.as_deref().unwrap_or_default())
    }

    fn tool(&mut self, patterns: &[String], verb: &str) -> Result<Tool> {
        let adapter = self.adapter.clone();
        let tools = self.tools()?;
        adapter.resolve(tools, patterns).cloned().ok_or_else(|| {
            let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            anyhow!(
                "{}: no tool for '{verb}' (looked for {patterns:?}; server has {names:?})",
                adapter.name
            )
        })
    }

    fn call(&mut self, tool: &Tool, args: Value) -> Result<Value> {
        let name = tool.name.clone();
        let r = self.with_auth(|c| c.call_tool(&name, args.clone()))?;
        if r.is_error {
            bail!("{name} failed: {}", snippet(&r.text()));
        }
        Ok(r.json())
    }

    /// Call a tool by exact name (read-only inspection; never routed around the policy
    /// for generation: names containing `generate` are refused).
    pub fn call_named(&mut self, name: &str, args: Value) -> Result<Value> {
        if name.contains("generate") {
            bail!("'{name}' spends credits; use `providers generate`");
        }
        let t = self
            .tools()?
            .iter()
            .find(|t| t.name == name)
            .cloned()
            .ok_or_else(|| anyhow!("no tool '{name}'"))?;
        self.call(&t, args)
    }

    /// Generation arguments for `tool` from the request.
    fn args(&self, tool: &Tool, req: &AssetRequest) -> Value {
        let a = &self.adapter;
        let declares = |k: &str| {
            tool.input_schema
                .get("properties")
                .and_then(|p| p.get(k))
                .is_some()
        };
        let mut top = Map::new();
        if let Some(k) = field(tool, &a.model_field) {
            top.insert(k.into(), Value::String(req.model.clone()));
        }
        if declares(&a.mode_field) {
            let mode = match req.kind {
                AssetKind::Image => &a.image_mode,
                AssetKind::Video => &a.video_mode,
            };
            top.insert(a.mode_field.clone(), Value::String(mode.clone()));
        }
        let nested = declares(&a.params_field);
        let mut form = Map::new();
        for (cands, v) in [
            (&a.prompt_field, Some(req.prompt.as_str())),
            (&a.quality_field, req.quality.as_deref()),
            (&a.aspect_field, req.aspect.as_deref()),
        ] {
            // Nested form fields are not in the tool schema: use the first candidate.
            let k = if nested {
                cands.first().map(String::as_str)
            } else {
                field(tool, cands)
            };
            if let (Some(k), Some(v)) = (k, v) {
                form.insert(k.into(), Value::String(v.into()));
            }
        }
        for (k, v) in &req.params {
            form.insert(k.clone(), v.clone());
        }
        if nested {
            top.insert(a.params_field.clone(), Value::Object(form));
        } else {
            top.extend(form);
        }
        Value::Object(top)
    }

    fn verb_tool(&mut self, kind: AssetKind) -> Result<Tool> {
        match kind {
            AssetKind::Image => self.tool(&self.adapter.image.clone(), "generate image"),
            AssetKind::Video => self.tool(&self.adapter.video.clone(), "generate video"),
        }
    }
}

impl AssetProvider for McpProvider<'_> {
    fn name(&self) -> &str {
        &self.adapter.name
    }

    fn catalog(&mut self) -> Result<Vec<ModelInfo>> {
        let t = self.tool(&self.adapter.catalog.clone(), "catalog")?;
        let v = self.call(&t, Value::Object(Map::new()))?;
        let s = |o: &Value, keys: &[&str]| {
            keys.iter()
                .find_map(|k| o.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string()
        };
        Ok(first_object_array(&v)
            .map(|a| {
                a.iter()
                    .filter(|o| o.is_object())
                    .map(|o| ModelInfo {
                        id: s(o, &["id", "model_id", "modelId", "slug", "model"]),
                        name: s(o, &["name", "display_name", "displayName", "title"]),
                        kind: s(o, &["type", "kind", "category", "media_type"]),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    fn balance(&mut self) -> Result<Option<f64>> {
        let t = self.tool(&self.adapter.balance.clone(), "balance")?;
        let v = self.call(&t, Value::Object(Map::new()))?;
        Ok(find_number(&v, BALANCE_KEYS))
    }

    fn quote(&mut self, req: &AssetRequest) -> Result<Quote> {
        let t = self.tool(&self.adapter.quote.clone(), "quote")?;
        let v = self.call(&t, self.args(&t, req))?;
        let credits = find_number(&v, CREDIT_KEYS).ok_or_else(|| {
            anyhow!(
                "quote reply has no credit figure: {}",
                snippet(&v.to_string())
            )
        })?;
        Ok(Quote {
            provider: self.adapter.name.clone(),
            model: req.model.clone(),
            credits,
        })
    }

    fn submit(&mut self, req: &AssetRequest) -> Result<Job> {
        let t = self.verb_tool(req.kind)?;
        let v = self.call(&t, self.args(&t, req))?;
        let id = find_string(
            &v,
            &self
                .adapter
                .job_field
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default();
        let urls = media_urls(&v);
        if id.is_empty() && urls.is_empty() {
            bail!(
                "submit reply has neither a job id nor media: {}",
                snippet(&v.to_string())
            );
        }
        Ok(Job { id, urls })
    }

    fn poll(&mut self, job: &Job) -> Result<JobState> {
        if !job.urls.is_empty() {
            return Ok(JobState::Done(job.urls.clone()));
        }
        let t = self.tool(&self.adapter.status.clone(), "status")?;
        let key = field(&t, &self.adapter.job_field)
            .unwrap_or("id")
            .to_string();
        let mut m = Map::new();
        m.insert(key, Value::String(job.id.clone()));
        let v = self.call(&t, Value::Object(m))?;
        let status = find_string(&v, &["status", "state"])
            .unwrap_or_default()
            .to_ascii_lowercase();
        if FAILED.iter().any(|f| status.contains(f)) {
            return Ok(JobState::Failed(snippet(&v.to_string())));
        }
        let urls = media_urls(&v);
        if !urls.is_empty() && !PENDING.contains(&status.as_str()) {
            return Ok(JobState::Done(urls));
        }
        Ok(JobState::Pending)
    }

    fn download(&mut self, url: &str) -> Result<Vec<u8>> {
        if !url.starts_with("https://") {
            bail!("refusing non-https download");
        }
        let r = self.transport.get(url, &[])?;
        if r.status != 200 || r.body.is_empty() {
            bail!("download failed: HTTP {}", r.status);
        }
        Ok(r.body)
    }
}
