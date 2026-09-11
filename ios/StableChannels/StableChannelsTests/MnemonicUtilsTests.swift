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

    func testWordCountBoundaryCases() {
        func makeWords(_ count: Int) -> String {
            Array(repeating: "abandon", count: count).joined(separator: " ")
        }
        XCTAssertFalse(MnemonicUtils.isValidWordCount(makeWords(1)))
        XCTAssertFalse(MnemonicUtils.isValidWordCount(makeWords(11)))
        XCTAssertTrue(MnemonicUtils.isValidWordCount(makeWords(12)))
        XCTAssertFalse(MnemonicUtils.isValidWordCount(makeWords(13)))
        XCTAssertFalse(MnemonicUtils.isValidWordCount(makeWords(23)))
        XCTAssertTrue(MnemonicUtils.isValidWordCount(makeWords(24)))
        XCTAssertFalse(MnemonicUtils.isValidWordCount(makeWords(25)))
    }

    func testNonAlphabeticCharacterRejection() {
        XCTAssertFalse(MnemonicUtils.isAlphabeticWord("abandon1"))
        XCTAssertFalse(MnemonicUtils.isAlphabeticWord("abandon-word"))
        XCTAssertFalse(MnemonicUtils.isAlphabeticWord("abandon_word"))
        XCTAssertFalse(MnemonicUtils.isAlphabeticWord("abandon@"))
        XCTAssertFalse(MnemonicUtils.isAlphabeticWord("abándon"))
        XCTAssertTrue(MnemonicUtils.isAlphabeticWord("abandon"))
        XCTAssertTrue(MnemonicUtils.isAlphabeticWord("ABANDON"))
    }

    func testFormatForDisplayNormalizesSpaces() {
        let messy = "  abandon\t\tabandon   \n  abandon   about  "
        XCTAssertEqual(MnemonicUtils.formatForDisplay(messy), "abandon abandon abandon about")
    }
}
