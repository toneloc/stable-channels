import Foundation
import CryptoKit
import CommonCrypto

enum CryptoError: Error, LocalizedError {
    case keyDerivationFailed
    case encryptionFailed
    case decryptionFailed
    case invalidChecksum
    case invalidFormat

    var errorDescription: String? {
        switch self {
        case .keyDerivationFailed: return "Failed to derive encryption key"
        case .encryptionFailed: return "Encryption failed"
        case .decryptionFailed: return "Decryption failed"
        case .invalidChecksum: return "Checksum mismatch"
        case .invalidFormat: return "Invalid backup format"
        }
    }
}

private struct EncryptedPayload: Codable {
    let version: UInt16
    let mnemonic: String
    let createdAt: Date
}

enum CryptoService {
    static let magicBytes = "SBCKP001"
    static let currentVersion: UInt16 = 1

    static func deriveKey(passphrase: String, salt: Data) throws -> SymmetricKey {
        guard let passphraseData = passphrase.data(using: .utf8) else {
            throw CryptoError.keyDerivationFailed
        }

        var derivedKey = Data(count: 32)
        let derivationResult = passphraseData.withUnsafeBytes { passphraseBytes -> Int32 in
            salt.withUnsafeBytes { saltBytes -> Int32 in
                derivedKey.withUnsafeMutableBytes { derivedKeyBytes -> Int32 in
                    CCKeyDerivationPBKDF(
                        CCPBKDFAlgorithm(kCCPBKDF2),
                        passphraseBytes.baseAddress,
                        passphraseData.count,
                        saltBytes.baseAddress,
                        salt.count,
                        CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256),
                        310_000,
                        derivedKeyBytes.baseAddress,
                        32
                    )
                }
            }
        }

        guard derivationResult == kCCSuccess else {
            throw CryptoError.keyDerivationFailed
        }

        return SymmetricKey(data: derivedKey)
    }

    static func encrypt(mnemonic: String, passphrase: String) throws -> (data: Data, checksum: String) {
        var salt = Data(count: 32)
        let rngStatus = salt.withUnsafeMutableBytes { saltBytes in
            SecRandomCopyBytes(kSecRandomDefault, 32, saltBytes.baseAddress!)
        }
        guard rngStatus == errSecSuccess else {
            throw CryptoError.encryptionFailed
        }
        let key = try deriveKey(passphrase: passphrase, salt: salt)

        let payload = EncryptedPayload(version: 1, mnemonic: mnemonic, createdAt: Date())
        let payloadData = try JSONEncoder().encode(payload)

        let sealedBox = try AES.GCM.seal(payloadData, using: key)

        var result = Data()
        result.append(salt)
        result.append(sealedBox.combined!)

        return (result, checksum(of: result))
    }

    static func decrypt(data: Data, passphrase: String) throws -> BackupFile {
        guard data.count > 32 + 12 + 16 else {
            throw CryptoError.invalidFormat
        }

        let salt = data.prefix(32)
        let combined = data.dropFirst(32)

        let key = try deriveKey(passphrase: passphrase, salt: Data(salt))

        let sealedBox = try AES.GCM.SealedBox(combined: combined)
        let decryptedData = try AES.GCM.open(sealedBox, using: key)

        let payload = try JSONDecoder().decode(EncryptedPayload.self, from: decryptedData)
        return BackupFile(
            metadata: BackupMetadata(version: payload.version, checksum: "", timestamp: payload.createdAt,
                                     network: .bitcoin, cipher: .aes256gcm),
            mnemonic: payload.mnemonic,
            createdAt: payload.createdAt
        )
    }

    static func checksum(of data: Data) -> String {
        SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Key-based encryption (for iCloud - key from Keychain)

    static func encrypt(mnemonic: String, key: SymmetricKey) throws -> (data: Data, checksum: String) {
        let payload = EncryptedPayload(version: 1, mnemonic: mnemonic, createdAt: Date())
        let payloadData = try JSONEncoder().encode(payload)
        let sealedBox = try AES.GCM.seal(payloadData, using: key)
        guard let combined = sealedBox.combined else {
            throw CryptoError.encryptionFailed
        }
        return (combined, checksum(of: combined))
    }

    static func decrypt(data: Data, key: SymmetricKey) throws -> BackupFile {
        let sealedBox = try AES.GCM.SealedBox(combined: data)
        let decryptedData = try AES.GCM.open(sealedBox, using: key)
        let payload = try JSONDecoder().decode(EncryptedPayload.self, from: decryptedData)
        return BackupFile(
            metadata: BackupMetadata(version: payload.version, checksum: "", timestamp: payload.createdAt,
                                     network: .bitcoin, cipher: .aes256gcm),
            mnemonic: payload.mnemonic,
            createdAt: payload.createdAt
        )
    }
}
