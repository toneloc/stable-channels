import Foundation

/// Single source of fee-rate truth. Strategy-per-source, parallel fetch, async cache.
protocol FeeRateSource: Sendable {
    /// Fetch 6-block target rate (sat/vB). Throws on timeout/network/parse.
    func fetchRate() async throws -> UInt64
}

/// Blockstream esplora `{"6": <sat/vB>}` — stable, no deprecation.
struct BlockstreamFeeSource: FeeRateSource {
    let baseURL: URL
    let timeout: TimeInterval

    func fetchRate() async throws -> UInt64 {
        let url = baseURL.appendingPathComponent("fee-estimates")
        let data = try await Self.fetch(url: url, timeout: timeout)
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let rate = json["6"] as? Double
        else { throw FeeRateError.parseFailed(source: "blockstream") }
        return UInt64(rate.rounded(.up))
    }
}

/// Mempool v1 `/api/v1/fees/recommended` — `{"hourFee": N}`. hourFee ≈ 6-block target.
struct MempoolV1FeeSource: FeeRateSource {
    let baseURL: URL
    let timeout: TimeInterval

    func fetchRate() async throws -> UInt64 {
        let url = baseURL.appendingPathComponent("api/v1/fees/recommended")
        let data = try await Self.fetch(url: url, timeout: timeout)
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let rate = json["hourFee"] as? Double
        else { throw FeeRateError.parseFailed(source: "mempool-v1") }
        return UInt64(rate.rounded(.up))
    }
}

enum FeeRateError: Error {
    case timeout
    case http(Int)
    case parseFailed(source: String)
    case network(Error)
}

extension FeeRateSource {
    /// Shared async fetch with hard timeout. Returns Data on 200, throws otherwise.
    static func fetch(url: URL, timeout: TimeInterval) async throws -> Data {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = timeout
        config.timeoutIntervalForResource = timeout
        let session = URLSession(configuration: config)
        defer { session.finishTasksAndInvalidate() }

        do {
            let (data, response) = try await session.data(from: url)
            guard let http = response as? HTTPURLResponse else {
                throw FeeRateError.http(-1)
            }
            guard http.statusCode == 200 else {
                throw FeeRateError.http(http.statusCode)
            }
            return data
        } catch let err as FeeRateError {
            throw err
        } catch let err as URLError where err.code == .timedOut {
            throw FeeRateError.timeout
        } catch {
            throw FeeRateError.network(error)
        }
    }
}

/// Off-main cache + in-flight dedup. Sources fetched in parallel; first success wins.
actor FeeRateCache {
    private let sources: [FeeRateSource]
    private let cacheTTL: Duration
    private let fallback: UInt64
    private var cachedRate: UInt64?
    private var cachedAt: ContinuousClock.Instant?
    private var inFlight: Task<UInt64, Never>?

    init(
        sources: [FeeRateSource],
        cacheTTL: Duration = .seconds(60),
        fallback: UInt64 = 2
    ) {
        self.sources = sources
        self.cacheTTL = cacheTTL
        self.fallback = fallback
    }

    /// Returns rate. Coalesces concurrent callers — only one network fetch in flight.
    /// First successful source wins; rest cancelled.
    func currentRate() async -> UInt64 {
        if let rate = cachedRate,
           let at = cachedAt,
           ContinuousClock.now - at < cacheTTL {
            return rate
        }
        if let task = inFlight {
            return await task.value
        }
        let task = Task { [sources, fallback] () -> UInt64 in
            await withTaskGroup(of: UInt64?.self, returning: UInt64.self) { group in
                for source in sources {
                    group.addTask {
                        do {
                            return try await source.fetchRate()
                        } catch {
                            return nil
                        }
                    }
                }
                for await rate in group {
                    if let rate {
                        group.cancelAll()
                        return rate
                    }
                }
                return fallback
            }
        }
        inFlight = task
        let rate = await task.value
        inFlight = nil
        if rate != fallback {
            cachedRate = rate
            cachedAt = ContinuousClock.now
        }
        return rate
    }

    func invalidate() {
        cachedRate = nil
        cachedAt = nil
    }
}

/// Public façade. Caller-friendly; defers to cache actor for all state.
final class FeeRateService: Sendable {
    private let cache: FeeRateCache

    init(
        sources: [FeeRateSource] = [
            BlockstreamFeeSource(
                baseURL: URL(string: "https://blockstream.info/api")!,
                timeout: 5
            ),
            MempoolV1FeeSource(
                baseURL: URL(string: "https://mempool.space")!,
                timeout: 5
            )
        ],
        cacheTTL: Duration = .seconds(60),
        fallback: UInt64 = 2
    ) {
        self.cache = FeeRateCache(sources: sources, cacheTTL: cacheTTL, fallback: fallback)
    }

    func currentRate() async -> UInt64 {
        await cache.currentRate()
    }

    func invalidate() async {
        await cache.invalidate()
    }

    /// Test seam: inject a pre-built cache.
    init(cache: FeeRateCache) {
        self.cache = cache
    }
}
