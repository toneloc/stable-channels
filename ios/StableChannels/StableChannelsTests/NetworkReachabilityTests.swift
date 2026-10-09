@testable import StableChannels
import XCTest

final class NetworkReachabilityTests: XCTestCase {
    // MARK: - NetworkStatus Tests

    func testNetworkStatusEqualityAndConnectionFlag() {
        let online = NetworkStatus.online
        let offline = NetworkStatus.offline

        XCTAssertTrue(online.isConnected)
        XCTAssertFalse(offline.isConnected)
        XCTAssertEqual(online, NetworkStatus.online)
        XCTAssertEqual(offline, NetworkStatus.offline)
        XCTAssertNotEqual(online, offline)
    }

    // MARK: - NetworkReachabilityEvaluator Tests

    func testEvaluatorRecognizesURLErrors() {
        let urlErrors: [URLError.Code] = [
            .notConnectedToInternet,
            .networkConnectionLost,
            .cannotConnectToHost,
            .timedOut,
            .cannotFindHost,
            .dnsLookupFailed,
            .resourceUnavailable,
            .dataNotAllowed
        ]

        for code in urlErrors {
            let error = URLError(code)
            XCTAssertTrue(
                NetworkReachabilityEvaluator.isNetworkError(error),
                "Expected \(code) to be classified as a network error"
            )
        }
    }

    func testEvaluatorRecognizesPOSIXNetworkErrors() {
        let posixErrors: [Int32] = [
            ENETDOWN,
            ENETUNREACH,
            ECONNRESET,
            ECONNREFUSED,
            ETIMEDOUT,
            ENOTCONN
        ]

        for code in posixErrors {
            let error = NSError(domain: NSPOSIXErrorDomain, code: Int(code))
            XCTAssertTrue(
                NetworkReachabilityEvaluator.isNetworkError(error),
                "Expected POSIX error code \(code) to be classified as a network error"
            )
        }
    }

    func testEvaluatorRejectsNonNetworkErrors() {
        let nonNetworkErrors: [Error] = [
            NSError(domain: "DatabaseError", code: 1, userInfo: [NSLocalizedDescriptionKey: "Syntax error in SQL"]),
            NSError(domain: "AuthError", code: 401, userInfo: [NSLocalizedDescriptionKey: "Invalid credentials"]),
            NSError(
                domain: NSCocoaErrorDomain,
                code: NSFileNoSuchFileError,
                userInfo: [NSLocalizedDescriptionKey: "File not found"]
            )
        ]

        for error in nonNetworkErrors {
            XCTAssertFalse(
                NetworkReachabilityEvaluator.isNetworkError(error),
                "Expected \(error) not to be classified as a network error"
            )
        }
    }

    func testShouldPresentOfflineNoticeLogic() {
        XCTAssertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(error: nil, isNetworkOffline: true),
            "Must present offline notice when device is offline regardless of error"
        )

        let networkError = URLError(.notConnectedToInternet)
        XCTAssertTrue(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(error: networkError, isNetworkOffline: false),
            "Must present offline notice when network error occurred"
        )

        let genericError = NSError(
            domain: "CustomDomain",
            code: 500,
            userInfo: [NSLocalizedDescriptionKey: "Database locked"]
        )
        XCTAssertFalse(
            NetworkReachabilityEvaluator.shouldPresentOfflineNotice(error: genericError, isNetworkOffline: false),
            "Must not present offline notice for non-network error when device is online"
        )
    }

    // MARK: - MockNetworkMonitor Tests

    func testMockNetworkMonitorStateLifecycle() {
        let monitor = MockNetworkMonitor(initialStatus: .online)
        XCTAssertTrue(monitor.isOnline)
        XCTAssertEqual(monitor.currentStatus, .online)

        monitor.start()
        XCTAssertTrue(monitor.didStart)

        monitor.setStatus(.offline)
        XCTAssertFalse(monitor.isOnline)
        XCTAssertEqual(monitor.currentStatus, .offline)

        monitor.stop()
        XCTAssertTrue(monitor.didStop)
    }
}

final class MockNetworkMonitor: NetworkMonitoring, @unchecked Sendable {
    private let lock = NSLock()
    private var internalStatus: NetworkStatus
    var didStart = false
    var didStop = false
    var onStatusChange: (@Sendable (NetworkStatus) -> Void)?

    init(initialStatus: NetworkStatus = .online) {
        self.internalStatus = initialStatus
    }

    var currentStatus: NetworkStatus {
        lock.lock()
        defer { lock.unlock() }
        return internalStatus
    }

    var isOnline: Bool {
        currentStatus.isConnected
    }

    func setStatus(_ status: NetworkStatus) {
        lock.lock()
        internalStatus = status
        let handler = onStatusChange
        lock.unlock()
        handler?(status)
    }

    func start() {
        lock.lock()
        didStart = true
        lock.unlock()
    }

    func stop() {
        lock.lock()
        didStop = true
        lock.unlock()
    }
}
