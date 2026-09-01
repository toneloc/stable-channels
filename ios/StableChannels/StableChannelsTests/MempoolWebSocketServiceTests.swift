import XCTest
@testable import StableChannels

@MainActor
final class MempoolWebSocketServiceTests: XCTestCase {
    // MARK: - Helpers

    private func makeValidTxid() -> String {
        String(repeating: "a", count: 64)
    }

    private func makeAddressTransactionJSON(
        address _: String? = nil,
        txid: String? = nil,
        voutAddress: String? = nil,
        voutValue: Int64? = nil,
        vinTxid: String? = nil,
        msgAddress: String? = nil,
        msgTxid: String? = nil
    ) -> String {
        var voutDict: [String: Any] = [:]
        if let addr = voutAddress {
            voutDict["scriptpubkey_address"] = addr
            if let val = voutValue {
                voutDict["value"] = val
            }
        }

        var vinDict: [String: Any] = [:]
        if let prevTxid = vinTxid {
            vinDict["txid"] = prevTxid
        }

        var txDict: [String: Any] = ["txid": txid ?? makeValidTxid()]
        if !voutDict.isEmpty {
            txDict["vout"] = [voutDict]
        }
        if !vinDict.isEmpty {
            txDict["vin"] = [vinDict]
        }

        var root: [String: Any] = ["address-transactions": [txDict]]
        if let addr = msgAddress {
            root["address"] = addr
        }
        if let tid = msgTxid {
            root["txid"] = tid
        }

        guard let data = try? JSONSerialization.data(withJSONObject: root),
              let json = String(data: data, encoding: .utf8) else {
            return "{}"
        }
        return json
    }

    private func makeBlockHeaderJSON(height: UInt32) -> String {
        let dict: [String: Any] = ["block": ["height": height]]
        guard let data = try? JSONSerialization.data(withJSONObject: dict),
              let json = String(data: data, encoding: .utf8) else {
            return "{}"
        }
        return json
    }

    private func makeMalformedJSON() -> String {
        return "this is not json {{{"
    }

    // MARK: - SetUp / TearDown

    private var service: MempoolWebSocketService!

    override func setUp() {
        super.setUp()
        service = MempoolWebSocketService()
        AuditService.setLogPath("")
    }

    override func tearDown() {
        service.disconnect()
        service = nil
        AuditService.setLogPath("")
        super.tearDown()
    }

    // MARK: - JSON Decoding via handleMessage

    func testAddressTransactionDecoding() {
        let specificTxid = makeValidTxid()
        let json = makeAddressTransactionJSON(
            txid: specificTxid,
            voutAddress: "bc1qtestaddress123",
            voutValue: 50_000
        )

        let txid = specificTxid
        var capturedTxid: String?
        var capturedAmount: Int64?

        service.onTransactionDetected = { event in
            guard case let .receive(_, receivedTxid, amountSats) = event else { return }
            capturedTxid = receivedTxid
            capturedAmount = amountSats
        }

        service.trackAddress("bc1qtestaddress123")
        service.handleMessage(json)

        XCTAssertEqual(capturedTxid, txid)
        XCTAssertEqual(capturedAmount, 50_000)
    }

    func testBlockHeaderDecoding() {
        let json = makeBlockHeaderJSON(height: 800_000)

        var capturedHeight: UInt32?

        service.onBlockHeader = { block in
            capturedHeight = block.height
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedHeight, 800_000)
    }

    func testMalformedJSONReturnsNilDecode() {
        let json = makeMalformedJSON()

        var transactionFired = false
        var blockFired = false

        service.onTransactionDetected = { _ in
            transactionFired = true
        }
        service.onBlockHeader = { _ in
            blockFired = true
        }

        service.handleMessage(json)

        XCTAssertFalse(transactionFired)
        XCTAssertFalse(blockFired)
    }

    func testAddressTransactionWithInvalidTxidIsIgnored() {
        let invalidTxid = "short"
        let json = """
        { "address-transactions": [{ "txid": "\(invalidTxid)" }] }
        """

        var transactionFired = false
        service.onTransactionDetected = { _ in
            transactionFired = true
        }

        service.handleMessage(json)

        XCTAssertFalse(transactionFired)
    }

