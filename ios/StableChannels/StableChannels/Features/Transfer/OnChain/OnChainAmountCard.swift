import SwiftUI

/// Card view for entering USD/BTC amount or selecting "Send All" in onchain transfers.
struct OnChainAmountCard: View {
    @Environment(AppState.self) private var appState
    let hasReadyChannel: Bool
    @Binding var sendAll: Bool
    @Binding var amountUSDStr: String
    let amountSats: UInt64?

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Image(systemName: "dollarsign.circle")
                    .foregroundStyle(.secondary)
                Text(String(localized: "header_amount", defaultValue: "Amount"))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
            }

            let available = sendAll ? appState
                .spendableOnchainSats :
                (hasReadyChannel && !appState.isSweeping ? appState.totalBalanceSats : appState.spendableOnchainSats)
            let availableUSD = appState
                .accountingBTCPrice > 0 ? (Double(available) / Double(Constants.satsInBTC)) * appState
                .accountingBTCPrice : 0

            if sendAll {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(String(localized: "label_dollar_sign", defaultValue: "$"))
                        .font(.system(size: 36, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                    Text(verbatim: String(format: "%.2f", availableUSD))
                        .font(.system(size: 36, weight: .semibold, design: .rounded))
                        .foregroundStyle(.primary)
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)
                }
                if available > 0 {
                    HStack(spacing: 6) {
                        Image(systemName: "bitcoinsign.circle.fill")
                            .foregroundStyle(.primary)
                        Text("\(available.btcSpacedFormatted) BTC")
                            .font(.subheadline.weight(.medium))
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.ultraThinMaterial, in: Capsule())
                }
            } else {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(String(localized: "label_dollar_sign", defaultValue: "$"))
                        .font(.system(size: 36, weight: .semibold, design: .rounded))
                        .foregroundStyle(.secondary)
                    TextField(
                        String(localized: "placeholder_amount_usd", defaultValue: "0.00"),
                        text: $amountUSDStr
                    )
                    .keyboardType(.decimalPad)
                    .font(.system(size: 36, weight: .semibold, design: .rounded))
                    .lineLimit(1)
                    .minimumScaleFactor(0.5)
                    .onChange(of: amountUSDStr) { _, new in
                        let sanitized = InputSanitizer.decimal(new)
                        amountUSDStr = sanitized.count > 16 ? String(sanitized.prefix(16)) : sanitized
                    }
                }
                let isExceeded = (amountSats ?? 0) > available
                if let sats = amountSats, sats > 0 {
                    HStack(spacing: 6) {
                        Image(systemName: "bitcoinsign.circle.fill")
                            .foregroundStyle(.primary)
                        Text("\(sats.btcSpacedFormatted) BTC")
                            .font(.subheadline.weight(.medium))
                            .contentTransition(.numericText())
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.ultraThinMaterial, in: Capsule())
                    .animation(.snappy, value: sats)
                }
                if isExceeded {
                    HStack(spacing: 6) {
                        Image(systemName: "exclamationmark.circle.fill")
                            .font(.caption)
                            .foregroundStyle(.red)
                        Text(String(
                            localized: "error_amount_exceeds_balance",
                            defaultValue: "Amount exceeds your balance"
                        ))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.red)
                    }
                    .transition(.opacity)
                }
            }

            Toggle(isOn: $sendAll) {
                HStack(spacing: 8) {
                    LemniscateBloomIcon(isActive: sendAll, size: 16, tint: .green)
                    Text(String(localized: "toggle_send_all", defaultValue: "Send All"))
                        .font(.subheadline)
                }
            }
            .tint(.green)
        }
        .padding(16)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(.ultraThinMaterial)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }
}
