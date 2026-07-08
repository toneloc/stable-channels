import Foundation
import UserNotifications

/// Logging infrastructure
protocol Logger {
    func log(_ message: String)
}

struct FileLogger: Logger {
    private let appGroup: String
    private let fileName: String

    init(appGroup: String, fileName: String = "nse_debug.log") {
        self.appGroup = appGroup
        self.fileName = fileName
    }

    func log(_ message: String) {
        NSLog("[NSE] \(message)")
        guard let container = FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: appGroup) else { return }
        let logFile = container.appendingPathComponent(fileName)
        let line = "\(Date()): \(message)\n"
        if let data = line.data(using: .utf8) {
            if let handle = try? FileHandle(forWritingTo: logFile) {
                handle.seekToEndOfFile()
                handle.write(data)
                handle.closeFile()
            } else {
                try? data.write(to: logFile)
            }
        }
    }
}
