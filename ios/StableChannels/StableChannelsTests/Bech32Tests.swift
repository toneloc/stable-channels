import XCTest
@testable import StableChannels

final class Bech32Tests: XCTestCase {
    func testValidBech32ChecksumAndDecode() throws {
        let validVectors = [
            "A12UEL5L",
            "a12uel5l",
            "an83characterlonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1tt5tgs",
            "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw",
            "11qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc8247j",
            "split1checkupstagehandshakeupstreamerranterredcaperred2y9e3w"
        ]

        for vector in validVectors {
            XCTAssertNoThrow(try Bech32.decode(vector), "Failed to decode valid Bech32 vector: \(vector)")
        }
    }

    func testLNURLDecoding() throws {
        let sampleLNURL = "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
        let decoded = try Bech32.decodeLNURL(sampleLNURL)
        XCTAssertEqual(decoded.absoluteString, "https://service.com/api/v1/lnurl/pay")
    }

    func testInvalidBech32Strings() {
        let invalidVectors = [
            " 1nwldj5",
            "abc1\u{7f}23456",
            "an84characterslonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1569xm5",
            "pzry9x0s0muk",
            "1pzry9x0s0muk",
            "x1b4n0q5v",
            "li1dgmt3"
        ]

        for vector in invalidVectors {
            XCTAssertThrowsError(try Bech32.decode(vector), "Should throw for invalid Bech32 vector: \(vector)")
        }
    }

    func testMixedCaseError() {
        XCTAssertThrowsError(try Bech32.decode("A12uel5l")) { error in
            XCTAssertEqual(error as? Bech32.Error, .mixedCase)
        }
    }
}
