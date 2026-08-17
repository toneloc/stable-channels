import XCTest
@testable import StableChannels

final class PriceServiceExtractionTests: XCTestCase {
    func testExtractPrice_objectPath() {
        let json: [String: Any] = ["data": ["amount": "63000.975"]]
        XCTAssertEqual(PriceService.extractPrice(from: json, path: ["data", "amount"]), 63000.975)
    }

    func testExtractPrice_arrayPath() {
        let json: [Any] = [["p": 63_100.50]]
        XCTAssertEqual(PriceService.extractPrice(from: json, path: ["0", "p"]), 63_100.50)
    }

    func testExtractPrice_krakenArrayValue() {
        let json: [String: Any] = [
            "result": ["XXBTZUSD": ["c": ["63029.80000", "0.01579728"]]]
        ]
        XCTAssertEqual(
            PriceService.extractPrice(from: json, path: ["result", "XXBTZUSD", "c"]),
            63_029.80
        )
    }

    func testExtractPrice_missingOrOutOfRangePathReturnsNil() {
        XCTAssertNil(PriceService.extractPrice(from: ["data": ["amount": "1"]], path: ["missing"]))
        XCTAssertNil(PriceService.extractPrice(from: [1, 2], path: ["2"]))
    }

    func testMedian() {
        XCTAssertEqual(PriceService.median([]), 0)
        XCTAssertEqual(PriceService.median([3, 1, 2]), 2)
        XCTAssertEqual(PriceService.median([4, 1, 3, 2]), 2.5)
    }
}

final class PriceOracleTests: XCTestCase {
    private func named(_ values: [Double], prefix: String = "feed") -> [NamedPrice] {
        values.enumerated().map { NamedPrice(feedName: "\(prefix)-\($0.offset)", value: $0.element) }
    }

    func testDirectUSDConsensusIsPreferred() throws {
        let result = try PriceOracle.resolve(
            usdPrices: named([64_000, 64_050, 63_950]),
            usdtPrices: named([80_000, 80_100, 79_900], prefix: "usdt"),
            pegPrices: named([1.0, 1.0, 1.0], prefix: "peg"),
            lastTrustedPrice: 64_000
        )

        XCTAssertEqual(result.source, .directUSD)
        XCTAssertEqual(result.price, 64_000)
        XCTAssertNil(result.usdtUSD)
    }

    func testUSDTFallbackUsesMeasuredPeg() throws {
        let result = try PriceOracle.resolve(
            usdPrices: named([64_000, 64_050]),
            usdtPrices: named([64_064, 64_074, 64_054], prefix: "usdt"),
            pegPrices: named([0.999, 0.9991, 0.9989], prefix: "peg"),
            lastTrustedPrice: 64_000
        )

        XCTAssertEqual(result.source, .normalizedUSDT)
        XCTAssertEqual(try XCTUnwrap(result.usdtUSD), 0.999, accuracy: 0.000_001)
        XCTAssertEqual(result.price, 63_999.936, accuracy: 1.0)
    }

    func testUSDTFallbackRejectsDepeg() {
        XCTAssertThrowsError(
            try PriceOracle.resolve(
                usdPrices: named([64_000]),
                usdtPrices: named([64_500, 64_520, 64_480], prefix: "usdt"),
                pegPrices: named([0.98, 0.981, 0.979], prefix: "peg"),
                lastTrustedPrice: 64_000
            )
        ) { error in
            guard case PriceOracleFailure.invalidUSDTPeg = error else {
                return XCTFail("Unexpected error: \(error)")
            }
        }
    }

    func testUSDTFallbackRequiresThreePegFeeds() {
        XCTAssertThrowsError(
            try PriceOracle.resolve(
                usdPrices: [],
                usdtPrices: named([64_000, 64_010, 63_990], prefix: "usdt"),
                pegPrices: named([1.0, 1.0], prefix: "peg"),
                lastTrustedPrice: nil
            )
        )
    }

    func testConsensusRejectsSingleSurvivingFeed() {
        XCTAssertThrowsError(
            try PriceOracle.validateBitcoinConsensus(named([64_000]), lastTrustedPrice: nil)
        )
    }

    func testConsensusDropsOutlier() throws {
        let result = try PriceOracle.validateBitcoinConsensus(
            named([64_000, 64_100, 63_900, 500_000]),
            lastTrustedPrice: 64_000
        )
        XCTAssertEqual(result.feeds.count, 3)
        XCTAssertEqual(result.price, 64_000)
    }

    func testLargeMoveQuarantinesPrice() {
        XCTAssertThrowsError(
            try PriceOracle.validateBitcoinConsensus(
                named([80_000, 80_100, 79_900]),
                lastTrustedPrice: 64_000
            )
        ) { error in
            guard let failure = error as? PriceOracleFailure else {
                return XCTFail("Unexpected error: \(error)")
            }
            XCTAssertTrue(failure.quarantinesPrice)
        }
    }

    func testPrimaryFeedListContainsOnlySixDirectUSDMarkets() {
        XCTAssertEqual(PriceOracle.directUSDFeeds.count, 6)
        XCTAssertEqual(
            Set(PriceOracle.directUSDFeeds.map(\.name)),
            Set(["Bitstamp", "Kraken", "Coinbase", "Bitfinex", "Gemini", "Bullish"])
        )
        XCTAssertFalse(PriceOracle.directUSDFeeds.contains { $0.urlFormat.uppercased().contains("USDT") })
    }

    func testFallbackIncludesRegionalSinglePairFeeds() {
        let names = Set(PriceOracle.bitcoinUSDTFeeds.map(\.name))
        XCTAssertTrue(names.contains("CoinDCX BTC/USDT"))
        XCTAssertTrue(names.contains("Luno BTC/USDT"))
        XCTAssertTrue(names.contains("BTCTurk BTC/USDT"))
    }
}
