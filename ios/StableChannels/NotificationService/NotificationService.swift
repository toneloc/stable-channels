import UserNotifications
import LDKNode
import SQLite3
import Darwin

/// Cross-process exclusive lock on the LDK wallet data directory.
///
/// The NSE must NEVER run a node (or strip gossip from ldk_node_data.sqlite)
/// while the main app's node is live: two writers can regress channel state
/// by a commitment and force-close the channel. flock is kernel-enforced and
/// auto-released if this process is killed mid-run.
///
/// Keep in sync with the copy in StableChannels/Services/NodeService.swift.
final class NodeDirLock: @unchecked Sendable {
    static let shared = NodeDirLock()
    static let lockFilename = "ldk-node.lock"

    private var fd: Int32 = -1
    private let queue = DispatchQueue(label: "com.stablechannels.nse.nodedirlock")

    /// Take the lock without blocking. Returns true if acquired or already
    /// held by this process.
    func tryAcquire(dataDir: URL) -> Bool {
        queue.sync {
            if fd >= 0 {
                return true
            }
            try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
            let path = dataDir.appendingPathComponent(Self.lockFilename).path
            let f = open(path, O_CREAT | O_RDWR, 0o644)
            guard f >= 0 else { return false }
            guard flock(f, LOCK_EX | LOCK_NB) == 0 else {
                close(f)
                return false
            }
            fd = f
            return true
        }
    }

    /// Release if held. Safe to call when not holding (no-op).
    func release() {
        queue.sync {
            guard fd >= 0 else { return }
            flock(fd, LOCK_UN)
            close(fd)
            fd = -1
        }
    }
}

/// Chain-freshness rule for stability payments (see #243): a user_to_lsp stability payment
/// may only be sent when LDK completed a Lightning-wallet chain sync within
/// `maxAgeSecs`. Paying on a stale chain tip understates outbound HTLC expiry, which LDK
/// later force-closes on. The timestamp is persisted by LDK (node_metrics), so a run whose
/// gossip strip kept node_metrics can inherit the main app's recent sync and send at once.
///
/// Keep in sync with `lightning_sync_is_fresh` in `src/stable.rs` and the copy in
/// StableChannels/Services/NodeService.swift.
enum StabilityFreshness {
    /// Keep in sync with STABILITY_MAX_LIGHTNING_SYNC_AGE_SECS in src/constants.rs.
    static let maxAgeSecs: UInt64 = 120

    /// True when the timestamp exists, is not in the future, and is at most `maxAgeSecs`
    /// old (exactly `maxAgeSecs` old is accepted).
    static func isFresh(_ latestSyncTimestampSecs: UInt64?, now nowSecs: UInt64) -> Bool {
        guard let ts = latestSyncTimestampSecs, ts <= nowSecs else { return false }
        return nowSecs - ts <= maxAgeSecs
    }

    /// Age of the last sync in seconds, or nil when missing or in the future.
    static func syncAgeSecs(_ latestSyncTimestampSecs: UInt64?, now nowSecs: UInt64) -> UInt64? {
        guard let ts = latestSyncTimestampSecs, ts <= nowSecs else { return nil }
        return nowSecs - ts
    }
}

/// Notification Service Extension — handles stability payments while main app is killed.
/// Uses dependency injection for testability and SOLID compliance.
class NotificationService: UNNotificationServiceExtension {
    // MARK: - Constants

    /// user_to_lsp only: how long after `extensionStart` the freshness wait may run. iOS
    /// grants the extension ~30s total; stopping the wait at +20s reserves ~10s for the
    /// send, persistence, notification mutation, and cleanup. This is an outer limit, not a
    /// promise — node build, LSP connect, and price work all consume part of it.
    private static let userToLspFreshnessDeadlineSecs: TimeInterval = 20
    private static let freshnessPollIntervalSecs: TimeInterval = 0.5

    // MARK: - Dependencies (injected for testability)

    private var logger: Logger = FileLogger(appGroup: Constants.appGroup)
    private var priceFetcher: PriceFetcher = ConcurrentPriceFetcher()
    private let contentMutator: NotificationContentMutator = DefaultNotificationContentMutator()
    private var nodeStarter: NodeStarter = DefaultNodeStarter()
    private var db: PaymentDatabase?

    // MARK: - State

    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttemptContent: UNMutableNotificationContent?
    private var node: Node?

    /// When this run's notification arrived — anchor for the user_to_lsp freshness
    /// deadline. Recorded before node build or price work so those costs count against
    /// the wait budget, not on top of it.
    private var extensionStart = Date()

