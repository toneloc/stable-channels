import Foundation
import LDKNode
@testable import StableChannels
import XCTest

final class SendChannelSpendPolicyTests: XCTestCase {
    private func createChannel(
        id: String,
        outboundSats: UInt64,
        isReady: Bool = true
    ) -> ChannelDetails {
        ChannelDetails(
            channelId: id,
            counterpartyNodeId: "020202020202020202020202020202020202020202020202020202020202020202",
            fundingTxo: nil,
            fundingRedeemScript: nil,
            shortChannelId: nil,
            outboundScidAlias: nil,
            inboundScidAlias: nil,
            channelValueSats: outboundSats + 100_000,
            unspendablePunishmentReserve: 1_000,
            userChannelId: id,
            feerateSatPer1000Weight: 253,
            outboundCapacityMsat: outboundSats * 1_000,
            inboundCapacityMsat: 100_000_000,
            confirmationsRequired: 1,
            confirmations: 6,
            isOutbound: false,
            isChannelReady: isReady,
            isUsable: isReady,
            isAnnounced: false,
            cltvExpiryDelta: 144,
            counterpartyUnspendablePunishmentReserve: 1_000,
            counterpartyOutboundHtlcMinimumMsat: 1_000,
            counterpartyOutboundHtlcMaximumMsat: 200_000_000,
            counterpartyForwardingInfoFeeBaseMsat: 1_000,
            counterpartyForwardingInfoFeeProportionalMillionths: 0,
            counterpartyForwardingInfoCltvExpiryDelta: 144,
            nextOutboundHtlcLimitMsat: outboundSats * 1_000,
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

    func testAvailableSpendableSats_lightningAggregatesChannels() throws {
        let ch1 = createChannel(id: "ch1", outboundSats: 30_000)
        let ch2 = createChannel(id: "ch2", outboundSats: 50_000)
        let testURL = try XCTUnwrap(URL(string: "https://example.com"))

        let spendable = SendChannelSpendPolicy.availableSpendableSats(
            destination: .lnurlPay(url: testURL),
            channels: [ch1, ch2],
            lightningBalanceSats: 100_000,
            onchainBalanceSats: 10_000,
            totalBalanceSats: 110_000,
            isSweeping: false
        )

        // Lightning aggregates all ready channels: 30k + 50k = 80k
        XCTAssertEqual(spendable, 80_000)
    }

    func testAvailableSpendableSats_onchainSpliceCapsToLargestSingleChannel() {
        let ch1 = createChannel(id: "ch1", outboundSats: 30_000)
        let ch2 = createChannel(id: "ch2", outboundSats: 50_000)

        let spendable = SendChannelSpendPolicy.availableSpendableSats(
            destination: .onchain(address: "bc1q...", amountSats: nil),
            channels: [ch1, ch2],
            lightningBalanceSats: 100_000,
            onchainBalanceSats: 10_000,
            totalBalanceSats: 110_000,
            isSweeping: false
        )

        // Splice-out cannot aggregate multiple channels; it must cap to max single channel (50k)
        XCTAssertEqual(spendable, 50_000)
    }

    func testAvailableSpendableSats_onchainNoReadyChannels_usesOnchainBalance() {
        let spendable = SendChannelSpendPolicy.availableSpendableSats(
            destination: .onchain(address: "bc1q...", amountSats: nil),
            channels: [],
            lightningBalanceSats: 100_000,
            onchainBalanceSats: 15_000,
            totalBalanceSats: 115_000,
            isSweeping: false
        )

        XCTAssertEqual(spendable, 15_000)
    }

    func testSelectSpliceChannel_picksBestFitChannel() {
        let chSmall = createChannel(id: "small", outboundSats: 20_000)
        let chMedium = createChannel(id: "medium", outboundSats: 45_000)
        let chLarge = createChannel(id: "large", outboundSats: 80_000)

        // Required 40,000 sats: chMedium fits and is closer than chLarge
        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [chSmall, chLarge, chMedium],
            amountSats: 40_000,
            feeRateSatVb: 0
        )

        XCTAssertEqual(selected?.userChannelId, "medium")
    }

    func testSelectSpliceChannel_skipsUnreadyChannels() {
        let chUnready = createChannel(id: "unready", outboundSats: 90_000, isReady: false)
        let chReady = createChannel(id: "ready", outboundSats: 30_000, isReady: true)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [chUnready, chReady],
            amountSats: 25_000,
            feeRateSatVb: 0
        )

        XCTAssertEqual(selected?.userChannelId, "ready")
    }

    func testSelectSpliceChannel_fallsBackToLargestWhenNoneSufficient() {
        let ch1 = createChannel(id: "ch1", outboundSats: 20_000)
        let ch2 = createChannel(id: "ch2", outboundSats: 35_000)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [ch1, ch2],
            amountSats: 50_000,
            feeRateSatVb: 0
        )

