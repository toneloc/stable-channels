import XCTest
@testable import StableChannels

final class TradeFormModelTests: XCTestCase {
    private func form(_ action: TradeAction, _ amount: String, max: Double, price: Double = 100_000) -> TradeFormModel {
        TradeFormModel(action: action, amountText: amount, maxTradeUSD: max, price: price)
    }

    func testFeeAndNetAmountFollowTheTradeFeeRate() {
        let sell = form(.sellBTC, "100", max: 500)

        XCTAssertEqual(sell.amountUSD, 100)
        XCTAssertEqual(sell.feeUSD, 100 * Constants.stableChannelTradeFeeRate, accuracy: 1e-9)
        XCTAssertEqual(sell.netAmountUSD, 100 - sell.feeUSD, accuracy: 1e-9)
        XCTAssertEqual(sell.tradedUSD, sell.netAmountUSD, "a sell credits the post-fee USD")
        XCTAssertEqual(form(.buyBTC, "100", max: 500).tradedUSD, 100, "a buy converts the whole amount")
    }

    func testBtcAmountsNeedAPrice() {
        XCTAssertEqual(form(.buyBTC, "100", max: 500).btcAmount, 0.001, accuracy: 1e-12)
        XCTAssertEqual(form(.buyBTC, "100", max: 500, price: 0).btcAmount, 0)
        XCTAssertEqual(form(.buyBTC, "100", max: 500, price: 0).btcAmountFinal, 0)
    }

    func testUnreadableAmountIsZeroAndNotAccepted() {
        let typed = form(.sellBTC, "abc", max: 500)

        XCTAssertEqual(typed.amountUSD, 0)
        XCTAssertFalse(typed.acceptsAmount)
        XCTAssertFalse(typed.exceedsLimit)
    }

    func testSellLimitIsInclusive() {
        XCTAssertTrue(form(.sellBTC, "12.34", max: 12.34).acceptsAmount)
        XCTAssertFalse(form(.sellBTC, "12.35", max: 12.34).acceptsAmount)
        XCTAssertTrue(form(.sellBTC, "12.35", max: 12.34).exceedsLimit)
    }

    func testLimitCentsAbsorbBinaryRounding() {
        XCTAssertEqual(TradeFormModel.cents(0.29), 29)
        XCTAssertEqual(form(.sellBTC, "", max: 0.29).maxTradeCents, 29)
        XCTAssertEqual(TradeFormModel.maxTradeUSD(action: .sellBTC, expectedUSD: 50, maxSellCents: 1_234), 12.34)
        XCTAssertEqual(
            TradeFormModel.maxTradeUSD(action: .buyBTC, expectedUSD: 50.129, maxSellCents: 0),
            BuyAmountPolicy.maximumUsd(50.129)
        )
    }

    @MainActor
    func testOutcomePollingRefreshesUntilTheOutcomeIsKnown() async {
        var refreshes = 0
        await TradeOutcomePolling.awaitOutcome(
            isKnown: { refreshes >= 3 },
            refresh: { refreshes += 1 },
            interval: .milliseconds(1)
        )
        XCTAssertEqual(refreshes, 3)
    }
}
