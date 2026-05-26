//! JSON-RPC façade over [`crate::client::JupiterClient`] +
//! [`crate::refresh`].
//!
//! Methods mirror Jupiter's REST endpoints by name and payload shape:
//!
//! | JSON-RPC method            | Upstream equivalent             | Notes |
//! | -------------------------- | ------------------------------- | ----- |
//! | `jupiter_quote`            | `GET /quote`                    | Passthrough proxy. |
//! | `jupiter_swap`             | `POST /swap`                    | Adds blockhash rewrite + pool refresh; returns [`SurfpoolSwapResponse`]. |
//! | `jupiter_refreshAccounts`  | _(surfpool-specific)_           | Manually purge cached pool snapshots. |

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use jsonrpc_core::{BoxFuture, Result as RpcResult};
use jsonrpc_derive::rpc;
use solana_commitment_config::CommitmentConfig;
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::{Response as RpcResponse, RpcResponseContext};
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_core::{
    rpc::{RunloopContext, State, SurfnetRpcContext, SurfpoolMiddleware},
    surfnet::svm::MAX_RECENT_BLOCKHASHES_STANDARD,
};

use crate::{
    client::JupiterHttpClient,
    config::JupiterConfig,
    error::JupiterError,
    refresh,
    types::{
        QuoteRequest, QuoteResponse, RefreshAccountsRequest, RefreshAccountsResponse,
        SurfpoolSwapResponse, SwapRequest, SwapResponse,
    },
};

#[rpc]
pub trait Jupiter {
    type Metadata;

    /// Quote a swap route — equivalent to Jupiter's `GET /quote`.
    #[rpc(meta, name = "jupiter_quote")]
    fn jupiter_quote(
        &self,
        meta: Self::Metadata,
        request: QuoteRequest,
    ) -> BoxFuture<RpcResult<QuoteResponse>>;

    /// Build a Jupiter swap transaction stamped with surfpool's blockhash and
    /// with every writable pool account purged from the local cache.
    /// Equivalent payload to Jupiter's `POST /swap`, but the response is
    /// wrapped in [`SurfpoolSwapResponse`] so the caller can introspect what
    /// was refreshed.
    #[rpc(meta, name = "jupiter_swap")]
    fn jupiter_swap(
        &self,
        meta: Self::Metadata,
        request: SwapRequest,
    ) -> BoxFuture<RpcResult<RpcResponse<SurfpoolSwapResponse>>>;

    /// Purge an explicit list of accounts from surfpool's local cache so the
    /// next access re-fetches mainnet state. Useful before retrying a swap
    /// transaction the caller built earlier.
    #[rpc(meta, name = "jupiter_refreshAccounts")]
    fn jupiter_refresh_accounts(
        &self,
        meta: Self::Metadata,
        request: RefreshAccountsRequest,
    ) -> RpcResult<RpcResponse<RefreshAccountsResponse>>;
}

#[derive(Clone)]
pub struct JupiterRpc {
    config: Arc<JupiterConfig>,
    client: Arc<JupiterHttpClient>,
}

impl JupiterRpc {
    pub fn new(config: JupiterConfig) -> Self {
        let client = Arc::new(JupiterHttpClient::new(&config));
        Self {
            config: Arc::new(config),
            client,
        }
    }
}

impl Jupiter for JupiterRpc {
    type Metadata = Option<RunloopContext>;

    fn jupiter_quote(
        &self,
        _meta: Self::Metadata,
        request: QuoteRequest,
    ) -> BoxFuture<RpcResult<QuoteResponse>> {
        let client = self.client.clone();
        let config = self.config.clone();
        Box::pin(async move {
            ensure_enabled(&config)?;
            client.quote(&request).await.map_err(JupiterError::into_rpc)
        })
    }

