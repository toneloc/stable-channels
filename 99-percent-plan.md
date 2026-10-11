# 99% Maximum Stabilization — implementation plan (v3)

Verified against `c9123b2`. No such rule exists today anywhere. Core design (unchanged
through three reviews): **cap the resulting allocation, never scale the order by 0.99.**

**This is a trade-entry limit, not a guarantee.** Settlements and price drift can and will
push existing positions above the line; that is correct behavior and stays untouched.

## Rule (normative)

All integer satoshi arithmetic. `B` = spendable post-fee receiver sats.

```
Client:  B = own outbound_capacity_msat / 1000 − fee_sats
             // outbound capacity already excludes the client's OWN
             // unspendable_punishment_reserve
LSP:     B = user's inbound_capacity_msat / 1000
             // inbound capacity already excludes the USER's reserve
             // (counterparty_unspendable_punishment_reserve — never the LSP's own
             // unspendable_punishment_reserve; the two are independent and routinely
             // differ by far more than the 50-sat margin).
             // Observed AFTER the fee keysend landed — do NOT subtract the fee again.

cap        = min( floor(99 × B / 100),  B − ABSOLUTE_MIN_NATIVE_SATS )
             // min() of two floors rounds conservatively; a bare floor(B/100)
             // reserve rounds the WRONG way (B=1,000,001 → cap 990,001 > 99%)

client_limit = cap − CLIENT_SAFETY_MARGIN_SATS   (if cap ≤ margin: treat as at-limit,
                                                  max order = 0, safe rejection)

LSP    accepts iff newBackingSats ≤ cap           (unbuffered — this IS the limit)
Client accepts iff newBackingSats ≤ client_limit  (ALL surfaces: helper, Max button,
                                                  slider clamp, manual entry, and the
                                                  service-layer validation — one number)
```

**The two maxima are deliberately different.** Every client surface enforces the same
buffered `client_limit`; only the LSP enforces the raw `cap`. An order between the two
would be accepted by the LSP but is unreachable through a correct client — the buffer
exists purely to absorb view skew.

- **The LSP's strict check is the definition of the limit.** No server-side slack —
  the advertised bound is the enforced bound. The client's small fixed margin keeps
  normal Max orders clear of view skew (in-flight HTLCs, a settlement landing mid-flight).
  Note honestly: the margin makes post-fee rejection *rare*, not impossible; if fee-loss
  reports actually materialize, the fix is carrying a signed balance basis in the trade
  (like quotes do for price) — future work, do not build it now.
- Reject safely (never clamp) when B can't cover the fee, reserve, or floor.
- Why spendable sats: displayed receiver balance here is `capacity + punishment_reserve`
  (`src/stable.rs:568`, `src/user.rs:7782`, `stable_manager.rs:33`); a percentage of the
  raw number can leave zero actually-spendable native BTC on small channels.
- Authoritative derivation for B is the per-peer capacity fields as in
  `channel_peer_balances` (`stable_manager.rs:31`). Do NOT derive the user's balance as
  `channel_value − our_balance` (`src/stable.rs:571` still does) — that misassigns the
  funder's commitment fee, exactly the mistake `channel_peer_balances`' doc comment warns
  about.
- `ABSOLUTE_MIN_NATIVE_SATS = 2000` (decided): routing-fee/HTLC headroom is an absolute
  need — 1% of a 200k-sat channel buys nothing. 2,000 sats (~$1.60 at $80k) covers many
  keysend/routing fees and is invisible above ~200k-sat channels.
- `CLIENT_SAFETY_MARGIN_SATS = 50` (decided): absorbs typical view skew from in-flight
  msat rounding and sub-threshold drift. It does NOT cover a settlement landing on the
  LSP side that the client hasn't observed — revalidation runs on the client's view, so
  that residual case is the accepted rare fee loss noted above.
- USD → BTC is uncapped (existing drift-preservation rules still apply; full exit allowed).
- **Trade-entry paths only** — trade prepare/accept and the manual `STABLE_EDITED` edit
  path (capping the edit also fixes the standing bug where an edit sets `backing_sats`
  above real sats). NEVER in settlement/reconcile paths; comment this at every site.

