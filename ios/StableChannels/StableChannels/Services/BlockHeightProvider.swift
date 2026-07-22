import Foundation

protocol BlockHeightProvider: Sendable {
    func currentHeight() async throws -> UInt32
}
