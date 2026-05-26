//! Typed HTTP wrapper around Jupiter's REST API.

use std::time::Duration;

use reqwest::Client;

use crate::{
    config::JupiterConfig,
    error::{JupiterError, JupiterResult},
    types::{QuoteRequest, QuoteResponse, SwapRequest, SwapResponse, SwapMode},
};

/// Thin HTTP wrapper that turns Jupiter's REST contract into typed calls.
///
/// The client is stateless beyond the underlying reqwest pool, so it's cheap
/// to clone and share across the RPC handler thread pool.
#[derive(Clone, Debug)]
pub struct JupiterHttpClient {
    http: Client,
    base_url: String,
    api_key: Option<String>,
}

impl JupiterHttpClient {
    pub fn new(config: &JupiterConfig) -> Self {
        // Jupiter routinely takes 1–3s to return a quote; 30s is a generous
        // ceiling that absorbs the occasional spike without holding sockets
        // forever.
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("reqwest client init");
        Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key: config.api_key.clone(),
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// `GET /quote` — returns a Jupiter route quote.
    pub async fn quote(&self, params: &QuoteRequest) -> JupiterResult<QuoteResponse> {
        let url = format!("{}/quote", self.base_url);
        let query = quote_query_params(params);
        let mut request = self.http.get(&url).query(&query);
        if let Some(api_key) = self.api_key.as_deref() {
            request = request.header("x-api-key", api_key);
        }

        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(JupiterError::Upstream {
                status: status.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// `POST /swap` — converts a quote into a built v0 transaction.
    pub async fn swap(&self, request: &SwapRequest) -> JupiterResult<SwapResponse> {
        let url = format!("{}/swap", self.base_url);
        let mut builder = self.http.post(&url).json(request);
        if let Some(api_key) = self.api_key.as_deref() {
            builder = builder.header("x-api-key", api_key);
        }
        let response = builder.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(JupiterError::Upstream {
                status: status.as_u16(),
                body,
            });
        }
        Ok(serde_json::from_str(&body)?)
    }
}

/// Flatten [`QuoteRequest`] into the query string Jupiter expects.
///
/// `reqwest::RequestBuilder::query(&Q)` requires `Q: Serialize`, but our
/// struct contains `Option`s and nested vectors that don't all encode cleanly
/// as query params (`dexes` arrives as a comma-joined string upstream). This
/// helper hand-rolls the encoding to stay loyal to Jupiter's URL contract.
fn quote_query_params(params: &QuoteRequest) -> Vec<(&'static str, String)> {
    let mut out: Vec<(&'static str, String)> = vec![
        ("inputMint", params.input_mint.clone()),
        ("outputMint", params.output_mint.clone()),
        ("amount", params.amount.to_string()),
    ];

    if let Some(v) = params.slippage_bps {
        out.push(("slippageBps", v.to_string()));
    }
    if let Some(v) = params.swap_mode {
        out.push((
            "swapMode",
            match v {
                SwapMode::ExactIn => "ExactIn",
                SwapMode::ExactOut => "ExactOut",
            }
            .to_string(),
        ));
    }
    if let Some(v) = params.dexes.as_ref() {
        out.push(("dexes", v.join(",")));
    }
    if let Some(v) = params.exclude_dexes.as_ref() {
        out.push(("excludeDexes", v.join(",")));
    }
    if let Some(v) = params.restrict_intermediate_tokens {
        out.push(("restrictIntermediateTokens", v.to_string()));
    }
    if let Some(v) = params.only_direct_routes {
        out.push(("onlyDirectRoutes", v.to_string()));
    }
    if let Some(v) = params.as_legacy_transaction {
        out.push(("asLegacyTransaction", v.to_string()));
    }
    if let Some(v) = params.platform_fee_bps {
        out.push(("platformFeeBps", v.to_string()));
    }
    if let Some(v) = params.max_accounts {
        out.push(("maxAccounts", v.to_string()));
    }
    if let Some(v) = params.auto_slippage {
        out.push(("autoSlippage", v.to_string()));
    }
    if let Some(v) = params.max_auto_slippage_bps {
        out.push(("maxAutoSlippageBps", v.to_string()));
    }
    if let Some(v) = params.auto_slippage_collision_usd_value {
        out.push(("autoSlippageCollisionUsdValue", v.to_string()));
    }
    if let Some(v) = params.minimize_slippage {
        out.push(("minimizeSlippage", v.to_string()));
    }
    // `extra` is intentionally ignored on the query path — unknown fields are
    // accepted on the request struct for round-tripping but Jupiter rejects
    // arbitrary query params anyway.
    let _ = &params.extra;
    out
}
