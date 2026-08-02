import Foundation

final class HeaderRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    /// Maximum number of block headers to retain in the local chain.
    /// 2016 = one Bitcoin difficulty epoch (~2 weeks): wide enough to cover any realistic
    /// reorg depth while keeping the table under ~160 KB on disk.
    static let headerRetentionDepth: UInt32 = 2016

    func insertHeader(height: UInt32, hash: String, prevHash: String, timestamp: UInt32) throws {
        let sql = """
        INSERT OR REPLACE INTO block_headers (height, hash, prev_hash, timestamp)
        VALUES (?, ?, ?, ?);
        """
        try rawSQL.execute(sql, params: [
            .integer(Int64(height)),
            .text(hash),
            .text(prevHash),
            .integer(Int64(timestamp))
        ])
    }

    /// Returns true if we already have a header for this height+hash (idempotent guard).
    func headerExists(height: UInt32, hash: String) throws -> Bool {
        let sql = "SELECT 1 FROM block_headers WHERE height = ? AND hash = ? LIMIT 1;"
        let rows = try rawSQL.query(sql, params: [.integer(Int64(height)), .text(hash)])
        return !rows.isEmpty
    }

    func fetchLatestHeader() throws -> BlockHeaderRecord? {
        let sql = "SELECT height, hash, prev_hash, timestamp FROM block_headers ORDER BY height DESC LIMIT 1;"
        let rows = try rawSQL.query(sql)
        guard let row = rows.first,
              let rawHeight = row[0] as? Int64,
              let hash = row[1] as? String,
              let prevHash = row[2] as? String,
              let rawTimestamp = row[3] as? Int64,
              let height = UInt32(exactly: rawHeight),
              let timestamp = UInt32(exactly: rawTimestamp) else {
            return nil
        }
        return BlockHeaderRecord(
            height: height,
            hash: hash,
            prevHash: prevHash,
            timestamp: timestamp
        )
    }

    func findCommonAncestorHeight(prevHash: String) throws -> UInt32? {
        let sql = "SELECT height FROM block_headers WHERE hash = ? LIMIT 1;"
        let rows = try rawSQL.query(sql, params: [.text(prevHash)])
        guard let row = rows.first,
              let rawHeight = row[0] as? Int64,
              let height = UInt32(exactly: rawHeight) else {
            return nil
        }
        return height
    }

    func rollbackHeadersAbove(height: UInt32) throws {
        let sql = "DELETE FROM block_headers WHERE height > ?;"
        try rawSQL.execute(sql, params: [.integer(Int64(height))])
    }

    func rollbackPaymentsConfirmedAfter(height: UInt32) throws {
        let sql = """
        UPDATE payments
        SET status = 'pending', confirmations = 0, tx_block_height = NULL
        WHERE tx_block_height IS NOT NULL AND tx_block_height > ?;
        """
        try rawSQL.execute(sql, params: [.integer(Int64(height))])
    }

    /// Rolling-window prune that keeps only the most recent `headerRetentionDepth` headers.
    /// Older entries are silently discarded — they predate any realistic reorg depth.
    func pruneOldHeaders(currentHeight: UInt32) throws {
        guard currentHeight >= Self.headerRetentionDepth else { return }
        let cutoff = currentHeight - Self.headerRetentionDepth
        let sql = "DELETE FROM block_headers WHERE height < ?;"
        try rawSQL.execute(sql, params: [.integer(Int64(cutoff))])
    }
}
