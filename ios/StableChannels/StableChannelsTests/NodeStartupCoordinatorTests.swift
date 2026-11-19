import LDKNode
import os
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
    private var chainURLSuite = ""
    private var chainURLDefaults: UserDefaults!

    override func setUp() async throws {
        startedURLs = []
        sleeps = []
        events = []
        chainURLSuite = "node-startup-test-\(UUID().uuidString)"
        chainURLDefaults = UserDefaults(suiteName: chainURLSuite)
    }

    override func tearDown() async throws {
        chainURLDefaults.removePersistentDomain(forName: chainURLSuite)
        // A test node that started keeps the process-wide wallet-dir lock; free it for later tests.
        NodeDirLock.shared.release()
    }

    private var chainURLStore: UserDefaultsChainURLStore {
        UserDefaultsChainURLStore(defaults: chainURLDefaults)
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
            log: { name, data in self.events.append((name, data)) },
            chainURLStore: chainURLStore
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

    // MARK: - Start with failover

    func testStartWithFailover_recordsTheProviderThatWorked() async throws {
        var remaining: [TestError?] = [.feeEstimation(1), nil]
        let url = try await makeCoordinator().startWithFailover(
            preferred: primary,
            providers: [primary, fallback],
            retryDelay: delay,
            isRetryable: { _ in true }
        ) { url in
            self.startedURLs.append(url)
            if let error = remaining.removeFirst() { throw error }
        }

        XCTAssertEqual(url, fallback)
        XCTAssertEqual(startedURLs, [primary, fallback])
        XCTAssertEqual(chainURLStore.load(), fallback)
    }

    func testStartWithFailover_totalFailureKeepsTheRecordedProvider() async {
        chainURLStore.save(fallback)

        do {
            _ = try await makeCoordinator().startWithFailover(
                preferred: primary,
                providers: [primary, fallback],
                retryDelay: delay,
                isRetryable: { _ in true }
            ) { url in
                self.startedURLs.append(url)
                throw TestError.feeEstimation(1)
            }
            XCTFail("Expected the first error")
        } catch {
            XCTAssertEqual(error as? TestError, .feeEstimation(1))
        }
        XCTAssertEqual(startedURLs, [primary, fallback, primary])
        XCTAssertEqual(chainURLStore.load(), fallback)
    }

    /// The key is shared with the notification extension and values already saved on devices.
    func testChainURLStore_usesTheSharedAppGroupKey() {
        XCTAssertNil(chainURLStore.load())

        chainURLStore.save(fallback)

        XCTAssertEqual(chainURLDefaults.string(forKey: "esplora_chain_url"), fallback)
        XCTAssertEqual(chainURLStore.load(), fallback)
        XCTAssertEqual(Constants.SharedDefaultsKey.workingChainURL, "esplora_chain_url")
        XCTAssertEqual(Constants.SharedDefaultsKey.mainAppLastActive, "main_app_last_active")
    }

    // MARK: - Chain source probe

    func testProbe_reachablePrimary_startsOnPrimary() async {
        let probed = OSAllocatedUnfairLock(initialState: [URL]())
        let logged = OSAllocatedUnfairLock(initialState: [String]())
        let probe = ChainSourceProbe(
            isReachable: { url in
                probed.withLock { $0.append(url) }
                return true
            },
            primary: primary,
            fallback: fallback,
            log: { name, _ in logged.withLock { $0.append(name) } }
        )

        let url = await probe.resolve()

        XCTAssertEqual(url, primary)
        XCTAssertEqual(probed.withLock { $0 }.map(\.absoluteString), ["\(primary)/blocks/tip/height"])
        XCTAssertEqual(logged.withLock { $0 }, ["CHAIN_SOURCE_RESOLVED"])
    }

    func testProbe_unreachablePrimary_startsOnFallback() async {
        let logged = OSAllocatedUnfairLock(initialState: [(name: String, using: String?)]())
        let probe = ChainSourceProbe(
            isReachable: { _ in false },
            primary: primary,
            fallback: fallback,
            log: { name, data in
                let using = data["using"] as? String
                logged.withLock { $0.append((name, using)) }
            }
        )

        let url = await probe.resolve()

        XCTAssertEqual(url, fallback)
        XCTAssertEqual(logged.withLock { $0 }.map(\.name), ["CHAIN_SOURCE_FALLBACK"])
        XCTAssertEqual(logged.withLock { $0 }.first?.using, fallback)
    }

    /// Every live probe shares one session capped at the launch budget, so no probe leaves a session behind.
    func testLiveProbe_sharesOneSessionWithTheLaunchBudget() {
        let config = ChainSourceProbe.liveSession.configuration

        XCTAssertEqual(config.timeoutIntervalForRequest, Constants.chainProbeRequestTimeoutSecs)
        XCTAssertEqual(config.timeoutIntervalForResource, Constants.chainProbeResourceTimeoutSecs)
        XCTAssertEqual(Constants.chainProbeRequestTimeoutSecs, 2)
        XCTAssertEqual(Constants.chainProbeResourceTimeoutSecs, 3)
    }

    // MARK: - Retry scheduling

    /// A retry run with fakes: `shouldRun` gates it, `probe` returns `url` after `probe` runs, and
    /// `start` reports `ran` (false = turned away).
    private func run(
        shouldRun: @escaping @MainActor () -> Bool = { true },
        probe: @escaping @MainActor () async -> Void = {},
        start: @escaping @MainActor (String) async -> Void = { _ in },
        ran: Bool = true
    ) -> NodeStartupCoordinator.RetryRun {
        NodeStartupCoordinator.RetryRun(
            shouldRun: shouldRun,
            probe: {
                await probe()
                return self.fallback
            },
            start: { url in
                await start(url)
                return ran
            }
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
        XCTAssertEqual(coordinator.retryAttempt, 0, "A retry its gate turns away spends no attempt")
    }

    /// A retry whose start is turned away (another start or retry already in flight) spends no
    /// attempt, so the next one keeps the same backoff.
    func testScheduleRetry_startTurnedAwaySpendsNoAttempt() async {
        let coordinator = makeCoordinator()
        let started = expectation(description: "start asked")

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in
            started.fulfill()
        }, ran: false))
        await fulfillment(of: [started], timeout: 1)
        await waitUntilIdle(coordinator)

        XCTAssertEqual(coordinator.retryAttempt, 0)
        XCTAssertTrue(coordinator.isRetrying)
    }

    /// Stopping or wiping the node while the app is backgrounded must not lift the pause: only
    /// foreground does, so no start runs LDK in the suspended app.
    func testResetWhilePaused_stillRefusesAStartUntilForeground() async throws {
        let coordinator = makeCoordinator()
        coordinator.pause()

        coordinator.reset()

        XCTAssertTrue(coordinator.isPaused)
        do {
            _ = try await start(coordinator, results: [nil])
            XCTFail("A start after reset while paused must end as cancelled")
        } catch {
            XCTAssertTrue(error is CancellationError, "\(error)")
        }
        XCTAssertEqual(startedURLs, [], "No LDK start while the app is suspended")
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

        XCTAssertEqual(coordinator.retryAttempt, 0, "Only a retry that runs is counted")
        XCTAssertTrue(coordinator.isRetrying)
        XCTAssertEqual(eventNames(), ["NODE_START_RETRY_SCHEDULED"])
        coordinator.reset()
    }

    /// A start asked for while the app is already backgrounded (the paused launch's follow-up retry,
    /// a network-online callback) must not run LDK or take the wallet dir; foreground lets it through.
    func testStartWhilePaused_neverStartsTheNodeUntilForeground() async throws {
        let coordinator = makeCoordinator()
        coordinator.pause()

        do {
            _ = try await coordinator.startWithFailover(
                preferred: primary,
                providers: [primary, fallback],
                retryDelay: delay,
                isRetryable: { _ in true }
            ) { self.startedURLs.append($0) }
            XCTFail("A start while paused must end as cancelled")
        } catch {
            XCTAssertTrue(error is CancellationError, "\(error)")
        }
        XCTAssertEqual(startedURLs, [], "No LDK start while the app is suspended")

        coordinator.resume()
        let url = try await coordinator.startWithFailover(
            preferred: primary,
            providers: [primary, fallback],
            retryDelay: delay,
            isRetryable: { _ in true }
        ) { self.startedURLs.append($0) }
        XCTAssertEqual(url, primary)
        XCTAssertEqual(startedURLs, [primary])
    }

    func testPause_keepsBackoffAndReset_clearsIt() {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run())

        coordinator.pause()
        XCTAssertTrue(coordinator.isRetrying)
        coordinator.resumeIfIdle(maxDelay: maxDelay, reason: "resumed", run: run())
        XCTAssertEqual(coordinator.retryAttempt, 0, "A retry that never ran spends no attempt")
        XCTAssertEqual(events.compactMap { $0.data["delay_s"] as? String }, ["1", "1"])

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
        XCTAssertEqual(coordinator.retryAttempt, 0, "The interrupted retry runs again on foreground")
    }

    /// Repeated background and foreground while a retry waits must not spend the retry budget,
    /// and once retries run they keep the normal 1s, 2s, ... schedule.
    func testPauseAndResumeWhilePending_spendsNoAttemptUntilARetryRuns() async {
        var holdRetries = true
        let coordinator = makeCoordinator(sleep: { _ in
            if holdRetries { try await Task.sleep(for: .seconds(60)) }
        })
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
        for _ in 0 ..< 10 {
            coordinator.pause()
            coordinator.resumeIfIdle(maxDelay: maxDelay, reason: "resumed on foreground", run: retryRun())
        }
        XCTAssertEqual(coordinator.retryAttempt, 0)
        XCTAssertTrue(coordinator.isRetrying)

        holdRetries = false
        coordinator.pause()
        coordinator.resumeIfIdle(maxDelay: maxDelay, reason: "resumed on foreground", run: retryRun())
        await fulfillment(of: [second], timeout: 1)

        XCTAssertEqual(coordinator.retryAttempt, 2)
        let delays = events.compactMap { $0.data["delay_s"] as? String }
        XCTAssertEqual(delays, Array(repeating: "1", count: 12) + ["2"])
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

    /// Background during the wallet sync that follows a successful retry must neither cancel that
    /// sync nor refund an attempt, so a later failure still gets exactly the full retry budget.
    func testPauseAfterSuccessfulStartInsideARetry_keepsSyncAndFullBudget() async {
        let coordinator = makeCoordinator()
        let finished = expectation(description: "retry finished")
        var cancelledAfterPause = true

        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in
            _ = try? await coordinator.start(self.plan, isRetryable: { _ in true }) { _ in }
            // Stands in for backgrounding while the post-start wallet sync runs.
            coordinator.pause()
            cancelledAfterPause = Task.isCancelled
            finished.fulfill()
        }))
        await fulfillment(of: [finished], timeout: 1)
        await waitUntilIdle(coordinator)

        XCTAssertFalse(cancelledAfterPause, "The post-start sync must not be cancelled")
        XCTAssertEqual(coordinator.retryAttempt, 0)

        coordinator.resume()
        let exhausted = expectation(description: "retry budget spent")
        var failedRuns = 0
        func failingRun() -> NodeStartupCoordinator.RetryRun {
            run(start: { _ in
                failedRuns += 1
                let schedule = coordinator.scheduleRetry(
                    maxDelay: self.maxDelay,
                    reason: "stall again",
                    run: failingRun()
                )
                if schedule == .exhausted { exhausted.fulfill() }
            })
        }
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: failingRun())
        await fulfillment(of: [exhausted], timeout: 1)

        XCTAssertEqual(failedRuns, Constants.nodeStartMaxRetries)
        XCTAssertFalse(coordinator.isRetrying, "Exhaustion resets the backoff")
        XCTAssertEqual(eventNames().last, "NODE_START_RETRIES_EXHAUSTED")
    }

    func testRescheduleIfResumed_waitsForForegroundWhilePaused() {
        let coordinator = makeCoordinator(sleep: { _ in try await Task.sleep(for: .seconds(60)) })

        coordinator.pause()
        XCTAssertNil(coordinator.rescheduleIfResumed(maxDelay: maxDelay, reason: "paused", run: run()))
        XCTAssertFalse(coordinator.isRetrying)

        coordinator.resume()
        XCTAssertEqual(coordinator.rescheduleIfResumed(maxDelay: maxDelay, reason: "resumed", run: run()), .scheduled)
        XCTAssertTrue(coordinator.isRetrying)
        coordinator.reset()
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
        XCTAssertEqual(coordinator.retryAttempt, 1, "Only the retry that ran is counted")
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

    func testCancelledRetry_doesNotRun() async {
        let coordinator = makeCoordinator()
        var started = false

        // The retry task cannot run before this test yields, so it wakes already cancelled.
        coordinator.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(start: { _ in started = true }))
        coordinator.reset()
        for _ in 0 ..< 100 where sleeps.isEmpty {
            await Task.yield()
        }
        await Task.yield()

        XCTAssertEqual(sleeps, [.seconds(1)], "The retry woke up")
        XCTAssertFalse(started)
    }

    // MARK: - App wiring

    /// A coordinator for AppState tests: the in-start failover pause passes at once, while the
    /// retry backoff waits until reset cancels it, so no real retry runs.
    private func heldRetries() -> NodeStartupCoordinator {
        makeCoordinator(sleep: { duration in
            guard duration != Constants.nodeStartRetryDelay else { return }
            try await Task.sleep(for: .seconds(60))
        })
    }

    /// A recovered start reloads the books the NSE wrote while the app was suspended, so
    /// stability work never resumes on the launch-time snapshot.
    func testReloadAfterSuspend_picksUpBooksTheNSEWrote() throws {
        let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let appState = AppState()
        let db = try DatabaseService(dataDir: dataDir)
        appState.databaseService = db
        appState.stableChannel.userChannelId = "user-channel"
        appState.stableChannel.expectedUSD = USD(amount: 100)

        // The NSE settles a stability payment while the app is suspended.
        try db.channelRepo.saveChannel(
            channelId: "channel",
            userChannelId: "user-channel",
            expectedUSD: 150,
            backingSats: 150_000,
            note: nil
        )
        appState.reloadStateWrittenWhileSuspended()

        XCTAssertEqual(appState.stableChannel.expectedUSD.amount, 150)
        XCTAssertEqual(appState.stableChannel.backingSats, 150_000)
    }

    /// AppState opens the wallet database through the injected factory, and a failed open ends the
    /// start before any node method runs.
    func testStart_opensTheDatabaseThroughTheInjectedFactory() async {
        NodeDirLock.shared.release()
        var openedDirs: [URL] = []
        let node = FakeNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries(),
            chainSourceProbe: ChainSourceProbe(isReachable: { _ in true }, log: { _, _ in }),
            makeDatabase: { dir in
                openedDirs.append(dir)
                throw TestError.database
            }
        )

        await appState.start()

        XCTAssertEqual(openedDirs, [node.dataDir], "The database opens in the node service's wallet folder")
        XCTAssertEqual(appState.phase, .error("Database init failed: \(TestError.database.localizedDescription)"))
        XCTAssertNil(node.databaseService)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "A failed database open hands the wallet dir back")
    }

    /// With a working database, AppState starts the injected node once on the probed provider and
    /// reaches the wallet with no LDK node behind it.
    func testStart_withAWorkingDatabase_startsTheFakeNodeAndReachesTheWallet() async throws {
        NodeDirLock.shared.release()
        let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        // The started wallet writes a heartbeat to the app group; put the old one back.
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        let lastActive = shared?.object(forKey: Constants.SharedDefaultsKey.mainAppLastActive)
        defer { shared?.set(lastActive, forKey: Constants.SharedDefaultsKey.mainAppLastActive) }
        let db = try DatabaseService(dataDir: dataDir)
        let node = FakeNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries(),
            chainSourceProbe: ChainSourceProbe(isReachable: { _ in true }, log: { _, _ in }),
            makeDatabase: { _ in db }
        )
        defer { appState.stop() }

        await appState.start()

        XCTAssertEqual(node.startCalls, [
            FakeNodeService.StartCall(
                network: .bitcoin,
                esploraURL: Constants.primaryChainURL,
                mnemonic: "",
                lspConfig: appState.activeLSP
            )
        ])
        XCTAssertEqual(appState.phase, .wallet)
        XCTAssertFalse(appState.isSyncing)
        XCTAssertTrue(appState.hasCompletedInitialSync)
        XCTAssertTrue(node.databaseService === db)
    }

    /// A scheduled retry runs `retryConnection` with the syncing screen up; when that start fails
    /// again it must schedule the next retry instead of leaving the syncing screen up for good.
    func testFailedStartInsideARetry_schedulesTheNextRetry() async {
        let appState = AppState(
            nodeService: StallingNodeService(),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .syncing

        await appState.retryConnection()

        XCTAssertEqual(appState.phase, .syncing)
        XCTAssertTrue(appState.nodeStartup.isRetrying, "The failed retry must schedule the next one")
        appState.nodeStartup.reset()
    }

    /// Node start walks the current provider, every other configured provider, then the current
    /// one again after the start retry delay; a total failure keeps the current provider.
    func testAppStateStart_walksConfiguredProvidersInOrder() async {
        let node = FeeStallingNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .syncing

        await appState.retryConnection()

        XCTAssertEqual(
            node.startedURLs,
            [Constants.primaryChainURL, Constants.fallbackChainURL, Constants.primaryChainURL]
        )
        XCTAssertEqual(sleeps.first, Constants.nodeStartRetryDelay)
        XCTAssertEqual(appState.chainURL, Constants.primaryChainURL)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "A total failure hands the wallet dir back")
        appState.nodeStartup.reset()
    }

    /// A retry cancelled while waiting for the wallet dir reports the pause, not a locked dir,
    /// starts no node and leaves the lock free.
    func testCancelledLockWait_endsPausedWithoutTheLock() async throws {
        NodeDirLock.shared.release()
        try FileManager.default.createDirectory(at: Constants.userDataDir, withIntermediateDirectories: true)
        let lockPath = Constants.userDataDir.appendingPathComponent(NodeDirLock.lockFilename).path
        let otherHolder = open(lockPath, O_CREAT | O_RDWR, 0o644)
        XCTAssertGreaterThanOrEqual(otherHolder, 0)
        defer { close(otherHolder) }
        XCTAssertEqual(flock(otherHolder, LOCK_EX | LOCK_NB), 0, "Another holder takes the wallet dir")
        let node = FeeStallingNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .syncing

        let retry = Task { await appState.retryConnection() }
        retry.cancel()
        await retry.value

        XCTAssertTrue(node.startedURLs.isEmpty)
        XCTAssertFalse(NodeDirLock.shared.isHeld)
        XCTAssertTrue(
            events
                .contains {
                    $0.name == "NODE_START_RETRY_SCHEDULED" && $0.data["error"] as? String == "resumed after pause"
                },
            "The cancelled wait must take the pause path"
        )
        appState.nodeStartup.reset()
    }

    /// A start that fails over adopts the provider that worked and records it in the app group,
    /// where the notification extension reads it to start from a known-working provider.
    func testAppStateFailover_adoptsAndRecordsTheWorkingProvider() async {
        let node = PrimaryStallingNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .syncing

        await appState.retryConnection()

        XCTAssertEqual(node.startedURLs, [Constants.primaryChainURL, Constants.fallbackChainURL])
        XCTAssertEqual(appState.chainURL, Constants.fallbackChainURL)
        XCTAssertEqual(chainURLStore.load(), Constants.fallbackChainURL)
        appState.nodeStartup.reset()
    }

    /// A scheduled retry holds the wallet-dir lock across its whole failover, so a failed primary
    /// attempt never frees the dir for the notification extension before the fallback starts.
    func testRetryFailover_holdsTheWalletDirLockAcrossAttempts() async {
        NodeDirLock.shared.release()
        let node = LeaseMirroringNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .syncing

        await appState.retryConnection()

        XCTAssertEqual(node.startedURLs, [Constants.primaryChainURL, Constants.fallbackChainURL])
        XCTAssertEqual(node.lockHeldAtAttemptStart, [true, true], "Every attempt must run under one held lease")
        XCTAssertTrue(NodeDirLock.shared.isHeld, "The started node keeps the wallet dir")
        appState.nodeStartup.reset()
    }

    /// Background stop during the delay between failover attempts hands the wallet dir to the NSE at
    /// once (a suspended app never wakes to release it), and the failover starts no further node.
    func testBackgroundStopBetweenFailoverAttempts_releasesTheLockAndStartsNoFurtherAttempt() async {
        NodeDirLock.shared.release()
        let node = FeeStallingNodeService()
        var appState: AppState?
        var lockHeldAfterStop: Bool?
        let coordinator = makeCoordinator(sleep: { duration in
            guard duration == Constants.nodeStartRetryDelay else { return }
            appState?.performBackgroundStop()
            lockHeldAfterStop = NodeDirLock.shared.isHeld
        })
        appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: coordinator
        )
        appState?.phase = .syncing

        await appState?.retryConnection()

        XCTAssertEqual(lockHeldAfterStop, false, "Background stop must hand the wallet dir back")
        XCTAssertEqual(
            node.startedURLs,
            [Constants.primaryChainURL, Constants.fallbackChainURL],
            "No attempt may start after background stop"
        )
        XCTAssertTrue(coordinator.isPaused)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "The stopped failover hands the wallet dir back")
        coordinator.reset()
    }

    /// A restore that finds startup paused once it holds the wallet dir touches no wallet data,
    /// hands the dir back and reports the interruption.
    func testRestoreWhilePaused_deletesNothingAndThrowsInterrupted() async throws {
        keepInstalledAppUntouched()
        NodeDirLock.shared.release()
        let walletDir = try makeWalletDir(seedPhrase: Self.existingWords)
        let appState = AppState(
            nodeService: FeeStallingNodeService(dataDir: walletDir),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .wallet
        appState.nodeStartup.pause()

        await assertRestoreInterrupted(appState)

        XCTAssertEqual(try seedPhrase(in: walletDir), Self.existingWords)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "The paused restore hands the wallet dir back")
        XCTAssertEqual(appState.phase, .wallet)
    }

    /// A restore paused after the wipe keeps the restored words on disk, so the next start opens the
    /// restored wallet instead of creating a new empty one, and reports the interruption.
    func testRestorePausedAfterTheWipe_keepsTheRestoredSeedForTheNextStart() async throws {
        keepInstalledAppUntouched()
        NodeDirLock.shared.release()
        let walletDir = try makeWalletDir(seedPhrase: Self.existingWords)
        var appState: AppState?
        let coordinator = makeCoordinator(sleep: { duration in
            guard duration == Constants.nodeStartRetryDelay else { return }
            appState?.performBackgroundStop()
        })
        appState = AppState(
            nodeService: FeeStallingNodeService(dataDir: walletDir),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: coordinator
        )
        appState?.phase = .wallet

        if let appState { await assertRestoreInterrupted(appState) }

        XCTAssertEqual(try seedPhrase(in: walletDir), Self.restoredWords)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "No node runs, so the wallet dir is handed back")
        XCTAssertTrue(coordinator.isPaused, "Foreground, not the failed restore, ends the pause")
    }

    /// Real words with a wrong checksum, even after "Continue Anyway", are rejected before the restore
    /// wipes or writes anything, so the wallet stays as it was.
    func testRestoreWithABadChecksum_leavesTheWalletUntouchedAndThrowsInvalidMnemonic() async throws {
        keepInstalledAppUntouched()
        NodeDirLock.shared.release()
        let walletDir = try makeWalletDir(seedPhrase: Self.existingWords)
        let walletBefore = try walletFiles(in: walletDir)
        var appState: AppState?
        let coordinator = makeCoordinator(sleep: { duration in
            guard duration == Constants.nodeStartRetryDelay else { return }
            appState?.performBackgroundStop()
        })
        appState = AppState(
            nodeService: FeeStallingNodeService(dataDir: walletDir),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: coordinator
        )
        appState?.phase = .wallet

        do {
            try await appState?.restoreWalletFromMnemonic(Self.badChecksumWords, acknowledgeForceClose: true)
            XCTFail("A phrase with a bad checksum must not restore")
        } catch AppState.WalletRestoreError.invalidMnemonic {
        } catch {
            XCTFail("Expected .invalidMnemonic, got \(error)")
        }

        XCTAssertEqual(try walletFiles(in: walletDir), walletBefore, "The wallet folder must be untouched")
        XCTAssertEqual(appState?.phase, .wallet)
        coordinator.reset()
    }

    func testBIP39Check_acceptsValidPhrasesAndRejectsBadChecksumsCountsAndWords() {
        let abandon = { (count: Int) in Array(repeating: "abandon", count: count).joined(separator: " ") }
        XCTAssertTrue(MnemonicUtils.isValidBIP39(Self.restoredWords))
        XCTAssertTrue(MnemonicUtils.isValidBIP39(Self.existingWords))
        XCTAssertTrue(MnemonicUtils.isValidBIP39(abandon(23) + " art"))
        XCTAssertFalse(MnemonicUtils.isValidBIP39(Self.badChecksumWords), "Bad checksum")
        XCTAssertFalse(MnemonicUtils.isValidBIP39(abandon(23) + " about"), "Bad 24-word checksum")
        XCTAssertFalse(MnemonicUtils.isValidBIP39(abandon(14) + " address"), "Valid BIP39, but not 12 or 24 words")
        XCTAssertFalse(MnemonicUtils.isValidBIP39(abandon(11)), "Wrong word count")
        XCTAssertFalse(MnemonicUtils.isValidBIP39(abandon(11) + " bitcoin"), "Not a BIP39 word")
        XCTAssertTrue(MnemonicUtils.isValidBIP39(Self.restoredWords.replacingOccurrences(of: " ", with: "\n")))
        XCTAssertTrue(MnemonicUtils.isValidBIP39(Self.restoredWords.replacingOccurrences(of: " ", with: " \t  ")))
    }

    /// A seed_phrase saved with a bad checksum fails start with an error instead of LDKNode aborting the
    /// app, and leaves the wallet folder exactly as it was.
    func testStartWithASavedBadChecksumPhrase_throwsAndLeavesTheWalletUntouched() async throws {
        NodeDirLock.shared.release()
        let walletDir = try makeWalletDir(seedPhrase: Self.badChecksumWords)
        let walletBefore = try walletFiles(in: walletDir)
        let service = NodeService(dataDir: walletDir)

        do {
            try await service.start(network: .signet, esploraURL: primary, mnemonic: "")
            XCTFail("A saved phrase with a bad checksum must not start")
        } catch NodeServiceError.invalidSeedPhrase {
        } catch {
            XCTFail("Expected .invalidSeedPhrase, got \(error)")
        }

        // Only the app's own wallet-dir lock, taken before the phrase is read, may appear.
        var walletAfter = try walletFiles(in: walletDir)
        walletAfter[NodeDirLock.lockFilename] = nil
        XCTAssertEqual(walletAfter, walletBefore, "The wallet folder must be untouched")
        XCTAssertNil(service.node, "No LDK node may be built")
        XCTAssertFalse(service.isRunning)
    }

    private static let existingWords = Array(repeating: "zoo", count: 11).joined(separator: " ") + " wrong"
    private static let badChecksumWords = Array(repeating: "abandon", count: 12).joined(separator: " ")
    private static let restoredWords = Array(repeating: "abandon", count: 11).joined(separator: " ") + " about"

    private func assertRestoreInterrupted(_ appState: AppState) async {
        do {
            try await appState.restoreWalletFromMnemonic(Self.restoredWords, acknowledgeForceClose: true)
            XCTFail("A paused restore must not report success")
        } catch AppState.WalletRestoreError.interrupted {
        } catch {
            XCTFail("Expected .interrupted, got \(error)")
        }
    }

    /// A restore clears saved settings the installed app keeps in the app group and standard defaults, and must
    /// not touch its wallet files: after the test, put those settings back exactly and check the files.
    private func keepInstalledAppUntouched() {
        let installedWallet = installedWalletFiles()
        let group = Constants.appGroupIdentifier
        let stores = [(UserDefaults(suiteName: group), group), (.standard, Bundle.main.bundleIdentifier ?? "")]
        let saved = stores.map { $0.0?.persistentDomain(forName: $0.1) ?? [:] }
        addTeardownBlock { [self] in
            for ((store, domain), settings) in zip(stores, saved) {
                guard let defaults = store else { continue }
                let added = (defaults.persistentDomain(forName: domain) ?? [:]).keys.filter { settings[$0] == nil }
                added.forEach(defaults.removeObject(forKey:))
                settings.forEach { defaults.set($1, forKey: $0) }
            }
            XCTAssertEqual(installedWalletFiles(), installedWallet, "The installed app's wallet must be untouched")
        }
    }

    /// Size and modification date of each file a restore wipes or writes in the installed app's wallet
    /// folder; read-only, so it is safe to run on a device with a real wallet.
    private func installedWalletFiles() -> [String: String] {
        let names = ["keys_seed", "seed_phrase", "ldk_node_data.sqlite", DatabaseService.dbFilename]
            .flatMap { [$0, "\($0)-wal", "\($0)-shm"] }
        return names.reduce(into: [:]) { files, name in
            let url = Constants.userDataDir.appendingPathComponent(name)
            guard let values = try? url.resourceValues(forKeys: [.fileSizeKey, .contentModificationDateKey]) else {
                return
            }
            files[name] = "\(values.fileSize ?? 0) \(values.contentModificationDate?.timeIntervalSince1970 ?? 0)"
        }
    }

    /// A fresh wallet folder holding `seedPhrase`, removed after the test.
    private func makeWalletDir(seedPhrase: String) throws -> URL {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("wallet-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        try seedPhrase.write(to: dir.appendingPathComponent("seed_phrase"), atomically: true, encoding: .utf8)
        return dir
    }

    /// A node that stalls on fee estimation, on its own wallet folder: its start opens the wallet database
    /// and backfills prices into it, so it must never use the installed one.
    private func makeStallingNodeOnTempWallet() throws -> FeeStallingNodeService {
        try FeeStallingNodeService(dataDir: makeWalletDir(seedPhrase: Self.restoredWords))
    }

    /// Name and contents of each file in a wallet folder.
    private func walletFiles(in walletDir: URL) throws -> [String: Data] {
        try FileManager.default.contentsOfDirectory(atPath: walletDir.path).reduce(into: [:]) { files, name in
            files[name] = try Data(contentsOf: walletDir.appendingPathComponent(name))
        }
    }

    private func seedPhrase(in walletDir: URL) throws -> String {
        try String(contentsOf: walletDir.appendingPathComponent("seed_phrase"), encoding: .utf8)
    }

    /// LDK's start can't be cancelled: a start that finishes after background stop ran stops its
    /// node, hands the wallet dir back and ends paused for foreground to resume.
    func testStartFinishingAfterThePause_stopsItsNodeAndReleasesTheLock() async {
        NodeDirLock.shared.release()
        let node = LatePausedNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        node.whileStarting = { appState.performBackgroundStop() }
        appState.phase = .syncing

        await appState.retryConnection()

        XCTAssertEqual(node.stopCount, 1, "The node that came up after the pause must be stopped")
        XCTAssertFalse(node.isRunning)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "The stopped node hands the wallet dir back")
        XCTAssertTrue(appState.nodeStartup.isPaused)
        appState.nodeStartup.reset()
    }

    func testStartFinishingAfterForeground_keepsItsNode() async {
        NodeDirLock.shared.release()
        let node = LatePausedNodeService()
        let appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        node.whileStarting = {
            appState.nodeStartup.pause()
            appState.nodeStartup.resume()
        }
        appState.phase = .syncing

        await appState.retryConnection()

        XCTAssertEqual(node.stopCount, 0, "A start that finishes after foreground must keep the node")
        XCTAssertTrue(NodeDirLock.shared.isHeld)
        appState.nodeStartup.reset()
    }

    func testForegroundInAnyPhase_endsThePause() async {
        let appState = AppState()
        appState.phase = .offline
        appState.nodeStartup.pause()

        await appState.restartNodeFromForeground()

        XCTAssertFalse(appState.nodeStartup.isPaused, "A later retry must keep its node in foreground")
    }

    func testForegroundWhileRetrying_disarmsTheBackgroundStop() async {
        let appState = AppState(nodeStartup: heldRetries())
        appState.phase = .syncing
        appState.nodeStartup.scheduleRetry(maxDelay: maxDelay, reason: "stall", run: run(shouldRun: { false }))
        appState.stopNodeForBackground()
        XCTAssertTrue(appState.hasPendingBackgroundStop)

        await appState.restartNodeFromForeground()

        XCTAssertFalse(appState.hasPendingBackgroundStop, "Foreground must disarm the pending background stop")
        XCTAssertTrue(appState.nodeStartup.isRetrying)
        appState.nodeStartup.reset()
    }

    func testForegroundInOfflinePhase_disarmsTheBackgroundStop() async {
        let appState = AppState(nodeStartup: heldRetries())
        appState.phase = .offline
        appState.stopNodeForBackground()
        XCTAssertTrue(appState.hasPendingBackgroundStop)

        await appState.restartNodeFromForeground()

        XCTAssertFalse(
            appState.hasPendingBackgroundStop,
            "Foreground in offline phase must disarm the pending background stop"
        )
    }

    func testPausedOfflineRetry_resumesAutomaticallyOnForeground() async {
        let node = FeeStallingNodeService()
        var appState: AppState?
        let coordinator = makeCoordinator(sleep: { duration in
            guard duration == Constants.nodeStartRetryDelay else { return }
            appState?.performBackgroundStop()
        })
        appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: coordinator
        )
        appState?.phase = .offline

        await appState?.retryConnection()

        XCTAssertTrue(node.startedURLs.count <= 2, "Only primary and fallback tried before background stop")
        XCTAssertEqual(appState?.phase, .offline, "Phase remains offline while paused in background")

        let attemptsBeforeForeground = node.startedURLs.count

        // Returning to foreground must automatically resume the interrupted recovery
        await appState?.restartNodeFromForeground()

        XCTAssertGreaterThan(
            node.startedURLs.count,
            attemptsBeforeForeground,
            "Foreground must resume the interrupted start"
        )
        coordinator.reset()
    }

    /// A launch that background paused between provider attempts hands the wallet dir back,
    /// so the suspended app never keeps it from the notification extension.
    func testLaunchPausedBetweenAttempts_releasesTheWalletDir() async throws {
        NodeDirLock.shared.release()
        let node = try makeStallingNodeOnTempWallet()
        var appState: AppState?
        let coordinator = makeCoordinator(sleep: { duration in
            guard duration == Constants.nodeStartRetryDelay else { return }
            appState?.performBackgroundStop()
        })
        appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: coordinator
        )

        await appState?.start()

        XCTAssertTrue(coordinator.isPaused)
        XCTAssertFalse(NodeDirLock.shared.isHeld, "No node runs, so the paused launch hands the wallet dir back")
        coordinator.reset()
    }

    /// A foreground restart picks up the funding txid the notification extension saved while the
    /// app was suspended, like every other recovered start.
    func testForegroundRestart_reloadsTheFundingTxidSavedWhileSuspended() async {
        NodeDirLock.shared.release()
        let shared = UserDefaults(suiteName: Constants.appGroupIdentifier)
        let saved = shared?.string(forKey: "funding_txid")
        defer { shared?.set(saved, forKey: "funding_txid") }
        let appState = AppState(
            nodeService: LatePausedNodeService(),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.phase = .wallet
        shared?.set("nse-funding-txid", forKey: "funding_txid")

        await appState.restartNodeFromForeground()

        XCTAssertEqual(appState.fundingTxid, "nse-funding-txid")
        appState.nodeStartup.reset()
    }

    /// The notification extension can still hold the wallet dir and settle a stability payment
    /// while a foreground restart waits for it; the restart must reload the books after taking
    /// the lock, so its save keeps that write instead of the pre-lock snapshot.
    func testForegroundRestart_keepsBooksTheNSEWroteWhileWaitingForTheLock() async throws {
        NodeDirLock.shared.release()
        let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let db = try DatabaseService(dataDir: dataDir)
        try db.channelRepo.saveChannel(
            channelId: "channel",
            userChannelId: "user-channel",
            expectedUSD: 100,
            backingSats: 100_000,
            note: nil
        )
        try FileManager.default.createDirectory(at: Constants.userDataDir, withIntermediateDirectories: true)
        let lockPath = Constants.userDataDir.appendingPathComponent(NodeDirLock.lockFilename).path
        let extensionHolder = open(lockPath, O_CREAT | O_RDWR, 0o644)
        XCTAssertGreaterThanOrEqual(extensionHolder, 0)
        defer { close(extensionHolder) }
        XCTAssertEqual(flock(extensionHolder, LOCK_EX | LOCK_NB), 0, "The extension holds the wallet dir")
        let appState = AppState(
            nodeService: LatePausedNodeService(),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.databaseService = db
        appState.stableChannel.userChannelId = "user-channel"
        appState.phase = .wallet

        let restart = Task { await appState.restartNodeFromForeground() }
        // Once the flow is in progress the restart is parked on the lock wait; then the extension
        // settles a payment and lets go.
        await waitUntilRestartIsWaitingForTheLock(appState)
        try db.channelRepo.saveChannel(
            channelId: "channel",
            userChannelId: "user-channel",
            expectedUSD: 150,
            backingSats: 150_000,
            note: nil
        )
        flock(extensionHolder, LOCK_UN)
        await restart.value

        let saved = try XCTUnwrap(db.channelRepo.loadChannel(userChannelId: "user-channel"))
        XCTAssertEqual(saved.expectedUSD, 150, "The restart must not overwrite the extension's write")
        XCTAssertEqual(saved.backingSats, 150_000)
        XCTAssertEqual(appState.stableChannel.expectedUSD.amount, 150)
        appState.nodeStartup.reset()
    }

    func testForegroundRestart_showsTheExtensionsBooksWhenTheLockWaitEnds() async throws {
        NodeDirLock.shared.release()
        let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dataDir) }
        let db = try DatabaseService(dataDir: dataDir)
        try db.channelRepo.saveChannel(
            channelId: "channel",
            userChannelId: "user-channel",
            expectedUSD: 100,
            backingSats: 100_000,
            note: nil
        )
        try FileManager.default.createDirectory(at: Constants.userDataDir, withIntermediateDirectories: true)
        let lockPath = Constants.userDataDir.appendingPathComponent(NodeDirLock.lockFilename).path
        let extensionHolder = open(lockPath, O_CREAT | O_RDWR, 0o644)
        XCTAssertGreaterThanOrEqual(extensionHolder, 0)
        defer {
            flock(extensionHolder, LOCK_UN)
            close(extensionHolder)
        }
        XCTAssertEqual(flock(extensionHolder, LOCK_EX | LOCK_NB), 0, "The extension holds the wallet dir")
        let appState = AppState(
            nodeService: LatePausedNodeService(),
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: heldRetries()
        )
        appState.databaseService = db
        appState.stableChannel.userChannelId = "user-channel"
        appState.phase = .wallet

        let restart = Task { await appState.restartNodeFromForeground() }
        await waitUntilRestartIsWaitingForTheLock(appState)
        try db.channelRepo.saveChannel(
            channelId: "channel",
            userChannelId: "user-channel",
            expectedUSD: 150,
            backingSats: 150_000,
            note: nil
        )
        // The wait ends without the lock (here cancelled; a timeout takes the same path).
        restart.cancel()
        await restart.value

        XCTAssertEqual(appState.stableChannel.expectedUSD.amount, 150, "The books on screen must not stay stale")
        XCTAssertEqual(appState.stableChannel.backingSats, 150_000)
        appState.nodeStartup.reset()
    }

    /// The restart sets its flow flag on the main actor and runs straight to the lock wait, so
    /// seeing the flag set means it is parked there.
    private func waitUntilRestartIsWaitingForTheLock(_ appState: AppState) async {
        while await !(MainActor.run { appState.isNodeFlowInProgress }) {
            await Task.yield()
        }
    }

    func testBackgroundStopDuringLaunch_doesNotProduceTerminalError() async throws {
        let node = try makeStallingNodeOnTempWallet()
        var appState: AppState?
        let coordinator = makeCoordinator(sleep: { duration in
            guard duration == Constants.nodeStartRetryDelay else { return }
            appState?.performBackgroundStop()
        })
        appState = AppState(
            nodeService: node,
            networkMonitor: MockNetworkMonitor(initialStatus: .online),
            nodeStartup: coordinator
        )

        await appState?.start()

        if case .error = appState?.phase {
            XCTFail("Must not enter terminal error on background pause")
        }
        coordinator.reset()
    }
}

