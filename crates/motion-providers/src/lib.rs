//! (P1) Asset providers: the engine spends the user's generation credits.
//!
//! [`AssetProvider`] is the engine-side verb set (catalog, quote, submit,
//! poll, download). [`McpProvider`] implements it over any streamable-HTTP MCP
//! server through a small [`Adapter`] that maps verbs to tool names discovered
//! with `tools/list`; [`MossProvider`] (v2.3) over a local MOSS
//! image server (0 credits, the last resort of the image fallback). Spending
//! is gated by [`Policy`] (model allow-list, caps, a quote before every
//! spend), deduplicated by [`AssetCache`] and recorded in the [`SpendLog`].
//! Server text is data: nothing in a response is executed.

pub mod cache;
pub mod cutout;
pub mod jsonx;
pub mod lockfile;
pub mod mcp;
pub mod moss;
pub mod oauth;
pub mod pipeline;
pub mod policy;
pub mod provider;
pub mod store;
pub mod transport;

pub use cache::AssetCache;
pub use lockfile::{LockAction, LockEntry, Lockfile};
pub use mcp::{McpClient, Tool, ToolResult, Unauthorized};
pub use moss::{MossProvider, Unavailable};
pub use pipeline::{
    generate, generate_image_with_fallback, generate_keyed, ImageSpec, Outcome, PollOpts, Resolved,
};
pub use policy::{Policy, SpendEntry, SpendLog};
pub use provider::{
    Adapter, AssetKind, AssetProvider, AssetRequest, Job, JobState, McpProvider, ModelInfo, Quote,
};
pub use store::{Auth, Connection, Connections};
pub use transport::{HttpResponse, Transport, UreqTransport};
