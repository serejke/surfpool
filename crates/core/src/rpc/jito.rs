use std::str::FromStr;

use jsonrpc_core::{BoxFuture, Error, Result};
use jsonrpc_derive::rpc;
use solana_client::rpc_config::{RpcSendTransactionConfig, RpcSimulateTransactionConfig};
use solana_client::rpc_custom_error::RpcCustomError;
use solana_rpc_client_api::response::{Response as RpcResponse, RpcResponseContext, RpcSimulateTransactionResult};
use solana_signature::Signature;

use super::{
    RunloopContext,
    full::{Full, SurfpoolFullRpc},
};

/// Jito-specific RPC methods for bundle submission
#[rpc]
pub trait Jito {
    type Metadata;

    /// Sends a bundle of transactions to be processed sequentially.
    ///
    /// This RPC method accepts a bundle of transactions (Jito-compatible format) and processes them
    /// one by one in order. All transactions in the bundle must succeed for the bundle to be accepted.
    ///
    /// ## Parameters
    /// - `transactions`: An array of serialized transaction data (base64 or base58 encoded).
    /// - `config`: Optional configuration for encoding format.
    ///
    /// ## Returns
    /// - `Result<String>`: A bundle ID (SHA-256 hash of comma-separated signatures).
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "sendBundle",
    ///   "params": [
    ///     ["base64EncodedTx1", "base64EncodedTx2"],
    ///     { "encoding": "base64" }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Notes
    /// - Transactions are processed sequentially in the order provided
    /// - Each transaction must complete successfully before the next one starts
    /// - If any transaction fails, the entire bundle is rejected
    /// - The bundle ID is calculated as SHA-256 hash of comma-separated transaction signatures
    #[rpc(meta, name = "sendBundle")]
    fn send_bundle(
        &self,
        meta: Self::Metadata,
        transactions: Vec<String>,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String>;

    /// Simulates a bundle of transactions sequentially without executing them on-chain.
    ///
    /// This RPC method allows testing how a bundle of transactions would execute atomically
    /// without actually committing them to the blockchain. Each transaction is simulated
    /// with the account state changes from previous transactions applied.
    ///
    /// ## Parameters
    /// - `transactions`: An array of serialized transaction data (base64 or base58 encoded).
    /// - `config`: Optional configuration for encoding format and simulation settings.
    ///
    /// ## Returns
    /// - `BoxFuture<Result<Vec<RpcResponse<RpcSimulateTransactionResult>>>>`: A vector of simulation results, one for each transaction.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "simulateBundle",
    ///   "params": [
    ///     ["base64EncodedTx1", "base64EncodedTx2"],
    ///     { "encoding": "base64", "sigVerify": false }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Notes
    /// - Transactions are simulated sequentially in the order provided
    /// - Each transaction sees the account state changes from previous transactions
    /// - If any transaction fails, subsequent transactions are not simulated
    /// - The simulation does not modify the actual blockchain state
    /// - This is useful for testing bundle execution before submitting with sendBundle
    #[rpc(meta, name = "simulateBundle")]
    fn simulate_bundle(
        &self,
        meta: Self::Metadata,
        transactions: Vec<String>,
        config: Option<RpcSimulateTransactionConfig>,
    ) -> BoxFuture<Result<Vec<RpcResponse<RpcSimulateTransactionResult>>>>;
}

#[derive(Clone)]
pub struct SurfpoolJitoRpc;

impl Jito for SurfpoolJitoRpc {
    type Metadata = Option<RunloopContext>;

