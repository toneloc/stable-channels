import Foundation

enum BackupCipher: UInt8, Codable {
    case aes256gcm = 0
}

enum BackupNetwork: UInt8, Codable {
    case bitcoin = 0
    case testnet = 1
}

struct BackupMetadata: Codable {
    let version: UInt16
    /// Hex-encoded SHA-256 checksum of encrypted payload
    let checksum: String
    let timestamp: Date
    let network: BackupNetwork
    let cipher: BackupCipher

    static let currentVersion: UInt16 = 1
}

struct BackupFile: Codable {
    let metadata: BackupMetadata
    let mnemonic: String
    let createdAt: Date
}

struct EncryptedBackup: Codable {
    let version: UInt16
    let mnemonic: String
    let createdAt: Date
}
