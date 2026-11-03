import Foundation

/// Picks the provider a launch starts against by probing the primary once. It sits on the
/// critical path before first paint, so the live probe has a short budget of its own.
struct ChainSourceProbe: Sendable {
    /// Whether the provider answered its tip-height endpoint with HTTP 200.
    let isReachable: @Sendable (URL) async -> Bool
    var primary = Constants.primaryChainURL
    var fallback = Constants.fallbackChainURL
    var log: @Sendable (String, [String: Any]) -> Void = { AuditService.log($0, data: $1) }

    func resolve() async -> String {
        guard let url = URL(string: "\(primary)/blocks/tip/height") else { return fallback }
        let startedAt = Date()
        let reachable = await isReachable(url)
        let chosen = NodeStartupPolicy.launchProvider(primaryReachable: reachable, primary: primary, fallback: fallback)
        let ms = "\(Int(Date().timeIntervalSince(startedAt) * 1000))"
        if reachable {
            log("CHAIN_SOURCE_RESOLVED", ["using": chosen, "ms": ms])
        } else {
            log("CHAIN_SOURCE_FALLBACK", ["primary": primary, "using": chosen, "ms": ms])
        }
        return chosen
    }
}

extension ChainSourceProbe {
    /// One session for every probe, so launches and retries don't each leave a session behind.
    /// URLSession.shared defaults to a 60s request timeout, which would hold the launch screen
    /// for a minute on a slow primary.
    static let liveSession: URLSession = {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = Constants.chainProbeRequestTimeoutSecs
        config.timeoutIntervalForResource = Constants.chainProbeResourceTimeoutSecs
        return URLSession(configuration: config)
    }()

    static var live: ChainSourceProbe {
        ChainSourceProbe(isReachable: { url in
            guard let (_, response) = try? await liveSession.data(from: url) else {
                return false
            }
            return (response as? HTTPURLResponse)?.statusCode == 200
        })
    }
}
