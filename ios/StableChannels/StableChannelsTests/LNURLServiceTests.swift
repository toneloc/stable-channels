import CryptoKit
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
        expectedMetadataHashHex _: String?
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
        XCTAssertTrue(safeAction.isSameOriginOrSubdomain(callbackURL: callbackURL))

        let subdomainAction = LNURLSuccessAction(
            tag: "url",
            description: "Receipt",
            url: "https://sub.pay.shop.com/receipt/123",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertTrue(subdomainAction.isSameOriginOrSubdomain(callbackURL: callbackURL))

        let phishingAction = LNURLSuccessAction(
            tag: "url",
            description: "Phishing",
            url: "https://malicious-redirect.com/login",
            message: nil,
            ciphertext: nil,
            iv: nil
        )
        XCTAssertFalse(phishingAction.isSameOriginOrSubdomain(callbackURL: callbackURL))
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
}
