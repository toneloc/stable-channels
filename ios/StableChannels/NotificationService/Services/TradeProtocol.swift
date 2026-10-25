import CryptoKit
import CoreFoundation
import Foundation
import Security

// This file belongs to BOTH the app and notification extension targets.

/// The one sats/USD conversion. Both targets use it; their `Constants.satsInBTC` derive from it.
enum BitcoinUnits {
    static let satsPerBTC: Double = 100_000_000

    static func usd(sats: some BinaryInteger, price: Double) -> Double {
        Double(sats) / satsPerBTC * price
    }

    static func usd(msats: some BinaryInteger, price: Double) -> Double {
        Double(msats) / 1000 / satsPerBTC * price
    }

    /// Unrounded: callers pick the rounding their rule needs.
    static func sats(usd: Double, price: Double) -> Double {
        usd / price * satsPerBTC
    }

    /// The cumulative floor every stable target uses, or nil when it is not a representable
    /// non-negative sats amount (invalid price, overflow).
    static func floorTargetSats(usd: Double, price: Double) -> UInt64? {
        let target = sats(usd: usd, price: price)
        guard target.isFinite, target >= 0, target < Double(UInt64.max) else { return nil }
        return UInt64(target.rounded(.down))
    }
}

// Trade-entry only:
// never enforce this policy in accepted-result reconciliation or stability settlements.
enum StabilizationPolicy {
    static let maxStableAllocationPercent: UInt64 = 99
    static let clientSafetyMarginSats: UInt64 = 50

    static func backingCap(_ postFeeSpendable: UInt64) -> UInt64? {
        return (postFeeSpendable / 100) * maxStableAllocationPercent
            + ((postFeeSpendable % 100) * maxStableAllocationPercent) / 100
    }

    static func clientLimit(_ postFeeSpendable: UInt64) -> UInt64? {
        guard let cap = backingCap(postFeeSpendable), cap > clientSafetyMarginSats else { return nil }
        return cap - clientSafetyMarginSats
    }

    static func maximumMessage(_ cents: UInt64) -> String {
        String(
            format: String(localized: "maximum_additional_trade", defaultValue: "Maximum additional trade: $%.2f"),
            Double(cents) / 100
        )
    }

    static func limitExceededMessage(_ cents: UInt64) -> String {
        let explanation = String(localized: "stabilization_reserve_explanation",
                                 defaultValue: "Keeps a small BTC reserve in the channel.")
        return maximumMessage(cents) + "\n" + explanation
    }
}

/// Settlement-side allocation rules, shared by the app and notification extension so every
/// incoming-stability write path credits backing the same way (#320). Mirrors Android's
/// `StabilityService.backingAfterIncomingStability`.
enum StabilityAllocation {
    /// Use Rust's local shortfall rule: excess receipts stay native and existing surplus stays
    /// backing. Apply the credit to the committed books, bounded by the actual received sats. Do not
    /// cap against a live balance snapshot: it can already include a later withdrawal whose event
    /// has not been accounted for. Nil on invalid books, so the caller rolls back and retries.
    static func backingAfterIncomingStability(
        currentBackingSats: Int64,
        expectedUSD: Double,
        price: Double,
        amountSats: Int64
    ) -> Int64? {
        guard currentBackingSats >= 0, expectedUSD.isFinite, expectedUSD >= 0,
              price.isFinite, price > 0, amountSats > 0 else { return nil }
        guard let equilibrium = BitcoinUnits.floorTargetSats(usd: expectedUSD, price: price),
              equilibrium <= UInt64(Int64.max) else { return nil }
        let targetSats = Int64(equilibrium)
        if currentBackingSats >= targetSats { return currentBackingSats }
        // Cap the credit before adding so even an excessive amount cannot overflow.
        return currentBackingSats + min(amountSats, targetSats - currentBackingSats)
    }

    /// The backing a payment row writes: incoming stability credits only the local shortfall;
    /// every other delta already moved sats, so it keeps the existing floor-at-zero debit clamp.
    /// Nil when an incoming-stability credit cannot be computed (row must be retried).
    static func backingAfterPayment(
        currentBackingSats: Int64,
        expectedUSD: Double,
        price: Double?,
        deltaSats: Int64,
        paymentType: String,
        direction: String
    ) -> Int64? {
        guard paymentType == "stability", direction == "received" else {
            return max(0, currentBackingSats + deltaSats)
        }
        return backingAfterIncomingStability(
            currentBackingSats: currentBackingSats,
            expectedUSD: expectedUSD,
            price: price ?? 0,
            amountSats: deltaSats
        )
    }
}

