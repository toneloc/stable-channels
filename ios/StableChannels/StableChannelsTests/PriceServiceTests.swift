import XCTest
@testable import StableChannels

final class PriceServiceExtractionTests: XCTestCase {
    func testExtractPrice_objectPath() {
        let json: [String: Any] = ["data": ["amount": "63000.975"]]
        let result = PriceService.extractPrice(from: json, path: ["data", "amount"])
        XCTAssertNotNil(result); XCTAssertEqual(result, 63000.975)
    }

    func testExtractPrice_nestedObject() {
        let json: [String: Any] = ["bitcoin": ["usd": 63007]]
        let result = PriceService.extractPrice(from: json, path: ["bitcoin", "usd"])
        XCTAssertNotNil(result); XCTAssertEqual(result, 63007)
    }

    func testExtractPrice_numericString() throws {
        let json: [String: Any] = ["lastPrice": "63069.67000000"]
        let result = PriceService.extractPrice(from: json, path: ["lastPrice"])
        XCTAssertNotNil(result); XCTAssertEqual(try XCTUnwrap(result), 63069.67, accuracy: 0.01)
    }

    func testExtractPrice_arrayIndex() throws {
        let json: [String: Any] = ["c": ["63029.80", "0.01579728"]]
        let result = PriceService.extractPrice(from: json, path: ["c"])
        XCTAssertNotNil(result); XCTAssertEqual(try XCTUnwrap(result), 63029.80, accuracy: 0.01)
    }

    func testExtractPrice_numericArrayIndex() {
        let array: [Any] = [63074, 63081, 4.7897, -373]
        let json: [String: Any] = ["result": array]
        let result = PriceService.extractPrice(from: json, path: ["result", "6"])
        // Index 6 doesn't exist — should return nil
        XCTAssertNil(result)
    }

    func testExtractPrice_nestedArrayIndex() throws {
        let json: [String: Any] = ["data": [["last": "63100.50"]]]
        let result = PriceService.extractPrice(from: json, path: ["data", "0", "last"])
        XCTAssertNotNil(result); XCTAssertEqual(try XCTUnwrap(result), 63100.50, accuracy: 0.01)
    }

    func testExtractPrice_integerValue() {
        let json: [String: Any] = ["USD": ["last": 63044]]
        let result = PriceService.extractPrice(from: json, path: ["USD", "last"])
        XCTAssertNotNil(result); XCTAssertEqual(result, 63044)
    }

    func testExtractPrice_missingKey() {
        let json: [String: Any] = ["data": ["amount": "63000.975"]]
        let result = PriceService.extractPrice(from: json, path: ["data", "nonexistent"])
        XCTAssertNil(result)
    }

    func testExtractPrice_deepNested() throws {
        let json: [String: Any] = [
            "result": [
                "XXBTZUSD": [
                    "c": ["63029.80000", "0.01579728"]
                ]
            ]
        ]
        let result = PriceService.extractPrice(from: json, path: ["result", "XXBTZUSD", "c"])
        XCTAssertNotNil(result); XCTAssertEqual(try XCTUnwrap(result), 63029.80, accuracy: 0.01)
    }

    func testMedian_oddCount() {
        let values = [1.0, 3.0, 2.0]
        XCTAssertEqual(PriceService.median(values), 2.0)
    }

    func testMedian_evenCount() {
        let values = [1.0, 4.0, 2.0, 3.0]
        XCTAssertEqual(PriceService.median(values), 2.5)
    }

    func testMedian_empty() {
        XCTAssertEqual(PriceService.median([]), 0.0)
    }

    func testMedian_singleValue() {
        XCTAssertEqual(PriceService.median([5.0]), 5.0)
    }

    func testFilterJSON_arrayOfDicts() {
        let json: [String: Any] = [
            "data": [
                ["pair": "BTCUSDT", "last_price": "63104.01"],
                ["pair": "ETHUSDT", "last_price": "3100.50"]
            ]
        ]
        let result = PriceService.filterJSON(json, filterKey: "pair", filterValue: "BTCUSDT")
        XCTAssertEqual(result?["last_price"] as? String, "63104.01")
    }

    func testFilterJSON_nestedDict() {
        let json: [String: Any] = [
            "tickers": [
                ["pairNormalized": "BTCUSDT", "last": "63000"],
                ["pairNormalized": "ETHUSDT", "last": "3100"]
            ]
        ]
        let result = PriceService.filterJSON(json, filterKey: "pairNormalized", filterValue: "BTCUSDT")
        XCTAssertEqual(result?["last"] as? String, "63000")
    }

    func testFilterJSON_noMatch() {
        let json: [String: Any] = [
            "data": [
                ["pair": "ETHUSDT", "last_price": "3100.50"]
            ]
        ]
        let result = PriceService.filterJSON(json, filterKey: "pair", filterValue: "BTCUSDT")
        XCTAssertNil(result)
    }

    func testDefaultFeeds_nonEmpty() {
        let feeds = Constants.defaultPriceFeeds
        XCTAssertGreaterThan(feeds.count, 0)

        // All feeds must have name, urlFormat, and jsonPath
        for feed in feeds {
            XCTAssertFalse(feed.name.isEmpty)
            XCTAssertFalse(feed.urlFormat.isEmpty)
            XCTAssertFalse(feed.jsonPath.isEmpty)
        }
    }

    func testDefaultFeeds_noBitstamp() {
        let names = Constants.defaultPriceFeeds.map(\.name)
        XCTAssertFalse(names.contains("Bitstamp"), "Bitstamp should be removed (DNS down)")
    }

    func testDefaultFeeds_containsTopExchanges() {
        let names = Constants.defaultPriceFeeds.map(\.name)
        let required = ["Binance", "Kraken", "Coinbase", "Bitfinex", "Coinlore", "Bybit"]
        for exchange in required {
            XCTAssertTrue(names.contains(exchange), "Missing feed: \(exchange)")
        }
    }
}
