@testable import StableChannels
import LDKNode
import XCTest

final class NodeStartupFailoverTests: XCTestCase {
    // MARK: - Error Classification Tests

    func testNonEsploraErrorsDoNotTriggerRetry() {
        let nonRetryableErrors: [Error] = [
            NodeServiceError.alreadyRunning,
            NodeServiceError.notRunning,
            NodeServiceError.dataDirLocked,
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
            NodeError.FeerateEstimationUpdateTimeout(message: "Connection timed out"),
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

        for error in retryableErrors {
            XCTAssertTrue(
                error.isRetryableEsploraStartupError,
                "Feerate error '\(error)' must be classified as retryable"
            )
        }
    }

    // MARK: - NodeDirLock Lease Retention Tests

    func testNodeDirLockContinuousHoldAcrossFailoverAttempts() async {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("NodeStartupLockTest-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer {
            NodeDirLock.shared.forceRelease()
            try? FileManager.default.removeItem(at: tempDir)
        }

        // 1. Outer caller (AppState.startWallet) acquires lock
        let acquiredByApp = await NodeDirLock.shared.acquire(dataDir: tempDir, timeout: 5)
        XCTAssertTrue(acquiredByApp, "AppState must acquire the directory lock")
        XCTAssertTrue(NodeDirLock.shared.isHeld, "Lock must be held after initial acquisition")

        // 2. Inner attempt 1 (primary NodeService.start) takes an inner lease
        let acquiredByPrimary = NodeDirLock.shared.tryAcquire(dataDir: tempDir)
        XCTAssertTrue(acquiredByPrimary, "Inner attempt must succeed under existing lock lease")
        XCTAssertTrue(NodeDirLock.shared.isHeld, "Lock must remain held")

        // 3. Primary attempt fails and runs defer { release() }
        NodeDirLock.shared.release()

        // 4. VERIFY: Lock remains held exclusively by AppState during backoff!
        XCTAssertTrue(
            NodeDirLock.shared.isHeld,
            "Lock must NOT be dropped between primary failure and fallback attempt"
        )

        // 5. Inner attempt 2 (fallback NodeService.start) takes lease and succeeds
        let acquiredByFallback = NodeDirLock.shared.tryAcquire(dataDir: tempDir)
        XCTAssertTrue(acquiredByFallback, "Fallback attempt must succeed under existing lock lease")
        XCTAssertTrue(NodeDirLock.shared.isHeld, "Lock must remain held during fallback execution")

        // 6. When outer lifecycle finishes / cleans up
        NodeDirLock.shared.release()
        NodeDirLock.shared.release()
        XCTAssertFalse(NodeDirLock.shared.isHeld, "Lock must release when all leases are returned")
    }

    func testNodeDirLockForceRelease() async {
        let tempDir = FileManager.default.temporaryDirectory
            .appendingPathComponent("NodeStartupForceReleaseTest-\(UUID().uuidString)")
        try? FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defer {
            NodeDirLock.shared.forceRelease()
            try? FileManager.default.removeItem(at: tempDir)
        }

        _ = await NodeDirLock.shared.acquire(dataDir: tempDir, timeout: 5)
        _ = NodeDirLock.shared.tryAcquire(dataDir: tempDir)
        _ = NodeDirLock.shared.tryAcquire(dataDir: tempDir)
        XCTAssertTrue(NodeDirLock.shared.isHeld)

        NodeDirLock.shared.forceRelease()
        XCTAssertFalse(NodeDirLock.shared.isHeld, "forceRelease must immediately clear the lock")
    }

    // MARK: - Failover Simulation Runner Tests

    private struct SimulatedStartupCoordinator {
        var startHandler: (String) async throws -> Void

        func startWithFailover(initialURL: String, fallbackURL: String) async throws -> String {
            do {
                try await startHandler(initialURL)
                return initialURL
            } catch {
                guard error.isRetryableEsploraStartupError else {
                    // Immediate propagation of non-Esplora errors
                    throw error
                }

                let primaryError = error
                do {
                    try await startHandler(fallbackURL)
                    return fallbackURL
                } catch {
                    // Preserves original primary error on fallback failure
                    throw primaryError
                }
            }
        }
    }

    func testNonEsploraErrorPropagatesImmediatelyWithoutCallingFallback() async {
        var attempts: [String] = []
        let coordinator = SimulatedStartupCoordinator { url in
            attempts.append(url)
            throw NodeServiceError.alreadyRunning
        }

        do {
            _ = try await coordinator.startWithFailover(
                initialURL: "https://blockstream.info/api",
                fallbackURL: "https://mempool.space/api"
            )
            XCTFail("Should have thrown error")
        } catch {
            guard let serviceError = error as? NodeServiceError, serviceError == .alreadyRunning else {
                XCTFail("Expected NodeServiceError.alreadyRunning, got \(error)")
                return
            }
            XCTAssertEqual(
                attempts,
                ["https://blockstream.info/api"],
                "Must only attempt primary and not call fallback for non-Esplora errors"
            )
        }
    }

    func testFeerateErrorTriggersFallbackAndSucceeds() async throws {
        var attempts: [String] = []
        let coordinator = SimulatedStartupCoordinator { url in
            attempts.append(url)
            if url == "https://blockstream.info/api" {
                throw NodeError.FeerateEstimationUpdateFailed(message: "HTTP 429")
            }
        }

        let resolvedURL = try await coordinator.startWithFailover(
            initialURL: "https://blockstream.info/api",
            fallbackURL: "https://mempool.space/api"
        )

        XCTAssertEqual(resolvedURL, "https://mempool.space/api")
        XCTAssertEqual(
            attempts,
            ["https://blockstream.info/api", "https://mempool.space/api"],
            "Must attempt primary first, failover on feerate error, and succeed on secondary"
        )
    }

    func testFallbackFailurePreservesOriginalPrimaryError() async {
        var attempts: [String] = []
        let coordinator = SimulatedStartupCoordinator { url in
            attempts.append(url)
            if url == "https://blockstream.info/api" {
                throw NodeError.FeerateEstimationUpdateFailed(message: "Blockstream 429 Rate Limited")
            } else {
                throw NodeError.FeerateEstimationUpdateTimeout(message: "Mempool Connection Timeout")
            }
        }

        do {
            _ = try await coordinator.startWithFailover(
                initialURL: "https://blockstream.info/api",
                fallbackURL: "https://mempool.space/api"
            )
            XCTFail("Should have thrown error")
        } catch {
            if case let NodeError.FeerateEstimationUpdateFailed(msg) = error {
                XCTAssertEqual(
                    msg,
                    "Blockstream 429 Rate Limited",
                    "Must preserve the primary error when fallback also fails"
                )
            } else {
                XCTFail("Expected original FeerateEstimationUpdateFailed error, got \(error)")
            }
            XCTAssertEqual(
                attempts,
                ["https://blockstream.info/api", "https://mempool.space/api"]
            )
        }
    }
}
