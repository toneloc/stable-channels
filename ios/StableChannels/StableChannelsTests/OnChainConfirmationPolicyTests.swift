@testable import StableChannels
import XCTest

@MainActor
final class OnChainConfirmationPolicyTests: XCTestCase {
    // MARK: - Confirmation Policy Tests

    func testConfirmationPolicyThresholds() {
        XCTAssertEqual(ConfirmationPolicy.requiredConfirmations(for: "splice_in"), 1)
        XCTAssertEqual(ConfirmationPolicy.requiredConfirmations(for: "splice_out"), 1)
        XCTAssertEqual(ConfirmationPolicy.requiredConfirmations(for: "onchain"), 6)
        XCTAssertEqual(ConfirmationPolicy.requiredConfirmations(for: "channel_close"), 6)
        XCTAssertEqual(ConfirmationPolicy.requiredConfirmations(for: "unknown"), 6)
        XCTAssertEqual(ConfirmationPolicy.requiredConfirmations, 6)
        XCTAssertEqual(ConfirmationPolicy.spliceRequiredConfirmations, 1)
        XCTAssertEqual(ConfirmationPolicy.defaultRequiredConfirmations, 6)
    }

    // MARK: - Confirmation Calculator Tests

    func testCalculatorForSplice() {
        let calc = ConfirmationCalculator()
        let required = ConfirmationPolicy.requiredConfirmations(for: "splice_in")
        XCTAssertEqual(required, 1)

        // 0 confirmations (tx block height is in future or unconfirmed)
        let p0 = calc.progress(for: 101, currentBlockHeight: 100, required: required)
        XCTAssertEqual(p0.raw, 0)
        XCTAssertEqual(p0.display, 0)
        XCTAssertFalse(p0.isComplete)
        XCTAssertEqual(p0.label, "0/1 confirmed")

        // 1 confirmation (same block)
        let p1 = calc.progress(for: 100, currentBlockHeight: 100, required: required)
        XCTAssertEqual(p1.raw, 1)
        XCTAssertEqual(p1.display, 1)
        XCTAssertTrue(p1.isComplete)
        XCTAssertEqual(p1.label, "Confirmed")

        // 3 confirmations
        let p3 = calc.progress(for: 100, currentBlockHeight: 102, required: required)
        XCTAssertEqual(p3.raw, 3)
        XCTAssertEqual(p3.display, 1)
        XCTAssertTrue(p3.isComplete)
        XCTAssertEqual(p3.label, "Confirmed")
    }

    func testCalculatorForOnChain() {
        let calc = ConfirmationCalculator()
        let required = ConfirmationPolicy.requiredConfirmations(for: "onchain")
        XCTAssertEqual(required, 6)

        // 0 confirmations
        let p0 = calc.progress(for: 501, currentBlockHeight: 500, required: required)
        XCTAssertEqual(p0.display, 0)
        XCTAssertFalse(p0.isComplete)
        XCTAssertEqual(p0.label, "0/6 confirmed")

        // 1 confirmation
        let p1 = calc.progress(for: 500, currentBlockHeight: 500, required: required)
        XCTAssertEqual(p1.display, 1)
        XCTAssertFalse(p1.isComplete)
        XCTAssertEqual(p1.label, "1/6 confirmed")

        // 5 confirmations
        let p5 = calc.progress(for: 500, currentBlockHeight: 504, required: required)
        XCTAssertEqual(p5.display, 5)
        XCTAssertFalse(p5.isComplete)
        XCTAssertEqual(p5.label, "5/6 confirmed")

        // 6 confirmations
        let p6 = calc.progress(for: 500, currentBlockHeight: 505, required: required)
        XCTAssertEqual(p6.display, 6)
        XCTAssertTrue(p6.isComplete)
        XCTAssertEqual(p6.label, "Confirmed")

        // 10 confirmations
        let p10 = calc.progress(for: 500, currentBlockHeight: 509, required: required)
        XCTAssertEqual(p10.display, 6)
        XCTAssertTrue(p10.isComplete)
        XCTAssertEqual(p10.label, "Confirmed")
    }

    // MARK: - Ghost Balance Fix Tests (Issue #260)

    func testTotalBalanceWithoutReadyChannelExcludesStaleLightningBalance() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.isOpeningChannel = false
        appState.isChannelClosing = false

