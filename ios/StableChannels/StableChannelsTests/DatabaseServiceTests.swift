import XCTest
@testable import StableChannels

final class DatabaseServiceTests: XCTestCase {
    private var service: DatabaseService!
    private var dataDir: URL!

    override func setUp() {
        super.setUp()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("DatabaseServiceTests-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        service = try? DatabaseService(dataDir: dataDir)
    }

    override func tearDown() {
        try? FileManager.default.removeItem(at: dataDir)
        service = nil
        super.tearDown()
    }

    // MARK: - pending_operations

    func testPendingOperationsInsertFetch() {
        let ok = service.insertPendingOperation(
            opId: "close-abc",
            opType: "channel_close",
            fundingOutpointTxid: "deadbeef",
            fundingOutpointVout: 1
        )
        XCTAssertTrue(ok)

        let ops = service.fetchPendingOperations()
        XCTAssertEqual(ops.count, 1)
        let op = ops[0]
        XCTAssertEqual(op.opId, "close-abc")
        XCTAssertEqual(op.opType, "channel_close")
        XCTAssertEqual(op.fundingOutpointTxid, "deadbeef")
        XCTAssertEqual(op.fundingOutpointVout, 1)
        XCTAssertEqual(op.status, "pending")
        XCTAssertNil(op.closingTxid)
        XCTAssertNil(op.resolvedAt)
    }

    func testPendingOperationsUpdatePreservesRow() {
        _ = service.insertPendingOperation(
            opId: "close-xyz",
            opType: "channel_close",
            fundingOutpointTxid: "cafebabe",
            fundingOutpointVout: 0
        )
        let ok = service.updatePendingOperation(
            opId: "close-xyz",
            closingTxid: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            status: "resolved"
        )
        XCTAssertTrue(ok)

        let ops = service.fetchPendingOperations()
        XCTAssertEqual(ops.count, 1)
        let op = ops[0]
        XCTAssertEqual(op.opId, "close-xyz")
        XCTAssertEqual(op.opType, "channel_close")
        XCTAssertEqual(op.fundingOutpointTxid, "cafebabe")
        XCTAssertEqual(op.fundingOutpointVout, 0)
        XCTAssertEqual(op.closingTxid,
                       "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        XCTAssertEqual(op.status, "resolved")
        XCTAssertNotNil(op.resolvedAt)
    }

    func testUpdatePendingOperationOnlyUpdatesPending() {
        _ = service.insertPendingOperation(
            opId: "close-q",
            opType: "channel_close",
            fundingOutpointTxid: nil,
            fundingOutpointVout: nil
        )
        // First update succeeds and flips status to resolved.
        let first = service.updatePendingOperation(
            opId: "close-q",
            closingTxid: "first",
            status: "resolved"
        )
        XCTAssertTrue(first)

        // Second update must be a no-op because the row is no longer pending.
        let second = service.updatePendingOperation(
            opId: "close-q",
            closingTxid: "second",
            status: "resolved"
        )
        XCTAssertFalse(second, "Second update must not clobber a resolved row")

        let ops = service.fetchPendingOperations()
        XCTAssertEqual(ops.first?.closingTxid, "first")
    }
}
