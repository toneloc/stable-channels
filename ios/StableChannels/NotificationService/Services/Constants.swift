import Foundation

enum Constants {
    static let appGroup = "group.com.stablechannels.app"
    static let lspPubkey = "0388948c5c7775a5eda3ee4a96434a270f20f5beeed7e9c99f242f21b87d658850"
    static let lspAddress = "stablechannels.com:9735"
    static let stableChannelTLVType: UInt64 = 13_377_331
    static let signedStabilityTLVType: UInt64 = 13_377_333
    static let syncMessageType = "SYNC_V1"
    static let satsInBTC = BitcoinUnits.satsPerBTC
    static let stabilityThresholdPercent: Double = 0.1
    /// Below this the position counts as zero-target; a zero target with no backing is closed.
    static let minimumStableUSD: Double = 0.01
    static let primaryChainURL = "https://blockstream.info/api"
    static let fallbackChainURL = "https://mempool.space/api"
}

enum Diagnostics {
    static func residentMemoryBytes() -> UInt64 {
        var info = mach_task_basic_info()
        var count = mach_msg_type_number_t(MemoryLayout<mach_task_basic_info>.size) / 4
        let kerr: kern_return_t = withUnsafeMutablePointer(to: &info) {
            $0.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
                task_info(mach_task_self_, task_flavor_t(MACH_TASK_BASIC_INFO), $0, &count)
            }
        }
        if kerr == KERN_SUCCESS {
            return info.resident_size
        }
        return 0
    }
}