        XCTAssertEqual(selected?.userChannelId, "ch2")
    }

    func testSelectSpliceChannel_exactCapacityMatchIsSufficient() {
        let small = createChannel(id: "small", outboundSats: 20000)
        let exact = createChannel(id: "exact", outboundSats: 40000)
        let large = createChannel(id: "large", outboundSats: 80000)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [small, large, exact],
            amountSats: 40000,
            feeRateSatVb: 0
        )

        XCTAssertEqual(selected?.userChannelId, "exact")
    }

    func testSelectSpliceChannel_withAmount_requiresFeeHeadroom() {
        let amount: UInt64 = 40000
        let fee = PaymentFeeEstimator.estimateSpliceOutFee(feeRateSatVb: 10)
        let short = createChannel(id: "short", outboundSats: amount + fee - 1)
        let large = createChannel(id: "large", outboundSats: amount + fee + 10000)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [short, large],
            amountSats: amount,
            feeRateSatVb: 10
        )

        XCTAssertEqual(
            selected?.userChannelId,
            "large",
            "A channel one sat short of amount + fee cannot fund the splice"
        )
    }

    func testSelectSpliceChannel_withAmount_exactAmountPlusFeeIsBestFit() {
        let amount: UInt64 = 40000
        let fee = PaymentFeeEstimator.estimateSpliceOutFee(feeRateSatVb: 10)
        let exact = createChannel(id: "exact", outboundSats: amount + fee)
        let large = createChannel(id: "large", outboundSats: amount + fee + 10000)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [large, exact],
            amountSats: amount,
            feeRateSatVb: 10
        )

        XCTAssertEqual(selected?.userChannelId, "exact")
    }

    func testAvailableOnchainSpendableSats_spliceOutIsBoundedByLargestChannel() {
        let channels = [
            createChannel(id: "a", outboundSats: 30000),
            createChannel(id: "b", outboundSats: 50000)
        ]

        let splice = SendChannelSpendPolicy.availableOnchainSpendableSats(
            channels: channels,
            lightningBalanceSats: 80000,
            onchainBalanceSats: 7000,
            isSweeping: false
        )
        XCTAssertEqual(splice, 50000, "A splice-out draws on one channel, not the 80,000 sat total")

        let sweeping = SendChannelSpendPolicy.availableOnchainSpendableSats(
            channels: channels,
            lightningBalanceSats: 80000,
            onchainBalanceSats: 7000,
            isSweeping: true
        )
        XCTAssertEqual(sweeping, 7000)

        let noChannels = SendChannelSpendPolicy.availableOnchainSpendableSats(
            channels: [],
            lightningBalanceSats: 80000,
            onchainBalanceSats: 7000,
            isSweeping: false
        )
        XCTAssertEqual(noChannels, 7000)
    }

    func testCalculateMaxSendable_principalCapBoundsAmountNotFee() {
        let fee: (UInt64) -> UInt64 = { $0 / 100 + 50 }
        // Cap fits with its fee inside the balance: the cap itself.
        XCTAssertEqual(SendChannelSpendPolicy.calculateMaxSendable(
            available: 100_000,
            principalCap: 50_000,
            feeEstimator: fee
        ), 50_000)
        // Balance binds first: the fee-inclusive max, unaffected by the cap.
        XCTAssertEqual(SendChannelSpendPolicy.calculateMaxSendable(
            available: 30_000,
            principalCap: 50_000,
            feeEstimator: fee
        ), SendChannelSpendPolicy.calculateMaxSendable(available: 30_000, feeEstimator: fee))
    }

    func testIsOnchainSendAll_onlyForMaxOnchainWithoutSpliceOut() {
        let onchain = SendDestination.onchain(address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", amountSats: nil)
        let ready = [createChannel(id: "a", outboundSats: 50000)]

        XCTAssertTrue(SendChannelSpendPolicy.isOnchainSendAll(
            isMax: true,
            destination: onchain,
            channels: [],
            isSweeping: false
        ))
        XCTAssertFalse(SendChannelSpendPolicy.isOnchainSendAll(
            isMax: false,
            destination: onchain,
            channels: [],
            isSweeping: false
        ))
        XCTAssertFalse(
            SendChannelSpendPolicy
                .isOnchainSendAll(isMax: true, destination: onchain, channels: ready, isSweeping: false),
            "A splice-out Max draws on one channel"
        )
        XCTAssertTrue(SendChannelSpendPolicy.isOnchainSendAll(
            isMax: true,
            destination: onchain,
            channels: ready,
            isSweeping: true
        ))
        XCTAssertFalse(SendChannelSpendPolicy.isOnchainSendAll(
            isMax: true,
            destination: nil,
            channels: [],
            isSweeping: false
        ))
    }
}
