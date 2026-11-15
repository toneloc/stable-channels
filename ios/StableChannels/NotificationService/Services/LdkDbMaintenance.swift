import Foundation
import SQLite3

/// Raw-sqlite upkeep of the gossip rows in LDK's `ldk_node_data.sqlite`, shared by the app and
/// the notification extension. Callers hold the wallet-dir lock (NodeDirLock) with no node running.
struct LdkDbMaintenance {
    let dataDir: URL
    var log: (String) -> Void = { print($0) }

    var databaseURL: URL { dataDir.appendingPathComponent("ldk_node_data.sqlite") }
    var graphFileURL: URL { dataDir.appendingPathComponent("network_graph.bin") }
    var metricsFileURL: URL { dataDir.appendingPathComponent("node_metrics.bin") }

    /// Extract network_graph blob from SQLite to a file, then delete it from the DB.
    /// This shrinks the DB from ~8.7MB to ~30KB so the NSE can load it.
    func extractGossip() {
        let dbPath = databaseURL.path
        let gossipPath = graphFileURL
        let metricsPath = metricsFileURL

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
                log("Saved network_graph (\(blobSize) bytes) to file")
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
            log("Stripped gossip data from DB")
        }
    }

    /// Delete network_graph from SQLite if it's too small (empty graph with stale timestamp).
    /// Also deletes scorer data so routing starts fresh.
    func purgeEmptyGraph() {
        let dbPath = databaseURL.path
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
                log("network_graph in DB too small (\(size) bytes), purging for fresh RGS sync")
            }
        } else {
            // No network_graph row at all — check if node_metrics has a stale timestamp
            needsPurge = true
            log("No network_graph in DB, purging node_metrics for fresh RGS sync")
        }

        if needsPurge {
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'scorer'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'node_metrics'", nil, nil, nil)
        }
    }

    /// Restore network_graph and node_metrics from files back into SQLite.
    func restoreGossip() {
        let dbPath = databaseURL.path
        let gossipPath = graphFileURL
        let metricsPath = metricsFileURL

        guard FileManager.default.fileExists(atPath: gossipPath.path),
              let graphData = try? Data(contentsOf: gossipPath) else { return }

        // Skip restoring an empty/corrupt graph
        if graphData.count < 1024 {
            log("network_graph.bin too small (\(graphData.count) bytes), deleting for fresh sync")
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
        log("Restored network_graph (\(graphData.count) bytes) + node_metrics to DB")
    }

    /// Returns true when the strip ran and deleted node_metrics (resetting LDK's persisted
    /// Lightning-sync timestamp along with the graph and scorer).
    func stripGossip() -> Bool {
        var db: OpaquePointer?
        guard sqlite3_open(databaseURL.path, &db) == SQLITE_OK else { return false }
        defer { sqlite3_close(db) }

        var stmt: OpaquePointer?
        let sql = "SELECT LENGTH(value) FROM ldk_node_data WHERE key = 'network_graph'"
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { return false }
        let hasGraph: Bool
        if sqlite3_step(stmt) == SQLITE_ROW {
            let size = sqlite3_column_int64(stmt, 0)
            hasGraph = size > 100_000
        } else {
            hasGraph = false
        }
        sqlite3_finalize(stmt)

        if hasGraph {
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'network_graph'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'scorer'", nil, nil, nil)
            sqlite3_exec(db, "DELETE FROM ldk_node_data WHERE key = 'node_metrics'", nil, nil, nil)
            sqlite3_exec(db, "VACUUM", nil, nil, nil)
        }
        return hasGraph
    }
}
