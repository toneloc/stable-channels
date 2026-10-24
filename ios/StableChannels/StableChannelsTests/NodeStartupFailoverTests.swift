@testable import StableChannels
import LDKNode
import XCTest

final class NodeStartupFailoverTests: XCTestCase {
    // MARK: - Error Classification

    func testNonEsploraErrorsDoNotTriggerRetry() {
        let nonRetryableErrors: [Error] = [
            NodeServiceError.alreadyRunning,
            NodeServiceError.notRunning,
            NodeServiceError.dataDirLocked,
            NodeServiceError.invalidFeeRate,
            NodeError.PersistenceFailed(message: "Disk full"),
            NodeError.InvalidAddress(message: "Bad address"),
            NodeError.ChannelCreationFailed(message: "Channel open error"),
            NSError(domain: "DatabaseError", code: 1, userInfo: [NSLocalizedDescriptionKey: "SQLite corrupt"])
        ]

        for error in nonRetryableErrors {
            XCTAssertFalse(
                error.isRetryableEsploraStartupError,
                "Non-Esplora error '\(error)' must not be classified as retryable"
            )
        }
    }

    func testFeerateEstimationErrorsAreRetryable() {
        let retryableErrors: [Error] = [
            NodeError.FeerateEstimationUpdateFailed(message: "HTTP 429 Too Many Requests"),
            NodeError.FeerateEstimationUpdateTimeout(message: "Connection timed out")
        ]

        for error in retryableErrors {
            XCTAssertTrue(
                error.isRetryableEsploraStartupError,
                "Feerate error '\(error)' must be classified as retryable"
            )
        }
    }

    /// The reported failure: an online fee-rate timeout whose text says "timed out" must retry,
    /// not land on the offline page.
    func testOnlineFeeRateTimeout_retriesDespiteTimedOutText() {
        let error = NodeError.FeerateEstimationUpdateTimeout(message: "Connection timed out")
        let outcome = NodeStartupPolicy.failureOutcome(
            isNetworkOffline: false,
            looksOffline: NetworkReachabilityEvaluator.shouldPresentOfflineNotice(
                error: error,
                isNetworkOffline: false
            ),
            isRetryable: error.isRetryableNodeStartError
        )

        XCTAssertEqual(outcome, .retry)
    }

    func testBridgedErrorsCarryingCaseNameTextAreNotRetryable() {
        // Only typed NodeError cases qualify. A foreign error whose description happens to
        // contain the case name must not trigger a provider retry.
        let impostors: [Error] = [
            NSError(
                domain: "LDKNode",
                code: 100,
                userInfo: [NSLocalizedDescriptionKey: "FeerateEstimationUpdateFailed(HTTP 502)"]
            ),
            NSError(
                domain: "LDKNode",
                code: 101,
                userInfo: [NSLocalizedDescriptionKey: "FeerateEstimationUpdateTimeout(esplora)"]
            )
        ]

        for error in impostors {
            XCTAssertFalse(
                error.isRetryableEsploraStartupError,
                "Non-NodeError '\(error)' must not be classified as retryable"
            )
        }
    }

    // MARK: - NodeDirLock (binary lease with ownership reporting)

    private func makeTempDir() -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("NodeStartupLockTest-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir
    }

    /// Regression test for the wallet-lock leak: the normal app lifecycle is
    /// outer AppState acquire → inner NodeService.start acquire (retained on
    /// success) → ONE release when backgrounding. That single release must free
    /// the flock, or the NSE is locked out for the life of the suspended app.
    func testSuccessLifecycleSingleReleaseFreesLock() {
        let tempDir = makeTempDir()
        defer {
            NodeDirLock.shared.release()
            try? FileManager.default.removeItem(at: tempDir)
        }

        // Outer AppState lease
        let outer = NodeDirLock.shared.tryAcquireReportingNew(dataDir: tempDir)
        XCTAssertTrue(outer.held)
        XCTAssertTrue(outer.newlyAcquired, "First acquire must own the flock")

        // Inner NodeService.start lease — start succeeds, so it never releases
        let inner = NodeDirLock.shared.tryAcquireReportingNew(dataDir: tempDir)
        XCTAssertTrue(inner.held)
        XCTAssertFalse(inner.newlyAcquired, "Nested acquire must not claim ownership")

        // performBackgroundStop: exactly one release hands the dir to the NSE
        NodeDirLock.shared.release()
        XCTAssertFalse(
            NodeDirLock.shared.isHeld,
            "One release must free the lock — anything else starves the NSE while the app is suspended"
        )
    }

