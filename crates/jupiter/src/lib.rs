//! Opt-in Jupiter aggregator proxy for surfpool.
//!
//! The crate fronts Jupiter's REST API as a transparent path-prefixed proxy
//! at `<SURFPOOL_URL>/jupiter/*`. A consumer points any stock Jupiter client
//! at that base URL and it works unmodified — the wire contract beneath
//! `/jupiter` is byte-for-byte Jupiter's. Two local-fork gotchas are fixed
//! server-side, invisibly:
//!
//! 1. Jupiter stamps the transaction with a mainnet blockhash that the local
//!    fork doesn't recognize — the swap handler restamps the local one.
//! 2. Lazy-fetched DEX pool snapshots can be stale on first hit and revert
//!    with `Custom error 5` (`1027565`) — the swap handler purges the
//!    route's writable pool accounts so they re-fetch fresh.
//!
//! The crate is layered so each piece can be consumed independently:
//!
//! - [`config`] — runtime configuration (`JupiterConfig`).
//! - [`types`] — wire types mirroring Jupiter's REST request/response schemas.
//! - [`client`] — typed HTTP wrapper (`JupiterClient`).
//! - [`refresh`] — surfpool-specific pool refresh helper.
//! - [`http`] — the transparent `/jupiter/*` HTTP proxy middleware.
//!
//! The runloop in [`surfpool-core`] doesn't depend on this crate; the CLI
//! supplies the middleware factory via `start_local_surfnet`'s
//! `http_middleware` parameter, so disabling the feature is a pure no-op.

pub mod client;
pub mod config;
pub mod error;
pub mod http;
pub mod refresh;
pub mod types;

use std::sync::Arc;

use surfpool_core::surfnet::{locker::SurfnetSvmLocker, remote::SurfnetRemoteClient};

pub use config::{
    DEFAULT_JUPITER_LITE_BASE_URL, DEFAULT_JUPITER_PRO_BASE_URL, JupiterConfig,
};
pub use error::{JupiterError, JupiterResult};
pub use http::JupiterHttpMiddleware;

/// Build the `/jupiter/*` HTTP middleware for the given config, bound to the
/// live SVM state. Returned as the boxed trait object surfpool-core's HTTP
/// server accepts via its `RequestMiddleware` hook. The CLI calls this once
/// at startup when `--enable-jupiter` is set.
pub fn make_http_middleware(
    config: JupiterConfig,
    locker: SurfnetSvmLocker,
    remote_client: Option<SurfnetRemoteClient>,
) -> Arc<dyn jsonrpc_http_server::RequestMiddleware> {
    Arc::new(JupiterHttpMiddleware::new(config, locker, remote_client))
}
