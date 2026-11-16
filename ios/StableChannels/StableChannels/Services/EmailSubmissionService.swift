import Foundation

/// Saves or removes the email address the user chose to share with the Stable Channels server.
protocol EmailSubmitting {
    /// Saves `email` for this wallet's node, or removes the saved one when `email` is nil.
    func submit(_ email: EmailAddress?) async throws
}

enum EmailSubmissionError: Error, Equatable {
    case walletNotReady
    case unreachable
    case rejected(statusCode: Int)
}

/// Signs email requests with the wallet's Lightning node key so the server knows which node sent them.
protocol EmailRequestSigning: AnyObject {
    var isRunning: Bool { get }
    var nodeId: String { get }
    func signMessage(_ message: [UInt8]) throws -> String
}

extension NodeService: EmailRequestSigning {}

/// Builds the signed `REGISTER_EMAIL_V1` request body, the same envelope shape as the keysend messages.
enum EmailRegistrationEnvelope {
    /// Whether the server's reply body says `"ok": true`; anything else is a refusal, so it is never shown as saved.
    static func isAccepted(_ body: Data) -> Bool {
        (try? JSONSerialization.jsonObject(with: body) as? [String: Any])?["ok"] as? Bool == true
    }

    /// The server refuses a `ts` not newer than the last one it stored for the node, so never reuse or go back.
    /// A stored `ts` beyond the server window came from a clock that ran ahead; reusing it would lock saves out.
    static func nextTimestamp(now: Date, lastSent: Int64) -> Int64 {
        let current = Int64(now.timeIntervalSince1970)
        guard lastSent + 1 <= current + Constants.emailServerTimestampWindowSeconds else { return current }
        return max(current, lastSent + 1)
    }

    /// Sorted-key JSON of the payload; an empty `email` asks the server to remove the saved address.
    static func payload(email: String, nodeId: String, timestamp: Int64) -> String? {
        let object: [String: Any] = [
            "email": email,
            "node_id": nodeId,
            "ts": timestamp,
            "type": Constants.emailRegisterMessageType
        ]
        return json(object)
    }

    static func body(payload: String, signature: String) -> Data? {
        json(["payload": payload, "signature": signature]).map { Data($0.utf8) }
    }

    private static func json(_ object: [String: Any]) -> String? {
        guard let data = try? JSONSerialization.data(
            withJSONObject: object,
            options: [.sortedKeys, .withoutEscapingSlashes]
        ) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

struct EmailSubmissionService: EmailSubmitting {
    let signer: EmailRequestSigning
    let timestamps: EmailTimestampStoring
    let endpoint: String
    let timeout: TimeInterval
    let configuration: URLSessionConfiguration
    let now: () -> Date

    init(
        signer: EmailRequestSigning,
        timestamps: EmailTimestampStoring = EmailPromptStore.shared,
        endpoint: String = Constants.emailSubmitURL,
        timeout: TimeInterval = Constants.emailSubmitTimeoutSeconds,
        configuration: URLSessionConfiguration = .ephemeral,
        now: @escaping () -> Date = { Date() }
    ) {
        self.signer = signer
        self.timestamps = timestamps
        self.endpoint = endpoint
        self.timeout = timeout
        self.configuration = configuration
        self.now = now
    }

    func submit(_ email: EmailAddress?) async throws {
        let nodeId = signer.nodeId
        guard signer.isRunning, !nodeId.isEmpty else { throw EmailSubmissionError.walletNotReady }
        let timestamp = EmailRegistrationEnvelope.nextTimestamp(now: now(), lastSent: timestamps.lastSentTimestamp)
        guard let url = URL(string: endpoint),
              let payload = EmailRegistrationEnvelope.payload(
                  email: email?.value ?? "",
                  nodeId: nodeId,
                  timestamp: timestamp
              ) else { throw EmailSubmissionError.unreachable }

        let signature: String
        do {
            signature = try signer.signMessage(Array(payload.utf8))
        } catch {
            throw EmailSubmissionError.walletNotReady
        }
        // Stored before sending: the server may record it even when the reply never arrives.
        timestamps.lastSentTimestamp = timestamp

        var request = URLRequest(url: url, timeoutInterval: timeout)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.httpBody = EmailRegistrationEnvelope.body(payload: payload, signature: signature)

        let session = URLSession(configuration: configuration)
        defer { session.finishTasksAndInvalidate() }
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await session.data(for: request)
        } catch {
            throw EmailSubmissionError.unreachable
        }

        let statusCode = (response as? HTTPURLResponse)?.statusCode ?? 0
        guard (200 ..< 300).contains(statusCode), EmailRegistrationEnvelope.isAccepted(data) else {
            throw EmailSubmissionError.rejected(statusCode: statusCode)
        }
    }
}
