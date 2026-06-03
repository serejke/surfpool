# Implement `slotsUpdatesSubscribe` WebSocket RPC

## Objective

Replace the existing no-op stub for `slotsUpdatesSubscribe` / `slotsUpdatesUnsubscribe` in `crates/core/src/rpc/ws.rs` with a fully working implementation that mirrors Solana's reference specification:

- Streams tagged slot-lifecycle updates (`createdBank`, `frozen`, `optimisticConfirmation`, `root`, etc.) to subscribed WebSocket clients under the `slotsUpdatesNotification` method.
- Follows the established conventions used by the other WebSocket methods already implemented in `ws.rs` (e.g., `slotSubscribe`, `logsSubscribe`, `accountSubscribe`).
- Hooks into the SVM's existing slot/block lifecycle inside `confirm_current_block` so that real notifications fire as the surfnet chain advances.

The deliverable matches the wire format documented at <https://solana.com/docs/rpc/websocket/slotsupdatessubscribe.md>, reusing the upstream `solana_rpc_client_api::response::SlotUpdate` enum (already a transitive dependency) to guarantee `#[serde(tag = "type", rename_all = "camelCase")]` compatibility.

## Initial Assessment

### Project Structure Summary
- `crates/core/src/rpc/ws.rs` (`crates/core/src/rpc/ws.rs:1-1947`): contains the `#[rpc]` `Rpc` trait, the `SurfpoolWsRpc` struct, and per-method implementations.
- `crates/core/src/surfnet/svm.rs` (`crates/core/src/surfnet/svm.rs:250-3320`): defines `SurfnetSvm` plus its subscription vectors (`slot_subscriptions`, `logs_subscriptions`, `snapshot_subscriptions`) and the `subscribe_for_*` / `notify_*_subscribers` helpers.
- `crates/core/src/surfnet/locker.rs` (`crates/core/src/surfnet/locker.rs:3402-3450`): wraps the SVM subscription helpers behind the `SurfnetSvmLocker` used by the WS RPC layer.
- `crates/core/src/surfnet/mod.rs` (`crates/core/src/surfnet/mod.rs:30-211`): centralizes shared type aliases (`LogsSubscriptionData`, `SnapshotSubscriptionData`, `SignatureSubscriptionType`, `GeyserSlotStatus`, etc.).
- `crates/core/src/runloops/mod.rs` (`crates/core/src/runloops/mod.rs:998-1009`): instantiates `SurfpoolWsRpc` and is where any new subscription map field must be initialized.
- `crates/core/src/tests/integration.rs` (`crates/core/src/tests/integration.rs:6840-6970`): existing patterns for slot-subscription integration tests.

### Relevant Files Examination
- The trait already declares `slots_updates_subscribe` (`crates/core/src/rpc/ws.rs:753-773`) but with the wrong payload type (`Subscriber<RpcResponse<()>>`). The impl is a stub that only logs a warning (`crates/core/src/rpc/ws.rs:1774-1790`).
- `subscribe_for_slot_updates` (`crates/core/src/surfnet/svm.rs:3178-3187`) is the closest existing analogue and produces `SlotInfo`. This is **a different RPC** (`slotSubscribe`) and must not be re-used directly — `slotsUpdatesSubscribe` requires the tagged `SlotUpdate` variants.
- `confirm_current_block` (`crates/core/src/surfnet/svm.rs:2262-2404`) is the single canonical place where slot/block lifecycle transitions happen today: a new slot is created, transactions are confirmed (`confirm_transactions`, `crates/core/src/surfnet/svm.rs:2033-2085`), then finalized (`finalize_transactions`, `crates/core/src/surfnet/svm.rs:2091-2129`), and Geyser slot-status events are emitted. This is where new `SlotUpdate` emissions must be inserted.
- `solana_rpc_client_api::response::SlotUpdate` is already part of the dependency graph (`Cargo.lock` lists `solana-rpc-client-api` in multiple crates). The same module is already used in `ws.rs` (it imports `Response as RpcResponse, SlotInfo`).

