import XCTest
@testable import StableChannels

final class MnemonicUtilsTests: XCTestCase {
    let valid12 = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
    let valid24 = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon art"

    func testParseMnemonic() {
        let raw = "  Abandon  ABANDON   abandon   about  "
        let parsed = MnemonicUtils.parseMnemonic(raw)
        XCTAssertEqual(parsed, ["abandon", "abandon", "abandon", "about"])
    }

    func testWordCountValidation() {
        XCTAssertTrue(MnemonicUtils.isValidWordCount(valid12))
        XCTAssertTrue(MnemonicUtils.isValidWordCount(valid24))
        XCTAssertFalse(MnemonicUtils.isValidWordCount("abandon abandon"))
        XCTAssertFalse(MnemonicUtils.isValidWordCount(""))
    }

    func testCharacterFormatValidation() {
        XCTAssertTrue(MnemonicUtils.hasValidCharacterFormat(valid12))
        XCTAssertFalse(MnemonicUtils.hasValidCharacterFormat("abandon abandon123"))
        XCTAssertFalse(MnemonicUtils.hasValidCharacterFormat("abandon ab!out"))
    }

    func testParseMnemonicWhitespaceVariations() {
        let inputWithTabsAndNewlines = "abandon\tabandon\nabandon\r\nabandon abandon  abandon   abandon\tabandon\nabandon abandon abandon about"
        let words = MnemonicUtils.parseMnemonic(inputWithTabsAndNewlines)
        XCTAssertEqual(words.count, 12)
        XCTAssertEqual(words.last, "about")

        let result = MnemonicUtils.validate(inputWithTabsAndNewlines)
        XCTAssertTrue(result.isValid)
        XCTAssertEqual(result.words.count, 12)
    }

    func testConsolidatedValidation() {
        let result12 = MnemonicUtils.validate(valid12)
        XCTAssertTrue(result12.isValid)
        XCTAssertEqual(result12.words.count, 12)

        let resultInvalid = MnemonicUtils.validate("abandon abandon 123")
        XCTAssertFalse(resultInvalid.isValid)
        XCTAssertFalse(resultInvalid.isValidWordCount)
        XCTAssertFalse(resultInvalid.hasValidCharacterFormat)
    }
}
