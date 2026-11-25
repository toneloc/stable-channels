@testable import StableChannels
import XCTest

final class TradeServiceAvailabilityTests: XCTestCase {
    private var tempDir: URL!
    private var databaseService: DatabaseService!
    private var nodeService: NodeService!

    override func setUpWithError() throws {
        try super.setUpWithError()
        tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("TradeServiceAvailabilityTests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        databaseService = try DatabaseService(dataDir: tempDir)
        nodeService = NodeService(dataDir: tempDir)
    }

    override func tearDownWithError() throws {
        nodeService = nil
        databaseService = nil
        if let tempDir {
            try? FileManager.default.removeItem(at: tempDir)
        }
        try super.tearDownWithError()
    }

    private func sampleChannel() -> StableChannel {
        var sc = StableChannel.default
        sc.expectedUSD = USD(amount: 50)
        return sc
    }

    func testBuyRefusedWhenStabilityIsNotInTheBuild() throws {
        let service = TradeService(
            nodeService: nodeService,
            databaseService: databaseService,
            balanceCache: nil,
            stabilityEnabled: false
        )

        XCTAssertThrowsError(
            try service.executeBuy(sc: sampleChannel(), amountUSD: 1, feeUSD: 0, price: 100_000)
        ) { error in
            XCTAssertEqual(error as? TradeValidationError, .unavailableInThisBuild)
        }
        XCTAssertFalse(try databaseService.channelRepo.hasUnattachedPreparedTrade())
    }

    func testSellRefusedWhenStabilityIsNotInTheBuild() throws {
        let service = TradeService(
            nodeService: nodeService,
            databaseService: databaseService,
            balanceCache: nil,
            stabilityEnabled: false
        )

        XCTAssertThrowsError(
            try service.executeSell(sc: sampleChannel(), amountUSD: 10, feeUSD: 0, price: 100_000)
        ) { error in
            XCTAssertEqual(error as? TradeValidationError, .unavailableInThisBuild)
        }
        XCTAssertFalse(try databaseService.channelRepo.hasUnattachedPreparedTrade())
    }

    func testUnavailableCopy() {
        XCTAssertEqual(
            TradeValidationError.unavailableInThisBuild.errorDescription,
            "Not available in this version."
        )
    }

    func testNormalBuildKeepsStability() {
        XCTAssertTrue(BuildFeatures.stability)
    }

    func testMaxSellIsZeroWhenStabilityIsNotInTheBuild() throws {
        let suite = "TradeServiceAvailabilityTests-\(UUID().uuidString)"
        let cache = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { cache.removePersistentDomain(forName: suite) }
        cache.set(Int64(50_000), forKey: AppState.BalanceCacheKey.lightning)
        cache.set(true, forKey: AppState.BalanceCacheKey.hasReadyChannel)
        func service(stabilityEnabled: Bool) -> TradeService {
            TradeService(
                nodeService: nodeService,
                databaseService: databaseService,
                balanceCache: cache,
                stabilityEnabled: stabilityEnabled
            )
        }
        XCTAssertGreaterThan(service(stabilityEnabled: true).maxSellCents(sc: StableChannel.default, price: 60_000), 0)
        XCTAssertEqual(service(stabilityEnabled: false).maxSellCents(sc: StableChannel.default, price: 60_000), 0)
    }
}
