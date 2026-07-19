import Foundation

struct BlockHeightResolver: BlockHeightProvider {
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

    func currentHeight() async throws -> UInt32 {
        try await withCheckedThrowingContinuation { cont in
            Task {
                var resolved = false
                await client.run(
                    endpointBuilder: { base in
                        ["\(ResilientEsploraClient.trimSlash(base))/blocks/tip/height"]
                    },
                    resultParser: { (data: Data) -> UInt32? in
                        guard let str = String(data: data, encoding: .utf8)?
                            .trimmingCharacters(in: .whitespacesAndNewlines),
                            let h = UInt32(str) else { return nil }
                        return h
                    },
                    onResolved: { (height: UInt32) in
                        resolved = true
                        cont.resume(returning: height)
                    }
                )
                if !resolved {
                    cont.resume(throwing: EsploraError.invalidResponse)
                }
            }
        }
    }
}
