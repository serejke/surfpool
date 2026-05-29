//! Transparent `/jupiter/*` HTTP proxy.
//!
//! Surfpool's RPC server is a single JSON-RPC POST endpoint at `/`. This
//! module plugs into `jsonrpc-http-server`'s [`RequestMiddleware`] hook to
//! carve out one path prefix — `/jupiter` — and serve it as a
//! protocol-preserving proxy of Jupiter's REST API. Requests that don't
//! start with `/jupiter` fall through untouched to the JSON-RPC handler.
//!
//! Why a transparent proxy rather than the old JSON-RPC façade: a consumer
//! sets `JUPITER_API_URL=<SURFPOOL_URL>/jupiter` and any stock Jupiter
//! client — `@jup-ag/api`, an SDK, curl — works unmodified, because the wire
//! contract beneath `/jupiter` is byte-for-byte Jupiter's. The fork's
//! local-only fixes (local blockhash, pool-snapshot refresh) happen
//! server-side and stay invisible in the response schema.
//!
//! Routing under `/jupiter`:
//!
//! | Incoming                         | Upstream                       | Handling |
//! | -------------------------------- | ------------------------------ | -------- |
//! | `POST /jupiter/swap/v1/swap`     | `POST {base}/swap`             | typed: blockhash rewrite + pool refresh |
//! | anything else (`/quote`, `/price/v3`, …) | `{method} {base_origin}{rest}` | raw byte passthrough |
//!
//! The passthrough is deliberately generic: surfpool doesn't need to model
//! every Jupiter endpoint, only to *not get in the way* of the ones it
//! doesn't rewrite. `base_origin` is the scheme+host of the configured
//! Jupiter base URL, so `/jupiter/price/v3?ids=…` forwards to
//! `https://lite-api.jup.ag/price/v3?ids=…` even though the swap base path
//! is `/swap/v1`.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use jsonrpc_http_server::{
    RequestMiddleware, RequestMiddlewareAction,
    hyper::{Body, Method, Request, Response, StatusCode, body::to_bytes},
};
use solana_commitment_config::CommitmentConfig;
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_core::surfnet::{
    locker::SurfnetSvmLocker, remote::SurfnetRemoteClient, svm::MAX_RECENT_BLOCKHASHES_STANDARD,
};

use crate::{
    client::JupiterHttpClient, config::JupiterConfig, refresh, types::SwapRequest,
};

/// Path prefix this middleware owns. Everything beneath it is treated as the
/// Jupiter REST surface; everything else falls through to JSON-RPC.
const PREFIX: &str = "/jupiter";

/// Middleware that intercepts `/jupiter/*` and proxies it to Jupiter, with
/// surfpool's local-fork fixes applied to the swap path.
pub struct JupiterHttpMiddleware {
    config: Arc<JupiterConfig>,
    client: Arc<JupiterHttpClient>,
    /// Scheme+authority of the Jupiter base URL, e.g. `https://lite-api.jup.ag`.
    /// Used to forward non-swap paths (`/quote`, `/price/v3`) verbatim.
    upstream_origin: String,
    locker: SurfnetSvmLocker,
    remote_client: Option<SurfnetRemoteClient>,
    http: reqwest::Client,
}

impl JupiterHttpMiddleware {
    pub fn new(
        config: JupiterConfig,
        locker: SurfnetSvmLocker,
        remote_client: Option<SurfnetRemoteClient>,
    ) -> Self {
        let client = Arc::new(JupiterHttpClient::new(&config));
        let upstream_origin = origin_of(&config.base_url);
        Self {
            config: Arc::new(config),
            client,
            upstream_origin,
            locker,
            remote_client,
            http: reqwest::Client::new(),
        }
    }
}

