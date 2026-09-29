import CryptoKit
import LDKNode
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
        callback _: String,
        amountMsat _: UInt64,
        comment _: String?,
        expectedMetadataHashHex _: String
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
        let standard = try LNURLService.resolveEndpoint(from: "satoshi@bitcoin.org")
        XCTAssertEqual(standard.absoluteString, "https://bitcoin.org/.well-known/lnurlp/satoshi")

        let personal = try LNURLService.resolveEndpoint(from: "prabal@0xprabal.com")
        XCTAssertEqual(personal.absoluteString, "https://0xprabal.com/.well-known/lnurlp/prabal")

        let bech32Sample = "lnurl1dp68gurn8ghj7um9wfmxjcm99e3k7mf0v9cxjtmkxyhkcmn4wfkz7urp0yvwqajv"
        let resolvedBech32 = try LNURLService.resolveEndpoint(from: bech32Sample)
        XCTAssertEqual(resolvedBech32.absoluteString, "https://service.com/api/v1/lnurl/pay")

        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "invalid-target"))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "user@@domain.com"))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "user@nodomain"))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "http://insecure-clearnet.com"))
        XCTAssertThrowsError(try LNURLService.resolveEndpoint(from: "anon@xyz.onion"))
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
        let service = LNURLService(urlSession: session)

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
            XCTAssertEqual(reason, "User not found")
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }

    func testMockedLNURLPayResolution() async throws {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let service = LNURLService(urlSession: session)

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

    private func makeMockService() -> (LNURLService, URL) {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        let session = URLSession(configuration: config)
        let service = LNURLService(urlSession: session)
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

        let result = try await service.fetchInvoice(
            callback: callbackURL.absoluteString,
            amountMsat: 50_000,
            comment: "Tip",
            expectedMetadataHashHex: Self.metadataHashA
        )

        XCTAssertEqual(result.pr, Self.valid50kInvoice)
        XCTAssertFalse(result.isError)
    }

    func testFetchInvoice_mismatchedAmount_throwsInvoiceAmountMismatch() async throws {
        let (service, callbackURL) = makeMockService()
        MockURLProtocol.requestHandler = { _ in
            let response = HTTPURLResponse(url: callbackURL, statusCode: 200, httpVersion: nil, headerFields: nil)!
            let json = "{\"pr\":\"\(Self.mismatchedAmountInvoice100k)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        do {
            _ = try await service.fetchInvoice(
                callback: callbackURL.absoluteString,
                amountMsat: 50_000,
                comment: nil,
                expectedMetadataHashHex: Self.metadataHashA
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

        let differentHash = "0000000000000000000000000000000000000000000000000000000000000000"
        do {
            _ = try await service.fetchInvoice(
                callback: callbackURL.absoluteString,
                amountMsat: 50_000,
                comment: nil,
                expectedMetadataHashHex: differentHash
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

        do {
            _ = try await service.fetchInvoice(
                callback: callbackURL.absoluteString,
                amountMsat: 50_000,
                comment: nil,
                expectedMetadataHashHex: Self.metadataHashA
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

        do {
            _ = try await service.fetchInvoice(
                callback: callbackURL.absoluteString,
                amountMsat: 50_000,
                comment: nil,
                expectedMetadataHashHex: Self.metadataHashA
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

        do {
            _ = try await service.fetchInvoice(
                callback: callbackURL.absoluteString,
                amountMsat: 50_000,
                comment: nil,
                expectedMetadataHashHex: Self.metadataHashA
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

        _ = try await service.fetchInvoice(
            callback: callbackURL.absoluteString,
            amountMsat: 50_000,
            comment: "Thanks & hello? test=1",
            expectedMetadataHashHex: Self.metadataHashA
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
        let service = LNURLService(urlSession: session)

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
                url: insecureRedirectURL,
                statusCode: 200,
                httpVersion: nil,
                headerFields: nil
            )!
            let json = "{\"pr\":\"\(Self.valid50kInvoice)\",\"status\":\"OK\"}"
            return (response, Data(json.utf8))
        }

        do {
            _ = try await service.fetchInvoice(
                callback: callbackURL.absoluteString,
                amountMsat: 50_000,
                comment: nil,
                expectedMetadataHashHex: Self.metadataHashA
            )
            XCTFail("Expected LNURLError.insecureEndpoint on HTTP downgrade")
        } catch LNURLError.insecureEndpoint {
            // Expected
        } catch {
            XCTFail("Unexpected error: \(error)")
        }
    }
}
