import Foundation
import SwiftUI
import LDKNode
import SQLite3

@Observable
class AppState {
    // MARK: - App Lifecycle

    enum Phase {
        case loading
        case onboarding
        case syncing
        case wallet
        case error(String)
    }

    var phase: Phase = .loading

    // MARK: - Services

    let nodeService = NodeService()
    let priceService = PriceService()
    var databaseService: DatabaseService?
    var tradeService: TradeService?

    // MARK: - State

    var stableChannel: StableChannel = .default
    var btcPrice: Double { priceService.currentPrice }
    var statusMessage: String = ""
    var paymentFlash: Bool = false
    var isChannelClosing: Bool = false
    var isOpeningChannel: Bool = false
    var isSyncing: Bool = false

    // Balance (derived) — initialized from cache for instant display
    var lightningBalanceSats: UInt64 = {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        return UInt64(bitPattern: Int64(ud?.integer(forKey: "cached_lightning_sats") ?? 0))
    }()
    var onchainBalanceSats: UInt64 = {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        return UInt64(bitPattern: Int64(ud?.integer(forKey: "cached_onchain_sats") ?? 0))
    }()
    var totalBalanceSats: UInt64 {
        if isChannelClosing { return onchainBalanceSats }
        if isOpeningChannel { return lightningBalanceSats > 0 ? lightningBalanceSats : onchainBalanceSats }
        return lightningBalanceSats + onchainBalanceSats
    }
    var onchainReceiveAddress: String?

    var totalBalanceUSD: Double {
        guard btcPrice > 0 else { return 0 }
        return Double(totalBalanceSats) / Double(Constants.satsInBTC) * btcPrice
    }

    var stableUSD: Double { stableChannel.expectedUSD.amount }
    var nativeBTC: Bitcoin { stableChannel.nativeChannelBTC }

    // MARK: - Internal

    private var eventObserver: NSObjectProtocol?
    private var stabilityTimer: Task<Void, Never>?
    private(set) var chainURL: String = Constants.primaryChainURL

    // Auto-sweep state
    private(set) var isSweeping = false
    private var sweepOnchainStart: UInt64 = 0
    private var prevOnchainSats: UInt64 = {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        return UInt64(bitPattern: Int64(ud?.integer(forKey: "cached_onchain_sats") ?? 0))
    }()
    private var fundingTxid: String?

    // Pending trade payments — deferred until PaymentSuccessful/PaymentFailed
    var pendingTradePayments: [String: PendingTradePayment] = [:]

    // Pending splice info
    var pendingSplice: PendingSplice?

    // MARK: - Startup

    func start() async {
        // Migrate data from old Application Support dir to shared App Group container
        migrateDataDirIfNeeded()

        // Wait for NSE to finish if it was recently active
        await waitForNSE()

        // Pick best esplora endpoint
        chainURL = await resolveChainURL()

        // Initialize database
        do {
            databaseService = try DatabaseService(dataDir: Constants.userDataDir)
        } catch {
            await MainActor.run { phase = .error("Database init failed: \(error.localizedDescription)") }
            return
        }

        tradeService = TradeService(nodeService: nodeService)

        // Set audit log path
        let auditPath = Constants.userDataDir.appendingPathComponent("audit_log.txt").path
        AuditService.setLogPath(auditPath)

        // Load saved channel state from DB
        loadChannelFromDB()

        // Seed historical price data for charts
        seedHistoricalPrices()

        // Backfill hourly prices from Kraken for smooth 1D/1W/1M charts
        Task { await backfillHourlyPrices() }

        // Seed price from cache so UI can compute native USD immediately
        if stableChannel.latestPrice > 0 {
            priceService.currentPrice = stableChannel.latestPrice
        }

        // Start price fetching
        priceService.startAutoRefresh()

        // Subscribe to LDK events
        subscribeToEvents()

        // Subscribe to push notifications (background wake)
        subscribeToPushNotifications()

        // Check for existing wallet (keys_seed from default path, OR seed_phrase from mnemonic path)
        let seedPath = Constants.userDataDir.appendingPathComponent("keys_seed")
        let seedPhrasePath = Constants.userDataDir.appendingPathComponent("seed_phrase")
        if FileManager.default.fileExists(atPath: seedPath.path)
            || FileManager.default.fileExists(atPath: seedPhrasePath.path) {
            // Show wallet immediately with cached data from DB
            let hasCachedData = !stableChannel.userChannelId.isEmpty
            await MainActor.run {
                phase = hasCachedData ? .wallet : .syncing
                if hasCachedData { isSyncing = true }
            }

            // Purge empty network graph from DB to force fresh RGS sync
            purgeEmptyNetworkGraph()

            do {
                try await nodeService.start(
                    network: .bitcoin,
                    esploraURL: chainURL,
                    mnemonic: ""  // Uses existing seed from data dir
                )
                // Store node_id in shared UserDefaults for NSE and push registration
                let nodeId = nodeService.nodeId
                if !nodeId.isEmpty {
                    UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .set(nodeId, forKey: "node_id")
                }

                await MainActor.run {
                    phase = .wallet
                    isSyncing = false
                    refreshBalances()
                    updateStableBalances()
                }
                startStabilityTimer()
                // Ensure LSP connection shortly after startup — initial connect may not have completed
                Task {
                    try? await Task.sleep(nanoseconds: 3_000_000_000)
                    await MainActor.run { ensureLSPConnected() }
                }
                // Re-register push token with node_id now that node is running
                reregisterPushTokenIfNeeded()
                // Check if NSE flagged a pending payment while app was killed
                await processPendingPushPayment()
            } catch {
                await MainActor.run { phase = .error("Node start failed: \(error.localizedDescription)") }
            }
        } else {
            // New wallet — auto-create
            await MainActor.run { phase = .syncing }
            do {
                try await nodeService.start(
                    network: .bitcoin,
                    esploraURL: chainURL,
                    mnemonic: ""
                )
                let nodeId = nodeService.nodeId
                if !nodeId.isEmpty {
                    UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .set(nodeId, forKey: "node_id")
                }
                await MainActor.run {
                    phase = .wallet
                    refreshBalances()
                    updateStableBalances()
                }
                startStabilityTimer()
                reregisterPushTokenIfNeeded()
            } catch {
                await MainActor.run { phase = .error("Wallet creation failed: \(error.localizedDescription)") }
            }
        }
    }