impl RequestMiddleware for JupiterHttpMiddleware {
    fn on_request(&self, request: Request<Body>) -> RequestMiddlewareAction {
        let path = request.uri().path();
        if path != PREFIX && !path.starts_with(&format!("{PREFIX}/")) {
            // Not ours — hand back to the JSON-RPC stack untouched.
            return request.into();
        }

        // `/jupiter/health` — cheap capability probe, no upstream call.
        if path == format!("{PREFIX}/health") {
            return health_response(self.config.enabled).into();
        }

        if !self.config.enabled {
            return json_error(
                StatusCode::NOT_FOUND,
                "jupiter extension is disabled in this surfpool instance",
            )
            .into();
        }

        // `rest` is the path with `/jupiter` stripped: `/swap/v1/swap`,
        // `/swap/v1/quote`, `/price/v3`, …
        let rest = path[PREFIX.len()..].to_string();
        let query = request.uri().query().map(|q| q.to_string());

        if request.method() == Method::POST && rest == "/swap/v1/swap" {
            let this = self.clone_for_task();
            return RequestMiddlewareAction::Respond {
                should_validate_hosts: false,
                response: Box::pin(async move { Ok(this.handle_swap(request).await) }),
            };
        }

        // Everything else: opaque byte passthrough to the Jupiter origin.
        let this = self.clone_for_task();
        RequestMiddlewareAction::Respond {
            should_validate_hosts: false,
            response: Box::pin(async move { Ok(this.handle_passthrough(request, rest, query).await) }),
        }
    }
}

/// Cheap clone of just the bits a per-request task needs.
struct TaskCtx {
    client: Arc<JupiterHttpClient>,
    upstream_origin: String,
    api_key: Option<String>,
    locker: SurfnetSvmLocker,
    remote_client: Option<SurfnetRemoteClient>,
    http: reqwest::Client,
}

impl JupiterHttpMiddleware {
    fn clone_for_task(&self) -> TaskCtx {
        TaskCtx {
            client: self.client.clone(),
            upstream_origin: self.upstream_origin.clone(),
            api_key: self.config.api_key.clone(),
            locker: self.locker.clone(),
            remote_client: self.remote_client.clone(),
            http: self.http.clone(),
        }
    }
}

impl TaskCtx {
    /// `POST /jupiter/swap/v1/swap` — the one path that isn't a verbatim
    /// passthrough. Deserialize the Jupiter swap request, force the two
    /// surfpool-mandatory fields, forward to upstream, then rewrite the
    /// returned transaction (refresh writable pool accounts, bump the SVM
    /// clock, restamp the local blockhash, zero signatures). The response
    /// stays schema-identical to Jupiter's `/swap`: `swapTransaction` +
    /// `lastValidBlockHeight`, with surfpool extras tucked into the
    /// flattened `extra` map so stock clients ignore them.
    async fn handle_swap(self, request: Request<Body>) -> Response<Body> {
        let body = match to_bytes(request.into_body()).await {
            Ok(b) => b,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("read body: {e}")),
        };
        let mut swap_req: SwapRequest = match serde_json::from_slice(&body) {
            Ok(r) => r,
            Err(e) => {
                return json_error(StatusCode::BAD_REQUEST, &format!("invalid swap request: {e}"));
            }
        };

        let user_pubkey = match swap_req.user_public_key.parse::<Pubkey>() {
            Ok(pk) => pk,
            Err(e) => {
                return json_error(
                    StatusCode::BAD_REQUEST,
                    &format!("invalid userPublicKey `{}`: {e}", swap_req.user_public_key),
                );
            }
        };

        // Two surfpool-mandatory overrides (see prior JSON-RPC impl): the
        // user's mainnet state is always wrong on a fork, and a 0 priority
        // fee keeps local tx fees deterministic. Every other Jupiter option
        // the caller set is preserved.
        swap_req.skip_user_accounts_rpc_calls = Some(true);
        if swap_req.prioritization_fee_lamports.is_none() {
            swap_req.prioritization_fee_lamports = Some(0);
        }

        let upstream = match self.client.swap(&swap_req).await {
            Ok(r) => r,
            Err(e) => return json_error(StatusCode::BAD_GATEWAY, &e.to_string()),
        };

