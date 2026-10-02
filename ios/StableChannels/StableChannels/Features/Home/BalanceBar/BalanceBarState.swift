import SwiftUI

/// UI presentation state for the balance bar component.
/// Strictly manages UI visual flags and delegates interaction geometry to BalanceBarInteraction
/// and financial trade evaluation to BalanceBarTradeCalculator.
@Observable
final class BalanceBarState {
    var userSelectedFraction: CGFloat?
    var isPressing = false
    var atSellLimit = false
    var showDepositPrompt = false

    private var hasTriggeredHaptic = false
    private var depositPromptTimer: DispatchWorkItem?

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
            let withinThumb = BalanceBarInteraction.isWithinThumb(
                touchX: gesture.startLocation.x,
                thumbX: currentThumbX,
                thumbDiameter: thumbDiameter
            )
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

        let baseFraction = allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
        let rawFraction = BalanceBarInteraction.calculateTargetFraction(
            initialFraction: baseFraction,
            translationX: gesture.translation.width,
            barWidth: barWidth
        )

        if allocation.isEmpty {
            userSelectedFraction = rawFraction
            return
        }

        let clampedResult = BalanceBarTradeCalculator.clampFraction(
            initialFraction: baseFraction,
            rawFraction: rawFraction,
            totalUSD: allocation.totalUSD,
            stableUSD: allocation.stableUSD,
            maxSellUSD: maxSellUSD
        )

        userSelectedFraction = clampedResult.fraction

        if clampedResult.isAtSellLimit {
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
            if BalanceBarInteraction.isTap(translationX: gesture.translation.width) {
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
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: baseFraction,
            targetFraction: selected,
            totalUSD: allocation.totalUSD,
            stableUSD: allocation.stableUSD,
            maxSellUSD: maxSellUSD
        )

        if evaluation.isValidTrade, let request = evaluation.tradeRequest {
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            onTradeRequest?(request)
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
