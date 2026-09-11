import XCTest
@testable import StableChannels

final class InputSanitizerTests: XCTestCase {
    func testDecimalSanitization() {
        XCTAssertEqual(InputSanitizer.decimal(""), "")
        XCTAssertEqual(InputSanitizer.decimal("."), "0.")
        XCTAssertEqual(InputSanitizer.decimal(".5"), "0.5")
        XCTAssertEqual(InputSanitizer.decimal("0"), "0")
        XCTAssertEqual(InputSanitizer.decimal("00"), "0")
        XCTAssertEqual(InputSanitizer.decimal("00012.34"), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("0.05"), "0.05")
        XCTAssertEqual(InputSanitizer.decimal("12.3.4.5"), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("12abc.34xyz"), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("12.3456", maxDecimals: 2), "12.34")
        XCTAssertEqual(InputSanitizer.decimal("12.3456", maxDecimals: 3), "12.345")
    }
}