### Prioritized Challenges & Risks
1. **Trait signature change.** The current `Subscriber<RpcResponse<()>>` will not serialize a tagged enum and must change to `Subscriber<Arc<SlotUpdate>>` (or `Subscriber<SlotUpdate>`). Because the JSON-RPC notification format wraps payloads directly under `params.result` (not under `{context, value}`), we follow the same pattern as `slotSubscribe` which uses `Subscriber<SlotInfo>` without an `RpcResponse` wrapper. This is the most fundamental change.
2. **Mapping surfpool's simplified block lifecycle to Solana's seven `SlotUpdate` variants.** Surfpool produces one entry per block and has no gossip/shred layer, so `FirstShredReceived` and `Completed` are not naturally produced. We must pick a sane subset and document the choice.
3. **Plumbing the new channel through every constructor.** `SurfnetSvm` is constructed in two places inside `crates/core/src/surfnet/svm.rs` (the normal constructor near line 550 and the sandbox builder near line 993), both of which must initialize the new `slots_updates_subscriptions: Vec::new()` field, or risk breaking the sandbox/snapshot path.
4. **Runloop wiring.** `runloops/mod.rs:998-1009` builds the `SurfpoolWsRpc` struct literal — every new map field needs to be added there explicitly, otherwise the crate will not compile.
5. **Timestamp source.** Solana emits `timestamp` in milliseconds (Unix epoch). Surfpool already has `self.updated_at` (millis-based) on the SVM (`crates/core/src/surfnet/svm.rs:2274`). We must use a consistent source (`chrono::Utc::now().timestamp_millis()` is used elsewhere; e.g., the snapshot path uses `chrono::Utc::now().timestamp_nanos_opt()` at `crates/core/src/rpc/ws.rs:1880`). Using wall-clock millis avoids leaking simulated time semantics into the protocol.

## Assumptions

- The wire protocol must follow the upstream `SlotUpdate` enum exactly (`#[serde(tag = "type", rename_all = "camelCase")]`); we will reuse `solana_rpc_client_api::response::SlotUpdate` and `SlotTransactionStats` rather than redefining them.
- Surfpool will emit a meaningful subset of slot-update variants. The supported set will be: `CreatedBank`, `Frozen`, `OptimisticConfirmation`, `Root`. `FirstShredReceived` and `Completed` are not naturally produced by surfpool's simulated execution model (no shred/gossip layer); `Dead` is reserved for fatal block production errors (currently unreachable in surfpool — left as a future enhancement).
- The notification payload is *not* wrapped in `RpcResponse<...>` — Solana sends the `SlotUpdate` object directly under `params.result`. The trait subscription type will therefore be `Subscriber<Arc<SlotUpdate>>`, matching how `slotSubscribe` uses `Subscriber<SlotInfo>` (no `RpcResponse` wrapping).
- Subscription IDs continue to come from the existing atomic counter `self.uid` shared with every other subscription type.
- Channel back-pressure is handled the same way as the other subscriptions: `crossbeam_channel::unbounded()` with `try_recv` polling at 50 ms intervals (consistent with `slot_subscribe`).
- Tests will live in `crates/core/src/tests/integration.rs`, alongside the existing `test_ws_slot_subscribe_*` tests.
- No public-API breakage outside the WebSocket method itself is desired; the JSON shape exposed to clients must match the Solana reference exactly.

## Implementation Plan

- [DONE] Task 1. **Introduce a shared `SlotsUpdatesSubscriptionData` alias in `crates/core/src/surfnet/mod.rs`.**
    - Add `pub type SlotsUpdatesSubscriptionData = Sender<Arc<SlotUpdate>>;` (re-exporting/importing `SlotUpdate` from `solana_rpc_client_api::response`).
    - Rationale: follows the convention established by `LogsSubscriptionData` and `SnapshotSubscriptionData` at `crates/core/src/surfnet/mod.rs:173-179` and keeps subscription types centralized.

