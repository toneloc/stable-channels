import LDKNode
import XCTest
@testable import StableChannels

/// The LSP's splice push (#475): its notification, and when the extension's wait for the splice ends.
final class LspSplicePushTests: XCTestCase {
    private let ucid = "42"
    private let start = Date(timeIntervalSince1970: 1_000)

    private func wait(startFunding: String? = "old", deadlineSecs: TimeInterval = 20) -> LspSpliceWait {
        LspSpliceWait(
            userChannelId: ucid,
            startFundingTxid: startFunding,
            deadline: start.addingTimeInterval(deadlineSecs),
            settleSecs: 3
        )
    }

    private func ready(_ txid: String?, channel: String? = nil) -> Event {
        .channelReady(
            channelId: "chan", userChannelId: channel ?? ucid, counterpartyNodeId: "lsp",
            fundingTxo: txid.map { OutPoint(txid: $0, vout: 0) }
        )
    }

    private func negotiated(_ txid: String, channel: String? = nil) -> Event {
        .spliceNegotiated(
            channelId: "chan", userChannelId: channel ?? ucid, counterpartyNodeId: "lsp",
            newFundingTxo: OutPoint(txid: txid, vout: 0)
        )
    }

    private func failed(channel: String? = nil) -> Event {
        .spliceNegotiationFailed(channelId: "chan", userChannelId: channel ?? ucid, counterpartyNodeId: "lsp")
    }

    private func at(_ secs: TimeInterval) -> Date {
        start.addingTimeInterval(secs)
    }

    private let noLiveFunding: (String) -> String? = { _ in nil }

    private func channel(userChannelId: String) -> ChannelDetails {
        ChannelDetails(
            channelId: "chan-\(userChannelId)",
            counterpartyNodeId: "lsp",
            fundingTxo: OutPoint(txid: "funding-\(userChannelId)", vout: 0),
            fundingRedeemScript: nil,
            shortChannelId: nil,
            outboundScidAlias: nil,
            inboundScidAlias: nil,
            channelValueSats: 100_000,
            unspendablePunishmentReserve: 1_000,
            userChannelId: userChannelId,
            feerateSatPer1000Weight: 253,
            outboundCapacityMsat: 50_000_000,
            inboundCapacityMsat: 50_000_000,
            confirmationsRequired: 1,
            confirmations: 6,
            isOutbound: false,
            isChannelReady: true,
            isUsable: true,
            isAnnounced: false,
            cltvExpiryDelta: 144,
            counterpartyUnspendablePunishmentReserve: 1_000,
            counterpartyOutboundHtlcMinimumMsat: 1_000,
            counterpartyOutboundHtlcMaximumMsat: 200_000_000,
            counterpartyForwardingInfoFeeBaseMsat: 1_000,
            counterpartyForwardingInfoFeeProportionalMillionths: 0,
            counterpartyForwardingInfoCltvExpiryDelta: 144,
            nextOutboundHtlcLimitMsat: 50_000_000,
            nextOutboundHtlcMinimumMsat: 1_000,
            forceCloseSpendDelay: 144,
            inboundHtlcMinimumMsat: 1_000,
            inboundHtlcMaximumMsat: 200_000_000,
            config: ChannelConfig(
                forwardingFeeProportionalMillionths: 100,
                forwardingFeeBaseMsat: 1000,
                cltvExpiryDelta: 144,
                maxDustHtlcExposure: .fixedLimit(limitMsat: 5_000_000),
                forceCloseAvoidanceMaxFeeSatoshis: 10_000,
                acceptUnderpayingHtlcs: false
            ),
            channelShutdownState: nil
        )
    }

    // MARK: - Notification

    func testTheSpliceNotificationIsNotTaggedAsAPayment() {
        XCTAssertEqual(LspSpliceNotice.notificationType, "lsp_splice")
        XCTAssertFalse(LspSpliceNotice.notificationType.hasPrefix("payment"))
    }

    // MARK: - The wait

