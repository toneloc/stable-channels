import XCTest
@testable import StableChannels

final class QRCodeExtractorTests: XCTestCase {
    // MARK: - normalizeAddress

    func testLowercaseBech32MainnetPassesThrough() {
        let addr = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(addr), addr)
    }

    func testLowercaseBech32TestnetPassesThrough() {
        let addr = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(addr), addr)
    }

    func testLowercaseBech32RegtestPassesThrough() {
        let addr = "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(addr), addr)
    }

    func testUppercaseBech32MainnetIsLowercased() {
        XCTAssertEqual(
            QRCodeExtractor.normalizeAddress("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4"),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        )
    }

    func testUppercaseBech32TestnetIsLowercased() {
        XCTAssertEqual(
            QRCodeExtractor.normalizeAddress("TB1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KXPJZSX"),
            "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        )
    }

    func testUppercaseBech32RegtestIsLowercased() {
        XCTAssertEqual(
            QRCodeExtractor.normalizeAddress("BCRT1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KYGT080"),
            "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"
        )
    }

    func testMixedCaseBech32IsLowercased() {
        XCTAssertEqual(
            QRCodeExtractor.normalizeAddress("Bc1Qw508d6qejXTDG4Y5R3ZARVARY0C5XW7KV8F3T4"),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        )
    }

    func testBase58MainnetCasePreserved() {
        let addr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(addr), addr)
    }

    func testBase58P2SHCasePreserved() {
        let addr = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(addr), addr)
    }

    func testBase58TestnetCasePreserved() {
        let addr = "2N3oefVeg6stiTb5Kh3ozCRPgMBLCnBKE1m"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(addr), addr)
    }

    func testEmptyStringReturnsEmpty() {
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(""), "")
    }

    func testWhitespaceOnlyReturnsTrimmed() {
        XCTAssertEqual(QRCodeExtractor.normalizeAddress("   "), "")
    }

    func testWhitespaceAroundAddressIsTrimmed() {
        XCTAssertEqual(
            QRCodeExtractor.normalizeAddress("  bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4  "),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        )
    }

    func testShortStringsSafe() {
        XCTAssertEqual(QRCodeExtractor.normalizeAddress("bc"), "bc")
        XCTAssertEqual(QRCodeExtractor.normalizeAddress("b"), "b")
        XCTAssertEqual(QRCodeExtractor.normalizeAddress("1"), "1")
    }

    func testNonBech32UppercasePreserved() {
        let input = "SomeRandomString"
        XCTAssertEqual(QRCodeExtractor.normalizeAddress(input), input)
    }

    // MARK: - sanitizeAddress (bitcoin: URI strip + normalize)

    func testSanitizeAddressStripsBitcoinPrefix() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("bitcoin:bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        )
    }

    func testSanitizeAddressStripsBitcoinDoubleSlash() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("bitcoin://BC1QTEST"),
            "bc1qtest"
        )
    }

    func testSanitizeAddressCaseInsensitivePrefix() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("BITCOIN:BC1QTEST"),
            "bc1qtest"
        )
    }

    func testSanitizeAddressStripsQueryParams() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("bitcoin:BC1QTEST?amount=0.001&label=test"),
            "bc1qtest"
        )
    }

    func testSanitizeAddressPreservesBase58Case() {
        let addr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        XCTAssertEqual(QRCodeExtractor.sanitizeAddress(addr), addr)
    }

    func testSanitizeAddressHandlesMultilineInput() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("\n\nbitcoin:BC1QTEST\nsecond line\n"),
            "bc1qtest"
        )
    }

    func testSanitizeAddressHandlesNBSP() {
        // NBSP (U+00A0) is replaced with space and trimmed
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("bitcoin:\u{00A0}bc1qtest"),
            "bc1qtest"
        )
    }

    func testSanitizeAddressEmptyReturnsEmpty() {
        XCTAssertEqual(QRCodeExtractor.sanitizeAddress(""), "")
    }

    func testSanitizeAddressPlainAddressNormalized() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeAddress("BC1QTEST"),
            "bc1qtest"
        )
    }

    // MARK: - sanitizeLightningInput

    func testSanitizeLightningStripsPrefix() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeLightningInput("lightning:lnbc1pvjluezpp5"),
            "lnbc1pvjluezpp5"
        )
    }

    func testSanitizeLightningStripsDoubleSlash() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizeLightningInput("lightning://lnbc1pvjluezpp5"),
            "lnbc1pvjluezpp5"
        )
    }

    // MARK: - sanitizePaymentInput (combined bitcoin + lightning strip + normalize)

    func testSanitizePaymentInputBitcoinURI() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizePaymentInput("bitcoin:BC1QTEST?amount=0.001"),
            "bc1qtest"
        )
    }

    func testSanitizePaymentInputLightningURI() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizePaymentInput("lightning:lnbc1pvjluezpp5"),
            "lnbc1pvjluezpp5"
        )
    }

    func testSanitizePaymentInputPlainAddress() {
        XCTAssertEqual(
            QRCodeExtractor.sanitizePaymentInput("BC1QTEST"),
            "bc1qtest"
        )
    }

    func testSanitizePaymentInputBase58Preserved() {
        let addr = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        XCTAssertEqual(QRCodeExtractor.sanitizePaymentInput(addr), addr)
    }
}