        // Stale closing channel lightning balance exists from LDK
        appState.lightningBalanceSats = 100_000
        appState.onchainBalanceSats = 50_000
        appState.pendingSweepBalanceSats = 10_000

        // Total should only be onchain + pending sweep
        XCTAssertEqual(appState.totalBalanceSats, 60_000)

        // Issue #260 condition: User performs Send Max after channel close
        // onchain drops to 0. Total must be 0, NEVER falling through to lightningBalanceSats (100_000)
        appState.onchainBalanceSats = 0
        appState.pendingSweepBalanceSats = 0
        XCTAssertEqual(
            appState.totalBalanceSats,
            0,
            "Stale lightning balance must not resurrect when onchain balance reaches 0"
        )
    }

    func testTotalBalanceWithReadyChannelIncludesBothBalances() {
        let appState = AppState()
        appState.hasReadyChannel = true
        appState.isOpeningChannel = false
        appState.isChannelClosing = false

        appState.lightningBalanceSats = 80_000
        appState.onchainBalanceSats = 40_000

        XCTAssertEqual(appState.totalBalanceSats, 120_000)
    }

    func testOnchainSendBroadcastedImmediateDeduction() {
        let appState = AppState()
        appState.onchainBalanceSats = 100_000
        appState.spendableOnchainSats = 95_000

        // Partial send of 30,000 sats
        appState.onchainSendBroadcasted(amountSats: 30_000, isSendAll: false)
        XCTAssertEqual(appState.onchainBalanceSats, 70_000)
        XCTAssertEqual(appState.spendableOnchainSats, 65_000)

        // Send All zeroes both
        appState.onchainSendBroadcasted(amountSats: 70_000, isSendAll: true)
        XCTAssertEqual(appState.onchainBalanceSats, 0)
        XCTAssertEqual(appState.spendableOnchainSats, 0)
    }

    func testTotalBalanceOpeningChannelIncludesLightningOrOnchain() {
        let appState = AppState()
        appState.hasReadyChannel = false
        appState.isOpeningChannel = true
        appState.isChannelClosing = false

        appState.lightningBalanceSats = 50_000
        appState.onchainBalanceSats = 0
        XCTAssertEqual(appState.totalBalanceSats, 50_000)

        appState.lightningBalanceSats = 0
        appState.onchainBalanceSats = 60_000
        XCTAssertEqual(appState.totalBalanceSats, 60_000)
    }

    // MARK: - Pure BalanceCalculator Tests (SOLID SRP & DIP)

    func testPureBalanceCalculationStaticEntryPoints() {
        // Ready channel
        let ready = AppState.calculateTotalBalance(
            lightning: 100_000,
            onchain: 50_000,
            hasReadyChannel: true
        )
        XCTAssertEqual(ready, 150_000)

        // Channel closing
        let closing = AppState.calculateTotalBalance(
            lightning: 100_000,
            onchain: 50_000,
            hasReadyChannel: false,
            isChannelClosing: true
        )
        XCTAssertEqual(closing, 50_000)

        // Sweeping
        let sweeping = AppState.calculateTotalBalance(
            lightning: 100_000,
            onchain: 50_000,
            hasReadyChannel: true,
            isSweeping: true
        )
        XCTAssertEqual(sweeping, 100_000)

        // Ghost balance: no ready channel & no channels exist
        let ghost = AppState.calculateTotalBalance(
            lightning: 100_000,
            onchain: 0,
            hasReadyChannel: false,
            hasAnyChannel: false,
            pendingSweep: 0
        )
        XCTAssertEqual(ghost, 0)

        // Pending channel (channel exists, but not ready yet)
        let pending = AppState.calculateTotalBalance(
            lightning: 80_000,
            onchain: 20_000,
            hasReadyChannel: false,
            hasAnyChannel: true
        )
        XCTAssertEqual(pending, 100_000)
    }

    func testBalanceCalculatorDirectly() {
        let state = BalanceCalculator.ChannelState(
            hasReadyChannel: false,
            hasAnyChannel: false,
            isChannelClosing: false,
            isOpeningChannel: false,
            isSweeping: false
        )
        let total = BalanceCalculator.calculateTotalBalance(
            lightning: 90_000,
            onchain: 30_000,
            pendingSweep: 5_000,
            channelState: state
        )
        XCTAssertEqual(total, 35_000)
    }
}
