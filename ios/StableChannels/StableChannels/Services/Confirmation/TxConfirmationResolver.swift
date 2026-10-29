import Foundation

private struct TxStatusResponse: Decodable {
    let confirmed: Bool
    let blockHeight: Int?

    enum CodingKeys: String, CodingKey {
        case confirmed
        case blockHeight = "block_height"
    }

    var txStatus: TxStatus {
        guard confirmed, let blockHeight, blockHeight > 0 else { return TxStatus(blockHeight: nil) }
        return TxStatus(blockHeight: UInt32(blockHeight))
    }
}

/// `/tx/:txid` body: the same `status` object `/tx/:txid/status` returns. Outputs are read by
/// `MissedReceiveRecovery.outputsPaying`, the one address-matching rule.
private struct TxResponse: Decodable {
    let status: TxStatusResponse?
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
                wallClockBudgetSeconds: wallClockBudgetSeconds,
                // A receive paid by a large batch must still confirm from its full `/tx` body.
                maxBodyBytes: ResilientEsploraClient.Config.fullTransactionMaxBodyBytes
            )
        )
    }

    func status(for txid: String) async throws -> EsploraLookup<TxStatus> {
        try Task.checkCancellation()
        let result = await client.lookup(
            endpointBuilder: { base in
                ["\(ResilientEsploraClient.trimSlash(base))/tx/\(txid)/status"]
            },
            resultParser: { (data: Data) -> TxStatus? in
                try JSONDecoder().decode(TxStatusResponse.self, from: data).txStatus
            }
        )
        try Task.checkCancellation()
        return result
    }

    /// One `/tx/:txid` request answers both "does it pay `address`" and the confirmation
    /// status, so a receive row needs no second `/status` request.
    func receiveCheck(for txid: String, paysTo address: String) async throws -> EsploraLookup<ReceiveTxCheck> {
        try Task.checkCancellation()
        guard !PaymentDestinationClassifier.normalizeAddress(address).isEmpty else { return .unavailable }
        let result = await client.lookup(
            endpointBuilder: { base in
                ["\(ResilientEsploraClient.trimSlash(base))/tx/\(txid)"]
            },
            resultParser: { (data: Data) -> ReceiveTxCheck? in
                let tx = try JSONDecoder().decode(TxResponse.self, from: data)
                let body = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
                // A body without an output list cannot answer the address question.
                guard let paying = MissedReceiveRecovery.outputsPaying(body, address: address) else { return nil }
                return ReceiveTxCheck(
                    paysToAddress: !paying.isEmpty,
                    status: tx.status?.txStatus
                )
            }
        )
        try Task.checkCancellation()
        return result
    }
}
