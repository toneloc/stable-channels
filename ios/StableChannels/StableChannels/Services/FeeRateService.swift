import Foundation

/// Multi-tier recommended fee rates (sat/vB) matching target confirmation block conventions.
struct RecommendedFees: Codable, Equatable, Sendable {
    static let minAllowedFeeRate: Double = 0.1
    static let maxAllowedFeeRate: Double = 10_000.0

    let fastestFee: Double
    let halfHourFee: Double
    let hourFee: Double
    let economyFee: Double?
    let minimumFee: Double

    static func sanitizeRate(_ rate: Double, fallback: Double = 1.0) -> Double {
        guard rate.isFinite && rate > 0 else { return fallback }
        return max(minAllowedFeeRate, min(rate, maxAllowedFeeRate))
    }

    func rate(for tier: NetworkFeeSpeedTier) -> Double {
        let minRate = max(Self.minAllowedFeeRate, min(minimumFee, halfHourFee, Self.maxAllowedFeeRate))
        let rawEconomy: Double
        if let economyFee, economyFee > 0 {
            rawEconomy = max(minRate, economyFee)
        } else {
            rawEconomy = max(minRate, hourFee)
        }
        let economyRate = min(Self.maxAllowedFeeRate, rawEconomy)
        let standardRate = min(Self.maxAllowedFeeRate, max(economyRate, max(minRate, halfHourFee)))
        let priorityRate = min(Self.maxAllowedFeeRate, max(standardRate, max(minRate, fastestFee)))

        switch tier {
        case .priority:
            return priorityRate
        case .standard:
            return standardRate
        case .economy:
            return economyRate
        }
    }

    init(
        fastestFee: Double,
        halfHourFee: Double,
        hourFee: Double,
        economyFee: Double? = nil,
        minimumFee: Double
    ) {
        let cleanMin = Self.sanitizeRate(minimumFee, fallback: 1.0)
        self.minimumFee = cleanMin
        self.fastestFee = Self.sanitizeRate(fastestFee, fallback: cleanMin)
        self.halfHourFee = Self.sanitizeRate(halfHourFee, fallback: cleanMin)
        self.hourFee = Self.sanitizeRate(hourFee, fallback: cleanMin)
        if let economyFee {
            self.economyFee = Self.sanitizeRate(economyFee, fallback: cleanMin)
        } else {
            self.economyFee = nil
        }
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let fastestFee = try container.decode(Double.self, forKey: .fastestFee)
        let halfHourFee = try container.decode(Double.self, forKey: .halfHourFee)
        let hourFee = try container.decode(Double.self, forKey: .hourFee)
        let economyFee = try container.decodeIfPresent(Double.self, forKey: .economyFee)
        let minimumFee = try container.decode(Double.self, forKey: .minimumFee)
        self.init(
            fastestFee: fastestFee,
            halfHourFee: halfHourFee,
            hourFee: hourFee,
            economyFee: economyFee,
            minimumFee: minimumFee
        )
    }

    init(wsFees: MempoolWSFees) {
        self.init(
            fastestFee: wsFees.fastestFee,
            halfHourFee: wsFees.halfHourFee,
            hourFee: wsFees.hourFee,
            economyFee: wsFees.economyFee,
            minimumFee: wsFees.minimumFee
        )
    }
}

/// Single source of fee-rate truth. Strategy-per-source, parallel fetch, async cache.
protocol FeeRateSource: Sendable {
    /// Fetch full multi-tier recommended fee structure.
    func fetchRecommendedFees() async throws -> RecommendedFees
}

/// Blockstream esplora `{"1": ..., "3": ..., "6": ..., "144": ...}`
struct BlockstreamFeeSource: FeeRateSource {
    let baseURL: URL
    let timeout: TimeInterval

    func fetchRecommendedFees() async throws -> RecommendedFees {
        let url = baseURL.appendingPathComponent("fee-estimates")
        let data = try await Self.fetch(url: url, timeout: timeout)
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let block1 = (json["1"] as? Double) ?? (json["2"] as? Double),
            let block3 = (json["3"] as? Double) ?? (json["4"] as? Double),
            let block6 = json["6"] as? Double
        else { throw FeeRateError.parseFailed(source: "blockstream") }

        let minFee = (json["144"] as? Double) ?? (json["25"] as? Double) ?? (json["1008"] as? Double) ?? 1.0

        return RecommendedFees(
            fastestFee: block1,
            halfHourFee: block3,
            hourFee: block6,
            minimumFee: max(0.1, minFee)
        )
    }
}

/// Mempool v1 `/api/v1/fees/recommended` — `{fastestFee, halfHourFee, hourFee, minimumFee}`.
struct MempoolV1FeeSource: FeeRateSource {
    let baseURL: URL
    let timeout: TimeInterval

