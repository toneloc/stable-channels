# Index

Catalog of all pages in this wiki. Grouped by section. One line per page. The agent updates this on every ingest.

> Scale guidance: keep this file under ~200 lines. If it grows past that, split into per-section index files and link them here.

## Overview

- [[overview]] — what Stable Channels is, the project state, and how to navigate this wiki.

## Concepts

- [[concepts/stability-mechanism]] — the periodic loop that keeps the user's USD value pegged.
- [[concepts/backing-vs-native-sats]] — the dual accounting (stable portion vs floating BTC).
- [[concepts/price-feed-aggregation]] — five feeds + median + cache; the oracle defense.
- [[concepts/stable-receiver]] — the user role; targets USD stability.
- [[concepts/stable-provider]] — the LSP / company role; holds leveraged BTC exposure.
- [[concepts/overcollateralization]] — matched-equal deposits at entry; the -50% drop limit.
- [[concepts/cooldown-and-thresholds]] — 60s tick, 0.1%/$0.25 thresholds, 120s cooldown.
- [[concepts/non-routing-channel]] — current product shape; routing is future work.
- [[concepts/jit-channel-lsps2]] — first-channel UX via LSPS2 just-in-time channels.
- [[concepts/trade-vs-stability-payment]] — user-initiated trades vs automatic settlement.
- [[concepts/lsp]] — the company-operated Lightning Service Provider.
- [[concepts/splice-out-state-machine]] — cross-side timeline: what happens on user + LSP when a user splices out, what messages cross, when `expected_usd` changes on each side.
- [[concepts/notifications]] — how push notifications wake the mobile wallet so it can send / receive stability payments while offline.

## Modules

- [[modules/stable-rs]] — `src/stable.rs` — core stability + reconciliation logic.
- [[modules/stable-channels-user]] — `stable-channels` binary landing page (`src/main.rs` + `src/user.rs`). Sub-pages:
  - [[modules/user/boot-and-init]] — `UserApp::new()`: LDK config, seed handling, first stability check.
  - [[modules/user/background-thread]] — 30s stability + auto-sweep + deposit-detection loop.
  - [[modules/user/event-loop]] — LDK `Event` dispatch (Channel*, Payment*, Splice*).
  - [[modules/user/trade-flow]] — buy/sell → TRADE_V1 keysend → confirm on `PaymentSuccessful`.
  - [[modules/user/splice-flow]] — auto-sweep splice-in + user-initiated splice-out + reconciliation.
  - [[modules/user/payment-flows]] — `send_unified` (Bolt11/Bolt12/onchain), receive paths, SYNC_V1.
  - [[modules/user/ui-screens]] — screens, tabs, modals, toasts.
- [[modules/lsp-backend]] — LSP backend landing page. Canonical lives in `~/Code/ldk-server` branch `server-redo` (a fork of upstream LDK Server with stable-channels merged in). The in-repo `src/bin/lsp_backend.rs` is the legacy prototype. Sub-pages:
  - [[modules/lsp/boot-and-init]] — daemon init: config, API key, LDK Node Builder, listeners, manager instantiation.
  - [[modules/lsp/event-loop]] — tokio `select!` multiplexing LDK events, REST/push accepts, stability tick, signals.
  - [[modules/lsp/stable-manager]] — `StableChannelManager`: stability check, push targets, channel-event handlers, persistence, operator API.
  - [[modules/lsp/trade-and-sync]] — TRADE_V1 receive (signature verify + apply_trade), SYNC_V1 emit, `handle_payment_forwarded`.
  - [[modules/lsp/service-and-auth]] — HMAC-SHA256 auth, protobuf transport, route dispatch.
  - [[modules/lsp/push-notifications]] — APNs + FCM, 10-min cooldown, 15s NSE retry.
  - [[modules/lsp/rest-api-surface]] — full endpoint inventory (6 stable-channels-specific + 32 inherited).
- [[modules/lsp-frontend]] — `lsp_frontend` binary; operator dashboard egui app.
- [[modules/price-feeds-rs]] — `src/price_feeds.rs` — aggregation + cache + Kraken OHLC.
- [[modules/db-rs]] — `src/db.rs` — SQLite persistence (trades, payments, prices, push tokens).
- [[modules/audit-rs]] — `src/audit.rs` — append-only JSONL event log.
- [[modules/ios-app]] — `ios/StableChannels` — SwiftUI mobile client.
- [[modules/android-app]] — `android/app` — Android mobile client.

## Sources

- [[sources/delving-bitcoin-thread]] — primary technical discussion thread; objections + responses.
- [[sources/stablechannels-com]] — public marketing site.
- [[sources/ldk-node-docs]] — LDK Node reference (the Lightning library used).
- [[sources/lightning-crate-docs]] — rust-lightning (LDK) lower-level reference.
- [[sources/counsel-letter-2026-05]] — internal regulatory ask (stub; raw text gitignored).
- [[sources/ldk-server-fork]] — pointer to upstream LDK Server + Tony's `server-redo` fork.

## Decisions

- [[decisions/regulatory-positioning]] — MSB / CFTC / marketing-language analysis. **Gitignored.**
- [[decisions/cross-platform-consistency]] — strategy for keeping Rust desktop / iOS / Android wallets behaviorally aligned. Tracks known divergences. **Gitignored.**
