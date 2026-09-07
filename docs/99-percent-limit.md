# Maximum stabilization: implementation and rollout

This is a **trade-entry limit**, not a permanent allocation invariant. Settlement, reconciliation,
and USD-to-BTC reductions remain governed by their existing drift-preservation rules.
It does not change on-chain Send Max or Lightning payment amounts.

## Policy

- Maximum stabilized backing: 99% of post-fee spendable receiver sats, rounded down.
- Minimum unstabilized spendable reserve: 2,000 sats; this may bind below 99% on small channels.
- All client entry points use a further 50-sat safety margin. If the resulting limit is zero
  or unavailable, no additional stabilization is offered.
- The client uses its own outbound capacity minus the trade fee. The LSP uses inbound capacity
  **after** receiving the fee. Neither calculation includes punishment reserves or assigns the
  funder's commitment fee to the other peer.
- The helper searches whole-cent additional order amounts and composes fee coverage, native
  availability, existing drift rules, and the buffered backing limit. Manual input and final
  preparation use the same validation; no target is silently reduced.
- Max labels show the **additional gross order amount**, not the resulting total position.

Rust: `src/stabilization.rs`. Android: `StabilizationPolicy.kt`. Swift: the shared
`NotificationService/Services/TradeProtocol.swift`, already compiled into both targets.
The canonical regression vectors are `tests/fixtures/stabilization-limits.json`; Kotlin and
Swift mirror them with explicit source comments.

## Server rollout (separate deployment decision)

The LSP defaults to **shadow mode**. The top-level TOML setting
`enforce_max_stabilization = false` preserves existing client behavior and emits
`MAX_STABILIZATION_REJECTED` with `enforced: false` for would-be rejections. Enforcing mode
emits the same event with `enforced: true`, reuses the deployed `InsufficientCapacity` code,
and rejects before changing the stored allocation. Correlated requests receive a signed
rejection; legacy requests retain their existing response semantics. Manual target increases
use the same policy. No new wire enum variant is introduced.

Observe for four weeks after capped mobile clients ship. Enable only after an explicit operator
decision and **zero cap events in the trailing two weeks**. Set the flag to true and restart the
daemon. Do not infer readiness from the absence of traffic alone.

Rollback: an organic cap rejection at a requested backing of at most 100.5% of the observed
post-fee spendable balance warrants disabling the flag and restarting while investigating.
The event includes the balance, cap, backing, channel, and entry path needed for diagnosis.
Distinguish a dormant old client's full-Max request from a new client's boundary request.
A rollback disables the server backstop globally; updated clients still enforce their limits.

Accepted residual risk: old clients can pay a non-refundable fee and then be rejected after
enforcement starts. The client margin also cannot guarantee acceptance when the two peers see
different balances or prices. No deployed config is changed by this branch.

## Verification

Run Rust/LSP tests, Android unit tests, iOS XCTest, and the pinned SwiftFormat linter.
Tests cover the shared vectors, Max versus the next cent, reserve/rounding boundaries,
changed snapshots, direct preparation, reductions/full exits, both LSP trade paths, manual
edits, shadow/enforcing behavior, and asymmetric reserves with post-payment balances.

Before production rollout, run one **regtest-only** interactive flow on each client:
drag to the limit, press Max, attempt one cent above it, confirm Max, and compare the persisted
allocation on both peers. Repeat with a balance change while the confirmation screen is open,
and with an existing position above the limit. Never use real funds for these checks.

Implementation verification (2026-09-07): Rust library 200 passed (2 ignored), desktop
219 passed (2 ignored), LSP 160 passed, Android 118 passed, iOS 221 passed. Clippy completed
with existing repository warnings; the pinned SwiftFormat lint passed. Changed Rust ranges
were formatted without reformatting unrelated legacy code. The 11 ignored regtest tests and
the three interactive client flows were not run; local regtest services were unavailable.
