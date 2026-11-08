import Foundation
import os.log

/// Manages SPV block header ingestion, local header persistence, and reorg detection/rollback.
///
/// - Keeps a rolling 2016-block window (one difficulty epoch) to bound SQLite storage.
/// - Distinguishes a real **chain reorg** (same or lower height, different hash) from an
///   **offline catch-up gap** (height jumped by more than one — app was backgrounded).
/// - Idempotent insert guard prevents double-writes on WebSocket reconnection.
/// - `updateHeight()` fires `blockHeightService.onHeightUpdated → pollOnce()` automatically,
///   so `SPVHeaderChainService` does NOT trigger a second poll on the normal block path.
@MainActor
final class SPVHeaderChainService {
    private let databaseService: DatabaseService
    private let blockHeightService: BlockHeightService
    private let confirmationPollingService: ConfirmationPollingService
    private let logger = Logger(subsystem: "com.stablechannels", category: "spv")

    init(
        databaseService: DatabaseService,
        blockHeightService: BlockHeightService,
        confirmationPollingService: ConfirmationPollingService
    ) {
        self.databaseService = databaseService
        self.blockHeightService = blockHeightService
        self.confirmationPollingService = confirmationPollingService
    }

    // MARK: - Public Entry Point

    /// Called for every block event arriving from the Mempool WebSocket.
    func processBlockHeader(_ block: MempoolWSBlock) async {
        let height = block.height

        // If this block payload has no hash/prevHash (e.g. mempool-blocks payload), we
        // still advance the height counter — but skip header storage entirely.
        guard let hash = block.id, let prevHash = block.previousblockhash else {
            advanceHeight(to: height)
            return
        }

        // A stale verified tip (app was away) must not turn a real catch-up block away: refresh once first.
        if !HeaderChainPolicy.canStore(incomingHeight: height, verifiedTip: blockHeightService.currentHeight) {
            await blockHeightService.refresh()
        }
        guard HeaderChainPolicy.canStore(incomingHeight: height, verifiedTip: blockHeightService.currentHeight) else {
            logger.warning("[SPV] Header #\(height) is too far ahead of the verified tip. Not stored.")
            AuditService.log("SPV_HEADER_REJECTED", data: [
                "height": "\(height)",
                "verifiedTip": "\(blockHeightService.currentHeight)"
            ])
            advanceHeight(to: height)
            return
        }

        let timestamp = block.timestamp ?? UInt32(Date().timeIntervalSince1970)

        do {
            // Idempotency guard: if we already stored this exact height+hash, skip.
            // This covers the normal WebSocket reconnection scenario.
            guard try !databaseService.headerRepo.headerExists(height: height, hash: hash) else {
                logger.debug("[SPV] Header #\(height) already processed. Skipping.")
                advanceHeight(to: height)
                return
            }

            let tip = try databaseService.fetchLatestHeader()

            if let tip {
                try await processWithKnownTip(
                    tip: tip,
                    incomingHeight: height,
                    incomingHash: hash,
                    incomingPrevHash: prevHash,
                    timestamp: timestamp
                )
            } else {
                // Cold start: no headers stored yet — seed the chain at current tip.
                try databaseService.storeBlockHeader(
                    BlockHeaderRecord(
                        height: height,
                        hash: hash,
                        prevHash: prevHash,
                        timestamp: timestamp
                    )
                )
                logger.info("[SPV] Header chain seeded at #\(height) (\(hash.prefix(8))…)")
            }

            // Prune entries older than one difficulty epoch (~2 weeks / 2016 blocks).
            try databaseService.pruneHeadersOlderThan(height: height)

        } catch {
            logger.error("[SPV] Header processing error: \(error.localizedDescription)")
        }

        advanceHeight(to: height)
    }

    // MARK: - Private Chain Processing

