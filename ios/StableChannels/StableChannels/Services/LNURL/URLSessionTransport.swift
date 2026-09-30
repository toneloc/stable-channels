import Foundation

#if DEBUG

    // MARK: - Secure Redirect Delegate

    final class SecureRedirectDelegate: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
        private let lock = NSLock()
        private var _encounteredInsecureRedirect = false
        private let hostResolver: HostIPResolving

        var encounteredInsecureRedirect: Bool {
            lock.withLock { _encounteredInsecureRedirect }
        }

        init(hostResolver: HostIPResolving = SystemHostIPResolver()) {
            self.hostResolver = hostResolver
        }

        func urlSession(
            _: URLSession,
            task: URLSessionTask,
            willPerformHTTPRedirection _: HTTPURLResponse,
            newRequest: URLRequest,
            completionHandler: @escaping (URLRequest?) -> Void
        ) {
            guard let targetURL = newRequest.url,
                  SecureEndpointValidator.isSecureEndpoint(url: targetURL, hostResolver: hostResolver)
            else {
                lock.withLock { _encounteredInsecureRedirect = true }
                completionHandler(nil)
                return
            }

            // Strictly constrain onion endpoints: reject clearnet redirects from onion services
            let isFromOnion = [task.originalRequest?.url?.host, task.currentRequest?.url?.host]
                .compactMap { $0 }
                .contains { SecureEndpointValidator.cleanHostString($0).hasSuffix(".onion") }

            if isFromOnion {
                let targetHost = SecureEndpointValidator.cleanHostString(targetURL.host ?? "")
                if !targetHost.hasSuffix(".onion") {
                    lock.withLock { _encounteredInsecureRedirect = true }
                    completionHandler(nil)
                    return
                }
            }

            completionHandler(newRequest)
        }
    }

    // MARK: - URLSession Transport (Testing / Compatibility Fallback)

    final class URLSessionTransport: SecureHTTPTransporting {
        static func makeEphemeralSession(timeout: TimeInterval) -> URLSession {
            let cfg = URLSessionConfiguration.ephemeral
            cfg.timeoutIntervalForRequest = timeout
            cfg.timeoutIntervalForResource = timeout
            return URLSession(configuration: cfg)
        }

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
            let req = SecureEndpointValidator.createSecureRequest(from: url)
            let delegate = SecureRedirectDelegate(hostResolver: hostResolver)
            do {
                let (data, response) = try await urlSession.data(for: req, delegate: delegate)
                guard !delegate.encounteredInsecureRedirect,
                      let http = response as? HTTPURLResponse,
                      let finalURL = http.url,
                      SecureEndpointValidator.isSecureEndpoint(url: finalURL, hostResolver: hostResolver) else {
                    throw LNURLError.insecureEndpoint
                }
                guard data.count <= HTTPResponseParser.maxResponseBytes else { throw LNURLError.invalidResponse }
                return (data, http)
            } catch let error as LNURLError {
                throw error
            } catch {
                throw LNURLError.networkError(error.localizedDescription)
            }
        }
    }
#endif
