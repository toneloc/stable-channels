import UserNotifications
import LDKNode
import SQLite3

/// Notification Service Extension — starts a lightweight LDK node to handle
/// stability payments while the main app is killed.
///
/// Push payload includes `stability.direction`:
/// - `lsp_to_user`: LSP owes user sats (price dropped). Start node, wait for incoming payment.
/// - `user_to_lsp`: User owes LSP sats (price rose). Start node, calculate amount, send keysend.
class NotificationService: UNNotificationServiceExtension {
    private static let appGroup = "group.com.stablechannels.app"
    private static let lspPubkey = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    private static let lspAddress = "34.198.44.89:9735"
    private static let stableChannelTLVType: UInt64 = 13_377_331
    private static let satsInBTC: Double = 100_000_000.0
    private static let stabilityThresholdPercent: Double = 0.1

    private var contentHandler: ((UNNotificationContent) -> Void)?
    private var bestAttemptContent: UNMutableNotificationContent?
    private var node: Node?

    private enum PaymentInsertResult {
        case inserted, duplicate, failed, missingChannelRow
    }

    private struct PendingOutgoingStabilityPayment {
        let paymentId: String
        let amountMsat: UInt64
        let btcPrice: Double
        let createdAt: Int64
    }

    // MARK: - Logging

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

        // Parse direction from push payload
        let userInfo = request.content.userInfo
        let direction: String
        if let stability = userInfo["stability"] as? [String: Any],
           let dir = stability["direction"] as? String {
            direction = dir
        } else if let stabilityStr = userInfo["stability"] as? String,
                  let data = stabilityStr.data(using: .utf8),
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let dir = json["direction"] as? String {
            // Server may send stability as a JSON-encoded string
            direction = dir
        } else {
            direction = "lsp_to_user"
        }

        nseLog("didReceive: direction=\(direction) userInfo=\(userInfo)")

        let shared = UserDefaults(suiteName: Self.appGroup)
        shared?.set(true, forKey: "nse_processing")

        // Check if main app is active — skip node start if so
        let lastActive = shared?.double(forKey: "main_app_last_active") ?? 0
        let now = Date().timeIntervalSince1970
        if (now - lastActive) < 10 {
            nseLog("Main app is active, skipping node start")
            shared?.set(false, forKey: "nse_processing")
            // Flag so main app handles it
            shared?.set(true, forKey: "pending_push_payment")
            contentHandler(content)
            return
        }