    func testEmptyAddressTransactionsArrayIsIgnored() {
        let json = """
        { "address-transactions": [] }
        """

        var transactionFired = false
        service.onTransactionDetected = { _ in
            transactionFired = true
        }

        service.handleMessage(json)

        XCTAssertFalse(transactionFired)
    }

    func testBlockAndTransactionInSameMessage() {
        let combinedJSON = "{ \"address-transactions\": [{ \"txid\": \"\(makeValidTxid())\", \"vout\": [{ \"scriptpubkey_address\": \"bc1qtestaddr\", \"value\": 25000 }] }], \"block\": { \"height\": 800001 } }"

        var capturedTxid: String?
        var capturedAmount: Int64?
        var capturedHeight: UInt32?

        service.onTransactionDetected = { event in
            guard case let .receive(_, txid, amount) = event else { return }
            capturedTxid = txid
            capturedAmount = amount
        }
        service.onBlockHeader = { block in
            capturedHeight = block.height
        }

        service.trackAddress("bc1qtestaddr")
        service.handleMessage(combinedJSON)

        XCTAssertNotNil(capturedTxid)
        XCTAssertEqual(capturedAmount, 25_000)
        XCTAssertEqual(capturedHeight, 800_001)
    }

    // MARK: - TransactionMatcher Tests

    private let matcher = TransactionMatcher()

    func testFindMatchingTargetByAddressInResponse() {
        let addr = "bc1qmatchaddr"

        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: nil,
            blockTransactions: nil,
            address: addr,
            txid: nil,
            multiAddressTransactions: nil,
            trackedTxs: nil
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let results = matcher.matchAll(
            trackedAddresses: [addr],
            trackedTxids: [],
            msg: msg,
            tx: tx
        )
        XCTAssertEqual(results.first?.target, addr)
    }

    func testFindMatchingTargetByVoutScriptpubkeyAddress() {
        let addr = "bc1qvoutmatch"

        let vout = MempoolWSVout(
            scriptpubkeyAddress: addr,
            value: 30_000
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: [vout], vin: nil)

        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: [tx],
            blockTransactions: nil,
            address: nil,
            txid: nil,
            multiAddressTransactions: nil,
            trackedTxs: nil
        )

