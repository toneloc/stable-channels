import XCTest
@testable import StableChannels

final class ExtensionsFormattingTests: XCTestCase {
    func testSatsFormatted() {
        let sats: UInt64 = 1_234_567
        XCTAssertEqual(sats.satsFormatted, "1,234,567 sats")
    }

    func testUSDFormatted() {
        let usd = 1234.56
        XCTAssertEqual(usd.usdFormatted, "$1,234.56")
    }

    func testBtcSpacedFormatted() {
        let sats: UInt64 = 19_0079
        XCTAssertEqual(sats.btcSpacedFormatted, "0.00\u{2009}190\u{2009}079")
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