- [DONE] Task 2. **Add a `slots_updates_subscriptions: Vec<SlotsUpdatesSubscriptionData>` field to `SurfnetSvm` in `crates/core/src/surfnet/svm.rs`.**
    - Insert the field next to the existing `slot_subscriptions` field at `crates/core/src/surfnet/svm.rs:276`.
    - Initialize the field to `Vec::new()` in both `SurfnetSvm` constructors:
        - The standard constructor around `crates/core/src/surfnet/svm.rs:550`.
        - The sandbox/snapshot constructor around `crates/core/src/surfnet/svm.rs:993`.
    - Rationale: omitting initialization in either constructor will fail to compile or silently break the sandbox path.

- [DONE] Task 3. **Add subscription/notify helpers on `SurfnetSvm` in `crates/core/src/surfnet/svm.rs`.**
    - `pub fn subscribe_for_slots_updates(&mut self) -> Receiver<Arc<SlotUpdate>>` — creates an unbounded channel, pushes the sender onto `self.slots_updates_subscriptions`, returns the receiver. Mirror `subscribe_for_slot_updates` (`crates/core/src/surfnet/svm.rs:3178-3182`).
    - `pub fn notify_slots_updates_subscribers(&mut self, update: SlotUpdate)` — wraps the update in `Arc`, fan-outs to every active sender with `retain(|tx| tx.send(arc.clone()).is_ok())` to auto-prune disconnected receivers. Mirror `notify_slot_subscribers` (`crates/core/src/surfnet/svm.rs:3184-3187`).
    - Rationale: keeps the SVM as the single source of truth for subscription fan-out and matches the architecture used for the existing slot/logs/snapshot subscriptions.

- [DONE] Task 4. **Emit `SlotUpdate` events from `confirm_current_block` in `crates/core/src/surfnet/svm.rs:2262-2404`.**
    - Compute `timestamp = chrono::Utc::now().timestamp_millis() as u64` once at the top of the function.
    - After the new slot has been computed (`new_slot`, `parent_slot`, `root` at `crates/core/src/surfnet/svm.rs:2321-2323`), emit:
        - `SlotUpdate::CreatedBank { slot: new_slot, parent: parent_slot, timestamp }` immediately before `notify_slot_subscribers(...)`.
        - `SlotUpdate::Frozen { slot: new_slot, timestamp, stats: SlotTransactionStats { num_transaction_entries: 1, num_successful_transactions, num_failed_transactions, max_transactions_per_entry } }` after the entry/stat info is known. Compute `num_successful_transactions` / `num_failed_transactions` from the result of `confirm_transactions` and from the recorded transaction errors (use `confirmed_signatures.len()` as a baseline and refine using `transactions_queued_for_confirmation` error variants tracked during `confirm_transactions`; `max_transactions_per_entry` equals `num_transactions` for surfpool since there is exactly one entry per block).
        - `SlotUpdate::OptimisticConfirmation { slot, timestamp }` alongside the existing `GeyserEvent::UpdateSlotStatus { ... Confirmed }` emission at `crates/core/src/surfnet/svm.rs:2329-2335`.
        - `SlotUpdate::Root { slot: root, timestamp }` inside the existing `if root >= self.genesis_slot { ... }` guard at `crates/core/src/surfnet/svm.rs:2385-2393`, right next to the `Rooted` Geyser emission.
    - Rationale: piggy-backing on the existing block-lifecycle code path keeps the slot-update timeline consistent with the rest of surfpool's notifications and avoids introducing a second slot-progression code path.
    - **Sub-decision:** if tracking per-tx failure counts inside `confirm_transactions` is non-trivial, fall back to emitting `num_failed_transactions: 0` with a `TODO` and an inline comment. This keeps the wire format valid while leaving an enhancement hook.

- [DONE] Task 5. **Expose a thin wrapper on `SurfnetSvmLocker` in `crates/core/src/surfnet/locker.rs`.**
    - Add `pub fn subscribe_for_slots_updates(&self) -> Receiver<Arc<SlotUpdate>>` next to `subscribe_for_slot_updates` (`crates/core/src/surfnet/locker.rs:3437-3439`) that delegates via `self.with_svm_writer(|svm| svm.subscribe_for_slots_updates())`.
    - Rationale: mirrors the existing per-subscription wrappers and keeps locking semantics consistent.

