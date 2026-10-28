import SwiftUI
import UniformTypeIdentifiers

struct DiagnosticsSettingsView: View {
    @State private var isExporting = false
    @State private var logDocument: LogTextDocument? = nil

    var body: some View {
        List {
            Section {
                let logUrls = Constants.exportableLogURLs()
                if !logUrls.isEmpty {
                    ShareLink(items: logUrls) {
                        HStack {
                            Image(systemName: "square.and.arrow.up")
                                .foregroundStyle(.green)
                            Text(String(localized: .buttonShareLogs))
                                .foregroundStyle(.primary)
                        }
                    }
                    Button {
                        var allLogs = ""
                        for url in logUrls {
                            if let content = try? String(contentsOf: url) {
                                allLogs += "=== \(url.lastPathComponent) ===\n\(content)\n\n"
                            }
                        }
                        logDocument = LogTextDocument(text: allLogs)
                        isExporting = true
                    } label: {
                        HStack {
                            Image(systemName: "arrow.down.doc")
                                .foregroundStyle(.green)
                            Text(String(localized: .buttonDownloadLogs))
                                .foregroundStyle(.primary)
                        }
                    }
                } else {
                    Button {} label: {
                        HStack {
                            Image(systemName: "square.and.arrow.up")
                            Text(String(localized: .buttonShareLogs))
                        }
                        .foregroundStyle(.gray)
                    }.disabled(true)
                    Button {} label: {
                        HStack {
                            Image(systemName: "arrow.down.doc")
                            Text(String(localized: .buttonDownloadLogs))
                        }
                        .foregroundStyle(.gray)
                    }.disabled(true)
                }
            } footer: {
                Text(String(localized: .footerSaveLogs))
            }
        }
        .navigationTitle(String(localized: .titleLogsDiagnostics))
        .navigationBarTitleDisplayMode(.inline)
        .fileExporter(
            isPresented: $isExporting,
            document: logDocument,
            contentType: .plainText,
            defaultFilename: "stable_channels_logs.txt"
        ) { result in
            switch result {
            case .success(let url):
                print("Saved to \(url)")
            case .failure(let error):
                print("Failed to save: \(error.localizedDescription)")
            }
        }
    }
}

struct LogTextDocument: FileDocument {
    static var readableContentTypes: [UTType] { [.plainText] }
    var text: String

    init(text: String) {
        self.text = text
    }

    init(configuration: ReadConfiguration) throws {
        if let data = configuration.file.regularFileContents,
           let string = String(data: data, encoding: .utf8) {
            text = string
        } else {
            text = ""
        }
    }

    func fileWrapper(configuration _: WriteConfiguration) throws -> FileWrapper {
        let data = text.data(using: .utf8) ?? Data()
        return FileWrapper(regularFileWithContents: data)
    }
}
