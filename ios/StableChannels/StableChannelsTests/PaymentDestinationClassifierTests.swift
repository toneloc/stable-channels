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
        XCTAssertTrue(dest.requiresManualAmount)
    }

    func testClassifyOnchainAddress() {
        let input = "bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n"
        let result = PaymentDestinationClassifier.classify(input)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination")
            return
        }

        guard case .onchain(let addr, let amountSats) = dest else {
            XCTFail("Expected .onchain")
            return
        }

        XCTAssertEqual(addr, input)
        XCTAssertNil(amountSats)
        XCTAssertTrue(dest.requiresManualAmount)
    }

    func testClassifyBIP21WithOnchainAddressWithoutAmount() {
        let input = "bitcoin:bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n"
        let result = PaymentDestinationClassifier.classify(input)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination")
            return
        }

        guard case .onchain(let addr, let amountSats) = dest else {
            XCTFail("Expected .onchain")
            return
        }

        XCTAssertEqual(addr, "bc1plvty62zgr8ae8vat0xyqrae0qgnlmcra5nnhpamhn46kdjuwh43q0wes3n")
        XCTAssertNil(amountSats)
        XCTAssertTrue(dest.requiresManualAmount)
    }

    func testClassifyBIP21PreservesAmountInSatoshis() {
        let uri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.001"
        let result = PaymentDestinationClassifier.classify(uri)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination from BIP21 with amount")
            return
        }

        guard case .onchain(let addr, let amountSats) = dest else {
            XCTFail("Expected .onchain")
            return
        }

        XCTAssertEqual(addr, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
        XCTAssertEqual(amountSats, 100_000)
        XCTAssertFalse(dest.requiresManualAmount)
    }

    func testClassifyBIP21DecimalPrecisionAndExtraParams() {
        let oneSatUri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.00000001&label=test"
        guard case .valid(let dest1) = PaymentDestinationClassifier.classify(oneSatUri),
              case .onchain(_, let sats1) = dest1 else {
            XCTFail("Expected 1 sat from 0.00000001 BTC")
            return
        }
        XCTAssertEqual(sats1, 1)

        let largeUri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=1.5"
        guard case .valid(let destLarge) = PaymentDestinationClassifier.classify(largeUri),
              case .onchain(_, let satsLarge) = destLarge else {
            XCTFail("Expected 150_000_000 sats from 1.5 BTC")
            return
        }
        XCTAssertEqual(satsLarge, 150_000_000)

        let invalidAmountUri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=-0.5"
        guard case .valid(let destInvalid) = PaymentDestinationClassifier.classify(invalidAmountUri),
              case .onchain(_, let satsInvalid) = destInvalid else {
            XCTFail("Expected onchain with nil amount for negative BTC")
            return
        }
        XCTAssertNil(satsInvalid)
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

    func testClassifyUnsupportedLightningPrefix() {
        let result = PaymentDestinationClassifier.classify("lnxyz1234567890")
        guard case .invalid(let reason) = result else {
            XCTFail("Expected invalid result for unsupported lightning prefix")
            return
        }
        XCTAssertTrue(reason.contains("Unsupported"))
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

    func testClassifyBIP21WithInvalidLightningFallbackFallsBackToOnchain() {
        let bip21 = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.001&lightning=invalid_invoice"
        let result = PaymentDestinationClassifier.classify(bip21)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid onchain destination when lightning fallback is invalid")
            return
        }
        guard case .onchain(let addr, let amountSats) = dest else {
            XCTFail("Expected .onchain fallback")
            return
        }
        XCTAssertEqual(addr, "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq")
        XCTAssertEqual(amountSats, 100_000)
    }

    func testClassifyBIP21UppercaseScheme() {
        let bip21 = "BITCOIN:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        let result = PaymentDestinationClassifier.classify(bip21)
        guard case .valid(let dest) = result, case .onchain(let addr, _) = dest else {
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

        XCTAssertEqual(PaymentDestinationClassifier.classify(p2pkh), .valid(.onchain(address: p2pkh, amountSats: nil)))
        XCTAssertEqual(PaymentDestinationClassifier.classify(p2sh), .valid(.onchain(address: p2sh, amountSats: nil)))
        XCTAssertEqual(
            PaymentDestinationClassifier.classify(testnet),
            .valid(.onchain(address: testnet, amountSats: nil))
        )
        XCTAssertEqual(
            PaymentDestinationClassifier.classify(regtest),
            .valid(.onchain(address: regtest, amountSats: nil))
        )
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
              case .lightningAddress(let handle, let domain, let url) = dest else {
            XCTFail("Expected valid lightning address with plus tag")
            return
        }
        XCTAssertEqual(handle, "alice+tips")
        XCTAssertEqual(domain, "lightning.example.com")
        XCTAssertEqual(url.absoluteString, "https://lightning.example.com/.well-known/lnurlp/alice+tips")

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

    func testClassifyBIP21WithBase58AddressPreservesCase() {
        let base58 = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa"
        let bip21WithDoubleSlash = "bitcoin://\(base58)?amount=0.5"
        let result = PaymentDestinationClassifier.classify(bip21WithDoubleSlash)

        guard case .valid(let dest) = result else {
            XCTFail("Expected valid destination from bitcoin:// URI with Base58 address")
            return
        }
        guard case .onchain(let addr, let amountSats) = dest else {
            XCTFail("Expected .onchain")
            return
        }
        XCTAssertEqual(addr, base58)
        XCTAssertEqual(amountSats, 50_000_000)
    }

    func testClassifyLightningAddressStartingWithLnPrefix() {
        let lnbits = "lnbits@example.com"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(lnbits),
              case .lightningAddress(let handle, let domain, let url) = dest else {
            XCTFail("Expected valid lightning address for lnbits@example.com")
            return
        }
        XCTAssertEqual(handle, "lnbits")
        XCTAssertEqual(domain, "example.com")
        XCTAssertEqual(url.absoluteString, "https://example.com/.well-known/lnurlp/lnbits")

        let lnurl = "lnurl@service.com"
        guard case .valid(let dest2) = PaymentDestinationClassifier.classify(lnurl),
              case .lightningAddress(let handle2, _, _) = dest2 else {
            XCTFail("Expected valid lightning address for lnurl@service.com")
            return
        }
        XCTAssertEqual(handle2, "lnurl")
    }

    func testClassifyUppercaseSegwitAddress() {
        let uppercase = "BC1QAR0SRRR7XFKVY5L643LYDNW9RE59GTZZWF5MDQ"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(uppercase),
              case .onchain(let addr, _) = dest else {
            XCTFail("Expected valid onchain for uppercase Segwit address")
            return
        }
        XCTAssertEqual(addr, uppercase.lowercased())
    }

    func testClassifyBIP21WithoutOnchainAddress() {
        let invoiceStr = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        let bip21 = "bitcoin:?lightning=\(invoiceStr)"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(bip21),
              case .bolt11 = dest else {
            XCTFail("Expected valid bolt11 from BIP21 with no on-chain address")
            return
        }
    }

    func testClassifyBIP21SubSatoshiAndOverCapRejections() {
        // Sub-satoshi: 9 decimal places
        let subSat = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.000000015"
        guard case .valid(let destSub) = PaymentDestinationClassifier.classify(subSat),
              case .onchain(_, let amountSub) = destSub else {
            XCTFail("Expected valid destination envelope")
            return
        }
        XCTAssertNil(amountSub)

        // Over 21M BTC cap
        let overCap = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=21000001.0"
        guard case .valid(let destCap) = PaymentDestinationClassifier.classify(overCap),
              case .onchain(_, let amountCap) = destCap else {
            XCTFail("Expected valid destination envelope")
            return
        }
        XCTAssertNil(amountCap)
    }

    func testClassifyNetworkMismatchRejection() {
        let mainnet = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq"
        let testnet = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"

        // Mainnet on bitcoin -> valid
        XCTAssertEqual(
            PaymentDestinationClassifier.classify(mainnet, network: .bitcoin),
            .valid(.onchain(address: mainnet, amountSats: nil))
        )

        // Testnet on bitcoin -> invalid
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(testnet, network: .bitcoin) else {
            XCTFail("Expected invalid when testnet address used with bitcoin network")
            return
        }
        XCTAssertTrue(reason.contains("network"))

        // Mainnet on testnet -> invalid
        guard case .invalid = PaymentDestinationClassifier.classify(mainnet, network: .testnet) else {
            XCTFail("Expected invalid when mainnet address used with testnet network")
            return
        }
    }

    func testClassifyValidBolt12Offer() {
        let validOffer = "lno1pgx9getnwss8vetrw3hhyuckyypwa3eyt44h6txtxquqh7lz5djge4afgfjn7k4rgrkuag0jsd5xvxg"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(validOffer),
              case .bolt12(let offer, let raw) = dest else {
            XCTFail("Expected valid bolt12 destination for test vector offer")
            return
        }
        XCTAssertEqual(raw, validOffer)
        XCTAssertFalse(offer.description.isEmpty)
    }

    func testClassifyBIP21ExponentAmountRejection() {
        let expBip21 = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=1e3"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(expBip21),
              case .onchain(_, let amountSats) = dest else {
            XCTFail("Expected valid onchain envelope")
            return
        }
        XCTAssertNil(amountSats)
    }

    func testClassifySignetAddressSupport() {
        let signetSegwit = "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(signetSegwit, network: .signet),
              case .onchain(let addr, _) = dest else {
            XCTFail("Expected valid onchain for signet Segwit address")
            return
        }
        XCTAssertEqual(addr, signetSegwit.lowercased())
    }

    func testClassifyBIP21NetworkMismatchRejection() {
        let bip21Testnet = "bitcoin:tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx"
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(bip21Testnet, network: .bitcoin) else {
            XCTFail("Expected invalid when testnet address inside BIP21 is used with bitcoin network")
            return
        }
        XCTAssertEqual(reason, "Address network does not match active network.")
    }

    func testClassifyBolt11NetworkMismatchRejection() {
        let mainnetInvoice = "lnbc1pn8g249pp5f6ytj32ty90jhvw69enf30hwfgdhyymjewywcmfjevflg6s4z86qdqqcqzzgxqyz5vqrzjqwnvuc0u4txn35cafc7w94gxvq5p3cu9dd95f7hlrh0fvs46wpvhdfjjzh2j9f7ye5qqqqryqqqqthqqpysp5mm832athgcal3m7h35sc29j63lmgzvwc5smfjh2es65elc2ns7dq9qrsgqu2xcje2gsnjp0wn97aknyd3h58an7sjj6nhcrm40846jxphv47958c6th76whmec8ttr2wmg6sxwchvxmsc00kqrzqcga6lvsf9jtqgqy5yexa"
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(mainnetInvoice, network: .testnet)
        else {
            XCTFail("Expected invalid for mainnet invoice on testnet network")
            return
        }
        XCTAssertTrue(reason.contains("network"))
    }

    func testClassifyBolt12OfferNetworkMismatchRejection() {
        let mainnetOffer = "lno1pgx9getnwss8vetrw3hhyuckyypwa3eyt44h6txtxquqh7lz5djge4afgfjn7k4rgrkuag0jsd5xvxg"
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(mainnetOffer, network: .testnet) else {
            XCTFail("Expected invalid for mainnet offer on testnet network")
            return
        }
        XCTAssertTrue(reason.contains("network"))
    }

    func testClassifyBIP21UnhandledReqParameterRejection() {
        let uri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.01&req-custom-feature=true"
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(uri) else {
            XCTFail("Expected rejection for unhandled req- parameter in BIP21")
            return
        }
        XCTAssertTrue(reason.contains("req-custom-feature"))
    }

    func testClassifyBIP21DuplicateParameterRejection() {
        let uri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?amount=0.01&amount=0.02"
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(uri) else {
            XCTFail("Expected rejection for duplicate amount parameters in BIP21")
            return
        }
        XCTAssertEqual(reason, "Duplicate BIP21 parameter detected.")
    }

    func testClassifyBIP21PopParameterRejection() {
        let uri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?pop=https://example.com/pop"
        guard case .invalid(let reason) = PaymentDestinationClassifier.classify(uri) else {
            XCTFail("Expected rejection for unhandled pop parameter in BIP21")
            return
        }
        XCTAssertTrue(reason.contains("pop"))
    }

    func testClassifyBIP21WithBolt12OfferFallback() {
        let offerStr = "lno1pgx9getnwss8vetrw3hhyuckyypwa3eyt44h6txtxquqh7lz5djge4afgfjn7k4rgrkuag0jsd5xvxg"
        let uri = "bitcoin:bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq?lno=\(offerStr)"
        guard case .valid(let dest) = PaymentDestinationClassifier.classify(uri, network: .bitcoin),
              case .bolt12(_, let raw) = dest else {
            XCTFail("Expected valid bolt12 destination from BIP21 lno fallback")
            return
        }
        XCTAssertEqual(raw, offerStr)
    }
}
