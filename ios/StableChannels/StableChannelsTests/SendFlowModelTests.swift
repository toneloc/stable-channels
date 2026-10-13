import LDKNode
import UIKit
import XCTest
@testable import StableChannels

@MainActor
final class SendFlowModelTests: XCTestCase {
    override func setUp() {
        super.setUp()
        if let ud = UserDefaults(suiteName: Constants.appGroupIdentifier) {
            ud.removePersistentDomain(forName: Constants.appGroupIdentifier)
        }
    }

    override func tearDown() {
        if let ud = UserDefaults(suiteName: Constants.appGroupIdentifier) {
            ud.removePersistentDomain(forName: Constants.appGroupIdentifier)
        }
        super.tearDown()
    }

    func testComputeEffectiveSatsAcrossUnits() {
        let model = SendFlowModel()
        let btcPrice: Double = 65_000

        // USD mode
        model.amountUnit = .usd
        model.amountInputText = "65.00"
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: btcPrice), 100_000)

        // Sats mode
        model.amountUnit = .sats
        model.amountInputText = "50000"
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: btcPrice), 50_000)

        // BTC mode
        model.amountUnit = .btc
        model.amountInputText = "0.001"
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: btcPrice), 100_000)
    }

    func testSwitchUnitPreservesValue() {
        let model = SendFlowModel()
        let btcPrice: Double = 65_000

        model.amountUnit = .usd
        model.amountInputText = "65.00"

        model.switchUnit(to: .sats, btcPrice: btcPrice)
        XCTAssertEqual(model.amountUnit, .sats)
        XCTAssertEqual(model.amountInputText, "100000")

        model.switchUnit(to: .btc, btcPrice: btcPrice)
        XCTAssertEqual(model.amountUnit, .btc)
        XCTAssertEqual(model.amountInputText, "0.00100000")

        model.switchUnit(to: .usd, btcPrice: btcPrice)
        XCTAssertEqual(model.amountUnit, .usd)
        XCTAssertEqual(model.amountInputText, "65.00")
    }

    func testApplyPercentageAcrossUnits() {
        let model = SendFlowModel()
        let btcPrice: Double = 65_000
        let totalSats: UInt64 = 200_000

        model.amountUnit = .sats
        model.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: btcPrice)
        XCTAssertEqual(model.amountInputText, "100000")

        model.amountUnit = .usd
        model.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: btcPrice)
        XCTAssertEqual(model.amountInputText, "65.00")

        model.amountUnit = .btc
        model.applyPercentage(50, totalBalanceSats: totalSats, btcPrice: btcPrice)
        XCTAssertEqual(model.amountInputText, "0.00100000")
    }

    func testNormalizeAmountInput() {
        let model = SendFlowModel()

        // USD normalization
        model.amountUnit = .usd
        model.amountInputText = "12"
        model.normalizeAmountInput()
        XCTAssertEqual(model.amountInputText, "12.00")

        model.amountInputText = "12.5"
        model.normalizeAmountInput()
        XCTAssertEqual(model.amountInputText, "12.50")

        model.amountInputText = "12."
        model.normalizeAmountInput()
        XCTAssertEqual(model.amountInputText, "12.00")

        model.amountInputText = ".5"
        model.normalizeAmountInput()
        XCTAssertEqual(model.amountInputText, "0.50")

        // Sats normalization
        model.amountUnit = .sats
        model.amountInputText = "0050"
        model.normalizeAmountInput()
        XCTAssertEqual(model.amountInputText, "50")

        // BTC normalization
        model.amountUnit = .btc
        model.amountInputText = ".001"
        model.normalizeAmountInput()
        XCTAssertEqual(model.amountInputText, "0.001")
    }

    func testComputeEffectiveSatsWithZeroOrNegativePrice() {
        let model = SendFlowModel()
        model.amountUnit = .usd
        model.amountInputText = "100.00"

        // Zero price
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: 0), 0)
        // Negative price
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: -50_000), 0)
    }

    func testComputeEffectiveSatsWithMassiveAmount() {
        let model = SendFlowModel()
        model.amountUnit = .sats
        model.amountInputText = "99999999999999999999999999"
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: 65_000), 0)
    }

    func testApplyPercentageWithZeroBalanceOrPrice() {
        let model = SendFlowModel()
        model.amountUnit = .usd
        model.applyPercentage(50, totalBalanceSats: 0, btcPrice: 65_000)
        XCTAssertEqual(model.amountInputText, "")

        model.applyPercentage(50, totalBalanceSats: 100_000, btcPrice: 0)
        XCTAssertEqual(model.amountInputText, "")
    }

    func testProceedFromAmountWithLNURLBounds() throws {
        let model = SendFlowModel()
        let appState = AppState()
        appState.lightningBalanceSats = 100_000
        model.destination = .lnurlPay(url: try XCTUnwrap(URL(string: "https://ln.tips/user")))
        model.lnurlParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://ln.tips/cb",
            minSendable: 1_000_000, // 1,000 sats
            maxSendable: 50_000_000, // 50,000 sats
            metadata: "[]",
            commentAllowed: nil
        )

        model.step = .amount
        model.amountUnit = .sats

        // Below minimum
        model.amountInputText = "500"
        model.proceedFromAmount(appState: appState)
        XCTAssertEqual(model.step, .amount)
        XCTAssertEqual(model.errorMessage, "Amount must be between 1000 and 50000 sats.")

        // Above maximum
        model.amountInputText = "60000"
        model.proceedFromAmount(appState: appState)
        XCTAssertEqual(model.step, .amount)
        XCTAssertEqual(model.errorMessage, "Amount must be between 1000 and 50000 sats.")

        // Exact minimum
        model.amountInputText = "1000"
        model.proceedFromAmount(appState: appState)
        XCTAssertEqual(model.step, .confirm)
        XCTAssertNil(model.errorMessage)
    }

    func testProceedFromAmount_blocksWhenBalanceIsZero() throws {
        let model = SendFlowModel()
        let appState = AppState()
        appState.lightningBalanceSats = 0
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )
        model.step = .amount
        model.amountUnit = .usd
        model.amountInputText = "15.00"

        model.proceedFromAmount(appState: appState)

        XCTAssertEqual(model.step, .amount)
        XCTAssertEqual(model.errorMessage, "Insufficient balance. Your available balance is 0 sats.")
    }

    func testProceedFromAmount_blocksWhenAmountExceedsAvailableBalance() throws {
        let model = SendFlowModel()
        let appState = AppState()
        appState.lightningBalanceSats = 5_000
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )
        model.step = .amount
        model.amountUnit = .sats
        model.amountInputText = "10000"

        model.proceedFromAmount(appState: appState)

        XCTAssertEqual(model.step, .amount)
        XCTAssertNotNil(model.errorMessage)
        XCTAssertTrue(model.errorMessage?.contains("Amount exceeds your balance") == true)
    }

    func testProceedFromAmount_allowsWhenAmountWithinBalance() throws {
        let model = SendFlowModel()
        let appState = AppState()
        appState.lightningBalanceSats = 50_000
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )
        model.step = .amount
        model.amountUnit = .sats
        model.amountInputText = "10000"

        model.proceedFromAmount(appState: appState)

        XCTAssertEqual(model.step, .confirm)
        XCTAssertNil(model.errorMessage)
    }

    func testEstimatedFeeSats_calculatesExpectedFees() {
        let appState = AppState()
        let model = SendFlowModel()

        // Onchain target
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.amountUnit = .sats
        model.amountInputText = "50000"
        model.feeRateSatVb = 10
        model.selectedFeeTier = .standard

        let onchainFee = model.estimatedFeeSats(appState: appState)
        // Standard onchain send: 140 vB * 10 sat/vB = 1400 sats
        XCTAssertEqual(onchainFee, 1_400)

        // Priority tier: 13 sat/vB -> 140 * 13 = 1820 sats
        model.selectedFeeTier = .priority
        XCTAssertEqual(model.estimatedFeeSats(appState: appState), 1_820)
    }

    func testIsInsufficientBalance_considersTotalDebitWithFee() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 0
        let model = SendFlowModel()

        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.amountUnit = .sats

        // When spendable balance is 0, any send is insufficient
        model.amountInputText = "100"
        XCTAssertTrue(model.isInsufficientBalance(appState: appState))
    }

    func testEffectiveFeeRate_tierScaling() {
        let model = SendFlowModel()
        model.feeRateSatVb = 20

        model.selectedFeeTier = .economy
        XCTAssertEqual(model.effectiveFeeRateSatVb, 16)

        model.selectedFeeTier = .standard
        XCTAssertEqual(model.effectiveFeeRateSatVb, 20)

        model.selectedFeeTier = .priority
        XCTAssertEqual(model.effectiveFeeRateSatVb, 26)
    }

    func testCalculateMaxSendableSats_onchainDeductsFee() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 10_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.feeRateSatVb = 10
        model.selectedFeeTier = .standard

        let maxSats = model.calculateMaxSendableSats(appState: appState)
        let fee = model.estimatedFeeSatsForAmount(sats: maxSats, appState: appState)
        XCTAssertEqual(maxSats + fee, 10_000)
    }

    func testCalculateMaxSendableSats_spliceOutDeductsSpliceFee() {
        let appState = AppState()
        appState.hasReadyChannel = true
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 50_000)]
        appState.lightningBalanceSats = 50_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.feeRateSatVb = 10
        model.selectedFeeTier = .standard

        let maxSats = model.calculateMaxSendableSats(appState: appState)
        let fee = model.estimatedFeeSatsForAmount(sats: maxSats, appState: appState)
        XCTAssertGreaterThan(fee, 0)
        XCTAssertEqual(fee, 10 * Constants.estimatedSpliceOutVBytes)
        XCTAssertEqual(maxSats + fee, 50_000)
    }

    func testCalculateMaxSendableSats_lightningDeductsRoutingFee() throws {
        let appState = AppState()
        appState.lightningBalanceSats = 50_000
        let model = SendFlowModel()
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )

        let maxSats = model.calculateMaxSendableSats(appState: appState)
        let fee = model.estimatedFeeSatsForAmount(sats: maxSats, appState: appState)
        // Reserves the routing-fee ceiling: 49,455 + (1% + 50 = 545) = 50,000.
        XCTAssertEqual(maxSats, 49_455)
        XCTAssertEqual(fee, 545)
    }

    func testResetFlowRestoresInitialState() {
        let model = SendFlowModel()
        model.step = .confirm
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.amountInputText = "5000"
        model.amountUnit = .sats
        model.errorMessage = "Some error"

        model.resetFlow()

        XCTAssertEqual(model.step, .recipient)
        XCTAssertEqual(model.inputText, "")
        XCTAssertNil(model.destination)
        XCTAssertEqual(model.amountInputText, "")
        XCTAssertEqual(model.amountUnit, .usd)
        XCTAssertNil(model.errorMessage)
    }

    func testResetTokenIncrementsOnEarlyReturn() async {
        let appState = AppState()
        let model = SendFlowModel()
        model.step = .confirm
        let initialToken = model.resetToken

        await model.executeSend(appState: appState)
        XCTAssertEqual(model.resetToken, initialToken + 1)
    }

    func testProceedFromAmount_preservesBIP21OnchainAmount() async {
        let appState = AppState()
        appState.spendableOnchainSats = 50_000
        let model = SendFlowModel()

        // URI specifying 1,000 sats (0.00001000 BTC)
        model.inputText = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.00001000"
        model.onInputChanged()

        guard case .onchain(let addr, let amountSats) = model.destination else {
            XCTFail("Expected .onchain destination")
            return
        }
        XCTAssertEqual(addr, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
        XCTAssertEqual(amountSats, 1_000)
        XCTAssertEqual(model.amountInputText, "1000")
        XCTAssertEqual(model.amountUnit, .sats)

        // Effective sats computed correctly from the model
        let btcPrice: Double = 50_000
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: btcPrice), 1_000)

        // Switch to USD unit preserves effective value ($0.50 at $50k/BTC)
        model.switchUnit(to: .usd, btcPrice: btcPrice)
        XCTAssertEqual(model.amountUnit, .usd)
        XCTAssertEqual(model.amountInputText, "0.50")
        XCTAssertEqual(model.computeEffectiveSats(btcPrice: btcPrice), 1_000)

        // Switch back to sats preserves 1000 sats
        model.switchUnit(to: .sats, btcPrice: btcPrice)
        XCTAssertEqual(model.amountUnit, .sats)
        XCTAssertEqual(model.amountInputText, "1000")

        // Advancing from recipient step moves to amount step
        await model.proceedFromRecipient(appState: appState)
        XCTAssertEqual(model.step, .amount)

        // Advancing from amount proceeds to confirm step
        model.proceedFromAmount(appState: appState)
        XCTAssertEqual(model.step, .confirm)
        XCTAssertNil(model.errorMessage)
    }

    func testProceedFromRecipient_failsClosedWhenNetworkUnknownForLNURL() async throws {
        let appState = AppState()
        let model = SendFlowModel() // expectedNetwork is nil by default
        model.destination = .lnurlPay(url: try XCTUnwrap(URL(string: "https://ln.tips/user")))

        await model.proceedFromRecipient(appState: appState)

        XCTAssertEqual(model.step, .recipient)
        XCTAssertEqual(model.errorMessage, "Wallet network is not initialized. Please wait until connected.")
    }

    func testExecuteSend_failsClosedWhenNetworkUnknownForLNURL() async throws {
        let appState = AppState()
        let model = SendFlowModel() // expectedNetwork is nil by default
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )
        model.step = .confirm
        model.amountInputText = "1000"
        model.amountUnit = .sats

        await model.executeSend(appState: appState)

        XCTAssertEqual(model.errorMessage, "Wallet network is not initialized. Please wait until connected.")
        XCTAssertEqual(model.resetToken, 1)
    }

    // MARK: - Fixtures

    /// A ready channel with the given outbound balance. Mirrors the
    /// fixture in StabilityServiceTests; only the fields the send flow reads are meaningful.
    private func readyChannel(outboundSats: UInt64) -> ChannelDetails {
        ChannelDetails(
            channelId: "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20",
            counterpartyNodeId: "020202020202020202020202020202020202020202020202020202020202020202",
            fundingTxo: nil,
            fundingRedeemScript: nil,
            shortChannelId: nil,
            outboundScidAlias: nil,
            inboundScidAlias: nil,
            channelValueSats: outboundSats + 100_000,
            unspendablePunishmentReserve: 1_000,
            userChannelId: "send-flow-test-chan",
            feerateSatPer1000Weight: 253,
            outboundCapacityMsat: outboundSats * 1_000,
            inboundCapacityMsat: 100_000_000,
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

    // MARK: - Review fixes

    func testExecuteSend_blocksWhenOffline() async {
        let appState = AppState()
        appState.isOnline = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.amountUnit = .sats
        model.amountInputText = "1000"
        model.feeRateSatVb = 10
        model.step = .confirm
        let initialToken = model.resetToken

        await model.executeSend(appState: appState)

        XCTAssertEqual(model.step, .confirm, "an offline send must not reach the success step")
        XCTAssertNotNil(model.errorMessage)
        XCTAssertTrue(model.errorMessage?.localizedCaseInsensitiveContains("offline") == true)
        XCTAssertEqual(model.resetToken, initialToken + 1, "the slider must snap back")
        XCTAssertFalse(model.isSending)
    }

    func testAvailableSpendableSats_onchainWithReadyChannelIsTheChannelBalance() {
        // A send to an address with a ready channel is a splice-out from that channel, so the
        // on-chain balance must not be counted as spendable for it.
        let appState = AppState()
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 20_000)]
        appState.lightningBalanceSats = 20_000
        appState.spendableOnchainSats = 30_000
        appState.isSweeping = false
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()

        XCTAssertEqual(model.availableSpendableSats(appState: appState), 20_000)
        let spliceFee = model.estimatedFeeSatsForAmount(sats: 20_000, appState: appState)
        XCTAssertEqual(model.calculateMaxSendableSats(appState: appState) + spliceFee, 20_000)

        model.amountUnit = .sats
        model.amountInputText = "25000"
        XCTAssertTrue(model.isInsufficientBalance(appState: appState))

        // With no ready channel the on-chain balance is what is spendable.
        appState.nodeService.channelsOverride = []
        model.feeRateSatVb = 10
        XCTAssertEqual(model.availableSpendableSats(appState: appState), 30_000)
    }

    func testCalculateMaxSendableSats_reservesRoutingFeeCeiling() throws {
        // The ceiling is 1% + 50 sats, rounded up: max is the largest amount with
        // max + cap(max) <= available, and max + 1 + cap(max + 1) > available.
        let appState = AppState()
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 10_000_000)]
        let model = SendFlowModel()
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )

        let cases: [(available: UInt64, max: UInt64, cap: UInt64)] = [
            (100_000, 98_960, 1_040),
            (50_000, 49_455, 545),
            (1_000, 940, 60),
            (100, 49, 51),
            (52, 1, 51),
            (51, 0, 0),
            (1, 0, 0)
        ]
        for expected in cases {
            appState.lightningBalanceSats = expected.available
            let maxSats = model.calculateMaxSendableSats(appState: appState)
            XCTAssertEqual(maxSats, expected.max, "available \(expected.available)")
            XCTAssertEqual(model.estimatedFeeSatsForAmount(sats: maxSats, appState: appState), expected.cap)
        }

        for available: UInt64 in [50_052, 50_053, 50_054, 12_345, 1_000, 999_999, 1, 2, 3] {
            appState.lightningBalanceSats = available
            let maxSats = model.calculateMaxSendableSats(appState: appState)
            let fee = model.estimatedFeeSatsForAmount(sats: maxSats, appState: appState)
            XCTAssertLessThanOrEqual(maxSats + fee, available, "available \(available): \(maxSats) + \(fee) overshoots")
            if maxSats > 0 {
                let nextFee = model.estimatedFeeSatsForAmount(sats: maxSats + 1, appState: appState)
                XCTAssertGreaterThan(
                    maxSats + 1 + nextFee,
                    available,
                    "available \(available): \(maxSats) is not maximal"
                )
            }
        }
    }

    func testLightningFee_isTheRoutingFeeCeilingRegardlessOfChannelPolicy() throws {
        let appState = AppState()
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 10_000_000)]
        let model = SendFlowModel()
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )

        XCTAssertEqual(model.estimatedFeeSatsForAmount(sats: 0, appState: appState), 0)
        XCTAssertEqual(model.estimatedFeeSatsForAmount(sats: 1, appState: appState), 51)
        XCTAssertEqual(model.estimatedFeeSatsForAmount(sats: 100_000, appState: appState), 1_050)
    }

    func testInsufficientBalance_flipsExactlyWhereAmountPlusRoutingCeilingExceedsBalance() throws {
        let appState = AppState()
        appState.lightningBalanceSats = 50_000
        let model = SendFlowModel()
        model.destination = .lightningAddress(
            handle: "alice",
            domain: "tips.net",
            url: try XCTUnwrap(URL(string: "https://tips.net"))
        )
        model.amountUnit = .sats

        // 49,455 + 545 = 50,000 fits exactly.
        model.amountInputText = "49455"
        XCTAssertFalse(model.isInsufficientBalance(appState: appState))
        model.step = .amount
        model.proceedFromAmount(appState: appState)
        XCTAssertEqual(model.step, .confirm)

        // 49,456 + 545 = 50,001 does not, although the amount alone fits.
        model.amountInputText = "49456"
        XCTAssertTrue(model.isInsufficientBalance(appState: appState))
        model.step = .amount
        model.proceedFromAmount(appState: appState)
        XCTAssertEqual(model.step, .amount)
        XCTAssertTrue(model.errorMessage?.contains("estimated fee") == true)
    }

    func testSendPaymentResult_exposesOutcomeAccessors() {
        let lightning = SendPaymentResult(sentAmountSats: 10, outcome: .lightning(paymentId: "pid"))
        XCTAssertEqual(lightning.paymentId, "pid")
        XCTAssertNil(lightning.txid)

        let onchain = SendPaymentResult(sentAmountSats: 10, outcome: .onchain(txid: "txid"))
        XCTAssertEqual(onchain.txid, "txid")
        XCTAssertNil(onchain.paymentId)

        let splice = SendPaymentResult(sentAmountSats: 10, outcome: .spliceOut)
        XCTAssertNil(splice.paymentId)
        XCTAssertNil(splice.txid)
    }

    func testProceedFromAmount_blocksWhenAmountPlusFeeExceedsBalance() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 10_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.feeRateSatVb = 10
        model.selectedFeeTier = .standard
        model.step = .amount
        model.amountUnit = .sats
        // 9,000 fits on its own, but 9,000 + 1,400 fee does not.
        model.amountInputText = "9000"

        model.proceedFromAmount(appState: appState)

        XCTAssertEqual(model.step, .amount)
        XCTAssertTrue(model.errorMessage?.contains("estimated fee") == true)
    }

    func testSuccessStatus_reflectsSettlementState() {
        let model = SendFlowModel()

        model.successTxid = "txid"
        model.isPendingSettlement = false
        XCTAssertEqual(model.successStatus, .awaitingConfirmation)

        model.successTxid = nil
        model.successPaymentId = "pid"
        XCTAssertEqual(model.successStatus, .completed)

        model.isPendingSettlement = true
        XCTAssertEqual(model.successStatus, .pending)

        model.successPaymentId = nil
        XCTAssertEqual(model.successStatus, .pending, "A splice-out has no id yet and is pending")
    }

    func testCancelProceedFromRecipient_resetsStateAndIgnoresLateFailure() async throws {
        let service = SuspendingLNURLService(delays: [.milliseconds(100)], shouldThrow: true)
        let model = SendFlowModel(expectedNetwork: .bitcoin, lnurlService: service)
        model.destination = .lnurlPay(url: try XCTUnwrap(URL(string: "https://ln.tips/user")))

        model.startProceedFromRecipient(appState: AppState())
        await waitUntil { model.isFetchingLNURL }
        model.cancelProceedFromRecipient()

        XCTAssertFalse(model.isFetchingLNURL)
        XCTAssertEqual(model.step, .recipient)
        XCTAssertNil(model.errorMessage)

        // The cancelled fetch still throws later; its error must not surface.
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertFalse(model.isFetchingLNURL)
        XCTAssertEqual(model.step, .recipient)
        XCTAssertNil(model.errorMessage)
        XCTAssertNil(model.lnurlParams)
    }

    func testStartProceedFromRecipient_ignoresDuplicateTapsAndStaleCancelledTail() async throws {
        // First fetch finishes before the second, so the cancelled tail runs while the newer fetch is in flight.
        let service = SuspendingLNURLService(delays: [.milliseconds(100), .milliseconds(600)], shouldThrow: false)
        let model = SendFlowModel(expectedNetwork: .bitcoin, lnurlService: service)
        model.destination = .lnurlPay(url: try XCTUnwrap(URL(string: "https://ln.tips/user")))
        let appState = AppState()

        model.startProceedFromRecipient(appState: appState)
        model.startProceedFromRecipient(appState: appState)
        await waitUntil { model.isFetchingLNURL }
        let firstCallCount = await service.callCount
        XCTAssertEqual(firstCallCount, 1, "A duplicate tap must not start a second fetch")

        model.cancelProceedFromRecipient()
        model.startProceedFromRecipient(appState: appState)
        await waitUntil { await service.callCount == 2 }

        // Cancelled first fetch returns params; it must neither advance nor hide the newer spinner.
        try await Task.sleep(for: .milliseconds(300))
        XCTAssertTrue(model.isFetchingLNURL)
        XCTAssertEqual(model.step, .recipient)
        XCTAssertNil(model.lnurlParams)

        await waitUntil { model.step == .amount }
        XCTAssertEqual(model.step, .amount)
        XCTAssertNotNil(model.lnurlParams)
        XCTAssertFalse(model.isFetchingLNURL)
    }

    private func waitUntil(timeout: Duration = .seconds(3), _ condition: () async -> Bool) async {
        let deadline = ContinuousClock.now + timeout
        while await !condition(), ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(10))
        }
    }

    /// Ignores cancellation and resolves after a per-call delay, so late results of cancelled fetches are exercised.
    private actor SuspendingLNURLService: LNURLServiceProtocol {
        private let delays: [Duration]
        private let shouldThrow: Bool
        private(set) var callCount = 0

        init(delays: [Duration], shouldThrow: Bool) {
            self.delays = delays
            self.shouldThrow = shouldThrow
        }

        func fetchPayParams(from _: URL) async throws -> LNURLPayParams {
            let delay = delays[min(callCount, delays.count - 1)]
            callCount += 1
            try? await Task.sleep(for: delay)
            if shouldThrow { throw LNURLError.invalidResponse }
            return LNURLPayParams(
                tag: "payRequest",
                callback: "https://ln.tips/cb",
                minSendable: 1000,
                maxSendable: 50_000_000,
                metadata: "[]",
                commentAllowed: nil
            )
        }

        func fetchInvoice(params _: LNURLPayParams, amountMsat _: UInt64,
                          comment _: String?) async throws -> LNURLPayInvoiceResponse {
            throw LNURLError.invalidResponse
        }
    }

    func testApplySettlement_timedOutStaysPendingNotCompleted() {
        let model = SendFlowModel()
        model.applySettlement(.timedOut, sentSats: 21000, paymentId: "pid")

        XCTAssertEqual(model.step, .success)
        XCTAssertEqual(model.successStatus, .pending)
        XCTAssertEqual(model.successPaymentId, "pid")
        XCTAssertEqual(model.sentAmountSats, 21000)
    }

    func testApplySettlement_settledCompletesAndFailedStaysOnStep() {
        let settled = SendFlowModel()
        settled.applySettlement(.settled, sentSats: 1000, paymentId: "pid")
        XCTAssertEqual(settled.successStatus, .completed)

        let failed = SendFlowModel()
        failed.step = .confirm
        failed.applySettlement(.failed(reason: "Route not found"), sentSats: 1000, paymentId: "pid")
        XCTAssertEqual(failed.step, .confirm)
        XCTAssertEqual(failed.errorMessage, "Route not found")
    }

    func testApplyLateSettlement_pendingReceiptFollowsOutcome() {
        let settled = SendFlowModel()
        settled.applySettlement(.timedOut, sentSats: 1000, paymentId: "pid")
        settled.applyLateSettlement(.timedOut)
        XCTAssertEqual(settled.successStatus, .pending)
        settled.applyLateSettlement(.settled)
        XCTAssertEqual(settled.step, .success)
        XCTAssertEqual(settled.successStatus, .completed)

        let failed = SendFlowModel()
        failed.applySettlement(.timedOut, sentSats: 1000, paymentId: "pid")
        failed.applyLateSettlement(.failed(reason: "Route not found"))
        XCTAssertEqual(failed.step, .confirm)
        XCTAssertEqual(failed.errorMessage, "Route not found")
        XCTAssertFalse(failed.isPendingSettlement)
    }

    func testApplyLateSettlement_ignoredWhenReceiptIsNotPending() {
        let model = SendFlowModel()
        model.applySettlement(.settled, sentSats: 1000, paymentId: "pid")
        model.applyLateSettlement(.failed(reason: "Route not found"))
        XCTAssertEqual(model.step, .success)
        XCTAssertEqual(model.successStatus, .completed)
        XCTAssertNil(model.errorMessage)
    }

    func testPaymentRecordStatus_pendingIsNeitherSettledNorFailed() {
        func record(_ status: String) -> PaymentRecord {
            PaymentRecord(
                id: 1,
                paymentId: "pid",
                paymentType: "lightning",
                direction: "sent",
                amountMsat: 1000,
                amountUSD: nil,
                btcPrice: nil,
                counterparty: nil,
                status: status,
                createdAt: 0,
                feeMsat: 0,
                txid: nil,
                address: nil,
                confirmations: 0,
                txBlockHeight: nil
            )
        }
        XCTAssertFalse(record("pending").isSettled)
        XCTAssertFalse(record("pending").isFailed)
        XCTAssertTrue(record("completed").isSettled)
        XCTAssertTrue(record("failed").isFailed)
        XCTAssertFalse(record("failed").isSettled)
    }

    func testSendOnchain_refusesWhileSweeping() async {
        let appState = AppState()
        appState.isSweeping = true
        do {
            _ = try await SendPaymentExecutor.sendOnchain(
                address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                effectiveSats: 10000,
                price: 0,
                feeRateSatVb: 5,
                appState: appState
            )
            XCTFail("An on-chain send must not dispatch while a sweep holds the wallet's UTXOs")
        } catch {
            XCTAssertEqual(error.localizedDescription, "A splice is already in progress")
        }
    }

    func testLnurlParams_cachesDownsampledPayeeAvatar() throws {
        let side: CGFloat = 1000
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        let png = UIGraphicsImageRenderer(size: CGSize(width: side, height: side), format: format)
            .pngData { context in
                UIColor.orange.setFill()
                context.fill(CGRect(x: 0, y: 0, width: side, height: side))
            }
        let metadata = "[[\"text/plain\",\"Tip\"],[\"image/png;base64\",\"\(png.base64EncodedString())\"]]"
        let model = SendFlowModel()

        model.lnurlParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/callback",
            minSendable: 1000,
            maxSendable: 1_000_000,
            metadata: metadata,
            commentAllowed: nil
        )

        let avatar = try XCTUnwrap(model.payeeAvatar)
        XCTAssertLessThanOrEqual(max(avatar.width, avatar.height), LNURLAvatar.maxPixelSize)
        model.lnurlParams = nil
        XCTAssertNil(model.payeeAvatar)
    }

    // Regtest invoice that expired one second after creation.
    private static let expiredRegtestInvoice =
        "lnbcrt500n1p4tkum6hp5uwcvgs5clswpfxhm7nyfjmaeysn6us0yvjdexn9yjkv3k7zjhp2snp4qf7cucxts7m2m34lwa78u7mv7x5kdpclmamjugwgm3jpevvwtge0jpp583gggyx4gapnxcynr0xyufahkyxthd6a9h235s904n5pmjwvxvpqsp5gz0j34zt87chh6dr0etekngg3y7snw6tnsrdtpw03vr2h5nuetdq9qyysgqcqzp2xqppd49a5hs3dpn5ssgqryd6m7utpcf6ysz4t9yhx6hl2cl4j94283m5dchu6xduq8ca4946qggzw7n28wgutuud4qga56l4lww8pkjmu9qpdkkksu"

    func testInput_rejectsInvoiceForAnotherNetwork() {
        let model = SendFlowModel(expectedNetwork: .bitcoin)
        model.inputText = Self.expiredRegtestInvoice

        XCTAssertNil(model.destination)
        guard case .invalid = model.classification else {
            return XCTFail("A regtest invoice must be rejected on a mainnet wallet")
        }
    }

    func testInput_reclassifiesWhenNetworkBecomesKnown() {
        let model = SendFlowModel()
        model.inputText = Self.expiredRegtestInvoice
        XCTAssertNotNil(model.destination)

        model.expectedNetwork = .bitcoin

        XCTAssertNil(model.destination)
        guard case .invalid = model.classification else {
            return XCTFail("Learning the network must re-check text typed before it was known")
        }
    }

    func testProceedFromRecipient_rejectsExpiredInvoice() async {
        let model = SendFlowModel()
        model.inputText = Self.expiredRegtestInvoice

        await model.proceedFromRecipient(appState: AppState())

        XCTAssertEqual(model.step, .recipient)
        XCTAssertTrue(model.errorMessage?.contains("expired") == true)
    }

    func testIsFeeRateReady_onchainWaitsForARealRate() {
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.selectedFeeTier = .standard
        XCTAssertFalse(model.isFeeRateReady, "The review must not offer a placeholder rate as the fee")

        model.feeRateSatVb = 4
        XCTAssertTrue(model.isFeeRateReady)
    }

    // MARK: - Max amount

    func testMaxAmount_onchainFollowsSelectedFeeTierAndEmptiesWallet() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.feeRateSatVb = 10
        model.applyPercentage(100, totalBalanceSats: 100_000, btcPrice: 65000, appState: appState)

        XCTAssertTrue(model.isMaxAmount)
        XCTAssertTrue(model.sendsEntireOnchainBalance(appState: appState))

        // Send-all transaction (250 vB): every tier is selectable and amount + fee is the whole balance.
        let expected: [(NetworkFeeSpeedTier, UInt64)] = [(.economy, 98000), (.standard, 97500), (.priority, 96750)]
        for (tier, amount) in expected {
            model.selectedFeeTier = tier
            XCTAssertEqual(model.sendAmountSats(appState: appState), amount, "\(tier)")
            XCTAssertEqual(
                model.sendAmountSats(appState: appState) + model.estimatedFeeSats(appState: appState),
                100_000
            )
            XCTAssertFalse(model.isInsufficientBalance(appState: appState), "\(tier)")
        }
    }

    func testMaxAmount_spliceOutFollowsSelectedFeeTier() {
        let appState = AppState()
        appState.hasReadyChannel = true
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 50000)]
        appState.lightningBalanceSats = 50000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.feeRateSatVb = 10
        model.applyPercentage(100, totalBalanceSats: 50000, btcPrice: 65000, appState: appState)

        XCTAssertFalse(model.sendsEntireOnchainBalance(appState: appState), "A splice-out is not a send-all")
        // Splice-out (180 vB) drawn from the single channel.
        model.selectedFeeTier = .standard
        XCTAssertEqual(model.sendAmountSats(appState: appState), 48200)
        model.selectedFeeTier = .priority
        XCTAssertEqual(model.sendAmountSats(appState: appState), 47660)
        XCTAssertFalse(model.isInsufficientBalance(appState: appState))
    }

    func testMaxAmount_turnedOffByOtherPercentageOrNewDestination() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.feeRateSatVb = 10

        model.applyPercentage(100, totalBalanceSats: 100_000, btcPrice: 65000, appState: appState)
        model.applyPercentage(50, totalBalanceSats: 100_000, btcPrice: 65000, appState: appState)
        XCTAssertFalse(model.isMaxAmount)

        model.applyPercentage(100, totalBalanceSats: 100_000, btcPrice: 65000, appState: appState)
        model.inputText = "bc1q9vza2e8x573nczrlzms0wvx3gsqjx7vavgkx0l"
        XCTAssertFalse(model.isMaxAmount)
    }

    func testSendAllOnchain_refusesWhileSweeping() async {
        let appState = AppState()
        appState.isSweeping = true
        do {
            _ = try await SendPaymentExecutor.sendAllOnchain(
                address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
                price: 0,
                feeRateSatVb: 5,
                appState: appState
            )
            XCTFail("Send-all spends every UTXO a sweep has committed")
        } catch {
            XCTAssertEqual(error.localizedDescription, "A splice is already in progress")
        }
    }

    func testAmountEdited_keepsMaxOnlyWhileTextIsTheMaxAmount() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.feeRateSatVb = 10
        model.amountUnit = .sats
        model.applyPercentage(100, totalBalanceSats: 100_000, btcPrice: 65000, appState: appState)

        model.amountEdited(appState: appState)
        XCTAssertTrue(model.isMaxAmount, "Max text unchanged keeps Max")

        model.amountInputText = "5000"
        model.amountEdited(appState: appState)
        XCTAssertFalse(model.isMaxAmount)
        XCTAssertEqual(model.sendAmountSats(appState: appState), 5000)
    }

    // MARK: - On-chain wallet send-all (Settings)

    func testOnchainWalletSendAll_withReadyChannelEmptiesWalletInsteadOfSplicing() {
        let appState = AppState()
        appState.hasReadyChannel = true
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 50000)]
        appState.lightningBalanceSats = 50000
        appState.spendableOnchainSats = 100_000
        appState.isSweeping = false
        let model = SendFlowModel()
        model.destination = .onchain(address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", amountSats: nil)
        model.feeRateSatVb = 10
        XCTAssertTrue(model.isSpliceOut(appState: appState), "An entered amount with a ready channel splices out")

        model.sendsAllFromOnchainWallet = true

        XCTAssertFalse(model.isMaxAmount, "The on-chain wallet send-all is not the main-flow Max")
        XCTAssertFalse(model.isSpliceOut(appState: appState))
        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 10, isSendAll: true)
        XCTAssertEqual(model.estimatedFeeSats(appState: appState), fee)
        XCTAssertEqual(model.availableSpendableSats(appState: appState), 100_000)
        XCTAssertEqual(model.totalDebitSats(appState: appState), 100_000)
        XCTAssertEqual(model.sendAmountSats(appState: appState), 100_000 - fee)
        XCTAssertFalse(model.isInsufficientBalance(appState: appState))

        model.resetFlow()
        XCTAssertFalse(model.sendsAllFromOnchainWallet)
    }

    func testIsSpliceOut_falseWhileSweepingEvenWithReadyChannel() {
        let appState = AppState()
        appState.hasReadyChannel = true
        appState.nodeService.channelsOverride = [readyChannel(outboundSats: 50000)]
        appState.isSweeping = true
        let model = SendFlowModel()

        XCTAssertFalse(model.isSpliceOut(appState: appState))

        appState.isSweeping = false
        XCTAssertTrue(model.isSpliceOut(appState: appState))
    }

    func testTotalDebitAndBaseFeeRate_withoutOnchainWalletSendAll() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        XCTAssertEqual(model.baseFeeRateSatVb, 10, "Placeholder until a rate loads")
        model.feeRateSatVb = 4
        XCTAssertEqual(model.baseFeeRateSatVb, 4)
        model.amountUnit = .sats
        model.amountInputText = "5000"

        let fee = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: 4, isSendAll: false)
        XCTAssertEqual(model.estimatedFeeSats(appState: appState), fee)
        XCTAssertEqual(model.totalDebitSats(appState: appState), 5000 + fee)
    }

    func testOnchainWalletSendAll_balanceBelowFeeSendsNothingButDebitsBalance() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100
        let model = SendFlowModel()
        model.destination = .onchain(address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", amountSats: nil)
        model.feeRateSatVb = 10
        model.sendsAllFromOnchainWallet = true

        XCTAssertGreaterThan(model.estimatedFeeSats(appState: appState), 100)
        XCTAssertEqual(model.sendAmountSats(appState: appState), 0)
        XCTAssertEqual(model.totalDebitSats(appState: appState), 100)
        XCTAssertFalse(model.isInsufficientBalance(appState: appState))
    }

    func testOnchainAmountReviewRules_amountMode() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.destination = .onchain(address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", amountSats: nil)
        model.feeRateSatVb = 10
        model.amountUnit = .sats

        model.amountInputText = ""
        XCTAssertEqual(model.displayedAmountSats(appState: appState), 0)
        XCTAssertFalse(model.exceedsAvailableAmount(appState: appState))
        XCTAssertFalse(model.canReviewOnchainAmount(appState: appState), "A zero amount cannot be reviewed")

        model.amountInputText = "5000"
        XCTAssertEqual(model.displayedAmountSats(appState: appState), 5000)
        XCTAssertFalse(model.exceedsAvailableAmount(appState: appState))
        XCTAssertTrue(model.canReviewOnchainAmount(appState: appState))

        model.amountInputText = "200000"
        XCTAssertEqual(model.displayedAmountSats(appState: appState), 200_000)
        XCTAssertTrue(model.exceedsAvailableAmount(appState: appState))
        XCTAssertFalse(model.canReviewOnchainAmount(appState: appState))
    }

    func testOnchainAmountReviewRules_sendAllMode() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.spendableOnchainSats = 100_000
        let model = SendFlowModel()
        model.destination = .onchain(address: "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq", amountSats: nil)
        model.feeRateSatVb = 10
        model.amountUnit = .sats
        model.amountInputText = "200000"
        model.sendsAllFromOnchainWallet = true

        XCTAssertEqual(model.displayedAmountSats(appState: appState), 100_000, "Send-all shows the whole balance")
        XCTAssertFalse(model.exceedsAvailableAmount(appState: appState), "A typed amount is ignored for send-all")
        XCTAssertTrue(model.canReviewOnchainAmount(appState: appState))

        appState.spendableOnchainSats = 100
        XCTAssertEqual(model.displayedAmountSats(appState: appState), 100)
        XCTAssertTrue(model.canReviewOnchainAmount(appState: appState), "Send-all is reviewable even below the fee")
    }

    // MARK: - Settlement observer (database fast path)

    private func makeAppStateWithDatabase() throws -> (AppState, DatabaseService) {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let db = try DatabaseService(dataDir: dir)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        let appState = AppState()
        appState.databaseService = db
        return (appState, db)
    }

    private func seedPayment(_ db: DatabaseService, id: String, status: String) throws {
        _ = try db.paymentRepo.recordPayment(
            paymentId: id, paymentType: "lightning", direction: "sent", amountMsat: 1_000_000,
            amountUSD: nil, btcPrice: nil, counterparty: nil, status: "pending"
        )
        if status != "pending" {
            try db.paymentRepo.updatePaymentStatus(paymentId: id, status: status)
        }
    }

    func testAwaitSettlement_returnsSettledImmediatelyWhenDatabaseRecordIsCompleted() async throws {
        let (appState, db) = try makeAppStateWithDatabase()
        try seedPayment(db, id: "pid-completed", status: "completed")

        let start = ContinuousClock.now
        let outcome = await PaymentSettlementObserver.awaitSettlement(
            paymentId: "pid-completed", timeoutSeconds: 1, appState: appState
        )
        let elapsed = start.duration(to: .now)

        guard case .settled = outcome else { return XCTFail("expected .settled, got \(outcome)") }
        XCTAssertLessThan(elapsed, .milliseconds(500), "a settled record must resolve without waiting for the timeout")
    }

    func testAwaitSettlement_returnsFailedImmediatelyWhenDatabaseRecordIsFailed() async throws {
        let (appState, db) = try makeAppStateWithDatabase()
        try seedPayment(db, id: "pid-failed", status: "failed")

        let start = ContinuousClock.now
        let outcome = await PaymentSettlementObserver.awaitSettlement(
            paymentId: "pid-failed", timeoutSeconds: 1, appState: appState
        )
        let elapsed = start.duration(to: .now)

        guard case .failed = outcome else { return XCTFail("expected .failed, got \(outcome)") }
        XCTAssertLessThan(elapsed, .milliseconds(500), "a failed record must resolve without waiting for the timeout")
    }

    func testAwaitSettlement_timesOutWhenDatabaseRecordIsStillPending() async throws {
        let (appState, db) = try makeAppStateWithDatabase()
        try seedPayment(db, id: "pid-pending", status: "pending")

        let outcome = await PaymentSettlementObserver.awaitSettlement(
            paymentId: "pid-pending", timeoutSeconds: 0.2, appState: appState
        )

        guard case .timedOut = outcome else { return XCTFail("expected .timedOut, got \(outcome)") }
    }

    // MARK: - Pre-dispatch balance guard and exact-msat pricing

    func testExecuteSend_blocksOnchainAmountAboveBalanceWithoutDispatching() async throws {
        let (appState, db) = try makeAppStateWithDatabase()
        appState.spendableOnchainSats = 10_000
        let model = SendFlowModel()
        model.inputText = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        model.onInputChanged()
        model.amountUnit = .sats
        model.amountInputText = "50000"
        model.feeRateSatVb = 10
        model.step = .confirm

        await model.executeSend(appState: appState)

        XCTAssertEqual(
            model.errorMessage,
            "Amount exceeds your balance. Available: \(UInt64(10_000).btcSpacedFormatted) BTC"
        )
        XCTAssertEqual(model.step, .confirm, "a blocked send must stay on the confirm step")
        XCTAssertFalse(model.isSending)
        let rows = try db.rawSQL.query("SELECT COUNT(*) FROM payments").first?.int64(0)
        XCTAssertEqual(rows, 0, "nothing may be recorded before the balance guard passes")
    }

    func testEstimatedFeeSats_fixedAmountBolt11UsesExactMsatNotRoundedSats() throws {
        let raw = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        let invoice = try Bolt11Invoice.fromStr(invoiceStr: raw)
        let model = SendFlowModel()
        model.destination = .bolt11(invoice: invoice, raw: raw, amountMsat: 100_000_999)

        // 100_000_999 / 100 + 50_000 = 1_050_009 msat -> rounds up to 1051 sats.
        XCTAssertEqual(model.estimatedFeeSats(appState: AppState()), 1_051)
        // Pricing the floored 100_000 sats would under-reserve by one sat.
        XCTAssertEqual(model.estimatedFeeSatsForAmount(sats: 100_000, appState: AppState()), 1_050)
    }
}
