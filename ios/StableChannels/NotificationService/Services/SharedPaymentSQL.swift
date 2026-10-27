import Foundation

/// SQL both writers run against the shared database: the app's repositories and the
/// notification extension's raw sqlite3 layer. One text per rule keeps the two processes from
/// drifting; each side binds the `?` parameters in the order documented here.
enum SharedPaymentSQL {
    // MARK: - Payments

    /// ?1 status, ?2 fee msat, ?3 payment id. Only a pending row moves.
    static let resolvePendingPaymentWithFee =
        "UPDATE payments SET status = ?, fee_msat = ? WHERE payment_id = ? AND status = 'pending'"

    /// ?1 status, ?2 payment id. Only a pending row moves.
    static let resolvePendingPayment =
        "UPDATE payments SET status = ? WHERE payment_id = ? AND status = 'pending'"

    // MARK: - Trades

    /// Statuses a fee payment outcome may still move; signed results and paid fees are final.
    private static let unresolvedTradeStatuses = "('prepared','sent','uncertain')"
    /// Binds ?1 reason code.
    private static let sendFailedAssignments =
        "status = 'send_failed', outcome = 'send_failed', reason_code = ?, resolved_at = strftime('%s', 'now')"

    /// ?1 trade payment id. Fee settlement is not trade acceptance: the trade waits for its result.
    static let markTradeFeePaid = """
    UPDATE trades SET status = 'fee_paid'
    WHERE trade_payment_id = ? AND status IN \(unresolvedTradeStatuses)
    """

    /// ?1 reason code, ?2 trade payment id.
    static let markTradeSendFailedByPaymentId = """
    UPDATE trades SET \(sendFailedAssignments)
    WHERE trade_payment_id = ? AND status IN \(unresolvedTradeStatuses)
    """

    /// ?1 reason code, ?2 trade row id.
    static let markTradeSendFailedById = """
    UPDATE trades SET \(sendFailedAssignments)
    WHERE id = ? AND status IN \(unresolvedTradeStatuses)
    """

    /// A recent prepared trade whose fee payment id was never attached, matched by fee amount.
    /// Callers select their own columns; binds ?1 fee msat, ?2 created-at cutoff.
    static let unattachedPreparedTradeMatch = """
    FROM trades
    WHERE trade_payment_id IS NULL AND status = 'prepared' AND fee_msat = ? AND created_at >= ?
    ORDER BY id DESC LIMIT 2
    """

    /// ?1 payment id, ?2 trade payment id, ?3 trade row id. Fee settled, the trade awaits its result.
    static let adoptPaidPaymentForUnattachedTrade = """
    UPDATE trades SET payment_id = ?, trade_payment_id = ?, status = 'fee_paid'
    WHERE id = ? AND trade_payment_id IS NULL AND status = 'prepared'
    """

    /// ?1 payment id, ?2 trade payment id, ?3 reason code, ?4 trade row id.
    static let adoptFailedPaymentForUnattachedTrade = """
    UPDATE trades
    SET payment_id = ?, trade_payment_id = ?, \(sendFailedAssignments)
    WHERE id = ? AND trade_payment_id IS NULL AND status = 'prepared'
    """
}
