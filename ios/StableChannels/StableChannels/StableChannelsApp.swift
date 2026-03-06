import SwiftUI
import UIKit
import UserNotifications

@main
struct StableChannelsApp: App {
    @UIApplicationDelegateAdaptor(AppDelegate.self) var appDelegate
    @State private var appState = AppState()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(appState)
                .task {
                    await appState.start()
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.didEnterBackgroundNotification)) { _ in
                    appState.stopNodeForBackground()
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.willEnterForegroundNotification)) { _ in
                    Task { await appState.restartNodeFromForeground() }
                }
                .onReceive(NotificationCenter.default.publisher(for: UIApplication.willTerminateNotification)) { _ in
                    appState.stop()
                }
        }
    }
}

// MARK: - AppDelegate for Push Notifications

class AppDelegate: NSObject, UIApplicationDelegate, UNUserNotificationCenterDelegate {

    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        UNUserNotificationCenter.current().delegate = self

        // Request notification permission — required for receiving stability payments while offline
        UNUserNotificationCenter.current().requestAuthorization(
            options: [.alert, .sound, .badge]
        ) { granted, error in
            if let error {
                print("[Push] Authorization error: \(error.localizedDescription)")
            }
            // Register regardless — silent pushes work even without permission
            DispatchQueue.main.async {
                application.registerForRemoteNotifications()
            }
            if !granted {
                print("[Push] Permission denied — stability payments require notifications")
            }
        }

        return true
    }

    // MARK: - Token Registration

    func application(
        _ application: UIApplication,
        didRegisterForRemoteNotificationsWithDeviceToken deviceToken: Data
    ) {
        let token = deviceToken.map { String(format: "%02.2hhx", $0) }.joined()
        print("[Push] APNs device token: \(token)")

        // Store locally so we can display it in settings / send to LSP
        UserDefaults.standard.set(token, forKey: "apns_device_token")

        // Send token to LSP so it can push us when payments arrive
        Task {
            await registerTokenWithLSP(token)
        }
    }

    func application(
        _ application: UIApplication,
        didFailToRegisterForRemoteNotificationsWithError error: Error
    ) {
        print("[Push] Registration failed: \(error.localizedDescription)")
    }

    // MARK: - Background Push (content-available: 1)

    func application(
        _ application: UIApplication,
        didReceiveRemoteNotification userInfo: [AnyHashable: Any],
        fetchCompletionHandler completionHandler: @escaping (UIBackgroundFetchResult) -> Void
    ) {
        print("[Push] Background push received: \(userInfo)")

        // Post to NotificationCenter so AppState can handle it
        NotificationCenter.default.post(name: .pushPaymentNotification, object: userInfo)

        // Give the node up to 25 seconds to connect and receive the pending payment
        DispatchQueue.main.asyncAfter(deadline: .now() + 25) {
            completionHandler(.newData)
        }
    }

    // MARK: - Foreground Notification Display

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        willPresent notification: UNNotification,
        withCompletionHandler completionHandler: @escaping (UNNotificationPresentationOptions) -> Void
    ) {
        // Show banner + sound even when app is in foreground
        completionHandler([.banner, .sound])
    }

    // MARK: - Notification Tap

    func userNotificationCenter(
        _ center: UNUserNotificationCenter,
        didReceive response: UNNotificationResponse,
        withCompletionHandler completionHandler: @escaping () -> Void
    ) {
        // App will be opened/foregrounded — normal startup handles payment processing
        print("[Push] User tapped notification")
        completionHandler()
    }

    // MARK: - LSP Token Registration

    private func registerTokenWithLSP(_ token: String) async {
        guard let url = URL(string: "http://\(Constants.defaultLSPAddress.replacingOccurrences(of: ":9737", with: ":8080"))/api/register-push") else { return }

        var request = URLRequest(url: url)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")

        // Include node_id so LSP can send targeted push notifications
        let nodeId = UserDefaults(suiteName: Constants.appGroupIdentifier)?
            .string(forKey: "node_id") ?? ""

        #if DEBUG
        let apnsEnvironment = "sandbox"
        #else
        let apnsEnvironment = "production"
        #endif

        let body: [String: String] = [
            "device_token": token,
            "platform": "ios",
            "node_id": nodeId,
            "environment": apnsEnvironment,
        ]

        guard let httpBody = try? JSONSerialization.data(withJSONObject: body) else { return }
        request.httpBody = httpBody

        do {
            let (_, response) = try await URLSession.shared.data(for: request)
            if let http = response as? HTTPURLResponse {
                print("[Push] LSP registration response: \(http.statusCode) node_id: \(nodeId.prefix(16))...")
            }
        } catch {
            print("[Push] LSP registration failed: \(error.localizedDescription)")
        }
    }
}

// MARK: - Notification Names

extension Notification.Name {
    static let pushPaymentNotification = Notification.Name("pushPaymentNotification")
}