/// Why a trade could not be prepared or executed on this device. Distinct from a signed
/// TRADE_REJECTED_V1 reason code: these are decided locally, before anything is signed or
/// sent, so no fee has been spent and there is nothing to reconcile with the LSP.
///
/// Every local refusal must map to one of these. Collapsing them into a bare `nil` is what
/// let a failed local check surface as one generic order-failure message (issue #272).
/// The taxonomy and copy mirror Android's `TradeFailure`.
enum TradeValidationError: LocalizedError, Equatable {
    case invalidAmount
    case invalidPrice
    case exceedsStableBalance
    case channelNotReady
    case feeUnavailable
    case feeExceedsBalance
    case allocationUnavailable
    case stabilizationLimit(UInt64)
    /// LDK may fail to persist its payment row after the fee keysend started. The prepared
    /// row stays recoverable and blocks the next order until the outcome is known.
    case sendOutcomeUnknown

    /// Fixed local copy. Says what happened and, where the user can act, what to do about it.
    var errorDescription: String? {
        switch self {
        case .invalidAmount: return "Enter a valid amount and try again."
        case .invalidPrice: return "A fresh BTC/USD consensus is required before trading."
        case .exceedsStableBalance: return "That is more than your stabilized balance. Reduce the amount."
        case .channelNotReady: return "This channel is not ready to trade yet."
        case .feeUnavailable: return "The trade fee could not be calculated. Refresh the price and try again."
        case .feeExceedsBalance: return "Your balance cannot cover this trade and its fee. Reduce the amount."
        case .allocationUnavailable: return "That is more than this channel can convert right now. Reduce the amount."
        case .stabilizationLimit(let cents): return StabilizationPolicy.limitExceededMessage(cents)
        case .sendOutcomeUnknown:
            return "The trade may have been sent, but its status could not be saved. "
                + "Keep the wallet connected and check History. Do not place the order again."
        }
    }
}

struct StabilizationSnapshot {
    let receiverSats: UInt64
    let spendableSats: UInt64
    let backingSats: UInt64
    let expectedUSD: Double
    let price: Double

    /// Receiver sats not backing the stable position.
    private var nativeSats: UInt64 {
        receiverSats >= backingSats ? receiverSats - backingSats : 0
    }

    func accepts(_ orderCents: UInt64) -> Bool {
        fits(orderCents, searching: false)
    }

    private func fits(_ orderCents: UInt64, searching: Bool) -> Bool {
        guard orderCents > 0, price.isFinite, price > 0, expectedUSD.isFinite, expectedUSD >= 0 else { return false }
        let amount = Double(orderCents) / 100
        let required = (BitcoinUnits.sats(usd: amount, price: price)).rounded(.up)
        let native = nativeSats
        guard required.isFinite, required <= Double(native) else { return false }
        let target = TradeProtocol.normalizeExpectedUSD(expectedUSD + (amount - amount * TradeProtocol.feeRate))
        guard target >= expectedUSD, searching || target > expectedUSD,
              let feeMsat = TradeProtocol.expectedTradeFeeMsat(
                  oldExpectedUSD: expectedUSD,
                  newExpectedUSD: target,
                  quotePrice: price
              ) else { return false }
        let fee = feeMsat / 1000
        guard fee <= receiverSats, fee <= spendableSats,
              let limit = StabilizationPolicy.clientLimit(spendableSats - fee),
              target <= BitcoinUnits.usd(sats: receiverSats - fee, price: price) else { return false }
        // Ignore the lower no-op bound only during the upper-bound search.
        if searching, backingSats == 0,
           (BitcoinUnits.sats(usd: target, price: price)).rounded(.down) == (BitcoinUnits.sats(
               usd: expectedUSD,
               price: price
           ))
           .rounded(.down) { return true }
        guard let backing = TradeProtocol.tradeBackingAfterDelta(
            receiverSats: receiverSats - fee,
            currentBackingSats: backingSats,
            currentExpectedUSD: expectedUSD,
            newExpectedUSD: target,
            price: price
        ) else { return false }
        return backing <= limit
    }

