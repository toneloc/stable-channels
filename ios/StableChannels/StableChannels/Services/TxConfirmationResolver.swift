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
        let result = await client.fetch(
            endpointBuilder: { base in
                ["\(ResilientEsploraClient.trimSlash(base))/tx/\(txid)/status"]
            },
            resultParser: { (data: Data) -> UInt32? in
                let status = try JSONDecoder().decode(TxStatusResponse.self, from: data)
                // Return 0 sentinel for valid "not confirmed yet" responses so
                // the client stops immediately instead of retrying with backoff.
                // nil is reserved for actual parse failures that warrant a retry.
                guard status.confirmed, let height = status.blockHeight, height > 0 else { return UInt32(0) }
                return UInt32(height)
            }
        )
        // Convert the 0 sentinel back to nil for the caller (meaning "pending").
        guard let height = result, height > 0 else { return nil }
        return height
    }
}
