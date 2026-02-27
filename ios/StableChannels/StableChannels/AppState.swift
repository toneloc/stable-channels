import Foundation
import SwiftUI
import LDKNode

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

    // Balance (derived)
    var totalBalanceSats: UInt64 = 0
    var lightningBalanceSats: UInt64 = 0
    var onchainBalanceSats: UInt64 = 0

    var totalBalanceUSD: Double {
        guard btcPrice > 0 else { return 0 }
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

        // Start price fetching
        priceService.startAutoRefresh()

        // Subscribe to LDK events
        subscribeToEvents()

        // Check for existing seed / mnemonic
        let seedPath = Constants.userDataDir.appendingPathComponent("keys_seed")
        if FileManager.default.fileExists(atPath: seedPath.path) {
            await MainActor.run { phase = .syncing }

            do {
                try await nodeService.start(
                    network: .bitcoin,
                    esploraURL: Constants.defaultChainURL,
                    mnemonic: ""  // Uses existing seed from data dir
                )
                await MainActor.run {
                    phase = .wallet
                    refreshBalances()
                    prevOnchainSats = onchainBalanceSats
                }
                startStabilityTimer()
            } catch {
                await MainActor.run { phase = .error("Node start failed: \(error.localizedDescription)") }
            }
        } else {
            await MainActor.run { phase = .onboarding }
        }
    }

    func stop() {
        stabilityTimer?.cancel()
        stabilityTimer = nil
        if let observer = eventObserver {
            NotificationCenter.default.removeObserver(observer)
            eventObserver = nil
        }
        priceService.stopAutoRefresh()
        nodeService.stop()
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

        // Check for SYNC_V1 message from LSP
        if handleSyncMessage(customRecords: customRecords, paymentHash: paymentHashStr) {
            refreshBalances()
            updateStableBalances()
            return
        }

        // Normal payment received
        AuditService.log("PAYMENT_RECEIVED", data: [
            "amount_msat": "\(amountMsat)",
            "payment_hash": paymentHashStr,
        ])

        // Record in DB
        let price = stableChannel.latestPrice
        let amountUSD: Double? = price > 0 ? (Double(amountMsat) / 1000.0 / 100_000_000.0) * price : nil
        let paymentType = stableChannel.expectedUSD.amount > 0 ? "stability" : "lightning"

        _ = try? databaseService?.recordPayment(
            paymentId: paymentHashStr,
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

        // Check if this is a pending trade payment
        if let pid = paymentId, let trade = pendingTradePayments.removeValue(forKey: "\(pid)") {
            // Trade payment confirmed — now apply the trade
            StabilityService.applyTrade(&stableChannel, newExpectedUSD: trade.newExpectedUSD, price: trade.price)
            try? databaseService?.updateTradeStatus(trade.tradeDbId, status: "completed")

            AuditService.log("TRADE_CONFIRMED", data: [
                "payment_hash": paymentHashStr,
                "action": trade.action,
                "new_expected_usd": "\(trade.newExpectedUSD)",
                "fee_paid_msat": feePaidMsat.map { "\($0)" } ?? "nil",
            ])

            refreshBalances()
            updateStableBalances()
            saveChannelToDB()

            let verb = trade.action == "buy" ? "Buy" : "Sell"
            statusMessage = "\(verb) confirmed"
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
        let currentOnchain = nodeService.spendableOnchainSats()
        if currentOnchain > prevOnchainSats && prevOnchainSats > 0 && !isSweeping {
            let depositSats = currentOnchain - prevOnchainSats
            let price = stableChannel.latestPrice
            let amountUSD: Double? = price > 0 ? Double(depositSats) / 100_000_000.0 * price : nil

            _ = try? databaseService?.recordPayment(
                paymentId: nil,
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
        let price = btcPrice > 0 ? btcPrice : stableChannel.latestPrice
        StabilityService.updateBalances(
            &stableChannel,
            channels: nodeService.channels,
            onchainBalanceSats: onchainBalanceSats,
            price: price
        )
    }

    // MARK: - Persistence

    private func saveChannelToDB() {
        guard !stableChannel.userChannelId.isEmpty else { return }
        do {
            try databaseService?.saveChannel(
                channelId: stableChannel.channelId,
                userChannelId: stableChannel.userChannelId,
                expectedUSD: stableChannel.expectedUSD.amount,
                backingSats: stableChannel.backingSats,
                note: stableChannel.note
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
