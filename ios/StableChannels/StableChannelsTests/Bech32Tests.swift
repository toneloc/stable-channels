import XCTest
@testable import StableChannels

final class Bech32Tests: XCTestCase {
    func testValidBech32Checksum() {
        let validVectors = [
            "A12UEL5L",
            "a12uel5l",
            "an83characterlonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1tt5tgs",
            "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw",
            "11qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqc8247j",
            "split1checkupstagehandshakeupstreamerranterredcaperred2y9e3w",
            "?1ezyfcl"
        ]

        for vector in validVectors {
            XCTAssertNotNil(
                Bech32.verifyChecksum(bechString: vector),
                "Failed to verify valid Bech32 vector: \(vector)"
            )
        }
    }

    func testValidBech32mChecksum() {
        let validBech32mVectors = [
            "A1LQFN3A",
            "a1lqfn3a",
            "an83characterlonghumanreadablepartthatcontainsthetheexcludedcharactersbioandnumber11sg7hg6",
            "abcdef1l7aum6echk45nj3s0wdvt2fg8x9yrzpqzd3ryx",
            "11llllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllludsr8",
            "split1checkupstagehandshakeupstreamerranterredcaperredlc445v",
            "?1v759aa"
        ]

        for vector in validBech32mVectors {
            XCTAssertNotNil(
                Bech32.verifyChecksum(bechString: vector),
                "Failed to verify valid Bech32m vector: \(vector)"
            )
        }
    }

    func testVerifySegwitAddress_valid() {
        let validAddresses: [(address: String, hrp: String)] = [
            ("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4", "bc"),
            ("tb1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3q0sl5k7", "tb"),
            ("bc1pw508d6qejxtdg4y5r3zarvary0c5xw7kw508d6qejxtdg4y5r3zarvary0c5xw7kt5nd6y", "bc"),
            ("BC1SW50QGDZ25J", "bc"),
            ("bc1zw508d6qejxtdg4y5r3zarvaryvaxxpcs", "bc"),
            ("tb1qqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesrxh6hy", "tb"),
            ("tb1pqqqqp399et2xygdj5xreqhjjvcmzhxw4aywxecjdzew6hylgvsesf3hn0c", "tb"),
            ("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0", "bc")
        ]

        for item in validAddresses {
            XCTAssertTrue(
                Bech32.verifySegwitAddress(item.address, expectedHrp: item.hrp),
                "Expected address to be valid: \(item.address)"
            )
        }
    }

    func testVerifySegwitAddress_invalid() {
        let invalidAddresses: [(address: String, hrp: String)] = [
            ("tc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq5zuyut", "bc"),
            ("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqh2y7hd", "bc"),
            ("tb1z0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqglt7rf", "tb"),
            ("BC1S0XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQ54WELL", "bc"),
            ("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kemeawh", "bc"),
            ("tb1q0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq24jc47", "tb"),
            ("bc1p38j9r5y49hruaue7wxjce0updqjuyyx0kh56v8s25huc6995vvpql3jow4", "bc"),
            ("BC130XLXVLHEMJA6C4DQV22UAPCTQUPFHLXM9H8Z3K2E72Q4K9HCZ7VQ7ZWS8R", "bc"),
            ("bc1pw5dgrnzv", "bc"),
            ("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7v8n0nx0muaewav253zgeav", "bc"),
            ("BC1QR508D6QEJXTDG4Y5R3ZARVARYV98GJ9P", "bc"),
            ("tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq47Zagq", "tb"),
            ("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7v07qwwzcrf", "bc"),
            ("tb1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vpggkg4j", "tb"),
            ("bc1gmk9yu", "bc")
        ]

        for item in invalidAddresses {
            XCTAssertFalse(
                Bech32.verifySegwitAddress(item.address, expectedHrp: item.hrp),
                "Expected address to be invalid: \(item.address)"
            )
        }
    }

    func testLNURLDecoding() throws {
        let sampleLNURL = "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
        let decoded = try Bech32.decodeLNURL(sampleLNURL)
        XCTAssertEqual(decoded.absoluteString, "https://service.com/api/v1/lnurl/pay")
    }

    func testLNURLDecoding_torHttpAllowed_andClearnetHttpRejected() throws {
        // Tor hidden service over HTTP is permitted
        let torLNURL = "lnurl1dp68gup69uhhxetjwe5kxefwdahxjmmw9acxz7gt0xmg5"
        let decodedTor = try Bech32.decodeLNURL(torLNURL)
        XCTAssertEqual(decodedTor.absoluteString, "http://service.onion/pay")

        // Clearnet endpoint over HTTP is rejected
        let clearnetHttpLNURL = "lnurl1dp68gup69uhhxetjwe5kxefwvdhk6tmsv9us85tvxr"
        XCTAssertThrowsError(try Bech32.decodeLNURL(clearnetHttpLNURL)) { error in
            XCTAssertEqual(error as? Bech32.Error, .insecureClearnetScheme)
        }
    }

    func testInvalidBech32Strings() {
        let invalidVectors = [
            " 1nwldj5",
            "\u{20}1nwldj5",
            "\u{7F}1axkwrx",
            "\u{80}1eym55h",
            "abc1\u{7f}23456",
            "an84characterslonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1569xm5",
            "pzry9x0s0muk",
            "1pzry9x0s0muk",
            "x1b4n0q5v",
            "li1dgmt3",
            "11llllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllllludsr7"
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
