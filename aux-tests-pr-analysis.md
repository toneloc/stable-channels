# Closed-PR analysis for E2E regression coverage

Date: 2026-09-02 · Branch: `autotest` (main merged in) · Scope: merged PRs #169–#252.

Legend per line: `#N — behavior — [already-covered by flow X | test-worthy | not-e2e-testable | no-behavior-change]`.
"not-e2e-testable" means: not testable in the current Maestro + regtest-harness model (push/NSE
delivery, upgrade paths, visual layout, adversarial wire input) — most of these are better held
by unit tests, which the PRs generally added.

Existing coverage baseline: flows 01–12 (lifecycle), 13 (quote-deviation rejection), 14 (trade
result survives kill/restart), 15 (rejection while backgrounded). Harness: /pay /invoice /address
/send /mine /price. test_config overrides: network, primary/fallback chain URLs, lsp pubkey/addr,
price_feed_base, push_register_url, channel_exists_url, disable_send_auth, sync_interval_secs,
price_refresh_secs.

## Per-PR verdicts

- #252 — Android splice-out txid tracking — **test-worthy**. Flow 07 only asserts "Splice-out
  initiated"; nothing asserts the pending-splice row gets its txid assigned, survives a kill/restart
  (`resumePendingSpliceConfirmation`), and completes after mining — the exact class #252's strict
  `assignPendingSpliceTxid` reworked, where a regression yields a permanent "Failed" row for a splice
  that confirmed on-chain (and a skipped `reconcileOutgoing`). Android (iOS splice monitor shares the
  class via #174).
- #251 — Android quick-switch node-stop grace — **test-worthy**. The P1 race found in review
  (cancel nulling the join handle) would have produced a *silent dead node* after resume — wallet UI
  attached to a stopped node, no payments, no stability ticks. A flow that backgrounds 1–3s, resumes,
  asserts no "Syncing wallet…" flash AND immediately receives a payment (proves the node is actually
  alive), then backgrounds >10s and asserts one clean restart, pins both sides of the grace window.
  Android only. (Note: with the 10s grace, flow 15's Android leg no longer exercises LSP durable
  retry. CORRECTION on review: flipping it to `stopApp` would race the LSP's sub-second rejection
  — the same race flow 15's iOS comment documents — and make the message assert flaky. Flow 15's
  comments were instead updated to reflect what it now tests (outcome committed during the grace
  window, then rehydrated across the post-grace node restart); stopped-node redelivery stays
  covered by flow 14's acceptance path.)
- #250 — Android node-start serialization + activation copy — **already-covered by flow 01**
  (autotest commit 1a73c318 already updated the copy assertion; the start-race itself is a
  concurrency window Maestro can't hit deterministically — flow 01 catches a gross regression).
- #249 — iOS project.yml/QRCode dependency sync — **no-behavior-change** (build config).
- #248 — mobile signed trade-result accounting — **already-covered by flows 02, 13, 14, 15**
  (these flows were built for this work: rejection surfaced + allocation unchanged, durable-retry
  redelivery after kill, background rehydration). Residual not-covered: the strict allowed-keys
  rejection parser dropping payloads after a benign server field addition — a server-skew scenario,
  keep as the flagged server-side contract note.
- #247 — Android version bump — **no-behavior-change**.
- #242 — iOS Esplora startup failover + lock ownership — **test-worthy**. If failover regresses
  (or the wallet-dir lock leaks on a failed inner start), the app fails to start whenever the primary
  Esplora is down — an onboarding/startup-death UX break. test_config already carries separate
  `primary_chain_url`/`fallback_chain_url`: point primary at a dead port, fallback at :30000, assert
  the app reaches Home. iOS (Android/desktop have their own fallback plumbing worth the same run).
- #223 — desktop mempool-ws receive tracking + unique payment_id migration — **test-worthy**
  (medium). A regression duplicates onchain-receive history rows or leaves stuck "pending" rows
  (websocket row + balance-delta row for the same funds). Assertable in the native Mac runner and in
  a mobile flow-05 extension: one deposit → exactly one history row, txid attached, completes at
  6 conf. Desktop (+parity on mobile via #222/#228).
- #243 — stale-tip payment gate — **already-covered (implicitly) by flows 03/06** for the
  false-positive direction: an over-eager gate would block sends in regtest (3s sync interval) and
  fail existing flows. The positive stale-defer path needs a way to stall the chain source the app
  syncs from without killing the LSP's — not worth harness surgery; pilot metrics are the plan of
  record (memory: check `stability_gate` deferrals ~1 week post-pilot).
- #239 — price-feed quorum / fail-closed accounting — **test-worthy**. Fail-closed means "feeds
  unavailable → trading disabled with the consensus error, and recovers when feeds return". A
  regression either lets trades execute at a stale price (money) or never recovers (UX). Testable
  with a flow-13-style pre-hook: freeze/kill the mock feed, assert the trade path shows the
  fresh-consensus error and no allocation change; restore, assert trade works. All platforms + LSP
  (shared crate keeps consensus parameters in lockstep).
- #238 — durable signed TRADE_REJECTED_V1 + retry worker — **already-covered by flows 13, 14, 15**
  (rejection reason + unchanged allocation; durable redelivery after kill; background arrival).
- #230 — LSP REST/keysend DoS hardening — **not-e2e-testable** (adversarial unauthenticated
  traffic; the over-blocking direction — legit clients rejected — would fail flows 01/02 immediately,
  which is the regression that matters).
- #233 — authenticated, amount-bound stability settlements — **already-covered by flow 03** for
  the accept path (audit-log amount assertion 373,000 msat ±8k). Forged/replayed-settlement rejection
  is adversarial wire input — unit-test territory (the PR added 2,800 lines of it).
- #231 — settle stability by amount paid, not blind equilibrium — **test-worthy**. The regression
  class: after a settlement, backing is reset to equilibrium instead of debited/credited by the
  actual amount → books drift from reality → wrong next settlement (memory: debit-on-settlement
  invariant). Flow 03 asserts the payment was *sent* but never asserts the *post-settlement books*:
  add "next tick sends nothing" (no double settlement) + post-settlement USD display unchanged.
  LSP + all wallets.
- #225 — preserve stability drift across trades (delta accounting) — **test-worthy (top)**. The
  pre-#225 behavior let any trade re-anchor the whole position and silently erase accrued unsettled
  drift — accounting corruption exploitable for value capture. Flow: create drift (price move, before
  the settlement tick), execute a small trade, then assert the settlement for the accrued drift still
  arrives at the pre-trade magnitude. Also assert a full exit is refused while drift is unsettled
  ("Settle the current stability adjustment, then retry"). LSP + desktop wallet directly; mobile via
  the same sync path.
- #224 — iOS NSE wait-flag removal / flock startup — **not-e2e-testable** (NSE push processing
  can't be driven in the simulator harness; a gross startup regression trips flow 01's 30s waits).
- #228 — iOS ws receive reconciliation amount-guard — **test-worthy** (merged into the
  onchain-receive-integrity flow below). Regression = resolving one deposit deletes another
  deposit's history row under address reuse. iOS.
- #227 — iOS Face ID overhaul — **not-e2e-testable** (harness runs with `disable_send_auth: true`;
  biometric prompts can't be exercised by Maestro on sim).
- #222 — Android mempool-ws receive tracking + dedup — **test-worthy** (same integrity flow).
  Regression classes fixed in review revs: silent omission of a second deposit, confirmation-replay
  duplicate row, placeholder hijack across deposits. Android.
- #221 — backing derived locally, peer backing ignored — **already-covered by flows 02/08** for
  the honest path (post-trade USD balance asserted); the adversarial peer-supplied-backing path is
  wire-level. Its residual economics (H1/H2 boundaries) were fixed by #225 — see that flow.
- #219 — sheet-to-edge fix — **not-e2e-testable** (visual layout).
- #216 — show actual fee paid in sent status — **not-e2e-testable** worth having (copy-level; a
  fee-string assert in flow 06 would be brittle for near-zero value).