    func testTheWaitEndsAFewSecondsAfterTheSpliceLocks() {
        var wait = wait()
        wait.see(negotiated("new"), at: at(2), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .negotiated)
        XCTAssertFalse(wait.isDone(at: at(3)))
        wait.see(ready("new"), at: at(5), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .locked)
        XCTAssertFalse(wait.isDone(at: at(7)), "a stability payment may follow the lock")
        XCTAssertTrue(wait.isDone(at: at(8)))
    }

    func testAReadyForTheFundingTheWaitStartedFromIsNotTheLock() {
        var wait = wait()
        wait.see(ready("old"), at: at(1), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .timedOut)
        XCTAssertFalse(wait.isDone(at: at(19)))
        XCTAssertTrue(wait.isDone(at: at(20)), "the deadline ends it")
    }

    func testAReadyWithoutFundingIsJudgedByTheLiveChannel() {
        var stale = wait()
        stale.see(ready(nil), at: at(1)) { _ in "old" }
        XCTAssertEqual(stale.outcome, .timedOut)
        var moved = wait()
        moved.see(ready(nil), at: at(1)) { $0 == self.ucid ? "new" : nil }
        XCTAssertEqual(moved.outcome, .locked)
    }

    func testWithoutTheStartingFundingNoReadyEndsTheWaitEarly() {
        var wait = wait(startFunding: nil)
        wait.see(ready("new"), at: at(1), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .timedOut)
        XCTAssertFalse(wait.isDone(at: at(10)))
    }

    func testAFailedNegotiationEndsTheWait() {
        var wait = wait()
        wait.see(negotiated("new"), at: at(1), liveFunding: noLiveFunding)
        wait.see(failed(), at: at(2), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .failed)
        XCTAssertTrue(wait.isDone(at: at(2)))
    }

    func testAFailureAfterTheLockKeepsTheLock() {
        var wait = wait()
        wait.see(ready("new"), at: at(1), liveFunding: noLiveFunding)
        wait.see(failed(), at: at(2), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .locked)
        XCTAssertTrue(wait.isDone(at: at(2)))
    }

    func testEventsOnAnotherChannelAreIgnored() {
        var wait = wait()
        wait.see(negotiated("new", channel: "7"), at: at(1), liveFunding: noLiveFunding)
        wait.see(ready("new", channel: "7"), at: at(2), liveFunding: noLiveFunding)
        wait.see(failed(channel: "7"), at: at(3), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .timedOut)
        XCTAssertFalse(wait.isDone(at: at(10)))
    }

    func testTheWaitWatchesTheSavedChannelElseTheOnlyOne() {
        let saved = channel(userChannelId: "42"), other = channel(userChannelId: "7")
        XCTAssertEqual(LspSpliceWait.channel(in: [other, saved], savedUserChannelId: "42")?.userChannelId, "42")
        XCTAssertEqual(LspSpliceWait.channel(in: [other], savedUserChannelId: "42")?.userChannelId, "7")
        XCTAssertEqual(LspSpliceWait.channel(in: [other], savedUserChannelId: nil)?.userChannelId, "7")
        XCTAssertNil(LspSpliceWait.channel(in: [other, channel(userChannelId: "8")], savedUserChannelId: "42"))
        XCTAssertNil(LspSpliceWait.channel(in: [], savedUserChannelId: "42"))
    }

    func testWithNoChannelToWatchOnlyTheDeadlineEndsTheWait() {
        var wait = LspSpliceWait(userChannelId: nil, startFundingTxid: "old", deadline: at(20), settleSecs: 3)
        wait.see(ready("new"), at: at(1), liveFunding: noLiveFunding)
        wait.see(failed(), at: at(2), liveFunding: noLiveFunding)
        XCTAssertEqual(wait.outcome, .timedOut)
        XCTAssertFalse(wait.isDone(at: at(19)))
        XCTAssertTrue(wait.isDone(at: at(20)))
    }
}