    /// The Esplora failover window: a failed primary NodeService.start must not
    /// release the flock when AppState holds the outer lease, so no other
    /// process can take the wallet dir between the two attempts.
    func testFailedInnerAttemptKeepsOuterLeaseAcrossFailover() {
        let tempDir = makeTempDir()
        defer {
            NodeDirLock.shared.release()
            try? FileManager.default.removeItem(at: tempDir)
        }

        // Outer AppState lease
        XCTAssertTrue(NodeDirLock.shared.tryAcquireReportingNew(dataDir: tempDir).held)

        // Primary attempt acquires without ownership, fails, and — per the
        // ownership rule — performs no release.
        let primary = NodeDirLock.shared.tryAcquireReportingNew(dataDir: tempDir)
        XCTAssertTrue(primary.held)
        XCTAssertFalse(primary.newlyAcquired)
        XCTAssertTrue(
            NodeDirLock.shared.isHeld,
            "Lock must survive a failed primary attempt while the outer lease exists"
        )

        // Fallback attempt runs under the same lease and succeeds.
        let fallback = NodeDirLock.shared.tryAcquireReportingNew(dataDir: tempDir)
        XCTAssertTrue(fallback.held)
        XCTAssertFalse(fallback.newlyAcquired)

        // Background stop: one release frees everything.
        NodeDirLock.shared.release()
        XCTAssertFalse(NodeDirLock.shared.isHeld)
    }

    /// A standalone start (no outer lease) owns the flock and must release it
    /// on failure so the NSE isn't blocked by a dead startup.
    func testStandaloneFailedStartReleasesOwnLease() {
        let tempDir = makeTempDir()
        defer {
            NodeDirLock.shared.release()
            try? FileManager.default.removeItem(at: tempDir)
        }

        let lease = NodeDirLock.shared.tryAcquireReportingNew(dataDir: tempDir)
        XCTAssertTrue(lease.held)
        XCTAssertTrue(lease.newlyAcquired, "Standalone start must own its lease")

        // Failure path: owner releases.
        NodeDirLock.shared.release()
        XCTAssertFalse(NodeDirLock.shared.isHeld)
    }

    func testReleaseWithoutHoldIsSafeNoOp() {
        let tempDir = makeTempDir()
        defer { try? FileManager.default.removeItem(at: tempDir) }

        NodeDirLock.shared.release()
        NodeDirLock.shared.release()
        XCTAssertFalse(NodeDirLock.shared.isHeld)

        XCTAssertTrue(NodeDirLock.shared.tryAcquire(dataDir: tempDir))
        NodeDirLock.shared.release()
        NodeDirLock.shared.release()
        XCTAssertFalse(NodeDirLock.shared.isHeld, "Extra releases must stay harmless no-ops")
    }

    /// A paused node-start retry may be waiting on the NSE's lock; cancellation must end the
    /// poll at once instead of spinning to the 35s deadline.
    func testCancelledLockWait_returnsNotHeldPromptly() async {
        let tempDir = makeTempDir()
        // Another process's flock: a separate open file description conflicts even in-process.
        let other = open(tempDir.appendingPathComponent(NodeDirLock.lockFilename).path, O_CREAT | O_RDWR, 0o644)
        XCTAssertEqual(flock(other, LOCK_EX | LOCK_NB), 0)
        defer {
            close(other)
            NodeDirLock.shared.release()
            try? FileManager.default.removeItem(at: tempDir)
        }

        let wait = Task { await NodeDirLock.shared.acquireReportingNew(dataDir: tempDir, timeout: 35) }
        try? await Task.sleep(for: .milliseconds(300))
        let cancelledAt = ContinuousClock.now
        wait.cancel()
        let lease = await wait.value

        XCTAssertFalse(lease.held)
        XCTAssertLessThan(ContinuousClock.now - cancelledAt, .seconds(1))
    }
}
