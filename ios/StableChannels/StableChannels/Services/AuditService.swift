import Foundation

/// File-based audit logging — port of src/audit.rs
enum AuditService {
    private static var logPath: String?
    static var onLog: ((String, [String: Any]) -> Void)?

    /// Size at which the log rotates. The previous file is kept as `<name>.1.<ext>`, so the
    /// log never uses much more than twice this on disk, however many events arrive.
    static var maxFileBytes: UInt64 = 5 * 1024 * 1024

    /// Where the previous log goes when `path` rotates: `audit_log.txt` -> `audit_log.1.txt`.
    static func rotatedPath(for path: String) -> String {
        let url = URL(fileURLWithPath: path)
        let ext = url.pathExtension
        let base = url.deletingPathExtension().lastPathComponent
        let name = ext.isEmpty ? "\(base).1" : "\(base).1.\(ext)"
        return url.deletingLastPathComponent().appendingPathComponent(name).path
    }

    static func setLogPath(_ path: String) {
        logPath = path
    }

    static func log(_ event: String, data: [String: Any]) {
        onLog?(event, data)
        guard let path = logPath else { return }

        let url = URL(fileURLWithPath: path)
        if let parent = url.deletingLastPathComponent() as URL? {
            try? FileManager.default.createDirectory(at: parent, withIntermediateDirectories: true)
        }

        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]

        let logEntry: [String: Any] = [
            "ts": formatter.string(from: Date()),
            "event": event,
            "data": data
        ]

        guard let jsonData = try? JSONSerialization.data(withJSONObject: logEntry),
              let jsonStr = String(data: jsonData, encoding: .utf8) else {
            return
        }

        let line = jsonStr + "\n"
        rotateIfFull(path)
        if let fileHandle = FileHandle(forWritingAtPath: path) {
            fileHandle.seekToEndOfFile()
            fileHandle.write(line.data(using: .utf8) ?? Data())
            fileHandle.closeFile()
        } else {
            try? line.write(toFile: path, atomically: true, encoding: .utf8)
        }
    }

    /// Moves a full log aside, replacing the previous rotation, so writes start a fresh file.
    private static func rotateIfFull(_ path: String) {
        let fileManager = FileManager.default
        guard let size = (try? fileManager.attributesOfItem(atPath: path))?[.size] as? UInt64,
              size >= maxFileBytes else { return }
        let rotated = rotatedPath(for: path)
        try? fileManager.removeItem(atPath: rotated)
        try? fileManager.moveItem(atPath: path, toPath: rotated)
    }
}