    func maxOrderCents() -> UInt64 {
        guard price.isFinite, price > 0, expectedUSD.isFinite, expectedUSD >= 0 else { return 0 }
        let native = nativeSats
        let cents = (BitcoinUnits.usd(sats: native, price: price) * 100).rounded(.down)
        guard cents.isFinite, cents >= 1, cents < Double(Int64.max) else { return 0 }
        var low: UInt64 = 0
        var high = UInt64(cents)
        while low < high {
            let distance = high - low
            let mid = low + distance / 2 + distance % 2
            if fits(mid, searching: true) { low = mid } else { high = mid - 1 }
        }
        return low > 0 && accepts(low) ? low : 0
    }
}

struct TradeCorrelation: Equatable {
    let tradeId: String
    let tradePaymentId: String
    let requestHash: String
}

enum TradeControlMessage: Equatable {
    struct Sync: Equatable {
        let channelId: String
        let userChannelId: String
        let expectedUSD: Double
        let backingSats: UInt64
        let syncVersion: UInt64
        let correlation: TradeCorrelation?
    }

    struct Rejected: Equatable {
        let channelId: String
        let correlation: TradeCorrelation
        let reasonCode: String
        let decidedAt: UInt64
    }

    case sync(Sync)
    case rejected(Rejected)
}

struct PreparedMobileTrade {
    let channelId: String
    let userChannelId: String
    let tradeId: String
    let requestHash: String
    let requestPayload: String
    let action: String
    let amountUSD: Double
    let amountBTC: Double
    let feeUSD: Double
    let feeMsat: UInt64
    let oldExpectedUSD: Double
    let newExpectedUSD: Double
    let newBackingSats: UInt64
    let quotePrice: Double
    let createdAt: UInt64
    let expiresAt: UInt64
}

enum TradeProtocol {
    /// Signed control TLVs larger than this are refused before any parsing.
    static let maxControlTLVBytes = 8 * 1024
    static let resultControlAmountMsat: UInt64 = 1
    static let resultTimeoutSecs: UInt64 = 15 * 60
    static let responseRetryWindowSecs: UInt64 = 14 * 24 * 60 * 60
    static let feeRate = 0.01
    private static let stabilityThresholdUSD = 0.25
    private static let stabilityThresholdPercent = 0.1
    private static let rejectionReasons: Set<String> = [
        "invalid_amount", "stale_request", "invalid_fee", "invalid_quote",
        "quote_deviation", "insufficient_capacity", "settlement_required",
        "unsafe_allocation", "internal_failure"
    ]

    static func normalizeExpectedUSD(_ value: Double) -> Double {
        value.isFinite && value >= 0 && value < 0.01 ? 0 : value
    }

    static func requestHash(_ bytes: Data) -> String {
        SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }

    static func expectedTradeFeeMsat(
        oldExpectedUSD: Double,
        newExpectedUSD: Double,
        quotePrice: Double
    ) -> UInt64? {
        guard oldExpectedUSD.isFinite, oldExpectedUSD >= 0,
              newExpectedUSD.isFinite, newExpectedUSD >= 0,
              quotePrice.isFinite, quotePrice > 0,
              feeRate.isFinite, feeRate >= 0, feeRate < 1 else { return nil }
        let targetDelta = abs(newExpectedUSD - oldExpectedUSD)
        let grossUSD = newExpectedUSD > oldExpectedUSD ? targetDelta / (1 - feeRate) : targetDelta
        let feeSats = BitcoinUnits.sats(usd: grossUSD * feeRate, price: quotePrice)
        // Swift traps on invalid floating-point to UInt64 conversions. Match Rust's valid-input
        // truncation only after proving the value can be represented and multiplied by 1,000.
        guard feeSats.isFinite, feeSats >= 0, feeSats <= Double(UInt64.max / 1000) else { return nil }
        return max(UInt64(feeSats.rounded(.towardZero)) * 1000, 1)
    }

    /// Nullable form, kept for callers that only care whether a trade could be built.
    static func prepare(
        channelId: String,
        userChannelId: String,
        currentExpectedUSD: Double,
        currentBackingSats: UInt64,
        receiverSats: UInt64,
        spendableSats: UInt64,
        action: String,
        amountUSD: Double,
        amountBTC: Double,
        feeUSD: Double,
        newExpectedUSD: Double,
        quotePrice: Double,
        now: UInt64 = UInt64(Date().timeIntervalSince1970),
        tradeId: String = randomIdentifier()
    ) -> PreparedMobileTrade? {
        try? prepareOrFailure(
            channelId: channelId,
            userChannelId: userChannelId,
            currentExpectedUSD: currentExpectedUSD,
            currentBackingSats: currentBackingSats,
            receiverSats: receiverSats,
            spendableSats: spendableSats,
            action: action,
            amountUSD: amountUSD,
            amountBTC: amountBTC,
            feeUSD: feeUSD,
            newExpectedUSD: newExpectedUSD,
            quotePrice: quotePrice,
            now: now,
            tradeId: tradeId
        ).get()
    }

