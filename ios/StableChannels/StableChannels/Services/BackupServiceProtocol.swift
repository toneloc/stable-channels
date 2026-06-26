import Foundation

enum SyncStatus: Equatable {
    case idle
    case syncing
    case synced
    case error(String)
    case iCloudNotAvailable
    case notSupported
}

protocol BackupServiceProtocol {
    var backupExists: Bool { get }
    var syncStatus: SyncStatus { get }
    var iCloudAvailable: Bool { get }
    var lastBackupDate: Date? { get set }

    func checkAccountStatus() async
    func generateAndStoreKey() async throws
    func saveBackupToCloud() async throws
    func restoreFromCloud() async throws -> BackupFile
    func deleteBackup() async throws
    func refreshStatus()
}
