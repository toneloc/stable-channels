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
        resolutionId = service?.onchainRepo.insertOnchainReceiveResolution(address: testAddress)
        XCTAssertNotNil(resolutionId, "Failed to insert seed row for test")
        XCTAssertTrue(service.onchainRepo.recordOnchainPaymentWithResolution(
            paymentId: "pending-deposit", amountMsat: 100_000, amountUSD: nil,
            btcPrice: nil, resolutionId: resolutionId
        ))

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
        let transactions = body.map { transaction in
            var transaction = transaction
            if transaction["vout"] == nil {
                transaction["vout"] = [["scriptpubkey_address": testAddress, "value": 100]]
            }
            return transaction
        }
        let data = (try? JSONSerialization.data(withJSONObject: transactions)) ?? Data()
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
                let numId = Int64(id.split(separator: "-").last ?? "0") ?? 0
                await captureBox.set(id: numId, txid: txid)
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

    @MainActor
    func testRestoredAddressSkipsOldEqualDepositAndResolvesNewMempoolDeposit() async throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "deposit-test-\(UUID().uuidString)"))
        let store = TxidLinkStore(defaults: defaults)
        store.setReceiveAddress(testAddress)
        let restored = TxidLinkStore(defaults: defaults)
        XCTAssertEqual(restored.onchainReceiveAddress, testAddress)
        defer { store.clearReceiveAddress() }
        let oldTxid = String(repeating: "b", count: 64)
        try service.paymentRepo.recordPayment(
            paymentId: "old-deposit", paymentType: "onchain", direction: "received",
            amountMsat: 100_000, amountUSD: nil, btcPrice: nil, counterparty: nil,
            status: "completed", txid: oldTxid
        )
        MockURLProtocol.requestHandler = { req in
            self.jsonArrayResponse(body: [["txid": req.url!.path.hasSuffix("/txs/chain") ? oldTxid : self.validTxid]])
        }
        await makeResolver().resolve(
            resolutionId: resolutionId, address: try XCTUnwrap(restored.onchainReceiveAddress), databaseService: service
        )
        let captured = await box.value
        XCTAssertEqual(captured?.txid, validTxid)
        XCTAssertNotNil(service.paymentRepo.payment(txid: oldTxid))
        XCTAssertNotNil(service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resolutionId))
        XCTAssertTrue(MockURLProtocol.seenURLs.contains { $0.path.hasSuffix("/txs/mempool") })
        restored.setReceiveAddress(nil)
        XCTAssertNil(TxidLinkStore(defaults: defaults).onchainReceiveAddress)
    }

    func testOnlyHistoricalTransactionLeavesDepositPending() async throws {
        try service.paymentRepo.recordPayment(
            paymentId: "old-deposit", paymentType: "onchain", direction: "received",
            amountMsat: 100_000, amountUSD: nil, btcPrice: nil, counterparty: nil,
            status: "completed", txid: validTxid
        )
        MockURLProtocol.requestHandler = { _ in self.jsonArrayResponse(body: [["txid": self.validTxid]]) }
        await makeResolver().resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        let captured = await box.value
        XCTAssertNil(captured)
        XCTAssertNotNil(service.onchainRepo.fetchPendingOnchainReceives().first { $0.id == resolutionId })
        XCTAssertNotNil(service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resolutionId))
        XCTAssertFalse(service.onchainRepo.updateOnchainReceiveResolution(id: resolutionId, txid: validTxid))
    }

    func testWrongIncomingAmountLeavesDepositPending() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonArrayResponse(body: [["txid": self.validTxid,
                                           "vout": [["scriptpubkey_address": self.testAddress, "value": 99]]]])
        }
        await makeResolver().resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        let captured = await box.value
        XCTAssertNil(captured)
        XCTAssertNotNil(service.onchainRepo.fetchPendingOnchainReceives().first { $0.id == resolutionId })
    }

    func testResolutionCannotReuseAnotherResolutionTransaction() throws {
        let other = try XCTUnwrap(service.onchainRepo.insertOnchainReceiveResolution(address: testAddress))
        XCTAssertTrue(service.onchainRepo.updateOnchainReceiveResolution(id: other, txid: validTxid))
        XCTAssertFalse(service.onchainRepo.updateOnchainReceiveResolution(id: resolutionId, txid: validTxid))
        XCTAssertTrue(try service.onchainRepo.recordedReceiveTxids().contains(validTxid))
        XCTAssertNotNil(service.onchainRepo.fetchPendingOnchainReceiveRow(resolutionId: resolutionId))
    }

    func testAddressHistorySpendDoesNotResolveIncomingDeposit() async {
        MockURLProtocol.requestHandler = { _ in
            self.jsonArrayResponse(body: [["txid": self.validTxid,
                                           "vout": [["scriptpubkey_address": "another-address", "value": 100]]]])
        }
        await makeResolver().resolve(resolutionId: resolutionId, address: testAddress, databaseService: service)
        let captured = await box.value
        XCTAssertNil(captured)
        XCTAssertNotNil(service.onchainRepo.fetchPendingOnchainReceives().first { $0.id == resolutionId })
    }

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
        let pending = service.onchainRepo.fetchPendingOnchainReceives().first { $0.id == resolutionId }
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
        let pending = service.onchainRepo.fetchPendingOnchainReceives().first { $0.id == resolutionId }
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
        let pending = service.onchainRepo.fetchPendingOnchainReceives().first { $0.id == resolutionId }
        XCTAssertNotNil(pending)
    }

    func testResolve_allFailingResponses_neverFiresOnResolved() async throws {
        // Force a 503 on every response so onResolved can never fire
        // regardless of cancellation timing.
        MockURLProtocol.requestHandler = { _ in
            self.jsonArrayResponse(body: [], status: 503)
        }

        let captureBox = try XCTUnwrap(self.box)
        let resolver = OnchainTxidResolver(
            chainURLs: ["https://primary.local/api", "https://fallback.local/api"],
            onResolved: { id, txid in
                let numId = Int64(id.split(separator: "-").last ?? "0") ?? 0
                await captureBox.set(id: numId, txid: txid)
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
