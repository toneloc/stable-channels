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

### 4. Payment sent

The paying side sends a keysend for the drift amount. A 120-second cooldown is set to prevent rapid-fire payments. `backing_sats` is NOT reset immediately — LDK's `send()` only means the payment was accepted for sending, not delivered. The next stability check after the cooldown detects any remaining drift and corrects.

### 5. Payment received

The receiving side runs `reconcile_incoming`, which only recomputes `native_channel_btc = receiver_sats - backing_sats`. `backing_sats` is not touched on the receiver side — the receiver's stability check won't fire for the same direction (CHECK_ONLY mode).

## Who Pays Who

| Condition | User action | LSP action |
|-----------|------------|------------|
| Price went up (user above par) | **PAY** — sends excess to LSP | CHECK_ONLY — waits |
| Price went down (user below par) | CHECK_ONLY — waits | **PAY** — sends shortfall to user |
| Within threshold | STABLE | STABLE |

## Regular Payments (Non-Stability)

When the user sends a regular Lightning payment:
- If covered by `native_sats` → native decreases, stable position unchanged
- If it overflows into backing → `expected_usd` is reduced proportionally, `backing_sats` recalculated, cooldown set

When the user receives a regular payment:
- `native_sats` increases, `backing_sats` unchanged

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
