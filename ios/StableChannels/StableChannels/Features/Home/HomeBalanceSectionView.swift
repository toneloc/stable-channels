import SwiftUI

struct HomeBalanceSectionView: View {
    @Environment(AppState.self) private var appState
    @Binding var showBTC: Bool
    let flashScale: CGFloat

    private var displaySats: UInt64 {
        appState.totalBalanceSats > 0
            ? appState.totalBalanceSats
            : appState.stableChannel.stableReceiverBTC.sats
    }

    private var displayUSD: Double {
        if appState.totalBalanceUSD > 0 {
            return appState.totalBalanceUSD
        }
        if appState.btcPrice > 0 && displaySats > 0 {
            return Double(displaySats) / Double(Constants.satsInBTC) * appState.btcPrice
        }
        return appState.stableUSD
    }

    private var hasBalance: Bool {
        displayUSD > 0 || displaySats > 0
    }

    var body: some View {
        VStack(spacing: 4) {
            Text(String(localized: "label_total_balance", defaultValue: "Total Balance"))
                .font(.caption)
                .foregroundStyle(.secondary)

            if !hasBalance && appState.isSyncing {
                Text(String(localized: "label_dash", defaultValue: "—"))
                    .font(.system(size: 42, weight: .bold, design: .rounded))
                    .foregroundStyle(.secondary)

                Text(String(localized: "loading_balance", defaultValue: "Loading balance..."))
                    .font(.caption)
                    .foregroundStyle(.tertiary)
            } else if showBTC {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    RollingDigitLabel(
                        text: displaySats.btcSpacedFormatted,
                        value: Double(displaySats),
                        font: .system(size: 32, weight: .bold, design: .monospaced),
                        baseColor: appState.paymentFlash ? .green : .primary
                    )
                    Text(String(localized: "label_btc", defaultValue: "BTC"))
                        .font(.system(size: 18, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                RollingDigitLabel(
                    text: displayUSD.usdFormatted,
                    value: displayUSD,
                    font: .caption,
                    baseColor: .secondary
                )
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    RollingDigitLabel(
                        text: displayUSD.usdFormatted,
                        value: displayUSD,
                        font: .system(size: 42, weight: .bold, design: .rounded),
                        baseColor: appState.paymentFlash ? .green : .primary
                    )
                    Text(String(localized: "label_usd", defaultValue: "USD"))
                        .font(.system(size: 18, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                HStack(spacing: 3) {
                    RollingDigitLabel(
                        text: displaySats.btcSpacedFormatted,
                        value: Double(displaySats),
                        font: .caption,
                        baseColor: .secondary
                    )
                    Text(String(localized: "label_btc", defaultValue: "BTC"))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .scaleEffect(flashScale)
        .padding(.top, 8)
        .onTapGesture {
            withAnimation(.easeInOut(duration: 0.2)) {
                showBTC.toggle()
            }
        }
    }
}
