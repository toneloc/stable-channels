import Foundation

/// Settlement outcome of an in-flight payment.
enum PaymentSettlementOutcome: Sendable {
    case settled(paymentHash: String?)
    case failed(reason: String)
    case timedOut
}

/// Asynchronous observer for payment settlement events emitted over NotificationCenter or persisted in the database.
@MainActor
enum PaymentSettlementObserver {
    static func awaitSettlement(
        paymentId: String,
        timeoutSeconds: TimeInterval,
        appState: AppState? = nil
    ) async -> PaymentSettlementOutcome {
        // Fast-path: check if payment settled or failed before observer was attached
        if let immediate = checkDatabaseRecord(paymentId: paymentId, appState: appState) {
            return immediate
        }

        var settledObserver: (any NSObjectProtocol)?
        var failedObserver: (any NSObjectProtocol)?

        let stream = AsyncStream<PaymentSettlementOutcome> { continuation in
            settledObserver = NotificationCenter.default.addObserver(
                forName: .paymentSettled,
                object: nil,
                queue: .main
            ) { note in
                guard let pid = note.userInfo?["paymentId"] as? String, pid == paymentId else { return }
                let hash = note.userInfo?["paymentHash"] as? String
                continuation.yield(.settled(paymentHash: hash))
                continuation.finish()
            }

            failedObserver = NotificationCenter.default.addObserver(
                forName: .paymentFailed,
                object: nil,
                queue: .main
            ) { note in
                guard let pid = note.userInfo?["paymentId"] as? String, pid == paymentId else { return }
                let reason = note.userInfo?["errorMessage"] as? String
                    ?? note.userInfo?["reason"] as? String
                    ?? "The payment did not complete. Check its status in History before trying again."
                continuation.yield(.failed(reason: reason))
                continuation.finish()
            }

            continuation.onTermination = { @Sendable _ in }
        }

        defer {
            if let obs = settledObserver { NotificationCenter.default.removeObserver(obs) }
            if let obs = failedObserver { NotificationCenter.default.removeObserver(obs) }
        }

        // Re-check database once observers are registered to close the registration race window
        if let immediate = checkDatabaseRecord(paymentId: paymentId, appState: appState) {
            return immediate
        }

        return await withTaskGroup(of: PaymentSettlementOutcome.self) { group in
            group.addTask {
                for await outcome in stream {
                    return outcome
                }
                return .timedOut
            }

            group.addTask {
                try? await Task.sleep(nanoseconds: UInt64(timeoutSeconds * 1_000_000_000))
                return .timedOut
            }

            let result = await group.next() ?? .timedOut
            group.cancelAll()
            return result
        }
    }

    private static func checkDatabaseRecord(
        paymentId: String,
        appState: AppState?
    ) -> PaymentSettlementOutcome? {
        guard let record = appState?.databaseService?.paymentRepo.payment(paymentId: paymentId) else {
            return nil
        }
        if record.status == "completed" || record.status == "succeeded" {
            return .settled(paymentHash: record.paymentId)
        } else if record.status == "failed" {
            return .failed(reason: "The payment did not complete. Check its status in History before trying again.")
        }
        return nil
    }
}
