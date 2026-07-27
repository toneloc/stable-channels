import Foundation
import os.log

/// Manages SPV block header ingestion, local header persistence, and reorg detection/rollback.
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

    /// Process a new block header from WebSocket or polling source.
    func processBlockHeader(_ block: MempoolWSBlock) async {
        let height = block.height
        guard let hash = block.id, let prevHash = block.previousblockhash else {
            await updateHeightAndPoll(height: height)
            return
        }
        let timestamp = block.timestamp ?? UInt32(Date().timeIntervalSince1970)

        do {
            let latestHeader = try databaseService.fetchLatestHeader()

            if let tip = latestHeader {
                if prevHash == tip.hash {
                    // Sequential Tip Extension: Main chain growing normally
                    try databaseService.insertHeader(
                        height: height,
                        hash: hash,
                        prevHash: prevHash,
                        timestamp: timestamp
                    )
                    logger.info("[SPV] Added block header #\(height) (\(hash.prefix(8)))")
                    AuditService.log("SPV_HEADER_ADDED", data: ["height": "\(height)", "hash": hash])
                } else if hash != tip.hash {
                    // Reorg Detected! prevHash doesn't match tip hash
                    logger.warning("[SPV] Reorg detected at height #\(height)! Previous hash mismatch.")
                    AuditService.log(
                        "SPV_REORG_DETECTED",
                        data: ["height": "\(height)", "hash": hash, "prevHash": prevHash]
                    )

                    try handleReorg(
                        incomingHeight: height,
                        incomingHash: hash,
                        incomingPrevHash: prevHash,
                        timestamp: timestamp
                    )
                }
            } else {
                // First header initialization
                try databaseService.insertHeader(height: height, hash: hash, prevHash: prevHash, timestamp: timestamp)
                logger.info("[SPV] Initialized header chain at #\(height)")
            }
        } catch {
            logger.error("[SPV] Header processing error: \(error.localizedDescription)")
        }

        await updateHeightAndPoll(height: height)
    }

    private func handleReorg(
        incomingHeight: UInt32,
        incomingHash: String,
        incomingPrevHash: String,
        timestamp: UInt32
    ) throws {
        // Search backwards for common ancestor in stored header chain
        if let commonAncestorHeight = try databaseService.findCommonAncestorHeight(prevHash: incomingPrevHash) {
            logger.info("[SPV] Found common ancestor at height #\(commonAncestorHeight)")

            // Delete orphaned headers above common ancestor
            try databaseService.rollbackHeadersAbove(height: commonAncestorHeight)

            // Roll back any payments confirmed after common ancestor height to "pending"
            try databaseService.rollbackPaymentsConfirmedAfter(height: commonAncestorHeight)
            logger
                .info("[SPV] Rolled back orphaned block headers and confirmed payments above #\(commonAncestorHeight)")
            AuditService.log("SPV_REORG_ROLLED_BACK", data: ["commonAncestor": "\(commonAncestorHeight)"])
        } else {
            logger.warning("[SPV] Common ancestor not found in local headers. Clearing stale header tip.")
            try databaseService.rollbackHeadersAbove(height: incomingHeight > 0 ? incomingHeight - 1 : 0)
        }

        // Insert new block header into clean chain
        try databaseService.insertHeader(
            height: incomingHeight,
            hash: incomingHash,
            prevHash: incomingPrevHash,
            timestamp: timestamp
        )
    }

    private func updateHeightAndPoll(height: UInt32) async {
        if height > blockHeightService.currentHeight {
            blockHeightService.updateHeight(height)
        }
        await confirmationPollingService.pollOnce()
    }
}