    // MARK: - Data Migration

    /// Migrate LDK data from old Application Support directory to shared App Group container.
    /// This is needed so the Notification Service Extension can access the node data.
    private func migrateDataDirIfNeeded() {
        let oldDir = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
            .appendingPathComponent("StableChannels").appendingPathComponent("user")
        let newDir = Constants.userDataDir
        let fm = FileManager.default

        // Only migrate if old seed exists and new seed does not
        guard fm.fileExists(atPath: oldDir.appendingPathComponent("keys_seed").path),
              !fm.fileExists(atPath: newDir.appendingPathComponent("keys_seed").path) else { return }

        try? fm.createDirectory(at: newDir, withIntermediateDirectories: true)
        if let contents = try? fm.contentsOfDirectory(atPath: oldDir.path) {
            for file in contents {
                try? fm.copyItem(
                    at: oldDir.appendingPathComponent(file),
                    to: newDir.appendingPathComponent(file)
                )
            }
        }

        AuditService.log("DATA_MIGRATED", data: [
            "from": oldDir.path,
            "to": newDir.path,
        ])
    }

    // MARK: - NSE Coordination

    /// Wait for the Notification Service Extension to finish if it's currently processing.
    /// Prevents two processes from running LDK on the same data directory simultaneously.
    private func waitForNSE() async {
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        var waited = 0
        while shared?.bool(forKey: "nse_processing") == true {
            try? await Task.sleep(nanoseconds: 1_000_000_000)  // 1 second
            waited += 1
            if waited >= 10 { break }  // timeout after 10 seconds
        }
        if waited > 0 {
            AuditService.log("NSE_WAIT", data: ["seconds": "\(waited)"])
        }
    }

    func stop() {
        stabilityTimer?.cancel()
        stabilityTimer = nil
        if let observer = eventObserver {
            NotificationCenter.default.removeObserver(observer)
            eventObserver = nil
        }
        if let observer = pushObserver {
            NotificationCenter.default.removeObserver(observer)
            pushObserver = nil
        }
        priceService.stopAutoRefresh()
        nodeService.stop()
    }

    private var backgroundStopWorkItem: DispatchWorkItem?
    private var backgroundTaskID: UIBackgroundTaskIdentifier = .invalid

