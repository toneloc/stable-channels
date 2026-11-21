import Foundation
import SQLite3

/// Raw-sqlite upkeep of the gossip rows in LDK's `ldk_node_data.sqlite`, shared by the app and
/// the notification extension. Callers hold the wallet-dir lock (NodeDirLock) with no node running.
///
/// Every multi-row change runs in one `BEGIN IMMEDIATE ... COMMIT`, and files are written
/// atomically before the rows they copy are deleted, so a kill at any point loses no gossip.
struct LdkDbMaintenance {
    struct SQLiteError: Error, Equatable {
        /// The extended SQLite result code, e.g. SQLITE_BUSY or SQLITE_CANTOPEN.
        let code: Int32
        let message: String
    }

    /// A graph blob this small is empty or corrupt, so it is never saved or restored.
    static let minimumGraphBytes = 1024
    /// How long a statement waits for another connection (app or extension) to release its lock.
    static let busyTimeoutMilliseconds: Int32 = 2000

    let dataDir: URL
    var log: (String) -> Void = { print($0) }

    var databaseURL: URL { dataDir.appendingPathComponent("ldk_node_data.sqlite") }
    var graphFileURL: URL { dataDir.appendingPathComponent("network_graph.bin") }
    var metricsFileURL: URL { dataDir.appendingPathComponent("node_metrics.bin") }

    /// Moves the network graph and node metrics (RGS timestamp) out of the DB into files, so ~8.7MB of row
    /// data leaves the DB and the NSE can load it (the file itself only shrinks on VACUUM). Rows go only once
    /// this call has written the graph file.
    func extractGossip() throws {
        guard hasDatabase else { return }
        try withDatabase { db in
            var savedGraph = false
            if let graph = try blob(db, key: "network_graph"), graph.count > Self.minimumGraphBytes {
                try graph.write(to: graphFileURL, options: .atomic)
                savedGraph = true
                log("Saved network_graph (\(graph.count) bytes) to file")
            }
            if let metrics = try blob(db, key: "node_metrics"), !metrics.isEmpty {
                try metrics.write(to: metricsFileURL, options: .atomic)
            }
            // An older graph file is not a copy of these rows, so only a graph saved just now lets them go.
            guard savedGraph else { return }
            try deleteGossipRows(db)
            log("Stripped gossip data from DB")
        }
    }

    /// Deletes a graph too small to be real (empty graph with a stale RGS timestamp), with the
    /// scorer and metrics, so the next start syncs gossip fresh.
    func purgeEmptyGraph() throws {
        guard hasDatabase else { return }
        try withDatabase { db in
            if let size = try graphSize(db) {
                guard size < 500_000 else { return }
                log("network_graph in DB too small (\(size) bytes), purging for fresh RGS sync")
            } else {
                log("No network_graph in DB, purging node_metrics for fresh RGS sync")
            }
            try deleteGossipRows(db)
        }
    }

    /// Puts extracted gossip back into the DB, then deletes the files.
    func restoreGossip() throws {
        guard FileManager.default.fileExists(atPath: graphFileURL.path) else { return }
        let graph = try Data(contentsOf: graphFileURL)
        guard graph.count >= Self.minimumGraphBytes else {
            log("network_graph.bin too small (\(graph.count) bytes), deleting for fresh sync")
            try removeGossipFiles()
            return
        }
        let metricsExists = FileManager.default.fileExists(atPath: metricsFileURL.path)
        let metrics = metricsExists ? try Data(contentsOf: metricsFileURL) : Data()
        try withDatabase { db in
            try inTransaction(db) {
                try upsert(db, key: "network_graph", value: graph)
                // node_metrics holds the RGS timestamp, which must match the graph.
                if !metrics.isEmpty { try upsert(db, key: "node_metrics", value: metrics) }
            }
        }
        try removeGossipFiles()
        log("Restored network_graph (\(graph.count) bytes) + node_metrics to DB")
    }