        let tx_bytes = match BASE64.decode(&upstream.swap_transaction) {
            Ok(b) => b,
            Err(e) => {
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    &format!("base64 decode of swapTransaction failed: {e}"),
                );
            }
        };
        let mut tx: VersionedTransaction = match bincode::deserialize(&tx_bytes) {
            Ok(t) => t,
            Err(e) => {
                return json_error(
                    StatusCode::BAD_GATEWAY,
                    &format!("bincode deserialize of swapTransaction failed: {e}"),
                );
            }
        };

        let remote_ctx = self
            .remote_client
            .as_ref()
            .map(|c| (c.clone(), CommitmentConfig::confirmed()));
        if let Err(e) = refresh::refresh_writable_accounts(
            &self.locker,
            &remote_ctx,
            &tx.message,
            &user_pubkey,
            &[user_pubkey],
        )
        .await
        {
            return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
        }

        let commitment = CommitmentConfig::confirmed();
        let blockhash = self
            .locker
            .get_latest_blockhash(&commitment)
            .unwrap_or_else(|| self.locker.latest_absolute_blockhash());
        let epoch_info = self.locker.get_epoch_info();
        let computed_lvbh = epoch_info.block_height + MAX_RECENT_BLOCKHASHES_STANDARD as u64;

        match &mut tx.message {
            VersionedMessage::Legacy(m) => m.recent_blockhash = blockhash,
            VersionedMessage::V0(m) => m.recent_blockhash = blockhash,
        }
        for sig in tx.signatures.iter_mut() {
            *sig = Signature::default();
        }

        let final_bytes = match bincode::serialize(&tx) {
            Ok(b) => b,
            Err(e) => {
                return json_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    &format!("re-encode swapTransaction failed: {e}"),
                );
            }
        };

        // Build the response as a JSON object identical in shape to
        // Jupiter's /swap, overriding the two fields the fork must own and
        // preserving every other upstream field (incl. unknowns via `extra`).
        let mut payload = upstream;
        payload.swap_transaction = BASE64.encode(&final_bytes);
        payload
            .extra
            .insert("upstreamLastValidBlockHeight".to_string(), payload.last_valid_block_height.into());
        payload.last_valid_block_height = computed_lvbh;

        match serde_json::to_vec(&payload) {
            Ok(bytes) => json_ok(bytes),
            Err(e) => json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("serialize swap response failed: {e}"),
            ),
        }
    }

    /// Opaque passthrough for every non-swap path. Forwards method, body,
    /// query and the `x-api-key` header to `{origin}{rest}` and streams the
    /// upstream status + body straight back. No schema knowledge required.
    async fn handle_passthrough(
        self,
        request: Request<Body>,
        rest: String,
        query: Option<String>,
    ) -> Response<Body> {
        let method = request.method().clone();
        let url = match &query {
            Some(q) => format!("{}{}?{}", self.upstream_origin, rest, q),
            None => format!("{}{}", self.upstream_origin, rest),
        };

        let body = match to_bytes(request.into_body()).await {
            Ok(b) => b,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &format!("read body: {e}")),
        };

        let reqwest_method = match reqwest::Method::from_bytes(method.as_str().as_bytes()) {
            Ok(m) => m,
            Err(e) => {
                return json_error(StatusCode::BAD_REQUEST, &format!("bad method: {e}"));
            }
        };
        let mut builder = self.http.request(reqwest_method, &url);
        if !body.is_empty() {
            builder = builder
                .body(body.to_vec())
                .header("content-type", "application/json");
        }
        if let Some(key) = self.api_key.as_deref() {
            builder = builder.header("x-api-key", key);
        }

        match builder.send().await {
            Ok(resp) => {
                let status = resp.status();
                let bytes = resp.bytes().await.unwrap_or_default();
                Response::builder()
                    .status(StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY))
                    .header("content-type", "application/json; charset=utf-8")
                    .body(Body::from(bytes.to_vec()))
                    .unwrap()
            }
            Err(e) => json_error(StatusCode::BAD_GATEWAY, &format!("upstream request failed: {e}")),
        }
    }
}

/// Extract scheme+authority from a base URL, dropping any path.
/// `https://lite-api.jup.ag/swap/v1` -> `https://lite-api.jup.ag`.
fn origin_of(base_url: &str) -> String {
    // Find the third `/` (after `scheme://`). Everything before it is origin.
    if let Some(scheme_end) = base_url.find("://") {
        let after = scheme_end + 3;
        match base_url[after..].find('/') {
            Some(path_start) => base_url[..after + path_start].to_string(),
            None => base_url.trim_end_matches('/').to_string(),
        }
    } else {
        base_url.trim_end_matches('/').to_string()
    }
}

fn json_ok(bytes: Vec<u8>) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json; charset=utf-8")
        .body(Body::from(bytes))
        .unwrap()
}

fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({ "error": message }).to_string();
    Response::builder()
        .status(status)
        .header("content-type", "application/json; charset=utf-8")
        .body(Body::from(body))
        .unwrap()
}

fn health_response(enabled: bool) -> Response<Body> {
    let body = serde_json::json!({ "jupiter": enabled }).to_string();
    json_ok(body.into_bytes())
}
