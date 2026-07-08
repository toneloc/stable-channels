import UserNotifications
import LDKNode
import SQLite3
import Darwin

/// Notification Service Extension — handles stability payments while main app is killed.
/// Uses dependency injection for testability and SOLID compliance.
class NotificationService: UNNotificationServiceExtension {
    // MARK: - Constants

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

    // MARK: - Entry Point

    override func didReceive(
        _ request: UNNotificationRequest,
        withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        self.contentHandler = contentHandler
        self.bestAttemptContent = (request.content.mutableCopy() as? UNMutableNotificationContent)

        guard let content = bestAttemptContent else {
            contentHandler(request.content)
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

        // Signal processing started
        let shared = UserDefaults(suiteName: Constants.appGroup)
        shared?.set(true, forKey: "nse_processing")

        // Skip if main app is active
        if isMainAppActive() {
            logger.log("Main app is active, skipping node start")
            shared?.set(false, forKey: "nse_processing")
            shared?.set(true, forKey: "pending_push_payment")
            contentHandler(content)
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
        cleanup()
        if let content = bestAttemptContent, let handler = contentHandler {
            content.title = "Payment Pending"
            content.body = "Open app to process your payment"
            handler(content)
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
            contentHandler(content)
            return
        }

        // Acquire lock
        guard acquireDBLock(container: container) else {
            cleanup()
            contentHandler(content)
            return
        }

        let dataDir = container
            .appendingPathComponent("StableChannels")
            .appendingPathComponent("user")

        // Check for seed
        guard hasSeed(dataDir: dataDir) else {
            logger.log("FAILED: No seed found")
            releaseLock()
            cleanup()
            contentHandler(content)
            return
        }

        logger.log("Building node from \(dataDir.path)")

        // Build and start node
        do {
            let node = try nodeStarter.buildNode(dataDir: dataDir, logger: logger)
            self.node = node
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
            processPayment(
                node: node,
                content: content,
                direction: direction,
                contentHandler: contentHandler
            )
        } catch {
            logger.log("NODE FAILED: \(error)")
            cleanup()
            content.title = "Payment Pending"
            content.body = "Open app to process your payment"
            contentHandler(content)
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
                    mutator: contentMutator) { [weak self] resultContent, shouldPersist in
                guard let self else { return }
                self.logger.log("Payment handled: persist=\(shouldPersist)")

                let shared = UserDefaults(suiteName: Constants.appGroup)
                shared?.set(shouldPersist, forKey: "pending_push_payment")
                shared?.synchronize()

                self.stopHeartbeat()
                self.cleanup()
                contentHandler(resultContent)
            }
    }

    private func finishWithError(
        content: UNMutableNotificationContent,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        releaseLock()
        cleanup()
        content.title = "Payment Pending"
        content.body = "Open app to process your payment"
        contentHandler(content)
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

    // MARK: - Lock Management

    private var lockFD: Int32 = -1

    private func acquireDBLock(container: URL) -> Bool {
        let lockFile = container.appendingPathComponent("stablechannels.db.lock")
        lockFile.withUnsafeFileSystemRepresentation { path in
            if let path {
                lockFD = open(path, O_WRONLY | O_CREAT, 0o644)
            }
        }
        guard lockFD >= 0 else { return false }
        return flock(lockFD, LOCK_EX) == 0
    }

    private func releaseLock() {
        if lockFD >= 0 {
            flock(lockFD, LOCK_UN)
            close(lockFD)
            lockFD = -1
        }
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
        releaseLock()
        try? node?.stop()
        node = nil
        UserDefaults(suiteName: Constants.appGroup)?.set(false, forKey: "nse_processing")
    }
}
