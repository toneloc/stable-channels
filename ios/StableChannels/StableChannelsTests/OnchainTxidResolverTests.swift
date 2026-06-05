import XCTest
@testable import StableChannels

final class OnchainTxidResolverTests: XCTestCase {
    private let validTxid = String(repeating: "a", count: 64)
    private let testAddress = "bc1qtest0000000000000000000000000000000000"
    private var dataDir: URL!
    private var service: DatabaseService!
    private var session: URLSession!
    private var resolutionId: Int64!
    private var box: ResolvedOnchainBox!

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("OnchainTxidResolverTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)
        resolutionId = service?.insertOnchainReceiveResolution(address: testAddress)
        XCTAssertNotNil(resolutionId, "Failed to insert seed row for test")

        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [MockURLProtocol.self]
        session = URLSession(configuration: config)

        MockURLProtocol.requestHandler = nil
        MockURLProtocol.callCount = 0
        MockURLProtocol.seenURLs = []

        box = ResolvedOnchainBox()
    }

    override func tearDown() {
        session = nil
        service = nil
        box = nil
        MockURLProtocol.requestHandler = nil
        try? FileManager.default.removeItem(at: dataDir)
        super.tearDown()
    }

    private func jsonArrayResponse(body: [[String: Any]], status: Int = 200) -> (HTTPURLResponse, Data) {
        let data = (try? JSONSerialization.data(withJSONObject: body)) ?? Data()
        let resp = HTTPURLResponse(
            url: URL(string: "https://mock.local")!,
            statusCode: status,
            httpVersion: "HTTP/1.1",
            headerFields: nil
        )!
        return (resp, data)
    }

    private func emptyArrayResponse() -> (HTTPURLResponse, Data) {
        jsonArrayResponse(body: [])
    }

    private func makeResolver(
        chainURLs: [String] = ["https://primary.local/api", "https://fallback.local/api"],
        maxAttempts: Int = 2,
        backoffSeconds: [UInt64] = [0]
    ) -> OnchainTxidResolver {
        // setUp guarantees box != nil; force-unwrap is safe.
        let captureBox = self.box!
        return OnchainTxidResolver(
            chainURLs: chainURLs,
            onResolved: { id, txid in
                await captureBox.set(id: id, txid: txid)
            },
            urlSession: session,
            maxAttempts: maxAttempts,
            backoffSeconds: backoffSeconds,
            esploraTimeout: 5
        )
    }

    // MARK: - isValidTxid delegates

    func testIsValidTxid_validatesThroughClient() {
        XCTAssertTrue(ResilientEsploraClient.isValidTxid(validTxid))
        XCTAssertFalse(ResilientEsploraClient.isValidTxid("short"))
    }

    // MARK: - resolve()

    func testResolve_findsTxidOnChainEndpoint() async {
        MockURLProtocol.requestHandler = { req in
            if let path = req.url?.path, path.hasSuffix("/txs/chain") {
                return self.jsonArrayResponse(body: [["txid": self.validTxid]])
            }
            return self.emptyArrayResponse()
        }

        let resolver = makeResolver(maxAttempts: 2, backoffSeconds: [0])
        await resolver.resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        try? await Task.sleep(nanoseconds: 100_000_000) // yield to MainActor

        let captured = await box.value
        XCTAssertEqual(captured?.id, resolutionId)
        XCTAssertEqual(captured?.txid, validTxid)

        // DB should be updated to resolved; row no longer pending
        let pending = service.fetchPendingOnchainReceives().first { $0.id == resolutionId }
        XCTAssertNil(pending, "Row should no longer be pending after resolve")
    }

    func testResolve_findsTxidOnMempoolEndpoint() async {
        MockURLProtocol.requestHandler = { req in
            if let path = req.url?.path, path.hasSuffix("/txs/chain") {
                return self.emptyArrayResponse()
            }
            if let path = req.url?.path, path.hasSuffix("/txs/mempool") {
                return self.jsonArrayResponse(body: [["txid": self.validTxid]])
            }
            return self.emptyArrayResponse()
        }

        let resolver = makeResolver(maxAttempts: 2, backoffSeconds: [0])
        await resolver.resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        try? await Task.sleep(nanoseconds: 100_000_000)

        let captured = await box.value
        XCTAssertEqual(captured?.txid, validTxid)
    }

