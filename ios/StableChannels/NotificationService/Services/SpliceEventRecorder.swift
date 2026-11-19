import Foundation

/// The payment types of a splice history row.
enum SplicePaymentType: String, CaseIterable {
    case spliceIn = "splice_in"
    case spliceOut = "splice_out"

    /// The payment row direction: a splice-out leaves the channel, a splice-in enters it.
    var direction: String {
        self == .spliceOut ? "sent" : "received"
    }
}

/// SQL for durable splice operation identity and event receipts, shared by the app and the
/// notification extension (both write the same app-group database). Statements are plain SQL
/// so each writer runs them with its own SQLite wrapper.
enum SpliceEventRecorder {
    /// Every splice payment type, as an SQL list for `payment_type IN ...`.
    static let spliceTypesSQL = "(" + SplicePaymentType.allCases.map { "'\($0.rawValue)'" }.joined(separator: ", ") + ")"

    static let createTableStatements = [
        """
        CREATE TABLE IF NOT EXISTS splice_operations (
            payment_row_id INTEGER PRIMARY KEY,
            user_channel_id TEXT NOT NULL,
            channel_id TEXT NOT NULL,
            previous_funding_txid TEXT,
            self_send INTEGER NOT NULL DEFAULT 0
        )
        """,
        """
        CREATE TABLE IF NOT EXISTS splice_events (
            event_key TEXT PRIMARY KEY,
            event_type TEXT NOT NULL,
            user_channel_id TEXT NOT NULL,
            channel_id TEXT NOT NULL,
            txid TEXT,
            vout INTEGER,
            payment_row_id INTEGER
        )
        """,
        // A splice the LSP made into the channel, kept apart from the user's history so it never
        // shows as something received: "negotiated" until the channel is ready on its funding,
        // then "locked". `value_before_sats` is the channel's value before it; `move_row_id` is the
        // one Move that was waiting for a transaction when it was negotiated, which the ready event
        // may yet show it was, if it is a splice-out (`settleSpliceOutStatements`).
        """
        CREATE TABLE IF NOT EXISTS lsp_splices (
            txid TEXT PRIMARY KEY,
            user_channel_id TEXT NOT NULL,
            channel_id TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            locked_at INTEGER,
            move_row_id INTEGER,
            value_before_sats INTEGER
        )
        """
    ]

    /// The lsp_splices columns, for `addLspSpliceColumnsSQL`.
    static let lspSpliceColumnsSQL = "SELECT name FROM pragma_table_info('lsp_splices')"

    /// Adds the lsp_splices columns a build before them created the table without.
    static func addLspSpliceColumnsSQL(existing: Set<String>) -> [String] {
        [("move_row_id", "INTEGER"), ("value_before_sats", "INTEGER")]
            .filter { !existing.contains($0.0) }
            .map { "ALTER TABLE lsp_splices ADD COLUMN \($0.0) \($0.1)" }
    }

    /// The event statements bind: ?1 event key, ?2 kind, ?3 user channel id, ?4 channel id,
    /// ?5 funding txid, ?6 funding vout; `insertEventSQL` and the LSP splice statements also bind
    /// ?7, 1 when the caller knows the splice is the LSP's, and the LSP splice statements ?8, the
    /// time in epoch seconds, and ?9, the channel's value as the event saw it
    /// (`SpliceEventReceipt.channelValueSats`).
    ///
    /// Captures the operation once, at first receipt; a replay keeps its original capture and an
    /// unmatched event is never reassigned to an operation created later. Each tier must yield
    /// exactly one row: an ambiguous match captures nothing rather than guess.
    /// - exact: a row already carrying this txid (also recognizes terminal replays without
    ///   reviving them).
    /// - tracked: a NULL-txid operation on the same channel. LDK keeps the channel id across a
    ///   splice, so a ready event must also prove funding moved off the saved outpoint, or an
    ///   initial channel-ready replay would look like a splice. A `pending` operation is preferred
    ///   over `expired` ones, so an overdue attempt left behind by a retry cannot make the retry
    ///   ambiguous.
    /// - legacy: rows created before operation identity was persisted, scoped to the saved channel.
    ///
    /// Only the exact tier runs for the LSP's splice: when the caller knows it is the LSP's (?7), or
    /// its funding is an LSP splice already recorded. A Move still waiting for its transaction is
    /// then not given the LSP's, while a replay of the user's own negotiation is still recognised.
    static let insertEventSQL = """
    WITH
    exact AS (
        SELECT p.id FROM payments p LEFT JOIN splice_operations s ON s.payment_row_id = p.id
        WHERE p.txid = ?5 AND p.payment_type IN \(spliceTypesSQL)
          AND (s.user_channel_id IS NULL OR s.user_channel_id = ?3)
        LIMIT 2
    ),
    \(trackedCTE),
    legacy AS (
        SELECT p.id FROM payments p
        WHERE ?2 = 'negotiated' AND p.payment_type IN \(spliceTypesSQL)
          AND p.status IN ('pending', 'expired') AND p.txid IS NULL
          AND NOT EXISTS (SELECT 1 FROM splice_operations s WHERE s.payment_row_id = p.id)
          AND EXISTS (SELECT 1 FROM channels c WHERE c.user_channel_id = ?3 AND c.channel_id = ?4)
        LIMIT 2
    )
    INSERT OR IGNORE INTO splice_events (event_key, event_type, user_channel_id, channel_id, txid, vout, payment_row_id)
    SELECT ?1, ?2, ?3, ?4, ?5, ?6, CASE
        WHEN EXISTS (SELECT 1 FROM exact) THEN (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM exact)
        WHEN ?7 = 1 OR EXISTS (SELECT 1 FROM lsp_splices l WHERE l.txid = ?5) THEN NULL
        WHEN EXISTS (SELECT 1 FROM tracked) THEN (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM preferred)
        ELSE (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM legacy)
    END
    """

