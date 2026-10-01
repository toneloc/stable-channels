import Foundation
import Network
import Security

// MARK: - Secure HTTP Transport Protocol

protocol SecureHTTPTransporting: Sendable {
    func executeGet(url: URL) async throws -> (Data, HTTPURLResponse)
}

// MARK: - NWConnection Session (Lifecycle and Cancellation Manager)

final class NWConnectionSession: @unchecked Sendable {
    private let lock = NSLock()
    private var isFinished = false
    private var buffer = Data()
    private var timerItem: DispatchWorkItem?
    private var selfRetain: NWConnectionSession?
    private let connection: NWConnection
    private let requestData: Data
    private let timeoutInterval: TimeInterval
    private let queue: DispatchQueue
    private let continuation: CheckedContinuation<(data: Data, cleanClose: Bool), Error>

    init(
        connection: NWConnection,
        requestData: Data,
        timeoutInterval: TimeInterval,
        queue: DispatchQueue,
        continuation: CheckedContinuation<(data: Data, cleanClose: Bool), Error>
    ) {
        self.connection = connection
        self.requestData = requestData
        self.timeoutInterval = timeoutInterval
        self.queue = queue
        self.continuation = continuation
    }

    func start() {
        lock.withLock {
            guard !isFinished else { return }
            selfRetain = self
            connection.stateUpdateHandler = { [weak self] state in
                self?.handleState(state)
            }
            let item = DispatchWorkItem { [weak self] in
                self?.complete(with: .failure(LNURLError.networkError("Connection timed out.")))
            }
            timerItem = item
            queue.asyncAfter(deadline: .now() + timeoutInterval, execute: item)
            connection.start(queue: queue)
        }
    }

    func cancel() {
        complete(with: .failure(CancellationError()))
    }

    private func complete(with result: Result<(data: Data, cleanClose: Bool), Error>) {
        lock.withLock {
            guard !isFinished else { return }
            isFinished = true
            timerItem?.cancel()
            timerItem = nil
            connection.stateUpdateHandler = nil
            connection.cancel()
            let retained = selfRetain
            selfRetain = nil
            continuation.resume(with: result)
            _ = retained
        }
    }

    private func handleState(_ state: NWConnection.State) {
        switch state {
        case .ready:
            connection.send(content: requestData, completion: .contentProcessed { [weak self] sendError in
                guard let self else { return }
                if let sendError {
                    self.complete(with: .failure(LNURLError.networkError(sendError.localizedDescription)))
                } else {
                    self.readLoop()
                }
            })
        case let .failed(error), let .waiting(error):
            complete(with: .failure(LNURLError.networkError(error.localizedDescription)))
        case .cancelled:
            complete(with: .failure(CancellationError()))
        default:
            break
        }
    }

    private func readLoop() {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { [weak self] data, _, isComplete, error in
            guard let self else { return }
            if let data, !data.isEmpty {
                self.buffer.append(data)
                if self.buffer.count > HTTPResponseParser.maxResponseBytes {
                    self.complete(with: .failure(LNURLError.invalidResponse))
                    return
                }
            }
            if let error {
                let res: Result<(data: Data, cleanClose: Bool), Error> = self.buffer.isEmpty ?
                    .failure(LNURLError.networkError(error.localizedDescription)) :
                    .success((data: self.buffer, cleanClose: false))
                self.complete(with: res)
            } else if isComplete {
                self.complete(with: .success((data: self.buffer, cleanClose: true)))
            } else {
                self.readLoop()
            }
        }
    }
}

// MARK: - NWConnection Transport (Socket-Pinned Imperative Shell)

final class NWConnectionTransport: SecureHTTPTransporting {
    private let hostResolver: HostIPResolving
    private let timeoutInterval: TimeInterval
    private let onionTransport: SecureHTTPTransporting?
    private let nwQueue = DispatchQueue(label: "org.stablechannels.nwtransport")

    #if DEBUG
        var dialTargetOverride: ((_ endpoint: NWEndpoint, _ params: NWParameters) -> (NWEndpoint, NWParameters))?
    #endif

