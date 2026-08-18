import XCTest
@testable import StableChannels

final class PriceServiceExtractionTests: XCTestCase {
    func testExtractPrice_objectPath() {
        let json: [String: Any] = ["data": ["amount": "63000.975"]]
        XCTAssertEqual(PriceOracle.extractPrice(from: json, path: ["data", "amount"]), 63000.975)
    }

    func testExtractPrice_arrayPath() {
        let json: [Any] = [["p": 63_100.50]]
        XCTAssertEqual(PriceOracle.extractPrice(from: json, path: ["0", "p"]), 63_100.50)
    }

    func testExtractPrice_viaPriceFeedConfig() {
        let feed = PriceFeedConfig(name: "TestFeed", urlFormat: "", jsonPath: ["0", "p"])
        let json: [Any] = [["p": 63_100.50]]
        XCTAssertEqual(feed.extractPrice(from: json), 63_100.50)
    }

    func testExtractPrice_krakenArrayValue() {
        let json: [String: Any] = [
            "result": ["XXBTZUSD": ["c": ["63029.80000", "0.01579728"]]]
        ]
        XCTAssertEqual(
            PriceOracle.extractPrice(from: json, path: ["result", "XXBTZUSD", "c"]),
            63_029.80
        )
    }

    func testExtractPrice_missingOrOutOfRangePathReturnsNil() {
        XCTAssertNil(PriceOracle.extractPrice(from: ["data": ["amount": "1"]], path: ["missing"]))
        XCTAssertNil(PriceOracle.extractPrice(from: [1, 2], path: ["2"]))
    }

