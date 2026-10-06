//! MotionEngine as MCP tools (Sprint 0.21, `docs/plans/SPRINT_0_21_MCP_SERVER.md`).
//!
//! Fat tools, thin schemas: a model writes one small story object and gets back
//! one short answer; the engine does the multi-step work. Three profiles
//! ([`profile::Profile`]) share one job store, one validator and one engine;
//! control grows with model strength, the contract never does (story in, no
//! pixel or timing fields, the same QA).
//!
//! Module map (who owns what in Phase 1):
//! * frozen contracts (1a): [`profile`], [`schema`], [`args`], [`lite`],
//!   [`reply`], [`job`], [`policy`] (limits, `Prepared`, entry points);
//! * 1b server: `main.rs`, [`server`], [`service`], [`engine`], [`store`];
//! * 1c story checks: the bodies in [`policy`], [`pictures`], [`byo`];
//! * 1d guidance: [`resources`];
//! * 1f creator tools: [`creator`].
//!
//! The server makes no network calls: voice synthesis happens inside the
//! `motion-engine reel` subprocess (`motion-voice`).

pub mod args;
pub mod byo;
pub mod creator;
pub mod engine;
pub mod job;
pub mod lite;
pub mod pictures;
pub mod policy;
pub mod profile;
pub mod reply;
pub mod resources;
pub mod schema;
pub mod server;
pub mod service;
pub mod store;

pub use profile::{Profile, ServerConfig};
pub use reply::{Fix, Reply, Status, ToolOutput};
