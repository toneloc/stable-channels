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
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run())
        XCTAssertTrue(coordinator.isRetrying)

        _ = try await start(coordinator, results: [nil])

        XCTAssertEqual(coordinator.retryAttempt, 0)
        XCTAssertFalse(coordinator.isRetrying)
    }

    // MARK: - Retry scheduling

    /// A retry run with fakes: `shouldRun` gates it, `probe` returns `url` after `probe` runs.
    private func run(
        shouldRun: @escaping @MainActor () -> Bool = { true },
        probe: @escaping @MainActor () async -> Void = {},
        start: @escaping @MainActor (String) async -> Void = { _ in },
        stop: @escaping @MainActor () -> Void = {}
    ) -> NodeStartupCoordinator.RetryRun {
        NodeStartupCoordinator.RetryRun(
            shouldRun: shouldRun,
            probe: {
                await probe()
                return self.fallback
            },
            start: start,
            stop: stop
        )
    }

    private func waitUntilIdle(_ coordinator: NodeStartupCoordinator) async {
        // The running handle is cleared just after the run returns.
        for _ in 0 ..< 100 where coordinator.hasRetryRunning {
            await Task.yield()
        }
    }

    func testScheduleRetry_waitsPolicyDelayThenProbesAndStarts() async {
        let coordinator = makeCoordinator()
        let started = expectation(description: "node started")

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { url in
            self.startedURLs.append(url)
            started.fulfill()
        }))
        await fulfillment(of: [started], timeout: 1)

        XCTAssertEqual(sleeps, [.seconds(1)])
        XCTAssertEqual(startedURLs, [fallback])
        XCTAssertEqual(coordinator.retryAttempt, 1)
        XCTAssertEqual(eventNames(), ["NODE_START_RETRY_SCHEDULED"])
        XCTAssertEqual(events[0].data["attempt"] as? String, "1")
        XCTAssertEqual(events[0].data["delay_s"] as? String, "1")
    }

    func testScheduleRetry_gateCanStopTheRunBeforeProbing() async {
        let coordinator = makeCoordinator()
        var probed = false

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(
            shouldRun: { false },
            probe: { probed = true }
        ))
        for _ in 0 ..< 100 where sleeps.isEmpty {
            await Task.yield()
        }
        await waitUntilIdle(coordinator)

        XCTAssertFalse(probed)
        XCTAssertFalse(coordinator.hasRetryRunning)
    }

    func testScheduleRetry_failureInsideRetryBacksOffFurther() async {
        let coordinator = makeCoordinator()
        let second = expectation(description: "second retry started")
        var runs = 0

        func retryRun() -> NodeStartupCoordinator.RetryRun {
            run(start: { _ in
                runs += 1
                if runs == 1 {
                    coordinator.scheduleRetry(maxDelay: self.maxDelay, reason: "stall again", run: retryRun())
                } else {
                    second.fulfill()
                }
            })
        }
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: retryRun())
        await fulfillment(of: [second], timeout: 1)

        XCTAssertEqual(sleeps, [.seconds(1), .seconds(2)])
        XCTAssertEqual(coordinator.retryAttempt, 2)
    }

    func testScheduleRetry_pendingRetryIsNotDuplicated() {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run())
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run())

        XCTAssertEqual(coordinator.retryAttempt, 1)
        XCTAssertEqual(eventNames(), ["NODE_START_RETRY_SCHEDULED"])
        coordinator.reset()
    }

    func testPause_keepsBackoffAndReset_clearsIt() {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run())

        coordinator.pause()
        XCTAssertTrue(coordinator.isRetrying)
        coordinator.resumeIfIdle(maxDelay: maxDelay, reason: "resumed", run: run())
        XCTAssertEqual(coordinator.retryAttempt, 2)

        coordinator.reset()
        XCTAssertEqual(coordinator.retryAttempt, 0)
        XCTAssertFalse(coordinator.isRetrying)
    }

    func testPauseDuringProbe_neverStartsTheNode() async {
        let coordinator = makeCoordinator()
        let probing = expectation(description: "probe started")
        let probeEnded = expectation(description: "probe ended")
        var started = false

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(
            probe: {
                probing.fulfill()
                // Stands in for the provider probe; cancellation cuts it short.
                try? await Task.sleep(for: .seconds(5))
                probeEnded.fulfill()
            },
            start: { _ in started = true }
        ))
        await fulfillment(of: [probing], timeout: 1)
        XCTAssertTrue(coordinator.hasRetryRunning)

        coordinator.pause()
        await fulfillment(of: [probeEnded], timeout: 1)
        await waitUntilIdle(coordinator)

        XCTAssertFalse(started, "A retry paused mid-probe must not start the node")
        XCTAssertFalse(coordinator.hasRetryRunning)
        XCTAssertTrue(coordinator.isRetrying, "Pause keeps the backoff for foreground to resume")
    }

    func testResumeIfIdle_doesNotScheduleWhileARetryIsRunning() async {
        let coordinator = makeCoordinator()
        let probing = expectation(description: "probe started")

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(
            probe: {
                probing.fulfill()
                try? await Task.sleep(for: .seconds(5))
            }
        ))
        await fulfillment(of: [probing], timeout: 1)

        coordinator.resumeIfIdle(maxDelay: maxDelay, reason: "resumed on foreground", run: run())

        XCTAssertEqual(coordinator.retryAttempt, 1)
        XCTAssertEqual(eventNames(), ["NODE_START_RETRY_SCHEDULED"])
        coordinator.reset()
    }

    func testRunningRetry_isClearedWhenItFinishes() async {
        let coordinator = makeCoordinator()
        let started = expectation(description: "node started")

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in started.fulfill() }))
        await fulfillment(of: [started], timeout: 1)
        await waitUntilIdle(coordinator)

        XCTAssertFalse(coordinator.hasRetryRunning)
    }

    func testPauseDuringStart_stopsTheStartedNode() async {
        let coordinator = makeCoordinator()
        let stopped = expectation(description: "node stopped")

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(
            // Stands in for backgrounding while LDK's uncancellable start blocks.
            start: { _ in coordinator.pause() },
            stop: { stopped.fulfill() }
        ))

        await fulfillment(of: [stopped], timeout: 1)
        coordinator.reset()
    }

    func testForegroundDuringStart_keepsTheStartedNode() async {
        let coordinator = makeCoordinator()
        let finished = expectation(description: "start returned")
        var stopped = false

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(
            start: { _ in
                coordinator.pause()
                coordinator.resumeIfIdle(maxDelay: self.maxDelay, reason: "resumed on foreground", run: self.run())
                finished.fulfill()
            },
            stop: { stopped = true }
        ))
        await fulfillment(of: [finished], timeout: 1)

        XCTAssertFalse(stopped, "A start that finishes after foreground must keep the node")
        coordinator.reset()
    }

    func testSuccessfulStartInsideARetry_doesNotCancelTheRetry() async {
        let coordinator = makeCoordinator()
        let finished = expectation(description: "retry finished")
        var cancelledAfterStart = true

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in
            _ = try? await coordinator.start(self.plan, isRetryable: { _ in true }) { _ in }
            // The wallet sync after the start runs in this task, so success must not cancel it.
            cancelledAfterStart = Task.isCancelled
            finished.fulfill()
        }))
        await fulfillment(of: [finished], timeout: 1)

        XCTAssertFalse(cancelledAfterStart)
        XCTAssertFalse(coordinator.isRetrying)
    }

    func testScheduleWhilePaused_countsTheAttemptAndForegroundArmsIt() async {
        let coordinator = makeCoordinator()
        let started = expectation(description: "node started")
        var startedWhilePaused = false

        coordinator.pause()
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in
            startedWhilePaused = true
        }))
        for _ in 0 ..< 20 {
            await Task.yield()
        }
        XCTAssertFalse(startedWhilePaused, "A suspended app must not start the node")
        XCTAssertTrue(sleeps.isEmpty)
        XCTAssertTrue(coordinator.isRetrying, "The attempt is kept for foreground")

        coordinator.resumeIfIdle(maxDelay: maxDelay, reason: "resumed on foreground", run: run(start: { _ in
            started.fulfill()
        }))
        await fulfillment(of: [started], timeout: 1)
        XCTAssertEqual(coordinator.retryAttempt, 2)
    }

    func testCancelledStart_runsNoFurtherAttempts() async {
        let coordinator = makeCoordinator()
        let task = Task { () -> Error? in
            do {
                _ = try await self.start(coordinator, results: [nil])
                return nil
            } catch {
                return error
            }
        }
        task.cancel()
        let error = await task.value

        XCTAssertTrue(error is CancellationError)
        XCTAssertTrue(startedURLs.isEmpty)
    }

    func testCancelledRetry_doesNotRun() async throws {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .milliseconds(50)) })
        var started = false

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in started = true }))
        coordinator.reset()
        try await Task.sleep(for: .milliseconds(150))

        XCTAssertFalse(started)
    }

    // MARK: - App wiring

    func testForegroundInAnyPhase_endsThePause() async {
        let appState = AppState()
        appState.phase = .offline
        appState.nodeStartup.pause()

        await appState.restartNodeFromForeground()

        XCTAssertFalse(appState.nodeStartup.isPaused, "A later retry must keep its node in foreground")
    }

    func testForegroundWhileRetrying_disarmsTheBackgroundStop() async {
        let appState = AppState()
        appState.phase = .syncing
        appState.nodeStartup.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(shouldRun: { false }))
        appState.stopNodeForBackground()
        XCTAssertTrue(appState.hasPendingBackgroundStop)

        await appState.restartNodeFromForeground()

        XCTAssertFalse(appState.hasPendingBackgroundStop, "Foreground must disarm the pending background stop")
        XCTAssertTrue(appState.nodeStartup.isRetrying)
        appState.nodeStartup.reset()
    }
}
