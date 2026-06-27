import Foundation
import CloudKit
import CryptoKit
import LDKNode

@MainActor
@Observable
final class CloudBackupService: BackupServiceProtocol {
    static let shared: BackupServiceProtocol = {
        let service = CloudBackupService(nodeService: NodeService.shared)
        service.initialize()
        return service
    }()

    private let recordType = "SeedBackup"
    private let recordIDName = "latestSeedBackup"

    private let keychain = KeychainService.shared
    private let nodeService: NodeServiceProtocol

    private(set) var backupExists: Bool = false
    private(set) var hasLocalBackup: Bool = UserDefaults.standard.bool(forKey: "backupEnabled")
    private(set) var syncStatus: SyncStatus = .idle
    private(set) var iCloudAvailable: Bool = false

    init(nodeService: NodeServiceProtocol) {
        self.nodeService = nodeService
    }

    func initialize() {
        Task { await checkAccountStatus() }
    }

    // MARK: - iCloud Account

    func checkAccountStatus() async {
        do {
            let container = CKContainer.default()
            let status = try await container.accountStatus()
            iCloudAvailable = status == .available
            syncStatus = status == .available ? .idle : .iCloudNotAvailable

            if iCloudAvailable {
                await detectExistingBackup()
            }
        } catch {
            iCloudAvailable = false
            syncStatus = .notSupported
        }
    }

    private func detectExistingBackup() async {
        let container = CKContainer.default()
        let db = container.privateCloudDatabase
        let recordID = CKRecord.ID(recordName: recordIDName)

        do {
            let record = try await db.record(for: recordID)
            backupExists = true
            if let timestamp = record["timestamp"] as? Date {
                lastBackupDate = timestamp
            }
        } catch {
            // Record does not exist or network failure
        }
    }

    func checkRemoteBackupExists() async -> Bool {
        let container = CKContainer.default()
        let db = container.privateCloudDatabase
        let recordID = CKRecord.ID(recordName: recordIDName)

        do {
            _ = try await db.record(for: recordID)
            return true
        } catch {
            return false
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
            throw KeychainError.keychainUnavailable
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
        UserDefaults.standard.set(true, forKey: "backupEnabled")
        hasLocalBackup = true
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
            throw BackupError.backupNotFound
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
        syncStatus = .idle

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
        if iCloudAvailable {
            let container = CKContainer.default()
            let db = container.privateCloudDatabase
            let recordID = CKRecord.ID(recordName: recordIDName)
            do {
                try await db.deleteRecord(withID: recordID)
            } catch let ckError as CKError where ckError.code == .unknownItem {}
        }

        keychain.deleteKey()

        backupExists = false
        UserDefaults.standard.set(false, forKey: "backupEnabled")
        hasLocalBackup = false
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

    func markLocalBackupAsEnabled() {
        backupExists = true
        UserDefaults.standard.set(true, forKey: "backupEnabled")
        hasLocalBackup = true
        syncStatus = .synced
    }
}