/// A node whose start always fails with a transient wallet-sync error.
private class StallingNodeService: NodeService {
    private(set) var startedURLs: [String] = []

    override func start(
        network _: Network,
        esploraURL: String,
        mnemonic _: String,
        lspConfig _: LSPConfig
    ) async throws {
        startedURLs.append(esploraURL)
        if let error = stall(esploraURL) { throw error }
    }

    func stall(_: String) -> NodeError? { .TxSyncFailed(message: "sync stalled") }
}

/// A node whose start fails fee-rate estimation against every provider, which fails over.
private class FeeStallingNodeService: StallingNodeService {
    override func stall(_: String) -> NodeError? { .FeerateEstimationUpdateFailed(message: "stalled") }
}

/// A node whose start fails fee-rate estimation only against the primary provider.
private class PrimaryStallingNodeService: FeeStallingNodeService {
    override func stall(_ url: String) -> NodeError? { url == Constants.primaryChainURL ? super.stall(url) : nil }
}

/// A node whose start succeeds after `whileStarting` ran, holding the wallet dir like a real node.
private final class LatePausedNodeService: NodeService {
    var whileStarting: @MainActor () -> Void = {}
    private(set) var stopCount = 0
    private var started = false

    override var isRunning: Bool { started }

    override func start(
        network _: Network,
        esploraURL _: String,
        mnemonic _: String,
        lspConfig _: LSPConfig
    ) async throws {
        await whileStarting()
        _ = NodeDirLock.shared.tryAcquire(dataDir: Constants.userDataDir)
        started = true
    }

