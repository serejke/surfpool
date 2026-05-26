//! Pool-snapshot refresh helper.
//!
//! Surfpool lazy-fetches mainnet account state on access. The first time a
//! DEX pool gets touched, the cached snapshot can already be stale enough
//! that Jupiter's slippage check trips and the swap reverts with
//! `Custom error 5` (`1027565`). The fix used in production is to purge the
//! affected accounts from the cache and let surfpool re-fetch on the next
//! read — `reset_account` is a no-op for absent accounts, so blanket-purging
//! every writable key the route touches is safe.
//!
//! There is one class of writable account we must NOT purge: the user's own
//! token ATAs. On a local fork, those were typically seeded via
//! `surfnet_setTokenAccount` (or just created by a prior local swap) and
//! have no mainnet equivalent — purging them resets to "not present" and
//! the next swap leg reverts. DEX pool vault accounts also live in the SPL
//! Token program, but their `owner` field is a pool-program PDA, not the
//! user. We distinguish by parsing the token-account data and checking
//! whose `owner` is recorded inside.

use std::time::{SystemTime, UNIX_EPOCH};

use solana_clock::Clock;
use solana_commitment_config::CommitmentConfig;
use solana_message::VersionedMessage;
use solana_program_pack::Pack;
use solana_pubkey::Pubkey;
use spl_token_2022_interface::id as token_2022_id;
use spl_token_interface::{id as token_id, state::Account as TokenAccount};

use surfpool_core::surfnet::{
    GetAccountResult, locker::SurfnetSvmLocker, remote::SurfnetRemoteClient,
};

use crate::error::{JupiterError, JupiterResult};

/// Resolve every writable account a transaction touches (static section +
/// ALT-loaded), and purge each from surfpool's local cache — except the
/// user's own token ATAs (see module docs).
///
/// `extra_skip` lets callers spare additional accounts (typically the user
/// signer's wallet pubkey, whose mainnet state is irrelevant on a local
/// fork).
///
/// Returns the pubkeys that were actually purged, in resolution order.
pub async fn refresh_writable_accounts(
    locker: &SurfnetSvmLocker,
    remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
    message: &VersionedMessage,
    user_pubkey: &Pubkey,
    extra_skip: &[Pubkey],
) -> JupiterResult<Vec<Pubkey>> {
    let writable = collect_writable_pubkeys(locker, remote_ctx, message).await?;
    let mut purged = Vec::with_capacity(writable.len());
    for pk in writable {
        if extra_skip.contains(&pk) {
            continue;
        }
        if is_user_owned_token_account(locker, &pk, user_pubkey) {
            log::debug!(
                "jupiter refresh: skipping user-owned token ATA {pk} (owner {user_pubkey})"
            );
            continue;
        }
        if let Err(e) = locker.reset_account(pk, false) {
            log::debug!("jupiter refresh: reset_account({pk}) failed: {e:?}");
            continue;
        }
        purged.push(pk);
    }

    // Surfpool's SVM clock advances deterministically from genesis slots
    // and lags wall-clock time. Freshly-refreshed mainnet accounts carry
    // `last_updated_timestamp` ≈ wall-now. If the SVM clock is behind,
    // pool programs that compare `Clock::unix_timestamp >= last_updated`
    // (Orca Whirlpool's `InvalidTimestamp` = 6022, etc.) reject the swap.
    // Bump the SVM clock forward — never backward.
    //
    // This must happen here in the /swap handler, not in a "principled"
    // deeper layer like the locker's remote-fetch path: surfpool's
    // per-slot `confirm_current_block` runloop reconstructs `Clock` from
    // the slot-derived `updated_at` at every slot tick (~slot_time
    // cadence), which overwrites any earlier bump unless the bump fires
    // immediately before tx execution. A hook in `get_account_local_then_remote`
    // would survive only ~slot_time milliseconds, often less than the
    // gap between the client's RPC roundtrip and the on-svm tx execution.
    advance_clock_to_wall_now(locker);

    Ok(purged)
}

fn advance_clock_to_wall_now(locker: &SurfnetSvmLocker) {
    let wall_now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => return,
    };
    locker.with_svm_writer(|svm_writer| {
        let mut clock = svm_writer.inner.get_sysvar::<Clock>();
        if clock.unix_timestamp < wall_now {
            log::debug!(
                "jupiter refresh: advancing svm clock {} -> {} ({}s)",
                clock.unix_timestamp,
                wall_now,
                wall_now - clock.unix_timestamp,
            );
            clock.unix_timestamp = wall_now;
            svm_writer.inner.set_sysvar(&clock);
        }
    });
}

/// Bare-bones variant of [`refresh_writable_accounts`] that takes an explicit
/// list of pubkeys — exposed via the `jupiter_refreshAccounts` cheatcode for
/// callers that already know which pools they want to flush.
pub fn refresh_accounts_by_pubkey(
    locker: &SurfnetSvmLocker,
    pubkeys: &[Pubkey],
) -> Vec<Pubkey> {
    let mut purged = Vec::with_capacity(pubkeys.len());
    for pk in pubkeys {
        if let Err(e) = locker.reset_account(*pk, false) {
            log::debug!("jupiter refresh: reset_account({pk}) failed: {e:?}");
            continue;
        }
        purged.push(*pk);
    }
    purged
}

fn is_user_owned_token_account(
    locker: &SurfnetSvmLocker,
    pubkey: &Pubkey,
    user_pubkey: &Pubkey,
) -> bool {
    let result = locker.get_account_local(pubkey).inner;
    let account = match result {
        GetAccountResult::FoundAccount(_, acc, _)
        | GetAccountResult::FoundProgramAccount((_, acc), _)
        | GetAccountResult::FoundTokenAccount((_, acc), _) => acc,
        GetAccountResult::None(_) => return false,
    };
    if account.owner != token_id() && account.owner != token_2022_id() {
        return false;
    }
    // Both Token and Token-2022 token accounts start with the same 165-byte
    // layout; `TokenAccount::unpack` reads the first 165 bytes.
    match TokenAccount::unpack(&account.data) {
        Ok(parsed) => &parsed.owner == user_pubkey,
        Err(_) => false,
    }
}

async fn collect_writable_pubkeys(
    locker: &SurfnetSvmLocker,
    remote_ctx: &Option<(SurfnetRemoteClient, CommitmentConfig)>,
    message: &VersionedMessage,
) -> JupiterResult<Vec<Pubkey>> {
    let mut out = Vec::new();
    for (idx, pk) in message.static_account_keys().iter().enumerate() {
        if message.is_maybe_writable(idx, None) {
            out.push(*pk);
        }
    }

    if let VersionedMessage::V0(m0) = message {
        if !m0.address_table_lookups.is_empty() {
            let loaded = locker
                .get_loaded_addresses(remote_ctx, message)
                .await
                .map_err(|e| JupiterError::LookupResolution(e.to_string()))?;
            if let Some(loaded) = loaded {
                let la = loaded.loaded_addresses();
                out.extend(la.writable.iter().copied());
            }
        }
    }

    Ok(out)
}
