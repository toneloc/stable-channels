# How Stable Channels Settlement Works

## Overview

Stable Channels creates synthetic USD stability inside a Lightning channel. Two parties — a Stable Receiver (user, wants dollar stability) and a Stable Provider (LSP, takes leveraged BTC exposure) — continuously settle based on the BTC/USD price.

## Key Variables

- **`expected_usd`** — the dollar amount the user wants to keep stable. Only changes on trades (buy/sell BTC).
- **`backing_sats`** — the BTC (in satoshis) backing the stable position. Set at trade time as `expected_usd / price * 1e8`.
- **`native_sats`** — the user's free-floating BTC that isn't part of the stable position. Changes when the user sends/receives regular payments.

## The Settlement Cycle

### 1. A trade establishes the position

User sells $50 of BTC exposure at $70,000/BTC:
- `expected_usd = 50.00`
- `backing_sats = 50 / 70000 * 1e8 = 71,429 sats`
- `native_sats = total_user_sats - backing_sats`

### 2. BTC price moves

`backing_sats` doesn't change — it's a fixed amount of BTC in the channel. But its dollar value drifts:
- `stable_usd_value = backing_sats * current_price / 1e8`

If price rose to $71,000: `71,429 * 71000 / 1e8 = $50.71` — user has $0.71 too much.
If price fell to $69,000: `71,429 * 69000 / 1e8 = $49.29` — user has $0.71 too little.

### 3. Stability check runs (every 60 seconds)

Both the server and mobile app independently check for drift:

```
drift = stable_usd_value - expected_usd
percent = abs(drift / expected_usd) * 100
```

If `percent < 0.1%` or `abs(drift) < $0.10` → **STABLE**, do nothing.

If drift exceeds threshold → **PAY**:
- Price went **up** → user pays LSP the excess (user is above par)
- Price went **down** → LSP pays user the shortfall (user is below par)

### 4. Stability payment sent

The paying side sends a keysend to its channel counterparty for the drift amount. This is an internal channel rebalancing payment, not a payment to an external destination. A 120-second cooldown is set to prevent rapid-fire payments. `backing_sats` is NOT reset immediately — LDK's `send()` only means the payment was accepted for sending, not delivered. The next stability check after the cooldown detects any remaining drift and corrects.

### 5. Stability payment received

The receiving side runs `reconcile_incoming`, which only recomputes `native_channel_btc = receiver_sats - backing_sats`. `backing_sats` is not touched on the receiver side — the receiver's stability check won't fire for the same direction (CHECK_ONLY mode).

## Who Pays Who

| Condition | User action | LSP action |
|-----------|------------|------------|
| Price went up (user above par) | **PAY** — sends excess to LSP | CHECK_ONLY — waits |
| Price went down (user below par) | CHECK_ONLY — waits | **PAY** — sends shortfall to user |
| Within threshold | STABLE | STABLE |

## Regular/External Payments (Non-Stability)

### Lightning payments

When the user sends a Lightning payment to an external destination:
- The payment is routed through the LSP to the recipient
- If covered by `native_sats` → native decreases, stable position unchanged
- If it overflows into backing → `expected_usd` is reduced proportionally, `backing_sats` recalculated, cooldown set
- The LSP's forwarded payment handler detects the overflow and sends a SYNC_V1 message to update the user's `expected_usd`

When the user receives a Lightning payment from an external sender:
- The payment is routed through the LSP to the user
- `native_sats` increases, `backing_sats` unchanged
- The user can then manually trade native BTC into a stable USD position using the balance bar slider (a "Sell BTC" trade that increases `expected_usd` and moves sats from native to backing)
- If the user has no channel yet, the LSP opens a JIT channel via LSPS2

### On-chain payments

**Receiving on-chain (deposit):**
- Funds arrive at the user's BDK on-chain wallet (a separate wallet from the Lightning channel)
- On-chain balance shows in the app under "On-chain" section
- Funds must be confirmed before they are spendable
- To use on-chain funds for Lightning payments or trading, the user must **splice in** — this moves the on-chain sats into the Lightning channel via a splice transaction
- The app shows a "Swap" button when the channel exists and on-chain funds are confirmed
- During the splice, the app shows "Swap pending..." with a link to the transaction on mempool.space

**Sending on-chain:**
- If the user has an active Lightning channel: the send is executed as a **splice out** — sats are removed from the Lightning channel and sent to the destination Bitcoin address in a single on-chain transaction
- If no channel exists: the send is a standard on-chain transaction from the BDK wallet. This covers the situation where the user wants to completely off-board from the wallet — close the channel (funds return on-chain), then send all on-chain funds to an external address.
- Splice-out reduces the channel capacity; if the amount exceeds `native_sats`, it overflows into the stable position and `expected_usd` is reduced accordingly

## Safety Mechanisms

- **120-second cooldown** after any stability payment or forwarded payment reconciliation — prevents rapid-fire payments from price micro-ticks
- **Threshold** (0.1% or $0.10) — filters out noise
- **No optimistic reset** — `backing_sats` is never reset until the actual channel balance confirms the payment landed, preventing silent state corruption from failed in-flight payments

## Future: Multiple Positions

The current model supports one stable position per channel (USD). Extending to N assets (EUR, gold, S&P 500) requires minimal structural change:

```
Channel:
  positions: Vec<Position>    // replaces single expected_usd + backing_sats
  native_sats: u64            // = total_user_sats - sum(positions.backing_sats)

Position:
  asset: String               // "USD", "EUR", "XAU"
  expected_amount: f64         // target value in asset units
  backing_sats: u64            // BTC backing this position
```

The stability check becomes a loop over positions. Each position is independent — its own price feed, its own drift calculation, its own payment:

```rust
for pos in &mut channel.positions {
    let price = get_price(pos.asset);  // BTC/EUR, BTC/XAU, etc.
    let value = pos.backing_sats as f64 / 1e8 * price;
    let drift = value - pos.expected_amount;
    if abs(drift) > threshold {
        send_payment(drift_in_sats);
    }
}
```

Trades move sats between native and a specific position (`buy_usd`, `sell_eur`), or between positions (`swap USD → EUR`). The settlement logic per position is identical to today's single-asset flow — the only new complexity is price feed management and ensuring `native_sats = total - sum(all backings)` stays consistent.

## Vision: A Global Derivatives Settlement Marketplace

Through this mechanism — continuous, bilateral, self-custodial settlement over Lightning — we can create a global derivatives settlement marketplace built entirely on Bitcoin.

The stability check is, at its core, a perpetual swap settled in real-time. The sats flowing between counterparties are economically equivalent to funding rate payments on perpetual futures contracts: the long side (Stable Provider) and the short side (Stable Receiver) continuously exchange value based on price movement, with no expiry and no centralized exchange.

Adding periodic funding rates — a small fee paid by one side to the other based on demand imbalance — would complete the analogy to perpetual futures. If more users want stability (short BTC exposure) than want leverage (long BTC exposure), the stability seekers pay a funding rate to attract providers. If demand flips, providers pay seekers. This rate could be set by the market, adjusted per-channel, or governed by a simple algorithm based on aggregate demand across the LSP's channel portfolio.

The result: peer-to-peer perpetual swaps on any asset with a price feed, settled continuously over Lightning, with no token, no exchange, no custody, and no counterparty beyond your direct channel partner. Every Lightning channel becomes a potential derivatives contract. Every LSP becomes a decentralized clearing house.
