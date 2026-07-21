import Foundation

enum Constants {
    // MARK: - Network

    static let satsInBTC: UInt64 = 100_000_000
    static let stableChannelTLVType: UInt64 = 13_377_331
    static let tradeMessageType = "TRADE_V1"
    static let syncMessageType = "SYNC_V1"

    // MARK: - Default Configuration

    static let defaultNetwork = "bitcoin"
    static let defaultUserAlias = "user"
    static let defaultUserPort: UInt16 = 9736
    static let defaultLSPAlias = "lsp"
    static let defaultLSPPort: UInt16 = 9735

    static let primaryChainURL = "https://blockstream.info/api"
    static let fallbackChainURL = "https://mempool.space/api"
    static let esploraChainURLs: [String] = [primaryChainURL, fallbackChainURL]
    static let txExplorerURL = "https://mempool.space/tx"

    static let feeRateBlockstreamURL = primaryChainURL
    static let feeRateMempoolURL = "https://mempool.space"

    // MARK: - Service Endpoints

    static let lspPushRegisterURL = "https://stablechannels.com/api/register-push"
    static let lspChannelExistsURL = "https://stablechannels.com/api/channel-exists"
    static let privacyPolicyURL = "https://stablechannels.com/privacy.html"

    static func txExplorerLink(for txid: String) -> URL? {
        URL(string: "\(txExplorerURL)/\(txid)")
    }

    static let defaultLSPPubkey = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    static let defaultLSPAddress = "stablechannels.com:9735"

    // MARK: - Timing

    static let priceCacheRefreshSecs: UInt64 = 5
    static let priceFetchRetryDelayMs: UInt64 = 300
    static let priceFetchMaxRetries = 3

    static let onchainWalletSyncIntervalSecs: UInt64 = 120
    static let lightningWalletSyncIntervalSecs: UInt64 = 60
    static let feeRateCacheUpdateIntervalSecs: UInt64 = 1200

    static let invoiceExpirySecs: UInt32 = 3600
    static let balanceUpdateIntervalSecs: UInt64 = 30
    static let stabilityCheckIntervalSecs: UInt64 = 60

    // MARK: - Business Logic

    static let maxRiskLevel: Int32 = 100
    static let stabilityThresholdPercent: Double = 0.1
    static let stabilityThresholdUSD: Double = 0.25
    static let stabilityPaymentCooldownSecs: UInt64 = 120
    static let minDisplayUSD: Double = 2.0
    static let maxChannelUSD: Double = 100.0

    // MARK: - Channel

    static let defaultChannelLifetime: UInt32 = 2016
    static let defaultMaxClientToSelfDelay: UInt32 = 1024
    static let minPaymentSizeMsat: UInt64 = 0
    static let maxPaymentSizeMsat: UInt64 = 100_000_000_000
    static let channelOverProvisioningPPM: UInt32 = 1_000_000
    static let channelOpeningFeePPM: UInt32 = 0
    static let minChannelOpeningFeeMsat: UInt64 = 0
    static let minChannelLifetime: UInt32 = 100
    static let maxProportionalLSPFeeLimitPPMMsat: UInt64 = 10_000_000

    // MARK: - Price Feeds

    static let defaultPriceFeeds: [PriceFeedConfig] = [
        PriceFeedConfig(
            name: "Bitstamp",
            urlFormat: "https://www.bitstamp.net/api/v2/ticker/btcusd/",
            jsonPath: ["last"]
        ),
        PriceFeedConfig(
            name: "CoinGecko",
            urlFormat: "https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd",
            jsonPath: ["bitcoin", "usd"]
        ),
        PriceFeedConfig(
            name: "Kraken",
            urlFormat: "https://api.kraken.com/0/public/Ticker?pair=XXBTZUSD",
            jsonPath: ["result", "XXBTZUSD", "c"]
        ),
        PriceFeedConfig(
            name: "Coinbase",
            urlFormat: "https://api.coinbase.com/v2/prices/spot?currency=USD",
            jsonPath: ["data", "amount"]
        ),
        PriceFeedConfig(
            name: "Blockchain.com",
            urlFormat: "https://blockchain.info/ticker",
            jsonPath: ["USD", "last"]
        )
    ]

    // MARK: - RGS (Rapid Gossip Sync) Servers

    enum RGSServer {
        static let bitcoin = "https://rapidsync.lightningdevkit.org/snapshot/"
        static let signet = "https://rgs.mutinynet.com/snapshot/"
        static let testnet = "https://rapidsync.lightningdevkit.org/testnet/snapshot/"
    }

    // MARK: - Push Notifications

    static let appGroupIdentifier = "group.com.stablechannels.app"

    // MARK: - Data Directory

    static var userDataDir: URL {
        if let shared = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupIdentifier
        ) {
            return shared.appendingPathComponent("StableChannels")
                .appendingPathComponent(defaultUserAlias)
        }
        // Fallback to Application Support if App Group is unavailable
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        return appSupport.appendingPathComponent("StableChannels").appendingPathComponent(defaultUserAlias)
    }
}

struct PriceFeedConfig: Codable {
    let name: String
    let urlFormat: String
    let jsonPath: [String]
}

// MARK: - Seed Constants

