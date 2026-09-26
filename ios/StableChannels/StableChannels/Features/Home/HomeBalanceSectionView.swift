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

    private var hasBalance: Bool {
        appState.totalBalanceUSD > 0 || displaySats > 0
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
                    Text(displaySats.btcSpacedFormatted)
                        .font(.system(size: 32, weight: .bold, design: .monospaced))
                        .foregroundStyle(appState.paymentFlash ? .green : .primary)
                        .contentTransition(.numericText())
                        .animation(.default, value: displaySats)
                    Text(String(localized: "label_btc", defaultValue: "BTC"))
                        .font(.system(size: 18, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                Text(appState.totalBalanceUSD.usdFormatted)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 6) {
                    Text(appState.totalBalanceUSD.usdFormatted)
                        .font(.system(size: 42, weight: .bold, design: .rounded))
                        .foregroundStyle(appState.paymentFlash ? .green : .primary)
                        .contentTransition(.numericText())
                        .animation(.default, value: appState.totalBalanceUSD)
                        .animation(.easeInOut(duration: 0.3), value: appState.paymentFlash)
                    Text(String(localized: "label_usd", defaultValue: "USD"))
                        .font(.system(size: 18, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                }

                Text("\(displaySats.btcSpacedFormatted) BTC")
                    .font(.caption)
                    .foregroundStyle(.secondary)
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
