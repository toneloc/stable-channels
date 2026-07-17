import Foundation

struct EsploraTxConfirmationClient: TxConfirmationProvider {
    let baseURL: URL
    let urlSession: URLSession

    init(baseURL: URL, urlSession: URLSession = .shared) {
        self.baseURL = baseURL
        self.urlSession = urlSession
    }

    func blockHeight(for txid: String) async throws -> UInt32? {
        let url = baseURL.appendingPathComponent("tx/\(txid)/status")
        let (data, response) = try await urlSession.data(from: url)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200,
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw EsploraError.invalidResponse
        }

        guard let confirmed = json["confirmed"] as? Bool, confirmed else {
            return nil
        }
        guard let raw = json["block_height"] as? Int, raw > 0 else {
            return nil
        }
        return UInt32(raw)
    }
}
