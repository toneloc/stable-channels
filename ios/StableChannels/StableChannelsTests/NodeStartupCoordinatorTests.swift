import XCTest
@testable import StableChannels

@MainActor
final class NodeStartupCoordinatorTests: XCTestCase {
    private enum TestError: Error, Equatable {
        case feeEstimation(Int)
        case database
    }

    private let primary = "https://primary.example/api"
    private let fallback = "https://fallback.example/api"
    private let delay = Duration.seconds(5)
    private let maxDelay = Duration.seconds(60)

    private var startedURLs: [String] = []
    private var sleeps: [Duration] = []
    private var events: [(name: String, data: [String: Any])] = []

    override func setUp() async throws {
        startedURLs = []
        sleeps = []
        events = []
    }

    private var plan: [NodeStartupPolicy.Attempt] {
        NodeStartupPolicy.attempts(preferred: primary, providers: [primary, fallback], retryDelay: delay)
    }

    private func makeCoordinator(sleep: ((Duration) async throws -> Void)? = nil) -> NodeStartupCoordinator {
        NodeStartupCoordinator(
            sleep: { duration in
                self.sleeps.append(duration)
                try await sleep?(duration)
            },
            log: { name, data in self.events.append((name, data)) }
        )
    }

    /// Starts the node with scripted results in order (nil = success).
    private func start(
        _ coordinator: NodeStartupCoordinator,
        results: [TestError?]
    ) async throws -> String {
        var remaining = results
        return try await coordinator.start(plan, isRetryable: { ($0 as? TestError) != .database }) { url in
            self.startedURLs.append(url)
            if let error = remaining.removeFirst() {
                throw error
            }
        }
    }

    private func eventNames() -> [String] {
        events.map(\.name)
    }

    // MARK: - Start

    func testFirstAttemptSucceeds_returnsItsURLWithoutSleepOrFailoverLogs() async throws {
        let url = try await start(makeCoordinator(), results: [nil])

        XCTAssertEqual(url, primary)
        XCTAssertEqual(startedURLs, [primary])
        XCTAssertTrue(sleeps.isEmpty)
        XCTAssertTrue(events.isEmpty)
    }

    func testTwoRetryableFailures_thirdAttemptSucceedsAfterDelay() async throws {
        let url = try await start(makeCoordinator(), results: [.feeEstimation(1), .feeEstimation(2), nil])

        XCTAssertEqual(url, primary)
        XCTAssertEqual(startedURLs, [primary, fallback, primary])
        XCTAssertEqual(sleeps, [delay])
        XCTAssertEqual(eventNames(), ["NODE_START_INITIAL_FAILED", "NODE_START_FAILOVER_SUCCESS"])
        XCTAssertEqual(events[0].data["esploraURL"] as? String, primary)
        XCTAssertEqual(events[1].data["using"] as? String, primary)
        XCTAssertEqual(events[1].data["attempt"] as? String, "2")
    }

    func testNonRetryableError_isRethrownWithoutFurtherAttempts() async {
        do {
            _ = try await start(makeCoordinator(), results: [.database])
            XCTFail("Expected error")
        } catch {
            XCTAssertEqual(error as? TestError, .database)
        }
        XCTAssertEqual(startedURLs, [primary])
        XCTAssertTrue(events.isEmpty)
    }

    func testAllAttemptsFail_throwsFirstErrorAndLogsFailoverFailed() async {
        do {
            _ = try await start(makeCoordinator(), results: [.feeEstimation(1), .feeEstimation(2), .feeEstimation(3)])
            XCTFail("Expected error")
        } catch {
            XCTAssertEqual(error as? TestError, .feeEstimation(1))
        }
        XCTAssertEqual(startedURLs, [primary, fallback, primary])
        XCTAssertEqual(eventNames(), ["NODE_START_INITIAL_FAILED", "NODE_START_FAILOVER_FAILED"])
        XCTAssertEqual(events[1].data["attempts"] as? String, "3")
    }

    func testSleepCancellation_propagatesAndStopsFurtherAttempts() async {
        let coordinator = makeCoordinator(sleep: { _ in throw CancellationError() })
        do {
            _ = try await start(coordinator, results: [.feeEstimation(1), .feeEstimation(2), nil])
            XCTFail("Expected cancellation")
        } catch {
            XCTAssertTrue(error is CancellationError)
        }
        XCTAssertEqual(startedURLs, [primary, fallback])
        XCTAssertFalse(eventNames().contains("NODE_START_FAILOVER_FAILED"))
    }

    func testEmptyAttempts_throwsNoAttempts() async {
        do {
            _ = try await makeCoordinator().start([], isRetryable: { _ in true }) { _ in }
            XCTFail("Expected error")
        } catch {
            XCTAssertEqual(error as? NodeStartupError, .noAttempts)
        }
    }

    func testSuccessfulStart_resetsRetryBackoff() async throws {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") {}
        XCTAssertTrue(coordinator.isRetrying)

        _ = try await start(coordinator, results: [nil])

        XCTAssertEqual(coordinator.retryAttempt, 0)
        XCTAssertFalse(coordinator.isRetrying)
    }

    // MARK: - Retry scheduling

    func testScheduleRetry_waitsPolicyDelayThenRuns() async {
        let coordinator = makeCoordinator()
        let ran = expectation(description: "retry ran")

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") { ran.fulfill() }
        await fulfillment(of: [ran], timeout: 1)

        XCTAssertEqual(sleeps, [.seconds(1)])
        XCTAssertEqual(coordinator.retryAttempt, 1)
        XCTAssertEqual(eventNames(), ["NODE_START_RETRY_SCHEDULED"])
        XCTAssertEqual(events[0].data["attempt"] as? String, "1")
        XCTAssertEqual(events[0].data["delay_s"] as? String, "1")
    }

    func testScheduleRetry_failureInsideRetryBacksOffFurther() async {
        let coordinator = makeCoordinator()
        let second = expectation(description: "second retry ran")
        var runs = 0

        func retry() {
            runs += 1
            if runs == 1 {
                coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall again") { retry() }
            } else {
                second.fulfill()
            }
        }
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") { retry() }
        await fulfillment(of: [second], timeout: 1)

        XCTAssertEqual(sleeps, [.seconds(1), .seconds(2)])
        XCTAssertEqual(coordinator.retryAttempt, 2)
    }

    func testScheduleRetry_pendingRetryIsNotDuplicated() {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") {}
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") {}

        XCTAssertEqual(coordinator.retryAttempt, 1)
        XCTAssertEqual(eventNames(), ["NODE_START_RETRY_SCHEDULED"])
        coordinator.reset()
    }

    func testPause_keepsBackoffAndReset_clearsIt() {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") {}

        coordinator.pause()
        XCTAssertTrue(coordinator.isRetrying)
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "resumed") {}
        XCTAssertEqual(coordinator.retryAttempt, 2)

        coordinator.reset()
        XCTAssertEqual(coordinator.retryAttempt, 0)
        XCTAssertFalse(coordinator.isRetrying)
    }

    func testCancelledRetry_doesNotRun() async throws {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .milliseconds(50)) })
        var ran = false

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall") { ran = true }
        coordinator.reset()
        try await Task.sleep(for: .milliseconds(150))

        XCTAssertFalse(ran)
    }
}
