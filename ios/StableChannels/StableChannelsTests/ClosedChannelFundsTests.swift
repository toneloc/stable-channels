import LDKNode
@testable import StableChannels
import XCTest

final class ClosedChannelFundsTests: XCTestCase {
    private let peer = "02" + String(repeating: "a", count: 64)

    private func onClose(_ channelId: String, _ sats: UInt64) -> LightningBalance {
        .claimableOnChannelClose(
            channelId: channelId,
            counterpartyNodeId: peer,
            amountSatoshis: sats,
            transactionFeeSatoshis: 0,
            outboundPaymentHtlcRoundedMsat: 0,
            outboundForwardedHtlcRoundedMsat: 0,
            inboundClaimingHtlcRoundedMsat: 0,
            inboundHtlcRoundedMsat: 0
        )
    }

    private func awaiting(_ sats: UInt64, _ height: UInt32, _ source: BalanceSource) -> LightningBalance {
        .claimableAwaitingConfirmations(
            channelId: "closed",
            counterpartyNodeId: peer,
            amountSatoshis: sats,
            confirmationHeight: height,
            source: source
        )
    }

    func testOpenChannelIsNotCounted() {
        XCTAssertNil(ClosedChannelFunds.from(
            lightningBalances: [onClose("open", 50000)],
            pendingSweeps: [],
            openChannelIds: ["open"],
            coopClosedChannelIds: [],
            currentHeight: 900_000
        ))
    }

    func testClosedChannelBeforeCloseTxConfirmsIsCounted() {
        XCTAssertEqual(
            ClosedChannelFunds.from(
                lightningBalances: [onClose("closed", 50000)],
                pendingSweeps: [],
                openChannelIds: [],
                coopClosedChannelIds: [],
                currentHeight: 900_000
            ),
            ClosedChannelFunds(amountSats: 50000, blocksLeft: nil)
        )
    }

    func testHolderForceCloseReportsBlocksLeft() {
        XCTAssertEqual(
            ClosedChannelFunds.from(
                lightningBalances: [awaiting(40000, 900_144, .holderForceClosed)],
                pendingSweeps: [],
                openChannelIds: [],
                coopClosedChannelIds: [],
                currentHeight: 900_000
            ),
            ClosedChannelFunds(amountSats: 40000, blocksLeft: 144)
        )
    }

    func testConfirmedCoopCloseIsNotCounted() {
        XCTAssertNil(ClosedChannelFunds.from(
            lightningBalances: [awaiting(40000, 900_005, .coopClose)],
            pendingSweeps: [],
            openChannelIds: [],
            coopClosedChannelIds: [],
            currentHeight: 900_000
        ))
    }

    func testSweepIsCountedUntilItIsBroadcast() {
        let sweeps: [PendingSweepBalance] = [
            .pendingBroadcast(channelId: "closed", amountSatoshis: 39000),
            .broadcastAwaitingConfirmation(
                channelId: "closed",
                latestBroadcastHeight: 900_010,
                latestSpendingTxid: "tx",
                amountSatoshis: 1000
            )
        ]
        XCTAssertEqual(
            ClosedChannelFunds.from(
                lightningBalances: [],
                pendingSweeps: sweeps,
                openChannelIds: [],
                coopClosedChannelIds: [],
                currentHeight: 900_020
            ),
            ClosedChannelFunds(amountSats: 39000, blocksLeft: nil, sweepSats: 39000)
        )
    }

    func testBlocksLeftNeverGoesNegative() {
        let funds = ClosedChannelFunds.from(
            lightningBalances: [awaiting(40000, 899_990, .holderForceClosed)],
            pendingSweeps: [],
            openChannelIds: [],
            coopClosedChannelIds: [],
            currentHeight: 900_000
        )
        XCTAssertEqual(funds?.blocksLeft, 0)
    }

    func testMessageShowsAmountAndTimeLeft() {
        XCTAssertEqual(
            ClosedChannelFunds(amountSats: 40000, blocksLeft: 288).message(btcPrice: 100_000),
            "$40.00 is safe on the Bitcoin blockchain. It unlocks in 288 blocks (about 2 days), "
                + "then moves to your onchain wallet automatically. Keep the app installed until it arrives."
        )
        XCTAssertEqual(
            ClosedChannelFunds(amountSats: 40000, blocksLeft: nil).message(btcPrice: 100_000),
            "$40.00 is safe on the Bitcoin blockchain. It moves to your onchain wallet automatically, "
                + "usually within a few hours. Keep the app installed until it arrives."
        )
    }

