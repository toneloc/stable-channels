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
        XCTAssertLessThanOrEqual(maxSats + fee, 50_000)
        XCTAssertGreaterThan(maxSats, 49_000)
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
}
