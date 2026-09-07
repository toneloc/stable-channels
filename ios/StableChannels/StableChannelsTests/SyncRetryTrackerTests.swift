import XCTest
@testable import StableChannels

final class SyncRetryTrackerTests: XCTestCase {
    func testInitialAttemptDoesNotGiveUp() {
        let tracker = SyncRetryTracker(maxAttempts: 3, maxDurationSeconds: 60)
        let shouldGiveUp = tracker.recordAttemptAndShouldGiveUp(key: "hash-1")
        XCTAssertFalse(shouldGiveUp)
    }

    func testGivesUpWhenMaxAttemptsExceeded() {
        let tracker = SyncRetryTracker(maxAttempts: 3, maxDurationSeconds: 600)

        // Attempts 1, 2, 3 should not give up
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1")) // attempt 1
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1")) // attempt 2
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1")) // attempt 3

        // Attempt 4 exceeds maxAttempts (3) -> returns true (give up) and resets
        XCTAssertTrue(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        // Next attempt starts fresh
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))
    }

    func testGivesUpWhenMaxDurationExceeded() {
        var currentTime = Date(timeIntervalSince1970: 1000)
        let tracker = SyncRetryTracker(maxAttempts: 100, maxDurationSeconds: 60, now: { currentTime })

        // Initial attempt
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        // 30 seconds later (within duration)
        currentTime = Date(timeIntervalSince1970: 1030)
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        // 65 seconds later (exceeds 60s maxDuration)
        currentTime = Date(timeIntervalSince1970: 1065)
        XCTAssertTrue(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        // Key was evicted upon giving up, so next attempt starts fresh
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))
    }

    func testClearEvictsKey() {
        let tracker = SyncRetryTracker(maxAttempts: 2, maxDurationSeconds: 60)
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        // Clear tracking
        tracker.clear(key: "hash-1")

        // Starts fresh count at 1
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))
    }

    func testIndependentKeys() {
        let tracker = SyncRetryTracker(maxAttempts: 2, maxDurationSeconds: 60)
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-2"))

        // hash-1 reaches limit
        XCTAssertTrue(tracker.recordAttemptAndShouldGiveUp(key: "hash-1"))

        // hash-2 is still at attempt 2
        XCTAssertFalse(tracker.recordAttemptAndShouldGiveUp(key: "hash-2"))
        XCTAssertTrue(tracker.recordAttemptAndShouldGiveUp(key: "hash-2"))
    }

    func testConcurrentAccessThreadSafety() {
        let tracker = SyncRetryTracker(maxAttempts: 50, maxDurationSeconds: 60)
        let group = DispatchGroup()
        let queue = DispatchQueue(label: "SyncRetryTrackerTests.concurrent", attributes: .concurrent)

        for i in 0..<100 {
            group.enter()
            queue.async {
                let key = "key-\(i % 5)"
                _ = tracker.recordAttemptAndShouldGiveUp(key: key)
                if i % 10 == 0 {
                    tracker.clear(key: key)
                }
                group.leave()
            }
        }

        let waitResult = group.wait(timeout: .now() + 5.0)
        XCTAssertEqual(waitResult, .success)
    }
}