    /// Builds a trade, or reports which local check refused it.
    static func prepareOrFailure(
        channelId: String,
        userChannelId: String,
        currentExpectedUSD: Double,
        currentBackingSats: UInt64,
        receiverSats: UInt64,
        spendableSats: UInt64,
        action: String,
        amountUSD: Double,
        amountBTC: Double,
        feeUSD: Double,
        newExpectedUSD: Double,
        quotePrice: Double,
        now: UInt64 = UInt64(Date().timeIntervalSince1970),
        tradeId: String = randomIdentifier()
    ) -> Result<PreparedMobileTrade, TradeValidationError> {
        let normalizedExpected = normalizeExpectedUSD(newExpectedUSD)
        guard isCanonicalIdentifier(channelId), !userChannelId.isEmpty,
              isCanonicalIdentifier(tradeId) else { return .failure(.channelNotReady) }
        guard amountUSD.isFinite, amountUSD > 0,
              amountBTC.isFinite, amountBTC >= 0, feeUSD.isFinite, feeUSD >= 0 else {
            return .failure(.invalidAmount)
        }
        guard let feeMsat = expectedTradeFeeMsat(
            oldExpectedUSD: currentExpectedUSD,
            newExpectedUSD: normalizedExpected,
            quotePrice: quotePrice
        ) else { return .failure(.feeUnavailable) }
        let feeSats = feeMsat / 1000
        guard feeSats <= receiverSats else { return .failure(.feeExceedsBalance) }
        let postFeeReceiver = receiverSats - feeSats
        guard let backing = tradeBackingAfterDelta(
            receiverSats: postFeeReceiver,
            currentBackingSats: currentBackingSats,
            currentExpectedUSD: currentExpectedUSD,
            newExpectedUSD: normalizedExpected,
            price: quotePrice
        ) else { return .failure(.allocationUnavailable) }

        // Preparation is the last pure guard before persistence/payment, not a settlement rule.
        if action == "sell" || normalizedExpected > currentExpectedUSD {
            let snapshot = StabilizationSnapshot(receiverSats: receiverSats, spendableSats: spendableSats,
                                                 backingSats: currentBackingSats, expectedUSD: currentExpectedUSD,
                                                 price: quotePrice)
            guard feeSats <= spendableSats else { return .failure(.feeExceedsBalance) }
            guard let limit = StabilizationPolicy.clientLimit(spendableSats - feeSats), backing <= limit,
                  amountUSD * 100 < Double(Int64.max),
                  snapshot.accepts(UInt64((amountUSD * 100 + 1e-7).rounded(.down))) else {
                return .failure(.stabilizationLimit(snapshot.maxOrderCents()))
            }
        }

        let object: [String: Any] = [
            "type": "TRADE_V1",
            "channel_id": channelId,
            "user_channel_id": userChannelId,
            "trade_id": tradeId,
            "expected_usd": normalizedExpected,
            "quote_price": quotePrice,
            "ts": now
        ]
        // Unreachable once the identifiers and amounts above are validated; keep the amount
        // reason rather than inventing an internal one the user could never act on.
        guard JSONSerialization.isValidJSONObject(object),
              let payloadData = try? JSONSerialization.data(
                  withJSONObject: object,
                  options: [.sortedKeys, .withoutEscapingSlashes]
              ),
              let payload = String(data: payloadData, encoding: .utf8) else {
            return .failure(.invalidAmount)
        }
        return .success(PreparedMobileTrade(
            channelId: channelId,
            userChannelId: userChannelId,
            tradeId: tradeId,
            requestHash: requestHash(payloadData),
            requestPayload: payload,
            action: action,
            amountUSD: amountUSD,
            amountBTC: amountBTC,
            feeUSD: feeUSD,
            feeMsat: feeMsat,
            oldExpectedUSD: currentExpectedUSD,
            newExpectedUSD: normalizedExpected,
            newBackingSats: backing,
            quotePrice: quotePrice,
            createdAt: now,
            expiresAt: now + resultTimeoutSecs
        ))
    }

