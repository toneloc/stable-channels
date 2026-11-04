import Foundation

struct TxConfirmationResolver: TxConfirmationProvider, TxPresenceProviding {
    private let client: ResilientEsploraClient

    /// One quick pass over the explorers, for checks the next tick retries anyway.
    static func singlePass(chainURLs: [String]) -> TxConfirmationResolver {
        TxConfirmationResolver(client: ResilientEsploraClient(config: .singlePass(
            chainURLs: chainURLs,
            maxBodyBytes: ResilientEsploraClient.Config.fullTransactionMaxBodyBytes
        )))
    }

    private init(client: ResilientEsploraClient) {
        self.client = client
    }

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
                try (JSONSerialization.jsonObject(with: data) as? [String: Any]).flatMap(Self.txStatus)
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
                // A body without an output list cannot answer the address question.
                guard let body = try JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let paying = EsploraTransaction.outputsPaying(body, address: address) else { return nil }
                // The same `status` object `/tx/:txid/status` returns; a malformed one is unreadable.
                var status: TxStatus?
                if let raw = body["status"], !(raw is NSNull) {
                    guard let parsed = (raw as? [String: Any]).flatMap(Self.txStatus) else { return nil }
                    status = parsed
                }
                return ReceiveTxCheck(paysToAddress: !paying.isEmpty, status: status)
            }
        )
        try Task.checkCancellation()
        return result
    }

    /// One request per explorer, no retries. Esplora answers 404 for a txid it does not know; the
    /// transaction is gone only when every explorer answered that, since one may lag the others.
    func isKnown(txid: String) async -> Bool? {
        let singlePass = ResilientEsploraClient(
            urlSession: client.urlSession,
            config: .singlePass(chainURLs: client.config.chainURLs)
        )
        let lookup = await singlePass.lookup(
            endpointBuilder: { ["\(ResilientEsploraClient.trimSlash($0))/tx/\(txid)/status"] },
            resultParser: { _ in true }
        )
        switch lookup {
        case .found: return true
        case .notFound: return false
        case .unavailable: return nil
        }
    }

    /// An Esplora `status` object; nil when `confirmed` is missing.
    private static func txStatus(_ json: [String: Any]) -> TxStatus? {
        guard let confirmed = json["confirmed"] as? Bool else { return nil }
        let height = (json["block_height"] as? Int).flatMap(UInt32.init(exactly:)) ?? 0
        return TxStatus(blockHeight: confirmed && height > 0 ? height : nil)
    }
}
