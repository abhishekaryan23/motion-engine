//! The MCP server: tools, policy, sandbox, rendering.
//! One test binary per area (fewer links, fewer first-launch scans); each
//! file is a module. Run one: `cargo test -p motion-mcp --test mcp <module>::`.

#[path = "../common/mod.rs"]
mod common;
mod e2e_render;
mod lite_golden;
mod music_word;
mod pictures;
mod policy;
mod resources;
mod sandbox;
mod server_stdio;
mod takes;
mod tool_budget;
mod v2;
