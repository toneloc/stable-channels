import Foundation
import Observation

@Observable
@MainActor
final class LSPService {
    private(set) var activeLSP: LSPConfig

    init(initialConfig: LSPConfig = .load()) {
        self.activeLSP = initialConfig
    }

    func switchLSP(
        to newConfig: LSPConfig,
        nodeService: NodeServiceProtocol,
        chainURL: String,
        onSuccess: @MainActor () -> Void = {}
    ) async -> Bool {
        guard !nodeService.channels.contains(where: { $0.isChannelReady || $0.isUsable }) else {
            return false
        }

        let oldConfig = activeLSP
        activeLSP = newConfig
        newConfig.save()

        nodeService.stop()
        NodeDirLock.shared.release()

        do {
            try await nodeService.start(
                network: .bitcoin,
                esploraURL: chainURL,
                mnemonic: "",
                lspConfig: newConfig
            )
            onSuccess()
            return true
        } catch {
            print("[LSPService] Failed to start node with new LSP config: \(error.localizedDescription)")
            activeLSP = oldConfig
            oldConfig.save()

            nodeService.stop()
            NodeDirLock.shared.release()

            try? await nodeService.start(
                network: .bitcoin,
                esploraURL: chainURL,
                mnemonic: "",
                lspConfig: oldConfig
            )
            return false
        }
    }
}