## Copy

Show the **computed maximum as a dollar amount** everywhere, and it means the
**additional order amount** (what the helper returns), not the resulting total — existing
positions make those materially different: **"Maximum additional trade: $X.XX"**. Always
true whether the 99% branch or the absolute floor binds, so there is one copy path
instead of two. The "99%" figure appears only in the explanatory sentence
("keeps ~1% of your balance in BTC for network fees"). Do NOT hardcode "99% USD / 1% BTC"
at the slider endpoint: the bar renders a USD-value ratio, the cap is in backing sats, and
drift makes them diverge — clamp and label in the bar's own display units.

## 1. Constants

`MAX_STABLE_ALLOCATION_PERCENT = 99`, `ABSOLUTE_MIN_NATIVE_SATS`,
`CLIENT_SAFETY_MARGIN_SATS` in Android `util/Constants.kt`, iOS, Rust `src/constants.rs`.

iOS: there are TWO `Constants.swift` (`StableChannels/Utilities/`,
`NotificationService/Services/`), and enforcement in
`NotificationService/Services/TradeProtocol.swift` must see the values — put them in one
file that's a member of BOTH targets (check `project.yml`); if impossible, duplicate with
a unit test asserting equality.

## 2. Canonical max-trade helper

One pure helper per platform: the largest cent order satisfying **every LSP accept rule
with the cap replaced by the buffered `client_limit`** (fee coverage, native
availability, drift preservation via `tradeBackingAfterDelta` /
`local_trade_backing_sats`, AND `newBackingSats ≤ client_limit`).
A cap-only Max lands on amounts the LSP still rejects (`SettlementRequired`) — a Max
button that generates rejections.

**Rust first, then port.** `max_sell_trade_usd_cents()` (`src/user.rs:11811`, tests
~12011) already binary-searches an accept predicate, and `local_trade_backing_sats`
(`:11759`) already enforces an implicit 100% ceiling — this tightens one existing
predicate. Add the cap to `fits_allocation`; the search converges for free.

## 3–5. Clients (Android / iOS / Desktop)

Same shape on all three:

- **Balance bar**: clamp the BTC→USD drag at `max(current allocation, cap)` in display
  units — a drift-inflated ≥99% position renders its real number and is never offered a
  "reduction" to 99%. Past the endpoint: limit feedback + the dollar-max copy.
- **Sell screen**: Max button uses the helper (iOS gains a Max action for parity);
  prefill with the helper result, never raw native balance; reject manual over-limit
  input with the dollar-max copy; **recompute and revalidate at submission** (balances
  move while the sheet is open — a mid-flight settlement is the common case).
- **Service layer** (`TradeService`/`TradeProtocol` on mobile, `execute_sell()` on
  desktop): enforce the cap before persisting or sending the non-refundable fee; return
  a distinct validation failure, never silently clamp `newExpectedUSD`.
- **Android error typing (required, or the copy above never renders)**: `executeSell`/
  `executeBuy` return `TradeResult?` with unreasoned `return null`s, and both call sites
  collapse every null into `?: throw Exception("Trade service unavailable")`
  (`SellScreen.kt` / `BuyScreen.kt`). Change the return to a sealed result (or typed
  exception) carrying the reason, and split both call sites. This also fixes the
  known #272 UX (zero-USD buys shown as "Trade service unavailable").
- Files: Android `ui/home/BalanceBar.kt`, `ui/trade/SellScreen.kt`, `services/TradeService.kt`,
  `TradeProtocol.kt` · iOS `Features/Home/BalanceBarView.swift`, `Features/Trade/SellView.swift`,
  `Services/TradeService.swift`, `NotificationService/Services/TradeProtocol.swift`,
  `Localizable.xcstrings` · Desktop `src/user.rs` (helper predicate, bar boundary, sell
  dialog, `execute_sell()` revalidation; cents floored, sats rounded conservatively).

## 6. LSP enforcement

