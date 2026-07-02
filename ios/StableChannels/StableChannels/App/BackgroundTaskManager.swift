import Foundation
import BackgroundTasks
import UIKit

final class BackgroundTaskManager: @unchecked Sendable {
    static let shared = BackgroundTaskManager()

    static let keepAliveIdentifier = "com.stablechannels.app.keepAlive"

    private init() {
        register()
    }

    private func register() {
        do {
            try BGTaskScheduler.shared.register(
                forTaskWithIdentifier: Self.keepAliveIdentifier,
                using: nil
            ) { [weak self] task in
                self?.handleKeepAlive(task: task)
            }
            print("[BGTask] Registered keep-alive task")
        } catch {
            print("[BGTask] Registration failed: \(error.localizedDescription)")
        }
    }

    func scheduleKeepAlive() {
        let request = BGProcessingTaskRequest(identifier: Self.keepAliveIdentifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        request.requiresNetworkConnectivity = true
        request.requiresExternalPower = false

        do {
            try BGTaskScheduler.shared.submit(request)
            print("[BGTask] Keep-alive scheduled")
        } catch {
            print("[BGTask] Schedule failed: \(error.localizedDescription)")
        }
    }

    func cancelKeepAlive() {
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.keepAliveIdentifier)
        print("[BGTask] Keep-alive cancelled")
    }

    private func handleKeepAlive(task: BGTask) {
        guard let processingTask = task as? BGProcessingTask else { return }

        processingTask.expirationHandler = { [weak self] in
            print("[BGTask] Expiration — cancelling keep-alive")
            self?.cancelKeepAlive()
        }

        print("[BGTask] Fired — isRunning: \(NodeService.shared.isRunning)")

        if NodeService.shared.isRunning {
            NodeService.shared.refreshChannels()
            let allUsable = !NodeService.shared.channels.isEmpty
                && NodeService.shared.channels.allSatisfy(\.isUsable)
            if !allUsable {
                Task {
                    try? await NodeService.shared.node?.connect(
                        nodeId: Constants.defaultLSPPubkey,
                        address: Constants.defaultLSPAddress,
                        persist: true
                    )
                }
            }
        }

        scheduleKeepAlive()
        processingTask.setTaskCompleted(success: true)
    }
}
