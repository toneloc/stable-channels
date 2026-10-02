import SwiftUI

struct BalanceBarView: View {
    let allocation: ChannelAllocation
    var maxSellUSD: Double = 0
    var isTrading: Bool = false
    var onDragStarted: (() -> Void)?
    var onTradeRequest: ((TradeRequest) -> Void)?
    var onEmptyInteraction: (() -> Void)?

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    @State private var userSelectedFraction: CGFloat?
    @State private var isPressing = false
    @State private var atSellLimit = false
    @State private var showDepositPrompt = false
    @State private var pulseScale: CGFloat = 1.0
    @State private var hasTriggeredHaptic = false
    @State private var animator = BalanceBarAnimationCoordinator()

    private let thumbDiameter: CGFloat = 28
    private let barHeight: CGFloat = 20
    private let baseHeaderHeight: CGFloat = 24
    private let verticalSpacing: CGFloat = 6
    private let minTradeUSD: Double = 1.0

    private var interactive: Bool {
        onTradeRequest != nil || onEmptyInteraction != nil
    }

    private var currentHeaderHeight: CGFloat {
        atSellLimit ? 34 : baseHeaderHeight
    }

    private var totalHeight: CGFloat {
        interactive ? (currentHeaderHeight + verticalSpacing + thumbDiameter) : 10
    }

    private var currentBarHeight: CGFloat {
        interactive ? barHeight : 10
    }

    private var effectiveFraction: CGFloat {
        if let userFraction = userSelectedFraction { return userFraction }
        if let settled = animator.settleFraction { return settled }
        return allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
    }

    private var visibleFraction: CGFloat {
        effectiveFraction
    }

