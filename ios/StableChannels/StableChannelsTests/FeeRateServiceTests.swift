import XCTest
@testable import StableChannels

// MARK: - Stub

/// Deterministic FeeRateSource for tests. Avoids real network.
struct StubFeeRateSource: FeeRateSource {
    let rate: Double?
    let delay: Duration
    let throwsError: Error?

    init(rate: Double?, delay: Duration = .zero, throwsError: Error? = nil) {
        self.rate = rate
        self.delay = delay
        self.throwsError = throwsError
    }

    func fetchRecommendedFees() async throws -> RecommendedFees {
        if delay > .zero {
            try? await Task.sleep(for: delay)
        }
        if let err = throwsError {
            throw err
        }
        guard let rate else { throw FeeRateError.parseFailed(source: "stub") }
        return RecommendedFees(
            fastestFee: max(0.1, rate * 1.3),
            halfHourFee: rate,
            hourFee: max(0.1, rate * 0.8),
            minimumFee: max(0.1, rate * 0.5)
        )
    }
}

// MARK: - Tests

final class FeeRateServiceTests: XCTestCase {
    // 1. Cache hit returns same value without re-fetching

    func testCacheHit_returnsSameValueWithoutRefetching() async {
        let counter = FetchCounter()
        let source = CountingSource(rate: 7.0, counter: counter)
        let cache = FeeRateCache(sources: [source], cacheTTL: .seconds(60), fallback: 2.0)

        let first = await cache.currentRate()
        let second = await cache.currentRate()
        let third = await cache.currentRate()

        XCTAssertEqual(first, 7.0)
        XCTAssertEqual(second, 7.0)
        XCTAssertEqual(third, 7.0)
        XCTAssertEqual(counter.count, 1, "Second/third hit should be cache, not network")
    }

    // 2. In-flight coalescing — parallel callers share one fetch

    func testInflightDedup_parallelCallersShareOneFetch() async {
        let counter = FetchCounter()
        let slow = CountingSource(rate: 11.0, counter: counter, delay: .milliseconds(150))
        let cache = FeeRateCache(sources: [slow], cacheTTL: .seconds(60), fallback: 2.0)

        async let a = cache.currentRate()
        async let b = cache.currentRate()
        async let c = cache.currentRate()

        let results = await [a, b, c]

        XCTAssertEqual(results, [11.0, 11.0, 11.0])
        XCTAssertEqual(counter.count, 1, "Three callers should coalesce into one fetch")
    }

    // 3. First-success-wins — faster good source beats slower good source

    func testFirstSuccessWins_fasterGoodBeatsSlowerGood() async {
        let slow = StubFeeRateSource(rate: 999.0, delay: .milliseconds(300))
        let fast = StubFeeRateSource(rate: 4.0, delay: .milliseconds(20))
        let cache = FeeRateCache(sources: [slow, fast], cacheTTL: .seconds(60), fallback: 2.0)

        let rate = await cache.currentRate()
        XCTAssertEqual(rate, 4.0, "First successful source wins; slow path is cancelled")
    }

    // 4. All sources fail → fallback returned

    func testAllSourcesFail_returnsFallback() async {
        let bad1 = StubFeeRateSource(rate: nil)
        let bad2 = StubFeeRateSource(rate: 1.0, throwsError: FeeRateError.timeout)
        let cache = FeeRateCache(sources: [bad1, bad2], cacheTTL: .seconds(60), fallback: 2.0)

        let rate = await cache.currentRate()
        XCTAssertEqual(rate, 2.0, "Both sources fail → fallback")
    }

    func testAllSourcesFail_fallbackNotCachedForTTL() async {
        let counter = FetchCounter()
        let failingSource = CountingSource(rate: 1.0, counter: counter, throwsError: FeeRateError.timeout)
        let cache = FeeRateCache(sources: [failingSource], cacheTTL: .seconds(60), fallback: 2.0)

        let first = await cache.currentRate()
        XCTAssertEqual(first, 2.0)
        XCTAssertEqual(counter.count, 1)

        // Second call should attempt re-fetching because fallback was NOT cached
        let second = await cache.currentRate()
        XCTAssertEqual(second, 2.0)
        XCTAssertEqual(counter.count, 2, "Fallback must not be cached; second call must re-attempt fetch")
    }

