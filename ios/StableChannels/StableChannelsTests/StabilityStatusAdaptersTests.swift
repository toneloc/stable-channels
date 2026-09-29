import XCTest
@testable import StableChannels

final class StabilityStatusAdaptersTests: XCTestCase {
    private var dataDir: URL!
    private var databaseService: DatabaseService!

    override func setUpWithError() throws {
        try super.setUpWithError()
        dataDir = FileManager.default
            .temporaryDirectory
            .appendingPathComponent("StabilityStatusAdaptersTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        databaseService = try DatabaseService(dataDir: dataDir)
    }

    override func tearDownWithError() throws {
        databaseService = nil
        if let dir = dataDir {
            try? FileManager.default.removeItem(at: dir)
        }
        try super.tearDownWithError()
    }

    func testDatabaseSpliceStatusAdapterWithCleanDatabaseReportsNoPendingSplice() {
        let adapter = DatabaseSpliceStatusAdapter(databaseService: databaseService)
        XCTAssertFalse(adapter.hasPendingSplice())
    }

    func testDatabaseSpliceStatusAdapterWithPendingSpliceReportsTrue() throws {
        let adapter = DatabaseSpliceStatusAdapter(databaseService: databaseService)
        _ = try databaseService.paymentRepo.recordPayment(
            paymentId: nil,
            paymentType: "splice_out",
            direction: "sent",
            amountMsat: 50_000_000,
            amountUSD: 25.0,
            btcPrice: 50_000.0,
            counterparty: nil,
            status: "pending",
            address: "bc1qtestaddress"
        )
        XCTAssertTrue(adapter.hasPendingSplice())
    }

    func testDatabaseStabilitySendStatusAdapterWithCleanDatabaseReportsNoPendingSend() {
        let adapter = DatabaseStabilitySendStatusAdapter(databaseService: databaseService)
        XCTAssertFalse(adapter.hasPendingStabilitySend())
    }

    func testDatabaseStabilitySendStatusAdapterWithPendingSendReportsTrue() {
        let adapter = DatabaseStabilitySendStatusAdapter(databaseService: databaseService)
        _ = databaseService.stabilityRepo.claimPendingSend(amountMsat: 10_000_000, price: 100_000)
        XCTAssertTrue(adapter.hasPendingStabilitySend())
    }

    func testLDKPaymentStatusAdapterWhenNodeReportsPendingPaymentReturnsTrueEvenIfDatabaseClean() {
        let adapter = LDKPaymentStatusAdapter(
            nodeService: NodeService(),
            databaseService: databaseService,
            hasPendingNodePaymentCheck: { true }
        )
        // LDK in-flight payment takes precedence over clean database
        XCTAssertTrue(adapter.hasPendingOutgoingPayments())
    }

    func testLDKPaymentStatusAdapterWhenNodeReportsNoPendingPaymentAndDatabaseCleanReturnsFalse() {
        let adapter = LDKPaymentStatusAdapter(
            nodeService: NodeService(),
            databaseService: databaseService,
            hasPendingNodePaymentCheck: { false }
        )
        XCTAssertFalse(adapter.hasPendingOutgoingPayments())
    }

    func testLDKPaymentStatusAdapterWhenNodeReportsNoPendingPaymentAndDatabaseHasPendingPaymentReturnsTrue() throws {
        let adapter = LDKPaymentStatusAdapter(
            nodeService: NodeService(),
            databaseService: databaseService,
            hasPendingNodePaymentCheck: { false }
        )
        _ = try databaseService.paymentRepo.recordPayment(
            paymentId: "test-payment",
            paymentType: "lightning",
            direction: "sent",
            amountMsat: 10_000_000,
            amountUSD: 10.0,
            btcPrice: 100_000.0,
            counterparty: nil,
            status: "pending"
        )
        XCTAssertTrue(adapter.hasPendingOutgoingPayments())
    }
}