    static func tradeBackingAfterDelta(
        receiverSats: UInt64,
        currentBackingSats: UInt64,
        currentExpectedUSD: Double,
        newExpectedUSD: Double,
        price: Double
    ) -> UInt64? {
        let normalizedExpected = normalizeExpectedUSD(newExpectedUSD)
        guard currentExpectedUSD.isFinite, currentExpectedUSD >= 0,
              normalizedExpected.isFinite, normalizedExpected >= 0,
              price.isFinite, price > 0 else { return nil }
        let receiverUSD = BitcoinUnits.usd(sats: receiverSats, price: price)
        guard normalizedExpected <= receiverUSD else { return nil }
        if normalizedExpected == 0 {
            return allocationDriftIsActionable(
                backingSats: currentBackingSats,
                expectedUSD: currentExpectedUSD,
                price: price
            ) ? nil : 0
        }

        guard let currentTargetSats = BitcoinUnits.floorTargetSats(usd: currentExpectedUSD, price: price),
              let newTargetSats = BitcoinUnits.floorTargetSats(usd: normalizedExpected, price: price)
        else { return nil }
        let backing: UInt64
        if normalizedExpected >= currentExpectedUSD {
            let delta = newTargetSats - currentTargetSats
            let (value, overflow) = currentBackingSats.addingReportingOverflow(delta)
            guard !overflow else { return nil }
            backing = value
        } else {
            let delta = currentTargetSats - newTargetSats
            guard delta <= currentBackingSats else { return nil }
            backing = currentBackingSats - delta
        }
        var normalizedBacking = backing
        if currentExpectedUSD < 0.01, currentBackingSats == 0, backing <= receiverSats {
            let nativeUSD = BitcoinUnits.usd(sats: receiverSats - backing, price: price)
            if nativeUSD < 0.01 { normalizedBacking = receiverSats }
        }
        return normalizedBacking > 0 && normalizedBacking <= receiverSats ? normalizedBacking : nil
    }

    /// Reconcile an authenticated target using our allocation and price, bounded by our balance.
    /// Unlike a new trade, a correction must not fail just because its target exceeds capacity
    /// (#318). Zero-target settlement is handled separately by the caller.
    static func syncBackingAfterDelta(
        receiverSats: UInt64,
        currentBackingSats: UInt64,
        currentExpectedUSD: Double,
        newExpectedUSD: Double,
        price: Double
    ) -> UInt64? {
        guard currentExpectedUSD.isFinite, currentExpectedUSD >= 0,
              newExpectedUSD.isFinite, newExpectedUSD > 0,
              price.isFinite, price > 0 else { return nil }
        if let backing = tradeBackingAfterDelta(
            receiverSats: receiverSats,
            currentBackingSats: currentBackingSats,
            currentExpectedUSD: currentExpectedUSD,
            newExpectedUSD: newExpectedUSD,
            price: price
        ) {
            return backing
        }

        // Use cumulative floors, as trades do, without repricing the existing position or
        // adopting the peer's backing. This preserves unsettled drift through corrections.
        let int64Max = UInt64(Int64.max)
        guard let currentTarget = BitcoinUnits.floorTargetSats(usd: currentExpectedUSD, price: price),
              let newTarget = BitcoinUnits.floorTargetSats(usd: newExpectedUSD, price: price),
              currentTarget <= int64Max, newTarget <= int64Max,
              currentBackingSats <= int64Max, receiverSats <= int64Max else { return nil }
        let delta = Int64(newTarget) - Int64(currentTarget)
        let current = Int64(currentBackingSats)
        let receiver = Int64(receiverSats)
        let backing: Int64
        if delta >= 0 {
            // Clamp before adding to avoid overflowing on a large correction.
            let available = max(receiver - current, 0)
            backing = min(current, receiver) + min(delta, available)
        } else {
            backing = min(max(current + delta, 0), receiver)
        }
        return backing > 0 ? UInt64(backing) : nil
    }

