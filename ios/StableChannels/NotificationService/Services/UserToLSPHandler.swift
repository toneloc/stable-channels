import Foundation
import UserNotifications
import LDKNode

/// Deadline- and cancellation-aware context for the user_to_lsp chain-freshness wait,
/// built by NotificationService so the handler's wait observes the extension lifecycle.
struct StabilitySyncGate {
    /// Absolute wall-clock limit for the wait (extensionStart + 20s). The deadline is
    /// authoritative: once passed, the run defers even if a sync just became fresh, so the
    /// reserved tail of the execution window is never consumed by payment work started late.
    let deadline: Date
    /// True once the run expired or already finished; polled between wait iterations
    /// (reads the extension's lifecycle state under its lock).
    let isCancelled: () -> Bool
    /// Structured pilot-metric sink (the extension's file logger).
    let log: (String) -> Void
}

/// Handler for "user_to_lsp" direction - calculate and send payment to LSP
final class UserToLSPHandler: PaymentHandler {
    var direction: PaymentDirection { .userToLsp }

    private static let syncPollIntervalSecs: TimeInterval = 0.5

    private let syncGate: StabilitySyncGate?

    init(syncGate: StabilitySyncGate? = nil) {
        self.syncGate = syncGate
    }

    func handle(
        node: LDKNode.Node,
        db: PaymentDatabase,
        priceFetcher: PriceFetcher,
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool?) -> Void
    ) {
        // Check pending outgoing
        guard db.reconcilePendingOutgoingPayment(node: node) else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Sent",
                    body: "Open app to finish syncing the stability payment"
                ),
                true
            )
            return
        }

        // Cooldown check
        let shared = UserDefaults(suiteName: Constants.appGroup)
        shared?.synchronize()
        let lastSent = shared?.double(forKey: "nse_last_stability_sent") ?? 0
        if lastSent > 0 && Date().timeIntervalSince1970 - lastSent < 120 {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), nil)
            return
        }

        // Read channel state
        guard let channelState = db.readChannelState() else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
            return
        }

        let backingSats = channelState.backingSats
        guard channelState.expectedUSD >= 0.01 else {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), nil)
            return
        }

        // Fetch price
        let price = priceFetcher.fetchPrice()
        guard price > 0 else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
            return
        }

        // Calculate stability payment
        let stableUSDValue = Double(backingSats) / Constants.satsInBTC * price
        let targetUSD = channelState.expectedUSD
        let dollarsFromPar = stableUSDValue - targetUSD
        let percentFromPar = targetUSD > 0 ? abs(dollarsFromPar / targetUSD) * 100.0 : 0.0

        // Within threshold - no payment needed
        guard percentFromPar >= Constants.stabilityThresholdPercent && abs(dollarsFromPar) >= 0.25 else {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), nil)
            return
        }

        // User is above expected (price rose) - should pay LSP
        guard stableUSDValue > targetUSD else {
            completion(mutator.buildStablePosition(base: baseContent, body: "Position is stable"), nil)
            return
        }

        // Calculate amount
        let dollarsAbs = abs(dollarsFromPar)
        let btcAmount = dollarsAbs / price
        let amountMsat = UInt64(btcAmount * Constants.satsInBTC * 1000)
        let amountSats = amountMsat / 1000

        // Chain-freshness gate at the send boundary (see #243): never keysend on a stale
        // chain tip — an outbound HTLC built on an old best block understates its expiry,
        // and LDK later force-closes on it. The gate sits here, after the cooldown/threshold
        // checks and the price fetch, so stable positions never wait or defer, and LDK's
        // background sync runs concurrently with the price work above. Checked BEFORE the
        // claim so a deferral never leaves a claimed-but-unsent marker.
        let gateStart = Date()
        let initialSyncAge = StabilityFreshness.syncAgeSecs(
            node.status().latestLightningWalletSyncTimestamp,
            now: UInt64(gateStart.timeIntervalSince1970)
        )
        logGateEvent("stability_background_attempted", initialAge: initialSyncAge, gateStart: gateStart)
        var waitedForSync = false
        while true {
            // Order matters: cancellation, then deadline, then freshness — a fresh sync
            // observed after the deadline still defers, keeping the reserved execution
            // budget intact for runs that started slow.
            if let gate = syncGate, gate.isCancelled() {
                logGateEvent("stability_background_deferred_stale_sync",
                             initialAge: initialSyncAge, gateStart: gateStart, outcome: "cancelled")
                deferToForeground(baseContent: baseContent, mutator: mutator, completion: completion)
                return
            }
            if let gate = syncGate, Date() >= gate.deadline {
                logGateEvent("stability_background_deferred_stale_sync",
                             initialAge: initialSyncAge, gateStart: gateStart, outcome: "deadline_reached")
                deferToForeground(baseContent: baseContent, mutator: mutator, completion: completion)
                return
            }
            let now = UInt64(Date().timeIntervalSince1970)
            if StabilityFreshness.isFresh(node.status().latestLightningWalletSyncTimestamp, now: now) {
                logGateEvent(
                    waitedForSync ? "stability_background_fresh_after_wait" : "stability_background_fresh_ready",
                    initialAge: initialSyncAge,
                    gateStart: gateStart,
                    outcome: "fresh"
                )
                break
            }
            guard syncGate != nil else {
                // No wait context (direct construction) — single check, fail safe.
                logGateEvent("stability_background_deferred_stale_sync",
                             initialAge: initialSyncAge, gateStart: gateStart, outcome: "no_wait_context")
                deferToForeground(baseContent: baseContent, mutator: mutator, completion: completion)
                return
            }
            waitedForSync = true
            Thread.sleep(forTimeInterval: Self.syncPollIntervalSecs)
        }

        // Claim slot
        guard db.claimPendingSend(amountMsat: amountMsat, price: price) else {
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
            return
        }

        // Re-check after the claim: the SQLite claim can take up to ~2s under cross-process
        // contention and could carry the timestamp past the 120s boundary. No send happened,
        // so clear the claim rather than blocking the foreground retry.
        let nowAfterClaim = UInt64(Date().timeIntervalSince1970)
        guard StabilityFreshness.isFresh(node.status().latestLightningWalletSyncTimestamp, now: nowAfterClaim) else {
            db.clearPendingSend()
            logGateEvent("stability_background_deferred_stale_sync",
                         initialAge: initialSyncAge, gateStart: gateStart, outcome: "stale_after_claim")
            deferToForeground(baseContent: baseContent, mutator: mutator, completion: completion)
            return
        }

        // Send keysend
        do {
            let tlvRecord = CustomTlvRecord(typeNum: Constants.stableChannelTLVType, value: Data([1]))
            let paymentId = try node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat: amountMsat,
                nodeId: Constants.lspPubkey,
                routeParameters: nil,
                customTlvs: [tlvRecord]
            )

            // Only an accepted send counts as sent_* — cooldown, no-action, denied-claim,
            // and send-failure runs must not inflate the pilot's send numbers.
            logGateEvent(waitedForSync ? "stability_background_sent_after_sync" : "stability_background_sent_fresh",
                         initialAge: initialSyncAge, gateStart: gateStart, outcome: "sent")

            // Payment ID Guard
            let guardSaved = db.setPendingSendPaymentId(paymentId: "\(paymentId)")

            // Update cooldown
            shared?.set(Date().timeIntervalSince1970, forKey: "nse_last_stability_sent")
            shared?.synchronize()

            guard guardSaved else {
                completion(
                    mutator.buildPending(
                        base: baseContent,
                        title: "Payment Sent",
                        body: "Open app to finish syncing the stability payment"
                    ),
                    true
                )
                return
            }

            // Record payment
            let result = db.recordPayment(
                paymentId: "\(paymentId)",
                paymentType: "stability",
                direction: "sent",
                amountMsat: amountMsat,
                amountUSD: dollarsAbs,
                btcPrice: price,
                backingDeltaSats: -Int64(amountSats),
                userChannelId: channelState.userChannelId
            )

            switch result {
            case .inserted, .duplicate:
                db.clearPendingSend()
                completion(mutator.buildForSent(base: baseContent, amountSats: amountSats, dollars: dollarsAbs), false)
            case .failed, .missingChannelRow:
                completion(
                    mutator
                        .buildPending(
                            base: baseContent,
                            title: "Payment Sent",
                            body: "Open app to finish syncing the stability payment"
                        ),
                    true
                )
            }
        } catch {
            db.clearPendingSend()
            completion(
                mutator.buildPending(
                    base: baseContent,
                    title: "Payment Pending",
                    body: "Open app to process stability payment"
                ),
                true
            )
        }
    }

    /// Freshness-gate deferral: hand the payment to the foreground via the existing
    /// pending-notification path. If the run was cancelled by expiry, the completion is
    /// harmless — the extension's exactly-once finish already delivered the notification.
    private func deferToForeground(
        baseContent: UNMutableNotificationContent,
        mutator: NotificationContentMutator,
        completion: @escaping (UNMutableNotificationContent, Bool?) -> Void
    ) {
        completion(
            mutator.buildPending(
                base: baseContent,
                title: "Payment Pending",
                body: "Open app to process stability payment"
            ),
            true
        )
    }

    /// Structured pilot metric for the user_to_lsp freshness gate. No wallet secrets or
    /// payment identifiers — platform, sync age, wait time, and outcome only.
    private func logGateEvent(
        _ event: String, initialAge: UInt64?, gateStart: Date, outcome: String? = nil
    ) {
        guard let log = syncGate?.log else { return }
        let waitedMs = Int(Date().timeIntervalSince(gateStart) * 1000)
        let age = initialAge.map { "\($0)" } ?? "null"
        let outcomeJson = outcome.map { "\"\($0)\"" } ?? "null"
        log(
            "stability_gate {\"event\":\"\(event)\",\"platform\":\"ios\"," +
                "\"prev_sync_age_secs\":\(age),\"waited_ms\":\(waitedMs)," +
                "\"deadline_outcome\":\(outcomeJson)}"
        )
    }
}