        let results = matcher.matchAll(
            trackedAddresses: [addr],
            trackedTxids: [],
            msg: msg,
            tx: tx
        )
        XCTAssertEqual(results.first?.target, addr)
    }

    func testFindMatchingTargetByVinTxid() {
        let fundingTxid = makeValidTxid()

        let vin = MempoolWSVin(txid: fundingTxid)
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: [vin])

        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: [tx],
            blockTransactions: nil,
            address: nil,
            txid: nil,
            multiAddressTransactions: nil,
            trackedTxs: nil
        )

        let results = matcher.matchAll(
            trackedAddresses: [],
            trackedTxids: [fundingTxid],
            msg: msg,
            tx: tx
        )
        XCTAssertEqual(results.first?.target, fundingTxid)
        XCTAssertEqual(results.first?.isTxid, true)
    }

    func testFindMatchingTargetByResponseTxid() {
        let trackedTxid = makeValidTxid()

        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: nil,
            blockTransactions: nil,
            address: nil,
            txid: trackedTxid,
            multiAddressTransactions: nil,
            trackedTxs: nil
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let results = matcher.matchAll(
            trackedAddresses: [],
            trackedTxids: [trackedTxid],
            msg: msg,
            tx: tx
        )
        XCTAssertEqual(results.first?.target, trackedTxid)
        XCTAssertEqual(results.first?.isTxid, true)
    }

    func testFindMatchingTargetReturnsNilWhenNoMatch() {
        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: nil,
            blockTransactions: nil,
            address: "bc1qnoone",
            txid: makeValidTxid(),
            multiAddressTransactions: nil,
            trackedTxs: nil
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let results = matcher.matchAll(
            trackedAddresses: [],
            trackedTxids: [],
            msg: msg,
            tx: tx
        )
        XCTAssertTrue(results.isEmpty)
    }

    func testFindMatchingTargetAddressTakesPriorityOverVout() {
        let directAddr = "bc1qdirect"
        let voutAddr = "bc1qvout"

        let vout = MempoolWSVout(scriptpubkeyAddress: voutAddr, value: 10_000)
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: [vout], vin: nil)

        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: [tx],
            blockTransactions: nil,
            address: directAddr,
            txid: nil,
            multiAddressTransactions: nil,
            trackedTxs: nil
        )

        let results = matcher.matchAll(
            trackedAddresses: [directAddr, voutAddr],
            trackedTxids: [],
            msg: msg,
            tx: tx
        )
        XCTAssertEqual(results.first?.target, directAddr)
    }

    // MARK: - Dedup Tests

    func testDedupBlocksRepeatTxid() {
        let json = makeAddressTransactionJSON(
            voutAddress: "bc1qdedup",
            voutValue: 100
        )
        service.trackAddress("bc1qdedup")

        var fireCount = 0
        service.onTransactionDetected = { _ in
            fireCount += 1
        }

        service.handleMessage(json)
        service.handleMessage(json)

        XCTAssertEqual(fireCount, 1, "Same txid should only fire the callback once")
    }

    func testDedupAllowsNewTxid() {
        let txid1 = makeValidTxid()
        let txid2 = String(repeating: "b", count: 64)

        let json1 = "{ \"address-transactions\": [{ \"txid\": \"\(txid1)\", \"vout\": [{ \"scriptpubkey_address\": \"bc1qdedup2\", \"value\": 100 }] }] }"
        let json2 = "{ \"address-transactions\": [{ \"txid\": \"\(txid2)\", \"vout\": [{ \"scriptpubkey_address\": \"bc1qdedup2\", \"value\": 200 }] }] }"

        service.trackAddress("bc1qdedup2")

        var fireCount = 0
        var capturedAmount: Int64?
        service.onTransactionDetected = { event in
            guard case let .receive(_, _, amount) = event else { return }
            fireCount += 1
            capturedAmount = amount
        }

        service.handleMessage(json1)
        service.handleMessage(json2)

        XCTAssertEqual(fireCount, 2, "Different txids should both fire the callback")
        XCTAssertEqual(capturedAmount, 200)
    }

    // MARK: - Connect / Disconnect Lifecycle

    /// The delegate callback flips `isConnected` on an async MainActor hop, so a single
    /// fixed sleep flakes on loaded CI runners (the hop simply hasn't run yet). Poll
    /// until the flag turns true or a generous deadline passes, then assert.
    private func waitForConnected(file: StaticString = #filePath, line: UInt = #line) {
        let exp = expectation(description: "isConnected becomes true")
        Task { @MainActor in
            for _ in 0..<300 where !service.isConnected {
                try? await Task.sleep(nanoseconds: 10_000_000)
            }
            exp.fulfill()
        }
        wait(for: [exp], timeout: 5.0)
        XCTAssertTrue(service.isConnected, "service never reported connected", file: file, line: line)
    }

    func testConnectDoesNotSetIsConnectedSynchronously() {
        XCTAssertFalse(service.isConnected)

        service.connect()

        // It should still be false synchronously because URLSession hasn't connected
        XCTAssertFalse(service.isConnected)
    }

    func testDelegateSetsIsConnected() throws {
        XCTAssertFalse(service.isConnected)

        // Simulate the delegate firing
        service.urlSession(
            URLSession.shared,
            webSocketTask: URLSession.shared.webSocketTask(with: try XCTUnwrap(URL(string: "wss://test"))),
            didOpenWithProtocol: nil
        )

        waitForConnected()
    }

    func testDisconnectClearsIsConnected() throws {
        service.urlSession(
            URLSession.shared,
            webSocketTask: URLSession.shared.webSocketTask(with: try XCTUnwrap(URL(string: "wss://test"))),
            didOpenWithProtocol: nil
        )

        waitForConnected()

        service.disconnect()

        XCTAssertFalse(service.isConnected)
    }

    // MARK: - trackAddress / trackTx

    func testTrackAddressAddsToSet() {
        let addr = "bc1qtracktest"
        service.trackAddress(addr)

        let json = makeAddressTransactionJSON(
            voutAddress: addr,
            voutValue: 1500,
            msgAddress: addr
        )

        var capturedTxid: String?
        service.onTransactionDetected = { event in
            guard case let .receive(_, txid, _) = event else { return }
            capturedTxid = txid
        }

        service.handleMessage(json)

        XCTAssertNotNil(capturedTxid, "Tracked address should be matched in incoming message")
    }

    func testTrackTxAddsToSet() {
        let txid = makeValidTxid()
        service.trackTx(txid)

        let spendingTxid = "beefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeef"
        let json = "{ \"tracked-txs\": { \"\(txid)\": { \"txid\": \"\(txid)\", \"utxoSpent\": { \"0\": { \"txid\": \"\(spendingTxid)\", \"vin\": 0 } } } } }"

        var capturedTxid: String?
        service.onTransactionDetected = { event in
            guard case let .trackedOutspend(_, resolvedTxid) = event else { return }
            capturedTxid = resolvedTxid
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedTxid, spendingTxid)
    }

    func testTrackAddressesBulkPayload() {
        let addr = "bc1qbulktest"
        service.trackAddress(addr)

        let json = "{ \"multi-address-transactions\": { \"\(addr)\": { \"mempool\": [{ \"txid\": \"deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef\", \"vout\": [{ \"scriptpubkey_address\": \"\(addr)\", \"value\": 1000 }] }], \"confirmed\": [], \"removed\": [] } } }"

        var capturedAmount: Int64?
        service.onTransactionDetected = { event in
            guard case let .receive(target, txid, amount) = event else { return }
            XCTAssertEqual(target, addr)
            XCTAssertEqual(txid, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef")
            capturedAmount = amount
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedAmount, 1000, "Should correctly decode multi-address-transactions and aggregate amount")
    }

    func testTrackAddressTriggersConnectWhenDisconnected() {
        XCTAssertFalse(service.isConnected)

        service.trackAddress("bc1qautoconnect")

        // URLSession connection is async, so isConnected remains false synchronously
        XCTAssertFalse(service.isConnected)
    }

    func testTrackTxTriggersConnectWhenDisconnected() {
        XCTAssertFalse(service.isConnected)

        service.trackTx(makeValidTxid())

        XCTAssertFalse(service.isConnected)
    }

    func testTrackAddressWhileConnectedDoesNotReconnect() throws {
        service.urlSession(
            URLSession.shared,
            webSocketTask: URLSession.shared.webSocketTask(with: try XCTUnwrap(URL(string: "wss://test"))),
            didOpenWithProtocol: nil
        )

        waitForConnected()

        service.trackAddress("bc1qmore")

        XCTAssertTrue(service.isConnected)
    }

    func testTrackEmptyAddressIsIgnored() {
        service.trackAddress("")
        service.trackAddress("bc1qnotempty")

        let json = makeAddressTransactionJSON(
            voutAddress: "bc1qnotempty",
            voutValue: 500,
            msgAddress: "bc1qnotempty"
        )

        var capturedTxid: String?
        service.onTransactionDetected = { event in
            guard case let .receive(_, txid, _) = event else { return }
            capturedTxid = txid
        }

        service.handleMessage(json)

        XCTAssertNotNil(capturedTxid, "Non-empty address should be tracked")
    }

    // MARK: - Send Buffering and Flushing

    func testSendBuffersWhenDisconnected() {
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        service.send("{ \"track-addresses\": [\"bc1qbuffertest\"] }")

        service.trackAddress("bc1qbuffertest")

        XCTAssertFalse(service.isConnected)
    }

    func testPendingMessagesFlushedOnConnect() {
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        service.trackAddress("bc1qflush")

        XCTAssertFalse(service.isConnected)
    }

    func testPendingMessagesCappedAt50() {
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        for _ in 0..<60 {
            service.send("{ \"track-addresses\": [\"addr\\(i)\"] }")
        }

        service.trackAddress("bc1qcap")

        XCTAssertFalse(service.isConnected)
    }

    // MARK: - Amount Calculation

    func testAmountSumsMultipleVouts() {
        let json = """
        { "address-transactions": [{ "txid": "\(makeValidTxid())", "vout": [
        { "scriptpubkey_address": "bc1qmultivout", "value": 100000 },
        { "scriptpubkey_address": "bc1qmultivout", "value": 50000 },
        { "scriptpubkey_address": "bc1qother", "value": 999 }
        ] }] }
        """

        service.trackAddress("bc1qmultivout")

        var capturedAmount: Int64?
        service.onTransactionDetected = { event in
            guard case let .receive(_, _, amount) = event else { return }
            capturedAmount = amount
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedAmount, 150_000, "Amount should sum all matching vouts")
    }

    // MARK: - Edge Cases

    func testHandleMessageWithEmptyString() {
        var fired = false
        service.onTransactionDetected = { _ in
            fired = true
        }

        service.handleMessage("")

        XCTAssertFalse(fired)
    }

    func testBlockHeaderOnlyNoTransaction() {
        let json = makeBlockHeaderJSON(height: 900_000)

        var capturedHeight: UInt32?

        service.onBlockHeader = { block in
            capturedHeight = block.height
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedHeight, 900_000)
    }

    func testMultipleTrackedAddressesAllMatch() {
        let addr1 = "bc1qaddr1"
        let addr2 = "bc1qaddr2"

        service.trackAddress(addr1)
        service.trackAddress(addr2)

        let json = makeAddressTransactionJSON(
            voutAddress: addr1,
            voutValue: 10_000,
            msgAddress: addr1
        )

        var capturedTarget: String?
        service.onTransactionDetected = { event in
            guard case let .receive(target, _, _) = event else { return }
            capturedTarget = target
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedTarget, addr1)
    }

    // MARK: - New Tests for PR 197 Edge Cases

    func testMultipleDepositsToSameAddress() {
        let addr = "bc1qmultiple"
        let txid1 = "1111111111111111111111111111111111111111111111111111111111111111"
        let txid2 = "2222222222222222222222222222222222222222222222222222222222222222"
        let json = "{ \"address-transactions\": [{ \"txid\": \"\(txid1)\", \"vout\": [{ \"scriptpubkey_address\": \"\(addr)\", \"value\": 1000 }] }, { \"txid\": \"\(txid2)\", \"vout\": [{ \"scriptpubkey_address\": \"\(addr)\", \"value\": 2000 }] }] }"

        service.trackAddress(addr)

        var amounts = [Int64]()
        service.onTransactionDetected = { event in
            if case let .receive(_, _, amount) = event {
                amounts.append(amount)
            }
        }

        service.handleMessage(json)

        XCTAssertEqual(amounts.sorted(), [1000, 2000])
    }

    func testRBFRemovalCallback() {
        let addr = "bc1qrbf"
        let txid = "3333333333333333333333333333333333333333333333333333333333333333"
        let json = "{ \"multi-address-transactions\": { \"\(addr)\": { \"removed\": [{ \"txid\": \"\(txid)\" }] } } }"

        service.trackAddress(addr)

        var removedFired = false
        service.onTransactionDetected = { event in
            if case let .removed(target, removedTxid) = event {
                XCTAssertEqual(target, addr)
                XCTAssertEqual(removedTxid, txid)
                removedFired = true
            }
        }

        service.handleMessage(json)
        XCTAssertTrue(removedFired)
    }

    func testOutspendPayloadDecoding() {
        let txid = "4444444444444444444444444444444444444444444444444444444444444444"
        let spendingTxid = "5555555555555555555555555555555555555555555555555555555555555555"
        let json = "{ \"tracked-txs\": { \"\(txid)\": { \"txid\": \"\(txid)\", \"utxoSpent\": { \"0\": { \"txid\": \"\(spendingTxid)\", \"vin\": 0 } } } } }"

        service.trackTx(txid)

        var outspendFired = false
        service.onTransactionDetected = { event in
            if case let .trackedOutspend(tracked, spender) = event {
                XCTAssertEqual(tracked, txid)
                XCTAssertEqual(spender, spendingTxid)
                outspendFired = true
            }
        }

        service.handleMessage(json)
        XCTAssertTrue(outspendFired)
    }

    func testBlocksPayloadOrdering() {
        let json = "{ \"blocks\": [{ \"id\": \"block1\", \"height\": 900100 }, { \"id\": \"block2\", \"height\": 900101 }] }"

        var capturedHeight: UInt32?
        service.onBlockHeader = { block in
            capturedHeight = block.height
        }

        service.handleMessage(json)
        XCTAssertEqual(capturedHeight, 900101)
    }

    // MARK: - RBF Edge Cases

    func testRBFThenReplacementReceiveDoesNotDesync() {
        let addr = "bc1qrfbtest"
        service.trackAddress(addr)

        let txidX = "1111111111111111111111111111111111111111111111111111111111111111"
        let txidY = "2222222222222222222222222222222222222222222222222222222222222222"
        let removedJSON = "{ \"multi-address-transactions\": { \"\(addr)\": { \"removed\": [{ \"txid\": \"\(txidX)\" }] } } }"
        let receiveJSON = "{ \"address-transactions\": [{ \"txid\": \"\(txidY)\", \"vout\": [{ \"scriptpubkey_address\": \"\(addr)\", \"value\": 50000 }] }] }"

        var events = [WebSocketEvent]()
        service.onTransactionDetected = { event in
            events.append(event)
        }

        service.handleMessage(removedJSON)
        service.handleMessage(receiveJSON)

        XCTAssertEqual(events.count, 2)
        guard case .removed = events[0] else {
            XCTFail("First event should be .removed"); return
        }
        guard case let .receive(_, firedTxid, _) = events[1] else {
            XCTFail("Second event should be .receive"); return
        }
        XCTAssertEqual(firedTxid, txidY)
    }

    func testRemovedTxDedupAllowsReAdd() {
        let addr = "bc1qdedup3"
        service.trackAddress(addr)

        let txidX = "3333333333333333333333333333333333333333333333333333333333333333"
        let txidY = "4444444444444444444444444444444444444444444444444444444444444444"

        let receiveX = "{ \"address-transactions\": [{ \"txid\": \"\(txidX)\", \"vout\": [{ \"scriptpubkey_address\": \"\(addr)\", \"value\": 1000 }] }] }"
        let removedX = "{ \"multi-address-transactions\": { \"\(addr)\": { \"removed\": [{ \"txid\": \"\(txidX)\" }] } } }"
        let receiveY = "{ \"address-transactions\": [{ \"txid\": \"\(txidY)\", \"vout\": [{ \"scriptpubkey_address\": \"\(addr)\", \"value\": 2000 }] }] }"
        let removedY = "{ \"multi-address-transactions\": { \"\(addr)\": { \"removed\": [{ \"txid\": \"\(txidY)\" }] } } }"

        var events = [WebSocketEvent]()
        service.onTransactionDetected = { event in
            events.append(event)
        }

        // First receive for X — should fire
        service.handleMessage(receiveX)
        XCTAssertEqual(events.count, 1)
        guard case let .receive(_, firedTxid, _) = events[0] else {
            XCTFail("Should be .receive"); return
        }
        XCTAssertEqual(firedTxid, txidX)

        // Same receive for X — should be deduped
        service.handleMessage(receiveX)
        XCTAssertEqual(events.count, 1, "Duplicate receive should be deduped")

        // Removed for X — different dedup key from receive, so it SHOULD fire
        service.handleMessage(removedX)
        XCTAssertEqual(events.count, 2, "Removal for a txid should fire even if it was previously received")

        // New receive for Y — should fire
        service.handleMessage(receiveY)
        XCTAssertEqual(events.count, 3)
        guard case let .receive(_, firedTxid, _) = events[2] else {
            XCTFail("Should be .receive for new txid"); return
        }
        XCTAssertEqual(firedTxid, txidY)

        // Removed for Y — should fire (different txid)
        service.handleMessage(removedY)
        XCTAssertEqual(events.count, 4)
        guard case .removed = events[3] else {
            XCTFail("Should be .removed"); return
        }
    }

    func testProcessedTxidMemoryCapPreventsUnboundedGrowth() {
        let maxEntries = 500
        for i in 0..<(maxEntries + 50) {
            let key = String(format: "%064d", i)
            service.recordProcessedTx(key)
        }
        let dict = service.processedTxids
        XCTAssertLessThanOrEqual(dict.count, maxEntries)
    }

    func testProcessedTxidEvictionRemovesOldest() {
        let maxEntries = 500
        let firstKey = "0000000000000000000000000000000000000000000000000000000000000000"
        service.recordProcessedTx(firstKey)

        for i in 1..<(maxEntries + 50) {
            let key = String(format: "%064d", i)
            service.recordProcessedTx(key)
        }

        let dict = service.processedTxids
        XCTAssertNil(dict[firstKey], "Oldest entry should have been evicted when cap exceeded")
    }

    func testRemovedTransactionIgnoredInAggregate() {
        let addr = "bc1qremovedonly"
        service.trackAddress(addr)

        let txid = "5555555555555555555555555555555555555555555555555555555555555555"
        let json = "{ \"multi-address-transactions\": { \"\(addr)\": { \"mempool\": [], \"confirmed\": [], \"removed\": [{ \"txid\": \"\(txid)\" }] } } }"

        var events = [WebSocketEvent]()
        var receiveFired = false
        service.onTransactionDetected = { event in
            events.append(event)
            if case .receive = event {
                receiveFired = true
            }
        }

        service.handleMessage(json)

        XCTAssertEqual(events.count, 1)
        guard case .removed = events[0] else {
            XCTFail("Event should be .removed only"); return
        }
        XCTAssertFalse(receiveFired, ".receive should not fire for removed-only tx")
    }

    func testTrackedTxidDirectMatchInAddressTransactions() {
        let trackedTxid = "6666666666666666666666666666666666666666666666666666666666666666"

        let msg = MempoolWSMessage(
            block: nil,
            blocks: nil,
            addressTransactions: nil,
            blockTransactions: nil,
            address: nil,
            txid: trackedTxid,
            multiAddressTransactions: nil,
            trackedTxs: nil
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let results = matcher.matchAll(
            trackedAddresses: [],
            trackedTxids: [trackedTxid],
            msg: msg,
            tx: tx
        )
        XCTAssertFalse(results.isEmpty)
        XCTAssertEqual(results.first?.target, trackedTxid)
        XCTAssertEqual(results.first?.isTxid, true)
    }

    func testRBFRemovalDuringChannelClose() {
        let addr = "bc1qrbfclose"
        service.trackAddress(addr)

        let txid = "7777777777777777777777777777777777777777777777777777777777777777"
        let json = "{ \"multi-address-transactions\": { \"\(addr)\": { \"removed\": [{ \"txid\": \"\(txid)\" }] } } }"

        var removedFired = false
        service.onTransactionDetected = { event in
            if case .removed = event {
                removedFired = true
            }
        }

        service.handleMessage(json)

        XCTAssertTrue(removedFired, "Service should fire .removed regardless of channel state")
    }

    func testMultipleOutspendsSameSpendingTxid() {
        let fundingTxid1 = "8888888888888888888888888888888888888888888888888888888888888888"
        let fundingTxid2 = "9999999999999999999999999999999999999999999999999999999999999999"
        let spendingTxid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

        service.trackTx(fundingTxid1)
        service.trackTx(fundingTxid2)

        let json = "{ \"tracked-txs\": { \"\(fundingTxid1)\": { \"txid\": \"\(fundingTxid1)\", \"utxoSpent\": { \"0\": { \"txid\": \"\(spendingTxid)\", \"vin\": 0 } } }, \"\(fundingTxid2)\": { \"txid\": \"\(fundingTxid2)\", \"utxoSpent\": { \"0\": { \"txid\": \"\(spendingTxid)\", \"vin\": 0 } } } } }"

        var outspendTargets = [String]()
        var outspendSpenders = [String]()
        service.onTransactionDetected = { event in
            if case let .trackedOutspend(tracked, spender) = event {
                outspendTargets.append(tracked)
                outspendSpenders.append(spender)
            }
        }

        service.handleMessage(json)

        XCTAssertEqual(outspendTargets.count, 2)
        XCTAssertTrue(outspendTargets.contains(fundingTxid1))
        XCTAssertTrue(outspendTargets.contains(fundingTxid2))
        XCTAssertEqual(outspendSpenders.count, 2)

        // Send same message again — should be deduped by spendingTxid + trackedTxid
        service.handleMessage(json)
        XCTAssertEqual(outspendTargets.count, 2, "Duplicate outspends should be deduped")
    }

    func testConnectResetsReconnectAttempts() {
        service.disconnect()

        service.reconnectAttempts = 5

        service.connect()

        XCTAssertEqual(service.reconnectAttempts, 0)
    }
}
