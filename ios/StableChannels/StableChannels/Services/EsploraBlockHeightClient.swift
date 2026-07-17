import Foundation

enum EsploraError: LocalizedError {
    case invalidResponse
    var errorDescription: String? { "Invalid response from Esplora" }
}

struct EsploraBlockHeightClient: BlockHeightProvider {
    let baseURL: URL
    let urlSession: URLSession

    init(baseURL: URL, urlSession: URLSession = .shared) {
        self.baseURL = baseURL
        self.urlSession = urlSession
    }

    func currentHeight() async throws -> UInt32 {
        let url = baseURL.appendingPathComponent("blocks/tip/height")
        let (data, _) = try await urlSession.data(from: url)
        guard let str = String(data: data, encoding: .utf8)?.trimmingCharacters(in: .whitespacesAndNewlines),
              let h = UInt32(str) else {
            throw EsploraError.invalidResponse
        }
        return h
    }
}
