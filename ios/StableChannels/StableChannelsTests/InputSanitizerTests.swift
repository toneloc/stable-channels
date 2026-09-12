import XCTest
@testable import StableChannels

final class InputSanitizerTests: XCTestCase {
    func testDecimalSanitization() {
        XCTAssertEqual(InputSanitizer.decimal(""), "")
        XCTAssertEqual(InputSanitizer.decimal("."), "0.")
        XCTAssertEqual(InputSanitizer.decimal(".5"), "0.5")
        XCTAssertEqual(InputSanitizer.decimal("0"), "0")
        XCTAssertEqual(InputSanitizer.decimal("00"), "0")
        XCTAssertEqual(InputSanitizer.decimal("0000"), "0")
        XCTAssertEqual(InputSanitizer.decimal("00012.34"), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("0.05"), "0.05")
        XCTAssertEqual(InputSanitizer.decimal("00.05"), "0.05")
        XCTAssertEqual(InputSanitizer.decimal("0.000", maxDecimals: 3), "0.000")
        XCTAssertEqual(InputSanitizer.decimal("100."), "100.")
        XCTAssertEqual(InputSanitizer.decimal("12.3.4.5"), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("12abc.34xyz"), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("$1,234.56"), "1234.56")
        XCTAssertEqual(InputSanitizer.decimal("-42.50"), "42.50")
        XCTAssertEqual(InputSanitizer.decimal("  50.25  "), "50.25")
        XCTAssertEqual(InputSanitizer.decimal("abc!@#"), "")
        XCTAssertEqual(InputSanitizer.decimal("12.3456", maxDecimals: 2), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("12.3456", maxDecimals: 3), "12.345")
        XCTAssertEqual(InputSanitizer.decimal("12.3456", maxDecimals: 0), "12")
        XCTAssertEqual(InputSanitizer.decimal("0.123456789", maxDecimals: 8), "0.12345678")
    }
}