    func testBlocksTextUsesTheClosestUnit() {
        XCTAssertEqual(ClosedChannelFunds.blocksText(1), "1 block (about 10 minutes)")
        XCTAssertEqual(ClosedChannelFunds.blocksText(6), "6 blocks (about 1 hour)")
        XCTAssertEqual(ClosedChannelFunds.blocksText(7), "7 blocks (about 2 hours)")
        XCTAssertEqual(ClosedChannelFunds.blocksText(144), "144 blocks (about 1 day)")
        XCTAssertEqual(ClosedChannelFunds.blocksText(288), "288 blocks (about 2 days)")
    }

    private func from(
        _ lightning: [LightningBalance] = [],
        sweeps: [PendingSweepBalance] = [],
        open: Set<String> = [],
        coopClosed: Set<String> = []
    ) -> ClosedChannelFunds? {
        ClosedChannelFunds.from(
            lightningBalances: lightning,
            pendingSweeps: sweeps,
            openChannelIds: open,
            coopClosedChannelIds: coopClosed,
            currentHeight: 900_000
        )
    }

    /// Case A: the LSP force-closes and LDK, which still has the channel data, sweeps the funds.
    func testLspForceCloseIsCountedUntilTheSweepIsBroadcast() {
        XCTAssertNil(from([onClose("closed", 50000)], open: ["closed"]))
        // Closed, the close tx not confirmed yet.
        XCTAssertEqual(from([onClose("closed", 50000)]), ClosedChannelFunds(amountSats: 50000, blocksLeft: nil))
        // Close confirmed: our output unlocks one block later.
        XCTAssertEqual(
            from([awaiting(50000, 900_001, .counterpartyForceClosed)]),
            ClosedChannelFunds(amountSats: 50000, blocksLeft: 1)
        )
        // LDK sweeps it to the wallet.
        XCTAssertEqual(
            from(sweeps: [.pendingBroadcast(channelId: "closed", amountSatoshis: 49800)]),
            ClosedChannelFunds(amountSats: 49800, blocksLeft: nil, sweepSats: 49800)
        )
        // Sweep broadcast: the wallet counts it from its next sync.
        XCTAssertNil(from(sweeps: [.broadcastAwaitingConfirmation(
            channelId: "closed",
            latestBroadcastHeight: 900_002,
            latestSpendingTxid: "sweep",
            amountSatoshis: 49800
        )]))
        XCTAssertNil(from(sweeps: [.awaitingThresholdConfirmations(
            channelId: "closed",
            latestSpendingTxid: "sweep",
            confirmationHash: "block",
            confirmationHeight: 900_003,
            amountSatoshis: 49800
        )]))
    }

    /// Case A, closed from the app: our output is locked for the delay the LSP set (144 blocks).
    func testOwnForceCloseShowsTheTimeLeft() {
        let funds = from([awaiting(50000, 900_144, .holderForceClosed)])
        XCTAssertEqual(funds, ClosedChannelFunds(amountSats: 50000, blocksLeft: 144))
        XCTAssertEqual(
            funds?.message(btcPrice: 100_000),
            "$50.00 is safe on the Bitcoin blockchain. It unlocks in 144 blocks (about 1 day), "
                + "then moves to your onchain wallet automatically. Keep the app installed until it arrives."
        )
    }

    func testPendingHtlcsOfAClosedChannelAreCountedAsLdkCountsThem() {
        let funds = from([
            .contentiousClaimable(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 1000,
                timeoutHeight: 900_010, paymentHash: "h", paymentPreimage: "p"
            ),
            // Our own payment: the recipient may claim it.
            .maybeTimeoutClaimableHtlc(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 2000,
                claimableHeight: 900_010, paymentHash: "h", outboundPayment: true
            ),
            .maybeTimeoutClaimableHtlc(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 500,
                claimableHeight: 900_010, paymentHash: "h", outboundPayment: false
            ),
            .maybePreimageClaimableHtlc(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 3000,
                expiryHeight: 900_010, paymentHash: "h"
            ),
            .counterpartyRevokedOutputClaimable(channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 4000)
        ])
        XCTAssertEqual(funds, ClosedChannelFunds(amountSats: 5500, blocksLeft: nil))
    }

