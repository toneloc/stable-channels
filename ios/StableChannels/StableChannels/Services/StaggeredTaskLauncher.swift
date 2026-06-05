import Foundation

/// Manages one cancellable `Task` per `opId` on the MainActor, with an
/// optional staggered start delay (used to spread burst replays across
/// time to avoid hammering the public Esplora endpoints).
@MainActor
final class StaggeredTaskLauncher {
    private var tasks: [String: Task<Void, Never>] = [:]
    private var generations: [String: UUID] = [:]

    /// Launch `task` after `delaySeconds` (SECONDS, not milliseconds — multiplied
    /// by 1e9 below to get nanoseconds for `Task.sleep`). If a task is already
    /// running for `opId`, it is cancelled and replaced. The closure may exit
    /// early by checking `Task.isCancelled`.
    func launch(opId: String, delaySeconds: UInt64 = 0, _ task: @escaping @MainActor () async -> Void) {
        tasks[opId]?.cancel()
        let generation = UUID()
        generations[opId] = generation
        let newTask = Task { @MainActor [weak self] in
            if delaySeconds > 0 {
                // Unit: seconds. Caller passes e.g. UInt64(i) where i is a per-op index
                // in a burst-replay loop, e.g. 0s, 2s, 4s, 6s for 4 ops.
                try? await Task.sleep(nanoseconds: delaySeconds * 1_000_000_000)
                if Task.isCancelled { return }
            }
            await task()
            if self?.generations[opId] == generation {
                self?.tasks[opId] = nil
                self?.generations[opId] = nil
            }
        }
        tasks[opId] = newTask
    }

    /// Cancel any running task for `opId`.
    func cancel(opId: String) {
        tasks[opId]?.cancel()
        tasks[opId] = nil
        generations[opId] = nil
    }

    /// Cancel all running tasks.
    func cancelAll() {
        for task in tasks.values {
            task.cancel()
        }
        tasks.removeAll()
        generations.removeAll()
    }
}
