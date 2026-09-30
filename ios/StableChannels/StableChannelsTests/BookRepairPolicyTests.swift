import XCTest
@testable import StableChannels

final class BookRepairPolicyTests: XCTestCase {
    func testCanAttemptRepairWhenAllConditionsMet() {
        let result = BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )
        XCTAssertTrue(result)
    }

    func testCannotAttemptRepairWhenUserChannelIdMissing() {
        let result = BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: false,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )
        XCTAssertFalse(result)
    }

    func testCannotAttemptRepairWhenChannelNotReady() {
        let result = BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: false,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )
        XCTAssertFalse(result)
    }

    func testCannotAttemptRepairWhenChannelClosing() {
        let result = BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: true,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )
        XCTAssertFalse(result)
    }

    func testCannotAttemptRepairWhenSweeping() {
        let result = BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: true,
            hasPendingSpliceInMemory: false,
            price: 100_000.0
        )
        XCTAssertFalse(result)
    }

    func testCannotAttemptRepairWhenPendingSpliceInMemory() {
        let result = BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: true,
            price: 100_000.0
        )
        XCTAssertFalse(result)
    }

    func testCannotAttemptRepairWhenPriceZeroOrNegative() {
        XCTAssertFalse(BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: 0.0
        ))
        XCTAssertFalse(BookRepairPolicy.canAttemptRepair(
            hasUserChannelId: true,
            hasReadyChannel: true,
            isChannelClosing: false,
            isSweeping: false,
            hasPendingSpliceInMemory: false,
            price: -10_000.0
        ))
    }
}