    /// The backing an uncorrelated SYNC writes. A zero target keeps leftover backing only up to
    /// what the LSP still books: its signed residue settles via the stability machinery (#322),
    /// while a signed backing of 0 means the LSP already booked the close, so the wallet's stale
    /// pre-exit allocation is released rather than paid away as phantom surplus.
    /// Nil means the SYNC cannot be applied yet and must be retried.
    static func uncorrelatedSyncBacking(
        receiverSats: UInt64,
        currentBackingSats: UInt64,
        currentExpectedUSD: Double,
        sync: TradeControlMessage.Sync,
        price: Double
    ) -> UInt64? {
        if normalizeExpectedUSD(sync.expectedUSD) == 0 {
            return min(currentBackingSats, sync.backingSats, receiverSats)
        }
        if currentBackingSats > 0, sync.expectedUSD == currentExpectedUSD {
            return min(currentBackingSats, receiverSats)
        }
        return syncBackingAfterDelta(
            receiverSats: receiverSats,
            currentBackingSats: currentBackingSats,
            currentExpectedUSD: currentExpectedUSD,
            newExpectedUSD: sync.expectedUSD,
            price: price
        )
    }

    /// Size, strict UTF-8 and field validation all run before signature work, so oversize or
    /// malformed peer input never reaches the node's verifier (matches Android).
    static func parseSignedControl(
        data: Data,
        expectedCounterparty: String,
        verifySignature: ([UInt8], String, String) -> Bool
    ) -> TradeControlMessage? {
        guard data.count <= maxControlTLVBytes,
              let raw = String(data: data, encoding: .utf8),
              let envelope = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any],
              let payload = envelope["payload"] as? String,
              let signature = envelope["signature"] as? String,
              let object = try? JSONSerialization.jsonObject(with: Data(payload.utf8)) as? [String: Any],
              let type = object["type"] as? String else { return nil }
        let message: TradeControlMessage?
        switch type {
        case "SYNC_V1": message = parseSync(object).map(TradeControlMessage.sync)
        case "TRADE_REJECTED_V1": message = parseRejection(object).map(TradeControlMessage.rejected)
        default: message = nil
        }
        guard let message, verifySignature(Array(payload.utf8), signature, expectedCounterparty) else { return nil }
        return message
    }

    private static func parseSync(_ object: [String: Any]) -> TradeControlMessage.Sync? {
        guard let channelId = object["channel_id"] as? String,
              let userChannelId = object["user_channel_id"] as? String,
              let expectedValue = jsonDouble(object["expected_usd"]),
              let backingSigned = jsonInteger(object["backing_sats"]),
              let versionSigned = jsonInteger(object["sync_version"]) else { return nil }
        let expected = normalizeExpectedUSD(expectedValue)
        guard isCanonicalIdentifier(channelId), !userChannelId.isEmpty,
              expected.isFinite, expected >= 0, backingSigned >= 0, versionSigned > 0 else { return nil }
        let fields = ["trade_id", "trade_payment_id", "request_hash"]
        let present = fields.filter { object[$0] != nil }.count
        let correlation: TradeCorrelation?
        switch present {
        case 0: correlation = nil
        case 3:
            guard let tradeId = object["trade_id"] as? String,
                  let paymentId = object["trade_payment_id"] as? String,
                  let hash = object["request_hash"] as? String,
                  isCanonicalIdentifier(tradeId), isCanonicalIdentifier(paymentId),
                  isCanonicalIdentifier(hash) else { return nil }
            correlation = TradeCorrelation(
                tradeId: tradeId,
                tradePaymentId: paymentId,
                requestHash: hash
            )
        default: return nil
        }
        return TradeControlMessage.Sync(
            channelId: channelId,
            userChannelId: userChannelId,
            expectedUSD: expected,
            backingSats: UInt64(backingSigned),
            syncVersion: UInt64(versionSigned),
            correlation: correlation
        )
    }

    private static func parseRejection(_ object: [String: Any]) -> TradeControlMessage.Rejected? {
        let allowed: Set = [
            "type", "channel_id", "trade_id", "trade_payment_id", "request_hash",
            "reason_code", "decided_at"
        ]
        guard Set(object.keys).isSubset(of: allowed),
              let channelId = object["channel_id"] as? String,
              let tradeId = object["trade_id"] as? String,
              let paymentId = object["trade_payment_id"] as? String,
              let hash = object["request_hash"] as? String,
              let reason = object["reason_code"] as? String,
              let decidedAt = jsonInteger(object["decided_at"]),
              decidedAt >= 0,
              isCanonicalIdentifier(channelId), isCanonicalIdentifier(tradeId),
              isCanonicalIdentifier(paymentId), isCanonicalIdentifier(hash),
              rejectionReasons.contains(reason) else { return nil }
        return TradeControlMessage.Rejected(
            channelId: channelId,
            correlation: TradeCorrelation(
                tradeId: tradeId,
                tradePaymentId: paymentId,
                requestHash: hash
            ),
            reasonCode: reason,
            decidedAt: UInt64(decidedAt)
        )
    }

    static func rejectionMessage(_ reason: String) -> String {
        switch reason {
        case "invalid_amount": return "The trade amount is invalid. Review the amount and retry."
        case "stale_request": return "The quote expired before it could be accepted. Refresh and retry."
        case "invalid_fee": return "The trade fee was invalid. Refresh the quote before retrying."
        case "invalid_quote": return "A valid market quote is required. Refresh and retry."
        case "quote_deviation": return "The market moved outside the quote range. Refresh and retry."
        case "insufficient_capacity": return "The channel does not have enough capacity for this trade. Reduce the amount."
        case "settlement_required": return "Settle the current stability adjustment before retrying this trade."
        case "unsafe_allocation": return "This trade cannot preserve the current channel allocation safely."
        default: return "The provider could not process the trade. Try again later."
        }
    }

    static func isCanonicalIdentifier(_ value: String) -> Bool {
        value.count == 64 && value.utf8.allSatisfy {
            ($0 >= 48 && $0 <= 57) || ($0 >= 97 && $0 <= 102)
        }
    }

    private static func jsonDouble(_ value: Any?) -> Double? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID(),
              number.doubleValue.isFinite else { return nil }
        return number.doubleValue
    }

    private static func jsonInteger(_ value: Any?) -> Int64? {
        guard let number = value as? NSNumber,
              CFGetTypeID(number) != CFBooleanGetTypeID(),
              !CFNumberIsFloatType(number) else { return nil }
        return number.int64Value
    }

    private static func allocationDriftIsActionable(
        backingSats: UInt64,
        expectedUSD: Double,
        price: Double
    ) -> Bool {
        let currentValue = BitcoinUnits.usd(sats: backingSats, price: price)
        let driftUSD = abs(currentValue - expectedUSD)
        if expectedUSD < 0.01 { return driftUSD >= stabilityThresholdUSD }
        let driftPercent = driftUSD / expectedUSD * 100
        return driftUSD >= stabilityThresholdUSD && driftPercent >= stabilityThresholdPercent
    }

    private static func randomIdentifier() -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        guard SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess else {
            return UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
                + UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased()
        }
        return bytes.map { String(format: "%02x", $0) }.joined()
    }
}

