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
            currentHeight: 900_000
        ))
    }

    func testClosedChannelBeforeCloseTxConfirmsIsCounted() {
        XCTAssertEqual(
            ClosedChannelFunds.from(
                lightningBalances: [onClose("closed", 50000)],
                pendingSweeps: [],
                openChannelIds: [],
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
            currentHeight: 900_000
        ))
    }

    func testSweepIsCountedUntilItConfirms() {
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
                currentHeight: 900_020
            ),
            ClosedChannelFunds(amountSats: 40000, blocksLeft: nil)
        )
    }

    func testBlocksLeftNeverGoesNegative() {
        let funds = ClosedChannelFunds.from(
            lightningBalances: [awaiting(40000, 899_990, .holderForceClosed)],
            pendingSweeps: [],
            openChannelIds: [],
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
        open: Set<String> = []
    ) -> ClosedChannelFunds? {
        ClosedChannelFunds.from(
            lightningBalances: lightning,
            pendingSweeps: sweeps,
            openChannelIds: open,
            currentHeight: 900_000
        )
    }

    /// Case A: the LSP force-closes and LDK, which still has the channel data, sweeps the funds.
    func testLspForceCloseShowsTheCardUntilTheSweepConfirms() {
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
            ClosedChannelFunds(amountSats: 49800, blocksLeft: nil)
        )
        XCTAssertEqual(
            from(sweeps: [.broadcastAwaitingConfirmation(
                channelId: "closed",
                latestBroadcastHeight: 900_002,
                latestSpendingTxid: "sweep",
                amountSatoshis: 49800
            )]),
            ClosedChannelFunds(amountSats: 49800, blocksLeft: nil)
        )
        // Sweep confirmed: the wallet shows it, so the card goes.
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

    func testPendingHtlcsOfAClosedChannelAreCounted() {
        let funds = from([
            .contentiousClaimable(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 1000,
                timeoutHeight: 900_010, paymentHash: "h", paymentPreimage: "p"
            ),
            .maybeTimeoutClaimableHtlc(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 2000,
                claimableHeight: 900_010, paymentHash: "h", outboundPayment: true
            ),
            .maybePreimageClaimableHtlc(
                channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 3000,
                expiryHeight: 900_010, paymentHash: "h"
            ),
            .counterpartyRevokedOutputClaimable(channelId: "closed", counterpartyNodeId: peer, amountSatoshis: 4000)
        ])
        XCTAssertEqual(funds, ClosedChannelFunds(amountSats: 10000, blocksLeft: nil))
    }

    /// Case B funds show only when LDK tracks no close itself.
    func testLdkFundsComeBeforeRecoveredFunds() {
        let ldk = ClosedChannelFunds(amountSats: 50000, blocksLeft: 3)
        XCTAssertEqual(ClosedChannelFunds.orRecovered(ldk, recoveredSats: 94467), ldk)
        XCTAssertEqual(
            ClosedChannelFunds.orRecovered(nil, recoveredSats: 94467),
            ClosedChannelFunds(amountSats: 94467, blocksLeft: nil)
        )
        XCTAssertNil(ClosedChannelFunds.orRecovered(nil, recoveredSats: 0))
    }
}
