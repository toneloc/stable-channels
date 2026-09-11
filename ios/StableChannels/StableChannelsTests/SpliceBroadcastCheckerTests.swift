import XCTest
@testable import StableChannels

private final class SpliceBroadcastMockURLProtocol: URLProtocol {
    typealias Handler = (URLRequest) throws -> (HTTPURLResponse, Data)
    static var requestHandler: Handler?

    override class func canInit(with _: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }
        do {
            let (response, data) = try handler(request)
            client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

final class SpliceBroadcastCheckerTests: XCTestCase {
    private var session: URLSession!

    override func setUp() {
        super.setUp()
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SpliceBroadcastMockURLProtocol.self]
        session = URLSession(configuration: config)
        SpliceBroadcastMockURLProtocol.requestHandler = nil
    }

    override func tearDown() {
        SpliceBroadcastMockURLProtocol.requestHandler = nil
        session = nil
        super.tearDown()
    }

    func testReturnsExistsWhenEndpointReturns200() async {
        SpliceBroadcastMockURLProtocol.requestHandler = { request in
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: 200,
                httpVersion: nil,
                headerFields: nil
            )!
            let json = #"{"confirmed":false}"#.data(using: .utf8)!
            return (response, json)
        }

        let checker = SpliceBroadcastChecker(urlSession: session, retries: 1, retryDelaySeconds: 0)
        let status = await checker.checkStatus(txid: "abc123def456", endpointURLs: ["https://mempool.space/api"])
        XCTAssertEqual(status, .exists)
    }

    func testReturnsNotFoundWhenAllEndpoints404AcrossRetries() async {
        var callCount = 0
        SpliceBroadcastMockURLProtocol.requestHandler = { request in
            callCount += 1
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: 404,
                httpVersion: nil,
                headerFields: nil
            )!
            return (response, Data())
        }

        let checker = SpliceBroadcastChecker(
            urlSession: session,
            retries: 2,
            retryDelaySeconds: 0,
            sleeper: { _ in }
        )
        let status = await checker.checkStatus(
            txid: "deadbeef",
            endpointURLs: ["https://mempool.space/api", "https://blockstream.info/api"]
        )
        XCTAssertEqual(status, .notFound)
        XCTAssertEqual(callCount, 4) // 2 retries * 2 endpoints
    }

    func testReturnsInconclusiveOnServerErrorOrTimeout() async {
        SpliceBroadcastMockURLProtocol.requestHandler = { request in
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: 503,
                httpVersion: nil,
                headerFields: nil
            )!
            return (response, Data())
        }

        let checker = SpliceBroadcastChecker(urlSession: session, retries: 1, retryDelaySeconds: 0)
        let status = await checker.checkStatus(txid: "deadbeef", endpointURLs: ["https://mempool.space/api"])
        XCTAssertEqual(status, .inconclusive)
    }

    func testReturnsInconclusiveOnNetworkFailure() async {
        SpliceBroadcastMockURLProtocol.requestHandler = { _ in
            throw URLError(.notConnectedToInternet)
        }

        let checker = SpliceBroadcastChecker(urlSession: session, retries: 1, retryDelaySeconds: 0)
        let status = await checker.checkStatus(txid: "deadbeef", endpointURLs: ["https://mempool.space/api"])
        XCTAssertEqual(status, .inconclusive)
    }

    func testReturnsInconclusiveWhenNoValidURLsProvided() async {
        let checker = SpliceBroadcastChecker(urlSession: session, retries: 1, retryDelaySeconds: 0)
        let status = await checker.checkStatus(txid: "deadbeef", endpointURLs: ["   ", ""])
        XCTAssertEqual(status, .inconclusive)
    }

    func testReturnsExistsIfFirstEndpoint404sAndSecondReturns200() async {
        SpliceBroadcastMockURLProtocol.requestHandler = { request in
            let statusCode = request.url?.absoluteString.contains("blockstream") == true ? 200 : 404
            let response = HTTPURLResponse(
                url: request.url!,
                statusCode: statusCode,
                httpVersion: nil,
                headerFields: nil
            )!
            return (response, Data())
        }

        let checker = SpliceBroadcastChecker(urlSession: session, retries: 1, retryDelaySeconds: 0)
        let status = await checker.checkStatus(
            txid: "deadbeef",
            endpointURLs: ["https://mempool.space/api", "https://blockstream.info/api"]
        )
        XCTAssertEqual(status, .exists)
    }
}