- #201 — Android custom LSP config + rollback — **test-worthy (low)**. The harness itself proves
  the custom-LSP *config* path daily (test_config IS a custom LSP); the switching UI + rollback-on
  -failed-restart path (regression = dead node after a failed switch) is uncovered but needs a second
  LSP in the harness — defer unless LSP-switching becomes a promoted feature. Android (iOS #207 same).
- #211 — QR photo-picker + background-stop ANR — **not-e2e-testable** (photo-picker intents are
  outside Maestro's reliable reach; ANR is a timing artifact).
- #215 — iOS DB indexes + safe unique-index migration — **not-e2e-testable in current harness**
  (the risk is the *upgrade* path on existing installs; the suite always starts from a fresh
  install. An upgrade-path harness lane would be a separate investment).
- #213 — 90-day price-history pruning — **not-e2e-testable** (timescale).
- #203 — iOS SPV header chain + reorg handling — **not-e2e-testable today** (needs a harness
  /invalidateblock endpoint to force a regtest reorg; worth noting as a future harness extension —
  reorg mishandling can mark confirmed payments wrong).
- #209 — iOS DB repository refactor — **no-behavior-change** (refactor; any regression surfaces
  across all existing flows).
- #208 — iOS Settings UI modularization — **no-behavior-change** (refactor; flow 10's settings
  navigation covers gross breakage).
