import SQLite3
import XCTest
@testable import StableChannels

/// Gossip upkeep of `ldk_node_data.sqlite`, run on temp-dir fixtures with LDK's table layout.
final class LdkDbMaintenanceTests: XCTestCase {
    private var dataDir: URL!
    private var maintenance: LdkDbMaintenance!

    private let bigGraph = Data(repeating: 7, count: 600_000)
    private let metrics = Data([1, 2, 3, 4])
    private let scorer = Data([9, 9])
    private let monitor = Data([5, 5, 5])

    override func setUpWithError() throws {
        dataDir = FileManager.default.temporaryDirectory.appendingPathComponent("LdkDb-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        maintenance = LdkDbMaintenance(dataDir: dataDir, log: { _ in })
        try exec("""
        CREATE TABLE ldk_node_data (
            primary_namespace TEXT NOT NULL, secondary_namespace TEXT DEFAULT '' NOT NULL,
            key TEXT NOT NULL CHECK (key <> ''), value BLOB,
            PRIMARY KEY (primary_namespace, secondary_namespace, key))
        """)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: dataDir)
    }

    // MARK: - Extract

    func testExtract_movesGraphAndMetricsIntoFilesAndDeletesGossipRows() throws {
        try seed(graph: bigGraph)

        try maintenance.extractGossip()

        XCTAssertEqual(try Data(contentsOf: maintenance.graphFileURL), bigGraph)
        XCTAssertEqual(try Data(contentsOf: maintenance.metricsFileURL), metrics)
        XCTAssertEqual(try keys(), ["monitor"])
    }

