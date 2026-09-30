import Foundation
import Network
import Security

// MARK: - Secure HTTP Transport Protocol

protocol SecureHTTPTransporting: Sendable {
    func executeGet(url: URL) async throws -> (Data, HTTPURLResponse)
}

// MARK: - HTTP Response Parser (Pure Functional Core)

enum HTTPResponseParser {
    static func parse(data: Data, url: URL) throws -> (Data, HTTPURLResponse) {
        guard let crlf2Range = data.range(of: Data([0x0D, 0x0A, 0x0D, 0x0A])) else {
            throw LNURLError.invalidResponse
        }
        let headerData = data[..<crlf2Range.lowerBound]
        let rawBody = data[crlf2Range.upperBound...]

        guard let headerStr = String(data: headerData, encoding: .utf8) else {
            throw LNURLError.invalidResponse
        }
        let lines = headerStr.components(separatedBy: "\r\n")
        guard let statusLine = lines.first else {
            throw LNURLError.invalidResponse
        }
        let statusParts = statusLine.split(separator: " ")
        guard statusParts.count >= 2, let statusCode = Int(statusParts[1]) else {
            throw LNURLError.invalidResponse
        }

        var headers: [String: String] = [:]
        for line in lines.dropFirst() {
            guard let colon = line.firstIndex(of: ":") else { continue }
            let key = String(line[..<colon]).trimmingCharacters(in: .whitespaces)
            let val = String(line[line.index(after: colon)...]).trimmingCharacters(in: .whitespaces)
            headers[key.lowercased()] = val
        }

        let body: Data
        if headers["transfer-encoding"]?.lowercased().contains("chunked") == true {
            guard let decoded = decodeChunked(data: Data(rawBody)) else {
                throw LNURLError.invalidResponse
            }
            body = decoded
        } else {
            body = Data(rawBody)
        }

        guard let response = HTTPURLResponse(
            url: url,
            statusCode: statusCode,
            httpVersion: "HTTP/1.1",
            headerFields: headers
        ) else {
            throw LNURLError.invalidResponse
        }
        return (body, response)
    }

    private static func decodeChunked(data: Data) -> Data? {
        var result = Data()
        var offset = 0
        let count = data.count
        while offset < count {
            guard let crlf = data[offset...].range(of: Data([0x0D, 0x0A])) else { return nil }
            guard let hex = String(data: data[offset..<crlf.lowerBound], encoding: .utf8)?
                .trimmingCharacters(in: .whitespaces) else { return nil }
            let cleanHex = hex.split(separator: ";").first.map(String.init) ?? hex
            guard let size = Int(cleanHex, radix: 16) else { return nil }
            if size == 0 { return result }
            let start = crlf.upperBound
            let end = start + size
            guard end <= count else { return nil }
            result.append(data[start..<end])
            offset = end + 2
        }
        return result
    }
}

// MARK: - NWConnection Transport (Socket-Pinned Imperative Shell)

final class NWConnectionTransport: SecureHTTPTransporting {
    private let hostResolver: HostIPResolving
    private let timeoutInterval: TimeInterval

    init(
        hostResolver: HostIPResolving = SystemHostIPResolver(),
        timeoutInterval: TimeInterval = 15.0
    ) {
        self.hostResolver = hostResolver
        self.timeoutInterval = timeoutInterval
    }

    func executeGet(url: URL) async throws -> (Data, HTTPURLResponse) {
        try await executeGetInternal(url: url, hop: 0)
    }

    private func executeGetInternal(url: URL, hop: Int) async throws -> (Data, HTTPURLResponse) {
        guard hop <= 3 else { throw LNURLError.insecureEndpoint }
        guard SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        let rawHost = url.host ?? ""
        let cleanHost = SecureEndpointValidator.cleanHostString(rawHost)
        let vettedIP: String
        if SecureEndpointValidator.evaluateNumericIP(cleanHost) == false {
            vettedIP = cleanHost
        } else {
            let ips = hostResolver.resolveHostIPs(cleanHost)
            let allowedIPs = ips.filter { !SecureEndpointValidator.isPrivateOrLoopbackHost($0) }
            guard let selected = allowedIPs.first(where: { $0.contains(".") }) ?? allowedIPs.first else {
                throw LNURLError.insecureEndpoint
            }
            vettedIP = selected
        }

        let isHTTPS = url.scheme?.lowercased() == "https"
        let portValue = UInt16(url.port ?? (isHTTPS ? 443 : 80))
        guard let port = NWEndpoint.Port(rawValue: portValue) else {
            throw LNURLError.invalidResponse
        }

        let tcpOptions = NWProtocolTCP.Options()
        let params: NWParameters
        if isHTTPS {
            let tlsOptions = NWProtocolTLS.Options()
            sec_protocol_options_set_tls_server_name(tlsOptions.securityProtocolOptions, cleanHost)
            params = NWParameters(tls: tlsOptions, tcp: tcpOptions)
        } else {
            params = NWParameters(tls: nil, tcp: tcpOptions)
        }

        let endpoint = NWEndpoint.hostPort(host: NWEndpoint.Host(vettedIP), port: port)
        let connection = NWConnection(to: endpoint, using: params)

        let path = (url.path.isEmpty ? "/" : url.path) + (url.query.map { "?\($0)" } ?? "")
        let requestString = "GET \(path) HTTP/1.1\r\nHost: \(cleanHost)\r\nUser-Agent: StableChannels\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        guard let requestData = requestString.data(using: .utf8) else {
            throw LNURLError.invalidResponse
        }

