import Foundation
import XCTest
@testable import StableChannels

final class StabilitySettlementTests: XCTestCase {
    private let channelId = String(repeating: "ab", count: 32)
    private let settlementId = String(repeating: "cd", count: 32)
    private let now: UInt64 = 1_700_000_000
    private let ttl = TradeProtocol.stabilitySettlementTTLSecs

    private func sign(_: [UInt8]) throws -> String { "sig" }

    private func makeEnvelope(
        settlementId: String? = nil,
        channelId: String? = nil,
        amountMsat: Int64 = 25_000,
        direction: String = "lsp_to_user",
        expectedUSD: Double = 10.5,
        createdAt: Int64 = 1_700_000_000,
        expiresAt: Int64? = nil,
        signature: String = "sig"
    ) -> Data {
        let payload: [String: Any] = [
            "type": "STABILITY_PAYMENT_V1",
            "settlement_id": settlementId ?? self.settlementId,
            "channel_id": channelId ?? self.channelId,
            "amount_msat": amountMsat,
            "direction": direction,
            "expected_usd": expectedUSD,
            "created_at": createdAt,
            "expires_at": expiresAt ?? createdAt + 1_209_600
        ]
        let payloadData = try! JSONSerialization.data(
            withJSONObject: payload,
            options: [.sortedKeys, .withoutEscapingSlashes]
        )
        let envelope: [String: Any] = [
            "payload": String(data: payloadData, encoding: .utf8)!,
            "signature": signature
        ]
        return try! JSONSerialization.data(
            withJSONObject: envelope,
            options: [.sortedKeys, .withoutEscapingSlashes]
        )
    }

    private func parse(
        _ data: Data,
        expectedDirection: String = "lsp_to_user",
        expectedChannelId: String? = nil,
        actualAmountMsat: UInt64 = 25_000,
        now: UInt64? = nil,
        verifySignature: @escaping ([UInt8], String, String) -> Bool = { _, sig, _ in sig == "sig" }
    ) -> StabilitySettlementValidation {
        TradeProtocol.parseSignedStabilitySettlement(
            data: data,
            expectedDirection: expectedDirection,
            expectedChannelId: expectedChannelId ?? channelId,
            actualAmountMsat: actualAmountMsat,
            expectedCounterparty: "counterparty",
            now: now ?? self.now,
            verifySignature: verifySignature
        )
    }

    func testRoundTripHappyPath() throws {
        let envelope = try XCTUnwrap(TradeProtocol.buildSignedStabilitySettlement(
            channelId: channelId,
            amountMsat: 25_000,
            direction: TradeProtocol.stabilityDirectionLspToUser,
            expectedUSD: 10.5,
            now: now,
            settlementId: settlementId,
            sign: sign
        ))
        XCTAssertLessThanOrEqual(envelope.count, TradeProtocol.stabilitySettlementMaxEnvelopeBytes)

        guard case .valid(let settlement) = parse(envelope) else {
            return XCTFail("expected valid settlement")
        }
        XCTAssertEqual(settlement.settlementId, settlementId)
        XCTAssertEqual(settlement.channelId, channelId)
        XCTAssertEqual(settlement.amountMsat, 25_000)
        XCTAssertEqual(settlement.direction, "lsp_to_user")
        XCTAssertEqual(settlement.expectedUSD, 10.5)
        XCTAssertEqual(settlement.createdAt, now)
        XCTAssertEqual(settlement.expiresAt, now + ttl)
    }

    func testBuildRejectsNonWholeSatAmounts() {
        XCTAssertNil(TradeProtocol.buildSignedStabilitySettlement(
            channelId: channelId, amountMsat: 1_500,
            direction: TradeProtocol.stabilityDirectionUserToLsp,
            expectedUSD: 10.5, now: now, settlementId: settlementId, sign: sign
        ))
        XCTAssertNil(TradeProtocol.buildSignedStabilitySettlement(
            channelId: channelId, amountMsat: 0,
            direction: TradeProtocol.stabilityDirectionUserToLsp,
            expectedUSD: 10.5, now: now, settlementId: settlementId, sign: sign
        ))
    }

    func testBuildRejectsNonCanonicalIds() {
        XCTAssertNil(TradeProtocol.buildSignedStabilitySettlement(
            channelId: "not-hex", amountMsat: 25_000,
            direction: TradeProtocol.stabilityDirectionUserToLsp,
            expectedUSD: 10.5, now: now, settlementId: settlementId, sign: sign
        ))
        XCTAssertNil(TradeProtocol.buildSignedStabilitySettlement(
            channelId: channelId, amountMsat: 25_000,
            direction: TradeProtocol.stabilityDirectionUserToLsp,
            expectedUSD: 10.5, now: now,
            settlementId: String(repeating: "AB", count: 32), sign: sign
        ))
    }