    /// Coordinates serviceExtensionTimeWillExpire with an in-flight node
    /// build: while `buildInFlight`, the expire handler must NOT release the
    /// wallet-dir lock (the build completes and starts a node regardless —
    /// releasing early would let the main app start a second node against
    /// the same wallet). The builder observes `timeExpired` and does the
    /// stop+release itself; if iOS kills the process, the kernel releases.
    private let lifecycleLock = NSLock()
    private var buildInFlight = false
    private var timeExpired = false
    private var didFinish = false

    /// Deliver notification content exactly once. Expiry and the async
    /// processing pipeline can race to finish; whichever loses becomes a
    /// no-op instead of double-calling the content handler.
    private func finish(_ content: UNNotificationContent) {
        lifecycleLock.lock()
        let alreadyFinished = didFinish
        didFinish = true
        lifecycleLock.unlock()
        guard !alreadyFinished, let handler = contentHandler else { return }
        handler(content)
    }

    // MARK: - Entry Point

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        extensionStart = Date()
        self.contentHandler = contentHandler
        self.bestAttemptContent = (request.content.mutableCopy() as? UNMutableNotificationContent)

        guard let content = bestAttemptContent else {
            finish(request.content)
            return
        }

        // Group payment notifications under a single thread to coalesce them
        content.threadIdentifier = "stable-channels-payment"

        // Parse direction
        let direction = PaymentDirection(from: request.content.userInfo)

        // Tag with notification type
        var updatedUserInfo = content.userInfo
        updatedUserInfo["notification_type"] = direction.notificationType
        content.userInfo = updatedUserInfo

        logger.log("didReceive: direction=\(direction.rawValue)")

        let shared = UserDefaults(suiteName: Constants.appGroup)

        // Skip if main app is active
        if isMainAppActive() {
            logger.log("Main app is active, skipping node start")
            shared?.set(true, forKey: "pending_push_payment")
            finish(content)
            return
        }

        // Initialize database
        let dbPath = getDBPath()
        self.db = SQLitePaymentDatabase(dbPath: dbPath)

        // Start processing
        DispatchQueue.global(qos: .userInitiated).async {
            self.startProcessing(content: content, direction: direction, contentHandler: contentHandler)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        logger.log("TIME EXPIRED")
        lifecycleLock.lock()
        timeExpired = true
        let deferToBuilder = buildInFlight
        let alreadyFinished = didFinish
        lifecycleLock.unlock()
        if !alreadyFinished {
            // Whatever was in flight did not complete — flag it so the main
            // app processes the payment on next open. (Every timeout path
            // must set this; a completed run has already finished and skips.)
            UserDefaults(suiteName: Constants.appGroup)?.set(true, forKey: "pending_push_payment")
        }
        if deferToBuilder {
            // A node build owns the flock right now. Don't release it here:
            // the builder will stop+release when the build returns, and if
            // iOS kills us first the kernel releases. Releasing early would
            // reopen the dual-node force-close window.
            logger.log("Expire during node build; deferring stop/release to builder")
        } else {
            cleanup()
        }
        if let content = bestAttemptContent {
            content.title = "Payment Pending"
            content.body = "Open app to process your payment"
            finish(content)
        }
    }

    // MARK: - Processing

    private func startProcessing(
        content: UNMutableNotificationContent,
        direction: PaymentDirection,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        guard let container = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: Constants.appGroup) else {
            logger.log("FAILED: No shared container")
            cleanup()
            finish(content)
            return
        }

        let dataDir = container
            .appendingPathComponent("StableChannels")
            .appendingPathComponent("user")

        // Check for seed
        guard hasSeed(dataDir: dataDir) else {
            logger.log("FAILED: No seed found")
            cleanup()
            finish(content)
            return
        }

        // From here until the build resolves, the expire handler must defer
        // lock release to us (see lifecycleLock docs).
        lifecycleLock.lock()
        buildInFlight = true
        lifecycleLock.unlock()

        // Cross-process exclusivity: if the main app's node holds the wallet
        // dir (including its 30s background grace period), do NOT start a
        // second node or touch the LDK DB — defer to the app.
        guard NodeDirLock.shared.tryAcquire(dataDir: dataDir) else {
            lifecycleLock.lock()
            buildInFlight = false
            lifecycleLock.unlock()
            logger.log("Wallet dir locked by main app; deferring payment")
            UserDefaults(suiteName: Constants.appGroup)?.set(true, forKey: "pending_push_payment")
            cleanup()
            finish(content)
            return
        }

