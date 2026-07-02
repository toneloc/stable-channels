import Foundation

enum BackupError: Error, LocalizedError {
    case backupCorrupted
    case backupNotFound
    case checksumMismatch
    case iCloudNotSignedIn
    case invalidFormat
    case rateLimitExceeded(remainingSeconds: Int)

    var errorDescription: String? {
        switch self {
        case .backupCorrupted: return "Backup is corrupted"
        case .backupNotFound: return "Backup not found"
        case .checksumMismatch: return "Backup file is corrupted"
        case .iCloudNotSignedIn: return "Sign in to iCloud to enable backup"
        case .invalidFormat: return "Invalid backup file format"
        case .rateLimitExceeded(let secs): return "Too many attempts. Try again in \(secs) seconds"
        }
    }
}
