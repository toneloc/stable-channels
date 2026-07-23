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
        vinPrevoutAddress: String? = nil,
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
        if let prevAddr = vinPrevoutAddress {
            vinDict["prevout"] = ["scriptpubkey_address": prevAddr]
        }
        if let prevTxid = vinTxid {
            if vinDict["prevout"] == nil {
                vinDict["prevout"] = [:]
            }
            var prevout = vinDict["prevout"] as? [String: Any] ?? [:]
            prevout["txid"] = prevTxid
            vinDict["prevout"] = prevout
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

        service.onTransactionDetected = { _, receivedTxid, amountSats in
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

        service.onBlockHeader = { height in
            capturedHeight = height
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedHeight, 800_000)
    }

    func testMalformedJSONReturnsNilDecode() {
        let json = makeMalformedJSON()

        var transactionFired = false
        var blockFired = false

        service.onTransactionDetected = { _, _, _ in
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
        service.onTransactionDetected = { _, _, _ in
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
        service.onTransactionDetected = { _, _, _ in
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

        service.onTransactionDetected = { _, txid, amount in
            capturedTxid = txid
            capturedAmount = amount
        }
        service.onBlockHeader = { height in
            capturedHeight = height
        }

        service.trackAddress("bc1qtestaddr")
        service.handleMessage(combinedJSON)

        XCTAssertNotNil(capturedTxid)
        XCTAssertEqual(capturedAmount, 25_000)
        XCTAssertEqual(capturedHeight, 800_001)
    }

    // MARK: - findMatchingTarget Tests

    func testFindMatchingTargetByAddressInResponse() {
        let addr = "bc1qmatchaddr"
        service.trackAddress(addr)

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: nil,
            address: addr,
            txid: nil
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertEqual(result, addr)
    }

    func testFindMatchingTargetByVoutScriptpubkeyAddress() {
        let addr = "bc1qvoutmatch"
        service.trackAddress(addr)

        let vout = MempoolWSVout(
            scriptpubkeyAddress: addr,
            value: 30_000
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: [vout], vin: nil)

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: [tx],
            address: nil,
            txid: nil
        )

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertEqual(result, addr)
    }

    func testFindMatchingTargetByVinPrevoutScriptpubkeyAddress() {
        let addr = "bc1qvinprevout"
        service.trackAddress(addr)

        let prevout = MempoolWSPrevout(scriptpubkeyAddress: addr)
        let vin = MempoolWSVin(txid: nil, prevout: prevout)
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: [vin])

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: [tx],
            address: nil,
            txid: nil
        )

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertEqual(result, addr)
    }

    func testFindMatchingTargetByVinTxid() {
        let fundingTxid = makeValidTxid()
        service.trackTx(fundingTxid)

        let vin = MempoolWSVin(txid: fundingTxid, prevout: nil)
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: [vin])

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: [tx],
            address: nil,
            txid: nil
        )

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertEqual(result, fundingTxid)
    }

    func testFindMatchingTargetByResponseTxid() {
        let trackedTxid = makeValidTxid()
        service.trackTx(trackedTxid)

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: nil,
            address: nil,
            txid: trackedTxid
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertEqual(result, trackedTxid)
    }

    func testFindMatchingTargetReturnsNilWhenNoMatch() {
        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: nil,
            address: "bc1qnoone",
            txid: makeValidTxid()
        )
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: nil)

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertNil(result)
    }

    func testFindMatchingTargetAddressTakesPriorityOverVout() {
        let directAddr = "bc1qdirect"
        let voutAddr = "bc1qvout"

        service.trackAddress(directAddr)
        service.trackAddress(voutAddr)

        let vout = MempoolWSVout(scriptpubkeyAddress: voutAddr, value: 10_000)
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: [vout], vin: nil)

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: [tx],
            address: directAddr,
            txid: nil
        )

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)
        XCTAssertEqual(result, directAddr)
    }

    // MARK: - Dedup Tests

    func testDedupBlocksRepeatTxid() {
        let json = makeAddressTransactionJSON(
            voutAddress: "bc1qdedup",
            voutValue: 100
        )
        service.trackAddress("bc1qdedup")

        var fireCount = 0
        service.onTransactionDetected = { _, _, _ in
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
        service.onTransactionDetected = { _, _, amount in
            fireCount += 1
            capturedAmount = amount
        }

        service.handleMessage(json1)
        service.handleMessage(json2)

        XCTAssertEqual(fireCount, 2, "Different txids should both fire the callback")
        XCTAssertEqual(capturedAmount, 200)
    }

    // MARK: - Connect / Disconnect Lifecycle

    func testConnectSetsIsConnected() {
        XCTAssertFalse(service.isConnected)

        service.connect()

        XCTAssertTrue(service.isConnected)
    }

    func testDoubleConnectIsNoop() {
        service.connect()
        XCTAssertTrue(service.isConnected)

        service.connect()
        XCTAssertTrue(service.isConnected)
    }

    func testDisconnectClearsIsConnected() {
        service.connect()
        XCTAssertTrue(service.isConnected)

        service.disconnect()

        XCTAssertFalse(service.isConnected)
    }

    func testDisconnectThenConnectReconnects() {
        service.connect()
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        service.connect()

        XCTAssertTrue(service.isConnected)
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
        service.onTransactionDetected = { _, txid, _ in
            capturedTxid = txid
        }

        service.handleMessage(json)

        XCTAssertNotNil(capturedTxid, "Tracked address should be matched in incoming message")
    }

    func testTrackTxAddsToSet() {
        let txid = makeValidTxid()
        service.trackTx(txid)

        let json = makeAddressTransactionJSON(
            msgTxid: txid
        )

        var capturedTxid: String?
        service.onTransactionDetected = { _, receivedTxid, _ in
            capturedTxid = receivedTxid
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedTxid, txid, "Tracked txid should be matched in incoming message")
    }

    func testTrackAddressTriggersConnectWhenDisconnected() {
        XCTAssertFalse(service.isConnected)

        service.trackAddress("bc1qautoconnect")

        XCTAssertTrue(service.isConnected, "trackAddress should auto-connect when disconnected")
    }

    func testTrackTxTriggersConnectWhenDisconnected() {
        XCTAssertFalse(service.isConnected)

        service.trackTx(makeValidTxid())

        XCTAssertTrue(service.isConnected, "trackTx should auto-connect when disconnected")
    }

    func testTrackAddressWhileConnectedDoesNotReconnect() {
        service.connect()
        XCTAssertTrue(service.isConnected)

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
        service.onTransactionDetected = { _, txid, _ in
            capturedTxid = txid
        }

        service.handleMessage(json)

        XCTAssertNotNil(capturedTxid, "Non-empty address should be tracked")
    }

    // MARK: - Send Buffering and Flushing

    func testSendBuffersWhenDisconnected() {
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        service.send("{ \"track-address\": \"bc1qbuffertest\" }")

        service.trackAddress("bc1qbuffertest")

        XCTAssertTrue(service.isConnected)
    }

    func testPendingMessagesFlushedOnConnect() {
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        service.trackAddress("bc1qflush")

        XCTAssertTrue(service.isConnected)
    }

    func testPendingMessagesCappedAt50() {
        service.disconnect()
        XCTAssertFalse(service.isConnected)

        for i in 0..<60 {
            service.send("{ \"track-address\": \"addr\(i)\" }")
        }

        service.trackAddress("bc1qcap")

        XCTAssertTrue(service.isConnected)
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
        service.onTransactionDetected = { _, _, amount in
            capturedAmount = amount
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedAmount, 150_000, "Amount should sum all matching vouts")
    }

    // MARK: - Edge Cases

    func testHandleMessageWithEmptyString() {
        var fired = false
        service.onTransactionDetected = { _, _, _ in
            fired = true
        }

        service.handleMessage("")

        XCTAssertFalse(fired)
    }

    func testBlockHeaderOnlyNoTransaction() {
        let json = makeBlockHeaderJSON(height: 900_000)

        var capturedHeight: UInt32?
        service.onBlockHeader = { height in
            capturedHeight = height
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
        service.onTransactionDetected = { target, _, _ in
            capturedTarget = target
        }

        service.handleMessage(json)

        XCTAssertEqual(capturedTarget, addr1)
    }

    func testVinPrevoutAddressTakesPriorityOverVinTxid() {
        let addr = "bc1qvinaddr"
        let vinTxid = makeValidTxid()

        service.trackAddress(addr)
        service.trackTx(vinTxid)

        let vinPrevout = MempoolWSPrevout(scriptpubkeyAddress: addr)
        let vin = MempoolWSVin(txid: vinTxid, prevout: vinPrevout)
        let tx = MempoolWSTransaction(txid: makeValidTxid(), vout: nil, vin: [vin])

        let msg = MempoolWSMessage(
            block: nil,
            addressTransactions: [tx],
            address: nil,
            txid: nil
        )

        let result = service.findMatchingTarget(msg: msg, firstTx: tx)

        XCTAssertEqual(result, addr)
    }
}
