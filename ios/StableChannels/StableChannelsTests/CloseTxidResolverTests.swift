import XCTest
@testable import StableChannels

final class MockURLProtocol: URLProtocol {
    typealias Handler = (URLRequest) -> (HTTPURLResponse, Data)

    static var requestHandler: Handler?
    static var callCount: Int = 0
    static var seenURLs: [URL] = []

    override class func canInit(with _: URLRequest) -> Bool {
        return true
    }

    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = Self.requestHandler else {
            client?.urlProtocol(self, didFailWithError: URLError(.badURL))
            return
        }
        Self.callCount += 1
        if let url = request.url {
            Self.seenURLs.append(url)
        }
        let (response, data) = handler(request)
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: data)
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}
}

final class CloseTxidResolverTests: XCTestCase {
    private var dataDir: URL!
    private var service: DatabaseService!
    private var session: URLSession!
    private let validTxid = String(repeating: "0", count: 64)

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("CloseTxidResolverTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)

        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        session = URLSession(configuration: config)

        MockURLProtocol.requestHandler = nil
        MockURLProtocol.callCount = 0
        MockURLProtocol.seenURLs = []
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dataDir)
        service = nil
        session = nil
        MockURLProtocol.requestHandler = nil
        super.tearDown()
    }

    private func seedPending(opId: String = "close-test") {
        _ = service.insertPendingOperation(
            opId: opId,
            opType: "channel_close",
            fundingOutpointTxid: String(repeating: "a", count: 64),
            fundingOutpointVout: 0
        )
    }

    private func jsonResponse(body: [String: Any], status: Int = 200) -> (HTTPURLResponse, Data) {
        let data = (try? JSONSerialization.data(withJSONObject: body)) ?? Data()
        let resp = HTTPURLResponse(
            url: URL(string: "https://mock.local")!,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: nil
        )!
        return (resp, data)
    }

    private func makeResolver(
        chainURLs: [String] = ["https://primary.local/api", "https://fallback.local/api"],
        maxAttempts: Int = 3,
        backoffSeconds: [UInt64] = [0, 0, 0],
        onResolved: @escaping @Sendable (String, String) async -> Void = { _, _ in }
    ) -> CloseTxidResolver {
        let config = CloseTxidResolver.Config(
            maxAttempts: maxAttempts,
            backoffSeconds: backoffSeconds,
            esploraTimeout: 5,
            chainURLs: chainURLs
        )
        return CloseTxidResolver(
            chainURLs: chainURLs,
            onResolved: onResolved,
            urlSession: session,
            config: config
        )
    }

    // MARK: - isValidTxid

    func testIsValidTxidAccepts64LowercaseHex() {
        XCTAssertTrue(CloseTxidResolver.isValidTxid(validTxid))
    }

    func testIsValidTxidRejectsWrongLength() {
        XCTAssertFalse(CloseTxidResolver.isValidTxid("abc"))
    }

    func testIsValidTxidRejectsUppercase() {
        let upper = "ABCDEF" + String(repeating: "0", count: 58)
        XCTAssertFalse(CloseTxidResolver.isValidTxid(upper))
    }

    // MARK: - resolve()

    func testResolveFindsTxidOnSecondAttempt() async {
        seedPending()

        var attempts = 0
        MockURLProtocol.requestHandler = { _ in
            attempts += 1
            if attempts == 1 {
                return self.jsonResponse(body: ["spent": false])
            }
            return self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let resolver = makeResolver()
        await resolver.resolve(opId: "close-test", databaseService: service)

        // PK lookup: fetchPendingOperations() filters by status='pending',
        // so a resolved row is excluded.
        let op = service.fetchPendingOperation(opId: "close-test")
        XCTAssertNotNil(op)
        guard let op else { return }
        XCTAssertEqual(op.status, "resolved")
        XCTAssertEqual(op.closingTxid, validTxid)
    }

    func testResolveExhaustsAttempts() async {
        seedPending()

        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": false])
        }

        let resolver = makeResolver(maxAttempts: 3, backoffSeconds: [0, 0])
        await resolver.resolve(opId: "close-test", databaseService: service)

        let ops = service.fetchPendingOperations()
        XCTAssertEqual(ops.count, 1)
        XCTAssertEqual(ops[0].status, "pending",
                       "Row must remain pending when resolver exhausts attempts")
        XCTAssertNil(ops[0].closingTxid)
    }

    func testResolveRetriesOnNetworkError() async {
        seedPending()

        var attempts = 0
        MockURLProtocol.requestHandler = { _ in
            attempts += 1
            if attempts <= 2 {
                // Throw a network error on first two attempts.
                // Note: our handler can't actually `throw` (URLProtocol must
                // return a response or call didFailWithError), so we simulate
                // a 5xx; the resolver treats that as a non-200 and
                // continues to the next attempt.
                return self.jsonResponse(body: [:], status: 503)
            }
            return self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let resolver = makeResolver(maxAttempts: 5, backoffSeconds: [0, 0, 0, 0])
        await resolver.resolve(opId: "close-test", databaseService: service)

        let op = service.fetchPendingOperation(opId: "close-test")
        XCTAssertNotNil(op)
        guard let op else { return }
        XCTAssertEqual(op.status, "resolved")
        XCTAssertEqual(op.closingTxid, validTxid)
    }

    func testResolveValidatesTxidShape() async {
        seedPending()

        MockURLProtocol.requestHandler = { _ in
            self.jsonResponse(body: ["spent": true, "txid": "not-hex"])
        }

        let resolver = makeResolver(maxAttempts: 2, backoffSeconds: [0])
        await resolver.resolve(opId: "close-test", databaseService: service)

        let ops = service.fetchPendingOperations()
        XCTAssertEqual(ops[0].status, "pending",
                       "Row must stay pending when Esplora returns garbage")
        XCTAssertNil(ops[0].closingTxid)
    }

    func testResolveUsesBothChainURLs() async {
        seedPending()

        var attempts = 0
        MockURLProtocol.requestHandler = { req in
            attempts += 1
            // First 3 calls go to the primary (5xx every time).
            if let host = req.url?.host, host == "primary.local" {
                return self.jsonResponse(body: [:], status: 500)
            }
            // Fallback succeeds.
            return self.jsonResponse(body: ["spent": true, "txid": self.validTxid])
        }

        let resolver = makeResolver(maxAttempts: 3, backoffSeconds: [0, 0])
        await resolver.resolve(opId: "close-test", databaseService: service)

        let op = service.fetchPendingOperation(opId: "close-test")
        XCTAssertNotNil(op)
        guard let op else { return }
        XCTAssertEqual(op.status, "resolved",
                       "Resolver should fall back to the second chain URL")
        XCTAssertEqual(op.closingTxid, validTxid)
    }
}
