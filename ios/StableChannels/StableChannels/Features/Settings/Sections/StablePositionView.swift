import SwiftUI
import UIKit

struct StablePositionView: View {
    @Environment(AppState.self) private var appState
    @State private var copiedCounterparty = false

    var body: some View {
        List {
            Section {
                HStack {
                    Text(String(localized: .labelExpectedUsd))
                    Spacer()
                    Text(appState.stableChannel.expectedUSD.formatted)
                        .fontWeight(.medium)
                }
                HStack {
                    Text(String(localized: .labelBackingSats))
                    Spacer()
                    Text("\(appState.stableChannel.backingSats.btcSpacedFormatted) BTC")
                }
                HStack {
                    Text(String(localized: .labelNativeBtc))
                    Spacer()
                    Text("\(appState.stableChannel.nativeChannelBTC.sats.btcSpacedFormatted) BTC")
                }

                if appState.btcPrice > 0 && appState.stableChannel.expectedUSD.amount > 0 {
                    let result = StabilityService.checkStabilityAction(
                        appState.stableChannel, price: appState.btcPrice
                    )
                    HStack {
                        Text(String(localized: .labelStatus))
                        Spacer()
                        Text(result.action.rawValue)
                            .foregroundStyle(stabilityColor(result.action))
                            .fontWeight(.medium)
                    }
                    HStack {
                        Text(String(localized: .labelDistanceFromPar))
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
                            Text(String(localized: .labelCounterparty))
                                .foregroundStyle(.primary)
                            Spacer()
                            if copiedCounterparty {
                                Label(
                                    String(localized: .buttonCopied),
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
        .navigationTitle(String(localized: .titleStablePosition))
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