        // Bail before the expensive build if the window already expired.
        lifecycleLock.lock()
        let expiredBeforeBuild = timeExpired
        lifecycleLock.unlock()
        if expiredBeforeBuild {
            lifecycleLock.lock()
            buildInFlight = false
            lifecycleLock.unlock()
            logger.log("Time expired before node build; releasing")
            UserDefaults(suiteName: Constants.appGroup)?.set(true, forKey: "pending_push_payment")
            cleanup()
            return // expire handler already delivered the notification
        }

        logger.log("Building node from \(dataDir.path)")

        // Build and start node
        do {
            let node = try nodeStarter.buildNode(dataDir: dataDir, logger: logger)
            self.node = node
            // If the execution window expired mid-build, the expire handler
            // deferred stop/release to us: hand the wallet dir back now.
            lifecycleLock.lock()
            buildInFlight = false
            let expiredDuringBuild = timeExpired
            lifecycleLock.unlock()
            if expiredDuringBuild {
                logger.log("Time expired during node build; stopping node and releasing")
                UserDefaults(suiteName: Constants.appGroup)?.set(true, forKey: "pending_push_payment")
                cleanup() // stops the node and releases the wallet-dir lock
                return // expire handler already delivered the notification
            }
            logger.log("Node started")

            // Connect to LSP
            do {
                try nodeStarter.connectToLSP(node: node)
            } catch {
                logger.log("LSP connect failed (continuing anyway): \(error)")
            }
            // Wait for connection
            Thread.sleep(forTimeInterval: 3)

            startHeartbeat()

            // Chain-freshness gate (see #243) — user_to_lsp only: never keysend on a stale
            // chain tip. An outbound HTLC built on an old best block understates its expiry,
            // and LDK later force-closes on it. Waits here on the background processing
            // queue (never the main thread), before any send claim is taken.
            if direction == .userToLsp {
                switch waitForFreshLightningSync(node: node) {
                case .fresh:
                    break
                case .deadlineReached:
                    stopHeartbeat()
                    UserDefaults(suiteName: Constants.appGroup)?.set(true, forKey: "pending_push_payment")
                    cleanup()
                    finish(contentMutator.buildPending(
                        base: content,
                        title: "Payment Pending",
                        body: "Open app to process stability payment"
                    ))
                    return
                case .cancelled:
                    // Expiry got there first: the expire handler already set the pending
                    // flag and delivered the notification. Just hand back the wallet dir.
                    stopHeartbeat()
                    cleanup()
                    return
                }
            }

            processPayment(
                node: node,
                content: content,
                direction: direction,
                contentHandler: contentHandler
            )
        } catch {
            lifecycleLock.lock()
            buildInFlight = false
            lifecycleLock.unlock()
            logger.log("NODE FAILED: \(error)")
            cleanup()
            content.title = "Payment Pending"
            content.body = "Open app to process your payment"
            finish(content)
        }
    }

    private func processPayment(
        node: Node,
        content: UNMutableNotificationContent,
        direction: PaymentDirection,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        guard let db else {
            finishWithError(content: content, contentHandler: contentHandler)
            return
        }

        let handler = PaymentHandlerFactory.handler(for: direction)
        handler
            .handle(node: node, db: db, priceFetcher: priceFetcher, baseContent: content,
                    mutator: contentMutator) { [weak self] resultContent, newPendingState in
                guard let self else { return }
                self.logger.log("Payment handled: newPendingState=\(String(describing: newPendingState))")

                let shared = UserDefaults(suiteName: Constants.appGroup)
                if let newPendingState {
                    shared?.set(newPendingState, forKey: "pending_push_payment")
                }
                shared?.synchronize()

                self.stopHeartbeat()
                self.cleanup()
                self.finish(resultContent)
            }
    }

    private enum FreshnessWaitOutcome {
        case fresh
        case deadlineReached
        case cancelled
    }

    /// Poll `node.status()` every 500ms until the Lightning-wallet sync timestamp is fresh,
    /// the extension-wide deadline (`extensionStart` + 20s) passes, or the run is cancelled
    /// by expiry. The timestamp is persisted by LDK, so a run that kept node_metrics can be
    /// fresh immediately off the main app's last sync; otherwise this rides the initial
    /// background sync ldk-node kicks off at start(). Each iteration re-checks lifecycle
    /// state under the lock so expiry cancels the wait promptly.
    private func waitForFreshLightningSync(node: Node) -> FreshnessWaitOutcome {
        let deadline = extensionStart.addingTimeInterval(Self.userToLspFreshnessDeadlineSecs)
        let initialAge = StabilityFreshness.syncAgeSecs(
            node.status().latestLightningWalletSyncTimestamp,
            now: UInt64(Date().timeIntervalSince1970)
        )
        let waitStart = Date()
        logGateEvent("stability_background_attempted", initialAge: initialAge, waitStart: waitStart)

        var waited = false
        while true {
            lifecycleLock.lock()
            let aborted = timeExpired || didFinish
            lifecycleLock.unlock()
            if aborted {
                logGateEvent("stability_background_deferred_stale_sync",
                             initialAge: initialAge, waitStart: waitStart, outcome: "cancelled")
                return .cancelled
            }
            let now = UInt64(Date().timeIntervalSince1970)
            if StabilityFreshness.isFresh(node.status().latestLightningWalletSyncTimestamp, now: now) {
                logGateEvent(waited ? "stability_background_sent_after_sync" : "stability_background_sent_fresh",
                             initialAge: initialAge, waitStart: waitStart, outcome: "fresh")
                return .fresh
            }
            if Date() >= deadline {
                logGateEvent("stability_background_deferred_stale_sync",
                             initialAge: initialAge, waitStart: waitStart, outcome: "deadline_reached")
                return .deadlineReached
            }
            waited = true
            Thread.sleep(forTimeInterval: Self.freshnessPollIntervalSecs)
        }
    }

    /// Structured pilot metric for the background user_to_lsp freshness gate. No wallet
    /// secrets or payment identifiers — platform, sync age, wait time, and outcome only.
    private func logGateEvent(
        _ event: String, initialAge: UInt64?, waitStart: Date, outcome: String? = nil
    ) {
        let waitedMs = Int(Date().timeIntervalSince(waitStart) * 1000)
        let age = initialAge.map { "\($0)" } ?? "null"
        let outcomeJson = outcome.map { "\"\($0)\"" } ?? "null"
        logger.log(
            "stability_gate {\"event\":\"\(event)\",\"platform\":\"ios\"," +
                "\"prev_sync_age_secs\":\(age),\"waited_ms\":\(waitedMs)," +
                "\"deadline_outcome\":\(outcomeJson)}"
        )
    }

    private func finishWithError(
        content: UNMutableNotificationContent,
        contentHandler _: @escaping (UNNotificationContent) -> Void
    ) {
        cleanup()
        content.title = "Payment Pending"
        content.body = "Open app to process your payment"
        finish(content)
    }

    // MARK: - Helpers

    private func isMainAppActive() -> Bool {
        let shared = UserDefaults(suiteName: Constants.appGroup)
        let lastActive = shared?.double(forKey: "main_app_last_active") ?? 0
        return (Date().timeIntervalSince1970 - lastActive) < 10
    }

    private func getDBPath() -> String {
        guard let container = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: Constants.appGroup) else {
            return ""
        }
        return container
            .appendingPathComponent("StableChannels")
            .appendingPathComponent("user")
            .appendingPathComponent("stablechannels.db")
            .path
    }

    private func hasSeed(dataDir: URL) -> Bool {
        let keySeedPath = dataDir.appendingPathComponent("keys_seed")
        let seedPhrasePath = dataDir.appendingPathComponent("seed_phrase")
        return FileManager.default.fileExists(atPath: keySeedPath.path)
            || FileManager.default.fileExists(atPath: seedPhrasePath.path)
    }

    // MARK: - Heartbeat

    private var heartbeatTimer: DispatchSourceTimer?
    private let heartbeatQueue = DispatchQueue(label: "com.stablechannels.heartbeat")

    private func startHeartbeat() {
        let timer = DispatchSource.makeTimerSource(queue: heartbeatQueue)
        timer.schedule(deadline: .now(), repeating: 2.0)
        timer.setEventHandler {
            UserDefaults(suiteName: Constants.appGroup)?
                .set(Date().timeIntervalSince1970, forKey: "nse_last_active")
        }
        timer.resume()
        heartbeatTimer = timer
    }

    private func stopHeartbeat() {
        if let timer = heartbeatTimer {
            heartbeatQueue.async {
                timer.cancel()
            }
            heartbeatTimer = nil
        }
    }

    // MARK: - Cleanup

    private func cleanup() {
        logger.log("CLEANUP")
        try? node?.stop()
        node = nil
        NodeDirLock.shared.release()
    }
}
