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
            requiredSats: 40_000
        )

        XCTAssertEqual(selected?.userChannelId, "medium")
    }

    func testSelectSpliceChannel_skipsUnreadyChannels() {
        let chUnready = createChannel(id: "unready", outboundSats: 90_000, isReady: false)
        let chReady = createChannel(id: "ready", outboundSats: 30_000, isReady: true)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [chUnready, chReady],
            requiredSats: 25_000
        )

        XCTAssertEqual(selected?.userChannelId, "ready")
    }

    func testSelectSpliceChannel_fallsBackToLargestWhenNoneSufficient() {
        let ch1 = createChannel(id: "ch1", outboundSats: 20_000)
        let ch2 = createChannel(id: "ch2", outboundSats: 35_000)

        let selected = SendChannelSpendPolicy.selectSpliceChannel(
            channels: [ch1, ch2],
            requiredSats: 50_000
        )

        XCTAssertEqual(selected?.userChannelId, "ch2")
    }
}
