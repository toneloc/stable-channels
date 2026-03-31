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
        let stability = userInfo["stability"] as? [String: Any]
        let direction = stability?["direction"] as? String ?? "lsp_to_user"

        nseLog("didReceive: direction=\(direction)")

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

            // If wallet uses mnemonic (seed_phrase), set it on the builder
            if FileManager.default.fileExists(atPath: seedPhrasePath.path),
               let words = try? String(contentsOfFile: seedPhrasePath.path, encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines),
               !words.isEmpty {
                nseLog("Using seed_phrase mnemonic")
                builder.setEntropyBip39Mnemonic(mnemonic: words, passphrase: nil)
            }

            // Relaxed sync intervals — NSE doesn't need frequent syncing
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

            let ldkNode = try builder.build()
            try ldkNode.start()
            self.node = ldkNode
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

        let startTime = Date()
        let timeout: TimeInterval = 22  // Leave ~8s for cleanup + notification delivery
        var received = false
        var amountMsatTotal: UInt64 = 0
        var paymentIdStr: String?

        while Date().timeIntervalSince(startTime) < timeout {
            if let event = node.nextEvent() {
                nseLog("Event: \(event)")
                switch event {
                case .paymentReceived(let paymentId, _, let amountMsat, _):
                    amountMsatTotal = amountMsat
                    paymentIdStr = paymentId.map { "\($0)" }
                    nseLog("Payment received: \(amountMsat / 1000) sats")
                    try? node.eventHandled()
                    received = true
                default:
                    try? node.eventHandled()
                }
                if received { break }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }

        if received {
            let price = Self.fetchBTCPrice()
            let amountSats = amountMsatTotal / 1000
            if price > 0 {
                let usd = Double(amountSats) / Self.satsInBTC * price
                content.title = "Stability Payment Received"
                content.body = String(format: "$%.2f received", usd)
            } else {
                content.title = "Stability Payment Received"
                content.body = "\(amountSats) sats received"
            }
            let dbPath = dataDir.appendingPathComponent("stablechannels.db").path
            recordPaymentInDB(
                dbPath: dbPath,
                paymentId: paymentIdStr,
                paymentType: "stability",
                direction: "received",
                amountMsat: amountMsatTotal,
                btcPrice: price
            )
            UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "pending_push_payment")
        } else {
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
        var totalMsat: UInt64 = 0
        var lastPaymentId: String?
        let dbPath = dataDir.appendingPathComponent("stablechannels.db").path

        while Date().timeIntervalSince(startTime) < timeout {
            if let event = node.nextEvent() {
                nseLog("Event: \(event)")
                switch event {
                case .paymentReceived(let paymentId, _, let amountMsat, _):
                    totalMsat += amountMsat
                    lastPaymentId = paymentId.map { "\($0)" }
                    nseLog("Payment received: \(amountMsat / 1000) sats")
                    try? node.eventHandled()
                    received = true
                    // Keep polling — there might be more payments
                default:
                    try? node.eventHandled()
                }
            }
            Thread.sleep(forTimeInterval: 0.5)
        }

        if received {
            let price = Self.fetchBTCPrice()
            let totalSats = totalMsat / 1000
            if price > 0 {
                let usd = Double(totalSats) / Self.satsInBTC * price
                content.title = "Payment Received"
                content.body = String(format: "$%.2f received", usd)
            } else {
                content.title = "Payment Received"
                content.body = "\(totalSats) sats received"
            }
            recordPaymentInDB(
                dbPath: dbPath,
                paymentId: lastPaymentId,
                paymentType: "lightning",
                direction: "received",
                amountMsat: totalMsat,
                btcPrice: price
            )
            content.sound = .default
            UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "pending_push_payment")
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

        // 1. Read channel state from SQLite
        let dbPath = dataDir.appendingPathComponent("stablechannels.db").path
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

        nseLog("Stability check: stableUSD=\(String(format: "%.2f", stableUSDValue)), target=\(String(format: "%.2f", targetUSD)), pct=\(String(format: "%.3f", percentFromPar))%")

        guard percentFromPar >= Self.stabilityThresholdPercent else {
            nseLog("Within threshold, no payment needed")
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

        // 5. Send keysend with stability TLV marker
        do {
            let tlvRecord = CustomTlvRecord(typeNum: Self.stableChannelTLVType, value: [1])  // marker byte
            let paymentId = try node.spontaneousPayment().sendWithCustomTlvs(
                amountMsat: amountMsat,
                nodeId: Self.lspPubkey,
                routeParameters: nil,
                customTlvs: [tlvRecord]
            )
            nseLog("Keysend sent: \(paymentId)")

            // Wait for payment to settle
            Thread.sleep(forTimeInterval: 3)

            recordPaymentInDB(
                dbPath: dbPath,
                paymentId: "\(paymentId)",
                paymentType: "stability",
                direction: "sent",
                amountMsat: amountMsat,
                btcPrice: price
            )

            // Reset backing_sats to equilibrium after payment
            let newBacking = UInt64((channelState.expectedUSD / price) * Self.satsInBTC)
            updateBackingSatsInDB(dbPath: dbPath, backingSats: newBacking)
            nseLog("Reset backingSats to \(newBacking)")

            content.title = "Stability Payment Sent"
            content.body = String(format: "Sent %d sats ($%.2f) to maintain stable position", amountSats, dollarsAbs)
            UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "pending_push_payment")
        } catch {
            nseLog("Keysend failed: \(error)")
            content.title = "Payment Pending"
            content.body = "Open app to process stability payment"
            UserDefaults(suiteName: Self.appGroup)?.set(true, forKey: "pending_push_payment")
        }

        cleanup()
        contentHandler(content)
    }

    // MARK: - Record Payment in DB

    /// Write a payment record to stablechannels.db so the main app's history is complete.
    /// Uses INSERT with dedup on payment_id (matches DatabaseService.recordPayment schema).
    private func recordPaymentInDB(
        dbPath: String,
        paymentId: String?,
        paymentType: String,
        direction: String,
        amountMsat: UInt64,
        btcPrice: Double
    ) {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK else {
            nseLog("recordPayment: SQLite open failed")
            return
        }
        defer { sqlite3_close(db) }

        // Dedup: skip if payment_id already exists
        if let pid = paymentId, !pid.isEmpty {
            var checkStmt: OpaquePointer?
            let checkSql = "SELECT id FROM payments WHERE payment_id = ?"
            if sqlite3_prepare_v2(db, checkSql, -1, &checkStmt, nil) == SQLITE_OK {
                sqlite3_bind_text(checkStmt, 1, (pid as NSString).utf8String, -1, nil)
                if sqlite3_step(checkStmt) == SQLITE_ROW {
                    sqlite3_finalize(checkStmt)
                    nseLog("recordPayment: already exists, skipping")
                    return
                }
                sqlite3_finalize(checkStmt)
            }
        }

        let amountUSD = btcPrice > 0 ? (Double(amountMsat) / 1000.0 / Self.satsInBTC) * btcPrice : 0.0

        var stmt: OpaquePointer?
        let sql = """
            INSERT INTO payments (payment_id, payment_type, direction, amount_msat, amount_usd, btc_price, status)
            VALUES (?, ?, ?, ?, ?, ?, 'completed')
        """
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            nseLog("recordPayment: prepare failed")
            return
        }
        defer { sqlite3_finalize(stmt) }

        if let pid = paymentId {
            sqlite3_bind_text(stmt, 1, (pid as NSString).utf8String, -1, nil)
        } else {
            sqlite3_bind_null(stmt, 1)
        }
        sqlite3_bind_text(stmt, 2, (paymentType as NSString).utf8String, -1, nil)
        sqlite3_bind_text(stmt, 3, (direction as NSString).utf8String, -1, nil)
        sqlite3_bind_int64(stmt, 4, Int64(amountMsat))
        sqlite3_bind_double(stmt, 5, amountUSD)
        sqlite3_bind_double(stmt, 6, btcPrice)

        if sqlite3_step(stmt) == SQLITE_DONE {
            nseLog("recordPayment: saved \(direction) \(amountMsat) msat ($\(String(format: "%.2f", amountUSD)))")
        } else {
            nseLog("recordPayment: insert failed")
        }
    }

    private func updateBackingSatsInDB(dbPath: String, backingSats: UInt64) {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }
        let sql = "UPDATE stable_channels SET backing_sats = ? WHERE id = (SELECT MAX(id) FROM stable_channels)"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(stmt) }
        sqlite3_bind_int64(stmt, 1, Int64(backingSats))
        sqlite3_step(stmt)
    }

    // MARK: - Lightweight SQLite Reader

    struct ChannelState {
        let expectedUSD: Double
        let backingSats: UInt64
        let nativeSats: UInt64
        let receiverSats: UInt64
        let latestPrice: Double
    }

    /// Read channel state directly from SQLite using C API (no DatabaseService dependency).
    private func readChannelState(dbPath: String) -> ChannelState? {
        var db: OpaquePointer?
        guard sqlite3_open_v2(dbPath, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            nseLog("SQLite open failed: \(dbPath)")
            return nil
        }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT expected_usd, stable_sats, receiver_sats, latest_price, native_sats FROM channels LIMIT 1"
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

        return ChannelState(
            expectedUSD: expectedUSD,
            backingSats: backingSats,
            nativeSats: nativeSats,
            receiverSats: receiverSats,
            latestPrice: latestPrice
        )
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

        group.wait()

        guard !prices.isEmpty else { return 0 }
        let sorted = prices.sorted()
        return sorted[sorted.count / 2]  // median
    }

    // MARK: - Cleanup

    private func cleanup() {
        nseLog("CLEANUP")
        try? node?.stop()
        node = nil
        UserDefaults(suiteName: Self.appGroup)?.set(false, forKey: "nse_processing")
    }
}
