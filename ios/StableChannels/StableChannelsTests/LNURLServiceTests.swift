import CryptoKit
import LDKNode
import Network
import Security
import XCTest
@testable import StableChannels

actor MockLNURLService: LNURLServiceProtocol {
    private var stubbedParams: LNURLPayParams?
    private var stubbedInvoiceResponse: LNURLPayInvoiceResponse?
    private var shouldThrowError: Error?

    func setStubbedParams(_ params: LNURLPayParams?) {
        self.stubbedParams = params
    }

    func setStubbedInvoiceResponse(_ response: LNURLPayInvoiceResponse?) {
        self.stubbedInvoiceResponse = response
    }

    func setShouldThrowError(_ error: Error?) {
        self.shouldThrowError = error
    }

    func fetchPayParams(from _: URL) async throws -> LNURLPayParams {
        if let error = shouldThrowError {
            throw error
        }
        guard let params = stubbedParams else {
            throw LNURLError.invalidResponse
        }
        return params
    }

    func fetchInvoice(
        params _: LNURLPayParams,
        amountMsat _: UInt64,
        comment _: String?
    ) async throws -> LNURLPayInvoiceResponse {
        if let error = shouldThrowError {
            throw error
        }
        guard let response = stubbedInvoiceResponse else {
            throw LNURLError.invalidResponse
        }
        return response
    }
}

final class LNURLServiceTests: XCTestCase {
    override func tearDown() {
        super.tearDown()
        MockURLProtocol.requestHandler = nil
    }

    func testMetadataParsingAndHash() {
        let metadataJSON = "[[\"text/plain\",\"Coffee Tip\"],[\"image/png;base64\",\"abc123def\"]]"
        let params = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/callback",
            minSendable: 1000,
            maxSendable: 2_100_000_000_000_000_000,
            metadata: metadataJSON,
            commentAllowed: 140
        )

        XCTAssertEqual(params.minSats, 1)
        XCTAssertEqual(params.maxSats, 2_100_000_000_000_000)
        XCTAssertEqual(params.plainTextDescription, "Coffee Tip")
        XCTAssertEqual(params.imageDescription?.mimeType, "image/png;base64")
        XCTAssertEqual(params.imageDescription?.base64Data, "abc123def")
        XCTAssertFalse(params.hasCustomSendBounds)
        XCTAssertTrue(params.hasValidBounds)
        XCTAssertTrue(params.isAmountValid(msat: 50_000))
        XCTAssertFalse(params.isAmountValid(msat: 500))

