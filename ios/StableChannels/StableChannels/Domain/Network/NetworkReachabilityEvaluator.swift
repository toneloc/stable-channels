import Foundation

/// Pure domain evaluation rules for network connectivity conditions and error classification.
public enum NetworkReachabilityEvaluator {
    /// Determines whether a given error represents a network reachability or socket connectivity failure.
    public static func isNetworkError(_ error: Error) -> Bool {
        if let urlError = error as? URLError {
            switch urlError.code {
            case .notConnectedToInternet,
                 .networkConnectionLost,
                 .cannotConnectToHost,
                 .timedOut,
                 .cannotFindHost,
                 .dnsLookupFailed,
                 .resourceUnavailable,
                 .internationalRoamingOff,
                 .callIsActive,
                 .dataNotAllowed:
                return true
            default:
                break
            }
        }

        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            switch nsError.code {
            case NSURLErrorNotConnectedToInternet,
                 NSURLErrorNetworkConnectionLost,
                 NSURLErrorCannotConnectToHost,
                 NSURLErrorTimedOut,
                 NSURLErrorCannotFindHost,
                 NSURLErrorDNSLookupFailed,
                 NSURLErrorResourceUnavailable,
                 NSURLErrorInternationalRoamingOff,
                 NSURLErrorCallIsActive,
                 NSURLErrorDataNotAllowed:
                return true
            default:
                break
            }
        }

        if nsError.domain == NSPOSIXErrorDomain {
            switch Int32(nsError.code) {
            case ENETDOWN, ENETUNREACH, ENETRESET, ECONNABORTED, ECONNRESET, ENOTCONN, ETIMEDOUT, ECONNREFUSED,
                 EHOSTDOWN, EHOSTUNREACH:
                return true
            default:
                break
            }
        }

        let description = error.localizedDescription.lowercased()
        if description.contains("offline")
            || description.contains("not connected to the internet")
            || description.contains("network connection was lost")
            || description.contains("cannot connect to host")
            || description.contains("the internet connection appears to be offline")
            || description.contains("timed out") {
            return true
        }

        return false
    }

    /// Evaluates whether an operation failure should be presented as an offline state.
    public static func shouldPresentOfflineNotice(error: Error?, isNetworkOffline: Bool) -> Bool {
        if isNetworkOffline {
            return true
        }
        guard let error else {
            return false
        }
        return isNetworkError(error)
    }
}
