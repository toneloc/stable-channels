import Foundation
import CryptoKit
import CommonCrypto

enum CryptoError: Error, LocalizedError {
    case keyDerivationFailed
    case encryptionFailed
    case decryptionFailed
    case invalidChecksum
    case invalidFormat
    case invalidMagicBytes
    case unsupportedVersion(UInt16)

    var errorDescription: String? {
        switch self {
        case .keyDerivationFailed: return "Failed to derive encryption key"
        case .encryptionFailed: return "Encryption failed"
        case .decryptionFailed: return "Decryption failed"
        case .invalidChecksum: return "Checksum mismatch"
        case .invalidFormat: return "Invalid backup format"
        case .invalidMagicBytes: return "Not a valid Stable Channels backup file"
        case .unsupportedVersion(let v): return "Unsupported backup version: \(v)"
        }
    }
}

private struct EncryptedPayload: Codable {
    let version: UInt16
    let mnemonic: String
    let createdAt: Date
}

private struct FileHeader {
    static let magicBytes = "SBCKP001"
    static let headerSize = 8 + 2 // magic (8) + version (2)
    static let currentVersion: UInt16 = 1

    let version: UInt16

    func toData() -> Data {
        var data = Data()
        data.append(Self.magicBytes.data(using: .utf8)!)
        withUnsafeBytes(of: version.bigEndian) { data.append(contentsOf: $0) }
        return data
    }

    static func fromData(_ data: Data) -> FileHeader? {
        guard data.count >= headerSize else { return nil }
        let magic = String(data: data.prefix(8), encoding: .utf8)
        guard magic == Self.magicBytes else { return nil }
        // Safe byte-by-byte parsing (no alignment issues)
        let b0 = UInt16(data[8])
        let b1 = UInt16(data[9])
        let version = (b0 << 8) | b1
        return FileHeader(version: version)
    }
}

enum CryptoService {
    // MARK: - Key Derivation

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

    // MARK: - Checksum

    static func checksum(of data: Data) -> String {
        SHA256.hash(data: data).compactMap { String(format: "%02x", $0) }.joined()
    }

    // MARK: - File Export Encryption (passphrase-based)

    static func encrypt(mnemonic: String, passphrase: String) throws -> (data: Data, checksum: String) {
        var salt = Data(count: 32)
        let rngStatus = salt.withUnsafeMutableBytes { saltBytes in
            SecRandomCopyBytes(kSecRandomDefault, 32, saltBytes.baseAddress!)
        }
        guard rngStatus == errSecSuccess else {
            throw CryptoError.encryptionFailed
        }

        let key = try deriveKey(passphrase: passphrase, salt: salt)

        let payload = EncryptedPayload(version: FileHeader.currentVersion, mnemonic: mnemonic, createdAt: Date())
        let payloadData = try JSONEncoder().encode(payload)

        let sealedBox = try AES.GCM.seal(payloadData, using: key)

        guard let encrypted = sealedBox.combined else {
            throw CryptoError.encryptionFailed
        }

        // Build file: header + salt + encrypted
        var result = Data()
        result.append(FileHeader(version: FileHeader.currentVersion).toData())
        result.append(salt)
        result.append(encrypted)

        return (result, checksum(of: result))
    }

    static func decrypt(data: Data, passphrase: String) throws -> BackupFile {
        // Parse header
        guard data.count > FileHeader.headerSize + 32 else {
            throw CryptoError.invalidFormat
        }

        guard let header = FileHeader.fromData(data) else {
            throw CryptoError.invalidMagicBytes
        }

        guard header.version <= FileHeader.currentVersion else {
            throw CryptoError.unsupportedVersion(header.version)
        }

        // Extract salt and encrypted data
        let salt = data.subdata(in: FileHeader.headerSize..<(FileHeader.headerSize + 32))
        let encrypted = data.dropFirst(FileHeader.headerSize + 32)

        let key = try deriveKey(passphrase: passphrase, salt: salt)

        let sealedBox = try AES.GCM.SealedBox(combined: encrypted)
        let decryptedData = try AES.GCM.open(sealedBox, using: key)

        let payload = try JSONDecoder().decode(EncryptedPayload.self, from: decryptedData)
        return BackupFile(
            metadata: BackupMetadata(version: payload.version, checksum: "", timestamp: payload.createdAt,
                                     network: .bitcoin, cipher: .aes256gcm),
            mnemonic: payload.mnemonic,
            createdAt: payload.createdAt
        )
    }

    // MARK: - iCloud Encryption (key-based)

    static func encrypt(mnemonic: String, key: SymmetricKey) throws -> (data: Data, checksum: String) {
        let payload = EncryptedPayload(version: FileHeader.currentVersion, mnemonic: mnemonic, createdAt: Date())
        let payloadData = try JSONEncoder().encode(payload)

        let sealedBox = try AES.GCM.seal(payloadData, using: key)

        guard let combined = sealedBox.combined else {
            throw CryptoError.encryptionFailed
        }

        // Build file: header + encrypted
        var result = Data()
        result.append(FileHeader(version: FileHeader.currentVersion).toData())
        result.append(combined)

        return (result, checksum(of: result))
    }

    static func decrypt(data: Data, key: SymmetricKey) throws -> BackupFile {
        guard data.count > FileHeader.headerSize else {
            throw CryptoError.invalidFormat
        }

        guard let header = FileHeader.fromData(data) else {
            throw CryptoError.invalidMagicBytes
        }

        guard header.version <= FileHeader.currentVersion else {
            throw CryptoError.unsupportedVersion(header.version)
        }

        let encrypted = data.dropFirst(FileHeader.headerSize)

        let sealedBox = try AES.GCM.SealedBox(combined: encrypted)
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
