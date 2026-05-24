import SwiftUI
import UIKit

struct StablePositionView: View {
    @Environment(AppState.self) private var appState
    @State private var copiedCounterparty = false

    var body: some View {
        List {
            Section {
                HStack {
                    Text(String(localized: "label_expected_usd", defaultValue: "Expected USD"))
                    Spacer()
                    Text(appState.stableChannel.expectedUSD.formatted)
                        .fontWeight(.medium)
                }
                HStack {
                    Text(String(localized: "label_backing_sats", defaultValue: "Backing Sats"))
                    Spacer()
                    Text(appState.stableChannel.backingSats.satsFormatted)
                }
                HStack {
                    Text(String(localized: "label_native_btc", defaultValue: "Native BTC"))
                    Spacer()
                    Text(appState.stableChannel.nativeChannelBTC.sats.satsFormatted)
                }

                if appState.btcPrice > 0 && appState.stableChannel.expectedUSD.amount > 0 {
                    let result = StabilityService.checkStabilityAction(
                        appState.stableChannel, price: appState.btcPrice
                    )
                    HStack {
                        Text(String(localized: "label_status", defaultValue: "Status"))
                        Spacer()
                        Text(result.action.rawValue)
                            .foregroundStyle(stabilityColor(result.action))
                            .fontWeight(.medium)
                    }
                    HStack {
                        Text(String(localized: "label_distance_from_par", defaultValue: "Distance from Par"))
                        Spacer()
                        Text(String(format: "%.2f%%", result.percentFromPar))
                            .foregroundStyle(result.percentFromPar < 0.1 ? .green : .orange)
                    }
                }

                if !appState.stableChannel.counterparty.isEmpty {
                    Button {
                        UIPasteboard.general.string = appState.stableChannel.counterparty
                        withAnimation { copiedCounterparty = true }
                        DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                            withAnimation { copiedCounterparty = false }
                        }
                    } label: {
                        HStack {
                            Text(String(localized: "label_counterparty", defaultValue: "Counterparty"))
                                .foregroundStyle(.primary)
                            Spacer()
                            if copiedCounterparty {
                                Label(
                                    String(localized: "button_copied", defaultValue: "Copied"),
                                    systemImage: "checkmark"
                                )
                                .font(.caption)
                                .foregroundStyle(.green)
                                .transition(.scale.combined(with: .opacity))
                            } else {
                                Text(String(appState.stableChannel.counterparty.prefix(8)) + "..."
                                    + String(appState.stableChannel.counterparty.suffix(8)))
                                    .font(.system(.caption, design: .monospaced))
                                    .foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
        }
        .navigationTitle(String(localized: "title_stable_position", defaultValue: "Stable Position"))
        .navigationBarTitleDisplayMode(.inline)
    }

    private func stabilityColor(_ action: StabilityService.StabilityAction) -> Color {
        switch action {
        case .stable: return .green
        case .pay: return .orange
        case .checkOnly: return .blue
        case .highRiskNoAction: return .red
        }
    }
}