// MARK: - Signed stability settlements (STABILITY_PAYMENT_V1, issue #270)

struct StabilitySettlement: Equatable {
    let settlementId: String
    let channelId: String
    let amountMsat: UInt64
    let direction: String
    let expectedUSD: Double
    let createdAt: UInt64
    let expiresAt: UInt64
}

enum StabilitySettlementValidation: Equatable {
    case valid(StabilitySettlement)
    case invalid(String)
}

extension TradeProtocol {
    /// Resolve authentication from the channel being accounted for, never from the default
    /// LSP or a different channel. Readiness is irrelevant for an already received payment.
    static func settlementCounterparty(
        channelId: String,
        userChannelId: String,
        channels: [(channelId: String, userChannelId: String, counterparty: String)]
    ) -> String? {
        guard !channelId.isEmpty, !userChannelId.isEmpty else { return nil }
        let matching = channels.filter {
            $0.channelId == channelId && $0.userChannelId == userChannelId
        }
        guard matching.count == 1, let peer = matching.first?.counterparty,
              !peer.isEmpty else { return nil }
        return peer
    }

    static let stabilitySettlementMessageType = "STABILITY_PAYMENT_V1"
    static let stabilityDirectionUserToLsp = "user_to_lsp"
    static let stabilityDirectionLspToUser = "lsp_to_user"
    static let stabilitySettlementTTLSecs: UInt64 = 1_209_600
    static let stabilitySettlementClockSkewSecs: UInt64 = 60
    static let stabilitySettlementMaxEnvelopeBytes = 8 * 1024

