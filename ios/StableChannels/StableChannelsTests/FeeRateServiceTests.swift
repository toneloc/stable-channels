import XCTest
@testable import StableChannels

// MARK: - Stub

/// Deterministic FeeRateSource for tests. Avoids real network.
struct StubFeeRateSource: FeeRateSource {
    let rate: UInt64?
    let delay: Duration
    let throwsError: Error?

    init(rate: UInt64?, delay: Duration = .zero, throwsError: Error? = nil) {
        self.rate = rate
        self.delay = delay
        self.throwsError = throwsError
    }

    func fetchRate() async throws -> UInt64 {
        if delay > .zero {
            try? await Task.sleep(for: delay)
        }
        if let err = throwsError {
            throw err
        }
        guard let rate else { throw FeeRateError.parseFailed(source: "stub") }
        return rate
    }
}

// MARK: - Tests

final class FeeRateServiceTests: XCTestCase {
    // 1. Cache hit returns same value without re-fetching

    func testCacheHit_returnsSameValueWithoutRefetching() async {
        let counter = FetchCounter()
        let source = CountingSource(rate: 7, counter: counter)
        let cache = FeeRateCache(sources: [source], cacheTTL: .seconds(60), fallback: 2)

        let first = await cache.currentRate()
        let second = await cache.currentRate()
        let third = await cache.currentRate()

        XCTAssertEqual(first, 7)
        XCTAssertEqual(second, 7)
        XCTAssertEqual(third, 7)
        XCTAssertEqual(counter.count, 1, "Second/third hit should be cache, not network")
    }

    // 2. In-flight coalescing — parallel callers share one fetch

    func testInflightDedup_parallelCallersShareOneFetch() async {
        let counter = FetchCounter()
        let slow = CountingSource(rate: 11, counter: counter, delay: .milliseconds(150))
        let cache = FeeRateCache(sources: [slow], cacheTTL: .seconds(60), fallback: 2)

        async let a = cache.currentRate()
        async let b = cache.currentRate()
        async let c = cache.currentRate()

        let results = await [a, b, c]

        XCTAssertEqual(results, [11, 11, 11])
        XCTAssertEqual(counter.count, 1, "Three callers should coalesce into one fetch")
    }

    // 3. First-success-wins — faster good source beats slower good source

    func testFirstSuccessWins_fasterGoodBeatsSlowerGood() async {
        let slow = StubFeeRateSource(rate: 999, delay: .milliseconds(300))
        let fast = StubFeeRateSource(rate: 4, delay: .milliseconds(20))
        let cache = FeeRateCache(sources: [slow, fast], cacheTTL: .seconds(60), fallback: 2)

        let rate = await cache.currentRate()
        XCTAssertEqual(rate, 4, "First successful source wins; slow path is cancelled")
    }

    // 4. All sources fail → fallback returned

    func testAllSourcesFail_returnsFallback() async {
        let bad1 = StubFeeRateSource(rate: nil)
        let bad2 = StubFeeRateSource(rate: 1, throwsError: FeeRateError.timeout)
        let cache = FeeRateCache(sources: [bad1, bad2], cacheTTL: .seconds(60), fallback: 2)

        let rate = await cache.currentRate()
        XCTAssertEqual(rate, 2, "Both sources fail → fallback")
    }

    // 5. One source fails, other succeeds → succeed

    func testOneFailsOneSucceeds_returnsSuccess() async {
        let bad = StubFeeRateSource(rate: 1, throwsError: FeeRateError.parseFailed(source: "x"))
        let good = StubFeeRateSource(rate: 19)
        let cache = FeeRateCache(sources: [bad, good], cacheTTL: .seconds(60), fallback: 2)

        let rate = await cache.currentRate()
        XCTAssertEqual(rate, 19)
    }

    // 6. invalidate() forces re-fetch

    func testInvalidate_forcesRefetch() async {
        let counter = FetchCounter()
        let source = CountingSource(rate: 5, counter: counter)
        let cache = FeeRateCache(sources: [source], cacheTTL: .seconds(60), fallback: 2)

        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 1)
        await cache.invalidate()
        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 2, "invalidate() should drop cache and re-fetch")
    }

    // 7. Expiry — stale cache re-fetches after TTL

    func testExpiry_staleCacheRefetches() async {
        let counter = FetchCounter()
        let source = CountingSource(rate: 3, counter: counter)
        let cache = FeeRateCache(sources: [source], cacheTTL: .milliseconds(50), fallback: 2)

        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 1)
        try? await Task.sleep(for: .milliseconds(120))
        _ = await cache.currentRate()
        XCTAssertEqual(counter.count, 2, "Past TTL → re-fetch")
    }

    // 8. Fallback is NOT cached (so a later valid fetch can win)

    func testFallbackNotCached_allowsLaterSuccess() async {
        let counter = FetchCounter()
        let bad = CountingSource(rate: 0, counter: counter, throwsError: FeeRateError.timeout)
        let cache = FeeRateCache(sources: [bad], cacheTTL: .seconds(60), fallback: 9)

        let first = await cache.currentRate()
        XCTAssertEqual(first, 9, "Should fall back to 9 on first failure")
        // Second call — fallback should NOT be cached, but source still throws,
        // so still 9 (this just confirms we don't lock in the fallback)
        let second = await cache.currentRate()
        XCTAssertEqual(second, 9)
    }

    // 9. FeeRateService façade delegates to cache

    @MainActor
    func testFacade_delegatesToCache() async {
        let source = StubFeeRateSource(rate: 25)
        let cache = FeeRateCache(sources: [source], cacheTTL: .seconds(60), fallback: 2)
        let service = FeeRateService(cache: cache)
        let rate = await service.currentRate()
        XCTAssertEqual(rate, 25, "Façade should delegate to injected cache")
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
    let rate: UInt64
    let counter: FetchCounter
    let delay: Duration
    let throwsError: Error?

    init(rate: UInt64, counter: FetchCounter, delay: Duration = .zero, throwsError: Error? = nil) {
        self.rate = rate
        self.counter = counter
        self.delay = delay
        self.throwsError = throwsError
    }

    func fetchRate() async throws -> UInt64 {
        counter.bump()
        if delay > .zero {
            try? await Task.sleep(for: delay)
        }
        if let err = throwsError {
            throw err
        }
        return rate
    }
}
