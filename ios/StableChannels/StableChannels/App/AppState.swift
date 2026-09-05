import Foundation
import SwiftUI
import LDKNode
import SQLite3

private enum SyncMessageHandlingResult {
    case notSync
    case applied
    case retry
}

@MainActor
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

    // MARK: - Authentication

    /// Injected biometric authenticator — defaults to `BiometricService.shared`.
    /// Depend on the `BiometricAuthenticating` protocol, not the concrete type (DI).
    let biometricAuth: BiometricAuthenticating = BiometricService.shared

    /// Whether user has passed biometric/passcode auth this session.
    /// Reset to false on app termination (no persistence = no bypass on restart).
    private(set) var isUnlocked: Bool = false

    /// Prevents double-trigger of auth (onAppear + onChange both firing).
    var isAuthenticating: Bool = false

    /// Last auth error for UI display.
    var authError: String?

    func lock() {
        isUnlocked = false
        authError = nil
    }

    func authenticate(reason: String = "Authenticate with Stable Channels") async -> Bool {
        guard !isAuthenticating else { return false }
        isAuthenticating = true
        defer { isAuthenticating = false }
        authError = nil

        let success: Bool
        do {
            success = try await biometricAuth.authenticate(reason: reason, allowPasscodeFallback: true)
        } catch let error as BiometricError {
            if error != .cancelled {
                authError = error.errorDescription
            }
            success = false
        } catch {
            authError = "Authentication failed. Please try again."
            success = false
        }

        if success {
            isUnlocked = true
        }

        return success
    }

    // MARK: - Services

    let nodeService = NodeService.shared
    let priceService = PriceService()
    let feeRateService = FeeRateService()
    var databaseService: DatabaseService?
    var tradeService: TradeService?
    let blockHeightService = BlockHeightService(
        provider: BlockHeightResolver(chainURLs: Constants.esploraChainURLs)
    )
    let confirmationService = ConfirmationService(
        provider: TxConfirmationResolver(chainURLs: Constants.esploraChainURLs)
    )
    var confirmationPollingService: ConfirmationPollingService?
    var spvHeaderChainService: SPVHeaderChainService?
    /// Incremented after each confirmation poll cycle completes a DB write.
    /// Views observe this to reload payment data at the right time.
    var confirmationUpdateEpoch: Int = 0
    let mempoolWebSocketService: MempoolWebSocketProtocol = MempoolWebSocketService()
    let lspService = LSPService()
    private let lifecycleManager = WalletLifecycleManager(
        validator: { mnemonic in
            // Building an LDK node to derive an identity is heavy — never on the
            // main actor (restore UI would freeze for the whole build).
            await Task.detached(priority: .utility) {
                AppState.deriveNodeId(mnemonic: mnemonic) != nil
            }.value
        }
    )

    init() {
        // Set audit log path
        let auditPath = Constants.userDataDir.appendingPathComponent("audit_log.txt").path
        AuditService.setLogPath(auditPath)

        // Hook WalletKeychainService logging into AuditService
        WalletKeychainService.onLog = { event, data in
            AuditService.log(event, data: data)
        }
    }

    // MARK: - State

    var stableChannel: StableChannel = .default
    var btcPrice: Double { priceService.currentPrice }
    var accountingBTCPrice: Double { priceService.accountingPrice }
    var statusMessage: String = ""
    var paymentFlash: Bool = false
    var isChannelClosing: Bool = false
    var isOpeningChannel: Bool = false
    var isSyncing: Bool = false
    var spendableOnchainSats: UInt64 = 0

    // Balance (derived) — initialized from cache for instant display
    var lightningBalanceSats: UInt64 = {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        return UInt64(bitPattern: Int64(ud?.integer(forKey: "cached_lightning_sats") ?? 0))
    }()

    var onchainBalanceSats: UInt64 = {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        return UInt64(bitPattern: Int64(ud?.integer(forKey: "cached_onchain_sats") ?? 0))
    }()

    var hasReadyChannel: Bool = false

    /// The active LSP configuration managed by `LSPService`.
    var activeLSP: LSPConfig { lspService.activeLSP }

    var totalBalanceSats: UInt64 {
        if isChannelClosing {
            return onchainBalanceSats
        }
        if isOpeningChannel {
            return lightningBalanceSats > 0 ? lightningBalanceSats : onchainBalanceSats
        }
        if isSweeping {
            return lightningBalanceSats
        }
        // If no open channels but both balances exist, lightning balance is
        // pending-close claimable that overlaps with on-chain — avoid double-count.
        if !hasReadyChannel && lightningBalanceSats > 0 && onchainBalanceSats > 0 {
            return onchainBalanceSats
        }
        return lightningBalanceSats + onchainBalanceSats
    }

    var totalBalanceUSD: Double {
        guard btcPrice > 0 else { return 0 }
        return Double(totalBalanceSats) / Double(Constants.satsInBTC) * btcPrice
    }

    var stableUSD: Double { stableChannel.expectedUSD.amount }
    var nativeBTC: Bitcoin { stableChannel.nativeChannelBTC }

    // MARK: - Internal

    private var eventObserver: NSObjectProtocol?
    private var stabilityTimer: Task<Void, Never>?
    private var heartbeatTimer: Task<Void, Never>?
    private(set) var chainURL: String = Constants.primaryChainURL

    // Auto-sweep state
    private(set) var isSweeping = false
    var spliceTxid: String?
    private var spliceConfirmationTask: Task<Void, Never>?
    private var monitoredSpliceTxid: String?
    private var sweepOnchainStart: UInt64 = 0
    private var prevOnchainSats: UInt64 = {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        return UInt64(bitPattern: Int64(ud?.integer(forKey: "cached_onchain_sats") ?? 0))
    }()

    var fundingTxid: String? {
        didSet {
            UserDefaults(suiteName: Constants.appGroupIdentifier)?
                .set(fundingTxid, forKey: "funding_txid")
        }
    }

    // MARK: - Transaction Link Service

    let transactionLinkService = TransactionLinkService()
    let txidResolutionService = TxidResolutionService()

    // Pending trade payments — deferred until PaymentSuccessful/PaymentFailed
    var pendingTradePayments: [String: PendingTradePayment] = [:]

    /// Terminal result of a correlated trade, keyed by its fee payment id. Only a signed
    /// acceptance or rejection lands here — the trade sheets read this instead of inferring
    /// success from absence in the pending map (a rejection also clears pending; the old
    /// absence heuristic showed "Order Confirmed" for rejected trades — e2e flow 13).
    struct TradeOutcome: Equatable {
        let accepted: Bool
        let message: String
    }

    var tradeOutcomes: [String: TradeOutcome] = [:]

    /// Rehydrate a trade's terminal outcome from SQLite. The NSE and background handlers
    /// commit accepted/rejected results directly to the database without touching this
    /// in-memory map, so the trade sheets poll this while pending and it runs for every
    /// known payment id on startup and foreground.
    func refreshTradeOutcome(paymentId: String) {
        guard tradeOutcomes[paymentId] == nil, let db = databaseService else { return }
        guard let terminal =
            (try? db.channelRepo.terminalTradeOutcome(paymentId: paymentId)) ?? nil else { return }
        tradeOutcomes[paymentId] = terminal.accepted
            ? TradeOutcome(accepted: true, message: "")
            : TradeOutcome(
                accepted: false,
                message: TradeProtocol.rejectionMessage(terminal.reasonCode ?? "internal_failure")
            )
        pendingTradePayments.removeValue(forKey: paymentId)
    }

    private func refreshAllTradeOutcomes() {
        for paymentId in Set(tradeOutcomes.keys).union(pendingTradePayments.keys) {
            refreshTradeOutcome(paymentId: paymentId)
        }
    }

    // Pending splice info
    var pendingSplice: PendingSplice?

    enum WalletRestoreError: LocalizedError {
        case invalidMnemonic
        case activeChannelDetected
        case channelCheckUnavailable
        case walletBusy

        var errorDescription: String? {
            switch self {
            case .invalidMnemonic:
                return "Enter a valid 12 or 24-word seed phrase."
            case .activeChannelDetected:
                return "This wallet still has an open Lightning channel with the LSP. "
                    + "Restoring from seed alone will force-close it on-chain."
            case .channelCheckUnavailable:
                return "Could not verify whether this wallet has an open Lightning channel."
            case .walletBusy:
                return "Wallet is busy in another process. Please try again."
            }
        }
    }

    private func initializeDatabaseServices() throws {
        let db = try DatabaseService(dataDir: Constants.userDataDir)
        databaseService = db
        nodeService.databaseService = databaseService

        txidResolutionService.databaseService = databaseService
        txidResolutionService.mempoolWebSocketService = mempoolWebSocketService
        txidResolutionService.configureResolvers(urls: Constants.esploraChainURLs)

        txidResolutionService.didResolveTxid = { [weak self] event in
            switch event {
            case .channelClose(let opId, let closingTxid):
                self?.handleCloseTxidResolved(opId: opId, closingTxid: closingTxid)
            case .onchainReceive(let resolutionId, let txid):
                self?.handleOnchainReceiveResolved(resolutionId: resolutionId, txid: txid)
            }
        }

        tradeService = TradeService(nodeService: nodeService, databaseService: db)
        _ = try db.channelRepo.markExpiredTradesUncertain()
        pendingTradePayments = try db.channelRepo.unresolvedTradePayments()
        refreshAllTradeOutcomes()
        let pollingService = databaseService.map { db in
            ConfirmationPollingService(
                databaseService: db,
                blockHeightService: blockHeightService,
                confirmationService: confirmationService
            )
        }
        confirmationPollingService = pollingService
        pollingService?.onUpdate = { [weak self] in
            self?.confirmationUpdateEpoch += 1
        }
        if let db = databaseService, let polling = pollingService {
            let spvService = SPVHeaderChainService(
                databaseService: db,
                blockHeightService: blockHeightService,
                confirmationPollingService: polling
            )
            spvHeaderChainService = spvService

            // SPV path: advanceHeight() inside SPVHeaderChainService calls updateHeight(),
            // which fires onHeightUpdated → pollOnce(). Wire onHeightUpdated here so the
            // polling hook still runs for any height update that comes from HTTP polling
            // (BlockHeightService.refresh) rather than from the WebSocket block header.
            blockHeightService.onHeightUpdated = { [weak polling] _ in
                Task { @MainActor in
                    await polling?.pollOnce()
                }
            }

            mempoolWebSocketService.onBlockHeader = { [weak spvService] block in
                Task { @MainActor in
                    await spvService?.processBlockHeader(block)
                }
            }
        } else {
            // Fallback when DB is unavailable: still advance height and poll via HTTP.
            blockHeightService.onHeightUpdated = { [weak pollingService] _ in
                Task { @MainActor in
                    await pollingService?.pollOnce()
                }
            }
            mempoolWebSocketService.onBlockHeader = { [weak self] _ in
                Task { @MainActor in
                    await self?.blockHeightService.refresh()
                }
            }
        }
        mempoolWebSocketService.onTransactionDetected = { [weak self] event in
            Task { @MainActor in
                self?.handleWebSocketTransactionDetected(event: event)
            }
        }
    }

    /// Replace the active wallet with a restored seed in one app-owned flow.
    ///
    /// Restore is destructive for the current local wallet state, so AppState
    /// owns the full sequence: stop LDK, drop DB handles, wipe LDK + app DB
    /// files, clear in-memory/app-group cache, then start the node fresh.
    func restoreWalletFromMnemonic(_ mnemonic: String, acknowledgeForceClose: Bool = false) async throws {
        let words = MnemonicUtils.formatForDisplay(mnemonic)
        guard MnemonicUtils.isValidWordCount(words),
              MnemonicUtils.hasValidCharacterFormat(words) else {
            throw WalletRestoreError.invalidMnemonic
        }

        let priorPhase = phase
        phase = .syncing
        isSyncing = true
        statusMessage = "Restoring wallet..."
        nodeFlowInProgress = true
        defer {
            isSyncing = false
            nodeFlowInProgress = false
        }

        // Restore-divergence guard: a seed-only restore wipes LDK channel
        // state; if this node_id still has a channel with the LSP, the next
        // reestablish presents empty state and force-closes it. Detect that
        // and require explicit acknowledgement. Fails open if the LSP is
        // unreachable or derivation fails.
        if !acknowledgeForceClose {
            statusMessage = "Checking for an existing channel..."
            let derivedNodeId = await Task.detached(priority: .utility) {
                AppState.deriveNodeId(mnemonic: words)
            }.value
            var channelExists: Bool?
            if let nodeId = derivedNodeId {
                channelExists = await lspChannelExists(nodeId: nodeId)
            }
            switch channelExists {
            case .some(true):
                AuditService.log(
                    "RESTORE_ACTIVE_CHANNEL_DETECTED",
                    data: ["node_id": derivedNodeId ?? ""]
                )
                phase = priorPhase
                statusMessage = ""
                throw WalletRestoreError.activeChannelDetected
            case .none:
                // Guard couldn't run (LSP unreachable, proxy misroute, derive
                // failed). Fail-warn: surface it and require the user to opt
                // in, since a hidden open channel would be force-closed.
                AuditService.log(
                    "RESTORE_GUARD_UNAVAILABLE",
                    data: ["node_id": derivedNodeId ?? "derive_failed"]
                )
                phase = priorPhase
                statusMessage = ""
                throw WalletRestoreError.channelCheckUnavailable
            case .some(false):
                break // verified: no open channel — safe to proceed
            }
            statusMessage = "Restoring wallet..."
        }

        cancelBackgroundStop()

        // Own the wallet dir before stopping/wiping: restore can be reached
        // while the lock is not held (e.g. after a startup failure released
        // it), and wiping under a live NSE node would corrupt its state.
        if await !(NodeDirLock.shared.acquire(dataDir: Constants.userDataDir, timeout: 35)) {
            AuditService.log("NODE_LOCK_TIMEOUT", data: ["where": "restoreWalletFromMnemonic"])
            phase = priorPhase
            statusMessage = ""
            throw WalletRestoreError.walletBusy
        }

        // Execute staged restore transaction via decoupled WalletLifecycleManager
        do {
            try await lifecycleManager.restoreMnemonic(
                words,
                onStopNode: {
                    self.stabilityTimer?.cancel()
                    self.stabilityTimer = nil
                    self.txidResolutionService.cancelAllLaunchers()
                    self.nodeService.stop()
                    self.resetInMemoryWalletState()
                    self.dropDatabaseServices()
                },
                onWipePersistence: {
                    try self.wipeWalletPersistence()
                }
            )
        } catch {
            // Pre-stop failures (validation, pending-slot write) throw while the
            // node is still running — the lock must stay held then, or the NSE
            // could start a second node on the live wallet dir (the July
            // multi-writer force-close class).
            if !nodeService.isRunning {
                NodeDirLock.shared.release()
            }
            phase = .error("Restore failed: \(error.localizedDescription). Please retry.")
            statusMessage = ""
            throw error
        }

        do {
            try initializeDatabaseServices()
            try await startNodeWithFailover(mnemonic: words)

            let nodeId = nodeService.nodeId
            if !nodeId.isEmpty {
                UserDefaults(suiteName: Constants.appGroupIdentifier)?
                    .set(nodeId, forKey: "node_id")
            }

            phase = .wallet
            refreshBalances()
            updateStableBalances()
            startStabilityTimer()
            blockHeightService.start()
            mempoolWebSocketService.connect()
            if let addr = transactionLinkService.onchainReceiveAddress, !addr.isEmpty {
                mempoolWebSocketService.trackAddress(addr)
            }
            Task { await confirmationPollingService?.pollOnce() }
            reregisterPushTokenIfNeeded()
            statusMessage = ""
        } catch {
            // If we failed before the node came up (e.g. DB init threw), no
            // node owns the wallet dir — release so the NSE isn't blocked.
            // (startNodeWithFailover releases on its own failures; a second
            // release here is a safe no-op. If the node somehow IS running,
            // the lock must stay held.)
            if !nodeService.isRunning {
                NodeDirLock.shared.release()
            }
            phase = .error("Wallet restore failed: \(error.localizedDescription)")
            statusMessage = ""
            throw error
        }
    }

    private func resetInMemoryWalletState() {
        stableChannel = .default
        statusMessage = ""
        paymentFlash = false
        isChannelClosing = false
        isOpeningChannel = false
        isSweeping = false
        spliceTxid = nil
        spliceConfirmationTask?.cancel()
        spliceConfirmationTask = nil
        blockHeightService.stop()
        monitoredSpliceTxid = nil
        sweepOnchainStart = 0
        prevOnchainSats = 0
        fundingTxid = nil
        lightningBalanceSats = 0
        onchainBalanceSats = 0
        hasReadyChannel = false
        spendableOnchainSats = 0
        transactionLinkService.onchainReceiveAddress = nil
        transactionLinkService.clearCloseTxid()
        transactionLinkService.clearReceiveTxid()
        pendingTradePayments.removeAll()
        pendingSplice = nil

        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.removeObject(forKey: "funding_txid")
        shared?.removeObject(forKey: "node_id")
        shared?.set(Int64(0), forKey: "cached_lightning_sats")
        shared?.set(Int64(0), forKey: "cached_onchain_sats")
        shared?.set(false, forKey: "pending_push_payment")
    }

    private func dropDatabaseServices() {
        blockHeightService.stop()
        blockHeightService.onHeightUpdated = nil
        mempoolWebSocketService.disconnect()
        confirmationPollingService = nil
        nodeService.databaseService = nil
        nodeService.clearSavedMnemonic()
        tradeService = nil
        databaseService = nil
        txidResolutionService.clearResolvers()
    }

    private func wipeWalletPersistence() throws {
        try Self.wipeAllWalletState(wipePending: false)
    }

    static func wipeAllWalletState(wipePending: Bool = false) throws {
        let keychain: any MnemonicStorageProtocol = WalletKeychainService.shared
        try NodeService.wipeWalletData(keychain: keychain)

        let dir = Constants.userDataDir
        let filesToDelete = [
            DatabaseService.dbFilename,
            "\(DatabaseService.dbFilename)-wal",
            "\(DatabaseService.dbFilename)-shm"
        ]

        for file in filesToDelete {
            let path = dir.appendingPathComponent(file)
            if FileManager.default.fileExists(atPath: path.path) {
                try FileManager.default.removeItem(at: path)
            }
        }

        // Delete active keychain mnemonic
        try keychain.deleteMnemonic()

        // Only delete pending slot on explicit full wipe, never during restore wipe!
        if wipePending {
            try keychain.deletePendingMnemonic()
        }
    }

    /// Derive the node_id a mnemonic maps to by building (never starting) a
    /// throwaway node in a temp directory. Returns nil on any failure so the
    /// restore guard fails open.
    private nonisolated static func deriveNodeId(mnemonic: String) -> String? {
        // LDKNode's generated binding aborts the process (try!) on an invalid
        // mnemonic — full wordlist + checksum validation MUST run first. This
        // also makes the restore validator's `deriveNodeId != nil` check safe.
        guard let canonicalMnemonic = BIP39.validatedCanonicalMnemonic(mnemonic) else { return nil }
        let tmp = FileManager.default.temporaryDirectory
            .appendingPathComponent("nodeid-probe-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: tmp) }

        var config = defaultConfig()
        config.storageDirPath = tmp.path
        config.network = .bitcoin
        let builder = Builder.fromConfig(config: config)
        // A chain source is required to build; nothing syncs without start().
        let syncConfig = EsploraSyncConfig(
            backgroundSyncConfig: BackgroundSyncConfig(
                onchainWalletSyncIntervalSecs: Constants.onchainWalletSyncIntervalSecs,
                lightningWalletSyncIntervalSecs: Constants.lightningWalletSyncIntervalSecs,
                feeRateCacheUpdateIntervalSecs: Constants.feeRateCacheUpdateIntervalSecs
            ),
            timeoutsConfig: SyncTimeoutsConfig(
                onchainWalletSyncTimeoutSecs: 60,
                lightningWalletSyncTimeoutSecs: 60,
                feeRateCacheUpdateTimeoutSecs: 60,
                txBroadcastTimeoutSecs: 30,
                perRequestTimeoutSecs: 15
            )
        )
        builder.setChainSourceEsplora(serverUrl: Constants.primaryChainURL, config: syncConfig)
        let entropy = NodeEntropy.fromBip39Mnemonic(mnemonic: canonicalMnemonic, passphrase: nil)
        guard let node = try? builder.build(nodeEntropy: entropy) else { return nil }
        return node.nodeId()
    }

    /// Ask the LSP whether this node_id still has channels open with it.
    /// Returns nil (unknown) on any network/parse failure — callers fail open.
    private func lspChannelExists(nodeId: String) async -> Bool? {
        guard let url = URL(string: Constants.lspChannelExistsURL) else { return nil }
        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = try? JSONSerialization.data(withJSONObject: ["node_id": nodeId])
        request.timeoutInterval = 8
        guard let (data, response) = try? await URLSession.shared.data(for: request),
              (response as? HTTPURLResponse)?.statusCode == 200,
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let exists = json["exists"] as? Bool else {
            return nil
        }
        return exists
    }

    // MARK: - Startup

    func start() async {
        // Guard the whole flow: performBackgroundStop must not release the
        // wallet-dir lock while this startup is still live (there is a window
        // between our acquire and NodeService.start setting isStarting).
        nodeFlowInProgress = true
        defer { nodeFlowInProgress = false }

        // Logging must be live before the startup prologue: chain resolution and
        // the wallet-dir lock both emit audit events, and both run before
        // initializeDatabaseServices() sets the path.
        AuditService.setLogPath(
            Constants.userDataDir.appendingPathComponent("audit_log.txt").path
        )

        // The launch screen (phase == .loading) is shown for this entire prologue,
        // so every step below is measured — several of them previously logged only
        // on failure, which left the whole window invisible.
        let prologueStart = Date()
        func elapsedMs(_ since: Date) -> String { "\(Int(Date().timeIntervalSince(since) * 1000))" }

        // Migrate data from old Application Support dir to shared App Group container
        let migrateStart = Date()
        migrateDataDirIfNeeded()
        let migrateMs = elapsedMs(migrateStart)

        // Pick best esplora endpoint BEFORE taking the lock — pure network,
        // no DB access, and it can stall; no reason to hold the dir for it.
        let chainStart = Date()
        chainURL = await resolveChainURL()
        let chainMs = elapsedMs(chainStart)

        // Take the wallet-dir lock before any DB access (network-graph purge,
        // database init, node start). Kernel-enforced; outlasts a live NSE.
        let lockStart = Date()
        if await !(NodeDirLock.shared.acquire(dataDir: Constants.userDataDir, timeout: 35)) {
            AuditService.log("NODE_LOCK_TIMEOUT", data: ["where": "AppState.start"])
            await MainActor.run { phase = .error("Wallet is busy. Please reopen the app.") }
            return
        }
        let lockMs = elapsedMs(lockStart)

        // Self-healing restore recovery: Delegate to WalletLifecycleManager
        do {
            try lifecycleManager.runRecoveryIfNeeded {
                try self.wipeWalletPersistence()
            }
        } catch {
            await MainActor.run {
                phase = .error("Restore recovery failed: \(error.localizedDescription). Please restart the app.")
            }
            return // Stop startup, keep directory lock and marker intact
        }

        // Initialize database
        let dbStart = Date()
        do {
            try initializeDatabaseServices()
        } catch {
            // No node will run — free the wallet dir so the NSE isn't blocked
            // for the lifetime of a broken app process.
            NodeDirLock.shared.release()
            await MainActor.run { phase = .error("Database init failed: \(error.localizedDescription)") }
            return
        }

        let dbMs = elapsedMs(dbStart)

        // Load saved channel state from DB
        let dbReadStart = Date()
        loadChannelFromDB()

        // Refresh payment status from DB (NSE may have recorded payments while app was closed)
        refreshLatestPaymentStatus()

        // Seed historical price data for charts
        seedHistoricalPrices()
        let dbReadMs = elapsedMs(dbReadStart)

        AuditService.log("STARTUP_PROLOGUE", data: [
            "total_ms": elapsedMs(prologueStart),
            "migrate_ms": migrateMs,
            "chain_resolve_ms": chainMs,
            "dir_lock_ms": lockMs,
            "db_init_ms": dbMs,
            "db_read_ms": dbReadMs
        ])

        // Backfill hourly prices from Kraken for smooth 1D/1W/1M charts
        Task { await backfillHourlyPrices() }

        // Seed price from cache so UI can compute native USD immediately
        if stableChannel.latestPrice > 0 {
            priceService.seedDisplayPrice(stableChannel.latestPrice)
        }

        // Start price fetching
        priceService.startAutoRefresh()

        // Subscribe to LDK events
        subscribeToEvents()

        // Subscribe to push notifications (background wake)
        subscribeToPushNotifications()

        // Evaluate startup state using WalletLifecycleManager (Issue 13 / SOLID cleanup)
        switch lifecycleManager.detectStartupState() {
        case .ready:
            // Safe state: Both seed and database present. Start node.
            let hasCachedData = !stableChannel.userChannelId.isEmpty
            await MainActor.run {
                phase = hasCachedData ? .wallet : .syncing
                if hasCachedData {
                    isSyncing = true
                }
            }

            // Purge empty network graph from DB to force fresh RGS sync
            purgeEmptyNetworkGraph()

            do {
                try await startNodeWithFailover(mnemonic: "")
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
                    // Restore fundingTxid from UserDefaults
                    fundingTxid = UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .string(forKey: "funding_txid")
                    resumePendingSpliceConfirmation()
                    blockHeightService.start()
                    mempoolWebSocketService.connect()
                    Task { await confirmationPollingService?.pollOnce() }
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
                txidResolutionService.replayPendingChannelCloses()
                txidResolutionService.replayPendingOnchainReceives()
            } catch {
                await MainActor.run { phase = .error("Node start failed: \(error.localizedDescription)") }
            }
        case .newWallet:
            // Safe state: Neither present. Auto-create new wallet.
            await MainActor.run { phase = .syncing }
            do {
                try await startNodeWithFailover(mnemonic: "", allowCreate: true)
                let nodeId = nodeService.nodeId
                if !nodeId.isEmpty {
                    UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .set(nodeId, forKey: "node_id")
                }
                await MainActor.run {
                    phase = .wallet
                    blockHeightService.start()
                    mempoolWebSocketService.connect()
                    Task { await confirmationPollingService?.pollOnce() }
                    refreshBalances()
                    updateStableBalances()
                }
                startStabilityTimer()
                reregisterPushTokenIfNeeded()
                txidResolutionService.replayPendingChannelCloses()
                txidResolutionService.replayPendingOnchainReceives()
            } catch {
                await MainActor.run { phase = .error("Wallet creation failed: \(error.localizedDescription)") }
            }
        case .seedOnlyMismatch:
            // Mismatched state: Seed present but database missing
            AuditService.log("STARTUP_MISMATCH_SEED_ONLY", data: [:])
            NodeDirLock.shared.release()
            await MainActor.run {
                phase =
                    .error(
                        "Mismatched state: Wallet seed exists, but the channel database is missing. Please restore using your backup seed words."
                    )
            }
        case .dbOnlyMismatch:
            // Mismatched state: Database present but seed missing (e.g., device migration)
            AuditService.log("STARTUP_MISMATCH_DB_ONLY", data: [:])
            NodeDirLock.shared.release()
            await MainActor.run {
                phase =
                    .error(
                        "Mismatched state: Local channel database exists, but the wallet seed is missing. Please restore using your backup seed words."
                    )
            }
        case .seedStorageMismatch:
            AuditService.log("STARTUP_SEED_STORAGE_MISMATCH", data: [:])
            NodeDirLock.shared.release()
            await MainActor.run {
                phase =
                    .error(
                        "Mismatched state: Secure Keychain seed does not match plaintext backup file. Please restore using your backup seed words."
                    )
            }
        case .storageError(let msg):
            AuditService.log("STARTUP_STORAGE_ERROR", data: ["error": msg])
            NodeDirLock.shared.release()
            await MainActor.run {
                phase =
                    .error(
                        "Secure storage access failed: \(msg). Please verify your device credentials and restart the app."
                    )
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
            "to": newDir.path
        ])
    }

    // MARK: - NSE Coordination

    //
    // Coordination with the Notification Service Extension is handled entirely by
    // NodeDirLock (flock on ldk-node.lock). The kernel releases that lock when the
    // holding process dies, so it self-heals; the previous `nse_processing`
    // UserDefaults flag did not, and a jetsammed NSE left it set permanently —
    // every launch then burned the full 30s ceiling waiting on a dead process.

    func stop() {
        stabilityTimer?.cancel()
        stabilityTimer = nil
        heartbeatTimer?.cancel()
        heartbeatTimer = nil
        spliceConfirmationTask?.cancel()
        spliceConfirmationTask = nil
        blockHeightService.stop()
        monitoredSpliceTxid = nil
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
        NodeDirLock.shared.release()
    }

    private var backgroundStopWorkItem: DispatchWorkItem?
    /// True while a whole node-owning flow (start / foreground restart /
    /// restore) is live — including the stretch before NodeService.start sets
    /// isStarting. performBackgroundStop must not release the wallet-dir lock
    /// out from under such a flow.
    private var nodeFlowInProgress = false
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
            // Backgrounded before the node came up. If no start/restore flow
            // is live, a held lock serves no one — release it so the NSE isn't
            // starved of stability pushes for the suspended app's lifetime.
            // While a flow IS live (even pre-isStarting, e.g. stalled in DB
            // init), the lock stays: that flow owns it and may still resume.
            if !nodeService.isStarting && !nodeFlowInProgress {
                NodeDirLock.shared.release()
            }
            if backgroundTaskID != .invalid {
                UIApplication.shared.endBackgroundTask(backgroundTaskID)
                backgroundTaskID = .invalid
            }
            return
        }
        print("[App] Stopping node for background")
        stabilityTimer?.cancel()
        stabilityTimer = nil
        heartbeatTimer?.cancel()
        heartbeatTimer = nil
        spliceConfirmationTask?.cancel()
        spliceConfirmationTask = nil
        monitoredSpliceTxid = nil
        mempoolWebSocketService.disconnect()
        nodeService.stop()
        extractGossipFromDB()
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.set(Date().timeIntervalSince1970, forKey: "main_app_last_active")
        // Last DB write is done — hand the wallet dir to the NSE.
        NodeDirLock.shared.release()
        if backgroundTaskID != .invalid {
            UIApplication.shared.endBackgroundTask(backgroundTaskID)
            backgroundTaskID = .invalid
        }
    }

    /// Restore gossip data and restart the node when returning to foreground.
    func restartNodeFromForeground() async {
        guard case .wallet = phase else { return }
        nodeFlowInProgress = true
        defer { nodeFlowInProgress = false }
        cancelBackgroundStop()
        loadChannelFromDB()
        // Payments received while backgrounded are recorded by the NSE, not the foreground
        // event loop — so refresh the banner from the newest DB row instead of leaving it stale.
        refreshLatestPaymentStatus()
        // Same story for trade results: the NSE writes accepted/rejected to SQLite while
        // we're backgrounded, so pull any terminal outcomes into the in-memory map.
        refreshAllTradeOutcomes()
        if nodeService.isRunning {
            // Node never stopped, so gossip was never extracted — do NOT touch
            // ldk_node_data.sqlite while LDK has it open (a stale
            // network_graph.bin would otherwise be written into the live DB).
            print("[App] Node still running (grace period), reconnecting")
            ensureLSPConnected()
            refreshBalances()
            mempoolWebSocketService.connect()
            updateStableBalances()
            resumePendingSpliceConfirmation()
            return
        }
        print("[App] Restarting node from foreground")
        // Reclaim the wallet dir before writing gossip back into the LDK DB.
        if await !(NodeDirLock.shared.acquire(dataDir: Constants.userDataDir, timeout: 35)) {
            AuditService.log("NODE_LOCK_TIMEOUT", data: ["where": "restartNodeFromForeground"])
            return
        }
        restoreGossipToDB()
        do {
            try await startNodeWithFailover(mnemonic: "")
            refreshBalances()
            blockHeightService.start()
            mempoolWebSocketService.connect()
            Task { await confirmationPollingService?.pollOnce() }
            updateStableBalances()
            StabilityService.reconcileIncoming(&stableChannel)
            saveChannelToDB()
            resumePendingSpliceConfirmation()
            reregisterPushTokenIfNeeded()
            startStabilityTimer()
            await processPendingPushPayment()
        } catch {
            print("[App] Node restart failed: \(error)")
        }
    }

    /// Set the home banner from the newest recorded payment. Payments received while the app was
    /// backgrounded are persisted by the NSE, so the foreground event loop never sees them and
    /// `statusMessage` would otherwise stay frozen on the last foreground-processed payment.
    private func refreshLatestPaymentStatus() {
        guard let db = databaseService,
              let latest = db.paymentRepo.latestReceivedPayment() else { return }
        if let usd = latest.amountUSD {
            statusMessage = "Payment received: \(usd.usdFormatted)"
        } else if let usd = usdValue(sats: latest.amountSats, rowPrice: latest.btcPrice) {
            statusMessage = "Payment received: \(usd.usdFormatted)"
        } else {
            statusMessage = "Payment received: \(latest.amountSats.btcSpacedFormatted) BTC"
        }
    }

    /// USD value for a payment row that was recorded without one: prefer the
    /// price stored on the row, else the current price. Nil only if no price
    /// is available at all.
    private func usdValue(sats: UInt64, rowPrice: Double?) -> Double? {
        let price: Double
        if let rowPrice, rowPrice > 0 {
            price = rowPrice
        } else if btcPrice > 0 {
            price = btcPrice
        } else if stableChannel.latestPrice > 0 {
            price = stableChannel.latestPrice
        } else {
            return nil
        }
        return Double(sats) / Double(Constants.satsInBTC) * price
    }

    private func sentPaymentStatusMessage(paymentId: PaymentId?) -> String {
        guard let paymentId,
              let payment = databaseService?.paymentRepo.payment(paymentId: "\(paymentId)") else {
            return "Payment sent"
        }
        if let usd = payment.amountUSD {
            return "Payment sent: \(usd.usdFormatted)"
        }
        if let usd = usdValue(sats: payment.amountSats, rowPrice: payment.btcPrice) {
            return "Payment sent: \(usd.usdFormatted)"
        }
        return "Payment sent: \(payment.amountSats.btcSpacedFormatted) BTC"
    }

    // MARK: - Gossip Data Management

    /// Extract network_graph blob from SQLite to a file, then delete it from the DB.
    /// This shrinks the DB from ~8.7MB to ~30KB so the NSE can load it.
    private func extractGossipFromDB() {
        let dbPath = Constants.userDataDir.appendingPathComponent("ldk_node_data.sqlite").path
        let gossipPath = Constants.userDataDir.appendingPathComponent("network_graph.bin")
        let metricsPath = Constants.userDataDir.appendingPathComponent("node_metrics.bin")

        var db: OpaquePointer?
        guard sqlite3_open(dbPath, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }

        // Extract network_graph
        var stmt: OpaquePointer?
        let query = "SELECT value FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, query, -1, &stmt, nil) == SQLITE_OK else { return }

        if sqlite3_step(stmt) == SQLITE_ROW {
            let blobSize = sqlite3_column_bytes(stmt, 0)
            if blobSize > 1024, let blobPtr = sqlite3_column_blob(stmt, 0) {
                let data = Data(bytes: blobPtr, count: Int(blobSize))
                try? data.write(to: gossipPath)
                print("[App] Saved network_graph (\(blobSize) bytes) to file")
            }
        }
        sqlite3_finalize(stmt)

        // Extract node_metrics (contains RGS timestamp)
        var metricsStmt: OpaquePointer?
        let metricsQuery = "SELECT value FROM ldk_node_data WHERE key = 'node_metrics'"
        if sqlite3_prepare_v2(db, metricsQuery, -1, &metricsStmt, nil) == SQLITE_OK {
            if sqlite3_step(metricsStmt) == SQLITE_ROW {
                let size = sqlite3_column_bytes(metricsStmt, 0)
                if size > 0, let ptr = sqlite3_column_blob(metricsStmt, 0) {
                    try? Data(bytes: ptr, count: Int(size)).write(to: metricsPath)
                }
            }
            sqlite3_finalize(metricsStmt)
        }

        // Only delete from DB if file was saved successfully
        if FileManager.default.fileExists(atPath: gossipPath.path) {
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'node_metrics'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'scorer'", nil, nil, nil)
            print("[App] Stripped gossip data from DB")
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

    /// Restore network_graph and node_metrics from files back into SQLite.
    private func restoreGossipToDB() {
        let dbPath = Constants.userDataDir.appendingPathComponent("ldk_node_data.sqlite").path
        let gossipPath = Constants.userDataDir.appendingPathComponent("network_graph.bin")
        let metricsPath = Constants.userDataDir.appendingPathComponent("node_metrics.bin")

        guard FileManager.default.fileExists(atPath: gossipPath.path) else { return }
        guard let graphData = try? Data(contentsOf: gossipPath) else { return }

        // Skip restoring an empty/corrupt graph
        if graphData.count < 1024 {
            print("[App] network_graph.bin too small (\(graphData.count) bytes), deleting for fresh sync")
            try? FileManager.default.removeItem(at: gossipPath)
            try? FileManager.default.removeItem(at: metricsPath)
            return
        }

        var db: OpaquePointer?
        guard sqlite3_open(dbPath, &db) == SQLITE_OK else { return }
        defer { sqlite3_close(db) }

        // Restore network_graph
        let upsert = "INSERT OR REPLACE INTO ldk_node_data (primary_namespace, secondary_namespace, key, value) VALUES ('', '', 'network_graph', ?)"
        var stmt: OpaquePointer?
        if sqlite3_prepare_v2(db, upsert, -1, &stmt, nil) == SQLITE_OK {
            graphData.withUnsafeBytes { ptr in
                sqlite3_bind_blob(stmt, 1, ptr.baseAddress, Int32(graphData.count), nil)
            }
            sqlite3_step(stmt)
            sqlite3_finalize(stmt)
        }

        // Restore node_metrics (contains RGS timestamp — must match the graph)
        if let metricsData = try? Data(contentsOf: metricsPath), !metricsData.isEmpty {
            let metricsUpsert = "INSERT OR REPLACE INTO ldk_node_data (primary_namespace, secondary_namespace, key, value) VALUES ('', '', 'node_metrics', ?)"
            var metricsStmt: OpaquePointer?
            if sqlite3_prepare_v2(db, metricsUpsert, -1, &metricsStmt, nil) == SQLITE_OK {
                metricsData.withUnsafeBytes { ptr in
                    sqlite3_bind_blob(metricsStmt, 1, ptr.baseAddress, Int32(metricsData.count), nil)
                }
                sqlite3_step(metricsStmt)
                sqlite3_finalize(metricsStmt)
            }
        }

        // Clean up files
        try? FileManager.default.removeItem(at: gossipPath)
        try? FileManager.default.removeItem(at: metricsPath)
        print("[App] Restored network_graph (\(graphData.count) bytes) + node_metrics to DB")
    }

    // MARK: - Push Token Re-registration

    /// Re-register the push token with the LSP now that we have a node_id.
    /// The initial registration may have happened before the node started.
    private func reregisterPushTokenIfNeeded() {
        guard let token = UserDefaults.standard.string(forKey: "apns_device_token"),
              !nodeService.nodeId.isEmpty else { return }

        let nodeId = nodeService.nodeId
        guard let url = URL(string: Constants.lspPushRegisterURL) else { return }

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
                "environment": apnsEnvironment
            ]

            guard let httpBody = try? JSONSerialization.data(withJSONObject: body) else { return }
            request.httpBody = httpBody

            do {
                let (_, response) = try await URLSession.shared.data(for: request)
                if response is HTTPURLResponse {
                    print("[Push] Re-registered with node_id: \(nodeId.prefix(16))...")
                }
            } catch {
                print("[Push] Re-registration failed: \(error.localizedDescription)")
            }
        }
    }

    // MARK: - Pending Push Payment (app was killed)

    /// Check if NSE flagged a pending payment while app was killed
    /// Reconnect to LSP so pending stability payment can land
    private func processPendingPushPayment() async {
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        guard shared?.bool(forKey: "pending_push_payment") == true else { return }

        print("[Push] Processing pending push payment from NSE flag")
        guard reconcilePendingOutgoingStabilityPayment() else {
            shared?.set(true, forKey: "pending_push_payment")
            return
        }

        // Reconnect to LSP so pending payment can be received
        do {
            try nodeService.node?.connect(
                nodeId: activeLSP.pubkey,
                address: activeLSP.address,
                persist: true
            )

            Self.updatePendingPushPaymentFlag(shared, reconnectSucceeded: true)
            refreshBalances()
            updateStableBalances()

            AuditService.log("PUSH_PENDING_PAYMENT_RECONNECT_OK", data: [
                "node_running": "\(nodeService.isRunning)"
            ])
        } catch {
            Self.updatePendingPushPaymentFlag(shared, reconnectSucceeded: false)
            AuditService.log("PUSH_PENDING_PAYMENT_RECONNECT_FAILED", data: [
                "error": error.localizedDescription,
                "node_running": "\(nodeService.isRunning)"
            ])
        }
    }

    static func updatePendingPushPaymentFlag(_ shared: UserDefaults?, reconnectSucceeded: Bool) {
        if reconnectSucceeded {
            // Clear pending marker only after successful reconnect attempt
            shared?.set(false, forKey: "pending_push_payment")
        } else {
            // Keep marker set so foreground or startup can retry
            shared?.set(true, forKey: "pending_push_payment")
        }
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
                nodeId: self.activeLSP.pubkey,
                address: self.activeLSP.address,
                persist: true
            )
            self.refreshBalances()
            self.updateStableBalances()

            AuditService.log("PUSH_WAKE", data: [
                "node_running": "\(self.nodeService.isRunning)"
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
            let ackToken = notification.userInfo?["ackToken"] as? EventAckToken
            self.handleEvent(event, ackToken: ackToken)
        }
    }

    private func handleEvent(_ event: Event, ackToken: EventAckToken? = nil) {
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
                "funding_txo": "\(fundingTxo)"
            ])

        case .channelReady(let channelId, let userChannelId, _, _):
            // In 0-conf channels, ChannelReady can fire before the splice tx confirms.
            // Treat it as metadata only; the splice stays pending until the tx has 1 conf.
            let channelIdChanged = stableChannel.userChannelId == userChannelId
                && !stableChannel.channelId.isEmpty
                && stableChannel.channelId != channelId

            stableChannel.channelId = channelId
            refreshBalances()
            updateStableBalances()

            var pendingSpliceCandidate: String?
            if stableChannel.userChannelId == userChannelId {
                nodeService.refreshChannels()
                let fundingTxid: String? = nodeService.channels
                    .first(where: { "\($0.userChannelId)" == "\(userChannelId)" })
                    .flatMap { $0.fundingTxo.map { "\($0.txid)" } }
                let pendingDbTxid = (try? databaseService?.spliceRepo.getPendingSpliceTxid()) ?? nil
                pendingSpliceCandidate = [pendingDbTxid, spliceTxid]
                    .compactMap { $0 }
                    .first { candidate in
                        !candidate.isEmpty && candidate == fundingTxid
                    }
            }
            let isSplice = pendingSpliceCandidate != nil || channelIdChanged

            if isSplice {
                isSweeping = true
                let txid = pendingSpliceCandidate ?? spliceTxid ?? fundingTxid
                spliceTxid = txid
                if let txid, !txid.isEmpty {
                    startSpliceConfirmationMonitor(txid: txid)
                }
                statusMessage = "Move pending confirmation"
            }

            saveChannelToDB()

            AuditService.log("CHANNEL_READY", data: [
                "channel_id": channelId,
                "user_channel_id": userChannelId
            ])
            if !isSplice {
                statusMessage = "Channel is ready"
            }

        case .paymentReceived(let paymentId, let paymentHash, let amountMsat, let customRecords):
            handlePaymentReceived(
                paymentId: paymentId,
                amountMsat: amountMsat,
                paymentHash: paymentHash,
                customRecords: customRecords,
                ackToken: ackToken
            )

        case .paymentSuccessful(let paymentId, let paymentHash, _, let feePaidMsat, _):
            handlePaymentSuccessful(
                paymentId: paymentId,
                paymentHash: paymentHash,
                feePaidMsat: feePaidMsat
            )

        case .paymentFailed(let paymentId, let paymentHash, let reason):
            // Check if this is a pending trade payment
            var failedTrade = paymentId.flatMap { pendingTradePayments["\($0)"] }
            if let pid = paymentId, failedTrade == nil,
               let payment = nodeService.node?.payment(paymentId: pid),
               payment.direction == .outbound,
               let amountMsat = payment.amountMsat {
                failedTrade = try? databaseService?.channelRepo.failUnattachedPreparedTrade(
                    paymentId: "\(pid)",
                    amountMsat: amountMsat
                )
            }
            if let pid = paymentId, let trade = failedTrade {
                pendingTradePayments.removeValue(forKey: "\(pid)")
                _ = try? databaseService?.channelRepo.markTradeSendFailed(tradeDbId: trade.tradeDbId)
                statusMessage = "Order failed"

                AuditService.log("TRADE_FAILED", data: [
                    "payment_hash": paymentHash.map { "\($0)" } ?? "nil",
                    "action": trade.action,
                    "new_expected_usd": "\(trade.newExpectedUSD)",
                    "reason": reason.map { "\($0)" } ?? "unknown"
                ])
            } else {
                // Update payment status in DB
                if let pid = paymentId {
                    try? databaseService?.paymentRepo.updatePaymentStatus(
                        paymentId: "\(pid)", status: "failed"
                    )
                }
                // If this is the in-flight stability send, the failure means no sats
                // moved — clear the marker so future sends are unblocked (no debit).
                if let pid = paymentId,
                   let pending = databaseService?.stabilityRepo.loadPendingSend(),
                   !pending.paymentId.isEmpty,
                   pending.paymentId == "\(pid)" {
                    databaseService?.stabilityRepo.clearPendingSend()
                    AuditService.log("STABILITY_PAYMENT_SEND_MARKER_CLEARED", data: [
                        "payment_id": "\(pid)",
                        "reason": "payment_failed"
                    ])
                }
                AuditService.log("PAYMENT_FAILED", data: [
                    "payment_id": paymentId.map { "\($0)" } ?? "nil",
                    "payment_hash": paymentHash.map { "\($0)" } ?? "nil",
                    "reason": reason.map { "\($0)" } ?? "unknown"
                ])
                let reasonStr = reason.map { "\($0)" } ?? "unknown"
                statusMessage = "Payment failed: \(reasonStr)"
            }

        case .spliceNegotiated(let channelId, let userChannelId, _, let newFundingTxo):
            handleSplicePending(
                channelId: channelId,
                userChannelId: userChannelId,
                newFundingTxo: newFundingTxo
            )

        case .spliceNegotiationFailed(let channelId, let userChannelId, _):
            isSweeping = false
            spliceTxid = nil
            spliceConfirmationTask?.cancel()
            spliceConfirmationTask = nil
            monitoredSpliceTxid = nil
            sweepOnchainStart = 0
            pendingSplice = nil
            databaseService?.spliceRepo.failLatestPendingSplice()

            AuditService.log("SPLICE_FAILED", data: [
                "channel_id": "\(channelId)",
                "user_channel_id": "\(userChannelId)"
            ])
            statusMessage = "Splice failed"

        case .channelClosed(let channelId, let userChannelId, let counterpartyNodeId, let reason):
            handleChannelClosed(
                channelId: channelId,
                userChannelId: userChannelId,
                counterpartyNodeId: counterpartyNodeId,
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
        customRecords: [CustomTlvRecord],
        ackToken: EventAckToken? = nil
    ) {
        let paymentHashStr = "\(paymentHash)"
        let paymentIdStr = paymentId.map { "\($0)" } ?? paymentHashStr

        // Check for SYNC_V1 message from LSP. A valid sync that cannot yet be applied must
        // remain in LDK's event queue; treating it like malformed control traffic loses it.
        switch handleSyncMessage(
            customRecords: customRecords,
            paymentHash: paymentHashStr,
            amountMsat: amountMsat
        ) {
        case .applied:
            refreshBalances()
            updateStableBalances()
            return
        case .retry:
            ackToken?.shouldAck = false
            return
        case .notSync:
            break
        }

        let hasStableControlTLV = customRecords.contains {
            $0.typeNum == Constants.stableChannelTLVType && $0.value != Data([1])
        }
        if hasStableControlTLV || amountMsat < 1000 {
            // A non-marker stable TLV is a control message, not a user payment.
            // If it wasn't a valid SYNC_V1 above, don't turn it into a fake
            // "Received 0 sats" lightning row. Ack it so a malformed/unknown
            // control packet cannot loop forever in the foreground.
            AuditService.log("PAYMENT_RECEIVED_IGNORED", data: [
                "payment_hash": paymentHashStr,
                "amount_msat": "\(amountMsat)",
                "reason": hasStableControlTLV ? "unhandled_stable_control_tlv" : "sub_sat_amount"
            ])
            return
        }

        // Normal payment received
        AuditService.log("PAYMENT_RECEIVED", data: [
            "amount_msat": "\(amountMsat)",
            "payment_id": paymentIdStr,
            "payment_hash": paymentHashStr
        ])

        let price = stableChannel.latestPrice
        let amountUSD: Double? = price > 0 ? (Double(amountMsat) / 1000.0 / 100_000_000.0) * price : nil
        let isStabilityPayment = customRecords
            .contains { $0.typeNum == Constants.stableChannelTLVType && $0.value == Data([1]) }
        let paymentType = isStabilityPayment ? "stability" : "lightning"
        let backingDelta: Int64? = isStabilityPayment ? Int64(amountMsat / 1000) : nil

        // Atomically insert payment row and increment backing sats in one SQLite transaction.
        // On DB failure, veto the ack so LDK re-delivers the event.
        guard let databaseService else {
            ackToken?.shouldAck = false
            return
        }
        let record: () throws -> PaymentPersistenceResult = {
            try databaseService.paymentRepo.recordPaymentAndMaybeUpdateBacking(
                paymentId: paymentIdStr,
                paymentType: paymentType,
                direction: "received",
                amountMsat: amountMsat,
                amountUSD: amountUSD,
                btcPrice: price > 0 ? price : nil,
                status: "completed",
                userChannelId: isStabilityPayment ? self.stableChannel.userChannelId : nil,
                backingDeltaSats: backingDelta
            )
        }
        let persistence: PaymentPersistenceResult
        do {
            persistence = try record()
        } catch DatabaseError.missingChannelRow(let ucid) {
            // The channels row vanished (e.g. fresh DB). Recreate it from in-memory
            // state via the full save, then retry the atomic record exactly once.
            AuditService.log("PAYMENT_CHANNEL_ROW_MISSING", data: [
                "user_channel_id": ucid,
                "payment_id": paymentIdStr
            ])
            saveChannelToDB()
            do {
                persistence = try record()
            } catch {
                ackToken?.shouldAck = false
                return
            }
        } catch {
            ackToken?.shouldAck = false
            return
        }

        refreshBalances()
        updateStableBalances()
        if isStabilityPayment {
            guard let backing = persistence.backingSats else {
                ackToken?.shouldAck = false
                return
            }
            stableChannel.backingSats = backing
        }
        StabilityService.reconcileIncoming(&stableChannel)
        saveChannelToDB(preserveBacking: isStabilityPayment)

        // Only announce genuinely new receipts. A re-delivered/duplicate event dedups to
        // isNewPayment == false; announcing it would overwrite the banner with a stale amount
        // (recomputed at the current price), which is exactly the "reverted to $0.57" bug.
        guard persistence.isNewPayment else { return }

        if let usd = amountUSD {
            statusMessage = "Payment received: \(usd.usdFormatted)"
        } else if let usd = usdValue(sats: amountMsat / 1000, rowPrice: nil) {
            statusMessage = "Payment received: \(usd.usdFormatted)"
        } else {
            let sats = amountMsat / 1000
            statusMessage = "Payment received: \(sats.btcSpacedFormatted) BTC"
        }

        // Trigger payment received animation
        paymentFlash = true
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
            self?.paymentFlash = false
        }
    }

    /// Parse a SYNC_V1 TLV and distinguish invalid control traffic from retryable processing.
    private func handleSyncMessage(
        customRecords: [CustomTlvRecord],
        paymentHash: String,
        amountMsat: UInt64
    ) -> SyncMessageHandlingResult {
        for tlv in customRecords {
            guard tlv.typeNum == Constants.stableChannelTLVType else { continue }

            guard let message = TradeProtocol.parseSignedControl(
                data: tlv.value,
                expectedCounterparty: stableChannel.counterparty,
                verifySignature: { [weak self] msg, sig, pubkey in
                    self?.nodeService.verifySignature(message: msg, signature: sig, pubkey: pubkey) ?? false
                }
            ) else { continue }
            guard amountMsat == TradeProtocol.resultControlAmountMsat else {
                AuditService.log("TRADE_RESULT_CONTROL_AMOUNT_INVALID", data: [
                    "amount_msat": "\(amountMsat)",
                    "payment_hash": paymentHash
                ])
                return .applied
            }
            guard let databaseService else { return .retry }
            let result: TradeControlApplyResult
            switch message {
            case .rejected(let rejection):
                result = databaseService.channelRepo.applyTradeRejection(rejection)
            case .sync(let sync):
                if sync.correlation != nil {
                    result = databaseService.channelRepo.applyCorrelatedTradeAcceptance(sync)
                } else {
                    let price = accountingBTCPrice
                    guard price > 0 else {
                        AuditService.log("SYNC_V1_DEFERRED", data: [
                            "reason": "untrusted_price",
                            "payment_hash": paymentHash
                        ])
                        return .retry
                    }
                    result = databaseService.channelRepo.applyUncorrelatedSyncIfNewer(
                        sync,
                        trustedPrice: price
                    )
                }
            }
            switch result.status {
            case .retry:
                _ = try? databaseService.channelRepo.markTradeResponseNotCommittable(message)
                AuditService.log("TRADE_RESULT_DEFERRED", data: ["payment_hash": paymentHash])
                return .retry
            case .invalid:
                AuditService.log("TRADE_RESULT_INVALID", data: ["payment_hash": paymentHash])
                return .applied
            case .duplicate:
                if let paymentId = result.paymentId {
                    pendingTradePayments.removeValue(forKey: paymentId)
                    if case .rejected(let rejection) = message {
                        tradeOutcomes[paymentId] = TradeOutcome(
                            accepted: false,
                            message: TradeProtocol.rejectionMessage(rejection.reasonCode)
                        )
                    } else {
                        tradeOutcomes[paymentId] = TradeOutcome(accepted: true, message: "")
                    }
                }
                guard let channel = try? databaseService.channelRepo.loadChannel(
                    userChannelId: stableChannel.userChannelId
                ) else { return .retry }
                stableChannel.channelId = channel.channelId
                stableChannel.expectedUSD = USD(amount: channel.expectedUSD)
                stableChannel.backingSats = channel.backingSats
                stableChannel.nativeSats = channel.nativeSats
                stableChannel.nativeChannelBTC = Bitcoin(sats: channel.nativeSats)
                stableChannel.latestPrice = channel.latestPrice
                return .applied
            case .applied:
                if let paymentId = result.paymentId {
                    pendingTradePayments.removeValue(forKey: paymentId)
                    if case .rejected(let rejection) = message {
                        tradeOutcomes[paymentId] = TradeOutcome(
                            accepted: false,
                            message: TradeProtocol.rejectionMessage(rejection.reasonCode)
                        )
                    } else {
                        tradeOutcomes[paymentId] = TradeOutcome(accepted: true, message: "")
                    }
                }
                guard let channel = try? databaseService.channelRepo.loadChannel(
                    userChannelId: stableChannel.userChannelId
                ) else { return .retry }
                stableChannel.channelId = channel.channelId
                stableChannel.expectedUSD = USD(amount: channel.expectedUSD)
                stableChannel.backingSats = channel.backingSats
                stableChannel.nativeSats = channel.nativeSats
                stableChannel.nativeChannelBTC = Bitcoin(sats: channel.nativeSats)
                stableChannel.latestPrice = channel.latestPrice
                let diverged = result.localBackingSats != nil &&
                    result.peerBackingSats != nil &&
                    result.localBackingSats != result.peerBackingSats
                AuditService.log("TRADE_RESULT_APPLIED", data: [
                    "payment_hash": paymentHash,
                    "local_backing_sats": result.localBackingSats.map { "\($0)" } ?? "nil",
                    "peer_backing_sats": result.peerBackingSats.map { "\($0)" } ?? "nil",
                    "allocation_diverged": "\(diverged)",
                    "allocation_applied": result.allocationApplied.map { "\($0)" } ?? "nil"
                ])
                switch message {
                case .rejected(let rejection):
                    statusMessage = TradeProtocol.rejectionMessage(rejection.reasonCode)
                case .sync:
                    if result.paymentId != nil {
                        statusMessage = "Order confirmed"
                        paymentFlash = true
                        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                            self?.paymentFlash = false
                        }
                    }
                }
                return .applied
            }
        }
        return .notSync
    }

    // MARK: - Payment Successful

    private func handlePaymentSuccessful(
        paymentId: PaymentId?,
        paymentHash: PaymentHash,
        feePaidMsat: UInt64?
    ) {
        let paymentHashStr = "\(paymentHash)"

        // Fee settlement is not trade acceptance. Keep the durable correlation pending until a
        // signed SYNC_V1 or TRADE_REJECTED_V1 commits the outcome.
        if let pid = paymentId {
            let paymentIdString = "\(pid)"
            var trade = pendingTradePayments[paymentIdString]
            var recognizedTrade = trade != nil
            if let databaseService {
                let marked: Bool
                if let trade {
                    marked = (try? databaseService.channelRepo.markKnownTradeFeePaid(
                        tradeDbId: trade.tradeDbId,
                        paymentId: paymentIdString
                    )) == true
                } else {
                    marked = (try? databaseService.channelRepo.markTradeFeePaid(
                        paymentId: paymentIdString
                    )) == true
                }
                recognizedTrade = recognizedTrade || marked

                let paymentDetails = recognizedTrade ? nil : nodeService.node?.payment(paymentId: pid)
                let eventAmountMsat = paymentDetails?.direction == .outbound
                    ? paymentDetails?.amountMsat
                    : nil
                if !recognizedTrade, let eventAmountMsat {
                    trade = try? databaseService.channelRepo.adoptUnattachedPreparedTrade(
                        paymentId: paymentIdString,
                        amountMsat: eventAmountMsat
                    )
                    recognizedTrade = trade != nil
                }
                if !recognizedTrade {
                    recognizedTrade = (try? databaseService.channelRepo.tradePaymentExists(
                        paymentId: paymentIdString
                    )) == true
                }
                if !recognizedTrade, eventAmountMsat == nil,
                   (try? databaseService.channelRepo.hasUnattachedPreparedTrade()) == true {
                    statusMessage = "Payment confirmed; awaiting signed trade result"
                    AuditService.log("TRADE_FEE_ID_UNRESOLVED", data: [
                        "payment_id": paymentIdString
                    ])
                    return
                }
            }
            if recognizedTrade {
                if let trade {
                    pendingTradePayments[paymentIdString] = PendingTradePayment(
                        newExpectedUSD: trade.newExpectedUSD,
                        price: trade.price,
                        tradeDbId: trade.tradeDbId,
                        action: trade.action,
                        status: "fee_paid"
                    )
                }
                AuditService.log("TRADE_FEE_PAID", data: [
                    "payment_hash": paymentHashStr,
                    "action": trade?.action ?? "unknown",
                    "new_expected_usd": trade.map { "\($0.newExpectedUSD)" } ?? "unknown",
                    "fee_paid_msat": feePaidMsat.map { "\($0)" } ?? "nil"
                ])
                statusMessage = "Trade fee paid; awaiting signed result"
                return
            }
        }

        if handleStabilityPaymentSuccessful(paymentId: paymentId, feePaidMsat: feePaidMsat) {
            return
        }

        // Normal (non-trade) outgoing payment
        refreshBalances()
        updateStableBalances()

        let price = stableChannel.latestPrice

        // Reconcile: if outgoing payment exceeded native BTC, deduct from stable
        let oldExpected = stableChannel.expectedUSD.amount
        if let usdDeducted = StabilityService.reconcileOutgoing(&stableChannel, price: price) {
            // Set cooldown so stability check doesn't immediately re-fire
            stableChannel.lastStabilityPayment = Int64(Date().timeIntervalSince1970)
            saveChannelToDB()
            AuditService.log("OUTGOING_STABLE_DEDUCTED", data: [
                "payment_hash": paymentHashStr,
                "usd_deducted": "\(usdDeducted)",
                "old_expected_usd": "\(oldExpected)",
                "new_expected_usd": "\(stableChannel.expectedUSD.amount)",
                "btc_price": "\(price)"
            ])
        }

        // Update payment status in DB
        if let pidStr = paymentId.map({ "\($0)" }) {
            try? databaseService?.paymentRepo.updatePaymentStatus(
                paymentId: pidStr,
                status: "completed",
                feeMsat: feePaidMsat
            )
        }

        saveChannelToDB()

        AuditService.log("PAYMENT_SUCCESSFUL", data: [
            "payment_hash": paymentHashStr,
            "fee_paid_msat": feePaidMsat.map { "\($0)" } ?? "nil"
        ])
        statusMessage = sentPaymentStatusMessage(paymentId: paymentId)
    }

    private func handleStabilityPaymentSuccessful(
        paymentId: PaymentId?,
        feePaidMsat: UInt64?
    ) -> Bool {
        let paymentIdString = paymentId.map { "\($0)" }
        if var pending = databaseService?.stabilityRepo.loadPendingSend() {
            if pending.paymentId.isEmpty {
                // Process died between the keysend and the marker update. Adopt this
                // event if it is our send (amount matches the marker), then reconcile
                // through the normal non-empty-id path below.
                if let paymentId, let paymentIdString,
                   let details = nodeService.node?.payment(paymentId: paymentId),
                   details.direction == .outbound,
                   details.amountMsat == pending.amountMsat {
                    databaseService?.stabilityRepo.setPendingSendPaymentId(paymentIdString)
                    pending = PendingStabilitySend(
                        paymentId: paymentIdString,
                        amountMsat: pending.amountMsat,
                        price: pending.price,
                        createdAt: pending.createdAt
                    )
                    AuditService.log("STABILITY_PAYMENT_MARKER_ADOPTED", data: [
                        "payment_id": paymentIdString,
                        "amount_msat": "\(pending.amountMsat)"
                    ])
                } else {
                    // Not our send (or node unavailable). Leave the marker for the
                    // reconcile path and avoid flushing in-memory backing through
                    // the normal outgoing-payment path.
                    UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .set(true, forKey: "pending_push_payment")
                    if let paymentIdString {
                        try? databaseService?.paymentRepo.updatePaymentStatus(
                            paymentId: paymentIdString,
                            status: "completed",
                            feeMsat: feePaidMsat
                        )
                    }
                    saveChannelToDB(preserveBacking: true)
                    statusMessage = "Payment confirmed; syncing stability payment"
                    return true
                }
            }

            let matchesPendingStabilityPayment = paymentIdString.map { $0 == pending.paymentId } ?? false
            let reconciled = reconcilePendingOutgoingStabilityPayment(
                status: matchesPendingStabilityPayment ? "completed" : "pending"
            )
            if matchesPendingStabilityPayment {
                if reconciled {
                    if let paymentIdString {
                        try? databaseService?.paymentRepo.updatePaymentStatus(
                            paymentId: paymentIdString,
                            status: "completed",
                            feeMsat: feePaidMsat
                        )
                    }
                    refreshBalances()
                    updateStableBalances()
                    statusMessage = "Payment confirmed"
                } else {
                    UserDefaults(suiteName: Constants.appGroupIdentifier)?
                        .set(true, forKey: "pending_push_payment")
                    saveChannelToDB(preserveBacking: true)
                    statusMessage = "Payment confirmed; syncing stability payment"
                }
                return true
            }

            if !reconciled {
                if let paymentIdString {
                    try? databaseService?.paymentRepo.updatePaymentStatus(
                        paymentId: paymentIdString,
                        status: "completed",
                        feeMsat: feePaidMsat
                    )
                }
                saveChannelToDB(preserveBacking: true)
                statusMessage = "Payment confirmed; syncing stability payment"
                return true
            }
        }

        guard let paymentIdString else { return false }
        let isRecordedStabilityPayment: Bool
        if let databaseService {
            isRecordedStabilityPayment =
                (try? databaseService.stabilityRepo.isOutgoingStabilityPayment(paymentId: paymentIdString)) == true
        } else {
            isRecordedStabilityPayment = false
        }
        guard isRecordedStabilityPayment else { return false }

        try? databaseService?.paymentRepo.updatePaymentStatus(
            paymentId: paymentIdString,
            status: "completed",
            feeMsat: feePaidMsat
        )
        refreshBalances()
        updateStableBalances()
        saveChannelToDB(preserveBacking: true)
        statusMessage = "Payment confirmed"
        return true
    }

    // MARK: - Channel Closed

    func requestChannelClose(
        userChannelId: UserChannelId,
        counterpartyNodeId: PublicKey,
        fundingOutpointTxid: String,
        fundingOutpointVout: UInt32
    ) async throws {
        // Snapshot here: AppState globals could be wrong for rapid re-tap or multi-channel
        let balanceSats = stableChannel.stableReceiverBTC.sats
        let price = btcPrice > 0 ? btcPrice : stableChannel.latestPrice
        let balanceUSD: Double? = price > 0
            ? Double(balanceSats) / Double(Constants.satsInBTC) * price
            : nil
        let counterparty: String? = stableChannel.counterparty.isEmpty
            ? nil
            : stableChannel.counterparty

        try await nodeService.requestChannelClose(
            userChannelId: userChannelId,
            counterpartyNodeId: counterpartyNodeId,
            fundingOutpointTxid: fundingOutpointTxid,
            fundingOutpointVout: fundingOutpointVout,
            balanceSats: balanceSats,
            balanceUsd: balanceUSD,
            btcPrice: price > 0 ? price : nil,
            counterparty: counterparty
        )
    }

    private func closureReasonData(_ reason: ClosureReason?) -> [String: Any] {
        guard let reason else { return ["kind": "UNKNOWN"] }
        switch reason {
        case .counterpartyForceClosed(let peerMsg):
            return ["kind": "COUNTERPARTY_FORCE_CLOSED", "peer_msg": peerMsg]
        case .holderForceClosed(let broadcastedLatestTxn, let message):
            var d: [String: Any] = ["kind": "HOLDER_FORCE_CLOSED", "message": message]
            if let broadcastedLatestTxn { d["broadcasted_latest_txn"] = broadcastedLatestTxn }
            return d
        case .legacyCooperativeClosure:
            return ["kind": "LEGACY_COOPERATIVE_CLOSURE"]
        case .counterpartyInitiatedCooperativeClosure:
            return ["kind": "COUNTERPARTY_INITIATED_COOPERATIVE_CLOSURE"]
        case .locallyInitiatedCooperativeClosure:
            return ["kind": "LOCALLY_INITIATED_COOPERATIVE_CLOSURE"]
        case .commitmentTxConfirmed:
            return ["kind": "COMMITMENT_TX_CONFIRMED"]
        case .fundingTimedOut:
            return ["kind": "FUNDING_TIMED_OUT"]
        case .processingError(let err):
            return ["kind": "PROCESSING_ERROR", "err": err]
        case .disconnectedPeer:
            return ["kind": "DISCONNECTED_PEER"]
        case .outdatedChannelManager:
            return ["kind": "OUTDATED_CHANNEL_MANAGER"]
        case .counterpartyCoopClosedUnfundedChannel:
            return ["kind": "COUNTERPARTY_COOP_CLOSED_UNFUNDED_CHANNEL"]
        case .locallyCoopClosedUnfundedChannel:
            return ["kind": "LOCALLY_COOP_CLOSED_UNFUNDED_CHANNEL"]
        case .fundingBatchClosure:
            return ["kind": "FUNDING_BATCH_CLOSURE"]
        case .htlCsTimedOut(let paymentHash):
            var d: [String: Any] = ["kind": "HTLCS_TIMED_OUT"]
            if let paymentHash { d["payment_hash"] = paymentHash }
            return d
        case .peerFeerateTooLow(let peerFeerateSatPerKw, let requiredFeerateSatPerKw):
            return [
                "kind": "PEER_FEERATE_TOO_LOW",
                "peer_feerate_sat_per_kw": Int(peerFeerateSatPerKw),
                "required_feerate_sat_per_kw": Int(requiredFeerateSatPerKw)
            ]
        @unknown default:
            return ["kind": "OTHER"]
        }
    }

    private func handleChannelClosed(
        channelId: ChannelId,
        userChannelId: UserChannelId,
        counterpartyNodeId: PublicKey?,
        reason: ClosureReason?
    ) {
        let balanceSats = stableChannel.stableReceiverBTC.sats

        var data: [String: Any] = [
            "channel_id": "\(channelId)",
            "user_channel_id": "\(userChannelId)",
            "reason": closureReasonData(reason),
            "balance_sats": "\(balanceSats)"
        ]
        if let counterpartyNodeId {
            data["counterparty_node_id"] = "\(counterpartyNodeId)"
        }
        AuditService.log("CHANNEL_CLOSED", data: data)

        // Funding txid is NOT close txid; defer payments row to handleCloseTxidResolved

        // Clear stable state if this is our channel or no channels remain
        if stableChannel.userChannelId == userChannelId || nodeService.channels.isEmpty {
            try? databaseService?.channelRepo.deleteChannel(userChannelId: stableChannel.userChannelId)
            stableChannel.expectedUSD = .zero
            stableChannel.backingSats = 0
            stableChannel.nativeSats = 0
            stableChannel.nativeChannelBTC = .zero
            stableChannel.stableReceiverBTC = .zero
            stableChannel.stableReceiverUSD = .zero
            stableChannel.channelId = ""
            stableChannel.userChannelId = ""
            // Stale after close: would otherwise link to the old channel's funding tx.
            fundingTxid = nil
            UserDefaults(suiteName: Constants.appGroupIdentifier)?
                .removeObject(forKey: "funding_txid")
        }

        // refreshBalances() self-clears isChannelClosing when lightning balance hits 0
        refreshBalances()
        statusMessage = "Channel closed"

        // Cancel prior; new task only clears state if generation still matches
        let opId = "close-\(userChannelId)"
        txidResolutionService.startCloseTxidResolver(opId: opId)
    }

    private func handleCloseTxidResolved(opId: String, closingTxid: String) {
        let op = databaseService?.pendingOpRepo.fetchPendingOperation(opId: opId)
        let balanceSats = op?.balanceSats ?? 0
        let balanceUSD = op?.balanceUsd
        let price = op?.btcPrice ?? 0
        let counterparty = op?.counterparty

        transactionLinkService.setCloseTxid(closingTxid)

        do {
            try databaseService?.paymentRepo.recordPayment(
                paymentId: opId,
                paymentType: "channel_close",
                direction: "received",
                amountMsat: balanceSats * 1000,
                amountUSD: balanceUSD,
                btcPrice: price > 0 ? price : nil,
                counterparty: counterparty,
                status: "completed",
                txid: closingTxid
            )
        } catch {
            AuditService.log("CLOSE_TXID_PAYMENT_ROW_FAILED", data: [
                "op_id": opId,
                "closing_txid": closingTxid,
                "error": "\(error)"
            ])
        }

        AuditService.log("CLOSE_TXID_RESOLVED", data: [
            "op_id": opId,
            "closing_txid": closingTxid
        ])
    }

    private func handleOnchainReceiveResolved(resolutionId: Int64, txid: String) {
        if let db = databaseService,
           let row = db.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resolutionId) {
            if let wsPayment = db.paymentRepo.payment(txid: txid) {
                if wsPayment.amountMsat == UInt64(row.amountMsat) {
                    // WebSocket beat us to it, this fallback placeholder is a duplicate.
                    db.paymentRepo.deletePayment(paymentId: row.paymentId)
                } else {
                    // Mismatched amount! This placeholder belongs to a different deposit.
                    // Leave it intact to prevent erasing it from history.
                    AuditService.log("ONCHAIN_RECEIVE_RES_MISMATCHED_AMOUNT", data: [
                        "resolution_id": "\(resolutionId)",
                        "txid": txid,
                        "placeholder_msat": "\(row.amountMsat)",
                        "websocket_msat": "\(wsPayment.amountMsat)"
                    ])
                }
            } else {
                // No websocket row exists. Attach the txid to this placeholder.
                db.paymentRepo.updatePaymentTxid(paymentId: row.paymentId, txid: txid, status: "completed")
            }
        }
        if let db = databaseService,
           let latest = db.onchainRepo.fetchLatestResolvedOnchainTxid() {
            transactionLinkService.setReceiveTxid(latest)
        } else {
            transactionLinkService.setReceiveTxid(txid)
        }
        AuditService.log("ONCHAIN_RECEIVE_RESOLVED", data: [
            "resolution_id": "\(resolutionId)",
            "txid": txid
        ])
    }

    // MARK: - Splice Pending

    private func handleSplicePending(
        channelId: ChannelId,
        userChannelId: UserChannelId,
        newFundingTxo: OutPoint
    ) {
        fundingTxid = "\(newFundingTxo.txid)"
        isSweeping = true

        AuditService.log("SPLICE_PENDING", data: [
            "channel_id": "\(channelId)",
            "user_channel_id": "\(userChannelId)",
            "funding_txo": "\(newFundingTxo)"
        ])

        // Record/update splice payment
        let txidStr = "\(newFundingTxo.txid)"
        spliceTxid = txidStr

        if let splice = pendingSplice {
            pendingSplice = nil
            if splice.direction == "in" {
                // Auto-sweep splice_in was already recorded — update with txid
                try? databaseService?.spliceRepo.setPendingSpliceTxid(txidStr)
            } else {
                let price = stableChannel.latestPrice
                let amountMsat = splice.amountSats * 1000
                let amountUSD: Double? = price > 0 ? Double(splice.amountSats) / 100_000_000.0 * price : nil
                _ = try? databaseService?.paymentRepo.recordPayment(
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
        } else {
            // pendingSplice is in-memory and lost across relaunch. If this event
            // is a restart replay, the latest NULL-txid splice row is this
            // splice's initiation row — stamp it so ChannelReady can complete it
            // and the no-txid expiry can't mark it failed.
            try? databaseService?.spliceRepo.setPendingSpliceTxid(txidStr)
        }

        refreshBalances()
        updateStableBalances()
        statusMessage = "Splice pending"
        startSpliceConfirmationMonitor(txid: txidStr)
    }

    func beginSpliceOut(amountSats: UInt64, address: String) throws {
        guard !isSweeping else {
            throw NSError(
                domain: "",
                code: 0,
                userInfo: [NSLocalizedDescriptionKey: "A splice is already in progress — try again shortly"]
            )
        }
        isSweeping = true
        pendingSplice = PendingSplice(direction: "out", amountSats: amountSats, address: address)
        statusMessage = "Move pending..."
    }

    func cancelPendingSpliceStart() {
        guard spliceTxid == nil else { return }
        isSweeping = false
        pendingSplice = nil
        statusMessage = ""
    }

    private func startSpliceConfirmationMonitor(txid: String) {
        let normalizedTxid = txid.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !normalizedTxid.isEmpty else { return }
        if monitoredSpliceTxid == normalizedTxid, spliceConfirmationTask != nil {
            return
        }

        spliceConfirmationTask?.cancel()
        monitoredSpliceTxid = normalizedTxid
        spliceConfirmationTask = Task { [weak self] in
            while !Task.isCancelled {
                guard let self else { return }
                if await self.isTxConfirmed(normalizedTxid) {
                    self.completeConfirmedSplice(txid: normalizedTxid)
                    return
                }
                try? await Task.sleep(nanoseconds: 30_000_000_000)
            }
        }
    }

    private func resumePendingSpliceConfirmation() {
        guard let hasSplice = try? databaseService?.spliceRepo.hasPendingSplice(), hasSplice else { return }
        isSweeping = true
        spliceTxid = (try? databaseService?.spliceRepo.getPendingSpliceTxid()) ?? spliceTxid ?? fundingTxid
        if let txid = spliceTxid, !txid.isEmpty {
            startSpliceConfirmationMonitor(txid: txid)
        }
    }

    private func isTxConfirmed(_ txid: String) async -> Bool {
        var urls: [String] = []
        for url in [chainURL, Constants.primaryChainURL, Constants.fallbackChainURL] where !urls.contains(url) {
            urls.append(url)
        }
        for baseURL in urls {
            guard let url =
                URL(string: "\(baseURL.trimmingCharacters(in: CharacterSet(charactersIn: "/")))/tx/\(txid)/status")
            else {
                continue
            }
            do {
                let (data, response) = try await URLSession.shared.data(from: url)
                guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
                    continue
                }
                let json = try JSONSerialization.jsonObject(with: data) as? [String: Any]
                if json?["confirmed"] as? Bool == true {
                    return true
                }
            } catch {
                AuditService.log("SPLICE_CONFIRMATION_CHECK_FAILED", data: [
                    "txid": txid,
                    "error": error.localizedDescription
                ])
            }
        }
        return false
    }

    private func completeConfirmedSplice(txid: String) {
        let completed = databaseService?.spliceRepo.completeSplice(txid: txid) == true
        if completed {
            refreshBalances()
            updateStableBalances()

            let price = stableChannel.latestPrice
            if let usdDeducted = StabilityService.reconcileOutgoing(&stableChannel, price: price) {
                stableChannel.lastStabilityPayment = Int64(Date().timeIntervalSince1970)
                AuditService.log("SPLICE_OUT_STABLE_DEDUCTED", data: [
                    "usd_deducted": "\(usdDeducted)",
                    "new_expected_usd": "\(stableChannel.expectedUSD.amount)",
                    "btc_price": "\(price)"
                ])
            }
            saveChannelToDB()
        }

        isSweeping = false
        pendingSplice = nil
        sweepOnchainStart = 0
        if spliceTxid == txid {
            spliceTxid = nil
        }
        monitoredSpliceTxid = nil
        spliceConfirmationTask = nil
        statusMessage = "Move confirmed"

        AuditService.log("SPLICE_CONFIRMED", data: [
            "txid": txid,
            "completed_row": "\(completed)"
        ])
    }

    // MARK: - Stability Timer

    private func startStabilityTimer() {
        stabilityTimer?.cancel()
        heartbeatTimer?.cancel()

        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.set(Date().timeIntervalSince1970, forKey: "main_app_last_active")
        heartbeatTimer = Task {
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: 5_000_000_000)
                guard !Task.isCancelled else { break }
                shared?.set(Date().timeIntervalSince1970, forKey: "main_app_last_active")
            }
        }

        stabilityTimer = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(nanoseconds: Constants.stabilityCheckIntervalSecs * 1_000_000_000)
                guard !Task.isCancelled else { break }

                // ensureLSPConnected dispatches its own blocking work off-main internally.
                await ensureLSPConnected()

                await MainActor.run { [weak self] in
                    self?.recordCurrentPrice()
                    self?.refreshTradeUncertainty()
                    self?.runStabilityCheck()
                    self?.detectOnchainDeposit()
                }
            }
        }
    }

    private func refreshTradeUncertainty() {
        guard let databaseService,
              let changed = try? databaseService.channelRepo.markExpiredTradesUncertain(),
              changed > 0 else { return }
        if let restored = try? databaseService.channelRepo.unresolvedTradePayments() {
            pendingTradePayments = restored
        }
        statusMessage = "Trade result delayed; it will still be accepted when received"
        AuditService.log("TRADE_RESULT_UNCERTAIN", data: [
            "reason": "no_response",
            "count": "\(changed)"
        ])
    }

    func ensureLSPConnected() {
        guard let node = nodeService.node else { return }
        nodeService.refreshChannels()
        let allUsable = !nodeService.channels.isEmpty && nodeService.channels.allSatisfy(\.isUsable)
        guard !allUsable else { return }
        // node.connect() does a TCP + Noise XK handshake (3 RTTs). On bad networks this
        // can block for seconds — long enough for the iOS watchdog to kill the app.
        // Dispatch off the main actor so the UI stays responsive. .utility priority because
        // this is opportunistic plumbing — no user is actively waiting on it.
        let lspPubkey = activeLSP.pubkey
        let lspAddress = activeLSP.address
        Task.detached(priority: .utility) {
            try? node.connect(nodeId: lspPubkey, address: lspAddress, persist: true)
        }
    }

    private func runStabilityCheck() {
        guard reconcilePendingOutgoingStabilityPayment() else { return }

        let price = accountingBTCPrice
        guard price > 0 else {
            AuditService.log("STABILITY_PAYMENT_SKIPPED", data: [
                "reason": priceService.isQuarantined ? "quarantined_price" : "stale_price"
            ])
            return
        }

        refreshBalances()
        updateStableBalances()

        guard stableChannel.expectedUSD.amount > 0,
              !nodeService.channels.isEmpty else { return }

        // Do NOT recalculate backingSats here — it's set at trade time and stays fixed.
        // As price moves, the stability check detects drift and sends payments to rebalance.

        let result = StabilityService.checkStabilityAction(stableChannel, price: price)

        guard result.action == .pay else { return }

        // Cooldown check
        let now = Int64(Date().timeIntervalSince1970)
        guard now - stableChannel.lastStabilityPayment >= Int64(Constants.stabilityPaymentCooldownSecs) else { return }

        let amountMsat = USD(amount: abs(result.dollarsFromPar)).toMsats(price: price)
        guard amountMsat > 0 else { return }

        guard let databaseService else { return }

        // Chain-freshness gate (see #243): never pay on a stale chain tip, and check BEFORE
        // claiming so a deferral leaves no claimed-but-unsent marker. The next stability
        // tick retries once LDK's background sync catches up.
        let syncAge = nodeService.lightningSyncAgeSecs()
        guard let syncAge, syncAge <= Constants.stabilityMaxLightningSyncAgeSecs else {
            AuditService.log("STABILITY_PAYMENT_SKIPPED", data: [
                "reason": "stale_lightning_sync",
                "sync_age_secs": syncAge.map { "\($0)" } ?? "never"
            ])
            return
        }

        // Claim the durable send slot (cross-process atomic via BEGIN IMMEDIATE).
        // If the NSE — or a previous run — already holds it, abort this run.
        guard databaseService.stabilityRepo.claimPendingSend(amountMsat: amountMsat, price: price) else {
            AuditService.log("STABILITY_PAYMENT_SKIPPED", data: [
                "reason": "pending_send_already_claimed"
            ])
            return
        }

        // Send stability payment
        let paymentId: PaymentId
        do {
            // Tag with the STABLE_CHANNEL_TLV [0x01] marker so the LSP classifies
            // this as a settlement (operator GUI) and runs reconcile_incoming_stability
            // immediately, matching every other sender. See issue #161.
            paymentId = try nodeService.sendStabilityPayment(
                amountMsat: amountMsat,
                to: stableChannel.counterparty,
                tlvs: [CustomTlvRecord(typeNum: Constants.stableChannelTLVType, value: Data([1]))]
            )
        } catch NodeServiceError.staleLightningSync {
            // The wrapper's send-boundary gate fired (sync went stale after the precheck
            // above). Send never happened — release the claim and retry next tick.
            databaseService.stabilityRepo.clearPendingSend()
            AuditService.log("STABILITY_PAYMENT_SKIPPED", data: [
                "reason": "stale_lightning_sync"
            ])
            return
        } catch {
            databaseService.stabilityRepo.clearPendingSend()
            AuditService.log("STABILITY_PAYMENT_FAILED", data: [
                "error": error.localizedDescription
            ])
            return
        }

        let paymentIdString = "\(paymentId)"
        let guardSaved = databaseService.stabilityRepo.setPendingSendPaymentId(paymentIdString)
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        shared?.set(Date().timeIntervalSince1970, forKey: "nse_last_stability_sent")
        shared?.synchronize()
        guard guardSaved else {
            shared?.set(true, forKey: "pending_push_payment")
            AuditService.log("STABILITY_PAYMENT_PERSISTENCE_FAILED", data: [
                "error": "payment_sent_but_id_guard_update_failed"
            ])
            return
        }

        do {
            let persistence = try databaseService.paymentRepo.recordPaymentAndMaybeUpdateBacking(
                paymentId: paymentIdString,
                paymentType: "stability",
                direction: "sent",
                amountMsat: amountMsat,
                amountUSD: abs(result.dollarsFromPar),
                btcPrice: price,
                status: "pending",
                userChannelId: stableChannel.userChannelId,
                backingDeltaSats: -Int64(amountMsat / 1000)
            )
            guard let backing = persistence.backingSats else {
                throw DatabaseError.executeFailed("DB did not return backing after outgoing stability payment")
            }
            stableChannel.lastStabilityPayment = now
            stableChannel.paymentMade = true
            stableChannel.backingSats = backing
            saveChannelToDB(preserveBacking: true)
            databaseService.stabilityRepo.clearPendingSend()

            AuditService.log("STABILITY_PAYMENT_SENT", data: [
                "amount_msat": "\(amountMsat)",
                "dollars_from_par": "\(result.dollarsFromPar)",
                "percent_from_par": "\(result.percentFromPar)",
                "btc_price": "\(price)"
            ])
        } catch {
            // The send already succeeded. Keep the durable marker and block all later sends
            // until the payment row and backing delta can be committed together.
            stableChannel.lastStabilityPayment = now
            stableChannel.paymentMade = true
            shared?.set(true, forKey: "pending_push_payment")
            AuditService.log("STABILITY_PAYMENT_PERSISTENCE_FAILED", data: [
                "error": error.localizedDescription
            ])
        }
    }

    private func reconcilePendingOutgoingStabilityPayment(status: String = "pending") -> Bool {
        guard let databaseService else { return false }
        guard var pending = databaseService.stabilityRepo.loadPendingSend() else { return true }

        if pending.paymentId.isEmpty {
            // Process died mid-keysend. Resolve the marker against LDK's payment
            // store instead of blocking sends forever.
            guard let node = nodeService.node else {
                UserDefaults(suiteName: Constants.appGroupIdentifier)?
                    .set(true, forKey: "pending_push_payment")
                AuditService.log("STABILITY_PAYMENT_RECONCILE_BLOCKED", data: [
                    "error": "unresolved_send_marker"
                ])
                return false
            }
            let candidates = node.listPayments().filter { payment in
                guard payment.direction == .outbound,
                      payment.amountMsat == pending.amountMsat,
                      Int64(payment.latestUpdateTimestamp) >= pending.createdAt - 10,
                      case .spontaneous = payment.kind else { return false }
                return true
            }
            if let succeeded = candidates.first(where: {
                if case .succeeded = $0.status { return true } else { return false }
            }) {
                // Sats left the channel — adopt the id and replay the debit below
                // (recordPaymentAndMaybeUpdateBacking dedups on payment_id).
                let adoptedId = "\(succeeded.id)"
                databaseService.stabilityRepo.setPendingSendPaymentId(adoptedId)
                pending = PendingStabilitySend(
                    paymentId: adoptedId,
                    amountMsat: pending.amountMsat,
                    price: pending.price,
                    createdAt: pending.createdAt
                )
                AuditService.log("STABILITY_PAYMENT_MARKER_ADOPTED", data: [
                    "payment_id": adoptedId,
                    "amount_msat": "\(pending.amountMsat)"
                ])
            } else if candidates.contains(where: {
                if case .pending = $0.status { return true } else { return false }
            }) {
                // Still in flight — wait for a terminal state.
                return false
            } else if let failed = candidates.first(where: {
                if case .failed = $0.status { return true } else { return false }
            }) {
                // Send failed — no sats moved, no debit to record.
                databaseService.stabilityRepo.clearPendingSend()
                AuditService.log("STABILITY_PAYMENT_SEND_MARKER_CLEARED", data: [
                    "payment_id": "\(failed.id)",
                    "reason": "payment_failed"
                ])
                return true
            } else if Int64(Date().timeIntervalSince1970) - pending.createdAt > 120 {
                // No matching payment ever appeared — the send never left.
                databaseService.stabilityRepo.clearPendingSend()
                AuditService.log("STABILITY_PAYMENT_SEND_MARKER_CLEARED", data: [
                    "reason": "send_never_left",
                    "amount_msat": "\(pending.amountMsat)"
                ])
                return true
            } else {
                // Too young to declare dead — the payment store may not have caught up.
                return false
            }
        }

        guard !stableChannel.userChannelId.isEmpty else {
            return false
        }

        do {
            let amountUSD = pending.price > 0
                ? Double(pending.amountMsat) / 1000.0 / Double(Constants.satsInBTC) * pending.price
                : nil
            let persistence = try databaseService.paymentRepo.recordPaymentAndMaybeUpdateBacking(
                paymentId: pending.paymentId,
                paymentType: "stability",
                direction: "sent",
                amountMsat: pending.amountMsat,
                amountUSD: amountUSD,
                btcPrice: pending.price > 0 ? pending.price : nil,
                status: status,
                userChannelId: stableChannel.userChannelId,
                backingDeltaSats: -Int64(pending.amountMsat / 1000)
            )
            guard let backing = persistence.backingSats else {
                throw DatabaseError.executeFailed("DB did not return backing during outgoing reconciliation")
            }
            stableChannel.backingSats = backing
            saveChannelToDB(preserveBacking: true)
            databaseService.stabilityRepo.clearPendingSend()
            return true
        } catch {
            UserDefaults(suiteName: Constants.appGroupIdentifier)?
                .set(true, forKey: "pending_push_payment")
            AuditService.log("STABILITY_PAYMENT_RECONCILE_FAILED", data: [
                "error": error.localizedDescription
            ])
            return false
        }
    }

    // MARK: - On-Chain Deposit Detection

    private func detectOnchainDeposit() {
        // Use already-updated onchainBalanceSats — refreshBalances() was just called before this
        let currentOnchain = onchainBalanceSats

        // Skip detection while a channel close is in flight: LDK sweeps the
        // channel balance to the on-chain wallet, which makes onchainBalanceSats
        // jump and would otherwise be recorded as a phantom "Received on-chain"
        // row. The real close row is written by handleCloseTxidResolved.
        if isChannelClosing {
            prevOnchainSats = currentOnchain
            return
        }

        if currentOnchain > prevOnchainSats && !isSweeping && pendingSplice == nil {
            let depositSats = currentOnchain - prevOnchainSats
            // Ignore tiny fluctuations from fee estimation changes
            guard depositSats >= 1000 else {
                prevOnchainSats = currentOnchain
                return
            }

            // We detected a balance increase, clear the old txid link so we don't show a stale one
            transactionLinkService.clearReceiveTxid()

            let price = stableChannel.latestPrice > 0 ? stableChannel.latestPrice : btcPrice
            let amountUSD: Double? = price > 0 ? Double(depositSats) / 100_000_000.0 * price : nil

            // UUID -> unique depositId -> recordPayment dedup on retry.
            let depositId = "onchain_deposit_\(UUID().uuidString)"

            // Select recorder based on whether we know the current receive
            // address. Each strategy handles its own crash-safe ordering and
            // resolver launching. See DepositRecorder.swift for invariants.
            let address = transactionLinkService.onchainReceiveAddress
            let recorder: DepositRecorder = (address?.isEmpty == false)
                ? KnownAddressDepositRecorder(
                    databaseService: databaseService,
                    onLaunchResolver: { [weak self] resolutionId, addr in
                        self?.txidResolutionService.startOnchainTxidResolver(resolutionId: resolutionId, address: addr)
                    }
                )
                : UnknownAddressDepositRecorder(databaseService: databaseService)

            let deposit = DepositRecordInput(
                depositId: depositId,
                depositSats: Int64(depositSats),
                amountUSD: amountUSD,
                btcPrice: price > 0 ? price : nil
            )
            let recorded = recorder.record(deposit: deposit, address: address)
            if !recorded {
                prevOnchainSats = currentOnchain
                return
            }
            // NOTE: do NOT clear lastReceiveTxid here. The view should
            // show the most recent resolved txid, not be blanked during
            // the re-detection window. The resolver will update
            // lastReceiveTxid via handleOnchainReceiveResolved.

            AuditService.log("ONCHAIN_DEPOSIT_DETECTED", data: [
                "amount_sats": "\(depositSats)",
                "prev_onchain": "\(prevOnchainSats)",
                "new_onchain": "\(currentOnchain)",
                "address_known": "\(transactionLinkService.onchainReceiveAddress != nil)"
            ])
        }
        prevOnchainSats = currentOnchain
    }

    func handleWebSocketTransactionDetected(event: WebSocketEvent) {
        guard let db = databaseService else { return }

        switch event {
        case .trackedOutspend(let trackedTxid, let spendingTxid):
            guard isChannelClosing else { return }

            // A tracked funding txid was outspent: this is a channel close!
            txidResolutionService.mempoolWebSocketService?.untrackTx(trackedTxid)

            // Instantly resolve the close payment row
            if let op = db.pendingOpRepo.fetchPendingOperationByFundingTxid(trackedTxid) {
                handleCloseTxidResolved(opId: op.opId, closingTxid: spendingTxid)
            }

        case .removed(let target, let txid):
            guard !isChannelClosing, !isSweeping, pendingSplice == nil else { return }
            do {
                try db.paymentRepo.failPaymentByTxid(txid: txid)
                let currentOnchain = onchainBalanceSats
                if currentOnchain < prevOnchainSats {
                    prevOnchainSats = currentOnchain
                }
                AuditService.log("WEBSOCKET_RBF_FAILED_PAYMENT", data: ["txid": txid, "target": target])
            } catch {
                AuditService.log("WEBSOCKET_RBF_FAIL_FAILED", data: ["error": "\(error)", "txid": txid])
            }

        case .receive(let target, let txid, let amountSats):
            guard !isChannelClosing, !isSweeping, pendingSplice == nil else { return }

            // 1. If amountSats > 0 and address is known, record pending payment in SQLite instantly
            if amountSats >= 1000 {
                let price = stableChannel.latestPrice > 0 ? stableChannel.latestPrice : btcPrice
                let amountUSD: Double? = price > 0 ? Double(amountSats) / 100_000_000.0 * price : nil
                let paymentId = "onchain_receive_\(txid)"

                do {
                    let recorded = try db.paymentRepo.recordPayment(
                        paymentId: paymentId,
                        paymentType: "onchain",
                        direction: "received",
                        amountMsat: UInt64(amountSats * 1000),
                        amountUSD: amountUSD,
                        btcPrice: price > 0 ? price : nil,
                        counterparty: nil,
                        status: "pending",
                        txid: txid,
                        address: target
                    )
                    if recorded {
                        AuditService.log(
                            "WEBSOCKET_INSTANT_PAYMENT_RECORDED",
                            data: ["txid": txid, "sats": "\(amountSats)"]
                        )
                        paymentFlash.toggle()
                    }
                } catch {
                    AuditService.log("WEBSOCKET_RECORD_PAYMENT_FAILED", data: ["error": "\(error)"])
                }
            }
        }
    }

    // MARK: - Sweep to Channel

    /// Sweep all on-chain funds into the Lightning channel (user-initiated splice-in).
    func openChannelWithOnchainFunds() {
        guard !isOpeningChannel else { return }
        let spendable = nodeService.spendableOnchainSats()
        guard spendable > 10_000 else {
            statusMessage = "Not enough onchain funds"
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
                    pubkey: activeLSP.pubkey,
                    address: activeLSP.address,
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
        isSweeping = true
        statusMessage = "Fetching fee rate..."

        Task { @MainActor in
            guard let channel = nodeService.channels.first(where: { $0.isChannelReady }) else {
                statusMessage = "No ready channel"
                isSweeping = false
                return
            }
            guard let balances = nodeService.balances() else {
                statusMessage = "Could not read balances"
                isSweeping = false
                return
            }

            let feeRateSatVb = await feeRateService.currentRate()
            let spendable = balances.spendableOnchainBalanceSats
            guard spendable > 0 else {
                statusMessage = "Insufficient onchain balance"
                isSweeping = false
                return
            }
            let sweepAmount = spendable
            let price = btcPrice > 0 ? btcPrice : stableChannel.latestPrice
            let amountUSD: Double? = price > 0
                ? Double(sweepAmount) / Double(Constants.satsInBTC) * price
                : nil

            do {
                try nodeService.spliceInWithAll(
                    userChannelId: channel.userChannelId,
                    counterpartyNodeId: channel.counterpartyNodeId
                )
                sweepOnchainStart = balances.totalOnchainBalanceSats
                pendingSplice = PendingSplice(direction: "in", amountSats: sweepAmount, address: nil)
                statusMessage = "Moving all onchain funds to channel..."

                _ = try? databaseService?.paymentRepo.recordPayment(
                    paymentId: nil,
                    paymentType: "splice_in",
                    direction: "received",
                    amountMsat: sweepAmount * 1000,
                    amountUSD: amountUSD,
                    btcPrice: price > 0 ? price : nil,
                    counterparty: nil,
                    status: "pending"
                )

                AuditService.log("SWEEP_TO_CHANNEL", data: [
                    "amount_sats": "\(sweepAmount)",
                    "fee_rate_sat_vb": "\(feeRateSatVb)",
                    "mode": "splice_in_with_all"
                ])
            } catch {
                statusMessage = "Sweep failed: \(error.localizedDescription)"
                AuditService.log("SWEEP_FAILED", data: [
                    "error": error.localizedDescription
                ])
                isSweeping = false
            }
        }
    }

    /// Starts the node with the current chainURL, and automatically falls back to the
    /// secondary Esplora source (e.g. mempool.space) if primary startup encounters an
    /// Esplora fee-rate estimation failure or timeout.
    ///
    /// Two failover layers exist by design: `resolveChainURL()` probes connectivity once
    /// at launch and picks the starting provider; this helper covers failures that only
    /// surface inside `node.start()` itself (fee-rate estimation against the chosen host).
    /// Both record the working URL in the app group so the NSE starts from it too.
    ///
    /// On total failure the wallet-dir lock is released (a no-op when NodeService.start
    /// already released its own lease) so a broken app process never starves the NSE.
    private func startNodeWithFailover(mnemonic: String = "", allowCreate: Bool = false) async throws {
        do {
            try await startNodeOrFailover(mnemonic: mnemonic, allowCreate: allowCreate)
        } catch {
            if !nodeService.isRunning {
                NodeDirLock.shared.release()
            }
            throw error
        }
    }

    private func startNodeOrFailover(mnemonic: String, allowCreate: Bool) async throws {
        let initialURL = chainURL
        do {
            try await nodeService.start(
                network: .bitcoin,
                esploraURL: initialURL,
                mnemonic: mnemonic,
                lspConfig: activeLSP,
                allowCreate: allowCreate
            )
            publishWorkingChainURL(initialURL)
        } catch {
            guard error.isRetryableEsploraStartupError else {
                // Non-Esplora errors (database, storage, lock, entropy, already running)
                // must propagate immediately without retrying.
                throw error
            }

            let initialError = error
            AuditService.log("NODE_START_INITIAL_FAILED", data: [
                "esploraURL": initialURL,
                "error": initialError.localizedDescription
            ])

            let fallbackURL = (initialURL == Constants.primaryChainURL)
                ? Constants.fallbackChainURL
                : Constants.primaryChainURL

            do {
                try await nodeService.start(
                    network: .bitcoin,
                    esploraURL: fallbackURL,
                    mnemonic: mnemonic,
                    lspConfig: activeLSP,
                    allowCreate: allowCreate
                )
                self.chainURL = fallbackURL
                publishWorkingChainURL(fallbackURL)
                AuditService.log("NODE_START_FAILOVER_SUCCESS", data: [
                    "primary": initialURL,
                    "using": fallbackURL
                ])
            } catch let fallbackError {
                AuditService.log("NODE_START_FAILOVER_FAILED", data: [
                    "primary": initialURL,
                    "fallback": fallbackURL,
                    "primary_error": initialError.localizedDescription,
                    "fallback_error": fallbackError.localizedDescription
                ])
                // Preserve the original primary error when fallback also fails
                throw initialError
            }
        }
    }

    /// Record the Esplora URL a node actually started against, so the NSE begins with a
    /// known-working provider instead of paying a degraded primary's failure first.
    private func publishWorkingChainURL(_ url: String) {
        UserDefaults(suiteName: Constants.appGroupIdentifier)?
            .set(url, forKey: "esplora_chain_url")
    }

    /// Test Blockstream connectivity; fall back to mempool.space if unreachable.
    private func resolveChainURL() async -> String {
        guard let url = URL(string: "\(Constants.primaryChainURL)/blocks/tip/height") else {
            return Constants.fallbackChainURL
        }
        // This sits on the critical path before first paint, so it gets a short
        // budget of its own: URLSession.shared defaults to a 60s request timeout,
        // which would hold the launch screen for a minute on a slow primary.
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 2
        config.timeoutIntervalForResource = 3
        let session = URLSession(configuration: config)
        let startedAt = Date()
        do {
            let (_, response) = try await session.data(from: url)
            if let http = response as? HTTPURLResponse, http.statusCode == 200 {
                AuditService.log("CHAIN_SOURCE_RESOLVED", data: [
                    "using": Constants.primaryChainURL,
                    "ms": "\(Int(Date().timeIntervalSince(startedAt) * 1000))"
                ])
                return Constants.primaryChainURL
            }
        } catch {}
        AuditService.log("CHAIN_SOURCE_FALLBACK", data: [
            "primary": Constants.primaryChainURL,
            "using": Constants.fallbackChainURL,
            "ms": "\(Int(Date().timeIntervalSince(startedAt) * 1000))"
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
        hasReadyChannel = nodeService.channels.contains { $0.isChannelReady }
        spendableOnchainSats = balances.spendableOnchainBalanceSats

        // Clear closing flag once lightning balance fully resolves
        if isChannelClosing && lightning == 0 {
            isChannelClosing = false
        }

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

    private func currentChannelFundingTxidMatches(_ txid: String) -> Bool {
        nodeService.refreshChannels()
        return nodeService.channels.contains { channel in
            guard channel.isChannelReady, let fundingTxo = channel.fundingTxo else {
                return false
            }
            return txid == "\(fundingTxo.txid)"
        }
    }

    // MARK: - Persistence

    func saveChannelToDB(preserveBacking: Bool = false) {
        guard !stableChannel.userChannelId.isEmpty else { return }
        do {
            if preserveBacking {
                try databaseService?.channelRepo.saveChannelPreservingBacking(
                    channelId: stableChannel.channelId,
                    userChannelId: stableChannel.userChannelId,
                    expectedUSD: stableChannel.expectedUSD.amount,
                    nativeSats: stableChannel.nativeSats,
                    note: stableChannel.note,
                    receiverSats: stableChannel.stableReceiverBTC.sats,
                    latestPrice: stableChannel.latestPrice
                )
            } else {
                try databaseService?.channelRepo.saveChannel(
                    channelId: stableChannel.channelId,
                    userChannelId: stableChannel.userChannelId,
                    expectedUSD: stableChannel.expectedUSD.amount,
                    backingSats: stableChannel.backingSats,
                    nativeSats: stableChannel.nativeSats,
                    note: stableChannel.note,
                    receiverSats: stableChannel.stableReceiverBTC.sats,
                    latestPrice: stableChannel.latestPrice
                )
            }
        } catch {
            AuditService.log("DB_SAVE_CHANNEL_FAILED", data: ["error": error.localizedDescription])
        }
    }

    private func loadChannelFromDB() {
        guard let db = databaseService else { return }
        do {
            if let record = try db.channelRepo.loadChannel(userChannelId: stableChannel.userChannelId),
               !record.userChannelId.isEmpty {
                stableChannel.channelId = record.channelId
                stableChannel.userChannelId = record.userChannelId
                stableChannel.expectedUSD = USD(amount: record.expectedUSD)
                stableChannel.backingSats = record.backingSats
                stableChannel.nativeSats = record.nativeSats
                stableChannel.note = record.note

                // Restore the counterparty from the same channel identity used to load this record.
                // Readiness is intentionally irrelevant: pending channels still have an authoritative peer.
                if let matchingChannel = nodeService.channels.first(where: {
                    $0.userChannelId == record.userChannelId
                }) {
                    stableChannel.counterparty = matchingChannel.counterpartyNodeId
                }

                // Restore cached balances so UI shows immediately
                if record.receiverSats > 0 {
                    stableChannel.stableReceiverBTC = Bitcoin(sats: record.receiverSats)
                    stableChannel.stableReceiverUSD = USD.fromBitcoin(
                        stableChannel.stableReceiverBTC,
                        price: record.latestPrice
                    )
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
            try databaseService?.priceRepo.recordPrice(price, source: "median")
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
        if let oldest = try? db.priceRepo.getOldestPriceHistoryTimestamp(), oldest < thirtyDaysAgo {
            // Already have old enough data, just fill gaps from the newest record
            since = (try? db.priceRepo.getPriceHistory(hours: 1).last?.timestamp) ?? thirtyDaysAgo
        } else {
            since = thirtyDaysAgo
        }

        let candles = await priceService.fetchKrakenOHLC(since: since)
        guard !candles.isEmpty else { return }

        do {
            let count = try db.priceRepo.backfillHourlyPrices(candles)
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
            if let oldest = try db.priceRepo.getOldestDailyPriceDate() {
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
            let count = try db.priceRepo.bulkInsertDailyPrices(HistoricalPrices.seedPrices)
            print("[Chart] Seeded \(count) historical price records")
        } catch {
            print("[Chart] Failed to seed historical prices: \(error)")
        }
    }

    // MARK: - LSP Configuration

    /// Switch to a new LSP configuration via LSPService.
    @MainActor
    func switchLSP(to newConfig: LSPConfig) async -> Bool {
        await lspService.switchLSP(
            to: newConfig,
            nodeService: nodeService,
            chainURL: chainURL,
            onSuccess: { [weak self] in
                guard let self else { return }
                if self.stableChannel.channelId.isEmpty {
                    self.stableChannel.counterparty = newConfig.pubkey
                }
                self.refreshBalances()
            }
        )
    }
}