    override func stop() {
        stopCount += 1
        started = false
    }
}

/// Fails over like `PrimaryStallingNodeService` and takes the wallet-dir lease the way `NodeService.start`
/// does: a failed attempt releases it only when that attempt newly acquired it.
private final class LeaseMirroringNodeService: PrimaryStallingNodeService {
    private(set) var lockHeldAtAttemptStart: [Bool] = []

    override func start(network: Network, esploraURL: String, mnemonic: String, lspConfig: LSPConfig) async throws {
        lockHeldAtAttemptStart.append(NodeDirLock.shared.isHeld)
        let lease = NodeDirLock.shared.tryAcquireReportingNew(dataDir: Constants.userDataDir)
        do {
            try await super.start(network: network, esploraURL: esploraURL, mnemonic: mnemonic, lspConfig: lspConfig)
        } catch {
            if lease.newlyAcquired {
                NodeDirLock.shared.release()
            }
            throw error
        }
    }
}

extension AppState {
    /// The default node, for tests that drive its DEBUG overrides.
    var concreteNodeService: NodeService {
        guard let node = nodeService as? NodeService else { fatalError("AppState was built with a fake node") }
        return node
    }
}

/// A node service with no LDK node behind it: AppState must run on the protocol alone.
private final class FakeNodeService: AppNodeService {
    var node: Node? { nil }
    var isRunning = false
    var isStarting = false
    var nodeId = ""
    var activeNetwork: Network?
    var channels: [ChannelDetails] = []
    var savedMnemonic: String?
    var databaseService: DatabaseService?
    /// Never the installed app's wallet folder.
    let dataDir = FileManager.default.temporaryDirectory.appendingPathComponent("fake-node-\(UUID().uuidString)")

