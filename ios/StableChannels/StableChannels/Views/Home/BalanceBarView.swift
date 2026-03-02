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

        VStack(spacing: 4) {
            GeometryReader { geometry in
                HStack(spacing: 2) {
                    if stableFraction > 0.01 {
                        RoundedRectangle(cornerRadius: 6)
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
                        RoundedRectangle(cornerRadius: 6)
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
            .frame(height: 14)
            .clipShape(RoundedRectangle(cornerRadius: 7))

            HStack {
                Text(String(format: "$%.2f stable", stableUSD))
                    .font(.caption2)
                    .foregroundStyle(.green)
                Spacer()
                Text(String(format: "$%.2f native", nativeUSD))
                    .font(.caption2)
                    .foregroundStyle(.orange)
            }
        }
    }
}
