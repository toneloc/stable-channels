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
            let effectiveSats = sendAll ? available : (amountSats ?? 0)
            let displayUSD = sendAll
                ? String(format: "%.2f", availableUSD)
                : (amountUSDStr.isEmpty ? "0.00" : amountUSDStr)

            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(String(localized: "label_dollar_sign", defaultValue: "$"))
                    .font(.system(size: 36, weight: .semibold, design: .rounded))
                    .foregroundStyle(.secondary)

                ZStack(alignment: .leading) {
                    Text(displayUSD)
                        .font(.system(size: 36, weight: .semibold, design: .rounded).monospacedDigit())
                        .foregroundStyle(
                            sendAll
                                ? Color.primary
                                : (amountUSDStr.isEmpty ? Color.secondary.opacity(0.4) : Color.primary)
                        )
                        .contentTransition(.numericText())
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)

                    if !sendAll {
                        TextField(
                            String(localized: "placeholder_amount_usd", defaultValue: "0.00"),
                            text: $amountUSDStr
                        )
                        .keyboardType(.decimalPad)
                        .font(.system(size: 36, weight: .semibold, design: .rounded).monospacedDigit())
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)
                        .foregroundStyle(Color.clear)
                        .onChange(of: amountUSDStr) { _, new in
                            let sanitized = InputSanitizer.decimal(new)
                            amountUSDStr = sanitized.count > 16 ? String(sanitized.prefix(16)) : sanitized
                        }
                    }
                }
            }

            if effectiveSats > 0 {
                HStack(spacing: 6) {
                    Image(systemName: "bitcoinsign.circle.fill")
                        .foregroundStyle(.primary)
                    Text("\(effectiveSats.btcSpacedFormatted) BTC")
                        .font(.subheadline.weight(.medium).monospacedDigit())
                        .contentTransition(.numericText())
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 6)
                .background(.ultraThinMaterial, in: Capsule())
                .transition(.opacity.combined(with: .scale(scale: 0.96)))
            }

            let isExceeded = !sendAll && (amountSats ?? 0) > available
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

            Toggle(isOn: $sendAll.animation(.snappy(duration: 0.32, extraBounce: 0.04))) {
                HStack(spacing: 8) {
                    LemniscateBloomIcon(isActive: sendAll, size: 16, tint: .green)
                    Text(String(localized: "toggle_send_all", defaultValue: "Send All"))
                        .font(.subheadline)
                }
            }
            .tint(.green)
            .onChange(of: sendAll) { _, isAll in
                UISelectionFeedbackGenerator().selectionChanged()
                if isAll {
                    UIApplication.shared.sendAction(
                        #selector(UIResponder.resignFirstResponder),
                        to: nil,
                        from: nil,
                        for: nil
                    )
                }
            }
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
