import XCTest
@testable import StableChannels

final class ChannelSweepTests: XCTestCase {
    var dbService: DatabaseService!
    let tempDbUrl: URL = {
        let tempDir = FileManager.default.temporaryDirectory
        return tempDir.appendingPathComponent("test_sweep_\(UUID().uuidString).sqlite")
    }()

    override func setUpWithError() throws {
        dbService = try DatabaseService(dbURL: tempDbUrl)
        try dbService.migrate()
    }

    override func tearDownWithError() throws {
        dbService = nil
        try? FileManager.default.removeItem(at: tempDbUrl)
    }

    func testReconcileChannelsSurvivesOnMatchedChannelId() throws {
        try dbService.channelRepo.saveChannel(
            channelId: "live_chan_1",
            userChannelId: "stale_user_id_1",
            expectedUSD: 100.0,
            backingSats: 100000,
            nativeSats: 0,
            note: "",
            receiverSats: 100000,
            latestPrice: 60000.0
        )

        // Sweep with a different userChannelId but same channelId
        try dbService.channelRepo.reconcileChannels(
            liveUserChannelIds: ["new_user_id_1"],
            liveChannelIds: ["live_chan_1"]
        )

        let record = try dbService.channelRepo.loadChannel(userChannelId: "stale_user_id_1")
        XCTAssertNotNil(record)
        XCTAssertEqual(record?.channelId, "live_chan_1")
    }

    func testReconcileChannelsDeletesOnNeitherMatch() throws {
        try dbService.channelRepo.saveChannel(
            channelId: "dead_chan_1",
            userChannelId: "dead_user_id_1",
            expectedUSD: 100.0,
            backingSats: 100000,
            nativeSats: 0,
            note: "",
            receiverSats: 100000,
            latestPrice: 60000.0
        )

        // Sweep with completely different live lists
        try dbService.channelRepo.reconcileChannels(
            liveUserChannelIds: ["live_user_id"],
            liveChannelIds: ["live_chan_id"]
        )

        let record = try dbService.channelRepo.loadChannel(userChannelId: "dead_user_id_1")
        XCTAssertNil(record)
    }

    func testReconcileChannelsTruncatesOnZeroChannelBranch() throws {
        try dbService.channelRepo.saveChannel(
            channelId: "dead_chan_1",
            userChannelId: "dead_user_id_1",
            expectedUSD: 100.0,
            backingSats: 100000,
            nativeSats: 0,
            note: "",
            receiverSats: 100000,
            latestPrice: 60000.0
        )

        try dbService.channelRepo.reconcileChannels(
            liveUserChannelIds: [],
            liveChannelIds: []
        )

        let record = try dbService.channelRepo.loadChannel(userChannelId: "dead_user_id_1")
        XCTAssertNil(record)
    }
}
