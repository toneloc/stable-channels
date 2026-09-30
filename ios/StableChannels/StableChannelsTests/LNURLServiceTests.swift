import CryptoKit
import LDKNode
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

    func testSuccessActionDomainMatching() throws {
        let callbackURL = try XCTUnwrap(URL(string: "https://pay.shop.com/callback"))

        let safeAction = LNURLSuccessAction(
            tag: "url",
            description: "Receipt",
            url: "https://pay.shop.com/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertTrue(safeAction.isSameHostOrSubdomain(callbackURL: callbackURL))

        let subdomainAction = LNURLSuccessAction(
            tag: "url",
            description: "Receipt",
            url: "https://sub.pay.shop.com/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertTrue(subdomainAction.isSameHostOrSubdomain(callbackURL: callbackURL))

        let phishingAction = LNURLSuccessAction(
            tag: "url",
            description: "Phishing",
            url: "https://malicious-redirect.com/login",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertFalse(phishingAction.isSameHostOrSubdomain(callbackURL: callbackURL))
    }

    func testSuccessActionSchemeValidation() {
        let httpsAction = LNURLSuccessAction(
            tag: "url",
            description: "Receipt",
            url: "https://pay.shop.com/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        if case .url(let desc, let url) = httpsAction.actionType {
            XCTAssertEqual(desc, "Receipt")
            XCTAssertEqual(url.absoluteString, "https://pay.shop.com/receipt/123")
        } else {
            XCTFail("Expected .url action")
        }

        let torAction = LNURLSuccessAction(
            tag: "url",
            description: "Tor Receipt",
            url: "http://shop.onion/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        if case .url(let desc, let url) = torAction.actionType {
            XCTAssertEqual(desc, "Tor Receipt")
            XCTAssertEqual(url.absoluteString, "http://shop.onion/receipt/123")
        } else {
            XCTFail("Expected .url action for Tor")
        }

        let insecureClearnetAction = LNURLSuccessAction(
            tag: "url",
            description: "Insecure",
            url: "http://insecure-shop.com/receipt",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertEqual(insecureClearnetAction.actionType, .unknown(tag: "url"))

        let javascriptAction = LNURLSuccessAction(
            tag: "url",
            description: "Exploit",
            url: "javascript:alert(1)",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertEqual(javascriptAction.actionType, .unknown(tag: "url"))
    }

    func testMockServiceSubstitution() async throws {
        let mock = MockLNURLService()
        await mock.setStubbedParams(LNURLPayParams(
            tag: "payRequest",
            callback: "https://test.com/cb",
            minSendable: 1000,
            maxSendable: 10000,
            metadata: "[[\"text/plain\",\"Test\"]]",
            commentAllowed: nil
        ))

        let url = try XCTUnwrap(URL(string: "https://test.com/lnurlp"))
        let params = try await mock.fetchPayParams(from: url)
        XCTAssertEqual(params.callback, "https://test.com/cb")
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

    func testFetchInvoice_withParams_validatesBoundsAndComment_andSucceeds() async throws {
        let (service, callbackURL) = makeMockService()
        let metadataJSON = ""
        let params = LNURLPayParams(
            tag: "payRequest",
            callback: callbackURL.absoluteString,
            minSendable: 10_000,
            maxSendable: 100_000,
            metadata: metadataJSON,
            commentAllowed: 20
        )

        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

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

        // Comment too long throws invalidResponse
        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: String(repeating: "c", count: 25)
            )
            XCTFail("Expected invalidResponse for comment exceeding limit")
        } catch LNURLError.invalidResponse {
            // Success
        }

        // Valid params and amount within bounds succeeds
        let result = try await service.fetchInvoice(params: params, amountMsat: 50_000, comment: "Hello")
        XCTAssertEqual(result.pr, Self.valid50kInvoice)
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

        let params = makeParams(callback: callbackURL.absoluteString, metadata: "")
        _ = try await service.fetchInvoice(
            params: params,
            amountMsat: 50_000,
            comment: "Thanks & hello? test=1"
        )

        let resolvedURL = try XCTUnwrap(capturedURL)
        let components = try XCTUnwrap(URLComponents(url: resolvedURL, resolvingAgainstBaseURL: false))
        let queryItems = try XCTUnwrap(components.queryItems)

        XCTAssertEqual(queryItems.first(where: { $0.name == "amount" })?.value, "50000")
        XCTAssertEqual(queryItems.first(where: { $0.name == "comment" })?.value, "Thanks & hello? test=1")
        XCTAssertTrue(resolvedURL.query?.contains("%26") ?? false)
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

    func testFetchPayParams_torHttpAccepted_andClearnetHttpRejected() async throws {
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
            return (response, Data(validJson.utf8))
        }

        // Tor hidden service over HTTP succeeds
        let params = try await service.fetchPayParams(from: torURL)
        XCTAssertEqual(params.callback, "http://service.onion/callback")

        // Clearnet endpoint over HTTP is rejected
        do {
            _ = try await service.fetchPayParams(from: clearnetHttpURL)
            XCTFail("Expected LNURLError.insecureEndpoint for clearnet HTTP")
        } catch LNURLError.insecureEndpoint {
            // Expected
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_torHttpAccepted_andClearnetHttpRejected() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let service = LNURLService(urlSession: session)

        let torCallback = "http://service.onion/callback"
        let clearnetHttpCallback = "http://service.com/callback"

        MockURLProtocol.requestHandler = { req in
            let response = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        // Tor callback over HTTP succeeds
        let torParams = makeParams(callback: torCallback, metadata: "")
        let result = try await service.fetchInvoice(
            params: torParams,
            amountMsat: 50_000,
            comment: nil
        )
        XCTAssertEqual(result.pr, Self.valid50kInvoice)

        // Clearnet callback over HTTP is rejected
        let clearnetParams = makeParams(callback: clearnetHttpCallback, metadata: "")
        do {
            _ = try await service.fetchInvoice(
                params: clearnetParams,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected LNURLError.insecureEndpoint for clearnet HTTP callback")
        } catch LNURLError.insecureEndpoint {
            // Expected
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
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

    func testFetchInvoice_redirectsToLoopback_isVetoedPreHop() async throws {
        let (service, callbackURL) = makeMockService()
        let loopbackRedirectURL = try XCTUnwrap(URL(string: "https://127.0.0.1/callback"))

        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(
                url: callbackURL,
                statusCode: 302,
                httpVersion: nil,
                headerFields: ["Location": loopbackRedirectURL.absoluteString]
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
            XCTFail("Expected LNURLError.insecureEndpoint on loopback redirect")
        } catch LNURLError.insecureEndpoint {
            // Expected
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(loopbackRedirectURL))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testBech32_verifySegwitAddress_trimsSurroundingWhitespace() {
        let validAddress = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        let paddedAddress = "  \n\t bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4 \r\n  "

        let direct = Bech32.verifySegwitAddress(validAddress, expectedHrp: "bc")
        let padded = Bech32.verifySegwitAddress(paddedAddress, expectedHrp: "bc")
        XCTAssertTrue(direct)
        XCTAssertTrue(padded)
    }

    func testBech32_decode_multibyteHRPCharacter_throwsInvalidCharacterWithoutGarbling() {
        XCTAssertThrowsError(try Bech32.decode("🔥1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4")) { error in
            guard case let Bech32.Error.invalidCharacter(c) = error else {
                XCTFail("Expected invalidCharacter, got \(error)")
                return
            }
            XCTAssertEqual(c, "🔥")
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

    func testFetchInvoice_dnsResolvesToPrivateIP_isRejectedAsInsecureEndpoint() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let mockResolver = MockHostIPResolver(mapping: [
            "rebinding.example.com": ["127.0.0.1"]
        ])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://rebinding.example.com/callback"))
        let params = makeParams(callback: targetURL.absoluteString, metadata: "")

        MockURLProtocol.seenURLs = []
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await service.fetchInvoice(
                params: params,
                amountMsat: 50_000,
                comment: nil
            )
            XCTFail("Expected LNURLError.insecureEndpoint for DNS resolving to private IP")
        } catch LNURLError.insecureEndpoint {
            // Expected - request must be vetoed before opening socket / making network call
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(targetURL))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchPayParams_dnsResolvesToPrivateIP_isRejectedAsInsecureEndpoint() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let mockResolver = MockHostIPResolver(mapping: [
            "rebinding.example.com": ["10.0.0.5"]
        ])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

        let targetURL = try XCTUnwrap(URL(string: "https://rebinding.example.com/.well-known/lnurlp/alice"))

        MockURLProtocol.seenURLs = []
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: targetURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (response, Data())
        }

        do {
            _ = try await service.fetchPayParams(from: targetURL)
            XCTFail("Expected LNURLError.insecureEndpoint for DNS resolving to private IP")
        } catch LNURLError.insecureEndpoint {
            // Expected - request must be vetoed before opening socket / making network call
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(targetURL))
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testFetchInvoice_redirectsToDnsResolvingToPrivateIP_isVetoedPreHop() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)

        let callbackURL = try XCTUnwrap(URL(string: "https://service.example.com/callback"))
        let redirectTargetURL = try XCTUnwrap(URL(string: "https://rebinding-redirect.example.com/internal-service"))

        let mockResolver = MockHostIPResolver(mapping: [
            "service.example.com": ["93.184.216.34"],
            "rebinding-redirect.example.com": ["192.168.1.50"]
        ])
        let service = LNURLService(urlSession: session, hostResolver: mockResolver)

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
            XCTFail("Expected LNURLError.insecureEndpoint on redirect to host resolving to private IP")
        } catch LNURLError.insecureEndpoint {
            // Expected - redirect is vetoed pre-hop
            XCTAssertFalse(MockURLProtocol.seenURLs.contains(redirectTargetURL))
        } catch {
            XCTFail("Unexpected error: \(error)")
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

    func testFetchInvoice_dnsRebindsToPrivateIP_isRejected() async throws {
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
            XCTFail("Expected LNURLError.insecureEndpoint when DNS rebinds to private IP on second check")
        } catch LNURLError.insecureEndpoint {
            // Rebinding was caught and rejected
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

        // Public IP literal is accepted
        let publicIPURL = try XCTUnwrap(URL(string: "https://8.8.8.8/api"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: publicIPURL, hostResolver: resolver))

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
            "::ffff:0:10.0.0.1" // SIIT IPv4-translated (RFC 2765)
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
        XCTAssertEqual(SecureEndpointValidator.evaluateNumericIP("0177.0.0.1"), false)

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
        XCTAssertFalse(SecureEndpointValidator.isPrivateOrLoopbackHost("0177.0.0.1"))

        let blockedURLs = [
            "https://0127.0.0.1/api",
            "https://00127.0.0.1/api",
            "https://010.0.0.1/api",
            "https://0172.016.0.1/api",
            "https://0100.064.0.1/api",
            "https://169.0254.169.254/api"
        ]
        for urlStr in blockedURLs {
            let url = try XCTUnwrap(URL(string: urlStr))
            XCTAssertFalse(
                SecureEndpointValidator.isSecureEndpoint(url: url),
                "Expected \(urlStr) to be blocked"
            )
        }

        let publicURL = try XCTUnwrap(URL(string: "https://0177.0.0.1/api"))
        XCTAssertTrue(SecureEndpointValidator.isSecureEndpoint(url: publicURL))

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
        let (body1, resp1) = try HTTPResponseParser.parse(data: rawContentLength, url: url)
        XCTAssertEqual(resp1.statusCode, 200)
        XCTAssertEqual(String(data: body1, encoding: .utf8), "{\"status\":\"OK\"}")

        // Chunked Transfer-Encoding response
        let rawChunked = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: application/json\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
            .data(using: .utf8)!
        let (body2, resp2) = try HTTPResponseParser.parse(data: rawChunked, url: url)
        XCTAssertEqual(resp2.statusCode, 200)
        XCTAssertEqual(String(data: body2, encoding: .utf8), "hello world")

        // 404 response
        let raw404 = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found".data(using: .utf8)!
        let (body3, resp3) = try HTTPResponseParser.parse(data: raw404, url: url)
        XCTAssertEqual(resp3.statusCode, 404)
        XCTAssertEqual(String(data: body3, encoding: .utf8), "Not Found")
    }

    func testHTTPResponseParser_maliciousChunkSizeOverflow_doesNotCrashAndIsRejected() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))

        // Chunk size that overflows Int addition if not bounds-checked (SIGTRAP 133 regression test)
        let maliciousOverflow = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n7FFFFFFFFFFFFFFF\r\nAB\r\n0\r\n\r\n"
            .data(using: .utf8)!
        XCTAssertThrowsError(try HTTPResponseParser.parse(data: maliciousOverflow, url: url)) { error in
            guard let lnurlError = error as? LNURLError, case .invalidResponse = lnurlError else {
                XCTFail("Expected LNURLError.invalidResponse, got \(error)")
                return
            }
        }

        // Chunk size exceeding maxResponseBytes (2MB limit)
        let maliciousLargeChunk = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n300000\r\nABC\r\n0\r\n\r\n"
            .data(using: .utf8)!
        XCTAssertThrowsError(try HTTPResponseParser.parse(data: maliciousLargeChunk, url: url)) { error in
            guard let lnurlError = error as? LNURLError, case .invalidResponse = lnurlError else {
                XCTFail("Expected LNURLError.invalidResponse, got \(error)")
                return
            }
        }
    }

    func testHTTPResponseParser_negativeChunkSize_doesNotCrashAndIsRejected() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))
        let rawNegativeChunk = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n-1\r\nX\r\n0\r\n\r\n"
            .data(using: .utf8)!
        XCTAssertThrowsError(try HTTPResponseParser.parse(data: rawNegativeChunk, url: url)) { error in
            guard let lnurlError = error as? LNURLError, case .invalidResponse = lnurlError else {
                XCTFail("Expected LNURLError.invalidResponse, got \(error)")
                return
            }
        }
    }

    func testHTTPResponseParser_truncatedChunkedBody_isRejected() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))
        let rawTruncated = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n".data(using: .utf8)!
        XCTAssertThrowsError(try HTTPResponseParser.parse(data: rawTruncated, url: url)) { error in
            guard let lnurlError = error as? LNURLError, case .invalidResponse = lnurlError else {
                XCTFail("Expected LNURLError.invalidResponse, got \(error)")
                return
            }
        }
    }

    func testHTTPResponseParser_contentLengthTruncation_isRejected() throws {
        let url = try XCTUnwrap(URL(string: "https://example.com/api"))
        let rawTruncated = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort body".data(using: .utf8)!
        XCTAssertThrowsError(try HTTPResponseParser.parse(data: rawTruncated, url: url)) { error in
            guard let lnurlError = error as? LNURLError, case .invalidResponse = lnurlError else {
                XCTFail("Expected LNURLError.invalidResponse, got \(error)")
                return
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
        let rebindURL = try XCTUnwrap(URL(string: "https://rebind.example.com/test"))

        do {
            _ = try await transport.executeGet(url: rebindURL)
            XCTFail("Expected insecureEndpoint error when DNS rebinds to loopback")
        } catch LNURLError.insecureEndpoint {
            // Success: socket pinning check caught the rebinding and rejected before connecting!
        }
        XCTAssertEqual(resolver.callCount, 2)
    }

    func testNWConnectionTransport_crlfInURLPath_isRejected() throws {
        let resolver = MockHostIPResolver(mapping: ["attacker.example": ["93.184.216.34"]])
        let transport = NWConnectionTransport(hostResolver: resolver)
        let injectedURL = try XCTUnwrap(URL(string: "https://attacker.example/a%0D%0AX-Injected:%201"))

        // Path contains encoded CRLF which percent-decoding would make dangerous, but NWConnectionTransport preserves
        // encoding
        let path = injectedURL.path(percentEncoded: true)
        XCTAssertFalse(path.contains("\r"))
        XCTAssertFalse(path.contains("\n"))
        XCTAssertTrue(path.contains("%0D%0A"))
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