- [DONE] Task 6. **Update the `Rpc` trait signature for `slots_updates_subscribe` in `crates/core/src/rpc/ws.rs:753-763`.**
    - Change `subscriber: Subscriber<RpcResponse<()>>` to `subscriber: Subscriber<Arc<solana_rpc_client_api::response::SlotUpdate>>`.
    - Add full rustdoc that follows the documentation style used for `slot_subscribe` (`crates/core/src/rpc/ws.rs:353-413`) and `logs_subscribe` (`crates/core/src/rpc/ws.rs:466-538`), including the JSON request/response/notification examples taken from the Solana spec (`createdBank`, `frozen`, `optimisticConfirmation`, `root`).
    - Leave the `slots_updates_unsubscribe` trait signature intact (`crates/core/src/rpc/ws.rs:764-773`) but add equivalent rustdoc.
    - Rationale: the existing `RpcResponse<()>` typing prevents the macro from generating a notification with the right shape; aligning with `Subscriber<Arc<SlotUpdate>>` mirrors how `slot_subscribe` exposes a non-RpcResponse-wrapped payload.

- [DONE] Task 7. **Add `slots_updates_subscription_map` to `SurfpoolWsRpc` in `crates/core/src/rpc/ws.rs:961-975`.**
    - Field type: `Arc<RwLock<HashMap<SubscriptionId, Sink<Arc<SlotUpdate>>>>>`.
    - Update the struct-level rustdoc comment (`crates/core/src/rpc/ws.rs:928-960`) to mention the new subscription map alongside the existing entries.
    - Rationale: mirrors the existing per-subscription maps and is necessary so both the subscribe and unsubscribe methods can locate sinks.

- [DONE] Task 8. **Replace the stub implementation of `slots_updates_subscribe` in `crates/core/src/rpc/ws.rs:1774-1782` with a real handler.**
    - Pattern to follow: `slot_subscribe` (`crates/core/src/rpc/ws.rs:1397-1470`).
    - Steps inside the impl:
        - Log a debug message via `meta.log_debug(...)`.
        - Allocate a `SubscriptionId::Number` from `self.uid.fetch_add(...)`.
        - Call `subscriber.assign_id(sub_id.clone())` and error-handle exactly as the slot subscription does.
        - Resolve the SVM locker via `meta.get_svm_locker()` with the same error-handling style.
        - Spawn an async task on `self.tokio_handle` that:
            1. Inserts the sink into `self.slots_updates_subscription_map`.
            2. Calls `svm_locker.subscribe_for_slots_updates()` to obtain the receiver.
            3. Loops: check the map for the sub id (break if removed), `try_recv` from the channel, sleep `50 ms` if empty, otherwise `sink.notify(Ok(arc_slot_update))`.
            4. Logs and breaks on `log::error!` for sink failures, matching the existing convention.
    - Rationale: structural parity with the other subscriptions reduces cognitive load and matches the project's existing async polling pattern.

- [DONE] Task 9. **Replace the stub `slots_updates_unsubscribe` in `crates/core/src/rpc/ws.rs:1784-1790` with the real handler.**
    - Pattern: `slot_unsubscribe` (`crates/core/src/rpc/ws.rs:1472-1488`).
    - Acquire a write lock on `self.slots_updates_subscription_map`, remove the entry, return `Ok(true)`. Return `Err(Error { code: InternalError, ... })` when the lock cannot be acquired.

- [DONE] Task 10. **Initialize the new subscription map in `crates/core/src/runloops/mod.rs:998-1009`.**
    - Add `slots_updates_subscription_map: Arc::new(RwLock::new(HashMap::new())),` to the `SurfpoolWsRpc { ... }` struct literal, slotted next to the existing `slot_subscription_map`.
    - Rationale: required for compilation after Task 7 and for the runloop to forward updates correctly.

