import SwiftUI

@Observable
final class BalanceBarState {
    var userSelectedFraction: CGFloat?
    var isPressing = false
    var atSellLimit = false
    var showDepositPrompt = false

    private var hasTriggeredHaptic = false
    private var depositPromptTimer: DispatchWorkItem?
    private let minTradeUSD: Double = 1.0

    func effectiveFraction(allocation: ChannelAllocation, settleFraction: CGFloat?) -> CGFloat {
        if let userFraction = userSelectedFraction { return userFraction }
        if let settled = settleFraction { return settled }
        return allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
    }

    func handleDragChange(
        gesture: DragGesture.Value,
        barWidth: CGFloat,
        currentThumbX: CGFloat,
        thumbDiameter: CGFloat,
        allocation: ChannelAllocation,
        maxSellUSD: Double,
        isAwakening: Bool,
        onDragStarted: (() -> Void)?
    ) {
        guard barWidth > 0, !isAwakening else { return }

        if !isPressing {
            let withinThumb = abs(gesture.startLocation.x - currentThumbX) < thumbDiameter * 1.5
            guard allocation.isEmpty || withinThumb else { return }
            isPressing = true
            hasTriggeredHaptic = false
            atSellLimit = false
            depositPromptTimer?.cancel()
            showDepositPrompt = false
            onDragStarted?()
            UIImpactFeedbackGenerator(style: .light).impactOccurred()
        }
        guard isPressing else { return }

        let rawFraction = min(max(gesture.location.x / barWidth, 0.0), 1.0)

        if allocation.isEmpty {
            userSelectedFraction = rawFraction
            return
        }

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

    func handleDragEnd(
        gesture: DragGesture.Value,
        barWidth: CGFloat,
        allocation: ChannelAllocation,
        maxSellUSD: Double,
        isAwakening: Bool,
        reduceMotion: Bool,
        onEmptyInteraction: (() -> Void)?,
        onTradeRequest: ((TradeRequest) -> Void)?
    ) {
        guard isPressing else { return }
        isPressing = false
        atSellLimit = false

        if allocation.isEmpty {
            let dragDistance = abs(gesture.translation.width)
            if dragDistance < 5 {
                userSelectedFraction = nil
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                onEmptyInteraction?()
            } else {
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                withAnimation(reduceMotion ? .easeInOut(duration: 0.25) : .spring(
                    response: 0.38,
                    dampingFraction: 0.68
                )) {
                    userSelectedFraction = nil
                }
                showDepositPrompt = true
                depositPromptTimer?.cancel()
                let timer = DispatchWorkItem { [weak self] in
                    withAnimation(.easeInOut(duration: 0.2)) {
                        self?.showDepositPrompt = false
                    }
                }
                depositPromptTimer = timer
                DispatchQueue.main.asyncAfter(deadline: .now() + 2.5, execute: timer)
            }
            return
        }

        guard barWidth > 0, !isAwakening,
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

    func resetSelection() {
        withAnimation(.spring(response: 0.35, dampingFraction: 0.8)) {
            userSelectedFraction = nil
        }
    }

    func cancelTimers() {
        depositPromptTimer?.cancel()
    }

    private func triggerSellLimitHaptic() {
        guard !hasTriggeredHaptic else { return }
        hasTriggeredHaptic = true
        UINotificationFeedbackGenerator().notificationOccurred(.warning)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.hasTriggeredHaptic = false
        }
    }
}
