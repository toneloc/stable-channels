import SwiftUI

struct BalanceBarView: View {
    let stableUSD: Double
    let nativeSats: UInt64
    let totalSats: UInt64
    let btcPrice: Double

    var body: some View {
        let nativeUSD = btcPrice > 0
            ? Double(nativeSats) / Double(Constants.satsInBTC) * btcPrice
            : 0.0
        let totalUSD = stableUSD + nativeUSD
        let stableFraction = totalUSD > 0 ? stableUSD / totalUSD : 0
        let nativeFraction = totalUSD > 0 ? nativeUSD / totalUSD : 0

        GeometryReader { geometry in
            HStack(spacing: 2) {
                if stableFraction > 0.01 {
                    RoundedRectangle(cornerRadius: 5)
                        .fill(
                            LinearGradient(
                                colors: [.green.opacity(0.8), .green],
                                startPoint: .leading,
                                endPoint: .trailing
                            )
                        )
                        .frame(width: max(geometry.size.width * stableFraction, 4))
                }
                if nativeFraction > 0.01 {
                    RoundedRectangle(cornerRadius: 5)
                        .fill(
                            LinearGradient(
                                colors: [.orange, .orange.opacity(0.8)],
                                startPoint: .leading,
                                endPoint: .trailing
                            )
                        )
                        .frame(width: max(geometry.size.width * nativeFraction, 4))
                }
            }
            .animation(.easeInOut(duration: 0.3), value: stableFraction)
        }
        .frame(height: 10)
        .clipShape(RoundedRectangle(cornerRadius: 5))
    }
}
