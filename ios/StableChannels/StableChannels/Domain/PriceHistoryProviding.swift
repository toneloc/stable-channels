import Foundation

/// Protocol defining the contract for loading, filtering, and sampling historical price series.
/// Decouples Presentation components from concrete database and network infrastructure.
protocol PriceHistoryProviding: Sendable {
    func fetchPriceHistory(for period: ChartPeriod, force: Bool) async -> [PriceRecord]
}
