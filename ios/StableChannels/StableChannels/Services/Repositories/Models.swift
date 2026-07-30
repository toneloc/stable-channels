import Foundation
import SQLite3

struct PaymentPersistenceResult {
    let isNewPayment: Bool
    let backingSats: UInt64?
}

enum DatabaseError: LocalizedError {
    case openFailed(String)
    case prepareFailed(String)
    case executeFailed(String)
    /// No channels row exists for the given user_channel_id — recoverable by
    /// recreating the row from in-memory state, unlike a plain execute failure.
    case missingChannelRow(String)

    var errorDescription: String? {
        switch self {
        case .openFailed(let msg): return "Database open failed: \(msg)"
        case .prepareFailed(let msg): return "SQL prepare failed: \(msg)"
        case .executeFailed(let msg): return "SQL execute failed: \(msg)"
        case .missingChannelRow(let ucid): return "No channel row for user_channel_id=\(ucid)"
        }
    }
}

enum SQLValue {
    case text(String)
    case integer(Int64)
    case real(Double)
    case null
}

extension [Any?] {
    func string(_ idx: Int, default def: String = "") -> String {
        guard idx < count else { return def }
        return self[idx] as? String ?? def
    }

    func optString(_ idx: Int) -> String? {
        guard idx < count else { return nil }
        return self[idx] as? String
    }

    func double(_ idx: Int, default def: Double = 0.0) -> Double {
        guard idx < count else { return def }
        return self[idx] as? Double ?? def
    }

    func optDouble(_ idx: Int) -> Double? {
        guard idx < count else { return nil }
        return self[idx] as? Double
    }

    func int64(_ idx: Int, default def: Int64 = 0) -> Int64 {
        guard idx < count else { return def }
        return self[idx] as? Int64 ?? def
    }

    func optInt64(_ idx: Int) -> Int64? {
        guard idx < count else { return nil }
        return self[idx] as? Int64
    }

    func uInt64(_ idx: Int, default def: UInt64 = 0) -> UInt64 {
        guard idx < count else { return def }
        return UInt64((self[idx] as? Int64) ?? Int64(def))
    }

    func optUInt64(_ idx: Int) -> UInt64? {
        guard idx < count else { return nil }
        return (self[idx] as? Int64).map { UInt64($0) }
    }

    func uInt32(_ idx: Int, default def: UInt32 = 0) -> UInt32 {
        guard idx < count else { return def }
        return UInt32((self[idx] as? Int64) ?? Int64(def))
    }

    func optUInt32(_ idx: Int) -> UInt32? {
        guard idx < count else { return nil }
        return (self[idx] as? Int64).map { UInt32($0) }
    }

    func bool(_ idx: Int, default def: Bool = false) -> Bool {
        guard idx < count else { return def }
        return (self[idx] as? Int64 ?? 0) != 0
    }
}

final class RawSQL {
    let getDB: () -> OpaquePointer?

    init(getDB: @escaping () -> OpaquePointer?) {
        self.getDB = getDB
    }

    private let SQLITE_TRANSIENT = unsafeBitCast(-1, to: sqlite3_destructor_type.self)

    func bindParams(_ stmt: OpaquePointer?, params: [SQLValue]) {
        for (i, param) in params.enumerated() {
            let idx = Int32(i + 1)
            switch param {
            case .text(let s):
                _ = s.withCString { cStr in
                    sqlite3_bind_text(stmt, idx, cStr, -1, SQLITE_TRANSIENT)
                }
            case .integer(let n): sqlite3_bind_int64(stmt, idx, n)
            case .real(let d): sqlite3_bind_double(stmt, idx, d)
            case .null: sqlite3_bind_null(stmt, idx)
            }
        }
    }

    func execute(_ sql: String, params: [SQLValue] = []) throws {
        guard let db = getDB() else {
            throw DatabaseError.executeFailed("Database handle is nil")
        }
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            throw DatabaseError.prepareFailed(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }

        bindParams(stmt, params: params)

        let result = sqlite3_step(stmt)
        guard result == SQLITE_DONE || result == SQLITE_ROW else {
            throw DatabaseError.executeFailed(String(cString: sqlite3_errmsg(db)))
        }
    }

    func query(_ sql: String, params: [SQLValue] = []) throws -> [[Any?]] {
        guard let db = getDB() else {
            throw DatabaseError.executeFailed("Database handle is nil")
        }
        var stmt: OpaquePointer?
        guard sqlite3_prepare_v2(db, sql, -1, &stmt, nil) == SQLITE_OK else {
            throw DatabaseError.prepareFailed(String(cString: sqlite3_errmsg(db)))
        }
        defer { sqlite3_finalize(stmt) }

        bindParams(stmt, params: params)

        var rows: [[Any?]] = []
        let colCount = sqlite3_column_count(stmt)
        while sqlite3_step(stmt) == SQLITE_ROW {
            var row: [Any?] = []
            for col in 0..<colCount {
                switch sqlite3_column_type(stmt, col) {
                case SQLITE_INTEGER: row.append(sqlite3_column_int64(stmt, col))
                case SQLITE_FLOAT: row.append(sqlite3_column_double(stmt, col))
                case SQLITE_TEXT: row.append(String(cString: sqlite3_column_text(stmt, col)))
                case SQLITE_NULL: row.append(nil)
                default: row.append(nil)
                }
            }
            rows.append(row)
        }
        return rows
    }

    var changes: Int32 {
        guard let db = getDB() else { return 0 }
        return sqlite3_changes(db)
    }

    var lastInsertRowId: Int64 {
        guard let db = getDB() else { return 0 }
        return Int64(sqlite3_last_insert_rowid(db))
    }
}

/// Durable marker row for an in-flight outgoing stability payment.
/// `paymentId` is empty between the claim and the keysend returning an id.
struct PendingStabilitySend: Equatable {
    let paymentId: String
    let amountMsat: UInt64
    let price: Double
    let createdAt: Int64
}

struct OnchainReceiveResolution: Hashable {
    let id: Int64
    let address: String
    let txid: String?
    let status: String
    let createdAt: Int64
    let resolvedAt: Int64?
}

struct PendingOnchainPayment: Hashable {
    let paymentId: String
    let amountMsat: Int64
    let createdAt: Int64
}

struct PendingOperation: Equatable {
    let opId: String
    let opType: String
    let fundingOutpointTxid: String?
    let fundingOutpointVout: UInt32?
    let closingTxid: String?
    let balanceSats: UInt64?
    let balanceUsd: Double?
    let btcPrice: Double?
    let counterparty: String?
    let status: String
    let createdAt: Int64
    let resolvedAt: Int64?
}
