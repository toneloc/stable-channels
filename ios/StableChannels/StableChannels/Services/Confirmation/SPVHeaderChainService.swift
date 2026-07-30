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

        let timestamp = block.timestamp ?? UInt32(Date().timeIntervalSince1970)

        do {
            // Idempotency guard: if we already stored this exact height+hash, skip.
            // This covers the normal WebSocket reconnection scenario.
            if try databaseService.headerExists(height: height, hash: hash) {
                logger.info("[SPV] Header #\(height) already known — skipping duplicate.")
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
                try databaseService.insertHeader(
                    height: height,
                    hash: hash,
                    prevHash: prevHash,
                    timestamp: timestamp
                )
                logger.info("[SPV] Header chain seeded at #\(height) (\(hash.prefix(8))…)")
            }

            // Prune entries older than one difficulty epoch (~2 weeks / 2016 blocks).
            try databaseService.pruneOldHeaders(currentHeight: height)

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
        if incomingPrevHash == tip.hash {
            // prevHash links directly to our stored tip — normal chain growth.
            try databaseService.insertHeader(
                height: incomingHeight,
                hash: incomingHash,
                prevHash: incomingPrevHash,
                timestamp: timestamp
            )
            logger.info("[SPV] Appended header #\(incomingHeight) (\(incomingHash.prefix(8))…)")
            AuditService.log("SPV_HEADER_ADDED", data: ["height": "\(incomingHeight)"])

        } else if incomingHeight > tip.height + 1 {
            // The app was backgrounded and we missed N intermediate blocks.
            // Store the new tip and trigger full revalidation of recently confirmed payments via Esplora.
            logger.info(
                "[SPV] Offline gap: tip=#\(tip.height), incoming=#\(incomingHeight). Refreshing Esplora & revalidating payments."
            )
            try databaseService.insertHeader(
                height: incomingHeight,
                hash: incomingHash,
                prevHash: incomingPrevHash,
                timestamp: timestamp
            )
            AuditService.log("SPV_GAP_SYNC", data: [
                "prevTip": "\(tip.height)",
                "newTip": "\(incomingHeight)"
            ])

            await confirmationPollingService.revalidateRecentPayments()

        } else {
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

            blockHeightService.setHeightSilently(incomingHeight)
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
        try databaseService.inTransaction {
            if let commonAncestorHeight = try databaseService.findCommonAncestorHeight(prevHash: incomingPrevHash) {
                // Found the fork point — remove orphaned headers and revert payment statuses.
                try databaseService.rollbackHeadersAbove(height: commonAncestorHeight)
                try databaseService.rollbackPaymentsConfirmedAfter(height: commonAncestorHeight)
                logger.info("[SPV] Reorg rolled back to common ancestor #\(commonAncestorHeight)")
                AuditService.log("SPV_REORG_ROLLED_BACK", data: ["commonAncestor": "\(commonAncestorHeight)"])
            } else {
                // Fork point predates our rolling window — clear above the incoming block's floor.
                let safeFloor = incomingHeight > 0 ? incomingHeight - 1 : 0
                logger.warning("[SPV] Common ancestor not in window. Clearing headers above #\(safeFloor).")
                try databaseService.rollbackHeadersAbove(height: safeFloor)
                try databaseService.rollbackPaymentsConfirmedAfter(height: safeFloor)
            }

            // Plant the new canonical tip on the clean chain.
            try databaseService.insertHeader(
                height: incomingHeight,
                hash: incomingHash,
                prevHash: incomingPrevHash,
                timestamp: timestamp
            )
        }
    }

    // MARK: - Height Update

    /// Advances the in-memory block height counter.
    /// `updateHeight()` fires `onHeightUpdated → pollOnce()` automatically;
    /// no additional poll call is needed here.
    private func advanceHeight(to height: UInt32) {
        if height > blockHeightService.currentHeight {
            blockHeightService.updateHeight(height)
        }
    }
}
