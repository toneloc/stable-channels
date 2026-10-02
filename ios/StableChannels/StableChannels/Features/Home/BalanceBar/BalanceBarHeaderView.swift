import SwiftUI

struct BalanceBarHeaderView: View {
    let visFrac: CGFloat
    let isPressing: Bool
    let hasSelectedFraction: Bool
    let isAwakening: Bool
    let atSellLimit: Bool
    let maxSellUSD: Double
    let showDepositPrompt: Bool

    private var usdPct: Int {
        Int(round(visFrac * 100))
    }

    private var btcPct: Int {
        100 - usdPct
    }

    private var showConversion: Bool {
        isPressing || hasSelectedFraction || isAwakening || showDepositPrompt
    }

    var body: some View {
        HStack {
            Spacer()
            pillContent
                .opacity(showConversion ? 1 : 0)
                .animation(.easeInOut(duration: 0.15), value: showConversion)
            Spacer()
        }
    }

    @ViewBuilder
    private var pillContent: some View {
        if showDepositPrompt {
            depositPromptPill
        } else if atSellLimit {
            sellLimitPill
        } else {
            conversionPill
        }
    }

    private var depositPromptPill: some View {
        HStack(spacing: 6) {
            Image(systemName: "arrow.down.circle.fill")
                .font(.caption2)
                .foregroundStyle(.green)
            Text(String(localized: "deposit_to_balance_prompt", defaultValue: "Deposit to balance channel"))
                .font(.caption2.bold())
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .strokeBorder(Color.green.opacity(0.35), lineWidth: 1)
        )
    }

    private var sellLimitPill: some View {
        VStack(spacing: 1) {
            Text(StabilizationPolicy.maximumMessage(UInt64(maxSellUSD * 100 + 1e-7)))
                .font(.caption2.bold())
                .foregroundStyle(.red)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
            Text(String(
                localized: "stabilization_reserve_explanation",
                defaultValue: "Keeps a small BTC reserve in the channel."
            ))
            .font(.system(size: 9))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .minimumScaleFactor(0.8)
        }
        .multilineTextAlignment(.center)
        .padding(.horizontal, 10)
        .padding(.vertical, 2)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .strokeBorder(Color.red.opacity(0.35), lineWidth: 1)
        )
    }

    private var conversionPill: some View {
        let metrics = SliderConversionMetrics.calculate(usdPct: usdPct, btcPct: btcPct)
        return HStack(spacing: 8) {
            Text("\(usdPct)% USD")
                .font(.caption2.bold().monospacedDigit())
                .foregroundStyle(.green)
                .frame(width: metrics.sideWidth, alignment: .trailing)
                .lineLimit(1)
            Text("·")
                .font(.caption2.bold())
                .foregroundStyle(.secondary)
            Text("\(btcPct)% BTC")
                .font(.caption2.bold().monospacedDigit())
                .foregroundStyle(.orange)
                .frame(width: metrics.sideWidth, alignment: .leading)
                .lineLimit(1)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 12))
        .overlay(
            RoundedRectangle(cornerRadius: 12)
                .strokeBorder(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }
}