    fn jupiter_swap(
        &self,
        meta: Self::Metadata,
        request: SwapRequest,
    ) -> BoxFuture<RpcResult<RpcResponse<SurfpoolSwapResponse>>> {
        let client = self.client.clone();
        let config = self.config.clone();
        Box::pin(async move {
            ensure_enabled(&config)?;

            let user_pubkey = parse_pubkey(&request.user_public_key)?;
            let SurfnetRpcContext {
                svm_locker,
                remote_ctx,
            } = meta.get_rpc_context(CommitmentConfig::confirmed())?;

            let jupiter_context_slot = request.quote_response.context_slot;

            // Override two fields before forwarding to Jupiter — both are
            // surfpool-specific necessities, not preferences:
            //
            //   1. skipUserAccountsRpcCalls: the user's mainnet state is
            //      always wrong on a local fork (fresh keypair has zero SOL
            //      on mainnet), so Jupiter's pre-simulation would always
            //      report `simulationError` and a non-skipping `/swap` would
            //      embed mismatched setup instructions.
            //   2. prioritizationFeeLamports: local surfpool doesn't need a
            //      real priority fee; forcing 0 keeps the tx fee deterministic.
            //
            // Clients can still set every other Jupiter swap option; only
            // these two are surfpool-mandatory.
            let mut upstream_request = request;
            upstream_request.skip_user_accounts_rpc_calls = Some(true);
            if upstream_request.prioritization_fee_lamports.is_none() {
                upstream_request.prioritization_fee_lamports = Some(0);
            }
            let user_public_key = upstream_request.user_public_key.clone();
            let _ = user_public_key; // already parsed above; kept for symmetry

            let upstream = client
                .swap(&upstream_request)
                .await
                .map_err(JupiterError::into_rpc)?;

            // ---- Deserialize, refresh, restamp ------------------------------
            let tx_bytes = BASE64.decode(&upstream.swap_transaction).map_err(|e| {
                JupiterError::Transaction(format!("base64 decode failed: {e}")).into_rpc()
            })?;
            let mut tx: VersionedTransaction = bincode::deserialize(&tx_bytes).map_err(|e| {
                JupiterError::Transaction(format!("bincode deserialize failed: {e}")).into_rpc()
            })?;

            let purged = refresh::refresh_writable_accounts(
                &svm_locker,
                &remote_ctx,
                &tx.message,
                &user_pubkey,
                &[user_pubkey],
            )
            .await
            .map_err(JupiterError::into_rpc)?;
            let refreshed_accounts: Vec<String> =
                purged.iter().map(|pk| pk.to_string()).collect();

            let commitment = CommitmentConfig::confirmed();
            let blockhash = svm_locker
                .get_latest_blockhash(&commitment)
                .unwrap_or_else(|| svm_locker.latest_absolute_blockhash());
            let epoch_info = svm_locker.get_epoch_info();
            let computed_lvbh =
                epoch_info.block_height + MAX_RECENT_BLOCKHASHES_STANDARD as u64;

            match &mut tx.message {
                VersionedMessage::Legacy(m) => m.recent_blockhash = blockhash,
                VersionedMessage::V0(m) => m.recent_blockhash = blockhash,
            }
            // Zero any placeholder signatures Jupiter may have set: the caller
            // signs after surfpool hands the tx back.
            for sig in tx.signatures.iter_mut() {
                *sig = Signature::default();
            }

            let final_bytes = bincode::serialize(&tx).map_err(|e| {
                JupiterError::Transaction(format!("re-encode failed: {e}")).into_rpc()
            })?;

            // Replace upstream's last_valid_block_height with the local fork's
            // — Jupiter's value is anchored to mainnet and would expire
            // immediately against surfpool's clock.
            let mut jupiter_payload = SwapResponse {
                swap_transaction: BASE64.encode(&final_bytes),
                last_valid_block_height: computed_lvbh,
                prioritization_fee_lamports: upstream.prioritization_fee_lamports,
                compute_unit_limit: upstream.compute_unit_limit,
                extra: upstream.extra,
            };
            // Preserve Jupiter's reported lvbh under a well-known key in
            // `extra` so callers debugging slot drift can still see it.
            jupiter_payload.extra.insert(
                "upstreamLastValidBlockHeight".to_string(),
                serde_json::Value::from(upstream.last_valid_block_height),
            );

            let value = SurfpoolSwapResponse {
                jupiter: jupiter_payload,
                blockhash: blockhash.to_string(),
                refreshed_accounts,
                jupiter_context_slot,
            };

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_locker.get_latest_absolute_slot()),
                value,
            })
        })
    }

    fn jupiter_refresh_accounts(
        &self,
        meta: Self::Metadata,
        request: RefreshAccountsRequest,
    ) -> RpcResult<RpcResponse<RefreshAccountsResponse>> {
        ensure_enabled(&self.config)?;
        let svm_locker = meta.get_svm_locker()?;
        let mut pubkeys = Vec::with_capacity(request.accounts.len());
        for raw in &request.accounts {
            pubkeys.push(parse_pubkey(raw)?);
        }
        let purged = refresh::refresh_accounts_by_pubkey(&svm_locker, &pubkeys);
        Ok(RpcResponse {
            context: RpcResponseContext::new(svm_locker.get_latest_absolute_slot()),
            value: RefreshAccountsResponse {
                refreshed_accounts: purged.iter().map(|pk| pk.to_string()).collect(),
            },
        })
    }
}

fn ensure_enabled(config: &JupiterConfig) -> RpcResult<()> {
    if config.enabled {
        Ok(())
    } else {
        Err(JupiterError::Disabled.into_rpc())
    }
}

fn parse_pubkey(raw: &str) -> RpcResult<Pubkey> {
    raw.parse::<Pubkey>().map_err(|e| {
        JupiterError::Pubkey {
            pubkey: raw.to_string(),
            reason: e.to_string(),
        }
        .into_rpc()
    })
}

/// Convenience helper used by surfpool-cli to wire the extension into the
/// shared RPC IO handler. Keeps the call site in cli a one-liner and means
/// the trait + impl don't need to be visible there.
pub fn register_extension(
    io: &mut jsonrpc_core::MetaIoHandler<Option<RunloopContext>, SurfpoolMiddleware>,
    config: JupiterConfig,
) {
    io.extend_with(JupiterRpc::new(config).to_delegate());
}