    /// An old graph file from an earlier run is not a copy of what is in the DB now.
    func testExtract_oldGraphFileButTinyGraphInDB_keepsRows() throws {
        try bigGraph.write(to: maintenance.graphFileURL)
        try seed(graph: Data(repeating: 1, count: 1024))

        try maintenance.extractGossip()

        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    func testExtract_oldGraphFileButNoGraphInDB_keepsRows() throws {
        try bigGraph.write(to: maintenance.graphFileURL)
        try seed(graph: nil)

        try maintenance.extractGossip()

        XCTAssertEqual(try keys(), ["monitor", "node_metrics", "scorer"])
    }

    func testExtract_tinyGraph_writesNothingAndKeepsRows() throws {
        try seed(graph: Data(repeating: 1, count: 1024))

        try maintenance.extractGossip()

        XCTAssertFalse(FileManager.default.fileExists(atPath: maintenance.graphFileURL.path))
        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    // MARK: - Purge

    func testPurge_smallGraph_deletesGraphScorerAndMetrics() throws {
        try seed(graph: Data(repeating: 1, count: 499_999))

        try maintenance.purgeEmptyGraph()

        XCTAssertEqual(try keys(), ["monitor"])
    }

    func testPurge_realGraph_keepsEverything() throws {
        try seed(graph: Data(repeating: 1, count: 500_000))

        try maintenance.purgeEmptyGraph()

        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    func testPurge_noGraph_deletesScorerAndMetrics() throws {
        try seed(graph: nil)

        try maintenance.purgeEmptyGraph()

        XCTAssertEqual(try keys(), ["monitor"])
    }

    // MARK: - Restore

    func testRestore_putsFilesBackIntoTheDatabaseAndDeletesThem() throws {
        try seed(graph: nil, metricsAndScorer: false)
        try bigGraph.write(to: maintenance.graphFileURL)
        try metrics.write(to: maintenance.metricsFileURL)

        try maintenance.restoreGossip()

        XCTAssertEqual(try value("network_graph"), bigGraph)
        XCTAssertEqual(try value("node_metrics"), metrics)
        XCTAssertFalse(FileManager.default.fileExists(atPath: maintenance.graphFileURL.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: maintenance.metricsFileURL.path))
    }

    func testRestore_tinyGraphFile_deletesFilesAndLeavesTheDatabase() throws {
        try seed(graph: nil, metricsAndScorer: false)
        try Data(repeating: 1, count: 1023).write(to: maintenance.graphFileURL)
        try metrics.write(to: maintenance.metricsFileURL)

        try maintenance.restoreGossip()

        XCTAssertEqual(try keys(), ["monitor"])
        XCTAssertFalse(FileManager.default.fileExists(atPath: maintenance.graphFileURL.path))
        XCTAssertFalse(FileManager.default.fileExists(atPath: maintenance.metricsFileURL.path))
    }

    func testExtractThenRestore_roundTripsTheGossip() throws {
        try seed(graph: bigGraph)

        try maintenance.extractGossip()
        try maintenance.restoreGossip()

        XCTAssertEqual(try value("network_graph"), bigGraph)
        XCTAssertEqual(try value("node_metrics"), metrics)
        XCTAssertNil(try value("scorer"), "The scorer is rebuilt, not kept")
    }

    // MARK: - Strip

    func testStrip_largeGraph_deletesGossipRowsAndReportsTheMetricsReset() throws {
        try seed(graph: Data(repeating: 1, count: 100_001))

        XCTAssertTrue(try maintenance.stripGossip())

        XCTAssertEqual(try keys(), ["monitor"])
    }

    func testStrip_smallGraph_keepsRows() throws {
        try seed(graph: Data(repeating: 1, count: 100_000))

        XCTAssertFalse(try maintenance.stripGossip())

        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    func testStrip_missingDatabase_returnsTheErrorWithoutCreatingIt() throws {
        try FileManager.default.removeItem(at: maintenance.databaseURL)

        XCTAssertThrowsError(try maintenance.stripGossip())

        XCTAssertFalse(FileManager.default.fileExists(atPath: maintenance.databaseURL.path))
    }

    func testMissingDatabase_errorCarriesTheSQLiteResultCode() throws {
        try FileManager.default.removeItem(at: maintenance.databaseURL)

        XCTAssertThrowsError(try maintenance.stripGossip()) { error in
            XCTAssertEqual((error as? LdkDbMaintenance.SQLiteError)?.code, SQLITE_CANTOPEN)
        }
    }

    /// Once the delete commits the rows are gone, so the reset must be reported even if VACUUM fails.
    func testStrip_failedVacuum_stillReportsTheMetricsReset() throws {
        try seed(graph: bigGraph)
        var reader: OpaquePointer?
        var messages: [String] = []
        defer { sqlite3_close(reader) }
        maintenance.log = { [databaseURL = maintenance.databaseURL] message in
            messages.append(message)
            // A reader holding its lock after the delete makes VACUUM fail with SQLITE_BUSY.
            guard reader == nil else { return }
            sqlite3_open(databaseURL.path, &reader)
            sqlite3_exec(reader, "BEGIN; SELECT COUNT(*) FROM ldk_node_data;", nil, nil, nil)
        }

        XCTAssertTrue(try maintenance.stripGossip())

        XCTAssertNotNil(reader, "The delete must log before VACUUM runs")
        XCTAssertTrue(messages.contains { $0.hasPrefix("VACUUM after gossip strip failed") }, "\(messages)")
        XCTAssertEqual(try keys(), ["monitor"])
    }

    /// The app and the extension can touch the DB together; a short write lock is waited out, not failed on.
    func testStrip_waitsOutABriefWriteLock() throws {
        try seed(graph: bigGraph)
        var writer: OpaquePointer?
        XCTAssertEqual(sqlite3_open(maintenance.databaseURL.path, &writer), SQLITE_OK)
        XCTAssertEqual(sqlite3_exec(writer, "BEGIN IMMEDIATE", nil, nil, nil), SQLITE_OK)
        let released = expectation(description: "lock released")
        DispatchQueue.global().asyncAfter(deadline: .now() + 0.3) { [writer] in
            sqlite3_exec(writer, "COMMIT", nil, nil, nil)
            sqlite3_close(writer)
            released.fulfill()
        }

        XCTAssertTrue(try maintenance.stripGossip())

        wait(for: [released], timeout: 5)
        XCTAssertEqual(try keys(), ["monitor"])
    }

    // MARK: - Crash safety

    /// A graph that cannot be saved must stay in the DB, or the gossip is gone.
    func testExtract_unwritableGraphFile_keepsTheGossipRows() throws {
        try seed(graph: bigGraph)
        try FileManager.default.createDirectory(at: maintenance.graphFileURL, withIntermediateDirectories: false)

        XCTAssertThrowsError(try maintenance.extractGossip())

        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    /// A kill mid-write must never leave a truncated graph file for the next restore.
    func testExtract_replacesTheGraphFileAtomically() throws {
        try seed(graph: bigGraph)
        try Data([0]).write(to: maintenance.graphFileURL)
        let staleInode = try inode(maintenance.graphFileURL)

        try maintenance.extractGossip()

        XCTAssertNotEqual(try inode(maintenance.graphFileURL), staleInode, "Written in place, not swapped in")
        XCTAssertEqual(try Data(contentsOf: maintenance.graphFileURL), bigGraph)
    }

    /// A delete that fails part-way leaves every gossip row, never metrics without their graph.
    func testExtract_failedDelete_rollsBackEveryGossipRow() throws {
        try seed(graph: bigGraph)
        try failDeletes(of: "scorer")

        XCTAssertThrowsError(try maintenance.extractGossip())

        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    func testStrip_failedDelete_rollsBackEveryGossipRow() throws {
        try seed(graph: bigGraph)
        try failDeletes(of: "node_metrics")

        XCTAssertThrowsError(try maintenance.stripGossip())

        XCTAssertEqual(try keys(), ["monitor", "network_graph", "node_metrics", "scorer"])
    }

    /// Files are the only copy of the gossip until the DB has it back.
    func testRestore_failedInsert_keepsTheFiles() throws {
        try seed(graph: nil, metricsAndScorer: false)
        try bigGraph.write(to: maintenance.graphFileURL)
        try metrics.write(to: maintenance.metricsFileURL)
        try exec("CREATE TRIGGER no_insert BEFORE INSERT ON ldk_node_data BEGIN SELECT RAISE(ABORT, 'disk full'); END")

        XCTAssertThrowsError(try maintenance.restoreGossip())

        XCTAssertEqual(try Data(contentsOf: maintenance.graphFileURL), bigGraph)
        XCTAssertEqual(try Data(contentsOf: maintenance.metricsFileURL), metrics)
    }

    // MARK: - Fixtures

    private func seed(graph: Data?, metricsAndScorer: Bool = true) throws {
        try insert("monitor", monitor, namespace: "monitors")
        if let graph { try insert("network_graph", graph) }
        guard metricsAndScorer else { return }
        try insert("node_metrics", metrics)
        try insert("scorer", scorer)
    }

    private func withDB<T>(_ body: (OpaquePointer?) throws -> T) throws -> T {
        var db: OpaquePointer?
        guard sqlite3_open(maintenance.databaseURL.path, &db) == SQLITE_OK else { throw Fixture.sqlite }
        defer { sqlite3_close(db) }
        return try body(db)
    }

    private func exec(_ sql: String) throws {
        try withDB { db in
            guard sqlite3_exec(db, sql, nil, nil, nil) == SQLITE_OK else { throw Fixture.sqlite }
        }
    }

    private func insert(_ key: String, _ value: Data, namespace: String = "") throws {
        try withDB { db in
            var stmt: OpaquePointer?
            let sql = "INSERT INTO ldk_node_data (primary_namespace, secondary_namespace, key, value) VALUES (?, '', ?, ?)"
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { throw Fixture.sqlite }
            defer { sqlite3_finalize(stmt) }
            let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
            sqlite3_bind_text(stmt, 1, namespace, -1, transient)
            sqlite3_bind_text(stmt, 2, key, -1, transient)
            _ = value.withUnsafeBytes { sqlite3_bind_blob(stmt, 3, $0.baseAddress, Int32(value.count), transient) }
            guard sqlite3_step(stmt) == SQLITE_DONE else { throw Fixture.sqlite }
        }
    }

    private func keys() throws -> [String] {
        try withDB { db in
            var stmt: OpaquePointer?
            guard sqlite3_prepare_v2(db, "SELECT key FROM ldk_node_data ORDER BY key", -1, &stmt, nil) == SQLITE_OK
            else { throw Fixture.sqlite }
            defer { sqlite3_finalize(stmt) }
            var keys: [String] = []
            while sqlite3_step(stmt) == SQLITE_ROW {
                keys.append(String(cString: sqlite3_column_text(stmt, 0)))
            }
            return keys
        }
    }

    private func value(_ key: String) throws -> Data? {
        try withDB { db in
            var stmt: OpaquePointer?
            let sql = "SELECT value FROM ldk_node_data WHERE key = ?"
            guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else { throw Fixture.sqlite }
            defer { sqlite3_finalize(stmt) }
            sqlite3_bind_text(stmt, 1, key, -1, unsafeBitCast(-1, to: sqlite3_destructor_type.self))
            guard sqlite3_step(stmt) == SQLITE_ROW, let bytes = sqlite3_column_blob(stmt, 0) else { return nil }
            return Data(bytes: bytes, count: Int(sqlite3_column_bytes(stmt, 0)))
        }
    }

    private func failDeletes(of key: String) throws {
        try exec(
            "CREATE TRIGGER fail_delete BEFORE DELETE ON ldk_node_data WHEN OLD.key = '\(key)' BEGIN SELECT RAISE(ABORT, 'killed'); END"
        )
    }

    private func inode(_ url: URL) throws -> Int {
        try XCTUnwrap(FileManager.default.attributesOfItem(atPath: url.path)[.systemFileNumber] as? Int)
    }

    private enum Fixture: Error { case sqlite }
}
