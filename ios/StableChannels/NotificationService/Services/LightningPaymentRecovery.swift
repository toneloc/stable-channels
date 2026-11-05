import Foundation

/// Terminal state of an outbound Lightning payment, mapped by the caller from an LDK event
/// or LDK's payment store. Mirrors Android's `LightningPaymentResolution`.
enum LightningPaymentResolution: Equatable {
    case succeeded(feeMsat: UInt64)
    case failed(reasonCode: String?)

    /// The payment row this resolution writes.
    var rowStatus: String {
        switch self {
        case .succeeded: return "completed"
        case .failed: return "failed"
        }
    }

    var rowFeeMsat: UInt64? {
        if case .succeeded(let feeMsat) = self { return feeMsat }
        return nil
    }
}

/// Which durable row a failed outbound payment resolves, in Android's
/// `PaymentFailureRecorder` priority order.
enum LightningPaymentFailureTarget: Equatable {
    /// The in-flight stability keysend failed: no sats moved, so only its marker clears.
    case stabilitySend
    /// A trade fee already correlated by its durable payment id.
    case trade
    /// A trade fee that failed before its id was attached, matched by amount. When no
    /// single prepared trade matches, the payment row is marked failed instead.
    case unattachedTrade(amountMsat: UInt64)
    /// An ordinary outbound payment row.
    case payment
}

/// Shared rules for the NSE event consumers and the foreground recovery sweep, so an LDK
/// payment event is never acknowledged before its outcome is in SQLite.
enum LightningPaymentRecovery {
    /// Bound on pending rows the foreground sweep resolves per run (Android default).
    static let reconcileLimit = 100

    static func failureTarget(
        paymentId: String,
        pendingStabilityPaymentId: String?,
        tradeExists: Bool,
        hasUnattachedPreparedTrade: Bool,
        lookupAmountMsat: () -> UInt64?
    ) -> LightningPaymentFailureTarget {
        if !paymentId.isEmpty, pendingStabilityPaymentId == paymentId { return .stabilitySend }
        // Signed results and already-paid fees are protected by the row update itself.
        if tradeExists { return .trade }
        if hasUnattachedPreparedTrade, let amountMsat = lookupAmountMsat() {
            return .unattachedTrade(amountMsat: amountMsat)
        }
        return .payment
    }

    /// Resolve pending outbound rows whose terminal event was acknowledged elsewhere.
    /// `lookup` returns nil while LDK still reports the payment pending (or unknown);
    /// `apply` returns false when the row update did not persist. Returns the repaired count.
    static func reconcilePending(
        paymentIds: [String],
        lookup: (String) -> LightningPaymentResolution?,
        apply: (String, LightningPaymentResolution) -> Bool
    ) -> Int {
        paymentIds.reduce(0) { repaired, paymentId in
            guard let resolution = lookup(paymentId), apply(paymentId, resolution) else { return repaired }
            return repaired + 1
        }
    }

    /// Stored `reason_code` for a failed send, matching Android's `PaymentFailureReason.name`
    /// (`routeNotFound` -> `ROUTE_NOT_FOUND`) so both platforms write the same vocabulary.
    static func reasonCode(caseName: String?) -> String? {
        guard let caseName, !caseName.isEmpty else { return nil }
        var code = ""
        for character in caseName {
            if character.isUppercase, !code.isEmpty { code.append("_") }
            code.append(contentsOf: character.uppercased())
        }
        return code
    }
}
