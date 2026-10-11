# Aux E2E flows — implementation notes (gathered 2026-09-02, branch `autotest` @ d441ea40)

All paths absolute from repo root `/Users/t/Code/stable-channels`. Copy strings are exact;
Maestro regexes shown escaped where relevant.

---

## 1. Splice-out restart recovery (PR #252, Android)

### Android mechanism
- **Initiation persists BEFORE the native call**: `AppState.beginSpliceOut()`
  `android/.../AppState.kt:1414-1437` — `db.recordPayment(paymentType="splice_out",
  direction="sent", status="pending", address=...)` (row has **no txid yet**), then
  `isSweeping = true`, `pendingSplice = PendingSplice("out", ...)`,
  `_statusMessage = "Move pending..."` (line 1436).
- LDK `SplicePending` event → `handleSplicePending()` `AppState.kt:1390-1412`:
  stamps txid onto the row via `DatabaseService.assignPendingSpliceTxid()`
  (`DatabaseService.kt:1359`), sets `_statusMessage = "Move pending confirmation"`,
  starts `startSpliceConfirmationMonitor(txid)` (polls `<chainUrl>/tx/<txid>/status`).
- **Kill + relaunch**: startup path `AppState.kt:408` (also foreground-restart paths 630, 653)
  calls `resumePendingSpliceConfirmation()` (`AppState.kt:1467-1472`): if
  `databaseService.hasPendingSplice()` → `isSweeping = true`, reload txid from DB, restart
  the confirmation monitor. On confirmation `completeConfirmedSplice()` (`AppState.kt:1494-1522`)
  → `DatabaseService.completeSplice(txid)` (`DatabaseService.kt:1420-1444`, sets
  `status='completed', confirmations=1`), `_statusMessage = "Move confirmed"`, audit
  `SPLICE_CONFIRMED`.
- `isSweeping` is **not a blocking UI state** — it renders as a home-card pending row and
  blocks *new* splices/trades (`AppState.kt:2372` guards stability/trade paths;
  `beginSpliceOut` throws `"A splice is already in progress — try again shortly"`).

