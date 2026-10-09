import SwiftUI

struct BalanceBarHeaderView: View {
    let visFrac: CGFloat
    let isPressing: Bool
    let hasSelectedFraction: Bool
    let isAwakening: Bool
    let atSellLimit: Bool
    let maxSellUSD: Double
    let showDepositPrompt: Bool
    var isOnline: Bool = true
    var onEmptyInteraction: (() -> Void)?

    @State private var offlinePulse: Bool = false

    private var usdPct: Int {
        Int(round(visFrac * 100))
    }

    private var btcPct: Int {
        100 - usdPct
    }

    private var showConversion: Bool {
        !isOnline || isPressing || hasSelectedFraction || isAwakening || showDepositPrompt
    }

    var body: some View {
        HStack {
            Spacer()
            pillContent
                .opacity(showConversion ? 1 : 0)
                .animation(.easeInOut(duration: 0.15), value: showConversion)
                .animation(.easeInOut(duration: 0.15), value: atSellLimit)
            Spacer()
        }
    }

    @ViewBuilder
    private var pillContent: some View {
        if !isOnline {
            offlinePill
        } else if showDepositPrompt {
            depositPromptPill
        } else if atSellLimit {
            sellLimitPill
        } else {
            conversionPill
        }
    }

    private var offlinePill: some View {
        VStack(spacing: 1) {
            HStack(spacing: 5) {
                Image(systemName: "wifi.slash")
                    .font(.system(size: 10, weight: .semibold))
                    .foregroundStyle(.red)
                    .symbolEffect(.pulse.byLayer, options: .repeating)

                Text(String(localized: "offline_title", defaultValue: "No Internet Connection"))
                    .font(.caption2.bold())
                    .foregroundStyle(.red)

                Circle()
                    .fill(Color.red)
                    .frame(width: 4, height: 4)
                    .scaleEffect(offlinePulse ? 1.3 : 0.8)
                    .opacity(offlinePulse ? 1.0 : 0.5)
            }

            Text(String(
                localized: "offline_check_network",
                defaultValue: "Please check your network connection"
            ))
            .font(.system(size: 9))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .minimumScaleFactor(0.8)
        }
        .multilineTextAlignment(.center)
        .padding(.horizontal, 10)
        .padding(.vertical, 3)
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(Color.red.opacity(0.35), lineWidth: 1)
        )
        .shadow(color: Color.red.opacity(0.18), radius: 4, x: 0, y: 1)
        .onAppear {
            withAnimation(.easeInOut(duration: 1.0).repeatForever(autoreverses: true)) {
                offlinePulse = true
            }
        }
    }

    private var depositPromptPill: some View {
        Button {
            onEmptyInteraction?()
        } label: {
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
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 6))
            .overlay(
                RoundedRectangle(cornerRadius: 6)
                    .strokeBorder(Color.green.opacity(0.35), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
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
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6)
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
        .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 6))
        .overlay(
            RoundedRectangle(cornerRadius: 6)
                .strokeBorder(Color.primary.opacity(0.08), lineWidth: 1)
        )
    }
}
