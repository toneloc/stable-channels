import UserNotifications
import LDKNode

/// Notification Service Extension — starts a lightweight LDK node to receive payments
/// while the main app is killed.
///
/// The main app extracts the network_graph (gossip data) to a separate file on background,
/// leaving the SQLite small enough (~30KB) for the NSE to load within its 24MB memory limit.
/// Same data directory, same channel state — no copying, no dual state.
class NotificationService: UNNotificationServiceExtension {

    private static let appGroup = "group.com.stablechannels.app"
    private static let lspPubkey = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    private static let lspAddress = "100.25.168.115:9737"

    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttemptContent: UNMutableNotificationContent?
    private var node: Node?

    private func nseLog(_ msg: String) {
        NSLog("[NSE] \(msg)")
        guard let container = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: Self.appGroup) else { return }
        let logFile = container.appendingPathComponent("nse_debug.log")
        let line = "\(Date()): \(msg)\n"
        if let handle = try? FileHandle(forWritingTo: logFile) {
            handle.seekToEndOfFile()
            handle.write(line.data(using: .utf8)!)
            handle.closeFile()
        } else {
            try? line.data(using: .utf8)?.write(to: logFile)
        }
    }

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

        nseLog("STEP 1: didReceive called")

        let shared = UserDefaults(suiteName: Self.appGroup)
        shared?.set(true, forKey: "pending_push_payment")
        shared?.set(true, forKey: "nse_processing")

        // Check if main app is active
        let lastActive = shared?.double(forKey: "main_app_last_active") ?? 0
        let now = Date().timeIntervalSince1970
        if (now - lastActive) < 10 {
            nseLog("Main app is active, skipping node start")
            shared?.set(false, forKey: "nse_processing")
            contentHandler(content)
            return
        }

        DispatchQueue.global(qos: .userInitiated).async {
            self.startNodeAndReceive(content: content, contentHandler: contentHandler)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        nseLog("TIME EXPIRED")
        cleanup()
        if let content = bestAttemptContent, let handler = contentHandler {
            content.title = "Payment Pending"
            content.body = "Open app to receive your payment"
            handler(content)
        }
    }

    private func startNodeAndReceive(
        content: UNMutableNotificationContent,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        guard let container = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: Self.appGroup) else {
            nseLog("FAILED: No shared container")
            cleanup()
            contentHandler(content)
            return
        }

        let dataDir = container
            .appendingPathComponent("StableChannels")
            .appendingPathComponent("user")
        let seedPath = dataDir.appendingPathComponent("keys_seed")

        guard FileManager.default.fileExists(atPath: seedPath.path) else {
            nseLog("FAILED: No seed")
            cleanup()
            contentHandler(content)
            return
        }

        nseLog("STEP 2: Seed found, building node from shared data dir")

        do {
            var config = defaultConfig()
            config.storageDirPath = dataDir.path
            config.network = .bitcoin
            config.trustedPeers0conf = [Self.lspPubkey]
            config.anchorChannelsConfig = AnchorChannelsConfig(
                trustedPeersNoReserve: [Self.lspPubkey],
                perChannelReserveSats: 25_000
            )

            let builder = Builder.fromConfig(config: config)

            let syncConfig = EsploraSyncConfig(
                backgroundSyncConfig: BackgroundSyncConfig(
                    onchainWalletSyncIntervalSecs: 600,
                    lightningWalletSyncIntervalSecs: 600,
                    feeRateCacheUpdateIntervalSecs: 3600
                )
            )
            builder.setChainSourceEsplora(
                serverUrl: "https://blockstream.info/api",
                config: syncConfig
            )

            nseLog("STEP 3: Calling build()")
            let ldkNode = try builder.build()
            nseLog("STEP 4: build() succeeded!")

            try ldkNode.start()
            self.node = ldkNode
            nseLog("STEP 5: Node started, connecting to LSP")

            try? ldkNode.connect(
                nodeId: Self.lspPubkey,
                address: Self.lspAddress,
                persist: true
            )

            // Wait for connection, then send 5-sat keysend test
            nseLog("STEP 6: Waiting 5s for connection")
            Thread.sleep(forTimeInterval: 5)

            nseLog("STEP 7: Sending 5-sat keysend to LSP")
            var sent = false
            do {
                let paymentId = try ldkNode.spontaneousPayment().send(
                    amountMsat: 5_000,
                    nodeId: Self.lspPubkey,
                    routeParameters: nil
                )
                nseLog("STEP 8: Keysend sent! \(paymentId)")
                sent = true
            } catch {
                nseLog("Keysend failed: \(error)")
            }

            // Wait a bit for payment to settle
            Thread.sleep(forTimeInterval: 3)

            if sent {
                content.title = "NSE Payment Sent"
                content.body = "Sent 5 sats to LSP from background"
                UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "pending_push_payment")
            } else {
                content.title = "Payment Pending"
                content.body = "Open app to receive your payment"
                nseLog("No payment received within time limit")
            }

            cleanup()
            contentHandler(content)

        } catch {
            nseLog("NODE FAILED: \(error)")
            content.title = "Payment Pending"
            content.body = "Open app to receive your payment"
            cleanup()
            contentHandler(content)
        }
    }

    private func cleanup() {
        nseLog("CLEANUP")
        try? node?.stop()
        node = nil
        UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "nse_processing")
    }
}