- #210 — Umbrel packaging — **not-e2e-testable** here (has its own umbrel/test env).
- #207 — iOS custom LSP — same verdict as #201. **test-worthy (low)**, deferred.
- #191 — Logs & Diagnostics UI — **not-e2e-testable** worth having (log export UI; the
  share-sheet-resync suppression is subsumed by the #251 lifecycle flow's "no resync on return").
- #200 — LSP channel ledger — **test-worthy (low/medium)**. Server-side authoritative ledger;
  a cheap add: after flow 02/03, hit the LSP audit/ledger surface and assert a ledger row exists for
  the trade and the settlement (extends the existing audit-log assertion pattern in flow 03). LSP.
- #199 — Android API 36 target — **no-behavior-change** (build; the FGS-type compliance fix would
  crash visibly in any background-service use).
- #198 — LSP GUI contextual help — **not-e2e-testable** (operator GUI, optional container).
- #194 — pre-payment fee estimates — **already-covered-adjacent**: flow 02 asserts the Review
  screen; adding one `Expected fee` visibility assert there is a one-line cheap add, not a new flow.
- #193 — copy changes — **no-behavior-change** (flows already updated: 46bfc574).
- #190 — iOS event-driven confirmation tracking — **test-worthy** (merged into the
  onchain-receive-integrity flow: assert the n/6 badge advances as the harness mines and the row
  completes at 6). iOS (Android parity via #222).
- #177 — TLV on iOS/Android foreground payments — **test-worthy**. The TLV marks stability
  payments so the LSP classifies them; the *user-pays-LSP* settlement direction (price up) has ZERO
  e2e coverage today — flow 03 only tests LSP-pays-user (price down). A regression here misclassifies
  the settlement and corrupts both books (memory: reconcileIncoming backing_sats bug is this same
  class on the receive side). iOS + Android.
- #180 — Android UI polish/explorer links/close-race fixes — **already-covered by flow 09** for
  close basics; explorer-link correctness (known open port of the close-tx-vs-funding-tx bug) is a
  display item, not flow-worthy yet.
- #188 — LSP GUI redesign — **not-e2e-testable** (GUI).
- #186 — payment-detail navigation from home bubble — **not-e2e-testable** worth having (pure UI
  navigation, low regression value).
- #183 — txid resolution extraction + "new address doesn't clear active deposit txid" — **test-worthy**
  (folded into the onchain-receive-integrity flow: request a fresh address mid-deposit, assert the
  pending deposit's txid/row survives). iOS.
- #179 — desktop copy — **no-behavior-change**.
- #175 — bounded HTTP timeouts — **not-e2e-testable** (negative network conditions; regression
  direction (hangs) trips existing flow timeouts).
- #176 — gui feature gate — **no-behavior-change** (build).
- #169 — iOS NSE refactor/pending-payment handling — **not-e2e-testable** (push/NSE delivery
  can't be driven from the harness; covered by the "Payment Pending" foreground-recovery design).
- #195 — desktop/LSP stable accounting + signed sync + failed-settlement rollback — **already-covered
  by flow 03** for the settle path. The rollback-on-PaymentFailed CAS path needs a forced payment
  failure mid-settlement — not reachable from the current harness; holds as LSP unit tests (which are
  thorough). The over-backed repair guard is asserted indirectly by post-trade stability behavior.
- #197 — iOS mempool websocket service — superset-covered by #228's verdict → **test-worthy** via
  the same onchain-receive-integrity flow.
- #174 — force-close fixes: NodeDirLock + restore guard (/api/channel-exists) — **test-worthy
  (high)**. The restore guard is the anti-force-close backstop for the proven July root cause:
  restoring from mnemonic while a live LSP channel exists must surface the "Restore Anyway" warning
  instead of silently reestablishing with empty state (= guaranteed force close, funds CSV-locked).
  Flow 11 restores only *after* close, so the guard's firing path has no coverage.
  `channel_exists_url` is already in test_config. Opt-in flow (like 10/11): with the channel from
  02 still open, clear app state, restore the same seed, assert the warning; cancel; restore state.
  iOS (+ Android when ported). The flock itself is not-e2e-testable (needs the NSE as second writer).