    fn send_bundle(
        &self,
        meta: Self::Metadata,
        transactions: Vec<String>,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String> {
        if transactions.is_empty() {
            return Err(Error::invalid_params("Bundle cannot be empty"));
        }

        let Some(_ctx) = &meta else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        let full_rpc = SurfpoolFullRpc;
        let mut bundle_signatures = Vec::new();

        // Process each transaction in the bundle sequentially using Full RPC
        for (idx, tx_data) in transactions.iter().enumerate() {
            // Delegate to Full RPC's sendTransaction method
            match full_rpc.send_transaction(meta.clone(), tx_data.clone(), config.clone()) {
                Ok(signature_str) => {
                    // Parse the signature to collect for bundle ID calculation
                    let signature = Signature::from_str(&signature_str).map_err(|e| {
                        Error::invalid_params(format!("Failed to parse signature: {e}"))
                    })?;
                    bundle_signatures.push(signature);
                }
                Err(e) => {
                    // Add bundle transaction index to error message
                    return Err(Error {
                        code: e.code,
                        message: format!("Bundle transaction {} failed: {}", idx, e.message),
                        data: e.data,
                    });
                }
            }
        }

        // Calculate bundle ID by hashing comma-separated signatures (Jito-compatible)
        // https://github.com/jito-foundation/jito-solana/blob/master/sdk/src/bundle/mod.rs#L21
        use sha2::{Digest, Sha256};
        let concatenated_signatures = bundle_signatures
            .iter()
            .map(|sig| sig.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut hasher = Sha256::new();
        hasher.update(concatenated_signatures.as_bytes());
        let bundle_id = hasher.finalize();
        Ok(hex::encode(bundle_id))
    }

    fn simulate_bundle(
        &self,
        meta: Self::Metadata,
        transactions: Vec<String>,
        config: Option<RpcSimulateTransactionConfig>,
    ) -> BoxFuture<Result<Vec<RpcResponse<RpcSimulateTransactionResult>>>> {
        use solana_transaction::versioned::VersionedTransaction;
        use solana_transaction_status::UiTransactionEncoding;
        use crate::rpc::utils::decode_and_deserialize;

        Box::pin(async move {
            if transactions.is_empty() {
                return Err(Error::invalid_params("Bundle cannot be empty"));
            }

            let config = config.unwrap_or_default();
            let tx_encoding = config.encoding.unwrap_or(UiTransactionEncoding::Base58);
            let binary_encoding = tx_encoding.into_binary_encoding().ok_or_else(|| {
                Error::invalid_params(format!(
                    "unsupported encoding: {tx_encoding}. Supported encodings: base58, base64"
                ))
            })?;

            // Decode all transactions first
            let mut decoded_txs = Vec::with_capacity(transactions.len());
            for (idx, tx_data) in transactions.iter().enumerate() {
                let (_, tx) = decode_and_deserialize::<VersionedTransaction>(
                    tx_data.clone(),
                    binary_encoding,
                )
                .map_err(|e| {
                    Error {
                        code: e.code,
                        message: format!("Failed to decode transaction {}: {}", idx, e.message),
                        data: e.data,
                    }
                })?;
                decoded_txs.push(tx);
            }

            let Some(ctx) = meta else {
                return Err(RpcCustomError::NodeUnhealthy {
                    num_slots_behind: None,
                }
                .into());
            };

            let svm_locker = ctx.svm_locker.clone();
            let sigverify = config.sig_verify;

            // Simulate the bundle using the SVM locker
            let simulation_results = svm_locker.simulate_bundle(decoded_txs, sigverify);

            // Convert simulation results to RPC format
            let mut rpc_results = Vec::with_capacity(simulation_results.len());
            let slot = svm_locker.get_latest_absolute_slot();

            for sim_result in simulation_results {
                let value = match sim_result {
                    Ok(info) => {
                        RpcSimulateTransactionResult {
                            err: None,
                            logs: Some(info.meta.logs),
                            accounts: None,
                            units_consumed: Some(info.meta.compute_units_consumed),
                            return_data: if info.meta.return_data.data.is_empty() {
                                None
                            } else {
                                Some(solana_transaction_status::UiTransactionReturnData {
                                    program_id: info.meta.return_data.program_id.to_string(),
                                    data: (bs58::encode(&info.meta.return_data.data).into_string(), solana_transaction_status::UiReturnDataEncoding::Base64),
                                })
                            },
                            inner_instructions: None,
                            replacement_blockhash: None,
                            loaded_accounts_data_size: None,
                        }
                    }
                    Err(failed) => {
                        RpcSimulateTransactionResult {
                            err: Some(failed.err),
                            logs: Some(failed.meta.logs),
                            accounts: None,
                            units_consumed: Some(failed.meta.compute_units_consumed),
                            return_data: if failed.meta.return_data.data.is_empty() {
                                None
                            } else {
                                Some(solana_transaction_status::UiTransactionReturnData {
                                    program_id: failed.meta.return_data.program_id.to_string(),
                                    data: (bs58::encode(&failed.meta.return_data.data).into_string(), solana_transaction_status::UiReturnDataEncoding::Base64),
                                })
                            },
                            inner_instructions: None,
                            replacement_blockhash: None,
                            loaded_accounts_data_size: None,
                        }
                    }
                };

                rpc_results.push(RpcResponse {
                    context: RpcResponseContext::new(slot),
                    value,
                });
            }

            Ok(rpc_results)
        })
    }
}
