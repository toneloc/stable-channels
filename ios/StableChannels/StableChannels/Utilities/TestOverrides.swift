import Foundation

/// Debug-only endpoint overrides for E2E testing against the regtest harness
/// (e2e/harness). Release builds NEVER read the file (compile-time gate).
///
/// The test rig writes test_config.json into the app's Documents directory
/// (simulator: `xcrun simctl get_app_container booted <bundle> data`) after
/// the harness boots — the LSP node id is only known then, which is why this
/// is a runtime file, not build configuration. Same JSON shape as Android's
/// TestOverrides (util/TestOverrides.kt).
enum TestOverrides {
    struct Values {
        var network: String?
        var primaryChainUrl: String?
        var fallbackChainUrl: String?
        var lspPubkey: String?
        var lspAddress: String?
        var pushRegisterUrl: String?
        var channelExistsUrl: String?
        var priceFeedBase: String?
        var disableSendAuth = false
        var syncIntervalSecs: UInt64?
    }

    /// Loaded once on first access; thread-safe via `static let`.
    static let shared: Values = load()

    private static func load() -> Values {
        var v = Values()
        #if DEBUG
            let docs = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask).first
            guard let url = docs?.appendingPathComponent("test_config.json"),
                  let data = try? Data(contentsOf: url),
                  let json = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            else { return v }
            func opt(_ key: String) -> String? {
                (json[key] as? String).flatMap { $0.isEmpty ? nil : $0 }
            }
            v.network = opt("network")
            v.primaryChainUrl = opt("primary_chain_url")
            v.fallbackChainUrl = opt("fallback_chain_url")
            v.lspPubkey = opt("lsp_pubkey")
            v.lspAddress = opt("lsp_address")
            v.pushRegisterUrl = opt("push_register_url")
            v.channelExistsUrl = opt("channel_exists_url")
            v.priceFeedBase = opt("price_feed_base")
            v.disableSendAuth = (json["disable_send_auth"] as? Bool) ?? false
            if let secs = (json["sync_interval_secs"] as? NSNumber)?.uint64Value, secs > 0 {
                v.syncIntervalSecs = secs
            }
            print(
                "[TestOverrides] E2E overrides ACTIVE: network=\(v.network ?? "-") lsp=\(v.lspAddress ?? "-") chain=\(v.primaryChainUrl ?? "-")"
            )
        #endif
        return v
    }

    /// Harness-served replacements for the five production price feeds.
    static func priceFeeds(base: String) -> [PriceFeedConfig] {
        [
            PriceFeedConfig(name: "Bitstamp", urlFormat: "\(base)/feeds/bitstamp", jsonPath: ["last"]),
            PriceFeedConfig(name: "CoinGecko", urlFormat: "\(base)/feeds/coingecko", jsonPath: ["bitcoin", "usd"]),
            PriceFeedConfig(name: "Kraken", urlFormat: "\(base)/feeds/kraken", jsonPath: ["result", "XXBTZUSD", "c"]),
            PriceFeedConfig(name: "Coinbase", urlFormat: "\(base)/feeds/coinbase", jsonPath: ["data", "amount"]),
            PriceFeedConfig(name: "Blockchain.com", urlFormat: "\(base)/feeds/blockchain", jsonPath: ["USD", "last"])
        ]
    }
}