- [DONE] Task 11. **Add integration tests in `crates/core/src/tests/integration.rs`.**
    - Place tests next to the existing `test_ws_slot_subscribe_*` group (`crates/core/src/tests/integration.rs:6840-6970`).
    - Required test cases (using the `TestType` matrix already in place: `sqlite`, `in_memory`, `no_db`, optional `postgres` feature):
        - `test_ws_slots_updates_subscribe_basic` — drives `confirm_current_block` once and asserts that the channel produces a `CreatedBank` followed by `Frozen`, `OptimisticConfirmation`, then eventually `Root` (after enough slot advancements to exceed `FINALIZATION_SLOT_THRESHOLD`).
        - `test_ws_slots_updates_subscribe_payload_shape` — asserts that the serialized JSON contains a `type` discriminator and the documented fields per variant by serializing the `SlotUpdate` via `serde_json::to_value`.
        - `test_ws_slots_updates_subscribe_multiple_subscribers` — mirrors `test_ws_slot_subscribe_multiple_subscribers` (`crates/core/src/tests/integration.rs:6905-6937`), checking fan-out to ≥ 3 receivers.
        - `test_ws_slots_updates_unsubscribe` — calls the new unsubscribe method (or drops the receiver) and verifies the SVM stops retaining the closed sender (relies on the `retain(|tx| tx.send(...).is_ok())` semantics).
    - Rationale: the documented variant matrix is the most fragile contract, so the JSON shape assertion is essential to prevent silent breakage on dependency upgrades.

- [DONE] Task 12. **Update inline rustdoc for `SurfpoolWsRpc` struct fields list (`crates/core/src/rpc/ws.rs:935-960`).**
    - Document the new `slots_updates_subscription_map` field alongside the existing maps, briefly mentioning that it streams `slotsUpdatesNotification` payloads.
    - Rationale: keeps the documentation in sync with the public-facing surface, matching the level of detail given to the other subscription maps.

- [DONE] Task 13. **Run `cargo check -p surfpool-core` and `cargo test -p surfpool-core slots_updates` (or the equivalent target) to validate.**
    - The plan does not execute commands, but the implementer should expect this verification step. Confirm the `solana-rpc-client-api` re-export path used (`solana_rpc_client_api::response::SlotUpdate`) compiles given the workspace's pinned version.
    - Rationale: ensures all wiring changes (trait signature, runloop struct literal, SVM constructors) are coherent.

## Verification Criteria

- A client sending `{"jsonrpc":"2.0","id":1,"method":"slotsUpdatesSubscribe"}` over the WS endpoint receives a numeric subscription id (no longer the `unimplemented` warning).
- Once subscribed, the client receives notifications under method `slotsUpdatesNotification` whose `params.result` matches one of the documented tagged variants: `createdBank`, `frozen` (with a `stats` block), `optimisticConfirmation`, `root`. Each notification includes a millisecond `timestamp` field.
- A subsequent `{"jsonrpc":"2.0","id":2,"method":"slotsUpdatesUnsubscribe","params":[<id>]}` returns `"result": true` and the client stops receiving further notifications.
- Multiple concurrent subscriptions to `slotsUpdatesSubscribe` all receive the same lifecycle events (fan-out works).
- `cargo build -p surfpool-core` succeeds with no new warnings introduced by the change.
- The new integration tests in `tests/integration.rs` pass across every `TestType` variant.
- Serializing a `SlotUpdate::Frozen { ... }` produces JSON with `"type": "frozen"` and a `stats` object containing exactly `numTransactionEntries`, `numSuccessfulTransactions`, `numFailedTransactions`, `maxTransactionsPerEntry` (camelCase), confirming wire compatibility with the Solana reference.
- The original `slot_subscribe` behavior remains unchanged (regression check: existing `test_ws_slot_subscribe_*` tests still pass).

## Potential Risks and Mitigations

1. **Trait signature change breaks the JSON-RPC macro output.**
   Mitigation: Verify that the `jsonrpc_pubsub` macro still generates a valid notification when the subscriber type is `Subscriber<Arc<SlotUpdate>>`. If the macro requires the inner type to implement `Serialize` directly (not via `Arc`), fall back to `Subscriber<SlotUpdate>` and clone-send through the channel. `SlotUpdate: Clone` is implemented upstream, so this fallback is safe.

