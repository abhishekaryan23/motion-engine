//! (1b) The `rmcp` stdio server: `tools/list` from [`crate::schema`] for the
//! profile, `tools/call` into [`crate::service`], progress notifications,
//! cancellation, resources and prompts from [`crate::resources`].
//!
//! The handler is written by hand (not with the `#[tool]` macros) because the
//! tool list and every input schema depend on the profile chosen at start.

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, Implementation, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, Prompt, PromptArgument, PromptMessage, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ResourceTemplate, Role,
    ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value};

use crate::profile;
use crate::reply::{Progress, ToolOutput};
use crate::resources;
use crate::schema;
use crate::service::{CallCtx, Service};
use crate::store::resolve_path;

/// Server name in `initialize`.
pub const SERVER_NAME: &str = "motionengine";
/// The one-sentence `instructions`.
pub const INSTRUCTIONS: &str =
    "Make a video with make_video; guide: resource motionengine://guide/lite-story";

/// The MCP face of a [`Service`].
#[derive(Clone)]
pub struct McpServer {
    service: Arc<Service>,
}

impl McpServer {
    pub fn new(service: Arc<Service>) -> Self {
        McpServer { service }
    }

    pub fn service(&self) -> &Arc<Service> {
        &self.service
    }

    /// The profile's tool list as `rmcp` tools (input schema = the profile's
    /// generated JSON schema; no output schema).
    pub fn tools(&self) -> Vec<Tool> {
        let config = self.service.config();
        schema::tool_defs(config.profile, config.allow_scene_edits)
            .into_iter()
            .map(|d| {
                let schema = match d.input_schema {
                    Value::Object(map) => map,
                    _ => Map::new(),
                };
                Tool::new(d.name, d.description, Arc::new(schema))
            })
            .collect()
    }

