import XCTest
@testable import StableChannels

final class PaymentDestinationClassifierTests: XCTestCase {
    func testClassifyLightningAddress() {
        let input = "alice@lightning.example.com"
        let result = PaymentDestinationClassifier.classify(input)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination")
            return
        }

        guard case .lightningAddress(let handle, let domain, let url) = dest else {
            XCTFail("Expected .lightningAddress")
            return
        }

        XCTAssertEqual(handle, "alice")
        XCTAssertEqual(domain, "lightning.example.com")
        XCTAssertEqual(url.absoluteString, "https://lightning.example.com/.well-known/lnurlp/alice")
    }

    func testClassifyOnchainAddress() {
        let input = "bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n"
        let result = PaymentDestinationClassifier.classify(input)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination")
            return
        }

        guard case .onchain(let addr) = dest else {
            XCTFail("Expected .onchain")
            return
        }

        XCTAssertEqual(addr, input)
    }

    func testClassifyBIP21WithOnchainAddress() {
        let input = "bitcoin:bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n"
        let result = PaymentDestinationClassifier.classify(input)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination")
            return
        }

        guard case .onchain(let addr) = dest else {
            XCTFail("Expected .onchain")
            return
        }

        XCTAssertEqual(addr, "bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n")
    }

    func testClassifyEmptyInput() {
        let result = PaymentDestinationClassifier.classify("   \n ")
        XCTAssertEqual(result, .empty)
    }

    func testClassifyInvalidInput() {
        let result = PaymentDestinationClassifier.classify("random_garbage_string")
        guard case .invalid = result else {
            XCTFail("Expected invalid result")
            return
        }
    }

    func testClassifyBIP21WithLightningFallbackPrioritizesLightning() {
        let invoiceStr = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        let bip21 = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.001&lightning=\(invoiceStr)"
        let result = PaymentDestinationClassifier.classify(bip21)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination from BIP21")
            return
        }
        guard case .bolt11 = dest else {
            XCTFail("Expected .bolt11 to take priority over onchain fallback in BIP21")
            return
        }
    }

    func testClassifyBIP21UppercaseScheme() {
        let bip21 = "BITCOIN:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        let result = PaymentDestinationClassifier.classify(bip21)
        guard case .valid(let dest) = result, case .onchain(let addr) = dest else {
            XCTFail("Expected valid onchain destination from uppercase BITCOIN: URI")
            return
        }
        XCTAssertEqual(addr, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
    }

    func testClassifyLightningPrefixVariations() {
        let invoiceStr = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"

        let withColon = "lightning:\(invoiceStr)"
        let withDoubleSlash = "lightning://\(invoiceStr)"

        guard case .valid(let destColon) = PaymentDestinationClassifier.classify(withColon),
              case .bolt11 = destColon else {
            XCTFail("Expected valid bolt11 from lightning: prefix")
            return
        }

        guard case .valid(let destSlash) = PaymentDestinationClassifier.classify(withDoubleSlash),
              case .bolt11 = destSlash else {
            XCTFail("Expected valid bolt11 from lightning:// prefix")
            return
        }
    }

    func testClassifyLNURLPayBech32() {
        let lnurlStr = "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
        let result = PaymentDestinationClassifier.classify(lnurlStr)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid LNURL destination")
            return
        }
        guard case .lnurlPay(let url) = dest else {
            XCTFail("Expected .lnurlPay")
            return
        }
        XCTAssertEqual(url.host, "service.com")
    }

    func testClassifyLegacyAndTestnetOnchainAddresses() {
        let p2pkh = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        let p2sh = "3J98t1WpEZ73CNmQviecrnyiWrnqRhWNLy"
        let testnet = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        let regtest = "bcrt1qw508d6qejxtdg4y5r3zarvary0c5xw7kygt080"

        XCTAssertEqual(PaymentDestinationClassifier.classify(p2pkh), .valid(.onchain(address: p2pkh)))
        XCTAssertEqual(PaymentDestinationClassifier.classify(p2sh), .valid(.onchain(address: p2sh)))
        XCTAssertEqual(PaymentDestinationClassifier.classify(testnet), .valid(.onchain(address: testnet)))
        XCTAssertEqual(PaymentDestinationClassifier.classify(regtest), .valid(.onchain(address: regtest)))
    }

    func testClassifyInvalidAddressLengthsAndCharacters() {
        let tooShort = "1A1zP1eP5QGefi2DMPTfTL5SL"
        guard case .invalid = PaymentDestinationClassifier.classify(tooShort) else {
            XCTFail("Expected invalid for address under 26 characters")
            return
        }

        let tooLong = "bc1" + String(repeating: "q", count: 88)
        guard case .invalid = PaymentDestinationClassifier.classify(tooLong) else {
            XCTFail("Expected invalid for address over 90 characters")
            return
        }

        let nonAlphanumeric = "bc1qar0srrr7xfkvy5l643!@#$dnw9re59gtzzwf5mdq"
        guard case .invalid = PaymentDestinationClassifier.classify(nonAlphanumeric) else {
            XCTFail("Expected invalid for non-alphanumeric address")
            return
        }
    }

    func testClassifyLightningAddressEdgeCases() {
        let plusAddr = "alice+tips@lightning.example.com"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(plusAddr),
              case .lightningAddress(let handle, let domain, _) = dest else {
            XCTFail("Expected valid lightning address with plus tag")
            return
        }
        XCTAssertEqual(handle, "alice+tips")
        XCTAssertEqual(domain, "lightning.example.com")

        guard case .invalid = PaymentDestinationClassifier.classify("alice @lightning.example.com") else {
            XCTFail("Expected invalid for address with spaces")
            return
        }

        guard case .invalid = PaymentDestinationClassifier.classify("@lightning.example.com") else {
            XCTFail("Expected invalid for missing handle")
            return
        }

        guard case .invalid = PaymentDestinationClassifier.classify("alice@") else {
            XCTFail("Expected invalid for missing domain")
            return
        }
    }
}