    func fetchRecommendedFees() async throws -> RecommendedFees {
        let url = baseURL.appendingPathComponent("api/v1/fees/recommended")
        let data = try await Self.fetch(url: url, timeout: timeout)
        guard
            let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let fastest = (json["fastestFee"] as? Double) ?? (json["fastestFee"] as? Int).map(Double.init),
            let halfHour = (json["halfHourFee"] as? Double) ?? (json["halfHourFee"] as? Int).map(Double.init),
            let hour = (json["hourFee"] as? Double) ?? (json["hourFee"] as? Int).map(Double.init),
            let minimumFee = (json["minimumFee"] as? Double) ?? (json["minimumFee"] as? Int).map(Double.init)
        else { throw FeeRateError.parseFailed(source: "mempool-v1") }

        let economy = (json["economyFee"] as? Double) ?? (json["economyFee"] as? Int).map(Double.init)

        return RecommendedFees(
            fastestFee: fastest,
            halfHourFee: halfHour,
            hourFee: hour,
            economyFee: economy,
            minimumFee: max(0.1, minimumFee)
        )
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
    private let fallback: Double
    private var cachedRecommendedFees: RecommendedFees?
    private var cachedAt: ContinuousClock.Instant?
    private var inFlight: Task<(fees: RecommendedFees, isFallback: Bool), Never>?
    private var updateGeneration: UInt64 = 0

    init(
        sources: [FeeRateSource],
        cacheTTL: Duration = .seconds(60),
        fallback: Double = 2.0
    ) {
        self.sources = sources
        self.cacheTTL = cacheTTL
        self.fallback = fallback
    }

    func updateRecommendedFees(_ fees: RecommendedFees) {
        updateGeneration &+= 1
        cachedRecommendedFees = fees
        cachedAt = ContinuousClock.now
    }

    /// Returns multi-tier recommended fees. Coalesces concurrent network requests.
    func recommendedFees() async -> RecommendedFees {
        if let rec = cachedRecommendedFees,
           let at = cachedAt,
           ContinuousClock.now - at < cacheTTL {
            return rec
        }

        let generationBeforeWait = updateGeneration
        let task: Task<(fees: RecommendedFees, isFallback: Bool), Never>
        let isInitiator: Bool

        if let existing = inFlight {
            task = existing
            isInitiator = false
        } else {
            let sources = self.sources
            let fallback = self.fallback
            let newTask = Task { [sources, fallback] () -> (fees: RecommendedFees, isFallback: Bool) in
                await withTaskGroup(of: RecommendedFees?.self,
                                    returning: (fees: RecommendedFees, isFallback: Bool).self) { group in
                    for source in sources {
                        group.addTask {
                            do {
                                return try await source.fetchRecommendedFees()
                            } catch {
                                return nil
                            }
                        }
                    }
                    for await fees in group {
                        if let fees {
                            group.cancelAll()
                            return (fees: fees, isFallback: false)
                        }
                    }
                    let fallbackFees = RecommendedFees(
                        fastestFee: max(0.1, fallback * 1.3),
                        halfHourFee: fallback,
                        hourFee: max(0.1, fallback * 0.8),
                        minimumFee: max(0.1, fallback * 0.5)
                    )
                    return (fees: fallbackFees, isFallback: true)
                }
            }
            inFlight = newTask
            task = newTask
            isInitiator = true
        }

        let result = await task.value
        if isInitiator {
            inFlight = nil
        }

        // If the cache was updated or invalidated while suspended, do not overwrite with the older fetch result.
        if updateGeneration != generationBeforeWait {
            if let fresh = cachedRecommendedFees {
                return fresh
            }
            return result.fees
        }

        // If another waiter already populated the cache with fresh data, return it.
        if let fresh = cachedRecommendedFees,
           let at = cachedAt,
           ContinuousClock.now - at < cacheTTL {
            return fresh
        }

        if !result.isFallback {
            updateGeneration &+= 1
            cachedRecommendedFees = result.fees
            cachedAt = ContinuousClock.now
            return result.fees
        } else if let lastKnownGood = cachedRecommendedFees {
            // Retain last-known-good cached rates during transient network outages
            return lastKnownGood
        }
        return result.fees
    }

    func currentRate() async -> Double {
        let fees = await recommendedFees()
        return fees.rate(for: .standard)
    }

    func invalidate() {
        updateGeneration &+= 1
        cachedRecommendedFees = nil
        cachedAt = nil
    }
}

/// Public façade. Caller-friendly; defers to cache actor for all state.
final class FeeRateService: Sendable {
    private let cache: FeeRateCache

    init(
        sources: [FeeRateSource] = [
            BlockstreamFeeSource(
                baseURL: URL(string: Constants.feeRateBlockstreamURL)!,
                timeout: 5
            ),
            MempoolV1FeeSource(
                baseURL: URL(string: Constants.feeRateMempoolURL)!,
                timeout: 5
            )
        ],
        cacheTTL: Duration = .seconds(60),
        fallback: Double = 2.0
    ) {
        self.cache = FeeRateCache(sources: sources, cacheTTL: cacheTTL, fallback: fallback)
    }

    func currentRate() async -> Double {
        await cache.currentRate()
    }

    func currentRateSatVb() async -> UInt64 {
        let rate = await currentRate()
        guard rate.isFinite && rate > 0 else { return 1 }
        if rate >= Double(UInt64.max) { return UInt64.max }
        return UInt64(max(1.0, rate.rounded()))
    }

    func recommendedFees() async -> RecommendedFees {
        await cache.recommendedFees()
    }

    func updateRecommendedFees(_ fees: RecommendedFees) async {
        await cache.updateRecommendedFees(fees)
    }

    func invalidate() async {
        await cache.invalidate()
    }

    /// Test seam: inject a pre-built cache.
    init(cache: FeeRateCache) {
        self.cache = cache
    }
}
