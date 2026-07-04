import Foundation
import BackgroundTasks
import UIKit

final class BackgroundTaskManager: @unchecked Sendable {
    static let shared = BackgroundTaskManager()

    static let channelSyncIdentifier = "com.stablechannels.app.channelSync"

    private init() {
        register()
    }

    private func register() {
        do {
            try BGTaskScheduler.shared.register(
                forTaskWithIdentifier: Self.channelSyncIdentifier,
                using: nil
            ) { [weak self] task in
                self?.handleChannelSync(task: task)
            }
            print("[BGTask] Registered channel sync task")
        } catch {
            print("[BGTask] Registration failed: \(error.localizedDescription)")
        }
    }

    func scheduleChannelSync() {
        let request = BGProcessingTaskRequest(identifier: Self.channelSyncIdentifier)
        request.earliestBeginDate = Date(timeIntervalSinceNow: 15 * 60)
        request.requiresNetworkConnectivity = true
        request.requiresExternalPower = false

        do {
            try BGTaskScheduler.shared.submit(request)
            print("[BGTask] Channel sync scheduled")
        } catch {
            print("[BGTask] Schedule failed: \(error.localizedDescription)")
        }
    }

    func cancelChannelSync() {
        BGTaskScheduler.shared.cancel(taskRequestWithIdentifier: Self.channelSyncIdentifier)
        print("[BGTask] Channel sync cancelled")
    }

    private func handleChannelSync(task: BGTask) {
        guard let processingTask = task as? BGProcessingTask else { return }

        processingTask.expirationHandler = { [weak self] in
            print("[BGTask] Expiration — cancelling channel sync")
            self?.cancelChannelSync()
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

        scheduleChannelSync()
        processingTask.setTaskCompleted(success: true)
    }
}