- **Reuse `InsufficientCapacity` as the rejection reason.** It is the code the adjacent
  100% ceiling already uses (`stable_manager.rs:2813`), so the cap is a tightening of an
  existing rejection, and its shipped copy ("…Reduce the amount.") tells the user what to
  do. A NEW reason is off the table: `TradeRejectionReason` (`src/trade.rs:10`) is a
  closed serde enum under `deny_unknown_fields`, parsed with `.ok()?` (`src/user.rs:11723`)
  — an unknown reason drops the whole rejection and wedges the deployed desktop wallet
  ("awaiting the provider", later trades blocked). The client produces the precise 99%
  copy pre-submission anyway.
- Strict check (`newBackingSats ≤ cap`, formula above, no fee re-deduction) in BOTH the
  correlated and legacy paths in `stable_manager.rs`, plus the `STABLE_EDITED` path.
- **Audit event in both modes**: emit `MAX_STABILIZATION_REJECTED {enforced: bool}` from
  the check permanently — since the reason code is shared with other conditions, this
  event is the only way to tell a cap rejection from an allocation-math failure in the
  logs, and it's what the rollback criterion monitors.
- **Shadow mode first**: land the check emitting the event with `enforced: false` only,
  behind a config flag. Decision rule (explicit, so enforcement is a decision and
  not a drift): observe for **4 weeks after the capped mobile clients ship**; flip to
  enforcing when the trailing-2-week event rate is zero. Rollback criterion: any
  post-enforcement rejection of an organic (non-test) trade at ≤ 100.5% → flip the flag
  back and reassess the margin. **Known, accepted limitation**: a zero event rate can
  just mean dormant old clients haven't traded lately — after enforcement starts, a
  dormant old install submitting a full-Max trade WILL lose its non-refundable fee and
  see the generic `InsufficientCapacity` copy. We accept this rather than build version
  negotiation; the audit log will show any such case.

## 7. Edge cases

- Position ≥ 99% from drift: block further BTC→USD, allow USD→BTC, render the real
  percentage, never rewrite.
- B too small for a one-cent trade + floor: at-limit copy, safe failure.
- Mid-flight settlement between Max computation and submission: submission revalidation
  is the defense; test it.
- Pending trades when price later pushes the would-be allocation over cap: nothing
  happens — cap is evaluated at entry only (stated so nobody "fixes" it).

## 8. Tests

**Snapshot contract** (against CLIENT validation, at the buffered `client_limit`): on
identical frozen state + quote inputs — Max is accepted and Max + 1 cent is rejected.
The LSP may legitimately accept Max + 1 cent (it sits inside the buffer); LSP tests
assert its own boundary at the unbuffered `cap`. Add a vector where `cap ≤ margin`
(max order = 0, safe at-limit rejection). Never assert acceptance at a literal
"exactly 99%": the boundary of a two-party computation is not a stable test target.
**Changed-state tests** assert only that revalidation rejected safely — not which error,
since stale price / fees / drift each produce their own.

One canonical table of test vectors (inputs: receiver sats, reserve, fee, drift, price;
outputs: cap, max cents) lives in one file; each platform's table-driven test mirrors it
verbatim with a pointer comment. Vectors must include: floor-binding small channel
(1% < `ABSOLUTE_MIN_NATIVE_SATS`), nonzero punishment reserve, the rounding boundary
(B = 1,000,001), and a fee-at-the-edge case.

Also: positions ≥99% can buy but not sell; settlement/reconcile paths remain uncapped
(regression guard); LSP double-fee-deduction guard (boundary trade accepted against
post-payment balance); `MAX_STABILIZATION_REJECTED` fires for a 100% target in both
shadow and enforcing modes.

Run: Android unit tests, iOS XCTest, Rust fmt/clippy/tests incl. LSP handler tests.
One E2E per client (drag/press Max → confirm → both peers persist ≤ cap) — fits the aux
suite as flow 21; existing flows trade well under the cap and are unaffected.

## 9. Rollout

1. Rust helper + vectors + tests → desktop UI + `execute_sell` revalidation.
2. Android, then iOS (settle the iOS constants/target question first).
3. LSP check in **shadow mode** (deployable any time — it's log-only).
4. Flip LSP to enforcing when the shadow event rate is ~zero.
