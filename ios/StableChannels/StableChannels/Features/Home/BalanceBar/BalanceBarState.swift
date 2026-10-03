import SwiftUI

/// UI presentation state for the balance bar component.
/// Strictly coordinates UI state transitions, delegates geometry and trade evaluation
/// to BalanceBarTradeCalculator, and delegates haptic execution to an injected BalanceBarHaptics provider.
@Observable
final class BalanceBarState {
    var userSelectedFraction: CGFloat?
    var isPressing = false
    var atSellLimit = false
    var showDepositPrompt = false

    private let haptics: BalanceBarHaptics
    private var hasTriggeredHaptic = false
    private var lastTranslationX: CGFloat = 0
    private(set) var cumulativeDragDistance: CGFloat = 0
    private var depositPromptTimer: DispatchWorkItem?
    private var dragStartBaseFraction: CGFloat?
    private var startedEmpty = false

    init(haptics: BalanceBarHaptics = SystemBalanceBarHaptics()) {
        self.haptics = haptics
    }

    func effectiveFraction(allocation: ChannelAllocation, settleFraction: CGFloat?) -> CGFloat {
        if let userFraction = userSelectedFraction { return userFraction }
        if let settled = settleFraction { return settled }
        return allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
    }

    func handleDragChange(
        touchStartX: CGFloat,
        translationX: CGFloat,
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
            let withinThumb = BalanceBarTradeCalculator.isWithinThumb(
                touchX: touchStartX,
                thumbX: currentThumbX,
                thumbDiameter: thumbDiameter
            )
            guard allocation.isEmpty || withinThumb else { return }
            isPressing = true
            hasTriggeredHaptic = false
            atSellLimit = false
            lastTranslationX = 0
            cumulativeDragDistance = 0
            dragStartBaseFraction = allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction)
            startedEmpty = allocation.isEmpty
            depositPromptTimer?.cancel()
            showDepositPrompt = false
            onDragStarted?()
            haptics.tick()
        }
        guard isPressing else { return }

        cumulativeDragDistance += abs(translationX - lastTranslationX)
        lastTranslationX = translationX

        let baseFraction = dragStartBaseFraction ?? (allocation.isEmpty ? 0.5 : CGFloat(allocation.stableFraction))
        let rawFraction = BalanceBarTradeCalculator.calculateTargetFraction(
            initialFraction: baseFraction,
            translationX: translationX,
            barWidth: barWidth,
            thumbDiameter: thumbDiameter
        )

        if allocation.isEmpty || startedEmpty {
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
        translationX _: CGFloat,
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

        let wasStartedEmpty = startedEmpty
        let startBaseFraction = dragStartBaseFraction
        dragStartBaseFraction = nil
        startedEmpty = false

        if allocation.isEmpty || wasStartedEmpty {
            if BalanceBarTradeCalculator.isTap(totalDistance: cumulativeDragDistance) {
                userSelectedFraction = nil
                haptics.tick()
                onEmptyInteraction?()
            } else {
                haptics.tick()
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

        let baseFraction = startBaseFraction ?? CGFloat(allocation.stableFraction)
        let evaluation = BalanceBarTradeCalculator.calculateSelection(
            initialFraction: baseFraction,
            targetFraction: selected,
            totalUSD: allocation.totalUSD,
            stableUSD: allocation.stableUSD,
            maxSellUSD: maxSellUSD
        )

        if evaluation.isValidTrade, let direction = evaluation.direction, let onTradeRequest {
            haptics.impact()
            let request = TradeRequest(direction: direction, amountUSD: evaluation.clampedUSD)
            onTradeRequest(request)
        } else {
            withAnimation(.spring(response: 0.25, dampingFraction: 0.8)) {
                userSelectedFraction = nil
            }
        }
    }

    func resetSelection() {
        dragStartBaseFraction = nil
        startedEmpty = false
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
        haptics.warning()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) { [weak self] in
            self?.hasTriggeredHaptic = false
        }
    }
}
