import XCTest
@testable import StableChannels

final class NodeStartupPolicyTests: XCTestCase {
    private let primary = "https://primary.example/api"
    private let fallback = "https://fallback.example/api"
    private let delay = Duration.seconds(2)
    private let maxDelay = Duration.seconds(60)

    private func attempt(_ url: String, _ delayBefore: Duration = .zero) -> NodeStartupPolicy.Attempt {
        NodeStartupPolicy.Attempt(url: url, delayBefore: delayBefore)
    }

    // MARK: - Attempts

    func testPreferredPrimary_triesFallbackThenPrimaryAgain() {
        let attempts = NodeStartupPolicy.attempts(preferred: primary, providers: [primary, fallback], retryDelay: delay)

        XCTAssertEqual(attempts, [attempt(primary), attempt(fallback), attempt(primary, delay)])
    }

    func testPreferredFallback_triesPrimaryThenFallbackAgain() {
        let attempts = NodeStartupPolicy.attempts(
            preferred: fallback,
            providers: [primary, fallback],
            retryDelay: delay
        )

        XCTAssertEqual(attempts, [attempt(fallback), attempt(primary), attempt(fallback, delay)])
    }

    func testDuplicateProviders_areTriedOnce() {
        let attempts = NodeStartupPolicy.attempts(
            preferred: primary,
            providers: [fallback, primary, fallback],
            retryDelay: delay
        )

        XCTAssertEqual(attempts, [attempt(primary), attempt(fallback), attempt(primary, delay)])
    }

    func testEmptyProviders_triesPreferredTwice() {
        let attempts = NodeStartupPolicy.attempts(preferred: primary, providers: [], retryDelay: delay)

        XCTAssertEqual(attempts, [attempt(primary), attempt(primary, delay)])
    }

    func testOnlyLastAttemptWaits() {
        let attempts = NodeStartupPolicy.attempts(preferred: primary, providers: [primary, fallback], retryDelay: delay)

        XCTAssertEqual(attempts.dropLast().map(\.delayBefore), [.zero, .zero])
        XCTAssertEqual(attempts.last?.delayBefore, delay)
    }

    // MARK: - Retry delay

    func testRetryDelay_doublesFromOneSecond() {
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: 0, maxDelay: maxDelay), .seconds(1))
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: 1, maxDelay: maxDelay), .seconds(2))
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: 5, maxDelay: maxDelay), .seconds(32))
    }

    func testRetryDelay_isCappedWithoutOverflow() {
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: 6, maxDelay: maxDelay), maxDelay)
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: 1000, maxDelay: maxDelay), maxDelay)
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: Int.max, maxDelay: maxDelay), maxDelay)
    }

    func testRetryDelay_negativeAttemptIsTreatedAsFirst() {
        XCTAssertEqual(NodeStartupPolicy.retryDelay(forAttempt: -3, maxDelay: maxDelay), .seconds(1))
    }

    // MARK: - Failure outcome

    func testFailureOutcome_offlineWinsOverRetryable() {
        XCTAssertEqual(NodeStartupPolicy.failureOutcome(isOffline: true, isRetryable: true), .offline)
        XCTAssertEqual(NodeStartupPolicy.failureOutcome(isOffline: true, isRetryable: false), .offline)
    }

    func testFailureOutcome_onlineRetriesTransientAndFailsTheRest() {
        XCTAssertEqual(NodeStartupPolicy.failureOutcome(isOffline: false, isRetryable: true), .retry)
        XCTAssertEqual(NodeStartupPolicy.failureOutcome(isOffline: false, isRetryable: false), .fail)
    }
}
