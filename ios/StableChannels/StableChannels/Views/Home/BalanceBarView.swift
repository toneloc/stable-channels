import SwiftUI

enum TradeDirection {
    case buy   // drag left: grow BTC
    case sell  // drag right: grow USD
}

struct TradeRequest: Identifiable {
    let id = UUID()
    let direction: TradeDirection
    let amountUSD: Double
}

struct BalanceBarView: View {
    let stableUSD: Double
    let nativeSats: UInt64
    let totalSats: UInt64
    let btcPrice: Double
    var onTradeRequest: ((TradeDirection, Double) -> Void)? = nil

    @State private var dragOffset: CGFloat = 0
    @State private var isPressing = false
    @State private var pulseScale: CGFloat = 1.0

    private let thumbDiameter: CGFloat = 28
    private let barHeight: CGFloat = 20
    private let minTradeUSD: Double = 1.0

    private var nativeUSD: Double {
        btcPrice > 0 ? Double(nativeSats) / Double(Constants.satsInBTC) * btcPrice : 0
    }
    private var totalUSD: Double { stableUSD + nativeUSD }
    private var stableFraction: Double { totalUSD > 0 ? stableUSD / totalUSD : 0 }
    private var interactive: Bool { onTradeRequest != nil }

    var body: some View {
        GeometryReader { geo in
            let barWidth = geo.size.width
            let baseX = barWidth * stableFraction
            let thumbX = max(0, min(barWidth, baseX + dragOffset))
            let visFrac = barWidth > 0 ? thumbX / barWidth : stableFraction
            let h = interactive ? barHeight : 10

            let usdPct = Int(round(visFrac * 100))
            let btcPct = 100 - usdPct

            ZStack {
                // Bar segments
                HStack(spacing: 2) {
                    if visFrac > 0.01 {
                        RoundedRectangle(cornerRadius: 5)
                            .fill(
                                LinearGradient(
                                    colors: [.green.opacity(0.8), .green],
                                    startPoint: .leading,
                                    endPoint: .trailing
                                )
                            )
                            .frame(width: max(barWidth * visFrac, 4), height: h)
                    }
                    if (1 - visFrac) > 0.01 {
                        RoundedRectangle(cornerRadius: 5)
                            .fill(
                                LinearGradient(
                                    colors: [.orange, .orange.opacity(0.8)],
                                    startPoint: .leading,
                                    endPoint: .trailing
                                )
                            )
                            .frame(width: max(barWidth * (1 - visFrac), 4), height: h)
                    }
                }
                .clipShape(RoundedRectangle(cornerRadius: 5))
                .animation(isPressing ? nil : .easeInOut(duration: 0.3), value: stableFraction)

                // Draggable thumb with percentage label
                if interactive {
                    Circle()
                        .fill(.white)
                        .frame(width: thumbDiameter, height: thumbDiameter)
                        .shadow(color: .black.opacity(0.2), radius: 4, y: 2)
                        .scaleEffect(isPressing ? 1.15 : pulseScale)
                        .overlay(alignment: .top) {
                            if isPressing {
                                Text("\(usdPct)% USD  \(btcPct)% BTC")
                                    .font(.caption2.bold())
                                    .fixedSize()
                                    .foregroundStyle(.primary)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(.ultraThinMaterial, in: Capsule())
                                    .fixedSize()
                                    .offset(y: -34)
                                    .transition(.opacity.combined(with: .scale(scale: 0.8)))
                            }
                        }
                        .position(x: thumbX, y: geo.size.height / 2)
                        .animation(.easeOut(duration: 0.15), value: isPressing)
                        .onAppear {
                            withAnimation(.easeInOut(duration: 1.5).repeatForever(autoreverses: true)) {
                                pulseScale = 1.08
                            }
                        }
                }
            }
            .contentShape(Rectangle())
            .coordinateSpace(name: "balanceBar")
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .named("balanceBar"))
                    .onChanged { value in
                        guard interactive else { return }
                        if !isPressing {
                            guard abs(value.startLocation.x - baseX) < thumbDiameter * 1.5 else { return }
                            isPressing = true
                            UIImpactFeedbackGenerator(style: .light).impactOccurred()
                        }
                        guard isPressing else { return }
                        let translation = value.location.x - value.startLocation.x
                        dragOffset = max(-baseX, min(barWidth - baseX, translation))
                    }
                    .onEnded { _ in
                        defer {
                            withAnimation(.easeOut(duration: 0.25)) { dragOffset = 0 }
                            isPressing = false
                        }
                        guard isPressing else { return }
                        let fraction = barWidth > 0 ? dragOffset / barWidth : 0
                        let tradeUSD = abs(fraction) * totalUSD
                        guard tradeUSD >= minTradeUSD else { return }
                        UIImpactFeedbackGenerator(style: .medium).impactOccurred()
                        let direction: TradeDirection = dragOffset > 0 ? .sell : .buy
                        let clamped = direction == .buy
                            ? min(tradeUSD, stableUSD)
                            : min(tradeUSD, nativeUSD)
                        onTradeRequest?(direction, clamped)
                    }
            )
        }
        .frame(height: interactive ? thumbDiameter : 10)
    }
}