    func testPendingHtlcsOfAnOpenChannelAreNotCounted() {
        let funds = from([
            onClose("open", 50000),
            .maybeTimeoutClaimableHtlc(
                channelId: "open", counterpartyNodeId: peer, amountSatoshis: 2000,
                claimableHeight: 900_010, paymentHash: "h", outboundPayment: false
            ),
            .maybePreimageClaimableHtlc(
                channelId: "open", counterpartyNodeId: peer, amountSatoshis: 3000,
                expiryHeight: 900_010, paymentHash: "h"
            )
        ], open: ["open"])
        XCTAssertNil(funds)
    }

    func testACooperativeCloseIsNotCountedBeforeItConfirms() {
        XCTAssertNil(from([onClose("closed", 50000)], coopClosed: ["closed"]))
        XCTAssertEqual(
            from([onClose("coop", 50000), onClose("forced", 40000)], coopClosed: ["coop"]),
            ClosedChannelFunds(amountSats: 40000, blocksLeft: nil)
        )
    }

    func testOnlyCooperativeClosuresAreCooperative() {
        XCTAssertTrue(ClosedChannelFunds.isCooperative(.locallyInitiatedCooperativeClosure))
        XCTAssertTrue(ClosedChannelFunds.isCooperative(.counterpartyInitiatedCooperativeClosure))
        XCTAssertTrue(ClosedChannelFunds.isCooperative(.legacyCooperativeClosure))
        XCTAssertFalse(ClosedChannelFunds.isCooperative(.holderForceClosed(broadcastedLatestTxn: true, message: "m")))
        XCTAssertFalse(ClosedChannelFunds.isCooperative(.counterpartyForceClosed(peerMsg: "m")))
        XCTAssertFalse(ClosedChannelFunds.isCooperative(nil))
    }

    /// The total adds these to the wallet balance, so a broadcast sweep must not be among them.
    func testOnlySweepsNotYetBroadcastAreAdded() {
        let sweeps: [PendingSweepBalance] = [
            .pendingBroadcast(channelId: "a", amountSatoshis: 39000),
            .broadcastAwaitingConfirmation(
                channelId: "b",
                latestBroadcastHeight: 900_002,
                latestSpendingTxid: "sweep",
                amountSatoshis: 6188
            ),
            .awaitingThresholdConfirmations(
                channelId: "c",
                latestSpendingTxid: "sweep",
                confirmationHash: "block",
                confirmationHeight: 900_003,
                amountSatoshis: 1000
            )
        ]
        XCTAssertEqual(ClosedChannelFunds.unbroadcastSweepSats(sweeps), 39000)
    }

    func testTheLastOfSeveralUnlocksIsShown() {
        let funds = from([awaiting(30000, 900_010, .holderForceClosed), awaiting(20000, 900_144, .holderForceClosed)])
        XCTAssertEqual(funds, ClosedChannelFunds(amountSats: 50000, blocksLeft: 144))
    }

    /// A closed channel has 50,000 sats in LDK's lightning balance and a 30,000 sat sweep LDK has not
    /// broadcast. Whatever the channel state, the Home total and the card count each sat once.
    func testTheCardShowsOnlyWhatTheTotalLeavesOut() {
        let funds = ClosedChannelFunds(amountSats: 80000, blocksLeft: 10, sweepSats: 30000)
        func home(_ state: BalanceCalculator.ChannelState, openChannelSats: UInt64 = 0) -> [UInt64?] {
            let lightning = openChannelSats + 50000
            let total = BalanceCalculator.calculateTotalBalance(
                lightning: lightning,
                onchain: 20000,
                pendingSweep: 30000,
                channelState: state
            )
            let card = BalanceCalculator.closedChannelFundsLeftOut(funds, lightning: lightning, channelState: state)
            return [total, card?.amountSats]
        }

        // No channel: the total counts the sweep, the card the rest.
        XCTAssertEqual(home(.init(hasReadyChannel: false)), [50000, 50000])
        // A channel open: the total counts the lightning balance, the card the sweep.
        XCTAssertEqual(
            home(.init(hasReadyChannel: true, hasAnyChannel: true), openChannelSats: 100_000),
            [170_000, 30000]
        )
        // Closing: the total is the onchain balance alone.
        XCTAssertEqual(home(.init(hasReadyChannel: false, isChannelClosing: true)), [20000, 80000])
        // Opening, and moving onchain funds into the channel: the lightning balance alone.
        XCTAssertEqual(home(.init(hasReadyChannel: false, isOpeningChannel: true)), [50000, 30000])
        XCTAssertEqual(
            home(.init(hasReadyChannel: true, hasAnyChannel: true, isSweeping: true), openChannelSats: 100_000),
            [150_000, 30000]
        )
    }
}