    /// Build the signed STABILITY_PAYMENT_V1 envelope TLV value for an outgoing settlement.
    /// Returns nil on invalid inputs or signing failure; callers must skip the payment
    /// entirely — there is no legacy [1] marker fallback.
    static func buildSignedStabilitySettlement(
        channelId: String,
        amountMsat: UInt64,
        direction: String,
        expectedUSD: Double,
        now: UInt64 = UInt64(Date().timeIntervalSince1970),
        settlementId: String = randomIdentifier(),
        sign: ([UInt8]) throws -> String
    ) -> Data? {
        guard isCanonicalIdentifier(settlementId), isCanonicalIdentifier(channelId),
              amountMsat > 0, amountMsat % 1000 == 0,
              direction == stabilityDirectionUserToLsp || direction == stabilityDirectionLspToUser,
              expectedUSD.isFinite, expectedUSD >= 0 else { return nil }
        let object: [String: Any] = [
            "type": stabilitySettlementMessageType,
            "settlement_id": settlementId,
            "channel_id": channelId,
            "amount_msat": amountMsat,
            "direction": direction,
            "expected_usd": expectedUSD,
            "created_at": now,
            "expires_at": now + stabilitySettlementTTLSecs
        ]
        guard JSONSerialization.isValidJSONObject(object),
              let payloadData = try? JSONSerialization.data(
                  withJSONObject: object,
                  options: [.sortedKeys, .withoutEscapingSlashes]
              ),
              let payload = String(data: payloadData, encoding: .utf8),
              let signature = try? sign(Array(payload.utf8)) else { return nil }
        let envelope: [String: Any] = [
            "payload": payload,
            "signature": signature
        ]
        guard let envelopeData = try? JSONSerialization.data(
            withJSONObject: envelope,
            options: [.sortedKeys, .withoutEscapingSlashes]
        ), envelopeData.count <= stabilitySettlementMaxEnvelopeBytes else { return nil }
        return envelopeData
    }

    /// Parse and fully validate an inbound signed settlement TLV value: envelope size and
    /// shape, payload fields, freshness window, direction, channel, amount binding, and
    /// counterparty signature. The invalid reason strings are stable for log assertions.
    static func parseSignedStabilitySettlement(
        data: Data,
        expectedDirection: String,
        expectedChannelId: String,
        actualAmountMsat: UInt64,
        expectedCounterparty: String,
        now: UInt64 = UInt64(Date().timeIntervalSince1970),
        verifySignature: ([UInt8], String, String) -> Bool
    ) -> StabilitySettlementValidation {
        guard data.count <= stabilitySettlementMaxEnvelopeBytes else {
            return .invalid("envelope_too_large")
        }
        guard let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let payload = envelope["payload"] as? String,
              let signature = envelope["signature"] as? String,
              let payloadData = payload.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: payloadData) as? [String: Any],
              let type = object["type"] as? String,
              type == stabilitySettlementMessageType else {
            return .invalid("malformed_envelope")
        }
        guard let settlementId = object["settlement_id"] as? String,
              let channelId = object["channel_id"] as? String,
              let direction = object["direction"] as? String,
              let amountSigned = jsonInteger(object["amount_msat"]),
              let expectedValue = jsonDouble(object["expected_usd"]),
              let createdSigned = jsonInteger(object["created_at"]),
              let expiresSigned = jsonInteger(object["expires_at"]),
              isCanonicalIdentifier(settlementId), isCanonicalIdentifier(channelId),
              amountSigned > 0, amountSigned % 1000 == 0,
              expectedValue.isFinite, expectedValue >= 0,
              createdSigned >= 0, expiresSigned >= createdSigned,
              expiresSigned - createdSigned <= Int64(stabilitySettlementTTLSecs) else {
            return .invalid("invalid_fields")
        }
        let settlement = StabilitySettlement(
            settlementId: settlementId,
            channelId: channelId,
            amountMsat: UInt64(amountSigned),
            direction: direction,
            expectedUSD: expectedValue,
            createdAt: UInt64(createdSigned),
            expiresAt: UInt64(expiresSigned)
        )
        guard settlement.createdAt <= now + stabilitySettlementClockSkewSecs,
              now <= settlement.expiresAt + stabilitySettlementClockSkewSecs else {
            return .invalid("stale")
        }
        guard settlement.direction == expectedDirection else { return .invalid("wrong_direction") }
        guard settlement.channelId == expectedChannelId else { return .invalid("channel_mismatch") }
        guard settlement.amountMsat == actualAmountMsat else { return .invalid("amount_mismatch") }
        guard verifySignature(Array(payload.utf8), signature, expectedCounterparty) else {
            return .invalid("bad_signature")
        }
        return .valid(settlement)
    }
}