        let expectedDigest = SHA256.hash(data: Data(metadataJSON.utf8)).map { String(format: "%02x", $0) }.joined()
        XCTAssertEqual(params.metadataHashHex, expectedDigest)
    }

    func testCustomBoundsDetectionAndValidation() {
        let restrictedParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/callback",
            minSendable: 50_000,
            maxSendable: 1_000_000,
            metadata: "[[\"text/plain\",\"Coffee\"]]",
            commentAllowed: nil
        )

        XCTAssertTrue(restrictedParams.hasCustomSendBounds)
        XCTAssertTrue(restrictedParams.hasValidBounds)
        XCTAssertEqual(restrictedParams.minSats, 50)
        XCTAssertEqual(restrictedParams.maxSats, 1000)

        let invertedParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/callback",
            minSendable: 500_000,
            maxSendable: 100_000,
            metadata: "[[\"text/plain\",\"Inverted\"]]",
            commentAllowed: nil
        )
        XCTAssertFalse(invertedParams.hasValidBounds)
    }

    func testCommentValidation() {
        let params = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/callback",
            minSendable: 1000,
            maxSendable: 10000,
            metadata: "[[\"text/plain\",\"Test\"]]",
            commentAllowed: 50
        )

        XCTAssertTrue(params.isCommentValid(nil))
        XCTAssertTrue(params.isCommentValid(""))
        XCTAssertTrue(params.isCommentValid("Thank you!"))
        XCTAssertFalse(params.isCommentValid(String(repeating: "x", count: 51)))
    }

    func testResolveLightningAddressEndpoint() throws {
        let mockResolver = MockHostIPResolver(mapping: [
            "bitcoin.org": ["93.184.216.34"],
            "0xprabal.com": ["93.184.216.34"],
            "service.com": ["93.184.216.34"]
        ])
        let standard = try LNURLService.resolveEndpoint(from: "satoshi@bitcoin.org", hostResolver: mockResolver)
        XCTAssertEqual(standard.absoluteString, "https://bitcoin.org/.well-known/lnurlp/satoshi")

        let personal = try LNURLService.resolveEndpoint(from: "prabal@0xprabal.com", hostResolver: mockResolver)
        XCTAssertEqual(personal.absoluteString, "https://0xprabal.com/.well-known/lnurlp/prabal")

        let uriStandard = try LNURLService.resolveEndpoint(
            from: "lightning:satoshi@bitcoin.org",
            hostResolver: mockResolver
        )
        XCTAssertEqual(uriStandard.absoluteString, "https://bitcoin.org/.well-known/lnurlp/satoshi")

        let doubleSlash = try LNURLService.resolveEndpoint(
            from: "lightning://satoshi@bitcoin.org",
            hostResolver: mockResolver
        )
        XCTAssertEqual(doubleSlash.absoluteString, "https://bitcoin.org/.well-known/lnurlp/satoshi")

        let bech32Sample = "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
        let resolvedBech32 = try LNURLService.resolveEndpoint(from: bech32Sample, hostResolver: mockResolver)
        XCTAssertEqual(resolvedBech32.absoluteString, "https://service.com/api/v1/lnurl/pay")

        let bech32DoubleSlash = try LNURLService.resolveEndpoint(
            from: "lightning://\(bech32Sample)",
            hostResolver: mockResolver
        )
        XCTAssertEqual(bech32DoubleSlash.absoluteString, "https://service.com/api/v1/lnurl/pay")

        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "invalid-target", hostResolver: mockResolver))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "user@@domain.com", hostResolver: mockResolver))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "user@nodomain", hostResolver: mockResolver))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(
            from: "http://insecure-clearnet.com",
            hostResolver: mockResolver
        ))

        let torAddress = try LNURLService.resolveEndpoint(from: "anon@xyz.onion", hostResolver: mockResolver)
        XCTAssertEqual(torAddress.absoluteString, "http://xyz.onion/.well-known/lnurlp/anon")

        let directTor = try LNURLService.resolveEndpoint(
            from: "http://xyz.onion/api/lnurlp",
            hostResolver: mockResolver
        )
        XCTAssertEqual(directTor.absoluteString, "http://xyz.onion/api/lnurlp")
    }

    func testSuccessAction_validationAndDomainMatching() throws {
        let callbackURL = try XCTUnwrap(URL(string: "https://pay.shop.com/callback"))

        // Host and subdomain matching
        let sameHostAction = LNURLSuccessAction(
            tag: "url",
            description: "Receipt",
            url: "https://pay.shop.com/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertTrue(sameHostAction.isSameHostOrSubdomain(callbackURL: callbackURL))
        if case .url(let desc, let url) = sameHostAction.validatedAction(callbackURL: callbackURL) {
            XCTAssertEqual(desc, "Receipt")
            XCTAssertEqual(url.absoluteString, "https://pay.shop.com/receipt/123")
        } else {
            XCTFail("Expected valid URL action")
        }

        let subdomainAction = LNURLSuccessAction(
            tag: "url",
            description: "Receipt",
            url: "https://sub.pay.shop.com/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertTrue(subdomainAction.isSameHostOrSubdomain(callbackURL: callbackURL))
        if case .url = subdomainAction.validatedAction(callbackURL: callbackURL) {} else {
            XCTFail("Expected valid subdomain URL action")
        }

        let phishingAction = LNURLSuccessAction(
            tag: "url",
            description: "Phishing",
            url: "https://malicious-redirect.com/login",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertFalse(phishingAction.isSameHostOrSubdomain(callbackURL: callbackURL))
        XCTAssertEqual(phishingAction.validatedAction(callbackURL: callbackURL), .unknown(tag: "url"))

        // Onion HTTP vs Clearnet HTTP vs javascript schemes
        let torCallback = try XCTUnwrap(URL(string: "http://shop.onion/callback"))
        let torAction = LNURLSuccessAction(
            tag: "url",
            description: "Tor Receipt",
            url: "http://shop.onion/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        if case .url(let desc, let url) = torAction.validatedAction(callbackURL: torCallback) {
            XCTAssertEqual(desc, "Tor Receipt")
            XCTAssertEqual(url.absoluteString, "http://shop.onion/receipt/123")
        } else {
            XCTFail("Expected valid Tor URL action")
        }

        let insecureClearnetAction = LNURLSuccessAction(
            tag: "url",
            description: "Insecure",
            url: "http://insecure-shop.com/receipt",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertEqual(insecureClearnetAction.validatedAction(callbackURL: callbackURL), .unknown(tag: "url"))

        let javascriptAction = LNURLSuccessAction(
            tag: "url",
            description: "Exploit",
            url: "javascript:alert(1)",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertEqual(javascriptAction.validatedAction(callbackURL: callbackURL), .unknown(tag: "url"))

        // Message character limits (max 144)
        let validMessageAction = LNURLSuccessAction(
            tag: "message",
            description: nil,
            url: nil,
            message: "Payment received!",
            ciphertext: nil,
            iv: nil
        )
        XCTAssertEqual(validMessageAction.validatedAction(callbackURL: callbackURL), .message("Payment received!"))

        let longMessageAction = LNURLSuccessAction(
            tag: "message",
            description: nil,
            url: nil,
            message: String(repeating: "a", count: 145),
            ciphertext: nil,
            iv: nil
        )
        XCTAssertEqual(longMessageAction.validatedAction(callbackURL: callbackURL), .unknown(tag: "message"))

        // AES ciphertext and IV length validation (IV must be 16 bytes decoded)
        let validAESAction = LNURLSuccessAction(
            tag: "aes",
            description: "Secret",
            url: nil,
            message: nil,
            ciphertext: "AQIDBA==",
            iv: "MDEyMzQ1Njc4OWFiY2RlZg=="
        )
        if case .aes(let desc, _, _) = validAESAction.validatedAction(callbackURL: callbackURL) {
            XCTAssertEqual(desc, "Secret")
        } else {
            XCTFail("Expected valid AES action")
        }

        let invalidIVAction = LNURLSuccessAction(
            tag: "aes",
            description: "Secret",
            url: nil,
            message: nil,
            ciphertext: "AQIDBA==",
            iv: "AQID"
        )
        XCTAssertEqual(invalidIVAction.validatedAction(callbackURL: callbackURL), .unknown(tag: "aes"))
    }

    func testErrorResponseParsing() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let mockResolver = MockHostIPResolver(mapping: ["service.example.com": ["93.184.216.34"]])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://service.example.com/.well-known/lnurlp/invalid"))
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 404, httpVersion: nil, headerFields: nil)!
            let data = Data("{\"status\":\"ERROR\",\"reason\":\"User not found\"}".utf8)
            return (response, data)
        }

        do {
            _ = try await service.fetchPayParams(from: targetURL)
            XCTFail("Expected LNURLError.errorResponse")
        } catch let LNURLError.errorResponse(reason) {
            // After reordering, 404 status gate fires before body parsing
            XCTAssertEqual(reason, "Recipient address not found.")
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testMockedLNURLPayResolution() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let mockResolver = MockHostIPResolver(mapping: ["service.example.com": ["93.184.216.34"]])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://service.example.com/.well-known/lnurlp/prabal"))
        let responseJson = """
        {
            "tag": "payRequest",
            "callback": "https://service.example.com/callback",
            "minSendable": 1000,
            "maxSendable": 1000000000,
            "metadata": "[[\\"text/plain\\",\\"prabal\\"]]",
            "commentAllowed": 140
        }
        """
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let data = Data(responseJson.utf8)
            return (response, data)
        }

        let params = try await service.fetchPayParams(from: targetURL)
        XCTAssertEqual(params.tag.lowercased(), "payrequest")
        XCTAssertTrue(params.maxSendable >= params.minSendable)
        XCTAssertTrue(params.callback.starts(with: "https://"))
    }

    // MARK: - Invoice Verification Tests

    private static let metadataHashA = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"

    // 50,000 msat invoice matching metadataHashA
    private static let valid50kInvoice =
        "lnbcrt500n1p4tku6ghp5uwcvgs5clswpfxhm7nyfjmaeysn6us0yvjdexn9yjkv3k7zjhp2snp4qd0e5h2zerw5ftxv43r40z7vecunmwk55jvs2wr822zve50el2apvpp5w4qq8tyxn20d5f67jghar9zq9d426geks40etmyn8yzkn8mpqltqsp5aryyc4xdqfpgswmjzxudq96dgyy7yl9ulxrrg2kmzsvaxszvc4zs9qyysgqcqzp2xq97zvuqxzzzqpwe94dfnmrnuaz067kd2jpmggwvgxgy9hfnx6qxcwhcexdxw65qw286uvnpn59nw3k82y94ga76pn7cujulr8ddhmja5atzjjqqcu9kyg"

    // 100,000 msat invoice matching metadataHashA
    private static let mismatchedAmountInvoice100k =
        "lnbcrt1u1p4tku6ghp5uwcvgs5clswpfxhm7nyfjmaeysn6us0yvjdexn9yjkv3k7zjhp2snp4qd0e5h2zerw5ftxv43r40z7vecunmwk55jvs2wr822zve50el2apvpp5gwmcnhr25vzznnky7j9fkn6jhytwnjfdjhugf9tts5vanr8hkscqsp5j5araz875hhj74qa6v9vjwaktzjmlz34yr46nfxl032hcg444yxs9qyysgqcqzp2xq97zvuqv4l34uxylnpl2s59krt4yxrhct6v9yqn6f4uk7egxxg7n3xtu50pqtxfuxg5swcqzhg9nkj07z6ggtflrz0evte9g9tpq8smskq44agq8qjnhu"

    // 50,000 msat invoice with direct text description instead of h tag
    private static let directDescriptionInvoice =
        "lnbcrt500n1p4tku6gdqjg35hyetrwssygetnvvnp4qd0e5h2zerw5ftxv43r40z7vecunmwk55jvs2wr822zve50el2apvpp530vltuvw6z2cjxtjgqtk89w9aa9tkjs7kjmn3p00u9lg6lvqfscssp5x6kv3wggegd35thl2h7cfv27vgxf6dfgmg9gy4uy9wa3r2y6e82s9qyysgqcqzp2xq97zvuqpaxgcxn97nu274q09u8r93xh3mee2ep78j5c8vgd0a3zvlpuwysqx2ezm9mm8uasuyw3r0xua6wy0vd20zkkynpepzkz3n5jyrlt4zgpe8hhg3"

    // Expired invoice (expirySecs: 1)
    private static let expiredInvoice =
        "lnbcrt500n1p4tkum6hp5uwcvgs5clswpfxhm7nyfjmaeysn6us0yvjdexn9yjkv3k7zjhp2snp4qf7cucxts7m2m34lwa78u7mv7x5kdpclmamjugwgm3jpevvwtge0jpp583gggyx4gapnxcynr0xyufahkyxthd6a9h235s904n5pmjwvxvpqsp5gz0j34zt87chh6dr0etekngg3y7snw6tnsrdtpw03vr2h5nuetdq9qyysgqcqzp2xqppd49a5hs3dpn5ssgqryd6m7utpcf6ysz4t9yhx6hl2cl4j94283m5dchu6xduq8ca4946qggzw7n28wgutuud4qga56l4lww8pkjmu9qpdkkksu"

    // Amountless invoice
    private static let amountlessInvoice =
        "lnbcrt1p4tkumuhp5uwcvgs5clswpfxhm7nyfjmaeysn6us0yvjdexn9yjkv3k7zjhp2snp4qf7cucxts7m2m34lwa78u7mv7x5kdpclmamjugwgm3jpevvwtge0jpp53hupfs45s6g5d8344dz6d09f5t3vhj3rhdwxe23xdv6tqueyqleqsp5nxdytqnnlneq489wc8h04megxf7tcfq07tfpek03yl2d0p426y0q9qyysgqcqzp2xq97zvuqdkurx2kmkkqp3d7pvhw76vpr7e5zmlyljmmcl4mm2wjkf4a69w4nwt9spch3yrxny9auez5taj0d03mh38lqkn7j9v636kk5cl2jttgqd7gvvp"

    private func makeParams(
        callback: String,
        metadata: String = "",
        minSendable: UInt64 = 1_000,
        maxSendable: UInt64 = 100_000_000,
        commentAllowed: Int? = 100
    ) -> LNURLPayParams {
        LNURLPayParams(
            tag: "payRequest",
            callback: callback,
            minSendable: minSendable,
            maxSendable: maxSendable,
            metadata: metadata,
            commentAllowed: commentAllowed
        )
    }

    private func makeMockService(
        hostMapping: [String: [String]] = ["service.example.com": ["93.184.216.34"]]
    ) -> (LNURLService, URL) {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let resolver = MockHostIPResolver(mapping: hostMapping)
        let service = LNURLService(urlSession: session, hostResolver: resolver)
        let callbackURL = URL(string: "https://service.example.com/callback")!
        return (service, callbackURL)
    }

    func testFetchInvoice_validInvoice_succeeds() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        let result = try await service.fetchInvoice(
            params: params,
            amountMsat: 50_000,
            comment: "Tip"
        )

        XCTAssertEqual(result.pr, Self.valid50kInvoice)
        XCTAssertFalse(result.isError)
    }

    func testFetchInvoice_amountOutOfBounds_throwsError() async throws {
        let (service, callbackURL) = makeMockService()
        let params = LNURLPayParams(
            tag: "payRequest",
            callback: callbackURL.absoluteString,
            minSendable: 10_000,
            maxSendable: 100_000,
            metadata: "",
            commentAllowed: 20
        )

        // Amount below minSendable throws amountOutOfBounds
        do {
            _ = try await service.fetchInvoice(params: params, amountMsat: 5_000, comment: nil)
            XCTFail("Expected amountOutOfBounds")
        } catch LNURLError.amountOutOfBounds {
            // Success
        }

        // Amount above maxSendable throws amountOutOfBounds
        do {
            _ = try await service.fetchInvoice(params: params, amountMsat: 150_000, comment: nil)
            XCTFail("Expected amountOutOfBounds")
        } catch LNURLError.amountOutOfBounds {
            // Success
        }
    }

    func testFetchInvoice_mismatchedAmount_throwsInvoiceAmountMismatch() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.mismatchedAmountInvoice100k)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected invoiceAmountMismatch error")
        } catch let LNURLError.invoiceAmountMismatch(expectedMsat, actualMsat) {
            XCTAssertEqual(expectedMsat, 50_000)
            XCTAssertEqual(actualMsat, 100_000)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_mismatchedDescriptionHash_throwsErrorResponse() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "[[\"text/plain\",\"Coffee\"]]")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected errorResponse due to description hash mismatch")
        } catch let LNURLError.errorResponse(reason) {
            XCTAssertTrue(reason.contains("does not match payee metadata"))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_directDescription_throwsErrorResponse() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.directDescriptionInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected errorResponse due to direct description")
        } catch let LNURLError.errorResponse(reason) {
            XCTAssertTrue(reason.contains("direct description"))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_expiredInvoice_throwsErrorResponse() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.expiredInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected errorResponse due to expired invoice")
        } catch let LNURLError.errorResponse(reason) {
            XCTAssertTrue(reason.contains("expired"))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_amountlessInvoice_throwsErrorResponse() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.amountlessInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected errorResponse due to amountless invoice")
        } catch let LNURLError.errorResponse(reason) {
            XCTAssertTrue(reason.contains("Amountless invoices are not permitted"))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_queryParametersAndEncoding() async throws {
        let (service, callbackURL) = makeMockService()
        var capturedURL: URL?

        MockURLProtocol.requestHandler = { request in
            capturedURL = request.url
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "", commentAllowed: 100)
        _ = try await service.fetchInvoice(
            params: params,
            amountMsat: 50_000,
            comment: "coffee+croissant & hello? test=1"
        )

        let resolvedURL = try XCTUnwrap(capturedURL)
        let components = try XCTUnwrap(URLComponents(url: resolvedURL, resolvingAgainstBaseURL: false))
        let queryItems = try XCTUnwrap(components.queryItems)

        XCTAssertEqual(queryItems.first(where: { $0.name == "amount" })?.value, "50000")
        XCTAssertEqual(queryItems.first(where: { $0.name == "comment" })?.value, "coffee+croissant & hello? test=1")
        let rawQuery = try XCTUnwrap(resolvedURL.query(percentEncoded: true))
        XCTAssertTrue(rawQuery.contains("%26"))
        XCTAssertTrue(rawQuery.contains("%2B"))
    }

    func testFetchPayParams_missingPlainTextMetadata_throwsInvalidMetadata() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let mockResolver = MockHostIPResolver(mapping: ["service.example.com": ["93.184.216.34"]])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://service.example.com/.well-known/lnurlp/alice"))
        let invalidMetadataJson = """
        {
            "tag": "payRequest",
            "callback": "https://service.example.com/callback",
            "minSendable": 1000,
            "maxSendable": 1000000000,
            "metadata": "[[\\"image/png;base64\\",\\"abc123def\\"]]",
            "commentAllowed": 140
        }
        """
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data(invalidMetadataJson.utf8))
        }

        do {
            _ = try await service.fetchPayParams(from: targetURL)
            XCTFail("Expected LNURLError.invalidMetadata")
        } catch LNURLError.invalidMetadata {
            // Expected
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_insecureRedirectDowngrade_throwsInsecureEndpoint() async throws {
        let (service, callbackURL) = makeMockService()
        let insecureRedirectURL = try XCTUnwrap(URL(string: "http://service.example.com/insecure-callback"))

        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(
                url: callbackURL,
                statusCode: 302,
                httpVersion: nil,
                headerFields: ["Location": insecureRedirectURL.absoluteString]
            )!
            return (response, Data())
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected LNURLError.insecureEndpoint on HTTP downgrade")
        } catch LNURLError.insecureEndpoint {
            // Expected
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(insecureRedirectURL))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testLNURLService_torHttpAccepted_andClearnetHttpRejected() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let service = LNURLService(urlSession: session)

        let torURL = try XCTUnwrap(URL(string: "http://service.onion/lnurlp"))
        let clearnetHttpURL = try XCTUnwrap(URL(string: "http://service.com/lnurlp"))
        let validJson = """
        {
            "tag": "payRequest",
            "callback": "http://service.onion/callback",
            "minSendable": 1000,
            "maxSendable": 10000000,
            "metadata": "[[\\"text/plain\\",\\"Tor Service\\"]]",
            "commentAllowed": 50
        }
        """

        MockURLProtocol.requestHandler = { req in
            let response = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            if req.url?.path.contains("callback") == true {
                let invoiceJson = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
                return (response, Data(invoiceJson.utf8))
            }
            return (response, Data(validJson.utf8))
        }

        // Tor hidden service over HTTP succeeds for both stages
        let params = try await service.fetchPayParams(from: torURL)
        XCTAssertEqual(params.callback, "http://service.onion/callback")
        let torInvoiceParams = makeParams(callback: "http://service.onion/callback", metadata: "")
        let invoiceResult = try await service.fetchInvoice(params: torInvoiceParams, amountMsat: 50_000, comment: nil)
        XCTAssertEqual(invoiceResult.pr, Self.valid50kInvoice)

        // Clearnet endpoint over HTTP is rejected for both stages
        do {
            _ = try await service.fetchPayParams(from: clearnetHttpURL)
            XCTFail("Expected LNURLError.insecureEndpoint for clearnet HTTP pay params")
        } catch LNURLError.insecureEndpoint {}

        let clearnetParams = makeParams(callback: "http://service.com/callback", metadata: "")
        do {
            _ = try await service.fetchInvoice(params: clearnetParams, amountMsat: 50_000, comment: nil)
            XCTFail("Expected LNURLError.insecureEndpoint for clearnet HTTP callback")
        } catch LNURLError.insecureEndpoint {}
    }

    func testIsPrivateOrLoopbackHost_identifiesAllRestrictedRanges() {
        let restricted = [
            "localhost",
            "sub.localhost",
            "router.local",
            "cluster.internal",
            "127.0.0.1",
            "127.255.255.254",
            "10.0.0.1",
            "10.254.1.9",
            "172.16.0.1",
            "172.31.255.254",
            "192.168.0.1",
            "192.168.100.200",
            "169.254.169.254",
            "100.64.0.1",
            "100.127.255.254",
            "198.18.0.1",
            "198.19.255.254",
            "192.0.0.1",
            "192.0.2.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "239.255.255.250",
            "240.0.0.1",
            "255.255.255.255",
            "0.0.0.0",
            "::1",
            "[::1]",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456:789a::1",
            "ff00::1",
            "ff02::1",
            "100::1",
            "2001:db8::1",
            "2002:0a00:0001::",
            "2002:7f00:0001::",
            "2001:0000:0000:0000:0000:0000:f5ff:fffe",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.1.1"
        ]
        for host in restricted {
            XCTAssertTrue(
                LNURLService.isPrivateOrLoopbackHost(host),
                "Expected \(host) to be detected as private/loopback"
            )
        }

        let allowed = [
            "bitcoin.org",
            "ln.tips",
            "example.com",
            "service.onion",
            "8.8.8.8",
            "172.15.255.255",
            "172.32.0.1",
            "100.128.0.1",
            "198.20.0.1",
            "1.1.1.1",
            "2002:5db8:d822::",
            "2606:4700:4700::1111"
        ]
        for host in allowed {
            XCTAssertFalse(LNURLService.isPrivateOrLoopbackHost(host), "Expected \(host) to be public")
        }
    }

    func testResolveEndpoint_rejectsPrivateAndLoopbackHosts() {
        let privateAddresses = [
            "alice@127.0.0.1",
            "bob@localhost",
            "carol@192.168.1.1",
            "david@10.0.0.1",
            "eve@169.254.169.254",
            "frank@mydevice.local"
        ]
        for target in privateAddresses {
            XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: target)) { error in
                guard case LNURLError.invalidTarget = error else {
                    XCTFail("Expected invalidTarget for \(target), got \(error)")
                    return
                }
            }
        }

        let privateURLs = [
            "https://127.0.0.1/lnurlp",
            "https://localhost/lnurlp",
            "https://192.168.1.1/.well-known/lnurlp/alice",
            "http://127.0.0.1/lnurlp",
            // All 6 private targets encoded as Bech32 lnurl1
            "lnurl1dp68gurn8ghj7mr0vdskc6r0wd6z7mrww4excuq5ex0g8", // localhost
            "lnurl1dp68gurn8ghj7vfjxuhrqt3s9ccj7mrww4excuqw06qrc", // 127.0.0.1
            "lnurl1dp68gurn8ghj7vfs9cczuvpwxyhkcmn4wfk8qdmhwmg", // 10.0.0.1
            "lnurl1dp68gurn8ghj7vfk8yhrydf59ccnvwfwxg6ngtmvde6hymrsn3sg3p", // 169.254.169.254
            "lnurl1dp68gurn8ghj7vfexghrzd3c9ccjuvf0d3h82unvwq0hrk4j", // 192.168.1.1
            "lnurl1dp68gurn8ghj7mtev3jhv6trv5hxcmmrv9kz7mrww4excuq6q74wt" // mydevice.local
        ]
        for target in privateURLs {
            XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: target)) { error in
                guard case LNURLError.invalidTarget = error else {
                    XCTFail("Expected invalidTarget for \(target), got \(error)")
                    return
                }
            }
        }
    }

    func testFetchPayParams_uint64MaxMinSendable_doesNotTrapAndThrowsAmountOutOfBounds() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let mockResolver = MockHostIPResolver(mapping: ["service.example.com": ["93.184.216.34"]])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://service.example.com/.well-known/lnurlp/alice"))
        let overflowingJson = """
        {
            "tag": "payRequest",
            "callback": "https://service.example.com/callback",
            "minSendable": \(UInt64.max),
            "maxSendable": \(UInt64.max),
            "metadata": "[[\\"text/plain\\",\\"alice\\"]]",
            "commentAllowed": 140
        }
        """

        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data(overflowingJson.utf8))
        }

        do {
            _ = try await service.fetchPayParams(from: targetURL)
            XCTFail("Expected LNURLError.amountOutOfBounds without trapping")
        } catch let LNURLError.amountOutOfBounds(minSats, maxSats) {
            let expectedCeiling = (UInt64.max / 1000) + 1
            XCTAssertEqual(minSats, expectedCeiling)
            XCTAssertEqual(maxSats, UInt64.max / 1000)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testMinSatsCeiling_uint64MaxValues_doesNotOverflow() {
        let maxParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/cb",
            minSendable: UInt64.max,
            maxSendable: UInt64.max,
            metadata: "[[\"text/plain\",\"Overflow Test\"]]",
            commentAllowed: nil
        )
        let expectedCeil = (UInt64.max / 1000) + 1
        XCTAssertEqual(maxParams.minSats, expectedCeil)
        XCTAssertFalse(maxParams.hasValidBounds)

        let nearMaxParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/cb",
            minSendable: UInt64.max - 998,
            maxSendable: UInt64.max,
            metadata: "[[\"text/plain\",\"Near Max Test\"]]",
            commentAllowed: nil
        )
        let expectedNearCeil = ((UInt64.max - 998) / 1000) + 1
        XCTAssertEqual(nearMaxParams.minSats, expectedNearCeil)

        let exactMultipleParams = LNURLPayParams(
            tag: "payRequest",
            callback: "https://service.com/cb",
            minSendable: 10_000,
            maxSendable: 50_000,
            metadata: "[[\"text/plain\",\"Exact Multiple Test\"]]",
            commentAllowed: nil
        )
        XCTAssertEqual(exactMultipleParams.minSats, 10)
    }

    func testLNURLService_dnsResolvesToPrivateIP_isRejectedAsInsecureEndpoint() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let mockResolver = MockHostIPResolver(mapping: [
            "rebinding.example.com": ["127.0.0.1"]
        ])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let payURL = try XCTUnwrap(URL(string: "https://rebinding.example.com/.well-known/lnurlp/alice"))
        let callbackURL = try XCTUnwrap(URL(string: "https://rebinding.example.com/callback"))

        MockURLProtocol.seenURLs = []
        MockURLProtocol.requestHandler = { req in
            let response = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await service.fetchPayParams(from: payURL)
            XCTFail("Expected LNURLError.insecureEndpoint for fetchPayParams")
        } catch LNURLError.insecureEndpoint {
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(payURL))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(params: params, amountMsat: 50_000, comment: nil)
            XCTFail("Expected LNURLError.insecureEndpoint for fetchInvoice")
        } catch LNURLError.insecureEndpoint {
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(callbackURL))
        }
    }

    func testFetchInvoice_redirectsToPrivateTarget_isVetoedPreHop() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let callbackURL = try XCTUnwrap(URL(string: "https://service.example.com/callback"))
        let mockResolver = MockHostIPResolver(mapping: [
            "service.example.com": ["93.184.216.34"],
            "rebinding-redirect.example.com": ["192.168.1.50"]
        ])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targets = [
            try XCTUnwrap(URL(string: "https://127.0.0.1/callback")),
            try XCTUnwrap(URL(string: "https://rebinding-redirect.example.com/internal-service"))
        ]

        for redirectTargetURL in targets {
            MockURLProtocol.seenURLs = []
            MockURLProtocol.requestHandler = { _ in
                let response = HTTPURLResponse(
                    url: callbackURL,
                    statusCode: 302,
                    httpVersion: nil,
                    headerFields: ["Location": redirectTargetURL.absoluteString]
                )!
                return (response, Data())
            }

            let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
            do {
                _ = try await service.fetchInvoice(
                    params: params,
                    amountMsat: 50_000,
                    comment: nil
                )
                XCTFail("Expected LNURLError.insecureEndpoint on redirect to \(redirectTargetURL)")
            } catch LNURLError.insecureEndpoint {
                XCTAssertFalse(MockURLProtocol.seenURLs.contains(redirectTargetURL))
            } catch {
                XCTFail("Unexpected error for \(redirectTargetURL): \(error)")
            }
        }
    }

    func testResolveEndpoint_withHostResolver_rejectsDNSResolvingToPrivateIP() {
        let mockResolver = MockHostIPResolver(mapping: [
            "evil.example.com": ["169.254.169.254"]
        ])

        XCTAssertThrowsError(
            try LNURLService.resolveEndpoint(
                from: "https://evil.example.com/pay",
                hostResolver: mockResolver
            )
        ) { error in
            guard case LNURLError.invalidTarget = error else {
                XCTFail("Expected invalidTarget, got \(error)")
                return
            }
        }
    }

    func testFetchPayParams_dnsResolutionFails_failsClosed() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let mockResolver = MockHostIPResolver(mapping: [:])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://nonexistent.example.com/.well-known/lnurlp/alice"))

        MockURLProtocol.seenURLs = []
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await service.fetchPayParams(from: targetURL)
            XCTFail("Expected LNURLError.insecureEndpoint for failed DNS resolution")
        } catch LNURLError.insecureEndpoint {
            // Expected - must fail closed and never touch network
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(targetURL))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testURLSessionTransport_dnsRebindsPostFlight_isRejected() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let callbackURL = try XCTUnwrap(URL(string: "https://rebinding.example.com/callback"))

        final class StatefulResolver: HostIPResolving, @unchecked Sendable {
            private let lock = NSLock()
            private var callCount = 0

            func resolveHostIPs(_: String) -> [String] {
                lock.withLock {
                    callCount += 1
                    if callCount == 1 {
                        return ["93.184.216.34"]
                    } else {
                        return ["10.0.0.1"]
                    }
                }
            }
        }

        let statefulResolver = StatefulResolver()
        let service = LNURLService(urlSession: session, hostResolver: statefulResolver)

        MockURLProtocol.seenURLs = []
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected LNURLError.insecureEndpoint when DNS rebinds to private IP on post-flight check")
        } catch LNURLError.insecureEndpoint {
            // Rebinding was caught and rejected by post-flight validation
            XCTAssertFalse(MockURLProtocol.seenURLs.isEmpty)
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testIsSecureEndpoint_validatesPublicHostsAndBlocksPrivateOrLoopback() throws {
        let resolver = MockHostIPResolver(mapping: [
            "service.example.com": ["93.184.216.34"],
            "ipv6.example.com": ["2606:4700:4700::1111"],
            "private.example.com": ["10.0.0.1"]
        ])

        // Standard HTTPS URL
        let url = try XCTUnwrap(URL(string: "https://service.example.com/api?foo=bar"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: resolver))

        // Non-default port
        let portURL = try XCTUnwrap(URL(string: "https://service.example.com:8443/api"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: portURL, hostResolver: resolver))

        // IPv6 resolution
        let ipv6URL = try XCTUnwrap(URL(string: "https://ipv6.example.com/api"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: ipv6URL, hostResolver: resolver))

        // Tor onion service is accepted over HTTP
        let torURL = try XCTUnwrap(URL(string: "http://service.onion/api"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: torURL, hostResolver: resolver))

        // Clearnet IP literal is rejected for LNURL endpoints per RFC 6066 SNI rules
        let publicIPURL = try XCTUnwrap(URL(string: "https://8.8.8.8/api"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: publicIPURL, hostResolver: resolver))
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("8.8.8.8"), false)

        // Private IP literal returns false
        let privateIPURL = try XCTUnwrap(URL(string: "https://127.0.0.1/api"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: privateIPURL, hostResolver: resolver))

        // Host resolving to private IP returns false
        let privateHostURL = try XCTUnwrap(URL(string: "https://private.example.com/api"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: privateHostURL, hostResolver: resolver))
    }

    func testExecuteSecureGet_preservesHostnameForTLS() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let resolver = MockHostIPResolver(mapping: [
            "pay.example.com": ["93.184.216.34"]
        ])
        let service = LNURLService(urlSession: session, hostResolver: resolver)

        var capturedRequest: URLRequest?
        let targetURL = try XCTUnwrap(URL(string: "https://pay.example.com/.well-known/lnurlp/alice"))
        let validJson = """
        {
            "tag": "payRequest",
            "callback": "https://pay.example.com/callback",
            "minSendable": 1000,
            "maxSendable": 10000000,
            "metadata": "[[\\"text/plain\\",\\"Alice\\"]]",
            "commentAllowed": 50
        }
        """

        MockURLProtocol.requestHandler = { req in
            capturedRequest = req
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data(validJson.utf8))
        }

        let params = try await service.fetchPayParams(from: targetURL)
        XCTAssertEqual(params.callback, "https://pay.example.com/callback")

        let actualRequest = try XCTUnwrap(capturedRequest)
        XCTAssertEqual(actualRequest.url?.host, "pay.example.com")
        XCTAssertEqual(actualRequest.url?.path, "/.well-known/lnurlp/alice")
    }

    func testSecureEndpointValidator_blocksAllMissingSubnets() {
        let blocked = [
            "64:ff9b::a9fe:a9fe", // NAT64 169.254.169.254 (RFC 6052)
            "64:ff9b::7f00:1", // NAT64 127.0.0.1 (RFC 6052)
            "64:ff9b:1::1", // Local-use NAT64 (RFC 8215)
            "fec0::1", // Deprecated site-local (RFC 3879)
            "2001:10::1", // ORCHID (RFC 4843)
            "2001:20::1", // ORCHIDv2 (RFC 7343)
            "2002:c058:6301::", // 6to4 embedding 192.88.99.1
            "192.88.99.1", // 6to4 anycast relay (RFC 3068/7526)
            "::ffff:0:127.0.0.1", // SIIT IPv4-translated (RFC 2765)
            "::ffff:0:10.0.0.1", // SIIT IPv4-translated (RFC 2765)
            "3fff::1" // IPv6 documentation prefix (RFC 9637)
        ]
        for ip in blocked {
            XCTAssertTrue(SecureEndpointValidator.isPrivateOrLoopbackHost(ip), "Expected \(ip) to be blocked")
        }

        let allowed = [
            "2606:4700::1111",
            "1.1.1.1",
            "93.184.216.34",
            "::ffff:0:8.8.8.8"
        ]
        for ip in allowed {
            XCTAssertFalse(SecureEndpointValidator.isPrivateOrLoopbackHost(ip), "Expected \(ip) to be allowed")
        }
    }

    func testSecureEndpointValidator_blocksNonCanonicalIPv4Literals() throws {
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("0127.0.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("00127.0.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("010.0.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("0172.016.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("0100.064.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("169.0254.169.254"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("0177.0.0.1"), true)

        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("127.0.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("10.0.0.1"), true)
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("8.8.8.8"), false)

        // Non-canonical forms are not recognized as numeric literals by inet_pton
        XCTAssertNil(SecureEndpointValidator.evaluateNumericIP("0x7f.0.0.1"))
        XCTAssertNil(SecureEndpointValidator.evaluateNumericIP("2130706433"))
        XCTAssertNil(SecureEndpointValidator.evaluateNumericIP("127.1"))

        XCTAssertTrue(SecureEndpointValidator.isPrivateOrLoopbackHost("0127.0.0.1"))
        XCTAssertTrue(SecureEndpointValidator.isPrivateOrLoopbackHost("010.0.0.1"))
        XCTAssertTrue(SecureEndpointValidator.isPrivateOrLoopbackHost("169.0254.169.254"))
        XCTAssertTrue(SecureEndpointValidator.isPrivateOrLoopbackHost("0177.0.0.1"))

        let blockedURLs = [
            "https://0127.0.0.1/api",
            "https://00127.0.0.1/api",
            "https://010.0.0.1/api",
            "https://0172.016.0.1/api",
            "https://0100.064.0.1/api",
            "https://169.0254.169.254/api",
            "https://0177.0.0.1/api"
        ]
        for urlStr in blockedURLs {
            let url = try XCTUnwrap(URL(string: urlStr))
            XCTAssertFalse(
                SecureEndpointValidator.isSecureEndpoint(url: url),
                "Expected \(urlStr) to be blocked"
            )
        }

        let publicURL = try XCTUnwrap(URL(string: "https://8.8.8.8/api"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: publicURL))
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("8.8.8.8"), false)

        // Legacy non-canonical forms fall through to DNS resolution and are blocked if resolving to loopback
        let legacyForms = ["https://0x7f.0.0.1/api", "https://2130706433/api", "https://127.1/api"]
        let legacyResolver = MockHostIPResolver(mapping: [
            "0x7f.0.0.1": ["127.0.0.1"],
            "2130706433": ["127.0.0.1"],
            "127.1": ["127.0.0.1"]
        ])
        for legacyStr in legacyForms {
            let legacyURL = try XCTUnwrap(URL(string: legacyStr))
            XCTAssertFalse(
                SecureEndpointValidator.isSecureEndpoint(url: legacyURL, hostResolver: legacyResolver),
                "Expected \(legacyStr) to be blocked via resolver path"
            )
        }
    }

    func testIsSecureEndpoint_dualStackPermittedWhenPublic() throws {
        let resolver = MockHostIPResolver(mapping: [
            "dual.example.com": ["2606:4700::1111", "93.184.216.34"]
        ])
        let url = try XCTUnwrap(URL(string: "https://dual.example.com/api"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: resolver))
    }

    func testIsSecureEndpoint_rejectsUserinfo() throws {
        let resolver = MockHostIPResolver(mapping: [
            "auth.example.com": ["93.184.216.34"]
        ])
        let url = try XCTUnwrap(URL(string: "https://user:password@auth.example.com/api"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: resolver))
    }

    func testSecureRedirectDelegate_redirect_allowsSecureTargetAndBlocksInsecureTarget() throws {
        let resolver = MockHostIPResolver(mapping: [
            "redirected.example.com": ["93.184.216.35"]
        ])
        let delegate = SecureRedirectDelegate(hostResolver: resolver)
        let session = URLSession(configuration: .ephemeral)
        let task = session.dataTask(with: try XCTUnwrap(URL(string: "https://initial.example.com/step1")))
        let redirectResponse = try XCTUnwrap(HTTPURLResponse(
            url: try XCTUnwrap(URL(string: "https://initial.example.com/step1")),
            statusCode: 302,
            httpVersion: nil,
            headerFields: nil
        ))
        let targetRequest = URLRequest(url: try XCTUnwrap(URL(string: "https://redirected.example.com/step2")))

        var nextRequest: URLRequest?
        delegate.urlSession(
            session,
            task: task,
            willPerformHTTPRedirection: redirectResponse,
            newRequest: targetRequest
        ) { req in
            nextRequest = req
        }

        XCTAssertNotNil(nextRequest)
        XCTAssertEqual(nextRequest?.url?.host, "redirected.example.com")
        XCTAssertFalse(delegate.encounteredInsecureRedirect)

        // Insecure redirect to loopback
        let badRequest = URLRequest(url: try XCTUnwrap(URL(string: "https://127.0.0.1/bad")))
        var rejectedRequest: URLRequest? = targetRequest
        delegate.urlSession(
            session,
            task: task,
            willPerformHTTPRedirection: redirectResponse,
            newRequest: badRequest
        ) { req in
            rejectedRequest = req
        }
        XCTAssertNil(rejectedRequest)
        XCTAssertTrue(delegate.encounteredInsecureRedirect)

        // Verify default platform certificate trust handling is preserved without custom challenge override
        XCTAssertFalse(
            delegate.responds(to: #selector(URLSessionTaskDelegate.urlSession(_:task:didReceive:completionHandler:))),
            "SecureRedirectDelegate should not implement custom challenge handler, leaving trust to system PKI"
        )
    }

    func testHTTPResponseParser_parsesContentLengthAndChunkedBodies() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))

        // Standard Content-Length response
        let rawContentLength = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 15\r\n\r\n{\"status\":\"OK\"}"
            .data(using: .utf8)!
        let (body1, resp1) = try HTTPResponseParser.parse(data: rawContentLength, url: url, cleanClose: true)
        XCTAssertEqual(resp1.statusCode, 200)
        XCTAssertEqual(String(data: body1, encoding: .utf8), "{\"status\":\"OK\"}")

        // Chunked Transfer-Encoding response
        let rawChunked = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
            .data(using: .utf8)!
        let (body2, resp2) = try HTTPResponseParser.parse(data: rawChunked, url: url, cleanClose: true)
        XCTAssertEqual(resp2.statusCode, 200)
        XCTAssertEqual(String(data: body2, encoding: .utf8), "hello world")

        // 404 response
        let raw404 = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found".data(using: .utf8)!
        let (body3, resp3) = try HTTPResponseParser.parse(data: raw404, url: url, cleanClose: true)
        XCTAssertEqual(resp3.statusCode, 404)
        XCTAssertEqual(String(data: body3, encoding: .utf8), "Not Found")
    }

    func testHTTPResponseParser_malformedChunkedEncoding_isRejected() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))
        let invalidPayloads = [
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7FFFFFFFFFFFFFFF\r\nAB\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n300000\r\nABC\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n-1\r\nX\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n+5\r\nhello\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n"
        ]
        for payload in invalidPayloads {
            XCTAssertThrowsError(try HTTPResponseParser.parse(
                data: Data(payload.utf8),
                url: url,
                cleanClose: true
            )) { error in
                XCTAssertEqual(error as? LNURLError, .invalidResponse)
            }
        }
    }

    func testHTTPResponseParser_invalidFramingAndHeaders_areRejected() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))
        let invalidPayloads = [
            "HTTP/2.0 200 OK\r\nContent-Length: 4\r\n\r\ntest",
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 10\r\nContent-Length: 20\r\n\r\n1234567890",
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort body",
            "HTTP/1.1 200 OK\r\nContent-Length: -5\r\n\r\nhello",
            "HTTP/1.1 200 OK\r\nContent-Length: 99999999999999999999\r\n\r\nhello"
        ]
        for payload in invalidPayloads {
            XCTAssertThrowsError(try HTTPResponseParser.parse(
                data: Data(payload.utf8),
                url: url,
                cleanClose: true
            )) { error in
                XCTAssertEqual(error as? LNURLError, .invalidResponse)
            }
        }
    }

    func testNWConnectionTransport_blocksInsecureEndpointsPreFlight() async throws {
        let resolver = MockHostIPResolver(mapping: [
            "private.example.com": ["10.0.0.1"],
            "loopback.example.com": ["127.0.0.1"]
        ])
        let transport = NWConnectionTransport(hostResolver: resolver)

        do {
            _ = try await transport.executeGet(url: try XCTUnwrap(URL(string: "https://private.example.com/test")))
            XCTFail("Expected insecureEndpoint error for private IP resolution")
        } catch LNURLError.insecureEndpoint {
            // Expected
        }

        do {
            _ = try await transport.executeGet(url: try XCTUnwrap(URL(string: "https://loopback.example.com/test")))
            XCTFail("Expected insecureEndpoint error for loopback resolution")
        } catch LNURLError.insecureEndpoint {
            // Expected
        }

        do {
            _ = try await transport.executeGet(url: try XCTUnwrap(URL(string: "https://127.0.0.1/test")))
            XCTFail("Expected insecureEndpoint error for literal loopback IP")
        } catch LNURLError.insecureEndpoint {
            // Expected
        }

        do {
            _ = try await transport.executeGet(url: try XCTUnwrap(URL(string: "https://user:pass@example.com/test")))
            XCTFail("Expected insecureEndpoint error for userinfo URL")
        } catch LNURLError.insecureEndpoint {
            // Expected
        }
    }

    func testNWConnectionTransport_dnsRebindingToPrivateIP_isRejectedAtSocketDial() async throws {
        let listener = try NWListener(using: .tcp, on: .any)
        final class AtomicFlag: @unchecked Sendable {
            private let lock = NSLock()
            private var value = false
            func set() { lock.withLock { value = true } }
            func get() -> Bool { lock.withLock { value } }
        }
        let listenerSawConnection = AtomicFlag()
        listener.newConnectionHandler = { conn in
            listenerSawConnection.set()
            conn.cancel()
        }
        let queue = DispatchQueue(label: "org.stablechannels.testlistener")
        listener.start(queue: queue)
        defer { listener.cancel() }

        var listenerPort: UInt16?
        for _ in 0..<100 {
            if let p = listener.port?.rawValue, p > 0 {
                listenerPort = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let port = try XCTUnwrap(listenerPort)

        final class StatefulResolver: HostIPResolving, @unchecked Sendable {
            var callCount = 0
            func resolveHostIPs(_: String) -> [String] {
                callCount += 1
                if callCount == 1 {
                    return ["93.184.216.34"] // Valid public IP on first call (validation)
                } else {
                    return ["127.0.0.1"] // Rebinds to loopback IP on second call (socket pin)
                }
            }
        }

        let resolver = StatefulResolver()
        let transport = NWConnectionTransport(hostResolver: resolver)
        let rebindURL = try XCTUnwrap(URL(string: "https://rebind.example.com:\(port)/test"))

        do {
            _ = try await transport.executeGet(url: rebindURL)
            XCTFail("Expected insecureEndpoint error when DNS rebinds to loopback")
        } catch LNURLError.insecureEndpoint {
            // Success: socket pinning check caught the rebinding and rejected before connecting!
        }
        XCTAssertEqual(resolver.callCount, 2)
        XCTAssertFalse(listenerSawConnection.get(), "Rebinding answer was contacted on listener!")
    }

    func testNWConnectionTransport_buildRequest_crlfInURLPath_isPreservedEncoded() throws {
        let injectedURL = try XCTUnwrap(URL(string: "https://attacker.example/a%0D%0AX-Injected:%201"))

        // Path contains encoded CRLF which percent-decoding would make dangerous, but buildRequest preserves encoding
        let reqData = try NWConnectionTransport.buildRequest(
            url: injectedURL,
            cleanHost: "attacker.example",
            portValue: 443,
            defaultPort: 443
        )
        let reqStr = try XCTUnwrap(String(data: reqData, encoding: .utf8))
        XCTAssertTrue(reqStr.contains("GET /a%0D%0AX-Injected:%201 HTTP/1.1\r\n"))
        XCTAssertTrue(reqStr.contains("Host: attacker.example\r\n"))
    }

    func testNWConnectionTransport_routesOnionEndpointsToOnionTransport() async throws {
        let mockOnion = MockSecureTransport()
        let onionURL = try XCTUnwrap(URL(string: "http://testpaypoint.onion/.well-known/lnurlp/alice"))
        let expectedResponse = try XCTUnwrap(HTTPURLResponse(
            url: onionURL,
            statusCode: 200,
            httpVersion: nil,
            headerFields: ["Content-Type": "application/json"]
        ))
        mockOnion.mockResult = (Data("{\"status\":\"OK\"}".utf8), expectedResponse)

        let transport = NWConnectionTransport(onionTransport: mockOnion)
        let (data, response) = try await transport.executeGet(url: onionURL)

        XCTAssertEqual(response.statusCode, 200)
        XCTAssertEqual(String(data: data, encoding: .utf8), "{\"status\":\"OK\"}")
        XCTAssertEqual(mockOnion.executedURL, onionURL)
    }

    func testNWConnectionTransport_onionEndpointRedirectingToClearnetHost_isVetoedAndNeverContacted(
    ) async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        final class ClearnetResolver: HostIPResolving, @unchecked Sendable {
            func resolveHostIPs(_: String) -> [String] {
                ["93.184.216.34"]
            }
        }

        let resolver = ClearnetResolver()
        let onionURL = try XCTUnwrap(URL(string: "http://attacker.onion/.well-known/lnurlp/alice"))
        let clearnetRedirectURL = try XCTUnwrap(URL(string: "https://rebinding.attacker.com/internal-api"))

        MockURLProtocol.seenURLs = []
        MockURLProtocol.requestHandler = { request in
            if request.url == onionURL {
                let response = HTTPURLResponse(
                    url: onionURL,
                    statusCode: 302,
                    httpVersion: nil,
                    headerFields: ["Location": clearnetRedirectURL.absoluteString]
                )!
                return (response, Data())
            } else {
                let response = HTTPURLResponse(
                    url: clearnetRedirectURL,
                    statusCode: 200,
                    httpVersion: nil,
                    headerFields: nil
                )!
                return (response, Data("EXPLOIT".utf8))
            }
        }

        let onionTransport = URLSessionTransport(urlSession: session, hostResolver: resolver)
        let transport = NWConnectionTransport(hostResolver: resolver, onionTransport: onionTransport)

        do {
            _ = try await transport.executeGet(url: onionURL)
            XCTFail("Expected insecureEndpoint error when onion redirects to clearnet host")
        } catch LNURLError.insecureEndpoint {
            // Expected: SecureRedirectDelegate vetoed clearnet redirect from onion service pre-hop
        }

        // Prove the private address / clearnet redirect URL is never contacted
        XCTAssertFalse(MockURLProtocol.seenURLs.contains(clearnetRedirectURL))
    }

    func testNWConnectionTransport_oversizedPort_throwsInvalidTargetWithoutCrashing() async throws {
        let resolver = MockHostIPResolver(mapping: ["example.com": ["93.184.216.34"]])
        let transport = NWConnectionTransport(hostResolver: resolver)

        let oversizedURL = try XCTUnwrap(URL(string: "https://example.com:99999/cb"))
        do {
            _ = try await transport.executeGet(url: oversizedURL)
            XCTFail("Expected invalidTarget error for port > 65535")
        } catch LNURLError.invalidTarget {
            // Expected: safely rejected without UInt16 integer overflow trap
        }

        let zeroPortURL = try XCTUnwrap(URL(string: "https://example.com:0/cb"))
        do {
            _ = try await transport.executeGet(url: zeroPortURL)
            XCTFail("Expected invalidTarget error for port 0")
        } catch LNURLError.invalidTarget {
            // Expected: port 0 rejected cleanly
        }
    }

    func testSecureEndpointValidator_canonicalNumericIP_variousInputs() {
        // Leading-zero dotted quad returns nil
        XCTAssertNil(SecureEndpointValidator.canonicalNumericIP("0177.0.0.1"))

        // Standard public IPv4 returns canonical string unchanged
        XCTAssertEqual(SecureEndpointValidator.canonicalNumericIP("8.8.8.8"), "8.8.8.8")

        // Uncompressed IPv6 compresses canonically
        XCTAssertEqual(
            SecureEndpointValidator.canonicalNumericIP("2606:4700:0:0:0:0:0:1111"),
            "2606:4700::1111"
        )

        // Scoped IPv6 address with zone identifier returns nil
        XCTAssertNil(SecureEndpointValidator.canonicalNumericIP("fe80::1%en0"))
    }

    func testResolveEndpoint_rejectsNonASCIIDomainHomograph() {
        // Cyrillic 'a' (U+0430) instead of ASCII 'a'
        let cyrillicDomain = "alice@ex\u{0430}mple.com"
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: cyrillicDomain)) { error in
            XCTAssertEqual(error as? LNURLError, .invalidTarget)
        }
    }

    // MARK: - Cancellation & NWConnectionSession Tests

    func testNWConnectionTransport_preCancelledTask_abortsBeforeDial() async throws {
        final class TrackingResolver: HostIPResolving, @unchecked Sendable {
            var callCount = 0
            func resolveHostIPs(_: String) -> [String] {
                callCount += 1
                return ["93.184.216.34"]
            }
        }
        let resolver = TrackingResolver()
        let transport = NWConnectionTransport(hostResolver: resolver)
        let testURL = try XCTUnwrap(URL(string: "https://example.com/test"))

        let task = Task { () -> (Data, HTTPURLResponse) in
            withUnsafeCurrentTask { $0?.cancel() }
            return try await transport.executeGet(url: testURL)
        }

        do {
            _ = try await task.value
            XCTFail("Expected CancellationError for pre-cancelled task")
        } catch is CancellationError {
            // Expected: aborted before dial
        } catch {
            XCTFail("Expected CancellationError, got \(error)")
        }
        XCTAssertEqual(resolver.callCount, 0, "Host resolver was contacted for pre-cancelled task!")
    }

    func testNWConnectionSession_roundTrip_succeedsOverLocalListener() async throws {
        let listener = try NWListener(using: .tcp, on: .any)
        let expectedResponse = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"OK\"}"
        listener.newConnectionHandler = { incoming in
            incoming.start(queue: .global())
            incoming.receive(minimumIncompleteLength: 1, maximumLength: 1024) { data, _, _, _ in
                if data != nil {
                    incoming.send(
                        content: Data(expectedResponse.utf8),
                        isComplete: true,
                        completion: .contentProcessed { _ in
                            incoming.cancel()
                        }
                    )
                }
            }
        }
        let listenerQueue = DispatchQueue(label: "org.stablechannels.testsession")
        listener.start(queue: listenerQueue)
        defer { listener.cancel() }

        var portValue: UInt16?
        for _ in 0..<100 {
            if let p = listener.port?.rawValue, p > 0 {
                portValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let port = try XCTUnwrap(portValue)

        let endpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: try XCTUnwrap(NWEndpoint.Port(rawValue: port)))
        let connection = NWConnection(to: endpoint, using: .tcp)
        let requestData = Data("GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".utf8)

        let result = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<
            (data: Data, cleanClose: Bool),
            Error
        >) in
            NWConnectionSession(
                connection: connection,
                requestData: requestData,
                timeoutInterval: 5.0,
                queue: listenerQueue,
                continuation: continuation
            ).start()
        }

        XCTAssertTrue(result.cleanClose)
        let (body, response) = try HTTPResponseParser.parse(
            data: result.data,
            url: try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/test")),
            cleanClose: result.cleanClose
        )
        XCTAssertEqual(response.statusCode, 200)
        XCTAssertEqual(String(data: body, encoding: .utf8), "{\"status\":\"OK\"}")
    }

    func testNWConnectionSession_uncleanClose_returnsCleanCloseFalse() async throws {
        let serverFd = Darwin.socket(AF_INET, SOCK_STREAM, 0)
        var sin = sockaddr_in()
        sin.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        sin.sin_family = sa_family_t(AF_INET)
        sin.sin_port = 0
        sin.sin_addr.s_addr = inet_addr("127.0.0.1")
        withUnsafePointer(to: &sin) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                _ = Darwin.bind(serverFd, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        Darwin.listen(serverFd, 1)
        var len = socklen_t(MemoryLayout<sockaddr_in>.size)
        Darwin.getsockname(serverFd, withUnsafeMutablePointer(to: &sin) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { $0 }
        }, &len)
        let port = UInt16(bigEndian: sin.sin_port)
        defer { Darwin.close(serverFd) }

        DispatchQueue.global().async {
            var clientSin = sockaddr_in()
            var clientLen = socklen_t(MemoryLayout<sockaddr_in>.size)
            let clientFd = Darwin.accept(serverFd, withUnsafeMutablePointer(to: &clientSin) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { $0 }
            }, &clientLen)
            var buf = [UInt8](repeating: 0, count: 1024)
            _ = Darwin.read(clientFd, &buf, 1024)
            let payload = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"part"
            _ = Darwin.write(clientFd, payload, payload.utf8.count)
            var sl = linger(l_onoff: 1, l_linger: 0)
            Darwin.setsockopt(clientFd, SOL_SOCKET, SO_LINGER, &sl, socklen_t(MemoryLayout<linger>.size))
            Darwin.close(clientFd)
        }

        let endpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: try XCTUnwrap(NWEndpoint.Port(rawValue: port)))
        let connection = NWConnection(to: endpoint, using: .tcp)
        let requestData = Data("GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".utf8)
        let queue = DispatchQueue(label: "org.stablechannels.uncleanclose")

        let result = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<
            (data: Data, cleanClose: Bool),
            Error
        >) in
            NWConnectionSession(
                connection: connection,
                requestData: requestData,
                timeoutInterval: 5.0,
                queue: queue,
                continuation: continuation
            ).start()
        }

        XCTAssertFalse(result.cleanClose)
        let testURL = try XCTUnwrap(URL(string: "http://127.0.0.1:\(port)/test"))
        XCTAssertThrowsError(try HTTPResponseParser.parse(
            data: result.data,
            url: testURL,
            cleanClose: result.cleanClose
        )) { error in
            XCTAssertEqual(error as? LNURLError, .invalidResponse)
        }
    }

    func testNWConnectionSession_timeout_throwsNetworkError() async throws {
        let listener = try NWListener(using: .tcp, on: .any)
        listener.newConnectionHandler = { incoming in
            incoming.start(queue: .global())
        }
        let listenerQueue = DispatchQueue(label: "org.stablechannels.testsession")
        listener.start(queue: listenerQueue)
        defer { listener.cancel() }

        var portValue: UInt16?
        for _ in 0..<100 {
            if let p = listener.port?.rawValue, p > 0 {
                portValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let port = try XCTUnwrap(portValue)

        let endpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: try XCTUnwrap(NWEndpoint.Port(rawValue: port)))
        let connection = NWConnection(to: endpoint, using: .tcp)
        let requestData = Data("GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".utf8)

        do {
            _ = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<
                (data: Data, cleanClose: Bool),
                Error
            >) in
                NWConnectionSession(
                    connection: connection,
                    requestData: requestData,
                    timeoutInterval: 0.1,
                    queue: listenerQueue,
                    continuation: continuation
                ).start()
            }
            XCTFail("Expected timeout network error")
        } catch let LNURLError.networkError(msg) {
            XCTAssertTrue(msg.contains("timed out"))
        } catch {
            XCTFail("Expected networkError, got \(error)")
        }
    }

    func testNWConnectionSession_taskCancellation_abortsConnection() async throws {
        let listener = try NWListener(using: .tcp, on: .any)
        listener.newConnectionHandler = { incoming in
            incoming.start(queue: .global())
        }
        let listenerQueue = DispatchQueue(label: "org.stablechannels.testsession")
        listener.start(queue: listenerQueue)
        defer { listener.cancel() }

        var portValue: UInt16?
        for _ in 0..<100 {
            if let p = listener.port?.rawValue, p > 0 {
                portValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let port = try XCTUnwrap(portValue)

        let endpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: try XCTUnwrap(NWEndpoint.Port(rawValue: port)))
        let connection = NWConnection(to: endpoint, using: .tcp)
        let requestData = Data("GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".utf8)

        final class Holder: @unchecked Sendable {
            var session: NWConnectionSession?
        }
        let holder = Holder()

        let task = Task { () -> (data: Data, cleanClose: Bool) in
            try await withTaskCancellationHandler {
                try await withCheckedThrowingContinuation { continuation in
                    let session = NWConnectionSession(
                        connection: connection,
                        requestData: requestData,
                        timeoutInterval: 10.0,
                        queue: listenerQueue,
                        continuation: continuation
                    )
                    holder.session = session
                    session.start()
                }
            } onCancel: {
                holder.session?.cancel()
            }
        }

        try await Task.sleep(nanoseconds: 30_000_000)
        task.cancel()

        do {
            _ = try await task.value
            XCTFail("Expected CancellationError when task is cancelled")
        } catch is CancellationError {
            // Success: connection cancelled cleanly
        } catch {
            XCTFail("Expected CancellationError, got \(error)")
        }
    }

    func testNWConnectionSession_oversizedResponse_abortsWithInvalidResponse() async throws {
        let listener = try NWListener(using: .tcp, on: .any)
        listener.newConnectionHandler = { incoming in
            incoming.start(queue: .global())
            incoming.receive(minimumIncompleteLength: 1, maximumLength: 1024) { data, _, _, _ in
                if data != nil {
                    let oversized = Data(repeating: 0x41, count: 2 * 1024 * 1024 + 1024)
                    incoming.send(content: oversized, completion: .contentProcessed { _ in
                        incoming.cancel()
                    })
                }
            }
        }
        let listenerQueue = DispatchQueue(label: "org.stablechannels.testsession")
        listener.start(queue: listenerQueue)
        defer { listener.cancel() }

        var portValue: UInt16?
        for _ in 0..<100 {
            if let p = listener.port?.rawValue, p > 0 {
                portValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let port = try XCTUnwrap(portValue)

        let endpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: try XCTUnwrap(NWEndpoint.Port(rawValue: port)))
        let connection = NWConnection(to: endpoint, using: .tcp)
        let requestData = Data("GET /test HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n".utf8)

        do {
            _ = try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<
                (data: Data, cleanClose: Bool),
                Error
            >) in
                NWConnectionSession(
                    connection: connection,
                    requestData: requestData,
                    timeoutInterval: 5.0,
                    queue: listenerQueue,
                    continuation: continuation
                ).start()
            }
            XCTFail("Expected invalidResponse for oversized response")
        } catch LNURLError.invalidResponse {
            // Expected
        } catch {
            XCTFail("Expected invalidResponse, got \(error)")
        }
    }

    // MARK: - Unframed Body Clean Close Verification

    func testHTTPResponseParser_unframedBody_cleanCloseRequired() throws {
        let testURL = try XCTUnwrap(URL(string: "https://example.com/api"))
        let rawHTTP = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"status\":\"OK\"}"
        let data = Data(rawHTTP.utf8)

        // When cleanClose is true, unframed body parses successfully
        let (bodyClean, responseClean) = try HTTPResponseParser.parse(data: data, url: testURL, cleanClose: true)
        XCTAssertEqual(responseClean.statusCode, 200)
        XCTAssertEqual(String(data: bodyClean, encoding: .utf8), "{\"status\":\"OK\"}")

        // When cleanClose is false (connection dropped mid-stream), unframed body is rejected
        XCTAssertThrowsError(try HTTPResponseParser.parse(data: data, url: testURL, cleanClose: false)) { error in
            XCTAssertEqual(error as? LNURLError, .invalidResponse)
        }
    }

    func testHTTPResponseParser_framedBody_toleratesCleanCloseFalseIfDataComplete() throws {
        let testURL = try XCTUnwrap(URL(string: "https://example.com/api"))

        // Content-Length framed response: complete data is accepted even if socket close was unclean
        let clHTTP = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"OK\"}"
        let clData = Data(clHTTP.utf8)
        let (bodyCL, responseCL) = try HTTPResponseParser.parse(data: clData, url: testURL, cleanClose: false)
        XCTAssertEqual(responseCL.statusCode, 200)
        XCTAssertEqual(String(data: bodyCL, encoding: .utf8), "{\"status\":\"OK\"}")

        // Chunked framed response: complete termination is accepted even if socket close was unclean
        let chunkedHTTP = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nF\r\n{\"status\":\"OK\"}\r\n0\r\n\r\n"
        let chunkedData = Data(chunkedHTTP.utf8)
        let (bodyChunked, responseChunked) = try HTTPResponseParser.parse(
            data: chunkedData,
            url: testURL,
            cleanClose: false
        )
        XCTAssertEqual(responseChunked.statusCode, 200)
        XCTAssertEqual(String(data: bodyChunked, encoding: .utf8), "{\"status\":\"OK\"}")
    }

    // MARK: - Trailing-Dot Host Validation

    func testSecureEndpointValidator_trailingDotHosts_areBlockedByStaticBlocklist() throws {
        XCTAssertEqual(SecureEndpointValidator.cleanHostString("localhost."), "localhost")
        XCTAssertEqual(SecureEndpointValidator.cleanHostString("service.internal."), "service.internal")
        XCTAssertEqual(SecureEndpointValidator.cleanHostString("router.local."), "router.local")
        XCTAssertEqual(SecureEndpointValidator.cleanHostString("myhost.localhost."), "myhost.localhost")

        // Blocked at static validation stage before DNS lookup
        let loopbackDotURL = try XCTUnwrap(URL(string: "https://localhost./pay"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: loopbackDotURL))

        let internalDotURL = try XCTUnwrap(URL(string: "https://server.internal./pay"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: internalDotURL))

        let localDotURL = try XCTUnwrap(URL(string: "https://gateway.local./pay"))
        XCTAssertFalse(SecureEndpointValidator.isSecureEndpoint(url: localDotURL))
    }

    // MARK: - Appended Query Parameter Encoding

    func testFetchInvoice_preservesPreExistingQueryPlus_andEncodesAppendedPlus() async throws {
        let callbackURL = try XCTUnwrap(URL(string: "https://example.com/api?tag=pay&search=hello+world"))
        let mockTransport = MockSecureTransport()
        let invoiceData = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}".data(using: .utf8)!
        let httpResponse = try XCTUnwrap(HTTPURLResponse(
            url: callbackURL,
            statusCode: 200,
            httpVersion: nil,
            headerFields: nil
        ))
        mockTransport.mockResult = (invoiceData, httpResponse)

        let service = LNURLService(transport: mockTransport)
        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")

        _ = try await service.fetchInvoice(
            params: params,
            amountMsat: 50_000,
            comment: "tip+bonus=yes;extra"
        )

        guard let executedURL = mockTransport.executedURL else {
            XCTFail("No URL executed by transport")
            return
        }

        let query = executedURL.query(percentEncoded: true) ?? ""
        // Pre-existing search query should keep its '+' intact
        XCTAssertTrue(
            query.contains("search=hello+world"),
            "Expected pre-existing query '+' to be preserved, got \(query)"
        )
        // Appended comment with '+', '=', and ';' should have them percent-encoded
        XCTAssertTrue(
            query.contains("comment=tip%2Bbonus%3Dyes%3Bextra"),
            "Expected appended chars to be encoded as %2B, %3D, %3B, got \(query)"
        )
        XCTAssertTrue(query.contains("amount=50000"))
    }

    // MARK: - Bolt11 Invoice Validation & Network Check

    func testFetchInvoice_malformedBolt11_mapsToInvalidResponse() async throws {
        let callbackURL = try XCTUnwrap(URL(string: "https://example.com/callback"))
        let mockTransport = MockSecureTransport()
        // pr is malformed and fails Bolt11Invoice.fromStr
        let invoiceData = "{\"pr\":\"lnbcnotaninvoice12345\",\"status\":\"OK\"}".data(using: .utf8)!
        let httpResponse = try XCTUnwrap(HTTPURLResponse(
            url: callbackURL,
            statusCode: 200,
            httpVersion: nil,
            headerFields: nil
        ))
        mockTransport.mockResult = (invoiceData, httpResponse)

        let service = LNURLService(transport: mockTransport)
        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")

        do {
            _ = try await service.fetchInvoice(params: params, amountMsat: 50_000)
            XCTFail("Expected LNURLError.invalidResponse for malformed bolt11 string")
        } catch LNURLError.invalidResponse {
            // Expected: correctly mapped to invalidResponse instead of escaping LDK error
        } catch {
            XCTFail("Expected LNURLError.invalidResponse, got \(error)")
        }
    }

    func testFetchInvoice_invoiceNetworkMismatch_throwsErrorResponse() async throws {
        let callbackURL = try XCTUnwrap(URL(string: "https://example.com/callback"))
        let mockTransport = MockSecureTransport()
        // valid50kInvoice is a regtest invoice (lnbcrt...)
        let invoiceData = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}".data(using: .utf8)!
        let httpResponse = try XCTUnwrap(HTTPURLResponse(
            url: callbackURL,
            statusCode: 200,
            httpVersion: nil,
            headerFields: nil
        ))
        mockTransport.mockResult = (invoiceData, httpResponse)

        // Configure service expecting testnet (mismatches regtest invoice)
        let service = LNURLService(transport: mockTransport, expectedNetwork: .testnet)
        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")

        do {
            _ = try await service.fetchInvoice(params: params, amountMsat: 50_000)
            XCTFail("Expected errorResponse when invoice currency does not match wallet network")
        } catch let LNURLError.errorResponse(reason) {
            XCTAssertTrue(reason.contains("network"))
        } catch {
            XCTFail("Expected LNURLError.errorResponse, got \(error)")
        }
    }

    // MARK: - Error Classification

    func testResolveEndpoint_oversizedPort_throwsInvalidTarget() {
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "https://example.com:99999/pay")) { error in
            XCTAssertEqual(error as? LNURLError, .invalidTarget)
        }
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "https://example.com:0/pay")) { error in
            XCTAssertEqual(error as? LNURLError, .invalidTarget)
        }
    }

    func testFetchInvoice_overlongComment_throwsInvalidComment() async throws {
        let callbackURL = try XCTUnwrap(URL(string: "https://example.com/callback"))
        let mockTransport = MockSecureTransport()
        let service = LNURLService(transport: mockTransport)
        let params = makeParams(callback: callbackURL.absoluteString, metadata: "", commentAllowed: 10)

        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: "this comment is far too long for limit 10"
            )
            XCTFail("Expected invalidComment error for comment exceeding limit")
        } catch LNURLError.invalidComment {
            // Expected
        } catch {
            XCTFail("Expected LNURLError.invalidComment, got \(error)")
        }
    }

    func testFetchInvoice_unparsableCallback_throwsInvalidTarget() async throws {
        let mockTransport = MockSecureTransport()
        let service = LNURLService(transport: mockTransport)
        let params = makeParams(callback: "not a valid url at all", metadata: "")

        do {
            _ = try await service.fetchInvoice(params: params, amountMsat: 50_000)
            XCTFail("Expected invalidTarget for unparsable callback")
        } catch LNURLError.invalidTarget {
            // Expected
        } catch {
            XCTFail("Expected LNURLError.invalidTarget, got \(error)")
        }
    }

    // MARK: - Pin Selection & Redirect Policy

    func testNWConnectionTransport_selectPinnedIP_prefersIPv4() throws {
        let ips = ["2606:4700::1111", "93.184.216.34", "2606:4700::2222"]
        let selected = try NWConnectionTransport.selectPinnedIP(for: "example.com", resolvedIPs: ips)
        XCTAssertEqual(selected, "93.184.216.34")
    }

    func testNWConnectionTransport_selectPinnedIP_acceptsIPv6WhenOnlyIPv6Available() throws {
        let ips = ["2606:4700::1111", "2606:4700::2222"]
        let selected = try NWConnectionTransport.selectPinnedIP(for: "example.com", resolvedIPs: ips)
        XCTAssertEqual(selected, "2606:4700::1111")
    }

    func testNWConnectionTransport_selectPinnedIP_failsClosedOnPrivateOrMixedIPs() {
        // Pure private IP fails
        XCTAssertThrowsError(
            try NWConnectionTransport.selectPinnedIP(for: "internal.example", resolvedIPs: ["10.0.0.1"])
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }

        // Mixed public and private IP fails closed (consistent with validate-time reject-all)
        XCTAssertThrowsError(
            try NWConnectionTransport.selectPinnedIP(
                for: "split.example",
                resolvedIPs: ["93.184.216.34", "127.0.0.1"]
            )
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }
    }

    func testNWConnectionTransport_selectPinnedIP_failsClosedOnEmptyIPs() {
        XCTAssertThrowsError(
            try NWConnectionTransport.selectPinnedIP(for: "missing.example", resolvedIPs: [])
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }
    }

    func testNWConnectionTransport_selectPinnedIP_canonicalPublicNumericIP() throws {
        let selected = try NWConnectionTransport.selectPinnedIP(for: "8.8.8.8", resolvedIPs: [])
        XCTAssertEqual(selected, "8.8.8.8")

        // Leading-zero non-canonical IP fails
        XCTAssertThrowsError(
            try NWConnectionTransport.selectPinnedIP(for: "0177.0.0.1", resolvedIPs: [])
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }
    }

    func testNWConnectionTransport_validateRedirectTarget_hopLimitAndLocation() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://example.com/api/v1/pay"))

        // Relative redirect within limit
        let target = try NWConnectionTransport.validateRedirectTarget(
            currentURL: baseURL,
            locationHeader: "/api/v2/pay",
            hop: 0,
            hostResolver: MockHostIPResolver(mapping: ["example.com": ["93.184.216.34"]])
        )
        XCTAssertEqual(target.absoluteString, "https://example.com/api/v2/pay")

        // Exceeding hop limit fails with networkError
        XCTAssertThrowsError(
            try NWConnectionTransport.validateRedirectTarget(
                currentURL: baseURL,
                locationHeader: "/api/v2/pay",
                hop: 3,
                hostResolver: MockHostIPResolver(mapping: ["example.com": ["93.184.216.34"]])
            )
        ) { error in
            guard case let LNURLError.networkError(msg) = error else {
                XCTFail("Expected networkError, got \(error)")
                return
            }
            XCTAssertTrue(msg.contains("Too many redirects"))
        }

        // Missing location header throws invalidResponse
        XCTAssertThrowsError(
            try NWConnectionTransport.validateRedirectTarget(
                currentURL: baseURL,
                locationHeader: nil,
                hop: 0,
                hostResolver: MockHostIPResolver(mapping: ["example.com": ["93.184.216.34"]])
            )
        ) { error in
            XCTAssertEqual(error as? LNURLError, .invalidResponse)
        }
    }

    func testNWConnectionTransport_validateRedirectTarget_clearnetToOnionVetoed() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://example.com/pay"))
        XCTAssertThrowsError(
            try NWConnectionTransport.validateRedirectTarget(
                currentURL: baseURL,
                locationHeader: "http://service.onion/pay",
                hop: 0
            )
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }
    }

    func testNWConnectionTransport_validateRedirectTarget_httpsToHttpDowngradeVetoed() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://example.com/pay"))
        XCTAssertThrowsError(
            try NWConnectionTransport.validateRedirectTarget(
                currentURL: baseURL,
                locationHeader: "http://example.com/pay",
                hop: 0,
                hostResolver: MockHostIPResolver(mapping: ["example.com": ["93.184.216.34"]])
            )
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }
    }

    func testNWConnectionTransport_validateRedirectTarget_privateIPRedirectVetoed() throws {
        let baseURL = try XCTUnwrap(URL(string: "https://example.com/pay"))
        XCTAssertThrowsError(
            try NWConnectionTransport.validateRedirectTarget(
                currentURL: baseURL,
                locationHeader: "https://internal.lan/pay",
                hop: 0,
                hostResolver: MockHostIPResolver(mapping: ["internal.lan": ["10.0.0.1"]])
            )
        ) { error in
            XCTAssertEqual(error as? LNURLError, .insecureEndpoint)
        }
    }

    // MARK: - Onion Transport Routing

    func testLNURLService_defaultInitWithoutOnionTransport_onionEndpointThrowsProxyRequired() async throws {
        let service = LNURLService(expectedNetwork: .regtest)
        let onionURL = try XCTUnwrap(URL(string: "http://service.onion/lnurlp"))

        do {
            _ = try await service.fetchPayParams(from: onionURL)
            XCTFail("Expected networkError when fetching onion endpoint without configured onion transport")
        } catch let LNURLError.networkError(message) {
            XCTAssertTrue(
                message.contains("onion proxy transport"),
                "Expected error message mentioning onion proxy transport, got: \(message)"
            )
        } catch {
            XCTFail("Expected LNURLError.networkError, got \(error)")
        }
    }

    func testLNURLService_initWithOnionTransport_delegatesOnionRequestToOnionTransport() async throws {
        let onionURL = try XCTUnwrap(URL(string: "http://service.onion/lnurlp"))
        let mockOnion = MockSecureTransport()
        let validPayResponse = "{\"tag\":\"payRequest\",\"callback\":\"http://service.onion/callback\",\"minSendable\":1000,\"maxSendable\":100000000,\"metadata\":\"[[\\\"text/plain\\\",\\\"test\\\"]]\"}"
        let httpResponse = try XCTUnwrap(HTTPURLResponse(
            url: onionURL,
            statusCode: 200,
            httpVersion: nil,
            headerFields: nil
        ))
        mockOnion.mockResult = (Data(validPayResponse.utf8), httpResponse)

        let service = LNURLService(
            expectedNetwork: .regtest,
            onionTransport: mockOnion
        )

        let params = try await service.fetchPayParams(from: onionURL)
        XCTAssertEqual(mockOnion.executedURL, onionURL)
        XCTAssertEqual(params.callback, "http://service.onion/callback")
    }

    // MARK: - Virtual-Hosted TLS SNI Verification

    func testNWConnectionTransport_virtualHostedTLS_routesSelectedCertificateBySNI() async throws {
        let identityA = try XCTUnwrap(Self.loadTestIdentity(base64: Self.testCertP12Base64A))
        let identityB = try XCTUnwrap(Self.loadTestIdentity(base64: Self.testCertP12Base64B))

        let listenerQueue = DispatchQueue(label: "org.stablechannels.testtlslistener")
        let listenerTLS = NWProtocolTLS.Options()
        sec_protocol_options_set_challenge_block(listenerTLS.securityProtocolOptions, { metadata, completion in
            let presentedSNI = sec_protocol_metadata_get_server_name(metadata).map { String(cString: $0) }
            if presentedSNI == "vhost-a.example" {
                completion(identityA)
            } else if presentedSNI == "vhost-b.example" {
                completion(identityB)
            } else {
                completion(nil)
            }
        }, listenerQueue)

        let listener = try NWListener(
            using: NWParameters(tls: listenerTLS, tcp: NWProtocolTCP.Options()),
            on: .any
        )
        listener.newConnectionHandler = { incoming in
            incoming.start(queue: listenerQueue)
            incoming.receive(minimumIncompleteLength: 1, maximumLength: 1024) { data, _, _, _ in
                if data != nil {
                    incoming.send(
                        content: Data("HTTP/1.1 200 OK\r\nContent-Length: 15\r\n\r\n{\"status\":\"OK\"}".utf8),
                        isComplete: true,
                        completion: .contentProcessed { _ in incoming.cancel() }
                    )
                }
            }
        }
        listener.start(queue: listenerQueue)
        defer { listener.cancel() }

        var portValue: UInt16?
        for _ in 0..<100 {
            if let p = listener.port?.rawValue, p > 0 {
                portValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let port = try XCTUnwrap(portValue)
        let endpointPort = try XCTUnwrap(NWEndpoint.Port(rawValue: port))

        for host in ["vhost-a.example", "vhost-b.example"] {
            final class CertCapture: @unchecked Sendable {
                let lock = NSLock()
                var commonName: String?
                func set(_ name: String?) { lock.withLock { commonName = name } }
                func get() -> String? { lock.withLock { commonName } }
            }
            let capture = CertCapture()

            let resolver = MockHostIPResolver(mapping: [host: ["93.184.216.34"]])
            let transport = NWConnectionTransport(hostResolver: resolver, timeoutInterval: 5.0)

            let clientQueue = DispatchQueue(label: "org.stablechannels.testtlsclient.\(host)")
            transport.dialTargetOverride = { _, params in
                if let tlsOpts = params.defaultProtocolStack.applicationProtocols.first(where: {
                    $0 is NWProtocolTLS.Options
                }) as? NWProtocolTLS.Options {
                    sec_protocol_options_set_verify_block(tlsOpts.securityProtocolOptions, { _, secTrust, completion in
                        let serverTrust = sec_trust_copy_ref(secTrust).takeRetainedValue()
                        if let chain = SecTrustCopyCertificateChain(serverTrust) as? [SecCertificate],
                           let cert = chain.first,
                           let summary = SecCertificateCopySubjectSummary(cert) as String? {
                            capture.set(summary)
                        }
                        completion(true)
                    }, clientQueue)
                }
                let localEndpoint = NWEndpoint.hostPort(host: "127.0.0.1", port: endpointPort)
                return (localEndpoint, params)
            }

            let testURL = try XCTUnwrap(URL(string: "https://\(host)/pay"))
            let (body, response) = try await transport.executeGet(url: testURL)

            XCTAssertEqual(response.statusCode, 200)
            XCTAssertEqual(String(data: body, encoding: .utf8), "{\"status\":\"OK\"}")
            XCTAssertEqual(capture.get(), host, "Expected TLS server to present SNI-selected certificate for \(host)")
        }
    }

    // MARK: - Redirect Rebinding Acceptance Verification

    func testNWConnectionTransport_redirectHopToRebindingHost_isVetoedAndTargetListenerNeverContacted() async throws {
        // Listener 1: origin listener that returns 302 redirect to target
        let originListener = try NWListener(using: .tcp, on: .any)
        let originQueue = DispatchQueue(label: "org.stablechannels.testorigin")
        originListener.newConnectionHandler = { conn in
            conn.start(queue: originQueue)
            conn.receive(minimumIncompleteLength: 1, maximumLength: 1024) { data, _, _, _ in
                if data != nil {
                    let resp = "HTTP/1.1 302 Found\r\nLocation: https://target.example.com/redirect\r\nContent-Length: 0\r\n\r\n"
                    conn.send(content: Data(resp.utf8), isComplete: true, completion: .contentProcessed { _ in
                        conn.cancel()
                    })
                }
            }
        }
        originListener.start(queue: originQueue)
        defer { originListener.cancel() }

        var originPortValue: UInt16?
        for _ in 0..<100 {
            if let p = originListener.port?.rawValue, p > 0 {
                originPortValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let originPort = try XCTUnwrap(originPortValue)
        let originEndpointPort = try XCTUnwrap(NWEndpoint.Port(rawValue: originPort))

        // Listener 2: target listener that must NEVER be contacted
        let targetListener = try NWListener(using: .tcp, on: .any)
        final class AtomicFlag: @unchecked Sendable {
            private let lock = NSLock()
            private var value = false
            func set() { lock.withLock { value = true } }
            func get() -> Bool { lock.withLock { value } }
        }
        let targetSawConnection = AtomicFlag()
        targetListener.newConnectionHandler = { conn in
            targetSawConnection.set()
            conn.cancel()
        }
        let targetQueue = DispatchQueue(label: "org.stablechannels.testtarget")
        targetListener.start(queue: targetQueue)
        defer { targetListener.cancel() }

        var targetPortValue: UInt16?
        for _ in 0..<100 {
            if let p = targetListener.port?.rawValue, p > 0 {
                targetPortValue = p
                break
            }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        let targetPort = try XCTUnwrap(targetPortValue)
        let targetEndpointPort = try XCTUnwrap(NWEndpoint.Port(rawValue: targetPort))

        final class RedirectStatefulResolver: HostIPResolving, @unchecked Sendable {
            private let lock = NSLock()
            private var count = 0
            func resolveHostIPs(_ host: String) -> [String] {
                lock.withLock {
                    if host == "origin.example.com" {
                        return ["93.184.216.34"]
                    }
                    count += 1
                    if count == 1 {
                        return ["93.184.216.34"] // Valid public answer during pre-flight redirect check
                    } else {
                        return ["127.0.0.1"] // Rebinds to loopback on connection attempt
                    }
                }
            }

            func getCallCount() -> Int { lock.withLock { count } }
        }

        let resolver = RedirectStatefulResolver()
        let transport = NWConnectionTransport(hostResolver: resolver, timeoutInterval: 5.0)

        // Seam routes origin host to local originListener and target host to local targetListener
        transport.dialTargetOverride = { endpoint, params in
            switch endpoint {
            case let .hostPort(host, _):
                if host == "93.184.216.34" {
                    return (.hostPort(host: "127.0.0.1", port: originEndpointPort), NWParameters.tcp)
                } else {
                    return (.hostPort(host: "127.0.0.1", port: targetEndpointPort), NWParameters.tcp)
                }
            default:
                return (endpoint, params)
            }
        }

        let originURL = try XCTUnwrap(URL(string: "https://origin.example.com/pay"))
        do {
            _ = try await transport.executeGet(url: originURL)
            XCTFail("Expected insecureEndpoint when redirect target rebinds to loopback")
        } catch LNURLError.insecureEndpoint {
            // Expected: hop 1 rebind was intercepted and vetoed by selectPinnedIP/isSecureEndpoint
        }

        XCTAssertFalse(targetSawConnection.get(), "Target listener was contacted across redirect hop!")
        XCTAssertEqual(resolver.getCallCount(), 2, "Expected target host to be resolved twice (pre-flight and dial)")
    }

    func testNWConnectionTransport_20DigitPort_throwsInvalidTargetWithoutCrashing() async throws {
        let resolver = MockHostIPResolver(mapping: ["example.com": ["93.184.216.34"]])
        let transport = NWConnectionTransport(hostResolver: resolver)

        let port20URL = try XCTUnwrap(URL(string: "https://example.com:99999999999999999999/x"))
        do {
            _ = try await transport.executeGet(url: port20URL)
            XCTFail("Expected invalidTarget for 20-digit port")
        } catch LNURLError.invalidTarget {
            // Success: unrepresentable port token rejected without coercion to 443
        }

        XCTAssertThrowsError(try LNURLService
            .resolveEndpoint(from: "https://example.com:99999999999999999999/pay")) { error in
                XCTAssertEqual(error as? LNURLError, .invalidTarget)
            }
    }

    func testHTTPResponseParser_negativeStatusCode_andNonStandardHTTPVersion_areRejected() throws {
        let testURL = try XCTUnwrap(URL(string: "https://example.com/api"))

        let negativeStatus = "HTTP/1.1 -824 OK\r\nContent-Length: 2\r\n\r\nOK"
        XCTAssertThrowsError(try HTTPResponseParser.parse(
            data: Data(negativeStatus.utf8),
            url: testURL,
            cleanClose: true
        )) { error in
            XCTAssertEqual(error as? LNURLError, .invalidResponse)
        }

        let nonStandardVersion = "HTTP/1.9 200 OK\r\nContent-Length: 2\r\n\r\nOK"
        XCTAssertThrowsError(try HTTPResponseParser.parse(
            data: Data(nonStandardVersion.utf8),
            url: testURL,
            cleanClose: true
        )) { error in
            XCTAssertEqual(error as? LNURLError, .invalidResponse)
        }
    }

    private static func loadTestIdentity(base64: String) -> OS_sec_identity? {
        guard let data = Data(base64Encoded: base64) else { return nil }
        let options: [String: Any] = [kSecImportExportPassphrase as String: "testpass"]
        var rawItems: CFArray?
        guard SecPKCS12Import(data as CFData, options as CFDictionary, &rawItems) == 0,
              let items = rawItems as? [[String: Any]],
              let first = items.first,
              let secIdentity = first[kSecImportItemIdentity as String] else {
            return nil
        }
        return sec_identity_create(secIdentity as! SecIdentity)
    }

    private static let testCertP12Base64A =
        "MIIFyQIBAzCCBY8GCSqGSIb3DQEHAaCCBYAEggV8MIIFeDCCAncGCSqGSIb3DQEHBqCCAmgwggJkAgEAMIICXQYJKoZIhvcNAQcBMBwGCiqGSIb3DQEMAQYwDgQI1UMnzxzUBcYCAggAgIICMEYkvm5QtYlmxe1xREWUwvg8bBkg3f7nTmpDpzqaRUrdsmBv572O+kXXFh5Z21gkMiZ4FDzJo6MfxQr3wgOHPAT1QntQAI0enPcrqEXr8q6M4zOj4GYTjm6D6aNN8QPkBVhKekQIlbA+ddjjT4WO3+8sk4EIIy9pvy9Ltsd7J2rbq6nCw4bNRscSKWBWY68yH7o4+3SjRpg/1nyKHWHGMgCKIrds43dR8rZMNJ5x+y9kk+/x+o13456VVim6VCSW+qxw+fKUgvwectpEvwTXr7BqTPut9BRBks8VNk/49c+G4MCGDeGOu2f7bU1e/cOcay01tzuPefWM3yp2w1IF9UXgNobNfHZrtJtebnleU3G6yg5V804pMihKBfGWl8hUNx713NzVvMah7pc+1Zkv6Kp62Od+iwc1At3mighK746Uxk3c/BsEEJSdk7Qa/cwIRhFfeuP8wydEnxrLDtI/fSC8ymm94Ar/0ZPE+yKbM2nIG8ykIou1IbhprfZz9ibDsID3WObmzOLuV3vCZ4ravSGTYZ8lALoDh/P+ZVF2Ma5XIvkINv6AyGBB14iXXTEl+44No+iTR0DmqDBTdMG1KxrOirFwY/8v6EIFByfw3zNCudOhDqsgmVmNfcvPQ55byF3JmhXSkk2I7y/VYuzSZSuq6oSi9oR+NMqlxHfTGCiWRGNg6Ty7eJdi0J5ZnKRKsTFawXyXwJ4lxMmuuYxTzXycY5e38gqa7DVKd0LCQv/zMIIC+QYJKoZIhvcNAQcBoIIC6gSCAuYwggLiMIIC3gYLKoZIhvcNAQwKAQKgggKmMIICojAcBgoqhkiG9w0BDAEDMA4ECKbHyQYqDoJOAgIIAASCAoBqT+X2x8RgJ8q71zW6DiIhhLsUpdgSAP8YHWUpPAeTjCIIOvOVyh5Jlv1eJp1bM5fyqQOzPwAh26g+OAWWy6ssZrzkW3KoR/vTJwNBskjSkNHNQEkpYq9Bn3IPMcb9WT7nRsIMhW78c/MV0+MZQEe6ue/+xNNF73ba/wFjizDrOjJxRgLHQfswFM8nR+3qIiSDjtD/DFNSmJ5NBK6oKFfJiOZtwW4DTqPWVmA23tuBfgYwDvQu+5RBtTpzilB3QzhYBAVVoGWibSNUep5b5KRgQoX3g8CjCvFMKKYj8G37rnB2bq4nMDQBFgUT7ALpzPhofvggq0NFRDj5lzGVqlZ+9c74LgzQ9CdpoYWdHLkYoae7ImBwOZFCEZaGzhAN0jmaMdhrIilPQPtLGCdBrf14Bl9dP7ZCS6LDoMy4Z79/5ztfLdKovw2Qnihivw/8EcLEcmZspRYiTegqOeDnDnnQuz+jXebeu69v0v2FMDvWEd1lpeRhYt0Dontv7lUheu3swZ20RaDv/DqQWVnShwukFZZ6uz46uniQXdWmFyTubzXZwJPBax9KAJmTpjuQ9rdG/UJAJBIM/2BI590ow4Oq6FYSpMAvosOqn7639CcVrnUaEggrVTk4QOsjKjfrr72xK1jBHU96D2gRFVlEK45sOvBXYKrngAb4Dn3CPo4I3M6jj9S0pn03/WH1QBGUcdKC8tUoo6rF1Re3vTdrR0eKm76zO2plqVXNhPk04Iw31Dp+z2isc4NPCBkKMYHeBUVZTVPjR09vPjgT1WoaekApaUmyKhb5UM+ux4N/VA6iTYl2Io0c9pHetHyAJPTMwpNLeZBGWKlf3dG/FQZNavg1MSUwIwYJKoZIhvcNAQkVMRYEFDoZhwbdbadqLSXgIA40dFIvXMy2MDEwITAJBgUrDgMCGgUABBSIZgeoci8+QDb/Vu0pHvI0jyOj4gQI5UdROF/zJS8CAggA"

    private static let testCertP12Base64B =
        "MIIFyQIBAzCCBY8GCSqGSIb3DQEHAaCCBYAEggV8MIIFeDCCAncGCSqGSIb3DQEHBqCCAmgwggJkAgEAMIICXQYJKoZIhvcNAQcBMBwGCiqGSIb3DQEMAQYwDgQImo8r39534IwCAggAgIICMLbMuWYIkPGABDcFj4QFxbzBR4j07G5RmW+2snCk+wSX/s+kXRWX1AZOKgZ4h6ICZL7dGCueZ2QopbB1L39q1I0GYNB5Yq6WacbepBJB04vEcpJx1r8WqVLBlTBRROK9J2rLOtrT1G84rPsIdZ3irFZesullNqWA8N9X/ZJoam6mnBa2SNdh7bVbbdt/EnEx5Y6n6/I3DYScfMy4eA0ApEf06zg2ObuHsOHNbYg1KYJrt8DkZVkfP+u1bndz+ERhPNeU5P8yEoFRkcMcofG+bgv5/UQoy68GQat7H+fwgZbnEmo9swjOrc2AYTUjM+c2k4os2uesKfpZbFAyL19f0VcqfZos6vWob8/4ICccLXv52jOk4uTR2jO8tD63rtpc2LpfUgF6oef2T8bn40e52t5ZijOgi+xGNkr/c1B8rDGMWRwQE/YOOFYG63Kuy9YBq3H45mIGmpdJFIYS9JmEQpXWNVzss7oyBfdCyVPyKq7EbWP9ya92fQZDbNf2mQlsRIL/9sZfBSUFUke39vyhYLL3yOyH+c3qdSbN1lFgtGEdzoxeLnqw4+K0yNAvEeqYi7wFWUcxH8d3ZOS6nR3EQ0K2xEEXpydZmwuIHMIIdF8Q9UhNbhQmtyGqHIOwJXFqQlQzjGI3jHiLLTuxCQRAaHXcFRdRO9Mdikjr2y/QfrsvokvEQjarBFvRwJgWCOlcX2CxFrMEO1YUo7mb3dKh3XG0MMj1OoCTDXWIsClZEA+QMIIC+QYJKoZIhvcNAQcBoIIC6gSCAuYwggLiMIIC3gYLKoZIhvcNAQwKAQKgggKmMIICojAcBgoqhkiG9w0BDAEDMA4ECPbRJVMOs7QiAgIIAASCAoArJ43gpQVEBiKPOXIuXRIbAULQanYPTRS5Gnmm4GDOcSox2jDuv4E1xyGWim54LEzZoB7hSHOroXgDxlO2nnQu+4uuMZe6TMzMtGllPVlbkfUpVTzYO/Wop8ZZq9eNIm01YuFy9SWazYkf0JxegjuA/XXZl6NBUokRZC/I0E3wyzMQfYEdqfhnhBH+z9M3BpRfGf76ULcucFjBQouFOQn6g2yBv3xsJ64aSXETzYA362xYEGwqmWuEBPVR0BT39vJms3cliAv+Gt+W9WVIEnFir4g/O9L8vyb+ZJ7VBVbPyiYHUhr6tcZV1wjO/QstoZr3myAI7SQ6A7eSTNp/e/rCz1v1WaZFcqkgWo7uyeRA4emGQdvnkghfDeI9RebOYqR3TWegsaJRIp2lab+MtPS7z+l3rtB05YxYipo+fTBTX9dW09kgRb7HGK8b/O6QydiBWAxhzusNUS4KM6rkZtd4P5VNOrtX3rFgw6XvQNfa1w3WUtdkPHgXE1wv5/MQ82QWboSIQM6A3EYDRn8vnQUxdyYYqSYCubC5RKiFyF7f2wvDEb6QWtnrcl3TA9LFmthP/Rw0Da2p0pFH3QbUqJFIyCe/dEaGhWlSRl3crUMEC3p390+2jp3ipNzcCliXfiC+EEClrdkY9W0u0+NBuRaGT2gTNf+20cwiP0PigU2oKtaN7Rpvd1vOd6QUyPEPbR89BkGglve9lRS7XFqn3RqSlNEVg6bRlH31ZnBFAaHcxg5r1OUOaTzmVkkzIox+KvFwSM+YxM+8EgKz65XEHiBmLNHZPsh9CYz/zdsZjTVCRBdFchsnjMpm/AZEl9OmJxRrUqChkCz4q2ZmS7oOqB+qMSUwIwYJKoZIhvcNAQkVMRYEFC+CfGcnUSKE/a1fx5i96bzSP48jMDEwITAJBgUrDgMCGgUABBRPxLdParpuWEquhRWWWFXHjYJVGQQIT/mzWW41YxoCAggA"
}

private struct MockHostIPResolver: HostIPResolving {
    var mapping: [String: [String]] = [:]

    func resolveHostIPs(_ host: String) -> [String] {
        mapping[host] ?? []
    }
}

private final class MockSecureTransport: SecureHTTPTransporting, @unchecked Sendable {
    var executedURL: URL?
    var mockResult: (Data, HTTPURLResponse)?

    func executeGet(url: URL) async throws -> (Data, HTTPURLResponse) {
        executedURL = url
        if let mockResult {
            return mockResult
        }
        throw LNURLError.invalidResponse
    }
}
