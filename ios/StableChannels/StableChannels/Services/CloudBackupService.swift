import Foundation
import CloudKit
import CryptoKit

@MainActor
@Observable
final class CloudBackupService {
    static let shared = CloudBackupService()

    private let recordType = "SeedBackup"
    private let recordIDName = "latestSeedBackup"

    private let keychain = KeychainService.shared

    private(set) var backupExists: Bool = false
    private(set) var syncStatus: SyncStatus = .idle
    private(set) var iCloudAvailable: Bool = false

    enum SyncStatus: Equatable {
        case idle
        case syncing
        case synced
        case error(String)
        case iCloudNotAvailable
        case notSupported
    }

    init() {
        Task { await checkAccountStatus() }
    }

    // MARK: - iCloud Account

    func checkAccountStatus() async {
        do {
            let container = CKContainer.default()
            let status = try await container.accountStatus()
            iCloudAvailable = status == .available
            syncStatus = status == .available ? .idle : .iCloudNotAvailable
        } catch {
            iCloudAvailable = false
            syncStatus = .notSupported
        }
    }

    // MARK: - Key Generation (First Enable)

    func generateAndStoreKey() async throws {
        try throwIfUnavailable()
        try keychain.generateAndStoreKey()
        try await saveBackupToCloud()
    }

    private func throwIfUnavailable() throws {
        if !iCloudAvailable {
            throw BackupError.iCloudNotSignedIn
        }
    }

    // MARK: - Backup Operations

    func saveBackupToCloud() async throws {
        try throwIfUnavailable()
        syncStatus = .syncing

        guard let mnemonic = getMnemonic() else {
            syncStatus = .error("No mnemonic available")
            throw BackupError.keychainUnavailable
        }

        let keyData = try keychain.loadKey()
        let key = SymmetricKey(data: keyData)

        let payload = EncryptedBackup(version: 1, mnemonic: mnemonic, createdAt: Date())
        let payloadData = try JSONEncoder().encode(payload)
        let sealedBox = try AES.GCM.seal(payloadData, using: key)
        guard let encryptedData = sealedBox.combined else {
            syncStatus = .error("Encryption failed")
            throw BackupError.exportFailed("AES.GCM seal produced no output")
        }

        let checksum = SHA256.hash(data: encryptedData).compactMap { String(format: "%02x", $0) }.joined()

        let container = CKContainer.default()
        let db = container.privateCloudDatabase

        let recordID = CKRecord.ID(recordName: recordIDName)
        let record: CKRecord
        do {
            record = try await db.record(for: recordID)
        } catch {
            record = CKRecord(recordType: recordType, recordID: recordID)
        }

        record["encryptedData"] = encryptedData as CKRecordValue
        record["checksum"] = checksum as CKRecordValue
        record["timestamp"] = Date() as CKRecordValue
        record["version"] = 1 as CKRecordValue

        try await db.save(record)

        backupExists = true
        lastBackupDate = Date()
        syncStatus = .synced
    }

    // MARK: - Restore from iCloud

    func restoreFromCloud() async throws -> BackupFile {
        try throwIfUnavailable()
        syncStatus = .syncing

        let keyData = try keychain.loadKey()
        let key = SymmetricKey(data: keyData)

        let container = CKContainer.default()
        let db = container.privateCloudDatabase

        let recordID = CKRecord.ID(recordName: recordIDName)
        let record: CKRecord
        do {
            record = try await db.record(for: recordID)
        } catch let ckError as CKError where ckError.code == .unknownItem {
            syncStatus = .error("Backup not found")
            throw BackupError.backupCorrupted
        }

        guard let encryptedData = record["encryptedData"] as? Data,
              let checksum = record["checksum"] as? String,
              let timestamp = record["timestamp"] as? Date else {
            syncStatus = .error("Invalid format")
            throw BackupError.invalidFormat
        }

        let computed = SHA256.hash(data: encryptedData).compactMap { String(format: "%02x", $0) }.joined()
        guard computed == checksum else {
            syncStatus = .error("Checksum mismatch")
            throw BackupError.checksumMismatch
        }

        let sealedBox = try AES.GCM.SealedBox(combined: encryptedData)
        let decryptedData = try AES.GCM.open(sealedBox, using: key)
        let decrypted = try JSONDecoder().decode(EncryptedBackup.self, from: decryptedData)

        syncStatus = .synced

        return BackupFile(
            metadata: BackupMetadata(
                version: 1,
                checksum: checksum,
                timestamp: timestamp,
                network: .bitcoin,
                cipher: .aes256gcm
            ),
            mnemonic: decrypted.mnemonic,
            createdAt: decrypted.createdAt
        )
    }

    // MARK: - Delete

    func deleteBackup() async throws {
        keychain.deleteKey()

        if iCloudAvailable {
            let container = CKContainer.default()
            let db = container.privateCloudDatabase
            let recordID = CKRecord.ID(recordName: recordIDName)
            do {
                try await db.deleteRecord(withID: recordID)
            } catch let ckError as CKError where ckError.code == .unknownItem {}
        }

        backupExists = false
        lastBackupDate = nil
        syncStatus = .idle
    }

    // MARK: - Helpers

    private func getMnemonic() -> String? {
        NodeService().savedMnemonic
    }

    func refreshStatus() {
        Task { await checkAccountStatus() }
    }

    var lastBackupDate: Date? {
        get { UserDefaults.standard.object(forKey: "lastBackupDate") as? Date }
        set { UserDefaults.standard.set(newValue, forKey: "lastBackupDate") }
    }
}
