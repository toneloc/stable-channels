import SwiftUI

/// Card view for entering USD/BTC amount or selecting "Send All" in onchain transfers.
struct OnChainAmountCard: View {
    @Environment(AppState.self) private var appState
    @Bindable var model: SendFlowModel

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack {
                Image(systemName: "dollarsign.circle")
                    .foregroundStyle(.secondary)
                Text(String(localized: .headerAmount))
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(.secondary)
                Spacer()
            }

            let sendAll = model.sendsAllFromOnchainWallet
            let amountUSDStr = model.amountInputText
            let available = model.availableSpendableSats(appState: appState)
            let effectiveSats = model.displayedAmountSats(appState: appState)
            let availableUSD = SendAmountCalculator.formatSatsForUnit(
                available,
                unit: .usd,
                btcPrice: appState.accountingBTCPrice
            )
            let displayUSD = sendAll
                ? (availableUSD.isEmpty ? "0.00" : availableUSD)
                : (amountUSDStr.isEmpty ? "0.00" : amountUSDStr)

            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(String(localized: .labelDollarSign))
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
                            String(localized: .placeholderAmountUsd),
                            text: $model.amountInputText
                        )
                        .keyboardType(.decimalPad)
                        .font(.system(size: 36, weight: .semibold, design: .rounded).monospacedDigit())
                        .lineLimit(1)
                        .minimumScaleFactor(0.5)
                        .foregroundStyle(Color.clear)
                        .onChange(of: model.amountInputText) { _, new in
                            let sanitized = InputSanitizer.decimal(new)
                            model.amountInputText = sanitized.count > 16 ? String(sanitized.prefix(16)) : sanitized
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

            if model.exceedsAvailableAmount(appState: appState) {
                HStack(spacing: 6) {
                    Image(systemName: "exclamationmark.circle.fill")
                        .font(.caption)
                        .foregroundStyle(.red)
                    Text(String(localized: .errorAmountExceedsBalance))
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.red)
                }
                .transition(.opacity)
            }

            Toggle(isOn: $model.sendsAllFromOnchainWallet.animation(.snappy(duration: 0.32, extraBounce: 0.04))) {
                HStack(spacing: 8) {
                    LemniscateBloomIcon(isActive: sendAll, size: 16, tint: .green)
                    Text(String(localized: .toggleSendAll))
                        .font(.subheadline)
                }
            }
            .tint(.green)
            .onChange(of: model.sendsAllFromOnchainWallet) { _, isAll in
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