    func saveSeedPhrase(_: String) throws {}
    func wipeWalletData() {}

    struct StartCall: Equatable {
        let network: Network
        let esploraURL: String
        let mnemonic: String
        let lspConfig: LSPConfig
    }

    private(set) var startCalls: [StartCall] = []

    func start(network: Network, esploraURL: String, mnemonic: String, lspConfig: LSPConfig) async throws {
        startCalls.append(StartCall(network: network, esploraURL: esploraURL, mnemonic: mnemonic, lspConfig: lspConfig))
        isRunning = true
    }

    func stop() {}
    func syncWallets() throws {}
    func clearSavedMnemonic() {}
    func refreshChannels() {}
    func balances() -> BalanceDetails? { nil }
    func inboundOnchain() -> WalletInboundOnchain? { nil }
    func spendableOnchainSats() -> UInt64 { 0 }
    func lightningSyncAgeSecs() -> UInt64? { nil }
    func newOnchainAddress() throws -> String { throw NodeServiceError.notRunning }
    func signMessage(_: [UInt8]) throws -> String { throw NodeServiceError.notRunning }
    func verifySignature(message _: [UInt8], signature _: String, pubkey _: PublicKey) -> Bool { false }
    func connectAndOpenChannel(pubkey _: String, address _: String, amountSats _: UInt64) async throws {
        throw NodeServiceError.notRunning
    }