        let rawResponseData = try await sendAndReceive(connection: connection, requestData: requestData)
        let (body, httpResponse) = try HTTPResponseParser.parse(data: rawResponseData, url: url)

        if (300...399).contains(httpResponse.statusCode) {
            guard let location = httpResponse.allHeaderFields["location"] as? String ??
                httpResponse.allHeaderFields["Location"] as? String,
                let targetURL = URL(string: location, relativeTo: url)?.absoluteURL else {
                throw LNURLError.invalidResponse
            }
            guard SecureEndpointValidator.isSecureEndpoint(url: targetURL, hostResolver: hostResolver) else {
                throw LNURLError.insecureEndpoint
            }
            return try await executeGetInternal(url: targetURL, hop: hop + 1)
        }

        guard let finalURL = httpResponse.url,
              SecureEndpointValidator.isSecureEndpoint(url: finalURL, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        return (body, httpResponse)
    }

    private func sendAndReceive(connection: NWConnection, requestData: Data) async throws -> Data {
        try await withCheckedThrowingContinuation { continuation in
            let lock = NSLock()
            var isFinished = false
            var buffer = Data()

            func complete(with result: Result<Data, Error>) {
                lock.withLock {
                    guard !isFinished else { return }
                    isFinished = true
                    connection.cancel()
                    continuation.resume(with: result)
                }
            }

            func readLoop() {
                connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { data, _, isComplete, error in
                    if let data, !data.isEmpty {
                        buffer.append(data)
                    }
                    if let error {
                        if buffer.isEmpty {
                            complete(with: .failure(LNURLError.networkError(error.localizedDescription)))
                        } else {
                            complete(with: .success(buffer))
                        }
                        return
                    }
                    if isComplete {
                        complete(with: .success(buffer))
                        return
                    }
                    readLoop()
                }
            }

            connection.stateUpdateHandler = { state in
                switch state {
                case .ready:
                    connection.send(content: requestData, completion: .contentProcessed { sendError in
                        if let sendError {
                            complete(with: .failure(LNURLError.networkError(sendError.localizedDescription)))
                        } else {
                            readLoop()
                        }
                    })
                case .failed(let error):
                    complete(with: .failure(LNURLError.networkError(error.localizedDescription)))
                default:
                    break
                }
            }

            connection.start(queue: .global())

            DispatchQueue.global().asyncAfter(deadline: .now() + self.timeoutInterval) {
                complete(with: .failure(LNURLError.networkError("Connection timed out.")))
            }
        }
    }
}

// MARK: - URLSession Transport (Testing / Compatibility Fallback)

final class URLSessionTransport: SecureHTTPTransporting {
    private let urlSession: URLSession
    private let hostResolver: HostIPResolving

    init(urlSession: URLSession, hostResolver: HostIPResolving = SystemHostIPResolver()) {
        self.urlSession = urlSession
        self.hostResolver = hostResolver
    }

    func executeGet(url: URL) async throws -> (Data, HTTPURLResponse) {
        guard SecureEndpointValidator.isSecureEndpoint(url: url, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        let request = SecureEndpointValidator.createSecureRequest(from: url)
        let redirectDelegate = SecureRedirectDelegate(hostResolver: hostResolver)
        let data: Data
        let response: URLResponse
        do {
            (data, response) = try await urlSession.data(for: request, delegate: redirectDelegate)
        } catch {
            throw LNURLError.networkError(error.localizedDescription)
        }

        if redirectDelegate.encounteredInsecureRedirect {
            throw LNURLError.insecureEndpoint
        }

        guard let httpResponse = response as? HTTPURLResponse,
              let finalURL = httpResponse.url else {
            throw LNURLError.invalidResponse
        }

        guard SecureEndpointValidator.isSecureEndpoint(url: finalURL, hostResolver: hostResolver) else {
            throw LNURLError.insecureEndpoint
        }

        return (data, httpResponse)
    }
}