    /// A [`ToolOutput`] as an MCP tool result.
    pub fn to_result(&self, out: ToolOutput) -> CallToolResult {
        let mut content = vec![ContentBlock::text(out.text)];
        for image in &out.images {
            match std::fs::read(&image.path) {
                Ok(bytes) => content.push(ContentBlock::image(base64(&bytes), image.mime)),
                Err(e) => content.push(ContentBlock::text(format!(
                    "(image {} unreadable: {e})",
                    image.path.display()
                ))),
            }
        }
        for link in &out.links {
            let path = resolve_path(self.service.config(), link);
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| link.clone());
            let mut resource = Resource::new(file_uri(&path), name);
            if let Some(mime) = mime_of(&path) {
                resource = resource.with_mime_type(mime);
            }
            content.push(ContentBlock::resource_link(resource));
        }
        let mut result = if out.is_error {
            CallToolResult::error(content)
        } else {
            CallToolResult::success(content)
        };
        result.structured_content = Some(out.structured);
        result
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> rmcp::model::ServerConfig {
        let caps = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_prompts()
            .build();
        let implementation = Implementation::new(SERVER_NAME, env!("CARGO_PKG_VERSION"))
            .with_title("MotionEngine")
            .with_description(format!(
                "MotionEngine {} ({} profile)",
                profile::build_version(),
                self.service.profile().name()
            ));
        rmcp::model::ServerConfig::new(caps)
            .with_server_info(implementation)
            .with_instructions(INSTRUCTIONS)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        Ok(ListToolsResult::with_all_items(self.tools()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        let token = context.meta.get_progress_token();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Progress>();
        let mut ctx = CallCtx::new();
        if token.is_some() {
            ctx = ctx.with_progress(move |p| {
                let _ = tx.send(p);
            });
        }
        let cancel = ctx.cancel_flag();
        let service = Arc::clone(&self.service);
        let tool = name.clone();
        let mut task = tokio::task::spawn_blocking(move || service.call(&tool, args, &ctx));
        let peer = context.peer.clone();
        let notify = |p: Progress| {
            let peer = peer.clone();
            let token = token.clone();
            async move {
                if let Some(token) = token {
                    let param = ProgressNotificationParam::new(token, f64::from(p.percent))
                        .with_total(100.0)
                        .with_message(p.stage);
                    let _ = peer.notify_progress(param).await;
                }
            }
        };
        let mut cancelled = false;
        let joined = loop {
            tokio::select! {
                joined = &mut task => break joined,
                Some(p) = rx.recv() => notify(p).await,
                _ = context.ct.cancelled(), if !cancelled => {
                    cancelled = true;
                    cancel.store(true, Ordering::SeqCst);
                    self.service.wake();
                    eprintln!("motion-mcp: {name} cancelled by the client");
                }
            }
        };
        while let Ok(p) = rx.try_recv() {
            notify(p).await;
        }
        let out = match joined {
            Ok(out) => out,
            Err(e) => {
                eprintln!("motion-mcp: {name} crashed: {e}");
                return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                    "internal error in {name}; the server is still running"
                ))])
                .into());
            }
        };
        Ok(self.to_result(out).into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let items = resources::list(self.service.profile())
            .into_iter()
            .map(|d| {
                Resource::new(d.uri, d.name)
                    .with_description(d.description)
                    .with_mime_type(d.mime)
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(items))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, ErrorData> {
        let items = resources::templates(self.service.profile())
            .into_iter()
            .map(|d| {
                ResourceTemplate::new(d.uri, d.name)
                    .with_description(d.description)
                    .with_mime_type(d.mime)
            })
            .collect();
        Ok(ListResourceTemplatesResult::with_all_items(items))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let uri = request.uri;
        let service = Arc::clone(&self.service);
        let wanted = uri.clone();
        let text =
            tokio::task::spawn_blocking(move || resources::read(&wanted, &service.resource_ctx()))
                .await
                .map_err(|e| ErrorData::internal_error(format!("reading {uri}: {e}"), None))?;
        match text {
            Some(text) => {
                let mime = resource_mime(self.service.profile(), &uri);
                let contents = ResourceContents::text(text, uri).with_mime_type(mime);
                Ok(ReadResourceResult::new(vec![contents]).into())
            }
            None => Err(ErrorData::resource_not_found(
                format!("unknown resource {uri}"),
                None,
            )),
        }
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let items = resources::prompts(self.service.profile())
            .into_iter()
            .map(|p| {
                let args = p
                    .arguments
                    .iter()
                    .map(|a| {
                        PromptArgument::new(a.name)
                            .with_description(a.description)
                            .with_required(a.required)
                    })
                    .collect();
                Prompt::new(p.name, Some(p.description), Some(args))
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(items))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let profile = self.service.profile();
        let args = request.arguments.unwrap_or_default();
        match resources::get_prompt(profile, &request.name, &args) {
            Some(text) => {
                let description = resources::prompts(profile)
                    .into_iter()
                    .find(|p| p.name == request.name)
                    .map(|p| p.description);
                let mut result =
                    GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]);
                if let Some(d) = description {
                    result = result.with_description(d);
                }
                Ok(result.into())
            }
            None => Err(ErrorData::invalid_params(
                format!("unknown prompt '{}'", request.name),
                None,
            )),
        }
    }
}

/// The MIME type of a resource URI: its definition's, else plain text.
fn resource_mime(profile: profile::Profile, uri: &str) -> &'static str {
    let fixed = resources::list(profile).into_iter().find(|d| d.uri == uri);
    let templated = || {
        resources::templates(profile).into_iter().find(|d| {
            let prefix = d.uri.split('{').next().unwrap_or(&d.uri);
            !prefix.is_empty() && uri.starts_with(prefix)
        })
    };
    fixed
        .or_else(templated)
        .map(|d| d.mime)
        .unwrap_or("text/plain")
}

fn mime_of(path: &Path) -> Option<&'static str> {
    match path.extension()?.to_str()? {
        "mp4" => Some("video/mp4"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "json" => Some("application/json"),
        _ => None,
    }
}

/// `file://` URI of an absolute path (percent-encoding everything outside the
/// unreserved set and `/`).
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.' | b'~') {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextet = |shift: u32| ALPHABET[((n >> shift) & 63) as usize] as char;
        out.push(sextet(18));
        out.push(sextet(12));
        out.push(if chunk.len() > 1 { sextet(6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(0) } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_uris() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe]), "//4=");
        assert_eq!(
            file_uri(Path::new("/a b/j_1/video.mp4")),
            "file:///a%20b/j_1/video.mp4"
        );
    }
}
