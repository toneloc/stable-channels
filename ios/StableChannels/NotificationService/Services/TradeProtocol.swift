import CryptoKit
import CoreFoundation
import Foundation
import Security

// This file belongs to BOTH the app and notification extension targets. Trade-entry only:
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
        }
    }
}

struct StabilizationSnapshot {
    let receiverSats: UInt64
    let spendableSats: UInt64
    let backingSats: UInt64
    let expectedUSD: Double
    let price: Double

    func accepts(_ orderCents: UInt64) -> Bool {
        fits(orderCents, searching: false)
    }

    private func fits(_ orderCents: UInt64, searching: Bool) -> Bool {
        guard orderCents > 0, price.isFinite, price > 0, expectedUSD.isFinite, expectedUSD >= 0 else { return false }
        let amount = Double(orderCents) / 100
        let required = (amount / price * 100_000_000).rounded(.up)
        let native = receiverSats >= backingSats ? receiverSats - backingSats : 0
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
              target <= Double(receiverSats - fee) / 100_000_000 * price else { return false }
        // Ignore the lower no-op bound only during the upper-bound search.
        if searching, backingSats == 0,
           (target / price * 100_000_000).rounded(.down) == (expectedUSD / price * 100_000_000)
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
        let native = receiverSats >= backingSats ? receiverSats - backingSats : 0
        let cents = (Double(native) / 100_000_000 * price * 100).rounded(.down)
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
    static let resultControlAmountMsat: UInt64 = 1
    static let resultTimeoutSecs: UInt64 = 15 * 60
    static let responseRetryWindowSecs: UInt64 = 14 * 24 * 60 * 60
    private static let satsInBTC = 100_000_000.0
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
        let feeSats = grossUSD * feeRate / quotePrice * satsInBTC
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
        let receiverUSD = Double(receiverSats) / satsInBTC * price
        guard normalizedExpected <= receiverUSD else { return nil }
        if normalizedExpected == 0 {
            return allocationDriftIsActionable(
                backingSats: currentBackingSats,
                expectedUSD: currentExpectedUSD,
                price: price
            ) ? nil : 0
        }

        let currentTarget = currentExpectedUSD / price * satsInBTC
        let newTarget = normalizedExpected / price * satsInBTC
        guard currentTarget.isFinite, newTarget.isFinite,
              currentTarget >= 0, newTarget >= 0,
              currentTarget < Double(UInt64.max), newTarget < Double(UInt64.max) else { return nil }
        let currentTargetSats = UInt64(currentTarget.rounded(.down))
        let newTargetSats = UInt64(newTarget.rounded(.down))
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
            let nativeUSD = Double(receiverSats - backing) / satsInBTC * price
            if nativeUSD < 0.01 { normalizedBacking = receiverSats }
        }
        return normalizedBacking > 0 && normalizedBacking <= receiverSats ? normalizedBacking : nil
    }

    static func parseSignedControl(
        data: Data,
        expectedCounterparty: String,
        verifySignature: ([UInt8], String, String) -> Bool
    ) -> TradeControlMessage? {
        guard let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let payload = envelope["payload"] as? String,
              let signature = envelope["signature"] as? String,
              verifySignature(Array(payload.utf8), signature, expectedCounterparty),
              let payloadData = payload.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: payloadData) as? [String: Any],
              let type = object["type"] as? String else { return nil }
        switch type {
        case "SYNC_V1": return parseSync(object).map(TradeControlMessage.sync)
        case "TRADE_REJECTED_V1": return parseRejection(object).map(TradeControlMessage.rejected)
        default: return nil
        }
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
        let currentValue = Double(backingSats) / satsInBTC * price
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