    init(
        hostResolver: HostIPResolving = SystemHostIPResolver(),
        timeoutInterval: TimeInterval = 15.0,
        onionTransport: SecureHTTPTransporting? = nil
    ) {
        self.hostResolver = hostResolver
        self.timeoutInterval = timeoutInterval
        self.onionTransport = onionTransport
    }

    /// Pure request builder validating CRLF injection and preserving percent encoding.
    static func buildRequest(url: URL, cleanHost: String, portValue: UInt16, defaultPort: UInt16) throws -> Data {
        let path = (url.path(percentEncoded: true).isEmpty ? "/" : url.path(percentEncoded: true)) +
            (url.query(percentEncoded: true).map { "?\($0)" } ?? "")
        let hostStr = cleanHost.contains(":") ? "[\(cleanHost)]" : cleanHost
        let hostHeader = (url.port != nil && portValue != defaultPort) ? "\(hostStr):\(portValue)" : hostStr

        guard path.rangeOfCharacter(from: CharacterSet(charactersIn: "\r\n")) == nil,
              hostHeader.rangeOfCharacter(from: CharacterSet(charactersIn: "\r\n")) == nil,
              let requestData = "GET \(path) HTTP/1.1\r\nHost: \(hostHeader)\r\nUser-Agent: StableChannels\r\nAccept: application/json\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n"
              .data(using: .utf8) else {
            throw LNURLError.invalidResponse
        }
        return requestData
    }

    /// Pure function for selecting the pinned IP address from resolved host IPs.
    static func selectPinnedIP(for host: String, resolvedIPs: [String]) throws -> String {
        let clean = SecureEndpointValidator.cleanHostString(host)
        if let canonical = SecureEndpointValidator.canonicalNumericIP(clean),
           SecureEndpointValidator.evaluateNumericIP(clean) == false {
            return canonical
        }
        guard !resolvedIPs.isEmpty,
              resolvedIPs.allSatisfy({ SecureEndpointValidator.evaluateNumericIP($0) == false }),
              let selected = resolvedIPs.first(where: { $0.contains(".") }) ?? resolvedIPs.first else {
            throw LNURLError.insecureEndpoint
        }
        return selected
    }

    /// Pure function for validating a redirect target against the current request URL.
    static func validateRedirectTarget(
        currentURL: URL,
        locationHeader: String?,
        hop: Int,
        hostResolver: HostIPResolving = SystemHostIPResolver()
    ) throws -> URL {
        guard hop < 3 else {
            throw LNURLError.networkError("Too many redirects.")
        }
        guard let locationHeader,
              let targetURL = URL(string: locationHeader, relativeTo: currentURL)?.absoluteURL else {
            throw LNURLError.invalidResponse
        }
        let currentHost = SecureEndpointValidator.cleanHostString(currentURL.host ?? "")
        let targetHost = SecureEndpointValidator.cleanHostString(targetURL.host ?? "")
        if !currentHost.hasSuffix(".onion") && targetHost.hasSuffix(".onion") {
            throw LNURLError.insecureEndpoint
        }
        guard SecureEndpointValidator.isSecureEndpoint(url: targetURL, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }
        return targetURL
    }

    /// Pure function for constructing dial parameters (NWEndpoint and NWParameters with TLS SNI).
    static func buildDialParameters(
        cleanHost: String,
        vettedIP: String,
        port: NWEndpoint.Port,
        isHTTPS: Bool
    ) -> (endpoint: NWEndpoint, parameters: NWParameters) {
        let tls = isHTTPS ? NWProtocolTLS.Options() : nil
        if let tls, !SecureEndpointValidator.isNumericIP(cleanHost) {
            sec_protocol_options_set_tls_server_name(tls.securityProtocolOptions, cleanHost)
        }
        let params = NWParameters(tls: tls, tcp: NWProtocolTCP.Options())
        let endpoint = NWEndpoint.hostPort(host: NWEndpoint.Host(vettedIP), port: port)
        return (endpoint, params)
    }

