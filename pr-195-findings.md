# PR #195 Review Findings — Round 3

Commit reviewed: `0d7cc326` ("Address remaining desktop and LSP review feedback")

## Status of round-2 findings

1. **Over-backed repair during in-flight HTLCs** — FIXED. New `repair_overbacked_allocation_if_safe` gates repair on `has_pending_outbound_lightning_payment` (outbound + Pending + not on-chain), and `check_stability` additionally aborts the entire stability decision when over-backed with a pending outbound payment, so the false drift signal can't trigger a payment either. Unit test covers the predicate.
2. **Mobile SYNC_V1 signed allocation** — DEFERRED by design; PR rescoped to desktop + LSP. Verified the compatibility path: `quote_price`/`backing_sats`/`ts` are optional in TRADE_V1, unsigned quotes take the legacy fee-tolerance path, and missing signed allocation falls back to LSP-side pricing. LSP-first rollout therefore does not break current mobile clients; mobile gets exact-allocation + replay protection in a follow-up (file an issue so it doesn't drop).
3. **LSP failed stability payment rollback** — FIXED, and well. The send path persists the full reversible transition (`backing_sats_before/after`, `native_sats_before`, `expected_usd`, `last_stability_payment_before`, `outcome='pending'`) in `settlement_payments`; `PaymentSuccessful` marks the outcome consumed; `PaymentFailed` runs a consume-once, transaction-guarded compare-and-swap that restores the channel row only if the allocation is still the exact optimistic state, with matching in-memory CAS, throttle clearing, and audit events. Restart-safe (in-memory reloads from DB). Tests cover applied, skipped-newer-allocation, and cannot-rollback-after-success.
4. **Desktop price quarantine bypass** — FIXED for accounting: the background loop uses `refresh_cached_price()`, `stable::update_balances` reads `get_fresh_cached_price_no_fetch()` (age- and quarantine-gated), the UI refresh button routes through `refresh_cached_price()`, and the LSP price task already used it. One residual below (new finding 2).
5. **Signed sell fee tolerance at sat boundaries** — FIXED. Tolerance is now exactly 1 sat (1000 msat) for signed quotes with a unit test demonstrating the 113/114-sat boundary case. Verified the wallet's `stable_trade_fee` is exact 1% in f64 (no cent rounding), so wallet/LSP divergence is ulp-level and cannot exceed 1 sat. Legacy tolerance retained for unsigned mobile quotes.

## New findings (round 3) — none blocking for the desktop/LSP scope

1. **Node-wide pending-payment guard can wedge repair on a stuck payment.**
   `has_pending_outbound_lightning_payment` scans all payments unfiltered; any outbound payment stuck in `Pending` (LDK does eventually abandon these, but it can take a while) blocks over-backed repair — and, when over-backed, the entire stability decision — indefinitely, with `OVERBACKED_REPAIR_SKIPPED_PENDING_HTLC` audited every tick. Suggest scoping the guard to recent payments (e.g., created within the last hour) via `list_payments_with_filter`, which also avoids the full-store scan each tick.
2. **Desktop trade-quote composer still reads the raw price cache.**
   `user.rs:8775` builds (and the flow then signs) a trade quote from `get_cached_price_no_fetch()`, ignoring staleness/quarantine. The LSP's quote-deviation check rejects off-market signed quotes, so this is a UX/consistency issue rather than fund-safety — but the composer should use `get_fresh_cached_price_no_fetch()` and surface "price unavailable" instead of letting users sign quotes at a quarantined price. (`user.rs:1645`, `:8712`, `:9020` raw reads are display-only and fine.)
3. **Test flake under parallel execution.**
   `audit::tests::test_capture_records_events_when_enabled` fails intermittently in a parallel `cargo test --lib` run (got 2 events, expected 1) because the PR's new tests exercise audit-emitting functions concurrently with the global capture buffer. Passes in isolation and single-threaded (118/118). Fix by serializing audit tests with a shared mutex or scoping the capture.
4. **Cosmetic:** the over-backed branch in `check_stability` calls `repair_overbacked_allocation_if_safe` purely for its audit side effect and then returns; the two-step structure invites drift if either condition changes. `mark_settlement_succeeded` fires for every `PaymentSuccessful` regardless of kind — harmless today since only stability rows carry rollback metadata.

## Verification

- LSP crate (`server/stable-channels-lsp`): `cargo test` — **122 passed, 0 failed**.
- Root crate: `cargo test --lib` — **118 passed single-threaded** (one parallel-run flake, finding 3; the round-2 `electrsd` build blocker did not recur for `--lib`).
- Fee boundary, rollback CAS, quarantine gating, and legacy mobile tolerance all verified against source at `0d7cc326`, not just commit messages.

## Rollout reminders

- LSP-first, wallets-second (as decided in round 1); LSP retains legacy `user_channel_id` sync fields during transition.
- Follow-ups to file: mobile signed SYNC_V1 + replay protection; trade NACK/expiry (deferred from round 1); quarantine-aware quote composer; pending-payment guard scoping.