    func requestChannelClose(
        userChannelId _: UserChannelId,
        counterpartyNodeId _: PublicKey,
        fundingOutpointTxid _: String,
        fundingOutpointVout _: UInt32,
        balanceSats _: UInt64,
        balanceUsd _: Double?,
        btcPrice _: Double?,
        counterparty _: String?,
        force _: Bool
    ) async throws {
        throw NodeServiceError.notRunning
    }

    func spliceInWithAll(userChannelId _: UserChannelId, counterpartyNodeId _: PublicKey) throws {
        throw NodeServiceError.notRunning
    }

    func spliceOut(
        userChannelId _: UserChannelId,
        counterpartyNodeId _: PublicKey,
        address _: String,
        amountSats _: UInt64
    ) throws {
        throw NodeServiceError.notRunning
    }

    func outboundPaymentStatus(paymentHash _: PaymentHash) -> PaymentStatus? { nil }
    func sendPayment(invoice _: Bolt11Invoice) throws -> PaymentId { throw NodeServiceError.notRunning }
    func sendPaymentUsingAmount(invoice _: Bolt11Invoice, amountMsat _: UInt64) throws -> PaymentId {
        throw NodeServiceError.notRunning
    }

    func sendBolt12UsingAmount(offer _: Offer, amountMsat _: UInt64) throws -> PaymentId {
        throw NodeServiceError.notRunning
    }

    func sendKeysendWithTLV(amountMsat _: UInt64, to _: PublicKey, tlvs _: [CustomTlvRecord]) throws -> PaymentId {
        throw NodeServiceError.notRunning
    }

    func sendStabilityPayment(amountMsat _: UInt64, to _: PublicKey, tlvs _: [CustomTlvRecord]) throws -> PaymentId {
        throw NodeServiceError.notRunning
    }

    func sendOnchain(address _: String, amountSats _: UInt64, feeRateSatVb _: Double?) throws -> Txid {
        throw NodeServiceError.notRunning
    }

    func sendAllOnchain(address _: String, feeRateSatVb _: Double?) throws -> Txid { throw NodeServiceError.notRunning }
    func receivePayment(amountMsat _: UInt64, description _: String) throws -> Bolt11Invoice {
        throw NodeServiceError.notRunning
    }

    func receiveVariablePayment(description _: String) throws -> Bolt11Invoice { throw NodeServiceError.notRunning }
    func receiveViaJitChannel(amountMsat _: UInt64, description _: String) throws -> Bolt11Invoice {
        throw NodeServiceError.notRunning
    }
}