### Exact user-visible copy (Android)
- (a) right after initiation — SendScreen path (flow 07): `"Splice-out initiated"`
  (`ui/transfer/SendScreen.kt:728`, unchanged — flow 07's assertion still matches).
  OnChainScreen variant: `"Splice-out initiated for X,XXX sats."` (`OnChainScreen.kt:323`).
  Home status capsule: `"Move pending..."` → after SplicePending event `"Move pending confirmation"`.
  Home onchain card while `isSweeping`: `PendingRow("Move pending...", spliceTxid)`
  (`ui/home/HomeScreen.kt:353` — but card only rendered when `onchainSats > 0`, line 326;
  the status capsule at `HomeScreen.kt:457-462` shows regardless).
- (b) payment history pending row (`ui/history/HistoryScreen.kt`):
  title `"Sent"`, subtitle `"Splice Out · <relative time>"` (type label line 252), amount
  `-$X.XX`, status badge `"0/1 confirmed"` (`historyStatusLabel()` lines 335-345;
  `SPLICE_REQUIRED_CONFIRMATIONS = 1`, line 36). Detail dialog type: `"Splice Out"`
  (`PaymentDetailDialog.kt:92`).
- (c) after confirmation: status badge `"Confirmed"` (confirmations 1/1), status capsule
  `"Move confirmed"` (`AppState.kt:1516`).

### iOS comparison
- **iOS does NOT persist at initiation.** `beginSpliceOut()` `ios/.../App/AppState.swift:2144-2155`
  is in-memory only (`pendingSplice` struct + `statusMessage = "Move pending..."`).
  The `splice_out` payment row is recorded only inside `handleSplicePending()` (LDK
  `spliceNegotiated` event) `AppState.swift:2090-2142`, with the txid, status "pending";
  `statusMessage = "Splice pending"` (line 2140). A restart replay stamps the latest
  NULL-txid row via `spliceRepo.setPendingSpliceTxid` (line 2135 — comment explicitly says
  "pendingSplice is in-memory and lost across relaunch").
- iOS **does** have restart resume: `resumePendingSpliceConfirmation()` `AppState.swift:2187-2194`
  (called at 698/903/922) via `SpliceRepository` (`Services/Repositories/SpliceRepository.swift`;
  `hasPendingSplice()` also expires >600s-old NULL-txid rows to `failed`, lines 32-49).
  On confirm: `"Move confirmed"` (`AppState.swift:2252`), `completeSplice` sets
  `status='completed', confirmations=1`.
- So: kill **before** `spliceNegotiated` on iOS → no DB row → nothing resumes (gap vs Android);
  kill **after** → same recovery as Android.
- iOS copy: SendView success section shows `"Payment sent: $X.XX"` / `"Payment sent"`
  (`Features/Transfer/SendView.swift:371` — flow 07 iOS asserts `Payment sent.*`);
  OnChainView success title `"Splice-out initiated!"` (`OnChainView.swift:74`);
  pre-send route hint `"Will route via splice-out"` (`SendView.swift:345`); home pending row
  `"Move pending..."` (`Features/Home/HomeView.swift:424`, key `status_sweeping`);
  history/detail type `"Splice Out"` (`PaymentDetailView.swift:183`).

### E2E implications
A new "kill during pending splice-out" flow is **deterministic on Android**: do flow-07 steps,
wait for `Move pending.*`, `stopApp` + `launchApp`, assert `"Move pending.*"` reappears
(resume), mine 6 (`helpers/mine_blocks.js`), assert `"Move confirmed"` and history badge
`Confirmed`. On iOS it is deterministic only if the kill happens after `spliceNegotiated`
(wait for `Move pending.*` first, which is set at initiation... use history row presence or a
short wait to be safe).

---

## 2. Stale-tip / pre-sync payment gate (PR #243)

- **Scope: stability payments ONLY.** User-initiated sends (SendScreen/SendView, trades,
  splices) are NOT gated. The gate = LDK `status().latest_lightning_wallet_sync_timestamp`
  must be ≤ **120 s** old (`STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS = 120`:
  `src/constants.rs:83`, `android/.../util/Constants.kt:74`; iOS
  `Constants.stabilityMaxLightningSyncAgeSecs` used by `StabilityFreshness`,
  `ios/.../Services/NodeService.swift:635-660`). Missing or future timestamp also blocks.
- The 5 send boundaries:
  1. Android foreground: precheck `AppState.kt:1867-1877` + wrapper
     `NodeService.sendStabilityPayment()` `services/NodeService.kt:279-287`
     (throws `StaleLightningSyncException`).
  2. Android background push: `push/StabilityProcessingService.kt:583-625`
     (waits in 500 ms polls, then defers to foreground).
  3. iOS foreground: `App/AppState.swift:2357-2394` + wrapper
     `Services/NodeService.swift:516-526` (`NodeServiceError.staleLightningSync`).
  4. iOS NSE: `ios/StableChannels/NotificationService/Services/UserToLSPHandler.swift:118-190`.
  5. Desktop/LSP shared Rust: `src/stable.rs:1003-1020` (`lightning_sync_is_fresh`,
     `src/stable.rs:794`); LSP `run_tick` shares constants.
- **User-facing message when blocked: NONE, on all three platforms.** The skip is silent —
  no banner, no toast. Evidence is audit-log only:
  - Android/iOS app audit log (`audit_log.txt` in the app data dir): event `STABILITY_SKIP`
    with `{"reason": "stale_lightning_sync", "sync_age_secs": N}`.
  - Desktop Rust: `STABILITY_SKIP` with `latest_lightning_wallet_sync_timestamp`,
    `max_age_secs` (`src/stable.rs:1009-1019`).
  - Android background metric line: `STABILITY_GATE {...}` logcat +
    `stability_background_deferred_stale_sync` (`StabilityProcessingService.kt:600,624,701`);
    iOS NSE logs the same outcome string.
  - The only user-visible *string* that exists is iOS's error description
    `"Lightning wallet chain sync is too old to safely pay"`
    (`NodeService.swift:630`) — but every caller catches it, so it never reaches UI.
- **Determinism**: no `test_config.json` key overrides the 120 s threshold (see §7 — not
  overridable). To make the tip stale: `docker stop block-explorer` (compose service +
  `container_name: block-explorer`, `e2e/harness/docker-compose.yml:63-64`) **while the app
  is foregrounded and the node already started**, then wait > 120 s (e2e
  `sync_interval_secs: 3` means the last-sync timestamp freezes almost immediately). Then
  create a stability-drift condition (price move) and assert the app audit log gains
  `STABILITY_SKIP`/`stale_lightning_sync` and that **no** settlement lands (LSP-side:
  absence of `STABILITY_PAYMENT_V1_SENT` in `/audit-tail` for the `user_to_lsp` direction
  requires the *user* to be the payer, i.e. price ABOVE par after a sell). Caveat: stopping
  block-explorer also stalls the LSP's own sync — LSP-side sends hit the same shared gate
  (`src/stable.rs`), which is itself testable via `/audit-tail` `STABILITY_SKIP`.
- **Cold start with chain source down does NOT reach home**:
  - Android: retryable startup failure → `Phase.SYNCING` + status
    `"Network unstable. Retrying wallet sync..."` (`AppState.kt:676-680`; retryable set
    `AppState.kt:691-710`). SyncingView copy: `"Syncing wallet..."` / `"This may take a moment"`
    (`ui/ContentView.kt:90-92`).
  - iOS: launch probe `resolveChainURL()` (2 s timeout) picks fallback; if `node.start`
    fails on both → `phase = .error` (`AppState.swift:2865-2874`). So the "cached home"
    scenario requires killing the chain source *after* startup, not before.

---

## 3. iOS Esplora failover (PR #242)

- Config: `Constants.primaryChainURL` / `Constants.fallbackChainURL`
  (`ios/.../Utilities/Constants.swift:21-25`), overridable in debug builds via
  `test_config.json` keys **`primary_chain_url`** and **`fallback_chain_url`**
  (`Utilities/TestOverrides.swift:41-42`). `e2e/harness/push-test-config-ios.sh` currently
  writes both to `http://localhost:30000`; a new flow's `.pre.sh` can write primary = dead
  URL (e.g. `http://localhost:1/api`) and fallback = `http://localhost:30000` by generating
  the JSON itself (the stock script offers no per-URL env hooks — only `HARNESS_HOST`
  applies to both).
- Two failover layers (`AppState.swift:2803-2914`):
  1. **Launch probe** `resolveChainURL()` (line 2886): GET `<primary>/blocks/tip/height`,
     2 s request timeout → dead primary deterministically selects fallback; audit
     `CHAIN_SOURCE_FALLBACK` (or `CHAIN_SOURCE_RESOLVED`).
  2. **node.start failover** `startNodeOrFailover()` (line 2825): only fires for typed LDK
     `FeerateEstimationUpdateFailed/Timeout` (`NodeService.swift:670-681`); audit
     `NODE_START_INITIAL_FAILED` then `NODE_START_FAILOVER_SUCCESS` / `NODE_START_FAILOVER_FAILED`.
     A plain dead URL is caught by layer 1, so layer 2 needs a host that answers the tip
     probe but fails `/fee-estimates` — the harness has no such mode; **layer 2 is not
     deterministically e2e-testable today.**
- **No user-visible signal**: success via failover is just the normal home screen. Evidence
  is `audit_log.txt` in the app container
  (`xcrun simctl get_app_container booted com.stablechannels.app data` → `Documents/../audit_log.txt`
  — path set at `AppState.swift:331`), events `CHAIN_SOURCE_FALLBACK` /
  `NODE_START_FAILOVER_SUCCESS`. The working URL is published to the app group as
  `esplora_chain_url` (line 2880).
- Android has the equivalent launch probe (`AppState.kt:2256-2295`, audit
  `CHAIN_SOURCE_FALLBACK`) but no node.start-level failover.

---

## 4. Android quick-switch deferred stop (PRs #250/#251/#252)

- Mechanism: `MainActivity.onPause` → `AppState.stopNodeForBackground()`
  (`AppState.kt:515-540`) which **defers the actual node stop by
  `QUICK_SWITCH_GRACE_MS = 10_000`** (`AppState.kt:62`). `onResume` →
  `AppState.cancelBackgroundStop()` synchronously on main (`MainActivity.kt:95-120`),
  then `restartNodeFromForeground()` (`AppState.kt:613-661`): if the node is still running
  (returned within 10 s) it only reconnects/refreshes — **no phase change, no resync UI**.
  If the node did stop (away > 10 s): `_phase.value = Phase.SYNCING` (line 640) →
  full-screen SyncingView until `nodeService.start` returns.
- **Regression signature ("resync flash")** on a < 10 s background/foreground cycle:
  the full-screen SyncingView — exact copy `"Syncing wallet..."` and
  `"This may take a moment"` (`ui/ContentView.kt:90-92`, regex `Syncing wallet\.\.\.`).
  Secondary indicator: home top bar row `"Syncing..."` (`ui/home/HomeScreen.kt:317`,
  shown while `isSyncing`). (Phase.ONBOARDING also renders SyncingView; the post-#250
  activation copy in onboarding is separate — flow 01 was updated for it in 1a73c318.)
- **Proof of NO resync** (normal home): action buttons `"USD → BTC"` and `"BTC → USD"`
  (`ui/home/HomeScreen.kt:453-454` — real arrow char `→`, same labels both platforms), and
  the absence of `Syncing wallet...` throughout.
- **Maestro backgrounding**: yes — flow 15 does exactly this on Android:
  `- pressKey: Home` then later `- launchApp` *without* `stopApp: true` resumes the same
  process (`e2e/flows/15_trade_rejected_in_background.yaml:50-71`). iOS has no Maestro Home
  key — flow 15 substitutes `stopApp` (a kill), so a "quick-switch without kill" flow is
  **Android-only**. Suggested flow: launch → assert `BTC → USD` → `pressKey: Home` →
  `wait_secs 3` (< 10 s) → `launchApp` (no stopApp) → `assertNotVisible: "Syncing wallet\.\.\."` +
  `assertVisible: "BTC → USD"` within a tight timeout (the flash is transient — use
  `extendedWaitUntil: visible: "BTC → USD"` with a short timeout plus the notVisible check
  immediately after resume).

---

## 5. Stability drift preserved across trades (PR #225) + settle-by-amount (#231/#233)

### Thresholds & mechanics
- LSP tick (`server/stable-channels-lsp/src/stable_manager.rs:1706+`, tick interval
  **5 s** in e2e builds / 60 s prod, `src/constants.rs:91-96`): value =
  `backing_sats * price`; skips when `percent_from_par < 0.1` **OR**
  `dollars_from_par < 0.25` (lines 1820-1823; constants `src/constants.rs:115-116`).
  Both must be met to settle. Cooldown 120 s (`STABILITY_PAYMENT_COOLDOWN_SECS`,
  `src/constants.rs:119`). Settlement amount = **full cumulative** `dollars_from_par`
  converted at the current price (line 1863: `amount_sats = (dollars_from_par/price)*1e8`).
  After a settlement the LSP rebuilds backing to par at the settlement price (lines
  1968-1969, 2028) — start drift experiments from a settled baseline.
- Trades preserve drift by design: `apply_trade` (`src/stable.rs:474-487`) converts only
  the target **delta** at the current price and adds/subtracts it from the existing
  `backing_sats` (`trade_backing_after_delta`, `src/stable.rs:381-441`) — the pre-trade
  gap between backing value and expected_usd carries through. A delta that cannot
  preserve drift is **rejected** (`TRADE_ALLOCATION_REJECTED`, reason
  `"target delta cannot preserve the current stability drift"`, `stable_manager.rs:3071-3084`;
  hardened path rejects with `SettlementRequired`/`UnsafeAllocation`, lines 2824-2840).

### Concrete numbers ($75 sell at $100k base → expected $74.25, backing 74,250 sats; 1% fee `STABLE_CHANNEL_TRADE_FEE_RATE = 0.01`)
1. Price → **$99,700** (−0.3%): value = 74,250 × 99,700/1e8 = $74.02725; drift $0.22275
   (< $0.25 while 0.3% ≥ 0.1%) → **no settlement** (dollar threshold binds; the
   sub-threshold window at this position size is any move < $0.25/$74.25 ≈ 0.337%).
2. Sell another **$10** at 99,700 (net +$9.90): new expected = $84.15; delta sats =
   floor(84.15/99700·1e8) − floor(74.25/99700·1e8) = 84,403 − 74,473 = 9,930 →
   backing = **84,180** sats. `TRADE_ACCEPTED` audit shows `expected_usd: 84.15,
   backing_sats: 84180` — a **reset** implementation would show 84,403
   (= floor(expected/price·1e8)); that single field distinguishes preserved vs reset.
3. Price → **$99,500**: value = 84,180 × 99,500/1e8 = $83.7591; drift **$0.3909**
   (0.465%) → both thresholds met → LSP pays **≈ 392 sats = 392,000 msat** lsp_to_user
   (cumulative drift from the pre-trade mark). Under drift-reset the drift would be only
   $0.169 → **no settlement at all**. So the assertion is simply "a settlement of
   ~392,000 msat fired"; use tolerance ±8,000 msat like flow 03.

### Audit events proving preservation vs reset (via `HARNESS_API/audit-tail`)
- `TRADE_ACCEPTED` (hardened path, `stable_manager.rs:2898-2907`: `expected_usd`,
  `backing_sats`, `trade_id`, `sync_version`) or `TRADE_APPLIED` (legacy path, line 3113:
  `new_expected_usd`, `backing_sats`, `quote_price`, `lsp_price`).
- Settlement: **`STABILITY_PAYMENT_V1_SENT`** with `payment_id`, `settlement_id`,
  `amount_msat`, `direction: "lsp_to_user"` (`stable_manager.rs:1975-1985`).
  ⚠ **`e2e/flows/helpers/assert_lsp_stability_payment.js:39` still matches
  `ev.event === 'STABILITY_PAYMENT_SENT'`, but the LSP now emits
  `STABILITY_PAYMENT_V1_SENT`** — new flows should match the V1 name (and fix/copy the
  helper); `STABILITY_PAYMENT_SENT` today only exists as a desktop/GUI ledger event type
  (`src/db.rs:2552,3880`).
- Negative assertion between steps 1-2: no `STABILITY_PAYMENT_V1_SENT` after the mark
  (reuse `set_price_and_mark.js` → `output.settlementAfterIso`); LSP quiet-skip is
  throttled (`stability_should_log`) so absence of events is expected, not an error.
- Client-side gotcha: the app itself may settle first in the `user_to_lsp` direction only
  when above par; below-par moves (as here) are LSP-paid, so the app can't preempt.

---

## 6. Restore guard (PR #174) — iOS AND Android

- **Both platforms have it.**
  - iOS: `AppState.restoreWalletFromMnemonic()` `ios/.../App/AppState.swift:340-395` —
    derives node id from the entered mnemonic, calls `lspChannelExists(nodeId:)`
    (`AppState.swift:550`, POSTs to `Constants.lspChannelExistsURL`). true →
    `WalletRestoreError.activeChannelDetected`; unreachable/derive-fail →
    `.channelCheckUnavailable` (fail-warn, not fail-open). UI:
    `Features/Settings/Backup/RestoreSeedSheet.swift:96-124, 240-248` — alert title
    `"Open Channel Detected"`, message
    `"This wallet still has an open Lightning channel with the LSP. Restoring from seed alone cannot restore the channel and it will be force-closed on-chain; funds return after a timelock. Only continue if this is your only way back into the wallet."`,
    buttons `Cancel` / `Restore Anyway`. Unavailable variant: title
    `"Couldn't Verify Channel Status"`, button `Continue Anyway`. Error-string variant
    (AppState.swift:244-246): `"This wallet still has an open Lightning channel with the LSP. Restoring from seed alone will force-close it on-chain."`
  - Android: `ui/settings/BackupView.kt:347-466` (duplicate in `SettingsScreen.kt:390+`) —
    same dialog copy: title `"Open Channel Detected"` / `"Couldn't Verify Channel Status"`,
    body ends `"...Only continue if this is your only way back into the wallet."`, buttons
    `Cancel` / `Restore Anyway` (or `Continue Anyway`). Uses
    `NodeService.deriveNodeId` + `appState.lspChannelExists` (`AppState.kt:2236`).
- **Applies in e2e**: the endpoint is `test_config.json` `channel_exists_url` →
  `http://<host>:3002/api/channel-exists`, served by the harness `sc-lsp`
  (`server/stable-channels-lsp/src/main.rs:269`, handler
  `handlers/channel_exists.rs:38`). Debug builds honor the override on both platforms.
- **Triggerable flow**: after flow 01 (channel open), Settings → back-up → reveal/copy seed
  (flow 10 selectors), then open the restore/import sheet and enter the SAME seed →
  guard fires deterministically (`assertVisible: "Open Channel Detected"` → `tapOn: "Cancel"`).
  Audit: `RESTORE_ACTIVE_CHANNEL_DETECTED` / `RESTORE_GUARD_UNAVAILABLE` on both platforms.
  Negative variant: stop `sc-lsp` container → `"Couldn't Verify Channel Status"`.
  Note flow 11 today runs post-close with `RESTORE_SEED`, deliberately avoiding the guard.

---

## 7. test_config.json override surface (Android + iOS — identical key sets)

Parsers: Android `android/.../util/TestOverrides.kt:54-82` (file:
`/sdcard/Android/data/com.stablechannels.app/files/test_config.json`, DEBUG builds only);
iOS `ios/.../Utilities/TestOverrides.swift:29-60` (file: app `Documents/test_config.json`,
`#if DEBUG` only). **Every key below exists on BOTH platforms; there are no
platform-exclusive keys.**

| key | type | consumed at |
|---|---|---|
| `network` | string ("regtest") | Constants.DEFAULT_NETWORK / defaultNetwork |
| `primary_chain_url` | string | Constants.kt:34 / Constants.swift:21 |
| `fallback_chain_url` | string | Constants.kt:36 / Constants.swift:25 |
| `lsp_pubkey` | string | Constants.kt:38 / Constants.swift:51 |
| `lsp_address` | string host:port | Constants.kt:40 / Constants.swift:55 |
| `push_register_url` | string | Constants.kt:28 / Constants.swift:37 |
| `channel_exists_url` | string | Constants.kt:30 / Constants.swift:41 |
| `price_feed_base` | string | replaces ALL five price feeds with `<base>/feeds/*`; also empties USDT cross-feeds (Constants.kt:88-100 / Constants.swift:126-146) |
| `disable_send_auth` | bool | Android BiometricService.kt:84; iOS AppState.swift:59-61 |
| `sync_interval_secs` | int > 0 | all LDK sync cadences (Constants.kt:51-65 / Constants.swift:68-88) |
| `price_refresh_secs` | int > 0 | price auto-refresh loop (AppState.kt:357 / AppState.swift:655); used by flows 13/15 pre-hooks via `SC_TEST_PRICE_REFRESH_SECS` env into `push-test-config*.sh` |

**Not overridable** (hardcoded): stale-tip threshold (120 s), mempool websocket URL
(`wss://mempool.space/api/v1/ws` — Android
`services/websocket/MempoolWebSocketService.kt:15`, iOS
`Services/WebSocket/MempoolWebSocketService.swift:120`; harmless in regtest — it just
never reports), trade fee rate, stability thresholds/cooldown. Config is loaded **once at
process start** — every change needs a force-stop/terminate + relaunch (the pre/post hook
pattern in `13_trade_rejected.pre.sh`).

---

## 8. Fee visibility (PR #194) + actual fee paid (PR #216)

### Trade review sheet ("Review BTC -> USD" — note ASCII `->` in the sheet title, vs `→` on the home buttons)
- Android `ui/trade/SellScreen.kt:198-213` (BuyScreen mirror): rows
  `"Amount"` / `"Fee (1%)"` (feeLabel = `String.format("Fee (%.0f%%)", 0.01*100)`, line 49)
  / `"BTC Price"` / `"You receive"` — values `usdFormatted()` e.g. `$0.75`. Confirm button
  `"Confirm Order"`. Post-confirm status: `"Order pending (fee: $0.75)"`
  (`SellScreen.kt:243`, format `Order pending \(fee: \$\d+\.\d{2}\)`).
- iOS `Features/Trade/SellView.swift:139-193`: same rows `"Amount"`, `"Fee (1%)"`,
  `"BTC Price"`, `"You receive"` (value `"$X.XX USD"`); status
  `"Sell pending (fee: $0.75)"` / `"Buy pending (fee: $0.20)"` (SellView:321 / BuyView:316).

### Send-screen fee estimates (PR #194)
- Android `ui/transfer/SendScreen.kt:221-237`: lightning —
  `"Expected fee: none"` / `"Expected fee: ~1 000 BTC ($1.00)"` (spaced-sats format) /
  `"Expected fee: depends on amount"`; onchain —
  `"Expected network fee: ~<sats> BTC (<rate> sat/vB)"` / `"Estimating network fee..."`.
- iOS `Features/Transfer/SendView.swift:111-159`: row label `"Fee"` (onchain row label
  `"Network fee"`), values `"~$X.XX (<sats> BTC)"` / `"No fee expected"` / `"~<sats> BTC"`;
  onchain `"~<sats> BTC (<n> sat/vB)"` / `"Estimating..."`.

### Actual fee after a Lightning send (PR #216)
- **Android**: success status/banner appends the paid fee:
  `"Payment sent: $5.00 (fee: 1 sats)"` (`AppState.kt:1304-1307`; suffix format exactly
  `" (fee: <spaced sats> sats)"`, omitted when LDK reports no fee). Regex:
  `Payment sent.*\(fee: .* sats\)`.
- **iOS**: the status message does NOT include the fee (`sentPaymentStatusMessage`,
  `AppState.swift:963-975` → `"Payment sent: $X.XX"`). The actual fee appears only in the
  payment detail sheet: row `"Fee"` → `"<N> msat"` (`Features/History/PaymentDetailView.swift:71-72`).
  Android detail dialog shows `"Fee"` → `"<N> sats"` (`PaymentDetailDialog.kt:112-114`).

---

## Deterministically testable vs not (summary)

| Scenario | Deterministic? |
|---|---|
| 1. Android splice-out kill/relaunch recovery | YES (kill any time after "Splice-out initiated") |
| 1b. iOS same | Only if killed after `spliceNegotiated`; pre-event kill loses the row (known gap) |
| 2. Stale-tip gate | Partially — `docker stop block-explorer` + >120 s wait works, but the block is **silent**; assertions must read audit logs (app `audit_log.txt` via adb/simctl, LSP `/audit-tail`), not UI. No threshold override exists. |
| 3. iOS esplora failover layer 1 (launch probe) | YES via custom test_config (dead primary + harness fallback); success signal = normal home + `CHAIN_SOURCE_FALLBACK` in app audit log |
| 3b. layer 2 (node.start feerate failover) | NO (needs a half-broken esplora the harness lacks) |
| 4. Quick-switch no-resync | YES, Android only (`pressKey: Home` + `launchApp` w/o stopApp, <10 s); iOS Maestro can't background without killing |
| 5. Drift preservation across trades | YES with the 99,700 → trade → 99,500 sequence; assert via `TRADE_ACCEPTED.backing_sats` + `STABILITY_PAYMENT_V1_SENT` (~392,000 msat). ⚠ helper matches the old `STABILITY_PAYMENT_SENT` name. |
| 6. Restore guard | YES on both platforms (same-seed import after onboard → "Open Channel Detected") |
