import Foundation
import CloudKit
import CryptoKit

@MainActor
@Observable
final class CloudBackupService {
    static let shared = CloudBackupService(nodeService: NodeService.shared)

    private let recordType = "SeedBackup"
    private let recordIDName = "latestSeedBackup"

    private let keychain = KeychainService.shared
    private let nodeService: NodeService

    private(set) var backupExists: Bool = false
    private(set) var syncStatus: SyncStatus = .idle
    private(set) var iCloudAvailable: Bool = false

    init(nodeService: NodeService) {
        self.nodeService = nodeService
        Task { await checkAccountStatus() }
    }

    enum SyncStatus: Equatable {
        case idle
        case syncing
        case synced
        case error(String)
        case iCloudNotAvailable
        case notSupported
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

        let (encryptedData, checksum) = try CryptoService.encrypt(mnemonic: mnemonic, key: key)

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

        let backup = try CryptoService.decrypt(data: encryptedData, key: key)
        backupExists = true
        syncStatus = .synced

        // Return with CloudKit-specific data merged
        return BackupFile(
            metadata: BackupMetadata(
                version: backup.metadata.version,
                checksum: checksum,
                timestamp: timestamp,
                network: .bitcoin,
                cipher: .aes256gcm
            ),
            mnemonic: backup.mnemonic,
            createdAt: backup.createdAt
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
        nodeService.savedMnemonic
    }

    func refreshStatus() {
        Task { await checkAccountStatus() }
    }

    var lastBackupDate: Date? {
        get { UserDefaults.standard.object(forKey: "lastBackupDate") as? Date }
        set { UserDefaults.standard.set(newValue, forKey: "lastBackupDate") }
    }
}