    /// Deletes a large graph with the scorer and metrics, then vacuums, so the NSE's node loads
    /// a small DB. Returns true when it deleted them (LDK's persisted sync timestamp reset).
    func stripGossip() throws -> Bool {
        return try withDatabase { db in
            guard let size = try graphSize(db), size > 100_000 else { return false }
            try deleteGossipRows(db)
            log("Stripped gossip data from DB")
            // VACUUM cannot run inside a transaction. It only reclaims space; the rows are already gone,
            // so its failure must not hide the reset.
            do {
                try exec(db, "VACUUM")
            } catch {
                log("VACUUM after gossip strip failed: \(error)")
            }
            return true
        }
    }

    // MARK: - SQLite

    /// LDK creates its database on the first node start; before that there is no gossip to touch.
    private var hasDatabase: Bool { FileManager.default.fileExists(atPath: databaseURL.path) }

    private func withDatabase<T>(_ body: (OpaquePointer?) throws -> T) throws -> T {
        var db: OpaquePointer?
        defer { sqlite3_close(db) }
        // READWRITE without CREATE: a missing DB is an error, never an empty new file.
        guard sqlite3_open_v2(databaseURL.path, &db, SQLITE_OPEN_READWRITE, nil) == SQLITE_OK else { throw error(db) }
        sqlite3_busy_timeout(db, Self.busyTimeoutMilliseconds)
        return try body(db)
    }

    private func deleteGossipRows(_ db: OpaquePointer?) throws {
        try inTransaction(db) {
            for key in ["network_graph", "scorer", "node_metrics"] {
                try exec(db, "DELETE FROM ldk_node_data WHERE key = '\(key)'")
            }
        }
    }

    private func inTransaction(_ db: OpaquePointer?, _ body: () throws -> Void) throws {
        try exec(db, "BEGIN IMMEDIATE")
        do {
            try body()
            try exec(db, "COMMIT")
        } catch {
            // A failed rollback leaves the transaction open; sqlite3_close then rolls it back.
            sqlite3_exec(db, "ROLLBACK", nil, nil, nil)
            throw error
        }
    }

    private func graphSize(_ db: OpaquePointer?) throws -> Int64? {
        try query(db, "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'") { stmt in
            sqlite3_column_int64(stmt, 0)
        }
    }

    private func blob(_ db: OpaquePointer?, key: String) throws -> Data? {
        try query(db, "SELECT value FROM ldk_node_data WHERE key = '\(key)'") { stmt in
            // Blob before bytes, the order SQLite documents for reading a value.
            let bytes = sqlite3_column_blob(stmt, 0)
            let size = Int(sqlite3_column_bytes(stmt, 0))
            guard size > 0, let bytes else { return Data() }
            return Data(bytes: bytes, count: size)
        }
    }

    /// The first row mapped by `row`, or nil when there is none.
    private func query<T>(_ db: OpaquePointer?, _ sql: String, row: (OpaquePointer?) -> T) throws -> T? {
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { throw error(db) }
        defer { sqlite3_finalize(stmt) }
        switch sqlite3_step(stmt) {
        case SQLITE_ROW: return row(stmt)
        case SQLITE_DONE: return nil
        default: throw error(db)
        }
    }

    private func upsert(_ db: OpaquePointer?, key: String, value: Data) throws {
        let sql = """
        INSERT OR REPLACE INTO ldk_node_data (primary_namespace, secondary_namespace, key, value) \
        VALUES ('', '', '\(key)', ?)
        """
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { throw error(db) }
        defer { sqlite3_finalize(stmt) }
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        _ = value.withUnsafeBytes { sqlite3_bind_blob(stmt, 1, $0.baseAddress, Int32(value.count), transient) }
        guard sqlite3_step(stmt) == SQLITE_DONE else { throw error(db) }
    }

    private func exec(_ db: OpaquePointer?, _ sql: String) throws {
        guard sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK else { throw error(db) }
    }

    private func error(_ db: OpaquePointer?) -> SQLiteError {
        guard let db else { return SQLiteError(code: SQLITE_NOMEM, message: "out of memory") }
        return SQLiteError(code: sqlite3_extended_errcode(db), message: String(cString: sqlite3_errmsg(db)))
    }

    private func removeGossipFiles() throws {
        for url in [graphFileURL, metricsFileURL] where FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.removeItem(at: url)
        }
    }
}