enum SeedConstants {
    static let wordCount12 = 12
    static let wordCount24 = 24
    static let maxWordCount = 24
    static let clipboardClearSeconds: TimeInterval = 60
    static let defaultWordCount = 12
    static let animationDuration: TimeInterval = 0.3
    static let successDisplaySeconds: UInt64 = 1_500_000_000
}

enum LogLevel: Int, Comparable {
    case debug = 0
    case info = 1
    case warn = 2
    case error = 3

    static func < (lhs: LogLevel, rhs: LogLevel) -> Bool {
        return lhs.rawValue < rhs.rawValue
    }
    
    var stringValue: String {
        switch self {
        case .debug: return "DEBUG"
        case .info: return "INFO"
        case .warn: return "WARN"
        case .error: return "ERROR"
        }
    }
}

class AppLogger {
    static let shared = AppLogger()
    
    private let logFileName = "app_debug.log"
    private let oldLogFileName = "app_debug.old.log"
    private let maxFileSize: UInt64 = 5 * 1024 * 1024 // 5MB
    
    private let dateFormatter: ISO8601DateFormatter
    
    // Naive regex to redact 12-24 word BIP39 seed phrases
    private let seedRegex: NSRegularExpression?
    
    // Default to info for release, debug for debug builds
    var minLevel: LogLevel = {
        #if DEBUG
        return .debug
        #else
        return .info
        #endif
    }()
    
    private let ioQueue = DispatchQueue(label: "com.stablechannels.app.logger")
    
    private init() {
        self.dateFormatter = ISO8601DateFormatter()
        self.dateFormatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        self.seedRegex = try? NSRegularExpression(pattern: "(?<=\\b|\\s)(?:[a-z]+\\s){11,23}[a-z]+(?=\\b|\\s)", options: [])
    }
    
    private var logFileURL: URL? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: Constants.appGroupIdentifier) else {
            return nil
        }
        return container.appendingPathComponent(logFileName)
    }
    
    private var oldLogFileURL: URL? {
        guard let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: Constants.appGroupIdentifier) else {
            return nil
        }
        return container.appendingPathComponent(oldLogFileName)
    }
    
    func d(tag: String, _ message: String) { self.log(level: .debug, tag: tag, message) }
    func i(tag: String, _ message: String) { self.log(level: .info, tag: tag, message) }
    func w(tag: String, _ message: String) { self.log(level: .warn, tag: tag, message) }
    func e(tag: String, _ message: String) { self.log(level: .error, tag: tag, message) }
    
    private func log(level: LogLevel, tag: String, _ message: String) {
        guard level >= minLevel else { return }
        
        var finalMessage = message
        if let regex = seedRegex {
            let range = NSRange(location: 0, length: finalMessage.utf16.count)
            finalMessage = regex.stringByReplacingMatches(in: finalMessage, options: [], range: range, withTemplate: "[REDACTED_SEED]")
        }
        
        let timestamp = dateFormatter.string(from: Date())
        let logLine = "[\(timestamp)] [\(level.stringValue)] [\(tag)] \(finalMessage)\n"
        
        // Print to console
        #if DEBUG
        print(logLine, terminator: "")
        #endif
        
        ioQueue.async { [weak self] in
            self?.writeToFile(logLine)
        }
    }
    
    private func writeToFile(_ line: String) {
        guard let fileURL = logFileURL, let data = line.data(using: .utf8) else { return }
        
        let fileManager = FileManager.default
        
        if fileManager.fileExists(atPath: fileURL.path) {
            do {
                let attrs = try fileManager.attributesOfItem(atPath: fileURL.path)
                let fileSize = attrs[.size] as? UInt64 ?? 0
                
                if fileSize > maxFileSize {
                    rotateLogs()
                }
            } catch {
                // Ignore
            }
        }
        
        if !fileManager.fileExists(atPath: fileURL.path) {
            try? data.write(to: fileURL)
            return
        }
        
        if let fileHandle = try? FileHandle(forWritingTo: fileURL) {
            fileHandle.seekToEndOfFile()
            fileHandle.write(data)
            fileHandle.closeFile()
        }
    }
    
    private func rotateLogs() {
        guard let fileURL = logFileURL, let oldFileURL = oldLogFileURL else { return }
        let fileManager = FileManager.default
        
        if fileManager.fileExists(atPath: oldFileURL.path) {
            try? fileManager.removeItem(at: oldFileURL)
        }
        
        try? fileManager.moveItem(at: fileURL, to: oldFileURL)
    }
    
    func getExportLogs() -> [URL] {
        var urls: [URL] = []
        let fileManager = FileManager.default
        
        if let fileURL = logFileURL, fileManager.fileExists(atPath: fileURL.path) {
            urls.append(fileURL)
        }
        
        if let oldFileURL = oldLogFileURL, fileManager.fileExists(atPath: oldFileURL.path) {
            urls.append(oldFileURL)
        }
        
        // Audit log
        let auditURL = Constants.userDataDir.appendingPathComponent("audit_log.txt")
        if fileManager.fileExists(atPath: auditURL.path) {
            urls.append(auditURL)
        }
        
        // LDK node log
        let ldkLogURL = Constants.userDataDir.appendingPathComponent("ldk-node.log")
        if fileManager.fileExists(atPath: ldkLogURL.path) {
            urls.append(ldkLogURL)
        }
        
        return urls
    }
>>>>>>> d635c6f (feat: Add Logs & Diagnostics UI to Android, iOS, and Desktop)
}
