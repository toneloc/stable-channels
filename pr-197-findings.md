# PR #197 Review Findings — Round 3

Commit reviewed: `aac0cd41d198bbc305feeaa1bba3eac37bfa8ab2`

## Status of round-2 findings

1. **Phantom receives during close/sweep/splice** — FIXED. `handleWebSocketTransactionDetected` now guards `!isChannelClosing, !isSweeping, pendingSplice == nil` for the address path, and prevout parsing was removed from `MempoolWSVin`, so spends from the tracked address no longer match as deposits.
2. **Plural response decoding** — FIXED. `MempoolWSMessage` decodes `multi-address-transactions` (keyed by address, with `mempool`/`confirmed`/`removed` arrays) and `tracked-txs`. Request shapes (`{"track-addresses": [...]}` / `{"track-txs": [...]}`, empty arrays to clear) match mempool.space's documented API.
3. **WS/LDK-fallback duplicate rows** — IMPROVED, one hole remains (new finding 2 below).
4. **Close txid discarded** — FIXED in code. The `isTxid` branch now maps the funding txid to its opId via `fetchPendingOperationByFundingTxid` and calls `handleCloseTxidResolved`. Protocol-level doubt remains (new finding 3).
5. **Only first tx processed / block skipped on dup** — FIXED. `handleMessage` aggregates all transaction payloads and handles the block header outside the loop.
6. **Red tests** — FIXED and verified: `MempoolWebSocketServiceTests` executed locally on commit `aac0cd4` (iPhone 17 Pro simulator): 31 tests, 0 failures. Connection-state tests now correctly assert async semantics.

## New findings (round 3)

1. **RBF retraction is dead code, and replaced transactions leave zombie pending rows.**
   `aggregateTransactions` includes each group's `removed` array in the main detection loop, which calls `recordProcessedTx` for every tx. `handleRemovedTransactions` then checks `isRecentlyProcessed` and therefore always skips the very txids the same message just recorded — the amount-0 retraction callback can never fire. Even if it did fire, `handleWebSocketTransactionDetected` has no retraction branch (`amountSats >= 1000` guard just drops it). Net effect: when a sender RBF-replaces a deposit, the `onchain_receive_<oldTxid>` row stays `pending` forever — it never confirms, shows as a stuck payment in history, and permanently occupies one of `paymentsNeedingConfirmation()`'s LIMIT-50 slots. Additionally, a `removed` tx not seen before (e.g., subscribe raced the replacement) flows through the normal match path with a positive vout amount and is recorded as a NEW pending receive for a transaction that was just evicted from the mempool. Fix: exclude `removed` txs from the detection aggregate, don't mark them processed before `handleRemovedTransactions`, and add a retraction path (mark the payment row `failed`/`replaced` by txid).

2. **The WS-vs-fallback dedup window (900s, status='pending') can still duplicate rows.**
   `hasRecentPendingOnchainReceive` only matches rows that are still `pending` and younger than 15 minutes. The WS row stays pending for ~6 confirmations (~60 min), but the LDK balance jump can be observed more than 15 minutes after insert — e.g., deposit arrives while the app is open (WS row inserted), the app is suspended before LDK syncs, and the user returns 20+ minutes later: `detectOnchainDeposit` fires, the WS row is older than the cutoff, and a duplicate `onchain_deposit_<UUID>` row is inserted. Suggest widening the window to cover the confirmation horizon (e.g., 2h), including `completed` rows, and preferring a txid/address-based match over amount-only matching.

3. **Close-tx acceleration relies on undocumented mempool.space push behavior — verify empirically before relying on it.**
   mempool.space's docs document `track-txs` for confirmation/position/RBF status of the tracked tx itself; outspend push (notification when a tracked tx's *outputs* are spent) is not documented, and the assumed `tracked-txs: {trackedTxid: <spending tx>}` response shape is unverified. The vin-match fallback in `TransactionMatcher` only helps if the spending (close) tx is delivered for some other tracked target — it won't be, since close outputs pay untracked addresses. The REST close resolver is still launched unconditionally (good), so worst case this path is inert. Recommend testing against a real close before counting the WS path as a feature; also note `guard isChannelClosing` in the `isTxid` branch drops WS close signals for remote force-closes.

4. **Reconnect policy: fixed 3s delay, no backoff, forever.**
   Every receive failure schedules a reconnect in 3s and each `connect()` builds a fresh URLSession. Offline devices will churn connections indefinitely every 3 seconds, which undercuts the PR's battery rationale. Suggest exponential backoff with a cap, and disconnecting when the app backgrounds.

5. **Minor:**
   - `startOnchainTxidResolver` calls `trackAddress` *before* the resolver-configured guard, so an unconfigured resolver still leaves the address tracked with no untrack path. Used receive addresses are also never untracked after a deposit completes (only on FundWalletView rotation); tracked-address count grows over a long session and mempool.space caps tracked addresses.
   - `hasRecentPendingOnchainReceive(matching:minAgeSeconds:)` — the parameter is a max-age/window, not a min age; rename.
   - Privacy: the persistent socket streams the user's receive addresses and funding txids to mempool.space in real time tied to their IP. Consistent with the app's existing Esplora usage, but worth a conscious sign-off.

## Verified strengths

- `TransactionMatcher` extraction (pure function) and the `MempoolWebSocketProtocol` seam are real testability improvements.
- Payment-ID dedup (`onchain_receive_<txid>`) is idempotent across WS redeliveries.
- `StaggeredTaskLauncher` closures are `@MainActor`, so the `untrackTx` call from the close launcher is correctly isolated (checked).
- `created_at` is stored as Unix seconds, so the dedup query's time comparison is sound (checked).
- Round-1's overly broad txid dedup in `recordPayment` is gone; dedup is by `payment_id` only.
- Test suite: 31/31 green, verified locally on this commit.