    func testMedian() {
        XCTAssertNil(PriceOracle.median([]))
        XCTAssertEqual(PriceOracle.median([3, 1, 2]), 2)
        XCTAssertEqual(PriceOracle.median([4, 1, 3, 2]), 2.5)
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

    func testFreshAnchorCanProtectNotificationExtension() {
        let now = Date(timeIntervalSince1970: 1_000)
        let anchor = PriceOracleAnchor(price: 64_000, acceptedAt: now.addingTimeInterval(-60))

        XCTAssertEqual(PriceOracleAnchorStore.freshPrice(from: anchor, now: now), 64_000)
    }

    func testStaleOrFutureAnchorIsRejected() {
        let now = Date(timeIntervalSince1970: 1_000)
        let stale = PriceOracleAnchor(price: 64_000, acceptedAt: now.addingTimeInterval(-61))
        let future = PriceOracleAnchor(price: 64_000, acceptedAt: now.addingTimeInterval(1))

        XCTAssertNil(PriceOracleAnchorStore.freshPrice(from: stale, now: now))
        XCTAssertNil(PriceOracleAnchorStore.freshPrice(from: future, now: now))
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
        XCTAssertTrue(names.contains("BTCTurk BTC/USDT"))
        XCTAssertFalse(names.contains("Luno BTC/USDT"))
    }

    // MARK: - Consensus under feed loss (varying numbers of feeds down)

    /// Assert `validateBitcoinConsensus` throws `insufficientBitcoinConsensus`
    /// with the expected count of surviving agreeing feeds.
    private func assertInsufficient(_ prices: [NamedPrice], available: Int, line: UInt = #line) {
        XCTAssertThrowsError(
            try PriceOracle.validateBitcoinConsensus(prices, lastTrustedPrice: nil),
            line: line
        ) { error in
            XCTAssertEqual(
                error as? PriceOracleFailure,
                .insufficientBitcoinConsensus(available: available),
                line: line
            )
        }
    }

    func testConsensus_manyFeedsAgree_returnsMedian() throws {
        let result = try PriceOracle.validateBitcoinConsensus(
            named([64_000, 64_010, 63_990, 64_005, 63_995, 64_000]),
            lastTrustedPrice: nil
        )
        XCTAssertEqual(result.price, 64_000, accuracy: 5)
        XCTAssertEqual(result.feeds.count, 6)
    }

    func testConsensus_exactlyThreeAgreeingFeeds_succeeds() throws {
        // Quorum boundary: minimumAgreeingFeeds == 3.
        let result = try PriceOracle.validateBitcoinConsensus(
            named([64_000, 64_010, 63_990]),
            lastTrustedPrice: nil
        )
        XCTAssertEqual(result.price, 64_000, accuracy: 5)
        XCTAssertEqual(result.feeds.count, 3)
    }

    func testConsensus_twoFeeds_belowQuorum_throws() {
        assertInsufficient(named([64_000, 64_010]), available: 2)
    }

    func testConsensus_singleSurvivingFeed_isRejected() {
        // The headline guarantee: one surviving feed can NEVER set the price.
        assertInsufficient(named([64_000]), available: 1)
    }

    func testConsensus_noFeeds_throws() {
        assertInsufficient([], available: 0)
    }

    func testConsensus_outlierDoesNotCountTowardQuorum() {
        // Three feeds up, but one disagrees hard — only two agree, below quorum.
        // A wrong or manipulated feed cannot pad the count to reach consensus.
        assertInsufficient(named([64_000, 64_010, 90_000]), available: 2)
    }

    func testConsensus_outlierExcludedButQuorumStillMet() throws {
        // Four feeds up, one wild outlier; three clean feeds still agree, and the
        // median stays anchored to the cluster (the outlier never reaches it).
        let result = try PriceOracle.validateBitcoinConsensus(
            named([64_000, 64_010, 63_990, 90_000]),
            lastTrustedPrice: nil
        )
        XCTAssertEqual(result.price, 64_000, accuracy: 5)
        XCTAssertEqual(result.feeds.count, 3)
    }

    func testConsensus_agreeingFeedsOnSuspiciousMove_areQuarantined() {
        // Feeds agree, but on a value >10% from the last trusted price: quarantine,
        // do not accept (and this failure must NOT fall back to USDT).
        XCTAssertThrowsError(
            try PriceOracle.validateBitcoinConsensus(
                named([72_000, 72_010, 71_990]),
                lastTrustedPrice: 64_000
            )
        ) { error in
            guard case .largeBitcoinMove = (error as? PriceOracleFailure) else {
                return XCTFail("expected largeBitcoinMove, got \(error)")
            }
            XCTAssertTrue((error as? PriceOracleFailure)?.quarantinesPrice ?? false)
        }
    }

    func testResolve_usdTierBelowQuorum_fallsBackToUSDT() throws {
        // Only two USD feeds survive (below quorum) but the USDT tier is healthy:
        // fall back and normalize through the measured peg.
        let result = try PriceOracle.resolve(
            usdPrices: named([64_000, 64_010]),
            usdtPrices: named([64_100, 64_110, 64_090]),
            pegPrices: named([0.999, 1.000, 1.001]),
            lastTrustedPrice: nil
        )
        XCTAssertEqual(result.source, .normalizedUSDT)
        XCTAssertEqual(try XCTUnwrap(result.usdtUSD), 1.000, accuracy: 0.001)
        XCTAssertEqual(result.price, 64_100, accuracy: 100)
    }

    func testResolve_bothTiersDown_throws() {
        // One USD feed and one USDT feed: neither tier reaches quorum.
        XCTAssertThrowsError(
            try PriceOracle.resolve(
                usdPrices: named([64_000]),
                usdtPrices: named([64_100]),
                pegPrices: named([1.000, 1.000, 1.000]),
                lastTrustedPrice: nil
            )
        ) { error in
            XCTAssertEqual(
                error as? PriceOracleFailure,
                .insufficientBitcoinConsensus(available: 1)
            )
        }
    }

    func testPegGateSurvivesDirectUSDHostOutage() {
        // The USDT fallback's peg gate needs minimumAgreeingPegFeeds. If too many peg feeds
        // share hosts with the direct-USD tier, the fallback fails exactly when the primary
        // tier is unreachable — the outage it exists to survive.
        func host(_ url: String) -> String {
            URL(string: url)?.host ?? ""
        }
        let usdHosts = Set(PriceOracle.directUSDFeeds.map { host($0.urlFormat) })
        let disjoint = PriceOracle.usdtUSDFeeds.filter { !usdHosts.contains(host($0.urlFormat)) }
        // Quorum + 2 margin: a single rate-limited or flaky disjoint host must not be
        // able to drop the gate below quorum in the outage the fallback exists for.
        XCTAssertGreaterThanOrEqual(disjoint.count, PriceOracle.minimumAgreeingPegFeeds + 2)
    }
}
