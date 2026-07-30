import Foundation
import SQLite3

final class ChannelRepository {
    private let rawSQL: RawSQL

    init(rawSQL: RawSQL) {
        self.rawSQL = rawSQL
    }

    func saveChannel(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        backingSats: UInt64,
        nativeSats: UInt64 = 0,
        note: String?,
        receiverSats: UInt64 = 0,
        latestPrice: Double = 0.0
    ) throws {
        let updateSQL = """
            UPDATE channels SET channel_id = ?, expected_usd = ?, stable_sats = ?,
                native_sats = ?, note = ?, receiver_sats = ?, latest_price = ?, updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ?
        """
        try rawSQL.execute(updateSQL, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(backingSats)),
            .integer(Int64(nativeSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)), .real(latestPrice),
            .text(userChannelId)
        ])

        if rawSQL.changes == 0 {
            let insertSQL = """
                INSERT INTO channels (channel_id, user_channel_id, expected_usd, stable_sats, native_sats, note, receiver_sats, latest_price)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(channel_id) DO UPDATE SET
                    user_channel_id = excluded.user_channel_id,
                    expected_usd = excluded.expected_usd,
                    stable_sats = excluded.stable_sats,
                    native_sats = excluded.native_sats,
                    note = excluded.note,
                    receiver_sats = excluded.receiver_sats,
                    latest_price = excluded.latest_price,
                    updated_at = strftime('%s', 'now')
            """
            try rawSQL.execute(insertSQL, params: [
                .text(channelId), .text(userChannelId), .real(expectedUSD),
                .integer(Int64(backingSats)), .integer(Int64(nativeSats)),
                note.map { .text($0) } ?? .null,
                .integer(Int64(receiverSats)), .real(latestPrice)
            ])
        }
    }

    /// Persist channel metadata without touching stable_sats.
    func saveChannelPreservingBacking(
        channelId: String,
        userChannelId: String,
        expectedUSD: Double,
        nativeSats: UInt64 = 0,
        note: String?,
        receiverSats: UInt64 = 0,
        latestPrice: Double = 0.0
    ) throws {
        let sql = """
            UPDATE channels SET channel_id = ?, expected_usd = ?, native_sats = ?, note = ?,
                receiver_sats = ?, latest_price = ?, updated_at = strftime('%s', 'now')
            WHERE user_channel_id = ?
        """
        try rawSQL.execute(sql, params: [
            .text(channelId), .real(expectedUSD), .integer(Int64(nativeSats)),
            note.map { .text($0) } ?? .null, .integer(Int64(receiverSats)),
            .real(latestPrice), .text(userChannelId)
        ])
        let changedRows = rawSQL.changes
        guard changedRows == 1 else {
            throw DatabaseError.executeFailed(
                "channel metadata UPDATE affected \(changedRows) rows for user_channel_id=\(userChannelId)"
            )
        }
    }

    func loadChannel(userChannelId: String? = nil) throws -> ChannelRecord? {
        let sql: String
        let params: [SQLValue]
        if let id = userChannelId, !id.isEmpty {
            sql = "SELECT channel_id, expected_usd, note, stable_sats, user_channel_id, receiver_sats, latest_price, native_sats FROM channels WHERE user_channel_id = ?"
            params = [.text(id)]
        } else {
            sql = """
                SELECT channel_id, expected_usd, note, stable_sats, user_channel_id,
                       receiver_sats, latest_price, native_sats
                FROM channels
                ORDER BY updated_at DESC, channel_id DESC
                LIMIT 1
            """
            params = []
        }
        let rows = try rawSQL.query(sql, params: params)
        guard let row = rows.first else { return nil }

        return ChannelRecord(
            channelId: row.string(0),
            userChannelId: row.string(4),
            expectedUSD: row.double(1),
            note: row.optString(2),
            backingSats: row.uInt64(3),
            nativeSats: row.uInt64(7),
            receiverSats: row.uInt64(5),
            latestPrice: row.double(6)
        )
    }

    func deleteChannel(userChannelId: String) throws {
        try rawSQL.execute("DELETE FROM channels WHERE user_channel_id = ?", params: [.text(userChannelId)])
    }

    func recordTrade(
        channelId: String,
        action: String,
        amountUSD: Double,
        amountBTC: Double,
        btcPrice: Double,
        feeUSD: Double,
        paymentId: String?,
        status: String
    ) throws -> Int64 {
        let sql = """
            INSERT INTO trades (channel_id, action, amount_usd, amount_btc, btc_price, fee_usd, payment_id, status)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """
        try rawSQL.execute(sql, params: [
            .text(channelId), .text(action), .real(amountUSD), .real(amountBTC),
            .real(btcPrice), .real(feeUSD),
            paymentId.map { .text($0) } ?? .null, .text(status)
        ])
        return rawSQL.lastInsertRowId
    }

    func getRecentTrades(limit: Int) throws -> [TradeRecord] {
        let sql = """
            SELECT id, channel_id, action, amount_usd, amount_btc, btc_price, fee_usd,
                   payment_id, status, created_at
            FROM trades ORDER BY id DESC LIMIT ?
        """
        let rows = try rawSQL.query(sql, params: [.integer(Int64(limit))])
        return rows.map { row in
            TradeRecord(
                id: row.int64(0),
                channelId: row.string(1),
                action: row.string(2),
                amountUSD: row.double(3),
                amountBTC: row.double(4),
                btcPrice: row.double(5),
                feeUSD: row.double(6),
                paymentId: row.optString(7),
                status: row.string(8),
                createdAt: row.int64(9)
            )
        }
    }
}
