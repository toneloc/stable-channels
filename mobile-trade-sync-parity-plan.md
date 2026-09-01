# Plan — Harden the mobile trade handshake (#204)

## Branch and pull request

- **Branch:** `mobile/trade-sync-handshake`
- **PR title:** `Harden Android and iOS trade-result accounting`
- **Base:** a clean worktree from the latest `origin/main`
- **Closes:** [#204](https://github.com/toneloc/stable-channels/issues/204)

Do not build this on the current `autotest` checkout or mix in the unmerged iOS
`fix/authenticated-stability-settlements-ios` branch. That branch contains broad, unrelated
refactors. Port only small pieces after verifying them against the current desktop/LSP contract.

## Outcome

Bring Android and iOS to parity with the current desktop/LSP `TRADE_V1` → signed result
handshake:

1. The mobile wallet validates and durably records a trade before paying the non-refundable fee.
2. The signed `TRADE_V1` binds the current channel, target, quote, timestamp and random trade ID.
3. `PaymentSuccessful` confirms only that the fee moved; it does **not** apply the trade.
4. A signed, correlated `SYNC_V1` is the only accepted result that changes the mobile allocation.
5. A signed `TRADE_REJECTED_V1` records a terminal rejection and shows a useful reason.
6. Every accepted sync is channel-bound, ordered by persisted `sync_version`, atomic, and
   drift-preserving in both foreground and background execution.

Android and iOS must implement the same protocol and state transitions. Platform-specific code
may differ, but the accepted payloads, database states, audit outcomes and tests should match.

## Protocol invariants

These are requirements, not implementation suggestions:

- Sign and verify the exact UTF-8 payload bytes received on the wire. Do not parse and
  re-serialize before signature verification or request hashing.
- Generate `trade_id` as 32 random bytes encoded as 64 lowercase hexadecimal characters.
- Compute `request_hash` as plain, forward-order, lowercase SHA-256 hex of the exact signed
  `TRADE_V1` payload bytes. Do not reverse the digest as a Bitcoin display hash.
- Include `type`, `channel_id`, `user_channel_id`, `trade_id`, `expected_usd`, `quote_price` and
  `ts` in mobile `TRADE_V1`.
- Persist the prepared trade and proposed local allocation before sending its fee.
- Attach the returned Lightning payment ID after the send. A failure to attach it must leave a
  recoverable durable record rather than silently applying or forgetting the trade.
- Treat a trade as unresolved until a valid correlated acceptance or rejection is committed.
- Correlate results by `trade_id`, `trade_payment_id` and `request_hash`; also require the current
  `channel_id`.
- Require accepted/rejected correlated result payments to be exactly 1 msat, the protocol control
  amount used by the current LSP.
- Treat correlated request timestamps as fresh for 15 minutes (`TRADE_RESULT_TIMEOUT_SECS`). The
  five-minute `TRADE_SIG_WINDOW_SECS` applies only to legacy, uncorrelated compatibility traffic.
- Require `sync_version > 0` and strictly newer than the version persisted for that
  `user_channel_id` before changing allocation. Duplicate or older versions never roll allocation
  back; a matched delayed acceptance may still close its trade as accepted, matching desktop.
- Preserve accumulated drift. A trade acceptance uses the locally prepared post-fee allocation;
  a non-trade sync preserves an unchanged target or applies only the locally priced target delta.
- Never blindly adopt the LSP's `backing_sats`. Require it in `SYNC_V1`, compare it with the
  locally derived state, and emit structured divergence telemetry.
- Commit the channel allocation, sync version and trade outcome in one database transaction.
- A missing trusted price, unavailable channel state or database failure must defer event
  acknowledgement so the control payment can be retried. Invalid signatures, malformed payloads,
  wrong channels and replayed terminal results must not mutate accounting.
- A 15-minute local timeout may mark the UI/result as uncertain, but it must not delete the
  correlation record. Retain it for at least the server's 14-day response retry window; the
  server retains response detail for 30 days. A later valid signed response remains authoritative.
- Match the Rust fee calculation exactly. Increases gross up the target delta by `1 / (1 -
  fee_rate)`; decreases do not. Convert valid non-negative finite fee sats by truncating toward
  zero, then multiply by 1,000 and clamp to at least 1 msat. Guard Swift conversions against NaN,
  negative and out-of-range values before constructing `UInt64`; never round to the nearest sat.
- A signed-quote fee may differ from the server's expected fee by at most exactly 1,000 msat in
  either direction. Material overpayment is rejected just like underpayment.

## Scope

### Included

- Hardened `TRADE_V1` creation and local preflight on Android and iOS.
- Durable prepared/sent/fee-paid/accepted/rejected/uncertain trade states.
- Signed correlated `SYNC_V1` acceptance.
- Signed `TRADE_REJECTED_V1` handling and user-facing reason mapping.
- Ordered, drift-preserving uncorrelated `SYNC_V1` for spend, splice and recovery updates.
- Foreground and background/notification-extension handlers.
- Database creation, upgrades, indexes and atomic transition helpers.
- Equivalent Android/iOS fixtures, migration tests and behavior tests.
- Small LSP/Rust fixture or contract tests only if needed to prove byte-level compatibility; no
  server behavior redesign.

### Not included

- Signed `STABILITY_PAYMENT_V1` on mobile.
- Removal of the legacy `[1]` stability marker.
- A two-phase trade protocol or fee refund.
- New sync reason/delta fields for spend, splice or recovery.
- Price-oracle redesign, UI redesign or broad repository/refactor work.
- Adopting the LSP's backing as canonical wallet state.

## 1. Pin cross-platform contract fixtures first

Before changing either app, add compact fixtures that describe:

- one canonical `TRADE_V1` payload and its exact request hash, including this literal byte-order
  check:

  ```text
  payload: {"type":"TRADE_V1","user_channel_id":"7","expected_usd":25.0}
  sha256:  c07dcdff3aae2fc7ebd4fb19a7f1cd60b8e61c94a89acd35c5c600935d671602
  ```

- literal fee-amount vectors generated from Rust `expected_trade_fee_msat`, covering an increase,
  a decrease, sub-cent normalization, an exact-sat boundary and a value just below one;
- one correlated accepted `SYNC_V1`;
- one correlated `TRADE_REJECTED_V1` for each supported reason code;
- one uncorrelated newer sync and one duplicate/older sync;
- wrong-channel, incomplete-correlation, malformed-number and bad-signature cases.

Use the current Rust builders/parsers as the source of truth. Check the same literal payload bytes,
identifiers and expected outcomes into Android and iOS tests. This catches JSON-number, field-name,
hex and hashing differences before they can become live protocol bugs.

Do not require a canonical JSON field order across implementations. Require only that each sender
sign and hash the exact bytes it actually sends, and that the receiver verifies those exact bytes.
This rule is message-specific: `RegisterPushSigned` still requires declaration-order
serialization, so do not generalize this behavior to every signed protocol message.

## 2. Add durable mobile schema and migrations

### Channel state

Add `sync_version INTEGER NOT NULL DEFAULT 0` to `channels` on both platforms. Indexing is not
needed for the single active-channel lookup, but every foreground/background query must address the
row by `user_channel_id` and verify `channel_id`.

### Trade state

Extend `trades` with the minimum durable protocol fields:

- `trade_id` — unique lowercase-hex ID;
- `request_hash` — hash of exact signed payload bytes;
- `request_payload` — exact payload retained for audit/recovery;
- `trade_payment_id` — attached after the fee send;
- `old_expected_usd` and `new_expected_usd`;
- `new_backing_sats` — wallet's prepared drift-preserving allocation;
- `quote_price`, `fee_msat` and `expires_at`;
- `outcome`/`reason_code` and result timestamps.

Use a constrained status model such as:

`prepared → sent → fee_paid → accepted | rejected | uncertain | send_failed`

`uncertain` is locally non-terminal: a later matching signed response may transition it to
`accepted` or `rejected`. Record whether uncertainty means `no_response` or
`response_not_committable` so acknowledgement/retry behavior remains observable. Do not allow two
unresolved trades for the same channel.

### Migration rules

- Android: increment `DatabaseService.DB_VERSION`, make `onUpgrade` additive and preserve all
  existing channel, payment and trade rows.
- iOS: extend the existing `PRAGMA table_info` migration pattern; the main app owns schema
  creation, while the notification extension must fail/defer safely when it encounters an old
  schema during an upgrade race.
- Add uniqueness/index constraints for non-null `trade_id`, `request_hash` and payment ID where
  compatible with legacy rows.
- Test a real version-2 Android database and a representative pre-change iOS database, not only a
  fresh database.

Implement transaction methods rather than distributing SQL across UI/event handlers:

- `recordPreparedTrade`
- `attachTradePaymentId`
- `markTradeFeePaid`
- `applyCorrelatedTradeAcceptance`
- `applyTradeRejection`
- `applyUncorrelatedSyncIfNewer`
- `markExpiredTradesUncertain`

Each result method must be idempotent and compare the stored correlation/channel before mutation.

## 3. Extract pure allocation and parser logic

Replace the mobile `applyTrade` full-repricing call with a pure drift-preserving allocator matching
the current Rust behavior:

- normalize sub-cent targets;
- value the target delta at the wallet's trusted local quote;
- account for the non-refundable fee and live post-fee receiver capacity;
- reject underflow, backing exhaustion, unsafe full exits and target-over-capacity results;
- preserve existing backing drift and native balance;
- use the same rounding rules on Android and iOS.

Replace Android's `Triple<String, Double, String>` and the equivalent loose iOS dictionaries at the
business boundary with typed models:

- `TradeV1`
- `SyncV1`
- `TradeRejectedV1`
- `SignedEnvelope`
- `TradeCorrelation`

Parsing must reject missing/partial correlation, non-finite values, invalid lowercase-hex IDs,
zero/out-of-range versions and unknown rejection reasons. Signature verification happens before
payload interpretation.

## 4. Android implementation

### Send path

Update `TradeService.kt`, trade view models/screens and `DatabaseService.kt`:

1. Obtain the admitted accounting quote and current channel snapshot.
2. Run the shared preflight/allocator before any payment.
3. Generate the payload, trade ID and exact-byte request hash.
4. Persist the prepared trade.
5. Send the fee carrying the signed TLV.
6. Attach the Lightning payment ID and expose the durable pending state to the UI.

Remove the in-memory map as the authority. It may remain only as a view cache derived from SQLite.

### Result path

Update `AppState.kt` so `PaymentSuccessful` only marks the fee paid. It must not call
`StabilityService.applyTrade`, update the stable target, or show the trade as accepted.

Replace `handleSyncMessage` with a signed-result dispatcher:

- correlated `SYNC_V1` → validate, atomically apply stored allocation/version and accept trade;
- `TRADE_REJECTED_V1` → validate, atomically reject and show mapped reason;
- uncorrelated `SYNC_V1` → derive local delta and apply only if version is newer.

Update `StabilityProcessingService.kt` to use the same typed parsing and transactional database
methods. Background code must not maintain a second accounting algorithm in raw SQL. If code
sharing with the main process is impractical, share pure models/calculators and keep equivalent
tests over both adapters.

On startup, rebuild pending UI state from the database and mark expired unresolved rows uncertain;
do not apply them or automatically resend the trade fee.

## 5. iOS implementation

### Send path

Update `TradeService.swift`, `Trade.swift`, Buy/Sell views and the repository layer using the same
ordering as Android: preflight → exact payload/hash → durable prepared row → send → attach payment
ID. Replace `pendingTradePayments` as the source of truth with repository-backed state.

`PaymentSuccessful` in `AppState.swift` marks the fee paid only. It must not call
`StabilityService.applyTrade` or complete the trade.

### Result path

Create one typed signed-control parser shared in behavior by the app and Notification Service
Extension. Update:

- `AppState.handleSyncMessage`
- `StableControlParser`
- `PaymentDatabase` protocol
- `SQLitePaymentDatabase`

The main app and extension must call the same transaction semantics for accepted, rejected,
duplicate, stale, retryable and invalid results. The extension must use the trusted-price rules
already shared through the app group and return `deferToForeground` when it cannot safely derive or
commit the allocation.

Add any new Swift files to both the Xcode project and `project.yml` deliberately. Do not run
XcodeGen over the existing hand-maintained project as an incidental rewrite.

On launch/foreground recovery, reload unresolved trades, reconcile any known LDK payment outcome,
and keep late signed responses applicable.

## 6. Tests

Add equivalent Android and iOS suites for:

### Wire contract

- exact payload signature verification and request hash fixture;
- literal forward-order SHA-256 fixture matches on Rust, Kotlin and Swift;
- all required hardened trade fields;
- complete correlation required together;
- accepted and rejected response parsing;
- invalid signature/channel/amount/version/identifier rejection.

### Allocation

- literal fee-amount vectors match Rust on Kotlin and Swift for increase, decrease,
  sub-cent normalization and sat-boundary cases;
- signed-quote fee tolerance accepts exactly ±1,000 msat and rejects values beyond it;
- increase and decrease preserve pre-existing drift;
- unchanged target leaves backing unchanged;
- safe full exit and settlement-required full exit;
- fee-aware capacity boundary and rounding edges;
- LSP `backing_sats` divergence audits but does not overwrite local state.

### Persistence and ordering

- fresh database and upgrade from the current production schema;
- prepared intent exists before the send seam is invoked;
- `PaymentSuccessful` does not change allocation;
- correlated acceptance updates trade, channel and `sync_version` atomically;
- rejection leaves allocation unchanged and is idempotent;
- duplicate/stale sync is a no-op;
- newer uncorrelated sync applies the local delta once;
- database failure defers acknowledgement and retry succeeds exactly once;
- restart reconstructs pending/uncertain state and accepts a late response;
- foreground/background processors produce the same database result.

### LSP compatibility

- current LSP accepts hardened Android and iOS fixtures;
- accepted responses carry exact stored correlation;
- rejection reason codes map consistently on both apps;
- legacy mobile traffic remains accepted during rollout.

## 7. Verification

Run locally before pushing:

```text
cd android
./gradlew --no-daemon testDebugUnitTest

xcodebuild test \
  -project ios/StableChannels/StableChannels.xcodeproj \
  -scheme StableChannels \
  -destination 'platform=iOS Simulator,OS=26.2,name=iPhone 17' \
  -parallel-testing-enabled NO

cd ios
swiftformat --lint . --strict
```

Also run the relevant Rust/LSP tests if contract fixtures or server code change.
If Rust fixtures or server code are touched, add or enable a pull-request CI job that runs
`cargo test --locked` and `cargo build --locked`; local-only Rust verification is insufficient.

Confirm that every new iOS test source is a member of the Xcode test target and appears in the
`xcodebuild test` log. Presence on disk or in `project.yml` alone does not count as execution.

Use the local E2E stack for one accepted and one rejected trade per platform. Assert database and
audit state, not only UI text. Then perform the physical-device checks that automation cannot
prove:

- Android foreground and background delivery;
- iOS foreground and Notification Service Extension delivery;
- app killed/restarted while fee is paid but result is pending;
- delayed and replayed response;
- stale price and stale Lightning-sync deferral;
- device upgrade with existing channel/trade history.

No production or meaningful-value mainnet trade should be used until regtest/simulator and small
controlled test cases are green.

## 8. Rollout and compatibility

- Keep the LSP's legacy mobile compatibility path during the release window.
- Add audit counters for hardened versus legacy mobile trade requests before the first mobile
  release. Missing `trade_id` silently selects the legacy parser path, so this counter must make a
  malformed hardened client visible. Also count accepted/rejected/uncertain outcomes,
  stale/duplicate syncs and allocation divergence.
- Release Android and iOS with the same protocol behavior before tightening the server.
- Watch result-delivery retries, rejection rates, sync-version conflicts and unresolved trade age.
- Remove legacy trade tolerance only in a separate change after deployed-version telemetry shows
  legacy use is no longer significant.
- Signed mobile stability settlement and legacy `[1]` removal remain a separate follow-up.

## Commit sequence

Keep reviewable commits in this order on the requested single feature branch. Treat the Android
and iOS protocol transitions as independent review units even though they ship in one PR:

1. `test: pin mobile trade-result protocol fixtures`
2. `db: add durable mobile trade and sync state`
3. `android: harden TRADE_V1 and signed result handling`
4. `ios: harden TRADE_V1 and signed result handling`
5. `test: cover mobile migrations, replay, recovery and LSP compatibility`
6. `docs: describe the hardened mobile trade handshake`

Do not merge a commit state that sends hardened correlated `TRADE_V1` while still applying the
trade on `PaymentSuccessful`. If intermediate commits cannot compile and test safely on their own,
squash the protocol transition per platform before review.

## Definition of done

- Android and iOS send the complete hardened `TRADE_V1` payload.
- A literal cross-language fee-amount vector table passes in Rust, Kotlin and Swift.
- The request-hash fixture asserts
  `c07dcdff3aae2fc7ebd4fb19a7f1cd60b8e61c94a89acd35c5c600935d671602`.
- Correlated freshness is tested and documented as 15 minutes; the five-minute legacy window and
  issue #234 are documented accurately.
- The fee payment never applies the target by itself.
- Both platforms accept only correctly signed, channel-bound, correlated and ordered trade results.
- Accepted trades use the prepared local drift-preserving allocation.
- Rejections are durable, idempotent and understandable to the user.
- Foreground and background paths share the same state-transition rules.
- Production database upgrades preserve existing wallet history and channel state.
- All Android, iOS and affected Rust/LSP tests pass.
- New iOS tests are members of the Xcode test target and visibly execute under `xcodebuild test`.
- LSP hardened-versus-legacy request counters are live before the first mobile release.
- Physical-device results and audit evidence are attached to the PR before it is marked ready.
- The PR contains no signed-stability work, legacy-marker removal or unrelated refactor.
