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
    var isSyncing: Bool = false

    // Balance (derived)
    var totalBalanceSats: UInt64 = 0
    var lightningBalanceSats: UInt64 = 0
    var onchainBalanceSats: UInt64 = 0

    var totalBalanceUSD: Double {
        guard btcPrice > 0 else { return 0 }
        // Before node syncs, use cached channel balance if available
        if totalBalanceSats == 0 && stableChannel.stableReceiverBTC.sats > 0 {
            return Double(stableChannel.stableReceiverBTC.sats + stableChannel.onchainBTC.sats)
                / Double(Constants.satsInBTC) * btcPrice
        }
        return Double(totalBalanceSats) / Double(Constants.satsInBTC) * btcPrice
    }

    var stableUSD: Double { stableChannel.expectedUSD.amount }
    var nativeBTC: Bitcoin { stableChannel.nativeChannelBTC }

    // MARK: - Internal

    private var eventObserver: NSObjectProtocol?
    private var stabilityTimer: Task<Void, Never>?

    // Auto-sweep state
    private var isSweeping = false
    private var sweepOnchainStart: UInt64 = 0
    private var prevOnchainSats: UInt64 = 0

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

        // Check for existing seed / mnemonic
        let seedPath = Constants.userDataDir.appendingPathComponent("keys_seed")
        if FileManager.default.fileExists(atPath: seedPath.path) {
            // Show wallet immediately with cached data from DB
            let hasCachedData = !stableChannel.userChannelId.isEmpty
            await MainActor.run {
                phase = hasCachedData ? .wallet : .syncing
                if hasCachedData { isSyncing = true }
            }

            do {
                try await nodeService.start(
                    network: .bitcoin,
                    esploraURL: Constants.defaultChainURL,
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
                    // Use totalOnchainBalanceSats to match detectOnchainDeposit
                    prevOnchainSats = nodeService.balances()?.totalOnchainBalanceSats ?? 0
                }
                startStabilityTimer()
                // Re-register push token with node_id now that node is running
                reregisterPushTokenIfNeeded()
                // Check if NSE flagged a pending payment while app was killed
                await processPendingPushPayment()
            } catch {
                await MainActor.run { phase = .error("Node start failed: \(error.localizedDescription)") }
            }
        } else {
            await MainActor.run { phase = .onboarding }
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

    /// Stop the node and extract gossip data so the NSE can open the lightweight DB.
    func stopNodeForBackground() {
        print("[App] Stopping node for background")
        stabilityTimer?.cancel()
        stabilityTimer = nil
        nodeService.stop()
        extractGossipFromDB()
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.set(Date().timeIntervalSince1970, forKey: "main_app_last_active")
    }

    /// Restore gossip data and restart the node when returning to foreground.
    func restartNodeFromForeground() async {
        guard case .wallet = phase else { return }
        print("[App] Restarting node from foreground")
        await waitForNSE()
        restoreGossipToDB()
        do {
            try await nodeService.start(
                network: .bitcoin,
                esploraURL: Constants.defaultChainURL,
                mnemonic: ""
            )
            refreshBalances()
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

    /// Restore network_graph blob from file back into SQLite.
    private func restoreGossipToDB() {
        let dbPath = Constants.userDataDir.appendingPathComponent("ldk_node_data.sqlite").path
        let gossipPath = Constants.userDataDir.appendingPathComponent("network_graph.bin")

        guard FileManager.default.fileExists(atPath: gossipPath.path) else { return }

        guard let data = try? Data(contentsOf: gossipPath) else { return }

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
        guard let url = URL(string: "http://\(Constants.defaultLSPAddress.replacingOccurrences(of: ":9737", with: ":8080"))/api/register-push") else { return }

        Task {
            var request = URLRequest(url: url)
            request.httpMethod = "POST"
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")

            let body: [String: String] = [
                "device_token": token,
                "platform": "ios",
                "node_id": nodeId,
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

        let sats = amountMsat / 1000
        if let usd = amountUSD {
            statusMessage = "Received \(sats) sats ($\(String(format: "%.2f", usd)))"
        } else {
            statusMessage = "Received \(sats) sats"
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
        AuditService.log("CHANNEL_CLOSED", data: [
            "channel_id": "\(channelId)",
            "user_channel_id": "\(userChannelId)",
            "reason": reason.map { "\($0)" } ?? "unknown",
        ])

        // Clear stable state if this is our channel or no channels remain
        if stableChannel.userChannelId == userChannelId || nodeService.channels.isEmpty {
            try? databaseService?.deleteChannel(userChannelId: stableChannel.userChannelId)
            stableChannel.expectedUSD = .zero
            stableChannel.backingSats = 0
            stableChannel.nativeChannelBTC = .zero
            stableChannel.stableReceiverBTC = .zero
            stableChannel.stableReceiverUSD = .zero
            stableChannel.channelId = ""
            stableChannel.userChannelId = ""
        }

        statusMessage = "Channel closed"
    }

    // MARK: - Splice Pending

    private func handleSplicePending(
        channelId: ChannelId,
        userChannelId: UserChannelId,
        newFundingTxo: OutPoint
    ) {
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

                    self?.recordCurrentPrice()
                    self?.runStabilityCheck()
                    self?.detectOnchainDeposit()
                    self?.runAutoSweep()
                }
            }
        }
    }

    private func runStabilityCheck() {
        let price = btcPrice
        guard price > 0 else { return }

        refreshBalances()
        updateStableBalances()

        guard stableChannel.expectedUSD.amount > 0,
              !nodeService.channels.isEmpty else { return }

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

        if currentOnchain > prevOnchainSats && !isSweeping {
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

    // MARK: - Auto-Sweep

    private func runAutoSweep() {
        // If a sweep is in progress, check if on-chain balance dropped (confirming splice tx)
        if isSweeping {
            let current = nodeService.balances()?.totalOnchainBalanceSats ?? 0
            if sweepOnchainStart > 0 && current < sweepOnchainStart {
                isSweeping = false
                sweepOnchainStart = 0
                AuditService.log("AUTO_SWEEP_CONFIRMED", data: [
                    "prev_onchain": "\(sweepOnchainStart)",
                    "new_onchain": "\(current)",
                ])
            }
            return
        }

        // Find a ready channel to sweep into
        guard let channel = nodeService.channels.first(where: { $0.isChannelReady }) else { return }
        guard let balances = nodeService.balances() else { return }

        guard balances.totalOnchainBalanceSats > Constants.autoSweepMinSats else { return }

        // Fetch fee rate from esplora (6-block target)
        let feeRateSatVb = fetchFeeRate() ?? 2  // conservative fallback

        // Splice tx ~170 vbytes; reserve exactly enough for fees
        let feeReserve = feeRateSatVb * 170
        let spendable = balances.spendableOnchainBalanceSats
        guard spendable > feeReserve else { return }
        let sweepAmount = spendable - feeReserve

        guard sweepAmount > 0 else { return }

        AuditService.log("AUTO_SWEEP_ATTEMPT", data: [
            "onchain_sats": "\(balances.totalOnchainBalanceSats)",
            "spendable_sats": "\(spendable)",
            "fee_rate_sat_vb": "\(feeRateSatVb)",
            "fee_reserve": "\(feeReserve)",
            "sweep_amount": "\(sweepAmount)",
        ])

        do {
            try nodeService.spliceIn(
                userChannelId: channel.userChannelId,
                counterpartyNodeId: channel.counterpartyNodeId,
                amountSats: sweepAmount
            )
            isSweeping = true
            sweepOnchainStart = balances.totalOnchainBalanceSats
            pendingSplice = PendingSplice(direction: "in", amountSats: sweepAmount, address: nil)

            // Record pending splice-in in payment history
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

            AuditService.log("AUTO_SWEEP_INITIATED", data: [
                "amount_sats": "\(sweepAmount)",
            ])
        } catch {
            AuditService.log("AUTO_SWEEP_FAILED", data: [
                "error": error.localizedDescription,
            ])
        }
    }

    /// Fetch fee rate (sat/vB) for 6-block target from esplora.
    private func fetchFeeRate() -> UInt64? {
        guard let url = URL(string: "\(Constants.defaultChainURL)/fee-estimates") else { return nil }
        guard let data = try? Data(contentsOf: url),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let rate = json["6"] as? Double else { return nil }
        return UInt64(rate.rounded(.up))
    }

    // MARK: - Balance Refresh

    func refreshBalances() {
        guard let balances = nodeService.balances() else { return }
        let onchain = balances.totalOnchainBalanceSats

        nodeService.refreshChannels()
        let lightning = nodeService.channels.reduce(UInt64(0)) { sum, ch in
            let reserve = ch.unspendablePunishmentReserve ?? 0
            return sum + (ch.outboundCapacityMsat / 1000) + reserve
        }

        totalBalanceSats = onchain + lightning
        lightningBalanceSats = lightning
        onchainBalanceSats = onchain
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
}
