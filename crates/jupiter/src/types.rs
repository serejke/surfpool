//! Wire types mirroring Jupiter's REST API.
//!
//! Field names and shapes intentionally track Jupiter's published JSON so
//! existing Jupiter clients can be pointed at surfpool with the same payloads.
//! Forward-compatibility is built in via `#[serde(flatten)] extra`: unknown
//! fields from newer Jupiter releases pass through untouched on both
//! quote/swap requests and responses.
//!
//! Reference: <https://station.jup.ag/docs/swap-api/swap>.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SwapMode {
    ExactIn,
    ExactOut,
}

impl Default for SwapMode {
    fn default() -> Self {
        SwapMode::ExactIn
    }
}

/// Parameters for `jupiter_quote` — mirrors the query string of Jupiter's
/// `GET /quote` endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteRequest {
    pub input_mint: String,
    pub output_mint: String,
    /// Atomic units of the input mint when [`SwapMode::ExactIn`],
    /// otherwise atomic units of the output mint.
    pub amount: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slippage_bps: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swap_mode: Option<SwapMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dexes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_dexes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restrict_intermediate_tokens: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_direct_routes: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_legacy_transaction: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_fee_bps: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_accounts: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_slippage: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_auto_slippage_bps: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_slippage_collision_usd_value: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimize_slippage: Option<bool>,
    /// Forwards unrecognized fields verbatim to upstream so newer Jupiter
    /// releases stay usable without a crate bump.
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// One leg of a Jupiter route.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutePlanStep {
    pub swap_info: SwapInfo,
    pub percent: u8,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapInfo {
    pub amm_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub input_mint: String,
    pub output_mint: String,
    pub in_amount: String,
    pub out_amount: String,
    // Not every DEX adapter reports fee_amount / fee_mint (and Jupiter omits
    // them outright on some legs). Treat them as best-effort so an exotic
    // route shape doesn't fail the whole quote round trip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_amount: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_mint: Option<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformFee {
    pub amount: String,
    pub fee_bps: u16,
}

/// Response shape of `GET /quote`. Numeric token amounts come back as strings
/// (matching Jupiter) so values larger than `u53` don't round-trip lossily
/// through JavaScript clients.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteResponse {
    pub input_mint: String,
    pub in_amount: String,
    pub output_mint: String,
    pub out_amount: String,
    pub other_amount_threshold: String,
    pub swap_mode: SwapMode,
    pub slippage_bps: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_fee: Option<PlatformFee>,
    pub price_impact_pct: String,
    pub route_plan: Vec<RoutePlanStep>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_taken: Option<f64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Request body for `POST /swap`. `quote_response` is the verbatim payload
/// returned by `/quote` (or `jupiter_quote`) — Jupiter requires every field
/// from the original response, so we forward it untouched.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapRequest {
    pub user_public_key: String,
    pub quote_response: QuoteResponse,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrap_and_unwrap_sol: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub use_shared_accounts: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracking_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_unit_price_micro_lamports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prioritization_fee_lamports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_legacy_transaction: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destination_token_account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_compute_unit_limit: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_user_accounts_rpc_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_slippage: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Response shape of `POST /swap`. When returned via `jupiter_swap`, the
/// `swap_transaction` field has already been re-stamped with surfpool's
/// blockhash — the client signs and sends as-is.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SwapResponse {
    /// Base64-encoded `VersionedTransaction`.
    pub swap_transaction: String,
    pub last_valid_block_height: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prioritization_fee_lamports: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compute_unit_limit: Option<u64>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
