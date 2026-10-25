import Foundation

/// SQL for durable splice operation identity and event receipts, shared by the app and the
/// notification extension (both write the same app-group database). Statements are plain SQL
/// so each writer runs them with its own SQLite wrapper.
enum SpliceEventRecorder {
    static let createTableStatements = [
        """
        CREATE TABLE IF NOT EXISTS splice_operations (
            payment_row_id INTEGER PRIMARY KEY,
            user_channel_id TEXT NOT NULL,
            channel_id TEXT NOT NULL,
            previous_funding_txid TEXT
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
        """
    ]

    /// Both statements bind: ?1 event key, ?2 kind, ?3 user channel id, ?4 channel id,
    /// ?5 funding txid, ?6 funding vout.
    ///
    /// Captures the operation once, at first receipt; a replay keeps its original capture and an
    /// unmatched event is never reassigned to an operation created later. Each tier must yield
    /// exactly one row: an ambiguous match captures nothing rather than guess.
    /// - exact: a row already carrying this txid (also recognizes terminal replays without
    ///   reviving them).
    /// - tracked: a NULL-txid operation on the same channel. LDK keeps the channel id across a
    ///   splice, so a ready event must also prove funding moved off the saved outpoint, or an
    ///   initial channel-ready replay would look like a splice.
    /// - legacy: rows created before operation identity was persisted, scoped to the saved channel.
    static let insertEventSQL = """
    WITH
    exact AS (
        SELECT p.id FROM payments p LEFT JOIN splice_operations s ON s.payment_row_id = p.id
        WHERE p.txid = ?5 AND p.payment_type IN ('splice_in', 'splice_out')
          AND (s.user_channel_id IS NULL OR s.user_channel_id = ?3)
        LIMIT 2
    ),
    tracked AS (
        SELECT p.id FROM payments p JOIN splice_operations s ON s.payment_row_id = p.id
        WHERE p.status IN ('pending', 'expired') AND p.txid IS NULL
          AND s.user_channel_id = ?3 AND s.channel_id = ?4
          AND CASE WHEN ?2 = 'ready'
              THEN s.previous_funding_txid IS NOT NULL AND (?5 IS NULL OR s.previous_funding_txid != ?5)
              ELSE s.previous_funding_txid IS NULL OR s.previous_funding_txid != ?5 END
        LIMIT 2
    ),
    legacy AS (
        SELECT p.id FROM payments p
        WHERE ?2 = 'negotiated' AND p.payment_type IN ('splice_in', 'splice_out')
          AND p.status IN ('pending', 'expired') AND p.txid IS NULL
          AND NOT EXISTS (SELECT 1 FROM splice_operations s WHERE s.payment_row_id = p.id)
          AND EXISTS (SELECT 1 FROM channels c WHERE c.user_channel_id = ?3 AND c.channel_id = ?4)
        LIMIT 2
    )
    INSERT OR IGNORE INTO splice_events (event_key, event_type, user_channel_id, channel_id, txid, vout, payment_row_id)
    SELECT ?1, ?2, ?3, ?4, ?5, ?6, CASE
        WHEN EXISTS (SELECT 1 FROM exact) THEN (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM exact)
        WHEN EXISTS (SELECT 1 FROM tracked) THEN (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM tracked)
        ELSE (SELECT CASE WHEN COUNT(*) = 1 THEN MAX(id) END FROM legacy)
    END
    """

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
    /// operation is waiting for, they describe the same transaction and are dropped so the
    /// assignment that follows can stamp the splice row. Any other holder keeps the txid.
    static let reclaimSelfSendDepositsSQL = """
    WITH
    \(capturedCandidatesCTE),
    reclaimable AS MATERIALIZED (
        SELECT c.txid FROM candidates c
        WHERE (SELECT COUNT(*) FROM candidates d WHERE d.txid = c.txid) = 1
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
        "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') LIMIT 1"

    /// A pending splice still waiting for its negotiated txid. Splice rows are created with a
    /// NULL txid and only ever stamped with an event's txid, so NULL is the one "no txid" form.
    static let hasPendingSpliceWithoutTxidSQL =
        "SELECT 1 FROM payments WHERE status = 'pending' AND payment_type IN ('splice_in', 'splice_out') AND txid IS NULL LIMIT 1"
}
