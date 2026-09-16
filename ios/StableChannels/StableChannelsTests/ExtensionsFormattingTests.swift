import XCTest
@testable import StableChannels

final class ExtensionsFormattingTests: XCTestCase {
    func testSatsFormatted() {
        let cases: [UInt64] = [0, 1, 999, 1_000, 1_234_567, 100_000_000, 2_100_000_000_000_000]
        for val in cases {
            let expectedNumber = NumberFormatter.localizedString(from: NSNumber(value: val), number: .decimal)
            XCTAssertEqual(val.satsFormatted, "\(expectedNumber) sats")
        }
    }

    func testSatsFormattedExplicitLocale() {
        let usLocale = Locale(identifier: "en_US")
        let usCases: [(UInt64, String)] = [
            (0, "0 sats"),
            (1, "1 sats"),
            (999, "999 sats"),
            (1_000, "1,000 sats"),
            (1_234_567, "1,234,567 sats"),
            (100_000_000, "100,000,000 sats"),
            (2_100_000_000_000_000, "2,100,000,000,000,000 sats")
        ]
        for (val, expected) in usCases {
            XCTAssertEqual(val.satsFormatted(locale: usLocale), expected)
        }

        let inLocale = Locale(identifier: "en_IN")
        XCTAssertEqual((1_234_567 as UInt64).satsFormatted(locale: inLocale), "12,34,567 sats")
        XCTAssertEqual((100_000_000 as UInt64).satsFormatted(locale: inLocale), "10,00,00,000 sats")
        XCTAssertEqual(
            (2_100_000_000_000_000 as UInt64).satsFormatted(locale: inLocale),
            "2,10,00,00,00,00,00,000 sats"
        )
    }

    func testUSDFormatted() {
        let cases: [Double] = [0.0, 0.05, 10.5, 1234.56, 1_000_000.0]
        let currencyFormatter = NumberFormatter()
        currencyFormatter.locale = .autoupdatingCurrent
        currencyFormatter.numberStyle = .currency
        currencyFormatter.currencyCode = "USD"
        currencyFormatter.maximumFractionDigits = 2

        for val in cases {
            let expected = currencyFormatter.string(from: NSNumber(value: val)) ?? "$0.00"
            XCTAssertEqual(val.usdFormatted, expected)
        }
    }

    func testUSDFormattedExplicitLocale() {
        let usLocale = Locale(identifier: "en_US")
        let usCases: [(Double, String)] = [
            (0.0, "$0.00"),
            (0.05, "$0.05"),
            (10.5, "$10.50"),
            (1234.56, "$1,234.56"),
            (1_000_000.0, "$1,000,000.00")
        ]
        for (val, expected) in usCases {
            XCTAssertEqual(val.usdFormatted(locale: usLocale), expected)
        }

        let inLocale = Locale(identifier: "en_IN")
        XCTAssertEqual(1_000_000.0.usdFormatted(locale: inLocale), "$10,00,000.00")
    }

    func testBtcSpacedFormatted() {
        let sats: UInt64 = 19_0079
        XCTAssertEqual(sats.btcSpacedFormatted, "0.00\u{2009}190\u{2009}079")
        XCTAssertEqual((0 as UInt64).btcSpacedFormatted, "0.00\u{2009}000\u{2009}000")
        XCTAssertEqual((1 as UInt64).btcSpacedFormatted, "0.00\u{2009}000\u{2009}001")
        XCTAssertEqual((10 as UInt64).btcSpacedFormatted, "0.00\u{2009}000\u{2009}010")
        XCTAssertEqual((100 as UInt64).btcSpacedFormatted, "0.00\u{2009}000\u{2009}100")
        XCTAssertEqual((1_000 as UInt64).btcSpacedFormatted, "0.00\u{2009}001\u{2009}000")
        XCTAssertEqual((100_000_000 as UInt64).btcSpacedFormatted, "1.00\u{2009}000\u{2009}000")
        XCTAssertEqual((123_456_789 as UInt64).btcSpacedFormatted, "1.23\u{2009}456\u{2009}789")
        XCTAssertEqual((2_100_000_000_000_000 as UInt64).btcSpacedFormatted, "21000000.00\u{2009}000\u{2009}000")
    }

    func testDateFormatting() {
        let now = Date()
        XCTAssertFalse(now.relativeString.isEmpty)
        XCTAssertFalse(now.shortString.isEmpty)
    }

    func testAppFormattersDirect() {
        let usLocale = Locale(identifier: "en_US")
        XCTAssertEqual(AppFormatters.formatSats(50_000, locale: usLocale), "50,000")
        XCTAssertEqual(AppFormatters.formatUSD(50.5, locale: usLocale), "$50.50")
        let expectedSats = NumberFormatter.localizedString(from: 50_000, number: .decimal)
        XCTAssertEqual(AppFormatters.formatSats(50_000), expectedSats)
        XCTAssertFalse(AppFormatters.formatShortDate(Date()).isEmpty)
    }

    func testCachedDateFormattersRefreshTimezone() throws {
        let originalDefault = NSTimeZone.default
        defer { NSTimeZone.default = originalDefault }

        // Initialize cached formatters in UTC
        let utc = try XCTUnwrap(TimeZone(identifier: "UTC"))
        NSTimeZone.default = utc
        let epochZero = Date(timeIntervalSince1970: 0)
        let utcShort = epochZero.shortString

        // Switch default timezone to America/Los_Angeles (UTC-8)
        let la = try XCTUnwrap(TimeZone(identifier: "America/Los_Angeles"))
        NSTimeZone.default = la
        let laShort = epochZero.shortString

        XCTAssertNotEqual(utcShort, laShort)
    }
}