    func testResolve_fallsBackToSecondaryChain() async {
        MockURLProtocol.requestHandler = { req in
            guard let host = req.url?.host else {
                return self.emptyArrayResponse()
            }
            if host == "primary.local" {
                return self.emptyArrayResponse()
            }
            // fallback chain returns the hit on its chain endpoint
            if let path = req.url?.path, path.hasSuffix("/txs/chain") {
                return self.jsonArrayResponse(body: [["txid": self.validTxid]])
            }
            return self.emptyArrayResponse()
        }

        let resolver = makeResolver(maxAttempts: 2, backoffSeconds: [0])
        await resolver.resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        try? await Task.sleep(nanoseconds: 100_000_000)

        let captured = await box.value
        XCTAssertEqual(captured?.txid, validTxid)
        XCTAssertTrue(MockURLProtocol.seenURLs.contains { $0.host == "fallback.local" })
    }

    func testResolve_doesNotFireOnEmptyResponse() async {
        MockURLProtocol.requestHandler = { _ in self.emptyArrayResponse() }

        let resolver = makeResolver(maxAttempts: 2, backoffSeconds: [0])
        await resolver.resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        try? await Task.sleep(nanoseconds: 100_000_000)

        let captured = await box.value
        XCTAssertNil(captured, "onResolved must not fire when all responses are empty")

        // Row must remain pending
        let pending = service.fetchPendingOnchainReceives().first { $0.id == resolutionId }
        XCTAssertNotNil(pending, "Row should still be pending")
    }

    func testResolve_rejectsInvalidTxid() async {
        MockURLProtocol.requestHandler = { req in
            // Always return a non-64-hex "txid"
            if let path = req.url?.path, path.hasSuffix("/txs/chain") {
                return self.jsonArrayResponse(body: [["txid": "short"]])
            }
            return self.emptyArrayResponse()
        }

        let resolver = makeResolver(maxAttempts: 2, backoffSeconds: [0])
        await resolver.resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        try? await Task.sleep(nanoseconds: 100_000_000)

        let captured = await box.value
        XCTAssertNil(captured, "Invalid txid must not fire onResolved")

        // Row must remain pending
        let pending = service.fetchPendingOnchainReceives().first { $0.id == resolutionId }
        XCTAssertNotNil(pending)
    }

    func testResolve_cancelsBeforeAnyFire_strict() async throws {
        // Force a 503 on every response so onResolved can never fire
        // regardless of cancellation timing.
        MockURLProtocol.requestHandler = { _ in
            self.jsonArrayResponse(body: [], status: 503)
        }

        let captureBox = try XCTUnwrap(self.box)
        let resolver = OnchainTxidResolver(
            chainURLs: ["https://primary.local/api", "https://fallback.local/api"],
            onResolved: { id, txid in
                await captureBox.set(id: id, txid: txid)
            },
            urlSession: session,
            maxAttempts: 5,
            backoffSeconds: [1, 1, 1, 1],
            esploraTimeout: 5
        )

        let task = Task {
            await resolver.resolve(
                resolutionId: self.resolutionId,
                address: self.testAddress,
                databaseService: self.service
            )
        }
        task.cancel()
        await task.value
        try? await Task.sleep(nanoseconds: 100_000_000)

        let captured = await box.value
        XCTAssertNil(captured, "Cancelled task with all-failing responses must never fire onResolved")
    }
}

/// Captures (id, txid) hits from the resolver's `@MainActor` callback.
actor ResolvedOnchainBox {
    struct Captured: Equatable {
        let id: Int64
        let txid: String
    }

    var value: Captured?

    func set(id: Int64, txid: String) {
        value = Captured(id: id, txid: txid)
    }
}
