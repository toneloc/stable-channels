import Foundation

/// File-based audit logging — port of src/audit.rs
enum AuditService {
    private static var logPath: String?

    static func setLogPath(_ path: String) {
        logPath = path
    }

    static func log(_ event: String, data: [String: Any]) {
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
            "data": data,
        ]

        guard let jsonData = try? JSONSerialization.data(withJSONObject: logEntry),
              let jsonStr = String(data: jsonData, encoding: .utf8) else {
            return
        }

        let line = jsonStr + "\n"
        if let fileHandle = FileHandle(forWritingAtPath: path) {
            fileHandle.seekToEndOfFile()
            fileHandle.write(line.data(using: .utf8) ?? Data())
            fileHandle.closeFile()
        } else {
            try? line.write(toFile: path, atomically: true, encoding: .utf8)
        }
    }
}
