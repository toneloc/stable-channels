---
type: concept
status: active
tags: [notifications, push, apns, fcm, lsp, background]
last_updated: 2026-05-26
sources: [ldk-server/src/main.rs, ldk-server/src/push/, ios/StableChannels/StableChannels/StableChannelsApp.swift, android/app/src/main/java/com/stablechannels/app/push/]
---

# Notifications

We use push notifications to wake the mobile wallet when it needs to act on the user's behalf — currently, that's when the LSP detects the user's [[stable-receiver|stable position]] has drifted past par and needs a rebalance payment. The wallet is offline most of the time; pushes are how we get back its attention.

## How it works

1. **Phone registers with the server**: on first launch (and on reconnect), the wallet gets its APNs token (iOS) or FCM token (Android) from the OS, then posts it to the LSP at `/api/register-push` along with its Lightning node ID. The LSP stores the token in `push_tokens.db` so it knows where to send pushes for that node ID.

2. **Server fires a push**: when the periodic [[stability-mechanism|stability check]] finds an offline user whose channel has drifted past par, the LSP looks up their device token and asks APNs / FCM to deliver a push. See [[modules/lsp/push-notifications]] for the 10-min cooldown and 15s NSE retry logic.

3. **Phone wakes a background handler**: on iOS, the OS spins up a Notification Service Extension with its own LDK node; on Android, `FCMService` starts a `StabilityProcessingService` that reuses the main app's `NodeService`. iOS is working-ish but needs review; Android validation is in progress.

4. **Wallet does the work**: reconnects to the LSP, refreshes the latest price, sends or receives the stability payment, then exits before the OS revokes its background budget.

## Why we need APNs / FCM at all

Even though we have our own LSP, **only Apple (APNs) and Google (FCM) can deliver pushes to a device that isn't actively running our app**. The OS only allows one persistent push channel per platform, and that channel terminates at Apple/Google — not at arbitrary servers. Our LSP is the originator (decides *when* and *who*); APNs / FCM is the courier (delivers to the phone). FCM is wired up on Android via `google-services.json` + the `com.google.gms.google-services` Gradle plugin; APNs uses the keys configured in the LSP's `apns.rs`.

## What we *don't* push for today

- Incoming HTLCs (e.g., someone pays the user's invoice while the user is offline)
- LSPS2 JIT channel handshake events
- Channel-state changes (ChannelReady, ChannelClosed, splice events)
- Trade settlement (TRADE_V1 receipt)
- Forwarded payments

There is exactly one push trigger: stability rebalance, fired from the periodic stability tick in `ldk-server/src/main.rs` via `get_stability_push_targets()`. The "receive a payment while offline" future enhancement would require adding an HTLC-arrival push trigger on the LSP and a corresponding handler on the phone.

## Related

- [[stability-mechanism]]
- [[modules/lsp/push-notifications]] — LSP-side internals (APNs / FCM clients, cooldown, retry)
- [[decisions/cross-platform-consistency]] — flags iOS NSE vs Android ForegroundService as the highest-risk parity gap