Closed-unmerged PRs #240, #217, #220, #192, #142, #33, #32, #34, #178, #173 — unmerged, no
coverage needed (#217's economics merged via #221/#225, covered above; #240's ldk-node bump and
#220's send-auth scope had no merged successor behavior beyond what #248 carries).

## Ranked top test-worthy behaviors (impact-ordered)

Impact order: money-loss / state-loss / accounting-corruption first, then UX-breaking.
Suite constraint honored: 13–15 run right after 01 on a fresh wallet; the canonical price after 03
is $99,500 and later flows depend on it — any new price-moving flow must restore it via a post-hook.

1. **Stability drift preserved across trades** (#225, #221 residuals) — accounting corruption /
   value-capture exploit if it regresses. COMBINE → **Flow 16 "drift survives a trade"** (run after
   03, position exists): move price, trade a small amount before the tick, assert the settlement for
   the pre-trade accrued drift still arrives (audit-log amount assert, flow-03 pattern) and that a
   full exit is refused while drift is unsettled.
2. **User-pays-LSP settlement direction + amount-bound settle + TLV classification** (#177, #231,
   #233) — the entire outbound settlement half of the product has zero coverage; regressions
   corrupt both books silently. COMBINE with #1 into the same **Flow 16**: after the drift trade,
   move price *up*, assert the app sends the settlement (LSP audit log, direction+amount), assert
   backing debited only on confirmed settlement (no balance change at send), assert the next tick
   sends nothing (no double settlement). Post-hook restores $99,500. Also closes the #195/#200
   ledger assert almost for free (one extra audit/ledger check).
3. **Restore guard fires on live channel** (#174) — direct force-close prevention (proven mainnet
   money-loss class; ~230k sats across two incidents). Opt-in **Flow 17** (like 10/11, separate
   Maestro run): while the flow-02 channel is open, clear app data, restore the same mnemonic,
   assert the "Restore Anyway" warning appears and cancel restores safely.
4. **Splice-out completes and survives restart** (#252, #190) — stuck/failed splice rows and skipped
   settlement reconciliation. COMBINE → **extend flow 07**: after "Splice-out initiated", mine 6
   blocks via /mine, assert the row completes (n/6 badge → completed); add a kill-mid-splice variant
   (stopApp after initiate, relaunch, mine, assert resume + completion). Android first; iOS shares
   the monitor class.
5. **Onchain receive history integrity** (#222, #228, #223, #183, #197, #190) — duplicated, erased,
   or permanently-pending deposit rows. COMBINE → **extend flow 05** into one integrity flow: one
   deposit → exactly one history row with txid + explorer link; request a new address mid-deposit
   (row survives, #183); mine to 6 conf (badge advances, row completes, no second row appears —
   the confirmation-replay dup of #222). All three platforms (desktop via the Mac runner).
6. **Quick-switch resume liveness** (#251) — silent-dead-node regression: UI attached to a stopped
   node = missed payments and stability ticks until next cold start. **Flow 18** (Android): background
   1–3s → resume → assert no resync flash AND a harness /pay lands immediately (proves liveness);
   background >10s → resume → assert exactly one clean restart. Cheap, fully deterministic.
   Also: flip flow 15's Android branch to `stopApp` to restore LSP durable-retry coverage.
7. **Esplora startup failover** (#242) — app cannot start when the primary chain source is down.
   **Flow 19** (config-hook flow like 13): push test_config with primary→dead port, fallback→:30000,
   relaunch, assert Home reachable. iOS first (the PR), same run valuable on Android/desktop.
8. **Price-feed fail-closed + recovery** (#239) — trading at a stale price (money) or permanently
   disabled trading (UX). **Flow 20** (pre/post-hook): make the mock feed unreachable, assert the
   trade path shows the fresh-consensus error and allocation unchanged; restore feed, assert a trade
   succeeds. All platforms + LSP.

Combination summary: items 1+2 are ONE new flow (16); 4 and 5 are extensions of existing flows
07 and 05; 3, 6, 7 are small standalone flows (3 opt-in, 6–7 config/lifecycle hooks); 8 is one
hook-flow. Net new surface: ~3 new flows + 2 extensions + 1 opt-in + flow-15 Android tweak.

Deferred/backlog (documented above, not ranked): custom-LSP switch rollback (#201/#207 — needs a
second harness LSP), regtest reorg endpoint for SPV (#203), upgrade-path lane for migrations (#215),
stale-tip positive path (#243 — pilot metrics instead), fee-line assert in flow 02 (#194 one-liner).