    /// The tracked tier: NULL-txid operations on the event's channel, and the ones preferred among
    /// them (at most two, so one can be told from several).
    private static let trackedCTE = """
    tracked AS (
        SELECT p.id, p.status = 'pending' AS live
        FROM payments p JOIN splice_operations s ON s.payment_row_id = p.id
        WHERE p.status IN ('pending', 'expired') AND p.txid IS NULL
          AND s.user_channel_id = ?3 AND s.channel_id = ?4
          AND CASE WHEN ?2 = 'ready'
              THEN s.previous_funding_txid IS NOT NULL AND (?5 IS NULL OR s.previous_funding_txid != ?5)
              ELSE s.previous_funding_txid IS NULL OR s.previous_funding_txid != ?5 END
    ),
    preferred AS (
        SELECT id FROM tracked WHERE live = (SELECT MAX(live) FROM tracked) LIMIT 2
    )
    """

    /// Run on an event's first receipt only. Records a negotiation no operation claimed as the
    /// LSP's splice: when the caller knows it is the LSP's (?7), or no Move is waiting for a
    /// transaction at all, so a Move left ambiguous between two attempts is not taken for the LSP's.
    ///
    /// The negotiation names no size, so a Move negotiated while the LSP's splice is expected looks
    /// the same as the LSP's. The one operation the tracked tier would have matched is kept as
    /// `move_row_id` with the channel's value before (?9), for the ready event to settle by which
    /// way the value moved (`settledSpliceOutCTE`).
    static let recordLspSpliceSQL = """
    WITH
    \(trackedCTE)
    INSERT OR IGNORE INTO lsp_splices
        (txid, user_channel_id, channel_id, status, created_at, move_row_id, value_before_sats)
    SELECT ?5, ?3, ?4, 'negotiated', ?8, (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM preferred), ?9
    WHERE ?2 = 'negotiated' AND ?5 IS NOT NULL
      AND EXISTS (SELECT 1 FROM splice_events WHERE event_key = ?1 AND payment_row_id IS NULL)
      AND (?7 = 1 OR NOT EXISTS (
          SELECT 1 FROM payments
          WHERE status IN ('pending', 'expired') AND txid IS NULL AND payment_type IN \(spliceTypesSQL)
      ))
    """

    /// A splice recorded as the LSP's while a splice-out waited (`move_row_id`) that the ready event
    /// on its funding shows was that splice-out: the channel is now worth less (?9) than before
    /// it, which the LSP's splice-in never leaves, and the splice-out still waits for a transaction.
    /// A splice-in grows the channel as the LSP's does, so it is never settled this way.
    private static let settledSpliceOutCTE = """
    settled AS (
        SELECT l.txid, l.move_row_id FROM lsp_splices l JOIN payments p ON p.id = l.move_row_id
        WHERE ?2 = 'ready' AND l.txid = ?5 AND l.user_channel_id = ?3 AND l.channel_id = ?4
          AND l.status = 'negotiated' AND ?9 < l.value_before_sats
          AND p.payment_type = '\(SplicePaymentType.spliceOut.rawValue)'
          AND p.status IN ('pending', 'expired') AND p.txid IS NULL
    )
    """

    /// Run in order before a ready event is matched. When it settles a splice as the splice-out's
    /// (`settledSpliceOutCTE`), the negotiation is given to the splice-out and the LSP splice row is
    /// dropped, so the assignment that follows stamps the splice-out with its transaction. The
    /// second statement's change count says whether that happened.
    static let settleSpliceOutStatements = [
        """
        WITH
        \(settledSpliceOutCTE)
        UPDATE splice_events SET payment_row_id = (SELECT move_row_id FROM settled)
        WHERE event_type = 'negotiated' AND user_channel_id = ?3 AND channel_id = ?4 AND txid = ?5
          AND payment_row_id IS NULL AND EXISTS (SELECT 1 FROM settled)
        """,
        """
        WITH
        \(settledSpliceOutCTE)
        DELETE FROM lsp_splices WHERE txid IN (SELECT txid FROM settled)
        """
    ]

    /// The ready event on an LSP splice's funding locks it.
    static let lockLspSpliceSQL = """
    UPDATE lsp_splices SET status = 'locked', locked_at = ?8
    WHERE txid = ?5 AND status = 'negotiated' AND ?2 = 'ready'
    """

    static let lspSpliceStatusSQL = "SELECT status FROM lsp_splices WHERE txid = ?"

