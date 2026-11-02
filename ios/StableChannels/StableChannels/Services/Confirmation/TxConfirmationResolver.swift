import Foundation

private struct TxStatusResponse: Decodable {
    let confirmed: Bool
    let blockHeight: Int?

    enum CodingKeys: String, CodingKey {
        case confirmed
        case blockHeight = "block_height"
    }
}

struct TxConfirmationResolver: TxConfirmationProvider, TxPresenceProviding {
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
        try Task.checkCancellation()
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
        try Task.checkCancellation()
        // Convert the 0 sentinel back to nil for the caller (meaning "pending").
        guard let height = result, height > 0 else { return nil }
        return height
    }

    /// One request per explorer, no retries. Esplora answers 404 for a txid it does not know; the
    /// transaction is gone only when every explorer answered that, since one may lag the others.
    func isKnown(txid: String) async -> Bool? {
        var allNotFound = !client.config.chainURLs.isEmpty
        for base in client.config.chainURLs {
            let path = "\(ResilientEsploraClient.trimSlash(base))/tx/\(txid)/status"
            let status = await client.fetchResponse(path: path)?.statusCode
            if status == 200 { return true }
            if status != 404 { allNotFound = false }
        }
        return allNotFound ? false : nil
    }
}
