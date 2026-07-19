import Foundation

private struct TxStatusResponse: Decodable {
    let confirmed: Bool
    let blockHeight: Int?

    enum CodingKeys: String, CodingKey {
        case confirmed
        case blockHeight = "block_height"
    }
}

struct TxConfirmationResolver: TxConfirmationProvider {
    private let client: ResilientEsploraClient

    init(
        chainURLs: [String] = Constants.esploraChainURLs,
        urlSession: URLSession = .shared,
        maxAttempts: Int = 5,
        backoffSeconds: [UInt64] = [1, 4, 16, 64, 256],
        esploraTimeout: TimeInterval = 8,
        wallClockBudgetSeconds: TimeInterval = 900
    ) {
        self.client = ResilientEsploraClient(
            urlSession: urlSession,
            config: .init(
                chainURLs: chainURLs,
                maxAttempts: maxAttempts,
                backoffSeconds: backoffSeconds,
                timeout: esploraTimeout,
                wallClockBudgetSeconds: wallClockBudgetSeconds
            )
        )
    }

    func blockHeight(for txid: String) async throws -> UInt32? {
        return await client.fetch(
            endpointBuilder: { base in
                ["\(ResilientEsploraClient.trimSlash(base))/tx/\(txid)/status"]
            },
            resultParser: { (data: Data) -> UInt32? in
                let status = try JSONDecoder().decode(TxStatusResponse.self, from: data)
                guard status.confirmed, let height = status.blockHeight, height > 0 else { return nil }
                return UInt32(height)
            }
        )
    }
}