    /// Stop the node and extract gossip data so the NSE can open the lightweight DB.
    /// Delays shutdown by 30 seconds so in-flight JIT channel opens can complete.
    func stopNodeForBackground() {
        print("[App] Scheduling node stop after 30s grace period")
        backgroundStopWorkItem?.cancel()
        backgroundTaskID = UIApplication.shared.beginBackgroundTask {
            self.performBackgroundStop()
        }
        let workItem = DispatchWorkItem { [weak self] in
            self?.performBackgroundStop()
        }
        backgroundStopWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + 30, execute: workItem)
    }

    /// Cancel a pending background stop (called when returning to foreground).
    func cancelBackgroundStop() {
        if let workItem = backgroundStopWorkItem {
            workItem.cancel()
            backgroundStopWorkItem = nil
            print("[App] Cancelled pending background stop")
        }
        if backgroundTaskID != .invalid {
            UIApplication.shared.endBackgroundTask(backgroundTaskID)
            backgroundTaskID = .invalid
        }
    }

    private func performBackgroundStop() {
        backgroundStopWorkItem = nil
        guard nodeService.isRunning else {
            if backgroundTaskID != .invalid {
                UIApplication.shared.endBackgroundTask(backgroundTaskID)
                backgroundTaskID = .invalid
            }
            return
        }
        print("[App] Stopping node for background")
        stabilityTimer?.cancel()
        stabilityTimer = nil
        nodeService.stop()
        extractGossipFromDB()
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.set(Date().timeIntervalSince1970, forKey: "main_app_last_active")
        if backgroundTaskID != .invalid {
            UIApplication.shared.endBackgroundTask(backgroundTaskID)
            backgroundTaskID = .invalid
        }
    }

    /// Restore gossip data and restart the node when returning to foreground.
    func restartNodeFromForeground() async {
        guard case .wallet = phase else { return }
        cancelBackgroundStop()
        if nodeService.isRunning {
            print("[App] Node still running (grace period), reconnecting to LSP")
            ensureLSPConnected()
            refreshBalances()
            updateStableBalances()
            return
        }
        print("[App] Restarting node from foreground")
        await waitForNSE()
        restoreGossipToDB()
        do {
            try await nodeService.start(
                network: .bitcoin,
                esploraURL: chainURL,
                mnemonic: ""
            )
            refreshBalances()
            updateStableBalances()
            // Reconcile backingSats — NSE may have received payments while backgrounded
            StabilityService.reconcileIncoming(&stableChannel)
            saveChannelToDB()
            reregisterPushTokenIfNeeded()
            startStabilityTimer()
            await processPendingPushPayment()
        } catch {
            print("[App] Node restart failed: \(error)")
        }
    }

    // MARK: - Gossip Data Management

    /// Extract network_graph blob from SQLite to a file, then delete it from the DB.
    /// This shrinks the DB from ~8.7MB to ~30KB so the NSE can load it.
    private func extractGossipFromDB() {
        let dbPath = Constants.userDataDir.appendingPathComponent("ldk_node_data.sqlite").path
        let gossipPath = Constants.userDataDir.appendingPathComponent("network_graph.bin").path

        var db: OpaquePointer?
        guard sqlite3_open(dbPath, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }

        // Read the network_graph blob
        var stmt: OpaquePointer?
        let query = "SELECT value FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, query, -1, &stmt, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(stmt) }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let blobSize = sqlite3_column_bytes(stmt, 0)
            if blobSize > 0, let blobPtr = sqlite3_column_blob(stmt, 0) {
                let data = Data(bytes: blobPtr, count: Int(blobSize))
                do {
                    try data.write(to: URL(fileURLWithPath: gossipPath))
                    print("[App] Saved network_graph (\(blobSize) bytes) to file")

                    // Delete from DB and compact
                    sqlite3_finalize(stmt)
                    stmt = nil
                    sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
                    sqlite3_exec(db, "VACUUM", nil, nil, nil)
                    print("[App] Stripped network_graph from DB")
                } catch {
                    print("[App] Failed to save network_graph: \(error)")
                }
            }
        }
    }

    /// Delete network_graph from SQLite if it's too small (empty graph with stale timestamp).
    /// Also deletes scorer data so routing starts fresh.
    private func purgeEmptyNetworkGraph() {
        let dbPath = Constants.userDataDir.appendingPathComponent("ldk_node_data.sqlite").path
        var db: OpaquePointer?
        guard sqlite3_open(dbPath, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(stmt) }

        let needsPurge: Bool
        if sqlite3_step(stmt) == SQLITE_ROW {
            let size = sqlite3_column_int64(stmt, 0)
            needsPurge = size < 500_000
            if needsPurge {
                print("[App] network_graph in DB too small (\(size) bytes), purging for fresh RGS sync")
            }
        } else {
            // No network_graph row at all — check if node_metrics has a stale timestamp
            needsPurge = true
            print("[App] No network_graph in DB, purging node_metrics for fresh RGS sync")
        }

        if needsPurge {
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'scorer'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'node_metrics'", nil, nil, nil)
        }
    }

    /// Restore network_graph blob from file back into SQLite.
    private func restoreGossipToDB() {
        let dbPath = Constants.userDataDir.appendingPathComponent("ldk_node_data.sqlite").path
        let gossipPath = Constants.userDataDir.appendingPathComponent("network_graph.bin")

        guard FileManager.default.fileExists(atPath: gossipPath.path) else { return }

        guard let data = try? Data(contentsOf: gossipPath) else { return }

        // Skip restoring an empty/corrupt graph — forces fresh full RGS sync
        if data.count < 1024 {
            print("[App] network_graph.bin too small (\(data.count) bytes), deleting for fresh sync")
            try? FileManager.default.removeItem(at: gossipPath)
            return
        }

        var db: OpaquePointer?
        guard sqlite3_open(dbPath, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }

        let upsert = "INSERT OR REPLACE INTO ldk_node_data (primary_namespace, secondary_namespace, key, value) VALUES ('', '', 'network_graph', ?)"
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, upsert, -1, &stmt, nil) == SQLITE_OK else { return }
        defer { sqlite3_finalize(stmt) }

        data.withUnsafeBytes { ptr in
            sqlite3_bind_blob(stmt, 1, ptr.baseAddress, Int32(data.count), nil)
        }
        sqlite3_step(stmt)

        // Clean up the file
        try? FileManager.default.removeItem(at: gossipPath)
        print("[App] Restored network_graph (\(data.count) bytes) to DB")
    }

    // MARK: - Push Token Re-registration

    /// Re-register the push token with the LSP now that we have a node_id.
    /// The initial registration may have happened before the node started.
    private func reregisterPushTokenIfNeeded() {
        guard let token = UserDefaults.standard.string(forKey: "apns_device_token"),
              !nodeService.nodeId.isEmpty else { return }

        let nodeId = nodeService.nodeId
        guard let url = URL(string: "https://\(Constants.defaultLSPAddress.replacingOccurrences(of: ":9735", with: ":8443"))/api/register-push") else { return }

        Task {
            var request = URLRequest(url: url)
            request.httpMethod = "POST"
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")

            #if DEBUG
            let apnsEnvironment = "sandbox"
            #else
            let apnsEnvironment = "production"
            #endif

            let body: [String: String] = [
                "device_token": token,
                "platform": "ios",
                "node_id": nodeId,
                "environment": apnsEnvironment,
            ]

            guard let httpBody = try? JSONSerialization.data(withJSONObject: body) else { return }
            request.httpBody = httpBody

            do {
                let (_, response) = try await URLSession.shared.data(for: request)
                if let http = response as? HTTPURLResponse {
                    print("[Push] Re-registered with node_id: \(http.statusCode)")
                }
            } catch {
                print("[Push] Re-registration failed: \(error.localizedDescription)")
            }
        }
    }

    // MARK: - Pending Push Payment (app was killed)

    /// Check if the NSE flagged a pending payment while the app was killed.
    /// Reconnect to LSP so the pending stability payment can land.
    private func processPendingPushPayment() async {
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        guard shared?.bool(forKey: "pending_push_payment") == true else { return }

        shared?.set(false, forKey: "pending_push_payment")
        print("[Push] Processing pending push payment from NSE flag")

        // Reconnect to LSP so pending payment can be received
        try? nodeService.node?.connect(
            nodeId: Constants.defaultLSPPubkey,
            address: Constants.defaultLSPAddress,
            persist: true
        )
        refreshBalances()
        updateStableBalances()
    }

    // MARK: - Push Notification Handling

    private var pushObserver: NSObjectProtocol?

    private func subscribeToPushNotifications() {
        if let observer = pushObserver {
            NotificationCenter.default.removeObserver(observer)
        }
        pushObserver = NotificationCenter.default.addObserver(
            forName: .pushPaymentNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            guard let self, self.nodeService.isRunning else { return }
            // Re-connect to LSP to ensure we can receive the pending payment
            try? self.nodeService.node?.connect(
                nodeId: Constants.defaultLSPPubkey,
                address: Constants.defaultLSPAddress,
                persist: true
            )
            self.refreshBalances()
            self.updateStableBalances()

            AuditService.log("PUSH_WAKE", data: [
                "node_running": "\(self.nodeService.isRunning)",
            ])
        }
    }

    // MARK: - Event Handling

    private func subscribeToEvents() {
        if let observer = eventObserver {
            NotificationCenter.default.removeObserver(observer)
        }
        eventObserver = NotificationCenter.default.addObserver(
            forName: .ldkEventReceived,
            object: nil,
            queue: .main
        ) { [weak self] notification in
            guard let self, let event = notification.object as? Event else { return }
            self.handleEvent(event)
        }
    }

    private func handleEvent(_ event: Event) {
        switch event {
        case .channelPending(let channelId, let userChannelId, _, let counterpartyNodeId, let fundingTxo):
            if stableChannel.userChannelId.isEmpty {
                stableChannel.userChannelId = userChannelId
            }
            fundingTxid = "\(fundingTxo.txid)"
            refreshBalances()
            updateStableBalances()

            AuditService.log("CHANNEL_PENDING", data: [
                "channel_id": channelId,
                "user_channel_id": userChannelId,
                "counterparty": counterpartyNodeId,
                "funding_txo": "\(fundingTxo)",
            ])

        case .channelReady(let channelId, let userChannelId, _, _):
            let isSplice = stableChannel.userChannelId == userChannelId
                && stableChannel.channelId != channelId

            stableChannel.channelId = channelId
            refreshBalances()
            updateStableBalances()

            // After splice confirms, reconcile outgoing
            if isSplice {
                let price = stableChannel.latestPrice
                if let usdDeducted = StabilityService.reconcileOutgoing(&stableChannel, price: price) {
                    AuditService.log("SPLICE_OUT_STABLE_DEDUCTED", data: [
                        "usd_deducted": "\(usdDeducted)",
                        "new_expected_usd": "\(stableChannel.expectedUSD.amount)",
                        "btc_price": "\(price)",
                    ])
                }
            }

            saveChannelToDB()

            AuditService.log("CHANNEL_READY", data: [
                "channel_id": channelId,
                "user_channel_id": userChannelId,
            ])
            statusMessage = "Channel is ready"

        case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
            handlePaymentReceived(
                paymentId: paymentId,
                amountMsat: amountMsat,
                paymentHash: paymentHash,
                customRecords: customRecords
            )

        case .paymentSuccessful(let paymentId, let paymentHash, _, let feePaidMsat):
            handlePaymentSuccessful(
                paymentId: paymentId,
                paymentHash: paymentHash,
                feePaidMsat: feePaidMsat
            )

        case .paymentFailed(let paymentId, let paymentHash, let reason):
            // Check if this is a pending trade payment
            if let pid = paymentId, let trade = pendingTradePayments.removeValue(forKey: "\(pid)") {
                try? databaseService?.updateTradeStatus(trade.tradeDbId, status: "failed")
                let verb = trade.action == "buy" ? "Buy" : "Sell"
                statusMessage = "\(verb) trade failed"

                AuditService.log("TRADE_FAILED", data: [
                    "payment_hash": paymentHash.map { "\($0)" } ?? "nil",
                    "action": trade.action,
                    "new_expected_usd": "\(trade.newExpectedUSD)",
                    "reason": reason.map { "\($0)" } ?? "unknown",
                ])
            } else {
                AuditService.log("PAYMENT_FAILED", data: [
                    "payment_id": paymentId.map { "\($0)" } ?? "nil",
                    "payment_hash": paymentHash.map { "\($0)" } ?? "nil",
                    "reason": reason.map { "\($0)" } ?? "unknown",
                ])
                statusMessage = "Payment failed"
            }

        case .splicePending(let channelId, let userChannelId, _, let newFundingTxo):
            handleSplicePending(
                channelId: channelId,
                userChannelId: userChannelId,
                newFundingTxo: newFundingTxo
            )

        case .spliceFailed(let channelId, let userChannelId, _, _):
            isSweeping = false
            sweepOnchainStart = 0
            pendingSplice = nil

            AuditService.log("SPLICE_FAILED", data: [
                "channel_id": "\(channelId)",
                "user_channel_id": "\(userChannelId)",
            ])
            statusMessage = "Splice failed"

        case .channelClosed(let channelId, let userChannelId, _, let reason):
            handleChannelClosed(
                channelId: channelId,
                userChannelId: userChannelId,
                reason: reason
            )

        default:
            break
        }
    }

    // MARK: - Payment Received

    private func handlePaymentReceived(
        paymentId: PaymentId?,
        amountMsat: UInt64,
        paymentHash: PaymentHash,
        customRecords: [CustomTlvRecord]
    ) {
        let paymentHashStr = "\(paymentHash)"
        let paymentIdStr = paymentId.map { "\($0)" } ?? paymentHashStr

        // Check for SYNC_V1 message from LSP
        if handleSyncMessage(customRecords: customRecords, paymentHash: paymentHashStr) {
            refreshBalances()
            updateStableBalances()
            return
        }

        // Normal payment received
        AuditService.log("PAYMENT_RECEIVED", data: [
            "amount_msat": "\(amountMsat)",
            "payment_id": paymentIdStr,
            "payment_hash": paymentHashStr,
        ])

        // Record in DB (dedup by paymentIdStr)
        let price = stableChannel.latestPrice
        let amountUSD: Double? = price > 0 ? (Double(amountMsat) / 1000.0 / 100_000_000.0) * price : nil
        let isStabilityPayment = customRecords.contains { $0.typeNum == Constants.stableChannelTLVType }
        let paymentType = isStabilityPayment ? "stability" : "lightning"

        _ = try? databaseService?.recordPayment(
            paymentId: paymentIdStr,
            paymentType: paymentType,
            direction: "received",
            amountMsat: amountMsat,
            amountUSD: amountUSD,
            btcPrice: price > 0 ? price : nil,
            counterparty: nil,
            status: "completed"
        )

        // Update balances and reconcile incoming
        refreshBalances()
        updateStableBalances()
        StabilityService.reconcileIncoming(&stableChannel)
        saveChannelToDB()

        if let usd = amountUSD {
            statusMessage = "Received \(usd.usdFormatted)"
        } else {
            let sats = amountMsat / 1000
            statusMessage = "Received \(sats.btcSpacedFormatted) BTC"
        }

        // Trigger payment received animation
        paymentFlash = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
            self?.paymentFlash = false
        }
    }

    /// Parse and handle a SYNC_V1 TLV message. Returns true if handled.
    private func handleSyncMessage(customRecords: [CustomTlvRecord], paymentHash: String) -> Bool {
        for tlv in customRecords {
            guard tlv.typeNum == Constants.stableChannelTLVType else { continue }

            guard let parsed = TradeService.parseIncomingTLV(
                data: tlv.value,
                expectedCounterparty: stableChannel.counterparty,
                verifySignature: { [weak self] msg, sig, pubkey in
                    self?.nodeService.verifySignature(message: msg, signature: sig, pubkey: pubkey) ?? false
                }
            ) else { continue }

            guard parsed.type == Constants.syncMessageType else { continue }

            let oldExpected = stableChannel.expectedUSD.amount
            let price = stableChannel.latestPrice
            StabilityService.applyTrade(&stableChannel, newExpectedUSD: parsed.expectedUSD, price: price)
            saveChannelToDB()

            AuditService.log("SYNC_V1_APPLIED", data: [
                "old_expected_usd": "\(oldExpected)",
                "new_expected_usd": "\(parsed.expectedUSD)",
                "btc_price": "\(price)",
                "payment_hash": paymentHash,
            ])
            return true
        }
        return false
    }

    // MARK: - Payment Successful

    private func handlePaymentSuccessful(
        paymentId: PaymentId?,
        paymentHash: PaymentHash,
        feePaidMsat: UInt64?
    ) {
        let paymentHashStr = "\(paymentHash)"

        // Check if this is a pending trade payment — apply trade now that payment confirmed
        if let pid = paymentId, let trade = pendingTradePayments.removeValue(forKey: "\(pid)") {
            // Apply the trade (deferred until confirmation — matches desktop)
            StabilityService.applyTrade(
                &stableChannel,
                newExpectedUSD: trade.newExpectedUSD,
                price: trade.price
            )
            saveChannelToDB()

            try? databaseService?.updateTradeStatus(trade.tradeDbId, status: "completed")

            AuditService.log("TRADE_CONFIRMED", data: [
                "payment_hash": paymentHashStr,
                "action": trade.action,
                "new_expected_usd": "\(trade.newExpectedUSD)",
                "fee_paid_msat": feePaidMsat.map { "\($0)" } ?? "nil",
            ])

            refreshBalances()
            updateStableBalances()

            let verb = trade.action == "buy" ? "Buy" : "Sell"
            statusMessage = "\(verb) confirmed"

            // Flash so user notices the confirmation
            paymentFlash = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                self?.paymentFlash = false
            }
            return
        }

        // Normal (non-trade) outgoing payment
        refreshBalances()
        updateStableBalances()

        let price = stableChannel.latestPrice

        // Reconcile: if outgoing payment exceeded native BTC, deduct from stable
        let oldExpected = stableChannel.expectedUSD.amount
        if let usdDeducted = StabilityService.reconcileOutgoing(&stableChannel, price: price) {
            AuditService.log("OUTGOING_STABLE_DEDUCTED", data: [
                "payment_hash": paymentHashStr,
                "usd_deducted": "\(usdDeducted)",
                "old_expected_usd": "\(oldExpected)",
                "new_expected_usd": "\(stableChannel.expectedUSD.amount)",
                "btc_price": "\(price)",
            ])
        }

        // Update payment status in DB
        if let pidStr = paymentId.map({ "\($0)" }) {
            try? databaseService?.updatePaymentStatus(
                paymentId: pidStr,
                status: "completed",
                feeMsat: feePaidMsat
            )
        }

        saveChannelToDB()

        AuditService.log("PAYMENT_SUCCESSFUL", data: [
            "payment_hash": paymentHashStr,
            "fee_paid_msat": feePaidMsat.map { "\($0)" } ?? "nil",
        ])
        statusMessage = "Payment confirmed"
    }

    // MARK: - Channel Closed

    private func handleChannelClosed(
        channelId: ChannelId,
        userChannelId: UserChannelId,
        reason: ClosureReason?
    ) {
        let reasonStr = reason.map { "\($0)" } ?? "unknown"
        let balanceSats = stableChannel.stableReceiverBTC.sats
        let price = btcPrice > 0 ? btcPrice : stableChannel.latestPrice
        let balanceUSD: Double? = price > 0
            ? Double(balanceSats) / Double(Constants.satsInBTC) * price
            : nil

        AuditService.log("CHANNEL_CLOSED", data: [
            "channel_id": "\(channelId)",
            "user_channel_id": "\(userChannelId)",
            "reason": reasonStr,
            "balance_sats": "\(balanceSats)",
        ])

        // Record in payment history
        _ = try? databaseService?.recordPayment(
            paymentId: "close_\(channelId)",
            paymentType: "channel_close",
            direction: "received",
            amountMsat: balanceSats * 1000,
            amountUSD: balanceUSD,
            btcPrice: price > 0 ? price : nil,
            counterparty: stableChannel.counterparty.isEmpty ? nil : stableChannel.counterparty,
            status: "completed"
        )

        // Clear stable state if this is our channel or no channels remain
        if stableChannel.userChannelId == userChannelId || nodeService.channels.isEmpty {
            try? databaseService?.deleteChannel(userChannelId: stableChannel.userChannelId)
            stableChannel.expectedUSD = .zero
            stableChannel.backingSats = 0
            stableChannel.nativeSats = 0
            stableChannel.nativeChannelBTC = .zero
            stableChannel.stableReceiverBTC = .zero
            stableChannel.stableReceiverUSD = .zero
            stableChannel.channelId = ""
            stableChannel.userChannelId = ""
        }

        // Refresh balances so lightning drops to 0 immediately
        refreshBalances()
        isChannelClosing = false
        statusMessage = "Channel closed"
    }

    // MARK: - Splice Pending

    private func handleSplicePending(
        channelId: ChannelId,
        userChannelId: UserChannelId,
        newFundingTxo: OutPoint
    ) {
        fundingTxid = "\(newFundingTxo.txid)"

        AuditService.log("SPLICE_PENDING", data: [
            "channel_id": "\(channelId)",
            "user_channel_id": "\(userChannelId)",
            "funding_txo": "\(newFundingTxo)",
        ])

        // Record/update splice payment
        if let splice = pendingSplice {
            pendingSplice = nil
            let txidStr = "\(newFundingTxo.txid)"
            if splice.direction == "in" {
                // Auto-sweep splice_in was already recorded — update with txid
                try? databaseService?.setPendingSpliceTxid(txidStr)
            } else {
                let price = stableChannel.latestPrice
                let amountMsat = splice.amountSats * 1000
                let amountUSD: Double? = price > 0 ? Double(splice.amountSats) / 100_000_000.0 * price : nil
                _ = try? databaseService?.recordPayment(
                    paymentId: txidStr,
                    paymentType: "splice_out",
                    direction: "sent",
                    amountMsat: amountMsat,
                    amountUSD: amountUSD,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending",
                    txid: txidStr,
                    address: splice.address
                )
            }
        }

        refreshBalances()
        updateStableBalances()
        statusMessage = "Splice pending"
    }

    // MARK: - Stability Timer

    private func startStabilityTimer() {
        stabilityTimer?.cancel()
        stabilityTimer = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Constants.stabilityCheckIntervalSecs * 1_000_000_000)
                guard !Task.isCancelled else { break }

                await MainActor.run { [weak self] in
                    // Heartbeat so NSE knows main app is active
                    UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .set(Date().timeIntervalSince1970, forKey: "main_app_last_active")

                    // Reconnect to LSP if peer dropped — keeps channel usable
                    self?.ensureLSPConnected()

                    self?.recordCurrentPrice()
                    self?.runStabilityCheck()
                    self?.detectOnchainDeposit()
                }
            }
        }
    }

    func ensureLSPConnected() {
        guard let node = nodeService.node else { return }
        nodeService.refreshChannels()
        let allUsable = !nodeService.channels.isEmpty && nodeService.channels.allSatisfy { $0.isUsable }
        guard !allUsable else { return }
        try? node.connect(
            nodeId: Constants.defaultLSPPubkey,
            address: Constants.defaultLSPAddress,
            persist: true
        )
    }

    private func runStabilityCheck() {
        let price = btcPrice
        guard price > 0 else { return }

        refreshBalances()
        updateStableBalances()

        guard stableChannel.expectedUSD.amount > 0,
              !nodeService.channels.isEmpty else { return }

        // Derive backing_sats from expectedUSD and price — this is what backing represents.
        if price > 0 {
            stableChannel.backingSats = UInt64((stableChannel.expectedUSD.amount / price) * Double(Constants.satsInBTC))
        }

        let result = StabilityService.checkStabilityAction(stableChannel, price: price)

        guard result.action == .pay else { return }

        // Cooldown check
        let now = Int64(Date().timeIntervalSince1970)
        guard now - stableChannel.lastStabilityPayment >= Int64(Constants.stabilityPaymentCooldownSecs) else { return }

        let amountMsat = USD(amount: abs(result.dollarsFromPar)).toMsats(price: price)
        guard amountMsat > 0 else { return }

        // Send stability payment
        do {
            let paymentId = try nodeService.sendKeysend(
                amountMsat: amountMsat,
                to: stableChannel.counterparty
            )
            stableChannel.lastStabilityPayment = now
            stableChannel.paymentMade = true

            // No manual backing_sats reset needed — it's derived from
            // receiver_sats - native_sats at the start of each check.
            saveChannelToDB()

            // Record as pending payment
            _ = try? databaseService?.recordPayment(
                paymentId: "\(paymentId)",
                paymentType: "stability",
                direction: "sent",
                amountMsat: amountMsat,
                amountUSD: abs(result.dollarsFromPar),
                btcPrice: price,
                counterparty: stableChannel.counterparty,
                status: "pending"
            )

            AuditService.log("STABILITY_PAYMENT_SENT", data: [
                "amount_msat": "\(amountMsat)",
                "dollars_from_par": "\(result.dollarsFromPar)",
                "percent_from_par": "\(result.percentFromPar)",
                "btc_price": "\(price)",
            ])
        } catch {
            AuditService.log("STABILITY_PAYMENT_FAILED", data: [
                "error": error.localizedDescription,
            ])
        }
    }

    // MARK: - On-Chain Deposit Detection

    private func detectOnchainDeposit() {
        // Use totalOnchainBalanceSats consistently (not spendable, which excludes unconfirmed)
        guard let balances = nodeService.balances() else { return }
        let currentOnchain = balances.totalOnchainBalanceSats

        if currentOnchain > prevOnchainSats && !isSweeping && pendingSplice == nil {
            let depositSats = currentOnchain - prevOnchainSats
            // Ignore tiny fluctuations from fee estimation changes
            guard depositSats >= 1000 else {
                prevOnchainSats = currentOnchain
                return
            }

            let price = stableChannel.latestPrice > 0 ? stableChannel.latestPrice : btcPrice
            let amountUSD: Double? = price > 0 ? Double(depositSats) / 100_000_000.0 * price : nil

            // Use timestamp-based ID so dedup works
            let depositId = "onchain_\(Int64(Date().timeIntervalSince1970))_\(depositSats)"
            _ = try? databaseService?.recordPayment(
                paymentId: depositId,
                paymentType: "onchain",
                direction: "received",
                amountMsat: depositSats * 1000,
                amountUSD: amountUSD,
                btcPrice: price > 0 ? price : nil,
                counterparty: nil,
                status: "completed"
            )

            AuditService.log("ONCHAIN_DEPOSIT_DETECTED", data: [
                "amount_sats": "\(depositSats)",
                "prev_onchain": "\(prevOnchainSats)",
                "new_onchain": "\(currentOnchain)",
            ])
        }
        prevOnchainSats = currentOnchain
    }

    // MARK: - Sweep to Channel

    /// Sweep all on-chain funds into the Lightning channel (user-initiated splice-in).
    func openChannelWithOnchainFunds() {
        guard !isOpeningChannel else { return }
        let spendable = nodeService.spendableOnchainSats()
        guard spendable > 10_000 else {
            statusMessage = "Not enough on-chain funds"
            return
        }

        isOpeningChannel = true
        statusMessage = "Opening channel..."

        Task {
            do {
                ensureLSPConnected()
                // Reserve some sats for fees
                let channelSats = spendable - 5_000
                try await nodeService.connectAndOpenChannel(
                    pubkey: Constants.defaultLSPPubkey,
                    address: Constants.defaultLSPAddress,
                    amountSats: channelSats
                )
                await MainActor.run {
                    refreshBalances()
                    statusMessage = "Channel opening..."
                }
            } catch {
                await MainActor.run {
                    statusMessage = "Open channel failed: \(error.localizedDescription)"
                }
            }
            await MainActor.run {
                isOpeningChannel = false
            }
        }
    }

    func sweepToChannel() {
        guard !isSweeping else {
            statusMessage = "Sweep already in progress"
            return
        }

        guard let channel = nodeService.channels.first(where: { $0.isChannelReady }) else {
            statusMessage = "No ready channel"
            return
        }

        guard let balances = nodeService.balances() else {
            statusMessage = "Could not read balances"
            return
        }

        let feeRateSatVb = fetchFeeRate() ?? 2
        let feeReserve = feeRateSatVb * 170
        let spendable = balances.spendableOnchainBalanceSats
        guard spendable > feeReserve else {
            statusMessage = "Insufficient on-chain balance"
            return
        }
        let sweepAmount = spendable - feeReserve

        do {
            try nodeService.spliceIn(
                userChannelId: channel.userChannelId,
                counterpartyNodeId: channel.counterpartyNodeId,
                amountSats: sweepAmount
            )
            isSweeping = true
            sweepOnchainStart = balances.totalOnchainBalanceSats
            pendingSplice = PendingSplice(direction: "in", amountSats: sweepAmount, address: nil)
            statusMessage = "Moving \(sweepAmount) sats to channel..."

            _ = try? databaseService?.recordPayment(
                paymentId: nil,
                paymentType: "splice_in",
                direction: "received",
                amountMsat: sweepAmount * 1000,
                amountUSD: nil,
                btcPrice: nil,
                counterparty: nil,
                status: "pending"
            )

            AuditService.log("SWEEP_TO_CHANNEL", data: [
                "amount_sats": "\(sweepAmount)",
                "fee_rate_sat_vb": "\(feeRateSatVb)",
            ])
        } catch {
            statusMessage = "Sweep failed: \(error.localizedDescription)"
            AuditService.log("SWEEP_FAILED", data: [
                "error": error.localizedDescription,
            ])
        }
    }

    /// Fetch fee rate (sat/vB) for 6-block target from esplora (with fallback).
    private func fetchFeeRate() -> UInt64? {
        for baseURL in [Constants.primaryChainURL, Constants.fallbackChainURL] {
            guard let url = URL(string: "\(baseURL)/fee-estimates") else { continue }
            guard let data = try? Data(contentsOf: url),
                  let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                  let rate = json["6"] as? Double else { continue }
            return UInt64(rate.rounded(.up))
        }
        return nil
    }

    /// Test Blockstream connectivity; fall back to mempool.space if unreachable.
    private func resolveChainURL() async -> String {
        guard let url = URL(string: "\(Constants.primaryChainURL)/blocks/tip/height") else {
            return Constants.fallbackChainURL
        }
        do {
            let (_, response) = try await URLSession.shared.data(from: url)
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                return Constants.primaryChainURL
            }
        } catch {}
        AuditService.log("CHAIN_SOURCE_FALLBACK", data: [
            "primary": Constants.primaryChainURL,
            "using": Constants.fallbackChainURL,
        ])
        return Constants.fallbackChainURL
    }

    // MARK: - Balance Refresh

    func refreshBalances() {
        nodeService.refreshChannels()
        guard let balances = nodeService.balances() else { return }
        let onchain = balances.totalOnchainBalanceSats
        let lightning = balances.totalLightningBalanceSats

        lightningBalanceSats = lightning
        onchainBalanceSats = onchain

        // Cache for instant display on next launch
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        ud?.set(Int64(bitPattern: lightning), forKey: "cached_lightning_sats")
        ud?.set(Int64(bitPattern: onchain), forKey: "cached_onchain_sats")
    }

    /// Update the StableChannel struct from current LDK channel data + price.
    private func updateStableBalances() {
        let hadChannelId = !stableChannel.userChannelId.isEmpty
        let price = btcPrice > 0 ? btcPrice : stableChannel.latestPrice
        StabilityService.updateBalances(
            &stableChannel,
            channels: nodeService.channels,
            onchainBalanceSats: onchainBalanceSats,
            price: price
        )
        // If userChannelId was just discovered, reload saved state (expectedUSD etc.) from DB
        if !hadChannelId && !stableChannel.userChannelId.isEmpty {
            loadChannelFromDB()
        }
    }

    // MARK: - Persistence

    func saveChannelToDB() {
        guard !stableChannel.userChannelId.isEmpty else { return }
        do {
            try databaseService?.saveChannel(
                channelId: stableChannel.channelId,
                userChannelId: stableChannel.userChannelId,
                expectedUSD: stableChannel.expectedUSD.amount,
                backingSats: stableChannel.backingSats,
                nativeSats: stableChannel.nativeSats,
                note: stableChannel.note,
                receiverSats: stableChannel.stableReceiverBTC.sats,
                latestPrice: stableChannel.latestPrice
            )
        } catch {
            AuditService.log("DB_SAVE_CHANNEL_FAILED", data: ["error": error.localizedDescription])
        }
    }

    private func loadChannelFromDB() {
        guard let db = databaseService else { return }
        do {
            if let record = try db.loadChannel(userChannelId: stableChannel.userChannelId),
               !record.userChannelId.isEmpty {
                stableChannel.channelId = record.channelId
                stableChannel.userChannelId = record.userChannelId
                stableChannel.expectedUSD = USD(amount: record.expectedUSD)
                stableChannel.backingSats = record.backingSats
                stableChannel.nativeSats = record.nativeSats
                stableChannel.note = record.note

                // Restore cached balances so UI shows immediately
                if record.receiverSats > 0 {
                    stableChannel.stableReceiverBTC = Bitcoin(sats: record.receiverSats)
                    stableChannel.stableReceiverUSD = USD.fromBitcoin(stableChannel.stableReceiverBTC, price: record.latestPrice)
                    StabilityService.recomputeNative(&stableChannel)
                }
                if record.latestPrice > 0 {
                    stableChannel.latestPrice = record.latestPrice
                }
            }
        } catch {
            AuditService.log("DB_LOAD_CHANNEL_FAILED", data: ["error": error.localizedDescription])
        }
    }

    // MARK: - Record Price

    func recordCurrentPrice() {
        let price = btcPrice
        guard price > 0 else { return }
        do {
            try databaseService?.recordPrice(price, source: "median")
        } catch {
            // Price recording is best-effort, don't log every failure
        }
    }

    // MARK: - Hourly Price Backfill

    /// Fetch hourly candles from Kraken and backfill price_history for smooth 1D/1W/1M charts.
    private func backfillHourlyPrices() async {
        guard let db = databaseService else { return }

        // Determine how far back we need data — up to 30 days
        let thirtyDaysAgo = Int64(Date().timeIntervalSince1970) - 30 * 24 * 3600
        let since: Int64
        if let oldest = try? db.getOldestPriceHistoryTimestamp(), oldest < thirtyDaysAgo {
            // Already have old enough data, just fill gaps from the newest record
            since = (try? db.getPriceHistory(hours: 1).last?.timestamp) ?? thirtyDaysAgo
        } else {
            since = thirtyDaysAgo
        }

        let candles = await priceService.fetchKrakenOHLC(since: since)
        guard !candles.isEmpty else { return }

        do {
            let count = try db.backfillHourlyPrices(candles)
            if count > 0 {
                print("[Chart] Backfilled \(count) hourly price points from Kraken")
            }
        } catch {
            print("[Chart] Hourly backfill failed: \(error)")
        }
    }

    // MARK: - Historical Price Seeding

    private func seedHistoricalPrices() {
        guard let db = databaseService else { return }

        let needsSeed: Bool
        do {
            if let oldest = try db.getOldestDailyPriceDate() {
                needsSeed = !oldest.hasPrefix("2013")
            } else {
                needsSeed = true
            }
        } catch {
            needsSeed = true
        }

        guard needsSeed else {
            print("[Chart] Historical prices already seeded")
            return
        }

        print("[Chart] Seeding historical price data (2013-present)...")
        do {
            let count = try db.bulkInsertDailyPrices(HistoricalPrices.seedPrices)
            print("[Chart] Seeded \(count) historical price records")
        } catch {
            print("[Chart] Failed to seed historical prices: \(error)")
        }
    }
}