    var body: some View {
        GeometryReader { geometry in
            let barWidth = geometry.size.width
            let visFrac = visibleFraction
            let thumbX = thumbPosition(barWidth: barWidth, visFrac: visFrac)

            VStack(spacing: verticalSpacing) {
                if interactive {
                    BalanceBarHeaderView(
                        visFrac: visFrac,
                        isPressing: isPressing,
                        hasSelectedFraction: userSelectedFraction != nil,
                        isAwakening: animator.isAwakening,
                        atSellLimit: atSellLimit,
                        maxSellUSD: maxSellUSD,
                        showDepositPrompt: showDepositPrompt
                    )
                    .frame(height: currentHeaderHeight)
                    .animation(.easeInOut(duration: 0.15), value: atSellLimit)
                }

                ZStack {
                    BalanceBarTrackView(
                        barWidth: barWidth,
                        height: currentBarHeight,
                        fraction: visFrac,
                        isEmpty: allocation.isEmpty,
                        isAwakening: animator.isAwakening,
                        floodScale: animator.radialFloodScale,
                        floodOpacity: animator.radialFloodOpacity
                    )

                    if interactive {
                        thumbView(thumbX: thumbX)
                    }
                }
                .frame(width: barWidth, height: interactive ? thumbDiameter : 10)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { handleDragChange(gesture: $0, barWidth: barWidth, currentThumbX: thumbX) }
                        .onEnded { handleDragEnd(gesture: $0, barWidth: barWidth) }
                )
            }
            .animation(.spring(response: 0.45, dampingFraction: 0.75), value: allocation.isEmpty)
            .onChange(of: allocation.isEmpty) { wasEmpty, isEmpty in
                if wasEmpty && !isEmpty && !reduceMotion {
                    animator.triggerAwakening(targetFraction: CGFloat(allocation.stableFraction))
                }
            }
            .onChange(of: isTrading) { wasTrading, isTrading in
                if wasTrading && !isTrading {
                    withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) { userSelectedFraction = nil }
                }
            }
            .onChange(of: allocation.stableFraction) { _, _ in
                guard !isTrading else { return }
                withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) { userSelectedFraction = nil }
            }
        }
        .frame(height: totalHeight)
    }

    // MARK: - Subviews

    private func thumbView(thumbX: CGFloat) -> some View {
        Circle()
            .fill(.white)
            .frame(width: thumbDiameter, height: thumbDiameter)
            .shadow(color: .black.opacity(0.2), radius: 4, y: 2)
            .overlay(
                Circle()
                    .strokeBorder(Color.primary.opacity(0.08), lineWidth: 1)
            )
            .scaleEffect(animator.isAwakening ? animator
                .thumbAwakenScale : (isPressing ? 1.15 : (allocation.isEmpty ? 1.0 : pulseScale)))
            .position(x: thumbX, y: thumbDiameter / 2)
            .animation(.easeOut(duration: 0.15), value: isPressing)
            .onAppear {
                withAnimation(.easeInOut(duration: 1.5).repeatForever(autoreverses: true)) { pulseScale = 1.08 }
            }
    }

    // MARK: - Layout Calculations

    private func thumbPosition(barWidth: CGFloat, visFrac: CGFloat) -> CGFloat {
        thumbDiameter / 2 + (barWidth - thumbDiameter) * visFrac
    }

    // MARK: - Drag Handling

    private func handleDragChange(gesture: DragGesture.Value, barWidth: CGFloat, currentThumbX: CGFloat) {
        guard interactive, barWidth > 0, !animator.isAwakening else { return }

        if allocation.isEmpty {
            if !isPressing {
                isPressing = true
                showDepositPrompt = true
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                onEmptyInteraction?()
                DispatchQueue.main.asyncAfter(deadline: .now() + 1.8) {
                    showDepositPrompt = false
                }
            }
            return
        }

        if !isPressing {
            guard abs(gesture.startLocation.x - currentThumbX) < thumbDiameter * 1.5 else { return }
            isPressing = true
            hasTriggeredHaptic = false
            atSellLimit = false
            onDragStarted?()
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        }
        guard isPressing else { return }

        let rawFraction = min(max(gesture.location.x / barWidth, 0.0), 1.0)
        let baseFraction = CGFloat(allocation.stableFraction)
        let totalUSD = allocation.totalUSD
        let maxSellFraction = totalUSD > 0 ? CGFloat(max(0, maxSellUSD) / totalUSD) : 0
        let maxBuyFraction = totalUSD > 0 ? CGFloat(max(0, allocation.stableUSD) / totalUSD) : 0

        let minAllowedFraction = max(0.0, baseFraction - maxBuyFraction)
        let maxAllowedFraction = min(1.0, baseFraction + maxSellFraction)

        let clampedFraction = min(max(rawFraction, minAllowedFraction), maxAllowedFraction)
        userSelectedFraction = clampedFraction

        if rawFraction > maxAllowedFraction {
            if !atSellLimit {
                atSellLimit = true
                triggerSellLimitHaptic()
            }
        } else {
            atSellLimit = false
        }
    }

    private func handleDragEnd(gesture _: DragGesture.Value, barWidth: CGFloat) {
        guard isPressing else { return }
        isPressing = false
        atSellLimit = false

        if allocation.isEmpty { return }

        guard barWidth > 0, !animator.isAwakening,
              let selected = userSelectedFraction else { return }

        let baseFraction = CGFloat(allocation.stableFraction)
        let deltaFraction = selected - baseFraction
        let fractionMoved = abs(deltaFraction)

        guard fractionMoved > 0.02 else {
            withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                userSelectedFraction = nil
            }
            return
        }

        let direction: TradeDirection = deltaFraction > 0 ? .sell : .buy
        let totalUSD = allocation.totalUSD
        var requestedUSD = totalUSD * Double(fractionMoved)

        if direction == .sell {
            requestedUSD = min(requestedUSD, maxSellUSD)
        } else {
            requestedUSD = min(requestedUSD, allocation.stableUSD)
        }

        if requestedUSD >= minTradeUSD {
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            onTradeRequest?(TradeRequest(direction: direction, amountUSD: requestedUSD))
        } else {
            withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                userSelectedFraction = nil
            }
        }
    }

    private func triggerSellLimitHaptic() {
        guard !hasTriggeredHaptic else { return }
        hasTriggeredHaptic = true
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
        withAnimation(.easeInOut(duration: 0.12).repeatCount(2, autoreverses: true)) { pulseScale = 1.08 }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            hasTriggeredHaptic = false
            pulseScale = 1.0
        }
    }
}