        DispatchQueue.global(qos: .userInitiated).async {
            self.startNodeAndProcess(content: content, direction: direction, contentHandler: contentHandler)
        }
    }

    override func serviceExtensionTimeWillExpire() {
        nseLog("TIME EXPIRED")
        cleanup()
        if let content = bestAttemptContent, let handler = contentHandler {
            content.title = "Payment Pending"
            content.body = "Open app to process your payment"
            handler(content)
        }
    }

    // MARK: - Node Start + Process

    private func startNodeAndProcess(
        content: UNMutableNotificationContent,
        direction: String,
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
        let keySeedPath = dataDir.appendingPathComponent("keys_seed")
        let seedPhrasePath = dataDir.appendingPathComponent("seed_phrase")

        guard FileManager.default.fileExists(atPath: keySeedPath.path)
            || FileManager.default.fileExists(atPath: seedPhrasePath.path) else {
            nseLog("FAILED: No seed (checked keys_seed and seed_phrase)")
            cleanup()
            contentHandler(content)
            return
        }

        nseLog("Building node from \(dataDir.path)")

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

            // Derive node entropy (now passed to build()): prefer the seed_phrase
            // mnemonic if present, else fall back to the existing keys_seed file.
            let nodeEntropy: NodeEntropy
            if FileManager.default.fileExists(atPath: seedPhrasePath.path),
               let words = try? String(contentsOfFile: seedPhrasePath.path, encoding: .utf8)
               .trimmingCharacters(in: .whitespacesAndNewlines),
               !words.isEmpty {
                nseLog("Using seed_phrase mnemonic")
                nodeEntropy = NodeEntropy.fromBip39Mnemonic(mnemonic: words, passphrase: nil)
            } else {
                nodeEntropy = try NodeEntropy.fromSeedPath(seedPath: keySeedPath.path)
            }

            // Relaxed sync intervals — NSE doesn't need frequent syncing
            let syncConfig = EsploraSyncConfig(
                backgroundSyncConfig: BackgroundSyncConfig(
                    onchainWalletSyncIntervalSecs: 600,
                    lightningWalletSyncIntervalSecs: 600,
                    feeRateCacheUpdateIntervalSecs: 3600
                ),
                timeoutsConfig: SyncTimeoutsConfig(
                    onchainWalletSyncTimeoutSecs: 60,
                    lightningWalletSyncTimeoutSecs: 60,
                    feeRateCacheUpdateTimeoutSecs: 60,
                    txBroadcastTimeoutSecs: 30,
                    perRequestTimeoutSecs: 15
                )
            )
            builder.setChainSourceEsplora(
                serverUrl: "https://blockstream.info/api",
                config: syncConfig
            )

            // --- Strip gossip from DB so build() doesn't OOM the NSE ---
            let ldkDbPath = dataDir.appendingPathComponent("ldk_node_data.sqlite")
            Self.stripGossipFromDB(path: ldkDbPath.path)

            // --- Diagnostics ---
            if let attrs = try? FileManager.default.attributesOfItem(atPath: ldkDbPath.path),
               let dbSize = attrs[.size] as? UInt64 {
                nseLog("DIAG: ldk_node_data.sqlite = \(dbSize) bytes (after strip)")
            }
            let memUsage = Self.residentMemoryBytes()
            nseLog("DIAG: memory before build() = \(memUsage / 1024)KB")

            let ldkNode = try builder.build(nodeEntropy: nodeEntropy)

            let memAfterBuild = Self.residentMemoryBytes()
            nseLog(
                "DIAG: memory after build() = \(memAfterBuild / 1024)KB (delta +\((memAfterBuild - memUsage) / 1024)KB)"
            )

            try ldkNode.start()
            self.node = ldkNode

            let memAfterStart = Self.residentMemoryBytes()
            nseLog("DIAG: memory after start() = \(memAfterStart / 1024)KB")
            nseLog("Node started, connecting to LSP")

            try? ldkNode.connect(
                nodeId: Self.lspPubkey,
                address: Self.lspAddress,
                persist: true
            )

            // Wait for connection
            Thread.sleep(forTimeInterval: 3)

            // Start heartbeat timer
            let heartbeatTimer = Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { _ in
                UserDefaults(suiteName: Self.appGroup)?
                    .set(Date().timeIntervalSince1970, forKey: "nse_last_active")
            }
            RunLoop.current.add(heartbeatTimer, forMode: .common)

            // Branch on direction
            switch direction {
            case "user_to_lsp":
                handleUserToLSP(node: ldkNode, dataDir: dataDir, content: content, contentHandler: contentHandler)
            case "incoming_payment":
                handleIncomingPayment(node: ldkNode, dataDir: dataDir, content: content, contentHandler: contentHandler)
            default: // "lsp_to_user"
                handleLSPToUser(node: ldkNode, dataDir: dataDir, content: content, contentHandler: contentHandler)
            }

            heartbeatTimer.invalidate()

        } catch {
            nseLog("NODE FAILED: \(error)")
            content.title = "Payment Pending"
            content.body = "Open app to process your payment"
            cleanup()
            contentHandler(content)
        }
    }

    // MARK: - lsp_to_user: Wait for Incoming Payment

    private func handleLSPToUser(
        node: Node,
        dataDir: URL,
        content: UNMutableNotificationContent,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        nseLog("lsp_to_user: Waiting for incoming payment")

        let dbPath = dataDir.appendingPathComponent("stablechannels.db").path
        let startTime = Date()
        let timeout: TimeInterval = 22 // Leave ~8s for cleanup + notification delivery
        var received = false
        var price = 0.0

        eventLoop: while Date().timeIntervalSince(startTime) < timeout {
            if let event = node.nextEvent() {
                nseLog("Event: \(event)")
                switch event {
                case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
                    let isStabilityPayment = customRecords.contains { $0.typeNum == Self.stableChannelTLVType && $0.value == Data([1]) }
                    // Always provide a non-nil ID for dedup so replays don't insert duplicates.
                    let payId = paymentId.map { "\($0)" } ?? "\(paymentHash)"
                    if price <= 0 { price = Self.fetchBTCPrice() }
                    if isStabilityPayment {
                        let amountSats = amountMsat / 1000
                        let result = recordPaymentAndMaybeUpdateBackingInDB(
                            dbPath: dbPath, paymentId: payId, paymentType: "stability",
                            direction: "received", amountMsat: amountMsat, btcPrice: price,
                            backingDeltaSats: Int64(amountSats),
                            userChannelId: activeUserChannelId(dbPath: dbPath)
                        )
                        switch result {
                        case .inserted, .duplicate:
                            try? node.eventHandled()
                            if price > 0 {
                                let usd = Double(amountSats) / Self.satsInBTC * price
                                content.title = "Stability Payment Received"
                                content.body = String(format: "$%.2f received", usd)
                            } else {
                                content.title = "Stability Payment Received"
                                content.body = "\(amountSats) sats received"
                            }
                            if result == .inserted { nseLog("Updated backingSats += \(amountSats) (delta)") }
                            UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "pending_push_payment")
                            received = true
                        case .failed, .missingChannelRow:
                            nseLog("DB write failed for stability payment — not acknowledging, LDK will retry")
                        }
                    } else {
                        let result = recordPaymentAndMaybeUpdateBackingInDB(
                            dbPath: dbPath, paymentId: payId, paymentType: "lightning",
                            direction: "received", amountMsat: amountMsat, btcPrice: price,
                            backingDeltaSats: nil
                        )
                        switch result {
                        case .inserted, .duplicate:
                            try? node.eventHandled()
                        case .failed, .missingChannelRow:
                            nseLog("DB write failed for non-stability payment — not acknowledging")
                        }
                        nseLog("Non-stability payment (\(amountMsat / 1000) sats), continuing to poll")
                    }
                default:
                    try? node.eventHandled()
                }
                if received { break eventLoop }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }

        if !received {
            content.title = "Payment Pending"
            content.body = "Open app to receive your payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
        }

        cleanup()
        contentHandler(content)
    }

    // MARK: - incoming_payment: Wake node to receive any pending payments

    private func handleIncomingPayment(
        node: Node,
        dataDir: URL,
        content: UNMutableNotificationContent,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        nseLog("incoming_payment: Waking node to receive payments")

        let startTime = Date()
        let timeout: TimeInterval = 22
        var received = false
        var persistenceFailed = false
        var totalMsat: UInt64 = 0
        var price = 0.0
        let dbPath = dataDir.appendingPathComponent("stablechannels.db").path

        while Date().timeIntervalSince(startTime) < timeout {
            if let event = node.nextEvent() {
                nseLog("Event: \(event)")
                switch event {
                case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
                    if price <= 0 { price = Self.fetchBTCPrice() }
                    let payId = paymentId.map { "\($0)" } ?? "\(paymentHash)"
                    let isStabilityPayment = customRecords.contains {
                        $0.typeNum == Self.stableChannelTLVType && $0.value == Data([1])
                    }
                    let result: PaymentInsertResult
                    if isStabilityPayment {
                        result = recordPaymentAndMaybeUpdateBackingInDB(
                            dbPath: dbPath,
                            paymentId: payId,
                            paymentType: "stability",
                            direction: "received",
                            amountMsat: amountMsat,
                            btcPrice: price,
                            backingDeltaSats: Int64(amountMsat / 1000),
                            userChannelId: activeUserChannelId(dbPath: dbPath)
                        )
                    } else {
                        result = recordPaymentAndMaybeUpdateBackingInDB(
                            dbPath: dbPath,
                            paymentId: payId,
                            paymentType: "lightning",
                            direction: "received",
                            amountMsat: amountMsat,
                            btcPrice: price,
                            backingDeltaSats: nil
                        )
                    }
                    switch result {
                    case .inserted, .duplicate:
                        try? node.eventHandled()
                        totalMsat += amountMsat
                        received = true
                        nseLog("Payment persisted before ack: \(amountMsat / 1000) sats")
                    case .failed, .missingChannelRow:
                        persistenceFailed = true
                        nseLog("DB write failed for incoming payment — not acknowledging")
                    }
                // Keep polling — there might be more payments
                default:
                    try? node.eventHandled()
                }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }

        if received {
            let totalSats = totalMsat / 1000
            if price > 0 {
                let usd = Double(totalSats) / Self.satsInBTC * price
                content.title = "Payment Received"
                content.body = String(format: "$%.2f received", usd)
            } else {
                content.title = "Payment Received"
                content.body = "\(totalSats) sats received"
            }
            content.sound = .default
            UserDefaults(suiteName: Self.appGroup)?
                .set(persistenceFailed, forKey: "pending_push_payment")
        } else if persistenceFailed {
            content.title = "Payment Pending"
            content.body = "Open app to finish recording your payment"
            content.sound = nil
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
        } else {
            // No payment arrived — this wake push just kept the node online briefly.
            // Can't fully suppress an NSE notification, so show a minimal message.
            content.title = ""
            content.body = ""
            content.sound = nil
        }

        cleanup()
        contentHandler(content)
    }

    // MARK: - user_to_lsp: Calculate and Send Payment

    private func handleUserToLSP(
        node: Node,
        dataDir: URL,
        content: UNMutableNotificationContent,
        contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        nseLog("user_to_lsp: Calculating stability payment")

        let dbPath = dataDir.appendingPathComponent("stablechannels.db").path
        guard reconcilePendingOutgoingPayment(dbPath: dbPath, node: node) else {
            content.title = "Payment Sent"
            content.body = "Open app to finish syncing the stability payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
            cleanup()
            contentHandler(content)
            return
        }

        // Cooldown: skip if we sent a stability payment recently
        let shared = UserDefaults(suiteName: Self.appGroup)
        shared?.synchronize()
        let lastSent = shared?.double(forKey: "nse_last_stability_sent") ?? 0
        let secondsSinceLast = Date().timeIntervalSince1970 - lastSent
        nseLog("Cooldown check: lastSent=\(lastSent), secondsSince=\(Int(secondsSinceLast))")
        if lastSent > 0 && secondsSinceLast < 120 {
            nseLog("Cooldown: \(Int(secondsSinceLast))s since last payment, skipping (120s required)")
            content.title = "Stability Check"
            content.body = "Position is stable"
            cleanup()
            contentHandler(content)
            return
        }

        // 1. Read channel state from SQLite
        guard let channelState = readChannelState(dbPath: dbPath) else {
            nseLog("Failed to read channel state from DB")
            content.title = "Payment Pending"
            content.body = "Open app to process stability payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
            cleanup()
            contentHandler(content)
            return
        }

        // Use backingSats from DB directly — it was set at trade time and reset after payments
        let backingSats = channelState.backingSats

        nseLog("Channel state: expectedUSD=\(channelState.expectedUSD), backingSats=\(backingSats)")

        guard channelState.expectedUSD >= 0.01 else {
            nseLog("expectedUSD too small, skipping")
            cleanup()
            contentHandler(content)
            return
        }

        // 2. Fetch fresh BTC price
        let price = Self.fetchBTCPrice()
        guard price > 0 else {
            nseLog("Price fetch failed, skipping payment")
            content.title = "Payment Pending"
            content.body = "Open app to process stability payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
            cleanup()
            contentHandler(content)
            return
        }

        nseLog("Price: $\(String(format: "%.0f", price))")

        // 3. Calculate stability payment using backing_sats from DB
        let stableUSDValue = Double(backingSats) / Self.satsInBTC * price
        let targetUSD = channelState.expectedUSD
        let dollarsFromPar = stableUSDValue - targetUSD
        let percentFromPar = targetUSD > 0 ? abs(dollarsFromPar / targetUSD) * 100.0 : 0.0

        nseLog(
            "Stability check: stableUSD=\(String(format: "%.2f", stableUSDValue)), target=\(String(format: "%.2f", targetUSD)), pct=\(String(format: "%.3f", percentFromPar))%"
        )

        guard percentFromPar >= Self.stabilityThresholdPercent && abs(dollarsFromPar) >= 0.25 else {
            nseLog(
                "Within threshold (pct=\(String(format: "%.3f", percentFromPar))%, drift=$\(String(format: "%.2f", abs(dollarsFromPar)))), no payment needed"
            )
            content.title = "Stability Check"
            content.body = "Position is stable"
            cleanup()
            contentHandler(content)
            return
        }

        // User is above expected (price rose) — user should pay LSP
        guard stableUSDValue > targetUSD else {
            nseLog("User below expected in user_to_lsp direction — unexpected, skipping")
            cleanup()
            contentHandler(content)
            return
        }

        // 4. Calculate payment amount in msats
        let dollarsAbs = abs(dollarsFromPar)
        let btcAmount = dollarsAbs / price
        let amountMsat = UInt64(btcAmount * Self.satsInBTC * 1000)
        let amountSats = amountMsat / 1000

        nseLog("Sending \(amountSats) sats (\(amountMsat) msat) to LSP")

        // Claim the durable send slot (cross-process atomic via BEGIN IMMEDIATE).
        // If the foreground app — or a previous run — already holds it, abort this run.
        guard claimPendingSend(dbPath: dbPath, amountMsat: amountMsat, price: price) else {
            nseLog("Pending send slot already claimed (or claim failed) — aborting this run")
            content.title = "Payment Pending"
            content.body = "Open app to process stability payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
            cleanup()
            contentHandler(content)
            return
        }

        // 5. Send keysend with stability TLV marker
        do {
            let tlvRecord = CustomTlvRecord(typeNum: Self.stableChannelTLVType, value: Data([1])) // marker byte
            let paymentId = try node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat: amountMsat,
                nodeId: Self.lspPubkey,
                routeParameters: nil,
                customTlvs: [tlvRecord]
            )
            nseLog("Keysend sent: \(paymentId)")

            let cooldownDefaults = UserDefaults(suiteName: Self.appGroup)
            let paymentIdString = "\(paymentId)"
            let guardSaved = setPendingSendPaymentId(dbPath: dbPath, paymentId: paymentIdString)
            cooldownDefaults?.set(Date().timeIntervalSince1970, forKey: "nse_last_stability_sent")
            cooldownDefaults?.synchronize()
            nseLog("Cooldown: stamped at \(Date().timeIntervalSince1970)")

            guard guardSaved else {
                nseLog("Payment sent but payment ID guard update failed — blocking future sends")
                content.title = "Payment Sent"
                content.body = "Open app to finish syncing the stability payment"
                UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
                cleanup()
                contentHandler(content)
                return
            }

            // Settle-on-success: do NOT debit backing here. The NSE can be suspended
            // before the HTLC settles, so debiting at send would silently lose backing if
            // the payment later fails. The marker (with the real payment id) stays as the
            // in-flight guard; the authoritative debit is applied exactly once — deduped
            // by payment_id — by whichever process next reconciles the marker against LDK
            // and observes a succeeded payment (foreground reconcile / success handler, or
            // a later NSE run). Leave pending_push_payment set so the app finalizes it.
            nseLog("Keysend initiated; leaving marker for reconcile to finalize on success")
            content.title = "Stability Payment Sent"
            content.body = String(format: "Sent %d sats ($%.2f) to maintain stable position", amountSats, dollarsAbs)
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
        } catch {
            clearPendingSend(dbPath: dbPath)
            nseLog("Keysend failed: \(error)")
            content.title = "Payment Pending"
            content.body = "Open app to process stability payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
        }

        cleanup()
        contentHandler(content)
    }

    // MARK: - Pending Stability Send (durable cross-process marker in stablechannels.db)

    private static let pendingSendTableSQL = """
        CREATE TABLE IF NOT EXISTS pending_stability_send (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            payment_id TEXT NOT NULL,
            amount_msat INTEGER NOT NULL,
            price REAL NOT NULL,
            created_at INTEGER NOT NULL
        )
        """

    /// Open stablechannels.db for pending-send operations with the busy timeout
    /// set and the marker table guaranteed to exist (either process may create it).
    private func openPendingSendDB(dbPath: String) -> OpaquePointer? {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK else {
            nseLog("pendingSend: open failed")
            sqlite3_close(db)
            return nil
        }
        sqlite3_busy_timeout(db, 2000)
        guard sqlite3_exec(db, Self.pendingSendTableSQL, nil, nil, nil) == SQLITE_OK else {
            nseLog("pendingSend: CREATE TABLE failed")
            sqlite3_close(db)
            return nil
        }
        return db
    }

    /// Claim the single outgoing-send slot under BEGIN IMMEDIATE so the NSE and
    /// the foreground app can never both send for the same drift.
    /// Returns true only if this caller inserted the marker row.
    private func claimPendingSend(dbPath: String, amountMsat: UInt64, price: Double) -> Bool {
        guard let db = openPendingSendDB(dbPath: dbPath) else { return false }
        defer { sqlite3_close(db) }

        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else {
            nseLog("claimPendingSend: BEGIN failed")
            return false
        }
        var checkStmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "SELECT id FROM pending_stability_send WHERE id = 1", -1, &checkStmt, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }
        let alreadyClaimed = sqlite3_step(checkStmt) == SQLITE_ROW
        sqlite3_finalize(checkStmt)
        if alreadyClaimed {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            nseLog("claimPendingSend: slot already claimed")
            return false
        }
        var stmt: OpaquePointer?
        let insertSql = "INSERT INTO pending_stability_send (id, payment_id, amount_msat, price, created_at) VALUES (1, '', ?, ?, ?)"
        guard sqlite3_prepare_v2(db, insertSql, -1, &stmt, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }
        sqlite3_bind_int64(stmt, 1, Int64(amountMsat))
        sqlite3_bind_double(stmt, 2, price)
        sqlite3_bind_int64(stmt, 3, Int64(Date().timeIntervalSince1970))
        let inserted = sqlite3_step(stmt) == SQLITE_DONE
        sqlite3_finalize(stmt)
        guard inserted, sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return false
        }
        return true
    }

    /// Attach the real payment id to the claimed send marker once the keysend returns.
    private func setPendingSendPaymentId(dbPath: String, paymentId: String) -> Bool {
        guard let db = openPendingSendDB(dbPath: dbPath) else { return false }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, "UPDATE pending_stability_send SET payment_id = ? WHERE id = 1", -1, &stmt, nil) == SQLITE_OK else {
            return false
        }
        sqlite3_bind_text(stmt, 1, (paymentId as NSString).utf8String, -1, nil)
        let ok = sqlite3_step(stmt) == SQLITE_DONE
        sqlite3_finalize(stmt)
        return ok
    }

    private func loadPendingSend(dbPath: String) -> PendingOutgoingStabilityPayment? {
        guard let db = openPendingSendDB(dbPath: dbPath) else { return nil }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT payment_id, amount_msat, price, created_at FROM pending_stability_send WHERE id = 1"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return nil }
        defer { sqlite3_finalize(stmt) }
        guard sqlite3_step(stmt) == SQLITE_ROW else { return nil }

        let paymentId = sqlite3_column_text(stmt, 0).map { String(cString: $0) } ?? ""
        return PendingOutgoingStabilityPayment(
            paymentId: paymentId,
            amountMsat: UInt64(sqlite3_column_int64(stmt, 1)),
            btcPrice: sqlite3_column_double(stmt, 2),
            createdAt: sqlite3_column_int64(stmt, 3)
        )
    }

    private func clearPendingSend(dbPath: String) {
        guard let db = openPendingSendDB(dbPath: dbPath) else { return }
        defer { sqlite3_close(db) }
        sqlite3_exec(db, "DELETE FROM pending_stability_send WHERE id = 1", nil, nil, nil)
    }

    private func reconcilePendingOutgoingPayment(dbPath: String, node: Node?) -> Bool {
        guard var pending = loadPendingSend(dbPath: dbPath) else { return true }

        if pending.paymentId.isEmpty {
            // Process died mid-keysend. Resolve the marker against LDK's payment
            // store instead of blocking sends forever.
            guard let node else {
                nseLog("Previous outgoing payment marker is unresolved — refusing to send again")
                return false
            }
            let candidates = node.listPayments().filter { payment in
                guard payment.direction == .outbound,
                      payment.amountMsat == pending.amountMsat,
                      Int64(payment.latestUpdateTimestamp) >= pending.createdAt - 10,
                      case .spontaneous = payment.kind else { return false }
                return true
            }
            if let succeeded = candidates.first(where: { $0.status == .succeeded }) {
                // Sats left the channel — adopt the id and replay the debit below
                // (the atomic record dedups on payment_id).
                let adoptedId = "\(succeeded.id)"
                _ = setPendingSendPaymentId(dbPath: dbPath, paymentId: adoptedId)
                pending = PendingOutgoingStabilityPayment(
                    paymentId: adoptedId,
                    amountMsat: pending.amountMsat,
                    btcPrice: pending.btcPrice,
                    createdAt: pending.createdAt
                )
                nseLog("Adopted payment id \(adoptedId) for unresolved send marker")
            } else if candidates.contains(where: { $0.status == .pending }) {
                nseLog("Unresolved send marker still in flight — waiting")
                return false
            } else if candidates.contains(where: { $0.status == .failed }) {
                nseLog("Unresolved send marker matches a failed payment — clearing (no debit)")
                clearPendingSend(dbPath: dbPath)
                return true
            } else if Int64(Date().timeIntervalSince1970) - pending.createdAt > 120 {
                nseLog("Unresolved send marker never left the node — clearing")
                clearPendingSend(dbPath: dbPath)
                return true
            } else {
                nseLog("Unresolved send marker too young to resolve — waiting")
                return false
            }
        } else {
            // The marker already carries the real payment id. Under settle-on-success the
            // marker is held from send until the HTLC settles, so verify the outcome
            // against LDK before debiting: only a succeeded payment debits backing, a
            // failed one clears with no debit, and an in-flight one keeps waiting. This
            // prevents prematurely debiting (and never reverting) an in-flight send.
            guard let node else {
                nseLog("Outgoing send marker unresolved (no node) — refusing to send again")
                return false
            }
            let match = node.listPayments().first { "\($0.id)" == pending.paymentId }
            switch match?.status {
            case .some(.succeeded):
                break  // settled — fall through to record the debit exactly once
            case .some(.failed):
                clearPendingSend(dbPath: dbPath)
                nseLog("Outgoing send marker \(pending.paymentId) failed — clearing (no debit)")
                return true
            default:
                nseLog("Outgoing send marker \(pending.paymentId) still in flight — waiting")
                return false
            }
        }

        let result = recordPaymentAndMaybeUpdateBackingInDB(
            dbPath: dbPath,
            paymentId: pending.paymentId,
            paymentType: "stability",
            direction: "sent",
            amountMsat: pending.amountMsat,
            btcPrice: pending.btcPrice,
            backingDeltaSats: -Int64(pending.amountMsat / 1000),
            userChannelId: activeUserChannelId(dbPath: dbPath)
        )
        switch result {
        case .inserted, .duplicate:
            clearPendingSend(dbPath: dbPath)
            nseLog("Reconciled previously sent outgoing payment \(pending.paymentId)")
            return true
        case .failed, .missingChannelRow:
            nseLog("Could not reconcile previously sent payment — refusing to send again")
            return false
        }
    }

    // MARK: - Record Payment and Update Backing in DB

    /// Insert a payment and atomically update channel backing sats in one SQLite transaction.
    /// BEGIN IMMEDIATE is acquired first so the dedup check and INSERT are atomic cross-process.
    private func recordPaymentAndMaybeUpdateBackingInDB(
        dbPath: String,
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: UInt64,
        btcPrice: Double,
        backingDeltaSats: Int64?,
        userChannelId: String? = nil
    ) -> PaymentInsertResult {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK else {
            nseLog("recordPaymentAndMaybeBacking: open failed")
            return .failed
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 2000)

        // Acquire write lock first — dedup check runs while holding it, eliminating cross-process races.
        guard sqlite3_exec(db, "BEGIN IMMEDIATE", nil, nil, nil) == SQLITE_OK else {
            nseLog("recordPaymentAndMaybeBacking: BEGIN failed")
            return .failed
        }

        if let pid = paymentId, !pid.isEmpty {
            var checkStmt: OpaquePointer?
            if sqlite3_prepare_v2(db, "SELECT id FROM payments WHERE payment_id = ?", -1, &checkStmt, nil) == SQLITE_OK {
                sqlite3_bind_text(checkStmt, 1, (pid as NSString).utf8String, -1, nil)
                let alreadyExists = sqlite3_step(checkStmt) == SQLITE_ROW
                sqlite3_finalize(checkStmt)
                if alreadyExists {
                    sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                    nseLog("recordPaymentAndMaybeBacking: already exists, skipping")
                    return .duplicate
                }
            }
        }

        let amountUSD = btcPrice > 0 ? (Double(amountMsat) / 1000.0 / Self.satsInBTC) * btcPrice : 0.0
        var stmt: OpaquePointer?
        // Insert as 'pending' so the foreground app's pending-filtered status updates and
        // reconcile can act on the row; the settlement path flips it to completed/failed.
        let insertSql = "INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status) VALUES (?, ?, ?, ?, ?, ?, 'pending')"
        guard sqlite3_prepare_v2(db, insertSql, -1, &stmt, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return .failed
        }
        if let pid = paymentId { sqlite3_bind_text(stmt, 1, (pid as NSString).utf8String, -1, nil) } else { sqlite3_bind_null(stmt, 1) }
        sqlite3_bind_text(stmt, 2, (paymentType as NSString).utf8String, -1, nil)
        sqlite3_bind_text(stmt, 3, (direction as NSString).utf8String, -1, nil)
        sqlite3_bind_int64(stmt, 4, Int64(amountMsat))
        sqlite3_bind_double(stmt, 5, amountUSD)
        sqlite3_bind_double(stmt, 6, btcPrice)
        guard sqlite3_step(stmt) == SQLITE_DONE else {
            sqlite3_finalize(stmt)
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return .failed
        }
        sqlite3_finalize(stmt)

        if let delta = backingDeltaSats {
            // Target the backing UPDATE by the explicit user_channel_id — never by recency —
            // so a push-triggered payment can't credit/debit the wrong channel row.
            guard let ucid = userChannelId, !ucid.isEmpty else {
                nseLog("recordPaymentAndMaybeBacking: backing delta requested without user_channel_id — rolling back")
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .failed
            }
            var selectStmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, "SELECT stable_sats FROM channels WHERE user_channel_id = ?", -1, &selectStmt, nil) == SQLITE_OK else {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .failed
            }
            sqlite3_bind_text(selectStmt, 1, (ucid as NSString).utf8String, -1, nil)
            guard sqlite3_step(selectStmt) == SQLITE_ROW else {
                sqlite3_finalize(selectStmt)
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                nseLog("recordPaymentAndMaybeBacking: no channel row for user_channel_id=\(ucid) — rolling back")
                return .missingChannelRow
            }
            let currentBacking = sqlite3_column_int64(selectStmt, 0)
            sqlite3_finalize(selectStmt)

            // Clamp instead of refusing: this runs after a successful keysend, so
            // recording reality beats wedging reconcile forever.
            let newBacking = max(0, currentBacking + delta)
            if currentBacking + delta < 0 {
                nseLog("BACKING_CLAMPED: current=\(currentBacking) delta=\(delta) — clamping backing to 0")
            }
            let updateSql = "UPDATE channels SET stable_sats = ?, updated_at = strftime('%s', 'now') WHERE user_channel_id = ?"
            var updateStmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, updateSql, -1, &updateStmt, nil) == SQLITE_OK else {
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .failed
            }
            sqlite3_bind_int64(updateStmt, 1, newBacking)
            sqlite3_bind_text(updateStmt, 2, (ucid as NSString).utf8String, -1, nil)
            let stepRc = sqlite3_step(updateStmt)
            let changedRows = sqlite3_changes(db)
            sqlite3_finalize(updateStmt)
            guard stepRc == SQLITE_DONE, changedRows == 1 else {
                nseLog("recordPaymentAndMaybeBacking: UPDATE affected \(changedRows) rows, expected 1 — rolling back")
                sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
                return .failed
            }
        }

        guard sqlite3_exec(db, "COMMIT", nil, nil, nil) == SQLITE_OK else {
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            return .failed
        }
        nseLog("recordPaymentAndMaybeBacking: saved \(direction) \(amountMsat) msat (\(String(format: "%.2f", amountUSD)) USD)")
        return .inserted
    }

    // MARK: - Lightweight SQLite Reader

    struct ChannelState {
        let expectedUSD: Double
        let backingSats: UInt64
        let nativeSats: UInt64
        let receiverSats: UInt64
        let latestPrice: Double
        let userChannelId: String
    }

    /// Read channel state directly from SQLite using C API (no DatabaseService dependency).
    private func readChannelState(dbPath: String) -> ChannelState? {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            nseLog("SQLite open failed: \(dbPath)")
            return nil
        }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 2000)

        var stmt: OpaquePointer?
        /// Pick the single active channel deterministically. The returned user_channel_id is the
        /// stable key every backing UPDATE targets by — the write never re-selects by recency.
        let sql = """
            SELECT expected_usd, stable_sats, receiver_sats, latest_price, native_sats, user_channel_id
            FROM channels
            WHERE user_channel_id IS NOT NULL AND user_channel_id != ''
            ORDER BY updated_at DESC, channel_id DESC
            LIMIT 1
        """
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            nseLog("SQLite prepare failed")
            return nil
        }
        defer { sqlite3_finalize(stmt) }

        guard sqlite3_step(stmt) == SQLITE_ROW else {
            nseLog("No channel row found")
            return nil
        }

        let expectedUSD = sqlite3_column_double(stmt, 0)
        let backingSats = UInt64(sqlite3_column_int64(stmt, 1))
        let receiverSats = UInt64(sqlite3_column_int64(stmt, 2))
        let latestPrice = sqlite3_column_double(stmt, 3)
        let nativeSats = UInt64(sqlite3_column_int64(stmt, 4))
        let userChannelId = sqlite3_column_text(stmt, 5).map { String(cString: $0) } ?? ""

        return ChannelState(
            expectedUSD: expectedUSD,
            backingSats: backingSats,
            nativeSats: nativeSats,
            receiverSats: receiverSats,
            latestPrice: latestPrice,
            userChannelId: userChannelId
        )
    }

    /// Resolve the single active channel's user_channel_id — the stable key backing UPDATEs target.
    /// Returns nil when no channel row exists, in which case a backing update must fail (not guess).
    private func activeUserChannelId(dbPath: String) -> String? {
        guard let ucid = readChannelState(dbPath: dbPath)?.userChannelId, !ucid.isEmpty else { return nil }
        return ucid
    }

    // MARK: - Price Fetch

    /// Fetch BTC/USD price from 5 sources concurrently, return median.
    /// All requests fire at once via DispatchGroup so wall time = slowest feed, not sum.
    private static func fetchBTCPrice() -> Double {
        let lock = NSLock()
        var prices: [Double] = []
        let group = DispatchGroup()

        func append(_ p: Double) { lock.lock(); prices.append(p); lock.unlock() }

        // Bitstamp
        group.enter()
        if let url = URL(string: "https://www.bitstamp.net/api/v2/ticker/btcusd/") {
            URLSession.shared.dataTask(with: url) { data, _, _ in
                defer { group.leave() }
                if let data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let s = json["last"] as? String, let p = Double(s) { append(p) }
            }.resume()
        } else { group.leave() }

        // Coinbase
        group.enter()
        if let url = URL(string: "https://api.coinbase.com/v2/prices/spot?currency=USD") {
            URLSession.shared.dataTask(with: url) { data, _, _ in
                defer { group.leave() }
                if let data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let d = json["data"] as? [String: Any],
                   let s = d["amount"] as? String, let p = Double(s) { append(p) }
            }.resume()
        } else { group.leave() }

        // Blockchain.com
        group.enter()
        if let url = URL(string: "https://blockchain.info/ticker") {
            URLSession.shared.dataTask(with: url) { data, _, _ in
                defer { group.leave() }
                if let data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let usd = json["USD"] as? [String: Any],
                   let p = usd["last"] as? Double { append(p) }
            }.resume()
        } else { group.leave() }

        // Kraken
        group.enter()
        if let url = URL(string: "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD") {
            URLSession.shared.dataTask(with: url) { data, _, _ in
                defer { group.leave() }
                if let data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let result = json["result"] as? [String: Any],
                   let pair = result["XXBTZUSD"] as? [String: Any],
                   let c = pair["c"] as? [Any],
                   let s = c.first as? String, let p = Double(s) { append(p) }
            }.resume()
        } else { group.leave() }

        // CoinGecko
        group.enter()
        if let url = URL(string: "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd") {
            URLSession.shared.dataTask(with: url) { data, _, _ in
                defer { group.leave() }
                if let data, let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                   let btc = json["bitcoin"] as? [String: Any],
                   let p = btc["usd"] as? Double { append(p) }
            }.resume()
        } else { group.leave() }

        _ = group.wait(timeout: .now() + 8)

        guard !prices.isEmpty else { return 0 }
        let sorted = prices.sorted()
        return sorted[sorted.count / 2] // median
    }

    // MARK: - Cleanup

    private func cleanup() {
        nseLog("CLEANUP")
        try? node?.stop()
        node = nil
        UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "nse_processing")
    }

    /// Resident memory in bytes (RSS) via Mach task_info.
    private static func residentMemoryBytes() -> UInt64 {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size) / 4
        let result = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        return result == KERN_SUCCESS ? info.resident_size : 0
    }

    /// Delete network_graph, scorer, and node_metrics from the LDK SQLite DB.
    /// The NSE doesn't need gossip (it only routes to the LSP, a direct peer).
    /// This shrinks the DB from ~10MB to ~30KB and prevents OOM kills.
    private static func stripGossipFromDB(path: String) {
        var db: OpaquePointer?
        guard sqlite3_open(path, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }
        sqlite3_busy_timeout(db, 2000)

        // Check if network_graph exists and is large enough to matter
        var stmt: OpaquePointer?
        let sql = "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return }
        let hasGraph: Bool
        if sqlite3_step(stmt) == SQLITE_ROW {
            let size = sqlite3_column_int64(stmt, 0)
            hasGraph = size > 100_000 // Only strip if >100KB
        } else {
            hasGraph = false
        }
        sqlite3_finalize(stmt)

        guard hasGraph else { return }

        sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
        sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'scorer'", nil, nil, nil)
        sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'node_metrics'", nil, nil, nil)
        sqlite3_exec(db, "VACUUM", nil, nil, nil)

        NSLog("[NSE] Stripped gossip from DB")
    }
}