    // 5. One source fails, other succeeds → succeed

    func testOneFailsOneSucceeds_returnsSuccess() async {
        let bad = StubFeeRateSource(rate: 1.0, throwsError: FeeRateError.parseFailed(source: "x"))
        let good = StubFeeRateSource(rate: 19.0)
        let cache = FeeRateCache(sources: [bad, good], cacheTTL: .seconds(60), fallback: 2.0)

        let rate = await cache.currentRate()
        XCTAssertEqual(rate, 19.0)
    }

    // 6. invalidate() forces re-fetch

    func testInvalidate_forcesRefetch() async {
        let counter = FetchCounter()
        let source = CountingSource(rate: 5.0, counter: counter)
        let cache = FeeRateCache(sources: [source], cacheTTL: .seconds(60), fallback: 2.0)

        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 1)
        await cache.invalidate()
        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 2, "invalidate() should drop cache and re-fetch")
    }

    // 7. Expiry — stale cache re-fetches after TTL

    func testExpiry_staleCacheRefetches() async {
        let counter = FetchCounter()
        let source = CountingSource(rate: 3.0, counter: counter)
        let cache = FeeRateCache(sources: [source], cacheTTL: .milliseconds(50), fallback: 2.0)

        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 1)
        try? await Task.sleep(for: .milliseconds(120))
        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 2, "Past TTL → re-fetch")
    }

    // 8. Fallback is NOT cached (so a later valid fetch can win)

    func testFallbackNotCached_allowsLaterSuccess() async {
        let counter = FetchCounter()
        let bad = CountingSource(rate: 0.0, counter: counter, throwsError: FeeRateError.timeout)
        let cache = FeeRateCache(sources: [bad], cacheTTL: .seconds(60), fallback: 9.0)

        let first = await cache.currentRate()
        XCTAssertEqual(first, 9.0, "Should fall back to 9 on first failure")
        // Second call — fallback should NOT be cached, but source still throws,
        // so still 9 (this just confirms we don't lock in the fallback)
        let second = await cache.currentRate()
        XCTAssertEqual(second, 9.0)
    }

    // 9. FeeRateService façade delegates to cache

    @MainActor
    func testFacade_delegatesToCache() async {
        let source = StubFeeRateSource(rate: 25.0)
        let cache = FeeRateCache(sources: [source], cacheTTL: .seconds(60), fallback: 2.0)
        let service = FeeRateService(cache: cache)
        let rate = await service.currentRate()
        XCTAssertEqual(rate, 25.0, "Façade should delegate to injected cache")
    }

    // 10. WebSocket push during in-flight fetch is returned to both initiator and coalesced waiters

    func testWebsocketFeeUpdate_notOverwrittenByOlderInflightFetch() async {
        let slowSource = StubFeeRateSource(rate: 10.0, delay: .milliseconds(120))
        let cache = FeeRateCache(sources: [slowSource], cacheTTL: .seconds(60), fallback: 2.0)

        // Initiator caller
        let fetchTask1 = Task {
            await cache.recommendedFees()
        }

        // Wait to guarantee fetchTask1 entered inFlight
        try? await Task.sleep(for: .milliseconds(20))

        // Coalesced waiter caller
        let fetchTask2 = Task {
            await cache.recommendedFees()
        }

        try? await Task.sleep(for: .milliseconds(10))

        // Intervening WebSocket update arrives while both are waiting
        let wsFees = RecommendedFees(fastestFee: 65.0, halfHourFee: 50.0, hourFee: 40.0, minimumFee: 25.0)
        await cache.updateRecommendedFees(wsFees)

        // Both initiator and coalesced waiter must receive the fresh WebSocket update (50)
        let result1 = await fetchTask1.value
        let result2 = await fetchTask2.value

        XCTAssertEqual(result1.halfHourFee, 50.0, "Initiating caller should receive fresh WebSocket update")
        XCTAssertEqual(result2.halfHourFee, 50.0, "Coalesced waiter caller should receive fresh WebSocket update")

        let currentCached = await cache.recommendedFees()
        XCTAssertEqual(currentCached.halfHourFee, 50.0)
        let currentRate = await cache.currentRate()
        XCTAssertEqual(currentRate, 50.0)
    }

    func testRecommendedFees_ingressSanitizationAndCeiling() {
        // Negative, zero, NaN, and infinite values should be sanitized to fallback
        let hostile = RecommendedFees(
            fastestFee: -10.0,
            halfHourFee: Double.nan,
            hourFee: Double.infinity,
            economyFee: 0.0,
            minimumFee: -1.0
        )
        XCTAssertEqual(hostile.minimumFee, 1.0)
        XCTAssertEqual(hostile.fastestFee, 1.0)
        XCTAssertEqual(hostile.halfHourFee, 1.0)
        XCTAssertEqual(hostile.hourFee, 1.0)
        XCTAssertEqual(hostile.economyFee, 1.0)

        // Absurd rate exceeding maxAllowedFeeRate (10,000) should be clamped
        let absurd = RecommendedFees(
            fastestFee: 5_000_000.0,
            halfHourFee: 100.0,
            hourFee: 50.0,
            minimumFee: 10.0
        )
        XCTAssertEqual(absurd.fastestFee, RecommendedFees.maxAllowedFeeRate)
        XCTAssertEqual(absurd.rate(for: .priority), RecommendedFees.maxAllowedFeeRate)
    }

    func testCurrentRateSatVb_hugeRateDoesNotTrap() async {
        let hugeSource = StubFeeRateSource(rate: 5_000_000.0)
        let service = FeeRateService(sources: [hugeSource])
        let satVb = await service.currentRateSatVb()
        XCTAssertEqual(satVb, UInt64(RecommendedFees.maxAllowedFeeRate))
    }

    func testNetworkFailureAfterValidFetch_preservesLastKnownGoodRates() async {
        final class FlakySource: FeeRateSource, @unchecked Sendable {
            var shouldFail = false
            func fetchRecommendedFees() async throws -> RecommendedFees {
                if shouldFail {
                    throw FeeRateError.timeout
                }
                return RecommendedFees(
                    fastestFee: 60.0,
                    halfHourFee: 45.0,
                    hourFee: 30.0,
                    minimumFee: 15.0
                )
            }
        }

        let source = FlakySource()
        let cache = FeeRateCache(sources: [source], cacheTTL: .milliseconds(50), fallback: 2.0)

        let initial = await cache.currentRate()
        XCTAssertEqual(initial, 45.0)

        // Wait past TTL and make network fail
        source.shouldFail = true
        try? await Task.sleep(for: .milliseconds(100))

        let afterFailure = await cache.currentRate()
        XCTAssertEqual(afterFailure, 45.0, "Should preserve last known good rate instead of dropping to 2.0 fallback")
    }
}

// MARK: - Counting helpers

/// Thread-safe counter for verifying fetch count across concurrent awaits.
final class FetchCounter: @unchecked Sendable {
    private var n: Int = 0
    private let lock = NSLock()
    var count: Int {
        lock.lock(); defer { lock.unlock() }
        return n
    }

    func bump() {
        lock.lock(); defer { lock.unlock() }
        n += 1
    }
}

struct CountingSource: FeeRateSource {
    let rate: Double
    let counter: FetchCounter
    let delay: Duration
    let throwsError: Error?

    init(rate: Double, counter: FetchCounter, delay: Duration = .zero, throwsError: Error? = nil) {
        self.rate = rate
        self.counter = counter
        self.delay = delay
        self.throwsError = throwsError
    }

    func fetchRecommendedFees() async throws -> RecommendedFees {
        counter.bump()
        if delay > .zero {
            try? await Task.sleep(for: delay)
        }
        if let err = throwsError {
            throw err
        }
        return RecommendedFees(
            fastestFee: max(0.1, rate * 1.3),
            halfHourFee: rate,
            hourFee: max(0.1, rate * 0.8),
            minimumFee: max(0.1, rate * 0.5)
        )
    }
}