2. **Surfpool does not produce `firstShredReceived` / `completed` / `dead` variants.**
   Mitigation: Document the supported subset in the trait rustdoc and add a `// NOTE:` comment in `confirm_current_block`. The variants we do emit cover the most common client use-cases (validator-style monitoring of bank lifecycle + finalization).

3. **`num_failed_transactions` may not be readily available in `confirm_transactions`.**
   Mitigation: Either (a) extend `confirm_transactions` to return `(Vec<Signature>, u64 /* failed */)`, or (b) leave the value at `0` for v1 with a clearly labeled `TODO`. The wire format remains valid either way.

4. **Timestamp drift between simulated clock and wall-clock.**
   Mitigation: Use `chrono::Utc::now().timestamp_millis()` (same approach the snapshot subscription already uses at `crates/core/src/rpc/ws.rs:1878-1881`). Clients of `slotsUpdates` historically expect a real Unix timestamp; surfpool's simulated slot time would not satisfy that.

5. **Channel sender leakage when subscribers disconnect uncleanly.**
   Mitigation: Use `Vec::retain(|tx| tx.send(...).is_ok())` in `notify_slots_updates_subscribers` — same self-pruning pattern already used by `notify_slot_subscribers` at `crates/core/src/surfnet/svm.rs:3185-3187`.

6. **Sandbox / snapshot path drops the new field on clone/replace.**
   Mitigation: Verify both constructor sites in `svm.rs` (around lines 550 and 993) explicitly initialize `slots_updates_subscriptions: Vec::new()`. This is the same invariant already maintained for `slot_subscriptions`, `logs_subscriptions`, and `snapshot_subscriptions` at `crates/core/src/surfnet/svm.rs:550-552, 997-998`.

7. **Geyser plugins or other consumers may incidentally rely on the order of emissions in `confirm_current_block`.**
   Mitigation: Insert the new `notify_slots_updates_subscribers` calls **alongside** the existing Geyser emissions rather than reordering them. The new calls are pure additions and should not perturb the existing Geyser event order.

## Alternative Approaches

1. **Define a custom `SlotUpdate` enum in `surfnet/mod.rs` instead of reusing `solana_rpc_client_api::response::SlotUpdate`.**
   Trade-offs: gives surfpool the flexibility to add variants (e.g., a `BlockProduced` variant tailored to simulated execution) but requires manually maintaining the `#[serde(tag = "type", rename_all = "camelCase")]` derivation and risks subtle wire-format drift if the upstream enum gains variants. Not preferred unless an upstream variant is missing.

2. **Map `slotsUpdatesSubscribe` notifications onto the existing `slot_subscriptions` infrastructure by emitting `SlotInfo` wrapped in a synthetic variant.**
   Trade-offs: avoids new vectors/methods but conflates two semantically distinct RPCs (`slotSubscribe` vs. `slotsUpdatesSubscribe`) and forces JSON-shape gymnastics in the WS layer. Strongly rejected — it would violate the principle of one subscription type per RPC method that the rest of `ws.rs` follows.

3. **Drive emissions from the geyser runloop (`crates/core/src/runloops/mod.rs:744-748`) by translating `GeyserEvent::UpdateSlotStatus` into `SlotUpdate` variants.**
   Trade-offs: keeps `confirm_current_block` untouched, but it means subscribers would only ever receive `OptimisticConfirmation` and `Root` (no `CreatedBank` / `Frozen`, because those have no Geyser counterpart). Useful as a fallback if Task 4 turns out to be more invasive than expected, but the resulting subscription would be less informative than the spec promises.

4. **Emit notifications synchronously inside `confirm_current_block` rather than via a channel + polling task.**
   Trade-offs: lower latency but couples block production to slow WS clients and risks slot advancement stalling when a subscriber's sink is full. Rejected — the existing channel + 50 ms polling pattern is the standard throughout `ws.rs`.