    private func processWithKnownTip(
        tip: BlockHeaderRecord,
        incomingHeight: UInt32,
        incomingHash: String,
        incomingPrevHash: String,
        timestamp: UInt32
    ) async throws {
        let step = HeaderChainPolicy.step(
            tipHeight: tip.height,
            tipHash: tip.hash,
            incomingHeight: incomingHeight,
            incomingPrevHash: incomingPrevHash
        )
        switch step {
        case .append:
            // prevHash links directly to our stored tip — normal chain growth.
            try databaseService.storeBlockHeader(
                BlockHeaderRecord(
                    height: incomingHeight,
                    hash: incomingHash,
                    prevHash: incomingPrevHash,
                    timestamp: timestamp
                )
            )
            logger.info("[SPV] Appended header #\(incomingHeight) (\(incomingHash.prefix(8))…)")
            AuditService.log("SPV_HEADER_ADDED", data: ["height": "\(incomingHeight)"])

        case .catchUp:
            // The app was backgrounded and we missed N intermediate blocks.
            // Store the new tip and trigger full revalidation of recently confirmed payments via Esplora.
            logger.info(
                "[SPV] Offline gap: tip=#\(tip.height), incoming=#\(incomingHeight). Refreshing Esplora & revalidating payments."
            )
            try databaseService.storeBlockHeader(
                BlockHeaderRecord(
                    height: incomingHeight,
                    hash: incomingHash,
                    prevHash: incomingPrevHash,
                    timestamp: timestamp
                )
            )
            AuditService.log("SPV_GAP_SYNC", data: [
                "prevTip": "\(tip.height)",
                "newTip": "\(incomingHeight)"
            ])

            await confirmationPollingService.revalidateRecentPayments()

        case .reorg:
            // prevHash mismatch at the same or lower height — a competing block won the
            // chain race. Walk back to the common ancestor and roll back affected payments.
            logger
                .warning(
                    "[SPV] Reorg detected at #\(incomingHeight)! Expected prevHash=\(tip.hash.prefix(8))… got \(incomingPrevHash.prefix(8))…"
                )
            AuditService.log("SPV_REORG_DETECTED", data: [
                "height": "\(incomingHeight)",
                "hash": incomingHash,
                "prevHash": incomingPrevHash
            ])

            try handleReorg(
                incomingHeight: incomingHeight,
                incomingHash: incomingHash,
                incomingPrevHash: incomingPrevHash,
                timestamp: timestamp
            )

            // The incoming height is unverified: keep the HTTP-verified tip; `advanceHeight` refreshes it.
            await confirmationPollingService.revalidateRecentPayments()
        }
    }

    private func handleReorg(
        incomingHeight: UInt32,
        incomingHash: String,
        incomingPrevHash: String,
        timestamp: UInt32
    ) throws {
        // Wrap header rollback + payment rollback + tip insert in one transaction.
        // If any step fails, all three are rolled back together.
        try databaseService.inTransaction(mode: "IMMEDIATE") {
            if let commonAncestorHeight = try databaseService.headerRepo
                .findCommonAncestorHeight(prevHash: incomingPrevHash) {
                // Found the fork point — remove orphaned headers and revert payment statuses.
                try databaseService.rollbackHeadersAbove(height: commonAncestorHeight)
                try databaseService.headerRepo.rollbackPaymentsConfirmedAfter(height: commonAncestorHeight)
                logger.info("[SPV] Reorg rolled back to common ancestor #\(commonAncestorHeight)")
                AuditService.log("SPV_REORG_ROLLED_BACK", data: ["commonAncestor": "\(commonAncestorHeight)"])
            } else {
                // Fork point predates our rolling window — clear above the incoming block's floor.
                let safeFloor = incomingHeight > 0 ? incomingHeight - 1 : 0
                logger.warning("[SPV] Common ancestor not in window. Clearing headers above #\(safeFloor).")
                try databaseService.rollbackHeadersAbove(height: safeFloor)
                try databaseService.headerRepo.rollbackPaymentsConfirmedAfter(height: safeFloor)
            }

            // Plant the new canonical tip on the clean chain.
            try databaseService.storeBlockHeader(
                BlockHeaderRecord(
                    height: incomingHeight,
                    hash: incomingHash,
                    prevHash: incomingPrevHash,
                    timestamp: timestamp
                )
            )
        }
    }

    // MARK: - Height Update

    /// Triggers an authoritative Esplora HTTP refresh when a WebSocket block event arrives.
    /// `refresh()` verifies the tip height via HTTP and updates `blockHeightService.currentHeight`,
    /// preventing unverified/inflated WebSocket values from driving fake confirmation counts.
    private func advanceHeight(to _: UInt32) {
        Task {
            await blockHeightService.refresh()
        }
    }
}
