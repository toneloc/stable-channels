@testable import StableChannels
import XCTest

@MainActor
final class OnChainConfirmationPolicyTests: XCTestCase {
    override func setUp() {
        super.setUp()
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        ud?.removeObject(forKey: "pending_outbound_onchain_sats")
        ud?.removeObject(forKey: "pending_outbound_is_send_all")
        ud?.removeObject(forKey: "pending_outbound_baseline_sats")
        ud?.removeObject(forKey: "cached_onchain_sats")
        ud?.removeObject(forKey: "cached_spendable_onchain_sats")
        ud?.removeObject(forKey: "cached_lightning_sats")
    }

    override func tearDown() {
        let ud = UserDefaults(suiteName: Constants.appGroupIdentifier)
        ud?.removeObject(forKey: "pending_outbound_onchain_sats")
        ud?.removeObject(forKey: "pending_outbound_is_send_all")
        ud?.removeObject(forKey: "pending_outbound_baseline_sats")
        ud?.removeObject(forKey: "cached_onchain_sats")
        ud?.removeObject(forKey: "cached_spendable_onchain_sats")
        ud?.removeObject(forKey: "cached_lightning_sats")
        super.tearDown()
    }

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

    // MARK: - Interleaving & Pending Outbound Send Tests

    func testEffectiveBalancesDeductsPendingOutbound() {
        // Normal send: raw onchain 100k, spendable 95k, pending send 30k
        let pendingPartial = BalanceCalculator.PendingOutboundSend(
            amountSats: 30_000,
            isSendAll: false,
            baselineOnchainSats: 100_000
        )
        let effPartial = BalanceCalculator.calculateEffectiveBalances(
            rawOnchain: 100_000,
            rawSpendable: 95_000,
            pending: pendingPartial
        )
        XCTAssertEqual(effPartial.onchain, 70_000)
        XCTAssertEqual(effPartial.spendable, 65_000)

        // Send All: zeroes both balances
        let pendingAll = BalanceCalculator.PendingOutboundSend(
            amountSats: 100_000,
            isSendAll: true,
            baselineOnchainSats: 100_000
        )
        let effAll = BalanceCalculator.calculateEffectiveBalances(
            rawOnchain: 100_000,
            rawSpendable: 95_000,
            pending: pendingAll
        )
        XCTAssertEqual(effAll.onchain, 0)
        XCTAssertEqual(effAll.spendable, 0)

        // Empty pending: passes through raw balances
        let pendingNone = BalanceCalculator.PendingOutboundSend()
        let effNone = BalanceCalculator.calculateEffectiveBalances(
            rawOnchain: 100_000,
            rawSpendable: 95_000,
            pending: pendingNone
        )
        XCTAssertEqual(effNone.onchain, 100_000)
        XCTAssertEqual(effNone.spendable, 95_000)
    }

    func testResolvePendingOutboundSendIncorporation() {
        let pending = BalanceCalculator.PendingOutboundSend(
            amountSats: 30_000,
            isSendAll: false,
            baselineOnchainSats: 100_000
        )

        // Before wallet sync incorporates the spend: raw balance is still 100k
        let stillPending = BalanceCalculator.resolvePendingOutboundSend(
            rawOnchain: 100_000,
            pending: pending
        )
        XCTAssertEqual(stillPending.amountSats, 30_000, "Pending send must not clear before balance drops")
        XCTAssertFalse(stillPending.isSendAll)

        // After wallet sync incorporates the spend: raw balance dropped to 70k or less (with fees)
        let incorporated = BalanceCalculator.resolvePendingOutboundSend(
            rawOnchain: 69_850,
            pending: pending
        )
        XCTAssertEqual(incorporated.amountSats, 0, "Pending send must clear once spend is incorporated")

        // Send All incorporation
        let pendingAll = BalanceCalculator.PendingOutboundSend(
            amountSats: 100_000,
            isSendAll: true,
            baselineOnchainSats: 100_000
        )
        let allStillPending = BalanceCalculator.resolvePendingOutboundSend(
            rawOnchain: 100_000,
            pending: pendingAll
        )
        XCTAssertTrue(allStillPending.isSendAll)

        let allIncorporated = BalanceCalculator.resolvePendingOutboundSend(
            rawOnchain: 0,
            pending: pendingAll
        )
        XCTAssertFalse(allIncorporated.isSendAll)
        XCTAssertEqual(allIncorporated.amountSats, 0)
    }

    func testOnchainSendBroadcastedUpdatesPendingOutboundState() {
        let appState = AppState()
        appState.onchainBalanceSats = 100_000
        appState.spendableOnchainSats = 95_000

        // Broadcast 30,000 sats send
        appState.onchainSendBroadcasted(amountSats: 30_000, isSendAll: false)
        XCTAssertEqual(appState.onchainBalanceSats, 70_000)
        XCTAssertEqual(appState.spendableOnchainSats, 65_000)
        XCTAssertEqual(appState.pendingOutboundSend.amountSats, 30_000)
        XCTAssertEqual(appState.pendingOutboundSend.baselineOnchainSats, 100_000)
        XCTAssertFalse(appState.pendingOutboundSend.isSendAll)
    }

    func testRefreshBalancesBeforeSyncCompletesPreservesOptimisticDeduction() {
        // Simulates: broadcast send of 30,000 sats when raw balance is 100,000 sats.
        // Intervening refresh receives old 100,000 balance from LDK before sync incorporates the spend.
        // Invariant: effective balance MUST remain 70,000 and never resurrect the pre-send 100,000.
        let rawPreSend: UInt64 = 100_000
        let rawSpendablePreSend: UInt64 = 95_000
        let sendAmount: UInt64 = 30_000

        var pending = BalanceCalculator.PendingOutboundSend(
            amountSats: sendAmount,
            isSendAll: false,
            baselineOnchainSats: rawPreSend
        )

        // Intervening refresh: LDK still reports old 100k balance
        let rawDuringSync = rawPreSend
        let rawSpendableDuringSync = rawSpendablePreSend

        pending = BalanceCalculator.resolvePendingOutboundSend(rawOnchain: rawDuringSync, pending: pending)
        XCTAssertEqual(
            pending.amountSats,
            30_000,
            "Pending send deduction must not be dropped while raw balance is old"
        )

        let effectiveDuringSync = BalanceCalculator.calculateEffectiveBalances(
            rawOnchain: rawDuringSync,
            rawSpendable: rawSpendableDuringSync,
            pending: pending
        )
        XCTAssertEqual(
            effectiveDuringSync.onchain,
            70_000,
            "Balance must not resurrect to 100,000 during intervening refresh"
        )
        XCTAssertEqual(effectiveDuringSync.spendable, 65_000)

        // Once sync completes and LDK reports the incorporated balance (e.g. 69,850 with fee):
        let rawPostSync: UInt64 = 69_850
        let rawSpendablePostSync: UInt64 = 64_850

        pending = BalanceCalculator.resolvePendingOutboundSend(rawOnchain: rawPostSync, pending: pending)
        XCTAssertEqual(pending.amountSats, 0, "Pending send must clear once raw balance reflects the spend")

        let effectivePostSync = BalanceCalculator.calculateEffectiveBalances(
            rawOnchain: rawPostSync,
            rawSpendable: rawSpendablePostSync,
            pending: pending
        )
        XCTAssertEqual(effectivePostSync.onchain, 69_850)
        XCTAssertEqual(effectivePostSync.spendable, 64_850)
    }
}