    func testParseRejectsAmountNotMultipleOf1000() {
        XCTAssertEqual(parse(makeEnvelope(amountMsat: 1_500)), .invalid("invalid_fields"))
        XCTAssertEqual(parse(makeEnvelope(amountMsat: 0)), .invalid("invalid_fields"))
    }

    func testParseRejectsNonCanonicalIds() {
        XCTAssertEqual(parse(makeEnvelope(settlementId: "zz" + String(settlementId.dropFirst(2)))),
                       .invalid("invalid_fields"))
        XCTAssertEqual(parse(makeEnvelope(channelId: String(channelId.dropFirst(2)))),
                       .invalid("invalid_fields"))
    }

    func testParseRejectsTTLExceeded() {
        let data = makeEnvelope(createdAt: 1_700_000_000, expiresAt: 1_700_000_000 + 1_209_601)
        XCTAssertEqual(parse(data), .invalid("invalid_fields"))
    }

    func testParseRejectsExpiredSettlement() {
        XCTAssertEqual(parse(makeEnvelope(), now: now + ttl + 61), .invalid("stale"))
        // Boundary: exactly expires_at + skew is still fresh.
        guard case .valid = parse(makeEnvelope(), now: now + ttl + 60) else {
            return XCTFail("expected valid at freshness boundary")
        }
    }

    func testParseRejectsFutureCreatedAt() {
        XCTAssertEqual(parse(makeEnvelope(), now: now - 61), .invalid("stale"))
    }

    func testParseRejectsWrongDirection() {
        XCTAssertEqual(parse(makeEnvelope(direction: "user_to_lsp")), .invalid("wrong_direction"))
    }

    func testParseRejectsChannelMismatch() {
        let other = String(repeating: "ef", count: 32)
        XCTAssertEqual(parse(makeEnvelope(), expectedChannelId: other), .invalid("channel_mismatch"))
    }

    func testParseRejectsAmountMismatch() {
        XCTAssertEqual(parse(makeEnvelope(), actualAmountMsat: 26_000), .invalid("amount_mismatch"))
    }

    func testParseRejectsBadSignature() {
        XCTAssertEqual(
            parse(makeEnvelope(), verifySignature: { _, _, _ in false }),
            .invalid("bad_signature")
        )
    }

    func testParseRejectsOversizedEnvelope() {
        XCTAssertEqual(parse(Data(count: 8_193)), .invalid("envelope_too_large"))
    }

    func testParseRejectsMalformedEnvelope() throws {
        XCTAssertEqual(parse(Data("[]".utf8)), .invalid("malformed_envelope"))
        XCTAssertEqual(parse(Data("{\"payload\":{}}".utf8)), .invalid("malformed_envelope"))
        let wrongType = makeEnvelope(direction: "lsp_to_user")
        var tampered = try XCTUnwrap(
            JSONSerialization.jsonObject(with: wrongType) as? [String: Any]
        )
        tampered["payload"] = "{\"type\":\"SYNC_V1\"}"
        let tamperedData = try JSONSerialization.data(withJSONObject: tampered)
        XCTAssertEqual(parse(tamperedData), .invalid("malformed_envelope"))
    }

    func testLegacyMarkerValueNeverValidatesAsSettlement() {
        // The legacy one-byte [1] stability marker is removed (issue #270 follow-up):
        // a marker-only TLV value must never be accepted as a settlement.
        XCTAssertEqual(parse(Data([1])), .invalid("malformed_envelope"))
    }

    func testSignedEnvelopeAloneValidatesWithoutMarker() throws {
        // Signed-only send: the envelope on 13377333 with no legacy marker attached
        // must be sufficient to classify the payment as a stability settlement.
        let envelope = try XCTUnwrap(TradeProtocol.buildSignedStabilitySettlement(
            channelId: channelId,
            amountMsat: 25_000,
            direction: TradeProtocol.stabilityDirectionLspToUser,
            expectedUSD: 10.5,
            now: now,
            settlementId: settlementId,
            sign: sign
        ))
        guard case .valid(let settlement) = parse(envelope) else {
            return XCTFail("expected signed-only envelope to validate")
        }
        XCTAssertEqual(settlement.settlementId, settlementId)
    }
}
