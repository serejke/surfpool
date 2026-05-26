//! Opt-in Jupiter aggregator proxy for surfpool.
//!
//! The crate fronts Jupiter's REST API (`/quote`, `/swap`) over JSON-RPC and
//! transparently fixes the two local-fork gotchas every client otherwise
//! re-implements:
//!
//! 1. Jupiter stamps the transaction with a mainnet blockhash that the local
//!    fork doesn't recognize.
//! 2. Lazy-fetched DEX pool snapshots can be stale on first hit and revert
//!    with `Custom error 5` (`1027565`).
//!
//! The crate is layered so each piece can be consumed independently:
//!
//! - [`config`] — runtime configuration (`JupiterConfig`).
//! - [`types`] — wire types mirroring Jupiter's REST request/response schemas.
//! - [`client`] — typed HTTP wrapper (`JupiterClient`).
//! - [`refresh`] — surfpool-specific pool refresh helper.
//! - [`rpc`] — the JSON-RPC façade plus the `Jupiter` trait.
//!
//! The runloop in [`surfpool-core`] doesn't depend on this crate; the CLI
//! wires the extension in via the `extensions` parameter of
//! `start_local_surfnet` so disabling the feature is a pure no-op.

pub mod client;
pub mod config;
pub mod error;
pub mod refresh;
pub mod rpc;
pub mod types;

pub use config::{
    DEFAULT_JUPITER_LITE_BASE_URL, DEFAULT_JUPITER_PRO_BASE_URL, JupiterConfig,
};
pub use error::{JupiterError, JupiterResult};
pub use rpc::{Jupiter, JupiterRpc, register_extension};
