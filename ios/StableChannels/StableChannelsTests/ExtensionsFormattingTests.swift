import XCTest
@testable import StableChannels

final class ExtensionsFormattingTests: XCTestCase {
    func testSatsFormatted() {
        let sats: UInt64 = 1_234_567
        XCTAssertEqual(sats.satsFormatted, "1,234,567 sats")
        XCTAssertEqual((0 as UInt64).satsFormatted, "0 sats")
        XCTAssertEqual((1 as UInt64).satsFormatted, "1 sats")
        XCTAssertEqual((999 as UInt64).satsFormatted, "999 sats")
        XCTAssertEqual((1_000 as UInt64).satsFormatted, "1,000 sats")
        XCTAssertEqual((100_000_000 as UInt64).satsFormatted, "100,000,000 sats")
        XCTAssertEqual((2_100_000_000_000_000 as UInt64).satsFormatted, "2,100,000,000,000,000 sats")
    }

    func testUSDFormatted() {
        XCTAssertEqual(0.0.usdFormatted, "$0.00")
        XCTAssertEqual(0.05.usdFormatted, "$0.05")
        XCTAssertEqual(10.5.usdFormatted, "$10.50")
        XCTAssertEqual(1234.56.usdFormatted, "$1,234.56")
        XCTAssertEqual(1_000_000.0.usdFormatted, "$1,000,000.00")
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
        XCTAssertEqual(AppFormatters.formatSats(50_000), "50,000")
        XCTAssertEqual(AppFormatters.formatUSD(50.5), "$50.50")
        XCTAssertFalse(AppFormatters.formatShortDate(Date()).isEmpty)
    }
}
