import Foundation
import SwiftUI
import os.log

@Observable
@MainActor
final class BlockHeightService {
    private(set) var currentHeight: UInt32 = 0
    private let provider: BlockHeightProvider
    private let pollInterval: TimeInterval
    private var pollTask: Task<Void, Never>?
    var onHeightUpdated: ((UInt32) -> Void)?

    init(provider: BlockHeightProvider, pollInterval: TimeInterval = 30) {
        self.provider = provider
        self.pollInterval = pollInterval
    }

    func start() {
        guard pollTask == nil else { return }
        pollTask = Task { [weak self] in
            while !Task.isCancelled {
                let start = Date()
                await self?.refresh()
                let elapsed = Date().timeIntervalSince(start)
                let delay = max(0, (self?.pollInterval ?? 30) - elapsed)
                try? await Task.sleep(nanoseconds: UInt64(delay * 1_000_000_000))
            }
        }
    }

    func stop() {
        pollTask?.cancel()
        pollTask = nil
    }

    func refresh() async {
        do {
            let height = try await provider.currentHeight()
            currentHeight = height
            onHeightUpdated?(height)
        } catch {
            os_log("Block height refresh failed: %{public}@", log: .default, type: .error, error.localizedDescription)
        }
    }
}
