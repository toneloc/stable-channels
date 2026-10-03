import Foundation
import SQLite3
import XCTest
@testable import StableChannels

final class SQLitePaymentDatabaseTests: XCTestCase {
    private var tempDBPath: String!

    override func setUp() {
        super.setUp()
        let filename = "test_sqlite_payment_db_\(UUID().uuidString).sqlite"
        tempDBPath = (NSTemporaryDirectory() as NSString).appendingPathComponent(filename)
    }

    override func tearDown() {
        if let path = tempDBPath, FileManager.default.fileExists(atPath: path) {
            try? FileManager.default.removeItem(atPath: path)
        }
        super.tearDown()
    }

    private func executeRawSQL(path: String, sql: String) throws {
        var db: OpaquePointer?
        guard sqlite3_open(path, &db) == SQLITE_OK else {
            throw NSError(
                domain: "SQLitePaymentDatabaseTests",
                code: 1,
                userInfo: [NSLocalizedDescriptionKey: "Failed to open sqlite"]
            )
        }
        defer { sqlite3_close(db) }
        var errMsg: UnsafeMutablePointer<CChar>?
        if sqlite3_exec(db, sql, nil, nil, &errMsg) != SQLITE_OK {
            let error = errMsg != nil ? String(cString: errMsg!) : "Unknown error"
            sqlite3_free(errMsg)
            throw NSError(domain: "SQLitePaymentDatabaseTests", code: 2, userInfo: [NSLocalizedDescriptionKey: error])
        }
    }

    private func queryColumnNames(path: String, table: String) throws -> Set<String> {
        var db: OpaquePointer?
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            throw NSError(
                domain: "SQLitePaymentDatabaseTests",
                code: 3,
                userInfo: [NSLocalizedDescriptionKey: "Failed to open sqlite"]
            )
        }
        defer { sqlite3_close(db) }

        var cols = Set<String>()
        var stmt: OpaquePointer?
        if sqlite3_prepare_v2(db, "PRAGMA table_info(\(table))", -1, &stmt, nil) == SQLITE_OK {
            while sqlite3_step(stmt) == SQLITE_ROW {
                if let namePtr = sqlite3_column_text(stmt, 1) {
                    cols.insert(String(cString: namePtr))
                }
            }
            sqlite3_finalize(stmt)
        }
        return cols
    }

    func testOpenDB_migratesPreMigrationSchemaFromExtensionPath() throws {
        // Pre-migration schema without is_placeholder or backing_applied
        let legacySchema = """
        CREATE TABLE payments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            payment_id TEXT UNIQUE,
            payment_type TEXT NOT NULL,
            direction TEXT NOT NULL,
            amount_msat INTEGER NOT NULL,
            amount_usd REAL,
            btc_price REAL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        CREATE TABLE channels (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            channel_id TEXT NOT NULL,
            user_channel_id TEXT NOT NULL UNIQUE,
            stable_sats INTEGER NOT NULL
        );
        """
        try executeRawSQL(path: tempDBPath, sql: legacySchema)

        let initialCols = try queryColumnNames(path: tempDBPath, table: "payments")
        XCTAssertFalse(initialCols.contains("is_placeholder"))
        XCTAssertFalse(initialCols.contains("backing_applied"))

        let db = SQLitePaymentDatabase(dbPath: tempDBPath)
        let result = db.recordPayment(
            paymentId: "migrated-payment-1",
            paymentType: "lightning",
            direction: "received",
            amountMsat: 50_000_000,
            amountUSD: 50.0,
            btcPrice: 100_000.0,
            backingDeltaSats: nil,
            userChannelId: nil,
            settlementId: nil
        )
        XCTAssertEqual(result, .inserted)

        let migratedCols = try queryColumnNames(path: tempDBPath, table: "payments")
        XCTAssertTrue(migratedCols.contains("is_placeholder"))
        XCTAssertTrue(migratedCols.contains("backing_applied"))
    }

    func testRecordPayment_healUpdateFailureRollsBackBackingCredit() throws {
        let setupSQL = """
        CREATE TABLE payments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            payment_id TEXT UNIQUE,
            payment_type TEXT NOT NULL,
            direction TEXT NOT NULL,
            amount_msat INTEGER NOT NULL,
            amount_usd REAL,
            btc_price REAL,
            status TEXT NOT NULL,
            is_placeholder INTEGER NOT NULL DEFAULT 0,
            backing_applied INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        CREATE TABLE channels (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            channel_id TEXT NOT NULL,
            user_channel_id TEXT NOT NULL UNIQUE,
            stable_sats INTEGER NOT NULL,
            updated_at INTEGER NOT NULL DEFAULT (strftime('%s', 'now'))
        );
        INSERT INTO channels (channel_id, user_channel_id, stable_sats) VALUES ('chan-1', 'ucid-1', 50000);
        INSERT INTO payments (payment_id, payment_type, direction, amount_msat, status, is_placeholder, backing_applied)
        VALUES ('heal-fail-1', 'lightning', 'sent', 0, 'completed', 1, 0);

        CREATE TRIGGER force_heal_update_failure BEFORE UPDATE OF payment_type ON payments
        BEGIN
            SELECT RAISE(ABORT, 'forced heal failure');
        END;
        """
        try executeRawSQL(path: tempDBPath, sql: setupSQL)

        let db = SQLitePaymentDatabase(dbPath: tempDBPath)
        let result = db.recordPayment(
            paymentId: "heal-fail-1",
            paymentType: "stability",
            direction: "sent",
            amountMsat: 10_000_000,
            amountUSD: 10.0,
            btcPrice: 100_000.0,
            backingDeltaSats: 5000,
            userChannelId: "ucid-1",
            settlementId: "sid-must-not-burn"
        )
        XCTAssertEqual(result, .failed)

        // Verify backing was rolled back to 50000
        var sqlite: OpaquePointer?
        guard sqlite3_open_v2(tempDBPath, &sqlite, SQLITE_OPEN_READONLY, nil) == SQLITE_OK else {
            XCTFail("Failed to open db for verification")
            return
        }
        defer { sqlite3_close(sqlite) }

        var stmt: OpaquePointer?
        if sqlite3_prepare_v2(
            sqlite,
            "SELECT stable_sats FROM channels WHERE user_channel_id = 'ucid-1'",
            -1,
            &stmt,
            nil
        ) == SQLITE_OK {
            XCTAssertEqual(sqlite3_step(stmt), SQLITE_ROW)
            XCTAssertEqual(sqlite3_column_int64(stmt, 0), 50000)
            sqlite3_finalize(stmt)
        } else {
            XCTFail("Failed to prepare query on channels")
        }

        // Verify settlement ID was NOT burned
        var seenStmt: OpaquePointer?
        if sqlite3_prepare_v2(
            sqlite,
            "SELECT 1 FROM seen_stability_settlements WHERE settlement_id = 'sid-must-not-burn'",
            -1,
            &seenStmt,
            nil
        ) == SQLITE_OK {
            XCTAssertEqual(sqlite3_step(seenStmt), SQLITE_DONE) // No rows found
            sqlite3_finalize(seenStmt)
        }
    }
}