    /// A negotiated LSP splice not yet locked, newer than ?1 (epoch seconds): the channel is
    /// changing under the books, which must not be repaired against it meanwhile.
    static let hasPendingLspSpliceSQL =
        "SELECT 1 FROM lsp_splices WHERE status = 'negotiated' AND created_at > ? LIMIT 1"

    /// How long a negotiated LSP splice holds the books repair back.
    static let lspSplicePendingMaxAgeSecs: Int64 = 24 * 60 * 60

    /// Whether the Move `row` is the one a negotiated LSP splice named, first named by one newer
    /// than `since` (epoch seconds): its ready event has yet to say whether that splice was the
    /// Move's own splice-out, so until then the Move may still spend the channel's funding output,
    /// and no failure is its. The day counts from the first LSP splice that named the Move, so a
    /// later one (an RBF of it, or a new splice) cannot extend it.
    static func heldByLspSpliceSQL(_ row: String, since: String) -> String {
        """
        EXISTS (
            SELECT 1 FROM lsp_splices l
            WHERE l.move_row_id = \(row) AND l.status = 'negotiated'
              AND NOT EXISTS (
                  SELECT 1 FROM lsp_splices f WHERE f.move_row_id = \(row) AND f.created_at <= \(since)
              )
        )
        """
    }

    /// A replayed ready event may carry funding its first receipt lacked.
    static let attachFundingSQL = """
    UPDATE splice_events SET txid = ?5, vout = ?6
    WHERE event_key = ?1 AND txid IS NULL AND ?5 IS NOT NULL
    """

    /// Each captured, still-unassigned operation with the txid its latest event carries.
    private static let capturedCandidatesCTE = """
    candidates AS MATERIALIZED (
        SELECT e.payment_row_id AS row_id, e.txid AS txid, MAX(e.rowid)
        FROM splice_events e
        JOIN payments p ON p.id = e.payment_row_id
        LEFT JOIN splice_operations s ON s.payment_row_id = p.id
        WHERE p.status IN ('pending', 'expired') AND p.txid IS NULL AND e.txid IS NOT NULL
          AND (e.event_type != 'ready' OR s.previous_funding_txid != e.txid)
        GROUP BY e.payment_row_id
    )
    """

    /// Run in order, in one transaction: reclaim, then assign.
    static let assignmentStatements = [reclaimSelfSendDepositsSQL, recoverAssignmentsSQL]

    /// A splice-out paying our own address is first seen by the websocket receive handler and
    /// recorded as an onchain deposit (#316), which would block its txid from ever reaching the
    /// splice row. When such deposit rows are the only holders of a txid exactly one captured
    /// operation is waiting for, and that operation is a splice-out saved as paying our own
    /// address, they describe the same transaction and are dropped so the assignment that follows
    /// can stamp the splice row. Any other holder keeps the txid, as does a splice row saved without
    /// its operation (self-send unknown).
    static let reclaimSelfSendDepositsSQL = """
    WITH
    \(capturedCandidatesCTE),
    reclaimable AS MATERIALIZED (
        SELECT c.txid FROM candidates c
        WHERE (SELECT COUNT(*) FROM candidates d WHERE d.txid = c.txid) = 1
          AND EXISTS (
              SELECT 1 FROM payments sp JOIN splice_operations s ON s.payment_row_id = sp.id
              WHERE sp.id = c.row_id AND sp.payment_type = '\(SplicePaymentType.spliceOut.rawValue)'
                AND s.self_send = 1
          )
          AND NOT EXISTS (
              SELECT 1 FROM payments o WHERE o.txid = c.txid
                AND NOT (o.payment_type = 'onchain' AND o.direction = 'received')
          )
    )
    DELETE FROM payments
    WHERE payment_type = 'onchain' AND direction = 'received' AND txid IN (SELECT txid FROM reclaimable)
    """

    /// Stamps each captured operation with its event's txid (the most recent receipt wins).
    /// An `expired` row returns to `pending`: the event proved negotiation happened. Failed rows
    /// stay terminal, and a txid already on a row or wanted by two operations is never assigned.
    static let recoverAssignmentsSQL = """
    WITH
    \(capturedCandidatesCTE),
    assignable AS MATERIALIZED (
        SELECT c.row_id, c.txid FROM candidates c
        WHERE NOT EXISTS (SELECT 1 FROM payments o WHERE o.txid = c.txid)
          AND (SELECT COUNT(*) FROM candidates d WHERE d.txid = c.txid) = 1
    )
    UPDATE payments
    SET txid = (SELECT a.txid FROM assignable a WHERE a.row_id = payments.id), status = 'pending'
    WHERE id IN (SELECT row_id FROM assignable)
    """

    static let hasPendingSpliceSQL =
        "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN \(spliceTypesSQL) LIMIT 1"

    /// A pending splice still waiting for its negotiated txid. Splice rows are created with a
    /// NULL txid and only ever stamped with an event's txid, so NULL is the one "no txid" form.
    static let hasPendingSpliceWithoutTxidSQL =
        "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN \(spliceTypesSQL) AND txid IS NULL LIMIT 1"
}
