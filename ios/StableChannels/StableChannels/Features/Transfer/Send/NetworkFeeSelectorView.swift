import SwiftUI

/// Speed selection card providing Economy, Standard, and Priority onchain fee choices.
struct NetworkFeeSelectorView: View {
    @Binding var selectedTier: NetworkFeeSpeedTier
    let baseFeeRateSatVb: Double
    let isSendAll: Bool
    let btcPrice: Double
    var showExplanation: Bool = false
    var recommendedFees: RecommendedFees?

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(String(localized: "header_speed_fee", defaultValue: "Confirmation Speed"))
                    .font(.footnote.weight(.medium))
                    .foregroundStyle(.secondary)
                Spacer()
                Text(verbatim: "\(formatRate(displayedRate(for: selectedTier))) sat/vB")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(Color.deepSendBlue)
            }

            HStack(spacing: 8) {
                ForEach(NetworkFeeSpeedTier.allCases) { tier in
                    feeTierOption(tier)
                }
            }

            if showExplanation {
                Text(verbatim: feeExplanationText)
                    .font(.caption2)
                    .foregroundStyle(.secondary)
            }
        }
        .sendCard()
    }

    private func feeTierOption(_ tier: NetworkFeeSpeedTier) -> some View {
        let isSelected = selectedTier == tier
        let rate = displayedRate(for: tier)
        let feeSats = PaymentFeeEstimator.estimateOnchainFee(feeRateSatVb: rate, isSendAll: isSendAll)
        let feeUSD = BitcoinUnits.usd(sats: feeSats, price: btcPrice)

        return Button {
            withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                selectedTier = tier
            }
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        } label: {
            VStack(spacing: 4) {
                Text(tier.title)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(isSelected ? Color.primary : .secondary)

                Text(tier.estimatedTime)
                    .font(.caption2)
                    .foregroundStyle(.secondary)

                Text(verbatim: "\(formatRate(rate)) sat/vB")
                    .font(.system(size: 11, weight: .bold, design: .monospaced))
                    .foregroundStyle(isSelected ? Color.deepSendBlue : .secondary)

                Text(verbatim: btcPrice > 0 ? "≈ \(feeUSD.usdFormatted)" : feeSats.satsFormatted)
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(isSelected ? Color.primary : .secondary)
            }
            .frame(maxWidth: .infinity)
            .padding(.vertical, 10)
            .padding(.horizontal, 4)
            .background(
                RoundedRectangle(cornerRadius: 10)
                    .fill(isSelected ? Color.deepSendBlue.opacity(0.12) : Color(uiColor: .tertiarySystemFill)
                        .opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 10)
                    .stroke(isSelected ? Color.deepSendBlue : Color.clear, lineWidth: 1.5)
            )
        }
        .buttonStyle(.plain)
    }

    /// Mirrors `SendFlowModel.effectiveFeeRateSatVb` so the displayed rate equals the rate sent.
    private func displayedRate(for tier: NetworkFeeSpeedTier) -> Double {
        tier.effectiveRate(baseRate: baseFeeRateSatVb, recommendedFees: recommendedFees)
    }

    private func formatRate(_ rate: Double) -> String {
        rate.truncatingRemainder(dividingBy: 1) == 0 ? String(format: "%.0f", rate) : String(format: "%.1f", rate)
    }

    private var feeExplanationText: String {
        let vbytes = isSendAll ? Constants.estimatedOnchainSendAllVBytes : Constants.estimatedOnchainSendVBytes
        let rate = displayedRate(for: selectedTier)
        return "Estimated at \(formatRate(rate)) sat/vB for ~\(vbytes) vB transaction. Higher fee density prioritizes block inclusion."
    }
}