    func executeGet(url: URL) async throws -> (Data, HTTPURLResponse) {
        try await executeGetInternal(url: url, hop: 0)
    }

    private func executeGetInternal(url: URL, hop: Int) async throws -> (Data, HTTPURLResponse) {
        try Task.checkCancellation()
        guard hop <= 3 else {
            throw LNURLError.networkError("Too many redirects.")
        }

        let isHTTPS = url.scheme?.lowercased() == "https"
        let defaultPort: UInt16 = isHTTPS ? 443 : 80
        if let rawPort = url.port {
            guard let valid = UInt16(exactly: rawPort), valid > 0 else {
                throw LNURLError.invalidTarget
            }
        } else if URLComponents(url: url, resolvingAgainstBaseURL: false)?.rangeOfPort != nil {
            throw LNURLError.invalidTarget
        }
        let portValue = url.port.flatMap { UInt16(exactly: $0) } ?? defaultPort
        guard portValue > 0 else {
            throw LNURLError.invalidTarget
        }
        let port = NWEndpoint.Port(rawValue: portValue)!

        guard SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        let cleanHost = SecureEndpointValidator.cleanHostString(url.host ?? "")
        if cleanHost.hasSuffix(".onion") {
            guard let onionTransport else {
                throw LNURLError.networkError("Tor .onion endpoints require an onion proxy transport.")
            }
            return try await onionTransport.executeGet(url: url)
        }

        let vettedIP = try Self.selectPinnedIP(for: cleanHost, resolvedIPs: hostResolver.resolveHostIPs(cleanHost))
        var (endpoint, params) = Self.buildDialParameters(
            cleanHost: cleanHost,
            vettedIP: vettedIP,
            port: port,
            isHTTPS: isHTTPS
        )
        #if DEBUG
            if let override = dialTargetOverride {
                (endpoint, params) = override(endpoint, params)
            }
        #endif
        let connection = NWConnection(to: endpoint, using: params)

        let requestData = try Self.buildRequest(
            url: url,
            cleanHost: cleanHost,
            portValue: portValue,
            defaultPort: defaultPort
        )
        let rawResponse = try await sendAndReceive(connection: connection, requestData: requestData)
        let (body, httpResponse) = try HTTPResponseParser.parse(
            data: rawResponse.data,
            url: url,
            cleanClose: rawResponse.cleanClose
        )

        if [301, 302, 303, 307, 308].contains(httpResponse.statusCode) {
            let loc = (httpResponse.allHeaderFields["location"] ?? httpResponse.allHeaderFields["Location"]) as? String
            let target = try Self.validateRedirectTarget(
                currentURL: url,
                locationHeader: loc,
                hop: hop,
                hostResolver: hostResolver
            )
            return try await executeGetInternal(url: target, hop: hop + 1)
        }

        return (body, httpResponse)
    }

    private func sendAndReceive(
        connection: NWConnection,
        requestData: Data
    ) async throws -> (data: Data, cleanClose: Bool) {
        try Task.checkCancellation()
        let holder = SessionHolder()
        return try await withTaskCancellationHandler {
            try await withCheckedThrowingContinuation { continuation in
                let session = NWConnectionSession(
                    connection: connection,
                    requestData: requestData,
                    timeoutInterval: self.timeoutInterval,
                    queue: self.nwQueue,
                    continuation: continuation
                )
                holder.setSession(session)
                session.start()
            }
        } onCancel: {
            holder.cancel()
            connection.cancel()
        }
    }
}

private final class SessionHolder: @unchecked Sendable {
    private let lock = NSLock()
    private var session: NWConnectionSession?
    private var isCancelled = false

    func setSession(_ s: NWConnectionSession) {
        let shouldCancel = lock.withLock { () -> Bool in
            session = s
            return isCancelled
        }
        if shouldCancel {
            s.cancel()
        }
    }

    func cancel() {
        lock.withLock {
            isCancelled = true
            session?.cancel()
        }
    }
}
